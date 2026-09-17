---
id: c93b5084-8890-45fd-9fb5-847a32d8d701
permalink: coordination-benchmark-arms-wall-metric-and-pre-registered-decision-rules
title: "Coordination benchmark: arms, wall metric and pre-registered decision rules"
kind: decision
tags: []
paths:
- examples/e2e/lib/aggregate.py
- examples/e2e/bench
- examples/e2e/prompt
- examples/e2e/setup.sh
- docs/3-tests
author: bench-prep
updated_by: bench-prep
created_at: 2026-09-17T15:21:16Z
updated_at: 2026-09-17T15:21:16Z
---

Task 5c603223 compares three worker protocols on one frozen daemon binary (bench/freeze_bin.sh), selected by setup.sh --arm, differing only in four prompt paragraphs: baseline = file claims held for the task, sleep-retry, task_pull poll; windows = file claims in edit windows, claim wait_secs 120, task_pull wait_secs 120; symbols = windows with ADR-0029 anchors (imports and __all__ ride on the symbol claim, each anchor of a multi-symbol edit, whole files for new and import-only files). Wall time is done_time_s (first coordination call to last task done). R1: windows ships if the mean relative paired difference vs baseline in done_time_s or cost (cost_usd, else total_tokens, else cost_units) is < 0 with a 95% bootstrap CI entirely below 0, and the CI of the paired hidden and integration test differences is not entirely below 0. R2: ADR-0029 Accepted only if 1 - sum(C blocked_s)/sum(B blocked_s) >= 0.20 with CI lower bound > 0, lost_writes 0 in every symbols run, and total failing test runs of symbols <= windows; else Rejected. Missing inputs = undecided. Provisional under 3 pairs per scenario. Implemented in lib/aggregate.py.

## Rationale

Rules must be fixed before real LLM rounds so the verdict cannot be tuned to the data. done_time_s instead of first-to-last call: a wait_secs pull on an empty board (before the pull fix) kept B/C workers 120 s past the last task, which measures stopping, not work. Imports riding on symbol claims follows ADR-0029 as proposed (lead confirmed): C must test the design as written; lost-write detection catches it if unsafe.

## Alternatives

- wall_time_s (first to last coordination call) as the wall metric
- whole-file claims for any edit that adds an import in the symbols arm (the brief's first wording)
- absolute paired differences pooled across scenarios for R1 (scales differ between Forge and hubs)
- a strict mean hidden-test difference >= 0 for 'no drop' (fails on one-test noise with 3 rounds)
