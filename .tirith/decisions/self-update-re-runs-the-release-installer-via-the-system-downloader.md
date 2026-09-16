---
id: 1d39d9a5-6c5d-433e-b578-50e4a712ae49
permalink: self-update-re-runs-the-release-installer-via-the-system-downloader
title: Self-update re-runs the release installer via the system downloader
kind: decision
tags: []
paths:
- src/update.rs
author: claude-self-update
updated_by: claude-self-update
created_at: 2026-09-16T01:10:44Z
updated_at: 2026-09-16T01:10:44Z
---

tirith update fetches the latest tag from the GitHub API and re-runs the cargo-dist installer for that tag with curl (sh) or PowerShell, pinned to the running binary's directory via TIRITH_MCP_UNMANAGED_INSTALL / CARGO_DIST_FORCE_INSTALL_DIR

## Rationale

Keeps TLS out of the binary and reuses the exact install path users already trust; axoupdater would add a large dependency tree

## Alternatives

- axoupdater library
- cargo-dist install-updater (separate tirith-update binary)
