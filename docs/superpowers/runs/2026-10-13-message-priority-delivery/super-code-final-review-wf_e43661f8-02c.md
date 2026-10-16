**Verdict: not ready.** No full-suite run exists for this branch: the caller runs that sweep after this review, so the branch is untested as a whole. The pre-release wake e2e has not been run either.

## Step 1: my own review against the spec

**F1 (Important, confirmed by reading the code): a stale disconnected binding keeps every delivery row pending.**
- **What changed.** Fix commit a815a211 (ht-yha.32) changed the shared macro `seat_unread_sql!` in `/Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/src/store/lazy_delivery/mod.rs:108`. It used to count only connected top-level bindings; it now counts every open one. The roast's fix hint only needed `MARK_DISPLAYED_SQL` to change. The macro is also used by:
  - the wake seeks `wake_frontier`, `has_wake_rows` and `oldest_pending_at` (`src/store/lazy_delivery/wake.rs`);
  - the seat-level digest (`attention::pending_lazy`, `seat_lazy_pending`);
  - the seat filter of the ordered row seek (`lazy_delivery/inbox.rs`).
- **It contradicts the spec.** §Ordering says the seat-level wake frontier and digest "keep today's `seat_unread_sql!` meaning ('… some connected top-level binding …')".
- **Why it matters.** TRUST-POLICY.md (lines 200-201, 241, 258) says enrollment never ends another binding and that disconnection (process exit, end event, superseded) never ends one. So every earlier session of a seat leaves an open, disconnected top-level binding with a stale cursor. That binding finds every later row in the seat's joined threads unread. The consequences, for practically every seat that has restarted its harness:
  - (a) `MARK_DISPLAYED_SQL` (`completion.rs:69`) never flips `state`. Rows stay in the `state='pending'` partial indexes for good.
  - (b) The wake frontier never clears. Take a p≤T row a busy session already got in-turn: when that session goes idle, the frontier is still above `last_lazy_ordinal`, so it gets nudged for something it has already seen. At T=-1 this happens after every message. That breaks Delivery contract rule 4 and the e2e promise "never nudged, during or after the turn".
  - (c) `seek_pending` charges every row the reader already displayed as a visit against `CANDIDATE_LIMIT`=100, starting from (-1,0,0) each time. Once about 100 such rows pile up, CLI `inbox` and `hook_inbox` first pages come back with no rows plus a `next:` line. Delivery pull walks the whole history (up to `MAX_DRAIN_PAGES`=4096) on every non-Unchanged pull.
- **Evidence.** The test a815a211 added, `tests/store/binding_lazy.rs` `a_disconnected_sibling_still_owes_…`, shows the mechanism: state stays pending while a disconnected binding exists. No test combines a stale binding with wake-after-turn or with backlog volume. The commit message says "wake.rs untouched", but wake behaviour changed through the macro anyway.

**F2: the design leans on the seat-wide `state` column again (one problem, several symptoms).**
- The v47 partial indexes filter on `state='pending'`, so the seat-wide state column now decides delivery again. Before, the V34 rule meant predicates read only cursors.
- Ledger lines 80, 81, 133 and 188 and the roast's two seat-pending findings are all symptoms of this. Each fix has moved the defect rather than removed it.
- The doc comment at `src/store/lazy_delivery/inbox.rs:389` still claims state is never read.
- The fix needs one deliberate rule for "who still owes a display" that is cheap for wake and the seek, recorded in TRUST-POLICY.md. Patching the macro again would not be enough.

**F3: model-visible text still describes pair invitations (pre-D7).**
- `integrations/skill/SKILL.md:17` ("Automatic handoff creates the initial invitation with the pair")
- `SKILL.md:199` ("Ensure creates the pair with its initial invitation")
- `src/cli/help.rs:151` ("use Ensure for initial invitation only")
- New pairs auto-join, so agents reading this will expect an invitation and an accept step.

**F4 (minor): `nudge-threshold` treats a failed capability probe as an old daemon.**
`src/cli/nudge_threshold.rs` uses `supports_capability`, which reads a failed probe as "old daemon". The result is a misleading "does not support" error, or a set routed through the legacy `SetWakeOn`. This is the same class ht-yha.36 fixed for send. It does not lose any value.

**Checked and found consistent:** v47 migration and triggers, `DeliveryPriority`, `NudgeThreshold`, `DeliveryOrderKey`, send staging under D9 (human ACK recipients get p0 rows; the p0 unavailable-recipient warning is kept), D2 re-judging on a threshold set, pair auto-join through `record_join`, the invitation policy, the CLI priority rules, the old-daemon shape and probe, `batch_key` with its JS mirror, and the mod's passive/wake rule.

## Step 2: ledger triage

