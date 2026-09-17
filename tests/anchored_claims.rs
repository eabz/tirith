//! Symbol-anchored claims (`path#Symbol`, ADR-0029, experimental) through a
//! real daemon: conflicts, `wait_secs`, claim-aware `task_pull`, briefs,
//! and the known starvation of a whole-file waiter behind anchors.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::time::Duration;

use common::{call, options};
use serde_json::{Value, json};
use tirith::client::call_tool;
use tirith::server::start;

fn claim_args(agent: &str, paths: &[&str]) -> Value {
    json!({ "agent": agent, "paths": paths, "reason": "r", "brief": false })
}

#[tokio::test]
async fn anchors_conflict_with_their_file_and_enclosing_symbol_only() {
    let dir = tempfile::tempdir().unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();

    let alice = call(
        &handle,
        "claim",
        claim_args("alice", &["app/config.py#Config.from_env"]),
    )
    .await;
    assert_eq!(alice["status"], "ok", "{alice}");
    assert_eq!(
        alice["new_paths"],
        json!(["app/config.py#Config::from_env"]),
        "responses echo the stored form"
    );

    let bob = call(
        &handle,
        "claim",
        claim_args("bob", &["app/config.py#Config::gzip_min_size"]),
    )
    .await;
    assert_eq!(
        bob["status"], "ok",
        "sibling anchors do not conflict: {bob}"
    );

    for target in ["app/config.py", "app/config.py#config", "app"] {
        let carol = call(&handle, "claim", claim_args("carol", &[target])).await;
        assert_eq!(carol["status"], "conflict", "{target}: {carol}");
    }
    let carol = call(&handle, "claim", claim_args("carol", &["app/config.py"])).await;
    let owners: Vec<&str> = carol["conflicts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["owner"].as_str().unwrap())
        .collect();
    assert!(
        owners.contains(&"alice") && owners.contains(&"bob"),
        "{carol}"
    );

    // A directory written with a trailing slash cannot carry an anchor.
    let bad = call(&handle, "claim", claim_args("carol", &["app/#Config"])).await;
    assert_ne!(bad["status"], "ok", "{bad}");

    // claims_list with an anchored path uses the same rule.
    let listed = call(
        &handle,
        "claims_list",
        json!({ "agent": "dave", "path": "app/config.py#Config::from_env" }),
    )
    .await;
    assert_eq!(listed["count"], 1, "{listed}");
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_waiting_claim_wakes_when_the_overlapping_anchor_is_released() {
    let dir = tempfile::tempdir().unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();
    call(
        &handle,
        "claim",
        claim_args("alice", &["src/pricing.py#shipping_cost"]),
    )
    .await;

    let started = std::time::Instant::now();
    let release_later = async {
        tokio::time::sleep(Duration::from_secs(1)).await;
        call(
            &handle,
            "release",
            json!({ "agent": "alice", "paths": ["src/pricing.py#shipping_cost"] }),
        )
        .await
    };
    let mut args = claim_args("bob", &["src/pricing.py"]);
    args["wait_secs"] = json!(30);
    let wait = call(&handle, "claim", args);
    let (released, bob) = tokio::join!(release_later, wait);
    assert_eq!(released["status"], "ok", "{released}");
    assert_eq!(bob["status"], "ok", "{bob}");
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_millis(900) && waited < Duration::from_secs(5),
        "waited {waited:?}"
    );
    handle.shutdown().await.unwrap();
}

/// ADR-0029 names this risk: retries are not queued, so a whole-file claim
/// waits as long as anchor claims in that file keep arriving. This test
/// pins today's behavior; FIFO waiting would change it.
#[tokio::test]
async fn a_whole_file_waiter_is_not_queued_ahead_of_new_anchor_claims() {
    let dir = tempfile::tempdir().unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();
    let url = handle.mcp_url();
    call(
        &handle,
        "claim",
        claim_args("alice", &["src/money.py#round_money"]),
    )
    .await;

    let mut args = claim_args("bob", &["src/money.py"]);
    args["wait_secs"] = json!(30);
    let bob_url = url.clone();
    let bob = tokio::spawn(async move { call_tool(&bob_url, "claim", args).await.unwrap() });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!bob.is_finished());

    // A new anchor claim in the same file is granted while bob waits.
    let carol = call(
        &handle,
        "claim",
        claim_args("carol", &["src/money.py#parse_money"]),
    )
    .await;
    assert_eq!(carol["status"], "ok", "{carol}");

    // alice releases, but carol's anchor still overlaps bob's file claim.
    call(&handle, "release", json!({ "agent": "alice" })).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!bob.is_finished(), "bob still waits behind carol's anchor");

    call(&handle, "release", json!({ "agent": "carol" })).await;
    let bob = tokio::time::timeout(Duration::from_secs(5), bob)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(bob["status"], "ok", "{bob}");
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn task_pull_treats_an_anchor_as_holding_its_whole_file() {
    let dir = tempfile::tempdir().unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();
    let held = call(
        &handle,
        "task_create",
        json!({ "agent": "lead", "title": "whole file", "priority": 9, "paths": ["src/pricing.py"] }),
    )
    .await;
    let free = call(
        &handle,
        "task_create",
        json!({ "agent": "lead", "title": "other file", "priority": 1, "paths": ["src/invoice.py"] }),
    )
    .await;
    call(
        &handle,
        "claim",
        claim_args("alice", &["src/pricing.py#tax_for"]),
    )
    .await;

    let bob = call(&handle, "task_pull", json!({ "agent": "bob" })).await;
    assert_eq!(bob["task"]["id"], free["task"]["id"], "{bob}");
    assert!(bob.get("waiting_on").is_none(), "{bob}");

    let carol = call(&handle, "task_pull", json!({ "agent": "carol" })).await;
    assert_eq!(carol["task"]["id"], held["task"]["id"], "{carol}");
    assert_eq!(carol["waiting_on"][0]["path"], "src/pricing.py#tax_for");
    assert_eq!(carol["waiting_on"][0]["owner"], "alice");
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn briefs_match_anchored_claims_on_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();
    for affected in ["src/pricing.py", "src/pricing.py#apply_discount", "src"] {
        call(
            &handle,
            "notice_publish",
            json!({ "agent": "other", "kind": "behavior", "summary": affected,
                "affected_paths": [affected] }),
        )
        .await;
    }
    call(
        &handle,
        "notice_publish",
        json!({ "agent": "other", "kind": "behavior", "summary": "elsewhere",
            "affected_paths": ["src/invoice.py#render_invoice"] }),
    )
    .await;
    let me = call(
        &handle,
        "claim",
        json!({ "agent": "me", "paths": ["src/pricing.py#shipping_cost"], "reason": "r" }),
    )
    .await;
    assert_eq!(me["status"], "ok", "{me}");
    let mut summaries: Vec<&str> = me["notices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["summary"].as_str().unwrap())
        .collect();
    summaries.sort_unstable();
    assert_eq!(
        summaries,
        ["src", "src/pricing.py", "src/pricing.py#apply_discount"]
    );
    handle.shutdown().await.unwrap();
}
