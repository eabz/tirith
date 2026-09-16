//! The MCP tool surface and the daemon.
//!
//! Every tool maps to exactly one [`State`] call and formats the outcome
//! as JSON with a top-level `status`: `ok`, `conflict`, `not_found`,
//! `none`, or `invalid`. Domain outcomes are never MCP-level errors, so
//! clients can branch on `status` instead of parsing prose.
//!
//! [`start`] wires the tools, the dashboard, and JSON persistence into one
//! HTTP server bound to localhost. See ADR-0002.

use std::convert::identity;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, SecondsFormat, Utc};
use rmcp::RoleServer;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ContentBlock, Implementation, ListToolsResult, PaginatedRequestParams,
    ServerCapabilities, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData as McpError, ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::claims::ClaimError;
use crate::clock::{Clock, SystemClock};
use crate::contracts::{ContractError, ContractKind, NewContract};
use crate::dashboard::{self, DashboardContext};
use crate::decisions::{DecisionError, NewDecision};
use crate::memory::{
    DEFAULT_SEARCH_LIMIT, MAX_CONTEXT_DEPTH, MAX_SEARCH_LIMIT, MemoryError, MemoryKind, MemoryNote,
    MemorySearch, NewMemory, Permalink,
};
use crate::notices::{NewNotice, NoticeError, NoticeFilter, NoticeKind};
use crate::state::State;
use crate::store::{DaemonInfo, JsonStore, Persister, StoreError};
use crate::tasks::{NewTask, TaskError, TaskStatus};
use crate::types::{
    AgentId, ClaimId, ContractId, DecisionId, IdError, NoticeId, Page, PathError, PrefixError,
    RepoPath, TaskId, clamp_limit, parse_paths,
};

/// Crate version reported to clients and the dashboard.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Default bind address.
pub const DEFAULT_BIND: &str = "127.0.0.1:7477";

/// How long [`ServerHandle::shutdown`] waits for open connections before
/// closing them. Idle SSE streams never finish on their own.
pub const SHUTDOWN_DEADLINE: std::time::Duration = std::time::Duration::from_secs(2);

/// Instructions sent to every MCP client on initialize.
pub const INSTRUCTIONS: &str = "Tirith coordinates parallel coding agents working in one repository. \
Pick a stable, unique `agent` name and pass it to every call. Before editing files, call `claim` \
with the paths you will touch; if the status is `conflict`, do not edit those paths. Read `notice_list` \
and `contract_list` for the paths you touch before acting. Publish a `contract_publish` before \
implementing an interface another agent will consume, and `notice_publish` for renames or signature \
changes that affect other files. Call `release` when done. Any call renews your claim leases.";

// ---------------------------------------------------------------------------
// Tool inputs. Doc comments become schema descriptions.
// ---------------------------------------------------------------------------

/// Input for `claim`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ClaimInput {
    /// Your stable agent name.
    pub agent: String,
    /// Repo-relative files or directories. A directory covers everything beneath it.
    pub paths: Vec<String>,
    /// Why you need these paths; shown to any agent that is refused.
    pub reason: String,
    /// Lease length in seconds. Default 600, max 3600.
    #[serde(default)]
    pub ttl_secs: Option<u64>,
}

/// Input for `release`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReleaseInput {
    /// Your stable agent name.
    pub agent: String,
    /// Paths to release. Omit to release everything you hold.
    #[serde(default)]
    pub paths: Option<Vec<String>>,
}

/// Input for tools that only need the caller's name.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AgentInput {
    /// Your stable agent name.
    pub agent: String,
}

/// Input for `claims_list`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ClaimsListInput {
    /// Your stable agent name.
    pub agent: String,
    /// Also include claims overlapping this path, whoever holds them.
    #[serde(default)]
    pub path: Option<String>,
    /// List every live claim, not just yours and those overlapping `path`.
    #[serde(default)]
    pub all: Option<bool>,
    /// Rows to return (default 20, max 200), newest first.
    #[serde(default)]
    pub limit: Option<usize>,
    /// Only claims older than this cursor: an RFC 3339 timestamp or a
    /// previous response's `next_before`.
    #[serde(default)]
    pub before: Option<String>,
}

/// Input for `task_create`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TaskCreateInput {
    /// Your stable agent name.
    pub agent: String,
    /// Short imperative title.
    pub title: String,
    /// Longer description.
    #[serde(default)]
    pub description: Option<String>,
    /// Higher pulls first. Default 0.
    #[serde(default)]
    pub priority: Option<i32>,
    /// Task ids that must be done first.
    #[serde(default)]
    pub depends_on: Option<Vec<String>>,
    /// Paths the task will likely touch.
    #[serde(default)]
    pub paths: Option<Vec<String>>,
}

/// Input for `task_update`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TaskUpdateInput {
    /// Your stable agent name.
    pub agent: String,
    /// The task id.
    pub task_id: String,
    /// One of `todo`, `in_progress`, `blocked`, `done`.
    pub status: String,
    /// A note to append. For `blocked`, this is the reason.
    #[serde(default)]
    pub note: Option<String>,
    /// Take or close a task another agent has in progress.
    #[serde(default)]
    pub force: Option<bool>,
}

/// Input for `task_list`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TaskListInput {
    /// Your stable agent name.
    pub agent: String,
    /// Only tasks with this status.
    #[serde(default)]
    pub status: Option<String>,
    /// Only tasks owned by this agent.
    #[serde(default)]
    pub owner: Option<String>,
    /// Rows to return (default 20, max 200), most recently updated first.
    #[serde(default)]
    pub limit: Option<usize>,
    /// Only tasks updated before this cursor: an RFC 3339 timestamp or a
    /// previous response's `next_before`.
    #[serde(default)]
    pub before: Option<String>,
}

/// Input for `contract_publish`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ContractPublishInput {
    /// Your stable agent name.
    pub agent: String,
    /// Unique contract name, for example `POST /api/sessions` or `State::claim`.
    pub name: String,
    /// One of `http`, `function`, `type`, `event`, `cli`, `other`.
    pub kind: String,
    /// The interface shape as JSON: signatures, request and response types, errors.
    pub shape: Value,
    /// Paths expected to depend on this contract.
    #[serde(default)]
    pub consumers: Option<Vec<String>>,
    /// Free-text notes.
    #[serde(default)]
    pub notes: Option<String>,
    /// Refuse unless the contract is at this version now (0 = absent).
    #[serde(default)]
    pub expected_version: Option<u32>,
}

/// Input for `contract_get`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ContractGetInput {
    /// Your stable agent name.
    pub agent: String,
    /// Contract name or id.
    pub name: String,
}

