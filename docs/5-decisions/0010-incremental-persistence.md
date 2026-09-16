# ADR-0010: Incremental persistence with a coalescing background writer

**Status:** Accepted, 2026-09-16. Refines ADR-0003; the file format and
layout are unchanged.

## Context

ADR-0003 wrote the whole state to disk after every tool call that changed
anything. Two things made "changed anything" almost every call: a lease
renewal counts as a change, and every call by an agent holding claims
renews its leases. So `claims_list`, `status`, and `notice_list` all
rewrote every file: claims, tasks, all notices, all decisions, and one
file per contract. Callers waited for their own write behind one lock.

Measured on 2026-09-15 (release build, macOS, 2000 notices, 500 tasks,
50 contracts): a flat ceiling of about 95 tool calls per second no matter
how many agents, and latency equal to the agent count times 10 ms. Ten
agents saw 125 ms per call, fifty saw 590 ms, two hundred saw 2.3 s. With
an empty store the ceiling was about 1000 calls per second. The in-memory
logic was sub-millisecond throughout. The cost grew with project history,
not with the size of the change.

## Decision

1. **Per-primitive deltas.** `State` tracks what changed since the last
   persist and hands the persister a `Delta`, not a `Snapshot`. Claims and
   tasks are small and are rewritten whole when they change. Contracts are
   written one file per changed contract. Notices and decisions are JSON
   Lines logs: new entries are appended; only an in-place edit (a notice
   acknowledgement) rewrites the log. Two helpers cover every primitive,
   `LogCursor` for logs and `ChangedIds` for one-file-per-item
   directories, so a new primitive picks one and adds a `Delta` field.
2. **Lease renewals are not dirty.** A renewal sets a `touched` flag that
   is folded into the next claims write. The persister ticks once a second
   to pick up renewals nobody else flushed, and shutdown writes them.
   Between ticks, a crash loses at most one second of renewals, which only
   makes leases expire slightly earlier than they would have.
3. **One background writer.** `Persister` is a task that drains deltas
   from `State` in sequence order. A tool call that mutated state calls
   `flush`, which wakes the task and waits until everything dirty at that
   moment is on disk; concurrent callers wait on the same write. A burst
   of 200 claims becomes one write of `claims.json`. Read-only calls never
   wait. After a failed write the state is marked fully dirty so the next
   attempt rewrites every file, and the error is shown as `persist_error`.

## Alternatives

- **Debounce the whole-snapshot write.** Fixes throughput but not the
  bytes written per change, which still grow with project history.
- **SQLite, redb, or sled.** Named in ADR-0003 as the fallback. Not needed:
  the delta design gives per-change writes of a few hundred bytes to a
  few kilobytes inside the JSON layout, and keeps contracts, notices, and
  decisions diffable in pull requests.
- **Persist renewals synchronously but only to `claims.json`.** Simpler
  than the tick, but still puts a disk write on every read by every
  agent, which is the common case in a swarm.

## Consequences

- A tool call returns after its change is on disk, and says so honestly:
  if the write failed, the result carries `persist_error` and its text
  line starts with `warning: not persisted`. Lease renewals are
  eventually consistent within one second.
- Two durability tiers, decided 2026-09-16 after measuring that Rust's
  `sync_all` is a full device flush on macOS (F_FULLFSYNC): runtime state
  (`claims.json`, `tasks.json`, `meta.json`) is fsynced before its
  rename, because nothing else holds a copy; every committed file
  (contract files, memory notes, the notices and decisions logs) is
  written, appended, or renamed without a per-file fsync, because git
  holds the content and a torn file or last line is reported at load
  rather than fatal. Every directory touched by an apply is fsynced once
  at the end, which makes the renames durable, and `meta.json` is written
  last. A batch of one hundred note writes therefore costs two or three
  device flushes instead of two hundred.
- Loading never refuses to start over one bad line or file: it is
  skipped, logged, reported as `load_errors` on `status`, `/api/health`,
  and the dashboard, and kept in place when the file is rewritten.
- `notices.jsonl` and `decisions.jsonl` are appended to, so a hand edit
  that removes the trailing newline is tolerated (one is added before the
  next append). Corrupt lines are still load errors, never panics.
- A restarted daemon seeds the writer's progress from the loaded sequence
  number, so a clean shutdown with nothing dirty returns immediately.
- `examples/swarm_bench.rs` reproduces the measurement above so regressions
  are visible. Run it before changing anything on the write path.
