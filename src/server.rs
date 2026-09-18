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
use std::future::Future;
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
use crate::contracts::{Contract, ContractError, ContractKind, NewContract};
use crate::dashboard::{self, DashboardContext};
use crate::decisions::{Decision, DecisionError, NewDecision};
use crate::guide;
use crate::hangup;
use crate::lead::LeadPolicy;
use crate::memory::{
    DEFAULT_SEARCH_LIMIT, MAX_CONTEXT_DEPTH, MAX_SEARCH_LIMIT, MemoryError, MemoryKind, MemoryNote,
    MemorySearch, NewMemory, Permalink,
};
use crate::messages::{HUMAN, MessageError, MessageFilter, NewMessage, is_human};
use crate::notices::{NewNotice, Notice, NoticeError, NoticeFilter, NoticeKind};
use crate::output_schemas;
use crate::registry::{DaemonEntry, REREGISTER_EVERY, Registration};
use crate::state::{Brief, State};
use crate::store::{DaemonInfo, JsonStore, Persister, StoreError};
use crate::tasks::{NewTask, Pulled, TaskError, TaskStatus};
use crate::types::{
    AgentId, ClaimId, ContractId, DecisionId, IdError, MessageId, NoticeId, Page, PathError,
    PrefixError, RepoPath, TaskId, clamp_limit, parse_paths,
};

/// Crate version reported to clients and the dashboard.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Default bind address.
pub const DEFAULT_BIND: &str = "127.0.0.1:7477";

/// How long [`ServerHandle::shutdown`] waits for open connections before
/// closing them. Idle SSE streams never finish on their own.
pub const SHUTDOWN_DEADLINE: std::time::Duration = std::time::Duration::from_secs(2);

/// Instructions sent to every MCP client on initialize.
pub const INSTRUCTIONS: &str = "Tirith coordinates parallel coding agents in one repository. Pick a \
stable, unique `agent` name and pass it to every call. `claim` the paths you will edit before editing; \
the ok reply carries a brief of the unread notices, contracts, decisions and memory notes for those \
paths, so read it first. `conflict` means do not edit those paths. `contract_publish` before \
implementing an interface another agent consumes; `notice_publish` for renames or signature changes \
that affect other files; `release` when done. Any call renews your leases; a `lost` field means a \
lease ended and you must claim again. Call `guide` for the whole protocol.";

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
    /// Attach the brief. Default true.
    #[serde(default)]
    pub brief: Option<bool>,
    /// On conflict, wait up to this many seconds (max 120) for the paths to free.
    #[serde(default)]
    pub wait_secs: Option<u64>,
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

/// Input for `task_pull`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TaskPullInput {
    /// Your stable agent name.
    pub agent: String,
    /// Wait up to this many seconds (max 120) while todo tasks are claimed or wait on dependencies.
    #[serde(default)]
    pub wait_secs: Option<u64>,
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
    /// What needs doing, for whoever pulls the task.
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
    /// The task's id, or a unique prefix of it.
    pub task_id: String,
    /// One of `todo`, `in_progress`, `blocked`, `done`.
    #[schemars(extend("enum" = ["todo", "in_progress", "blocked", "done"]))]
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
    #[schemars(extend("enum" = ["todo", "in_progress", "blocked", "done"]))]
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
    #[schemars(extend("enum" = ["http", "function", "type", "event", "cli", "other"]))]
    pub kind: String,
    /// The interface shape as JSON: signatures, request and response types, errors.
    pub shape: Value,
    /// Paths expected to depend on this contract.
    #[serde(default)]
    pub consumers: Option<Vec<String>>,
    /// Free-text notes stored with this version.
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
    /// The contract's name, its id, or a unique prefix of the id.
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
    #[schemars(extend("enum" = ["http", "function", "type", "event", "cli", "other"]))]
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
    #[schemars(extend("enum" = ["rename", "signature", "removed", "moved", "behavior"]))]
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
    /// Only notices you neither published nor were already shown. Without
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

/// Input for `decision_record`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DecisionRecordInput {
    /// Your stable agent name.
    pub agent: String,
    /// Short title, the line shown in briefs.
    pub title: String,
    /// What was decided, stated so it can be followed.
    pub decision: String,
    /// Why it was decided.
    #[serde(default)]
    pub rationale: Option<String>,
    /// The alternatives that were considered and rejected.
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

/// Input for `message_send`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct MessageSendInput {
    /// Your stable agent name.
    pub agent: String,
    /// The recipient's agent name, `*` for every agent active in the last
    /// hour, or `human` for the human queue.
    pub to: String,
    /// The message, at most 1000 characters.
    pub text: String,
    /// Id, or a unique prefix, of the message this one answers.
    #[serde(default)]
    pub reply_to: Option<String>,
    /// Paths the message is about.
    #[serde(default)]
    pub paths: Option<Vec<String>>,
}

