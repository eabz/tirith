# Example: two agents claim overlapping files

This is milestone 1's acceptance test. It lives in `examples/demo.sh` and is
short enough to read in one glance. Run it with:

```bash
./examples/demo.sh
```

It builds the binary, starts a daemon on a free port inside a temporary
directory, waits for `.tirith/runtime/daemon.json` (which is how the CLI
finds the daemon), and runs:

```bash
tirith claim --agent alice --reason "refactor session handling" src/auth/
tirith claim --agent bob   --reason "fix login redirect"        src/auth/login.rs || true
tirith claims
tirith release --agent alice
tirith claim --agent bob   --reason "fix login redirect"        src/auth/login.rs
```

Output from a real run:

```
ok       alice  src/auth  expires 00:16:23Z
conflict src/auth/login.rs overlaps src/auth (alice: "refactor session handling", expires 00:16:23Z)
alice    src/auth                     refactor session handling        expires 00:16:23Z
released alice  src/auth
ok       bob  src/auth/login.rs  expires 00:16:23Z
```

The trailing slash on `src/auth/` is accepted and normalized away; a
claimed path always covers everything beneath it.

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
