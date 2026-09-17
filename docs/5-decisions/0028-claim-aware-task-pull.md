# ADR-0028: `task_pull` skips tasks whose paths another agent holds

**Status:** Accepted, 2026-09-17. Refines
[ADR-0018](0018-task-ownership-and-contract-republish.md).

## Context

`task_pull` handed out the highest-priority unblocked `todo` task without
looking at claims. When that task's `paths` overlapped another agent's
live claim, or the paths of a task another agent already had in
progress, the puller's first `claim` was refused and it sat in a
sleep-retry loop while a lower-priority task it could have done stayed on
the board. Replaying this repository's 49 finished tasks through a parallel
simulator, two agents finished 24% sooner (measured task
durations; 29% with uniform durations) when the pull skipped held tasks.

## Decision

- **A free task beats a held one.** Candidates keep their order (priority,
  then age), but `task_pull` takes the first one whose `paths` overlap
  neither another agent's live claim nor the `paths` of another agent's
  `in_progress` task. Overlap is the claim overlap rule (ADR-0005): equal
  paths, or one a directory prefix of the other.
- **The caller's own work never blocks it.** Its own claims and its own
  in-progress tasks are ignored.
- **A task with no `paths` is never held.** The board cannot know what it
  touches, so it is pulled in plain order.
- **When everything is held, the old pick still goes out, with
  `waiting_on`.** The first candidate is assigned as before, and the result
  adds `waiting_on`: the held paths that overlap it and who holds them,
  sorted and deduplicated. Returning `none` instead would hide work that is
  merely queued behind a lease.
- **Filtering happens in `State`, the choice in `TaskBoard`.** `State`
  turns live claims into holds; `TaskBoard::pull` adds in-progress task
  paths and picks. The tasks module stays pure and knows nothing of
  claims.

## Alternatives

- **Return `none` when every candidate is held.** Rejected: the simulator's
  idle agents did no better than waiting ones, and the agent loses the
  information of what it is queued behind.
- **Only look at claims, not in-progress task paths.** Rejected: an agent
  pulls first and claims later, so two agents pulling back to back would
  both get the same file before either claimed it.
- **Score overlap instead of filtering (fewer held paths wins).** Rejected
  for now: partial overlap still ends in a refused claim, and the
  deterministic rule is easy to predict and test.
