# Main v0.9.0 composed test selection proposal

Preparation only, 2026-10-11. Released Main source `b6dd5dd122772599a02e2cc7f76b20e5492f0af3`, tree `3a7ba4d535aa4d9556aff9c21207953b4b29f5a6`; frozen reference selection `f330bdab3abee8535213228e5465b085b65be750`, tree5576d252, exact391. Main names below were statically traversed from src/lib.rs and explicit Cargo roots through external/inline modules to test/tokio::test attributes. No Cargo, build/runtime, source/Git/runner mutation or subagent. This proposal is not a final immutable V090 inventory or success stamp.

Keep the original391 target/name keys (385 focused plus6 mandatory) unless an actual composed semantic rename is established on source. Preserve both C cases, all8 B wire controls, both automatic-envelope ALLSLOTS positives, D recovery/owner/bound controls and mandatory recovery30s/20ms waits. The released Main proposal below has **244 distinct concrete keys: 244 proposed focused additions and 0 already in391**. Membership is deduplicated by target/name; group counts can overlap. This is a static released-input delta, not a future final total: uncommitted author cases and final registration can change it.

The frontend author's three intentional Main integration rename dispositions affect **none of the frozen391 keys**:
- clear_rebind_stale_generation_ack_refused_and_restreamed → clear_rebind_pending_item_restreamed_to_new_session.
- clear_within_rebind_grace_emits_no_session_start_digest_and_no_native_kick → clear_new_binding_gets_no_digest_or_wake_for_mail_already_read + clear_new_binding_digests_and_wakes_for_its_unread_mail_without_grace.
- resume_reack_settles → resume_keeps_binding_and_reack_settles.

These new concrete Main behaviors are included below; do not resurrect incompatible generation-only assertions. Missing D names on standalone Main are not evidence of removal in the union. Historic D schema39/40 fixture names remain retained keys pending the schema author's truthful composed precursor adaptation; changing physical predecessors to43/44/45 must not silently create false released39 evidence.

## Concrete released Main key groups

Each row names the actual target and full registered key. `+` appends to focused membership; `=` already belongs to the original391 and needs refreshed source correspondence, not duplication. All source/line references below are immutable Mainb6dd, not active V090 line numbers.

### Canonical binding identity, reconnect and lineage

Same seat/generation cannot alias session/agent bindings. Preserve honest env versus lifecycle provenance, migrated readoption and exact process/liveness refusals.

Static group: 40 cases, 40 new to391.

| Delta | Target | Full registered key | Main source:line |
| --- | --- | --- | --- |
| + | `lib` | `store::bindings::tests::availability::lifecycle_reconnect_closes_the_episode` | `tests/store/bindings.rs:2075` |
| + | `lib` | `store::bindings::tests::binding_key_from_claim_reads_seat_harness_session_and_agent` | `tests/store/bindings.rs:344` |
| + | `lib` | `store::bindings::tests::exclusivity::a_plain_call_enrollment_never_adopts_the_placeholder` | `tests/store/bindings.rs:1545` |
| + | `lib` | `store::bindings::tests::lineage::lineage_from_another_process_is_ignored` | `tests/store/bindings.rs:2190` |
| + | `lib` | `store::bindings::tests::lineage::lineage_successor_starts_at_predecessor_cursors` | `tests/store/bindings.rs:2171` |
| + | `lib` | `store::bindings::tests::lineage::link_lineage_never_moves_cursors` | `tests/store/bindings.rs:2242` |
| + | `lib` | `store::bindings::tests::lineage::validate_lineage_requires_a_top_level_predecessor_of_the_same_seat_and_harness` | `tests/store/bindings.rs:2208` |
| + | `lib` | `store::bindings::tests::lineage_successor_copies_predecessor_cursors` | `tests/store/bindings.rs:241` |
| + | `lib` | `store::bindings::tests::liveness::different_harness_same_pid_does_not_supersede` | `tests/store/bindings.rs:1900` |
| + | `lib` | `store::bindings::tests::liveness::lineage_successor_copies_cursors_before_superseding` | `tests/store/bindings.rs:1914` |
| + | `lib` | `store::bindings::tests::liveness::pid_reuse_does_not_supersede` | `tests/store/bindings.rs:1891` |
| + | `lib` | `store::bindings::tests::liveness::plain_call_enrollment_never_supersedes_a_same_process_binding` | `tests/store/bindings.rs:1835` |
| + | `lib` | `store::bindings::tests::liveness::same_pid_supersede_disconnects_the_prior_top_level_binding_at_enrollment` | `tests/store/bindings.rs:1799` |
| + | `lib` | `store::bindings::tests::liveness::subagents_never_supersede_nor_are_superseded` | `tests/store/bindings.rs:1853` |
| + | `lib` | `store::bindings::tests::match_claim_distinguishes_agent_ids` | `tests/store/bindings.rs:132` |
| + | `lib` | `store::bindings::tests::match_claim_ignores_ended_rows` | `tests/store/bindings.rs:116` |
| + | `lib` | `store::bindings::tests::registries::enroll_hooks_are_the_placeholder_replace_then_the_same_pid_supersede` | `tests/store/bindings.rs:2273` |
| + | `lib` | `store::bindings::tests::resolution::attach_subagent_attaches_to_an_existing_parent_without_touching_it` | `tests/store/bindings.rs:1074` |
| + | `lib` | `store::bindings::tests::resolution::codex_subagent_uses_env_session_provenance` | `tests/store/bindings.rs:1065` |
| + | `lib` | `store::bindings::tests::resolution::hermes_cli_claim_of_a_registered_child_resolves_to_the_subagent_binding` | `tests/store/bindings.rs:569` |
| + | `lib` | `store::bindings::tests::resolution::lifecycle_enrollment_uses_the_harness_cooperative_provenance` | `tests/store/bindings.rs:638` |
| + | `lib` | `store::bindings::tests::resolution::lifecycle_reconnects_a_disconnected_binding_and_runs_hooks` | `tests/store/bindings.rs:928` |
| + | `lib` | `store::bindings::tests::resolution::lifecycle_refreshes_the_placement_of_an_open_binding` | `tests/store/bindings.rs:987` |
| + | `lib` | `store::bindings::tests::resolution::migrated_binding_is_readopted_by_a_plain_call_with_its_session` | `tests/store/bindings.rs:447` |
| + | `lib` | `store::bindings::tests::resolution::migrated_binding_with_another_session_is_not_readopted` | `tests/store/bindings.rs:483` |
| + | `lib` | `store::bindings::tests::resolution::plain_call_does_not_reconnect_an_end_event_binding` | `tests/store/bindings.rs:789` |
| + | `lib` | `store::bindings::tests::resolution::plain_call_does_not_reconnect_when_a_lifecycle_binding_holds_the_process` | `tests/store/bindings.rs:805` |
| + | `lib` | `store::bindings::tests::resolution::plain_call_of_an_unknown_session_enrolls_with_env_provenance` | `tests/store/bindings.rs:539` |
| + | `lib` | `store::bindings::tests::resolution::plain_call_on_a_retired_seat_does_not_reconnect` | `tests/store/bindings.rs:914` |
| + | `lib` | `store::bindings::tests::resolution::plain_call_with_a_fingerprint_reconnects_a_disconnected_binding` | `tests/store/bindings.rs:654` |
| + | `lib` | `store::bindings::tests::resolution::plain_call_without_a_fingerprint_serves_a_disconnected_binding_without_reconnecting` | `tests/store/bindings.rs:686` |
| + | `lib` | `store::bindings::tests::resolution::readoption_records_the_fingerprint` | `tests/store/bindings.rs:500` |
| + | `lib` | `store::bindings::tests::resolution::subagent_claim_attaches_to_its_parent_and_enrolls_a_missing_parent` | `tests/store/bindings.rs:1039` |
| + | `lib` | `store::bindings::tests::resolution::subagent_role_without_agent_id_is_invalid` | `tests/store/bindings.rs:1104` |
| + | `lib` | `store::bindings::tests::resolution::superseded_binding_plain_call_stays_disconnected_and_supersedes_nothing` | `tests/store/bindings.rs:711` |
| + | `lib` | `store::bindings::tests::resolution::two_sessions_resolve_to_two_open_bindings` | `tests/store/bindings.rs:1019` |
| + | `lib` | `store::bindings::tests::resolution::v32_readopt_refused_while_a_lifecycle_binding_holds_the_process` | `tests/store/bindings.rs:886` |
| + | `lib` | `store::bindings::tests::resolution::v32_readopted_binding_then_session_start_successor_stays_superseded` | `tests/store/bindings.rs:856` |
| + | `lib` | `store::bindings::tests::subagent_records_parent_and_parent_of_reads_it` | `tests/store/bindings.rs:218` |
| + | `lib` | `store::bindings::tests::two_sessions_hold_two_open_bindings_on_one_seat` | `tests/store/bindings.rs:94` |

