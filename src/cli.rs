//! Command-line interface: `serve` runs the daemon; every other subcommand
//! is a thin MCP client call rendered for humans.

// The CLI's job is printing to stdout.
#![allow(clippy::print_stdout)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use clap::{Args, Parser, Subcommand};
use serde_json::{Value, json};
use tirith::client;
use tirith::server::{self, DEFAULT_BIND, ServeOptions};
use tirith::stdio;
use tirith::store::JsonStore;
use tirith::update;

const DEFAULT_URL: &str = "http://127.0.0.1:7477/mcp";

/// Coordination server for parallel coding agents.
#[derive(Debug, Parser)]
#[command(name = "tirith", version, about, propagate_version = true)]
struct Cli {
    /// MCP endpoint of the daemon. Defaults to the address recorded in
    /// `.tirith/runtime/daemon.json`, then `http://127.0.0.1:7477/mcp`.
    #[arg(long, global = true, env = "TIRITH_URL")]
    url: Option<String>,

    /// Your agent name for coordination calls.
    #[arg(
        short,
        long,
        global = true,
        env = "TIRITH_AGENT",
        default_value = "cli"
    )]
    agent: String,

    /// Repository root whose .tirith/ directory is used.
    #[arg(long, global = true, default_value = ".")]
    root: PathBuf,

    /// Print raw JSON instead of a human summary.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

/// Paging for list subcommands. Lists are newest first, 20 rows by default.
#[derive(Debug, Args)]
struct Page {
    /// Rows to return (default 20, max 200).
    #[arg(long)]
    limit: Option<usize>,
    /// Only rows older than this: an RFC 3339 time, or the `next_before`
    /// value a truncated listing printed.
    #[arg(long)]
    before: Option<String>,
}

impl Page {
    fn args(&self) -> Value {
        json!({ "limit": self.limit, "before": self.before })
    }
}

/// Merges two JSON objects; `extra` wins on duplicate keys.
fn merge(mut base: Value, extra: Value) -> Value {
    if let (Value::Object(base), Value::Object(extra)) = (&mut base, extra) {
        base.extend(extra);
    }
    base
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the daemon for the repository at --root.
    Serve {
        /// Address to bind.
        #[arg(long, default_value = DEFAULT_BIND)]
        bind: SocketAddr,
        /// Seconds an in-progress task's owner may be silent before the
        /// task returns to todo (0 disables).
        #[arg(long, default_value_t = tirith::state::DEFAULT_TASK_ORPHAN_SECS)]
        task_orphan_secs: u64,
        /// Do not start the menu bar tray (macOS) alongside this daemon.
        /// `TIRITH_NO_TRAY=1` does the same, for tests and tooling; empty,
        /// 0, false, no, n, f and off leave the tray on.
        #[arg(long, env = "TIRITH_NO_TRAY", value_parser = clap::builder::FalseyValueParser::new())]
        no_tray: bool,
    },
    /// Serve MCP over stdin/stdout for clients that spawn servers themselves,
    /// starting the repository's daemon if none is running.
    Stdio {
        /// Address to bind if the daemon has to be started.
        #[arg(long, default_value = DEFAULT_BIND)]
        bind: String,
    },
    /// Show a menu bar icon listing every Tirith daemon on this machine;
    /// clicking a daemon opens its dashboard. Stays until Quit.
    #[cfg(all(feature = "tray", target_os = "macos"))]
    Tray,
    /// Update tirith to the latest release (or a given version) in place.
    Update {
        /// Install this version instead of the latest, e.g. 0.2.0 or v0.2.0.
        #[arg(long = "to", value_name = "VERSION")]
        to: Option<String>,
        /// Only report whether an update is available. Exits 1 if one is.
        #[arg(long)]
        check: bool,
    },
    /// Show daemon status and who holds what.
    Status,
    /// Explain what Tirith is for and how to work with it.
    Guide {
        /// One primitive to explain: claims, tasks, contracts, notices,
        /// decisions, memory, messages, lead, or server.
        topic: Option<String>,
    },
    /// Claim files or directories before editing them.
    Claim {
        /// Why you need them; shown to anyone refused.
        #[arg(short, long)]
        reason: String,
        /// Lease length in seconds (default 600, max 3600).
        #[arg(long)]
        ttl: Option<u64>,
        /// Repo-relative paths.
        #[arg(required = true)]
        paths: Vec<String>,
    },
    /// Release claims. With no paths, releases everything you hold.
    Release { paths: Vec<String> },
    /// Renew every lease you hold.
    Renew,
    /// List live claims: yours, plus any overlapping --path.
    Claims {
        /// Only claims overlapping this path.
        #[arg(long)]
        path: Option<String>,
        /// The whole board, not just your own claims.
        #[arg(long)]
        all: bool,
        #[command(flatten)]
        page: Page,
    },
    /// Task board.
    Task {
        #[command(subcommand)]
        command: TaskCommand,
    },
    /// Interface contracts.
    Contract {
        #[command(subcommand)]
        command: ContractCommand,
    },
    /// Change notices.
    Notice {
        #[command(subcommand)]
        command: NoticeCommand,
    },
    /// Decisions log.
    Decision {
        #[command(subcommand)]
        command: DecisionCommand,
    },
    /// Memory notes scoped to repository paths.
    Memory {
        #[command(subcommand)]
        command: MemoryCommand,
    },
    /// Messages to and from other agents, delivered on their next call.
    Message {
        #[command(subcommand)]
        command: MessageCommand,
    },
    /// The swarm lead and its policy's decision log (ADR-0027).
    Lead {
        #[command(subcommand)]
        command: LeadCommand,
    },
    /// List the tools the daemon exposes.
    Tools,
    /// Call any tool with raw JSON arguments.
    Call {
        /// Tool name.
        tool: String,
        /// JSON object of arguments. `agent` is filled in if missing.
        #[arg(default_value = "{}")]
        arguments: String,
    },
}

