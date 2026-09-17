---
id: 0d3fec16-2c05-4b35-a2f5-cd3584956943
permalink: scripts-check-sh-failure-digest-quick-digest-raw-step-logs
title: "scripts/check.sh: failure digest, --quick, --digest, raw step logs"
kind: fact
tags:
- tooling
- check
- digest
paths:
- scripts/check.sh
author: tooling-check
updated_by: tooling-check
created_at: 2026-09-17T04:02:06Z
updated_at: 2026-09-17T04:02:06Z
---

Task 2c3110d2 (2026-09-16), from explore-verify's P1/P2.

- [fact] Every step writes raw output to `${CARGO_TARGET_DIR:-target}/check-<step>.log` (via a mktemp file moved into place, so concurrent runs do not read half-written logs). clippy, test, doc steps print a digest under FAIL; fmt/machete/deny only print log path + rerun command.
- [fact] Digest is POSIX awk inside the `digest()` function (no gawk features; tested with macOS BWK awk only, no Linux awk was available). Rules: strip ANSI; first panic block per `panicked at` location (doctest bundle panics keyed by doctest name), 12 lines, 300 chars/line; first `error...:` / `Caused by:` block per message line, 20 lines, other `-->` locations deduplicated on one `(same error also at: ...)` line; a `---- name stdout ----` section with no panic keeps its first 12 lines (tests returning Err, should_panic); `failed <target>: names` per `error: test failed, to rerun pass`. No block parsed: last 40 non-blank, non-`... ok` lines (5 if every suite in the log passed).
- [fact] `scripts/check.sh --digest [log]` (stdin default) runs before the `cd` to the repo root, so relative paths work. `--quick` = `cargo test --all-features --lib` then `--doc`, stops on first failure, prints a reminder that it is not done.
- [lesson] Fixture validation (explore-verify logs, 18 files): 23/23 unique panic messages incl. left/right values, 14/14 error blocks kept; 10.4 KB over the 14 failing logs, 11.8 KB over all 18 (raw 271 KB). Harness lives outside the repo: scratchpad check-dev/test_digest.py.
- [gotcha] Cargo compiles lib code for several targets, so the same compiler error repeats with the same location; dedupe is by message then location.
