# Tirith

Coordination for parallel coding agents in one repository: leases on paths, a task board, interface contracts, change notices, decisions, path-scoped memory notes, agent messages. Nothing is pushed or polled; every call returns what you need.

## Every call
- One stable `agent` name per session (`claude-<task>`); `human` is reserved.
- Results are JSON with `status`: `ok`, `conflict`, `not_found`, `none`, `invalid`. Read `message` unless `ok`.
- Any result may add `lost` (your leases ended: claim again before editing), `inbox` (messages, delivered once), `persist_error`.
- Lists: 20 rows, newest first; page with `before` = `next_before`.
- Every call renews your leases; after four TTLs only `claim` or `renew` does.

## Loop
1. `memory_search` (no query), `status`, then `task_pull`: the board is the committed plan; work it before new work. Finish what you pull or hand it back (`task_update` `todo` with a note); never stop with a task `in_progress`. A task you create is a commitment: assign it or do it, and name what is still `todo` in your report.
2. `claim` paths before editing, with a readable reason. The `ok` brief lists unread notices, contracts, decisions and notes for those paths: read it. `conflict`: nothing claimed, do not edit; retry with `wait_secs` (max 120) or take other work. Hold shared files only while writing; claim `path#Symbol` on them and edit by replacement.
3. `contract_publish` an interface before either side builds it. `notice_publish` renames, signature changes, removals, moves that reach unclaimed files.
4. `task_update` done. `memory_write` what the next agent needs about these paths. `decision_record` what is settled.
5. `release`. Never sleep or end the turn to wait: `claim` and `task_pull` wait server-side.

## Roles
Spawning agents makes you lead; being spawned makes you worker; alone, skip this section.

Lead: `claim` `.tirith/lead` (`ttl_secs` 3600) before spawning, re-claim on `lost`, release last. Split work into tasks with disjoint `paths`; `contract_publish` shared interfaces first; `task_create` with `paths`, `priority`, `depends_on`. Spawn one worker per `todo` task with disjoint paths, never more workers than free tasks, none if every task shares files. Brief each worker: its name, your name, task id, contract, paths, this loop. Your inbox arrives on every call: answer it; relay to `human` only what only the human can settle. Review each diff against claims and contract, run the full check once; only you commit, push, open PRs, deploy. Strongest model for you; smaller, cheaper model for workers.

Worker: loop `task_pull` with `wait_secs` 120 until `none` and no `todo` left, then release, `message_send` the lead, stop. Test only your paths. Report done, blocked, proposals to the lead; never `human`, never `.tirith/lead`, never commit or push. While a lead exists, `contract_publish`, `task_create`, `decision_record`, `memory_delete` and `task_update` `force` are the lead's: you get `conflict`; propose instead.
