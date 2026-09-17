---
id: 69b21202-ae34-4c46-b8a9-edd6ea5e79c0
permalink: jev-bench-decision-conflict-and-task-depends-labeling-conventions
title: jev_bench decision_conflict and task_depends labeling conventions
kind: fact
tags:
- jev
- benchmark
- labels
paths:
- examples/jev_bench/cases/decision_conflict.json
- examples/jev_bench/cases/task_depends.json
- examples/jev_bench/README.md
author: bench-labels-t1
updated_by: bench-labels-t1
created_at: 2026-09-17T04:26:47Z
updated_at: 2026-09-17T04:26:47Z
---

Blind labels written 2026-09-17 (task 3e035902) with no Jev call, before the contradiction check and suggested_depends_on were implemented.

- [convention] decision_conflict `supersedes` lists records whose actual decision the new one makes wrong. A contradiction only in a side detail or the rationale, or of a record a newer seeded record already superseded, goes in `tolerated`, which scores neither as a hit nor as a false flag.
- [convention] task_depends `depends_on` lists only OPEN seeded tasks that are direct prerequisites. Done tasks are never labeled. Transitive prerequisites, soft orderings and overlapping scopes go in `tolerated`.
- [convention] Repo-derived task text has file names, paths and 8-hex task ids stripped; `paths` keeps the real hints. The validator checks this with regexes.
- [fact] Recorded depends_on on the board is incomplete in places: 5341e03f omits Tier 1 tasks, and c9af0bb7 omits the escalation bench its acceptance needs (tdep-018 labels it, marked uncertain).
- [fact] Path-overlap baseline: decisions recall 0.90 / precision 0.25; tasks 3/41 exact.
- [gotcha] The validator lives outside the repo (session scratchpad bench-t1/validate.py). The harness needs its own validation for these two sites.
