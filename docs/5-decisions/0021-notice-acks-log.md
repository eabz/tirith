# ADR-0021: Acknowledgements are their own append-only runtime log

**Status:** Accepted, 2026-09-16. Refines ADR-0003 and ADR-0010 for notices.
Implemented by task 66f16442.

## Context

A notice row in `notices.jsonl` carried its own `acked_by` list, so every
`notice_ack` was an in-place edit that rewrote the whole log (ADR-0010's
`mark_rewrite`). At the sizes a busy repository reaches, that is an 8 MB
rewrite per acknowledgement, and `acked_by` grew by one entry per agent
per notice, fattening every row on the wire. Brief on claim (ADR-0014)
deliberately kept its delivery marks in memory to avoid the same cost, so
acking was the last in-place edit on a committed log.

## Decision

1. **Notice rows are immutable once published.** `notices.jsonl` is
   append-only again; nothing rewrites it after a publish except the
   full rewrite that follows a persist failure.
2. **Acks are a separate log.** `NoticeBoard` keeps a `Vec<NoticeAck>`
   of `{notice_id, agent, at}`; `ack_at` appends one entry the first time
   an agent acknowledges a notice and nothing on a repeat. The persister
   drains it through a `LogCursor` like the other logs, to
   `.tirith/runtime/notice_acks.jsonl`.
3. **Acks are runtime state, not committed.** They are per-agent session
   facts ("this session handled that change"); committing them would add
   merge noise for every contributor and no knowledge. A fresh clone sees
   every notice as unread, which is correct for a new machine.
4. **The in-memory shape is unchanged.** On load the board replays the
   ack log into each notice's `acked_by`, so `unread_by`, the compact list
   rows, brief delivery and the dashboard keep working with no change.
   Rows on disk that still carry an `acked_by` from before this ADR keep
   it; it is merged on load and never written again.

## Alternatives

- **Keep acks in the row and debounce the rewrite.** Still rewrites the
  whole file, still grows every row.
- **Commit the ack log next to the notices.** Rejected: acks are not
  repository knowledge, and a team of five would conflict on the file
  daily.
- **Per-agent cursor ("everything before seq N is read").** Rejected:
  acks are path-scoped, a cursor is not; claiming `src/a` must not consume
  notices about `src/b` (same reasoning as ADR-0014).

## Consequences

- `notice_ack` costs one appended line. The `acked_by` field stays on
  the wire for `notice_list` full rows and the dashboard.
- `.tirith/runtime/` gains `notice_acks.jsonl`, gitignored with the rest
  of the runtime directory.
- Follow-ups noted on task 66f16442: brief delivery could be persisted
  in the same log as a `seen` kind, and the dashboard could show "seen by
  n of m consumers" per notice instead of a global unacknowledged count.
