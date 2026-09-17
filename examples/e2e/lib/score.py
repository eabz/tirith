#!/usr/bin/env python3
"""Score one benchmark run. Usage: score.py <kit_dir> <run_dir>. Prints JSON."""

import ast
import json
import math
import shutil
import subprocess
import sys
import tempfile
from collections import Counter, defaultdict
from datetime import datetime
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import anchors as anchor_metrics  # noqa: E402
import turns as turn_metrics  # noqa: E402

RUNNER = r"""
import json, sys, unittest
name = sys.argv[1]
out = {"run": 0, "passed": [], "failed": [], "import_error": None}
try:
    suite = unittest.defaultTestLoader.loadTestsFromName(name)
except Exception as err:  # the module does not import (e.g. missing middleware module)
    out["import_error"] = "%s: %s" % (type(err).__name__, err)
    print(json.dumps(out)); sys.exit(0)
class Result(unittest.TestResult):
    def addSuccess(self, test):
        super().addSuccess(test); out["passed"].append(test.id().split(".")[-1])
result = Result()
suite.run(result)
for test, _ in result.failures + result.errors:
    out["failed"].append(test.id().split(".")[-1] if hasattr(test, "id") else str(test))
out["run"] = result.testsRun
print(json.dumps(out))
"""


def cli(run, tool, args):
    url = (run / "url").read_text().strip()
    tirith = (run / "tirith_bin").read_text().strip()
    proc = subprocess.run([tirith, "--url", url, "--json", "-a", "scorer", "call", tool, json.dumps(args)],
                          capture_output=True, text=True, timeout=30)
    try:
        return json.loads(proc.stdout)
    except json.JSONDecodeError:
        return {"status": "error", "message": (proc.stderr or proc.stdout).strip()[:300]}


def expected_tests(path):
    tree = ast.parse(path.read_text())
    return sorted(node.name for cls in tree.body if isinstance(cls, ast.ClassDef)
                  for node in cls.body if isinstance(node, ast.FunctionDef) and node.name.startswith("test"))


def percentile(values, pct):
    if not values:
        return None
    ordered = sorted(values)
    return ordered[max(0, math.ceil(pct / 100.0 * len(ordered)) - 1)]


def latency(values):
    if not values:
        return {"mean": None, "p50": None, "p95": None, "max": None}
    return {"mean": round(sum(values) / len(values), 1), "p50": percentile(values, 50),
            "p95": percentile(values, 95), "max": max(values)}


def hook_metrics(run):
    """Counts from hooks_state/events.jsonl (runs set up with --protocol hooks), else None."""
    path = run / "hooks_state" / "events.jsonl"
    if not path.is_file():
        return None
    actions, per_agent, claim_ms, gate_ms = Counter(), defaultdict(Counter), [], []
    for raw in path.read_text().splitlines():
        try:
            row = json.loads(raw)
        except ValueError:
            continue
        actions[row.get("action")] += 1
        per_agent[str(row.get("agent"))][row.get("action")] += 1
        if row.get("action") in ("claim", "deny") and row.get("ms") is not None:
            claim_ms.append(row["ms"])
        if row.get("gate") and row.get("ms") is not None:
            gate_ms.append(row["ms"])
    errors = run / "hooks_state" / "errors.log"
    return {"actions": dict(sorted(actions.items())),
            "per_agent": {agent: dict(sorted(c.items())) for agent, c in sorted(per_agent.items())},
            "claim_ms": latency(claim_ms), "gate_ms": latency(gate_ms),
            "errors": len(errors.read_text().splitlines()) if errors.is_file() else 0}


def parse_ts(ts):
    return datetime.strptime(ts, "%Y-%m-%dT%H:%M:%S.%fZ")


def copy_repo(repo, dest, with_git):
    ignore = [".env", "__pycache__", "*.pyc"] + ([] if with_git else [".git", ".tirith"])
    shutil.copytree(repo, dest, ignore=shutil.ignore_patterns(*ignore))


