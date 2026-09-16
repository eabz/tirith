---
id: 3f6d77d9-6dbb-46ee-a3de-76389dfacf84
permalink: no-manual-acknowledgement-a-notice-is-seen-when-it-is-delivered
title: "No manual acknowledgement: a notice is seen when it is delivered"
kind: decision
tags: []
paths:
- src/notices.rs
- src/state.rs
- src/server.rs
- src/store.rs
- src/cli.rs
- src/dashboard.html
- docs/1-about/04-primitives.md
- README.md
- AGENTS.md
- docs/6-agent-workflow/03-tirith-dogfooding.md
author: head-dev-fable
updated_by: head-dev-fable
created_at: 2026-09-16T03:41:35Z
updated_at: 2026-09-16T03:41:35Z
---

The notice_ack tool, the `tirith notice ack` subcommand and the dashboard's "unacknowledged" tile are removed. A notice counts as seen by an agent the moment Tirith delivers it to that agent: in a brief on claim, or in a notice_list with unread:true. Seen marks are per agent, persisted append-only in .tirith/runtime/notice_seen.jsonl (the log ADR-0021 introduced, renamed from acks to seen) and replayed on load; `unread` means "never delivered to this agent". The dashboard shows, per notice, which agents have seen it, with no global warning count.

## Rationale

eabz, 2026-09-16: a state nobody maintains is noise; either enforce it or remove it. Enforcing an explicit ack would add a mandatory call to every edit cycle, the token cost brief exists to remove. Delivery is already tracked for brief paging, so "seen" costs nothing and is true whenever it says so. Removing a tool also shrinks tools/list.

## Alternatives

- Enforce ack by refusing a claim until undelivered notices are acked (rejected: one more mandatory round trip per edit)
- Keep ack optional and hide the tile (rejected: dead surface)
- Auto-ack on delivery but keep the tool (rejected: two names for one fact)
