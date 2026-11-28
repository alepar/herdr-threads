# Final review (bounded): ht-z41e, super-auto/per-seat-config-side-wake

Captured source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head aab6af62de2b0c79dbb4985db291cdc93d2701ec (both verified with `git rev-parse --verify`). Read from captured Git only (`git diff`, `git show <head>:<path>`). The caller checkout was not switched, and no worktree was created.

## Scope of this pass

This is the bounded route. The review-inputs sidecar reads status unavailable, sourceAvailable true, wholeRangeReady false, omitted []. It keeps two gaps, both at run.md:124 inv-f5f76bdd-1609-4df6-8910-d80c7bb25568 (incomplete claimed collection; incomplete split bundle, missing cluster report or slices). The range has 610 changed paths and 106777 insertions, about 94k of them under docs/superpowers/runs. I read the whole changed-path inventory. I inspected every product hunk in src/ outside the test-only hunks of src/host/native.rs, plus the three migrations, the release tooling hunks that enable queue wake per seat, the policy and docs hunks, and the full design doc at head. bounded-coverage.json lists every inspected hunk. Test files, the remaining wake-e2e and selftest hunks, evidence logs and the run documents were not inspected.

## Requirement baseline trace

- B1 (replace the node setting with a per-seat one): delivered. `QueueWakeRuntime` loses its process flag (src/app.rs, src/notification/queue.rs). `queue_wake_selection` reads `seat_config::programmatic_wake_on` inside its transaction and fails closed (src/store/wake_intents/selection.rs). `arm` reads it again (src/store/wake_intents.rs). `InstanceSettings.programmatic_wake` is `Option<bool>`, still parsed but inert, and doctor flags it as obsolete.
- B2 and S1 (a generic key=value store as a closed typed registry): delivered. src/protocol/seat_config.rs `REGISTRY` validates exact values and refuses unknown keys by listing every key. `seat_config` table (migration 0020). A new Table key needs no SQL change.
- B3 (CLI read and write) and B4 (list with descriptions): delivered. src/cli/config_seat.rs has a static `list` (text and `--json`: key, values, default, tier, description) and `get`/`set`/`unset` gated on `seat.config_v1`. The daemon routes these in src/service/dispatch.rs `seat_config_write`.
- B5 (prose to an agent works): delivered. An agent-tier write runs on the caller's own seat. A human-tier key is refused before sending, naming the exact `ht human config seat set KEY VALUE` as typed (src/cli/commands.rs `refuse_agent_config_write`). SKILL.md states the commands once.
- B6 and B7 (tiers: every key readable without the human path, nudge_threshold writable without it): delivered. `GetSeatConfig` is a no-claim read route. The agent tier goes through `cooperative_mutation`, then `seat_config::set_agent` (subagent and read-only refusals). The human tier goes through `OperatorActor::from_peer` and `seat_config_operator`. `nudge_threshold` is column-backed and shares `write_threshold`/`write_column` with the aliases, which re-judge the wake and write the audit row.
- B8 (wakes go to the root in the background while /side is open; spikes): delivered in code. The Codex hook reports a null `transcript_path` as `auxiliary_session`. The binding bit is sticky (`mark_auxiliary`, migration 0022). `process_root` and `CodexAdapter::wake_process_root` choose the root. The queue has its own admission (`admit_queue_wake`). The dispatcher exempts a queue candidate from its early refusals. Receipt is root-scoped. The spikes are recorded (spike-side-identity.md, evidence run-2 and run-3). No live end-to-end run has happened at the final SHA: ht-z41e.10 is open. I list that under untested scope, not as non-delivery.
- B9 (native wake never types into a side chat): delivered. The side placeholder was removed from `CODEX_PLACEHOLDERS`, so a side composer reads as Text and `idle_wake_gate` refuses before sending. A queue candidate that ends on the native path still passes through that gate in `submit_prompt_mode`.
- B10 (remove the node setting with no migration; own seat, while a human may name any seat; the key sits over the existing column; live spikes): delivered as traced above.

## Cross-task integration observations

- The dispatcher's queue-candidate exemption (ActiveTurn/HumanInput) composes safely with every native fallback. `submit_prompt_if_owed` calls `submit_prompt_mode`, which re-runs `idle_wake_gate` (status, composer and dwell), so text is typed only into an empty main composer. `NativeCli::safe_wake_target` passes HumanInput, and ActiveTurn goes to `safe_poke_target` (ht-z41e.47).
- Transport bookkeeping is consistent. Native and NativeFallback reset the transport to Composer, so a queue-selected attempt that falls back stays seat-scoped. `roots` is filled only for Submitted and Unknown.
- `WakeAttentionWitness.programmatic_wake` is not part of `validate_reservation`, which uses only `valid_at` fields. A flip between reservation and validation therefore cannot invalidate every reservation. It does lift the `unreceived` suppression, as designed.
- New behavior beyond /side: every Submitted queue wake now enters `AwaitingReceipt` with root scope (src/scheduler/mod.rs, `transport == Composer || root_session.is_some()`). Before this change the queue transport was never closed-loop. This covers lone-root queue wakes too, consistent with the design's receipt amendment. The default queue-proof matrix has not been re-run at the head.

## Findings

