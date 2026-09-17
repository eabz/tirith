//! Escalation routing by the lead policy (ADR-0027 section 3) through a
//! real daemon.
//!
//! Routing is deterministic: text matching a human rule goes to the human
//! queue and the lead is told; anything else goes to the lead's inbox; with
//! no lead it goes to the human queue. The tests check the triggers, every
//! route's delivery, the human queue and the outcome fields.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use serde_json::{Value, json};
use tirith::lead::{LeadEntry, LeadEvent, human_queue_of};
use tirith::server::{ServerHandle, start};

use common::{call, options};

/// A daemon where `worker` owns an in-progress task, and `boss` holds the
/// lead lease when `lead` is set.
async fn swarm(lead: bool) -> (tempfile::TempDir, ServerHandle, String) {
    let dir = tempfile::tempdir().unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();
    if lead {
        let out = call(
            &handle,
            "claim",
            json!({ "agent": "boss", "paths": [".tirith/lead"], "reason": "swarm", "brief": false }),
        )
        .await;
        assert_eq!(out["status"], "ok", "{out}");
    }
    let task = call(
        &handle,
        "task_create",
        json!({ "agent": "boss", "title": "matcher", "paths": ["src/matcher.rs"] }),
    )
    .await;
    let task_id = task["task"]["id"].as_str().unwrap().to_owned();
    call(
        &handle,
        "task_update",
        json!({ "agent": "worker", "task_id": task_id, "status": "in_progress" }),
    )
    .await;
    (dir, handle, task_id)
}

/// Every escalation row, newest first.
fn escalations(handle: &ServerHandle) -> Vec<LeadEntry> {
    handle
        .state()
        .lead_log(50, Some(LeadEvent::EscalationRaised))
}

/// Every message from `tirith` in `agent`'s inbox, read once.
async fn from_tirith(handle: &ServerHandle, agent: &str) -> Vec<String> {
    tirith_texts(&call(handle, "status", json!({ "agent": agent })).await)
}

