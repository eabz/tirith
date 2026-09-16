# ADR-0016: The stdio shim replaces a daemon of another version and refuses one of another repository

**Status:** Accepted, 2026-09-16. Extends ADR-0006 and supersedes its
consequence "the daemon is never stopped automatically".

## Context

ADR-0006 made `tirith stdio` start the repository's daemon on demand and
leave it running across sessions. That created two problems.

On upgrade: after `cargo install --path .` or a release install, every new
session's shim found the old daemon healthy and kept proxying to it. The
daemon kept serving the old tool list until someone read the pid out of
`.tirith/runtime/daemon.json` and killed it by hand, which the dogfooding
guide documented as a step. During the memory-primitive work the daemon on
this repository served 17 tools for an hour after the binary had 20.

Across repositories: every daemon defaults to port 7477, and the shim
identified a daemon only by whether its health endpoint answered. A stale
`daemon.json` in repository B could point at the live daemon of repository
A, and B's sessions would then place their claims, notices, and decisions
in A without any error.

## Decision

The shim checks two things about the daemon it finds before proxying, and
acts on each:

- **Repository identity.** `/api/health` now reports `root`, the canonical
  path the daemon serves. The shim canonicalizes its own root and compares.
  A daemon for another root is *foreign*: it is never signalled, and the
  shim starts a separate daemon for its own repository. If the requested
  bind fails because the port is taken, which is what the foreign daemon
  on 7477 causes, the shim starts one more daemon on an ephemeral port.
  `daemon.json` records whichever address was bound, so the CLI and later
  shims still find it without configuration.
- **Version.** `daemon.json` records the version the daemon started with,
  and `/api/health` reports the version of the process answering. Both
  must equal the shim's crate version; otherwise the daemon is *stale*.
  A stale daemon of our own repository is stopped and replaced.

The stop is graceful. On Unix the shim sends `SIGINT`, the signal
`tirith serve` already waits for, so `ServerHandle::shutdown` flushes
pending writes and removes `daemon.json`. On Windows a detached process
cannot receive ctrl-c, so it is terminated; state is written through as it
changes, so nothing beyond an in-flight write is at risk. The shim then
waits for the old process itself to be gone (a zombie counts as gone), up
to ten seconds, and only then starts the new daemon. Waiting for the
health endpoint alone was not enough: a daemon that has released its port
can live on while it drains open streams, and a replacement bound to the
same port would race it. If the old process is still alive at the
deadline, the shim fails with a message naming the pid rather than
escalating to a kill or starting a second daemon.

A daemon the shim spawns is handed to a thread that waits on it, so it
never lingers as a zombie of a shim that is still running.

Only a process that answered `/api/health` with a Tirith version is ever
signalled. A record whose endpoint does not answer that way is treated as
absent, as before, and the shim simply starts a daemon.

Every restart, refusal, and fallback is appended to
`.tirith/runtime/serve.log`, next to the daemon's own output.

## Alternatives

- **Keep the manual kill.** Rejected: it is the step everyone forgets, and
  the failure mode (agents using a stale tool list) is silent.
- **Compare only the recorded version.** Rejected: a record can be edited
  or left behind; the running process is the truth. Both are compared
  because the record is what the test suite can vary without a second
  binary.
- **Put `root` in `daemon.json` instead of `/api/health`.** Deferred. The
  record is what goes stale; the running process is what must identify
  itself. Adding it to the record as well is harmless and can follow when
  `store.rs` is next touched.
- **Have the daemon exit on its own** when it sees a newer binary on disk.
  Rejected: the daemon does not know which binary clients will spawn, and
  polling the filesystem for it adds a background loop for no gain.
- **A per-repository default port** derived from the root path. Rejected:
  it makes the documented address unpredictable and still collides.
  Ephemeral fallback keeps 7477 as the address in every normal case.
- **`SIGTERM` plus a handler in the daemon.** Deferred. `SIGINT` already
  has the graceful path; adding `SIGTERM` handling to `tirith serve` is a
  small, separate improvement (a plain `kill` today leaves `daemon.json`
  behind).
- **A `/api/shutdown` endpoint.** Rejected for now: a localhost endpoint
  anyone can POST to is a larger surface than a signal to a pid the shim
  read from the repository's own runtime directory.

## Consequences

- The first session after an install upgrades every agent on the
  repository. Sessions that were mid-call on the old daemon see one failed
  request and reconnect through their own shim, which finds the new daemon.
- A downgrade behaves the same way: the version the client spawned wins,
  because the client's binary is the one its user installed.
- Two repositories open at once on one machine get two daemons: the first
  on 7477, the second wherever the OS put it. `tirith status` in each
  repository reads the right address from its own `daemon.json`; the
  dashboard URL printed by `tirith serve` and recorded there is the one to
  open.
- Two shims of the new version starting at once both try to stop and
  replace the old daemon. The second stop finds no health endpoint and
  moves on; only the daemon that binds the port writes `daemon.json`, as
  in ADR-0006.
- Daemons older than this change report no `root`. The shim assumes such a
  daemon is its own, so at the one upgrade boundary a stale record in B can
  cause B's shim to stop A's old daemon. A's next session starts a current
  daemon for A, which reports its root from then on, so the condition heals
  itself and cannot recur.
- A daemon started from a development tree with the same version string
  as the installed binary is not detected as different. Bump the version
  or stop it by hand in that case.
