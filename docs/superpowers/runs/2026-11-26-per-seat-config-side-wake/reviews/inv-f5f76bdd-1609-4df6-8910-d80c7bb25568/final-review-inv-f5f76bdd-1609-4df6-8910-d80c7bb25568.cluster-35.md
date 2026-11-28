# Final review: cluster 35 (tests-scheduler)

Source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head c3e9ab0becd137adb68f23a1c9e1a87669d03168 (both verified with `git rev-parse --verify`; review source bound through `bind-review-source`, exit 0). Caller checkout was not switched or modified; no worktree was needed (all reads via `git show <head>:<path>`).

Cluster files: `tests/scheduler/dispatch.rs`, `tests/scheduler/poke_flow.rs`, `tests/scheduler/wake_receipt.rs` (slices 000183-000185). Spec: `docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-26-per-seat-config-side-wake-design.md` (root epic ht-z41e), task ht-z41e.4 acceptance for the flip test.

## What the cluster tests, and against what production code

- `dispatch.rs::side_composer_refusals_retry_past_the_ladder_length_until_a_wake_submits` (Section 3): drives `DispatchState` through 12 `Refused(Unsafe)` outcomes with `restore_prior_guard`, asserting each retry is due within 30 s (the ladder never advanced to its 300 s step), the seat stays reservable, and a `Submitted` clears `refusal_attempts`. The cause matches production: the native host refuses a non-empty composer with `refuse("composer is nonempty")` (`src/host/native.rs:2222`), mapped through `refusal_for_error` to `RefusalCause::Unsafe` (`src/notification/dispatch.rs:186`), and the scheduler restores the prior guard on refusal (`src/scheduler/mod.rs:1389,1434`). That the side placeholder now reads as non-empty is the composer cluster's, not this one's.
- `poke_flow.rs`: mechanical `auxiliary_session: false` on a `CallerClaim` literal for the new field. Nothing to judge.
- `wake_receipt.rs` (Section 2 "Receipt mechanism"): the new `queued_retry` harness runs the real `Scheduler::drive_wakes` with the real `DeliveryPullState` as `HookPullOutlook`, so the receipt decision under test is the production `AwaitingReceipt.silent` branch (`src/scheduler/mod.rs:1008-1022`) and the production per-(seat, session) contact book (`src/service/delivery_pull.rs:156-230`). Cases: side-only contact marks a queued root wake `unreceived` once and never retypes it over three 300 s steps; root contact (with side contact also present) settles it; a native wake settles on any binding's contact; a queue-selected attempt that fell back to native, with no contact, is marked `unreceived`.
- `programmatic_wake_flip_lifts_unreceived_and_the_next_attempt_queues`: both flip directions lift a parked `unreceived` native wake, and an unchanged flag keeps it parked. Production: `AwaitingReceipt.programmatic_wake` is captured from the reservation's witness, which is the candidate's witness (`src/store/wake.rs:870-936`), filled by `with_programmatic_wake(seat_config::programmatic_wake_on(..))` in `load_candidate` (`src/store/wake.rs:272`).

## Requirement baseline trace (cluster share)

- B8 (wake reaches the root with `/side` open): the receipt half is traced from production. Contact is recorded for every seat-claiming call with the claim's own `native_session` (`DomainService::handle_with_output`, `src/service/dispatch.rs:2137-2145`); the queue root is handed over by `QueueWakeRuntime::submit_captured` only for `Submitted`/`Unknown` dispatches and taken through `NativeWakeDispatcher::take_queue_root` (`src/notification/dispatch.rs:834`, `src/notification/queue.rs:173-183,466`); the scheduler root-scopes only when `take_transport` reports `Queue` (`src/scheduler/mod.rs:1087-1089`). Root selection and the `/side` marker are other clusters' and the seam reviewer's to trace.
- B9 (native defers on a side chat): the scheduler's half (refusals retry on the refusal backoff, never climb or exhaust the ladder) is pinned; the composer classification is the composer cluster's.
- B1 (per-seat flag): the scheduler's reaction to a per-seat flip is pinned; that the next attempt actually selects the queue is in the selection/queue tests outside this cluster (seam).
- No baseline row is shown undelivered by this cluster.

