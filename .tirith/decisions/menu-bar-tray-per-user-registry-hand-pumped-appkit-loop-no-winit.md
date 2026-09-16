---
id: c52185b8-956c-4ed3-b4b3-67bd8d9c408d
permalink: menu-bar-tray-per-user-registry-hand-pumped-appkit-loop-no-winit
title: "Menu bar tray: per-user registry, hand-pumped AppKit loop, no winit"
kind: decision
tags: []
paths:
- src/registry.rs
- src/tray.rs
- Cargo.toml
- deny.toml
author: claude-ci-speedup
updated_by: claude-ci-speedup
created_at: 2026-09-16T03:19:39Z
updated_at: 2026-09-16T03:19:39Z
---

Daemons announce themselves in a per-user registry (Application Support on macOS, XDG state dir elsewhere); `tirith tray` (feature tray, macOS only) reads it, prunes daemons whose /api/state does not answer, and drives AppKit's event loop by hand with nextEventMatchingMask:untilDate:inMode:dequeue: and a 5 s wake-up using only the safe objc2 surface, keeping forbid(unsafe_code). The icon is a bitmap in code rasterized at 2x, not a PNG. ADR-0019.

## Rationale

A status item needs no window, so winit/tao would add most of a GUI toolkit for nothing; tray-icon takes raw RGBA so a PNG would need a decoder crate; the registry is platform-neutral so Linux/Windows trays are additive later.

## Alternatives

- scan .tirith/runtime/daemon.json across known repos (nothing knows the repos)
- winit or tao event loop (size)
- a web page listing daemons (needs a port to find)
- PNG icon assets with include_bytes! (needs a decoder)
