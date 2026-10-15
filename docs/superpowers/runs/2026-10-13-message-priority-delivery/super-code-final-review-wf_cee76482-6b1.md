**Verdict: not ready yet.** The code holds together and matches the spec, and I found no blocking defect in the areas I checked. What stops it landing is that the branch has never been run as a whole: no full-suite run covers the tip `f3e28996`, and main is still 4 commits ahead.

## Step 1: my own review against the spec

The review package is `.superpowers/sdd/ht-yha-plan/review-dc2996c3..f3e28996.diff` (154 commits, 267 files). At 5 MB I could not read all of it, so I read the spec and the inbox-merge subspec, then checked the places where tasks meet, concentrating on what landed since the last final review (ht-yha.57, .58, .59).

- **Per-session pending views fit together.**
  - Rows enter a view only for open, connected bindings: the publish and legacy-set triggers in `migrations/0047_message_priority.sql` and the seed in `src/store/lazy_delivery/mod.rs` all use that rule.
  - Every disconnect drops the view (`bindings.rs:505`), and the one reconnect path re-seeds it (`bindings.rs:571`). New bindings are seeded last in `read_cursors::seed`.
  - Cursor-advance, display-mark, legacy-clear and binding-end triggers remove rows as expected.
  - The only places that delete `lazy_recipients` rows (`materialization.rs:802`, `staging.rs:88`) touch unpublished rows only, so the foreign key from `binding_pending` cannot fail in production.
- **Wake follows the spec.** `wake_frontier`, `oldest_pending_at` and `has_wake_rows` seek each connected top-level view once per admitted priority. A deep backlog of p1 rows cannot hide a p0 row; `p0_row_behind_1001_p1_rows_wakes` covers this. `reserve` re-reads the current threshold, which is the D2 rule. The extra served-in-turn check for `lazy` wakes decides from the woken binding's own cursor and marks.
- **Sending follows the spec.**
  - `resolve` in `src/cli/send_priority.rs` and `validate_delivery_options` in `protocol/commands.rs` enforce D3 and D4 on both the CLI and the daemon side.
  - `stage_recipient` gives receipts only to non-human explicit ACK recipients (D9). Human recipients get a p0 row, and only p0 row sends keep the unavailable-recipient warning.
  - `resolved_priority` is the single place an old journaled intent's `delivery_mode` becomes a priority (D10).
