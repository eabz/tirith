//! Task board: work items with status, owner, and dependencies.
//!
//! Agents pull the next unblocked task instead of being assigned one. A
//! task's owner exists only while it is in progress, blocked, or done, so
//! ownership is carried inside [`TaskState`] rather than as a separate
//! field that could disagree with the status.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::types::{AgentId, RepoPath, TaskId};

/// Where a task is in its life, plus who is responsible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum TaskState {
    /// Not started. Anyone may pull it once its dependencies are done.
    Todo,
    /// Being worked on.
    InProgress {
        /// The agent working on it.
        owner: AgentId,
    },
    /// Cannot proceed. Stays with its owner, if any, until unblocked.
    Blocked {
        /// The agent that owned it when it became blocked.
        owner: Option<AgentId>,
        /// Why it is blocked.
        reason: String,
    },
    /// Finished.
    Done {
        /// The agent that finished it.
        by: AgentId,
    },
}

impl TaskState {
    /// The responsible agent, if the state has one.
    pub fn owner(&self) -> Option<&AgentId> {
        match self {
            Self::Todo => None,
            Self::InProgress { owner } => Some(owner),
            Self::Blocked { owner, .. } => owner.as_ref(),
            Self::Done { by } => Some(by),
        }
    }

    /// The status without its data.
    pub fn status(&self) -> TaskStatus {
        match self {
            Self::Todo => TaskStatus::Todo,
            Self::InProgress { .. } => TaskStatus::InProgress,
            Self::Blocked { .. } => TaskStatus::Blocked,
            Self::Done { .. } => TaskStatus::Done,
        }
    }
}

/// A status name, used for filters and updates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// See [`TaskState::Todo`].
    Todo,
    /// See [`TaskState::InProgress`].
    InProgress,
    /// See [`TaskState::Blocked`].
    Blocked,
    /// See [`TaskState::Done`].
    Done,
}

impl TaskStatus {
    /// The wire name of the status.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::InProgress => "in_progress",
            Self::Blocked => "blocked",
            Self::Done => "done",
        }
    }
}

impl fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for TaskStatus {
    type Err = TaskError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "todo" => Ok(Self::Todo),
            "in_progress" | "in-progress" | "inprogress" => Ok(Self::InProgress),
            "blocked" => Ok(Self::Blocked),
            "done" => Ok(Self::Done),
            other => Err(TaskError::UnknownStatus(other.to_owned())),
        }
    }
}

/// A note appended to a task by an agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TaskNote {
    /// When the note was written.
    pub at: DateTime<Utc>,
    /// Who wrote it.
    pub by: AgentId,
    /// The note.
    pub text: String,
}

/// A work item on the board.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Task {
    /// Unique identifier.
    pub id: TaskId,
    /// Short imperative title.
    pub title: String,
    /// Longer description, may be empty.
    pub description: String,
    /// Higher pulls first. Default 0.
    pub priority: i32,
    /// Tasks that must be done before this one can be pulled.
    pub depends_on: Vec<TaskId>,
    /// Paths the task is expected to touch; a hint for claims.
    pub paths: Vec<RepoPath>,
    /// Who created it.
    pub created_by: AgentId,
    /// When it was created.
    pub created_at: DateTime<Utc>,
    /// When it last changed.
    pub updated_at: DateTime<Utc>,
    /// Current status and owner.
    #[serde(flatten)]
    pub state: TaskState,
    /// Notes in chronological order.
    pub notes: Vec<TaskNote>,
}

/// Input for creating a task.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewTask {
    /// See [`Task::title`].
    pub title: String,
    /// See [`Task::description`].
    pub description: String,
    /// See [`Task::priority`].
    pub priority: i32,
    /// See [`Task::depends_on`].
    pub depends_on: Vec<TaskId>,
    /// See [`Task::paths`].
    pub paths: Vec<RepoPath>,
}

