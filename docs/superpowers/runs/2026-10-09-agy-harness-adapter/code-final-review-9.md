# Final review 9 — ae4e6095

**Verdict: not ready.** The code is in good shape: I found no Critical or Important defect in the new work. It is not ready for two reasons. Nobody has run the full suite on this branch yet, and the genuine-agy evidence plus the R13 report were made before the last agy delivery change.

- HEAD is `ae4e6095`. The fork point is `7c40c83c`.
- The review package is `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/.superpowers/sdd/ht-poh-plan/review-7c40c83c..ae4e6095.diff`: 193 commits, 682 files.
- Review 8 (`code-final-review-8.md`) already reviewed the whole branch at `417c534f`. So in Step 1 I reviewed the reopened work in depth: ht-poh.53, 54, 55 and 56 (`417c534f..ae4e6095`). I also re-checked the branch-wide seams that work touches.

These checks pass at HEAD:
- `cargo fmt --check`.
- `cargo clippy --locked --all-targets --all-features -D warnings`.
- `scripts/check-harness-neutral`: 0 violations, 85 allowlisted, 0 to-migrate.
- A focused nextest run covering pull, notice, `mod_seat_view`, agy delivery and the `golden_surfaces` seed tests: 74 of 74 passed. `check-no-leaked-processes --run-id` was clean afterwards.

## Step 1: my own review

**What checks out:**
- **ht-poh.53 (S1, notices on agy).** It does fix the problem: notices now reach an agy session at the next turn-start pull.
  - The pull reads the notice page without writing. It then settles only if the notices it carries are still exactly the oldest-first prefix (`src/store/attention.rs` `settle_pulled_notices`), under the current binding.
  - So a pull racing a check-in returns false instead of double-delivering. The check-in reads and settles in one writer transaction, so a pull cannot slip in between.
  - `newest_pending_notice` is kept out of `ModFingerprint`, so Claude's watch path is unchanged.
  - A page with no offered id (notices or the attention block only) no longer opens or uses up a probation turn, and does not set `injected_this_turn`. That is correct, because a transcript scan has nothing to confirm on such a page.
- **ht-poh.54 and ht-poh.55.** The fixtures loop over `stage_snapshot_targets` until every target is staged, which is the real root cause of the flakes. The 15 s `WAIT` is restored. The one-syscall write in `save_progress_at` produces the same bytes on disk.

**Findings:**
- **N1 (Minor): pending notices can make a wake fire early.** The pull sets `entry.more_pending = more || notices_left` (`src/service/delivery_pull.rs`). `SeatOffers::next_pull` treats `more_pending` as "this pull delivers something" (`src/delivery/ledger.rs:62`).
  - So when notices are left over, the scheduler's hook-pull gate (`src/scheduler/mod.rs:726`) stops holding back a wake it would otherwise delay. "Left over" means more than one page of notices, a full page, or a refused settle.
  - Example: a nudge was offered 10 s ago, so its 60 s re-offer is not due yet. Normally the wake waits for that re-offer; with leftover notices the idle seat is woken now, and its next pull shows only the notices.
  - That breaks "notices never wake" in spirit. The effect is bounded: at most one early wake per page of leftover notices, and only when a nudge or `wake_on: all` candidate already exists.
  - Fix: track leftover notices separately from `more_pending`, so they only stop the `Unchanged` reply, not the wake gate.
- **N2 (Must fix, doc): the R13 report is stale.** ht-poh.53 is an agy-driven change of about 290 lines outside the adapter, across 6 core source files: `src/ports.rs` (a new `StorePort` trait method with a neutral default), `src/store/{attention,mod,seats}.rs`, `src/service/delivery_pull.rs` and `src/protocol/watch.rs`.
  - `report.md` (written at `9ec7da52`) still says "three small production edits outside the adapter" after the neutral core landed. R13 requires the final report to state this, and it must be updated.
  - In the same doc pass:
    - SKILL line 136 still says notices settle only when "carried by a committed check-in offer"; it should also name the hook-pull page.
    - The `review:` line in `run.md` is stale.
