# ADR-0039: Every text an agent reads uses the fewest words; `tools/list` budget 33,000

**Status:** Accepted, 2026-10-10, by the owner's decision. Supersedes
the budget and the prose rule of
[ADR-0034](0034-complete-tool-definitions-and-guide.md); its
completeness rules (a description on every parameter, enums, an output
schema per tool, annotations) stand.

## Context

ADR-0034 made every tool definition complete and let `tools/list` grow
to 44,500 chars, paid by every session. The owner then asked that every
explanation, context text and tool an agent reads use the fewest words
possible, and the protocol moved to the server instructions
([ADR-0038](0038-instructions-carry-the-protocol.md)), which already
say what the loop is, what a brief carries and what `wait_secs` does.
A tool description that repeats them is paid twice per session.

## Decision

1. **A description says what a name cannot.** Each tool keeps one or
   two sentences: what it does, the one fact about its result an agent
   needs, and a sibling only where confusion is real. Parameter
   descriptions are phrases. Output fields list their keys and nothing
   else. The instructions, not the descriptions, carry the protocol.
2. **Nothing is dropped from the wire.** Every parameter is still
   described, closed values still carry enums, every tool still has an
   output schema and annotations. Only words went.
3. **The budget moves down.** `tools/list` 33,000 chars (measured
   32,522 for 22 tools) and 2,700 per tool, in `tests/budgets.rs`. The
   instructions stay under 4 KB (measured about 3 KB). The rule for
   every later addition: fit the budget, or write the ADR that raises
   it and says what the bytes buy.
4. **The same rule governs the rest of what agents read:** the
   instructions, the `tirith init` block, briefs, memory notes and the
   playbook. Measure before adding; prefer removing.

## Alternatives

- **Keep ADR-0034's prose and trust the budget.** Rejected: the owner
  ranked tokens above completeness-by-prose once the protocol had a
  cheaper home.
- **Drop parameter descriptions or output schemas** to go lower.
  Rejected: they are what lets an agent call a tool right the first
  time; the words around them were the waste.

## Consequences

- About 12,000 fewer chars per session than before, 10,000 fewer than
  the day's start, with the same information on the wire.
- `docs/1-about/04-primitives.md` stays the long form; a description
  that needs a paragraph points there by being short.