/// Why a task operation was refused.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TaskError {
    /// No task has this id.
    #[error("no task with id {0}")]
    NotFound(TaskId),
    /// A dependency refers to a task that does not exist.
    #[error("dependency {0} does not exist")]
    UnknownDependency(TaskId),
    /// The title was empty.
    #[error("task title must not be empty")]
    EmptyTitle,
    /// The status string was not recognized.
    #[error("unknown status {0:?}; expected todo, in_progress, blocked, or done")]
    UnknownStatus(String),
}

/// All tasks.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskBoard {
    tasks: Vec<Task>,
}

impl TaskBoard {
    /// Rebuilds a board from persisted tasks.
    pub fn from_tasks(tasks: Vec<Task>) -> Self {
        Self { tasks }
    }

    /// All tasks in creation order.
    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    /// Looks up a task by id.
    pub fn get(&self, id: TaskId) -> Option<&Task> {
        self.tasks.iter().find(|t| t.id == id)
    }

    /// Adds a task in the `todo` state.
    pub fn create(
        &mut self,
        by: AgentId,
        new: NewTask,
        now: DateTime<Utc>,
    ) -> Result<&Task, TaskError> {
        let title = new.title.trim().to_owned();
        if title.is_empty() {
            return Err(TaskError::EmptyTitle);
        }
        if let Some(missing) = new.depends_on.iter().find(|d| self.get(**d).is_none()) {
            return Err(TaskError::UnknownDependency(*missing));
        }
        self.tasks.push(Task {
            id: TaskId::new(),
            title,
            description: new.description,
            priority: new.priority,
            depends_on: new.depends_on,
            paths: new.paths,
            created_by: by,
            created_at: now,
            updated_at: now,
            state: TaskState::Todo,
            notes: Vec::new(),
        });
        Ok(self
            .tasks
            .last()
            .unwrap_or_else(|| unreachable!("just pushed")))
    }

    /// Whether every dependency of `task` is done.
    pub fn is_unblocked(&self, task: &Task) -> bool {
        task.depends_on
            .iter()
            .all(|d| matches!(self.get(*d).map(|t| &t.state), Some(TaskState::Done { .. })))
    }

