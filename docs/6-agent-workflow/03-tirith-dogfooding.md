# Using Tirith on Tirith

**Requirement.** Every agent that edits this repository coordinates
through a Tirith daemon running for this repository. Claim before editing,
publish notices for breaking changes, release when done. An agent that
cannot reach the daemon and cannot start it must stop and say so; editing
without a claim is a rule violation, not a fallback.

## The daemon

Tirith is registered for Claude Code in `.mcp.json` and for Cursor in
`.cursor/mcp.json` as the stdio server `tirith stdio`. The shim starts the
daemon at `http://127.0.0.1:7477` on the first session if none is running,
and nothing has to be started by hand. To start or inspect it anyway:

```bash
tirith serve                        # or: cargo run --quiet -- serve
tirith status                       # reads .tirith/runtime/daemon.json
```

The dashboard is at `http://127.0.0.1:7477/`. State lives in this repo's
`.tirith/` directory: `runtime/` (leases, task board, seen marks,
messages, daemon record) is gitignored; `contracts/`, `memory/`,
`notices.jsonl`, and `decisions.jsonl` are committed so the next session
inherits them.

After installing a new Tirith version nothing needs stopping: the next
session's shim sees that the running daemon reports another version,
stops it cleanly, and starts the new one
([ADR-0016](../5-decisions/0016-shim-replaces-stale-daemon.md)); the
restart is written to `.tirith/runtime/serve.log`. The one case it cannot
detect is a daemon built from a working tree with the same version string
as the installed binary. Stop that one by hand with the pid in
`daemon.json`; the Serena memory `workflow/daemon-restart` has the steps.

## Protocol for every agent

1. **Identify.** Pick a stable `agent` name for the session, e.g.
   `claude-claims-refactor` or `cursor-docs`. Use it in every call.
2. **Claim before editing.** `claim` the files or directories, with a
   reason another agent can understand. The `ok` reply carries a brief for
   those paths: unread notices, contracts, decisions, and memory notes,
   five newest of each, with `more` counts. Read it. Page with
   `notice_list`, `contract_list`, `decision_list`, or `memory_search`
   only when `more` says there is more, or for paths you are not
   claiming. On `conflict`, do not edit; either pull a different task or
   wait for the lease to end.
3. **Contract before interface work.** If your change creates or changes
   something another agent will call (a `State` method signature, a tool
   schema, a store format), `contract_publish` it first.
4. **Notice on every breaking change.** `notice_publish` for renames,
   signature changes, removed items, and moved files, listing affected
   paths. `find_referencing_symbols` in Serena tells you the affected
   paths. A notice is seen by an agent when Tirith delivers it, in a brief
   or an unread listing; there is nothing to acknowledge
   ([ADR-0021](../5-decisions/0021-notice-acks-log.md)).
5. **Renew during long work.** Any call renews your leases. A lease also
   ends after four TTLs however active you are; only `claim` or `renew`
   restarts that clock. If a response carries `lost`, the lease on those
   paths ended: stop editing them and claim them again.
6. **Talk through Tirith.** Coordination talk between agents goes through
   `message_send` (to an agent name, or `*` for everyone active in the
   last hour) and arrives as `inbox` on the recipient's next call, five
   at a time; `message_list` is the history. This works for every MCP
   client, unlike a chat app's own session messaging.
7. **Release when done.** `release` all claims. Record settled choices
   with `decision_record`, and write what you learned about the paths you
   touched with `memory_write` ([02-memory.md](02-memory.md)).

The report at the end of a task names the claims held, the notices
published, and any claim that was refused (AGENTS.md section 2).

## Why this matters here

Tirith's own development is the first real test of its primitives. If the
protocol is annoying for agents building Tirith, it will be annoying for
everyone. Friction found here becomes an issue or an ADR, not a workaround.
