//! The single owner of all mutable coordination state.
//!
//! [`State`] wraps every primitive behind one lock, reaps expired claims
//! before each access, renews the calling agent's leases on activity, and
//! tracks what changed since the last persist so the store writes only
//! that ([`Delta`]). Whole [`Snapshot`]s are for loading and the dashboard.
//! Domain rules live in the primitive modules; this module only sequences
//! them.
//!
//! Two bookkeeping helpers cover every primitive: [`LogCursor`] for
//! append-mostly JSON Lines files (notices, decisions) and [`ChangedIds`]
//! for one-file-per-item directories (contracts, memory notes). A new primitive picks
//! one, adds a field to [`Delta`], and is done. See ADR-0010.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::claims::{Claim, ClaimBook, ClaimError, DEFAULT_TTL_SECS, Granted, LostLease, Reaped};
use crate::clock::Clock;
use crate::contracts::{
    Contract, ContractError, ContractKind, ContractRegistry, NewContract, Published,
};
use crate::decisions::{Decision, DecisionError, DecisionLog, NewDecision};
use crate::memory::{MemoryBook, MemoryError, MemoryNote, MemorySearch, MemoryWritten, NewMemory};
use crate::messages::{
    BROADCAST_WINDOW, Inbox, Message, MessageBoard, MessageError, MessageFilter, NewMessage,
};
use crate::notices::{NewNotice, Notice, NoticeBoard, NoticeError, NoticeFilter, NoticeKind};
use crate::store::LoadError;
use crate::tasks::{NewTask, Task, TaskBoard, TaskError, TaskStatus};
use crate::types::{
    AgentId, ClaimId, ContractId, DecisionId, MemoryId, MessageId, NoticeId, Page, PrefixError,
    RepoPath, TaskId,
};

/// Everything worth persisting, as plain values.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    /// Monotonic counter, incremented each time a dirty snapshot is taken.
    pub seq: u64,
    /// Live claims.
    pub claims: Vec<Claim>,
    /// All tasks.
    pub tasks: Vec<Task>,
    /// All contracts.
    pub contracts: Vec<Contract>,
    /// All notices.
    pub notices: Vec<Notice>,
    /// All decisions.
    pub decisions: Vec<Decision>,
    /// All memory notes.
    pub memory: Vec<MemoryNote>,
    /// Agent-to-agent messages still within retention (runtime only).
    #[serde(default)]
    pub messages: Vec<Message>,
    /// Files and lines the store skipped while loading. Never written.
    #[serde(default)]
    pub load_errors: Vec<LoadError>,
}

/// How an append-mostly log changed since it was last persisted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Log<T> {
    /// Nothing to write.
    #[default]
    Unchanged,
    /// Only these items were added since the last persist; append them.
    Appended(Vec<T>),
    /// An item was edited in place; the whole log must be rewritten.
    Rewritten(Vec<T>),
}

impl<T> Log<T> {
    /// Whether there is nothing to write.
    pub fn is_unchanged(&self) -> bool {
        matches!(self, Self::Unchanged)
    }
}

/// Tracks how much of an append-mostly log is already on disk.
///
/// ```
/// use tirith::state::{Log, LogCursor};
///
/// let mut cursor = LogCursor::new(1);
/// assert_eq!(cursor.take(&["a", "b"]), Log::Appended(vec!["b"]));
/// assert_eq!(cursor.take(&["a", "b"]), Log::Unchanged);
/// cursor.mark_rewrite();
/// assert_eq!(cursor.take(&["a", "b"]), Log::Rewritten(vec!["a", "b"]));
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogCursor {
    persisted: usize,
    rewrite: bool,
}

impl LogCursor {
    /// A cursor for a log whose first `persisted` items are on disk.
    pub fn new(persisted: usize) -> Self {
        Self {
            persisted,
            rewrite: false,
        }
    }

    /// Records that an existing item changed, forcing a full rewrite.
    pub fn mark_rewrite(&mut self) {
        self.rewrite = true;
    }

    /// Whether a log of `len` items has anything unpersisted.
    pub fn is_dirty(&self, len: usize) -> bool {
        self.rewrite || len > self.persisted
    }

    /// What to write for `items`, and marks all of them persisted.
    pub fn take<T: Clone>(&mut self, items: &[T]) -> Log<T> {
        let log = if self.rewrite {
            Log::Rewritten(items.to_vec())
        } else if items.len() > self.persisted {
            Log::Appended(items[self.persisted..].to_vec())
        } else {
            Log::Unchanged
        };
        self.persisted = items.len();
        self.rewrite = false;
        log
    }
}

/// Tracks which items of a one-file-per-item primitive changed.
///
/// ```
/// use tirith::state::ChangedIds;
///
/// let mut changed = ChangedIds::default();
/// changed.mark(2);
/// let items = [(1, "a"), (2, "b")];
/// assert_eq!(changed.take(&items, |i| i.0), vec![(2, "b")]);
/// assert!(changed.is_empty());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedIds<I: Ord> {
    ids: BTreeSet<I>,
    removed: BTreeSet<I>,
    all: bool,
}

impl<I: Ord> Default for ChangedIds<I> {
    fn default() -> Self {
        Self {
            ids: BTreeSet::new(),
            removed: BTreeSet::new(),
            all: false,
        }
    }
}

impl<I: Ord> ChangedIds<I> {
    /// Records that the item with `id` must be written.
    pub fn mark(&mut self, id: I) {
        self.ids.insert(id);
    }

    /// Records that every item must be written.
    pub fn mark_all(&mut self) {
        self.all = true;
    }

    /// Records that the item with `id` was deleted and its file must go.
    pub fn remove(&mut self, id: I) {
        self.ids.remove(&id);
        self.removed.insert(id);
    }

    /// Whether nothing needs writing or deleting.
    pub fn is_empty(&self) -> bool {
        !self.all && self.ids.is_empty() && self.removed.is_empty()
    }

    /// The ids recorded as deleted since the last take, and clears them.
    pub fn take_removed(&mut self) -> Vec<I> {
        std::mem::take(&mut self.removed).into_iter().collect()
    }

