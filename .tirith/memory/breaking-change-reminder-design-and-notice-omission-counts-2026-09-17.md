---
id: 921fb499-f4e2-4534-bb87-3f79865af924
permalink: breaking-change-reminder-design-and-notice-omission-counts-2026-09-17
title: Breaking-change reminder design and notice-omission counts (2026-09-17)
kind: research
tags:
- jev
- notices
- adr-0030
- e2e
- measurement
paths:
- docs/5-decisions/0030-breaking-change-reminder.md
- examples/jev_e2e
- src/lead.rs
- src/claims.rs
author: design-notice
updated_by: design-notice
created_at: 2026-09-17T04:15:42Z
updated_at: 2026-09-17T04:15:42Z
---

Task 3c72d2bf. ADR-0030 (Proposed) designs a release-time "publish a notice" reminder, gated on a phase 0 measurement. No src/ changes were made.

- [measured] In the e2e rounds r2 and r3 (4 runs, 12 workers), 28 changes needed a notice and 28/28 got one. They were 4 signature changes (ctx hooks), 4 default-behavior changes (request_id always first), 20 opt-in middleware registrations, and the from_env parsing additions (named inside the task notices). Every notice came before that task's done and final release.
- [measured] Downstream effects:
  - 0 surviving breaks: 57/57 hidden tests and 5/5 integration tests in all 4 runs.
  - 1 in-flight follow-up in 4 rounds: in r2-jev, cors.py still used request.extras. The breaking agent caught it with a direct message and it was fixed before done.
  - The request_id-first change broke test_router's order assertion in 4/4 rounds. The author fixed it in the same window, and the tests caught it, not a notice.
- [gotcha] The e2e counts are PROMPTED compliance. WORKER_PROMPT.md step 4 and the ctx-interface acceptance criteria both tell workers to publish notices, so an unprompted kit variant is needed to measure omissions.
- [measured] The e2e work loop's 45 releases:
  - 28 were final releases (release with no paths), and every one came after the agent's notice.
  - 17 were partial releases mid-task, and 15 of those came before the notice. A check that fires on a partial release would mostly remind too early, so defer it to done or the final release.
- [measured] This repo: 41 notices. There were 21 committed pub-item changes in src/ across 6 sweeping commits, and 15 have a notice naming the item. The other 6 are dead code, contained changes, or covered by a feature-level notice: 0 clear omissions. Attribution is weak.
- [design] Scoping works because claims are exclusive. Snapshot the claimed files at grant, before the claim response, and diff at the final point. Never use `git diff HEAD`.
- [design] The worker gets a fixed-template `notice_reminder` field inline in the task_update/release response, with no scores. The lead gets a lead-log row (`work_finished`, site `notice_reminder`), plus an inbox message only when the reminder was ignored.
- [design] Rule `already_noticed` (the agent published a notice covering any changed source file since its claim) settles all 28 e2e final releases without Jev.
