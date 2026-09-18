//! The token budgets of ADR-0013, one test per row of its table, measured
//! against a daemon seeded like a busy repository: hundreds of claims,
//! notices and decisions, dozens of contracts and memory notes. Every
//! agent pays these bytes on every call, so a budget that silently grows
//! is a regression. The bounds here may be lowered, never raised without
//! an ADR that names what the raise pays for (ADR-0017, ADR-0033).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "common/raw_client.rs"]
mod raw_client;

use std::net::SocketAddr;

use raw_client::raw_client;
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::{RoleClient, RunningService};
use serde_json::{Value, json};
use tirith::server::{ServeOptions, ServerHandle, start};

/// `tools/list` for the whole tool surface, in serialized JSON chars.
/// Every agent downloads it once per session, so it is a per-session
/// token cost. Raise it only for a real tool or parameter, never for
/// prose; per ADR-0017 every raise names what it pays for:
///
/// - Measured 2026-09-16: 10,445 chars for 17 tools before slimming
///   (largest 1,040); about 6,100 for 20 tools after, with a floor of
///   4,170 if every description were removed.
/// - 6,144 to 6,656 for the paging parameters: `limit` and `before` on
///   five list tools, `all` on two, `verbose` on `status`.
/// - 6,656 to 7,168 for the `memory_delete` tool and the `if_updated_at`
///   input on `memory_write`.
/// - 7,168 to 7,800 for `message_send` and `message_list` (ADR-0020),
///   about 640 chars of schema between them.
/// - 7,800 to 10,500 for MCP tool annotations on all 22 tools and a usage
///   clause on the 17 tools whose guidance was only implied (ADR-0033):
///   about 950 chars of structured hints and 1,800 of prose. The per-tool
///   bound rises from 500 to 650 for the same reason.
const TOOLS_LIST_MAX: usize = 10_500;
/// Any single tool in `tools/list`.
const TOOL_MAX: usize = 650;
/// The text content block of any result.
const TEXT_MAX: usize = 200;
/// Rows a list tool returns without a `limit`.
const DEFAULT_ROWS: usize = 20;
/// `status` without `verbose`, however many claims are live.
const STATUS_MAX: usize = 1024;
/// `claims_list` without `all`, for an agent holding two paths.
const OWN_CLAIMS_MAX: usize = 1024;
/// `renew`.
const RENEW_MAX: usize = 300;
/// A successful `claim` with its brief, on a busy module.
const CLAIM_BRIEF_MAX: usize = 4096;

/// A running daemon with one MCP session to drive it and the temp dir it
/// lives in (dropped last, so the daemon never outlives its root).
struct Daemon {
    handle: Option<ServerHandle>,
    client: RunningService<RoleClient, ()>,
    _dir: tempfile::TempDir,
}

