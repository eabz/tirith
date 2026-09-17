//! Jev-assisted coordination. Experimental ([ADR-0024]).
//!
//! Every function here is one place where Tirith asks [Jev](crate::jev) a
//! question instead of applying a fixed rule, and every one of them has
//! the same contract: when Jev is unreachable, slow, or unsure, the answer
//! is exactly what Tirith would have done without it. Nothing that must
//! be exact goes through here: claims, leases, overlap, persistence, and
//! task ownership stay deterministic.
//!
//! | site | question | effect |
//! |---|---|---|
//! | [`Site::Brief`] | must the agent read this row before editing? | irrelevant brief rows are left out |
//! | [`Site::TaskPull`] | which equal-priority task suits this agent? | affinity instead of age |
//! | [`Site::NoticeFanout`] | does this change break that agent's work? | the agent gets an inbox message now |
//! | [`Site::Broadcast`] | does this agent need this broadcast? | `*` skips agents it does not concern |
//! | [`Site::MemorySearch`] | does this note help with the query? | semantic rerank and filter |
//! | [`Site::DecisionSearch`] | does this decision answer the query? | semantic `decision_list` query |
//! | [`Site::ConflictAdvice`] | wait, work elsewhere, coordinate, or narrow? | `advice` on a refused claim |
//! | [`Site::TaskDuplicate`] | does an open task already cover this? | `possible_duplicate` hint |
//! | [`Site::NoteDuplicate`] | does a note already say this? | `similar_note` hint |
//!
//! The module decides; `server.rs` fetches candidates from `State`, calls
//! one function here, and applies the result.
//!
//! [ADR-0024]: https://github.com/eabz/tirith/blob/main/docs/5-decisions/0024-jev-assist-experiment.md

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::claims::{Claim, Conflict};
use crate::decisions::Decision;
use crate::jev::{Evaluator, Question, Request, Response};
use crate::memory::MemoryNote;
use crate::notices::{Notice, NoticeKind};
use crate::state::{BRIEF_LIMIT, Brief, BriefMore};
use crate::tasks::Task;
use crate::types::{AgentId, RepoPath};

/// Rows per brief section Jev judges; older rows count toward `more`.
pub const BRIEF_CANDIDATES: usize = 15;
/// Notes a search hands Jev to rerank.
pub const NOTE_CANDIDATES: usize = 30;
/// Equal-priority tasks Jev chooses among.
const TASK_CANDIDATES: usize = 20;
/// Existing tasks or notes a duplicate check compares against.
pub const DUPLICATE_CANDIDATES: usize = 50;
/// Characters of prose per item sent to Jev.
const EXCERPT: usize = 280;

/// A brief row is kept at or above this P(must read).
const BRIEF_KEEP: f64 = 0.5;
/// Lower bar for renames, signature changes, removals, and moves.
const BRIEF_KEEP_BREAKING: f64 = 0.25;
/// A task choice is followed at or above this probability.
const TASK_PICK: f64 = 0.4;
/// A holder is messaged about a notice at or above this probability.
const FANOUT_NOTIFY: f64 = 0.5;
/// A broadcast recipient is kept at or above this probability.
const BROADCAST_KEEP: f64 = 0.35;
/// A searched note or decision is kept at or above this probability.
const SEARCH_KEEP: f64 = 0.3;
/// A duplicate is reported at or above this probability.
const DUPLICATE_FLAG: f64 = 0.6;
/// Conflict advice is given at or above this probability.
const ADVICE_MIN: f64 = 0.4;

/// Where Tirith asked Jev something.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Site {
    /// Filtering a claim brief.
    Brief,
    /// Choosing among equal-priority tasks.
    TaskPull,
    /// Pushing a notice to agents it affects.
    NoticeFanout,
    /// Narrowing a `*` message.
    Broadcast,
    /// Reranking a memory search.
    MemorySearch,
    /// Answering a decision query by meaning.
    DecisionSearch,
    /// Advising a refused claim.
    ConflictAdvice,
    /// Spotting a duplicate task.
    TaskDuplicate,
    /// Spotting a duplicate note.
    NoteDuplicate,
}

/// Counters for one [`Site`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct SiteStats {
    /// Evaluations attempted.
    pub calls: u64,
    /// Evaluations that fell back.
    pub failures: u64,
    /// Questions asked across all calls.
    pub questions: u64,
    /// Input tokens billed.
    pub input_tokens: u64,
    /// Cost in USD as reported by the gateway.
    pub cost_usd: f64,
    /// Sum of round-trip times.
    pub total_ms: u64,
    /// Slowest round trip.
    pub max_ms: u64,
}

