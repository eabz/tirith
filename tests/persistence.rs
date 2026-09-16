//! Persistence under load: many agents at once, reads that do not write,
//! and everything surviving a restart. The unit tests in `src/store.rs`
//! cover the delta mechanics; these drive the real daemon.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;

use chrono::{Duration, TimeZone, Utc};
use common::{call, options};
use serde_json::json;
use tirith::client::call_tool;
use tirith::clock::ManualClock;
use tirith::server::start;
use tirith::store::JsonStore;

fn modified(path: &Path) -> SystemTime {
    fs::metadata(path).unwrap().modified().unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fifty_agents_at_once_survive_a_restart_and_reads_do_not_write() {
    let dir = tempfile::tempdir().unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();
    let url = handle.mcp_url();
    // Files are only created once their primitive changes, so make sure
    // the task board exists before checking that reads leave it alone.
    call(
        &handle,
        "task_create",
        json!({ "agent": "lead", "title": "t" }),
    )
    .await;

    let mut agents = Vec::new();
    for i in 0..50 {
        let url = url.clone();
        agents.push(tokio::spawn(async move {
            let agent = format!("a{i}");
            let claim = call_tool(
                &url,
                "claim",
                json!({ "agent": agent, "paths": [format!("src/m{i}")], "reason": "r" }),
            )
            .await
            .unwrap();
            assert_eq!(claim["status"], "ok", "{claim}");
            let notice = call_tool(
                &url,
                "notice_publish",
                json!({ "agent": agent, "kind": "rename", "summary": format!("n{i}"), "affected_paths": [format!("src/m{i}")] }),
            )
            .await
            .unwrap();
            assert_eq!(notice["status"], "ok", "{notice}");
        }));
    }
    for agent in agents {
        agent.await.unwrap();
    }

    let status = call(&handle, "status", json!({})).await;
    assert_eq!(status["claims"], 50, "{status}");
    assert_eq!(status["notices"], 50);
    let tirith = dir.path().join(".tirith");
    let notices = fs::read_to_string(tirith.join("notices.jsonl")).unwrap();
    assert_eq!(notices.lines().count(), 50, "one appended line per notice");

    // Reads by an agent that holds claims renew its leases but never
    // rewrite the logs or the task board.
    let notices_before = modified(&tirith.join("notices.jsonl"));
    let tasks_before = modified(&tirith.join("runtime/tasks.json"));
    for _ in 0..3 {
        let listed = call(
            &handle,
            "claims_list",
            json!({ "agent": "a1", "all": true, "limit": 100 }),
        )
        .await;
        assert_eq!(listed["count"], 50);
        call(
            &handle,
            "notice_list",
            json!({ "agent": "a1", "unread": true }),
        )
        .await;
    }
    assert_eq!(modified(&tirith.join("notices.jsonl")), notices_before);
    assert_eq!(modified(&tirith.join("runtime/tasks.json")), tasks_before);

    handle.shutdown().await.unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();
    let status = call(&handle, "status", json!({})).await;
    assert_eq!(status["claims"], 50, "{status}");
    assert_eq!(status["notices"], 50);
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn lease_renewals_reach_disk_by_shutdown() {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(ManualClock::new(
        Utc.with_ymd_and_hms(2026, 9, 16, 12, 0, 0).unwrap(),
    ));
    let handle = start(options(dir.path(), Some(clock.clone())))
        .await
        .unwrap();
    call(
        &handle,
        "claim",
        json!({ "agent": "alice", "paths": ["src/a"], "reason": "r", "ttl_secs": 600 }),
    )
    .await;
    let store = JsonStore::new(dir.path());
    let first = store.load().unwrap().claims[0].expires_at;

    clock.advance(Duration::seconds(120));
    call(&handle, "claims_list", json!({ "agent": "alice" })).await;
    handle.shutdown().await.unwrap();

    let renewed = store.load().unwrap().claims[0].expires_at;
    assert_eq!(renewed, first + Duration::seconds(120));
}

/// ADR-0021: delivering a notice appends to a runtime log and leaves the
/// committed notice log byte-identical; the seen mark survives a restart.
#[tokio::test]
async fn a_delivery_never_rewrites_the_notice_log_and_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();
    let published = call(
        &handle,
        "notice_publish",
        json!({ "agent": "alice", "kind": "rename", "summary": "x to y", "affected_paths": ["src/x.rs"] }),
    )
    .await;
    let id = published["notice"]["id"].as_str().unwrap().to_owned();
    handle.shutdown().await.unwrap();
    let notices_path = dir.path().join(".tirith/notices.jsonl");
    let before = fs::read(&notices_path).unwrap();

    let handle = start(options(dir.path(), None)).await.unwrap();
    let unread = call(
        &handle,
        "notice_list",
        json!({ "agent": "bob", "path": "src/x.rs", "unread": true }),
    )
    .await;
    assert_eq!(unread["count"], 1, "{unread}");
    assert_eq!(unread["notices"][0]["id"], id[..8], "{unread}");
    // Listing it delivered it; delivery is the acknowledgement (ADR-0021).
    handle.shutdown().await.unwrap();
    assert_eq!(
        fs::read(&notices_path).unwrap(),
        before,
        "notices.jsonl is byte-identical"
    );
    let seen = fs::read_to_string(dir.path().join(".tirith/runtime/notice_seen.jsonl")).unwrap();
    assert_eq!(seen.lines().count(), 1, "{seen}");
    assert!(seen.contains("\"bob\""));

    let handle = start(options(dir.path(), None)).await.unwrap();
    let after = call(
        &handle,
        "notice_list",
        json!({ "agent": "bob", "path": "src/x.rs", "unread": true }),
    )
    .await;
    assert_eq!(
        after["count"], 0,
        "the seen mark was replayed on load: {after}"
    );
    let carol = call(
        &handle,
        "notice_list",
        json!({ "agent": "carol", "path": "src/x.rs", "unread": true }),
    )
    .await;
    assert_eq!(carol["count"], 1, "only bob was shown it: {carol}");
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_bad_line_does_not_stop_the_daemon_and_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let tirith = dir.path().join(".tirith");
    fs::create_dir_all(&tirith).unwrap();
    fs::write(tirith.join("notices.jsonl"), "<<<<<<< HEAD\n").unwrap();
    fs::create_dir_all(tirith.join("contracts")).unwrap();
    fs::write(tirith.join("contracts/broken-00000000.json"), "{ nope").unwrap();

    let handle = start(options(dir.path(), None)).await.unwrap();
    let errors = handle.state().load_errors();
    assert_eq!(errors.len(), 2, "{errors:?}");
    let health: serde_json::Value = reqwest::get(format!("{}api/health", handle.dashboard_url()))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(health["ok"], true, "the daemon is up");
    let paths: Vec<&str> = health["load_errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["path"].as_str().unwrap())
        .collect();
    assert!(paths.contains(&"notices.jsonl"), "{health}");
    assert!(
        paths.contains(&"contracts/broken-00000000.json"),
        "{health}"
    );
    assert_eq!(
        health["load_errors"][0]["error"]
            .as_str()
            .map(str::is_empty),
        Some(false)
    );

    // The daemon keeps working and a later append leaves the bad line in place.
    call(
        &handle,
        "notice_publish",
        json!({ "agent": "alice", "kind": "rename", "summary": "still works", "affected_paths": ["src"] }),
    )
    .await;
    handle.shutdown().await.unwrap();
    let text = fs::read_to_string(tirith.join("notices.jsonl")).unwrap();
    assert!(text.starts_with("<<<<<<< HEAD\n"), "{text}");
    assert_eq!(text.lines().count(), 2);
}

#[cfg(unix)]
#[tokio::test]
async fn a_failed_write_is_reported_on_the_mutating_response() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();
    let first = call(
        &handle,
        "claim",
        json!({ "agent": "alice", "paths": ["src/a"], "reason": "r" }),
    )
    .await;
    assert_eq!(first["status"], "ok");
    assert!(first["persist_error"].is_null());

    let runtime = dir.path().join(".tirith/runtime");
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o555)).unwrap();
    let second = call(
        &handle,
        "claim",
        json!({ "agent": "bob", "paths": ["src/b"], "reason": "r" }),
    )
    .await;
    assert_eq!(second["status"], "ok", "the in-memory decision stands");
    assert!(second["persist_error"].is_string(), "{second}");
    let status = call(&handle, "status", json!({})).await;
    assert!(status["persist_error"].is_string());

    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o755)).unwrap();
    let third = call(
        &handle,
        "claim",
        json!({ "agent": "carol", "paths": ["src/c"], "reason": "r" }),
    )
    .await;
    assert!(third["persist_error"].is_null(), "recovered: {third}");
    handle.shutdown().await.unwrap();
    let store = JsonStore::new(dir.path());
    assert_eq!(
        store.load().unwrap().claims.len(),
        3,
        "the full rewrite caught bob up"
    );
}

