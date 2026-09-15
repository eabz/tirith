# Project structure

```
tirith/
├── AGENTS.md            Rules for coding agents. Read before editing.
├── CLAUDE.md            Claude Code entry point; imports AGENTS.md.
├── README.md            Overview and quick examples.
├── LICENSE              MIT.
├── Cargo.toml           Single crate: library + binary. Lints live here.
├── rustfmt.toml         Formatter config (defaults, pinned edition).
├── src/
│   ├── main.rs          CLI entry (clap).
│   ├── lib.rs           Crate root: module declarations, re-exports.
│   ├── server.rs        MCP tool surface (rmcp).
│   ├── state.rs         Shared in-memory state.
│   ├── store.rs         JSON persistence.
│   ├── clock.rs         Injectable time.
│   ├── claims.rs        Claims domain.
│   ├── tasks.rs         Task board domain.          (planned)
│   ├── contracts.rs     Contracts domain.           (planned)
│   ├── notices.rs       Change notices domain.      (planned)
│   └── decisions.rs     Decisions log domain.       (planned)
├── tests/               Integration tests (real server over localhost HTTP).
├── examples/
│   └── demo.sh          Two agents, overlapping claims, second refused.
├── docs/                All documentation. See docs/README.md.
│   ├── 1-about/
│   ├── 2-examples/
│   ├── 3-tests/
│   ├── 4-style/
│   ├── 5-decisions/
│   ├── 6-agent-workflow/
│   └── _static/images/
├── .memory/             Basic Memory notes, agent-managed, committed.
├── .serena/             Serena project config and memories.
├── .mcp.json            Claude Code project MCP servers (basic-memory).
├── .cursor/mcp.json     Cursor MCP servers (basic-memory).
└── .tirith/             Created at runtime in the *target* repo, and also
                         here once Tirith coordinates its own development.
```

## Where things go

| You are adding | Put it in |
|---|---|
| A new MCP tool | A method on `State` plus a domain function, then the mapping in `server.rs`, then the schema in `docs/1-about/04-primitives.md` |
| A domain rule (overlap, expiry, dependency ordering) | The domain module, with a unit test in the same file |
| A test that needs the HTTP server | `tests/` |
| A usage example | `docs/2-examples/` and, if short, `README.md` |
| A design decision | `docs/5-decisions/NNNN-title.md` |
| A picture | `docs/_static/images/` |
| A fact agents need that the code does not express | A Serena memory, or a Basic Memory note in `.memory/` for long-form |

## Single crate, for now

Tirith is one crate with a library (`src/lib.rs`) and a binary
(`src/main.rs`). The binary is thin: argument parsing and wiring. Everything
testable is in the library. A workspace split (`tirith-core`,
`tirith-server`, `tirith-cli`) is a small refactor if the crate ever earns
it; do not do it pre-emptively.
