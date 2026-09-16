//! Change notices: "something changed and these paths care".
//!
//! Dependents list notices for the paths they are about to touch before
//! acting, then acknowledge the ones they have handled so `unread_by`
//! acting. A notice counts as seen by an agent once Tirith has delivered
//! it to that agent, in a brief or an unread listing; there is no manual
//! acknowledgement (ADR-0021). Seen marks are their own append-only log
//! ([`NoticeSeen`]): a notice row is never rewritten after it is
//! published, and the in-memory `acked_by` set is rebuilt from the log on
//! load.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::types::{AgentId, ContractId, NoticeId, Page, PrefixError, RepoPath, resolve_prefix};

/// What kind of change a notice announces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeKind {
    /// Something was renamed; `from` and `to` carry the names.
    Rename,
    /// A signature or shape changed.
    Signature,
    /// Something was removed.
    Removed,
    /// A file or item moved; `from` and `to` carry the locations.
    Moved,
    /// Behavior changed without a visible signature change.
    Behavior,
    /// A contract got a new version. Emitted automatically.
    Contract,
}

impl NoticeKind {
    /// The wire name of the kind.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rename => "rename",
            Self::Signature => "signature",
            Self::Removed => "removed",
            Self::Moved => "moved",
            Self::Behavior => "behavior",
            Self::Contract => "contract",
        }
    }
}

impl fmt::Display for NoticeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for NoticeKind {
    type Err = NoticeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "rename" | "renamed" => Ok(Self::Rename),
            "signature" => Ok(Self::Signature),
            "removed" | "remove" => Ok(Self::Removed),
            "moved" | "move" => Ok(Self::Moved),
            "behavior" | "behaviour" => Ok(Self::Behavior),
            "contract" => Ok(Self::Contract),
            other => Err(NoticeError::UnknownKind(other.to_owned())),
        }
    }
}

/// A published change notice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Notice {
    /// Unique identifier.
    pub id: NoticeId,
    /// What kind of change.
    pub kind: NoticeKind,
    /// One-line human summary.
    pub summary: String,
    /// The old name, location, or shape, when meaningful.
    pub from: Option<String>,
    /// The new name, location, or shape, when meaningful.
    pub to: Option<String>,
    /// Paths whose code is affected.
    pub affected_paths: Vec<RepoPath>,
    /// The contract this notice concerns, if any.
    pub contract_id: Option<ContractId>,
    /// Who published it.
    pub published_by: AgentId,
    /// When it was published.
    pub published_at: DateTime<Utc>,
    /// Agents it has been delivered to (the wire keeps the historical
    /// field name).
    pub acked_by: Vec<AgentId>,
}

impl Notice {
    /// Whether `path` overlaps any affected path.
    pub fn affects(&self, path: &RepoPath) -> bool {
        self.affected_paths.iter().any(|p| p.overlaps(path))
    }

    /// Whether `agent` still needs to read this notice: they neither
    /// published it nor had it delivered.
    pub fn unread_by(&self, agent: &AgentId) -> bool {
        &self.published_by != agent && !self.acked_by.contains(agent)
    }
}

/// One delivery: `notice_id` was shown to `agent` at `at`. Appended to
/// its own runtime log so marking never rewrites the notices log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct NoticeSeen {
    /// The delivered notice.
    pub notice_id: NoticeId,
    /// Who saw it.
    pub agent: AgentId,
    /// When.
    pub at: DateTime<Utc>,
}

/// Input for publishing a notice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewNotice {
    /// See [`Notice::kind`].
    pub kind: NoticeKind,
    /// See [`Notice::summary`].
    pub summary: String,
    /// See [`Notice::from`].
    pub from: Option<String>,
    /// See [`Notice::to`].
    pub to: Option<String>,
    /// See [`Notice::affected_paths`].
    pub affected_paths: Vec<RepoPath>,
    /// See [`Notice::contract_id`].
    pub contract_id: Option<ContractId>,
}

/// Filters for listing notices. All are optional and combine with AND.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NoticeFilter {
    /// Only notices affecting this path.
    pub path: Option<RepoPath>,
    /// Only notices published at or after this instant.
    pub since: Option<DateTime<Utc>>,
    /// Only notices this agent has not published or acknowledged.
    pub unread_by: Option<AgentId>,
}

/// Why a notice operation was refused.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum NoticeError {
    /// No notice has this id.
    #[error("no notice with id {0}")]
    NotFound(NoticeId),
    /// The summary was empty.
    #[error("notice summary must not be empty")]
    EmptySummary,
    /// The kind string was not recognized.
    #[error(
        "unknown notice kind {0:?}; expected rename, signature, removed, moved, behavior, or contract"
    )]
    UnknownKind(String),
}

