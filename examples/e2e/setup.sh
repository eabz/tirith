#!/usr/bin/env bash
# setup.sh <run_dir> <port> [--scenario forge|hubs] [--claims file|symbol] [--wait sleep|server]
#          [--protocol manual|hooks]
#
# Prepares one benchmark run: a fresh git repo copy of the scenario, a
# Tirith daemon for it, the seeded coordination history and task board, and
# the `tb` wrapper workers call. Prints the MCP URL.
#
# --scenario forge (default) is the shared-files and breaking-change
#   scenario at the kit root; hubs is the sub-file contention scenario in
#   scenarios/hubs/.
# --claims file|symbol (hubs only, default file) selects the claim
#   granularity paragraph of the worker prompt; symbol requires a daemon
#   that supports `path#Symbol` anchors (probed here).
# --wait sleep|server (hubs only; default server) selects how workers wait
#   for refused claims: claim `wait_secs` 120 (probed here), or a foreground
#   python sleep and retry for daemons without wait_secs. Both claims arms of
#   a comparison must use the same value.
# --protocol manual|hooks (default manual): hooks writes Claude Code hook
#   settings into <run_dir>/repo/.claude/settings.json (and <run_dir>/hooks/
#   settings.json) that claim, release, and feed tasks for the worker; see
#   hooks/README.md. Hooks claim whole files with wait_secs (probed here), so
#   it needs --claims file and implies --wait server. Only the run directory
#   is touched; a run directory inside the Tirith repository is refused.
#   HOOKS_CONFIG='{...}' overrides hooks/common.py DEFAULTS for the run.
#
# Env: TIRITH_BIN (default ~/.cargo/bin/tirith), TIRITH_REPO (the Tirith
# checkout, used to refuse hook runs inside it; default: the repository this
# kit lives in).
# UNSAFE_SKIP_ANCHOR_PROBE=1 lets --claims symbol run on a daemon without
# anchor support, for plumbing checks only: such a daemon treats `f#X` as a
# path unrelated to `f`, so the run measures nothing. meta.json records it.
set -euo pipefail

usage() {
  echo "usage: $0 <run_dir> <port> [--scenario forge|hubs] [--claims file|symbol] [--wait sleep|server] [--protocol manual|hooks]" >&2
  exit 2
}
[ $# -ge 2 ] || usage
RUN_ARG=$1 PORT=$2
shift 2
SCENARIO=forge CLAIMS=file WAIT= CLAIMS_SET= PROTOCOL=manual
while [ $# -gt 0 ]; do
  [ $# -ge 2 ] || usage
  case "$1" in
    --scenario) SCENARIO=$2 ;;
    --claims) CLAIMS=$2 CLAIMS_SET=1 ;;
    --wait) WAIT=$2 ;;
    --protocol) PROTOCOL=$2 ;;
    *) usage ;;
  esac
  shift 2
done
case "$PORT" in ''|*[!0-9]*) usage ;; esac
case "$SCENARIO" in forge|hubs) ;; *) usage ;; esac
case "$CLAIMS" in file|symbol) ;; *) usage ;; esac
case "$PROTOCOL" in manual|hooks) ;; *) usage ;; esac
if [ "$PROTOCOL" = hooks ]; then
  [ "$CLAIMS" = file ] || { echo "--protocol hooks claims whole files; use --claims file" >&2; exit 2; }
  [ "$WAIT" != sleep ] || { echo "--protocol hooks waits with claim wait_secs; drop --wait sleep" >&2; exit 2; }
fi
if [ "$SCENARIO" = forge ]; then
  if [ -n "$CLAIMS_SET" ] || [ -n "$WAIT" ]; then echo "--claims and --wait apply to --scenario hubs only" >&2; exit 2; fi
  WAIT=sleep
fi
WAIT=${WAIT:-server}
if [ "$PROTOCOL" = hooks ]; then WAIT=server; fi
case "$WAIT" in sleep|server) ;; *) usage ;; esac

KIT=$(cd "$(dirname "$0")" && pwd)
if [ "$SCENARIO" = forge ]; then SCEN=$KIT; else SCEN=$KIT/scenarios/$SCENARIO; fi
TIRITH_REPO=${TIRITH_REPO:-$(cd "$KIT/../.." && pwd)}
TIRITH_BIN=${TIRITH_BIN:-$HOME/.cargo/bin/tirith}
[ -x "$TIRITH_BIN" ] || { echo "tirith binary not found at $TIRITH_BIN" >&2; exit 1; }
for tool in git jq python3 perl curl tar; do
  command -v "$tool" >/dev/null || { echo "missing $tool" >&2; exit 1; }
