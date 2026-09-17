# ADR-0028: `task_pull` skips tasks whose paths another agent holds

**Status:** Accepted, 2026-09-17. Refines
[ADR-0018](0018-task-ownership-and-contract-republish.md). Revised
2026-09-17: in-progress task paths rank a task below free ones but no
longer make a pull wait, and an empty board answers at once; see
[Revised 2026-09-17](#revised-2026-09-17-what-holds-a-task-and-when-a-pull-waits),
which takes precedence over the decision below where they differ.

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

## Revised 2026-09-17: what holds a task, and when a pull waits

### Context

`task_pull` gained `wait_secs` (a long poll; cancel rules in
[ADR-0031](0031-waits-end-when-the-caller-goes.md)), which waited while no
candidate was free under the rule above. The benchmark dry run (Forge
scenario, fake workers) showed two costs:

- **Pulls serialized on hub files.** Every Forge task lists
  `config`, `router` or `__init__` in its `paths`, and every hubs task a
  hub file. Once one agent had a task in progress, every other candidate
  was held by its task paths, and a worker with `wait_secs` waited up to
  120 s behind it, however short that agent's edit window on the hub.
  Edit windows and symbol claims
  ([ADR-0029](0029-symbol-anchored-claims.md)) exist so agents can share
  such files; the pull defeated both.
- **Workers idled at the end.** With no `todo` task left, a pull with
  `wait_secs: 120` waited the full 120 s before answering `none`: the last
  call came at 198 s, the last task was done at 77 s.

### Decision

- **Two kinds of hold.** Another agent's live claim is a lock: a `claim`
  on an overlapping path is refused. Another agent's `in_progress` task
  paths are a forecast: that agent will probably edit those files, under
  claims of its own, at some point.
- **Three tiers.** Candidates keep their order (priority, then age)
  within each tier, and the pull takes the first of the highest tier:
  1. free of both kinds of hold;
  2. overlapping only other agents' in-progress task paths: assigned with
     `waiting_on` listing those paths and owners;
  3. overlapping a live claim, only when every candidate does: the first,
     with every overlapping claim and task path in `waiting_on`.

  Overlap is `RepoPath::claim_overlaps`, so a symbol anchor holds its
  whole file but not a sibling anchor. A pull without `wait_secs` uses the
  same tiers. The caller's own claims and tasks, and tasks with no
  `paths`, still never hold.
- **A pull with `wait_secs` waits only for what waiting can change.** It
  takes a tier 1 or tier 2 task at once. It waits while every candidate
  overlaps a live claim, or while the only `todo` tasks wait on
  unfinished dependencies (they are not candidates, but a `done` can make
  them one). With no `todo` task at all it answers `none` at once, and a
  pull already waiting ends with `none` as soon as the last `todo` task is
  taken: a successful pull now wakes waiting pulls. At the deadline it
  returns what a plain pull returns then.
- **Cancel-safety is unchanged.** Each attempt takes a task only in one
  synchronous call under the state lock (`State::task_pull_ready`), never
  across an await, so a cancel or disconnect at any await assigns nothing
  (ADR-0031).

### Alternatives

- **Keep waiting behind in-progress task paths, with a shorter cap.**
  Rejected: every second spent behind a forecast is idle, and a shorter
  cap still serializes pulls on shared files.
- **Ignore in-progress task paths.** Rejected, for the reason in the
  original decision: two agents pulling back to back would take tasks on
  the same file while a free one sits on the board. Tier 1 still prefers
  the free task.
- **Keep the long poll over an empty board, to catch tasks created
  later.** Rejected: boards are seeded before workers start, and a worker
  with nothing left should finish, not hold a connection for two minutes.
  A task that returns to `todo` later is taken by the next pull.

### Consequences

- An agent can be handed a task whose files another agent is working on;
  `waiting_on` names them, and its first `claim` on a shared file may need
  `wait_secs`.
- A worker loop that used a long poll on an empty board to wait for new
  tasks now gets `none` at once and must pull again later. The e2e
  `--pull wait` setup probe stopped timing an empty-board pull.
- `tests/http_roundtrip.rs` covers an empty board answering at once, a
  task behind another agent's task paths assigned at once, a claimed task
  waited for and assigned on release, and a dependency-blocked board
  waiting until the dependency is done.
