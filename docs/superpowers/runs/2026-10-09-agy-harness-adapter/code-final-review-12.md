# Final review 12 — 00db9827

**Final whole-epic review of ht-poh: `super-auto/agy-harness-adapter` @ `00db9827`**

Fork point is `945107f6`, which is main's current tip, so main v0.6.4 has been absorbed. Review package: `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/.superpowers/sdd/ht-poh-plan/review-945107f6..00db9827.diff` (209 commits). Spec: `docs/superpowers/runs/2026-10-09-agy-harness-adapter/2026-10-09-agy-harness-adapter-design.md` (R1–R13, R17, R18).

Checks I ran at HEAD:
- **clippy** (`--all-features -D warnings`): clean.
- **`cargo fmt --check`**: clean.
- **`scripts/check-default-features`**: exit 0.
- **`scripts/check-harness-neutral`**: 0 violations, 85 allowlisted, 0 to-migrate.
- **`scripts/test-claude-mod`**: 95/95 on the merged `register.js`.
- **Focused nextest** (delivery_pull, ledger, hook_pull, notice, agy): 294/294 passed in 44 s. The leak check afterwards with `--run-id` was clean.

Final reviews 8–11 already covered the whole branch through `eb0a1eff`. So I looked hardest at what changed since then: the main absorb `d9c19759` and ht-poh.60 `92e8e1aa`. I also re-checked the R6 doctor items against `src/harness/agy/doctor.rs`.

## STEP 1: my own findings

**F1. The main absorb is correct.**
- The README conflict kept R18's "append then submit" text and added main's log-line note.
- `delivery.test.ts` now expects main's unprefixed `delivered N message(s)`.
- Main's thread-search changes in `src/store/queries.rs` and `src/cli/commands.rs` add no harness names, and the neutral check passes.

**F2. ht-poh.60's fix for review 11's S2 (a full-table pull walking the whole backlog) is sound.**
- The new `whole_read` value is keyed on the pending set's version without notices. That version folds in the pending counts and the highest pending ordinals. So an arrival or a settle elsewhere moves it and forces a whole read.
- A settle that races the read leaves `whole_read` stale, which only costs one extra whole read.
- After a whole read, the offered ids are a subset of the pending set, so stopping once every due offer has been met cannot skip a due id.
- A pull that stops short sets `read_attention` but never calls `retain_pending` or advances `whole_read`.
- Ids already shown 3 times are never due, so a table full of them skips the drain until the pending set moves. That is ht-poh.59's intended parking for `inbox`.
- The renamed test took 15.2 s under focused-suite load. The commit says 1.9 s, which was measured alone. It no longer threatens the budget.

**F3. ht-poh.60's fix for S3 (settled notices lost to a rotation race) is sound.**
- When a rotation lands after the pull has settled notices, the pull now returns a page holding only those notices, with no offered or truncated ids. With nothing settled it still answers `Busy`.
- It adds no new loss path. The general TRUST-POLICY A7 wording ("response loss can lose an informational offer after commitment") covers the remaining reply-loss case.

**F4. One notice-loss path is still unrecorded in TRUST-POLICY (ht-7jh, open P3).**
- On a downgraded agy seat in its probation turn, a pull settles notices and then injects a `systemMessage` step that may silently miss.
- A miss re-offers the messages, but the notices are already gone. A page with only notices can never count as a miss, because `is_miss` scans only `page.offered`.
- This is a silent injection miss, not response loss, so the A7 sentence does not cover it. AGENTS.md requires an accepted limit to be written into TRUST-POLICY in the same change.

**F5. The run records are stale again (the same doc-drift class as before).**
- `report.md` still says "final code `9af7ae9f`", "qualifier: none" and "merge-base `7c40c83c`", and that a probe merge with main `6107075f` was conflict-free.
- Its R13 accounting has nothing for ht-poh.59's or ht-poh.60's core edits (`src/delivery/ledger.rs`, `src/service/delivery_pull.rs`, `src/ports.rs`, `TRUST-POLICY.md`), and nothing for the absorb's mod edits.
- `run.md` is still at `phase: fix-loop`.

**F6. The genuine-agy evidence predates ht-poh.59 and ht-poh.60.**
- The native gate last ran at `9af7ae9f`, and the smoke at `2bf61304` (the same code).
- `src/harness/agy/**` has not changed since then. But the daemon-side page agy injects has: notices now ride backlog pages, a full table parks for `inbox`, a full-table pull stops at its due offers, and a lost race returns a page with only notices.

**F7. Suite budget risk.**
- Two adapter tests took about 39–41 s each under focused load: `harness::agy::doctor::tests::an_over_cap_brain_is_unbounded_and_limited` and `harness::agy::witness::tests::a_brain_directory_past_the_cap_reads_as_a_child_and_one_at_the_cap_is_examined`.
- The ledger recorded 7–10 s for these (ht-poh.4.3 `[slow-test]`).
- Together with ht-poh.54's 362 s under-load run and the last clean sweep of 287 s, the 300 s budget is not demonstrated at this SHA.

## STEP 2: ledger triage

**Recurring clusters.** None is a blocker on its own.
- **`[missing-edge-case-test]` ×5** (ht-poh.2.1, 4.3, 5, 50, 53): stateful machinery tested one branch at a time with synthetic fixtures, never as a state table. The setup phases, the downgrade/probation record and the real-parent identity are the instances. Follow-up: one table-driven probation/downgrade test.
- **`[tdd-order]` ×3** (ht-poh.6.3, 3.2, 2.3): a pipeline defect. No failing test is captured before the fix.
- **`[doc-formatting]` ×3** (ht-poh.47, 41, 53): nothing wraps or lints the markdown.
- **`[doc-drift]` ×3** (ht-poh.7, 4.8, 6.9): docs are not kept in step with shipped behaviour. F5 is the live instance and is must-fix.

