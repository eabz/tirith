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

On top of both, the **coordination benchmark** (`bench/`, `setup.sh --arm`)
compares three worker protocols, baseline, edit windows with server waits,
and symbol claims, on the same frozen daemon, with pre-registered decision
rules. See [Coordination benchmark](#coordination-benchmark).

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
| `setup.sh` | `setup.sh <run_dir> <port> [--scenario forge\|hubs] [--arm baseline\|windows\|symbols] [--claims file\|symbol] [--hold task\|edit] [--wait sleep\|server] [--pull poll\|wait] [--protocol manual\|hooks]`: repo copy + git baseline, detached daemon, capability probes, seeding, prints the MCP URL |
| `tb` | the only coordination command workers use; logs every call and snapshots the code when a task is marked done |
| `WORKER_PROMPT.md` | the Forge worker prompt template (placeholders `RUN_DIR`, `AGENT_NAME`, and the protocol paragraphs `{{CLAIMS}}`, `{{HOLD}}`, `{{WAIT}}`, `{{PULL}}`), hardened preamble included |
| `prompt/` | the protocol paragraphs: `claims_file\|symbol.md` (Forge examples), `hold_task\|edit.md`, `wait_sleep\|server.md`, `pull_poll\|wait.md` (shared; a scenario's own `prompt/` wins) |
| `prompt.sh` | `prompt.sh <run_dir> <agent>`: the rendered prompt for one worker of a prepared run, for either scenario and any protocol |
| `score.sh` | `score.sh <run_dir>`: JSON summary, also written to `<run_dir>/score.json` |
| `stop.sh` | `stop.sh <run_dir>`: SIGINT the run's daemon (only if that pid is a tirith daemon) |
| `lib/seed.py`, `lib/score.py`, `lib/anchors.py`, `lib/prompt.py`, `lib/turns.py`, `lib/violations.py`, `lib/benchstats.py`, `lib/aggregate.py` | helpers for setup, scoring, claim metrics and lost writes, prompt rendering, worker turns, writes without a claim, the flat `bench` block, and the cross-run aggregate with the decision rules |
| `bench/` | the coordination benchmark: `freeze_bin.sh` (one binary for every arm), `prepare_round.sh` (6 runs, prompts, runbook), `prompt_diff.sh` (prompts differ only in protocol paragraphs), `watch.py` (watchdog) |
| `hooks/` | the hooks protocol prototype (`--protocol hooks`): Claude Code hooks that claim, release and feed tasks, with a deterministic stop gate; see [hooks/README.md](hooks/README.md) |
| `dryrun/` | scripted workers, no LLM, to check the plumbing: `fake_worker_forge.py` and `fake_worker_hubs.py` (manual protocol, any arm, on `fake_common.py`) driven by `run_fake.sh` (one run) or `fake_round.sh` (a prepared benchmark round); `fake_worker_hooks.py` + `run_hooks.sh` (hooks); `transcript.py` writes the synthetic Claude Code transcripts they leave for the turn and violation metrics |
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
until the daemon answers. It probes what the run relies on (claim
`wait_secs` for `--wait server`, `task_pull` `wait_secs` for `--pull wait`,
and anchor overlap for `--claims symbol`) and refuses to
continue if the daemon lacks it, seeds history then tasks through the CLI,
commits the seeded `.tirith/` as the scoring base, and copies `tb` into the
run directory. If setup fails after starting the daemon, it stops it. Setup
takes ~9 s for Forge.

Run directory after setup: `repo/`, `tb`, `url`, `tirith_bin`,
`daemon.pid`, `serve.log`, `meta.json` (scenario, arm, claims, hold, wait,
pull, protocol, anchor probe, binary, version, and `tirith_build` / `tirith_commit`
from a `bench/freeze_bin.sh` sidecar), `task_ids.json`, `setup/seed.jsonl`,
`coord.jsonl`, `coord_full.jsonl`, and `snapshots/`.

Without `--arm`, the protocol options default to each scenario's original
prompt: Forge claims whole files for the whole task and sleeps on refusals
(`file task sleep poll`), hubs uses edit windows and server waits (`file edit
server poll`).

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

## Protocols: manual and hooks

`--protocol` selects who performs the mechanical coordination steps; the
scenario, board, history and scoring are the same.

- `manual` (default): the worker prompt (`WORKER_PROMPT.md`) tells the model
  to `tb claim`, `tb release`, `tb task_pull` and `tb task_update` itself.
- `hooks`: Claude Code hooks do those four steps outside the model loop
  (claim before each edit with `wait_secs`, deny the edit if refused, release
  when the edit window closes, mark the task done through a stop gate, feed
  the next task when the model stops). Workers get `WORKER_PROMPT_HOOKS.md`,
  the same prompt without those steps. Setup copies `hooks/` to
  `<run_dir>/hooks/`, writes the settings to `<run_dir>/hooks/settings.json`
  and `<run_dir>/repo/.claude/settings.json`, and keeps hook state in
  `<run_dir>/hooks_state/`. It requires `--claims file` and a daemon with
  `wait_secs`, and refuses a run directory inside the Tirith checkout, one
  that contains it, or `$HOME`. `HOOKS_CONFIG='{...}'` overrides the knobs in
  `hooks/common.py` `DEFAULTS` (claim wait, gate scope, idle budget).

Hooks only run in Claude Code sessions that load those settings: start each
worker as its own `TB_AGENT=w1 claude -p` in `<run_dir>/repo`, or pass
`--settings <run_dir>/hooks/settings.json`. Launch both arms the same way,
so the difference is the protocol, not the runtime. The stop gate decides
done on the task's own acceptance tests, not the whole suite, which makes it
an oracle the manual arm lacks; see [hooks/README.md](hooks/README.md#stop-gate)
before comparing acceptance across protocols.

For turn and violation metrics, save each worker's Claude Code transcript as
`<run_dir>/transcripts/<agent>.jsonl` (hook runs also find the transcript
paths the hooks recorded).

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
- `anchors` (every scenario): claim metrics from the call logs (`blocked_s`,
  refusal and wait episodes, holds, granted keys by kind; `per_anchor` and
  `per_file` for scenarios with task anchors) and `lost_writes` from the
  done-time snapshots. See [What is measured](#what-is-measured-scorejson-in-addition-to-the-forge-fields)
  under hubs; in Forge a regression's `anchors` are empty (`attribution: task`).
- `bench`: the flat numbers the benchmark arms are compared on; see
  [Scoring](#scoring-scorejson-bench).
- `protocol`: `manual` or `hooks`.
- `worker_turns` (null without transcripts): per worker and in total, model
  turns (assistant rows sharing a message id) by kind, with estimated cost
  units (input 1, cache write 1.25, cache read 0.1, output 5): `mechanical`
  (only `tb claim|release|task_pull|task_update`), `coordination` (only `tb`
  calls, at least one other tool), `work`, `text`, `coordination_only`
  (mechanical + coordination), and `coordination_only_with_a_mechanical_call`.
  See `lib/turns.py`.
- `violations` (null without transcripts): repository writes a worker made
  without a live claim of its own. Tool edits (Edit, Write, MultiEdit,
  NotebookEdit, Serena edit tools) are exact; shell writes (`>`, `>>`, `tee`,
  `sed -i`, Python `open(..., "w")`) are heuristic. Each write is timed at its
  tool result and checked against the writer's claim intervals from
  `coord_full.jsonl` (claim to release or lost lease). Per worker and total:
  `tool_edits`, `bash_writes`, `unclaimed`, `anchor_only` (covered only by a
  `file#Symbol` claim), and per worker `unclaimed_paths`. Denied edits are
  not writes. See `lib/violations.py`.
- `workers` (null without transcripts): one row per worker: `turns`,
  `coordination_only_turns`, `mechanical_turns`, `cost_units`,
  `coordination_only_cost_share`, `unclaimed_writes`.
- `hooks` (hook runs only): hook decisions from `hooks_state/events.jsonl`
  (`actions` overall and per agent: claim, deny, release, task-done,
  task-blocked, block-gate, block-task, ...), `claim_ms` and `gate_ms`
  latency, and the number of hook errors. `coordination.per_origin` splits
  calls between `hook` and `model`.
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

- Worker LLM tokens, cost and thinking time (runtime only). Turns and
  estimated cost shares come from transcripts only when the runner saves them.
- Writes outside the tools the transcripts show (a shell write the heuristic
  misses, or a subagent's edits, which live in the subagent's own transcript).
- Whether a worker actually read or used a brief row or push message, beyond what its final report says; the logs show only that the
  field was delivered.
- Code quality beyond the hidden tests (style, design, test quality).
- Behavior at scale (many agents, long histories, days of work).

## Validating the kit

```bash
TIRITH_BIN=<release build> dryrun/run_fake.sh <lab>/e2e-dryrun/forge 7850 3
TIRITH_BIN=<release build> dryrun/run_fake.sh <lab>/e2e-dryrun/forge-rogue 7851 3 --rogue
```

`run_fake.sh <run_dir> <port> [workers] [--rogue] [setup.sh options...]`
passes the options to `setup.sh` (`--scenario hubs`, `--arm symbols`, ...)
and picks the scenario's fake worker.

The hooks protocol has its own check, which runs a selfcheck of the hook
scripts (deny, release, gate scope, violation counting and more) and then
scripted workers that only talk to the hooks; see
[hooks/README.md](hooks/README.md#plumbing-check-no-llm):

```bash
TIRITH_BIN=<build with wait_secs> dryrun/run_hooks.sh <lab>/hooks-dry 7892 3
```

The Forge fake workers follow the run's protocol mechanically and really
implement the tasks from `reference/`: `ctx-interface` copies the files only
it changes, each feature task copies its middleware module and merges its
own reference lines (found by keyword) into the shared files after the
nearest preceding reference line the file already has, and
`correlation-id` is marked done as covered by `request-id`.
`fake_worker_forge.py --selfcheck` applies all tasks in 200 random orders
and gets `reference/` back. A clean run scores 8/8, 57/57, 5/5 integration,
no lost writes. With `--rogue` the last worker claims nothing and writes its
task over baseline copies of the shared files after the others finish,
which must show up as lost writes and unclaimed writes. Scoring the
reference solution gives 57/57, 8 tasks accepted, 5/5 integration.

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
rules: prepare without a claim, claim, write once, run the tests, release
(`prompt/hold_edit.md`); wait with `wait_secs` (`--wait server`, the
default) or, for daemons without it, a foreground python sleep (`--wait
sleep`); use the same `--wait` in both arms. The coordination benchmark's
`windows` and `symbols` arms are these two arms with `task_pull`
`wait_secs` added (`--pull wait`). The symbol paragraph uses ADR-0029's syntax
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
# exactly one line differs: the claims paragraph ($KIT/bench/prompt_diff.sh checks it)
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

`scenario`, `arm`, `claims`, `hold`, `wait`, `pull` at the top, and `anchors`, computed from the
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
TIRITH_BIN=<build with wait_secs> dryrun/run_fake.sh <lab>/hubs-dry/file 7870 3 --scenario hubs --claims file
TIRITH_BIN=<build with wait_secs> dryrun/run_fake.sh <lab>/hubs-dry/rogue 7871 3 --rogue --scenario hubs --claims file
TIRITH_BIN=<build with anchors>   dryrun/run_fake.sh <lab>/hubs-dry/symbol 7872 3 --scenario hubs --claims symbol
```

The hubs fake workers really implement the tasks by copying the task's
symbols from `reference/` into the live file inside each claim window, so
a clean run scores 8/8 done, 48/48 hidden, 4/4 integration, no
regressions. With `--hold task` they claim every key of the task first and
release after done. With `--rogue`, the last worker claims nothing, copies its
files from the baseline commit, waits until the other tasks are done and
writes its symbols into the stale copies; the scorer must then list
regressions with `attribution: changed` on the anchors it overwrote.
Measured 2026-09-17 on a 1.0.4 dev build (with the earlier `run_hubs.sh`): file arm 8/8, 48/48, 4/4, 0
regressions, 3 server waits, 2.5 s blocked, 17 s wall; rogue run (rogue
took `bulk-discount`) 18/48, 0/4 integration, 33 regressions: those on
the overwritten `pricing.py` and `rules.py` symbols with `attribution:
changed`, the invoice and integration tests it broke indirectly with
`attribution: task`; symbol arm on that daemon (no anchors, probe skipped,
plumbing only) 8/8, 48/48, 4/4, 24 symbol and 9 file grants, 0
regressions.

## Coordination benchmark

Board task `5c603223`. The claim under test: the deterministic protocol
changes cut agent wall time and tokens under contention, and symbol claims
(ADR-0029, Proposed) cut agent-blocked time further. Earlier numbers were
replay estimates from old logs.

### Arms

Every arm runs the same frozen daemon binary, the same scenario, board,
history, hardened preamble and scoring. The arms differ only in four prompt
paragraphs, chosen by `setup.sh --arm`:

| arm | `--claims` | `--hold` | `--wait` | `--pull` | protocol |
|---|---|---|---|---|---|
| `baseline` (A) | file | task | sleep | poll | Tirith 1.0.3 behavior: claim every file of the task at its start and hold until done; on a refused claim sleep 30 s and retry; `task_pull` without `wait_secs`, sleep 30 s when nothing is free |
| `windows` (B) | file | edit | server | wait | edit windows (prepare, claim just before writing, release after the check); `claim` `wait_secs` 120; `task_pull` `wait_secs` 120 |
| `symbols` (C) | symbol | edit | server | wait | B with ADR-0029 anchors: `path#Symbol` for the symbols an edit changes (members as `Class.member`, new ones by their new name, each anchor of a multi-symbol edit); imports and `__all__` entries ride on the symbol claim; whole files for new files and for import-only files |

The paragraphs are `prompt/<option>_<value>.md`; hubs has its own
`claims_*.md` with Tally examples. `bench/prompt_diff.sh <run_a> <run_b>`
fails unless two runs' templates are byte-identical outside the paragraphs
and prints the rendered diff: `baseline` and `windows` differ in three
paragraphs (hold, wait, pull), `windows` and `symbols` in one (claims).
B changes three things at once, so the benchmark can say whether B beats A,
not which paragraph did it.

### Runbook

```bash
KIT=examples/e2e
LAB=/path/outside/the/repo/coord-bench
cargo build --release
$KIT/bench/freeze_bin.sh . "$LAB/bin"      # once: every arm and round runs this copy
                                           # (FREEZE_NOTE='...' describes a dirty tree)
export TIRITH_BIN=$LAB/bin/tirith          # meta.json records its version and commit
$KIT/bench/prepare_round.sh "$LAB" 1       # rounds 2 and 3 the same
```

`prepare_round.sh <lab> <round> [base_port]` sets up the 6 runs of a round,
`<lab>/r<round>-<scenario>-<arm>`, on ports `base_port`..`base_port+5`
(default `7900 + 10*round`), writes `bench.json` (round, order, port) into
each, renders `<lab>/prompts/<run>/w1.md`..`w3.md` (`WORKERS` changes K),
runs the prompt check for both scenarios, and prints the runbook for that
round. The arm order rotates by round (a Latin square over 3 rounds) and the
scenario that goes first alternates. If a setup fails, the runs it already
started are stopped. It spawns nothing:

1. Spawn 3 workers per run with the prompt files, in the printed order: same
   model, effort and tool permissions everywhere, working directory
   `<run>/repo`, worker names `w1`..`w3`.
2. `python3 $KIT/bench/watch.py <runs>` in the foreground: one line per run
   (tasks done, in-progress owners, live claims, seconds since the last
   coordination call) from the daemon's read-only `/api/state`. Exit 0: all
   tasks done; 2: `--max-secs` (default 540) elapsed, run it again; 3: a
   daemon does not answer or a run made no call for `--stall-secs` (900).
3. When all workers of a run have reported: append one row per worker to
   `<lab>/usage.tsv` from its completion notice, copy each transcript to
   `<run>/transcripts/w<i>.jsonl` (Agent-tool workers:
   `~/.claude/projects/<project>/<session>/subagents/agent-<id>.jsonl`),
   then `score.sh` while the daemon is up, then `stop.sh`.
4. After the last round: `python3 $KIT/lib/aggregate.py --json
   $LAB/aggregate.json $LAB > $LAB/aggregate.md`. Results go to
   `docs/3-tests/` with a memory note.

`usage.tsv` is tab-separated with a header row: `run` (the run directory
name), `worker`, `total_tokens`, `tool_uses`, `duration_ms`, and optionally
`output_tokens`, `cost_usd`. It may sit in the run directory or the lab
directory; `aggregate.py` rereads it, so rows added after scoring count.

### Scoring (`score.json` `bench`)

`lib/benchstats.py` flattens each run into the numbers the arms are compared
on (null when the source is missing):

- `done_time_s`: first coordination call to the last task marked done, the
  benchmark's wall time. `wall_time_s` (to the last call) sits beside it:
  a worker that pulls with `wait_secs` can spend up to 120 s learning the
  board is empty.
- `hidden_passed`/`hidden_total`, `tasks_accepted`, `integration_passed`,
  `visible_tests_ok`, `tasks_done`.
- `lost_writes`: hidden tests that passed at a done-time snapshot and fail
  on the final code (`anchors.lost_writes`), for both scenarios. In Forge a
  later task breaking an earlier one's test also counts; read the
  regressions before blaming claims.
- `blocked_s`, `blocked_episodes`: claim-blocked agent seconds (refusal
  episodes plus server-side claim waits), ADR-0029's gate metric.
- `pull_idle_s`: agent seconds without a task: `task_pull` calls that waited
  server-side (>= 0.25 s) plus the time from a `none` pull to that agent's
  next pull. `waiting_s` = `blocked_s` + `pull_idle_s`.
- `claims_granted`, `claims_refused`, `claims_waited` and `claim_wait_s`
  (claims with `wait_secs` that took >= 0.25 s, granted or not), `pulls`,
  `pulls_none`, `pulls_held` (handed out with `waiting_on`), `pulls_waited`.
- From transcripts: `turns`, `coordination_only_turns` (`lib/turns.py`),
  `cost_units`, `test_runs` and `failing_test_runs` (Bash tool uses running
  `unittest` whose result is an error or prints `FAILED (`),
  `unclaimed_writes` (`lib/violations.py`).
- From `usage.tsv`: `usage_workers` and the sums of `total_tokens`,
  `output_tokens`, `tool_uses`, `duration_ms`, `cost_usd` (a sum is null
  unless every row of the run has the column).

### Aggregation and decision rules

`lib/aggregate.py [--json out.json] [--resamples N] <lab or runs>` prints
Markdown: per scenario and arm the mean, median and a 95% bootstrap CI of the
mean; paired differences by round (same scenario and round) for `windows -
baseline` and `symbols - windows`, absolute and relative, with bootstrap CIs
over pairs; the decision rules; and the verdicts. Bootstrap intervals are
percentile intervals from 10000 resamples with a fixed seed. The rules were
written before any real run:

- **R1.** `windows` ships as the documented protocol if, against
  `baseline`, wall time or cost improves and hidden tests do not drop. Wall
  time is `done_time_s`; cost is `cost_usd` when every paired run has it,
  else `total_tokens`, else `cost_units`. Improves: the mean relative paired
  difference (B - A) / A over all (scenario, round) pairs is below 0 and its
  95% CI lies entirely below 0. No drop: the 95% CI of the mean paired
  difference in hidden tests passed is not entirely below 0, and the same
  for integration tests.
- **R2.** ADR-0029 is Accepted only if, against `windows`: over all pairs,
  `1 - sum(C blocked_s) / sum(B blocked_s) >= 0.20` with a 95% CI lower bound
  above 0; `lost_writes` is 0 in every `symbols` run; and `symbols` has no
  more failing test runs in total than `windows` over the same pairs.
  Otherwise it is Rejected and anchors are removed. A rule whose inputs are
  missing (no transcripts, no usage) is undecided, never passed.
- Both verdicts are marked provisional with fewer than 3 pairs per scenario.

### Dry runs

`dryrun/fake_round.sh <lab> <round>` runs 3 fake workers on every run of a
round prepared by `prepare_round.sh`, all at once as the real rounds do,
then the watchdog, `score.sh`, `stop.sh`, and one line per run. The fakes
emit synthetic transcripts, so every `bench` field except usage is filled.
Fake timings (`FAKE_SLEEP_S`, `FAKE_PREP_S`, `FAKE_WORK_S`) are seconds, not
the minutes an LLM takes, so dry-run times say nothing about the arms.

```bash
export TIRITH_BIN=$LAB/bin/tirith
$KIT/bench/prepare_round.sh "$LAB/dry" 1 8410
$KIT/dryrun/fake_round.sh "$LAB/dry" 1
$KIT/dryrun/run_fake.sh "$LAB/rogue/forge" 8420 3 --rogue --arm baseline   # and --arm symbols
$KIT/dryrun/run_fake.sh "$LAB/rogue/hubs" 8421 3 --rogue --scenario hubs --arm baseline
python3 $KIT/lib/aggregate.py "$LAB/dry"
```

Measured 2026-09-17 on the 1.0.5 working tree with the pull fix (task
`7bc643e2`; cba7afe plus uncommitted changes, built 15:38), K=3 fake
workers, the 6 runs of the round at once and the rogue runs one at a time
(plumbing only, not a benchmark):

| run | tasks | hidden | integration | lost writes | blocked s | pull idle s | claims granted/refused/waited | done s | wall s | unclaimed writes |
|---|---|---|---|---|---|---|---|---|---|---|
| forge baseline | 8/8 | 57/57 | 5/5 | 0 | 91.3 | 0 | 7/82/0 | 66.4 | 66.8 | 0 |
| forge windows | 8/8 | 57/57 | 5/5 | 0 | 5.7 | 4.8 | 37/0/4 | 41.7 | 42.1 | 0 |
| forge symbols | 8/8 | 57/57 | 5/5 | 0 | 4.5 | 5.3 | 37/0/4 | 42.3 | 42.6 | 0 |
| hubs baseline | 8/8 | 48/48 | 4/4 | 0 | 56.6 | 0 | 8/51/0 | 44.3 | 44.7 | 0 |
| hubs windows | 8/8 | 48/48 | 4/4 | 0 | 2.7 | 2.4 | 26/0/2 | 29.8 | 30.1 | 0 |
| hubs symbols | 8/8 | 48/48 | 4/4 | 0 | 2.9 | 0 | 26/0/2 | 28.1 | 28.5 | 0 |
| forge baseline, rogue | 8/8 | 42/57 | 0/5 | 13 | 26.9 | 0 | 6/24/0 | 48.8 | 49.0 | 5 |
| forge symbols, rogue | 8/8 | 42/57 | 0/5 | 15 | 0 | 1.7 | 32/0/0 | 49.4 | 49.6 | 5 |
| hubs baseline, rogue | 8/8 | 18/48 | 0/4 | 33 | 18.2 | 0 | 7/16/0 | 39.7 | 39.9 | 3 |
| hubs symbols, rogue | 8/8 | 18/48 | 0/4 | 33 | 0 | 2.0 | 23/0/0 | 35.9 | 36.2 | 3 |

Every combination finishes 8/8 with full hidden and integration acceptance
and no lost or unclaimed writes; `aggregate.py` over this one round gives
provisional verdicts (R1 ship, R2 rejected at a 12% cut), which says only
that the rules run end to end. Every rogue run (the rogue took `cors` in
Forge and `bulk-discount` in hubs) loses 13 to 33 hidden tests that had
passed at other tasks' done snapshots (hubs: 27 with `attribution:
changed`) and shows the rogue's 5 or 3 writes as unclaimed. Fake times are
seconds, so the arms' done times say nothing about LLM workers.

On the binary before the pull fix (cba7afe) the same round showed why it was
needed: the windows and symbols fakes never waited on a claim because each
`task_pull` `wait_secs` waited behind the previous task's in-progress paths
(pull idle 423-485 s per run), and the last pulls waited 120 s on the empty
board (wall 175-206 s against done 55-86 s).

### Threats specific to the benchmark

- **Claim-aware pulls.** Every Forge feature task lists `app/config.py`,
  `app/router.py` and `app/middlewares/__init__.py`, and every hubs task a
  hub file, so `task_pull` (ADR-0028) sees most candidates as overlapping.
  Which overlaps make a `wait_secs` pull wait is daemon behavior (since the
  pull fix, only live claims and dependencies; an empty board answers at
  once), so freeze the binary once and read `pull_idle_s` next to
  `blocked_s`.
- **Concurrent arms.** Running the arms of a round at once shares the
  machine and the model's rate limits; the rotation spreads start order,
  not load. Record the time of day.
- **Transcripts and usage are the runner's job.** Without them R2's
  failing-test check and R1's cost are undecided or fall back to the
  transcript estimate.
- **Fakes cannot show compliance.** A baseline worker may narrow claims, a
  windows worker may hold a file across a long think, a symbols worker may
  edit outside its anchor. `granted_keys_by_kind`, `unclaimed_writes` and
  `coord_full.jsonl` show some of it.
