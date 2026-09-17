"""Per-anchor claim metrics and lost-write detection for scenarios whose
tasks list `anchors` (``path#Symbol`` strings). Used by score.py.

Claim keys are the strings the daemon granted or refused: a file, a
directory, or an anchor. Anchors follow ADR-0029: split at the first ``#``;
nested separators ``.``, ``/`` and ``::`` are equivalent and segments
compare ASCII case-insensitively. Two keys overlap when one is a directory
above or the file of the other, or both name the same file and their
anchors are equal or nested (``Order`` and ``Order.notes``). Every metric
is computed from ``coord.jsonl`` / ``coord_full.jsonl`` alone, so file and
symbol arms are scored the same way:

- conflicts: refused claim rows whose requested path and held path both
  overlap the anchor.
- hold_s / holds: seconds from a grant to the release (or lost lease, or the
  end of the log) of each claim key that overlaps the anchor, summed over
  holds; covered_s is the union of those intervals (time the anchor was
  unavailable to everyone but its holder).
- waits / wait_s: for each agent refused on the anchor, the time from the
  start of the first refused call to the end of the claim call that finally
  granted a key overlapping it. A claim with ``wait_secs`` that was granted
  after waiting at least WAIT_FLOOR_S counts its own duration as a wait.
  unresolved_waits: refusals never followed by a grant.
- blocked_s (top level and per agent): the union, per agent, of its refusal
  episodes on any key (first refused call to the grant of an overlapping
  key, or to the end of the log) and of server-side waits; ADR-0029's gate
  metric.
- lost_writes: hidden tests that passed in some done-time snapshot and fail
  on the final code. Each is attributed to the anchors of the test's task
  whose source differs between the last snapshot where it passed and the
  final code (``attribution: changed``), or to all the task's anchors when
  none differs (``attribution: task``).
"""

import ast
import json
from collections import defaultdict
from datetime import datetime, timedelta

WAIT_FLOOR_S = 0.25


def parse_time(text):
    text = text.strip()
    if text.endswith("Z"):
        text = text[:-1] + "+00:00"
    if "." in text:
        head, _, tail = text.partition(".")
        frac, sign, offset = tail, "", ""
        for mark in ("+", "-"):
            if mark in tail:
                frac, sign, offset = tail.partition(mark)
                break
        text = "%s.%s%s%s" % (head, (frac + "000000")[:6], sign, offset)
        return datetime.strptime(text, "%Y-%m-%dT%H:%M:%S.%f%z")
    return datetime.strptime(text, "%Y-%m-%dT%H:%M:%S%z")


def seconds(td):
    return round(td.total_seconds(), 3)


def split(key):
    """(path, [symbol segments]) with segments lowercased."""
    path, _, symbol = key.partition("#")
    segments = [seg for seg in symbol.replace("::", "/").replace(".", "/").lower().split("/") if seg.strip()]
    return path.strip().strip("/"), [seg.strip() for seg in segments]


def covers(key, other):
    """Whether ``key`` is ``other`` or contains it (directory, file, or enclosing symbol)."""
    kpath, ksym = split(key)
    opath, osym = split(other)
    if not ksym:
        return kpath in ("", ".") or kpath == opath or opath.startswith(kpath + "/")
    return kpath == opath and osym[:len(ksym)] == ksym


def overlaps(key, other):
    """Whether two claim keys (files, directories or anchors) overlap."""
    return covers(key, other) or covers(other, key)


def key_kind(key):
    path, symbol = split(key)
    if symbol:
        return "symbol"
    return "file" if "." in path.rsplit("/", 1)[-1] else "dir"


def union_seconds(intervals):
    total, current = 0.0, None
    for start, end in sorted(intervals):
        if current is None or start > current[1]:
            if current:
                total += (current[1] - current[0]).total_seconds()
            current = [start, end]
        else:
            current[1] = max(current[1], end)
    if current:
        total += (current[1] - current[0]).total_seconds()
    return round(total, 3)


