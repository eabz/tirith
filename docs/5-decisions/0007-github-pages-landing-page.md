# ADR-0007: A static landing page in `docs/`, served by GitHub Pages

**Status:** Accepted, 2026-09-16. Amended the same day: the Pages source
was set to the repository root rather than `/docs`, so a redirect stub
`/index.html` forwards the site root to `docs/`. The page itself stays in
`docs/`; nothing else in this record changes.

## Context

The README is the first thing a visitor sees on GitHub, but it has no
home outside GitHub and cannot offer platform tabs or copy buttons for the
install commands. The project needs one public page whose job is to get
`tirith` installed and connected in under a minute, and the page must
stay cheap to maintain for a solo developer plus coding agents.

## Decision

The landing page is a single hand-written file, `docs/index.html`, with
inline CSS and a few lines of vanilla JavaScript (install-method tabs and
copy buttons). GitHub Pages serves it from the `main` branch with the
`/docs` folder as the source; `docs/.nojekyll` disables Jekyll so the
file is served as-is. The page links into the Markdown docs on GitHub
rather than rendering them itself.

The page repeats only what `README.md` and
`1-about/05-installation.md` already say. When an install command or a
client setup changes, all three are updated in the same change.

## Alternatives

- **A static site generator (mdBook, Docusaurus, Zola).** Adds a build
  step, a toolchain, and a CI job to keep a single page alive. The
  Markdown docs already render well on GitHub; the generator would mostly
  duplicate that.
- **GitHub's Jekyll theme on the README.** No control over layout, no
  tabs, no copy buttons, and Jekyll processing surprises on files under
  `docs/` such as `_static/`.
- **An `index.html` at the repository root.** Violates the layout rule
  that documentation lives under `docs/` (AGENTS.md section 5).

## Consequences

- Publishing is a repository setting: Pages source `main` / `/docs`. No
  workflow, no build, no dependencies.
- Anything under `docs/` is now public at
  `https://eabz.github.io/tirith/`, including the Markdown files served
  raw. That is acceptable; they are already public on GitHub.
- The page is a third copy of the install commands. The docs index and
  the release process note that a change to install paths must touch
  `README.md`, `1-about/05-installation.md`, and `docs/index.html`.
- If the page ever needs more than one file of content, this ADR is
  superseded by one that picks a generator.
