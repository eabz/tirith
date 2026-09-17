#!/usr/bin/env bash
# run_hubs.sh <run_dir> <port> <file|symbol> [workers=3] [--wait sleep|server] [--rogue]
# Plumbing check of the hubs scenario without LLMs: setup, K
# scripted workers in parallel (the last one rogue with --rogue), score, stop.
set -euo pipefail
usage() { echo "usage: $0 <run_dir> <port> <file|symbol> [workers] [--wait sleep|server] [--rogue]" >&2; exit 2; }
[ $# -ge 3 ] || usage
KIT=$(cd "$(dirname "$0")/.." && pwd)
RUN=$1 PORT=$2 CLAIMS=$3
shift 3
K=3 WAIT=server ROGUE=
if [ $# -gt 0 ] && [ "${1#--}" = "$1" ]; then K=$1; shift; fi
while [ $# -gt 0 ]; do
  case "$1" in
    --wait) [ $# -ge 2 ] || usage; WAIT=$2; shift 2 ;;
    --rogue) ROGUE=1; shift ;;
    *) usage ;;
  esac
done
"$KIT/setup.sh" "$RUN" "$PORT" --scenario hubs --claims "$CLAIMS" --wait "$WAIT"
trap '"$KIT/stop.sh" "$RUN"' EXIT
PIDS=()
for i in $(seq 1 "$K"); do
  if [ -n "$ROGUE" ] && [ "$i" -eq "$K" ]; then
    python3 "$KIT/dryrun/fake_worker_hubs.py" "$RUN" "fake$i" --rogue & PIDS+=($!)
  else
    python3 "$KIT/dryrun/fake_worker_hubs.py" "$RUN" "fake$i" & PIDS+=($!)
  fi
done
for pid in "${PIDS[@]}"; do wait "$pid"; done
"$KIT/score.sh" "$RUN" > /dev/null
echo "score: $RUN/score.json"
