super-roast verdict: Should-fix (1 confirmed, fixed) — 1 nit
mode: PR        iteration: 1 of 3
profile (assumed): Internal developer tooling (local same-user daemon and CLI). The change touches client-local journal retention only; no wire, store schema, authority or provenance change. Blast radius: a wrongly discarded intent loses pending-ops visibility and exact-key retry for that one operation; a wrongly kept one leaks a small file.
inputs: bead/ht-2cbu@8cbcf4f7e vs main@3d7c26c53 — stated intent: ht-2cbu (discard an operator intent on a definitive first-submission refusal, keep it on an unknown outcome; user decision relayed by the coordinator)
coverage: unavailable — inline run, no coverage object · lanes walked by hand: correctness, classification parity, retry path, docs/policy, tests
independence: none (inline) · rung: inline

## Confirmed findings

- [Should-fix] src/cli/retry.rs `is_deterministic_rejection` — adding `ThreadNotOrphaned` to the shared list also changed `is_continuity_refusal` (which delegates to it) and the cooperative discard set, both pinned by tests. Found by the full suite rather than by this review: `continuity_refusal_table_is_pinned` (tests/cli/hook.rs:4513) and `cooperative_typed_rejection_discards_intent_but_transport_failure_keeps_it` (tests/harness/bridge.rs:1426) failed with `ThreadNotOrphaned`.
  verdict: confirmed (reproduced by running the two tests)
  fix-shape hint: leave the shared list unchanged and add `ThreadNotOrphaned` only in the operator branch of the discarding runner. Fixed in the follow-up commit.

## Checked and cleared

- Classification parity. Operator scope now uses `is_deterministic_rejection` (the list cooperative scope uses) plus `ThreadNotOrphaned`. Before this change, an Operator scope passed to the discarding runner would have fallen into the seat-resolution branch (any code but `UnknownOutcome`), which would have discarded on `StoreBusy`/`DeadlineExceeded`. `tests/cli/journal.rs::new_operator_intent_discards_only_deterministic_first_submission_rejection` pins this (on the old helper it fails at `StoreBusy kept=true`).
- `ThreadNotOrphaned` is deterministic. Its only emitter, `src/store/control/invitation.rs:705`, sits in the check closure of `execute_idempotent_transaction`, before the commit closure, so nothing commits under the key. It is now added only in the operator branch, so the cooperative and continuity classifications are unchanged.
- Other operator mutations. Retire and orphan invite are one `mutate_operator` transaction. Rebind, fresh seat and replace decide in `mutate_operator` after a host observation (`src/identity/repair.rs:285`). Their host-dependent refusals (`StaleHostObservation`, `TargetUnresolved`, `HostUnavailable`) are unlisted and keep the intent. A peer-UID mismatch answers `Unauthorized` before dispatch, which is deterministic for the same user and owner.
- Client call equivalence. `call_with_output` is `exchange_selected(..).and_then(identity)`, and `call_with_output_definitive` is `exchange_selected` itself (`src/client/local.rs:131-160`). The bytes, budget and output validation are identical; only the outer/inner split is now visible to the runner. Client-side output validation and transport failures stay outer errors, so they keep the intent.
- Retries unaffected. `human retry` still goes through `run_retry_api_to_writer`, which never discards. `operator_first_submission_refusal_discards_intent_only_when_definitive` retries a kept intent into a `ThreadNotOrphaned` refusal and asserts the intent stays.
- Existing fixture tests that refuse operator rebind/fresh seat (`seat_repair_by_name_freezes_ids`) snapshot intents at request time, so a discard after the answer does not break them (verified passing).
- Docs. The cli-design retention sentence and the store-design cross-reference are updated in the same commit. TRUST-POLICY needs no change: client-local intents carry no authority (A2) and no provenance; daemon-side operator decisions and receipts are untouched.

## Unverified nits (spot-checked)

- [Nit] src/cli/retry.rs `is_deterministic_rejection`: `Conflict`/`NotFound` from an operator rebind or replace can depend on the published host observation (durable state that a later reconciliation may change), so an identical retry could succeed later. This matches the cooperative criterion ("refused identically unless another operation changes durable state first"), and the user's decision is "the same way cooperative commands already do it". A rerun issues a fresh key. No change.

## Rejected (with reason)

None.

## Escalations (need human)

None.
