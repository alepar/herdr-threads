# Final review: cluster 29 (tests-hook-entrypoint)

Source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head c3e9ab0becd137adb68f23a1c9e1a87669d03168 (both verified with `git rev-parse --verify`). Review source bound to invocation inv-f5f76bdd-1609-4df6-8910-d80c7bb25568 (bind-review-source exit 0). Inputs: `tests/hook_entrypoint/agy.rs`, slice `slices/000168.diff`; context from `digest.md` and the epic spec `docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-26-per-seat-config-side-wake-design.md`.

## What the cluster changes

One line in `tests/hook_entrypoint/agy.rs` (module `activation`, `invite()` fixture, line 1137 at head): the `CallerClaim` struct literal that the test builds from the canonical `occupant_bindings` row of the Claude origin session gains `auxiliary_session: false`. This is the compile-required follow-on to `CallerClaim` gaining the `auxiliary_session: bool` field in `src/protocol/authority.rs` (ht-z41e.21).

## Analysis

- Correctness of the value: the row the fixture queries is a Claude top-level binding (`harness='claude'`, `role='top_level'`, `agent_id=''`). The spec's root marker (design §Selection, "Root marker") is Codex `transcript_path` nullness only; `src/harness/claude.rs:467` and `src/harness/agy/decode.rs:136` both emit `auxiliary_session: false`, so the canonical row's column is the migration default 0 and `false` matches what production would claim for this caller.
- Wire neutrality: the field is `#[serde(default, skip_serializing_if = "std::ops::Not::not")]`, so a `false` claim serializes exactly as before the change; the test's daemon round trip is unaffected, including against `deny_unknown_fields` on `CallerClaim`.
- The literal is the only `CallerClaim {` construction in the file (checked at head), so no other site in this file was missed.
- Fidelity choice: the fixture hard-codes `false` rather than reading `occupant_bindings.auxiliary_session` alongside the other canonical columns. For this Claude binding the two are equal by construction; reading the column would only matter if the fixture were reused for a Codex side binding, which it is not.

## Requirement baseline tracing

- The spec has a `## Requirement baseline` (B1-B10, S1). This cluster is a test fixture compile fix and delivers no baseline row itself, nor does it claim to. B8 (wake routed to the top-level session past a `/side` session) depends on the `auxiliary_session` evidence path through `src/harness/codex.rs`, `src/harness/bridge.rs`, `src/cli/hook.rs`, the binding migration `0022_binding_auxiliary_session.sql` and queue root selection; those paths leave this cluster and are the seam reviewer's and their own clusters' to trace. B1-B7, B9, B10 do not touch this file.

## Findings

- none

## Concerns

- none

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- none

## Deferred OK
- none
