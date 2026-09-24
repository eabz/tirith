# ADR-0035: Agent conventions shared with the other repositories

**Status:** Accepted, 2026-09-23, by the owner's decision.

## Context

The owner's other repositories converged on one layout for coding
agents: a short `AGENTS.md` that names the environment, the mandatory
reading, the key rules and the skills, with every long-form rule in
`docs/`; a skill set under `.agents/skills/` that `.claude/skills` links
to, identical across repositories except for one repository-specific
file; a committed `.claude/settings.json` that enables the project MCP
servers and denies reads of large generated files; a graphify knowledge
graph consulted before reading code; and editor memories that only
point at `AGENTS.md` and the docs.

Tirith had grown the opposite way. `AGENTS.md` was 9 KB and restated
the Rust rules, the layout rules and the git rules that `docs/` already
held. `CLAUDE.md` carried its own notes. `.serena/memories/` held five
dated status and procedure notes, two of them about a push that ended on
2026-09-16, one duplicating ADR-0016 and the shim's source. The e2e
benchmark kit under `examples/` and `examples/swarm_bench.rs` were
measurement harnesses from the v1 push whose numbers live in the ADRs
that used them.

## Decision

1. **Skills.** `.agents/skills/` holds `less-code` and `dead-code`,
   copied verbatim from the other repositories, and `tirith`, this
   repository's own skill for coordinating through its daemon. The
   marketing, UI and SEO skills of the web repositories are not
   installed: nothing here is a web product. `.claude/skills` is a
   symlink to `.agents/skills`. Where a shared skill names `bun run
   check` or knip, this repository runs `scripts/check.sh`; `cargo
   machete` and clippy's dead-code lints are the equivalents.
2. **`AGENTS.md` is short and links.** It names the environment, the
   commands, the mandatory reading, the key rules in one paragraph, the
   skills, the token rules (graphify, Serena, `memory_search`) and the
   Tirith requirement. The full definition of done moved to
   [../6-agent-workflow/04-definition-of-done.md](../6-agent-workflow/04-definition-of-done.md).
   `CLAUDE.md` only imports `AGENTS.md`.
3. **`.claude/settings.json` is committed** and enables `tirith`,
   `serena` and `graphify` from `.mcp.json`, and denies reads of
   `Cargo.lock`, `target/`, `.tirith/runtime/`, the graph files and
   images. `.gitignore` keeps everything else under `.claude/` local.
4. **graphify.** `.mcp.json` registers `graphify-mcp` on
   `graphify-out/graph.json`. The graph covers the code, `docs/`, and the
   committed `.tirith/` decisions and memory notes, because the decision
   trail is what an agent most often needs to find; `.graphifyignore`
   keeps agent config, `.tirith/runtime/`, HTML and images out.
   `graphify-out/` is gitignored and rebuilt per machine with `graphify
   update .`, which needs no LLM for code; documents changed since the
   last build need a semantic pass (`/graphify --update`).
5. **Serena memories are entry points.** One memory, `repo-map`, says to
   read `AGENTS.md` and the docs index. Facts about paths are Tirith
   memory notes; facts everyone needs are docs; session state is the
   Tirith board.
6. **Removed.** `.cursor/mcp.json` (clients other than Claude Code add
   `tirith stdio` to their own config, as
   [../2-examples/02-client-setup.md](../2-examples/02-client-setup.md)
   shows), `examples/e2e/` and `examples/swarm_bench.rs` (the benchmarks
   will be redone; the ADRs keep the numbers they produced), the memory
   notes whose paths were those files, and ten memory notes that were
   dated status reports or restated an ADR, a contract, or a doc
   (kickoff, pre-alpha, v1 audit, mutation gaps, check.sh digest, tool
   schema budget, release container, upsert fix, claim-aware pull,
   symbol-anchor measurements).

## Alternatives

- **Install the whole shared skill set** for uniformity. Rejected: six
  of the eight skills are about web UI, SEO and school marketing, and
  every description an agent loads costs tokens on every turn.
- **Keep the procedures in Serena memories.** Rejected: a memory that
  states a fact goes stale silently; the same text in `docs/` is checked
  when the behavior changes.
- **Commit `graphify-out/`.** Rejected: the graph is derived from the
  tree and the other repositories do not commit it either.

## Consequences

- `AGENTS.md` is under 5 KB; the rules it summarises are binding through
  the linked docs, which did not lose content.
- A new skill is a folder under `.agents/skills/` plus a row in its
  README; `less-code` and `dead-code` are updated by copying from the
  other repositories, never edited here.
- The cargo unit and integration tests stay the definition of done; only
  the LLM-driven coordination benchmark and the load benchmark are gone.
- `.claude/settings.json` denies reads of `graphify-out/.graphify_*`, so
  graphify's semantic extraction subagents write their chunk files
  outside the repository and the build copies them in.
