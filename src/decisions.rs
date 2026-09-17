//! Decisions: settled choices so nothing is re-decided.
//!
//! These are decisions agents make while working on a project. Decisions
//! about Tirith itself are ADRs in `docs/5-decisions/`. Each decision is
//! one committed Markdown file under `.tirith/decisions/`, written in the
//! memory-note format with kind `decision` (ADR-0022); [`Decision::to_note`]
//! and [`Decision::from_note`] are the two directions.

use std::collections::BTreeSet;

use chrono::{DateTime, Timelike, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::memory::{MemoryKind, MemoryNote, Permalink};
use crate::types::{AgentId, DecisionId, MemoryId, Page, RepoPath};

const RATIONALE_HEADING: &str = "## Rationale";
const ALTERNATIVES_HEADING: &str = "## Alternatives";

/// A recorded decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Decision {
    /// Unique identifier.
    pub id: DecisionId,
    /// Short title, for example "Use JSON files for storage".
    pub title: String,
    /// What was decided.
    pub decision: String,
    /// Why.
    pub rationale: String,
    /// What else was considered.
    pub alternatives: Vec<String>,
    /// Paths the decision constrains.
    pub affects_paths: Vec<RepoPath>,
    /// Who recorded it.
    pub recorded_by: AgentId,
    /// The file stem under `.tirith/decisions/`, derived from the title
    /// once and never changed. Absent on rows written before ADR-0022; the
    /// log fills it in on load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permalink: Option<Permalink>,
    /// When it was recorded.
    pub recorded_at: DateTime<Utc>,
}

impl Decision {
    /// The memory-note form of this decision: the decision text, then a
    /// `## Rationale` section and a `## Alternatives` bullet list when they
    /// are not empty. `to_markdown` on the result is the file on disk.
    pub fn to_note(&self) -> MemoryNote {
        let mut body = self.decision.trim().to_owned();
        if !self.rationale.trim().is_empty() {
            body.push_str("\n\n");
            body.push_str(RATIONALE_HEADING);
            body.push_str("\n\n");
            body.push_str(self.rationale.trim());
        }
        if !self.alternatives.is_empty() {
            body.push_str("\n\n");
            body.push_str(ALTERNATIVES_HEADING);
            body.push('\n');
            for alternative in &self.alternatives {
                body.push_str("\n- ");
                body.push_str(alternative.trim());
            }
        }
        MemoryNote {
            id: memory_id(self.id),
            permalink: self
                .permalink
                .clone()
                .unwrap_or_else(|| Permalink::for_title(&self.title, memory_id(self.id))),
            title: self.title.clone(),
            kind: MemoryKind::Decision,
            body,
            observations: Vec::new(),
            relations: Vec::new(),
            paths: self.affects_paths.clone(),
            tags: Vec::new(),
            author: self.recorded_by.clone(),
            updated_by: self.recorded_by.clone(),
            created_at: self.recorded_at,
            updated_at: self.recorded_at,
        }
    }

    /// The decision a note file holds: the inverse of [`to_note`](Self::to_note).
    /// Fails only when the decision text is empty.
    pub fn from_note(note: &MemoryNote) -> Result<Self, DecisionError> {
        let mut decision = String::new();
        let mut rationale = String::new();
        let mut alternatives = Vec::new();
        let mut section = 0;
        for line in note.body.lines() {
            match line.trim_end() {
                RATIONALE_HEADING => section = 1,
                ALTERNATIVES_HEADING => section = 2,
                text => match section {
                    0 => {
                        decision.push_str(text);
                        decision.push('\n');
                    }
                    1 => {
                        rationale.push_str(text);
                        rationale.push('\n');
                    }
                    _ => {
                        if let Some(item) = text.trim_start().strip_prefix("- ") {
                            alternatives.push(item.trim().to_owned());
                        }
                    }
                },
            }
        }
        let decision = decision.trim().to_owned();
        if decision.is_empty() {
            return Err(DecisionError::EmptyDecision);
        }
        Ok(Self {
            id: DecisionId::parse(&note.id.to_string()).unwrap_or_default(),
            title: note.title.clone(),
            decision,
            rationale: rationale.trim().to_owned(),
            alternatives,
            affects_paths: note.paths.clone(),
            recorded_by: note.author.clone(),
            recorded_at: note.created_at,
            permalink: Some(note.permalink.clone()),
        })
    }

    /// The path of this decision's file, relative to `.tirith/decisions/`.
    /// Decisions loaded through [`DecisionLog`] always have a permalink.
    pub fn file_path(&self) -> String {
        self.permalink
            .as_ref()
            .map_or_else(
                || Permalink::for_title(&self.title, memory_id(self.id)),
                Clone::clone,
            )
            .file_path()
    }

    /// Whether `path` overlaps any affected path.
    pub fn affects(&self, path: &RepoPath) -> bool {
        self.affects_paths.iter().any(|p| p.overlaps(path))
    }

    /// Case-insensitive substring match over title, decision, and
    /// rationale.
    pub fn matches(&self, query: &str) -> bool {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return true;
        }
        [&self.title, &self.decision, &self.rationale]
            .iter()
            .any(|field| field.to_lowercase().contains(&needle))
    }
}

