# Releases and installation

- Built by cargo-dist 0.32: `dist-workspace.toml` (targets, installers
  shell+powershell, hosting github, `publish-jobs` for crates.io),
  generated `.github/workflows/release.yml` (never hand-edit; run
  `dist generate`). Triggered by semver tags (`v0.1.0`); `scripts/bump.sh`
  bumps, commits and tags. `[profile.dist]` in Cargo.toml: thin LTO,
  strip, codegen-units 1. Both macOS targets build on `macos-14`
  (ADR-0009).
- Eight targets: aarch64/x86_64 for apple-darwin, unknown-linux-gnu,
  unknown-linux-musl, pc-windows-msvc. No TLS crates in the tree
  (localhost HTTP only), so cross builds are pure Rust.
- Install one-liner: `curl -LsSf https://eabz.github.io/tirith/install.sh | sh`
  (`install.sh` at the repo root, served by GitHub Pages, is a shim over
  `releases/latest/download/tirith-mcp-installer.sh`; honors
  `TIRITH_VERSION`). Windows: `irm https://eabz.github.io/tirith/install.ps1 | iex`.
  In place upgrade: `tirith update` (`--check` only reports).
- Offline installer test: serve `target/distrib` with
  `python3 -m http.server`, set `TIRITH_MCP_DOWNLOAD_URL` and
  `TIRITH_MCP_UNMANAGED_INSTALL` (prefixed with the crate name, not the
  binary name).
- crates.io: the package is `tirith-mcp` (the name `tirith` belongs to an
  unrelated project); the binary and library are `tirith`. Published by
  the `publish-crates` job after the GitHub release, needs the
  `CARGO_REGISTRY_TOKEN` secret.
- Docs: `docs/1-about/05-installation.md` (users),
  `docs/7-release/01-release-process.md` (maintainers).