/// What `status` shows about Jev while it is enabled.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AssistReport {
    /// The model asked.
    pub model: String,
    /// Evaluations attempted.
    pub calls: u64,
    /// Evaluations that fell back.
    pub failures: u64,
    /// Input tokens billed.
    pub input_tokens: u64,
    /// Cost in USD.
    pub cost_usd: f64,
    /// Mean round trip.
    pub avg_ms: u64,
    /// Per-site counters, only for sites that were used.
    pub sites: BTreeMap<Site, SiteStats>,
}

/// A brief after Jev judged it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BriefSelection {
    /// The rows to show, most relevant first, and the relevant rows left out.
    pub brief: Brief,
    /// Per section, the rows Jev judged irrelevant.
    pub skipped: BriefMore,
}

/// What a refused claim should do next.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Advice {
    /// `wait`, `work_elsewhere`, `coordinate`, or `narrow_claim`.
    pub action: &'static str,
    /// Jev's probability for that action.
    pub confidence: f64,
    /// What the action means.
    pub hint: &'static str,
}

const ADVICE: [(&str, &str); 4] = [
    (
        "wait",
        "Wait for the other lease: it ends soon and the rest of the task depends on these paths.",
    ),
    (
        "work_elsewhere",
        "Do other work that does not touch these paths; the other agent's task is long or unrelated.",
    ),
    (
        "coordinate",
        "Message the owner: both tasks need to agree on a change to the shared code.",
    ),
    (
        "narrow_claim",
        "Claim only the requested paths that do not overlap; the overlap is incidental.",
    ),
];

/// Jev plus the counters for every site. One per daemon.
#[derive(Debug)]
pub struct Assist {
    evaluator: Arc<dyn Evaluator>,
    stats: Mutex<BTreeMap<Site, SiteStats>>,
}

impl Assist {
    /// Asks `evaluator` at every site.
    pub fn new(evaluator: Arc<dyn Evaluator>) -> Self {
        Self {
            evaluator,
            stats: Mutex::new(BTreeMap::new()),
        }
    }

    /// Totals and per-site counters so far.
    pub fn report(&self) -> AssistReport {
        let sites = self
            .stats
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let calls = sites.values().map(|s| s.calls).sum();
        let total_ms: u64 = sites.values().map(|s| s.total_ms).sum();
        AssistReport {
            model: self.evaluator.model().to_owned(),
            calls,
            failures: sites.values().map(|s| s.failures).sum(),
            input_tokens: sites.values().map(|s| s.input_tokens).sum(),
            cost_usd: sites.values().map(|s| s.cost_usd).sum(),
            avg_ms: total_ms.checked_div(calls).unwrap_or(0),
            sites,
        }
    }

    /// One evaluation, counted. `None` on any failure; the caller falls back.
    async fn ask(&self, site: Site, request: Request) -> Option<Response> {
        if request.is_empty() {
            return None;
        }
        let questions = u64::try_from(request.questions.len()).unwrap_or(u64::MAX);
        let started = Instant::now();
        let result = self.evaluator.evaluate(request).await;
        let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let mut stats = self.stats.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = stats.entry(site).or_default();
        entry.calls += 1;
        entry.questions += questions;
        entry.total_ms += ms;
        entry.max_ms = entry.max_ms.max(ms);
        match result {
            Ok(response) => {
                entry.input_tokens += response.input_tokens;
                entry.cost_usd += response.cost_usd.unwrap_or(0.0);
                drop(stats);
                tracing::info!(?site, ms, questions, tokens = response.input_tokens, "jev");
                Some(response)
            }
            Err(error) => {
                entry.failures += 1;
                drop(stats);
                tracing::warn!(?site, ms, %error, "jev failed; falling back");
                None
            }
        }
    }

