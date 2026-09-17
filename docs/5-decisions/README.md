# Architecture decision records

Each file records one decision about Tirith itself: context, the decision,
alternatives considered, and consequences. Accepted ADRs are not edited in
substance; they are superseded or refined by a new ADR that links back,
and the old ADR's **Status** line names the newer one.

Format: `NNNN-short-title.md`, with sections **Status**, **Context**,
**Decision**, **Alternatives**, **Consequences**.

| ADR | Title | Status |
|---|---|---|
| [0001](0001-rust-and-rmcp.md) | Rust with the official rmcp SDK | Accepted |
| [0002](0002-single-daemon-over-http.md) | One daemon per repo over streamable HTTP | Accepted; refined by 0006, 0016 |
| [0003](0003-json-file-storage.md) | In-memory state written through to JSON files | Accepted; refined by 0010, 0021 |
| [0004](0004-caller-supplied-agent-identity.md) | Agent identity is a caller-supplied string with TTL leases | Accepted; refined by 0015 |
| [0005](0005-prefix-paths-no-globs.md) | Claims are files or directory prefixes; no globs in v1 | Accepted |
| [0006](0006-stdio-shim-starts-daemon.md) | The stdio shim starts the daemon on demand | Accepted; superseded in part by 0016 |
| [0007](0007-github-pages-landing-page.md) | A static landing page in `docs/`, served by GitHub Pages | Accepted; amended (page moved to the repo root) |
| [0008](0008-embedded-dashboard-assets.md) | Dashboard assets are embedded in the binary from `src/` | Accepted |
| [0009](0009-release-build-cache-and-runners.md) | Release builds cache dependencies and cross-compile Intel macOS on Apple Silicon | Accepted; cache superseded by 0025 |
| [0010](0010-incremental-persistence.md) | Incremental persistence with a coalescing background writer | Accepted |
| [0011](0011-memory-primitive.md) | Memory notes are Markdown files scoped to repository paths | Accepted; refined by 0012, 0013 |
| [0012](0012-retire-basic-memory.md) | Basic Memory is retired; Tirith memory notes are the only long-form memory | Accepted |
| [0013](0013-v1-definition.md) | What v1 means: coordination, memory, and token budgets with tests | Accepted |
| [0014](0014-brief-on-claim.md) | `claim` returns a brief of notices, contracts, decisions, and memory for the claimed paths | Accepted; refined by 0021 |
| [0015](0015-lease-loss-and-max-age.md) | Lost leases are reported in the next response; activity alone cannot hold a lease past a maximum age | Accepted |
| [0016](0016-shim-replaces-stale-daemon.md) | The stdio shim replaces a daemon of another version and refuses one of another repository | Accepted |
| [0017](0017-tool-result-and-schema-budget.md) | Tool results travel once, and tool schemas have a byte budget | Accepted; refined by 0013 |
| [0018](0018-task-ownership-and-contract-republish.md) | Tasks belong to their owner until they go silent; contract republishes are guarded | Accepted |
| [0019](0019-menu-bar-tray.md) | A per-user daemon registry and a macOS menu bar tray | Accepted |
| [0020](0020-agent-messages.md) | Agent-to-agent messages, delivered on the next call | Accepted |
| [0021](0021-notice-acks-log.md) | Notices are seen when delivered; the seen log is runtime-only | Accepted |
| [0022](0022-decisions-as-markdown.md) | Decisions are one Markdown file each, in the memory-note format | Accepted |
| [0023](0023-stable-toolchain-for-release-containers.md) | A `rust-toolchain.toml` pins the stable channel so release containers meet `rust-version` | Accepted |
| [0025](0025-no-release-build-cache.md) | Release builds drop the dependency cache | Accepted |
| [0027](0027-swarm-lead-and-escalation.md) | Every swarm has a lead; escalations, the human queue, and the lead log are deterministic | Accepted |
| [0028](0028-claim-aware-task-pull.md) | `task_pull` skips tasks whose paths another agent holds | Accepted |
| [0029](0029-symbol-anchored-claims.md) | Symbol-anchored claims and edit-window holds | Proposed |

These are decisions about building Tirith. Decisions that agents make while
using Tirith on some other project go in that project's `.tirith/decisions/`
through the `decision_record` tool.