#[derive(Debug, Subcommand)]
enum TaskCommand {
    /// Add a task.
    Create {
        /// Short imperative title.
        title: String,
        #[arg(short, long, default_value = "")]
        description: String,
        /// Higher pulls first.
        #[arg(short, long, default_value_t = 0)]
        priority: i32,
        /// Task ids that must be done first.
        #[arg(long = "depends-on")]
        depends_on: Vec<String>,
        /// Paths the task will likely touch.
        #[arg(long = "path")]
        paths: Vec<String>,
    },
    /// Pull the next unblocked task for yourself.
    Pull,
    /// Change a task's status.
    Update {
        task_id: String,
        /// One of `todo`, `in_progress`, `blocked`, `done`.
        status: String,
        /// Note to append; the reason when blocking.
        #[arg(short, long)]
        note: Option<String>,
        /// Take or close a task another agent has in progress.
        #[arg(long)]
        force: bool,
    },
    /// List tasks.
    List {
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        owner: Option<String>,
        #[command(flatten)]
        page: Page,
    },
}

#[derive(Debug, Subcommand)]
enum ContractCommand {
    /// Publish a contract or a new version of one.
    Publish {
        /// Unique name, e.g. "POST /api/sessions".
        name: String,
        /// http, function, type, event, cli, or other.
        #[arg(short, long, default_value = "other")]
        kind: String,
        /// The shape as a JSON document.
        #[arg(short, long)]
        shape: String,
        /// Paths expected to consume it.
        #[arg(long = "consumer")]
        consumers: Vec<String>,
        #[arg(short, long, default_value = "")]
        notes: String,
        /// Refuse to publish unless the contract is at this version now
        /// (0 for "does not exist yet"). Omit --consumer to keep the list.
        #[arg(long, value_name = "N")]
        expected_version: Option<u32>,
    },
    /// Fetch a contract by name or id.
    Get { name: String },
    /// List contracts.
    List {
        #[arg(long)]
        path: Option<String>,
        #[arg(long)]
        kind: Option<String>,
        #[command(flatten)]
        page: Page,
    },
}

#[derive(Debug, Subcommand)]
enum NoticeCommand {
    /// Publish a change notice.
    Publish {
        /// rename, signature, removed, moved, or behavior.
        #[arg(short, long)]
        kind: String,
        /// One-line summary.
        summary: String,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        /// Affected paths.
        #[arg(long = "path", required = true)]
        paths: Vec<String>,
        #[arg(long)]
        contract: Option<String>,
    },
    /// List notices.
    List {
        #[arg(long)]
        path: Option<String>,
        /// RFC 3339 timestamp.
        #[arg(long)]
        since: Option<String>,
        /// Only notices not yet delivered to you.
        #[arg(long)]
        unread: bool,
        /// Every notice, not only those on the paths you hold.
        #[arg(long)]
        all: bool,
        #[command(flatten)]
        page: Page,
    },
}

#[derive(Debug, Subcommand)]
enum DecisionCommand {
    /// Record a decision.
    Record {
        title: String,
        /// What was decided.
        #[arg(short, long)]
        decision: String,
        #[arg(short, long, default_value = "")]
        rationale: String,
        #[arg(long = "alternative")]
        alternatives: Vec<String>,
        #[arg(long = "path")]
        paths: Vec<String>,
    },
    /// List decisions.
    List {
        #[arg(long)]
        path: Option<String>,
        #[arg(long)]
        query: Option<String>,
        #[command(flatten)]
        page: Page,
    },
}

#[derive(Debug, Subcommand)]
enum MemoryCommand {
    /// Write a note, or update the note at --permalink.
    ///
    /// The body comes from --body, from --file (`-` for stdin), or from
    /// stdin when neither is given, so multi-line Markdown needs no shell
    /// quoting: `tirith memory write "Title" --path src/store.rs < note.md`.
    Write {
        title: String,
        /// The Markdown body, inline.
        #[arg(short, long, conflicts_with = "file")]
        body: Option<String>,
        /// Read the body from this file; `-` reads stdin.
        #[arg(short, long, value_name = "PATH")]
        file: Option<PathBuf>,
        /// fact, lesson, gotcha, handoff, research, or note (the default).
        #[arg(short, long)]
        kind: Option<String>,
        /// Repo-relative paths the note is about.
        #[arg(long = "path")]
        paths: Vec<String>,
        #[arg(long = "tag")]
        tags: Vec<String>,
        /// Target an existing note instead of creating one.
        #[arg(long)]
        permalink: Option<String>,
        /// Only update if the note's `updated_at` still equals this RFC 3339
        /// time; otherwise the write is a conflict and nothing is lost.
        #[arg(long, value_name = "RFC3339")]
        if_updated_at: Option<String>,
    },
    /// Delete one note by permalink, id, or exact title.
    Delete { name: String },
    /// Read one note by permalink, id, or exact title.
    Read {
        name: String,
        /// Also return notes related within this many relation hops.
        #[arg(long, default_value_t = 0)]
        depth: usize,
    },
    /// Search notes. With no query, lists the most recently updated.
    Search {
        query: Option<String>,
        /// Only notes whose paths overlap this path.
        #[arg(long)]
        path: Option<String>,
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        tag: Option<String>,
        /// Only notes updated at or after this RFC 3339 timestamp.
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        limit: Option<usize>,
    },
}

#[derive(Debug, Subcommand)]
enum LeadCommand {
    /// Show the lead and its newest decisions, newest first, from the
    /// daemon's `/api/lead`.
    Log {
        /// Rows to show (default 50, max 500).
        #[arg(long)]
        limit: Option<usize>,
        /// Only decisions on this event, e.g. `task_requested`.
        #[arg(long)]
        event: Option<String>,
    },
    /// Show the human queue, most agents blocked first, from `/api/human`:
    /// messages sent to `human`, and escalations raised while nobody held
    /// `.tirith/lead`, not yet answered.
    Human {
        #[command(subcommand)]
        command: Option<HumanCommand>,
    },
}

