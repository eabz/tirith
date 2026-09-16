---
id: a6838e75-4543-4e95-9ac4-c7fa341c92b4
permalink: agent-messages-ride-on-the-next-result-runtime-only-24-h-retention
title: Agent messages ride on the next result; runtime-only, 24 h retention
kind: decision
tags: []
paths:
- src/messages.rs
- src/state.rs
- src/store.rs
- src/server.rs
author: claude-ci-speedup
updated_by: claude-ci-speedup
created_at: 2026-09-16T03:38:47Z
updated_at: 2026-09-16T03:38:47Z
---

message_send/message_list are the sixth primitive. Delivery is a piggyback in TirithServer::finish next to lost: the recipient's next result carries `inbox` (newest 5, text cut to 200) and `inbox_more`; delivered marks are per daemon lifetime; broadcasts (`*`) are addressed at send time to agents seen in the last hour minus the sender and store that audience. Messages live in .tirith/runtime/messages.jsonl (never committed), pruned to 24 h on load with a rewrite on the next persist. `inbox_more` is a flat count, not a `more` object, to avoid two shapes under one key. ADR-0020.

## Rationale

Coordination talk must reach every MCP client, not only Claude sessions; the next-result path already exists and is proven by lost and brief; a conversation is not repository knowledge, so it is not committed.

## Alternatives

- keep using the client's session messaging
- a message_receive polling tool
- commit messages beside notices
- a per-section more object like the brief
