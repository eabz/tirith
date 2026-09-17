#!/usr/bin/env python3
"""A scripted, LLM-free worker for the Forge scenario, to validate the kit.

Usage: fake_worker_forge.py <run_dir> <agent> [--rogue]
       fake_worker_forge.py --selfcheck

It follows the Forge WORKER_PROMPT protocol mechanically for the run's arm
(meta.json claims, hold, wait, pull; see fake_common.py) and really
implements the tasks from reference/:

- ctx-interface: the files only it changes (pipeline, the four existing
  middlewares, tests/test_pipeline.py) are copied from reference/.
- each feature task: its new middleware module is copied from reference/,
  its lines in the shared files (app/config.py, app/middlewares/__init__.py,
  app/router.py, and tests/test_router.py for request-id) are merged into
  the current file: every reference line the task owns (by keyword) is
  inserted after the nearest preceding reference line the file already has,
  so any order of tasks ends at the reference file; plus a placeholder test.
- correlation-id: covered by request-id; marked done with a note, no edits.

With --claims symbol the keys come from an ast diff of the edit
(fake_common.changed_anchors): members of Config, build_pipeline, __all__,
whole files for new files. A clean run scores 8/8 tasks and 57/57 hidden
tests. --selfcheck applies every task in 200 random orders to the scenario
and checks the result against reference/.

--rogue claims nothing, captures its files at the baseline commit, waits
until every other task is done, and writes its edits over those stale
copies, which drops the other tasks' shared-file lines (lost writes).
"""

import difflib
import json
import random
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from fake_common import FakeWorker  # noqa: E402

KIT = Path(__file__).resolve().parent.parent
SHARED = ["app/config.py", "app/middlewares/__init__.py", "app/router.py", "tests/test_router.py"]
KEYWORDS = {
    "rate-limit": ["rate_limit", "RateLimit"],
    "cors": ["cors", "Cors", "Tuple"],
    "request-id": ["request_id", "RequestId"],
    "auth": ["api_tokens", "BearerAuth"],
    "access-log": ["access_log", "AccessLog"],
    "gzip": ["gzip", "Gzip"],
}
NEW_MODULES = {"rate-limit": "ratelimit", "cors": "cors", "request-id": "request_id", "auth": "auth",
               "access-log": "access_log", "gzip": "gzip"}
CTX_FILES = ["app/pipeline.py", "app/middlewares/timing.py", "app/middlewares/security.py",
             "app/middlewares/body_limit.py", "app/middlewares/slash.py", "tests/test_pipeline.py"]
PLACEHOLDER_TEST = ("import unittest\n\n\nclass FakeTest(unittest.TestCase):\n"
                    "    def test_placeholder(self):\n        pass\n")


def lines_of(text):
    return text.splitlines(keepends=True)


def owned_changes(kit):
    """file -> task -> {"inserts": [reference line index], "replaces": [(old line, new line)]}."""
    out = {}
    for rel in SHARED:
        base = lines_of((kit / "scenario" / rel).read_text())
        ref = lines_of((kit / "reference" / rel).read_text())
        per_task = out.setdefault(rel, {})
        matcher = difflib.SequenceMatcher(None, base, ref, autojunk=False)
        for tag, i1, i2, j1, j2 in matcher.get_opcodes():
            if tag == "equal":
                continue
            for offset, j in enumerate(range(j1, j2)):
                owners = [k for k, words in KEYWORDS.items() if any(w in ref[j] for w in words)]
                if len(owners) != 1:
                    raise SystemExit("reference line %s:%d has owners %s" % (rel, j + 1, owners))
                row = per_task.setdefault(owners[0], {"inserts": [], "replaces": []})
                if tag == "replace" and i1 + offset < i2:
                    row["replaces"].append((base[i1 + offset], ref[j]))
                else:
                    row["inserts"].append(j)
    return out


