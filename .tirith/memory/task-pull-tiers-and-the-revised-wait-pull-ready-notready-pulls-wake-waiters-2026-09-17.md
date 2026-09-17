---
id: 6a98a10a-c1eb-45f3-b57c-d59199445420
permalink: task-pull-tiers-and-the-revised-wait-pull-ready-notready-pulls-wake-waiters-2026-09-17
title: "task_pull tiers and the revised wait: pull_ready, NotReady, pulls wake waiters (2026-09-17)"
kind: fact
tags:
- task_pull
- wait_secs
- adr-0028
- tiers
paths:
- src/tasks.rs
- src/state.rs
- src/lead.rs
- tests/http_roundtrip.rs
- tests/stdio_shim.rs
- docs/5-decisions/0028-claim-aware-task-pull.md
- docs/1-about/04-primitives.md
author: pull-fix
updated_by: pull-fix
created_at: 2026-09-17T15:34:13Z
updated_at: 2026-09-17T15:34:13Z
---

Task 7bc643e2, ADR-0028 "Revised 2026-09-17" section.

- [fact] `TaskBoard::choose` (private) ranks candidates in three tiers, order kept within each: free of claims and of other agents' in-progress task paths > overlapping only task paths (`waiting_on` = those task holds) > overlapping a live claim (first such; `waiting_on` = claim + task holds, sorted, deduped). Plain `pull` and `pull_ready` share it. Claim holds still come from `State` (`claim_holds`); task holds from `TaskBoard::task_holds`. Overlap is `blockers()` -> `RepoPath::claim_overlaps` (anchor-aware).
- [fact] `TaskBoard::pull_ready` / `State::task_pull_ready` -> `Result<Pulled, NotReady>`: assigns tiers 1-2 only. `NotReady::Held` when the choice is claim-held or there is no candidate but some task is `todo` (dependency-blocked). `NotReady::NoTodo` when no task is `todo` (in_progress/blocked/done only, or empty).
- [fact] `LeadPolicy::task_pull_waiting` loop: `watch_board()` first; past deadline -> plain `task_pull`; `Ok` -> return; `NoTodo` -> `None` at once; `Held` -> `watch.changed(..)`. The assignment is one sync call under the state lock, so ADR-0031 cancel-safety holds.
- [gotcha] `State::task_pull` and `task_pull_ready` now `tasks_changed.notify_waiters()` after a successful pull; without it a waiter behind a claim keeps waiting after another agent takes the last todo task (lead test `a_waiting_pull_ends_in_none_when_the_last_todo_task_is_taken`).
- [fact] Removed `State::task_candidates` and `TaskBoard::free_candidates` (no callers left).
- [gotcha] Tests that relied on a pull waiting on an EMPTY board had to get something to wait for: `hold_a_task` helpers in tests/http_roundtrip.rs (alice claims src/a.rs + task on it) and tests/stdio_shim.rs (holder + task `freed`). The shim tests' `cancels.len() == 2` proves the pull really waited. `returns_none_when_the_wait_ends` now uses a dependency-blocked board.
- [fact] Empty-board pull with wait_secs 120 answers in ~5-10 ms over HTTP (debug build); tests assert < 200 ms.
- [fact] examples/e2e/setup.sh's `--pull wait` probe used to time a pull on the empty board; bench-prep changed it to check the tool description instead.
