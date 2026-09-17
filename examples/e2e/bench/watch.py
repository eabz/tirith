#!/usr/bin/env python3
"""Watchdog for benchmark runs: report progress, flag stalls, never touch processes.

Usage: watch.py [--max-secs 540] [--interval 30] [--stall-secs 900] <run_dir>...

Every interval it prints one line per run: tasks done/total, who holds
in-progress tasks, live claims, and seconds since the run's last coordination
call (coord.jsonl). It reads the daemon's read-only dashboard API
(GET /api/state), so it adds no agent and no call to the run's logs. It runs
in the foreground and exits:

  0  every run has all tasks done: score the runs (workers that pull with
     wait_secs may still be finishing their last empty-board pull)
  2  --max-secs elapsed with work left: run it again (keeps each call under a
     10-minute tool timeout)
  3  a run looks stuck: its daemon does not answer, or no coordination call
     for --stall-secs while tasks remain. Check the workers; stop a run only
     with stop.sh after scoring what it has.
"""

import json
import sys
import time
import urllib.request
from datetime import datetime, timezone
from pathlib import Path


def state(run):
    port = json.loads((run / "meta.json").read_text())["port"]
    with urllib.request.urlopen("http://127.0.0.1:%d/api/state" % port, timeout=5) as response:
        return json.loads(response.read())


def task_status(task):
    status = task.get("status")
    if isinstance(status, dict):
        return status.get("state") or status.get("status") or next(iter(status), None), status.get("owner")
    return status, task.get("owner")


def last_call_age(run):
    path = run / "coord.jsonl"
    try:
        last = path.read_text().strip().splitlines()[-1]
        ts = json.loads(last)["ts"]
    except (OSError, IndexError, ValueError, KeyError):
        return None
    when = datetime.strptime(ts, "%Y-%m-%dT%H:%M:%S.%fZ").replace(tzinfo=timezone.utc)
    return (datetime.now(timezone.utc) - when).total_seconds()


def main():
    args = sys.argv[1:]
    opts = {"--max-secs": 540.0, "--interval": 30.0, "--stall-secs": 900.0}
    while args and args[0] in opts:
        if len(args) < 2:
            raise SystemExit(__doc__)
        opts[args[0]] = float(args[1])
        args = args[2:]
    if not args:
        raise SystemExit(__doc__)
    runs = [Path(a).resolve() for a in args]
    started = time.time()
    while True:
        finished, stuck = 0, []
        stamp = time.strftime("%H:%M:%S")
        for run in runs:
            try:
                snapshot = state(run)
            except (OSError, ValueError) as err:
                print("%s %-28s daemon not answering: %s" % (stamp, run.name, err))
                stuck.append(run.name)
                continue
            tasks = snapshot.get("tasks") or []
            done = owners = 0
            holders = []
            for task in tasks:
                status, owner = task_status(task)
                done += status == "done"
                if status == "in_progress":
                    owners += 1
                    holders.append(str(owner))
            age = last_call_age(run)
            all_done = bool(tasks) and done == len(tasks)
            finished += all_done
            print("%s %-28s tasks %d/%d  in_progress %s  claims %s  last call %s s ago" % (
                stamp, run.name, done, len(tasks), ",".join(sorted(holders)) or "-",
                (snapshot.get("counts") or {}).get("claims"), "n/a" if age is None else int(age)))
            if not all_done and age is not None and age > opts["--stall-secs"]:
                stuck.append(run.name)
        sys.stdout.flush()
        if finished == len(runs):
            print("all runs done: score them (score.sh), then stop.sh")
            return 0
        if stuck:
            print("stuck: %s" % ", ".join(stuck))
            return 3
        if time.time() - started + opts["--interval"] > opts["--max-secs"]:
            print("still running; run the watchdog again")
            return 2
        time.sleep(opts["--interval"])


if __name__ == "__main__":
    sys.exit(main())