    /// The changed items among `items`, and clears the record.
    pub fn take<T: Clone>(&mut self, items: &[T], id_of: impl Fn(&T) -> I) -> Vec<T> {
        let all = std::mem::take(&mut self.all);
        let ids = std::mem::take(&mut self.ids);
        items
            .iter()
            .filter(|item| all || ids.contains(&id_of(item)))
            .cloned()
            .collect()
    }
}

/// What changed since the last persist, ready to be written.
///
/// `None` and [`Log::Unchanged`] mean "leave that file alone".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Delta {
    /// Sequence number this delta brings the store to.
    pub seq: u64,
    /// All live claims, when any lease changed.
    pub claims: Option<Vec<Claim>>,
    /// All tasks, when any task changed.
    pub tasks: Option<Vec<Task>>,
    /// Only the contracts that changed.
    pub contracts: Vec<Contract>,
    /// Notice log changes.
    pub notices: Log<Notice>,
    /// Decision log changes.
    pub decisions: Log<Decision>,
    /// Only the memory notes that changed.
    pub memory: Vec<MemoryNote>,
    /// Memory notes deleted since the last persist; their files are removed.
    pub memory_removed: Vec<MemoryId>,
    /// Message log changes (runtime only).
    pub messages: Log<Message>,
}

impl Delta {
    /// A delta that rewrites everything in `snapshot`.
    pub fn full(snapshot: &Snapshot) -> Self {
        Self {
            seq: snapshot.seq,
            claims: Some(snapshot.claims.clone()),
            tasks: Some(snapshot.tasks.clone()),
            contracts: snapshot.contracts.clone(),
            notices: Log::Rewritten(snapshot.notices.clone()),
            decisions: Log::Rewritten(snapshot.decisions.clone()),
            memory: snapshot.memory.clone(),
            memory_removed: Vec::new(),
            messages: Log::Rewritten(snapshot.messages.clone()),
        }
    }
}

/// One agent's current footprint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSummary {
    /// The agent.
    pub agent: AgentId,
    /// Paths it currently holds.
    pub paths: Vec<RepoPath>,
    /// When its latest lease ends.
    pub expires_at: Option<DateTime<Utc>>,
    /// Tasks it has in progress.
    pub tasks_in_progress: usize,
}

/// Counts and per-agent summaries for `status` and the dashboard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusReport {
    /// When the state was created (daemon start).
    pub started_at: DateTime<Utc>,
    /// The clock's reading when the report was built.
    pub now: DateTime<Utc>,
    /// Seconds since start.
    pub uptime_secs: u64,
    /// Current snapshot sequence number.
    pub seq: u64,
    /// Live claims.
    pub claims: usize,
    /// Tasks not yet done.
    pub tasks_open: usize,
    /// Tasks done.
    pub tasks_done: usize,
    /// Contracts.
    pub contracts: usize,
    /// Notices.
    pub notices: usize,
    /// Decisions.
    pub decisions: usize,
    /// Memory notes.
    pub memory: usize,
    /// Tasks returned to `todo` because their owner went silent.
    pub tasks_orphaned: usize,
    /// Agents with live claims or tasks in progress.
    pub agents: Vec<AgentSummary>,
    /// Files and lines the store skipped at startup.
    pub load_errors: Vec<LoadError>,
}

/// Default for [`State::set_task_orphan_secs`]: 30 minutes.
pub const DEFAULT_TASK_ORPHAN_SECS: u64 = 1800;

/// Rows shown per section of a claim brief (ADR-0014).
pub const BRIEF_LIMIT: usize = 5;

/// What a successful claim carries back about the claimed paths: one
/// capped section per primitive, newest first. `more` counts the matching
/// rows that were left out, so the caller knows whether to page with the
/// list tools (ADR-0014).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Brief {
    /// Unread notices not yet shown to the agent in a brief.
    pub notices: Vec<Notice>,
    /// Contracts consumed by the claimed paths.
    pub contracts: Vec<Contract>,
    /// Decisions affecting the claimed paths.
    pub decisions: Vec<Decision>,
    /// Memory notes about the claimed paths.
    pub memory: Vec<MemoryNote>,
    /// Rows left out of each section.
    pub more: BriefMore,
}

/// Per-section counts of the rows a [`Brief`] left out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct BriefMore {
    /// Unread, undelivered notices beyond [`BRIEF_LIMIT`].
    pub notices: usize,
    /// Contracts beyond [`BRIEF_LIMIT`].
    pub contracts: usize,
    /// Decisions beyond [`BRIEF_LIMIT`].
    pub decisions: usize,
    /// Memory notes beyond [`BRIEF_LIMIT`].
    pub memory: usize,
}

/// Who held a path until its lease ended, told to the agent that claims
/// it soon after (contract "Lost lease reporting", ADR-0015).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PreviousOwner {
    /// The path just claimed.
    pub path: RepoPath,
    /// The agent whose lease on an overlapping path ended.
    pub owner: AgentId,
    /// When that lease ended.
    pub reaped_at: DateTime<Utc>,
}

#[derive(Debug)]
struct Inner {
    claims: ClaimBook,
    tasks: TaskBoard,
    contracts: ContractRegistry,
    notices: NoticeBoard,
    decisions: DecisionLog,
    /// Behind an `Arc` so a read can take a pointer clone, release the
    /// lock, and score outside it. Writes use `Arc::make_mut`, which copies
    /// only while a reader still holds the previous version.
    memory: Arc<MemoryBook>,
    seq: u64,
    claims_dirty: bool,
    tasks_dirty: bool,
    contracts_changed: ChangedIds<ContractId>,
    memory_changed: ChangedIds<MemoryId>,
    notices_log: LogCursor,
    decisions_log: LogCursor,
    /// Agent-to-agent messages and who has received what (ADR-0020).
    messages: MessageBoard,
    messages_log: LogCursor,
    /// Leases were renewed. Not a real change; folded into the next
    /// claims write instead of forcing one.
    touched: bool,
    /// When each agent last called anything, for orphan reaping. Not
    /// persisted: after a restart every owner starts silent from its
    /// task's last change.
    last_seen: BTreeMap<AgentId, DateTime<Utc>>,
    /// How long an in-progress task's owner may be silent before the task
    /// returns to `todo`.
    task_orphan: Duration,
    /// Tasks returned to `todo` from silent owners since start.
    tasks_orphaned: usize,
    /// Leases that ended recently: reported once to their owner, and
    /// visible to whoever claims the path next (ADR-0015). Not persisted.
    reaped: Reaped,
    /// Notices already shown to each agent in a brief, for the daemon's
    /// lifetime. Not an acknowledgement and never persisted (ADR-0014).
    delivered: BTreeMap<AgentId, BTreeSet<NoticeId>>,
}

