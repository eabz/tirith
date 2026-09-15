# Architecture

## One daemon per repository

MCP clients such as Claude Code and Cursor start a fresh stdio server for
every session. If Tirith were a normal stdio server, ten agents would get ten
processes with ten private states, and nothing would coordinate. Tirith is
therefore a single long-running process per repository, and every agent
connects to it over **streamable HTTP** on localhost.

```
                     ┌──────────────────────────┐
  Claude Code ──────▶│                          │
  Cursor ───────────▶│   tirith serve           │──▶ .tirith/  (JSON)
  LangGraph ────────▶│   http://127.0.0.1:7477  │
  plain script ─────▶│                          │
                     └──────────────────────────┘
```

**Planned:** a `tirith stdio` subcommand that speaks stdio to the client and
proxies to the daemon, starting it if needed, for clients that cannot use
HTTP. The daemon remains the only place state lives.

## Layers

```
src/main.rs        CLI (clap): serve | claim | release | status | stdio
src/server.rs      MCP surface (rmcp): tool definitions, input schemas,
                   response formatting. No business rules.
src/state.rs       In-memory State behind a lock. All mutation goes through
                   its methods. Reaps expired leases lazily on access.
src/claims.rs      Claim, overlap rules, lease logic. Pure, testable.
src/tasks.rs       Task board.                          (milestone 2)
src/contracts.rs   Contracts.                           (milestone 3)
src/notices.rs     Change notices.                      (milestone 3)
src/decisions.rs   Decisions log.                       (milestone 4)
src/store.rs       Persistence: JSON files under .tirith/, atomic writes.
src/clock.rs       Clock trait; real clock in prod, manual clock in tests.
```

Rules that keep the layers honest:

- `server.rs` maps a tool call to exactly one `State` method and formats the
  result. If you find yourself writing an `if` about domain rules there,
  it belongs in a domain module.
- Domain modules never import rmcp, axum, or tokio. They are plain Rust and
  are unit tested without a runtime.
- `State` is the only owner of mutable data. Domain types are values.

## Identity and leases

Agents identify themselves with a caller-supplied `agent` string on every
call. MCP sessions reconnect, and HTTP gives no reliable signal when a
client dies, so identity cannot hang off the transport.

Claims are leases with a TTL (default 10 minutes). Any call from the owning
agent renews the lease. Expired leases are removed lazily when state is
read, so a dead agent's claims disappear without a background thread. A
background tick may be added later purely to persist the cleanup.

## Path model

In milestone 1 a claimed path is either a file or a directory prefix (ending
in `/`). Two claims overlap when one path is equal to or a prefix of the
other. Globs are deliberately excluded: overlap between two arbitrary globs
is hard to decide and easy to get wrong. If globs arrive later they will
use a conservative rule (any shared literal prefix is a conflict).

Paths are relative to the repository root, normalized (no `./`, no `..`,
forward slashes). The server refuses paths that escape the root.

## Storage

State is small: tens of claims, hundreds of tasks, maybe thousands of
notices over a project's life. It is held in memory and written through to
JSON files under `.tirith/` in the target repository:

```
.tirith/
  runtime/            gitignored
    daemon.json       port, pid, started_at
    claims.json
    tasks.json
  contracts/          committed, one file per contract
  notices.jsonl       committed, append-only
  decisions.jsonl     committed, append-only
```

Committed files are human-readable and git-diffable on purpose: contracts
and decisions are exactly what a future session should inherit. A `Store`
trait isolates the format so SQLite could replace it without touching the
domain modules. See [../5-decisions/0003-json-file-storage.md](../5-decisions/0003-json-file-storage.md).

## Transport and port

Default bind is `127.0.0.1:7477`, path `/mcp`. Configurable with
`--bind`. The daemon writes its address to `.tirith/runtime/daemon.json` so
the CLI and the stdio shim can find it without configuration.

## Tool response shape

Every tool returns structured JSON with a top-level `status` field so agents
can branch without parsing prose:

```json
{ "status": "ok", ... }
{ "status": "conflict", "conflicts": [ ... ] }
{ "status": "not_found", "message": "..." }
{ "status": "invalid", "message": "..." }
```

A human-readable summary is also included as text content for clients that
show only text.
