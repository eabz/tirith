//! End-to-end tests: a real daemon on an ephemeral localhost port, driven
//! through the MCP client the CLI uses.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;
#[path = "common/raw_client.rs"]
mod raw_client;

use std::sync::Arc;

use chrono::{Duration, TimeZone, Utc};
use common::{call, options};
use raw_client::raw_client;
use rmcp::model::{CallToolRequest, CallToolRequestParams, ClientRequest};
use rmcp::service::PeerRequestOptions;
use serde_json::{Value, json};
use tirith::client::list_tools;
use tirith::clock::ManualClock;
use tirith::server::{ServeOptions, start};

#[tokio::test]
async fn second_agent_is_refused_and_succeeds_after_release() {
    let dir = tempfile::tempdir().unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();

    let alice = call(
        &handle,
        "claim",
        json!({ "agent": "alice", "paths": ["src/auth/"], "reason": "refactor session handling" }),
    )
    .await;
    assert_eq!(alice["status"], "ok");
    assert_eq!(alice["new_paths"], json!(["src/auth"]));

    let bob = call(
        &handle,
        "claim",
        json!({ "agent": "bob", "paths": ["src/auth/login.rs"], "reason": "fix login redirect" }),
    )
    .await;
    assert_eq!(bob["status"], "conflict");
    let conflict = &bob["conflicts"][0];
    assert_eq!(conflict["owner"], "alice");
    assert_eq!(conflict["overlaps"], "src/auth");
    assert_eq!(conflict["reason"], "refactor session handling");

    let listed = call(&handle, "claims_list", json!({ "agent": "bob" })).await;
    assert_eq!(
        listed["count"], 0,
        "bob holds nothing and asked for no path"
    );
    let listed = call(
        &handle,
        "claims_list",
        json!({ "agent": "bob", "path": "src/auth/login.rs" }),
    )
    .await;
    assert_eq!(listed["count"], 1, "claims overlapping the path are shown");
    assert_eq!(listed["claims"][0]["owner"], "alice");
    assert_eq!(
        listed["claims"][0]["id"].as_str().map(str::len),
        Some(8),
        "compact row"
    );
    let listed = call(
        &handle,
        "claims_list",
        json!({ "agent": "bob", "all": true }),
    )
    .await;
    assert_eq!(listed["count"], 1);

    let released = call(&handle, "release", json!({ "agent": "alice" })).await;
    assert_eq!(released["status"], "ok");

    let bob = call(
        &handle,
        "claim",
        json!({ "agent": "bob", "paths": ["src/auth/login.rs"], "reason": "fix login redirect" }),
    )
    .await;
    assert_eq!(bob["status"], "ok");

    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn state_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();
    call(
        &handle,
        "claim",
        json!({ "agent": "alice", "paths": ["src/a"], "reason": "r" }),
    )
    .await;
    let published = call(
        &handle,
        "contract_publish",
        json!({ "agent": "alice", "name": "POST /x", "kind": "http", "shape": {"a": 1}, "consumers": ["src/client"] }),
    )
    .await;
    assert_eq!(published["status"], "ok");
    call(
        &handle,
        "decision_record",
        json!({ "agent": "alice", "title": "t", "decision": "d" }),
    )
    .await;
    let addr = handle.addr();
    handle.shutdown().await.unwrap();
    assert!(!dir.path().join(".tirith/runtime/daemon.json").exists());

    let handle = start(ServeOptions {
        bind: addr,
        ..options(dir.path(), None)
    })
    .await
    .unwrap();
    let status = call(&handle, "status", json!({})).await;
    assert_eq!(status["claims"], 1, "{status}");
    assert_eq!(status["contracts"], 1);
    assert_eq!(status["decisions"], 1);
    let contract = call(
        &handle,
        "contract_get",
        json!({ "agent": "bob", "name": "POST /x" }),
    )
    .await;
    assert_eq!(contract["contract"]["current"]["shape"], json!({"a": 1}));
    assert!(dir.path().join(".tirith/.gitignore").exists());
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn leases_expire_without_activity() {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(ManualClock::new(
        Utc.with_ymd_and_hms(2026, 9, 15, 18, 0, 0).unwrap(),
    ));
    let handle = start(options(dir.path(), Some(clock.clone())))
        .await
        .unwrap();
    call(
        &handle,
        "claim",
        json!({ "agent": "alice", "paths": ["a"], "reason": "r", "ttl_secs": 30 }),
    )
    .await;
    clock.advance(Duration::seconds(31));
    let bob = call(
        &handle,
        "claim",
        json!({ "agent": "bob", "paths": ["a"], "reason": "r" }),
    )
    .await;
    assert_eq!(bob["status"], "ok");
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn task_contract_notice_flow() {
    let dir = tempfile::tempdir().unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();

    let first = call(
        &handle,
        "task_create",
        json!({ "agent": "planner", "title": "build server side", "priority": 1 }),
    )
    .await;
    let first_id = first["task"]["id"].as_str().unwrap().to_owned();
    call(
        &handle,
        "task_create",
        json!({ "agent": "planner", "title": "build client side", "priority": 9, "depends_on": [first_id] }),
    )
    .await;
    let pulled = call(&handle, "task_pull", json!({ "agent": "alice" })).await;
    assert_eq!(pulled["task"]["title"], "build server side");
    assert_eq!(
        call(&handle, "task_pull", json!({ "agent": "bob" })).await["status"],
        "none"
    );
    call(
        &handle,
        "task_update",
        json!({ "agent": "alice", "task_id": first_id, "status": "done" }),
    )
    .await;
    assert_eq!(
        call(&handle, "task_pull", json!({ "agent": "bob" })).await["task"]["title"],
        "build client side"
    );

    call(
        &handle,
        "contract_publish",
        json!({ "agent": "alice", "name": "POST /sessions", "kind": "http", "shape": {"session_id": "string"}, "consumers": ["src/client"] }),
    )
    .await;
    let v2 = call(
        &handle,
        "contract_publish",
        json!({ "agent": "alice", "name": "POST /sessions", "kind": "http", "shape": {"token": "string"}, "consumers": ["src/client"] }),
    )
    .await;
    assert_eq!(v2["previous_version"], 1);
    assert!(v2["notice_id"].is_string());

    let unread = call(
        &handle,
        "notice_list",
        json!({ "agent": "bob", "path": "src/client/sessions.rs", "unread": true }),
    )
    .await;
    assert_eq!(unread["count"], 1);
    // Listing unread notices delivers them, and delivery is the
    // acknowledgement (ADR-0021).
    let unread = call(
        &handle,
        "notice_list",
        json!({ "agent": "bob", "path": "src/client", "unread": true }),
    )
    .await;
    assert_eq!(unread["count"], 0);

    let invalid = call(
        &handle,
        "claim",
        json!({ "agent": "", "paths": ["a"], "reason": "" }),
    )
    .await;
    assert_eq!(invalid["status"], "invalid");
    let missing = call(&handle, "release", json!({ "agent": "nobody" })).await;
    assert_eq!(missing["status"], "not_found");

    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn dashboard_and_tool_list_are_served() {
    let dir = tempfile::tempdir().unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();
    let tools = list_tools(&handle.mcp_url()).await.unwrap();
    let names: Vec<&str> = tools.iter().map(|(n, _)| n.as_str()).collect();
    // The 22 tools docs/1-about/04-primitives.md documents, and no other.
    let documented = [
        "claim",
        "release",
        "renew",
        "claims_list",
        "task_create",
        "task_pull",
        "task_update",
        "task_list",
        "contract_publish",
        "contract_get",
        "contract_list",
        "notice_publish",
        "notice_list",
        "decision_record",
        "decision_list",
        "memory_write",
        "memory_read",
        "memory_search",
        "memory_delete",
        "message_send",
        "message_list",
        "status",
    ];
    for expected in documented {
        assert!(names.contains(&expected), "missing tool {expected}");
    }
    assert_eq!(
        names.len(),
        documented.len(),
        "undocumented tool: {names:?}"
    );
    let state: Value = reqwest_get(&format!("{}api/state", handle.dashboard_url())).await;
    assert_eq!(state["server"]["mcp_url"], handle.mcp_url());
    assert!(state["claims"].is_array());
    let page = reqwest_text(&handle.dashboard_url()).await;
    assert!(page.contains("<title>Tirith</title>"));
    assert!(page.contains("/logo.png"));
    let logo = raw_response(&format!("{}logo.png", handle.dashboard_url())).await;
    let (headers, body) = logo.split_once("\r\n\r\n").unwrap();
    assert!(headers.starts_with("HTTP/1.1 200"));
    assert!(
        headers
            .to_ascii_lowercase()
            .contains("content-type: image/png")
    );
    assert!(body.len() > 1000, "logo body is {} bytes", body.len());
    handle.shutdown().await.unwrap();
}

async fn reqwest_text(url: &str) -> String {
    let response = raw_response(url).await;
    let (_, body) = response.split_once("\r\n\r\n").unwrap();
    body.to_owned()
}

/// The full HTTP/1.1 response, headers included, as lossy UTF-8.
async fn raw_response(url: &str) -> String {
    let stream = tokio::net::TcpStream::connect(url_host(url)).await.unwrap();
    raw_get(stream, url).await
}

async fn reqwest_get(url: &str) -> Value {
    let body = reqwest_text(url).await;
    serde_json::from_str(&body).unwrap()
}

fn url_host(url: &str) -> String {
    url.trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap()
        .to_owned()
}

async fn raw_get(mut stream: tokio::net::TcpStream, url: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let path = url
        .trim_start_matches("http://")
        .split_once('/')
        .map_or("/", |(_, p)| p);
    let host = url_host(url);
    stream
        .write_all(
            format!("GET /{path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").as_bytes(),
        )
        .await
        .unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    String::from_utf8_lossy(&response).into_owned()
}

#[tokio::test]
async fn lists_are_bounded_and_page_without_gaps() {
    let dir = tempfile::tempdir().unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();
    for i in 0..300 {
        call(
            &handle,
            "notice_publish",
            json!({ "agent": "alice", "kind": "rename", "summary": format!("n{i}"), "affected_paths": ["src/x"] }),
        )
        .await;
    }
    let first = call(
        &handle,
        "notice_list",
        json!({ "agent": "bob", "path": "src/x" }),
    )
    .await;
    assert_eq!(first["count"], 20, "{first}");
    assert_eq!(first["total"], 300);
    assert_eq!(first["truncated"], true);
    assert_eq!(first["notices"][0]["summary"], "n299", "newest first");
    let row = first["notices"][0].as_object().unwrap();
    assert!(
        !row.contains_key("acked_by"),
        "compact rows drop acked_by: {row:?}"
    );
    assert!(!row.contains_key("from"), "nulls are omitted: {row:?}");
    assert_eq!(row["id"].as_str().map(str::len), Some(8));
    assert!(
        !row["published_at"].as_str().unwrap().contains('.'),
        "seconds only"
    );

    // Page to the end with the cursor; every notice appears exactly once.
    let mut seen = Vec::new();
    let mut before = first["next_before"].as_str().map(str::to_owned);
    for n in first["notices"].as_array().unwrap() {
        seen.push(n["summary"].as_str().unwrap().to_owned());
    }
    while let Some(cursor) = before.take() {
        let page = call(
            &handle,
            "notice_list",
            json!({ "agent": "bob", "path": "src/x", "limit": 200, "before": cursor }),
        )
        .await;
        for n in page["notices"].as_array().unwrap() {
            seen.push(n["summary"].as_str().unwrap().to_owned());
        }
        before = page["next_before"].as_str().map(str::to_owned);
    }
    assert_eq!(seen.len(), 300, "no gaps, no duplicates");
    let mut unique = seen.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 300);

    let capped = call(
        &handle,
        "notice_list",
        json!({ "agent": "bob", "path": "src/x", "limit": 5000 }),
    )
    .await;
    assert_eq!(capped["count"], 200, "limit is capped");
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn id_prefixes_are_accepted_and_ambiguity_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();
    let created = call(
        &handle,
        "task_create",
        json!({ "agent": "alice", "title": "prefix me" }),
    )
    .await;
    let full = created["task"]["id"].as_str().unwrap().to_owned();
    let updated = call(
        &handle,
        "task_update",
        json!({ "agent": "bob", "task_id": &full[..8], "status": "in_progress" }),
    )
    .await;
    assert_eq!(updated["status"], "ok", "{updated}");
    let missing = call(
        &handle,
        "task_update",
        json!({ "agent": "bob", "task_id": "zzzz", "status": "done" }),
    )
    .await;
    assert_eq!(missing["status"], "not_found");
    // Every id starts with the empty string, so an empty prefix is refused too.
    let empty = call(
        &handle,
        "task_update",
        json!({ "agent": "bob", "task_id": "", "status": "done" }),
    )
    .await;
    assert_ne!(empty["status"], "ok");

    // Two tasks whose ids share a prefix long enough to be ambiguous is
    // astronomically unlikely, so exercise ambiguity through the domain.
    let ambiguous = handle.state().resolve_task("");
    assert!(ambiguous.is_err());
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn status_stays_small_without_verbose_and_unread_defaults_to_held_paths() {
    let dir = tempfile::tempdir().unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();
    // Published before anyone holds the paths, and claimed without a
    // brief, so neither a push nor a brief delivers them before the unread
    // listings below.
    call(
        &handle,
        "notice_publish",
        json!({ "agent": "a2", "kind": "rename", "summary": "about m1", "affected_paths": ["src/m1/f.rs"] }),
    )
    .await;
    call(
        &handle,
        "notice_publish",
        json!({ "agent": "a2", "kind": "rename", "summary": "about m7", "affected_paths": ["src/m7"] }),
    )
    .await;
    for i in 0..300 {
        let claimed = call(
            &handle,
            "claim",
            json!({ "agent": format!("a{i}"), "paths": [format!("src/m{i}")], "reason": "r", "brief": false }),
        )
        .await;
        assert_eq!(claimed["status"], "ok");
    }
    let status = call(&handle, "status", json!({})).await;
    assert_eq!(status["claims"], 300);
    assert_eq!(status["agents_active"], 300);
    assert!(status["agents"].is_null(), "no agent rows by default");
    assert!(status["load_errors"].is_array());
    assert!(
        serde_json::to_string(&status).unwrap().len() < 1024,
        "{status}"
    );
    let verbose = call(&handle, "status", json!({ "verbose": true })).await;
    assert_eq!(verbose["agents"].as_array().unwrap().len(), 50);
    assert_eq!(verbose["agents_truncated"], true);
    assert_eq!(verbose["agents"][0]["paths_count"], 1);

    let renewed = call(&handle, "renew", json!({ "agent": "a1" })).await;
    assert_eq!(renewed["count"], 1);
    assert!(renewed["claims"].is_null(), "renew no longer echoes claims");
    assert!(renewed["expires_at"].is_string());

    let mine = call(
        &handle,
        "notice_list",
        json!({ "agent": "a1", "unread": true }),
    )
    .await;
    assert_eq!(mine["count"], 1, "scoped to held paths: {mine}");
    assert_eq!(mine["notices"][0]["summary"], "about m1");
    let all = call(
        &handle,
        "notice_list",
        json!({ "agent": "a1", "unread": true, "all": true }),
    )
    .await;
    assert_eq!(
        all["count"], 1,
        "the scoped listing delivered `about m1`; only `about m7` is still unread: {all}"
    );
    let nobody = call(
        &handle,
        "notice_list",
        json!({ "agent": "outsider", "unread": true }),
    )
    .await;
    assert_eq!(nobody["count"], 0);
    assert!(nobody["message"].is_string(), "{nobody}");
    handle.shutdown().await.unwrap();
}

/// Audit tasks 401ed7c9 and 5ed3595f over the wire: a foreign agent cannot
/// take or close an in-progress task without `force`, a republish keeps the
/// consumers it does not repeat and notifies old and new ones, and a stale
/// `expected_version` is a conflict.
#[tokio::test]
async fn foreign_task_updates_and_stale_contract_versions_are_conflicts() {
    let dir = tempfile::tempdir().unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();
    let created = call(
        &handle,
        "task_create",
        json!({ "agent": "alice", "title": "Own me" }),
    )
    .await;
    let id = created["task"]["id"].as_str().unwrap().to_owned();
    call(
        &handle,
        "task_update",
        json!({ "agent": "alice", "task_id": id, "status": "in_progress" }),
    )
    .await;
    let refused = call(
        &handle,
        "task_update",
        json!({ "agent": "bob", "task_id": id, "status": "done" }),
    )
    .await;
    assert_eq!(refused["status"], "conflict", "{refused}");
    assert_eq!(refused["owner"], "alice");
    assert!(refused["since"].is_string());
    let forced = call(
        &handle,
        "task_update",
        json!({ "agent": "bob", "task_id": id, "status": "done", "force": true }),
    )
    .await;
    assert_eq!(forced["status"], "ok", "{forced}");
    assert_eq!(forced["task"]["status"], "done");
    let notes = forced["task"]["notes"].as_array().unwrap();
    assert!(
        notes.last().unwrap()["text"]
            .as_str()
            .unwrap()
            .starts_with("forced by bob")
    );

    let v1 = call(
        &handle,
        "contract_publish",
        json!({ "agent": "alice", "name": "Seam", "kind": "type", "shape": {"v": 1}, "consumers": ["src/a.rs"] }),
    )
    .await;
    assert_eq!(v1["status"], "ok", "{v1}");
    let kept = call(
        &handle,
        "contract_publish",
        json!({ "agent": "alice", "name": "Seam", "kind": "type", "shape": {"v": 2}, "expected_version": 1 }),
    )
    .await;
    assert_eq!(kept["status"], "ok", "{kept}");
    assert_eq!(kept["contract"]["consumers"], json!(["src/a.rs"]));
    let moved = call(
        &handle,
        "contract_publish",
        json!({ "agent": "alice", "name": "Seam", "kind": "type", "shape": {"v": 3}, "consumers": ["src/b.rs"] }),
    )
    .await;
    assert_eq!(moved["contract"]["consumers"], json!(["src/b.rs"]));
    let notices = call(
        &handle,
        "notice_list",
        json!({ "agent": "carol", "path": "src/a.rs" }),
    )
    .await;
    assert_eq!(
        notices["count"], 2,
        "old consumer hears about v2 and v3: {notices}"
    );
    let stale = call(
        &handle,
        "contract_publish",
        json!({ "agent": "bob", "name": "Seam", "kind": "type", "shape": {"v": 4}, "expected_version": 2 }),
    )
    .await;
    assert_eq!(stale["status"], "conflict", "{stale}");
    assert_eq!(stale["current_version"], 3);
    assert_eq!(stale["published_by"], "alice");
    handle.shutdown().await.unwrap();
}

/// A task whose owner goes silent longer than the orphan threshold returns
/// to `todo`, with a note, and `status` counts it.
#[tokio::test]
async fn silent_owners_lose_in_progress_tasks() {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(ManualClock::new(
        Utc.with_ymd_and_hms(2026, 9, 16, 0, 0, 0).unwrap(),
    ));
    let handle = start(options(dir.path(), Some(Arc::clone(&clock))))
        .await
        .unwrap();
    let created = call(
        &handle,
        "task_create",
        json!({ "agent": "alice", "title": "Orphan me" }),
    )
    .await;
    let id = created["task"]["id"].as_str().unwrap().to_owned();
    let pulled = call(&handle, "task_pull", json!({ "agent": "ghost" })).await;
    assert_eq!(pulled["task"]["id"], id);
    clock.advance(Duration::seconds(1799));
    let still = call(
        &handle,
        "task_list",
        json!({ "agent": "alice", "status": "in_progress" }),
    )
    .await;
    assert_eq!(still["count"], 1, "{still}");
    clock.advance(Duration::seconds(2));
    let back = call(
        &handle,
        "task_list",
        json!({ "agent": "alice", "status": "todo" }),
    )
    .await;
    assert_eq!(back["count"], 1, "{back}");
    let status = call(&handle, "status", json!({})).await;
    assert_eq!(status["tasks_orphaned"], 1, "{status}");
    handle.shutdown().await.unwrap();
}

/// Every stdio shim keeps an SSE stream open on `/mcp`. Shutdown must not
/// wait for it: state is synced, the stream is cut at the deadline, and
/// `daemon.json` is removed.
#[tokio::test]
async fn shutdown_does_not_wait_for_an_open_sse_stream() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let dir = tempfile::tempdir().unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();
    let url = handle.mcp_url();
    // A session, then a standalone SSE stream for it, held open.
    let init = reqwest::Client::new()
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#)
        .send()
        .await
        .unwrap();
    let session = init
        .headers()
        .get("mcp-session-id")
        .expect("a session id")
        .to_str()
        .unwrap()
        .to_owned();
    let mut stream = tokio::net::TcpStream::connect(url_host(&url))
        .await
        .unwrap();
    stream
        .write_all(
            format!(
                "GET /mcp HTTP/1.1\r\nHost: {}\r\nAccept: text/event-stream\r\nMcp-Session-Id: {session}\r\n\r\n",
                url_host(&url)
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut head = [0_u8; 256];
    let n = tokio::time::timeout(std::time::Duration::from_secs(3), stream.read(&mut head))
        .await
        .expect("response headers before the timeout")
        .unwrap();
    assert!(
        String::from_utf8_lossy(&head[..n]).starts_with("HTTP/1.1 200"),
        "expected an open SSE stream, got {:?}",
        String::from_utf8_lossy(&head[..n])
    );
    // Something to sync on the way out.
    call(
        &handle,
        "claim",
        json!({ "agent": "alice", "paths": ["src/a.rs"], "reason": "r" }),
    )
    .await;
    let daemon_json = dir.path().join(".tirith/runtime/daemon.json");
    assert!(daemon_json.exists());
    let started = std::time::Instant::now();
    tokio::time::timeout(std::time::Duration::from_secs(3), handle.shutdown())
        .await
        .expect("shutdown must finish within 3 s despite the open stream")
        .unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(3));
    assert!(!daemon_json.exists(), "daemon.json must be removed");
    let claims = std::fs::read_to_string(dir.path().join(".tirith/runtime/claims.json")).unwrap();
    assert!(
        claims.contains("src/a.rs"),
        "the claim was synced: {claims}"
    );
    drop(stream);
}

/// A `daemon.json` written by another (newer) daemon is never removed by
/// this one's shutdown.
#[tokio::test]
async fn shutdown_leaves_another_daemons_record_alone() {
    let dir = tempfile::tempdir().unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();
    let daemon_json = dir.path().join(".tirith/runtime/daemon.json");
    let mut record: Value =
        serde_json::from_str(&std::fs::read_to_string(&daemon_json).unwrap()).unwrap();
    assert_eq!(record["pid"], std::process::id());
    record["pid"] = json!(std::process::id() + 1);
    std::fs::write(&daemon_json, serde_json::to_vec(&record).unwrap()).unwrap();
    handle.shutdown().await.unwrap();
    let after: Value =
        serde_json::from_str(&std::fs::read_to_string(&daemon_json).unwrap()).unwrap();
    assert_eq!(after["pid"], std::process::id() + 1);
}

/// A daemon given a registry path announces itself there on start and
/// leaves on shutdown, which is how the menu bar tray finds daemons.
#[tokio::test]
async fn a_daemon_registers_on_start_and_unregisters_on_shutdown() {
    use tirith::registry::Registry;

    let dir = tempfile::tempdir().unwrap();
    let registry = dir.path().join("state/tirith/daemons.json");
    let mut options = options(dir.path(), None);
    options.registry = Some(registry.clone());
    let handle = start(options).await.unwrap();
    let entries = Registry::load(&registry).unwrap();
    assert_eq!(entries.entries().len(), 1, "{entries:?}");
    let entry = &entries.entries()[0];
    assert_eq!(entry.pid, std::process::id());
    assert_eq!(entry.root, dir.path());
    assert_eq!(entry.dashboard_url, handle.dashboard_url());
    handle.shutdown().await.unwrap();
    assert!(Registry::load(&registry).unwrap().entries().is_empty());
}

/// ADR-0020: a message rides on the recipient's next result, once; a
/// broadcast reaches the agents seen recently; listing pages; and the
/// text limit is enforced.
#[tokio::test]
async fn messages_ride_on_the_next_result_once() {
    let dir = tempfile::tempdir().unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();
    // bob and carol are "seen" before the broadcast; dave is not.
    call(&handle, "status", json!({ "agent": "bob" })).await;
    call(&handle, "status", json!({ "agent": "carol" })).await;

    let sent = call(
        &handle,
        "message_send",
        json!({ "agent": "alice", "to": "bob", "text": "take task X", "paths": ["src/a.rs"] }),
    )
    .await;
    assert_eq!(sent["status"], "ok", "{sent}");
    assert_eq!(sent["message"]["to"], "bob");
    assert_eq!(
        sent["message"]["id"].as_str().unwrap().len(),
        8,
        "compact id"
    );

    // Delivered on bob's next call, whatever it is, exactly once.
    let claim = call(
        &handle,
        "claim",
        json!({ "agent": "bob", "paths": ["src/b.rs"], "reason": "r" }),
    )
    .await;
    assert_eq!(claim["status"], "ok", "{claim}");
    let inbox = claim["inbox"]
        .as_array()
        .expect("inbox on bob's next result");
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0]["from"], "alice");
    assert_eq!(inbox[0]["text"], "take task X");
    assert!(claim.get("inbox_more").is_none());
    let again = call(&handle, "status", json!({ "agent": "bob" })).await;
    assert!(again.get("inbox").is_none(), "not delivered twice: {again}");
    // Nobody else sees it; alice sees her own message in her history only.
    let carol = call(&handle, "status", json!({ "agent": "carol" })).await;
    assert!(carol.get("inbox").is_none());
    let mine = call(&handle, "message_list", json!({ "agent": "alice" })).await;
    assert_eq!(mine["count"], 1, "{mine}");

    // A broadcast reaches bob and carol (seen before it), not dave or alice.
    call(
        &handle,
        "message_send",
        json!({ "agent": "alice", "to": "*", "text": "server.rs is free" }),
    )
    .await;
    for who in ["bob", "carol"] {
        let next = call(&handle, "status", json!({ "agent": who })).await;
        assert_eq!(
            next["inbox"].as_array().map(Vec::len),
            Some(1),
            "{who}: {next}"
        );
    }
    let dave = call(&handle, "status", json!({ "agent": "dave" })).await;
    assert!(dave.get("inbox").is_none(), "{dave}");
    let alice = call(&handle, "status", json!({ "agent": "alice" })).await;
    assert!(alice.get("inbox").is_none(), "{alice}");

    // Seven more to bob: the inbox carries five and counts the rest.
    for i in 0..7 {
        call(
            &handle,
            "message_send",
            json!({ "agent": "alice", "to": "bob", "text": format!("m{i}") }),
        )
        .await;
    }
    let batch = call(&handle, "status", json!({ "agent": "bob" })).await;
    assert_eq!(batch["inbox"].as_array().map(Vec::len), Some(5), "{batch}");
    assert_eq!(batch["inbox_more"], 2);
    assert_eq!(batch["inbox"][0]["text"], "m6", "newest first");

    // Listing pages with a cursor and filters by conversation.
    let page = call(
        &handle,
        "message_list",
        json!({ "agent": "bob", "with": "alice", "limit": 4 }),
    )
    .await;
    assert_eq!(page["count"], 4, "{page}");
    assert_eq!(page["total"], 9, "1 direct + 1 broadcast + 7: {page}");
    assert_eq!(page["truncated"], true);
    let rest = call(
        &handle,
        "message_list",
        json!({ "agent": "bob", "with": "alice", "limit": 10, "before": page["next_before"] }),
    )
    .await;
    assert_eq!(rest["count"], 5, "{rest}");

    // Too long is invalid; the text line stays short even with an inbox.
    let long = "x".repeat(1001);
    let refused = call(
        &handle,
        "message_send",
        json!({ "agent": "alice", "to": "bob", "text": long }),
    )
    .await;
    assert_eq!(refused["status"], "invalid", "{refused}");
    handle.shutdown().await.unwrap();
}

/// A claim carries a brief: the newest five unread notices for the paths
/// with `more` counting the rest, contracts and decisions the same way,
/// and empty sections omitted. The next claim shows the next five without
/// repeating one, delivery is not an ack, and `brief: false` returns the
/// bare outcome (ADR-0014).
#[tokio::test]
async fn claim_briefs_unread_notices_five_at_a_time() {
    let dir = tempfile::tempdir().unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();
    for i in 0..40 {
        call(
            &handle,
            "notice_publish",
            json!({ "agent": "other", "kind": "behavior", "summary": format!("change {i}"),
                "affected_paths": ["src/m/f.rs"] }),
        )
        .await;
    }
    call(
        &handle,
        "contract_publish",
        json!({ "agent": "other", "name": "Widget", "kind": "function",
            "shape": { "fn": "w()" }, "consumers": ["src/m"] }),
    )
    .await;
    call(
        &handle,
        "decision_record",
        json!({ "agent": "other", "title": "Use widgets", "decision": "yes",
            "affects_paths": ["src/m"] }),
    )
    .await;

    let first = call(
        &handle,
        "claim",
        json!({ "agent": "me", "paths": ["src/m/f.rs"], "reason": "r" }),
    )
    .await;
    assert_eq!(first["status"], "ok", "{first}");
    let notices = first["notices"].as_array().unwrap();
    assert_eq!(notices.len(), 5);
    assert_eq!(notices[0]["summary"], "change 39", "newest first");
    assert_eq!(notices[0]["id"].as_str().unwrap().len(), 8);
    assert!(notices[0].get("acked_by").is_none());
    assert_eq!(first["more"]["notices"], 35);
    assert_eq!(first["contracts"][0]["name"], "Widget");
    assert_eq!(first["contracts"][0]["version"], 1);
    assert_eq!(first["more"]["contracts"], 0);
    assert_eq!(first["decisions"][0]["title"], "Use widgets");
    assert!(first.get("memory").is_none(), "empty sections are omitted");

    let second = call(
        &handle,
        "claim",
        json!({ "agent": "me", "paths": ["src/m/f.rs"], "reason": "r" }),
    )
    .await;
    let again = second["notices"].as_array().unwrap();
    assert_eq!(again.len(), 5);
    assert_eq!(
        again[0]["summary"], "change 34",
        "the next five, not the same"
    );
    assert_eq!(second["more"]["notices"], 30);

    let unread = call(
        &handle,
        "notice_list",
        json!({ "agent": "me", "path": "src/m/f.rs", "unread": true }),
    )
    .await;
    assert_eq!(
        unread["total"], 30,
        "the two briefs delivered ten; delivery is the acknowledgement (ADR-0021)"
    );

    let bare = call(
        &handle,
        "claim",
        json!({ "agent": "me", "paths": ["src/m/f.rs"], "reason": "r", "brief": false }),
    )
    .await;
    assert_eq!(bare["status"], "ok");
    for key in ["notices", "contracts", "decisions", "memory", "more"] {
        assert!(bare.get(key).is_none(), "{key} present without brief");
    }
    let refused = call(
        &handle,
        "claim",
        json!({ "agent": "someone", "paths": ["src/m/f.rs"], "reason": "r" }),
    )
    .await;
    assert_eq!(refused["status"], "conflict");
    assert!(refused.get("more").is_none(), "conflicts carry no brief");
    handle.shutdown().await.unwrap();
}

/// However much matches, the whole `ok` claim response stays under
/// `BRIEF_MAX_BYTES`: the oldest rows of the largest section are dropped
/// and its `more` count raised, so nothing is silently lost.
#[tokio::test]
async fn claim_brief_stays_under_the_byte_cap() {
    let dir = tempfile::tempdir().unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();
    for i in 0..40 {
        call(
            &handle,
            "notice_publish",
            json!({ "agent": "other", "kind": "signature",
                "summary": format!("{i:03} {}", "s".repeat(157)),
                "affected_paths": ["src/m/f.rs"] }),
        )
        .await;
        call(
            &handle,
            "decision_record",
            json!({ "agent": "other", "title": format!("Decision {i:03} {}", "t".repeat(100)),
                "decision": "yes", "affects_paths": ["src/m"] }),
        )
        .await;
    }
    for i in 0..20 {
        call(
            &handle,
            "contract_publish",
            json!({ "agent": "other", "name": format!("Contract {i:03} {}", "n".repeat(80)),
                "kind": "function", "shape": { "fn": "f()" }, "consumers": ["src/m"] }),
        )
        .await;
        call(
            &handle,
            "memory_write",
            json!({ "agent": "other", "title": format!("Note {i:03} {}", "w".repeat(80)),
                "body": "b".repeat(400), "paths": ["src/m"] }),
        )
        .await;
    }
    let claim = call(
        &handle,
        "claim",
        json!({ "agent": "me", "paths": ["src/m/f.rs"], "reason": "r" }),
    )
    .await;
    assert_eq!(claim["status"], "ok", "{claim}");
    let bytes = serde_json::to_vec(&claim).unwrap().len();
    assert!(bytes <= tirith::server::BRIEF_MAX_BYTES, "{bytes} bytes");
    for (section, seeded) in [
        ("notices", 40),
        ("decisions", 40),
        ("contracts", 20),
        ("memory", 20),
    ] {
        let shown = claim[section].as_array().map_or(0, Vec::len);
        let more = claim["more"][section].as_u64().unwrap();
        assert_eq!(
            shown as u64 + more,
            seeded,
            "{section}: {shown} shown, {more} more"
        );
        assert!(shown <= 5, "{section} shows {shown}");
    }
    assert!(claim["notices"].as_array().is_some_and(|n| !n.is_empty()));
    handle.shutdown().await.unwrap();
}

/// A lease that ended is reported once, on the owner's next call whatever
/// the tool, with a warning prefix on the text line; whoever claims the
/// path soon after learns who held it (ADR-0015).
#[tokio::test]
async fn a_lost_lease_is_reported_once_and_names_the_previous_owner() {
    use rmcp::model::CallToolRequestParams;

    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(ManualClock::new(
        Utc.with_ymd_and_hms(2026, 9, 16, 3, 0, 0).unwrap(),
    ));
    let handle = start(options(dir.path(), Some(clock.clone())))
        .await
        .unwrap();
    call(
        &handle,
        "claim",
        json!({ "agent": "alice", "paths": ["a"], "reason": "r", "ttl_secs": 30 }),
    )
    .await;
    clock.advance(Duration::seconds(31));

    let client = raw_client(&handle.mcp_url()).await;
    let arguments = json!({ "agent": "alice" }).as_object().cloned().unwrap();
    let result = client
        .call_tool(CallToolRequestParams::new("claims_list").with_arguments(arguments))
        .await
        .unwrap();
    let structured = result.structured_content.as_ref().unwrap();
    assert_eq!(structured["lost"][0]["path"], "a", "{structured}");
    assert_eq!(structured["lost"][0]["owner"], "alice");
    let text = result.content[0].as_text().unwrap().text.as_str();
    assert!(
        text.starts_with("warning: lost lease on 1 path(s); ok:"),
        "{text}"
    );
    let _ = client.cancel().await;

    let again = call(&handle, "claims_list", json!({ "agent": "alice" })).await;
    assert!(again.get("lost").is_none(), "reported once: {again}");

    let bob = call(
        &handle,
        "claim",
        json!({ "agent": "bob", "paths": ["a"], "reason": "r" }),
    )
    .await;
    assert_eq!(bob["status"], "ok");
    assert_eq!(bob["previous_owner"][0]["owner"], "alice", "{bob}");
    assert_eq!(bob["previous_owner"][0]["path"], "a");
    handle.shutdown().await.unwrap();
}

/// Activity renews a lease only up to four TTLs from when it was claimed;
/// after that it ends like any other and the owner sees it in `lost`.
#[tokio::test]
async fn activity_cannot_extend_a_lease_past_four_ttls() {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(ManualClock::new(
        Utc.with_ymd_and_hms(2026, 9, 16, 3, 0, 0).unwrap(),
    ));
    let handle = start(options(dir.path(), Some(clock.clone())))
        .await
        .unwrap();
    call(
        &handle,
        "claim",
        json!({ "agent": "alice", "paths": ["a"], "reason": "r", "ttl_secs": 30 }),
    )
    .await;
    for _ in 0..4 {
        clock.advance(Duration::seconds(25));
        let renewed = call(&handle, "claims_list", json!({ "agent": "alice" })).await;
        assert!(renewed.get("lost").is_none(), "still held: {renewed}");
    }
    clock.advance(Duration::seconds(25));
    let after = call(&handle, "claims_list", json!({ "agent": "alice" })).await;
    assert_eq!(after["lost"][0]["path"], "a", "125 s > 4 x 30 s: {after}");
    let bob = call(
        &handle,
        "claim",
        json!({ "agent": "bob", "paths": ["a"], "reason": "r" }),
    )
    .await;
    assert_eq!(bob["status"], "ok");
    handle.shutdown().await.unwrap();
}

/// ADR-0027: the lead is whoever holds the reserved `.tirith/lead` claim.
/// `status` names it, and it is gone after a release or when the lease ends.
#[tokio::test]
async fn status_reports_the_holder_of_the_lead_claim() {
    let dir = tempfile::tempdir().unwrap();
    let clock = Arc::new(ManualClock::new(
        Utc.with_ymd_and_hms(2026, 9, 17, 3, 0, 0).unwrap(),
    ));
    let handle = start(options(dir.path(), Some(clock.clone())))
        .await
        .unwrap();
    let status = call(&handle, "status", json!({})).await;
    assert!(status["lead"].is_null(), "no lead yet: {status}");

    let lead_claim =
        json!({ "agent": "boss", "paths": [".tirith/lead"], "reason": "swarm", "ttl_secs": 30 });
    let claimed = call(&handle, "claim", lead_claim.clone()).await;
    assert_eq!(claimed["status"], "ok", "{claimed}");
    let refused = call(
        &handle,
        "claim",
        json!({ "agent": "worker", "paths": [".tirith/lead"], "reason": "r" }),
    )
    .await;
    assert_eq!(refused["status"], "conflict", "one lead at a time");
    let status = call(&handle, "status", json!({ "verbose": true })).await;
    assert_eq!(status["lead"], "boss", "{status}");
    assert!(status["lead_expires_at"].is_string(), "{status}");

    call(&handle, "release", json!({ "agent": "boss" })).await;
    let status = call(&handle, "status", json!({})).await;
    assert!(status["lead"].is_null(), "released: {status}");

    call(&handle, "claim", lead_claim).await;
    assert_eq!(call(&handle, "status", json!({})).await["lead"], "boss");
    clock.advance(Duration::seconds(31));
    let status = call(&handle, "status", json!({})).await;
    assert!(status["lead"].is_null(), "expired: {status}");
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_claim_with_wait_secs_is_granted_when_the_holder_releases() {
    let dir = tempfile::tempdir().unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();
    let alice = call(
        &handle,
        "claim",
        json!({ "agent": "alice", "paths": ["src/auth"], "reason": "r" }),
    )
    .await;
    assert_eq!(alice["status"], "ok");

    let started = std::time::Instant::now();
    let release_later = async {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        call(&handle, "release", json!({ "agent": "alice" })).await
    };
    let wait = call(
        &handle,
        "claim",
        json!({ "agent": "bob", "paths": ["src/auth/login.rs"], "reason": "r", "wait_secs": 30 }),
    );
    let (released, bob) = tokio::join!(release_later, wait);
    let waited = started.elapsed();
    assert_eq!(released["status"], "ok");
    assert_eq!(bob["status"], "ok", "{bob}");
    assert!(
        waited >= std::time::Duration::from_millis(900)
            && waited < std::time::Duration::from_secs(5),
        "waited {waited:?}"
    );

    // Without anyone releasing, a short wait ends in the usual conflict.
    let carol = call(
        &handle,
        "claim",
        json!({ "agent": "carol", "paths": ["src/auth"], "reason": "r", "wait_secs": 1 }),
    )
    .await;
    assert_eq!(carol["status"], "conflict");
    assert_eq!(carol["conflicts"][0]["owner"], "bob");
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_claim_with_wait_secs_is_granted_when_the_lease_expires() {
    let dir = tempfile::tempdir().unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();
    call(
        &handle,
        "claim",
        json!({ "agent": "alice", "paths": ["a"], "reason": "r", "ttl_secs": 1 }),
    )
    .await;
    let started = std::time::Instant::now();
    // Nobody calls while bob waits, so only the expiry timer can wake him.
    let bob = call(
        &handle,
        "claim",
        json!({ "agent": "bob", "paths": ["a"], "reason": "r", "wait_secs": 30 }),
    )
    .await;
    assert_eq!(bob["status"], "ok", "{bob}");
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_task_pull_with_wait_secs_takes_a_task_created_while_it_waits() {
    let dir = tempfile::tempdir().unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();
    // The only task is under alice's claim, so bob's pull waits.
    hold_a_task(&handle, "fix login").await;
    let started = std::time::Instant::now();
    let create_later = async {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        call(
            &handle,
            "task_create",
            json!({ "agent": "planner", "title": "late work" }),
        )
        .await
    };
    // Above the 120 s cap: clamped like claim's wait_secs, not refused.
    let wait = call(
        &handle,
        "task_pull",
        json!({ "agent": "bob", "wait_secs": 600 }),
    );
    let (created, bob) = tokio::join!(create_later, wait);
    let waited = started.elapsed();
    assert_eq!(created["status"], "ok");
    assert_eq!(bob["status"], "ok", "{bob}");
    assert_eq!(bob["task"]["title"], "late work");
    assert_eq!(bob["task"]["status"], "in_progress");
    assert!(
        waited >= std::time::Duration::from_millis(900)
            && waited < std::time::Duration::from_secs(5),
        "waited {waited:?}"
    );
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_task_pull_with_wait_secs_wakes_when_a_dependency_is_done() {
    let dir = tempfile::tempdir().unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();
    let first = call(
        &handle,
        "task_create",
        json!({ "agent": "planner", "title": "server side" }),
    )
    .await;
    let first_id = first["task"]["id"].as_str().unwrap().to_owned();
    call(
        &handle,
        "task_create",
        json!({ "agent": "planner", "title": "client side", "depends_on": [first_id] }),
    )
    .await;
    let alice = call(&handle, "task_pull", json!({ "agent": "alice" })).await;
    assert_eq!(alice["task"]["title"], "server side");

    let started = std::time::Instant::now();
    let finish_later = async {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        call(
            &handle,
            "task_update",
            json!({ "agent": "alice", "task_id": first_id, "status": "done" }),
        )
        .await
    };
    let wait = call(
        &handle,
        "task_pull",
        json!({ "agent": "bob", "wait_secs": 30 }),
    );
    let (done, bob) = tokio::join!(finish_later, wait);
    let waited = started.elapsed();
    assert_eq!(done["status"], "ok");
    assert_eq!(bob["task"]["title"], "client side", "{bob}");
    assert!(
        waited >= std::time::Duration::from_millis(900)
            && waited < std::time::Duration::from_secs(5),
        "waited {waited:?}"
    );
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_task_pull_with_wait_secs_wakes_when_the_claim_on_its_paths_is_released() {
    let dir = tempfile::tempdir().unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();
    call(
        &handle,
        "claim",
        json!({ "agent": "alice", "paths": ["src/auth"], "reason": "r" }),
    )
    .await;
    call(
        &handle,
        "task_create",
        json!({ "agent": "planner", "title": "fix login", "paths": ["src/auth/login.rs"] }),
    )
    .await;

    let started = std::time::Instant::now();
    let release_later = async {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        call(&handle, "release", json!({ "agent": "alice" })).await
    };
    let wait = call(
        &handle,
        "task_pull",
        json!({ "agent": "bob", "wait_secs": 30 }),
    );
    let (released, bob) = tokio::join!(release_later, wait);
    let waited = started.elapsed();
    assert_eq!(released["status"], "ok");
    assert_eq!(bob["task"]["title"], "fix login", "{bob}");
    assert!(bob.get("waiting_on").is_none(), "free once released: {bob}");
    assert!(
        waited >= std::time::Duration::from_millis(900)
            && waited < std::time::Duration::from_secs(5),
        "waited {waited:?}"
    );
    handle.shutdown().await.unwrap();
}

/// With no `todo` task, nothing can free one, so a pull answers `none` at
/// once whatever its `wait_secs`: agents no longer idle two minutes at the
/// end of a run (ADR-0028, revised).
#[tokio::test]
async fn a_task_pull_with_wait_secs_answers_none_at_once_when_no_task_is_todo() {
    let dir = tempfile::tempdir().unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();
    let pull = json!({ "agent": "bob", "wait_secs": 120 });
    let started = std::time::Instant::now();
    let bob = call(&handle, "task_pull", pull.clone()).await;
    let waited = started.elapsed();
    assert_eq!(bob["status"], "none", "{bob}");
    assert!(
        waited < std::time::Duration::from_millis(200),
        "empty board: {waited:?}"
    );

    // One task done, one in progress: still nothing todo.
    for title in ["finished", "underway"] {
        call(
            &handle,
            "task_create",
            json!({ "agent": "planner", "title": title }),
        )
        .await;
        let alice = call(&handle, "task_pull", json!({ "agent": "alice" })).await;
        assert_eq!(alice["task"]["title"], title);
        if title == "finished" {
            let id = alice["task"]["id"].as_str().unwrap();
            call(
                &handle,
                "task_update",
                json!({ "agent": "alice", "task_id": id, "status": "done" }),
            )
            .await;
        }
    }
    let started = std::time::Instant::now();
    let bob = call(&handle, "task_pull", pull).await;
    let waited = started.elapsed();
    assert_eq!(bob["status"], "none", "{bob}");
    assert!(
        waited < std::time::Duration::from_millis(200),
        "no todo task: {waited:?}"
    );
    handle.shutdown().await.unwrap();
}

/// A candidate whose only overlap is another agent's in-progress task
/// paths is handed out at once, with `waiting_on`; waiting would only
/// serialize agents on shared files that edit windows already coordinate.
#[tokio::test]
async fn a_task_pull_with_wait_secs_takes_a_task_behind_another_agents_task_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();
    for title in ["register route a", "register route b"] {
        call(
            &handle,
            "task_create",
            json!({ "agent": "planner", "title": title, "paths": ["app/router.py"] }),
        )
        .await;
    }
    let alice = call(&handle, "task_pull", json!({ "agent": "alice" })).await;
    assert_eq!(alice["task"]["title"], "register route a");

    let started = std::time::Instant::now();
    let bob = call(
        &handle,
        "task_pull",
        json!({ "agent": "bob", "wait_secs": 120 }),
    )
    .await;
    let waited = started.elapsed();
    assert_eq!(bob["task"]["title"], "register route b", "{bob}");
    assert_eq!(
        bob["waiting_on"],
        json!([{ "path": "app/router.py", "owner": "alice" }]),
        "{bob}"
    );
    assert!(
        waited < std::time::Duration::from_millis(200),
        "waited {waited:?}"
    );
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_task_pull_with_wait_secs_returns_none_when_the_wait_ends() {
    let dir = tempfile::tempdir().unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();
    // The only todo task waits on one alice has in progress.
    let first = call(
        &handle,
        "task_create",
        json!({ "agent": "planner", "title": "server side" }),
    )
    .await;
    let first_id = first["task"]["id"].as_str().unwrap().to_owned();
    call(
        &handle,
        "task_create",
        json!({ "agent": "planner", "title": "client side", "depends_on": [first_id] }),
    )
    .await;
    call(&handle, "task_pull", json!({ "agent": "alice" })).await;
    let started = std::time::Instant::now();
    let bob = call(
        &handle,
        "task_pull",
        json!({ "agent": "bob", "wait_secs": 1 }),
    )
    .await;
    let waited = started.elapsed();
    assert_eq!(bob["status"], "none", "{bob}");
    assert!(
        waited >= std::time::Duration::from_secs(1) && waited < std::time::Duration::from_secs(3),
        "waited {waited:?}"
    );
    handle.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_waiting_task_pull_or_claim_that_is_cancelled_takes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();
    hold_a_task(&handle, "after a cancel").await;
    // The client cancels both requests, as an interrupted agent's client does.
    let client = raw_client(&handle.mcp_url()).await;
    let mut requests = Vec::new();
    for (tool, args) in [
        ("task_pull", json!({ "agent": "ghost", "wait_secs": 30 })),
        (
            "claim",
            json!({ "agent": "ghost", "paths": ["src/a.rs"], "reason": "r", "wait_secs": 30 }),
        ),
    ] {
        let params =
            CallToolRequestParams::new(tool).with_arguments(args.as_object().unwrap().clone());
        let request = client
            .peer()
            .send_cancellable_request(
                ClientRequest::CallToolRequest(CallToolRequest::new(params)),
                PeerRequestOptions::no_options(),
            )
            .await
            .unwrap();
        requests.push(request);
    }
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    for request in requests {
        request.cancel(None).await.unwrap();
    }
    wait_for_cancelled_claim(&handle, 1).await;

    // What frees up after the cancel stays free for someone else.
    call(&handle, "release", json!({ "agent": "alice" })).await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_ghost_took_nothing(&handle, "after a cancel").await;
    handle.shutdown().await.unwrap();
}

/// A client whose HTTP connection drops while its `claim` or `task_pull`
/// waits gets nothing: the daemon notices the dropped response and stops
/// waiting (ADR-0031). Closing the session does the same.
#[tokio::test]
async fn a_waiting_call_whose_connection_drops_or_session_closes_takes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();
    let url = handle.mcp_url();
    hold_a_task(&handle, "after a hangup").await;

    let session = open_session(&url).await;
    let pull = start_call(
        &url,
        &session,
        "task_pull",
        json!({ "agent": "ghost", "wait_secs": 30 }),
    )
    .await;
    let claim = start_call(
        &url,
        &session,
        "claim",
        json!({ "agent": "ghost", "paths": ["src/a.rs"], "reason": "r", "wait_secs": 30 }),
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    drop(pull);
    drop(claim);
    wait_for_cancelled_claim(&handle, 1).await;

    // Another wait in a second session, ended by closing the session.
    let second = open_session(&url).await;
    let _held_open = start_call(
        &url,
        &second,
        "claim",
        json!({ "agent": "ghost", "paths": ["src/a.rs"], "reason": "r", "wait_secs": 30 }),
    )
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let deleted = reqwest::Client::new()
        .delete(&url)
        .header("Mcp-Session-Id", &second)
        .send()
        .await
        .unwrap();
    assert!(deleted.status().is_success(), "{}", deleted.status());
    wait_for_cancelled_claim(&handle, 2).await;

    call(&handle, "release", json!({ "agent": "alice" })).await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_ghost_took_nothing(&handle, "after a hangup").await;
    handle.shutdown().await.unwrap();
}

/// Waits up to 5 s for `count` `claim_waited` rows with outcome
/// `cancelled`, which is how a test knows the daemon stopped waiting
/// before it frees the paths.
async fn wait_for_cancelled_claim(handle: &tirith::server::ServerHandle, count: usize) {
    let url = format!("{}api/lead?event=claim_waited", handle.dashboard_url());
    let mut log = Value::Null;
    let mut cancelled = 0;
    for _ in 0..50 {
        log = reqwest_get(&url).await;
        cancelled = log["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["details"]["outcome"] == "cancelled")
            .count();
        if cancelled >= count {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(cancelled, count, "cancelled claim_waited rows: {log}");
    let row = &log["entries"][0];
    assert_eq!(row["agent"], "ghost", "{log}");
    assert!(
        row["details"]["waited_ms"].as_u64().unwrap() < 5_000,
        "{log}"
    );
}

/// `alice` claims `src/a.rs` and `planner` adds the task `title` on it, so
/// a waiting `task_pull` or `claim` by anyone else has something to wait for.
async fn hold_a_task(handle: &tirith::server::ServerHandle, title: &str) {
    let claimed = call(
        handle,
        "claim",
        json!({ "agent": "alice", "paths": ["src/a.rs"], "reason": "r" }),
    )
    .await;
    assert_eq!(claimed["status"], "ok", "{claimed}");
    let created = call(
        handle,
        "task_create",
        json!({ "agent": "planner", "title": title, "paths": ["src/a.rs"] }),
    )
    .await;
    assert_eq!(created["status"], "ok", "{created}");
}

/// `ghost` holds no claim and was assigned no task; the task titled
/// `title` is still on the board for the next agent.
async fn assert_ghost_took_nothing(handle: &tirith::server::ServerHandle, title: &str) {
    let claims = call(handle, "claims_list", json!({ "agent": "planner" })).await;
    assert_eq!(claims["count"], 0, "{claims}");
    let listed = call(handle, "task_list", json!({ "agent": "planner" })).await;
    assert_eq!(listed["tasks"][0]["status"], "todo", "{listed}");
    let bob = call(handle, "task_pull", json!({ "agent": "bob" })).await;
    assert_eq!(bob["task"]["title"], title, "{bob}");
    let granted = format!("{}api/lead?event=claim_granted", handle.dashboard_url());
    let granted = reqwest_get(&granted).await;
    assert!(
        granted["entries"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["agent"] != "ghost"),
        "{granted}"
    );
}

/// Opens an MCP session with bare HTTP requests and returns its id.
async fn open_session(url: &str) -> String {
    let client = reqwest::Client::new();
    let init = client
        .post(url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#)
        .send()
        .await
        .unwrap();
    let session = init
        .headers()
        .get("mcp-session-id")
        .expect("a session id")
        .to_str()
        .unwrap()
        .to_owned();
    // Read the initialize reply before the session takes more.
    init.text().await.unwrap();
    let initialized = client
        .post(url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("Mcp-Session-Id", &session)
        .body(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
        .send()
        .await
        .unwrap();
    assert!(
        initialized.status().is_success(),
        "{}",
        initialized.status()
    );
    session
}

/// Sends a `tools/call` in `session` on a connection of its own and
/// returns that connection once the response has begun, so the test can
/// drop it while the call waits.
async fn start_call(url: &str, session: &str, tool: &str, args: Value) -> tokio::net::TcpStream {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let host = url_host(url);
    let body = json!({
        "jsonrpc": "2.0", "id": 7, "method": "tools/call",
        "params": { "name": tool, "arguments": args }
    })
    .to_string();
    let mut stream = tokio::net::TcpStream::connect(&host).await.unwrap();
    stream
        .write_all(
            format!(
                "POST /mcp HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nMcp-Session-Id: {session}\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut head = [0_u8; 512];
    let n = tokio::time::timeout(std::time::Duration::from_secs(3), stream.read(&mut head))
        .await
        .expect("response headers before the timeout")
        .unwrap();
    let head = String::from_utf8_lossy(&head[..n]);
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    assert!(
        !head.contains("\"result\""),
        "the call must still be waiting: {head}"
    );
    stream
}

/// A notice is pushed at once to every holder of an affected path, or of an
/// ancestor or descendant of one, as a message from `tirith`; the push counts
/// as seen, so the next brief and unread listing skip it, and the decision
/// log has one row for it.
#[tokio::test]
async fn a_notice_reaches_affected_holders_at_once_and_is_not_briefed_again() {
    let dir = tempfile::tempdir().unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();
    for (agent, path) in [
        ("bob", "src/auth"),
        ("dave", "src/db/pool.rs"),
        ("carol", "src/billing"),
    ] {
        let out = call(
            &handle,
            "claim",
            json!({ "agent": agent, "paths": [path], "reason": "work" }),
        )
        .await;
        assert_eq!(out["status"], "ok");
    }
    let out = call(
        &handle,
        "notice_publish",
        json!({ "agent": "alice", "kind": "behavior", "summary": "token refresh is async", "affected_paths": ["src/auth/token.rs", "src/db"] }),
    )
    .await;
    assert_eq!(out["status"], "ok", "{out}");

    for agent in ["bob", "dave"] {
        let out = call(&handle, "status", json!({ "agent": agent })).await;
        assert_eq!(out["inbox"][0]["from"], "tirith", "{out}");
        let text = out["inbox"][0]["text"].as_str().unwrap_or_default();
        assert!(text.contains("token refresh is async"), "{text}");
    }
    let carol = call(&handle, "status", json!({ "agent": "carol" })).await;
    assert!(carol.get("inbox").is_none(), "{carol}");

    // Pushed means delivered: bob's next brief and unread list skip it.
    let again = call(
        &handle,
        "claim",
        json!({ "agent": "bob", "paths": ["src/auth/token.rs"], "reason": "more" }),
    )
    .await;
    assert_eq!(again["status"], "ok", "{again}");
    assert!(
        again["notices"].as_array().is_none_or(Vec::is_empty),
        "{again}"
    );
    let unread = call(
        &handle,
        "notice_list",
        json!({ "agent": "bob", "unread": true }),
    )
    .await;
    assert_eq!(unread["count"], 0, "{unread}");

    let url = format!("{}api/lead?event=notice_published", handle.dashboard_url());
    let log = reqwest_get(&url).await;
    assert_eq!(log["count"], 1, "{log}");
    let row = &log["entries"][0];
    assert_eq!(row["agent"], "alice");
    assert_eq!(row["candidates"], json!(["bob", "dave"]));
    assert_eq!(row["details"]["pushed"], json!(["bob", "dave"]));
    handle.shutdown().await.unwrap();
}
