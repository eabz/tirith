//! The swarm lead (ADR-0027): who it is, what its policy does, and the log
//! of what that policy decided.
//!
//! **Identity.** The lead agent is whoever holds a claim on the reserved
//! path [`LEAD_PATH`]. It is an ordinary claim, so exclusivity, expiry and
//! `lost` reporting come from [`crate::claims`]; [`Lead::from_claims`] only
//! says which claim makes an agent the lead.
//!
//! **Decision log.** Every lead policy decision is a [`LeadEntry`] in
//! `.tirith/runtime/lead_log.jsonl`, kept for [`LOG_RETENTION_DAYS`] days:
//! claim lifecycle events, notice pushes and escalations. [`LeadLog`] holds
//! them; `State` owns one and the store persists it like the other runtime
//! logs. A row records the event, the candidates, the deterministic rule
//! that decided, and the action taken. Its `outcome` is filled in later,
//! when the result becomes observable.
//!
//! **Policy.** [`LeadPolicy`] is deterministic code acting on the lead's
//! behalf: it waits for free tasks, pushes notices to the holders they
//! affect, and raises and routes escalations, acting through `State` and
//! logging each decision. `server.rs` calls it and formats the result.

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::claims::{Claim, Conflict, Granted};
use crate::messages::Message;
use crate::notices::Notice;
use crate::state::{MAX_CLAIM_WAIT_SECS, State};
use crate::tasks::{Pulled, Task, TaskState};
use crate::types::{AgentId, RepoPath, TaskId};

/// The reserved path whose holder is the swarm lead. Workers never claim it.
pub const LEAD_PATH: &str = ".tirith/lead";

/// Days a decision log row is kept; older rows are dropped on load.
pub const LOG_RETENTION_DAYS: i64 = 7;
/// Rows returned by a log listing when no limit is given.
pub const LOG_PAGE_DEFAULT: usize = 50;
/// Most rows a log listing returns.
pub const LOG_PAGE_MAX: usize = 500;

/// The current lead agent and when its lease on [`LEAD_PATH`] ends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lead {
    /// The agent holding the lead lease.
    pub agent: AgentId,
    /// When that lease ends unless renewed.
    pub expires_at: DateTime<Utc>,
}

impl Lead {
    /// The lead among live `claims`: the owner of a claim naming exactly
    /// [`LEAD_PATH`]. A claim on an ancestor such as `.tirith` blocks others
    /// from claiming the lead path but does not make its owner the lead:
    /// leadership is an explicit act.
    pub fn from_claims<'a>(claims: impl IntoIterator<Item = &'a Claim>) -> Option<Self> {
        claims
            .into_iter()
            .find(|c| c.paths.iter().any(|p| p.as_str() == LEAD_PATH))
            .map(|c| Self {
                agent: c.owner.clone(),
                expires_at: c.expires_at,
            })
    }
}

/// What the lead policy reacted to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeadEvent {
    /// A notice was pushed to the holders of the paths it affects.
    NoticePublished,
    /// A worker escalated: blocked, asked the lead for the human, or was
    /// refused repeatedly (ADR-0027 section 3).
    EscalationRaised,
    /// A claim was granted (new or renewed paths).
    ClaimGranted,
    /// A claim was refused over overlapping claims.
    ClaimRefused,
    /// A claim waited for a conflicting lease to end (`wait_secs`).
    ClaimWaited,
    /// Paths were released.
    ClaimReleased,
    /// A lease ended without release: expired or past its maximum age.
    LeaseEnded,
}

impl LeadEvent {
    /// The name used in the log.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoticePublished => "notice_published",
            Self::EscalationRaised => "escalation_raised",
            Self::ClaimGranted => "claim_granted",
            Self::ClaimRefused => "claim_refused",
            Self::ClaimWaited => "claim_waited",
            Self::ClaimReleased => "claim_released",
            Self::LeaseEnded => "lease_ended",
        }
    }
}

impl fmt::Display for LeadEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A decision log row's identifier: increasing within one log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EntryId(u64);

impl EntryId {
    /// The number.
    pub fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for EntryId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A decision about to be logged: everything the policy knows. The log
/// adds the id, the time and the state sequence number.
#[derive(Debug, Clone, PartialEq)]
pub struct NewEntry {
    event: LeadEvent,
    candidates: Vec<String>,
    agent: Option<AgentId>,
    details: Option<Value>,
    rule: Option<String>,
    action: String,
}

impl NewEntry {
    /// A decision on `event` that did `action`.
    pub fn new(event: LeadEvent, action: impl Into<String>) -> Self {
        Self {
            event,
            candidates: Vec::new(),
            agent: None,
            details: None,
            rule: None,
            action: action.into(),
        }
    }

    /// The ids the policy chose among.
    #[must_use]
    pub fn with_candidates(mut self, candidates: impl IntoIterator<Item = String>) -> Self {
        self.candidates = candidates.into_iter().collect();
        self
    }

    /// The agent the event is about: who claimed, pulled, or was refused.
    #[must_use]
    pub fn with_agent(mut self, agent: AgentId) -> Self {
        self.agent = Some(agent);
        self
    }

    /// Structured facts of the event, e.g. `{"paths": [..], "ttl_secs": 600}`.
    #[must_use]
    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    /// The deterministic rule that decided.
    #[must_use]
    pub fn with_rule(mut self, rule: impl Into<String>) -> Self {
        self.rule = Some(rule.into());
        self
    }

    /// Changes the action after the fact, e.g. once delivery decided.
    #[must_use]
    pub fn with_action(mut self, action: impl Into<String>) -> Self {
        self.action = action.into();
        self
    }
}

/// One row of the decision log (ADR-0027 section 4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LeadEntry {
    /// Increasing within the log.
    pub id: EntryId,
    /// When the decision was made.
    pub at: DateTime<Utc>,
    /// What the policy reacted to.
    pub event: LeadEvent,
    /// The state sequence number read.
    pub seq: u64,
    /// The ids chosen among.
    pub candidates: Vec<String>,
    /// The agent the event is about, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentId>,
    /// Structured facts of the event, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    /// The deterministic rule that decided, if any.
    pub rule: Option<String>,
    /// What was done through Tirith.
    pub action: String,
    /// What happened afterwards, once observable.
    #[serde(default)]
    pub outcome: Option<Value>,
}

