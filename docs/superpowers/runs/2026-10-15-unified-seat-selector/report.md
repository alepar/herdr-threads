status: clean
metrics: pending (upstream-feedback not yet run)

# Report: unified seat selector (run 2026-10-15-unified-seat-selector)

Branch `super-auto/unified-seat-selector`, base `main`. Epic `ht-u3ym`.

## Implemented

All 10 beads landed. Nothing was escalated, left pending or parked. The phase-6 sweep at 82f475f56 passed 6538 tests with 0 failures, and the leak check was clean. (run.md codeBuckets)

| Bead | What landed | Commits | Source |
|---|---|---|---|
| ht-u3ym.3.1 | Move-only extraction of the parent-narrowing helper (`panes::resolve_parents`) and the seat-allocation helper | 71a56aa..ec0fb0c | ledger; bd |
| ht-u3ym.2 | Seat argument slots in the parser: `SeatSlot`, `SeatMapping`, `SeatNarrowing` | ec0fb0c..0288c4b | ledger; bd |
| ht-u3ym.1 | Pure seat selector resolver: readings, tiering, 32-candidate cap, path renderer | 0288c4b..8b9730f | ledger; bd |
| ht-u3ym.3.2 | Seat resolution stage core, plus the parser flip (placeholders, post-freeze checks, leftover guard) | 8b9730f..a74b2c7 | ledger; bd |
| ht-u3ym.3.3 | Seat resolution wired into send and direct send, invite, `handoff --existing`, the read commands, and `seat inspect`/`retire`/`rebind` | 909e505..6f6b086 | ledger; bd |
| ht-u3ym.5 | Docs, skill and trust policy: `docs/agent-usage.md`, `integrations/skill/SKILL.md`, TRUST-POLICY C1 | 6f6b086..58a6566 | ledger; bd |
| ht-u3ym.4 | Configuration smoke test on every seat-taking command (`seat_selector_matrix`) | 58a6566..2e086f2 | ledger; bd |
| ht-u3ym.6 | Integration sweep, including a fix for help-text drift | 2e086f2..c33da4a | ledger; bd |
| ht-u3ym.11 | Fix: the seat stage now allocates last (see below) | f2cf94c..4bfb8d9 | ledger; bd; run.md stepBackCode-round-1 |
| ht-u3ym.12 | Fix: the session refusal runs before the seat stage, and the TRUST-POLICY C1 allocation claim was narrowed to match the code | 71fe6ea..a73a582 | ledger; bd; run.md final-review fix round |

Every review was clean.

How ht-u3ym.11 changed allocation: pass 1 (`resolve_cli_seats`) makes no journal writes. Pass 2 (`allocate_deferred_seats`) runs after `derive_caller`, the operator-actor check and the direct-send capability check.

## Remaining

- **Out of scope (filtered).** [Nit] `src/cli/seat_resolution.rs:230; :234`: a caller-read error other than HostUnavailable, such as a `pane current` timeout, is treated as "no caller". The recorded reason: D3 names only "outside Herdr" or a stale HERDR_PANE_ID; the timeout case is hardening, and it never selects the wrong target. (run.md scopeFilter-round-1; roast-pr-1)
- **Round-2 punch list.** [Nit] `src/cli/seat_resolution.rs:327`: the direct-ACK rule (D7) is implemented twice, using made-up seat IDs and an `expect_err` in production code. Still open. (roast-pr-2; post-cap audit)
- **Post-cap audit, unverified Nit.** The comments at `src/cli/mod.rs:729` and `src/cli/direct_send.rs:269` still say "a refused command allocates nothing", which commit 9aadb5b2a narrowed everywhere else. This is documentation only. (roast-pr-post-cap-audit.md)
- **Final review** (all items from final-review-reentry-2.md):
  - **Needs your confirmation.** `seat retire`, `seat rebind` and `--replace` by name use proximity tiering, as planned. When a match in the caller's tab and another match elsewhere both exist, the nearer seat is chosen silently, and retire cannot be undone. The only safeguards are the `resolved:` line and `--operator`.
  - The session refusal now runs earlier for every command. A session-less agent running `--human read` or `--operator` now gets `session_claim_required` (exit 4). That matches TRUST-POLICY, but the change was never reviewed on its own. `caller::derive` also runs twice (mod.rs:733 and :1211).
  - Stale comments also sit at mod.rs:725-730 and on `allocate_deferred_seats` (seat_resolution.rs:345-348).
  - A `resolved:` line printed in pass 1 can be followed by a later refusal. This is cosmetic.
