//! Load benchmark: N agents on persistent MCP sessions hammering a daemon
//! started in a temporary directory that is pre-seeded with realistic
//! state. Prints throughput and latency percentiles per tool.
//!
//! ```bash
//! cargo run --release --example swarm_bench -- 200 10
//! ```
//!
//! Arguments: agents (default 50), rounds per agent (default 10), then
//! optionally seeded notices, tasks, contracts, decisions, and memory
//! notes (default 500, each with a prose body, because that is what
//! search pays for). Set `THINK_MS` to pause each agent between calls,
//! which is what real agents do; the default is zero, the worst case for
//! the daemon.
//!
//! Every round each agent claims, lists, searches memory on its module,
//! reads the top hit, writes a note on its claimed path, and releases.
//! The claim response is checked for the `memory` array it must carry.
//!
//! Bytes are measured as well as time, because every response is paid
//! for in tokens by every agent: per tool the average structured and text
//! bytes and the largest structured response, the `tools/list` size each
//! session downloads once, the bytes an agent receives per round, and how
//! much `.tirith/` grew on disk.
//!
//! After the run it checks the invariants that matter under load: no two
//! agents hold overlapping paths, `persist_error` is null, and the
//! notices the daemon reports equal the lines in `notices.jsonl`.

use std::error::Error;
use std::net::SocketAddr;
use std::path::Path;
use std::time::{Duration, Instant};

use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
};
use serde_json::{Map, Value, json};
use tirith::server::{ServeOptions, start};

type Client = RunningService<RoleClient, ()>;
type BoxError = Box<dyn Error + Send + Sync>;

const TOOLS: [&str; 8] = [
    "claim",
    "notice_list",
    "claims_list",
    "status",
    "memory_search",
    "memory_read",
    "memory_write",
    "release",
];

async fn connect(url: &str) -> Result<Client, BoxError> {
    let config = StreamableHttpClientTransportConfig::with_uri(url.to_owned());
    let transport = StreamableHttpClientTransport::with_client(reqwest::Client::default(), config);
    Ok(().serve(transport).await?)
}

/// One measured call: how long it took and how many bytes came back.
#[derive(Debug, Clone, Copy)]
struct Sample {
    latency: Duration,
    /// Bytes of `structured_content`, what an MCP client parses.
    structured: usize,
    /// Bytes of the text blocks, what a text-only client shows the model.
    text: usize,
}

async fn measure(client: &Client, tool: &str, args: Value) -> Result<(Value, Sample), BoxError> {
    let Value::Object(map) = args else {
        return Err("arguments must be an object".into());
    };
    let map: Map<String, Value> = map;
    let started = Instant::now();
    let result = client
        .call_tool(CallToolRequestParams::new(tool.to_owned()).with_arguments(map))
        .await?;
    let latency = started.elapsed();
    let structured = result
        .structured_content
        .as_ref()
        .map_or(0, |v| v.to_string().len());
    let text = serde_json::to_string(&result.content)?.len();
    Ok((
        result.structured_content.unwrap_or(Value::Null),
        Sample {
            latency,
            structured,
            text,
        },
    ))
}

async fn call(client: &Client, tool: &str, args: Value) -> Result<Value, BoxError> {
    Ok(measure(client, tool, args).await?.0)
}

/// Bytes and files under `path`, recursively.
fn dir_size(path: &Path) -> (u64, usize) {
    let mut bytes = 0;
    let mut files = 0;
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let child = entry.path();
            if child.is_dir() {
                let (b, f) = dir_size(&child);
                bytes += b;
                files += f;
            } else if let Ok(meta) = entry.metadata() {
                bytes += meta.len();
                files += 1;
            }
        }
    }
    (bytes, files)
}

fn percentile(sorted: &[Duration], num: usize, den: usize) -> Duration {
    match sorted.len() {
        0 => Duration::ZERO,
        len => sorted[(len - 1) * num / den],
    }
}

