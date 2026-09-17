#!/usr/bin/env python3
"""A scripted, LLM-free worker for the hubs scenario, to validate the kit.

Usage: fake_worker_hubs.py <run_dir> <agent> [--rogue]

It follows the hubs WORKER_PROMPT protocol mechanically for the run's arm
(meta.json: claims file|symbol, wait sleep|server): pull a task, and for
each shared file claim the file or the task's anchors in it, copy those
top-level symbols from reference/ into the file (read, splice, atomic
write), release, write a trivial test file, publish a notice for the
breaking task, mark done, release. Unlike the Forge fake it really
implements the tasks, so a clean run reaches full hidden acceptance.

--rogue simulates the failure the scenario must detect: it claims nothing,
copies its files as they were at the baseline commit, waits until every
other task is done, then writes its symbols into those stale copies,
overwriting everyone else's edits to the same files.
"""

import ast
import json
import os
import random
import subprocess
import sys
import time
from pathlib import Path


def tb(run, agent, tool, args):
    proc = subprocess.run([str(run / "tb"), tool, json.dumps(args)], capture_output=True, text=True,
                          env=dict(os.environ, TB_RUN=str(run), TB_AGENT=agent), timeout=180)
    try:
        return json.loads(proc.stdout.strip().splitlines()[-1])
    except (json.JSONDecodeError, IndexError):
        return {"status": "error", "message": (proc.stderr or proc.stdout)[:300]}


def symbol_span(tree, name):
    """1-based (first, last) line of a top-level def, class or assignment named ``name``."""
    for node in tree.body:
        names = []
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            names = [node.name]
        elif isinstance(node, ast.Assign):
            names = [t.id for t in node.targets if isinstance(t, ast.Name)]
        elif isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name):
            names = [node.target.id]
        if name in names:
            first = min([node.lineno] + [d.lineno for d in getattr(node, "decorator_list", [])])
            return first, node.end_lineno
    return None


def splice(current, reference, symbols):
    """``current`` with each symbol's source replaced by (or appended from) ``reference``."""
    ref_lines = reference.splitlines(keepends=True)
    ref_tree = ast.parse(reference)
    for name in symbols:
        span = symbol_span(ref_tree, name)
        if span is None:
            raise SystemExit("symbol %s not in reference" % name)
        segment = ref_lines[span[0] - 1:span[1]]
        lines = current.splitlines(keepends=True)
        here = symbol_span(ast.parse(current), name)
        if here is None:
            lines = lines + ["\n", "\n"] + segment
        else:
            lines = lines[:here[0] - 1] + segment + lines[here[1]:]
        current = "".join(lines)
    return current


def write_atomic(path, text):
    # One temporary name per process: with symbol claims two workers may
    # write the same file at once.
    tmp = path.with_name("%s.fake-tmp-%d" % (path.name, os.getpid()))
    tmp.write_text(text)
    os.replace(tmp, path)


def claim(run, agent, meta, keys, reason):
    args = {"paths": keys, "reason": reason, "ttl_secs": 1800}
    if meta.get("wait") == "server":
        args["wait_secs"] = 120
    for _ in range(240):
        response = tb(run, agent, "claim", args)
        if response.get("status") == "ok":
            return True
        time.sleep(0.5)
    return False


def main():
    if len(sys.argv) not in (3, 4) or (len(sys.argv) == 4 and sys.argv[3] != "--rogue"):
        raise SystemExit("usage: fake_worker_hubs.py <run_dir> <agent> [--rogue]")
    run, agent, rogue = Path(sys.argv[1]).resolve(), sys.argv[2], len(sys.argv) == 4
    meta = json.loads((run / "meta.json").read_text())
    scen = Path(meta["scenario_dir"])
    specs = {t["title"]: t for t in json.loads((scen / "tasks.json").read_text())["tasks"]}
    repo = run / "repo"
    if rogue:
        time.sleep(2)  # let the honest workers take the first tasks
    tb(run, agent, "memory_search", {})
    idle = 0
    while idle < 120:
        pulled = tb(run, agent, "task_pull", {})
        if pulled.get("status") != "ok":
            if tb(run, agent, "task_list", {"status": "todo"}).get("total", 0) == 0:
                break
            idle += 1
            time.sleep(0.5)
            continue
        task = pulled["task"]
        spec = specs[task["title"]]
        groups = {}
        for anchor in spec["anchors"]:
            path, _, symbol = anchor.partition("#")
            groups.setdefault(path, []).append(symbol)
        if rogue:
            stale = {p: subprocess.run(["git", "-C", str(repo), "show", "HEAD:" + p], capture_output=True,
                                       text=True).stdout for p in groups}
            others = len(specs) - 1
            for _ in range(600):
                if tb(run, agent, "task_list", {"status": "done", "limit": 50}).get("total", 0) >= others:
                    break
                time.sleep(0.5)
        for path, symbols in groups.items():
            keys = [path] if meta.get("claims") != "symbol" else ["%s#%s" % (path, s) for s in symbols]
            if not rogue and not claim(run, agent, meta, keys, task["title"]):
                raise SystemExit("%s: gave up claiming %s" % (agent, keys))
            target = repo / path
            base = stale[path] if rogue else target.read_text()
            write_atomic(target, splice(base, (scen / "reference" / path).read_text(), symbols))
            time.sleep(random.uniform(1.0, 2.5))
            if not rogue:
                tb(run, agent, "release", {"paths": keys})
        test_path = "tests/test_%s.py" % spec["key"].replace("-", "_")
        if not rogue:
            claim(run, agent, meta, [test_path], task["title"])
        write_atomic(repo / test_path, "import unittest\n\n\nclass FakeTest(unittest.TestCase):\n"
                                       "    def test_placeholder(self):\n        pass\n")
        if spec["title"].startswith("BREAKING"):
            if not rogue:
                claim(run, agent, meta, ["tests/test_money.py"], task["title"])
                (repo / "tests" / "test_money.py").write_text((scen / "reference" / "tests" / "test_money.py").read_text())
            tb(run, agent, "notice_publish", {"kind": "signature", "summary": "round_money(amount, currency): currency is now required",
                                              "from": "round_money(amount)", "to": "round_money(amount, currency)",
                                              "affected_paths": ["tally/money.py", "tally/pricing.py"]})
        tb(run, agent, "task_update", {"task_id": task["id"], "status": "done", "note": "fake worker"})
        tb(run, agent, "release", {})
    print("%s finished" % agent)


if __name__ == "__main__":
    main()
