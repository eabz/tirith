---
id: 0bb44b86-6b63-427a-b54b-c49e57fca595
permalink: symbol-anchored-claims-vs-edit-windows-measured-opportunity-2026-09-17
title: "Symbol-anchored claims vs edit windows: measured opportunity (2026-09-17)"
kind: research
tags:
- claims
- anchors
- edit-window
- wait_secs
- adr-0029
- benchmark
paths:
- docs/5-decisions/0029-symbol-anchored-claims.md
- src/claims.rs
- src/types.rs
author: claims-symbols
updated_by: claims-symbols
created_at: 2026-09-17T04:17:25Z
updated_at: 2026-09-17T04:17:25Z
---

Task 06e67391, ADR-0029 (Proposed), contract "Symbol-anchored claims" v1.

- [method] Replayed every shared-file write (python str.replace heredocs, Write/Edit tools, sed -i) from the 12 worker transcripts of jevlab e2e r2/r3 x off/jev against each run's base commit in timestamp order; all 12 final shared files matched byte for byte (73 edits). Changed lines mapped to Python symbols with ast. Claim holds/refusals from coord_full.jsonl. Scripts: lead-session scratchpad claims-symbols/{extract,replay,symbols,measure,measure_combo}.py (session-local).
- [fact] Edits touched imports 53, Config fields 24, __all__ 24, build_pipeline 24, Config.from_env 12, new helpers 5. Every feature task edits build_pipeline, so symbol anchors cannot separate them.
- [fact] 79 holds on shared files, median 44 s, 4187 s total, 122 s of edit commands inside. Read->write gap quartiles 9/13/18 s.
- [fact] 48 refusal episodes: 0 disjoint symbols, 20 append-only overlap, 21 same symbol, 7 holder never edited. 57% of episode wait was retry slack (file already free).
- [fact] Agent-blocked seconds (4 runs): today 2762 of 5923 worker-s; file+wait_secs 1331; strict anchors 2762; anchors (new members by name, imports ride along) 2410, +wait 1097; edit windows+wait at -10/+15, -20/+30, -30/+60 s: 5, 112, 731; anchors+windows+wait at -20/+30: 63, at -30/+60: 558.
- [lesson] Hold duration, not claim width, is the cost. Edit windows (claim hub file only around the write, with wait_secs) capture nearly all the gain with no code; anchors are gated on the bench-symbols e2e arm (>=20% over edit windows, no lost writes).
- [gotcha] Current daemons accept `f#X` as an unrelated RepoPath that does NOT overlap `f`: anchors fail open until implemented. RepoPath normalization would also split an anchor containing '/', so the anchor must be split off before path normalization.
- [gotcha] claim_waiting retries are not queued: a file-claim waiter could starve behind anchor claims in the same file; FIFO if measured.
- [fact] Prototype: pure Target::parse + overlaps with 19 unit tests, clippy pedantic clean, at lead-session scratchpad claims-symbols/anchors-proto (not in the crate). Overlap table is in the ADR and contract.
- [fact] This repo: src/server.rs in 35 of 74 tasks, src/state.rs 25, 04-primitives.md 18 (runtime tasks.json).
