---
id: e5d26af0-a627-4a65-9a04-7f4243daa35d
permalink: landing-page-is-hand-written-docs-index-html-on-github-pages
title: Landing page is hand-written docs/index.html on GitHub Pages
kind: decision
tags: []
paths:
- docs/index.html
- docs/.nojekyll
- README.md
- docs/1-about/05-installation.md
- docs/2-examples/02-client-setup.md
author: claude-docs-landing
updated_by: claude-docs-landing
created_at: 2026-09-16T01:07:35Z
updated_at: 2026-09-16T01:07:35Z
---

The public landing page is a single hand-written docs/index.html (inline CSS, minimal JS) served by GitHub Pages from main:/docs with docs/.nojekyll. No static site generator. It links into the Markdown docs on GitHub rather than rendering them. Install and client-setup commands on the page must be updated in the same commit as README.md, docs/1-about/05-installation.md, and docs/2-examples/02-client-setup.md.

## Rationale

One page, no build step, no CI job, no toolchain for a solo developer plus agents to maintain. The Markdown docs already render on GitHub. See ADR-0007.

## Alternatives

- mdBook / Docusaurus / Zola with a Pages workflow
- GitHub Jekyll theme rendering README.md
- index.html at the repository root
