# ht-54r.90: fix-round tests proven red on the pre-fix code

Red/green runs: against the integration tip `f7341e91` (fix commits `212dbd0c..20955d16`). The branch was then rebased onto `82bf76b2` (ht-54r.87, .89, .91 merged); the edited test files and their neighbour modules (829 lib tests under nextest) pass there, and the .81 refusal assertion follows the .87 wording of the refusal text. The red logs were not repeated on the new base.

## Method

The brief's primary method (212dbd0c `src/` plus the tip's `tests/`) does not compile: the tip tests use
APIs the fixes introduced (`ConnectTrigger::PlainCall`, `ModBindingView.disconnect_reason`; E0599/E0560).
A compile failure is not a red result, so each fix commit used one of the fallbacks. In every case the
fix's own `src/` diff was reverse-applied and nothing else in `src/` changed.

- **A (tip revert)**: the tip tree with `git diff <fix>^ <fix> -- src | git apply -R`. Tests are the tip's,
  including the strengthened ones. Used for .76, .78, .82, .83. For .79 two hunks of
  `src/service/delivery_pull.rs` did not reverse-apply (later fixes touched the neighbours) and were
  reverted by hand (the `COOPERATIVE_TOP_LEVEL_PROVENANCE` import and gate).
- **B (fix-commit tree)**: a tree at the fix commit with its own `src/` diff reverse-applied. Used for .77,
  .80, .81 (the later fixes edit the same `delivery_pull.rs` lines, so a tip revert is not clean). For .77,
  `tests/service/mod_channels.rs` is taken from `4fb24bc5^` (its only .77 change is the fixture field
  `disconnect_reason: None`, which needs the fix to compile). For .81 the `ConnectTrigger::PlainCall` variant is
  kept as a compile shim (nothing in `src/` constructs it), and `tests/store/receipts.rs` is the task branch's
  (strengthened) file.

Logs: `ht-54r.<n>-old.log` (red run, first lines say which tree and revert) and `ht-54r.<n>-tip.log` (green run
at the task branch). The `.81` e2e test ran under `cargo nextest` (`ht-54r.81-e2e-{old,tip}.log`); the leak check
(`scripts/check-no-leaked-processes --run-id ...`) reported nothing afterwards.

## Per test

"red" means the old code failed the test; "pass" means the old code also passed it.

