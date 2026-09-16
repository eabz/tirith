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

/// A bare rmcp client, for tests that need the raw `CallToolResult` or the
/// raw `tools/list` rather than the structured value `tirith::client` picks.
async fn raw_client(url: &str) -> rmcp::service::RunningService<rmcp::RoleClient, ()> {
    use rmcp::ServiceExt;
    use rmcp::transport::common::client_side_sse::NeverRetry;
    use rmcp::transport::streamable_http_client::{
        StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
    };
    let mut config = StreamableHttpClientTransportConfig::with_uri(url.to_owned());
    config.retry_config = Arc::new(NeverRetry::default());
    let transport = StreamableHttpClientTransport::with_client(reqwest::Client::default(), config);
    ().serve(transport).await.unwrap()
}

/// Every agent downloads `tools/list` once per session, so its size is a
/// per-session token cost. This pins it so it cannot creep back up; the
/// bound is the measured size plus a small margin, and goes down, not up.
#[tokio::test]
async fn tool_list_stays_small() {
    let dir = tempfile::tempdir().unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();
    let client = raw_client(&handle.mcp_url()).await;
    let tools = client.list_all_tools().await.unwrap();
    let _ = client.cancel().await;
    let total = serde_json::to_string(&tools).unwrap().len();
    eprintln!("tools/list: {total} chars for {} tools", tools.len());
    let mut largest = (String::new(), 0);
    for tool in &tools {
        let size = serde_json::to_string(tool).unwrap().len();
        assert!(
            size <= TOOL_LIMIT,
            "tool {} is {size} chars, limit {TOOL_LIMIT}",
            tool.name
        );
        if size > largest.1 {
            largest = (tool.name.to_string(), size);
        }
    }
    assert!(
        total <= TOOL_LIST_LIMIT,
        "tools/list is {total} chars for {} tools (largest {} at {}), limit {TOOL_LIST_LIMIT}",
        tools.len(),
        largest.0,
        largest.1
    );
    handle.shutdown().await.unwrap();
}

/// Every tool result travels once, as structured content. The text block
/// is a one-line summary for text-only clients, never the JSON again:
/// clients feed the text block to the model, so a copy doubles the cost
/// of every call.
#[tokio::test]
async fn tool_results_carry_a_short_text_line_not_the_json() {
    use rmcp::model::CallToolRequestParams;

    let dir = tempfile::tempdir().unwrap();
    let handle = start(options(dir.path(), None)).await.unwrap();
    let client = raw_client(&handle.mcp_url()).await;
    let calls = [
        (
            "claim",
            json!({ "agent": "alice", "paths": ["src/a.rs", "src/b.rs"], "reason": "r" }),
        ),
        (
            "claim",
            json!({ "agent": "bob", "paths": ["src/a.rs"], "reason": "r" }),
        ),
        ("claims_list", json!({ "agent": "bob" })),
        (
            "task_create",
            json!({ "agent": "alice", "title": "Write the summary" }),
        ),
        (
            "release",
            json!({ "agent": "nobody", "paths": ["src/zzz.rs"] }),
        ),
        ("status", json!({})),
    ];
    for (tool, args) in calls {
        let arguments = args.as_object().cloned().unwrap();
        let result = client
            .call_tool(CallToolRequestParams::new(tool).with_arguments(arguments))
            .await
            .unwrap();
        let structured = result.structured_content.as_ref().unwrap();
        assert!(structured["status"].is_string(), "{tool}: {structured}");
        let texts: Vec<&str> = result
            .content
            .iter()
            .filter_map(|block| block.as_text().map(|t| t.text.as_str()))
            .collect();
        assert_eq!(texts.len(), 1, "{tool} should carry exactly one text block");
        let text = texts[0];
        assert!(
            text.len() < 200,
            "{tool} text is {} bytes: {text}",
            text.len()
        );
        assert!(
            !text.trim_start().starts_with('{'),
            "{tool} text is JSON: {text}"
        );
        assert!(
            text.starts_with(structured["status"].as_str().unwrap()),
            "{tool} text should start with the status: {text}"
        );
    }
    let _ = client.cancel().await;
    handle.shutdown().await.unwrap();
}

/// Bounds for `tool_list_stays_small`, in serialized JSON chars.
/// Measured 2026-09-16: 10,445 for 17 tools before slimming (largest
/// 1,040); about 6,100 for 20 tools after, with a floor of 4,170 if every
/// description were removed. Lower them when the schemas shrink. Raised
/// once, 2026-09-16, from 6,144 to 6,656 for the paging parameters:
/// `limit` and `before` on five list tools, `all` on two, `verbose` on
/// `status` (13 properties, about 450 chars). Do not raise for prose.
// Raised from 6_656 for two real additions in the memory primitive: the
// `memory_delete` tool and the `if_updated_at` input on `memory_write`.
// Per ADR-0017, every raise names what it pays for.
const TOOL_LIST_LIMIT: usize = 7_000;
const TOOL_LIMIT: usize = 500;

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
    let published = call(
        &handle,
        "notice_publish",
        json!({ "agent": "alice", "kind": "rename", "summary": "s", "affected_paths": ["src"] }),
    )
    .await;
    let full = published["notice"]["id"].as_str().unwrap().to_owned();
    let acked = call(
        &handle,
        "notice_ack",
        json!({ "agent": "bob", "notice_id": &full[..8] }),
    )
    .await;
    assert_eq!(acked["status"], "ok", "{acked}");
    let missing = call(
        &handle,
        "notice_ack",
        json!({ "agent": "bob", "notice_id": "zzzz" }),
    )
    .await;
    assert_eq!(missing["status"], "not_found");
    // Every id starts with the empty string, so an empty prefix is refused too.
    let empty = call(
        &handle,
        "notice_ack",
        json!({ "agent": "bob", "notice_id": "" }),
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
    for i in 0..300 {
        let claimed = call(
            &handle,
            "claim",
            json!({ "agent": format!("a{i}"), "paths": [format!("src/m{i}")], "reason": "r" }),
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
    assert_eq!(all["count"], 2);
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
