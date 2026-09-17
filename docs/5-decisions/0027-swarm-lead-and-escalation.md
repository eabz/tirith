# ADR-0027: Every swarm has a lead; escalations, the human queue, and the lead log are deterministic

**Status:** Accepted, 2026-09-17.

## Context

Tirith has had no notion of a lead: every agent pulled tasks and resolved
conflicts on its own, and anything it could not resolve waited for the
human. In practice one session always acts as lead: the conversation the
human starts, which spawns the other agents. Measured in 17 sessions on
this repository, agents spent 19–22% of their wall time waiting on the
human. 76 of 78 of those waits followed a "done" or status report, and
only 2 followed a question. In the e2e benchmark runs, 20–29% of summed
agent time went to foreground sleeps before retrying refused claims.

Routine events (a blocked task, a claim refused again and again, a notice
that breaks what someone holds) need a place to go that is not the human,
and must not cost an LLM turn each. What the daemon does on the lead's
behalf also has to be auditable afterwards.

## Decision

### 1. Two halves of the lead

- **The lead agent** is a session: an LLM agent, or a human at the CLI. It
  holds the lead lease, starts the swarm, receives escalations, and answers
  them or passes them to the human.
- **The lead policy** is deterministic code in the daemon (`src/lead.rs`).
  It acts on the lead's behalf on routine events: waiting `task_pull`
  calls, notice push, escalation triggers and routing, the human queue. It
  writes the decision log and never needs an LLM turn or a network call.

### 2. Who the lead is

- The lead agent is **whoever holds the lease on the reserved path
  `.tirith/lead`**. It is an ordinary claim, so exclusivity, expiry, `lost`
  reporting and `previous_owner` all come free and no new MCP tool is
  needed (the `tools/list` budget has little room left).
- **Protocol:** the session that spawns other agents claims `.tirith/lead`
  first, with `ttl_secs: 3600` and a reason naming the swarm. Workers never
  claim it. `AGENTS.md` and the dogfooding doc say so, and the worker brief
  in the lead's spawn prompt repeats it.
- **Keeping it:** any call renews the lease, and like every lease it ends
  after four TTLs, so a long-running lead re-claims when told `lost`. When
  the lease ends the swarm has **no lead**: escalations go to the human
  queue until someone claims it.
- **Reporting:** `status` reports `lead` (the holder, or null) and the
  dashboard shows it.

### 3. Escalations

An escalation is raised by:

- `task_update` to `blocked`, whose note is the reason;
- the third refusal of the same claim (same agent, same paths) within the
  refusal window, three full claim waits (360 s);
- a message to the lead **only** when its text matches a human rule
  (below). Any other message to the lead is ordinary conversation.

Routing is a fixed rule over the escalation's own text (the note, the
message, or the refused claim's reason and paths):

| Condition | Route |
|---|---|
| a human rule matches: credentials, permissions, spending, destructive or irreversible operations, or explicitly addressed to the human | the human queue, and the lead agent is told |
| no human rule matches, and the swarm has a lead | the lead agent's inbox, as a message from `tirith` |
| the swarm has no lead, or the lead is the agent escalating | the human queue |

The phrase list is `lead::HUMAN_RULES`, matched at word boundaries. Workers
never see routes or rules.

**The human queue** is every escalation delivered to it and not yet
answered, ranked by how many agents it blocks, then by how long it has
waited. It is the dashboard's "Needs you" list, `/api/human`,
`tirith lead human`, and a count and notification in the macOS tray. An
escalation counts as answered when anyone messages the escalating agent,
its task leaves `blocked`, or the refused claim is granted.

### 4. Decision log

- **Where:** lead policy decisions are appended to
  `.tirith/runtime/lead_log.jsonl` by the persister in the background. It
  is runtime state, gitignored, and kept for 7 days.
- **Rows, deterministic only:** the claim lifecycle (granted, refused,
  waited, released, lease ended), notice pushes, and escalations. Each row
  carries `id`, `at`, `event`, `seq` (the state sequence number read),
  `candidates`, `rule` (the rule that decided), `action` (what was done
  through Tirith), optional `agent` and `details`, and `outcome`, filled in
  later when observable (for an escalation: who answered, how, and after
  how long).
- **Access:** through the dashboard (`/api/lead`) and the CLI
  (`tirith lead log`), not through a new MCP tool.
- **Purpose:** auditing what the daemon did on the lead's behalf, and
  measuring how long escalations wait.

## Alternatives

- **A new `lead_acquire` tool.** Rejected: the schema budget, and a
  reserved claim gives expiry and reporting for free.
- **The first agent seen is the lead, implicitly.** Rejected: it breaks on
  reconnects and when a worker happens to call first; leadership should be
  an explicit act.
- **An LLM lead handles every event.** Rejected: a turn per routine event
  is the cost this ADR exists to avoid.
- **Every message to the lead is an escalation.** Rejected: most messages
  are conversation; logging and queueing them adds noise without routing
  anything the inbox does not already deliver.
- **A model classifies escalations** (answerable from records, lead,
  human). Rejected: the fixed rules route the cases that matter (the human
  must see credentials, spending and destructive steps; everything else
  goes to the lead), and a classifier adds latency, cost and an external
  dependency without a measured gain.
- **Auto-continue on every stop.** Deferred behind measurement: a wrong
  continue costs work quality, and Stop hook blocking still needs a live
  check.

## Consequences

- One reserved path, `.tirith/lead`, gets special meaning in `status`, the
  dashboard and the escalation router; overlap rules are unchanged.
- The human sees fewer, ranked escalations. The lead agent sees more
  inbox traffic and must read it.
- Routing is predictable and testable: the same text and board always take
  the same route (`tests/lead_escalation.rs`).
- A human rule that misses a phrasing sends that escalation to the lead
  instead of the human; the lead agent is expected to pass it on.
