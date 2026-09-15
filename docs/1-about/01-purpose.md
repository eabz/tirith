# Purpose

Tirith exists to let 5 to 10 coding agents work in one repository at the
same time without corrupting each other's work.

## The three failure modes it targets

1. **Concurrent edits.** Two agents change the same file; the later write
   wins and the earlier work is lost or half-merged.
2. **Renames under someone's feet.** One agent renames or re-signs a
   function while another agent is mid-way through code that calls it.
3. **Interface drift.** Two agents build the two sides of an interface to
   different shapes because the shape was never written down before work
   began.

Claims and the task board address the first failure and are table stakes.
Contracts and change notices address the second and third, and they are
the reason Tirith exists: there is no good workaround for them today.

## What Tirith is not

- Not a memory layer. It does not store knowledge about the codebase,
  conversation history, or embeddings. Use a memory server for that
  (see [../6-agent-workflow/02-memory.md](../6-agent-workflow/02-memory.md)).
- Not an orchestrator. It does not start, stop, or schedule agents. Agents
  pull work; nothing pushes work at them.
- Not a lock on the filesystem. A claim is a social contract enforced by
  cooperating agents that ask before editing. Tirith cannot stop an agent
  that never asks.

## Who it is for

Primarily its author, running many agents from Claude Code, Cursor, and
scripts on the same projects. It is open source under MIT so anyone with the
same problem can use it, but design decisions favor a single-user, single
machine, many-agent setup over multi-tenant deployment.

## Roadmap

| Milestone | Scope | Status |
|---|---|---|
| 1 | `tirith serve`, the `claim` / `release` / `renew` / `claims_list` tools, a CLI, and a demo where two agents claim overlapping files and the second is refused | In progress |
| 2 | Task board, `tirith stdio` shim for stdio-only clients, Tirith used on its own repo | Planned |
| 3 | Contracts and change notices, with notices linked to contracts and claims | Planned |
| 4 | Decisions log, MCP resources for read-only views, optional glob claims | Planned |
