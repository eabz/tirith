//! Change notices: "something changed and these paths care".
//!
//! Dependents list notices for the paths they are about to touch before
//! acting, then acknowledge the ones they have handled so `unread_by`
//! filters stay meaningful.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::types::{AgentId, ContractId, NoticeId, RepoPath};

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
    /// Agents that have handled it.
    pub acked_by: Vec<AgentId>,
}

impl Notice {
    /// Whether `path` overlaps any affected path.
    pub fn affects(&self, path: &RepoPath) -> bool {
        self.affected_paths.iter().any(|p| p.overlaps(path))
    }

    /// Whether `agent` still needs to read this notice: they neither
    /// published nor acknowledged it.
    pub fn unread_by(&self, agent: &AgentId) -> bool {
        &self.published_by != agent && !self.acked_by.contains(agent)
    }
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

/// All notices, oldest first.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoticeBoard {
    notices: Vec<Notice>,
}

impl NoticeBoard {
    /// Rebuilds a board from persisted notices.
    pub fn from_notices(notices: Vec<Notice>) -> Self {
        Self { notices }
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

    /// Marks a notice as handled by `agent`. Acknowledging twice is fine.
    pub fn ack(&mut self, agent: AgentId, id: NoticeId) -> Result<&Notice, NoticeError> {
        let notice = self
            .notices
            .iter_mut()
            .find(|n| n.id == id)
            .ok_or(NoticeError::NotFound(id))?;
        if !notice.acked_by.contains(&agent) {
            notice.acked_by.push(agent);
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
        board.ack(agent("bob"), id).unwrap();
        board.ack(agent("bob"), id).unwrap();
        let unread = board.list(&NoticeFilter {
            unread_by: Some(agent("bob")),
            ..NoticeFilter::default()
        });
        assert_eq!(unread.len(), 1);
        assert_eq!(board.notices()[0].acked_by.len(), 1);
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
            board.ack(agent("a"), ghost).err(),
            Some(NoticeError::NotFound(ghost))
        );
    }
}
