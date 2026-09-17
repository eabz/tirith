---
id: b3e3c9fb-fb71-494c-a9d4-38138d9ba0de
permalink: claim-aware-pull-claim-wait-secs-and-contract-notice-coalescing-2026-09-17
title: Claim-aware pull, claim wait_secs, and contract notice coalescing (2026-09-17)
kind: fact
tags:
- tier0
- task_pull
- claim
- brief
- notices
paths:
- src/tasks.rs
- src/state.rs
- src/server.rs
- src/notices.rs
- tests/http_roundtrip.rs
- docs/1-about/04-primitives.md
- docs/5-decisions/0028-claim-aware-task-pull.md
author: core-sched
updated_by: core-sched
created_at: 2026-09-17T04:13:45Z
updated_at: 2026-09-17T04:13:45Z
---

Three Tier 0 changes landed by core-sched.

1. Claim-aware task_pull (ADR-0028). TaskBoard::pull(agent, holds: &[Hold], now) -> Option<Pulled {task, waiting_on}>. State builds holds from other agents' live claims (claim_holds in state.rs); TaskBoard adds other agents' in_progress task paths itself. First free candidate wins; if all held, the first candidate is assigned and waiting_on lists overlapping holds (sorted, deduped). Pathless tasks never held; caller's own claims/tasks never hold. State::task_candidates (Jev affinity input) is filtered to free candidates, so Jev's index>0 choice is always free. Sim replay of jevlab parallel_probe (49 done tasks): N=2 makespan -24% measured durations, -29% uniform; N=3 -13%.

2. claim wait_secs (max 120, MAX_CLAIM_WAIT_SECS in state.rs). State::claim_waiting is async: registers a tokio Notify (State.freed) with enable() before each atomic retry, then select!s on the notify or a sleep until the earliest overlapping expires_at + 20 ms, bounded by the deadline. freed.notify_waiters() fires on successful release and when access() reaps expired claims. Gotcha: the per-tool tools/list cap is 500 chars and claim was ~490 before; the description had to be shortened to fit (496 now). Gotcha: with a ManualClock, expiry wake-ups use wall time, so only release wakes are fast in unit tests.

3. Brief coalescing. notices::coalesce_contract_versions(newest_first) partitions into shown and superseded; only NoticeKind::Contract notices with a contract_id coalesce (agent-written notices naming a contract are kept: on this repo's data, coalescing by contract_id alone would hide 2 fyi behavior notices). State::brief marks superseded ids delivered only for contracts whose newest notice is in the shown (capped) rows. Contract "Brief on claim" is at v2. comms 42-pair replay: act 14/14, 40/42 rows shown.

Replay scripts: scratchpad core-sched/replay.py and comms_replay.py (session-local).