impl Inner {
    /// Whether a mutation other than a lease renewal is unpersisted.
    fn is_dirty(&self) -> bool {
        self.claims_dirty
            || self.tasks_dirty
            || !self.contracts_changed.is_empty()
            || self.notices_log.is_dirty(self.notices.notices().len())
            || self
                .decisions_log
                .is_dirty(self.decisions.decisions().len())
            || !self.memory_changed.is_empty()
            || self.messages_log.is_dirty(self.messages.messages().len())
    }
}

/// Shared, lock-protected coordination state.
#[derive(Debug)]
pub struct State {
    inner: Mutex<Inner>,
    clock: Arc<dyn Clock>,
    started_at: DateTime<Utc>,
    load_errors: Vec<LoadError>,
}

impl State {
    /// Builds state from a persisted snapshot and a clock.
    pub fn new(clock: Arc<dyn Clock>, snapshot: Snapshot) -> Self {
        let started_at = clock.now();
        let load_errors = snapshot.load_errors.clone();
        // Messages past retention are dropped here; the file is rewritten
        // to match on the next persist.
        let loaded_messages = snapshot.messages.len();
        let messages = MessageBoard::from_messages(snapshot.messages, started_at);
        let mut messages_log = LogCursor::new(messages.messages().len());
        if messages.messages().len() != loaded_messages {
            messages_log.mark_rewrite();
        }
        Self {
            inner: Mutex::new(Inner {
                claims: ClaimBook::from_claims(snapshot.claims),
                tasks: TaskBoard::from_tasks(snapshot.tasks),
                contracts: ContractRegistry::from_contracts(snapshot.contracts),
                notices_log: LogCursor::new(snapshot.notices.len()),
                notices: NoticeBoard::from_notices(snapshot.notices),
                decisions_log: LogCursor::new(snapshot.decisions.len()),
                decisions: DecisionLog::from_decisions(snapshot.decisions),
                memory: Arc::new(MemoryBook::from_notes(snapshot.memory)),
                messages,
                messages_log,
                seq: snapshot.seq,
                claims_dirty: false,
                tasks_dirty: false,
                contracts_changed: ChangedIds::default(),
                memory_changed: ChangedIds::default(),
                touched: false,
                last_seen: BTreeMap::new(),
                task_orphan: Duration::seconds(
                    i64::try_from(DEFAULT_TASK_ORPHAN_SECS).unwrap_or(i64::MAX),
                ),
                tasks_orphaned: 0,
                reaped: Reaped::default(),
                delivered: BTreeMap::new(),
            }),
            clock,
            started_at,
            load_errors,
        }
    }

    /// Sets how long an in-progress task's owner may be silent before the
    /// task returns to `todo`. Zero disables reaping.
    pub fn set_task_orphan_secs(&self, secs: u64) {
        self.lock().task_orphan = Duration::seconds(i64::try_from(secs).unwrap_or(i64::MAX));
    }

    /// Files and lines the store skipped when this state was loaded.
    /// Empty after a clean load.
    pub fn load_errors(&self) -> &[LoadError] {
        &self.load_errors
    }

    /// The clock's current reading.
    pub fn now(&self) -> DateTime<Utc> {
        self.clock.now()
    }

