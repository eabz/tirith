//! `tirith stdio`: a stdio MCP server that MCP clients spawn per session.
//!
//! Clients such as Claude Code and Cursor start stdio servers themselves,
//! which is what makes Serena "just work". Tirith needs one shared daemon
//! per repository instead, so this shim bridges the two: when a client
//! spawns it, it finds the daemon for the repository (starting one if none
//! is healthy), then proxies every MCP request to it over HTTP. The daemon
//! outlives the session so other agents keep sharing it. See ADR-0006.
//!
//! The shim also keeps the daemon current: when the daemon it finds reports
//! a version other than the shim's own, the shim stops it gracefully and
//! starts a fresh one, so the first session after an install upgrades every
//! agent on the repository. It also refuses a daemon that serves another
//! repository, which a stale record can point at when both use the default
//! port, and starts its own on a free port instead. See ADR-0016.
//!
//! A call can end before the daemon answers it: the client cancels it, or
//! closes stdin because the session is over. The shim cancels the daemon
//! request either way, so a `claim` or `task_pull` waiting with `wait_secs`
//! never grants anything to a client that is gone, and logs each forwarded
//! cancel to `serve.log`. See ADR-0031.

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::{Child, Command, Stdio};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use rmcp::model::{
    CallToolRequest, CallToolRequestParams, CallToolResponse, ClientRequest, Implementation,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, ServerResult,
};
use rmcp::service::{
    Peer, PeerRequestOptions, RequestContext, RoleClient, RoleServer, ServiceError,
};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::{ErrorData as McpError, ServerHandler, ServiceExt};
use serde::Deserialize;
use thiserror::Error;
use tokio::io::{AsyncRead, ReadBuf};
use tokio::sync::{mpsc, watch};

use crate::server::{INSTRUCTIONS, VERSION};
use crate::store::{DaemonInfo, JsonStore, StoreError};

/// How long to wait for a freshly spawned daemon to answer.
pub const START_TIMEOUT: Duration = Duration::from_secs(15);

/// How long to wait for a daemon of another version to exit after it has
/// been told to stop.
pub const STOP_TIMEOUT: Duration = Duration::from_secs(10);

/// The bind that lets the OS pick a free port.
const EPHEMERAL: &str = "127.0.0.1:0";

/// Why the shim could not run.
#[derive(Debug, Error)]
pub enum StdioError {
    /// Reading or writing `.tirith/` failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// The repository root could not be resolved.
    #[error("repository root {path}: {source}")]
    Root {
        /// The root as given.
        path: PathBuf,
        /// What went wrong.
        #[source]
        source: io::Error,
    },
    /// The daemon process could not be spawned.
    #[error("cannot start `tirith serve`: {0}")]
    Spawn(#[source] io::Error),
    /// The daemon did not become healthy in time.
    #[error("daemon did not become healthy within {0:?}; see .tirith/runtime/serve.log")]
    Timeout(Duration),
    /// A daemon of another version could not be signalled to stop.
    #[error("cannot stop daemon {version} (pid {pid}): {source}")]
    Stop {
        /// The version the daemon reported.
        version: String,
        /// Its process id.
        pid: u32,
        /// What went wrong.
        #[source]
        source: io::Error,
    },
    /// A daemon of another version was told to stop but kept answering.
    #[error(
        "daemon {version} (pid {pid}) did not stop within {timeout:?}; stop it by hand and retry"
    )]
    Stale {
        /// The version the daemon reported.
        version: String,
        /// Its process id.
        pid: u32,
        /// How long the shim waited.
        timeout: Duration,
    },
    /// The MCP connection to the daemon failed.
    #[error("cannot connect to daemon at {url}: {message}")]
    Connect {
        /// The daemon's MCP endpoint.
        url: String,
        /// What went wrong.
        message: String,
    },
    /// The stdio server itself failed.
    #[error("stdio server: {0}")]
    Serve(String),
}

