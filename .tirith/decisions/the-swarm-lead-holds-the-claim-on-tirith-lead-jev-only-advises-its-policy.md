---
id: ccb207b6-1d17-4972-b527-7bfd502c4cd2
permalink: the-swarm-lead-holds-the-claim-on-tirith-lead-jev-only-advises-its-policy
title: The swarm lead holds the claim on .tirith/lead; Jev only advises its policy
kind: decision
tags: []
paths:
- src/lead.rs
- src/assist.rs
- src/server.rs
- src/state.rs
- .tirith/lead
- AGENTS.md
- docs/6-agent-workflow/03-tirith-dogfooding.md
author: jev-lead
updated_by: jev-lead
created_at: 2026-09-17T03:50:38Z
updated_at: 2026-09-17T03:50:38Z
---

The session that spawns other agents claims `.tirith/lead` (ttl 3600) and is the lead agent; workers never claim it. Routine lead decisions run as deterministic policy in the daemon (src/lead.rs), which asks Jev bounded questions, logs every decision to .tirith/runtime/lead_log.jsonl, and routes escalations (records / lead / human). Jev never acts, never suppresses information, and its metadata is hidden from worker responses.

## Rationale

Jev lab 2026-09-17: Jev helped only when the server acted on its answer without a worker turn; exposing Jev fields and suppressing information hurt. See ADR-0026 and ADR-0027.