done

if [ "$PROTOCOL" = hooks ]; then
  # Hook settings apply to every Claude Code session started under the repo
  # copy; never let that be the Tirith checkout, anything above it, or $HOME.
  REAL_RUN=$(python3 -c 'import os, sys; print(os.path.realpath(sys.argv[1]))' "$RUN_ARG")
  REAL_TIRITH=$(cd "$TIRITH_REPO" && pwd -P)
  case "$REAL_RUN/" in "$REAL_TIRITH"/*) echo "refusing --protocol hooks: $RUN_ARG is inside $TIRITH_REPO" >&2; exit 1 ;; esac
  case "$REAL_TIRITH/" in "$REAL_RUN"/*) echo "refusing --protocol hooks: $TIRITH_REPO is inside $RUN_ARG" >&2; exit 1 ;; esac
  [ "$REAL_RUN" != "$(cd "$HOME" && pwd -P)" ] || { echo "refusing --protocol hooks: run directory is \$HOME" >&2; exit 1; }
fi
mkdir -p "$RUN_ARG"
RUN=$(cd "$RUN_ARG" && pwd)
REPO=$RUN/repo
[ ! -e "$REPO" ] || { echo "$REPO already exists; use a fresh run_dir" >&2; exit 1; }
if curl -s -o /dev/null --max-time 1 "http://127.0.0.1:$PORT/"; then
  echo "port $PORT is already serving something" >&2; exit 1
fi

# 1. Repository copy with a local-only git history.
cp -R "$SCEN/scenario" "$REPO"
find "$REPO" -name __pycache__ -type d -prune -exec rm -rf {} +
git -C "$REPO" init -q
git -C "$REPO" add -A
git -C "$REPO" -c user.name=bench -c user.email=bench@localhost commit -q -m "$SCENARIO baseline"

# 2. Daemon, fully detached (own session, stdin closed, output to a log) so
#    it survives the shell or agent tool that ran setup. Scrub inherited
#    Tirith variables so every run starts alike.
printf '%s\n' "$TIRITH_BIN" > "$RUN/tirith_bin"
DAEMON_ENV=(env -u TIRITH_URL -u TIRITH_AGENT)
SERVE=("$TIRITH_BIN" --root "$REPO" serve --bind "127.0.0.1:$PORT" --no-tray)
PID=$(python3 - "$RUN/serve.log" "${DAEMON_ENV[@]}" "${SERVE[@]}" <<'PY'
import subprocess, sys
log = open(sys.argv[1], "ab")
proc = subprocess.Popen(sys.argv[2:], stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT,
                        start_new_session=True, close_fds=True)
print(proc.pid)
PY
)
echo "$PID" > "$RUN/daemon.pid"

URL="http://127.0.0.1:$PORT/mcp"
printf '%s\n' "$URL" > "$RUN/url"
call() { "$TIRITH_BIN" --url "$URL" --json -a "$1" call "$2" "$3"; }
trap 'rc=$?; if [ $rc -ne 0 ]; then "$KIT/stop.sh" "$RUN" >&2 || true; fi' EXIT
fail() { echo "$1" >&2; "$KIT/stop.sh" "$RUN" >&2 || true; exit 1; }
for _ in $(seq 1 150); do
  if call setup status '{}' > /dev/null 2>&1; then break; fi
  kill -0 "$PID" 2>/dev/null || { echo "daemon exited; see $RUN/serve.log" >&2; tail -5 "$RUN/serve.log" >&2; exit 1; }
  python3 -c "import time; time.sleep(0.1)"
done
call setup status '{}' > /dev/null || fail "daemon did not answer"

# 3. Probe what the run relies on, before anything is seeded. Probe claims
#    use paths outside the scenario and are released at once.
if [ "$WAIT" = server ]; then
  call probe-a claim '{"paths":["zz-probe/wait.txt"],"reason":"setup probe","brief":false}' > /dev/null
  T0=$(perl -MTime::HiRes=time -e 'printf "%.3f", time')
  call probe-b claim '{"paths":["zz-probe/wait.txt"],"reason":"setup probe","brief":false,"wait_secs":1}' > /dev/null 2>&1 || true
  T1=$(perl -MTime::HiRes=time -e 'printf "%.3f", time')
  call probe-a release '{}' > /dev/null 2>&1 || true; call probe-b release '{}' > /dev/null 2>&1 || true
  perl -e "exit(($T1 - $T0) >= 0.8 ? 0 : 1)" || fail "--wait server / --protocol hooks: this daemon ignores claim wait_secs"
fi
if [ "$CLAIMS" = symbol ]; then
  call probe-a claim '{"paths":["zz-probe/a.py"],"reason":"setup probe","brief":false}' > /dev/null
  NESTED=$(call probe-b claim '{"paths":["zz-probe/a.py#f"],"reason":"setup probe","brief":false}' 2>/dev/null | jq -r .status || true)
  call probe-a claim '{"paths":["zz-probe/b.py#f"],"reason":"setup probe","brief":false}' > /dev/null
  SIBLING=$(call probe-b claim '{"paths":["zz-probe/b.py#g"],"reason":"setup probe","brief":false}' 2>/dev/null | jq -r .status || true)
  call probe-a release '{}' > /dev/null 2>&1 || true; call probe-b release '{}' > /dev/null 2>&1 || true
  if [ "$NESTED" = conflict ] && [ "$SIBLING" = ok ]; then
    ANCHOR_PROBE=ok
  elif [ "${UNSAFE_SKIP_ANCHOR_PROBE:-}" = 1 ]; then
    ANCHOR_PROBE=failed-skipped
    echo "WARNING: daemon does not support anchors; plumbing check only, not a symbol-arm measurement" >&2
  else
    fail "--claims symbol: daemon does not support path#Symbol anchors (file vs anchor: $NESTED, sibling anchors: $SIBLING)"
  fi
fi

# 4. History and tasks.
python3 "$KIT/lib/seed.py" "$SCEN" "$RUN"
if [ "$PROTOCOL" = hooks ]; then
  # Hook scripts and state live next to the repo copy, the settings inside it
  # (committed with the seeded baseline, so they are not scored as code).
  mkdir -p "$RUN/hooks" "$RUN/hooks_state" "$REPO/.claude"
  cp "$KIT"/hooks/*.py "$RUN/hooks/"
  chmod +x "$RUN"/hooks/*.py
  printf '%s' "${HOOKS_CONFIG:-}" | jq -s -c '{claim_wait_secs: 120, stop_gate: "placeholder"} + (.[0] // {})' \
    > "$RUN/hooks_state/config.json" || fail "HOOKS_CONFIG is not a JSON object"
  python3 "$KIT/hooks/settings.py" "$RUN" > "$RUN/hooks/settings.json" || fail "could not write hook settings"
  cp "$RUN/hooks/settings.json" "$REPO/.claude/settings.json"
fi
git -C "$REPO" add -A
git -C "$REPO" -c user.name=bench -c user.email=bench@localhost commit -q -m "Seed coordination history"
BASE=$(git -C "$REPO" rev-parse HEAD)

# 5. Worker wrapper and run metadata.
cp "$KIT/tb" "$RUN/tb"
chmod +x "$RUN/tb"
: > "$RUN/coord.jsonl"
: > "$RUN/coord_full.jsonl"
# Scenarios with per-task anchors score lost writes from done-time snapshots.
if jq -e '[.tasks[] | has("anchors")] | any' "$SCEN/tasks.json" > /dev/null; then mkdir -p "$RUN/snapshots"; fi
jq -n --arg url "$URL" --argjson port "$PORT" --argjson pid "$PID" \
  --arg base "$BASE" --arg kit "$KIT" --arg bin "$TIRITH_BIN" \
  --arg version "$("$TIRITH_BIN" --version)" --arg at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg scenario "$SCENARIO" --arg scen "$SCEN" --arg claims "$CLAIMS" --arg wait "$WAIT" \
  --arg probe "${ANCHOR_PROBE:-}" --arg protocol "$PROTOCOL" \
  '{url:$url, port:$port, pid:$pid, base_commit:$base, kit:$kit, tirith_bin:$bin,
    tirith_version:$version, setup_at:$at, scenario:$scenario, scenario_dir:$scen,
    claims:(if $scenario == "forge" then null else $claims end), wait:$wait,
    anchor_probe:(if $probe == "" then null else $probe end), protocol:$protocol}' \
  > "$RUN/meta.json"

echo "scenario=$SCENARIO claims=$CLAIMS wait=$WAIT protocol=$PROTOCOL pid=$PID run=$RUN"
echo "$URL"
