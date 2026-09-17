#!/usr/bin/env python3
"""Aggregate coordination-benchmark runs per scenario and arm, and apply the decision rules.

Usage: aggregate.py [--json out.json] [--resamples N] <run_dir or lab_dir>...

A directory with score.json is a run; any other directory contributes its
children that have one. Each run's numbers come from score.json "bench"
(lib/benchstats.py), with the usage columns re-read from usage.tsv so the
file can be filled after scoring. The round comes from <run>/bench.json
(bench/prepare_round.sh writes it) or an `r<N>-` prefix of the run name.

Prints, as Markdown:
  1. per scenario and arm: n, mean, median, and a 95% bootstrap CI of the mean
  2. paired differences by round (same scenario, same round): windows - baseline
     and symbols - windows, mean with a 95% bootstrap CI
  3. the decision rules below, fixed before any real run, and the verdict

Decision rules (pre-registered 2026-09-17, task 5c603223):

  R1  Edit windows + waits (arm "windows", B) ship as the documented protocol
      if, against "baseline" (A), wall time or cost improves and hidden tests
      do not drop:
      - wall time is done_time_s (first coordination call to the last task
        marked done); cost is cost_usd when every paired run has it, else
        total_tokens from usage.tsv, else the transcript estimate cost_units.
      - improves: the mean of the relative paired differences (B - A) / A over
        all (scenario, round) pairs is below 0 and its 95% bootstrap CI lies
        entirely below 0.
      - hidden tests do not drop: the 95% bootstrap CI of the mean paired
        difference in hidden tests passed (B - A) is not entirely below 0,
        and the same for integration tests.
  R2  ADR-0029 (symbol anchors, arm "symbols", C) is Accepted only if, against
      "windows" (B):
      - C cuts agent-blocked time by at least 20%: over all (scenario, round)
        pairs, 1 - sum(C blocked_s) / sum(B blocked_s) >= 0.20, and the 95%
        bootstrap CI of that reduction has a lower bound above 0;
      - no lost writes: lost_writes is 0 in every C run;
      - no extra failing test runs: sum(C failing_test_runs) <=
        sum(B failing_test_runs) over the same pairs.
      Otherwise ADR-0029 is Rejected (anchors are removed). A rule whose
      inputs are missing is reported as undecided, never as passed.
  Both verdicts are provisional with fewer than 3 pairs per scenario.

Bootstrap: percentile intervals from N resamples (default 10000) with a fixed
seed, resampling runs (per-arm summaries) or pairs (differences).
"""

import json
import random
import re
import statistics
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import benchstats  # noqa: E402

ARMS = ("baseline", "windows", "symbols")
METRICS = ("done_time_s", "wall_time_s", "tasks_done", "hidden_passed", "integration_passed", "lost_writes",
           "blocked_s", "pull_idle_s", "waiting_s", "claims_granted", "claims_refused", "claims_waited",
           "turns", "coordination_only_turns", "test_runs", "failing_test_runs", "unclaimed_writes",
           "output_tokens", "total_tokens", "tool_uses", "duration_ms", "cost_usd", "cost_units")
PAIRED = ("done_time_s", "wall_time_s", "hidden_passed", "integration_passed", "lost_writes", "blocked_s",
          "pull_idle_s", "waiting_s", "turns", "coordination_only_turns", "failing_test_runs", "unclaimed_writes",
          "output_tokens", "total_tokens", "cost_usd", "cost_units")
SEED = 20260917
MIN_PAIRS = 3
BLOCKED_CUT = 0.20


def load_runs(paths):
    runs = []
    dirs = []
    for raw in paths:
        path = Path(raw)
        if (path / "score.json").is_file():
            dirs.append(path)
        elif path.is_dir():
            dirs.extend(sorted(p.parent for p in path.glob("*/score.json")))
    seen = set()
    for run in dirs:
        if run.resolve() in seen:
            continue
        seen.add(run.resolve())
        try:
            score = json.loads((run / "score.json").read_text())
        except ValueError:
            print("skipping %s: score.json does not parse" % run, file=sys.stderr)
            continue
        bench = dict(score.get("bench") or {})
        if not bench.get("arm") or not bench.get("scenario"):
            print("skipping %s: no bench block with an arm (score it with the current score.sh)" % run,
                  file=sys.stderr)
            continue
        fresh = benchstats.usage(run)
        if fresh:
            bench.update(fresh)
        info = {}
        if (run / "bench.json").is_file():
            info = json.loads((run / "bench.json").read_text())
        match = re.match(r"r(\d+)-", run.name)
        bench["round"] = info.get("round") if info.get("round") is not None else (int(match.group(1)) if match
                                                                                   else None)
        bench["run"] = run.name
        twin = next((r for r in runs if (r["scenario"], r["round"], r["arm"]) ==
                     (bench["scenario"], bench["round"], bench["arm"]) and bench["round"] is not None), None)
        if twin:
            print("warning: %s and %s are both %s/%s round %s; pairs use %s" % (
                twin["run"], run.name, bench["scenario"], bench["arm"], bench["round"], run.name), file=sys.stderr)
        runs.append(bench)
    return runs


