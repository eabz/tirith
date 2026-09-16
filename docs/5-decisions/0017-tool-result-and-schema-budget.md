# ADR-0017: Tool results travel once, and tool schemas have a byte budget

**Status:** Accepted, 2026-09-16

## Context

Every MCP client feeds a tool result's text content to the model, and many
feed the structured content too. Until now `TirithServer::finish` used
rmcp's `CallToolResult::structured`, which copies the whole outcome JSON
into the text block, so every call cost twice its size: a `claims_list`
with 200 live claims was 47 KB of structured content plus 45 KB of text.

Every client also downloads `tools/list` once per session. Straight from
schemars it was 10,445 characters for 17 tools: a `$schema` URL on each
tool, `default: null` and `["T", "null"]` unions on every optional field,
`format` and `minimum` on integers, and multi-sentence descriptions on
tools and parameters. With the three memory tools that grew to 20 tools.
The structural floor, with every description removed, is 4,170 characters
because 74 parameters each cost their name plus a type object.

## Decision

- **The outcome travels once.** `finish` keeps the JSON as structured
  content and sets the text block to one line produced by `summary`: the
  status, then the one fact that identifies the outcome, such as
  `ok: claimed src/a.rs until 02:10:00Z`, `conflict: overlapping claims
  held by other agents`, `ok: 40 notices`, or `not_found: no such task`.
  It is capped at 160 characters and never starts with `{`.
- **Schemas are post-processed in `tools/list`.** A hand-written
  `ServerHandler::list_tools` (the `tool_handler` macro skips generating
  one when it exists) runs `slim_schema` over every input schema: it drops
  `$schema`, `default: null`, `format`, `minimum`, collapses nullable type
  unions, and removes per-parameter descriptions.
- **Each tool's description is one sentence** and carries the semantics a
  parameter name does not: the allowed `kind` values, and the bounds of
  `ttl_secs`, `limit`, and `depth`. Full parameter documentation stays in
  the input structs' doc comments (rustdoc) and in
  `docs/1-about/04-primitives.md`, which remains the source of truth.
- **Budgets are pinned by tests** in `tests/http_roundtrip.rs`: one text
  block under 200 bytes that starts with the status; `tools/list` at most
  6,144 characters and no tool over 500. Measured at 6,050 for 20 tools.
  The bounds may be lowered, never raised.

## Alternatives

- **Keep short descriptions on every parameter.** Rejected by arithmetic:
  74 parameters times the 17-character `"description":"",` overhead is
  1,258 characters before a single word, which alone breaks a 5,500
  target and leaves under 500 characters for all descriptions under a
  6 KB one.
- **Per-tool summary strings written in each handler.** Rejected; twenty
  call sites to keep consistent for the same information `summary` reads
  from the outcome's shape (`new_paths`, `count`, `task`, `message`).
- **Configure schemars instead of post-processing.** Rejected; schemars
  has no switch for most of what is dropped, and post-processing keeps the
  input structs plain `JsonSchema` derives.
- **Fold `contract_get` into `contract_list` and `task_pull` into
  `task_update`** to remove two schemas. Not done here: it changes the
  tool surface and needs the README, docs, and examples in the same
  change. Open for a separate decision.

## Consequences

- Every call is roughly half the bytes it was; `tools/list` is 42 percent
  smaller than before slimming despite three more tools.
- Text-only clients see a status line instead of JSON. `tirith::client`
  reads structured content first and is unaffected; the stdio shim
  forwards results unchanged.
- A new tool must fit the budget or the size test fails, which is the
  intended pressure. Non-obvious parameter semantics go in the tool
  description, not on the parameter.
- `INSTRUCTIONS` is untouched here; the `brief`-on-claim work (ADR-0014)
  rewrites the protocol text.
