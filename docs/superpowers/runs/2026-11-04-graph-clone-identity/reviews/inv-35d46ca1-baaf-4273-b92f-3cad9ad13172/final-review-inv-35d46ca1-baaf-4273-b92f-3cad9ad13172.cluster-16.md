# Final review: cluster 16 (src-identity), epic ht-ma6i

Source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head 1b058307490f29ece8bb4e3fe7a862f8356d992f. Both verified as commits; the caller checkout was at the head and clean when read and again after the reproduction.

Files: `src/identity/mod.rs`, `src/identity/reconcile.rs`, `src/identity/repair.rs`, `src/identity/shell_continuity.rs`. Standard: the root design's requirement baseline (B1 to B4) and §4.1, plus the cluster's parts of §4.2, §4.3 and §4.4. Beyond the cluster I read only what a named risk required: the shell continuity store functions, the `Move` arm of reconciliation, the worker lane and the C1 pin guard.

## Baseline trace

- B1 (continuity key). Inside the cluster: `claim_evidence` selects `ContinuityEvidence::Key` both at intake (`continuity`, reached from `src/service/dispatch.rs:2269`) and when the worker decides a kept claim (`decide_kept_continuity`). The process path sends `continuity_key: None`. An unreserved key falls through as cooperative in the store's `ContinuitySelection::keyed_seat`. The route from hook to dispatch and the verdict table belong to the seam and store reviewers.
- B2 (pane close retires, undo reverses). Inside the cluster: `reconcile_published_page_for` treats `RetiredReversibly` as a transition with no job, and `retirements_started` counts only `RetirementStarted`. `plan_page_for` still skips every retired seat, and `saved_seats_page` leaves out waiting seats. Un-retire itself sits in the store and service clusters.
- B3 (agent exit keeps the seat and memberships). This cluster touches no membership. Shell continuity opens no binding and ends none.
- B4 (Herdr unchanged). `pane_shell_pid` uses the existing `pane process-info` read through `run_unfenced`; nothing new is asked of Herdr.

## Shell continuity lane against §4.1

Most of the design matches the code:

- The lane runs after `decide_process_continuity`, only after a complete pass (`src/service/workers.rs:2554`).
- Both cursors move past unreadable and declined seats and wrap at the end of the listing.
- Reads are capped at 16 per pass for recording and 32 candidates per page for deciding, inside a 3 s budget with 750 ms per read.
- Declines are keyed by (seat, generation, pid, start).
- `NotContinuous` and definitive refusals are remembered. `Gone`, `Unknown`, an unread pane and not-ready refusals are retried.
- A seat with a live undeclined process candidate is left to that path.
- `shell_continuity::judge` adds pid equality, session leadership and recorded-tty equality on top of `process_continuity::judge`.
- The kept-claim refusal is `ServiceBusy`, so it is not remembered as a decline. The owned-target refusal is definitive unless reconciliation lags.
- The rival listing is complete for a target, because `ProcessContinuityPage.target` removes the limit.

## Pane move reproduction

The one defect is what a pane move does to the fingerprint row. The evidence is a repro added in a review-owned detached worktree at the head, since removed; `Fx::observation` was changed so a pane named `moved` keeps the terminal of `p1` in the first incarnation.

The sequence: a seat resolves at `p1` and records its row; `publish_and_pass(["moved"])` applies a real `Move`; `handoff(["moved"])` follows.

Observed output:

```
move outcomes: [Applied]
after move: seat target=moved state=("resolved", 1) row target=p1 gaps=0
after handoff: state=("unresolved", 2) candidates=0
```

The gap query keys a current row on (boot, incarnation, terminal), so the moved seat is never re-read. The candidate query joins `f.target_id=s.target_id`, so the stale row can never match. `shell_candidate_stands` has the same join. The `Move` arm updates only `seats` and `occupant_bindings`, and nothing else writes that column.

`herdr pane move --workspace` gives a pane a new id with the same terminal (`TRUST-POLICY.md:349`, `docs/validation/herdr-release-matrix.md:46`), so this is an ordinary production sequence. The binding path is unaffected because `Move` updates bindings, so only seats with no live agent are hit, which are exactly the seats G2 exists for.

Control: a second repro with two consecutive handoffs and no move passed. The row is re-recorded after a decision and the seat recovers again.

## Verdict

not ready (a bare seat whose pane moved before a live handoff is never a shell continuity candidate and waits for an operator)

## Must fix before landing

- A bare or exited-agent seat whose pane was moved (`herdr pane move --workspace`, new id, same terminal) before a live handoff is not recovered, against the design Goal "live handoff recovers every unnamed non-person pane seat unattended" and §4.1, with no accepted limit covering it: the `Move` arm leaves `seat_shell_fingerprints.target_id` on the old pane, `shell_fingerprint_gaps` (`src/store/seats/continuity.rs:1120`) still calls the row current, and `shell_continuity_candidates` (`:1225`) and `shell_candidate_stands` (`:1300`) join on `f.target_id=s.target_id`, so the repro shows `candidates=0` after the handoff and the target stays shown, which keeps G5 from ever retiring the seat; the store lines are outside this cluster's files, but they break the contract `record_shell_fingerprints` and `decide_shell_continuity` rely on; fix by keeping the row's target in step in the `Move` and `ReconfirmStructure` transactions (no extra host read, and the policy's "there" stays true) or by dropping the join, since the saved target only selects the pane to ask and the kernel facts decide; add the repro as a store test and extend the isolated-Herdr handoff test.

## Untested scope

- `judge_shell_candidate`'s remembered definitive refusal (`result.is_err() && !continuity_not_ready`) has no lane test: `tests/service/observation_lane.rs` covers only the `NotContinuous` decline and retried `Gone`, so neither a definitive refusal being remembered nor a `ServiceBusy` refusal being retried is pinned at this level.
- `decide_shell_continuity`'s cursor has no lane test: paging past 32 candidates, wrapping, and resuming after budget exhaustion are not exercised, while the recording cursor is (`an_unreadable_shell_cannot_starve_the_others_and_reads_are_capped`).
- The unreadable-rival branch of `has_rival_in_pane` is not exercised from the shell path: `a_rival_decides_nothing` covers a live rival process but not `Verdict::Unknown`, despite its doc comment naming "live or unreadable".
- I ran only the two review repro tests at this head and not the cluster's existing tests; suite evidence is the caller's to supply.

## Deferred OK

- An unresolved candidate whose target is absent from the published snapshot costs one `herdr pane process-info` spawn per pass (up to 32, bounded by the 3 s budget) until G5 retires it or a kept claim settles; §4.1 accepts retrying an unread pane, but filtering candidates on the published snapshot would remove reads that can never decide anything, in the spirit of ht-kaw9.
- `record_shell_fingerprints` reads the pane by target with no check that the terminal is still the one the gap listed, so a pane id reused between publication and read could record another shell under the old terminal; the window is one pass and the judge's pid, start and tty checks still apply at decision time, so the likely result is a missed recovery rather than a wrong rebind.
- `with_process_info` feeds only the shell path, while `decide_process_continuity` and `decide_kept_continuity` still construct `HostProcessTable` directly; production behaves the same, but a lane test cannot put a fixture under both paths in one pass, so moving them to `self.processes` would be a small cleanup.
