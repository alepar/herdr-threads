# Automatic inbox delivery verification matrix

Status: coverage audit of the stacked implementation; final candidate execution pending.
This document is a command plan, not a passing result or a final-SHA stamp. The
caller runs the affected checks after code roast and records their exact SHA and
results. No paid live harness run or routine full suite is required for this change.

Existing tests cover most integration seams below. The audit added the missing
native output-failure regression and a write/flush boundary test. The native hook
retry test drives the real hook entry point, and the mod truncation test follows
the emitted inbox command through the real CLI and daemon.

| Requirement | Existing coverage | Focused command |
| --- | --- | --- |
| Complete/empty native injection has no inbox or manual ACK recipe; mixed prefix, escaping, byte budgets, legacy capability and routed recovery | `tests/cli/hook_inbox.rs`: complete/empty, unavailable/incomplete, legacy, prefix and quote-dense cases | `nice cargo test --locked --all-features --lib cli::hook_inbox::tests::` |
| Exact original claim and post-flush completion; subagent display has no receipt authority | Same module: `receipt_only_completion_uses_original_claim_and_no_fake_lazy_ids`, `inbox_sends_claim_and_advances_after_full_flush`, `subagent_inbox_never_sends_ack_displayed` | Same hook-inbox command |
| Canonical full native receipt settles once; stale/disconnected/foreign/subagent/wrong-source refuses; no-receipt lazy state stays separate | `tests/store/lazy_inbox.rs`: `hook_receipt_completion_*` | `nice cargo test --locked --all-features --test combined lazy_inbox::hook_receipt_completion_` |
| Additive native-hook receipt capability and legacy empty-field serialization; bounded watch framing and exact truncation recovery routing | Protocol capability/lazy-delivery/watch tests and delivery assembler extraction tests | `nice cargo test --locked --all-features --lib protocol::capabilities::tests::`; `nice cargo test --locked --all-features --test combined lazy_delivery::`; `nice cargo test --locked --all-features --lib protocol::watch::tests::`; `nice cargo test --locked --all-features --lib delivery::page::tests::` |
| Receipt-only failure before settlement, cursor failure after settlement and lost response retry on the next actual unchanged-attention hook, then quiet | `tests/hook_entrypoint.rs`: `native_hook_completion_failures_retry_on_the_next_unchanged_attention_hook` | `nice cargo test --locked --all-features --test hook_entrypoint native_hook_completion_failures_retry_on_the_next_unchanged_attention_hook -- --exact` |
| Actual native stdout EPIPE leaves receipt/cursor unchanged, normal hook retries then becomes quiet; accepting bytes but failing flush gives no delivery authorization; encoder failure/empty/observer output does not consume context | `native_hook_broken_stdout_does_not_settle_or_hide_receipt_mail`; `tests/cli/hook.rs`: `native_output_requires_successful_write_and_flush`, `adapter_budget_caps_and_encoding_delivery_are_conservative` | `nice cargo test --locked --all-features --test hook_entrypoint native_hook_broken_stdout_does_not_settle_or_hide_receipt_mail -- --exact`; `nice cargo test --locked --all-features --lib cli::hook::tests::native_output_requires_successful_write_and_flush -- --exact`; `nice cargo test --locked --all-features --lib cli::hook::tests::adapter_budget_caps_and_encoding_delivery_are_conservative -- --exact` |
| Installed Claude lifecycle and Codex tool/user-prompt entry points inject complete mail and settle receipts | `tests/hook_entrypoint.rs`: installed Claude and `lazy_mail_reaches_the_next_codex_*` | `nice cargo test --locked --all-features --test hook_entrypoint installed_claude_hook_checks_in_over_socket_and_never_blocks -- --exact`; `nice cargo test --locked --all-features --test hook_entrypoint lazy_mail_reaches_the_next_codex_` |
| Text inbox automatic receipt/display settlement; JSON/machine/explicit-seat views remain read-only; multi-chunk continuation and frozen lost-response retry | `tests/integration/lazy_config_smoke.rs` | `nice cargo test --locked --all-features --test combined lazy_config_smoke::` |
| Text inbox independent ordinary/lazy completion failures preserve exact retry; no fabricated success on partial response | `tests/cli/lazy_settlement.rs` | `nice cargo test --locked --all-features --test combined lazy_settlement::` |
| Invitation acceptance stays separate; accept prints eligible opening mail and has bounded inbox recovery | `tests/cli/accept_delivery.rs`; real pair acceptance entry points | `nice cargo test --locked --all-features --lib cli::accept_delivery::tests::`; `nice cargo test --locked --all-features --test hook_entrypoint text_pair_accept_` |
| Channel 32/33 item and 64 KiB rendered boundary, UTF-8/quotes/framing/continuation reserve, retained or oversized tail, failed append/context, invitation nonacceptance, automatic ACK retry | `integrations/claude/mod/tests/delivery.test.ts`, including bounded-prefix and overflow tests | `scripts/test-claude-mod` (a printed skip is not passing mod evidence) |
| Truncated channel message refuses channel ACK; its exact routed text-inbox continuation settles it; truncated lazy row completes display only | `tests/integration/mod_delivery.rs`: `truncated_body_streamed_with_marker_and_ack_refused_terminal`, `truncated_lazy_row_marker_names_inbox` | `nice cargo test --locked --all-features --test integration mod_delivery::truncated_` |
| Full channel delivery provenance, duplicate inbox/channel race, reconnect retry and live-channel standard-hook suppression | Same module: provenance, concurrent ACK, reconnect and lifecycle tests | `nice cargo test --locked --all-features --test integration mod_delivery::send_attention_drain_ack_settles_with_mod_delivery_provenance -- --exact`; `nice cargo test --locked --all-features --test integration mod_delivery::concurrent_mod_ack_and_inbox_ack_settle_exactly_once -- --exact`; `nice cargo test --locked --all-features --test integration mod_delivery::reconnect_within_grace_keeps_wake_suppressed_restreams_and_late_ack_settles_once -- --exact`; `nice cargo test --locked --all-features --test integration mod_delivery::session_start_and_pre_tool_use_emit_no_digest_while_live -- --exact` |
| Hook-pull retains transcript-confirmation authority, full/truncated predicates and independent binding offers | `tests/service/delivery_pull.rs`: framed ordinary confirm, truncated refusal, ineligible callers and interleaved bindings | `nice cargo test --locked --all-features --lib service::delivery_pull::tests::pull_returns_a_framed_page_whose_ordinary_ids_settle_on_confirm -- --exact`; `nice cargo test --locked --all-features --lib service::delivery_pull::tests::a_truncated_id_is_listed_truncated_and_refused_on_confirm -- --exact`; `nice cargo test --locked --all-features --lib service::delivery_pull::tests::session_mismatch_subagent_held_and_unresolved_seats_are_refused -- --exact`; `nice cargo test --locked --all-features --lib service::delivery_pull::tests::interleaved_pulls_of_two_bindings_confirm_their_own_offers -- --exact` |