/// Input for `contract_list`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ContractListInput {
    /// Your stable agent name.
    pub agent: String,
    /// Only contracts whose consumers overlap this path.
    #[serde(default)]
    pub path: Option<String>,
    /// Only contracts of this kind.
    #[serde(default)]
    pub kind: Option<String>,
    /// Rows to return (default 20, max 200), most recently published first.
    #[serde(default)]
    pub limit: Option<usize>,
    /// Only contracts published before this cursor: an RFC 3339 timestamp
    /// or a previous response's `next_before`.
    #[serde(default)]
    pub before: Option<String>,
}

/// Input for `notice_publish`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct NoticePublishInput {
    /// Your stable agent name.
    pub agent: String,
    /// One of `rename`, `signature`, `removed`, `moved`, `behavior`.
    pub kind: String,
    /// One-line summary, for example `renamed session_id to token`.
    pub summary: String,
    /// The old name, location, or shape.
    #[serde(default)]
    pub from: Option<String>,
    /// The new name, location, or shape.
    #[serde(default)]
    pub to: Option<String>,
    /// Paths whose code must react.
    pub affected_paths: Vec<String>,
    /// Related contract id, if any.
    #[serde(default)]
    pub contract_id: Option<String>,
}

/// Input for `notice_list`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct NoticeListInput {
    /// Your stable agent name.
    pub agent: String,
    /// Only notices affecting this path.
    #[serde(default)]
    pub path: Option<String>,
    /// Only notices published at or after this RFC 3339 timestamp.
    #[serde(default)]
    pub since: Option<String>,
    /// Only notices you have not published or acknowledged. Without
    /// `path` or `all`, this is scoped to the paths you currently hold.
    #[serde(default)]
    pub unread: Option<bool>,
    /// With `unread`, look beyond the paths you hold.
    #[serde(default)]
    pub all: Option<bool>,
    /// Rows to return (default 20, max 200), newest first.
    #[serde(default)]
    pub limit: Option<usize>,
    /// Only notices published before this cursor: an RFC 3339 timestamp
    /// or a previous response's `next_before`.
    #[serde(default)]
    pub before: Option<String>,
}

/// Input for `notice_ack`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct NoticeAckInput {
    /// Your stable agent name.
    pub agent: String,
    /// The notice id.
    pub notice_id: String,
}

/// Input for `decision_record`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DecisionRecordInput {
    /// Your stable agent name.
    pub agent: String,
    /// Short title.
    pub title: String,
    /// What was decided.
    pub decision: String,
    /// Why.
    #[serde(default)]
    pub rationale: Option<String>,
    /// What else was considered.
    #[serde(default)]
    pub alternatives: Option<Vec<String>>,
    /// Paths the decision constrains.
    #[serde(default)]
    pub affects_paths: Option<Vec<String>>,
}

/// Input for `decision_list`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DecisionListInput {
    /// Your stable agent name.
    pub agent: String,
    /// Only decisions affecting this path.
    #[serde(default)]
    pub path: Option<String>,
    /// Case-insensitive text to search for.
    #[serde(default)]
    pub query: Option<String>,
    /// Rows to return (default 20, max 200), newest first.
    #[serde(default)]
    pub limit: Option<usize>,
    /// Only decisions recorded before this cursor: an RFC 3339 timestamp
    /// or a previous response's `next_before`.
    #[serde(default)]
    pub before: Option<String>,
}

/// Input for `status`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct StatusInput {
    /// Your stable agent name, optional here.
    #[serde(default)]
    pub agent: Option<String>,
    /// Also list active agents (at most 50) with what they hold.
    #[serde(default)]
    pub verbose: Option<bool>,
}

/// Input for `memory_write`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemoryWriteInput {
    /// Your stable agent name.
    pub agent: String,
    /// Short title. Becomes the permalink the first time.
    pub title: String,
    /// The note itself, as Markdown.
    pub body: String,
    /// One of `fact`, `lesson`, `gotcha`, `handoff`, `research`, `note`.
    #[serde(default)]
    pub kind: Option<String>,
    /// Repo-relative paths this note is about.
    #[serde(default)]
    pub paths: Option<Vec<String>>,
    /// Tags for filtering.
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    /// Overwrite this note instead of matching on the title.
    #[serde(default)]
    pub permalink: Option<String>,
    /// RFC 3339 `updated_at` from a read; refused if the note changed since.
    #[serde(default)]
    pub if_updated_at: Option<String>,
}

/// Input for `memory_delete`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemoryDeleteInput {
    /// Your stable agent name.
    pub agent: String,
    /// A permalink, an id, or an exact title.
    pub name: String,
}

/// Input for `memory_read`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemoryReadInput {
    /// Your stable agent name.
    pub agent: String,
    /// A permalink, an id, or an exact title.
    pub name: String,
    /// How many relation hops of related notes to include, 0 to 3.
    #[serde(default)]
    pub depth: Option<usize>,
}

/// Input for `memory_search`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct MemorySearchInput {
    /// Your stable agent name.
    pub agent: String,
    /// Free text. Omit for recent activity.
    #[serde(default)]
    pub query: Option<String>,
    /// Only notes whose paths overlap this one.
    #[serde(default)]
    pub path: Option<String>,
    /// Only notes of this kind.
    #[serde(default)]
    pub kind: Option<String>,
    /// Only notes carrying this tag.
    #[serde(default)]
    pub tag: Option<String>,
    /// Only notes updated at or after this RFC 3339 timestamp.
    #[serde(default)]
    pub since: Option<String>,
    /// How many to return. Default 20, max 100.
    #[serde(default)]
    pub limit: Option<usize>,
}

// ---------------------------------------------------------------------------
// Outcome helpers.
// ---------------------------------------------------------------------------

type Outcome = Result<Value, Value>;

fn with_status(status: &str, fields: Value) -> Value {
    let mut map = match fields {
        Value::Object(map) => map,
        other => {
            let mut map = serde_json::Map::new();
            map.insert("result".into(), other);
            map
        }
    };
    map.insert("status".into(), Value::String(status.to_owned()));
    Value::Object(map)
}

fn ok(fields: Value) -> Value {
    with_status("ok", fields)
}

// Taking `impl ToString` by value keeps `.map_err(invalid)` ergonomic at every call site.
#[allow(clippy::needless_pass_by_value)]
fn invalid(message: impl ToString) -> Value {
    with_status("invalid", json!({ "message": message.to_string() }))
}

// Same reasoning as `invalid`.
#[allow(clippy::needless_pass_by_value)]
fn not_found(message: impl ToString) -> Value {
    with_status("not_found", json!({ "message": message.to_string() }))
}

