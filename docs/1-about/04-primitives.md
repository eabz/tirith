# Primitives and tool reference

This is the authoritative list of MCP tools. Tool names and input fields
here are the contract; a change to them must update this file, the
examples, and `README.md` in the same commit.

Every tool takes an `agent` string identifying the caller and returns JSON
with a `status` field (see [02-architecture.md](02-architecture.md#tool-response-shape)).
Timestamps are RFC 3339 in UTC.

Status per tool: **Built**, **In progress**, or **Planned**. All tools below are **Built** as of 0.1.0 (pre-alpha); schemas may still change before 1.0.

## Claims

Leases on files or directories. Overlap is refused.

### `claim` — Built

| Field | Type | Notes |
|---|---|---|
| `agent` | string | Caller identity |
| `paths` | string[] | Repo-relative. A path covers everything beneath it, so `src/auth` and `src/auth/` mean the same thing |
| `reason` | string | Short, human-readable. Shown to whoever is refused |
| `ttl_secs` | integer, optional | Default 600. Max 3600 |

Returns `ok` with `claim_id`, `new_paths`, `renewed_paths`, and `expires_at`,
or `conflict` with one entry per overlapping path: `path`, `overlaps`,
`owner`, `reason`, `expires_at`. The call is atomic: on conflict, none of
the requested paths are claimed.

Re-claiming a path you already hold renews it (it appears in
`renewed_paths`) and returns `ok`.

### `release` — Built

| Field | Type | Notes |
|---|---|---|
| `agent` | string | |
| `paths` | string[], optional | Omit to release everything the agent holds |

Returns `ok` with `released`. Releasing a path you do not hold is
`not_found` and nothing is released; releasing when you hold nothing is
also `not_found`.

### `renew` — Built

Extends every lease held by `agent` by its original TTL. Returns `ok` with
`claims`, each carrying its new `expires_at`. Any other tool call by the
agent also renews, so this is only needed during long silent work.

### `claims_list` — Built

| Field | Type | Notes |
|---|---|---|
| `agent` | string | |
| `path` | string, optional | Only claims overlapping this path |

Returns `count` and `claims`: `id`, `owner`, `paths`, `reason`,
`claimed_at`, `expires_at`, `ttl_secs`. Expired claims are reaped before
the answer is built.

## Task board — Built

| Tool | Purpose |
|---|---|
| `task_create` | `title`, `description`, `priority` (higher pulls first, default 0), `depends_on[]` (task ids), `paths[]` (hint for claims). Returns `task` |
| `task_pull` | Returns the highest-priority `todo` task whose dependencies are all `done`, assigns it to `agent`, marks it `in_progress`. Ties go to the oldest task. Status `none` if nothing is unblocked |
| `task_update` | `task_id`, `status` in `todo`, `in_progress`, `blocked`, `done`, plus optional `note`. `in_progress` makes the caller the owner; `blocked` keeps the current owner and stores the note as `reason` |
| `task_list` | Filter by `status` or `owner`. Returns `count` and `tasks` |

A task's owner lives inside its status: `in_progress` has `owner`,
`blocked` has optional `owner` and `reason`, `done` has `by`.

## Contracts — Built

An interface shape published before implementation.

| Tool | Purpose |
|---|---|
| `contract_publish` | `name`, `kind` (`http`, `function`, `type`, `event`, `cli`, `other`), `shape` (free JSON), `consumers[]` (paths expected to depend on it), `notes`. Returns `contract`, `previous_version`, and `notice_id` |
| `contract_get` | `name` is a contract name or id. Returns `contract` with `current` and `history` |
| `contract_list` | Filter by `path` (overlaps `consumers`) or `kind`. Returns `count` and `contracts` |

Publishing a contract with an existing name creates a new version and
automatically emits a change notice referencing both versions.

## Change notices — Built

"Something changed and these files care."

| Tool | Purpose |
|---|---|
| `notice_publish` | `kind` (`rename`, `signature`, `removed`, `moved`, `behavior`), `summary`, `from`, `to`, `affected_paths[]`, optional `contract_id`. Returns `notice` |
| `notice_list` | `path` (notices whose `affected_paths` overlap it), `since` (RFC 3339), `unread` (boolean: only notices the caller neither published nor acknowledged). Returns `count` and `notices` |
| `notice_ack` | `notice_id`. Marks it handled by `agent` so `unread` filtering works. Returns `notice` |

A sixth kind, `contract`, is emitted automatically when a contract gets a
new version; agents do not publish it themselves.

## Decisions log — Built

| Tool | Purpose |
|---|---|
| `decision_record` | `title`, `decision`, `rationale`, `alternatives[]`, `affects_paths[]`. Returns `decision` |
| `decision_list` | Filter by `path` or case-insensitive `query` over title, decision, and rationale. Returns `count` and `decisions` |

## Status — Built

`status` takes an optional `agent` and returns counts (`claims`,
`tasks_open`, `tasks_done`, `contracts`, `notices`, `decisions`),
`uptime_secs`, `seq`, `version`, `persist_error`, and `agents`: per agent,
the `paths` held, when the latest lease `expires_at`, and
`tasks_in_progress`.

Decisions recorded here are agent-level project decisions, distinct from the
ADRs in `docs/5-decisions/`, which are decisions about Tirith itself.

## Resources — Planned

Read-only MCP resources mirroring the list tools, for clients that prefer
resources over tool calls:

- `tirith://claims`
- `tirith://tasks`
- `tirith://contracts`
- `tirith://notices`
- `tirith://decisions`
