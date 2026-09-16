---
id: a6a239e1-ce81-45c5-9a3d-325dee71fb61
permalink: basic-memory-retired-tirith-memory-notes-are-the-only-long-form-memory
title: Basic Memory retired; Tirith memory notes are the only long-form memory
kind: decision
tags: []
paths:
- .mcp.json
- .cursor/mcp.json
- .tirith/memory
- docs/6-agent-workflow/02-memory.md
- AGENTS.md
author: claude-ci-speedup
updated_by: claude-ci-speedup
created_at: 2026-09-16T01:52:57Z
updated_at: 2026-09-16T01:52:57Z
---

Remove the basic-memory MCP registration from .mcp.json and .cursor/mcp.json, delete .memory/, and move its two notes to .tirith/memory/design/ with kind: research and paths: added. Two layers remain: Serena for short navigation facts, Tirith memory notes for everything else. ADR-0012.

## Rationale

After one day .memory/ held two notes and every observation named a file, so all of it belonged in Tirith memory by the repo's own rule. A third layer cost a Python tool per machine, a second MCP server per session, and one more answer to "where does this go". Approved by eabz 2026-09-16.

## Alternatives

- keep Basic Memory for cross-project knowledge (personal tool, not a repo registration)
- keep it for semantic search (two notes; embeddings deferred in ADR-0011)
- keep .memory/ as an import inbox
