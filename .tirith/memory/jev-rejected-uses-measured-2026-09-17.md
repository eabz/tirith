---
id: a21b6baf-499e-4844-86a9-a535be54732b
permalink: jev-rejected-uses-measured-2026-09-17
title: Jev rejected uses (measured 2026-09-17)
kind: lesson
tags:
- jev
- rejected
- do-not-retry
- benchmark
paths:
- src/assist.rs
- src/lead.rs
- src/jev.rs
- docs/5-decisions/0026-jev-semantic-judgment-layer.md
- examples/jev_bench
author: jev-lead
updated_by: jev-lead
created_at: 2026-09-17T04:04:38Z
updated_at: 2026-09-17T04:04:38Z
---

Registry of Jev uses that were tried and did not work. Check here before proposing a Jev feature; retry only with a new hypothesis and a better measurement plan. Source: Jev lab 2026-09-17 (report https://claude.ai/artifact/3xpexqZ9a3yTbcFj1Jk3NR), ADR-0026, memory note jev-lab-results-2026-09-17-benchmark-verdicts-and-where-jev-helps.

Harmful or no effect (removed from Tirith):
- [rejected] Claim brief filtering: recall of must-read rows 0.978 -> 0.889, all-must-read shown 95% -> 62%, for ~164 tokens saved per claim. #brief
- [rejected] Broadcast audience narrowing: F1 0.787 -> 0.794 (no effect) and it suppresses recipients. #broadcast
- [rejected] Conflict advice as text to workers: 62% acceptable, noisiest labels (kappa 0.79); workers deliberated more. May return only as a lead action (e.g. WAIT -> server wait), measured that way. #claims
- [rejected] Showing Jev metadata to workers (skipped, advice, relevance, picked_by...): e2e Jev arm slower (541 s vs 446 s mean), workers paged more after `skipped`. #e2e

Did not beat a deterministic baseline:
- [rejected] Documentation pointers on task assignment: recall@3 0.425 vs BM25 0.383, CI includes 0, +680 ms. See [[jev-docs-pointers-on-task-assignment-probe-results-2026-09-17]]. #docs
- [rejected] Test selection / skipping test files: missed 1 of 7 failing files at P>=0.5; a static module->test map missed none. #tests
- [rejected] Failure log triage / chunk pruning: kept 9 of 14 failure blocks; a parser keeps all; called a real timeout bug `flaky` at 0.78. #tests
- [rejected] Test log line filtering: a regex kept 4/4 needed chunks, Jev 3/4. #tests
- [rejected] File prediction from task text for scheduling: title-only recall 0.43, leak-free 0.48; worse than declared paths at N>=5. #scheduling
- [rejected] Picking files to open from a repo map: 58% of needed files at P>=0.5. #context

Unsafe or no value by design:
- [rejected] Command / permission risk approval: rated `env | sort` (prints keys) read-only at 0.79; batching flipped `git checkout -- .` to local edit. Never approve anything with Jev. #safety
- [rejected] Question deflection to recorded decisions: 0 of 245 real agent messages were answerable from decisions. #escalation
- [rejected] Inbox urgency / interrupt-now scoring: all 18 real end-of-turn messages scored < 0.5; can only rank. #escalation
- [rejected] Next-tool choice for agents: removes no turn (agent still pays to ask and read). #loop
- [rejected] Mid-session model switching: each switch rewrites the prompt cache (~$6.40). Model choice only at task/subagent start, and it is a policy, not a Jev win. #routing
- [rejected] Three-level read-now/skim/skip brief: Jev could not separate act from FYI. #brief
- [rejected] Token savings from filtering reads in general: <=0.5-2.7% of spend, never saves time (5k input tokens ~0.08 s < one Jev call), and a re-read cancels it. #tokens

Inconclusive (kept only as hints to the lead): task duplicate detection (accuracy 0.60 -> 0.88, FP 19%, CI crosses 0).
