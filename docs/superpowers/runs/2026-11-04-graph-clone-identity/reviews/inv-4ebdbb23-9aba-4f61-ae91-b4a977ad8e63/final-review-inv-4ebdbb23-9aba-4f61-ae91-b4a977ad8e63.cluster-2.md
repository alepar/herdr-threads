# Final review, cluster 2 (docs-superpowers), epic ht-ma6i

Source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head 0755e609de0328e6d73584dd65eb2854e1fd7bf7; both verified. This review ran read-only and executed no tests. The caller checkout was observed at the reviewed head with a clean status.

Inputs: the 13 run documents the cluster lists (design spec, two design roasts, step-back record, coverage and findings ledgers, friction log, run record, task-tree snapshot, INDEX row), the source design `herdr-graph/docs/reviews/2026-10-29-seat-review-H2-identity-design.md`, and beads. Source files were opened only to trace baseline rows and to check named spec claims.

## Delivery state against the spec

The spec's Goal has four clauses. Two have no implementation at the reviewed head.

- Live handoff for every unnamed non-person pane seat (§4.1, ht-ma6i.1): bead open. `src/identity/shell_continuity.rs` does not exist; `decide_shell_continuity` and `structural_shell_continuity` appear nowhere in `src`; `seat_shell_fingerprints` exists only in migration 0016 and the table list in `src/service/kicks.rs`. Branch `task-ht-ma6i.1` is 0 commits ahead and 9 behind.
- A seat whose pane never came back stops waiting (§4.4, ht-ma6i.4): bead open. `restored_target_absence` has the same shape, a migration table and a kicks entry, with no writer. `RestoredPane` has no target-absence variant, and `restored_target_absent` appears in `src` only as a passthrough in `reversible.rs:216`. Branch `task-ht-ma6i.4` is 0 commits ahead and 9 behind.
- The integration sweep (§4.8, ht-ma6i.11) is open, and its dependencies .1 and .4 are open with it.

The ledger explains how this happened. Tasks 5 and 9 were held on review bead ht-ma6i.16; the detector's round 2 reported 0 ready with 3 waiting on deps; the run then went to Finish with "completed 8". ht-ma6i.16 closed at 2026-11-05T10:34:30Z, and `bd ready` now lists ht-ma6i.1 and ht-ma6i.4. So the held tasks were never re-dispatched and final review started on a partial tree. Nothing in the run record or spec drops G2 or G5: `## Human scope changes` is `none`, and `run.md` records no descope.

G2 and G5 are not baseline rows, so these are spec-Goal failures rather than baseline-row failures. They still decide the verdict, because the spec, the INDEX summary and the epic all promise them.

## Baseline trace

- B1 "Continuity key: approved": a production path exists. A registered service session frames `ReserveContinuityKey`, gated by `ServiceSessionLevel::V3`, and `service_controls::operate` reaches `continuity_keys::reserve`. An agent's hook reads the variable through `site_capability::presented_continuity_key`, sends it on `ContinuityCheckIn`, `ResolveSeat` or `CheckIn`, and `key_decision` and `KeyVerdict` judge it. `PersistentServiceClient` has no v3 surface; `docs/programmatic-service-client.md` documents self-framing `ServiceWireRequest` as the supported route. The detail beyond these entry points is the seam reviewer's to trace.
- B2 "Pane close retires immediately; graph undo reverses it": a production path exists. `ReconciliationAction::BeginRetirement` calls `begin_retirement_authored` with `Closed`; `key_of_seat` selects the reversible fence. Undo is `UnretireSeat`, reaching `reversible::unretire`, and reattachment is `ContinuityEvidence::Key`. The keyless-final scope matches source D2 (c15). A pane closed during downtime depends on G5 and is not covered at this head. The hook-to-reattach composition after un-retire is the seam reviewer's to trace; the store test `close_unretire_reattach_close_is_reversible_twice` calls `rebind_unresolved_seat` directly, and the sweep that would exercise the whole chain has not run.
- B3 "An agent exiting while its pane stays open keeps the seat and its memberships": no threads change was needed, and regressions are pinned at `tests/store/restored_pane.rs:671` and `tests/identity/reconcile.rs:4211`. The live criterion sits in the open sweep.
- B4 "Herdr will not be changed": the diff touches only this repository, and no service places a seat (`UnretireSeat` leaves `target: None`).

No baseline row is found undelivered on the paths inside this cluster's reach.

## Baseline fidelity

The Requirement baseline quotes source §4 accurately. Two refinements in the spec body keep the requested behaviour rather than lowering it.

- The source's D1 names `HERDR_GRAPH_CLONE`; the spec uses the neutral `HERDR_THREADS_CONTINUITY_KEY`. The key still works; only the graph's launch line changes.
- The source's D1 records any presented key; the spec requires a reservation first. That gives D2's "that service's launches" rule its evidence, and reserve-with-seat covers clones that already exist.

Memberships staying in place instead of being released follows the "reversible retired state" option source D2 permits, and it removes the re-acceptance the source flagged.

