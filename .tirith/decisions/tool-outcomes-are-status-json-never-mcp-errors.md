---
id: 8cb8bf32-02c9-44a7-8c72-7257cf837c0a
permalink: tool-outcomes-are-status-json-never-mcp-errors
title: Tool outcomes are status JSON, never MCP errors
kind: decision
tags: []
paths:
- src/server.rs
author: claude-scaffold
updated_by: claude-scaffold
created_at: 2026-09-16T00:15:57Z
updated_at: 2026-09-16T00:15:57Z
---

Every tool returns structured JSON with a top-level status (ok, conflict, not_found, none, invalid); refusals are normal results so agents branch on status

## Rationale

Clients that only see isError cannot distinguish a refused claim from a broken server

## Alternatives

- Return MCP isError for conflicts
