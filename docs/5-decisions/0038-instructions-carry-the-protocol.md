# ADR-0038: The server instructions carry the protocol; `guide` is removed; `tirith init` prepares a repository

**Status:** Accepted, 2026-10-10, by the owner's decision. Supersedes the
`guide` tool of [ADR-0034](0034-complete-tool-definitions-and-guide.md);
the complete tool definitions of that ADR stand.

## Context

An agent has to know how to use Tirith before it makes its first call,
and the owner's first concern was how that knowledge reaches it. ADR-0034
put the protocol behind a `guide` tool: correct, but only read by an
agent that already knew to call it, and the server's short `instructions`
text had to say so. The owner also wants teams to run Tirith with a
protocol of their own, and asked for a command that makes a repository
ready in one step.

MCP has a channel built for exactly this. The `instructions` field of
the initialize result is placed by every client in front of its model,
once per session, with no tool call and no project file. This session's
own context carried Tirith's old instructions that way.

## Decision

1. **The instructions are the protocol.** `src/instructions.md`, embedded
   in the binary, is served as the MCP `instructions` on every handshake:
   what Tirith is, what every result carries, the working loop, and the
   two roles with the spawn rule, the model split and the worker brief.
   It is terse Markdown of about 2.7 KB, because every session pays for
   it; `tests/budgets.rs` holds it under 4 KB and checks that the client
   receives it. The stdio shim already
   forwards the daemon's instructions.
2. **`guide` is removed.** Its overview is the instructions; its per-tool
   "when" clauses already live in every tool's description (ADR-0033).
   `tools/list` drops from about 44,400 to 42,900 chars. A test in
   `server.rs` checks that every tool the instructions name exists, so a
   rename cannot leave a stale name in the protocol.
3. **A repository can serve its own protocol.** When
   `.tirith/instructions.md` exists and is not blank, the daemon serves
   it instead of the built-in text, read on every handshake so an edit
   needs no restart. That is how a team runs Tirith "with its own
   protocol": the tools stay, the text is theirs.
4. **`tirith init`** registers `tirith stdio` in `.mcp.json` (keeping
   every other server), puts a short marker-delimited block in the
   agents' file (`AGENTS.md` by default, `--file` for another) that says
   the protocol arrives on connect, and with `--instructions` copies the
   built-in protocol to `.tirith/instructions.md` once, for editing.
   Every step is idempotent.

## Alternatives

- **Keep `guide` next to the instructions.** Rejected: two copies of the
  same text that drift, and 1.5 KB of `tools/list` per session for a
  tool the instructions make unnecessary.
- **`tirith init` writes the whole protocol into `AGENTS.md`.** Rejected:
  every session would then carry the text twice, once from the client's
  project file and once from the server, and the file copy goes stale
  when the daemon is upgraded. The block only points at the server.
- **A protocol per client or per agent.** Rejected: the daemon has one
  protocol per repository, which is the unit teams share.

## Consequences

- A session that connects knows the protocol before its first call, and
  the lead knows when to spawn and what to tell a worker, without reading
  the docs.
- `tirith guide` is gone from the CLI. Humans read
  [../2-examples/03-swarm-playbook.md](../2-examples/03-swarm-playbook.md)
  or `cat src/instructions.md`.
- A repository with its own `.tirith/instructions.md` does not pick up
  upstream protocol changes; that is the point, and `tirith init
  --instructions` never overwrites it.
- Hosting the daemon for a team (behind a tunnel, or as a Worker) needs
  nothing more from this decision: the protocol is served by the
  endpoint agents connect to.
