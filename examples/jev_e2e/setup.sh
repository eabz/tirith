#!/usr/bin/env bash
# setup.sh <run_dir> <port> <arm: off|jev>
#
# Prepares one benchmark run: a fresh git repo copy of the scenario, a
# Tirith daemon for it (the ONLY difference between arms is `--jev` with
# TIRITH_JEV_PROVIDER=gateway), the seeded coordination history and task
# board, and the `tb` wrapper workers call. Prints the MCP URL.
#
# Env: TIRITH_BIN (default ~/.cargo/bin/tirith), TIRITH_REPO (where .env
# is copied from; default: the repository this kit lives in).
set -euo pipefail

usage() { echo "usage: $0 <run_dir> <port> <off|jev>" >&2; exit 2; }
[ $# -eq 3 ] || usage
RUN_ARG=$1 PORT=$2 ARM=$3
case "$ARM" in off|jev) ;; *) usage ;; esac
case "$PORT" in ''|*[!0-9]*) usage ;; esac

KIT=$(cd "$(dirname "$0")" && pwd)
TIRITH_REPO=${TIRITH_REPO:-$(cd "$KIT/../.." && pwd)}
TIRITH_BIN=${TIRITH_BIN:-$HOME/.cargo/bin/tirith}
[ -x "$TIRITH_BIN" ] || { echo "tirith binary not found at $TIRITH_BIN" >&2; exit 1; }
[ -f "$TIRITH_REPO/.env" ] || { echo "no .env in $TIRITH_REPO" >&2; exit 1; }
for tool in git jq python3 perl curl; do
  command -v "$tool" >/dev/null || { echo "missing $tool" >&2; exit 1; }
done

mkdir -p "$RUN_ARG"
RUN=$(cd "$RUN_ARG" && pwd)
REPO=$RUN/repo
[ ! -e "$REPO" ] || { echo "$REPO already exists; use a fresh run_dir" >&2; exit 1; }
if curl -s -o /dev/null --max-time 1 "http://127.0.0.1:$PORT/"; then
  echo "port $PORT is already serving something" >&2; exit 1
fi

# 1. Repository copy with a local-only git history.
cp -R "$KIT/scenario" "$REPO"
find "$REPO" -name __pycache__ -type d -prune -exec rm -rf {} +
git -C "$REPO" init -q
git -C "$REPO" add -A
git -C "$REPO" -c user.name=bench -c user.email=bench@localhost commit -q -m "Forge 0.4.0 baseline"

# 2. Jev keys for the daemon (identical in both arms; only --jev reads them).
cp "$TIRITH_REPO/.env" "$REPO/.env"
chmod 600 "$REPO/.env"

# 3. Daemon. Scrub inherited Tirith variables so both arms start alike.
printf '%s\n' "$TIRITH_BIN" > "$RUN/tirith_bin"
DAEMON_ENV=(env -u TIRITH_JEV -u TIRITH_JEV_PROVIDER -u TIRITH_JEV_MODEL -u TIRITH_JEV_ENDPOINT
  -u TIRITH_JEV_TIMEOUT_MS -u TIRITH_URL -u TIRITH_AGENT -u TYPESAFE_API_KEY -u AI_GATEWAY_API_KEY)
SERVE=("$TIRITH_BIN" --root "$REPO" serve --bind "127.0.0.1:$PORT" --no-tray)
if [ "$ARM" = jev ]; then
  DAEMON_ENV+=(TIRITH_JEV_PROVIDER=gateway)
  SERVE+=(--jev)
fi
nohup "${DAEMON_ENV[@]}" "${SERVE[@]}" > "$RUN/serve.log" 2>&1 < /dev/null &
PID=$!
echo "$PID" > "$RUN/daemon.pid"

URL="http://127.0.0.1:$PORT/mcp"
printf '%s\n' "$URL" > "$RUN/url"
for _ in $(seq 1 150); do
  if "$TIRITH_BIN" --url "$URL" --json -a setup call status '{}' > /dev/null 2>&1; then break; fi
  kill -0 "$PID" 2>/dev/null || { echo "daemon exited; see $RUN/serve.log" >&2; tail -5 "$RUN/serve.log" >&2; exit 1; }
  sleep 0.1
done
"$TIRITH_BIN" --url "$URL" --json -a setup call status '{}' > /dev/null || { echo "daemon did not answer" >&2; exit 1; }
if [ "$ARM" = jev ]; then
  # The daemon answers before Jev is warmed up and enabled; wait for that.
  for _ in $(seq 1 200); do grep -q '^jev: on (gateway' "$RUN/serve.log" && break; sleep 0.1; done
  grep -q '^jev: on (gateway' "$RUN/serve.log" || { echo "jev arm but the daemon did not report 'jev: on (gateway'" >&2; kill -INT "$PID"; exit 1; }
else
  if grep -q '^jev: on' "$RUN/serve.log"; then echo "off arm but jev is on" >&2; kill -INT "$PID"; exit 1; fi
fi

# 4. History and tasks.
python3 "$KIT/lib/seed.py" "$KIT" "$RUN"
git -C "$REPO" add -A
git -C "$REPO" -c user.name=bench -c user.email=bench@localhost commit -q -m "Seed coordination history"
BASE=$(git -C "$REPO" rev-parse HEAD)
"$TIRITH_BIN" --url "$URL" --json -a setup call status '{}' | jq -c . > "$RUN/setup/status.json"

# 5. Worker wrapper and run metadata.
cp "$KIT/tb" "$RUN/tb"
chmod +x "$RUN/tb"
: > "$RUN/coord.jsonl"
: > "$RUN/coord_full.jsonl"
jq -n --arg arm "$ARM" --arg url "$URL" --argjson port "$PORT" --argjson pid "$PID" \
  --arg base "$BASE" --arg kit "$KIT" --arg bin "$TIRITH_BIN" \
  --arg version "$("$TIRITH_BIN" --version)" --arg at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  '{arm:$arm, url:$url, port:$port, pid:$pid, base_commit:$base, kit:$kit, tirith_bin:$bin,
    tirith_version:$version, setup_at:$at}' > "$RUN/meta.json"

echo "arm=$ARM pid=$PID run=$RUN"
echo "$URL"
