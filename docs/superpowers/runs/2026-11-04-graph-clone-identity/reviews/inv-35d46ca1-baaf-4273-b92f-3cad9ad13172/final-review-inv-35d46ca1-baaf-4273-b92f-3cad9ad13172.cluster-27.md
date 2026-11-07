# Final review: cluster 27 (tests-harness), epic ht-ma6i

Source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head 1b058307490f29ece8bb4e3fe7a862f8356d992f, both verified with `git rev-parse --verify`. The caller checkout stayed at the head and clean throughout. Nothing was compiled or run: this cluster's change is static, and no build was started in the caller's worktree.

## Scope read

Two files, `tests/harness/agy_delivery.rs` and `tests/harness/agy_wake.rs`, slices 000148 and 000149. Four added lines in total, all `continuity_key: None,` in `CheckIn` struct literals, from one commit (8945c48ca, which describes them as part of a mechanical sweep of 182 literals). Both files are compiled as test modules of `src/harness/agy/delivery.rs` through `#[path]` (lines 1426 and 1429).

## Analysis of the change

- The field exists on `CheckIn` at the head (`src/protocol/commands.rs:844`) as `Option<ContinuityKey>` with `#[serde(default, skip_serializing_if = "Option::is_none")]`, as spec 4.2 states. The four literals name all six fields of the struct, so they match the struct definition by inspection.
- `None` preserves what these tests assert. They are agy delivery and wake tests that use a check-in as setup: lifecycle check-in, `/clear` rotation and an origin check-in. Spec 4.2 says a key with no row is ignored and a request without a key is ordinary behaviour, so keyless setup is the correct input and no assertion changed.
- No test body, assertion, fixture or helper in the two files changed.

## Requirement baseline trace

- B1 (continuity key): these files never present a key, so they neither deliver nor weaken it. Production flows through `src/cli/hook.rs` (`continuity_key`, `with_continuity_key`), outside this cluster; that path is the seam reviewer's to trace.
- B2 (pane close retires, undo reverses), B3 (agent exit keeps the seat) and B4 (Herdr unchanged): nothing in this cluster touches them.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- No test presents a continuity key through the agy adapter: `tests/harness/agy_delivery.rs` and `tests/harness/agy_wake.rs` pass only `continuity_key: None`, no file under `tests/harness` or `src/harness/agy` names a key, and `tests/clone_identity_sweep.rs` drives stand-in Codex agents, so agy's reattachment by key rests on the harness-independent code in `src/cli/hook.rs` and has no agy-specific evidence.
- This review did not compile or run the two test modules; the type match of the four literals is by inspection of the struct at the head, so a compile and test pass has to come from the integration run.

## Deferred OK
- none
