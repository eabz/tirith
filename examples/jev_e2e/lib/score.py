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

JEV_FIELDS = ("advice", "skipped", "picked_by", "possible_duplicate", "similar_note", "ranked_by",
              "skipped_recipients")

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


def numeric_delta(now, before):
    if isinstance(now, dict):
        before = before if isinstance(before, dict) else {}
        return {k: numeric_delta(v, before.get(k)) for k, v in now.items()
                if isinstance(v, (int, float, dict)) and not isinstance(v, bool)}
    if isinstance(now, (int, float)):
        base = before if isinstance(before, (int, float)) else 0
        return round(now - base, 6) if isinstance(now, float) else now - base
    return now


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


def main():
    kit, run = Path(sys.argv[1]), Path(sys.argv[2])
    repo = run / "repo"
    meta = json.loads((run / "meta.json").read_text())
    tasks_spec = json.loads((kit / "tasks.json").read_text())["tasks"]
    task_ids = json.loads((run / "task_ids.json").read_text())
    summary = {"run": str(run), "arm": meta["arm"], "tirith_version": meta.get("tirith_version")}

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

    with tempfile.TemporaryDirectory(prefix="jev_e2e_score_") as tmp:
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
        hidden = tmp / "hidden"
        copy_repo(repo, hidden, with_git=False)
        (hidden / "hidden_e2e").mkdir()
        (hidden / "hidden_e2e" / "__init__.py").write_text("")
        per_task, integration = {}, None
        passed_total = expected_total = 0
        accepted = 0
        files = {t["key"]: t["hidden_test"] for t in tasks_spec}
        files["integration"] = "test_hidden_integration.py"
        for key, name in files.items():
            source = kit / "hidden_tests" / name
            shutil.copy(source, hidden / "hidden_e2e" / name)
            expected = expected_tests(source)
            try:
                proc = subprocess.run(["python3", "-c", RUNNER, "hidden_e2e." + name[:-3]], cwd=hidden,
                                      capture_output=True, text=True, timeout=120, env=env)
                result = json.loads(proc.stdout.strip().splitlines()[-1])
            except (subprocess.TimeoutExpired, json.JSONDecodeError, IndexError):
                result = {"passed": [], "import_error": "runner failed or timed out"}
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
            "note": "2 ctx-interface regression tests pass on the untouched baseline",
        }

        # Code changes against the seeded baseline commit.
        gitcopy = tmp / "git"
        copy_repo(repo, gitcopy, with_git=True)
        subprocess.run(["git", "-C", str(gitcopy), "add", "-A"], capture_output=True)
        summary["git"] = {
            "code": git_numstat(gitcopy, meta["base_commit"], [".", ":(exclude).tirith"]),
            "tirith_state": git_numstat(gitcopy, meta["base_commit"], [".tirith"]),
        }

    # Duplicate work on the near-duplicate pair.
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
    for line in lines:
        tool = per_tool[line["tool"]]
        tool["calls"] += 1
        tool["response_bytes"] += line["response_bytes"]
        tool["request_bytes"] += line["request_bytes"]
        tool["ms"].append(line["ms"])
        per_agent[line["agent"]]["calls"] += 1
        per_agent[line["agent"]]["response_bytes"] += line["response_bytes"]
        statuses[line["status"]] += 1
    jev_seen = Counter()
    lost_events = lost_paths = pushes = inbox_msgs = 0
    for entry in full:
        response = entry.get("response")
        if not isinstance(response, dict):
            continue
        for field in JEV_FIELDS:
            if field in response:
                jev_seen["%s.%s" % (entry["tool"], field)] += 1
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
        "calls": len(lines), "statuses": dict(statuses),
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
        "jev_fields_seen": dict(sorted(jev_seen.items())),
        "unparseable_log_lines": bad_lines,
    }
    summary["wall_time_s"] = wall

    # Jev counters from the daemon, minus what setup seeding spent.
    status_now = cli(run, "status", {})
    setup_status = json.loads((run / "setup" / "status.json").read_text())
    jev_now, jev_setup = status_now.get("jev"), setup_status.get("jev")
    during = numeric_delta(jev_now, jev_setup) if jev_now else None
    if during:
        # avg_ms does not subtract; recompute it from the per-site totals.
        sites = during.get("sites", {})
        total_ms = sum(site.get("total_ms", 0) for site in sites.values())
        during["avg_ms"] = round(total_ms / during["calls"], 1) if during.get("calls") else 0
        during["sites"] = {name: site for name, site in sites.items() if site.get("calls")}
    summary["jev"] = {"enabled": jev_now is not None, "during_run": during,
                      "setup": jev_setup, "daemon_status": status_now.get("status")}
    summary["daemon"] = {k: status_now.get(k) for k in ("tasks_orphaned", "persist_error", "load_errors", "claims")}
    summary["worker_llm"] = {"tokens_in": None, "tokens_out": None, "cost_usd": None, "wall_time_s": None,
                             "note": "filled in by jev-lead from the agent runtime; not visible to the kit"}
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
