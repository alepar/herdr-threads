# Final review (bounded): ht-ma6i graph clone identity

Invocation inv-1a8ebbff-aefc-4596-b77d-4e6dee7299e1. Captured source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head c00972c75e3048e486aaeeda5c47eb27e6d03a79 (both verified with `git rev-parse --verify`). Caller checkout observed at head c00972c75 with a clean status; nothing was switched or restored.

## Route and provenance

This is a bounded captured-source review, not a whole-range review. The review-inputs sidecar reads status `unavailable`, `sourceAvailable: true`, `wholeRangeReady: false`, no omissions, and three retained gaps, all on `docs/superpowers/runs/2026-11-04-graph-clone-identity/run.md:146` for inv-35d46ca1-baaf-4273-b92f-3cad9ad13172: incomplete claimed collection; invalid null-review bundle; incomplete split bundle (missing cluster report or slices). The failed assembly and its sidecar were left as found. No whole-range package was built or loaded.

The range changes 635 paths: 468 under `docs/superpowers/runs/2026-11-04-graph-clone-identity/` (run records, roast and review archives) and 167 elsewhere (production code, migrations, tests, TRUST-POLICY, docs). This pass read 22 production and migration paths, every captured hunk in each (100 hunks), selected along the requirement baseline: the schema delta, the reversible-retirement state machine and its fence, the G5 target-absence path, the key ledger and verdict table, the hook and client key gates, the service v3 dispatch, the write-driving retired sites, the operator finalization path, the replicated seat facts and the shell fingerprint judge. Two further production regions were read from the head tree outside the diff hunks (the retirement-fact materializers in `src/store/seats/publication.rs` and the key-conflict cause in `src/store/seats/continuity.rs`); their hunks are not claimed in coverage.

## Requirement baseline trace

- B1 (continuity key). Production entry: `ServiceOperation::ReserveContinuityKey` over service v3 (`src/store/service_controls.rs` to `continuity_keys::reserve`), gated by `ServiceSessionLevel::required_by` in `src/daemon/transport/service_connection.rs`; the hook reads `HERDR_THREADS_CONTINUITY_KEY` for top-level, non-subagent events only behind `seat.continuity_key_v1` (`src/cli/site_capability.rs`), carries it on the claim, the enrollment resolve and the continuity intent (`src/cli/hook.rs`), and a keyed lifecycle event now probes a resolved pane so a fresh agent's claim reaches the daemon during lag. The verdict table in `src/store/seats/key_verdict.rs` matches every row of spec 4.2 including the lag and claimless rows. An unread capability never yields a keyless enrollment (`enrollment_key`). The daemon-side reattach transaction (`src/store/seats/continuity.rs`, `cooperative.rs`, `resolution.rs`) was not inspected in this pass, so B1 is traced to the judge, not through the rebind itself.
- B2 (pane close retires immediately; undo reverses it). Every `Closed` retirement (closure proof, snapshot `BeginRetirement`, restored-pane close, G5 absence) goes through `begin_retirement_authored`, which for a key holder runs the same `retirement_transition` (state retired, bindings ended, wake work reset) and writes a `reversible_retirements` row instead of a job, so the close is immediate and nothing is settled. `UnretireSeat` (`reversible::unretire`) is owner-checked, flips the seat to unresolved with no target under the relaxed terminal trigger (the seat update precedes the row-state change, which the trigger requires), shifts owed deadlines, reopens an unavailability episode and authors `seat.unretired`. Frozen rows are excluded from every due scan touched (physical and sparse receipts, extension lapses, soft pokes, invitation due), and window facts write `pending` (`project_receipt`, `insert_invitation`). Delivered from production entry points, subject to the open deadline-shift items below.
- B3 (agent exit with pane open keeps the seat). No inspected path retires or unresolves a seat on agent exit: G5 counts only an absent target, the restored-pane watch only a closed terminal, and shell continuity only adds a recovery path. The test pinning this (`agent_exit_with_pane_open_keeps_the_seat`, `tests/clone_identity_sweep.rs:945`) was judged weak by the code roast and was not mutated (ledger line 146); the behaviour holds by inspection, not by a killing test.
- B4 (Herdr unchanged). The range touches no Herdr source; shell continuity reads Herdr's existing `pane_shell_pid` and the process table only.

## Integration observations

