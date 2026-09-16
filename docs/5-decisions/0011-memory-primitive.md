# ADR-0011: Memory notes are Markdown files scoped to repository paths

**Status:** Accepted, 2026-09-16. Refined by
[ADR-0012](0012-retire-basic-memory.md), which retires Basic Memory
instead of keeping it for path-free notes, and by
[ADR-0013](0013-v1-definition.md), which adds a fourth tool,
`memory_delete`, and bounds search rows to excerpts.

## Context

Every other document in this repository says Tirith is not a memory layer,
and points agents at Basic Memory for durable knowledge. That split has
held up badly in practice.

The coordination primitives already store durable knowledge: a decision is
a settled choice with a rationale, kept forever, scoped to paths. It is
memory under a narrower name. What is missing is everything that is not a
decision: a lesson learned the hard way, a trap in a dependency, the state
of unfinished work when a lease expires.

At the same time, an external memory server cannot do the one thing that
matters most here. It does not know what an agent is about to edit. Tirith
does, because the agent must claim paths before editing them. Knowledge
that arrives at claim time is knowledge nobody had to search for, and
searching is exactly the step agents skip.

## Decision

Tirith gains a fifth durable primitive, `memory`, and stops describing
itself as "not a memory layer". It is a coordination server whose memory is
scoped to the same paths it coordinates.

- **One Markdown file per note**, at `.tirith/memory/<permalink>.md`,
  committed. YAML frontmatter carries the structured fields; the body is
  Markdown prose.
- **Notes are scoped to repository paths** and found by the same overlap
  rule as claims, so a note about `src/store.rs` is found by a claim on
  `src`.
- **The format is deliberately Basic Memory compatible.** Observations
  (`- [category] text #tag`) and relations (`- kind [[target]]`) use the
  same line syntax, and the parser accepts the frontmatter Basic Memory
  writes. That made the migration in
  [ADR-0012](0012-retire-basic-memory.md) a file move with no rewriting,
  and it keeps the door open to importing notes from other Markdown memory
  tools.
- **Permalinks are immutable.** A note's permalink is slugified from its
  title at creation and never follows a later retitle, so `[[links]]` and
  filenames stay valid.
- **Permalinks may carry folder segments** (`tirith/design/note`), which
  become nested directories. New notes are flat; a folder is used only
  when a permalink is given explicitly or read from an existing file.
  Segments are lowercase letters, digits, and dashes, so `.` cannot be
  spelled, so `..` cannot be spelled, so a permalink cannot escape the
  memory directory.
- **Search is in-process and dependency-free**: term scoring that weights
  titles and tags above observations and observations above body prose. No
  embeddings, no index, no new crate.
- **Three tools**: `memory_write`, `memory_read`, `memory_search`. Recent
  activity is `memory_search` with no query; relation walking is
  `memory_read` with a depth.

## Alternatives

- **A JSONL log like notices and decisions.** Rejected. These notes are
  long-form prose, and JSONL puts each one on a single line with every
  newline escaped. That destroys the git-diffability that ADR-0003 exists
  to protect, and it is unreadable in a pull request.
- **SQLite with FTS5, or an embedded vector store.** Rejected for the same
  reason ADR-0003 rejected SQLite: a binary file is not reviewable, and it
  adds a C build dependency. Revisit only if scoring proves inadequate.
- **Semantic search with static embeddings** (`model2vec-rs`, or
  `fastembed` via ONNX). Deferred, not rejected. Term scoring is enough for
  the thousands of notes a repository produces, and the binary must stay
  small. If it arrives it belongs behind a cargo feature, off by default.
- **Keep using Basic Memory only.** Rejected. It has no notion of
  repository paths, so it cannot deliver a note to the agent claiming the
  file the note is about, which is the whole point.
- **A flat permalink namespace**, with no folders. This was the original
  decision, reversed during implementation: the repository's own Basic
  Memory notes used permalinks like `tirith/design/pre-alpha-build`, so a
  flat namespace would have broken the compatibility promise above on our
  own files. The integration test that parses every note the repository
  ships caught it. The cost is a recursive directory scan and `mkdir -p`
  before a write, which is smaller than the promise was worth.

## Consequences

- The "not a memory layer" line is revised wherever it appeared:
  `AGENTS.md`, `README.md`, `docs/1-about/01-purpose.md`, and
  `docs/6-agent-workflow/02-memory.md`, plus the Serena memory
  `workflow/project-status`. Serena memories keep their job for short
  facts that are not tied to a path;
  [ADR-0012](0012-retire-basic-memory.md) retires Basic Memory entirely on
  the strength of this one.
- Notes are committed, so they arrive in pull requests as readable
  Markdown and a human can edit or delete them with a text editor.
- One file per note fits the incremental persistence seam: an edit rewrites
  exactly one file, never the whole log. See the Tirith contract
  "Persistence seam for primitives" v2.
- The frontmatter parser is a hand-written subset of YAML rather than a new
  dependency. It must stay strict and must never panic: unparseable files
  are `MemoryError`, reported with a line number. This is the main
  maintenance cost of the decision and is covered by unit tests.
- `.tirith/memory/` joins `contracts/` as a committed directory, so the
  storage tree in `docs/1-about/02-architecture.md` changes. Unlike
  `contracts/`, it may contain subdirectories, so the loader walks it
  recursively and the writer creates parent directories.
