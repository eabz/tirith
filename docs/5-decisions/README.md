# Architecture decision records

Each file records one decision about Tirith itself: context, the decision,
alternatives considered, and consequences. Accepted ADRs are not edited;
they are superseded by a new ADR that links back.

Format: `NNNN-short-title.md`, with sections **Status**, **Context**,
**Decision**, **Alternatives**, **Consequences**.

| ADR | Title | Status |
|---|---|---|
| [0001](0001-rust-and-rmcp.md) | Rust with the official rmcp SDK | Accepted |
| [0002](0002-single-daemon-over-http.md) | One daemon per repo over streamable HTTP | Accepted |
| [0003](0003-json-file-storage.md) | In-memory state written through to JSON files | Accepted |
| [0004](0004-caller-supplied-agent-identity.md) | Agent identity is a caller-supplied string with TTL leases | Accepted |
| [0005](0005-prefix-paths-no-globs.md) | Claims are files or directory prefixes; no globs in v1 | Accepted |

These are decisions about building Tirith. Decisions that agents make while
using Tirith on some other project go in that project's `.tirith/decisions.jsonl`
through the `decision_record` tool.
