# ADR-0021: Notices are seen when delivered; the seen log is runtime-only

**Status:** Accepted, 2026-09-16. Refines ADR-0003, ADR-0010 and ADR-0014
for notices. Implemented by task 66f16442; decision 3f6d77d9 on the board.

## Context

A notice row in `notices.jsonl` carried an `acked_by` list, and a
`notice_ack` tool let an agent add itself to it. Two things were wrong
with that. Every ack was an in-place edit that rewrote the whole log
(ADR-0010's `mark_rewrite`), an 8 MB write per ack on a busy repository.
And nobody acked: agents received notices through the brief on claim
(ADR-0014), acted on them, and never made the second call, so the
dashboard's "unacknowledged" count only ever grew and meant nothing.

## Decision

1. **Delivery is the acknowledgement.** A notice is seen by an agent the
   moment Tirith delivers it to that agent, in a brief on claim or in a
   `notice_list` with `unread: true`. `unread` means "never delivered to
   this agent". The `notice_ack` tool, the CLI subcommand and the
   dashboard's unacknowledged tile are removed.
2. **Notice rows are immutable once published.** `notices.jsonl` is
   append-only; nothing rewrites it except the full rewrite that follows
   a persist failure.
3. **Seen marks are a separate log.** `NoticeBoard` keeps a
   `Vec<NoticeSeen>` of `{notice_id, agent, at}`; `mark_seen` appends one
   entry the first time a notice reaches an agent and nothing on a repeat.
   The persister drains it through a `LogCursor` like the other logs, to
   `.tirith/runtime/notice_seen.jsonl`.
4. **Seen marks are runtime state, not committed.** They are per-agent
   session facts; committing them would add merge noise for every
   contributor and no knowledge. A fresh clone sees every notice as
   unread, which is correct for a new machine.
5. **The in-memory shape is unchanged.** On load the board replays the
   seen log into each notice's `acked_by` (the field keeps its historical
   name on the wire), so unread filtering, brief paging and the dashboard
   need no new code path. Rows on disk that still carry an `acked_by`
   from before this ADR keep it; it is merged on load and never written
   again.

## Alternatives

- **Enforce the ack** by refusing a claim while undelivered notices are
  unacked. Rejected: one more mandatory round trip per edit, the cost
  brief exists to remove.
- **Keep ack optional and hide the tile.** Rejected: dead surface, and
  one more tool schema in every session.
- **Commit the seen log next to the notices.** Rejected: not repository
  knowledge, and a team of five would conflict on the file daily.
- **A per-agent cursor ("everything before seq N is read").** Rejected:
  deliveries are path-scoped, a cursor is not; claiming `src/a` must not
  consume notices about `src/b` (same reasoning as ADR-0014).

## Consequences

- One fewer tool; `tools/list` shrinks. Delivery costs one appended line.
- `.tirith/runtime/` gains `notice_seen.jsonl`, gitignored with the rest
  of the runtime directory; brief's volatile delivery map for notices is
  replaced by it, so paging through notices survives a restart.
- The dashboard's notices table shows who has seen each notice and no
  longer warns about a global count.
