//! Task board: work items with status, owner, and dependencies.
//!
//! Agents pull the next unblocked task instead of being assigned one. A
//! task's owner exists only while it is in progress, blocked, or done, so
//! ownership is carried inside [`TaskState`] rather than as a separate
//! field that could disagree with the status.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::types::{AgentId, Page, PrefixError, RepoPath, TaskId, resolve_prefix};

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

/// Input for creating a task. Build it with [`NewTask::new`] and the
/// `with_*` setters outside this module, so a new optional field never
/// breaks a caller (decision 19c6595c).
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

impl NewTask {
    /// A task titled `title` with no description, priority 0, no
    /// dependencies, and no path hints.
    ///
    /// ```
    /// use tirith::tasks::NewTask;
    ///
    /// let task = NewTask::new("Write the summary").with_priority(5);
    /// assert_eq!(task.priority, 5);
    /// assert!(task.depends_on.is_empty());
    /// ```
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            ..Self::default()
        }
    }

    /// Sets the longer description.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Sets the priority; higher pulls first.
    #[must_use]
    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    /// Sets the tasks that must be done before this one can be pulled.
    #[must_use]
    pub fn with_depends_on(mut self, depends_on: Vec<TaskId>) -> Self {
        self.depends_on = depends_on;
        self
    }

    /// Sets the paths the task is expected to touch.
    #[must_use]
    pub fn with_paths(mut self, paths: Vec<RepoPath>) -> Self {
        self.paths = paths;
        self
    }
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
    /// The task is in progress under another agent; pass `force` to take it.
    #[error("task {id} is in progress under {owner} since {since}; pass force to take it")]
    OwnedByOther {
        /// The task.
        id: TaskId,
        /// Who holds it.
        owner: AgentId,
        /// When it last changed under that owner.
        since: DateTime<Utc>,
    },
}

/// A path another agent is working on, through a live claim or an
/// in-progress task.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Hold {
    /// The held path.
    pub path: RepoPath,
    /// The agent holding it.
    pub owner: AgentId,
}

/// A task assigned by [`TaskBoard::pull`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pulled {
    /// The task, now in progress.
    pub task: Task,
    /// Held paths overlapping the task's paths, sorted and deduplicated,
    /// which the caller waits for. Empty when the task was free. Otherwise
    /// other agents' in-progress task paths, plus their live claims when
    /// every candidate overlapped one.
    pub waiting_on: Vec<Hold>,
}

/// Why [`TaskBoard::pull_ready`] assigned nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotReady {
    /// No task is `todo`: no claim ending or dependency finishing can free
    /// one, so there is nothing to wait for.
    NoTodo,
    /// Some task is `todo`, but each one waits on unfinished dependencies
    /// or overlaps another agent's live claim.
    Held,
}

/// The candidate a pull takes, before it is assigned.
struct Choice {
    id: TaskId,
    waiting_on: Vec<Hold>,
    /// Whether it overlaps another agent's live claim, which a choice only
    /// does when every candidate does.
    claim_held: bool,
}