def merge(current, reference, change):
    """``current`` with one task's owned reference lines merged in (idempotent)."""
    lines = lines_of(current)
    for old, new in change["replaces"]:
        if new not in lines and old in lines:
            lines[lines.index(old)] = new
    for j in change["inserts"]:
        if reference[j] in lines:
            continue
        matcher = difflib.SequenceMatcher(None, reference, lines, autojunk=False)
        at = {}
        for block in matcher.get_matching_blocks():
            for k in range(block.size):
                at[block.a + k] = block.b + k
        before = [k for k in at if k < j]
        lines.insert(at[max(before)] + 1 if before else 0, reference[j])
    return "".join(lines)


def steps_for(key, kit, changes):
    """[(rel, anchors or None, apply)] for one task key."""
    if key == "ctx-interface":
        return [(rel, None, lambda text, body=(kit / "reference" / rel).read_text(): body) for rel in CTX_FILES]
    if key not in NEW_MODULES:
        return []
    module = "app/middlewares/%s.py" % NEW_MODULES[key]
    steps = [(module, None, lambda text, body=(kit / "reference" / module).read_text(): body)]
    for rel in SHARED:
        change = changes[rel].get(key)
        if change:
            reference = lines_of((kit / "reference" / rel).read_text())
            steps.append((rel, None, lambda text, reference=reference, change=change: merge(text, reference, change)))
    steps.append(("tests/test_%s.py" % key.replace("-", "_"), None, lambda text: PLACEHOLDER_TEST))
    return steps


def selfcheck():
    changes = owned_changes(KIT)
    keys = [t["key"] for t in json.loads((KIT / "tasks.json").read_text())["tasks"]]
    rng = random.Random(7)
    for _ in range(200):
        files = {}
        order = keys[:]
        rng.shuffle(order)
        for key in order:
            for rel, _, apply in steps_for(key, KIT, changes):
                path = KIT / "scenario" / rel
                before = files.get(rel, path.read_text() if path.is_file() else None)
                files[rel] = apply(before)
                files[rel] = apply(files[rel])  # idempotent
        for rel, text in files.items():
            if rel.startswith("tests/test_") and text == PLACEHOLDER_TEST:
                continue
            if text != (KIT / "reference" / rel).read_text():
                raise SystemExit("selfcheck: %s differs from reference after order %s" % (rel, order))
    print("selfcheck ok: 200 random task orders reproduce reference/ (%d shared files)" % len(SHARED))


def main():
    if sys.argv[1:] == ["--selfcheck"]:
        return selfcheck()
    if len(sys.argv) not in (3, 4) or (len(sys.argv) == 4 and sys.argv[3] != "--rogue"):
        raise SystemExit("usage: fake_worker_forge.py <run_dir> <agent> [--rogue] | --selfcheck")
    worker = FakeWorker(sys.argv[1], sys.argv[2])
    rogue = len(sys.argv) == 4
    kit = Path(worker.meta["kit"])
    specs = {t["title"]: t for t in json.loads((kit / "tasks.json").read_text())["tasks"]}
    changes = owned_changes(kit)
    if rogue:
        time.sleep(2)  # let the honest workers take the first tasks
    worker.tb("memory_search", {})
    while True:
        task = worker.pull()
        if task is None:
            break
        key = specs[task["title"]]["key"]
        steps = steps_for(key, kit, changes)
        if rogue:
            worker.do_rogue(task, steps, others=len(specs) - 1)
            continue
        if not steps:
            worker.done(task, "covered by request-id: X-Request-ID minted/honored, ctx.request_id, outermost")
            continue
        notice = None
        if key == "ctx-interface":
            def notice():
                worker.tb("notice_publish", {
                    "kind": "signature",
                    "summary": "Middleware hooks now take ctx as the last parameter; Request.extras removed",
                    "from": "process_request(request)", "to": "process_request(request, ctx)",
                    "affected_paths": ["app/middlewares/", "app/pipeline.py"]})
        worker.do_task(task, steps, before_done=notice)
    worker.transcript.text("%s finished: %s" % (worker.agent, dict(worker.stats)))
    print("%s finished %s" % (worker.agent, json.dumps(dict(worker.stats), sort_keys=True)))


if __name__ == "__main__":
    main()
