---
id: 1884c3a0-3494-4cea-be10-032e76921763
permalink: tray-and-registry-what-to-know-before-editing
title: "Tray and registry: what to know before editing"
kind: gotcha
tags:
- tray
- macos
paths:
- src/tray.rs
- src/registry.rs
- deny.toml
author: claude-ci-speedup
updated_by: agent-2
created_at: 2026-09-16T03:21:13Z
updated_at: 2026-09-16T04:09:44Z
---

`src/tray.rs` is macOS-only behind the `tray` feature and pumps AppKit by hand; `src/registry.rs` is the platform-neutral daemon registry it reads. See ADR-0019.

## Observations
- [gotcha] The crate forbids unsafe, so the tray cannot use extern statics like NSDefaultRunLoopMode or any FFI loop; the run loop mode is `NSString::from_str("kCFRunLoopDefaultMode")` and the pump is `nextEventMatchingMask_untilDate_inMode_dequeue` + `sendEvent`, both safe in objc2-app-kit 0.3 #macos
- [gotcha] tray-icon's `Icon::from_rgba` wants raw pixels; the icon is a 22-line `#`/`.` bitmap const rasterized at 2x, not a PNG, to avoid a decoder crate #tray
- [gotcha] muda menu items must be removed on every refresh or separators accumulate; `Rows.items` keeps `Box<dyn IsMenuItem>` for that #tray
- [gotcha] cargo-deny walks tray-icon's Linux gtk branch even though the dep is macOS-only, because feature-activated optional target deps are not filtered by `[graph] targets`; hence the RUSTSEC-2024-0370 ignore with a reason in deny.toml. `cargo deny --target aarch64-apple-darwin check` passes without it #deny
- [fact] Registration is wired: `ServerHandle` registers the daemon on start and unregisters it on shutdown (server.rs), `tirith serve` launches the tray through `tray::launch_if_absent` unless `--no-tray`, and `tests/http_roundtrip.rs::a_daemon_registers_on_start_and_unregisters_on_shutdown` covers it. The stdio shim itself touches neither #tray

## Relations
- documented_in [[ADR-0019]]
