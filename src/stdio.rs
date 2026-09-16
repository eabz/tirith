//! `tirith stdio`: a stdio MCP server that MCP clients spawn per session.
//!
//! Clients such as Claude Code and Cursor start stdio servers themselves,
//! which is what makes Serena "just work". Tirith needs one shared daemon
//! per repository instead, so this shim bridges the two: when a client
//! spawns it, it finds the daemon for the repository (starting one if none
//! is healthy), then proxies every MCP request to it over HTTP. The daemon
//! outlives the session so other agents keep sharing it. See ADR-0006.

use std::fs::OpenOptions;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, Implementation, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerConfig,
};
use rmcp::service::{RequestContext, RoleClient, RoleServer, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::{ErrorData as McpError, ServerHandler, ServiceExt};
use thiserror::Error;

use crate::server::{INSTRUCTIONS, VERSION};
use crate::store::{DaemonInfo, JsonStore, StoreError};

/// How long to wait for a freshly spawned daemon to answer.
pub const START_TIMEOUT: Duration = Duration::from_secs(15);

/// Why the shim could not run.
#[derive(Debug, Error)]
pub enum StdioError {
    /// Reading or writing `.tirith/` failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// The daemon process could not be spawned.
    #[error("cannot start `tirith serve`: {0}")]
    Spawn(#[source] io::Error),
    /// The daemon did not become healthy in time.
    #[error("daemon did not become healthy within {0:?}; see .tirith/runtime/serve.log")]
    Timeout(Duration),
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
/// none answers. Safe to call from several shims at once: only the daemon
/// that actually binds records its address, and the others simply find it.
pub async fn ensure_daemon(root: &Path, bind: &str) -> Result<DaemonInfo, StdioError> {
    let store = JsonStore::new(root);
    store.init()?;
    if let Some(info) = healthy(&store).await {
        return Ok(info);
    }
    tracing::info!(root = %root.display(), bind, "starting tirith daemon");
    spawn_daemon(root, bind, &store)?;
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        if let Some(info) = healthy(&store).await {
            return Ok(info);
        }
        if Instant::now() > deadline {
            return Err(StdioError::Timeout(START_TIMEOUT));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// The recorded daemon, if it answers its health endpoint.
async fn healthy(store: &JsonStore) -> Option<DaemonInfo> {
    let info = store.read_daemon_info().ok().flatten()?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(1))
        .build()
        .ok()?;
    let url = format!("{}api/health", info.dashboard_url);
    let response = client.get(&url).send().await.ok()?;
    response.status().is_success().then_some(info)
}

/// Spawns `tirith serve` detached, with its output appended to
/// `.tirith/runtime/serve.log`. The child is not waited on; it belongs to
/// the repository, not to this session.
fn spawn_daemon(root: &Path, bind: &str, store: &JsonStore) -> Result<(), StdioError> {
    let exe = std::env::current_exe().map_err(StdioError::Spawn)?;
    let log_path = store.dir().join("runtime").join("serve.log");
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
    command.spawn().map_err(StdioError::Spawn)?;
    Ok(())
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

/// Forwards `tools/list` and `tools/call` to the daemon.
struct Proxy {
    daemon: RunningService<RoleClient, ()>,
    instructions: String,
}

fn internal(error: impl std::fmt::Display) -> McpError {
    McpError::internal_error(error.to_string(), None)
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
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let result = self.daemon.call_tool(request).await.map_err(internal)?;
        Ok(result.into())
    }
}

/// Ensures a daemon for `root`, then serves MCP over this process's stdin
/// and stdout until the client closes the pipe.
pub async fn run(root: PathBuf, bind: String) -> Result<(), StdioError> {
    let info = ensure_daemon(&root, &bind).await?;
    let transport = StreamableHttpClientTransport::from_uri(info.url.clone());
    let daemon = ().serve(transport).await.map_err(|e| StdioError::Connect {
        url: info.url.clone(),
        message: e.to_string(),
    })?;
    let instructions = daemon
        .peer_info()
        .and_then(|peer| peer.instructions.clone())
        .unwrap_or_else(|| INSTRUCTIONS.to_owned());
    tracing::info!(url = %info.url, pid = info.pid, "proxying stdio to daemon");
    let proxy = Proxy {
        daemon,
        instructions,
    };
    let running = proxy
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|e| StdioError::Serve(e.to_string()))?;
    running
        .waiting()
        .await
        .map_err(|e| StdioError::Serve(e.to_string()))?;
    Ok(())
}
