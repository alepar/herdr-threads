# Final review: cluster 23 (src-store), epic ht-z41e

Source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head c3e9ab0becd137adb68f23a1c9e1a87669d03168 (both verified with `git rev-parse --verify`). The caller checkout read HEAD = c3e9ab0be with a clean status at review time. Inputs: the 14 cluster files listed for `src-store` in clusters.json, the digest, the design doc `docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-26-per-seat-config-side-wake-design.md` (spec of `bd show ht-z41e`), and the matching store tests at head. I read source and tests at the reviewed head and ran no tests myself. I rely on the evidence the implementers recorded.

## Scope read

- `src/store/seat_config.rs` (new): registry-driven get, agent-tier write (accountable cooperative transaction on the claim's own seat) and human-tier write (idempotent operator decision), audit rows for the column-backed key, final-retirement cleanup, `programmatic_wake_on` (fails closed), `seats_with_value` for doctor.
- `src/store/seats/threshold.rs`: `write_threshold` made crate-visible with a `refuse_readonly` flag. `write_column` is the single column write: it updates `seats.nudge_threshold` and `wake_on`, calls `note_new_attention` on a change, and upserts the audit row on every call.
- `src/store/wake_intents/selection.rs`: reads the per-seat `programmatic_wake` inside the selection transaction, runs a bounded census (64 rows), and uses `process_root` (one harness process, adapter-named root, lone concurrent or auxiliary binding never a target), with side sessions passed through to the intent.
- `src/store/wake_intents.rs`: `side_sessions` column persisted and decoded, plus a seat flag re-read at arm (the stale-authority path).
- `src/store/wake.rs`: `load_candidate` fills the attention witness's `programmatic_wake` per seat.
- `src/store/bindings.rs`: sticky `mark_auxiliary` on every top-level `resolve_claim_with_hooks`. It skips subagent and person claims, never writes 0 and never touches `concurrent_session`.
- `src/store/control/retirement.rs`: both final-retirement inserters drop `seat_config` rows.
- `src/store/queries.rs`, `src/store/mod.rs`: the `GetSeatConfig` query, `SetSeatConfig` and `UnsetSeatConfig` permit mutations with matching cooperative payload hashes and obligations, and the operator port.
- `src/store/schema.rs`: NodeDeltas 20 to 22 (`seat_config` STRICT table with a seats FK, `wake_intents.side_sessions`, `occupant_bindings.auxiliary_session` with a CHECK constraint and default 0).
- Remaining files: mechanical `auxiliary_session: false` claim fields and the generalized `require_local_seat_setting`.

## Requirement baseline trace (store side)

- B1 (seat level replaces node level): the queue path reads the seat key at three points. `load_candidate` feeds the scheduler's `use_queue`, `queue_wake_selection` re-reads it in its transaction, and `arm` re-reads it before arming. None of them reads a node flag. Removing the node setting (daemon/settings, doctor) belongs to other clusters.
- B2 with S1 (closed typed registry): the store names no key in SQL except the `nudge_threshold` audit label and the `PROGRAMMATIC_WAKE` constant. A `Table` key needs no store code, and `a_new_table_key_needs_no_schema_change` covers that. Unknown keys and bad values are refused through `protocol::seat_config::key` and `validate`.
- B3, B6 and B7 (read and write tiers): `get` serves any caller. `agent_write` refuses a human-tier key with the `ht human` spelling, and also refuses another seat, a subagent and a read-only binding. Agent-tier `nudge_threshold` goes through the same `write_threshold` as the aliases. `operator_write` accepts any key on any local seat under operator provenance. It refuses a remote seat, an unknown seat and a finally retired seat (`seat retired: <seat>`), and accepts a reversibly retired one. The CLI and service entry points, and the harness ask-rule, are for the seam and CLI reviewers.
- B8 (side routing): `process_root` and `mark_auxiliary` implement the amended Root-marker rules. A lone concurrent or auxiliary binding is never a target. Bindings across processes, or a rootless process, stay native. Busy sides have no blocking rule: only the root's native turn must be `Ended` in `select_binding`. The Codex `wake_process_root` predicate and the hook-to-claim evidence path are outside this cluster (seam).
- B10: the node setting has no migration onto seats, the agent tier writes its own seat while the human tier may name any seat, and `nudge_threshold` is a key over the existing column with no data migration.
- B4, B5 and B9 have no store component in this cluster.

## Correctness notes

- The operator precheck (local, finally retired, known) runs in the same IMMEDIATE transaction as the write (`execute_idempotent_transaction_presented`), so it cannot race a final retirement. A refused write records no operation row, and a test asserts this.
- `seats` rows are never deleted, so the new `seat_config` foreign key cannot block an existing path. Final retirement is never undone (`retirements` rows are never deleted), so the `seat retired` refusal is permanent only where it should be.
- `CallerClaim.auxiliary_session` uses `skip_serializing_if = Not::not`, so cooperative payload hashes of existing operations stay unchanged.
- On an agent replay of the column-backed key, `write_threshold` returns the stored `NudgeThreshold`, which is re-mapped to `SeatConfigWritten`. On a table key, the stored `SeatConfigWritten` is returned as is. Both are covered by `agent_replay_returns_the_recorded_result_and_a_changed_payload_conflicts`.
- When selection's early returns are taken (`!eligibility_needed`, or the flag off), they still carry the retained native fences, so native fallback fencing is unchanged.
- Store tests at head cover each spec Testing bullet for this cluster:
  - registry, defaults, column backing, audit rows, strict range versus alias clamp, and attention re-judgement for both column and table keys;
  - tiers and read-only bindings;
  - remote seats, final and reversible retirement;
  - per-seat flag isolation and toggling, read failure falling back to native, and `load_candidate` filling the witness;
  - root, side, fork, rootless-root, cross-process, plain-call, non-naming-adapter and census cases, all enrolled through real Codex payloads;
  - auxiliary marking that is sticky, scoped to its binding row and limited to top-level bindings.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- This review ran no tests. The store test results come from the implementers' recorded evidence (evidence/ht-z41e.3, evidence/ht-z41e.11 and the task reports).
- The Codex `wake_process_root` predicate and the bridge/CLI path that sets `CallerClaim.auxiliary_session` on every hook event are outside this cluster. They are the seam reviewer's to trace from a real Codex hook to `mark_auxiliary`.
- The service-side authorization that sends `seat_config_operator` only for a peer matched as the elected owner is outside this cluster (service/dispatch). The store trusts its caller for it.

## Deferred OK
- src/store/seats/threshold.rs `write_threshold` doc comment still says "setting the stored value again records nothing", but `write_column` now upserts the audit row on every call (doc drift only).
- src/store/wake_intents.rs `matching` compares the whole preparation, including the diagnostic `side_sessions`. A second prepare of the same attempt after a side binding enrolls returns `wake intent frozen fields differ` instead of `Existing`, which the runtime maps to Refused. Neither sends a wake, and the only production prepare runs once per attempt (src/notification/queue.rs:200).
- src/store/seat_config.rs:491 final-retirement check is inline SQL rather than a helper shared with control/retirement.rs (already recorded as a Task 13 deferred minor).
