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
contains it, and `$HOME`. No hook has handled a real Claude Code tool call
yet: the live check (task `ae630714`) was blocked by authentication, see
[Live verification](#live-verification).

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
<run>/hooks_state/stdin.jsonl       raw hook input per call, only with config "log_stdin": true
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

`stop_gate.PlaceholderGate` is deterministic. It returns done when the
task's tests pass and every acceptance criterion is mentioned in
`last_assistant_message`. A criterion counts as mentioned when one of its
backticked identifiers appears, or, without backticks, when half of its
longer words do. Otherwise the gate returns continue with the failing checks
and the unaddressed criteria. After `gate_max_continues` (3) continues on one
task it escalates (`task_update blocked`). Another gate plugs in through
`GATES` by implementing the same `evaluate`.

Which tests count is `gate_tests`:

- `task` (default): the task's own acceptance module. The gate finds the
  task in the scenario's `tasks.json` (through `<run>/meta.json` and
  `task_ids.json`, by id, then by title), copies the repository to a scratch
  directory, adds `hidden_tests/<hidden_test>` there and runs it. The full
  suite (`test_command`) still runs, but only as evidence (`suite_ok` in
  `events.jsonl`). A task the scenario does not know falls back to the suite.
  A continue names the failing test functions (or the import error), never
  their source or tracebacks.
- `suite`: `test_command` in the repository, the first prototype's gate.

Why: in a shared repository the full suite can fail because of another
agent's half-finished task. With the suite as the gate, a worker was held on,
and finally escalated, a task that was fine: in the first dry run every
worker escalated its first task. With `task`, the latest dry run marked 3 of
8 tasks done while the suite was red, and escalated none.

Threat for arm comparisons: the `task` gate is an oracle the manual arm does
not have. A worker held by it learns that its task's hidden checks fail and
which test functions fail. Acceptance numbers of a hooks run are therefore
not comparable with a manual run's; compare turns, cost, calls and
violations, or run the hooks arm with `gate_tests: "suite"`
(`HOOKS_CONFIG='{"gate_tests":"suite"}'`) when acceptance is the measure.
A task whose checks need another task's work that nobody has taken still
escalates after `gate_max_continues`; tasks with `depends_on` avoid that.

## Hook contract this relies on

Source: the Claude Code hooks reference, <https://code.claude.com/docs/en/hooks>
(docs.claude.com redirects there), read in full as Markdown on 2026-09-17.
Section anchors are given. Status: **docs** = verified by the docs,
**assumed** = not stated or inferred, **live** = needs a live check. The last
column is the live check (task `ae630714`, below): **verified** = seen in a
headless run, **partly** = the part named was seen, **not verified** = with
the reason, **-** = not part of the check.

