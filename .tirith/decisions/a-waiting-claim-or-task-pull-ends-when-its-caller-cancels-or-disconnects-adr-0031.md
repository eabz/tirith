---
id: 012eb2bd-304c-4ce2-a973-1ac99f5ce05e
permalink: a-waiting-claim-or-task-pull-ends-when-its-caller-cancels-or-disconnects-adr-0031
title: A waiting claim or task_pull ends when its caller cancels or disconnects (ADR-0031)
kind: decision
tags: []
paths:
- src/hangup.rs
- src/server.rs
- src/state.rs
- src/stdio.rs
- docs/5-decisions/0031-waits-end-when-the-caller-goes.md
author: shim-cancel
updated_by: shim-cancel
created_at: 2026-09-17T15:00:30Z
updated_at: 2026-09-17T15:00:30Z
---

The daemon ties a Hangup signal to each /mcp response body (src/hangup.rs) and combines it with the request's cancellation token; claim and task_pull with wait_secs stop waiting on either, granting nothing (status cancelled, claim_waited outcome cancelled) and skipping finish() piggybacks. tirith stdio forwards client cancels and cancels in-flight calls on stdin EOF, then closes its daemon session.

## Rationale

rmcp 3.4 reports an MCP cancel at once and a closed session after a 5 s drain, but nothing for a dropped connection in session mode; hyper does drop the response body on disconnect, so the body is the signal. No protocol change or heartbeat needed.

## Alternatives

- Heartbeat pings during a wait: needs a client protocol extension
- Stateless HTTP mode: turns sessions off for every client
- Undo a grant after the fact: others may already have read it
- Fix only the shim: killed agents and HTTP clients stay uncovered
