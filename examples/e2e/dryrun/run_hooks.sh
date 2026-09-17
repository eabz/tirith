#!/usr/bin/env bash
# run_hooks.sh <run_dir> <port> [workers=3]
# Plumbing check of --protocol hooks without LLMs: setup (hubs, file
# claims, hooks), the hook selfcheck, K scripted workers that only talk to
# the hook scripts (the last one without TB_AGENT, so hooks name it), score,
# stop. Short waits via HOOKS_CONFIG; never touches anything outside run_dir.
# The stop gate runs each task's own acceptance module (gate_tests "task").
# The fakes splice final reference symbols, which use symbols of other tasks,
# so a task's checks can fail until those land; the gate then continues and
# the fake stops again later, hence the higher gate_max_continues.
set -euo pipefail
[ $# -ge 2 ] || { echo "usage: $0 <run_dir> <port> [workers]" >&2; exit 2; }
KIT=$(cd "$(dirname "$0")/.." && pwd)
RUN=$1 PORT=$2 K=${3:-3}
HOOKS_CONFIG=${HOOKS_CONFIG:-'{"claim_wait_secs":3,"poll_secs":1,"poll_slice_secs":20,"idle_budget_secs":180,"gate_max_continues":15,"test_timeout_secs":120}'} \
  "$KIT/setup.sh" "$RUN" "$PORT" --scenario hubs --claims file --protocol hooks
trap '"$KIT/stop.sh" "$RUN"' EXIT
python3 "$KIT/dryrun/fake_worker_hooks.py" "$RUN" selfcheck
PIDS=()
for i in $(seq 1 "$K"); do
  if [ "$i" -eq "$K" ] && [ "$K" -gt 1 ]; then
    python3 "$KIT/dryrun/fake_worker_hooks.py" "$RUN" "fake$i" --auto-identity & PIDS+=($!)
  else
    python3 "$KIT/dryrun/fake_worker_hooks.py" "$RUN" "fake$i" & PIDS+=($!)
  fi
done
for pid in "${PIDS[@]}"; do wait "$pid"; done
"$KIT/score.sh" "$RUN" > /dev/null
jq -c '{tasks: .tasks.done, of: .tasks.total, hidden: "\(.acceptance.hidden_passed)/\(.acceptance.hidden_total)",
        integration: .acceptance.integration.passed, regressions: (.anchors.lost_writes.regressions | length),
        calls_by_origin: .coordination.per_origin, hook_actions: .hooks.actions, hook_errors: .hooks.errors,
        claim_ms: .hooks.claim_ms, gate_ms: .hooks.gate_ms, violations: .violations.total,
        turns: .worker_turns.total.turns, mechanical_turns: .worker_turns.total.mechanical.turns,
        coordination_only_turns: .worker_turns.total.coordination_only.turns, workers: .workers}' "$RUN/score.json"
echo "score: $RUN/score.json"
