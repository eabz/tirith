# Running a swarm on your project

This is what to tell your agents so that a session knows, the moment it
starts, whether it is the lead or a worker and what each does. The daemon
serves the same rules as `guide` with topic `roles`; this page is the
human-readable version, with the text to paste into your project.

Setup is two steps, both in [02-client-setup.md](02-client-setup.md):
install `tirith` and register `tirith stdio` with your client. State lives
in `.tirith/`; commit `contracts/`, `decisions/`, `memory/` and
`notices.jsonl`.

## 1. Put the protocol in the project

Add this to the file your agents read first (`AGENTS.md`, `CLAUDE.md`, a
Cursor rule). It is short on purpose: the daemon's `guide` tool carries
the rest, and it matches the version that answers.

```markdown
## Coordination (Tirith)

This repository runs a Tirith daemon (`tirith stdio` in the MCP config).
Start every session with `guide` (topic `roles`) and `memory_search` with
no query. Pick one stable `agent` name and pass it on every call.

- The session the human talks to is the **lead**. Before spawning anyone
  it claims `.tirith/lead` (ttl 3600). It publishes the contracts,
  creates the tasks, spawns one worker per task whose paths overlap no
  other open task, reads its inbox on every call, reviews each worker's
  diff, and is the only session that commits, pushes, opens pull
  requests or deploys. It releases `.tirith/lead` last.
- A spawned session is a **worker**. It loops on `task_pull` with
  `wait_secs: 120`, claims the task's paths (a `path#Symbol` anchor on a
  file others edit), reads the brief, edits, runs the module tests,
  publishes a notice for every rename or signature change, writes a
  memory note for what the next agent must know, releases, marks the
  task done and reports to the lead with `message_send`. It never claims
  `.tirith/lead`, never messages `human`, never commits or pushes, and
  proposes contracts, tasks and decisions to the lead instead of
  writing them (the daemon refuses them with `conflict`).
- Claim before editing. A `conflict` means do not edit: take other work
  or claim again with `wait_secs`. A `lost` field means claim again.
```

## 2. What the lead does when it starts

In this order, before any code:

1. `memory_search` with no query, then `status`: what happened last, and
   whether another lead is live (`lead` in the result).
2. `claim` `.tirith/lead` with `ttl_secs: 3600` and a reason naming the
   swarm. If that is refused, another session leads; join it as a worker
   or wait.
3. Plan. Read the request, the relevant notes and decisions, and split
   the work into tasks whose `paths` do not overlap. A task whose
   result another task needs gets `depends_on`.
4. `contract_publish` every interface two tasks share, before either
   task exists. The contract is what the workers build against; a
   worker that needs it changed proposes the change and waits for the
   republish.
5. `task_create` each task with `paths`, `priority` and `depends_on`.
6. Decide whether to spawn (next section). If yes, spawn and brief.
7. Loop: every call returns the inbox. Answer proposals, republish a
   contract when a proposal is right, relay to `human` only what the
   human alone can settle, and `task_create` the follow-up work workers
   report.
8. When the board is empty: review each diff, simplify, run the full
   check once, record decisions, commit, push, open the pull request,
   deploy. Release `.tirith/lead`.

### When to spawn, and how many

| Board | Do |
|---|---|
| One task, or every task touches the same files | No swarm. Do it in the lead session. |
| Two or more `todo` tasks with disjoint `paths` | One worker per such task, at once. |
| More disjoint tasks than you want sessions | Spawn for the highest priorities; the rest wait on the board and the next free worker pulls them. |
| A task depends on another | Spawn for the dependency only; the dependent is pulled when its dependency is done. |

Never spawn more workers than free tasks: a worker with nothing to pull
answers `none` and stops, and spawning it cost a session. A worker does
not spawn workers.

### Which model

The lead plans, designs contracts, reviews and merges. Run it on the
strongest model you have. A worker receives its contract, its paths and
its brief; what is left is execution. Run workers on a smaller, cheaper
and faster model, and move a task to the lead when a worker reports it
blocked twice. The split is the point of the roles: judgment once, at
the lead; execution many times, cheaply.

## 3. The worker brief

The spawn prompt for every worker, with the placeholders filled:

```text
You are worker <AGENT NAME> in a Tirith swarm led by <LEAD NAME>.
Pass agent="<AGENT NAME>" on every Tirith call.

Your task is <TASK ID>: <TITLE>. It is already assigned to you.
Contract: <CONTRACT NAME> (read it with contract_get). Paths: <PATHS>.

Loop:
1. claim the paths before editing (ttl_secs 600). On a file other
   workers edit, claim path#Symbol and edit by replacement. Read the
   brief the claim returns before touching anything.
2. Edit. Run the module tests for your paths, not the whole suite.
3. notice_publish for every rename, signature change, removal or move
   that reaches files you did not claim.