def unittest_summary(output):
    ran, ok, failures, errors = None, False, 0, 0
    for line in output.splitlines():
        if line.startswith("Ran "):
            ran = int(line.split()[1])
        if line.strip() == "OK" or line.startswith("OK ("):
            ok = True
        if line.startswith("FAILED ("):
            for part in line[len("FAILED ("):-1].split(","):
                key, _, value = part.strip().partition("=")
                if key == "failures":
                    failures = int(value)
                if key == "errors":
                    errors = int(value)
    return {"ran": ran, "ok": ok, "failures": failures, "errors": errors}


def git_numstat(repo_copy, base, pathspec):
    out = subprocess.run(["git", "-C", str(repo_copy), "diff", "--cached", "--numstat", base, "--"] + pathspec,
                         capture_output=True, text=True).stdout
    files, ins, dels = 0, 0, 0
    for line in out.splitlines():
        a, d, _ = line.split("\t", 2)
        files += 1
        ins += int(a) if a.isdigit() else 0
        dels += int(d) if d.isdigit() else 0
    return {"files": files, "insertions": ins, "deletions": dels}


def run_hidden(code, scen, names, dest, env):
    """Run each hidden test module against a copy of ``code``; name -> result."""
    copy_repo(code, dest, with_git=False)
    (dest / "hidden_e2e").mkdir()
    (dest / "hidden_e2e" / "__init__.py").write_text("")
    results = {}
    for name in names:
        shutil.copy(scen / "hidden_tests" / name, dest / "hidden_e2e" / name)
        try:
            proc = subprocess.run(["python3", "-c", RUNNER, "hidden_e2e." + name[:-3]], cwd=dest,
                                  capture_output=True, text=True, timeout=120, env=env)
            results[name] = json.loads(proc.stdout.strip().splitlines()[-1])
        except (subprocess.TimeoutExpired, json.JSONDecodeError, IndexError):
            results[name] = {"passed": [], "import_error": "runner failed or timed out"}
    return results


