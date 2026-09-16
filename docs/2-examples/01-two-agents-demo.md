# Example: two agents, one directory, and a note between them

This is the acceptance demo. It lives in `examples/demo.sh` and is short
enough to read in one glance. Run it with:

```bash
./examples/demo.sh
```

It builds the binary, starts a daemon on a free port inside a temporary
directory, waits for `.tirith/runtime/daemon.json` (which is how the CLI
finds the daemon), and runs:

```bash
tirith memory write --agent alice "Session ids are opaque" -k gotcha --path src/auth/ --tag auth <<'NOTE'
Tokens are opaque strings. Compare them, never parse them.

- [gotcha] the session id format changed twice; parsing it broke the client both times #auth
- [lesson] the shape lives in the contract "POST /api/sessions", not in the code
NOTE

tirith claim --agent alice --reason "refactor session handling" src/auth/
tirith claim --agent bob   --reason "fix login redirect"        src/auth/login.rs || true
tirith claims --agent bob --path src/auth/

tirith contract publish --agent alice "POST /api/sessions" -k http \
  -s '{"request":{"email":"string","password":"string"},"response":{"token":"string"}}' --consumer src/auth/login.rs
tirith notice publish --agent alice -k rename    "renamed session_id to token" --from session_id --to token --path src/auth/
tirith notice publish --agent alice -k signature "login() now returns Result<Session>" --path src/auth/login.rs
tirith decision record --agent alice "Tokens are opaque" -d "Clients compare tokens, never parse them" --path src/auth/
tirith release --agent alice

tirith claim --agent bob --reason "fix login redirect" src/auth/login.rs

tirith notice list --agent bob --path src/auth/ --unread --limit 1

tirith memory search --agent bob "session id"
tirith memory read   --agent bob session-ids-are-opaque
```

Output from a real run:

```
created  session-ids-are-opaque gotcha   03:30:02Z  paths src/auth  Session ids are opaque: Tokens are opaque strings. Compare them, never parse them.
ok       alice  src/auth  expires 03:40:03Z
memory:
  session-ids-are-opaque gotcha   03:30:02Z  paths src/auth  Session ids are opaque: Tokens are opaque strings. Compare them, never parse them. - [gotcha] the session id format changed twice; parsing it broke the client both times #auth - [lesso...
conflict src/auth/login.rs overlaps src/auth (alice: "refactor session handling", expires 03:40:03Z)
alice    src/auth                     refactor session handling        expires 03:40:03Z
89c7ba21 POST /api/sessions               v1 http      consumers src/auth/login.rs
be128ea5 rename    renamed session_id to token                      affects src/auth  by alice
781727ec signature login() now returns Result<Session>              affects src/auth/login.rs  by alice
1a61c1ae Tokens are opaque: Clients compare tokens, never parse them
released alice  src/auth
ok       bob  src/auth/login.rs  expires 03:40:03Z
notices:
  781727ec signature login() now returns Result<Session>  by alice
  be128ea5 rename    renamed session_id to token  by alice
contracts:
  POST /api/sessions  v1 http
decisions:
  1a61c1ae Tokens are opaque
memory:
  session-ids-are-opaque gotcha   03:30:02Z  paths src/auth  Session ids are opaque: Tokens are opaque strings. Compare them, never parse them. - [gotcha] the session id format changed twice; parsing it broke the client both times #auth - [lesso...
781727ec signature login() now returns Result<Session>              affects src/auth/login.rs  by alice
…1 more, pass --before 2026-09-16T03:30:03.353167Z|781727ec-7260-4140-a002-18141a5d3220
session-ids-are-opaque gotcha   03:30:02Z  paths src/auth  Session ids are opaque: Tokens are opaque strings. Compare them, never parse them. - [gotcha] the session id format changed twice; parsing it broke the client both times #auth - [lesso... score 27
session-ids-are-opaque gotcha   03:30:02Z  paths src/auth  Session ids are opaque: Tokens are opaque strings. Compare them, never parse them.
tags: auth
by alice at 03:30:02Z  (created by alice at 03:30:02Z)

Tokens are opaque strings. Compare them, never parse them.

- [gotcha] the session id format changed twice; parsing it broke the client both times #auth
- [lesson] the shape lives in the contract "POST /api/sessions", not in the code
```

The note body comes from stdin, so multi-line Markdown needs no shell
quoting; `--body` and `--file` work too. The trailing slash on `src/auth/`
is accepted and normalized away; a claimed path always covers everything
beneath it.

## What each line proves

1. A note is written once, scoped to `src/auth`, and gets a permalink
   derived from its title.
2. A claim on the directory carries the note back as an excerpt row, so
   the agent about to edit sees it without searching.
3. A file inside a claimed directory is refused, and the refusal names the
   owner, their reason, and when the lease ends.
4. `claims` shows your own claims plus any overlapping the path you ask
   about; the whole board needs `--all`.
5. While Alice holds the directory she publishes the interface, two change
   notices, and a decision, all scoped to `src/auth`. Releasing frees the
   path.
6. Bob's claim now succeeds and carries a brief: the newest notices,
   contracts, decisions, and notes for the path, at most five per section,
   as compact rows with 8-character ids, plus `more` counts for what was
   left out. Sections that would be empty are omitted, and the whole brief
   is capped at 4 KB. This is the raw tool result an agent sees; the four
   reads the protocol used to require before an edit are now one call.
7. Every list is bounded (default 20 rows, here 1): the footer says how
   many more there are and gives the `--before` cursor for the next page.
8. Search finds the note by text with a score; `memory read` returns the
   whole body, which no list ever does.

## The same flow over MCP

The CLI uses the same MCP tools an agent would. Here is the refused call as
a `tools/call` request and response:

```json
{ "jsonrpc": "2.0", "id": 2, "method": "tools/call",
  "params": { "name": "claim",
              "arguments": { "agent": "bob",
                             "paths": ["src/auth/login.rs"],
                             "reason": "fix login redirect" } } }
```

```json
{ "status": "conflict",
  "conflicts": [ { "path": "src/auth/login.rs",
                   "overlaps": "src/auth",
                   "owner": "alice",
                   "reason": "refactor session handling",
                   "expires_at": "2026-09-15T18:20:00Z" } ] }
```

The successful claim with its brief is the JSON block in the transcript
above: at most five rows per section, newest first, excerpts of about 160
characters for notes, never a body; a claim on a path nobody wrote about
carries no sections at all. Pass `brief: false` to skip it.
