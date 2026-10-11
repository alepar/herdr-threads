# Final review 13 — 38c5fef6

## Verdict: not ready

Final whole-epic review of `super-auto/agy-harness-adapter` at `38c5fef6`. The fork point is `478ec4a9`, which is main's v0.7.0 tip.

**No full-suite run exists for this branch at `eb0a1eff` or later, including HEAD.** The last sweep was 4926/4926 in 287 s at `9af7ae9f`. The caller's sweep is still to come, so this branch should not be treated as tested.

I found no Critical or Important code defect. What blocks landing is evidence and records that are older than the code, not the code itself.

What I ran at HEAD (no files edited, no commits):
- **clippy** (`--all-features -D warnings`): clean.
- **`scripts/check-harness-neutral`**: 0 violations, 85 allowlisted, 0 to-migrate. It is wired into CI at `ci.yml:197`.
- **`scripts/test-claude-mod`** under an isolated config: 95/95.
- **Focused nextest** (delivery_pull, ledger, hook_pull, topology_handoff, agy, wake_on, schema): 582/582 in 29.7 s. The two agy over-cap tests took 15.4 s and 13.2 s. The leak check with `--run-id` was clean.

### Step 1: my own findings, before reading the ledger

- **S1. ht-poh.61 (automatic new-tab refusal) is sound.**
  - The refusal compares `compose_bootstrap_v2_argv(caller)` with the caller argv after preflight. Preflight only reads.
  - agy composes to the identity, and the end-to-end automatic test covers it with `--prompt-interactive`.
  - Hermes and synthetic_fourth are refused and pointed at the explicit V2 route. Hermes V2 is covered at preparation only (`v2_new_tab_hermes_places_the_prompt_with_query`).
- **S2. The main v0.7.0 merge seam is correct.**
  - Main's harness literals in the automatic producer now go through the registry `bootstrap_target`, and the neutral check is green.
  - The schema renumber is consistent: main's V29 and V30 run first, then `wake_on` as V31.
  - One edge: a database created by a pre-merge branch build is stamped 29 and already has `seats.wake_on`. After this merge it fails closed with `IncompatibleSchema` (`verify_existing_v29` looks for `direct_threads`). That is safe, but such a database would need repair. Every gate and test database I know of was isolated.
- **S3. The ht-poh.59/.60 offer-ledger and pull logic composes correctly with the scheduler.**
  - The scheduler's `HookPullOutlook` reads the same `wake_work.attention_version` counter that `mod_seat_view` uses.
  - `seat_mut` never goes back to an older generation.
  - The full-table short read can only stop after every due offer has been met.
  - Leftover page bound (minor, also raised by review 11): with no notice on the page, the attention block is not reserved. A page can carry 32 message blocks plus the attention block, and can pass 64 KiB by about 100 bytes.
- **S4. The run records are stale.** `report.md` still names final code `9af7ae9f` and merge-base `7c40c83c`, and `run.md` is still at `phase: fix-loop`.
  - The R13 accounting has no lines for the core edits in ht-poh.59 and .60 (`src/delivery/ledger.rs`, `src/service/delivery_pull.rs`, `src/ports.rs`) or ht-poh.61 (`src/cli/topology_handoff.rs`), and nothing for the v0.7.0 merge's semantic follow-ups.
  - `outside-adapter-report.md` still measures `f3431b31..1eaf635b`.
  - Run docs still cite `migrations/0029_seat_wake_on.sql`, which is now `0031`.
- **S5. The genuine-agy evidence is older than the code.** The native gate last ran at `9af7ae9f` and the live smoke at `2bf61304`. Since then the daemon-side page that agy injects has changed (.59, .60), the v0.7.0 merge landed, and .61 landed.
- **S6. Final review 12's items:**
  - F4 is closed: `7d3218a5` records the limit in TRUST-POLICY.
  - Its must-fix items 2 (stale run records), 3 (native gate re-run) and 4 (full suite) are still open.

### Step 2: ledger triage

**Recurring clusters**, named by class:
- **`[missing-edge-case-test]` ×5** (ht-poh.2.1, 4.3, 5, 50, 53): stateful machinery (setup phases, the downgrade/probation record, the real-parent identity) is tested one branch at a time with synthetic fixtures, never as a state table. This is a test-design smell, not a blocker.
- **`[tdd-order]` ×3**, plus the same issue under about 15 other spellings (`missing-red-run`, `no-red-run`, `tdd-process` and others) across nearly every task: no failing test was captured before the fix. This is a pipeline defect in the run, not a product defect.
- **`[doc-drift]` ×3** (ht-poh.7, 4.8, 6.9): docs are not kept in step with shipped behaviour. S4 is the live instance, and it is must-fix.
- **`[doc-formatting]` ×3** (ht-poh.47, 41, 53): nothing wraps or lints the markdown. Cosmetic.

**Parked lines:** none. The fix-pass BLOCKED on ht-poh.4.1 (missing evidence in a report) was resolved later.

**BLOCKED lines that were not code defects:**
- ht-poh.4.5 was blocked on the agy binary, later resolved with a genuine 1.3.2 binary.
- ht-poh.28 was blocked on the resume witness defect, later fixed by ht-poh.30.
- ht-poh.59 stopped for lack of user authorization, then completed and reviewed clean.

**BLOCKED-AUTH, ht-poh.13 (frame goldens and mod tests):** the human then ran the goldens generator (ht-poh.19 gate, ht-poh.41). But R17's `register.js` and `delivery.test.ts` edits had never been run through the mod's TypeScript test runner, and CI does not run it. I ran it at HEAD: 95/95. This gap is closed for now, but it is not guarded in CI.

