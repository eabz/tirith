"""Flat per-run numbers for the coordination benchmark: score.json "bench".

score.py computes the detailed sections; block() copies the numbers the
arms are compared on into one flat object, adds the claim, pull and test-run
counts that no other section has, and joins the worker usage the runner
records. lib/aggregate.py reads only this block (and usage.tsv).

Fields (null when the source is missing):
  done_time_s          first coordination call to the last task marked done
                       (the benchmark's wall time: workers that pull with
                       wait_secs spend up to 120 s on an empty board before
                       they learn there is nothing left)
  wall_time_s          first to last coordination call
  tasks_done/_total, hidden_passed/_total, tasks_accepted,
  integration_passed/_total, visible_tests_ok
  lost_writes          hidden tests that passed at a done-time snapshot and
                       fail on the final code (anchors.lost_writes)
  blocked_s            claim-blocked agent seconds (ADR-0029's gate metric:
                       refusal episodes plus server-side claim waits)
  pull_idle_s          agent seconds without a task: task_pull calls that
                       waited server-side (>= 0.25 s) plus, after a pull that
                       returned none, the time until that agent's next pull
  waiting_s            blocked_s + pull_idle_s
  claims_granted/refused  claim calls answered ok / conflict
  claims_waited, claim_wait_s  claim calls with wait_secs that took >= 0.25 s
  pulls, pulls_none, pulls_held (ok with waiting_on), pulls_waited
  turns, coordination_only_turns, cost_units   lib/turns.py over transcripts
  test_runs, failing_test_runs   Bash tool uses running unittest, failing
                       when the result is an error or prints "FAILED ("
  unclaimed_writes     lib/violations.py (writes without the writer's claim)
  usage_workers, total_tokens, output_tokens, tool_uses, duration_ms,
  cost_usd             sums over this run's rows in usage.tsv

usage.tsv (tab-separated, header row, filled by the runner from the agent
runtime's completion notices) lives in the run directory or its parent (one
file for a whole lab). Columns: run, worker, total_tokens, tool_uses,
duration_ms, and optionally output_tokens, cost_usd. `run` is the run
directory's name (or its full path). A sum is null unless every row of the
run has that column.
"""

import csv
import json
import re
from collections import defaultdict
from datetime import timedelta
from pathlib import Path

import anchors as anchor_metrics
import turns as turn_metrics

WAIT_FLOOR_MS = 250.0
USAGE_COLUMNS = ("total_tokens", "tool_uses", "duration_ms", "output_tokens", "cost_usd")
FAILED_RE = re.compile(r"FAILED \((?:failures|errors)=")


def call_stats(lines, full):
    """Claim and pull counts, and pull idle seconds, from the coordination logs."""
    ms_by_call = {(l["ts"], l["agent"], l["tool"]): l["ms"] for l in lines}
    stats = {"done_time_s": None, "claims_granted": 0, "claims_refused": 0, "claims_waited": 0, "claim_wait_s": 0.0,
             "pulls": 0, "pulls_none": 0, "pulls_held": 0, "pulls_waited": 0}
    idle = defaultdict(list)
    after_none = {}
    first = last_done = None
    for entry in sorted((e for e in full if isinstance(e.get("response"), dict)), key=lambda e: e["ts"]):
        tool, agent, response = entry.get("tool"), entry.get("agent"), entry["response"]
        request = entry.get("request") if isinstance(entry.get("request"), dict) else {}
        ms = ms_by_call.get((entry["ts"], agent, tool), 0.0) or 0.0
        end = anchor_metrics.parse_time(entry["ts"])
        start = end - timedelta(milliseconds=ms)
        first = end if first is None else min(first, end)
        status = response.get("status")
        if tool == "task_update" and status == "ok" and request.get("status") == "done":
            last_done = end
        if tool == "claim":
            stats["claims_granted"] += status == "ok"
            stats["claims_refused"] += status == "conflict"
            if request.get("wait_secs") and ms >= WAIT_FLOOR_MS:
                stats["claims_waited"] += 1
                stats["claim_wait_s"] += ms / 1000.0
        elif tool == "task_pull":
            stats["pulls"] += 1
            if agent in after_none:
                idle[agent].append((after_none.pop(agent), start))
            if ms >= WAIT_FLOOR_MS:
                stats["pulls_waited"] += 1
                idle[agent].append((start, end))
            if status == "none":
                stats["pulls_none"] += 1
                after_none[agent] = end
            elif status == "ok" and response.get("waiting_on"):
                stats["pulls_held"] += 1
    if first is not None and last_done is not None:
        stats["done_time_s"] = round((last_done - first).total_seconds(), 1)
    stats["claim_wait_s"] = round(stats["claim_wait_s"], 3)
    stats["pull_idle_s"] = round(sum(anchor_metrics.union_seconds(spans) for spans in idle.values()), 3)
    return stats


