# Working with Serena

Serena is the code-intelligence MCP server configured for this repo
(`.serena/project.yml`, language server `rust`, backed by rust-analyzer).
It gives agents symbol-level navigation and editing, which is cheaper and
safer than reading and rewriting whole files.

## Session start

1. Load Serena's tools (in Claude Code: ToolSearch for `serena`).
2. Call `initial_instructions` once.
3. Call `list_memories` and read the ones whose names match your task.
4. If Serena reports that onboarding has not been performed, run
   `onboarding` and review the memories it writes.

## Navigation

| Want to | Use |
|---|---|
| See what a file defines | `get_symbols_overview` |
| Find a type, function, or method | `find_symbol` with a name path such as `State/claim`, `include_body` only when you need the code |
| Know who calls something before changing it | `find_referencing_symbols` |
| Locate text that is not a symbol (a string, a comment) | your own grep, then Serena for the follow-up |

Do not read whole source files to discover structure. Read whole files only
when you have the overview and need a few lines around something.

## Editing

| Change | Tool |
|---|---|
| Rewrite a whole function, impl, or struct | `replace_symbol_body` |
| Add a new item next to an existing one | `insert_after_symbol` / `insert_before_symbol` |
| Change a few lines inside a larger body | `replace_content` |
| Same edit across many files | `replace_in_files` with `dry_run` first |
| Rename a symbol everywhere | `rename_symbol` |
| Remove a symbol safely | `safe_delete_symbol` |

When `rename_symbol` or `safe_delete_symbol` succeeds, the refactor is
complete across references; do not re-verify it by re-reading files.

## Memories

Serena memories live in `.serena/memories/`, committed, and hold short
facts that are not tied to a repository path: what the current milestone
is, how to restart the daemon, which session is which agent. Name them by
topic (`workflow/project-status`, `workflow/stdio-shim`), keep them short,
and link to `docs/` instead of repeating it. Knowledge about specific
paths goes in a Tirith memory note instead; the rule is in
[02-memory.md](02-memory.md).

## Diagnostics

`get_diagnostics_for_file` returns rust-analyzer errors and warnings for a
file. Run it on every file you edited before running `cargo clippy`; it is
faster and catches most problems.
