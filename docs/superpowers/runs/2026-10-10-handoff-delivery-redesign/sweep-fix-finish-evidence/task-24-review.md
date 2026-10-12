# Task ht-0lo.16 light review

Status: CLEAN.
Base: dc15706ead701e609ce530c69dd170ecb6ad123c.
Author HEAD: d0444f797e8c3c5f736c41faf4a6edf6b0ca51b9.

## Spec compliance

✅ Current bead description, frozen Task 24 and brief agree. Read `bd --sandbox --readonly show ht-0lo.16 --json` and `bd --sandbox --readonly comments ht-0lo.16`; no comments or task amendments exist. Parent runtime override supplements the older report with actual focused GREEN and required gates, and prohibits reviewer runtime/tracker/Herdr/source mutations.

✅ Only tests/daemon/watch_connection.rs changes. Positive test explicitly elects true, asserts each of four actual advertised capabilities and expects the independent fixture token directly. Original post-outcome false/true helper checks and historical omission-byte assertion remain intact. Shared request helper remains false. Added negative control deliberately supplies a fake lease but tests real serve socket output as both decoded None and literal historical JSON shape. No production or public/V1 representation changes.

✅ Correction follows the independent policy in TRUST-POLICY.md:1872: absent/false election retains historical wire, and capable Live owners receive leases. Removing the implementation-derived expected-value helper from the positive oracle strengthens its preservation check. The actual-wire negative oracle is historical literal JSON with only envelope version/instance/boot interpolated. No assertion is loosened, skipped or deleted.

## Issues

Critical: none.
Important: none.
Minor: none.

## Focused checks outside the diff

Risk: the request fixture might still exercise a stub instead of real wire emission. Checked the shared helper/start path at tests/daemon/watch_connection.rs:155 and :194: false defaults are unchanged, start invokes real serve over UnixStream::pair; WatchFake supplies the known registration lease. Positive and negative assertions consume socket frames. No shared boundary or production entrypoint changed, so a wider reachability audit is unnecessary.

Risk: election requirements might merely be copied from implementation. Checked independent TRUST-POLICY.md:1872 and WatchRequest contract at src/protocol/watch.rs:262; checked actual transport election at src/daemon/transport/watch_connection.rs:159 and CLI capability negotiation at src/cli/watch.rs:359. These corroborate the selected policy and four-capability prerequisites rather than supplying expected token or wire bytes.

## Test evidence

The implementer report accurately records original RED command/output and explicitly leaves runtime validation to the parent. Dispatch and subsequent parent evidence supersede its pending wording. Actual parent command from /private/tmp/ht-sweep-fix-focused-corrected-f746/execution.json:

