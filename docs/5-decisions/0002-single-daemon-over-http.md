# ADR-0002: One daemon per repository over streamable HTTP

**Status:** Accepted, 2026-09-15

## Context

MCP clients launch stdio servers per session. Coordination requires that
all agents on a repository share one state. A per-session process cannot
provide that.

## Decision

Tirith runs as one process per repository, bound to `127.0.0.1:7477` by
default, speaking MCP streamable HTTP at `/mcp`. Every client connects to
that URL. A `tirith stdio` shim (planned) proxies stdio clients to the
daemon and starts it if it is not running.

## Alternatives

- **Stdio server with shared storage on disk.** Each session would run its
  own process and coordinate through files with locks. Rejected: file
  locking across processes is fragile, leases need a single clock, and
  every process would re-read state on every call.
- **Stdio server that elects a leader.** Too much machinery for the gain.
- **Remote hosted server.** Not needed for one machine and adds auth,
  TLS, and latency.

## Consequences

- Clients need HTTP MCP support. All targeted clients have it.
- The daemon must be started once per repo; the shim removes that friction.
- One address per repo means two repos need two ports; the daemon records
  its address in `.tirith/runtime/daemon.json` for discovery.
