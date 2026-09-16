# Architecture

## One daemon per repository

MCP clients such as Claude Code and Cursor start a fresh stdio server for
every session. If Tirith were a normal stdio server, ten agents would get ten
processes with ten private states, and nothing would coordinate. Tirith is
therefore a single long-running process per repository, and every agent
connects to it over **streamable HTTP** on localhost.

```
  Claude Code ──▶ tirith stdio ──┐
  Cursor ───────▶ tirith stdio ──┤   ┌──────────────────────────┐
                                 ├──▶│   tirith serve           │──▶ .tirith/  (JSON)
  LangGraph ─────── HTTP ────────┤   │   http://127.0.0.1:7477  │
  plain script ──── HTTP ────────┘   └──────────────────────────┘
```

`tirith stdio` speaks stdio to the client and proxies to the daemon,
starting it if none is healthy, so clients that spawn servers per session
need no setup step. The daemon remains the only place state lives. See
[../5-decisions/0006-stdio-shim-starts-daemon.md](../5-decisions/0006-stdio-shim-starts-daemon.md).

## Layers

```
src/main.rs        Binary entry: runs cli::run.
src/stdio.rs       `tirith stdio`: finds, replaces, or starts the daemon,
                   proxies stdio to it.
src/cli.rs         CLI (clap): serve, stdio, tray, update, plus one
                   subcommand per tool. Binary only.
src/update.rs      `tirith update`: re-runs the release installer in place.
src/server.rs      MCP surface (rmcp): tool inputs, outcome formatting,
                   and `start`, which wires everything into one HTTP server.
src/dashboard.rs   `/` (embedded dashboard.html), `/logo.png`, `/api/state`,
                   `/api/health`.
src/registry.rs    Per-user registry of running daemons (every platform).
src/tray.rs        `tirith tray`, the macOS menu bar icon (feature `tray`).
src/client.rs      MCP client used by the CLI and integration tests.
src/state.rs       In-memory State behind a lock. All mutation goes through
                   its methods. Reaps expired leases and renews the caller's
                   leases on every access.
src/types.rs       AgentId, RepoPath, and the id newtypes.
src/claims.rs      Claim, overlap rules, lease logic. Pure, testable.
src/tasks.rs       Task board.
src/contracts.rs   Contracts with version history.
src/notices.rs     Change notices and the per-agent seen log.
src/decisions.rs   Decisions log.
src/memory.rs      Memory notes and the Markdown file format they are
                   stored in. Pure: no rmcp, axum, tokio, or I/O.
src/messages.rs    Agent-to-agent messages and their delivery marks.
src/store.rs       Persistence: JSON files under .tirith/, atomic writes
                   and appends, and the background Persister that writes
                   deltas in sequence order.
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
agent renews the lease, but a lease cannot live longer than four TTLs (at
most four hours) unless the agent calls `claim` or `renew` again, so
activity alone cannot hold a path forever. Expired leases are removed
lazily when state is read, so a dead agent's claims disappear without a
background thread, and the former owner is told in its next response
([ADR-0015](../5-decisions/0015-lease-loss-and-max-age.md)). Renewals are
not written on the request path; the persister's one-second tick picks
them up (see Storage below).

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
    serve.log         daemon output, when the stdio shim started it
    meta.json         persist sequence number
    claims.json
    tasks.json
    notice_seen.jsonl which notice reached which agent, one line each
    messages.jsonl    agent messages, pruned to 24 hours on load
  contracts/          committed, one file per contract (<slug>-<id>.json)
  memory/             committed, one Markdown file per note; a permalink
                      with `/` segments becomes a subdirectory
  notices.jsonl       committed, one notice per line
  decisions.jsonl     committed, one decision per line
```