/// A memory note trimmed for a listing: no body, just enough to decide
/// whether to read it. Claim responses are polled and must stay small.
fn memory_digest(note: &MemoryNote) -> Value {
    to_value(&note.digest())
}

fn to_value<T: Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

/// Agents listed by a verbose `status` at most.
const STATUS_AGENTS_MAX: usize = 50;
/// Fields dropped from list rows: bulky and rarely acted on.
const ROW_DROP: [&str; 1] = ["acked_by"];

/// Trims a list row for agents: nulls and empty arrays go, ids become
/// their eight-character prefix (every id input accepts a unique prefix),
/// and timestamps lose their sub-second digits. The full item is always
/// available from the tool that returns one item.
fn compact(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter(|(k, v)| {
                    !ROW_DROP.contains(&k.as_str())
                        && !v.is_null()
                        && !v.as_array().is_some_and(Vec::is_empty)
                })
                .map(|(k, v)| (k, compact(v)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(compact).collect()),
        Value::String(text) => Value::String(compact_string(text)),
        other => other,
    }
}

fn compact_string(text: String) -> String {
    if text.len() == 36 && uuid::Uuid::parse_str(&text).is_ok() {
        return text[..8].to_owned();
    }
    if text.len() > 20 && text.ends_with('Z') {
        if let Ok(at) = DateTime::parse_from_rfc3339(&text) {
            return at
                .with_timezone(&Utc)
                .to_rfc3339_opts(SecondsFormat::Secs, true);
        }
    }
    text
}

/// Parses a paging cursor. A bare RFC 3339 timestamp means "strictly
/// older than"; the `next_before` value of a previous page also carries
/// the id of the oldest row returned, so equal timestamps page cleanly.
fn cursor<I: Copy>(
    raw: Option<&str>,
    parse: impl Fn(&str) -> Result<I, IdError>,
    nil: I,
) -> Result<Option<(DateTime<Utc>, I)>, Value> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    Ok(Some(match raw.split_once('|') {
        Some((at, id)) => (timestamp(at)?, parse(id)?),
        None => (timestamp(raw)?, nil),
    }))
}

/// The list response shape shared by every list tool: newest first,
/// bounded, with `total`, `truncated`, and a `next_before` cursor when
/// more rows exist.
fn listing<T: Serialize, I: std::fmt::Display>(
    what: &str,
    page: &Page<T>,
    key: impl Fn(&T) -> (DateTime<Utc>, I),
) -> Value {
    let rows: Vec<Value> = page.items.iter().map(|t| compact(to_value(t))).collect();
    let mut value = json!({
        "count": rows.len(),
        "total": page.total,
        "truncated": page.truncated(),
        what: rows,
    });
    if let Some((at, id)) = page.next_before(key) {
        value["next_before"] = Value::String(format!(
            "{}|{id}",
            at.to_rfc3339_opts(SecondsFormat::AutoSi, true)
        ));
    }
    ok(value)
}

/// Result keys whose array length is the natural summary of a listing.
const LISTS: [&str; 7] = [
    "claims",
    "tasks",
    "contracts",
    "notices",
    "decisions",
    "notes",
    "agents",
];
/// Result keys holding the single item a mutation produced.
const ITEMS: [&str; 5] = ["task", "contract", "notice", "decision", "note"];
/// Longest text block a tool result carries, in characters.
const SUMMARY_MAX: usize = 160;

/// One line describing an outcome for clients that show only text. Never
/// the JSON itself; the structured content carries that.
fn summary(outcome: &Value) -> String {
    let status = outcome["status"].as_str().unwrap_or("ok");
    let line = if let Some(message) = outcome["message"].as_str() {
        format!("{status}: {message}")
    } else if let Some(paths) = outcome["new_paths"].as_array() {
        let renewed = outcome["renewed_paths"].as_array().map_or(0, Vec::len);
        format!(
            "{status}: claimed {} until {}",
            join_paths(paths, renewed),
            outcome["expires_at"].as_str().unwrap_or("?")
        )
    } else if let Some(paths) = outcome["released"].as_array() {
        format!("{status}: released {}", join_paths(paths, 0))
    } else if let Some(count) = outcome["count"].as_u64() {
        let what = LISTS
            .iter()
            .find(|k| outcome[k].is_array())
            .copied()
            .unwrap_or("items");
        format!("{status}: {count} {what}")
    } else if let Some((kind, item)) = ITEMS
        .iter()
        .find_map(|k| outcome[k].as_object().map(|o| (*k, o)))
    {
        let id = item.get("id").and_then(Value::as_str).unwrap_or("");
        let name = ["title", "name", "summary", "permalink"]
            .iter()
            .find_map(|k| item.get(*k).and_then(Value::as_str))
            .unwrap_or("");
        format!("{status}: {kind} {} {name}", &id[..id.len().min(8)])
    } else {
        status.to_owned()
    };
    let mut cut: String = line.chars().take(SUMMARY_MAX).collect();
    if cut.chars().count() < line.chars().count() {
        cut.push('…');
    }
    cut
}

fn join_paths(paths: &[Value], renewed: usize) -> String {
    let mut parts: Vec<String> = paths
        .iter()
        .filter_map(Value::as_str)
        .take(3)
        .map(str::to_owned)
        .collect();
    if paths.len() > 3 {
        parts.push(format!("+{}", paths.len() - 3));
    }
    if renewed > 0 {
        parts.push(format!("(+{renewed} renewed)"));
    }
    parts.join(" ")
}

/// Strips what schemars emits that an MCP client does not need: the
/// `$schema` URL, `default: null`, integer `format` and `minimum`, the
/// `["T", "null"]` unions on optional fields (absence from `required`
/// already says optional), and per-parameter descriptions. Parameter
/// names are self-describing; the semantics that are not obvious live in
/// the tool's one-sentence description and in `docs/1-about/04-primitives.md`.
/// Every agent downloads every schema once per session, so this is a
/// per-session token cost, pinned by a test in `tests/http_roundtrip.rs`.
fn slim_schema(schema: &mut Value) {
    let Some(obj) = schema.as_object_mut() else {
        return;
    };
    obj.remove("$schema");
    obj.remove("format");
    obj.remove("minimum");
    if obj.get("default").is_some_and(Value::is_null) {
        obj.remove("default");
    }
    if let Some(Value::Array(types)) = obj.get("type") {
        let kept: Vec<Value> = types
            .iter()
            .filter(|t| t.as_str() != Some("null"))
            .cloned()
            .collect();
        if let [only] = kept.as_slice() {
            obj.insert("type".into(), only.clone());
        }
    }
    if let Some(Value::Object(props)) = obj.get_mut("properties") {
        for prop in props.values_mut() {
            if let Some(field) = prop.as_object_mut() {
                field.remove("description");
            }
            slim_schema(prop);
        }
    }
    if let Some(items) = obj.get_mut("items") {
        slim_schema(items);
    }
}

