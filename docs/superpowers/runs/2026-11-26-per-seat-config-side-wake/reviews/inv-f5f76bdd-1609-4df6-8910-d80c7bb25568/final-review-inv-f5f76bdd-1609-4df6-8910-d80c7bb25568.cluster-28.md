# Final review: cluster 28 (tests-harness), epic ht-z41e

Source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head c3e9ab0becd137adb68f23a1c9e1a87669d03168 (both verified with `git rev-parse --verify`). The caller checkout was at c3e9ab0be with a clean status for the whole review. No worktree was created and nothing was built or run. Reading the code was enough, and the cluster's test evidence on the same file contents is recorded in task-11-report.md.

## Scope

Cluster files (slices 000163-000167):

- `tests/harness/agy_delivery.rs`, `tests/harness/agy_wake.rs`, `tests/harness/role_witness.rs`, `tests/harness/bridge.rs`: mechanical `auxiliary_session: false` added to `CallerClaim`, `LifecycleEvent` and `EventMetadata` literals. These literals are needed for the new struct field and change no behaviour.
- `tests/harness/codex.rs`: one new test, `null_transcript_path_is_auxiliary_session_evidence_on_every_hook_event` (ht-z41e.21).

`git log 136484396..c3e9ab0be` lists no later change to these files, to `src/harness/codex.rs`, to `src/harness/adapter.rs` or to `tests/fixtures/codex-0.163.0-side/`. So the task-11 run (`cargo test --lib ... null_transcript ...`: 643 passed, 0 failed) covers the reviewed content. Clippy across all targets was also recorded there.

## Spec trace

Spec: `docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-26-per-seat-config-side-wake-design.md`, §Section 2 "Root marker" (amended 2026-11-27) and §Testing "Root marker".

- The spec's rule is that a null `transcript_path` on a top-level hook event is auxiliary evidence, while an absent key, a path or a child event is not. Production code: `src/harness/codex.rs` `auxiliary_session()`, called from `parse_shape`. It matches SessionStart, UserPromptSubmit, PreToolUse and Stop, requires `agent_id` to be absent or null, and requires `transcript_path` to be present and null.
- The new test covers each of these cases:
  - Null is evidence: side-session fixtures for SessionStart fork, UserPromptSubmit, PreToolUse and Stop, plus `pre-tool-use-root-null` (a root that emits null loses only queue routing, as the r3 amendment accepts).
  - A path is not evidence: root SessionStart, UserPromptSubmit and PreToolUse; a non-null fork SessionStart (the "non-/side fork" case); a non-null side PreToolUse (no evidence on that event alone, while stickiness lives in the store).
  - An absent key is not evidence.
  - A child event with an `agent_id` and a null path is not evidence.
- The test also checks that the bit survives the `DecodedEvent::from_native(..).context_event()` round trip, which is the hop into the check-in context.
- The fixtures under `tests/fixtures/codex-0.163.0-side/` match the shapes the spec attributes to spike S1.
- The spec lists PostToolUse among the classified events. Codex registers no PostToolUse hook and `parse_shape` rejects one, so there is no payload to classify. The divergence is disclosed in task-11-report.md ("Assumptions / Concerns").

## Requirement baseline

- B8 (wake routing to the top-level session when `/side` sessions exist) is the only baseline row this cluster touches. The cluster delivers the decode-level evidence: the adapter reads the marker from a real Codex payload. The rest of the production path runs from the hook bridge (`src/harness/bridge.rs` `prepare_event_as` and `prepare_qualified_turn` set `claim.auxiliary_session`), through the daemon store (`bindings::mark_auxiliary`), to queue selection (`wake_process_root`). That path leaves this cluster and is for the seam reviewer to trace.
- Live qualification (ht-z41e.10 and ht-z41e.31, still open) is also a whole-epic or seam matter, not this cluster's.
- B1-B7, B9 and B10 are outside this cluster.

## Test-design notes

- Many existing Codex helpers in `tests/harness/codex.rs` (`session_start`, `stop`, `pre_tool`, `session_end`) and the codex-0.158.0 fixtures use `transcript_path: null` for root events. With this change, every synthetic root binding enrolled from them reads as auxiliary. Only queue root selection reads that bit, and the spec accepts it for `codex exec --ephemeral` roots. Any future queue-routing test built on these helpers will therefore select native, which the author should expect.
- The store and selection tests copy `event.auxiliary_session` onto the claim by hand (`tests/fixtures/wake_intent.rs` `enroll_payload`, `tests/store/cooperative_checkin.rs` around line 3640). They do not go through the bridge. So no test in the bridge suite proves that the bridge forwards the bit, or that `decode_request` and the echoed-claim compare in `src/cli/mod.rs` ignore it. If either reset were missing, every `/side` check-in through a real hook would be refused, and no test in this cluster would catch it.

## Findings

- none

## Concerns

- No test in `tests/harness/bridge.rs` drives a null-`transcript_path` Codex payload through `prepare_event_as` / `prepare_qualified_turn` and then through `decode_request`. Production propagation (bridge.rs:430, :513) and the frozen-claim reset (bridge.rs:573) therefore depend on live qualification alone. The selection and store tests set the claim bit by hand instead.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- Bridge propagation of the auxiliary marker is not exercised by any test: `prepare_event_as` and `prepare_qualified_turn` assign `claim.auxiliary_session` (src/harness/bridge.rs:430, :513), and the `decode_request` frozen-claim reset (:573) and the echoed-claim reset (src/cli/mod.rs:3642) are untested with the bit set; test helpers copy the bit onto the claim by hand.
- Exclusion by event name is not asserted: a null `transcript_path` on SessionEnd, SubagentStart or SubagentStop must carry no evidence, and the new test covers only the four included event names plus the child case.

## Deferred OK
- The spec lists PostToolUse among the classified events, but Codex registers no PostToolUse hook and `parse_shape` rejects one; the decoder and test omit it, which task-11-report.md discloses.
- Generic Codex test helpers default to a null `transcript_path`, so synthetic root bindings read as auxiliary; this is harmless today because only queue root selection reads the bit, but future queue-routing tests built on them will select native.
