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
src/main.rs        Binary entry: runs cli::run.
src/cli.rs         CLI (clap): serve, plus one subcommand per tool. Binary only.
src/server.rs      MCP surface (rmcp): tool inputs, outcome formatting,
                   and `start`, which wires everything into one HTTP server.
src/dashboard.rs   `/` (embedded dashboard.html) and `/api/state`.
src/client.rs      MCP client used by the CLI and integration tests.
src/state.rs       In-memory State behind a lock. All mutation goes through
                   its methods. Reaps expired leases and renews the caller's
                   leases on every access.
src/types.rs       AgentId, RepoPath, and the id newtypes.
src/claims.rs      Claim, overlap rules, lease logic. Pure, testable.
src/tasks.rs       Task board.
src/contracts.rs   Contracts with version history.
src/notices.rs     Change notices with acknowledgements.
src/decisions.rs   Decisions log.
src/store.rs       Persistence: JSON files under .tirith/, atomic writes,
                   and the Persister that serializes writes by sequence.
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

A claimed path is a repo-relative file or directory; a trailing `/` is
accepted and stripped. Two paths overlap when they are equal or one is an
ancestor directory of the other at a segment boundary, so `src/auth`
overlaps `src/auth/login.rs` but not `src/authz`. Globs are deliberately
excluded: overlap between two arbitrary globs
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
  .gitignore          written by tirith; ignores runtime/
  runtime/            gitignored
    daemon.json       mcp url, dashboard url, pid, started_at, version
    meta.json         snapshot sequence number
    claims.json
    tasks.json
  contracts/          committed, one file per contract (<slug>-<id>.json)
  notices.jsonl       committed, one notice per line
  decisions.jsonl     committed, one decision per line
```

Every tool call that changed something takes a snapshot with a bumped
sequence number and hands it to a persister that writes the files in the
background, in order, skipping any snapshot older than the last one
written. A failed write is logged and surfaced as `persist_error` in
`status` and on the dashboard; the in-memory state stays authoritative.

Committed files are human-readable and git-diffable on purpose: contracts
and decisions are exactly what a future session should inherit. A `Store`
trait isolates the format so SQLite could replace it without touching the
domain modules. See [../5-decisions/0003-json-file-storage.md](../5-decisions/0003-json-file-storage.md).

## Dashboard

The same HTTP server serves a read-only dashboard at `/` and its data at
`/api/state`. The page is a single embedded HTML file that polls every two
seconds and shows agents, claims with lease countdowns, the task board,
contracts with their current shape, change notices, and decisions.

## Transport and port

Default bind is `127.0.0.1:7477`, path `/mcp`. Configurable with
`--bind`. The daemon writes its address to `.tirith/runtime/daemon.json` so
the CLI and the stdio shim can find it without configuration.

## Tool response shape

Every tool returns structured JSON with a top-level `status` field so agents
can branch without parsing prose:

```json
{ "status": "ok", ... }
{ "status": "conflict", "conflicts": [ ... ], "message": "..." }
{ "status": "not_found", "message": "..." }
{ "status": "none", "message": "..." }
{ "status": "invalid", "message": "..." }
```

These are all successful tool results at the MCP level (`isError` is
false); a refused claim is a normal outcome, not a protocol error.

A human-readable summary is also included as text content for clients that
show only text.
