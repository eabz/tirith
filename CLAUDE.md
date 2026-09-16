@AGENTS.md

Claude Code specific notes:

- Serena is configured as an MCP server for this repo (`.serena/project.yml`,
  language server: rust). Load its tools via ToolSearch before reading code.
- Tirith itself is registered in `.mcp.json` as `tirith stdio`, which
  starts the repo's daemon on demand. Using it is required (see AGENTS.md
  section 3). If the `tirith` server shows as unavailable, the binary is
  missing from PATH: `cargo install --path .`, then reconnect.
