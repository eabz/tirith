---
id: f80a1720-3c60-412e-937f-78c4c610a48d
permalink: e2e-coordination-benchmark-arms-runbook-scoring-dry-run-gotchas
title: "e2e coordination benchmark: arms, runbook, scoring, dry-run gotchas"
kind: gotcha
tags:
- bench
- e2e
- adr-0029
- 5c603223
paths:
- examples/e2e
author: bench-prep
updated_by: bench-prep
created_at: 2026-09-17T15:22:11Z
updated_at: 2026-09-17T15:49:01Z
---

Coordination benchmark (task 5c603223) plumbing, 2026-09-17.

- [fact] setup.sh --arm baseline|windows|symbols sets the four prompt paragraphs --claims file|symbol, --hold task|edit, --wait sleep|server, --pull poll|wait (prompt/<opt>_<value>.md; hubs keeps its own claims_*.md). Without --arm each scenario keeps its original prompt (forge file/task/sleep/poll, hubs file/edit/server/poll) #arms
- [fact] runbook: bench/freeze_bin.sh <checkout> <lab>/bin (tirith.build.json sidecar with commit, src_dirty, sha256, FREEZE_NOTE; setup copies it into meta.json), bench/prepare_round.sh <lab> <round> [base_port] (6 runs r<N>-<scenario>-<arm>, prompts in <lab>/prompts, prompt_diff check, printed runbook), bench/watch.py (reads /api/state, exit 0 done / 2 rerun / 3 stuck), usage.tsv + transcripts, score.sh before stop.sh, lib/aggregate.py <lab> #runbook
- [fact] score.json "bench" (lib/benchstats.py) is the flat block aggregate.py reads; usage.tsv (run, worker, total_tokens, tool_uses, duration_ms[, output_tokens, cost_usd]) in the run or lab dir is reread at aggregate time. Wall metric is done_time_s (first call -> last task done), decided with the lead; decision rules R1/R2 recorded as a Tirith decision and in aggregate.py #scoring
- [gotcha] before the pull fix (task 7bc643e2), task_pull wait_secs waited 120 s on an empty board and behind other agents' in_progress task paths, so B/C fakes never waited on claims (pull idle 420-485 s). After the fix the same round: B/C 2-4 server claim waits, blocked 2.7-5.7 s, done 28-42 s vs baseline 44-66 s #pull
- [gotcha] setup's --pull wait probe checks that the task_pull description mentions wait_secs: timing a pull needs a held task on the board, and an empty board answers none at once after the fix #setup
- [gotcha] lost_writes now runs for Forge too (snapshots for every scenario); a Forge regression can also be a later task breaking an earlier test, and its anchors are empty (attribution task) #lost-writes
- [lesson] fake_worker_forge.py implements Forge from reference/ by merging each task's keyword-owned reference lines after the nearest preceding reference line present (difflib alignment); --selfcheck proves 200 random orders give reference/. Rogue runs detect 13-33 lost writes and 3-5 unclaimed writes in both scenarios and arms #dryrun
- [gotcha] the Bash tool shell is zsh: `set -- $var` does not word-split; wrap loops in bash -c. Port 7960 was taken by something else on this machine #shell
