---
id: e6f1f921-3723-4e04-a345-0becd6b9f595
permalink: tool-schema-budget-in-server-rs
title: Tool schema budget in server.rs
kind: lesson
tags:
- tokens
- mcp
paths:
- src/server.rs
- tests/budgets.rs
author: claude-ci-speedup
updated_by: agent-5
created_at: 2026-09-16T02:49:26Z
updated_at: 2026-09-16T04:22:40Z
---

Adding or changing a tool in `src/server.rs` must fit the tools/list budget pinned by `tools_list_stays_under_7800_chars_and_no_tool_over_500` in `tests/budgets.rs` (total and per-tool; the constant's doc comment there is the log of every raise), and the text block of every result must stay one line (`summary` in server.rs, pinned by `every_result_text_block_is_a_status_line_under_200_bytes` in the same file). See ADR-0017.

## Observations
- [lesson] tools/list is 4,170 chars for 20 tools with every description removed: 74 parameters each cost their name plus a type object. Per-parameter descriptions are stripped by `slim_schema`, so anything an agent must know about a parameter goes in the tool's one-sentence `description` (enum values, default/max of a limit) #tokens
- [gotcha] rmcp's `#[tool_handler]` macro skips generating `list_tools` when the impl defines one; that is how `slim_schema` post-processes schemas without touching the `#[tool]` fns #rmcp
- [gotcha] `missing_docs` is on, so field doc comments on the Input structs cannot be deleted to shrink schemas; they stay for rustdoc and are dropped on the wire instead #tokens
- [lesson] Raising the size bound is only acceptable for genuinely new parameters, never for prose; storage-claude raised it 6,144 to 6,656 for limit/before/all/verbose on 2026-09-16 #tokens
- [lesson] The one-line text summary is derived from the outcome's shape (message, new_paths, released, count + a list key, or a single item key); a new tool whose outcome has none of these prints just the status #tokens
- [gotcha] `tests/http_roundtrip.rs::dashboard_and_tool_list_are_served` pins the tool names to the 22 in docs/1-about/04-primitives.md exactly; a new tool fails it until the doc and that list agree #mcp

## Relations
- documented_in [[ADR-0017]]