fn run(f: impl FnOnce() -> Outcome) -> Value {
    f().unwrap_or_else(identity)
}

fn agent(raw: &str) -> Result<AgentId, Value> {
    AgentId::new(raw).map_err(invalid)
}

fn opt_agent(raw: Option<&str>) -> Result<Option<AgentId>, Value> {
    raw.map(agent).transpose()
}

fn paths(raw: &[String]) -> Result<Vec<RepoPath>, Value> {
    parse_paths(raw).map_err(invalid)
}

fn opt_paths(raw: Option<&Vec<String>>) -> Result<Vec<RepoPath>, Value> {
    raw.map_or(Ok(Vec::new()), |r| paths(r))
}

fn opt_path(raw: Option<&str>) -> Result<Option<RepoPath>, Value> {
    raw.map(|r| RepoPath::new(r).map_err(invalid)).transpose()
}

fn parse<T: std::str::FromStr>(raw: &str) -> Result<T, Value>
where
    T::Err: ToString,
{
    raw.parse().map_err(invalid)
}

fn opt_parse<T: std::str::FromStr>(raw: Option<&str>) -> Result<Option<T>, Value>
where
    T::Err: ToString,
{
    raw.map(parse).transpose()
}

fn timestamp(raw: &str) -> Result<DateTime<Utc>, Value> {
    DateTime::parse_from_rfc3339(raw.trim())
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| invalid(format!("timestamps must be RFC 3339: {e}")))
}

impl From<PrefixError> for Value {
    fn from(e: PrefixError) -> Self {
        match e {
            PrefixError::NotFound { .. } => not_found(e),
            PrefixError::Ambiguous { .. } => invalid(e),
        }
    }
}

impl From<IdError> for Value {
    fn from(e: IdError) -> Self {
        invalid(e)
    }
}

impl From<PathError> for Value {
    fn from(e: PathError) -> Self {
        invalid(e)
    }
}

impl From<ClaimError> for Value {
    fn from(e: ClaimError) -> Self {
        match e {
            ClaimError::Conflict(conflicts) => with_status(
                "conflict",
                json!({ "conflicts": conflicts, "message": "overlapping claims held by other agents" }),
            ),
            ClaimError::NotHeld { .. } | ClaimError::NoClaims(_) => not_found(e),
            ClaimError::NoPaths | ClaimError::InvalidTtl(_) => invalid(e),
        }
    }
}

impl From<TaskError> for Value {
    fn from(e: TaskError) -> Self {
        let message = e.to_string();
        match e {
            TaskError::NotFound(_) | TaskError::UnknownDependency(_) => not_found(e),
            TaskError::EmptyTitle | TaskError::UnknownStatus(_) => invalid(e),
            TaskError::OwnedByOther { owner, since, .. } => with_status(
                "conflict",
                json!({ "owner": owner, "since": since, "message": message }),
            ),
        }
    }
}

impl From<ContractError> for Value {
    fn from(e: ContractError) -> Self {
        let message = e.to_string();
        match e {
            ContractError::NotFound(_) => not_found(e),
            ContractError::EmptyName | ContractError::UnknownKind(_) => invalid(e),
            ContractError::VersionConflict {
                current,
                published_by,
                ..
            } => with_status(
                "conflict",
                json!({ "current_version": current, "published_by": published_by, "message": message }),
            ),
        }
    }
}

impl From<NoticeError> for Value {
    fn from(e: NoticeError) -> Self {
        match e {
            NoticeError::NotFound(_) => not_found(e),
            NoticeError::EmptySummary | NoticeError::UnknownKind(_) => invalid(e),
        }
    }
}

impl From<DecisionError> for Value {
    fn from(e: DecisionError) -> Self {
        invalid(e)
    }
}

impl From<MemoryError> for Value {
    fn from(e: MemoryError) -> Self {
        match e {
            MemoryError::NotFound(_) => not_found(e),
            MemoryError::Conflict {
                ref permalink,
                updated_at,
            } => with_status(
                "conflict",
                json!({
                    "permalink": permalink,
                    "updated_at": updated_at,
                    "message": e.to_string(),
                }),
            ),
            other => invalid(other),
        }
    }
}

// ---------------------------------------------------------------------------
// The MCP server.
// ---------------------------------------------------------------------------

/// The MCP handler. Cheap to clone; one is created per client session.
#[derive(Debug, Clone)]
pub struct TirithServer {
    state: Arc<State>,
    persister: Arc<Persister>,
}

impl TirithServer {
    /// A handler over shared state and a persister.
    pub fn new(state: Arc<State>, persister: Arc<Persister>) -> Self {
        Self { state, persister }
    }

    /// The shared state.
    pub fn state(&self) -> &Arc<State> {
        &self.state
    }

    /// Waits for pending changes to reach disk and wraps `outcome` as a
    /// tool result. Calls that only renewed leases do not wait.
    ///
    /// The outcome travels once, as structured content. The text block is
    /// a one-line summary for clients that show only text; it is never the
    /// JSON again, which would double the tokens of every call.
    async fn finish(&self, mut outcome: Value) -> Result<CallToolResult, McpError> {
        let mut not_persisted = None;
        if self.state.is_dirty() {
            if let Err(error) = self.persister.flush().await {
                tracing::error!(%error, "failed to persist state");
                not_persisted = Some(error.to_string());
            }
        }
        // The domain decision stands (it is what every other agent sees),
        // but the caller must know it may not survive a restart.
        if let Some(error) = &not_persisted {
            if let Value::Object(map) = &mut outcome {
                map.insert("persist_error".into(), Value::String(error.clone()));
            }
        }
        let text = match &not_persisted {
            Some(error) => format!("warning: not persisted ({error}); {}", summary(&outcome)),
            None => summary(&outcome),
        };
        let mut result = CallToolResult::structured(outcome);
        result.content = vec![ContentBlock::text(text)];
        Ok(result)
    }
}

