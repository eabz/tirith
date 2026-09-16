---
id: f9f949f6-d3a9-4d4c-bbf1-366bdc2f6478
permalink: presentation-layer-review-2026-09-16-shapes-the-cli-and-dashboard-may-rely-on
title: "Presentation layer review 2026-09-16: shapes the CLI and dashboard may rely on"
kind: lesson
tags:
- review
- cli
- dashboard
paths:
- src/cli.rs
- src/dashboard.html
- src/server.rs
author: agent-3
updated_by: agent-3
created_at: 2026-09-16T04:12:09Z
updated_at: 2026-09-16T04:12:09Z
---

# What the renderers can assume

Found while reviewing the presentation layer (task b8d1fcf6). Every dead branch removed that day came from a renderer guessing at a shape the server never sends; check the serialized type before adding a fallback.

- [fact] `Task` serializes `TaskState` with `#[serde(flatten)]` and `tag = "status"`: a task row has top-level `status`, plus `owner` (in_progress, blocked) or `by` (done). There is no nested `state` object.
- [fact] `renew` returns `{agent, count, expires_at}` and never echoes the claims (pinned by `tests/http_roundtrip.rs`).
- [fact] `status` with `verbose` sends `agents[].paths_count`, never the path list; the dashboard gets the list from `/api/state` instead.
- [fact] `lost` rows are `LostLease {path, owner, at}`; `owner` is the agent that lost the lease, and there is no `now_held_by`.
- [fact] `memory_search` rows are the note digest with a `score` key at the top level, not wrapped in `note`.
- [fact] Notice rows still serialize `acked_by` (kept for wire compatibility after the seen-log rename), so `ROW_DROP` in server.rs and the dashboard's "seen by" column read that key.
- [gotcha] clippy on stable 1.98 flags nested `if let` as `collapsible_if` when it can be a let-chain; CI installs newest stable, so run `rustup update stable` before trusting a local pass.
