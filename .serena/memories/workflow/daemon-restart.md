# Restarting the live daemon by hand (updated 2026-09-16)

Normally unnecessary: after an install, the next session's `tirith stdio`
shim stops a daemon whose version differs and starts the new binary
(ADR-0016). The manual path is for the one case the shim cannot detect: a
daemon built from a working tree whose `Cargo.toml` version equals the
installed binary's (0.1.3 until the release checklist, task 9252b03e,
bumps it).

- `cargo install --path . --locked --force` (plain install refuses to
  overwrite an equal version).
- Read the pid from `.tirith/runtime/daemon.json` and `kill -INT <pid>`.
  Shutdown flushes pending writes, gives open SSE streams at most 2 s
  (`SHUTDOWN_DEADLINE` in server.rs), removes `daemon.json` and the
  registry entry, and exits; no `kill -9` is needed. Wait until the pid is
  gone before starting anything.
- Never let an old daemon finish its shutdown after a new one has written
  `daemon.json`: the old one may delete the new record.
- Start the new daemon with the next MCP session (the shim does it) or by
  hand: `nohup tirith serve >> .tirith/runtime/serve.log 2>&1 &`.
- Old shims (per-session `tirith stdio`) survive the restart and
  reconnect; sessions see the new tools/list only after their MCP client
  reconnects.
- `scripts/check.sh` fails its last step if a test left a
  `target/debug/tirith serve --root /var/folders/...` daemon behind;
  `pgrep -fl "tirith serve"` finds them.
