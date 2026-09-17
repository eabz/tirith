---
id: 348cfa71-bf75-4648-b887-9703a56920c9
permalink: jev-docs-pointers-on-task-assignment-probe-results-2026-09-17
title: "Jev docs pointers on task assignment: probe results 2026-09-17"
kind: research
tags:
- jev
- benchmark
- adr-0026
- phase-2
- rejected
paths:
- docs
author: docs-probe
updated_by: jev-lead
created_at: 2026-09-17T03:59:19Z
updated_at: 2026-09-17T04:04:12Z
---

ADR-0026 phase 2 item "documentation pointers on task assignment". Task bce1f9cd. VERDICT: rejected; the prototype (examples/jev_docs_probe, untracked Python) was deleted on 2026-09-17 at the user's request to reduce noise. Numbers kept here.

Setup: deterministic index of AGENTS.md + docs/**/*.md headings (h1-h3), 280-286 sections, 300-char excerpts, build ~12 ms. 40 real tasks from .tirith/runtime/tasks.json, must-read sections labeled by hand BEFORE any run (120 needs, 77 implicit), plus a blind second labeler (147 picks). Baseline B2 = BM25 with task-referenced sections first. Jev J1 = top-3 by probability over bounded candidates (BM25 top-40 + referenced, cap 50), one call per task.

- [measured] recall@3 labels A: B2 46/120 (0.383), J1 51/120 (0.425), diff CI95 [-0.040, +0.165]; labels B: 43/147 vs 48/147, CI [-0.038, +0.129]. Precision@3 A: 0.475 both. #benchmark
- [measured] Label noise (12-22% disagreement) is larger than the effect (4 points). #benchmark
- [measured] Variants J2 (p>=0.5) 0.367 and J3 (Jev reorders B2 top-10) 0.408 do not help. #benchmark
- [measured] Jev p50 680 ms, p90 866 ms, ~9.2k input tokens/call, $0.00039/call; deterministic rank 3 ms. #latency
- [lesson] Implicit-need recall looks like a Jev win vs B2 (31/77 vs 17/77) only because B2 ranks named files first; plain BM25 gets 29/77. #jev
- [lesson] If pointers ever ship, use deterministic ranking and prefer sections the task does not already name. Relates to [[jev-rejected-uses-measured-2026-09-17]]. #jev
