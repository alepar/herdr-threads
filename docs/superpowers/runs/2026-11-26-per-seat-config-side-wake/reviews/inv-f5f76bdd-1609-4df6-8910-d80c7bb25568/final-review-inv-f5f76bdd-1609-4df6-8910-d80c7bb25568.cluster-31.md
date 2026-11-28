# Final review, cluster 31 (tests-notification)

Source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head c3e9ab0becd137adb68f23a1c9e1a87669d03168 (both verified with `git rev-parse --verify`). Review source bound with `dispatch-text.mjs bind-review-source` (exit 0). Cluster input: `tests/notification/queue_runtime.rs`, slice `slices/000177.diff`. Read against the epic spec `docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-26-per-seat-config-side-wake-design.md` (Requirement baseline, Section 1 "Removing the node setting", Section 2 "Busy siblings", Testing "Queue"). No worktree was created and nothing was built or run; the caller checkout was only read.

## What the cluster changes

- The existing fence-ordering test `queue_runtime_checks_fences_before_capture_and_never_helpers_a_known_busy_binding` no longer configures a process-wide flag via the removed `QueueWakeRuntime::with_programmatic_wake`. It writes or deletes the seat's `programmatic_wake` row (`set_programmatic_wake`, the same row `ht human config seat set` writes) after the reservation is taken and before `attempt`. The fixture seeds `programmatic_wake on`, and the helper deletes it first, so `None` and `Some(false)` both mean "no row / default off".
- New test `a_busy_side_binding_does_not_refuse_the_root_wake_and_a_busy_root_does` (spec S2, "busy-sibling rule: drop"). It enrolls a `/side` session through the real hook path (`enroll_payload` with the captured Codex `pre-tool-use-side.json` payload, null `transcript_path`), with the same pid as the root and before the reservation. It then records root and side turn states in the `TurnStateBook` and runs `attempt`.

## Trace against the spec and production code

- Per-seat flag read per attempt (Testing, "Queue": "per-seat flag read per attempt (on/off without restart)"). At head, `QueueWakeRuntime::attempt` (`src/notification/queue.rs`) passes `use_queue` to `queue_wake_selection`, and gates queue use on `selection.programmatic_enabled`, which comes from the store's selection transaction. The rewritten test flips the row after `fixture()` reserved the wake. So the `Some(true)` cases (calls == 1 when unblocked and idle) and the off cases (Native, calls == 0) show that the attempt reads the seat row at attempt time, not at reservation or construction time. That matches Section 1. The node key being ignored is not this cluster's to show.
- Busy-sibling drop (Section 2 "Busy siblings": "a `Busy` side binding does not refuse the root; only the root's own idle state gates the queued wake"). At head, `attempt` builds the `TurnKey` from `selection.binding.scope.native_session` (the selected root) and refuses only on that key's `Busy`. The new test's two cases discriminate in both directions:
  - (root idle, side busy) expects Native with one `process` call, so capture was reached. A mutant that restores an all-bindings busy rule, or that selects the side as target, would return Refused.
  - (root busy, side idle) expects Refused with zero calls. A mutant that ignores the root's state, or that keys the check on the side, would reach capture.
- Producer of the side binding. The side is enrolled through `enroll_payload`, which decodes the real fixture payload and checks in through `issue_cooperative_permit` and `register_available` (the daemon's cooperative path). It never writes a raw SQL flag, as the amended Testing section requires. `tests/store/wake_queue_cost.rs::queue_targets_the_root_with_a_side_first_seen_by_user_prompt_submit_or_pre_tool_use` shows that this same enrollment (same payload, pid 10, inside `fixture_with` before the reservation) yields a binding with `(concurrent_session, auxiliary_session) = (false, true)`, and that selection returns the root with the side in `side_sessions`. The new test does not assert this itself (see below).
- Baseline row B8 (side routing). This cluster only covers the queue-runtime gate of B8: a busy side never blocks the root's queued wake. Tracing B8 from the production hook entry point through selection, receipt scope and the live `side-queue/group` e2e leaves this cluster, so it is the seam reviewer's. No other baseline row (B1-B7, B9, B10) is exercised by this file.
- Rule 4 of the delivery contract (a busy session is never nudged) still holds for the root. The busy-root case asserts Refused before capture. Native composer admission keeps its all-bindings-idle rule, which is outside this file.

## Test hygiene

- Both tests create their queue-runtime directory through `test_support::scratch::temp_path`, which is removed at process exit even when the trailing `remove_dir_all` is skipped by a panic. No processes are spawned. The module stays inside `tests/combined.rs` (`#[path = "notification/queue_runtime.rs"]`) and keeps no process-wide state, which matches AGENTS.md.
- The new helpers it relies on (`fixture_with`, `enroll_payload`, `set_programmatic_wake` in `tests/fixtures/wake_intent.rs`) carry `#[allow(dead_code)]` because the fixture is `#[path]`-included in several modules. That matches the existing fixture style.

## Findings

- (nit) `a_busy_side_binding_does_not_refuse_the_root_wake_and_a_busy_root_does` never asserts that the side binding was actually enrolled, either as an `occupant_bindings` row with `auxiliary_session=1` or as a non-empty `side_sessions`. If `enroll_payload` ever stopped creating the binding (for example a changed check-in mode for a first-seen PreToolUse), the lone-root path would give the same Native/Refused results, and the test would pass without exercising a side. Today the selection tests in `tests/store/wake_queue_cost.rs` cover this, so this is a robustness gap, not a defect. A one-line `SELECT auxiliary_session FROM occupant_bindings WHERE native_session=SIDE` check would close it.
- (nit) In `queue_runtime_checks_fences_before_capture_and_never_helpers_a_known_busy_binding`, `(None, false, false, false)` and `(Some(false), false, false, false)` are now the same case. `set_programmatic_wake(.., false)` deletes the row for both, and the runtime-default distinction that `None` used to express no longer exists. This is harmless redundancy. The `Option` could become a `bool`, or `None` could be made to mean "fixture default (on)".

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- Neither test was executed in this review: the caller's prebuilt `combined` binary predates the head commits, a fresh build was not started, and the full-suite sweep is recorded as deferred to the caller (`progress.md` Recovery `ht-z41e:sweep:...`). Their pass status at c3e9ab0 rests on the caller's sweep.
- The queue-runtime busy-sibling test stops at physical capture (MissingProcess makes `observe` fail, so the attempt falls back to native). It does not show that a root queued while a side is busy is actually submitted and accepted, or that the intent's `side_sessions` carries the side. That is covered only by the live `side-queue/group` e2e and spike S2, outside this cluster.
- Turn state Unknown for the root while the side is enrolled (production returns Native), and both sessions busy, are not covered by the new test. Both follow from the same single-key match, so the risk is low.

## Deferred OK
- Add an explicit side-enrollment assertion (auxiliary_session=1 for the side session) to the busy-side queue-runtime test so it cannot pass vacuously if enrollment regresses.
- Collapse the duplicate `None`/`Some(false)` cases in the fence-ordering test now that the runtime flag is gone.
