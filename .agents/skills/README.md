# Skills

Skills are automatic: the agent loads each description and reads the whole skill only when the task matches. They can also be invoked by hand with `/name`.

`.claude/skills` is a link to this folder. `less-code` and `dead-code` are shared verbatim with the other repositories; `tirith` is this repository's own.

| Skill | When it activates |
|---|---|
| [less-code](less-code/SKILL.md) | Any code change: reuse, delete and simplify before adding. |
| [dead-code](dead-code/SKILL.md) | Removing unused code, dependencies, scripts, images and docs. Reports before deleting. |
| [tirith](tirith/SKILL.md) | Any edit in this repository: claim through the Tirith daemon, read the brief, publish notices, release. |

Where a shared skill names `bun run check` or knip, this repository runs `scripts/check.sh`; `cargo machete` and clippy's dead-code lints (`unreachable_pub`, `dead_code`, `-D warnings`) are the knip equivalents, and `node .agents/skills/dead-code/scripts/scan.mjs .` still finds orphan docs and scripts. After a change, run `/simplify` (built into Claude Code) and `scripts/check.sh`.
