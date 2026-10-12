# Q3 wire/registry causal corrections handoff

2026-10-11. Prepared against parent-recorded candidate `9ef83ba7`; parent then explicitly unfroze the two owned test files and authorized the cfg(test)-only seam in `src/service/mod_channels.rs`. Applied only those three files after checking each still matched its captured before snapshot. No Cargo, index/Git, Herdr, runtime tests or nested agents. Parent alone attributes actual RED/PASS. Patch artifact: `resume-v090-q3-wire-causal-corrections.patch`, with normal `a/` and `b/` repo-relative paths.

## Quiescent release

Scoped `rustfmt --edition 2024 --config skip_children=true` and its `--check` passed. Edition is explicit because the isolated patch staging tree has no Cargo.toml and otherwise rustfmt rejects the existing Rust 2024 let chains.

- `tests/cli/watch.rs`: SHA-256 `23e90adf66944fd1150fbd1578857c7d7db5dcf2917e5a5218d67ea601c79824`.
- `tests/service/mod_channels.rs`: SHA-256 `6779114e4a80692c21e804dcd7df6aba92bc9305ad94801549856a1e352c5144`.
- `src/service/mod_channels.rs`: SHA-256 `90c16a53464d59c4cfba6a573c00855417d6539ac67e5d6dfc787b5194da1372`.

All files are released to the parent/registry author. The source hash describes this release, before their production edits.

## CQ2: bounded fixture lifecycle

`Server::start` delegates to `start_raw`, preserving all original scripts and raw capture. Its listener and framed receiver use nonblocking I/O with one absolute existing HANDSHAKE_TIMEOUT deadline; partial prefix/body progress cannot restart that deadline. A bounded Condvar wait (at most 10 ms, clipped to the remaining deadline) paces readiness polling and wakes immediately on cancellation. No socket bytes are normalized before the raw control records them. A test-only 1 MiB cap bounds malformed declared request allocation.

The cancellation object owns a clone of the currently accepted socket. Cleanup sets cancellation, shuts down both directions, wakes the Condvar, joins the helper thread and removes the temporary directory. Scripted read/write timeouts use the existing handshake budget. `finish` reports thread/cleanup errors only after joining and removal; Drop does the same cleanup and suppresses a second panic during client assertion unwind. Cancellation intentionally permits completion when negotiation returns before connecting; the actual negotiation assertion/capture still reports that regression, instead of hanging.

New lifecycle selectors:

- `cli::watch::tests::q3_raw_watch_server_finishes_when_the_client_never_connects`.
- `cli::watch::tests::q3_raw_watch_server_drop_joins_and_removes_socket_on_unwind`.

Existing exact raw-byte and no-connection controls remain unchanged in substance. Existing tests intentionally exercising V2 now advertise all prerequisites, including `mod.watch_presentation_v2`; capability-omission controls keep their omission.

## CQ4: exact canonical sibling claim

The Q3 request helper loads generation, execution and target from the exact connected fixture session's canonical binding row. The mixed-mode test asserts distinct binding ids, distinct executions and exactly two connected rows. Before making the sibling Legacy, it registers that sibling capable and positively admits a Pull under its own lease. It separately refuses an explicitly stale execution and then replaces the sibling with Legacy before checking the borrowed original-owner token. The negative can therefore no longer pass because the sibling accidentally carries the original execution.

The shared old request helper explicitly defaults the new wire boolean false, retaining the original legacy frame oracle. The old real-store presentation-owner authority test explicitly elects true using the corrected helper. These fixture rows and synthetic cooperative fingerprints prove canonical admission behavior, not real OS harness execution.

## CQ5: worker versus native wake evidence

The real closed-receiver control installs a dedicated `set_worker_wake` atomic counter before registration, verifies/resets the one initial registration event and asserts zero further worker kicks after each failed enqueue. Its existing dirty-bit, native wake-kick, attempt-count and no-health checks remain. This is an actual closed mpsc receiver, not queue overflow.

## Actual final-gap seam and controls

The only production-path source additions are cfg(test)-guarded: a `BeforeFinalPush` alias, per-registry mutex/optional one-shot callback, constructor initialization and callback invocation. Signature: `FnOnce(&BindingId, ModChannelId, bool) + Send`; bool identifies the active rather than passive branch. Invocation occurs after `observe_fingerprint`'s first registry-state guard is dropped and before the actual final push. The callback is taken out of its own mutex before executing.

The test callback checks the captured binding/channel and active flag, uses immediate `try_lock` to prove the first state guard was released, verifies the old entry already holds its observation, then performs a real same-binding `register`. This synchronous callback is the deterministic interleaving: it needs no sleeps, auxiliary model, indefinite barrier or stale helper call. A Weak registry reference avoids an ownership cycle if the callback is never reached. The resumed production path must leave the new channel's frames, health, fingerprint and passive-version observation empty. The old sink must receive the real Replaced close.