#[tokio::test]
async fn a_legacy_decisions_jsonl_is_imported_into_one_file_per_decision() {
    let dir = tempfile::tempdir().unwrap();
    let tirith = dir.path().join(".tirith");
    fs::create_dir_all(&tirith).unwrap();
    let rows: Vec<String> = (1..=3)
        .map(|i| {
            json!({
                "id": uuid_like(i),
                "title": format!("Old decision {i}"),
                "decision": format!("Choice {i}"),
                "rationale": if i == 1 { "because" } else { "" },
                "alternatives": if i == 2 { vec!["other"] } else { vec![] },
                "affects_paths": ["src"],
                "recorded_by": "legacy",
                "recorded_at": "2026-09-15T10:00:00Z"
            })
            .to_string()
        })
        .collect();
    fs::write(tirith.join("decisions.jsonl"), rows.join("\n") + "\n").unwrap();

    let handle = start(options(dir.path(), None)).await.unwrap();
    let listed = call(&handle, "decision_list", json!({ "agent": "reader" })).await;
    assert_eq!(listed["total"], 3, "{listed}");
    let files: Vec<_> = fs::read_dir(tirith.join("decisions"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(files.len(), 3, "{files:?}");
    assert!(
        files
            .iter()
            .all(|f| Path::new(f).extension().is_some_and(|e| e == "md")),
        "{files:?}"
    );
    assert!(
        !tirith.join("decisions.jsonl").exists(),
        "the JSON Lines log is removed once every file is written"
    );
    handle.shutdown().await.unwrap();

    // A restart reads the files, not the removed log, and a hand-broken
    // file is reported rather than fatal.
    fs::write(tirith.join("decisions").join("broken.md"), "not: [valid\n").unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();
    let listed = call(&handle, "decision_list", json!({ "agent": "reader" })).await;
    assert_eq!(listed["total"], 3, "{listed}");
    let status = call(&handle, "status", json!({ "agent": "reader" })).await;
    assert_eq!(
        status["load_errors"].as_array().map(Vec::len),
        Some(1),
        "{status}"
    );
    handle.shutdown().await.unwrap();
}

fn uuid_like(i: u32) -> String {
    format!("00000000-0000-4000-8000-{i:012}")
}