/// The decision log in memory: rows in the order they were made.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LeadLog {
    entries: Vec<LeadEntry>,
}

impl LeadLog {
    /// Rebuilds a log from persisted rows, dropping those older than
    /// [`LOG_RETENTION_DAYS`] at `now`.
    pub fn from_entries(entries: Vec<LeadEntry>, now: DateTime<Utc>) -> Self {
        let keep_after = now - chrono::Duration::days(LOG_RETENTION_DAYS);
        Self {
            entries: entries.into_iter().filter(|e| e.at >= keep_after).collect(),
        }
    }

    /// Every row, oldest first.
    pub fn entries(&self) -> &[LeadEntry] {
        &self.entries
    }

    /// Appends `new`, made at `at` after reading state `seq`.
    pub fn append(&mut self, new: NewEntry, at: DateTime<Utc>, seq: u64) -> EntryId {
        let id = EntryId(self.entries.last().map_or(1, |e| e.id.0 + 1));
        let NewEntry {
            event,
            candidates,
            agent,
            details,
            rule,
            action,
        } = new;
        self.entries.push(LeadEntry {
            id,
            at,
            event,
            seq,
            candidates,
            agent,
            details,
            rule,
            action,
            outcome: None,
        });
        id
    }

    /// Records what came of decision `id`. `false` when no such row is
    /// kept.
    pub fn set_outcome(&mut self, id: EntryId, outcome: Value) -> bool {
        match self.entries.iter_mut().rev().find(|e| e.id == id) {
            Some(entry) => {
                entry.outcome = Some(outcome);
                true
            }
            None => false,
        }
    }

    /// The newest `limit` rows (at most [`LOG_PAGE_MAX`]), newest first,
    /// optionally only those for `event`.
    pub fn newest(&self, limit: usize, event: Option<LeadEvent>) -> Vec<LeadEntry> {
        self.entries
            .iter()
            .rev()
            .filter(|e| event.is_none_or(|ev| e.event == ev))
            .take(limit.min(LOG_PAGE_MAX))
            .cloned()
            .collect()
    }
}

/// The log row for a granted claim: who, which paths are new, renewed or
/// absorbed, why, and for how long.
pub fn claim_granted(agent: &AgentId, granted: &Granted, reason: &str, ttl_secs: u64) -> NewEntry {
    NewEntry::new(
        LeadEvent::ClaimGranted,
        format!(
            "granted {} new and {} renewed path(s)",
            granted.new_paths.len(),
            granted.renewed_paths.len()
        ),
    )
    .with_agent(agent.clone())
    .with_candidates(
        granted
            .new_paths
            .iter()
            .chain(&granted.renewed_paths)
            .map(|p| p.as_str().to_owned()),
    )
    .with_details(serde_json::json!({
        "new_paths": granted.new_paths,
        "renewed_paths": granted.renewed_paths,
        "absorbed_paths": granted.absorbed_paths,
        "reason": reason,
        "ttl_secs": ttl_secs,
        "expires_at": granted.expires_at,
    }))
}

/// The log row for a claim refused over `conflicts`.
pub fn claim_refused(
    agent: &AgentId,
    paths: &[RepoPath],
    reason: &str,
    conflicts: &[Conflict],
) -> NewEntry {
    let overlaps: Vec<Value> = conflicts
        .iter()
        .map(|c| serde_json::json!({ "path": c.path, "overlaps": c.overlaps, "owner": c.owner }))
        .collect();
    NewEntry::new(
        LeadEvent::ClaimRefused,
        format!("refused {} path(s)", paths.len()),
    )
    .with_rule("overlap")
    .with_agent(agent.clone())
    .with_candidates(paths.iter().map(|p| p.as_str().to_owned()))
    .with_details(serde_json::json!({ "reason": reason, "conflicts": overlaps }))
}

/// The log row for a claim that waited `waited` of `wait` for conflicting
/// leases to end, and was `granted` or not.
pub fn claim_waited(
    agent: &AgentId,
    paths: &[RepoPath],
    wait: Duration,
    waited: Duration,
    granted: bool,
) -> NewEntry {
    NewEntry::new(
        LeadEvent::ClaimWaited,
        if granted {
            "granted after waiting"
        } else {
            "still refused after waiting"
        },
    )
    .with_agent(agent.clone())
    .with_candidates(paths.iter().map(|p| p.as_str().to_owned()))
    .with_details(serde_json::json!({
        "wait_secs": wait.as_secs(),
        "waited_ms": u64::try_from(waited.as_millis()).unwrap_or(u64::MAX),
        "outcome": if granted { "granted" } else { "refused" },
    }))
}

/// The log row for `released` paths.
pub fn claim_released(agent: &AgentId, released: &[RepoPath]) -> NewEntry {
    NewEntry::new(
        LeadEvent::ClaimReleased,
        format!("released {} path(s)", released.len()),
    )
    .with_agent(agent.clone())
    .with_candidates(released.iter().map(|p| p.as_str().to_owned()))
}

/// The log row for a lease that ended without a release: past its TTL
/// without activity, or past its maximum age (ADR-0015).
pub fn lease_ended(claim: &Claim) -> NewEntry {
    let rule = if claim.expires_at >= claim.max_expiry() {
        "max_lease_age"
    } else {
        "ttl_without_activity"
    };
    NewEntry::new(
        LeadEvent::LeaseEnded,
        format!("lease on {} path(s) ended", claim.paths.len()),
    )
    .with_rule(rule)
    .with_agent(claim.owner.clone())
    .with_candidates(claim.paths.iter().map(|p| p.as_str().to_owned()))
    .with_details(serde_json::json!({
        "claim_id": claim.id,
        "reason": claim.reason,
        "claimed_at": claim.claimed_at,
        "expires_at": claim.expires_at,
    }))
}

