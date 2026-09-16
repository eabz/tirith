# The stdio shim (`tirith stdio`)

- `src/stdio.rs`: `ensure_daemon(root, bind)` reads `.tirith/runtime/daemon.json`, GETs `{dashboard_url}api/health` (1s timeout), and if unhealthy spawns `current_exe() serve --root <root> --bind <bind>` detached (unix `process_group(0)`, windows DETACHED_PROCESS) with output appended to `.tirith/runtime/serve.log`, then polls up to 15s. `run()` then connects an rmcp HTTP client to the daemon and serves a `Proxy: ServerHandler` over `rmcp::transport::stdio()` forwarding `tools/list` and `tools/call`.
- Only a daemon that actually bound the port writes `daemon.json`, so concurrent shims converge on one daemon. The daemon is never auto-stopped.
- `.mcp.json` / `.cursor/mcp.json` register `tirith` as `command: tirith, args: [stdio]` (Serena-style). Requires the installed `tirith` binary to have the `stdio` subcommand (0.1.3+); older installs must be updated with `cargo install --path .` or the next release.
- Test: `tests/stdio_shim.rs` spawns `CARGO_BIN_EXE_tirith stdio --bind 127.0.0.1:0` in a temp dir, speaks raw JSON-RPC over pipes, checks a second shim reuses the daemon, kills the daemon by pid at the end.
- ADR-0006 documents the decision.
