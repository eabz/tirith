#!/usr/bin/env bash
# freeze_bin.sh <tirith_checkout> <dest_dir> -- copy the checkout's release
# build to <dest_dir>/tirith and describe it in <dest_dir>/tirith.build.json
# (version, git commit, whether src/ or Cargo files had uncommitted changes,
# sha256). Every arm of a benchmark then runs this copy, so a later
# `cargo build` cannot change the daemon mid-benchmark. setup.sh records the
# sidecar in meta.json as `tirith_build` when TIRITH_BIN points at the copy.
#
# Build first: cargo build --release (in the checkout). FREEZE_NOTE='...' is
# stored as `note` (e.g. which uncommitted work a dirty tree contained).
set -euo pipefail
[ $# -eq 2 ] || { echo "usage: $0 <tirith_checkout> <dest_dir>" >&2; exit 2; }
SRC=$(cd "$1" && pwd)
BIN=$SRC/target/release/tirith
[ -x "$BIN" ] || { echo "no release build at $BIN; run cargo build --release" >&2; exit 1; }
mkdir -p "$2"
DEST=$(cd "$2" && pwd)
if [ -e "$DEST/tirith" ]; then
  echo "$DEST/tirith already exists; freeze into a fresh directory" >&2
  exit 1
fi
cp "$BIN" "$DEST/tirith"
chmod +x "$DEST/tirith"
DIRTY=$(git -C "$SRC" status --porcelain -- src Cargo.toml Cargo.lock build.rs | wc -l | tr -d ' ')
jq -n --arg version "$("$DEST/tirith" --version)" \
  --arg commit "$(git -C "$SRC" rev-parse HEAD)" \
  --argjson dirty "$([ "$DIRTY" = 0 ] && echo false || echo true)" \
  --arg built_at "$(date -u -r "$BIN" +%Y-%m-%dT%H:%M:%SZ)" \
  --arg frozen_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg sha256 "$(shasum -a 256 "$DEST/tirith" | cut -d' ' -f1)" \
  --arg source "$BIN" --arg note "${FREEZE_NOTE:-}" \
  '{version:$version, commit:$commit, src_dirty:$dirty, built_at:$built_at, frozen_at:$frozen_at,
    sha256:$sha256, source:$source, note:(if $note == "" then null else $note end)}' > "$DEST/tirith.build.json"
cat "$DEST/tirith.build.json"
echo "TIRITH_BIN=$DEST/tirith"