impl Daemon {
    /// An empty daemon.
    async fn empty() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let handle = start(ServeOptions {
            bind: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
            repo_root: dir.path().to_path_buf(),
            clock: None,

            registry: None,
        })
        .await
        .unwrap();
        let client = raw_client(&handle.mcp_url()).await;
        Self {
            handle: Some(handle),
            client,
            _dir: dir,
        }
    }

    /// A daemon seeded like a busy repository: `agents` agents each holding
    /// one path with a notice and a decision on it, 30 tasks, 50 contracts,
    /// 50 memory notes, and 40 notices on `src/busy` that nobody has read.
    async fn seeded(agents: usize) -> Self {
        let daemon = Self::empty().await;
        let peer = daemon.client.peer().clone();
        let mut calls: Vec<(String, Value)> = Vec::new();
        for i in 0..agents {
            let agent = format!("agent-{i}");
            let path = format!("src/m{}/file{i}.rs", i % 30);
            calls.push((
                "claim".into(),
                json!({ "agent": agent, "paths": [path], "reason": format!("editing module {}", i % 30) }),
            ));
            calls.push((
                "notice_publish".into(),
                json!({ "agent": agent, "kind": "behavior", "summary": format!("change {i} in module {}", i % 30), "affected_paths": [path] }),
            ));
            calls.push((
                "decision_record".into(),
                json!({ "agent": agent, "title": format!("decision {i}"), "decision": format!("module {} does it this way", i % 30), "affects_paths": [path] }),
            ));
        }
        for i in 0..30 {
            calls.push((
                "task_create".into(),
                json!({ "agent": "planner", "title": format!("task {i}"), "priority": i % 5, "paths": [format!("src/m{i}")] }),
            ));
        }
        for i in 0..50 {
            calls.push((
                "contract_publish".into(),
                json!({ "agent": "architect", "name": format!("Contract {i}"), "kind": "type", "shape": {"field": i}, "consumers": [format!("src/m{}", i % 30)] }),
            ));
            calls.push((
                "memory_write".into(),
                json!({ "agent": "scribe", "title": format!("Lesson {i} about module {}", i % 30), "body": format!("# Lesson {i}\n\n{}", "A paragraph of prose that a real note would carry, repeated to look like one. ".repeat(20)), "kind": "lesson", "paths": [format!("src/m{}", i % 30)] }),
            ));
        }
        for i in 0..40 {
            calls.push((
                "notice_publish".into(),
                json!({ "agent": "busy-bee", "kind": "signature", "summary": format!("busy change {i}"), "affected_paths": ["src/busy/lib.rs"] }),
            ));
        }
        // Batches of concurrent calls let the persister coalesce writes.
        for batch in calls.chunks(50) {
            let mut set = tokio::task::JoinSet::new();
            for (tool, args) in batch {
                let peer = peer.clone();
                let tool = tool.clone();
                let args = args.as_object().cloned().unwrap();
                set.spawn(async move {
                    peer.call_tool(CallToolRequestParams::new(tool).with_arguments(args))
                        .await
                        .unwrap()
                });
            }
            while let Some(result) = set.join_next().await {
                let result = result.unwrap();
                let status = result.structured_content.as_ref().unwrap()["status"].clone();
                assert_eq!(status, "ok", "seeding failed: {result:?}");
            }
        }
        daemon
    }

    /// One tool call, raw.
    async fn call(&self, tool: &str, args: Value) -> CallToolResult {
        self.client
            .call_tool(
                CallToolRequestParams::new(tool.to_owned())
                    .with_arguments(args.as_object().cloned().unwrap()),
            )
            .await
            .unwrap()
    }

    /// One tool call, its structured outcome.
    async fn outcome(&self, tool: &str, args: Value) -> Value {
        self.call(tool, args).await.structured_content.unwrap()
    }

    async fn stop(mut self) {
        let _ = self.client.cancel().await;
        self.handle.take().unwrap().shutdown().await.unwrap();
    }
}

fn bytes(v: &Value) -> usize {
    serde_json::to_string(v).unwrap().len()
}

/// A compact row: no null fields and no empty arrays, at any depth.
fn assert_compact(row: &Value, context: &str) {
    match row {
        Value::Object(map) => {
            for (key, value) in map {
                assert!(!value.is_null(), "{context}: null field {key:?} in {row}");
                assert!(
                    value.as_array().is_none_or(|a| !a.is_empty()),
                    "{context}: empty array {key:?} in {row}"
                );
                assert_compact(value, context);
            }
        }
        Value::Array(items) => {
            for item in items {
                assert_compact(item, context);
            }
        }
        _ => {}
    }
}

#[tokio::test]
async fn tools_list_stays_under_10500_chars_and_no_tool_over_650() {
    let daemon = Daemon::empty().await;
    let tools = daemon.client.list_all_tools().await.unwrap();
    let total = serde_json::to_string(&tools).unwrap().len();
    eprintln!("tools/list: {total} chars for {} tools", tools.len());
    let mut largest = (String::new(), 0);
    for tool in &tools {
        let size = serde_json::to_string(tool).unwrap().len();
        assert!(
            size <= TOOL_MAX,
            "tool {} is {size} chars, budget {TOOL_MAX}",
            tool.name
        );
        if size > largest.1 {
            largest = (tool.name.to_string(), size);
        }
    }
    assert!(
        total <= TOOLS_LIST_MAX,
        "tools/list is {total} chars for {} tools (largest {} at {}), budget {TOOLS_LIST_MAX}",
        tools.len(),
        largest.0,
        largest.1
    );
    daemon.stop().await;
}

