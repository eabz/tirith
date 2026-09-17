# Jev micro-benchmark

Runs Tirith on hand-labeled cases twice, once with the experimental Jev
sites off (`off`) and once on (`jev`, ADR-0024), and scores each
arm against the same labels. It measures whether Jev's judgment at each
site beats Tirith's deterministic behavior, and what that costs in latency,
bytes, and money. A result showing Jev does not help counts as a result.

## Running

```bash
# validate the case files only (no daemon, no network)
cargo run --release --example jev_bench -- --validate

# the full benchmark, both arms, Vercel AI Gateway
cargo run --release --example jev_bench

# a subset, three repeats, TypeSafe direct, custom output directory
cargo run --release --example jev_bench -- --sites brief,memory_search \
  --limit 10 --repeat 3 --provider typesafe --out /tmp/jev-run

# the smoke cases (2 per site) used to check the harness itself
cargo run --release --example jev_bench -- --cases examples/jev_bench/smoke
```

| flag | default | meaning |
|---|---|---|
| `--cases <dir>` | `examples/jev_bench/cases` | every `*.json` in the directory |
| `--sites a,b` | all | only these sites |
| `--limit N` | none | the first N cases per site, in file order |
| `--repeat R` | 1 | run every case R times per arm |
| `--provider` | `gateway` | `gateway` or `typesafe`, passed to `JevConfig::from_env` as `TIRITH_JEV_PROVIDER` |
| `--arms` | `off,jev` | either or both |
| `--out <dir>` | `target/jev_bench/<timestamp>` | where results go |
| `--validate` | off | load and check cases, then exit |

Keys come from the environment or the repository's `.env`
(`AI_GATEWAY_API_KEY` or `TYPESAFE_API_KEY`) and are never printed. The
`off` arm alone needs no key and makes no network call.

## What one case does

Every case, arm, and repeat gets a fresh temporary directory and its own
in-process daemon on an ephemeral port.

1. **Seed** in the order the schema fixes: memory, decisions, contracts,
   notices, tasks, task states, claims (`brief: false`, one-hour TTL), then
   one `status` call per `active_agents` name. Jev is still off while
   seeding, so both arms start from identical state and seeding costs no
   Jev calls. A task's `pulled_by` and `done_by` are replayed with
   `task_update` (`in_progress` as `pulled_by`, then `done` as `done_by`)
   on that exact task, never with `task_pull`, which could pick another.
2. **Switch Jev on** for the `jev` arm. The process creates one Jev
   client, warms up its TLS connection once, and shares it between all
   daemons, so the first case does not pay the handshake.
3. **Act**: the labeled tool call, with `agent` added. Its wall time is
   measured client side over an MCP session that is already open, along
   with the bytes of its structured response.
4. **Read the prediction** and score it (table below). For
   `notice_fanout`, every claim holder other than the publisher is polled
   with `status` every 100 ms for 3 s, in both arms so both arms wait the
   same way. That polling is not counted in the action latency. The
   notice push runs in the background, so its own delay is reported as
   `slowest_push_ms_mean`.
5. **Jev usage** is the change in `ServerHandle::assist_report` from just
   before the action to after it, including the polling window: calls,
   failures, questions, input tokens, cost, and Jev round-trip time.

Arms alternate which one runs first from case to case.

## Metrics

All metrics are computed per site and per arm, and per repeat when
`--repeat` is above 1. With repeats, a cell shows the mean and the
(min–max) range. Every site also reports `bytes_mean` and
`tokens_est_mean`, plus p50 and p95 of the action latency.

**Bytes and tokens.** `bytes` is the length of the action's JSON
structured content, which is what an agent reads. `tokens_est = bytes / 4`
approximates tokens for English and JSON. It is not a tokenizer count, so
compare arms with it, but don't read it as an absolute cost.