/// Returns the healthy daemon for `root`, starting one bound to `bind` if
/// none answers. A daemon of another version is stopped first, so the
/// binary that the client just spawned is always the one serving; a daemon
/// that serves another repository is left alone and a separate one is
/// started on a free port. Safe to call from several shims at once: only
/// the daemon that actually binds records its address, and the others
/// simply find it.
pub async fn ensure_daemon(root: &Path, bind: &str) -> Result<DaemonInfo, StdioError> {
    let root = root.canonicalize().map_err(|source| StdioError::Root {
        path: root.to_path_buf(),
        source,
    })?;
    let store = JsonStore::new(&root);
    store.init()?;
    if let Some(found) = probe(&store).await {
        if found.foreign(&root) {
            tracing::warn!(
                theirs = ?found.root,
                ours = %root.display(),
                "recorded daemon serves another repository; starting our own"
            );
            log_line(
                &store,
                &format!(
                    "[tirith stdio] daemon at {} serves {}, not {}; starting a separate one",
                    found.info.url,
                    found.root.as_deref().unwrap_or(Path::new("?")).display(),
                    root.display()
                ),
            );
        } else if found.current() {
            return Ok(found.info);
        } else {
            tracing::warn!(
                running = %found.running(),
                installed = VERSION,
                pid = found.info.pid,
                "replacing daemon of another version"
            );
            stop_daemon(&store, &found).await?;
        }
    }
    start(&store, &root, bind).await
}

