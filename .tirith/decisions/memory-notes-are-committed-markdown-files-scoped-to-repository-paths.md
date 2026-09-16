---
id: ed3e13b9-94fd-4bd6-b20b-da0ed10dc134
permalink: memory-notes-are-committed-markdown-files-scoped-to-repository-paths
title: Memory notes are committed Markdown files scoped to repository paths
kind: decision
tags: []
paths:
- src/memory.rs
- src/state.rs
- src/store.rs
- src/server.rs
- tests/memory_layer.rs
- docs/5-decisions/0011-memory-primitive.md
- docs/1-about/04-primitives.md
author: claude-memory-layer
updated_by: claude-memory-layer
created_at: 2026-09-16T01:50:29Z
updated_at: 2026-09-16T01:50:29Z
---

Tirith gains a fifth durable primitive, `memory`, and stops calling itself "not a memory layer". Notes are one committed Markdown file per note at .tirith/memory/<permalink>.md, YAML frontmatter plus a Markdown body, deliberately Basic Memory compatible (same `- [category] text #tag` observation and `- kind [[target]]` relation syntax; permalinks may carry `/` folder segments). Notes are scoped to repository paths and matched by the same overlap rule as claims. Permalinks are immutable once created. Search is in-process term scoring that weights titles and tags above observations and observations above body prose: no embeddings, no index, no new dependency. Tool surface is three tools, memory_write / memory_read / memory_search; recent activity is memory_search with no query, and relation walking is memory_read with a depth.

## Rationale

The decisions log was already durable path-scoped knowledge under a narrower name, so the "not a memory layer" line was never quite true. The thing an external memory server cannot do is know what an agent is about to edit; Tirith knows, because agents must claim paths first, so a note scoped to a path can be handed to whoever claims it. That removes the step agents actually skip, which is searching. JSONL was rejected because prose on one escaped line destroys the git-diffability ADR-0003 exists to protect. Folders were added mid-implementation after an integration test against the repo's own .memory/ notes showed a flat namespace would have broken the Basic Memory compatibility promise on our own files.</decision>
<parameter name="alternatives">["JSONL log like notices and decisions: unreadable diffs for prose", "SQLite with FTS5 or an embedded vector store: binary file, not reviewable, C build dependency, already rejected by ADR-0003", "Static embeddings via model2vec-rs or fastembed/ONNX: deferred behind a future cargo feature, binary size and no demonstrated need", "Keep using Basic Memory only: no notion of repository paths, so it cannot deliver a note to the agent claiming the file it is about", "A flat permalink namespace with no folders: reversed, it broke compatibility with the repo's own notes"]
