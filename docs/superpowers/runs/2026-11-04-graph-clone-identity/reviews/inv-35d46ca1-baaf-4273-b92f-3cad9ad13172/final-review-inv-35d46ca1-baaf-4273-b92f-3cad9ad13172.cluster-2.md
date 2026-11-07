# Final review, cluster 2 (root), epic ht-ma6i, integration branch super-auto/graph-clone-identity

Source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head 1b058307490f29ece8bb4e3fe7a862f8356d992f, both verified with `git rev-parse --verify`. The source binding for invocation inv-35d46ca1-baaf-4273-b92f-3cad9ad13172 was accepted before this file was written. The caller checkout read clean at the head at the start and before this write; no commit or status change was observed.

Inputs: `slices/000002.diff` (`Cargo.toml`) and `slices/000003.diff` (`TRUST-POLICY.md`), the digest, the spec at the head, `bd show ht-ma6i` and `ht-ma6i.23`. Source was read with `git show <head>:<path>` only to check what the policy text states. herdr-graph at 0dbe477 was read, not changed.

Executed here: nothing was built or run. The judgments below come from reading the text and the code at the head. Test evidence named here belongs to the task reports and other cluster reviewers.

## Baseline trace

This cluster is the normative text and one manifest entry, so each row is traced as far as the policy describes it. Paths that leave the cluster are the seam reviewer's to trace.

B1, "Continuity key: approved." The policy delivers the C-row the source design asks for (line 215), the outcome table (`store::seats::key_verdict` matches every row I compared), and the ledger rules (`store::continuity_keys` matches on one row per pane, one alert, the 256 cap, and the hold and clearing kinds). The Hermes limit does not lower the row: the source design's D1 is "the graph launches every clone agent", and herdr-graph's `src` has no Hermes reference (matches exist only in its vendored `third_party/herdr-threads`). The rest of the hook-to-store path is the seam reviewer's.

B2, "Pane close retires immediately; graph undo reverses it." The policy's *Reversible retirement* matches `begin_retirement_authored`, `reversible::unretire`, `reversible::finalize` and the fragments' compositions (due scans, archival, hold lift, repair guidance). `PersistentServiceClient` now frames v3 (`tests/clone_identity_sweep.rs` uses it), so the previous review's missing producer is closed. The rest is the seam reviewer's.

B3, "An agent exiting while its pane stays open keeps the seat and its memberships." Stated at the *Fence* bullet and consistent with the source: an exit writes no row and retires nothing. The seam reviewer traces the writer side.

B4, "Herdr will not be changed." The policy adds no Herdr behaviour or call. New Herdr reads are the existing `pane process-info` and `observe_current_target`.

No baseline row is undelivered from this cluster. The items that hold the verdict are normative text that no longer describes the delivered behaviour.

## Policy text checked against code

Statements that hold, by reading the head:

- Continuity key: lag rule and its exception for a retired seat (`key_verdict::judge`), `refuse` codes (`TargetAlreadyOwned` for alert-only kinds, `TargetUnresolved` for hold kinds, `ServiceBusy`, `ContinuityKeyPending`), recording after rollback (`record_stashed_conflict`), inline recording at check-in (`settle_key`), the hold's placement in `current_resolution_owner` only, and each clearing rule (`rebind_unresolved_seat`, `operator.rs:297`, `clear_conflicts_absent_from`, `delete_key`, `clear_alert_only_if_owner_changed`). A refused keyed probe on a resolved pane falls through to the check-in (`hook.rs:2082`), so "T keeps working" holds.
- Continuity key binding: `require_bindable_seat` is the stated guard. The binding writes no seat row.
- Reversible retirement: un-retire's transition, its authorization order (`Unauthorized` before not-found), the deadline shift bound on replay, the finalize paths and the provenance kinds. Both writers of a `retirements` row release the key (`insert_retirement_job`, `insert_replicated_retirement_job`).
- Shell continuity: scope, candidate listing, rival rule (non-retired only), decline keys, per-pass cap of 16, row deletion at final retirement and un-retire. "Allocation asks the lane for a capture" was not re-traced here; the task report's `new_bare_seat_gets_its_row_with_no_other_trigger` is its evidence.
- Target absence: 30 s step, row key, kept-claim and marker blocks, 10 minutes, evidence columns, host-kind exclusion, hold lift.
- The items the previous review raised against this file are closed: the inherited-key limit now lists the real exits, the C1 count is gone, and the G2, G5 and decision-record text from spec 4.6 is present.