| bead | file | test | old | tip | strengthened | method |
|---|---|---|---|---|---|---|
| .76 | tests/store/mod_ack.rs | mod_ack_of_ordinary_and_lazy_advances_the_cursor_past_both | red | ok | `>= 2` replaced by `== thread tail (3)`: ordinary seq 1, lazy seq 2, the ack event seq 3 the walk skips | A |
| .76 | tests/store/mod_ack.rs | mod_ack_of_only_the_lazy_row_stops_before_an_unshown_ordinary_message | pass, now red | ok | fixture: shown o5 (seq 1), unshown o6 (seq 2), lazy (seq 3); acks `[o5, lazy]`; cursor `== sequence_of(o5)`; o6 stays pending. The old fixture (cursor 0 either way) could not tell the fix from the old code. Its `receipt_state` read the `receipts` table, which a manifest message never updates, so `pending` was decoration: now `manifest_receipt_state` | A |
| .76 | tests/store/mod_ack.rs | truncated_ordinary_next_to_a_lazy_row_does_not_let_the_cursor_pass | pass, now red | ok | fixture: shown ok1 (seq 1), truncated big (seq 2), lazy (seq 3); cursor `== sequence_of(ok1)` (0 before the fix, 3 if it passed big) | A |
| .76 | tests/store/mod_ack.rs | already_settled_ordinary_still_advances_a_second_bindings_cursor | red | ok | the two `>= 1` replaced by `== thread tail (2)` for each binding (o6 seq 1, ack event seq 2; the already-settled display writes no event) | A |
| .76 | tests/service/lazy_hook_pull.rs | hook_pull_confirm_of_ordinary_and_lazy_advances_the_cursor_and_does_not_reoffer | red | ok | `c >= lazy_sequence` replaced by `== thread tail` (and `tail > lazy_sequence`) | A |
| .78 | tests/store/control.rs | continuity_reattach_matches_when_a_later_binding_enrolled_after_the_main_session | red | ok | no | A |
| .78 | tests/store/control.rs | continuity_reattach_reopens_the_sessions_binding_with_its_cursors | red | ok | no | A |
| .78 | tests/store/control.rs | continuity_reattach_without_a_reopenable_row_enrolls_at_the_seat_read_position | red | ok | no | A |
| .78 | tests/cli/hook.rs | a_reused_intent_with_another_fingerprint_submits_the_current_one | red | ok | no | A |
| .78 | tests/cli/hook.rs | a_reused_intent_without_a_fingerprint_submits_the_current_one | red | ok | no | A |
| .79 | tests/service/mod_ack_dispatch.rs | a_plain_call_binding_with_a_live_channel_mod_acks_and_keeps_its_provenance | red | ok | no | A (hand-reverted) |
| .79 | tests/service/delivery_pull.rs | a_plain_call_enrolled_hook_pull_binding_is_served | red | ok | no | A (hand-reverted) |
| .79 | tests/service/mod_channels.rs | watch_register_accepts_a_plain_call_enrolled_binding | red | ok | no | A (hand-reverted) |
| .79 | tests/store/bindings.rs | plain_call_enrollment_never_supersedes_a_same_process_binding | red | ok | no | A (hand-reverted) |
| .79 | tests/service/cooperative.rs, tests/service/liveness_worker.rs | hermes_child_cli_call_before_subagent_start_leaves_the_parent_connected, a_plain_call_only_binding_neither_supersedes_nor_is_superseded | not run | n/a | owned by Task 51 | |
| .77 | tests/service/delivery_pull.rs | a_pull_re_adopts_a_migrated_binding_and_delivers | red | ok | no | B |
| .77 | tests/service/delivery_pull.rs | a_pull_without_a_fingerprint_does_not_re_adopt_and_is_refused_no_binding | red | ok | no | B |
| .77 | tests/service/delivery_pull.rs | a_pull_on_a_disconnected_non_migrated_binding_is_refused_no_binding (renamed by .81 to a_pull_reconnects_a_disconnected_non_migrated_binding_and_delivers) | red | see .81 | no | B |
| .77 | tests/cli/hook_local_evidence.rs | hook_step_runs_for_top_level_observer_events_with_claim_state_dir_and_daemon (claim carries the fingerprint) | red | ok | no | B |
| .77 | tests/harness/agy_delivery.rs | a_no_binding_refusal_keeps_the_state_file | **pass** | ok | **not strengthenable in tests**: it pins the agy client (`src/harness/agy/delivery.rs`), which this fix does not change and which already keeps the state file on every refusal but `session_mismatch`. The server half of the fix (no_binding instead of session_mismatch) is red in the three tests above. Reported, not edited | B |
| .81 | tests/service/delivery_pull.rs | a_pull_reconnects_a_disconnected_non_migrated_binding_and_delivers | red | ok | no | B |
| .81 | tests/store/receipts.rs | disconnected_binding_ack_reconnects_and_settles | red | ok | no | B |
| .81 | tests/store/receipts.rs | display_ack_reconnects_a_disconnected_plain_call_enrolled_binding | red | ok | no | B |
| .81 | tests/store/receipts.rs | fingerprintless_ack_of_a_disconnected_binding_is_refused | pass, now red | ok | adds `error.detail.contains("fingerprinted call")` (the .87 wording; .81 originally said "liveness fingerprint"): the old code refused too, with "cooperative occupant context changed" | B |
| .81 | tests/store/receipts.rs | fingerprintless_display_ack_of_a_disconnected_binding_is_refused | pass, now red | ok | same assertion (old: "display ACK requires a connected top-level agent binding...") | B |
| .81 | tests/store/first_contact.rs | plain_call_reconnect_never_fast_forwards | red | ok | no | B |
| .81 | tests/integration/session_bindings_e2e.rs | a_one_shot_adds_sends_exits_and_a_later_call_reconnects_until_its_process_exits | red (times out waiting for the reconnect: `disconnected_at > first_disconnect` never holds) | ok | no | B |
| .81 | tests/service/liveness_worker.rs | a_dead_fingerprint_reconnect_is_disconnected_on_the_next_pass | **pass** | ok | owned by Task 51 (file): not edited | B |
| .81 | tests/store/bindings.rs | plain_call_with_a_fingerprint_reconnects_a_disconnected_binding, superseded_binding_plain_call_reconnects_without_superseding | red | ok | owned by Task 51: not edited | B |
| .81 | tests/store/bindings.rs | plain_call_without_a_fingerprint_serves_a_disconnected_binding_without_reconnecting, plain_call_on_a_retired_seat_does_not_reconnect | **pass** | ok | owned by Task 51 (weak: negative guards): not edited | B |
| .81 | tests/service/liveness_worker.rs | a_stale_callers_reconnect_of_a_superseded_binding_is_superseded_again | **pass** | ok | owned by Task 51: not edited | B |
| .81 | tests/service/cooperative.rs | hermes_registered_child_call_after_subagent_stop_reconnects_it | red | ok | owned by Task 51: not edited | B |
| .80 | tests/harness/caller_env.rs | agy_child_conversation_is_a_subagent_claim_of_its_parent, agy_conversation_without_lock_or_listing_is_unattributed, agy_invalid_conversation_id_is_unattributed, agy_top_level_conversation_with_a_presence_lock, agy_unreadable_listing_is_unattributed_never_top_level | pass (characterization, no defect to catch; the commit message names none) | ok | no | B |
| .82 | tests/scheduler/dispatch.rs | single_target_composer_wake_requires_the_foreground_match | red | ok | no | A |
| .82 | tests/scheduler/dispatch.rs | a_launch_placeholder_target_matches_the_started_process | pass, now red | ok | adds the mismatch case (foreground pid 43 against fingerprint 42: pre-send refusal, nothing prompted); the old single-harness rule typed anyway | A |
| .82 | tests/store/wake.rs | composer_choice_over_wake_targets_is_the_unique_fingerprint_pid_match (single-target block) | red | ok | no | A |
| .83 | tests/protocol/output.rs | compact_participants_mark_active_and_inactive_members | red | ok | no | A |
| .83 | tests/cli/golden/{participants,thread}.compact.txt via cli::human::golden_contract::golden_files_match | red | ok | no | A |

## Not fixed here

- The `wait_until` race in `tests/integration/session_bindings_smoke.rs` (the `/clear` block) is owned by Task 51.
- `tests/harness/agy_delivery.rs::a_no_binding_refusal_keeps_the_state_file` cannot fail on the old code without a
  `src/` change (see its row). No `src/` file was touched.