#[derive(Debug, Subcommand)]
enum HumanCommand {
    /// Mark a human queue item answered. With `--reply`, its sender gets
    /// the reply as a message from `human` on their next call.
    Done {
        /// The item's id, as `tirith lead human` prints it (`#7` or `7`).
        id: String,
        /// What to tell the sender, at most 1000 characters.
        #[arg(long)]
        reply: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum MessageCommand {
    /// Send a message; it reaches the recipient on their next tool call.
    Send {
        /// An agent name, `*` for every agent active in the last hour, or
        /// `human` for the human queue.
        to: String,
        /// The message, at most 1000 characters.
        text: String,
        /// The message this answers (id or unique prefix).
        #[arg(long)]
        reply_to: Option<String>,
        /// Paths the message is about.
        #[arg(long = "path")]
        paths: Vec<String>,
    },
    /// List your messages, newest first.
    List {
        /// Only the conversation with this agent.
        #[arg(long)]
        with: Option<String>,
        /// RFC 3339 timestamp.
        #[arg(long)]
        since: Option<String>,
        /// Only messages to you that you have not received yet.
        #[arg(long)]
        unread: bool,
        #[command(flatten)]
        page: Page,
    },
}

/// Common options every remote command needs.
#[derive(Debug)]
struct Remote {
    url: String,
    agent: String,
    json: bool,
}

/// Parses arguments and runs the chosen command.
pub(crate) async fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    let url = match cli.url {
        Some(url) => url,
        None => JsonStore::new(&cli.root)
            .read_daemon_info()
            .ok()
            .flatten()
            .map_or_else(|| DEFAULT_URL.to_owned(), |info| info.url),
    };
    let remote = Remote {
        url,
        agent: cli.agent,
        json: cli.json,
    };
    // The local commands return from here; everything else is a tool call.
    let (tool, arguments) = match cli.command {
        Command::Serve {
            bind,
            task_orphan_secs,
            no_tray,
        } => return serve(bind, task_orphan_secs, no_tray, cli.root).await,
        Command::Stdio { bind } => return stdio_shim(bind, cli.root).await,
        Command::Update { to, check } => return self_update(to, check, &cli.root).await,
        #[cfg(all(feature = "tray", target_os = "macos"))]
        Command::Tray => {
            // The tray owns the main thread; run it on a plain thread's
            // worth of stack outside the async runtime's control.
            return tokio::task::block_in_place(tirith::tray::run)
                .map(|()| ExitCode::SUCCESS)
                .map_err(anyhow::Error::from);
        }
        Command::Tools => return tools(&remote).await,
        Command::Lead {
            command: LeadCommand::Log { limit, event },
        } => return lead_log(&remote, limit, event).await,
        Command::Lead {
            command: LeadCommand::Human { command: None },
        } => return lead_human(&remote).await,
        Command::Lead {
            command:
                LeadCommand::Human {
                    command: Some(HumanCommand::Done { id, reply }),
                },
        } => return lead_human_done(&remote, &id, reply).await,
        Command::Call { tool, arguments } => {
            let mut value: Value =
                serde_json::from_str(&arguments).context("arguments must be a JSON object")?;
            if let Value::Object(map) = &mut value {
                map.entry("agent")
                    .or_insert_with(|| Value::String(remote.agent.clone()));
            }
            (tool, value)
        }
        // A human is reading, so ask for the per-agent rows.
        Command::Status => ("status".to_owned(), json!({ "verbose": true })),
        Command::Guide { topic } => ("guide".to_owned(), json!({ "topic": topic })),
        Command::Claim { reason, ttl, paths } => (
            "claim".to_owned(),
            json!({ "paths": paths, "reason": reason, "ttl_secs": ttl }),
        ),
        Command::Release { paths } => (
            "release".to_owned(),
            json!({ "paths": if paths.is_empty() { Value::Null } else { json!(paths) } }),
        ),
        Command::Renew => ("renew".to_owned(), json!({})),
        Command::Claims { path, all, page } => (
            "claims_list".to_owned(),
            merge(json!({ "path": path, "all": all }), page.args()),
        ),
        Command::Task { command } => task_call(command),
        Command::Contract { command } => contract_call(command)?,
        Command::Notice { command } => notice_call(command),
        Command::Decision { command } => decision_call(command),
        Command::Memory { command } => memory_call(command)?,
        Command::Message { command } => message_call(command),
    };
    call(&remote, &tool, arguments).await
}

fn task_call(command: TaskCommand) -> (String, Value) {
    match command {
        TaskCommand::Create {
            title,
            description,
            priority,
            depends_on,
            paths,
        } => (
            "task_create".to_owned(),
            json!({ "title": title, "description": description, "priority": priority, "depends_on": depends_on, "paths": paths }),
        ),
        TaskCommand::Pull => ("task_pull".to_owned(), json!({})),
        TaskCommand::Update {
            task_id,
            status,
            note,
            force,
        } => (
            "task_update".to_owned(),
            json!({ "task_id": task_id, "status": status, "note": note, "force": force }),
        ),
        TaskCommand::List {
            status,
            owner,
            page,
        } => (
            "task_list".to_owned(),
            merge(json!({ "status": status, "owner": owner }), page.args()),
        ),
    }
}

fn contract_call(command: ContractCommand) -> Result<(String, Value)> {
    Ok(match command {
        ContractCommand::Publish {
            name,
            kind,
            shape,
            consumers,
            notes,
            expected_version,
        } => {
            let shape: Value = serde_json::from_str(&shape).context("--shape must be JSON")?;
            // No --consumer means "keep the existing list" on a republish.
            let consumers = if consumers.is_empty() {
                Value::Null
            } else {
                json!(consumers)
            };
            (
                "contract_publish".to_owned(),
                json!({ "name": name, "kind": kind, "shape": shape, "consumers": consumers, "notes": notes, "expected_version": expected_version }),
            )
        }
        ContractCommand::Get { name } => ("contract_get".to_owned(), json!({ "name": name })),
        ContractCommand::List { path, kind, page } => (
            "contract_list".to_owned(),
            merge(json!({ "path": path, "kind": kind }), page.args()),
        ),
    })
}

fn notice_call(command: NoticeCommand) -> (String, Value) {
    match command {
        NoticeCommand::Publish {
            kind,
            summary,
            from,
            to,
            paths,
            contract,
        } => (
            "notice_publish".to_owned(),
            json!({ "kind": kind, "summary": summary, "from": from, "to": to, "affected_paths": paths, "contract_id": contract }),
        ),
        NoticeCommand::List {
            path,
            since,
            unread,
            all,
            page,
        } => (
            "notice_list".to_owned(),
            merge(
                json!({ "path": path, "since": since, "unread": unread, "all": all }),
                page.args(),
            ),
        ),
    }
}

fn decision_call(command: DecisionCommand) -> (String, Value) {
    match command {
        DecisionCommand::Record {
            title,
            decision,
            rationale,
            alternatives,
            paths,
        } => (
            "decision_record".to_owned(),
            json!({ "title": title, "decision": decision, "rationale": rationale, "alternatives": alternatives, "affects_paths": paths }),
        ),
        DecisionCommand::List { path, query, page } => (
            "decision_list".to_owned(),
            merge(json!({ "path": path, "query": query }), page.args()),
        ),
    }
}

fn memory_call(command: MemoryCommand) -> Result<(String, Value)> {
    Ok(match command {
        MemoryCommand::Write {
            title,
            body,
            file,
            kind,
            paths,
            tags,
            permalink,
            if_updated_at,
        } => {
            let body = match (body, file) {
                (Some(body), _) => body,
                (None, Some(path)) if path.as_os_str() == "-" => read_stdin()?,
                (None, Some(path)) => std::fs::read_to_string(&path)
                    .with_context(|| format!("reading {}", path.display()))?,
                (None, None) => read_stdin()?,
            };
            (
                "memory_write".to_owned(),
                json!({ "title": title, "body": body, "kind": kind, "paths": paths, "tags": tags, "permalink": permalink, "if_updated_at": if_updated_at }),
            )
        }
        MemoryCommand::Delete { name } => ("memory_delete".to_owned(), json!({ "name": name })),
        MemoryCommand::Read { name, depth } => (
            "memory_read".to_owned(),
            json!({ "name": name, "depth": depth }),
        ),
        MemoryCommand::Search {
            query,
            path,
            kind,
            tag,
            since,
            limit,
        } => (
            "memory_search".to_owned(),
            json!({ "query": query, "path": path, "kind": kind, "tag": tag, "since": since, "limit": limit }),
        ),
    })
}

fn message_call(command: MessageCommand) -> (String, Value) {
    match command {
        MessageCommand::Send {
            to,
            text,
            reply_to,
            paths,
        } => (
            "message_send".to_owned(),
            json!({ "to": to, "text": text, "reply_to": reply_to, "paths": paths }),
        ),
        MessageCommand::List {
            with,
            since,
            unread,
            page,
        } => (
            "message_list".to_owned(),
            merge(
                json!({ "with": with, "since": since, "unread": unread }),
                page.args(),
            ),
        ),
    }
}

fn read_stdin() -> Result<String> {
    let body =
        std::io::read_to_string(std::io::stdin()).context("reading the note body from stdin")?;
    if body.trim().is_empty() {
        anyhow::bail!("the note body is empty; pass --body, --file, or pipe Markdown on stdin");
    }
    Ok(body)
}

async fn serve(
    bind: SocketAddr,
    task_orphan_secs: u64,
    no_tray: bool,
    root: PathBuf,
) -> Result<ExitCode> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();
    let root = root
        .canonicalize()
        .with_context(|| format!("repository root {}", root.display()))?;
    // Announce the daemon for the menu bar tray; a machine without a
    // resolvable state directory just runs without one.
    let registry = match tirith::registry::Registry::default_path() {
        Ok(path) => Some(path),
        Err(error) => {
            eprintln!("warning: {error}; the tray will not see this daemon");
            None
        }
    };
    let handle = server::start(ServeOptions {
        bind,
        repo_root: root.clone(),
        clock: None,
        registry,
    })
    .await?;
    handle.set_task_orphan_secs(task_orphan_secs);
    #[cfg(all(feature = "tray", target_os = "macos"))]
    if !no_tray && let Err(error) = tirith::tray::launch_if_absent() {
        eprintln!("warning: could not start the menu bar tray: {error}");
    }
    #[cfg(not(all(feature = "tray", target_os = "macos")))]
    let _ = no_tray;
    println!("tirith {} serving {}", server::VERSION, root.display());
    println!("  mcp       {}", handle.mcp_url());
    println!("  dashboard {}", handle.dashboard_url());
    let why = wait_for_stop(&root).await?;
    eprintln!("shutting down: {why}");
    handle.shutdown().await?;
    Ok(ExitCode::SUCCESS)
}

