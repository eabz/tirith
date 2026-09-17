#!/usr/bin/env python3
"""LLM-free plumbing check for --protocol hooks (hubs scenario).

Usage:
  fake_worker_hooks.py <run_dir> selfcheck
  fake_worker_hooks.py <run_dir> <agent> [--auto-identity]

A worker plays a Claude Code session: it never calls claim, release,
task_pull or task_update itself. It sends the hook scripts in
<run_dir>/hooks the stdin JSON Claude Code documents for UserPromptSubmit,
PreToolUse, PostToolUse and Stop, obeys their outputs (a denied edit is
retried later, a blocked stop carries the next task or the gate's reason),
implements each task by copying its symbols from reference/ (as
fake_worker_hubs.py does), and appends a synthetic transcript
(dryrun/transcript.py: tool_use and tool_result rows, a denied edit as an
error result) so the scorer's turn and violation metrics run. Its first stop
on its first task omits the acceptance criteria, to exercise the gate's
continue path. The reference symbols use other tasks' symbols, so a task's
own acceptance checks would fail until those tasks land and every worker
would wait in the gate on a task nobody is working on. A real worker writes
code that works with what is there; the fake instead also splices the
reference symbols of the other tasks its checks need (its closure, computed
once per run on the baseline and cached in <run>/fake_closures.json). That
keeps the gate's decision about the task's own checks while the full suite
is still red. --auto-identity launches without TB_AGENT, so the hooks assign
the name.

selfcheck runs targeted checks on the same run before workers start
(deny with holder, brief context, release on the next non-edit call,
in-flight edits not released, Bash write claimed after the fact, helper
SubagentStop ignored, auto identity, settings matchers, gate criteria
parsing, gate test scope, violation counting, transcript turn classes, hooks
prompt) and exits non-zero on failure.
"""

import json
import os
import random
import re
import subprocess
import sys
import time
import uuid
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent / "lib"))
sys.path.insert(0, str(HERE.parent / "hooks"))
from fake_worker_hubs import splice, write_atomic  # noqa: E402
from transcript import Transcript  # noqa: E402


class Session:
    def __init__(self, run, agent, env_identity=True):
        self.run, self.agent, self.env_identity = run, agent, env_identity
        self.session_id = str(uuid.uuid4())
        self.repo = run / "repo"
        tdir = run / "fake_transcripts"
        tdir.mkdir(exist_ok=True)
        self.transcript = tdir / ("%s.jsonl" % (agent or self.session_id[:8]))
        self.stop_active = False
        self.stats = {"denied": 0, "edits": 0, "stops": 0, "gate_continues": 0, "tasks": 0}

    def event(self, name, **fields):
        base = {"session_id": self.session_id, "transcript_path": str(self.transcript), "cwd": str(self.repo),
                "permission_mode": "acceptEdits", "hook_event_name": name}
        base.update(fields)
        return base

    def hook(self, script, event):
        env = {k: v for k, v in os.environ.items() if k not in ("TB_RUN", "TB_AGENT", "TB_ORIGIN")}
        if self.env_identity and self.agent:
            env["TB_AGENT"] = self.agent
        proc = subprocess.run([str(self.run / "hooks" / script)], input=json.dumps(event), capture_output=True,
                              text=True, env=env, timeout=700)
        out = {}
        if proc.stdout.strip():
            out = json.loads(proc.stdout)
        return proc.returncode, out

    def record(self):
        return Transcript(self.transcript, self.session_id[:8])

    def tool(self, name, tool_input):
        """A non-edit tool call: PreToolUse (releases), the tool's result, then PostToolUse."""
        use_id = self.record().use(name, tool_input)
        _, pre = self.hook("pre_other.py", self.event("PreToolUse", tool_name=name, tool_input=tool_input,
                                                      tool_use_id=use_id))
        self.record().result(use_id)
        if name == "Bash":
            self.hook("post_edit.py", self.event("PostToolUse", tool_name=name, tool_input=tool_input,
                                                 tool_response={"stdout": "", "stderr": "", "interrupted": False,
                                                                "isImage": False}, tool_use_id=use_id))
        return pre

    def edit(self, rel, make_text, tool="Edit", attempts=400):
        """Edit or Write a file through the hooks; make_text(current) -> new text, computed once allowed."""
        path = self.repo / rel
        for _ in range(attempts):
            current = path.read_text() if path.exists() else ""
            tool_input = {"file_path": str(path), "content": "<new>"} if tool == "Write" else \
                {"file_path": str(path), "old_string": current[:40], "new_string": "<new>", "replace_all": False}
            use_id = self.record().use(tool, tool_input)
            _, pre = self.hook("pre_edit.py", self.event("PreToolUse", tool_name=tool, tool_input=tool_input,
                                                         tool_use_id=use_id))
            if (pre.get("hookSpecificOutput") or {}).get("permissionDecision") == "deny":
                self.record().result(use_id, is_error=True, text=pre["hookSpecificOutput"].get(
                    "permissionDecisionReason", ""))
                self.stats["denied"] += 1
                time.sleep(random.uniform(0.3, 1.0))
                self.tool("Read", {"file_path": str(path)})
                continue
            path.parent.mkdir(parents=True, exist_ok=True)
            write_atomic(path, make_text(path.read_text() if path.exists() else ""))
            self.record().result(use_id)
            self.stats["edits"] += 1
            self.hook("post_edit.py", self.event("PostToolUse", tool_name=tool, tool_input=tool_input,
                                                 tool_response={"filePath": str(path), "type": "update"},
                                                 tool_use_id=use_id, duration_ms=3))
            return pre
        raise SystemExit("%s: edit of %s denied %d times" % (self.agent, rel, attempts))

    def stop(self, message):
        self.stats["stops"] += 1
        self.record().text(message)
        _, out = self.hook("stop.py", self.event("Stop", stop_hook_active=self.stop_active,
                                                 last_assistant_message=message, background_tasks=[],
                                                 session_crons=[]))
        self.stop_active = out.get("decision") == "block"
        return out


