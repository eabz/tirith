//! Escalation routing by the lead policy (ADR-0027 section 3) through a
//! real daemon.
//!
//! Routing is deterministic: while there is a lead, escalations go to its
//! inbox, tagged when a human rule matches, and only a message to `human`
//! reaches the human queue; with no lead, escalations go to the human
//! queue. The tests check the triggers, every route's delivery, the human
//! queue, answering it, and the outcome fields, with the texts that
//! wrongly reached the human under the first rules as regression cases.

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

/// `GET /api/human`.
async fn human(handle: &ServerHandle) -> Value {
    let url = format!("{}api/human", handle.dashboard_url());
    let body = reqwest::get(&url).await.unwrap().text().await.unwrap();
    serde_json::from_str(&body).unwrap()
}

/// `POST /api/human/{id}/done` with `body`, as the CLI sends it.
async fn done(handle: &ServerHandle, id: u64, body: Value) -> Value {
    let url = format!("{}api/human/{id}/done", handle.dashboard_url());
    let text = reqwest::Client::new()
        .post(&url)
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    serde_json::from_str(&text).unwrap()
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
    assert_eq!(row.rule.as_deref(), Some("live_lead"), "{row:?}");
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
async fn with_a_lead_a_block_about_credentials_reaches_the_lead_tagged_not_the_human() {
    let (_dir, handle, task) = swarm(true).await;
    block(&handle, &task, "need a working API key in .env").await;
    let rows = escalations(&handle);
    let row = &rows[0];
    assert_eq!(row.rule.as_deref(), Some("live_lead"));
    assert_eq!(details(row)["route"], "lead");
    assert_eq!(details(row)["tag"], "credentials");
    assert_eq!(details(row)["delivered"], json!(["lead"]));
    let lead = from_tirith(&handle, "boss").await;
    assert_eq!(lead.len(), 1, "{lead:?}");
    assert!(
        lead[0].contains("(task_blocked; may need the human: credentials): need a working API key"),
        "{lead:?}"
    );
    let queue = human(&handle).await;
    assert_eq!(queue["count"], 0, "{queue}");
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn with_no_lead_an_escalation_goes_to_the_human_queue_and_an_answer_clears_it() {
    let (_dir, handle, task) = swarm(false).await;
    block(&handle, &task, "need a working API key in .env").await;
    let rows = escalations(&handle);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].rule.as_deref(), Some("no_lead"));
    assert_eq!(details(&rows[0])["route"], "human");
    assert_eq!(details(&rows[0])["tag"], "credentials");
    assert_eq!(details(&rows[0])["delivered"], json!(["human_queue"]));
    let queue = human(&handle).await;
    assert_eq!(queue["count"], 1, "{queue}");
    assert_eq!(queue["items"][0]["agent"], "worker");
    assert_eq!(queue["items"][0]["text"], "need a working API key in .env");
    assert_eq!(queue["items"][0]["blocked_agents"], json!(["worker"]));

    // Anyone answering the worker takes the item off the human queue.
    call(
        &handle,
        "message_send",
        json!({ "agent": "helper", "to": "worker", "text": "the key is in the keychain" }),
    )
    .await;
    assert!(human_queue_of(handle.state()).is_empty());
    let outcome = escalations(&handle)[0].outcome.clone().unwrap();
    assert_eq!(outcome["via"], "message");
    assert_eq!(outcome["answered_by"], "helper");
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_message_to_the_lead_is_never_an_escalation() {
    let (_dir, handle, _task) = swarm(true).await;
    // The done report that reached the human under the first rules:
    // "grant/assign nothing" matched the permissions phrase "grant".
    let report = "Done: task 82f613bc. claim/task_pull with wait_secs now stop waiting \
                  (grant/assign nothing, status \"cancelled\") when the caller cancels. \
                  check.sh green; released all claims.";
    for (to, text) in [
        ("boss", report),
        ("boss", "can you grant my token repo:admin?"),
        ("someone", "need the API key"),
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
    assert_eq!(human(&handle).await["count"], 0);
    let lead = call(&handle, "status", json!({ "agent": "boss" })).await;
    let inbox = lead["inbox"].as_array().unwrap();
    let from = |who: &str| inbox.iter().filter(|m| m["from"] == who).count();
    assert_eq!((from("worker"), from("tirith")), (3, 0), "{inbox:?}");
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn the_lead_relays_to_human_and_done_with_a_reply_answers_the_lead() {
    let (_dir, handle, task) = swarm(true).await;
    // The real need from the same swarm: the worker tells the lead, and the
    // lead writes to the human.
    block(
        &handle,
        &task,
        "headless claude CLI is not logged in: `claude -p` says Please run /login",
    )
    .await;
    assert_eq!(from_tirith(&handle, "boss").await.len(), 1);
    assert_eq!(human(&handle).await["count"], 0);

    let text = "The headless claude CLI on this machine is not logged in.\n\
                Run `claude login` once so the workers can start.";
    let sent = call(
        &handle,
        "message_send",
        json!({ "agent": "boss", "to": "Human", "text": text }),
    )
    .await;
    assert_eq!(sent["status"], "ok", "{sent}");
    assert_eq!(sent["message"]["to"], "human");
    let message_id = sent["message"]["id"].as_str().unwrap().to_owned();
    let queue = human(&handle).await;
    assert_eq!(queue["count"], 1, "{queue}");
    let item = &queue["items"][0];
    assert_eq!(item["agent"], "boss");
    assert_eq!(item["text"], text);
    assert_eq!(item["rule"], "to_human");
    assert_eq!(item["trigger"], "message_to_human");
    // Tool results shorten ids; the queue carries the full one.
    assert!(
        item["message"].as_str().unwrap().starts_with(&message_id),
        "{item}"
    );
    let id = item["id"].as_u64().unwrap();

    // `human` is not an agent: nobody calls as it, no broadcast reaches it,
    // and talking to the lead does not answer the item.
    let refused = call(&handle, "status", json!({ "agent": "human" })).await;
    assert_eq!(refused["status"], "invalid", "{refused}");
    let broadcast = call(
        &handle,
        "message_send",
        json!({ "agent": "boss", "to": "*", "text": "waiting on the human" }),
    )
    .await;
    assert_eq!(
        broadcast["message"]["audience"],
        json!(["worker"]),
        "{broadcast}"
    );
    call(
        &handle,
        "message_send",
        json!({ "agent": "worker", "to": "boss", "text": "ok, waiting" }),
    )
    .await;
    assert_eq!(human(&handle).await["count"], 1);

    let answered = done(&handle, id, json!({ "reply": "Logged in, go ahead." })).await;
    assert_eq!(answered["status"], "ok", "{answered}");
    assert_eq!(human(&handle).await["count"], 0);
    let lead = call(&handle, "status", json!({ "agent": "boss" })).await;
    let replies: Vec<&Value> = lead["inbox"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["from"] == "human")
        .collect();
    assert_eq!(replies.len(), 1, "{lead}");
    assert_eq!(replies[0]["text"], "Logged in, go ahead.");
    // The conversation with the human answers the message that queued it.
    let history = call(
        &handle,
        "message_list",
        json!({ "agent": "boss", "with": "human" }),
    )
    .await;
    let rows = history["messages"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "{history}");
    assert_eq!(rows[0]["from"], "human");
    assert_eq!(rows[0]["reply_to"], message_id.as_str(), "{history}");
    let outcome = escalations(&handle)[0].outcome.clone().unwrap();
    assert_eq!(outcome["answered_by"], "human");
    assert_eq!(outcome["via"], "reply");

    let again = done(&handle, id, json!({})).await;
    assert_eq!(again["status"], "not_found", "{again}");
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
