@AGENTS.md

Claude Code specific notes:

- Serena is configured as an MCP server for this repo (`.serena/project.yml`,
  language server: rust). Load its tools via ToolSearch before reading code.
- After milestone 1, Tirith itself is added as an HTTP MCP server for this
  repo. Setup is in `docs/2-examples/02-client-setup.md`.
