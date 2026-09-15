# ADR-0005: Claims are files or directory prefixes; no globs in v1

**Status:** Accepted, 2026-09-15

## Context

Agents naturally want to say "I own `src/auth/**`". Refusal must be
decidable and predictable: an agent that is refused needs to understand
why in one line.

## Decision

A claimed path is a repo-relative file path or a directory path ending in
`/`. Two paths overlap when they are equal or one is a prefix of the other
at a path-segment boundary. Globs are not accepted in v1.

## Alternatives

- **Full glob support.** Deciding whether two globs can match a common
  path is expensive and surprising in edge cases. Deferred.
- **Exact file paths only.** Too chatty; a refactor of a module would need
  dozens of paths.

## Consequences

- `src/auth/` conflicts with `src/auth/login.rs` and with `src/`, but not
  with `src/authz/`.
- If globs are added later, the rule will be conservative: any shared
  literal prefix before the first wildcard is treated as overlap, and this
  ADR will be superseded.
