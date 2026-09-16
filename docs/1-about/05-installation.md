# Installation

Tirith ships as a single static binary per platform, built and published by
CI on every tagged release. Pick whichever of these fits.

## One line (macOS, Linux)

```bash
curl -LsSf https://raw.githubusercontent.com/eabz/tirith/main/install.sh | sh
```

That shim fetches the installer attached to the latest GitHub release. It
detects your OS and CPU, downloads the matching archive, verifies its
checksum, installs `tirith` into `$CARGO_HOME/bin` (`~/.cargo/bin` by
default, created if missing), and tells you if your `PATH` needs a line. Pin a version
with `TIRITH_VERSION=v0.1.0`.

If you would rather not go through the shim, the release asset itself is
the same thing:

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/eabz/tirith/releases/latest/download/tirith-installer.sh | sh
```

## One line (Windows)

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/eabz/tirith/releases/latest/download/tirith-installer.ps1 | iex"
```

## Prebuilt archives

Every release on
[github.com/eabz/tirith/releases](https://github.com/eabz/tirith/releases)
carries an archive per target plus a `sha256` file:

| Platform | Target |
|---|---|
| macOS Apple Silicon | `aarch64-apple-darwin` |
| macOS Intel | `x86_64-apple-darwin` |
| Linux x86_64 (glibc) | `x86_64-unknown-linux-gnu` |
| Linux ARM64 (glibc) | `aarch64-unknown-linux-gnu` |
| Linux x86_64 (static, musl) | `x86_64-unknown-linux-musl` |
| Linux ARM64 (static, musl) | `aarch64-unknown-linux-musl` |
| Windows x86_64 | `x86_64-pc-windows-msvc` |
| Windows ARM64 | `aarch64-pc-windows-msvc` |

Unpack and put `tirith` somewhere on your `PATH`.

## cargo-binstall

```bash
cargo binstall tirith --git https://github.com/eabz/tirith
```

`cargo-binstall` understands the release layout and downloads the prebuilt
binary instead of compiling.

## From source

```bash
git clone https://github.com/eabz/tirith
cd tirith
cargo install --path .
```

Needs a stable Rust toolchain (1.85 or newer). There are no C
dependencies: Tirith speaks plain HTTP on localhost, so no TLS library is
compiled in.

## Note on crates.io

The crate name `tirith` on crates.io belongs to an unrelated project, so
`cargo install tirith` installs something else. Tirith is distributed
through GitHub releases and the methods above, not crates.io, until it is
published under a different package name.

## After installing

```bash
cd /path/to/your/repo
tirith serve
```

Then point your agents at `http://127.0.0.1:7477/mcp`; see
[../2-examples/02-client-setup.md](../2-examples/02-client-setup.md).
