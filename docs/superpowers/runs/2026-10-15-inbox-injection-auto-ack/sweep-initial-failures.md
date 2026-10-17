# Final affected sweep — initial failure

Measured source: d51ca79772e9896bd6e4beb6980f0080f4835b81 (later cd0c32d1 changes only run artifacts).
Command: affected-sweep-command.txt nextest command; isolated TMPDIR, HT_LEAK_RUN_ID8b584c50-9e6b-4ff5-a831-ec042ce10116.
Result:1555 passed,17 failed,4841 skipped,54.164s runtime. No leaked processes or scratch.

Failures:
- herdr-threads::hook_entrypoint agy::agy_lazy_row_settles_only_through_the_transcript_confirm_never_hook_context
- herdr-threads::hook_entrypoint cooperative_seat_on_own_target_startup_clear_resume_register_session_bindings
- herdr-threads::integration mod_delivery::agy_sweep::a_clear_rotation_discards_the_pull_ledger_and_reoffers_the_pending_message
- herdr-threads::integration mod_delivery::agy_sweep::agy_pull_confirm_and_the_claude_watch_path_coexist_on_one_daemon
- herdr-threads::integration mod_delivery::agy_sweep::wake_on_all_wakes_a_native_hook_pull_seat_for_a_lazy_row_and_nudge_never
- herdr-threads::integration mod_delivery::clear_new_binding_digests_and_wakes_for_its_unread_mail_without_grace
- herdr-threads::integration mod_delivery::clear_rebind_pending_item_restreamed_to_new_session
- herdr-threads::hook_entrypoint hook_path_takeover_replaces_a_human_occupant_with_a_new_agent_generation
- herdr-threads::hook_entrypoint installed_claude_hook_checks_in_over_socket_and_never_blocks
- herdr-threads harness::bridge_tests::hook_emits_compact_instructions_without_ack_and_child_emits_only_summary_guidance
- herdr-threads harness::cooperative::declared_child_never_dispatches_and_receipt_actions_stay_top_level
- herdr-threads harness::cooperative::each_role_renders_one_child_restriction_and_only_top_level_receipt_instructions
- herdr-threads protocol::capabilities::tests::bare_daemon_advertises_legacy_capabilities_without_v2_recorder
- herdr-threads protocol::capabilities::tests::capability_constants_are_stable
- herdr-threads protocol::capabilities::tests::every_advertised_capability_has_a_handler
- herdr-threads::hook_entrypoint two_hundred_tool_hooks_on_one_seat_stay_healthy_and_write_no_journal
- herdr-threads::combined golden_surfaces::handoff_bootstrap_plans

Observed causes: obsolete successful-lifecycle Use-inbox/shared text assertions; capability expected arrays and handler probe omit hook.context_receipts_v1; current handoff goldens retain superseded unconditional retrieval. Updated spec sections1,2,7 supersede these expectations. Frozen V1 remains frozen. Full raw output is .superpowers/final-feature-sweep.log.
