#!/usr/bin/env bash
# Two agents, one directory: a note left for whoever edits it next, an
# overlapping claim refused until the first agent releases, and the note
# found again by search.
set -euo pipefail
cd "$(dirname "$0")/.." && cargo build --quiet && T="$PWD/target/debug/tirith"
cd "$(mktemp -d)"
"$T" serve --bind 127.0.0.1:0 >/dev/null 2>&1 & trap 'kill $!' EXIT
until [ -f .tirith/runtime/daemon.json ]; do sleep 0.1; done   # the CLI reads the daemon's address from here

# Alice leaves a note about src/auth/ for whoever edits it next.
"$T" memory write --agent alice "Session ids are opaque" -k gotcha --path src/auth/ --tag auth <<'NOTE'
Tokens are opaque strings. Compare them, never parse them.

- [gotcha] the session id format changed twice; parsing it broke the client both times #auth
- [lesson] the shape lives in the contract "POST /api/sessions", not in the code
NOTE

# A claim on the directory carries that note back. Bob's overlapping claim
# is refused with the owner, reason, and expiry, until Alice releases.
"$T" claim --agent alice --reason "refactor session handling" src/auth/
"$T" claim --agent bob   --reason "fix login redirect"        src/auth/login.rs || true
"$T" claims --agent bob --path src/auth/        # lists your own claims plus any overlapping the path
"$T" release --agent alice
"$T" claim --agent bob   --reason "fix login redirect"        src/auth/login.rs

# Alice announces two changes that touch src/auth. Lists are bounded: Bob
# asks for one row and gets a cursor for the rest.
"$T" notice publish --agent alice -k rename    "renamed session_id to token" --from session_id --to token --path src/auth/
"$T" notice publish --agent alice -k signature "login() now returns Result<Session>" --path src/auth/login.rs
"$T" notice list --agent bob --path src/auth/ --unread --limit 1

# Anyone can find the note by text and read it in full.
"$T" memory search --agent bob "session id"
"$T" memory read   --agent bob session-ids-are-opaque
