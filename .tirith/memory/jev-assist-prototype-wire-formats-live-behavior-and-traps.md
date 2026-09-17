---
id: 64e019d9-d9d6-45e9-9708-1e7b4db4a3d2
permalink: jev-assist-prototype-wire-formats-live-behavior-and-traps
title: "Jev assist prototype: wire formats, live behavior, and traps"
kind: research
tags:
- jev
- experiment
- tokens
- latency
paths:
- src/jev.rs
- src/assist.rs
- src/server.rs
- src/state.rs
- tests/jev_assist.rs
- docs/5-decisions/0024-jev-assist-experiment.md
author: jev-proto
updated_by: jev-proto
created_at: 2026-09-17T01:27:57Z
updated_at: 2026-09-17T01:27:57Z
---

Branch `experiment/jev`, 2026-09-16. ADR-0024 holds the design; this note holds what is not in the files.

- [wire] Vercel gateway: `POST https://ai-gateway.vercel.sh/v4/ai/evaluation-model`, headers `ai-gateway-protocol-version: 0.0.1`, `ai-gateway-auth-method: api-key`, `ai-evaluation-model-specification-version: 4`, `ai-model-id: typesafe-ai/jev` (`typesafe-ai/jev-latest` is 404 on the gateway even though the SDK types list it). Taken from `@ai-sdk/gateway` source, not documented. #jev
- [wire] TypeSafe direct: `POST https://api.typesafe.ai/v1/systemone`, bearer `TYPESAFE_API_KEY`, `model: jev-latest` in the body, boolean questions are type `noul` and answer field `noul`; usage is snake_case; no cost field (computed at $0.042/M input). #jev
- [gotcha] A Vercel key on the free tier answers 429 "Free tier requests on this model are rate-limited" after about five calls a minute; enabling a budget alone did not lift it on 2026-09-16. Retrying a 429 doubled time-to-fallback, so 429 is not retried. #jev
- [measured] Gateway warm latency 240-600 ms for 1-13 questions from macOS; first call ~1.3 s with TLS handshake (JevClient::warm_up exists for that). A 6-question brief was ~1,000 input tokens, $0.00004. #latency
- [measured] Quality on a middleware scenario was good: a CORS task's brief kept the CORS and trait-signature notices and dropped the rate-limit decision, log notes and log-format notice; a rephrased gotcha note was flagged as duplicate at 1.0. #jev
- [gotcha] Brief-skipped notices are NOT marked seen (State::brief_candidates + mark_briefed), so they reappear in unread listings and get re-judged at the next claim. #brief
- [gotcha] `live_run.sh` style manual runs: `tirith --root <scratch> serve --bind 127.0.0.1:<port> --no-tray --jev` with a copied `.env`; strip ANSI from serve.log before grepping the `jev` lines. #experiment
