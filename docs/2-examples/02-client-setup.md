# Client setup

There are two ways to connect, and the first needs no setup step at all.

## Recommended: let the client spawn `tirith stdio`

`tirith stdio` is a stdio MCP server, like Serena or any other server a
client launches per session. When it starts it looks for the repository's
daemon and starts one if none is running, then proxies every call to it.
The daemon keeps running after the session so other agents share it.

Claude Code:

```bash
claude mcp add tirith -- tirith stdio
```

Cursor, in `.cursor/mcp.json`:

```json
{ "mcpServers": { "tirith": { "command": "tirith", "args": ["stdio"] } } }
```

Codex:

```bash
codex mcp add tirith -- tirith stdio
```

The repository root is the client's working directory; pass `--root` to
override it. The daemon's log is `.tirith/runtime/serve.log`.

## Alternative: connect to the daemon over HTTP

Tirith serves MCP over streamable HTTP at `http://127.0.0.1:7477/mcp` by
default. Start it once per repository:

```bash
cd /path/to/repo
tirith serve
```

That prints the MCP URL and the dashboard URL, and records them in
`.tirith/runtime/daemon.json` so the `tirith` CLI run from the same
repository finds the daemon without flags. Open the dashboard at
`http://127.0.0.1:7477/` to watch agents, claims, tasks, contracts,
notices, and decisions update live.

Then connect each client. Every agent must pass a stable `agent` name in
its tool calls; the setups below suggest where to put that instruction.

### Claude Code

```bash
claude mcp add --transport http tirith http://127.0.0.1:7477/mcp
```

Add to the project's `CLAUDE.md` or `AGENTS.md`:

```
Before editing files, call the tirith `claim` tool with agent="<your session name>".
If the result is a conflict, do not edit those files.
```

### Cursor

`.cursor/mcp.json` in the repository:

```json
{
  "mcpServers": {
    "tirith": { "url": "http://127.0.0.1:7477/mcp" }
  }
}
```

Add the same claim instruction to `.cursor/rules/` or `AGENTS.md`.

### Codex CLI

```bash
codex mcp add tirith --url http://127.0.0.1:7477/mcp
```

### Python: LangGraph

Using `langchain-mcp-adapters`:

```python
from langchain_mcp_adapters.client import MultiServerMCPClient

client = MultiServerMCPClient({
    "tirith": {"url": "http://127.0.0.1:7477/mcp", "transport": "streamable_http"},
})
tools = await client.get_tools()   # claim, release, renew, claims_list, ...
```

Give each graph node a fixed `agent` name and pass it in every call.

### Python: CrewAI

```python
from crewai_tools import MCPServerAdapter

with MCPServerAdapter({"url": "http://127.0.0.1:7477/mcp",
                       "transport": "streamable-http"}) as tools:
    agent = Agent(role="backend", tools=tools, ...)
```

### Plain script (curl)

Streamable HTTP is JSON-RPC over POST. Initialize once, then call tools.
The `Mcp-Session-Id` header returned by `initialize` must be echoed back,
and the `Accept` header must list both `application/json` and
`text/event-stream` or the daemon answers 406. Every reply, including a
single JSON-RPC response, comes back framed as a server-sent event: a
`data:` line carrying the JSON. Pipe through `grep`/`cut` to get the JSON
alone.

```bash
curl -s http://127.0.0.1:7477/mcp \
  -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"script","version":"0"}}}' -i
```

```bash
curl -s http://127.0.0.1:7477/mcp \
  -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
  -H "Mcp-Session-Id: $SESSION" \
  -d '{"jsonrpc":"2.0","method":"notifications/initialized"}'
```

```bash
curl -s http://127.0.0.1:7477/mcp \
  -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
  -H "Mcp-Session-Id: $SESSION" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"claim","arguments":{"agent":"script-1","paths":["src/auth/"],"reason":"batch rename"}}}' \
  | grep '^data: {' | cut -c7-
```

That is the whole handshake: `initialize` (kept with `-i` so the
`Mcp-Session-Id` response header is visible; copy it into `SESSION`), the
`notifications/initialized` notice, which gets an empty 202, and then any
number of `tools/call` requests carrying the header. The tool result is in
`result.structuredContent`; `result.content` is a one-line text summary
for clients that show only text.

Scripts and other agents should not do this by hand: the `tirith` CLI
speaks the protocol, keeps the session, and prints compact text or
`--json`, which is why it exists (decision 48538341).

### Verifying the connection

```bash
tirith status
# tirith 0.1.3 up 1s  seq 0  claims 0  tasks 0 open / 0 done  contracts 0  notices 0  decisions 0
# no active agents
```

## What responses carry

Every tool result is `{ "status": ..., ... }` plus a one-line text
summary. A few fields can appear on any result, so a client should read
them wherever they show up:

- **`lost`** on any result: leases the caller held that expired since its
  last call, with the text line prefixed `warning: lost lease on N
  path(s)`. Stop editing those paths; someone else may hold them now.
- **`inbox`** on any result: messages other agents sent the caller that
  it has not received yet, with the text line prefixed `warning: inbox:
  n;`. Answer with `message_send`.
- **The brief** on a successful `claim`: `notices`, `contracts`,
  `decisions`, and `memory` for the claimed paths, five newest each as
  compact rows, with `more` counts for the rest, and `previous_owner`
  when a path was reaped from another agent less than one lease ago, so
  the file may be half-edited. Pass `brief: false` to skip the brief. The
  demo in [01-two-agents-demo.md](01-two-agents-demo.md) shows one.
- **`load_errors`** in `status` and `/api/health`: files under `.tirith/`
  the daemon could not load at start (a merge-conflict marker in a
  committed log, an unparseable note). The daemon runs without them;
  fix the file and restart to load it.

Field shapes are in
[../1-about/04-primitives.md](../1-about/04-primitives.md#conventions-shared-by-every-tool).

## The CLI

Every tool has a subcommand. `--agent` (or `TIRITH_AGENT`) sets your name,
`--url` (or `TIRITH_URL`) overrides the daemon address, and `--json` prints
the raw tool result.

```bash
tirith status
tirith claim --agent alice --reason "refactor sessions" src/auth/
tirith claims --path src/auth/login.rs
tirith release --agent alice
tirith task create --agent planner "Build sessions handler" -p 5 --path src/api/sessions.rs
tirith task pull --agent alice
tirith task update --agent alice <task-id> done -n "merged"
tirith contract publish --agent alice "POST /api/sessions" -k http -s '{"response":{"token":"string"}}' --consumer src/client/
tirith notice publish --agent alice -k rename "renamed session_id to token" --from session_id --to token --path src/client/
tirith notice list --agent bob --unread
tirith decision record --agent alice "Tokens are opaque" -d "Clients never parse tokens" --path src/client/
tirith message send --agent alice bob "server.rs is free"
tirith memory write --agent alice "Session ids are opaque" -k gotcha --path src/auth/ --body "Compare them, never parse them."
tirith tools                       # list every tool with its description
tirith call claims_list '{}'       # call any tool with raw JSON
```

Non-`ok` outcomes (`conflict`, `not_found`, `invalid`) exit with status 1,
so the CLI composes with `&&` and `||` in scripts. `lost` and the inbox
are printed after the result whenever a call carries them.
