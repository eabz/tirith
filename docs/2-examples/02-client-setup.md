# Client setup

Tirith serves MCP over streamable HTTP at `http://127.0.0.1:7477/mcp` by
default. Start it once per repository:

```bash
cd /path/to/repo
tirith serve
```

Then connect each client. Every agent must pass a stable `agent` name in
its tool calls; the setups below suggest where to put that instruction.

## Claude Code

```bash
claude mcp add --transport http tirith http://127.0.0.1:7477/mcp
```

Add to the project's `CLAUDE.md` or `AGENTS.md`:

```
Before editing files, call the tirith `claim` tool with agent="<your session name>".
If the result is a conflict, do not edit those files.
```

**Planned:** `claude mcp add tirith -- tirith stdio` once the stdio shim
ships, for setups that cannot keep a daemon running.

## Cursor

`.cursor/mcp.json` in the repository:

```json
{
  "mcpServers": {
    "tirith": { "url": "http://127.0.0.1:7477/mcp" }
  }
}
```

Add the same claim instruction to `.cursor/rules/` or `AGENTS.md`.

## Codex CLI

```bash
codex mcp add tirith --url http://127.0.0.1:7477/mcp
```

## Python: LangGraph

Using `langchain-mcp-adapters`:

```python
from langchain_mcp_adapters.client import MultiServerMCPClient

client = MultiServerMCPClient({
    "tirith": {"url": "http://127.0.0.1:7477/mcp", "transport": "streamable_http"},
})
tools = await client.get_tools()   # claim, release, renew, claims_list, ...
```

Give each graph node a fixed `agent` name and pass it in every call.

## Python: CrewAI

```python
from crewai_tools import MCPServerAdapter

with MCPServerAdapter({"url": "http://127.0.0.1:7477/mcp",
                       "transport": "streamable-http"}) as tools:
    agent = Agent(role="backend", tools=tools, ...)
```

## Plain script (curl)

Streamable HTTP is JSON-RPC over POST. Initialize once, then call tools.
The `Mcp-Session-Id` header returned by `initialize` must be echoed back.

```bash
curl -s http://127.0.0.1:7477/mcp \
  -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"script","version":"0"}}}' -i
```

```bash
curl -s http://127.0.0.1:7477/mcp \
  -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
  -H "Mcp-Session-Id: $SESSION" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"claim","arguments":{"agent":"script-1","paths":["src/auth/"],"reason":"batch rename"}}}'
```

For anything beyond a demo, use the `tirith` CLI instead; it handles the
session and prints structured output.

## Verifying the connection

```bash
tirith status          # daemon address, uptime, live claims
```
