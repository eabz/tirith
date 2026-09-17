//! Jev micro-benchmark: Tirith with and without the experimental Jev
//! sites (ADR-0024), scored against hand-labeled cases.
//!
//! ```bash
//! cargo run --release --example jev_bench -- --sites brief --limit 5
//! ```
//!
//! Every case in `examples/jev_bench/cases/<site>.json` (format fixed in
//! the lab's `SCHEMA.md`, summarized in this directory's `README.md`) runs
//! once per arm and repeat, each time in a fresh temporary directory with
//! its own in-process daemon on an ephemeral port:
//!
//! 1. The seed is written with Jev off, so both arms start from the same
//!    state and seeding costs no Jev calls.
//! 2. For the `jev` arm, Jev is switched on (one shared, warmed-up client
//!    per process, so no case pays the TLS handshake).
//! 3. The labeled action is called once and timed client side; its
//!    structured response is measured in bytes (bytes / 4 is the token
//!    proxy used throughout).
//! 4. The prediction is read from the response (for `notice_fanout`, from
//!    the holders' inboxes, polled for three seconds in both arms) and
//!    scored against the labels.
//!
//! Output goes to `target/jev_bench/<timestamp>/` (or `--out`):
//! `per_case.jsonl`, `results.json`, and `results.md`. Nothing is tuned
//! here: thresholds are the ones in `src/assist.rs`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::error::Error;
use std::fmt::Write as _;
use std::io::Write as _;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::Parser;
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
};
use serde::Serialize;
use serde_json::{Map, Value, json};
use tirith::assist::AssistReport;
use tirith::jev::{JevClient, JevConfig};
use tirith::server::{ServeOptions, ServerHandle, start};
use tirith::state::BRIEF_LIMIT;

type BoxError = Box<dyn Error + Send + Sync>;
type Client = RunningService<RoleClient, ()>;

/// Each site and the tool its action must call.
const SITES: [(&str, &str); 9] = [
    ("brief", "claim"),
    ("notice_fanout", "notice_publish"),
    ("broadcast", "message_send"),
    ("memory_search", "memory_search"),
    ("decision_search", "decision_list"),
    ("task_duplicate", "task_create"),
    ("note_duplicate", "memory_write"),
    ("task_pull", "task_pull"),
    ("conflict_advice", "claim"),
];
/// How long holders' inboxes are watched after a notice is published.
const FANOUT_WINDOW: Duration = Duration::from_secs(3);
/// Inbox polling interval during that window.
const POLL_EVERY: Duration = Duration::from_millis(100);
/// Cut-off for the ranked search metrics.
const TOP_K: usize = 5;
/// Bootstrap resamples per confidence interval.
const RESAMPLES: usize = 1000;
/// Fixed bootstrap seed, so a rerun of the same rows gives the same CI.
const BOOT_SEED: u64 = 0x9E37_79B9_7F4A_7C15;
/// Printed in every report, because it is the premise of the numbers.
const PREMISE: &str =
    "labels written independently of Jev; thresholds from src/assist.rs unchanged";

#[derive(Debug, Parser)]
#[command(about = "Jev micro-benchmark: Tirith with and without Jev on labeled cases")]
struct Args {
    /// Directory of `<site>.json` case files [default: `examples/jev_bench/cases`].
    #[arg(long)]
    cases: Option<PathBuf>,
    /// Comma-separated sites to run [default: every site found].
    #[arg(long, value_delimiter = ',')]
    sites: Vec<String>,
    /// At most this many cases per site, in file order.
    #[arg(long)]
    limit: Option<usize>,
    /// Run every case this many times per arm (Jev may be non-deterministic).
    #[arg(long, default_value_t = 1)]
    repeat: usize,
    /// Jev provider: `gateway` or `typesafe`.
    #[arg(long, default_value = "gateway")]
    provider: String,
    /// Comma-separated arms: `off`, `jev`.
    #[arg(long, value_delimiter = ',', default_value = "off,jev")]
    arms: Vec<String>,
    /// Output directory [default: `target/jev_bench/<timestamp>`].
    #[arg(long)]
    out: Option<PathBuf>,
    /// Only load and validate the cases, print the problems, and exit.
    #[arg(long)]
    validate: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Arm {
    Off,
    Jev,
}

impl Arm {
    fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Jev => "jev",
        }
    }
}

/// One labeled case, validated.
#[derive(Debug, Clone)]
struct Case {
    site: String,
    id: String,
    difficulty: String,
    seed: Value,
    agent: String,
    tool: String,
    args: Map<String, Value>,
    expect: Value,
}

/// Jev counters spent by one action.
#[derive(Debug, Clone, Copy, Default, Serialize)]
struct Usage {
    calls: u64,
    failures: u64,
    questions: u64,
    input_tokens: u64,
    cost_usd: f64,
    total_ms: u64,
}

/// One case, one arm, one repeat.
#[derive(Debug, Serialize)]
struct Row {
    repeat: usize,
    site: String,
    case: String,
    difficulty: String,
    arm: &'static str,
    /// The action's `status` field.
    status: Option<String>,
    /// Client-side wall time of the action call alone.
    latency_ms: f64,
    /// Bytes of the action's structured content, what an agent parses.
    structured_bytes: usize,
    /// Bytes of the text blocks.
    text_bytes: usize,
    /// `structured_bytes / 4`, a rough token proxy.
    tokens_est: usize,
    jev: Usage,
    /// Jev was asked and at least one evaluation failed, so the site fell
    /// back to Tirith's deterministic behavior.
    jev_failed: bool,
    prediction: Value,
    expect: Value,
    /// Per-case numbers the aggregates are built from.
    counts: BTreeMap<&'static str, f64>,
    warnings: Vec<String>,
    error: Option<String>,
}

fn main() -> Result<(), BoxError> {
    let args = Args::parse();
    if !matches!(args.provider.as_str(), "gateway" | "typesafe") {
        return Err(format!(
            "--provider must be gateway or typesafe, not {}",
            args.provider
        )
        .into());
    }
    let arms = parse_arms(&args.arms)?;
    // `JevConfig::from_env` picks the provider from `TIRITH_JEV_PROVIDER`;
    // setting a variable in-process needs `unsafe` in edition 2024, so run
    // again as a child with the variable set.
    let wants_jev = arms.contains(&Arm::Jev) && !args.validate;
    if wants_jev
        && std::env::var("TIRITH_JEV_PROVIDER").ok().as_deref() != Some(args.provider.as_str())
    {
        let status = std::process::Command::new(std::env::current_exe()?)
            .args(std::env::args_os().skip(1))
            .env("TIRITH_JEV_PROVIDER", &args.provider)
            .status()?;
        std::process::exit(status.code().unwrap_or(1));
    }
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run(args, arms))
}

fn parse_arms(raw: &[String]) -> Result<Vec<Arm>, BoxError> {
    let mut arms = BTreeSet::new();
    for arm in raw {
        match arm.trim() {
            "off" => arms.insert(Arm::Off),
            "jev" => arms.insert(Arm::Jev),
            other => return Err(format!("unknown arm {other}; use off, jev").into()),
        };
    }
    if arms.is_empty() {
        return Err("no arms".into());
    }
    Ok(arms.into_iter().collect())
}