4. memory_write what the next agent needs to know about these paths,
   if the repository does not already say it.
5. release, then task_update <TASK ID> done with a one-line note.
6. task_pull with wait_secs 120. If it returns a task, go to 1. If it
   returns none and task_list shows no todo task, message_send
   <LEAD NAME> your report and stop.

Rules:
- A conflict means do not edit: claim again with wait_secs 120, or
  pull other work.
- A lost field on any reply means claim those paths again.
- Blocked: task_update blocked with the reason as note; the lead is
  told. Do not wait for an answer; pull other work.
- Propose contracts, tasks and decisions to <LEAD NAME> with
  message_send; the daemon refuses them from you.
- Never claim .tirith/lead, never message human, never commit, push,
  open a pull request or deploy, never run the full check.
- Do not sleep and do not end your turn to wait; task_pull and claim
  wait server-side.
```

## 4. Keeping agents busy

Idle time in a swarm comes from four places; each has a rule.

| Idle source | Rule | What the daemon does |
|---|---|---|
| A refused claim, then a sleep and a retry | Claim with `wait_secs: 120`; never sleep | Waits server-side, retries the moment a lease ends, wakes on release ([ADR-0031](../5-decisions/0031-waits-end-when-the-caller-goes.md)) |
| A worker finishing and waiting for the next instruction | Pull again with `wait_secs`; stop only when no `todo` task is left | `task_pull` waits only for what waiting can change and answers `none` at once on an empty board ([ADR-0028](../5-decisions/0028-claim-aware-task-pull.md)) |
| Everyone queued behind one shared file | Hold shared files only while writing them; claim a symbol anchor | Anchors in one file do not overlap each other ([ADR-0029](../5-decisions/0029-symbol-anchored-claims.md)); `task_pull` ranks free tasks first |
| A question to the human | Workers escalate to the lead; the lead answers from the records and relays only what needs the human | Routes blocked tasks and repeated refusals to the lead's inbox, never the human's, while there is a lead ([ADR-0027](../5-decisions/0027-swarm-lead-and-escalation.md)) |

Measure it with the lead log: `tirith lead log --event claim_waited`
shows every wait and how it ended, `--event claim_refused` the refusals,
and `/api/lead` the same rows for a script. A swarm whose `claim_waited`
rows sum to more than a few minutes has tasks with overlapping paths;
split them or add anchors.

## 5. Tests and checks in a swarm

| Who | Runs | Why |
|---|---|---|
| Worker | The tests for the module it edited (`cargo test --lib <module>`, `bun test <file>`, the project's equivalent) | Fast, and a failure elsewhere is another worker's compile error, not its own |
| Lead, before merging | The project's full check once, after every worker released | One run of the expensive chain, on the merged tree |
| Lead, on a failure | `task_create` with the failing test's paths; or fix it in the lead session when it is one line | The worker that caused it may already be gone |

A bench for the coordination itself is a scripted swarm: fake workers
that pull, claim with `wait_secs`, hold their paths for a scripted
duration, release and finish, against a real daemon, reporting wall
time, time spent waiting in `claim` and `task_pull`, refused claims and
escalations. The previous harness was removed with
[ADR-0035](../5-decisions/0035-shared-agent-conventions.md) and its
numbers kept in [../3-tests/01-testing-strategy.md](../3-tests/01-testing-strategy.md);
rebuilding it is on this repository's board.

## 6. GitHub, deploys and other services

Only the lead crosses the repository boundary: it commits, pushes, opens
and updates pull requests, reads review comments and CI results, and
deploys, with its client's own tools (`gh`, `wrangler`, the client's
connectors) under the human's credentials. Workers never do, and hold no
token. A review comment or a failing check the lead decides to act on
becomes a task; a worker pulls it like any other. Tirith adds no tool for
any of this: [ADR-0037](../5-decisions/0037-lead-owns-the-integration-boundary.md).

## 7. Finer claims than a file

A claim may name a symbol: `src/api.rs#Router::dispatch`,
`app/config.py#Config.from_env`. Two anchors in one file do not overlap
unless one encloses the other, so two workers edit one file at once; a
whole-file claim still overlaps every anchor in it. The worker brief
above asks for anchors on shared files. The rules and limits are in
[../1-about/04-primitives.md](../1-about/04-primitives.md#claim) and
[ADR-0029](../5-decisions/0029-symbol-anchored-claims.md).

The daemon checks an anchor's syntax, never that the edit stayed inside
the symbol: a claim is a social contract enforced by agents that ask
before editing. What would enforce it is an audit at review time, when
the lead maps each hunk of a worker's diff to the symbol that contains
it and compares that with the worker's claims. That needs a parser per
language and is not built; the lead reads diffs against claims by hand
today.