### Binding cursors, ordinary inbox and lazy reader isolation

Per-binding unread stream, own-display contiguous cursor, seat-level invitations/obligations and retained legacy lazy rows. These are canonical store controls, not V2 mocks.

Static group: 33 cases, 33 new to391.

| Delta | Target | Full registered key | Main source:line |
| --- | --- | --- | --- |
| + | `lib` | `store::lazy_delivery::binding_lazy_tests::a_binding_less_completion_counts_for_every_open_top_level_binding` | `tests/store/binding_lazy.rs:268` |
| + | `lib` | `store::lazy_delivery::binding_lazy_tests::a_later_binding_inherits_the_seat_maximum_cursor` | `tests/store/binding_lazy.rs:104` |
| + | `lib` | `store::lazy_delivery::binding_lazy_tests::a_seat_without_a_connected_top_level_binding_counts_only_legacy_rows` | `tests/store/binding_lazy.rs:172` |
| + | `lib` | `store::lazy_delivery::binding_lazy_tests::advance_through_displayed_stops_at_the_first_gap` | `tests/store/binding_lazy.rs:214` |
| + | `lib` | `store::lazy_delivery::binding_lazy_tests::completion_state_follows_the_binding` | `tests/store/binding_lazy.rs:279` |
| + | `lib` | `store::lazy_delivery::binding_lazy_tests::has_unread_is_lazy_aware` | `tests/store/binding_lazy.rs:256` |
| + | `lib` | `store::lazy_delivery::binding_lazy_tests::lazy_message_is_delivered_once_per_binding` | `tests/store/binding_lazy.rs:67` |
| + | `lib` | `store::lazy_delivery::binding_lazy_tests::left_threads_are_not_lazy_unread_for_a_binding_without_a_cursor` | `tests/store/binding_lazy.rs:313` |
| + | `lib` | `store::lazy_delivery::binding_lazy_tests::legacy_pending_is_shown_to_top_level_bindings_until_displayed` | `tests/store/binding_lazy.rs:119` |
| + | `lib` | `store::lazy_delivery::binding_lazy_tests::legacy_pending_survives_a_subagent_display_attempt` | `tests/store/binding_lazy.rs:153` |
| + | `lib` | `store::lazy_delivery::binding_lazy_tests::legacy_pending_survives_leaving_the_thread` | `tests/store/binding_lazy.rs:348` |
| + | `lib` | `store::queries::binding_inbox_tests::binding_enrolled_after_leave_gets_no_old_lazy_history` | `tests/store/binding_inbox.rs:751` |
| + | `lib` | `store::queries::binding_inbox_tests::bound_lazy_stream_is_per_binding` | `tests/store/binding_inbox.rs:718` |
| + | `lib` | `store::queries::binding_inbox_tests::bound_warning_cutoff_is_the_reading_bindings_own_offer_frontier` | `tests/store/binding_inbox.rs:821` |
| + | `lib` | `store::queries::binding_inbox_tests::invitations_are_listed_per_seat` | `tests/store/binding_inbox.rs:441` |
| + | `lib` | `store::queries::binding_inbox_tests::join_gives_every_open_binding_a_cursor_at_the_join_point` | `tests/store/binding_inbox.rs:505` |
| + | `lib` | `store::queries::binding_inbox_tests::no_duplicate_delivery_after_v29_migration` | `tests/store/binding_inbox.rs:567` |
| + | `lib` | `store::queries::binding_inbox_tests::one_bindings_display_never_hides_mail_from_the_other` | `tests/store/binding_inbox.rs:364` |
| + | `lib` | `store::queries::binding_inbox_tests::pending_obligations_older_than_the_binding_are_listed` | `tests/store/binding_inbox.rs:421` |
| + | `lib` | `store::queries::binding_inbox_tests::subagent_display_advances_only_its_own_cursor` | `tests/store/binding_inbox.rs:390` |
| + | `lib` | `store::queries::binding_inbox_tests::two_bindings_each_receive_every_message_exactly_once` | `tests/store/binding_inbox.rs:342` |
| + | `lib` | `store::queries::binding_inbox_tests::unknown_session_inbox_shows_no_backlog_past_the_seat_maximum` | `tests/store/binding_inbox.rs:460` |
| + | `lib` | `store::read_cursors::tests::advance_is_monotonic` | `tests/store/read_cursors.rs:52` |
| + | `lib` | `store::read_cursors::tests::advance_through_displayed_counts_only_this_bindings_display_acks` | `tests/store/read_cursors.rs:362` |
| + | `lib` | `store::read_cursors::tests::advance_through_displayed_stops_at_the_first_gap` | `tests/store/read_cursors.rs:324` |
| + | `lib` | `store::read_cursors::tests::copy_lineage_copies_and_keeps_the_higher_cursor` | `tests/store/read_cursors.rs:102` |
| + | `lib` | `store::read_cursors::tests::has_unread_tracks_ordinary_mail_past_the_cursor` | `tests/store/read_cursors.rs:286` |
| + | `lib` | `store::read_cursors::tests::init_for_join_gives_every_open_binding_a_cursor_at_the_join_point` | `tests/store/read_cursors.rs:66` |
| + | `lib` | `store::read_cursors::tests::init_for_new_binding_starts_at_the_seat_maximum_else_head` | `tests/store/read_cursors.rs:28` |
| + | `lib` | `store::read_cursors::tests::readopt_never_lowers_a_cursor` | `tests/store/read_cursors.rs:240` |
| + | `lib` | `store::read_cursors::tests::retired_seat_keeps_its_cursor_rows` | `tests/store/read_cursors.rs:262` |
| + | `lib` | `store::read_cursors::tests::seed_modes` | `tests/store/read_cursors.rs:128` |
| + | `lib` | `store::read_cursors::tests::seed_precedence_readopt_over_lineage_over_seat_max` | `tests/store/read_cursors.rs:190` |

### ACK attribution and AlreadySettled cursor completion

First seat receipt ACK identity/time remains fixed; a sibling whole display advances only its own cursor. Disconnected/fingerprintless and migrated admissions stay canonical.

Static group: 12 cases, 12 new to391.