/// The holds overlapping any of `task`'s paths under the claim rule
/// (symbol anchors included, ADR-0029), sorted and deduplicated.
fn blockers(task: &Task, holds: &[Hold]) -> Vec<Hold> {
    let mut found: Vec<Hold> = holds
        .iter()
        .filter(|h| task.paths.iter().any(|p| p.claim_overlaps(&h.path)))
        .cloned()
        .collect();
    found.sort();
    found.dedup();
    found
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

    /// Returns to `todo` every in-progress task whose owner has been silent
    /// for longer than `threshold`, so a dead agent never blocks the tasks
    /// that depend on its work. `last_seen` gives an owner's last activity;
    /// an owner never seen counts as silent since the task last changed.
    /// Returns the ids reaped, in board order.
    pub fn reap_orphans(
        &mut self,
        last_seen: impl Fn(&AgentId) -> Option<DateTime<Utc>>,
        threshold: Duration,
        now: DateTime<Utc>,
    ) -> Vec<TaskId> {
        let mut reaped = Vec::new();
        for task in &mut self.tasks {
            let TaskState::InProgress { owner } = &task.state else {
                continue;
            };
            let silent_since = last_seen(owner).unwrap_or(task.updated_at);
            if now - silent_since <= threshold {
                continue;
            }
            task.notes.push(TaskNote {
                at: now,
                by: owner.clone(),
                text: format!(
                    "returned to todo: owner {owner} silent since {}",
                    silent_since.to_rfc3339_opts(SecondsFormat::Secs, true)
                ),
            });
            task.state = TaskState::Todo;
            task.updated_at = now;
            reaped.push(task.id);
        }
        reaped
    }

    /// Whether every dependency of `task` is done.
    pub fn is_unblocked(&self, task: &Task) -> bool {
        task.depends_on
            .iter()
            .all(|d| matches!(self.get(*d).map(|t| &t.state), Some(TaskState::Done { .. })))
    }

    /// Assigns the best unblocked `todo` task to `agent` and marks it in
    /// progress (ADR-0028). Candidates go in [`candidates`](Self::candidates)
    /// order and are ranked in three tiers: a task no hold overlaps first;
    /// then one whose only overlaps are other agents' in-progress task
    /// paths, listed in [`Pulled::waiting_on`]; then, when every candidate
    /// overlaps a live claim in `claims` (other agents' claims, supplied by
    /// the caller), the first of them, with every hold it overlaps. A task
    /// with no paths is never held, and the caller's own claims and tasks
    /// never hold.
    pub fn pull(&mut self, agent: AgentId, claims: &[Hold], now: DateTime<Utc>) -> Option<Pulled> {
        let choice = self.choose(&agent, claims)?;
        self.assign(agent, choice, now)
    }

    /// [`pull`](Self::pull) for a caller that can wait (ADR-0028, revised):
    /// assigns a task only while no live claim in `claims` overlaps it, so a
    /// claimed file is waited for rather than handed out. When nothing is
    /// ready, says whether waiting can help: [`NotReady::Held`] while a
    /// `todo` task waits on a claim or on dependencies, [`NotReady::NoTodo`]
    /// when the board holds no `todo` task at all.
    pub fn pull_ready(
        &mut self,
        agent: AgentId,
        claims: &[Hold],
        now: DateTime<Utc>,
    ) -> Result<Pulled, NotReady> {
        let ready = self
            .choose(&agent, claims)
            .filter(|choice| !choice.claim_held)
            .and_then(|choice| self.assign(agent, choice, now));
        match ready {
            Some(pulled) => Ok(pulled),
            None if self.tasks.iter().any(|t| t.state == TaskState::Todo) => Err(NotReady::Held),
            None => Err(NotReady::NoTodo),
        }
    }

    /// The candidate [`pull`](Self::pull) takes, in its tier order, or
    /// `None` when no `todo` task is unblocked.
    fn choose(&self, agent: &AgentId, claims: &[Hold]) -> Option<Choice> {
        let tasks = self.task_holds(agent);
        let mut behind_tasks: Option<Choice> = None;
        let mut behind_claims: Option<Choice> = None;
        for task in self.candidates() {
            let by_claims = blockers(task, claims);
            let by_tasks = blockers(task, &tasks);
            match (by_claims.is_empty(), by_tasks.is_empty()) {
                (true, true) => {
                    return Some(Choice {
                        id: task.id,
                        waiting_on: Vec::new(),
                        claim_held: false,
                    });
                }
                (true, false) if behind_tasks.is_none() => {
                    behind_tasks = Some(Choice {
                        id: task.id,
                        waiting_on: by_tasks,
                        claim_held: false,
                    });
                }
                (false, _) if behind_claims.is_none() => {
                    let mut waiting_on = by_claims;
                    waiting_on.extend(by_tasks);
                    waiting_on.sort();
                    waiting_on.dedup();
                    behind_claims = Some(Choice {
                        id: task.id,
                        waiting_on,
                        claim_held: true,
                    });
                }
                _ => {}
            }
        }
        behind_tasks.or(behind_claims)
    }

    /// Takes `choice`, unless another agent pulled it in between.
    fn assign(&mut self, agent: AgentId, choice: Choice, now: DateTime<Utc>) -> Option<Pulled> {
        let task = self.pull_id(agent, choice.id, now)?.clone();
        Some(Pulled {
            task,
            waiting_on: choice.waiting_on,
        })
    }

    /// The paths of in-progress tasks owned by agents other than `agent`.
    fn task_holds(&self, agent: &AgentId) -> Vec<Hold> {
        let mut all = Vec::new();
        for task in &self.tasks {
            if let TaskState::InProgress { owner } = &task.state
                && owner != agent
            {
                all.extend(task.paths.iter().map(|path| Hold {
                    path: path.clone(),
                    owner: owner.clone(),
                }));
            }
        }
        all
    }

    /// Unblocked `todo` tasks in pull order: highest priority first, ties
    /// to the oldest.
    pub fn candidates(&self) -> Vec<&Task> {
        let mut found: Vec<(usize, &Task)> = self
            .tasks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.state == TaskState::Todo && self.is_unblocked(t))
            .collect();
        found.sort_by(|(ia, a), (ib, b)| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| a.created_at.cmp(&b.created_at))
                .then_with(|| ia.cmp(ib))
        });
        found.into_iter().map(|(_, t)| t).collect()
    }

    /// Assigns task `id` to `agent` if it is still an unblocked `todo`
    /// task; `None` otherwise, so a stale choice never takes a task
    /// someone else already pulled.
    pub fn pull_id(&mut self, agent: AgentId, id: TaskId, now: DateTime<Utc>) -> Option<&Task> {
        let index = self.tasks.iter().position(|t| t.id == id)?;
        if self.tasks[index].state != TaskState::Todo || !self.is_unblocked(&self.tasks[index]) {
            return None;
        }
        let task = &mut self.tasks[index];
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
        force: bool,
        now: DateTime<Utc>,
    ) -> Result<&Task, TaskError> {
        let task = self
            .tasks
            .iter_mut()
            .find(|t| t.id == id)
            .ok_or(TaskError::NotFound(id))?;
        let mut note = note.map(|n| n.trim().to_owned()).filter(|n| !n.is_empty());
        if let TaskState::InProgress { owner } = &task.state
            && *owner != agent
        {
            if !force {
                return Err(TaskError::OwnedByOther {
                    id,
                    owner: owner.clone(),
                    since: task.updated_at,
                });
            }
            let forced = format!("forced by {agent}: was in progress under {owner}");
            note = Some(match note {
                Some(text) => format!("{forced}. {text}"),
                None => forced,
            });
        }
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

    /// Most recently updated `limit` tasks matching the filters and
    /// updated before `before`. See [`Page`].
    pub fn list_page(
        &self,
        status: Option<TaskStatus>,
        owner: Option<&AgentId>,
        before: Option<&(DateTime<Utc>, TaskId)>,
        limit: usize,
    ) -> Page<&Task> {
        Page::newest_first(
            self.list(status, owner),
            |t| (t.updated_at, t.id),
            before,
            limit,
        )
    }

    /// Resolves a full id or a unique prefix to a task id.
    pub fn resolve_id(&self, raw: &str) -> Result<TaskId, PrefixError> {
        resolve_prefix("task", self.tasks.iter().map(|t| t.id), raw)
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
        NewTask::new(title)
            .with_priority(priority)
            .with_depends_on(deps)
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
        assert_eq!(board.pull(agent("b"), &[], t0()).unwrap().task.id, high_old);
        assert_eq!(board.pull(agent("b"), &[], t0()).unwrap().task.id, high_new);
        assert_eq!(board.pull(agent("b"), &[], t0()).unwrap().task.id, low);
        assert!(board.pull(agent("b"), &[], t0()).is_none());
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
        assert_eq!(board.pull(agent("b"), &[], t0()).unwrap().task.id, first);
        assert!(board.pull(agent("c"), &[], t0()).is_none());
        board
            .update(agent("b"), first, TaskStatus::Done, None, false, t0())
            .unwrap();
        assert_eq!(board.pull(agent("c"), &[], t0()).unwrap().task.id, second);
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
        board.pull(agent("b"), &[], t0()).unwrap();
        let updated = board
            .update(
                agent("b"),
                id,
                TaskStatus::Blocked,
                Some("waiting on api".into()),
                false,
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

    #[test]
    fn a_foreign_agent_cannot_take_or_close_an_in_progress_task() {
        let mut board = TaskBoard::default();
        let id = board
            .create(agent("a"), task("x", 0, vec![]), t0())
            .unwrap()
            .id;
        board.pull(agent("b"), &[], t0()).unwrap();
        let err = board
            .update(agent("c"), id, TaskStatus::Done, None, false, t0())
            .unwrap_err();
        assert_eq!(
            err,
            TaskError::OwnedByOther {
                id,
                owner: agent("b"),
                since: t0()
            }
        );
        assert_eq!(board.get(id).unwrap().state.owner(), Some(&agent("b")));
        // The owner itself, and a forced caller, may change it.
        board
            .update(
                agent("b"),
                id,
                TaskStatus::Blocked,
                Some("api".into()),
                false,
                t0(),
            )
            .unwrap();
        board
            .update(agent("b"), id, TaskStatus::InProgress, None, false, t0())
            .unwrap();
        let forced = board
            .update(
                agent("c"),
                id,
                TaskStatus::Done,
                Some("taking over".into()),
                true,
                t0(),
            )
            .unwrap();
        assert_eq!(forced.state, TaskState::Done { by: agent("c") });
        let last = forced.notes.last().unwrap();
        assert_eq!(last.by, agent("c"));
        assert_eq!(
            last.text,
            "forced by c: was in progress under b. taking over"
        );
    }

    #[test]
    fn silent_owners_lose_their_tasks_and_dependents_become_pullable() {
        let mut board = TaskBoard::default();
        let first = board
            .create(agent("a"), task("first", 0, vec![]), t0())
            .unwrap()
            .id;
        let second = board
            .create(agent("a"), task("second", 0, vec![first]), t0())
            .unwrap()
            .id;
        board.pull(agent("dead"), &[], t0()).unwrap();
        board.pull(agent("alive"), &[], t0());
        let threshold = Duration::seconds(1800);
        let later = t0() + Duration::seconds(1801);
        // Nothing to reap while everyone is within the threshold.
        assert!(
            board
                .reap_orphans(|_| Some(t0()), threshold, t0())
                .is_empty()
        );
        // `alive` was seen recently, `dead` only at t0 (and holds `first`).
        let seen = |a: &AgentId| (a == &agent("alive")).then_some(later).or(Some(t0()));
        assert_eq!(board.reap_orphans(seen, threshold, later), vec![first]);
        let task = board.get(first).unwrap();
        assert_eq!(task.state, TaskState::Todo);
        assert_eq!(task.updated_at, later);
        assert_eq!(
            task.notes.last().unwrap().text,
            "returned to todo: owner dead silent since 2026-09-15T18:00:00Z"
        );
        // Not pullable until `first` is done; a live agent finishes it.
        assert!(!board.is_unblocked(board.get(second).unwrap()));
        board.pull(agent("alive"), &[], later).unwrap();
        board
            .update(agent("alive"), first, TaskStatus::Done, None, false, later)
            .unwrap();
        assert!(board.is_unblocked(board.get(second).unwrap()));
        assert_eq!(
            board.pull(agent("alive"), &[], later).unwrap().task.id,
            second
        );
        // An owner never seen counts as silent since the task last changed.
        let none = board.reap_orphans(|_| None, threshold, later + Duration::seconds(1));
        assert!(none.is_empty(), "alive's task changed at `later`");
        assert_eq!(
            board.reap_orphans(|_| None, threshold, later + Duration::seconds(1801)),
            vec![second]
        );
    }

    fn at(title: &str, priority: i32, paths: &[&str]) -> NewTask {
        NewTask::new(title)
            .with_priority(priority)
            .with_paths(paths.iter().map(|p| RepoPath::new(p).unwrap()).collect())
    }

    fn hold(path: &str, owner: &str) -> Hold {
        Hold {
            path: RepoPath::new(path).unwrap(),
            owner: agent(owner),
        }
    }

    #[test]
    fn pull_skips_a_task_under_another_agents_in_progress_task() {
        let mut board = TaskBoard::default();
        board
            .create(agent("a"), at("first", 9, &["src/state.rs"]), t0())
            .unwrap();
        board
            .create(agent("a"), at("same file", 8, &["src/state.rs"]), t0())
            .unwrap();
        let free = board
            .create(agent("a"), at("other file", 1, &["src/tasks.rs"]), t0())
            .unwrap()
            .id;
        board.pull(agent("b"), &[], t0()).unwrap();
        let pulled = board.pull(agent("c"), &[], t0()).unwrap();
        assert_eq!(pulled.task.id, free);
        assert!(pulled.waiting_on.is_empty());
    }

    #[test]
    fn pull_skips_held_paths_and_directories_but_not_pathless_tasks() {
        let mut board = TaskBoard::default();
        board
            .create(agent("a"), at("held", 9, &["src/claims.rs"]), t0())
            .unwrap();
        let pathless = board
            .create(agent("a"), at("anywhere", 5, &[]), t0())
            .unwrap()
            .id;
        let holds = [hold("src", "x")];
        assert_eq!(
            board.pull(agent("b"), &holds, t0()).unwrap().task.id,
            pathless
        );
    }

    #[test]
    fn anchored_holds_block_whole_file_tasks_but_not_sibling_anchors() {
        let mut board = TaskBoard::default();
        let whole = board
            .create(agent("a"), at("whole file", 9, &["app/config.py"]), t0())
            .unwrap()
            .id;
        let sibling = board
            .create(
                agent("a"),
                at("sibling", 5, &["app/config.py#Config::gzip"]),
                t0(),
            )
            .unwrap()
            .id;
        let holds = [hold("app/config.py#Config::from_env", "x")];
        let pulled = board.pull(agent("b"), &holds, t0()).unwrap();
        assert_eq!(pulled.task.id, sibling);
        assert!(pulled.waiting_on.is_empty());
        // The anchored claim still holds the whole-file task for a waiter.
        assert_eq!(
            board.pull_ready(agent("c"), &holds, t0()),
            Err(NotReady::Held)
        );
        assert_eq!(board.get(whole).unwrap().state, TaskState::Todo);
    }

    #[test]
    fn pull_hands_out_the_first_task_with_waiting_on_when_all_are_held() {
        let mut board = TaskBoard::default();
        let top = board
            .create(agent("a"), at("top", 9, &["src/state.rs", "docs"]), t0())
            .unwrap()
            .id;
        board
            .create(agent("a"), at("next", 1, &["src/state.rs"]), t0())
            .unwrap();
        let holds = [
            hold("src/state.rs", "x"),
            hold("src/state.rs", "x"),
            hold("docs/a.md", "y"),
        ];
        let pulled = board.pull(agent("b"), &holds, t0()).unwrap();
        assert_eq!(pulled.task.id, top);
        assert_eq!(
            pulled.waiting_on,
            vec![hold("docs/a.md", "y"), hold("src/state.rs", "x")]
        );
    }

    #[test]
    fn own_in_progress_tasks_do_not_hold_paths() {
        let mut board = TaskBoard::default();
        board
            .create(agent("a"), at("mine", 9, &["src/state.rs"]), t0())
            .unwrap();
        let next = board
            .create(agent("a"), at("also mine", 8, &["src/state.rs"]), t0())
            .unwrap()
            .id;
        board.pull(agent("b"), &[], t0()).unwrap();
        let pulled = board.pull(agent("b"), &[], t0()).unwrap();
        assert_eq!(pulled.task.id, next);
        assert!(pulled.waiting_on.is_empty());
    }

    #[test]
    fn a_task_behind_task_paths_beats_one_behind_a_claim() {
        let mut board = TaskBoard::default();
        board
            .create(agent("a"), at("taken", 10, &["src/hub.rs"]), t0())
            .unwrap();
        board
            .create(agent("a"), at("claimed", 9, &["src/claims.rs"]), t0())
            .unwrap();
        let behind_task = board
            .create(agent("a"), at("same hub", 5, &["src/hub.rs"]), t0())
            .unwrap()
            .id;
        board.pull(agent("x"), &[], t0()).unwrap();
        let claims = [hold("src/claims.rs", "y")];

        let pulled = board.pull_ready(agent("b"), &claims, t0()).unwrap();
        assert_eq!(pulled.task.id, behind_task);
        assert_eq!(pulled.waiting_on, vec![hold("src/hub.rs", "x")]);
    }

    #[test]
    fn a_free_task_beats_one_behind_task_paths() {
        let mut board = TaskBoard::default();
        board
            .create(agent("a"), at("taken", 10, &["src/hub.rs"]), t0())
            .unwrap();
        board
            .create(agent("a"), at("same hub", 9, &["src/hub.rs"]), t0())
            .unwrap();
        let free = board
            .create(agent("a"), at("elsewhere", 1, &["src/leaf.rs"]), t0())
            .unwrap()
            .id;
        board.pull(agent("x"), &[], t0()).unwrap();
        let pulled = board.pull_ready(agent("b"), &[], t0()).unwrap();
        assert_eq!(pulled.task.id, free);
        assert!(pulled.waiting_on.is_empty());
    }

    #[test]
    fn a_ready_pull_leaves_claimed_tasks_that_a_plain_pull_hands_out() {
        let mut board = TaskBoard::default();
        let top = board
            .create(agent("a"), at("top", 9, &["src/state.rs"]), t0())
            .unwrap()
            .id;
        board
            .create(agent("a"), at("taken", 10, &["src/state.rs"]), t0())
            .unwrap();
        board.pull(agent("x"), &[], t0()).unwrap();
        let claims = [hold("src/state.rs", "y")];

        assert_eq!(
            board.pull_ready(agent("b"), &claims, t0()),
            Err(NotReady::Held)
        );
        assert_eq!(board.get(top).unwrap().state, TaskState::Todo);
        // A plain pull hands it out anyway, with every hold it overlaps.
        let pulled = board.pull(agent("b"), &claims, t0()).unwrap();
        assert_eq!(pulled.task.id, top);
        assert_eq!(
            pulled.waiting_on,
            vec![hold("src/state.rs", "x"), hold("src/state.rs", "y")]
        );
    }

    #[test]
    fn a_ready_pull_waits_only_while_a_todo_task_is_left() {
        let mut board = TaskBoard::default();
        assert_eq!(
            board.pull_ready(agent("b"), &[], t0()),
            Err(NotReady::NoTodo)
        );

        let first = board
            .create(agent("a"), task("first", 0, vec![]), t0())
            .unwrap()
            .id;
        let second = board
            .create(agent("a"), task("second", 0, vec![first]), t0())
            .unwrap()
            .id;
        board.pull(agent("x"), &[], t0()).unwrap();
        // `second` waits on `first`: not a candidate, but worth waiting for.
        assert_eq!(board.pull_ready(agent("b"), &[], t0()), Err(NotReady::Held));
        board
            .update(agent("x"), first, TaskStatus::Done, None, false, t0())
            .unwrap();
        assert_eq!(
            board.pull_ready(agent("b"), &[], t0()).unwrap().task.id,
            second
        );
        // Everything is in progress or done: nothing left to wait for.
        assert_eq!(
            board.pull_ready(agent("c"), &[], t0()),
            Err(NotReady::NoTodo)
        );
    }
}