async fn run(args: Args, arms: Vec<Arm>) -> Result<(), BoxError> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let dir = args
        .cases
        .clone()
        .unwrap_or_else(|| root.join("examples/jev_bench/cases"));
    let (mut cases, problems, warnings) = load_cases(&dir)?;
    for problem in &problems {
        eprintln!("invalid: {problem}");
    }
    for warning in &warnings {
        eprintln!("warning: {warning}");
    }
    if !args.sites.is_empty() {
        cases.retain(|c| args.sites.iter().any(|s| s.trim() == c.site));
    }
    if let Some(limit) = args.limit {
        let mut seen: HashMap<String, usize> = HashMap::new();
        cases.retain(|c| {
            let n = seen.entry(c.site.clone()).or_default();
            *n += 1;
            *n <= limit
        });
    }
    let mut per_site: BTreeMap<&str, usize> = BTreeMap::new();
    for case in &cases {
        *per_site.entry(case.site.as_str()).or_default() += 1;
    }
    eprintln!(
        "{} valid cases ({}), {} invalid, {} warnings",
        cases.len(),
        per_site
            .iter()
            .map(|(s, n)| format!("{s} {n}"))
            .collect::<Vec<_>>()
            .join(", "),
        problems.len(),
        warnings.len()
    );
    if args.validate {
        return if problems.is_empty() {
            Ok(())
        } else {
            Err(format!("{} invalid cases", problems.len()).into())
        };
    }
    if cases.is_empty() {
        return Err(format!("no runnable cases in {}", dir.display()).into());
    }

    let jev = if arms.contains(&Arm::Jev) {
        let config = JevConfig::from_env(&root)
            .ok_or("no Jev key for the chosen provider in the environment or .env")?;
        if config.provider().as_str() != args.provider {
            return Err(format!(
                "asked for {}, configuration resolved {}",
                args.provider,
                config.provider()
            )
            .into());
        }
        let model = format!("{}/{}", config.provider(), config.model());
        let client = Arc::new(JevClient::new(config)?);
        let started = Instant::now();
        client.warm_up().await;
        eprintln!(
            "jev client for {model} warmed up in {:.0?}",
            started.elapsed()
        );
        Some((client, model))
    } else {
        None
    };
    let (jev, model) = match jev {
        Some((client, model)) => (Some(client), model),
        None => (None, String::new()),
    };

    let out = args.out.clone().unwrap_or_else(|| {
        root.join("target/jev_bench")
            .join(chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string())
    });
    std::fs::create_dir_all(&out)?;
    let mut jsonl = std::fs::File::create(out.join("per_case.jsonl"))?;

    let started = Instant::now();
    let mut rows = Vec::new();
    let total = cases.len() * args.repeat.max(1);
    for repeat in 0..args.repeat.max(1) {
        for (index, case) in cases.iter().enumerate() {
            // Alternate which arm goes first, so neither always runs on a
            // colder process or a warmer connection.
            let mut order = arms.clone();
            if (index + repeat) % 2 == 1 {
                order.reverse();
            }
            for arm in order {
                let row = run_case(case, arm, jev.as_ref(), repeat).await;
                eprintln!(
                    "[{}/{total}] {} {} {:>3}: {:>7.1} ms {:>5} B jev {}/{} {}{}",
                    repeat * cases.len() + index + 1,
                    case.site,
                    case.id,
                    arm.as_str(),
                    row.latency_ms,
                    row.structured_bytes,
                    row.jev.calls,
                    row.jev.failures,
                    short(&row.prediction.to_string(), 90),
                    row.error
                        .as_deref()
                        .map(|e| format!(" ERROR {}", short(e, 200)))
                        .unwrap_or_default(),
                );
                writeln!(jsonl, "{}", serde_json::to_string(&row)?)?;
                rows.push(row);
            }
        }
    }

    let meta = json!({
        "premise": PREMISE,
        "provider": if jev.is_some() { Some(args.provider.as_str()) } else { None },
        "model": model,
        "arms": arms.iter().map(|a| a.as_str()).collect::<Vec<_>>(),
        "repeat": args.repeat.max(1),
        "cases_dir": dir.display().to_string(),
        "cases_per_site": per_site,
        "invalid_cases": problems,
        "case_warnings": warnings,
        "wall_secs": started.elapsed().as_secs(),
        "finished_at": chrono::Utc::now().to_rfc3339(),
        "top_k": TOP_K,
        "bootstrap": {"resamples": RESAMPLES, "seed": BOOT_SEED, "interval": "percentile 95%"},
        "tokens_proxy": "tokens_est = structured bytes / 4",
        "fanout_window_ms": FANOUT_WINDOW.as_millis(),
    });
    let aggregates = aggregate(&rows, &arms);
    std::fs::write(
        out.join("results.json"),
        serde_json::to_string_pretty(&json!({
            "meta": meta, "aggregates": aggregates, "rows": rows,
        }))?,
    )?;
    let markdown = render_markdown(&meta, &aggregates);
    std::fs::write(out.join("results.md"), &markdown)?;
    eprintln!("{markdown}");
    eprintln!("wrote {}", out.display());
    Ok(())
}

// ---------------------------------------------------------------------------
// Cases

fn expected_tool(site: &str) -> Option<&'static str> {
    SITES.iter().find(|(s, _)| *s == site).map(|(_, t)| *t)
}

fn strings(value: &Value) -> Option<Vec<String>> {
    value
        .as_array()?
        .iter()
        .map(|v| v.as_str().map(str::to_owned))
        .collect()
}

fn seed_rows<'a>(seed: &'a Value, section: &str) -> &'a [Value] {
    seed.get(section)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn seed_keys(seed: &Value, sections: &[&str]) -> BTreeSet<String> {
    sections
        .iter()
        .flat_map(|s| seed_rows(seed, s))
        .filter_map(|r| r.get("key").and_then(Value::as_str).map(str::to_owned))
        .collect()
}

/// Valid cases, invalid-case lines, and warnings.
type Loaded = (Vec<Case>, Vec<String>, Vec<String>);

/// Every case file in `dir`: the valid cases, one line per invalid case,
/// and warnings about cases that run but may not measure what they mean to.
fn load_cases(dir: &Path) -> Result<Loaded, BoxError> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    let mut cases = Vec::new();
    let mut problems = Vec::new();
    let mut warnings = Vec::new();
    let mut ids = BTreeSet::new();
    for file in files {
        let name = file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let parsed: Value = match std::fs::read_to_string(&file)
            .map_err(|e| e.to_string())
            .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
        {
            Ok(v) => v,
            Err(e) => {
                problems.push(format!("{name}: {e}"));
                continue;
            }
        };
        let Some(site) = parsed["site"]
            .as_str()
            .filter(|s| expected_tool(s).is_some())
        else {
            problems.push(format!(
                "{name}: missing or unknown site {}",
                parsed["site"]
            ));
            continue;
        };
        for (index, raw) in seed_rows(&parsed, "cases").iter().enumerate() {
            let label = raw["id"]
                .as_str()
                .map_or_else(|| format!("{name}#{index}"), |id| format!("{name}:{id}"));
            match validate(site, raw, &mut warnings, &label) {
                Ok(case) if ids.insert(case.id.clone()) => cases.push(case),
                Ok(case) => problems.push(format!("{label}: duplicate id {}", case.id)),
                Err(e) => problems.push(format!("{label}: {e}")),
            }
        }
    }
    Ok((cases, problems, warnings))
}

