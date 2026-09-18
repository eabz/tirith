---
id: 32cc37c4-d719-458c-a120-b4ee477f38e8
permalink: tool-definitions-are-complete-on-the-wire-guide-is-the-23rd-tool-adr-0034
title: Tool definitions are complete on the wire; guide is the 23rd tool (ADR-0034)
kind: decision
tags: []
paths:
- src/server.rs
- src/output_schemas.rs
- src/guide.rs
- tests/budgets.rs
- docs/1-about/04-primitives.md
author: claude-tdqs-guide
updated_by: claude-tdqs-guide
created_at: 2026-09-18T22:30:49Z
updated_at: 2026-09-18T22:30:49Z
---

tools/list sends a description for every parameter, enums on closed string params, MCP annotations and an output schema per tool, and the guide tool explains the protocol. Budget 44,500 chars, 3,600 per tool.

## Rationale

Owner's ranking, 2026-09-18: meeting every requirement of the TDQS rubric outranks the per-session size of tools/list. Do not re-argue it from byte counts alone; supersede ADR-0034 instead.

## Alternatives

- Stay at ADR-0033: annotations and usage clauses only, 10.4 KB
- Strict output schemas or Rust enums for kind/status: rejected, they would turn status outcomes into client or MCP errors
