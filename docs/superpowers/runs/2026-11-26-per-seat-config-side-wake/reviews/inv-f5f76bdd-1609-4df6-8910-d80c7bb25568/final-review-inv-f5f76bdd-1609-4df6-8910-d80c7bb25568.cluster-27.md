# Final review, cluster 27 (tests-fixtures), epic ht-z41e

Source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head c3e9ab0becd137adb68f23a1c9e1a87669d03168 (both verified with `git rev-parse --verify`). Review-source binding recorded before this write (dispatch-text bind-review-source, exit 0). The caller checkout was clean on branch super-auto/per-seat-config-side-wake when the review began. Nothing was built or run in it.

Files reviewed (slices 000152-000162): nine new hook payloads under `tests/fixtures/codex-0.163.0-side/`, `tests/fixtures/herdr-matrix/codex.js` (the stand-in Codex) and `tests/fixtures/wake_intent.rs` (the shared wake fixture).

## Scope and baseline rows touched

The spec has a `## Requirement baseline` section. Two of its rows reach this cluster:

- B8 (queue wake reaches the top-level session in the background while `/side` sessions exist). The Codex payload fixtures and `enroll_payload` feed the tests for decode (`tests/harness/codex.rs` `null_transcript_path_is_auxiliary_session_evidence_on_every_hook_event`), binding persistence (`tests/store/cooperative_checkin.rs`), selection (`tests/store/wake_queue_cost.rs`) and runtime (`tests/notification/queue_runtime.rs`).
- B9 (a native wake never types into a side chat). The stand-in Codex's new Ctrl-C side-close drives the inverted `/side` expectation in `tests/integration/herdr_release_smoke.rs` `herdr_release_wake`.

The fixtures do not deliver behaviour themselves. Delivery is traced from production by the clusters that own `src/harness/codex.rs`, `src/harness/bridge.rs`, `src/store/bindings.rs`, `src/store/wake_intents/selection.rs` and `src/harness/codex/composer.rs`. That includes the hook to bridge to check-in hop: does a `/side` session's first UserPromptSubmit or PreToolUse produce a check-in that carries `auxiliary_session` in a live process? That is the seam reviewer's to trace. Within this cluster, nothing stands in for a production path that does not exist.

## Fidelity of the payload fixtures

- I compared the field sets with the live captures in `docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/evidence/run-3/side-results.json`. They match exactly per event:
  - SessionStart: cwd, hook_event_name, model, permission_mode, session_id, source, transcript_path.
  - UserPromptSubmit: the shared fields plus prompt and turn_id.
  - PreToolUse: the shared fields plus tool_input, tool_name, tool_use_id and turn_id.
  - Stop: the shared fields plus last_assistant_message, stop_hook_active and turn_id.
- The nullness pattern matches spike S1: the side fixtures (`*-side.json`, `stop-side.json`) carry `transcript_path: null`, and the root ones (`session-start-root`, `user-prompt-submit-root`, `pre-tool-use-root`) carry a path. `session-start-root.json` has `source: startup`, as observed.
- Three fixtures are counterfactual inputs for fail-closed tests, not observed payloads:
  - `pre-tool-use-root-null.json`: a root that emits null. It is expected to make the process rootless, so the wake stays native.
  - `pre-tool-use-side-nonnull.json`: a side's later event with a path. The auxiliary mark stays set.
  - `session-start-fork-nonnull.json`: a non-`/side` fork, which sets only the concurrent bit and stays native.
  Their names say what they model, and the tests use them for exactly those cases. That is legitimate. Two labels overclaim, though:
  - The doc comment in `tests/store/wake_queue_cost.rs` calls the whole directory "Real Codex hook payloads".
  - The directory is named for 0.163.0, but the spike observed `codex-cli 0.172.0`.
  Both are labels only. AGENTS.md treats version facts as historical, so this gates nothing.

## The wake_intent fixture

