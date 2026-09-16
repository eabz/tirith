---
id: e3a3c46b-cf3a-49ec-9993-70bb0fde9d77
permalink: messages-primitive-delivery-and-budget-notes
title: "Messages primitive: delivery and budget notes"
kind: gotcha
tags:
- messages
- tokens
paths:
- src/messages.rs
- src/server.rs
- src/state.rs
author: claude-ci-speedup
updated_by: claude-ci-speedup
created_at: 2026-09-16T03:41:26Z
updated_at: 2026-09-16T03:41:26Z
---

`src/messages.rs` is the domain; delivery is a piggyback in `TirithServer::finish` (server.rs) next to `lost`; the log is runtime-only. See ADR-0020.

## Observations
- [gotcha] `State::take_inbox` runs on every call through `access(None, ..)`, like `take_lost`, so it must never renew leases or dirty state; delivered marks are in-memory only (`MessageBoard.delivered`) #messages
- [gotcha] A broadcast's audience is fixed at send time from `Inner.last_seen` (agents seen within BROADCAST_WINDOW, minus the sender) and stored on the message; an agent that first appears after the send does not get it #messages
- [gotcha] Pruning happens in `State::new` (24 h); when it drops anything the LogCursor is marked for rewrite so `runtime/messages.jsonl` shrinks on the next persist #messages
- [lesson] Two tools cost ~640 chars of tools/list schema even with one-sentence descriptions; the bound went 7,000 -> 7,800 in tests/budgets.rs and tests/http_roundtrip.rs and ADR-0013's row; the floor per tool is ~210 chars of structure #tokens
- [fact] The inbox count beyond the five delivered is `inbox_more` (flat), not `more`, because a claim already carries the brief's `more` object #messages

## Relations
- documented_in [[ADR-0020]]
- relates_to [[result-assembly-in-server-rs-brief-lost-and-the-tools-list-budget]]
