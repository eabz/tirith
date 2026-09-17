# Jev end-to-end benchmark kit

A macro benchmark for Tirith with and without Jev assist (ADR-0024): K real
LLM coding agents implement the same 8 tasks on the same small repository,
coordinating only through Tirith. The two arms differ in one thing, the
daemon flag: `--jev` with `TIRITH_JEV_PROVIDER=gateway`, or nothing.
Everything else (scenario, seeded history, task board, worker prompt,
coordination wrapper, scoring) is byte-identical.

The kit does not spawn agents. Whoever runs the benchmark spawns the
workers with `WORKER_PROMPT.md`, then scores.

## Layout

| path | what |
|---|---|
| `scenario/` | "Forge", a Python 3.9 stdlib-only request-pipeline library: `app/pipeline.py` (Headers, Request, Response, Context, Middleware, Pipeline), `app/router.py` (Router, `build_pipeline`), `app/config.py`, four middlewares, handlers, a WSGI adapter, 24 tests (`python3 -m unittest`, ~0.1 s). ~620 lines of app code, ~280 of tests |
| `tasks.json` | 8 tasks with acceptance criteria, paths, priorities, dependencies |
| `hidden_tests/` | acceptance tests per task (57) plus 5 cross-task ordering tests; never copied into a run |
| `reference/` | a full solution; the hidden tests pass 57/57 and 5/5 on it (validation only, never copied into a run) |
| `history.json` | seeded coordination history: 30 memory notes, 15 decisions, 2 contracts (one republished, which emits one automatic notice), 40 notices |
| `history_labels.json` | per-item relevance to each task (relevant, partial, misleading, general, noise); analysis only |
| `setup.sh` | `setup.sh <run_dir> <port> <off\|jev>`: repo copy + git baseline, `.env` copy, daemon, seeding, prints the MCP URL |
| `tb` | the only coordination command workers use; logs every call |
| `WORKER_PROMPT.md` | the worker prompt template (placeholders `RUN_DIR`, `AGENT_NAME`) |
| `score.sh` | `score.sh <run_dir>`: JSON summary, also written to `<run_dir>/score.json` |
| `stop.sh` | `stop.sh <run_dir>`: SIGINT the run's daemon |
| `lib/seed.py`, `lib/score.py` | helpers for setup and scoring |
| `dryrun/` | `fake_worker.sh` (scripted, no LLM) and `run_arm.sh` to check the plumbing |

## Design

### Tasks and where coordination matters

| key | priority | depends on | why it is here |
|---|---|---|---|
| `ctx-interface` | 8 | | BREAKING: middleware hooks gain `ctx`, `Request.extras` is removed. Lands while other agents write middlewares against the old interface |
| `rate-limit` | 5 | | runs concurrently with the break; must adapt or its middleware raises TypeError |
| `cors` | 5 | | same; must also sit before auth |
| `request-id` | 6 | ctx-interface | must be outermost |
| `auth` | 4 | ctx-interface | ordering against cors and rate-limit |
| `access-log` | 3 | ctx-interface | must log short-circuited responses; reads `ctx.request_id`, `ctx.user` |
| `gzip` | 4 | | runs concurrently with the break |
| `correlation-id` | 2 | ctx-interface | a near-duplicate of `request-id`, worded differently, with wide paths (`app/middlewares/`, `tests/`) |

Every feature task edits the same three shared files (`app/config.py`,
`app/router.py`, `app/middlewares/__init__.py`), so claims contend, and the
order of registration in `build_pipeline` is checked by the integration
tests. With K=3, three tasks can start at once (`ctx-interface`,
`rate-limit`, `cors`); the rest unlock when the break lands.

Hidden tests exercise only what the acceptance criteria state (names,
signatures, headers, config fields, registration order). The
`correlation-id` tests check only observable `build_pipeline` behavior, so
a correct `request-id` implementation satisfies them; a second
implementation is scored as duplicate work.

### Seeded history

About a quarter of the rows matter to some task (labels: 6 of 40 notices,
4 of 15 decisions, 7 of 30 notes, both contracts are relevant; more are
partial), and most noise is written against the same shared paths, so a
claim brief of the default newest-five-per-section shows mostly noise. A
CORS claim on the off arm returned a 3.4 KB brief with 19 more unread
notices, none of the three CORS-relevant rows among those shown. One
decision is stale and contradicts a task (`d07`, nginx rate limiting).
Labels were written before any Jev output was seen.

### Setup, per arm

`setup.sh` copies `scenario/` to `<run_dir>/repo`, makes a local git
baseline commit, copies the Tirith repo `.env` (`chmod 600`, gitignored in
the copy), starts `tirith --root <run_dir>/repo serve --bind 127.0.0.1:<port>
--no-tray` (plus `--jev` and `TIRITH_JEV_PROVIDER=gateway` on the jev arm;
inherited `TIRITH_*` and key variables are scrubbed in both), waits until
the daemon answers (and, on the jev arm, until it reports
`jev: on (gateway`), seeds history then tasks through the CLI, commits the
seeded `.tirith/` as the scoring base, snapshots `status` (so Jev calls
spent on seeding are subtracted from the run), and copies `tb` into the run
directory. Setup takes ~9 s (off) and ~19 s (jev, which spends ~35 Jev
calls on duplicate checks while seeding).

