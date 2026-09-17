# Primitives and tool reference

This is the authoritative list of MCP tools, and the only place their
schemas are written down. Tool names and input fields here are the
contract; a change to them must update this file, the examples, and
`README.md` in the same commit.

The daemon exposes 22 tools. Every tool takes an `agent` string
identifying the caller and returns JSON with a `status` field (see
[02-architecture.md](02-architecture.md#tool-response-shape)). Timestamps
are RFC 3339 in UTC. Schemas may still change before 1.0.

## Conventions shared by every tool

**Outcomes are status JSON, never MCP errors.** Every result has a
top-level `status`: `ok`, `conflict`, `not_found`, `none`, or `invalid`.
The structured content carries the outcome; the text block is a one-line
summary. If the change could not be written to disk, a mutating result
also carries `persist_error` and its summary starts with
`warning: not persisted`; the in-memory decision still stands.

**Lists are bounded and newest first.** `claims_list`, `task_list`,
`contract_list`, `notice_list`, `decision_list`, and `message_list` take
`limit` (default 20, maximum 200) and `before`, and return `count` (rows
in this response), `total` (rows that matched), `truncated`, and, when
truncated, `next_before`. Pass `next_before` back as `before` to get the
next older page; a bare RFC 3339 timestamp also works as `before` and
means "strictly older than". Two rows written in the same instant are
never skipped or repeated between pages because the cursor carries the
row id. `memory_search` is ranked rather than paged: it takes `limit`
only.

**Rows are compact.** List rows omit null fields and empty arrays, show
ids as their first eight characters, and round timestamps to seconds.
The tool that returns one item (`contract_get`, `task_update`, ...)
returns it in full.

**A lost lease is reported once, on your next call.** If a lease you held
has ended, the next response to you, whatever the tool, carries `lost`:
one `{path, owner, at}` per path, and its text line starts with
`warning: lost lease on N path(s)`. Stop editing those paths and claim
them again. `lost` is omitted when there is nothing to report, so
unaffected responses do not grow.

**Messages arrive the same way.** If another agent has messaged you, your
next response carries `inbox` (see [Messages](#messages)). Nothing is
attached when nothing is waiting.

**Any id input accepts a unique prefix.** `task_id`, `notice_id`,
`depends_on`, and `contract_get`'s `name` take a full id or any prefix
that matches exactly one item; an ambiguous prefix is `invalid`, an
unknown one `not_found`.

## Claims

Leases on files or directories. Overlap is refused.

### `claim`

| Field | Type | Notes |
|---|---|---|
| `agent` | string | Caller identity |
| `paths` | string[] | Repo-relative. A path covers everything beneath it, so `src/auth` and `src/auth/` mean the same thing |
| `reason` | string | Short, human-readable. Shown to whoever is refused |
| `ttl_secs` | integer, optional | Default 600. Max 3600 |
| `brief` | boolean, optional | Default `true`: attach the brief described below. `false` returns the bare claim outcome |
| `wait_secs` | integer, optional | Default 0. Max 120 (larger values are capped). On conflict, the daemon waits up to this long for the overlapping leases to be released or to expire, retrying the whole claim each time one ends, instead of the agent sleeping and calling again |

Returns `ok` with `claim_id`, `new_paths`, `renewed_paths`, `expires_at`,
the brief, and, when not empty, `absorbed_paths` (paths you already held
beneath a directory you just claimed, folded into the directory lease) and
`previous_owner` (for each new path whose lease another agent lost in the
last hour: `path`, `owner`, `reaped_at`). Or `conflict` with one entry per
overlapping path: `path`, `overlaps`, `owner`, `reason`, `expires_at`. The
call is atomic: on conflict, none of the requested paths are claimed.
With `wait_secs`, the result is `ok` as soon as every overlapping lease has
ended (each retry is atomic, so another agent can still win the paths
first), or the last `conflict` once the wait is over. The daemon holds no
lock while it waits: a release or an expiry wakes waiting claims.

**Symbol anchors (experimental, [ADR-0029](../5-decisions/0029-symbol-anchored-claims.md), Proposed).**
A `paths` entry may name a symbol inside a file: `path#Anchor`, for
example `src/state.rs#State::brief` or `app/config.py#Config.from_env`.
Daemons after 1.0.4 support it; older daemons treat `f#X` as an unrelated
path that does not overlap `f`, so check the version before relying on it.

- The string is split at the first `#`; the path part is normalized as
  usual and cannot end in `/`. Anchor segments may be separated by `::`,
  `.` or `/` and are stored and echoed with `::`. Generic arguments
  (`Store<T>::load`) and a Go receiver (`(*Server).Handle`) are dropped.
  Empty segments and whitespace inside a segment are refused. The daemon
  checks syntax only, never that the symbol exists.
- Overlap: a file or directory claim overlaps every anchor in or under it;
  two anchors in the same file overlap when they are equal or one encloses
  the other by whole segments (`Config` and `Config::from_env` yes,
  `Config::a` and `Config::b` no, `State` and `StateView` no), compared
  ASCII case-insensitively. The same rule applies to `claim` conflicts,
  `wait_secs` retries, `claims_list` with `path`, and `task_pull` holds.
- Your own claims: an anchor inside a file or anchor you hold is renewed,
  not added; claiming a file (or an enclosing anchor) absorbs your anchors
  inside it. `release` stays exact: releasing `f` does not release `f#X`.
- The brief, notices, contracts, decisions and memory match on the file
  part only, so any anchor in a file brings that file's rows.
- A new member is claimed by its new name (`app/config.py#Config::gzip`);
  the imports and export-list entries it needs ride on that claim. Under an
  anchor, edit by replacement and never rewrite the whole file.
- A whole-file claim waiting with `wait_secs` is not queued: new anchor
  claims in that file can keep it waiting.

**The brief** ([ADR-0014](../5-decisions/0014-brief-on-claim.md)) is what
the claim teaches you about the paths you just claimed, so you do not have
to ask four tools first. Four sections, each the five newest rows, each
omitted when empty:

- `notices`: unread notices you have not been shown before: `id` (8
  chars), `kind`, `summary` (at most 160 characters), `by`. Automatic
  `contract` notices coalesce: only the newest per contract is shown, and
  the older version notices for that contract are marked delivered with
  it, so they do not come back in a later brief or unread listing.
  Notices of other kinds that name a contract are never coalesced.
- `contracts`: contracts consumed by the paths: `name`, `version`, `kind`.
  Fetch a body with `contract_get`.
- `decisions`: decisions affecting the paths: `id`, `title`.
- `memory`: [memory notes](#memory-notes) about the paths:
  `permalink`, `title`, `kind`, `paths`, `tags`, `updated_at`, and a
  160-character `excerpt`. Never a whole body.

`more` is always present and counts, per section, the matching rows that
were left out; page with `notice_list`, `contract_list`, `decision_list`,
or `memory_search` when it is not zero. A notice shown in a brief is
marked seen by you, durably, and is not shown again by your next claim or
by `notice_list` with `unread: true`, so repeated claims page through the
notices you have not seen. There is no manual acknowledgement: delivery
is the acknowledgement (decision 3f6d77d9). The whole `ok` response is capped
at 4,096 bytes; when a brief would exceed that, the oldest rows of the
largest section are dropped and its `more` count raised. **Read the brief
before you edit.** It is the reason those rows were written.

Re-claiming a path you already hold renews it (it appears in
`renewed_paths`) and returns `ok`. A lease ends when its TTL passes
without activity, and after four TTLs (at most four hours) regardless of
activity; only `claim` or `renew` restarts that age. When a lease you held
has ended, your next response carries `lost` (see the conventions above).

### The reserved lead path

The swarm lead is whoever holds a claim naming exactly `.tirith/lead`
([ADR-0027](../5-decisions/0027-swarm-lead-and-escalation.md)). It is an
ordinary claim, so only one agent holds it, it expires and is reported in
`lost` like any other, and no extra tool exists. The session that spawns
other agents claims it first, with `ttl_secs: 3600` and a reason naming
the swarm; workers never claim it. A claim on an ancestor such as
`.tirith` blocks others from taking the lead path but does not make its
holder the lead. When the lease ends the swarm has no lead until someone
claims it again. `status` reports the holder as `lead`.

### `release`

| Field | Type | Notes |
|---|---|---|
| `agent` | string | |
| `paths` | string[], optional | Omit to release everything the agent holds |

Returns `ok` with `released`. Releasing a path you do not hold is
`not_found` and nothing is released; releasing when you hold nothing is
also `not_found`.

### `renew`

Extends every lease held by `agent` by its original TTL. Returns `ok` with
`count` (leases renewed) and `expires_at` (the latest new expiry). Any
other tool call by the agent also renews, so this is only needed during
long silent work.

### `claims_list`

| Field | Type | Notes |
|---|---|---|
| `agent` | string | |
| `path` | string, optional | Also include claims overlapping this path, whoever holds them |
| `all` | boolean, optional | Every live claim, not just yours and those overlapping `path` |
| `limit`, `before` | | See the list convention above |

By default the answer is the caller's own claims plus any claim
overlapping `path`, which is what a conflict check needs; `all: true` is
the whole board. Rows: `id`, `owner`, `paths`, `reason`, `claimed_at`,
`expires_at`, `ttl_secs`. Expired claims are reaped before the answer is
built.

## Task board

| Tool | Purpose |
|---|---|
| `task_create` | `title`, `description`, `priority` (higher pulls first, default 0), `depends_on[]` (task ids), `paths[]` (hint for claims). Returns `task` |
| `task_pull` | Returns the highest-priority `todo` task whose dependencies are all `done`, assigns it to `agent`, marks it `in_progress`. Ties go to the oldest task. A task whose `paths` overlap another agent's live claim or in-progress task is skipped for the next free one; when every candidate is held, the first is returned anyway with `waiting_on` (`path`, `owner`). Status `none` if nothing is unblocked. Optional `wait_secs` (default 0, max 120, larger values are capped) waits for a free task first; see below |
| `task_update` | `task_id`, `status` in `todo`, `in_progress`, `blocked`, `done`, plus optional `note` and `force`. `in_progress` makes the caller the owner; `blocked` keeps the current owner and stores the note as `reason`. Changing a task another agent has `in_progress` returns `conflict` with `owner` and `since` unless `force` is true, which records who forced it in the notes |
| `task_list` | Filter by `status` or `owner`; most recently updated first, paged (`limit`, `before`). Returns `count`, `total`, `truncated`, `next_before`, and `tasks` |

A task's owner lives inside its status: `in_progress` has `owner`,
`blocked` has optional `owner` and `reason`, `done` has `by`.

An owner that goes silent loses the task. The daemon remembers each
agent's last call; an `in_progress` task whose owner has made no call for
longer than the orphan threshold (1800 seconds by default,
`tirith serve --task-orphan-secs`, 0 disables) returns to `todo` with a
note `returned to todo: owner <name> silent since <time>`, so a dead agent
never blocks the tasks that depend on its work. The reap runs lazily on
every call, next to lease expiry, and `status` counts it as
`tasks_orphaned`. Last activity is not persisted: after a restart, an owner
counts as silent since its task last changed. See
[ADR-0018](../5-decisions/0018-task-ownership-and-contract-republish.md).

`task_pull` looks at claims so a puller is not sent straight into a
refused `claim`. Candidates keep their order, but a free lower-priority
task beats a held higher-priority one: held means one of its `paths`
overlaps (as claims do) a live claim of another agent or the `paths` of
another agent's `in_progress` task. The caller's own claims and tasks
never hold anything, and a task with no `paths` is never held. If every
candidate is held, the old pick is assigned and `waiting_on` lists the
overlapping held paths and their owners. See
[ADR-0028](../5-decisions/0028-claim-aware-task-pull.md).

**Waiting for work.** With `wait_secs`, a `task_pull` that finds no free
task (none unblocked, or every candidate held) does not answer at once.
The daemon waits, up to `wait_secs`, for something that could free one: a
claim released or expiring, a task created, or a task changing status.
Each time, it checks again, and as soon as a free task exists the pull
goes through the normal claim-aware path above; if another agent takes
that task first, it keeps waiting. On timeout it returns what a pull
without `wait_secs` returns: `none`, or a held task with `waiting_on`.
Nothing is assigned while it waits, and nothing is held against the
caller. Idle workers call it this way instead of ending their turn or
sleeping between pulls.

## Contracts

An interface shape published before implementation.

| Tool | Purpose |
|---|---|
| `contract_publish` | `name`, `kind` (`http`, `function`, `type`, `event`, `cli`, `other`), `shape` (free JSON), `consumers[]` (paths expected to depend on it), `notes`, optional `expected_version`. Returns `contract`, `previous_version`, and `notice_id`. When `expected_version` is set and the contract is at another version (0 means it must not exist yet), the result is `conflict` with `current_version` and `published_by` |
| `contract_get` | `name` is a contract name, id, or unique id prefix. Returns `contract` with `current` and `history` |
| `contract_list` | Filter by `path` (overlaps `consumers`) or `kind`; most recently published first, paged (`limit`, `before`). Returns `count`, `total`, `truncated`, `next_before`, and `contracts` |

Publishing a contract with an existing name creates a new version and
automatically emits a change notice referencing both versions. Omitting
`consumers` on a republish keeps the existing list; an explicit empty
list clears it. The notice goes to the union of the old and the new
consumers, so a path that stops consuming still hears about the version
that dropped it. Pass `expected_version` when two agents may publish the
same name concurrently: the loser gets `conflict` instead of silently
overwriting.

## Change notices

"Something changed and these files care."

| Tool | Purpose |
|---|---|
| `notice_publish` | `kind` (`rename`, `signature`, `removed`, `moved`, `behavior`), `summary`, `from`, `to`, `affected_paths[]`, optional `contract_id`. Returns `notice`. Every other agent holding one of `affected_paths` exactly, or an ancestor or descendant of one, gets a message from `tirith` in its inbox now, and the notice counts as seen by it, so its next brief does not repeat it |
| `notice_list` | `path` (notices whose `affected_paths` overlap it), `since` (RFC 3339), `unread` (boolean: only notices never delivered to the caller; listing them marks them seen), `all`; newest first, paged (`limit`, `before`). `unread` without `path` is scoped to the paths the caller currently holds claims on, so "what must I react to" is one call; `all: true` looks beyond them, and a caller holding nothing gets zero rows and a `message` saying so. Returns `count`, `total`, `truncated`, `next_before`, and `notices` |

A sixth kind, `contract`, is emitted automatically when a contract gets a
new version; agents do not publish it themselves.

## Decisions log

| Tool | Purpose |
|---|---|
| `decision_record` | `title`, `decision`, `rationale`, `alternatives[]`, `affects_paths[]`. Returns `decision`, which carries the `permalink` of its file under `.tirith/decisions/` (one committed Markdown file per decision, in the memory-note format, ADR-0022) |
| `decision_list` | Filter by `path` or case-insensitive `query` over title, decision, and rationale; newest first, paged (`limit`, `before`). Returns `count`, `total`, `truncated`, `next_before`, and `decisions` |

Decisions recorded here are agent-level project decisions, distinct from
the ADRs in `docs/5-decisions/`, which are decisions about Tirith itself.

## Memory notes

Durable, path-scoped knowledge an agent leaves for whoever comes next: a
lesson, a trap, the state of unfinished work. Each note is a committed
Markdown file at `.tirith/memory/<permalink>.md` with YAML frontmatter.
See [ADR-0011](../5-decisions/0011-memory-primitive.md).

This is the primitive that makes the other five worth keeping after a
session ends, and it is the one a general-purpose memory server cannot
provide: notes carry the paths they are about, so a successful `claim`
hands the agent the notes for the paths it just claimed. See
[the `claim` tool](#claim).

A note's `permalink` is a slug made from its title when it is created, and
it never changes afterwards, so links and filenames stay valid when the
title is edited. A permalink may carry `/` segments
(`tirith/design/pre-alpha-build`), which become nested directories. Every
segment is lowercase letters, digits, and dashes, so a permalink cannot
escape `.tirith/memory/`.

### `memory_write`

| Field | Type | Notes |
|---|---|---|
| `agent` | string | Caller identity |
| `title` | string | Becomes the permalink the first time |
| `body` | string | Markdown. Observations and relations are parsed out of it |
| `kind` | string, optional | `fact`, `lesson`, `gotcha`, `handoff`, `research`, `decision`, or `note` (default) |
| `paths` | string[], optional | Repo-relative paths the note is about |
| `tags` | string[], optional | Lowercased and deduplicated; a leading `#` is stripped |
| `permalink` | string, optional | Overwrite this note instead of matching on the title |
| `if_updated_at` | string, optional | RFC 3339. Refuse the write unless the note was last updated at exactly this instant |

Returns `ok` with `note` and `created`. Writing a title whose permalink
already exists updates that note in place, keeping its id and
`created_at`; `author` stays the first writer and `updated_by` becomes the
caller. An explicit `permalink` that does not exist is `not_found`.

Two agents editing the same note would otherwise silently lose one body.
Pass back the `updated_at` you read as `if_updated_at`: if the note changed
since, the write is refused with status `conflict` carrying `permalink` and
the real `updated_at`, so you can re-read and retry instead of clobbering.

Two line formats inside `body` are parsed. They are the ones Basic Memory
used, which is where this repository's own notes came from before
[ADR-0012](../5-decisions/0012-retire-basic-memory.md), so notes written
by other Markdown memory tools import without translation:

```markdown
- [design] Writes go through spawn_blocking #storage
- follows [[pre-alpha-build]]
```

The first is an observation: a category, text, and hashtags. The second is
a relation to another note, by permalink or title.

### `memory_read`

| Field | Type | Notes |
|---|---|---|
| `agent` | string | |
| `name` | string | A permalink, an id, or an exact title |
| `depth` | integer, optional | Default 0, maximum 3. Above 0, also returns notes linked within that many hops, following relations in both directions |

Returns `ok` with `note` and `related`, or `not_found`. `note` is the only
place a body is ever returned. `related` rows are digests (see
`memory_search`), at most 20 of them. A `depth` above 3 is `invalid`: a
dense relation graph reaches every note in a few hops.

### `memory_search`

| Field | Type | Notes |
|---|---|---|
| `agent` | string | |
| `query` | string, optional | Free text |
| `path` | string, optional | Only notes whose paths overlap this one |
| `kind` | string, optional | One of the kinds above |
| `tag` | string, optional | Matches frontmatter tags and observation hashtags |
| `since` | string, optional | RFC 3339; only notes updated at or after it |
| `limit` | integer, optional | Default 10, clamped to 50. Ten digest rows are about 2.3 KB, which keeps a search below a notice listing |

Returns `ok` with `count`, `truncated`, and `notes`. Each row is a
**digest**, never a whole note: `permalink`, `title`, `kind`, `paths`,
`tags`, `updated_at`, a 160-character `excerpt`, and `score`. Bodies are
Markdown prose and often kilobytes; twenty of them to answer "what is
here?" would spend your context on text you did not ask for. Read the one
you want. Scoring weights titles and tags above observations, and
observations above body prose; notes matching every term rank above notes
matching some. `truncated` is true when the limit hid further matches.

**With no `query` this is recent activity**, newest first, which is the
call to make when a session starts.

Search is plain term scoring in process, and matching is on substrings, so
`lease` finds `leases`. There are no embeddings and no vector index,
deliberately: see the alternatives in ADR-0011.

### `memory_delete`

| Field | Type | Notes |
|---|---|---|
| `agent` | string | |
| `name` | string | A permalink, an id, or an exact title |

Returns `ok` with `removed` (a digest) or `not_found`. The note's file is
deleted and does not come back on restart. A note holding a secret or a
plainly wrong fact has to be retractable through the same path that wrote
it; deleting the file by hand is undone by the next full rewrite.

## Messages

Short notes between agents, for the coordination talk that used to go
through a client's own chat: "take task X", "I released server.rs". They
are runtime state in `.tirith/runtime/messages.jsonl`, never committed,
and dropped after 24 hours. See
[ADR-0020](../5-decisions/0020-agent-messages.md).

| Tool | Purpose |
|---|---|
| `message_send` | `to` (an agent name, or `*` for every agent seen in the last hour), `text` (at most 1000 characters), optional `reply_to` (a message id or unique prefix) and `paths[]`. Returns `message` |
| `message_list` | Your own conversations: messages you sent or received, newest first, paged (`limit`, `before`). Filter by `with` (the other agent), `since` (RFC 3339), or `unread` (to you, not yet received). Returns `count`, `total`, `truncated`, `next_before`, and `messages` |

Delivery needs no tool. The recipient's next result, whatever it asked
for, carries `inbox`: the newest five undelivered messages as
`{id, from, text, at}` with `text` cut to 200 characters, plus
`inbox_more` when more are waiting, and the text line starts with
`warning: inbox: n;`. Each message is delivered once per daemon lifetime;
after a restart anything still within retention is delivered again. A
broadcast is addressed, at send time, to every agent that made a call in
the previous hour, minus the sender; agents that show up later do not
receive it. Nothing is attached when nothing is waiting.

## Status

`status` takes an optional `agent` and `verbose`. It returns counts
(`claims`, `tasks_open`, `tasks_done`, `tasks_orphaned`, `contracts`,
`notices`, `decisions`, `memory`, `agents_active`), `started_at`, `now`,
`uptime_secs`, `seq`, `version`, `lead` (the agent holding the
[`.tirith/lead`](#the-reserved-lead-path) claim, or null), `persist_error`
(the last write failure,
or null), and `load_errors`: files or lines the daemon skipped at startup
because they would not parse, each as `path` (relative to `.tirith/`),
optional `line`, and `error`. A daemon never refuses to start over one
bad file; it reports it here, on `/api/health`, and on the dashboard, and
keeps the bad lines in place when it rewrites the file.

With `verbose: true` it also returns `agents` (at most 50 rows; then
`agents_truncated` is true): per agent, `paths_count`, when the latest
lease `expires_at`, and `tasks_in_progress`, plus `lead_expires_at` when
there is a lead. The dashboard's `/api/state` remains the full view for
humans; it shows the lead and its lease in the header (`server.lead`).

## The lead decision log

What the lead policy (`src/lead.rs`, deterministic, no network) decides is
appended to `.tirith/runtime/lead_log.jsonl` (runtime state, gitignored,
rows older than 7 days dropped at startup) by the persister in the
background; no tool call waits for it
([ADR-0027](../5-decisions/0027-swarm-lead-and-escalation.md)). It is not
an MCP tool. Read it with `GET /api/lead` on the dashboard port
(`?limit=N`, default 50, max 500; `?event=claim_refused` to filter), which
returns `lead`, `count`, and `entries` newest first, or with
`tirith lead log [--limit N] [--event E]`.

Each row has `id`, `at`, `event`, `seq` (the state sequence number read),
`candidates` (agents or paths), `rule` (the rule that decided, or null),
`action`, `outcome` (filled in later, when observable), and, when present,
`agent` and `details`.

Events:

- `claim_granted` (`details`: new, renewed and absorbed paths, reason,
  `ttl_secs`), `claim_refused` (the conflicts and their owners),
  `claim_waited` (a `wait_secs` claim: `waited_ms` and whether it was
  granted), `claim_released`, and `lease_ended` (rule
  `ttl_without_activity` or `max_lease_age`), so claims stay auditable
  after they are gone.
- `notice_published`: a notice pushed to the holders of its affected
  paths (rule `holds_affected_path`; `details`: `notice`, `kind`,
  `affected_paths`, `pushed`). No row when nobody holds such a path.
- `escalation_raised`: one row per routed escalation (below).

## Escalation routing

The lead policy raises and routes escalations
([ADR-0027](../5-decisions/0027-swarm-lead-and-escalation.md) section 3)
with fixed rules, as part of the call that triggered them. No tool or
parameter was added.

**Triggers.**

- A `task_update` to `blocked` with a note; the note is the text.
- The third refusal of the same claim (same agent, same paths) within
  360 s, three full claim waits, including a refused `wait_secs` claim.
  One escalation per run of refusals; the text names the paths, the
  claim's reason, and who holds them.
- A `message_send` to the lead (the holder of `.tirith/lead`) by name,
  from anyone else, **only** when its text matches a human rule. Any other
  message to the lead is just delivered.

**Routes.**

| condition | route | delivery |
|---|---|---|
| the text (and paths) match a human rule | `human` | the human queue, and a message from `tirith` to the lead |
| no human rule matches, and there is a lead other than the escalating agent | `lead` | a message from `tirith` to the lead |
| no lead, or the lead is the escalating agent | `human` | the human queue (rule `no_lead`) |

The human rules match phrases at word boundaries: credentials,
permissions, spending, a destructive or irreversible step, or a text
addressed to the human. The phrase list is `lead::HUMAN_RULES`; the row's
`rule` is `human:<category>`. Workers never see routes or rules.

**The log row** has `agent` and `details`: `trigger` (`task_blocked`,
`message_to_lead`, `claims_refused`), `text` (at most 1000 characters),
`task`, `paths`, `route`, and `delivered` (`lead`, `human_queue`). Its
`outcome` is filled in when the escalation is answered: a message to the
escalating agent from anyone (`via: message`), its task leaving `blocked`
(`via: task_in_progress`, `task_done`, `task_todo`), or the refused claim
granted (`via: claim_granted`), with `answered_by` and `after_secs`.

**The human queue** is every `escalation_raised` row delivered to
`human_queue` with no outcome yet, ranked by how many agents it blocks
(the escalating agent, others escalating on the same task, and owners of
open tasks that depend on it), then by how long it has waited. Read it
with `GET /api/human` (`lead`, `count`, `items`), `tirith lead human`, the
dashboard's "Needs you" list (`needs_you` in `/api/state`), or the macOS
tray, which shows the count per daemon and posts a notification when it
grows.

## Resources — Planned

Read-only MCP resources mirroring the list tools, for clients that prefer
resources over tool calls:

- `tirith://claims`
- `tirith://tasks`
- `tirith://contracts`
- `tirith://notices`
- `tirith://decisions`
