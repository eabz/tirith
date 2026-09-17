//! The stdio shim: spawned like an MCP client would spawn it, it must start
//! a daemon for the repository and proxy tool calls to it.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use tirith::registry::Registry;
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
    let reply: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(
        reply["id"], message["id"],
        "a reply to another request: {reply}"
    );
    Some(reply)
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

/// The built binary, kept away from the user's machine-wide state: the
/// daemons it starts launch no menu bar tray and register in a registry
/// inside the test's own directory instead of the user's.
fn tirith(root: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_tirith"));
    command
        .env("TIRITH_NO_TRAY", "1")
        .env("TIRITH_STATE_DIR", state_dir(root));
    command
}

/// The daemon registry directory for the test repository at `root`.
fn state_dir(root: &std::path::Path) -> std::path::PathBuf {
    root.join("machine-state")
}

#[tokio::test]
async fn shim_starts_daemon_and_proxies_tools() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = DaemonGuard::new(dir.path());
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
    // It registered in the registry TIRITH_STATE_DIR names, not the user's,
    // and TIRITH_NO_TRAY kept it from probing the tray lock beside it.
    let registry_file = state_dir(dir.path()).join("daemons.json");
    let registry = Registry::load(&registry_file).unwrap();
    assert!(
        registry.entries().iter().any(|e| e.pid == pid),
        "{:?}",
        registry.entries()
    );
    assert!(
        !state_dir(dir.path()).join("tray.lock").exists(),
        "no tray was launched"
    );
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
    let (second, mut stdin2, mut lines2, _) = shim(dir.path(), "127.0.0.1:0").await;
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

    // Stopped cleanly, the daemon leaves no entry behind.
    drop((child, second));
    drop(daemon);
    let registry = Registry::load(&registry_file).unwrap();
    assert!(registry.entries().is_empty(), "{:?}", registry.entries());
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
    let mut child = tirith(root)
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
    let mut old = tirith(dir.path())
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
    let _daemon_a = tirith(a.path())
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

/// A `tools/call` request line with `id`.
fn tool_call(id: u64, tool: &str, arguments: &Value) -> Value {
    json!({
        "jsonrpc": "2.0", "id": id, "method": "tools/call",
        "params": { "name": tool, "arguments": arguments }
    })
}

/// Writes one message without waiting for a reply.
async fn send(stdin: &mut tokio::process::ChildStdin, message: &Value) {
    stdin
        .write_all(format!("{message}\n").as_bytes())
        .await
        .unwrap();
}