/// All notices, oldest first, plus the seen log in the order it was
/// appended.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoticeBoard {
    notices: Vec<Notice>,
    #[serde(default)]
    seen: Vec<NoticeSeen>,
}

impl NoticeBoard {
    /// Rebuilds a board from persisted notices.
    pub fn from_notices(notices: Vec<Notice>) -> Self {
        Self {
            notices,
            seen: Vec::new(),
        }
    }

    /// All notices in publish order.
    pub fn notices(&self) -> &[Notice] {
        &self.notices
    }

    /// Publishes a notice.
    pub fn publish(
        &mut self,
        by: AgentId,
        new: NewNotice,
        now: DateTime<Utc>,
    ) -> Result<&Notice, NoticeError> {
        let summary = new.summary.trim().to_owned();
        if summary.is_empty() {
            return Err(NoticeError::EmptySummary);
        }
        self.notices.push(Notice {
            id: NoticeId::new(),
            kind: new.kind,
            summary,
            from: new.from,
            to: new.to,
            affected_paths: new.affected_paths,
            contract_id: new.contract_id,
            published_by: by,
            published_at: now,
            acked_by: Vec::new(),
        });
        Ok(self
            .notices
            .last()
            .unwrap_or_else(|| unreachable!("just pushed")))
    }

    /// Notices matching `filter`.
    pub fn list(&self, filter: &NoticeFilter) -> Vec<&Notice> {
        self.notices
            .iter()
            .filter(|n| filter.path.as_ref().is_none_or(|p| n.affects(p)))
            .filter(|n| filter.since.is_none_or(|s| n.published_at >= s))
            .filter(|n| filter.unread_by.as_ref().is_none_or(|a| n.unread_by(a)))
            .collect()
    }

    /// Newest `limit` notices matching `filter` and older than the
    /// `(published_at, id)` cursor `before`. See [`Page`].
    pub fn list_page(
        &self,
        filter: &NoticeFilter,
        before: Option<&(DateTime<Utc>, NoticeId)>,
        limit: usize,
    ) -> Page<&Notice> {
        Page::newest_first(self.list(filter), |n| (n.published_at, n.id), before, limit)
    }

    /// Resolves a full id or a unique prefix to a notice id.
    pub fn resolve_id(&self, raw: &str) -> Result<NoticeId, PrefixError> {
        resolve_prefix("notice", self.notices.iter().map(|n| n.id), raw)
    }

    /// Rebuilds a board from persisted notices and the seen log, replaying
    /// each delivery into the notice's `acked_by`. Entries for notices
    /// that no longer exist are dropped.
    pub fn from_parts(notices: Vec<Notice>, seen: Vec<NoticeSeen>) -> Self {
        let mut board = Self {
            notices,
            seen: Vec::with_capacity(seen.len()),
        };
        for entry in seen {
            if let Some(notice) = board.notices.iter_mut().find(|n| n.id == entry.notice_id) {
                if !notice.acked_by.contains(&entry.agent) {
                    notice.acked_by.push(entry.agent.clone());
                }
                board.seen.push(entry);
            }
        }
        board
    }

    /// The seen log, oldest first. The persister appends new entries from
    /// here through a `LogCursor`.
    pub fn seen(&self) -> &[NoticeSeen] {
        &self.seen
    }

    /// Records that `id` was delivered to `agent` at `now`, appending to
    /// the seen log the first time; a repeat changes nothing and appends
    /// nothing. Called by brief on claim and by unread listings.
    pub fn mark_seen(
        &mut self,
        agent: &AgentId,
        id: NoticeId,
        now: DateTime<Utc>,
    ) -> Result<&Notice, NoticeError> {
        let notice = self
            .notices
            .iter_mut()
            .find(|n| n.id == id)
            .ok_or(NoticeError::NotFound(id))?;
        if !notice.acked_by.contains(agent) {
            notice.acked_by.push(agent.clone());
            self.seen.push(NoticeSeen {
                notice_id: id,
                agent: agent.clone(),
                at: now,
            });
        }
        Ok(notice)
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};

    use super::*;

