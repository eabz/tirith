---
id: 2137c4c0-13da-41cf-ad55-599f75e44875
permalink: agent-config-and-graphify-traps-when-editing-skills-settings-and-the-graph-adr-0035
title: "Agent config and graphify: traps when editing skills, settings and the graph (ADR-0035)"
kind: note
tags: []
paths:
- .agents
- .claude
- .graphifyignore
- .mcp.json
- AGENTS.md
author: claude-cleanup
updated_by: claude-cleanup
created_at: 2026-09-24T04:53:23Z
updated_at: 2026-09-24T04:58:14Z
---

- [trap] `.claude/settings.json` denies Read of `graphify-out/.graphify_*`, so the Write tool refuses graphify chunk files there. Extraction subagents write chunks to the session scratchpad; the build copies them into `graphify-out/`.
- [fact] The graph covers code, `docs/`, and committed `.tirith/` decisions and memory; `.graphifyignore` drops `.tirith/runtime/`, agent config, HTML and images. `graphify update .` refreshes code only and resets community names to "Community N"; changed docs or `.tirith` notes need `/graphify --update` (semantic pass).
- [fact] 2026-09-23 cleanup kept four decisions (CLI never retries, self-update, status JSON outcomes, tray prune rule) and removed every contract and the notice log; the rest were superseded or restated ADRs.
- [trap] `less-code` and `dead-code` in `.agents/skills/` are copied verbatim from the portal repo; never edit them here. They say `bun run check` and knip; here that is `scripts/check.sh`, `cargo machete` and clippy's dead-code lints (see `.agents/skills/README.md`).
- [fact] `.claude/skills` is a symlink to `../.agents/skills`; `.gitignore` versions only it and `settings.json` under `.claude/`.