/// Input for `message_list`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct MessageListInput {
    /// Your stable agent name.
    pub agent: String,
    /// Only the conversation with this agent.
    #[serde(default)]
    pub with: Option<String>,
    /// Only messages sent at or after this RFC 3339 instant.
    #[serde(default)]
    pub since: Option<String>,
    /// Only messages to you that you have not received yet.
    #[serde(default)]
    pub unread: Option<bool>,
    /// Rows to return; default 20, max 200.
    #[serde(default)]
    pub limit: Option<usize>,
    /// Only messages older than this cursor: an RFC 3339 timestamp or a
    /// previous response's `next_before`.
    #[serde(default)]
    pub before: Option<String>,
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
    /// One of `fact`, `lesson`, `gotcha`, `handoff`, `research`, `decision`,
    /// `note` (the default).
    #[serde(default)]
    #[schemars(extend("enum" = ["fact", "lesson", "gotcha", "handoff", "research", "decision", "note"]))]
    pub kind: Option<String>,
    /// Repo-relative paths this note is about.
    #[serde(default)]
    pub paths: Option<Vec<String>>,
    /// Tags for filtering; lowercased, and a leading `#` is dropped.
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
    #[schemars(extend("enum" = ["fact", "lesson", "gotcha", "handoff", "research", "decision", "note"]))]
    pub kind: Option<String>,
    /// Only notes carrying this tag.
    #[serde(default)]
    pub tag: Option<String>,
    /// Only notes updated at or after this RFC 3339 timestamp.
    #[serde(default)]
    pub since: Option<String>,
    /// How many to return. Default 10, max 50.
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Input for `guide`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct GuideInput {
    /// Your stable agent name, optional here. With it, waiting messages and
    /// lost leases ride on the reply as on any other call.
    #[serde(default)]
    pub agent: Option<String>,
    /// One primitive to explain instead of the overview.
    #[serde(default)]
    #[schemars(extend("enum" = ["overview", "claims", "tasks", "contracts", "notices", "decisions", "memory", "messages", "lead", "server"]))]
    pub topic: Option<String>,
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

/// A wait that ended because the caller went away (ADR-0031). Nobody
/// reads it; the status only keeps the log and tests honest.
// Same reasoning as `invalid`.
#[allow(clippy::needless_pass_by_value)]
fn cancelled(message: impl ToString) -> Value {
    with_status("cancelled", json!({ "message": message.to_string() }))
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

/// A notice as a brief row: enough to decide whether to fetch it.
fn notice_digest(notice: &Notice) -> Value {
    compact(json!({
        "id": notice.id,
        "kind": notice.kind,
        "summary": truncate(&notice.summary, BRIEF_SUMMARY_MAX),
        "by": notice.published_by,
    }))
}

/// A contract as a brief row; the body comes from `contract_get`.
fn contract_digest(contract: &Contract) -> Value {
    json!({
        "name": contract.name,
        "version": contract.current.version,
        "kind": contract.kind,
    })
}

/// A decision as a brief row.
fn decision_digest(decision: &Decision) -> Value {
    compact(json!({ "id": decision.id, "title": decision.title }))
}

/// `text` cut to `max` characters, with an ellipsis when it was longer.
fn truncate(text: &str, max: usize) -> String {
    let mut cut: String = text.chars().take(max).collect();
    if cut.chars().count() < text.chars().count() {
        cut.push('…');
    }
    cut
}

/// Adds the brief sections and `more` to an ok claim `value`, then drops
/// the oldest rows of the largest section until the whole response fits
/// in [`BRIEF_MAX_BYTES`] (ADR-0014). Empty sections are omitted.
fn attach_brief(value: &Value, brief: &Brief) -> Value {
    let mut sections: Vec<(&str, Vec<Value>, usize)> = vec![
        (
            "notices",
            brief.notices.iter().map(notice_digest).collect(),
            brief.more.notices,
        ),
        (
            "contracts",
            brief.contracts.iter().map(contract_digest).collect(),
            brief.more.contracts,
        ),
        (
            "decisions",
            brief.decisions.iter().map(decision_digest).collect(),
            brief.more.decisions,
        ),
        (
            "memory",
            brief.memory.iter().map(memory_digest).collect(),
            brief.more.memory,
        ),
    ];
    loop {
        let rendered = render_brief(value, &sections);
        if json_len(&rendered) <= BRIEF_MAX_BYTES {
            return rendered;
        }
        let largest = sections
            .iter()
            .enumerate()
            .filter(|(_, (_, rows, _))| !rows.is_empty())
            .max_by_key(|(_, (_, rows, _))| json_len(rows))
            .map(|(i, _)| i);
        let Some(i) = largest else {
            return rendered;
        };
        sections[i].1.pop();
        sections[i].2 += 1;
    }
}

/// `base` plus every non-empty section and the `more` counts.
fn render_brief(base: &Value, sections: &[(&str, Vec<Value>, usize)]) -> Value {
    let mut value = base.clone();
    if let Some(object) = value.as_object_mut() {
        let mut more = serde_json::Map::new();
        for (name, rows, left) in sections {
            if !rows.is_empty() {
                object.insert((*name).to_owned(), Value::Array(rows.clone()));
            }
            more.insert((*name).to_owned(), Value::from(*left));
        }
        object.insert("more".to_owned(), Value::Object(more));
    }
    value
}

/// Serialized size in bytes; zero if it cannot be serialized.
fn json_len<T: Serialize>(value: &T) -> usize {
    serde_json::to_vec(value).map_or(0, |bytes| bytes.len())
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
    if text.len() > 20
        && text.ends_with('Z')
        && let Ok(at) = DateTime::parse_from_rfc3339(&text)
    {
        return at
            .with_timezone(&Utc)
            .to_rfc3339_opts(SecondsFormat::Secs, true);
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
const LISTS: [&str; 8] = [
    "messages",
    "claims",
    "tasks",
    "contracts",
    "notices",
    "decisions",
    "notes",
    "agents",
];
/// Result keys holding the single item a mutation produced.
const ITEMS: [&str; 6] = ["task", "contract", "notice", "decision", "note", "message"];
/// Longest text block a tool result carries, in characters.
const SUMMARY_MAX: usize = 160;
/// How much of a message the inbox piggyback carries; the rest is a
/// `message_list` away.
const INBOX_EXCERPT: usize = 200;

/// Upper bound on a serialized `ok` claim response, brief included
/// (ADR-0014).
pub const BRIEF_MAX_BYTES: usize = 4096;

/// Longest notice summary shown in a brief row.
const BRIEF_SUMMARY_MAX: usize = 160;

/// One line describing an outcome for clients that show only text. Never
/// the JSON itself; the structured content carries that.
fn summary(outcome: &Value) -> String {
    let status = outcome["status"].as_str().unwrap_or("ok");
    let line = if let Some(message) = outcome["message"].as_str() {
        format!("{status}: {message}")
    } else if let Some(paths) = outcome["new_paths"].as_array() {
        let renewed = outcome["renewed_paths"].as_array().map_or(0, Vec::len);
        let mut line = format!(
            "{status}: claimed {} until {}",
            join_paths(paths, renewed),
            outcome["expires_at"].as_str().unwrap_or("?")
        );
        // The brief's row counts, so a text-only client knows there is
        // something to read in the structured content.
        let counts: Vec<String> = [
            ("notices", "notices"),
            ("contracts", "contracts"),
            ("decisions", "decisions"),
            ("memory", "notes"),
        ]
        .iter()
        .filter_map(|(key, what)| {
            outcome[key]
                .as_array()
                .map(|rows| format!("{} {what}", rows.len()))
        })
        .collect();
        if !counts.is_empty() {
            line.push_str("; brief: ");
            line.push_str(&counts.join(", "));
        }
        line
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
        let id: String = item
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .chars()
            .take(8)
            .collect();
        let name = ["title", "name", "summary", "permalink"]
            .iter()
            .find_map(|k| item.get(*k).and_then(Value::as_str))
            .unwrap_or("");
        format!("{status}: {kind} {id} {name}")
    } else {
        status.to_owned()
    };
    truncate(&line, SUMMARY_MAX)
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
/// already says optional). Parameter descriptions stay: ADR-0034 sends
/// them so a caller can fill in a call without a second source, which
/// supersedes the rule of ADR-0017 that dropped them. Every agent
/// downloads every schema once per session, so what is left is a
/// per-session token cost, pinned by a test in `tests/budgets.rs`.
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

/// [`run`] for handlers that await between `State` calls.
async fn run_async(f: impl Future<Output = Outcome>) -> Value {
    f.await.unwrap_or_else(identity)
}

/// The calling agent's name. `human` is reserved: it names the human
/// queue, never an agent (ADR-0027).
fn agent(raw: &str) -> Result<AgentId, Value> {
    if is_human(raw) {
        return Err(invalid(format!(
            "`{HUMAN}` is reserved for the human queue; pick another agent name"
        )));
    }
    AgentId::new(raw).map_err(invalid)
}

/// An agent name used as a filter, where `human` is allowed.
fn opt_agent(raw: Option<&str>) -> Result<Option<AgentId>, Value> {
    raw.map(|r| AgentId::new(r).map_err(invalid)).transpose()
}

fn paths(raw: &[String]) -> Result<Vec<RepoPath>, Value> {
    parse_paths(raw).map_err(invalid)
}

fn opt_paths(raw: Option<&[String]>) -> Result<Vec<RepoPath>, Value> {
    raw.map_or(Ok(Vec::new()), paths)
}

/// `raw` trimmed, or `None` when absent or blank: optional text inputs
/// that clients send as `""` mean "not given".
fn nonblank(raw: Option<&str>) -> Option<&str> {
    raw.map(str::trim).filter(|r| !r.is_empty())
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

/// Errors whose every variant is an `invalid` outcome.
macro_rules! invalid_from {
    ($($error:ty),* $(,)?) => {
        $(
            impl From<$error> for Value {
                fn from(e: $error) -> Self {
                    invalid(e)
                }
            }
        )*
    };
}

invalid_from!(IdError, PathError, DecisionError);

impl From<ClaimError> for Value {
    fn from(e: ClaimError) -> Self {
        match e {
            ClaimError::Conflict(conflicts) => with_status(
                "conflict",
                json!({ "conflicts": conflicts, "message": "overlapping claims held by other agents" }),
            ),
            ClaimError::NotHeld { .. } | ClaimError::NoClaims(_) => not_found(e),
            ClaimError::NoPaths | ClaimError::InvalidTtl(_) => invalid(e),
            ClaimError::Cancelled => cancelled(e),
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

impl From<MessageError> for Value {
    fn from(e: MessageError) -> Self {
        match e {
            MessageError::NotFound(_) => not_found(e),
            other => invalid(other),
        }
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
    /// The lead policy (ADR-0027), shared by every session.
    lead: Arc<LeadPolicy>,
}

/// The result of a call whose caller is gone (ADR-0031). Nobody reads it,
/// so it skips [`TirithServer::finish`]: lost leases and inbox messages
/// stay for the agent's next call instead of riding on a reply that is
/// dropped. State written meanwhile reaches disk on the persister's tick.
fn unheard(outcome: Value) -> CallToolResult {
    CallToolResult::structured(outcome)
}

impl TirithServer {
    /// A handler over shared state and a persister.
    pub fn new(state: Arc<State>, persister: Arc<Persister>) -> Self {
        let lead = Arc::new(LeadPolicy::new(Arc::clone(&state)));
        Self {
            state,
            persister,
            lead,
        }
    }

    /// Waits for pending changes to reach disk and wraps `outcome` as a
    /// tool result. Calls that only renewed leases do not wait.
    ///
    /// The outcome travels once, as structured content. The text block is
    /// a one-line summary for clients that show only text; it is never the
    /// JSON again, which would double the tokens of every call.
    ///
    /// `agent` is the caller: if any lease it held has ended since it was
    /// last told, the outcome gains `lost` and the text line a warning,
    /// whatever tool was called (ADR-0015).
    async fn finish(
        &self,
        agent: Option<&str>,
        mut outcome: Value,
    ) -> Result<CallToolResult, McpError> {
        let mut not_persisted = None;
        if self.state.is_dirty()
            && let Err(error) = self.persister.flush().await
        {
            tracing::error!(%error, "failed to persist state");
            not_persisted = Some(error.to_string());
        }
        let mut warnings = Vec::new();
        // Every outcome is an object (see `with_status`); the piggybacks
        // below are keys on it.
        if let Value::Object(map) = &mut outcome {
            // The domain decision stands (it is what every other agent
            // sees), but the caller must know it may not survive a restart.
            if let Some(error) = not_persisted {
                warnings.push(format!("not persisted ({error})"));
                map.insert("persist_error".into(), Value::String(error));
            }
            if let Some(agent) = agent.and_then(|raw| AgentId::new(raw).ok()) {
                // A lease that ended while the agent may still be editing
                // is the one thing it must hear about whatever it asked.
                let lost = self.state.take_lost(&agent);
                if !lost.is_empty() {
                    warnings.push(format!("lost lease on {} path(s)", lost.len()));
                    map.insert("lost".into(), compact(to_value(&lost)));
                }
                // Messages from other agents ride on whatever was asked, once.
                let inbox = self.state.take_inbox(&agent);
                if !inbox.messages.is_empty() {
                    warnings.push(format!("inbox: {}", inbox.messages.len() + inbox.more));
                    let rows: Vec<Value> = inbox
                        .messages
                        .iter()
                        .map(|m| {
                            compact(json!({
                                "id": m.id,
                                "from": m.from,
                                "text": m.text.chars().take(INBOX_EXCERPT).collect::<String>(),
                                "at": m.at,
                            }))
                        })
                        .collect();
                    map.insert("inbox".into(), Value::Array(rows));
                    if inbox.more > 0 {
                        map.insert("inbox_more".into(), json!(inbox.more));
                    }
                }
            }
        }
        let text = if warnings.is_empty() {
            summary(&outcome)
        } else {
            format!("warning: {}; {}", warnings.join("; "), summary(&outcome))
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
        description = "Lease repo paths to yourself before editing them, atomically: every path becomes yours or none does. A directory covers its contents, and re-claiming a path you hold renews it. ok carries a brief of the unread notices, contracts, decisions and memory notes for those paths, and marks the notices shown as seen: read it before editing. conflict lists each overlapping lease; do not edit, or pass wait_secs so the daemon waits for the paths instead of you retrying. To see who holds a path without taking it use claims_list; to extend leases use renew.",
        annotations(destructive_hint = false)
    )]
    async fn claim(
        &self,
        Parameters(input): Parameters<ClaimInput>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run_async(async {
            let agent = agent(&input.agent)?;
            let paths = paths(&input.paths)?;
            let reason = input.reason.trim().to_owned();
            let result = match input.wait_secs.filter(|secs| *secs > 0) {
                Some(secs) => {
                    let wait = std::time::Duration::from_secs(secs);
                    // Stops waiting, claiming nothing, once the caller is gone.
                    let gone = hangup::caller_gone(&context);
                    self.state
                        .claim_waiting(
                            agent.clone(),
                            paths.clone(),
                            reason,
                            input.ttl_secs,
                            wait,
                            gone,
                        )
                        .await
                }
                None => self
                    .state
                    .claim(agent.clone(), paths.clone(), reason, input.ttl_secs),
            };
            // Repeated refusals escalate (ADR-0027); the lead policy counts
            // them without making this call wait.
            match &result {
                Ok(_) => self.lead.claim_granted(&agent, &paths),
                Err(ClaimError::Conflict(_)) => {
                    self.lead.claim_refused(&agent, &paths, input.reason.trim());
                }
                Err(_) => {}
            }
            let granted = result?;
            let mut value = to_value(&granted);
            // Whoever lost these paths a moment ago may have left them
            // half-edited; the new owner should know before touching them.
            let previous = self.state.previous_owners(&granted.new_paths);
            if !previous.is_empty()
                && let Some(object) = value.as_object_mut()
            {
                object.insert("previous_owner".to_owned(), compact(to_value(&previous)));
            }
            // The brief: what the agent needs to know about these paths,
            // delivered with the claim instead of asked for in four calls.
            if input.brief.unwrap_or(true) {
                let brief = self.state.brief(&agent, &paths);
                value = attach_brief(&value, &brief);
            }
            Ok(ok(value))
        })
        .await;
        if hangup::is_caller_gone(&context) {
            return Ok(unheard(outcome));
        }
        self.finish(Some(&input.agent), outcome).await
    }

    /// Release claims when done.
    #[tool(
        name = "release",
        description = "Give up leases so other agents can claim the paths; omit paths to release everything you hold. Use when you finish editing, not to extend time (renew) or to inspect (claims_list). Paths are matched exactly: releasing a file does not release a file#Symbol anchor inside it. not_found if you do not hold a named path, and then nothing is released.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    async fn release(
        &self,
        Parameters(input): Parameters<ReleaseInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = agent(&input.agent)?;
            let paths = input.paths.as_deref().map(paths).transpose()?;
            let released = self.state.release(&agent, paths)?;
            Ok(ok(json!({ "agent": agent, "released": released })))
        });
        self.finish(Some(&input.agent), outcome).await
    }

    /// Renew every lease you hold.
    #[tool(
        name = "renew",
        description = "Extend every lease you hold by its original TTL and restart the four-TTL age limit that ends a lease however active you are. Needed only during long work with no other call, because any tool call already renews; to take new paths use claim. Returns how many leases were renewed and the latest expiry; not_found when you hold none.",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
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
        self.finish(Some(&input.agent), outcome).await
    }

    /// List live claims.
    #[tool(
        name = "claims_list",
        description = "List live leases: by default yours plus any lease overlapping path, which is what a conflict check needs; all=true lists the whole board. Use before claim to see who holds a path and until when; for counts and daemon health use status. Expired leases are never listed. Newest first, 20 rows per page; pass next_before as before for older rows.",
        annotations(read_only_hint = true)
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
        self.finish(Some(&input.agent), outcome).await
    }

    /// Create a task.
    #[tool(
        name = "task_create",
        description = "Add a todo task to the shared board for any agent to pull. depends_on holds it back until those tasks are done, higher priority is pulled first, and paths lets task_pull keep agents off each other's files. Use to queue work, not to start it: task_pull assigns work and task_update changes a task. Returns the task with its id.",
        annotations(destructive_hint = false)
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
                NewTask::new(input.title)
                    .with_description(input.description.unwrap_or_default())
                    .with_priority(input.priority.unwrap_or(0))
                    .with_depends_on(depends_on)
                    .with_paths(opt_paths(input.paths.as_deref())?),
            )?;
            Ok(ok(json!({ "task": task })))
        });
        self.finish(Some(&input.agent), outcome).await
    }

    /// Pull the next unblocked task.
    #[tool(
        name = "task_pull",
        description = "Take the next task: the highest-priority todo whose dependencies are done becomes in_progress and yours, preferring tasks whose paths nobody else holds. Use to get work; to look without taking use task_list, and to change a named task use task_update. none means nothing is unblocked; a task that comes with waiting_on overlaps paths others hold, so coordinate before editing them. wait_secs waits server-side for work instead of you polling.",
        annotations(destructive_hint = false)
    )]
    async fn task_pull(
        &self,
        Parameters(input): Parameters<TaskPullInput>,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run_async(async {
            let agent = agent(&input.agent)?;
            // With wait_secs the lead policy waits while every todo task is
            // claimed or waits on dependencies (ADR-0028, revised).
            let pulled = match input.wait_secs.filter(|secs| *secs > 0) {
                Some(secs) => {
                    let wait = std::time::Duration::from_secs(secs);
                    // Stop waiting once the caller is gone (cancelled,
                    // disconnected, session closed), so no task goes to it.
                    // Nothing is assigned before the waiting pull returns.
                    tokio::select! {
                        biased;
                        () = hangup::caller_gone(&context) => {
                            return Err(cancelled(
                                "the caller went away while the pull waited; nothing was assigned",
                            ));
                        }
                        pulled = self.lead.task_pull_waiting(agent, wait) => pulled,
                    }
                }
                None => self.state.task_pull(agent),
            };
            Ok(match pulled {
                // A held task is still handed out, with what it waits for
                // (ADR-0028).
                Some(Pulled { task, waiting_on }) if !waiting_on.is_empty() => {
                    ok(json!({ "task": task, "waiting_on": waiting_on }))
                }
                Some(Pulled { task, .. }) => ok(json!({ "task": task })),
                None => with_status("none", json!({ "message": "no unblocked todo tasks" })),
            })
        })
        .await;
        if hangup::is_caller_gone(&context) {
            return Ok(unheard(outcome));
        }
        self.finish(Some(&input.agent), outcome).await
    }

    /// Update a task's status.
    #[tool(
        name = "task_update",
        description = "Change one task's status: done to finish it, todo to hand it back, blocked to park it with note as the reason, in_progress to take it by id. Changing a task another agent has in_progress is a conflict naming the owner, unless force, which is recorded in the task's notes. To get the next task without naming one use task_pull. Returns the task in full.",
        annotations(destructive_hint = false, idempotent_hint = true)
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
            let task =
                self.state
                    .task_update(agent.clone(), id, status, input.note.clone(), force)?;
            // Blocked with a note escalates; other statuses answer open
            // escalations (ADR-0027).
            self.lead.task_updated(&agent, &task, input.note.as_deref());
            Ok(ok(json!({ "task": task })))
        });
        self.finish(Some(&input.agent), outcome).await
    }

    /// List tasks.
    #[tool(
        name = "task_list",
        description = "List tasks on the board, filtered by status or owner, most recently updated first. Use to inspect the board without taking anything; to take work use task_pull. Rows are compact, 20 rows per page; pass next_before as before for older rows.",
        annotations(read_only_hint = true)
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
        self.finish(Some(&input.agent), outcome).await
    }

    /// Publish an interface contract.
    #[tool(
        name = "contract_publish",
        description = "Publish the shape of an interface (an endpoint, function, type, event or CLI) before either side implements it, with the paths that will consume it. Publishing an existing name creates a new version and sends its consumers a change notice by itself, so do not also call notice_publish. Omitting consumers on a republish keeps the list, and expected_version refuses the write with conflict if another agent published first. To read contracts use contract_get or contract_list.",
        annotations(destructive_hint = false)
    )]
    async fn contract_publish(
        &self,
        Parameters(input): Parameters<ContractPublishInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = agent(&input.agent)?;
            let kind: ContractKind = parse(&input.kind)?;
            let mut new = NewContract::new(input.name, kind, input.shape)
                .with_notes(input.notes.unwrap_or_default());
            if let Some(consumers) = input.consumers.as_deref().map(paths).transpose()? {
                new = new.with_consumers(consumers);
            }
            if let Some(version) = input.expected_version {
                new = new.with_expected_version(version);
            }
            let (published, notice) = self.state.contract_publish(agent, new)?;
            let notice_id = notice.as_ref().map(|n| n.id);
            if let Some(notice) = notice {
                self.lead.notice_published(&notice);
            }
            Ok(ok(json!({
                "contract": published.contract,
                "previous_version": published.previous_version,
                "notice_id": notice_id,
            })))
        });
        self.finish(Some(&input.agent), outcome).await
    }

    /// Fetch a contract.
    #[tool(
        name = "contract_get",
        description = "Fetch one contract in full by name, id or unique id prefix: its current shape and every earlier version. Use when a brief or contract_list names a contract you consume; to find contracts by path or kind use contract_list. not_found if there is none, and an ambiguous prefix is invalid.",
        annotations(read_only_hint = true)
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
        self.finish(Some(&input.agent), outcome).await
    }

    /// List contracts.
    #[tool(
        name = "contract_list",
        description = "List contracts, filtered by a consumer path or by kind, most recently published first. Use to find what a path depends on when you are not claiming it, since a claim's brief already lists them; shapes come from contract_get. Rows are compact, 20 rows per page; pass next_before as before for older rows.",
        annotations(read_only_hint = true)
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
        self.finish(Some(&input.agent), outcome).await
    }

    /// Publish a change notice.
    #[tool(
        name = "notice_publish",
        description = "Announce a change other files must react to: a rename, signature change, removal, move or behavior change, with from, to and the affected_paths. Use after changing something used outside the paths you claimed; not for interface shapes, which contract_publish versions and announces itself. Every other agent holding an affected path gets it in its inbox at once, and the rest see it in the brief of their next claim there.",
        annotations(destructive_hint = false)
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
            let mut new = NewNotice::new(kind, input.summary)
                .with_affected_paths(paths(&input.affected_paths)?);
            if let Some(from) = input.from {
                new = new.with_from(from);
            }
            if let Some(to) = input.to {
                new = new.with_to(to);
            }
            if let Some(id) = contract_id {
                new = new.with_contract_id(id);
            }
            let notice = self.state.notice_publish(agent, new)?;
            self.lead.notice_published(&notice);
            Ok(ok(json!({ "notice": notice })))
        });
        self.finish(Some(&input.agent), outcome).await
    }

    /// List change notices.
    #[tool(
        name = "notice_list",
        description = "List change notices for a path, since a time, or only those you have not been shown (unread=true), newest first. unread without path is scoped to the paths you hold, and all=true looks beyond them. Listing unread notices marks them seen by you, durably, so they do not come back. A claim's brief already delivers the unread notices for the paths it claims; use this for other paths or older pages. 20 rows per page; pass next_before as before for older rows.",
        annotations(destructive_hint = false)
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
        self.finish(Some(&input.agent), outcome).await
    }

    /// Record a decision.
    #[tool(
        name = "decision_record",
        description = "Record a settled project choice with its rationale, the alternatives considered and the paths it constrains, so no agent decides it again. It is written as one committed Markdown file under .tirith/decisions/ and appears in the brief of later claims on those paths. Use only once the choice is final; for discussion use message_send, and for lessons about code use memory_write. Returns the decision with its permalink.",
        annotations(destructive_hint = false)
    )]
    async fn decision_record(
        &self,
        Parameters(input): Parameters<DecisionRecordInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = agent(&input.agent)?;
            let decision = self.state.decision_record(
                agent,
                NewDecision::new(input.title, input.decision)
                    .with_rationale(input.rationale.unwrap_or_default())
                    .with_alternatives(input.alternatives.unwrap_or_default())
                    .with_affects_paths(opt_paths(input.affects_paths.as_deref())?),
            )?;
            Ok(ok(json!({ "decision": decision })))
        });
        self.finish(Some(&input.agent), outcome).await
    }

    /// List decisions.
    #[tool(
        name = "decision_list",
        description = "List recorded decisions, filtered by an affected path or a case-insensitive text query over title, decision and rationale, newest first. Use before decision_record so nothing is decided twice; a claim's brief already carries the decisions for the paths it claims. 20 rows per page; pass next_before as before for older rows.",
        annotations(read_only_hint = true)
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
        self.finish(Some(&input.agent), outcome).await
    }

    /// Server status.
    #[tool(
        name = "status",
        description = "Report the daemon's health: counts of claims, tasks, contracts, notices, decisions and notes, the swarm lead, the last persistence failure and the files skipped at load. verbose=true adds up to 50 active agents with what they hold. Use for a health check or to find the lead; it publishes nothing and takes no lease. For your own leases use claims_list, and for how to work with Tirith use guide.",
        annotations(read_only_hint = true)
    )]
    async fn status(
        &self,
        Parameters(input): Parameters<StatusInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = input.agent.as_deref().map(agent).transpose()?;
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
                "lead": report.lead.as_ref().map(|l| &l.agent),
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
                if let Some(lead) = &report.lead {
                    value["lead_expires_at"] = to_value(&lead.expires_at);
                }
                value["agents_truncated"] = Value::Bool(report.agents.len() > agents.len());
                value["agents"] = Value::Array(agents);
            }
            Ok(ok(value))
        });
        self.finish(input.agent.as_deref(), outcome).await
    }

    /// Write a memory note.
    #[tool(
        name = "memory_write",
        description = "Save a durable note about repository paths (a lesson, trap, handoff or research) as committed Markdown; the next agent that claims those paths gets its excerpt in the brief. A title that already exists updates that note in place, and permalink targets one explicitly. Pass the updated_at you read as if_updated_at so a concurrent edit returns conflict instead of being overwritten. Not for settled choices (decision_record) or talk between agents (message_send).",
        annotations(destructive_hint = false, idempotent_hint = true)
    )]
    async fn memory_write(
        &self,
        Parameters(input): Parameters<MemoryWriteInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = agent(&input.agent)?;
            let kind: Option<MemoryKind> = opt_parse(nonblank(input.kind.as_deref()))?;
            let mut new = NewMemory::new(input.title, input.body)
                .with_kind(kind.unwrap_or_default())
                .with_paths(opt_paths(input.paths.as_deref())?)
                .with_tags(input.tags.unwrap_or_default());
            if let Some(raw) = nonblank(input.permalink.as_deref()) {
                new = new.with_permalink(Permalink::parse(raw)?);
            }
            if let Some(at) = input.if_updated_at.as_deref().map(timestamp).transpose()? {
                new = new.with_if_updated_at(at);
            }
            let written = self.state.memory_write(agent, new)?;
            Ok(ok(json!({
                "note": written.note,
                "created": written.created,
            })))
        });
        self.finish(Some(&input.agent), outcome).await
    }

    /// Read one memory note.
    #[tool(
        name = "memory_read",
        description = "Read one memory note in full by permalink, id or exact title. It is the only tool that returns a note's body; depth 1 to 3 adds the notes linked to it, as digests. Use after memory_search or a claim's brief names the note; to find notes use memory_search. not_found if there is none.",
        annotations(read_only_hint = true)
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
        self.finish(Some(&input.agent), outcome).await
    }

    /// Search memory notes.
    #[tool(
        name = "memory_search",
        description = "Find memory notes by free text, path, kind, tag or time, best match first; with no query it lists the most recently updated, which is the call to make when a session starts. Rows are digests with a 160-character excerpt and a score, never bodies: read one with memory_read. Terms match as substrings, with no embeddings. truncated is true when limit hid matches.",
        annotations(read_only_hint = true)
    )]
    async fn memory_search(
        &self,
        Parameters(input): Parameters<MemorySearchInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = agent(&input.agent)?;
            let kind: Option<MemoryKind> = opt_parse(nonblank(input.kind.as_deref()))?;
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
        self.finish(Some(&input.agent), outcome).await
    }

    /// Delete a memory note.
    #[tool(
        name = "memory_delete",
        description = "Delete one memory note by permalink, id or exact title, permanently: its file is removed and does not return on restart. Use to retract a note that holds a secret or a wrong fact; to correct a note use memory_write, which updates in place. Returns a digest of what was removed; not_found if there is none.",
        annotations(destructive_hint = true, idempotent_hint = true)
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
        self.finish(Some(&input.agent), outcome).await
    }

    /// Send a message to another agent.
    #[tool(
        name = "message_send",
        description = "Send a short message to one agent by name, to every agent active in the last hour with *, or to the human queue with human. It is delivered once, as inbox on the recipient's next call of any tool, and reply_to threads an answer. Use for coordination talk; not for lasting knowledge (memory_write, decision_record) or for code changes others must react to (notice_publish). Messages are dropped after 24 hours.",
        annotations(destructive_hint = false)
    )]
    async fn message_send(
        &self,
        Parameters(input): Parameters<MessageSendInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = agent(&input.agent)?;
            let mut new = NewMessage::new(input.to, input.text)
                .with_paths(opt_paths(input.paths.as_deref())?);
            if let Some(raw) = nonblank(input.reply_to.as_deref()) {
                new = new.with_reply_to(self.state.resolve_message(raw)?);
            }
            let message = self.state.message_send(agent, new)?;
            // A message to `human` becomes a human queue item; one to an
            // agent answers its open escalations (ADR-0027).
            self.lead.message_sent(&message);
            Ok(ok(json!({ "message": compact(to_value(&message)) })))
        });
        self.finish(Some(&input.agent), outcome).await
    }

    /// List your messages.
    #[tool(
        name = "message_list",
        description = "List your own conversations, sent and received, newest first; filter by the other agent (with), since a time, or unread messages to you. New messages already arrive as inbox on every reply, so use this for history, not to poll. with=human shows the human queue thread. 20 rows per page; pass next_before as before for older rows.",
        annotations(read_only_hint = true)
    )]
    async fn message_list(
        &self,
        Parameters(input): Parameters<MessageListInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = agent(&input.agent)?;
            let filter = MessageFilter {
                with: opt_agent(input.with.as_deref())?,
                since: input.since.as_deref().map(timestamp).transpose()?,
                unread: input.unread.unwrap_or(false),
            };
            let before = cursor(input.before.as_deref(), MessageId::parse, MessageId::nil())?;
            let page = self.state.messages_page(
                &agent,
                &filter,
                before.as_ref(),
                clamp_limit(input.limit),
            );
            Ok(listing("messages", &page, |m| (m.at, m.id)))
        });
        self.finish(Some(&input.agent), outcome).await
    }

    /// How to work with Tirith.
    #[tool(
        name = "guide",
        description = "Explain what Tirith is for and how to work with it: the working loop from claim to release, the rules every reply follows (status, lost, inbox, paging), and which tool each situation calls for. Call it first in a session, or when unsure which tool fits; topic narrows it to one primitive with when to use each of its tools. Returns static guidance only: for the daemon's live state use status.",
        annotations(read_only_hint = true)
    )]
    async fn guide(
        &self,
        Parameters(input): Parameters<GuideInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            input.agent.as_deref().map(agent).transpose()?;
            guide::guide(input.topic.as_deref())
                .map(ok)
                .map_err(invalid)
        });
        self.finish(input.agent.as_deref(), outcome).await
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

    /// The generated `tools/list`, with every input schema slimmed and
    /// each tool's output schema attached (ADR-0034). The
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
                if let Some(output) = output_schemas::output_schema(&tool.name) {
                    tool.output_schema = Some(Arc::new(output));
                }
                tool
            })
            .collect();
        Ok(ListToolsResult::with_all_items(tools))
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
    /// The per-user daemon registry to announce this daemon in, for the
    /// menu bar tray (ADR-0019). `None` registers nowhere, which is what
    /// tests want; `tirith serve` passes
    /// [`Registry::default_path`](crate::registry::Registry::default_path).
    /// The daemon puts its entry back every minute if it goes missing.
    pub registry: Option<PathBuf>,
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
    registration: Option<Registration>,
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
        if let Some(registration) = self.registration.take() {
            registration.leave().await;
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
    // Each MCP request carries a hangup tied to its response, so a call
    // that waits notices a caller that disconnected (ADR-0031).
    let mcp = axum::routing::any_service(mcp).layer(axum::middleware::from_fn(hangup::layer));
    let router = dashboard::router(context).nest_service("/mcp", mcp);

    let info = DaemonInfo {
        url: mcp_url(addr),
        dashboard_url: dashboard_url(addr),
        pid: std::process::id(),
        started_at: state.started_at(),
        version: VERSION.to_owned(),
    };
    store.write_daemon_info(&info)?;

    // Announce this daemon to the tray's registry, and keep it there while
    // it runs. Best effort: a registry that cannot be written must not stop
    // a daemon.
    let registration = match options.registry {
        Some(path) => {
            let entry = DaemonEntry {
                root: options.repo_root.clone(),
                url: info.url,
                dashboard_url: info.dashboard_url,
                pid: info.pid,
                version: info.version,
                started_at: info.started_at,
            };
            Some(Registration::start(path, entry, REREGISTER_EVERY).await)
        }
        None => None,
    };

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
        registration,
        shutdown: Some(tx),
        task,
    })
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
                    "agent": {"description": "Your agent name.", "type": "string"},
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
    fn the_guide_and_the_router_name_the_same_tools() {
        let mut routed: Vec<String> = TirithServer::tool_router()
            .list_all()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect();
        routed.sort_unstable();
        assert_eq!(routed, guide::tool_names());
    }

    #[test]
    fn every_routed_tool_declares_an_output_schema() {
        for tool in TirithServer::tool_router().list_all() {
            assert!(
                output_schemas::output_schema(&tool.name).is_some(),
                "{} has no output schema",
                tool.name
            );
        }
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