/// Characters of a title or summary quoted in a message.
const QUOTE_MAX: usize = 160;

/// The lead policy (ADR-0027 section 1): deterministic code acting on the
/// lead's behalf. One per daemon.
#[derive(Debug)]
pub struct LeadPolicy {
    state: Arc<State>,
    /// Recent refusals per agent and claimed path set, for the
    /// repeated-refusal escalation trigger.
    refusals: Mutex<HashMap<(AgentId, Vec<RepoPath>), Refusals>>,
}

impl LeadPolicy {
    /// A policy over `state`.
    pub fn new(state: Arc<State>) -> Self {
        Self {
            state,
            refusals: Mutex::new(HashMap::new()),
        }
    }

    /// [`State::task_pull`], but while no candidate is free (none is
    /// unblocked, or every one is held, ADR-0028) waits up to `wait`
    /// (capped at [`MAX_CLAIM_WAIT_SECS`]) for a claim to end or a task to
    /// be created or change status, then pulls. When the wait ends it
    /// returns whatever a plain pull returns then, `None` or a held task
    /// with `waiting_on`. Nothing is assigned before that pull, so dropping
    /// the future while it waits assigns nothing, and no lock is held
    /// across an await.
    pub async fn task_pull_waiting(&self, agent: AgentId, wait: Duration) -> Option<Pulled> {
        let wait = wait.min(Duration::from_secs(MAX_CLAIM_WAIT_SECS));
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            // Watched before the check, so a change in between still wakes.
            let watch = self.state.watch_board();
            let over = tokio::time::Instant::now() >= deadline;
            if over || !self.state.task_candidates(&agent).is_empty() {
                let pulled = self.state.task_pull(agent.clone());
                // `None` here means another agent took the free task first.
                if pulled.is_some() || over {
                    return pulled;
                }
            }
            watch.changed(&agent, deadline).await;
        }
    }

    /// Pushes `notice` now, through their inboxes, to every other agent
    /// holding one of its affected paths, or an ancestor or descendant of
    /// one, instead of at their next claim. A pushed notice counts as seen.
    /// Logs one row when anyone holds such a path.
    pub fn notice_published(&self, notice: &Notice) {
        let holders = self.state.notice_holders(notice);
        if holders.is_empty() {
            return;
        }
        let text = format!(
            "notice {} ({}) from {} changes paths you hold: {}",
            notice.id.short(),
            notice.kind,
            notice.published_by,
            clip(&notice.summary),
        );
        let mut pushed = Vec::new();
        for agent in &holders {
            match self.state.push_notice(agent, notice.id, text.clone()) {
                Ok(_) => pushed.push(agent.as_str().to_owned()),
                Err(error) => tracing::warn!(%error, %agent, "could not push notice"),
            }
        }
        let entry = NewEntry::new(
            LeadEvent::NoticePublished,
            format!(
                "pushed notice {} to {} holder(s)",
                notice.id.short(),
                pushed.len()
            ),
        )
        .with_rule("holds_affected_path")
        .with_agent(notice.published_by.clone())
        .with_candidates(holders.iter().map(|a| a.as_str().to_owned()))
        .with_details(serde_json::json!({
            "notice": notice.id,
            "kind": notice.kind,
            "affected_paths": notice.affected_paths,
            "pushed": pushed,
        }));
        self.state.lead_log_append(entry);
    }

    /// A task changed status. `blocked` with a note raises an escalation
    /// (ADR-0027 section 3); any other status answers the escalations open
    /// on the task.
    pub fn task_updated(&self, agent: &AgentId, task: &Task, note: Option<&str>) {
        match (&task.state, note.map(str::trim).filter(|n| !n.is_empty())) {
            (TaskState::Blocked { .. }, Some(note)) => {
                self.raise(
                    &Escalation::new(Trigger::TaskBlocked, agent.clone(), note).with_task(task.id),
                );
            }
            (TaskState::Blocked { .. }, None) => {}
            (state, _) => {
                let id = task.id.to_string();
                let via = format!("task_{}", state.status());
                self.answer_open(|e| details_str(e, "task") == Some(id.as_str()), agent, &via);
            }
        }
    }

    /// A message was sent. One to an agent answers that agent's open
    /// escalations. One to the lead by name, from anyone else, raises an
    /// escalation only when its text matches a human rule; otherwise the
    /// lead already has it and nothing more is done.
    pub fn message_sent(&self, message: &Message) {
        if message.is_broadcast() {
            return;
        }
        let Ok(to) = AgentId::new(&message.to) else {
            return;
        };
        if to != message.from {
            self.answer_open(
                |e| e.agent.as_ref() == Some(&to) && e.at <= message.at,
                &message.from,
                "message",
            );
        }
        let to_lead = self
            .state
            .lead()
            .is_some_and(|lead| lead.agent == to && lead.agent != message.from);
        if !to_lead {
            return;
        }
        let escalation = Escalation::new(
            Trigger::MessageToLead,
            message.from.clone(),
            message.text.clone(),
        )
        .with_paths(message.paths.clone());
        if human_rule(&escalation.rule_text()).is_some() {
            self.raise(&escalation);
        }
    }

    /// A claim by `agent` on `paths` was refused. The
    /// [`REFUSALS_TO_ESCALATE`]th refusal of the same paths within
    /// [`REFUSAL_WINDOW_SECS`] raises one escalation.
    pub fn claim_refused(&self, agent: &AgentId, paths: &[RepoPath], reason: &str) {
        let now = self.state.now();
        let window =
            chrono::Duration::seconds(i64::try_from(REFUSAL_WINDOW_SECS).unwrap_or(i64::MAX));
        let mut key = paths.to_vec();
        key.sort();
        key.dedup();
        let count = {
            let mut refusals = self.refusals.lock().unwrap_or_else(PoisonError::into_inner);
            refusals.retain(|_, r| r.at.last().is_some_and(|at| now - *at < window));
            let track = refusals.entry((agent.clone(), key.clone())).or_default();
            track.at.retain(|at| now - *at < window);
            track.at.push(now);
            if track.raised || track.at.len() < REFUSALS_TO_ESCALATE {
                return;
            }
            track.raised = true;
            track.at.len()
        };
        let held_by: Vec<String> = self
            .state
            .claims(None, None)
            .into_iter()
            .filter(|c| {
                c.owner != *agent
                    && c.paths
                        .iter()
                        .any(|p| key.iter().any(|k| k.claim_overlaps(p)))
            })
            .map(|c| c.owner.as_str().to_owned())
            .collect();
        // The holders' own reasons stay out of the text, so the human rules
        // judge only what this agent asked for.
        let text = format!(
            "claim on {} refused {count} times; reason: {reason}; held by {}",
            key.iter()
                .map(RepoPath::as_str)
                .collect::<Vec<_>>()
                .join(", "),
            if held_by.is_empty() {
                "nobody now".to_owned()
            } else {
                held_by.join(", ")
            },
        );
        self.raise(&Escalation::new(Trigger::ClaimsRefused, agent.clone(), text).with_paths(key));
    }

    /// A claim by `agent` on `paths` was granted: it answers an escalation
    /// raised by refusals of the same paths.
    pub fn claim_granted(&self, agent: &AgentId, paths: &[RepoPath]) {
        let mut key = paths.to_vec();
        key.sort();
        key.dedup();
        let raised = self
            .refusals
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&(agent.clone(), key.clone()))
            .is_some_and(|r| r.raised);
        if !raised {
            return;
        }
        let paths: Vec<String> = key.iter().map(|p| p.as_str().to_owned()).collect();
        self.answer_open(
            |e| {
                e.agent.as_ref() == Some(agent)
                    && details_str(e, "trigger") == Some(Trigger::ClaimsRefused.as_str())
                    && e.details.as_ref().and_then(|d| strings_of(&d["paths"]))
                        == Some(paths.clone())
            },
            agent,
            "claim_granted",
        );
    }

    /// Routes `escalation` (ADR-0027 section 3), delivers it through
    /// Tirith, and logs one decision row. Text matching a
    /// [human rule](human_rule) goes to the human queue and the lead is
    /// told; anything else goes to the lead's inbox. With no lead, or when
    /// the lead is the agent escalating, it goes to the human queue. Called
    /// by the triggers above; public for a stop hook.
    pub fn raise(&self, escalation: &Escalation) {
        let text: String = escalation.text.chars().take(ESCALATION_TEXT_MAX).collect();
        let worker = &escalation.agent;
        let lead = self.state.lead().filter(|l| l.agent != *worker);
        let tasks = self.state.tasks(None, None, None);
        let task = escalation
            .task
            .and_then(|id| tasks.iter().find(|t| t.id == id))
            .or_else(|| {
                tasks.iter().find(|t| {
                    matches!(
                        t.state,
                        TaskState::InProgress { .. } | TaskState::Blocked { .. }
                    ) && t.state.owner() == Some(worker)
                })
            });
        let about = task.map_or_else(String::new, |t| format!(" on task {}", t.id.short()));
        let quote = clip(&text);
        let rule = human_rule(&escalation.rule_text()).map(|rule| format!("human:{rule}"));
        let mut delivered = Vec::new();
        let mut entry = NewEntry::new(LeadEvent::EscalationRaised, "").with_agent(worker.clone());
        let (route, action) = match (rule, &lead) {
            (None, Some(lead)) => {
                let told = self.tell(
                    &lead.agent,
                    format!(
                        "escalation from {worker}{about} ({}): {quote}",
                        escalation.trigger.as_str()
                    ),
                );
                if told {
                    delivered.push("lead");
                }
                (Route::Lead, format!("told the lead {}", lead.agent))
            }
            (rule, lead) => {
                entry = entry.with_rule(rule.unwrap_or_else(|| "no_lead".to_owned()));
                delivered.push("human_queue");
                if let Some(lead) = lead {
                    let told = self.tell(
                        &lead.agent,
                        format!(
                            "needs the human: escalation from {worker}{about} is in the human \
                             queue (tirith lead human): {quote}"
                        ),
                    );
                    if told {
                        delivered.push("lead");
                    }
                }
                (Route::Human, "queued for the human".to_owned())
            }
        };
        let details = serde_json::json!({
            "trigger": escalation.trigger.as_str(),
            "text": text,
            "task": task.map(|t| t.id.to_string()),
            "paths": escalation.paths,
            "route": route.as_str(),
            "delivered": delivered,
        });
        self.state
            .lead_log_append(entry.with_details(details).with_action(action));
    }

    /// Fills the outcome of every open escalation row `matches` picks:
    /// answered by `by`, through `via`, after how long.
    fn answer_open(&self, matches: impl Fn(&LeadEntry) -> bool, by: &AgentId, via: &str) {
        let now = self.state.now();
        for entry in self
            .state
            .lead_log(LOG_PAGE_MAX, Some(LeadEvent::EscalationRaised))
            .iter()
            .filter(|e| e.outcome.is_none() && matches(e))
        {
            let outcome = serde_json::json!({
                "answered_by": by,
                "via": via,
                "at": now,
                "after_secs": (now - entry.at).num_seconds().max(0),
            });
            self.state.lead_log_outcome(entry.id, outcome);
        }
    }

    /// A message from `tirith` to `to`; `false` if it could not be sent.
    fn tell(&self, to: &AgentId, text: String) -> bool {
        match self.state.notify(to, text) {
            Ok(_) => true,
            Err(error) => {
                tracing::warn!(%error, agent = %to, "could not deliver an escalation");
                false
            }
        }
    }
}

