---
id: 5351abf0-0dce-4f04-8cd3-59c61dc4f9a8
permalink: tool-definitions-on-the-wire-where-things-live-and-the-traps-adr-0034
title: "Tool definitions on the wire: where things live and the traps (ADR-0034)"
kind: gotcha
tags:
- tools-list
- tdqs
paths:
- src/server.rs
- src/output_schemas.rs
- src/guide.rs
- tests/budgets.rs
author: claude-tdqs-guide
updated_by: claude-tdqs-guide
created_at: 2026-09-18T22:30:49Z
updated_at: 2026-09-18T22:30:49Z
---

What a tool looks like in tools/list is assembled from four places. Touch one, check the others.

- [fact] Description and annotations: the `#[tool(...)]` attribute in src/server.rs. Order of every description: what it does vs its siblings, what it changes that annotations cannot say, when / when not / which tool instead, what comes back and how it pages. Bounds live on the parameter, not here: the TDQS rubric gives no credit for repeating the schema.
- [fact] Parameter descriptions: the `///` doc comments on the input structs ARE wire text since ADR-0034 (slim_schema keeps them). Closed string params get `#[schemars(extend("enum" = [...]))]`; the Rust type stays String so a bad value is status `invalid`, never an MCP error.
- [fact] Output schemas: src/output_schemas.rs, attached in `list_tools`. Only `status` required, additional properties allowed, a property typed only when certain, nullable as `[T, "null"]` or untyped.
- [fact] The guide: src/guide.rs is data. `the_guide_and_the_router_name_the_same_tools` fails if a routed tool is missing from it or it names one that does not exist.
- [gotcha] `schema()` adds a shared string `message` with `or_insert_with`. `message_send` returns `message` as an OBJECT on ok, so it declares its own `["object","string"]`. The first version overwrote it and a strict client would have rejected every successful send. `every_result_conforms_to_the_output_schema_its_tool_declares` in tests/budgets.rs caught it; add a call to `sample_calls` for every new tool or outcome.
- [gotcha] `notice_list` with unread=true durably marks notices seen, so it is NOT readOnlyHint. A description that contradicts its annotations scores 1 on the rubric.
- [gotcha] `renew` holding nothing is `not_found`, not ok with count 0. Probe the live daemon before asserting an outcome in a schema.
- [fact] Measure with the reference linter, no key needed: `pip install tdqs`, dump tools/list from `target/debug/tirith --root <tmp> stdio` with TIRITH_STATE_DIR set to a temp dir, then `tdqs lint --file dump.json`. 2026-09-18: 0 errors, 2 warnings (shadow-candidate, structural: invocation cost 4 vs guide's 0), 0 notes.
- [fact] Budgets in tests/budgets.rs: tools/list 44,500 (measured 43,714), 3,600 per tool (claim 3,451), guide overview 3,072 (2,574). A raise needs an ADR that names what it pays for.
- follows [[design/v1-audit-2026-09-16]]
