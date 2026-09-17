# ADR-0019: A per-user daemon registry and a macOS menu bar tray

**Status:** Accepted, 2026-09-16. Refined the same day: the tray is
launched by `tirith serve` (`tray::launch_if_absent`, skipped with
`--no-tray`), not by the stdio shim, and since 04:15Z a click on the icon
always shows the daemon list, with a row opening that daemon's dashboard
(decision by eabz, code by agent-3). The launch and left-click paragraphs
below describe the first version. Refined by
[ADR-0032](0032-tray-single-instance-lock.md): a lock on `tray.lock`
replaces the pid file, registry changes re-read the file under a lock,
and `TIRITH_NO_TRAY` and `TIRITH_STATE_DIR` keep test daemons off the
user's tray and registry. Refined 2026-09-17 (task 6cbbe8a1), replacing
the pruning paragraph below: one missed 800 ms answer used to prune a busy
daemon, and nothing put it back until it restarted. A daemon that does not
answer now stays listed as `<folder>  not responding` with its `Stop` row;
the tray prunes it only when its root is gone, its pid is gone (`ps -o
stat= -p <pid>` finds no process, or a zombie; nothing is signalled), or
it stayed silent for 60 polls (five minutes, for a pid reused by another
process). Every daemon checks its entry once a minute under the registry
lock and puts it back if missing, so a wrong prune heals; the newest
daemon for a root keeps the root. The tray asks all daemons at once, so
silent daemons cost one timeout per poll between them.

## Context

Every repository gets its own daemon, started on demand by the stdio shim
(ADR-0006). A developer with several repositories open has several
daemons and no way to see them short of `ps`, and reaching a dashboard
means knowing which port a daemon took. Serena's launcher solves this
with a menu bar icon; eabz asked for the same for Tirith.

Two things are missing: a place where daemons announce themselves that is
not tied to any one repository, and a small native UI that reads it.

## Decision

- **A per-user registry**, `src/registry.rs`, on every platform:
  `~/Library/Application Support/tirith/daemons.json` on macOS and
  `$XDG_STATE_HOME/tirith/daemons.json` (default `~/.local/state`)
  elsewhere. One JSON array of `{root, url, dashboard_url, pid, version,
  started_at}`. A daemon registers on start and unregisters on clean
  shutdown, written atomically (temp file plus rename). Registering a root
  or pid that is already present replaces the old entry, since only a dead
  daemon can leave one behind.
- **The tray prunes.** Every poll it drops entries whose `/api/health`
  does not answer or whose root no longer exists, so a crashed daemon
  disappears within five seconds without anybody cleaning up. (First
  version; see the status note for the pid check and re-registration.)
- **`tirith tray`**, `src/tray.rs`, macOS only, behind the cargo feature
  `tray` (on by default; the module and its dependencies are gated on
  `target_os = "macos"`, so Linux and Windows builds contain nothing of
  it). It uses Tauri's `tray-icon` and `muda` crates over `objc2`, and
  drives AppKit's event loop by hand on the main thread with
  `nextEventMatchingMask:untilDate:inMode:dequeue:` and a five-second
  wake-up, which is enough for a menu and avoids pulling in `winit` or
  `tao`. The crate keeps `#![forbid(unsafe_code)]`: everything used is the
  safe surface of `objc2-app-kit` and `objc2-foundation`.
- **The menu** has one row per daemon, `<folder>  <n> agents, <m>
  claims`, read from `/api/state` every five seconds; clicking a row opens
  its dashboard with `open`; a `Stop <folder>` row sends SIGINT to the pid
  (which the daemon handles gracefully per the shutdown order in
  `docs/1-about/02-architecture.md`); `Quit tray` is the last row. A
  left-click on the icon with exactly one daemon opens that dashboard
  directly.
- **The icon** is a monochrome template image, a tower with battlements
  and a flag at 22 points, marked as a template so it follows light and
  dark menu bars. It is a 22-line bitmap in `src/tray.rs` rasterized at
  2x at startup rather than a PNG: `tray-icon` takes raw RGBA, so a PNG
  would have cost a decoder crate for 155 bytes of pixels.
- **Launch.** The stdio shim starts `tirith tray` detached the first time
  it starts a daemon, unless one is already running (a pid file in the
  same state directory); `tirith serve --no-tray` opts out; the tray exits
  only on Quit. This half lands after the registry writes in
  `server.rs` and `store.rs`, in the same claim window, because both sides
  of the registry must agree.

## Alternatives

- **Scan `.tirith/runtime/daemon.json` across known repositories.**
  Rejected; nothing knows which repositories exist, and a registry costs
  one file.
- **A web page listing daemons instead of a native icon.** Rejected; the
  point is to find the pages, and a page needs a port to find.
- **`winit` or `tao` for the event loop.** Rejected for size; a status
  item needs no window, and AppKit's event pump is four safe calls.
- **Cross-platform from day one.** Rejected; the tray is macOS only. The
  registry is platform-neutral (it also resolves `%LOCALAPPDATA%` and the
  XDG state directory) because every daemon registers itself wherever it
  runs, but the tray itself is not built for Windows or Linux. Windows
  would need a Win32 message pump, which only a windowing toolkit (`tao`,
  `winit`) or FFI provides, and this crate forbids `unsafe`; with no
  Windows machine to verify a build, that work is out of scope (decision
  9f574339, 2026-09-16). Linux additionally needs GTK or libappindicator
  development packages on the build host.

## Consequences

- New optional dependencies, macOS only: `tray-icon`, `muda`,
  `objc2-app-kit`, `objc2-foundation`, each with a justification line in
  `Cargo.toml`. The task note carries the measured dependency-tree and
  binary-size deltas.
- `cargo test --all-features` on Linux compiles none of the tray; on macOS
  the tray has no GUI tests, only the registry's unit tests.
- A crashed daemon still leaves the menu within one poll, since its pid
  is gone; a daemon that is only slow shows as not responding instead of
  vanishing, and one pruned by mistake returns within a minute.
- A second Tirith binary on the machine (a dev build from a checkout)
  shares the registry with the installed one, which is intended: the tray
  shows every daemon, whatever started it.
