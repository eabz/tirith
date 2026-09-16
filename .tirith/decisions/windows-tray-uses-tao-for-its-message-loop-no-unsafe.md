---
id: 187cbab6-4dcb-4a77-a85e-f6f7a07d655a
permalink: windows-tray-uses-tao-for-its-message-loop-no-unsafe
title: Windows tray uses tao for its message loop; no unsafe
kind: decision
tags: []
paths:
- src/tray.rs
- Cargo.toml
- docs/5-decisions/0019-menu-bar-tray.md
author: head-dev-fable
updated_by: head-dev-fable
created_at: 2026-09-16T03:20:29Z
updated_at: 2026-09-16T03:20:29Z
---

The tray on Windows runs its Win32 message loop through the `tao` crate, enabled only under cfg(target_os = "windows") inside the `tray` feature, so the dependency never enters macOS or Linux builds. `#![forbid(unsafe_code)]` stays absolute (AGENTS.md section 4); a hand-written GetMessage/DispatchMessage loop is not an option. The Windows tray is a follow-up task, implemented and verified on a Windows machine or from a release build; v1 ships the macOS tray and the platform-neutral registry.

## Rationale

One audited unsafe function would set a precedent the rule exists to prevent; sixty extra crates on one platform, behind a feature, cost nothing to the other platforms and are the same crates Tauri ships.

## Alternatives

- Lift forbid(unsafe_code) for one function (rejected: the rule has no exceptions)
- winit instead of tao (same size, tao is what tray-icon documents)
- No Windows tray (rejected by eabz)
