#!/usr/bin/env python3
"""Synthetic Claude Code transcripts for the LLM-free fake workers.

The scorer reads worker transcripts twice: lib/turns.py classifies model
turns (assistant rows grouped by message id), and lib/violations.py times
repository writes at their tool_result rows against the writer's claims.
This writes the subset of the Claude Code JSONL format both read:

  assistant row  {"type":"assistant","timestamp":..., "message":{"id","role","content":[tool_use|text],"usage"}}
  user row       {"type":"user","timestamp":..., "message":{"role":"user","content":[tool_result]}}

Timestamps are UTC with milliseconds, like the coordination logs, so a write
recorded right after it happened sorts correctly against claim and release
rows. Every tool call is its own turn with a fixed usage.

Library: Transcript(path, session).use(name, input) -> id; .result(id, is_error); .text(message).
CLI (for shell fakes), one call after the command has run:
  transcript.py <file> <session> tool <name> '<input json>' [--error]
  transcript.py <file> <session> text '<message>'
"""

import fcntl
import json
import sys
import time
import uuid

USAGE = {"input_tokens": 5, "cache_read_input_tokens": 2000, "cache_creation_input_tokens": 100,
         "output_tokens": 150}


def now_iso():
    t = time.time()
    return time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime(t)) + ".%03dZ" % int((t - int(t)) * 1000)


class Transcript:
    def __init__(self, path, session=None):
        self.path = path
        self.session = session or uuid.uuid4().hex[:8]

    def _append(self, row):
        self.path.parent.mkdir(parents=True, exist_ok=True)
        with open(str(self.path), "a") as handle:
            fcntl.flock(handle, fcntl.LOCK_EX)
            handle.write(json.dumps(row) + "\n")
            fcntl.flock(handle, fcntl.LOCK_UN)

    def use(self, name, tool_input):
        """One assistant turn with a single tool_use; returns its id."""
        use_id = "toolu_%s" % uuid.uuid4().hex[:16]
        self._append({"type": "assistant", "timestamp": now_iso(), "sessionId": self.session,
                      "message": {"id": "msg_%s" % uuid.uuid4().hex[:16], "role": "assistant",
                                  "content": [{"type": "tool_use", "id": use_id, "name": name,
                                               "input": tool_input}],
                                  "usage": dict(USAGE)}})
        return use_id

    def result(self, use_id, is_error=False, text=""):
        """The tool_result for use_id, stamped now (call it after the tool's effect)."""
        self._append({"type": "user", "timestamp": now_iso(), "sessionId": self.session,
                      "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": use_id,
                                                               "is_error": bool(is_error), "content": text}]}})

    def tool(self, name, tool_input, is_error=False):
        """use + result at once, for a call whose effect already happened."""
        self.result(self.use(name, tool_input), is_error)

    def text(self, message):
        self._append({"type": "assistant", "timestamp": now_iso(), "sessionId": self.session,
                      "message": {"id": "msg_%s" % uuid.uuid4().hex[:16], "role": "assistant",
                                  "content": [{"type": "text", "text": message}], "usage": dict(USAGE)}})


def main(argv):
    from pathlib import Path
    if len(argv) >= 5 and argv[3] == "tool":
        Transcript(Path(argv[1]), argv[2]).tool(argv[4], json.loads(argv[5]) if len(argv) > 5 else {},
                                                is_error="--error" in argv[6:])
        return 0
    if len(argv) == 5 and argv[3] == "text":
        Transcript(Path(argv[1]), argv[2]).text(argv[4])
        return 0
    sys.stderr.write("usage: transcript.py <file> <session> tool <name> '<input json>' [--error] | "
                     "<file> <session> text '<message>'\n")
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
