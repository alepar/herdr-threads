# Final review, cluster 28 (tests-store), epic ht-ma6i

Source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head 0755e609de0328e6d73584dd65eb2854e1fd7bf7, both verified. The caller checkout stayed at head with a clean status through the review.

## Scope and method

All 27 slices (000140 to 000166) were read against the design doc at head. Two files are the bulk of the new assertions on the key table (`continuity_key_decisions.rs`, `continuity_keys.rs`), three on reversible retirement (`reversible_retirement.rs`, `reversible_reads.rs`, `reversible_facts.rs`); `restored_pane.rs`, `control.rs`, `schema.rs`, `fact_convergence_seat.rs` and `service_reads.rs` add targeted cases. The other files only add `continuity_key: None` or unwrap `RetirementStart::into_job`.

Named risks were checked in source: `saved_seats_page`, `poke.rs` `collect`, `reservable_seat_sql`, `decide_continuity`, `rebind_unresolved_seat`, `disconnect_with_hooks`, `liveness.rs`, every `memberships` writer, and the conflict clearing paths.

One review-owned detached worktree at head (now removed) ran two uncommitted probes and a focused run:

- `review_probe_undo_then_keyed_reattach`: real fence, keyed claim held as `key_retired_reversibly` with one alert, real `reversible::unretire`, then a fresh session in another pane reattaches the same seat by key, with the hold cleared, the reversible row gone, no new seat, and `cooperative_continuity_key` recorded. Passed.
- `review_probe_real_process_exit_state_keeps_the_seat`: `disconnect_with_hooks(ProcessExit)`, three passes with the pane shown, keyless and keyed: seat stays resolved, no reversible row, no job. Passed.
- Focused run with `--all-features --lib` on filters `reversible continuity_key restored_pane graph_clone_identity seat_retirement_terminal`: 109 passed, 8.14 s. This is not the full suite, which stays the caller's.

## Requirement baseline trace

- B1 (continuity key): the store half is delivered and well pinned. The verdict table is exercised row by row in `key_verdict.rs` and on a real store at all three entry points, including lag, the cap, kept claims and hc or named hosts. The production entry to the store is `src/identity/repair.rs:1150` (`ContinuityEvidence::Key`). The hook, watch and bridge half leaves this cluster and is the seam reviewer's to trace.
- B2 (pane close retires, undo reverses): delivered at the store. Plan-driven closes reach `RetiredReversibly` in `restored_pane.rs`, and un-retire, finalization and the frozen paths are pinned. The composed undo flow has no committed test, but the probe above passed. The service session half is `tests/service/reversible_session.rs`, outside this cluster.
- B3 (agent exit keeps seat and memberships): delivered by trace. `src/service/liveness.rs` decides `ProcessExit`, and `disconnect_with_hooks` writes liveness only. The `memberships` writers are the invitation paths, `control/membership.rs` and the retirement job (`src/store/control/retirement.rs:510`); none runs on a disconnect. The probe confirms the seat half on the real state. The committed test does not pin what the row promises; see the verdict section below.
- B4 (Herdr unchanged): nothing in this cluster touches Herdr; the store tests use fixtures.

## Test quality notes

The new suites are strong on the whole: most tests name what they kill and carry a control (for example, deleting the `unretired` row in `unretired_seat_does_not_block_hold_lift` to prove the lift can refuse, and the hex pin on the keyless digest in `control.rs`). Temp paths go through `scratch::temp_path`, the `test-support` gating of `reversible_facts.rs` and of `reversible_reads.rs::peer` is right, and no test spawns a child.

The `agent_exit_with_pane_open_keeps_the_seat` defect is narrow: the production behaviour is right, so the repair is test-only.