- The final/reversible choice has a single choke point (`begin_retirement_authored`), and the key release on final retirement has two writers (`insert_retirement_job`, `insert_replicated_retirement_job`), both of which release the key and drop the shell fingerprint. Operator `seat retire` on a reversible or waiting seat routes to `reversible::finalize`; `--replace` and operator retire are final and release the key. This composes correctly across the .3, .4, .7 and .22 merges as far as the inspected hunks show.
- Replicated facts: `seat.unretired` is a no-op unless a `reversible` row of the same `retired_at` stands, and `episode_fact_applied` treats any earlier `applied` journal row of the same kind, seat and episode as a copy. A no-op application is still recorded `applied`, so an out-of-order `seat.unretired` (applied as a no-op before its `seat.retired_reversible`) blocks every later copy of itself, and the peer row stays retired and frozen until a final `seat.retired`. The ht-ma6i.22 task review raised exactly this for fork-repair re-issue (ledger line 152) and graded it minor; it is a known product defect with no bead filed.
- G5 (`target_absence_evidence`) is gated on a Herdr host, a coherent publication not showing the target, the reconciliation marker naming the current recovery, and no kept claim naming the seat's sessions or its key; it resets on a shown target and hands over to the watch. Waiting (unretired) seats are excluded from saved-seat pages, hold arming and hold lift.
- The schema delta rebuilds `allocation_decisions` with the four new kinds via the 0012 pattern and relaxes `seats_retirement_terminal` only while a `reversible` row stands; the original trigger body is otherwise preserved.

## Ledger triage

