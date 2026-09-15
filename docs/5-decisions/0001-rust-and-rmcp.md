# ADR-0001: Rust with the official rmcp SDK

**Status:** Accepted, 2026-09-15

## Context

Tirith is a long-running daemon that idles for hours next to many agent
processes. It should cost almost nothing in memory and CPU, ship as one
binary, and start instantly. The author prefers Rust for server-side tools.
The MCP SDK choice is the main risk: the protocol still evolves.

## Decision

Rust, edition 2024, stable toolchain. MCP via `rmcp`, the SDK maintained
under the `modelcontextprotocol` GitHub organization, which supports both
streamable HTTP server and stdio transports and derives tool schemas from
types.

## Alternatives

- **TypeScript with the reference SDK.** Most battle-tested SDK and every
  MCP client is tested against it. Rejected for runtime footprint (a node
  process per daemon) and because the author wants small static binaries.
- **Go with the official Go SDK.** Single binary, easy concurrency.
  Rejected because it offers nothing over Rust here and its SDK is younger.
- **Python.** Fastest to prototype. Rejected for footprint and packaging.

## Consequences

- Small binary, low idle cost, strong types for contracts later.
- We track `rmcp` releases closely; protocol changes land there first.
- Contributors need a Rust toolchain. Acceptable for a single-author tool.
