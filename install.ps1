# Tirith installer shim for Windows.
#
#   irm https://eabz.github.io/tirith/install.ps1 | iex
#
# From cmd.exe:  powershell -ExecutionPolicy Bypass -c "irm https://eabz.github.io/tirith/install.ps1 | iex"
#
# Like install.sh, this only fetches the real installer that cargo-dist
# attaches to every GitHub release, so the URL above stays stable across
# releases even though the release assets are named after the crate
# (tirith-mcp). Set $env:TIRITH_VERSION = "v0.2.0" to pin a release.
# Everything else (install dir, PATH handling, checksums) is handled by the
# downloaded installer; see
# https://github.com/eabz/tirith/blob/main/docs/1-about/05-installation.md
$ErrorActionPreference = "Stop"

$repo = "eabz/tirith"
$version = if ($env:TIRITH_VERSION) { $env:TIRITH_VERSION } else { "latest" }
$url = if ($version -eq "latest") {
    "https://github.com/$repo/releases/latest/download/tirith-mcp-installer.ps1"
} else {
    "https://github.com/$repo/releases/download/$version/tirith-mcp-installer.ps1"
}

# Windows PowerShell 5.1 defaults to TLS 1.0, which GitHub refuses.
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

Invoke-Expression (Invoke-RestMethod -Uri $url)
