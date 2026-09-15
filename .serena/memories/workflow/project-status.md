# Tirith project status

- Tirith: Rust MCP coordination server for parallel coding agents (claims, task board, contracts, change notices, decisions log). Not a memory layer.
- Stack decided 2026-09-15: Rust edition 2024, rmcp 3.x, single daemon per repo over streamable HTTP at 127.0.0.1:7477/mcp, in-memory state written through to JSON under `.tirith/`. ADRs in `docs/5-decisions/`.
- Milestone 1 (in progress): `claim`, `release`, `renew`, `claims_list` tools, `tirith` CLI, `examples/demo.sh` where two agents claim overlapping files and the second is refused.
- Binding rules for agents: `AGENTS.md` (summary) and `docs/4-style/01-rust-rules.md` (full). Tool schemas are authoritative in `docs/1-about/04-primitives.md`.
- No Rust code exists yet as of 2026-09-15; docs and rules were written first. Run Serena `onboarding` after the crate is scaffolded.
- Memory layers: Serena memories for short facts; Basic Memory proposed (not configured) for long-form notes, see `docs/6-agent-workflow/02-memory.md`.
