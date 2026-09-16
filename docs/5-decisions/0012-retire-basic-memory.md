# ADR-0012: Basic Memory is retired; Tirith memory notes are the only long-form memory

**Status:** Accepted, 2026-09-16. Supersedes the "Basic Memory keeps its
job for notes that are not about this repository's paths" consequence of
[ADR-0011](0011-memory-primitive.md).

## Context

Basic Memory was adopted on 2026-09-15 as the long-form memory layer,
registered in `.mcp.json` and `.cursor/mcp.json`, with notes committed
under `.memory/`. ADR-0011 then gave Tirith its own memory primitive and
kept Basic Memory for knowledge "not about this repository's paths".

After one day the repository held two Basic Memory notes, 66 lines in
total, neither touched since the day they were written. Every observation
in them names a file: the rmcp client retry trap is about `src/cli.rs`,
the offline installer test is about `dist-workspace.toml`, the crates.io
name clash is about `Cargo.toml`. By the rule in
`docs/6-agent-workflow/02-memory.md`, all of it belongs in Tirith memory.
Nothing path-free ever arrived.

Keeping the layer costs a second MCP server in every session, a Python
tool each contributor must install and point at the repository, a search
index outside the repository that is rebuilt per machine, and a third
answer to "where does this go", which ADR-0011 already found "held up
badly in practice".

## Decision

- Basic Memory is removed from the repository: the `basic-memory` entries
  in `.mcp.json` and `.cursor/mcp.json`, the `.memory/` directory, and the
  setup instructions.
- The two existing notes move into `.tirith/memory/design/`. Their
  permalinks drop the `tirith/` prefix, which was Basic Memory's project
  namespace and has no meaning inside `.tirith/memory/`; the loader takes
  the permalink from frontmatter, so file path and permalink must agree.
  Basic Memory's `type: note` becomes `kind: research`, and each note
  gains the `paths:` list that makes it reachable by claim, which Basic
  Memory had no field for. Body text is untouched.
- Two memory layers remain. Serena memories hold short navigation and
  workflow facts. Tirith memory notes hold everything else, scoped to the
  paths they are about, or to `docs/` when they are about the project as a
  whole.
- ADR-0011's format compatibility promise stands as an import guarantee:
  notes written by Basic Memory, or by hand in the same shape, parse
  without conversion. The integration test that proved it now reads the
  migrated notes in `.tirith/memory/`.

## Alternatives

- **Keep Basic Memory for cross-project knowledge.** Rejected for the
  repository. That knowledge is personal, not project state, so it belongs
  in a Basic Memory project on the contributor's own machine, not in the
  repository's MCP registration.
- **Keep it for semantic search.** Rejected. Two notes do not need
  embeddings, and ADR-0011 already deferred embeddings inside Tirith
  behind a cargo feature should term scoring fall short.
- **Keep `.memory/` as an import inbox.** Rejected. An empty directory
  with a rule attached is exactly the kind of ambiguity this decision
  removes; a file can be dropped straight into `.tirith/memory/`.

## Consequences

- One fewer server to install and approve per session. New-machine setup
  is `cargo install`, Serena, and nothing else.
- `AGENTS.md`, `README.md`, `docs/README.md`, `docs/1-about/03-project-structure.md`,
  and `docs/6-agent-workflow/02-memory.md` describe two layers.
- `tests/memory_layer.rs` keeps its Basic Memory fixture test, pointed at
  `.tirith/memory/`.
- Contributors who had run `basic-memory project add tirith` can remove
  that project; nothing in the repository refers to it any more.
- Until the memory tools are wired into `state.rs`, `store.rs`, and
  `server.rs` (blocked on the incremental persistence work of ADR-0010),
  the migrated notes are committed Markdown that no MCP tool can read or
  write. Nothing is lost in that window; `memory_search` simply does not
  exist yet.
