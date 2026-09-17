---
id: 985889d6-7c18-4bf8-96fe-88fd08138f07
permalink: lead-policy-and-escalations-where-things-live-and-traps
title: "Lead policy, escalations and decision log: where things live and traps"
kind: gotcha
tags:
- lead
- adr-0027
- decision-log
paths:
- src/lead.rs
- src/state.rs
- src/store.rs
- src/server.rs
- src/dashboard.rs
- src/cli.rs
- tests/lead_escalation.rs
author: lead-builder
updated_by: swarm-lead
created_at: 2026-09-17T04:27:34Z
updated_at: 2026-09-17T05:30:00Z
---

- [fact] Lead identity: `lead::Lead::from_claims` matches a claim naming exactly `.tirith/lead`; an ancestor claim (`.tirith`) blocks it but is not the lead. Trap: claiming `.tirith` while holding `.tirith/lead` folds (absorbs) the lead path into the directory claim and silently ends leadership.
- [fact] server.rs holds `lead: Arc<LeadPolicy>`; the policy is deterministic: claim-aware `task_pull` and `task_pull_waiting` (long poll), `notice_published` (push to holders of exact or nested affected paths), and the escalation hooks `task_updated`, `message_sent`, `claim_refused`, `claim_granted`. server.rs only formats.
- [fact] Escalation routes are `lead` and `human`. A `human_rule` match (credentials, permissions, spending, destructive, addressed to the human) goes to the human queue and the lead is told; everything else goes to the lead's inbox; with no lead, or when the lead itself escalates, it goes to the human queue (rule `no_lead`). A message to the lead escalates only on a human rule. Routing runs inside the tool call.
- [gotcha] Lead log rows and routed messages do NOT make `State::is_dirty()` true (like renewals, see `Inner::has_background_writes`), so `finish()` never flushes for them; they reach disk on the persister tick or with the next write. A test that reads the file right after a call must wait or call `persister.sync()`.
- [gotcha] Claim lifecycle rows are written inside State (claim_logged, release, the reaper in access()). `State::lead_log()` goes through `access()` so expired leases are reaped (and logged) before reading. Policy unit tests must filter by event: every claim adds a claim_granted row.
- [gotcha] `claim_waiting` logs one `claim_waited` row per wait (refusals during the wait are not logged separately).
- [gotcha] Removing a `LeadEvent` variant makes old rows in `.tirith/runtime/lead_log.jsonl` load errors on restart; filter the file when an event is removed.
- [gotcha] `cargo fmt -- <file>` formats the whole crate, including files other agents hold; use `rustfmt --edition 2024 <file>` in a swarm.
