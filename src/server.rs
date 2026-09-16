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

use chrono::{DateTime, Utc};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Implementation, ServerCapabilities, ServerConfig};
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
use crate::notices::{NewNotice, NoticeError, NoticeFilter, NoticeKind};
use crate::state::State;
use crate::store::{DaemonInfo, JsonStore, Persister, StoreError};
use crate::tasks::{NewTask, TaskError, TaskStatus};
use crate::types::{
    AgentId, ContractId, IdError, NoticeId, PathError, RepoPath, TaskId, parse_paths,
};

/// Crate version reported to clients and the dashboard.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Default bind address.
pub const DEFAULT_BIND: &str = "127.0.0.1:7477";

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
    /// Only claims overlapping this path.
    #[serde(default)]
    pub path: Option<String>,
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
    /// Only notices you have not published or acknowledged.
    #[serde(default)]
    pub unread: Option<bool>,
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
}

/// Input for `status`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct StatusInput {
    /// Your stable agent name, optional here.
    #[serde(default)]
    pub agent: Option<String>,
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

fn to_value<T: Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
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
        .map_err(|e| invalid(format!("since must be RFC 3339: {e}")))
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
        match e {
            TaskError::NotFound(_) | TaskError::UnknownDependency(_) => not_found(e),
            TaskError::EmptyTitle | TaskError::UnknownStatus(_) => invalid(e),
        }
    }
}

