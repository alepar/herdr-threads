# super-code final review (regression-only + harness-neutral re-entry; informational under deferSweep)

**Whole-epic review: ht-54r, session-keyed seat bindings**

Branch `super-auto/session-keyed-seat-bindings` at b2fb0aae. Fork point 43cc19cd is main's tip, with v0.8.0 already absorbed. Review package: `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/.superpowers/sdd/ht-54r-plan/review-43cc19cd..b2fb0aae.diff`.

**The full suite has not been run on this tip. The caller runs it next, so treat the branch as untested.** I ran only focused nextest runs, each followed by a clean `check-no-leaked-processes`:
- **Binding set (caller_env, session_claims, session_bindings smoke configs 1–9, mod_delivery, delivery_pull, continuity, liveness, first_contact, cursors, migrations, subagent):** 353 tests, 352 pass.
- **Wake, archival, attention, inbox, receipts, ACK, hooks, watch, setup/doctor and the harness adapters:** 1862 tests, 1861 pass.
- **Tests the ledger named as failing at base or flaky:** 299/299 pass.
- **`scripts/check-harness-neutral`:** 0 violations.
- **Two tests fail every time** (3 of 3 runs, also with a clean environment). Both are listed under Must fix.

## Step 1: my own review against the spec

Fixed since the last final review:
- Migrations are renumbered to 0032–0035, above main's 0031.
- agy goes through the adapter's `CallerEnvShape`.
- S2 (`plain_call_enrolled_agent_display_acks`) passes.
- The harness-neutral gate passes after ht-54r.71.
- ht-54r.70 now records a fingerprint on the continuity reattach.

Still open, all confirmed by reading the code at the tip:

1. **Two tests fail on the tip.**
   - `store::control::tests::continuity_successor_survives_a_liveness_pass_before_its_first_touch` is ht-54r.70's own regression test. It panics with "the successor is a candidate of the pass: []" (tests/store/control.rs:8539). The c1 fixture leaves a recovery hold open, and `liveness_candidates` (src/store/bindings.rs:620) skips every seat on an instance with an open hold. So the .70 fix is unverified: either the test is wrong, or the race it targets cannot happen while holds are open.
   - `lazy_display::lazy_display_json_machine_explicit_selector_readonly` gets "subagents may read and summarize; only top-level agents check in, accept or ACK" for a `Role::TopLevel` caller (tests/cli/lazy_display.rs:259). This is a regression on the read-only inbox selector path (`--json`, `--machine`, `--seat`).

2. **A Herdr restart makes a resumed session's whole history unread.** `decide_continuity` (src/store/seats.rs:3721) INSERTs a new binding row and never calls `read_cursors::seed`. The spec says to reopen the ended row and keep its cursors. A missing cursor counts as 0 (`UNREAD_EXTRA`), so every joined thread's history becomes unread for that binding. That affects:
   - inbox and hook notices;
   - `has_unread`, so the binding stays a wake target;
   - the mod-channel unread fingerprint.

   No test covers continuity reattach followed by an unread check.

3. **Cursors stop advancing after an ACK-required message delivered by push or hook-pull** (round-2 finding lazy_delivery.rs:507, still unfixed). This hits Claude's mod channel and agy's hook pull.
   - `ack_policy_delivered` (src/store/receipts.rs:319) settles ordinary messages through `settle_pending` without moving the cursor.
   - `complete_for_binding` passes only the lazy ids to `advance_through_displayed`, which stops at the first receivable message that was not displayed.
   - Result: the cursor sticks just before the ordinary message, and later lazy mail is delivered again and again. This breaks spec §2 ("display advances this binding's cursor") on the main Claude delivery path.

4. **agy loses hook delivery after the upgrade** (round-2 finding delivery_pull.rs:249, unfixed). `hook_pull_view` returns `SessionMismatch` for a binding that is only disconnected, which every V32-migrated binding is.
   - The agy client deletes its state file on that code (src/harness/agy/delivery.rs:834).
   - agy's turn hooks never check in, so they never re-adopt the binding.
   - Every agy conversation running at upgrade gets no hook delivery until it makes a CLI call.

