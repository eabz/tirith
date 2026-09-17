#!/usr/bin/env bash
# The definition-of-done chain from AGENTS.md, one line per step, so an
# agent reading the output spends a dozen tokens instead of thousands.
#
#   scripts/check.sh            full chain (the definition of done)
#   scripts/check.sh --quick    unit tests, then doctests; stops at the first
#                               failure. For the edit loop, not for done.
#   scripts/check.sh --digest [log]
#                               print the failure digest of a cargo log
#                               (stdin when no file is given)
#
# A failing clippy, test, or doc step prints a digest under its FAIL line:
# the first panic per location with its assertion message, the first
# compiler error per message with the other locations on one line, and the
# failed test names per target; the last 40 lines when nothing parses
# (5 when every test suite in the log passed), without blank lines and
# passing tests.
# The raw output of each step is kept in target/check-<step>.log.
# Exit status is non-zero if any step failed.
set -u

# Failure digest of cargo output (bash + POSIX awk, macOS and Linux).
digest() {
  awk '
    function cut(s) { return length(s) > 300 ? substr(s, 1, 300) "..." : s }
    function flush_section() {
      if (sect != "" && !sect_kept && sect_n > 0) add("---- " sect "\n" sect_buf)
      sect = ""; sect_buf = ""; sect_n = 0; sect_kept = 0
    }
    function add(text) { blk[++nblk] = text }
    function close_block() {
      if (mode == "") return
      if (mode == "skip") { mode = ""; buf = ""; n = 0; return }
      if (sect != "" && !sect_kept) { buf = "---- " sect "\n" buf; sect_kept = 1 }
      if (mode == "error") {
        # Cargo builds the same code for several targets: list each
        # location once.
        if (!(key in seen)) { add(buf); seen[key] = nblk; locs[nblk "|" blk_loc] = 1 }
        else if (!((seen[key] "|" blk_loc) in locs)) {
          i = seen[key]; locs[i "|" blk_loc] = 1
          also[i] = also[i] (also[i] == "" ? "" : ", ") (blk_loc != "" ? blk_loc : "?")
        }
      } else add(buf)
      mode = ""; buf = ""; n = 0; blk_loc = ""
    }
    function append(line, cap) {
      if (n < cap) buf = buf "\n" cut(line)
      else if (n == cap) buf = buf "\n..."
      n++
    }
    BEGIN { esc = sprintf("%c", 27) }
    {
      gsub(esc "\\[[0-9;]*[A-Za-z]", "")
      line = $0
      # The fallback tail skips blank lines and passing tests.
      if (line !~ /^[ \t]*$/ && line !~ / \.\.\. ok$/) tail[++nt % 40] = line
    }
    # A block ends at a blank line or where the next block starts.
    mode != "" && line ~ /^[ \t]*$/ { close_block(); next }
    mode == "panic" && line ~ /^note: run with `RUST_BACKTRACE/ { close_block(); next }
    mode == "panic" && line ~ /^thread .* panicked at / { close_block() }
    mode == "error" && line ~ /^(error|warning)(\[[A-Za-z0-9]+\])?: / { close_block() }
    mode != "" && line ~ /^(---- .* stdout ----|failures:|test result: )/ { close_block() }
    mode != "" {
      if (mode == "error" && blk_loc == "" && line ~ /^ *--> /) { blk_loc = line; sub(/^ *--> /, "", blk_loc) }
      append(line, cap); next
    }
    /^---- .* stdout ----$/ {
      flush_section(); sect = line; sub(/^---- /, "", sect); sub(/ stdout ----$/, "", sect); next
    }
    /^test result: ok/ { oks++ }
    /^test result: FAILED/ { fails++ }
    /^(failures:|test result: )/ { flush_section(); next }
    /^test .* \.\.\. FAILED$/ {
      name = line; sub(/^test /, "", name); sub(/ \.\.\. FAILED$/, "", name)
      pending = pending (pending == "" ? "" : ", ") name; next
    }
    /^error: (test|doctest) failed, to rerun pass `/ {
      target = line; sub(/^[^`]*`/, "", target); sub(/`.*$/, "", target)
      failed[++nfailed] = cut("failed " target ": " pending); pending = ""; next
    }
    /panicked at / {
      loc = line; sub(/^.* panicked at /, "", loc); sub(/:$/, "", loc)
      key = (loc ~ /doctest_bundle|rustdoctest/) ? sect "|" loc : loc
      if (key in panics) { mode = "skip"; sect_kept = 1; next }
      panics[key] = 1; mode = "panic"; cap = 12; buf = cut(line); n = 1; next
    }
    /^(error|warning)(\[[A-Za-z0-9]+\])?: / || /^Caused by:$/ {
      if (line ~ /^warning/) next
      if (line ~ /^error: (could not compile|doctest failed|test failed|[0-9]+ targets? failed)/) next
      if (line ~ /^error: aborting due to/) next
      mode = "error"; key = line; cap = 20; buf = cut(line); n = 1; next
    }
    sect != "" && line !~ /^[ \t]*$/ && sect_n < 12 {
      sect_buf = sect_buf (sect_n ? "\n" : "") cut(line); sect_n++
    }
    END {
      close_block(); flush_section()
      if (nblk == 0) {
        # When every suite passed the failure is outside the tests: keep it short.
        k = (oks && !fails) ? 5 : 40
        printf "(no failure parsed%s; last %d lines)\n", oks ? ", " oks " test suites ok" : "", k
        for (i = nt - k + 1; i <= nt; i++) if (i > 0) print cut(tail[i % 40])
      }
      for (i = 1; i <= nblk; i++) {
        if (blk[i] == "") continue
        print blk[i]
        if (also[i] != "") print cut("(same error also at: " also[i] ")")
      }
      for (i = 1; i <= nfailed; i++) print failed[i]
    }
  ' "$@"
}

# --digest reads paths relative to the caller, so it runs before the cd.
if [ "${1:-}" = --digest ]; then
  shift
  digest "$@"
  exit 0
fi
cd "$(dirname "$0")/.."
logdir=${CARGO_TARGET_DIR:-target}
# A daemon a test spawns must never start the menu bar tray (ADR-0019):
# several agents running this at once used to leave duplicate icons.
export TIRITH_NO_TRAY=1
status=0

# step NAME DIGEST CMD...: run CMD, keep its output in $logdir/check-NAME.log,
# print one line; on failure also print the digest when DIGEST is "yes".
step() {
  local name=$1 want=$2; shift 2
  mkdir -p "$logdir"
  local log="$logdir/check-$name.log" tmp
  tmp=$(mktemp "$logdir/.check-$name.XXXXXX")
  if "$@" >"$tmp" 2>&1; then
    case $name in
      test*) printf '%-8s pass  (%s suites)\n' "$name" "$(grep -c '^test result: ok' "$tmp")" ;;
      *) printf '%-8s pass\n' "$name" ;;
    esac
  else
    printf '%-8s FAIL  (log: %s; rerun: %s)\n' "$name" "$log" "$*"
    [ "$want" = yes ] && digest "$tmp"
    status=1
  fi
  mv -f "$tmp" "$log"
}

case ${1:-} in
  --quick)
    step test-lib yes cargo test --all-features --lib
    [ $status = 0 ] && step test-doc yes cargo test --all-features --doc
    printf '%-8s %s\n' quick "not the definition of done: run scripts/check.sh before release"
    exit $status
    ;;
  "") ;;
  *)
    printf 'usage: scripts/check.sh [--quick | --digest [log]]\n' >&2
    exit 2
    ;;
esac

step fmt     no  cargo fmt --all -- --check
step clippy  yes cargo clippy --all-targets --all-features -- -D warnings
step test    yes cargo test --all-features
step doc     yes env RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps
step machete no  cargo machete
step deny    no  cargo deny check
# Test daemons serve a tempfile directory: under $TMPDIR/T/ on macOS, /tmp/ on Linux.
leaked=$(pgrep -f "tirith serve --root (/.*/T/|/tmp/)" | wc -l | tr -d ' ')
[ "$leaked" = "0" ] && printf '%-8s pass\n' daemons || { printf '%-8s FAIL  (%s leaked test daemons)\n' daemons "$leaked"; status=1; }
exit $status
