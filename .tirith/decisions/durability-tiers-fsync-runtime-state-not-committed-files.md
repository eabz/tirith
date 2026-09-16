---
id: d9f6673a-5a59-4a42-afde-2ffeb4050953
permalink: durability-tiers-fsync-runtime-state-not-committed-files
title: "Durability tiers: fsync runtime state, not committed files"
kind: decision
tags: []
paths:
- src/store.rs
- docs/5-decisions/0010-incremental-persistence.md
author: head-dev-fable
updated_by: head-dev-fable
created_at: 2026-09-16T02:48:14Z
updated_at: 2026-09-16T02:48:14Z
---

Rust's File::sync_all is F_FULLFSYNC on macOS (10-20 ms), which is the whole latency of a single agent's mutation (17 ms p50) and makes memory_write 6x slower than other mutations at 100 agents. Two tiers from now on. Tier 1, runtime truth that only Tirith holds: claims.json, tasks.json, meta.json and the runtime logs get write, sync_all, rename, plus one directory sync per apply. Tier 2, committed files whose history git keeps: memory notes, contract files, and the appends to notices.jsonl and decisions.jsonl are written atomically (temp + rename, or append) with no per-file F_FULLFSYNC; a power loss may lose the last second of them, and the next persister tick rewrites whatever the in-memory state still holds. Directory sync once per apply stays for both tiers.

## Rationale

Tirith is a single-user, single-machine tool (01-purpose.md). The cost of losing a note or a notice append to a power cut is a re-run of memory_write; the cost of F_FULLFSYNC per file is paid by every agent on every mutation. Claims and tasks are the only state that cannot be reconstructed, so they keep the strong sync.

## Alternatives

- sync_all everywhere (current; measured 17 ms floor and 314 ms memory_write p50 at 100 agents)
- sync_data instead of sync_all (still F_FULLFSYNC on macOS in std)
- batch all changed files then one F_FULLFSYNC (helps a little; still one 15 ms stall per flush)
