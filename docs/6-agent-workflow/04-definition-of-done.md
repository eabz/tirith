# Definition of done

A change is done only when all of these hold. Never claim a step passed
without running it in this session.

- `cargo fmt --all -- --check` passes.
- `cargo clippy --all-targets --all-features -- -D warnings` passes.
- `cargo test --all-features` passes, and the change has tests: unit
  tests next to the code, integration tests in `tests/`
  ([../3-tests/01-testing-strategy.md](../3-tests/01-testing-strategy.md)).
- `cargo doc --no-deps` builds without warnings; every `pub` item is
  documented. `cargo machete` and `cargo deny check` pass.
- Documentation in `docs/` and `README.md` is updated when behavior, tool
  names, tool schemas, or the project layout changed. A tool's name or
  input schema never changes without
  [../1-about/04-primitives.md](../1-about/04-primitives.md), the
  examples, and the README changing with it.
- A new ADR exists in [../5-decisions/](../5-decisions/) when the change
  makes a design decision a future agent could otherwise re-decide.
- Every file edited was claimed through Tirith before the edit, a notice
  was published for every rename or signature change that affects callers
  outside the claimed files, and the claims were released at the end. The
  report names the claims held
  ([03-tirith-dogfooding.md](03-tirith-dogfooding.md)).
- A new dependency carries a one-line justification comment in
  `Cargo.toml` and is mentioned in the report. No lint is disabled to make
  CI pass; an `#[allow]` sits on the exact item, with a comment, and is
  mentioned in the report.
- No test makes a network call beyond the localhost round trips in
  `tests/`.
- The report is honest: what passed, what failed, what was skipped.

## Running the chain

`scripts/check.sh` runs the whole chain, one line per step, because full
test output read into an agent's context costs thousands of tokens. A
failing step prints a digest (panic and assertion messages, compiler
errors) and keeps the raw log in `target/check-<step>.log`; read the
digest before rerunning anything. While editing, run only the module's
tests (`cargo test --lib claims`) or `scripts/check.sh --quick` (unit
tests, then doctests, a few seconds). `--quick` is not done: run the full
chain once, before releasing your claims.

Commit only when asked ([../4-style/03-git-and-docs-conventions.md](../4-style/03-git-and-docs-conventions.md)).
