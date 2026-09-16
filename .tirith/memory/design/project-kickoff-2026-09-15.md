---
id: 552e3579-4f9e-47ad-bbcc-f2dd06582292
permalink: design/project-kickoff-2026-09-15
title: Project kickoff 2026-09-15
kind: research
tags:
- tirith
- kickoff
- architecture
paths:
- docs
author: unknown
updated_by: agent-2
created_at: 2026-09-16T03:54:46Z
updated_at: 2026-09-16T04:10:05Z
---

# Project kickoff 2026-09-15

Tirith is an MCP coordination server for 5 to 10 parallel coding agents on one repo.
Primitives at kickoff: claims (leases), task board, contracts, change notices, decisions log.
Contracts and change notices are the heart; claims and tasks are table stakes.
Memory notes (ADR-0011) and agent messages (ADR-0020) were added the next day.

## Decisions made today
- [decision] Rust edition 2024 with rmcp 3.x, one daemon per repo over streamable HTTP at 127.0.0.1:7477/mcp (ADR-0001, ADR-0002) #architecture
- [decision] In-memory state written through to JSON under .tirith/; runtime state gitignored, contracts/notices/decisions committed (ADR-0003) #storage
- [decision] Agent identity is a caller-supplied string; claims are TTL leases renewed by any call (ADR-0004) #identity
- [decision] Claims are files or directory prefixes; globs deferred (ADR-0005) #claims
- [decision] Serena for code intelligence, Basic Memory for long-form notes, Tirith itself for coordination from milestone 2 (Basic Memory was retired the next day, ADR-0012; its notes became the notes in this folder) #workflow

## Open questions
- [question] Should Tirith read git to warn when a claimed file has uncommitted changes from another agent?
- [question] Contract shape validation: free JSON in v1 (ADR-0013 keeps it out of v1)

## Relations
- documented_in [[design/pre-alpha-build-2026-09-15]]
- the ADRs are in docs/5-decisions/
