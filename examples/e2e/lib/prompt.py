#!/usr/bin/env python3
"""Render a worker prompt for one run. Usage: prompt.py <run_dir> <agent>.

Reads <run_dir>/meta.json (scenario, claims, wait) and prints the
scenario's WORKER_PROMPT.md with RUN_DIR and AGENT_NAME filled in and, for
scenarios with arm paragraphs, {{CLAIMS}} and {{WAIT}} replaced by
prompt/claims_<claims>.md and prompt/wait_<wait>.md. Every other byte is
identical across arms, so two prompts for the same agent differ only in
those paragraphs (and the run directory).

With meta.json protocol "hooks" it renders WORKER_PROMPT_HOOKS.md instead:
the same prompt with the protocol steps the hooks perform removed.
"""

import json
import sys
from pathlib import Path


def render(run: Path, agent: str) -> str:
    meta = json.loads((run / "meta.json").read_text())
    scenario_dir = Path(meta.get("scenario_dir") or meta["kit"])
    name = "WORKER_PROMPT_HOOKS.md" if meta.get("protocol") == "hooks" else "WORKER_PROMPT.md"
    text = (scenario_dir / name).read_text()
    blocks = {"{{CLAIMS}}": "claims_%s.md" % meta.get("claims"), "{{WAIT}}": "wait_%s.md" % meta.get("wait")}
    for marker, name in blocks.items():
        if marker in text:
            text = text.replace(marker, (scenario_dir / "prompt" / name).read_text().strip())
    if "{{" in text:
        raise SystemExit("unreplaced placeholder in %s" % (scenario_dir / name))
    return text.replace("RUN_DIR", str(run)).replace("AGENT_NAME", agent)


if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit("usage: prompt.py <run_dir> <agent>")
    sys.stdout.write(render(Path(sys.argv[1]).resolve(), sys.argv[2]))
