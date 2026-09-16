//! A read-only HTTP dashboard served next to the MCP endpoint.
//!
//! `GET /` returns a single self-contained page that polls
//! `GET /api/state` every two seconds. Memory notes are sent as
//! excerpts, never bodies, so the payload stays small at any note count.
//! `GET /logo.png` serves the logo the page shows in its header and uses
//! as its favicon. There is nothing to build or install; the page and the
//! logo are embedded in the binary.
//!
//! `/api/state` is served from a cached, pre-serialized view that a
//! background task rebuilds whenever the persister writes something, so
//! polling browsers never take the [`State`] lock. The `ETag` is the
//! state's sequence number and `If-None-Match` gets a `304`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use axum::Router;
use axum::extract::State as Extract;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Json, Response};
use axum::routing::get;
use serde_json::{Value, json};

use crate::memory::MemoryNote;
use crate::server::VERSION;
use crate::state::State;
use crate::store::Persister;

/// Characters of body shown per note on the dashboard.
const EXCERPT_CHARS: usize = 160;
/// Newest notices and decisions included in the view; the counts stay
/// exact so the page can say how many are not shown.
const LOG_ROWS: usize = 200;

/// A note without its body: what the dashboard shows per row.
fn memory_row(note: &MemoryNote) -> Value {
    json!({
        "permalink": note.permalink.as_str(),
        "title": note.title,
        "kind": note.kind.as_str(),
        "paths": note.paths,
        "tags": note.tags,
        "author": note.author,
        "updated_by": note.updated_by,
        "created_at": note.created_at,
        "updated_at": note.updated_at,
        "excerpt": note.excerpt(EXCERPT_CHARS),
    })
}

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

/// The serialized `/api/state` body for one rebuild of the cache.
#[derive(Debug)]
struct View {
    /// `"<seq>.<generation>"`: the state's sequence number plus a counter
    /// that grows on every rebuild, so a rebuild that changed something
    /// without a new sequence number (a persist error appearing, a load
    /// error) still gets a fresh tag and is never hidden behind a `304`.
    etag: String,
    bytes: Vec<u8>,
}

/// Handler state: the context plus the cached view.
#[derive(Debug, Clone)]
struct Shared {
    context: DashboardContext,
    view: Arc<RwLock<Arc<View>>>,
    /// How many times the view has been rebuilt since start.
    generation: Arc<AtomicU64>,
}

impl Shared {
    fn new(context: DashboardContext) -> Self {
        let view = Arc::new(build_view(&context, 0));
        Self {
            context,
            view: Arc::new(RwLock::new(view)),
            generation: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Rebuilds the view from the current state. Called by the refresh
    /// task after every write attempt, never by a request handler.
    fn refresh(&self) {
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let view = Arc::new(build_view(&self.context, generation));
        *self
            .view
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = view;
    }

    fn current(&self) -> Arc<View> {
        Arc::clone(
            &self
                .view
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }
}

/// Routes: `/`, `/logo.png`, `/api/state`, `/api/health`.
///
/// Must be called on a Tokio runtime: it spawns the task that refreshes
/// the cached view after each write.
pub fn router(context: DashboardContext) -> Router {
    let shared = Shared::new(context);
    let refresher = shared.clone();
    let mut attempts = shared.context.persister.attempts();
    tokio::spawn(async move {
        while attempts.changed().await.is_ok() {
            refresher.refresh();
        }
    });
    Router::new()
        .route("/", get(index))
        .route("/logo.png", get(logo))
        .route("/api/state", get(api_state))
        .route("/api/health", get(health))
        .with_state(shared)
}

async fn index() -> Html<&'static str> {
    Html(include_str!("dashboard.html"))
}

/// The logo, embedded so the dashboard needs no files on disk.
async fn logo() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        include_bytes!("dashboard-logo.png").as_slice(),
    )
}

async fn health(Extract(shared): Extract<Shared>) -> Json<Value> {
    let context = &shared.context;
    // `root` lets a stdio shim tell this daemon apart from one serving
    // another repository on the same port. See ADR-0016.
    // `ok` stays true when files were skipped at load: the daemon is up,
    // and `load_errors` tells a human what to fix.
    Json(json!({
        "ok": true,
        "version": VERSION,
        "root": context.repo_root,
        "load_errors": context.state.load_errors(),
    }))
}

