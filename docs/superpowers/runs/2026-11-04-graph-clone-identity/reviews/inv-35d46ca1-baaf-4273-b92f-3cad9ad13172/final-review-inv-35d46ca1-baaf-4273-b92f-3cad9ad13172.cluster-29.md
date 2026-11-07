# Final review, cluster 29 (tests-identity)

Source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head 1b058307490f29ece8bb4e3fe7a862f8356d992f, both verified with `git rev-parse --verify`. The caller checkout was observed clean at that head at the start and after the review worktree was removed. Inputs: `slices/000153.diff` (`tests/identity/kept_continuity_rules.rs`) and `slices/000154.diff` (`tests/identity/reconcile.rs`).

## Analysis

### What the cluster changes

Both files are test modules mounted into the library with `#[cfg(test)] #[path]`. The diff has three parts:

- `continuity_key: None` added to two existing `ResolveSeat` literals, a compile fix with no behaviour.
- In `transient_refusals_keep_the_claim_and_definitive_ones_do_not`, `TargetUnresolved` and `ContinuityKeyPending` added to the definitive list, plus the new test `a_claim_rests_on_its_key_when_it_presents_one`.
- The new test `target_absence_evidence_plans_its_action` for G5.

### Checked against the spec and production code

- The definitive list matches spec 4.2. A key conflict is "definitive (not retried inside the callback, not kept)". `refuse` in `src/store/seats/continuity.rs` answers `TargetUnresolved` for hold-bearing kinds, `TargetAlreadyOwned` for the alert-only kinds and `ContinuityKeyPending` for a pending key; `is_continuity_refusal` in `src/cli/retry.rs` classes all three as refusals, so `continuity_not_ready` is false. The `ServiceBusy` a lagging key answers stays on the transient list, which is what makes it a kept claim ("A keyed claim during lag needs no later event").
- `claim_evidence` maps a present key to `ContinuityEvidence::Key` and no key to `Cooperative`, as the spec's Trust paragraph requires. Both production sites call it: `src/identity/repair.rs:474` (intake) and `:531` (the observation worker).
- The G5 plan arms match `plan_page_for` in `src/identity/reconcile.rs`: `TargetShown` resets, `TargetAbsent` notes, `TargetGone` retires naming the saved target, and `None` stays `MarkUnresolved`. The retirement's target assertion is real: the fixture's saved target is the only source of `absent_target`.
- `may_complete_past_refusal` returning true for all three G5 actions is correct under its contract in `src/ports.rs`: the seat is already unresolved, so none of them can add a C1 candidate or a seat to protect. It matches the `RestoredPane` arms beside it.
- The fixture is honest for this planner. `plan_page` reads neither `target_count` nor the ordinals, and the page bounds it does check are satisfied.

### Reproduction

I ran the two modules' tests in a review-owned detached worktree at the head, seeded with a copy-on-write target and since removed: `cargo test --locked --all-features --lib -- target_absence_evidence_plans kept_continuity_rules review_probe`. All 4 cluster tests passed, as did one temporary probe.

The probe tested the claim in the G5 test's doc comment, which that test does not exercise. It planned `Resolved`, `Retired` and `Unresolved(HostInvalidation)` seats with each of `TargetShown`, `TargetAbsent` and `TargetGone`. None produced a G5 action: `Resolved` and `HostInvalidation` planned `MarkUnresolved`, and `Retired` planned nothing. The production behaviour is therefore correct; only the test's stated coverage is short. The store also confines the evidence, since `restored_pane_evidence` selects only `state='unresolved' AND unresolved_reason='other' AND target_id IS NOT NULL`.

### Coverage of the stated kills outside this cluster

- The worker site is killed by `kept_keyed_claim_is_decided_after_lag_with_no_further_event` in `tests/integration/trust_policy.rs`. It runs on a stand-in Herdr and asserts the `cooperative_continuity_key` decision, though it writes the key into the kept row by SQL.
- The intake site's only end-to-end coverage I found is `tests/clone_identity_sweep.rs`, which runs against real isolated Herdr sessions and skips when `herdr` is absent. Store tests call the decision with the evidence already chosen, so they do not reach this site. This comes from reading test sources and greps for the key variable; no mutation was run.
- The G5 applier's re-derivation is covered by `the_applier_refuses_an_absent_retirement_whose_evidence_changed_after_the_page` in `tests/store/restored_pane.rs`, and the host-kind limits by `named_hc_and_unretired_seats_are_never_retired_this_way`.

### Requirement baseline

- B1: this cluster pins refusal classification and evidence selection only. I confirmed the `repair.rs` call sites and that the client reads `CONTINUITY_KEY_ENV` in `src/cli/caller_location.rs`. The route from hook to daemon crosses clusters and is the seam reviewer's to trace.
- B2: the cluster pins only the plan step of G5's retirement. Reversible retirement on close and graph undo are outside it and are the seam reviewer's.
- B3 and B4: no production code here touches them, and nothing in the diff adds or removes delivery.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- `tests/identity/kept_continuity_rules.rs:72` says the test kills a keyed claim decided without its key "at intake or by the observation worker", but it only exercises the `claim_evidence` helper, so replacing the call at `src/identity/repair.rs:474` or `:531` with `Cooperative` passes it; the worker site is guarded by the stand-in Herdr test in `tests/integration/trust_policy.rs`, while the intake site's end-to-end guard is `tests/clone_identity_sweep.rs`, which skips when `herdr` is absent, so a run without Herdr has no end-to-end guard on that line.
- `tests/identity/reconcile.rs:4696` says "no other seat state takes one", but the `plan` closure is only ever called with `SeatState::Unresolved`; a temporary probe at the head confirmed production plans no G5 action for `Resolved`, `Retired` or `HostInvalidation` seats, and that probe was not committed, so no test in this module pins it.
- The `(TargetAbsent | TargetGone | TargetShown, target None)` fall-through to `MarkUnresolved` in `plan_page_for` has no test here; the store cannot emit that pair today because `restored_pane_evidence` requires `target_id IS NOT NULL`.
- This review ran only the two modules' tests and one probe at the head; the full suite is the caller's, and the ledger records that measurement as deferred to the caller.

## Deferred OK
- Tighten the two doc comments to what their tests prove, or add the missing assertions: pin the other seat states in `target_absence_evidence_plans_its_action` (which also gives the closure's `state` parameter a purpose), and reword the `a_claim_rests_on_its_key_when_it_presents_one` kill line to name the helper.