    /// Keeps the brief rows the agent must read before working on `paths`
    /// for `reason`, most relevant first, [`BRIEF_LIMIT`] per section.
    ///
    /// `candidates` holds up to [`BRIEF_CANDIDATES`] rows per section, with
    /// `more` counting the older rows nobody judged; those stay in `more`.
    pub async fn select_brief(
        &self,
        reason: &str,
        paths: &[RepoPath],
        candidates: Brief,
    ) -> BriefSelection {
        let mut items = Map::new();
        for (i, n) in candidates.notices.iter().enumerate() {
            items.insert(
                format!("n{i}"),
                json!({
                    "type": "change notice", "kind": n.kind, "summary": n.summary,
                    "from": n.from, "to": n.to, "affected_paths": n.affected_paths,
                }),
            );
        }
        for (i, c) in candidates.contracts.iter().enumerate() {
            items.insert(
                format!("c{i}"),
                json!({
                    "type": "interface contract", "name": c.name, "kind": c.kind,
                    "consumers": c.consumers, "notes": excerpt(&c.current.notes),
                }),
            );
        }
        for (i, d) in candidates.decisions.iter().enumerate() {
            items.insert(
                format!("d{i}"),
                json!({
                    "type": "settled decision", "title": d.title,
                    "decision": excerpt(&d.decision),
                }),
            );
        }
        for (i, m) in candidates.memory.iter().enumerate() {
            items.insert(
                format!("m{i}"),
                json!({
                    "type": "memory note", "kind": m.kind, "title": m.title,
                    "excerpt": excerpt(&m.body),
                }),
            );
        }
        let mut request = Request::new(Value::Null);
        for id in items.keys() {
            request = request.ask(
                id.clone(),
                Question::boolean(format!(
                    "The agent is about to edit claimed_paths to do `task`. Must it read item {id} \
                     first? True only if ignoring {id} could make its change wrong, break an \
                     interface, contradict a settled decision, or repeat a known mistake."
                )),
            );
        }
        request.state = json!({ "task": reason, "claimed_paths": paths, "items": items });
        let Some(response) = self.ask(Site::Brief, request).await else {
            return deterministic_brief(candidates);
        };
        let more = candidates.more;
        let (notices, notices_more, notices_skipped) =
            keep_relevant(candidates.notices, "n", &response, |n| {
                if is_breaking(n.kind) {
                    BRIEF_KEEP_BREAKING
                } else {
                    BRIEF_KEEP
                }
            });
        let (contracts, contracts_more, contracts_skipped) =
            keep_relevant(candidates.contracts, "c", &response, |_| BRIEF_KEEP);
        let (decisions, decisions_more, decisions_skipped) =
            keep_relevant(candidates.decisions, "d", &response, |_| BRIEF_KEEP);
        let (memory, memory_more, memory_skipped) =
            keep_relevant(candidates.memory, "m", &response, |_| BRIEF_KEEP);
        BriefSelection {
            brief: Brief {
                notices,
                contracts,
                decisions,
                memory,
                more: BriefMore {
                    notices: more.notices + notices_more,
                    contracts: more.contracts + contracts_more,
                    decisions: more.decisions + decisions_more,
                    memory: more.memory + memory_more,
                },
            },
            skipped: BriefMore {
                notices: notices_skipped,
                contracts: contracts_skipped,
                decisions: decisions_skipped,
                memory: memory_skipped,
            },
        }
    }

    /// Among the highest-priority unblocked tasks, the one closest to what
    /// `agent` already holds and recently did. `None` means take the
    /// deterministic pick.
    ///
    /// `candidates` must be in pull order (priority, then age). Jev is
    /// asked only when at least two tasks tie on the top priority and the
    /// agent has some context to match against; priority is never
    /// overridden.
    pub async fn choose_task(
        &self,
        agent: &AgentId,
        held: &[Claim],
        recent: &[Task],
        candidates: &[Task],
    ) -> Option<usize> {
        let top = candidates.first()?.priority;
        let tier: Vec<&Task> = candidates
            .iter()
            .take_while(|t| t.priority == top)
            .take(TASK_CANDIDATES)
            .collect();
        if tier.len() < 2 || (held.is_empty() && recent.is_empty()) {
            return None;
        }
        let tasks: Map<String, Value> = tier
            .iter()
            .enumerate()
            .map(|(i, t)| {
                (
                    format!("t{i}"),
                    json!({ "title": t.title, "description": excerpt(&t.description), "paths": t.paths }),
                )
            })
            .collect();
        let request = Request::new(json!({
            "agent": agent,
            "holds": holds(held),
            "recent_tasks": recent.iter().map(|t| json!({ "title": t.title, "paths": t.paths })).collect::<Vec<_>>(),
            "tasks": tasks,
        }))
        .ask(
            "next",
            Question::choice(
                "All tasks have equal priority. Which should this agent take next? Prefer the \
                 one closest to the files and work already in its context, so it re-reads the \
                 least code.",
                tier.iter()
                    .enumerate()
                    .map(|(i, t)| (format!("t{i}"), Some(excerpt(&t.title)))),
            ),
        );
        let response = self.ask(Site::TaskPull, request).await?;
        let (choice, p) = response.choice("next")?;
        if p < TASK_PICK {
            return None;
        }
        choice.strip_prefix('t')?.parse().ok()
    }

