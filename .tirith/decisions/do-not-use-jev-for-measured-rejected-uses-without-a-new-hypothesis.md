---
id: a23ab405-60df-47d1-b19b-39d8e53f123f
permalink: do-not-use-jev-for-measured-rejected-uses-without-a-new-hypothesis
title: Do not use Jev for measured-rejected uses without a new hypothesis
kind: decision
tags: []
paths:
- src/assist.rs
- src/lead.rs
- src/jev.rs
- examples/jev_bench
author: jev-lead
updated_by: jev-lead
created_at: 2026-09-17T04:04:42Z
updated_at: 2026-09-17T04:04:42Z
---

Jev is not used for: claim brief filtering, broadcast narrowing, conflict advice shown to workers, exposing Jev metadata to workers, documentation pointers, test selection or skipping, failure/log triage, file prediction for scheduling, file picking from a repo map, command or permission approval, question deflection, inbox urgency scoring, next-tool choice, mid-session model switching, read/skim/skip briefs, or token savings by filtering reads. The full evidence is the memory note `jev-rejected-uses-measured-2026-09-17`. A retry needs a new hypothesis, blind labels written before any Jev call, and a deterministic baseline.

## Rationale

Each was measured in the 2026-09-17 Jev lab and was harmful, unsafe, or did not beat a deterministic rule beyond label noise.

## Alternatives

- Keep experimenting ad hoc (rejected: repeats paid-for negative results)
- Remove Jev entirely (rejected: five sites beat deterministic rules; see ADR-0026)