| Delta | Target | Full registered key | Main source:line |
| --- | --- | --- | --- |
| + | `lib` | `store::receipts::mod_ack_tests::already_settled_ordinary_still_advances_a_second_bindings_cursor` | `tests/store/mod_ack.rs:572` |
| + | `lib` | `store::receipts::mod_ack_tests::mod_ack_of_only_the_lazy_row_stops_before_an_unshown_ordinary_message` | `tests/store/mod_ack.rs:545` |
| + | `lib` | `store::receipts::mod_ack_tests::mod_ack_of_ordinary_and_lazy_advances_the_cursor_past_both` | `tests/store/mod_ack.rs:509` |
| + | `lib` | `store::receipts::mod_ack_tests::truncated_ordinary_next_to_a_lazy_row_does_not_let_the_cursor_pass` | `tests/store/mod_ack.rs:606` |
| + | `lib` | `store::receipts::tests::binding_attribution::disconnected_binding_ack_reconnects_and_settles` | `tests/store/receipts.rs:3814` |
| + | `lib` | `store::receipts::tests::binding_attribution::explicit_ack_records_the_acting_binding` | `tests/store/receipts.rs:3796` |
| + | `lib` | `store::receipts::tests::binding_attribution::fingerprintless_ack_of_a_disconnected_binding_is_refused` | `tests/store/receipts.rs:3849` |
| + | `lib` | `store::receipts::tests::binding_attribution::two_top_level_bindings_may_display_ack_settled_once` | `tests/store/receipts.rs:3752` |
| + | `lib` | `store::receipts::tests::display_ack_reconnects_a_disconnected_plain_call_enrolled_binding` | `tests/store/receipts.rs:3949` |
| + | `lib` | `store::receipts::tests::display_ack_settles_for_a_plain_call_enrolled_binding` | `tests/store/receipts.rs:3932` |
| + | `lib` | `store::receipts::tests::display_ack_settles_for_a_readopted_migrated_binding` | `tests/store/receipts.rs:3983` |
| + | `lib` | `store::receipts::tests::fingerprintless_display_ack_of_a_disconnected_binding_is_refused` | `tests/store/receipts.rs:3965` |

### Notice/frontier and bounded binding view isolation

Another binding display/settlement cannot clear this reader notice frontier; pulled notice prefix and stale-generation/session refusal remain distinct from physical V2 Confirm.

Static group: 10 cases, 10 new to391.

| Delta | Target | Full registered key | Main source:line |
| --- | --- | --- | --- |
| + | `lib` | `store::attention::tests::bound_reader_informational_notice_follows_its_own_frontier` | `tests/store/attention.rs:2367` |
| + | `lib` | `store::attention::tests::display_by_one_binding_does_not_clear_the_others_notice` | `tests/store/attention.rs:2086` |
| + | `lib` | `store::attention::tests::held_mail_is_not_pushed_until_the_catch_up_releases_it` | `tests/store/attention.rs:2177` |
| + | `lib` | `store::attention::tests::newest_pending_notice_follows_the_frontier` | `tests/store/attention.rs:2225` |
| + | `lib` | `store::attention::tests::settle_pulled_notices_refuses_a_stale_generation_another_session_or_a_non_prefix` | `tests/store/attention.rs:2299` |
| + | `lib` | `store::attention::tests::settle_pulled_notices_settles_exactly_the_carried_prefix_for_the_open_binding` | `tests/store/attention.rs:2252` |
| + | `lib` | `store::attention::tests::subagent_gets_its_own_notices_and_never_advances_the_parents` | `tests/store/attention.rs:2115` |
| + | `lib` | `store::attention::tests::two_bindings_both_get_a_notice_for_one_new_message` | `tests/store/attention.rs:2057` |
| + | `lib` | `store::seats::mod_seat_view_tests::binding_view_fingerprint_moves_with_that_bindings_unread_only` | `tests/store/mod_seat_view.rs:431` |
| + | `lib` | `store::seats::mod_seat_view_tests::ended_or_disconnected_binding_view_reports_it` | `tests/store/mod_seat_view.rs:472` |

### Existing real V1 Hook binding/reconnect/settled-mail controls

Baseline Main authority behavior for later V2 parity: sibling-settled ordinary remains readable, lazy completion is local, real readoption/reconnect and offer/outlook scopes are independent.

Static group: 17 cases, 17 new to391.

| Delta | Target | Full registered key | Main source:line |
| --- | --- | --- | --- |
| + | `lib` | `service::delivery_pull::lazy_hook_pull_tests::claude_seat_hook_context_completion_per_main` | `tests/service/lazy_hook_pull.rs:333` |
| + | `lib` | `service::delivery_pull::lazy_hook_pull_tests::hook_pull_confirm_of_ordinary_and_lazy_advances_the_cursor_and_does_not_reoffer` | `tests/service/lazy_hook_pull.rs:351` |
| + | `lib` | `service::delivery_pull::lazy_hook_pull_tests::hook_pull_late_first_pull_after_a_sibling_settled_delivers` | `tests/service/lazy_hook_pull.rs:551` |
| + | `lib` | `service::delivery_pull::lazy_hook_pull_tests::hook_pull_lazy_row_completed_by_one_binding_is_not_reoffered_to_it` | `tests/service/lazy_hook_pull.rs:566` |
| + | `lib` | `service::delivery_pull::lazy_hook_pull_tests::hook_pull_lazy_row_is_delivered_exactly_once_through_pull` | `tests/service/lazy_hook_pull.rs:262` |
| + | `lib` | `service::delivery_pull::lazy_hook_pull_tests::hook_pull_ordinary_settled_by_a_sibling_still_reaches_the_other_binding` | `tests/service/lazy_hook_pull.rs:502` |
| + | `lib` | `service::delivery_pull::lazy_hook_pull_tests::hook_pull_seat_text_inbox_completion_still_works` | `tests/service/lazy_hook_pull.rs:322` |
| + | `lib` | `service::delivery_pull::tests::a_new_session_of_one_binding_keeps_its_siblings_offers` | `tests/service/delivery_pull.rs:1141` |
| + | `lib` | `service::delivery_pull::tests::a_plain_call_enrolled_hook_pull_binding_is_served` | `tests/service/delivery_pull.rs:539` |
| + | `lib` | `service::delivery_pull::tests::a_pull_on_a_binding_the_store_never_reconnects_writes_nothing` | `tests/service/delivery_pull.rs:671` |
| + | `lib` | `service::delivery_pull::tests::a_pull_re_adopts_a_migrated_binding_and_delivers` | `tests/service/delivery_pull.rs:636` |
| + | `lib` | `service::delivery_pull::tests::a_pull_reconnects_a_disconnected_non_migrated_binding_and_delivers` | `tests/service/delivery_pull.rs:691` |
| + | `lib` | `service::delivery_pull::tests::a_pull_without_a_fingerprint_does_not_re_adopt_and_is_refused_no_binding` | `tests/service/delivery_pull.rs:661` |
| + | `lib` | `service::delivery_pull::tests::a_resume_keeps_the_binding_and_its_offers` | `tests/service/delivery_pull.rs:501` |
| + | `lib` | `service::delivery_pull::tests::alternating_pulls_keep_each_bindings_reoffer_spacing_and_count` | `tests/service/delivery_pull.rs:1109` |
| + | `lib` | `service::delivery_pull::tests::interleaved_pulls_of_two_bindings_confirm_their_own_offers` | `tests/service/delivery_pull.rs:1089` |
| + | `lib` | `service::delivery_pull::tests::the_hook_pull_outlook_waits_for_every_target_binding` | `tests/service/delivery_pull.rs:1157` |

### Live registration, same-binding replacement, ACK grace and stall

Actual canonical domain register tests plus registry interleavings. Watch exact Live admission must not inherit legacy ACK grace; one healthy sibling cannot mask another stall.

Static group: 31 cases, 31 new to391.

