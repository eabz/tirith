---
id: 0699789d-6666-4803-9e10-73de73eaf81f
permalink: one-claim-response-replaces-the-four-reads-before-every-edit-adr-0014
title: One claim response replaces the four reads before every edit (ADR-0014)
kind: decision
tags: []
paths:
- src/server.rs
- src/state.rs
- tests/http_roundtrip.rs
- AGENTS.md
- docs/6-agent-workflow/03-tirith-dogfooding.md
- docs/1-about/04-primitives.md
- docs/5-decisions/0014-brief-on-claim.md
author: claude-token-diet
updated_by: claude-token-diet
created_at: 2026-09-16T03:09:46Z
updated_at: 2026-09-16T03:09:46Z
---

`claim` carries a brief for the claimed paths: the five newest unread-and-undelivered notices, contracts, decisions and memory notes as compact rows, plus `more` counts; empty sections are omitted; the whole ok response is hard-capped at 4,096 bytes by dropping the oldest rows of the largest section. `brief` defaults to true. Notices shown are marked delivered per agent for the daemon lifetime only, in State, never in notices.jsonl; delivery is not an ack. The protocol is now claim -> edit -> notice_publish -> release; the list tools are for paging when `more` is non-zero. Lost-lease reporting (ADR-0015) rides the same result-assembly path: finish(agent, outcome) attaches `lost` and a text-line warning.

## Rationale

Measured 2026-09-16: a work cycle was 7 round trips and ~2,350 shipped tokens on the swarm bench (~13,000 against this repo's real state), with contract_list alone at 7,869 tokens; per list row, JSON keys ~40%, uuid ~20%, timestamp ~15%, content ~25%. The daemon handles ~20k calls/s, so round trips and wire shape, not throughput, are the cost. Full reasoning in docs/5-decisions/0014-brief-on-claim.md.

## Alternatives

- A separate brief tool: one more schema in every session and one more call agents forget
- brief opt-in: the flag is the step agents skip
- Full bodies in the brief: unbounded
- Persisting delivered marks in notices.jsonl or an acks file: a rewrite per claim; delivery needs no durability
- A per-agent cursor: breaks path-scoped briefs
- A line-oriented text format: deferred until byte cuts and this land
