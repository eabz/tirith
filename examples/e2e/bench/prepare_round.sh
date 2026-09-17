#!/usr/bin/env bash
# prepare_round.sh <lab_dir> <round> [base_port]
#
# Sets up one round of the coordination benchmark: 6 runs (arms baseline,
# windows, symbols x scenarios forge, hubs) named <lab_dir>/r<round>-<scenario>-<arm>,
# each with its own daemon on port base_port+i (default 7900 + 10*round),
# in an order that rotates by round. Renders the worker prompts to
# <lab_dir>/prompts/<run>/w1.md .. wK.md, checks that prompts differ only in
# the protocol paragraphs (bench/prompt_diff.sh, diffs saved next to the
# prompts), and prints the runbook: spawn order, watchdog, score, stop,
# usage and transcripts, aggregate. Spawns nothing.
#
# Env: TIRITH_BIN (required: one frozen binary for every arm and round, see
# bench/freeze_bin.sh), WORKERS (default 3).
# If any setup fails, the runs this call already started are stopped.
set -euo pipefail
[ $# -ge 2 ] && [ $# -le 3 ] || { echo "usage: $0 <lab_dir> <round> [base_port]" >&2; exit 2; }
KIT=$(cd "$(dirname "$0")/.." && pwd)
ROUND=$2
case "$ROUND" in ''|*[!0-9]*) echo "round must be a number" >&2; exit 2 ;; esac
BASE=${3:-$((7900 + 10 * ROUND))}
K=${WORKERS:-3}
: "${TIRITH_BIN:?set TIRITH_BIN to the frozen binary (bench/freeze_bin.sh)}"
[ -f "$TIRITH_BIN.build.json" ] || echo "WARNING: no $TIRITH_BIN.build.json; meta.json will not record the commit" >&2
mkdir -p "$1"
LAB=$(cd "$1" && pwd)

# Arms rotate by round (a Latin square over 3 rounds); the scenario that goes
# first alternates. Order is the setup order and the suggested spawn order.
ARMS=(baseline windows symbols)
SCENARIOS=(forge hubs)
[ $((ROUND % 2)) -eq 0 ] && SCENARIOS=(hubs forge)
ORDER=()
for i in 0 1 2; do
  ARM=${ARMS[$(( (i + ROUND - 1) % 3 ))]}
  for SCENARIO in "${SCENARIOS[@]}"; do ORDER+=("$SCENARIO:$ARM"); done
done