| Delta | Target | Full registered key | Main source:line |
| --- | --- | --- | --- |
| + | `lib` | `service::mod_channels::tests::ack_landing_during_the_stall_read_prevents_the_stall_close` | `tests/service/mod_channels.rs:1654` |
| + | `lib` | `service::mod_channels::tests::close_binding_changed_starts_that_bindings_rebind_grace_only` | `tests/service/mod_channels.rs:383` |
| + | `lib` | `service::mod_channels::tests::closing_one_channel_leaves_the_other_live` | `tests/service/mod_channels.rs:1747` |
| + | `lib` | `service::mod_channels::tests::domain::closing_one_channel_leaves_the_other_live` | `tests/service/mod_channels.rs:1389` |
| + | `lib` | `service::mod_channels::tests::domain::disconnect_closes_that_bindings_channel` | `tests/service/mod_channels.rs:1429` |
| + | `lib` | `service::mod_channels::tests::domain::each_channel_receives_only_its_bindings_deliveries` | `tests/service/mod_channels.rs:1332` |
| + | `lib` | `service::mod_channels::tests::domain::no_binding` | `tests/service/mod_channels.rs:1141` |
| + | `lib` | `service::mod_channels::tests::domain::re_registering_a_binding_replaces_only_its_own_channel` | `tests/service/mod_channels.rs:1411` |
| + | `lib` | `service::mod_channels::tests::domain::registration_uses_the_bindings_generation_not_the_claims` | `tests/service/mod_channels.rs:1014` |
| + | `lib` | `service::mod_channels::tests::domain::two_claude_bindings_hold_two_live_channels` | `tests/service/mod_channels.rs:1318` |
| + | `lib` | `service::mod_channels::tests::domain::watch_register_accepts_a_plain_call_enrolled_binding` | `tests/service/mod_channels.rs:1120` |
| + | `lib` | `service::mod_channels::tests::domain::watch_register_accepts_matching_claude_binding` | `tests/service/mod_channels.rs:1002` |
| + | `lib` | `service::mod_channels::tests::each_channel_receives_only_its_bindings_deliveries` | `tests/service/mod_channels.rs:1713` |
| + | `lib` | `service::mod_channels::tests::pass_closes_channels_whose_seat_or_binding_no_longer_holds` | `tests/service/mod_channels.rs:668` |
| + | `lib` | `service::mod_channels::tests::pass_judges_each_channel_against_its_own_binding` | `tests/service/mod_channels.rs:738` |
| + | `lib` | `service::mod_channels::tests::re_registering_a_binding_replaces_only_its_own_channel` | `tests/service/mod_channels.rs:1782` |
| + | `lib` | `service::mod_channels::tests::rebind_grace_authorizes_acks_but_is_neither_seat_nor_binding_live` | `tests/service/mod_channels.rs:1583` |
| + | `lib` | `service::mod_channels::tests::record_ack_and_attention_push_for_another_binding_are_ignored` | `tests/service/mod_channels.rs:1668` |
| + | `lib` | `service::mod_channels::tests::record_ack_resets_stall_clock` | `tests/service/mod_channels.rs:591` |
| + | `lib` | `service::mod_channels::tests::register_then_is_live_for_its_binding_only` | `tests/service/mod_channels.rs:267` |
| + | `lib` | `service::mod_channels::tests::registration_landing_between_pass_snapshot_and_close_survives` | `tests/service/mod_channels.rs:1601` |
| + | `lib` | `service::mod_channels::tests::registration_landing_during_the_stall_read_is_not_closed_and_arms_no_cooldown` | `tests/service/mod_channels.rs:1640` |
| + | `lib` | `service::mod_channels::tests::registration_over_a_configured_cap_is_busy_but_replacement_is_not` | `tests/service/mod_channels.rs:797` |
| + | `lib` | `service::mod_channels::tests::registration_over_the_connection_bound_is_busy_but_replacement_is_not` | `tests/service/mod_channels.rs:779` |
| + | `lib` | `service::mod_channels::tests::seat_stalled_needs_every_channel_stalled` | `tests/service/mod_channels.rs:1858` |
| + | `lib` | `service::mod_channels::tests::stall_close_pushes_close_stalled_to_the_channel` | `tests/service/mod_channels.rs:572` |
| + | `lib` | `service::mod_channels::tests::stall_close_starts_cooldown_refusing_same_generation_for_ten_minutes` | `tests/service/mod_channels.rs:551` |
| + | `lib` | `service::mod_channels::tests::stall_cooldown_is_per_binding` | `tests/service/mod_channels.rs:1808` |
| + | `lib` | `service::mod_channels::tests::stalled_requires_old_ack_old_receipt_published_before_last_push` | `tests/service/mod_channels.rs:495` |
| + | `lib` | `service::mod_channels::tests::truncated_receipts_never_count_toward_stall` | `tests/service/mod_channels.rs:535` |
| + | `lib` | `service::mod_channels::tests::two_claude_bindings_hold_two_live_channels` | `tests/service/mod_channels.rs:1695` |

### Independent offered ledger and bounded per-binding eviction

Retain each binding offer count/spacing, generation fence and LRU cardinality. Preserve D snapshot/replay count4 versus admission3 and logical-allocation controls from original391.

Static group: 5 cases, 5 new to391.

| Delta | Target | Full registered key | Main source:line |
| --- | --- | --- | --- |
| + | `lib` | `delivery::ledger::tests::a_newer_generation_entry_is_not_touched` | `tests/delivery/ledger.rs:402` |
| + | `lib` | `delivery::ledger::tests::a_rotated_generation_discards_only_that_bindings_offers` | `tests/delivery/ledger.rs:381` |
| + | `lib` | `delivery::ledger::tests::eviction_counts_bindings_and_keeps_the_seat_index_in_step` | `tests/delivery/ledger.rs:416` |
| + | `lib` | `delivery::ledger::tests::outlook_over_targets` | `tests/delivery/ledger.rs:447` |
| + | `lib` | `delivery::ledger::tests::two_bindings_of_one_seat_keep_separate_offers_counts_and_spacing` | `tests/delivery/ledger.rs:353` |

### Liveness verdict race and per-binding wake admission

Same-process successor, fingerprint readoption/reconnect races, dead/held/subagent refusal and cadence/cancellation remain tested instead of heuristic continuity.

Static group: 21 cases, 21 new to391.

| Delta | Target | Full registered key | Main source:line |
| --- | --- | --- | --- |
| + | `lib` | `service::liveness::liveness_worker_tests::a_dead_fingerprint_reconnect_is_disconnected_on_the_next_pass` | `tests/service/liveness_worker.rs:444` |
| + | `lib` | `service::liveness::liveness_worker_tests::a_managed_launch_placeholder_never_wins_the_same_process_rule` | `tests/service/liveness_worker.rs:356` |
| + | `lib` | `service::liveness::liveness_worker_tests::a_plain_call_only_binding_loses_to_a_lifecycle_claimed_one_and_never_wins` | `tests/service/liveness_worker.rs:263` |
| + | `lib` | `service::liveness::liveness_worker_tests::a_plain_readopted_binding_loses_to_the_lifecycle_binding_of_its_process` | `tests/service/liveness_worker.rs:326` |
| + | `lib` | `service::liveness::liveness_worker_tests::a_reconnected_superseded_lifecycle_binding_is_superseded_again` | `tests/service/liveness_worker.rs:422` |
| + | `lib` | `service::liveness::liveness_worker_tests::exited_process_disconnects_its_binding` | `tests/service/liveness_worker.rs:184` |
| + | `lib` | `service::liveness::liveness_worker_tests::fingerprintless_bindings_disconnect_on_first_pass` | `tests/service/liveness_worker.rs:498` |
| + | `lib` | `service::liveness::liveness_worker_tests::fingerprintless_verdict_is_void_after_a_fingerprinted_reconnect` | `tests/service/liveness_worker.rs:529` |
| + | `lib` | `service::liveness::liveness_worker_tests::held_unresolved_or_retired_seats_are_skipped` | `tests/service/liveness_worker.rs:558` |
| + | `lib` | `service::liveness::liveness_worker_tests::human_bindings_are_never_touched` | `tests/service/liveness_worker.rs:548` |
| + | `lib` | `service::liveness::liveness_worker_tests::idle_pass_commits_nothing` | `tests/service/liveness_worker.rs:600` |
| + | `lib` | `service::liveness::liveness_worker_tests::pid_reuse_with_another_start_disconnects` | `tests/service/liveness_worker.rs:217` |
| + | `lib` | `service::liveness::liveness_worker_tests::recheck_before_wake_disconnects_a_dead_binding_before_it_is_targeted` | `tests/service/liveness_worker.rs:620` |
| + | `lib` | `service::liveness::liveness_worker_tests::same_fingerprint_newer_session_disconnects_the_older` | `tests/service/liveness_worker.rs:229` |
| + | `lib` | `service::liveness::liveness_worker_tests::same_fingerprint_tie_goes_to_the_later_enrolled_binding` | `tests/service/liveness_worker.rs:463` |
| + | `lib` | `service::liveness::liveness_worker_tests::subagents_neither_supersede_nor_are_superseded` | `tests/service/liveness_worker.rs:474` |
| + | `lib` | `service::liveness::liveness_worker_tests::the_thread_rechecks_on_its_cadence_and_stops_on_cancel` | `tests/service/liveness_worker.rs:649` |
| + | `lib` | `store::bindings::liveness_apply_tests::process_exit_verdicts_are_unchanged` | `tests/store/liveness_apply.rs:111` |
| + | `lib` | `store::bindings::liveness_apply_tests::superseded_verdict_is_dropped_when_the_group_shrank_to_one` | `tests/store/liveness_apply.rs:95` |
| + | `lib` | `store::bindings::liveness_apply_tests::superseded_verdict_is_dropped_when_the_loser_reclaims_first` | `tests/store/liveness_apply.rs:55` |
| + | `lib` | `store::bindings::liveness_apply_tests::superseded_verdict_still_applies_when_ordering_is_unchanged` | `tests/store/liveness_apply.rs:83` |

