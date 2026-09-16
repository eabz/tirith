@AGENTS.md

Claude Code specific notes:

- Serena is configured as an MCP server for this repo (`.serena/project.yml`,
  language server: rust). Load its tools via ToolSearch before reading code.
- Tirith itself is registered in `.mcp.json` as an HTTP MCP server for this
  repo. Using it is required (see AGENTS.md section 3). If the `tirith`
  server shows as unavailable, run `cargo run --quiet -- serve` from the
  repo root in a terminal, then reconnect.
