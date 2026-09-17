# ADR-0025: Release builds drop the dependency cache

**Status:** Accepted, 2026-09-16. Supersedes the build cache half of
[ADR-0009](0009-release-build-cache-and-runners.md).

## Context

ADR-0009 set `cache-builds = true` in `dist-workspace.toml`, which adds a
`swatinem/rust-cache` step keyed by target to every job of the release
build matrix. In practice the cache did not hit on that matrix: release
builds kept compiling every crate.

The likely cause is how GitHub scopes caches. A workflow run can restore
caches created on its own ref or on the default branch. The release
workflow runs on a version tag, and every release is a new tag, so a
cache one release saves is not visible to the next one. The cache step
therefore paid its upload time and quota on every release and never
returned anything. The `messense/cargo-xwin` container, which installs
current stable at build time (ADR-0023), would also change the rustc part
of the key for that target whenever stable moves.

## Decision

Remove `cache-builds` from `dist-workspace.toml` and regenerate
`.github/workflows/release.yml` with `dist generate` (cargo-dist 0.32.0).
The only change to the workflow is the removed `swatinem/rust-cache` step.

`ci.yml` keeps its caches: it runs on `main` and on pull requests against
it, where restores work.

The custom runner from ADR-0009 (`x86_64-apple-darwin` on `macos-14`)
stays; it shortens the slowest job without a cache.

## Alternatives

- **Warm the cache from `main`.** A job on the default branch could build
  every target with the `dist` profile so tags restore it. Rejected: it
  doubles build minutes on every push to `main` to save them on the rare
  release.
- **Keep the step.** Rejected: it never hit, and it costs time and eight
  cache entries per release against the 10 GB quota.

## Consequences

- Every release build is a cold build. Expect the times ADR-0009 measured
  before the cache (about 7.5 minutes wall clock, bound by the slowest
  target), less the Intel macOS runner saving it kept.
- If release build time becomes a problem, the next lever is the matrix or
  the `dist` profile, not a cache (see the alternatives in ADR-0009).