/// `text` cut to [`QUOTE_MAX`] characters, with an ellipsis when cut.
fn clip(text: &str) -> String {
    let mut cut: String = text.chars().take(QUOTE_MAX).collect();
    if cut.chars().count() < text.chars().count() {
        cut.push('…');
    }
    cut
}

// ---------------------------------------------------------------------------
// Escalations (ADR-0027 section 3)

/// Refusals of the same claim, by the same agent, that raise an escalation.
pub const REFUSALS_TO_ESCALATE: usize = 3;
/// The window those refusals fall within: three full claim waits
/// ([`MAX_CLAIM_WAIT_SECS`]), so an agent retrying with `wait_secs` is
/// counted as well as one retrying at once.
pub const REFUSAL_WINDOW_SECS: u64 = 3 * MAX_CLAIM_WAIT_SECS;
/// Characters of escalation text kept in the log and matched by the human
/// rules.
pub const ESCALATION_TEXT_MAX: usize = 1000;

/// The deterministic human rules (ADR-0027 section 3), in one place: an
/// escalation whose text contains one of a rule's phrases goes to the human
/// queue. Phrases match case-insensitively at word boundaries. Spending
/// also matches a dollar amount such as `$20`.
pub const HUMAN_RULES: [(&str, &[&str]); 5] = [
    (
        "credentials",
        &[
            "api key",
            "api keys",
            "api_key",
            "apikey",
            "secret",
            "secrets",
            "password",
            "passwords",
            "passphrase",
            "credential",
            "credentials",
            "access token",
            "auth token",
            "bearer token",
            "gh token",
            "github token",
            "npm token",
            "private key",
            "ssh key",
            ".env",
            "keychain",
        ],
    ),
    (
        "permissions",
        &[
            "sudo",
            "chown",
            "chmod",
            "owned by root",
            "as root",
            "root access",
            "admin access",
            "admin rights",
            "with admin",
            "repo:admin",
            "grant",
            "permission denied",
            "need permission",
            "needs permission",
            "permission to",
            "2fa",
            "0.0.0.0",
            "firewall",
        ],
    ),
    (
        "spending",
        &[
            "paid",
            "pay",
            "payment",
            "purchase",
            "buy",
            "billing",
            "credit card",
            "invoice",
            "subscription",
        ],
    ),
    (
        "destructive",
        &[
            "force-push",
            "force push",
            "push --force",
            "push -f",
            "force-with-lease",
            "filter-repo",
            "filter-branch",
            "rewrite history",
            "reset --hard",
            "rm -rf",
            "drop table",
            "drop database",
            "delete the repo",
            "delete the repository",
            "delete the branch",
            "delete branch",
            "kill",
            "pkill",
            "killall",
            "cargo publish",
            "npm publish",
            "yank",
            "irreversible",
            "cannot be undone",
            "can't be undone",
            "wipe",
        ],
    ),
    (
        "addressed_to_human",
        &[
            "ask the human",
            "ask the user",
            "human decision",
            "human approval",
            "needs a human",
            "need a human",
            "human to decide",
            "@human",
            "human:",
        ],
    ),
];

