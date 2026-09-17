---
id: 4c6d52d1-c4bb-48dc-8c64-615abcb6ebc5
permalink: escalation-routing-thresholds-and-human-queue-shape-adr-0027-section-3
title: Escalation routing thresholds and human queue shape (ADR-0027 section 3)
kind: decision
tags: []
paths:
- src/lead.rs
- src/assist.rs
- src/server.rs
- src/dashboard.rs
- src/tray.rs
- examples/jev_bench/main.rs
author: esc-router
updated_by: esc-router
created_at: 2026-09-17T04:58:53Z
updated_at: 2026-09-17T04:58:53Z
---

Escalation routing runs only with --jev, in the background. Deterministic human rules (lead::HUMAN_RULES: credentials, permissions, spending, destructive, addressed_to_human; phrase match at word boundaries on the escalation's own text, never on task titles or other agents' claim reasons) decide before Jev. Otherwise one Jev request with two choice questions (route; record). Thresholds set before any benchmark run: route followed at P >= 0.5 (ROUTE_CONFIDENT), else the lead (no lead: human queue); P(needs_human) >= 0.25 (HUMAN_FLOOR) sends to the human queue whatever was picked; a record is pointed to at P >= 0.25 (POINTER_KEEP), up to 3, none means the lead. Repeated-refusal trigger: 3 refusals of the same agent+path set within 360 s (3 x MAX_CLAIM_WAIT_SECS), tracked in memory. The human queue is a view over escalation_raised log rows delivered to human_queue with no outcome (no separate store); outcomes are filled by a message to the escalating agent, its task leaving blocked, or the refused claim granted. A stop_report (phase 2) is fed to the router as a message to the lead.

## Rationale

ADR-0027 names the routes but no thresholds. Deriving the queue from the log keeps one source of truth that survives restarts and the 7-day retention. The human floor is asymmetric on purpose: a human-required item routed away costs more than a glance.

## Alternatives

- A separate persisted HumanQueue store
- Deterministic routing (rules, else lead) with Jev off too
- Rule text including task title and holders' claim reasons (false human hits)