/// Input for recording a decision. Build it with [`NewDecision::new`] and
/// the `with_*` setters outside this module, so a new optional field
/// never breaks a caller (decision 19c6595c).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewDecision {
    /// See [`Decision::title`].
    pub title: String,
    /// See [`Decision::decision`].
    pub decision: String,
    /// See [`Decision::rationale`].
    pub rationale: String,
    /// See [`Decision::alternatives`].
    pub alternatives: Vec<String>,
    /// See [`Decision::affects_paths`].
    pub affects_paths: Vec<RepoPath>,
}

impl NewDecision {
    /// A decision titled `title` saying `decision`, with no rationale,
    /// alternatives, or paths yet.
    ///
    /// ```
    /// use tirith::decisions::NewDecision;
    /// use tirith::types::RepoPath;
    ///
    /// let decision = NewDecision::new("Tokens are opaque", "Compare them, never parse them")
    ///     .with_affects_paths(vec![RepoPath::new("src/auth").unwrap()]);
    /// assert_eq!(decision.affects_paths.len(), 1);
    /// ```
    pub fn new(title: impl Into<String>, decision: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            decision: decision.into(),
            ..Self::default()
        }
    }

    /// Sets why it was decided.
    #[must_use]
    pub fn with_rationale(mut self, rationale: impl Into<String>) -> Self {
        self.rationale = rationale.into();
        self
    }

    /// Sets what else was considered.
    #[must_use]
    pub fn with_alternatives(mut self, alternatives: Vec<String>) -> Self {
        self.alternatives = alternatives;
        self
    }

    /// Sets the paths the decision constrains.
    #[must_use]
    pub fn with_affects_paths(mut self, paths: Vec<RepoPath>) -> Self {
        self.affects_paths = paths;
        self
    }
}

/// Why a decision operation was refused.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum DecisionError {
    /// The title was empty.
    #[error("decision title must not be empty")]
    EmptyTitle,
    /// The decision text was empty.
    #[error("decision text must not be empty")]
    EmptyDecision,
}

/// All decisions, oldest first.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionLog {
    decisions: Vec<Decision>,
}

/// The decision id in the memory id space; both wrap a UUID.
fn memory_id(id: DecisionId) -> MemoryId {
    MemoryId::parse(&id.to_string()).unwrap_or_default()
}

/// A permalink for `title` that no decision in `taken` uses: the slug of
/// the title, or the slug plus the id when another decision has it.
fn free_permalink(title: &str, id: DecisionId, taken: &BTreeSet<String>) -> Permalink {
    let base = Permalink::for_title(title, memory_id(id));
    if !taken.contains(base.as_str()) {
        return base;
    }
    Permalink::parse(&format!("{}-{}", base.as_str(), id.short()))
        .unwrap_or_else(|_| Permalink::for_title("", memory_id(id)))
}

impl DecisionLog {
    /// Rebuilds a log from persisted decisions.
    /// Rows from before ADR-0022 have no permalink; each gets one that no
    /// other decision uses, so every decision maps to its own file.
    pub fn from_decisions(decisions: Vec<Decision>) -> Self {
        let mut taken: BTreeSet<String> = decisions
            .iter()
            .filter_map(|d| d.permalink.as_ref().map(|p| p.as_str().to_owned()))
            .collect();
        let decisions = decisions
            .into_iter()
            .map(|mut decision| {
                if decision.permalink.is_none() {
                    let permalink = free_permalink(&decision.title, decision.id, &taken);
                    taken.insert(permalink.as_str().to_owned());
                    decision.permalink = Some(permalink);
                }
                decision
            })
            .collect();
        Self { decisions }
    }

    /// Ids of every decision, in order; what the store marks as changed
    /// after importing an old JSON Lines log.
    pub fn ids(&self) -> Vec<DecisionId> {
        self.decisions.iter().map(|d| d.id).collect()
    }

    /// All decisions in record order.
    pub fn decisions(&self) -> &[Decision] {
        &self.decisions
    }

    /// Records a decision.
    pub fn record(
        &mut self,
        by: AgentId,
        new: NewDecision,
        now: DateTime<Utc>,
    ) -> Result<&Decision, DecisionError> {
        let title = new.title.trim().to_owned();
        if title.is_empty() {
            return Err(DecisionError::EmptyTitle);
        }
        let decision = new.decision.trim().to_owned();
        if decision.is_empty() {
            return Err(DecisionError::EmptyDecision);
        }
        let id = DecisionId::new();
        let taken: BTreeSet<String> = self
            .decisions
            .iter()
            .filter_map(|d| d.permalink.as_ref().map(|p| p.as_str().to_owned()))
            .collect();
        let permalink = free_permalink(&title, id, &taken);
        self.decisions.push(Decision {
            id,
            title,
            decision,
            rationale: new.rationale,
            alternatives: new.alternatives,
            affects_paths: new.affects_paths,
            recorded_by: by,
            // Whole seconds, so the Markdown file round-trips exactly.
            recorded_at: now.with_nanosecond(0).unwrap_or(now),
            permalink: Some(permalink),
        });
        Ok(self
            .decisions
            .last()
            .unwrap_or_else(|| unreachable!("just pushed")))
    }