/// Resolves when the daemon should stop: on ctrl-c, on SIGTERM, or when
/// the repository root it serves no longer exists (deleted or unmounted),
/// so a daemon never outlives its repository.
async fn wait_for_stop(root: &Path) -> Result<&'static str> {
    let mut root_check = tokio::time::interval(std::time::Duration::from_secs(2));
    root_check.tick().await; // the first tick is immediate
    #[cfg(unix)]
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context("installing the SIGTERM handler")?;
    #[cfg(not(unix))]
    let mut terminate = std::future::pending::<()>();
    loop {
        #[cfg(unix)]
        let sigterm = terminate.recv();
        #[cfg(not(unix))]
        let sigterm = &mut terminate;
        tokio::select! {
            result = tokio::signal::ctrl_c() => {
                result.context("waiting for ctrl-c")?;
                return Ok("interrupted");
            }
            _ = sigterm => return Ok("terminated"),
            _ = root_check.tick() => {
                if !root.is_dir() {
                    return Ok("repository root is gone");
                }
            }
        }
    }
}

async fn stdio_shim(bind: String, root: PathBuf) -> Result<ExitCode> {
    // Logs go to stderr; the MCP client owns stdout.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();
    let root = root
        .canonicalize()
        .with_context(|| format!("repository root {}", root.display()))?;
    stdio::run(root, bind).await?;
    Ok(ExitCode::SUCCESS)
}

async fn self_update(version: Option<String>, check: bool, root: &Path) -> Result<ExitCode> {
    let current = server::VERSION;
    let pinned = version.is_some();
    let target = match version {
        Some(v) => update::Release::from_tag(&v),
        None => update::latest_release().await?,
    };
    let newer = update::compare(&target.version, current) == std::cmp::Ordering::Greater;
    println!(
        "installed {current}, {} {}",
        if pinned { "requested" } else { "latest" },
        target.version
    );
    if check {
        return Ok(if newer {
            println!("update available: tirith update");
            ExitCode::FAILURE
        } else {
            println!("up to date");
            ExitCode::SUCCESS
        });
    }
    if !newer && !pinned {
        println!("up to date");
        return Ok(ExitCode::SUCCESS);
    }
    let dir = update::install_dir()?;
    println!("installing {} into {}", target.tag, dir.display());
    update::install(&target.tag, &dir).await?;
    println!("updated to {}", target.version);
    if let Ok(Some(info)) = JsonStore::new(root).read_daemon_info()
        && info.version != target.version
    {
        println!(
            "note: the daemon for {} (pid {}) still runs {}; stop it with `kill {}` so the next session starts {}",
            root.display(),
            info.pid,
            info.version,
            info.pid,
            target.version
        );
    }
    Ok(ExitCode::SUCCESS)
}