Run directory after setup: `repo/`, `tb`, `url`, `tirith_bin`,
`daemon.pid`, `serve.log`, `meta.json`, `task_ids.json`,
`setup/{seed.jsonl,status.json}`, `coord.jsonl`, `coord_full.jsonl`.

## Running one arm

```bash
KIT=examples/jev_e2e
RUN=$LAB/e2e/jev-r1          # a fresh directory per run
$KIT/setup.sh "$RUN" 7860 jev # or: off
# Spawn K workers (K=3 recommended), each with WORKER_PROMPT.md where
# RUN_DIR -> $RUN and AGENT_NAME -> w1, w2, w3. Same model, same effort,
# same tool permissions in both arms. Record each worker's token usage,
# cost, and wall time from the agent runtime.
# Wait until every worker has reported.
$KIT/score.sh "$RUN"          # before stop: tasks and Jev counters come from the live daemon
$KIT/stop.sh "$RUN"
```

Then fill `worker_llm` in `score.json` with the runtime's numbers.

Workers need shell access to `RUN_DIR` (bash, `python3`, `jq`, `perl` are
used by `tb`) and nothing else. The prompt forbids reading outside
`RUN_DIR/repo`; the harness cannot enforce that, so give workers a working
directory of `RUN_DIR/repo` and no access to this kit if the runtime allows.

## What is measured (`score.json`)

- `tasks`: done/total from the board, and who finished each.
- `acceptance`: hidden tests passed/total (57), tasks whose hidden tests all
  pass, per-task failures, and the 5 integration (ordering) tests. Two
  `ctx-interface` regression tests pass on the untouched baseline.
- `visible_tests`: the workers' own `python3 -m unittest` in a clean copy.
- `coordination`: calls, statuses, response bytes (and bytes/4 as an
  estimate of tokens the workers read from Tirith), request bytes, latency
  mean/p50/p95/max overall and per tool, per agent, claim conflicts, lost
  leases, notices/contracts/messages/notes/decisions written, inbox
  deliveries and `tirith` push messages, and which Jev fields appeared
  (`claim.skipped`, `claim.advice`, ...).
- `duplicate_work`: whether both `request-id` and `correlation-id` were
  marked done, and which `app/` files generate ids (more than one = two
  implementations).
- `wall_time_s`: first to last coordination call (call completion times).
- `git`: files/insertions/deletions against the seeded base, code and
  `.tirith/` separately.
- `jev`: the daemon's Jev counters during the run (calls, failures,
  questions, input tokens, cost, latency, per site), setup excluded.
- `worker_llm`: empty, for the runner to fill.

Latency in `tb` includes starting the CLI process (~20 ms); it is the same
in both arms, so differences between arms are daemon-side.

## Threats to validity

- **LLM variance dominates.** One run per arm says little. Do at least 2
  repeats per arm (3+ if the difference is small), randomize the arm order,
  use the same model and settings, and report every run, not the best.
- **K is small and the machine is shared.** With K=3 on one Mac, contention
  is moderate, and gateway latency varies with the network; record the time
  of day and do not run arms concurrently with other heavy work.
- **Rate limits.** The gateway key may return 429; Jev then fails over to
  deterministic behavior, which silently turns the jev arm into the off arm
  for those calls. Check `jev.during_run.failures`.
- **Scenario bias.** The kit author wrote the tasks, history, labels, and
  hidden tests, and knows what Jev does. The history was written to make
  filtering matter; a real repository's history may be more or less noisy.
  Labels are one person's judgement.
- **Prompt compliance.** Workers may skip reading briefs, ignore extra
  fields, over-claim, or read outside the repo. Rule 6 names Jev-only fields
  in both arms; that is identical text, but it may prime workers to look.
- **Wall time** is first-to-last coordination call and excludes worker
  start-up and final reporting; use the runtime's wall time as well.
- **Orphaning.** A worker silent for 1800 s loses its task, and a lease
  without activity ends after its TTL; the prompt asks for `ttl_secs: 1800`.
  Long silent thinking still produces `lost` events in either arm.
- **`.env` in the run copy.** The daemon reads keys from it; a worker that
  disobeys rule 1 could read them. Use a sandbox if the runtime has one.

## What the kit cannot measure

- Worker LLM tokens, cost, turns, and thinking time (runtime only).
- Whether a worker actually read or used a brief row, advice, or push
  message, beyond what its final report says; the logs show only that the
  field was delivered.
- Code quality beyond the hidden tests (style, design, test quality).
- Whether a Jev field misled a worker, unless the report says so or a
  failure can be traced to it by reading `coord_full.jsonl`.
- `task_create` duplicate detection and `memory_write` similar-note
  detection reach only the caller. Tasks are created by setup, so workers
  see `possible_duplicate` only if they create tasks themselves (setup's
  own answers are in `setup/seed.jsonl`).
- Behavior at scale (many agents, long histories, days of work).

## Validating the kit

```bash
dryrun/run_arm.sh <lab>/e2e-dryrun/off 7850 off 3
dryrun/run_arm.sh <lab>/e2e-dryrun/jev 7851 jev 3
```

The fake workers follow the protocol mechanically and append marker
comments instead of implementing, so tasks are all done while hidden
acceptance stays at the baseline 2/57. Scoring the reference solution
gives 57/57, 8 tasks accepted, 5/5 integration.
