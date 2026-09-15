# Using Tirith on Tirith

From milestone 2 onward, agents working on this repository coordinate
through a Tirith daemon running in this repository. Until then this page
describes the intended protocol; agents should follow the spirit of it by
stating in their report which files they touched.

## Setup

```bash
cargo run -- serve          # from the repo root; binds 127.0.0.1:7477
claude mcp add --transport http tirith http://127.0.0.1:7477/mcp
```

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