#[tool_router]
impl TirithServer {
    /// Claim files or directories before editing them.
    #[tool(
        name = "claim",
        description = "Claim paths before editing (a directory covers its contents); nothing is claimed on conflict. ttl_secs: default 600, max 3600."
    )]
    async fn claim(
        &self,
        Parameters(input): Parameters<ClaimInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = agent(&input.agent)?;
            let paths = paths(&input.paths)?;
            let granted = self.state.claim(
                agent,
                paths.clone(),
                input.reason.trim().to_owned(),
                input.ttl_secs,
            )?;
            // Knowledge about these paths reaches the agent that is about to
            // edit them, without anyone having to search for it. Capped and
            // excerpted so a directory claim cannot return the whole book.
            let memory: Vec<Value> = self
                .state
                .memory_for_paths(&paths)
                .iter()
                .map(memory_digest)
                .collect();
            let mut value = to_value(&granted);
            if let Some(object) = value.as_object_mut() {
                object.insert("memory".to_owned(), Value::Array(memory));
            }
            Ok(ok(value))
        });
        self.finish(outcome).await
    }

    /// Release claims when done.
    #[tool(
        name = "release",
        description = "Release paths, or everything you hold when paths is omitted."
    )]
    async fn release(
        &self,
        Parameters(input): Parameters<ReleaseInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = agent(&input.agent)?;
            let paths = input.paths.as_ref().map(|p| paths(p)).transpose()?;
            let released = self.state.release(&agent, paths)?;
            Ok(ok(json!({ "agent": agent, "released": released })))
        });
        self.finish(outcome).await
    }

    /// Renew every lease you hold.
    #[tool(name = "renew", description = "Extend every lease you hold.")]
    async fn renew(
        &self,
        Parameters(input): Parameters<AgentInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = agent(&input.agent)?;
            let claims = self.state.renew(&agent)?;
            let expires_at = claims.iter().map(|c| c.expires_at).max();
            Ok(ok(
                json!({ "agent": agent, "count": claims.len(), "expires_at": expires_at }),
            ))
        });
        self.finish(outcome).await
    }

    /// List live claims.
    #[tool(
        name = "claims_list",
        description = "Your live claims plus any overlapping path; all=true for every claim. Newest first, paged."
    )]
    async fn claims_list(
        &self,
        Parameters(input): Parameters<ClaimsListInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = agent(&input.agent)?;
            let path = opt_path(input.path.as_deref())?;
            let mine = (!input.all.unwrap_or(false)).then(|| agent.clone());
            let before = cursor(input.before.as_deref(), ClaimId::parse, ClaimId::nil())?;
            let page = self.state.claims_page(
                Some(&agent),
                mine.as_ref(),
                path.as_ref(),
                before.as_ref(),
                clamp_limit(input.limit),
            );
            Ok(listing("claims", &page, |c| (c.claimed_at, c.id)))
        });
        self.finish(outcome).await
    }

    /// Create a task.
    #[tool(
        name = "task_create",
        description = "Add a todo task; depends_on ids must be done before it can be pulled; higher priority pulls first."
    )]
    async fn task_create(
        &self,
        Parameters(input): Parameters<TaskCreateInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = agent(&input.agent)?;
            let depends_on = input
                .depends_on
                .as_deref()
                .unwrap_or_default()
                .iter()
                .map(|s| self.state.resolve_task(s))
                .collect::<Result<Vec<_>, _>>()?;
            let task = self.state.task_create(
                agent,
                NewTask {
                    title: input.title,
                    description: input.description.unwrap_or_default(),
                    priority: input.priority.unwrap_or(0),
                    depends_on,
                    paths: opt_paths(input.paths.as_ref())?,
                },
            )?;
            Ok(ok(json!({ "task": task })))
        });
        self.finish(outcome).await
    }

    /// Pull the next unblocked task.
    #[tool(
        name = "task_pull",
        description = "Take the highest-priority unblocked todo task as in_progress."
    )]
    async fn task_pull(
        &self,
        Parameters(input): Parameters<AgentInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = agent(&input.agent)?;
            Ok(match self.state.task_pull(agent) {
                Some(task) => ok(json!({ "task": task })),
                None => with_status("none", json!({ "message": "no unblocked todo tasks" })),
            })
        });
        self.finish(outcome).await
    }

    /// Update a task's status.
    #[tool(
        name = "task_update",
        description = "Set status (todo|in_progress|blocked|done) with an optional note; another agent's in_progress task is a conflict unless force."
    )]
    async fn task_update(
        &self,
        Parameters(input): Parameters<TaskUpdateInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = agent(&input.agent)?;
            let id = self.state.resolve_task(&input.task_id)?;
            let status: TaskStatus = parse(&input.status)?;
            let force = input.force.unwrap_or(false);
            let task = self
                .state
                .task_update(agent, id, status, input.note, force)?;
            Ok(ok(json!({ "task": task })))
        });
        self.finish(outcome).await
    }

    /// List tasks.
    #[tool(
        name = "task_list",
        description = "List tasks by status and owner, most recently updated first, paged."
    )]
    async fn task_list(
        &self,
        Parameters(input): Parameters<TaskListInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = agent(&input.agent)?;
            let status: Option<TaskStatus> = opt_parse(input.status.as_deref())?;
            let owner = opt_agent(input.owner.as_deref())?;
            let before = cursor(input.before.as_deref(), TaskId::parse, TaskId::nil())?;
            let page = self.state.tasks_page(
                Some(&agent),
                status,
                owner.as_ref(),
                before.as_ref(),
                clamp_limit(input.limit),
            );
            Ok(listing("tasks", &page, |t| (t.updated_at, t.id)))
        });
        self.finish(outcome).await
    }

    /// Publish an interface contract.
    #[tool(
        name = "contract_publish",
        description = "Publish an interface shape (kind: http|function|type|event|cli|other). Republish: new version, consumers kept unless given, conflict on stale expected_version."
    )]
    async fn contract_publish(
        &self,
        Parameters(input): Parameters<ContractPublishInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = agent(&input.agent)?;
            let kind: ContractKind = parse(&input.kind)?;
            let (published, notice) = self.state.contract_publish(
                agent,
                NewContract {
                    name: input.name,
                    kind,
                    shape: input.shape,
                    consumers: input.consumers.as_deref().map(paths).transpose()?,
                    notes: input.notes.unwrap_or_default(),
                    expected_version: input.expected_version,
                },
            )?;
            Ok(ok(json!({
                "contract": published.contract,
                "previous_version": published.previous_version,
                "notice_id": notice.map(|n| n.id),
            })))
        });
        self.finish(outcome).await
    }

    /// Fetch a contract.
    #[tool(
        name = "contract_get",
        description = "Fetch a contract by name or id, with its versions."
    )]
    async fn contract_get(
        &self,
        Parameters(input): Parameters<ContractGetInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = agent(&input.agent)?;
            Ok(match self.state.contract_get(Some(&agent), &input.name) {
                Some(contract) => ok(json!({ "contract": contract })),
                None => ContractError::NotFound(input.name).into(),
            })
        });
        self.finish(outcome).await
    }

    /// List contracts.
    #[tool(
        name = "contract_list",
        description = "List contracts by consumer path or kind, newest first, paged."
    )]
    async fn contract_list(
        &self,
        Parameters(input): Parameters<ContractListInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = agent(&input.agent)?;
            let path = opt_path(input.path.as_deref())?;
            let kind: Option<ContractKind> = opt_parse(input.kind.as_deref())?;
            let before = cursor(
                input.before.as_deref(),
                ContractId::parse,
                ContractId::nil(),
            )?;
            let page = self.state.contracts_page(
                Some(&agent),
                path.as_ref(),
                kind,
                before.as_ref(),
                clamp_limit(input.limit),
            );
            Ok(listing("contracts", &page, |c| {
                (c.current.published_at, c.id)
            }))
        });
        self.finish(outcome).await
    }

    /// Publish a change notice.
    #[tool(
        name = "notice_publish",
        description = "Announce a change (kind: rename|signature|removed|moved|behavior) and the affected_paths that must react."
    )]
    async fn notice_publish(
        &self,
        Parameters(input): Parameters<NoticePublishInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = agent(&input.agent)?;
            let kind: NoticeKind = parse(&input.kind)?;
            let contract_id = input
                .contract_id
                .as_deref()
                .map(ContractId::parse)
                .transpose()?;
            let notice = self.state.notice_publish(
                agent,
                NewNotice {
                    kind,
                    summary: input.summary,
                    from: input.from,
                    to: input.to,
                    affected_paths: paths(&input.affected_paths)?,
                    contract_id,
                },
            )?;
            Ok(ok(json!({ "notice": notice })))
        });
        self.finish(outcome).await
    }

    /// List change notices.
    #[tool(
        name = "notice_list",
        description = "List change notices by path, since (RFC 3339), or unread only, newest first, paged. Unread without a path covers the paths you hold unless all=true."
    )]
    async fn notice_list(
        &self,
        Parameters(input): Parameters<NoticeListInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = agent(&input.agent)?;
            let unread = input.unread.unwrap_or(false);
            let filter = NoticeFilter {
                path: opt_path(input.path.as_deref())?,
                since: input.since.as_deref().map(timestamp).transpose()?,
                unread_by: unread.then(|| agent.clone()),
            };
            // Unread with no path means "what should I react to", which is
            // the paths I hold; everything else is a request for `all`.
            let scoped = unread && filter.path.is_none() && !input.all.unwrap_or(false);
            let held = if scoped {
                self.state.held_paths(&agent)
            } else {
                Vec::new()
            };
            if scoped && held.is_empty() {
                return Ok(ok(json!({
                    "count": 0, "total": 0, "truncated": false, "notices": [],
                    "message": "you hold no claims; pass path or all=true to look beyond them",
                })));
            }
            let before = cursor(input.before.as_deref(), NoticeId::parse, NoticeId::nil())?;
            let page = self.state.notices_page(
                Some(&agent),
                &filter,
                &held,
                before.as_ref(),
                clamp_limit(input.limit),
            );
            Ok(listing("notices", &page, |n| (n.published_at, n.id)))
        });
        self.finish(outcome).await
    }

    /// Acknowledge a notice.
    #[tool(
        name = "notice_ack",
        description = "Mark a notice handled so it is no longer unread."
    )]
    async fn notice_ack(
        &self,
        Parameters(input): Parameters<NoticeAckInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = agent(&input.agent)?;
            let id = self.state.resolve_notice(&input.notice_id)?;
            let notice = self.state.notice_ack(agent, id)?;
            Ok(ok(json!({ "notice": notice })))
        });
        self.finish(outcome).await
    }

    /// Record a decision.
    #[tool(
        name = "decision_record",
        description = "Record a settled choice with its rationale, alternatives, and the paths it affects."
    )]
    async fn decision_record(
        &self,
        Parameters(input): Parameters<DecisionRecordInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = agent(&input.agent)?;
            let decision = self.state.decision_record(
                agent,
                NewDecision {
                    title: input.title,
                    decision: input.decision,
                    rationale: input.rationale.unwrap_or_default(),
                    alternatives: input.alternatives.unwrap_or_default(),
                    affects_paths: opt_paths(input.affects_paths.as_ref())?,
                },
            )?;
            Ok(ok(json!({ "decision": decision })))
        });
        self.finish(outcome).await
    }

    /// List decisions.
    #[tool(
        name = "decision_list",
        description = "List decisions by path or text query, newest first, paged."
    )]
    async fn decision_list(
        &self,
        Parameters(input): Parameters<DecisionListInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = agent(&input.agent)?;
            let path = opt_path(input.path.as_deref())?;
            let before = cursor(
                input.before.as_deref(),
                DecisionId::parse,
                DecisionId::nil(),
            )?;
            let page = self.state.decisions_page(
                Some(&agent),
                path.as_ref(),
                input.query.as_deref(),
                before.as_ref(),
                clamp_limit(input.limit),
            );
            Ok(listing("decisions", &page, |d| (d.recorded_at, d.id)))
        });
        self.finish(outcome).await
    }

    /// Server status.
    #[tool(
        name = "status",
        description = "Counts, persistence and load problems; verbose=true adds who holds what."
    )]
    async fn status(
        &self,
        Parameters(input): Parameters<StatusInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = opt_agent(input.agent.as_deref())?;
            let report = self.state.status(agent.as_ref());
            let mut value = json!({
                "version": VERSION,
                "started_at": report.started_at,
                "now": report.now,
                "uptime_secs": report.uptime_secs,
                "seq": report.seq,
                "claims": report.claims,
                "tasks_open": report.tasks_open,
                "tasks_done": report.tasks_done,
                "tasks_orphaned": report.tasks_orphaned,
                "contracts": report.contracts,
                "notices": report.notices,
                "decisions": report.decisions,
                "memory": report.memory,
                "agents_active": report.agents.len(),
                "persist_error": self.persister.last_error(),
                "load_errors": report.load_errors,
            });
            if input.verbose.unwrap_or(false) {
                let agents: Vec<Value> = report
                    .agents
                    .iter()
                    .take(STATUS_AGENTS_MAX)
                    .map(|a| {
                        json!({
                            "agent": a.agent,
                            "paths_count": a.paths.len(),
                            "expires_at": a.expires_at,
                            "tasks_in_progress": a.tasks_in_progress,
                        })
                    })
                    .collect();
                value["agents_truncated"] = Value::Bool(report.agents.len() > agents.len());
                value["agents"] = Value::Array(agents);
            }
            Ok(ok(value))
        });
        self.finish(outcome).await
    }

    /// Write a memory note.
    #[tool(
        name = "memory_write",
        description = "Write a durable note (kind: fact|lesson|gotcha|handoff|research|note); an existing title updates it."
    )]
    async fn memory_write(
        &self,
        Parameters(input): Parameters<MemoryWriteInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = agent(&input.agent)?;
            let kind = match input.kind.as_deref().map(str::trim) {
                Some(raw) if !raw.is_empty() => raw.parse::<MemoryKind>()?,
                _ => MemoryKind::default(),
            };
            let permalink = match input.permalink.as_deref().map(str::trim) {
                Some(raw) if !raw.is_empty() => Some(Permalink::parse(raw)?),
                _ => None,
            };
            let written = self.state.memory_write(
                agent,
                NewMemory {
                    title: input.title,
                    kind,
                    body: input.body,
                    paths: opt_paths(input.paths.as_ref())?,
                    tags: input.tags.unwrap_or_default(),
                    permalink,
                    if_updated_at: input.if_updated_at.as_deref().map(timestamp).transpose()?,
                },
            )?;
            Ok(ok(json!({
                "note": written.note,
                "created": written.created,
            })))
        });
        self.finish(outcome).await
    }

    /// Read one memory note.
    #[tool(
        name = "memory_read",
        description = "Read a note by permalink, id, or title; depth (0 to 3) adds related notes."
    )]
    async fn memory_read(
        &self,
        Parameters(input): Parameters<MemoryReadInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = agent(&input.agent)?;
            let depth = input.depth.unwrap_or(0);
            if depth > MAX_CONTEXT_DEPTH {
                return Ok(invalid(format!(
                    "depth must be at most {MAX_CONTEXT_DEPTH}"
                )));
            }
            let Some((note, related)) = self.state.memory_get(Some(&agent), &input.name, depth)
            else {
                return Ok(not_found(format!(
                    "no memory note matching `{}`",
                    input.name
                )));
            };
            let related: Vec<Value> = related.iter().map(memory_digest).collect();
            Ok(ok(json!({ "note": note, "related": related })))
        });
        self.finish(outcome).await
    }

    /// Search memory notes.
    #[tool(
        name = "memory_search",
        description = "Search notes, best first; no query lists the newest. limit: default 20, max 100."
    )]
    async fn memory_search(
        &self,
        Parameters(input): Parameters<MemorySearchInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = agent(&input.agent)?;
            let kind = match input.kind.as_deref().map(str::trim) {
                Some(raw) if !raw.is_empty() => Some(raw.parse::<MemoryKind>()?),
                _ => None,
            };
            let limit = input
                .limit
                .unwrap_or(DEFAULT_SEARCH_LIMIT)
                .clamp(1, MAX_SEARCH_LIMIT);
            let filter = MemorySearch {
                query: input.query,
                path: opt_path(input.path.as_deref())?,
                kind,
                tag: input.tag,
                since: input.since.as_deref().map(timestamp).transpose()?,
                // One more than asked, so `truncated` is known without
                // scoring the whole book twice.
                limit: Some(limit.saturating_add(1)),
            };
            let mut hits = self.state.memory_search(Some(&agent), &filter);
            let truncated = hits.len() > limit;
            hits.truncate(limit);
            let notes: Vec<Value> = hits
                .iter()
                .map(|(note, score)| {
                    let mut value = memory_digest(note);
                    if let Some(object) = value.as_object_mut() {
                        object.insert("score".to_owned(), Value::from(*score));
                    }
                    value
                })
                .collect();
            Ok(ok(json!({
                "count": notes.len(),
                "truncated": truncated,
                "notes": notes,
            })))
        });
        self.finish(outcome).await
    }

    /// Delete a memory note.
    #[tool(
        name = "memory_delete",
        description = "Delete a memory note by permalink, id, or exact title. Its file is removed."
    )]
    async fn memory_delete(
        &self,
        Parameters(input): Parameters<MemoryDeleteInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = agent(&input.agent)?;
            let Some(removed) = self.state.memory_delete(&agent, &input.name) else {
                return Ok(not_found(format!(
                    "no memory note matching `{}`",
                    input.name
                )));
            };
            Ok(ok(json!({ "removed": memory_digest(&removed) })))
        });
        self.finish(outcome).await
    }
}

