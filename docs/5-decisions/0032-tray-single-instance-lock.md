# ADR-0032: One tray per state directory, held by an OS lock; test daemons stay off the user's machine state

**Status:** Accepted, 2026-09-17. Refines the launch paragraph of
[ADR-0019](0019-menu-bar-tray.md).

## Context

eabz reported the macOS menu bar icon missing at some times and shown four
times at others. On 2026-09-17 this machine had three
`target/debug/tirith tray` processes with consecutive pids and a
`tray.pid` naming only the last one. The code explained it:

- **The single-instance check raced.** `launch_if_absent` and `tray::run`
  read `tray.pid`, ran `kill -0` on it, and only then wrote the file.
  Trays started inside that window all passed and all ran. The first
  duplicate to quit deleted the file while the others kept running, so the
  next daemon to start added one more.
- **A stale pid file hid the tray.** A tray that died without its cleanup
  (SIGKILL, or SIGINT and SIGHUP, which it does not handle) left `tray.pid`
  behind. Once any process of the user reused that pid, `kill -0`
  succeeded and no daemon launched a tray again.
- **The tray shared the daemon's process group.** A ctrl-c on a
  foreground `tirith serve`, or on `cargo test`, killed the tray with the
  daemon and left the stale pid file above.
- **Test daemons launched trays.** `tests/stdio_shim.rs` spawns the built
  binary as `serve`, and as `stdio`, which spawns `serve` itself, never
  with `--no-tray` and always with the user's `HOME`. Its tests run in
  parallel and several agents run `scripts/check.sh` at once, so test
  daemons raced into duplicate debug-build icons, and every one of them
  registered in the user's registry and flickered through the menu.

Checking the fix with four trays and two daemons started at the same
moment turned up one more race: the two daemons left one registry entry.
`register`, `unregister` and `prune` each wrote back the list they had
loaded, so the last writer erased the others' changes, and the tray's
prune, which runs up to a second after its read, erased any daemon that
registered in between.

## Decision

- **A lock, not a pid file.** A tray takes a non-blocking exclusive lock
  (`std::fs::File::try_lock`, an `flock` on macOS, stable since Rust 1.89)
  on `tray.lock` beside `daemons.json` and holds it for its whole life. A
  tray that cannot take it exits 0 at once. The kernel drops the lock when
  the process exits, however it exits, so a crash or a reused pid can no
  longer hide the tray. `tray.pid` is neither written nor read.
- **`launch_if_absent` probes the same lock**, lets go of it, and spawns
  only if it was free. Two daemons probing at the same moment may both
  spawn a tray; the second finds the lock taken and exits, so one icon
  remains.
- **The tray gets its own process group** (`CommandExt::process_group(0)`),
  so terminal signals meant for the daemon's group do not reach it. A new
  session (`setsid`) would need `pre_exec`, which is `unsafe`; the tray has
  no terminal (all three standard streams are null), so the group is
  enough.
- **Registry changes run under a lock too.** `register`, `unregister` and
  `prune` take an exclusive lock on `daemons.json.lock`, re-read the file,
  apply the change, and write it (still temp file plus rename, so readers
  need no lock). The tray prunes only daemons its poll saw dead, and keeps
  entries it has not asked yet.
- **`TIRITH_NO_TRAY`** is read by `tirith serve` as `--no-tray`, with clap's
  falsey parser: unset, empty, `0`, `false`, `no`, `n`, `f` and `off` keep
  the tray, anything else skips it. A stray value can therefore never stop
  a daemon from starting.
- **`TIRITH_STATE_DIR`** replaces the per-user state directory on every
  platform: the registry becomes `$TIRITH_STATE_DIR/daemons.json`, with the
  tray lock beside it. A relative value is taken from the working
  directory.
- **Tests never touch the user's tray or registry.** Every test that spawns
  the binary sets both variables (`tirith(root)` in `tests/stdio_shim.rs`,
  with a registry inside the test's temp directory), and `scripts/check.sh`
  exports `TIRITH_NO_TRAY=1`. No test starts a real tray; `AppKit` needs a
  GUI session.

## Alternatives

- **Keep the pid file, written with `create_new` and rechecked.** Rejected:
  a crash still leaves a file that only a liveness check can judge, and
  `kill -0` cannot tell the tray from a process that reused its pid.
- **Detect the tray by process name.** Rejected: a debug build and the
  installed binary have different paths, and a process name says nothing
  about which state directory a tray serves.
- **Pass `--no-tray` in the tests only.** Rejected as incomplete: the stdio
  shim spawns `serve` itself, so the flag never reaches that daemon, and
  test daemons would still register in the user's registry.

## Consequences

- A tray from a version before this one holds no lock, so the first new
  daemon starts a second icon next to it. After upgrading, quit old trays
  once from their menu. Daemons of older versions still read `tray.pid`,
  which nothing removes any more; the stdio shim replaces them on the next
  session (ADR-0016).
- `tray.lock` and `daemons.json.lock` are empty files that stay in the
  state directory; their existence means nothing, only the lock on them
  does.
- `TIRITH_STATE_DIR=<dir> tirith tray` runs a private tray over a private
  registry, which is how the tray's single-instance behavior can be checked
  by hand without touching the user's icon.
- A running tray with no visible icon can also be a full menu bar: macOS
  hides status items that do not fit, for example beside a notch.
