#!/usr/bin/env python3
"""A scripted, LLM-free worker for the hubs scenario, to validate the kit.

Usage: fake_worker_hubs.py <run_dir> <agent> [--rogue]

It follows the hubs WORKER_PROMPT protocol mechanically for the run's arm
(meta.json claims, hold, wait, pull; see fake_common.py): pull a task, and
for each shared file claim the file or the task's anchors in it, copy those
top-level symbols from reference/ into the file (read, splice, atomic
write), check, release (edit windows) or hold everything until done
(--hold task); write a trivial test file, publish a notice for the breaking
task, mark done, release. Unlike a marker-only fake it really implements the
tasks, so a clean run reaches full hidden acceptance. Tool calls and writes
go to a synthetic transcript for the turn and violation scorers.

--rogue simulates the failure the scenario must detect: it claims nothing,
copies its files as they were at the baseline commit, waits until every
other task is done, then writes its symbols into those stale copies,
overwriting everyone else's edits to the same files.
"""

import ast
import json
import os
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from fake_common import FakeWorker  # noqa: E402

PLACEHOLDER_TEST = ("import unittest\n\n\nclass FakeTest(unittest.TestCase):\n"
                    "    def test_placeholder(self):\n        pass\n")


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


def steps_for(spec, scen):
    """[(rel, anchors, apply)] for one task: its anchor groups, its test file, and for the break tests/test_money.py."""
    groups = {}
    for anchor in spec["anchors"]:
        path, _, symbol = anchor.partition("#")
        groups.setdefault(path, []).append(symbol)
    steps = []
    for path, symbols in groups.items():
        reference = (scen / "reference" / path).read_text()
        steps.append((path, ["%s#%s" % (path, s) for s in symbols],
                      lambda text, reference=reference, symbols=symbols: splice(text, reference, symbols)))
    steps.append(("tests/test_%s.py" % spec["key"].replace("-", "_"), None, lambda text: PLACEHOLDER_TEST))
    if spec["title"].startswith("BREAKING"):
        money_tests = (scen / "reference" / "tests" / "test_money.py").read_text()
        steps.append(("tests/test_money.py", ["tests/test_money.py"], lambda text: money_tests))
    return steps


def main():
    if len(sys.argv) not in (3, 4) or (len(sys.argv) == 4 and sys.argv[3] != "--rogue"):
        raise SystemExit("usage: fake_worker_hubs.py <run_dir> <agent> [--rogue]")
    worker = FakeWorker(sys.argv[1], sys.argv[2])
    rogue = len(sys.argv) == 4
    scen = Path(worker.meta["scenario_dir"])
    specs = {t["title"]: t for t in json.loads((scen / "tasks.json").read_text())["tasks"]}
    if rogue:
        time.sleep(2)  # let the honest workers take the first tasks
    worker.tb("memory_search", {})
    while True:
        task = worker.pull()
        if task is None:
            break
        spec = specs[task["title"]]
        steps = steps_for(spec, scen)
        if rogue:
            worker.do_rogue(task, steps, others=len(specs) - 1)
            continue
        notice = None
        if spec["title"].startswith("BREAKING"):
            def notice():
                worker.tb("notice_publish", {
                    "kind": "signature", "summary": "round_money(amount, currency): currency is now required",
                    "from": "round_money(amount)", "to": "round_money(amount, currency)",
                    "affected_paths": ["tally/money.py", "tally/pricing.py"]})
        worker.do_task(task, steps, before_done=notice)
    worker.transcript.text("%s finished: %s" % (worker.agent, dict(worker.stats)))
    print("%s finished %s" % (worker.agent, json.dumps(dict(worker.stats), sort_keys=True)))


if __name__ == "__main__":
    main()
