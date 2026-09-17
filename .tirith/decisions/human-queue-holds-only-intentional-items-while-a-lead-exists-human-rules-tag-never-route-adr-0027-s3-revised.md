---
id: 56a2f6be-64d4-4811-9b6d-f5665581755c
permalink: human-queue-holds-only-intentional-items-while-a-lead-exists-human-rules-tag-never-route-adr-0027-s3-revised
title: Human queue holds only intentional items while a lead exists; human rules tag, never route (ADR-0027 s3 revised)
kind: decision
tags: []
paths:
- src/lead.rs
- src/server.rs
- src/messages.rs
- src/dashboard.rs
- src/dashboard.html
- src/cli.rs
- src/tray.rs
- docs/5-decisions/0027-swarm-lead-and-escalation.md
- AGENTS.md
- docs/6-agent-workflow/03-tirith-dogfooding.md
author: queue-fix
updated_by: queue-fix
created_at: 2026-09-17T15:34:42Z
updated_at: 2026-09-17T15:34:42Z
---

With a live lead, blocked tasks and repeated claim refusals go to the lead's inbox (rule live_lead), tagged "may need the human: <rule>" when a human rule matches; the lead's own escalations are logged only (lead_itself); messages to the lead never escalate. The only way into the human queue while a lead exists is message_send to "human" (rule to_human, text = the message). With no lead, escalations go to the queue (no_lead). "human" is a reserved name (no caller, no broadcast, no inbox). Items are answered by POST /api/human/{id}/done (dashboard Done button, `tirith lead human done <id> [--reply]`); a reply reaches the sender as a message from human with reply_to. Messages to human are answered only by the human; no-lead items keep the automatic answer tracking.

## Rationale

The tray said "2 need you" with nothing to read: both items were reports to the lead routed by phrase rules, one a false positive ("grant/assign nothing" matched "grant"). Phrase rules match ordinary dev vocabulary and a routed item carried no text written for the human.

## Alternatives

- Tighten the phrase lists (still guesses, still no text for the human)
- A new escalate_human tool or parameter (tools/list budget; message_send already carries text, paths, reply_to)
- Classify with a model (latency, cost, external dependency)
