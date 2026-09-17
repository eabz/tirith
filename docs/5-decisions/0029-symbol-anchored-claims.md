# ADR-0029: Symbol-anchored claims and edit-window holds

**Status:** Proposed, 2026-09-17. The swarm lead decides after the hubs
benchmark (task `7ed13e79`). Would refine [ADR-0005](0005-prefix-paths-no-globs.md)
and build on `claim` `wait_secs` (task `ae4ceb58`).

## Context

A claim covers a whole file or directory (ADR-0005). In a swarm, a few hub
files take part in most tasks, so agents queue for a whole file to change a
few lines of it. The user proposed claiming parts of files, and asked that
such claims wait exactly like file claims (`wait_secs`).

Two ways to shorten that queue are on the table: narrower claims
(**symbol anchors**), and shorter claims (**edit windows**: hold a hub file
only while editing it). Before designing either, we measured what each
would have saved.

### Measurement

Data: the four valid e2e benchmark runs (`r2-a`, `r2-b`, `r3-a`,
`r3-b`). Each run has 3 Claude workers, 8 tasks on a small
Python web app, and a full claim log with timestamps. Nearly all conflicts
were on three shared files: `app/config.py`, `app/router.py` and
`app/middlewares/__init__.py`. The workers edited those files only through
scripted string replacements, so we replayed every write command from the
worker transcripts, in timestamp order, against each run's base commit.
The replay reproduced all 12 final files byte for byte and gave 73 edits
to the shared files. We then mapped every changed line to its enclosing
Python symbol with `ast`. The scripts live in the `claims-symbols` lab
directory of the lab session.

**What the edits touched** (out of 73 edits): import lines 53, new
`Config` fields 24, `__all__` entries 24, `build_pipeline` 24,
`Config.from_env` 12, new helper functions 5. Every feature task adds a
field to `Config`, an import and an `__all__` entry, and one
`pipeline.add(...)` call at a specific position in `build_pipeline`.

**Holds.** There were 79 holds on shared files. The median hold was 44 s
and the holds added up to 4,187 s. The edit commands inside them added up
to 122 s, and 8 holds contained no edit at all. From an agent's last read
of a file to its write, the quartiles were 9, 13 and 18 s (max 54 s). Most
workers ran the tests in the same command as the edit.

**Waiting.** There were 48 refusal episodes (one requester, one file, from
the first refusal until the claim was granted), totalling 5,650
episode-seconds. Of that, 2,424 s (43%) passed while another agent really
held the file. The other 3,227 s (57%) was **retry slack**: the file was
already free, but the requester had not retried yet. Taking the union per
agent, workers spent **2,762 s blocked out of 5,923 worker-seconds (47%)**.
Per run: 421, 793, 701 and 847 s.

**The 48 episodes by symbol:**

| Verdict | Episodes | Episode-seconds |
|---|---|---|
| The two agents' symbols were disjoint | 0 | 0 |
| They overlapped only on new fields, imports or `__all__` | 20 | 2,214 |
| They edited the same symbol (`build_pipeline`, `Config.from_env`) | 21 | 2,496 |
| The holder never edited the file (directory claim, or claimed early) | 7 | 941 |

**Counterfactual agent-blocked seconds**, all four runs together:

| Rule | Blocked s | vs today |
|---|---|---|
| File claims, sleep and retry (today) | 2,762 | |
| File claims + `wait_secs` | 1,331 | −52% |
| Strict symbol anchors (any shared container conflicts) | 2,762 | 0% |
| Symbol anchors, new members by name, imports ride along | 2,410 | −13% |
| … + `wait_secs` | 1,097 | −60% |
| Edit windows + `wait_secs`, window = edit −10 s / +15 s | 5 (2 refusals) | −99.8% |
| Edit windows + `wait_secs`, −20 s / +30 s | 112 (18 refusals) | −96% |
| Edit windows + `wait_secs`, −30 s / +60 s | 731 (46 refusals) | −74% |
| Anchors + edit windows + `wait_secs`, −20 s / +30 s | 63 (7 refusals) | −98% |
| Anchors + edit windows + `wait_secs`, −30 s / +60 s | 558 (19 refusals) | −80% |

Caveats. The window rows reuse edit timestamps from runs that were already
serialized by file claims. Without that serialization, agents would reach
`build_pipeline` sooner and closer together, so the window numbers are
optimistic. The sample is one Python app with a single ordering function
that every task edits. In Rust, a tool handler is its own method, so this
repository's hubs may have more disjoint symbols. That is a hypothesis,
not a measurement. `src/server.rs` appears in 35 of this repository's 74
tasks and `src/state.rs` in 25.

**What the numbers say.** Holding a hub file for its whole task is the
cost; the width of the claim matters much less. Symbol anchors cannot
separate tasks that all change the same function, and here every task did.
Edit windows capture almost all of the gain without any change to the
daemon. Anchors add a little on top (112 → 63 s, and 18 → 7 refusals at
the realistic window).