def claim_metrics(tasks_spec, lines, full):
    """Per-anchor, per-file and per-key claim metrics from the call logs."""
    anchors = {}
    for task in tasks_spec:
        for anchor in task.get("anchors", []):
            anchors.setdefault(anchor, []).append(task["key"])
    ms_by_call = {(l["ts"], l["agent"], l["tool"]): l["ms"] for l in lines}
    entries = sorted((e for e in full if isinstance(e.get("response"), dict)), key=lambda e: e["ts"])
    if not entries:
        return None
    last_ts = parse_time(entries[-1]["ts"])

    holds = []  # (agent, key, start, end, open_at_end)
    open_holds = defaultdict(dict)  # agent -> key -> start
    conflicts = []  # (agent, requested, held, owner, start)
    pending = defaultdict(dict)  # agent -> anchor -> start of first refusal
    waits = []  # (agent, anchor, start, end, kind)
    granted_kinds = defaultdict(int)
    episodes = defaultdict(list)  # agent -> [(start, end)] blocked on any key
    refused = defaultdict(dict)  # agent -> requested key -> start of first refusal

    def close(agent, key, end):
        start = open_holds[agent].pop(key, None)
        if start is not None:
            holds.append((agent, key, start, max(end, start), False))

    for entry in entries:
        agent, tool, response = entry["agent"], entry["tool"], entry["response"]
        end = parse_time(entry["ts"])
        ms = ms_by_call.get((entry["ts"], agent, tool), 0.0) or 0.0
        start = end - timedelta(milliseconds=ms)
        request = entry.get("request") if isinstance(entry.get("request"), dict) else {}
        for lost in response.get("lost", []) or []:
            if isinstance(lost, dict) and lost.get("path"):
                at = end
                try:
                    at = min(parse_time(lost["at"]), end) if lost.get("at") else end
                except ValueError:
                    pass
                close(agent, lost["path"], at)
        status = response.get("status")
        if tool == "claim" and status == "ok":
            new = [p for p in response.get("new_paths", []) or [] if isinstance(p, str)]
            renewed = [p for p in response.get("renewed_paths", []) or [] if isinstance(p, str)]
            for key in new:
                granted_kinds[key_kind(key)] += 1
                open_holds[agent].setdefault(key, end)
            for key in response.get("absorbed_paths", []) or []:
                if isinstance(key, str) and key in open_holds[agent]:
                    close(agent, key, end)
            granted = new + renewed
            waited_on_server = request.get("wait_secs") and ms / 1000.0 >= WAIT_FLOOR_S
            resolved = False
            for key in list(refused[agent]):
                if any(overlaps(key, g) for g in granted):
                    episodes[agent].append((refused[agent].pop(key), end))
                    resolved = True
            if waited_on_server and not resolved:
                episodes[agent].append((start, end))
            for anchor in anchors:
                if not any(overlaps(key, anchor) for key in granted):
                    continue
                first = pending[agent].pop(anchor, None)
                if first is not None:
                    waits.append((agent, anchor, first, end, "retry"))
                elif waited_on_server and any(overlaps(key, anchor) for key in new):
                    waits.append((agent, anchor, start, end, "server"))
        elif tool == "claim" and status == "conflict":
            for row in response.get("conflicts", []) or []:
                if not isinstance(row, dict) or not row.get("path"):
                    continue
                held = row.get("overlaps") or row["path"]
                conflicts.append((agent, row["path"], held, row.get("owner"), start))
                refused[agent].setdefault(row["path"], start)
                for anchor in anchors:
                    if overlaps(row["path"], anchor) and overlaps(held, anchor):
                        pending[agent].setdefault(anchor, start)
        elif tool == "release" and status == "ok":
            released = [p for p in response.get("released", []) or [] if isinstance(p, str)]
            for key in released:
                close(agent, key, end)
            if not request.get("paths"):
                for key in list(open_holds[agent]):
                    close(agent, key, end)

    for agent, keys in open_holds.items():
        for key, start in keys.items():
            holds.append((agent, key, start, last_ts, True))
    for agent, keys in refused.items():
        for start in keys.values():
            episodes[agent].append((start, last_ts))
    blocked = {agent: union_seconds(spans) for agent, spans in sorted(episodes.items())}

    per_anchor = {}
    for anchor, task_keys in sorted(anchors.items()):
        a_holds = [h for h in holds if overlaps(h[1], anchor)]
        a_conflicts = [c for c in conflicts if overlaps(c[1], anchor) and overlaps(c[2], anchor)]
        a_waits = [w for w in waits if w[1] == anchor]
        wait_values = [seconds(w[3] - w[2]) for w in a_waits]
        per_anchor[anchor] = {
            "tasks": task_keys,
            "conflicts": len(a_conflicts),
            "holds": len(a_holds),
            "hold_s": round(sum((h[3] - h[2]).total_seconds() for h in a_holds), 3),
            "covered_s": union_seconds([(h[2], h[3]) for h in a_holds]),
            "held_by_keys": sorted({h[1] for h in a_holds}),
            "waits": len(a_waits),
            "server_waits": sum(1 for w in a_waits if w[4] == "server"),
            "wait_s": round(sum(wait_values), 3),
            "max_wait_s": max(wait_values) if wait_values else None,
            "unresolved_waits": sum(1 for agent_pending in pending.values() if anchor in agent_pending),
            "lost_writes": 0,
        }

    per_file = {}
    for path in sorted({split(a)[0] for a in anchors}):
        f_holds = [h for h in holds if overlaps(h[1], path)]
        f_conflicts = [c for c in conflicts if overlaps(c[1], path) and overlaps(c[2], path)]
        f_waits = {(w[0], w[2], w[3]) for w in waits if split(w[1])[0] == path}
        per_file[path] = {
            "conflicts": len(f_conflicts),
            "holds": len(f_holds),
            "hold_s": round(sum((h[3] - h[2]).total_seconds() for h in f_holds), 3),
            "covered_s": union_seconds([(h[2], h[3]) for h in f_holds]),
            "waits": len(f_waits),
            "wait_s": round(sum((e - s).total_seconds() for _, s, e in f_waits), 3),
        }

    other = defaultdict(lambda: {"conflicts": 0, "holds": 0, "hold_s": 0.0})
    for h in holds:
        if not any(overlaps(h[1], a) for a in anchors):
            other[h[1]]["holds"] += 1
            other[h[1]]["hold_s"] = round(other[h[1]]["hold_s"] + (h[3] - h[2]).total_seconds(), 3)
    for c in conflicts:
        if not any(overlaps(c[1], a) and overlaps(c[2], a) for a in anchors):
            other[c[1]]["conflicts"] += 1

    unique_waits = {(w[0], w[2], w[3]) for w in waits}
    return {
        "blocked_s": round(sum(blocked.values()), 3),
        "blocked_s_per_agent": blocked,
        "blocked_episodes": sum(len(v) for v in episodes.values()),
        "granted_keys_by_kind": dict(sorted(granted_kinds.items())),
        "claim_conflict_rows": len(conflicts),
        "holds": len(holds),
        "holds_open_at_end": sum(1 for h in holds if h[4]),
        "wait_episodes": len(unique_waits),
        "wait_s": round(sum((e - s).total_seconds() for _, s, e in unique_waits), 3),
        "unresolved_waits": sum(len(v) for v in pending.values()),
        "per_anchor": per_anchor,
        "per_file": per_file,
        "other_keys": dict(sorted(other.items())),
    }


