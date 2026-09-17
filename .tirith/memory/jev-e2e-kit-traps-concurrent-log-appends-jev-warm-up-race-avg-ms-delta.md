---
id: 0dceb171-a495-4439-bf50-82ff8f3061f7
permalink: jev-e2e-kit-traps-concurrent-log-appends-jev-warm-up-race-avg-ms-delta
title: "jev_e2e kit traps: concurrent log appends, jev warm-up race, avg_ms delta"
kind: gotcha
tags:
- jev
- bench
- e2e
paths:
- examples/jev_e2e
author: bench-e2e
updated_by: bench-e2e
created_at: 2026-09-17T02:30:26Z
updated_at: 2026-09-17T02:30:26Z
---

End-to-end Jev benchmark kit (setup.sh / tb / score.sh / stop.sh, scenario "Forge" in Python 3.9 stdlib).

- [gotcha] tb responses exceed PIPE_BUF (task_pull, claim briefs ~3.5 KB); concurrent `>>` appends interleaved JSONL lines. tb appends under a perl flock #logging
- [gotcha] `tirith serve --jev` answers MCP calls before it prints `jev: on (...)`, warm_up runs after the listener starts; setup.sh waits for that line or seeding runs partly without Jev #jev
- [gotcha] status.jev.avg_ms cannot be subtracted between snapshots; score.py recomputes it from per-site total_ms #jev
- [fact] Jev during seeding: ~35 calls (28 note_duplicate, 7 task_duplicate); it flagged correlation-id as a duplicate of request-id (conf 1.0) and made two similar_note false positives (m06, m12) #jev
- [fact] reference/ solution passes 57/57 hidden + 5/5 integration; the baseline passes 2/57 #bench