## Decision

### 1. Recommendation

1. **Edit windows now, as protocol with no schema change.** Claim a hub file
   only for the edit, and pass `wait_secs`. The steps are in section 3.
2. **Symbol anchors: accept the design below, but implement it only if the
   benchmark passes.** In the `bench-symbols` e2e arm, anchors plus edit
   windows must cut agent-blocked time by at least 20% against edit windows
   alone, with no lost write and no more failing test runs. Otherwise this
   ADR is rejected for anchors and kept for edit windows.

### 2. Anchor design (for the implementer, if the gate passes)

**Syntax.** A claim target is `path` or `path#Anchor`, inside the existing
`paths` strings. There is no new parameter and no new tool.

- The string is split at the **first** `#`. The path part is normalized
  exactly like `RepoPath` today. A path written with a trailing `/` cannot
  carry an anchor (directories have no symbols).
- Anchor segments may be separated by `::`, `.` or `/`, so Rust
  `State::brief`, Python or TypeScript `Config.from_env`, and Serena name
  paths `State/brief` all work. The stored form uses `::`:
  `app/config.py#Config::from_env`. Responses echo the stored form.
- Generic arguments are dropped (`Store<T>::load` becomes `Store::load`).
  So is a Go receiver (`(*Server).Handle` becomes `Server::Handle`). Any
  `#` after the first is part of a segment (TypeScript `Counter.#count`).
- Segments must be non-empty and contain no whitespace. Everything else is
  honor system.
- A file whose name contains `#` can no longer be claimed by itself;
  claim its directory. Both spellings parse the same way, so two claims on
  it still conflict.

**Overlap** (a pure function; the prototype has 19 tests):

| A | B | Overlap |
|---|---|---|
| paths do not overlap (ADR-0005) | anything | no |
| file or directory, no anchor | anchor in or under it | **yes** |
| `f#X` | `f#X` | yes |
| `f#X` | `f#X::y` (nested) | yes |
| `f#X::a` | `f#X::b` (siblings) | no |
| `f#State` | `f#StateView` | no (whole segments only) |
| `f#Server` | `f#server` | yes (ASCII case-insensitive, to be conservative) |
| `f#X` | `g#X`, where `f` is an ancestor directory of `g` | yes (not a real tree; stay safe) |

The same rule is used everywhere claims are compared: `claim` conflicts,
`wait_secs` retries, `claims_list` with a path, and `task_pull` holds
(ADR-0028).

**Own claims.** An anchor inside a file or anchor the agent already holds
is *renewed*, not added. Claiming a file *absorbs* the agent's anchors in
it, the way claiming a directory absorbs files today. **Release stays
exact:** releasing `f` does not release `f#X`. Spellings that normalize to
the same stored form match. Calling `release` with no paths frees
everything.

**Validation: syntax only; the daemon never checks that a symbol exists.**
The daemon has no parser, and a new symbol is legitimately absent until it
is written. A word search in the file on every claim would add file I/O to
the claim path and still miss a wrong container. The risk is a misspelled
anchor that silently fails to overlap. We reduce it with case-insensitive
comparison and with agent guidance: copy names from Serena
(`get_symbols_overview`, `find_symbol` name paths), an LSP outline, or
`grep -n`. Revisit if the benchmark finds mis-anchored edits.

**New and appended code.** A new member is claimed by its new name, as a
sibling of the existing members: `app/config.py#Config::gzip_min_size`. Two
agents adding different fields do not conflict, and an agent claiming all
of `Config` (for a rename or rewrite) blocks both. A new top-level item is
claimed by its name: `app/config.py#_parse_pairs`. **Import lines, `use` and
`mod` declarations, and export-list entries** (`__all__`, `pub use`,
`export {}`) that a claimed symbol needs are covered by that symbol's claim.
They are order-insensitive one-line insertions. Imports appeared in 53 of
the 73 measured edits and `__all__` in 24, so a separate claim for them
would serialize every task again. There is no `append`
syntax.

**Renames and moves.** Claim the old anchor, the new anchor, and the
callers' files, and publish a `notice` (rename) as today. Other agents'
claims on the old name refuse the rename until they are released. Anchors
are names, not line ranges, so they do not drift when others insert lines.

**The same symbol.** Anchors do not help when several tasks change one
function (`build_pipeline`). Those tasks serialize on the anchor, and the
edit window keeps the queue short. The *order* inside the function is a
semantic question: settle it with a `decision_record` or a contract, not
with claims.

**Concurrent writes to one file.** Anchors let two agents write the same
file at once. Edit windows on a whole file do not.