Exact selectors:

- `service::mod_channels::tests::q3_replacement_after_observation_cannot_receive_the_old_active_push`: on the unfenced base, meaningful RED should show the new sink received the old Attention and its health was armed. Parent must record actual outcome before implementing the final active fence.
- `service::mod_channels::tests::q3_replacement_after_observation_cannot_receive_the_old_passive_push`: exercises the same actual interleaving on the already fenced passive branch, expected positive control.

Current direct registry fixture has no typed mode field. The controls presently prove exact channel replacement, not both mode directions. Once the mode is introduced, adapt election to exercise Legacy -> PresentationV2 and PresentationV2 -> Legacy without weakening the captured-channel assertion or moving the callback. Registry production author has the exact callback signature/location and is holding the final active fence until parent runs the causal RED.

Pending hint take/restore/replacement, post-enqueue socket-write loss, eligible DP5 retry comparisons and model-side passive/idle effects remain distinct implementation gates. This patch does not claim those missing controls or replace them with the new seam.

## Follow-up: explicit modes and pending hints

After registry mode source was released, parent authorized only `tests/service/mod_channels.rs` adaptation. Quiescent SHA-256 is now `ecee6b5f0aabb950b197b89a127febae8dc16ec5a489c849316555324e7f6a31`; scoped Rust 2024 formatting/check passed. No production file, CLI test, Cargo, runtime, Git/index or Herdr changes in this follow-up.

All six ModChannelRegistration constructors specify mode. The general fixture's register/try_register defaults to Legacy; a separate register_mode helper elects PresentationV2 only for lease, presentation-health and passive closed-receiver controls. The domain request helper remains false, preserving the original `1/1 -> 2/2 -> 3/2` real-store oracle and fake counterpart. The existing real-store lease-authority test already explicitly chooses true. Read-only verification confirmed both intended CLI V2 fixtures advertise cursor and new Watch-mode capabilities.

The original final-gap active selector now exercises Legacy -> PresentationV2. New exact selector `service::mod_channels::tests::q3_capable_to_legacy_replacement_cannot_receive_the_old_active_push` exercises PresentationV2 -> Legacy. Both use the actual callback after observation and real register replacement; both await parent-recorded meaningful RED before the final fence fix. The original passive selector exercises PresentationV2 -> Legacy, while `service::mod_channels::tests::q3_legacy_to_capable_replacement_cannot_receive_the_old_passive_push` supplies the other direction. Each verifies replacement mode and that no pending bit transferred.

Five new pending-hint controls (prefix all with `service::mod_channels::tests::`):

- `q3_passive_hints_coalesce_without_health_or_crossing_legacy_and_seat_scope`: two explicit hints yield one passive frame, no health changes or native wake kick, no sibling Legacy/foreign-seat pending bit or frame. The registered worker callback records exactly one call per explicit hint and none for consumption; the bounded bit/frame, not callback count, is what coalesces. Unknown-seat hint causes zero callbacks. Fixed-clock repeated safety pass remains quiet.
- `q3_second_hint_during_store_work_survives_the_first_successful_pass`: callback runs inside the registry's actual store read after the first hint is taken, issues the second hint, and verifies the first success does not clear it. Two passes emit two drains, the third is quiet; exact worker count two and no attention health/native kick.
- `q3_failed_hint_work_restores_the_same_exact_owner`: a captured hint survives StoreBusy and the next successful pass drains it without a recovery-induced worker kick.
- `q3_failed_store_work_keeps_a_newer_hint_without_a_captured_hint`: an initially unhinted pass receives a new hint during its read, then fails; failure cannot clear that newer bit. The subsequent normal pass drains it once.
- `q3_failed_hint_work_does_not_transfer_to_either_replacement_mode`: real register replacement occurs inside the read before the old StoreBusy returns; separate Legacy and PresentationV2 replacements retain clean pending, fingerprint, private-version and health state, and no old frame. The only worker callbacks are the original hint and actual registration.

These are registry unit boundaries with scripted store payloads, not canonical SQLite mutation classifications. The FakeStore now captures its requested failure before running the one-shot read callback, then returns that captured failure afterward; existing failure behavior without a callback is unchanged. This deliberately enables the old-read failure/replacement interleaving. No new timers, sleeps or worker threads are used by the hint/race controls. Actual RED/PASS remains parent-owned. Post-enqueue socket-write loss, DP5 eligible retry comparison and model-side effects remain outstanding separate gates.
