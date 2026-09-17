# ADR-0024: Jev makes coordination judgement calls, behind a flag and with deterministic fallbacks

**Status:** Experimental, 2026-09-16. Behind the `jev` cargo feature and
`tirith serve --jev`; not in release binaries. Accept, revise, or drop
after the A/B runs described below.

## Context

Tirith's own decisions (overlap, leases, ownership, persistence) are
exact and cost no tokens. What costs tokens and time is agents reading
coordination output and deciding what to do with it: a brief row that
does not concern the task, a broadcast to every agent, a search that
misses because the query used other words, two agents filing the same
task. ADR-0014 measured the read side: every row an agent receives is
re-read on every later turn.

Jev (`TypeSafe` AI, September 2026) is a model that does not generate
text. It takes a JSON state and named boolean, choice, and score
questions and returns calibrated probabilities for all of them in one
round trip, at $0.042 per million input tokens and free output. Live
calls measured 240–600 ms warm from this machine, one to thirteen
questions each. It is reachable directly (`api.typesafe.ai/v1/systemone`,
documented) and through the Vercel AI Gateway (format taken from SDK
source, marked as changeable in patch releases).

## Decision

1. **A connector, `src/jev.rs`.** Provider-neutral `Request`, `Question`,
   `Answer`, `Response`, behind an object-safe `Evaluator` trait so tests
   use a scripted fake and never the network. `JevClient` speaks both
   providers, prefers `TypeSafe` when `TYPESAFE_API_KEY` is set, else
   `AI_GATEWAY_API_KEY`; `TIRITH_JEV_PROVIDER` forces one. Keys come from
   the environment or a gitignored `.env` in the repository root. One
   retry on 408 and 5xx inside a 5 s budget; a 429 falls back at once,
   because retrying it doubled the time to fallback in live runs.
2. **HTTPS only in a `jev` cargo feature** (reqwest's rustls). Default
   builds stay localhost-only; `tirith serve --jev` in a build without the
   feature exits with an error.
3. **A policy module, `src/assist.rs`,** holding every question Tirith
   asks and how an answer is applied. `server.rs` fetches candidates from
   `State`, calls one function, and applies the result. The `State` lock
   is never held across a Jev call.
4. **Nine sites, each an addition or a filter, never a change to an
   exact rule:**

   | site | effect | when Jev fails or is unsure |
   |---|---|---|
   | claim brief | rows the task does not need are left out, relevant first; `skipped` counts them; skipped notices stay unread | newest five per section |
   | refused claim | `advice`: wait, work elsewhere, coordinate, or narrow the claim | no `advice` |
   | `task_pull` | among the top-priority tier only, the task nearest the agent's claims and recent tasks; `picked_by: jev` | oldest task |
   | `task_create` | `possible_duplicate` of an open task | nothing |
   | `notice_publish`, contract republish | holders whose work the change likely breaks get an inbox message from `tirith` now, in the background | no push (as today) |
   | `message_send` to `*` | recipients the message does not concern are skipped; `skipped_recipients` | everyone |
   | `memory_search` with a query | term hits plus newest notes, reranked and filtered by meaning; `ranked_by`, `relevance` | term ranking |
   | `decision_list` with a query, first page | decisions matching by meaning | substring filter |
   | `memory_write` creating a note | `similar_note` | nothing |

5. **Observable.** `status` carries `jev`: calls, failures, questions,
   input tokens, cost, and latency per site. Each call is logged.
6. **No tool schema changes.** Only optional output fields are added, so
   `tools/list` and its budget are untouched.

## Alternatives

- **Jev decides claims or ownership.** Rejected: a probabilistic answer
  to "may these two agents both edit this file" is a correctness bug.
- **Agents call Jev themselves.** Rejected: the expensive agent still
  pays a turn to ask and read the answer. The saving only exists when
  the server asks on the agent's behalf.
- **A general LLM for the same sites.** Rejected for this experiment: it
  costs 5–250x more per token and seconds per call; the questions are
  classification, which is what Jev is built for.
- **Gateway only.** Rejected: the key used first was on a free tier and
  rate-limited after five calls; the direct API is documented and one
  hop shorter.

## Consequences

- A claim, a pull, a create, a search, and a broadcast can each take one
  Jev round trip longer (hundreds of milliseconds) when Jev is on.
  Notice fan-out does not, because it runs in the background.
- Thresholds are guesses (`src/assist.rs` constants) until calibrated on
  labeled runs.
- Whether this pays is unmeasured. The A/B plan: the same multi-agent
  task on a scratch repository, once with `--jev` and once without,
  comparing agent tokens, wall time, rework (lost or conflicting edits),
  and Jev's cost. The ADR is accepted, revised, or dropped on that data.
