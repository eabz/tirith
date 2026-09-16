#!/bin/sh
# Tirith installer shim.
#
#   curl -LsSf https://eabz.github.io/tirith/install.sh | sh
#
# GitHub Pages serves this repository's root, which is what makes the URL
# short. This only fetches the real installer that cargo-dist attaches to every
# GitHub release, so the URL above stays stable across releases even though
# the release assets are named after the crate (tirith-mcp). Set
# TIRITH_VERSION=v0.2.0 to pin a release. Everything else (install dir,
# PATH handling, checksums) is handled by the downloaded installer; see
# https://github.com/eabz/tirith/blob/main/docs/1-about/05-installation.md
set -eu

repo="eabz/tirith"
version="${TIRITH_VERSION:-latest}"
if [ "$version" = "latest" ]; then
  url="https://github.com/${repo}/releases/latest/download/tirith-mcp-installer.sh"
else
  url="https://github.com/${repo}/releases/download/${version}/tirith-mcp-installer.sh"
fi

if command -v curl >/dev/null 2>&1; then
  curl --proto '=https' --tlsv1.2 -LsSf "$url" | sh
elif command -v wget >/dev/null 2>&1; then
  wget -qO- "$url" | sh
else
  echo "install.sh: need curl or wget" >&2
  exit 1
fi
