#!/usr/bin/env bash
# Two agents claim overlapping files; the second is refused until the first releases.
set -euo pipefail
cd "$(dirname "$0")/.." && cargo build --quiet && T="$PWD/target/debug/tirith"
cd "$(mktemp -d)"
"$T" serve --bind 127.0.0.1:0 >/dev/null 2>&1 & trap 'kill $!' EXIT
until [ -f .tirith/runtime/daemon.json ]; do sleep 0.1; done   # the CLI reads the daemon's address from here
"$T" claim --agent alice --reason "refactor session handling" src/auth/
"$T" claim --agent bob   --reason "fix login redirect"        src/auth/login.rs || true
"$T" claims
"$T" release --agent alice
"$T" claim --agent bob   --reason "fix login redirect"        src/auth/login.rs