#[tokio::test]
async fn every_result_text_block_is_a_status_line_under_200_bytes() {
    let daemon = Daemon::seeded(30).await;
    let task = daemon
        .outcome("task_pull", json!({ "agent": "worker" }))
        .await["task"]["id"]
        .clone();
    let calls = [
        (
            "claim",
            json!({ "agent": "alice", "paths": ["src/new/a.rs", "src/new/b.rs"], "reason": "r" }),
        ),
        (
            "claim",
            json!({ "agent": "bob", "paths": ["src/new/a.rs"], "reason": "r" }),
        ),
        ("renew", json!({ "agent": "alice" })),
        ("claims_list", json!({ "agent": "alice" })),
        ("claims_list", json!({ "agent": "alice", "all": true })),
        ("release", json!({ "agent": "nobody" })),
        (
            "task_create",
            json!({ "agent": "alice", "title": "Budget the text" }),
        ),
        (
            "task_update",
            json!({ "agent": "worker", "task_id": task, "status": "done", "note": "n" }),
        ),
        (
            "task_update",
            json!({ "agent": "alice", "task_id": "00000000", "status": "done" }),
        ),
        ("task_list", json!({ "agent": "alice" })),
        (
            "contract_publish",
            json!({ "agent": "alice", "name": "Contract 1", "kind": "type", "shape": {"v": 2} }),
        ),
        (
            "contract_get",
            json!({ "agent": "alice", "name": "Contract 1" }),
        ),
        ("contract_list", json!({ "agent": "alice" })),
        (
            "notice_publish",
            json!({ "agent": "alice", "kind": "rename", "summary": "x to y", "affected_paths": ["src/m1"] }),
        ),
        ("notice_list", json!({ "agent": "alice", "path": "src/m1" })),
        (
            "decision_record",
            json!({ "agent": "alice", "title": "T", "decision": "D" }),
        ),
        ("decision_list", json!({ "agent": "alice" })),
        ("status", json!({})),
        (
            "memory_write",
            json!({ "agent": "alice", "title": "Note", "body": "Body." }),
        ),
        ("memory_read", json!({ "agent": "alice", "name": "note" })),
        (
            "memory_search",
            json!({ "agent": "alice", "query": "module" }),
        ),
    ];
    for (tool, args) in calls {
        let result = daemon.call(tool, args).await;
        let status = result.structured_content.as_ref().unwrap()["status"]
            .as_str()
            .unwrap()
            .to_owned();
        let texts: Vec<&str> = result
            .content
            .iter()
            .filter_map(|c| c.as_text().map(|t| t.text.as_str()))
            .collect();
        assert_eq!(texts.len(), 1, "{tool}: one text block");
        let text = texts[0];
        assert!(
            text.len() < TEXT_MAX,
            "{tool}: {} bytes: {text}",
            text.len()
        );
        assert!(
            !text.trim_start().starts_with('{'),
            "{tool}: JSON in the text block: {text}"
        );
        assert!(
            text.starts_with(&status),
            "{tool}: text {text:?} does not start with {status}"
        );
    }
    daemon.stop().await;
}

#[tokio::test]
async fn list_tools_default_to_20_compact_rows_newest_first_with_a_cursor() {
    let daemon = Daemon::seeded(300).await;
    let lists = [
        (
            "claims_list",
            json!({ "agent": "reader", "all": true }),
            "claims",
        ),
        ("task_list", json!({ "agent": "reader" }), "tasks"),
        ("contract_list", json!({ "agent": "reader" }), "contracts"),
        ("notice_list", json!({ "agent": "reader" }), "notices"),
        ("decision_list", json!({ "agent": "reader" }), "decisions"),
    ];
    for (tool, args, key) in lists {
        let outcome = daemon.outcome(tool, args).await;
        let rows = outcome[key]
            .as_array()
            .unwrap_or_else(|| panic!("{tool}: no {key} array in {outcome}"));
        assert!(
            rows.len() <= DEFAULT_ROWS,
            "{tool}: {} rows by default",
            rows.len()
        );
        assert_eq!(outcome["count"], rows.len(), "{tool}: count");
        let total = usize::try_from(
            outcome["total"]
                .as_u64()
                .unwrap_or_else(|| panic!("{tool}: total")),
        )
        .unwrap();
        assert!(
            total > DEFAULT_ROWS,
            "{tool}: the seed should exceed one page, total {total}"
        );
        assert_eq!(outcome["truncated"], true, "{tool}: truncated");
        assert!(
            outcome["next_before"].is_string(),
            "{tool}: a before cursor: {outcome}"
        );
        for row in rows {
            assert_compact(row, tool);
        }
        eprintln!("{tool}: {} rows, {} bytes", rows.len(), bytes(&outcome));
    }
    daemon.stop().await;
}

