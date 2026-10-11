# Final review 10 — ae4e6095 (57/58 unmerged)

**Final review: ht-poh at `ae4e6095`** (fork point `7c40c83c`, package `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/.superpowers/sdd/ht-poh-plan/review-7c40c83c..ae4e6095.diff`: 193 commits, 682 files)

Checks I ran at HEAD:
- clippy with `-D warnings`: clean.
- `cargo fmt --check`: clean.
- `scripts/check-harness-neutral`: 0 violations, 85 allowlisted, 0 to-migrate.
- Focused nextest over the reopened scope (pull, notice, attention, `mod_seat_view`, `protocol::watch`, agy delivery, hook_entrypoint, `golden_surfaces`, `handoff_topology_cli`, `topology_activation`, `handoff_agy`): 154 of 154 passed.
- `check-no-leaked-processes --run-id`: clean.

Review 8 covered the whole branch at `417c534f`. I reviewed the reopened work (ht-poh.53–56) in depth, re-checked the seams it touches (scheduler hook-pull gate, agy step and probation, notice frontier, witness lifecycle), and spot-checked the spec's R1–R18 wiring.

## Step 1: my own findings
- **A. Leftover notices pull a wake forward (src/service/delivery_pull.rs ~397, src/delivery/ledger.rs:62, src/scheduler/mod.rs:726).** I confirmed this independently.
  - The pull sets `entry.more_pending = more || notices_left`, and `SeatOffers::next_pull` returns `Delivers` whenever `more_pending` is true.
  - So when notices are left over (a full notice page, a refused settle, or a failed `pull_notice_page`), the scheduler stops holding a wake whose 60 s re-offer is not yet due. That breaks the "notices never wake" rule in the AGENTS.md delivery contract.
  - The effect is bounded: it needs an existing nudge or `wake_on: all` candidate.
  - The fix is already built on `task-ht-poh.57` (8aa6c95c, adds a separate `notices_pending` flag) but is not merged.
- **B. Notices settle before the reply.**
  - When the hook's 1.5 s deadline runs out, or agy aborts the injection, a notice settled by the pull is lost. TRUST-POLICY covers this only under the general "response loss" limit.
  - The pull also takes the writer lock when notices are pending, so a busy writer can push the reply past the hook deadline. The messages on that page then come back after 60 s, so this costs latency, not data.
- **C. A waking notice is labelled "informational, no ACK".** The affected seat's own open overdue-condition warning is a waking notice. A pull carries it as a `notice` block labelled "informational, no ACK" and settles it through the frontier. Check-ins do the same, so this is parity, and the underlying message keeps its own receipt. Only the "another seat" case is tested.
- **D. No delivery on the resume turn.** The inv0 PreInvocation after a resume (or a re-checked `/clear`) is a lifecycle event, so it runs the check-in and not the pull step. Bodies wait for invocation 1 or the next turn. Review 9 deferred this as F2.
- **E. R13 drift.**
  - ht-poh.53 adds about 290 lines across 6 core files: `ports.rs`, `store/{attention,mod,seats}.rs`, `service/delivery_pull.rs` and `protocol/watch.rs`.
  - `report.md:11` still says "three small production edits outside the adapter".
  - `integrations/skill/SKILL.md:136` still says notices settle only when "carried by a committed check-in offer".
  - The `review:` line in `run.md` is stale.
- **F. Genuine-agy evidence predates ht-poh.53.**
  - The native gate ran at `c2f51b51` and the live smoke at `4d2ec663`.
  - ht-poh.53 then changed `src/harness/agy/delivery.rs` (which step is chosen, when probation opens) and the page contents (`notice` blocks).
- **G. Pipeline seam: the review 9 file is blocking merges.** The ledger shows the ht-poh.58 merge "held: integration worktree dirty" because of the untracked `docs/superpowers/runs/2026-10-09-agy-harness-adapter/code-final-review-9.md`. ht-poh.57 and ht-poh.58 are finished on their task branches and cannot land until that file is committed or moved.

## Step 2: ledger triage
- **Recurring `[missing-edge-case-test]` ×5 (2.1, 4.3, 5, 50, 53).**
  - Class: rare-state branches of stateful machinery are tested branch by branch with synthetic fixtures, never as a table or against the real environment. The instances are setup-manifest interruption phases, the downgrade/probation record (legacy record, due-and-downgraded page) and real-parent identity.
  - 4.3's instance was later closed by ht-poh.47.
  - Not blocking. The follow-up is a table-driven probation/downgrade test.
