//! Lead-only writes (ADR-0036) through a real daemon.
//!
//! While a swarm has a lead, `task_create`, `contract_publish`,
//! `decision_record`, `memory_delete` and `task_update` with `force` are
//! the lead's: anyone else gets `conflict` naming the lead. With no lead,
//! or once the lead releases, the same calls are open to every agent, so
//! solo sessions and humans at the CLI are never refused.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;
#[path = "common/lead.rs"]
mod lead;

use serde_json::{Value, json};
use tirith::server::{ServerHandle, start};

use common::{call, options};
use lead::claim_lead;

/// A daemon where `boss` holds the lead lease when `lead` is set, with one
/// note written by `worker` and one task in progress under `other`, both
/// from before any lead existed.
async fn swarm(lead: bool) -> (tempfile::TempDir, ServerHandle, String) {
    let dir = tempfile::tempdir().unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();
    let task = call(
        &handle,
        "task_create",
        json!({ "agent": "worker", "title": "matcher", "paths": ["src/matcher.rs"] }),
    )
    .await;
    assert_eq!(task["status"], "ok", "{task}");
    let id = task["task"]["id"].as_str().unwrap().to_owned();
    let out = call(
        &handle,
        "task_update",
        json!({ "agent": "other", "task_id": id, "status": "in_progress" }),
    )
    .await;
    assert_eq!(out["status"], "ok", "{out}");
    let note = call(
        &handle,
        "memory_write",
        json!({ "agent": "worker", "title": "Matcher trap", "body": "regexes are greedy" }),
    )
    .await;
    assert_eq!(note["status"], "ok", "{note}");
    if lead {
        claim_lead(&handle, "boss").await;
    }
    (dir, handle, id)
}

/// Every gated call as `agent`, with the task of `other` it may force.
async fn gated(handle: &ServerHandle, agent: &str, task: &str) -> Vec<(&'static str, Value)> {
    let mut outs = Vec::new();
    for (tool, args) in [
        ("task_create", json!({ "title": "follow-up" })),
        (
            "contract_publish",
            json!({ "name": "match", "kind": "function", "shape": { "returns": "bool" } }),
        ),
        (
            "decision_record",
            json!({ "title": "Greedy by default", "decision": "regexes stay greedy" }),
        ),
        (
            "task_update",
            json!({ "task_id": task, "status": "in_progress", "force": true }),
        ),
        ("memory_delete", json!({ "name": "matcher-trap" })),
    ] {
        let mut args = args;
        args["agent"] = json!(agent);
        outs.push((tool, call(handle, tool, args).await));
    }
    outs
}

#[tokio::test]
async fn a_worker_is_refused_every_lead_only_write_and_told_whom_to_ask() {
    let (_dir, handle, task) = swarm(true).await;
    for (tool, out) in gated(&handle, "worker", &task).await {
        assert_eq!(out["status"], "conflict", "{tool}: {out}");
        assert_eq!(out["lead"], "boss", "{tool}: {out}");
        let message = out["message"].as_str().unwrap();
        assert!(
            message.contains("only the lead (boss)") && message.contains("message_send"),
            "{tool}: {message}"
        );
    }
    // Nothing was written.
    let tasks = call(&handle, "task_list", json!({ "agent": "worker" })).await;
    assert_eq!(tasks["total"], 1, "{tasks}");
    let contracts = call(&handle, "contract_list", json!({ "agent": "worker" })).await;
    assert_eq!(contracts["total"], 0, "{contracts}");
    let decisions = call(&handle, "decision_list", json!({ "agent": "worker" })).await;
    assert_eq!(decisions["total"], 0, "{decisions}");
    let note = call(
        &handle,
        "memory_read",
        json!({ "agent": "worker", "name": "matcher-trap" }),
    )
    .await;
    assert_eq!(note["status"], "ok", "{note}");
}

#[tokio::test]
async fn the_lead_itself_is_never_refused() {
    let (_dir, handle, task) = swarm(true).await;
    for (tool, out) in gated(&handle, "boss", &task).await {
        assert_eq!(out["status"], "ok", "{tool}: {out}");
    }
}

#[tokio::test]
async fn with_no_lead_every_agent_may_write() {
    let (_dir, handle, task) = swarm(false).await;
    for (tool, out) in gated(&handle, "worker", &task).await {
        assert_eq!(out["status"], "ok", "{tool}: {out}");
    }
}

#[tokio::test]
async fn the_gates_lift_when_the_lead_releases() {
    let (_dir, handle, task) = swarm(true).await;
    let out = call(
        &handle,
        "task_create",
        json!({ "agent": "worker", "title": "follow-up" }),
    )
    .await;
    assert_eq!(out["status"], "conflict", "{out}");
    let out = call(&handle, "release", json!({ "agent": "boss" })).await;
    assert_eq!(out["status"], "ok", "{out}");
    for (tool, out) in gated(&handle, "worker", &task).await {
        assert_eq!(out["status"], "ok", "{tool}: {out}");
    }
}

#[tokio::test]
async fn a_worker_still_runs_its_own_task_and_leaves_notes() {
    let (_dir, handle, _) = swarm(true).await;
    // Taking a free task, finishing it, publishing a notice and writing a
    // note are a worker's ordinary loop and stay open.
    let task = call(
        &handle,
        "task_create",
        json!({ "agent": "boss", "title": "tests", "paths": ["tests/matcher.rs"] }),
    )
    .await;
    let task = task["task"]["id"].as_str().unwrap().to_owned();
    let out = call(
        &handle,
        "task_update",
        json!({ "agent": "worker", "task_id": task, "status": "in_progress" }),
    )
    .await;
    assert_eq!(out["status"], "ok", "{out}");
    // `force` on its own task is not a reassignment, so it is never gated.
    let out = call(
        &handle,
        "task_update",
        json!({ "agent": "worker", "task_id": task, "status": "in_progress", "force": true }),
    )
    .await;
    assert_eq!(out["status"], "ok", "{out}");
    let out = call(
        &handle,
        "notice_publish",
        json!({ "agent": "worker", "kind": "rename", "summary": "m to matcher", "affected_paths": ["src/lib.rs"] }),
    )
    .await;
    assert_eq!(out["status"], "ok", "{out}");
    let out = call(
        &handle,
        "memory_write",
        json!({ "agent": "worker", "title": "Matcher trap", "body": "regexes are greedy; anchor them" }),
    )
    .await;
    assert_eq!(out["status"], "ok", "{out}");
    let out = call(
        &handle,
        "task_update",
        json!({ "agent": "worker", "task_id": task, "status": "done" }),
    )
    .await;
    assert_eq!(out["status"], "ok", "{out}");
    // A forced take by the lead of a task a worker owns is still fine.
    let out = call(
        &handle,
        "task_update",
        json!({ "agent": "boss", "task_id": task, "status": "todo", "force": true }),
    )
    .await;
    assert_eq!(out["status"], "ok", "{out}");
}
