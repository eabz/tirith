//! End-to-end tests: a real daemon on an ephemeral localhost port, driven
//! through the MCP client the CLI uses.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use chrono::{Duration, TimeZone, Utc};
use serde_json::{Value, json};
use tirith::client::{call_tool, list_tools};
use tirith::clock::ManualClock;
use tirith::server::{ServeOptions, ServerHandle, start};

fn options(root: &Path, clock: Option<Arc<ManualClock>>) -> ServeOptions {
    ServeOptions {
        bind: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
        repo_root: root.to_path_buf(),
        clock: clock.map(|c| c as Arc<dyn tirith::clock::Clock>),
    }
}

async fn call(handle: &ServerHandle, tool: &str, args: Value) -> Value {
    call_tool(&handle.mcp_url(), tool, args).await.unwrap()
}

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
    let notice_id = unread["notices"][0]["id"].as_str().unwrap().to_owned();
    call(
        &handle,
        "notice_ack",
        json!({ "agent": "bob", "notice_id": notice_id }),
    )
    .await;
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
    for expected in [
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
        "notice_ack",
        "decision_record",
        "decision_list",
        "status",
    ] {
        assert!(names.contains(&expected), "missing tool {expected}");
    }
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
