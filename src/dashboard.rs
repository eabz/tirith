//! A read-only HTTP dashboard served next to the MCP endpoint.
//!
//! `GET /` returns a single self-contained page that polls
//! `GET /api/state` every two seconds. There is nothing to build or
//! install; the page is embedded in the binary.

use std::sync::Arc;

use axum::Router;
use axum::extract::State as Extract;
use axum::response::{Html, Json};
use axum::routing::get;
use serde_json::{Value, json};

use crate::server::VERSION;
use crate::state::State;
use crate::store::Persister;

/// What the dashboard handlers need.
#[derive(Debug, Clone)]
pub struct DashboardContext {
    /// The shared state.
    pub state: Arc<State>,
    /// For reporting persistence failures.
    pub persister: Arc<Persister>,
    /// The MCP endpoint, shown so clients can be pointed at it.
    pub mcp_url: String,
    /// The repository being coordinated.
    pub repo_root: String,
}

/// Routes: `/`, `/api/state`, `/api/health`.
pub fn router(context: DashboardContext) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/api/state", get(api_state))
        .route("/api/health", get(health))
        .with_state(context)
}

async fn index() -> Html<&'static str> {
    Html(include_str!("dashboard.html"))
}

async fn health() -> Json<Value> {
    Json(json!({ "ok": true, "version": VERSION }))
}

async fn api_state(Extract(context): Extract<DashboardContext>) -> Json<Value> {
    let report = context.state.status(None);
    let snapshot = context.state.snapshot();
    Json(json!({
        "server": {
            "version": VERSION,
            "mcp_url": context.mcp_url,
            "repo_root": context.repo_root,
            "started_at": report.started_at,
            "now": report.now,
            "uptime_secs": report.uptime_secs,
            "seq": report.seq,
            "persist_error": context.persister.last_error(),
        },
        "counts": {
            "claims": report.claims,
            "tasks_open": report.tasks_open,
            "tasks_done": report.tasks_done,
            "contracts": report.contracts,
            "notices": report.notices,
            "decisions": report.decisions,
        },
        "agents": report.agents,
        "claims": snapshot.claims,
        "tasks": snapshot.tasks,
        "contracts": snapshot.contracts,
        "notices": snapshot.notices,
        "decisions": snapshot.decisions,
    }))
}