/// Starts a daemon for `root` on `bind` and waits for it to answer. If the
/// process exits before it does, which is what happens when the port is
/// taken by another repository's daemon, one more daemon is started on an
/// ephemeral port; `daemon.json` records the address either way.
async fn start(store: &JsonStore, root: &Path, bind: &str) -> Result<DaemonInfo, StdioError> {
    tracing::info!(root = %root.display(), bind, "starting tirith daemon");
    let mut child = spawn_daemon(root, bind, store)?;
    let mut fallback = bind != EPHEMERAL;
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        if let Some(found) = probe(store).await
            && !found.foreign(root)
        {
            reap(child);
            return Ok(found.info);
        }
        if let Ok(Some(status)) = child.try_wait() {
            if !fallback {
                return Err(StdioError::Timeout(START_TIMEOUT));
            }
            fallback = false;
            tracing::warn!(bind, %status, "daemon exited at once; retrying on an ephemeral port");
            log_line(
                store,
                &format!(
                    "[tirith stdio] daemon on {bind} exited ({status}); retrying on {EPHEMERAL}"
                ),
            );
            child = spawn_daemon(root, EPHEMERAL, store)?;
        }
        if Instant::now() > deadline {
            return Err(StdioError::Timeout(START_TIMEOUT));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Waits on a spawned daemon from a thread so it never lingers as a
/// zombie while this shim is alive. The daemon keeps running; only its
/// eventual exit status is collected.
fn reap(child: Child) {
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
}

/// A daemon that answered its health endpoint.
struct Found {
    /// What `.tirith/runtime/daemon.json` records.
    info: DaemonInfo,
    /// The version `/api/health` reports.
    reported: String,
    /// The repository root `/api/health` reports; absent from daemons
    /// older than 0.1.4.
    root: Option<PathBuf>,
}

impl Found {
    /// Whether both the record and the running process match this binary.
    fn current(&self) -> bool {
        self.info.version == VERSION && self.reported == VERSION
    }

    /// Whether the daemon serves a repository other than `root`. A daemon
    /// that does not report its root is assumed to be ours.
    fn foreign(&self, root: &Path) -> bool {
        self.root.as_deref().is_some_and(|theirs| theirs != root)
    }

    /// The version to name in logs; both sources, when they disagree.
    fn running(&self) -> String {
        if self.reported == self.info.version {
            self.reported.clone()
        } else {
            format!("{} (recorded as {})", self.reported, self.info.version)
        }
    }
}

/// What `/api/health` answers.
#[derive(Deserialize)]
struct Health {
    version: String,
    #[serde(default)]
    root: Option<PathBuf>,
}

/// The recorded daemon, if it answers its health endpoint with a Tirith
/// version. Anything else is treated as absent, never as something to
/// stop: the shim only ever signals a process that proved to be Tirith.
async fn probe(store: &JsonStore) -> Option<Found> {
    let info = store.read_daemon_info().ok().flatten()?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(1))
        .build()
        .ok()?;
    let url = format!("{}api/health", info.dashboard_url);
    let response = client.get(&url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let health: Health = serde_json::from_str(&response.text().await.ok()?).ok()?;
    Some(Found {
        info,
        reported: health.version,
        root: health.root,
    })
}

/// Spawns `tirith serve` detached, with its output appended to
/// `.tirith/runtime/serve.log`. The child is not waited on; it belongs to
/// the repository, not to this session. The handle is returned only so the
/// caller can notice an immediate exit, such as a failed bind.
fn spawn_daemon(root: &Path, bind: &str, store: &JsonStore) -> Result<Child, StdioError> {
    let exe = std::env::current_exe().map_err(StdioError::Spawn)?;
    let log_path = log_path(store);
    let open_log = || OpenOptions::new().create(true).append(true).open(&log_path);
    let stdout = open_log().map_err(StdioError::Spawn)?;
    let stderr = open_log().map_err(StdioError::Spawn)?;
    let mut command = Command::new(exe);
    command
        .arg("serve")
        .arg("--root")
        .arg(root)
        .arg("--bind")
        .arg(bind)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr);
    detach(&mut command);
    command.spawn().map_err(StdioError::Spawn)
}

/// Stops a daemon of another version and waits for it to leave. The stop
/// request is the one `tirith serve` handles gracefully, so the daemon
/// flushes its state and removes `daemon.json` on the way out. The wait is
/// for the process itself, not only its record: a replacement must never
/// start while the old pid is alive, or two daemons end up on one port.
async fn stop_daemon(store: &JsonStore, found: &Found) -> Result<(), StdioError> {
    let running = found.running();
    let pid = found.info.pid;
    log_line(
        store,
        &format!("[tirith stdio] stopping daemon {running} (pid {pid}) to start {VERSION}"),
    );
    interrupt(pid).await.map_err(|source| StdioError::Stop {
        version: running.clone(),
        pid,
        source,
    })?;
    let deadline = Instant::now() + STOP_TIMEOUT;
    while alive(pid).await {
        if Instant::now() > deadline {
            log_line(
                store,
                &format!(
                    "[tirith stdio] daemon {running} (pid {pid}) still alive after {STOP_TIMEOUT:?}; not replacing it"
                ),
            );
            return Err(StdioError::Stale {
                version: running,
                pid,
                timeout: STOP_TIMEOUT,
            });
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    // A daemon that could not shut down gracefully leaves its record
    // behind. Remove it only if it is still that daemon's: a concurrent
    // shim may already have started the replacement.
    let stale = store
        .read_daemon_info()
        .ok()
        .flatten()
        .is_some_and(|info| info.pid == pid);
    if stale {
        store.clear_daemon_info()?;
    }
    log_line(
        store,
        &format!("[tirith stdio] daemon {running} (pid {pid}) stopped"),
    );
    Ok(())
}

/// `.tirith/runtime/serve.log`: the daemon's own output, which the shim
/// also writes its restart decisions to.
fn log_path(store: &JsonStore) -> PathBuf {
    store.dir().join("runtime").join("serve.log")
}

/// Appends one line to the daemon's log, so a restart is visible next to
/// what the daemon printed. The line goes out in one write, so lines
/// logged at the same moment (two forwarded cancels) never interleave.
fn log_line(store: &JsonStore, line: &str) {
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path(store))
    {
        let _ = file.write_all(format!("{line}\n").as_bytes());
    }
}

/// Whether the process `pid` is still running. A zombie counts as gone:
/// it has exited and only waits for a parent that never reaped it.
#[cfg(unix)]
async fn alive(pid: u32) -> bool {
    let output = tokio::process::Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .await;
    let Ok(output) = output else {
        return false;
    };
    let stat = String::from_utf8_lossy(&output.stdout);
    let stat = stat.trim();
    !stat.is_empty() && !stat.starts_with('Z')
}

/// Whether the process `pid` is still running.
#[cfg(windows)]
async fn alive(pid: u32) -> bool {
    let output = tokio::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .output()
        .await;
    output.is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
}

/// Asks the daemon at `pid` to shut down.
#[cfg(unix)]
async fn interrupt(pid: u32) -> io::Result<()> {
    // SIGINT is what `tirith serve` waits for (tokio's ctrl_c), so this is
    // the same clean shutdown as pressing ctrl-c in its terminal.
    run_checked("kill", &["-INT", &pid.to_string()]).await
}

/// Asks the daemon at `pid` to shut down.
#[cfg(windows)]
async fn interrupt(pid: u32) -> io::Result<()> {
    // A detached process has no console to receive ctrl-c, so the daemon
    // is terminated. Its state was written through as it changed, so
    // nothing is lost beyond the last in-flight write.
    run_checked("taskkill", &["/PID", &pid.to_string(), "/T", "/F"]).await
}

/// Runs `program` and fails unless it exits successfully.
async fn run_checked(program: &str, args: &[&str]) -> io::Result<()> {
    let status = tokio::process::Command::new(program)
        .args(args)
        .status()
        .await?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("{program} exited with {status}")))
    }
}