async fn tools(remote: &Remote) -> Result<ExitCode> {
    let tools = client::list_tools(&remote.url).await?;
    if remote.json {
        println!("{}", serde_json::to_string_pretty(&tools)?);
    } else {
        let width = tools.iter().map(|(n, _)| n.len()).max().unwrap_or(0);
        for (name, description) in tools {
            println!("{name:<width$}  {}", first_sentence(&description));
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// `tirith lead log`: the decision log is an HTTP route, not a tool
/// (ADR-0027), so this reads the dashboard next to the MCP endpoint.
async fn lead_log(
    remote: &Remote,
    limit: Option<usize>,
    event: Option<String>,
) -> Result<ExitCode> {
    let mut url = format!(
        "{}/api/lead?limit={}",
        dashboard_base(remote),
        limit.unwrap_or(50)
    );
    if let Some(event) = event {
        anyhow::ensure!(
            !event.is_empty() && event.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'),
            "--event is a snake_case event name, e.g. task_requested"
        );
        url.push_str("&event=");
        url.push_str(&event);
    }
    let body = get_json(&url).await?;
    if remote.json {
        println!("{}", serde_json::to_string_pretty(&body)?);
    } else {
        println!("{}", lead_log_lines(&body).join("\n"));
    }
    Ok(ExitCode::SUCCESS)
}

/// `tirith lead human`: the human queue, also an HTTP route (ADR-0027).
async fn lead_human(remote: &Remote) -> Result<ExitCode> {
    let body = get_json(&format!("{}/api/human", dashboard_base(remote))).await?;
    if remote.json {
        println!("{}", serde_json::to_string_pretty(&body)?);
    } else {
        println!("{}", human_lines(&body).join("\n"));
    }
    Ok(ExitCode::SUCCESS)
}

/// `tirith lead human done`: answers one item through the dashboard's
/// `POST /api/human/{id}/done`, with the reply when given.
async fn lead_human_done(remote: &Remote, id: &str, reply: Option<String>) -> Result<ExitCode> {
    let id: u64 = id
        .trim()
        .trim_start_matches('#')
        .parse()
        .context("the id is the number `tirith lead human` prints, e.g. 7 or #7")?;
    let url = format!("{}/api/human/{id}/done", dashboard_base(remote));
    let body = post_json(&url, &json!({ "reply": reply })).await?;
    if remote.json {
        println!("{}", serde_json::to_string_pretty(&body)?);
    } else {
        let answered = &body["answered"];
        match answered["reply"].as_object() {
            Some(reply) => println!(
                "#{id} done; reply sent to {}",
                reply.get("to").map_or("-", |to| to.as_str().unwrap_or("-"))
            ),
            None => println!("#{id} done"),
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// The dashboard origin next to the MCP endpoint.
fn dashboard_base(remote: &Remote) -> &str {
    let base = remote.url.strip_suffix("mcp").unwrap_or(&remote.url);
    base.trim_end_matches('/')
}

/// One line per human queue item: when, how many agents it blocks, who
/// raised it, the rule that sent it, and what it says.
fn human_lines(body: &Value) -> Vec<String> {
    let items = body["items"].as_array().map_or(&[][..], Vec::as_slice);
    if items.is_empty() {
        return vec!["nothing needs you".to_owned()];
    }
    items
        .iter()
        .map(|item| {
            let blocked = item["blocked_agents"].as_array().map_or(0, Vec::len);
            let task = item["task"].as_str().map_or_else(String::new, |t| {
                format!(" task {}", t.chars().take(8).collect::<String>())
            });
            format!(
                "#{} {} blocks {blocked} agent(s), from {}{task} [{}]: {}",
                item["id"],
                when(&item["at"]),
                s(&item["agent"]),
                item["rule"].as_str().unwrap_or("-"),
                s(&item["text"])
            )
        })
        .collect()
}

/// POSTs `body` as JSON to `url` on the daemon and parses its JSON answer.
async fn post_json(url: &str, body: &Value) -> Result<Value> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let response = client
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(serde_json::to_vec(body)?)
        .send()
        .await
        .with_context(|| format!("no daemon answered at {url}"))?;
    let status = response.status();
    let text = response.text().await?;
    let parsed: Option<Value> = serde_json::from_str(&text).ok();
    let message = parsed
        .as_ref()
        .and_then(|v| v["message"].as_str())
        .unwrap_or(&text);
    anyhow::ensure!(status.is_success(), "{url}: HTTP {status}: {message}");
    parsed.context("the daemon sent invalid JSON")
}

/// GETs `url` from the daemon and parses its JSON body.
async fn get_json(url: &str) -> Result<Value> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let response = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("no daemon answered at {url}"))?;
    let status = response.status();
    let text = response.text().await?;
    anyhow::ensure!(status.is_success(), "{url}: HTTP {status}: {text}");
    serde_json::from_str(&text).context("the daemon sent invalid JSON")
}

/// The lead, then one line per decision: id, when, event, agent, action,
/// and rule and outcome when present.
fn lead_log_lines(body: &Value) -> Vec<String> {
    let mut lines = vec![match body["lead"].as_object() {
        Some(lead) => format!(
            "lead: {} (lease until {})",
            s(&lead["agent"]),
            when(&lead["expires_at"])
        ),
        None => "lead: none (nobody holds .tirith/lead)".to_owned(),
    }];
    let entries = body["entries"].as_array().map_or(&[][..], Vec::as_slice);
    if entries.is_empty() {
        lines.push("no decisions logged".to_owned());
    }
    for e in entries {
        let who = e["agent"]
            .as_str()
            .map_or_else(String::new, |agent| format!(" {agent}"));
        let mut parts = vec![format!(
            "#{} {} {}{who}: {}",
            e["id"],
            when(&e["at"]),
            s(&e["event"]),
            s(&e["action"])
        )];
        if let Some(rule) = e["rule"].as_str() {
            parts.push(format!("rule: {rule}"));
        }
        if !e["outcome"].is_null() {
            parts.push(format!("outcome: {}", e["outcome"]));
        }
        lines.push(parts.join("; "));
    }
    lines
}

async fn call(remote: &Remote, tool: &str, mut arguments: Value) -> Result<ExitCode> {
    if let Value::Object(map) = &mut arguments {
        map.entry("agent")
            .or_insert_with(|| Value::String(remote.agent.clone()));
        map.retain(|_, v| !v.is_null());
    }
    let result = client::call_tool(&remote.url, tool, arguments).await?;
    if remote.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        let mut lines = vec![render(tool, &remote.agent, &result)];
        // Messages from other agents ride on any result; show them after it.
        if let Some(inbox) = result["inbox"].as_array().filter(|m| !m.is_empty()) {
            lines.push("inbox:".to_owned());
            lines.extend(inbox.iter().map(|m| format!("  {}", message_line(m))));
            if let Some(more) = result["inbox_more"].as_u64() {
                lines.push(format!(
                    "  …{more} more, run `tirith message list --unread`"
                ));
            }
        }
        println!("{}", lines.join("\n"));
    }
    let status = result["status"].as_str().unwrap_or("");
    Ok(if matches!(status, "ok" | "none") {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

// ---------------------------------------------------------------------------
// Human rendering.
// ---------------------------------------------------------------------------

fn first_sentence(text: &str) -> &str {
    text.split_once(". ").map_or(text, |(first, _)| first)
}

fn when(value: &Value) -> String {
    let Some(raw) = value.as_str() else {
        return "-".to_owned();
    };
    let Ok(time) = DateTime::parse_from_rfc3339(raw) else {
        return raw.to_owned();
    };
    let time = time.with_timezone(&Utc);
    if time.date_naive() == Utc::now().date_naive() {
        time.format("%H:%M:%SZ").to_string()
    } else {
        time.format("%Y-%m-%dT%H:%M:%SZ").to_string()
    }
}

fn strs(value: &Value) -> String {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|v| v.as_str().unwrap_or("?").to_owned())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
}

fn s(value: &Value) -> &str {
    value.as_str().unwrap_or("")
}

fn short(value: &Value) -> String {
    s(value).chars().take(8).collect()
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

/// One block of brief rows under a `name:` heading, omitted when empty.
fn section(out: &mut Vec<String>, name: &str, rows: &Value, line: impl Fn(&Value) -> String) {
    if let Some(rows) = rows.as_array().filter(|r| !r.is_empty()) {
        out.push(format!("{name}:"));
        out.extend(rows.iter().map(|r| format!("  {}", line(r))));
    }
}

/// Renders a tool result for a human, with the `lost` leases any result
/// may carry (ADR-0015) as warning lines above it.
fn render(tool: &str, agent: &str, v: &Value) -> String {
    let body = render_result(tool, agent, v);
    let lost: Vec<String> = v["lost"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|l| {
            format!(
                "warning: lost lease on {} at {}; stop editing it",
                s(&l["path"]),
                when(&l["at"])
            )
        })
        .collect();
    if lost.is_empty() {
        body
    } else {
        format!("{}\n{body}", lost.join("\n"))
    }
}

fn render_result(tool: &str, agent: &str, v: &Value) -> String {
    let status = s(&v["status"]);
    match (tool, status) {
        ("claim", "ok") => {
            let new_paths = strs(&v["new_paths"]);
            let renewed = strs(&v["renewed_paths"]);
            let paths = if renewed.is_empty() {
                new_paths
            } else {
                format!("{new_paths} (renewed: {renewed})")
                    .trim()
                    .to_owned()
            };
            let mut out = vec![format!(
                "ok       {agent}  {paths}  expires {}",
                when(&v["expires_at"])
            )];
            let absorbed = strs(&v["absorbed_paths"]);
            if !absorbed.is_empty() {
                out.push(format!("absorbed {absorbed}"));
            }
            // Someone lost this path recently: it may be half-edited.
            for p in v["previous_owner"].as_array().into_iter().flatten() {
                out.push(format!(
                    "previous owner {} on {} until {}",
                    s(&p["owner"]),
                    s(&p["path"]),
                    when(&p["reaped_at"])
                ));
            }
            // The brief: what to know before editing, five newest per section.
            section(&mut out, "notices", &v["notices"], |n| {
                format!(
                    "{} {:<9} {}  by {}",
                    short(&n["id"]),
                    s(&n["kind"]),
                    s(&n["summary"]),
                    s(&n["by"])
                )
            });
            section(&mut out, "contracts", &v["contracts"], |c| {
                format!("{}  v{} {}", s(&c["name"]), c["version"], s(&c["kind"]))
            });
            section(&mut out, "decisions", &v["decisions"], |d| {
                format!("{} {}", short(&d["id"]), s(&d["title"]))
            });
            section(&mut out, "memory", &v["memory"], memory_line);
            if let Some(more) = v["more"].as_object() {
                let rest: Vec<String> = more
                    .iter()
                    .filter(|(_, n)| n.as_u64().unwrap_or(0) > 0)
                    .map(|(k, n)| format!("{k} {n}"))
                    .collect();
                if !rest.is_empty() {
                    out.push(format!("more: {}", rest.join("  ")));
                }
            }
            out.join("\n")
        }
        ("claim", "conflict") => v["conflicts"]
            .as_array()
            .map(|cs| {
                cs.iter()
                    .map(|c| {
                        format!(
                            "conflict {} overlaps {} ({}: \"{}\", expires {})",
                            s(&c["path"]),
                            s(&c["overlaps"]),
                            s(&c["owner"]),
                            s(&c["reason"]),
                            when(&c["expires_at"])
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default(),
        ("release", "ok") => format!("released {agent}  {}", strs(&v["released"])),
        ("renew", "ok") => format!(
            "renewed  {agent}  {} claims until {}",
            v["count"],
            when(&v["expires_at"])
        ),
        ("claims_list", "ok") => page(v, "claims", "no live claims", |c| {
            format!(
                "{:<8} {:<28} {:<32} expires {}",
                s(&c["owner"]),
                strs(&c["paths"]),
                s(&c["reason"]),
                when(&c["expires_at"])
            )
        }),
        ("status", "ok") => {
            let mut out = vec![format!(
                "tirith {} up {}s  seq {}  claims {}  tasks {} open / {} done  contracts {}  notices {}  decisions {}",
                s(&v["version"]),
                v["uptime_secs"],
                v["seq"],
                v["claims"],
                v["tasks_open"],
                v["tasks_done"],
                v["contracts"],
                v["notices"],
                v["decisions"]
            )];
            if let Some(err) = v["persist_error"].as_str() {
                out.push(format!("PERSIST ERROR: {err}"));
            }
            if let Some(orphaned) = v["tasks_orphaned"].as_u64().filter(|n| *n > 0) {
                out.push(format!(
                    "  {orphaned} tasks returned to todo from silent owners"
                ));
            }
            out.push(list(&v["agents"], "no active agents", |a| {
                format!(
                    "  {:<12} holds {} paths  in progress {}  lease ends {}",
                    s(&a["agent"]),
                    a["paths_count"],
                    a["tasks_in_progress"],
                    when(&a["expires_at"])
                )
            }));
            out.join("\n")
        }
        ("guide", "ok") => guide_page(v),
        ("task_pull", "none") => "none     no unblocked todo tasks".to_owned(),
        ("task_create" | "task_pull" | "task_update", "ok") => task_line(&v["task"]),
        ("task_list", "ok") => page(v, "tasks", "no tasks", task_line),
        ("contract_publish", "ok") => {
            let notice = if v["notice_id"].is_string() {
                format!("  notice {}", short(&v["notice_id"]))
            } else {
                String::new()
            };
            format!("{}{notice}", contract_line(&v["contract"]))
        }
        ("contract_get", "ok") => {
            let c = &v["contract"];
            format!("{}\n{}", contract_line(c), pretty(&c["current"]["shape"]))
        }
        ("contract_list", "ok") => page(v, "contracts", "no contracts", contract_line),
        ("notice_publish", "ok") => notice_line(&v["notice"]),
        ("notice_list", "ok") => page(v, "notices", "no notices", notice_line),
        ("decision_record", "ok") => decision_line(&v["decision"]),
        ("decision_list", "ok") => page(v, "decisions", "no decisions", decision_line),
        ("memory_write", "ok") => {
            let verb = if v["created"].as_bool() == Some(true) {
                "created"
            } else {
                "updated"
            };
            format!("{verb:<8} {}", memory_line(&v["note"]))
        }
        ("memory_delete", "ok") => format!("deleted  {}", memory_line(&v["removed"])),
        ("message_send", "ok") => format!("sent     {}", message_line(&v["message"])),
        ("message_list", "ok") => page(v, "messages", "no messages", message_line),
        ("memory_read", "ok") => {
            let mut out = vec![memory_full(&v["note"])];
            if let Some(related) = v["related"].as_array().filter(|r| !r.is_empty()) {
                out.push("related:".to_owned());
                out.extend(related.iter().map(|n| format!("  {}", memory_line(n))));
            }
            out.join("\n")
        }
        ("memory_search", "ok") => page(v, "notes", "no notes", |note| {
            let score = note["score"]
                .as_u64()
                .map(|n| format!(" score {n}"))
                .unwrap_or_default();
            format!("{}{score}", memory_line(note))
        }),
        (_, "ok" | "none") => pretty(v),
        (_, other) => {
            let message = s(&v["message"]);
            if message.is_empty() {
                format!("{other}\n{}", pretty(v))
            } else {
                format!("{other:<8} {message}")
            }
        }
    }
}

/// A bounded listing: the rows under `key`, then how to see the rest when
/// the server truncated it.
fn page(v: &Value, key: &str, empty: &str, line: impl Fn(&Value) -> String) -> String {
    // An empty listing may carry a reason, e.g. "you hold no paths".
    let empty = v["message"].as_str().unwrap_or(empty);
    let mut out = list(&v[key], empty, line);
    if v["truncated"].as_bool() == Some(true) {
        let shown = v[key].as_array().map_or(0, Vec::len);
        let more = v["total"]
            .as_u64()
            .map(|t| t.saturating_sub(shown as u64))
            .map_or(String::new(), |n| format!("{n} more"));
        let cursor = v["next_before"]
            .as_str()
            .map(|b| format!(", pass --before {b}"))
            .unwrap_or_default();
        out.push_str("\n…");
        out.push_str(&more);
        out.push_str(&cursor);
    }
    out
}

fn list(items: &Value, empty: &str, line: impl Fn(&Value) -> String) -> String {
    match items.as_array() {
        Some(items) if !items.is_empty() => items.iter().map(line).collect::<Vec<_>>().join("\n"),
        _ => empty.to_owned(),
    }
}

fn task_line(t: &Value) -> String {
    let owner = t["owner"].as_str().or(t["by"].as_str()).unwrap_or("-");
    format!(
        "{} {:<12} {:<10} p{:<3} {}",
        short(&t["id"]),
        s(&t["status"]),
        owner,
        t["priority"],
        s(&t["title"])
    )
}

fn contract_line(c: &Value) -> String {
    format!(
        "{} {:<32} v{:<3} {:<9} consumers {}",
        short(&c["id"]),
        s(&c["name"]),
        c["current"]["version"],
        s(&c["kind"]),
        strs(&c["consumers"])
    )
}

fn notice_line(n: &Value) -> String {
    format!(
        "{} {:<9} {:<48} affects {}  by {}",
        short(&n["id"]),
        s(&n["kind"]),
        s(&n["summary"]),
        strs(&n["affected_paths"]),
        s(&n["published_by"])
    )
}

fn decision_line(d: &Value) -> String {
    format!(
        "{} {}: {}",
        short(&d["id"]),
        s(&d["title"]),
        s(&d["decision"])
    )
}

/// One line per note: permalink, kind, when, paths, title, and the first
/// line of the body. Never the whole body; that is what `memory read` is for.
fn memory_line(n: &Value) -> String {
    let paths = strs(&n["paths"]);
    let paths = if paths.is_empty() {
        String::new()
    } else {
        format!("  paths {paths}")
    };
    // Claim responses and search rows carry a server-made `excerpt` and no
    // body; a full note carries the body and no excerpt.
    let summary = match n["excerpt"].as_str() {
        Some(text) if !text.trim().is_empty() => text.trim().to_owned(),
        _ => excerpt(s(&n["body"])),
    };
    format!(
        "{} {:<8} {}{paths}  {}: {summary}",
        s(&n["permalink"]),
        s(&n["kind"]),
        when(&n["updated_at"]),
        s(&n["title"])
    )
}

/// The whole note, for `memory read`.
fn memory_full(n: &Value) -> String {
    let mut out = vec![memory_line(n)];
    let tags = strs(&n["tags"]);
    if !tags.is_empty() {
        out.push(format!("tags: {tags}"));
    }
    out.push(format!(
        "by {} at {}  (created by {} at {})",
        s(&n["updated_by"]),
        when(&n["updated_at"]),
        s(&n["author"]),
        when(&n["created_at"])
    ));
    out.push(String::new());
    out.push(s(&n["body"]).trim_end().to_owned());
    out.join("\n")
}

/// One line per message: id, when, from, to, and the text on one line.
fn message_line(m: &Value) -> String {
    let text: String = s(&m["text"])
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" / ");
    let to = if s(&m["to"]).is_empty() {
        "me"
    } else {
        s(&m["to"])
    };
    format!(
        "{} {} {} -> {}: {text}",
        short(&m["id"]),
        when(&m["at"]),
        s(&m["from"]),
        to
    )
}

/// The first non-empty, non-heading line of a body, cut to 160 characters.
fn excerpt(body: &str) -> String {
    let line = body
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with("---"))
        .unwrap_or_default();
    let mut cut: String = line.chars().take(160).collect();
    if cut.len() < line.len() {
        cut.push('…');
    }
    cut
}

/// The guide as plain text: the overview's loop, tools and rules, or one
/// topic's tools with when to call each.
fn guide_page(v: &Value) -> String {
    let mut out = Vec::new();
    for key in ["purpose", "summary"] {
        let text = s(&v[key]);
        if !text.is_empty() {
            out.push(text.to_owned());
        }
    }
    for (n, step) in v["loop"].as_array().into_iter().flatten().enumerate() {
        out.push(format!("{}. {}", n + 1, s(step)));
    }
    match &v["tools"] {
        Value::Object(groups) => {
            for (topic, names) in groups {
                out.push(format!("{topic:<10} {}", strs(names)));
            }
        }
        Value::Array(entries) => {
            for entry in entries {
                out.push(format!("{:<16} {}", s(&entry["tool"]), s(&entry["when"])));
            }
        }
        _ => {}
    }
    for rule in v["rules"].as_array().into_iter().flatten() {
        out.push(format!("- {}", s(rule)));
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serve_no_tray_is_a_flag() {
        // TIRITH_NO_TRAY feeds the same parser; the environment is not set
        // here (edition 2024 makes that unsafe), and the stdio shim tests
        // cover it end to end.
        let parsed = Cli::try_parse_from(["tirith", "serve", "--no-tray"]);
        assert!(matches!(
            parsed,
            Ok(Cli {
                command: Command::Serve { no_tray: true, .. },
                ..
            })
        ));
    }

    #[test]
    fn lead_human_lists_and_lead_human_done_answers() {
        assert!(matches!(
            Cli::try_parse_from(["tirith", "lead", "human"]),
            Ok(Cli {
                command: Command::Lead {
                    command: LeadCommand::Human { command: None }
                },
                ..
            })
        ));
        let done = Cli::try_parse_from([
            "tirith",
            "lead",
            "human",
            "done",
            "#7",
            "--reply",
            "use the keychain",
        ]);
        assert!(
            matches!(
                &done,
                Ok(Cli {
                    command: Command::Lead {
                        command: LeadCommand::Human {
                            command: Some(HumanCommand::Done { id, reply: Some(reply) }),
                        },
                    },
                    ..
                }) if id == "#7" && reply == "use the keychain"
            ),
            "{done:?}"
        );
    }

    #[test]
    fn the_human_queue_renders_one_line_per_item() {
        let body = json!({ "count": 1, "items": [{
            "id": 7, "at": "2026-09-17T04:00:00Z", "agent": "worker",
            "task": "9f0c1d2e-0000-0000-0000-000000000000", "rule": "human:credentials",
            "text": "need a key", "blocked_agents": ["worker", "other"],
        }]});
        let lines = human_lines(&body);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with("#7 "), "{lines:?}");
        assert!(
            lines[0].ends_with(
                "blocks 2 agent(s), from worker task 9f0c1d2e [human:credentials]: need a key"
            ),
            "{lines:?}"
        );
        assert_eq!(
            human_lines(&json!({ "items": [] })),
            vec!["nothing needs you"]
        );
    }

    #[test]
    fn the_lead_log_renders_one_line_per_decision() {
        let body = json!({
            "lead": { "agent": "boss", "expires_at": "2026-09-17T05:00:00Z" },
            "count": 2,
            "entries": [
                {
                    "id": 2, "at": "2026-09-17T04:00:00Z", "event": "escalation_raised",
                    "agent": "w", "seq": 9, "candidates": [], "rule": "human:credentials",
                    "action": "queued for the human", "outcome": null
                },
                {
                    "id": 1, "at": "2026-09-17T03:59:00Z", "event": "notice_published",
                    "seq": 8, "candidates": ["bob"], "rule": "holds_affected_path",
                    "action": "pushed notice n1 to 1 holder(s)", "outcome": "dismissed"
                }
            ]
        });
        let lines = lead_log_lines(&body);
        assert!(lines[0].starts_with("lead: boss"), "{lines:?}");
        assert!(lines[1].starts_with("#2 "), "{lines:?}");
        assert!(
            lines[1]
                .ends_with("escalation_raised w: queued for the human; rule: human:credentials"),
            "{lines:?}"
        );
        assert!(
            lines[2].ends_with(
                "notice_published: pushed notice n1 to 1 holder(s); rule: holds_affected_path; outcome: \"dismissed\""
            ),
            "{lines:?}"
        );
        let claim = lead_log_lines(&json!({ "lead": null, "entries": [{
            "id": 3, "at": "2026-09-17T04:01:00Z", "event": "claim_refused", "agent": "bob",
            "seq": 9, "candidates": ["src/a.rs"], "rule": "overlap",
            "action": "refused 1 path(s)", "outcome": null
        }] }));
        assert!(
            claim[1].ends_with("claim_refused bob: refused 1 path(s); rule: overlap"),
            "{claim:?}"
        );
        let empty = lead_log_lines(&json!({ "lead": null, "entries": [] }));
        assert_eq!(
            empty,
            [
                "lead: none (nobody holds .tirith/lead)",
                "no decisions logged"
            ]
        );
    }
}
