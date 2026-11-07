# Final review, cluster 12: src-identity (ht-ma6i)

## Source reviewed

Base cf13ed76c1c2871b321ad290d03229cb785c400e, head 0755e609de0328e6d73584dd65eb2854e1fd7bf7; both verified as commits. The source binding succeeded. The caller checkout stayed at the head and clean throughout. Files: `src/identity/reconcile.rs`, `src/identity/repair.rs` (slices 000039, 000040). This pass read code and tests statically at the head; it ran no build or test, and it relies on the ledger's per-merge check records for test results.

## What the diff does, and why it holds

`reconcile.rs`, published page: `RetiredReversibly` joins the job-less arm. It counts as a transition and not as a started retirement. In production nothing reads `retirements_started` (only `tests/service/crash_matrix.rs` and `tests/identity/reconcile.rs` do), so no retirement kick is lost, and the absence of a job matches spec "only `RetirementStarted(job)` advances a job".

`reconcile.rs`, invalidated page: `RetiredReversibly` is refused like `RetirementStarted`. `mark_unresolved_from_invalidation` has no retirement path, so this arm is defensive and consistent.

Planner: an un-retired seat never reaches `plan_page_for`, because `saved_seats_page` filters it with `waiting_seat_sql!`, and the invalidated page skips every unresolved seat. Spec 4.3 "the reconciliation plan skips it" holds.

`repair.rs`, `claim_evidence`: intake (`continuity`) and the worker (`decide_kept_continuity`) share one function, so a kept keyed claim is decided with its key. A key conflict answers `TargetUnresolved` or `TargetAlreadyOwned` and a claimless key answers `ContinuityKeyPending`, all definitive under `is_continuity_refusal`, so they are never kept. Lag answers `ServiceBusy`, which is kept. That matches spec 4.2 "the refusal is definitive" and "a keyed claim during lag needs no later event". `kept_continuity::forget` compares `command_json`, which includes the key, so a newer claim with another key survives.

`repair.rs`, process path: `continuity_key: None` on the daemon's own claim leaves its digest and evidence as they were. Candidates need a saved target and a binding ended `seat_unresolved`, so neither a reversibly retired seat nor an un-retired one can match.

Host scope: `claim_evidence` does not test host kind, but `key_decision` treats a key on any non-Herdr host as unreserved, so spec 4.2 "Scope" holds.

No defect was found in the 21 changed lines themselves.

## Requirement baseline rows

- B1: at this cluster's edge, `dispatch.rs` calls `OrdinaryIdentity::continuity`, which passes `Key` to `decide_continuity`; delivered in this cluster. The hook-to-daemon path beyond it is the seam reviewer's to trace.
- B2: the same-incarnation and restored-pane closes reach `begin_retirement_authored` through this planner and the arm above; delivered in this cluster. `UnretireSeat` and the graph's call are the seam reviewer's.
- B3: the planner retires only on target absence, not on agent exit, so an exited agent's open pane keeps its seat; unchanged and holding.
- B4: no Herdr surface was added; `pane_shell_pid` already existed.

G2 and G5 are spec goals, not baseline rows, so the first two items below name no row.

## Spec sections with no code at this head

`git grep` at the head finds `decide_shell_continuity`, `identity::shell_continuity` and any writer of `seat_shell_fingerprints` or `restored_target_absence` nowhere in `src/` except the table lists in `src/service/kicks.rs`; the two tables exist only in migration 0016. `src/identity/` holds `caller.rs`, `mod.rs`, `process_continuity.rs`, `reconcile.rs` and `repair.rs`. `RestoredPane` has no target-absence variant, and `workers.rs` calls only `decide_kept_continuity` and `decide_process_continuity` after a pass.

Observed state: beads `ht-ma6i.1` and `ht-ma6i.4` are open and listed by `bd ready`; `ht-ma6i.16`, which held them, is closed; branches `task-ht-ma6i.1` and `task-ht-ma6i.4` are ancestors of the head with no commits of their own; the ledger's last entry for each is "waiting on ht-ma6i.16", with no re-dispatch recorded. The cause of the missed re-dispatch was not established here. `ht-ma6i.11` (the end-to-end sweep) is open too.

## Verdict
not ready (G2 shell continuity and G5 target-absence retirement, spec 4.1 and 4.4, have no code in src/identity at this head; beads ht-ma6i.1 and ht-ma6i.4 are open)

## Must fix before landing
- Spec 4.1 (G2, ht-ma6i.1) is not delivered: `src/identity/repair.rs` has no fingerprint-recording lane, no `decide_shell_continuity` and no shell cursors, `src/identity/shell_continuity.rs` does not exist, and `src/service/workers.rs` does not call such a lane; so a live handoff of a bare, exited-agent, `managed_launch`-only or pre-agent pane seat stays unresolved, against the spec's stated goal. Re-dispatch ht-ma6i.1, whose blocker ht-ma6i.16 is closed.
- Spec 4.4 (G5, ht-ma6i.4) is not delivered: `src/identity/reconcile.rs` plans no target-absence action and `RestoredPane` has no target-absence variant, so an unresolved seat whose saved target never reappears after a restart plans `MarkUnresolved` and waits for an operator indefinitely. Re-dispatch ht-ma6i.4, whose blocker ht-ma6i.16 is closed.

## Untested scope
- `OrdinaryIdentity::continuity` intake with a keyed claim (`repair.rs`, `claim_evidence(&command)` at intake): no test drives a keyed `ContinuityCheckIn` through it. `tests/identity/kept_continuity_rules.rs` tests the pure function, `tests/store/continuity_key_decisions.rs` passes the evidence directly, `tests/cli/hook.rs` uses a fake client, and `kept_keyed_claim_is_decided_after_lag_with_no_further_event` adds the key to a kept row by SQL. By inspection, reverting the intake to `Cooperative` would fail no test while breaking a fresh keyed agent's reattachment outside lag (B1). Not confirmed by mutation; the open sweep ht-ma6i.11 is the natural owner.
- `reconcile_published_page_for` with a `RetiredReversibly` outcome: `tests/store/restored_pane.rs` drives `plan_page` and `apply_reconciliation_transition` directly, so no test asserts `transition_count` and `retirements_started` for a reversible close.
- The spec 4.8 isolated-Herdr end-to-end run (reserve, launch with key, close pane, un-retire, relaunch, same seat) has no test at this head; no script or test combines the isolated Herdr helper with the key. It belongs to open bead ht-ma6i.11.
- `reconcile_invalidated_page`'s `RetiredReversibly` refusal has no test; the arm is unreachable today.

## Deferred OK
- `TRUST-POLICY.md` line 197 still reads "Exactly four ways" above a seven-row table, and its conflict text names shell continuity, which has no code yet; this file is outside this cluster, and the wording should be settled when ht-ma6i.1 lands (spec 4.6).
