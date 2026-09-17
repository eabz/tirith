#!/usr/bin/env python3
"""PostToolUse / PostToolUseFailure hook for edit tools (and Bash on PostToolUse).

Edit tools: the edit is over, so the file is marked pending release; the
release itself happens on the next non-edit tool call (pre_other.py) or at
Stop, which keeps a burst of edits to one file inside one claim.

Bash: when Claude Code reports the files a command changed
(tool_response.bashEditDiff.changedFiles, see README), files not already
held are claimed after the fact without waiting and marked pending. A
refused claim cannot undo the write, so Claude gets a note naming the holder.
"""

import common as c


def bash_changed(run, event):
    response = event.get("tool_response") or {}
    diff = response.get("bashEditDiff") if isinstance(response, dict) else None
    if not isinstance(diff, dict):
        return []
    return sorted({p for p in (c.relative(run, raw) for raw in diff.get("changedFiles") or [])
                   if p and "__pycache__" not in p})


def main():
    event = c.read_event()
    run = c.run_dir()
    agent, source = c.resolve_identity(run, event)
    name = event.get("hook_event_name")
    if event.get("tool_name") == "Bash":
        if name != "PostToolUse":
            return
        changed = bash_changed(run, event)
        with c.agent_state(run, agent) as state:
            unheld = [p for p in changed if not state["files"].get(p, {}).get("held")]
        if not unheld:
            return
        cfg = c.config(run)
        with c.coord_lock(run, agent):
            response = c.tb(run, agent, "claim", {"paths": unheld, "reason": "files changed by a shell command",
                                                  "ttl_secs": cfg["claim_ttl_secs"]})
            ok = response.get("status") == "ok"
            if ok:
                with c.agent_state(run, agent) as state:
                    for path in unheld:
                        state["files"][path] = {"held": True, "pending": True, "inflight": 0, "since": c.now_iso()}
        notes = c.side_notes(run, agent, response)
        if not ok:
            notes.insert(0, "The shell command changed %s while another agent holds a claim on it (%s). That "
                            "agent may overwrite or be overwritten by this change; coordinate with it through "
                            "`tb message_send`." % (", ".join(unheld), response.get("status")))
        if notes:
            c.context("PostToolUse", "\n\n".join(notes))
        c.log(run, event, agent, "bash-claim" if ok else "bash-unclaimed-write", paths=unheld, identity=source)
        return
    paths = c.target_paths(run, event)
    if not paths:
        return
    with c.agent_state(run, agent) as state:
        for path in paths:
            entry = state["files"].get(path)
            if entry is None:
                continue
            entry["inflight"] = max(0, entry["inflight"] - 1)
            if entry["held"] and entry["inflight"] == 0:
                entry["pending"] = True
            if not entry["held"] and entry["inflight"] == 0:
                del state["files"][path]
    c.log(run, event, agent, "mark-release" if name == "PostToolUse" else "mark-release-after-failure",
          paths=paths, identity=source)


if __name__ == "__main__":
    c.guarded(main)