/// The first human rule `text` matches, or `None`.
pub fn human_rule(text: &str) -> Option<&'static str> {
    let text = text.to_lowercase();
    HUMAN_RULES
        .iter()
        .find(|(rule, phrases)| {
            phrases.iter().any(|p| has_phrase(&text, p))
                || (*rule == "spending" && has_amount(&text))
        })
        .map(|(rule, _)| *rule)
}

/// Whether `phrase` occurs in `text` without a letter or digit glued to
/// either end that is itself a letter or digit.
fn has_phrase(text: &str, phrase: &str) -> bool {
    let glued = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
    let starts_word = phrase.chars().next().is_some_and(char::is_alphanumeric);
    let ends_word = phrase.chars().last().is_some_and(char::is_alphanumeric);
    text.match_indices(phrase).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + phrase.len()..].chars().next();
        (!starts_word || !glued(before)) && (!ends_word || !glued(after))
    })
}

/// Whether `text` names a dollar amount, such as `$20`.
fn has_amount(text: &str) -> bool {
    text.match_indices('$').any(|(at, _)| {
        text[at + 1..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit())
    })
}

/// What raised an escalation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    /// `task_update` to `blocked` with a note.
    TaskBlocked,
    /// A message to the lead by name whose text matches a human rule.
    MessageToLead,
    /// The same claim refused [`REFUSALS_TO_ESCALATE`] times within
    /// [`REFUSAL_WINDOW_SECS`].
    ClaimsRefused,
}

impl Trigger {
    /// The name used in the log.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TaskBlocked => "task_blocked",
            Self::MessageToLead => "message_to_lead",
            Self::ClaimsRefused => "claims_refused",
        }
    }
}

/// Where an escalation went (ADR-0027 section 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Route {
    /// The lead agent's inbox.
    Lead,
    /// The human queue, and the lead agent's inbox when there is a lead.
    Human,
}

impl Route {
    /// The name used in the log.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lead => "lead",
            Self::Human => "human",
        }
    }
}

/// One escalation to route. Build it with [`Escalation::new`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Escalation {
    trigger: Trigger,
    agent: AgentId,
    text: String,
    task: Option<TaskId>,
    paths: Vec<RepoPath>,
}

impl Escalation {
    /// `agent` escalated through `trigger`, saying `text`.
    pub fn new(trigger: Trigger, agent: AgentId, text: impl Into<String>) -> Self {
        Self {
            trigger,
            agent,
            text: text.into(),
            task: None,
            paths: Vec::new(),
        }
    }

    /// The task it is about.
    #[must_use]
    pub fn with_task(mut self, task: TaskId) -> Self {
        self.task = Some(task);
        self
    }

    /// The paths it is about, e.g. the refused claim's.
    #[must_use]
    pub fn with_paths(mut self, paths: Vec<RepoPath>) -> Self {
        self.paths = paths;
        self
    }

