# super-code final review (phase-6 sweep-fix re-entry; informational under deferSweep)

# Final review: epic ht-54r, branch super-auto/session-keyed-seat-bindings at 212dbd0c

**Verdict: not ready.** The four sweep failures are fixed and a focused set of 781 tests passes. But five product defects found in final review 3 are still in the code, and so is stale text in TRUST-POLICY.md, the skill and the subagent context.

**The branch has no full-suite run.** The caller runs the sweep after this review, so treat the branch as untested.

What I ran on this tip, each followed by a clean `check-no-leaked-processes --run-id`:
- **The four sweep-failing tests, plus related tests:** 15 of 15 pass.
- **The binding set** (session_bindings, session_claims, continuity, read_cursors, bindings, receipts, delivery_pull, mod_delivery, first_contact, liveness, caller_env, claude_rewrite, archival, cooperative, lazy_display, multi_binding): 781 of 781 pass.
- **`scripts/check-harness-neutral`:** 0 violations.

The fork point is 43cc19cd, which is main's tip. Review package: `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/.superpowers/sdd/ht-54r-plan/review-43cc19cd..212dbd0c.diff`.

## Step 1: my own review against the spec

Only the four sweep-fix tasks have landed since final review 3 (b2fb0aae):
- **ht-54r.72:** test fixture only.
- **ht-54r.73:** a subagent with no identity of its own now reads its inbox read-only.
- **ht-54r.74:** test cap made configurable.
- **ht-54r.75:** removed the unused `foreground_binding`.

None of them touches the earlier findings. I re-checked each one against the code at this tip:

1. **A Herdr restart makes a resumed session's whole history unread.**
   - `decide_continuity` (src/store/seats.rs:3721) inserts a new `occupant_bindings` row directly. It never reopens the ended row and never seeds cursors.
   - `UNREAD_EXTRA` counts a missing cursor as 0 (read_cursors.rs:178). So every joined thread's history is unread for inbox, hook notices, `has_unread` (wake targets) and the mod fingerprint.
   - The general resolver does the right thing: it reopens a `seat_unresolved` row and calls `seed(Readopt)` (bindings.rs:1372–1423). Continuity bypasses it.
   - This INSERT also skips the enroll hooks (supersede, human end, placeholder replace).
2. **Cursors get stuck after a mod-channel or hook-pull ACK of an ordinary message.**
   - In `ack_policy_delivered` (receipts.rs:227–355), `SettlePlan::Settle` only calls `settle_pending` and never moves the cursor.
   - `complete_for_binding` is called one id at a time, for lazy ids only. Its `advance_through_displayed` stops at the first receivable message that was not displayed.
   - The Claude mod and agy hook pull never send `AdvanceCursors`. Only the CLI and hook_inbox do.
   - Result: on the main Claude delivery path the cursor sticks before each ordinary message. Later lazy mail is delivered again, and `has_unread` stays true, which breaks spec §2.
3. **agy hook delivery breaks after the upgrade.**
   - `hook_pull_view` (delivery_pull.rs:248) returns `SessionMismatch` for a binding that is only disconnected. Every V32-migrated binding is in that state.
   - agy deletes its state file on that refusal (agy/delivery.rs:834), and its turn hooks never re-adopt the binding.
4. **Seat restore matches only the newest binding row.** `continuity_candidates` (seats.rs:3416) still uses `b.ordinal = MAX(ordinal)`. If a one-shot, a subagent or a `/clear` successor enrolled after the main session, the resumed main session is not reattached.
5. **A Hermes child can disconnect its parent.**
   - Hermes `CallerEnvShape.role` is `None`, so the client always sends a top-level claim.
   - If the child's CLI call arrives before `subagent_start` registers it, the child enrolls as top-level with the parent's fingerprint.
   - `same_pid_supersede` (bindings.rs:722) then disconnects the parent, because `event.created` is true. The child can also write.
   - No test covers this ordering.
6. **Mod channel and hook pull are gated on provenance, not role.**
   - The gates are dispatch.rs:832 and :964, and delivery_pull.rs:241, all checking `cooperative_top_level`.
   - They work only because the non-lifecycle `register_available` UPDATE (seats.rs:4567) overwrites `cooperative_env_session` with that value, and the continuity INSERT writes it too.
   - This breaks TRUST-POLICY's "record claims honestly" rule. Fixing either half alone breaks delivery.
