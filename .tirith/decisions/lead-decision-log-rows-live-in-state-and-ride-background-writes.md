---
id: 909b15f3-892a-45e2-9191-c85c0b9d06b5
permalink: lead-decision-log-rows-live-in-state-and-ride-background-writes
title: "Lead decision log: rows live in State and ride background writes"
kind: decision
tags: []
paths:
- src/lead.rs
- src/state.rs
- src/store.rs
- src/server.rs
author: lead-builder
updated_by: swarm-lead
created_at: 2026-09-17T04:24:48Z
updated_at: 2026-09-17T05:30:00Z
---

The lead log (.tirith/runtime/lead_log.jsonl) is a runtime log owned by State (Snapshot/Delta.lead_log, LogCursor). A new row does not make State::is_dirty() true, like lease renewals: it is written on the next persister tick or with any other write, so no tool call waits for it. Rows cover claim lifecycle (granted, refused, waited, released, lease_ended), notice pushes and escalations, written from inside State under its lock. Outcomes rewrite the file. The lead is the holder of a claim naming exactly .tirith/lead; an ancestor claim does not count.

## Rationale

ADR-0027 section 4 requires the log never be on the request path, and the swarm lead needs claim auditing after release. Exact-path leadership keeps leadership an explicit act (ADR-0027 rejects implicit leads).

## Alternatives

- A separate log file writer owned by LeadPolicy (rejected: duplicates the persister's durability and failure handling)
- Append-only outcome rows instead of in-place outcome (deferred: outcomes are rare)
