# super-code final review (post-hand-back fix round)

**Final review: epic ht-54r, branch `super-auto/session-keyed-seat-bindings` at 20955d16 (fork point 43cc19cd)**

The branch has no full-suite run. The last green sweep was at abe1bf03 (5471/5471), and that is before all nine fix-round tasks (ht-54r.76 to .84). I ran no tests; this review is read-only. Treat the branch as untested until the caller's sweep and leak check pass.

Review package: `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/.superpowers/sdd/ht-54r-plan/review-43cc19cd..20955d16.diff`

## Step 1: my own review against the spec

The fix round closes final review 4's must-fix items 1 to 7:
- continuity reattach now goes through the general resolver and matches any of the seat's bindings;
- an ACK over the mod channel or hook pull now advances the binding's cursor;
- the agy delivery pull re-adopts a disconnected binding;
- delivery gates check role, liveness and harness, and provenance is no longer overwritten;
- the stale text is fixed;
- a single target now needs the foreground-PID match;
- compact output shows presence.

New problems where these tasks meet each other:

**1. A Hermes child's call before `subagent_start` leaves a stray top-level binding that never goes away (Important).**
- Since ht-54r.79, the child's early CLI call enrolls a top-level binding keyed `(seat, hermes, child, '')` with the parent's fingerprint. Because it came from a plain call, it no longer supersedes the parent.
- `subagent_start` then registers a different key, `(seat, hermes, child, agent_id=child)`, in `attach_registered_child` (src/store/seats.rs:4434). Nothing disconnects or converts the early row.
- The liveness worker skips rows that only plain calls touched (`lifecycle_claimed`, src/service/liveness.rs:61). Hermes' per-turn check-ins are `current`, not lifecycle (src/harness/hermes.rs:1034). So the stray row stays connected until the Hermes process exits or its next `/new`.
- Effects:
  - Until `subagent_start` arrives, the child can write, which breaks decision 13.
  - The stray row's cursor never advances, so it is a wake target for every new message.
  - When the parent is also a target, both share one PID, so the foreground match finds two and no composer wake fires.
  - When only the stray row is a target, Hermes gets nudged for mail it has already read.
- The new test `hermes_child_cli_call_before_subagent_start_leaves_the_parent_connected` (tests/service/cooperative.rs) asserts that the child stays a connected `top_level` binding, and checks nothing after `subagent_start`.
- Hermes callbacks can also be dropped by the bridge's single-flight lock (a parked escalation), which makes this ordering more likely.

**2. Any call now reconnects a disconnected binding (ht-54r.81), which contradicts spec §1 and makes superseded bindings flap (Important).**
- Spec §1 (design doc line 158) says a plain touch from a disconnected binding "leaves its liveness unchanged. This keeps a stale caller from flapping". ht-54r.81 applied §3's "any call the binding makes" instead, and §1 was never updated.
- A stale caller still carrying a pre-`/clear` session id now reconnects the superseded binding on every call (`plain_reconnect`, src/store/bindings.rs:1264). The worker then supersedes it again. Each flip bumps the instance-wide archival `mutation_revision` (migrations/0033 trigger on `liveness`), so a polling stale caller brings back the scan churn that spec §3 fixed. It also churns attention.
- For a binding that only plain calls have touched, the worker never supersedes it again. TRUST-POLICY's new sentence ("the liveness worker's same-process rule disconnects it `superseded` again") is then false, and the reconnect is permanent: two connected bindings with one PID, so no composer wake.
- That set includes every V32-migrated binding a plain call re-adopted (the readopt branch writes `cooperative_env_session`, bindings.rs:1349–1360). That covers each upgraded agent until its next SessionStart.
- `hook_pull_view` already treats `superseded` as stale (src/service/delivery_pull.rs:229). The CLI path is inconsistent with it.
- Suggested single fix for items 1 and 2: a plain call must not reconnect a binding disconnected as `superseded`. The worker should rank a binding touched only by plain calls below a lifecycle-claimed binding with the same fingerprint (it can lose, never win).

