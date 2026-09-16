---
id: 43149c47-5824-4511-bf1a-fe19b3e145f3
permalink: integration-test-helpers-tests-common-and-what-a-test-binary-lets-you-share
title: "Integration test helpers: tests/common and what a test binary lets you share"
kind: gotcha
tags:
- tests
- dead-code
paths:
- tests
- examples/demo.sh
author: agent-5
updated_by: agent-5
created_at: 2026-09-16T04:22:50Z
updated_at: 2026-09-16T04:22:50Z
---

`tests/common/mod.rs` holds `options(root, clock)` and `call(handle, tool, args)`; `tests/common/raw_client.rs` holds `raw_client(url)` and is pulled in with `#[path = "common/raw_client.rs"] mod raw_client;` only by budgets.rs and http_roundtrip.rs. Everything in a shared file is `pub(crate)`.

## Observations
- [gotcha] Each integration test file is its own binary, so an item in a shared module that one includer does not use is a `dead_code` warning there, and CI runs clippy with -D warnings. Rule 42 forbids `#[allow(dead_code)]`, so the split is by usage: a helper goes in `common/mod.rs` only if every file that declares `mod common;` uses it; otherwise it gets its own file and `#[path]` includers #dead-code
- [gotcha] `pub` in a test binary trips `unreachable_pub`; use `pub(crate)` #dead-code
- [lesson] The pure domain cases (search ranking, relation walking, corrupt frontmatter) are unit tests in src/memory.rs; tests/memory_layer.rs only keeps what crosses the filesystem or the daemon, so a rule is asserted once #tests
- [lesson] The stale-write conflict test drives the daemon with `ManualClock` and `clock.advance` instead of `sleep(1100ms)`; whole-second frontmatter timestamps need a 2 s step, not 1 #tests
- [gotcha] examples/demo.sh runs `serve --no-tray` and traps EXIT with `kill -INT $daemon; wait $daemon`; without --no-tray the daemon spawns the tray and the trap used to leave a process behind. Check with `pgrep -f "tirith serve"` after a run #demo
- [lesson] Since ADR-0021 a claim's brief marks notices seen, so a demo step that lists `--unread` right after a claim prints nothing; the paging demonstration lists without `--unread` #demo