Run tests with an isolated `TMPDIR` and a fresh exported `HT_LEAK_RUN_ID`. After the
affected integration sweep, run `scripts/check-no-leaked-processes --run-id
"$HT_LEAK_RUN_ID"`; its scratch-file check is part of the result. Stop owned helpers.
The final candidate also needs `cargo fmt --check`,
`nice cargo clippy --locked --all-targets --all-features -- -D warnings`,
`scripts/check-harness-neutral`, and before merge `nice scripts/check-default-features`.

If the exact recovery command itself cannot fit, coverage must remain unavailable
so the next eligible hook retries rather than marking undisplayed work as served.
The caller must include the native-hook leaf's final no-room recovery regression
when checking its final tip; this early stacked audit precedes that refinement.

The model-visible recipe audit covered current shared hook/handoff guidance and
the Codex, Claude, Hermes and mod guides. Complete injection has no retrieval or
explicit-ACK instruction. Read-only history/body commands still require exact
explicit ACK if receipt settlement is intended. Invitation accept/reject remains
an explicit top-level decision. The Codex launch example now supplies a task rather
than an unconditional inbox request. Historical captures and older run reports
remain historical evidence, not current delivery instructions.

Cross-node readiness remains caller-owned and unverified by this matrix. After
code roast, build and review a disposable combined candidate with the p2p work;
record the exact combined SHA. Exercise native-hook, text-inbox, mod and hook-pull
settlement from recipient B back to sender A, duplicate/replayed facts,
partition/reconnect, reordered prerequisites and negative partial/read-only
cases. Automatic sources use existing wire `how=policy`; detailed
`cooperative_hook_context`/`via` observations stay on B. No `SettleHow` extension
or protocol bump is required. Local tests alone cannot supply this evidence.
