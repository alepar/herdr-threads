# Final review: cluster 20 (src-store), epic ht-ma6i

Source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head 1b058307490f29ece8bb4e3fe7a862f8356d992f. Both verified with `git rev-parse --verify`. The caller checkout was at the head SHA with a clean status at the start and when last sampled during the review. All reads were from slices 000091 to 000115, the spec, and `git show <head>:<path>` for named risks. No test was executed: the read is static, and the owners' test evidence was not repeated.

## Requirement baseline trace

B1 "Continuity key: approved." Store side delivered. `operate` routes `ReserveContinuityKey`, `ReleaseContinuityKey` and `ContinuityAlerts` to `store::continuity_keys`. `key_verdict::judge` holds the 4.2 table as one pure function, and `key_decision` gathers its facts for the three callers: `decide_continuity` (claim), `resolve_observed` (claimless) and `register_available` (check-in). The keyed branch of `decide_continuity` rebinds through `rebind_unresolved_seat` and opens the successor binding through `resolve_claim`, so a fresh session with no matching binding reattaches. A refused transaction's conflict leaves as a stash and is recorded after rollback. `TargetUnresolved`, `TargetAlreadyOwned` and `ContinuityKeyPending` are all definitive under `is_continuity_refusal`, and `ServiceBusy` is kept. The kept column feeds `kept_claim_blocks`. The client presentation and the hook flow leave this cluster and belong to the seam reviewer.

B2 "Pane close retires immediately; graph undo reverses it." Store side delivered. `begin_retirement_authored` is the one choice: a `Closed` retirement of a key holder runs `retirement_transition`, writes the reversible row and authors `seat.retired_reversible`. All three production `Closed` routes reach it: `begin_retirement` (closure proof), `BeginRetirement` and `RetireRestoredPane`/`RetireAbsentTarget` in `apply_reconciliation_transition`. `unretire` restores the seat with no target, shifts deadlines and leaves memberships and episodes untouched. `finalize` inserts the postponed job with the original cutover and an advanced `retired_seq`. The retirement worker finds the job through `pending_retirement_jobs`. The service wire and session level leave this cluster.

B3 "An agent exiting while its pane stays open keeps the seat and its memberships." No store change is needed: the key's seat stays the pane's owner and a returning keyed agent judges `NoOp`.

B4 "Herdr will not be changed." Nothing in this cluster touches Herdr.

## Lifecycle checks that held

- The terminal trigger in delta 16 is relaxed only for a `reversible` row, and both `unretire_local` and the peer path in `unretire_by_fact` update the seat before moving or deleting the row. `finalize` and `finalize_by_fact` delete the row first, but they update only `retired_seq`, which the trigger does not watch.
- `shift_pending_deadlines` matches the send-time projection in `effective_receipt_with_key_deadline`, including the historical cutover spelling. The conflict clause cannot shift a logical row twice, since it updates only a pending row with a NULL deadline. The update trigger on `receipt_state` covers `deadline_at`, so `receipt_due_sparse` follows.
- A window fact keeps a deadline: for a frozen recipient with no state row, `effective_receipt_with_key_deadline` takes the projection arm, so `project_receipt` writes `pending` with a non-null deadline that the later shift moves. A receipt-timer job running over the same row keeps it through `COALESCE`.
- `advance_retirement` settles window rows: the logical phase compares against the advanced `retired_seq`, and the invitation and physical phases are not bounded by cutover.
- Fact order holds in the parked replay case: reversible and unretired are no-ops on a finally retired row, and `episode_fact_applied` absorbs a re-issued copy.
- Waiting seats are skipped where the spec names them: hold lift, `resolution_recovery_refusal`, `unresolved_seat_summary`, `saved_seats_page`, and `publish_snapshot_stage`. `reconcile_invalidated_page` passes over every unresolved seat, so an invalidation never marks a waiting seat. `has_nonretired_seat` still counts one, which can arm a hold on a new incarnation that the next pass lifts: harmless.
- Archival: a held-open member reaches `scan_members`, fails the `qualified` read (it requires `s.state='resolved'`) and blocks the thread, as the spec wants.
- G5 cannot count a shown target, a blocked kept claim or an unreconciled host, and the applier re-derives the evidence inside its transaction. The restored-pane actions return from the CAS block before the `unreachable!` arm.
- Keyless digests are byte-identical in all three keyed sites, and the permit path uses `check_in_digest`, so permit and transaction agree.

## Findings

- none

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- This review executed no tests: every statement above rests on reading the diff and the head source, and the store suites were read for coverage only.
- No store test exercises a keyed reattach or lag across two Herdr hosts: `tests/store/continuity_key_decisions.rs` attaches only a second `Named` host (line 957), so the `host_id` move in `rebind_unresolved_seat` under `Reattach` is unpinned for that case.
- `clear_alert_only_if_owner_changed` with `owner = None` is covered only as a unit call in `tests/store/continuity_keys.rs`; no production path reaches that form.
- No test covers a hold-bearing conflict row across `ReconfirmStructure` followed by the owner leaving the pane.

## Deferred OK
- Alert-only rows clear lazily: the only production caller of `clear_alert_only_if_owner_changed` is the owner's check-in in `src/store/seats/cooperative.rs:1053`, so when owner T retires or unresolves while the pane stays shown and no new owner checks in, `SeatInspect` on the key's seat keeps showing `pane_owned_by_other_seat` against spec 4.2 ("clears when owner_seat_id stops being the pane's resolved owner"); it is display only, since the kind never holds and publication or rebind still clears it.
- A hold-bearing row survives `ReconfirmStructure` (`src/store/seats/observation.rs:2803`, which does not go through `rebind_unresolved_seat`): it is inert while the reconfirmed seat owns the pane, but if that seat later leaves a pane that stays shown, the old row holds again with no re-judgment until a claim, operator action, key release or publication clears it; this needs a conflict recorded during a host-invalidation window, and spec 4.2's clearing list does not name this case.
- A C1 reattachment whose claim carries a free key leaves it unbound (`a_claim_with_a_free_key_decides_as_a_keyless_claim`), and the hook sends no check-in after a reattachment, so the key binds only at the next lifecycle check-in or a tool event whose digest advanced; a pane close before then retires that seat finally and `UnretireSeat` answers `Unauthorized`; the documented route for an existing seat is reserve-with-seat, so this is an off-path edge, and the hook half is the seam reviewer's to trace; binding in the reattach transaction would close it.
- `key_decision` reads `reconciliation_lags` for the pane's host, not the key seat's host: a keyed claim on another Herdr server while the seat's own server lags records `key_live_elsewhere` rather than `ServiceBusy`; spec 4.2 names `reconciliation_lags(host)` and a later claim re-judges the row, so this affects only multi-server nodes.
