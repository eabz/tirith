---
id: 4cce35d5-365c-4048-b37e-20219d393018
permalink: store-and-state-durability-tiers-msrv-and-what-stays-per-primitive
title: "Store and state: durability tiers, MSRV, and what stays per primitive"
kind: gotcha
tags:
- persistence
- durability
- msrv
- review
paths:
- src/store.rs
- src/state.rs
- Cargo.toml
- scripts/check.sh
author: agent-4
updated_by: agent-4
created_at: 2026-09-16T04:11:36Z
updated_at: 2026-09-16T04:11:36Z
---

Findings from the agent-4 production review (task 0aa31db0, 2026-09-16).

- [gotcha] The durability tier of a log is decided by its path: `Durability::for_file` in store.rs returns Synced for anything under `runtime/` and Lazy otherwise (decision d9f6673a). Do not hard-code a tier at a `write_log` call site; put the file in the right directory instead. #durability
- [gotcha] `rust-version` in Cargo.toml is 1.90 because the tray crates (default feature, macOS) need it; rmcp 3.4 needs 1.88. Only 1.89, 1.91 and 1.98 toolchains are installed locally, so the check was `cargo +1.89 check` with and without default features. Re-verify with `cargo metadata` (max `rust_version` in the resolved graph) after any dependency bump. #msrv
- [fact] The per-primitive plumbing in state.rs (`LogCursor` for logs, `ChangedIds` for one-file-per-item, Option<Vec> for claims/tasks) is already the generic path; `is_dirty`, `take_dirty`, `mark_all_dirty` and `Delta::full` must each name every primitive, and that enumeration is the whole cost of adding one. Folding it further would change the pub `Delta` shape that the tests and the dashboard consume for no behaviour gain.
- [fact] `State::brief` computes its memory section on an `Arc<MemoryBook>` clone outside the lock, like `memory_search` and `memory_get`; notices, contracts and decisions still scan under the lock because `mark_seen` must happen there.
- [gotcha] CI runs only on ubuntu-latest, and the `tray` feature compiles to nothing off macOS, so `src/tray.rs` is never built by CI. A macOS job (or `cargo check --target aarch64-apple-darwin` is not enough, it needs the AppKit crates) is the gap to close if the tray keeps living in this crate.
- [gotcha] `scripts/check.sh` leaks step counts pgrep for `tirith serve --root` under `/T/` (macOS `$TMPDIR`) or `/tmp/` (Linux); a new tempdir layout needs the pattern updated.
