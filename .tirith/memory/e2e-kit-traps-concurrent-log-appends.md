---
id: 0dceb171-a495-4439-bf50-82ff8f3061f7
permalink: e2e-kit-traps-concurrent-log-appends
title: "e2e kit traps: concurrent log appends"
kind: gotcha
tags:
- bench
- e2e
paths:
- examples/e2e
author: bench-e2e
updated_by: swarm-lead
created_at: 2026-09-17T02:30:26Z
updated_at: 2026-09-17T05:30:00Z
---

End-to-end benchmark kit (setup.sh / tb / score.sh / stop.sh, scenario "Forge" in Python 3.9 stdlib).

- [gotcha] tb responses exceed PIPE_BUF (task_pull, claim briefs ~3.5 KB); concurrent `>>` appends interleaved JSONL lines. tb appends under a perl flock #logging
- [gotcha] benchmark workers must never signal processes: a worker's `pkill -f cat` killed the run daemons and unrelated desktop helper processes and invalidated a round. Worker prompts forbid kill/pkill/killall/lsof/ps and background jobs #hardening
- [fact] reference/ solution passes 57/57 hidden + 5/5 integration; the baseline passes 2/57 #bench
