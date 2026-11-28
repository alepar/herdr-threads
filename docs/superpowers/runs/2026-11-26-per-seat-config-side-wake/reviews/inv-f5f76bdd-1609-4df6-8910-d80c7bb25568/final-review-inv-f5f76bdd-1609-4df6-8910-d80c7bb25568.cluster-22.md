# Final review: cluster 22 (src-service), epic ht-z41e

Source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head c3e9ab0becd137adb68f23a1c9e1a87669d03168 (both verified with `git rev-parse --verify`). Source binding recorded through `dispatch-text.mjs bind-review-source` (exit 0). Caller checkout observed at HEAD c3e9ab0be with a clean `git status --short`, before and during the review.

Files: src/service/config.rs, src/service/delivery_pull.rs, src/service/dispatch.rs, src/service/kicks.rs (slices 000108-000111). Spec: docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-26-per-seat-config-side-wake-design.md at head, including its Requirement baseline (B1-B10, S1).

## What the cluster changes

- config.rs: `ServiceConfig` loses its `programmatic_wake` field, getter, builder and the `from_settings` wiring. The node key is no longer read into service config. Whether it is still parsed (`deny_unknown_fields`) and reported by doctor is decided in `src/daemon/settings.rs` and `src/cli/doctor.rs`, which belong to other clusters.
- delivery_pull.rs: `DeliveryPullState` gains `session_contacts`, keyed by (seat, native session UUID). `record_contact` now takes the session and updates both maps monotonically (`max`), and `HookPullOutlook::contacted_since_session` answers from the session map. Two rebuilt `CallerClaim`s set `auxiliary_session: false`.
- dispatch.rs: `seat_config_write` routes `SetSeatConfig`/`UnsetSeatConfig` by tier. Agent tier: `PermitMutation::try_from` (accepted only for Agent tier with a claim) and then `cooperative_mutation(.., false)`, the same path as `SetNudgeThreshold`. Human tier: `OperatorActor::from_peer` against `operator_owner_uid` (the same guard as `OperatorRebind`/`OperatorRetire`), then `store.seat_config_operator`. `GetSeatConfig` joins the plain `store.query` read arm. `record_contact` receives `claim.native_session`. The new service test module is registered.
- kicks.rs: `seat_config` is listed in `KNOWN_UNMAPPED`.

## Requirement trace within this cluster

- B1 (seat level replaces node level): the node flag no longer reaches the service (config.rs). The per-seat read in queue selection and the witness are in store/notification clusters, so the seam reviewer traces those.
- B3/B6/B7 (read/write CLI; agent writes for low-risk keys, human path for gated keys; nudge threshold writable without the human path): the daemon entry point is `LocalService::handle_with_output`. It routes the agent tier to `cooperative_mutation` (elected-owner uid, instance, TopLevel role, then store `set_agent`, which refuses other seats and human-tier keys) and the human tier to the operator actor. Reads go through `store.query` and need no claim, so subagents can read. The early `accountable_claim().is_subagent()` refusal covers both tiers when a claim is attached. tests/service/seat_config.rs exercises each refusal (human-tier key, other seat, subagent, unknown key, `-5`, no claim, non-owner peer, read-only binding) plus the human tier on a seat with no binding, A5 threshold override with its wake re-judgement, unset, a non-owner operator, a missing seat and a remote seat. The CLI producers are other clusters.
- B8/R5 (root-only arrival of a queued wake): every claim-bearing call records per (seat, session) before routing. `contacted_since_session` answers only for the exact native UUID, so a `/side` call never reads as root contact, and a root re-registered under a new binding but the same UUID still counts. The consumer, `src/scheduler/mod.rs:1019`, chooses the session check only when `awaiting.root_session` is set. That choice belongs to the scheduler cluster.
- kicks: a column-backed write (`nudge_threshold`) re-judges through `wake::note_new_attention`, which writes mapped tables and kicks on its own. A `programmatic_wake` flip needs no kick, because the spec defines it as read per wake attempt, and a pending ladder retry or AwaitingReceipt is re-evaluated against the witness's `programmatic_wake` (`awaiting.programmatic_wake == programmatic_wake`).

## Risks checked and cleared

- Human-tier authority does not depend on `cooperative_runtime` and matches the existing operator commands. Agent-tier authority comes from `cooperative_mutation` and the store's own-seat check (A2), not from the claim's seat hint.
- `auxiliary_session: false` on the rebuilt DeliveryConfirm and AckModDelivered claims cannot clear the sticky bit. `src/store/bindings.rs:1483` writes only when the evidence is true, and the evidence rides only on the bridge's check-in claim.
- The root's native session in the intent comes from the binding row's `native_session`, which is written from the same `claim.native_session` that `record_contact` keys on. The formats agree.
- A woken root's first call is its UserPromptSubmit hook (queue selection requires `cooperative_top_level`), and `contact_claims_cover_the_calls_a_woken_session_makes` covers that claim shape.

## Concerns

- `session_contacts` is never pruned, so it grows by one entry for every distinct (seat, native session) that makes a claim-bearing call over the daemon's lifetime. The seat-keyed `contacts` map is not pruned either, but it is bounded by the seat count. The growth is bounded by real sessions (claims carry a binding's session, not a per-process id) and is reset at idle exit or restart, so the practical cost is small. Ending a binding or retiring a seat could drop its entries.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- Tests were not rerun for this review (no build in a review-owned worktree); the verdict relies on reading tests/service/seat_config.rs, tests/service/delivery_pull.rs `contact_is_recorded_per_seat_and_per_native_session` and the dispatch operator tests at head.
- No service-level test drives `handle_with_output` end to end and then reads `contacted_since_session` for a `/side` claim versus a root claim; the session map is unit-tested directly and the dispatch wiring is checked only by reading the code.

## Deferred OK
- src/service/delivery_pull.rs `session_contacts` grows by one entry for every native session over the daemon's lifetime with no pruning; small in practice, and a follow-up could prune when a binding ends or a seat retires.
