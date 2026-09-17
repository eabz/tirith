#!/usr/bin/env python3
"""UserPromptSubmit hook: give an agent without a task its first task as context.

Saves the turn in which a worker would otherwise end at once only for the
Stop hook to feed it a task. When nothing is free, it adds nothing and the
Stop hook waits for work instead.
"""

import common as c


def main():
    event = c.read_event()
    run = c.run_dir()
    agent, source = c.resolve_identity(run, event)
    if event.get("agent_id") or not c.config(run).get("autostart"):
        return
    with c.agent_state(run, agent) as state:
        if state.get("task"):
            return
    pulled = c.tb(run, agent, "task_pull", {})
    notes = c.side_notes(run, agent, pulled)
    if pulled.get("status") == "ok" and isinstance(pulled.get("task"), dict):
        with c.agent_state(run, agent) as state:
            state["task"] = pulled["task"]
            state["gate"] = {"task_id": pulled["task"].get("id"), "continues": 0}
        c.context("UserPromptSubmit", c.task_text(pulled["task"], notes))
        c.log(run, event, agent, "task-at-start", task=str(pulled["task"].get("id"))[:8], identity=source)
        return
    if notes:
        c.context("UserPromptSubmit", "\n\n".join(notes))
    c.log(run, event, agent, "no-task-at-start", status=pulled.get("status"), identity=source)


if __name__ == "__main__":
    c.guarded(main)
