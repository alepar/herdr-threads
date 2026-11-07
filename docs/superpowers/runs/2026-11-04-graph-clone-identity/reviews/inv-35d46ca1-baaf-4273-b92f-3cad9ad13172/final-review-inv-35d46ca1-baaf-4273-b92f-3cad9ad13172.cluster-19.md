# Final review, cluster 19: src-service (ht-ma6i)

Source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head 1b058307490f29ece8bb4e3fe7a862f8356d992f, both verified with `git rev-parse --verify`. The caller checkout read clean at that head throughout. Files: `src/service/dispatch.rs`, `src/service/kicks.rs`, `src/service/launch_gate.rs`, `src/service/workers.rs` (slices 000087 to 000090). Everything here is a static read of that source; no test was run by this review.

## What the cluster changes

Twenty changed lines. Seven `continuity_key: None` in `dispatch.rs` (four production bootstrap resolves, three in `operator_tests`), one in `launch_gate.rs`, six table names in `KNOWN_UNMAPPED`, and two calls on the observation lane.

## Requirement baseline trace

- B1 (continuity key): the three keyed commands pass through dispatch unchanged. `Command::ContinuityCheckIn` goes to `host.identity().continuity(continuity, budget)`, whose `claim_evidence` selects `ContinuityEvidence::Key`. `Command::ResolveSeat` goes to `identity().resolve(request, ..)`. `Command::CheckIn` goes through `PermitMutation::try_from(command)` with its struct intact. Nothing in dispatch drops or rewrites the field. Delivered as far as this cluster carries it. Client production of the key and the store verdict belong to their own clusters.
- B2 (pane close retires, graph undo reverses): no service file in this cluster handles the new operations; `ReserveContinuityKey`, `UnretireSeat` and `FinalizeRetirement` land in `src/store/service_controls.rs`. A reversible retirement has no `retirements` row, so the deadline lane's job advance has nothing to do until finalization inserts one, and `retirements` is already a deadline table. That path leaves this cluster and is the seam reviewer's to trace.
- B3 (agent exit keeps seat and memberships): no change here, as the spec states.
- B4 (Herdr unchanged): the worker's new passes use the existing `pane process-info` read and `observe_current_target`. Nothing here requires a Herdr change.

## Keyless service resolves

The spec says `ResolveSeat` presents a key only for the caller's own pane, never for `launch` resolving another pane. All five production sites build a resolve for a pane the daemon or launcher created, not the caller's, so `None` is correct. In the four bootstrap sites the value only feeds `with_bootstrap_observation`'s guard. `launch_gate::allocate_seat` is a real allocation, and with no key its `ordinary_resolution_digest` keeps the `resolve_seat` domain, so a replay across the upgrade matches. Serde omits a `None` key, so the wire bytes hold too (`continuity_key_is_absent_from_the_wire_when_none`). Ordinary resolution still applies the hold beside the restore-hold check, so a created pane is treated like any other.

## Kick classification

All six tables that delta 16 creates are listed; `allocation_decisions_new` is renamed back to the already classified name in the `_after` script, and `kept_continuity_claims` gains a column but keeps its entry. No table was found that needs a lane. For each new table, I checked whether a commit touching only that table could leave a lane sleeping past work it should do:

- Un-retire shifts `receipts`, `receipt_state`, `invitations` and writes `seats`, all mapped, so the deadline lane re-scans once a frozen row is due again. A pending row with no deadline has nothing to schedule.
- Finalization inserts `retirements`, a deadline table. On a peer, where the seat row may not change at all, that insert is still what kicks.
- The fence writes `seats` and `occupant_bindings`, which reach the wake lane and the mod worker. A frozen row dropping out of a due scan needs no kick: the lane wakes and finds nothing.
- `continuity_keys`, `continuity_key_conflicts` and `continuity_alerts` feed no lane; a held pane is re-presented by the hook's next event. `seat_shell_fingerprints` and `restored_target_absence` are written and read on the observation lane.