// The `tool_handler` macro generates async trait methods that return
// immediately; that is rmcp's code, not ours, so silence the lint here.
#[allow(unknown_lints, clippy::unused_async_trait_impl)]
#[tool_handler]
impl ServerHandler for TirithServer {
    fn get_info(&self) -> ServerConfig {
        let mut config = ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(INSTRUCTIONS);
        config.server_info = Implementation::new("tirith", VERSION);
        config
    }

    /// The generated `tools/list`, with every input schema slimmed. The
    /// `tool_handler` macro leaves this method alone because it is defined
    /// here.
    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let tools = Self::tool_router()
            .list_all()
            .into_iter()
            .map(|mut tool| {
                let mut schema = Value::Object((*tool.input_schema).clone());
                slim_schema(&mut schema);
                if let Value::Object(map) = schema {
                    tool.input_schema = Arc::new(map);
                }
                tool
            })
            .collect();
        Ok(ListToolsResult::with_all_items(tools))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn slim_schema_drops_what_clients_do_not_need() {
        let mut schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "properties": {
                "agent": {"description": "Your agent name.", "type": "string"},
                "paths": {"items": {"type": "string"}, "type": "array"},
                "ttl_secs": {"default": null, "format": "uint64", "minimum": 0, "type": ["integer", "null"]},
                "kind": {"default": null, "type": ["string", "null"]},
                "tags": {"default": null, "items": {"type": "string"}, "type": ["array", "null"]},
                "verbose": {"default": true, "type": "boolean"}
            },
            "required": ["agent", "paths"],
            "type": "object"
        });
        slim_schema(&mut schema);
        assert_eq!(
            schema,
            json!({
                "properties": {
                    "agent": {"type": "string"},
                    "paths": {"items": {"type": "string"}, "type": "array"},
                    "ttl_secs": {"type": "integer"},
                    "kind": {"type": "string"},
                    "tags": {"items": {"type": "string"}, "type": "array"},
                    "verbose": {"default": true, "type": "boolean"}
                },
                "required": ["agent", "paths"],
                "type": "object"
            })
        );
    }

    #[test]
    fn summaries_are_one_short_line_and_never_json() {
        let cases = [
            (
                json!({"status": "ok", "new_paths": ["src/a.rs", "src/b.rs"], "renewed_paths": [], "expires_at": "2026-09-16T02:10:00Z"}),
                "ok: claimed src/a.rs src/b.rs until 2026-09-16T02:10:00Z",
            ),
            (
                json!({"status": "conflict", "conflicts": [{"path": "src/a.rs"}], "message": "overlapping claims held by other agents"}),
                "conflict: overlapping claims held by other agents",
            ),
            (
                json!({"status": "ok", "released": ["src/a.rs"]}),
                "ok: released src/a.rs",
            ),
            (
                json!({"status": "ok", "count": 40, "notices": []}),
                "ok: 40 notices",
            ),
            (
                json!({"status": "ok", "task": {"id": "c6cf1a3c-b0f3", "title": "Fix it"}}),
                "ok: task c6cf1a3c Fix it",
            ),
            (
                json!({"status": "not_found", "message": "no such task"}),
                "not_found: no such task",
            ),
            (
                json!({"status": "ok", "version": "0.1.3", "claims": 3}),
                "ok",
            ),
        ];
        for (outcome, want) in cases {
            let got = summary(&outcome);
            assert_eq!(got, want);
            assert!(got.len() < 200 && !got.starts_with('{'));
        }
        let long = json!({"status": "ok", "message": "x".repeat(500)});
        assert_eq!(summary(&long).chars().count(), SUMMARY_MAX + 1);
    }
}

