# Task 24: lease test-fixture sweep

Task: `ht-0lo.16`. Status: IMPLEMENTED (authoring complete; parent-owned focused tests, gates and one light review remain unmeasured).

Base: `dc15706ead701e609ce530c69dd170ecb6ad123c`.
Commit: `d0444f797e8c3c5f736c41faf4a6edf6b0ca51b9`.
Branch: `super-auto/sweep-ht-0lo-16`.
Own worktree: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/sweep-ht-0lo-16`.

## Implemented

Changed only `tests/daemon/watch_connection.rs`.

The existing presentation-owner test now explicitly elects `presentation_v2 = true`, requires each actual advertised prerequisite (`MOD_WATCH`, `MOD_WATCH_PRESENTATION_V2`, `DELIVERY_PRESENTATION_V2`, `BINDING_CURSORS`), and expects the independent known token directly. The transport `serve` remains real. The shared request helper remains false. The original helper false/true assertions and historical omission byte assertion after the primary outcome assertion are unchanged, as are client drop and task join.

Added `legacy_accepted_wire_omits_even_a_registered_presentation_lease`: use the shared false request against real `serve`, deliberately populate the fake service's lease with the same known token, read the actual socket frame, require decoded `None`/attention version 7, and compare the entire raw reply with independently constructed historical JSON bytes. Expected dynamic envelope values are the request protocol version and fake instance/boot; Accepted bytes are literal. Client drop and task join remain explicit.

No production source, capability list, policy, public/V1 representation, canonical history, clock, timeout, punch-list, spec Goal or code-roast exit changed. New assertion diagnostic prose is test-only (roughly 45 fixed bytes plus the capability name per failure); no new model-visible product string.

## Assertions and actual falsifiers

Every capability assertion fails if its named prerequisite is absent from actual `ADVERTISED`; it is not a fixed-size/count assertion. The modern outcome fails if transport suppresses or substitutes the known token, returns an unexpected outcome, or alters the registration attention version from 7. The preserved helper false assertion fails if false advertises the populated lease; preserved helper true fails if true suppresses or alters it; preserved historical serialization assertion fails on a lease member (even null), changed attention version, changed shape or changed serialized bytes.

The new legacy decoded assertion fails on `Some(known_token)`, any other token, refusal, or attention version other than 7. The actual raw-frame assertion fails on a lease member (even null), unexpected envelope/Accepted field, reordered fields, or changed whitespace/shape/value. Removing the request election gate in production makes the new legacy control fail despite populated fake lease; suppressing modern leases makes the positive control fail. Changing the shared helper default to true also makes the legacy control fail. The timeout/read/decode/UTF-8 unwraps fail on missing, unreadable or malformed output; task join fails on transport panic/error. No fake state is asserted as a substitute for transport output.

## Existing actual RED evidence

Read parent-owned `/private/tmp/ht-final-C-three-repro-d8427582/execution.json` and `run.log`. Candidate HEAD `d8427582d2d0bfca0ed50e5b58f29b130ee12640`, tree `53d0c453d013d7173a61d363c98b99b26508f5b5`, binary SHA256 `28b9452ed3bfdd5508121f58f98fb77f934e936bcd900227181c24c6eb5fb6cc`.

Actual command recorded in that artifact:

```text
nice cargo nextest run --locked --all-features --lib --no-fail-fast -E 'test(=daemon::transport::watch_connection::tests::presentation_owner_accepted_wire_carries_the_registered_lease) | test(=service::delivery_pull::tests::q4_accept_controls::presentation_q4_accept_preserves_real_nonempty_carry_outbox_and_full_offer_window) | test(=store::attention::tests::presentation_exact_recovery_keeps_sibling_unread_after_receipt_and_lazy_settlement)'
```

Relevant actual failure:

```text
left: Accepted(WatchAccepted { presentation_lease: None, attention_version: 7 })
right: Accepted(WatchAccepted { presentation_lease: Some("0123456789abcdef0123456789abcdef"), attention_version: 7 })
Summary [0.751s] 3 tests run: 0 passed, 3 failed, 4939 skipped
```

This agent did not execute that run or any Cargo/runtime check. It explains the false-request fixture disagreement; it is not green evidence for this commit. Original assertions after the failing outcome were unmeasured in that RED run.

## Parent-owned final focused selection

Seven minimal comparison controls, including the new actual false-request wire control:

```text
binary(=herdr-threads) & (test(=daemon::transport::watch_connection::tests::presentation_owner_accepted_wire_carries_the_registered_lease) | test(=daemon::transport::watch_connection::tests::legacy_accepted_wire_omits_even_a_registered_presentation_lease) | test(=daemon::transport::watch_connection::tests::accepted_reply_carries_attention_version_and_the_claim) | test(=protocol::watch::tests::accepted_presentation_lease_survives_new_decoder_and_old_bytes_stay_omitted) | test(=service::mod_channels::tests::domain::q3_legacy_claim_only_registration_has_no_presentation_lease) | test(=service::mod_channels::tests::domain::q3_mixed_same_seat_modes_and_replacement_cannot_borrow_old_leases) | test(=cli::watch::tests::q3_all_watch_mode_capabilities_send_true_and_use_the_accepted_lease))
```

Optional existing missing-prerequisite CLI controls remain unchanged: `cli::watch::tests::q3_missing_watch_mode_capability_keeps_old_wire_and_consumer`, `cli::watch::tests::q3_missing_generic_presentation_capability_keeps_old_wire_and_consumer`, `cli::watch::tests::q3_missing_binding_cursors_capability_keeps_old_wire_and_consumer`.

GREEN: unrun by this agent per parent runtime override; no prediction is labeled PASS. Parent must attach actual focused output and required gates before the single light review.

## Authoring verification and self-review

Ran `rustfmt --edition 2024 tests/daemon/watch_connection.rs`: exit 0. Ran `git diff --check`: exit 0, no output. Inspected the complete diff against the task; only the positive fixture/oracle and directly neighboring real-wire negative control changed. Commit has truthful `Outside-adapter: tests/daemon/watch_connection.rs` trailer. Final `git status --short` was empty. No rebase was performed, per parent override.

These are formatting and source hygiene observations only, not real test/compiler/lint evidence.

## Test hygiene

No tests, daemon, private server, product probes or persistent helper processes were started by this agent; no runtime cleanup command was executed under the parent ownership override. Parent's existing RED execution artifact records scoped leak exit 0. No agent-owned process or `survived:` line exists.

## Assumptions and concerns

Read current bead via `bd --sandbox --readonly show ht-0lo.16 --json` and comments; bead matches the frozen brief and has no comments, so no amendment was needed. Parent runtime override owns all Cargo/tests/runtime/tracker/Herdr/ROOT mutations and supersedes template test/rebase/cleanup steps. The initial worktree-index staging command ran and failed with `Operation not permitted`; narrow approved Git-only escalations staged and committed the task branch successfully. No automatic approval rejection occurred and no human action remains from that event.

Read the root design's original Goal and all accepted refinement sections in its full current file, the lease diagnosis and relevant Q3 election contract. This tests-only change does not revise those contracts. Parent's actual focused results, per-change required gates and single light review remain outstanding; this report asserts no task-completion or green runtime verdict.

## Parent runtime evidence

Actual combined validation 0b04f91ba5fbaf4f5df4819fe8ccda6a704c26a2 / tree29f7d42d742fa09a56bf64f9de1648420e2565e3: exact26 tests PASS (13.506s nextest summary, process13.737s; list/build52.117s), scoped leaks0 UUID aa07d15e-4108-41cd-8b65-f16cde5c0b5e. Exact command, selection and output: /private/tmp/ht-sweep-fix-focused-corrected-f746/execution.json and run.log. Original3 now reach full post-setup preservation assertions. Every task-changed file blob independently matches author d0444f797e8c3c5f736c41faf4a6edf6b0ca51b9. Combined required fmt/neutral/strictclippy/default PASS, evidence /private/tmp/ht-sweep-combined-gates-0b04/execution.json. No production change, cap/time limit adjustment or failed assertion waiver.
