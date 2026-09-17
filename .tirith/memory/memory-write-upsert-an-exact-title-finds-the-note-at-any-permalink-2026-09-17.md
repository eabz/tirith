---
id: c1de37ad-a05e-4345-9834-bfcf0383fd15
permalink: memory-write-upsert-an-exact-title-finds-the-note-at-any-permalink-2026-09-17
title: "memory_write upsert: an exact title finds the note at any permalink (2026-09-17)"
kind: gotcha
tags:
- memory
- permalink
- upsert
paths:
- src/memory.rs
- docs/1-about/04-primitives.md
author: tray-daemons
updated_by: tray-daemons
created_at: 2026-09-17T15:47:43Z
updated_at: 2026-09-17T15:47:43Z
---

Task 29430f99, contract "Memory primitive" v7.

## Observations
- [gotcha] Before the fix, `MemoryBook::write` without `permalink` looked only at `Permalink::for_title(title)`, so a note with the same title under any other permalink got a new duplicate on every write: folder-filed notes (`design/v1-audit-2026-09-16`), id-suffixed notes from a slug collision ("Storage design" vs "Storage  design!"), and `note-<id>` notes whose title has no ASCII slug (the fresh id made even the second write of the same non-Latin title a new note) #memory
- [fact] Rule now: slug hit with equal title -> that note (hash lookup); else `MemoryBook::titled` scans for notes whose trimmed title equals the written one and picks the most recently updated (ties: later position); else a new note at the slug, or `slug-<id>` when another title holds the slug. Title matching is exact and case-sensitive on purpose, so titles that differ only in case or punctuation stay separate notes #memory
- [fact] Pinned by `memory::tests::writing_an_existing_title_updates_a_note_with_a_custom_permalink` and `a_title_shared_by_several_notes_updates_the_most_recent_one`; state.rs and server.rs needed no change #tests
- [lesson] Notes duplicated by the old bug still exist as pairs with one title; a write by that title now updates the one at the slug. Delete the stale copy with memory_delete by permalink if you find one #memory
