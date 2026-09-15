# Git and documentation conventions

## Commits

Conventional Commits, scoped by module:

```
feat(claims): refuse overlapping directory claims
fix(store): write JSON atomically via rename
docs(examples): add CrewAI setup
test(state): cover renew-on-activity
refactor(server): move response formatting into a helper
chore: bump rmcp to 3.4
```

- Subject in imperative mood, under 72 characters, no trailing period.
- Body explains why when the diff does not. Reference an ADR when the
  change implements one: `Implements ADR-0003.`
- One logical change per commit, with its tests and docs.
- Agents commit only when asked. Never force-push. Never rewrite `main`.

## Branches

`main` is always green. Feature branches are `feat/<short-name>`,
`fix/<short-name>`, `docs/<short-name>`. Delete after merge.

## Pull requests

- Title follows the commit convention.
- Body: what, why, how tested, and any lint allows or new dependencies.
- CI must pass. Reviewers do not check formatting or lints by hand.

## Documentation

- All docs live in `docs/`, numbered sections and files
  (see [../README.md](../README.md)).
- Behavior changes and doc changes ship in the same commit.
- Mark unbuilt features **Planned** in every doc that mentions them, and
  remove the marker in the commit that builds them.
- ADRs in `docs/5-decisions/` are immutable once accepted. To change a
  decision, write a new ADR that supersedes the old one and link both ways.
- Language: plain, present tense, short sentences. Prefer a table to a
  paragraph when comparing things. Expand an acronym the first time it
  appears in a file.
