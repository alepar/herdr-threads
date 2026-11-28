# Final review: cluster 17 (src-notification)

Source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head c3e9ab0becd137adb68f23a1c9e1a87669d03168 (both verified with `git rev-parse --verify`). Inputs: slices/000095.diff (src/notification/dispatch.rs), slices/000096.diff (src/notification/queue.rs), digest.md, and the root spec docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-26-per-seat-config-side-wake-design.md (Requirement baseline B1-B10, S1). No caller files were edited; no worktree was created; no tests were run (read-only review of the diff and of the head tree via `git show`).

## Scope of the change

- queue.rs: `QueueWakeRuntime` loses the process-wide `programmatic_wake` flag and `with_programmatic_wake`. `attempt` now passes `use_queue` alone to `queue_wake_selection` and gates the queue on `selection.programmatic_enabled`, which the store reads from the seat's own `programmatic_wake` inside the selection transaction.
- queue.rs: `submit_captured` is split into a wrapper plus `submit_prepared`. The wrapper retains the target root native session (`key.native_session`) per attempt id when the dispatch is `Submitted` or `Unknown`, in a mutex-guarded map bounded at `ROOTS_MAX = 256` (cleared wholesale on overflow). A new `QueueWakePort::take_root` (neutral default `None`) hands it out once.
- queue.rs: the preparation now carries `binding.side_sessions`, so the intent records the seat's side session UUIDs (spec Qualification: observable record names root and sides).
- dispatch.rs: `NotificationPort::take_queue_root` delegates to the queue port.
- New unit test: the seat opt-in flipped off inside the guard (after selection and prepare, before arm) refuses arm, records a never-armed NoEffect, leaves no fence, settles the attempt as `Native`, and retains no root.

## Requirement baseline trace

- B1 (replace node-level setting with seat level): delivered within this cluster. The runtime no longer holds any node flag; the queue is selected only when `queue_wake_selection` reads the seat's `programmatic_wake` as `on` (`src/store/wake_intents/selection.rs:174`, fail-closed: an unreadable value reads off via `programmatic_wake_on`), and the arm transaction rechecks it (`src/store/wake_intents.rs:285`), which the new flip test exercises through the real store. Daemon wiring no longer calls `with_programmatic_wake` on the runtime (grep of src/daemon at head); the node setting survives only as an inert parsed field in `src/daemon/settings.rs`. Node-settings/doctor handling and the CLI writer are other clusters' and the seam reviewer's to trace.
- B8 (wake-ups directed to the top-level session in the background with /side present): this cluster's share is handing the scheduler the queued target's root session for root-scoped receipt and recording side sessions on the intent. The root is the selected binding's `scope.native_session`; it is retained only on `Submitted`/`Unknown`, so a stale-authority or flag-off fallback to native keeps the seat-scoped receipt, as the spec's receipt-mechanism amendment requires. Consumption is in `src/scheduler/mod.rs:1086-1090` (taken only when transport is Queue; also drained on the poke-only path at 1294): the scheduler-side AwaitingReceipt and contact book are the seam reviewer's to trace.
- B2-B7, B9, B10: not touched by this cluster.

## Analysis of failure modes

- Missing root on a queued Submitted/Unknown (mutex poisoned, or the overflow clear): the scheduler sees `transport == Queue` with `root_session == None` and records no closed-loop receipt, which is the pre-run behaviour for queue wakes (no generic-contact receipt). It never degrades to the seat-scoped check, so a side binding's contact still cannot settle a queued root wake. Fail-safe direction.
- Overflow: every take path runs synchronously after `run_attempt` (wake and poke), and active prompts are bounded by `MAX_ACTIVE_PROMPTS = 4`, so 256 retained entries implies a leak; the wholesale clear only bounds memory in that case.
- Leak sources: an entry leaks only if a root is stored while the transport map reads non-Queue, which needs a poisoned transports mutex; negligible.
- `Err` from `submit_prepared` after a possible add effect (e.g. `result_slot.publish` failing) returns no root and the dispatcher refuses; this path and its outcome mapping predate this change.
- Selection cost when the seat flag is off: the flag is checked before the binding census, so a seat with queue wake off pays only the fence read plus one config read, as before.

## Findings

- none

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- The `Unknown` branch of root retention is not asserted: `accepted_effect_with_result_write_failure_is_unknown_and_never_native` (src/notification/queue.rs) checks the Unknown dispatch but never calls `take_root`, so a regression that retained roots only for Submitted would pass unit tests.
- `NotificationDispatcher::take_queue_root` (src/notification/dispatch.rs:834) is exercised by no test with a real dispatcher and queue runtime; scheduler receipt tests (tests/scheduler/wake_receipt.rs, tests/scheduler/queue_transport.rs) use fake notifiers that return a canned root.
- The `ROOTS_MAX` overflow clear has no test.

## Deferred OK
- `ROOTS_MAX` overflow clears all retained roots, including any in flight; reachable only after 256 leaked entries (active prompts bounded at 4, every path takes), and the consequence is no receipt wait for that attempt, never a seat-scoped one.
- A poisoned roots mutex silently drops the root (no diagnostic); the attempt then behaves as a pre-run queue wake with no closed-loop receipt.
