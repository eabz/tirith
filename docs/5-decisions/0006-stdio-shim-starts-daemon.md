# ADR-0006: The stdio shim starts the daemon on demand

**Status:** Accepted, 2026-09-15

## Context

ADR-0002 made Tirith one daemon per repository. That leaves a setup step:
someone has to run `tirith serve` before any agent can work, and forget it
and every session fails. Tools like Serena have no such step because MCP
clients spawn stdio servers themselves, per session.

## Decision

`tirith stdio` is a stdio MCP server meant to be spawned by the client. On
start it reads `.tirith/runtime/daemon.json`, checks the daemon's health
endpoint, and if nothing healthy answers, spawns `tirith serve` detached
(new process group, output appended to `.tirith/runtime/serve.log`) and
waits for it. It then proxies `tools/list` and `tools/call` to the daemon
over streamable HTTP and mirrors the daemon's instructions. The daemon
keeps running after the session ends.

Client configuration becomes `command: tirith, args: [stdio]`, the same
shape as any other stdio MCP server.

## Alternatives

- **Ask users to run `tirith serve` first.** Rejected: the one thing that
  must never be forgotten should not be manual.
- **A Claude Code hook that starts the daemon.** Rejected: client-specific,
  and Tirith is framework-agnostic.
- **A system service (launchd, systemd).** Rejected for now: heavy for a
  per-repository daemon, and it does not follow the user between
  repositories. Still an option for always-on setups.
- **Make the shim itself the daemon, elected among sessions.** Rejected:
  leader election for a localhost tool is complexity without benefit.

## Consequences

- Zero-setup for every client that can spawn a process; HTTP registration
  remains available for clients that prefer it.
- Two sessions starting at once both try to spawn; only the one that binds
  the port writes `daemon.json`, and the other finds it within a second.
- The daemon is never stopped automatically. `tirith status` shows it and
  the pid is in `daemon.json`. Idle shutdown can be added later.
