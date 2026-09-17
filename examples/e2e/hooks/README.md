# Hook-driven coordination protocol (prototype)

Task `1c2fe7d3`, step 1: the mechanical coordination steps (claim, release,
task pull, task done) run in Claude Code hooks instead of the model loop. In
the lab's Forge runs (496 worker turns, 12 transcripts) the strict count
(`lib/turns.py`) found 13.9% of turns and 11.8% of estimated cost in turns
that only ran `tb claim|release|task_pull|task_update`. Counting every
coordination-only turn with at least one of those calls gives 18.5% and
16.1%. The lead measured 19.8% and 17.1%.

Nothing here touches the Tirith daemon, `src/`, the Tirith checkout's
`.claude/`, or `~/.claude/`. `setup.sh --protocol hooks` copies these scripts
into a run directory and writes the settings into that run's repo copy only.
It refuses a run directory inside the Tirith repository, a run directory that
contains it, and `$HOME`. Nothing has run in a real Claude Code session yet.
Live verification is task `2b973249`.

## Files

| file | Claude Code event | what it does |
|---|---|---|
| `pre_edit.py` | PreToolUse on edit tools | claims the target file with `wait_secs` (a file already held is reused); on refusal, **denies** the edit with the holder in the reason; adds the claim brief as context only when it has rows, plus inbox messages and lost leases |
| `post_edit.py` | PostToolUse (edit tools, Bash), PostToolUseFailure (edit tools) | marks the file pending release; for Bash, claims files reported in `tool_response.bashEditDiff.changedFiles` after the fact, and warns Claude if another agent holds one |
| `pre_other.py` | PreToolUse on `*` (skips edit tools) | releases pending files that have no edit in flight: the edit window closes on the next non-edit tool call |
| `stop.py` | Stop, SubagentStop | releases everything; with a task, asks the stop gate (done: `task_update done`; continue: block the stop with the gate's reason; escalate: `task_update blocked` and stop for real); without a task, `task_pull` and block the stop with the task text; while todo tasks wait on dependencies, polls inside the hook within an idle budget; when the board is empty, asks once for the final report, then allows the stop |
| `prompt_submit.py` | UserPromptSubmit | an agent without a task gets its first task as context, which saves the turn it would spend stopping |
| `stop_gate.py` | (library) | gate interface `load_gate(name).evaluate(GateInput) -> Verdict(done/continue/escalate)` and the deterministic placeholder |
| `common.py` | (library) | run discovery, identity, per-agent state with locks, `tb` calls with `TB_ORIGIN=hook`, output helpers |
| `settings.py` | (generator) | prints the settings JSON for one run |

Edit tools: `Edit`, `Write`, `MultiEdit`, `NotebookEdit`, and Serena's
`replace_symbol_body`, `insert_after_symbol`, `insert_before_symbol`,
`replace_content`, `rename_symbol`, `safe_delete_symbol` (file claim on
`relative_path`; `rename_symbol` also changes callers in other files, which
this prototype does not claim).

## Run layout

```
<run>/hooks/*.py, settings.json     copies of this directory + generated settings (for `claude --settings`)
<run>/repo/.claude/settings.json    same settings, committed with the seeded baseline (not scored as code)
<run>/hooks_state/config.json       knobs (common.py DEFAULTS, overridden by HOOKS_CONFIG at setup)
<run>/hooks_state/identity.json     session_id / agent_id -> Tirith agent name
<run>/hooks_state/sessions/<id>.json  per Claude Code session: agent, transcript_path, subagents
<run>/hooks_state/agents/<name>.json  per Tirith agent: task, files {held, pending, inflight}, counters
<run>/hooks_state/events.jsonl      one line per hook decision; score.py summarizes it under "hooks"
<run>/hooks_state/errors.log        hook exceptions
```

State is kept per Tirith agent rather than per session, because claims and
tasks belong to the agent. The session registry keeps the per-session facts
(transcripts, subagents). Parallel hooks for one agent coordinate through a
state lock (never held across a Tirith call) and a coordination lock that
serializes the agent's claim and release calls. A release that finds that
lock busy skips and retries on the next call.

## Agent identity

Resolution order, first match wins:

1. `identity.json` `agents[agent_id]`: a subagent that is itself a worker (mapped by the harness).
2. `TB_AGENT` in the environment Claude Code was started with (hooks inherit it). This is the intended mode: `TB_AGENT=w1 claude -p ...`.
3. `identity.json` `sessions[session_id]`.
4. Auto: `hk-<first 8 alphanumerics of session_id>`, recorded under 3.

Tool calls inside a helper subagent resolve to the parent's name through
rule 2 or 3, so they share the parent's claims. A `SubagentStop` acts only
under rule 1, so a helper finishing never completes or pulls the worker's
task.

What can be verified: `agent_id` is present only inside subagents and is
unique per subagent, and hooks inherit the environment (docs). What cannot
be verified from the docs: that `session_id` stays the same across
`--resume`, and whether a subagent's `agent_id` is stable across resumes of
that subagent. Neither matters for `claude -p` workers that are not resumed.

## Stop gate

`stop_gate.PlaceholderGate` is deterministic. It returns done when
`test_command` (default `python3 -m unittest`) passes in the repo and every
acceptance criterion is mentioned in `last_assistant_message`. A criterion
counts as mentioned when one of its backticked identifiers appears, or,
without backticks, when half of its longer words do. Otherwise the gate
returns continue with the failing test tail and the unaddressed criteria.
After `gate_max_continues` (3) it escalates. Another gate plugs in through
`GATES` by implementing the same `evaluate`.

Known weakness, seen in the dry run: in a shared repo the full suite can fail
because of another agent's half-finished task. A worker is then held on, and
finally escalates, a task that is fine. In the first dry run every worker
escalated its first task this way. That run's fakes splice final reference
symbols, which depend on other tasks' symbols, so it is an extreme case, but
LLM workers can hit it too. A real gate should scope tests to the task or
tell others' failures from the worker's own.

## Hook contract this relies on

Source: the Claude Code hooks reference, <https://code.claude.com/docs/en/hooks>
(docs.claude.com redirects there), read in full as Markdown on 2026-09-17.
Section anchors are given. Status: **docs** = verified by the docs,
**assumed** = not stated or inferred, **live** = needs a live check (task `2b973249`).

| # | Behavior | Status | Where |
|---|---|---|---|
| 1 | Every hook gets `session_id`, `transcript_path`, `cwd`, `hook_event_name`; tool and stop events also get `permission_mode` (plus `prompt_id`, `effort`, `scratchpad_dir` on newer versions) | docs | #common-input-fields |
| 2 | `transcript_path` may lag the current turn; use `last_assistant_message` at Stop/SubagentStop | docs | #common-input-fields |
| 3 | `agent_id` is present only when the hook fires inside a subagent; `agent_type` is the agent name | docs | #common-input-fields |
| 4 | Hooks from settings files run inside subagents too; their tool events carry `agent_id`/`agent_type` | docs | #hook-locations |
| 5 | A hook process inherits the parent environment (so `TB_AGENT` identifies the worker) | docs | #common-input-fields |
| 6 | `session_id` is stable for the whole session, including across `--resume` | assumed; live | not stated |
| 7 | PreToolUse input has `tool_name`, `tool_input`, `tool_use_id`; `tool_input.file_path` for Write/Edit/Read is absolute | docs | #pretooluse-input |
| 8 | A `MultiEdit` tool exists with `file_path` | assumed | not in the docs' tool list; matching it is harmless |
| 9 | MCP tools are named `mcp__<server>__<tool>`; Serena's `relative_path` is repo-relative | docs (naming); assumed (field meaning, from Serena's schema) | #match-mcp-tools |
| 10 | Deny: exit 0 with `hookSpecificOutput.permissionDecision: "deny"`; `permissionDecisionReason` is shown to Claude | docs | #pretooluse-decision-control |
| 11 | PreToolUse `additionalContext` reaches Claude next to the tool result; it is ignored only for `defer`, so it can ride on allow (no decision) and deny | docs; live (seen together with deny) | #pretooluse-decision-control, #add-context-for-claude |
| 12 | Multiple PreToolUse decisions: deny > defer > ask > allow; exit 2 blocks with stderr as the reason | docs | #pretooluse-decision-control, #exit-code-2 |
| 13 | Exit codes other than 2 without valid JSON are non-blocking (the tool proceeds), and so is a hook that cannot start, so `pre_edit` catches its own errors and prints a deny | docs | #other-exit-codes |
| 14 | Command hook `timeout` is in seconds (default 600; UserPromptSubmit 30). A timed-out PreToolUse command hook does **not** block, so `pre_edit` timeout = `claim_wait_secs` + 90 | docs | #common-fields, #timeouts |
| 15 | PostToolUse can add context (`hookSpecificOutput.additionalContext`); top-level `decision: "block"` only adds `reason` next to the result; `updatedToolOutput` replaces what Claude sees | docs | #posttooluse-decision-control |
| 16 | PostToolUse runs only after success; PostToolUseFailure (with `error`, `is_interrupt`) runs when a started tool fails; neither runs for validation rejections or permission denials (PreToolUse does), so a claimed edit that is then denied by permissions leaves an in-flight entry (released at Stop or after 15 min) | docs | #posttooluse, #posttoolusefailure |
| 17 | PostToolUse fires concurrently for parallel tool calls; all matching hooks run in parallel | docs | #posttoolbatch, #hook-handler-fields |
| 18 | PreToolUse hooks for tool calls in one parallel batch may run concurrently (handled with locks) | assumed | not stated |
| 19 | Bash PostToolUse carries `tool_response.bashEditDiff.changedFiles` (best effort, public beta, v2.1.269+; always on in auto and bypassPermissions modes, else with `bashEditDiffEnabled`); whether project settings may set `bashEditDiffEnabled` | docs; live (the settings level) | #bash |
| 20 | Stop input: `stop_hook_active` (true when already continuing because of a stop hook) and `last_assistant_message` | docs | #stop-input |
| 21 | Stop/SubagentStop block: top-level `{"decision":"block","reason":...}`; `reason` is required and tells Claude why to continue (SubagentStop: delivered as the subagent's next instruction); `hookSpecificOutput.additionalContext` also continues, labeled feedback instead of an error | docs | #stop-decision-control, #subagentstop-input |
| 22 | Claude Code ends the turn after 8 consecutive stop blocks | docs | #stop-input |
| 23 | Whether tool calls between blocks reset that count (a worker fed more than 8 tasks by blocks needs it to) | live | not stated |
| 24 | Stop does not run on a user interrupt; API errors fire StopFailure instead | docs | #stop |
| 25 | Stop has no matcher; SubagentStop matches `agent_type` | docs | #matcher-patterns |
| 26 | Matchers: `*`/empty = all; only letters, digits, `_`, `-`, spaces, `,`, `\|` = exact names; anything else = unanchored JavaScript regex (so ours are anchored `^(...)$`) | docs | #matcher-patterns |
| 27 | UserPromptSubmit `additionalContext` is added next to the prompt | docs | #add-context-for-claude |
| 28 | UserPromptSubmit fires for the prompt given to `claude -p` | assumed; live | not stated |
| 29 | Exec form: `command` plus `args: []` spawns without a shell; the docs' examples use a placeholder path that expands to an absolute path | docs; assumed (a literal absolute path) | #exec-form-and-shell-form |
| 30 | Project hooks come from `.claude/settings.json` in the project where the session started; `--settings '<json>'` takes precedence over project and local settings | docs | #hook-locations, #disable-or-remove-hooks |
| 31 | `--settings <file path>` works like the inline JSON | assumed | not in the hooks page |
| 32 | `claude -p` treats the folder as trusted, so the run repo's hooks run headless; interactive sessions hold hooks back until the trust dialog is accepted | docs | #workspace-trust |
| 33 | Agent-tool workers spawned by a lead session started elsewhere do **not** load `<run>/repo/.claude/settings.json` (settings come from the session's project), so the hooks arm needs workers started as their own `claude -p` in `<run>/repo` (or with `--settings <run>/hooks/settings.json`) | assumed (inferred from 30); live | - |
| 34 | `additionalContext`, `reason`, and stdout are capped at 10,000 characters (we cap at 9,500) | docs | #json-output |
| 35 | Context text should be factual statements; imperative "system" phrasing can trip prompt-injection defenses | docs | #add-context-for-claude |

## Plumbing check (no LLM)

```bash
TIRITH_BIN=<release build with wait_secs> examples/e2e/dryrun/run_hooks.sh <lab>/hooks-dry 7892 3
```

The check runs setup (hubs, `--claims file --protocol hooks`, short
waits through `HOOKS_CONFIG`), then `fake_worker_hooks.py selfcheck`: deny
with holder, brief as context, in-flight edits survive a parallel call,
release on the next non-edit call, Bash writes claimed after the fact, a
helper SubagentStop does nothing, auto identity, settings matchers,
acceptance-criteria parsing, turn classes, and a hooks prompt with no
protocol steps. Then three scripted workers run in parallel. They only send
hooks the documented stdin JSON and obey the outputs (the last worker runs
without `TB_AGENT`), then the run is scored and stopped.

Measured 2026-09-17 on a 1.0.4 dev build (dry run `dry3`), stop gate test
command replaced by an import check:

- Selfcheck: 26/26 pass.
- Board: 8/8 tasks done, 48/48 hidden tests, 4/4 integration, 0 regressions.
- Tirith calls: 84 by hooks, 11 by the model (selfcheck probes and the
  breaking task's notice).
- Hook decisions: 27 claims, 27 releases, 8 tasks done, 5 tasks fed on stop,
  3 tasks fed at start, 1 gate continue (a deliberately bare "Done."), 3
  final-report blocks, 0 hook errors.
- Latency: claim p50 152 ms, p95 684 ms; gate p50 51 ms.
- Model turns in the synthetic transcripts: 0 of 82 mechanical.

With the full suite as the gate (dry run `dry2`), every worker escalated its
first task (see Stop gate).

Not covered by the fake: real Claude Code parsing of these outputs, the
8-block cap, trust and permission prompts, and the latency of hooks on every
tool call in a real session.