    /// The agents among `holders` whose current work `notice` likely
    /// breaks. Empty on failure, which is Tirith's behavior without Jev.
    pub async fn notice_audience(
        &self,
        notice: &Notice,
        holders: &[(AgentId, Vec<Claim>)],
    ) -> Vec<AgentId> {
        if holders.is_empty() {
            return Vec::new();
        }
        let request = agents_request(
            json!({
                "change": {
                    "kind": notice.kind, "summary": notice.summary, "from": notice.from,
                    "to": notice.to, "affected_paths": notice.affected_paths,
                },
            }),
            holders,
            |id| {
                format!(
                    "Will agent {id}'s in-progress work (the paths it holds and why) break, or \
                     need to change, because of this change?"
                )
            },
        );
        let Some(response) = self.ask(Site::NoticeFanout, request).await else {
            return Vec::new();
        };
        above(&response, holders, FANOUT_NOTIFY)
    }

    /// The recipients a broadcast should reach among `candidates`. `None`
    /// means everyone, as without Jev; so does a verdict that nobody should.
    pub async fn broadcast_audience(
        &self,
        text: &str,
        paths: &[RepoPath],
        candidates: &[(AgentId, Vec<Claim>)],
    ) -> Option<Vec<AgentId>> {
        if candidates.len() < 2 {
            return None;
        }
        let request = agents_request(
            json!({ "message": text, "about_paths": paths }),
            candidates,
            |id| {
                format!(
                    "Should agent {id} receive this broadcast? True if it concerns the paths {id} \
                     holds or the work it is doing, or is an announcement every agent needs. An \
                     agent holding nothing is between tasks and should get general announcements."
                )
            },
        );
        let response = self.ask(Site::Broadcast, request).await?;
        let kept = above(&response, candidates, BROADCAST_KEEP);
        (!kept.is_empty()).then_some(kept)
    }

    /// `candidates` reranked by how much each note helps with `query`,
    /// irrelevant notes dropped, each with its probability. `None` means
    /// keep the term ranking.
    pub async fn rank_notes(
        &self,
        query: &str,
        candidates: Vec<(MemoryNote, u32)>,
    ) -> Option<Vec<(MemoryNote, u32, f64)>> {
        let items: Vec<Value> = candidates.iter().map(|(n, _)| note_json(n)).collect();
        let ranked = self
            .rank(
                Site::MemorySearch,
                query,
                "notes",
                &items,
                "helps with the query? True if it answers it or holds knowledge someone asking \
                 this would need.",
            )
            .await?;
        let mut slots: Vec<Option<(MemoryNote, u32)>> = candidates.into_iter().map(Some).collect();
        Some(
            ranked
                .into_iter()
                .filter_map(|(i, p)| slots.get_mut(i)?.take().map(|(n, s)| (n, s, p)))
                .collect(),
        )
    }

    /// The decisions among `candidates` that answer `query`, most relevant
    /// first, each with its probability. `None` means fall back to the
    /// substring filter.
    pub async fn rank_decisions(
        &self,
        query: &str,
        candidates: Vec<Decision>,
    ) -> Option<Vec<(Decision, f64)>> {
        let items: Vec<Value> = candidates
            .iter()
            .map(|d| {
                json!({
                    "title": d.title, "decision": excerpt(&d.decision),
                    "rationale": excerpt(&d.rationale), "affects_paths": d.affects_paths,
                })
            })
            .collect();
        let ranked = self
            .rank(
                Site::DecisionSearch,
                query,
                "decisions",
                &items,
                "answers the query or settles the question it asks?",
            )
            .await?;
        let mut slots: Vec<Option<Decision>> = candidates.into_iter().map(Some).collect();
        Some(
            ranked
                .into_iter()
                .filter_map(|(i, p)| slots.get_mut(i)?.take().map(|d| (d, p)))
                .collect(),
        )
    }

