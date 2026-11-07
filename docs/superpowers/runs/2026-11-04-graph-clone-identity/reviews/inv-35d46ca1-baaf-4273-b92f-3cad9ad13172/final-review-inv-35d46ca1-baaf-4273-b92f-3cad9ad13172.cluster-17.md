# Final review: cluster 17 (src), epic ht-ma6i

Source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head 1b058307490f29ece8bb4e3fe7a862f8356d992f, both verified with `git rev-parse --verify`. Invocation inv-35d46ca1-baaf-4273-b92f-3cad9ad13172, source binding accepted. The caller checkout stayed at the captured head with a clean status.

## Scope and method

The cluster is one file, `src/ports.rs`, slice `slices/000077.diff` (226 insertions, 3 deletions). The slice is byte-identical to `git diff` of the captured range for that path.

This was a source-read review; no build or test ran. Every type here is a port contract, so the questions are whether each promise in its doc comment holds at its production producer and consumer, and whether the shapes match the design. Other files were opened only for that. Their own defects belong to their clusters.

## What the file adds

- `RetirementInputs` and `RetirementStart`, plus `into_job`; `begin_retirement` on the port now returns `RetirementStart`.
- `RestoredPane::{TargetShown, TargetAbsent, TargetGone}` and the matching `ReconciliationAction` variants, classified in `may_complete_past_refusal`.
- `ReconciliationOutcome::RetiredReversibly`.
- `ContinuityEvidence::Key`.
- The shell continuity types (`ShellFingerprintGap`, `ShellContinuityCandidate`, `ShellContinuityGuard`, `ShellContinuityRequest`), the two per-pass caps and five `StorePort` methods.
- `continuity_key: None` in three test fixtures.

## Checks against named risks

Retirement shapes (design, key abstractions). `RetirementInputs` holds exactly the six columns the design lists. The `UPDATE retirements SET closure_evidence` the design replaces is gone: the only production `UPDATE retirements` sites left are job progress at `src/store/control/retirement.rs:542,553`. The `&'static str` evidence values are the sole values the migration CHECKs allow (`migrations/node/0016_graph_clone_identity.sql:86-87,107-108`). The durable read at `src/store/seats/reversible.rs` maps them back explicitly and answers `StoreCorrupt` on anything else. `begin_retirement_authored` is the one chooser; `retirement_outcome` in `observation.rs` is the one translation; and `reconcile.rs:573-579` counts only `RetirementStarted`, which is what the variant's doc promises. The port's retried closure proof returns `Reversible` (`retirement.rs:59-61`), so it does not read as stale.

G5 variants. Every site that names `RetireRestoredPane` also names `RetireAbsentTarget`: the CAS arm and the unreachable arm in `observation.rs:2476-2528`, the application at `:2924-2978`, and the plan at `reconcile.rs:271-277`. No wildcard swallows the new actions in the applier, and the `(_, None)` plan fallthrough lands on the existing no-op `MarkUnresolved`. The evidence function (`observation.rs` `target_absence_evidence`) matches the variant docs: `TargetShown` only with a ledger row, `None` when the count is blocked by a kept claim or an unrecorded pass, `TargetAbsent` below the grace, `TargetGone` at it, Herdr hosts only. `may_complete_past_refusal` returning true is sound for all three: a retirement or ledger write adds no C1 candidate and no seat to protect, the same judgment as the watch actions. A stale plan re-derives at commit and refuses, and the next pass replans. The test at `tests/identity/reconcile.rs:4760-4792` pins both the plan and this classification.

`ContinuityEvidence::Key`. The doc says a decided reattachment records `cooperative_continuity_key` and anything else falls through as `Cooperative`. That holds at `src/store/seats/continuity.rs:934-945`: `SelectedSeat::Keyed` takes the key provenance and `SelectedSeat::One` keeps the selection's cooperative kind. The producer is `claim_evidence` (`src/identity/repair.rs`), used at intake and for a kept claim. A keyless digest stays byte-identical (`continuity.rs:227`).

`ShellContinuityGuard`. It mirrors `ContinuityTargetGuard` field for field and refuses on the same fence checks and lifetime. Two differences are deliberate and correct.
- The proof is mandatory at `try_new`, which is what lets the decision record it so later passes reconfirm the seat, as §4.1 requires.
- `consume` binds no command, because the guard is target-only. The binding the sibling gets from the command is supplied here by the transaction: the instance check at entry, `guard_continuity_pin` on (seat, generation), and `shell_candidate_stands` re-checking the seat, target, pid, start and incarnation against the table. A request whose seat does not stand at the guard's target refuses as `Conflict`.

Refusal classes match the lane's doc. A `try_new` failure and a stale fence are `StaleHostObservation`, which `is_continuity_refusal` treats as not ready, so they are retried rather than declined (`repair.rs:843`).