    /// What the human rules read: the text, cut to
    /// [`ESCALATION_TEXT_MAX`] characters, and the paths.
    fn rule_text(&self) -> String {
        let text: String = self.text.chars().take(ESCALATION_TEXT_MAX).collect();
        let paths: Vec<&str> = self.paths.iter().map(RepoPath::as_str).collect();
        format!("{text} {}", paths.join(" "))
    }
}

/// One item of the human queue: an escalation routed to the human and not
/// answered yet.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HumanItem {
    /// The decision log row.
    pub id: EntryId,
    /// When it was raised.
    pub at: DateTime<Utc>,
    /// Seconds it has waited.
    pub waited_secs: i64,
    /// Who escalated.
    pub agent: Option<AgentId>,
    /// What raised it.
    pub trigger: Value,
    /// The task it is about, if any.
    pub task: Value,
    /// What the agent said.
    pub text: Value,
    /// The deterministic rule that sent it here, if any.
    pub rule: Option<String>,
    /// The agents it holds up: the one that escalated, others escalating
    /// on the same task, and owners of open tasks that depend on it.
    pub blocked_agents: Vec<AgentId>,
}

/// The escalation log rows still open for the human among `entries`,
/// ranked by how many agents each blocks, then by how long it has waited.
pub fn human_queue(entries: &[LeadEntry], tasks: &[Task], now: DateTime<Utc>) -> Vec<HumanItem> {
    let open = |e: &&LeadEntry| e.event == LeadEvent::EscalationRaised && e.outcome.is_none();
    let mut items: Vec<HumanItem> = entries
        .iter()
        .filter(open)
        .filter(|e| delivered(e).contains(&"human_queue"))
        .map(|e| {
            let details = e.details.clone().unwrap_or(Value::Null);
            let task = details["task"].as_str().map(str::to_owned);
            let mut blocked: BTreeSet<AgentId> = e.agent.iter().cloned().collect();
            if let Some(task) = &task {
                blocked.extend(
                    entries
                        .iter()
                        .filter(open)
                        .filter(|o| {
                            o.details.as_ref().and_then(|d| d["task"].as_str()) == Some(task)
                        })
                        .filter_map(|o| o.agent.clone()),
                );
                blocked.extend(
                    tasks
                        .iter()
                        .filter(|t| !matches!(t.state, TaskState::Done { .. }))
                        .filter(|t| t.depends_on.iter().any(|d| d.to_string() == *task))
                        .filter_map(|t| t.state.owner().cloned()),
                );
            }
            HumanItem {
                id: e.id,
                at: e.at,
                waited_secs: (now - e.at).num_seconds().max(0),
                agent: e.agent.clone(),
                trigger: details["trigger"].clone(),
                task: details["task"].clone(),
                text: details["text"].clone(),
                rule: e.rule.clone(),
                blocked_agents: blocked.into_iter().collect(),
            }
        })
        .collect();
    items.sort_by(|a, b| {
        b.blocked_agents
            .len()
            .cmp(&a.blocked_agents.len())
            .then(a.at.cmp(&b.at))
    });
    items
}

/// The human queue of `state` (see [`human_queue`]).
pub fn human_queue_of(state: &State) -> Vec<HumanItem> {
    let entries = state.lead_log(LOG_PAGE_MAX, Some(LeadEvent::EscalationRaised));
    human_queue(&entries, &state.tasks(None, None, None), state.now())
}

/// Where a routed escalation row says it was delivered.
fn delivered(entry: &LeadEntry) -> Vec<&str> {
    entry
        .details
        .as_ref()
        .and_then(|d| d["delivered"].as_array())
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

/// Refusals of one claim by one agent, for the repeated-refusal trigger.
#[derive(Debug, Default)]
struct Refusals {
    at: Vec<DateTime<Utc>>,
    raised: bool,
}

/// The string at `field` of a row's details.
fn details_str<'a>(entry: &'a LeadEntry, field: &str) -> Option<&'a str> {
    entry.details.as_ref().and_then(|d| d[field].as_str())
}

