# ADR-0022: Decisions are one Markdown file each, in the memory-note format

**Status:** Accepted, 2026-09-16. Refines ADR-0003 and ADR-0010 for
decisions. Board decision 0a8615b7; asked for by eabz before 1.0.0.

## Context

Decisions were appended to `.tirith/decisions.jsonl`, one JSON object per
line. A decision is prose: a title, the choice, a rationale, alternatives.
On one escaped line it is unreadable in a pull request and cannot be
edited by hand, which is the property ADR-0003 exists to protect and the
reason memory notes were Markdown from the start (ADR-0011).

## Decision

1. **One file per decision** at `.tirith/decisions/<permalink>.md`,
   committed. The permalink is the slug of the title, made unique with
   the decision's short id when two titles collide, and never changes.
2. **The memory-note format, kind `decision`.** The file is exactly what
   `MemoryNote::to_markdown` writes: YAML frontmatter (id, permalink,
   title, kind, paths, author, updated_by, created_at, updated_at) and a
   body holding the decision text, then `## Rationale` and a
   `## Alternatives` bullet list when they are not empty. One parser
   serves notes and decisions; `Decision::to_note` and
   `Decision::from_note` are the two directions and round-trip exactly
   (timestamps are whole seconds).
3. **Same tools, same shapes.** `decision_record`, `decision_list`, the
   brief's decisions section, the dashboard and the CLI are unchanged.
   Persistence uses the per-file seam (`ChangedIds`) like contracts and
   notes, in the lazy durability tier because git keeps the history.
4. **One-time import.** On load, an existing `decisions.jsonl` is read,
   every row is written as a file, and the log is deleted only when every
   file was written; a failed write keeps the log and reports a
   `load_error`, so the next start retries.

## Alternatives

- **Merge decisions into memory notes now** (kind `decision`, two tools
  fewer). Rejected for 1.0.0: it changes the tool surface, the brief and
  the dashboard on release day. The format chosen here makes that merge a
  file move for 1.1.
- **Pretty-print the JSONL.** Still one object per line; no gain.

## Consequences

- `.tirith/decisions/` joins `contracts/` and `memory/` as a committed
  directory of one file per item; `decisions.jsonl` disappears from
  repositories on their first start with this version.
- A decision can be corrected in a text editor; a malformed file is a
  `load_error` in `status`, never a reason to refuse to start (ADR-0021's
  tolerant load applies).
- Rows on the wire gain a `permalink` field, the file's name.