The observation-origin rule in `kicks_for_commit` removes nothing for that lane, so a seat rebound by `decide_shell_continuity` still kicks wake and mod.

## Observation lane wiring

The two calls follow `decide_process_continuity` in the order §4.1 requires, inside the branch that requires a completed pass and `record_reconciliation_pass` returning `Ok(true)`. `Ok(true)` means the publication is current, not that the marker moved, so a steady pass still runs them; the steady capture cadence is 5 s, and §4.1's "first pass after allocation" holds.

§4.1 says the allocation path kicks the lane. No code was added for it, and none is needed: `resolve` reaches `with_read_inner` with `quiet` false, which calls `request_explicit_capture` and `kick_explicit` after the decision commits.

Ignoring the results with `let _ =` matches its three neighbours and fits the spec's retry-next-pass rule. A failed record does not stop the decision, and a failed process pass does not hand its seats to the shell path by mistake, because `judge_shell_candidate` re-checks for a live undeclined process candidate.

Cost stays bounded, per ht-kaw9. Each call has its own 3 s `maintenance_budget`, each read at most 750 ms, at most 16 and 32 rows per pass, checked between rows. A stalled host publishes nothing, so these passes do not run against it. Cancellation is the session's `stop`, set through `with_service_cancellation`, so the worker's join is prompt. The new calls take no lane lock across a host read of their own.

The non-Herdr sessions run the same two calls, and `SHELL_SEAT_SCOPE` (`h.kind='herdr'`) makes each one query that returns nothing. Native Herdr takes the default `ClosureAuthority::Closing`, so that scope matches §4.1's "closing Herdr host".

## Coverage observed

`tests/clone_identity_sweep.rs::live_handoff_with_mixed_panes` runs the real worker end to end: a real daemon, a private Herdr, bare and exited panes decided `structural_shell_continuity`, and a live pane decided `structural_process_continuity` with no hook of its own. CI's full-suite workflow installs `herdr`, so the test does not skip there. `tests/store/commit_kicks.rs` enforces the table classification against the real schema.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- No test pins the worker's gate on the two new calls: `tests/service/observation_lane.rs` calls `record_shell_fingerprints` and `decide_shell_continuity` directly, and only the sweep passes through `start_observation_worker_named`, so nothing asserts they stay silent after an incomplete pass or a stale publication (`Ok(false)`).
- This review ran no tests and the ledger records the full-suite measurement as deferred to the caller, so `commit_kicks` and `live_handoff_with_mixed_panes` are read as correct at this head, not observed passing here.

## Deferred OK
- Stale comment at `src/service/dispatch.rs` above `Command::ContinuityCheckIn`: it says the hook sends this command "only for a top-level resume", but since 227af9625 a keyed lifecycle event sends it too (`probes_resolved_pane` in `src/cli/hook.rs`); behaviour is unaffected because no person seat runs a hook, and the fix is a comment edit.
- Imprecise comment at `src/service/kicks.rs:259`: "Their writers change seats or attention in the same transaction" does not hold for `record_shell_fingerprint`, `ReserveContinuityKey` or conflict and alert writes, which touch neither; the clause after it, "these tables schedule nothing", is the accurate reason and could stand alone.
- For the identity cluster's reviewer, plausible and not reproduced: `record_shell_fingerprints` reads `pane_shell_pid` unfenced (`expected_boot` and `expected_epoch` are None) after the pass ends, and the store's CAS compares only seat columns, so a cold Herdr restart between the capture and that read could record the new shell under the old incarnation, and the next pass could then judge it `Continuous` against §4.1's "a cold restart cannot match"; the window is one pass, needs a seat with no row, and rebinds only to the seat's own saved target, and fencing the read on the published boot would close it.
- Marker-held passes also hold back recording: a non-excludable refusal that keeps `pass_finished` false stops fingerprint recording for every seat on that host, so a handoff during that stretch leaves new seats unmatched; this is the same gate kept claims and ht-hsyg use, and such a seat waits for an operator as §4.1's no-row limit already accepts.