- `fixture()` now delegates to `fixture_with(|_| {})`, so its callers see no change. `fixture_with` runs the `enrolled` hook after the root binding is inserted and before the reservation is taken. The doc comment gives the reason: a check-in advances `decision_seq` and would make an earlier reservation stale.
- The seed row `INSERT INTO seat_config(... 'programmatic_wake','on',0,'operator:local-user:1000')` matches migration 0020: STRICT, `writer_binding` nullable, and the operator provenance format that `src/store/seat_config.rs` writes. `set_programmatic_wake` deletes the row and then inserts it again, which is how an explicit set or an unset leaves the table (an unset leaves no row).
- `enroll_payload` decodes through the production `codex::parse_event_for_contract` with `CodexContract::registered()`, round-trips through `DecodedEvent::from_native(..).context_event()`, and checks in through the daemon's real `issue_cooperative_permit` and `register_available`.
  - Its mode mapping matches the production bridge (`src/harness/bridge.rs` `prepare_event_as`: Fork maps to `WireMode::Fork`, Current to `Current`, every other kind to `Lifecycle`).
  - It copies `auxiliary_session` from the decoded event onto the claim, as both bridge paths do.
  - It builds the claim by hand rather than through `ContextJournal::claim`. It therefore bypasses the client-side context-journal gating, such as the `LifecycleRequired` check for an unseen session (see Untested scope).
- `side_sessions: Vec::new()` and `auxiliary_session: false` are mechanical additions for the new struct fields.

## The stand-in Codex (`codex.js`)

- The file is Python despite its `.js` name, and it parses (checked with `ast.parse` on the head blob).
- Ctrl-C while the `/side` placeholder is displayed now restores the main placeholder, logs `side-close` and runs no Interrupt hook. Otherwise it behaves as before (Interrupt hook plus an `interrupt` log entry). The redraw happens through the existing `draw()` after the input loop. The release-smoke test waits for exactly that `side-close` log kind. The docstring was updated to match.
- The stand-in still models no forked session, so it exercises only the native defer half of B9 and none of the queue routing in B8. That is consistent with what the docstring says.

## Findings

- `tests/store/wake_queue_cost.rs:112` labels the `codex-0.163.0-side` fixtures "Real Codex hook payloads". Three of the nine new files are deliberate counterfactuals (`pre-tool-use-root-null`, `pre-tool-use-side-nonnull`, `session-start-fork-nonnull`), and the shapes were observed on 0.172.0, not 0.163.0. This is wording only, with no effect on behaviour.
- `tests/fixtures/wake_intent.rs` `enroll_payload` copies the bridge's mode mapping and claim construction by hand, rather than calling the bridge. It matches the bridge at this head, but nothing ties the two together.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- `enroll_payload` (tests/fixtures/wake_intent.rs) builds the claim and check-in mode by hand instead of through `src/harness/bridge.rs` and `ContextJournal`, so no fixture-driven test shows that a `/side` session's first UserPromptSubmit or PreToolUse gets through the client-side bridge gating (for example `LifecycleRequired` for an unseen session) with `auxiliary_session` set; this hop is for the seam reviewer.
- No test checks that `enroll_payload`'s mode mapping and claim fields still match the bridge's `prepare_event_as` and `prepare_qualified_turn`, so a later change to the bridge would leave the fixture-driven B8 tests green against a stale model.
- The new stand-in Ctrl-C side-close branch and the inverted `/side` expectation in `herdr_release_wake` are compiled but were not executed in this run (an ignored test that needs a private Herdr through scripts/herdr-release-smoke), as already recorded in Task 1's deferred minors.
- I did not rerun the fixture-consuming tests at head c3e9ab0be; the pass counts come from the Task 11 report (tree a99632425, before the ht-z41e.22 commits, which touched no fixture file).

## Deferred OK
- The doc comment in tests/store/wake_queue_cost.rs:112 and the `codex-0.163.0-side` directory name overstate where the payloads came from (three are synthetic counterfactuals, and the shapes were observed on codex 0.172.0); this is a wording fix only.
- The `#[allow(dead_code)]` attributes on the wake_intent fixture helpers are expected for a `#[path]` module shared by several test binaries.