### CLI claim/cursor plumbing and own-session Watch reads

Physical full stdout/flush before cursor advancement, truncated-prefix behavior, subagent read-only fallback and independent presentation/cursor capability negotiation.

Static group: 9 cases, 9 new to391.

| Delta | Target | Full registered key | Main source:line |
| --- | --- | --- | --- |
| + | `lib` | `cli::cooperative_tests::subagent_own_inbox_displays_advances_only_its_cursor_and_machine_read_carries_the_claim` | `tests/cli/cooperative.rs:5203` |
| + | `lib` | `cli::cooperative_tests::subagent_text_inbox_without_binding_cursors_is_a_read_only_read` | `tests/cli/cooperative.rs:5392` |
| + | `lib` | `cli::hook_inbox::tests::cli_inbox_prints_first_contact_once` | `tests/cli/hook_inbox.rs:518` |
| + | `lib` | `cli::hook_inbox::tests::inbox_sends_claim_and_advances_after_full_flush` | `tests/cli/hook_inbox.rs:313` |
| + | `lib` | `cli::hook_inbox::tests::subagent_inbox_never_sends_ack_displayed` | `tests/cli/hook_inbox.rs:396` |
| + | `lib` | `cli::hook_inbox::tests::truncated_page_advances_only_what_was_displayed` | `tests/cli/hook_inbox.rs:365` |
| + | `lib` | `cli::watch::tests::watch_drain_reads_the_registering_bindings_own_stream` | `tests/cli/watch.rs:1795` |
| + | `lib` | `cli::watch::tests::watch_drain_reads_the_seat_without_the_binding_cursors_capability` | `tests/cli/watch.rs:1815` |
| + | `lib` | `cli::watch::tests::watch_registers_with_its_own_session_claim` | `tests/cli/watch.rs:963` |

### Native lifecycle hook ownership and installed entrypoints

Union stable event-qualified registrations with Main required end/subagent events. Missing-event diagnostics and foreign/drift refusal are independent from operational exact-version gates.

Static group: 18 cases, 18 new to391.

| Delta | Target | Full registered key | Main source:line |
| --- | --- | --- | --- |
| + | `combined` | `setup_cli::doctor_flags_claude_install_missing_subagent_groups` | `tests/setup_cli.rs:5284` |
| + | `combined` | `setup_cli::doctor_flags_codex_install_missing_subagent_stop_and_session_end` | `tests/setup_cli.rs:5314` |
| + | `combined` | `setup_cli::setup_claude_registers_subagent_start_and_stop` | `tests/setup_cli.rs:513` |
| + | `combined` | `setup_cli::setup_codex_registers_subagent_stop_and_session_end` | `tests/setup_cli.rs:939` |
| + | `combined` | `setup_cli::unsetup_claude_removes_subagent_groups` | `tests/setup_cli.rs:538` |
| + | `hook_entrypoint` | `cooperative_seat_on_own_target_startup_clear_resume_register_session_bindings` | `tests/hook_entrypoint.rs:1639` |
| + | `hook_entrypoint` | `installed_claude_hook_rewrites_a_subagents_herdr_call_only` | `tests/hook_entrypoint.rs:1436` |
| + | `lib` | `cli::hook::tests::claude_subagent_pre_tool_use_rewrites_herdr_commands` | `tests/cli/hook.rs:9702` |
| + | `lib` | `cli::hook::tests::registered_resume::claude_binding_and_hermes_one_shot_coexist_on_one_seat` | `tests/cli/hook.rs:9585` |
| + | `lib` | `cli::hook::tests::registered_resume::claude_session_start_reports_no_binding_event` | `tests/cli/hook.rs:8956` |
| + | `lib` | `cli::hook::tests::registered_resume::claude_subagent_start_enrolls_its_binding_and_still_renders_guidance` | `tests/cli/hook.rs:8934` |
| + | `lib` | `cli::hook::tests::registered_resume::claude_subagent_stop_reports_a_stop_binding_event_for_its_agent` | `tests/cli/hook.rs:8910` |
| + | `lib` | `cli::hook::tests::registered_resume::codex_session_end_and_subagent_stop_report_end_binding_events` | `tests/cli/hook.rs:8984` |
| + | `lib` | `cli::hook::tests::registered_resume::hermes_finalize_and_delegate_events_report_binding_events` | `tests/cli/hook.rs:9024` |
| + | `lib` | `harness::registry::tests::required_hook_events_of_builtins_are_their_installed_sets` | `src/harness/registry.rs:3136` |
| + | `lib` | `harness::setup_tests::claude_setup_registers_event_per_group` | `tests/harness/setup.rs:2397` |
| + | `lib` | `harness::setup_tests::codex_session_plan_registers_event_per_group` | `tests/harness/setup.rs:2468` |
| + | `lib` | `harness::setup_tests::codex_setup_registers_event_per_group` | `tests/harness/setup.rs:2435` |

### Binding claim wire compatibility and canonical first contact

Absent optional binding fields retain old bytes; PID0 and defined provenance rejected/honest. First contact/cursor fast-forward must remain per binding.

Static group: 20 cases, 20 new to391.

