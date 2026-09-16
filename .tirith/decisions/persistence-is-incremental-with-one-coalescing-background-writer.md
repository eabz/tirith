---
id: 65e4ec66-0469-4c6d-ac7e-d03188e8754d
permalink: persistence-is-incremental-with-one-coalescing-background-writer
title: Persistence is incremental with one coalescing background writer
kind: decision
tags: []
paths:
- src/state.rs
- src/store.rs
- src/server.rs
author: storage-claude
updated_by: storage-claude
created_at: 2026-09-16T02:01:05Z
updated_at: 2026-09-16T02:01:05Z
---

State produces per-primitive Deltas (claims/tasks whole when changed, only changed contract files, JSONL appends for notices/decisions with rewrite only after in-place edits). One Persister task drains deltas in seq order; mutating tool calls flush and wait, reads never wait, lease renewals are folded into the next claims write or a 1 s tick. New primitives use LogCursor (JSONL) or ChangedIds (one file per item) and add a Delta field. ADR-0010.

## Rationale

Whole-snapshot-per-call capped the daemon at ~95 calls/s with 2000 notices + 50 contracts and made latency = agents x 10 ms (200 agents: 2.3 s p50). After: 5k-12k calls/s, 200 agents at 33 ms p50, measured with examples/swarm_bench.rs. Keeps the diffable JSON layout from ADR-0003.

## Alternatives

- Debounce whole-snapshot writes (bytes per change still grow with history)
- SQLite/redb/sled (loses PR diffability; not needed)
- Persist renewals synchronously to claims.json only (still a disk write on every read)
