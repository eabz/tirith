#!/usr/bin/env bash
# Bump the crate version, refresh Cargo.lock, commit, and tag.
#
#   scripts/bump.sh patch            0.1.0 -> 0.1.1
#   scripts/bump.sh minor            0.1.0 -> 0.2.0
#   scripts/bump.sh major            0.1.0 -> 1.0.0
#   scripts/bump.sh 0.3.0-rc.1       exact version
#
# Options:
#   --no-commit   only edit Cargo.toml and Cargo.lock
#   --push        push main and the tag (this triggers the release workflow)
#   --dry-run     print what would happen
set -euo pipefail

cd "$(dirname "$0")/.."

usage() { sed -n '2,14p' "$0" | sed 's/^# \{0,1\}//'; exit "${1:-0}"; }

part=""; commit=1; push=0; dry=0
for arg in "$@"; do
  case "$arg" in
    --no-commit) commit=0 ;;
    --push) push=1 ;;
    --dry-run) dry=1 ;;
    -h|--help) usage ;;
    -*) echo "unknown option: $arg" >&2; usage 1 ;;
    *) part="$arg" ;;
  esac
done
[ -n "$part" ] || usage 1

current=$(grep -m1 '^version = "' Cargo.toml | sed -E 's/^version = "([^"]+)"/\1/')
[ -n "$current" ] || { echo "cannot read version from Cargo.toml" >&2; exit 1; }

case "$part" in
  major|minor|patch)
    base="${current%%-*}"                       # drop any pre-release suffix
    IFS=. read -r major minor patch <<<"$base"
    case "$part" in
      major) major=$((major + 1)); minor=0; patch=0 ;;
      minor) minor=$((minor + 1)); patch=0 ;;
      patch) patch=$((patch + 1)) ;;
    esac
    next="$major.$minor.$patch" ;;
  *)
    [[ "$part" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.]+)?$ ]] || { echo "not a semver version: $part" >&2; exit 1; }
    next="$part" ;;
esac

tag="v$next"
echo "version: $current -> $next  (tag $tag)"
[ "$dry" -eq 1 ] && exit 0

if [ "$commit" -eq 1 ]; then
  git diff --quiet && git diff --cached --quiet || { echo "working tree is not clean; commit or stash first" >&2; exit 1; }
  git rev-parse -q --verify "refs/tags/$tag" >/dev/null && { echo "tag $tag already exists" >&2; exit 1; }
fi

sed -i.bak -E "0,/^version = \"[^\"]+\"/s//version = \"$next\"/" Cargo.toml && rm Cargo.toml.bak
cargo update --workspace --quiet          # refresh the crate's own entry in Cargo.lock
cargo check --quiet                       # fail fast if the manifest is broken

if [ "$commit" -eq 1 ]; then
  git add Cargo.toml Cargo.lock
  git commit -q -m "chore(release): $tag"
  git tag -a "$tag" -m "tirith $next"
  echo "committed and tagged $tag"
  if [ "$push" -eq 1 ]; then
    git push origin HEAD "$tag"
    echo "pushed; the release workflow is now building $tag"
  else
    echo "next: git push origin main $tag"
  fi
else
  echo "edited Cargo.toml and Cargo.lock (no commit)"
fi
