# super-code final review (fix-loop round 1 re-entry, informational under deferSweep)

**Whole-epic review: ht-54r, session-keyed seat bindings (branch super-auto/session-keyed-seat-bindings, fork point cb6052fc, tip dd85423f)**

The full suite has never been run on this tip; the caller runs it after this review, so the branch is not tested. I ran only focused nextest runs, each followed by a clean `check-no-leaked-processes`:
- The 12 tests the previous final review found failing every time, plus neighbours: 90/90 pass.
- `mod_delivery`, `session_bindings_*`, `session_claims`, `pane_close_retirement` and the liveness tests: 88/89 pass. The one failure is S2.
- 968 other binding-related tests (cursors, liveness, check-in, first contact, catch-up, archival, schema/V29, topology_handoff, continuity, multi_binding): all pass.

## Step 1: my own review against the spec

The previous final review's F1–F4 are fixed on the tip:
- F1 (lazy inbox per seat): fixed by ht-54r.57.
- F2 (subagent reads its own stream): fixed by ht-54r.58. `config1_subagent_display_advances_its_own_cursor` is no longer ignored and passes.
- F3 (display-ACK for plain-call bindings): the product fix is in by ht-54r.59, but its test now fails (S2).
- F4: the smoke configs 1–9 and the ht-54r.25 sweep landed and pass.
- The `CODEX_THREAD_ID` silent downgrade is now a refusal, and connected bindings with no fingerprint are now disconnected.

New findings:

**S1. Main has moved far ahead, and the branch cannot land as it is.** Main is 335 commits and 1004 files (+101k lines) ahead of the fork point.
- **Migration numbers collide.**
  - Main has `0029_direct_threads`, `0030_automatic_handoff` and `0031_seat_wake_on`.
  - The branch has `0029_session_bindings`, `0030_archival_scan_scope`, `0031_binding_delivery_state` and `0032_binding_attention_frontiers`.
  - A database already at main's schema 31 would skip three of this branch's four migrations.
- **Main adds a fourth built-in harness, agy.** `src/harness/caller_env.rs` (`classify`/`derive`) only knows claude, codex and hermes. After the merge, an agy agent's call would be treated as a person's (pane-context path) or refused.
- Main's `seat_wake_on` and its direct-thread and automatic-handoff work overlap the per-binding wake and seat changes here.

**S2. One test fails every time on the tip: `session_claims::plain_call_enrolled_agent_display_acks`** (tests/integration/session_claims.rs:511; 5 of 5 runs, left `["pending"]`).
- This is ht-54r.59's test for the F3 fix. To model a long-lived agent it forces the binding to connected with a NULL fingerprint.
- ht-54r.56 merged later and makes the liveness worker disconnect any connected binding with no fingerprint. The ACK is then refused.
- So display-ACK by a binding enrolled through a plain call is unverified again.
- The merge check is clippy only, so this seam went unnoticed.

**S3. Restoring a seat after a Herdr restart matches only the seat's newest binding row.** `continuity_candidates` (src/store/seats.rs:3403) is unchanged from the single-binding base: `b.ordinal = MAX(ordinal)`.
- Under the many-bindings model the newest row can belong to another session:
  - a one-shot (`hermes -z`, `claude -p`) that ran inside the pane;
  - a Hermes delegate child, whose `native_session` is the child's own id;
  - a `/clear` successor session.
- When that happens, a resumed Claude session no longer matches its seat and the seat is not reattached.
- Claude and Codex subagent rows carry the parent's session, so they do not cause it.
- Confirmed by reading the code; no test covers it. The ledger logged it as a minor (Task 9 [single-binding-assumption]) handed to an "owner" that does not exist.

**S4. A Hermes delegate child can disconnect its parent.** Plausible; not reproduced.
- If a child's CLI call arrives before `subagent_start` registers it (hook failed, timed out, or stale bridge), it enrolls as a top-level binding with the Hermes process's fingerprint.
- `same_pid_supersede` then disconnects the parent.
- Plain calls never reconnect, and a disconnected binding is refused for ACK. The parent stays crippled until its next lifecycle event. No test covers this ordering.

**S5. Part of the role-plus-liveness fix is still unapplied.**
- Mod channel registration and mod ACK still require `cooperative_top_level` provenance (src/service/dispatch.rs:593 and :717).
- They only work for re-adopted or plain-call Claude bindings because the non-lifecycle update in `register_available` (seats.rs:4545) overwrites provenance (deferred minor, Task 3 [provenance-overwrite]).
- The two defects mask each other: fixing the overwrite alone breaks mod delivery.

**S6. Smaller items.**
- `store::wake::foreground_binding` has no production caller, yet smoke config 9 asserts through it. Production uses `ports::composer_choice`.
- `carry_binding_forward` closes the seat's unavailability episode even when it carried only disconnected bindings (no liveness filter).
- The poke path calls the runner's no-op `before_wake_decision`, not the store's liveness re-check.

## Step 2: ledger triage

- **Clusters:** the ledger has no `Recurring` lines. The class I see is cross-merge test seams: merges are gated by clippy only and seam reviews run no tests, so a later merge silently breaks an earlier fix's test (S2). It is the same pipeline defect as the previous review's "red tip accepted as the baseline". It needs a focused-test merge gate.
- **Parked findings:**
  - ht-54r.5: 11 Hermes/continuity hook tests time out under load. Assign them to ht-zo4 and watch for them in the full suite.
  - ht-54r.12 → ht-ozo: the `/clear` wake model was decided autonomously (no rebind-grace suppression). It needs your sign-off.
  - ht-54r.19: the un-ignore is done; that test passes.
