---
id: 48538341-345a-41a8-9174-8a80e93ec5e8
permalink: the-mcp-endpoint-keeps-sessions-scripts-use-the-cli-curl-examples-show-the-handshake
title: The MCP endpoint keeps sessions; scripts use the CLI, curl examples show the handshake
kind: decision
tags: []
paths:
- src/server.rs
- docs/2-examples/02-client-setup.md
- docs/1-about/02-architecture.md
author: head-dev-fable
updated_by: head-dev-fable
created_at: 2026-09-16T02:48:21Z
updated_at: 2026-09-16T02:48:21Z
---

Task 7d6f23be: the daemon keeps rmcp's session-based streamable HTTP (ADR-0002 stands); no NeverSessionManager. The documented way for a plain script to talk to Tirith is the CLI (`tirith claim ...`, `tirith memory search ...`), which already speaks the protocol and prints compact text or JSON. The curl example in docs/2-examples/02-client-setup.md is rewritten to show the real three-step handshake (initialize, read mcp-session-id, notifications/initialized, then tools/call with the header) and to say responses arrive as SSE `data:` frames; no claim that curl gets plain JSON.

## Rationale

Sessions are what the stdio shim, the benchmark clients and Claude Code rely on; changing the transport for a curl example is the wrong trade. The CLI exists precisely for scripts and other agents.

## Alternatives

- Stateless server (NeverSessionManager): breaks nothing today but forecloses server-initiated messages and changes ADR-0002 for a docs problem
- Add a plain JSON REST shim next to /mcp: a second protocol surface to keep in sync