**3. Single-target composer wakes now depend on an unverified PID match (Important; needs evidence).**
- After ht-54r.82, every cooperative native wake needs Herdr to report exactly one foreground process that is not the shell, and its PID must equal the fingerprint (`started_process_pid`, src/host/native.rs:2356).
- No real-harness evidence covers:
  - Claude: whether `CLAUDE_PID` equals Herdr's foreground PID;
  - Codex installed via npm: the node shim plus the native `codex` binary would be two foreground processes;
  - Hermes started by `launch`.
- The ht-54r.82 review raised the same risk ([unmeasured-risk]). The single-agent pane is the most common setup, and this rule is the only idle-wake path for Codex and agy.

**4. Smaller spec gaps:**
- `decide_continuity` discards the `Reopened` resolution, so a reattach after a Herdr restart never fast-forwards (spec §2).
- `continuity_candidates` (seats.rs:3418) does not filter on `seat_unresolved`. A session that was resumed in a second pane gives two candidates, and the reattach is refused as ambiguous.
- The poke path still has no liveness re-check before the wake decision: `try_poke` calls the no-op at src/scheduler/mod.rs:749 (residue of S6).
- The design doc's Post-Implementation Notes (lines 446–447) still list the three deviations that are now fixed, and say "mod channel and hook-pull still provenance-gated — open".
- SKILL.md and docs/agent-usage.md list the session variables without agy's `ANTIGRAVITY_CONVERSATION_ID`.

## Step 2: ledger triage

**Metrics.** "landed≠completed 50 vs 49": every fix-round merge (.76 to .84) has a matching `complete` line. The gap is the earlier ht-54r.1 re-dispatch counting artifact, so the ledger is complete for this round.

**Recurring clusters.**
- [missing-red-run]: ×3 by the ledger's count (ht-54r.6, .21, .83). With ht-54r.77 [no-red-run] and ht-54r.82 [red-step-skipped] it is five tasks.
  - This is a pipeline defect: the RED step is not enforced, and the merge gate only builds.
  - So none of the fix round's tests has been shown to fail on the old code. Some are loose: ht-54r.76 checks cursors with `>=`, and the ht-54r.81 smoke test's `wait_until` can pass at once.
- [doc-drift]: ×3 (ht-54r.2, .21, .80). The class is normative and spec text not kept in step with the code. It is still live in items 2 and 4 above, and possibly at TRUST-POLICY.md:842.

**Minors re-graded to must-fix.**
- **ht-54r.84 [unseparated-gate-failures]:** 5 of 15 agy native gate flows fail (clear, resume, delivery, downgrade, wake), with no base-vs-branch run.
  - delivery, downgrade and wake fail at `accept` with "no lifecycle check-in context yet" (src/cli/mod.rs:2695).
  - That is the pane-context path for a caller with no harness, which the branch retired for agent seats.
  - This names failing tests and a possible product regression.
- **ht-54r.82 [unmeasured-risk]:** this is item 3 above.

**Parked lines.**
- ht-54r.5 (Hermes fixture timeouts under load): deferred; watch it in the full suite.
- ht-54r.12 → ht-ozo (the `/clear` wake model): settled by the run's "spec-settled decisions" entry (6342b5d5).
- ht-54r.19 (ignored subagent-cursor test): resolved. No `#[ignore]` was added anywhere on the branch.

**BLOCKED-AUTH, ht-54r.7:** its conflict was resolved outside the pipeline and never seam-reviewed. That is untested scope, not a finding.

## Verdict
Not ready.

## Must fix before landing
1. **Stray Hermes child binding** (item 1): disconnect or convert the plain-call top-level row of the child's session when `subagent_start` registers it, or apply the worker ranking rule from item 2. Add a test of the full order: early call, then `subagent_start`, then the parent stays the only connected top-level binding and the composer wake fires.
2. **Plain reconnect** (item 2):
   - Never reconnect a binding disconnected as `superseded`, or make the worker supersede plain-call-only bindings that lose to a lifecycle-claimed one.
   - Fix TRUST-POLICY A4's re-supersede sentence and design doc §1 in the same commit.
   - Test a stale caller against a superseded V32-migrated binding that was re-adopted by a plain call.