fn validate(
    site: &str,
    raw: &Value,
    warnings: &mut Vec<String>,
    label: &str,
) -> Result<Case, String> {
    let id = raw["id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("missing id")?;
    let seed = raw.get("seed").cloned().unwrap_or_else(|| json!({}));
    let action = &raw["action"];
    let agent = action["agent"].as_str().ok_or("action.agent missing")?;
    let tool = action["tool"].as_str().ok_or("action.tool missing")?;
    if Some(tool) != expected_tool(site) {
        return Err(format!(
            "site {site} expects tool {:?}",
            expected_tool(site)
        ));
    }
    let args = match &action["args"] {
        Value::Object(map) => map.clone(),
        Value::Null => Map::new(),
        _ => return Err("action.args must be an object".into()),
    };
    let expect = raw.get("expect").cloned().unwrap_or(Value::Null);

    for (section, required) in [
        ("memory", &["key", "by", "title", "body"][..]),
        ("decisions", &["key", "by", "title", "decision"][..]),
        ("contracts", &["key", "by", "name", "kind"][..]),
        ("notices", &["key", "by", "kind", "summary"][..]),
        ("tasks", &["key", "by", "title"][..]),
        ("claims", &["agent", "paths"][..]),
    ] {
        for (i, row) in seed_rows(&seed, section).iter().enumerate() {
            for field in required {
                if row.get(*field).is_none_or(Value::is_null) {
                    return Err(format!("seed.{section}[{i}] lacks {field}"));
                }
            }
        }
    }
    let all_keys = seed_keys(
        &seed,
        &["memory", "decisions", "contracts", "notices", "tasks"],
    );
    let key_count: usize = ["memory", "decisions", "contracts", "notices", "tasks"]
        .iter()
        .map(|s| seed_rows(&seed, s).len())
        .sum();
    if all_keys.len() != key_count {
        return Err("seed keys are missing or not unique".into());
    }
    let titles: Vec<&str> = seed_rows(&seed, "memory")
        .iter()
        .filter_map(|m| m["title"].as_str())
        .collect();
    if titles.iter().collect::<BTreeSet<_>>().len() != titles.len() {
        return Err("two seeded memory notes share a title (the second would overwrite)".into());
    }
    let claimers: BTreeSet<String> = seed_rows(&seed, "claims")
        .iter()
        .filter_map(|c| c["agent"].as_str().map(str::to_owned))
        .collect();
    let active: BTreeSet<String> = strings(&seed["active_agents"])
        .unwrap_or_default()
        .into_iter()
        .collect();

    let list = |field: &str| -> Result<Vec<String>, String> {
        strings(&expect[field]).ok_or_else(|| format!("expect.{field} must be a string array"))
    };
    let known = |keys: &[String], within: &BTreeSet<String>, what: &str| -> Result<(), String> {
        match keys.iter().find(|k| !within.contains(*k)) {
            Some(k) => Err(format!("label {k} is not a seeded {what}")),
            None => Ok(()),
        }
    };
    match site {
        "brief" => {
            let must = list("must_read")?;
            known(
                &must,
                &seed_keys(&seed, &["notices", "contracts", "decisions", "memory"]),
                "notice/contract/decision/memory key",
            )?;
            args.get("paths")
                .and_then(strings)
                .filter(|p| !p.is_empty())
                .ok_or("action.args.paths required")?;
        }
        "notice_fanout" => {
            let affected = list("affected")?;
            known(&affected, &claimers, "claim holder")?;
            if claimers.iter().all(|c| c == agent) {
                warnings.push(format!("{label}: no holders besides the publisher"));
            }
        }
        "broadcast" => {
            let needs = list("needs")?;
            let reachable: BTreeSet<String> = active.union(&claimers).cloned().collect();
            known(&needs, &reachable, "active agent or claim holder")?;
            if args.get("to").and_then(Value::as_str) != Some("*") {
                return Err("broadcast action needs args.to = \"*\"".into());
            }
            // Every seeding call marks its author as seen, so authors join
            // the baseline audience whether or not the labeler meant them to.
            let authors: BTreeSet<String> =
                ["memory", "decisions", "contracts", "notices", "tasks"]
                    .iter()
                    .flat_map(|s| seed_rows(&seed, s))
                    .filter_map(|r| r["by"].as_str().map(str::to_owned))
                    .filter(|a| a != agent && !reachable.contains(a))
                    .collect();
            if !authors.is_empty() {
                warnings.push(format!(
                    "{label}: seed authors {authors:?} are not in active_agents but will be in the audience"
                ));
            }
        }
        "memory_search" | "decision_search" => {
            let section = if site == "memory_search" {
                "memory"
            } else {
                "decisions"
            };
            let within = seed_keys(&seed, &[section]);
            known(&list("relevant")?, &within, section)?;
            if !expect["partial"].is_null() {
                known(&list("partial")?, &within, section)?;
            }
            args.get("query")
                .and_then(Value::as_str)
                .filter(|q| !q.trim().is_empty())
                .ok_or("action.args.query required")?;
        }
        "task_duplicate" | "note_duplicate" => {
            let section = if site == "task_duplicate" {
                "tasks"
            } else {
                "memory"
            };
            match expect.get("duplicate_of") {
                Some(Value::Null) => {}
                Some(Value::String(k)) => {
                    known(
                        std::slice::from_ref(k),
                        &seed_keys(&seed, &[section]),
                        section,
                    )?;
                }
                _ => return Err("expect.duplicate_of must be a key or null".into()),
            }
            if site == "note_duplicate"
                && args
                    .get("title")
                    .and_then(Value::as_str)
                    .is_some_and(|t| titles.contains(&t))
            {
                return Err("memory_write title equals a seeded title (update, not create)".into());
            }
        }
        "task_pull" => {
            let tasks = seed_keys(&seed, &["tasks"]);
            let best = list("best")?;
            if best.is_empty() {
                return Err("expect.best is empty".into());
            }
            known(&best, &tasks, "task")?;
            known(&list("acceptable")?, &tasks, "task")?;
        }
        "conflict_advice" => {
            let acceptable = list("acceptable")?;
            if let Some(a) = acceptable.iter().find(|a| {
                !matches!(
                    a.as_str(),
                    "wait" | "work_elsewhere" | "coordinate" | "narrow_claim" | "none"
                )
            }) {
                return Err(format!("unknown advice {a}"));
            }
            if claimers.is_empty() {
                return Err("conflict case seeds no claims".into());
            }
        }
        _ => return Err(format!("unknown site {site}")),
    }
    Ok(Case {
        site: site.to_owned(),
        id: id.to_owned(),
        difficulty: raw["difficulty"].as_str().unwrap_or("unknown").to_owned(),
        seed,
        agent: agent.to_owned(),
        tool: tool.to_owned(),
        args,
        expect,
    })
}

// ---------------------------------------------------------------------------
// Running

async fn connect(url: &str) -> Result<Client, BoxError> {
    let config = StreamableHttpClientTransportConfig::with_uri(url.to_owned());
    let transport = StreamableHttpClientTransport::with_client(reqwest::Client::default(), config);
    Ok(().serve(transport).await?)
}

/// One tool call, timed and measured.
struct Outcome {
    value: Value,
    latency: Duration,
    structured_bytes: usize,
    text_bytes: usize,
}

async fn call(client: &Client, tool: &str, args: Value) -> Result<Outcome, BoxError> {
    let Value::Object(map) = args else {
        return Err("arguments must be an object".into());
    };
    let started = Instant::now();
    let result = client
        .call_tool(CallToolRequestParams::new(tool.to_owned()).with_arguments(map))
        .await?;
    let latency = started.elapsed();
    let structured_bytes = result
        .structured_content
        .as_ref()
        .map_or(0, |v| v.to_string().len());
    let text_bytes = serde_json::to_string(&result.content)?.len();
    Ok(Outcome {
        value: result.structured_content.unwrap_or(Value::Null),
        latency,
        structured_bytes,
        text_bytes,
    })
}

async fn seed_call(client: &Client, tool: &str, args: Value) -> Result<Value, BoxError> {
    let value = call(client, tool, args).await?.value;
    if value["status"] == "ok" {
        Ok(value)
    } else {
        Err(format!("seed {tool}: {}", short(&value.to_string(), 300)).into())
    }
}

/// Maps what Tirith returns (ids, id prefixes, names, permalinks) back to
/// the labelers' keys.
#[derive(Debug, Default)]
struct KeyMap {
    exact: HashMap<String, String>,
    ids: Vec<(String, String)>,
}

impl KeyMap {
    fn name(&mut self, name: &str, key: &str) {
        self.exact.insert(name.to_owned(), key.to_owned());
    }

    fn id(&mut self, id: &str, key: &str) {
        self.exact.insert(id.to_owned(), key.to_owned());
        self.ids.push((id.to_owned(), key.to_owned()));
    }

    fn key(&self, raw: &str) -> Option<String> {
        if let Some(key) = self.exact.get(raw) {
            return Some(key.clone());
        }
        if raw.len() < 8 {
            return None;
        }
        let hits: BTreeSet<&String> = self
            .ids
            .iter()
            .filter(|(id, _)| id.starts_with(raw))
            .map(|(_, key)| key)
            .collect();
        match hits.into_iter().collect::<Vec<_>>().as_slice() {
            [one] => Some((*one).clone()),
            _ => None,
        }
    }

    /// The key, or `unmapped:<raw>` so an unknown row still counts as shown.
    fn key_or_raw(&self, raw: &str) -> String {
        self.key(raw).unwrap_or_else(|| format!("unmapped:{raw}"))
    }
}

fn field(row: &Value, name: &str) -> Value {
    row.get(name).cloned().unwrap_or(Value::Null)
}

fn by(row: &Value) -> Value {
    field(row, "by")
}

/// Writes `seed` in the order the schema fixes and returns the key map.
async fn seed(client: &Client, seed: &Value) -> Result<KeyMap, BoxError> {
    let mut keys = KeyMap::default();
    for m in seed_rows(seed, "memory") {
        let key = m["key"].as_str().unwrap_or_default();
        let out = seed_call(
            client,
            "memory_write",
            json!({
                "agent": by(m), "title": field(m, "title"), "body": field(m, "body"),
                "kind": field(m, "kind"), "paths": field(m, "paths"), "tags": field(m, "tags"),
            }),
        )
        .await?;
        if out["created"] != true {
            return Err(format!("memory {key} did not create a new note").into());
        }
        if let Some(p) = out["note"]["permalink"].as_str() {
            keys.name(p, key);
        }
        if let Some(id) = out["note"]["id"].as_str() {
            keys.id(id, key);
        }
    }
    for d in seed_rows(seed, "decisions") {
        let key = d["key"].as_str().unwrap_or_default();
        let out = seed_call(
            client,
            "decision_record",
            json!({
                "agent": by(d), "title": field(d, "title"), "decision": field(d, "decision"),
                "rationale": field(d, "rationale"), "alternatives": field(d, "alternatives"),
                "affects_paths": field(d, "affects_paths"),
            }),
        )
        .await?;
        if let Some(id) = out["decision"]["id"].as_str() {
            keys.id(id, key);
        }
        if let Some(p) = out["decision"]["permalink"].as_str() {
            keys.name(p, key);
        }
    }
    for c in seed_rows(seed, "contracts") {
        let key = c["key"].as_str().unwrap_or_default();
        let shape = c.get("shape").cloned().unwrap_or_else(|| json!({}));
        let out = seed_call(
            client,
            "contract_publish",
            json!({
                "agent": by(c), "name": field(c, "name"), "kind": field(c, "kind"),
                "shape": shape, "consumers": field(c, "consumers"), "notes": field(c, "notes"),
            }),
        )
        .await?;
        if let Some(name) = c["name"].as_str() {
            keys.name(name, key);
        }
        if let Some(id) = out["contract"]["id"].as_str() {
            keys.id(id, key);
        }
        // A republish emits a `contract` notice. It has no labeler key, so
        // it stays unkeyed and is counted apart from the scored rows.
    }
    for n in seed_rows(seed, "notices") {
        let key = n["key"].as_str().unwrap_or_default();
        let out = seed_call(
            client,
            "notice_publish",
            json!({
                "agent": by(n), "kind": field(n, "kind"), "summary": field(n, "summary"),
                "from": field(n, "from"), "to": field(n, "to"),
                "affected_paths": n.get("affected_paths").cloned().unwrap_or_else(|| json!([])),
            }),
        )
        .await?;
        if let Some(id) = out["notice"]["id"].as_str() {
            keys.id(id, key);
        }
    }
    let mut task_ids: HashMap<String, String> = HashMap::new();
    for t in seed_rows(seed, "tasks") {
        let key = t["key"].as_str().unwrap_or_default();
        let depends: Vec<Value> = strings(&t["depends_on"])
            .unwrap_or_default()
            .iter()
            .map(|k| Value::from(task_ids.get(k).cloned().unwrap_or_else(|| k.clone())))
            .collect();
        let out = seed_call(
            client,
            "task_create",
            json!({
                "agent": by(t), "title": field(t, "title"), "description": field(t, "description"),
                "priority": field(t, "priority"), "paths": field(t, "paths"), "depends_on": depends,
            }),
        )
        .await?;
        let id = out["task"]["id"]
            .as_str()
            .ok_or("task_create returned no id")?;
        keys.id(id, key);
        task_ids.insert(key.to_owned(), id.to_owned());
    }
    // Pulls are replayed as `task_update in_progress` on the named task, so
    // the seeded board never depends on pull order or on Jev.
    for t in seed_rows(seed, "tasks") {
        let id = task_ids
            .get(t["key"].as_str().unwrap_or_default())
            .cloned()
            .unwrap_or_default();
        if let Some(agent) = t["pulled_by"].as_str() {
            seed_call(
                client,
                "task_update",
                json!({"agent": agent, "task_id": id, "status": "in_progress"}),
            )
            .await?;
        }
        if let Some(agent) = t["done_by"].as_str() {
            seed_call(
                client,
                "task_update",
                json!({"agent": agent, "task_id": id, "status": "done", "force": true}),
            )
            .await?;
        }
    }
    for c in seed_rows(seed, "claims") {
        seed_call(
            client,
            "claim",
            json!({
                "agent": field(c, "agent"), "paths": field(c, "paths"),
                "reason": c.get("reason").cloned().unwrap_or_else(|| json!("work")),
                "ttl_secs": 3600, "brief": false,
            }),
        )
        .await?;
    }
    for agent in strings(&seed["active_agents"]).unwrap_or_default() {
        seed_call(client, "status", json!({ "agent": agent })).await?;
    }
    Ok(keys)
}

/// Jev spent between two reports, summed over sites.
fn usage(before: Option<&AssistReport>, after: Option<&AssistReport>) -> Usage {
    let Some(after) = after else {
        return Usage::default();
    };
    let mut usage = Usage::default();
    for (site, stats) in &after.sites {
        let old = before.and_then(|b| b.sites.get(site)).copied();
        let old = old.unwrap_or_default();
        usage.calls += stats.calls.saturating_sub(old.calls);
        usage.failures += stats.failures.saturating_sub(old.failures);
        usage.questions += stats.questions.saturating_sub(old.questions);
        usage.input_tokens += stats.input_tokens.saturating_sub(old.input_tokens);
        usage.cost_usd += (stats.cost_usd - old.cost_usd).max(0.0);
        usage.total_ms += stats.total_ms.saturating_sub(old.total_ms);
    }
    usage
}

async fn run_case(case: &Case, arm: Arm, jev: Option<&Arc<JevClient>>, repeat: usize) -> Row {
    let mut row = Row {
        repeat,
        site: case.site.clone(),
        case: case.id.clone(),
        difficulty: case.difficulty.clone(),
        arm: arm.as_str(),
        status: None,
        latency_ms: 0.0,
        structured_bytes: 0,
        text_bytes: 0,
        tokens_est: 0,
        jev: Usage::default(),
        jev_failed: false,
        prediction: Value::Null,
        expect: case.expect.clone(),
        counts: BTreeMap::new(),
        warnings: Vec::new(),
        error: None,
    };
    if let Err(error) = run_case_in_daemon(case, arm, jev, &mut row).await {
        row.error = Some(error.to_string());
    }
    row
}

async fn run_case_in_daemon(
    case: &Case,
    arm: Arm,
    jev: Option<&Arc<JevClient>>,
    row: &mut Row,
) -> Result<(), BoxError> {
    let dir = tempfile::tempdir()?;
    let handle = start(ServeOptions {
        bind: "127.0.0.1:0".parse::<SocketAddr>()?,
        repo_root: dir.path().to_path_buf(),
        clock: None,
        registry: None,
    })
    .await?;
    let result = drive(&handle, case, arm, jev, row).await;
    if let Err(error) = handle.shutdown().await {
        row.warnings.push(format!("shutdown: {error}"));
    }
    result
}

async fn drive(
    handle: &ServerHandle,
    case: &Case,
    arm: Arm,
    jev: Option<&Arc<JevClient>>,
    row: &mut Row,
) -> Result<(), BoxError> {
    let client = connect(&handle.mcp_url()).await?;
    let keys = seed(&client, &case.seed).await?;
    if arm == Arm::Jev {
        let evaluator = jev.ok_or("jev arm without a Jev client")?;
        handle.enable_assist(Arc::clone(evaluator) as Arc<dyn tirith::jev::Evaluator>);
    }
    let before = handle.assist_report();

    let mut args = case.args.clone();
    args.insert("agent".to_owned(), Value::from(case.agent.clone()));
    if let Some(depends) = args.get("depends_on").and_then(strings) {
        let mapped: Vec<Value> = depends
            .iter()
            .map(|k| {
                let id = keys.ids.iter().find(|(_, key)| key == k).map(|(id, _)| id);
                Value::from(id.cloned().unwrap_or_else(|| k.clone()))
            })
            .collect();
        args.insert("depends_on".to_owned(), Value::from(mapped));
    }
    let action_started = Instant::now();
    let outcome = call(&client, &case.tool, Value::Object(args)).await?;
    row.latency_ms = outcome.latency.as_secs_f64() * 1000.0;
    row.structured_bytes = outcome.structured_bytes;
    row.text_bytes = outcome.text_bytes;
    row.tokens_est = outcome.structured_bytes / 4;
    row.status = outcome.value["status"].as_str().map(str::to_owned);

    let pushes = if case.site == "notice_fanout" {
        Some(poll_pushes(&client, &holders(case), action_started).await?)
    } else {
        None
    };
    row.jev = usage(before.as_ref(), handle.assist_report().as_ref());
    row.jev_failed = row.jev.failures > 0;
    let _ = client.cancel().await;

    let value = &outcome.value;
    let expected_status = if case.site == "conflict_advice" {
        "conflict"
    } else {
        "ok"
    };
    if row.status.as_deref() != Some(expected_status) {
        row.warnings.push(format!(
            "action status {:?}, expected {expected_status}: {}",
            row.status,
            short(&value.to_string(), 200)
        ));
    }
    if arm == Arm::Jev && row.jev.calls == 0 {
        row.warnings.push("jev arm made no Jev call".to_owned());
    }
    let (prediction, counts) = score(case, value, &keys, pushes.as_ref());
    row.prediction = prediction;
    row.counts = counts;
    row.counts.insert("bytes", count(row.structured_bytes));
    row.counts.insert("latency_ms", row.latency_ms);
    Ok(())
}

/// Agents holding claims other than the publisher, who can receive a push.
fn holders(case: &Case) -> Vec<String> {
    seed_rows(&case.seed, "claims")
        .iter()
        .filter_map(|c| c["agent"].as_str())
        .filter(|a| *a != case.agent)
        .map(str::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Polls every holder's inbox for a push from `tirith` for the whole
/// window, in both arms, and returns who got one and after how long.
async fn poll_pushes(
    client: &Client,
    holders: &[String],
    since: Instant,
) -> Result<BTreeMap<String, f64>, BoxError> {
    let mut pushed = BTreeMap::new();
    while since.elapsed() < FANOUT_WINDOW {
        for holder in holders {
            if pushed.contains_key(holder) {
                continue;
            }
            let out = call(client, "status", json!({ "agent": holder })).await?;
            let from_tirith = out
                .value
                .get("inbox")
                .and_then(Value::as_array)
                .is_some_and(|rows| rows.iter().any(|m| m["from"] == "tirith"));
            if from_tirith {
                pushed.insert(holder.clone(), since.elapsed().as_secs_f64() * 1000.0);
            }
        }
        tokio::time::sleep(POLL_EVERY).await;
    }
    Ok(pushed)
}

// ---------------------------------------------------------------------------
// Scoring one case

fn count(n: usize) -> f64 {
    f64::from(u32::try_from(n).unwrap_or(u32::MAX))
}

fn flag(b: bool) -> f64 {
    if b { 1.0 } else { 0.0 }
}

fn label_set(expect: &Value, field: &str) -> BTreeSet<String> {
    strings(&expect[field])
        .unwrap_or_default()
        .into_iter()
        .collect()
}

fn column(value: &Value, rows: &str, id: &str, keys: &KeyMap) -> Vec<String> {
    value[rows]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|r| r[id].as_str())
                .map(|raw| keys.key_or_raw(raw))
                .collect()
        })
        .unwrap_or_default()
}