def mean(values):
    return sum(values) / len(values)


def bootstrap(items, stat, resamples, rng):
    """95% percentile interval of ``stat`` over resamples of ``items`` (None when it cannot be computed)."""
    if len(items) < 2:
        return None
    values = []
    for _ in range(resamples):
        sample = [items[rng.randrange(len(items))] for _ in items]
        try:
            value = stat(sample)
        except ZeroDivisionError:
            continue
        if value is not None:
            values.append(value)
    if len(values) < resamples // 2:
        return None
    values.sort()
    return [values[int(0.025 * (len(values) - 1))], values[int(0.975 * (len(values) - 1))]]


def fmt(value, digits=1):
    if value is None:
        return "n/a"
    if isinstance(value, float):
        return ("%%.%df" % digits) % value
    return str(value)


def fmt_ci(ci, digits=1):
    return "n/a" if ci is None else "[%s, %s]" % (fmt(ci[0], digits), fmt(ci[1], digits))


def summarize(runs, resamples):
    rng = random.Random(SEED)
    table = {}
    for scenario in sorted({r["scenario"] for r in runs}):
        for arm in ARMS:
            rows = [r for r in runs if r["scenario"] == scenario and r["arm"] == arm]
            if not rows:
                continue
            out = {"n": len(rows), "runs": [r["run"] for r in rows], "metrics": {}}
            for metric in METRICS:
                values = [float(r[metric]) for r in rows if r.get(metric) is not None]
                if not values:
                    out["metrics"][metric] = None
                    continue
                out["metrics"][metric] = {"n": len(values), "mean": mean(values), "median": statistics.median(values),
                                          "ci": bootstrap(values, mean, resamples, rng)}
            table["%s/%s" % (scenario, arm)] = out
    return table


def pairs(runs, arm_a, arm_b):
    """[(scenario, round, run_a, run_b)] for rounds that have both arms in one scenario."""
    out = []
    index = {(r["scenario"], r["round"], r["arm"]): r for r in runs if r.get("round") is not None}
    for (scenario, round_, arm), run in sorted(index.items(), key=lambda kv: (kv[0][0], kv[0][1], kv[0][2])):
        if arm == arm_a and (scenario, round_, arm_b) in index:
            out.append((scenario, round_, run, index[(scenario, round_, arm_b)]))
    return out


def paired_differences(matched, resamples):
    rng = random.Random(SEED + 1)
    out = {}
    for metric in PAIRED:
        diffs = [(b[metric] - a[metric]) for _, _, a, b in matched if a.get(metric) is not None
                 and b.get(metric) is not None]
        rel = [(b[metric] - a[metric]) / a[metric] for _, _, a, b in matched if a.get(metric)
               and b.get(metric) is not None]
        out[metric] = {"n": len(diffs), "mean": mean(diffs) if diffs else None,
                       "ci": bootstrap(diffs, mean, resamples, rng),
                       "relative_mean": mean(rel) if rel else None,
                       "relative_ci": bootstrap(rel, mean, resamples, rng)}
    return out


def complete(matched, metric):
    return bool(matched) and all(a.get(metric) is not None and b.get(metric) is not None for _, _, a, b in matched)


