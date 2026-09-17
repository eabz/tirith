"""Shared plumbing for the e2e kit coordination hooks (Python 3.9, stdlib only).

setup.sh --protocol hooks copies this directory to <run_dir>/hooks and writes
<run_dir>/repo/.claude/settings.json pointing at the copies, so a hook finds
its run as the parent of its own directory. State lives in
<run_dir>/hooks_state/:

  config.json            knobs written by setup (defaults below)
  identity.json          {"sessions": {session_id: agent}, "agents": {agent_id: agent}}
  sessions/<id>.json     one per Claude Code session: agent, transcript paths
  agents/<agent>.json    per Tirith agent: current task, files held, counters
  events.jsonl           one line per hook decision (the plumbing log)

Every Tirith call goes through the run's `tb` with TB_ORIGIN=hook, so
coord.jsonl keeps counting calls and marks the ones hooks made.
See README.md in this directory for the hook contract this relies on.
"""

import contextlib
import fcntl
import json
import os
import subprocess
import sys
import time
from pathlib import Path

HOOKS_DIR = Path(__file__).resolve().parent

# Built-in file tools whose tool_input names one file.
FILE_TOOLS = {"Edit": "file_path", "Write": "file_path", "MultiEdit": "file_path", "NotebookEdit": "notebook_path"}
# Serena edit tools (relative_path is relative to the Serena project, assumed to be the repo root).
SERENA_EDIT_TOOLS = {"mcp__serena__replace_symbol_body", "mcp__serena__insert_after_symbol",
                     "mcp__serena__insert_before_symbol", "mcp__serena__replace_content",
                     "mcp__serena__rename_symbol", "mcp__serena__safe_delete_symbol"}
EDIT_TOOLS = set(FILE_TOOLS) | SERENA_EDIT_TOOLS

DEFAULTS = {
    "claim_wait_secs": 120,      # claim wait_secs; the PreToolUse timeout must exceed it
    "claim_ttl_secs": 1800,
    "stop_gate": "placeholder",
    "gate_max_continues": 3,     # gate "continue" verdicts per task before escalating
    "test_command": ["python3", "-m", "unittest"],
    "test_timeout_secs": 300,
    "poll_secs": 20,             # task_pull retry interval while tasks wait on dependencies
    "poll_slice_secs": 480,      # longest wait inside one Stop hook (its timeout is 600)
    "idle_budget_secs": 900,     # total waiting for dependencies per agent
    "autostart": True,           # UserPromptSubmit pulls the first task
    "final_report": ("No tasks remain on the board. Write your final report now: the tasks you completed "
                     "(id and title), the files you changed, the final `python3 -m unittest` result, the "
                     "notices and messages you sent, edits that were refused and how you resolved them, and "
                     "anything left unfinished."),
}


class HookError(Exception):
    """A hook cannot do its job (no run directory, unreadable input)."""


def now_iso():
    t = time.time()
    return time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime(t)) + ".%03dZ" % int((t - int(t)) * 1000)


def read_event():
    try:
        return json.load(sys.stdin)
    except (json.JSONDecodeError, ValueError) as err:
        raise HookError("hook input is not JSON: %s" % err)


def run_dir():
    run = HOOKS_DIR.parent
    if os.environ.get("TB_RUN"):
        run = Path(os.environ["TB_RUN"]).resolve()
    if not (run / "url").is_file() or not (run / "tb").is_file():
        raise HookError("no e2e run directory at %s (expected url and tb)" % run)
    return run


def state_dir(run):
    path = run / "hooks_state"
    for sub in ("sessions", "agents"):
        (path / sub).mkdir(parents=True, exist_ok=True)
    return path


def config(run):
    merged = dict(DEFAULTS)
    path = state_dir(run) / "config.json"
    if path.is_file():
        merged.update(json.loads(path.read_text()))
    return merged


@contextlib.contextmanager
def flock(path):
    path.parent.mkdir(parents=True, exist_ok=True)
    with open(str(path), "a") as handle:
        fcntl.flock(handle, fcntl.LOCK_EX)
        try:
            yield
        finally:
            fcntl.flock(handle, fcntl.LOCK_UN)