5. **S3: restoring a seat matches only its newest binding row.** `continuity_candidates` (src/store/seats.rs:3416) still uses `b.ordinal = MAX(ordinal)`. If a one-shot, a Hermes child, or a `/clear` successor enrolled after the main session, a resumed main session is not reattached.

6. **S4: a Hermes child can disconnect its parent.** If the child's CLI call arrives before `subagent_start`, it enrolls as top-level with the parent process's fingerprint. `same_pid_supersede` (bindings.rs:722) then disconnects the parent, because `event.created` is true. No test covers this ordering.

7. **S5: mod channel and hook pull are still gated on `cooperative_top_level` provenance** (dispatch.rs:832, :964; delivery_pull.rs:241). They work only because the non-lifecycle `register_available` UPDATE (seats.rs:4567) overwrites `cooperative_env_session` with `cooperative_top_level`. That conflicts with the TRUST-POLICY rule to record claims honestly, and fixing either half alone breaks delivery.

8. **TRUST-POLICY.md:804–809 contradicts the code.** It says agy child calls "are not enrolled as subagent bindings", but the code enrolls them as read-only subagents of the parent. The stale ht-4ne "retires the pane's client-local context" wording at ~312/322 is also still there. TRUST-POLICY is normative.

9. **agy CLI calls may all be refused.** The branch refuses an agy CLI call without `ANTIGRAVITY_CONVERSATION_ID` (no pane fallback). Whether agy exports that variable to `run_command` shells has never been checked. If it does not, every agy CLI call is refused after upgrade (Blocking). The agy native gate has not run since merge 8d5120ed.

10. **Smaller items:**
    - The agy `Subagent`/`Unattributable` mapping in `caller_env::derive` has no test (round-2 finding caller_env.rs:358).
    - `foreground_binding` has no production caller.
    - `carry_binding_forward` has no liveness filter.
    - The poke path calls a no-op `before_wake_decision`.
    - `CHILD_RESTRICTION` text is stale.

## Step 2: ledger triage

- **Clusters:** the ledger has no `Recurring` lines. The class I see is the same pipeline defect for the third review in a row: the merge gate is clippy, default features and harness-neutral only, with no focused tests. ht-54r.70 landed with its own regression test red, and the lazy_display regression went unnoticed. This needs a focused-test merge gate.
- **Parked:**
  - ht-54r.5 (Hermes timeouts under load): those tests pass now. Deferred OK; watch them in the full suite.
  - ht-54r.12 → ht-ozo: the `/clear` wake model was decided autonomously and needs your sign-off.
  - ht-54r.19: resolved; the un-ignored test passes.
- **Fix passes marked BLOCKED (2):** both resolved (ht-54r.13 evidence re-run; ht-54r.19 unblocked by ht-54r.58).
- **BLOCKED-AUTH, ht-54r.7:** the conflict resolution was rebased outside the pipeline and never seam-reviewed. That is untested scope, not a finding.
- **Metrics (landed≠completed, 37 vs 36):** both new landings (.70, .71) have completion lines, so this is the same counting artifact as before.
- **check-fails 2 (fixed 1):** the unfixed one is .70, which was held until .71 landed. It then passed the lint-only gate with its own test red.
- **Minors regraded to must-fix:**
  - Task 9 [single-binding-assumption] continuity_candidates: this is item 5.
  - Task 3 [provenance-overwrite]: coupled to item 7.
  - Task 2 [unverified-platform-path]: the Linux `/proc` code has never been compiled here, and CI runs ubuntu.
- **Minors that are spec deviations (need your decision):**
  - Task 13: single-target seats keep the old composer rule.
  - Task 20: a disconnected binding is refused for ACK, while a mod-delivery ACK from it still settles.
- **The other 124 deferred minors:** process smells (RED runs not captured, thin evidence), performance, cosmetics. All the "pre-existing failure" minors I re-ran now pass.

## Verdict
**Not ready.**