Caps and cost. `SHELL_FINGERPRINT_READS_PER_PASS` is 16 as §4.1 states; both caps are enforced by `LIMIT` in the listing, and the lane pages with a cursor as the design's gap fill requires. The 32-candidate cap is not in the spec, but it fits §4.1's "one read per seat": each pass runs under the 3-second maintenance budget, each read under 750 ms, so a stalled host costs about four reads, not 32.

Scope of the listings. `SHELL_SEAT_SCOPE` (`s.is_local=1 AND s.name IS NULL AND s.target_id IS NOT NULL AND h.kind='herdr'`) gives the "local unnamed, Herdr host" the port docs promise. Only the named host is `NonClosing` (`src/host/named.rs:640`), so `kind='herdr'` is the design's "closing Herdr host". `shell_rival_exists` joins out retired seats as its doc says.

Wire and policy. None of the new types derives serde, so `scripts/check-wire-compat` is not engaged by this file. Every TRUST-POLICY section the new docs cite exists at the head (C1 rows at lines 214-215, *Shell continuity* at 461, *Reversible retirement* at 490, *Target absence* at 555). `StorePort`'s new methods have no defaults, so an adapter cannot silently omit them; the one test decorator delegates (`tests/daemon/harness_evidence.rs:2167-2195`).

## Requirement baseline trace

The root spec has a `## Requirement baseline` section with rows B1 to B4 and no human scope change rows.

- B1 (continuity key): this file carries only `ContinuityEvidence::Key` and the `continuity_key` field in fixtures. The part of the path that touches it, kept claim or intake evidence through to the recorded provenance, holds as traced above. The trace from the hook and CLI entry points to `ContinuityCheckIn` leaves this cluster and is the seam reviewer's.
- B2 (pane close retires immediately, graph undo reverses it): the production close path that uses this file's types is the observation lane (`src/service/workers.rs`) through `reconcile.rs` and the `BeginRetirement`, `RetireRestoredPane` and `RetireAbsentTarget` arms in `observation.rs`, to `begin_retirement_authored`, then `RetirementStart::Reversible`, then `RetiredReversibly`. `tests/store/restored_pane.rs:583,1038` pin that translation. The undo (`UnretireSeat`) has no type in this file; its trace is the seam reviewer's.
- B3 (agent exit keeps seat and memberships): the spec records no threads change, and nothing here ends a binding or a membership. Shell continuity explicitly opens no binding.
- B4 (Herdr unchanged): `HostPort` is not in this diff. `pane_shell_pid` and `observe_current_target` already existed, and the guard consumes them unchanged.

No row is undelivered on this cluster's own terms.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- `ShellContinuityGuard` refusals in `src/ports.rs:2007-2075`: no test reaches any of the three `try_new` refusals (target mismatch or empty instance, non-fresh provenance, no verified structural proof) or the three `consume` refusals (already consumed, changed fence, expired observation); `tests/store/shell_continuity.rs:289-297` always reads fresh immediately before deciding and unwraps `try_new`, so the stale-observation path of `decide_shell_continuity` (a guard older than a newer observation) never runs either, and a wrong edit to these checks would pass the suite. The code is a close copy of `ContinuityTargetGuard`, so the risk is drift, not a known defect.
- The lane's "a not-ready refusal is retried, not declined" branch at `src/identity/repair.rs:843` for shell candidates: `tests/service/observation_lane.rs` covers the decline memory and a dead shell, but I found no test that makes the store decision answer `StaleHostObservation` or `ServiceBusy` and then asserts the next pass decides.
- `reconcile.rs:573-579` counting a reversible retirement out of `retirements_started`: the store-level outcome is pinned at `tests/store/restored_pane.rs:594,1052`, but no test asserts the page counter for a reversible close. Low value, since the counter has no production consumer beyond the progress struct.

## Deferred OK
- `src/ports.rs` now names `crate::identity::shell_continuity::ShellFingerprint` in `ShellContinuityCandidate`, `ShellContinuityRequest` and two `StorePort` methods, the first `crate::identity` edge in this file (the base had none), while `identity::shell_continuity` imports `ports`; the module cycle compiles and is harmless, and moving the plain data struct into `ports` is a small cleanup for a later change.
- `RetirementStart::into_job` has no production caller, only test helpers (`tests/store/control.rs:27`, `tests/daemon/control.rs:115`, `tests/store/facade.rs:862`, `tests/integration/lazy_attention.rs:945`); its `Option` return still keeps "no caller may assume a job", so it does no harm on a production type.
- A candidate whose saved target is no longer in the publication still costs a `pane_shell_pid` read each pass (`src/identity/repair.rs`, retried as an unread pane) until G5 retires it, up to 32 per pass under the 3-second budget; TRUST-POLICY states the retry, it ends within the 10-minute grace, and answering it from the publication would save the read. This is the src-identity reviewer's to weigh; the cap is all this file contributes.
