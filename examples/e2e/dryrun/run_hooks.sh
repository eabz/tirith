#!/usr/bin/env bash
# run_hooks.sh <run_dir> <port> [workers=3]
# Plumbing check of --protocol hooks without LLMs: setup (hubs, file
# claims, hooks), the hook selfcheck, K scripted workers that only talk to
# the hook scripts (the last one without TB_AGENT, so hooks name it), score,
# stop. Short waits via HOOKS_CONFIG; never touches anything outside run_dir.
# The fakes splice final reference symbols, which reference symbols of other
# tasks, so the full suite fails until every task lands; the dry run's stop
# gate therefore checks that the package imports instead of running the suite.
set -euo pipefail
[ $# -ge 2 ] || { echo "usage: $0 <run_dir> <port> [workers]" >&2; exit 2; }
KIT=$(cd "$(dirname "$0")/.." && pwd)
RUN=$1 PORT=$2 K=${3:-3}
HOOKS_CONFIG=${HOOKS_CONFIG:-'{"claim_wait_secs":3,"poll_secs":1,"poll_slice_secs":20,"idle_budget_secs":180,"gate_max_continues":6,"test_timeout_secs":120,"test_command":["python3","-c","import tally.pricing, tally.rules, tally.money, tally.models, tally.invoice"]}'} \
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
        claim_ms: .hooks.claim_ms, gate_ms: .hooks.gate_ms,
        mechanical_turns: .worker_turns.total.mechanical, turns: .worker_turns.total.turns}' "$RUN/score.json"
echo "score: $RUN/score.json"
