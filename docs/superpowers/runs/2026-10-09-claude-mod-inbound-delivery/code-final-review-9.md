**Re-review of the post-review-8 mod fixes (1c66f49f..73a75cbf, integrations/claude/mod/), independent opus reviewer, read-only.**

**Verdict: ready.** Both commits fix what they claim. No must-fix defects. `scripts/test-claude-mod`: 84/84, strict validate OK.

- **dc5b2f6c (`framedIds`):**
  - The new `attention (\S+):$` alternative matches exactly the header `frame()` emits.
  - The daemon fixes attention ids as `attention:<u64>`, which contain no spaces.
  - Body lines are indented on every line terminator, so a body cannot match the pattern.
  - CRLF prompt text still parses.
  - Rule (b)'s `turnId` matching now also works for batches that carry an attention block.
- **73a75cbf (`settlePred`), no loss:**
  - `rec.delivered` is written only by `complete()`, after a successful submit, context or append, and by an earlier `settlePred`.
  - So an id that is already delivered was presented to the model.
  - An ack that was in flight at dispose stays in the loaded `unacked` and is retried on connect, on re-stream, and every 30 s.
  - `stale_generation` clears `delivered`, so the id counts as fresh again.
- **Invariants:**
  - `S.turns.open` is set before `settlePred` runs, so `pump()` stays busy and nothing is submitted inside a turn.
  - At-most-once still holds through the `predHas` exclusions.
- **Minors:**
  - Attention ids are not in `rec.delivered`, so a successor still writes a second `delivered` ledger entry for `attention:N` (ledger and record noise only; repro by the reviewer).
  - `settlePred` does not add settled attention ids to `runAttention`. This is harmless, because watch emits each attention id once per process.
  - The stale-`submitting` scenario is covered by a targeted delivery test, not by the stress model.
