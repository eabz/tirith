# Purpose

Tirith exists to let 5 to 10 coding agents work in one repository at the
same time without corrupting each other's work.

## The four failure modes it targets

1. **Concurrent edits.** Two agents change the same file; the later write
   wins and the earlier work is lost or half-merged.
2. **Renames under someone's feet.** One agent renames or re-signs a
   function while another agent is mid-way through code that calls it.
3. **Interface drift.** Two agents build the two sides of an interface to
   different shapes because the shape was never written down before work
   began.
4. **Knowledge that dies with the session.** An agent learns why a change
   is hard, finishes, and exits. The next agent re-learns it, or does not,
   and repeats the mistake.

Claims and the task board address the first failure and are table stakes.
Contracts and change notices address the second and third, and they are
the reason Tirith exists: there is no good workaround for them today.

Memory notes address the fourth. A general memory server can store the
knowledge, but it cannot know which agent needs it, because it does not
know what anyone is about to edit. Tirith does, because agents claim paths
before editing them, so a note scoped to a path can reach the agent
claiming it. Coordination is what makes the memory useful. See
[ADR-0011](../5-decisions/0011-memory-primitive.md).

Messages between agents are the small channel the rest needs: "take task
X", "I released server.rs". They travel through the daemon so every
client hears them, not only the one with its own session chat. See
[ADR-0020](../5-decisions/0020-agent-messages.md).

## What Tirith is not

- Not a general-purpose memory server. Tirith does keep durable knowledge,
  but only knowledge tied to this repository: decisions, and memory notes
  scoped to paths (see below). It stores no conversation history and no
  embeddings, and it is not a place to keep knowledge about anything other
  than this repository. See
  [../6-agent-workflow/02-memory.md](../6-agent-workflow/02-memory.md) for
  what belongs where.
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
| 1 | `tirith serve`, the claims tools, a CLI, JSON persistence, and a demo where two agents claim overlapping files and the second is refused | Done (0.1.0) |
| 1b | Initial versions of the task board, contracts, change notices, decisions log, and a web dashboard at `/` | Done (0.1.0) |
| 2 | `tirith stdio` shim that starts the daemon on demand, Tirith used on its own repo | Done (0.1.3) |
| 2b | Agent-facing hardening from real multi-agent use: the brief on claim, lost-lease reporting, orphaned tasks, contract republish, the shim replacing stale daemons, agent messages | Done (unreleased, ships in v1) |
| 2c | Memory notes: the `memory_*` tools over committed Markdown files, and notes for a claimed path returned by `claim` | Done (unreleased, ships in v1) |
| v1 | Coordination, memory, and token budgets all enforced by tests; see [ADR-0013](../5-decisions/0013-v1-definition.md) | In progress |
| 3 | Contract shape validation (JSON Schema), notices linked to claims, richer dashboard filters | Planned |
| 4 | MCP resources for read-only views, optional glob claims | Planned |
