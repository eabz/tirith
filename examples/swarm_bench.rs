//! The coordination benchmark: a scripted swarm of fake workers against a
//! real daemon, with and without claims (docs/3-tests/01-testing-strategy.md).
//!
//! The lead seeds a board of tasks whose paths overlap on purpose: every
//! task edits one of `--files` files, and every `--hub-every`th task also
//! edits the hub file. Workers pull with `wait_secs`, claim with
//! `wait_secs`, hold their paths for `--work-ms`, release and finish. In
//! `none` mode they skip claims, which is the counterfactual: how many
//! edits would have overlapped another agent's edit of the same path.
//!
//! ```bash
//! cargo run --release --example swarm_bench -- --agents 8 --tasks 40
//! cargo run --release --example swarm_bench -- --mode none --json
//! ```

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use clap::{Parser, ValueEnum};
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::common::client_side_sse::NeverRetry;
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
};
use serde::Serialize;
use serde_json::{Map, Value, json};
use tirith::lead::LeadEvent;
use tirith::server::{ServeOptions, ServerHandle, start};

/// How workers treat the paths they edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize)]
#[serde(rename_all = "lowercase")]
enum Mode {
    /// Claim before editing, with `wait_secs`.
    Claims,
    /// Edit without claiming: the counterfactual.
    None,
    /// Run both, `claims` first, and print both.
    Both,
}

#[derive(Debug, Parser)]
#[command(about = "Scripted swarm against a real Tirith daemon")]
struct Args {
    /// Fake workers pulling from the board.
    #[arg(long, default_value_t = 8)]
    agents: usize,
    /// Tasks the lead seeds.
    #[arg(long, default_value_t = 40)]
    tasks: usize,
    /// Distinct files tasks are spread over (task i edits file i mod files).
    #[arg(long, default_value_t = 12)]
    files: usize,
    /// Every Nth task also edits the hub file, src/hub.rs (0 for never).
    #[arg(long, default_value_t = 3)]
    hub_every: usize,
    /// Milliseconds a worker holds its paths per task: the scripted edit.
    #[arg(long, default_value_t = 150)]
    work_ms: u64,
    /// Claim before editing, skip claims, or both.
    #[arg(long, value_enum, default_value_t = Mode::Both)]
    mode: Mode,
    /// Print the report as JSON.
    #[arg(long)]
    json: bool,
}

/// One task's scripted edit: which paths, for how long.
#[derive(Debug, Clone)]
struct Scenario {
    tasks: Vec<Vec<String>>,
    work: Duration,
}

impl Scenario {
    fn from(args: &Args) -> Self {
        let tasks = (0..args.tasks)
            .map(|i| {
                let mut paths = vec![format!("src/mod_{}.rs", i % args.files.max(1))];
                if args.hub_every > 0 && i % args.hub_every == 0 {
                    paths.push("src/hub.rs".to_owned());
                }
                paths
            })
            .collect();
        Self {
            tasks,
            work: Duration::from_millis(args.work_ms),
        }
    }
}

/// What every worker records: call timings and sizes, and its edits.
#[derive(Debug, Default)]
struct Tally {
    calls: BTreeMap<String, Vec<(Duration, usize)>>,
    refused: usize,
    none: usize,
    edits: BTreeMap<String, Vec<(Instant, Instant)>>,
}

impl Tally {
    fn record(&mut self, tool: &str, elapsed: Duration, result: &Value) {
        let bytes = result.to_string().len();
        self.calls
            .entry(tool.to_owned())
            .or_default()
            .push((elapsed, bytes));
    }
}

/// One MCP session, as a shim holds it for a real agent.
struct Session(RunningService<RoleClient, ()>);

impl Session {
    async fn connect(url: &str) -> Result<Self> {
        let mut config = StreamableHttpClientTransportConfig::with_uri(url.to_owned());
        config.retry_config = Arc::new(NeverRetry::default());
        let transport =
            StreamableHttpClientTransport::with_client(reqwest::Client::default(), config);
        Ok(Self(().serve(transport).await?))
    }

    fn instructions_len(&self) -> usize {
        self.0
            .peer_info()
            .and_then(|info| info.instructions.as_ref().map(String::len))
            .unwrap_or(0)
    }

