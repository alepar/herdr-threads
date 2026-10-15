**Verdict: not ready.** The code matches the spec, including the connected-only views you approved for ht-yha.57. It is blocked by one test this branch introduced that fails under full-suite load with no known cause, 4 commits on main that the branch does not have yet, and no full-suite run that covers the ht-yha.57 change.

## Step 1: my own review against the spec

The review package is `.superpowers/sdd/ht-yha-plan/review-dc2996c3..698e747b.diff` (149 commits, 265 files). I focused on the newest seam, ht-yha.57 (commit 931fe52c), and on how it fits with the earlier view work in ht-yha.44, .54 and .55.

- **Connected-only views fit together correctly.**
  - One rule decides which bindings hold a view: open and connected. The two v47 triggers that add rows (`binding_pending_publish`, `binding_pending_legacy_set`) and the seed in `src/store/lazy_delivery/mod.rs` all use it.
  - `disconnect_with_hooks` (`src/store/bindings.rs:505`) now drops the view on every disconnect, process exit included.
  - Only `connect` (`bindings.rs:564`) sets a binding back to connected, and it re-seeds the view. New bindings are seeded at the end of `read_cursors::seed`.
  - The new seed query only looks at messages after the binding's cursors, joined or invited threads where it has no cursor, and legacy rows. That always includes every row the unread rule can accept, so the seed is exact and does not walk the seat's whole history.
- **No data loss from an empty view.** Cursor advance (`read_cursors.rs:429`) checks whether a delivery row exists in `lazy_recipients`, not whether it is in the view. So a disconnected binding that reads with no view can never move its cursor past rows it was not shown.
- **Hook pulls are safe.** `hook_pull_view_mode` (`src/service/delivery_pull.rs`) reconnects a process-exited binding before it reads.
- **New edge (minor).** A process-exited binding whose plain call carries no liveness fingerprint is served without reconnecting, so it now sees no delivery rows until a lifecycle check-in. Before ht-yha.57 it kept its view. This is covered by the accepted limit in TRUST-POLICY A8, which describes plain calls that do not reconnect.
- **Wake.** `wake_frontier`, `oldest_pending_at` and `has_wake_rows` read only connected top-level views plus legacy rows. That is the same rule main used, so dropping process-exit views loses no wakes.
- **Doc and comment drift (minor).**
  - The spec body (`2026-10-13-message-priority-delivery-design.md:101`) still says "A view is held only by a connected or process-exited binding", which contradicts the ht-yha.57 note at line 318. TRUST-POLICY A8 is correct.
  - `migrations/0047_message_priority.sql` still has the comment "also the repair tail of a pre-amendment v47 store" and two `DROP INDEX IF EXISTS` lines. Those indexes never existed on main, and e6de8153 removed the repair paths, so both are dead.
  - `src/protocol/watch.rs:30` and `:533` still mention an older daemon's `wake_on: all`.
- **Main is 4 commits ahead,** all test-only Hermes and stub-binary fixes. `git merge-tree` merges them cleanly. They target three of the four failures in the last full-suite run (`doctor_classifies_claude_on_path` and the two Hermes capture tests).
- **The branch's own stall test fails under load.**
  - `mod_delivery::stall_with_only_a_p0_delivery_row_hands_over_to_the_native_wake` (`tests/integration/mod_delivery.rs:4044`) failed in the e6de8153 full run, failed 2 of 3 early runs in ht-yha.57, and failed once in ht-yha.46.
  - In the failure output the watch stream shows the message but no `attention` line. A stall can only be detected after an Attention push is recorded (`entry.last_attention_push?` in `stalled_judgement`, `src/service/mod_channels.rs`).
  - `observe_fingerprint` uses up a fingerprint change without pushing when the channel is not yet `Live`. Later readings then see the same fingerprint and never push.
  - So this may not just be a slow test. If a seat has only a p0 delivery row, its stalled mod may never record a push and never hand the seat back to the native wake, which would be a missed p0 wake. I have not reproduced this; it is the most likely explanation from the trace and the code.

## Step 2: ledger triage

