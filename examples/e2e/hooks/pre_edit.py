#!/usr/bin/env python3
"""PreToolUse hook for edit tools: claim the target file through Tirith first.

Files this agent already holds are reused (their pending release is
cancelled by counting the edit in flight). Anything else is claimed with
wait_secs, so the server waits out a short overlap. On refusal the edit is
denied with the holder named in the reason (shown to Claude). A granted
claim whose brief has rows, and any inbox message or lost lease, is added
as context. Fails closed: if this hook errors, the edit is denied.
"""

import json
import time

import common as c


def brief_rows(response):
    sections = {k: response[k] for k in ("notices", "contracts", "decisions", "memory") if response.get(k)}
    return sections


def main():
    event = c.read_event()
    run = c.run_dir()
    agent, source = c.resolve_identity(run, event)
    paths = c.target_paths(run, event)
    with c.agent_state(run, agent) as state:
        state["tool_calls"] += 1
    if not paths:
        c.log(run, event, agent, "skip", reason="no repo path", identity=source)
        return
    cfg = c.config(run)
    started = time.time()
    with c.coord_lock(run, agent):
        with c.agent_state(run, agent) as state:
            files = state["files"]
            need = []
            for path in paths:
                entry = files.setdefault(path, {"held": False, "pending": False, "inflight": 0})
                entry["inflight"] += 1
                entry["pending"] = False
                entry["since"] = c.now_iso()
                if not entry["held"]:
                    need.append(path)
            task = state.get("task") or {}
        if not need:
            c.log(run, event, agent, "reuse", paths=paths, identity=source)
            return
        args = {"paths": need, "reason": task.get("title") or "edit through hooks", "ttl_secs": cfg["claim_ttl_secs"]}
        if cfg["claim_wait_secs"]:
            args["wait_secs"] = cfg["claim_wait_secs"]
        response = c.tb(run, agent, "claim", args, timeout=cfg["claim_wait_secs"] + 60)
        granted = response.get("status") == "ok"
        with c.agent_state(run, agent) as state:
            for path in need:
                entry = state["files"].get(path)
                if entry is None:
                    continue
                if granted:
                    entry["held"] = True
                else:
                    entry["inflight"] = max(0, entry["inflight"] - 1)
                    if not entry["held"] and entry["inflight"] == 0:
                        del state["files"][path]
    notes = c.side_notes(run, agent, response)
    waited_ms = round((time.time() - started) * 1000, 1)
    if not granted:
        holders = "; ".join("%s held by %s (%s)" % (row.get("path"), row.get("owner") or row.get("agent"),
                                                    row.get("reason", ""))
                            for row in response.get("conflicts") or [] if isinstance(row, dict))
        reason = ("Tirith did not grant the claim on %s after waiting %ss (status %s%s). The edit was not applied. "
                  "Another agent is editing this file: work on another part of the task and retry this edit later."
                  % (", ".join(need), cfg["claim_wait_secs"], response.get("status"),
                     ": " + holders if holders else ": " + str(response.get("message", ""))[:200]))
        c.deny(reason, "\n".join(notes) if notes else None)
        c.log(run, event, agent, "deny", paths=need, status=response.get("status"), ms=waited_ms, identity=source)
        return
    rows = brief_rows(response)
    parts = []
    if rows:
        rows["more"] = response.get("more")
        parts.append("Coordination brief from Tirith for %s (unread notices, contracts, decisions and memory "
                     "notes about these paths): %s" % (", ".join(need), json.dumps(rows, separators=(",", ":"))))
    parts.extend(notes)
    if parts:
        c.context("PreToolUse", "\n\n".join(parts))
    c.log(run, event, agent, "claim", paths=need, ms=waited_ms, brief_rows=sum(len(v) for k, v in rows.items()
                                                                                if k != "more"),
          notes=len(notes), identity=source)


if __name__ == "__main__":
    c.guarded(main, fail_closed_pre_tool=True)