- **Final review, deferred** (final-review-reentry-1.md and -2.md):
  - `check_unique_recipients` is dead code after the `retain`.
  - `--seat ""` can match an empty label.
  - A weak-assertion minor recurred in 3 beads (ht-u3ym.3.3, .4, .6). A follow-up should let the `Runtime` fixture assert end-to-end completion.
  - All coverage uses the fake host and daemon. No real Herdr or daemon ran.
  - The `resolved:` line is checked through a test-only thread-local, never on real stderr.
- **Parked graph changes** (graph-pass: depth 5→5 · width 1.6→1.6 · applied 0 · parked 2):
  - Narrow `ht-u3ym.5 <- ht-u3ym.3.3` to `<- ht-u3ym.3.2`.
  - Fold ht-u3ym.3.1 into ht-u3ym.3.2.
  
  (run.md parked)
- **Suite budget.** nextest took 312.7s (510s wall) against the 300s AGENTS.md budget. This is not a release gate. (friction.md; codeBuckets.sweep)

## Gotchas & surprises

- **Design roast round 1** produced 9 findings, applied as a patch (run.md stepBackDesign-round-1; roast-design-1 and its step-back):
  - one renderer contract, in D6;
  - `SeatNarrowing` became its own value, with no tab defaulting;
  - in leaf 2, every seat string became a slot;
  - allocation keeps refuse-before-allocate order;
  - the candidate cap is 32 everywhere.
- **Design roast round 2** converged after 6 more findings. Several of round 1's fixes had been incomplete: a move-only helper that also had to carry the refusal, a `{:?}` remnant, a test-only dispatch guard, the agent-name rule, and two narrowing mechanisms. (roast-design-2)
- **Coverage round 2 widened** instead of shrinking: 11 → 12 findings, all of them new. The two-round cap held. (run.md coverage-round-2)
- **Code roast round 1:** allocation happened slot by slot and before `derive_caller`. This was drift from D5, not a design flaw. It was fixed with a two-pass stage (ht-u3ym.11). (roast-pr-1 and its step-back)
- **M1:** pass 1 made daemon and Herdr calls before the agent-without-session refusal, so session-less agents got the wrong error. Fixed in ht-u3ym.12, then cleared by the post-cap audit. (final-review-reentry-1.md)
- **Bottleneck:** ht-u3ym.3.2 took 1h51m of the 3h20m critical path. The run was graph-bound, with achievable width 2 against a cap of 8. (ledger Profile)
- **Largest what-if** (a model estimate): halving ht-u3ym.3.2 saves 54 minutes (27%). (ledger Profile)
- **Base drift:** main (v1.1.0) was absorbed at b761a8194 with 0 conflicts. (run.md baseAbsorbed)

## Entrypoints

1. `src/cli/seat_selector.rs`: `resolve`, `verify_nickname`, `render_path`, `Tier`, `Outcome`. (ht-u3ym.1)
2. `src/cli/commands.rs`: `SeatNarrowing`, `SeatSlot`, `SeatMapping`. (ht-u3ym.2)
3. `src/cli/panes.rs`: `resolve_parents`. (ht-u3ym.3.1)
4. `src/cli/seat_resolution.rs`: `resolve_cli_seats` (pass 1), `allocate_deferred_seats` (pass 2), `post_freeze_checks`. (ht-u3ym.3.2, .11)
5. `src/cli/mod.rs` `run_parsed_in_pane`: `derive_refusing_origin`, then pass 1, then pass 2. (ht-u3ym.3.3, .12)
6. `src/cli/direct_send.rs`, `src/cli/handoff_delivery.rs`. (diff)
7. `tests/cli/seat_selector.rs`; `tests/cli/cooperative.rs` (`seat_selector_matrix`, the clap tree-walk). (diff)
8. TRUST-POLICY.md C1, `docs/agent-usage.md`, `integrations/skill/SKILL.md`. (ht-u3ym.5)

## Smells

- Parked findings: none. (ledger: fix-pass entered 0)
- Parked degraded-verdict records: none. (run.md)
- DONE_WITH_CONCERNS reports: none. (task reports)
- Merged after a fix pass without re-review: none. (ledger Metrics)
- regressionPass and sweepFix fixes: none recorded. (run.md)