- **Recurring minor `[doc-drift]` (×8 across 6 tasks).** Cause: no task owned the shared docs, because each task had a narrow file list. ht-yha.34 and .55 fixed most of it; the remaining items are the three listed above.
- **Recurring minor `[missing-edge-case-test]` (×3 across 3 tasks).** These are test-only gaps and can wait.
- **Repeated pattern: flaky failures waved through without evidence.** Tasks 4, 5, 12, 13, 32, 34, 38, 40 and 44 called failures "flaky" based on a passing rerun, with no run on the base for comparison. In one case this hid a real problem: the branch's own stall test (Task 34 / ht-yha.46 and Task 44 / ht-yha.57 minors, bead ht-3eic, P3) is graded minor but names a failing test with no known cause. I triage it as must-fix.
- **Task 42 q4 FK failure:** fixed in 0873fa12.
- **Task 30 `--max-bytes` 256..342:** fixed by ht-yha.50.
- **Task 15 vacuous `markers_ok` check:** fixed by ht-yha.51.
- **Task 9 wake seek reading `state`:** fixed by ht-yha.44 and .54.
- **Task 12 `pair_accept` failure:** fixed by ht-yha.34; it passed in the e6de8153 run.
- **Task 41 `[residual-history-walk]`:** `ReceiptScanScope::Seat` is still used by `inbox_batch` and `pending_receipts`. This is a cost issue only; file a bead for it.
- **BLOCKED lines** (ht-yha.33, .42, .44, Task 8): all settled by your decisions or by later beads (ht-yha.49, the .41/.42 test rewrites, .52). No `BLOCKED-AUTH` lines and no parked findings.
- **Metrics:** "landed≠completed 43 vs 40" means the ledger is incomplete. Earlier reviews matched the gap to the landings of ht-yha.1, .14 and .5.1, whose tracker closure was unconfirmed. Missing lines there are not evidence of a clean run.

## Verdict
**Not ready.**

## Must fix before landing
1. **Merge current main** (d7c60ddd, 4 commits, clean merge). It should clear three of the four failures from the last full run.
2. **Find the cause of `stall_with_only_a_p0_delivery_row_hands_over_to_the_native_wake` (ht-3eic).** Do not just widen the timeout. First check whether a p0-row-only seat can lose its Attention push when the fingerprint is first read while the channel is not yet `Live`. If it can, fix the product code. If it cannot, make the test wait for the `attention` line before `rig.advance`.
3. **Run the full suite on the merged tip** with `HT_LEAK_RUN_ID` set, then `scripts/check-no-leaked-processes --run-id`. Compare any failure against a run on the base before calling it flaky.

## Untested scope
- **No full-suite run exists for this branch tip (698e747b).** The last one, at e6de8153, came before ht-yha.57 and had 4 failures. ht-yha.57 changed the disconnect path, the view triggers in v47 and the re-seed query, and so far only its own focused tests cover it. The caller's sweep will be the first full measurement; until then treat the branch as untested.
- The branch has never been tested merged with the current main.
- The pre-release wake e2e (`scripts/wake-e2e.py`) has never run against this 1.0.0 candidate. It is required before tagging.
- Untested scenarios:
  - A v46→v47 migration on a realistic store with many open and disconnected bindings, including the view backfill.
  - The re-seed cost after a process exit for a seat with long histories in threads where the binding has no cursor.
  - D2 on the channel path: an item already queued in the mod keeps its `wakes` value after T is raised.
  - Search at 256–342 bytes on a p0 message against the real daemon.

## Deferred OK
- Docs and comments: the stale spec line 101, the migration comment and dead `DROP INDEX IF EXISTS` lines, the "older daemon" comments in `watch.rs`, the `PENDING_SUMMARY_CAP` comment in `nudge_threshold.rs`, and the CHANGELOG line about initial invitations. The spec line is worth a quick fix since it contradicts what you decided.
- `seat_owes_display_sql!` still counts process-exited bindings. It only affects the seat-wide `displayed` mark, which no delivery or wake decision reads.
- The new edge where a process-exited binding without a fingerprint sees no rows until a lifecycle check-in. It falls under the accepted limit in A8.
- Cost items:
  - The cursor-advance trigger costs time proportional to the binding's view size in that thread.
  - `effective_receipt` runs one lookup per hydrated message.
  - The seat-scope receipt walk can still visit settled history (file a bead).
- When several bindings tie for the highest cursor, a new binding inherits display marks from the lowest binding id.
- `LEGACY_PAIR_MARKER` is compiled only into test builds.
- `operator_orphan_invite` stores NULL confirmation; this is documented in TRUST-POLICY.
- ServiceSend writes `delivery_mode='ordinary'` for human-only explicit recipients. It is audit-only.
- Test hygiene: the hard-coded `p0_messages = 4` and `--max-bytes 384`, the leftover capability strip in `lazy_proxy.rs:99`, the weak capability-probe tests, the hand-copied v1 decoder, and TDD and report evidence gaps.
- The `register.js` queue-wide sort, and `drain_pages` dropping buffered items on a later page error. Undisplayed items are offered again, so nothing is lost.