7. **The composer wake breaks the spec for a single target.**
   - `composer_choice` (ports.rs:2126) returns `[only] => Binding(only)` with no foreground-PID check, and dispatch.rs:263 then sets `bound_harness` to that target's harness.
   - Example: Claude is in the foreground and a `hermes -z` one-shot is the only binding with unread mail. A Hermes-shaped wake is then typed into Claude's composer.
   - The spec says "if no binding matches, no composer wake fires". This also breaks the delivery contract's rule that nudges are rare.
8. **Normative and model-visible text contradicts the code.**
   - **TRUST-POLICY.md:814** says agy child calls "are not enrolled as subagent bindings". In fact `agy::witness::caller_session` returns `Subagent { parent }`.
   - **TRUST-POLICY.md:327–329** says "a lifecycle check-in … replaces the binding" and keeps the ht-4ne cross-harness context retire, which the spec removed.
   - **integrations/skill/SKILL.md:51** says "Delegated children never enroll" and "no resume continuity". Hermes children do get subagent bindings, and lineage exists.
   - **`CHILD_RESTRICTION`** (src/harness/mod.rs:184) still tells every subagent that text inbox is forbidden. Since ht-54r.58/.73 it is allowed.

## Step 2: ledger triage

- **Clusters:** the ledger has no `Recurring minor:` or `Recurring blocker:` lines.
  - The systemic smell is the merge gate, which is clippy, default features and harness-neutral only, with no focused tests. That is why ht-54r.70 landed with its own test red and the lazy_display regression went unseen until the sweep.
  - A second class: the per-seat to per-binding migration was applied unevenly. Findings 1, 2, 4 and 6 above are all one rule changed on one path but not its sibling.
- **Metrics:** the ledger reports "landed≠completed 41 vs 40". I checked it: every merge that passed its check has a matching `complete` line, so it is a counting artifact and the ledger is complete.
- **Parked findings:**
  - ht-54r.5 (Hermes fixture timeouts under load): not in the sweep's failing list. Deferred OK; watch it in the full suite.
  - ht-54r.12 → ht-ozo: the `/clear` wake model was decided autonomously (ht-ozo is closed). It still needs your sign-off.
  - ht-54r.19: resolved. `config1_subagent_display_advances_its_own_cursor` is present and not ignored, and the branch adds no `#[ignore]`.
- **Fix passes marked BLOCKED (ht-54r.13, ht-54r.19):** both resolved later.
- **BLOCKED-AUTH, ht-54r.7:** its conflict resolution was done outside the pipeline. That is untested scope, not a finding.
- **Minors re-graded to must-fix:**
  - Task 5 [prefix-rule-blocks-lazy] names a known product defect that is still live on the mod and hook-pull paths (finding 2).
  - Task 9 [single-binding-assumption] continuity_candidates (finding 4).
  - Task 3 [provenance-overwrite] (finding 6).
  - Task 33 [stale-doc-string] and Task 23 [doc-inconsistency] (finding 8).
- **Minors that are spec deviations needing your decision:**
  - Task 13 [spec-deviation-single-target] (finding 7).
  - Task 10 [behavior-gap]: an explicit ACK from a disconnected binding is refused, but a mod-delivery ACK from it still settles.
  - Task 16 [compact-form-gap]: agents' compact `participants` output omits presence, while spec §3 asks for active/inactive.
- **The other deferred minors:** missing RED runs, file drift from the brief, performance (per-member presence query, per-thread first-contact count, `binding_mail` capped at 4004 rows, full-scan probes), cosmetics, and "pre-existing failures" that the sweep no longer shows.

## Verdict
**Not ready.**

