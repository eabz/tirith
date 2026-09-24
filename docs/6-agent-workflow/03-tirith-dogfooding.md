# Using Tirith on Tirith

**Requirement.** Every agent that edits this repository coordinates
through a Tirith daemon running for this repository. Claim before editing,
publish notices for breaking changes, release when done. An agent that
cannot reach the daemon and cannot start it must stop and say so; editing
without a claim is a rule violation, not a fallback.

## The daemon

Tirith is registered in `.mcp.json` as the stdio server `tirith stdio`
(other clients add the same command to their own MCP config). The shim
starts the daemon at `http://127.0.0.1:7477` on the first session if none
is running, and nothing has to be started by hand. To start or inspect it
anyway:

```bash
tirith serve                        # or: cargo run --quiet -- serve
tirith status                       # reads .tirith/runtime/daemon.json
```

The dashboard is at `http://127.0.0.1:7477/`. State lives in this repo's
`.tirith/` directory: `runtime/` (leases, task board, seen marks,
messages, daemon record) is gitignored; `contracts/`, `memory/`,
`notices.jsonl`, and `decisions/` are committed so the next session
inherits them.

After installing a new Tirith version nothing needs stopping: the next
session's shim sees that the running daemon reports another version,
stops it cleanly, and starts the new one
([ADR-0016](../5-decisions/0016-shim-replaces-stale-daemon.md)); the
restart is written to `.tirith/runtime/serve.log`. The one case it cannot
detect is a daemon built from a working tree with the same version string
as the installed binary. Stop that one by hand: install with
`cargo install --path . --locked --force` (a plain install refuses an
equal version), read the pid from `.tirith/runtime/daemon.json` and
`kill -INT <pid>`. Shutdown flushes pending writes, gives open streams at
most 2 s, removes `daemon.json` and the registry entry, and exits; wait
until the pid is gone before starting anything, so the old daemon cannot
delete the new record. The next MCP session starts the new daemon, or
`nohup tirith serve >> .tirith/runtime/serve.log 2>&1 &` does. Old shims
reconnect on their own; sessions see the new tool list after their MCP
client reconnects. If `scripts/check.sh` fails its last step, a test left
a `tirith serve --root /var/folders/...` daemon behind: `pgrep -fl "tirith
serve"` finds it.

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
   wait with `wait_secs` (up to 120) so the server retries the moment the
   lease ends, instead of sleeping and retrying yourself.

   **Hold shared files only while editing them.** Files many tasks touch
   (in this repository `src/server.rs`, `src/state.rs`, `src/lead.rs`,
   `docs/1-about/04-primitives.md`, and their like) are claimed in edit
   windows: read and prepare the change without a claim, claim the file
   with `wait_secs`, write it in one go, run the quickest relevant check,
   and release it. Files only your task touches can be held for the whole
   task. Measured on four end-to-end runs (ADR-0029): shared-file holds
   were 4,187 s in total but only 122 s of editing, and 57% of all waiting
   happened after the file was already free; edit windows with
   `wait_secs` would have removed most of the 2,762 blocked worker-seconds.
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
7. **Check cheaply while holding claims, fully before releasing.** While
   editing, `scripts/check.sh --quick` runs the unit tests, then the
   doctests, in a few seconds. Before `release`, run the full
   `scripts/check.sh` once. A failing step prints a digest with the
   panic or compiler messages and keeps the raw log in
   `target/check-<step>.log`, so read the digest instead of rerunning
   the step. Other agents compile the same tree: a compile error in a
   file you do not hold is theirs, not yours; wait or message the
   holder.
8. **Release when done.** `release` all claims. Record settled choices
   with `decision_record`, and write what you learned about the paths you
   touched with `memory_write` ([02-memory.md](02-memory.md)).

## The swarm lead

When one session spawns other agents, that session is the swarm lead
([ADR-0027](../5-decisions/0027-swarm-lead-and-escalation.md)), and Tirith
knows it by one reserved claim:

1. **Claim `.tirith/lead` first**, before spawning anyone, with
   `ttl_secs: 3600` and a reason naming the swarm. `status` then reports
   you as `lead`, and the dashboard shows it.
2. **Workers never claim `.tirith/lead`.** Say so in the brief you give
   every spawned agent, together with your agent name, which is where they
   send escalations with `message_send`.
3. **Keep it.** Any call renews the lease, but like every lease it ends
   after four TTLs. When a response carries `lost` for `.tirith/lead`,
   claim it again at once; until then the swarm has no lead.
   **Read your inbox.** The daemon routes workers' escalations (a task
   set to `blocked`, the third refusal of the same claim within 360 s) to
   your inbox as messages from `tirith`, tagged "may need the human:
   credentials" (or permissions, spending, destructive) when the text
   matches a human rule. A tag is a hint; nothing reaches the human on its
   own while you hold the lead. Answering a worker by `message_send`, the
   task leaving `blocked`, or the refused claim being granted marks the
   escalation answered.
   **Relay to the human on purpose.** When only the human can settle
   something (a login, a key, a spend, an irreversible step, a product
   decision), send `message_send` to `human` with a text written for the
   human: what is needed, why, and what happens meanwhile. It becomes an
   item in the human queue (`tirith lead human`, the dashboard's "Needs
   you", the macOS tray), and the human's reply comes back to you as a
   message from `human`. Never forward done reports there. Workers never
   message `human`; they escalate to you. While there is no lead, blocked
   tasks and repeated refusals go to the human queue directly.
4. **Release it last**, after the workers have finished.

**Idle workers wait on the board, not on the turn.** A worker with nothing
to do calls `task_pull` with `wait_secs` (up to 120) instead of ending its
turn or sleeping, and calls it again while the answer is `none` and
`task_list` with status `todo` still shows tasks; with no `todo` task left
it finishes. A task whose paths overlap only another agent's in-progress
task comes back at once, with `waiting_on`. While every `todo` task is
under another agent's claim or waits on dependencies, the daemon holds the
call until a claim is released or expires, or a task is created, pulled,
or changes status, then pulls as usual; with no `todo` task at all it
answers `none` at once. On timeout it returns what a plain pull would: `none`, or a
claimed task with `waiting_on`. The worker claims the paths in `waiting_on`
with `wait_secs` when it reaches them. Put this in the brief for every
spawned worker, so a worker whose next task is blocked picks it up the
moment it unblocks (ADR-0028, revised).

The report at the end of a task names the claims held, the notices
published, and any claim that was refused (AGENTS.md section 2).

## Why this matters here

Tirith's own development is the first real test of its primitives. If the
protocol is annoying for agents building Tirith, it will be annoying for
everyone. Friction found here becomes an issue or an ADR, not a workaround.
