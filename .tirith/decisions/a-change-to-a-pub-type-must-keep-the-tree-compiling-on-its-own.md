---
id: 1a7acfb0-400b-40d4-bf0b-14b01e3bd597
permalink: a-change-to-a-pub-type-must-keep-the-tree-compiling-on-its-own
title: A change to a pub type must keep the tree compiling on its own
kind: decision
tags: []
paths:
- src
author: head-dev-fable
updated_by: head-dev-fable
created_at: 2026-09-16T02:36:20Z
updated_at: 2026-09-16T02:36:20Z
---

When you add or change a field on a pub type that other files construct (NewMemory, NewTask, ServeOptions, tool input structs), the change must compile without touching the consumer files: give the field a default and let consumers use `..Default::default()`, or add a constructor and keep the old shape until the consumer is updated. Publish a signature notice naming the consumer paths before landing it. Never leave the tree red for another agent to fix.

## Rationale

2026-09-16: a field added to NewMemory in src/memory.rs broke src/server.rs, which was held by a different agent mid-build. Five sessions share one working tree; a red tree stops all of them. Tirith notices tell the consumer what to change, but only a compile-safe change lets them do it on their own schedule.

## Alternatives

- Coordinate the two edits in one claim window (slower, and needs the consumer file free)
- Feature-gate new fields (overkill for a struct field)
