#!/usr/bin/env python3
"""Protocol violations: repository writes a worker made without a live claim of its own.

Usage: violations.py <run_dir>   (prints JSON; score.py embeds the same under "violations")

Writes come from the worker transcripts that turns.discover finds
(<run>/transcripts/<agent>*.jsonl, or the transcript paths the hooks
recorded):
  tool edits   Edit, Write, MultiEdit, NotebookEdit and Serena edit tools, timed
               at their tool_result; results marked is_error (including edits
               a hook denied) are skipped
  bash writes  heuristic targets in Bash commands: `>`/`>>` redirects and `tee`
               into *.py or existing repo files, `sed -i` on a file, Python
               open(..., "w"/"a") and Path(...).write_text; timed at the result
Claims come from coord_full.jsonl: a claim `ok` opens an interval per path
for that agent, closed by its release (listed in `released`, or every path
when the release named none) or by a `lost` report (at the lease's `at`).
A write is covered when one of the writer's open claims is the file, a
directory above it, or an anchor in it (`file#Symbol`, counted separately as
anchor_only). Lost writes (a done-time change later overwritten) are
score.py's anchors.lost_writes for scenarios with anchors.
"""

import json
import os
import re
import sys
from datetime import datetime
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import turns  # noqa: E402

EDIT_FIELDS = {"Edit": "file_path", "Write": "file_path", "MultiEdit": "file_path", "NotebookEdit": "notebook_path",
               "mcp__serena__replace_symbol_body": "relative_path", "mcp__serena__insert_after_symbol": "relative_path",
               "mcp__serena__insert_before_symbol": "relative_path", "mcp__serena__replace_content": "relative_path",
               "mcp__serena__rename_symbol": "relative_path", "mcp__serena__safe_delete_symbol": "relative_path"}
REDIRECT_RE = re.compile(r"(?<![0-9&<>])>{1,2}\s*([^\s;&|<>()'\"]+)")
TEE_RE = re.compile(r"\btee\s+(?:-a\s+)?([^\s;&|<>()]+)")
SED_RE = re.compile(r"\bsed\s+-i(?:\s*''|\s+\"\")?(?:\s+-e)?\s+(?:'[^']*'|\"[^\"]*\"|\S+)\s+([^\s;&|<>()]+)")
PY_RE = re.compile(r"""(?:open\(\s*['"]([^'"]+)['"]\s*,\s*['"][wa]|Path\(\s*['"]([^'"]+)['"]\s*\)\.write_text)""")
HEREDOC_RE = re.compile(r"<<-?\s*['\"]?(\w+)['\"]?[^\n]*\n.*?^\s*\1\s*$", re.S | re.M)


def parse_ts(text):
    try:
        return datetime.strptime(str(text)[:23] + "Z", "%Y-%m-%dT%H:%M:%S.%fZ")
    except ValueError:
        try:
            return datetime.strptime(str(text)[:19], "%Y-%m-%dT%H:%M:%S")
        except ValueError:
            return None


def repo_relative(repo, raw):
    if not raw or raw.startswith("$") or raw.startswith("-") or raw == "/dev/null":
        return None
    path = raw if os.path.isabs(raw) else os.path.join(repo, raw)
    real = os.path.realpath(path)
    if not real.startswith(repo + os.sep):
        return None
    rel = os.path.relpath(real, repo)
    if rel.startswith((".git" + os.sep, ".tirith" + os.sep)) or "__pycache__" in rel:
        return None
    return rel


def bash_targets(command, repo):
    targets = set()
    without_bodies = HEREDOC_RE.sub(lambda m: m.group(0).split("\n", 1)[0], command)
    for pattern, text in ((REDIRECT_RE, without_bodies), (TEE_RE, without_bodies), (SED_RE, without_bodies)):
        for raw in pattern.findall(text):
            rel = repo_relative(repo, raw)
            if rel and (rel.endswith(".py") or os.path.isfile(os.path.join(repo, rel))):
                targets.add(rel)
    for groups in PY_RE.findall(command):
        rel = repo_relative(repo, groups[0] or groups[1])
        if rel:
            targets.add(rel)
    return sorted(targets)


