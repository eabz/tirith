# ADR-0037: The lead owns the integration boundary; Tirith stays out of GitHub and deploy targets

**Status:** Proposed, 2026-10-10. Refines
[ADR-0027](0027-swarm-lead-and-escalation.md) and
[ADR-0036](0036-lead-only-writes.md).

## Context

A swarm's work leaves the repository through git pushes, pull requests
and deploys (GitHub Actions, Cloudflare Workers and Pages, package
registries), and work comes back in as review comments, CI failures and
issues. The owner asked how agents should connect to GitHub and
Cloudflare "natively", and who pushes and receives requests.

Two facts shape the answer. Tirith is not an orchestrator and never
starts agents or calls services
([../1-about/01-purpose.md](../1-about/01-purpose.md)); it coordinates
agents that already have tools. And every coding client a lead runs in
already speaks to GitHub and Cloudflare: `gh`, `wrangler`, and the
clients' own GitHub and Cloudflare connectors. Building a second
integration into the daemon would duplicate them behind one more
credential and one more schema in a `tools/list` that is at its budget.

## Decision

1. **The lead is the only session that crosses the boundary.** It
   commits, pushes, opens and updates pull requests, reads review
   comments and CI results, and deploys. It does so with its client's
   own tools, under the human's credentials, and never through Tirith.
2. **Workers never cross it.** A worker edits under its claims, runs the
   module tests, publishes notices, writes memory notes, releases, and
   reports to the lead. It holds no token, never runs `git push`, `gh`
   or `wrangler`, and never opens a pull request. The worker brief says
   so ([`guide` topic `roles`](../1-about/04-primitives.md#guide)).
3. **Incoming requests become tasks.** A review comment, a failing check
   or an issue the lead decides to act on is a `task_create` with the
   paths and `depends_on` it needs; a worker pulls it like any other
   task and never sees the request itself. The lead answers the review.
4. **The lead reviews before it pushes.** It reads each worker's diff
   against the worker's claims and the contract, simplifies, runs the
   full check once, and records the settled choices with
   `decision_record`. Deploy facts a later session needs (which
   environment, which command, what to verify) are memory notes on the
   deploy config's paths.
5. **Tirith adds no tool for this.** What it adds is the rule, in the
   `roles` guide and in
   [../2-examples/03-swarm-playbook.md](../2-examples/03-swarm-playbook.md),
   and the gates of ADR-0036 that keep the plan in the lead's hands.

## Alternatives

- **GitHub and Cloudflare tools in the daemon** (open a pull request,
  deploy, list checks). Rejected: duplicates the clients' connectors,
  needs credentials in a server that has none, and spends the schema
  budget.
- **Any worker may push to a branch of its own.** Rejected: review and
  simplification happen once, at the lead, or they happen never; N
  branches also means N CI runs for one change.
- **A git `pre-push` hook that refuses unless the pusher is the lead.**
  Deferred: agents share one shell user, so the hook could only trust an
  environment variable the agent sets itself. Worth building when a
  measured swarm shows workers pushing despite the brief.

## Consequences

- One session holds the credentials and the review; a worker on a
  cheaper model cannot ship anything by itself.
- The lead's turn is longer: it merges and reviews as well as plans.
  That is where the strongest model is spent on purpose.
- This ADR stays Proposed until a swarm has run under it; the playbook
  carries the rule now, and accepting the ADR is a line change.
