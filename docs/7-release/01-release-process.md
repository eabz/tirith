# Release process

Releases are built by [cargo-dist](https://github.com/axodotdev/cargo-dist),
the same tool that produces uv's installer. It generates
`.github/workflows/release.yml`, which on every `v*` tag builds the binary
for every target in `dist-workspace.toml`, packages archives with
checksums, generates the shell and PowerShell installers, and publishes
everything as a GitHub release.

## The workflow trigger

`release.yml` runs the full build and publish on any pushed tag that looks
like a semantic version (`v0.1.0`, `v1.2.3-rc.1`). On pull requests it only
runs the `plan` job, which validates the configuration without publishing.

## Cutting a release

1. Bump `version` in `Cargo.toml`, update the roadmap in
   `docs/1-about/01-purpose.md` if a milestone closed, and commit.
2. Tag and push:

   ```bash
   git tag v0.2.0
   git push origin main v0.2.0
   ```

3. Watch the `Release` workflow. When it finishes, the release page has:
   one archive per target, a `sha256` per archive,
   `tirith-mcp-installer.sh`, `tirith-mcp-installer.ps1`, and
   `dist-manifest.json`. The `publish-crates` job then runs
   `cargo publish` for `tirith-mcp`.
4. Check the install one-liner from a clean shell, and
   `cargo install tirith-mcp` if you want to confirm crates.io.

That is the whole process. There is no manual upload step. Steps 1 and 2
are what `scripts/bump.sh` does for you:

```bash
scripts/bump.sh patch            # 0.1.1 -> 0.1.2, commit, tag
scripts/bump.sh minor --push     # bump, commit, tag, push: starts the release
scripts/bump.sh 1.0.0-rc.1       # exact version
```

It refuses to run on a dirty tree or an existing tag, refreshes
`Cargo.lock`, runs `cargo check`, commits `chore(release): vX.Y.Z`, and
creates an annotated tag. Without `--push` it prints the push command so
you can run the quality gate first.

## crates.io

The package is `tirith-mcp` (the name `tirith` is taken); the binary and
library are `tirith`. Publishing is a custom cargo-dist publish job in
`.github/workflows/publish-crates.yml`, listed under `publish-jobs` in
`dist-workspace.toml`. It runs after the GitHub release exists and needs
a repository secret named `CARGO_REGISTRY_TOKEN` holding a crates.io API
token with publish scope for `tirith-mcp`. A published version can never be
replaced, so the job only runs when the whole build succeeded.

To publish by hand instead: `cargo publish` from the tagged commit.

## Files it owns

- `dist-workspace.toml`: the configuration (targets, installers, hosting,
  publish jobs).
- `.github/workflows/publish-crates.yml`: the crates.io publish job,
  hand-written, called by `release.yml`.
- `scripts/bump.sh`: version bump, commit, and tag.
- `.github/workflows/release.yml`: generated. Regenerate, never edit.
- `[profile.dist]` in `Cargo.toml`: the release profile the builds use
  (inherits `release`, plus thin LTO, one codegen unit, and stripped
  symbols).

## Targets

Configured in `dist-workspace.toml` under `targets`:

```
aarch64-apple-darwin      x86_64-apple-darwin
aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu
aarch64-unknown-linux-musl x86_64-unknown-linux-musl
aarch64-pc-windows-msvc   x86_64-pc-windows-msvc
```

macOS targets build on macOS runners, Windows on Windows runners, Linux
targets on Linux runners with cross toolchains where needed. Adding a
target is one line in that file plus `dist generate` to refresh the
workflow.

## Local checks

```bash
dist plan                 # every artifact a release would produce
dist build                # build archives for this machine's target into target/distrib/
dist generate --check     # confirm release.yml matches dist-workspace.toml
```

Run `dist generate --check` after changing `dist-workspace.toml`; the
release workflow's `plan` job also runs on pull requests and fails if the
workflow is stale.

To test the generated installer without a GitHub release, serve
`target/distrib/` over HTTP and point the installer at it:

```bash
(cd target/distrib && python3 -m http.server 8123 --bind 127.0.0.1) &
TIRITH_MCP_DOWNLOAD_URL=http://127.0.0.1:8123 TIRITH_MCP_UNMANAGED_INSTALL=/tmp/tirith-test/bin \
  sh target/distrib/tirith-mcp-installer.sh
/tmp/tirith-test/bin/tirith --version
```

`TIRITH_MCP_UNMANAGED_INSTALL` installs into the given directory and
leaves `PATH` and shell profiles alone. The variables are prefixed with the
crate name, not the binary name.

## Changing the dist configuration

- Edit `dist-workspace.toml`, never `release.yml` by hand.
- Run `dist generate` and commit both.
- Upgrading cargo-dist itself: `cargo install cargo-dist`, then
  `dist init` (keeps existing answers) and commit the regenerated
  workflow.

## Installer shim

`install.sh` at the repo root is a stable URL that redirects to the
release installer, so documentation never has to change when asset names
or hosting change. It takes `TIRITH_VERSION` to pin a release. Keep it
POSIX `sh` and dependency-free.