## Must fix before landing
1. Make both red tests pass: fix the .70 test fixture (or the hold gating it exposes), and fix the explicit-selector read-only inbox regression.
2. Continuity reattach: reopen the `seat_unresolved` row, or seed or copy cursors onto the new row (`SeedMode::Readopt`). Add a test for reattach followed by an unread check.
3. In `ack_policy_delivered`, advance the cursor past settled ordinary sequences together with the lazy ones. Test interleaved ordinary and lazy delivery over both the mod ACK and hook-pull confirm paths.
4. In `hook_pull_view`, give a disconnected same-session binding a distinct refusal (or re-adopt a migrated binding), and stop agy deleting its state on it. Add a DeliveryPull test on a V32-migrated agy binding.
5. Make `continuity_candidates` match any top-level, non-placeholder row ended `seat_unresolved`. Add a test with a one-shot or `/clear` binding enrolled after the main session.
6. S4: a plain-call enrollment of an unregistered Hermes child must not supersede its parent. Add a test.
7. S5: gate mod channel and hook pull on role, liveness and harness, and remove the provenance overwrite, in one change.
8. Correct TRUST-POLICY.md:804–809 and the stale ~312/322 wording.
9. Human check: does agy export `ANTIGRAVITY_CONVERSATION_ID` to `run_command` shells? Then re-run the agy native gate.
10. Your decisions: the ht-ozo `/clear` wake model, the single-target composer rule, and ACK refused for a disconnected binding.
11. The caller's full suite plus leak check, and a green Linux CI run.

## Untested scope
- **Full suite:** never run on this tip.
- **Linux:** the `/proc` paths have never been compiled here.
- **Real harness binaries:**
  - Claude: whether `CLAUDE_PID` matches Herdr's foreground PID;
  - Codex: the SubagentStop/SessionEnd payload shapes;
  - Hermes: the `subagent_*` payloads, `parent_session_id` lineage on compression, and `/resume`;
  - agy: `run_command` environment and the native gate since the merge.
- **Parked design escalations:** sandboxes hiding the ancestor PID; a sibling PreToolUse hook dropping the `--agent-id` rewrite.
- **ht-54r.7:** the conflict resolution done outside the pipeline.
- **Never exercised by a test:** seat restore with several bindings, the Hermes child-before-start ordering, and two live hook-pull bindings sharing a seat ledger (ledger.rs:162).
- **ht-54r.71:** the attribution refactor of all three builtins landed with no test changes. The existing caller_env tests pass, but the agy subagent and unattributable arms have no test.

## Deferred OK
- **Punch-list items:**
  - read_cursors.rs:197 full-scan probes (performance);
  - claude_rewrite error wording;
  - lazy_delivery.rs:46 index;
  - forward-only migration changelog;
  - ledger.rs:162 seat-keyed offer ledger (Nit, no mail lost).
- **S6 smalls:** test-only `foreground_binding`, no liveness filter in `carry_binding_forward`, the no-op poke `before_wake_decision`.
- **Process and doc smells:** missing RED runs, brief and file drift, redundant `exit.rs` arm, stale `mod_channels` `Entry.generation`, stale `CHILD_RESTRICTION` text, the redundant .70 fingerprint check, and the stray `claude` process (pid 26599) the .71 leak check flagged; it was not started by the tests.
- **Performance:** one query per member in presence, one count per thread at first contact, `binding_mail` limited to the newest 4004 rows.

Relevant files:
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/src/store/seats.rs (3416, 3721, 4567)
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/src/store/receipts.rs (227–355)
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/src/store/read_cursors.rs (238)
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/src/service/delivery_pull.rs (241–254)
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/src/store/bindings.rs (620, 722, 1285)
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/src/service/dispatch.rs (832, 964)
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/TRUST-POLICY.md (804)
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/tests/store/control.rs (8517)
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/tests/cli/lazy_display.rs (325)
- Test logs: /private/tmp/claude-501/-Users-alepar-AleCode-herdr-threads/76e60185-b070-440a-a1bc-85cd58cdd810/scratchpad/final3.log, final4.log
