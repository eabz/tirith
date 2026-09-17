#!/usr/bin/env python3
"""Print the Claude Code hook settings for one run. Usage: settings.py <run_dir>.

setup.sh --protocol hooks writes this to <run_dir>/repo/.claude/settings.json
(and a copy to <run_dir>/hooks/settings.json for `claude --settings`). It
refuses any run directory inside the kit's own repository, so this can
never configure hooks for the agents working on Tirith itself.
Hook commands use exec form (`args` present): absolute script paths, no shell.
"""

import json
import sys
from pathlib import Path

EDIT_NAMES = ("Edit|Write|MultiEdit|NotebookEdit|mcp__serena__(replace_symbol_body|insert_after_symbol|"
              "insert_before_symbol|replace_content|rename_symbol|safe_delete_symbol)")
EDIT_MATCHER = "^(%s)$" % EDIT_NAMES
EDIT_OR_BASH_MATCHER = "^(%s|Bash)$" % EDIT_NAMES


def command(run, script, timeout):
    return {"type": "command", "command": str(run / "hooks" / script), "args": [], "timeout": timeout}


def settings(run, claim_wait_secs=120):
    return {
        "hooks": {
            "UserPromptSubmit": [{"hooks": [command(run, "prompt_submit.py", 60)]}],
            "PreToolUse": [
                {"matcher": EDIT_MATCHER, "hooks": [command(run, "pre_edit.py", claim_wait_secs + 90)]},
                {"matcher": "*", "hooks": [command(run, "pre_other.py", 60)]},
            ],
            "PostToolUse": [{"matcher": EDIT_OR_BASH_MATCHER, "hooks": [command(run, "post_edit.py", 60)]}],
            "PostToolUseFailure": [{"matcher": EDIT_MATCHER, "hooks": [command(run, "post_edit.py", 60)]}],
            "Stop": [{"hooks": [command(run, "stop.py", 600)]}],
            "SubagentStop": [{"hooks": [command(run, "stop.py", 600)]}],
        },
        "bashEditDiffEnabled": True,
    }


def main():
    if len(sys.argv) != 2:
        raise SystemExit("usage: settings.py <run_dir>")
    run = Path(sys.argv[1]).resolve()
    kit_repo = Path(__file__).resolve().parents[3]
    if (kit_repo / "Cargo.toml").is_file() and (run == kit_repo or kit_repo in run.parents):
        raise SystemExit("refusing: run directory %s is inside the Tirith repository %s" % (run, kit_repo))
    wait = 120
    config = run / "hooks_state" / "config.json"
    if config.is_file():
        wait = int(json.loads(config.read_text()).get("claim_wait_secs", wait))
    print(json.dumps(settings(run, wait), indent=2))


if __name__ == "__main__":
    main()