def rule_windows(matched, resamples):
    rng = random.Random(SEED + 2)
    out = {"rule": "R1", "pairs": len(matched), "checks": {}}
    if not matched:
        out["verdict"] = "undecided: no round has both baseline and windows"
        return out
    cost = next((m for m in ("cost_usd", "total_tokens", "cost_units") if complete(matched, m)), None)
    improved = []
    for metric in ("done_time_s", cost):
        if metric is None:
            out["checks"]["cost"] = {"result": "undecided", "why": "no cost_usd, total_tokens or cost_units in "
                                                                   "every paired run"}
            continue
        rel = [(b[metric] - a[metric]) / a[metric] for _, _, a, b in matched if a.get(metric)
               and b.get(metric) is not None]
        ci = bootstrap(rel, mean, resamples, rng)
        ok = bool(rel) and ci is not None and mean(rel) < 0 and ci[1] < 0
        improved.append(ok)
        out["checks"][metric] = {"relative_mean": mean(rel) if rel else None, "ci": ci,
                                 "result": "improves" if ok else "no significant improvement"}
    quality = True
    for metric in ("hidden_passed", "integration_passed"):
        diffs = [b[metric] - a[metric] for _, _, a, b in matched if a.get(metric) is not None
                 and b.get(metric) is not None]
        ci = bootstrap(diffs, mean, resamples, rng)
        if not diffs:
            out["checks"][metric] = {"result": "undecided"}
            quality = None
            continue
        drop = ci is not None and ci[1] < 0
        if ci is None and mean(diffs) < 0:
            drop = True  # a single pair cannot show noise; a lower count is a drop
        out["checks"][metric] = {"mean": mean(diffs), "ci": ci, "result": "drops" if drop else "no drop"}
        if drop:
            quality = False
    if quality is None:
        out["verdict"] = "undecided: hidden or integration tests missing"
    elif any(improved) and quality:
        out["verdict"] = "SHIP windows as the documented protocol"
    else:
        out["verdict"] = "do not ship: " + ("hidden tests drop" if not quality else "no significant improvement")
    return out


def rule_symbols(matched, resamples):
    rng = random.Random(SEED + 3)
    out = {"rule": "R2", "pairs": len(matched), "checks": {}}
    if not matched:
        out["verdict"] = "undecided: no round has both windows and symbols"
        return out
    undecided = []
    if complete(matched, "blocked_s"):
        def reduction(sample):
            base = sum(b["blocked_s"] for _, _, b, _ in sample)
            if base <= 0:
                return None
            return 1 - sum(c["blocked_s"] for _, _, _, c in sample) / base

        point = reduction(matched)
        ci = bootstrap(matched, reduction, resamples, rng)
        ok = point is not None and point >= BLOCKED_CUT and ci is not None and ci[0] > 0
        out["checks"]["blocked_s"] = {
            "windows_total": sum(b["blocked_s"] for _, _, b, _ in matched),
            "symbols_total": sum(c["blocked_s"] for _, _, _, c in matched),
            "reduction": point, "ci": ci,
            "result": "cuts >= 20%" if ok else ("windows had no blocked time" if point is None else "cut < 20% or CI "
                                                                                                   "includes 0")}
        blocked_ok = ok
    else:
        undecided.append("blocked_s")
        blocked_ok = False
    lost = [c.get("lost_writes") for _, _, _, c in matched]
    if all(v is not None for v in lost):
        lost_ok = sum(lost) == 0
        out["checks"]["lost_writes"] = {"symbols_total": sum(lost),
                                        "windows_total": sum(b.get("lost_writes") or 0 for _, _, b, _ in matched),
                                        "result": "none" if lost_ok else "lost writes"}
    else:
        undecided.append("lost_writes")
        lost_ok = False
    if complete(matched, "failing_test_runs"):
        b_fail = sum(b["failing_test_runs"] for _, _, b, _ in matched)
        c_fail = sum(c["failing_test_runs"] for _, _, _, c in matched)
        fail_ok = c_fail <= b_fail
        out["checks"]["failing_test_runs"] = {"windows_total": b_fail, "symbols_total": c_fail,
                                              "result": "no extra" if fail_ok else "extra failing runs"}
    else:
        undecided.append("failing_test_runs")
        fail_ok = False
    for name in undecided:
        out["checks"][name] = {"result": "undecided: missing in some paired run"}
    failed = [name for name, ok in (("blocked_s", blocked_ok), ("lost_writes", lost_ok),
                                    ("failing_test_runs", fail_ok)) if not ok and name not in undecided]
    if failed:
        out["verdict"] = "ADR-0029 Rejected: " + ", ".join(failed)
    elif undecided:
        out["verdict"] = "undecided: missing " + ", ".join(undecided)
    else:
        out["verdict"] = "ADR-0029 Accepted"
    return out


def provisional(runs, matched_sets):
    notes = []
    for scenario in sorted({r["scenario"] for r in runs}):
        for name, matched in matched_sets:
            n = sum(1 for m in matched if m[0] == scenario)
            if n < MIN_PAIRS:
                notes.append("%s: %d %s pairs (< %d)" % (scenario, n, name, MIN_PAIRS))
    return notes