/// A JSON array of strings, or `None`.
fn strings_of(value: &Value) -> Option<Vec<String>> {
    value
        .as_array()?
        .iter()
        .map(|v| v.as_str().map(str::to_owned))
        .collect()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use serde_json::json;

    use super::*;
    use crate::claims::ClaimBook;
    use crate::types::RepoPath;

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 17, 3, 0, 0).unwrap()
    }

    fn book_with(agent: &str, paths: &[&str]) -> ClaimBook {
        let now = t0();
        let mut book = ClaimBook::default();
        let paths: Vec<RepoPath> = paths.iter().map(|p| RepoPath::new(p).unwrap()).collect();
        book.claim(AgentId::new(agent).unwrap(), paths, "r".into(), 3600, now)
            .unwrap();
        book
    }

    #[test]
    fn the_holder_of_the_exact_lead_path_is_the_lead() {
        let book = book_with("boss", &["src/a.rs", "./.tirith/lead/"]);
        let lead = Lead::from_claims(book.claims()).unwrap();
        assert_eq!(lead.agent.as_str(), "boss");
    }

    #[test]
    fn an_ancestor_claim_does_not_make_a_lead() {
        let book = book_with("worker", &[".tirith"]);
        assert!(Lead::from_claims(book.claims()).is_none());
        let book = book_with("worker", &["src"]);
        assert!(Lead::from_claims(book.claims()).is_none());
    }

    #[test]
    fn log_rows_get_increasing_ids_and_outcomes_later() {
        let mut log = LeadLog::default();
        let first = log.append(
            NewEntry::new(
                LeadEvent::ClaimGranted,
                "granted 2 new and 0 renewed path(s)",
            )
            .with_candidates(["src/a.rs".to_owned(), "src/b.rs".to_owned()]),
            t0(),
            7,
        );
        let second = log.append(
            NewEntry::new(LeadEvent::EscalationRaised, "queued for the human").with_rule("no_lead"),
            t0(),
            8,
        );
        assert!(second > first);
        assert!(log.set_outcome(first, json!({ "done": true })));
        assert!(!log.set_outcome(EntryId(99), json!(null)));
        let newest = log.newest(10, None);
        assert_eq!(newest[0].id, second);
        assert_eq!(newest[1].outcome, Some(json!({ "done": true })));
        assert_eq!(log.newest(10, Some(LeadEvent::EscalationRaised)).len(), 1);
        assert_eq!(log.newest(1, None).len(), 1);
    }

    #[test]
    fn a_row_serializes_to_the_documented_shape_and_back() {
        let mut log = LeadLog::default();
        log.append(
            NewEntry::new(LeadEvent::ClaimRefused, "refused 1 path(s)").with_rule("overlap"),
            t0(),
            3,
        );
        let row = serde_json::to_value(&log.entries()[0]).unwrap();
        assert_eq!(row["event"], "claim_refused");
        assert_eq!(row["rule"], "overlap");
        assert_eq!(row["seq"], 3);
        assert!(row.get("agent").is_none(), "{row}");
        assert!(row["outcome"].is_null());
        let back: LeadEntry = serde_json::from_value(row).unwrap();
        assert_eq!(back, log.entries()[0]);
    }

    #[test]
    fn rows_past_retention_are_dropped_on_load() {
        let mut log = LeadLog::default();
        log.append(NewEntry::new(LeadEvent::ClaimReleased, "a"), t0(), 1);
        log.append(
            NewEntry::new(LeadEvent::ClaimReleased, "b"),
            t0() + chrono::Duration::days(3),
            2,
        );
        let later = t0() + chrono::Duration::days(LOG_RETENTION_DAYS) + chrono::Duration::hours(1);
        let loaded = LeadLog::from_entries(log.entries().to_vec(), later);
        assert_eq!(loaded.entries().len(), 1);
        assert_eq!(loaded.entries()[0].action, "b");
        assert_eq!(
            LeadLog::from_entries(loaded.entries().to_vec(), later)
                .entries()
                .len(),
            1
        );
    }

    mod policy {
        use super::*;
        use crate::clock::ManualClock;
        use crate::state::Snapshot;
        use crate::tasks::NewTask;

        fn agent(name: &str) -> AgentId {
            AgentId::new(name).unwrap()
        }

        fn path(p: &str) -> RepoPath {
            RepoPath::new(p).unwrap()
        }

        fn policy() -> (Arc<LeadPolicy>, Arc<State>) {
            let state = Arc::new(State::new(
                Arc::new(ManualClock::new(t0())),
                Snapshot::default(),
            ));
            (Arc::new(LeadPolicy::new(Arc::clone(&state))), state)
        }

        /// Two equal-priority tasks, the second on `src/auth`, and a worker
        /// holding `src/auth`.
        fn board(state: &State) -> (Task, Task) {
            let make = |title: &str, p: &str| {
                state
                    .task_create(
                        agent("boss"),
                        NewTask::new(title)
                            .with_priority(5)
                            .with_paths(vec![path(p)]),
                    )
                    .unwrap()
            };
            let db = make("db", "src/db");
            let auth = make("auth", "src/auth");
            state
                .claim(agent("w"), vec![path("src/auth")], "r".into(), None)
                .unwrap();
            (db, auth)
        }

        #[tokio::test]
        async fn a_waiting_pull_takes_a_task_created_while_it_waits() {
            let (policy, state) = policy();
            let waiter = Arc::clone(&policy);
            let started = tokio::time::Instant::now();
            let pulling = tokio::spawn(async move {
                waiter
                    .task_pull_waiting(agent("w"), Duration::from_secs(30))
                    .await
            });
            tokio::time::sleep(Duration::from_millis(100)).await;
            let late = state
                .task_create(agent("boss"), NewTask::new("late"))
                .unwrap();
            let pulled = pulling.await.unwrap().unwrap();
            assert_eq!(pulled.task.id, late.id);
            assert!(started.elapsed() < Duration::from_secs(5));
        }

        #[tokio::test]
        async fn a_waiting_pull_waits_out_a_held_task() {
            let (policy, state) = policy();
            let (db, auth) = board(&state);
            assert_eq!(state.task_pull(agent("x")).unwrap().task.id, db.id);

            // Only `auth` is left and `w` holds its path: at the deadline
            // the plain pull hands it out with `waiting_on`.
            let started = tokio::time::Instant::now();
            let pulled = policy
                .task_pull_waiting(agent("y"), Duration::from_millis(150))
                .await
                .unwrap();
            assert!(started.elapsed() >= Duration::from_millis(150));
            assert_eq!(pulled.task.id, auth.id);
            assert_eq!(pulled.waiting_on.len(), 1);

            // Back on the board, it is taken free once `w` releases.
            state
                .task_update(
                    agent("y"),
                    auth.id,
                    crate::tasks::TaskStatus::Todo,
                    None,
                    false,
                )
                .unwrap();
            let waiter = Arc::clone(&policy);
            let pulling = tokio::spawn(async move {
                waiter
                    .task_pull_waiting(agent("y"), Duration::from_secs(30))
                    .await
            });
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert!(!pulling.is_finished(), "held tasks are waited on");
            state.release(&agent("w"), None).unwrap();
            let pulled = pulling.await.unwrap().unwrap();
            assert_eq!(pulled.task.id, auth.id);
            assert!(pulled.waiting_on.is_empty());
        }

        #[tokio::test]
        async fn a_waiting_pull_with_nothing_to_pull_ends_in_none() {
            let (policy, _state) = policy();
            let started = tokio::time::Instant::now();
            let pulled = policy
                .task_pull_waiting(agent("w"), Duration::from_millis(150))
                .await;
            assert!(pulled.is_none());
            assert!(started.elapsed() >= Duration::from_millis(150));
        }

        #[test]
        fn a_notice_is_pushed_to_same_path_and_nested_holders_and_logged() {
            let (policy, state) = policy();
            for (who, p) in [
                ("same", "src/auth"),
                ("inner", "src/db/pool.rs"),
                ("away", "docs"),
            ] {
                state
                    .claim(agent(who), vec![path(p)], "r".into(), None)
                    .unwrap();
            }
            let notice = state
                .notice_publish(
                    agent("alice"),
                    crate::notices::NewNotice::new(
                        crate::notices::NoticeKind::Behavior,
                        "tokens refresh async",
                    )
                    .with_affected_paths(vec![path("src/auth"), path("src/db")]),
                )
                .unwrap();
            policy.notice_published(&notice);
            for who in ["same", "inner"] {
                let inbox = state.take_inbox(&agent(who));
                assert_eq!(inbox.messages.len(), 1, "{who}");
                assert!(inbox.messages[0].text.contains("tokens refresh async"));
            }
            assert!(state.take_inbox(&agent("away")).messages.is_empty());
            let rows = state.lead_log(5, Some(LeadEvent::NoticePublished));
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].candidates, ["inner", "same"]);
            assert_eq!(
                rows[0].details.as_ref().unwrap()["pushed"],
                json!(["inner", "same"])
            );
        }

        #[test]
        fn escalations_go_to_the_lead_or_the_human_queue() {
            let (policy, state) = policy();
            policy.raise(&Escalation::new(
                Trigger::TaskBlocked,
                agent("w"),
                "which order?",
            ));
            let row = &state.lead_log(1, Some(LeadEvent::EscalationRaised))[0];
            assert_eq!(row.rule.as_deref(), Some("no_lead"));
            assert_eq!(row.details.as_ref().unwrap()["route"], "human");

            state
                .claim(agent("boss"), vec![path(LEAD_PATH)], "swarm".into(), None)
                .unwrap();
            policy.raise(&Escalation::new(
                Trigger::TaskBlocked,
                agent("w"),
                "which order?",
            ));
            let row = &state.lead_log(1, Some(LeadEvent::EscalationRaised))[0];
            assert_eq!(row.details.as_ref().unwrap()["route"], "lead");
            assert_eq!(row.details.as_ref().unwrap()["delivered"], json!(["lead"]));

            policy.raise(&Escalation::new(
                Trigger::TaskBlocked,
                agent("w"),
                "need the API key",
            ));
            let row = &state.lead_log(1, Some(LeadEvent::EscalationRaised))[0];
            assert_eq!(row.rule.as_deref(), Some("human:credentials"));
            assert_eq!(
                row.details.as_ref().unwrap()["delivered"],
                json!(["human_queue", "lead"])
            );

            // The lead escalating itself has nobody else to ask.
            policy.raise(&Escalation::new(
                Trigger::TaskBlocked,
                agent("boss"),
                "stuck",
            ));
            let row = &state.lead_log(1, Some(LeadEvent::EscalationRaised))[0];
            assert_eq!(row.details.as_ref().unwrap()["route"], "human");
            assert_eq!(state.take_inbox(&agent("boss")).messages.len(), 2);
            assert_eq!(human_queue_of(&state).len(), 3);
        }
    }

    #[test]
    fn human_rules_match_phrases_at_word_boundaries() {
        for (text, rule) in [
            ("need a working STRIPE_API_KEY", "credentials"),
            ("add the new key to .env", "credentials"),
            ("someone ran sudo cargo install", "permissions"),
            ("can you grant my token repo:admin?", "permissions"),
            ("bind 0.0.0.0 so the team can watch", "permissions"),
            ("upgrade to the paid tier", "spending"),
            ("it costs $20/mo", "spending"),
            (
                "scrub it with git filter-repo and force-push",
                "destructive",
            ),
            ("OK to pkill the old daemon?", "destructive"),
            ("should I run `cargo publish`?", "destructive"),
            ("Human: which license?", "addressed_to_human"),
        ] {
            assert_eq!(human_rule(text), Some(rule), "{text}");
        }
        for text in [
            "the task key and the token budget",
            "granted two paths; the skill test passed",
            "delete the flaky file and its [[test]] entry",
            "the payload is 400 bytes; costs rose",
            "renamed LeadRoute::Human to LeadRoute::HumanQueue",
            "escalations fall to the human queue",
            "the .envrc loader",
            "$HOME is unset",
        ] {
            assert_eq!(human_rule(text), None, "{text}");
        }
    }

    fn escalation_row(
        log: &mut LeadLog,
        agent: &str,
        task: &str,
        delivered: &[&str],
        at: DateTime<Utc>,
    ) -> EntryId {
        let entry = NewEntry::new(LeadEvent::EscalationRaised, "routed")
            .with_agent(AgentId::new(agent).unwrap())
            .with_details(json!({ "trigger": "task_blocked", "task": task, "text": "t", "delivered": delivered }));
        log.append(entry, at, 1)
    }

    #[test]
    fn the_human_queue_ranks_by_agents_blocked_then_wait_and_drops_answered_items() {
        let mut log = LeadLog::default();
        let old = escalation_row(&mut log, "a", "task-1", &["human_queue"], t0());
        let later = t0() + chrono::Duration::minutes(5);
        let wide = escalation_row(&mut log, "b", "task-2", &["human_queue", "lead"], later);
        escalation_row(&mut log, "c", "task-2", &["lead"], later);
        let answered = escalation_row(&mut log, "d", "task-3", &["human_queue"], t0());
        escalation_row(&mut log, "e", "task-4", &[], t0());
        assert!(log.set_outcome(answered, json!({ "answered_by": "boss" })));
        let now = t0() + chrono::Duration::minutes(10);
        let queue = human_queue(log.entries(), &[], now);
        let ids: Vec<EntryId> = queue.iter().map(|i| i.id).collect();
        assert_eq!(ids, vec![wide, old]);
        assert_eq!(queue[0].blocked_agents.len(), 2, "{queue:?}");
        assert_eq!(queue[1].waited_secs, 600);
    }

    #[test]
    fn route_names_serialize_as_logged() {
        for route in [Route::Lead, Route::Human] {
            assert_eq!(serde_json::to_value(route).unwrap(), route.as_str());
        }
    }
}
