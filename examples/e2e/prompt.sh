#!/usr/bin/env bash
# prompt.sh <run_dir> <agent> -- print the worker prompt for one worker of a
# run prepared by setup.sh (scenario, claims and wait arm come from meta.json).
set -euo pipefail
[ $# -eq 2 ] || { echo "usage: $0 <run_dir> <agent>" >&2; exit 2; }
KIT=$(cd "$(dirname "$0")" && pwd)
python3 "$KIT/lib/prompt.py" "$1" "$2"