/// The texts of the messages from `tirith` riding on one result.
fn tirith_texts(out: &Value) -> Vec<String> {
    out["inbox"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter(|m| m["from"] == "tirith")
                .filter_map(|m| m["text"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

async fn block(handle: &ServerHandle, task: &str, note: &str) -> Value {
    call(
        handle,
        "task_update",
        json!({ "agent": "worker", "task_id": task, "status": "blocked", "note": note }),
    )
    .await
}

fn details(row: &LeadEntry) -> &Value {
    row.details.as_ref().unwrap()
}

#[tokio::test]
async fn a_block_goes_to_the_lead_and_unblocking_answers_it() {
    let (_dir, handle, task) = swarm(true).await;
    let out = block(&handle, &task, "which order for t2 and t3?").await;
    assert_eq!(out["status"], "ok", "{out}");
    assert!(tirith_texts(&out).is_empty(), "{out}");
    let rows = escalations(&handle);
    assert_eq!(rows.len(), 1, "{rows:?}");
    let row = &rows[0];
    assert_eq!(row.agent.as_ref().unwrap().as_str(), "worker");
    assert_eq!(row.action, "told the lead boss");
    assert!(row.rule.is_none(), "{row:?}");
    let details = details(row);
    assert_eq!(details["trigger"], "task_blocked");
    assert_eq!(details["task"], task.as_str());
    assert_eq!(details["route"], "lead");
    assert_eq!(details["delivered"], json!(["lead"]));

    let lead = from_tirith(&handle, "boss").await;
    assert_eq!(lead.len(), 1, "{lead:?}");
    assert!(
        lead[0].starts_with("escalation from worker on task"),
        "{lead:?}"
    );
    assert!(lead[0].ends_with("which order for t2 and t3?"), "{lead:?}");
    assert!(from_tirith(&handle, "worker").await.is_empty());
    assert!(human_queue_of(handle.state()).is_empty());

    // Unblocking the task answers the escalation.
    call(
        &handle,
        "task_update",
        json!({ "agent": "worker", "task_id": task, "status": "in_progress" }),
    )
    .await;
    let outcome = escalations(&handle)[0].outcome.clone().unwrap();
    assert_eq!(outcome["via"], "task_in_progress", "{outcome}");
    assert_eq!(outcome["answered_by"], "worker");
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_human_rule_queues_the_item_tells_the_lead_and_an_answer_clears_it() {
    let (_dir, handle, task) = swarm(true).await;
    block(&handle, &task, "need a working API key in .env").await;
    let rows = escalations(&handle);
    let row = &rows[0];
    assert_eq!(row.rule.as_deref(), Some("human:credentials"));
    assert_eq!(row.action, "queued for the human");
    assert_eq!(details(row)["route"], "human");
    assert_eq!(details(row)["delivered"], json!(["human_queue", "lead"]));
    let lead = from_tirith(&handle, "boss").await;
    assert!(lead[0].starts_with("needs the human"), "{lead:?}");

    let url = format!("{}api/human", handle.dashboard_url());
    let body = reqwest::get(&url).await.unwrap().text().await.unwrap();
    let queue: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(queue["count"], 1, "{queue}");
    assert_eq!(queue["items"][0]["agent"], "worker");
    assert_eq!(queue["items"][0]["rule"], "human:credentials");

    // The lead answering the worker takes the item off the human queue.
    call(
        &handle,
        "message_send",
        json!({ "agent": "boss", "to": "worker", "text": "the key is in the keychain" }),
    )
    .await;
    assert!(human_queue_of(handle.state()).is_empty());
    let outcome = escalations(&handle)[0].outcome.clone().unwrap();
    assert_eq!(outcome["via"], "message");
    assert_eq!(outcome["answered_by"], "boss");
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn with_no_lead_an_escalation_goes_to_the_human_queue() {
    let (_dir, handle, task) = swarm(false).await;
    block(&handle, &task, "which order?").await;
    let rows = escalations(&handle);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].rule.as_deref(), Some("no_lead"));
    assert_eq!(details(&rows[0])["route"], "human");
    assert_eq!(details(&rows[0])["delivered"], json!(["human_queue"]));
    let queue = human_queue_of(handle.state());
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0].blocked_agents.len(), 1);
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_message_to_the_lead_escalates_only_when_it_needs_the_human() {
    let (_dir, handle, _task) = swarm(true).await;
    for (to, text) in [
        ("someone", "need the API key"),
        ("boss", "take t3 or t2?"),
        ("*", "can someone grant repo:admin?"),
    ] {
        let out = call(
            &handle,
            "message_send",
            json!({ "agent": "worker", "to": to, "text": text }),
        )
        .await;
        assert_eq!(out["status"], "ok", "{out}");
    }
    assert!(escalations(&handle).is_empty());

    call(
        &handle,
        "message_send",
        json!({ "agent": "worker", "to": "boss", "text": "can you grant my token repo:admin?" }),
    )
    .await;
    let rows = escalations(&handle);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(details(&rows[0])["trigger"], "message_to_lead");
    assert_eq!(rows[0].rule.as_deref(), Some("human:permissions"));
    assert_eq!(details(&rows[0])["route"], "human");

    // The lead has both messages from the worker and one from `tirith`.
    let lead = call(&handle, "status", json!({ "agent": "boss" })).await;
    let inbox = lead["inbox"].as_array().unwrap();
    let from = |who: &str| inbox.iter().filter(|m| m["from"] == who).count();
    assert_eq!((from("worker"), from("tirith")), (3, 1), "{inbox:?}");
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn the_third_refusal_of_the_same_claim_escalates_once_and_a_grant_answers_it() {
    let (_dir, handle, _task) = swarm(true).await;
    let held = call(
        &handle,
        "claim",
        json!({ "agent": "owner", "paths": ["src/server.rs"], "reason": "persister rework", "brief": false }),
    )
    .await;
    assert_eq!(held["status"], "ok", "{held}");
    for attempt in 0..4 {
        let out = call(
            &handle,
            "claim",
            json!({ "agent": "worker", "paths": ["src/server.rs"], "reason": "routing fix", "brief": false }),
        )
        .await;
        assert_eq!(out["status"], "conflict", "attempt {attempt}: {out}");
    }
    let rows = escalations(&handle);
    assert_eq!(rows.len(), 1, "{rows:?}");
    let details = details(&rows[0]);
    assert_eq!(details["trigger"], "claims_refused");
    assert_eq!(details["paths"], json!(["src/server.rs"]));
    assert_eq!(details["route"], "lead");
    let text = details["text"].as_str().unwrap();
    assert!(text.contains("refused 3 times"), "{text}");
    assert!(
        text.ends_with("reason: routing fix; held by owner"),
        "{text}"
    );

    call(&handle, "release", json!({ "agent": "owner" })).await;
    let out = call(
        &handle,
        "claim",
        json!({ "agent": "worker", "paths": ["src/server.rs"], "reason": "finally", "brief": false }),
    )
    .await;
    assert_eq!(out["status"], "ok", "{out}");
    let outcome = escalations(&handle)[0].outcome.clone().unwrap();
    assert_eq!(outcome["via"], "claim_granted", "{outcome}");
    handle.shutdown().await.unwrap();
}
