# Final review: cluster 21 (src-scheduler)

Source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head c3e9ab0becd137adb68f23a1c9e1a87669d03168 (both verified with `git rev-parse --verify`). Caller checkout observed at c3e9ab0be, clean, before the review. The cluster is one file, `src/scheduler/mod.rs` (slice 000107), changed by commits 0d0e2aaf4 (ht-z41e.4) and b38e8080c (ht-z41e.22).

## Scope and spec reference

Spec: `docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-26-per-seat-config-side-wake-design.md`, which has a `## Requirement baseline` section. In this cluster the relevant parts are §Selection "Receipt mechanism" (amended 2026-11-27), §Queue per-seat flag "read per wake attempt, so a change applies without a daemon restart", and baseline rows B1 (seat-level programmatic_wake) and B8 (wake goes to the top-level session while a `/side` chat is open).

## What the cluster changes

- `AwaitingReceipt` gains `programmatic_wake` (the seat flag from the reservation's attention witness) and `root_session` (the queued attempt's root native session UUID).
- The retry-due `silent` check now also requires the flag to be unchanged. When `root_session` is `Some`, it asks `hook_pull.contacted_since_session(seat, root, since)`. A native attempt still uses the seat-scoped `contacted_since`.
- After `run_attempt`, the wake path takes `take_queue_root` only when `take_transport` reported `Queue`. It now inserts an `AwaitingReceipt` for a Composer attempt or for a Queue attempt with a known root, on `Submitted`/`OutcomeUnknown`. The poke-only path drains `take_queue_root` so no root is retained for it.

## Tracing against the spec

- Receipt scope follows the transport actually used. `QueueWakeRuntime::submit_captured` (src/notification/queue.rs) records a root only for `QueueDispatch::Submitted | Unknown`. `NativeWakeDispatcher` (src/notification/dispatch.rs) rewrites the transport to `Composer` on `Native`/`NativeFallback`. So a queue-selected attempt that fell back to native leaves no root and keeps seat scope, as the spec requires. A queue attempt with no root (the runtime is absent, or `ROOTS_MAX` was cleared) is not awaited, which is the pre-change queue behaviour.
- Root re-registration: contact is keyed on (seat, native session) in `DeliveryPullState::record_contact`, fed from `contact_claim(&command)` with `claim.native_session` at `handle_with_output` entry (src/service/dispatch.rs:2140). A new binding id with the same root UUID therefore settles the attempt.
- Side contact: a side session UUID never matches the root key. Side contact therefore neither settles nor lifts `unreceived`, and it does not re-arm a retype, because the `silent` lift uses the same scoped predicate.
- `armed` stays `outcome == Submitted`. `arm_unsubmitted` fires only on a composer `SubmissionVerification::Unsubmitted`, which the queue path never produces. A queued `OutcomeUnknown` is therefore dropped at the next due retry and retried, as before the change.
- The flag lift is consistent with "read per wake attempt": a parked `unreceived` native wake is re-admitted after a flip, and the store's selection reads the new flag (store/wake.rs:272 `with_programmatic_wake(programmatic_wake_on(..))`).
- Docs at head match the code: AGENTS.md (closed-loop bullet, line 114) and TRUST-POLICY.md ("Arrival", around line 4309) describe the root-scoped receipt and the native-fallback seat scope.
- Baseline rows: B1's per-attempt flag read and B8's arrival side are served correctly by this cluster's part. Queue root selection, the per-seat flag store and the contact producer live in other clusters (store/wake_intents, store/seat_config, service/delivery_pull, service/dispatch), so tracing them end to end is the seam reviewer's job.
- The tests in tests/scheduler/wake_receipt.rs pin each spec bullet: side-only contact stays unreceived and is never retyped, any binding with the root UUID settles, native settles on any contact, a fallback-to-native attempt is seat-scoped, and a flag flip lifts `unreceived`. The task report records the focused lib and combined runs passing, plus clippy and the neutral/wire checks. They were not re-run here (read-only review, no duplicate qualification).

## Findings

- none

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- No scheduler test covers a queued `OutcomeUnknown` attempt with a root: it is inserted unarmed and must be dropped and retried at the next due step, not marked `unreceived`.
- `programmatic_wake_flip_lifts_unreceived_and_the_next_attempt_queues` exercises the flip lift only on a native (Composer) awaiting. A flip that lifts a root-scoped queued awaiting is not tested, though it is the same predicate.
- The poke-only path's `take_queue_root` drain (src/scheduler/mod.rs:1294) has no test showing that no root is left behind for a poke attempt.
- There is no live evidence that a real Codex root, after a queued wake, contacts under the same native session UUID the intent targeted. That is the seam with service/dispatch `contact_claim`, and side-queue/group in the wake e2e is the live check.

## Deferred OK
- A `programmatic_wake` flip lifts `unreceived` suppression even on a seat that cannot be queue-selected, such as a non-Codex harness or a seat with no named root. That allows one extra native retype per human config flip, which is bounded and operator-triggered.
- `QueueWakeRuntime` clears every retained root at once when `ROOTS_MAX` (256) is reached. That drops queued attempts back to "not awaited", which is the pre-change behaviour, and it is reachable only if the scheduler stops taking roots.
