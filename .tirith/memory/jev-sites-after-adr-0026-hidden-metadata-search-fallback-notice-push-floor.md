---
id: 8ac10560-a861-4ad7-be3b-d1fdaf8d9423
permalink: jev-sites-after-adr-0026-hidden-metadata-search-fallback-notice-push-floor
title: "Jev sites after ADR-0026: hidden metadata, search fallback, notice push floor"
kind: lesson
tags:
- jev
- notices
- search
- benchmark
paths:
- src/assist.rs
- src/state.rs
- src/lead.rs
- src/server.rs
- tests/jev_assist.rs
- tests/http_roundtrip.rs
- examples/jev_bench
author: core-prune
updated_by: core-prune
created_at: 2026-09-17T04:30:59Z
updated_at: 2026-09-17T04:30:59Z
---

core-prune, 2026-09-17 (tasks 126a5bfb, abaeb2b3, 1ad5e08c).

- [design] Worker tool results carry no Jev fields (skipped, advice, picked_by, ranked_by, relevance, possible_duplicate, similar_note, skipped_recipients). Duplicate flags go to the `.tirith/lead` holder's inbox from `tirith` with text `... may repeat task <short> ... (p 0.82)` / `... may repeat note <permalink> (p ..)`; examples/jev_bench parses that shape from the lead inbox (agent `bench-lead` claims `.tirith/lead` in duplicate cases). Jev is not asked for duplicates while there is no lead. #jev
- [design] Search: rank_notes/rank_decisions return None (caller uses the deterministic result) when Jev keeps nothing; hits whose title contains every query term are pinned (appended if Jev dropped them). rank_decisions takes ALL path-filtered decisions newest first and asks about substring hits then newest others, capped by DECISION_CANDIDATES=80 / DECISION_INPUT_BUDGET=48k chars. Measured: pin does not cost P@5 (0.629 vs 0.614). #search
- [design] Notice push: State::notice_holders splits holders into overlapping (exact same path, always pushed, Jev on or off), nested (ancestor/descendant: pushed unless Jev P < 0.2; always when Jev is off or fails) and others (P >= 0.5). State::push_notice sends the message AND marks the notice seen, so it is not repeated in the brief or unread list. Directory-overlap holders were labeled unaffected in bench traps (nf-003/004/011), which is why the floor is exact-only. #notices
- [gotcha] Claims cannot overlap across agents, so a test with an exact holder and a nested holder needs several affected paths (e.g. src/auth exact, src/api holder vs src/api/routes.rs, lib/cache.rs holder vs lib). #tests
- [gotcha] Any test that publishes a notice about a path another agent holds and then expects it unread now fails: the push delivered it. tests/http_roundtrip.rs unread-scoping test publishes before the claims with brief:false for that reason. #tests
- [measured] jev_bench brief cases can differ between arms with 0 Jev calls: rows tied on timestamp come back in a different order per daemon. Not a Jev effect. #benchmark