def symbol_source(code_dir, anchor):
    """Source of a top-level symbol, None when the file or symbol is missing,
    "<unparseable>" when the file does not parse."""
    path, _, name = anchor.partition("#")
    try:
        text = (code_dir / path).read_text()
        tree = ast.parse(text)
    except OSError:
        return None
    except SyntaxError:
        return "<unparseable>"
    lines = text.splitlines()
    head = name.replace("::", "/").replace(".", "/").split("/")[0]
    for node in tree.body:
        names = []
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            names = [node.name]
        elif isinstance(node, ast.Assign):
            names = [t.id for t in node.targets if isinstance(t, ast.Name)]
        elif isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name):
            names = [node.target.id]
        if head.lower() in [n.lower() for n in names]:
            first = min([node.lineno] + [d.lineno for d in getattr(node, "decorator_list", [])])
            return "\n".join(lines[first - 1:node.end_lineno])
    return None


def lost_writes(tasks_spec, task_ids, run, final, run_hidden):
    """Hidden tests that passed in a done-time snapshot and fail at the end.

    ``final`` maps module file name -> set of passed test names on the final
    code; ``run_hidden(code_dir)`` returns the same for a snapshot.
    """
    snap_root = run / "snapshots"
    if not snap_root.is_dir():
        return None
    by_id = {}
    for key, full_id in task_ids.items():
        by_id[full_id] = key
    module_task = {t["hidden_test"]: t for t in tasks_spec}
    snapshots = []
    for snap in sorted(p for p in snap_root.iterdir() if (p / "meta.json").is_file()):
        meta = json.loads((snap / "meta.json").read_text())
        task_key = by_id.get(meta.get("task_id")) or next(
            (k for i, k in by_id.items() if meta.get("task_id") and i.startswith(meta["task_id"])), None)
        snapshots.append({"dir": snap, "ts": meta.get("ts"), "agent": meta.get("agent"), "task": task_key,
                          "passed": run_hidden(snap / "code")})

    regressions, per_task = [], {}
    for module, final_passed in sorted(final.items()):
        task = module_task.get(module)
        first, last = {}, {}
        for snap in snapshots:
            for test in snap["passed"].get(module, set()):
                first.setdefault(test, snap)
                last[test] = snap
        for test in sorted(first):
            if test in final_passed:
                continue
            task_anchors = (task or {}).get("anchors", [])
            changed = [a for a in task_anchors
                       if symbol_source(last[test]["dir"] / "code", a) != symbol_source(run / "repo", a)]
            regressions.append({
                "module": module, "test": test, "task": task["key"] if task else "integration",
                "anchors": changed or task_anchors, "attribution": "changed" if changed else "task",
                "first_passed_after": "%s done by %s at %s" % (first[test]["task"], first[test]["agent"],
                                                                first[test]["ts"]),
                "last_passed_after": "%s done by %s at %s" % (last[test]["task"], last[test]["agent"],
                                                               last[test]["ts"]),
            })
    for task in tasks_spec:
        module = task["hidden_test"]
        mine = [s for s in snapshots if s["task"] == task["key"]]
        at_done = mine[-1]["passed"].get(module, set()) if mine else None
        per_task[task["key"]] = {
            "done_snapshots": len(mine),
            "passed_at_done": len(at_done) if at_done is not None else None,
            "passed_at_end": len(final.get(module, set())),
            "lost_after_done": sorted(at_done - final.get(module, set())) if at_done is not None else [],
        }
    return {"snapshots": len(snapshots), "regressions": regressions, "per_task": per_task}
