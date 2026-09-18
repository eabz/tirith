<div align="center">
  <img src="https://raw.githubusercontent.com/eabz/tirith/main/docs/_static/images/logo-512.jpeg" alt="Tirith" width="140">

  <h1>Tirith</h1>

  <p><strong>Coordination server for parallel coding agents.</strong><br>
  Claims, a task board, interface contracts, change notices, a decisions log,
  path-scoped memory notes, and agent messages, over MCP.</p>

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
claims files before editing, gets the notices, contracts, decisions, and
notes for those files back with the claim, and publishes the shape of an
interface before either side implements it.

It works with anything that speaks MCP, over stdio or HTTP: Claude Code,
Cursor, Codex, LangGraph, CrewAI, or a plain script.

It also remembers. Agents leave notes scoped to repository paths, so what
one agent learned about a file reaches the next agent that claims it.
Tirith stores no conversation history and no embeddings.

> **Status: v1 shipped.** Every primitive, the CLI, persistence, and the
> dashboard are built and tested, against the definition in
> [ADR-0013](docs/5-decisions/0013-v1-definition.md); tool schemas follow
> semantic versioning.

## Install

macOS and Linux:

```bash
curl -LsSf https://eabz.github.io/tirith/install.sh | sh
```

Windows (PowerShell):

```powershell
irm https://eabz.github.io/tirith/install.ps1 | iex
```

With cargo (the package is `tirith-mcp`, the binary is `tirith`):

```bash
cargo install tirith-mcp
```

