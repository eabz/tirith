---
id: f384dff8-8b93-4491-a46b-2f02c2c6a927
permalink: tool-results-travel-once-tools-list-has-a-byte-budget
title: Tool results travel once; tools/list has a byte budget
kind: decision
tags: []
paths:
- src/server.rs
- tests/http_roundtrip.rs
- docs/1-about/02-architecture.md
author: claude-ci-speedup
updated_by: claude-ci-speedup
created_at: 2026-09-16T02:28:11Z
updated_at: 2026-09-16T02:28:11Z
---

finish() sends the outcome only as structured_content and a one-line status summary as the text block. ServerHandler::list_tools post-processes schemas with slim_schema (drops $schema, default:null, format, minimum, nullable unions, per-parameter descriptions). Tool descriptions are one sentence carrying enum values and bounds. Tests pin: text < 200 bytes, tools/list <= 6144 chars, each tool <= 500; lower only. ADR-0017.

## Rationale

Every call cost double (JSON in both blocks). tools/list floor with no descriptions is 4,170 chars for 20 tools, so per-parameter descriptions cannot fit any sub-6 KB budget; 04-primitives.md and the input structs' doc comments keep the full semantics.

## Alternatives

- keep short descriptions on every parameter (1,258 chars of overhead alone)
- per-handler summary strings
- configure schemars instead of post-processing
- fold contract_get and task_pull away (surface change, left open)
