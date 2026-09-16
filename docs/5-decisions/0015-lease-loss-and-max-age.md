# ADR-0015: An agent is told when it loses a lease, and activity alone cannot hold one forever

**Status:** Accepted, 2026-09-16. Refines ADR-0004, which introduced TTL
leases renewed by any call. Implemented by task 44ee9c43.

## Context

Two behaviours from ADR-0004 are fine for a demo and wrong for ten agents
on one repository, as the 2026-09-16 code audit showed.

1. A lease that expires is reaped silently. The former owner gets no
   signal in any later response. If Alice's lease on a file lapses while
   she is still editing, Bob can claim the file, rewrite it, and Alice's
   next save wins. The claim system exists to prevent exactly this.
2. Every call by an agent renews every lease that agent holds. Leases were
   meant to expire when an agent crashes; but an agent that keeps polling
   `notice_list` while doing nothing on `src/` keeps `src/` for as long as
   the session lives, and nine other agents starve without any expiry in
   sight.

## Decision

1. **Lost leases are reported.** `State` remembers, per agent, the paths
   reaped from that agent since its last call. The next response of any
   tool to that agent carries `lost: [{path, at, now_held_by?}]`, and the
   result's text line starts with `warning: lost lease`. The list is
   cleared once delivered. An agent that sees `lost` must stop editing
   those paths until it claims them again.
2. **A takeover is flagged to the new owner.** When a claim succeeds on a
   path reaped from another agent less than one TTL ago, the response
   carries `previous_owner` and `reaped_at`, so the new owner knows the
   file may be half-edited and reads it before writing.
3. **Leases have a maximum age.** Activity still renews a lease, but a
   lease cannot live longer than `max_age = min(4 × ttl, 4 h)` from its
   `claimed_at` unless the agent calls `claim` or `renew` naming the path.
   Those two calls reset `claimed_at`. Any other call only extends the
   expiry within the age window. After the window, the lease expires like
   any other and the agent sees it in `lost`.
4. **Ancestor claims absorb child claims.** Claiming a directory while
   holding a file inside it folds the file into the directory claim, so
   there is one claim to release, not two.
5. **Corrupt lease data cannot panic.** TTLs read from `claims.json` are
   clamped to the valid range on load.

## Alternatives

- **Only `claim` and `renew` renew leases.** Simpler, but agents would
  lose leases during ordinary long work unless they remember to renew,
  which is the failure ADR-0004 avoided. The age cap keeps the
  convenience and bounds the abuse.
- **Fencing tokens on every edit.** Tirith does not sit between the agent
  and the filesystem (see "What Tirith is not"), so it cannot reject a
  stale write; it can only tell the agent. `lost` is the honest version.
- **Push notifications to the former owner.** MCP clients used here do not
  reliably surface server notifications; piggybacking on the next
  response works with every client.

## Consequences

- Every tool response may carry `lost`; the byte-size tests in ADR-0013
  count it (it is bounded by the number of paths the agent held).
- `docs/1-about/04-primitives.md` (claim section) and the lease paragraph
  in `AGENTS.md` describe `lost`, `previous_owner`, and the age cap.
- Long sessions must re-`claim` or `renew` roughly every four TTLs; the
  dogfooding protocol says so.