Writes are incremental. `State` tracks what changed since the last write
and produces a `Delta`: claims and tasks whole when they changed, only the
contracts that changed, and for each append-only log either the new lines
to append or, after a persist failure, a full rewrite. Notice rows are
never edited in place: a delivery goes to the runtime seen log
([ADR-0021](../5-decisions/0021-notice-acks-log.md)). A single background
`Persister` task drains deltas in sequence order. A tool call that
mutated state waits until its change is on disk; concurrent callers wait
on the same write, so a burst of claims from a swarm becomes one write of
`claims.json`. Read-only calls never wait, and
lease renewals are folded into the next claims write or picked up by a
one-second tick. A failed write is logged, surfaced as `persist_error` in
`status` and on the dashboard, and followed by a full rewrite on the next
attempt; the in-memory state stays authoritative. Measured with
`examples/swarm_bench.rs`: 200 agents on persistent sessions see single-
digit millisecond medians, where the previous whole-snapshot-per-call
design saw seconds. See
[../5-decisions/0010-incremental-persistence.md](../5-decisions/0010-incremental-persistence.md).

Committed files are human-readable and git-diffable on purpose: contracts
and decisions are exactly what a future session should inherit. A `Store`
trait isolates the format so SQLite could replace it without touching the
domain modules. See [../5-decisions/0003-json-file-storage.md](../5-decisions/0003-json-file-storage.md).

## Dashboard

The same HTTP server serves a read-only dashboard at `/` and its data at
`/api/state`. The page is a single embedded HTML file with no external
assets: it polls every two seconds and shows the Tirith logo, count tiles,
agents, claims with lease progress bars, the task board with status
filters, contracts, change notices, decisions, and memory notes. A text
filter narrows every table, and the page follows the system light or dark
theme with a manual toggle. The
logo is served from `/logo.png`, embedded from `src/dashboard-logo.png`
(a 192px cut of `docs/_static/images/logo.jpeg`), and doubles as the
favicon.
`/api/health` answers as long as the daemon is up; the stdio shim probes it
to decide whether the daemon recorded in `daemon.json` is still alive.

## Transport and port

Default bind is `127.0.0.1:7477`, path `/mcp`. Configurable with
`--bind`. The daemon writes its address to `.tirith/runtime/daemon.json` so
the CLI and the stdio shim can find it without configuration.

## Stopping

`tirith serve` stops on ctrl-c, on SIGTERM, or on its own when the
repository root it serves disappears (checked every two seconds), so a
daemon never outlives its repository. Shutdown is in this order: stop
accepting, give open connections two seconds, then close whatever is still
open (every stdio shim holds an SSE stream that would otherwise keep the
process alive indefinitely), sync everything pending to disk including
lease renewals, and finally remove `daemon.json`, but only if it still
records this process's pid. A record written by a newer daemon that took
the port meanwhile is left alone. The shim's restart path (ADR-0016)
relies on this: it waits for the old pid to exit before starting a
replacement.

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

The JSON travels once, as the result's structured content. The text
content block is a single line under 200 characters that starts with the
status, for example `ok: claimed src/a.rs until 02:10:00Z` or `conflict:
overlapping claims held by other agents`. It is never the JSON repeated:
clients feed text blocks to the model, so a copy would double the token
cost of every call. `tirith::client` reads the structured content and
falls back to the text only when a server sends none.

Two things ride on results without being asked for, because the next
result is the one delivery path that reaches every agent without polling.
A successful `claim` carries a brief: the unread notices, contracts,
decisions, and memory notes for the claimed paths, five newest each, as
compact rows ([ADR-0014](../5-decisions/0014-brief-on-claim.md)). Any
result may carry `lost`, the leases the caller held that ended since its
last call, and `inbox`, the messages waiting for it
([ADR-0015](../5-decisions/0015-lease-loss-and-max-age.md),
[ADR-0020](../5-decisions/0020-agent-messages.md)). Both are omitted when
empty, so the common call costs nothing extra. Field shapes are in
[04-primitives.md](04-primitives.md).

Every tool's input schema is post-processed in `tools/list`: the
`$schema` URL, `default: null`, integer `format` and `minimum`, the
nullable type unions on optional fields, and per-parameter descriptions
are dropped. Each tool keeps a one-sentence description that carries the
semantics a parameter name does not, such as the allowed `kind` values.
Agents download every schema once per session, so `tests/budgets.rs` and
`tests/http_roundtrip.rs` pin the whole listing under 7,800 characters
and each tool under 500 ([ADR-0017](../5-decisions/0017-tool-result-and-schema-budget.md)).
Parameter semantics live in [04-primitives.md](04-primitives.md) and in
the input structs' doc comments, not on the wire.