def writes(transcript, repo):
    """[(time, kind, repo path)] in one transcript."""
    uses, results = {}, {}
    with open(str(transcript)) as handle:
        for raw in handle:
            try:
                row = json.loads(raw)
            except ValueError:
                continue
            message = row.get("message") if isinstance(row.get("message"), dict) else {}
            for block in message.get("content") or [] if isinstance(message.get("content"), list) else []:
                if not isinstance(block, dict):
                    continue
                if block.get("type") == "tool_use":
                    uses[block.get("id")] = (row.get("timestamp"), block.get("name"), block.get("input") or {})
                elif block.get("type") == "tool_result":
                    results[block.get("tool_use_id")] = (row.get("timestamp"), str(block.get("is_error")) == "True")
    found = []
    for use_id, (ts, name, tool_input) in uses.items():
        result_ts, is_error = results.get(use_id, (ts, False))
        when = parse_ts(result_ts or ts)
        if when is None:
            continue
        if name in EDIT_FIELDS:
            if is_error:
                continue
            rel = repo_relative(repo, str(tool_input.get(EDIT_FIELDS[name]) or ""))
            if rel:
                found.append((when, "tool_edit", rel))
        elif name == "Bash":
            for rel in bash_targets(str(tool_input.get("command", "")), repo):
                found.append((when, "bash_write", rel))
    return sorted(found)


def claim_intervals(coord_full):
    """agent -> [(start, end or None, claimed path)]."""
    open_, done = {}, {}
    rows = []
    for raw in coord_full.read_text().splitlines():
        try:
            rows.append(json.loads(raw))
        except ValueError:
            continue
    rows.sort(key=lambda r: r.get("ts", ""))
    for row in rows:
        agent, tool, when = row.get("agent"), row.get("tool"), parse_ts(row.get("ts"))
        request = row.get("request") if isinstance(row.get("request"), dict) else {}
        response = row.get("response") if isinstance(row.get("response"), dict) else {}
        held = open_.setdefault(agent, {})
        for lost in response.get("lost") or []:
            path = lost.get("path") if isinstance(lost, dict) else lost
            path = str(path).replace("::", "#")
            if path in held:
                done.setdefault(agent, []).append((held.pop(path), parse_ts(lost.get("at")) if isinstance(lost, dict)
                                                   and lost.get("at") else when, path))
        if response.get("status") != "ok":
            continue
        if tool == "claim":
            for path in request.get("paths") or []:
                held.setdefault(str(path), when)
        elif tool == "release":
            names = response.get("released") or request.get("paths") or list(held)
            for path in names:
                path = str(path).replace("::", "#")
                if path in held:
                    done.setdefault(agent, []).append((held.pop(path), when, path))
    for agent, held in open_.items():
        for path, start in held.items():
            done.setdefault(agent, []).append((start, None, path))
    return done


def covered(intervals, when, rel):
    """'file', 'anchor', or None."""
    best = None
    for start, end, path in intervals:
        if start is None or start > when or (end is not None and end < when):
            continue
        base, _, anchor = path.partition("#")
        base = base.rstrip("/")
        if rel == base or rel.startswith(base + "/"):
            if not anchor:
                return "file"
            best = "anchor"
    return best


def per_worker(run):
    run = Path(run)
    found = turns.discover(run)
    if not found or not (run / "coord_full.jsonl").is_file():
        return None
    repo = os.path.realpath(str(run / "repo"))
    intervals = claim_intervals(run / "coord_full.jsonl")
    workers, total = {}, {"tool_edits": 0, "bash_writes": 0, "unclaimed": 0, "anchor_only": 0}
    for agent, paths in sorted(found.items()):
        row = {"tool_edits": 0, "bash_writes": 0, "unclaimed": 0, "anchor_only": 0, "unclaimed_paths": {}}
        for transcript in paths:
            for when, kind, rel in writes(transcript, repo):
                row["tool_edits" if kind == "tool_edit" else "bash_writes"] += 1
                cover = covered(intervals.get(agent, []), when, rel)
                if cover is None:
                    row["unclaimed"] += 1
                    row["unclaimed_paths"][rel] = row["unclaimed_paths"].get(rel, 0) + 1
                elif cover == "anchor":
                    row["anchor_only"] += 1
        for key in total:
            total[key] += row[key]
        workers[agent] = row
    return {"per_worker": workers, "total": total,
            "note": "writes from transcripts (tool edits exact, bash writes heuristic) vs the writer's own "
                    "claims in coord_full.jsonl; see lib/violations.py"}


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("usage: violations.py <run_dir>")
    print(json.dumps(per_worker(sys.argv[1]), indent=2))
