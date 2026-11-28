# Final review, cluster 11 (scripts-root), epic ht-z41e

Source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head c3e9ab0becd137adb68f23a1c9e1a87669d03168 (both verified with `git rev-parse --verify`). Caller checkout observed at HEAD c3e9ab0be with a clean status before and during the review; nothing in it was changed.

Cluster files: scripts/release-qualify, scripts/wake-e2e.py, scripts/wake_e2e_queue.py, scripts/wake_e2e_queue_selftest.py, scripts/wake_e2e_selftest.py (slices 000064, 000066-000069).

## What the cluster delivers against the spec

- §Removing the node setting, release-tooling paragraph (B1, B10 "Remove, no migration"): `scripts/release-qualify` drops the doctor/settings probe (`_doctor_report`, `operator_programmatic_wake`, the `queue-mode` step) and selects queue proof only from `--queue-proof`. The flag's help text says so. The wake driver no longer writes `programmatic_wake` into the owned node `settings.json`. `Recipient.enable_queue_wake()` runs `herdr-threads human --state-dir <owned> --host-endpoint <owned> --json config seat set --seat <seat> programmatic_wake on` for each Codex recipient under `--queue-proof`, after the threshold report and before the queue bootstrap, so before any send. I traced the argv through `Run.ht`: `human` is argv[1], which is the only thing that decides the human tier (`split_actor_os_argv`, AGENTS.md line 122 at head). The global options come after it, the same shape the driver already uses for `human seat resolve --operator`. The CLI accepts `--seat` before KEY VALUE (tests/cli/config_seat.rs:650). AGENTS.md:147 and RELEASE.md:38-51 at head describe the explicit-only selection.
- §Qualification `side-queue/group` (B8): an opt-in case, Codex at T=0 and hooked only, refused without `--queue-proof`. It leaves one `/side` chat displayed after it has run an `inbox` turn, then sends a p0. It passes only on four things: `side_queue_proof` (an Accepted intent whose scope equals the root's pre-case Ended baseline, from `process_root_scope`, with an Add witness), a non-empty `side_sessions` list that excludes the root, delivery in the root turn (`codex_turn_delivery` on the root session), and `side_pane_verdict` reporting no new wake marker and no change to the prompt or answer rows. `queue_snapshot` reads `wake_intents.side_sessions` when the column exists and projects None (unknown) when it does not, never an empty list.
- §Qualification `side-native-defer/group` (B9) with the 2026-11-27 causality amendment: an opt-in case, refused under `--queue-proof`. `bindings_idle()` runs before the send. The case polls `wake_state()` for a refused attempt completed after `sent_ms` with `retry_step` equal to its pre-send value, and treats a missing refusal as a setup failure, so no fixed sleep stands in for it. Then it closes the side chat and requires a wake and delivery. I checked the refusal semantics in the daemon. Every pre-send dispatcher exit is `WakeOutcome::Refused(cause)` (src/notification/dispatch.rs:182-189, 226-236). Completion restores the pre-reservation ladder row (src/store/wake.rs:1158-1176) and writes `last_outcome` as `unsafe|unavailable|timed_out`. That matches `REFUSED_OUTCOMES`, which resolves the deferred "unverified assumption" about `timed_out`. Reservation never resets `retry_step` (src/store/wake.rs:918), so "unchanged step" is a sound deferral signal. Saturation at step 3 is covered by the `last_outcome` check.
- `side-nudge`/`side-draft` per mode: under `--queue-proof` a hooked recipient's root session is set on the evidence, and the expectation is Accepted on the root (spike S2 "drop"). The side-draft verdict in queue mode needs the intact draft and delivery. Without the flag everything is native, and co-plain stays native in both modes.

## Verification run

- `git archive` of the head's scripts/, src/, tests/fixtures/, integrations/ and tests/herdr-releases.tsv into a review-owned scratch directory (no caller files touched).
- `python3 scripts/wake-e2e.py --self-test` ran 318 tests: OK. This includes the new SideQueueCases, the per-seat enablement tests and the never-writes-node-settings test for both modes.
- `python3 -m unittest` of scripts/tests/test_release_qualify.py ran 7 tests: OK.
- A first run that lacked src/ and fixtures failed three source-reading tests (missing files). That was my partial archive's fault, not a defect. The full-tree rerun is the result above.

## Findings

- scripts/wake_e2e_queue.py `side_queue_proof` (about line 317) accepts any non-empty `side_sessions` list that excludes the root. It never checks that the side session the driver actually displayed (the non-root bindings `process_root_scope` already computes as `side_sessions`) is in the intent's list. An intent that records a wrong or stale side UUID still passes. This is a judging weakness, not a product defect, because the daemon-side record is tested in Rust.
- scripts/release-qualify:13 `import signal` is now unused: its only use was in the removed `_doctor_report`.
- scripts/wake-e2e.py: a queue-proof run that puts an ordinary case (for example `send-p0/group`) after a side case on the same Codex recipient judges the ordinary case with the default single-binding `eligible_scope` baseline. The side bindings are still connected after Ctrl+C and the daemon now queue-targets the root, so that case fails under required proof. The planned qualification invocations order side cases last or run them alone, so this only matters for an ad-hoc `--cases` mix.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- No live run at the reviewed head: side-queue/group, side-native-defer/group and the queue-mode side-nudge/side-draft expectations have never run against a real Codex. The per-seat `ht human config seat set --seat ... programmatic_wake on` enablement has never run against a real daemon either. Open beads ht-z41e.10 and ht-z41e.31 track this.
- The driver's exact argv (`human --state-dir X --host-endpoint Y --json config seat set --seat S programmatic_wake on`) is verified only against a mocked `Run.ht` in the self-test. No CLI parse test covers global options between `human` and `config seat set`. Existing human commands use the same shape, so the risk is low.
- The side-native-defer post-close wake depends on the refusal-backoff retry landing within `SPACING_S + wake_window()` (35 s + 6 s). That timing is unmeasured live.
- side_view_changed may fail if Codex redraws the displayed side view for the background root turn. This is unmeasured live (already recorded as a deferred minor for ht-z41e.8).
- B5 (an agent told in prose sets a key) has no driver case at head. The opt-in config-prose/group, flag-flip/group, binding-bit and settled_by_root records belong to open bead ht-z41e.30. B5's production path (CLI plus skill) is in other clusters and belongs to the seam reviewer.

## Deferred OK
- scripts/wake_e2e_queue.py `side_queue_proof` could cross-check the intent's `side_sessions` against the displayed side binding's session from `process_root_scope`.
- scripts/release-qualify:13 has an unused `import signal`.
- scripts/wake-e2e.py: an ordinary queue-proof case after a side case on the same recipient gets the single-binding baseline. The fix is to set `root_session` for any hooked Codex recipient whose bootstrap root is known, or to order side cases last.
- scripts/wake-e2e.py `bindings_idle` docstring says `settled()` stands in for bindings without a turn row, but the code calls only `wait_idle()` (already recorded for ht-z41e.25).
- scripts/wake_e2e_selftest.py `test_enablement_runs_after_threshold_report_and_before_queue_bootstrap` compares source-text indices through `inspect.getsource`, which breaks under refactors (already recorded for ht-z41e.5).
- scripts/wake-e2e.py TmuxRecipient never calls `enable_queue_wake`. It is Claude-only, so it has no effect today (already recorded for ht-z41e.5).
