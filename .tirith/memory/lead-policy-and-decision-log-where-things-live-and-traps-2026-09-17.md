---
id: 985889d6-7c18-4bf8-96fe-88fd08138f07
permalink: lead-policy-and-decision-log-where-things-live-and-traps-2026-09-17
title: "Lead policy and decision log: where things live and traps (2026-09-17)"
kind: gotcha
tags:
- lead
- jev
- adr-0027
- decision-log
paths:
- src/lead.rs
- src/assist.rs
- src/state.rs
- src/store.rs
- src/server.rs
- src/dashboard.rs
- src/cli.rs
- tests/jev_assist.rs
- tests/budgets.rs
author: lead-builder
updated_by: lead-builder
created_at: 2026-09-17T04:27:34Z
updated_at: 2026-09-17T04:27:34Z
---

Tasks dce67dfc, 97c30f38, cf60a160 (lead-builder).

- [fact] Lead identity: `lead::Lead::from_claims` matches a claim naming exactly `.tirith/lead`; an ancestor claim (`.tirith`) blocks it but is not the lead. Trap: claiming `.tirith` while holding `.tirith/lead` folds (absorbs) the lead path into the directory claim and silently ends leadership.
- [fact] server.rs holds `lead: Arc<LeadPolicy>`; every Jev site is a LeadPolicy method (task_pull, search_decisions, search_notes, task_created, note_written, notice_published). server.rs only formats. `ServerHandle::enable_assist/assist_report` delegate to it (names kept for tests and cli).
- [fact] Assist sites return `Judged<T> { value, jev: JevTrace }`; `Assist::ask` applies `Site::budget()` with tokio::time::timeout and counts expiry as a failure. Skip rules live in lead.rs (`task_tier`, `task_pick_skip`), not in Assist.
- [gotcha] Lead log rows do NOT make `State::is_dirty()` true (like renewals, see `Inner::has_background_writes`), so `finish()` never flushes for them; they reach disk on the 1 s persister tick. A test that reads the file right after a call must wait or call `persister.sync()`.
- [gotcha] Claim lifecycle rows are written inside State (claim_logged, release, the reaper in access()). `State::lead_log()` goes through `access()` so expired leases are reaped (and logged) before reading. Policy unit tests must filter by event: every claim adds a claim_granted row.
- [gotcha] `claim_waiting` logs one `claim_waited` row per wait (refusals during the wait are not logged separately).
- [gotcha] Snapshot and Delta no longer derive Eq (f64 probabilities in rows).
- [gotcha] tokio `start_paused` needs the test-util feature, which is not enabled; budget tests use real time (1.5 s).
- [gotcha] `cargo fmt -- <file>` formats the whole crate, including files other agents hold; use `rustfmt --edition 2024 <file>` in a swarm.
- [measured] status with Jev on, all 6 sites used, 300 claims: 454 B without verbose.