- Roast round 1 (code roast) confirmed nine findings at this head and routed them to ht-ma6i.34, .35, .36 and .37; all four beads read `open`. The user's done criterion "The code super-roast findings are resolved." makes them landing conditions.
- Ledger line 152 (fork repair re-issues `seat.unretired` before `seat.retired_reversible`): misgraded as minor; it names a known data-state defect on a peer and has no bead. Triaged as a landing condition.
- Ledger line 84 (flaky `harness::hermes::assets::tests` Conflict failures under parallel cargo test): a flaky test outside this epic's code, not observed at base; needs its own bead, does not block on its own.
- Ledger line 145 (`scripts/freeze-peer-wire 3.0.0 --at v3.0.0 --check` still exits 1): open tooling failure, owner unassigned; follow-up.
- Ledger line 80 (`KEY_CONFLICT_CAUSE` prefix match quiets the hook): spec 4.2 says the hook's restore-hold outcome carries a hold, while the client now silences any detail starting with the key-conflict cause; whether a keyless hook in a key-held pane loses guidance it had was not traced here. Acceptable as a follow-up.
- Recurring blocker line 45 (stack parent not rebased before a dependent's cut, ht-ma6i.6/.7/.4): a process recurrence, already resolved by the landed merges; no code item.
- Recurring minor line 127 (missing evidence, ht-ma6i.6/.10/.20): evidence-retention gap; the affected scope stays untested until the sweep runs.
- The remaining `minor (deferred)` lines (doc drift, TDD order, file size, fixture duplication, the extra keyed round trip, audit-only columns, stale absence rows, the waiting-seat hold count, `available_at` not shifting) are accepted as follow-ups.
- Metrics: the ledger reports landed and completed counts that differ (14 vs 15, `completed-only [ht-ma6i.6]`, `git-only [ht-ma6i.6]`) and recovery gaps at progress.md lines 79, 142-144 and 192; the ledger is incomplete, so absence of a parked or fix-pass line there is not read as clean. ht-ma6i.6 is closed in the tracker and its merge commit 5d9454b91 is in the range.

## Verdict

not ready (bounded review only; nine roast findings open in ht-ma6i.34-.37, an unfiled peer-state defect, no full-suite run)

## Must fix before landing

- Code roast round 1 [Should-fix] src/store/seats/reversible.rs:370 and [Nit] src/store/seats/reversible.rs:412 are open (bead ht-ma6i.34, status open): the un-retire deadline shift for logical receipts has untested branches and is bounded by `created_by=i64::MAX` at home but by the payload's `unretired_at` on peer apply and own-log replay.
- Code roast round 1 [Nit] src/store/seats/cooperative.rs:1053 is open (bead ht-ma6i.35, status open): an alert-only key conflict is not cleared when its judged owner stops owning the pane, so SeatInspect shows a stale conflict.
- Code roast round 1 [Nit] src/identity/repair.rs:687 and src/identity/repair.rs:727 are open (bead ht-ma6i.36, status open): `record_shell_fingerprints` reads `pane_shell_pid` unfenced, so a cold restart between capture and read can record a new shell under the old incarnation and later judge it Continuous, and lane paging is untested.
- Code roast round 1 [Nit] tests/service/observation_lane.rs:1159, tests/clone_identity_sweep.rs:945, tests/service/continuity_key_session.rs:142 and docs/operations.md:249/286/290 are open (bead ht-ma6i.37, status open); the sweep test at :945 is the only pin of baseline row B3.
- Out-of-order `seat.unretired` leaves a peer seat frozen for good: `materialize_seat_unretired` applies it as a no-op when no `reversible` row of that episode stands, the journal row still reads `applied`, and `reversible::episode_fact_applied` then refuses every later copy, so a fork-repair re-issue that orders `seat.unretired` before `seat.retired_reversible` (ledger line 152) leaves the peer row retired with a `reversible` row until a final `seat.retired`; no bead tracks it (src/store/seats/publication.rs materialize_seat_unretired, src/store/seats/reversible.rs episode_fact_applied).

## Untested scope

- Whole-range review outstanding: this is a bounded captured-source pass; 145 of 167 non-run-doc changed paths and all 468 run-record paths were not inspected, including src/store/seats/continuity.rs, cooperative.rs, resolution.rs, publication.rs hunks, src/identity/repair.rs, src/client/service.rs, src/cli/mod.rs, src/cli/journal.rs, src/cli/caller_location.rs, src/protocol/service.rs, src/ports.rs, TRUST-POLICY.md, docs/programmatic-service-client.md, integrations/claude/mod/hooks/register.js, integrations/hermes/__init__.py and every test file.
- Review-input provenance gaps retained: run.md:146 inv-35d46ca1-baaf-4273-b92f-3cad9ad13172 incomplete claimed collection; invalid null-review bundle; incomplete split bundle (missing cluster report or slices); plus 34 out-of-range malformed collection records on the 2026-11-03 atomic-remote-handoff run.
- No full-suite measurement of this branch exists: the unfiltered nextest suite and leak checks (root acceptance `sweep`) are deferred to the caller after this invocation, so the branch is not treated as tested.
- B1 reattachment is traced to the verdict table only; the daemon-side rebind of a keyed claim, the kept-claim decision after lag and the hold refusal for keyless resolution were not read.
- A long-lived watch child whose first capability read failed caches "no capabilities" and then reads every key presentation as Unknown, which `presented_continuity_key` turns into no key; whether the Claude mod's watch child re-creates its client was not traced.
- Ledger line 84 flaky `harness::hermes::assets::tests` and ledger line 145 `scripts/freeze-peer-wire --check` exit 1 remain open runs with no owner.
- No live Herdr, Claude, Codex or Hermes run exercised the key path or reversible retirement in this review; the real-Herdr qualification the source design section 6 requires is outside this run.

## Deferred OK

- Ledger line 63: `has_nonretired_seat` counts a waiting seat, so a new incarnation on a host holding only waiting seats arms and then lifts a hold instead of taking the atomic empty-pass path.
- Ledger line 65: the un-retire shift does not move `available_at`, so soft pokes come earlier after a long freeze.
- Ledger line 80: the `KEY_CONFLICT_CAUSE` prefix match in `api_failure` quiets a key-conflict refusal for a keyless hook too; follow-up to confirm it does not hide restore-hold guidance.
- Ledger line 92: window messages are unread history with no delivery row on reattach, as spec 4.3 agreed.
- Ledger lines 153-154: the same-millisecond second episode limit of `episode_fact_applied` is documented only in a code comment, and the guard scans the journal with `json_extract` per apply of two rare kinds.
- Ledger lines 158-159: G5 never counts a seat whose target is shown but owned by another seat, and stale `restored_target_absence` rows are inert through the generation key.
- Ledger line 175: `retirements.closure_evidence` and `absence_evidence` are written but read on no production path.
- Ledger line 176: one extra round trip per keyed lifecycle event for the resolved-pane probe.
- Ledger lines 29, 130, 147, 166: documentation drift in docs/programmatic-service-client.md, TRUST-POLICY.md's amended log and docs/operations.md.
- Ledger TDD-order, fixture-substitution, file-size and duplicate-fixture minors (lines 40, 41, 59, 62, 71, 94, 167, 168).
