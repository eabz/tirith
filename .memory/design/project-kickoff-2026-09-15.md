---
title: Project kickoff 2026-09-15
type: note
tags:
- tirith
- kickoff
- architecture
permalink: tirith/design/project-kickoff-2026-09-15
---

# Project kickoff 2026-09-15

Tirith is an MCP coordination server for 5 to 10 parallel coding agents on one repo.
Primitives: claims (leases), task board, contracts, change notices, decisions log.
Contracts and change notices are the heart; claims and tasks are table stakes.

## Decisions made today
- [decision] Rust edition 2024 with rmcp 3.x, one daemon per repo over streamable HTTP at 127.0.0.1:7477/mcp #architecture
- [decision] In-memory state written through to JSON under .tirith/; runtime state gitignored, contracts/notices/decisions committed #storage
- [decision] Agent identity is a caller-supplied string; claims are TTL leases renewed by any call #identity
- [decision] Claims are files or directory prefixes; globs deferred #claims
- [decision] Serena for code intelligence, Basic Memory for long-form notes, Tirith itself for coordination from milestone 2 #workflow

## Open questions
- [question] Should Tirith read git to warn when a claimed file has uncommitted changes from another agent?
- [question] Contract shape validation: free JSON in v1, or JSON Schema from the start?

## Relations
- documented_in [[Tirith docs]]
- see docs/5-decisions/ in the repo for the ADRs