#!/usr/bin/env bash
# stop.sh <run_dir> -- stop the run's daemon (SIGINT is its graceful stop).
set -euo pipefail
[ $# -eq 1 ] || { echo "usage: $0 <run_dir>" >&2; exit 2; }
PID=$(cat "$1/daemon.pid")
if ! kill -0 "$PID" 2>/dev/null; then echo "daemon $PID not running"; exit 0; fi
# Only signal the process if it is the tirith daemon this run started.
ps -p "$PID" -o command= | grep -q "tirith.*serve" || { echo "pid $PID is not a tirith daemon; not touching it" >&2; exit 1; }
kill -INT "$PID"
for _ in $(seq 1 50); do kill -0 "$PID" 2>/dev/null || { echo "stopped $PID"; exit 0; }; sleep 0.1; done
echo "daemon $PID did not exit after SIGINT" >&2
exit 1