    /// Indexes into `items` whose P(`{id} {question}`) clears
    /// [`SEARCH_KEEP`], most probable first.
    async fn rank(
        &self,
        site: Site,
        query: &str,
        field: &str,
        items: &[Value],
        question: &str,
    ) -> Option<Vec<(usize, f64)>> {
        if items.is_empty() {
            return None;
        }
        let rows: Map<String, Value> = items
            .iter()
            .enumerate()
            .map(|(i, v)| (format!("i{i}"), v.clone()))
            .collect();
        let mut request = Request::new(Value::Null);
        for id in rows.keys() {
            request = request.ask(
                id.clone(),
                Question::boolean(format!("Item {id} {question}")),
            );
        }
        let mut state = Map::new();
        state.insert("query".to_owned(), Value::from(query));
        state.insert(field.to_owned(), Value::Object(rows));
        request.state = Value::Object(state);
        let response = self.ask(site, request).await?;
        let mut ranked: Vec<(usize, f64)> = (0..items.len())
            .filter_map(|i| {
                let p = response.probability(&format!("i{i}"))?;
                (p >= SEARCH_KEEP).then_some((i, p))
            })
            .collect();
        // Stable: equally relevant items keep the caller's order.
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
        Some(ranked)
    }

    /// What an agent refused `requested` for `reason` should do next.
    pub async fn conflict_advice(
        &self,
        reason: &str,
        requested: &[RepoPath],
        conflicts: &[Conflict],
        now: DateTime<Utc>,
    ) -> Option<Advice> {
        let held: Vec<Value> = conflicts
            .iter()
            .map(|c| {
                json!({
                    "requested": c.path, "held": c.overlaps, "owner": c.owner,
                    "owner_task": c.reason,
                    "lease_ends_in_secs": (c.expires_at - now).num_seconds().max(0),
                })
            })
            .collect();
        let request = Request::new(json!({
            "task": reason, "requested_paths": requested, "held_by_others": held,
        }))
        .ask(
            "action",
            Question::choice(
                "This agent's claim was refused because other agents hold overlapping paths. \
                 What should it do next?",
                ADVICE.map(|(name, hint)| (name, Some(hint.to_owned()))),
            ),
        );
        let response = self.ask(Site::ConflictAdvice, request).await?;
        let (choice, confidence) = response.choice("action")?;
        let (action, hint) = ADVICE.into_iter().find(|(name, _)| *name == choice)?;
        (confidence >= ADVICE_MIN).then_some(Advice {
            action,
            confidence,
            hint,
        })
    }

    /// The index in `open` of a task that already covers `new`, with its
    /// probability.
    pub async fn duplicate_task(&self, new: &Task, open: &[Task]) -> Option<(usize, f64)> {
        let existing: Vec<Value> = open.iter().map(task_json).collect();
        self.duplicate(
            Site::TaskDuplicate,
            json!({ "new_task": task_json(new) }),
            "open_tasks",
            &existing,
            "Does one of open_tasks already cover the work of new_task?",
            "No; new_task is distinct work.",
        )
        .await
    }

    /// The index in `others` of a note that already says what `note` says,
    /// with its probability.
    pub async fn similar_note(
        &self,
        note: &MemoryNote,
        others: &[MemoryNote],
    ) -> Option<(usize, f64)> {
        let existing: Vec<Value> = others.iter().map(note_json).collect();
        self.duplicate(
            Site::NoteDuplicate,
            json!({ "new_note": note_json(note) }),
            "existing_notes",
            &existing,
            "Does one of existing_notes already record the same knowledge as new_note, so the \
             two should be one note?",
            "No; new_note records something new.",
        )
        .await
    }

    async fn duplicate(
        &self,
        site: Site,
        mut state: Value,
        field: &str,
        existing: &[Value],
        instructions: &str,
        none: &str,
    ) -> Option<(usize, f64)> {
        if existing.is_empty() {
            return None;
        }
        let rows: Map<String, Value> = existing
            .iter()
            .take(DUPLICATE_CANDIDATES)
            .enumerate()
            .map(|(i, v)| (format!("x{i}"), v.clone()))
            .collect();
        let mut options = vec![("none".to_owned(), Some(none.to_owned()))];
        options.extend(rows.keys().map(|k| (k.clone(), None)));
        if let Some(object) = state.as_object_mut() {
            object.insert(field.to_owned(), Value::Object(rows));
        }
        let request = Request::new(state).ask("duplicate", Question::choice(instructions, options));
        let response = self.ask(site, request).await?;
        let (choice, p) = response.choice("duplicate")?;
        let index = choice.strip_prefix('x')?.parse().ok()?;
        (p >= DUPLICATE_FLAG).then_some((index, p))
    }
}