/// The prediction the action made and the per-case counts for its site.
fn score(
    case: &Case,
    value: &Value,
    keys: &KeyMap,
    pushes: Option<&BTreeMap<String, f64>>,
) -> (Value, BTreeMap<&'static str, f64>) {
    let mut counts = BTreeMap::new();
    let expect = &case.expect;
    let prediction = match case.site.as_str() {
        "brief" => {
            let mut shown = Vec::new();
            let mut unkeyed = Vec::new();
            let mut size_dropped = Vec::new();
            for (section, id) in [
                ("notices", "id"),
                ("contracts", "name"),
                ("decisions", "id"),
                ("memory", "permalink"),
            ] {
                let rows = column(value, section, id, keys);
                // A section shows up to BRIEF_LIMIT rows; fewer rows
                // with a non-zero `more` means the 4,096-byte cap cut it.
                let more = value["more"][section].as_u64().unwrap_or(0);
                if more > 0 && rows.len() < BRIEF_LIMIT {
                    size_dropped.push(section);
                }
                for row in rows {
                    if row.starts_with("unmapped:") {
                        unkeyed.push(row);
                    } else {
                        shown.push(row);
                    }
                }
            }
            let set: BTreeSet<String> = shown.iter().cloned().collect();
            let must = label_set(expect, "must_read");
            counts.insert("must", count(must.len()));
            counts.insert("shown", count(set.len()));
            counts.insert("hit", count(set.intersection(&must).count()));
            counts.insert("unkeyed", count(unkeyed.len()));
            counts.insert("size_dropped", flag(!size_dropped.is_empty()));
            json!({
                "shown": shown,
                "missed": must.difference(&set).collect::<Vec<_>>(),
                "unkeyed_shown": unkeyed,
                "size_cap_dropped_sections": size_dropped,
                "skipped": field(value, "skipped"),
                "more": field(value, "more"),
            })
        }
        "notice_fanout" | "broadcast" => {
            let (predicted, label, extra): (BTreeSet<String>, _, _) =
                if case.site == "notice_fanout" {
                    let received = pushes.cloned().unwrap_or_default();
                    (
                        received.keys().cloned().collect(),
                        label_set(expect, "affected"),
                        json!({ "push_ms": received, "holders": holders(case) }),
                    )
                } else {
                    (
                        strings(&value["message"]["audience"])
                            .unwrap_or_default()
                            .into_iter()
                            .collect(),
                        label_set(expect, "needs"),
                        json!({ "skipped_recipients": field(value, "skipped_recipients") }),
                    )
                };
            counts.insert("label", count(label.len()));
            counts.insert("pred", count(predicted.len()));
            counts.insert("hit", count(predicted.intersection(&label).count()));
            if let Some(slowest) = pushes
                .filter(|_| case.site == "notice_fanout")
                .and_then(|p| p.values().copied().reduce(f64::max))
            {
                counts.insert("push_ms", slowest);
            }
            json!({ "predicted": predicted, "detail": extra })
        }
        "memory_search" | "decision_search" => {
            let ranked = if case.site == "memory_search" {
                column(value, "notes", "permalink", keys)
            } else {
                column(value, "decisions", "id", keys)
            };
            let relevant = label_set(expect, "relevant");
            let partial = label_set(expect, "partial");
            let top: Vec<&String> = ranked.iter().take(TOP_K).collect();
            let hit5 = top.iter().filter(|k| relevant.contains(**k)).count();
            let lenient = top
                .iter()
                .filter(|k| relevant.contains(**k) || partial.contains(**k))
                .count();
            let rank = ranked.iter().position(|k| relevant.contains(k));
            counts.insert("rel", count(relevant.len()));
            counts.insert("top", count(top.len()));
            counts.insert("rows", count(ranked.len()));
            counts.insert("hit5", count(hit5));
            counts.insert("hit5_lenient", count(lenient));
            counts.insert(
                "hit_all",
                count(ranked.iter().filter(|k| relevant.contains(*k)).count()),
            );
            counts.insert("rr", rank.map_or(0.0, |r| 1.0 / count(r + 1)));
            json!({ "ranked": ranked, "ranked_by": field(value, "ranked_by") })
        }
        "task_duplicate" | "note_duplicate" => {
            let (object, id) = if case.site == "task_duplicate" {
                ("possible_duplicate", "id")
            } else {
                ("similar_note", "permalink")
            };
            let predicted = value[object][id].as_str().map(|raw| keys.key_or_raw(raw));
            let label = expect["duplicate_of"].as_str().map(str::to_owned);
            counts.insert("label_dup", flag(label.is_some()));
            counts.insert("pred_dup", flag(predicted.is_some()));
            counts.insert("correct", flag(predicted == label));
            counts.insert("tp", flag(label.is_some() && predicted == label));
            json!({ "duplicate_of": predicted, "confidence": field(&value[object], "confidence") })
        }
        "task_pull" => {
            let pulled = value["task"]["id"].as_str().map(|raw| keys.key_or_raw(raw));
            let in_label = |f: &str| {
                pulled
                    .as_ref()
                    .is_some_and(|p| label_set(expect, f).contains(p))
            };
            counts.insert("best", flag(in_label("best")));
            counts.insert("acceptable", flag(in_label("acceptable")));
            counts.insert("jev_pick", flag(value["picked_by"] == "jev"));
            counts.insert("none", flag(pulled.is_none()));
            json!({ "pulled": pulled, "picked_by": field(value, "picked_by") })
        }
        "conflict_advice" => {
            let action = value["advice"]["action"].as_str().map(str::to_owned);
            let acceptable = label_set(expect, "acceptable");
            let ok = acceptable.contains(action.as_deref().unwrap_or("none"));
            counts.insert("advised", flag(action.is_some()));
            counts.insert("acceptable", flag(ok));
            json!({ "action": action, "confidence": field(&value["advice"], "confidence") })
        }
        _ => Value::Null,
    };
    (prediction, counts)
}

