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
| [6-agent-workflow/](6-agent-workflow/) | How agents work on this repo: Serena, memory layer, using Tirith on itself, the definition of done |
| [7-release/](7-release/) | How releases are built and published (cargo-dist), targets, the installer shim |
| [_static/images/](_static/images/) | Diagrams and screenshots referenced from the docs |
| [../index.html](../index.html) | The public landing page at [eabz.github.io/tirith](https://eabz.github.io/tirith/): install, connect, verify. It lives at the repository root, which GitHub Pages serves; `docs/index.html` is only a redirect to it, and the root `.nojekyll` keeps Jekyll off |

Read [AGENTS.md](../AGENTS.md) before editing anything: it is the contract
for every agent working here, including the rule that files are claimed
through Tirith before they are edited.

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
- Agent memory is not documentation. Tirith memory notes live in
  `.tirith/memory/` (committed), written through the `memory_write`
  tool; Serena memories are entry points that link back here. No copies
  per editor, no parallel memories, no report per task, no second docs
  folder next to the code. See
  [6-agent-workflow/02-memory.md](6-agent-workflow/02-memory.md).
- Tool names and schemas are documented once, in
  [1-about/04-primitives.md](1-about/04-primitives.md). Other docs name
  tools and link there; they do not restate fields.
- Images: put files in `_static/images/`, reference them with relative
  paths, and keep them under 500 KB.
- The landing page is `index.html` at the repository root; `docs/index.html`
  only redirects there. The page is hand-written, has no build step, and
  repeats the install and client-setup commands from `README.md`,
  `1-about/05-installation.md`, and `2-examples/02-client-setup.md`. When
  those commands change, change the page in the same commit. Design
  rationale:
  [5-decisions/0007-github-pages-landing-page.md](5-decisions/0007-github-pages-landing-page.md).

## Skills

Skills in [`.agents/skills/`](../.agents/skills/README.md) (`.claude/skills`
links there) activate when a task matches their description: `less-code`,
`dead-code`, and `tirith`. Where a shared skill names `bun run check` or
knip, this repository runs `scripts/check.sh`
([ADR-0035](5-decisions/0035-shared-agent-conventions.md)).
