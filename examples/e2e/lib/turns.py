#!/usr/bin/env python3
"""Classify worker turns in Claude Code transcripts. Usage: turns.py <transcript.jsonl>...

A turn is one model response: the assistant transcript rows that share a
`message.id` (Claude Code writes thinking, text and each tool_use as
separate rows). Its cost is estimated from the usage on the turn's last row
in input-token equivalents: input 1, cache write 1.25, cache read 0.1,
output 5 (list-price ratios; only shares are reported).

Kinds:
  mechanical   every tool call is a Bash command whose only real work is
               `tb claim|release|task_pull|task_update` (plus cd, echo,
               variable assignments or a python sleep): the steps the hook
               protocol takes out of the model loop
  coordination the same, but at least one other tb tool (message_send,
               notice_publish, task_list, memory_search, ...)
  work         any other tool call
  text         no tool call (thinking or a final answer)

coordination_only_with_a_mechanical_call is the looser count: coordination-only
turns (mechanical or coordination) with at least one of the four mechanical tools.
"""

import json
import re
import shlex
import sys
from collections import OrderedDict
from pathlib import Path

MECHANICAL_TOOLS = {"claim", "release", "task_pull", "task_update"}
WEIGHTS = {"input_tokens": 1.0, "cache_creation_input_tokens": 1.25, "cache_read_input_tokens": 0.1,
           "output_tokens": 5.0}
SLEEP_RE = re.compile(r"""python3?\s+-c\s+(["'])import time;\s*time\.sleep\([0-9.]+\)\1""")
ASSIGN_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*=")
NEUTRAL = {"cd", "echo", "true", "sleep", ":", "printf"}


def load_turns(path):
    turns = OrderedDict()
    with open(str(path)) as handle:
        for raw in handle:
            try:
                row = json.loads(raw)
            except ValueError:
                continue
            if row.get("type") != "assistant" or not isinstance(row.get("message"), dict):
                continue
            message = row["message"]
            key = message.get("id") or row.get("uuid")
            turn = turns.setdefault(key, {"id": key, "tools": [], "usage": {}, "ts": row.get("timestamp")})
            for block in message.get("content") or []:
                if isinstance(block, dict) and block.get("type") == "tool_use":
                    turn["tools"].append({"name": block.get("name"), "input": block.get("input") or {}})
            if message.get("usage"):
                turn["usage"] = message["usage"]
    return list(turns.values())


def segments(command):
    """Top-level shell segments of a command, python sleeps and heredoc bodies removed."""
    command = SLEEP_RE.sub("sleep 0", command)
    command = re.sub(r"<<-?\s*'?(\w+)'?.*?^\s*\1\s*$", "", command, flags=re.S | re.M)
    return [s.strip() for s in re.split(r"&&|\|\||;|\n|\|", command) if s.strip()]


def segment_kind(segment):
    """('tb', tool) for a tb call, ('neutral', None), or ('work', None)."""
    try:
        words = shlex.split(segment)
    except ValueError:
        words = segment.split()
    while words and ASSIGN_RE.match(words[0]):
        words = words[1:]
    if not words:
        return "neutral", None
    head = words[0]
    if head.endswith("/tb") or head == "tb":
        return "tb", (words[1] if len(words) > 1 else "?")
    if head in NEUTRAL or head in ("(", ")", "{", "}", "done", "do", "fi", "then", "else"):
        return "neutral", None
    if head in ("for", "while", "until", "if"):
        return "neutral", None
    return "work", None


def classify(turn):
    if not turn["tools"]:
        return "text", []
    tb_tools = []
    for tool in turn["tools"]:
        if tool["name"] != "Bash":
            return "work", tb_tools
        for segment in segments(str(tool["input"].get("command", ""))):
            kind, name = segment_kind(segment)
            if kind == "work":
                return "work", tb_tools
            if kind == "tb":
                tb_tools.append(name)
    if not tb_tools:
        return "work", tb_tools
    return ("mechanical" if all(t in MECHANICAL_TOOLS for t in tb_tools) else "coordination"), tb_tools


def cost(usage):
    return sum(float(usage.get(k) or 0) * w for k, w in WEIGHTS.items())


def summarize(paths):
    kinds = ("mechanical", "coordination", "work", "text")
    out = {"turns": 0, "cost_units": 0.0, "transcripts": len(paths)}
    for kind in kinds:
        out[kind] = {"turns": 0, "cost_units": 0.0}
    loose = {"turns": 0, "cost_units": 0.0}
    for path in paths:
        for turn in load_turns(path):
            kind, tb_tools = classify(turn)
            units = cost(turn["usage"])
            out["turns"] += 1
            out["cost_units"] += units
            out[kind]["turns"] += 1
            out[kind]["cost_units"] += units
            if kind in ("mechanical", "coordination") and MECHANICAL_TOOLS & set(tb_tools):
                loose["turns"] += 1
                loose["cost_units"] += units
    out["coordination_only_with_a_mechanical_call"] = loose
    for kind in kinds + ("coordination_only_with_a_mechanical_call",):
        row = out[kind]
        row["cost_units"] = round(row["cost_units"])
        row["turn_share"] = round(row["turns"] / out["turns"], 3) if out["turns"] else None
        row["cost_share"] = round(row["cost_units"] / out["cost_units"], 3) if out["cost_units"] else None
    out["cost_units"] = round(out["cost_units"])
    return out


def discover(run):
    """agent -> transcript paths: <run>/transcripts/<agent>*.jsonl and hook session registries."""
    found = {}
    tdir = Path(run) / "transcripts"
    if tdir.is_dir():
        for path in sorted(tdir.glob("*.jsonl")):
            found.setdefault(path.stem.split(".")[0], []).append(path)
    sdir = Path(run) / "hooks_state" / "sessions"
    if sdir.is_dir():
        for reg_path in sorted(sdir.glob("*.json")):
            try:
                reg = json.loads(reg_path.read_text())
            except ValueError:
                continue
            path = Path(str(reg.get("transcript_path", ""))).expanduser()
            if reg.get("agent") and path.is_file() and path not in found.get(reg["agent"], []):
                found.setdefault(reg["agent"], []).append(path)
    return found


def per_worker(run):
    found = discover(run)
    if not found:
        return None
    workers = {agent: summarize(paths) for agent, paths in sorted(found.items())}
    return {"per_worker": workers, "total": summarize([p for paths in found.values() for p in paths]),
            "note": "mechanical = turns whose only tool calls are tb claim/release/task_pull/task_update"}


if __name__ == "__main__":
    if len(sys.argv) < 2:
        raise SystemExit("usage: turns.py <transcript.jsonl>...")
    print(json.dumps(summarize([Path(p) for p in sys.argv[1:]]), indent=2))