    /// When this state was created.
    pub fn started_at(&self) -> DateTime<Utc> {
        self.started_at
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Runs `f` with the lock held after reaping expired claims and
    /// renewing `agent`'s leases. `f` marks what it changed.
    fn access<R>(
        &self,
        agent: Option<&AgentId>,
        f: impl FnOnce(&mut Inner, DateTime<Utc>) -> R,
    ) -> R {
        let now = self.clock.now();
        let mut inner = self.lock();
        let expired = inner.claims.reap(now);
        if !expired.is_empty() {
            inner.reaped.record(&expired, now);
            inner.claims_dirty = true;
        }
        if let Some(agent) = agent {
            if inner.claims.touch(agent, now) > 0 {
                inner.touched = true;
            }
            inner.last_seen.insert(agent.clone(), now);
        }
        if inner.task_orphan > Duration::zero() {
            let Inner {
                tasks,
                last_seen,
                task_orphan,
                ..
            } = &mut *inner;
            let reaped = tasks.reap_orphans(|a| last_seen.get(a).copied(), *task_orphan, now);
            if !reaped.is_empty() {
                inner.tasks_orphaned += reaped.len();
                inner.tasks_dirty = true;
            }
        }
        f(&mut inner, now)
    }

    /// A snapshot of everything, without touching the dirty flags.
    pub fn snapshot(&self) -> Snapshot {
        self.access(None, |inner, _| snapshot_of(inner))
    }

    /// The sequence number of the last delta taken (or loaded).
    pub fn seq(&self) -> u64 {
        self.lock().seq
    }

    /// Whether a mutation other than a lease renewal is waiting to be
    /// persisted.
    pub fn is_dirty(&self) -> bool {
        self.lock().is_dirty()
    }

    /// The sequence number the store must reach to hold everything that
    /// is dirty now. With `include_renewals`, pending lease renewals count
    /// too (used at shutdown).
    pub fn persist_target(&self, include_renewals: bool) -> u64 {
        let inner = self.lock();
        let pending = inner.is_dirty() || (include_renewals && inner.touched);
        inner.seq + u64::from(pending)
    }

    /// If anything changed since the last call, bumps `seq` and returns
    /// what to write. Otherwise `None`. Lease renewals alone also produce
    /// a delta, so calling this on a timer keeps leases fresh on disk
    /// without putting a write on every request.
    pub fn take_dirty(&self) -> Option<Delta> {
        let mut guard = self.lock();
        let inner = &mut *guard;
        if !inner.is_dirty() && !inner.touched {
            return None;
        }
        inner.seq += 1;
        let claims = (inner.claims_dirty || inner.touched).then(|| inner.claims.claims().to_vec());
        let tasks = inner.tasks_dirty.then(|| inner.tasks.tasks().to_vec());
        inner.claims_dirty = false;
        inner.tasks_dirty = false;
        inner.touched = false;
        Some(Delta {
            seq: inner.seq,
            claims,
            tasks,
            contracts: inner
                .contracts_changed
                .take(inner.contracts.contracts(), |c| c.id),
            notices: inner.notices_log.take(inner.notices.notices()),
            decisions: inner.decisions_log.take(inner.decisions.decisions()),
            memory: inner.memory_changed.take(inner.memory.notes(), |n| n.id),
            memory_removed: inner.memory_changed.take_removed(),
            messages: inner.messages_log.take(inner.messages.messages()),
        })
    }

    /// Marks everything dirty so the next delta rewrites every file. The
    /// persister calls this after a failed write.
    pub fn mark_all_dirty(&self) {
        let mut inner = self.lock();
        inner.claims_dirty = true;
        inner.tasks_dirty = true;
        inner.contracts_changed.mark_all();
        inner.notices_log.mark_rewrite();
        inner.decisions_log.mark_rewrite();
        inner.memory_changed.mark_all();
        inner.messages_log.mark_rewrite();
    }

    /// Sends a message from `agent`. A broadcast (`to` = `*`) is addressed
    /// to every agent seen within the last hour, minus the sender.
    pub fn message_send(&self, agent: AgentId, new: NewMessage) -> Result<Message, MessageError> {
        self.access(Some(&agent.clone()), |inner, now| {
            let recent: Vec<AgentId> = inner
                .last_seen
                .iter()
                .filter(|(_, seen)| now - **seen <= BROADCAST_WINDOW)
                .map(|(a, _)| a.clone())
                .collect();
            inner.messages.send(agent, new, recent, now).cloned()
        })
    }

    /// Newest `limit` messages `agent` sent or received, matching
    /// `filter`, older than the cursor `before`.
    pub fn messages_page(
        &self,
        agent: &AgentId,
        filter: &MessageFilter,
        before: Option<&(DateTime<Utc>, MessageId)>,
        limit: usize,
    ) -> Page<Message> {
        self.access(Some(agent), |inner, _| {
            inner
                .messages
                .list_page(agent, filter, before, limit)
                .map(Clone::clone)
        })
    }

    /// The messages waiting for `agent`, delivered once. Called by the
    /// result assembly for every call, so it does not renew leases.
    pub fn take_inbox(&self, agent: &AgentId) -> Inbox {
        self.access(None, |inner, _| inner.messages.take_inbox(agent))
    }

    /// Resolves a message id or unique prefix.
    pub fn resolve_message(&self, raw: &str) -> Result<MessageId, PrefixError> {
        self.access(None, |inner, _| inner.messages.resolve_id(raw))
    }

    /// Claims paths for `agent`. See [`ClaimBook::claim`].
    pub fn claim(
        &self,
        agent: AgentId,
        paths: Vec<RepoPath>,
        reason: String,
        ttl_secs: Option<u64>,
    ) -> Result<Granted, ClaimError> {
        let ttl = ttl_secs.unwrap_or(DEFAULT_TTL_SECS);
        self.access(Some(&agent.clone()), |inner, now| {
            let granted = inner.claims.claim(agent, paths, reason, ttl, now)?;
            inner.claims_dirty = true;
            Ok(granted)
        })
    }

    /// Releases paths. See [`ClaimBook::release`].
    pub fn release(
        &self,
        agent: &AgentId,
        paths: Option<Vec<RepoPath>>,
    ) -> Result<Vec<RepoPath>, ClaimError> {
        self.access(Some(agent), |inner, _| {
            let released = inner.claims.release(agent, paths)?;
            inner.claims_dirty = true;
            Ok(released)
        })
    }

    /// Renews all leases held by `agent`.
    pub fn renew(&self, agent: &AgentId) -> Result<Vec<Claim>, ClaimError> {
        self.access(None, |inner, now| {
            let renewed = inner.claims.renew(agent, now)?;
            inner.claims_dirty = true;
            Ok(renewed)
        })
    }

    /// Live claims, optionally only those overlapping `path`.
    pub fn claims(&self, agent: Option<&AgentId>, path: Option<&RepoPath>) -> Vec<Claim> {
        self.access(agent, |inner, _| {
            inner.claims.list(path).into_iter().cloned().collect()
        })
    }

    /// Creates a task.
    pub fn task_create(&self, agent: AgentId, new: NewTask) -> Result<Task, TaskError> {
        self.access(Some(&agent.clone()), |inner, now| {
            let task = inner.tasks.create(agent, new, now)?.clone();
            inner.tasks_dirty = true;
            Ok(task)
        })
    }

    /// Pulls the next unblocked task for `agent`.
    pub fn task_pull(&self, agent: AgentId) -> Option<Task> {
        self.access(Some(&agent.clone()), |inner, now| {
            let task = inner.tasks.pull(agent, now).cloned();
            if task.is_some() {
                inner.tasks_dirty = true;
            }
            task
        })
    }

    /// Updates a task's status.
    pub fn task_update(
        &self,
        agent: AgentId,
        id: TaskId,
        status: TaskStatus,
        note: Option<String>,
        force: bool,
    ) -> Result<Task, TaskError> {
        self.access(Some(&agent.clone()), |inner, now| {
            let task = inner
                .tasks
                .update(agent, id, status, note, force, now)?
                .clone();
            inner.tasks_dirty = true;
            Ok(task)
        })
    }

    /// Tasks matching the filters.
    pub fn tasks(
        &self,
        agent: Option<&AgentId>,
        status: Option<TaskStatus>,
        owner: Option<&AgentId>,
    ) -> Vec<Task> {
        self.access(agent, |inner, _| {
            inner
                .tasks
                .list(status, owner)
                .into_iter()
                .cloned()
                .collect()
        })
    }

    /// Publishes a contract. A new version of an existing contract also
    /// publishes a `contract` notice to its consumers; that notice is
    /// returned alongside.
    pub fn contract_publish(
        &self,
        agent: AgentId,
        new: NewContract,
    ) -> Result<(Published, Option<Notice>), ContractError> {
        self.access(Some(&agent.clone()), |inner, now| {
            let published = inner.contracts.publish(agent.clone(), new, now)?;
            inner.contracts_changed.mark(published.contract.id);
            let notice = published.previous_version.and_then(|previous| {
                let contract = &published.contract;
                let notice = NewNotice {
                    kind: NoticeKind::Contract,
                    summary: format!(
                        "contract {} updated to v{} (was v{previous})",
                        contract.name, contract.current.version
                    ),
                    from: Some(format!("v{previous}")),
                    to: Some(format!("v{}", contract.current.version)),
                    affected_paths: published.notify.clone(),
                    contract_id: Some(contract.id),
                };
                // The summary above is never empty, so this cannot fail.
                inner.notices.publish(agent, notice, now).ok().cloned()
            });
            Ok((published, notice))
        })
    }

    /// Looks up a contract by name or id.
    pub fn contract_get(&self, agent: Option<&AgentId>, name_or_id: &str) -> Option<Contract> {
        self.access(agent, |inner, _| inner.contracts.get(name_or_id).cloned())
    }

    /// Contracts matching the filters.
    pub fn contracts(
        &self,
        agent: Option<&AgentId>,
        path: Option<&RepoPath>,
        kind: Option<ContractKind>,
    ) -> Vec<Contract> {
        self.access(agent, |inner, _| {
            inner
                .contracts
                .list(path, kind)
                .into_iter()
                .cloned()
                .collect()
        })
    }

    /// Publishes a notice.
    pub fn notice_publish(&self, agent: AgentId, new: NewNotice) -> Result<Notice, NoticeError> {
        self.access(Some(&agent.clone()), |inner, now| {
            inner.notices.publish(agent, new, now).cloned()
        })
    }

    /// Notices matching `filter`.
    pub fn notices(&self, agent: Option<&AgentId>, filter: &NoticeFilter) -> Vec<Notice> {
        self.access(agent, |inner, _| {
            inner.notices.list(filter).into_iter().cloned().collect()
        })
    }

    /// Acknowledges a notice.
    pub fn notice_ack(&self, agent: AgentId, id: NoticeId) -> Result<Notice, NoticeError> {
        self.access(Some(&agent.clone()), |inner, _| {
            let notice = inner.notices.ack(agent, id)?.clone();
            inner.notices_log.mark_rewrite();
            Ok(notice)
        })
    }

    /// Records a decision.
    pub fn decision_record(
        &self,
        agent: AgentId,
        new: NewDecision,
    ) -> Result<Decision, DecisionError> {
        self.access(Some(&agent.clone()), |inner, now| {
            inner.decisions.record(agent, new, now).cloned()
        })
    }

    /// Decisions matching the filters.
    pub fn decisions(
        &self,
        agent: Option<&AgentId>,
        path: Option<&RepoPath>,
        query: Option<&str>,
    ) -> Vec<Decision> {
        self.access(agent, |inner, _| {
            inner
                .decisions
                .list(path, query)
                .into_iter()
                .cloned()
                .collect()
        })
    }

    /// Writes a memory note. See [`MemoryBook::write`].
    pub fn memory_write(
        &self,
        agent: AgentId,
        new: NewMemory,
    ) -> Result<MemoryWritten, MemoryError> {
        self.access(Some(&agent.clone()), |inner, now| {
            let written = Arc::make_mut(&mut inner.memory).write(agent, new, now)?;
            inner.memory_changed.mark(written.note.id);
            Ok(written)
        })
    }

    /// Removes a note by permalink, id, or exact title, and schedules its
    /// file for deletion.
    ///
    /// Returns the removed note, or `None` when nothing matched. A note
    /// holding a secret or a wrong fact has to be retractable through the
    /// same path that wrote it; deleting the file by hand is undone by the
    /// next full rewrite.
    pub fn memory_delete(&self, agent: &AgentId, name: &str) -> Option<MemoryNote> {
        self.access(Some(agent), |inner, _| {
            let removed = Arc::make_mut(&mut inner.memory).remove(name)?;
            inner.memory_changed.remove(removed.id);
            Some(removed)
        })
    }

    /// The current book, as a pointer clone taken under the lock.
    ///
    /// Reads go through this so that scoring, relation walks, and path
    /// scans happen after the lock is released. The caller's leases are
    /// still renewed, because `access` does that before handing back the
    /// pointer. A writer that arrives meanwhile pays one copy of the book
    /// and moves on; the reader keeps scoring the version it took.
    fn memory_book(&self, agent: Option<&AgentId>) -> Arc<MemoryBook> {
        self.access(agent, |inner, _| Arc::clone(&inner.memory))
    }

    /// A note by permalink, id, or exact title, with the notes reachable
    /// from it within `depth` relation hops.
    ///
    /// A read, so it never marks anything for writing.
    pub fn memory_get(
        &self,
        agent: Option<&AgentId>,
        name: &str,
        depth: usize,
    ) -> Option<(MemoryNote, Vec<MemoryNote>)> {
        let book = self.memory_book(agent);
        let note = book.get(name)?.clone();
        let related = if depth == 0 {
            Vec::new()
        } else {
            book.context(name, depth)
                .into_iter()
                .skip(1)
                .cloned()
                .collect()
        };
        Some((note, related))
    }

    /// Notes matching a search, best first, each with its score.
    ///
    /// A read, so it never marks anything for writing.
    pub fn memory_search(
        &self,
        agent: Option<&AgentId>,
        filter: &MemorySearch,
    ) -> Vec<(MemoryNote, u32)> {
        self.memory_book(agent)
            .search(filter)
            .into_iter()
            .map(|hit| (hit.note.clone(), hit.score))
            .collect()
    }

    /// The brief a successful claim carries back (ADR-0014): the newest
    /// unread and undelivered notices, contracts, decisions and memory
    /// notes touching `paths`, each capped at [`BRIEF_LIMIT`].
    ///
    /// Notices returned are marked delivered to `agent` for the daemon's
    /// lifetime, so the next brief on the same paths shows the next ones
    /// instead of repeating these. Delivery is not an acknowledgement and
    /// is never persisted.
    pub fn brief(&self, agent: &AgentId, paths: &[RepoPath]) -> Brief {
        self.access(None, |inner, _| {
            let shown = inner.delivered.get(agent);
            let mut notices = newest(
                paths,
                |path| {
                    let filter = NoticeFilter {
                        path: Some(path.clone()),
                        since: None,
                        unread_by: Some(agent.clone()),
                    };
                    inner.notices.list(&filter)
                },
                |n| n.id,
                |n| n.published_at,
            );
            notices.retain(|n| !shown.is_some_and(|s| s.contains(&n.id)));
            let contracts = newest(
                paths,
                |path| inner.contracts.list(Some(path), None),
                |c| c.id,
                |c| c.current.published_at,
            );
            let decisions = newest(
                paths,
                |path| inner.decisions.list(Some(path), None),
                |d| d.id,
                |d| d.recorded_at,
            );
            let memory = newest(
                paths,
                |path| inner.memory.for_path(path, None),
                |m| m.id,
                |m| m.updated_at,
            );
            let more = BriefMore {
                notices: notices.len().saturating_sub(BRIEF_LIMIT),
                contracts: contracts.len().saturating_sub(BRIEF_LIMIT),
                decisions: decisions.len().saturating_sub(BRIEF_LIMIT),
                memory: memory.len().saturating_sub(BRIEF_LIMIT),
            };
            let brief = Brief {
                notices: capped(notices),
                contracts: capped(contracts),
                decisions: capped(decisions),
                memory: capped(memory),
                more,
            };
            inner
                .delivered
                .entry(agent.clone())
                .or_default()
                .extend(brief.notices.iter().map(|n| n.id));
            brief
        })
    }

    /// Leases `agent` lost since it was last told. Each is returned once;
    /// the server attaches them to whatever response comes next.
    pub fn take_lost(&self, agent: &AgentId) -> Vec<LostLease> {
        self.access(None, |inner, _| inner.reaped.take_for(agent))
    }

    /// For each of `paths`, who held an overlapping path until recently,
    /// if anyone did.
    pub fn previous_owners(&self, paths: &[RepoPath]) -> Vec<PreviousOwner> {
        self.access(None, |inner, _| {
            paths
                .iter()
                .filter_map(|path| {
                    inner.reaped.previous_owner(path).map(|lost| PreviousOwner {
                        path: path.clone(),
                        owner: lost.owner.clone(),
                        reaped_at: lost.at,
                    })
                })
                .collect()
        })
    }

    /// Newest live claims, older than `before`, at most `limit`. With
    /// `mine`, only that agent's claims plus any claim overlapping `path`;
    /// without it, every claim, or only those overlapping `path`.
    pub fn claims_page(
        &self,
        agent: Option<&AgentId>,
        mine: Option<&AgentId>,
        path: Option<&RepoPath>,
        before: Option<&(DateTime<Utc>, ClaimId)>,
        limit: usize,
    ) -> Page<Claim> {
        self.access(agent, |inner, _| {
            let rows = inner.claims.claims().iter().filter(|c| match mine {
                None => path.is_none_or(|p| c.covers(p)),
                Some(me) => &c.owner == me || path.is_some_and(|p| c.covers(p)),
            });
            Page::newest_first(rows, |c| (c.claimed_at, c.id), before, limit).map(Clone::clone)
        })
    }

    /// Paths `agent` currently holds, across all its claims.
    pub fn held_paths(&self, agent: &AgentId) -> Vec<RepoPath> {
        self.access(Some(agent), |inner, _| {
            inner
                .claims
                .claims()
                .iter()
                .filter(|c| &c.owner == agent)
                .flat_map(|c| c.paths.iter().cloned())
                .collect()
        })
    }

    /// Most recently updated tasks matching the filters. See
    /// [`TaskBoard::list_page`].
    pub fn tasks_page(
        &self,
        agent: Option<&AgentId>,
        status: Option<TaskStatus>,
        owner: Option<&AgentId>,
        before: Option<&(DateTime<Utc>, TaskId)>,
        limit: usize,
    ) -> Page<Task> {
        self.access(agent, |inner, _| {
            inner
                .tasks
                .list_page(status, owner, before, limit)
                .map(Clone::clone)
        })
    }

    /// Resolves a task id or unique prefix.
    pub fn resolve_task(&self, raw: &str) -> Result<TaskId, PrefixError> {
        self.lock().tasks.resolve_id(raw)
    }

    /// Most recently published contracts matching the filters. See
    /// [`ContractRegistry::list_page`].
    pub fn contracts_page(
        &self,
        agent: Option<&AgentId>,
        path: Option<&RepoPath>,
        kind: Option<ContractKind>,
        before: Option<&(DateTime<Utc>, ContractId)>,
        limit: usize,
    ) -> Page<Contract> {
        self.access(agent, |inner, _| {
            inner
                .contracts
                .list_page(path, kind, before, limit)
                .map(Clone::clone)
        })
    }

    /// Newest notices matching `filter` and, when `paths` is not empty,
    /// affecting at least one of them. See [`NoticeBoard::list_page`].
    pub fn notices_page(
        &self,
        agent: Option<&AgentId>,
        filter: &NoticeFilter,
        paths: &[RepoPath],
        before: Option<&(DateTime<Utc>, NoticeId)>,
        limit: usize,
    ) -> Page<Notice> {
        self.access(agent, |inner, _| {
            let rows = inner
                .notices
                .list(filter)
                .into_iter()
                .filter(|n| paths.is_empty() || paths.iter().any(|p| n.affects(p)));
            Page::newest_first(rows, |n| (n.published_at, n.id), before, limit).map(Clone::clone)
        })
    }

    /// Resolves a notice id or unique prefix.
    pub fn resolve_notice(&self, raw: &str) -> Result<NoticeId, PrefixError> {
        self.lock().notices.resolve_id(raw)
    }

    /// Newest decisions matching the filters. See [`DecisionLog::list_page`].
    pub fn decisions_page(
        &self,
        agent: Option<&AgentId>,
        path: Option<&RepoPath>,
        query: Option<&str>,
        before: Option<&(DateTime<Utc>, DecisionId)>,
        limit: usize,
    ) -> Page<Decision> {
        self.access(agent, |inner, _| {
            inner
                .decisions
                .list_page(path, query, before, limit)
                .map(Clone::clone)
        })
    }

    /// Counts and per-agent summaries.
    pub fn status(&self, agent: Option<&AgentId>) -> StatusReport {
        self.access(agent, |inner, now| {
            let mut agents: Vec<AgentSummary> = Vec::new();
            for claim in inner.claims.claims() {
                let entry = summary_for(&mut agents, &claim.owner);
                entry.paths.extend(claim.paths.iter().cloned());
                entry.expires_at = Some(
                    entry
                        .expires_at
                        .map_or(claim.expires_at, |e| e.max(claim.expires_at)),
                );
            }
            for task in inner.tasks.list(Some(TaskStatus::InProgress), None) {
                if let Some(owner) = task.state.owner() {
                    summary_for(&mut agents, owner).tasks_in_progress += 1;
                }
            }
            agents.sort_by(|a, b| a.agent.cmp(&b.agent));
            let tasks_done = inner.tasks.list(Some(TaskStatus::Done), None).len();
            StatusReport {
                started_at: self.started_at,
                now,
                uptime_secs: u64::try_from((now - self.started_at).num_seconds()).unwrap_or(0),
                seq: inner.seq,
                claims: inner.claims.claims().len(),
                tasks_open: inner.tasks.tasks().len() - tasks_done,
                tasks_done,
                contracts: inner.contracts.contracts().len(),
                notices: inner.notices.notices().len(),
                decisions: inner.decisions.decisions().len(),
                memory: inner.memory.notes().len(),
                tasks_orphaned: inner.tasks_orphaned,
                agents,
                load_errors: self.load_errors.clone(),
            }
        })
    }
}

fn summary_for<'a>(agents: &'a mut Vec<AgentSummary>, agent: &AgentId) -> &'a mut AgentSummary {
    let index = agents
        .iter()
        .position(|a| &a.agent == agent)
        .unwrap_or_else(|| {
            agents.push(AgentSummary {
                agent: agent.clone(),
                paths: Vec::new(),
                expires_at: None,
                tasks_in_progress: 0,
            });
            agents.len() - 1
        });
    &mut agents[index]
}