/// Serializes everything the page shows. Takes the state lock twice
/// (status, then snapshot) and is the only place the dashboard does so.
fn build_view(context: &DashboardContext, generation: u64) -> View {
    let report = context.state.status(None);
    let snapshot = context.state.snapshot();
    let tail = |len: usize| len.saturating_sub(LOG_ROWS);
    let body = json!({
        "server": {
            "version": VERSION,
            "mcp_url": context.mcp_url,
            "repo_root": context.repo_root,
            "started_at": report.started_at,
            "now": report.now,
            "uptime_secs": report.uptime_secs,
            "seq": report.seq,
            "persist_error": context.persister.last_error(),
            "load_errors": report.load_errors,
        },
        "counts": {
            "claims": report.claims,
            "tasks_open": report.tasks_open,
            "tasks_done": report.tasks_done,
            "contracts": report.contracts,
            "notices": report.notices,
            "decisions": report.decisions,
            "memory": snapshot.memory.len(),
        },
        "agents": report.agents,
        "claims": snapshot.claims,
        "tasks": snapshot.tasks,
        "contracts": snapshot.contracts,
        "notices": &snapshot.notices[tail(snapshot.notices.len())..],
        "decisions": &snapshot.decisions[tail(snapshot.decisions.len())..],
        "memory": snapshot.memory.iter().map(memory_row).collect::<Vec<_>>(),
    });
    View {
        etag: format!("\"{}.{generation}\"", report.seq),
        bytes: serde_json::to_vec(&body).unwrap_or_else(|_| b"{}".to_vec()),
    }
}

