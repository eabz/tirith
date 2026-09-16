---
id: 6c68bc8a-ee82-44fd-9298-a9cb8524e2c1
permalink: tasks-belong-to-their-owner-until-silent-contract-republishes-are-guarded
title: Tasks belong to their owner until silent; contract republishes are guarded
kind: decision
tags: []
paths:
- src/tasks.rs
- src/contracts.rs
- src/state.rs
- src/server.rs
- src/cli.rs
author: claude-ci-speedup
updated_by: claude-ci-speedup
created_at: 2026-09-16T02:49:01Z
updated_at: 2026-09-16T02:49:01Z
---

Foreign status changes on an in_progress task are conflict unless force (which is recorded in the note). In-progress tasks whose owner made no call for task_orphan_secs (default 1800, serve --task-orphan-secs, 0 disables) return to todo with a note; status counts tasks_orphaned; last activity is not persisted. Contract republish keeps consumers unless given, notifies the union of old and new consumers, and refuses a stale expected_version with conflict. ADR-0018.

## Rationale

Audit findings: silent reassignment of tasks, dead owners blocking dependency subtrees forever, republishes wiping consumers so notices went to nobody, and last-write-wins on concurrent publishes.

## Alternatives

- reap on lease expiry (agents without claims still own tasks)
- persist last activity (a write per call, undone by ADR-0010)
- require expected_version always (breaks callers)
- notify only new consumers
