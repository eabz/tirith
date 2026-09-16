# Tirith project status

- Tirith: Rust MCP coordination server for parallel coding agents (claims, task board, contracts, change notices, decisions log). Not a memory layer.
- Stack: Rust edition 2024, rmcp 3.4 (server + reqwest client), axum 0.8, single daemon per repo over streamable HTTP at 127.0.0.1:7477/mcp with a dashboard at `/`. In-memory `State` written through to JSON under `.tirith/` by a sequence-ordered `Persister`. ADRs in `docs/5-decisions/`.
- 0.1.0 pre-alpha built 2026-09-15: all 17 tools exist (`claim`, `release`, `renew`, `claims_list`, `task_*`, `contract_*`, `notice_*`, `decision_*`, `status`), CLI with one subcommand per tool (`src/cli.rs`, binary only), `examples/demo.sh` passes, 41 tests pass, clippy/doc/machete/deny clean.
- Layout: `src/server.rs` (tool inputs + outcome JSON + `start`), `src/state.rs` (lock, reap, touch, snapshots), `src/store.rs` (JsonStore + Persister), `src/dashboard.rs` + `dashboard.html`, `src/client.rs` (MCP client with NeverRetry transport), domain modules one per primitive, `src/types.rs` (AgentId, RepoPath, ids).
- Conventions that bite: every tool outcome is `{status: ok|conflict|not_found|none|invalid, ...}` and never an MCP error; any call by an agent renews its leases; `RepoPath` strips trailing `/` and overlap is ancestor-at-segment-boundary; `pub(crate)` default with `unreachable_pub` on; tests may `unwrap` via crate-level cfg_attr.
- Binding rules: `AGENTS.md` and `docs/4-style/01-rust-rules.md`. Tool schemas authoritative in `docs/1-about/04-primitives.md`.
- Tirith is registered in `.mcp.json` for this repo (HTTP); run `cargo run -- serve` first. Basic Memory notes in `.memory/`.
- Next: `tirith stdio` shim, use Tirith on itself, contract shape validation.
