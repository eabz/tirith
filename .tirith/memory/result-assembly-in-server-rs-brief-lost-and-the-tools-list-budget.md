---
id: 5d14a485-7de2-46d0-9e72-905a050a17c1
permalink: result-assembly-in-server-rs-brief-lost-and-the-tools-list-budget
title: "Result assembly in server.rs: brief, lost, and the tools/list budget"
kind: lesson
tags:
- tokens
- brief
- lost-lease
- budget
paths:
- src/server.rs
- src/state.rs
- tests/http_roundtrip.rs
- tests/budgets.rs
author: claude-token-diet
updated_by: agent-2
created_at: 2026-09-16T03:10:20Z
updated_at: 2026-09-16T04:09:36Z
---

Everything that piggybacks on a tool result goes through two places; the agent-messages inbox (ADR-0020) uses the same two, not a third path.

- `TirithServer::finish(agent, outcome)` is the per-result hook. It attaches `lost` (`State::take_lost`, once per call), `inbox` (`State::take_inbox`, same rule) and `persist_error`, and builds the text line from `summary()` plus a `warning: ...;` prefix list. Every tool passes `Some(&input.agent)`; `status` passes `input.agent.as_deref()`.
- `attach_brief(&value, &brief)` in server.rs is the claim-only section builder: digests per row, sections omitted when empty, `more` always present, then a loop that drops the oldest row of the largest section until `json_len` fits `BRIEF_MAX_BYTES` (4096). Row digests must go through `compact()` so ids are 8 chars and nulls vanish; `tests/budgets.rs::assert_compact` walks list rows and fails on any null or empty array.

- [lesson] Two tests pin the tools/list size: tests/budgets.rs and tests/http_roundtrip.rs::tool_list_stays_small (7,800 total and 500 per tool since ADR-0020 and ADR-0021). Adding one optional boolean to ClaimInput cost ~60 chars of schema; the claim description had to shrink to about 170 chars to fit. Budget every new field before writing its doc comment. #budget
- [gotcha] Empty brief sections are omitted, not `[]`. A test that asserts `claim["memory"] == json!([])` fails with `Null != []`; assert `.get("memory").is_none()` and `more.memory == 0` instead. Memory primitive contract v6 records this. #brief
- [fact] Seen marks for notices shown in a brief or an unread listing go through `mark_delivered` in state.rs into the notice board and `Inner.seen_log`, persisted append-only to `.tirith/runtime/notice_seen.jsonl` (ADR-0021), so they survive a restart. Reaped leases (`Inner.reaped`) and message delivery marks (`MessageBoard.delivered`) stay in memory and are lost on restart on purpose (ADR-0015, ADR-0020).
- [fact] `newest()` in state.rs is the shared per-path dedupe + newest-first helper for brief sections; `capped()` takes BRIEF_LIMIT (5).

- relates_to [[tool-schema-budget-in-server-rs]]
- relates_to [[messages-primitive-delivery-and-budget-notes]]
- documents [[design/v1-audit-2026-09-16]]