Already installed? `tirith update` replaces the binary in place with the
latest release. Prebuilt binaries for macOS, Linux, and Windows on x86_64
and ARM64 are on the [releases page](https://github.com/eabz/tirith/releases);
every option is in [docs/1-about/05-installation.md](docs/1-about/05-installation.md).

Listed in the [MCP Registry](https://registry.modelcontextprotocol.io/):
`mcp-name: io.github.eabz/tirith`.

## Quick start

Register `tirith stdio` with your client, the same way as any other stdio
MCP server. It starts the repository's daemon the first time a session
needs it, replaces a daemon of another version, and proxies to it after
that; nothing has to be started by hand.

| Client | Setup |
|---|---|
| Claude Code | `claude mcp add tirith -- tirith stdio` |
| Cursor | `.cursor/mcp.json`: `{ "mcpServers": { "tirith": { "command": "tirith", "args": ["stdio"] } } }` |
| Codex | `codex mcp add tirith -- tirith stdio` |
| LangGraph, CrewAI, curl | connect over HTTP, see [docs/2-examples/02-client-setup.md](docs/2-examples/02-client-setup.md) |

The daemon serves MCP at `http://127.0.0.1:7477/mcp` and a live dashboard
at `http://127.0.0.1:7477/`. State is written to `.tirith/` in your
repository: contracts, notices, decisions, and memory notes are meant to
be committed; `.tirith/runtime/` (claims, tasks, messages) is gitignored
by a `.gitignore` Tirith writes itself.

Every agent passes a stable `agent` name with each call. That is the only
convention it has to follow.

## What it does

| Primitive | Tools | Purpose |
|---|---|---|
| **Claims** | `claim`, `release`, `renew`, `claims_list` | Lease files or directories before editing. Overlaps are refused with the owner, reason, and expiry, or waited out server-side with `wait_secs`. Leases expire if the agent dies, and the agent is told on its next call. |
| **Task board** | `task_create`, `task_pull`, `task_update`, `task_list` | Tasks with priority, owner, and dependencies. Agents pull the next unblocked task, skipping tasks another agent holds, and can wait server-side for one with `wait_secs`; a task whose owner goes silent returns to the board. |
| **Contracts** | `contract_publish`, `contract_get`, `contract_list` | Interface shapes published and versioned before implementation. A new version notifies its consumers automatically. |
| **Change notices** | `notice_publish`, `notice_list` | "Renamed `X` to `Y`, these paths are affected." Dependents get them in the brief that comes back with a claim, once each. |
| **Decisions log** | `decision_record`, `decision_list` | Settled choices with rationale, so nothing is decided twice. |
| **Memory notes** | `memory_write`, `memory_read`, `memory_search`, `memory_delete` | Lessons, traps, and handoffs scoped to repository paths. Committed Markdown, searchable, and delivered to whoever claims the paths a note is about. |
| **Messages** | `message_send`, `message_list` | Short notes between agents, delivered on the recipient's next call, so any MCP client can take part. Runtime only. |
| **Status** | `status` | Counts, persistence and load problems; `verbose` adds who holds what. |

Twenty-two tools. Every result is JSON with a `status` field, lists are
paged, and any result may carry `lost` (a lease that ended) or `inbox`
(messages waiting). The full reference, the single source of truth for
tool schemas, is [docs/1-about/04-primitives.md](docs/1-about/04-primitives.md).

## Example

Two agents, one directory. The second is refused with enough information to
decide what to do next:

```bash
tirith claim --agent alice --reason "refactor session handling" src/auth/
# ok       alice  src/auth  expires 04:14:34Z

tirith claim --agent bob --reason "fix login redirect" src/auth/login.rs
# conflict src/auth/login.rs overlaps src/auth (alice: "refactor session handling", expires 04:14:34Z)
```

A note left for whoever edits a path next. Alice writes it once; it comes
back on Bob's claim without anyone searching, as an excerpt, never a body:

```bash
tirith memory write --agent alice "Session ids are opaque" -k gotcha --path src/auth/ <<'NOTE'
Tokens are opaque strings. Compare them, never parse them.
NOTE

tirith release --agent alice
tirith claim --agent bob --reason "fix login redirect" src/auth/login.rs
# ok       bob  src/auth/login.rs  expires 04:14:34Z
# memory:
#   session-ids-are-opaque gotcha   04:04:34Z  paths src/auth  Session ids are opaque: Tokens are opaque strings. Compare them, never parse them.
```

The full walkthrough, with contracts, notices, and the same calls over
raw MCP, is [docs/2-examples/01-two-agents-demo.md](docs/2-examples/01-two-agents-demo.md);
the script is [examples/demo.sh](examples/demo.sh).

## CLI

Every tool has a subcommand; the CLI uses the same MCP path agents do.

```bash
tirith serve                               # run the repo's daemon by hand
tirith stdio                               # per-session shim clients spawn; starts the daemon if needed
tirith update                              # replace the binary with the latest release
tirith status                              # counts, and who holds what
tirith claim | release | renew | claims
tirith task     create | pull | update | list
tirith contract publish | get | list
tirith notice   publish | list
tirith decision record | list
tirith memory   write | read | search | delete   # body from --body, --file, or stdin
tirith message  send | list                # talk to other agents; the inbox shows on any result
tirith lead log                            # the swarm lead and its policy's decisions
tirith lead human                          # items waiting for you, most agents blocked first
tirith lead human done <id> [--reply TEXT] # answer one; the reply reaches its sender
tirith tray                                # macOS only: menu bar icon listing every daemon
tirith tools                               # list tools with descriptions
tirith call <tool> '<json>'                # call any tool directly
```

`--agent` sets your name, `--json` prints the raw result, and non-`ok`
outcomes exit with status 1.

## Swarm lead and escalations

The session that spawns other agents claims the reserved path
`.tirith/lead` and becomes the swarm lead; `status` and the dashboard show
who holds it. The daemon then handles routine escalations with fixed
rules, without spending an LLM turn:

- **What escalates:** a task set to `blocked` (its note is the reason), or
  the third refusal of the same claim within 360 s. A message to the lead
  is never an escalation.
- **Where it goes:** while there is a lead, to the lead's inbox as a
  message from `tirith`, tagged "may need the human" when it mentions
  credentials, permissions, spending, or destructive steps. Nothing
  reaches you on its own: the lead relays what needs you with
  `message_send` to `human`, written for you. While there is no lead,
  escalations go to your queue.
- **Your queue:** the dashboard's "Needs you" list, `tirith lead human`,
  `/api/human` on the dashboard port, and the macOS tray, which lists each
  item's sender and first line and notifies you of new ones. Answer with
  the dashboard's Done button or `tirith lead human done <id> --reply
  "..."`; the reply reaches the sender as a message from `human`.

Claim grants, refusals, waits, releases and lease ends, notice pushes, and
escalations are appended to the lead decision log: `tirith lead log`, or
`/api/lead` on the dashboard port. Design:
[ADR-0027](docs/5-decisions/0027-swarm-lead-and-escalation.md); per tool:
[04-primitives.md](docs/1-about/04-primitives.md#escalation-routing).

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
| [index.html](index.html) | The landing page at [eabz.github.io/tirith](https://eabz.github.io/tirith/) |

## Contributing

Coding agents must read [AGENTS.md](AGENTS.md) first. This repository is
coordinated with Tirith itself: a daemon runs for the repo and agents claim
files through it before editing.

## License

[MIT](LICENSE)