- [minor] src/service/delivery_pull.rs `session_contacts`: one entry per (seat, native session) is kept for the daemon's lifetime and never pruned. Memory grows slowly with every session the daemon has seen. `contacts` is per seat and bounded by the seat count.
- [minor] src/notification/queue.rs `submit_captured`: when `roots` reaches `ROOTS_MAX` (256), it calls `clear()`, which drops the roots of attempts still in flight. Those attempts then get no `AwaitingReceipt`, so they fail open to no receipt tracking. Nothing is ever mis-targeted.
- [minor] src/host/native.rs `observe_target_with_composer`: `reset_empty` no longer resets on `composer_ui && ui != Idle`. It relies on `read_composer` resetting on a non-Empty read. For a harness without a composer policy, or a pane with no agent kind, the window is no longer reset at observation. `idle_wake_gate` still resets it on its own read, so I see no unsafe path, but nothing in the reviewed hunks explains the change.
- [minor] docs/evidence/ansi-composer/README.md:22 still says the plain read "classified the /side placeholder as typed Text, so every wake under /side was refused", followed by a parenthetical. It reads as contradictory (ledger Task 1 deferred minor). Wording only.

## Ledger triage

- Recurring minor, missing-evidence (×7 across ht-z41e.1, .2, .4, .7, .33, .37, .47): a process-class cluster, not a product defect. Reviewers did not re-run the test claims, so the claimed passes count as untested scope until the caller's full-suite sweep (ht-z41e.27) runs.
- Task 7 marker-deviation and mis-enrolled-side (concurrent_session as the root marker; a lone later-enrolled side being targeted): resolved at head. ht-z41e.21 replaced the marker with `auxiliary_session`, and `process_root` refuses a lone concurrent or auxiliary binding.
- Task 28 doc-drift (a side draft admitting a queue wake): resolved at head. TRUST-POLICY and AGENTS.md now say typed text refuses. Only the accepted limit remains, for a main draft that is exactly the side placeholder text.
- Task 6 tmux Codex seats stay native under --queue-proof: documented at head (AGENTS.md, "in a Herdr pane (not a tmux seat)").
- Task 11 retry-journal-downgrade (a journaled check-in carrying `auxiliary_session`, replayed against a downgraded daemon, is refused under deny_unknown_fields): reached only by a daemon downgrade between record and retry. Deferred OK.
- Task 24 root-text-proxy (`queue_text_ok` uses any target, not the root): correct only because of the selection invariant (root provenance `cooperative_top_level`). Deferred OK, but a future adapter whose root text differs would break it silently.
- Task 26 extra-host-round-trip (a lone working root reaches capture, prepare and journal before the arm-time refusal): per-retry cost that the amended design accepts. Deferred OK.
- Task 1 unexecuted-test (ignored `herdr_release_wake` side-close path) and Task 8 / Task 22 no-live-run: untested scope, assigned to ht-z41e.10.
- No parked findings, BLOCKED-AUTH lines or recurring blockers in the ledger.

## Verdict
not ready (bounded review: whole-range coverage and provenance gaps remain; full suite and live side-queue qualification at the final SHA not yet run)

## Must fix before landing
- none

## Untested scope
- Bounded review: provenance gaps at run.md:124 inv-f5f76bdd-1609-4df6-8910-d80c7bb25568 (incomplete claimed collection; incomplete split bundle, missing cluster report or slices) remain, and a whole-range review is still outstanding.
- Not inspected: every test file under tests/, the test-only hunks of src/host/native.rs (@@ -4991, -5977, -6630), the remaining src/cli/commands.rs hunks, the protocol command, result and output hunks (src/protocol/commands.rs, results.rs, mod.rs, service.rs, handoff.rs), the small src/ hunks (kicks, hook, nudge_threshold, peer_launch, test_support, other harness decode one-liners, permission_rules), the remaining scripts/wake-e2e.py hunks, wake_e2e_queue.py and the selftests, scripts/tests, evidence/ logs, RELEASE.md, README.md, docs/install.md, docs/agent-usage.md, integrations/codex/README.md, and the run documents under docs/superpowers/runs.
- No full-suite measurement of this branch exists yet (ht-z41e.27 is open, deferred to the caller). The branch is not tested as a whole.
- The live wake e2e at the final SHA has not run (ht-z41e.10 is open): side-queue/group (including the focused pane and the busy side turn the design requires), side-native-defer/group, side-nudge and side-draft in both modes, the default Codex --queue-proof matrix with per-seat enablement, and the ignored herdr_release_wake side-close path.
- Lone-root queue wakes now go through root-scoped `AwaitingReceipt`, and no live queue-proof run has observed that at the head.

## Deferred OK
- `session_contacts` in src/service/delivery_pull.rs is never pruned (slow memory growth per native session).
- `ROOTS_MAX` clear-all in src/notification/queue.rs drops in-flight roots (fails open to no receipt tracking).
- The `reset_empty` change in src/host/native.rs `observe_target_with_composer` leaves its rationale undocumented. `idle_wake_gate` still resets on its own read.
- docs/evidence/ansi-composer/README.md:22 wording reads as contradictory.
- The journaled check-in retry carrying `auxiliary_session` is refused only after a daemon downgrade (ledger Task 11).
- `queue_text_ok` relies on the selection invariant that the root is `cooperative_top_level` (ledger Task 24).
- A lone working root costs one extra capture/prepare round trip per retry before the arm-time refusal (ledger Task 26).
- The recurring missing-evidence process cluster (×7) belongs to process, not product. It is covered by the pending full-suite sweep.
