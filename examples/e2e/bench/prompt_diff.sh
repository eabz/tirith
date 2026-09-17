#!/usr/bin/env bash
# prompt_diff.sh <run_a> <run_b> [agent=w1] -- show how two runs' worker
# prompts differ, and fail unless they differ only in the protocol paragraphs.
#
# 1. The templates with only RUN_DIR and AGENT_NAME filled in (the
#    {{CLAIMS}} {{HOLD}} {{WAIT}} {{PULL}} markers left in place) must be
#    byte-identical once each run directory is replaced by RUN.
# 2. Prints which paragraphs the arms select and the unified diff of the
#    rendered prompts (run directories replaced by RUN).
# Exit 0 when (1) holds, 1 otherwise.
set -euo pipefail
[ $# -ge 2 ] && [ $# -le 3 ] || { echo "usage: $0 <run_a> <run_b> [agent]" >&2; exit 2; }
KIT=$(cd "$(dirname "$0")/.." && pwd)
A=$(cd "$1" && pwd) B=$(cd "$2" && pwd) AGENT=${3:-w1}
render() { python3 "$KIT/lib/prompt.py" "$1" "$AGENT" $2 | python3 -c 'import sys; sys.stdout.write(sys.stdin.read().replace(sys.argv[1], "RUN"))' "$1"; }
python3 - "$A/meta.json" "$B/meta.json" <<'PY'
import json, sys
a, b = (json.load(open(p)) for p in sys.argv[1:3])
for key in ("scenario", "protocol", "arm", "claims", "hold", "wait", "pull"):
    mark = "" if a.get(key) == b.get(key) else "   <- differs"
    print("%-9s %-10s %-10s%s" % (key, a.get(key), b.get(key), mark))
PY
if ! diff <(render "$A" --skeleton) <(render "$B" --skeleton) > /dev/null; then
  echo "FAIL: the prompts differ outside the protocol paragraphs:" >&2
  diff <(render "$A" --skeleton) <(render "$B" --skeleton) >&2 || true
  exit 1
fi
echo "OK: identical outside the protocol paragraphs; rendered diff (RUN = run directory):"
diff -u --label "$A" --label "$B" <(render "$A" "") <(render "$B" "") || true
