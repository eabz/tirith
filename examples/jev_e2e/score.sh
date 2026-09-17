#!/usr/bin/env bash
# score.sh <run_dir> -- print the run's JSON summary (also saved to <run_dir>/score.json).
# Run it after the workers finish and before stop.sh: task states and the
# Jev counters are read from the live daemon.
set -euo pipefail
[ $# -eq 1 ] || { echo "usage: $0 <run_dir>" >&2; exit 2; }
KIT=$(cd "$(dirname "$0")" && pwd)
RUN=$(cd "$1" && pwd)
python3 "$KIT/lib/score.py" "$KIT" "$RUN" | tee "$RUN/score.json"
