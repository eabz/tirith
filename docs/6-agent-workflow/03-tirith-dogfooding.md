# Using Tirith on Tirith

**Requirement.** Every agent that edits this repository coordinates
through a Tirith daemon running for this repository. Claim before editing,
publish notices for breaking changes, release when done. An agent that
cannot reach the daemon and cannot start it must stop and say so; editing
without a claim is a rule violation, not a fallback.

Tirith is registered for Claude Code in `.mcp.json` and for Cursor in
`.cursor/mcp.json` as the stdio server `tirith stdio`, which starts the
daemon at `http://127.0.0.1:7477` if it is not already running. Nothing
has to be started by hand.
Its state lives in this repo's `.tirith/` directory: runtime files are
gitignored, while `contracts/`, `notices.jsonl`, and `decisions.jsonl` are
committed so the next session inherits them.

## Setup

Nothing, normally: the shim starts the daemon on the first session. To
start it by hand anyway:

```bash
tirith serve                        # or: cargo run --quiet -- serve
```

Check it with `tirith status` (or `cargo run --quiet -- status`), which
reads the daemon address from `.tirith/runtime/daemon.json`.

The dashboard is at `http://127.0.0.1:7477/`. After installing a new
Tirith version nothing needs stopping: the next session's shim sees that
the running daemon reports another version, stops it cleanly, and starts
the new one (see
[../5-decisions/0016-shim-replaces-stale-daemon.md](../5-decisions/0016-shim-replaces-stale-daemon.md)).
The restart is written to `.tirith/runtime/serve.log`. The one case that
is not detected is a daemon built from a working tree with the same
version string as the installed binary; stop that one by hand with the
pid in `daemon.json`.

Runtime state lands in `.tirith/runtime/` (gitignored). Contracts, notices,
and decisions land in `.tirith/` and are committed.

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
   paths. `find_referencing_symbols` in Serena tells you the affected paths.
5. **Renew during long work.** If a task runs longer than the TTL, call
   `renew` or any other tool. A lease also ends after four TTLs however
   active you are; only `claim` or `renew` restarts that clock. If a
   response carries `lost`, the lease on those paths ended: stop editing
   them and claim them again.
6. **Release when done.** `release` all claims. Record settled choices with
   `decision_record`.
7. **Report.** Say which claims you held, which notices you published, and
   whether any claim was refused.

## Why this matters here

Tirith's own development is the first real test of its primitives. If the
protocol is annoying for agents building Tirith, it will be annoying for
everyone. Friction found here becomes an issue or an ADR, not a workaround.