// ---------------------------------------------------------------------------
// Aggregates

type Metrics = Vec<(&'static str, Option<f64>)>;

fn get(row: &Row, name: &str) -> f64 {
    row.counts.get(name).copied().unwrap_or(0.0)
}

fn sum(rows: &[&Row], name: &str) -> f64 {
    // `Iterator::sum` of no floats is -0.0, which prints as "-0.000".
    rows.iter().fold(0.0, |acc, r| acc + get(r, name))
}

fn ratio(num: f64, den: f64) -> Option<f64> {
    (den > 0.0).then(|| num / den)
}

/// Mean of `value` over the rows where `when` holds; `None` if none do.
fn mean_where(
    rows: &[&Row],
    when: impl Fn(&Row) -> bool,
    value: impl Fn(&Row) -> f64,
) -> Option<f64> {
    let picked: Vec<f64> = rows.iter().filter(|r| when(r)).map(|r| value(r)).collect();
    ratio(picked.iter().fold(0.0, |a, b| a + b), count(picked.len()))
}

/// Micro F1 as 2·TP / (predicted + labeled), so an arm that predicts
/// nothing scores 0 instead of having no precision.
fn f1_micro(rows: &[&Row]) -> Option<f64> {
    ratio(
        2.0 * sum(rows, "hit"),
        sum(rows, "pred") + sum(rows, "label"),
    )
}

/// The metric the bootstrap interval is computed for.
fn headline(site: &str) -> &'static str {
    match site {
        "brief" => "recall_macro",
        "notice_fanout" | "broadcast" => "f1_micro",
        "memory_search" | "decision_search" => "recall_at5_macro",
        "task_duplicate" | "note_duplicate" => "accuracy",
        "task_pull" => "top1_best_rate",
        "conflict_advice" => "acceptable_rate",
        _ => "",
    }
}