| Delta | Target | Full registered key | Main source:line |
| --- | --- | --- | --- |
| + | `lib` | `protocol::authority::tests::binding_fields_validation_rejects_pid_zero` | `src/protocol/authority.rs:594` |
| + | `lib` | `protocol::authority::tests::claim_without_binding_fields_serializes_exactly_as_before` | `src/protocol/authority.rs:564` |
| + | `lib` | `protocol::authority::tests::session_binding_provenances_are_defined_in_trust_policy` | `src/protocol/authority.rs:627` |
| + | `lib` | `store::first_contact::tests::a_peer_thread_name_cannot_add_lines` | `tests/store/first_contact.rs:231` |
| + | `lib` | `store::first_contact::tests::a_seat_with_no_history_gets_no_first_contact` | `tests/store/first_contact.rs:425` |
| + | `lib` | `store::first_contact::tests::at_most_eight_threads_by_recent_activity_then_more_line` | `tests/store/first_contact.rs:244` |
| + | `lib` | `store::first_contact::tests::counts_cover_only_messages_up_to_the_cursor` | `tests/store/first_contact.rs:298` |
| + | `lib` | `store::first_contact::tests::fast_forward_keeps_pending_obligations_listed` | `tests/store/first_contact.rs:552` |
| + | `lib` | `store::first_contact::tests::fast_forward_triggers_above_one_page_and_not_below` | `tests/store/first_contact.rs:470` |
| + | `lib` | `store::first_contact::tests::first_contact_is_attached_once_per_binding` | `tests/store/first_contact.rs:389` |
| + | `lib` | `store::first_contact::tests::line_format_matches_the_spec` | `tests/store/first_contact.rs:188` |
| + | `lib` | `store::first_contact::tests::many_small_messages_past_a_page_fast_forward_with_gap_counts` | `tests/store/first_contact.rs:505` |
| + | `lib` | `store::first_contact::tests::open_requests_come_from_the_summary_ledger` | `tests/store/first_contact.rs:320` |
| + | `lib` | `store::first_contact::tests::plain_call_reconnect_never_fast_forwards` | `tests/store/first_contact.rs:680` |
| + | `lib` | `store::first_contact::tests::plain_touch_never_fast_forwards` | `tests/store/first_contact.rs:650` |
| + | `lib` | `store::first_contact::tests::subagent_fast_forward_never_settles` | `tests/store/first_contact.rs:594` |
| + | `lib` | `store::first_contact::tests::supersede_never_fast_forwards_the_superseded_binding` | `tests/store/first_contact.rs:700` |
| + | `lib` | `store::first_contact::tests::threads_without_earlier_history_are_omitted` | `tests/store/first_contact.rs:272` |
| + | `lib` | `store::first_contact::tests::unenrolled_inbox_page_carries_stats_until_the_enrolling_advance` | `tests/store/first_contact.rs:761` |
| + | `lib` | `store::first_contact::tests::unnamed_thread_label` | `tests/store/first_contact.rs:216` |

### Published Main32..35 schema and genuine retained delivery data

Released32..35 keep their meanings and bytes. Verify migrated binding/catalog, lazy/catch-up, frontiers, tamper refusal and complete upgrade; append logical redesign migrations36..45 separately.

Static group: 13 cases, 13 new to391.

| Delta | Target | Full registered key | Main source:line |
| --- | --- | --- | --- |
| + | `lib` | `store::connection::tests::upgrade_from_v28_with_pending_lazy_and_catch_up_mail_delivers_exactly_once` | `tests/store/schema.rs:6710` |
| + | `lib` | `store::connection::tests::v28_upgrade_migrates_bindings_to_session_keyed_v32` | `tests/store/schema.rs:6277` |
| + | `lib` | `store::connection::tests::v32_fresh_matches_upgraded_catalog` | `tests/store/schema.rs:6460` |
| + | `lib` | `store::connection::tests::v32_session_key_is_unique_only_among_open_rows` | `tests/store/schema.rs:6445` |
| + | `lib` | `store::connection::tests::v32_startup_rejects_missing_or_altered_binding_objects` | `tests/store/schema.rs:6472` |
| + | `lib` | `store::connection::tests::v33_fresh_matches_upgraded_catalog` | `tests/store/schema.rs:6818` |
| + | `lib` | `store::connection::tests::v33_startup_rejects_an_altered_archival_trigger` | `tests/store/schema.rs:6841` |
| + | `lib` | `store::connection::tests::v33_to_v34_carries_lazy_and_catch_up_state` | `tests/store/schema.rs:6611` |
| + | `lib` | `store::connection::tests::v34_fresh_matches_upgraded_catalog_with_delivery_data` | `tests/store/schema.rs:6875` |
| + | `lib` | `store::connection::tests::v34_startup_rejects_altered_catch_up_and_lazy_objects` | `tests/store/schema.rs:6893` |
| + | `lib` | `store::connection::tests::v34_to_v35_moves_attention_frontiers_to_bindings` | `tests/store/schema.rs:6939` |
| + | `lib` | `store::connection::tests::v35_fresh_matches_upgraded_catalog_with_attention_data` | `tests/store/schema.rs:7005` |
| + | `lib` | `store::connection::tests::v35_startup_rejects_altered_attention_frontier_objects` | `tests/store/schema.rs:7017` |

### Actual Main mod delivery and session-binding integration seams

Full versus partial/machine body-read cursor, promoted lazy wakes, resume keeps binding, clear new-session read position, real sibling settlement and installed lifecycle coupling.

Static group: 15 cases, 15 new to391.

| Delta | Target | Full registered key | Main source:line |
| --- | --- | --- | --- |
| + | `integration` | `mod_delivery::clear_new_binding_digests_and_wakes_for_its_unread_mail_without_grace` | `tests/integration/mod_delivery.rs:2444` |
| + | `integration` | `mod_delivery::clear_new_binding_gets_no_digest_or_wake_for_mail_already_read` | `tests/integration/mod_delivery.rs:2395` |
| + | `integration` | `mod_delivery::clear_rebind_pending_item_restreamed_to_new_session` | `tests/integration/mod_delivery.rs:2323` |
| + | `integration` | `mod_delivery::full_body_read_of_truncated_message_unblocks_binding_cursor` | `tests/integration/mod_delivery.rs:1580` |
| + | `integration` | `mod_delivery::full_body_read_passes_later_message_the_binding_already_displayed` | `tests/integration/mod_delivery.rs:1631` |
| + | `integration` | `mod_delivery::machine_and_json_body_reads_do_not_advance_binding_cursor` | `tests/integration/mod_delivery.rs:1662` |
| + | `integration` | `mod_delivery::partial_body_read_does_not_advance_binding_cursor` | `tests/integration/mod_delivery.rs:1702` |
| + | `integration` | `mod_delivery::resume_keeps_binding_and_reack_settles` | `tests/integration/mod_delivery.rs:2500` |
| + | `integration` | `mod_delivery::wake_on_all_promoted_lazy_line_carries_wakes` | `tests/integration/mod_delivery.rs:1763` |
| + | `integration` | `session_bindings_e2e::a_one_shot_adds_sends_exits_and_a_later_call_reconnects_until_its_process_exits` | `tests/integration/session_bindings_e2e.rs:134` |
| + | `integration` | `session_bindings_e2e::two_agents_in_one_pane_settle_one_obligation_once_and_a_subagent_reads_only` | `tests/integration/session_bindings_e2e.rs:31` |
| + | `integration` | `session_bindings_smoke::config1_subagent_display_advances_its_own_cursor` | `tests/integration/session_bindings_smoke.rs:832` |
| + | `integration` | `session_bindings_smoke::config5_resume_within_a_page_and_past_it` | `tests/integration/session_bindings_smoke.rs:1333` |
| + | `integration` | `session_bindings_smoke::config9_a_late_drain_after_a_sibling_settles_still_delivers` | `tests/integration/session_bindings_smoke.rs:2266` |
| + | `integration` | `session_bindings_smoke::config9_foreground_and_background_agents_in_one_pane` | `tests/integration/session_bindings_smoke.rs:2070` |

## Actual uncommitted author rows: append only after release/freeze

These exact test bodies/attributes exist in the active working source at this observation, but authors are active and the merge is not frozen. They are outside the released-Main counts above. Do not guess a final count or copy a runtime verdict from either parent.

