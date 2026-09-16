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
reads the daemon address from `.tirith/runtime/daemon.json`. Stop it with
`kill $(python3 -c "import json; print(json.load(open('.tirith/runtime/daemon.json'))['pid'])")`.

The dashboard is at `http://127.0.0.1:7477/`. After installing a new
Tirith version, stop the old daemon so the next session starts the new one.

Runtime state lands in `.tirith/runtime/` (gitignored). Contracts, notices,
and decisions land in `.tirith/` and are committed.

## Protocol for every agent

1. **Identify.** Pick a stable `agent` name for the session, e.g.
   `claude-claims-refactor` or `cursor-docs`. Use it in every call.
2. **Read before acting.** `notice_list` for the paths you will touch,
   `contract_list` for the interfaces you will implement or consume,
   `decision_list` for the area.
3. **Claim before editing.** `claim` the files or directories, with a
   reason another agent can understand. On `conflict`, do not edit; either
   pull a different task or wait for the lease to end.
4. **Contract before interface work.** If your change creates or changes
   something another agent will call (a `State` method signature, a tool
   schema, a store format), `contract_publish` it first.
5. **Notice on every breaking change.** `notice_publish` for renames,
   signature changes, removed items, and moved files, listing affected
   paths. `find_referencing_symbols` in Serena tells you the affected paths.
6. **Renew during long work.** If a task runs longer than the TTL, call
   `renew` or any other tool.
7. **Release when done.** `release` all claims. Record settled choices with
   `decision_record`.
8. **Report.** Say which claims you held, which notices you published, and
   whether any claim was refused.

## Why this matters here

Tirith's own development is the first real test of its primitives. If the
protocol is annoying for agents building Tirith, it will be annoying for
everyone. Friction found here becomes an issue or an ADR, not a workaround.
