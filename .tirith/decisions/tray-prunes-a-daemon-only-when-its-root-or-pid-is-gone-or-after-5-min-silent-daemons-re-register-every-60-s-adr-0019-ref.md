---
id: 412cf5ea-b8e0-465c-b6c8-62478ebb3450
permalink: tray-prunes-a-daemon-only-when-its-root-or-pid-is-gone-or-after-5-min-silent-daemons-re-register-every-60-s-adr-0019-ref
title: Tray prunes a daemon only when its root or pid is gone (or after 5 min silent); daemons re-register every 60 s (ADR-0019 refinement)
kind: decision
tags: []
paths:
- src/tray.rs
- src/registry.rs
- src/server.rs
- docs/5-decisions/0019-menu-bar-tray.md
author: tray-daemons
updated_by: tray-daemons
created_at: 2026-09-17T15:43:33Z
updated_at: 2026-09-17T15:43:33Z
---

An unanswered /api/state poll marks a daemon "not responding", not dead. The tray prunes it when its root dir is gone, when `ps -o stat= -p <pid>` finds no process or a zombie, or after GIVE_UP_POLLS=60 consecutive silent polls (pid reuse). If ps cannot run, the pid counts as alive. Every daemon runs registry::Registration: registers on start, every REREGISTER_EVERY=60 s puts its entry back if missing (register_if_missing, under daemons.json.lock, writes only on change, newest started_at per root wins), and on shutdown stops the loop, awaits any in-flight check, then unregisters.

## Rationale

One 800 ms timeout erased busy daemons from the menu until restart. Liveness by pid avoids that without signalling; the 5-minute cap plus periodic re-registration covers pid reuse and zombies while any wrong prune heals within a minute. ps rather than kill -0 because it also reports zombies and has no signal semantics; no new crate (rustix would be one).

## Alternatives

- N=3 consecutive misses plus a dead pid (a dead pid alone is conclusive, so waiting only delays removing a crashed daemon)
- kill -0 via Command (misses zombies)
- rustix::process::test_kill_process (new normal dependency)
- never prune, rely on re-registration only (crashed daemons would linger)
