# Final review, cluster 27 (tests-service), epic ht-ma6i

Source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head 0755e609de0328e6d73584dd65eb2854e1fd7bf7. Both resolved with `git rev-parse --verify`. The caller checkout stayed on the head and clean throughout.

## Inputs read

Slices 000126 to 000139, covering all 14 files in the cluster: two new suites (`tests/service/continuity_key_session.rs`, `tests/service/reversible_session.rs`) and twelve files that gain `continuity_key: None` at construction sites.

Source opened only to check named risks: `src/store/seats/reversible.rs`, `src/store/continuity_keys.rs`, `src/store/service_controls.rs`, `src/store/service_substrate.rs`, `src/protocol/service.rs`, `src/store/control/retirement.rs`, `.config/nextest.toml`, and the test indexes of `tests/store/reversible_retirement.rs`, `tests/store/reversible_facts.rs`, `tests/store/continuity_keys.rs` and `tests/daemon/service_session_capability.rs`.

## Checks run at the head

- `heavy tb cargo test --locked --all-features --test service -- continuity_key_session reversible_session`, in a review-owned detached worktree at the head, seeded with a copy-on-write target: 10 passed, 0 failed, 2.19 s. The worktree and its target were removed afterwards and no process was left behind.
- `cargo fmt --all -- --check`, the CI format gate at `.github/workflows/ci.yml:325`: exit 0, no output.
- The full suite was not run. The ledger records that measurement as deferred to the caller.

## Baseline trace

- B1 (continuity key): `ReserveContinuityKey`, `ReleaseContinuityKey` and `ContinuityAlerts` are driven over a real `service_session_v3` socket into an in-process elected daemon, through `service_controls::operate` to `continuity_keys`. This is a production entry point, and the service half of the row holds at it. Key presentation by hooks and `KeyVerdict` leave this cluster and are the seam reviewer's to trace.
- B2 (pane close retires, undo reverses): `UnretireSeat` and `FinalizeRetirement` are driven the same way into `reversible::unretire` and `reversible::finalize`, and the deadline lane settles the job with no further call. The undo half holds at this entry point. The retire half does not run here: `make_reversible` seeds the state by SQL. Pane close through `begin_retirement_authored` is the seam reviewer's to trace; `tests/store/reversible_retirement.rs::closed_retirement_of_key_holder_writes_reversible_row_and_no_job` covers it at store level.
- B3 (agent exit keeps the seat): the design records no threads change for this row, and nothing in this cluster alters it.
- B4 (Herdr unchanged): the cluster changes tests only, and `UnavailableHost` implements the existing `HostPort`.

## Analysis of the two new suites

- Spec conformance holds for what the suites assert. Reserve is idempotent for the owner. A bound key refuses release with `Conflict`. A key bound elsewhere is `Conflict`, and a retired seat is refused. The binding moves nothing and records `service_reservation`. `Unauthorized` comes before `NotFound` for un-retire and finalize. Finalization keeps the original cutover, releases the key, removes the row and records `service_finalize`.
- Operation journaling holds through the session: a replay returns the journaled result even after the key row is gone, a reused key with another payload is `OperationPayloadMismatch`, and refusals leave no journal row, with the one gap listed below.
- The seeded `reversible_retirements` row uses a reason and closure inputs that `read_row` accepts, so the session suite does not depend on a shape the fence could not produce.
- Layers below the session are covered elsewhere, and I confirmed each by name: the v1 and v2 refusal of every v3 operation (`tests/daemon/service_session_capability.rs`), the `SeatUnretired.generation` value (`unretire_clears_retired_fields_and_proof_and_opens_an_episode`), `retired_seq` advancing (`finalization_advances_retired_seq`), and the own-history hold (`restore_hold_refuses_them_as_retryable`).
- Fixture lifecycle is sound. The daemon stops before its root is removed, the lock is released between the two fixtures in `alerts_answer_only_v3`, temp paths come from `temp_path`, and no child process is spawned.
- The mechanical `continuity_key: None` edits change no test's meaning. With `skip_serializing_if`, the kept claim JSON that `observation_lane.rs` seeds is unchanged.

## Findings

- none

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- No test joins a real pane close to the service undo: `reversible_session.rs` seeds the reversible state by SQL, and spec 4.8's end-to-end flow (reserve, keyed launch, close pane, send refused, un-retire, keyed relaunch, same seat, ACK deadline resumed, memberships intact) is absent at this head because `tests/clone_identity_sweep.rs` does not exist and ht-ma6i.11 is open; this path leaves the cluster, so it is the seam reviewer's to judge against B1 and B2.
- ht-ma6i.1 (G2) and ht-ma6i.4 (G5) are also open with no merge on this branch, so nothing in this head exercises them; they are not baseline rows but they are in the spec's goal, and judging that belongs to the seam reviewer.
- The session suites use unpublished seats only, so no `seat.retired` or `seat.unretired` fact is authored through a real service session; `tests/store/reversible_facts.rs` covers authoring at store level.
- Reserve with a seat is not driven over the session for a named seat, an hc-host seat or a seat holding another key; `tests/store/continuity_keys.rs` covers the binding rules at store level, and I did not check each of those three cases there individually.

## Deferred OK
- `continuity_key_session::` and `reversible_session::` take `IN_PROCESS_DAEMON`, but `.config/nextest.toml` limits the `service-daemon` group to `resolution`, `composition`, `operator` and `graph_d2`, so under nextest they run beside other daemons with 5 s publish and exchange waits and a 10 s job wait; AGENTS.md says a mutex does not span processes, but `graph_send` already runs ungrouped the same way and these ten tests take 2.19 s together, so add them to the group only if they flake.
- `unretire_authorization` checks `journaled(operation)` after the refused un-retire but not after the refused finalize that reuses the same key, so a journaled finalize refusal would pass this test; adding the same assertion after the finalize call closes it.
- `service_authors` allows one reserved author per instance (`ON CONFLICT(instance_id)` in `ensure_reserved_author`), so the "another service" refusals are reachable only through the `PRAGMA foreign_keys=OFF` seeds these tests use, and a second registering service acts as the same owner; this is consistent with the cooperative model, but the graph side should not read these tests as isolation between services.
- `graph_d2.rs` and `graph_send.rs` carry the inserted `continuity_key: None,` and the closing `}))` at column zero; `cargo fmt --check` passes, so this is cosmetic.
