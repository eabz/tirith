# ADR-0003: In-memory state written through to JSON files

**Status:** Accepted, 2026-09-15

## Context

State is tiny: tens of claims, hundreds of tasks, a few thousand notices
over a project's life. Some of it (contracts, notices, decisions) should
outlive the daemon and be inherited by future sessions, ideally through
git. Some of it (claims, task board) is runtime-only.

## Decision

The daemon holds all state in memory behind a lock and writes through to
files under `.tirith/` in the target repository. Runtime state goes in
`.tirith/runtime/` (gitignored). Contracts, notices, and decisions are
committed as JSON and JSONL. Writes are atomic (write to temp file, rename).
A `Store` trait hides the format.

## Alternatives

- **SQLite via rusqlite (bundled).** Robust and queryable. Rejected for v1:
  a binary file is not diffable or reviewable in a pull request, and it
  adds a C build dependency. Remains the fallback if JSON proves limiting.
- **redb or sled.** Pure Rust embedded stores. Same diffability objection.
- **Memory only.** Simplest. Rejected because a daemon restart would drop
  every claim while agents are still running.

## Consequences

- Contracts and decisions show up in pull requests as readable diffs.
- The store must tolerate hand edits and partial files: loading a corrupt
  file is an error with a clear message, never a panic.
- If a future primitive needs real queries, the `Store` trait is the seam.