- **The ht-yha.58 stall fix is sound.** `notify_binding` now records the push time as max(caller's clock, clock at the push), so anything the push announces counts as covered. This closes ht-3eic, the missed p0 wake. The small over-coverage it allows lasts milliseconds and the next pass pushes again.
- **Checks I ran on the tip:**
  - `cargo clippy --locked --all-targets --all-features -- -D warnings`: passed.
  - `cargo fmt --check`: passed.
  - `scripts/check-harness-neutral`: 0 violations.
  - `scripts/check-default-features`: passed.
- **New minor findings (none blocking):**
  1. The `binding_pending_publish` trigger accepts any `invited` membership, but the exact unread rule (`binding_lazy_scope_sql!`) accepts `invited` only for a pending direct-pair episode. So a view can hold a row that readers reject after the seek. That costs one wasted visit per seek and clears itself when a cursor is inserted. No wrong output.
  2. `STALL_ROWS_HEAD` in `seats.rs:5213` reads every row of every connected top-level view on each stall check. The cost grows with view size.
  3. Main is 4 commits ahead (`d7c60ddd`, test-only Hermes and stub-harness fixes). `git merge-tree` merges them cleanly. The previous final review asked for this merge and it has not happened.

## Step 2: ledger triage

- **Recurring minor `[doc-drift]` (×8 across 6 tasks).** The cause is that no task owned the shared docs. ht-yha.55 and .59 fixed it: spec line 101, the dead lines in the migration and the `watch.rs` comments are all updated. What is left is cosmetic: an over-long comment line in `watch.rs` around line 540, a misplaced doc comment in `output.rs` around line 423, and a stale `PENDING_SUMMARY_CAP` comment in a test.
- **Recurring minor `[missing-edge-case-test]` (×3 across 3 tasks).** Test-only gaps; they can wait.
- **Flaky failures waved through on a single rerun.** This happened in ht-yha.38, .40, .44, .50, .53 and elsewhere. The pattern already hid one real product bug, ht-3eic, which is now fixed by ht-yha.58 with a root cause and a regression test. The other "flaky" notes (Hermes hook tests, `doctor_reports`, `startup_enrollment_hermes_steady_turns_make_no_resolver_call`) are unproven until the sweep runs on the main-merged tip.
- **Settled items:**
  - The `[unrun-default-features-check]` minors (ht-yha.44, .52) are settled: I ran the check and it passes.
  - The ht-yha.54 foreign-key fixture failure was fixed in `0873fa12`.
  - The BLOCKED lines (ht-yha.33, .42, Task 8, ht-yha.44 config5) were all resolved by your decisions (ht-yha.49, the approved test rewrites) or by ht-yha.52.
- **No `BLOCKED-AUTH` lines, no parked findings.**
- **The ledger is incomplete.** The Finish metrics are UNAVAILABLE, and the last valid count was landed 43 vs completed 40 (ht-yha.1, .14 and .5.1 have unconfirmed tracker closure). So a line missing from the ledger does not mean that work was clean. The tracker does show every ht-yha bead closed.
- **`[residual-history-walk]` (ht-yha.52).** `ReceiptScanScope::Seat` is used the same way on main, so this is an existing cost, not a regression. The follow-up bead the previous review asked for was never filed.

## Verdict
**Not ready.** I found no Critical or Important code defect. Landing waits on the first full-suite measurement of the merged tip.

## Must fix before landing
1. **Merge main (`d7c60ddd`) into the branch.** It merges cleanly, and its test-only fixes target 3 of the 4 failures from the last run (at `e6de8153`).
2. **Run the full suite on the merged tip** (`nice cargo nextest run --locked --all-targets --all-features`) with `HT_LEAK_RUN_ID` set, then `scripts/check-no-leaked-processes --run-id "$HT_LEAK_RUN_ID"`. For any failure, compare a run on the base before calling it flaky. Nothing that landed since `e6de8153` has been through a full run: ht-yha.57 (disconnect and re-seed paths, v47 triggers), ht-yha.58 and ht-yha.59.

## Untested scope
- **No full-suite run of this branch tip exists.** The caller's sweep will be the first. Until it passes, treat the branch as untested.
- The branch has never been tested merged with current main.
- The pre-release wake e2e (`scripts/wake-e2e.py`) has never run against this 1.0.0 candidate. It is required before any `v*` tag.
- These scenarios have no coverage:
  - The v46→v47 migration and view backfill on a large real store with many bindings still recorded as connected.
  - The re-seed cost after reconnect for long backlogs.
  - D2 on the mod channel path, where an item already queued in the mod keeps its old `wakes` after the threshold is raised.
  - Search at 256–342 bytes on a p0 message against the real daemon.
- Several tasks reported filters that matched no tests or had no captured red step (ht-yha.45). Their evidence rests on the sweep.

## Deferred OK
- The extra `invited` rows in `binding_pending_publish` (wasted visits only, clears itself).
- The `STALL_ROWS_HEAD` full-view read and the O(view size) cursor-advance trigger (cost only).
- `seat_owes_display_sql!` still counting process-exited bindings. It only affects the seat-wide `displayed` mark, which no delivery or wake decision reads.
- The A8 accepted limit: a disconnected process that calls without reconnecting sees no delivery rows. It is documented in TRUST-POLICY and was your decision.
- The ht-yha.58 push-stamp over-coverage (milliseconds; the next pass pushes again).
- ServiceSend writing `delivery_mode='ordinary'` for human-only explicit recipients (audit only).
- The seat-scope receipt walk, which is on main too. A follow-up bead should still be filed.
- Test and evidence hygiene:
  - The hand-inserted stale `binding_pending` rows in `tests/store/lazy_inbox.rs:1703`.
  - The capability strip still in `tests/support/lazy_proxy.rs:99`.
  - The file-list deviations, comment wrapping and stale test comments.