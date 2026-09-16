# ADR-0004: Agent identity is a caller-supplied string with TTL leases

**Status:** Accepted, 2026-09-15. Refined by
[ADR-0015](0015-lease-loss-and-max-age.md): a reaped lease is reported to
its former owner in the next response, and activity alone cannot hold a
lease past four TTLs.

## Context

Claims must expire when an agent dies. HTTP sessions reconnect and give no
reliable death signal. Frameworks differ in how they expose session or
process identity to tools.

## Decision

Every tool takes an `agent` string chosen by the caller. Claims are leases
with a TTL (default 600 s, max 3600 s). Any call from the owning agent
renews all its leases. Expired leases are reaped lazily whenever state is
read. Nothing depends on MCP session identity.

## Alternatives

- **Bind identity to the MCP session.** Rejected: sessions drop and
  reconnect for reasons unrelated to agent death, and stdio-shim clients
  would all look alike.
- **OS process tracking.** Rejected: the daemon cannot see remote or
  containerized agents, and it breaks framework-agnosticism.
- **No expiry, explicit release only.** Rejected: dead agents would hold
  files forever.

## Consequences

- Agents must pick a stable, unique name. Two agents sharing a name share
  claims. This is documented in every client setup.
- A long-running agent that makes no Tirith calls for longer than its TTL
  loses its claims. Agents doing long work should call `renew`.
- Spoofing is trivial and accepted: Tirith runs on one developer's machine
  among cooperating agents.