// ---------------------------------------------------------------------------
// The daemon.
// ---------------------------------------------------------------------------

/// Options for [`start`].
#[derive(Debug, Clone)]
pub struct ServeOptions {
    /// Address to bind. Port 0 picks a free port.
    pub bind: SocketAddr,
    /// The repository whose `.tirith/` directory holds state.
    pub repo_root: PathBuf,
    /// The clock to use. `None` means the system clock.
    pub clock: Option<Arc<dyn Clock>>,
}

/// Why the daemon could not start or stop.
#[derive(Debug, Error)]
pub enum ServeError {
    /// Persistence failed while loading or recording daemon info.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// The socket could not be bound.
    #[error("bind {addr}: {source}")]
    Bind {
        /// The requested address.
        addr: SocketAddr,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },
    /// The server task ended with an error.
    #[error("server: {0}")]
    Server(String),
}

/// A running daemon. Dropping it without calling
/// [`shutdown`](Self::shutdown) leaves the server task running until the
/// runtime stops.
#[derive(Debug)]
pub struct ServerHandle {
    addr: SocketAddr,
    state: Arc<State>,
    store: Arc<JsonStore>,
    persister: Arc<Persister>,
    shutdown: Option<oneshot::Sender<()>>,
    task: JoinHandle<Result<(), std::io::Error>>,
}