/// Every metric of `site` over `rows` (one arm, any number of repeats).
fn metrics(site: &str, rows: &[&Row]) -> Metrics {
    let all = |_: &Row| true;
    let bytes = mean_where(rows, all, |r| get(r, "bytes"));
    let mut out: Metrics = match site {
        "brief" => {
            let labeled = |r: &Row| get(r, "must") > 0.0;
            vec![
                (
                    "recall_macro",
                    mean_where(rows, labeled, |r| get(r, "hit") / get(r, "must")),
                ),
                ("recall_micro", ratio(sum(rows, "hit"), sum(rows, "must"))),
                (
                    "all_must_read_shown_rate",
                    mean_where(rows, labeled, |r| flag(get(r, "hit") >= get(r, "must"))),
                ),
                (
                    "precision_micro",
                    ratio(sum(rows, "hit"), sum(rows, "shown")),
                ),
                (
                    "precision_macro",
                    mean_where(
                        rows,
                        |r| get(r, "shown") > 0.0,
                        |r| get(r, "hit") / get(r, "shown"),
                    ),
                ),
                (
                    "rows_shown_per_empty_label_case",
                    mean_where(rows, |r| get(r, "must") == 0.0, |r| get(r, "shown")),
                ),
                (
                    "rows_shown_mean",
                    mean_where(rows, all, |r| get(r, "shown")),
                ),
                ("unkeyed_rows_shown_total", Some(sum(rows, "unkeyed"))),
                (
                    "size_cap_dropped_rate",
                    mean_where(rows, all, |r| get(r, "size_dropped")),
                ),
            ]
        }
        "notice_fanout" | "broadcast" => {
            let p = ratio(sum(rows, "hit"), sum(rows, "pred"));
            let r = ratio(sum(rows, "hit"), sum(rows, "label"));
            vec![
                ("precision_micro", p),
                ("recall_micro", r),
                ("f1_micro", f1_micro(rows)),
                (
                    "recall_macro",
                    mean_where(
                        rows,
                        |r| get(r, "label") > 0.0,
                        |r| get(r, "hit") / get(r, "label"),
                    ),
                ),
                ("recipients_mean", mean_where(rows, all, |r| get(r, "pred"))),
                (
                    "slowest_push_ms_mean",
                    mean_where(
                        rows,
                        |r| r.counts.contains_key("push_ms"),
                        |r| get(r, "push_ms"),
                    ),
                ),
                (
                    "false_positives_per_empty_label_case",
                    mean_where(rows, |r| get(r, "label") == 0.0, |r| get(r, "pred")),
                ),
            ]
        }
        "memory_search" | "decision_search" => {
            let labeled = |r: &Row| get(r, "rel") > 0.0;
            let mut m = vec![
                (
                    "recall_at5_macro",
                    mean_where(rows, labeled, |r| get(r, "hit5") / get(r, "rel")),
                ),
                (
                    "precision_at5_macro",
                    mean_where(
                        rows,
                        |r| labeled(r) && get(r, "top") > 0.0,
                        |r| get(r, "hit5") / get(r, "top"),
                    ),
                ),
                (
                    "precision_at5_lenient_macro",
                    mean_where(
                        rows,
                        |r| labeled(r) && get(r, "top") > 0.0,
                        |r| get(r, "hit5_lenient") / get(r, "top"),
                    ),
                ),
                ("mrr", mean_where(rows, labeled, |r| get(r, "rr"))),
                (
                    "empty_result_rate",
                    mean_where(rows, labeled, |r| flag(get(r, "rows") == 0.0)),
                ),
                (
                    "false_positives_per_empty_label_case",
                    mean_where(rows, |r| !labeled(r), |r| get(r, "top")),
                ),
                ("rows_mean", mean_where(rows, all, |r| get(r, "rows"))),
            ];
            if site == "decision_search" {
                m.push((
                    "recall_all_rows_macro",
                    mean_where(rows, labeled, |r| get(r, "hit_all") / get(r, "rel")),
                ));
                m.push((
                    "precision_all_rows_micro",
                    ratio(sum(rows, "hit_all"), sum(rows, "rows")),
                ));
            }
            m
        }
        "task_duplicate" | "note_duplicate" => vec![
            ("accuracy", mean_where(rows, all, |r| get(r, "correct"))),
            ("precision", ratio(sum(rows, "tp"), sum(rows, "pred_dup"))),
            ("recall", ratio(sum(rows, "tp"), sum(rows, "label_dup"))),
            (
                "false_positive_rate",
                mean_where(rows, |r| get(r, "label_dup") == 0.0, |r| get(r, "pred_dup")),
            ),
            (
                "wrong_target_count",
                Some(count(
                    rows.iter()
                        .filter(|r| {
                            get(r, "label_dup") > 0.0
                                && get(r, "pred_dup") > 0.0
                                && get(r, "correct") == 0.0
                        })
                        .count(),
                )),
            ),
        ],
        "task_pull" => vec![
            ("top1_best_rate", mean_where(rows, all, |r| get(r, "best"))),
            (
                "top1_acceptable_rate",
                mean_where(rows, all, |r| get(r, "acceptable")),
            ),
            (
                "jev_pick_rate",
                mean_where(rows, all, |r| get(r, "jev_pick")),
            ),
            (
                "nothing_pulled_rate",
                mean_where(rows, all, |r| get(r, "none")),
            ),
        ],
        "conflict_advice" => vec![
            (
                "acceptable_rate",
                mean_where(rows, all, |r| get(r, "acceptable")),
            ),
            ("coverage", mean_where(rows, all, |r| get(r, "advised"))),
            (
                "acceptable_given_advice",
                mean_where(rows, |r| get(r, "advised") > 0.0, |r| get(r, "acceptable")),
            ),
        ],
        _ => Vec::new(),
    };
    out.push(("bytes_mean", bytes));
    out.push(("tokens_est_mean", bytes.map(|b| b / 4.0)));
    out
}