**ht-poh.60 minors.** Both deferred OK.
- `[missing-explanatory-comment]`: the page with only notices reports `more:false`. This is harmless: `reply_version` keeps the newest notice, and a new generation's entry always reads.
- `[partial-assertion-coverage]`: the backlog test checks the items-read bound only on one pull.

**ht-poh.59 minors.**
- `[slow-test]` and `[race-window]` were misgraded as minor. Both are now resolved by ht-poh.60 (F2, F3).
- `[imprecise-predicate]` is deferred OK.

**Earlier minors.**
- Task 17 `[skipped-verification]` (the Claude mod tests were not run): resolved, I ran them (95/95).
- Task 40 `[doc-comment-garbled]`: fixed in tree.
- Task 31 `[unsynchronised-state-write]` (`downgrade.json`): deferred, as reviews 8–11 decided.

**Other ledger lines.**
- **Parked:** 0.
- **Fix-pass BLOCKED:** 1 (ht-poh.4.1, missing evidence in the task report), resolved.
- **Task 71 (ht-poh.59) BLOCKED:** an authorization stop, not a code problem. It later completed and was reviewed clean.
- **BLOCKED-AUTH (Task 39, ht-poh.13):** coverage was restored through ht-poh.41 after the human ran the generator.
- **Metrics:** `ledger-check ok`, merges 68 = review clean 66 + after fix pass 2. Neither invalid marker appears.

## Verdict
**Not ready.** I found no Critical code defect, and review 11's code must-fixes (S2, S3, the main absorb) have landed correctly. Landing is blocked by stale run records, genuine-agy evidence that predates the last two core changes, an unrecorded trust limit, and no full-suite measurement of this branch: **none exists at `eb0a1eff` or later, including HEAD `00db9827`.** The caller's sweep has not run yet, so the branch should not be treated as tested.

## Must fix before landing
1. **Record F4 (ht-7jh) in TRUST-POLICY, or fix it.** Fixing means injecting a mixed page as `userMessage` during probation. Recording means one accepted-limit sentence next to A7's response-loss limit.
2. **Update `report.md` and `run.md` (F5):**
   - the final SHA, status and qualifier;
   - the merge-base `945107f6` and the absorb conflict;
   - R13 outside-adapter lines for ht-poh.59, ht-poh.60 and the absorb;
   - the run phase.
3. **Re-run the agy native gate at the final SHA (F6).** Re-running the live smoke (about $0.03) is the human's call.
4. **The caller's full-suite sweep at the final SHA.**
   - Export `HT_LEAK_RUN_ID` first, then run `scripts/check-no-leaked-processes --run-id`.
   - It must finish in 300 s or less on a quiet host. Watch the two 40 s agy over-cap tests (F7).

## Untested scope
- **The branch as a whole:** no full suite at `eb0a1eff` or later. The last sweep was 4926/4926 in 287 s at `9af7ae9f`.
- **ht-poh.59 and ht-poh.60 against genuine agy:**
  - notices riding a backlog page;
  - a full-table park and a wake that names `inbox`;
  - a short full-table pull;
  - the page with only notices after a lost rotation race.
- **A full-table pull on a large backlog under the real 1.5 s hook budget.** The reply was not measured end to end.
- **Carried over from review 10, never run for real:** the 24 h probation retry, a legacy probation record, real `proc_pidpath` identity, Linux `/proc`, `--sandbox`, and wake-flow phase (g).
- **The ht-poh.54 flake hunt's 180 s timeout:** still unexplained.

## Deferred OK
- The ht-poh.60 minors (the `more:false` comment, partial assertion coverage) and ht-poh.59 `[imprecise-predicate]`.
- **Review 11:** the attention block outside the page bounds; one possible duplicate showing after a concurrent pull fills the slots.
- **Review 10:**
  - writer-lock latency;
  - the "informational, no ACK" label on waking notices;
  - no pull on the resume turn's inv0;
  - the `SteppingClock` stub;
  - the overlong TRUST-POLICY line.
- **Review 8:**
  - downgrade misses colliding across conversations;
  - an abort marker from another plugin also triggering a downgrade;
  - `find_seat_for` ignoring the harness;
  - the stale `last_lazy_ordinal` column name;
  - the `downgrade.json` read-modify-write race;
  - a replayed confirm answering `not_offered`;
  - relay and fake-server edge cases;
  - commits missing the `Outside-adapter:` trailer;
  - the frozen v1 `HookParseFailure` shape.
- **Cluster follow-ups:** the table-driven probation test, a markdown wrap lint, and capturing a failing test before each fix.
- **Tracker housekeeping:** ht-vrg, ht-2w9 and ht-8zt are still open although ht-poh.55 has landed. Close them once the sweep confirms.

Files:
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/src/service/delivery_pull.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/src/delivery/ledger.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/src/harness/agy/delivery.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/TRUST-POLICY.md
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/docs/superpowers/runs/2026-10-09-agy-harness-adapter/report.md
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/docs/superpowers/runs/2026-10-09-agy-harness-adapter/run.md
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/docs/evidence/agy-native-gate/
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/.superpowers/sdd/ht-poh-plan/progress.md
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/.superpowers/sdd/ht-poh-plan/review-945107f6..00db9827.diff
