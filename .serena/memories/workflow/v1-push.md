# v1 push (started 2026-09-16)

- ADR-0013 (`docs/5-decisions/0013-v1-definition.md`) defines v1: coordination layer, memory layer, and token budgets, each enforced by a byte-size or behavior test. Read it before picking work; every bullet maps to a Tirith task id.
- Head-developer session runs as Tirith agent `head-dev-fable`; it assigns work through the task board and by messaging the other Claude sessions. Agent names to session titles: `claude-ci-speedup` = "GitHub Actions build speed", `claude-memory-layer` = "Tirith as memory layer", `storage-claude` = "Tirith storage performance and scaling", `claude-token-diet` = "Token consumption reduction", `claude-docs-audit` = "Documentation review".
- Handoff order for `src/server.rs` and `src/state.rs`: memory-layer (memory glue) → ci-speedup (result dedupe, schema shrink) → storage-claude (list paging, status/claims_list) → token-diet (brief on claim, ADR-0014).
- Two audits on 2026-09-16 (coordination and memory) are summarised in the Tirith memory note `design/v1-audit-2026-09-16`. Blockers found: corrupt JSONL/note file stops the daemon booting; shim never checks daemon version or repo root; persist failure reported as ok; leases reaped silently.
- The running daemon may be older than the working tree until task 2e31aea6 (shim restarts a mismatched daemon) lands; check `tirith status` version before trusting the tool list.
