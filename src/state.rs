//! The single owner of all mutable coordination state.
//!
//! [`State`] wraps every primitive behind one lock, reaps expired claims
//! before each access, renews the calling agent's leases on activity, and
//! hands out [`Snapshot`]s for persistence and the dashboard. Domain rules
//! live in the primitive modules; this module only sequences them.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::claims::{Claim, ClaimBook, ClaimError, DEFAULT_TTL_SECS, Granted};
use crate::clock::Clock;
use crate::contracts::{
    Contract, ContractError, ContractKind, ContractRegistry, NewContract, Published,
};
use crate::decisions::{Decision, DecisionError, DecisionLog, NewDecision};
use crate::notices::{NewNotice, Notice, NoticeBoard, NoticeError, NoticeFilter, NoticeKind};
use crate::tasks::{NewTask, Task, TaskBoard, TaskError, TaskStatus};
use crate::types::{AgentId, NoticeId, RepoPath, TaskId};

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
    /// Agents with live claims or tasks in progress.
    pub agents: Vec<AgentSummary>,
}

#[derive(Debug)]
struct Inner {
    claims: ClaimBook,
    tasks: TaskBoard,
    contracts: ContractRegistry,
    notices: NoticeBoard,
    decisions: DecisionLog,
    seq: u64,
    dirty: bool,
}

/// Shared, lock-protected coordination state.
#[derive(Debug)]
pub struct State {
    inner: Mutex<Inner>,
    clock: Arc<dyn Clock>,
    started_at: DateTime<Utc>,
}