/// The cached view, or `304 Not Modified` when the client already has it.
async fn api_state(Extract(shared): Extract<Shared>, headers: HeaderMap) -> Response {
    let view = shared.current();
    let matches = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|tag| tag.trim() == view.etag));
    let mut response = if matches {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        (
            [(header::CONTENT_TYPE, "application/json")],
            view.bytes.clone(),
        )
            .into_response()
    };
    if let Ok(etag) = header::HeaderValue::from_str(&view.etag) {
        response.headers_mut().insert(header::ETAG, etag);
    }
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-cache"),
    );
    response
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::*;
    use crate::clock::ManualClock;
    use crate::notices::{NewNotice, NoticeKind};
    use crate::state::Snapshot;
    use crate::store::JsonStore;
    use crate::tasks::{NewTask, TaskStatus};
    use crate::types::{AgentId, RepoPath};

    fn shared() -> (Shared, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualClock::new(
            Utc.with_ymd_and_hms(2026, 9, 16, 12, 0, 0).unwrap(),
        ));
        let state = Arc::new(State::new(clock, Snapshot::default()));
        let store = Arc::new(JsonStore::new(dir.path()));
        let persister = Persister::spawn(store, Arc::clone(&state));
        let context = DashboardContext {
            state,
            persister,
            mcp_url: "http://127.0.0.1:1/mcp".into(),
            repo_root: dir.path().display().to_string(),
        };
        (Shared::new(context), dir)
    }

    fn body(response: Response) -> Value {
        let bytes = futures_body(response);
        serde_json::from_slice(&bytes).unwrap()
    }

    fn futures_body(response: Response) -> Vec<u8> {
        let rt = tokio::runtime::Handle::current();
        let (_, body) = response.into_parts();
        rt.block_on(async {
            axum::body::to_bytes(body, usize::MAX)
                .await
                .unwrap()
                .to_vec()
        })
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn state_is_served_from_the_cache_until_refreshed() {
        let (shared, _dir) = shared();
        let first = api_state(Extract(shared.clone()), HeaderMap::new()).await;
        let etag = first.headers()[header::ETAG].to_str().unwrap().to_owned();
        assert_eq!(etag, "\"0.0\"", "seq 0, generation 0");
        let first = tokio::task::block_in_place(|| body(first));
        assert_eq!(first["counts"]["claims"], 0);

        // Mutating the state does not change what is served until a
        // refresh, which proves no request reads the state directly.
        shared
            .context
            .state
            .claim(
                AgentId::new("alice").unwrap(),
                vec![RepoPath::new("src").unwrap()],
                "r".into(),
                None,
            )
            .unwrap();
        let stale = api_state(Extract(shared.clone()), HeaderMap::new()).await;
        assert_eq!(
            tokio::task::block_in_place(|| body(stale))["counts"]["claims"],
            0
        );

        let mut headers = HeaderMap::new();
        headers.insert(header::IF_NONE_MATCH, etag.parse().unwrap());
        let unchanged = api_state(Extract(shared.clone()), headers.clone()).await;
        assert_eq!(unchanged.status(), StatusCode::NOT_MODIFIED);

        shared.context.persister.flush().await.unwrap();
        shared.refresh();
        let fresh = api_state(Extract(shared.clone()), headers).await;
        assert_eq!(fresh.status(), StatusCode::OK, "etag changed with seq");
        assert_ne!(fresh.headers()[header::ETAG], etag.as_str());
        assert_eq!(
            tokio::task::block_in_place(|| body(fresh))["counts"]["claims"],
            1
        );
        shared.context.persister.stop();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn task_changes_move_the_counts_and_the_etag() {
        let (shared, _dir) = shared();
        let alice = AgentId::new("alice").unwrap();
        let task = shared
            .context
            .state
            .task_create(
                alice.clone(),
                NewTask {
                    title: "Write the summary".into(),
                    description: String::new(),
                    priority: 0,
                    depends_on: vec![],
                    paths: vec![],
                },
            )
            .unwrap();
        shared.context.persister.flush().await.unwrap();
        shared.refresh();
        let opened = api_state(Extract(shared.clone()), HeaderMap::new()).await;
        let etag_open = opened.headers()[header::ETAG].to_str().unwrap().to_owned();
        let opened = tokio::task::block_in_place(|| body(opened));
        assert_eq!(opened["counts"]["tasks_open"], 1);
        assert_eq!(opened["counts"]["tasks_done"], 0);

        shared
            .context
            .state
            .task_update(alice, task.id, TaskStatus::Done, None, false)
            .unwrap();
        shared.context.persister.flush().await.unwrap();
        shared.refresh();
        let mut headers = HeaderMap::new();
        headers.insert(header::IF_NONE_MATCH, etag_open.parse().unwrap());
        let done = api_state(Extract(shared.clone()), headers).await;
        assert_eq!(
            done.status(),
            StatusCode::OK,
            "a task change is never a 304"
        );
        let etag_done = done.headers()[header::ETAG].to_str().unwrap().to_owned();
        assert_ne!(etag_done, etag_open);
        let done = tokio::task::block_in_place(|| body(done));
        assert_eq!(done["counts"]["tasks_open"], 0);
        assert_eq!(done["counts"]["tasks_done"], 1);

        // A rebuild without a new sequence number still gets a new tag, so
        // a view that changed for another reason (a persist error
        // appearing) is never hidden behind the client's If-None-Match.
        shared.refresh();
        let again = api_state(Extract(shared.clone()), HeaderMap::new()).await;
        assert_ne!(again.headers()[header::ETAG].to_str().unwrap(), etag_done);
        shared.context.persister.stop();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn logs_are_bounded_to_the_newest_rows_with_exact_counts() {
        let (shared, _dir) = shared();
        for i in 0..(LOG_ROWS + 5) {
            shared
                .context
                .state
                .notice_publish(
                    AgentId::new("alice").unwrap(),
                    NewNotice {
                        kind: NoticeKind::Rename,
                        summary: format!("n{i}"),
                        from: None,
                        to: None,
                        affected_paths: vec![],
                        contract_id: None,
                    },
                )
                .unwrap();
        }
        shared.refresh();
        let view = api_state(Extract(shared.clone()), HeaderMap::new()).await;
        let view = tokio::task::block_in_place(|| body(view));
        assert_eq!(view["counts"]["notices"], LOG_ROWS + 5);
        assert_eq!(view["notices"].as_array().unwrap().len(), LOG_ROWS);
        assert_eq!(
            view["notices"][0]["summary"], "n5",
            "oldest shown is the 6th"
        );
        shared.context.persister.stop();
    }
}
