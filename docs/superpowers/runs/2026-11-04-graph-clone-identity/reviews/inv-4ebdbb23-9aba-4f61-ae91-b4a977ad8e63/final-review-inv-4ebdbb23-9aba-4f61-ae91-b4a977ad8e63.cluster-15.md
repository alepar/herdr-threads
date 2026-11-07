# Final review, cluster 15 (src-store), epic ht-ma6i

Source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head 0755e609de0328e6d73584dd65eb2854e1fd7bf7, both verified with `git rev-parse --verify`. Caller checkout observed at the head SHA with a clean status at start and at the end of the review. Read-only: no cargo run, no worktree created, slices and `git show` at the head only. The implementers' store tests were read, not rerun.

## Baseline trace inside this cluster

- B1 (continuity key): the store half is delivered. `continuity_keys` owns reserve, release, bind, conflict rows and the alert feed; `key_verdict::judge` matches every row of the spec 4.2 table, including lag and claimless; `decide_continuity`, `resolve_observed` and `register_available` all act on the one verdict. A keyed claim reattaches an unresolved seat with a fresh native session, and a refused transaction keeps its conflict through the stash. The hook, watch and bridge entry points leave this cluster and are the seam reviewer's to trace.
- B2 (pane close retires, undo reverses): delivered for a key-bearing seat, as the source design's D2 scopes it. `begin_retirement_authored` is the one disposition; every closure route reaches it (`begin_retirement`, `BeginRetirement`, `RetireRestoredPane`). `UnretireSeat` and `FinalizeRetirement` are served from `service_controls::operate`; session gating and dispatch leave this cluster for the seam reviewer.
- B3 (agent exit keeps the seat): no store path here touches memberships on an agent exit, and the spec records no threads change for it.
- B4 (Herdr unchanged): nothing in this cluster changes Herdr.

No baseline row is undelivered inside this cluster.

## Spec body outside the baseline rows

The spec's Goal and sections 4.1 and 4.4 promise G2 and G5, and both have their store half specified in this cluster's files. Neither is present at the head. `git grep` finds `seat_shell_fingerprints`, `restored_target_absence` and `structural_shell_continuity` only in migration 0016 and the kicks table list; `decide_shell_continuity` and `identity::shell_continuity` do not exist; `RestoredPane` has no target-absence variant; every production `RetirementInputs` passes `absence_evidence: None`, so `restored_target_absent` has no producer. `bd` shows ht-ma6i.1, ht-ma6i.4 and ht-ma6i.11 open, and the ledger's last detector round reads "waiting on deps 3".

## Checked and sound

- Deadline shift: the logical `receipt_state` projection matches `effective_receipt_with_key_deadline`, including the historical cutover; the upsert cannot shift a row twice; `send_manifests.message_id` is NOT NULL, so an in-progress preparation cannot break the insert; the sparse due triggers follow.
- Frozen and waiting fragments reach every due scan (physical, sparse, extension lapse, invitation, both poke tables) and every hold or plan site (`saved_seats_page`, hold lift, publish, record snapshot, recovery refusal, health). The scan cursors restart, so a row skipped while frozen is seen after un-retire.
- Facts: live echoes are no-ops; own-log replay and parked-fact replay converge across repeated episodes, because `delete_unretired_row` runs before a second reversible row is inserted; `insert_fact_row` cannot hit the primary key.
- `seats_retirement_terminal` ordering holds on all three paths: seat first on un-retire, row first on finalize, and `retired_seq` is outside the trigger's columns.
- `operator_retire` selects with `LOCAL_SEAT`, so a peer's row cannot be finalized locally. An empty-input local job satisfies the `retirements` CHECK.
- Conflict clearing by key, target and publication matches the spec for hold-bearing rows. `is_continuity_refusal` treats `TargetUnresolved`, `TargetAlreadyOwned` and `ContinuityKeyPending` as definitive.
- Migration 0016 and its rebuild follow the 0012 pattern. No path deletes `seats`, `herdr_hosts` or `service_authors`, so the new foreign keys cannot block a delete.
- Window paths (`project_receipt`, invitee apply, archival, origin warnings) are pinned by `tests/sync/facts_seat_reversible.rs` and `tests/store/reversible_retirement.rs`.

## Verdict
not ready (spec 4.1 G2 and 4.4 G5 have no store implementation at the head; ht-ma6i.1 and ht-ma6i.4 are open)

## Must fix before landing
- Spec 4.1 (G2, ht-ma6i.1, open; not a baseline row): nothing writes `seat_shell_fingerprints` and there is no shell continuity store decision or `structural_shell_continuity` writer in `src/store/seats/continuity.rs`, so after a live handoff a bare, exited-agent or `managed_launch`-only pane seat stays unresolved, which the spec Goal says this epic ends. Land ht-ma6i.1 or record a human scope change.
- Spec 4.4 (G5, ht-ma6i.4, open; not a baseline row): nothing writes `restored_target_absence`, `RestoredPane` and `restored_pane_evidence` in `src/store/seats/observation.rs` have no target-absence variants, and `absence_evidence` is `None` at lines 2770 and 2817, so an unresolved seat whose target never reappears after a restart waits for an operator forever, and the kept key column blocks nothing. Land ht-ma6i.4 or record a human scope change.

## Untested scope
- The spec 4.8 end-to-end run on an isolated Herdr session (reserve, launch with key, close pane, send refused, un-retire, relaunch in a new pane, ACK deadline resumed) is ht-ma6i.11, still open; the store's keyed reattach and the window history pin are covered only by separate store tests, and `window_messages_are_unread_on_reattach` reattaches through `rebind_unresolved_seat` with seeded cursors rather than a keyed claim.
- No test asserts when a soft poke fires after un-retire; `deadlines_keep_remaining_time_including_logical_receipts_and_invitations` checks only `deadline_at`.
- No test covers a keyed claim whose bound seat sits on another Herdr server that is still lagging.
- This review ran no tests at the head; the ledger defers the full-suite measurement to the caller.

## Deferred OK
- `src/store/seats/reversible.rs:320` `shift_pending_deadlines` moves `deadline_at` but not `available_at`, so `poke.rs:165` computes `soft_point` over a window inflated by the frozen time: with the default 0.6, a 10 minute ACK with 1 minute elapsed at cutover and a frozen stretch over 12.5 minutes is due for its soft poke as soon as the seat reattaches, not at 60 percent of the time it has left. The hard deadline is correct and it is one extra reminder per seat; shifting `available_at` by the same delta in that function fixes it, with a test.
- `src/store/seats/continuity.rs:588` reads lag on the pane's host, not the bound seat's: a key whose seat is on another server that has restarted but not yet reconciled reads resolved, so the claim records a false `key_live_elsewhere` hold and one alert instead of `ServiceBusy`, and an idle agent there waits for its next event instead of being decided as a kept claim. It clears on that next event and needs two servers on one node; also testing `reconciliation_lags` on the seat's host closes it.
- `src/store/continuity_keys.rs:370` clears an alert-only row only at a check-in by another seat (`cooperative.rs:1053`), so an owner retired by the operator while its pane stays open and no successor checks in leaves a stale `pane_owned_by_other_seat` on the key seat's `SeatInspect`, against the spec's "clears when `owner_seat_id` stops being the pane's resolved owner". It holds nothing and clears at the next check-in, rebind or publication without the pane; clearing rows naming the seat in `retirement_transition` would close it.
