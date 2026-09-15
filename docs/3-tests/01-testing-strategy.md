# Testing strategy

Tirith is small and its correctness is mostly about rules: overlap,
expiry, dependency ordering, versioning. Those rules are pure functions and
are tested as such. The transport is tested once, end to end.

## Layers

| Layer | Where | What it covers | Runtime needed |
|---|---|---|---|
| Unit | `#[cfg(test)] mod tests` in each domain module | Overlap rules, lease expiry with a manual clock, path normalization, task dependency resolution, contract versioning | None |
| State | `src/state.rs` tests | Atomicity of multi-path claims, lazy reaping, renew-on-activity | None |
| Store | `src/store.rs` tests | Round-trip to a temp dir, atomic write survives a simulated crash, loading a corrupt file is an error, not a panic | tokio (for `spawn_blocking`) |
| Integration | `tests/*.rs` | Start the real server on an ephemeral localhost port, drive it with the rmcp client, assert on tool responses | tokio + localhost network |
| Demo | `examples/demo.sh` | Human-readable acceptance for each milestone | Built binary |

## Rules

- **No sleeping in tests.** Lease expiry is tested by advancing the
  injected `Clock`. If a test needs `sleep`, the code under test needs a
  clock parameter.
- **No shared global state.** Every test builds its own `State` and, for
  store tests, its own temp dir (`tempfile` crate).
- **Integration tests bind port 0** and read the assigned port back. Never
  hard-code 7477 in tests.
- **Every bug fix adds a test** that fails before the fix.
- **Every tool has at least one integration test** covering the happy path
  and one refusal or error path.
- `unwrap` and `expect` are allowed in test code only, via
  `#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]`.
- Test names say what is asserted: `directory_claim_blocks_file_inside_it`,
  not `test_claim_2`.

## Running

```bash
cargo test                       # everything
cargo test --lib                 # unit and state tests only, fastest
cargo test --test claims         # one integration file
cargo test -- --nocapture        # see server logs
```

CI runs, in order: `cargo fmt --check`, `cargo clippy --all-targets
--all-features -- -D warnings`, `cargo test --all-features`, `cargo doc
--no-deps` with `RUSTDOCFLAGS=-D warnings`, `cargo machete`, `cargo deny
check`. Coverage via `cargo llvm-cov` is reported, not gated.

## Coverage expectations

There is no numeric target. The expectation is qualitative: every rule in
[../1-about/04-primitives.md](../1-about/04-primitives.md) that says
"refused", "atomic", "expires", or "never" has a test whose name states it.