- `lib` / `store::connection::tests::published_v35_upgrade_preserves_exact_ack_bindings_and_delivery_history`, observed `tests/store/schema.rs:8127`, fileSHA256 `5f816a643b03edda5358dd3c1d6bb6e29eed801b838da79fdde1ac912cf6a0aa`. Populated real Main35 ACK binding values in both receipt relations, sender binding, independent cursors/frontiers; reopen and rollback preservation. Final source registration, authored successor/review and enumeration pending.
- `lib` / `store::attention::tests::presentation_exact_recovery_keeps_sibling_unread_after_receipt_and_lazy_settlement`, observed `tests/store/attention.rs:4113`, fileSHA256 `bf1d29ad212a7afc44e4d1f7a112b6e7e3366cba2089be88a7f49be20e4b984d`. Exact-key due/recovery retains sibling ordinary/lazy unread after seat-level terminal settlement. Final source registration, authored successor/review and enumeration pending.
- `lib` / `store::attention::tests::pulled_notice_settlement_uses_exact_binding_when_sessions_match_across_harnesses`, observed `tests/store/attention.rs:4194`, fileSHA256 `bf1d29ad212a7afc44e4d1f7a112b6e7e3366cba2089be88a7f49be20e4b984d`. Notice settlement selects canonical BindingId even when native-session strings match across harnesses. Final source registration, authored successor/review and enumeration pending.

Additional actual D V2 same-binding/sibling controls remain forthcoming from the delivery/owner author. Include their exact released names only when present and registered; do not invent placeholders. Required rows are:

- Two concurrent same-seat/same-generation canonical bindings A/B accepted; replacement/close/reconnect/stall/lease/token actions on A preserve B.
- Ordinary and lazy sibling-settled/unread pages survive due recovery, carry/outbox revalidation and 256 occupied offers; whole-message completion advances only B cursor while original ACK binding/time/actor stays fixed.
- Binding-local informational notice and Attention/Cleared frontier/control confirmations; identical native-session across harnesses cannot alias canonical bindings.
- Wrong binding/consumer/token/live lease and ledger-incarnation/generation/execution reject with no progress; owner replay preserves count4, logical slots/outlook, finite budgets and restart behavior.
- V1 and V2 legitimate Hook plain reconnect authority parity, without a legacy ordinary digest on bounded V2; Watch Pull never enrolls/reconnects a binding.
- Actual advertised consumer/registered Hook/Watch + real mod display path, including settled ordinary passive delivery, promoted lazy wake, failed/truncated/foreign completion and independent cursor versus receipt settlement.

The existing Main V1 and SQL/registry controls above are regression seeds for this composition. They do not close newly binding-scoped durable V2 behavior by themselves.

## Conflict and status attribution

Read-only active checkout observation: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/upstream-composition-v090`, HEAD `f330bdab3abee8535213228e5465b085b65be750`, MERGE_HEAD `b6dd5dd122772599a02e2cc7f76b20e5492f0af3`; index has **32 unresolved paths**. This counts unmerged index entries, not remaining working-file conflict markers. Authors may have resolved source while parent staging remains pending. No final tree/clean-source claim is made.

Current unmerged index paths:

- `TRUST-POLICY.md`
- `integrations/claude/mod/hooks/register.js`
- `src/cli/mod.rs`
- `src/cli/watch.rs`
- `src/delivery/assemble.rs`
- `src/delivery/ledger.rs`
- `src/ports.rs`
- `src/protocol/capabilities.rs`
- `src/service/delivery_pull.rs`
- `src/service/dispatch.rs`
- `src/service/mod_channels.rs`
- `src/store/mod.rs`
- `src/store/queries.rs`
- `src/store/receipts.rs`
- `src/store/schema.rs`
- `src/store/seats.rs`
- `tests/cli/commands.rs`
- `tests/cli/cooperative.rs`
- `tests/cli/watch.rs`
- `tests/harness/setup.rs`
- `tests/integration/channel_activation.rs`
- `tests/integration/mod_delivery.rs`
- `tests/protocol/capabilities.rs`
- `tests/service/mod_channels.rs`
- `tests/setup_cli.rs`
- `tests/store/attention.rs`
- `tests/store/handoff_fences.rs`
- `tests/store/lazy26_prerequisite.rs`
- `tests/store/receipts.rs`
- `tests/store/schema.rs`
- `tests/store/topology_handoff.rs`
- `tests/store/topology_handoff_schema.rs`

Author reports are attribution, not compiled evidence: frontend and CLI-test lanes report quiescence/source union/static formatting; delivery/core and store/schema composition remain subject to final parent release. The schema impact requires immutable Main1..35, logical handoff additions36..45 and retention of both receipts/receipt_state ACK binding fields through appended37 rebuild. Fresh real Main35 upgrade/reopen/rollback and original Main31→35→combined lineage remain separate required rows. Do not use successful empty-schema SQL or old391 runtime to certify these adaptations.

The frozen f330 comparison has actual parent-run 391/391 PASS (385 focused cases in33.523s, six mandatory cases in2.684s), UUID73818e8a-3984-4a3c-ac7c-c40068546e1e; per-sweep/final scoped leak exits0. This preparer read its execution record, but ran no test. Earlier Hook/capacity failures remain nonreproduced history, not erased or waived. V090 needs a fresh exact-source matching selection.

Historical execution record: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/resume-C-evidence/f330bdab-pressure-execution.json`, SHA256 `a3c41c53fd2918408e5d6d5aed60fff0a3269934a982e16b1844886aec87ef01`.


## Parent-only finalization and execution

After all source authors release and parent commits/freezes the actual merged tree: resolve every original391 key, the concrete Main delta and the newly authored controls; record current SHA/tree, full file/function/blob hashes and registration chains. Reject missing/renamed/ignored/zero/extra selections. Maintain target-qualified binary/name predicates; lib binary is the already discovered herdr_threads. Keep six recovery cases separate; run all heavy envelope positives through nextest groups. New actual Main targets above are still within lib/combined/hook_entrypoint/integration/service, but verify explicit Cargo registration/features on the final tree.

The unchanged f330 runner intentionally permits only the immutable389 baseline plus the two original C additions. It therefore **rejects this expanded Main proposal**. Parent must prepare/review a fresh explicit admitted-key set/runner for the final V090 inventory; do not bypass the old guard or edit it under this preparation. Freeze exact new names before execution instead of using broad regexes or whole-all-targets routinely.

One parent Cargo lane remains jobs4, nice, locked/all-features and the existing sole absolute D target. Export a fresh scoped UUID before the entire focused+recovery sweep, preserve ALLSLOTS and all accepted test groups, stop owned children and check scoped leaks after success/failure. Save selected/executed names, per-case outcomes, durations, command/source/binary/executable/log hashes. Perform fmt/strict clippy/neutral/default-feature gates plus changed real plugin/lifecycle/mod proofs and independent whole-composition reviews. No runtime counts or final readiness can be inferred from this proposal.

## Inspected immutable Main source fingerprints