    /// Assigns the highest-priority unblocked `todo` task to `agent` and
    /// marks it in progress. Ties go to the oldest task.
    pub fn pull(&mut self, agent: AgentId, now: DateTime<Utc>) -> Option<&Task> {
        let candidate = self
            .tasks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.state == TaskState::Todo && self.is_unblocked(t))
            .max_by(|(ia, a), (ib, b)| {
                a.priority
                    .cmp(&b.priority)
                    .then_with(|| b.created_at.cmp(&a.created_at))
                    .then_with(|| ib.cmp(ia))
            })
            .map(|(i, _)| i)?;
        let task = &mut self.tasks[candidate];
        task.state = TaskState::InProgress { owner: agent };
        task.updated_at = now;
        Some(task)
    }

    /// Moves a task to `status` on behalf of `agent`, appending `note` if
    /// given. For `blocked`, the note doubles as the reason.
    pub fn update(
        &mut self,
        agent: AgentId,
        id: TaskId,
        status: TaskStatus,
        note: Option<String>,
        now: DateTime<Utc>,
    ) -> Result<&Task, TaskError> {
        let task = self
            .tasks
            .iter_mut()
            .find(|t| t.id == id)
            .ok_or(TaskError::NotFound(id))?;
        let note = note.map(|n| n.trim().to_owned()).filter(|n| !n.is_empty());
        task.state = match status {
            TaskStatus::Todo => TaskState::Todo,
            TaskStatus::InProgress => TaskState::InProgress {
                owner: agent.clone(),
            },
            TaskStatus::Blocked => TaskState::Blocked {
                owner: task.state.owner().cloned().or_else(|| Some(agent.clone())),
                reason: note.clone().unwrap_or_default(),
            },
            TaskStatus::Done => TaskState::Done { by: agent.clone() },
        };
        task.updated_at = now;
        if let Some(text) = note {
            task.notes.push(TaskNote {
                at: now,
                by: agent,
                text,
            });
        }
        Ok(task)
    }

    /// Tasks matching the optional status and owner filters.
    pub fn list(&self, status: Option<TaskStatus>, owner: Option<&AgentId>) -> Vec<&Task> {
        self.tasks
            .iter()
            .filter(|t| status.is_none_or(|s| t.state.status() == s))
            .filter(|t| owner.is_none_or(|o| t.state.owner() == Some(o)))
            .collect()
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

    fn task(title: &str, priority: i32, deps: Vec<TaskId>) -> NewTask {
        NewTask {
            title: title.into(),
            priority,
            depends_on: deps,
            ..NewTask::default()
        }
    }

    #[test]
    fn pull_prefers_priority_then_age() {
        let mut board = TaskBoard::default();
        let low = board
            .create(agent("a"), task("low", 0, vec![]), t0())
            .unwrap()
            .id;
        let high_old = board
            .create(agent("a"), task("high-old", 5, vec![]), t0())
            .unwrap()
            .id;
        let high_new = board
            .create(
                agent("a"),
                task("high-new", 5, vec![]),
                t0() + Duration::seconds(1),
            )
            .unwrap()
            .id;
        assert_eq!(board.pull(agent("b"), t0()).unwrap().id, high_old);
        assert_eq!(board.pull(agent("b"), t0()).unwrap().id, high_new);
        assert_eq!(board.pull(agent("b"), t0()).unwrap().id, low);
        assert!(board.pull(agent("b"), t0()).is_none());
    }

    #[test]
    fn pull_skips_tasks_with_unfinished_dependencies() {
        let mut board = TaskBoard::default();
        let first = board
            .create(agent("a"), task("first", 0, vec![]), t0())
            .unwrap()
            .id;
        let second = board
            .create(agent("a"), task("second", 9, vec![first]), t0())
            .unwrap()
            .id;
        assert_eq!(board.pull(agent("b"), t0()).unwrap().id, first);
        assert!(board.pull(agent("c"), t0()).is_none());
        board
            .update(agent("b"), first, TaskStatus::Done, None, t0())
            .unwrap();
        assert_eq!(board.pull(agent("c"), t0()).unwrap().id, second);
    }

    #[test]
    fn unknown_dependency_is_refused() {
        let mut board = TaskBoard::default();
        let ghost = TaskId::new();
        assert_eq!(
            board
                .create(agent("a"), task("x", 0, vec![ghost]), t0())
                .err(),
            Some(TaskError::UnknownDependency(ghost))
        );
        assert_eq!(
            board.create(agent("a"), task("  ", 0, vec![]), t0()).err(),
            Some(TaskError::EmptyTitle)
        );
    }

    #[test]
    fn blocked_keeps_owner_and_records_reason() {
        let mut board = TaskBoard::default();
        let id = board
            .create(agent("a"), task("x", 0, vec![]), t0())
            .unwrap()
            .id;
        board.pull(agent("b"), t0()).unwrap();
        let updated = board
            .update(
                agent("b"),
                id,
                TaskStatus::Blocked,
                Some("waiting on api".into()),
                t0(),
            )
            .unwrap();
        assert_eq!(
            updated.state,
            TaskState::Blocked {
                owner: Some(agent("b")),
                reason: "waiting on api".into()
            }
        );
        assert_eq!(updated.notes.len(), 1);
        assert_eq!(
            board
                .list(Some(TaskStatus::Blocked), Some(&agent("b")))
                .len(),
            1
        );
        assert!(board.list(Some(TaskStatus::Todo), None).is_empty());
    }

    #[test]
    fn status_parses_common_spellings() {
        assert_eq!(
            "in-progress".parse::<TaskStatus>().unwrap(),
            TaskStatus::InProgress
        );
        assert_eq!("DONE".parse::<TaskStatus>().unwrap(), TaskStatus::Done);
        assert!("nope".parse::<TaskStatus>().is_err());
    }
}
