---
id: e8c748d1-ac6d-4f35-b731-d9f3204c7218
permalink: release-container-rustc-lags-aarch64-pc-windows-msvc-builds-in-messense-cargo-xwin
title: "Release container rustc lags: aarch64-pc-windows-msvc builds in messense/cargo-xwin"
kind: gotcha
tags:
- release
- cargo-dist
- toolchain
paths:
- rust-toolchain.toml
- dist-workspace.toml
- .github/workflows/release.yml
- Cargo.toml
author: head-dev-fable
updated_by: head-dev-fable
created_at: 2026-09-16T04:40:11Z
updated_at: 2026-09-16T04:40:11Z
---

Symptom seen on the v1.0.0 release (2026-09-16): `error: rustc 1.89.0 is not supported ... tirith-mcp@1.0.0 requires rustc 1.90`, right after "Downloading MSVC CRT". That is the `aarch64-pc-windows-msvc` job. `dist plan --output-format=json | jq '.ci.github.artifacts_matrix.include[]'` shows it runs on ubuntu-22.04 inside container `messense/cargo-xwin`, whose Dockerfile is `FROM rust:1.89.0` (rustup-managed but frozen). The generated release.yml only installs Rust in a container when `cargo` is missing, so the image's rustc is used.

Fix in place: `rust-toolchain.toml` with `channel = "stable"` (ADR-0023). rustup in the container honours it and downloads stable; dist runs `rustup target add` for cross targets itself (cargo-dist src/build/cargo.rs, "If we're trying to cross-compile, ensure the rustup toolchain is set up"). The file is not dist config, so `dist generate --check` is unaffected.

Re-verify when: cargo-dist is upgraded (check whether it still uses that container and still skips Rust install), or `rust-version` in Cargo.toml rises. `dist` 0.32 prints "rust-toolchain-version is deprecated, use rust-toolchain.toml" if anyone tries the dist-workspace.toml option instead.
