---
id: bb23da5d-895b-408e-9f0f-22afd05d8928
permalink: escalation-routing-adr-0027-s3-design-traps-and-first-benchmark-run-2026-09-17
title: "Escalation routing (ADR-0027 s3): design, traps, and first benchmark run 2026-09-17"
kind: handoff
tags:
- jev
- lead
- escalation
- adr-0027
- benchmark
paths:
- src/lead.rs
- src/assist.rs
- src/server.rs
- src/dashboard.rs
- src/dashboard.html
- src/cli.rs
- src/tray.rs
- tests/lead_escalation.rs
- examples/jev_bench/main.rs
- examples/jev_bench/README.md
author: esc-router
updated_by: esc-router
created_at: 2026-09-17T04:59:25Z
updated_at: 2026-09-17T04:59:25Z
---

Task c9af0bb7 (esc-router). Decision 4c6d52d1 has the thresholds; contract "Lead policy API" v3 has the shape.

- [fact] Wiring: server.rs claim -> LeadPolicy::claim_refused / claim_granted; task_update -> task_updated; message_send -> message_sent. All return at once when Jev is off; with Jev on they spawn. Routing = LeadPolicy::route (rules, then Assist::escalation, decide_route, deliver, one escalation_raised row, persister flush).
- [fact] Human queue has no store: lead::human_queue over escalation_raised rows whose details.delivered has human_queue and outcome is null. Served by GET /api/human, needs_you in /api/state (cached view), tirith lead human, tray (reads needs_you from /api/state; osascript notification when the count grows).
- [fact] Deterministic rules run on the escalation's own text only (note, message, refused claim reason + paths). Task titles and holders' claim reasons are excluded on purpose: this very task's title contains "credentials, permissions".
- [gotcha] The routed message to a worker can ride on the worker's own blocking task_update result (finish() awaits a flush, the spawned route runs meanwhile). Tests must read the action's inbox too.
- [gotcha] JevAnswer for a 2-question request keeps only the picked option's probability per question, so P(needs_human) is logged separately as details.needs_human_p.
- [gotcha] Refusal counting is in memory (LeadPolicy.refusals), keyed by agent + sorted path set; a restart forgets it. claim_waiting refusals count once per call.
- [measured] jev_bench escalation, gateway, repeat 3, run once with a-priori thresholds (target/jev_bench/escalation-run1): route_accuracy 0.793 (0.776-0.810), CI [0.684, 0.885]; off baseline (rules, else lead) 0.397. human_recall 0.786 (11/14 each repeat), CI [0.538, 1.0]: acceptance >= 0.95 NOT met. Misses are non-deterministic policy/product questions (esc-021, esc-029, esc-039) Jev routes to lead_can_decide at P 0.77-0.95, so no threshold fixes them. deterministic_recall 1.0, human_precision 0.917, false_human_rate 0.023 (esc-035, a message quoting the rule's own phrases), pointer_hit_rate 0.833, fallback 4.6%, 141 Jev calls, $0.015, 1 failure. Rules and prompt were written after one read of the cases, so deterministic_recall on this set is not independent.
- [lesson] Off arm of the escalation site is a harness-computed baseline (lead::human_rule, else lead), because the daemon routes nothing with Jev off.
