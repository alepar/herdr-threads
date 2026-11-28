# Final review: cluster 20 (src-root)

Source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head c3e9ab0becd137adb68f23a1c9e1a87669d03168 (both verified with `git rev-parse --verify`). Caller checkout observed at HEAD c3e9ab0be with a clean status. Cluster files: `src/app.rs` (slice 000070) and `src/ports.rs` (slice 000097). Spec: `docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-26-per-seat-config-side-wake-design.md`, which has a `## Requirement baseline` (B1-B10, S1).

## Scope of the cluster

- `src/app.rs`: the daemon no longer passes the node `programmatic_wake` into `QueueWakeRuntime` (`.with_programmatic_wake(config.programmatic_wake())` removed). The runtime is still built whenever a guarded state dir exists, and the per-seat key gates it.
- `src/ports.rs`:
  - `WakeAttentionWitness.programmatic_wake` with `with_programmatic_wake` and a getter.
  - `StorePort::seat_config_operator`, whose default returns `Unsupported`.
  - `HookPullOutlook::contacted_since_session`, whose default returns `None`.
  - `NotificationPort::take_queue_root`, whose default returns `None`.

## Baseline tracing for this cluster

- B1 (replace the node-level setting with a seat-level one), production path at head:
  - `src/service/config.rs` no longer has `programmatic_wake`.
  - `src/daemon/settings.rs` keeps it as `Option<bool>`, parsed and inert.
  - `src/app.rs` builds `QueueWakeRuntime` without the flag.
  - Selection gates on `store::seat_config::programmatic_wake_on` (`src/store/wake_intents/selection.rs:174`, `src/store/wake_intents.rs:285`).
  - `git grep programmatic_wake` at head finds no other consumer of the node value except `doctor`, which reports it as obsolete. The part of B1 in this cluster is delivered.
- B2-B7 (registry, CLI, tiers): the only part in this cluster is the `seat_config_operator` port. `SqliteStore` implements it (`src/store/mod.rs:1737`), and `src/service/dispatch.rs:1660` calls it in production. `SqliteStore` is the only production `StorePort` impl; the one other impl is the test `PausedStore` in `tests/daemon/harness_evidence.rs`. So the `Unsupported` default cannot hide the human-tier path in production. The CLI and service routing are other clusters, and the seam reviewer traces them.
- B8 (queue wake reaches the root with `/side` open), receipt side:
  - The production `NativeWakeDispatcher` (`src/notification/dispatch.rs:834`) forwards `take_queue_root` to `QueueWakeRuntime::take_root`.
  - Roots are recorded only on `Submitted`/`Unknown` queue dispatches, and the map is bounded by `ROOTS_MAX`.
  - The production `DeliveryPullState` (`src/service/delivery_pull.rs:215`) implements `contacted_since_session` from per-(seat, session) contacts.
  - The scheduler (`src/scheduler/mod.rs:1016-1022`) uses the root-scoped check when `root_session` is `Some`, and the seat-scoped check otherwise. That matches the amended receipt mechanism: a queue-delivered attempt is root-scoped, and a native or fallen-back attempt is seat-scoped.
  - The `None` defaults keep the earlier behaviour: an attempt with no queue root and transport `Queue` is not awaited, and `== Some(false)` never suppresses on unknown.
- Witness flag:
  - `load_candidate` sets it from the seat key (`src/store/wake.rs:272`). On any read error, `programmatic_wake_on` returns false, so it fails closed.
  - The scheduler compares the flag with `AwaitingReceipt.programmatic_wake` to lift `unreceived` suppression on a flip (`src/scheduler/mod.rs:1008-1013`).
  - `WakeAttentionWitness` derives `PartialEq`, but no witness-equality comparison exists in `src`. `valid_at`/`valid_for` compare named fields only, so the new field cannot make reservations look stale.

## Test coverage observed

- `tests/store/wake.rs:914`: `load_candidate_witness_carries_the_seat_programmatic_wake`.
- `tests/scheduler/wake_receipt.rs`:
  - `programmatic_wake_flip_lifts_unreceived_and_the_next_attempt_queues`
  - `a_queued_root_wake_with_only_side_contact_is_unreceived_and_not_retyped`
  - `root_session_contact_settles_a_queued_wake_whatever_binding_made_it`
  - `a_queue_selected_attempt_that_fell_back_to_native_is_seat_scoped`
- `tests/service/delivery_pull.rs:14821+`: `contacted_since_session` scoping.
- `tests/service/seat_config.rs`: the operator path through dispatch.

## Findings

- none

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- Daemon-level composition in `src/app.rs`: a test that a running daemon with node `programmatic_wake: true` in settings.json and the seat key off selects native, and with the seat key on selects queue without a restart. It is covered per seam (`src/notification/queue.rs` tests, `tests/notification/queue_runtime.rs`) and by the live `--queue-proof` wake e2e. The full daemon wiring is the seam reviewer's to trace.

## Deferred OK
- `src/ports.rs:2898` `#[serde(default)]` on `WakeAttentionWitness.programmatic_wake` has no effect because the struct derives only `Serialize` ("No wire decoder"); it seems to exist only to satisfy `scripts/check-wire-compat`, and a short comment saying so would stop readers expecting a decoder.
- `src/store/wake.rs:1233` `reserve_poke` builds its witness with `from_complete` and no `with_programmatic_wake`, so poke reservations always carry `programmatic_wake=false`; this does no harm today because `try_poke` never inserts an `AwaitingReceipt`, but a future consumer of the poke witness would read a wrong value.
