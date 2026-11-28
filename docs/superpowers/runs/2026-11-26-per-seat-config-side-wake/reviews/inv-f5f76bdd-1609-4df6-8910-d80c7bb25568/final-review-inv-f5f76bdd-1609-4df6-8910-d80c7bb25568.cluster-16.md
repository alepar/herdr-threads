# Final review, cluster 16 (src-host)

Source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head c3e9ab0becd137adb68f23a1c9e1a87669d03168 (both verified with `git rev-parse --verify`). Input: `slices/000094.diff`, the single file `src/host/native.rs`.

## Scope of the cluster

The cluster adds one unit test to `src/host/native.rs` and no production code:
`cooperative_wake_defers_on_a_side_composer_and_prompts_once_it_reads_empty` (head lines 5980-6034). It was added in commit 64d684da7 ("Codex native wake defers on a /side chat composer"). The production change it covers is the removal of `Ask a follow-up question` from `CODEX_PLACEHOLDERS` in `src/harness/codex/composer.rs`, which belongs to another cluster.

## Requirement trace (B9)

- B9 ("Stop native wake typing into a side chat. Remove the side prompt from the list of empty composers.") is the only baseline row this cluster touches. Production path at head: `src/notification/dispatch.rs:747` calls `HostPort::submit_prompt_if_owed`. On `NativeCli` (native.rs:1612) that calls `submit_prompt_mode` (native.rs:2258), which runs `idle_wake_gate` first. That gate does the `agent.get` recheck and the `agent.read --source detection` composer read through the bound harness's `ComposerPolicy`. Then comes the `still_owed` check, and only then `agent prompt`.
- A Codex side placeholder now reads as `Text`, not `Empty` (composer.rs head line 17: `CODEX_PLACEHOLDERS = &["Ask Codex to do anything"]`). The gate therefore refuses with `TargetUnsafe` before anything is typed. On this host-level side, the row is delivered from a real production entry point.
- The new test drives this real path through the fake Herdr socket: `cooperative_wake_submit` with real `registry::builtins()`, the Codex bound harness and the real `NativeComposer`.
- First half: the side screen gives `TargetUnsafe`. The `still_owed` closure panics if it is called, which proves the refusal comes before the owed check. The recorded methods are exactly `pane.get, agent.get, agent.read`: no `agent.prompt` or `pane.send_keys`, so nothing is typed.
- Second half: the same pane with the main placeholder gives `Submitted` with `agent.prompt {"target":"w4:p1","text":"wake marker"}`.
- The side screen is the captured `codex-q6-workers.read-detection.txt` with its last composer row rewritten. I checked the capture: `› Ask Codex to do anything` occurs exactly once (line 47), and it is the last `›` row that `codex_rows` (rposition) selects. So the replacement really changes the row the reader judges. If it silently failed to match, the first half would call `still_owed` and panic, so the test cannot pass vacuously.
- After a refusal, the wake is retried on the ladder until the side chat closes. That is the scheduler's job (`tests/scheduler/dispatch.rs` `side_composer_refusals_retry_past_the_ladder_length_until_a_wake_submits`), which sits outside this cluster. It is the seam reviewer's to trace.

## Other checks

- Ordering: the composer read runs inside `idle_wake_gate`, before `still_owed`. So a side screen whose wake is no longer owed also returns a `TargetUnsafe` refusal rather than `Ok(None)`. This matches the existing behaviour for any non-empty composer. The spec only asks that the wake refuse and retry, so this is not a defect.
- `#[cfg(any(target_os = "macos", target_os = "linux"))]` matches the helpers it uses (`recheck_exchange`, `cooperative_wake_submit`). No new model-visible text; no wire or harness-neutral impact. The test sits in a host module and names `codex` only as fixture data, like the neighbouring tests.
- Test evidence: task-1-report.md records `heavy tb cargo test --locked --all-features -- ... cooperative_wake_defers ...`, with 140 passed and 0 failed, plus clippy rc=0. I did not re-run it (read-only review; no build in the caller worktree).

## Findings

- nit: the second half asserts only that the last recorded method is `agent.prompt`. It does not check the full order (`pane.get, agent.get, agent.read, agent.prompt`) the way the first half does, so a stray extra host call before the prompt would go unnoticed. This does not affect correctness (src/host/native.rs:6032).
- nit: the side screen is made by editing the text of a main-session capture, not taken from a captured `/side` screen. The captured side placeholder is covered at the composer level in the other cluster (`captured_side_placeholder_is_not_empty_and_a_side_draft_is_text`), so the combined coverage is adequate.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- I did not re-run the new native test myself; it passed per task-1-report.md (`heavy tb cargo test ... cooperative_wake_defers`, 140 passed), with the log in task scratch only.
- The retry-until-side-closes half of B9 (ladder backoff after `TargetUnsafe`) is outside this cluster and is the seam reviewer's to trace via `tests/scheduler/dispatch.rs`.

## Deferred OK
- Tighten the second-half assertion to the full method sequence `pane.get, agent.get, agent.read, agent.prompt` (src/host/native.rs:6032).
