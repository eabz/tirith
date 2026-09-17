#!/usr/bin/env bash
# run_forge.sh <run_dir> <port> [workers=3] -- plumbing check of the Forge
# scenario without LLMs: setup, K scripted fake workers in parallel, score, stop.
set -euo pipefail
[ $# -ge 2 ] || { echo "usage: $0 <run_dir> <port> [workers]" >&2; exit 2; }
KIT=$(cd "$(dirname "$0")/.." && pwd)
RUN=$1 PORT=$2 K=${3:-3}
"$KIT/setup.sh" "$RUN" "$PORT"
trap '"$KIT/stop.sh" "$RUN"' EXIT
for i in $(seq 1 "$K"); do "$KIT/dryrun/fake_worker.sh" "$RUN" "fake$i" & done
wait
"$KIT/score.sh" "$RUN" > /dev/null
echo "score: $RUN/score.json"
