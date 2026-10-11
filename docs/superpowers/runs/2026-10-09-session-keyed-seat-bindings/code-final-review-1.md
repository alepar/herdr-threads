# super-code final review (phase 3, informational under deferSweep)

**Verdict: not ready.** The branch has 12 tests that fail every time, two of the spec's core flows are broken, and the end-to-end smoke and integration sweep never landed. No full-suite run of this branch exists yet. The caller runs it after this review, so nothing below should be read as a passing suite.

## Step 1: my own review against the spec, before reading the ledger

**F1. Lazy mail in the inbox is still worked out per seat, not per binding.** Confirmed by reading the code; I did not reproduce it with a test.
- In `src/store/queries.rs` (inbox page, `scan`, `Source::Lazy`, about line 6514), a bound read gets its lazy items from `lazy_delivery::pending_page`. That function filters with `seat_unread_sql!` (`src/store/lazy_delivery.rs:44`), which means "unread by *some* connected top-level binding of the seat". It never uses the reading binding's own cursor. The per-binding functions (`binding_lazy_unread`, `binding_lazy_window`) exist but the inbox page does not call them.
- Effect with two connected top-level bindings A and B:
  - after A displays lazy message M, A's next inbox shows M again for as long as B has not read it;
  - once every connected top-level binding has read M, a subagent, or a disconnected binding that is still being served, never sees M even though its cursor is short of it.
- This breaks decision 4 and §2 ("each binding sees its own full stream… one binding's display never hides mail from another").
- It is a seam between ht-54r.7 (inbox) and ht-54r.22 (lazy state). `tests/store/binding_lazy.rs` only tests the lower-level functions, and `tests/store/binding_inbox.rs` has no lazy case. The smoke task found the same thing independently (its report, finding 2).

**F2. A subagent cannot read its own stream.** This is bead ht-6v2.
- `src/cli/mod.rs:2799` sends only `Role::TopLevel` callers to the display inbox. A subagent's text inbox fails with "selected seat required".
- Even with that gate lifted, `src/service/dispatch.rs:808` refuses the `AdvanceCursors` call with `SubagentReadOnly`, and the CLI ignores the error.
- So decisions 4 and 13 do not work for subagents: their cursor never moves.

**F3. Display-ACK is likely refused for any top-level binding that was not created by a lifecycle check-in.** Plausible; not reproduced.
- Bindings enrolled or re-adopted by a plain call get provenance `cooperative_env_session` (`src/store/bindings.rs` around line 1243, through `refresh_placement`).
- The display-ACK check in `src/store/receipts.rs:315` requires `observation_provenance='cooperative_top_level'`. The inbox's ACK-candidate query (`queries.rs:6764`) and the archival idle-sample check (`archival.rs:386`) require the same.
- Bindings affected:
  - one-shots (`hermes -z`, `claude -p`);
  - every agent that was mid-session at the upgrade and re-adopts its migrated binding through its next CLI call. That is the spec's main upgrade path.
- For those bindings, displaying a message would not settle its ACK obligation until the next SessionStart. An explicit `ack` still works. No test covers a binding enrolled by a plain call doing a display-ACK.

**F4. The spec's §4 tests are not on the branch.**
- The configuration smoke (ht-54r.19), which includes the ht-4ne probe as a regression test, exists only on `task-ht-54r.19` and is BLOCKED on ht-6v2.
- The integration sweep (ht-54r.25) was cancelled and never ran. Its checks never happened: enroll and disconnect hooks registered, provenance values written, TRUST-POLICY consistent with the code.