fn snapshot_of(inner: &Inner) -> Snapshot {
    Snapshot {
        seq: inner.seq,
        claims: inner.claims.claims().to_vec(),
        tasks: inner.tasks.tasks().to_vec(),
        contracts: inner.contracts.contracts().to_vec(),
        notices: inner.notices.notices().to_vec(),
        decisions: inner.decisions.decisions().to_vec(),
        memory: inner.memory.notes().to_vec(),
        messages: inner.messages.messages().to_vec(),
        load_errors: Vec::new(),
    }
}

/// Rows from `list` for every path, deduplicated by `id`, newest first.
fn newest<'a, T, I, K>(
    paths: &[RepoPath],
    list: impl Fn(&RepoPath) -> Vec<&'a T>,
    id: impl Fn(&T) -> I,
    key: impl Fn(&T) -> K,
) -> Vec<&'a T>
where
    I: Ord,
    K: Ord,
{
    let mut seen = BTreeSet::new();
    let mut rows: Vec<&'a T> = Vec::new();
    for path in paths {
        for row in list(path) {
            if seen.insert(id(row)) {
                rows.push(row);
            }
        }
    }
    rows.sort_by_key(|row| std::cmp::Reverse(key(row)));
    rows
}

/// The first [`BRIEF_LIMIT`] rows, owned.
fn capped<T: Clone>(rows: Vec<&T>) -> Vec<T> {
    rows.into_iter().take(BRIEF_LIMIT).cloned().collect()
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};
    use serde_json::json;

    use super::*;
    use crate::clock::ManualClock;

    fn agent(name: &str) -> AgentId {
        AgentId::new(name).unwrap()
    }

    fn path(p: &str) -> RepoPath {
        RepoPath::new(p).unwrap()
    }

    fn state() -> (Arc<ManualClock>, State) {
        let clock = Arc::new(ManualClock::new(
            Utc.with_ymd_and_hms(2026, 9, 15, 18, 0, 0).unwrap(),
        ));
        let state = State::new(clock.clone(), Snapshot::default());
        (clock, state)
    }

    #[test]
    fn leases_expire_by_clock_and_activity_renews_them() {
        let (clock, state) = state();
        state
            .claim(agent("alice"), vec![path("a")], "x".into(), Some(60))
            .unwrap();
        clock.advance(Duration::seconds(50));
        assert_eq!(
            state.claims(Some(&agent("alice")), None).len(),
            1,
            "listing renews"
        );
        clock.advance(Duration::seconds(50));
        assert_eq!(
            state.claims(None, None).len(),
            1,
            "renewed at t=50, lives to t=110"
        );
        clock.advance(Duration::seconds(20));
        assert!(state.claims(None, None).is_empty(), "expired at t=110");
        assert!(
            state
                .claim(agent("bob"), vec![path("a")], "y".into(), None)
                .is_ok()
        );
    }

    #[test]
    fn deltas_carry_only_what_changed() {
        let (_, state) = state();
        assert!(state.take_dirty().is_none());
        state
            .claim(agent("alice"), vec![path("a")], "x".into(), None)
            .unwrap();
        assert!(state.is_dirty());
        assert_eq!(state.persist_target(false), 1);
        let delta = state.take_dirty().unwrap();
        assert_eq!(delta.seq, 1);
        assert_eq!(delta.claims.map(|c| c.len()), Some(1));
        assert!(delta.tasks.is_none(), "tasks untouched");
        assert!(delta.contracts.is_empty());
        assert!(delta.notices.is_unchanged());
        assert!(!state.is_dirty());
        assert_eq!(state.persist_target(false), 1);
        assert!(state.take_dirty().is_none());

        state
            .notice_publish(
                agent("alice"),
                NewNotice {
                    kind: NoticeKind::Rename,
                    summary: "first".into(),
                    from: None,
                    to: None,
                    affected_paths: vec![path("b")],
                    contract_id: None,
                },
            )
            .unwrap();
        let second = state
            .notice_publish(
                agent("alice"),
                NewNotice {
                    kind: NoticeKind::Rename,
                    summary: "second".into(),
                    from: None,
                    to: None,
                    affected_paths: vec![path("b")],
                    contract_id: None,
                },
            )
            .unwrap();
        let delta = state.take_dirty().unwrap();
        assert!(matches!(delta.notices, Log::Appended(ref n) if n.len() == 2));
        assert!(
            delta.claims.is_some(),
            "alice's publish renewed her lease; the renewal rides along"
        );

        state.notice_ack(agent("bob"), second.id).unwrap();
        let delta = state.take_dirty().unwrap();
        assert!(
            matches!(delta.notices, Log::Rewritten(ref n) if n.len() == 2),
            "an in-place edit rewrites the log"
        );
        assert!(
            delta.claims.is_none(),
            "bob holds nothing, so nothing was renewed"
        );
    }

    #[test]
    fn renewals_are_not_dirty_but_reach_the_next_delta() {
        let (clock, state) = state();
        state
            .claim(agent("alice"), vec![path("a")], "x".into(), None)
            .unwrap();
        state.take_dirty().unwrap();
        clock.advance(Duration::seconds(5));
        state.claims(Some(&agent("alice")), None);
        assert!(!state.is_dirty(), "a renewal is not a mutation");
        assert_eq!(state.persist_target(false), 1);
        assert_eq!(state.persist_target(true), 2, "but shutdown waits for it");
        let delta = state.take_dirty().unwrap();
        let claims = delta.claims.unwrap();
        assert_eq!(claims[0].expires_at, clock.now() + Duration::seconds(600));
    }

    #[test]
    fn removed_ids_are_taken_separately_and_never_rewritten() {
        let mut changed: ChangedIds<u32> = ChangedIds::default();
        changed.mark(1);
        changed.mark(2);
        changed.remove(2);
        assert!(!changed.is_empty());
        let items = [(1, "a"), (2, "b")];
        assert_eq!(changed.take(&items, |i| i.0), vec![(1, "a")]);
        assert_eq!(changed.take_removed(), vec![2]);
        assert!(changed.is_empty());
    }

    #[test]
    fn only_changed_contracts_are_written_and_errors_force_a_full_rewrite() {
        let (_, state) = state();
        let new = |name: &str| NewContract {
            name: name.into(),
            kind: ContractKind::Http,
            shape: json!({}),
            consumers: Some(vec![]),
            notes: String::new(),
            expected_version: None,
        };
        state.contract_publish(agent("alice"), new("a")).unwrap();
        state.contract_publish(agent("alice"), new("b")).unwrap();
        assert_eq!(state.take_dirty().unwrap().contracts.len(), 2);
        state.contract_publish(agent("alice"), new("b")).unwrap();
        let delta = state.take_dirty().unwrap();
        assert_eq!(delta.contracts.len(), 1);
        assert_eq!(delta.contracts[0].name, "b");
        assert!(delta.tasks.is_none());

        state.mark_all_dirty();
        let delta = state.take_dirty().unwrap();
        assert_eq!(delta.contracts.len(), 2);
        assert!(delta.claims.is_some());
        assert!(delta.tasks.is_some());
        assert!(matches!(delta.notices, Log::Rewritten(_)));
        assert!(matches!(delta.decisions, Log::Rewritten(_)));
    }

    #[test]
    fn republishing_a_contract_emits_a_notice_to_consumers() {
        let (_, state) = state();
        let new = |v: u32| NewContract {
            name: "POST /sessions".into(),
            kind: ContractKind::Http,
            shape: json!({ "v": v }),
            consumers: Some(vec![path("src/client")]),
            notes: String::new(),
            expected_version: None,
        };
        let (_, none) = state.contract_publish(agent("alice"), new(1)).unwrap();
        assert!(none.is_none());
        let (published, notice) = state.contract_publish(agent("alice"), new(2)).unwrap();
        let notice = notice.unwrap();
        assert_eq!(published.contract.current.version, 2);
        assert_eq!(notice.kind, NoticeKind::Contract);
        assert_eq!(notice.contract_id, Some(published.contract.id));
        let unread = state.notices(
            None,
            &NoticeFilter {
                path: Some(path("src/client/sessions.rs")),
                unread_by: Some(agent("bob")),
                since: None,
            },
        );
        assert_eq!(unread.len(), 1);
    }

    #[test]
    fn status_summarizes_agents() {
        let (_, state) = state();
        state
            .claim(agent("alice"), vec![path("a"), path("b")], "x".into(), None)
            .unwrap();
        state
            .task_create(
                agent("alice"),
                NewTask {
                    title: "t".into(),
                    ..NewTask::default()
                },
            )
            .unwrap();
        state.task_pull(agent("bob")).unwrap();
        let report = state.status(None);
        assert_eq!(report.claims, 1);
        assert_eq!(report.tasks_open, 1);
        assert_eq!(report.agents.len(), 2);
        assert_eq!(report.agents[0].paths.len(), 2);
        assert_eq!(report.agents[1].tasks_in_progress, 1);
    }
}
