# Final review 11 — eb0a1eff (after ht-poh.59)

**Verdict: not ready.** The code at `eb0a1eff` is sound, and I found no Critical defect in it. It can't land yet for four reasons: main has moved and the merge now conflicts, the last task added a test of 70 s or more against a suite that had only 13 s of headroom, the genuine-agy evidence and the R13 report predate ht-poh.59, and no full suite has run on this SHA.

What I ran at HEAD `eb0a1eff`:
- Fork point `7c40c83c`. Review package: `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/.superpowers/sdd/ht-poh-plan/review-7c40c83c..eb0a1eff.diff` (205 commits, 768 files).
- clippy with `--all-features -D warnings`: clean.
- `cargo fmt --check`: clean.
- `scripts/check-default-features`: exit 0.
- `scripts/check-harness-neutral`: 0 violations, 85 allowlisted, 0 to-migrate.
- A focused nextest run (delivery_pull, ledger, hook_pull, notice, agy): 293/293 passed. The leak check afterwards with `--run-id` was clean.

Final review 8 already reviewed the whole branch, and reviews 9 and 10 plus the scoped audit covered `417c534f..9af7ae9f`. Their must-fixes have landed: ht-poh.57 and ht-poh.58 merged, the SKILL wording was fixed, the R13 numbers were updated at 9af7ae9f, the gate and smoke re-ran at 9af7ae9f, and the sweep at 9af7ae9f passed 4926/4926 in 287 s. So I looked hardest at what came after that (ht-poh.59), and re-checked the seams it touches: the scheduler's hook-pull gate, the wake text the dispatcher chooses, notice settling and page bounds.

## STEP 1: my own findings

**S1. ht-poh.59 is correct for what it fixes.**
- `record()` no longer evicts, so a pending id is never offered afresh and the "3 offers, 60 s apart" bound holds.
- The scheduler's `NextPull::Nothing` plus `holds_parked` and the dispatcher's `holds_parked → MARKER` both treat a full table as "parked", so the idle wake names `inbox`.
- `withheld && !is_full()` re-opens the pull once a whole read has freed slots.
- Binding generations only go up (`seats.generation` is incremented by compare-and-swap, never lowered), so the new `seat_mut` returning `None` for an older generation can't strand a seat.
- The confirm path handles `None`.

**S2. Every pull on a full table walks the whole pending set (`src/service/delivery_pull.rs` ~334–357).**
- Ids withheld at capacity return `Continue`, so the drain no longer stops after collecting a page. Each pull that isn't answered `Unchanged` (attention version moved, an offer due, `more_pending` or `notices_pending`) reads every pending item, about 8 truncated items per store page.
- This is why the new test is slow: `a_truncated_backlog_over_the_offer_table_keeps_the_reoffer_bounds_and_parks_for_inbox` took 69.5 s alone and 76 s alongside the other focused tests here. The ledger records 72–96 s alone and 147 s under load.
- For the product the effect is bounded. A very large backlog overruns the hook's 1.5 s budget, the store page read fails and the pull is refused `Busy`. The ledger and notices then wait. The seat still fails toward `inbox`, because `holds_parked` makes the wake name it.

**S3. The 59 "notices ride every page" path can settle a notice and then refuse the reply.**
- `settle_pulled_notices` commits before the second `seat_mut` (~440). If a newer generation's entry appeared in between, the reply is refused `Busy` and the settled notice is never shown.
- This is a new loss window. AGENTS.md requires TRUST-POLICY to be updated in the same commit, and it wasn't. The sibling paths (hook deadline, aborted injection) are tracked in ht-7jh.

**S4. The attention block sits outside the page bounds (nit, existed before 59).**
- With no notices pending, the drain can fill 32 message blocks and then append the attention block, giving 33 blocks.
- With notices pending, the attention block's bytes are not counted against `PAGE_MAX_TEXT_BYTES`.
- The overrun is about 100 bytes.

