#!/usr/bin/env bash
# fake_round.sh <lab_dir> <round>
#
# Dry run of a round prepared by bench/prepare_round.sh, without LLMs: starts
# WORKERS (default 3) scripted fake workers for every run of the round at once
# (fake_worker_forge.py or fake_worker_hubs.py; synthetic transcripts go to
# <run>/transcripts/), waits for all of them, runs the watchdog once, then
# scores and stops each run and prints one summary line per run. Workers are
# child processes of one foreground python that waits for all of them.
set -euo pipefail
[ $# -eq 2 ] || { echo "usage: $0 <lab_dir> <round>" >&2; exit 2; }
KIT=$(cd "$(dirname "$0")/.." && pwd)
LAB=$(cd "$1" && pwd)
K=${WORKERS:-3}
RUNS=()
for run in "$LAB"/r"$2"-*/; do [ -f "$run/meta.json" ] && RUNS+=("${run%/}"); done
[ ${#RUNS[@]} -gt 0 ] || { echo "no prepared runs for round $2 in $LAB" >&2; exit 1; }
trap 'for run in "${RUNS[@]}"; do "$KIT/stop.sh" "$run" > /dev/null || true; done' EXIT
python3 - "$KIT" "$K" "${RUNS[@]}" <<'PY'
import json, subprocess, sys
from pathlib import Path
kit, k, runs = Path(sys.argv[1]), int(sys.argv[2]), sys.argv[3:]
fakes = {"forge": "fake_worker_forge.py", "hubs": "fake_worker_hubs.py"}
procs = []
for run in runs:
    scenario = json.loads((Path(run) / "meta.json").read_text())["scenario"]
    for i in range(1, k + 1):
        log = open(str(Path(run) / ("fake-w%d.log" % i)), "w")
        procs.append(subprocess.Popen(["python3", str(kit / "dryrun" / fakes[scenario]), run, "w%d" % i],
                                      stdout=log, stderr=subprocess.STDOUT))
codes = [p.wait() for p in procs]
sys.exit(1 if any(codes) else 0)
PY
python3 "$KIT/bench/watch.py" --max-secs 5 --interval 1 "${RUNS[@]}" || true
for run in "${RUNS[@]}"; do
  "$KIT/score.sh" "$run" > /dev/null
  "$KIT/stop.sh" "$run" > /dev/null
  jq -c '.bench | {run: (input_filename | split("/") | .[-2]), tasks: "\(.tasks_done)/\(.tasks_total)",
    hidden: "\(.hidden_passed)/\(.hidden_total)", integration: "\(.integration_passed)/\(.integration_total)",
    lost_writes, blocked_s, pull_idle_s, claims: "\(.claims_granted) granted/\(.claims_refused) refused/\(.claims_waited) waited",
    done_time_s, wall_time_s, turns, coordination_only_turns, failing_test_runs, unclaimed_writes}' "$run/score.json"
done
trap - EXIT