- **N3 (Must fix, evidence): the native gate and live smoke predate the last agy delivery change.** Both ran at `c2f51b51` or earlier.
  - ht-poh.53 then changed `src/harness/agy/delivery.rs` (which step is chosen and when probation starts) and the page contents (new `notice` blocks).
  - The spec runs the smoke "once at the final SHA". The model-free native gate has to be re-run at the final SHA. A live smoke re-run costs about $0.02, and whether to do it is the orchestrator's or the human's call.
- **N4 (Minor): the pull now waits on the writer lock.** When notices are pending, the formerly read-only `DeliveryPull` takes the writer lock to settle, inside the hook's 1.5 s budget.
  - If the writer is busy, the reply can miss the hook's deadline. The messages on that page are then offered but never injected. They come back after 60 s, so this costs latency, not data.
  - The settle also runs outside the `PermitMutation` operation path that check-ins use.
- **N5 (Doc nit): one loss path is not named.** A notice is settled before agy injects it. If agy then aborts the injection (the downgrade trigger), the notice is lost. TRUST-POLICY covers this only through its general "response loss" wording.
- **N6 (Nit):** each new notice moves the pull version, so a pending invitation's attention block is shown again alongside it.

## Step 2: ledger triage

**Recurring clusters:**
- **`[missing-edge-case-test]` ×5 across ht-poh.2.1, 4.3, 5, 50 and 53.** The class is the agy downgrade and probation state machine. Its states are: downgraded, probation, due, confirmable, abort. They are tested one branch at a time, with synthetic fixtures, never as a full table. The newest instance (ht-poh.53: a page with an offered id on a due, downgraded record) is covered by the existing probation tests plus the `confirmable` guard. Not blocking; a table-driven test is the right follow-up.
- **`[doc-formatting]` ×3 (ht-poh.47, 41, 53).** A pipeline smell: no lint wraps the markdown. Not blocking.
- **Earlier clusters.**
  - `[tdd-order]` and `tdd-process` (ht-poh.53 again ran no failing test first): a pipeline defect, not blocking.
  - `[doc-drift]`: the live instance is N2.

**Deferred minors that were misgraded and are must-fix:**
- **ht-poh.55 `[unfixed-sibling-pattern]`** (src/cli/journal.rs). This names a known product defect. The same unbuffered `serde_json::to_writer(&mut file, …)` pattern that ht-poh.55 measured stalling CLI exits under load is still in `src/cli/journal.rs:874`, `:1127` and `:1340`. It exists on main too. Fix it the same way as `ffd09598`, or file a bead with the measurement.
- **The same entry's staging-flake class.** About 11 other fixture files still stage the snapshot once. Main has this too: `tests/service/operator.rs` failed 1 in 20 in the hunt, and the list includes `hook_entrypoint.rs`, `host/bootstrap_recovery.rs`, `store/control.rs` and `identity/reconcile.rs`. It is recorded only in ht-veq's notes and has no bead of its own; file one for the class.
- **ht-poh.54 `[suite-time-over-budget]`.** One full run took 362 s, over the 300 s budget, which AGENTS.md treats as a bug. It is blamed on host load (an earlier run took 272 s). The caller's sweep has to settle it.

**Deferred minors that are OK:**
- **ht-yqj** (`cooperative_overdue_warning_wakes_idle_native_agent_exactly_once`). The head-to-head runs show the flake on main (42 of 360) but not on this branch (0 of 360). The cause is main's 100 ms pacer on the real clock. A bead is filed.
- **ht-poh.54 `[unproven-root-cause]`.** The captured panic came from `launch_and_handoff_reports`, which shares `Topology::seed`, so the fix covers both. One unexplained 180 s hunt timeout is untested scope (see below).
- **`SteppingClock.utc_now`** returns a constant 0; a trap if reused, harmless here.
- **ht-poh.53 swallowed `pull_notice_page` errors.** The retry happens through `more_pending`, which is tied to N1.
- **The overlong TRUST-POLICY line.**