| site | prediction | metrics |
|---|---|---|
| `brief` | keys of the rows the claim brief shows (notice id prefix, contract name, decision id, memory permalink) | `recall_macro` (headline; mean over cases with a non-empty `must_read`), `recall_micro`, `all_must_read_shown_rate`, `precision_micro`, `precision_macro`, `rows_shown_per_empty_label_case`, `rows_shown_mean`, `unkeyed_rows_shown_total` (rows with no labeler key, such as an automatic contract notice; excluded from precision and recall), `size_cap_dropped_rate` (the 4,096-byte cap cut a section: fewer than 5 rows shown while `more` > 0) |
| `notice_fanout` | holders that got a push from `tirith` within 3 s | `precision_micro`, `recall_micro`, `f1_micro` (headline, 2·TP / (predicted + labeled), so an arm that pushes nothing scores 0), `recall_macro`, `recipients_mean`, `slowest_push_ms_mean`, `false_positives_per_empty_label_case` |
| `broadcast` | `message.audience` | same as `notice_fanout` except push latency. Without Jev, the audience is everyone |
| `memory_search` | ranked note keys | `recall_at5_macro` (headline), `precision_at5_macro` (relevant rows among the top-5 rows actually returned; undefined when nothing is returned), `precision_at5_lenient_macro` (`partial` counts as relevant), `mrr`, `empty_result_rate`, `false_positives_per_empty_label_case` (top-5 rows returned when nothing is relevant), `rows_mean` |
| `decision_search` | ranked decision keys | as `memory_search`, plus `recall_all_rows_macro` and `precision_all_rows_micro` over every returned row. Without Jev, this is a substring match |
| `task_duplicate`, `note_duplicate` | `possible_duplicate` / `similar_note` key, or null | `accuracy` (headline; a prediction must equal the label, and null equal to null counts), `precision`, `recall`, `false_positive_rate` (flagged among non-duplicates), `wrong_target_count` (flagged a duplicate, but the wrong one) |
| `task_pull` | the pulled task key | `top1_best_rate` (headline), `top1_acceptable_rate`, `jev_pick_rate`, `nothing_pulled_rate`. Without Jev, the oldest task wins |
| `conflict_advice` | `advice.action`, or none | `acceptable_rate` (headline; over all cases, where no advice counts as unacceptable unless the label lists `none`), `coverage`, `acceptable_given_advice` |

Precision on a case with no relevant item is undefined. Those cases are
left out of the macro precision and recall and reported as
`false_positives_per_empty_label_case` instead.

**Confidence intervals.** For each site's headline metric: a 95%
percentile bootstrap over cases, with 1000 resamples and a fixed seed
(xorshift, no dependency). Each resample draws case ids with replacement
and scores all of their rows, across every repeat. The delta interval is
paired: the same resampled cases score both arms. With 15–30 cases per
site, expect wide intervals.

**Failures.** A `jev` row with `jev_failed` had at least one Jev
evaluation fail, so the site fell back to deterministic behavior. Such
cases are listed under "Fell back to baseline" per site. `jev_no_call_cases`
counts cases where Tirith never asked Jev, for example `task_pull` with no
agent context, which is by design. Rows with a harness or daemon error are
excluded from the metrics and listed in `results.json`.

## Output

- `per_case.jsonl`: one row per case, arm, and repeat, written as the run
  goes. Each row has the prediction, labels, per-case counts, latency,
  bytes, Jev usage, warnings, and any error.
- `results.json`: `meta` (provider, repeats, cases per site, invalid
  cases, case warnings), `aggregates` (per site: metrics per arm, latency
  percentiles, Jev totals, fall-backs, errors, headline CIs), and every row.
- `results.md`: one table per site (metric | off | jev | delta) with n,
  the headline CIs, and Jev cost and failures. It opens with the premise
  line: labels written independently of Jev; thresholds from
  `src/assist.rs` unchanged.

## Case files

`cases/<site>.json` holds the benchmark and `smoke/` holds harness
self-checks. The format is the lab schema: `site`, then `cases[]` with
`id`, `difficulty`, `why`, `seed`, `action {agent, tool, args}`, `expect`.
`--validate` checks that each action's tool matches its site, that
required seed fields are present, that keys are unique, that every label
refers to a seeded key or agent, and that no seeded memory titles collide
(a collision would update a note instead of creating one). It warns when
a broadcast case's seed authors will join the audience without being
listed as active.

## Known limitations

- **One labeler per case.** Labels are that labeler's judgment and were
  written without seeing Jev's output. No inter-rater agreement was
  measured, and labels flagged as uncertain are included as written.
- **Small n.** 15–30 cases per site. Read the CIs before reading the
  deltas.
- **Latency comes from one machine and one network path**, measured to
  the provider this run used, gateway or direct. The `off` arm includes
  the daemon's wait for disk persistence on mutating calls, and the `jev`
  arm adds a network round trip. Neither number is a production figure.
- **Tokens are estimated** as bytes / 4. The benchmark does not measure
  what an agent does with a shorter brief, such as a missed notice that
  leads to rework.
- **Synthetic state.** Every case starts from a fresh daemon with a small
  seed. In real repositories, briefs and searches choose among more
  candidates, and agents' activity histories are longer.
- **Seeding marks authors as seen.** Every seeding call counts its author
  as active, so seed authors can land in a broadcast audience even when
  a labeler did not think of them as active. `--validate` warns about this.
- **Jev may be non-deterministic.** Use `--repeat` and read the min–max
  range.
- **Thresholds are frozen** at the values in `src/assist.rs`. This
  benchmark evaluates that configuration and does not tune it.
