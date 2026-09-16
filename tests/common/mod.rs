//! Helpers shared by the integration tests that drive a real daemon.
//!
//! Every function here is used by every test file that declares
//! `mod common;`, because an unused item in a test binary is a
//! `dead_code` warning and CI turns warnings into errors. A helper only
//! some files need lives in its own file next to this one and is pulled
//! in with `#[path]` by those files alone.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use serde_json::Value;
use tirith::client::call_tool;
use tirith::clock::{Clock, ManualClock};
use tirith::server::{ServeOptions, ServerHandle};

/// Options for a daemon on an ephemeral localhost port rooted at `root`,
/// registered nowhere, on the system clock unless `clock` is given.
pub(crate) fn options(root: &Path, clock: Option<Arc<ManualClock>>) -> ServeOptions {
    ServeOptions {
        bind: "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
        repo_root: root.to_path_buf(),
        clock: clock.map(|c| c as Arc<dyn Clock>),
        registry: None,
    }
}

/// One tool call through the client the CLI uses, returning the
/// structured outcome.
pub(crate) async fn call(handle: &ServerHandle, tool: &str, args: Value) -> Value {
    call_tool(&handle.mcp_url(), tool, args).await.unwrap()
}