@contextlib.contextmanager
def try_flock(path):
    """Yields True with the lock held, or False at once when someone else holds it."""
    path.parent.mkdir(parents=True, exist_ok=True)
    with open(str(path), "a") as handle:
        try:
            fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError:
            yield False
            return
        try:
            yield True
        finally:
            fcntl.flock(handle, fcntl.LOCK_UN)


def write_json(path, value):
    tmp = path.with_name(path.name + ".tmp%d" % os.getpid())
    tmp.write_text(json.dumps(value, indent=1, sort_keys=True))
    os.replace(str(tmp), str(path))


def read_json(path, default):
    try:
        return json.loads(path.read_text())
    except (OSError, ValueError):
        return default


def log(run, event, agent, action, **detail):
    line = json.dumps(dict({"ts": now_iso(), "event": event.get("hook_event_name"), "agent": agent,
                            "session": event.get("session_id"), "subagent": event.get("agent_id"),
                            "tool": event.get("tool_name"), "action": action}, **detail))
    path = state_dir(run) / "events.jsonl"
    with flock(path.with_suffix(".lock")):
        with open(str(path), "a") as handle:
            handle.write(line + "\n")


# ---------------------------------------------------------------- identity

def resolve_identity(run, event):
    """(agent name, source) for a hook call.

    Order: an explicit agent_id mapping in identity.json (a subagent that is
    itself a worker), TB_AGENT inherited from the environment the harness
    launched Claude Code with, an explicit session mapping, then an
    auto-assigned `hk-<session prefix>` that is recorded for later calls.
    Tool calls inside a helper subagent resolve to the parent's name, so they
    share its claims. The session registry records transcript paths for the
    scorer.
    """
    sdir = state_dir(run)
    session = str(event.get("session_id") or "no-session")
    agent_id = event.get("agent_id")
    with flock(sdir / "identity.lock"):
        ids = read_json(sdir / "identity.json", {})
        ids.setdefault("sessions", {})
        ids.setdefault("agents", {})
        changed = False
        if agent_id and agent_id in ids["agents"]:
            name, source = ids["agents"][agent_id], "agent_map"
        elif os.environ.get("TB_AGENT"):
            name, source = os.environ["TB_AGENT"], "env"
            if session not in ids["sessions"]:
                ids["sessions"][session] = name
                changed = True
        elif session in ids["sessions"]:
            name, source = ids["sessions"][session], "session_map"
        else:
            name, source = "hk-" + "".join(c for c in session if c.isalnum())[:8], "auto"
            ids["sessions"][session] = name
            changed = True
        if changed:
            write_json(sdir / "identity.json", ids)
        reg_path = sdir / "sessions" / ("%s.json" % "".join(c for c in session if c.isalnum() or c in "-_"))
        reg = read_json(reg_path, {"session_id": session, "agent": name, "subagents": {}})
        before = json.dumps(reg, sort_keys=True)
        if event.get("transcript_path"):
            reg["transcript_path"] = event["transcript_path"]
        if agent_id:
            sub = reg["subagents"].setdefault(agent_id, {"agent_type": event.get("agent_type")})
            sub["resolved_agent"] = name
            if event.get("agent_transcript_path"):
                sub["agent_transcript_path"] = event["agent_transcript_path"]
        if json.dumps(reg, sort_keys=True) != before:
            write_json(reg_path, reg)
    return name, source


# ------------------------------------------------------------------- state

def agent_paths(run, agent):
    safe = "".join(c for c in agent if c.isalnum() or c in "-_.")
    base = state_dir(run) / "agents"
    return base / (safe + ".json"), base / (safe + ".lock"), base / (safe + ".coord.lock")


def empty_state(agent):
    return {"agent": agent, "task": None, "files": {}, "tool_calls": 0, "idle_waited_secs": 0,
            "gate": {"task_id": None, "continues": 0}, "stop": {"last_block": None},
            "final_report_requested": False, "done": [], "blocked": []}


@contextlib.contextmanager
def agent_state(run, agent):
    """Load, yield and save the agent's state under its lock. Never call Tirith inside."""
    path, lock, _ = agent_paths(run, agent)
    with flock(lock):
        state = read_json(path, None) or empty_state(agent)
        yield state
        write_json(path, state)


def coord_lock(run, agent, wait=True):
    """Serializes this agent's claim and release calls across parallel hooks."""
    _, _, lock = agent_paths(run, agent)
    return flock(lock) if wait else try_flock(lock)