## Spec and run-record consistency

- §4.5 protocol shapes match `src/protocol/service.rs` and `results.rs` field for field: all five operations, their results, `ContinuityAlert`, `SeatContinuityKey`, `ContinuityKeyConflict` and `SeatRetirementState`.
- Migration 0016 matches the tables in §4.1 to §4.4.
- All six round-2 roast items are addressed in the spec text: home-node archival, the `retired_seq` advance at finalize, narrowing the warnings to invitations with an accepted limit, alert cadence, `KeyVerdict::Pending`, and the alert-only SeatInspect rule.
- `task-tree.json` matches current bead descriptions except for the later `blocked-by` hold lines on .1 and .4; its open statuses are a snapshot, not an error.
- Coverage ledger entries c1 to c37 and g1 to g3 are consistent with the spec.

## Verdict
not ready (G2, G5 and the integration sweep are unimplemented at the reviewed head, so the spec Goal is not delivered)

## Must fix before landing
- Implement ht-ma6i.1 (spec Goal and §4.1, G2): at head there is no `src/identity/shell_continuity.rs`, no fingerprint writer and no `decide_shell_continuity`, so a live handoff still leaves bare, exited-agent, `managed_launch`-only and pre-agent panes unresolved; the bead is open and ready, its branch has no commits, and no human scope change drops it.
- Implement ht-ma6i.4 (spec Goal and §4.4, G5): at head `restored_target_absence` has no writer and `RestoredPane` has no target-absence variant, so an unresolved seat whose target is absent after a restart still waits for an operator, and a key-bearing seat whose pane closed during downtime never reaches the reversible path; the bead is open and ready, and no human scope change drops it.
- Run ht-ma6i.11 after those two and repeat final review on the resulting head (spec §4.8; `run.md` root-acceptance `final-review` and `sweep`): the end-to-end flow for B1, B2 and B3 on an isolated Herdr session, the wiring sweep and TRUST-POLICY reconciliation have not happened, and this head cannot satisfy the sweep's criteria while its dependencies are open.

## Untested scope
- This cluster is documents; nothing here was executed, and spec claims were checked by reading source at the reviewed head.
- No test runs reserve, keyed launch, enroll, bind, close, send refused, un-retire, keyed relaunch in a new pane, same seat id, resumed ACK deadline and intact memberships end to end (spec §4.8); that test belongs to the open ht-ma6i.11.
- Live handoff on bare and exited-agent panes on an isolated Herdr has no test, because G2 is absent.
- `run.md` root-acceptance `sweep` (unfiltered nextest plus leak checks) has no evidence recorded at this head.
- Real-harness behaviour for B1 is unverified: whether each harness passes `HERDR_THREADS_CONTINUITY_KEY` to its hook, and whether its start hook fires before a first turn (`run.md` parked items, c37). Spec §4.2 assigns both to the source design §6 qualification, but §6's list names neither and no threads bead tracks them, so they should be added to that list or given an owner.

## Deferred OK
- `findings-ledger-design.md` still reads "through r1" with five items open, while round 2 resolved or re-raised them and the punch list settled in 86375822e; this is a stale record with no behavioural effect.
- `run.md` reports coverage round 2 with 18 requirements, but no file defines R15 to R18, and c20 cites R17; round 1 has `coverage-round-1-requirements.md` and round 2 has no equivalent, so a reader cannot resolve that id.
- Spec §4.7's size statement is wrong: it says `retired (reversible)` adds 9 bytes and the key line costs 4 bytes plus key and owner, while `output_compact.rs` prints a separate `state: retired (reversible)\n` line of 28 bytes and `key <key> <owner>\n` with 6 bytes of overhead; AGENTS.md rule (f) wants the size stated correctly, and the output is operator-facing.
- The spec's Post-Implementation Notes section is empty despite small divergences, such as the print site above and TRUST-POLICY adding bootstrap creation to the §4.2 refused list; add a dated note when the epic finishes.
- TRUST-POLICY.md:197 still says C1 has "Exactly four ways" above a seven-row table, and spec §4.6 says "becomes six", which G2's row would also overtake; the root cluster's reviewer owns the policy text, and ht-ma6i.11's TRUST-POLICY criterion should settle the count.
- Spec §4.2 says an alert-only conflict row clears when `owner_seat_id` stops being the pane's resolved owner, but `clear_alert_only_if_owner_changed` has one production caller, the check-in at `cooperative.rs:1053`, and the SeatInspect read at `continuity_keys.rs:422` has no filter, so a stale alert-only row can show until the next check-in or until the target leaves a publication; roast round 2 spot-rejected this as a nit and these kinds never hold a pane.
- The source design still says `HERDR_GRAPH_CLONE` in D1 and §5 item 2, while §4.5 of this spec settles `HERDR_THREADS_CONTINUITY_KEY`; the graph side must use the spec's name when it implements launch, and that document lives in the herdr-graph repository.
