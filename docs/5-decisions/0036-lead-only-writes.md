# ADR-0036: Contracts, the board, decisions and deletions are the lead's while a swarm has one

**Status:** Accepted, 2026-10-10, by the owner's decision. Refines
[ADR-0027](0027-swarm-lead-and-escalation.md).

## Context

Every agent could call every tool. In a swarm that meant a worker could
publish a contract, change the shape another worker was already building
against, add tasks to the board the lead had planned, record a decision
nobody had reviewed, force another worker's task, or delete a note. The
contracts, the board and the decisions log are the lead's design of the
work; the primitives exist to stop interface drift and lost work, and an
open write surface let any worker cause both from inside the protocol.

Agent identity is a caller-supplied string
([ADR-0004](0004-caller-supplied-agent-identity.md)), so no gate here is
security against a hostile agent. The problem is drift by well-meaning
agents following their own judgment instead of the plan.

## Decision

1. **Five writes are lead-only while a swarm has a lead.** When an agent
   holds the lease on `.tirith/lead` ([ADR-0027](0027-swarm-lead-and-escalation.md))
   and the caller is another agent, these are refused:

   | Tool | Why it is the lead's |
   |---|---|
   | `contract_publish` | The shape every worker builds against |
   | `task_create` | The plan: what is independent, what depends on what |
   | `decision_record` | A settled choice needs the whole picture |
   | `task_update` with `force` | Taking a task from another agent is a reassignment |
   | `memory_delete` | The one destructive tool |

2. **The refusal is `conflict` naming the lead.** The result carries
   `lead` (the agent to message) and a `message` that says what to do:
   propose it with `message_send`. `conflict` already means "another
   agent holds what you need; coordinate", and the CLI, the output
   schemas and the clients already handle it; no new status was added.

3. **With no lead nothing is gated**, and the lead itself is never
   refused. A solo session, a human at the CLI, or a worker after the
   lead released see the same open surface as before.

4. **The worker's own loop is untouched:** `claim`, `task_pull`,
   `task_update` on its own task, `notice_publish`, `memory_write`,
   `message_send`, `release`. A worker reports and proposes; the lead
   records.

5. **Where it lives.** `Lead::require` in `src/lead.rs` is the rule;
   each gated `State` method calls it under the lock, and a refusal is
   the one `types::LeadOnly` error, carried transparently by the domain
   error of the tool, which `server.rs` maps to the `conflict`. `force`
   is refused only when it would take another agent's task; on a task
   the caller holds it is not a reassignment. The rule, the tool
   descriptions, the output schemas, the `guide` (rules, topics `lead`
   and `roles`) and
   [../1-about/04-primitives.md](../1-about/04-primitives.md) say so;
   `tests/lead_gates.rs` pins every gate, the lead's own calls, the
   no-lead case and the lift on release.

## Alternatives

- **A `forbidden` status.** Rejected: one more value in every status
  enum, the output schemas and the CLI for a case `conflict` already
  describes, and nothing a caller would do differently.
- **A daemon flag to turn the gates off.** Rejected: nobody asked for
  it, and "no lead, no gates" already covers every single-agent use.
- **Gate `notice_publish` and `memory_write` too.** Rejected: those are
  the worker's reports about what it did and learned; hiding them from
  the lead hides work.
- **Signed identity or a lead token.** Rejected: the daemon serves one
  user on one machine (ADR-0004); a gate against drift does not need a
  gate against forgery.

## Consequences

- A worker that used to create follow-up tasks now messages the lead,
  which creates them with `depends_on`; the plan stays in one place.
- The lead must read its inbox: proposals arrive there, and a worker
  blocked on a contract change waits for the lead's republish.
- `tools/list` grew by about 800 bytes to 44,424 of the 44,500 budget
  ([ADR-0034](0034-complete-tool-definitions-and-guide.md)); the next
  addition to the tool surface must trim text or raise the budget.
