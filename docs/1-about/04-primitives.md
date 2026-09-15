# Primitives and tool reference

This is the authoritative list of MCP tools. Tool names and input fields
here are the contract; a change to them must update this file, the
examples, and `README.md` in the same commit.

Every tool takes an `agent` string identifying the caller and returns JSON
with a `status` field (see [02-architecture.md](02-architecture.md#tool-response-shape)).
Timestamps are RFC 3339 in UTC.

Status per tool: **Built**, **In progress**, or **Planned**.

## Claims

Leases on files or directories. Overlap is refused.

### `claim` — In progress

| Field | Type | Notes |
|---|---|---|
| `agent` | string | Caller identity |
| `paths` | string[] | Repo-relative. A trailing `/` means the whole directory |
| `reason` | string | Short, human-readable. Shown to whoever is refused |
| `ttl_secs` | integer, optional | Default 600. Max 3600 |

Returns `ok` with `claim_id` and `expires_at`, or `conflict` with one entry
per overlapping path: `path`, `overlaps`, `owner`, `reason`, `expires_at`.
The call is atomic: on conflict, none of the requested paths are claimed.

Re-claiming a path you already hold renews it and returns `ok`.

### `release` — In progress

| Field | Type | Notes |
|---|---|---|
| `agent` | string | |
| `paths` | string[], optional | Omit to release everything the agent holds |

Returns `ok` with the released paths. Releasing a path you do not hold is
`not_found`, never a silent success.

### `renew` — In progress

Extends every lease held by `agent` by its original TTL. Returns `ok` with
the new `expires_at` per path. Any other tool call by the agent also renews.

### `claims_list` — In progress

| Field | Type | Notes |
|---|---|---|
| `agent` | string | |
| `path` | string, optional | Only claims overlapping this path |

Returns all live claims: `owner`, `paths`, `reason`, `claimed_at`,
`expires_at`. Expired claims are reaped before the answer is built.

## Task board — Planned

| Tool | Purpose |
|---|---|
| `task_create` | `title`, `description`, `depends_on[]`, `paths[]` (hint for claims). Returns `task_id` |
| `task_pull` | Returns the highest-priority task whose dependencies are all `done`, assigns it to `agent`, marks it `in_progress`. `none` if nothing is unblocked |
| `task_update` | `task_id`, `status` in `todo`, `in_progress`, `blocked`, `done`, plus optional `note` |
| `task_list` | Filter by `status` or `owner` |

## Contracts — Planned

An interface shape published before implementation.

| Tool | Purpose |
|---|---|
| `contract_publish` | `name`, `kind` (`http`, `function`, `type`, `event`, `cli`, `other`), `shape` (free JSON), `consumers[]` (paths expected to depend on it), `notes`. Returns `contract_id` and `version` |
| `contract_get` | By `name` or `contract_id`. Returns the latest version and its history |
| `contract_list` | Filter by `path` (matches `consumers`) or `kind` |

Publishing a contract with an existing name creates a new version and
automatically emits a change notice referencing both versions.

## Change notices — Planned

"Something changed and these files care."

| Tool | Purpose |
|---|---|
| `notice_publish` | `kind` (`rename`, `signature`, `removed`, `moved`, `behavior`), `summary`, `from`, `to`, `affected_paths[]`, optional `contract_id`. Returns `notice_id` |
| `notice_list` | `path` (returns notices whose `affected_paths` overlap it), `since` (timestamp or `notice_id`), `unread_by` (agent) |
| `notice_ack` | `notice_id`. Marks it handled by `agent` so `unread_by` filtering works |

## Decisions log — Planned

| Tool | Purpose |
|---|---|
| `decision_record` | `title`, `decision`, `rationale`, `alternatives[]`, `affects_paths[]`. Returns `decision_id` |
| `decision_list` | Filter by `path` or free-text `query` |

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