impl State {
    /// Builds state from a persisted snapshot and a clock.
    pub fn new(clock: Arc<dyn Clock>, snapshot: Snapshot) -> Self {
        let started_at = clock.now();
        Self {
            inner: Mutex::new(Inner {
                claims: ClaimBook::from_claims(snapshot.claims),
                tasks: TaskBoard::from_tasks(snapshot.tasks),
                contracts: ContractRegistry::from_contracts(snapshot.contracts),
                notices: NoticeBoard::from_notices(snapshot.notices),
                decisions: DecisionLog::from_decisions(snapshot.decisions),
                seq: snapshot.seq,
                dirty: false,
            }),
            clock,
            started_at,
        }
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
    /// renewing `agent`'s leases. `mutating` marks the state dirty.
    fn access<R>(
        &self,
        agent: Option<&AgentId>,
        mutating: bool,
        f: impl FnOnce(&mut Inner, DateTime<Utc>) -> R,
    ) -> R {
        let now = self.clock.now();
        let mut inner = self.lock();
        if !inner.claims.reap(now).is_empty() {
            inner.dirty = true;
        }
        if let Some(agent) = agent {
            if inner.claims.touch(agent, now) > 0 {
                inner.dirty = true;
            }
        }
        let result = f(&mut inner, now);
        if mutating {
            inner.dirty = true;
        }
        result
    }

    /// A snapshot of everything, without touching the dirty flag.
    pub fn snapshot(&self) -> Snapshot {
        self.access(None, false, |inner, _| snapshot_of(inner))
    }

    /// If anything changed since the last call, bumps `seq` and returns a
    /// snapshot to persist. Otherwise `None`.
    pub fn take_dirty(&self) -> Option<Snapshot> {
        let mut inner = self.lock();
        if !inner.dirty {
            return None;
        }
        inner.dirty = false;
        inner.seq += 1;
        Some(snapshot_of(&inner))
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
        self.access(Some(&agent.clone()), true, |inner, now| {
            inner.claims.claim(agent, paths, reason, ttl, now)
        })
    }

    /// Releases paths. See [`ClaimBook::release`].
    pub fn release(
        &self,
        agent: &AgentId,
        paths: Option<Vec<RepoPath>>,
    ) -> Result<Vec<RepoPath>, ClaimError> {
        self.access(Some(agent), true, |inner, _| {
            inner.claims.release(agent, paths)
        })
    }

    /// Renews all leases held by `agent`.
    pub fn renew(&self, agent: &AgentId) -> Result<Vec<Claim>, ClaimError> {
        self.access(None, true, |inner, now| inner.claims.renew(agent, now))
    }

    /// Live claims, optionally only those overlapping `path`.
    pub fn claims(&self, agent: Option<&AgentId>, path: Option<&RepoPath>) -> Vec<Claim> {
        self.access(agent, false, |inner, _| {
            inner.claims.list(path).into_iter().cloned().collect()
        })
    }

    /// Creates a task.
    pub fn task_create(&self, agent: AgentId, new: NewTask) -> Result<Task, TaskError> {
        self.access(Some(&agent.clone()), true, |inner, now| {
            inner.tasks.create(agent, new, now).cloned()
        })
    }

    /// Pulls the next unblocked task for `agent`.
    pub fn task_pull(&self, agent: AgentId) -> Option<Task> {
        self.access(Some(&agent.clone()), true, |inner, now| {
            inner.tasks.pull(agent, now).cloned()
        })
    }

    /// Updates a task's status.
    pub fn task_update(
        &self,
        agent: AgentId,
        id: TaskId,
        status: TaskStatus,
        note: Option<String>,
    ) -> Result<Task, TaskError> {
        self.access(Some(&agent.clone()), true, |inner, now| {
            inner.tasks.update(agent, id, status, note, now).cloned()
        })
    }

    /// Tasks matching the filters.
    pub fn tasks(
        &self,
        agent: Option<&AgentId>,
        status: Option<TaskStatus>,
        owner: Option<&AgentId>,
    ) -> Vec<Task> {
        self.access(agent, false, |inner, _| {
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
        self.access(Some(&agent.clone()), true, |inner, now| {
            let published = inner.contracts.publish(agent.clone(), new, now)?;
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
                    affected_paths: contract.consumers.clone(),
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
        self.access(agent, false, |inner, _| {
            inner.contracts.get(name_or_id).cloned()
        })
    }

    /// Contracts matching the filters.
    pub fn contracts(
        &self,
        agent: Option<&AgentId>,
        path: Option<&RepoPath>,
        kind: Option<ContractKind>,
    ) -> Vec<Contract> {
        self.access(agent, false, |inner, _| {
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
        self.access(Some(&agent.clone()), true, |inner, now| {
            inner.notices.publish(agent, new, now).cloned()
        })
    }

    /// Notices matching `filter`.
    pub fn notices(&self, agent: Option<&AgentId>, filter: &NoticeFilter) -> Vec<Notice> {
        self.access(agent, false, |inner, _| {
            inner.notices.list(filter).into_iter().cloned().collect()
        })
    }

    /// Acknowledges a notice.
    pub fn notice_ack(&self, agent: AgentId, id: NoticeId) -> Result<Notice, NoticeError> {
        self.access(Some(&agent.clone()), true, |inner, _| {
            inner.notices.ack(agent, id).cloned()
        })
    }

    /// Records a decision.
    pub fn decision_record(
        &self,
        agent: AgentId,
        new: NewDecision,
    ) -> Result<Decision, DecisionError> {
        self.access(Some(&agent.clone()), true, |inner, now| {
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
        self.access(agent, false, |inner, _| {
            inner
                .decisions
                .list(path, query)
                .into_iter()
                .cloned()
                .collect()
        })
    }

    /// Counts and per-agent summaries.
    pub fn status(&self, agent: Option<&AgentId>) -> StatusReport {
        self.access(agent, false, |inner, now| {
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
                agents,
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
    }
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
    fn dirty_snapshots_bump_seq_once_per_take() {
        let (_, state) = state();
        assert!(state.take_dirty().is_none());
        state
            .claim(agent("alice"), vec![path("a")], "x".into(), None)
            .unwrap();
        let snap = state.take_dirty().unwrap();
        assert_eq!(snap.seq, 1);
        assert_eq!(snap.claims.len(), 1);
        assert!(state.take_dirty().is_none());
        state.claims(None, None);
        assert!(
            state.take_dirty().is_none(),
            "reads without changes are not dirty"
        );
    }

    #[test]
    fn republishing_a_contract_emits_a_notice_to_consumers() {
        let (_, state) = state();
        let new = |v: u32| NewContract {
            name: "POST /sessions".into(),
            kind: ContractKind::Http,
            shape: json!({ "v": v }),
            consumers: vec![path("src/client")],
            notes: String::new(),
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
