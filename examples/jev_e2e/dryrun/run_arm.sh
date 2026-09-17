#!/usr/bin/env bash
# run_arm.sh <run_dir> <port> <off|jev> [workers=3] -- plumbing check without
# LLMs: setup, K scripted fake workers in parallel, score, stop.
set -euo pipefail
[ $# -ge 3 ] || { echo "usage: $0 <run_dir> <port> <off|jev> [workers]" >&2; exit 2; }
KIT=$(cd "$(dirname "$0")/.." && pwd)
RUN=$1 PORT=$2 ARM=$3 K=${4:-3}
"$KIT/setup.sh" "$RUN" "$PORT" "$ARM"
trap '"$KIT/stop.sh" "$RUN"' EXIT
for i in $(seq 1 "$K"); do "$KIT/dryrun/fake_worker.sh" "$RUN" "fake$i" & done
wait
"$KIT/score.sh" "$RUN" > /dev/null
echo "score: $RUN/score.json"
