# ADR-0020: Agent-to-agent messages, delivered on the next call

**Status:** Accepted, 2026-09-16

## Context

Coordinating a day of parallel sessions on this repository showed that
agents need a small, direct channel: "take task X", "I released
server.rs", "approved". Today that talk runs through the Claude desktop
app's session messaging, which only Claude sessions can use. Codex,
Cursor, and plain scripts sit on the same Tirith daemon and cannot hear
it. The daemon already has a delivery path that reaches every agent
without polling: the result of its next tool call, which carries `lost`
(ADR-0015) and the brief (ADR-0014).

## Decision

A sixth primitive, `messages`, runtime-only.

- **Tools.** `message_send {agent, to, text, reply_to?, paths?}` where `to`
  is an agent name or `*`; text is at most 1000 characters; returns the
  message. `message_list {agent, with?, since?, unread?, limit?, before?}`
  lists the caller's own conversations, paged and compact like every other
  list, newest first.
- **Delivery is a piggyback.** The recipient's next tool result, whatever
  the tool, carries `inbox`: the newest five undelivered messages as
  `{id, from, text (cut to 200 chars), at}` plus `inbox_more`, the count
  still waiting; the text line gains `inbox: n`. Delivered marks live in
  memory for the daemon's lifetime, like the brief's; a restart redelivers
  what is still within retention, which is the safe side. Nothing is
  attached when nothing is waiting, so the common call costs nothing.
- **Broadcasts.** `*` reaches every agent seen in the hour before the
  send, minus the sender; the audience is fixed at send time and stored
  with the message, so a listing shows who was meant to hear it.
- **Storage.** Append-only `.tirith/runtime/messages.jsonl`, gitignored
  like the rest of `runtime/`: conversation traffic is not repository
  knowledge. Messages older than 24 hours are dropped when the daemon
  loads the file.
- **Humans can talk too.** `tirith message send|list` in the CLI, so a
  developer can answer an agent from a terminal.

## Alternatives

- **Keep using the client's own messaging.** Rejected; it excludes every
  non-Claude client, which is the reason Tirith speaks MCP.
- **A dedicated `message_receive` tool.** Rejected; agents do not poll,
  and the next-result piggyback already exists and is proven by `lost`.
- **Commit the messages next to notices and decisions.** Rejected; a
  notice is a fact about the code, a message is a conversation. Anything
  worth keeping becomes a notice, decision, or memory note.
- **A `more` object as on the brief.** Rejected for the inbox; `more` on a
  claim is a per-section object, and one flat `inbox_more` count avoids
  two shapes under one key.

## Consequences

- Two more tools in `tools/list`; their descriptions are one short
  sentence each and the size test's bound is raised only by what their
  schemas cost, per ADR-0017.
- `finish` in `server.rs` gains a third piggyback next to `persist_error`
  and `lost`, on the same path.
- The dogfooding protocol tells agents to use `message_send` for
  coordination talk instead of their client's messaging.
- The dashboard may show the last messages later; it is not part of this
  change.
