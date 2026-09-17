//! The experimental Jev sites (ADR-0024) through a real daemon.
//!
//! Jev itself is replaced by a scripted evaluator that answers from the
//! request's state, so these tests check the wiring and the fallbacks,
//! never the model, and make no network call. Every site is driven once
//! with answers and once where it matters with a failing evaluator, which
//! must leave Tirith's deterministic behavior untouched.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tirith::jev::{Answer, EvalFuture, Evaluator, JevError, Question, Request, Response};
use tirith::server::{ServerHandle, start};

use common::{call, options};

/// Answers every question from `state`: booleans are true when the item
/// they name mentions `relevant` or is agent `bob`; choices pick the
/// option whose description or item mentions `auth`, else the first.
#[derive(Debug)]
struct ByKeyword;

/// Fails every evaluation, as a timeout would.
#[derive(Debug)]
struct Down;

fn mentions(state: &Value, id: &str, needle: &str) -> bool {
    let Some(object) = state.as_object() else {
        return false;
    };
    object
        .values()
        .filter_map(|v| v.get(id))
        .any(|item| item.to_string().contains(needle))
}

fn answer(state: &Value, id: &str, question: &Question) -> Answer {
    match question {
        Question::Boolean { .. } => {
            let yes = mentions(state, id, "relevant") || mentions(state, id, "\"bob\"");
            Answer::Boolean {
                probability: if yes { 0.9 } else { 0.05 },
            }
        }
        Question::Choice { criteria, .. } => {
            let pick = criteria
                .iter()
                .find(|(key, description)| {
                    description.as_deref().is_some_and(|d| d.contains("auth"))
                        || mentions(state, key, "auth")
                })
                .or_else(|| criteria.iter().next())
                .map(|(key, _)| key.clone())
                .unwrap();
            let probabilities = criteria
                .keys()
                .map(|k| (k.clone(), if *k == pick { 0.9 } else { 0.1 }))
                .collect();
            Answer::Choice {
                choice: pick,
                probabilities,
                confidence: None,
            }
        }
        Question::Score { .. } => Answer::Score {
            score: 0.0,
            probabilities: BTreeMap::new(),
            confidence: None,
        },
    }
}

impl Evaluator for ByKeyword {
    fn evaluate(&self, request: Request) -> EvalFuture<'_> {
        let answers = request
            .questions
            .iter()
            .map(|(id, q)| (id.clone(), answer(&request.state, id, q)))
            .collect();
        Box::pin(async move {
            Ok(Response {
                answers,
                input_tokens: 50,
                output_tokens: 0,
                cost_usd: Some(0.000_002_1),
                latency: Duration::from_millis(1),
            })
        })
    }

    fn model(&self) -> &'static str {
        "by-keyword"
    }
}

impl Evaluator for Down {
    fn evaluate(&self, _: Request) -> EvalFuture<'_> {
        Box::pin(async { Err(JevError::Timeout) })
    }

    fn model(&self) -> &'static str {
        "down"
    }
}

async fn daemon(evaluator: Arc<dyn Evaluator>) -> (tempfile::TempDir, ServerHandle) {
    let dir = tempfile::tempdir().unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();
    assert!(handle.enable_assist(evaluator));
    (dir, handle)
}

fn summaries(rows: &Value) -> Vec<&str> {
    rows.as_array()
        .map(|a| a.iter().filter_map(|r| r["summary"].as_str()).collect())
        .unwrap_or_default()
}

async fn publish(handle: &ServerHandle, by: &str, summary: &str, path: &str) {
    let out = call(
        handle,
        "notice_publish",
        json!({ "agent": by, "kind": "behavior", "summary": summary, "affected_paths": [path] }),
    )
    .await;
    assert_eq!(out["status"], "ok", "{out}");
}