**S5. Base drift: main is now `bcb49485` (v0.6.4).**
- `git merge-tree main HEAD` reports **CONFLICT in `integrations/claude/mod/README.md`**.
- `register.js` and `tests/delivery.test.ts` auto-merge with main's `f674333f` (the mod's log line drops its `herdr-threads:` prefix). This is the same file the R17/R18 re-fit changed, so the mod tests have to pass on the merged result.
- `report.md` still says the merge is "conflict-free".

**S6. The R13 report and run state are stale after ht-poh.59.**
- 59 changed `src/delivery/ledger.rs`, `src/service/delivery_pull.rs` (+165/−89 together), `src/ports.rs` and `TRUST-POLICY.md`: agy-driven HookPull core again.
- `report.md` still says "final code `9af7ae9f`", "status: completed" and "qualifier: none", and its R13 "reopened pass" line stops at ht-poh.57.
- `run.md` is at `phase: fix-loop`, and its `review:` line still says ready.

**S7. The genuine-agy evidence predates ht-poh.59.**
- The native gate ran at `9af7ae9f`. The smoke ran at `2bf61304`, whose code is the same as `9af7ae9f`.
- 59 changes what agy's injected page contains: notices now ride backlog pages, a backlog page holds one fewer message block, and a full table injects nothing new.
- The adapter source is unchanged, but the daemon-side page is not.

## STEP 2: ledger triage

**Recurring clusters.** The task lists are the latest cluster line for each signature.

| Cluster | Count and tasks | Class | Blocking? |
|---|---|---|---|
| `[missing-edge-case-test]` | ×5 (2.1, 4.3, 5, 50, 53) | Stateful machinery (setup-manifest phases, downgrade/probation record, real-parent identity) tested one branch at a time with synthetic fixtures, never as a state table. | No. Follow-up is a table-driven probation/downgrade test. |
| `[tdd-order]` / `tdd-process` | ×3 (6.3, 3.2, 2.3); 53 again | Pipeline defect: no failing test captured first. | No |
| `[doc-formatting]` | ×3 (47, 41, 53) | Nothing wraps or lints the markdown. | No |
| `[doc-drift]` | ×3 (7, 4.8, 6.9) | Docs not kept in step with shipped behaviour. The live instance is S6. | No |

**ht-poh.59 minors:**
- **`[slow-test]`: misgraded, must-fix.** The last sweep had 13 s of headroom (287 s against the 300 s budget). A test that takes 70–147 s threatens that budget, and AGENTS.md treats going over budget as a bug. The root cause is S2.
- **`[race-window]`: misgraded, must-fix.** It names a data-loss path (S3). The minimum fix is to record it in TRUST-POLICY next to ht-7jh's siblings. Better: return the settled notice blocks instead of `Busy`.
- **`[imprecise-predicate]` (`holds_parked` is true at exactly 256 with nothing withheld): deferred OK.** Naming `inbox` while 256 unconfirmed ids are pending is harmless, and the scheduler only reaches `holds_parked` once `next_pull` returns `Nothing`.

**Earlier deferred minors:**
- Reviews 9 and 10 already triaged them, and the must-fix ones (journal.rs, notices waking the seat) landed in 57 and 58.
- ht-poh.54 `[suite-time-over-budget]` is still open. Only the caller's sweep can settle it, and S2 makes it worse.

**Other ledger lines:**
- **Parked:** 0.
- **Fix-pass BLOCKED (ht-poh.4.1):** missing evidence in the task report, resolved.
- **BLOCKED-AUTH, Task 39 (ht-poh.13):** coverage came back through ht-poh.41 after the human ran the generator.
- **Task 71 BLOCKED (ht-poh.59):** blocked on authorization, not a code problem. The task was later completed under the relayed request "implement ht-poh.59".
- **Housekeeping:** ht-vrg, ht-2w9 and ht-8zt are still open, although their own notes say to close them once ht-poh.55 lands (it has).

**Metrics:** `ledger-check ok`, merges 67 = review clean 65 + after fix pass 2. Neither invalid marker appears.

## Verdict
Not ready.

