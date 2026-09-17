---
id: c4da8660-9d71-4970-bee6-9914abb107e6
permalink: tray-single-instance-by-flock-on-tray-lock-registry-changes-locked-and-re-read-tirith-no-tray-tirith-state-dir-for-tests
title: Tray single instance by flock on tray.lock; registry changes locked and re-read; TIRITH_NO_TRAY / TIRITH_STATE_DIR for tests (ADR-0032)
kind: decision
tags: []
paths:
- src/tray.rs
- src/registry.rs
- src/cli.rs
- tests/stdio_shim.rs
- scripts/check.sh
- docs/5-decisions/0032-tray-single-instance-lock.md
author: tray-fix
updated_by: tray-fix
created_at: 2026-09-17T15:09:51Z
updated_at: 2026-09-17T15:09:51Z
---

tirith tray holds File::try_lock on <state>/tray.lock for its life (no pid file); launch_if_absent probes the lock, drops it, spawns in its own process group. Registry register/unregister/prune lock daemons.json.lock and re-read before writing; the tray prunes only daemons its poll saw dead. serve reads TIRITH_NO_TRAY (clap FalseyValueParser); TIRITH_STATE_DIR replaces the per-user state dir. Tests spawning the binary set both; check.sh exports TIRITH_NO_TRAY=1.

## Rationale

Pid file check raced (duplicate icons) and went stale after hard kills or pid reuse (no icon); tests launched debug trays and registered in the user's registry; concurrent daemons lost registry entries. A kernel lock is released on any exit.

## Alternatives

- pid file with create_new and recheck (still stale after crash, kill -0 fooled by pid reuse)
- detect tray by process name
- --no-tray in tests only (shim spawns serve itself; registry still shared)