TASK_RE = re.compile(r"Tirith assigned you task (\S+): (.*)")


def anchor_groups(specs_by_key, keys):
    """path -> symbols for the anchors of the given tasks, in task order."""
    groups = {}
    for key in keys:
        for anchor in specs_by_key[key]["anchors"]:
            path, _, symbol = anchor.partition("#")
            if symbol not in groups.setdefault(path, []):
                groups[path].append(symbol)
    return groups


def closures(run, scen, specs_by_key):
    """task key -> task keys whose reference symbols its acceptance checks need (itself included).

    Starts from every task and drops each other task whose symbols the checks
    pass without, on a scratch copy of the baseline. Cached per run; parallel
    workers wait on a lock while the first one computes (~3 s).
    """
    import fcntl
    import shutil
    import tempfile
    import stop_gate
    cache = run / "fake_closures.json"
    with open(str(run / "fake_closures.lock"), "a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        if cache.is_file():
            return json.loads(cache.read_text())
        result = {}
        with tempfile.TemporaryDirectory(prefix="fake_closure_") as tmp:
            repo = Path(tmp) / "repo"
            shutil.copytree(str(scen / "scenario"), str(repo))
            baseline = {path: path.read_text() for path in repo.rglob("*.py")}
            package = repo / "closure_checks"
            package.mkdir()
            (package / "__init__.py").write_text("")
            for spec in specs_by_key.values():
                shutil.copy(str(scen / "hidden_tests" / spec["hidden_test"]), str(package / spec["hidden_test"]))

            def passes(keys, module):
                for path, text in baseline.items():
                    path.write_text(text)
                for rel, symbols in anchor_groups(specs_by_key, keys).items():
                    target = repo / rel
                    target.write_text(splice(target.read_text(), (scen / "reference" / rel).read_text(), symbols))
                proc = subprocess.run([sys.executable, "-B", "-c", stop_gate.RUNNER,
                                       "closure_checks." + module[:-3]], cwd=str(repo), capture_output=True,
                                      text=True, timeout=120)
                out = json.loads(proc.stdout.strip().splitlines()[-1])
                return not out["import_error"] and not out["failed"] and out["run"] > 0

            for key, spec in specs_by_key.items():
                chosen = [key] + [k for k in specs_by_key if k != key]
                if passes(chosen, spec["hidden_test"]):
                    for other in list(chosen[1:]):
                        trial = [k for k in chosen if k != other]
                        if passes(trial, spec["hidden_test"]):
                            chosen = trial
                result[key] = chosen
        write_atomic(cache, json.dumps(result, indent=1))
        return result


def summary_for(spec):
    import stop_gate
    criteria = stop_gate.acceptance_criteria(spec["description"])
    return "Implemented %s. How each acceptance criterion is met:\n%s\npython3 -m unittest passes." % (
        spec["title"], "\n".join("- " + crit for crit in criteria))


def implement(session, spec, scen, specs_by_key):
    groups = anchor_groups(specs_by_key, closures(session.run, scen, specs_by_key)[spec["key"]])
    for path in groups:
        session.tool("Read", {"file_path": str(session.repo / path)})
    for path, symbols in groups.items():
        reference = (scen / "reference" / path).read_text()
        session.edit(path, lambda current, r=reference, s=symbols: splice(current, r, s))
        time.sleep(random.uniform(0.3, 1.0))
        session.tool("Bash", {"command": "python3 -m unittest", "description": "Run tests"})
    test_path = "tests/test_%s.py" % spec["key"].replace("-", "_")
    session.edit(test_path, lambda _: "import unittest\n\n\nclass FakeTest(unittest.TestCase):\n"
                                      "    def test_placeholder(self):\n        pass\n", tool="Write")
    if spec["title"].startswith("BREAKING"):
        ref = (scen / "reference" / "tests" / "test_money.py").read_text()
        session.edit("tests/test_money.py", lambda _: ref, tool="Write")
        agent = session.agent or identity_of(session)
        args = {"kind": "signature", "summary": "round_money(amount, currency): currency is now required",
                "from": "round_money(amount)", "to": "round_money(amount, currency)",
                "affected_paths": ["tally/money.py", "tally/pricing.py"]}
        use_id = session.record().use("Bash", {"command": "TB_RUN=%s TB_AGENT=%s %s/tb notice_publish '%s'"
                                                   % (session.run, agent, session.run, json.dumps(args))})
        subprocess.run([str(session.run / "tb"), "notice_publish", json.dumps(args)], capture_output=True,
                       env=dict(os.environ, TB_RUN=str(session.run), TB_AGENT=agent), timeout=60)
        session.record().result(use_id)
    session.tool("Bash", {"command": "python3 -m unittest", "description": "Run tests"})


def identity_of(session):
    ids = json.loads((session.run / "hooks_state" / "identity.json").read_text())
    return ids["sessions"][session.session_id]


def worker(run, agent, auto_identity):
    meta = json.loads((run / "meta.json").read_text())
    scen = Path(meta["scenario_dir"])
    specs = {t["title"]: t for t in json.loads((scen / "tasks.json").read_text())["tasks"]}
    specs_by_key = {t["key"]: t for t in specs.values()}
    session = Session(run, None if auto_identity else agent, env_identity=not auto_identity)
    if auto_identity:
        session.transcript = run / "fake_transcripts" / ("%s-auto.jsonl" % agent)
    _, out = session.hook("prompt_submit.py", session.event("UserPromptSubmit",
                                                            prompt="You are a coding agent. Tasks arrive automatically."))
    text = (out.get("hookSpecificOutput") or {}).get("additionalContext", "")
    first_stop_sloppy = agent.endswith("1")
    for _ in range(200):
        match = TASK_RE.search(text)
        if match:
            spec = specs[match.group(2).strip()]
            session.stats["tasks"] += 1
            implement(session, spec, scen, specs_by_key)
            message = "Done." if first_stop_sloppy else summary_for(spec)
            first_stop_sloppy = False
            out = session.stop(message)
            while out.get("decision") == "block" and out.get("reason", "").startswith("Task "):
                session.stats["gate_continues"] += 1
                time.sleep(random.uniform(2.0, 4.0))  # the task's checks may wait on other tasks' symbols
                session.tool("Bash", {"command": "python3 -m unittest", "description": "Re-run tests"})
                out = session.stop(summary_for(spec))
        elif "final report" in text:
            out = session.stop("Final report: tasks done, see board.")
        else:
            out = session.stop("Nothing assigned yet.")
        if out.get("decision") != "block":
            break
        text = out.get("reason", "")
    name = session.agent or identity_of(session)
    print(json.dumps(dict(session.stats, agent=name)))


# ---------------------------------------------------------------- selfcheck

def selfcheck(run):
    failures, checks = [], []

    def check(name, ok, detail=""):
        print("%s %s%s" % ("PASS" if ok else "FAIL", name, (": " + str(detail)[:300]) if detail and not ok else ""))
        checks.append(name)
        if not ok:
            failures.append(name)

    def tb(agent, tool, args):
        proc = subprocess.run([str(run / "tb"), tool, json.dumps(args)], capture_output=True, text=True,
                              env=dict(os.environ, TB_RUN=str(run), TB_AGENT=agent), timeout=120)
        return json.loads(proc.stdout.strip().splitlines()[-1])

    def holders(path):
        rows = tb("sc-observer", "claims_list", {"path": path}).get("claims", [])
        return sorted({r.get("agent") or r.get("owner") for r in rows})

    def state(agent):
        return json.loads((run / "hooks_state" / "agents" / ("%s.json" % agent)).read_text())

    repo = run / "repo"
    a = Session(run, "sc-a")
    # Settings: valid, executable commands, matchers.
    settings = json.loads((repo / ".claude" / "settings.json").read_text())
    commands = [h["command"] for groups in settings["hooks"].values() for g in groups for h in g["hooks"]]
    check("settings commands exist and are executable", all(os.access(cmd, os.X_OK) for cmd in commands), commands)
    pre = settings["hooks"]["PreToolUse"]
    edit_re = re.compile(pre[0]["matcher"])
    check("edit matcher selects edit tools only",
          all(edit_re.search(t) for t in ("Edit", "Write", "MultiEdit", "mcp__serena__replace_symbol_body"))
          and not any(edit_re.search(t) for t in ("Bash", "Read", "NotebookRead", "mcp__serena__find_symbol")))
    check("settings live only in the run", str(run) in commands[0] and "GitHub/tirith/examples" not in commands[0])
    # Prompt variant.
    prompt = subprocess.run(["python3", str(HERE.parent / "lib" / "prompt.py"), str(run), "w1"], capture_output=True,
                            text=True).stdout
    check("hooks prompt has no protocol steps", prompt and not re.search(r"task_pull|task_update|tb claim|tb release",
                                                                          prompt), prompt[:200])
    # Outside the repo: allowed, nothing claimed.
    _, out = a.hook("pre_edit.py", a.event("PreToolUse", tool_name="Write", tool_input={"file_path": "/tmp/x.txt",
                                                                                       "content": ""}))
    check("edit outside the repo passes untouched", out == {}, out)
    # Deny with holder.
    tb("sc-blocker", "claim", {"paths": ["tally/rules.py"], "reason": "selfcheck blocker", "brief": False})
    started = time.time()
    _, out = a.hook("pre_edit.py", a.event("PreToolUse", tool_name="Edit",
                                           tool_input={"file_path": str(repo / "tally/rules.py"), "old_string": "a",
                                                       "new_string": "b"}))
    hso = out.get("hookSpecificOutput") or {}
    check("refused claim denies the edit", hso.get("permissionDecision") == "deny", out)
    check("deny reason names the holder", "sc-blocker" in hso.get("permissionDecisionReason", ""), hso)
    check("deny waited about claim_wait_secs", time.time() - started >= 1.5, round(time.time() - started, 2))
    check("denied file not in state", "tally/rules.py" not in state("sc-a")["files"])
    tb("sc-blocker", "release", {})
    # Grant, brief as context.
    _, out = a.hook("pre_edit.py", a.event("PreToolUse", tool_name="Edit",
                                           tool_input={"file_path": str(repo / "tally/rules.py"), "old_string": "a",
                                                       "new_string": "b"}))
    ctx = (out.get("hookSpecificOutput") or {}).get("additionalContext", "")
    check("granted edit is not denied", (out.get("hookSpecificOutput") or {}).get("permissionDecision") != "deny", out)
    check("claim held by sc-a", holders("tally/rules.py") == ["sc-a"], holders("tally/rules.py"))
    rows = json.loads(re.search(r": (\{.*\})$", ctx, re.S).group(1)) if "Coordination brief" in ctx else {}
    check("brief context only when rows exist", ("Coordination brief" in ctx) == bool(rows), ctx[:200])
    print("INFO brief context on tally/rules.py: %s" % ({k: len(v) for k, v in rows.items() if k != "more"} or "none"))
    # In flight: a parallel non-edit call must not release it.
    a.hook("pre_other.py", a.event("PreToolUse", tool_name="Bash", tool_input={"command": "ls"}))
    check("in-flight edit survives a parallel non-edit call", holders("tally/rules.py") == ["sc-a"])
    a.hook("post_edit.py", a.event("PostToolUse", tool_name="Edit",
                                   tool_input={"file_path": str(repo / "tally/rules.py")},
                                   tool_response={"filePath": str(repo / "tally/rules.py")}))
    check("post edit marks pending", state("sc-a")["files"]["tally/rules.py"]["pending"] is True)
    # A second edit to the same file reuses the claim.
    _, out = a.hook("pre_edit.py", a.event("PreToolUse", tool_name="Edit",
                                           tool_input={"file_path": str(repo / "tally/rules.py")}))
    check("second edit reuses the held claim", out == {} and state("sc-a")["files"]["tally/rules.py"]["inflight"] == 1)
    a.hook("post_edit.py", a.event("PostToolUseFailure", tool_name="Edit",
                                   tool_input={"file_path": str(repo / "tally/rules.py")}, error="old_string not found"))
    a.hook("pre_other.py", a.event("PreToolUse", tool_name="Read", tool_input={"file_path": str(repo / "README")}))
    check("next non-edit call releases", holders("tally/rules.py") == [], holders("tally/rules.py"))
    # Bash write claimed after the fact.
    a.hook("post_edit.py", a.event("PostToolUse", tool_name="Bash", tool_input={"command": "sed -i ..."},
                                   tool_response={"stdout": "", "bashEditDiff": {"changedFiles": [str(repo / "tally/money.py")]}}))
    check("bash-changed file claimed after the fact", holders("tally/money.py") == ["sc-a"], holders("tally/money.py"))
    a.hook("pre_other.py", a.event("PreToolUse", tool_name="Grep", tool_input={"pattern": "x"}))
    check("and released on the next call", holders("tally/money.py") == [])
    # Helper subagent stop does nothing.
    rc, out = a.hook("stop.py", a.event("SubagentStop", stop_hook_active=False, agent_id="helper-1",
                                        agent_type="Explore", agent_transcript_path="/tmp/none.jsonl",
                                        last_assistant_message="done"))
    check("helper SubagentStop is a no-op", rc == 0 and out == {} and state("sc-a")["task"] is None, out)
    # Auto identity.
    anon = Session(run, None, env_identity=False)
    anon.hook("pre_other.py", anon.event("PreToolUse", tool_name="Read", tool_input={"file_path": "x"}))
    name = identity_of(anon)
    check("auto identity assigned and recorded", name.startswith("hk-"), name)
    reg = json.loads((run / "hooks_state" / "sessions" / ("%s.json" % anon.session_id)).read_text())
    check("session registry keeps transcript path", reg.get("transcript_path") == str(anon.transcript), reg)
    # Gate parsing on every hubs task.
    import stop_gate
    meta = json.loads((run / "meta.json").read_text())
    specs = json.loads((Path(meta["scenario_dir"]) / "tasks.json").read_text())["tasks"]
    counts = [len(stop_gate.acceptance_criteria(t["description"])) for t in specs]
    check("acceptance criteria found in every task", all(counts), counts)
    check("summary mentions all criteria", all(
        all(stop_gate.mentioned(c, summary_for(t)) for c in stop_gate.acceptance_criteria(t["description"]))
        for t in specs))
    check("'Done.' mentions none", not any(stop_gate.mentioned(c, "Done.") for t in specs
                                           for c in stop_gate.acceptance_criteria(t["description"])
                                           if stop_gate.criterion_terms(c)))
    # Gate test scope: the task's own acceptance module decides, not the full suite.
    import shutil
    import tempfile
    cfg = dict(json.loads((run / "hooks_state" / "config.json").read_text()))
    cfg.update(test_command=["python3", "-m", "unittest"], gate_max_continues=2, gate_tests="task")
    by_key = {t["key"]: t for t in specs}
    ids = json.loads((run / "task_ids.json").read_text())
    scen = Path(meta["scenario_dir"])
    with tempfile.TemporaryDirectory(prefix="gate_selfcheck_") as tmp:
        grun = Path(tmp)
        shutil.copy(str(run / "meta.json"), str(grun / "meta.json"))
        shutil.copy(str(run / "task_ids.json"), str(grun / "task_ids.json"))
        shutil.copytree(str(scen / "scenario"), str(grun / "repo"))
        own = by_key["parse-money"]
        money = grun / "repo" / "tally" / "money.py"
        money.write_text(splice(money.read_text(), (scen / "reference" / "tally" / "money.py").read_text(),
                                ["parse_money"]))
        (grun / "repo" / "tests" / "test_other_task_unfinished.py").write_text(
            "import unittest\n\n\nclass OtherTask(unittest.TestCase):\n    def test_not_landed(self):\n"
            "        self.fail('another task is half done')\n")

        def verdict(spec, scope="task", continues=0, message=None):
            task = {"id": ids[spec["key"]][:8], "title": spec["title"], "description": spec["description"]}
            gate_cfg = dict(cfg, gate_tests=scope)
            return stop_gate.PlaceholderGate().evaluate(stop_gate.GateInput(
                task=task, last_message=message if message is not None else summary_for(spec),
                repo=grun / "repo", agent="sc-gate", config=gate_cfg, continues=continues, run=grun))

        v = verdict(own, continues=5)
        check("gate: own tests pass, suite red -> done, never escalates",
              v.decision == "done" and v.evidence.get("suite_ok") is False and v.evidence["tests_scope"] == "task",
              (v.decision, v.evidence))
        v = verdict(own, message="Done.")
        check("gate: own tests pass, criteria unaddressed -> continue", v.decision == "continue", (v.decision, v.reason))
        v = verdict(by_key["bulk-discount"])
        check("gate: own tests fail -> continue naming the failing checks",
              v.decision == "continue" and "acceptance checks" in v.reason and "test_" in v.reason
              and "Traceback" not in v.reason, (v.decision, v.reason[:300]))
        v = verdict(by_key["bulk-discount"], continues=2)
        check("gate: own tests still fail after the limit -> escalate", v.decision == "escalate", v.decision)
        v = verdict(own, scope="suite")
        check("gate: suite scope keeps the old behavior", v.decision == "continue" and
              v.evidence["tests_scope"] == "suite", (v.decision, v.evidence))
        unknown = stop_gate.task_acceptance_module(grun, {"id": "ffffffff", "title": "not a scenario task"})
        check("gate: a task the scenario does not know falls back to the suite", unknown is None, unknown)
    # Violations: writes without the writer's own live claim.
    import violations
    with tempfile.TemporaryDirectory(prefix="violations_selfcheck_") as tmp:
        vrun = Path(tmp)
        for rel in ("tally/a.py", "tally/b.py", "tally/c.py", "tally/d.py"):
            (vrun / "repo" / rel).parent.mkdir(parents=True, exist_ok=True)
            (vrun / "repo" / rel).write_text("x = 1\n")
        stamp = lambda sec: "2026-09-17T10:00:%02d.000Z" % sec  # noqa: E731
        coord = [{"ts": stamp(1), "agent": "v1", "tool": "claim", "request": {"paths": ["tally/a.py"]},
                  "response": {"status": "ok"}},
                 {"ts": stamp(1), "agent": "v1", "tool": "claim", "request": {"paths": ["tally/d.py#f"]},
                  "response": {"status": "ok"}},
                 {"ts": stamp(1), "agent": "v2", "tool": "claim", "request": {"paths": ["tally/b.py"]},
                  "response": {"status": "ok"}},
                 {"ts": stamp(2), "agent": "v1", "tool": "claim", "request": {"paths": ["tally/b.py"]},
                  "response": {"status": "conflict"}},
                 {"ts": stamp(5), "agent": "v1", "tool": "release", "request": {},
                  "response": {"status": "ok", "released": ["tally/a.py", "tally/d.py::f"]}}]
        (vrun / "coord_full.jsonl").write_text("".join(json.dumps(r) + "\n" for r in coord))
        rows = []
        for n, (sec, tool, tool_input, is_error) in enumerate([
                (3, "Edit", {"file_path": str(vrun / "repo/tally/a.py")}, False),   # covered
                (6, "Edit", {"file_path": str(vrun / "repo/tally/a.py")}, False),   # after release
                (3, "Write", {"file_path": str(vrun / "repo/tally/b.py")}, False),  # held by v2
                (3, "Edit", {"file_path": str(vrun / "repo/tally/b.py")}, True),    # denied: not a write
                (3, "Bash", {"command": "echo y > tally/c.py"}, False),              # shell write, unclaimed
                (3, "Edit", {"file_path": str(vrun / "repo/tally/d.py")}, False),   # anchor claim only
                (3, "Read", {"file_path": str(vrun / "repo/tally/a.py")}, False)]):
            use_id = "toolu_v%d" % n
            rows.append({"type": "assistant", "timestamp": stamp(sec - 1), "message": {
                "id": "m%d" % n, "content": [{"type": "tool_use", "id": use_id, "name": tool, "input": tool_input}]}})
            rows.append({"type": "user", "timestamp": stamp(sec), "message": {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": use_id, "is_error": is_error}]}})
        (vrun / "transcripts").mkdir()
        (vrun / "transcripts" / "v1.jsonl").write_text("".join(json.dumps(r) + "\n" for r in rows))
        got = (violations.per_worker(vrun) or {}).get("per_worker", {}).get("v1", {})
        want = {"tool_edits": 4, "bash_writes": 1, "unclaimed": 3, "anchor_only": 1,
                "unclaimed_paths": {"tally/a.py": 1, "tally/b.py": 1, "tally/c.py": 1}}
        check("violations: unclaimed writes per worker", got == want, got)
    # Turn classes.
    import turns
    fixture = run / "fake_transcripts" / "selfcheck-fixture.jsonl"
    rows = [("Bash", {"command": "TB_RUN=/r TB_AGENT=w1 /r/tb claim '{\"paths\":[\"a\"]}'"}),
            ("Bash", {"command": "python3 -c \"import time; time.sleep(30)\"; TB_RUN=/r TB_AGENT=w1 /r/tb task_pull '{}'"}),
            ("Bash", {"command": "TB_RUN=/r TB_AGENT=w1 /r/tb message_send '{\"to\":\"w2\",\"text\":\"hi\"}'"}),
            ("Bash", {"command": "cd /r/repo && python3 -m unittest && TB_RUN=/r TB_AGENT=w1 /r/tb release '{}'"}),
            ("Edit", {"file_path": "/r/repo/a.py"}), (None, None)]
    with open(str(fixture), "w") as handle:
        for i, (tool, tool_input) in enumerate(rows):
            content = [{"type": "tool_use", "name": tool, "input": tool_input}] if tool else [{"type": "text", "text": "x"}]
            handle.write(json.dumps({"type": "assistant", "message": {"id": "m%d" % i, "content": content,
                                                                      "usage": {"output_tokens": 10}}}) + "\n")
    kinds = [turns.classify(t)[0] for t in turns.load_turns(fixture)]
    check("turn classes", kinds == ["mechanical", "mechanical", "coordination", "work", "work", "text"], kinds)
    fixture.unlink()
    print("selfcheck: %d checks, %d failures" % (len(checks), len(failures)))
    return 1 if failures else 0


def main():
    if len(sys.argv) < 3:
        raise SystemExit("usage: fake_worker_hooks.py <run_dir> selfcheck | <agent> [--auto-identity]")
    run = Path(sys.argv[1]).resolve()
    if sys.argv[2] == "selfcheck":
        sys.exit(selfcheck(run))
    worker(run, sys.argv[2], "--auto-identity" in sys.argv[3:])


if __name__ == "__main__":
    main()
