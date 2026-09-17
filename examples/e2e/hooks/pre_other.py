#!/usr/bin/env python3
"""PreToolUse hook for every tool: release files whose edit window closed.

Edit tools return at once (pre_edit.py handles them). For any other tool,
files marked pending by post_edit.py with no edit in flight are released in
one call. If this agent's claim or release is already running in a parallel
hook, the release is skipped and retried on the next call or at Stop.
Never blocks the tool.
"""

import calendar
import time

import common as c

STALE_INFLIGHT_SECS = 900  # an edit denied later by permissions never reaches PostToolUse


def main():
    event = c.read_event()
    if event.get("tool_name") in c.EDIT_TOOLS:
        return
    run = c.run_dir()
    agent, source = c.resolve_identity(run, event)
    with c.coord_lock(run, agent, wait=False) as locked:
        with c.agent_state(run, agent) as state:
            state["tool_calls"] += 1
            if not locked:
                releasable = []
            else:
                releasable = sorted(p for p, e in state["files"].items()
                                    if e.get("held") and (e.get("pending") or _stale(e)) and
                                    (e.get("inflight", 0) == 0 or _stale(e)))
                for path in releasable:
                    del state["files"][path]
        if not releasable:
            if not locked:
                c.log(run, event, agent, "release-skipped-busy", identity=source)
            return
        response = c.tb(run, agent, "release", {"paths": releasable})
    notes = c.side_notes(run, agent, response)
    if notes:
        c.context("PreToolUse", "\n\n".join(notes))
    c.log(run, event, agent, "release", paths=releasable, status=response.get("status"), ms=response.get("_ms"),
          identity=source)


def _stale(entry):
    since = entry.get("since") or ""
    try:
        started = calendar.timegm(time.strptime(since[:19], "%Y-%m-%dT%H:%M:%S"))
    except ValueError:
        return False
    return time.time() - started > STALE_INFLIGHT_SECS


if __name__ == "__main__":
    c.guarded(main)
