# Tirith documentation

All project documentation lives here, in numbered sections so the reading
order is obvious. Files inside a section are also numbered.

| Section | Contents |
|---|---|
| [1-about/](1-about/) | What Tirith is, how it is built, how the repo is organized, what each primitive does |
| [2-examples/](2-examples/) | The two-agent demo and client setup for Claude Code, Cursor, Python frameworks, and raw JSON-RPC |
| [3-tests/](3-tests/) | Testing strategy, layout, and commands |
| [4-style/](4-style/) | Rust rules agents must follow, the sources they come from, git and docs conventions |
| [5-decisions/](5-decisions/) | Architecture decision records (ADRs). Settled. Supersede, do not silently re-decide |
| [6-agent-workflow/](6-agent-workflow/) | How agents work on this repo: Serena, memory layer, using Tirith on itself |
| [7-release/](7-release/) | How releases are built and published (cargo-dist), targets, the installer shim |
| [_static/images/](_static/images/) | Diagrams and screenshots referenced from the docs |
| [index.html](index.html) | The public landing page at [eabz.github.io/tirith](https://eabz.github.io/tirith/): install, connect, verify. Served by GitHub Pages from this folder; `.nojekyll` keeps Jekyll off |

## Requirements for working on this repo

- Read [AGENTS.md](../AGENTS.md) before editing.
- A Tirith daemon must be running for this repository, and every agent
  must claim files through it before editing them. See
  [6-agent-workflow/03-tirith-dogfooding.md](6-agent-workflow/03-tirith-dogfooding.md).
- Serena is the code-intelligence layer; Basic Memory notes live in
  `.memory/`.

## Conventions for writing docs

- One H1 per file, matching the file's subject. Filenames are
  `NN-kebab-case.md` inside each numbered section.
- Present tense, short sentences. Say what the code does, not what it will
  hopefully do. Mark unbuilt features with **Planned** so readers are not
  misled.
- Code and commands go in fenced blocks with a language tag.
- Link to other docs with relative paths so links work on GitHub and locally.
- When behavior changes, update the doc in the same change. Stale docs are
  bugs.
- Agent memory is not documentation. Basic Memory notes live in
  `.memory/` at the repo root (committed), written through the
  `write_note` tool. See
  [6-agent-workflow/02-memory.md](6-agent-workflow/02-memory.md).
- Images: put files in `_static/images/`, reference them with relative
  paths, and keep them under 500 KB.
- `index.html` is the only HTML here. It is hand-written, has no build
  step, and repeats the install and client-setup commands from
  `README.md`, `1-about/05-installation.md`, and
  `2-examples/02-client-setup.md`. When those commands change, change
  the page in the same commit. Design rationale:
  [5-decisions/0007-github-pages-landing-page.md](5-decisions/0007-github-pages-landing-page.md).
