# End-to-end benchmark kit

A macro benchmark for Tirith: K real LLM coding agents implement the same
8 tasks on the same small repository, coordinating only through Tirith.
It has two scenarios:

- **Forge** (the kit root, the default): shared files, a breaking change
  that lands while other agents build on the old interface, a
  near-duplicate task, and a noisy seeded history. Two builds of Tirith (or
  two settings) are compared by running the same scenario, seeded history,
  task board, worker prompt, coordination wrapper and scoring against each.
- **Hubs** (`scenarios/hubs/`, `--scenario hubs`): sub-file contention, file
  claims against symbol-anchored claims (ADR-0029). The two arms differ only
  in the claim-granularity paragraph of the worker prompt. See
  [Scenario: hubs](#scenario-hubs).

The kit does not spawn agents. Whoever runs the benchmark spawns the
workers with the prompt `prompt.sh` renders, then scores.

## Layout

| path | what |
|---|---|
| `scenario/` | "Forge", a Python 3.9 stdlib-only request-pipeline library: `app/pipeline.py` (Headers, Request, Response, Context, Middleware, Pipeline), `app/router.py` (Router, `build_pipeline`), `app/config.py`, four middlewares, handlers, a WSGI adapter, 24 tests (`python3 -m unittest`, ~0.1 s). ~620 lines of app code, ~280 of tests |
| `tasks.json` | 8 tasks with acceptance criteria, paths, priorities, dependencies |
| `hidden_tests/` | acceptance tests per task (57) plus 5 cross-task ordering tests; never copied into a run |
| `reference/` | a full solution; the hidden tests pass 57/57 and 5/5 on it (validation only, never copied into a run) |
| `history.json` | seeded coordination history: 30 memory notes, 15 decisions, 2 contracts (one republished, which emits one automatic notice), 40 notices |
| `setup.sh` | `setup.sh <run_dir> <port> [--scenario forge\|hubs] [--claims file\|symbol] [--wait sleep\|server] [--protocol manual\|hooks]`: repo copy + git baseline, detached daemon, capability probes, seeding, prints the MCP URL |
| `tb` | the only coordination command workers use; logs every call (and, for hubs, snapshots the code when a task is marked done) |
| `WORKER_PROMPT.md` | the Forge worker prompt template (placeholders `RUN_DIR`, `AGENT_NAME`), hardened preamble included |
| `prompt.sh` | `prompt.sh <run_dir> <agent>`: the rendered prompt for one worker of a prepared run, for either scenario and claims arm |
| `score.sh` | `score.sh <run_dir>`: JSON summary, also written to `<run_dir>/score.json` |
| `stop.sh` | `stop.sh <run_dir>`: SIGINT the run's daemon (only if that pid is a tirith daemon) |
| `lib/seed.py`, `lib/score.py`, `lib/anchors.py`, `lib/prompt.py`, `lib/turns.py`, `lib/violations.py` | helpers for setup, scoring, per-anchor metrics, prompt rendering, worker turns, writes without a claim |
| `hooks/` | the hooks protocol prototype (`--protocol hooks`): Claude Code hooks that claim, release and feed tasks, with a deterministic stop gate; see [hooks/README.md](hooks/README.md) |
| `dryrun/` | `fake_worker.sh` + `run_forge.sh` (Forge), `fake_worker_hubs.py` + `run_hubs.sh` (hubs), `fake_worker_hooks.py` + `run_hooks.sh` (hooks): scripted workers, no LLM, to check the plumbing |
| `scenarios/hubs/` | the hubs scenario: `scenario/`, `tasks.json` (with `anchors`), `hidden_tests/`, `reference/`, `history.json`, `WORKER_PROMPT.md`, `prompt/` (arm paragraphs) |

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

About a quarter of the rows matter to some task (6 of 40 notices, 4 of 15
decisions, 7 of 30 notes, both contracts; more are partial), and most noise
is written against the same shared paths, so a claim brief of the default
newest-five-per-section shows mostly noise. A CORS claim returned a 3.4 KB
brief with 19 more unread notices, none of the three CORS-relevant rows
among those shown. One decision is stale and contradicts a task (`d07`,
nginx rate limiting).

### Setup

`setup.sh` copies `scenario/` to `<run_dir>/repo`, makes a local git
baseline commit, starts `tirith --root <run_dir>/repo serve --bind
127.0.0.1:<port> --no-tray` fully detached (own session, stdin closed,
output to `serve.log`, so it survives the shell or agent tool that ran
setup; inherited `TIRITH_URL` and `TIRITH_AGENT` are scrubbed), and waits
until the daemon answers. It probes what the run relies on (hubs:
`wait_secs`, and anchor overlap for `--claims symbol`) and refuses to
continue if the daemon lacks it, seeds history then tasks through the CLI,
commits the seeded `.tirith/` as the scoring base, and copies `tb` into the
run directory. If setup fails after starting the daemon, it stops it. Setup
takes ~9 s for Forge.

Run directory after setup: `repo/`, `tb`, `url`, `tirith_bin`,
`daemon.pid`, `serve.log`, `meta.json` (scenario, claims, wait, protocol,
anchor probe, binary and version), `task_ids.json`, `setup/seed.jsonl`,
`coord.jsonl`, `coord_full.jsonl`, and for hubs `snapshots/`.

## Running one configuration

```bash
KIT=examples/e2e
RUN=$LAB/e2e/forge-r1        # a fresh directory per run
TIRITH_BIN=/path/to/tirith $KIT/setup.sh "$RUN" 7860
# Spawn K workers (K=3 recommended), each with the output of
# `$KIT/prompt.sh "$RUN" w1` (w2, w3, ...). Same model, same effort,
# same tool permissions for every configuration compared. Record each
# worker's token usage, cost, and wall time from the agent runtime.
# Wait until every worker has reported.
$KIT/score.sh "$RUN"          # before stop: task states come from the live daemon
$KIT/stop.sh "$RUN"
```

Then fill `worker_llm` in `score.json` with the runtime's numbers.

Workers need shell access to `RUN_DIR` (bash, `python3`, `jq`, `perl`,
`tar` are used by `tb`) and nothing else. The prompt forbids reading outside
`RUN_DIR/repo` and reading `.tirith/` files directly (some workers opened
memory notes in the copy instead of calling `tb`); the harness cannot
enforce that, so give workers a working directory of `RUN_DIR/repo` and no
access to this kit if the runtime allows.

The prompt starts with a hardened preamble from earlier benchmark rounds
(one round was invalidated when a worker ran `pkill -f "cat"` and killed the
daemons): no process signals, no `lsof`/`ps`, no Monitor or background
jobs, wait with a foreground `python3 -c "import time; time.sleep(N)"`
(plain `sleep` is blocked for agents), no `mcp__tirith__*` or Serena tools.

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
  deliveries and `tirith` push messages.
- `duplicate_work`: whether both `request-id` and `correlation-id` were
  marked done, and which `app/` files generate ids (more than one = two
  implementations).
- `wall_time_s`: first to last coordination call (call completion times).
- `git`: files/insertions/deletions against the seeded base, code and
  `.tirith/` separately.
- `daemon`: status at scoring time (orphaned tasks, persistence and load
  errors, claims).
- `worker_llm`: empty, for the runner to fill.

Latency in `tb` includes starting the CLI process (~20 ms); it is the same
for every configuration, so differences between configurations are
daemon-side.

## Threats to validity

- **LLM variance dominates.** One run per configuration says little. Do at
  least 2 repeats each (3+ if the difference is small), randomize the order,
  use the same model and settings, and report every run, not the best.
- **K is small and the machine is shared.** With K=3 on one Mac, contention
  is moderate; record the time of day and do not run configurations
  concurrently with other heavy work.
- **Scenario bias.** The kit author wrote the tasks, history, and hidden
  tests. The history was written to make filtering matter; a real
  repository's history may be more or less noisy.
- **Prompt compliance.** Workers may skip reading briefs, over-claim, or
  read outside the repo.
- **Wall time** is first-to-last coordination call and excludes worker
  start-up and final reporting; use the runtime's wall time as well.
- **Orphaning.** A worker silent for 1800 s loses its task, and a lease
  without activity ends after its TTL; the prompt asks for `ttl_secs: 1800`.
  Long silent thinking still produces `lost` events.

## What the kit cannot measure

- Worker LLM tokens, cost, turns, and thinking time (runtime only).
- Whether a worker actually read or used a brief row or push message, beyond what its final report says; the logs show only that the
  field was delivered.
- Code quality beyond the hidden tests (style, design, test quality).
- Behavior at scale (many agents, long histories, days of work).

## Validating the kit

```bash
TIRITH_BIN=<release build> dryrun/run_forge.sh <lab>/e2e-dryrun/forge 7850 3
```

The fake workers follow the protocol mechanically and append marker
comments instead of implementing, so tasks are all done while hidden
acceptance stays at the baseline 2/57. Scoring the reference solution
gives 57/57, 8 tasks accepted, 5/5 integration.

## Scenario: hubs

The measurement ADR-0029 asks for: do symbol-anchored claims
(`path#Symbol`) cut agent-blocked time by at least 20% against file claims,
when both arms already use edit windows and `claim` `wait_secs`, with no
lost write? Forge cannot answer it: its conflicts were on small files where
every task edits the same function (`build_pipeline`).

### Repository and tasks

"Tally" (`scenarios/hubs/scenario/`) is a Python 3.9 stdlib-only order
pricing library: `tally/money.py` (round, format, parse), `tally/rules.py`
(business tables), `tally/models.py` (`Item`, `Order`, `Totals`),
`tally/pricing.py` (`line_total`, `apply_discount`, `shipping_cost`,
`tax_for`, `order_total`), `tally/invoice.py`. ~190 lines of code, 12
visible tests (~1 ms). No task needs a new import, so symbol claims are
enough for every edit.

| key | priority | anchors it must edit | contention |
|---|---|---|---|
| `currency-precision` | 8 | `money.py#CURRENCY_DECIMALS`, `#round_money`, `#format_money`; `pricing.py#line_total`, `#apply_discount`, `#shipping_cost`, `#tax_for` | BREAKING: `round_money(amount, currency)` inside a shared file; every caller changes, including symbols three other tasks are editing |
| `bulk-discount` | 5 | `pricing.py#apply_discount`, `rules.py#DISCOUNT_CODES` | same symbol as the break; different symbols of `pricing.py`/`rules.py` than the next two |
| `free-shipping` | 5 | `pricing.py#shipping_cost`, `rules.py#FREE_SHIPPING_OVER` (new) | same symbol as the break |
| `tax-exempt` | 5 | `pricing.py#tax_for`, `rules.py#TAX_RATES`, `rules.py#TAX_EXEMPT_CATEGORIES` (new) | same symbol as the break |
| `parse-money` | 4 | `money.py#parse_money` | different symbol of the break's file |
| `gift-wrap` | 4 | `models.py#Order`, `models.py#Totals`, `pricing.py#order_total`, `rules.py#GIFT_WRAP_FEE` (new) | same symbols as `small-order-fee` (`Totals`, `order_total`) and `invoice-notes` (`Order`) |
| `small-order-fee` | 3 | `models.py#Totals`, `pricing.py#order_total`, `rules.py#SMALL_ORDER_FEE` (new) | same symbols as `gift-wrap` |
| `invoice-notes` | 3 | `models.py#Order`, `invoice.py#render_invoice` | same symbol as `gift-wrap` |

No task depends on another, so with K=3 the break races three features
that edit the functions it changes. `rules.py` is touched by five tasks on
six different symbols, four of them new; `pricing.py` by six tasks. Task
`paths` on the board are plain files in both arms; `anchors` in
`tasks.json` are for scoring and the fake worker, never shown to workers.
Each task asks for its own new test file, so test files do not contend.

Hidden tests (`hidden_tests/`): 48 per-task tests plus 4 integration tests
that price orders using every feature at once (USD, JPY, EUR), so an edit
lost to a concurrent write fails there even if its own module passed when
the task was marked done. 6 per-task tests pass on the untouched baseline
(unchanged USD pricing, 9-unit orders, plain parsing, general items taxed).
`reference/` passes 48/48 and 4/4. The seeded history is small (3 notes,
2 decisions, 1 notice, all accurate): this scenario does not measure
history filtering.

### Arms

Both arms run the same daemon with edit windows and `wait_secs` 120; they
differ only in the claim-granularity paragraph
(`scenarios/hubs/prompt/claims_file.md` or `claims_symbol.md`). The shared
rules: prepare without a claim, claim, write once, run the tests, release;
wait with `wait_secs` (`--wait server`, the default) or, for daemons
without it, a foreground python sleep (`--wait sleep`); use the same
`--wait` in both arms. The symbol paragraph uses ADR-0029's syntax
(`path#Symbol`, members as `Class.member`, new symbols by their new name,
imports ride on the symbol claim) and its safety rules (re-read before
writing, replace only your symbols' lines, leave the file importable, do
not fix a symbol someone else holds).

```bash
KIT=examples/e2e
export TIRITH_BIN=/path/to/tirith-with-wait_secs-and-anchors   # setup probes both
$KIT/setup.sh "$LAB/hubs/file-r1"   7880 --scenario hubs --claims file
$KIT/setup.sh "$LAB/hubs/symbol-r1" 7881 --scenario hubs --claims symbol
diff <($KIT/prompt.sh "$LAB/hubs/file-r1" w1 | sed "s#$LAB/hubs/file-r1#RUN#g") \
     <($KIT/prompt.sh "$LAB/hubs/symbol-r1" w1 | sed "s#$LAB/hubs/symbol-r1#RUN#g")
# exactly one line differs: the claims paragraph
# spawn K=3 workers per arm with prompt.sh output, then per run:
$KIT/score.sh "$RUN"; $KIT/stop.sh "$RUN"
```

`--claims symbol` refuses to start unless the daemon makes a file claim
conflict with an anchor in it and lets sibling anchors coexist. Daemons
without ADR-0029 accept `f#X` as a path unrelated to `f`, which fails open.
`UNSAFE_SKIP_ANCHOR_PROBE=1` overrides that for plumbing checks only;
`meta.json` records `anchor_probe: failed-skipped` and the run measures
nothing.

### What is measured (`score.json`, in addition to the Forge fields)

`scenario`, `claims`, `wait` at the top, and `anchors`, computed from the
call logs the same way in both arms:

- `blocked_s`, `blocked_s_per_agent`, `blocked_episodes`: the ADR-0029 gate
  metric. Per agent, the union of refusal episodes (first refused `claim`
  call to the end of the call that granted an overlapping key) and
  server-side waits (a granted claim with `wait_secs` that took at least
  0.25 s).
- `per_anchor` (every anchor in `tasks.json`): `conflicts` (refused rows
  whose requested and held keys both overlap the anchor), `holds`,
  `hold_s` (summed) and `covered_s` (union: time the anchor was
  unavailable to others), `held_by_keys`, `waits`, `server_waits`,
  `wait_s`, `max_wait_s`, `unresolved_waits`, `lost_writes`.
- `per_file`: the same without per-anchor duplication (a file claim counts
  once, not once per anchor), and `other_keys` for claims on non-anchor
  paths (test files).
- `granted_keys_by_kind` (file, symbol, dir): whether workers followed their
  arm's granularity.
- `lost_writes`: `tb` copies the code to `snapshots/` whenever a
  `task_update` to done succeeds. The scorer runs every hidden module on
  every snapshot; a test that passed in some snapshot and fails on the
  final code is a regression, listed with the task and agent after whose
  done it first and last passed. It is attributed to the task's anchors
  whose source differs between that last passing snapshot and the final
  code (`attribution: changed`), or to all of the task's anchors
  (`attribution: task`). `per_task` gives each task's tests passed at its
  done snapshot, at the end, and which were lost after done.

A key overlaps an anchor as the daemon decides it: directory or file above
it, the same anchor, or nested (`Order` and `Order.notes`), with `.`, `/`
and `::` equivalent and segments case-insensitive. Server-side waits are
attributed to every anchor newly granted by that claim, since the daemon
does not say which one it waited for; `blocked_s` is not affected.

### Threats specific to hubs

- **The file arm fails closed, the symbol arm fails open.** A worker that
  claims `pricing.py#apply_discount` and then also edits `tax_for` is not
  stopped by anything. Lost writes and `granted_keys_by_kind` show it only
  indirectly; read `coord_full.jsonl` and the diffs.
- **Lost-write detection needs a later passing snapshot.** A test that could
  only pass once every task landed (the integration tests, the break's JPY
  test until all callers are migrated) is not counted as lost if the last
  task's own write destroyed it; it shows as a plain failure. Snapshots are
  taken after the done call returns, so a concurrent half-written change by
  another agent can make a test fail in one snapshot and not another.
- **Edit tools differ.** Claude Code's Edit refuses stale writes; other
  clients, `cat >` or Serena symbol edits may not. Record the runtime.
- **`task_pull` holds.** Task `paths` are files in both arms; a daemon whose
  claim-aware `task_pull` compares them with anchor claims may order tasks
  differently between arms.
- **Small, synthetic hubs.** Symbols here are more disjoint than in
  Forge's measured runs (0 of 48 episodes were on disjoint symbols); a real
  repository sits somewhere between.

### Validating the hubs kit

```bash
TIRITH_BIN=<build with wait_secs> dryrun/run_hubs.sh <lab>/hubs-dry/file 7870 file 3
TIRITH_BIN=<build with wait_secs> dryrun/run_hubs.sh <lab>/hubs-dry/rogue 7871 file 3 --rogue
TIRITH_BIN=<build with anchors>   dryrun/run_hubs.sh <lab>/hubs-dry/symbol 7872 symbol 3
```

The hubs fake workers really implement the tasks by copying the task's
symbols from `reference/` into the live file inside each claim window, so
a clean run scores 8/8 done, 48/48 hidden, 4/4 integration, no
regressions. With `--rogue`, the last worker claims nothing, copies its
files from the baseline commit, waits until the other tasks are done and
writes its symbols into the stale copies; the scorer must then list
regressions with `attribution: changed` on the anchors it overwrote.
Measured 2026-09-17 on a 1.0.4 dev build: file arm 8/8, 48/48, 4/4, 0
regressions, 3 server waits, 2.5 s blocked, 17 s wall; rogue run (rogue
took `bulk-discount`) 18/48, 0/4 integration, 33 regressions: those on
the overwritten `pricing.py` and `rules.py` symbols with `attribution:
changed`, the invoice and integration tests it broke indirectly with
`attribution: task`; symbol arm on that daemon (no anchors, probe skipped,
plumbing only) 8/8, 48/48, 4/4, 24 symbol and 9 file grants, 0
regressions.
