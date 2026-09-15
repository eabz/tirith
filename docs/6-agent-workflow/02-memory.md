# Memory layers

Tirith is not a memory layer, so the agents building it need one from
elsewhere. There are two, with distinct jobs, and a rule for what goes
where.

## Layer 1: Serena memories (in use)

- Location: `.serena/memories/`, committed to git.
- Content: short project facts for navigation and workflow.
- Tools: `list_memories`, `read_memory`, `write_memory`, `edit_memory`.
- Good for: "the store writes go through `spawn_blocking` because of
  ADR-0003", "milestone 1 is claims only", "rmcp needs feature X for the
  HTTP server".

## Layer 2: Basic Memory (configured)

**Status: installed and configured on 2026-09-15.** Notes live **inside
the repo** at `.memory/` (repo root) as plain Markdown and are
committed, so every contributor's agents start with the same memory.
Registered for Claude Code in `.mcp.json` (project scope) and for Cursor in
`.cursor/mcp.json`, both restricted to the `tirith` project.

Decision: memory is shared project knowledge, not personal state, so it is
versioned with the code. The search index (SQLite) stays in
`~/.basic-memory/` and is rebuilt from the Markdown on each machine, so
nothing binary is committed. Personal notes that should not be shared
belong in a different Basic Memory project (for example `main`).

[Basic Memory](https://github.com/basicmachines-co/basic-memory) is a
local-first MCP memory server that stores everything as plain Markdown
files in a directory you choose, indexes them for semantic search, and
works with Claude Code, Claude Desktop, Cursor, Codex, and VS Code.

Why it fits this project:

- **Framework-agnostic**, like Tirith. Every agent we run can read and
  write the same notes.
- **Plain Markdown on disk.** Notes are readable and editable by a human,
  can be opened in Obsidian, and can be committed if we want.
- **Local and free.** No cloud account, no per-call quota.
- **Semantic search** so an agent can ask "what did we decide about lease
  renewal" without knowing the note's name.

Alternatives considered:

| Option | Why not |
|---|---|
| Mem0 / OpenMemory MCP | Extracts fragments automatically; good for chat products, less good for engineering notes you want to read. Free tier has retrieval caps, paid plans from $19/month |
| The reference `server-memory` knowledge graph | Zero-dependency and fine for small graphs, but entity-relation triples are awkward for design notes |
| Graphiti / Zep | Needs Neo4j; heavy for one developer |
| Only Serena memories | Adequate for facts, poor for long design notes and cross-project knowledge |

Setup on a new machine (the notes are already in the clone; this only
tells Basic Memory where they are):

```bash
uv tool install basic-memory
basic-memory project add tirith "$(pwd)/.memory"
```

The MCP registration is committed in `.mcp.json` (Claude Code) and
`.cursor/mcp.json` (Cursor), both running `basic-memory mcp --project tirith`.
Claude Code asks once to approve the project-scoped server.

Tools an agent will see: `write_note`, `read_note`, `edit_note`,
`search_notes`, `build_context`, `recent_activity`, and project management.
Use `search_notes` before starting design work and `write_note` when a
session produced reasoning worth keeping. Title notes by topic, add a
`tags` list, and link related notes with `[[Note Title]]`.

## What goes where

| Information | Where |
|---|---|
| How the code works | The code and its doc comments |
| Design decisions about Tirith | `docs/5-decisions/` |
| Behavior, usage, rules | `docs/` |
| Short facts an agent needs to navigate or build | Serena memory |
| Long design notes, research, comparisons, session journals | Basic Memory (`.memory/`) |
| Coordination state while working (who edits what, what changed) | Tirith itself |

If a note would help a human reader of the repo, it is documentation, not
memory. Put it in `docs/`.
