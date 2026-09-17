---
id: 7930a5ea-4d87-4503-8926-938acaafe750
permalink: e2e-hubs-scenario-file-vs-symbol-claims-per-anchor-scoring-lost-write-snapshots
title: "e2e hubs scenario: file vs symbol claims, per-anchor scoring, lost-write snapshots"
kind: gotcha
tags:
- bench
- e2e
- anchors
- adr-0029
paths:
- examples/e2e
author: bench-symbols
updated_by: swarm-lead
created_at: 2026-09-17T04:31:46Z
updated_at: 2026-09-17T05:30:00Z
---

Second e2e scenario "hubs" (scenarios/hubs/, Tally pricing lib, 8 tasks, 48 hidden + 4 integration, baseline 6/48, reference 48/48) for ADR-0029's gate: symbol anchors must cut agent-blocked time >=20% vs edit windows alone, with no lost write.

- [fact] setup.sh <run> <port> --scenario hubs --claims file|symbol [--wait server|sleep]; hubs default wait=server (wait_secs 120). prompt.sh <run> <agent> renders the prompt; arms differ in exactly one line (prompt/claims_*.md) #prompt
- [gotcha] setup probes the daemon: --wait server needs claim wait_secs, --claims symbol needs a file claim to conflict with f#X. Daemons older than 1.0.4 lack both (set TIRITH_BIN) and treat f#X as unrelated to f (fail open). UNSAFE_SKIP_ANCHOR_PROBE=1 is plumbing only #anchors
- [gotcha] a `tirith call release '{}'` by an agent holding nothing exits non-zero; under set -e it silently killed setup and left the daemon up. setup now traps EXIT and stops its daemon on failure #setup
- [fact] setup starts the daemon detached (python Popen start_new_session). Forge WORKER_PROMPT.md embeds the hardened preamble: no process signals, foreground python sleep, no reading .tirith/ #hardening
- [fact] tb snapshots the code into run/snapshots/ on every ok task_update done when that dir exists (hubs only). score.py runs all hidden modules per snapshot: passed-in-a-snapshot but failing at end = regression, attributed to task anchors whose source changed since the last passing snapshot, else to all task anchors #lost-writes
- [fact] anchors.blocked_s = per-agent union of refusal episodes plus server waits (granted wait_secs claims >= 0.25 s); per_anchor duplicates file-claim numbers across anchors, per_file does not #scoring
- [lesson] fake splice workers need reference symbols that are self-contained: a symbol using a module-level import or helper outside its anchor breaks when spliced alone #dryrun
- [gotcha] fake hubs workers must use a per-process temp file name: with real anchors two workers write the same file at once and a shared temp name crashed the first symbol dry run #dryrun
- [fact] dry runs 2026-09-17 on 1.0.4 (scripted workers, plumbing only, not a benchmark): file claims 8/8 tasks, 48/48 hidden, 4/4 integration, 0 regressions, 1.5 s blocked; symbol claims 8/8, 48/48, 4/4, 0 regressions, 1.0 s blocked (24 symbol + 9 file grants); --rogue run detects 33 regressions #validation
