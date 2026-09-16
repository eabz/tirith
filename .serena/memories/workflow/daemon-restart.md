# Restarting the live daemon after an install (learned 2026-09-16)

- `cargo install --path . --locked --force` (plain install refuses to overwrite), then stop the daemon recorded in `.tirith/runtime/daemon.json`.
- SIGINT alone does not finish: the daemon's graceful shutdown waits on the long-lived SSE streams every session's stdio shim holds, so the process lives on with the port released. Until task d0ad583a lands: `kill -INT <pid>`, wait a few seconds for the flush, then `kill -9 <pid>` if it is still alive, and only then start `tirith serve` detached (`nohup tirith serve >> .tirith/runtime/serve.log 2>&1 &`).
- Never let an old daemon finish its shutdown after a new one has written daemon.json: the old one may delete the new record.
- Old shims (per-session `tirith stdio`) survive the restart and reconnect; sessions see the new tools/list only after their MCP client reconnects.
- Check `pgrep -fl "tirith serve"` afterwards: shim tests leak `target/debug/tirith serve --root /var/folders/.../.tmp*` daemons (task 0a392cec); kill them.
- Cargo.toml version did not change between 0.1.3 and this build, so the ADR-0016 shim version check cannot distinguish them; the v1 release checklist (task 9252b03e) bumps it.
