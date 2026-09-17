---
id: 5043470a-b21d-4156-bdd9-47910b00e64a
permalink: symbol-anchored-claims-implementation-two-overlap-methods-on-repopath-2026-09-17
title: "Symbol-anchored claims implementation: two overlap methods on RepoPath (2026-09-17)"
kind: fact
tags:
- claims
- anchors
- adr-0029
- repo-path
paths:
- src/types.rs
- src/claims.rs
- src/tasks.rs
- tests/anchored_claims.rs
- docs/1-about/04-primitives.md
author: claims-anchors
updated_by: claims-anchors
created_at: 2026-09-17T04:44:41Z
updated_at: 2026-09-17T04:44:41Z
---

Task e3140352, ADR-0029 still Proposed (experimental).

- The anchor lives inside RepoPath's stored string: `path#Seg::Seg`. RepoPath::new splits at the first '#', normalizes separators (`::`, `.`, `/`) to `::`, strips generics and Go receivers. New PathError variants: EmptyAnchor, BadAnchorSegment, AnchorOnDirectory. `file()` / `anchor()` read the parts.
- Deviation from the ADR sketch, on purpose: `RepoPath::overlaps` IGNORES anchors (file-level). That keeps every brief/notice/contract/decision/memory matcher (notices.rs, decisions.rs, contracts.rs, memory.rs, state.rs notice_holders) matching on the file with zero edits. The anchor-aware claim rule is `RepoPath::claim_overlaps`; it is used by Claim::covers (claims_list path filter), ClaimBook::conflicts_for (claim + wait_secs retries), Reaped::previous_owner, and tasks.rs `blockers` (task_pull holds). Any new claim-vs-claim comparison must use claim_overlaps, not overlaps.
- `RepoPath::covers(held, target)` decides renew (target covered by an own held path) and absorb (`p != held && p.covers(held)`). `is_ancestor_of` compares file parts only. Release stays exact on the stored form (case differences are not the same stored form).
- Starvation is pinned by tests/anchored_claims.rs `a_whole_file_waiter_is_not_queued_ahead_of_new_anchor_claims`; FIFO waiting would change that test.
- Not updated: src/dashboard.html JS `overlaps()` for memory notes compares raw strings, so an anchored note path does not match its file there (dashboard only).
- Hubs dry run (fake workers, ports 7880/7881, release binary): anchor probe ok in symbol mode; both arms 48/48 hidden + 4/4 integration, no regressions; blocked_s 1.35 (file) vs 1.52 (symbol), plumbing only, not a measurement.
