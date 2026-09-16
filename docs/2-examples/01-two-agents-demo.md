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
tirith release --agent alice
tirith claim --agent bob   --reason "fix login redirect"        src/auth/login.rs

tirith notice publish --agent alice -k rename    "renamed session_id to token" --from session_id --to token --path src/auth/
tirith notice publish --agent alice -k signature "login() now returns Result<Session>" --path src/auth/login.rs
tirith notice list --agent bob --path src/auth/ --unread --limit 1

tirith memory search --agent bob "session id"
tirith memory read   --agent bob session-ids-are-opaque
```

Output from a real run:

```
created  session-ids-are-opaque gotcha   02:35:10Z  paths src/auth  Session ids are opaque: Tokens are opaque strings. Compare them, never parse them.
ok       alice  src/auth  expires 02:45:10Z
memory:
  session-ids-are-opaque gotcha   02:35:10Z  paths src/auth  Session ids are opaque: Tokens are opaque strings. Compare them, never parse them. - [gotcha] the session id format changed twice; parsing it broke the client both times #auth - [lesso...
conflict src/auth/login.rs overlaps src/auth (alice: "refactor session handling", expires 02:45:10Z)
alice    src/auth                     refactor session handling        expires 02:45:10Z
released alice  src/auth
ok       bob  src/auth/login.rs  expires 02:45:10Z
memory:
  session-ids-are-opaque gotcha   02:35:10Z  paths src/auth  Session ids are opaque: Tokens are opaque strings. Compare them, never parse them. - [gotcha] the session id format changed twice; parsing it broke the client both times #auth - [lesso...
d05b4a97 rename    renamed session_id to token                      affects src/auth  by alice
473aa8f6 signature login() now returns Result<Session>              affects src/auth/login.rs  by alice
473aa8f6 signature login() now returns Result<Session>              affects src/auth/login.rs  by alice
…1 more, pass --before 2026-09-16T02:35:10.726555Z|473aa8f6-2afd-41d3-84af-b65bb8962562
session-ids-are-opaque gotcha   02:35:10Z  paths src/auth  Session ids are opaque: Tokens are opaque strings. Compare them, never parse them. score 27
session-ids-are-opaque gotcha   02:35:10Z  paths src/auth  Session ids are opaque: Tokens are opaque strings. Compare them, never parse them.
tags: auth
by alice at 02:35:10Z  (created by alice at 02:35:10Z)

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
   the agent about to edit sees it without searching. A file inside the
   directory gets the same note, because notes follow the claim overlap
   rule.
3. A file inside a claimed directory is refused, and the refusal names the
   owner, their reason, and when the lease ends.
4. `claims` shows your own claims plus any overlapping the path you ask
   about; the whole board needs `--all`. Nothing an agent did not ask for
   lands in its context.
5. Releasing frees the path, and the previously refused claim succeeds.
6. Notices are published against paths and read back per path. Every list
   is bounded (default 20 rows, here 1): the footer says how many more
   there are and gives the `--before` cursor for the next page.
7. Search finds the note by text with a score; `memory read` returns the
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

And the successful claim, with the note that rides along:

```json
{ "status": "ok", "claim_id": "…", "new_paths": ["src/auth/login.rs"],
  "renewed_paths": [], "expires_at": "2026-09-15T18:30:00Z",
  "memory": [ { "permalink": "session-ids-are-opaque",
                "title": "Session ids are opaque", "kind": "gotcha",
                "paths": ["src/auth"], "updated_at": "2026-09-15T18:20:00Z",
                "excerpt": "Tokens are opaque strings. Compare them, never parse them." } ] }
```

At most five notes ride along, newest first, as excerpts of about 160
characters; a claim on a path nobody wrote about carries an empty array.
