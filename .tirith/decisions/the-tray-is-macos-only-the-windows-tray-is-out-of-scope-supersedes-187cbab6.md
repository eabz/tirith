---
id: 9f574339-db7e-4139-b9d1-ff42f04807e7
permalink: the-tray-is-macos-only-the-windows-tray-is-out-of-scope-supersedes-187cbab6
title: The tray is macOS only; the Windows tray is out of scope (supersedes 187cbab6)
kind: decision
tags: []
paths:
- src/tray.rs
- src/registry.rs
- docs/5-decisions/0019-menu-bar-tray.md
author: head-dev-fable
updated_by: head-dev-fable
created_at: 2026-09-16T03:33:08Z
updated_at: 2026-09-16T03:33:08Z
---

Tirith ships the menu bar tray on macOS only. The Windows tray (decision 187cbab6, tao message loop) is cancelled: no Windows machine is available to build or verify it, and an unverified GUI feature does not ship. The registry keeps resolving a Windows path because `tirith serve` registers daemons wherever it runs; nothing else Windows-specific exists for the tray. Reopen only if a Windows machine becomes available.

## Rationale

eabz, 2026-09-16: we will not have a Windows machine soon. Shipping platform code nobody can run is a liability, not a feature.

## Alternatives

- Keep the Windows follow-up open on the board (rejected: it would sit forever)
- Build blind and let users verify (rejected: a broken tray on first launch is worse than none)
