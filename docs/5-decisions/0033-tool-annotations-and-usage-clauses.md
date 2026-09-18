# ADR-0033: Every tool carries MCP annotations and says when to use it

**Status:** Accepted, 2026-09-18. Supersedes in part
[ADR-0017](0017-tool-result-and-schema-budget.md): the `tools/list`
budget rises once, for structured hints and one usage clause per tool.
The per-parameter rule of ADR-0017 stands. Updates the `tools/list` row
of the table in [ADR-0013](0013-v1-definition.md).

## Context

ADR-0017 cut `tools/list` to one sentence per tool and no description on
any parameter, and pinned the result at 7,800 characters. Measured on
2026-09-18 it was 7,616 for 22 tools, with the three largest tools at
490 to 499 of a 500-character bound.

Glama scores every listed server's tools with the open Tool Definition
Quality Score (TDQS, glama-ai/tool-definition-quality-score). Its
weights are Purpose 25%, Usage Guidelines 20%, Behavioral Transparency
20%, Parameter Semantics 15%, Conciseness 10%, Completeness 10%. Tirith
scored 2 of 5 on Usage Guidelines and Behavior for most tools, and its
reference linter reported the same three findings on all 22:
`missing-annotations`, `undocumented-parameters` (0% coverage) and
`no-output-schema`, plus `shadow-candidate` on `contract_publish` and
`notice_publish` against `status`, whose invocation cost is zero.

Two of those are the direct consequence of ADR-0017. The rubric is
explicit that the description gets no credit for repeating what
annotations or the schema already state, and that without annotations
"the description carries the full disclosure burden". So the cheapest
way to be understood by an agent is not longer prose but the structured
fields ADR-0017 dropped. The server `instructions` text has no weight at
all.

The score is not the point; what it measures is. Its studies found tools
with usable descriptions selected 260% more often, and 89% of public
tools omitting usage constraints. A client that reads `readOnlyHint`
(Claude Code, Cursor) also stops asking the user to confirm every
`claims_list` and `status`.

## Decision

- **Every tool declares MCP annotations**, only the hints that differ from
  the protocol defaults: `readOnlyHint: true` on the ten list, get,
  search, read and status tools; `destructiveHint: false` on every
  additive tool; `idempotentHint: true` where repeating the call with the
  same input changes nothing more (`renew`, `release`, `task_update`,
  `memory_write`, `memory_delete`); `destructiveHint: true` on
  `memory_delete`, which removes a file. `openWorldHint` is left at its
  default; the daemon is local, and a uniform hint on 22 tools is bytes
  without information. 949 characters in total, measured.
- **Each of the 17 tools whose guidance was implied gains one usage
  clause**: when to use it, when not and which sibling to use instead,
  and the one fact about its result an agent needs. The five tools that
  already scored 4 on guidance (`task_create`, `task_list`,
  `contract_publish`, `notice_list`, `message_send`) keep their sentence.
  1,829 characters, measured, after two rounds of cutting to fit: the
  first draft was 10,808 and put three tools over 650, and the budget
  won. Every clause restates
  `docs/1-about/04-primitives.md`; none introduces behavior.
- **`status` says it publishes nothing and takes no lease**, which is
  what separates it from the publish tools it was flagged as shadowing.
- **The budget moves once**, in `tests/budgets.rs`: `tools/list` from
  7,800 to 10,500 characters, and the per-tool bound from 500 to 650. The
  header rule becomes: never raised without an ADR naming what the raise
  pays for. This ADR is that ADR for this raise.
- **Parameters stay undescribed on the wire.** Describing all 104 costs
  about 7,000 characters (measured), nearly doubling `tools/list` for a
  dimension worth 15%; the 44 non-obvious ones alone cost 4,350. That is
  the arithmetic ADR-0017 rejected, and it still holds. Their
  documentation remains in the input structs and in 04-primitives.md.
- **No output schemas.** Completeness is worth 10% and every result shape
  is rich; the bytes are not justified.

## Alternatives

- **Describe every parameter.** Rejected above; `tools/list` would be
  about 14,600 characters.
- **Emit `openWorldHint: false` on every tool.** Rejected: 500 characters
  for a fact stated once in the server description.
- **Put the guidance in `instructions`.** Rejected: it reaches the agent
  once per session, which is fine, but no client or evaluator attributes
  it to a tool, and the shadowing between `status` and the publish tools
  can only be resolved on the tools themselves.
- **Serve a richer schema only to evaluators.** Rejected without
  measuring: the score exists to predict what an agent does with the
  real listing.

## Consequences

- `tools/list` grows from 7,616 to about 10,400 characters, a per-session
  cost every agent pays once. The per-call results, which dominate token
  spend (ADR-0013, ADR-0017), are unchanged.
- The next tool or parameter must fit 10,500 or raise it with an ADR,
  as before.
- `docs/1-about/04-primitives.md` gains an annotations convention. The
  README's primitives table is unchanged; it never quoted descriptions.
- Glama re-scores when a tool's definition hash changes, which happens on
  the next Glama release of the server.