- **Recurring `[doc-formatting]` ×3 and `[doc-drift]` ×3.** Pipeline smell: nothing wraps or lints the markdown. The live `[doc-drift]` instance is E.
- **`[tdd-order]` and `tdd-process` across several tasks.** A pipeline defect: implementers capture no failing test first. Not blocking.
- **ht-poh.55 `[unfixed-sibling-pattern]` was misgraded; it is a known product defect.**
  - `src/cli/journal.rs` still writes JSON with unbuffered `to_writer` at three sites, the same pattern ht-poh.55 measured stalling CLI exits under load.
  - Must fix. The fix is ht-poh.58 (6f238868), built but not merged.
  - The single-shot fixture-staging class from the same entry is test-only and filed as ht-86a, so deferring it is OK.
- **ht-poh.54 `[suite-time-over-budget]` (362 s against a 300 s budget).** AGENTS.md treats going over budget as a bug. Only the caller's sweep can settle it.
- **ht-poh.55 `[residual-flake]` ht-yqj.**
  - The test is flaky, not skipped. Review 9 reports it fails on main and not on this branch (I could not verify that), and it is filed.
  - Deferring it is OK.
- **Parked:** 0.
- **Fix-pass BLOCKED (ht-poh.4.1):** the problem was missing evidence in the task report, not a code defect, and it was resolved.
- **BLOCKED-AUTH, task 39 (ht-poh.13):** the frame-goldens run was refused. Coverage came back through ht-poh.41 after the human ran the generator.
- **Housekeeping:** the flake beads ht-veq, ht-vrg, ht-2w9 and ht-8zt are still open, although their notes say to close them once ht-poh.55 lands.

## Verdict
Not ready.

## Must fix before landing
1. Commit or move the untracked `code-final-review-9.md`, then merge `task-ht-poh.57` (finding A) and `task-ht-poh.58` (journal.rs).
2. Run the full-suite sweep at the final SHA, after those merges:
   - `nice cargo nextest run --locked --all-targets --all-features` with `HT_LEAK_RUN_ID` set, then `scripts/check-no-leaked-processes --run-id`;
   - it must finish within 300 s on a quiet host.
   - No full-suite measurement of this branch exists. The last ones were 272 s at `417c534f` and 362 s under load.
3. Re-run the agy native gate at the final SHA (finding F). Whether to re-run the live smoke (about $0.02) is the human's call.
4. Update the R13 accounting in `report.md` for ht-poh.53's core edits, and fix `SKILL.md:136` and the `run.md` review line (finding E).
5. Close or annotate ht-veq, ht-vrg, ht-2w9 and ht-8zt.

## Untested scope
- **The branch as a whole.** No full suite has run at `ae4e6095` or at the final SHA, so do not treat the branch as tested.
- **ht-poh.53 on genuine agy:**
  - `notice` blocks;
  - the `confirmable` step choice and probation;
  - notices arriving while the session is downgraded to `userMessage`.
- **Finding A's interaction:** no test at HEAD (one exists only on the unmerged `task-ht-poh.57`).
- **A pull whose writer-lock settle misses the hook deadline.**
- **The affected-seat waking notice carried by a pull (finding C).**
- **Never run for real:**
  - the 24 h probation retry;
  - a legacy probation record;
  - real `proc_pidpath` identity;
  - Linux `/proc`;
  - `--sandbox`;
  - wake-flow phase (g).
- **Unexplained:** one 180 s timeout in the ht-poh.54 hunt.

## Deferred OK
- **Smaller notice findings:**
  - B (loss on abort or deadline; writer-lock latency): document the abort path explicitly in TRUST-POLICY later;
  - C (the label on waking notices);
  - D / F2 (no pull on the resume inv0).
- **Ledger follow-ups:** the table-driven probation/downgrade test, a markdown wrap lint, red-first TDD capture, fixture staging (ht-86a), and ht-yqj.
- **ht-poh.54 `SteppingClock` stub:** returns a constant 0.
- **ht-poh.53 swallowed notice errors:** already logged on `task-ht-poh.57`.
- **The overlong TRUST-POLICY line.**
- **Review 8's deferred list:**
  - downgrade misses collide across conversations, which fails safe;
  - any abort marker at `Stop` downgrades, even when another plugin caused it;
  - `find_seat_for` ignores the harness;
  - the stale `last_lazy_ordinal` column name;
  - the `downgrade.json` read-modify-write race;
  - a replayed confirm answers `not_offered`;
  - relay and fake-server edge cases;
  - commits missing the `Outside-adapter:` trailer;
  - the v1 `HookParseFailure` wire shape cannot report agy (frozen).

Relevant files:
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/src/service/delivery_pull.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/src/delivery/ledger.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/src/scheduler/mod.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/src/store/attention.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/src/harness/agy/delivery.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/src/harness/agy/witness.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/src/cli/journal.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/integrations/skill/SKILL.md
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/docs/superpowers/runs/2026-10-09-agy-harness-adapter/report.md
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/docs/superpowers/runs/2026-10-09-agy-harness-adapter/code-final-review-9.md
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/.superpowers/sdd/ht-poh-plan/progress.md
