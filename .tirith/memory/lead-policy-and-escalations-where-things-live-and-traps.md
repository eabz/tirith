---
id: 985889d6-7c18-4bf8-96fe-88fd08138f07
permalink: lead-policy-and-escalations-where-things-live-and-traps
title: "Lead policy, escalations and decision log: where things live and traps"
kind: gotcha
tags:
- lead
- adr-0027
- decision-log
- human-queue
paths:
- src/lead.rs
- src/state.rs
- src/store.rs
- src/server.rs
- src/dashboard.rs
- src/cli.rs
- src/messages.rs
- src/tray.rs
- tests/lead_escalation.rs
author: lead-builder
updated_by: queue-fix
created_at: 2026-09-17T04:27:34Z
updated_at: 2026-09-17T15:35:22Z
---

- [fact] Lead identity: `lead::Lead::from_claims` matches a claim naming exactly `.tirith/lead`; an ancestor claim (`.tirith`) blocks it but is not the lead. Trap: claiming `.tirith` while holding `.tirith/lead` folds (absorbs) the lead path into the directory claim and silently ends leadership.
- [fact] server.rs holds `lead: Arc<LeadPolicy>`; the policy is deterministic: claim-aware `task_pull_waiting` (long poll), `notice_published` (push to holders of exact or nested affected paths), and the escalation hooks `task_updated`, `message_sent`, `claim_refused`, `claim_granted`. server.rs only formats.
- [fact] Routing (ADR-0027 s3, revised 2026-09-17), all in `LeadPolicy::raise`: trigger `message_to_human` -> human queue (rule `to_human`); live lead and someone else escalated -> lead inbox (rule `live_lead`, `details.tag` = human rule category, text "(trigger; may need the human: X)"); the lead escalated itself -> logged only (`lead_itself`, delivered []); no lead -> human queue (`no_lead`). Human rules never route. A message to the lead never escalates.
- [fact] `human` (messages::HUMAN, `is_human` ignores case) is reserved: server.rs `agent()` refuses it as a caller (status too), MessageBoard stores `to: "human"`, never delivers it to an inbox, and strips it from broadcast audiences. Replies are sent by `State::message_from_human` (not activity, like `notify`).
- [fact] Answering: `lead::answer_human(state, id, reply)` finds the id in `human_queue_of`, sends the reply first (reply_to only if the queued message is still kept), then sets outcome {answered_by: human, via: done|reply}. Dashboard `POST /api/human/{id}/done` requires Content-Type application/json and Origin host == Host (403 otherwise), calls `Shared::refresh()` so `/api/state` drops the item at once. CLI: `tirith lead human done <id|#id> [--reply]`.
- [gotcha] `answer_open` skips `message_to_human` rows: otherwise any message to the lead (the usual sender) would silently close the lead's own human item.
- [gotcha] Tool results `compact()` ids to 8 chars and inbox rows carry only id/from/text/at (no reply_to); compare full ids with starts_with, read reply_to via `message_list`.
- [gotcha] Policy unit tests run on a ManualClock, so messages share one timestamp and inbox order among them is random (sorted by (at, uuid)); assert with any(), not index.
- [gotcha] Lead log rows and routed messages do NOT make `State::is_dirty()` true, so `finish()` never flushes for them; they reach disk on the persister tick. A test reading the file right after a call must wait or `persister.sync()`.
- [gotcha] Claim lifecycle rows are written inside State; `State::lead_log()` goes through `access()` so expired leases are reaped (and logged) first. Policy unit tests must filter by event.
- [gotcha] Removing a `LeadEvent` variant breaks loading old `lead_log.jsonl` rows; `Trigger`/`Route` live in `details` as strings, so removing a Trigger (MessageToLead) is safe.
- [gotcha] `cargo fmt -- <file>` formats the whole crate; use `rustfmt --edition 2024 <file>` in a swarm.
- [gotcha] memory_write with an existing title but no `permalink` created a duplicate for this note (custom permalink); pass `permalink` to update it.