    async fn call(&self, tally: &Mutex<Tally>, tool: &str, args: Value) -> Result<Value> {
        let arguments: Map<String, Value> = match args {
            Value::Object(map) => map,
            _ => Map::new(),
        };
        let started = Instant::now();
        let result = self
            .0
            .call_tool(CallToolRequestParams::new(tool.to_owned()).with_arguments(arguments))
            .await
            .with_context(|| format!("calling {tool}"))?;
        let value = result
            .structured_content
            .ok_or_else(|| anyhow!("{tool} returned no structured content"))?;
        tally.lock().map_err(|_| anyhow!("tally poisoned"))?.record(
            tool,
            started.elapsed(),
            &value,
        );
        Ok(value)
    }

    async fn close(self) {
        let _ = self.0.cancel().await;
    }
}

/// One worker's life: pull, (claim,) edit, (release,) done, until the
/// board is empty.
async fn worker(
    url: String,
    name: String,
    mode: Mode,
    work: Duration,
    tally: Arc<Mutex<Tally>>,
) -> Result<()> {
    let session = Session::connect(&url).await?;
    loop {
        let pulled = session
            .call(
                &tally,
                "task_pull",
                json!({ "agent": name, "wait_secs": 120 }),
            )
            .await?;
        if pulled["status"] != "ok" {
            tally.lock().map_err(|_| anyhow!("tally poisoned"))?.none += 1;
            break;
        }
        let task = pulled["task"]["id"]
            .as_str()
            .ok_or_else(|| anyhow!("task without id"))?
            .to_owned();
        let paths: Vec<String> = pulled["task"]["paths"]
            .as_array()
            .map(|p| {
                p.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let mut held = false;
        if mode == Mode::Claims {
            let claimed = session
                .call(
                    &tally,
                    "claim",
                    json!({
                        "agent": name,
                        "paths": paths,
                        "reason": format!("task {}", &task[..8]),
                        "ttl_secs": 60,
                        "wait_secs": 120,
                        "brief": false,
                    }),
                )
                .await?;
            held = claimed["status"] == "ok";
            if !held {
                tally.lock().map_err(|_| anyhow!("tally poisoned"))?.refused += 1;
            }
        }
        if mode == Mode::None || held {
            let started = Instant::now();
            tokio::time::sleep(work).await;
            let ended = Instant::now();
            let mut tally = tally.lock().map_err(|_| anyhow!("tally poisoned"))?;
            for path in &paths {
                tally
                    .edits
                    .entry(path.clone())
                    .or_default()
                    .push((started, ended));
            }
        }
        if held {
            session
                .call(&tally, "release", json!({ "agent": name }))
                .await?;
        }
        let status = if mode == Mode::None || held {
            "done"
        } else {
            "todo"
        };
        session
            .call(
                &tally,
                "task_update",
                json!({ "agent": name, "task_id": task, "status": status }),
            )
            .await?;
    }
    session.close().await;
    Ok(())
}

/// Pairs of edits of the same path whose intervals overlap.
fn collisions(edits: &BTreeMap<String, Vec<(Instant, Instant)>>) -> (usize, usize) {
    let mut pairs = 0;
    let mut paths = 0;
    for intervals in edits.values() {
        let mut sorted = intervals.clone();
        sorted.sort_by_key(|(start, _)| *start);
        let before = pairs;
        for (i, (start, _)) in sorted.iter().enumerate() {
            pairs += sorted[..i].iter().filter(|(_, end)| end > start).count();
        }
        if pairs > before {
            paths += 1;
        }
    }
    (pairs, paths)
}

/// The `percent`th percentile of `sorted`, nearest rank.
fn percentile(sorted: &[Duration], percent: usize) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let rank = ((sorted.len() - 1) * percent).div_ceil(100);
    sorted[rank.min(sorted.len() - 1)]
}

/// One tool's numbers in the report.
#[derive(Debug, Serialize)]
struct ToolRow {
    calls: usize,
    p50_ms: f64,
    p95_ms: f64,
    total_ms: f64,
    avg_bytes: usize,
}

/// One run's report.
#[derive(Debug, Serialize)]
struct Report {
    mode: Mode,
    agents: usize,
    tasks: usize,
    wall_ms: f64,
    tasks_done: usize,
    collisions: usize,
    paths_collided: usize,
    claims_refused: usize,
    pulls_none: usize,
    claim_waits: usize,
    claim_waited_ms: u64,
    instructions_bytes: usize,
    tools: BTreeMap<String, ToolRow>,
}

async fn run(args: &Args, mode: Mode, root: &Path) -> Result<Report> {
    let scenario = Scenario::from(args);
    let handle: ServerHandle = start(ServeOptions {
        bind: "127.0.0.1:0".parse()?,
        repo_root: root.to_path_buf(),
        clock: None,
        registry: None,
    })
    .await?;
    let url = handle.mcp_url();
    let tally = Arc::new(Mutex::new(Tally::default()));

    let lead = Session::connect(&url).await?;
    let instructions_bytes = lead.instructions_len();
    lead.call(
        &tally,
        "claim",
        json!({ "agent": "lead", "paths": [".tirith/lead"], "reason": "bench", "ttl_secs": 3600, "brief": false }),
    )
    .await?;
    for (i, paths) in scenario.tasks.iter().enumerate() {
        lead.call(
            &tally,
            "task_create",
            json!({ "agent": "lead", "title": format!("task {i}"), "paths": paths }),
        )
        .await?;
    }

    let started = Instant::now();
    let mut workers = Vec::new();
    for i in 0..args.agents {
        workers.push(tokio::spawn(worker(
            url.clone(),
            format!("w{i}"),
            mode,
            scenario.work,
            Arc::clone(&tally),
        )));
    }
    for w in workers {
        w.await??;
    }
    let wall = started.elapsed();
    lead.close().await;

    let state = handle.state();
    let tasks_done = state.status(None).tasks_done;
    let waits = state.lead_log(500, Some(LeadEvent::ClaimWaited));
    let claim_waited_ms = waits
        .iter()
        .filter_map(|row| row.details.as_ref()?["waited_ms"].as_u64())
        .sum();
    handle.shutdown().await?;

    let tally = tally.lock().map_err(|_| anyhow!("tally poisoned"))?;
    let (collisions, paths_collided) = collisions(&tally.edits);
    let tools = tally
        .calls
        .iter()
        .map(|(tool, rows)| {
            let mut sorted: Vec<Duration> = rows.iter().map(|(d, _)| *d).collect();
            sorted.sort_unstable();
            let total: Duration = sorted.iter().sum();
            let bytes: usize = rows.iter().map(|(_, b)| *b).sum();
            (
                tool.clone(),
                ToolRow {
                    calls: rows.len(),
                    p50_ms: percentile(&sorted, 50).as_secs_f64() * 1000.0,
                    p95_ms: percentile(&sorted, 95).as_secs_f64() * 1000.0,
                    total_ms: total.as_secs_f64() * 1000.0,
                    avg_bytes: bytes / rows.len().max(1),
                },
            )
        })
        .collect();
    Ok(Report {
        mode,
        agents: args.agents,
        tasks: args.tasks,
        wall_ms: wall.as_secs_f64() * 1000.0,
        tasks_done,
        collisions,
        paths_collided,
        claims_refused: tally.refused,
        pulls_none: tally.none,
        claim_waits: waits.len(),
        claim_waited_ms,
        instructions_bytes,
        tools,
    })
}

fn print(out: &mut impl Write, report: &Report) -> Result<()> {
    writeln!(
        out,
        "{:<8} agents {:<3} tasks {:<4} wall {:>8.0} ms  done {:<4} collisions {:<4} (paths {:<3}) refused {:<3} waits {:<3} waited {:>6} ms  instructions {} B",
        format!("{:?}", report.mode).to_lowercase(),
        report.agents,
        report.tasks,
        report.wall_ms,
        report.tasks_done,
        report.collisions,
        report.paths_collided,
        report.claims_refused,
        report.claim_waits,
        report.claim_waited_ms,
        report.instructions_bytes,
    )?;
    for (tool, row) in &report.tools {
        writeln!(
            out,
            "  {tool:<12} calls {:<5} p50 {:>7.1} ms  p95 {:>8.1} ms  total {:>9.0} ms  avg {:>5} B",
            row.calls, row.p50_ms, row.p95_ms, row.total_ms, row.avg_bytes
        )?;
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let modes = match args.mode {
        Mode::Both => vec![Mode::Claims, Mode::None],
        one => vec![one],
    };
    let mut reports = Vec::new();
    for mode in modes {
        let dir = tempfile::tempdir()?;
        reports.push(run(&args, mode, dir.path()).await?);
    }
    let mut out = std::io::stdout().lock();
    if args.json {
        writeln!(out, "{}", serde_json::to_string_pretty(&reports)?)?;
    } else {
        for report in &reports {
            print(&mut out, report)?;
        }
    }
    Ok(())
}
