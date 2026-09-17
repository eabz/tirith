---
id: 7930a5ea-4d87-4503-8926-938acaafe750
permalink: jev-e2e-hubs-scenario-file-vs-symbol-claims-per-anchor-scoring-lost-write-snapshots
title: "jev_e2e hubs scenario: file vs symbol claims, per-anchor scoring, lost-write snapshots"
kind: gotcha
tags:
- bench
- e2e
- anchors
- adr-0029
paths:
- examples/jev_e2e
author: bench-symbols
updated_by: bench-symbols
created_at: 2026-09-17T04:31:46Z
updated_at: 2026-09-17T04:31:46Z
---

Second e2e scenario "hubs" (scenarios/hubs/, Tally pricing lib, 8 tasks, 48 hidden + 4 integration, baseline 6/48, reference 48/48) for ADR-0029's gate: symbol anchors must cut agent-blocked time >=20% vs edit windows alone, with no lost write.

- [fact] setup.sh <run> <port> off --scenario hubs --claims file|symbol [--wait server|sleep]; hubs default wait=server (wait_secs 120). prompt.sh <run> <agent> renders the prompt; arms differ in exactly one line (prompt/claims_*.md) #prompt
- [gotcha] setup probes the daemon: --wait server needs claim wait_secs (1.0.3 in ~/.cargo/bin lacks it; set TIRITH_BIN), --claims symbol needs a file claim to conflict with f#X. Pre-ADR-0029 daemons treat f#X as unrelated to f (fail open). UNSAFE_SKIP_ANCHOR_PROBE=1 is plumbing only #anchors
- [gotcha] a `tirith call release '{}'` by an agent holding nothing exits non-zero; under set -e it silently killed setup and left the daemon up. setup now traps EXIT and stops its daemon on failure #setup
- [fact] setup now starts the daemon detached (python Popen start_new_session) and deletes repo/.env once the daemon is up, for Forge too. Forge WORKER_PROMPT.md embeds PREAMBLE_v2, python sleep, no reading .tirith/ #hardening
- [fact] tb snapshots the code into run/snapshots/ on every ok task_update done when that dir exists (hubs only). score.py runs all hidden modules per snapshot: passed-in-a-snapshot but failing at end = regression, attributed to task anchors whose source changed since the last passing snapshot, else to all task anchors #lost-writes
- [fact] anchors.blocked_s = per-agent union of refusal episodes plus server waits (granted wait_secs claims >= 0.25 s); per_anchor duplicates file-claim numbers across anchors, per_file does not #scoring
- [lesson] fake splice workers need reference symbols that are self-contained: a symbol using a module-level import or helper outside its anchor breaks when spliced alone #dryrun
- [fact] dry runs 2026-09-17 (dev 1.0.4, ports 7870-7879): file+wait_secs 8/8, 48/48, 4/4, 0 regressions; --rogue 33 regressions detected; symbol plumbing (no anchors) 48/48. Symbol arm not measurable until an anchors build exists #validation