def main():
    kit, run = Path(sys.argv[1]), Path(sys.argv[2])
    repo = run / "repo"
    meta = json.loads((run / "meta.json").read_text())
    # Forge lives at the kit root; other scenarios record their directory.
    scen = Path(meta.get("scenario_dir") or kit)
    tasks_doc = json.loads((scen / "tasks.json").read_text())
    tasks_spec = tasks_doc["tasks"]
    task_ids = json.loads((run / "task_ids.json").read_text())
    summary = {"run": str(run), "tirith_version": meta.get("tirith_version"),
               "protocol": meta.get("protocol", "manual")}
    if meta.get("scenario"):
        summary.update({"scenario": meta["scenario"], "claims": meta.get("claims"), "wait": meta.get("wait")})

    # Task board, from the live daemon.
    listing = cli(run, "task_list", {"limit": 200})
    rows = listing.get("tasks", [])
    by_key = {}
    for key, full_id in task_ids.items():
        row = next((r for r in rows if full_id.startswith(str(r.get("id", "")))), None)
        by_key[key] = {k: row.get(k) for k in ("status", "owner", "by") if row and row.get(k) is not None} if row \
            else {"status": "unknown"}
    summary["tasks"] = {"done": sum(1 for v in by_key.values() if v.get("status") == "done"),
                        "total": len(task_ids), "daemon_answer": listing.get("status"), "by_key": by_key}

    with tempfile.TemporaryDirectory(prefix="e2e_score_") as tmp:
        tmp = Path(tmp)
        # Visible tests, exactly as workers run them.
        visible = tmp / "visible"
        copy_repo(repo, visible, with_git=False)
        env = {"PATH": "/usr/bin:/bin", "PYTHONDONTWRITEBYTECODE": "1"}
        try:
            proc = subprocess.run(["python3", "-m", "unittest"], cwd=visible, capture_output=True, text=True,
                                  timeout=120, env=env)
            summary["visible_tests"] = unittest_summary(proc.stderr)
        except subprocess.TimeoutExpired:
            summary["visible_tests"] = {"ran": None, "ok": False, "timeout": True}

        # Hidden acceptance tests, one module at a time, in a separate copy.
        files = {t["key"]: t["hidden_test"] for t in tasks_spec}
        if (scen / "hidden_tests" / "test_hidden_integration.py").is_file():
            files["integration"] = "test_hidden_integration.py"
        results = run_hidden(repo, scen, files.values(), tmp / "hidden", env)
        per_task, integration = {}, None
        passed_total = expected_total = 0
        accepted = 0
        for key, name in files.items():
            source = scen / "hidden_tests" / name
            expected = expected_tests(source)
            result = results[name]
            passed = sorted(set(result["passed"]) & set(expected))
            row = {"passed": len(passed), "total": len(expected)}
            if result.get("import_error"):
                row["import_error"] = result["import_error"][:200]
            if len(passed) < len(expected):
                row["failed"] = sorted(set(expected) - set(passed))
            if key == "integration":
                integration = row
                continue
            per_task[key] = row
            passed_total += len(passed)
            expected_total += len(expected)
            accepted += len(passed) == len(expected)
        summary["acceptance"] = {
            "hidden_passed": passed_total, "hidden_total": expected_total, "tasks_accepted": accepted,
            "per_task": per_task, "integration": integration,
            "note": tasks_doc.get("acceptance_note", "2 ctx-interface regression tests pass on the untouched baseline"),
        }

        # Sub-file scenarios: which passing tests were lost after a done-time snapshot.
        lost = None
        if any("anchors" in t for t in tasks_spec):
            final = {name: set(r["passed"]) for name, r in results.items()}
            counter = [0]

            def snapshot_results(code_dir):
                counter[0] += 1
                out = run_hidden(code_dir, scen, files.values(), tmp / ("snap%d" % counter[0]), env)
                return {name: set(r["passed"]) for name, r in out.items()}

            lost = anchor_metrics.lost_writes(tasks_spec, task_ids, run, final, snapshot_results)

        # Code changes against the seeded baseline commit.
        gitcopy = tmp / "git"
        copy_repo(repo, gitcopy, with_git=True)
        subprocess.run(["git", "-C", str(gitcopy), "add", "-A"], capture_output=True)
        summary["git"] = {
            "code": git_numstat(gitcopy, meta["base_commit"], [".", ":(exclude).tirith"]),
            "tirith_state": git_numstat(gitcopy, meta["base_commit"], [".tirith"]),
        }

    # Duplicate work on the near-duplicate pair (Forge only).
    if "request-id" in task_ids and "correlation-id" in task_ids:
        impl = sorted(str(p.relative_to(repo)) for p in (repo / "app").rglob("*.py")
                      if p.name not in ("config.py", "router.py") and "uuid4" in p.read_text(errors="ignore"))
        rid, cid = by_key.get("request-id", {}), by_key.get("correlation-id", {})
        summary["duplicate_work"] = {
            "request_id_done": rid.get("status") == "done", "correlation_id_done": cid.get("status") == "done",
            "both_done": rid.get("status") == "done" and cid.get("status") == "done",
            "id_generator_files": impl, "duplicate_implementation": len(impl) > 1,
        }

    # Coordination calls.
    bad_lines = 0

    def jsonl(path):
        nonlocal bad_lines
        rows = []
        for raw in path.read_text().splitlines():
            if not raw.strip():
                continue
            try:
                rows.append(json.loads(raw))
            except json.JSONDecodeError:
                bad_lines += 1
        return rows

    lines = jsonl(run / "coord.jsonl")
    full = jsonl(run / "coord_full.jsonl")
    per_tool = defaultdict(lambda: {"calls": 0, "response_bytes": 0, "request_bytes": 0, "ms": []})
    per_agent = defaultdict(lambda: {"calls": 0, "response_bytes": 0})
    statuses = Counter()
    origins = Counter()
    for line in lines:
        tool = per_tool[line["tool"]]
        tool["calls"] += 1
        tool["response_bytes"] += line["response_bytes"]
        tool["request_bytes"] += line["request_bytes"]
        tool["ms"].append(line["ms"])
        per_agent[line["agent"]]["calls"] += 1
        per_agent[line["agent"]]["response_bytes"] += line["response_bytes"]
        statuses[line["status"]] += 1
        origins[line.get("origin", "model")] += 1
    lost_events = lost_paths = pushes = inbox_msgs = 0
    for entry in full:
        response = entry.get("response")
        if not isinstance(response, dict):
            continue
        if response.get("lost"):
            lost_events += 1
            lost_paths += len(response["lost"])
        for message in response.get("inbox", []) or []:
            inbox_msgs += 1
            pushes += message.get("from") == "tirith"

    def ok_count(tool):
        return sum(1 for l in lines if l["tool"] == tool and l["status"] == "ok")

    all_ms = [l["ms"] for l in lines]
    response_bytes = sum(l["response_bytes"] for l in lines)
    wall = None
    if lines:
        stamps = [parse_ts(l["ts"]) for l in lines]
        wall = round((max(stamps) - min(stamps)).total_seconds(), 1)
    summary["coordination"] = {
        "calls": len(lines), "statuses": dict(statuses), "per_origin": dict(sorted(origins.items())),
        "response_bytes": response_bytes, "est_response_tokens": response_bytes // 4,
        "request_bytes": sum(l["request_bytes"] for l in lines),
        "latency_ms": latency(all_ms),
        "per_tool": {k: {"calls": v["calls"], "response_bytes": v["response_bytes"],
                         "request_bytes": v["request_bytes"], "latency_ms": latency(v["ms"])}
                     for k, v in sorted(per_tool.items())},
        "per_agent": dict(sorted(per_agent.items())),
        "claim_conflicts": sum(1 for l in lines if l["tool"] == "claim" and l["status"] == "conflict"),
        "task_update_conflicts": sum(1 for l in lines if l["tool"] == "task_update" and l["status"] == "conflict"),
        "lost_lease_events": lost_events, "lost_lease_paths": lost_paths,
        "notices_published": ok_count("notice_publish"), "contracts_published": ok_count("contract_publish"),
        "messages_sent": ok_count("message_send"), "memory_writes": ok_count("memory_write"),
        "decisions_recorded": ok_count("decision_record"), "tasks_created_by_workers": ok_count("task_create"),
        "inbox_messages_delivered": inbox_msgs, "tirith_push_messages": pushes,
        "unparseable_log_lines": bad_lines,
    }
    summary["wall_time_s"] = wall

    if any("anchors" in t for t in tasks_spec):
        anchors = anchor_metrics.claim_metrics(tasks_spec, lines, full) or {"per_anchor": {}}
        if lost is not None:
            per_anchor = anchors["per_anchor"]
            for row in lost["regressions"]:
                for anchor in row["anchors"]:
                    if anchor in per_anchor:
                        per_anchor[anchor]["lost_writes"] += 1
        anchors["lost_writes"] = lost
        summary["anchors"] = anchors

    # Daemon health at scoring time.
    status_now = cli(run, "status", {})
    summary["daemon"] = {k: status_now.get(k) for k in ("status", "tasks_orphaned", "persist_error", "load_errors",
                                                        "claims")}
    # Model turns per worker from Claude Code transcripts (<run>/transcripts/<agent>*.jsonl, or the
    # transcript paths hooks recorded); mechanical = only tb claim/release/task_pull/task_update.
    summary["worker_turns"] = turn_metrics.per_worker(run)
    summary["hooks"] = hook_metrics(run)
    summary["worker_llm"] = {"tokens_in": None, "tokens_out": None, "cost_usd": None, "wall_time_s": None,
                             "note": "filled in by the runner from the agent runtime; not visible to the kit"}
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
