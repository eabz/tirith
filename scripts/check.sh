#!/usr/bin/env bash
# The definition-of-done chain from AGENTS.md, one line per step, so an
# agent reading the output spends a dozen tokens instead of thousands.
# Exit status is non-zero if any step failed; rerun a failed step by hand
# to see its output.
set -u
cd "$(dirname "$0")/.."
status=0
step() {
  local name=$1; shift
  if "$@" >/dev/null 2>&1; then
    printf '%-8s pass\n' "$name"
  else
    printf '%-8s FAIL  (rerun: %s)\n' "$name" "$*"
    status=1
  fi
}
step fmt     cargo fmt --all -- --check
step clippy  cargo clippy --all-targets --all-features -- -D warnings
if out=$(cargo test --all-features 2>&1); then
  printf '%-8s pass  (%s)\n' test "$(printf '%s\n' "$out" | grep -c '^test result: ok')" | sed 's/(\(.*\))/(\1 suites)/'
else
  printf '%-8s FAIL\n' test
  printf '%s\n' "$out" | grep -E "^test .*FAILED|panicked at|^error" | head -20
  status=1
fi
step doc     env RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps
step machete cargo machete
step deny    cargo deny check
leaked=$(pgrep -f "tirith serve --root /.*/T/" | wc -l | tr -d ' ')
[ "$leaked" = "0" ] && printf '%-8s pass\n' daemons || { printf '%-8s FAIL  (%s leaked test daemons)\n' daemons "$leaked"; status=1; }
exit $status