async fn seed(
    url: &str,
    notices: usize,
    tasks: usize,
    contracts: usize,
    decisions: usize,
    notes: usize,
) -> Result<(), BoxError> {
    let client = connect(url).await?;
    // Notes carry prose bodies, which is what makes memory_search cost
    // what it costs; seeding one-line notes would measure nothing.
    let filler = "Notes hold prose, and scoring reads all of it. ".repeat(40);
    for i in 0..notes {
        call(
            &client,
            "memory_write",
            json!({
                "agent": "seeder",
                "title": format!("seeded note {i} about module {}", i % 40),
                "body": format!("{filler}\n\n- [design] observation {i} #seeded"),
                "kind": "lesson",
                "paths": [format!("src/mod{}", i % 40)],
                "tags": ["seeded"],
            }),
        )
        .await?;
    }
    for i in 0..notices {
        call(
            &client,
            "notice_publish",
            json!({
                "agent": "seeder", "kind": "rename",
                "summary": format!("renamed thing_{i} to other_{i} in a module somewhere"),
                "affected_paths": [format!("src/mod{}/file{i}.rs", i % 40)]
            }),
        )
        .await?;
    }
    for i in 0..tasks {
        call(
            &client,
            "task_create",
            json!({
                "agent": "seeder", "title": format!("task {i}"),
                "description": "a description of medium length, for realism",
                "paths": [format!("src/mod{}", i % 40)]
            }),
        )
        .await?;
    }
    for i in 0..contracts {
        call(
            &client,
            "contract_publish",
            json!({
                "agent": "seeder", "name": format!("POST /api/thing{i}"), "kind": "http",
                "shape": {"request": {"id": "string"}, "response": {"ok": "bool"}},
                "consumers": [format!("src/mod{}", i % 40)]
            }),
        )
        .await?;
    }
    for i in 0..decisions {
        call(
            &client,
            "decision_record",
            json!({
                "agent": "seeder", "title": format!("decision {i}"),
                "decision": "use the simple thing",
                "rationale": "because it was the simplest thing that could possibly work",
                "affects_paths": [format!("src/mod{}", i % 40)]
            }),
        )
        .await?;
    }
    client.cancel().await?;
    Ok(())
}

async fn agent(
    url: String,
    index: usize,
    rounds: usize,
    think: Duration,
) -> Result<Vec<(&'static str, Sample)>, BoxError> {
    let client = connect(&url).await?;
    let name = format!("agent-{index}");
    let mut latencies = Vec::with_capacity(rounds * TOOLS.len());
    for round in 0..rounds {
        if !think.is_zero() {
            tokio::time::sleep(think).await;
        }
        // Mostly private paths; every fifth round also touches a shared
        // directory so some claims conflict.
        let own = format!("src/mod{}/a{index}_r{round}.rs", index % 40);
        let paths = if round % 5 == 0 {
            vec![own, format!("src/shared/{}", round % 7)]
        } else {
            vec![own]
        };
        let (claimed, sample) = measure(
            &client,
            "claim",
            json!({ "agent": name, "paths": paths, "reason": "bench" }),
        )
        .await?;
        latencies.push(("claim", sample));
        let module = format!("src/mod{}", index % 40);
        for (tool, args) in [
            (
                "notice_list",
                json!({ "agent": name, "path": module, "unread": true }),
            ),
            ("claims_list", json!({ "agent": name })),
            ("status", json!({ "agent": name })),
        ] {
            let (_, sample) = measure(&client, tool, args).await?;
            latencies.push((tool, sample));
        }
        // A claim must hand back the notes for the paths it just claimed;
        // that is the whole point of the primitive, so assert it rather
        // than only timing it.
        if claimed["status"] == "ok" && !claimed["memory"].is_array() {
            return Err(format!("claim response carried no memory array: {claimed}").into());
        }

        let (found, sample) = measure(
            &client,
            "memory_search",
            json!({ "agent": name, "query": "observation design module", "path": module }),
        )
        .await?;
        latencies.push(("memory_search", sample));

        // Read the top hit, the way an agent would after searching.
        let top = found["notes"]
            .get(0)
            .and_then(|n| n["permalink"].as_str())
            .map(str::to_owned);
        if let Some(permalink) = top {
            let (_, sample) = measure(
                &client,
                "memory_read",
                json!({ "agent": name, "name": permalink, "depth": 1 }),
            )
            .await?;
            latencies.push(("memory_read", sample));
        }

        let (_, sample) = measure(
            &client,
            "memory_write",
            json!({
                "agent": name,
                "title": format!("what agent {index} learned in round {round}"),
                "body": "Something worth telling whoever claims this path next.",
                "kind": "lesson",
                "paths": [format!("src/mod{}", index % 40)],
            }),
        )
        .await?;
        latencies.push(("memory_write", sample));

        if claimed["status"] == "ok" {
            let (_, sample) = measure(&client, "release", json!({ "agent": name })).await?;
            latencies.push(("release", sample));
        }
    }
    client.cancel().await?;
    Ok(latencies)
}