    fn agent(name: &str) -> AgentId {
        AgentId::new(name).unwrap()
    }

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 15, 18, 0, 0).unwrap()
    }

    fn rename(paths: &[&str]) -> NewNotice {
        NewNotice {
            kind: NoticeKind::Rename,
            summary: "renamed session_id to token".into(),
            from: Some("session_id".into()),
            to: Some("token".into()),
            affected_paths: paths.iter().map(|p| RepoPath::new(p).unwrap()).collect(),
            contract_id: None,
        }
    }

    #[test]
    fn filters_by_path_since_and_unread() {
        let mut board = NoticeBoard::default();
        let id = board
            .publish(agent("alice"), rename(&["src/client"]), t0())
            .unwrap()
            .id;
        board
            .publish(
                agent("alice"),
                rename(&["src/server"]),
                t0() + Duration::seconds(10),
            )
            .unwrap();
        let client_file = RepoPath::new("src/client/sessions.rs").unwrap();
        let by_path = board.list(&NoticeFilter {
            path: Some(client_file),
            ..NoticeFilter::default()
        });
        assert_eq!(by_path.len(), 1);
        let since = board.list(&NoticeFilter {
            since: Some(t0() + Duration::seconds(5)),
            ..NoticeFilter::default()
        });
        assert_eq!(since.len(), 1);
        assert!(
            board
                .list(&NoticeFilter {
                    unread_by: Some(agent("alice")),
                    ..NoticeFilter::default()
                })
                .is_empty(),
            "publisher never has unread notices"
        );
        assert_eq!(
            board
                .list(&NoticeFilter {
                    unread_by: Some(agent("bob")),
                    ..NoticeFilter::default()
                })
                .len(),
            2
        );
        board.mark_seen(&agent("bob"), id, t0()).unwrap();
        board.mark_seen(&agent("bob"), id, t0()).unwrap();
        let unread = board.list(&NoticeFilter {
            unread_by: Some(agent("bob")),
            ..NoticeFilter::default()
        });
        assert_eq!(unread.len(), 1);
        assert_eq!(board.notices()[0].acked_by.len(), 1);
    }

    #[test]
    fn pages_are_newest_first_and_prefixes_resolve() {
        let mut board = NoticeBoard::default();
        for i in 0..5 {
            board
                .publish(agent("a"), rename(&["src"]), t0() + Duration::seconds(i))
                .unwrap();
        }
        let page = board.list_page(&NoticeFilter::default(), None, 2);
        assert_eq!(page.total, 5);
        assert!(page.truncated());
        assert_eq!(page.items[0].published_at, t0() + Duration::seconds(4));
        let next = page.next_before(|n| (n.published_at, n.id)).unwrap();
        let older = board.list_page(&NoticeFilter::default(), Some(&next), 10);
        assert_eq!(older.items.len(), 3);
        assert_eq!(older.items[0].published_at, t0() + Duration::seconds(2));
        assert!(!older.truncated());
        let id = board.notices()[3].id;
        assert_eq!(board.resolve_id(&id.short()), Ok(id));
        assert!(board.resolve_id("").is_err());
    }

    #[test]
    fn empty_summary_and_unknown_id_are_refused() {
        let mut board = NoticeBoard::default();
        let mut empty = rename(&[]);
        empty.summary = " ".into();
        assert_eq!(
            board.publish(agent("a"), empty, t0()).err(),
            Some(NoticeError::EmptySummary)
        );
        let ghost = NoticeId::new();
        assert_eq!(
            board.mark_seen(&agent("a"), ghost, t0()).err(),
            Some(NoticeError::NotFound(ghost))
        );
    }

    #[test]
    fn deliveries_are_logged_once_and_replayed_on_load() {
        let mut board = NoticeBoard::default();
        let id = board
            .publish(agent("alice"), rename(&["src/auth"]), t0())
            .unwrap()
            .id;
        let later = t0() + Duration::seconds(5);
        board.mark_seen(&agent("bob"), id, later).unwrap();
        board.mark_seen(&agent("bob"), id, later).unwrap();
        board.mark_seen(&agent("carol"), id, later).unwrap();
        assert_eq!(board.seen().len(), 2, "one log entry per agent");
        assert_eq!(board.seen()[0].agent, agent("bob"));
        assert_eq!(board.seen()[0].at, later);

        // What the store does on load: notice rows as published (no
        // deliveries inside them) plus the seen log.
        let mut rows = board.notices().to_vec();
        for row in &mut rows {
            row.acked_by.clear();
        }
        let reloaded = NoticeBoard::from_parts(rows, board.seen().to_vec());
        assert_eq!(
            reloaded.notices()[0].acked_by,
            vec![agent("bob"), agent("carol")]
        );
        assert!(!reloaded.notices()[0].unread_by(&agent("bob")));
        assert!(reloaded.notices()[0].unread_by(&agent("dave")));
        assert_eq!(reloaded.seen().len(), 2);

        // An entry for a notice that no longer exists is dropped, not kept.
        let stray = NoticeSeen {
            notice_id: NoticeId::new(),
            agent: agent("bob"),
            at: later,
        };
        let pruned = NoticeBoard::from_parts(board.notices().to_vec(), vec![stray]);
        assert!(pruned.seen().is_empty());
    }
}
