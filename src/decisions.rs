//! Decisions log: settled choices so nothing is re-decided.
//!
//! These are decisions agents make while working on a project. Decisions
//! about Tirith itself are ADRs in `docs/5-decisions/`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::types::{AgentId, DecisionId, Page, RepoPath};

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
    /// When it was recorded.
    pub recorded_at: DateTime<Utc>,
}

impl Decision {
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

/// Input for recording a decision.
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

impl DecisionLog {
    /// Rebuilds a log from persisted decisions.
    pub fn from_decisions(decisions: Vec<Decision>) -> Self {
        Self { decisions }
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
        self.decisions.push(Decision {
            id: DecisionId::new(),
            title,
            decision,
            rationale: new.rationale,
            alternatives: new.alternatives,
            affects_paths: new.affects_paths,
            recorded_by: by,
            recorded_at: now,
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
            NewDecision {
                title: "Use JSON storage".into(),
                decision: "State is written to JSON files".into(),
                rationale: "diffable in git".into(),
                affects_paths: vec![RepoPath::new("src/store.rs").unwrap()],
                ..NewDecision::default()
            },
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
        let mut new = NewDecision {
            title: " ".into(),
            decision: "x".into(),
            ..NewDecision::default()
        };
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
}