async fn run(url: &str, agents: usize, rounds: usize, think: Duration) -> Result<(), BoxError> {
    let started = Instant::now();
    let mut handles = Vec::with_capacity(agents);
    for index in 0..agents {
        handles.push(tokio::spawn(agent(url.to_owned(), index, rounds, think)));
    }
    let mut all: Vec<(&'static str, Sample)> = Vec::new();
    for handle in handles {
        all.extend(handle.await??);
    }
    let wall = started.elapsed();
    let calls = u32::try_from(all.len())?;
    eprintln!(
        "\n== {agents} agents x {rounds} rounds, think {think:?} = {calls} calls in {:.2}s ({:.0} calls/s)",
        wall.as_secs_f64(),
        f64::from(calls) / wall.as_secs_f64()
    );
    eprintln!(
        "  {:<12} {:<8} {:>9} {:>9} {:>9} {:>9} | {:>7} {:>6} {:>7} {:>6}",
        "tool", "n", "p50", "p95", "p99", "max", "struct", "text", "max", "~tok"
    );
    let mut total_bytes = 0usize;
    for tool in TOOLS {
        let samples: Vec<Sample> = all
            .iter()
            .filter(|(t, _)| *t == tool)
            .map(|(_, s)| *s)
            .collect();
        let mut sorted: Vec<Duration> = samples.iter().map(|s| s.latency).collect();
        sorted.sort();
        let n = samples.len().max(1);
        let structured: usize = samples.iter().map(|s| s.structured).sum::<usize>() / n;
        let text: usize = samples.iter().map(|s| s.text).sum::<usize>() / n;
        let largest = samples.iter().map(|s| s.structured).max().unwrap_or(0);
        total_bytes += samples.iter().map(|s| s.structured + s.text).sum::<usize>();
        eprintln!(
            "  {tool:<12} n={:<6} {:>9.2?} {:>9.2?} {:>9.2?} {:>9.2?} | {structured:>7} {text:>6} {largest:>7} {:>6}",
            samples.len(),
            percentile(&sorted, 1, 2),
            percentile(&sorted, 95, 100),
            percentile(&sorted, 99, 100),
            percentile(&sorted, 1, 1),
            (structured + text) / 4,
        );
    }
    let per_round = total_bytes / agents.max(1) / rounds.max(1);
    eprintln!(
        "  response bytes per agent per round: {per_round} (~{} tokens); whole run {} KB",
        per_round / 4,
        total_bytes / 1000
    );
    Ok(())
}

/// Checks the invariants that must hold after any run and returns the
/// first violation as an error.
async fn check_invariants(url: &str, tirith_dir: &Path) -> Result<(), BoxError> {
    let client = connect(url).await?;
    let status = call(&client, "status", json!({})).await?;
    if !status["persist_error"].is_null() {
        return Err(format!("persist_error is set: {}", status["persist_error"]).into());
    }
    let claims = call(&client, "claims_list", json!({ "agent": "checker" })).await?;
    let held: Vec<(String, String)> = claims["claims"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|claim| {
            let owner = claim["owner"].as_str().unwrap_or_default().to_owned();
            claim["paths"]
                .as_array()
                .into_iter()
                .flatten()
                .map(move |p| (owner.clone(), p.as_str().unwrap_or_default().to_owned()))
        })
        .collect();
    for (i, (owner_a, path_a)) in held.iter().enumerate() {
        for (owner_b, path_b) in &held[i + 1..] {
            let overlaps = path_a == path_b
                || path_a.starts_with(&format!("{path_b}/"))
                || path_b.starts_with(&format!("{path_a}/"));
            if owner_a != owner_b && overlaps {
                return Err(format!(
                    "overlapping claims: {owner_a} holds {path_a}, {owner_b} holds {path_b}"
                )
                .into());
            }
        }
    }
    let on_disk = std::fs::read_to_string(tirith_dir.join("notices.jsonl"))?
        .lines()
        .filter(|l| !l.trim().is_empty())
        .count();
    let in_memory = usize::try_from(status["notices"].as_u64().unwrap_or_default())?;
    if on_disk != in_memory {
        return Err(format!("{in_memory} notices in memory but {on_disk} lines on disk").into());
    }
    client.cancel().await?;
    eprintln!(
        "invariants ok: {} live claims without overlap, {in_memory} notices on disk, persist_error null",
        held.len()
    );
    Ok(())
}

fn arg(index: usize, default: usize) -> Result<usize, BoxError> {
    match std::env::args().nth(index) {
        Some(raw) => Ok(raw.parse()?),
        None => Ok(default),
    }
}

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    let agents = arg(1, 50)?;
    let rounds = arg(2, 10)?;
    let notices = arg(3, 2000)?;
    let tasks = arg(4, 500)?;
    let contracts = arg(5, 50)?;
    let decisions = arg(6, 200)?;
    let notes = arg(7, 500)?;
    let think = Duration::from_millis(match std::env::var("THINK_MS") {
        Ok(raw) => raw.parse()?,
        Err(_) => 0,
    });

    let dir = tempfile::tempdir()?;
    let handle = start(ServeOptions {
        bind: "127.0.0.1:0".parse::<SocketAddr>()?,
        repo_root: dir.path().to_path_buf(),
        clock: None,
    })
    .await?;
    let url = handle.mcp_url();
    let started = Instant::now();
    seed(&url, notices, tasks, contracts, decisions, notes).await?;
    eprintln!(
        "seeded {notices} notices, {tasks} tasks, {contracts} contracts, {decisions} decisions, {notes} notes in {:.2?}",
        started.elapsed()
    );
    let probe = connect(&url).await?;
    let tools = probe.list_all_tools().await?;
    let tools_bytes = serde_json::to_string(&tools)?.len();
    probe.cancel().await?;
    eprintln!(
        "tools/list: {} tools, {tools_bytes} bytes (~{} tokens), downloaded once per session",
        tools.len(),
        tools_bytes / 4
    );
    let tirith_dir = dir.path().join(".tirith");
    let (bytes_before, files_before) = dir_size(&tirith_dir);
    run(&url, agents, rounds, think).await?;
    check_invariants(&url, &tirith_dir).await?;
    handle.shutdown().await?;
    let (bytes_after, files_after) = dir_size(&tirith_dir);
    eprintln!(
        ".tirith on disk: {bytes_before} B / {files_before} files -> {bytes_after} B / {files_after} files (+{} B, +{} files)",
        bytes_after.saturating_sub(bytes_before),
        files_after.saturating_sub(files_before)
    );
    Ok(())
}