impl ServerHandle {
    /// The bound address.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The MCP endpoint URL.
    pub fn mcp_url(&self) -> String {
        mcp_url(self.addr)
    }

    /// Sets how long an in-progress task's owner may be silent before the
    /// task returns to `todo`; zero disables reaping. The default is
    /// [`crate::state::DEFAULT_TASK_ORPHAN_SECS`].
    pub fn set_task_orphan_secs(&self, secs: u64) {
        self.state.set_task_orphan_secs(secs);
    }

    /// The dashboard URL.
    pub fn dashboard_url(&self) -> String {
        dashboard_url(self.addr)
    }

    /// The shared state, for tests and embedding.
    pub fn state(&self) -> &Arc<State> {
        &self.state
    }

    /// Stops the daemon: stops accepting connections, gives in-flight
    /// requests [`SHUTDOWN_DEADLINE`] to finish, cuts whatever is still
    /// open after that (every stdio shim holds an SSE stream that would
    /// otherwise keep the server alive for good), writes pending state
    /// including lease renewals, and removes the recorded daemon address
    /// if it is still this process's. A newer daemon's `daemon.json` is
    /// left alone.
    pub async fn shutdown(mut self) -> Result<(), ServeError> {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        let result = match tokio::time::timeout(SHUTDOWN_DEADLINE, &mut self.task).await {
            Ok(finished) => finished,
            Err(_deadline) => {
                tracing::info!("connections still open after the deadline; closing them");
                self.task.abort();
                Ok(Ok(()))
            }
        };
        self.persister.sync().await?;
        self.persister.stop();
        let ours = self
            .store
            .read_daemon_info()
            .ok()
            .flatten()
            .is_some_and(|info| info.pid == std::process::id());
        if ours {
            self.store.clear_daemon_info()?;
        }
        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(ServeError::Server(e.to_string())),
            Err(e) if e.is_cancelled() => Ok(()),
            Err(e) => Err(ServeError::Server(e.to_string())),
        }
    }
}

fn mcp_url(addr: SocketAddr) -> String {
    format!("http://{addr}/mcp")
}

fn dashboard_url(addr: SocketAddr) -> String {
    format!("http://{addr}/")
}

/// Loads state from `.tirith/`, binds the HTTP server, and starts serving
/// MCP at `/mcp` and the dashboard at `/`.
pub async fn start(options: ServeOptions) -> Result<ServerHandle, ServeError> {
    let store = Arc::new(JsonStore::new(&options.repo_root));
    store.init()?;
    let snapshot = store.load()?;
    let clock = options
        .clock
        .unwrap_or_else(|| Arc::new(SystemClock) as Arc<dyn Clock>);
    let state = Arc::new(State::new(clock, snapshot));
    let persister = Persister::spawn(Arc::clone(&store), Arc::clone(&state));
    let handler = TirithServer::new(Arc::clone(&state), Arc::clone(&persister));

    let mcp = StreamableHttpService::new(
        move || Ok(handler.clone()),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default().with_json_response(true),
    );
    let listener = TcpListener::bind(options.bind)
        .await
        .map_err(|source| ServeError::Bind {
            addr: options.bind,
            source,
        })?;
    let addr = listener.local_addr().map_err(|source| ServeError::Bind {
        addr: options.bind,
        source,
    })?;
    let context = DashboardContext {
        state: Arc::clone(&state),
        persister: Arc::clone(&persister),
        mcp_url: mcp_url(addr),
        repo_root: options.repo_root.display().to_string(),
    };
    let router = dashboard::router(context).nest_service("/mcp", mcp);

    store.write_daemon_info(&DaemonInfo {
        url: mcp_url(addr),
        dashboard_url: dashboard_url(addr),
        pid: std::process::id(),
        started_at: state.started_at(),
        version: VERSION.to_owned(),
    })?;

    let (tx, rx) = oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = rx.await;
            })
            .await
    });
    tracing::info!(%addr, "tirith listening");
    Ok(ServerHandle {
        addr,
        state,
        store,
        persister,
        shutdown: Some(tx),
        task,
    })
}