/// The brief Tirith shows without Jev: newest [`BRIEF_LIMIT`] per section.
pub fn deterministic_brief(candidates: Brief) -> BriefSelection {
    fn cap<T>(mut rows: Vec<T>, more: &mut usize) -> Vec<T> {
        *more += rows.len().saturating_sub(BRIEF_LIMIT);
        rows.truncate(BRIEF_LIMIT);
        rows
    }
    let mut more = candidates.more;
    BriefSelection {
        brief: Brief {
            notices: cap(candidates.notices, &mut more.notices),
            contracts: cap(candidates.contracts, &mut more.contracts),
            decisions: cap(candidates.decisions, &mut more.decisions),
            memory: cap(candidates.memory, &mut more.memory),
            more,
        },
        skipped: BriefMore::default(),
    }
}

/// Rows at or above their threshold, most relevant first, capped at
/// [`BRIEF_LIMIT`]; plus how many relevant rows the cap left out and how
/// many were judged irrelevant. A row Jev did not answer for is kept.
fn keep_relevant<T>(
    rows: Vec<T>,
    prefix: &str,
    response: &Response,
    threshold: impl Fn(&T) -> f64,
) -> (Vec<T>, usize, usize) {
    let total = rows.len();
    let mut scored: Vec<(f64, T)> = rows
        .into_iter()
        .enumerate()
        .map(|(i, row)| {
            let p = response.probability(&format!("{prefix}{i}")).unwrap_or(1.0);
            (p, row)
        })
        .filter(|(p, row)| *p >= threshold(row))
        .collect();
    let skipped = total - scored.len();
    // Stable: equally relevant rows stay newest first.
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    let more = scored.len().saturating_sub(BRIEF_LIMIT);
    let kept = scored
        .into_iter()
        .take(BRIEF_LIMIT)
        .map(|(_, r)| r)
        .collect();
    (kept, more, skipped)
}

fn is_breaking(kind: NoticeKind) -> bool {
    matches!(
        kind,
        NoticeKind::Rename | NoticeKind::Signature | NoticeKind::Removed | NoticeKind::Moved
    )
}

/// A request with one boolean question per agent, `a0`, `a1`, ..., and the
/// agents with what they hold added to `state`.
fn agents_request(
    mut state: Value,
    agents: &[(AgentId, Vec<Claim>)],
    question: impl Fn(&str) -> String,
) -> Request {
    let rows: Map<String, Value> = agents
        .iter()
        .enumerate()
        .map(|(i, (name, claims))| {
            (
                format!("a{i}"),
                json!({ "name": name, "holds": holds(claims) }),
            )
        })
        .collect();
    let mut request = Request::new(Value::Null);
    for id in rows.keys() {
        request = request.ask(id.clone(), Question::boolean(question(id)));
    }
    if let Some(object) = state.as_object_mut() {
        object.insert("agents".to_owned(), Value::Object(rows));
    }
    request.state = state;
    request
}

/// The agents whose `a{i}` probability is at or above `threshold`.
fn above(response: &Response, agents: &[(AgentId, Vec<Claim>)], threshold: f64) -> Vec<AgentId> {
    agents
        .iter()
        .enumerate()
        .filter(|(i, _)| {
            response
                .probability(&format!("a{i}"))
                .is_some_and(|p| p >= threshold)
        })
        .map(|(_, (agent, _))| agent.clone())
        .collect()
}

fn holds(claims: &[Claim]) -> Vec<Value> {
    claims
        .iter()
        .map(|c| json!({ "paths": c.paths, "reason": c.reason }))
        .collect()
}

fn task_json(task: &Task) -> Value {
    json!({ "title": task.title, "description": excerpt(&task.description), "paths": task.paths })
}

fn note_json(note: &MemoryNote) -> Value {
    json!({
        "title": note.title, "kind": note.kind, "paths": note.paths,
        "tags": note.tags, "excerpt": excerpt(&note.body),
    })
}

