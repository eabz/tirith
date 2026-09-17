#!/usr/bin/env bash
# run_fake.sh <run_dir> <port> [workers=3] [--rogue] [setup.sh options...]
#
# Plumbing check of the manual protocol without LLMs: setup.sh with the given
# options (e.g. --scenario hubs --arm symbols), K scripted fake workers in
# parallel (fake_worker_forge.py or fake_worker_hubs.py by scenario; with
# --rogue the last one claims nothing and overwrites stale copies), score,
# stop, and a one-line summary. Workers are child processes of one
# foreground python, which waits for all of them.
set -euo pipefail
usage() { echo "usage: $0 <run_dir> <port> [workers] [--rogue] [setup.sh options...]" >&2; exit 2; }
[ $# -ge 2 ] || usage
KIT=$(cd "$(dirname "$0")/.." && pwd)
RUN=$1 PORT=$2
shift 2
K=3 ROGUE=
if [ $# -gt 0 ] && [ "${1#--}" = "$1" ]; then K=$1; shift; fi
if [ $# -gt 0 ] && [ "$1" = --rogue ]; then ROGUE=1; shift; fi
case "$K" in ''|*[!0-9]*) usage ;; esac
"$KIT/setup.sh" "$RUN" "$PORT" "$@"
trap '"$KIT/stop.sh" "$RUN"' EXIT
RUN=$(cd "$RUN" && pwd)
case "$(jq -r .scenario "$RUN/meta.json")" in
  forge) FAKE=$KIT/dryrun/fake_worker_forge.py ;;
  hubs) FAKE=$KIT/dryrun/fake_worker_hubs.py ;;
  *) echo "no fake worker for this scenario" >&2; exit 1 ;;
esac
python3 - "$FAKE" "$RUN" "$K" "$ROGUE" <<'PY'
import subprocess, sys
fake, run, k, rogue = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4] == "1"
procs = []
for i in range(1, k + 1):
    args = ["python3", fake, run, "fake%d" % i] + (["--rogue"] if rogue and i == k else [])
    procs.append(subprocess.Popen(args))
codes = [p.wait() for p in procs]
sys.exit(1 if any(codes) else 0)
PY
"$KIT/score.sh" "$RUN" > /dev/null
jq -c '{scenario, arm, tasks: "\(.tasks.done)/\(.tasks.total)", bench}' "$RUN/score.json"
echo "score: $RUN/score.json"