fn lookup(metrics: &Metrics, name: &str) -> Option<f64> {
    metrics
        .iter()
        .find(|(n, _)| *n == name)
        .and_then(|(_, v)| *v)
}

/// Tiny xorshift64*, enough for resampling without a dependency.
struct XorShift(u64);

impl XorShift {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let x = self.0.wrapping_mul(0x2545_F491_4F6C_DD1D);
        let n64 = u64::try_from(n).unwrap_or(u64::MAX).max(1);
        usize::try_from(x % n64).unwrap_or(0)
    }
}

fn percentile_f64(sorted: &[f64], num: usize, den: usize) -> Option<f64> {
    match sorted.len() {
        0 => None,
        len => Some(sorted[(len - 1) * num / den]),
    }
}

/// 95% percentile bootstrap over cases of `f`, which scores a resample of
/// case ids. Resamples where `f` is undefined are dropped.
fn bootstrap(case_ids: &[String], f: impl Fn(&[&String]) -> Option<f64>) -> Value {
    if case_ids.is_empty() {
        return Value::Null;
    }
    let mut rng = XorShift(BOOT_SEED);
    let mut values: Vec<f64> = (0..RESAMPLES)
        .filter_map(|_| {
            let sample: Vec<&String> = (0..case_ids.len())
                .map(|_| &case_ids[rng.below(case_ids.len())])
                .collect();
            f(&sample)
        })
        .collect();
    values.sort_by(f64::total_cmp);
    json!({
        "low": percentile_f64(&values, 25, 1000),
        "high": percentile_f64(&values, 975, 1000),
        "defined_resamples": values.len(),
    })
}

fn rows_for<'a>(rows: &[&'a Row], sample: &[&String]) -> Vec<&'a Row> {
    let mut by_case: HashMap<&str, Vec<&'a Row>> = HashMap::new();
    for row in rows {
        by_case.entry(row.case.as_str()).or_default().push(row);
    }
    sample
        .iter()
        .flat_map(|id| by_case.get(id.as_str()).cloned().unwrap_or_default())
        .collect()
}

fn summary(values: &[Option<f64>]) -> Value {
    let defined: Vec<f64> = values.iter().flatten().copied().collect();
    if defined.is_empty() {
        return Value::Null;
    }
    let mean = defined.iter().sum::<f64>() / count(defined.len());
    let min = defined.iter().copied().fold(f64::INFINITY, f64::min);
    let max = defined.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    json!({ "mean": mean, "min": min, "max": max, "repeats_defined": defined.len() })
}

