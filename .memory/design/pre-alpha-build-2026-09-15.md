---
title: Pre-alpha build 2026-09-15
type: note
tags: [tirith, build, architecture, lessons]
---

# Pre-alpha build 2026-09-15

Tirith 0.1.0 scaffolded in one session: all five primitives, JSON persistence, CLI, dashboard, 41 tests.

## Observations
- [design] Tool outcomes are always `{status: ...}` JSON, never MCP-level errors, so agents branch on status #api
- [design] Any tool call by an agent renews all its leases; lazy reaping on every access means no background reaper thread #claims
- [design] `RepoPath` normalizes away trailing slashes; overlap = equal or ancestor at a segment boundary, so `src/auth` covers `src/auth/login.rs` but not `src/authz` #claims
- [design] Republishing a contract auto-emits a `contract` notice to the consumers, which links contracts and notices without extra agent work #contracts
- [design] Persister writes snapshots by sequence number under a tokio mutex; stale snapshots are skipped, so concurrent tool calls cannot write an older state last #storage
- [lesson] rmcp's default client transport retries reconnects forever (`ExponentialBackoff { max_times: None }`); the CLI needs `NeverRetry` or an unreachable daemon hangs #rmcp
- [lesson] rmcp 3.4 requires reqwest 0.13, which needed a `cargo update` of the sparse index before it resolved #rmcp
- [lesson] The CLI finds the daemon through `.tirith/runtime/daemon.json`; scripts must wait for that file, not sleep a fixed time #cli
- [lesson] Clippy pedantic with `-D warnings` is manageable if `pub(crate)` is the default and doc comments use backticks for identifiers #rust

## Open questions
- [question] Should `claim` accept a `task_id` so claims and tasks link on the dashboard?
- [question] Contract `shape` is free JSON; validate against JSON Schema in a later milestone?

## Relations
- follows [[Project kickoff 2026-09-15]]

## Distribution (added later the same day)
- [decision] cargo-dist 0.32 builds and publishes releases for 8 targets (macOS arm/x86, Linux arm/x86 gnu+musl, Windows arm/x86); config in dist-workspace.toml, workflow generated #release
- [decision] `install.sh` at the repo root is a stable shim over the release asset `tirith-installer.sh`, so the README one-liner never changes #release
- [lesson] The generated installer can be tested offline with `TIRITH_DOWNLOAD_URL` pointing at a local http.server over target/distrib and `TIRITH_UNMANAGED_INSTALL` for the target dir #release
- [lesson] The crate name `tirith` on crates.io is taken by an unrelated terminal-security tool, so crates.io publishing needs a different package name #release