## Must fix before landing
1. **Absorb main `bcb49485` (v0.6.4).**
   - Resolve the `integrations/claude/mod/README.md` conflict.
   - Run `scripts/test-claude-mod` on the merged `register.js`/`delivery.test.ts`, plus clippy, `check-default-features` and `check-harness-neutral`.
2. **Make `a_truncated_backlog_over_the_offer_table_keeps_the_reoffer_bounds_and_parks_for_inbox` cheap (S2).**
   - Options: fewer pulls, a smaller backlog that still overfills the table, or a pull that stops walking once the table is full and nothing is due (keeping some way to free slots).
   - A nextest priority alone does not fix it.
3. **Record S3's pull-settled-notice loss on rotation in TRUST-POLICY, or fix it.** Recording means listing it with ht-7jh's deadline and abort siblings under "response loss". Fixing means returning the settled notices instead of `Busy`.
4. **Update `report.md` and `run.md` for ht-poh.59 (S6).**
   - R13 outside-adapter lines for 59's core edits.
   - The final SHA, status and qualifier.
   - The merge-with-main statement.
5. **Re-run the agy native gate at the final SHA (S7).** Whether to re-run the live smoke (about $0.03) is the human's call.
6. **The caller's full-suite sweep at the final merged SHA.**
   - Run it with `HT_LEAK_RUN_ID`, then `check-no-leaked-processes --run-id`.
   - It must finish in 300 s or less on a quiet host.
   - **No full-suite measurement of this branch exists at `eb0a1eff` or later.** The last one was 287 s at `9af7ae9f`, before the 70 s+ test.

## Untested scope
- **The branch as a whole:** no full suite at HEAD or at the post-absorb SHA.
- **ht-poh.59 against genuine agy:**
  - notice blocks riding a message backlog page;
  - full-table parking, and a wake that names `inbox`;
  - the generation-monotonic `Busy` refusal.
- **A full-table pull on a large backlog under the real 1.5 s hook budget** (the S2 product path).
- **The main absorb:** the merged Claude mod `register.js`.
- **Carried over from review 10, never run for real:** the 24 h probation retry, a legacy probation record, real `proc_pidpath` identity, Linux `/proc`, `--sandbox`, and wake-flow phase (g).
- **Unexplained:** a 180 s timeout in the ht-poh.54 flake hunt.

## Deferred OK
- ht-poh.59 `[imprecise-predicate]`.
- S4: the attention block outside the page bounds.
- The 59 comment "such an id is not shown again" in the case where a concurrent pull filled the slots. The id is shown once without being recorded, which costs at most one duplicate showing.
- **Review 10's deferred list:**
  - notice loss on deadline or abort, and writer-lock latency (ht-7jh);
  - the "informational, no ACK" label on waking notices;
  - no pull on the resume turn's inv0;
  - the `SteppingClock` stub;
  - the overlong TRUST-POLICY line.
- **Review 8's deferred items:**
  - downgrade misses collide across conversations;
  - an abort marker from another plugin downgrades too;
  - `find_seat_for` ignores the harness;
  - the stale `last_lazy_ordinal` column name;
  - the `downgrade.json` read-modify-write race;
  - a replayed confirm answers `not_offered`;
  - relay and fake-server edge cases;
  - commits missing the `Outside-adapter:` trailer;
  - the frozen v1 `HookParseFailure` shape.
- **Ledger follow-ups:** the table-driven probation/downgrade test, a markdown wrap lint, red-first TDD capture, ht-86a (fixture staging), ht-yqj (flake also seen on main), and ht-10p.
- **Tracker housekeeping:** close ht-vrg, ht-2w9 and ht-8zt.

Files:
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/src/service/delivery_pull.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/src/delivery/ledger.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/tests/service/delivery_pull.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/TRUST-POLICY.md
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/docs/superpowers/runs/2026-10-09-agy-harness-adapter/report.md
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/docs/superpowers/runs/2026-10-09-agy-harness-adapter/run.md
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/integrations/claude/mod/README.md
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/.superpowers/sdd/ht-poh-plan/progress.md