    /// Decisions matching the optional path and free-text filters.
    pub fn list(&self, path: Option<&RepoPath>, query: Option<&str>) -> Vec<&Decision> {
        self.decisions
            .iter()
            .filter(|d| path.is_none_or(|p| d.affects(p)))
            .filter(|d| query.is_none_or(|q| d.matches(q)))
            .collect()
    }

    /// Newest `limit` decisions matching the filters and older than the
    /// `(recorded_at, id)` cursor `before`. See [`Page`].
    pub fn list_page(
        &self,
        path: Option<&RepoPath>,
        query: Option<&str>,
        before: Option<&(DateTime<Utc>, DecisionId)>,
        limit: usize,
    ) -> Page<&Decision> {
        Page::newest_first(
            self.list(path, query),
            |d| (d.recorded_at, d.id),
            before,
            limit,
        )
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn agent(name: &str) -> AgentId {
        AgentId::new(name).unwrap()
    }

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 15, 18, 0, 0).unwrap()
    }

    #[test]
    fn query_and_path_filters() {
        let mut log = DecisionLog::default();
        log.record(
            agent("a"),
            NewDecision::new("Use JSON storage", "State is written to JSON files")
                .with_rationale("diffable in git")
                .with_affects_paths(vec![RepoPath::new("src/store.rs").unwrap()]),
            t0(),
        )
        .unwrap();
        assert_eq!(log.list(None, Some("DIFFABLE")).len(), 1);
        assert!(log.list(None, Some("sqlite")).is_empty());
        assert_eq!(
            log.list(Some(&RepoPath::new("src").unwrap()), None).len(),
            1
        );
        assert!(
            log.list(Some(&RepoPath::new("docs").unwrap()), None)
                .is_empty()
        );
    }

    #[test]
    fn empty_fields_are_refused() {
        let mut log = DecisionLog::default();
        let mut new = NewDecision::new(" ", "x");
        assert_eq!(
            log.record(agent("a"), new.clone(), t0()).err(),
            Some(DecisionError::EmptyTitle)
        );
        new.title = "t".into();
        new.decision = String::new();
        assert_eq!(
            log.record(agent("a"), new, t0()).err(),
            Some(DecisionError::EmptyDecision)
        );
    }

    #[test]
    fn a_decision_round_trips_through_its_markdown_file() {
        let mut log = DecisionLog::default();
        let full = NewDecision::new("Use one file per decision", "Markdown, one file each.")
            .with_rationale("Prose diffs; JSONL does not.")
            .with_alternatives(vec!["keep JSONL".into(), "SQLite".into()])
            .with_affects_paths(vec![RepoPath::new("src/store.rs").unwrap()]);
        let recorded = log.record(agent("a"), full, t0()).unwrap().clone();
        let text = recorded.to_note().to_markdown();
        let back = MemoryNote::from_markdown(&text, t0()).unwrap();
        assert_eq!(back.kind, MemoryKind::Decision);
        assert_eq!(Decision::from_note(&back).unwrap(), recorded);

        let bare = log
            .record(agent("a"), NewDecision::new("Bare", "Just the text."), t0())
            .unwrap()
            .clone();
        let back = MemoryNote::from_markdown(&bare.to_note().to_markdown(), t0()).unwrap();
        assert_eq!(Decision::from_note(&back).unwrap(), bare);
        assert!(bare.rationale.is_empty() && bare.alternatives.is_empty());
    }

    #[test]
    fn same_titles_get_distinct_permalinks_and_old_rows_get_one_on_load() {
        let mut log = DecisionLog::default();
        let first = log
            .record(agent("a"), NewDecision::new("Same title", "one"), t0())
            .unwrap()
            .clone();
        let second = log
            .record(agent("a"), NewDecision::new("Same title", "two"), t0())
            .unwrap()
            .clone();
        assert_ne!(first.file_path(), second.file_path());
        assert!(second.file_path().contains(&second.id.short()));

        let mut legacy = first.clone();
        legacy.permalink = None;
        legacy.id = DecisionId::new();
        let reloaded = DecisionLog::from_decisions(vec![first.clone(), second, legacy]);
        let paths: BTreeSet<String> = reloaded
            .decisions()
            .iter()
            .map(Decision::file_path)
            .collect();
        assert_eq!(paths.len(), 3, "every decision has its own file");
        assert!(
            reloaded.decisions().iter().all(|d| d.permalink.is_some()),
            "rows from before ADR-0022 get a permalink on load"
        );
    }

    #[test]
    fn an_empty_query_matches_every_decision() {
        let mut log = DecisionLog::default();
        let decision = log
            .record(
                agent("a"),
                NewDecision::new("Use JSON storage", "State is written to JSON files"),
                t0(),
            )
            .unwrap()
            .clone();
        assert!(decision.matches(""));
        assert!(decision.matches("   "));
        assert_eq!(log.list(None, Some("")).len(), 1);
    }
}