3. **agy native gate:** run the 15 flows on base and on the branch. Decide whether the `accept` failure is a product regression or a gate flow that needs updating to environment attribution, and fix whichever it is.
4. **Foreground PID evidence:** in an isolated Herdr, check `pane process-info` against the fingerprint PID for Codex (npm and native), Claude, launch-created Hermes and agy. If they don't match, the single-agent native wake is lost.
5. **Prove the fix-round tests catch the defects:** run them against 212dbd0c and confirm they fail. Tighten the `>=` cursor assertions in tests/store/mod_ack.rs and tests/service/lazy_hook_pull.rs, and the smoke test race in ht-54r.81.
6. **Runs:** the full suite with `check-no-leaked-processes --run-id`, `scripts/check-harness-neutral`, and a green Linux CI run.

## Untested scope
- **Full suite:** never run on 20955d16. The last green sweep (abe1bf03) predates ht-54r.76 to .84. The one config9 failure under load (Task 41) is still unexplained.
- **Linux:** the `/proc` fingerprint paths have never been compiled locally.
- **Real harness binaries:**
  - Claude: whether `CLAUDE_PID` matches the foreground PID;
  - Codex: the SubagentStop/SessionEnd payloads, and `CODEX_THREAD_ID` across `/new`, fork and resume;
  - Hermes: the `subagent_*` order against the child's first call, callbacks dropped by the single-flight lock, `/resume`, and lineage on compression;
  - Hermes `/background` side agents share one PID and now supersede the foreground session.
- **Parked escalations:** sandboxes that hide the ancestor PID, and a sibling PreToolUse hook dropping the `--agent-id` rewrite (claude-code issue 83353).
- **ht-54r.7:** the conflict resolution done outside the pipeline (BLOCKED-AUTH).
- **agy native gate:** the 5 failing flows have not been triaged.
- **Not covered by tests:** two live hook-pull bindings sharing the per-seat offer ledger, and a plain read (empty inbox, `thread list --json`) not reconnecting (ht-54r.81 [passive-read-no-reconnect]).

## Deferred OK
- **Spec gaps:** no fast-forward on a continuity reattach; `continuity_candidates` not filtering on reason; no liveness re-check on the poke path (low impact, since a dead PID fails the foreground match).
- **Docs:** stale design-doc Post-Implementation Notes and the missing agy session variable (fix them with the next doc pass).
- **TRUST-POLICY.md:842**, the Hermes "no C1 resume continuity" wording: probably still accurate, since Hermes has no resume-source SessionStart.
- **Cosmetics:**
  - the literal `"superseded"` at delivery_pull.rs:229;
  - TRUST-POLICY line wrapping and splicing (ht-54r.79/.81);
  - the dropped `checked_host_number` range check (ht-54r.78);
  - the config7 edit outside the task's file list, with a stated reason.
- **Performance and earlier smells:** read_cursors full-scan probes, the per-seat hook-pull ledger, reconcile's `active_binding` taking only the newest row, `binding_mail` row cap, first-contact counts per thread.
- **Process:** missing RED runs on tasks the fix round did not touch, deviations from briefs, the leak check run as `stop-run-processes --check` instead of `check-no-leaked-processes`.
- **ht-54r.5:** the Hermes load timeouts; watch them in the full suite.

Relevant files (under /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/session-keyed-seat-bindings/):
- src/store/bindings.rs (151, 708–776, 1264–1385)
- src/store/seats.rs (3418, 3721–3770, 4434–4452)
- src/service/liveness.rs (55–70)
- src/service/delivery_pull.rs (210–296)
- src/notification/dispatch.rs (242–266)
- src/ports.rs (2121–2140)
- src/host/native.rs (1214, 2356)
- src/scheduler/mod.rs (749, 1035)
- src/cli/mod.rs (2695)
- migrations/0033_archival_scan_scope.sql
- TRUST-POLICY.md (A4 Binding liveness)
- docs/superpowers/runs/2026-10-09-session-keyed-seat-bindings/2026-10-10-session-keyed-seat-bindings-design.md (158, 446–447)
- tests/service/cooperative.rs (`hermes_child_cli_call_before_subagent_start_leaves_the_parent_connected`)
- .superpowers/sdd/ht-54r-plan/progress.md
