---
id: 2d7a4cd1-20a3-4c68-8c21-8807b15eddd0
permalink: mutation-gaps-closed-2026-09-17
title: Mutation gaps closed 2026-09-17
kind: lesson
tags:
- tests
- mutation-testing
paths:
- src/decisions.rs
- src/memory.rs
- src/store.rs
- src/claims.rs
author: tests-gaps
updated_by: tests-gaps
created_at: 2026-09-17T03:57:50Z
updated_at: 2026-09-17T03:57:50Z
---

explore-verify's mutation run found 4 one-line bugs no test caught. Pinned by unit tests: decisions::tests::an_empty_query_matches_every_decision (empty/blank query matches), memory::tests::title_hits_weigh_more_than_permalink_hits (score_with exact value 6+10 for one title hit; builds Haystacks directly), store::tests::slugs_keep_48_characters (contract slug cap), claims::tests::a_lease_is_expired_exactly_at_its_expiry (is_expired uses <=). Each verified to fail under its mutant. Tip: to verify mutants while the shared tree is mid-edit by other agents, `git archive HEAD | tar -x` into a scratch dir, overlay your files, set CARGO_TARGET_DIR inside it. Beware zsh echo expanding \n in mutations.tsv regexes; use cut/printf.
