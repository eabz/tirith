# ADR-0013: What v1 means

**Status:** Accepted, 2026-09-16. Recorded by the head-developer session for
eabz. Refines the roadmap in [../1-about/01-purpose.md](../1-about/01-purpose.md).

## Context

Tirith is at 0.1.3, pre-alpha, and is already used to coordinate five
parallel Claude sessions on its own repository. That use surfaced three
things at once: the storage rework (ADR-0010), the memory primitive
(ADR-0011), and a set of measurements showing that the token cost of
talking to Tirith is the real limit on how many agents can use it. Without
a written definition of v1, every session would pick its own finish line.

## Decision

v1 is the first release for which all three statements below are true and
each is enforced by a test that fails when it stops being true.

### 1. It works as a coordination layer

- Five to ten agents can claim, pull tasks, publish contracts and notices,
  and release, with no overlapping claims ever granted and no lost
  mutation on a clean shutdown. `tests/persistence.rs` and
  `examples/swarm_bench.rs` (200 agents) prove it.
- The protocol has one mandatory read before an edit, not four: `claim`
  returns, for the claimed paths, the unread notices, the contracts, the
  decisions, and the memory notes that concern them (task 87e14abd,
  ADR-0014). `notice_list` and friends remain for paging.
- Every session talks to a daemon built from the same version as its
  shim. The shim replaces a daemon whose version differs from its own
  (task 2e31aea6). Until this lands, an install does not upgrade anyone.
- A task whose owner has vanished does not stay `in_progress` forever,
  and no agent can silently take another agent's task (task 401ed7c9).
- An agent learns that it lost a lease from its next response, and
  activity alone cannot keep a lease alive forever (task 44ee9c43,
  ADR-0015).
- Republishing a contract keeps its consumers and can refuse a stale
  version (task 5ed3595f).
- A persist failure is visible in the response that suffered it, writes
  are fsynced before rename, and a corrupt line or file in `.tirith/`
  is reported in `status` instead of stopping the daemon (tasks
  6f41fed4, 85cc7844).

### 2. It works as a memory layer

- `memory_write`, `memory_read`, and `memory_search` are registered on
  the server, persisted one Markdown file per note under
  `.tirith/memory/`, and survive a restart (task af9b4a0e).
- A `claim` carries the newest notes for the claimed paths as excerpts,
  never bodies, capped at five (task 6122de2d).
- Search results are bounded: indexed lookups by permalink, pre-tokenized
  bodies, a default and a maximum `limit`, a maximum `depth`
  (tasks 33642433, c3888a7b, done). Scoring work is still linear in the
  number of notes; an inverted index (task 5de9e786) is out of v1 unless
  the swarm benchmark with a memory workload shows search on the
  critical path.
- A note can be retracted with `memory_delete`, and a concurrent update
  to the same permalink is refused with `conflict` instead of silently
  overwriting (task d4f23a0a).
- A corrupt or unloadable note file is reported in `status`, never a
  reason for the daemon to refuse to start, and no write can produce one
  (task ff6b9d5b).
- Search rows are digests with an excerpt, scoring is term based, and
  any title yields a safe, unique permalink (task 6137f8af).
- The CLI and the dashboard expose notes (tasks 1e0de1eb, dd6d6450), and
  the docs mark the tools Built (task db91b056).

### 3. It uses the fewest tokens that still carry the information

Measured on 2026-09-16 against a 0.1.3 daemon holding this repository's
own state: `tools/list` 10.5 KB, `task_list` 66 KB, `decision_list`
34 KB, `contract_list` 30 KB, `notice_list` 12 KB, and every result sent
twice (structured plus the same JSON as text). The budgets below are the
v1 contract with callers, each guarded by a test in `tests/budgets.rs`
whose name reads like its row, run against a daemon seeded with 300
claims, notices and decisions (the first two rows are also pinned in
`tests/http_roundtrip.rs`). CI runs that file with the rest of the suite:

| Surface | Budget |
|---|---|
| `tools/list`, 22 tools | under 10.5 KB, no tool over 650 B (measured 6,656 B with 20 tools; `memory_delete` stayed under 7 KB; `message_send` and `message_list` added about 640 B, ADR-0020; `notice_ack` removed, ADR-0021; annotations on every tool and a usage clause on 17 raised 7.8 KB to about 10.4 KB, ADR-0033) |
| Text content block of any result | under 200 bytes, a status line, never JSON |
| Any list tool with default arguments | at most 20 rows, newest first, `truncated` and a `before` cursor |
| A list row | no null fields, no empty arrays, 8-char id prefix, seconds-precision timestamps |
| `status`, `claims_list`, `renew` | counts and the caller's own rows by default; the full board only with `all: true` |
| `claim` with brief | under 4 KB including notices, contracts, decisions, memory |
| `memory_search` row | excerpt of about 160 chars; only `memory_read` returns a body |

The tasks that implement these are e3e652b1, ae4126c7, 89b35cb6,
d50d3346, 87e14abd, and 6037c840 (the dashboard stops cloning the whole
state under the lock every two seconds).

### Out of v1

Contract shape validation, notices linked to claims, glob claims, MCP
resources, and embeddings stay on the roadmap (milestones 3 and 4).

### Release

v1 ships as 1.0.0 through the existing cargo-dist process
([../7-release/01-release-process.md](../7-release/01-release-process.md))
once the tasks above are `done` on the board, the definition-of-done chain
in `AGENTS.md` passes on `main`, and the swarm benchmark numbers are
recorded in [../3-tests/01-testing-strategy.md](../3-tests/01-testing-strategy.md).
Before the release the running daemon is stopped, the two malformed
decision records `ed3e13b9` and `54826acc` are removed from
`.tirith/decisions.jsonl` (their author superseded them with `618e21b8`),
and the daemon is started from the release binary.

## Alternatives

- **Ship 0.2 with the memory tools and call token work "later".** Rejected:
  the measurements show a busy repository exhausts an agent's context in
  one unfiltered `notice_list`, so the memory layer would be unusable at
  the scale it was built for.
- **Define v1 by feature list only, no budgets.** Rejected: without
  numbers and tests, every session re-decides what "small enough" means.
- **Move to a database to fix token cost.** Not related: the tokens are
  spent on the wire, not on disk. ADR-0003 and ADR-0010 stand.

## Consequences

- The roadmap in `01-purpose.md` gains a v1 row pointing here, and the
  task board carries one task per bullet above; a bullet without a task is
  a bug in this ADR.
- Tool schemas and response shapes change in one breaking step before
  1.0.0. `docs/1-about/04-primitives.md`, the README, and the examples
  change in the same commits (AGENTS.md section 7).
- Byte-size tests make some future features more expensive to add. That
  is the point.
