# ADR-0031: A waiting call ends when its caller goes

**Status:** Accepted, 2026-09-17. Refines
[ADR-0028](0028-claim-aware-task-pull.md) (`task_pull` with `wait_secs`)
and the `claim` wait; touches the shim of
[ADR-0006](0006-stdio-shim-starts-daemon.md).

## Context

`claim` and `task_pull` take `wait_secs` (up to 120): the daemon holds the
call until the paths or a task free up. If the agent leaves while the call
waits, the wait can still end in a grant or an assignment, and nobody
receives it. The task stays `in_progress` until the orphan timeout
(1800 s) and the lease until its TTL, and every other agent waits behind
them.

Reading rmcp 3.4, there are three ways a caller goes, and rmcp reports them
unevenly:

- **An MCP cancel** (`notifications/cancelled`) cancels the request
  context's token at once. `task_pull` watched it; `claim` did not.
- **A closed session** (`DELETE`, or the session worker ending) cancels
  the token too, but only after the serve loop's drain of up to 5 s.
- **A dropped HTTP connection** is not reported at all in session mode.
  Every client that sends `initialize` gets a session (the shim, Claude
  Code over HTTP, the CLI), and each request is answered with a
  server-sent event stream that the client could resume, so rmcp keeps the
  handler running. rmcp's disconnect guard exists only for stateless
  requests.

The stdio shim made it worse: it forwarded neither the client's cancels
nor the end of the session, so no wait behind a shim ever stopped early.

## Decision

- **What ends a wait:** a grant or free task, the timeout, a cancel, or the
  caller disconnecting. The last two assign and grant nothing.
- **The daemon ties a hangup to each response.** A middleware on `/mcp`
  (`src/hangup.rs`) adds a `Hangup` to every request and keeps its sender
  in the response body. Hyper drops the body when the client's connection
  closes (it reads EOF on a busy connection, `half_close` off), and rmcp
  ends it when the session closes; either way the hangup fires. The
  handler finds it in `http::request::Parts`, which rmcp puts in the
  request context's extensions. `caller_gone` combines it with the
  context's token; `claim` and `task_pull` stop waiting when it completes.
- **Cancel-safe by construction.** A claim is granted, and a task
  assigned, only in one synchronous attempt under the state lock, never
  across an await, so dropping the wait at any await point leaves no
  partial state. `State::claim_waiting` takes the `gone` future and returns
  `ClaimError::Cancelled`; its one `claim_waited` log row has outcome
  `granted`, `refused`, or `cancelled`. The tools answer status
  `cancelled`.
- **A reply nobody reads skips the piggybacks.** When the caller is gone,
  the handler returns without `finish`, so lost-lease reports and inbox
  messages stay for the agent's next call instead of riding on a dropped
  reply.
- **The shim forwards both signals.** `tirith stdio` sends each call as a
  cancellable request; when its client cancels, it cancels the daemon
  request. Its stdin reader marks end of file, and every in-flight call
  then cancels its daemon request; the shim waits up to 3 s for those
  cancels, closes its daemon session (`DELETE`), and exits. Each forwarded
  cancel is logged to `serve.log`.

## Alternatives

- **A heartbeat** (the client pings during a wait, the daemon stops after a
  missed ping). Rejected: it needs a protocol extension every client would
  have to speak, and the transport already knows when the connection is
  gone.
- **Stateless HTTP mode**, where rmcp cancels a request whose response is
  dropped. Rejected: it requires turning sessions off for every client, or
  the 2026-07-28 protocol, which current clients do not negotiate.
- **Undo a grant after the fact** when the reply cannot be delivered.
  Rejected: another agent may already have read the state, and the log
  would show a grant that never held.
- **Only fix the shim.** Rejected: an agent killed outright never runs the
  shim's exit path, and HTTP clients have no shim.

## Consequences

- A call's result can still be lost if the caller leaves in the moment
  between a grant and the reply; the next `claims_list` or `task_list`
  shows it, and the lease or orphan timeout ends it as before. Waits make
  that window no wider than for any other call.
- A dropped connection ends the wait even if the client meant to resume
  the call's event stream; it gets no grant and has to call again. On
  localhost that does not happen in practice.
- `http-body` becomes a direct dependency (it was already in the tree
  through axum and hyper) for `Frame` and `SizeHint`, which axum does not
  re-export.
- `tests/http_roundtrip.rs` covers a cancel and a dropped connection for
  both tools and a closed session for a claim; `tests/stdio_shim.rs`
  covers a cancel through the shim and a client closing stdin, for both.