**Parked:** none (parked 0).

**Fix passes:** the one BLOCKED pass, ht-poh.4.1, was a report-evidence gap, not a code defect.

**BLOCKED-AUTH:** Task 39 (ht-poh.13). Its coverage was restored by ht-poh.41, and its worktree has been removed.

**Tracker housekeeping:**
- ht-veq, ht-vrg, ht-2w9 and ht-8zt are still open. Their own notes say to close them when ht-poh.55 merges, and it has merged into the integration branch.
- ht-01s is closed.

## Must fix before landing

1. **The caller's full-suite sweep.**
   - Run `nice cargo nextest run --locked --all-targets --all-features` with `HT_LEAK_RUN_ID` set, then `scripts/check-no-leaked-processes --run-id`.
   - It must finish in 300 s or less on a quiet host. The last measurements were 272 s and 362 s.
   - No full-suite measurement of `ae4e6095` exists.
2. **Re-run the agy native gate at the final SHA** (N3); the live smoke re-run is the orchestrator's or human's call.
3. **Update the R13 accounting in `report.md`** for ht-poh.53's outside-adapter edits, and fix the SKILL line 136 and `run.md` drift (N2).
4. **src/cli/journal.rs:** buffer its three `to_writer` calls, or file a bead with evidence.
5. **File a bead for the single-shot `stage_snapshot_targets` fixture class**, and close or annotate ht-veq, ht-vrg, ht-2w9 and ht-8zt.

## Untested scope

- **No full-suite run of this branch at `ae4e6095`.** The caller runs it after this review, so do not treat the branch as tested.
- **Genuine agy has not run the ht-poh.53 code.** That covers the `notice` blocks, the `confirmable` step and probation choice, and notices arriving while the session is downgraded (`userMessage`).
- **No test covers the N1 interaction:** leftover notices combined with a held hook-pull wake.
- **One 180 s timeout in the ht-poh.54 hunt is unexplained.**
- **ht-yqj stays flaky under load** (it fails on main too).
- **Still never exercised for real (carried from review 8):**
  - the 24 h probation retry;
  - a legacy probation record without `probation_at`;
  - the real `proc_pidpath` identity;
  - Linux `/proc`;
  - `--sandbox`;
  - phase (g) of the wake flow;
  - the doctor `--debug` output change.

## Deferred OK

- **N1:** file a bead to separate leftover notices from `more_pending`. N4, N5 and N6 can wait.
- **S4 from review 8:** downgrade misses collide across conversations; this fails safe.
- **F2 and F3 from earlier reviews:** delivery waits a turn after resume, and any abort marker at `Stop` downgrades even when another plugin caused it.
- **Ledger cluster follow-ups:**
  - a table-driven test of the downgrade and probation state machine;
  - a markdown wrap lint;
  - making implementers capture a failing test first.
- **The remaining items from review 8's Deferred list:**
  - `find_seat_for` ignoring the harness;
  - the stale `wake_work.last_lazy_ordinal` column name;
  - the `downgrade.json` read-modify-write race;
  - a replayed confirm answering `not_offered`;
  - relay and fake-server edge cases;
  - commits missing the `Outside-adapter:` trailer;
  - v1 `HookParseFailure` cannot report agy (frozen wire).

Files:
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/src/service/delivery_pull.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/src/delivery/ledger.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/src/scheduler/mod.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/src/store/attention.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/src/harness/agy/delivery.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/src/cli/journal.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/integrations/skill/SKILL.md
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/docs/superpowers/runs/2026-10-09-agy-harness-adapter/report.md
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/.superpowers/sdd/ht-poh-plan/progress.md
