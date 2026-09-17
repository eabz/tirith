---
id: 67e802bb-5ebd-4df7-98f8-b3a278f6bfdd
permalink: e2e-hooks-arm-stop-gate-scope-violations-scorer-fake-closures
title: "e2e hooks arm: stop gate scope, violations scorer, fake closures"
kind: gotcha
tags:
- bench
- e2e
- hooks
paths:
- examples/e2e/hooks
- examples/e2e/lib/violations.py
- examples/e2e/lib/turns.py
- examples/e2e/lib/score.py
- examples/e2e/dryrun
author: hooks-finish
updated_by: hooks-finish
created_at: 2026-09-17T15:08:21Z
updated_at: 2026-09-17T15:08:21Z
---

Hooks protocol for the e2e kit (setup.sh --protocol hooks; task 2941432e).

- [fact] Stop gate test scope is config `gate_tests`: `task` (default) runs the task's own hidden acceptance module (tasks.json hidden_test, found via <run>/meta.json scenario_dir + task_ids.json, by id prefix then title) in a scratch copy of the repo; the full suite only runs as evidence (`suite_ok` in events.jsonl). `suite` is the old whole-suite gate. Unknown task -> suite. #gate
- [gotcha] The whole-suite gate escalated every worker's first task in a shared repo (others' half-done work turns the suite red). Task scope fixed it: dry run 8/8 done, 3 of them while the suite was red, 0 escalations. #gate
- [gotcha] The `task` gate is an oracle the manual arm lacks (failing hidden test names reach the model); compare acceptance across protocols only with gate_tests "suite". #validity
- [gotcha] Hubs reference symbols depend on other tasks' symbols (currency-precision needs free-shipping+tax-exempt; gift-wrap and small-order-fee need each other). A fake that splices only its own anchors can never pass a task gate: 1/8 done, 3 escalated. fake_worker_hooks.py splices the task's closure (computed once per run on the baseline, <run>/fake_closures.json, ~3 s with sys.executable). #dryrun
- [fact] score.json: `violations` (lib/violations.py; per_worker/total tool_edits, bash_writes, unclaimed, anchor_only, unclaimed_paths; null without transcripts), `workers` rollup (turns, coordination_only_turns, mechanical_turns, cost_units, coordination_only_cost_share, unclaimed_writes), `worker_turns.*.coordination_only` = mechanical + coordination. bench-prep's aggregate reads violations.total.unclaimed. #score
- [gotcha] violations.py times a write at its tool_result row and skips is_error results (denied edits); synthetic transcripts need tool_use ids, tool_result rows and millisecond timestamps (dryrun/transcript.py). Subagent edits live in the subagent transcript and are not counted. #score
- [gotcha] Gate runs python3 with PATH=/usr/bin:/bin (like score.py): /usr/bin/python3 starts slowly, gate p50 ~270 ms. #latency
- [fact] ADR-0031: a claim wait whose caller disconnects returns status "cancelled"; pre_edit treats any non-ok status as deny, so a killed hook's wait no longer grants an orphan claim (daemon builds with ADR-0031). #claims