## Findings

- nit: `a_queue_selected_attempt_that_fell_back_to_native_is_seat_scoped` uses a notifier identical to `a_native_wake_settles_on_any_binding_contact` (Composer, root `None`) and records no contact, so a root-scoped judgement would also mark it `unreceived`; it does not prove seat scope. Recording `["side"]` contact and asserting it settles (and giving the fake `root: Some("root")` with `Composer` transport) would kill a regression that drops the `transport == Queue` gate before `take_queue_root` in `src/scheduler/mod.rs:1087`. Today the production queue runtime also returns no root for a fallback (pinned in `src/notification/queue.rs` test `programmatic_wake_flipped_off_before_arm_refuses_no_effect_and_falls_back_native`), so the defect would need two regressions to surface.
- nit: `programmatic_wake_flip_lifts_unreceived_and_the_next_attempt_queues` is named for a queue attempt but its notifier is the native fake ("typed"); it proves the lift only. Task ht-z41e.4's acceptance ("unreceived native -> flip on -> queue attempt follows") relies on the selection tests for the second half, which the comment acknowledges.
- nit: `RESERVED_AT_UTC` documents attempt-1's `reserved_at_utc`, but `LadderStore::reserve_wake` still computes `n * 100` from a literal; the constant and the fake can drift silently.
- nit: `root_session_contact_settles_a_queued_wake_whatever_binding_made_it` claims re-registration coverage, but the contact book has no binding dimension; the property holds structurally (keyed by native session UUID) and depends on `contact_claim` passing `claim.native_session`, which is outside the cluster.

## Untested scope

- A `Queue`-transport attempt whose root is unknown (`take_queue_root` returns `None`, e.g. after the `ROOTS_MAX` clear) is not awaited and never marked `unreceived`; the deleted `queue_transport_never_awaits_generic_contact_or_marks_unreceived` covered queue non-awaiting and has no replacement for this branch.
- A queued `OutcomeUnknown` attempt with a known root (awaiting entry with `armed: false`, dropped on the next check) is no longer exercised; the old test looped over `Submitted` and `OutcomeUnknown`, the new receipt tests use `Submitted` only.
- Seat scope of a queue-selected native fallback is not distinguished from root scope by any cluster test (see the first nit).
- Root re-registration under a new binding id is not modelled; coverage is structural only.

## Deferred OK

- AGENTS.md "closed-loop" entry lists the lift conditions as seat heard from, newer item, or targets changed; the `programmatic_wake` flip lift is documented only in the per-seat queue entry ("a write re-judges the seat's wake and lifts its `unreceived` suppression"), not cross-referenced in the closed-loop list (minor doc consistency, outside this cluster).
- `DeliveryPullState::session_contacts` grows one entry per (seat, native session) for the daemon's lifetime with no pruning (outside this cluster; small, but every resumed or `/side` session adds a key).

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- Queue-transport attempt with unknown root (take_queue_root None) is not awaited: no test after the old queue non-awaiting test was deleted.
- Queued OutcomeUnknown attempt with a known root (armed false, dropped) is no longer exercised; new receipt tests use Submitted only.
- Fallback-to-native seat scope is not distinguished from root scope: the fallback test records no contact and passes root None, so it cannot kill removal of the transport == Queue gate in src/scheduler/mod.rs:1087.
- Flip test proves only the unreceived lift; the queue attempt that follows (ht-z41e.4 acceptance) is left to selection tests outside this cluster (seam).
- Root re-registration under a new binding id is covered structurally only (contact book keyed by native session); the contact_claim native_session wiring is the seam reviewer's.

## Deferred OK
- AGENTS.md closed-loop entry does not cross-reference the programmatic_wake flip lift, which only the per-seat queue entry documents (minor doc consistency, outside cluster).
- DeliveryPullState::session_contacts keeps one entry per (seat, native session) for the daemon lifetime with no pruning (outside cluster).
- RESERVED_AT_UTC constant in tests/scheduler/wake_receipt.rs is not used by LadderStore::reserve_wake, which still computes n * 100 from a literal.
