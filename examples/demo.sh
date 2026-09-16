#!/usr/bin/env bash
# Two agents, one directory: a note left for whoever edits it next, an
# overlapping claim refused until the first agent releases, and the claim
# that succeeds carrying everything Bob needs to know about the path.
set -euo pipefail
cd "$(dirname "$0")/.."
# TIRITH_BIN=/path/to/tirith runs the demo against a prebuilt binary (a release).
T="${TIRITH_BIN:-}"; [ -n "$T" ] || { cargo build --quiet && T="$PWD/target/debug/tirith"; }
cd "$(mktemp -d)"
# --no-tray keeps the daemon a single process; SIGINT is its graceful stop,
# and the wait makes sure it has exited before the shell does.
"$T" serve --bind 127.0.0.1:0 --no-tray >/dev/null 2>&1 &
daemon=$!
trap 'kill -INT "$daemon" 2>/dev/null; wait "$daemon" 2>/dev/null || true' EXIT
until [ -f .tirith/runtime/daemon.json ]; do sleep 0.1; done   # the CLI reads the daemon's address from here
# Alice leaves a note about src/auth/ for whoever edits it next.
"$T" memory write --agent alice "Session ids are opaque" -k gotcha --path src/auth/ --tag auth <<'NOTE'
Tokens are opaque strings. Compare them, never parse them.

- [gotcha] the session id format changed twice; parsing it broke the client both times #auth
- [lesson] the shape lives in the contract "POST /api/sessions", not in the code
NOTE

# Alice claims the directory; Bob's overlapping claim is refused with the
# owner, reason, and expiry.
"$T" claim --agent alice --reason "refactor session handling" src/auth/
"$T" claim --agent bob   --reason "fix login redirect"        src/auth/login.rs || true
"$T" claims --agent bob --path src/auth/        # your own claims plus any overlapping the path

# While she holds it, Alice publishes the interface, announces two changes,
# and records a decision, all against src/auth.
"$T" contract publish --agent alice "POST /api/sessions" -k http \
  -s '{"request":{"email":"string","password":"string"},"response":{"token":"string"}}' --consumer src/auth/login.rs
"$T" notice publish --agent alice -k rename    "renamed session_id to token" --from session_id --to token --path src/auth/
"$T" notice publish --agent alice -k signature "login() now returns Result<Session>" --path src/auth/login.rs
"$T" decision record --agent alice "Tokens are opaque" -d "Clients compare tokens, never parse them" --path src/auth/
"$T" release --agent alice

# Bob's claim now succeeds and carries a brief: the newest notices,
# contracts, decisions, and notes for the path, five each at most, with
# `more` counts for the rest. Empty sections are left out.
"$T" claim --agent bob --reason "fix login redirect" src/auth/login.rs

# The brief delivered both notices (delivery is the acknowledgement,
# ADR-0021), so an --unread listing would be empty. Lists are bounded:
# Bob asks for one row and gets a cursor for the rest.
"$T" notice list --agent bob --path src/auth/ --limit 1

# Anyone can find the note by text and read it in full.
"$T" memory search --agent bob "session id"
"$T" memory read   --agent bob session-ids-are-opaque
