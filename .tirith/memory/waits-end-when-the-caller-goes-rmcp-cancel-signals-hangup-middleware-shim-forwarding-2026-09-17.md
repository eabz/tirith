---
id: 5b6dcd96-d435-498a-b200-c67246ad3466
permalink: waits-end-when-the-caller-goes-rmcp-cancel-signals-hangup-middleware-shim-forwarding-2026-09-17
title: "Waits end when the caller goes: rmcp cancel signals, hangup middleware, shim forwarding (2026-09-17)"
kind: gotcha
tags:
- wait_secs
- cancel
- rmcp
- stdio
- adr-0031
paths:
- src/hangup.rs
- src/stdio.rs
- src/server.rs
- src/state.rs
- src/lead.rs
- tests/http_roundtrip.rs
- tests/stdio_shim.rs
- docs/5-decisions/0031-waits-end-when-the-caller-goes.md
author: shim-cancel
updated_by: shim-cancel
created_at: 2026-09-17T15:00:17Z
updated_at: 2026-09-17T15:00:17Z
---

Task 82f613bc, ADR-0031.

- [fact] rmcp 3.4 signals, read from source: MCP `notifications/cancelled` cancels `RequestContext.ct` at once (service.rs local_ct_pool). A closed session (DELETE, worker end) cancels ct only after the serve loop's drain (5 s for Closed). A dropped HTTP connection gives NO signal in session mode; every client that sends `initialize` (shim, CLI client, Claude Code over HTTP) is session mode, and the POST reply is an SSE stream from `LocalSessionManager::create_stream`. rmcp's disconnect guard (#857) is only on the stateless path. `json_response(true)` only affects stateless requests.
- [fact] Hyper 1.x drops a pending response body when the client closes the connection (`mid_message_detect_eof`, half_close off). `src/hangup.rs::layer` (axum `from_fn`, applied via `any_service(mcp).layer(..)` before `nest_service("/mcp")`) inserts a `Hangup` (watch receiver) into the HTTP request and keeps the sender in a `Tied` body wrapper; the sender is dropped at body end or drop. Handlers read it from `context.extensions.get::<http::request::Parts>()` then `parts.extensions`. `caller_gone(&context)` = ct OR hangup; `is_caller_gone` is the sync check.
- [gotcha] axum does not re-export `http_body::Frame`/`SizeHint`, so `http-body` is a direct dep. Forward `size_hint` in the wrapper or JSON replies lose Content-Length.
- [gotcha] A cancelled/disconnected call must skip `TirithServer::finish`, which consumes `lost` and inbox piggybacks; `unheard(outcome)` does that. Only claim and task_pull check it.
- [fact] `State::claim_waiting(.., wait, gone)` selects `biased` on `gone` first; the loop sets `&mut conflicted` so the single `claim_waited` row (outcome granted|refused|cancelled via `lead::WaitOutcome`) is written even when cancelled. Already-gone caller: no attempt, no row.
- [fact] Shim (`src/stdio.rs`): `Proxy` holds `Peer<RoleClient>`; `call_tool` uses `send_cancellable_request` and selects on `handle.rx`, `context.ct`, and a watch flag set by `EofWatch` (stdin reader returning 0 bytes). On cancel it calls `handle.cancel(reason)` and logs `[tirith stdio] cancelled request N at the daemon: <reason>` to serve.log. After `waiting()`, `run()` waits up to 3 s for `_calls` (mpsc sender dropped with the last Arc<Proxy>), then `daemon.close_with_timeout` (sends DELETE). rmcp drops the reply for a cancelled request, so test `rpc()` must not expect a line for it.
- [method] Tests: http_roundtrip `a_waiting_call_whose_connection_drops_or_session_closes_takes_nothing` uses raw TcpStream POSTs with Mcp-Session-Id, drops them, and polls `/api/lead?event=claim_waited` for outcome cancelled before freeing paths (deterministic). Removing the layer makes it fail. stdio_shim tests assert the serve.log cancel lines to prove the shim path, not the daemon's hangup, did it.
