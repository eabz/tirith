# Sources for the Rust rules

The rules in [01-rust-rules.md](01-rust-rules.md) are not invented here.
When a rule is unclear, or when a situation is not covered, consult these
in the order given and follow the most specific guidance. Agents should
fetch and read the relevant page rather than rely on memory.

## Primary

| Source | Use it for | URL |
|---|---|---|
| Rust API Guidelines | Naming, common traits, documentation, flexibility, type safety, future-proofing. The checklist is the canonical form | https://rust-lang.github.io/api-guidelines/checklist.html |
| Effective Rust (David Drysdale) | The reasoning behind type-driven design, error handling, borrow discipline, and dependency hygiene | https://effective-rust.com/ |
| Clippy lint list | What each lint means and the idiomatic fix. Read the entry before adding an `allow` | https://rust-lang.github.io/rust-clippy/master/ |
| The Rust Programming Language (the Book) | Fundamentals when a concept is unfamiliar | https://doc.rust-lang.org/book/ |
| Rust Design Patterns | Idioms and patterns (newtype, builder, RAII guards) and anti-patterns to avoid | https://rust-unofficial.github.io/patterns/ |

## Runtime and protocol

| Source | Use it for | URL |
|---|---|---|
| Tokio tutorial | Shared state, channels, select, spawning, blocking work | https://tokio.rs/tokio/tutorial |
| Tokio docs: "Which kind of mutex" | Why `std::sync::Mutex` is usually right and when `tokio::sync::Mutex` is needed | https://docs.rs/tokio/latest/tokio/sync/struct.Mutex.html#which-kind-of-mutex-should-you-use |
| rmcp (Rust MCP SDK) docs and examples | Tool routers, schemas, transports | https://docs.rs/rmcp and https://github.com/modelcontextprotocol/rust-sdk |
| MCP specification | Transport semantics (streamable HTTP, sessions), tool result shapes, resources | https://modelcontextprotocol.io/specification/latest |

## Reference

| Source | Use it for | URL |
|---|---|---|
| Rust standard library docs | Exact semantics of std APIs | https://doc.rust-lang.org/std/ |
| The Cargo Book | `[lints]`, features, workspaces, profiles | https://doc.rust-lang.org/cargo/ |
| The rustdoc Book | Doc comment syntax, doctests, intra-doc links | https://doc.rust-lang.org/rustdoc/ |
| Rust Edition Guide (2024) | What changed in edition 2024 | https://doc.rust-lang.org/edition-guide/rust-2024/ |
| cargo-machete, cargo-deny, cargo-llvm-cov | Unused dependencies, supply-chain checks, coverage | https://github.com/bnjbvr/cargo-machete, https://embarkstudios.github.io/cargo-deny/, https://github.com/taiki-e/cargo-llvm-cov |
| rustc lint list | Meaning of `dead_code`, `unreachable_pub`, and the other rustc lints | https://doc.rust-lang.org/rustc/lints/listing/index.html |
| thiserror and anyhow docs | Error type derivation, context | https://docs.rs/thiserror and https://docs.rs/anyhow |
| serde docs | Attributes, custom (de)serialization | https://serde.rs/ |

## How to use these as an agent

1. If a rule in `01-rust-rules.md` covers the case, follow it.
2. Otherwise find the closest API Guidelines checklist item and apply it.
3. If Clippy flags something you think is wrong, read the lint page first.
   The lint is usually right.
4. If you had to make a call the rules do not cover, write it down: add a
   rule to `01-rust-rules.md` with its source, and say so in your report.