**F5. Composer wake: the predicate the smoke tested is not the one production uses.**
- Config 9 of the unmerged smoke tests `store::wake::foreground_binding`.
- Production dispatch uses a duplicate, `composer_choice` in `ports.rs` (this matches ht-54r.13's minor [unused-predicate]).
- Matching a real Codex or Claude fingerprint PID against Herdr's foreground PID has never been tried with real harness binaries.

## Step 2: ledger triage

**Recurring cluster.** The ledger has no `Recurring` lines. Reading the entries myself, one class stands out: failing tests on the integration tip get logged as "pre-existing / fails at base" and are never assigned to anyone.
- It shows up in ht-54r.4, .5, .23, .11, .17, .9, .12 and .39.
- This is a pipeline defect: a red integration tip was accepted as the baseline for every later task.
- I re-ran the 12 tests listed in the ht-54r.9 report on the current tip. **All 12 fail, deterministically, 0 of 12 passed.** The process leak check afterwards was clean.

**Parked findings, both need action:**
- **ht-54r.12 → ht-ozo.** `mod_delivery::clear_within_rebind_grace…` and `resume_reack_settles` fail. Under the new model, `/clear` fires a native wake for the new binding (no rebind-grace suppression). This needs your product decision before landing; it is not just a test to rewrite.
- **ht-54r.5.** 11 Hermes/continuity hook tests time out under load. These are flakes. They are acceptable only if you assign them to the ht-zo4 flakiness work. Watch for them in the full-suite run.

**BLOCKED-AUTH, ht-54r.7.** A permission refusal stopped the merge conflict resolution. An agent outside the run later rebased the branch and it landed with a fix pass. The rebase itself was never seam-reviewed by the pipeline, and F1 sits in exactly that area.

**Ledger accounting.** The "landed≠completed 24 vs 23" gap comes from ht-54r.20, which is logged as both "cancelled" and "complete". It is a bookkeeping slip, not missing work.

**Minor findings I regraded to must-fix:**
- [pre-existing-failure] and [stale-baseline] lines: these are known failing tests (see the cluster above).
- ht-54r.19's `#[ignore]`d `config1_subagent_display_advances_its_own_cursor`: an ignored test covering a known product defect.
- ht-54r.23 [spec-deviation]: the liveness worker leaves connected bindings that have no fingerprint alone. ht-54r.5 has now landed, so this should be widened as planned. Decision 9 says no zombie bindings.
- ht-54r.2 [silent-role-downgrade]: a malformed `CODEX_THREAD_ID` gives a top-level claim, which lets a subagent write and breaks decision 13. It is cheap to make this a refusal.

**Spec deviations that need your sign-off** (they are written into TRUST-POLICY, but the spec says otherwise):
- ht-54r.13: seats with a single target keep the old composer-wake rule.
- ht-54r.20: a disconnected binding is refused for ACK and AckDisplayed.

## Must fix before landing
1. **F1:** make the inbox's lazy source read per binding (`binding_lazy_unread` / the reading binding's cursor), and add inbox-level tests with two bindings and with a subagent.
2. **F2 / ht-54r.19:** fix ht-6v2 so subagents can display and advance their own cursor. Then un-ignore the test and merge `task-ht-54r.19` (configs 1–9, including the ht-4ne probe).
3. **F3:** check, then fix, display-ACK for bindings with `cooperative_env_session` provenance (plain enrollment and migrated re-adoption). Add a test where an agent re-adopts with no lifecycle check-in, displays, and the ACK settles.
4. **Make the 12 deterministic failures green** by fixing or rewriting each one:
   - `no_duplicate_delivery_after_v29_migration` and `handoff_actual22_upgrade…` fail on LivenessFingerprintRequired. Until they pass, the migration's no-duplicate-delivery guarantee is unverified.
   - `bridge_tests::cooperative_typed_rejection…` (SubagentReadOnly).
   - The two `contracts::` tests.
   - `ports_have_no_unsupported_defaults…`.
   - `sweep::cooperative_setup_sweep…`.
   - `hermes_enrollment_sweep` and `hermes_enrollment_smoke`: they see 2 bindings and expect 1; check whether that is a stale test or a real defect.
   - `harness_adapter_wiring`.
   - The two `mod_delivery` tests, which wait on the ht-ozo decision.
5. **ht-ozo:** decide the `/clear` wake-suppression model.
6. **Run ht-54r.25 (integration sweep), then the full suite plus the leak check.**
7. Cheap fixes: the `CODEX_THREAD_ID` silent downgrade becomes a refusal; widen the fingerprint-less disconnect.

## Untested scope
- **Full suite:** never run on this tip. The caller's sweep is the first measurement.
- **Real harness binaries:** none were used. Unchecked:
  - Codex SubagentStop/SessionEnd and Hermes `subagent_*` payload shapes are assumed (ht-54r.11);
  - the real foreground-PID composer wake for Claude and Codex (F5);
  - Hermes `parent_session_id` lineage on compression, which the spec itself marks unverified.
- **Linux:** the `/proc` process-table and continuity paths were never compiled or run (ht-54r.2).
- **Coverage lost to the permission refusal:** ht-54r.7's conflict resolution, rebased outside the pipeline.
- **Pane close → retire → archive, Herdr restart hold, V28→V32 upgrade:** tested only on the unmerged smoke branch. The upgrade test on the branch itself fails (item 4).
- **Codex subagents** get no read-only notice (ht-54r.9).
- **Smaller gaps:** archival worker-level restart; JSON goldens for presence fields; the slow idle-daemon test.

## Deferred OK
- **Process smells:** the TDD-RED-not-captured cluster (ht-54r.6, .14, .21, .15, .22, .20, .11, .9); test placement and file drift from the briefs; doc and line-length nits; redundant `exit.rs` arm; the misplaced doc comment in `ports.rs`.
- **Performance:**
  - one query per member in presence;
  - first-contact runs one count per thread;
  - per-step unread walk;
  - `binding_mail` only walks the newest 4004 recipient rows;
  - full-history lazy scan for the seat view;
  - archival trigger `WHEN` is effectively always true because of `prepared_recipients`.
- **Accepted limits:** unparsed shell shapes in the Claude `--agent-id` rewrite (in TRUST-POLICY); `nice -n5`.
- **Unreleased only:** `0029` was edited in place. This only breaks dev databases already stamped V29.
- **Cosmetic or documented:** stale mod-channel `Entry.generation` in status output; extra Attention push on fingerprint churn; human claim taking the readopt path (unreachable, since the key includes the harness); `lineage_parent` honoured for any harness (same-harness validation still applies); open-request count lagging summaries; the empty-`AdvanceCursors` enroll (documented); the fast-forward cursor jump not seen by the mod fingerprint.

Relevant paths:
- `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/src/store/queries.rs`
- `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/src/store/lazy_delivery.rs`
- `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/src/store/receipts.rs`
- `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/src/store/bindings.rs`
- `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/src/cli/mod.rs`
- `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/src/service/dispatch.rs`
- `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/.superpowers/sdd/ht-54r-plan/review-cb6052fc..49e665c6.diff`
- Focused test log: `/private/tmp/claude-501/-Users-alepar-AleCode-herdr-threads/76e60185-b070-440a-a1bc-85cd58cdd810/scratchpad/focused.log`
