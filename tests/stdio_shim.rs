//! The stdio shim: spawned like an MCP client would spawn it, it must start
//! a daemon for the repository and proxy tool calls to it.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

async fn rpc(
    stdin: &mut tokio::process::ChildStdin,
    lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    message: Value,
) -> Option<Value> {
    let expects_reply = message.get("id").is_some();
    stdin
        .write_all(format!("{message}\n").as_bytes())
        .await
        .unwrap();
    if !expects_reply {
        return None;
    }
    let line = tokio::time::timeout(Duration::from_secs(30), lines.next_line())
        .await
        .expect("reply within 30s")
        .unwrap()
        .expect("a reply line");
    Some(serde_json::from_str(&line).unwrap())
}

#[tokio::test]
async fn shim_starts_daemon_and_proxies_tools() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_tirith"))
        .args(["stdio", "--root"])
        .arg(dir.path())
        .args(["--bind", "127.0.0.1:0"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();

    let init = rpc(
        &mut stdin,
        &mut lines,
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "protocolVersion": "2025-06-18", "capabilities": {},
                        "clientInfo": { "name": "test", "version": "0" } }
        }),
    )
    .await
    .unwrap();
    assert_eq!(init["result"]["serverInfo"]["name"], "tirith", "{init}");
    assert!(
        init["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("claim")
    );
    rpc(
        &mut stdin,
        &mut lines,
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
    )
    .await;

    let tools = rpc(
        &mut stdin,
        &mut lines,
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} }),
    )
    .await
    .unwrap();
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"claim") && names.contains(&"decision_list"),
        "{names:?}"
    );

    let claimed = rpc(&mut stdin, &mut lines, json!({
        "jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": { "name": "claim", "arguments": { "agent": "shim-test", "paths": ["src/a"], "reason": "r" } }
    }))
    .await
    .unwrap();
    assert_eq!(
        claimed["result"]["structuredContent"]["status"], "ok",
        "{claimed}"
    );

    // The daemon was started by the shim and recorded itself.
    let info: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join(".tirith/runtime/daemon.json")).unwrap(),
    )
    .unwrap();
    let pid = u32::try_from(info["pid"].as_u64().unwrap()).unwrap();
    assert_ne!(pid, child.id().unwrap(), "daemon is a separate process");
    let health = reqwest::Client::new()
        .get(format!(
            "{}api/health",
            info["dashboard_url"].as_str().unwrap()
        ))
        .send()
        .await
        .unwrap();
    assert!(health.status().is_success());

    // A second shim finds the existing daemon instead of starting another.
    let mut second = Command::new(env!("CARGO_BIN_EXE_tirith"))
        .args(["stdio", "--root"])
        .arg(dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut stdin2 = second.stdin.take().unwrap();
    let mut lines2 = BufReader::new(second.stdout.take().unwrap()).lines();
    rpc(&mut stdin2, &mut lines2, json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "t", "version": "0" } }
    }))
    .await;
    rpc(
        &mut stdin2,
        &mut lines2,
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
    )
    .await;
    let listed = rpc(
        &mut stdin2,
        &mut lines2,
        json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": { "name": "claims_list", "arguments": { "agent": "other" } }
        }),
    )
    .await
    .unwrap();
    assert_eq!(
        listed["result"]["structuredContent"]["count"], 1,
        "same daemon, same state"
    );
    let info_after: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join(".tirith/runtime/daemon.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(info_after["pid"], info["pid"], "no second daemon");

    // Clean up the detached daemon; the shims die with kill_on_drop.
    let _ = Command::new("kill").arg(pid.to_string()).status().await;
}
