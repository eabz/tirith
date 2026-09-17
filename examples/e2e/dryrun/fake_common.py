"""Shared plumbing for the scripted, LLM-free fake workers of the manual protocol.

A FakeWorker follows the rendered prompt's protocol paragraphs mechanically,
read from the run's meta.json:

  claims file|symbol  which keys a step claims (the scenario computes them)
  hold   task|edit    claim every step's keys at task start and release after
                      done, or claim each step just before writing and release
                      right after its check (an edit window)
  wait   sleep|server on a refused claim: sleep and retry, or claim with
                      wait_secs 120 and retry at once
  pull   poll|wait    task_pull without wait_secs and a sleep when nothing is
                      free, or task_pull with wait_secs 120

Every tb call and repository write is also recorded as a synthetic Claude
Code transcript (<run>/transcripts/<agent>.jsonl, dryrun/transcript.py), so
lib/turns.py and lib/violations.py have something to score. A sleep before a
retry is recorded in the same Bash command as the retry, as workers write it.

Timing knobs (seconds; the real prompt sleeps 30 s): FAKE_SLEEP_S (default 1),
FAKE_PREP_S "lo,hi" reading and planning without a claim (default 0.2,0.8),
FAKE_WORK_S "lo,hi" writing and checking under the claim (default 1.0,2.5).
"""

import ast
import json
import os
import random
import shlex
import subprocess
import sys
import time
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from transcript import Transcript  # noqa: E402

SLEEP_COMMAND = 'python3 -c "import time; time.sleep(30)"'


def bounds(name, default):
    lo, _, hi = os.environ.get(name, default).partition(",")
    return float(lo), float(hi or lo)


SLEEP_S = float(os.environ.get("FAKE_SLEEP_S", "1"))
PREP_S = bounds("FAKE_PREP_S", "0.2,0.8")
WORK_S = bounds("FAKE_WORK_S", "1.0,2.5")


def top_level_symbols(text):
    """name -> ast node for top-level defs, classes and assignments (imports excluded)."""
    found = {}
    for node in ast.parse(text).body:
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            found[node.name] = node
        elif isinstance(node, ast.Assign):
            for target in node.targets:
                if isinstance(target, ast.Name):
                    found[target.id] = node
        elif isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name):
            found[node.target.id] = node
    return found


