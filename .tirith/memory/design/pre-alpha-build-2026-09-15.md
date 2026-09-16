---
id: b7af6deb-4d10-4faa-91ed-7f873b55a87b
permalink: design/pre-alpha-build-2026-09-15
title: Pre-alpha build 2026-09-15
kind: research
tags:
- tirith
- build
- architecture
- lessons
paths:
- src/claims.rs
- src/store.rs
- src/contracts.rs
- src/cli.rs
- Cargo.toml
- dist-workspace.toml
- install.sh
author: unknown
updated_by: agent-2
created_at: 2026-09-16T03:54:46Z
updated_at: 2026-09-16T04:09:57Z
---

# Pre-alpha build 2026-09-15

Tirith 0.1.0 scaffolded in one session: all five primitives, JSON persistence, CLI, dashboard, 41 tests. Lines marked "since superseded" describe 0.1.0 and are kept as history.

## Observations
- [design] Tool outcomes are always `{status: ...}` JSON, never MCP-level errors, so agents branch on status #api
- [design] Any tool call by an agent renews all its leases; lazy reaping on every access means no background reaper thread (since refined: ADR-0015 caps a lease at four TTLs and reports lost leases) #claims
- [design] `RepoPath` normalizes away trailing slashes; overlap = equal or ancestor at a segment boundary, so `src/auth` covers `src/auth/login.rs` but not `src/authz` #claims
- [design] Republishing a contract auto-emits a `contract` notice to the consumers, which links contracts and notices without extra agent work #contracts
- [design] 0.1.0's persister wrote whole snapshots by sequence number under a tokio mutex, skipping stale ones (since superseded: ADR-0010 writes per-primitive deltas through one coalescing writer) #storage
- [lesson] rmcp's default client transport retries reconnects forever (`ExponentialBackoff { max_times: None }`); the CLI needs `NeverRetry` or an unreachable daemon hangs #rmcp
- [lesson] rmcp 3.4 requires reqwest 0.13, which needed a `cargo update` of the sparse index before it resolved #rmcp
- [lesson] The CLI finds the daemon through `.tirith/runtime/daemon.json`; scripts must wait for that file, not sleep a fixed time #cli
- [lesson] Clippy pedantic with `-D warnings` is manageable if `pub(crate)` is the default and doc comments use backticks for identifiers #rust

## Open questions
- [question] Should `claim` accept a `task_id` so claims and tasks link on the dashboard?
- [question] Contract `shape` is free JSON; validation against JSON Schema is out of v1 (ADR-0013), still open after it

## Relations
- follows [[design/project-kickoff-2026-09-15]]

## Distribution (added later the same day)
- [decision] cargo-dist 0.32 builds and publishes releases for 8 targets (macOS arm/x86, Linux arm/x86 gnu+musl, Windows arm/x86); config in dist-workspace.toml, workflow generated #release
- [decision] `install.sh` at the repo root is a stable shim over the release installer (today `tirith-mcp-installer.sh`), so the README one-liner never changes when asset names do #release
- [lesson] The generated installer can be tested offline with `TIRITH_MCP_DOWNLOAD_URL` pointing at a local http.server over target/distrib and `TIRITH_MCP_UNMANAGED_INSTALL` for the target dir; the variables are prefixed with the crate name, not the binary name #release
- [lesson] The crate name `tirith` on crates.io is taken by an unrelated terminal-security tool, so the package is published as `tirith-mcp` while the binary stays `tirith` #release