Statements that do not hold are the first three bullets of the landing list below. Each needs text only, with no behaviour change.

## Manifest

`Cargo.toml` adds one `[[test]]`, `clone_identity_sweep`, with `required-features = ["test-support"]`, and the file exists at the head. The entry is correct as written. The module it includes, `tests/integration/herdr_release_smoke.rs`, carries `#[ignore]` matrix tests, so those names now also exist in this binary. `scripts/herdr-release-smoke` selects `--test integration`, and a default nextest run skips ignored tests, so nothing runs twice.

## Verdict
not ready (TRUST-POLICY text contradicts delivered behaviour: retirement facts in C2 and N5, an unlisted target-absence limit, and two keyed-claim statements; text fixes only)

## Must fix before landing
- `TRUST-POLICY.md` lines 574-576 and 2044 (N5) say a published seat's retirement is a `seat.retired` fact and that a remote seat is retired only by `seat.retired`, but a key holder's close authors `seat.retired_reversible` (`begin_retirement_authored`), which makes the peer's copy read retired, and `seat.unretired` restores it; N4 was updated and these were not, so state both reversible facts as home-node facts there, and qualify line 404's "Memberships retire, receipts settle recipient-retired and invitations close" and line 374's hold-lift condition for a reversible and an unretired seat.
- `TRUST-POLICY.md` lines 551-553 and 562 say a seat with an unwatched target "waits for the operator or for target absence" and that a shown target hands over to the watch rule, but `restored_pane_evidence` falls through to `target_absence_evidence` when the target is shown and not watchable (not in the baseline or released, no terminal, or claimed by another seat), which counts no time, so a seat whose pane never came back and whose id now shows another pane waits for an operator and blocks that server's hold lift (`lift_baseline_hold_if_clear`); the code follows spec 4.4's reset rule, and AGENTS.md requires an accepted limit in the policy, so add this limit and correct both sentences (recorded only as a deferred minor of ht-ma6i.4 until now).
- `TRUST-POLICY.md` lines 274-275 say a pane under a holding row refuses "a claim with no key", while line 267 and the limit at line 2265 say C1 does not pass through that check and a C1 claim is an exit, which matches the code (`hold_kind` is reached only through `current_resolution_owner`); restrict the exception to ordinary resolution and a claim that finds no match, and correct line 215's "matching no other unresolved seat", because `key_decision` reattaches S when the session matches S among several seats.

## Untested scope
- This review ran no build, test, clippy or policy script; the full nextest suite and leak checks have no evidence at this head from this cluster, and the ledger defers them to the caller.
- No test pins a seat whose saved target stays shown by a pane it cannot watch: `tests/store/restored_pane.rs:1089` covers the reset only, so the limit named above has no test recording its outcome.
- A keyless C1 claim reattaching on a pane under a holding row was not checked for a test here; only the code path was read.

## Deferred OK
- `TRUST-POLICY.md` amendment log has no entry for ht-ma6i.6, ht-ma6i.7, ht-ma6i.11 or ht-ma6i.23 (the two new wire fact kinds, the keyed probe of a resolved pane, three limits); the bodies carry the rules, so this is audit-trail drift already recorded by ht-ma6i.23.
- `TRUST-POLICY.md` line 374 names only "cooperative continuity" among hold releases, though process and shell continuity also release a hold; the wording predates this epic for process continuity and both sections state the release.
- `src/store/seats/reversible.rs` `episode_fact_applied` reads a second episode of one seat in the same millisecond as a copy; this is stated only in a doc comment and is not worth a policy limit unless a second touch to that guard happens.
- ht-ma6i.22's report says fork repair can re-issue `seat.unretired` before `seat.retired_reversible`, leaving a peer's copy reading retired, and the policy is silent on it; I did not reproduce it, it belongs to the store and seam reviewers, and it still needs an owner.
- `docs/operations.md` around lines 284-297 is outside the range and lacks `structural_shell_continuity`, `cooperative_continuity_key`, `service_unretire` and `service_finalize` in its repair history table; it is operator documentation for a follow-up doc pass.
- `Cargo.toml` gives `clone_identity_sweep` its own binary although the file's header and AGENTS.md send a suite with no process-wide state to `combined`; its nextest group filters on the binary as `integration_sweep`'s does, so the cost is one more link and duplicate ignored test names.
