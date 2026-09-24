---
name: tirith
description: Coordinate every edit in this repository through its own Tirith daemon — claim paths before editing, read the brief, publish contracts and change notices, message other agents, release, record decisions and memory notes. Use at the start of any task that edits files here, when a claim is refused or reported lost, when spawning agents (swarm lead), when the tirith MCP server is unavailable, and before reporting a task done.
---

# Tirith on Tirith

Tirith coordinates its own development. Editing without a claim is a rule violation, not a fallback. Full protocol: [docs/6-agent-workflow/03-tirith-dogfooding.md](../../../docs/6-agent-workflow/03-tirith-dogfooding.md); tool reference: [docs/1-about/04-primitives.md](../../../docs/1-about/04-primitives.md).

## Reach the daemon

`.mcp.json` registers `tirith stdio`, which starts the repository's daemon at `http://127.0.0.1:7477` (dashboard at `/`) when none runs. If the MCP server shows as unavailable, the binary is not on PATH:

```bash
cargo install --path . --locked        # --force to overwrite the same version
```

Then reconnect the MCP server, or use the CLI, which speaks the same protocol (`tirith status` reads `.tirith/runtime/daemon.json`; start one by hand with `tirith serve --root .` in the background if none answers):

```bash
tirith --agent <name> claim --ttl 3600 -r "<why>" <paths...>
tirith --agent <name> release
tirith --agent <name> guide            # the protocol as the daemon states it
```

If the daemon cannot be reached or started, stop and say so in the report.

## The loop

1. **Identify.** One stable `agent` name per session (`claude-<task>`, `cursor-<task>`); use it in every call.
2. **Claim before editing** (`claim`, with a reason another agent can read). The `ok` reply is your brief: unread notices, contracts, decisions and memory notes for those paths, five newest each; `more` says whether to page with the list tools. On `conflict`, do not edit: wait with `wait_secs` (up to 120) or pick other work. Hold shared files (`src/server.rs`, `src/state.rs`, `src/lead.rs`, `docs/1-about/04-primitives.md`) only while editing them: prepare, claim with `wait_secs`, write, check, release.
3. **Contract before interface work** (`contract_publish`) when another agent will consume a `State` method, a tool schema or a store format.
4. **Notice on every breaking change** (`notice_publish`) for renames, signature changes, removed items and moved files, listing the affected paths (`find_referencing_symbols` in Serena finds them).
5. **Stay alive.** Any call renews leases; a lease still ends after four TTLs. A response carrying `lost` means stop editing those paths and claim again.
6. **Talk through Tirith** (`message_send` to a name or `*`); replies arrive as `inbox` on your next call. Never rely on a client's own session messaging.
7. **Check** with `scripts/check.sh --quick` while holding claims and the full `scripts/check.sh` once before releasing. Read the digest, not the raw log.
8. **Release** (`release`), then `decision_record` for settled choices and `memory_write` for what you learned about the paths you touched, if the docs do not already say it.

The report names the claims held, the notices published and any claim refused.

## Swarm lead

A session that spawns agents claims `.tirith/lead` first (`ttl_secs: 3600`, reason naming the swarm), re-claims it on `lost`, releases it last, and is the only one that messages `human`. Workers never claim `.tirith/lead`; they escalate to its holder (`status` reports it as `lead`) with `message_send`, and wait for work with `task_pull` and `wait_secs` instead of sleeping. Rules: [ADR-0027](../../../docs/5-decisions/0027-swarm-lead-and-escalation.md), [ADR-0028](../../../docs/5-decisions/0028-claim-aware-task-pull.md).

## Memory routing

Knowledge about specific paths is a Tirith memory note (`memory_write`, committed under `.tirith/memory/`); start a session with `memory_search` and no query. Anything a human reader needs goes in `docs/` instead. Serena memories are entry points only. Rules: [docs/6-agent-workflow/02-memory.md](../../../docs/6-agent-workflow/02-memory.md).
