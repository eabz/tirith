# ADR-0014: One `claim` response replaces the four reads before every edit

**Status:** Accepted, 2026-09-16. Refined by
[ADR-0021](0021-notice-acks-log.md): delivery in a brief is now the
acknowledgement, seen marks persist in `.tirith/runtime/notice_seen.jsonl`,
and `notice_ack` no longer exists. The "delivery is not an
acknowledgement" paragraph below describes the state before that ADR.

## Context

The dogfooding protocol tells every agent to run `notice_list`,
`contract_list`, `decision_list` and, since [ADR-0011](0011-memory-primitive.md),
`memory_search` for a path before claiming and editing it. Each of those is
a separate MCP round trip, and in an agent loop a round trip is not a cheap
HTTP call: the client re-sends the whole conversation to the model, and the
result is appended to that conversation and re-read on every later turn.

Measured on 2026-09-16 with a byte-counting copy of the swarm benchmark
(200 simulated agents, 3 s think time, list calls hitting seeded data):

- 7.0 calls per claim → release cycle; 336 response tokens per call;
  about 2,350 shipped tokens per cycle with tiny synthetic bodies.
- Against this repository's real state the same cycle is about 13,000
  tokens, because `contract_list` alone was 7,869 tokens for two contracts
  and `notice_list` 3,530.
- Inside one list row, JSON keys take ~40 %, the UUID ~20 %, the
  timestamp ~15 %; the content the agent wanted is ~25 %.

The daemon itself is not the constraint. [ADR-0010](0010-incremental-persistence.md)
took it to ~20,000 calls/s; no token budget can feed a hundredth of that.
The cost is the number of mandatory round trips and the shape of what they
return. Tasks `e3e652b1`, `89b35cb6`, `ae4126c7` and `d50d3346` cut bytes.
This ADR cuts round trips.

The memory primitive already attaches up to five path-scoped notes to every
ok `claim` response (contract "Memory primitive" v3), for the reason given in
ADR-0011: Tirith knows what an agent is about to edit, so knowledge can
arrive at claim time instead of being searched for. Notices, contracts and
decisions are the same kind of knowledge and deserve the same delivery.

## Decision

`claim` gains one boolean input, `brief`, default `true`. When on, the ok
response carries, **for the claimed paths only**, four capped sections next
to the existing `claim_id` / `new_paths` / `renewed_paths` / `expires_at`:

| section | rows | row shape |
|---|---|---|
| `notices` | newest 5 unread and not yet delivered | `{id (8 chars), kind, summary ≤ 160, by}` |
| `contracts` | newest 5 consumed by the paths | `{name, version, kind}` |
| `decisions` | newest 5 affecting the paths | `{id (8 chars), title}` |
| `memory` | newest 5 (unchanged from "Memory primitive" v3) | `{permalink, title, kind, paths, updated_at, excerpt ≤ 160}` |

plus `more: {notices, contracts, decisions, memory}`, the counts of matching
rows that were not shown, so the agent knows whether to page with the list
tools. A section with no rows is omitted, not sent as `[]`. `more` is always
present. `brief: false` returns today's bare claim outcome. Conflict
responses do not change.

The serialized ok response is **hard-capped at 4,096 bytes**. If a rendered
brief exceeds it, rows are dropped oldest-first from the largest section and
that section's `more` count is bumped, until it fits. An integration test in
`tests/http_roundtrip.rs` holds the line with maximum-length rows.

Notices shown in a brief are marked **delivered** to that agent so a second
brief on the same paths does not repeat them. Delivery is not an
acknowledgement: `acked_by` is untouched, `notice_ack` stays explicit, and
`notice_list` with `unread: true` still lists delivered-but-unacked notices.
Delivered marks live in `State` for the daemon's lifetime and are never
written to `notices.jsonl`; rewriting the log on every claim would undo
ADR-0010, and the worst a restart can do is brief a notice one extra time.

Rows use the compact conventions task `89b35cb6` is standardising for the
list tools: 8-character id prefixes, no `null` fields, no empty arrays.
Every tool that takes an id accepts a unique prefix; an ambiguous prefix is
`invalid`. The contract is "Brief on claim" v1.

The protocol in `AGENTS.md` and
[`docs/6-agent-workflow/03-tirith-dogfooding.md`](../6-agent-workflow/03-tirith-dogfooding.md)
becomes: **`claim` → edit → `notice_publish` on breaking changes →
`release`.** The four list tools remain for paging when `more` says there is
more, and for paths the agent is not claiming. The server `INSTRUCTIONS`
string is shortened to match.

## Alternatives

- **A separate `brief` tool taking paths.** Rejected: one more schema in
  `tools/list`, paid by every session forever (task `ae4126c7` is spending
  effort to shrink that list), and one more call agents would forget. The
  claim is the moment the paths are known and the moment the knowledge is
  needed.
- **`brief` opt-in (default `false`).** Rejected: an opt-in flag is exactly
  the step agents skip, and the point is to remove steps. The flag exists
  so renew-style re-claims and clients that render claims themselves can
  turn it off.
- **Full bodies in the brief.** Rejected: contract bodies are multi-KB and
  unbounded in history; the brief tells the agent what exists and it fetches
  what it needs. This is the same rule the memory row follows.
- **Persisting delivered marks in `notices.jsonl`.** Rejected: an in-place
  edit forces a rewrite of the whole log on every claim with unread notices,
  which at 14,000 notices is an 8 MB write per claim. Ack already pays that
  cost, and ack is rare; delivery is not.
- **An append-only `.tirith/runtime/notice_acks.jsonl` or a per-agent
  "newest delivered" cursor.** Not needed for delivery: a mark that only
  prevents a repeat within one daemon lifetime needs no durability at all,
  so it is a `HashMap<AgentId, HashSet<NoticeId>>` on `State` and costs no
  write. A per-agent cursor was also rejected because briefs are
  path-scoped: an agent that claimed `src/a` must still be briefed on older
  notices about `src/b` later. Moving `acked_by` out of the notice rows into
  an append-only log so that `notice_ack` stops rewriting the whole log is
  worth doing, but it is the ack path, not this ADR; it is tracked as its
  own board task.
- **Attaching the brief to `task_pull` as well.** Deferred: the shape is
  designed once here so a follow-up can reuse it for a pulled task's paths.
- **A line-oriented text format instead of JSON.** Deferred: the byte cuts
  in `89b35cb6` and the round-trip cut here come first; if list rows are
  still the dominant cost afterwards, a compact text rendering is the next
  ADR.

## Consequences

- A work cycle goes from 7 round trips to 4 (claim, publish, update,
  release); the reads that grew with repository history are gone from the
  hot path. Against this repository's current state the per-cycle cost
  drops from roughly 13,000 tokens to under 2,000 before the byte-level
  tasks land.
- The ok `claim` response can now be up to 4 KB instead of ~160 bytes.
  That is a fixed bound, independent of how many notices or contracts the
  repository accumulates.
- `src/client.rs` and the `tirith claim` CLI renderer must tolerate and
  render the new optional sections.
- The claim path does four more overlap scans under the `State` lock; each
  is the same linear scan the list tools already do, now bounded by
  the 5-row caps instead of returning everything.
- Implementation order on `src/server.rs` and `src/state.rs` is agreed with
  the other sessions: memory-layer, ci-speedup (`e3e652b1`, `ae4126c7`),
  storage-claude (`89b35cb6` wiring), then this ADR.
