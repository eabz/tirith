//! Command-line interface: `serve` runs the daemon; every other subcommand
//! is a thin MCP client call rendered for humans.

// The CLI's job is printing to stdout.
#![allow(clippy::print_stdout)]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use clap::{Args, Parser, Subcommand};
use serde_json::{Value, json};
use tirith::client;
use tirith::server::{self, DEFAULT_BIND, ServeOptions};
use tirith::stdio;
use tirith::store::JsonStore;

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

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the daemon for the repository at --root.
    Serve {
        /// Address to bind.
        #[arg(long, default_value = DEFAULT_BIND)]
        bind: SocketAddr,
    },
    /// Serve MCP over stdin/stdout for clients that spawn servers themselves,
    /// starting the repository's daemon if none is running.
    Stdio {
        /// Address to bind if the daemon has to be started.
        #[arg(long, default_value = DEFAULT_BIND)]
        bind: String,
    },
    /// Show daemon status and who holds what.
    Status,
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
    /// List live claims.
    Claims {
        /// Only claims overlapping this path.
        #[arg(long)]
        path: Option<String>,
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
    },
    /// List tasks.
    List {
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        owner: Option<String>,
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
    },
    /// Fetch a contract by name or id.
    Get { name: String },
    /// List contracts.
    List {
        #[arg(long)]
        path: Option<String>,
        #[arg(long)]
        kind: Option<String>,
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
        /// Only notices you have not acknowledged.
        #[arg(long)]
        unread: bool,
    },
    /// Acknowledge a notice.
    Ack { notice_id: String },
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
    },
}

/// Common options every remote command needs.
#[derive(Debug, Args)]
struct Remote {
    url: String,
    agent: String,
    json: bool,
}

/// Parses arguments and runs the chosen command.
pub(crate) async fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    if let Command::Serve { bind } = cli.command {
        return serve(bind, cli.root).await;
    }
    if let Command::Stdio { bind } = cli.command {
        return stdio_shim(bind, cli.root).await;
    }
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
    let (tool, arguments) = match cli.command {
        Command::Serve { .. } | Command::Stdio { .. } => unreachable!("handled above"),
        Command::Tools => return tools(&remote).await,
        Command::Call { tool, arguments } => {
            let mut value: Value =
                serde_json::from_str(&arguments).context("arguments must be a JSON object")?;
            if let Value::Object(map) = &mut value {
                map.entry("agent")
                    .or_insert_with(|| Value::String(remote.agent.clone()));
            }
            (tool, value)
        }
        Command::Status => ("status".to_owned(), json!({})),
        Command::Claim { reason, ttl, paths } => (
            "claim".to_owned(),
            json!({ "paths": paths, "reason": reason, "ttl_secs": ttl }),
        ),
        Command::Release { paths } => (
            "release".to_owned(),
            json!({ "paths": if paths.is_empty() { Value::Null } else { json!(paths) } }),
        ),
        Command::Renew => ("renew".to_owned(), json!({})),
        Command::Claims { path } => ("claims_list".to_owned(), json!({ "path": path })),
        Command::Task { command } => task_call(command),
        Command::Contract { command } => contract_call(command)?,
        Command::Notice { command } => notice_call(command),
        Command::Decision { command } => decision_call(command),
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
        } => (
            "task_update".to_owned(),
            json!({ "task_id": task_id, "status": status, "note": note }),
        ),
        TaskCommand::List { status, owner } => (
            "task_list".to_owned(),
            json!({ "status": status, "owner": owner }),
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
        } => {
            let shape: Value = serde_json::from_str(&shape).context("--shape must be JSON")?;
            (
                "contract_publish".to_owned(),
                json!({ "name": name, "kind": kind, "shape": shape, "consumers": consumers, "notes": notes }),
            )
        }
        ContractCommand::Get { name } => ("contract_get".to_owned(), json!({ "name": name })),
        ContractCommand::List { path, kind } => (
            "contract_list".to_owned(),
            json!({ "path": path, "kind": kind }),
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
        } => (
            "notice_list".to_owned(),
            json!({ "path": path, "since": since, "unread": unread }),
        ),
        NoticeCommand::Ack { notice_id } => {
            ("notice_ack".to_owned(), json!({ "notice_id": notice_id }))
        }
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
        DecisionCommand::List { path, query } => (
            "decision_list".to_owned(),
            json!({ "path": path, "query": query }),
        ),
    }
}

async fn serve(bind: SocketAddr, root: PathBuf) -> Result<ExitCode> {
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
    let handle = server::start(ServeOptions {
        bind,
        repo_root: root.clone(),
        clock: None,
    })
    .await?;
    println!("tirith {} serving {}", server::VERSION, root.display());
    println!("  mcp       {}", handle.mcp_url());
    println!("  dashboard {}", handle.dashboard_url());
    tokio::signal::ctrl_c()
        .await
        .context("waiting for ctrl-c")?;
    eprintln!("shutting down");
    handle.shutdown().await?;
    Ok(ExitCode::SUCCESS)
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
        println!("{}", render(tool, &remote.agent, &result));
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

fn render(tool: &str, agent: &str, v: &Value) -> String {
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
            format!(
                "ok       {agent}  {paths}  expires {}",
                when(&v["expires_at"])
            )
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
        ("renew", "ok") => {
            let latest = v["claims"]
                .as_array()
                .and_then(|cs| cs.iter().map(|c| when(&c["expires_at"])).max())
                .unwrap_or_default();
            format!("renewed  {agent}  until {latest}")
        }
        ("claims_list", "ok") => list(&v["claims"], "no live claims", |c| {
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
            out.push(list(&v["agents"], "no active agents", |a| {
                format!(
                    "  {:<12} holds {}  in progress {}  lease ends {}",
                    s(&a["agent"]),
                    strs(&a["paths"]),
                    a["tasks_in_progress"],
                    when(&a["expires_at"])
                )
            }));
            out.join("\n")
        }
        ("task_pull", "none") => "none     no unblocked todo tasks".to_owned(),
        ("task_create" | "task_pull" | "task_update", "ok") => task_line(&v["task"]),
        ("task_list", "ok") => list(&v["tasks"], "no tasks", task_line),
        ("contract_publish", "ok") => {
            let c = &v["contract"];
            let notice = v["notice_id"]
                .as_str()
                .map(|n| format!("  notice {}", &n[..8.min(n.len())]))
                .unwrap_or_default();
            format!("{}{notice}", contract_line(c))
        }
        ("contract_get", "ok") => {
            let c = &v["contract"];
            format!("{}\n{}", contract_line(c), pretty(&c["current"]["shape"]))
        }
        ("contract_list", "ok") => list(&v["contracts"], "no contracts", contract_line),
        ("notice_publish" | "notice_ack", "ok") => notice_line(&v["notice"]),
        ("notice_list", "ok") => list(&v["notices"], "no notices", notice_line),
        ("decision_record", "ok") => decision_line(&v["decision"]),
        ("decision_list", "ok") => list(&v["decisions"], "no decisions", decision_line),
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
