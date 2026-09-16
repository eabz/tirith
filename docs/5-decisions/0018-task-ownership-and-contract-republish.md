# ADR-0018: Tasks belong to their owner until they go silent; contract republishes are guarded

**Status:** Accepted, 2026-09-16

## Context

The v1 coordination audit (ADR-0013) found two ways the board could lie.

`task_update` let any agent set `in_progress` or `done` on a task another
agent owned, silently reassigning it. At the same time nothing ever
reaped an `in_progress` task, so an agent that died holding one left its
whole dependency subtree unpullable forever.

`contract_publish` replaced `consumers` wholesale: republishing a name
without repeating the list wiped it, and the change notice then went to
nobody. Two agents publishing v2 of the same name concurrently both
succeeded, and neither was told.

## Decision

- **A foreign status change is a conflict.** Changing a task that is
  `in_progress` under another agent returns `conflict` with the owner and
  the time it last changed. `force: true` overrides and prepends
  `forced by <caller>: was in progress under <owner>` to the note, so the
  takeover is on the record.
- **Silent owners lose their tasks.** `State` records each agent's last
  call. Every access reaps, next to lease expiry, any `in_progress` task
  whose owner has been silent longer than a threshold, returning it to
  `todo` with a note naming the owner and the time. The threshold defaults
  to 1800 seconds, is set with `tirith serve --task-orphan-secs`, and 0
  disables it. `status` reports `tasks_orphaned`. Last activity is not
  persisted: after a restart an owner counts as silent since the task
  last changed, which is the conservative reading.
- **Republishing keeps consumers unless told otherwise.** Omitted
  `consumers` keeps the list; an explicit empty list clears it. The change
  notice goes to the union of old and new consumers.
- **`expected_version` guards concurrent publishes.** When set and the
  contract is at another version (0 meaning "must not exist"), the publish
  is refused with `conflict`, the current version, and its publisher. It
  is optional so scripts that do not care keep working.

## Alternatives

- **Reap tasks on lease expiry instead of a separate threshold.** Rejected;
  agents that hold no claims (planning, reviewing) still own tasks, and a
  lease is ten minutes by default while an honest task takes longer.
- **Persist last activity.** Rejected for now; it would put a write on
  every call, exactly what ADR-0010 removed. The restart fallback is safe.
- **Always require `expected_version`.** Rejected; it breaks every
  existing caller and most publishes are not contended.
- **Notify only the new consumers.** Rejected; a path that was dropped
  from the list needs to know the contract moved on without it.

## Consequences

- `TaskBoard::update` and `State::task_update` take `force`; `NewContract`
  carries `consumers: Option<Vec<RepoPath>>` and `expected_version`, and
  `Published` carries `notify`. Two new error variants map to `conflict`.
- A task can return to `todo` between two calls of a slow but live agent
  that made no call for half an hour. Its next `task_update` then sees the
  task unowned and simply takes it back; the note trail shows what
  happened.
- The CLI grew `task update --force`, `contract publish
  --expected-version`, and `serve --task-orphan-secs`.