**Recurring problems, by class:**
- **Doc drift ×3 (ht-yha.13, .8, .11), and in practice more.** Each task's file list left out the shared docs, so drift was left for "an owner". ht-yha.34's sweep closed most of it but missed F3.
- **No RED test evidence:** tasks 12, 13, 20, 21 ("tdd-not-red-first", "tests-after-implementation", "red-phase-unshown", "tdd-evidence-gap").
- **Flakes claimed without base-run evidence:** tasks 4, 5, 12, 13. Only the full-suite sweep can settle these.
- **Changes outside the task's file list:** tasks 9, 13, 25. Task 13's was the seat-wide state flip itself (see F2).

**Minors that were really defects:**
- Task 9's `[state-dependent-wake-seek]` names a missed wake, which is a product defect. It went through 5.4 and a815a211 and became F1.
- Tasks 2 and 12 recorded failing tests as "pre-existing". 5170075c fixed them, so they are closed.

**Parked or blocked items:**
- ht-yha.33 is BLOCKED and still open: an old CLI cannot follow a new daemon's `ib2:` format-2 `next:` cursors. That breaks the D10 promise that reads stay old-readable. The user has to pick option a, b or c (or another scheme).
- ServiceSend keeping receipts for its whole audience (`AckAudience::Everyone`, against D9) was parked for the user. It has not been recorded in TRUST-POLICY.md or the spec.

**Ledger completeness:** "landed≠completed 25 vs 22" means the ledger is incomplete, so missing entries do not prove an area is clean. merge-failed 1 was ht-yha.1's false blocker, already resolved. Task 8's BLOCKED fix pass was about missing evidence in its report, not about code. There are no BLOCKED-AUTH lines.

## Verdict
Not ready. F1 is a broad delivery and wake regression for any seat with a past session. One compatibility decision (ht-yha.33) and one D9 decision (ServiceSend) are still open. No full-suite or wake-e2e run exists.

## Must fix before landing
1. **F1/F2.** Define "a binding that still owes a display" once, so a disconnected stale binding neither keeps rows in the pending index nor holds up the wake frontier. Possible directions: connected-or-recently-reconnectable bindings only, a per-binding lower bound on the seek, or keep connected-only for wake/digest and handle reconnects separately. Record the rule in TRUST-POLICY.md. Add tests for: a stale binding plus wake after an in-turn display (no nudge), and a stale binding plus more than 100 displayed rows (a first page still shows the new row). Fix `inbox.rs:389`.
2. **ht-yha.33.** The user decides a, b, c or another scheme, then it is implemented or recorded in §Compatibility with an old-client test.
3. **ServiceSend vs D9.** The user decides, and the decision goes into TRUST-POLICY.md and the spec.
4. **F3.** Fix the model-visible text in `SKILL.md:17`, `SKILL.md:199` and `src/cli/help.rs:151`.
5. **Full-suite sweep at HEAD** and `scripts/check-no-leaked-processes`. Also settle the flake claims made without base runs (tasks 4, 5, 12, 13).

## Untested scope
- No full suite on HEAD e1bc9b9f: the sweep is deferred to the caller.
- The wake e2e (the release gate, needs real agents and a person) has not run against the new matrix. Its `markers_ok` check can pass vacuously (ledger line 110), so fix that before relying on it for a release.
- Optional Linux e2e: not run.
- Untested combinations: stale disconnected bindings with wake or with backlog volume (F1); the mod's passive/wake split by priority in the cross-surface fixture (line 152); pair first send only at p1 (line 145).
- The capability sweep test checks the `ADVERTISED` constant, not what the running daemon actually returns (line 157).
- ht-yha.33's scope (old CLI continuation) has no implementation and no test.

## Deferred OK
- Test-shape minors: lines 12/46, 13/47, 28-31, 42, 79, 146, 147, 158, 160, 161.
- Code-hygiene minors: lines 64, 65, 74, 95, 115, 135, 196, 197, 201.
- `LEGACY_PAIR_MARKER` test-only branch (121). Remove it with the last pre-upgrade fixture.
- `operator_orphan_invite` storing a NULL confirmation (122): documented.
- Human handoff still asking for the Lazy policy (89/123). It only matters against old daemons and matches the spec.
- Skipped receipts-capability probe for priority-shaped plans (86): every daemon with `send.priority_v1` already has it.
- `register.js` sorting the whole queue (116).
- `drain_pages` error path (114): superseded by ht-yha.31.
- Remaining doc-comment drift (159): mostly swept in 5170075c.
- Unbounded wake probes for seats with no binding that can pass the filter (roast punch list): a performance issue, but revisit it together with F1.
- `mod_stall_oldest` still reading `delivery_mode='ordinary'` (`src/store/seats.rs:5225`): equivalent today, because a receipt implies the ordinary mode.
- The `nudge-threshold` probe (F4).