- Claude Code's Edit and Write tools refuse to write a file that changed
  since it was read, and the agent re-reads it. Nothing is lost, but it
  costs a retry.
- A scripted read-modify-write (Python `str.replace`, `sed -i`) is exposed
  only for milliseconds.
- **A whole-file rewrite from content read earlier** (`cat > f`, or Write
  after a stale read in a client without the check) loses the other agent's
  edit. Under an anchor claim this is forbidden: edit by replacement.
- Serena's symbol edits (`replace_symbol_body`, `insert_after_symbol`) must
  be tested for stale buffers in the benchmark before anchors ship. Until
  then, the anchor guidance tells agents not to use them on a file someone
  else holds anchors in.
- **No separate write lease.** A write lease on the whole file would
  conflict with the other agents' anchors in that file, so it would turn
  back into a file claim. The measured writes take 1–2 s. If the benchmark
  finds lost writes, the fix is a short file-level edit window around each
  write, not a new primitive.

**Half-done edits break other agents' builds.** This already happens
across files, and anchors make it more frequent inside one file. The rule:
every write leaves the file compiling or importable, so a multi-step change
to a symbol is composed first and written once. An agent whose check fails
inside someone else's anchor messages the holder (the conflict and
`claims_list` name the owner) and does not "fix" that symbol.

**Brief, notices, contracts, memory, tasks.** The brief matches on the
**file**, with the anchor stripped on both sides. Anyone claiming any
symbol in `app/router.py` gets the file's notices, decisions and memory
notes. Over-inclusion is cheap, since each section is capped at five rows,
and a symbol-level filter would hide file-wide conventions such as the
middleware order. `notice_publish` `affects`, contract consumers, memory
`paths` and task `paths` may carry anchors for precision. Task holds in
`task_pull` compare anchors with the overlap rule above.

**Budget.** No schema change. The `claim` description is at the per-tool
cap, so anchors are documented in `docs/1-about/04-primitives.md` and
`AGENTS.md`. The only output change is that responses echo the stored
form.

**`wait_secs`.** Nothing to add: a waiting claim re-runs the same atomic
overlap check whenever a lease is released or expires, so anchored claims
wait exactly like file claims. One risk: a waiting *file* claim can
starve behind a stream of anchor claims in that file, because retries are
not queued. If the benchmark shows it, make `claim_waiting` FIFO: a new
claim that overlaps an older waiter's request waits behind it.

**Compatibility.** Daemons without this change accept `f#X` as an
unrelated path that does not overlap `f`, which fails open. Anchor
guidance must name the first Tirith version that supports anchors. The
shim replaces a daemon of another version (ADR-0016), so every agent in a
repository sees the same rule.

**Implementation sketch.** Carry the anchor inside `RepoPath`, whose stored
string becomes `path#A::B`. Add `RepoPath::file()` and `anchor()`, make
`overlaps` anchor-aware, keep `is_ancestor_of` path-only, and add
`covers(held, p)` for renew and absorb. Brief matching calls `file()`.
Persisted JSON is unchanged (still a string). The pure prototype has the
parser, the overlap rule and the test table.

### 3. Edit-window protocol (ships now)

For a hub file (one that appears in the paths of more than one open task,
or one the brief shows other agents working in):

1. Read the file and prepare the change **without** a claim.
2. `claim` the file with `wait_secs` (up to 120).
3. Re-read the region if the claim waited, apply the change in one write,
   and run the quickest relevant check.
4. `release` that path immediately. Keep your own non-hub files claimed for
   the whole task as today.

The measured windows (read to write: 13 s median, 18 s p75) fit inside a
claim of well under a minute.

## Alternatives

- **Line-range claims** (`f:120-180`). Rejected: ranges drift as other
  agents insert lines, and agents cannot know the final ranges before
  writing.
- **A parser in the daemon** (tree-sitter) to verify anchors or derive them
  from diffs. Rejected: a heavy dependency for each language, and new
  symbols cannot be verified before they exist.
- **An `append` anchor** (`f#Config+`) that never conflicts with other
  appends. Rejected: naming the new member does the same job with no new
  syntax. Imports and export lists ride on the symbol claim.
- **A short write lease beside anchor claims.** Rejected for now: it
  conflicts with the other agents' anchors and collapses into a file claim.
  Revisit only if lost writes are measured.
- **Anchors only, without edit windows.** Rejected on the numbers: −13%
  (−60% with `wait_secs`), against −96% for edit windows with `wait_secs`.

## Consequences

- Edit windows need no code. `AGENTS.md` and the dogfooding guide should
  describe the four steps, and the benchmark should compare them with
  whole-task file claims.
- If anchors ship, claims become more precise, but agents must pick
  correct names, avoid whole-file rewrites on shared files, and keep each
  write compiling. The daemon cannot enforce any of that.
- A future ADR may add FIFO waiting if file claims starve behind anchors.