def members(node):
    """name -> ast node for the methods and fields of a class body."""
    found = {}
    for child in node.body:
        if isinstance(child, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            found[child.name] = child
        elif isinstance(child, ast.Assign):
            for target in child.targets:
                if isinstance(target, ast.Name):
                    found[target.id] = child
        elif isinstance(child, ast.AnnAssign) and isinstance(child.target, ast.Name):
            found[child.target.id] = child
    return found


def changed_anchors(rel, before, after):
    """ADR-0029 keys for an edit of ``rel`` from ``before`` to ``after``.

    A new file is claimed whole. Otherwise every top-level symbol whose
    source changed, was added or removed; a changed class whose members
    differ is claimed per member (new members by their new name). Import
    lines and ``__all__`` entries ride on those claims; a file whose only
    changes are imports or ``__all__`` is claimed whole.
    """
    if before is None:
        return [rel]
    old, new = top_level_symbols(before), top_level_symbols(after)
    keys = []
    for name in list(new) + [n for n in old if n not in new]:
        a, b = old.get(name), new.get(name)
        if a is not None and b is not None and ast.get_source_segment(before, a) == ast.get_source_segment(after, b):
            continue
        if isinstance(a, ast.ClassDef) and isinstance(b, ast.ClassDef):
            ma, mb = members(a), members(b)
            diff = [m for m in list(mb) + [m for m in ma if m not in mb]
                    if m not in ma or m not in mb
                    or ast.get_source_segment(before, ma[m]) != ast.get_source_segment(after, mb[m])]
            if diff:
                keys.extend("%s#%s.%s" % (rel, name, m) for m in diff)
                continue
        keys.append("%s#%s" % (rel, name))
    keys = sorted(set(keys) - {"%s#__all__" % rel})
    return keys or [rel]


class FakeWorker:
    def __init__(self, run, agent):
        self.run = Path(run).resolve()
        self.agent = agent
        self.meta = json.loads((self.run / "meta.json").read_text())
        scenario = self.meta.get("scenario") or "forge"
        self.claims = self.meta.get("claims") or "file"
        self.hold = self.meta.get("hold") or ("task" if scenario == "forge" else "edit")
        self.wait = self.meta.get("wait") or ("sleep" if scenario == "forge" else "server")
        self.pull_mode = self.meta.get("pull") or "poll"
        self.repo = self.run / "repo"
        self.transcript = Transcript(self.run / "transcripts" / ("%s.jsonl" % agent), agent)
        self.stats = Counter()

    # -- coordination -------------------------------------------------
    def tb(self, tool, args, slept=False):
        """One tb call, recorded as a Bash tool use (after a foreground sleep when ``slept``)."""
        if slept:
            time.sleep(SLEEP_S)
        command = "TB_RUN=%s TB_AGENT=%s %s/tb %s %s" % (self.run, self.agent, self.run, tool,
                                                        shlex.quote(json.dumps(args)))
        use = self.transcript.use("Bash", {"command": (SLEEP_COMMAND + " && " if slept else "") + command})
        proc = subprocess.run([str(self.run / "tb"), tool, json.dumps(args)], capture_output=True, text=True,
                              env=dict(os.environ, TB_RUN=str(self.run), TB_AGENT=self.agent), timeout=200)
        try:
            response = json.loads(proc.stdout.strip().splitlines()[-1])
        except (json.JSONDecodeError, IndexError):
            response = {"status": "error", "message": (proc.stderr or proc.stdout)[:300]}
        self.transcript.result(use, is_error=proc.returncode != 0)
        self.stats["tb_" + tool] += 1
        return response

    def claim(self, keys, reason, attempts=400):
        args = {"paths": keys, "reason": reason, "ttl_secs": 1800}
        if self.wait == "server":
            args["wait_secs"] = 120
        slept = False
        for _ in range(attempts):
            if self.tb("claim", args, slept=slept).get("status") == "ok":
                return True
            self.stats["claim_refused"] += 1
            slept = self.wait == "sleep"
        raise SystemExit("%s: gave up claiming %s" % (self.agent, keys))

    def release(self, keys=None):
        self.tb("release", {"paths": keys} if keys else {})

    def pull(self, max_idle=400):
        """The next task, or None when no todo task is left."""
        slept = False
        for _ in range(max_idle):
            args = {"wait_secs": 120} if self.pull_mode == "wait" else {}
            pulled = self.tb("task_pull", args, slept=slept)
            if pulled.get("status") == "ok":
                return pulled["task"]
            if self.tb("task_list", {"status": "todo"}).get("total", 0) == 0:
                return None
            slept = self.pull_mode == "poll"
        return None

    def done(self, task, note):
        self.tb("task_update", {"task_id": task["id"], "status": "done", "note": note})

    # -- repository ---------------------------------------------------
    def read(self, rel):
        path = self.repo / rel
        return path.read_text() if path.is_file() else None

    def write(self, rel, text):
        """Atomic write (one temp name per process), recorded as a Write tool use."""
        path = self.repo / rel
        use = self.transcript.use("Write", {"file_path": str(path), "content": "<%d bytes>" % len(text)})
        path.parent.mkdir(parents=True, exist_ok=True)
        tmp = path.with_name("%s.fake-tmp-%d" % (path.name, os.getpid()))
        tmp.write_text(text)
        os.replace(tmp, path)
        self.transcript.result(use)
        self.stats["writes"] += 1

    def check(self):
        """Run the visible tests, recorded as a Bash tool use (is_error when they fail)."""
        use = self.transcript.use("Bash", {"command": "cd %s && python3 -m unittest" % self.repo})
        proc = subprocess.run(["python3", "-m", "unittest"], cwd=str(self.repo), capture_output=True, text=True,
                              timeout=120, env=dict(os.environ, PYTHONDONTWRITEBYTECODE="1"))
        self.transcript.result(use, is_error=proc.returncode != 0)
        self.stats["test_runs"] += 1
        self.stats["failing_test_runs"] += proc.returncode != 0

    @staticmethod
    def pause(span):
        time.sleep(random.uniform(*span))

    # -- one task -----------------------------------------------------
    def keys_for(self, step, before):
        rel, anchors, apply = step
        if self.claims != "symbol":
            return [rel]
        if anchors is not None:
            return anchors
        return changed_anchors(rel, before, apply(before))

    def do_task(self, task, steps, before_done=None, note="fake worker"):
        """Implement ``steps`` [(rel, anchors or None, apply(text or None) -> text)] under the arm's protocol."""
        reason = task["title"]
        if self.hold == "task":
            self.pause(PREP_S)
            keys = []
            for step in steps:
                for key in self.keys_for(step, self.read(step[0])):
                    if key not in keys:
                        keys.append(key)
            if keys:
                self.claim(keys, reason)
            for rel, _, apply in steps:
                self.write(rel, apply(self.read(rel)))
                self.pause(WORK_S)
                self.check()
        else:
            for step in steps:
                rel, _, apply = step
                self.pause(PREP_S)
                keys = self.keys_for(step, self.read(rel))
                self.claim(keys, reason)
                self.write(rel, apply(self.read(rel)))  # re-read under the claim
                self.pause(WORK_S)
                self.check()
                self.release(keys)
        if before_done:
            before_done()
        self.done(task, note)
        self.release()

    def do_rogue(self, task, steps, others):
        """Claim nothing: capture the files at the baseline commit, wait until ``others`` tasks are done,
        then write this task's edits over those stale copies."""
        stale = {}
        for rel, _, _ in steps:
            proc = subprocess.run(["git", "-C", str(self.repo), "show", "HEAD:" + rel], capture_output=True, text=True)
            stale[rel] = proc.stdout if proc.returncode == 0 else None
        for _ in range(1200):
            if self.tb("task_list", {"status": "done", "limit": 50}).get("total", 0) >= others:
                break
            time.sleep(0.5)
        for rel, _, apply in steps:
            self.write(rel, apply(stale[rel]))
        self.done(task, "rogue fake worker")
