---
id: ccb207b6-1d17-4972-b527-7bfd502c4cd2
permalink: the-swarm-lead-holds-the-claim-on-tirith-lead
title: The swarm lead holds the claim on .tirith/lead
kind: decision
tags: []
paths:
- src/lead.rs
- src/server.rs
- src/state.rs
- .tirith/lead
- AGENTS.md
- docs/6-agent-workflow/03-tirith-dogfooding.md
author: swarm-lead
updated_by: swarm-lead
created_at: 2026-09-17T03:50:38Z
updated_at: 2026-09-17T05:30:00Z
---

The session that spawns other agents claims `.tirith/lead` (ttl 3600) and is the lead agent; workers never claim it. Routine lead decisions run as deterministic policy in the daemon (src/lead.rs), which logs to .tirith/runtime/lead_log.jsonl and routes escalations to the lead's inbox, or to the human queue for credentials, permissions, spending and destructive operations, or when there is no lead.

## Rationale

Across 17 sessions agents spent 19-22% of wall time waiting on the human; an explicit lead with an inbox, plus a ranked human queue, keeps routine escalations away from the human without an LLM turn per event. See ADR-0027.
