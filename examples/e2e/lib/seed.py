#!/usr/bin/env python3
"""Seed history.json and tasks.json into a running daemon through the CLI.

Usage: seed.py <kit_dir> <run_dir>

Writes <run_dir>/setup/seed.jsonl (every call and its response) and
<run_dir>/task_ids.json (task key -> id). Exits non-zero if any seeding
call does not return status ok, so a run never starts on a partial board.
"""

import json
import subprocess
import sys
from pathlib import Path


def main() -> int:
    kit, run = Path(sys.argv[1]), Path(sys.argv[2])
    url = (run / "url").read_text().strip()
    tirith = (run / "tirith_bin").read_text().strip()
    history = json.loads((kit / "history.json").read_text())
    tasks = json.loads((kit / "tasks.json").read_text())
    (run / "setup").mkdir(exist_ok=True)
    log = (run / "setup" / "seed.jsonl").open("w")
    failures = []

    def call(agent, tool, args, key):
        args = {k: v for k, v in args.items() if v is not None}
        proc = subprocess.run(
            [tirith, "--url", url, "--json", "-a", agent, "call", tool, json.dumps(args)],
            capture_output=True,
            text=True,
            timeout=60,
        )
        try:
            response = json.loads(proc.stdout)
        except json.JSONDecodeError:
            response = {"status": "error", "stderr": proc.stderr.strip()[:500]}
        log.write(json.dumps({"key": key, "agent": agent, "tool": tool, "response": response}) + "\n")
        if response.get("status") != "ok":
            failures.append((key, tool, response.get("status"), response.get("message") or response.get("stderr")))
        return response

    for m in history["memory"]:
        call(m["by"], "memory_write", {"title": m["title"], "body": m["body"], "kind": m["kind"],
                                       "paths": m["paths"], "tags": m.get("tags", [])}, m["key"])
    for d in history["decisions"]:
        call(d["by"], "decision_record", {"title": d["title"], "decision": d["decision"],
                                          "rationale": d["rationale"], "alternatives": d.get("alternatives", []),
                                          "affects_paths": d["affects_paths"]}, d["key"])
    for c in history["contracts"]:
        call(c["by"], "contract_publish", {"name": c["name"], "kind": c["kind"], "shape": c["shape"],
                                           "consumers": c["consumers"], "notes": c.get("notes")}, c["key"])
    for c in history.get("contract_republish", []):
        call(c["by"], "contract_publish", {"name": c["name"], "kind": c["kind"], "shape": c["shape"],
                                           "notes": c.get("notes")}, c["key"])
    for n in history["notices"]:
        call(n["by"], "notice_publish", {"kind": n["kind"], "summary": n["summary"], "from": n.get("from"),
                                         "to": n.get("to"), "affected_paths": n["affected_paths"]}, n["key"])

    ids = {}
    creator = tasks.get("created_by", "pm")
    for t in tasks["tasks"]:
        depends = [ids[k] for k in t.get("depends_on", [])]
        response = call(creator, "task_create", {"title": t["title"], "description": t["description"],
                                                 "priority": t["priority"], "depends_on": depends,
                                                 "paths": t["paths"]}, t["key"])
        task = response.get("task") or {}
        if "id" in task:
            ids[t["key"]] = task["id"]
    (run / "task_ids.json").write_text(json.dumps(ids, indent=2) + "\n")
    log.close()
    if failures:
        for failure in failures:
            print("seed failed: %s %s -> %s %s" % failure, file=sys.stderr)
        return 1
    print("seeded %d memory, %d decisions, %d contract publishes, %d notices, %d tasks" % (
        len(history["memory"]), len(history["decisions"]),
        len(history["contracts"]) + len(history.get("contract_republish", [])),
        len(history["notices"]), len(ids)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