# ------------------------------------------------------------------ Tirith

def tb(run, agent, tool, args, timeout=240):
    env = dict(os.environ, TB_RUN=str(run), TB_AGENT=agent, TB_ORIGIN="hook")
    started = time.time()
    try:
        proc = subprocess.run([str(run / "tb"), tool, json.dumps(args)], capture_output=True, text=True,
                              env=env, timeout=timeout)
        lines = proc.stdout.strip().splitlines()
        response = json.loads(lines[-1]) if lines else {"status": "error", "message": proc.stderr[:300]}
    except (subprocess.TimeoutExpired, ValueError, IndexError, OSError) as err:
        response = {"status": "error", "message": "tb failed: %s" % err}
    response["_ms"] = round((time.time() - started) * 1000, 1)
    return response


def side_notes(run, agent, response):
    """Text for inbox messages and lost leases in a response; drops lost files from state."""
    notes = []
    for message in response.get("inbox") or []:
        notes.append("Message for %s from %s: %s" % (agent, message.get("from", "?"), message.get("text", "")))
    if response.get("inbox_more"):
        notes.append("%s more messages are waiting: `tb message_list '{\"unread\":true}'`." % response["inbox_more"])
    lost = response.get("lost") or []
    if lost:
        paths = [row.get("path", row) if isinstance(row, dict) else row for row in lost]
        notes.append("Tirith reports these claims ended (lease lost): %s. Edits to them claim again automatically."
                     % ", ".join(str(p) for p in paths))
        with agent_state(run, agent) as state:
            for path in paths:
                state["files"].pop(str(path), None)
    return notes


def task_text(task, notes=()):
    lines = ["Tirith assigned you task %s: %s" % (str(task.get("id", ""))[:8], task.get("title", "")),
             "Paths (a hint, not an order): %s" % (", ".join(task.get("paths") or []) or "none"), "",
             (task.get("description") or "").strip(), "",
             "When the task is complete and `python3 -m unittest` passes, end your turn with a short summary "
             "of how each acceptance criterion is met."]
    return "\n".join(list(lines) + ([""] + list(notes) if notes else []))


# ------------------------------------------------------------------- paths

def repo_root(run):
    return (run / "repo").resolve()


def relative(run, raw):
    """Repo-relative path for an absolute or repo-relative path, or None outside the repo."""
    if not raw:
        return None
    root = repo_root(run)
    path = Path(str(raw).replace("\\", "/"))
    if not path.is_absolute():
        path = root / path
    try:
        rel = path.resolve().relative_to(root)
    except ValueError:
        return None
    text = rel.as_posix()
    if text in ("", ".") or text.startswith(".git/") or text.startswith(".tirith/"):
        return None
    return text


def target_paths(run, event):
    tool = event.get("tool_name") or ""
    tool_input = event.get("tool_input") or {}
    if tool in FILE_TOOLS:
        raw = [tool_input.get(FILE_TOOLS[tool])]
    elif tool in SERENA_EDIT_TOOLS:
        raw = [tool_input.get("relative_path")]
    else:
        raw = []
    return sorted({p for p in (relative(run, r) for r in raw) if p})


# ------------------------------------------------------------------ output

def emit(value):
    sys.stdout.write(json.dumps(value))
    sys.stdout.flush()


def context(event_name, text):
    emit({"hookSpecificOutput": {"hookEventName": event_name, "additionalContext": text[:9500]}})


def deny(reason, extra=None):
    out = {"hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": reason[:9500]}
    if extra:
        out["additionalContext"] = extra[:9500]
    emit({"hookSpecificOutput": out})


def block_stop(reason):
    emit({"decision": "block", "reason": reason[:9500]})


def guarded(main, fail_closed_pre_tool=False):
    """Run a hook main; errors are logged, and a PreToolUse edit hook denies (fails closed)."""
    try:
        main()
    except Exception as err:  # noqa: BLE001 - a hook must always answer
        try:
            run = run_dir()
            with open(str(state_dir(run) / "errors.log"), "a") as handle:
                handle.write("%s %s %r\n" % (now_iso(), Path(sys.argv[0]).name, err))
        except Exception:  # noqa: BLE001
            pass
        if fail_closed_pre_tool:
            deny("The coordination hook failed (%s), so this edit was not applied. Try again shortly." % err)
        sys.exit(0)