#[tokio::test]
async fn the_brief_leaves_out_what_the_task_does_not_need() {
    let (_dir, handle) = daemon(Arc::new(ByKeyword)).await;
    publish(
        &handle,
        "alice",
        "relevant: token refresh now async",
        "src/auth",
    )
    .await;
    publish(&handle, "alice", "log format tweak", "src/auth").await;

    let out = call(
        &handle,
        "claim",
        json!({ "agent": "bob", "paths": ["src/auth/token.rs"], "reason": "fix refresh" }),
    )
    .await;
    assert_eq!(out["status"], "ok", "{out}");
    assert_eq!(
        summaries(&out["notices"]),
        ["relevant: token refresh now async"]
    );
    assert_eq!(out["skipped"]["notices"], 1);

    // The skipped notice was not delivered, so it is still unread.
    let unread = call(
        &handle,
        "notice_list",
        json!({ "agent": "bob", "unread": true }),
    )
    .await;
    assert_eq!(summaries(&unread["notices"]), ["log format tweak"]);

    let status = call(&handle, "status", json!({})).await;
    assert_eq!(status["jev"]["model"], "by-keyword");
    assert!(status["jev"]["sites"]["brief"]["calls"].as_u64().unwrap() >= 1);
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_failing_jev_leaves_the_brief_as_it_was() {
    let (_dir, handle) = daemon(Arc::new(Down)).await;
    publish(&handle, "alice", "one", "src/a").await;
    publish(&handle, "alice", "two", "src/a").await;
    let out = call(
        &handle,
        "claim",
        json!({ "agent": "bob", "paths": ["src/a"], "reason": "x" }),
    )
    .await;
    assert_eq!(summaries(&out["notices"]), ["two", "one"]);
    assert!(out.get("skipped").is_none());
    let report = handle.assist_report().unwrap();
    assert_eq!(report.calls, report.failures);
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_notice_reaches_the_holder_it_affects_without_a_new_claim() {
    let (_dir, handle) = daemon(Arc::new(ByKeyword)).await;
    for (agent, path) in [("bob", "src/auth"), ("carol", "src/billing")] {
        let out = call(
            &handle,
            "claim",
            json!({ "agent": agent, "paths": [path], "reason": "work" }),
        )
        .await;
        assert_eq!(out["status"], "ok");
    }
    publish(
        &handle,
        "alice",
        "Session::new takes a clock",
        "src/session.rs",
    )
    .await;

    // The push runs in the background; poll bob's next results for it.
    let mut inbox = Value::Null;
    for _ in 0..50 {
        let out = call(&handle, "status", json!({ "agent": "bob" })).await;
        if let Some(rows) = out.get("inbox") {
            inbox = rows.clone();
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let text = inbox[0]["text"].as_str().unwrap_or_default();
    assert_eq!(inbox[0]["from"], "tirith", "{inbox}");
    assert!(text.contains("Session::new takes a clock"), "{text}");
    let carol = call(&handle, "status", json!({ "agent": "carol" })).await;
    assert!(carol.get("inbox").is_none(), "{carol}");
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn task_pull_prefers_the_task_near_the_agents_work_within_a_priority() {
    let (_dir, handle) = daemon(Arc::new(ByKeyword)).await;
    for (title, priority) in [("billing export", 1), ("auth cleanup", 1), ("docs pass", 2)] {
        call(
            &handle,
            "task_create",
            json!({ "agent": "alice", "title": title, "priority": priority }),
        )
        .await;
    }
    // Priority first, and no context to match yet: plain pull order.
    let first = call(&handle, "task_pull", json!({ "agent": "bob" })).await;
    assert_eq!(first["task"]["title"], "docs pass");
    assert!(first.get("picked_by").is_none());

    // Now bob has a task in context and two equal-priority tasks to pick
    // from; age alone would give it the billing export.
    let out = call(&handle, "task_pull", json!({ "agent": "bob" })).await;
    assert_eq!(out["task"]["title"], "auth cleanup", "{out}");
    assert_eq!(out["picked_by"], "jev");
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_duplicate_task_and_a_conflict_get_hints() {
    let (_dir, handle) = daemon(Arc::new(ByKeyword)).await;
    call(
        &handle,
        "task_create",
        json!({ "agent": "alice", "title": "auth token refresh" }),
    )
    .await;
    let out = call(
        &handle,
        "task_create",
        json!({ "agent": "bob", "title": "refresh auth tokens" }),
    )
    .await;
    assert_eq!(
        out["possible_duplicate"]["title"], "auth token refresh",
        "{out}"
    );

    call(
        &handle,
        "claim",
        json!({ "agent": "alice", "paths": ["src/a.rs"], "reason": "auth rewrite" }),
    )
    .await;
    let refused = call(
        &handle,
        "claim",
        json!({ "agent": "bob", "paths": ["src/a.rs"], "reason": "typo" }),
    )
    .await;
    assert_eq!(refused["status"], "conflict");
    assert!(refused["advice"]["action"].is_string(), "{refused}");
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn searches_rank_by_meaning_and_broadcasts_skip_the_unconcerned() {
    let (_dir, handle) = daemon(Arc::new(ByKeyword)).await;
    for (title, body) in [
        ("Leases", "relevant: leases end after four TTLs"),
        ("Dashboard colors", "the palette"),
    ] {
        call(
            &handle,
            "memory_write",
            json!({ "agent": "alice", "title": title, "body": body }),
        )
        .await;
    }
    let found = call(
        &handle,
        "memory_search",
        json!({ "agent": "alice", "query": "how long can an agent hold a path" }),
    )
    .await;
    assert_eq!(found["ranked_by"], "jev");
    let titles: Vec<&str> = found["notes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, ["Leases"]);

    call(
        &handle,
        "decision_record",
        json!({ "agent": "alice", "title": "Lease cap", "decision": "relevant: four TTLs" }),
    )
    .await;
    let decisions = call(
        &handle,
        "decision_list",
        json!({ "agent": "alice", "query": "max hold time" }),
    )
    .await;
    assert_eq!(decisions["ranked_by"], "jev");
    assert_eq!(decisions["count"], 1);

    for agent in ["bob", "carol"] {
        call(&handle, "status", json!({ "agent": agent })).await;
    }
    let sent = call(
        &handle,
        "message_send",
        json!({ "agent": "alice", "to": "*", "text": "rebasing main" }),
    )
    .await;
    assert_eq!(sent["skipped_recipients"], 1, "{sent}");
    let carol = call(&handle, "status", json!({ "agent": "carol" })).await;
    assert!(carol.get("inbox").is_none());
    let bob = call(&handle, "status", json!({ "agent": "bob" })).await;
    assert_eq!(bob["inbox"][0]["text"], "rebasing main");
    handle.shutdown().await.unwrap();
}
