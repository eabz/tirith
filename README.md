<p align="center">
  <img src="docs/_static/images/logo-512.jpeg" alt="Tirith logo: a white castle on a mountain, circuits at its base" width="320">
</p>

# Tirith

**A coordination server for parallel coding agents.**

Tirith is a small MCP server that keeps five or ten coding agents from
stepping on each other when they work in the same repository at the same
time. Agents claim files before editing, pull tasks from a shared board,
publish the shape of an interface before either side implements it, announce
renames so dependents can react, and record decisions so nothing gets decided
twice.

It is not a memory layer. It stores coordination state, not knowledge.

Tirith is framework-agnostic: anything that speaks MCP over HTTP can use it.
That includes Claude Code, Cursor, Codex, LangGraph, CrewAI, and a plain
script with `curl`.

> Status: pre-alpha (0.1.0). All five primitives, the CLI, JSON persistence,
> and the dashboard exist and are tested. Tool schemas may still change before
> 1.0. See [docs/1-about/01-purpose.md](docs/1-about/01-purpose.md) for the
> roadmap.

## The problem

Run several coding agents on one codebase and three things go wrong:

1. Two agents edit the same file and one silently overwrites the other.
2. One agent renames a function that another agent's in-progress work
   depends on.
3. Two agents build the two sides of an interface to different shapes,
   because nobody wrote the shape down first.

Claims and a task board fix the first problem. Contracts and change notices
fix the second and third, and those are the heart of Tirith, because there is
no good workaround for them today.

## Primitives

| Tool family | What it does |
|---|---|
| **Claims** | An agent claims files or directories before editing. Claims are leases: they expire if the agent dies. Overlapping claims are refused with the owner's name, paths, reason, and expiry. |
| **Task board** | Tasks with status, owner, and dependencies. Agents pull the next unblocked task instead of being assigned one. |
| **Contracts** | Interface shapes (function signatures, request and response types, schemas) published and agreed before implementation begins. |
| **Change notices** | "Renamed `X` to `Y`, callers in these files are affected." Dependents read notices for their area before acting. |
| **Decisions log** | Settled choices with rationale, so no agent re-decides them. |

The full tool list and schemas are in
[docs/1-about/04-primitives.md](docs/1-about/04-primitives.md).

## How it runs

One `tirith` daemon runs per repository and every agent connects to it over
streamable HTTP on localhost. That is the whole trick: MCP clients normally
spawn a fresh server per session, which would give ten agents ten separate
states and nothing to coordinate. Tirith is deliberately a single shared
process.

State is tiny and lives in memory, written through to JSON under `.tirith/`
in the target repository. Contracts, notices, and decisions are meant to be
committed so the next session inherits them. Claims and the task board are
runtime state and are gitignored.

## Install

```bash
curl -LsSf https://raw.githubusercontent.com/eabz/tirith/main/install.sh | sh
```

Windows:

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/eabz/tirith/releases/latest/download/tirith-installer.ps1 | iex"
```

Prebuilt binaries for macOS (Apple Silicon, Intel), Linux (x86_64, ARM64,
glibc and musl), and Windows (x86_64, ARM64) are on the
[releases page](https://github.com/eabz/tirith/releases). Other options,
including `cargo binstall` and building from source, are in
[docs/1-about/05-installation.md](docs/1-about/05-installation.md).

## Quick start

```bash
cd /path/to/your/repo
tirith serve            # MCP at http://127.0.0.1:7477/mcp, dashboard at http://127.0.0.1:7477/
```

The dashboard shows agents, claims with lease countdowns, the task board,
contracts with their current shape, change notices, and decisions,
refreshing every two seconds.

Point your agents at it. For Claude Code:

```bash
claude mcp add --transport http tirith http://127.0.0.1:7477/mcp
```

For Cursor, add to `.cursor/mcp.json`:

```json
{ "mcpServers": { "tirith": { "url": "http://127.0.0.1:7477/mcp" } } }
```

Other clients, including LangGraph, CrewAI, and raw JSON-RPC, are covered in
[docs/2-examples/02-client-setup.md](docs/2-examples/02-client-setup.md).

## Example: two agents, one file

Agent A claims the auth module. Agent B tries to claim a file inside it and
is refused with enough information to decide what to do next.

```bash
tirith serve &
tirith claim --agent alice --reason "refactor session handling" src/auth/
# ok       alice  src/auth  expires 18:20:00Z

tirith claim --agent bob --reason "fix login redirect" src/auth/login.rs
# conflict src/auth/login.rs overlaps src/auth (alice: "refactor session handling", expires 18:20:00Z)
```

The full script is [examples/demo.sh](examples/demo.sh); run it with
`./examples/demo.sh`.

The same calls from an agent look like this (MCP `tools/call`):

```json
{ "name": "claim",
  "arguments": { "agent": "bob",
                 "paths": ["src/auth/login.rs"],
                 "reason": "fix login redirect" } }
```

```json
{ "status": "conflict",
  "conflicts": [ { "path": "src/auth/login.rs",
                   "overlaps": "src/auth",
                   "owner": "alice",
                   "reason": "refactor session handling",
                   "expires_at": "2026-09-15T18:20:00Z" } ] }
```

## Example: a contract before the code

Agent A is building an HTTP handler, agent B the client that calls it. A
publishes the shape first; B reads it before writing a line.

```json
{ "name": "contract_publish",
  "arguments": {
    "agent": "alice",
    "name": "POST /api/sessions",
    "kind": "http",
    "shape": {
      "request":  { "email": "string", "password": "string" },
      "response": { "session_id": "string", "expires_at": "rfc3339" },
      "errors":   { "401": "invalid credentials" }
    },
    "consumers": ["src/client/sessions.rs"] } }
```

When A later republishes the contract with `token` instead of `session_id`,
Tirith emits a `contract` notice to the consumers automatically. A can also
publish an explicit notice, and B sees both on the next `notice_list` call
for `src/client/`:

```bash
tirith notice list --agent bob --path src/client/ --unread
# 20c12b66 contract  contract POST /api/sessions updated to v2 (was v1)  affects src/client/sessions.rs  by alice
# 60ece2ec rename    renamed session_id to token                          affects src/client/sessions.rs  by alice
```

## Every tool, from the CLI

```bash
tirith status
tirith claim | release | renew | claims
tirith task create | pull | update | list
tirith contract publish | get | list
tirith notice publish | list | ack
tirith decision record | list
tirith tools                    # list tools with descriptions
tirith call <tool> '<json>'     # call any tool directly
```

Add `--json` for the raw tool result. Non-`ok` outcomes exit with status 1.

## Documentation

Everything lives in [docs/](docs/README.md):

- `1-about/` purpose, architecture, project structure, primitives
- `2-examples/` demos and client setup for every supported framework
- `3-tests/` testing strategy and how to run the suite
- `4-style/` Rust rules, sources, git conventions
- `5-decisions/` architecture decision records
- `6-agent-workflow/` how agents work on this repo (Serena, memory, Tirith itself)

Contributing agents must read [AGENTS.md](AGENTS.md). This repository is
coordinated with Tirith itself: a daemon runs for the repo, and agents claim
files through it before editing. See
[docs/6-agent-workflow/03-tirith-dogfooding.md](docs/6-agent-workflow/03-tirith-dogfooding.md).

## License

MIT. See [LICENSE](LICENSE).
