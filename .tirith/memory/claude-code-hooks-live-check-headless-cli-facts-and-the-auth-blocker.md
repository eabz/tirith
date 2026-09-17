---
id: 7a44cfd8-3679-4a5c-b24e-ea02eeb23cad
permalink: claude-code-hooks-live-check-headless-cli-facts-and-the-auth-blocker
title: "Claude Code hooks live check: headless CLI facts and the auth blocker"
kind: gotcha
tags:
- bench
- e2e
- hooks
- claude-code
paths:
- examples/e2e/hooks
author: hooks-finish
updated_by: hooks-finish
created_at: 2026-09-17T15:41:48Z
updated_at: 2026-09-17T15:41:48Z
---

Task ae630714, 2026-09-17, Claude Code CLI 2.1.271 (desktop app bundle: ~/Library/Application Support/Claude/claude-code/2.1.271/claude.app/Contents/MacOS/claude, not on PATH).

- [gotcha] That CLI is not logged in (`claude auth status`: loggedIn false, authMethod none; init row apiKeySource none). `-p` runs end at once with "Failed to authenticate: OAuth session expired and could not be refreshed", with or without the host session's env. Logging in needs the user; blocked, 2 of 12 allowed runs used. #auth
- [gotcha] A CLI started from inside a Claude Code session inherits the host's CLAUDE* env (CLAUDE_CODE_SDK_HAS_OAUTH_REFRESH, CLAUDE_CODE_MESSAGING_SOCKET/TOKEN, CLAUDE_CODE_SESSION_ID, CLAUDECODE...). Scrub every CLAUDE* variable before a verification run. #env
- [fact] Verified live: UserPromptSubmit hooks from `--settings <file>` (with `--setting-sources user,local`, project excluded) run in `-p` mode before authentication; exec form (`command` absolute path, `args: []`) works; TB_AGENT from the launch env reaches hooks; stdin = session_id, transcript_path, cwd, prompt_id, permission_mode, hook_event_name, prompt. No Stop hook ran after the API (auth) error. #hooks
- [fact] `--include-hook-events` + stream-json emits system rows hook_started/hook_response (hook_id, hook_name, hook_event, session_id; response adds output, stdout, stderr, exit_code, outcome). `--max-turns` is accepted though missing from --help. Transcripts go to ~/.claude/projects/<cwd with / -> ->/<session_id>.jsonl. #cli
- [gotcha] User settings (~/.claude/settings.json) carry the env and an enabled plugin; excluding the user source would drop them, so isolate hooks with `--setting-sources user,local` plus `--settings`, and `--strict-mcp-config` for no MCP servers. #cli
- [handoff] Prepared: scratchpad hooklive/drive.py (foreground driver with timed release), NOTES.md, and a 6-run plan in examples/e2e/hooks/README.md "Live verification" (deny+context+PostToolUse+bash diff, --resume, Stop gate via project settings, subagent, lead-elsewhere subagent = row 33, claim-wait latency). #handoff
