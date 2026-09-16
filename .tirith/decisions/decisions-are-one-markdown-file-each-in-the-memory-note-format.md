---
id: 0a8615b7-8563-4680-9136-baac568315df
permalink: decisions-are-one-markdown-file-each-in-the-memory-note-format
title: Decisions are one Markdown file each, in the memory-note format
kind: decision
tags: []
paths:
- src/decisions.rs
- src/store.rs
- src/state.rs
- src/memory.rs
- .tirith/decisions
- docs/5-decisions/0022-decisions-as-markdown.md
author: head-dev-fable
updated_by: head-dev-fable
created_at: 2026-09-16T04:29:52Z
updated_at: 2026-09-16T04:29:52Z
---

decisions.jsonl is replaced by one committed Markdown file per decision at .tirith/decisions/<permalink>.md, written and parsed with the memory-note frontmatter and body format (MemoryNote::to_markdown / from_markdown) with kind `decision`, title = the decision title, body = the decision text, then "## Rationale" and "## Alternatives" (a bullet list), paths = affects_paths, author/updated_by = recorded_by, created_at = recorded_at, id shared with the memory id space. The decision_record and decision_list tools, the brief's decisions section, the dashboard and the CLI keep their shapes. Persistence uses the per-file seam (ChangedIds) like contracts and memory. On load, an existing .tirith/decisions.jsonl is imported once: every row becomes a file, the file set is written by the first persist, and the JSONL is deleted after that persist succeeds (git history keeps it). Follow-up for 1.1: fold the decisions board into the memory book so decisions are memory notes of kind decision with the same three tools reduced to two.

## Rationale

eabz, 2026-09-16: prose belongs in Markdown, JSONL is for events. A decision is a title plus rationale plus alternatives, exactly what a note is; one file per decision diffs cleanly in a pull request and can be edited by hand, and reusing the note format means one parser and a trivial path to the 1.1 merge.

## Alternatives

- Merge decisions into memory notes now, removing two tools (rejected for 1.0.0: changes the tool surface, the brief and the dashboard the same day as the release)
- Keep JSONL and pretty-print it (still one escaped line per decision)
