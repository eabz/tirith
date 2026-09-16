# ADR-0023: A `rust-toolchain.toml` pins the stable channel so release containers meet `rust-version`

**Status:** Accepted, 2026-09-16. Refines ADR-0009.

## Context

The v1.0.0 release workflow failed on the `aarch64-pc-windows-msvc` job:

```
error: rustc 1.89.0 is not supported by the following packages:
  tirith-mcp@1.0.0 requires rustc 1.90
```

cargo-dist has no Windows arm runner, so it builds that target on
`ubuntu-22.04` inside the `messense/cargo-xwin` container. That image is
`FROM rust:1.89.0`, a rustup-managed toolchain frozen at the version the
image was built with. The generated workflow installs Rust only when
`cargo` is missing, so the container's rustc is used as is. Every other
job runs on a GitHub runner whose stable toolchain is current.

`rust-version` moved from 1.85 to 1.90 for v1.0.0 because the tray crates
behind the default `tray` feature need it (comment in `Cargo.toml`). The
crate cannot go back below 1.90, and the container's pin is outside our
control.

## Decision

A `rust-toolchain.toml` at the repository root with `channel = "stable"`.
rustup reads it wherever cargo runs in this checkout: in the container it
downloads current stable into the image's `RUSTUP_HOME` before the build,
and dist then runs `rustup target add` for the cross target as it already
does. On developer machines and on the other runners it selects the
channel they already use, so nothing changes there. `ci.yml` keeps
`dtolnay/rust-toolchain@stable`; the two agree.

## Alternatives

- **`rust-toolchain-version` in `dist-workspace.toml`.** Generates a
  `rustup update X && rustup default X` step in every build job. dist
  marks it deprecated and prints a warning pointing at
  `rust-toolchain.toml`.
- **`github-build-setup` steps** that update rustup only when
  `matrix.container` is set. Works, but adds hand-written workflow YAML
  to a generated file's inputs for something rustup does on its own.
- **Lower `rust-version` to 1.89.** `cargo +1.89 check` fails with the
  default features; the tray crates need 1.90.
- **Drop the `aarch64-pc-windows-msvc` target.** A distribution decision,
  not a toolchain one; the target costs nothing once it builds.
- **Pin a version (`channel = "1.90"`).** Would force every developer's
  rustup to download that exact toolchain and would need bumping by hand.
  `rust-version` already states the floor; stable is what CI tests.

## Consequences

- The container job downloads a stable toolchain on every release (a few
  hundred megabytes, under a minute); `rust-cache` does not cache
  toolchains. Acceptable for one job per release.
- If the cargo-xwin image ever ships a rustc newer than stable (it will
  not; the image lags), the file still wins, which is the intent.
- The file is not dist configuration: `dist generate --check` is
  unaffected and `release.yml` does not change.
- When the crate's `rust-version` rises, nothing here needs editing.
