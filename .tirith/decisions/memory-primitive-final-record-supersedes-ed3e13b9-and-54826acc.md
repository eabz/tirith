---
id: 618e21b8-9f1a-4721-84b9-41cfa41f137c
permalink: memory-primitive-final-record-supersedes-ed3e13b9-and-54826acc
title: "Memory primitive: final record, supersedes ed3e13b9 and 54826acc"
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
created_at: 2026-09-16T01:51:09Z
updated_at: 2026-09-16T01:51:09Z
---

Authoritative record for the memory primitive. Two earlier records, ed3e13b9 and 54826acc, are malformed (I pasted field markup into the body); ignore both and read this one. Tirith gains a fifth durable primitive, memory, and stops calling itself "not a memory layer". Notes are one committed Markdown file per note at .tirith/memory/PERMALINK.md, YAML frontmatter plus a Markdown body. The format is deliberately Basic Memory compatible: the same "- [category] text #tag" observation lines and "- kind [[target]]" relation lines, and permalinks may carry slash-separated folder segments, at most 8 deep, each segment lowercase letters, digits and dashes only, so a dot cannot be spelled, so a permalink cannot escape the memory directory. Notes carry repository paths and are matched by the same overlap rule as claims, so a note about src/store.rs is found by a claim on src. Permalinks are immutable once created, so links and filenames survive a retitle. Search is in-process term scoring that weights titles and tags above observations and observations above body prose; no embeddings, no index, no new dependency. Three tools: memory_write, memory_read, memory_search. Recent activity is memory_search with no query; relation walking is memory_read with a depth. Full reasoning is in ADR-0011.

## Rationale

The decisions log was already durable path-scoped knowledge under a narrower name, so the "not a memory layer" line was never quite true. The thing an external memory server cannot do is know what an agent is about to edit. Tirith knows, because agents must claim paths before editing, so a note scoped to a path can be handed to whoever claims it. That removes the step agents actually skip, which is searching. JSONL was rejected because long-form prose on a single escaped line destroys the git-diffability that ADR-0003 exists to protect. Folders were added mid-implementation after an integration test that parses the repository's own .memory/ notes proved a flat namespace would have broken the Basic Memory compatibility promise on our own files.

## Alternatives

- JSONL log like notices and decisions: unreadable diffs for long-form prose
- SQLite with FTS5 or an embedded vector store: binary file, not reviewable in a pull request, C build dependency, already rejected by ADR-0003
- Static embeddings via model2vec-rs or fastembed over ONNX: deferred behind a future cargo feature, binary size cost and no demonstrated need yet
- Keep using Basic Memory only: it has no notion of repository paths, so it cannot deliver a note to the agent claiming the file the note is about
- A flat permalink namespace with no folders: reversed during implementation because it broke compatibility with the repository's own notes
