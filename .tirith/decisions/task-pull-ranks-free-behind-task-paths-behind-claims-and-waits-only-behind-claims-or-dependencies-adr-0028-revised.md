---
id: b6b5e271-fbe3-4326-933b-7230e0a5d09e
permalink: task-pull-ranks-free-behind-task-paths-behind-claims-and-waits-only-behind-claims-or-dependencies-adr-0028-revised
title: task_pull ranks free > behind task paths > behind claims, and waits only behind claims or dependencies (ADR-0028 revised)
kind: decision
tags: []
paths:
- src/tasks.rs
- src/state.rs
- src/lead.rs
- docs/5-decisions/0028-claim-aware-task-pull.md
- docs/1-about/04-primitives.md
author: pull-fix
updated_by: pull-fix
created_at: 2026-09-17T15:37:30Z
updated_at: 2026-09-17T15:37:30Z
---

Candidates keep priority/age order within three tiers: free of both holds; overlapping only other agents' in_progress task paths (assigned at once, waiting_on lists them); overlapping a live claim (only when every candidate does; first one, waiting_on lists all holds). task_pull with wait_secs takes tiers 1-2 at once, answers none at once when no task is todo, and waits only while every candidate is claimed or todo tasks wait on dependencies. A successful pull wakes waiting pulls.

## Rationale

The benchmark dry run showed pulls serializing on hub files listed in every task's paths (120 s waits behind forecasts, defeating edit windows and symbol claims) and workers idling 120 s on an empty board at the end of a run (last call 198 s vs last done 77 s). Claims are locks; task paths are forecasts.

## Alternatives

- Keep waiting behind in-progress task paths with a shorter cap
- Ignore in-progress task paths entirely
- Keep the long poll over an empty board to catch late tasks