## Verdict
not ready (B3's only regression test never asserts memberships, mislabels a retirements count as memberships, and fakes the exit state)

## Must fix before landing
- tests/store/restored_pane.rs:690 `agent_exit_with_pane_open_keeps_the_seat` is the only regression test for baseline row B3 and for the ht-ma6i.3 acceptance "its memberships and episodes effective", yet it binds `count(*) FROM retirements` to a variable named `memberships` (lines 690 and 699), seeds no membership or requirement episode in either branch, and at line 682 writes `ended_at=150, disconnect_reason='process_exit'`, a state production never writes for an exit (`disconnect_with_hooks` with `ProcessExit` leaves `ended_at` NULL and records `liveness_process_exit`); test-only fix: rename the count, seed a membership and a requirement episode and assert them effective after the passes and the relaunch, and drive the exit through `bindings::disconnect_with_hooks`. The production behaviour itself held under trace and probe.

## Untested scope
- G2 and G5 (spec 4.1, 4.4): no store test exists because no production code does: ht-ma6i.1 and ht-ma6i.4 are open with no commits (their task worktrees sit at 9d2394467, 9 behind head), and at head `seat_shell_fingerprints` and `restored_target_absence` appear only in migration 0016, the table list at `src/service/kicks.rs:261` and `tests/store/schema.rs`. Neither is a baseline row, but both are in the epic's goal, and ht-ma6i.11 (the spec 4.8 integration test) is open behind them; whether the branch lands without them is the seam reviewer's call.
- The composed flow close, keyed relaunch held, un-retire, keyed reattach (B1 with B2) has no committed test: `close_unretire_reattach_close_is_reversible_twice` (tests/store/reversible_retirement.rs:1047) and `Fx::reattach` (tests/store/reversible_reads.rs:231) go through `rebind_unresolved_seat`, and `Fx::retired_with_key` never un-retires. The uncommitted probe passed; ht-ma6i.11 is meant to cover it and is open.
- Spec 4.3 "a deadline already past at cutover stays past by the same margin": every fixture deadline in `deadlines_keep_remaining_time_including_logical_receipts_and_invitations` is after the cutover, and no fixture shifts a receipt already warned. The flat `deadline_at+?2` in `reversible.rs` reads right but is unpinned.
- tests/store/reversible_retirement.rs:754: the soft-poke assertion cannot fail on the frozen predicate: `reservable_seat_sql` already requires `state='resolved'` and `collect` re-reads the effective receipt, so removing `not_frozen` at `src/store/poke.rs:117` should leave the test green (by reading, no mutation run). There is no positive control that seat s2's row is due, though the comment says it is.
- Alert-only conflict rows: `conflict_clears_when_target_leaves_a_coherent_publication` uses only `KeyLiveElsewhere`, although `clear_conflicts_absent_from` deletes every kind. `clear_alert_only_if_owner_changed` is called only at check-in (`src/store/seats/cooperative.rs:1053`), so an owner that retires or unresolves with no later check-in leaves its row standing against spec 4.2's "clears when `owner_seat_id` stops being the pane's resolved owner"; the row holds nothing but can show on `SeatInspect` and counts against the 256 cap until the pane leaves a publication. No test pins either behaviour.
- tests/store/reversible_reads.rs:600: `window_messages_are_unread_on_reattach` claims to kill "history refused to the returning seat", but `HistoryQuery` carries no caller, so that clause is not exercised.

## Deferred OK
- tests/store/reversible_reads.rs:617 `shift_of_1000_pending_rows_finishes_inside_the_call_budget` asserts a real 3 s budget with no nextest test group; it passed here inside an 8.14 s focused run, and ht-ma6i.10 already records it as a minor to move into a group if it flakes under starvation.
- `reversible_reads.rs::peer` copies about 40 lines of node fixture from `reversible_facts.rs`, recorded as a minor on ht-ma6i.10; a shared helper can follow.
- tests/store/fact_convergence_seat.rs: the doc comment "`seat.retired_reversible` belongs to ht-ma6i.7" is a task-history note in a test comment, harmless now that ht-ma6i.7 has landed.