- **Fix passes marked BLOCKED (2):** both are resolved. ht-54r.13 only needed evidence, re-run 12/12. ht-54r.19 was unblocked by ht-54r.58.
- **BLOCKED-AUTH, ht-54r.7:** a permission refusal cost the pipeline its own conflict resolution. The rebase was done outside the pipeline and its resolution was never seam-reviewed. F1 sat in that area and is fixed, but the resolution itself remains untested scope.
- **Metrics (landed≠completed, 35 vs 34):** I checked: all 35 distinct merged tasks have completion lines, so the gap is a counting artifact.
- **Minors regraded to must-fix:**
  - Task 9 [single-binding-assumption]: this is S3.
  - Task 34 [test-db-shortcut]: that shortcut is now the failing test (S2).
  - Task 3 [provenance-overwrite]: coupled to S5.
  - Task 2 [unverified-platform-path]: the Linux code paths have never been compiled, and CI runs ubuntu.
- **Minors that are spec deviations (need your sign-off):**
  - Task 13: seats with a single wake target keep the old composer rule.
  - Task 20: a disconnected binding is refused for ACK. As Task 10 notes, a mod-delivery ACK from a disconnected binding still settles, which is inconsistent.
- **Minor to watch:** Task 26 [flaky-test-uninvestigated] `live::composition_actual_new_thread_child_requires_canonical_created_thread`; watch it in the full suite.

## Verdict
**Not ready.**

## Must fix before landing
1. Absorb main at its actual tip:
   - renumber this branch's migrations above main's maximum (32–35, or higher if more land first) and re-run the V28→Vmax upgrade tests and smoke config 8;
   - teach `caller_env` the agy harness (session variable and process shape);
   - reconcile per-binding wake with `seat_wake_on`;
   - re-review the absorbed branch.
2. Fix `plain_call_enrolled_agent_display_acks` using a long-lived stand-in harness process (`HT_TEST_HARNESS_PID` / `as_agent`), so the F3 display-ACK fix is verified again.
3. `continuity_candidates`: match any top-level, non-placeholder binding row of the unresolved seat (for example, those ended with `seat_unresolved`), not just the newest. Add a test with a one-shot or `/clear` binding enrolled after the main session.
4. Get your decisions on the ht-ozo `/clear` wake model and the two spec deviations (single-target composer rule; ACK refused for a disconnected binding).
5. S5: gate mod channel registration and mod ACK on role, liveness and harness, and stop the provenance overwrite, together in one change.
6. S4: add a test for a Hermes child's CLI call arriving before `subagent_start`. If it supersedes the parent, keep that enrollment from superseding.
7. The caller's full suite plus leak check, and a green Linux CI job (the Linux `read_process` and `parse_proc_stat` paths are uncompiled).

## Untested scope
- **Full suite:** never run on this tip.
- **Real harness binaries:**
  - Claude: whether `CLAUDE_PID` matches Herdr's foreground PID for the composer wake;
  - Codex: the SubagentStop/SessionEnd payload shapes;
  - Hermes: the `subagent_*` payloads, `parent_session_id` lineage on compression, and `/resume`.
- **Parked design escalations:** sandboxes hiding the ancestor PID; a sibling PreToolUse hook dropping the `--agent-id` rewrite.
- **Linux.**
- **ht-54r.7:** the conflict resolution rebased outside the pipeline.
- **Never exercised:** seat reattachment with several bindings (S3) and the Hermes child ordering (S4).
- **After the merge with main:** everything.

## Deferred OK
- **Process smells:** TDD RED runs not captured, tests placed away from the brief's files, doc and line-length nits, the redundant `exit.rs` arm, the misplaced `ports.rs` doc comment.
- **Performance:**
  - one query per member in presence;
  - one count per thread at first contact;
  - one query per step in the unread walk;
  - `binding_mail` walks only the newest 4004 rows (notices may miss older mail; the inbox still shows it);
  - the archival trigger's `WHEN` is effectively always true.
- **Cosmetic or documented:** stale mod-channel `Entry.generation`, the extra Attention push on fingerprint churn, the empty `AdvanceCursors` enroll, `lineage_parent` honoured for any harness, `nice -n5` and unparsed shell shapes (accepted limits in TRUST-POLICY), and 0029 edited in place (moot after the renumber).
- **Cheap doc fixes worth doing while in there:** `CHILD_RESTRICTION` in src/harness/mod.rs still tells subagents the text inbox is forbidden, and the SKILL.md and docs wording on subagent enrollment differs per harness.

Relevant files:
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/src/store/seats.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/tests/integration/session_claims.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/src/harness/caller_env.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/src/service/dispatch.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/src/store/bindings.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/migrations/
- Review package: /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/.superpowers/sdd/ht-54r-plan/review-cb6052fc..dd85423f.diff
- Test logs (scratchpad):
  - /private/tmp/claude-501/-Users-alepar-AleCode-herdr-threads/76e60185-b070-440a-a1bc-85cd58cdd810/scratchpad/focused2.log
  - /private/tmp/claude-501/-Users-alepar-AleCode-herdr-threads/76e60185-b070-440a-a1bc-85cd58cdd810/scratchpad/focused3.log
  - /private/tmp/claude-501/-Users-alepar-AleCode-herdr-threads/76e60185-b070-440a-a1bc-85cd58cdd810/scratchpad/focused4.log