fn aggregate(rows: &[Row], arms: &[Arm]) -> Value {
    let mut sites = Map::new();
    let names: BTreeSet<&str> = rows.iter().map(|r| r.site.as_str()).collect();
    let repeats = rows.iter().map(|r| r.repeat + 1).max().unwrap_or(1);
    for site in names {
        let site_rows: Vec<&Row> = rows.iter().filter(|r| r.site == site).collect();
        let case_ids: Vec<String> = site_rows
            .iter()
            .map(|r| r.case.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let mut per_arm = Map::new();
        let mut scored: HashMap<Arm, Vec<&Row>> = HashMap::new();
        for &arm in arms {
            let arm_rows: Vec<&Row> = site_rows
                .iter()
                .copied()
                .filter(|r| r.arm == arm.as_str())
                .collect();
            let ok: Vec<&Row> = arm_rows
                .iter()
                .copied()
                .filter(|r| r.error.is_none())
                .collect();
            // Metrics per repeat, then mean and range across repeats.
            let per_repeat: Vec<Metrics> = (0..repeats)
                .map(|rep| {
                    let subset: Vec<&Row> =
                        ok.iter().copied().filter(|r| r.repeat == rep).collect();
                    metrics(site, &subset)
                })
                .collect();
            let mut metric_values = Map::new();
            for (name, _) in metrics(site, &ok) {
                let values: Vec<Option<f64>> = per_repeat.iter().map(|m| lookup(m, name)).collect();
                metric_values.insert(name.to_owned(), summary(&values));
            }
            let head = headline(site);
            let ci = bootstrap(&case_ids, |sample| {
                lookup(&metrics(site, &rows_for(&ok, sample)), head)
            });
            let mut latencies: Vec<f64> = ok.iter().map(|r| r.latency_ms).collect();
            latencies.sort_by(f64::total_cmp);
            let jev = arm_rows.iter().fold(Usage::default(), |mut u, r| {
                u.calls += r.jev.calls;
                u.failures += r.jev.failures;
                u.questions += r.jev.questions;
                u.input_tokens += r.jev.input_tokens;
                u.cost_usd += r.jev.cost_usd;
                u.total_ms += r.jev.total_ms;
                u
            });
            let fell_back: Vec<String> = arm_rows
                .iter()
                .filter(|r| r.jev_failed)
                .map(|r| format!("{}#{}", r.case, r.repeat))
                .collect();
            let errors: Vec<Value> = arm_rows
                .iter()
                .filter_map(|r| {
                    r.error
                        .as_ref()
                        .map(|e| json!({"case": r.case, "repeat": r.repeat, "error": e}))
                })
                .collect();
            let no_call = arm_rows
                .iter()
                .filter(|r| arm == Arm::Jev && r.error.is_none() && r.jev.calls == 0)
                .count();
            per_arm.insert(
                arm.as_str().to_owned(),
                json!({
                    "rows": arm_rows.len(),
                    "metrics": metric_values,
                    "headline": {"metric": head, "ci95": ci},
                    "latency_ms": {
                        "p50": percentile_f64(&latencies, 50, 100),
                        "p95": percentile_f64(&latencies, 95, 100),
                        "max": latencies.last(),
                    },
                    "jev": jev,
                    "jev_fell_back_cases": fell_back,
                    "jev_no_call_cases": no_call,
                    "errors": errors,
                }),
            );
            scored.insert(arm, ok);
        }
        let delta_ci = match (scored.get(&Arm::Off), scored.get(&Arm::Jev)) {
            (Some(off), Some(jev)) => {
                let head = headline(site);
                bootstrap(&case_ids, |sample| {
                    let j = lookup(&metrics(site, &rows_for(jev, sample)), head)?;
                    let o = lookup(&metrics(site, &rows_for(off, sample)), head)?;
                    Some(j - o)
                })
            }
            _ => Value::Null,
        };
        sites.insert(
            site.to_owned(),
            json!({
                "cases": case_ids.len(),
                "headline": headline(site),
                "arms": per_arm,
                "headline_delta_ci95": delta_ci,
            }),
        );
    }
    Value::Object(sites)
}

// ---------------------------------------------------------------------------
// Markdown

fn num(v: &Value) -> Option<f64> {
    v.as_f64()
}

fn fmt_value(v: Option<f64>) -> String {
    v.map_or_else(|| "n/a".to_owned(), |x| format!("{x:.3}"))
}

fn cell(summary: &Value, repeats: u64) -> String {
    match num(&summary["mean"]) {
        None => "n/a".to_owned(),
        Some(mean) if repeats > 1 => format!(
            "{mean:.3} ({}–{})",
            fmt_value(num(&summary["min"])),
            fmt_value(num(&summary["max"]))
        ),
        Some(mean) => format!("{mean:.3}"),
    }
}

fn ci_text(ci: &Value) -> String {
    match (num(&ci["low"]), num(&ci["high"])) {
        (Some(l), Some(h)) => format!("[{l:.3}, {h:.3}]"),
        _ => "[n/a]".to_owned(),
    }
}

fn render_markdown(meta: &Value, aggregates: &Value) -> String {
    let repeats = meta["repeat"].as_u64().unwrap_or(1);
    let mut md = String::new();
    let _ = writeln!(md, "# Jev micro-benchmark results\n");
    let _ = writeln!(md, "- {PREMISE}");
    let _ = writeln!(
        md,
        "- provider: {}, arms: {}, repeats: {repeats}, finished {}",
        meta["provider"].as_str().unwrap_or("none"),
        meta["arms"],
        meta["finished_at"].as_str().unwrap_or_default()
    );
    let _ = writeln!(
        md,
        "- bytes are the action's structured response; tokens_est = bytes / 4 (a proxy, not a tokenizer count)"
    );
    let _ = writeln!(
        md,
        "- latency is the action call alone, client side, one machine; the notice_fanout inbox polling ({} ms window) is not included",
        meta["fanout_window_ms"]
    );
    let _ = writeln!(
        md,
        "- CI: 95% percentile bootstrap over cases, {RESAMPLES} resamples, fixed seed; delta CI is paired (same resampled cases for both arms)"
    );
    if repeats > 1 {
        let _ = writeln!(md, "- cells are mean (min–max) across repeats");
    }
    if let Some(invalid) = meta["invalid_cases"].as_array().filter(|a| !a.is_empty()) {
        let _ = writeln!(
            md,
            "- {} invalid cases were skipped (see results.json)",
            invalid.len()
        );
    }
    let Some(sites) = aggregates.as_object() else {
        return md;
    };
    let mut total_cost = 0.0;
    let mut total_failures = 0;
    for (site, agg) in sites {
        let off = &agg["arms"]["off"];
        let jev = &agg["arms"]["jev"];
        let _ = writeln!(md, "\n## {site} (n = {} cases)\n", agg["cases"]);
        let _ = writeln!(md, "| metric | off | jev | delta |\n|---|---|---|---|");
        let names: Vec<String> = [off, jev]
            .iter()
            .find_map(|a| a["metrics"].as_object())
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();
        // Keep the metric order the aggregates were built in.
        let ordered: Vec<String> = metrics(site, &[])
            .iter()
            .map(|(n, _)| (*n).to_owned())
            .filter(|n| names.contains(n))
            .collect();
        for name in ordered {
            let o = &off["metrics"][&name];
            let j = &jev["metrics"][&name];
            let delta = match (num(&o["mean"]), num(&j["mean"])) {
                (Some(o), Some(j)) => format!("{:+.3}", j - o),
                _ => "n/a".to_owned(),
            };
            let marker = if name == agg["headline"].as_str().unwrap_or_default() {
                " (headline)"
            } else {
                ""
            };
            let _ = writeln!(
                md,
                "| {name}{marker} | {} | {} | {delta} |",
                cell(o, repeats),
                cell(j, repeats)
            );
        }
        for (label, key) in [("latency p50 ms", "p50"), ("latency p95 ms", "p95")] {
            let o = num(&off["latency_ms"][key]);
            let j = num(&jev["latency_ms"][key]);
            let delta = match (o, j) {
                (Some(o), Some(j)) => format!("{:+.1}", j - o),
                _ => "n/a".to_owned(),
            };
            let _ = writeln!(
                md,
                "| {label} | {} | {} | {delta} |",
                o.map_or_else(|| "n/a".to_owned(), |v| format!("{v:.1}")),
                j.map_or_else(|| "n/a".to_owned(), |v| format!("{v:.1}"))
            );
        }
        let _ = writeln!(
            md,
            "\nHeadline `{}`: off CI {}, jev CI {}, delta (jev - off) CI {}.",
            agg["headline"].as_str().unwrap_or_default(),
            ci_text(&off["headline"]["ci95"]),
            ci_text(&jev["headline"]["ci95"]),
            ci_text(&agg["headline_delta_ci95"])
        );
        if jev.is_object() {
            let usage = &jev["jev"];
            let cost = num(&usage["cost_usd"]).unwrap_or(0.0);
            let failures = usage["failures"].as_u64().unwrap_or(0);
            total_cost += cost;
            total_failures += failures;
            let _ = writeln!(
                md,
                "Jev: {} calls, {} questions, {} input tokens, ${cost:.6}, {failures} failures; no Jev call in {} cases.",
                usage["calls"], usage["questions"], usage["input_tokens"], jev["jev_no_call_cases"]
            );
            if let Some(cases) = jev["jev_fell_back_cases"]
                .as_array()
                .filter(|a| !a.is_empty())
            {
                let _ = writeln!(
                    md,
                    "**Fell back to baseline (Jev failure):** {}",
                    cases
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
        }
        for (arm, a) in [("off", off), ("jev", jev)] {
            if let Some(errors) = a["errors"].as_array().filter(|e| !e.is_empty()) {
                let _ = writeln!(md, "**{arm} errors:** {} (see results.json)", errors.len());
            }
        }
    }
    let _ = writeln!(
        md,
        "\n## Totals\n\nJev cost across sites: ${total_cost:.6}; Jev failures: {total_failures}."
    );
    md
}

fn short(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_owned()
    } else {
        format!("{}…", text.chars().take(max).collect::<String>())
    }
}
