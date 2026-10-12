# Q3 wire and registry controls author handoff

2026-10-11. Tests-only authoring after parent explicitly lifted the materialization-run source freeze. Candidate: `.worktrees/upstream-composition-v090`; parent-recorded base HEAD `16ede4ed2eaf3c24b62077f8f38cb0faf24b23cc`, tree `9b0f6151d01e5b071b4cfbda28e9af62286da9b3`. Read candidate `TRUST-POLICY.md` and the root design `resume-v090-q3-channel-design.md`. No production edits, Cargo, Git, Herdr commands, new agent, or spawned test processes.

## Owned files and release hashes

Files released quiescent, after `rustfmt --config skip_children=true` and successful `rustfmt --check --config skip_children=true`:

- `tests/cli/watch.rs`: SHA-256 `638eafdb62001f135172d0b94e11c7656c13dce1ced89438543cfe151d06ae51`.
- `tests/service/mod_channels.rs`: SHA-256 `94f2fa6ee60ad7f017929d3fa36c8395cadb10427f31519d6cc6fb455e1ca227`.

No runtime or compilation outcome is claimed. Parent owns Cargo and actual RED/PASS attribution.

## Controls and selectors

Nine new CLI tests use filter `cli::watch::tests::q3_`:

- `q3_absent_watch_opt_in_preserves_historical_bytes`: historical claim-only request and lease-absent Accepted round-trip byte exactly.
- `q3_false_watch_opt_in_normalizes_to_historical_bytes`: explicit false is accepted and omitted on serialization, without accessing a future Rust field.
- `q3_true_watch_opt_in_is_explicit_and_strictly_typed`: raw true round-trip; malformed boolean and unknown field rejection; strict old request decoder rejects true.
- `q3_missing_watch_mode_capability_keeps_old_wire_and_consumer`.
- `q3_missing_generic_presentation_capability_keeps_old_wire_and_consumer`.
- `q3_missing_binding_cursors_capability_keeps_old_wire_and_consumer`.
- `q3_all_watch_mode_capabilities_send_true_and_use_the_accepted_lease`.
- `q3_watch_discovery_failure_never_opens_a_registration_socket`.
- `q3_missing_mod_watch_capability_never_opens_a_registration_socket`.

The first-frame socket capture preserves raw bytes before a typed decoder normalizes them. Missing presentation prerequisite controls compare the entire actual Watch frame to a strict historical decoder's reserialized bytes, despite an unsolicited syntactically valid Accepted lease; their commands must remain InboxBatchV2. The missing-cursors case additionally checks absence of the reader claim. Full conjunction checks `watch.presentation_v2:true` and actual V2 Pull with the Accepted lease. Discovery-error and unsupported cases use a nonblocking listener to prove no registration attempt. The helper closes and joins its socket thread before asserting outcomes. Quiet negotiated sessions emit status lines only, never manufactured model content.

Five new real-store tests use filter `service::mod_channels::tests::domain::q3_`:

- `q3_legacy_claim_only_registration_has_no_presentation_lease`.
- `q3_legacy_channel_cannot_pull_with_an_issued_or_guessed_lease`.
- `q3_legacy_channel_cannot_capture_owner_replay`.
- `q3_legacy_channel_cannot_confirm_with_an_issued_or_guessed_lease`.
- `q3_mixed_same_seat_modes_and_replacement_cannot_borrow_old_leases`.

Reuse the existing local SqliteStore/DomainService rig and existing APIs. Where V090 erroneously supplies a legacy token, the tests use that actual token rather than a random guess: RED must expose admitted legacy authority. The mixed-mode test decodes opt-in through raw JSON, then checks a legacy same-seat sibling, capable-to-legacy and legacy-to-capable replacement, old token refusal, fresh token admission, stale generation/session refusal, reconnect grace and replay/confirmation refusal. On the current base its first failure is a runtime decoder assertion, not a missing-field compiler failure. No private counters or native wake versions are seeded by these controls.

One new registry test uses exact selector `service::mod_channels::tests::q3_closed_receiver_preserves_passive_drain_without_an_immediate_kick_loop`. It owns a concrete Tokio unbounded sender whose real receiver is closed. The same entry's passive-version signal fails enqueue; a later safety pass at the same clock/version must retry. Neither pass may add attention health, mark immediate dirty, or issue a wake kick. Its current capable election uses existing V090 direct registration/lease semantics; change only that election to explicit mode once the internal mode is implemented. This is a closed-receiver test, never a queue-overflow claim.

Preserved existing controls unchanged in substance:

- `service::mod_channels::tests::domain::each_channel_receives_only_its_bindings_deliveries`, including all `1/1 -> 2/2 -> 3/2` assertions.
- `service::mod_channels::tests::each_channel_receives_only_its_bindings_deliveries` fake counterpart.
- Existing `passive_invitation_drain_does_not_arm_attention_stall_history`, exact-owner/lease/grace and presentation health tests.

## Honest deferred seams

No helper-only final-send test was authored. `FakeStore.on_read` fires before `observe_fingerprint` makes its first channel judgment, so it cannot prove the actual unlock/relock race. A stale helper call without the old channel id would also lack the causal ownership evidence.

Proposed parent-approved seam: a per-registry `#[cfg(test)]` one-shot callback, held in a separate mutex, invoked immediately after `observe_fingerprint` drops its first state guard and immediately before it calls the final active helper. Carry the captured binding/channel and, after implementation, immutable mode into callback context. Take the callback out of its mutex before calling it. Test callback sends a bounded `observed` event and waits for a bounded `resume` event; the test registers the replacement while the old pass is paused, then resumes and joins. Assert replacement receives no old frame, fingerprint, health or pending bit. Avoid a global failpoint, sleeps, or a callback under the state lock. Both mode directions require explicit internal mode first. The same final fence should be checked on the passive branch without mislabeling a pre-observation read hook as the final gap.

Concurrent hint success/failure, pending-bit replacement and post-enqueue socket-write loss remain deferred: the new hint method/pending field is absent on the base. Do not manufacture these names to force compile RED. Once the actual neutral hint entry point exists, use the existing per-store one-shot callback to inject a second hint during store work; separately fail that exact captured work while replacing the channel. Recovery must retain only the exact channel/mode and OR the captured bit, preserving a newer concurrent bit. The closed-receiver control already supplies compiling observable evidence for one necessary recovery behavior.

DP5 duplicate/due comparison is not claimed by this scope. The new closed-receiver control freezes the clock and does no ledger pulls. The mixed-owner control has no ordinary offers. The explicit 60-second eligible retry/three-offer comparison belongs with real presentation controls; a passive hint must never be treated as absolute silence across changed eligibility.