STARTED=()
cleanup() {
  rc=$?
  if [ $rc -ne 0 ] && [ ${#STARTED[@]} -gt 0 ]; then
    echo "setup failed; stopping the runs started so far" >&2
    for run in "${STARTED[@]}"; do "$KIT/stop.sh" "$run" >&2 || true; done
  fi
}
trap cleanup EXIT

for run in "${ORDER[@]}"; do
  NAME="r$ROUND-${run%%:*}-${run##*:}"
  [ ! -e "$LAB/$NAME" ] || { echo "$LAB/$NAME exists; pick a new round or lab" >&2; exit 1; }
done

INDEX=0
for run in "${ORDER[@]}"; do
  SCENARIO=${run%%:*} ARM=${run##*:}
  NAME="r$ROUND-$SCENARIO-$ARM" PORT=$((BASE + INDEX))
  RUN="$LAB/$NAME"
  echo "== $NAME (port $PORT)"
  "$KIT/setup.sh" "$RUN" "$PORT" --scenario "$SCENARIO" --arm "$ARM" | sed 's/^/   /'
  STARTED+=("$RUN")
  jq -n --argjson round "$ROUND" --argjson order "$INDEX" --arg scenario "$SCENARIO" --arg arm "$ARM" \
    --argjson port "$PORT" --argjson workers "$K" --arg at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    '{round:$round, order:$order, scenario:$scenario, arm:$arm, port:$port, workers:$workers, prepared_at:$at}' \
    > "$RUN/bench.json"
  mkdir -p "$LAB/prompts/$NAME"
  for w in $(seq 1 "$K"); do "$KIT/prompt.sh" "$RUN" "w$w" > "$LAB/prompts/$NAME/w$w.md"; done
  INDEX=$((INDEX + 1))
done

echo "== prompt check (identical outside the protocol paragraphs)"
for SCENARIO in forge hubs; do
  for pair in baseline:windows windows:symbols; do
    A="$LAB/r$ROUND-$SCENARIO-${pair%%:*}" B="$LAB/r$ROUND-$SCENARIO-${pair##*:}"
    OUT="$LAB/prompts/r$ROUND-$SCENARIO-${pair%%:*}-vs-${pair##*:}.diff"
    if "$KIT/bench/prompt_diff.sh" "$A" "$B" > "$OUT"; then
      echo "   ok  $SCENARIO ${pair%%:*} vs ${pair##*:}: $(grep -c '^[-+][^-+]' "$OUT") changed lines ($OUT)"
    else
      echo "   FAIL $SCENARIO ${pair%%:*} vs ${pair##*:}: see $OUT" >&2
      exit 1
    fi
  done
done
trap - EXIT

RUNS=()
for run in "${ORDER[@]}"; do RUNS+=("$LAB/r$ROUND-${run%%:*}-${run##*:}"); done
cat <<EOF

== Runbook, round $ROUND ($(jq -r .version "$TIRITH_BIN.build.json" 2>/dev/null || "$TIRITH_BIN" --version), commit $(jq -r .commit "$TIRITH_BIN.build.json" 2>/dev/null || echo unknown))

1. Spawn $K workers per run, same model, effort and tool permissions everywhere,
   working directory <run>/repo, each with the text of its prompt file.
   Name them <run>/w<i> in your notes (the usage rows need run and worker).
   Suggested order (interleaved; start one run's workers together):
EOF
for i in "${!RUNS[@]}"; do
  NAME=$(basename "${RUNS[$i]}")
  echo "   $((i + 1)). $NAME  prompts: $LAB/prompts/$NAME/w1.md .. w$K.md"
done
cat <<EOF

2. Watch (foreground, exits 0 when every run is done, 2 to rerun, 3 on a stall):
   python3 $KIT/bench/watch.py ${RUNS[*]}

3. When all workers of a run have reported, in this order:
   a. append one row per worker to $LAB/usage.tsv from its completion notice
      (tab-separated; write the header once per lab; output_tokens and
      cost_usd may stay empty):
      run	worker	total_tokens	tool_uses	duration_ms	output_tokens	cost_usd
   b. copy each worker's transcript to <run>/transcripts/w<i>.jsonl (Agent-tool
      workers: ~/.claude/projects/<project>/<session>/subagents/agent-<id>.jsonl);
      it gives turns, coordination-only turns, failing test runs, unclaimed writes
   c. score while the daemon is up (task states come from it), then stop it:
EOF
for run in "${RUNS[@]}"; do echo "      $KIT/score.sh $run > /dev/null && $KIT/stop.sh $run"; done
cat <<EOF
   Usage rows for this round:
EOF
for run in "${RUNS[@]}"; do
  for w in $(seq 1 "$K"); do printf '      %s\tw%s\t\t\t\t\t\n' "$(basename "$run")" "$w"; done
done
cat <<EOF
   A usage row added later is still picked up: aggregate.py rereads usage.tsv.

4. Check the round: every run 8/8 tasks, no stuck worker, daemons stopped:
   jq -c '.bench | {scenario, arm, tasks_done, done_time_s, hidden_passed, lost_writes, blocked_s, pull_idle_s}' $LAB/r$ROUND-*/score.json

5. After the last round:
   python3 $KIT/lib/aggregate.py --json $LAB/aggregate.json $LAB > $LAB/aggregate.md
EOF
