#!/usr/bin/env bash
# fake_worker.sh <run_dir> <agent> -- a scripted, LLM-free worker that follows
# the WORKER_PROMPT protocol mechanically, to validate the kit plumbing:
# pull, claim (narrowing on conflict), fake an edit, run tests, publish a
# notice for the breaking task, mark done, release. It never writes real
# features, so hidden acceptance stays near zero by design.
set -uo pipefail
[ $# -eq 2 ] || { echo "usage: $0 <run_dir> <agent>" >&2; exit 2; }
export TB_RUN=$1 TB_AGENT=$2
TB="$TB_RUN/tb"
REPO="$TB_RUN/repo"
SHARED='["app/config.py","app/router.py","app/middlewares/__init__.py"]'

"$TB" memory_search '{}' > /dev/null
for _ in $(seq 1 20); do
  PULL=$("$TB" task_pull '{}')
  [ "$(jq -r .status <<<"$PULL")" = ok ] || break
  ID=$(jq -r .task.id <<<"$PULL")
  TITLE=$(jq -r .task.title <<<"$PULL")
  PATHS=$(jq -c .task.paths <<<"$PULL")

  CLAIM=$("$TB" claim "$(jq -nc --argjson p "$PATHS" --arg r "$TITLE" '{paths:$p, reason:$r}')")
  if [ "$(jq -r .status <<<"$CLAIM")" = conflict ]; then
    # Narrow to the task's own files, then wait for the shared ones.
    OWN=$(jq -c --argjson s "$SHARED" '[.[] | select(. as $x | $s | index($x) | not)]' <<<"$PATHS")
    "$TB" claim "$(jq -nc --argjson p "$OWN" --arg r "$TITLE (own files)" '{paths:$p, reason:$r}')" > /dev/null
    for _ in $(seq 1 30); do
      "$TB" claim "$(jq -nc --argjson p "$PATHS" --arg r "$TITLE" '{paths:$p, reason:$r}')" > /dev/null && break
      sleep 1
    done
  fi

  # Fake edit: a marker comment in each claimed file path that is a .py file.
  for p in $(jq -r '.[] | select(endswith(".py"))' <<<"$PATHS"); do
    mkdir -p "$(dirname "$REPO/$p")"
    printf '# touched by %s for: %s\n' "$TB_AGENT" "$TITLE" >> "$REPO/$p"
  done
  sleep "$(perl -e 'printf "%.1f", 1 + rand(2)')"
  (cd "$REPO" && python3 -m unittest > /dev/null 2>&1) || true

  case "$TITLE" in
    BREAKING*)
      "$TB" notice_publish '{"kind":"signature","summary":"Middleware hooks now take ctx as the last parameter; Request.extras removed","from":"process_request(request)","to":"process_request(request, ctx)","affected_paths":["app/middlewares/","app/pipeline.py"]}' > /dev/null
      "$TB" message_send '{"to":"*","text":"ctx-interface landed: hooks take ctx last; migrate your middleware before marking done"}' > /dev/null
      ;;
  esac
  "$TB" task_update "$(jq -nc --arg id "$ID" '{task_id:$id, status:"done", note:"fake worker"}')" > /dev/null
  "$TB" release '{}' > /dev/null
done
echo "$TB_AGENT finished"
