---
id: e81fbe1e-a386-4287-b49b-5c49b863fcfd
permalink: user-facing-docs-where-each-fact-lives-and-how-to-re-verify-them
title: "User-facing docs: where each fact lives and how to re-verify them"
kind: lesson
tags:
- docs
- review
paths:
- README.md
- AGENTS.md
- docs/README.md
- docs/1-about
- docs/2-examples
- index.html
author: agent-1
updated_by: agent-1
created_at: 2026-09-16T04:12:41Z
updated_at: 2026-09-16T04:12:41Z
---

Production review of 2026-09-16 (task 13ae801e). What is not obvious from the files:

- [fact] `docs/1-about/04-primitives.md` is the only place tool schemas are written; README, index.html and 02-client-setup name tools and link there. Keep it that way: one `## <Primitive>` section per module, no Built/Planned labels (everything is built except `## Resources — Planned`).
- [fact] Line budgets the head developer set: README under 200 lines, AGENTS.md under 150, and no paragraph shared between them. README is pitch + install + quick start + tool table + one example + CLI + links; long explanations belong in 02-architecture or an ADR.
- [gotcha] The transcript in `docs/2-examples/01-two-agents-demo.md` must be regenerated from a real `examples/demo.sh` run after any change to the brief or to notice delivery. Since ADR-0021 the brief on Bob's claim delivers the notices, so the demo's `notice list --unread --limit 1` prints `no notices`; the doc explains that instead of pretending to show paging.
- [lesson] The cheapest way to verify every command in the docs: `tirith serve --bind 127.0.0.1:0 --no-tray` in a scratch git repo, wait for `.tirith/runtime/daemon.json`, run the commands with `TIRITH_URL` unset from that directory, then `kill -INT <pid>` from daemon.json. The curl handshake in 02-client-setup works verbatim; the `Mcp-Session-Id` header is lowercase in the response.
- [gotcha] index.html repeats install and connect commands from README, 05-installation and 02-client-setup; the `tirith status` sample there and in 02-client-setup is real output (version line plus an agents line), update both when the CLI renderer changes. Tirith writes `.tirith/.gitignore` itself, so no doc should tell users to add `.tirith/runtime/` to their own `.gitignore`.
- [fact] `tests/budgets.rs` and `tests/http_roundtrip.rs` both pin tools/list at 7,800 chars and 500 per tool; 02-architecture quotes those numbers. Bump the doc when the constants move.
