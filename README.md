<div align="center">
  <img src="https://raw.githubusercontent.com/eabz/tirith/main/docs/_static/images/logo-512.jpeg" alt="Tirith" width="140">

  <h1>Tirith</h1>

  <p><strong>Coordination server for parallel coding agents.</strong><br>
  Claims, a task board, interface contracts, change notices, and a decisions log, over MCP.</p>

  <p>
    <a href="https://github.com/eabz/tirith/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/eabz/tirith/actions/workflows/ci.yml/badge.svg"></a>
    <a href="https://github.com/eabz/tirith/releases/latest"><img alt="Release" src="https://img.shields.io/github/v/release/eabz/tirith?display_name=tag"></a>
    <a href="https://crates.io/crates/tirith-mcp"><img alt="crates.io" src="https://img.shields.io/crates/v/tirith-mcp"></a>
    <a href="LICENSE"><img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-blue"></a>
  </p>
</div>

Run five or ten coding agents on one repository and they edit the same
files, rename things others depend on, and build both sides of an interface
to different shapes. Tirith is a small daemon they all talk to: an agent
claims files before editing, reads change notices before acting, and
publishes the shape of an interface before either side implements it.

It works with anything that speaks MCP over HTTP: Claude Code, Cursor,
Codex, LangGraph, CrewAI, or a plain script. It is not a memory layer.

> **Status: pre-alpha.** All primitives, the CLI, persistence, and the
> dashboard exist and are tested. Tool schemas may change before 1.0.

## Install

macOS and Linux:

```bash
curl -LsSf https://raw.githubusercontent.com/eabz/tirith/main/install.sh | sh
```

Windows:

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/eabz/tirith/releases/latest/download/tirith-mcp-installer.ps1 | iex"
```

With cargo (the package is `tirith-mcp`, the binary is `tirith`):

```bash
cargo install tirith-mcp
```

Prebuilt binaries for macOS, Linux, and Windows on x86_64 and ARM64 are on
the [releases page](https://github.com/eabz/tirith/releases). More options
in [docs/1-about/05-installation.md](docs/1-about/05-installation.md), or
on the install page at [eabz.github.io/tirith](https://eabz.github.io/tirith/).

## Quick start

Register `tirith stdio` with your client, the same way as any other stdio
MCP server. It starts the repository's daemon the first time a session
needs it and proxies to it after that; nothing has to be started by hand.

| Client | Setup |
|---|---|
| Claude Code | `claude mcp add tirith -- tirith stdio` |
| Cursor | `.cursor/mcp.json`: `{ "mcpServers": { "tirith": { "command": "tirith", "args": ["stdio"] } } }` |
| Codex | `codex mcp add tirith -- tirith stdio` |
| LangGraph, CrewAI, curl | connect over HTTP, see [docs/2-examples/02-client-setup.md](docs/2-examples/02-client-setup.md) |

The daemon serves MCP at `http://127.0.0.1:7477/mcp` and a live dashboard
at `http://127.0.0.1:7477/`. You can also run it yourself with
`tirith serve` from the repository root.

Every agent passes a stable `agent` name with each call. That is the only
convention it has to follow.

## What it does

| Primitive | Purpose |
|---|---|
| **Claims** | Lease files or directories before editing. Overlaps are refused with the owner, reason, and expiry. Leases expire if the agent dies. |
| **Task board** | Tasks with priority, owner, and dependencies. Agents pull the next unblocked task. |
| **Contracts** | Interface shapes published and versioned before implementation. A new version notifies its consumers automatically. |
| **Change notices** | "Renamed `X` to `Y`, these paths are affected." Dependents read them before acting and acknowledge when handled. |
| **Decisions log** | Settled choices with rationale, so nothing is decided twice. |

Claims and the task board are table stakes. Contracts and change notices are
the reason Tirith exists: nothing else covers them today. The full tool
reference is in [docs/1-about/04-primitives.md](docs/1-about/04-primitives.md).

## Example

Two agents, one directory. The second is refused with enough information to
decide what to do next:

```bash
tirith claim --agent alice --reason "refactor session handling" src/auth/
# ok       alice  src/auth  expires 18:20:00Z

tirith claim --agent bob --reason "fix login redirect" src/auth/login.rs
# conflict src/auth/login.rs overlaps src/auth (alice: "refactor session handling", expires 18:20:00Z)
```

Over MCP the same refusal is structured, so agents branch on `status`
instead of parsing text:

```json
{ "status": "conflict",
  "conflicts": [ { "path": "src/auth/login.rs", "overlaps": "src/auth",
                   "owner": "alice", "reason": "refactor session handling",
                   "expires_at": "2026-09-15T18:20:00Z" } ] }
```

A contract before the code. Alice publishes the shape of an endpoint; Bob
reads it before writing the client. When Alice later changes the response,
the consumers get a notice without anyone remembering to send one:

```bash
tirith contract publish --agent alice "POST /api/sessions" -k http \
  -s '{"request":{"email":"string","password":"string"},"response":{"token":"string"}}' \
  --consumer src/client/sessions.rs

tirith notice list --agent bob --path src/client/ --unread
# 20c12b66 contract  contract POST /api/sessions updated to v2 (was v1)  affects src/client/sessions.rs  by alice
```

The full two-agent demo is [examples/demo.sh](examples/demo.sh).

## CLI

Every tool has a subcommand; the CLI uses the same MCP path agents do.

```bash
tirith status                              # counts and who holds what
tirith claim | release | renew | claims
tirith task     create | pull | update | list
tirith contract publish | get | list
tirith notice   publish | list | ack
tirith decision record | list
tirith tools                               # list tools with descriptions
tirith call <tool> '<json>'                # call any tool directly
```

`--agent` sets your name, `--json` prints the raw result, and non-`ok`
outcomes exit with status 1.

## How it runs

One daemon per repository, over streamable HTTP on localhost. MCP clients
normally spawn a fresh server per session, which would give ten agents ten
private states; Tirith is deliberately one shared process. State is held in
memory and written through to JSON under `.tirith/` in your repository.
Contracts, notices, and decisions are meant to be committed so the next
session inherits them; claims and tasks are runtime state and gitignored.

Details in [docs/1-about/02-architecture.md](docs/1-about/02-architecture.md)
and the decision records in [docs/5-decisions/](docs/5-decisions/README.md).

## Documentation

| Section | Contents |
|---|---|
| [1-about](docs/1-about/) | Purpose, architecture, project structure, tool reference, installation |
| [2-examples](docs/2-examples/) | The demo and client setup for every supported framework |
| [3-tests](docs/3-tests/) | Testing strategy |
| [4-style](docs/4-style/) | Rust rules and their sources, git conventions |
| [5-decisions](docs/5-decisions/) | Architecture decision records |
| [6-agent-workflow](docs/6-agent-workflow/) | How agents work on this repo: Serena, memory, Tirith on itself |
| [7-release](docs/7-release/) | Release process and version bumping |
| [index.html](docs/index.html) | The landing page served at [eabz.github.io/tirith](https://eabz.github.io/tirith/) |

## Contributing

Coding agents must read [AGENTS.md](AGENTS.md) first. This repository is
coordinated with Tirith itself: a daemon runs for the repo and agents claim
files through it before editing.

## License

[MIT](LICENSE)
