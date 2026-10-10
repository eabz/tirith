---
id: 58eb63ae-a9a4-4378-a16f-0290b86c6c5a
permalink: lead-only-writes-adr-0036-where-the-gate-lives-and-the-traps
title: "Lead-only writes (ADR-0036): where the gate lives and the traps"
kind: gotcha
tags:
- lead
- budget
paths:
- src/lead.rs
- src/state.rs
- src/server.rs
- src/types.rs
- src/output_schemas.rs
- src/guide.rs
- tests/lead_gates.rs
author: claude-cleanup
updated_by: claude-cleanup
created_at: 2026-10-10T20:06:25Z
updated_at: 2026-10-10T20:11:26Z
---

Task of 2026-10-10 (eabz cleanup request). What the files do not say by themselves:

- [fact] The rule is `Lead::require(claims, agent, action)` in src/lead.rs: Err only when someone else holds exactly `.tirith/lead`. `Inner::lead_only` in src/state.rs wraps it; each gated State method (task_create, task_update with force, contract_publish, decision_record, memory_delete) calls it under the lock. The error is the one `types::LeadOnly` struct, carried as a transparent `LeadOnly(#[from] LeadOnly)` variant of each domain error so domain modules import types.rs only, never lead.rs. server.rs has one `impl From<LeadOnly> for Value` (conflict with `lead`) #lead
- [trap] The force gate fires only when `force` would take another agent's task: `State::task_update` strips `force` for a gated caller and maps the resulting `OwnedByOther` to the refusal, so a worker forcing its own task is never refused. tests/lead_gates.rs `swarm()` therefore puts the task in progress under a third agent `other`
- [trap] `tools/list` is at about 44,424 of the 44,500 budget (tests/budgets.rs) after the five "Lead-only while a swarm has a lead." clauses and the `lead` output-schema field. The next tool-surface addition has about 70 bytes: trim text or raise the budget by an owner decision (ADR-0034) #budget
- [trap] rust 1.99 clippy adds `assert_is_empty`: a bare `assert!(x.is_empty())` fails `-D warnings`. A message argument silences it (`assert!(x.is_empty(), "x should be empty")`); 51 sites were converted that way on 2026-10-10. Write new emptiness asserts with a message
- [fact] `guide` topic `roles` (src/guide.rs) is the spawn/model/brief rule set; the overview stays under 3,072 bytes (measured 2,860) because sections only add their tool names to it. The five-tool list lives once, in the overview RULES; the `lead` topic and the server INSTRUCTIONS point at it
- [fact] tests/common/lead.rs holds `claim_lead`, pulled in with `#[path]` by lead_escalation.rs and lead_gates.rs only (common/mod.rs helpers must be used by every test file). Add a new gated tool in the `gated()` table of lead_gates.rs, in the primitives doc "Lead-only writes", in the tool description and in the output schema status text