fn excerpt(text: &str) -> String {
    text.chars().take(EXCERPT).collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::Duration;

    use super::*;
    use crate::jev::{Answer, EvalFuture, JevError};
    use crate::notices::NewNotice;
    use crate::notices::NoticeBoard;

    type Script = dyn Fn(&Request) -> Result<BTreeMap<String, Answer>, JevError> + Send + Sync;

    struct Scripted(Box<Script>);

    impl std::fmt::Debug for Scripted {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("Scripted")
        }
    }

    impl Evaluator for Scripted {
        fn evaluate(&self, request: Request) -> EvalFuture<'_> {
            let result = (self.0)(&request).map(|answers| Response {
                answers,
                input_tokens: 100,
                output_tokens: 0,
                cost_usd: Some(0.000_004_2),
                latency: Duration::from_millis(1),
            });
            Box::pin(async move { result })
        }

        fn model(&self) -> &'static str {
            "scripted"
        }
    }

    fn assist(
        script: impl Fn(&Request) -> Result<BTreeMap<String, Answer>, JevError> + Send + Sync + 'static,
    ) -> Assist {
        Assist::new(Arc::new(Scripted(Box::new(script))))
    }

    fn boolean(p: f64) -> Answer {
        Answer::Boolean { probability: p }
    }

    fn notices(summaries: &[(&str, NoticeKind)]) -> Vec<Notice> {
        let mut book = NoticeBoard::default();
        let now = Utc::now();
        summaries
            .iter()
            .map(|(s, kind)| {
                book.publish(
                    AgentId::new("pub").unwrap(),
                    NewNotice::new(*kind, *s)
                        .with_affected_paths(vec![RepoPath::new("src/a.rs").unwrap()]),
                    now,
                )
                .unwrap()
                .clone()
            })
            .collect()
    }

    #[tokio::test]
    async fn brief_keeps_relevant_rows_most_relevant_first() {
        let candidates = Brief {
            notices: notices(&[
                ("irrelevant", NoticeKind::Behavior),
                ("weakly relevant rename", NoticeKind::Rename),
                ("relevant", NoticeKind::Behavior),
            ]),
            ..Brief::default()
        };
        let assist = assist(|request| {
            assert_eq!(request.questions.len(), 3);
            Ok([("n0", 0.1), ("n1", 0.3), ("n2", 0.9)]
                .into_iter()
                .map(|(k, p)| (k.to_owned(), boolean(p)))
                .collect())
        });
        let selection = assist
            .select_brief("fix a", &[RepoPath::new("src/a.rs").unwrap()], candidates)
            .await;
        let kept: Vec<&str> = selection
            .brief
            .notices
            .iter()
            .map(|n| n.summary.as_str())
            .collect();
        // The rename clears the lower bar for breaking kinds.
        assert_eq!(kept, ["relevant", "weakly relevant rename"]);
        assert_eq!(selection.skipped.notices, 1);
        assert_eq!(selection.brief.more.notices, 0);
        let report = assist.report();
        assert_eq!(report.calls, 1);
        assert_eq!(report.sites[&Site::Brief].questions, 3);
    }

    #[tokio::test]
    async fn every_site_falls_back_when_jev_fails() {
        let assist = assist(|_| Err(JevError::Timeout));
        let rows: Vec<Notice> = notices(&[("x", NoticeKind::Behavior); 7]);
        let selection = assist
            .select_brief(
                "t",
                &[],
                Brief {
                    notices: rows,
                    ..Brief::default()
                },
            )
            .await;
        assert_eq!(selection.brief.notices.len(), BRIEF_LIMIT);
        assert_eq!(selection.brief.more.notices, 2);
        let holders = vec![(AgentId::new("b").unwrap(), Vec::new())];
        let notice = notices(&[("x", NoticeKind::Signature)]).remove(0);
        assert!(assist.notice_audience(&notice, &holders).await.is_empty());
        assert_eq!(assist.report().failures, 2);
    }

    #[tokio::test]
    async fn a_broadcast_nobody_needs_still_reaches_everyone() {
        let assist = assist(|request| {
            Ok(request
                .questions
                .keys()
                .map(|k| (k.clone(), boolean(0.01)))
                .collect())
        });
        let agents = vec![
            (AgentId::new("a").unwrap(), Vec::new()),
            (AgentId::new("b").unwrap(), Vec::new()),
        ];
        assert_eq!(assist.broadcast_audience("hi", &[], &agents).await, None);
    }
}
