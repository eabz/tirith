---
id: b0fc86cb-bf58-4beb-89fb-752d3add7d11
permalink: dashboard-assets-are-embedded-from-src
title: Dashboard assets are embedded from src/
kind: decision
tags: []
paths:
- src/dashboard.rs
- src/dashboard.html
- src/dashboard-logo.png
author: claude-dashboard-ui
updated_by: claude-dashboard-ui
created_at: 2026-09-16T01:11:58Z
updated_at: 2026-09-16T01:11:58Z
---

The dashboard serves only assets embedded at compile time from files next to src/dashboard.rs. The logo lives at src/dashboard-logo.png (192px PNG cut from docs/_static/images/logo.jpeg) and is served at /logo.png; the page loads no external fonts, scripts, or styles.

## Rationale

The crates.io package only includes /src/**, so include_bytes! from docs/ would break the published build; the daemon must run with nothing on disk and work offline. See docs/5-decisions/0008-embedded-dashboard-assets.md.

## Alternatives

- include_bytes! from docs/_static/images with a wider Cargo include list
- serve files from disk at runtime
- external CDN fonts or icons
