---
id: 54826acc-5c1a-49a5-be52-90930c1ad8c1
permalink: memory-notes-are-committed-markdown-files-scoped-to-repository-paths-corrected-record
title: Memory notes are committed Markdown files scoped to repository paths (corrected record)
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
created_at: 2026-09-16T01:50:49Z
updated_at: 2026-09-16T01:50:49Z
---

Replaces decision ed3e13b9, which I recorded with a malformed body: its text has a stray literal "<parameter name=...>" block in it and its alternatives list came out empty. Ignore that one and read this. The decision itself is unchanged: Tirith gains a fifth durable primitive, `memory`, and stops calling itself "not a memory layer". Notes are one committed Markdown file per note at .tirith/memory/<permalink>.md, YAML frontmatter plus a Markdown body, deliberately Basic Memory compatible (same `- [category] text #tag` observations and `- kind [[target]]` relations; permalinks may carry `/` folder segments, max depth 8, segments of [a-z0-9-] only so `..` cannot be spelled). Notes carry repository paths and are matched by the same overlap rule as claims. Permalinks are immutable once created. Search is in-process term scoring weighting titles and tags above observations and observations above body prose: no embeddings, no index, no new dependency. Three tools: memory_write, memory_read, memory_search. Recent activity is memory_search with no query; relation walking is memory_read with a depth. Full reasoning in ADR-0011.</decision>
<parameter name="rationale">The decisions log was already durable path-scoped knowledge under a narrower name, so "not a memory layer" was never quite true. What an external memory server cannot do is know what an agent is about to edit; Tirith knows, because agents must claim paths first, so a note scoped to a path can be handed to whoever claims it. That removes the step agents actually skip, which is searching. JSONL was rejected because prose on one escaped line destroys the git-diffability ADR-0003 exists to protect. Folders were added mid-implementation after an integration test against the repository's own .memory/ notes proved a flat namespace would have broken the Basic Memory compatibility promise on our own files.

## Alternatives

- JSONL log like notices and decisions: unreadable diffs for long-form prose
- SQLite with FTS5 or an embedded vector store: binary file, not reviewable in a pull request, C build dependency, already rejected by ADR-0003
- Static embeddings via model2vec-rs or fastembed over ONNX: deferred behind a future cargo feature, binary size cost and no demonstrated need
- Keep using Basic Memory only: it has no notion of repository paths, so it cannot deliver a note to the agent claiming the file the note is about
- A flat permalink namespace with no folders: reversed during implementation, it broke compatibility with the repository's own notes
