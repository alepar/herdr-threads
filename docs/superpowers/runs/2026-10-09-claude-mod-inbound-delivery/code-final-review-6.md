**Verdict: not ready.** I found no new code defect that blocks landing, and both blockers from final review 5 are fixed at HEAD 843c127c. Two gates remain open. No full-suite measurement of this branch exists yet; the caller's sweep runs after this review. And the mod is installed and enabled by default but has never run inside a real Claude Code session.

## Step 1: my own review against the spec

Review package: `.superpowers/sdd/ht-j16-plan/review-25db37f3..843c127c.diff` (73 commits). I checked it against the design doc `docs/superpowers/runs/2026-10-09-claude-mod-inbound-delivery/2026-10-09-claude-mod-inbound-delivery-design.md`. I read the full sources of `src/service/mod_channels.rs`, `src/daemon/transport/watch_connection.rs`, `src/cli/watch.rs`, the dispatch/ack/store/scheduler/hook diffs, `src/app.rs` wiring and `integrations/claude/mod/hooks/register.js`.

What I verified at HEAD:
- **Mod tests:** `scripts/test-claude-mod` passes: `validate --strict` passes and `claude plugin test` gives 53 pass, 0 fail. This is the first run after merging ht-j16.26 (49 tests) and ht-j16.27 (48 tests), which were tested separately.
- **Lint and format:** `cargo clippy --locked --all-targets --all-features -D warnings` and `cargo fmt --check` are both clean. The merge check had only run `cargo check`.

What composes correctly across tasks:
- **Liveness is per seat** for wake, poke and the hook digest.
- **A suppressed wake does not spin the lane:** the candidate is dropped and retried on the safety tick.
- **The registry is safe against races:** a close only acts on the entry it judged, and an unregister after a replace does nothing.
- **Acks are decided against the daemon's own records:** the binding, its generation and the stored body length (for truncation), not what the client claims. The resume re-ack path is covered by an integration test.
- **Prior blockers are fixed:**
  - F1: the mod now delivers only while its watch run is connected (ht-j16.26).
  - F2: body lines in the mod's frame are indented, so a peer cannot forge a header (ht-j16.27).
- **Rebind-grace test:** the "pane working" workaround is gone, and the test now asserts no native prompt during the grace.

