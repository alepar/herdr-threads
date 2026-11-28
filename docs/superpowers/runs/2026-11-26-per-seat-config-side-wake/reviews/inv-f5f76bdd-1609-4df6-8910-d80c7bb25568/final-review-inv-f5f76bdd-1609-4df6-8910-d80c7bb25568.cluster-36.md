# Final review: cluster 36 (tests-service)

Source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head c3e9ab0becd137adb68f23a1c9e1a87669d03168 (both verified with `git rev-parse --verify`; review source bound to inv-f5f76bdd-1609-4df6-8910-d80c7bb25568). Epic ht-z41e, spec docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-26-per-seat-config-side-wake-design.md at head. Read only from git objects; no worktree created, nothing built or run.

## Scope read

- Slices 000186-000202 for the 17 files of the cluster.
- 15 of the 17 files change only mechanically: each `CallerClaim` / `LifecycleEvent` literal gains `auxiliary_session: false` (cooperative.rs has 25 of these and nothing else). These follow the new neutral claim field from ht-z41e.21 and change no behavior.
- Real content is in two places: `tests/service/seat_config.rs` (new, 629 lines, compiled as `service::dispatch::seat_config_tests` through `#[path]` at src/service/dispatch.rs:2935) and one new test in `tests/service/delivery_pull.rs` (`contact_is_recorded_per_seat_and_per_native_session`).

## Spec trace for this cluster

- B3/B6/B7 (agent path writes nudge_threshold, reads open to every caller, human-tier keys gated): the production path is `ht config seat set` -> src/cli/config_seat.rs (agent tier with `current_claim`; `human` argv[1] sends tier human, no claim) -> `Command::SetSeatConfig` -> `DomainService::seat_config_write` (src/service/dispatch.rs:1628). There the agent tier goes through `PermitMutation` / `cooperative_mutation`, the same gate as `SetNudgeThreshold`, and the human tier needs the elected operator peer before `seat_config_operator`. The service tests drive this dispatch entry and assert each branch:
  - parity of `SetSeatConfig nudge_threshold` with the `SetNudgeThreshold` alias for pane Claude, pane Codex, named and terminal seats: threshold, `wake_on` mirror, `wake_work` re-judgement, `WakeStatus`, stale-claim refusal code and `cooperative_top_level` audit provenance;
  - agent refusals: the human-tier key (exact `ht human` line), another seat ("only the caller's own seat"), a subagent (client validate plus the daemon's own `SubagentReadOnly`), unknown key, out-of-range `-5`, no claim, a non-owner peer, and a read-only binding. Each refusal writes nothing;
  - human tier: an operator decision on a seat with no binding, provenance `operator:local-user:501`, the A5 exception (the operator sets another seat's nudge_threshold and the wake is re-judged), unset deletes the table row, and refusals for a non-owner peer, a missing seat, an unknown seat and the bad value `true`;
  - reads carry no claim (so no subagent refusal applies), cover any local seat and show defaults and writer;
  - a remote seat is refused with one line on both read and write.
- Receipt mechanism (amended 2026-11-27): `DeliveryPullState::record_contact` / `contacted_since_session` (src/service/delivery_pull.rs:156-231) keep a monotonic max per (seat, native session) beside the seat-scoped time. The new test shows that side contact is visible seat-wide but not root-scoped, that root contact settles from its own time, and that an out-of-order earlier timestamp does not lower it. Keying by native session rather than binding matches the spec's "any binding with that native session UUID". How the scheduler consumes this belongs to the scheduler cluster and the seam reviewer.
- Error-handling rows outside this cluster (operator write to a finally retired seat, strict range vs alias clamp at store level, `load_candidate` witness) are tested in store and other clusters; src/store/seat_config.rs:499 holds the `seat retired:` refusal. Whether they compose is the seam reviewer's call.

## Test quality notes

- The fixture models the terminal class by relabeling the host kind to `hc` while the claim uses the test pane locator. This is already a recorded deferred minor of ht-z41e.2.
- `programmatic_wake_on_a_seat_with_no_transport_is_accepted_and_inert`: its doc says it kills "changes anything beyond its own row", and its comment says "Only the wake is re-judged". It asserts only that the threshold and mirror are unchanged. It never checks that the wake was re-judged or that nothing else changed.
- Several mechanical edits add an explicit `auxiliary_session: false` to struct-update literals that already spread a base claim (`..prior.clone()`, `..sub`, `..s.main.clone()`, `..claim("sess-c")`). The base is false in every case, so this changes nothing today. It does pin the field against the base if a later fixture sets it.
- Execution evidence: evidence/ht-z41e.3/lib-tests.log shows all 8 seat_config_tests passing at ht-z41e.3. The file was edited after that (0d0e2aaf4, 136484396, b38e8080c), and the progress ledger defers the full-suite sweep at head to the caller. This review did not rerun the tests, to avoid duplicating that sweep.

## Findings

- tests/service/seat_config.rs:620 `programmatic_wake_on_a_seat_with_no_transport_is_accepted_and_inert` claims more than it checks. It asserts only that the threshold and mirror are unchanged, not the wake re-judgement its comment names or the absence of other effects. Nit; the behavior is covered at store level by ht-z41e.4.
- Redundant `auxiliary_session: false` in struct-update literals over a base claim (tests/service/composition.rs:637, tests/service/cooperative.rs struct updates, tests/service/mod_digest_flag.rs:287, tests/service/resolution.rs:1963). Harmless now. Nit.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- The cluster's tests were not run by this review at head c3e9ab0be; the last recorded run of seat_config_tests is the ht-z41e.3 log, and later commits edited the file, so the caller's deferred full-suite sweep must cover them.
- A human-tier SetSeatConfig/UnsetSeatConfig carrying a subagent claim (refused through `accountable_claim`, src/protocol/commands.rs:1447) is not exercised at service level.
- A human-tier unset of the column-backed nudge_threshold (default written, wake_on mirror reset, operator audit row kept) is not asserted at service level; only the agent unset and the table-backed programmatic_wake unset are.
- Operation-id replay of operator seat-config writes (`cooperative_payload_hash("set_seat_config_operator", ...)`, src/store/seat_config.rs:431) is not exercised at service level.
- `programmatic_wake_on_a_seat_with_no_transport_is_accepted_and_inert` does not assert the wake re-judgement or the absence of other effects that its doc claims.

## Deferred OK
- Terminal seat class modeled by relabeling the host kind to hc in tests/service/seat_config.rs (already a recorded ht-z41e.2 deferred minor).
- Redundant explicit `auxiliary_session: false` in struct-update test literals over a base claim whose field is already false.
