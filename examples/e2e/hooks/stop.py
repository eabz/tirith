#!/usr/bin/env python3
"""Stop / SubagentStop hook: finish the task, feed the next one, or stop for real.

1. Release every claim the agent holds (the edit window is over).
2. With a current task, ask the stop gate (stop_gate.py):
   done -> task_update done; continue -> block the stop with the gate's
   reason; escalate -> task_update blocked and stop for real.
3. Without a task, task_pull. A task -> block the stop with its text, so
   Claude continues on it. None -> if todo tasks remain (waiting on
   dependencies) poll inside the hook, within a per-agent idle budget; if
   none remain, ask once for the final report, then allow the stop.

SubagentStop acts only for a subagent explicitly mapped to its own Tirith
agent in identity.json; a helper subagent of a worker never finishes or
pulls the worker's tasks. Loop guards: the gate's continue limit, the idle
budget, one final-report block, and at most MAX_IDLE_BLOCKS consecutive
blocks with no tool call in between (Claude Code itself ends the turn after
8 consecutive blocks, see README).
"""

import time

import common as c
import stop_gate

MAX_IDLE_BLOCKS = 6


def release_all(run, agent):
    with c.agent_state(run, agent) as state:
        held = sorted(p for p, e in state["files"].items() if e.get("held"))
        state["files"] = {}
    if not held:
        return [], []
    with c.coord_lock(run, agent):
        response = c.tb(run, agent, "release", {})
    return held, c.side_notes(run, agent, response)


def block(run, event, agent, kind, reason, **detail):
    with c.agent_state(run, agent) as state:
        last = state["stop"].get("last_block") or {}
        idle = last.get("idle_blocks", 0) + 1 if last.get("tool_calls") == state["tool_calls"] else 0
        if idle >= MAX_IDLE_BLOCKS:
            state["stop"]["last_block"] = None
            guard = True
        else:
            state["stop"]["last_block"] = {"kind": kind, "tool_calls": state["tool_calls"], "idle_blocks": idle,
                                           "at": c.now_iso()}
            guard = False
    if guard:
        c.log(run, event, agent, "allow-stop", why="loop-guard", kind=kind, **detail)
        return
    c.block_stop(reason)
    c.log(run, event, agent, "block-" + kind, **detail)


def main():
    event = c.read_event()
    run = c.run_dir()
    agent, source = c.resolve_identity(run, event)
    if event.get("hook_event_name") == "SubagentStop" and source != "agent_map":
        c.log(run, event, agent, "noop", why="helper subagent", identity=source)
        return
    cfg = c.config(run)
    released, notes = release_all(run, agent)
    if released:
        c.log(run, event, agent, "release-at-stop", paths=released)

    with c.agent_state(run, agent) as state:
        task = state.get("task")
        continues = state["gate"]["continues"] if state["gate"].get("task_id") == (task or {}).get("id") else 0
        final_requested = state.get("final_report_requested")

    if task:
        gate = stop_gate.load_gate(cfg["stop_gate"])
        started = time.time()
        verdict = gate.evaluate(stop_gate.GateInput(task=task, last_message=event.get("last_assistant_message") or "",
                                                    repo=c.repo_root(run), agent=agent, config=cfg,
                                                    continues=continues))
        gate_ms = round((time.time() - started) * 1000, 1)
        detail = dict(task=str(task.get("id"))[:8], gate=getattr(gate, "name", "?"), ms=gate_ms, **verdict.evidence)
        if verdict.decision == "continue":
            with c.agent_state(run, agent) as state:
                state["gate"] = {"task_id": task.get("id"), "continues": continues + 1}
            block(run, event, agent, "gate", "Task %s is not finished yet (stop gate).\n\n%s%s"
                  % (str(task.get("id"))[:8], verdict.reason, "\n\n" + "\n".join(notes) if notes else ""), **detail)
            return
        status = "done" if verdict.decision == "done" else "blocked"
        summary = (event.get("last_assistant_message") or "").strip().splitlines()
        note = verdict.reason if status == "blocked" else "%s; %s" % (verdict.reason, (summary[0] if summary else "")[:160])
        response = c.tb(run, agent, "task_update", {"task_id": task.get("id"), "status": status, "note": note[:300]})
        notes += c.side_notes(run, agent, response)
        with c.agent_state(run, agent) as state:
            state["task"] = None
            state["gate"] = {"task_id": None, "continues": 0}
            state["done" if status == "done" else "blocked"].append(str(task.get("id")))
        c.log(run, event, agent, "task-" + status, update_status=response.get("status"), **detail)
        if status == "blocked":
            c.log(run, event, agent, "allow-stop", why="escalated")
            return

    waited_here = 0
    while True:
        pulled = c.tb(run, agent, "task_pull", {})
        notes += c.side_notes(run, agent, pulled)
        if pulled.get("status") == "ok" and isinstance(pulled.get("task"), dict):
            with c.agent_state(run, agent) as state:
                state["task"] = pulled["task"]
                state["gate"] = {"task_id": pulled["task"].get("id"), "continues": 0}
            block(run, event, agent, "task", c.task_text(pulled["task"], notes),
                  task=str(pulled["task"].get("id"))[:8], waited_s=waited_here)
            return
        if pulled.get("status") != "none":
            c.log(run, event, agent, "allow-stop", why="task_pull failed", status=pulled.get("status"),
                  message=str(pulled.get("message", ""))[:200])
            return
        todo = c.tb(run, agent, "task_list", {"status": "todo", "limit": 1})
        if todo.get("total", 0) == 0:
            if final_requested or not cfg.get("final_report"):
                c.log(run, event, agent, "allow-stop", why="no work left")
                return
            with c.agent_state(run, agent) as state:
                state["final_report_requested"] = True
            block(run, event, agent, "final", cfg["final_report"] + ("\n\n" + "\n".join(notes) if notes else ""))
            return
        with c.agent_state(run, agent) as state:
            idle_used = state["idle_waited_secs"]
        if idle_used >= cfg["idle_budget_secs"]:
            c.log(run, event, agent, "allow-stop", why="idle budget spent", todo=todo.get("total"))
            return
        if waited_here >= cfg["poll_slice_secs"]:
            block(run, event, agent, "wait",
                  "No task is free yet: %s tasks wait on dependencies other agents are finishing. Nothing needs "
                  "doing now; end your turn and the next free task is delivered then.%s"
                  % (todo.get("total"), "\n\n" + "\n".join(notes) if notes else ""), todo=todo.get("total"))
            return
        time.sleep(cfg["poll_secs"])
        waited_here += cfg["poll_secs"]
        with c.agent_state(run, agent) as state:
            state["idle_waited_secs"] += cfg["poll_secs"]


if __name__ == "__main__":
    c.guarded(main)
