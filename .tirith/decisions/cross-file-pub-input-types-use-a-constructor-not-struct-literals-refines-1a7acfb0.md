---
id: 19c6595c-2578-4949-aee7-256b0c1c5a36
permalink: cross-file-pub-input-types-use-a-constructor-not-struct-literals-refines-1a7acfb0
title: Cross-file pub input types use a constructor, not struct literals (refines 1a7acfb0)
kind: decision
tags: []
paths:
- src/memory.rs
- src/tasks.rs
- src/notices.rs
- src/contracts.rs
- src/decisions.rs
- src/server.rs
author: head-dev-fable
updated_by: head-dev-fable
created_at: 2026-09-16T02:40:37Z
updated_at: 2026-09-16T02:40:37Z
---

Refinement of decision 1a7acfb0 after clippy pedantic rejected the `..Default::default()` spread (needless_update) before the new field existed: for pub types that one module owns and another module constructs (NewMemory, NewTask, NewNotice, NewContract, NewDecision), the owner provides a constructor for the required fields plus `with_*` setters or a builder, and call sites in other files use that instead of a struct literal. Then a new optional field is additive with no change at the call site and no lint noise. Until a type has a constructor, the field and the call-site change land in one claim window by one agent who holds both files.

## Rationale

Two agents lost the tree twice in twenty minutes over one field on NewMemory because the struct and its only call site live in different files held by different agents. A constructor moves the coupling into the owning module.

## Alternatives

- #[allow(clippy::needless_update)] on each call site with a justification comment (allowed by AGENTS.md but noisy)
- One agent claims both files for every field change (works, but serializes on server.rs, the hottest file)
