# Installation

Tirith ships as a single static binary per platform, built and published by
CI on every tagged release, and as the crate `tirith-mcp` on crates.io. The
binary is always called `tirith`. Pick whichever of these fits.

## One line (macOS, Linux)

```bash
curl -LsSf https://eabz.github.io/tirith/install.sh | sh
```

That URL is `install.sh` at the repository root, served by GitHub Pages.
The shim fetches the installer attached to the latest GitHub release. It
detects your OS and CPU, downloads the matching archive, verifies its
checksum, installs `tirith` into `$CARGO_HOME/bin` (`~/.cargo/bin` by
default, created if missing), and tells you if your `PATH` needs a line. Pin a version
with `TIRITH_VERSION=v0.1.0`.

If you would rather not go through the shim, the release asset itself is
the same thing:

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/eabz/tirith/releases/latest/download/tirith-mcp-installer.sh | sh
```

## One line (Windows)

In PowerShell:

```powershell
irm https://eabz.github.io/tirith/install.ps1 | iex
```

From `cmd.exe`, or if your execution policy blocks it:

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://eabz.github.io/tirith/install.ps1 | iex"
```

`install.ps1` is the Windows twin of `install.sh`: it fetches the
PowerShell installer attached to the latest release and runs it. It
honours `$env:TIRITH_VERSION` to pin a release. The release asset itself
works too:

```powershell
irm https://github.com/eabz/tirith/releases/latest/download/tirith-mcp-installer.ps1 | iex
```

## Prebuilt archives

Every release on
[github.com/eabz/tirith/releases](https://github.com/eabz/tirith/releases)
carries an archive per target, named `tirith-mcp-<target>.tar.xz` (or
`.zip` on Windows), plus a `sha256` file:

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

## cargo install

```bash
cargo install tirith-mcp
```

Compiles from crates.io and installs the `tirith` binary. Works on any
platform with a Rust toolchain, including ones without prebuilt archives.

## cargo-binstall

```bash
cargo binstall tirith-mcp
```

`cargo-binstall` reads the crate metadata, finds the matching prebuilt
archive on the GitHub release, and installs it without compiling.

## From source

```bash
git clone https://github.com/eabz/tirith
cd tirith
cargo install --path .
```

Needs a stable Rust toolchain (1.85 or newer). There are no C
dependencies: Tirith speaks plain HTTP on localhost, so no TLS library is
compiled in.

## Updating

```bash
tirith update            # latest release, in place
tirith update --check    # only report; exits 1 if an update is available
tirith update --to 0.2.0 # a specific version
```

`tirith update` re-runs the release installer (the shell one on macOS and
Linux, PowerShell on Windows) into the directory the running binary lives
in, so it works for the one-liner and archive installs. Installs made with
`cargo install` are updated with `cargo install tirith-mcp` instead.

## Note on the crate name

The crate name `tirith` on crates.io belongs to an unrelated project, so
`cargo install tirith` installs something else. Tirith's package is
`tirith-mcp`; only the binary is named `tirith`.

## Menu bar (macOS)

`tirith tray` puts a tower icon in the menu bar that lists every Tirith
daemon running on this machine, one row per repository with its agent and
claim counts, refreshed every five seconds. Clicking a row opens that
daemon's dashboard; `Stop <repo>` shuts the daemon down cleanly; `Quit
tray` removes the icon. With exactly one daemon a left-click on the icon
opens its dashboard directly. Daemons find each other through a per-user
registry at `~/Library/Application Support/tirith/daemons.json`
(`$XDG_STATE_HOME/tirith/daemons.json` on other systems), which every
daemon writes on start; a crashed daemon drops off the menu within one
refresh. `tirith serve` (and so the stdio shim, which runs it) starts the tray the
first time a daemon comes up, unless it is already running or `--no-tray`
was given; it stays until you choose Quit. The tray is built in by default
on macOS (cargo feature `tray`) and compiles to nothing elsewhere; there
is no tray on Windows or Linux.
Design: [ADR-0019](../5-decisions/0019-menu-bar-tray.md).

## After installing

Register `tirith stdio` with your client, for example:

```bash
claude mcp add tirith -- tirith stdio
```

The shim starts the repository's daemon on the first session and proxies
to it afterwards. Clients that connect over HTTP instead need the daemon
started by hand (`tirith serve` in the repository) and pointed at
`http://127.0.0.1:7477/mcp`. Both setups, per client, are in
[../2-examples/02-client-setup.md](../2-examples/02-client-setup.md).
