# Example: two agents claim overlapping files

This is milestone 1's acceptance test. It lives in `examples/demo.sh` and is
short enough to read in one glance.

**Status: In progress.** The commands below are the target; run
`examples/demo.sh` for the version that matches the current build.

```bash
#!/usr/bin/env bash
set -euo pipefail
tirith serve --bind 127.0.0.1:7477 &
trap 'kill $!' EXIT
sleep 0.3
tirith claim --agent alice --reason "refactor session handling" src/auth/
tirith claim --agent bob   --reason "fix login redirect"        src/auth/login.rs || true
tirith claims list
tirith release --agent alice
tirith claim --agent bob   --reason "fix login redirect"        src/auth/login.rs
```

Expected output:

```
ok       alice  src/auth/            expires 18:20:00Z
conflict src/auth/login.rs overlaps src/auth/ (alice: "refactor session handling", expires 18:20:00Z)
alice    src/auth/            refactor session handling   expires 18:20:00Z
released alice  src/auth/
ok       bob    src/auth/login.rs    expires 18:20:05Z
```

## What each line proves

1. The daemon starts and answers on localhost.
2. A directory claim is accepted.
3. A file inside a claimed directory is refused, and the refusal names the
   owner, their reason, and when the lease ends.
4. Listing shows live claims only.
5. Releasing frees the path.
6. The previously refused claim now succeeds.

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
                   "overlaps": "src/auth/",
                   "owner": "alice",
                   "reason": "refactor session handling",
                   "expires_at": "2026-09-15T18:20:00Z" } ] }
```
