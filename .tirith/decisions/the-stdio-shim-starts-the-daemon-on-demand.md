---
id: 2b726932-b541-40c6-ab9a-ea58e84bd01b
permalink: the-stdio-shim-starts-the-daemon-on-demand
title: The stdio shim starts the daemon on demand
kind: decision
tags: []
paths:
- src/stdio.rs
- src/cli.rs
author: claude-stdio-shim
updated_by: claude-stdio-shim
created_at: 2026-09-16T01:05:23Z
updated_at: 2026-09-16T01:05:23Z
---

tirith stdio is spawned by the client per session, starts tirith serve detached if no healthy daemon is recorded in .tirith/runtime/daemon.json, and proxies tools/list and tools/call to it; the daemon is never auto-stopped

## Rationale

Removes the manual 'run tirith serve first' step, matching how Serena and other stdio MCP servers work; framework-agnostic unlike client hooks

## Alternatives

- Claude Code SessionStart hook
- launchd/systemd service
