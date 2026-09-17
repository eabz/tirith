#!/usr/bin/env python3
"""Render a worker prompt for one run. Usage: prompt.py <run_dir> <agent> [--skeleton].

Reads <run_dir>/meta.json (scenario, claims, hold, wait, pull) and prints
the scenario's WORKER_PROMPT.md with RUN_DIR and AGENT_NAME filled in and
the protocol placeholders replaced by paragraph files:

  {{CLAIMS}} prompt/claims_<claims>.md   file | symbol
  {{HOLD}}   prompt/hold_<hold>.md       task | edit
  {{WAIT}}   prompt/wait_<wait>.md       sleep | server
  {{PULL}}   prompt/pull_<pull>.md       poll | wait

A paragraph is read from the scenario's own prompt/ directory first, then
from the kit's (Forge lives at the kit root, so its prompt/ is the shared
one). Every other byte is identical across arms, so two prompts for the same
agent differ only in those paragraphs (and the run directory).
--skeleton prints the template with only RUN_DIR and AGENT_NAME filled in,
which bench/prompt_diff.sh compares across arms to prove that.

With meta.json protocol "hooks" it renders WORKER_PROMPT_HOOKS.md instead:
the same prompt with the protocol steps the hooks perform removed.
"""

import json
import sys
from pathlib import Path

# Runs set up before hold/pull existed: Forge held claims for the task, hubs
# used edit windows; both polled task_pull.
LEGACY_HOLD = {"forge": "task"}


def arm_values(meta):
    scenario = meta.get("scenario") or "forge"
    return {
        "CLAIMS": meta.get("claims") or "file",
        "HOLD": meta.get("hold") or LEGACY_HOLD.get(scenario, "edit"),
        "WAIT": meta.get("wait") or "sleep",
        "PULL": meta.get("pull") or "poll",
    }


def paragraph(meta, scenario_dir, name):
    for base in (scenario_dir / "prompt", Path(meta["kit"]) / "prompt"):
        if (base / name).is_file():
            return (base / name).read_text().strip()
    raise SystemExit("missing prompt paragraph %s (looked in %s/prompt and %s/prompt)"
                     % (name, scenario_dir, meta["kit"]))


def render(run: Path, agent: str, skeleton: bool = False) -> str:
    meta = json.loads((run / "meta.json").read_text())
    scenario_dir = Path(meta.get("scenario_dir") or meta["kit"])
    name = "WORKER_PROMPT_HOOKS.md" if meta.get("protocol") == "hooks" else "WORKER_PROMPT.md"
    text = (scenario_dir / name).read_text()
    if not skeleton:
        for key, value in arm_values(meta).items():
            marker = "{{%s}}" % key
            if marker in text:
                text = text.replace(marker, paragraph(meta, scenario_dir, "%s_%s.md" % (key.lower(), value)))
        if "{{" in text:
            raise SystemExit("unreplaced placeholder in %s" % (scenario_dir / name))
    return text.replace("RUN_DIR", str(run)).replace("AGENT_NAME", agent)


if __name__ == "__main__":
    args = [a for a in sys.argv[1:] if a != "--skeleton"]
    if len(args) != 2:
        raise SystemExit("usage: prompt.py <run_dir> <agent> [--skeleton]")
    sys.stdout.write(render(Path(args[0]).resolve(), args[1], skeleton="--skeleton" in sys.argv[1:]))