- `.config/nextest.toml` — SHA256 `43f442c85499e0136d8e9eba84d67bd0bb25cfc12e9ab70338b9fb308b97420b`.
- `Cargo.toml` — SHA256 `dc0beada672afffdebe30b56daf231148f87062f65804856f756426e957aadc7`.
- `src/cli/hook.rs` — SHA256 `e63c871a0ffb331def18db49d8de170b6c289d845224e09c2373acdff70e1bfc`.
- `src/cli/hook_inbox.rs` — SHA256 `222aa94997583c6a5246701e0f06e3cebf18676564cf2052d1afa9026876bc6a`.
- `src/cli/mod.rs` — SHA256 `e5325c5c5738e1c2ed50e58d2f3f8c0a15a764f0912c39a54868d37493a1c5e0`.
- `src/cli/watch.rs` — SHA256 `7c0264b5398ddde8d0041e9a4bfaf048edec67bc0c8b858e3a4626b2710e5443`.
- `src/delivery/ledger.rs` — SHA256 `be1631d723c61b7a226e4f664f1953271c5d69d17bb787894f4960aeafbe7304`.
- `src/delivery/mod.rs` — SHA256 `059a2726c775dea49f3b80350bbe45be3d9ed16a967b3613c7582d0fabcbc529`.
- `src/harness/mod.rs` — SHA256 `e2c5cbe0c148065ff10f66056da6d3f6046d4872b4c8efd82ad37123f2b6511d`.
- `src/harness/registry.rs` — SHA256 `6dfd95e735060e7084d72835ca4edc2bafcbeed5e9e24d676d3f3ff6ca5a6e2c`.
- `src/lib.rs` — SHA256 `55cf603f61276607c520f0fdab8708dfd528ff7b4c8d73769f08866eb1af4db1`.
- `src/protocol/authority.rs` — SHA256 `077a5e073ccf66ba3b231d6f8c41033038c64b9adbbf312904cb03d24152d162`.
- `src/protocol/mod.rs` — SHA256 `6be4e9b497bd399989bef5c82d45301f51fb020bdb53c9ff1fc0c569c26fea9e`.
- `src/service/delivery_pull.rs` — SHA256 `1c4dde1d73e09bbb6049db439851a98d58953f623565da0c02385a3a0b22f2d1`.
- `src/service/liveness.rs` — SHA256 `19dde64f4e2f54ca5f86ac21f7845462cc089068e33869ccdf728dfdb0e909f1`.
- `src/service/mod.rs` — SHA256 `71f046901281b54e4990c6bca568bc33a5e1248f9967f1c1ee2d6f345daec57a`.
- `src/service/mod_channels.rs` — SHA256 `dbf0b7c163f892a060876aa3bc698aeef8a369f169fd5db29598e5d732593e0a`.
- `src/store/attention.rs` — SHA256 `e542828256505879229846134e14c203621939e4d1d8710c435aa1e3aaaf24ec`.
- `src/store/bindings.rs` — SHA256 `e1525a7827908aacd6b80a2430f402ba77c5f05a3ed35077ef1551738cb2c518`.
- `src/store/connection.rs` — SHA256 `40e110ce4904fa977490f56e809dd12e2e3f28f1a9649bf8e28344027c39a2a7`.
- `src/store/first_contact.rs` — SHA256 `4ee8b9d792765a7abe7a882a3846eec13fb34468d4a3b55da98c126e75d126b4`.
- `src/store/lazy_delivery.rs` — SHA256 `1160360291e0140c8913181a754843b6cc45adaceec0c082cc26a35427d08ce9`.
- `src/store/mod.rs` — SHA256 `5ba1f3a774578797ef713d027eea80455f34f94f5587e0d545c25f73f77db42d`.
- `src/store/queries.rs` — SHA256 `6f6714563dff1d877a686ac87172da60501026ed112c00f68a06f9b7656da8aa`.
- `src/store/read_cursors.rs` — SHA256 `d7e333952363e4f92cb155736a411af8a0b3b5dd73c241ac2dce38a77c6c3c55`.
- `src/store/receipts.rs` — SHA256 `98ac9bb77084802e6de8af53ac44d4a56c8c2f8890620c9cdf970e7a08931345`.
- `src/store/seats.rs` — SHA256 `fe36776911b3c265619bbec44c3382013793a45bf9c4bfb0eb6706c2a63e3605`.
- `tests/cli/cooperative.rs` — SHA256 `136ab063f42356b1b7379c998c3901b603d0650db127c3c7c3fea5cd33d9d873`.
- `tests/cli/hook.rs` — SHA256 `7211063ecf7cc7fa50d84e8aba248fe4120145ff4c42c5534dd257e6cfd0dee7`.
- `tests/cli/hook_inbox.rs` — SHA256 `c3ef8154404e3a8554d24b68b2de4095f4019e3df077d5288367d2eb78920863`.
- `tests/cli/watch.rs` — SHA256 `3c6b8b4c5cbde4cd5df4b17d8c50bc31a1839fde50f74d53502eec981a809d54`.
- `tests/combined.rs` — SHA256 `ad1b989bfad62ef7163bbdac8c788e73ccbba4fb31393e6e7d4d4ad154932ae6`.
- `tests/delivery/ledger.rs` — SHA256 `e3e70db492ce9f6a3924d4f92929132059edc81404f4ce995a031f6be4a0af13`.
- `tests/harness/setup.rs` — SHA256 `ae845388cf6a8060674da90bcfbdcaf20cdd79cf9d85d408a6f4d621bcee6d9c`.
- `tests/hook_entrypoint.rs` — SHA256 `b1ef883a081dce4e95101d8add1a7779245565015455d67f611c3732c6c657f9`.
- `tests/integration.rs` — SHA256 `34b077528e32d30893086f6a6112bd61428ee3999f36ec7068611dedf288e108`.
- `tests/integration/mod_delivery.rs` — SHA256 `e7a7d506c7be9433ce993984948798f23e3a2a284e755930a7e26aa93e37c65c`.
- `tests/integration/session_bindings_e2e.rs` — SHA256 `1ba6aa56c217226e97b2da2566c6621695799be2acaf606763729ab572358e92`.
- `tests/integration/session_bindings_smoke.rs` — SHA256 `244f7fc55e463680ce92416e9d1ee6d4926baa104a267ae2f89d4f3ed61f99a0`.
- `tests/service/delivery_pull.rs` — SHA256 `47ddc75cbed23a2e32669e821f8ef576a832abb525e892068b77c4ec0117850f`.
- `tests/service/lazy_hook_pull.rs` — SHA256 `2780f67a34770cb951aaea08f8d9cc9870a2366e2cd7cc8c77c2e744c0ca0a9a`.
- `tests/service/liveness_worker.rs` — SHA256 `89a603303e605d869dbab5a9e327a4752e5e9c812e57f5e92ff7509e9c123536`.
- `tests/service/mod_channels.rs` — SHA256 `92c8186e946ddebde50623c929fcc201bd114c6a9c003e4c9b7afad27caa78a0`.
- `tests/setup_cli.rs` — SHA256 `bbdaf73d79b6d6c818c2cbc9bfdea144888fba3e84c44515e05c22a18b83af91`.
- `tests/store/attention.rs` — SHA256 `2ff4e019b7ca6b910d73605ed051689642eda33b32da2e1bcaeaf252adce291f`.
- `tests/store/binding_inbox.rs` — SHA256 `bbb33c8661c473447cb1891213f8f7caeefe9ab5fdfb7ae09d689e1d06cf642a`.
- `tests/store/binding_lazy.rs` — SHA256 `733a723e642f22047b2267b4b58065dc3ec89efdf426499ad6469c4ee06e28d4`.
- `tests/store/bindings.rs` — SHA256 `46813ccc68682b55e62b2951dcfea88a38683aefcc4a65d79a3ca196b185c9a1`.
- `tests/store/first_contact.rs` — SHA256 `6400eed412ae674567bc4c9f6c9be4ef094fedf56f6f01aad8d1b6ff42304a64`.
- `tests/store/liveness_apply.rs` — SHA256 `a8a90b9e883f7a8834b11a40164069b0f8ae42a4c0f1d29448fd96cd798d13fb`.
- `tests/store/mod_ack.rs` — SHA256 `ae3067360ceb3bae906a605fea402917d86bc09ee18cb1eeb140638a3288f92f`.
- `tests/store/mod_seat_view.rs` — SHA256 `fb369600911659e27354a73dcc1cda638cf14b85170aef27089616e76037ad40`.
- `tests/store/read_cursors.rs` — SHA256 `c3a746f6f7109cb3d416bd9eb1f3194c7d3d456f183312dea00ab1f7699ff5bb`.
- `tests/store/receipts.rs` — SHA256 `45343ae470fd9cfb3a59d52cae6a207c15a2102c63b6af994896041edf762e27`.
- `tests/store/schema.rs` — SHA256 `0567cb094a17282e6cbda571781bd84f52bdd1f272b129cdac389535350258eb`.