```sh
nice cargo nextest run --locked --all-features --lib --no-fail-fast --success-output immediate -E 'test(=cli::watch::tests::q3_all_watch_mode_capabilities_send_true_and_use_the_accepted_lease) | test(=cli::watch::tests::q3_missing_binding_cursors_capability_keeps_old_wire_and_consumer) | test(=cli::watch::tests::q3_missing_generic_presentation_capability_keeps_old_wire_and_consumer) | test(=cli::watch::tests::q3_missing_watch_mode_capability_keeps_old_wire_and_consumer) | test(=daemon::transport::watch_connection::tests::accepted_reply_carries_attention_version_and_the_claim) | test(=daemon::transport::watch_connection::tests::legacy_accepted_wire_omits_even_a_registered_presentation_lease) | test(=daemon::transport::watch_connection::tests::presentation_owner_accepted_wire_carries_the_registered_lease) | test(=protocol::watch::tests::accepted_presentation_lease_survives_new_decoder_and_old_bytes_stay_omitted) | test(=service::delivery_pull::tests::presentation_capacity_last_slot_parks_unknown_carry_and_only_continues_for_due_keys) | test(=service::delivery_pull::tests::presentation_full_pending_carry_reaches_quiet_without_evicting_offers) | test(=service::delivery_pull::tests::presentation_full_pending_carry_serves_due_known_offers_without_admitting_fresh_id) | test(=service::delivery_pull::tests::presentation_watch_settled_receipts_reclaim_live_offer_capacity) | test(=service::delivery_pull::tests::q4_accept_controls::presentation_q4_accept_fanout_includes_disconnected_open_and_keeps_absent_stale_consumers) | test(=service::delivery_pull::tests::q4_accept_controls::presentation_q4_accept_only_reopens_quiet_watch_and_hook_with_preserved_reader) | test(=service::delivery_pull::tests::q4_accept_controls::presentation_q4_accept_preserves_real_nonempty_carry_outbox_and_full_offer_window) | test(=service::delivery_pull::tests::q4_accept_controls::presentation_q4_accept_private_failure_rolls_back_episode_join_all_readers) | test(=service::mod_channels::tests::domain::q3_legacy_claim_only_registration_has_no_presentation_lease) | test(=service::mod_channels::tests::domain::q3_mixed_same_seat_modes_and_replacement_cannot_borrow_old_leases) | test(=store::attention::tests::presentation_capacity_due_exact_keys_have_bounded_actual_sql_plans_and_vm) | test(=store::attention::tests::presentation_exact_recovery_keeps_sibling_unread_after_receipt_and_lazy_settlement) | test(=store::queries::tests::query_work_q4::q4_episode_private_and_reconciliation_points_have_constant_sqlite_work) | test(=store::queries::tests::query_work_q4::q4_private_fresh_excludes_warning_history) | test(=store::queries::tests::query_work_q4::q4_private_resumed_excludes_warning_history) | test(=store::queries::tests::query_work_q4::q4_public_and_origin_warning_sources_remain_enabled) | test(=store::queries::tests::query_work_q4::q4_raw_hundred_scope_charge_and_lazy_due_fourteen_use_actual_helpers) | test(=store::queries::tests::query_work_q4::q4_scalar_agreement_and_strict_retirement_use_exact_scope)'
```

Run log: `Summary [13.506s] 26 tests run: 26 passed, 4917 skipped`. Includes both changed tests, existing accepted/legacy decoder controls, Q3 modern negotiation and each missing-capability control, real legacy registration and mixed-channel ownership controls. Original modern test reached its complete retained post-setup assertions. Run exit 0; scoped leak check exit 0 for aa07d15e-4108-41cd-8b65-f16cde5c0b5e. This evidence plausibly covers the task; no failure is glossed over and no runtime checks were rerun by this reviewer.

Blob equality verified with `git rev-parse HEAD:tests/daemon/watch_connection.rs 0b04f91ba5fbaf4f5df4819fe8ccda6a704c26a2:tests/daemon/watch_connection.rs`: both a9bbd747f730905c171e25d6c2204edc996452e8. Parent validated HEAD 0b04f91ba5fbaf4f5df4819fe8ccda6a704c26a2, tree 29f7d42d742fa09a56bf64f9de1648420e2565e3.

Required combined gates recorded in /private/tmp/ht-sweep-combined-gates-0b04/execution.json: cargo fmt -- --check exit0 (4.499s); scripts/check-harness-neutral exit0 (1.316s); nice cargo clippy --locked --all-targets --all-features -- -D warnings exit0 (22.406s); nice scripts/check-default-features exit0 (8.878s). No test/Cargo was run here.

Test changes commands (both exit0):

```sh
git diff --stat dc15706ead701e609ce530c69dd170ecb6ad123c..HEAD -- 'tests/**' 'test/**' 'spec/**' '**/tests/**' '**/test/**' '**/spec/**' '*_test.*' '*.test.*' 'test_*.*' '*_spec.*' '*.spec.*' '**/*_test.*' '**/*.test.*' '**/test_*.*' '**/*_spec.*' '**/*.spec.*'
git diff dc15706ead701e609ce530c69dd170ecb6ad123c..HEAD -- 'tests/**' 'test/**' 'spec/**' '**/tests/**' '**/test/**' '**/spec/**' '*_test.*' '*.test.*' 'test_*.*' '*_spec.*' '*.spec.*' '**/*_test.*' '**/*.test.*' '**/test_*.*' '**/*_spec.*' '**/*.spec.*'
```

Result: one file, 43 insertions / 5 deletions, strictly authorized fixture/oracle correction and real legacy control. Review package has one commit and nonempty changed files; ancestry check succeeded for the stated base.