## Must fix before landing
1. **Continuity reattach** (seats.rs `decide_continuity`): reopen the `seat_unresolved` row through the resolver path, or seed cursors with `SeedMode::Readopt` and run the enroll hooks. Add a test: reattach, then check unread.
2. **`ack_policy_delivered`:** advance the binding's cursor over settled ordinary sequences together with the lazy ones, in one `advance_through_displayed` call per thread. Test interleaved ordinary and lazy delivery over both the mod ACK path and the hook-pull confirm path.
3. **`hook_pull_view`:** re-adopt a migrated or disconnected same-session binding, or give it a distinct refusal, and stop agy deleting its state on it. Add a DeliveryPull test on a V32-migrated agy binding.
4. **`continuity_candidates`:** match any top-level, non-placeholder row ended with `seat_unresolved`, not only the newest row. Add a test with a one-shot or `/clear` binding enrolled after the main session.
5. **Hermes child before `subagent_start`:** a plain-call enrollment that shares a live top-level fingerprint must not supersede it, or must be held. Add a test.
6. **Provenance:** gate the mod channel and hook pull on role, liveness and harness, and remove the provenance overwrite (and the continuity INSERT's top-level value) in the same change.
7. **Text fixes:** correct TRUST-POLICY.md:814 and 327–329, SKILL.md:51 and `CHILD_RESTRICTION`.
8. **Your decisions:**
   - the single-target composer rule (finding 7; I recommend applying the foreground-PID match to one target too);
   - the ht-ozo `/clear` wake model;
   - a disconnected binding refused for ACK while its mod ACK settles;
   - compact output that omits presence.
9. **Human check:** does agy export `ANTIGRAVITY_CONVERSATION_ID` to `run_command` shells? If not, every agy CLI call is refused. Then re-run the agy native gate.
10. **Runs:** the caller's full suite plus leak check, and a green Linux CI run.

## Untested scope
- **Full suite:** never run on 212dbd0c (the caller's sweep is pending). The last sweep was at 7a06cd79: 3 failed and 1 timed out; those four are now fixed and pass when run alone. One config9 failure under load is still unexplained (Task 41).
- **Linux:** the `/proc` `read_process` and `parse_proc_stat` paths have never been compiled here. Only macOS targets are installed, and CI runs ubuntu.
- **Real harness binaries:**
  - Claude: whether `CLAUDE_PID` matches Herdr's foreground PID;
  - Codex: the SubagentStop/SessionEnd payload shapes;
  - Hermes: the `subagent_*` payloads, `parent_session_id` lineage on compression, `/resume`, and how `subagent_start` is ordered against the child's first call;
  - agy: the `run_command` environment.
- **Parked design escalations:** sandboxes hiding the ancestor PID; a sibling PreToolUse hook dropping the `--agent-id` rewrite (claude-code issue 83353).
- **ht-54r.7:** the conflict resolution done outside the pipeline, never seam-reviewed.
- **No test covers:** seat restore with several bindings, the Hermes child-before-start ordering, the agy subagent and unattributable arms of `caller_env::derive`, or two live hook-pull bindings sharing a seat ledger.

## Deferred OK
- **ht-54r.5:** Hermes load timeouts; watch them in the full suite.
- **Performance:** read_cursors full-scan probes, `advance_through_displayed` `Vec::contains`, per-member presence query, per-thread first-contact count, `binding_mail` 4004-row cap, the lazy_delivery.rs:46 index.
- **Code smells:**
  - stale `Entry.generation` in mod_channels;
  - extra Attention pushes from fingerprint churn;
  - `carry_binding_forward` picking an arbitrary row;
  - redundant `exit.rs` arm and redundant .70 fingerprint check;
  - duplicated `run_wire` fallback (.73);
  - unused `connect` parameters;
  - `binding_covered` checks stalls per seat, not per binding;
  - `liveness::classify` keeps its own copy of the winner key.
- **Process:** missing RED runs, file drift from briefs, loosened or indirect test assertions, foreign claude reviewer processes flagged by leak checks (not started by tests).
- **Migrations:** the in-place edit of an unreleased migration (now 0032) and forward-only migration notes; unreleased, so no shipped DB is affected.
- **Accepted limits:** unrecognized shell shapes and `nice -n5` stay attributed to the parent; empty `AdvanceCursors` enrolls a binding; the first-contact open-request count can lag the summary ledger.

Relevant files:
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/src/store/seats.rs (3416, 3721, 4567)
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/src/store/receipts.rs (227–355)
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/src/store/read_cursors.rs (178, 238)
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/src/store/bindings.rs (722, 1279–1423)
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/src/service/delivery_pull.rs (212–255)
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/src/service/dispatch.rs (832, 964)
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/src/ports.rs (2126)
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/src/notification/dispatch.rs (245–265)
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/src/harness/agy/delivery.rs (834)
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/src/harness/mod.rs (184)
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/TRUST-POLICY.md (327–329, 814)
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/integrations/skill/SKILL.md (51)