def result_text(content):
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return "\n".join(str(block.get("text", "")) for block in content if isinstance(block, dict))
    return ""


def test_runs(run):
    """{"test_runs", "failing_test_runs"} over the worker transcripts, or None without transcripts."""
    found = turn_metrics.discover(run)
    if not found:
        return None
    runs = failing = 0
    for paths in found.values():
        for path in paths:
            commands, results = {}, {}
            with open(str(path)) as handle:
                for raw in handle:
                    try:
                        row = json.loads(raw)
                    except ValueError:
                        continue
                    message = row.get("message") if isinstance(row.get("message"), dict) else {}
                    content = message.get("content") if isinstance(message.get("content"), list) else []
                    for block in content:
                        if not isinstance(block, dict):
                            continue
                        if block.get("type") == "tool_use" and block.get("name") == "Bash":
                            command = str((block.get("input") or {}).get("command", ""))
                            if "unittest" in command:
                                commands[block.get("id")] = command
                        elif block.get("type") == "tool_result":
                            results[block.get("tool_use_id")] = block
            for use_id in commands:
                runs += 1
                result = results.get(use_id) or {}
                failing += bool(str(result.get("is_error")) == "True" or FAILED_RE.search(
                    result_text(result.get("content"))))
    return {"test_runs": runs, "failing_test_runs": failing}


def usage(run):
    """Sums of this run's usage.tsv rows, or None when there are none."""
    run = Path(run)
    rows = []
    for path in (run / "usage.tsv", run.parent / "usage.tsv"):
        if not path.is_file():
            continue
        with open(str(path), newline="") as handle:
            for row in csv.DictReader(handle, delimiter="\t"):
                if (row.get("run") or "").strip() in (run.name, str(run)):
                    rows.append(row)
        if rows:
            break
    if not rows:
        return None
    out = {"usage_workers": len(rows)}
    for column in USAGE_COLUMNS:
        values = [(row.get(column) or "").strip() for row in rows]
        try:
            out[column] = round(sum(float(v) for v in values), 4) if all(values) else None
        except ValueError:
            out[column] = None
    return out


def block(summary, meta, run, lines, full):
    acceptance = summary.get("acceptance") or {}
    integration = acceptance.get("integration") or {}
    tasks = summary.get("tasks") or {}
    anchors = summary.get("anchors") or {}
    lost = anchors.get("lost_writes")
    turns_total = (summary.get("worker_turns") or {}).get("total")
    violations_total = (summary.get("violations") or {}).get("total")
    out = {key: meta.get(key) for key in ("scenario", "arm", "claims", "hold", "wait", "pull")}
    out.update({
        "wall_time_s": summary.get("wall_time_s"),
        "tasks_done": tasks.get("done"), "tasks_total": tasks.get("total"),
        "hidden_passed": acceptance.get("hidden_passed"), "hidden_total": acceptance.get("hidden_total"),
        "tasks_accepted": acceptance.get("tasks_accepted"),
        "integration_passed": integration.get("passed"), "integration_total": integration.get("total"),
        "visible_tests_ok": (summary.get("visible_tests") or {}).get("ok"),
        "lost_writes": len(lost["regressions"]) if lost else None,
        "blocked_s": anchors.get("blocked_s"),
        "blocked_episodes": anchors.get("blocked_episodes"),
    })
    out.update(call_stats(lines, full))
    out["waiting_s"] = round((out["blocked_s"] or 0.0) + out["pull_idle_s"], 3) if out["blocked_s"] is not None \
        else None
    if turns_total:
        coordination_only = turns_total.get("coordination_only") or {
            "turns": turns_total["mechanical"]["turns"] + turns_total["coordination"]["turns"]}
        out.update({"turns": turns_total["turns"], "coordination_only_turns": coordination_only["turns"],
                    "cost_units": turns_total["cost_units"]})
    else:
        out.update({"turns": None, "coordination_only_turns": None, "cost_units": None})
    out.update(test_runs(run) or {"test_runs": None, "failing_test_runs": None})
    out["unclaimed_writes"] = violations_total.get("unclaimed") if violations_total else None
    out.update(usage(run) or {"usage_workers": 0, **{column: None for column in USAGE_COLUMNS}})
    return out