impl From<ContractError> for Value {
    fn from(e: ContractError) -> Self {
        match e {
            ContractError::NotFound(_) => not_found(e),
            ContractError::EmptyName | ContractError::UnknownKind(_) => invalid(e),
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

    /// Persists pending changes and wraps `outcome` as a tool result.
    async fn finish(&self, outcome: Value) -> Result<CallToolResult, McpError> {
        if let Some(snapshot) = self.state.take_dirty() {
            if let Err(error) = self.persister.persist(snapshot).await {
                tracing::error!(%error, "failed to persist state");
            }
        }
        Ok(CallToolResult::structured(outcome))
    }
}

#[tool_router]
impl TirithServer {
    /// Claim files or directories before editing them.
    #[tool(
        name = "claim",
        description = "Claim repo-relative files or directories before editing them. A directory covers everything beneath it. Returns status `ok` with the lease expiry, or `conflict` listing who holds each overlapping path, why, and until when. Atomic: on conflict nothing is claimed. Re-claiming paths you hold renews them."
    )]
    async fn claim(
        &self,
        Parameters(input): Parameters<ClaimInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = agent(&input.agent)?;
            let paths = paths(&input.paths)?;
            let granted =
                self.state
                    .claim(agent, paths, input.reason.trim().to_owned(), input.ttl_secs)?;
            Ok(ok(to_value(&granted)))
        });
        self.finish(outcome).await
    }

    /// Release claims when done.
    #[tool(
        name = "release",
        description = "Release paths you claimed, or everything you hold when `paths` is omitted. Returns `not_found` if you try to release a path you do not hold; nothing is released in that case."
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
    #[tool(
        name = "renew",
        description = "Extend every lease you hold by its TTL. Any other tool call also renews, so this is only needed during long silent work."
    )]
    async fn renew(
        &self,
        Parameters(input): Parameters<AgentInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = agent(&input.agent)?;
            let claims = self.state.renew(&agent)?;
            Ok(ok(json!({ "agent": agent, "claims": claims })))
        });
        self.finish(outcome).await
    }

    /// List live claims.
    #[tool(
        name = "claims_list",
        description = "List live claims, optionally only those overlapping `path`. Expired leases are already removed."
    )]
    async fn claims_list(
        &self,
        Parameters(input): Parameters<ClaimsListInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = agent(&input.agent)?;
            let path = opt_path(input.path.as_deref())?;
            let claims = self.state.claims(Some(&agent), path.as_ref());
            Ok(ok(json!({ "count": claims.len(), "claims": claims })))
        });
        self.finish(outcome).await
    }

    /// Create a task.
    #[tool(
        name = "task_create",
        description = "Add a task to the board in the `todo` state. `depends_on` lists task ids that must be done before it can be pulled."
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
                .map(|s| TaskId::parse(s))
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
        description = "Take the highest-priority `todo` task whose dependencies are all done, assign it to you, and mark it `in_progress`. Returns status `none` when nothing is unblocked."
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
        description = "Set a task's status to `todo`, `in_progress`, `blocked`, or `done`, optionally appending a note. Setting `in_progress` makes you the owner."
    )]
    async fn task_update(
        &self,
        Parameters(input): Parameters<TaskUpdateInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = agent(&input.agent)?;
            let id = TaskId::parse(&input.task_id)?;
            let status: TaskStatus = parse(&input.status)?;
            let task = self.state.task_update(agent, id, status, input.note)?;
            Ok(ok(json!({ "task": task })))
        });
        self.finish(outcome).await
    }

    /// List tasks.
    #[tool(
        name = "task_list",
        description = "List tasks, optionally filtered by `status` and `owner`."
    )]
    async fn task_list(
        &self,
        Parameters(input): Parameters<TaskListInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = agent(&input.agent)?;
            let status: Option<TaskStatus> = opt_parse(input.status.as_deref())?;
            let owner = opt_agent(input.owner.as_deref())?;
            let tasks = self.state.tasks(Some(&agent), status, owner.as_ref());
            Ok(ok(json!({ "count": tasks.len(), "tasks": tasks })))
        });
        self.finish(outcome).await
    }

    /// Publish an interface contract.
    #[tool(
        name = "contract_publish",
        description = "Publish the shape of an interface before implementing either side of it. Publishing an existing name creates a new version and automatically emits a `contract` notice to its consumers."
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
                    consumers: opt_paths(input.consumers.as_ref())?,
                    notes: input.notes.unwrap_or_default(),
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
        description = "Fetch a contract by name or id, including its version history."
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
        description = "List contracts, optionally only those consumed by `path` or of a given `kind`. Read this before touching code that calls or implements an interface."
    )]
    async fn contract_list(
        &self,
        Parameters(input): Parameters<ContractListInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = agent(&input.agent)?;
            let path = opt_path(input.path.as_deref())?;
            let kind: Option<ContractKind> = opt_parse(input.kind.as_deref())?;
            let contracts = self.state.contracts(Some(&agent), path.as_ref(), kind);
            Ok(ok(
                json!({ "count": contracts.len(), "contracts": contracts }),
            ))
        });
        self.finish(outcome).await
    }

    /// Publish a change notice.
    #[tool(
        name = "notice_publish",
        description = "Announce a rename, signature change, removal, move, or behavior change and the paths that must react to it. Dependents read these with `notice_list` before acting."
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
        description = "List change notices, optionally only those affecting `path`, published since an RFC 3339 timestamp, or unread by you. Call this for the paths you are about to edit."
    )]
    async fn notice_list(
        &self,
        Parameters(input): Parameters<NoticeListInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = agent(&input.agent)?;
            let filter = NoticeFilter {
                path: opt_path(input.path.as_deref())?,
                since: input.since.as_deref().map(timestamp).transpose()?,
                unread_by: input.unread.unwrap_or(false).then(|| agent.clone()),
            };
            let notices = self.state.notices(Some(&agent), &filter);
            Ok(ok(json!({ "count": notices.len(), "notices": notices })))
        });
        self.finish(outcome).await
    }

    /// Acknowledge a notice.
    #[tool(
        name = "notice_ack",
        description = "Mark a notice as handled by you so it no longer shows up as unread."
    )]
    async fn notice_ack(
        &self,
        Parameters(input): Parameters<NoticeAckInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = agent(&input.agent)?;
            let id = NoticeId::parse(&input.notice_id)?;
            let notice = self.state.notice_ack(agent, id)?;
            Ok(ok(json!({ "notice": notice })))
        });
        self.finish(outcome).await
    }

    /// Record a decision.
    #[tool(
        name = "decision_record",
        description = "Record a settled choice with its rationale and alternatives so no agent re-decides it."
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
        description = "List recorded decisions, optionally only those affecting `path` or matching a text `query`. Check before making a design choice."
    )]
    async fn decision_list(
        &self,
        Parameters(input): Parameters<DecisionListInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = agent(&input.agent)?;
            let path = opt_path(input.path.as_deref())?;
            let decisions =
                self.state
                    .decisions(Some(&agent), path.as_ref(), input.query.as_deref());
            Ok(ok(
                json!({ "count": decisions.len(), "decisions": decisions }),
            ))
        });
        self.finish(outcome).await
    }

    /// Server status.
    #[tool(
        name = "status",
        description = "Counts of claims, tasks, contracts, notices, and decisions, plus which agents hold what."
    )]
    async fn status(
        &self,
        Parameters(input): Parameters<StatusInput>,
    ) -> Result<CallToolResult, McpError> {
        let outcome = run(|| {
            let agent = opt_agent(input.agent.as_deref())?;
            let report = self.state.status(agent.as_ref());
            let mut value = to_value(&report);
            if let Value::Object(map) = &mut value {
                map.insert("version".into(), Value::String(VERSION.to_owned()));
                map.insert(
                    "persist_error".into(),
                    to_value(&self.persister.last_error()),
                );
            }
            Ok(ok(value))
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

    /// The dashboard URL.
    pub fn dashboard_url(&self) -> String {
        dashboard_url(self.addr)
    }

    /// The shared state, for tests and embedding.
    pub fn state(&self) -> &Arc<State> {
        &self.state
    }

    /// Stops accepting connections, waits for the server task, and clears
    /// the recorded daemon address.
    pub async fn shutdown(mut self) -> Result<(), ServeError> {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        let result = (&mut self.task).await;
        self.store.clear_daemon_info()?;
        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(ServeError::Server(e.to_string())),
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
    let persister = Arc::new(Persister::new(Arc::clone(&store)));
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
        persister,
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
        shutdown: Some(tx),
        task,
    })
}
