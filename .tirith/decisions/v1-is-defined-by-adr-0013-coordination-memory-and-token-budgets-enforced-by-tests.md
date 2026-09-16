---
id: b1355eab-e339-4852-9cc0-554f0519fb24
permalink: v1-is-defined-by-adr-0013-coordination-memory-and-token-budgets-enforced-by-tests
title: "v1 is defined by ADR-0013: coordination, memory, and token budgets enforced by tests"
kind: decision
tags: []
paths:
- src
- docs/1-about/01-purpose.md
- docs/5-decisions/0013-v1-definition.md
- tests/http_roundtrip.rs
author: head-dev-fable
updated_by: head-dev-fable
created_at: 2026-09-16T02:15:38Z
updated_at: 2026-09-16T02:15:38Z
---

v1 (1.0.0) ships when three statements hold and each is guarded by a test: (1) coordination works for 5-10 agents with brief-on-claim, version-checked daemon, orphan reaping, lost-lease reporting, honest persist errors, and a daemon that never refuses to boot over a corrupt file; (2) the memory tools are wired, claim carries note excerpts, search is bounded and returns digests, notes can be deleted and updates can be made conflict-safe; (3) token budgets: tools/list under 6 KB, result text block under 200 bytes and never JSON, list tools default to 20 compact rows with a cursor, status/claims_list/renew return the caller's own view by default, brief under 4 KB. Every bullet maps to a task id on the board; see docs/5-decisions/0013-v1-definition.md.

## Rationale

Five sessions were working from their own finish lines. Measurements on this repository's own daemon show an unfiltered list call can exhaust an agent's context, so token budgets are part of correctness, not polish.

## Alternatives

- Ship 0.2 with memory tools and defer token work
- Feature list without numeric budgets
- Switch storage to a database to cut tokens (unrelated: tokens are spent on the wire)