| # | Behavior | Status | Where | Live, CLI 2.1.271, 2026-09-17 |
|---|---|---|---|---|
| 1 | Every hook gets `session_id`, `transcript_path`, `cwd`, `hook_event_name`; tool and stop events also get `permission_mode` (plus `prompt_id`, `effort`, `scratchpad_dir` on newer versions) | docs | #common-input-fields | verified live for UserPromptSubmit: `session_id` (same as the stream's), `transcript_path`, `cwd`, `prompt_id`, `permission_mode`, `hook_event_name`, `prompt`; tool and stop events not verified: no auth |
| 2 | `transcript_path` may lag the current turn; use `last_assistant_message` at Stop/SubagentStop | docs | #common-input-fields | not verified: no auth |
| 3 | `agent_id` is present only when the hook fires inside a subagent; `agent_type` is the agent name | docs | #common-input-fields | not verified: no auth |
| 4 | Hooks from settings files run inside subagents too; their tool events carry `agent_id`/`agent_type` | docs | #hook-locations | not verified: no auth |
| 5 | A hook process inherits the parent environment (so `TB_AGENT` identifies the worker) | docs | #common-input-fields | partly: `TB_AGENT` from the launching environment reached `prompt_submit.py` (identity `env`) |
| 6 | `session_id` is stable for the whole session, including across `--resume` | assumed; live | not stated | not verified: no auth |
| 7 | PreToolUse input has `tool_name`, `tool_input`, `tool_use_id`; `tool_input.file_path` for Write/Edit/Read is absolute | docs | #pretooluse-input | not verified: no auth |
| 8 | A `MultiEdit` tool exists with `file_path` | assumed | not in the docs' tool list; matching it is harmless | - |
| 9 | MCP tools are named `mcp__<server>__<tool>`; Serena's `relative_path` is repo-relative | docs (naming); assumed (field meaning, from Serena's schema) | #match-mcp-tools | - |
| 10 | Deny: exit 0 with `hookSpecificOutput.permissionDecision: "deny"`; `permissionDecisionReason` is shown to Claude | docs | #pretooluse-decision-control | not verified: no auth |
| 11 | PreToolUse `additionalContext` reaches Claude next to the tool result; it is ignored only for `defer`, so it can ride on allow (no decision) and deny | docs; live (seen together with deny) | #pretooluse-decision-control, #add-context-for-claude | not verified: no auth |
| 12 | Multiple PreToolUse decisions: deny > defer > ask > allow; exit 2 blocks with stderr as the reason | docs | #pretooluse-decision-control, #exit-code-2 | - |
| 13 | Exit codes other than 2 without valid JSON are non-blocking (the tool proceeds), and so is a hook that cannot start, so `pre_edit` catches its own errors and prints a deny | docs | #other-exit-codes | - |
| 14 | Command hook `timeout` is in seconds (default 600; UserPromptSubmit 30). A timed-out PreToolUse command hook does **not** block, so `pre_edit` timeout = `claim_wait_secs` + 90 | docs | #common-fields, #timeouts | not verified: no auth |
| 15 | PostToolUse can add context (`hookSpecificOutput.additionalContext`); top-level `decision: "block"` only adds `reason` next to the result; `updatedToolOutput` replaces what Claude sees | docs | #posttooluse-decision-control | not verified: no auth |
| 16 | PostToolUse runs only after success; PostToolUseFailure (with `error`, `is_interrupt`) runs when a started tool fails; neither runs for validation rejections or permission denials (PreToolUse does), so a claimed edit that is then denied by permissions leaves an in-flight entry (released at Stop or after 15 min) | docs | #posttooluse, #posttoolusefailure | not verified: no auth |
| 17 | PostToolUse fires concurrently for parallel tool calls; all matching hooks run in parallel | docs | #posttoolbatch, #hook-handler-fields | - |
| 18 | PreToolUse hooks for tool calls in one parallel batch may run concurrently (handled with locks) | assumed | not stated | - |
| 19 | Bash PostToolUse carries `tool_response.bashEditDiff.changedFiles` (best effort, public beta, v2.1.269+; always on in auto and bypassPermissions modes, else with `bashEditDiffEnabled`); whether project settings may set `bashEditDiffEnabled` | docs; live (the settings level) | #bash | not verified: no auth |
| 20 | Stop input: `stop_hook_active` (true when already continuing because of a stop hook) and `last_assistant_message` | docs | #stop-input | not verified: no auth |
| 21 | Stop/SubagentStop block: top-level `{"decision":"block","reason":...}`; `reason` is required and tells Claude why to continue (SubagentStop: delivered as the subagent's next instruction); `hookSpecificOutput.additionalContext` also continues, labeled feedback instead of an error | docs | #stop-decision-control, #subagentstop-input | not verified: no auth |
| 22 | Claude Code ends the turn after 8 consecutive stop blocks | docs | #stop-input | not verified: no auth |
| 23 | Whether tool calls between blocks reset that count (a worker fed more than 8 tasks by blocks needs it to) | live | not stated | not verified: no auth, and more than 8 blocks need more than `--max-turns 8` |
| 24 | Stop does not run on a user interrupt; API errors fire StopFailure instead | docs | #stop | partly: an authentication error ended the `-p` turn and no Stop hook ran |
| 25 | Stop has no matcher; SubagentStop matches `agent_type` | docs | #matcher-patterns | - |
| 26 | Matchers: `*`/empty = all; only letters, digits, `_`, `-`, spaces, `,`, `\|` = exact names; anything else = unanchored JavaScript regex (so ours are anchored `^(...)$`) | docs | #matcher-patterns | not verified: no auth |
| 27 | UserPromptSubmit `additionalContext` is added next to the prompt | docs | #add-context-for-claude | not verified: no auth |
| 28 | UserPromptSubmit fires for the prompt given to `claude -p` | assumed; live | not stated | verified: fires for the `-p` prompt, even before authentication (it ran in both failed runs) |
| 29 | Exec form: `command` plus `args: []` spawns without a shell; the docs' examples use a placeholder path that expands to an absolute path | docs; assumed (a literal absolute path) | #exec-form-and-shell-form | verified: exec form with a literal absolute path and `args: []` ran `prompt_submit.py` (exit 0) |
| 30 | Project hooks come from `.claude/settings.json` in the project where the session started; `--settings '<json>'` takes precedence over project and local settings | docs | #hook-locations, #disable-or-remove-hooks | partly: see 31; project `.claude/settings.json` not exercised |
| 31 | `--settings <file path>` works like the inline JSON | assumed | not in the hooks page | verified: `--settings <file path>` with `--setting-sources user,local` (project settings excluded) loaded and ran the hooks |
| 32 | `claude -p` treats the folder as trusted, so the run repo's hooks run headless; interactive sessions hold hooks back until the trust dialog is accepted | docs | #workspace-trust | partly: `claude --help` (2.1.271) says `-p` skips the trust dialog and silently ignores settings files that fail validation |
| 33 | Agent-tool workers spawned by a lead session started elsewhere do **not** load `<run>/repo/.claude/settings.json` (settings come from the session's project), so the hooks arm needs workers started as their own `claude -p` in `<run>/repo` (or with `--settings <run>/hooks/settings.json`) | assumed (inferred from 30); live | - | not verified: no auth |
| 34 | `additionalContext`, `reason`, and stdout are capped at 10,000 characters (we cap at 9,500) | docs | #json-output | - |
| 35 | Context text should be factual statements; imperative "system" phrasing can trip prompt-injection defenses | docs | #add-context-for-claude | - |
| 36 | A PreToolUse claim that waits for another agent's release adds that wait to the edit (dry run: claim p50 137 ms, p95 1,023 ms with 3 s waits) | assumed | - | not verified: no auth |

## Live verification

Task `ae630714`, 2026-09-17, Claude Code CLI 2.1.271 (the desktop app's
bundled binary, not on `PATH`), headless, in a scratch run outside this
repository: `setup.sh <scratch>/hooklive/run 7960 --scenario hubs --claims
file --protocol hooks` with `HOOKS_CONFIG` `autostart: false`,
`final_report: ""`, `log_stdin: true`, and the board emptied, so the Stop hook
feeds nothing. Command, run in the foreground from `<run>/repo`:

```bash
TB_AGENT=w-live claude -p '<prompt>' --settings <run>/hooks/settings.json \
  --setting-sources user,local --permission-mode acceptEdits \
  --output-format stream-json --verbose --include-hook-events --max-turns 8 \
  --model sonnet --effort low --strict-mcp-config --max-budget-usd 1 \
  --allowedTools 'Bash(sed:*)'
```

Result: blocked. That CLI has no credentials of its own (`claude auth
status`: `loggedIn: false`, `authMethod: none`; the stream's init row says
`apiKeySource: none`), and both runs ended at once with "Failed to
authenticate: OAuth session expired and could not be refreshed", before any
model call. Logging in needs the user. What the two runs did show is in the
last column of the table: UserPromptSubmit hooks from a `--settings` file run
in `-p` mode before authentication, exec form works, `TB_AGENT` reaches the
hooks, and no Stop hook runs after an API error.

CLI facts seen on the way:

- `--max-turns` is accepted although `--help` does not list it.
- Verified live: `--include-hook-events` (with stream-json) adds `system`
  rows `hook_started` and `hook_response` (`hook_id`, `hook_name`,
  `hook_event`, `session_id`; the response also `output`, `stdout`,
  `stderr`, `exit_code`, `outcome`), before the `init` row for
  UserPromptSubmit.
- Transcripts go to `~/.claude/projects/<cwd with / replaced by ->/<session_id>.jsonl`
  (`transcript_path`), so a benchmark runner copies them into
  `<run>/transcripts/` or relies on the hooks' session registry.
- A CLI started from inside a Claude Code session inherits the host's
  `CLAUDE*` variables (OAuth refresh, messaging socket, session ids). The
  runs scrubbed them so the check could not ride on the host session.

Plan for when the CLI is logged in (one run each, `--max-turns` 8):

1. Deny, context, PostToolUse, Bash write: another agent holds
   `tally/rules.py` and has sent the worker a message; the prompt edits
   `rules.py` (denied, the message as `additionalContext`), `tally/money.py`
   (granted, claim brief as context), runs `sed -i` on `tally/invoice.py`, and
   quotes every hook text in its final message (rows 1, 7, 10, 11, 15, 16, 19).
2. `--resume <session_id>` of run 1 with a one-line prompt (row 6).
3. Project settings only (no `--settings`), one board task whose acceptance
   criterion names a backticked marker, autostart on: gate continue, then done,
   then the final-report block (rows 20, 21, 27, 30, 32; `stop_hook_active`
   in `stdin.jsonl`).
4. A subagent spawned with the Agent tool edits a file, `--settings` (rows 3,
   4, subagent identity in stdin).
5. A session started outside the run (`--add-dir <run>/repo`, no
   `--settings`) whose subagent edits the run repo (row 33).
6. Another agent holds `tally/models.py`; the foreground driver releases it
   5 s after `pre_edit` starts waiting (row 36).

## Plumbing check (no LLM)

```bash
TIRITH_BIN=<release build with wait_secs> examples/e2e/dryrun/run_hooks.sh <lab>/hooks-dry 7892 3
```

The check runs setup (hubs, `--claims file --protocol hooks`, short
waits through `HOOKS_CONFIG`), then `fake_worker_hooks.py selfcheck`: deny
with holder, brief as context, in-flight edits survive a parallel call,
release on the next non-edit call, Bash writes claimed after the fact, a
helper SubagentStop does nothing, auto identity, settings matchers,
acceptance-criteria parsing, the gate's test scope (own checks pass while the
suite is red: done; own checks fail: continue, then escalate; `suite` scope;
unknown task), violation counting on a fixture, turn classes, and a hooks
prompt with no protocol steps. Then three scripted workers run in parallel.
They only send hooks the documented stdin JSON, obey the outputs (the last
worker runs without `TB_AGENT`), and write synthetic transcripts with tool
results (`dryrun/transcript.py`); then the run is scored and stopped.

The fakes splice final reference symbols, which use other tasks' symbols
(`currency-precision` needs `free-shipping` and `tax-exempt`, `gift-wrap`
and `small-order-fee` need each other). A fake therefore also splices the
reference symbols of the tasks its own checks need, found once per run on the
baseline (`<run>/fake_closures.json`). Without that, the `task` gate held
every worker on a task whose checks waited on a task nobody had taken:
1 of 8 done, 3 escalated.

Measured 2026-09-17 on a 1.0.5 release build (dry run `hooks-dry3`), gate
`task`, suite as evidence:

- Selfcheck: 32/32 pass.
- Board: 8/8 tasks done, 48/48 hidden tests, 4/4 integration, 0 regressions.
  3 of the 8 done verdicts came while the full suite was red; 0 escalations.
- Tirith calls: 104 by hooks, 11 by the model (selfcheck probes and the
  breaking task's notice).
- Hook decisions: 37 claims, 37 releases, 8 tasks done, 5 tasks fed on stop,
  3 tasks fed at start, 1 gate continue (a deliberately bare "Done."), 3
  final-report blocks, 0 hook errors.
- Latency: claim p50 137 ms, p95 1,023 ms; gate p50 272 ms (the task's module
  in a scratch copy plus the suite; the import-check gate of the first
  prototype took 51 ms).
- Synthetic transcripts: 112 turns, 0 mechanical, 1 coordination-only (the
  notice); 36 tool edits, 0 without a claim.

Before this, with the full suite as the gate, every worker escalated its
first task (see Stop gate).

Not covered by the fake: real Claude Code parsing of these outputs, the
8-block cap, trust and permission prompts, and the latency of hooks on every
tool call in a real session.
