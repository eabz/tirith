# Testing strategy

Tirith is small and its correctness is mostly about rules: overlap,
expiry, dependency ordering, versioning. Those rules are pure functions and
are tested as such. The transport is tested once, end to end.

## Layers

| Layer | Where | What it covers | Runtime needed |
|---|---|---|---|
| Unit | `#[cfg(test)] mod tests` in each domain module | Overlap rules, lease expiry with a manual clock, path normalization, task dependency resolution, contract versioning | None |
| State | `src/state.rs` tests | Atomicity of multi-path claims, lazy reaping, renew-on-activity, which primitives a delta carries, renewals folded into the next delta | None |
| Store | `src/store.rs` tests | Round-trip to a temp dir, deltas append or rewrite only their own files, the persister coalesces bursts and flushes, reports and recovers from a failed write, loading a corrupt file is an error, not a panic | tokio (for `spawn_blocking`) |
| Integration | `tests/http_roundtrip.rs` | Start the real server on an ephemeral localhost port, drive it with `tirith::client`: refusal and release, restart, lease expiry, the task/contract/notice flow, paging and id prefixes, task ownership and contract republish guards, claim-aware `task_pull` and its `wait_secs` wait, lost leases and the four-TTL cap, briefs, messages, shutdown with an open SSE stream, the daemon registry, the dashboard, and the two budgets also pinned in `tests/budgets.rs` | tokio + localhost network |
| Escalations | `tests/lead_escalation.rs` | The deterministic lead policy (ADR-0027) through a real daemon: each trigger (blocked task, third refusal of a claim, a message to `human`), routing to the lead (tagged by the human rules) or the human queue with and without a lead, messages to the lead never escalating (with the done report that once reached the human), answering an item with `POST /api/human/{id}/done` and a reply, and the outcome filled in when an escalation is answered | tokio + localhost network |
| Anchored claims | `tests/anchored_claims.rs` | Experimental `path#Symbol` claims (ADR-0029) through a real daemon: conflicts with the file and enclosing symbols only, `wait_secs` waking on an anchor release, a whole-file waiter not queued ahead of new anchors (today's behavior), `task_pull` holds, and briefs matching on the file | tokio + localhost network |
| Budgets | `tests/budgets.rs` | One test per row of the ADR-0013 table against a daemon seeded with 300 claims, notices and decisions: `tools/list` size, status-line text blocks, 20 compact rows with a cursor, `status`, `claims_list`, `renew`, brief, and search rows without bodies | tokio + localhost network |
| Persistence | `tests/persistence.rs` | The legacy decisions.jsonl importing into one file per decision, fifty agents claiming at once, reads that do not rewrite logs, lease renewals reaching disk by shutdown, seen marks that never rewrite the notice log, a bad line reported instead of stopping the daemon, a failed write reported on the response, everything surviving a restart | tokio + localhost network |
| Memory | `tests/memory_layer.rs` | One Markdown file per note round-tripped through a directory, an edit rewriting exactly one file, hand-written and corrupt files, folders in permalinks, and every note the repository ships in `.tirith/memory/` parsing; then the memory tools through the daemon: write, read, search bounds, relations, a claim carrying its notes, a restart | tokio (+ localhost network for the tool half) |
| Shim | `tests/stdio_shim.rs` | Spawn the built binary as `tirith stdio` the way a client would, speak JSON-RPC over its pipes: it starts one daemon and a second shim reuses it, replaces a daemon of another version or a dead record, and leaves another repository's daemon alone | tokio + localhost network + built binary |
| Demo | `examples/demo.sh` | Human-readable acceptance for each milestone | Built binary |

## Rules

- **No sleeping in tests.** Lease expiry is tested by advancing the
  injected `Clock`. If a test needs `sleep`, the code under test needs a
  clock parameter.
- **No shared global state.** Every test builds its own `State` and, for
  store tests, its own temp dir (`tempfile` crate).
- **Integration tests bind port 0** and read the assigned port back. Never
  hard-code 7477 in tests.
- **A spawned binary stays off the user's machine state.** A test that
  runs the built `tirith` sets `TIRITH_NO_TRAY=1` and points
  `TIRITH_STATE_DIR` at its temp directory (`tirith(root)` in
  `tests/stdio_shim.rs`), so its daemons start no menu bar tray and never
  register in the user's daemon registry (ADR-0032). No test starts a real
  tray; `AppKit` needs a GUI session.
- **Every bug fix adds a test** that fails before the fix.
- **Every tool has at least one integration test** covering the happy path
  and one refusal or error path.
- `unwrap` and `expect` are allowed in test code only, via
  `#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]`.
- Test names say what is asserted: `directory_claim_blocks_file_inside_it`,
  not `test_claim_2`.

## Reference numbers

Measured 2026-09-16 by the head-developer session on an Apple M1 Pro,
release build, daemon in-process in a temp repository seeded with 2000
notices, 500 tasks, 50 contracts and 200 decisions, one MCP session per
agent, zero think time. Each agent round is `claim`, `notice_list`,
`contract_list`, `decision_list`, `claims_list`, `status`, `memory_write`,
`memory_search`, `notice_publish` (every third round), `decision_record`
(every fifth) and `release`. "Live" is the working tree as built at
02:30Z, after ADR-0010, ADR-0011 and ADR-0017 but before list paging,
swarm-safe `status`, digest search rows and brief on claim, which landed
later that day. The table is the recorded baseline; ADR-0013 asks for a
rerun before the 1.0.0 release.

| Metric | 0.1.3, 1 agent | Live, 1 agent | Live, 100 agents | Live, 1000 agents |
|---|---|---|---|---|
| Throughput | 76 calls/s | 132 calls/s | 733 calls/s | 936 calls/s |
| Read p50 (`claims_list`) | 12.3 ms | 0.3 ms | 60 ms | 405 ms |
| Mutation p50 (`claim`) | 12.5 ms | 17.1 ms | 48 ms | 77 ms (p95 3.4 s) |
| `memory_write` p50 | n/a | 25.6 ms | 314 ms | 3.4 s |
| `memory_search` p50 | n/a | 0.6 ms | 263 ms | 1.0 s |
| Errors | 0 | 0 | 0 | 0 |
| `tools/list` | 10,455 B | 6,060 B | 6,060 B | 6,060 B |
| `notice_list`, one module | 31.0 KB | 14.7 KB | 15.2 KB | 17.5 KB |
| `claims_list` | 0.6 KB | 0.3 KB | 16.8 KB | 97 KB (max 216 KB) |
| `status` | 0.8 KB | 0.4 KB | 9.3 KB | 54 KB (max 120 KB) |
| `memory_search` | n/a | 6.0 KB | 4.3 KB | 10.5 KB |
| Bytes per agent per round | 38 KB (~9.5k tok) | 27 KB (~6.6k tok) | 50 KB (~12.6k tok) | 186 KB (~46k tok) |
| On-disk growth for the run | 0.7 KB | 10 KB, 21 files | 265 KB, 501 files | 1.25 MB, 2001 files |

What the numbers say: reads no longer wait on disk (ADR-0010), and the
result-once change (ADR-0017) halved the bytes of every call. Mutations
pay the flush, about 17 ms alone and a queue under load; `memory_write`
pays one fsync per note file on top (task fc218faa). The remaining bytes
are unpaged lists and the full-board `status` and `claims_list`, which
grow with the number of live agents (tasks 89b35cb6, d50d3346), plus
search rows that carry bodies (task 6137f8af). The load benchmark that
produced this table was removed with ADR-0035 and will be rebuilt; the
table stays as the recorded baseline.

## Running

```bash
scripts/check.sh                 # the whole chain, one line per step
scripts/check.sh --quick         # unit tests, then doctests; edit loop only
cargo test 2>&1 | scripts/check.sh --digest   # digest any cargo output
cargo test                       # everything
cargo test --lib                 # unit and state tests only, fastest
cargo test --test http_roundtrip # one integration file
cargo test -- --nocapture        # see server logs
```

`scripts/check.sh` is the definition-of-done chain from AGENTS.md and the
same order CI runs: `cargo fmt --check`, `cargo clippy --all-targets
--all-features -- -D warnings`, `cargo test --all-features`, `cargo doc
--no-deps` with `RUSTDOCFLAGS=-D warnings`, `cargo machete`, `cargo deny
check`; it also fails if a test left a `tirith serve` daemon running. It
exports `TIRITH_NO_TRAY=1`, so no daemon a test starts can launch a tray.
There is no coverage step. Each step's raw output goes to
`target/check-<step>.log`. A failing clippy, test, or doc step prints a
digest under its `FAIL` line: the first panic per location with its
assertion message (`left`/`right`), the first compiler error per message
with the other locations on one line, and the failed tests per target. If
nothing parses it prints the last 40 lines of the log instead.

`--quick` runs `cargo test --lib`, then `cargo test --doc`, and stops at
the first failure. The unit and doc tests take a couple of seconds after an
incremental build and caught 10 of the 11 mutants the whole suite detected
in the 2026-09-16 verification probe; the integration targets caught one
more and take 20 s or more. Use it while editing; the full chain is still
the definition of done.

## Coverage expectations

There is no numeric target. The expectation is qualitative: every rule in
[../1-about/04-primitives.md](../1-about/04-primitives.md) that says
"refused", "atomic", "expires", or "never" has a test whose name states it.