New findings:
1. **Default-on with no live evidence (needs your decision).** `setup claude`, `doctor --fix` and the installer now write `CLAUDE_CODE_PLUGIN_DIRS` by default, and `mod_delivery` defaults to `on`. A registered channel switches off native wake and hook digests for that seat. If the real engine behaves differently from the stubs, the risk is concrete: the context path acks optimistically when the hook returns, so receipts could settle for text the model never saw. A mod that registers but never delivers holds native wake off for 10 minutes per stall cycle.
2. **The mod-channel worker is not supervised** (this upgrades the ledger's Task 2 [health-visibility-gap]). If the `herdr-mod-channels` thread dies:
   - nothing sweeps, closes stalled channels, closes channels whose binding changed, or pushes Attention;
   - a `Live` entry has no grace deadline, so `seat_live` stays true and native wake and hook digests stay off;
   - the seat then gets nothing new until its watch disconnects.

   Messages stay pending (an outage, not loss), and a panic there looks unlikely. It deserves a bead: either make it a monitored lane, or treat the registry as not live when no sweep has run recently.
3. **Minor: the attention item is the bare marker.** `src/cli/watch.rs` emits `notification::policy::MARKER` without the launch selectors that ht-j16.24 added to truncation markers. With a live channel the hook's ready commands are omitted, so on a non-default state dir or endpoint the only instruction points at the wrong daemon.
4. **Minor: unbounded per-session sets.** `rec.delivered` in `$.store` and the watch's `emitted` set are never pruned, and the whole record is re-serialized on every save. Also, `rec.attentionVersions` is written but never read. Final review 5 raised the first point as F4 and it is still unaddressed.
5. **Housekeeping.** The ht-j16.26 task worktree was kept for an untracked `integrations/claude/mod/node_modules/`, which is not gitignored. Remove the worktree, and consider adding the ignore entry.

## Step 2: ledger triage

- **No `Recurring`, `parked` or `BLOCKED-AUTH` lines.** Metrics are consistent: 20 merges, 0 failures.
- **Unlabelled cluster, by class: no RED-first TDD evidence.** It appears in tasks 1, 2, 4, 10, 13, 15 and 17. This is a pipeline defect: implementer runs don't capture a failing run before the fix. The regression tests for fix beads ht-j16.17, .20, .22 and .24 are not shown to fail on the old code, and the ht-j16.19 tests passed before that change. It should be reported upstream; it does not block the code.
- **Already resolved by later tasks:**
  - Task 7 [unreachable-status-field] and [best-effort-guess] (`remote-settings.json`) were fixed by ht-j16.22.
  - Task 8 [workaround-masks-bug] was fixed by ht-j16.17.
  - Task 5 [unverified-integration] is covered by `tests/integration/mod_delivery.rs`.
- **Task 2 [unexplained-test-anomaly] and Task 4 [test-summary-swallowed]** come from stdio redirection in existing lib tests (`tests/service/composition.rs` uses `dup2`); the branch adds none. The exit code still reflects pass or fail, and nextest isolates each test. Deferrable.
- **Task 7 [scope-side-effect]** (`doctor --fix` and the installer install the mod) feeds into decision item 1.
- **Task 2 [health-visibility-gap]** is upgraded to a follow-up bead (finding 2).
- **Task 9's live-path minors** are untested scope, listed below.

## Must fix before landing
1. **Run the deferred full-suite sweep** (`cargo nextest run --locked --all-targets --all-features`), check the 5-minute budget, and run `scripts/check-no-leaked-processes` with `HT_LEAK_RUN_ID`. No measurement exists. The branch adds a roughly 2.2k-line integration suite with fixed sleeps of up to 3.2 s and a 64-watch cap test.
2. **Decide on default-on with no live evidence.** Either do the ht-j16.9 live dry run (`python3 tests/native/claude_mod/stress.py --profile <signed-in dir> --iterations 1`, then 20 iterations), or ship the mod opt-in (default `mod_delivery: off`, or setup not writing `CLAUDE_CODE_PLUGIN_DIRS` without a flag) until live evidence exists.
3. **Strongly recommended:** file a bead for the worker supervision gap (finding 2) before landing.

## Untested scope
- **Never run in a real Claude Code session.** None of the 14 live scenarios ran, because the copied profile was not signed in. That leaves unexercised:
  - real `$.prompt.submit`, `$.session.append` and context attachment;
  - turn and abort event ordering;
  - `stream.return()` killing the child;
  - module reload, `/clear`, `/resume` and `/branch` in a real engine.
- **The live driver itself has never run.** In `stress.py`, the tmux orchestration, thread bootstrap flags, receipts DB path and fake pane ids are untested. Its marker-file hooks (`stop-armed`, `block-ups`) are never installed, and it is not wired to an isolated Herdr session.
- **Recorded evidence is old.** `docs/evidence/claude-mod-delivery/` is at 13d795f9, not HEAD. The unit level was re-run at HEAD in this review (53 pass); the live gap stands.
- **Not run at HEAD:** the full nextest suite, the budget measurement and the leak check. Task 18 also skipped its leak check.
- **Single-process `cargo test --lib`:** the run summary is lost to stdio redirection in existing tests, so that mode shows no summary line.

## Deferred OK
- **Task 1:** [tdd-red-not-observed], [skipped-optional-check], [unverified-test-coverage].
- **Task 3:** [pre-auth-read] (cooperative model), [lint-allow].
- **Task 6:**
  - [context-delivery-timing], covered by the TRUST-POLICY limit "an outer mod could strip context";
  - [api-probe-weak], which recovers on its own through grace expiry or the stall handover;
  - [seed-not-overridable] and [spec-file-skipped];
  - [untested-live-behavior], which is untested scope above.
- **Task 2:** [unmeasured-cost] (bounded at 64 seats, coalesced by the dirty flag), [test-api-deviation], [non-atomic-register] (the next pass corrects it), [tdd-not-followed].
- **Task 4:** [missing-red-first-log].
- **Task 5:** [style-nit], [fake-only-coverage].
- **Task 8:** [tolerant-assertion]; [fixed-sleeps] depends on the budget measurement in must-fix item 1.
- **Task 10:** [default-impl-drift] (test fakes only).
- **Task 12:** [transport-close-reads-retryable] (only very old daemons are affected), [coincidental-red-tests].
- **Task 14:** [scope-deviation], [new-public-field].
- **Task 16:** [doc-line-wrap], [stale-fixture-text].
- **Task 17:** [style-import-placement].
- **Task 19:** [thin-test-margin] (the fixed seed makes it deterministic).
- **Task 20:** [unrequested-test-edit].
- **Accepted design trade-offs:**
  - `replaced` exits 3.
  - `mod_delivery` is read only at daemon start (bead ht-h2u is open).
  - The stall clock resets on each re-registration. It is recorded as an accepted limit in TRUST-POLICY, and simply carrying the clock over would stall a fresh registration at once.
- **Findings 3, 4 and 5** above.

Key files:
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto-claude-mod-inbound-delivery/integrations/claude/mod/hooks/register.js
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto-claude-mod-inbound-delivery/src/service/mod_channels.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto-claude-mod-inbound-delivery/src/service/workers.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto-claude-mod-inbound-delivery/src/cli/watch.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto-claude-mod-inbound-delivery/docs/evidence/claude-mod-delivery/README.md
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto-claude-mod-inbound-delivery/.superpowers/sdd/ht-j16-plan/progress.md