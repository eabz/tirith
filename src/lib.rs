//! Tirith is an MCP coordination server that keeps parallel coding agents
//! from stepping on each other in one repository.
//!
//! The library holds everything testable: the domain modules (one per
//! primitive), the shared [`state::State`], JSON persistence in [`store`],
//! the MCP tool surface in [`server`], the HTTP dashboard, and a thin MCP
//! client used by the CLI. The binary in `main.rs` only parses arguments.
//!
//! Primitives:
//!
//! - [`claims`]: leases on repository paths, refused on overlap.
//! - [`tasks`]: a task board agents pull from.
//! - [`contracts`]: interface shapes published before implementation.
//! - [`notices`]: change notices dependents read before acting.
//! - [`decisions`]: settled choices so nothing is decided twice.
//!
//! [`stdio`] is the shim MCP clients spawn per session; it starts the
//! daemon when needed and proxies to it. [`update`] replaces the running
//! binary with a newer release.

#![forbid(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod claims;
pub mod client;
pub mod clock;
pub mod contracts;
pub mod dashboard;
pub mod decisions;
pub mod notices;
pub mod server;
pub mod state;
pub mod stdio;
pub mod store;
pub mod tasks;
pub mod types;
pub mod update;
