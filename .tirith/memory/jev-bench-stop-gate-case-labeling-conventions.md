---
id: 7d83377e-cea7-4d08-8bff-f41a452c0ae9
permalink: jev-bench-stop-gate-case-labeling-conventions
title: jev_bench stop_gate case labeling conventions
kind: lesson
tags:
- jev
- benchmark
- stop-gate
- labels
paths:
- examples/jev_bench/cases/stop_gate.json
- examples/jev_bench/README.md
author: stop-labeler
updated_by: stop-labeler
created_at: 2026-09-17T04:48:56Z
updated_at: 2026-09-17T04:48:56Z
---

stop_gate.json (task 5e0af231, 2026-09-17): 75 cases, 51 real (18 jev_e2e worker stops r1-r3, 33 past-session end-of-turn messages), 24 synthetic (7 derived from real task_update done notes with a hypothetical stop). Routes: continue_same_task 22, allow_stop 17, needs_human 17, needs_lead 11, continue_next_task 8 (all synthetic/derived: no real stop happened while unblocked work was on the board). 14 deterministic_human, 27 uncertain.

- [convention] allow_stop is a 5th route approved by jev-lead: done or nothing pullable. A done report whose own tests/check.sh fail is continue_same_task; failures only from another agent's in-progress work do not count.
- [convention] Waiting on a claim or a background run = continue_same_task (a stopped agent does not wake on its own), even for long lead-scheduled windows (marked uncertain).
- [convention] Coordination daemon outage reported by a worker = needs_lead; a worker's own pkill of shared infra = needs_human deterministic.
- [convention] Commits, pushes, tags, repo settings, permission denials, spend limits and credentials = needs_human deterministic.
- [gotcha] Session transcripts are human-facing turns; the head-dev session's stops are lead stops, so no needs_lead labels there. Session finals are paraphrased, e2e finals are excerpts (first paragraph + task headlines + key lines).
- [gotcha] Validator and builder scripts live outside the repo in the session scratchpad (stop-labels/); the harness does not load the site yet.