#[cfg(unix)]
fn detach(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    // A new process group so the daemon survives the client killing ours.
    command.process_group(0);
}

#[cfg(windows)]
fn detach(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    command.creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
}

/// How long a shim whose client is gone gives in-flight calls to cancel
/// their daemon requests, and then its daemon session to close.
const CANCEL_TIMEOUT: Duration = Duration::from_secs(3);

/// Forwards `tools/list` and `tools/call` to the daemon, and passes on
/// every way a call can end early (ADR-0031): a cancel from the client,
/// and the client closing stdin, both cancel the daemon request, so a call
/// that waits (`claim` or `task_pull` with `wait_secs`) never grants
/// anything to a client that is gone.
struct Proxy {
    daemon: Peer<RoleClient>,
    instructions: String,
    /// Where forwarded cancels are logged.
    store: JsonStore,
    /// `true` once the client has closed stdin. The sender lives in the
    /// transport, so a dropped sender means the same.
    closed: watch::Receiver<bool>,
    /// A drop guard, never sent on. Every in-flight call holds the proxy
    /// through rmcp's `Arc`, so the channel closes when the last call
    /// finishes, and [`run`] waits for that before the process exits.
    _calls: mpsc::Sender<()>,
}

fn internal(error: impl std::fmt::Display) -> McpError {
    McpError::internal_error(error.to_string(), None)
}

/// The daemon's answer to a `tools/call`, passed through as it came.
fn tool_response(result: ServerResult) -> Result<CallToolResponse, McpError> {
    match result {
        ServerResult::CallToolResult(result) => Ok(CallToolResponse::Complete(result)),
        ServerResult::InputRequiredResult(result) => Ok(CallToolResponse::InputRequired(result)),
        ServerResult::CreateTaskResult(result) => Ok(CallToolResponse::Task(result)),
        _ => Err(internal(ServiceError::UnexpectedResponse)),
    }
}

impl ServerHandler for Proxy {
    fn get_info(&self) -> ServerConfig {
        let mut config = ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(self.instructions.clone());
        config.server_info = Implementation::new("tirith", VERSION);
        config
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let tools = self.daemon.list_all_tools().await.map_err(internal)?;
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let request = ClientRequest::CallToolRequest(CallToolRequest::new(request));
        let mut handle = self
            .daemon
            .send_cancellable_request(request, PeerRequestOptions::no_options())
            .await
            .map_err(internal)?;
        let mut closed = self.closed.clone();
        let reason = tokio::select! {
            biased;
            response = &mut handle.rx => {
                let result = response
                    .unwrap_or(Err(ServiceError::TransportClosed))
                    .map_err(internal)?;
                return tool_response(result);
            }
            () = context.ct.cancelled() => "the client cancelled it",
            // An error here means the transport is gone, so the client is too.
            _ = closed.wait_for(|closed| *closed) => "the client closed stdin",
        };
        let id = handle.id.clone();
        let sent = handle.cancel(Some(reason.to_owned())).await;
        let line = match sent {
            Ok(()) => format!("[tirith stdio] cancelled request {id} at the daemon: {reason}"),
            Err(error) => {
                format!(
                    "[tirith stdio] could not cancel request {id} at the daemon ({reason}): {error}"
                )
            }
        };
        let store = self.store.clone();
        let _ = tokio::task::spawn_blocking(move || log_line(&store, &line)).await;
        // rmcp drops this reply for a cancelled request; after a closed
        // stdin nobody reads it either.
        Err(McpError::internal_error(
            format!("cancelled: {reason}"),
            None,
        ))
    }
}

