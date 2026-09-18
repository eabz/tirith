# ADR-0034: Tool definitions are complete on the wire, and `guide` explains the protocol

**Status:** Accepted, 2026-09-18, by the owner's decision. Supersedes the
per-parameter rule of
[ADR-0017](0017-tool-result-and-schema-budget.md) and, in part,
[ADR-0033](0033-tool-annotations-and-usage-clauses.md) ("Parameters stay
undescribed on the wire", "No output schemas"). The annotations and the
usage clauses of ADR-0033 stand. Updates the `tools/list` row of the
table in [ADR-0013](0013-v1-definition.md).

## Context

ADR-0033 gave every tool annotations and a usage clause and stopped
there on purpose: the reference TDQS linter still reported
`undocumented-parameters` on all 22 tools (0% coverage) and
`no-output-schema` on all 22, because describing them was priced at more
bytes than ADR-0017 allowed.

The owner then ranked the two goals: a tool definition that meets every
requirement of the rubric outranks the per-session size of `tools/list`.
That is a product decision, not a measurement, and it is recorded here
so no later session re-argues it from the byte counts alone.

What the rubric asks for, read from its source
(glama-ai/tool-definition-quality-score):

- **Parameter Semantics** starts at 3 only above 80% schema description
  coverage; below 50% the description "must compensate".
- **Contextual Completeness**: "a return shape with described fields
  relieves the description of explaining return values; a bare
  `{"type": "object"}` relieves it of nothing".
- Its own HIGH calibration example has annotations and 100% coverage and
  still scores 3 on transparency and on parameters, because its
  description says nothing about the return format, paging, or how
  parameters interact. A 4 or 5 needs the description to add exactly
  that, without repeating the schema.
- A server's tool score is `0.6 × mean + 0.4 × min`, so one weak tool
  pulls every other down.

Separately, the protocol lived only in the server `instructions`, which
some clients truncate or drop, which scrolls out of a long session, and
which an agent joining a running swarm cannot ask for again.

## Decision

1. **Every parameter is described on the wire.** `slim_schema` no longer
   removes descriptions: 106 of 106. The input structs' doc comments are
   wire text now, so the thin ones (`/// Why.`) were rewritten.
2. **Closed string parameters carry an `enum`**: task status, contract,
   notice and memory kinds, guide topics, through
   `#[schemars(extend("enum" = [...]))]`. The Rust type stays `String`,
   so a bad value is still a result with status `invalid`, never an MCP
   error. Aliases the parsers accept (`renamed`, `in-progress`) are not
   advertised.
3. **Every tool declares an output schema** (`src/output_schemas.rs`),
   attached in `list_tools`. The schemas document and never constrain:
   only `status` is required, additional properties are allowed (so
   `lost`, `inbox`, `inbox_more` and `persist_error` never fail a
   validating client), and a property is typed only when its JSON type
   is certain; a field that can be null is typed `[T, "null"]` or left
   untyped. `tests/budgets.rs` validates one real result per tool and
   per notable outcome against the schema its tool declares. On its
   first run it caught that `message_send` returns `message` as an
   object where the shared `message` string had overwritten it: a strict
   client would have rejected every successful send.
4. **Every description is rewritten to the same order**: what the tool
   does, told apart from its siblings; what it changes that annotations
   cannot say (what is marked seen, written to disk, sent to whom, what
   is atomic); when to call it, when not, and which tool to call
   instead; what comes back and how it pages. Parameter bounds left the
   descriptions for the parameters they belong to, because the rubric
   gives no credit for repeating the schema.
5. **`notice_list` is not read-only.** With `unread` it durably marks
   what it lists as seen. ADR-0033 annotated it `readOnlyHint: true`;
   that was wrong and is corrected before the description discloses the
   side effect, since the rubric scores a description that contradicts
   its annotations 1.
6. **`guide` is the 23rd tool.** No required parameter, an optional
   `topic`, read-only. It returns the purpose, the working loop from
   `claim` to `release`, the rules every reply follows, and which tool a
   situation calls for; a topic narrows it to one primitive. The content
   is data in `src/guide.rs`. A test pins the tools it names to the tool
   router in both directions, so a new tool cannot ship without an
   entry. `instructions` stays as the short form and points at it. The
   CLI gains `tirith guide [TOPIC]`. The overview is budgeted at 3,072
   bytes (measured 2,574).
7. **The budget moves once more**, in `tests/budgets.rs`: `tools/list`
   from 10,500 to 44,500 characters (measured 43,714 for 23 tools) and
   the per-tool bound from 650 to 3,600 (largest: `claim`, 3,451). By
   part: descriptions 8,552 (19%), input schemas 11,820 (27%), output
   schemas 20,726 (47%), annotations 644.

## Alternatives

- **Stay at ADR-0033.** Rejected by the owner.
- **Generate output schemas from typed results** (`rmcp`'s `Json<T>`).
  Not now: results are built as `Value`, their shape depends on the
  status, and any of them may carry the piggybacks; typing them rewrites
  every handler. Hand-written schemas plus the conformance test get the
  same guarantee today. Open for a later ADR.
- **Strict schemas** (`additionalProperties: false`, a `oneOf` per
  status). Rejected: a validating client would reject a result that
  carries `lost` or `inbox`, and the documentation value is the same.
- **Rust enums for `kind` and `status` inputs.** Rejected: a serde
  failure is an MCP error, which breaks the convention that outcomes are
  status JSON.
- **The full `lost`/`inbox` note on every output schema.** Cut to one
  sentence that points at `guide`; it was 4.5 KB said 23 times.
- **`guide` as an MCP prompt or resource.** Rejected for now: client
  support for both is uneven and tools are universal. The same data can
  back a prompt later.
- **Fewer tools.** The rubric's coherence score rates 3 to 15 tools as
  well scoped and 16 to 25 as heavy. Reaching 15 means removing eight
  tools from a 1.x API. Rejected; see the consequences.

## Consequences

- `tools/list` is about 43.7 KB, 4.2 times ADR-0033 and roughly 11,000
  to 15,000 tokens, paid once per session by every agent. Per-call
  results, which dominate spend over a session (ADR-0013), are
  unchanged.
- Reference linter: 24 warnings and 22 notes down to 2 warnings and no
  notes. The two left are `shadow-candidate` on `contract_publish` and
  `notice_publish`, now against `guide`: they are computed from
  invocation cost (four required fields against none) and only the
  rubric's coherence evaluation can dismiss them.
- A known ceiling: with 23 tools the coherence dimension "Tool Count
  Appropriateness" is expected to score 3, whatever the definitions say.
- A new tool, parameter or result field must update
  `src/output_schemas.rs` and `src/guide.rs`, or a test fails. That is
  the intended pressure, as the byte budget was.
- Glama re-scores a tool when the hash of its definition changes, which
  happens on the server's next Glama release.
