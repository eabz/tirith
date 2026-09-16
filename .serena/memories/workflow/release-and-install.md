# Releases and installation

- Built by cargo-dist 0.32: `dist-workspace.toml` (targets, installers shell+powershell, hosting github), generated `.github/workflows/release.yml` (never hand-edit; run `dist generate`). Triggered by semver tags (`v0.1.0`). `[profile.dist]` in Cargo.toml: thin LTO, strip, codegen-units 1.
- Eight targets: aarch64/x86_64 for apple-darwin, unknown-linux-gnu, unknown-linux-musl, pc-windows-msvc. No TLS crates in the tree (localhost HTTP only), so cross builds are pure Rust.
- Install one-liner: `curl -LsSf https://raw.githubusercontent.com/eabz/tirith/main/install.sh | sh` (shim over `releases/latest/download/tirith-installer.sh`, honors `TIRITH_VERSION`). Windows: `irm .../tirith-installer.ps1 | iex`.
- Offline installer test: serve `target/distrib` with `python3 -m http.server`, set `TIRITH_DOWNLOAD_URL` and `TIRITH_UNMANAGED_INSTALL`. Verified working 2026-09-15.
- crates.io name `tirith` is taken by an unrelated project; do not `cargo publish` under that name.
- Docs: `docs/1-about/05-installation.md` (users), `docs/7-release/01-release-process.md` (maintainers).