def rounded(value):
    if isinstance(value, float):
        return round(value, 3)
    if isinstance(value, dict):
        return {k: rounded(v) for k, v in value.items()}
    if isinstance(value, list):
        return [rounded(v) for v in value]
    return value


def print_report(runs, table, diffs, verdicts, notes):
    print("# Coordination benchmark aggregate\n")
    print("%d runs: %s\n" % (len(runs), ", ".join(sorted(r["run"] for r in runs))))
    print("## Per scenario and arm (mean, median, 95% bootstrap CI of the mean)\n")
    shown = ("done_time_s", "wall_time_s", "hidden_passed", "integration_passed", "lost_writes", "blocked_s",
             "pull_idle_s", "waiting_s", "claims_granted", "claims_refused", "claims_waited", "turns",
             "coordination_only_turns", "test_runs", "failing_test_runs", "unclaimed_writes", "output_tokens",
             "total_tokens", "tool_uses", "duration_ms", "cost_usd", "cost_units")
    for key, row in table.items():
        print("### %s (n=%d)\n" % (key, row["n"]))
        print("| metric | n | mean | median | 95% CI |")
        print("|---|---|---|---|---|")
        for metric in shown:
            m = row["metrics"].get(metric)
            if m is None:
                print("| %s | 0 | n/a | n/a | n/a |" % metric)
            else:
                print("| %s | %d | %s | %s | %s |" % (metric, m["n"], fmt(m["mean"]), fmt(m["median"]),
                                                      fmt_ci(m["ci"])))
        print()
    for name, rows in diffs.items():
        print("## Paired differences: %s (n pairs = %d)\n" % (name, rows["pairs"]))
        print("| metric | n | mean diff | 95% CI | mean relative | 95% CI |")
        print("|---|---|---|---|---|---|")
        for metric, d in rows["metrics"].items():
            print("| %s | %d | %s | %s | %s | %s |" % (metric, d["n"], fmt(d["mean"]), fmt_ci(d["ci"]),
                                                       fmt(d["relative_mean"], 3), fmt_ci(d["relative_ci"], 3)))
        print()
    print("## Decision rules (pre-registered)\n")
    doc = __doc__
    print("```text\n" + doc[doc.index("Decision rules"):doc.index("Bootstrap:")].rstrip() + "\n```\n")
    print("## Verdicts\n")
    for verdict in verdicts:
        print("- %s (%d pairs): **%s**" % (verdict["rule"], verdict["pairs"], verdict["verdict"]))
        for check, value in verdict["checks"].items():
            print("  - %s: %s" % (check, json.dumps(rounded(value))))
    if notes:
        print("\nProvisional (fewer than %d pairs): %s" % (MIN_PAIRS, "; ".join(notes)))


def main():
    args = sys.argv[1:]
    out_json, resamples = None, 10000
    while args and args[0].startswith("--"):
        if args[0] == "--json" and len(args) > 1:
            out_json, args = args[1], args[2:]
        elif args[0] == "--resamples" and len(args) > 1:
            resamples, args = int(args[1]), args[2:]
        else:
            raise SystemExit("usage: aggregate.py [--json out.json] [--resamples N] <run_dir or lab_dir>...")
    if not args:
        raise SystemExit("usage: aggregate.py [--json out.json] [--resamples N] <run_dir or lab_dir>...")
    runs = load_runs(args)
    if not runs:
        raise SystemExit("no scored benchmark runs found")
    table = summarize(runs, resamples)
    ab, bc = pairs(runs, "baseline", "windows"), pairs(runs, "windows", "symbols")
    diffs = {"windows - baseline": {"pairs": len(ab), "metrics": paired_differences(ab, resamples)},
             "symbols - windows": {"pairs": len(bc), "metrics": paired_differences(bc, resamples)}}
    verdicts = [rule_windows(ab, resamples), rule_symbols(bc, resamples)]
    notes = provisional(runs, (("windows-baseline", ab), ("symbols-windows", bc)))
    for verdict in verdicts:
        if notes and not verdict["verdict"].startswith("undecided"):
            verdict["verdict"] += " (provisional)"
    print_report(runs, table, diffs, verdicts, notes)
    if out_json:
        Path(out_json).write_text(json.dumps({"runs": runs, "per_arm": table, "paired": diffs, "verdicts": verdicts,
                                              "provisional": notes}, indent=2, default=str) + "\n")


if __name__ == "__main__":
    main()
