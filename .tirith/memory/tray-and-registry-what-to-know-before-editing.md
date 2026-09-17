---
id: 1884c3a0-3494-4cea-be10-032e76921763
permalink: tray-and-registry-what-to-know-before-editing
title: "Tray and registry: what to know before editing"
kind: gotcha
tags:
- tray
- macos
- registry
- tests
paths:
- src/tray.rs
- src/registry.rs
- src/server.rs
- deny.toml
- tests/stdio_shim.rs
- scripts/check.sh
author: claude-ci-speedup
updated_by: tray-daemons
created_at: 2026-09-16T03:21:13Z
updated_at: 2026-09-17T15:47:37Z
---

`src/tray.rs` is macOS-only behind the `tray` feature and pumps AppKit by hand; `src/registry.rs` is the platform-neutral daemon registry it reads. See ADR-0019 (status note has the 2026-09-17 prune refinement) and ADR-0032 (single instance, env switches, registry lock).

## Observations
- [gotcha] The crate forbids unsafe, so the tray cannot use extern statics like NSDefaultRunLoopMode or any FFI loop; the run loop mode is `NSString::from_str("kCFRunLoopDefaultMode")` and the pump is `nextEventMatchingMask_untilDate_inMode_dequeue` + `sendEvent`, both safe in objc2-app-kit 0.3 #macos
- [gotcha] tray-icon's `Icon::from_rgba` wants raw pixels; the icon is a 22-line `#`/`.` bitmap const rasterized at 2x, not a PNG, to avoid a decoder crate #tray
- [gotcha] muda menu items must be removed on every refresh or separators accumulate; `Rows.items` keeps `Box<dyn IsMenuItem>` for that #tray
- [gotcha] cargo-deny walks tray-icon's Linux gtk branch even though the dep is macOS-only, because feature-activated optional target deps are not filtered by `[graph] targets`; hence the RUSTSEC-2024-0370 ignore with a reason in deny.toml. `cargo deny --target aarch64-apple-darwin check` passes without it #deny
- [fact] Registration is wired through crate-private `registry::Registration`: `server::start` awaits `Registration::start(path, entry, REREGISTER_EVERY)` (first register before returning, on spawn_blocking), and `ServerHandle::shutdown` calls `leave()`, which drops the stop sender, awaits the loop (so an in-flight re-register cannot land after the unregister), then unregisters. `tests/http_roundtrip.rs::a_daemon_registers_on_start_and_unregisters_on_shutdown` covers it; `ServeOptions.registry: Option<PathBuf>` is unchanged. The stdio shim touches neither #tray
- [fact] Single instance (ADR-0032): `tray::run` holds `File::try_lock` (flock) on `tray.lock` beside daemons.json for its whole life; a tray that cannot lock returns Ok at once. `tray.pid` is gone #tray
- [gotcha] flock is per open file description: two `File` handles in one process conflict, which the unit test relies on. Rust opens files O_CLOEXEC, so `open`/`osascript`/`kill`/`ps` children never inherit the lock. `launch_if_absent` must drop its probe handle before spawning, or the new tray finds the lock taken #tray
- [fact] The tray is spawned with `CommandExt::process_group(0)` so a ctrl-c on a foreground `tirith serve` or `cargo test` no longer kills it; `setsid` would need unsafe `pre_exec` #tray
- [gotcha] Registry writes go through `Registry::update`: lock `daemons.json.lock`, re-read, apply, save only if the closure returns true. Prune closures must keep entries they did not check (the tray drops only pids its poll judged `Health::Gone`) #registry
- [fact] Prune rule (task 6cbbe8a1): `tray::judge` returns Live / NotResponding / Gone. Gone = root dir missing, or no answer and (`pid_alive` false or `GIVE_UP_POLLS`=60 consecutive misses). `pid_alive` runs `ps -o stat= -p <pid>` (exit non-zero or empty = gone, stat starting with Z = zombie = gone; spawn failure = alive). NotResponding rows read `<folder>  not responding` and keep Open and Stop; their previous needs_you ids are carried over so recovery does not re-notify #tray
- [fact] Re-registration: every 60 s `Registry::register_if_missing` (private) writes only when the entry is absent; an entry for the same root with a different pid and started_at >= ours wins (the newest daemon per root keeps it), an older one or a dead daemon's entry under our pid is replaced #registry
- [gotcha] The tray asks all daemons concurrently (`ask_all`: tokio::spawn per entry on the tray's current-thread runtime, one shared reqwest Client), so silent daemons cost one 800 ms timeout per poll in total; the main AppKit thread blocks for that long #tray
- [gotcha] `pid_alive` is deliberately not unit-tested against real processes (the swarm rule forbids agents running ps/kill); `alive_from_ps` parses its output and `judge` takes the check as a closure #tests
- [gotcha] Tests that spawn the built binary must go through `tirith(root)` in tests/stdio_shim.rs (sets `TIRITH_NO_TRAY=1`, `TIRITH_STATE_DIR=root/machine-state`); a bare `Command::new(env!("CARGO_BIN_EXE_tirith"))` starts real menu bar trays from target/debug and registers in the user's registry. check.sh also exports `TIRITH_NO_TRAY=1`. Registry unit tests pass tempdir paths directly #tests
- [gotcha] `std::env::set_var` is unsafe in edition 2024, so registry path rules are tested through `Registry::path_from_env(impl Fn(&'static str) -> Option<OsString>)` with a fake env #tests
- [gotcha] `TIRITH_NO_TRAY` uses clap's `FalseyValueParser`, not `BoolishValueParser`: the boolish one made `TIRITH_NO_TRAY=` (empty) a CLI error, which would stop a daemon from starting #cli
- [lesson] Manual check without touching the user's icon or signalling anything but your own process: `TIRITH_STATE_DIR=<scratch> target/release/tirith tray` x4 started together leaves exactly one (others exit 0); daemons on temp roots exit 0 by themselves within ~2 s once their root dir is removed, and unregister #verify

## Relations
- documented_in [[ADR-0019]]
- documented_in [[ADR-0032]]
