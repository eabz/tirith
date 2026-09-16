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

/// Stops the daemon recorded under `root` when dropped, so a failing
/// assertion never leaves a `tirith serve` behind in a temp directory.
/// Drop runs during unwinding too, which is the whole point.
struct DaemonGuard {
    root: std::path::PathBuf,
}

impl DaemonGuard {
    fn new(root: &std::path::Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        let record = self.root.join(".tirith/runtime/daemon.json");
        let Ok(text) = std::fs::read_to_string(&record) else {
            return;
        };
        let Ok(info) = serde_json::from_str::<Value>(&text) else {
            return;
        };
        let Some(pid) = info["pid"].as_u64() else {
            return;
        };
        let pid = pid.to_string();
        // SIGINT is the daemon's graceful stop. Drop cannot await, so this
        // uses std::process; the whole wait is bounded.
        let _ = std::process::Command::new("kill")
            .args(["-INT", &pid])
            .status();
        for _ in 0..30 {
            let stat = std::process::Command::new("ps")
                .args(["-o", "stat=", "-p", &pid])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
                .unwrap_or_default();
            if stat.is_empty() || stat.starts_with('Z') {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = std::process::Command::new("kill")
            .args(["-KILL", &pid])
            .status();
    }
}

#[tokio::test]
async fn shim_starts_daemon_and_proxies_tools() {
    let dir = tempfile::tempdir().unwrap();
    let _daemon = DaemonGuard::new(dir.path());
    let (child, mut stdin, mut lines, init) = shim(dir.path(), "127.0.0.1:0").await;
    assert_eq!(init["result"]["serverInfo"]["name"], "tirith", "{init}");
    assert!(
        init["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("claim")
    );

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
    let (_second, mut stdin2, mut lines2, _) = shim(dir.path(), "127.0.0.1:0").await;
    let listed = rpc(
        &mut stdin2,
        &mut lines2,
        json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": { "name": "claims_list", "arguments": { "agent": "other", "all": true } }
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
}

/// The daemon record, once the daemon has written it.
async fn wait_for_daemon(root: &std::path::Path) -> Option<Value> {
    let file = root.join(".tirith/runtime/daemon.json");
    for _ in 0..200 {
        if let Ok(text) = std::fs::read_to_string(&file)
            && let Ok(value) = serde_json::from_str::<Value>(&text)
        {
            return Some(value);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    None
}

/// Spawns a shim for `root` and completes the MCP handshake, returning
/// the child, its pipes, and the `initialize` reply.
async fn shim(
    root: &std::path::Path,
    bind: &str,
) -> (
    tokio::process::Child,
    tokio::process::ChildStdin,
    tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    Value,
) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_tirith"))
        .args(["stdio", "--root"])
        .arg(root)
        .args(["--bind", bind])
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
    rpc(
        &mut stdin,
        &mut lines,
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
    )
    .await;
    (child, stdin, lines, init)
}

#[tokio::test]
async fn shim_replaces_a_daemon_of_another_version() {
    let dir = tempfile::tempdir().unwrap();
    let _daemon = DaemonGuard::new(dir.path());
    // A real daemon of this binary, standing in for the previous release.
    let mut old = Command::new(env!("CARGO_BIN_EXE_tirith"))
        .args(["serve", "--root"])
        .arg(dir.path())
        .args(["--bind", "127.0.0.1:0"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let info = wait_for_daemon(dir.path())
        .await
        .expect("daemon.json never appeared");
    let old_pid = info["pid"].as_u64().unwrap();
    // The shim compares the recorded version as well as the reported one,
    // so an older record is enough to make this daemon look outdated.
    let mut outdated = info.clone();
    outdated["version"] = json!("0.0.1");
    let record = dir.path().join(".tirith/runtime/daemon.json");
    std::fs::write(&record, outdated.to_string()).unwrap();

    let (_shim, mut stdin, mut lines, _) = shim(dir.path(), "127.0.0.1:0").await;
    let status = rpc(
        &mut stdin,
        &mut lines,
        json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": { "name": "status", "arguments": { "agent": "t" } }
        }),
    )
    .await
    .unwrap();
    assert_eq!(
        status["result"]["structuredContent"]["status"], "ok",
        "{status}"
    );

    let exit = tokio::time::timeout(Duration::from_secs(10), old.wait())
        .await
        .expect("old daemon exits")
        .unwrap();
    assert!(
        exit.success() || exit.code().is_none(),
        "old daemon shut down cleanly: {exit}"
    );
    let after: Value = serde_json::from_str(&std::fs::read_to_string(&record).unwrap()).unwrap();
    let new_pid = after["pid"].as_u64().unwrap();
    assert_ne!(new_pid, old_pid, "a fresh daemon replaced the old one");
    assert_eq!(after["version"], env!("CARGO_PKG_VERSION"));
    let log = std::fs::read_to_string(dir.path().join(".tirith/runtime/serve.log")).unwrap();
    assert!(
        log.contains("stopping daemon") && log.contains("recorded as 0.0.1"),
        "restart is logged:\n{log}"
    );
}

#[tokio::test]
async fn shim_replaces_a_record_whose_daemon_is_gone() {
    let dir = tempfile::tempdir().unwrap();
    let _daemon = DaemonGuard::new(dir.path());
    std::fs::create_dir_all(dir.path().join(".tirith/runtime")).unwrap();
    let record = dir.path().join(".tirith/runtime/daemon.json");
    // Nothing listens on port 1, and the pid must never be signalled.
    std::fs::write(
        &record,
        json!({
            "url": "http://127.0.0.1:1/mcp", "dashboard_url": "http://127.0.0.1:1/",
            "pid": 999_999, "started_at": "2026-01-01T00:00:00Z",
            "version": env!("CARGO_PKG_VERSION")
        })
        .to_string(),
    )
    .unwrap();

    let (_shim, mut stdin, mut lines, _) = shim(dir.path(), "127.0.0.1:0").await;
    let status = rpc(
        &mut stdin,
        &mut lines,
        json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": { "name": "status", "arguments": { "agent": "t" } }
        }),
    )
    .await
    .unwrap();
    assert_eq!(
        status["result"]["structuredContent"]["status"], "ok",
        "{status}"
    );

    let after: Value = serde_json::from_str(&std::fs::read_to_string(&record).unwrap()).unwrap();
    assert_ne!(after["pid"], 999_999, "the dead record was replaced");
    assert_ne!(after["url"], "http://127.0.0.1:1/mcp");
    let log =
        std::fs::read_to_string(dir.path().join(".tirith/runtime/serve.log")).unwrap_or_default();
    assert!(
        !log.contains("stopping daemon"),
        "nothing was signalled:\n{log}"
    );
}

#[tokio::test]
async fn shim_leaves_another_repositorys_daemon_alone() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let _daemon_a = DaemonGuard::new(a.path());
    let _daemon_b = DaemonGuard::new(b.path());
    // Repository A's daemon: healthy, current, and not ours to stop.
    let _daemon_a = Command::new(env!("CARGO_BIN_EXE_tirith"))
        .args(["serve", "--root"])
        .arg(a.path())
        .args(["--bind", "127.0.0.1:0"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let info_a = wait_for_daemon(a.path()).await.expect("daemon A's record");
    let pid_a = info_a["pid"].as_u64().unwrap();
    let url_a = info_a["url"].as_str().unwrap().to_owned();
    // Repository B's record points at A's daemon, which is what a stale
    // record looks like when both repositories use the default port.
    std::fs::create_dir_all(b.path().join(".tirith/runtime")).unwrap();
    let record_b = b.path().join(".tirith/runtime/daemon.json");
    std::fs::write(&record_b, info_a.to_string()).unwrap();
    // Ask for A's exact address so the first bind fails and the shim has to
    // fall back to an ephemeral port.
    let bind_a = url_a
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap()
        .to_owned();

    let (_shim, mut stdin, mut lines, _) = shim(b.path(), &bind_a).await;
    let status = rpc(
        &mut stdin,
        &mut lines,
        json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": { "name": "status", "arguments": { "agent": "t" } }
        }),
    )
    .await
    .unwrap();
    assert_eq!(
        status["result"]["structuredContent"]["status"], "ok",
        "{status}"
    );

    // A is untouched.
    let health_a = reqwest::Client::new()
        .get(format!(
            "{}api/health",
            info_a["dashboard_url"].as_str().unwrap()
        ))
        .send()
        .await
        .unwrap();
    assert!(health_a.status().is_success(), "daemon A still answers");
    let info_a_after: Value = serde_json::from_str(
        &std::fs::read_to_string(a.path().join(".tirith/runtime/daemon.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        info_a_after["pid"].as_u64().unwrap(),
        pid_a,
        "daemon A was not restarted"
    );

    // B got its own daemon on another port.
    let info_b: Value = serde_json::from_str(&std::fs::read_to_string(&record_b).unwrap()).unwrap();
    let pid_b = info_b["pid"].as_u64().unwrap();
    assert_ne!(pid_b, pid_a, "B has its own daemon");
    assert_ne!(info_b["url"].as_str().unwrap(), url_a, "on its own port");
    let log = std::fs::read_to_string(b.path().join(".tirith/runtime/serve.log")).unwrap();
    assert!(log.contains("serves"), "foreign daemon is logged:\n{log}");
    assert!(
        log.contains("retrying on 127.0.0.1:0"),
        "fallback is logged:\n{log}"
    );
    assert!(
        !log.contains("stopping daemon"),
        "A was never signalled:\n{log}"
    );
}
