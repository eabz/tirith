# Memory layers

There are two, with distinct jobs, and one rule for what goes where.

**The rule is the path.** If you can name the files the knowledge is
about, it is a Tirith memory note. If you cannot, it is a Serena memory.

## Layer 1: Tirith memory notes

- Location: `.tirith/memory/<permalink>.md`, committed to git.
- Content: durable knowledge tied to repository paths. A lesson, a trap,
  a design note, the state of unfinished work when a lease expires.
- Tools: `memory_write`, `memory_read`, `memory_search`.
- Good for: "editing `src/store.rs` means the persister's sequence
  ordering, here is what bit me", "`src/server.rs` must never branch on
  domain rules, I tried and it fell apart", "here is why we compared four
  storage formats and picked this one".

Notes are Markdown files with YAML frontmatter. The body carries prose,
plus two optional line formats:

```markdown
- [design] Writes go through spawn_blocking #storage
- follows [[pre-alpha-build]]
```

The first is an observation: a category, text, and hashtags. The second is
a relation to another note, by permalink or title. Both are plain enough
that notes import from other Markdown memory tools without translation;
the notes in this repository arrived that way. See
[ADR-0011](../5-decisions/0011-memory-primitive.md).

Why this layer lives inside a coordination server: a note carries the
paths it is about, and agents must claim paths before editing them, so the
note can reach the agent that is about to touch that code. A standalone
memory server cannot do that, because it does not know what anyone is
about to edit, so it depends on an agent remembering to search. This one
does not.

Start a session with `memory_search` and no query. That is recent
activity, newest first.

## Layer 2: Serena memories

- Location: `.serena/memories/`, committed to git.
- Content: short facts for navigating and building the project, not tied
  to any one path.
- Tools: `list_memories`, `read_memory`, `write_memory`, `edit_memory`.
- Good for: "milestone 1 is claims only", "rmcp needs feature X for the
  HTTP server", "integration tests need their own `allow` attribute
  because they are separate crates".

Read the relevant ones at session start. Write one when you learn
something non-obvious that no file in the repository would tell you.

## What goes where

| Information | Where |
|---|---|
| How the code works | The code and its doc comments |
| Design decisions about Tirith | `docs/5-decisions/` |
| Behavior, usage, rules | `docs/` |
| A lesson, trap, design note, or handoff about specific paths | Tirith memory notes |
| A short fact for navigating or building, not tied to a path | Serena memory |
| Coordination state while working (who edits what, what changed) | Tirith itself |

If a note would help a human reader of the repository understand how to
use Tirith, it is documentation, not memory. Put it in `docs/`.

Do not store anything in memory that belongs in `docs/` or in an ADR.
Memory is for what the repository does not already say.