#[tokio::test]
async fn status_without_verbose_is_under_1_kb_with_300_live_claims() {
    let daemon = Daemon::seeded(300).await;
    let status = daemon.outcome("status", json!({})).await;
    assert_eq!(status["claims"], 300, "{status}");
    let size = bytes(&status);
    eprintln!("status: {size} bytes");
    assert!(size < STATUS_MAX, "status is {size} bytes: {status}");
    assert!(
        status["agents"].is_null(),
        "no agent rows without verbose: {status}"
    );
    daemon.stop().await;
}

#[tokio::test]
async fn claims_list_without_all_is_under_1_kb_for_an_agent_holding_two_paths() {
    let daemon = Daemon::seeded(300).await;
    daemon
        .outcome("claim", json!({ "agent": "pair", "paths": ["src/pair/a.rs", "src/pair/b.rs"], "reason": "two files" }))
        .await;
    let claims = daemon
        .outcome("claims_list", json!({ "agent": "pair" }))
        .await;
    let size = bytes(&claims);
    eprintln!("claims_list (own): {size} bytes");
    assert!(
        size < OWN_CLAIMS_MAX,
        "claims_list is {size} bytes: {claims}"
    );
    assert_eq!(claims["count"], 1, "only the caller's claim: {claims}");
    daemon.stop().await;
}

#[tokio::test]
async fn renew_is_under_300_bytes() {
    let daemon = Daemon::seeded(30).await;
    daemon
        .outcome("claim", json!({ "agent": "holder", "paths": ["src/x/a.rs", "src/x/b.rs", "src/x/c.rs"], "reason": "three files" }))
        .await;
    let renewed = daemon.outcome("renew", json!({ "agent": "holder" })).await;
    let size = bytes(&renewed);
    eprintln!("renew: {size} bytes");
    assert_eq!(renewed["status"], "ok", "{renewed}");
    assert!(size < RENEW_MAX, "renew is {size} bytes: {renewed}");
    daemon.stop().await;
}

/// The brief a claim carries (notices, contracts, decisions, memory for the
/// claimed paths, ADR-0014) must fit with room to spare on a module with 40
/// unread notices.
#[tokio::test]
async fn claim_with_brief_is_under_4_kb_on_a_module_with_40_unread_notices() {
    let daemon = Daemon::seeded(30).await;
    let claim = daemon
        .outcome("claim", json!({ "agent": "newcomer", "paths": ["src/busy/lib.rs"], "reason": "fix", "brief": true }))
        .await;
    assert_eq!(claim["status"], "ok", "{claim}");
    let size = bytes(&claim);
    eprintln!("claim with brief: {size} bytes");
    assert!(size < CLAIM_BRIEF_MAX, "claim is {size} bytes: {claim}");
    daemon.stop().await;
}

#[tokio::test]
async fn memory_search_rows_carry_an_excerpt_and_no_body() {
    let daemon = Daemon::seeded(30).await;
    let found = daemon
        .outcome(
            "memory_search",
            json!({ "agent": "reader", "query": "lesson" }),
        )
        .await;
    let rows = found["notes"].as_array().unwrap();
    assert!(!rows.is_empty(), "{found}");
    for row in rows {
        assert!(
            row.get("body").is_none(),
            "search row carries a body: {row}"
        );
        let excerpt = row["excerpt"]
            .as_str()
            .unwrap_or_else(|| panic!("no excerpt: {row}"));
        assert!(
            excerpt.chars().count() <= 200,
            "excerpt too long: {excerpt}"
        );
    }
    let read = daemon
        .outcome(
            "memory_read",
            json!({ "agent": "reader", "name": rows[0]["permalink"].as_str().unwrap() }),
        )
        .await;
    assert!(
        read["note"]["body"].is_string(),
        "memory_read returns the body: {read}"
    );
    daemon.stop().await;
}