/// A reader that marks `closed` once it reaches end of file or fails, so
/// the proxy learns that its client has gone before rmcp's serve loop does.
struct EofWatch<R> {
    inner: R,
    closed: watch::Sender<bool>,
}

impl<R: AsyncRead + Unpin> AsyncRead for EofWatch<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let polled = Pin::new(&mut self.inner).poll_read(cx, buf);
        if let Poll::Ready(result) = &polled
            && (result.is_err() || (buf.filled().len() == before && buf.remaining() > 0))
        {
            self.closed.send_replace(true);
        }
        polled
    }
}

/// Ensures a daemon for `root`, then serves MCP over this process's stdin
/// and stdout until the client closes the pipe. Calls still waiting then
/// are cancelled at the daemon, and the daemon session is closed, before
/// this returns.
pub async fn run(root: PathBuf, bind: String) -> Result<(), StdioError> {
    let info = ensure_daemon(&root, &bind).await?;
    let transport = StreamableHttpClientTransport::from_uri(info.url.clone());
    let mut daemon = ().serve(transport).await.map_err(|e| StdioError::Connect {
        url: info.url.clone(),
        message: e.to_string(),
    })?;
    let instructions = daemon
        .peer_info()
        .and_then(|peer| peer.instructions.clone())
        .unwrap_or_else(|| INSTRUCTIONS.to_owned());
    tracing::info!(url = %info.url, pid = info.pid, "proxying stdio to daemon");
    let (closed_tx, closed) = watch::channel(false);
    let (calls, mut calls_done) = mpsc::channel(1);
    let proxy = Proxy {
        daemon: daemon.peer().clone(),
        instructions,
        store: JsonStore::new(&root),
        closed,
        _calls: calls,
    };
    let stdin = EofWatch {
        inner: tokio::io::stdin(),
        closed: closed_tx,
    };
    let running = proxy
        .serve((stdin, tokio::io::stdout()))
        .await
        .map_err(|e| StdioError::Serve(e.to_string()))?;
    let served = running
        .waiting()
        .await
        .map_err(|e| StdioError::Serve(e.to_string()));
    // The client is gone. Calls still in flight are cancelling their daemon
    // requests; the last one to finish drops the proxy and closes `calls`.
    let _ = tokio::time::timeout(CANCEL_TIMEOUT, calls_done.recv()).await;
    // Ending the session also ends, at the daemon, anything still tied to it.
    let _ = daemon.close_with_timeout(CANCEL_TIMEOUT).await;
    served.map(|_| ())
}

#[cfg(test)]
mod tests {
    use tokio::io::AsyncReadExt;

    use super::*;

    #[tokio::test]
    async fn eof_watch_marks_closed_only_at_end_of_file() {
        let (closed_tx, closed) = watch::channel(false);
        let mut reader = EofWatch {
            inner: &b"hello"[..],
            closed: closed_tx,
        };
        let mut buf = [0_u8; 5];
        reader.read_exact(&mut buf).await.unwrap();
        assert!(!*closed.borrow(), "data read is not the end");
        assert_eq!(reader.read(&mut buf).await.unwrap(), 0);
        assert!(*closed.borrow(), "end of file marks closed");
    }

    #[tokio::test]
    async fn an_empty_buffer_is_not_end_of_file() {
        let (closed_tx, closed) = watch::channel(false);
        let mut reader = EofWatch {
            inner: &b"hello"[..],
            closed: closed_tx,
        };
        assert_eq!(reader.read(&mut []).await.unwrap(), 0);
        assert!(!*closed.borrow());
    }
}
