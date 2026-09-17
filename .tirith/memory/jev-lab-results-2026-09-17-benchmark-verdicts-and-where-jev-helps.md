---
id: 2e86fa06-4ea9-4479-bd65-86f462fd274e
permalink: jev-lab-results-2026-09-17-benchmark-verdicts-and-where-jev-helps
title: "Jev lab results 2026-09-17: benchmark verdicts and where Jev helps"
kind: research
tags:
- jev
- benchmark
- tokens
- latency
- experiment
paths:
- src/assist.rs
- src/jev.rs
- examples/jev_bench
- examples/jev_e2e
- docs/5-decisions/0024-jev-assist-experiment.md
author: jev-lead
updated_by: jev-lead
created_at: 2026-09-17T03:14:21Z
updated_at: 2026-09-17T03:14:21Z
---

Lab run by jev-lead with 33 agents. Report artifact: https://claude.ai/artifact/3xpexqZ9a3yTbcFj1Jk3NR

- [measured] Micro-bench (175 blind cases, gateway x3, TypeSafe x1): brief filter HARMS recall (0.978 -> 0.889, all-must-read 95% -> 62%); wins: memory_search R@5 0.79->1.0, decision_search 0.36->0.94, task_pull top-1 0.40->0.91, note_duplicate 0.60->0.97, notice fan-out F1 0->0.94; no effect: broadcast; inconclusive: task_duplicate, conflict_advice. #jev
- [measured] Added latency per assisted call: gateway +300-350 ms p50, TypeSafe direct +150-185 ms. Cost ~$0.00003/call. #latency
- [measured] E2E (jev_e2e kit, 3 agents, 2 valid rounds per arm): both arms 57/57 hidden tests; Jev arm slower (mean 541 s vs 446 s); worker tokens equal (~388k); coordination bytes inconsistent. `skipped` in briefs made agents page more. #benchmark
- [gotcha] E2E round 1 invalid: a worker ran `pkill -f "cat"`, killing run daemons and Claude Helper processes. Worker prompts must forbid process signals and background waits; foreground `sleep` is blocked for agents, use python time.sleep. #experiment
- [lesson] Econ from 17 sessions: Tirith output ~2-5% of spend on v1; input-token cuts never save time (5k tokens ~0.08 s); peer waits are the largest bottleneck (59% in swarm) and deterministic fixes address them. #tokens
- [lesson] Deterministic wins found: claim-aware task_pull (confirmed bug), decision_list semantic search only considers newest 30, empty-result fallback when Jev rejects all, check.sh grep drops assertion messages, 4 test gaps (Decision::matches empty query, memory title weight, store slug, Claim::is_expired boundary). #roadmap
- [lesson] Jev-only wins with evidence: breaking public-signature detection in diffs (20/20 vs regex 12/20), completion check at Stop (3/4, tiny set), decision contradiction check (AUC 0.98, rare). Rejected: read/brief filtering for tokens, test skipping, log triage, command approval (rated `env | sort` safe), model routing via Jev, question deflection. #jev