/// `holder` claims `src/a.rs` and `planner` adds the task `freed` on it, so
/// a pull by anyone else has a task to wait for. Uses ids 2 and 3.
async fn hold_a_task(
    stdin: &mut tokio::process::ChildStdin,
    lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
) {
    let held = rpc(
        stdin,
        lines,
        tool_call(
            2,
            "claim",
            &json!({ "agent": "holder", "paths": ["src/a.rs"], "reason": "r" }),
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        held["result"]["structuredContent"]["status"], "ok",
        "{held}"
    );
    let created = rpc(
        stdin,
        lines,
        tool_call(
            3,
            "task_create",
            &json!({ "agent": "planner", "title": "freed", "paths": ["src/a.rs"] }),
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        created["result"]["structuredContent"]["status"], "ok",
        "{created}"
    );
}

/// The two calls that wait, both by `ghost` on what [`hold_a_task`] set up:
/// a pull whose only task is claimed and a claim on the claimed path, with
/// ids 10 and 11.
async fn start_waiting_calls(stdin: &mut tokio::process::ChildStdin) {
    send(
        stdin,
        &tool_call(
            10,
            "task_pull",
            &json!({ "agent": "ghost", "wait_secs": 30 }),
        ),
    )
    .await;
    send(
        stdin,
        &tool_call(
            11,
            "claim",
            &json!({ "agent": "ghost", "paths": ["src/a.rs"], "reason": "r", "wait_secs": 30 }),
        ),
    )
    .await;
    // Long enough for both to reach the daemon and start waiting.
    tokio::time::sleep(Duration::from_millis(700)).await;
}

/// Waits up to 5 s for the daemon's lead log to show the claim's wait
/// ended as `cancelled`, so the test frees the paths only after that.
async fn wait_for_cancelled_claim(root: &std::path::Path) {
    let info = wait_for_daemon(root).await.expect("daemon record");
    let url = format!(
        "{}api/lead?event=claim_waited",
        info["dashboard_url"].as_str().unwrap()
    );
    let mut log = Value::Null;
    for _ in 0..50 {
        log = reqwest::get(&url).await.unwrap().json().await.unwrap();
        if !log["entries"].as_array().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let row = &log["entries"][0];
    assert_eq!(
        row["agent"], "ghost",
        "the waiting claim never ended: {log}"
    );
    assert_eq!(row["details"]["outcome"], "cancelled", "{log}");
}

/// Frees what `ghost` waited for, through the shim on `stdin`, then checks
/// that `ghost` got none of it and that the task is still there for `bob`.
/// Uses ids from 20.
async fn assert_ghost_took_nothing(
    stdin: &mut tokio::process::ChildStdin,
    lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
) {
    let released = rpc(
        stdin,
        lines,
        tool_call(20, "release", &json!({ "agent": "holder" })),
    )
    .await
    .unwrap();
    assert_eq!(
        released["result"]["structuredContent"]["status"], "ok",
        "{released}"
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    let claims = rpc(
        stdin,
        lines,
        tool_call(22, "claims_list", &json!({ "agent": "planner" })),
    )
    .await
    .unwrap();
    assert_eq!(
        claims["result"]["structuredContent"]["count"], 0,
        "{claims}"
    );
    let bob = rpc(
        stdin,
        lines,
        tool_call(23, "task_pull", &json!({ "agent": "bob" })),
    )
    .await
    .unwrap();
    assert_eq!(
        bob["result"]["structuredContent"]["task"]["title"], "freed",
        "the task was still free: {bob}"
    );
}

/// The shim's log lines about cancels it forwarded.
fn forwarded_cancels(root: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(root.join(".tirith/runtime/serve.log"))
        .unwrap_or_default()
        .lines()
        .filter(|line| line.starts_with("[tirith stdio] cancelled request"))
        .map(str::to_owned)
        .collect()
}

#[tokio::test]
async fn shim_forwards_a_cancel_so_a_waiting_call_takes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let _daemon = DaemonGuard::new(dir.path());
    let (_child, mut stdin, mut lines, _) = shim(dir.path(), "127.0.0.1:0").await;
    hold_a_task(&mut stdin, &mut lines).await;

    start_waiting_calls(&mut stdin).await;
    for id in [10, 11] {
        send(
            &mut stdin,
            &json!({
                "jsonrpc": "2.0", "method": "notifications/cancelled",
                "params": { "requestId": id, "reason": "user pressed escape" }
            }),
        )
        .await;
    }
    wait_for_cancelled_claim(dir.path()).await;
    // A cancelled request gets no reply, so the next line answers id 20.
    assert_ghost_took_nothing(&mut stdin, &mut lines).await;
    let cancels = forwarded_cancels(dir.path());
    assert_eq!(cancels.len(), 2, "{cancels:?}");
    assert!(
        cancels
            .iter()
            .all(|l| l.ends_with("the client cancelled it")),
        "{cancels:?}"
    );
}

#[tokio::test]
async fn shim_cancels_waiting_calls_when_its_client_closes_stdin() {
    let dir = tempfile::tempdir().unwrap();
    let _daemon = DaemonGuard::new(dir.path());
    let (mut leaving, mut stdin, _leaving_lines, _) = shim(dir.path(), "127.0.0.1:0").await;
    let (_staying, mut stdin2, mut lines2, _) = shim(dir.path(), "127.0.0.1:0").await;
    hold_a_task(&mut stdin2, &mut lines2).await;

    start_waiting_calls(&mut stdin).await;
    let closed = std::time::Instant::now();
    drop(stdin);
    let exit = tokio::time::timeout(Duration::from_secs(10), leaving.wait())
        .await
        .expect("the shim exits once its client is gone")
        .unwrap();
    assert!(exit.success(), "{exit}");
    assert!(
        closed.elapsed() < Duration::from_secs(5),
        "{:?}",
        closed.elapsed()
    );
    // The shim itself told the daemon, before exiting.
    let cancels = forwarded_cancels(dir.path());
    assert_eq!(cancels.len(), 2, "{cancels:?}");
    assert!(
        cancels
            .iter()
            .all(|l| l.ends_with("the client closed stdin")),
        "{cancels:?}"
    );
    wait_for_cancelled_claim(dir.path()).await;
    assert_ghost_took_nothing(&mut stdin2, &mut lines2).await;
}
