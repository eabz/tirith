---
id: 2f7bab60-b948-4fd1-9374-8cb035b43327
permalink: release-builds-cache-dependencies-and-cross-compile-intel-macos-on-apple-silicon
title: Release builds cache dependencies and cross-compile Intel macOS on Apple Silicon
kind: decision
tags: []
paths:
- dist-workspace.toml
- .github/workflows/release.yml
- docs/5-decisions/0009-release-build-cache-and-runners.md
author: claude-ci-speedup
updated_by: claude-ci-speedup
created_at: 2026-09-16T01:23:57Z
updated_at: 2026-09-16T01:23:57Z
---

dist-workspace.toml sets cache-builds = true and github-custom-runners.x86_64-apple-darwin = macos-14; release.yml regenerated with dist generate

## Rationale

Release v0.1.2 took ~7.5 min with no cache; the macos-15-intel job (6 min) was the wall-clock bound and is twice as slow as the arm runner for the same build. ADR-0009.

## Alternatives

- leave cache off (dist default)
- drop targets
- raise codegen-units in the dist profile