**Minors that are really must-fix (data loss, product defect or test gap):**
- ht-poh.59 `[race-window]` (settled notices lost to a rotation race) and `[slow-test]`: both already fixed by ht-poh.60.
- ht-poh.4.8 `[core-defect-followup]` (`handoff --new-tab --kind agy` failed): fixed (ht-poh.22, .61). The tests pass at HEAD.
- ht-poh.6.6 `[real-home-incident]` (setup once run against the real HOME): I checked, and there is no `~/.gemini` at all, so nothing was left behind.
- ht-poh.17 `[open-divergence]` DC-3: resolved by ht-poh.42.
- ht-poh.16 `[doc-comment-garbled]`: fixed in the tree (review 12).
- ht-poh.6.2 `[unseeded-pin-manifest]`: 1.3.2 is now pinned.
- ht-poh.55 `[residual-flake]` `cooperative_overdue_warning_wakes_idle_native_agent_exactly_once` (ht-yqj) and ht-poh.54 `[suite-time-over-budget]` (362 s under load): these are not code fixes, but the final sweep must show whether they hold.

## Must fix before landing
1. **Update `report.md`, `run.md` and the R13 accounting.**
   - The final SHA `38c5fef6` and the merge-base `478ec4a9`.
   - Both absorbs, v0.6.4 and v0.7.0, including the 0029→0031 renumber.
   - R13 "outside-adapter" lines for ht-poh.59, .60 and .61.
   - The run phase.
   - Either regenerate `outside-adapter-report.md` or state its range plainly.

   R13 is a spec deliverable, so the report has to describe what actually lands.
2. **Re-run the agy native gate at the final SHA (all 14 flows, leak check clean).** Whether to re-run the live smoke (about $0.03) is the human's call.
3. **The caller's full-suite sweep at the final SHA.**
   - Export `HT_LEAK_RUN_ID` first, then run `scripts/check-no-leaked-processes --run-id` afterwards.
   - It must finish in 300 s or less on a quiet host. The test count is now about 5110, up from 4926, and the two agy over-cap tests take 13–15 s each.
   - Watch ht-yqj.
4. **Decide whether to rename `wake_work.last_lazy_ordinal` (it stores `decision_seq`).** This is the last moment a rename is an in-place edit, because migration 0031 has not shipped. Rename it now or accept the name permanently.

## Untested scope
- **The whole branch:** no full suite at `eb0a1eff` or later.
- **The latest changes against genuine agy:** ht-poh.59, .60 and .61 and the v0.7.0 merge were never exercised against real agy. That covers:
  - notices riding a backlog page;
  - a full-table park and a wake that names `inbox`;
  - a short full-table pull;
  - the notices-only page after a lost rotation race;
  - automatic new-tab handoff to agy.
- **A large-backlog pull under the real 1.5 s hook budget:** not measured end to end.
- **Never run for real:**
  - the 24 h probation retry and a legacy probation record;
  - real `proc_pidpath` identity and Linux `/proc`;
  - `--sandbox`;
  - wake flow phase (g);
  - live-smoke budget abort.
- **Hermes V2 handoff:** covered at preparation only, never through completion.
- **Claude mod TypeScript tests:** not in CI. They passed locally at HEAD (95/95).
- **The ht-poh.54 flake hunt's 180 s timeout:** still unexplained.

## Deferred OK
- **Page bounds:** the attention block outside the bounds (S3; review 11), and one possible extra showing after a concurrent pull fills the slots.
- **ht-poh.59 `[imprecise-predicate]`:** the scheduler checks `holds_parked` only on `NextPull::Nothing`, so it causes no spurious wake.
- **ht-poh.60 minors:** the uncommented `more:false` on the notices-only page, and partial assertion coverage.
- **ht-poh.3.4 replayed confirm:** it answers `not_offered`. Nothing is lost.
- **ht-poh.5 `downgrade.json` read-modify-write race:** at worst it delays a downgrade.
- **ht-poh.4.5 composer heuristic:** an extra full-width rule only widens the region scanned for `Agent(...)` rows, so it can miss a wake but never types into a busy agy (invariant 4 holds).
- **ht-poh.2.4:** `doctor --debug --harness X` dropped the `hooks.<other>: none` lines.
- **ht-poh.18:** unsetup keeps the backup and directory, as the human decided.
- **TRUST-POLICY accepted limit:** notices are lost on a silent injection miss (recorded in `7d3218a5`).
- **Pre-merge branch databases:** they fail closed with `IncompatibleSchema` (S2).
- **Relay, fake-server and gate-script edge cases** (ht-poh.6.x).
- **Cluster follow-ups:** a table-driven probation/downgrade test, a markdown wrap lint, red-first capture in the pipeline, and adding `scripts/test-claude-mod` to CI.

Files:
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/.superpowers/sdd/ht-poh-plan/review-478ec4a9..38c5fef6.diff
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/.superpowers/sdd/ht-poh-plan/progress.md
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/src/service/delivery_pull.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/src/delivery/ledger.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/src/cli/topology_handoff.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/src/store/schema.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/migrations/0031_seat_wake_on.sql
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/docs/superpowers/runs/2026-10-09-agy-harness-adapter/report.md
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/docs/superpowers/runs/2026-10-09-agy-harness-adapter/run.md
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/docs/superpowers/runs/2026-10-09-agy-harness-adapter/outside-adapter-report.md
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/agy-harness-adapter/docs/evidence/agy-native-gate/
