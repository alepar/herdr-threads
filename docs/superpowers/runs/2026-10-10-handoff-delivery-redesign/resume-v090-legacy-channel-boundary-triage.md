# V090 legacy channel boundary triage

Read-only source investigation, 2026-10-11. Inspected the composed V090 worktree at `.worktrees/upstream-composition-v090` under the handoff redesign root. The parent supplied frozen HEAD `9a…` / tree `5158…`; this investigation did not run Git to attest them. Current test files may concurrently receive their assigned fixture-only edits. No Cargo, application runtime, SQL execution, Herdr, source/index/Git mutation, or subagent was used. This report is the only write.

**Finding: the existing failure is a real production/fake-store behavior divergence, not repairable by adding missing V2 binding or version prerequisites while preserving its legacy fingerprint oracle.** Whether all of that historical oracle is still intended for a capable channel is a separate contract decision. Do not silently resolve that decision through fixture weakening.

## Causal chain

The supplied actual run `/private/tmp/ht-C-v090-final-composed-657/nextest-run.log:1630` reports `service::mod_channels::tests::domain::each_channel_receives_only_its_bindings_deliveries` failing at `tests/service/mod_channels.rs:1413`: actual `[]`, expected `[Attention { version: 5 }]`. The similarly named fake-store test passes in the same log.

1. `ModChannelRegistry::pass` (`src/service/mod_channels.rs:364`) unconditionally selects `presentation_seat_view`, then calls `presentation_refresh` before observing the fingerprint.
2. SQLite's override (`src/store/mod.rs:1245`) uses `seats::presentation_seat_view`; `mod_seat_view_mode(..., false)` (`src/store/seats.rs:4978`) reads canonical seat/binding and shared attention version but returns zero pending receipts, ordinary unread, Lazy unread, and other pending fields. It also returns zero invitation version.
3. Initial observation uses `ModFingerprint::has_pending` (`src/ports.rs:2723`), which deliberately ignores attention version and generation. Thus even `wake_work.attention_version=5` cannot make that initial fingerprint pending.
4. Refresh is correctly scoped here. It reconstructs its claim from the canonical binding's execution/session, then checks `binding_scope` (`src/store/invitation_content.rs:2779`, `:264`). The fixture has resolved seat, matching target/generation, connected top-level binding and no recovery hold. An incoming sibling request's stale execution is an independent fixture issue for later explicit V2 Pull; refresh does not use that incoming execution. No pre-existing private consumer row or relevance proof is needed: refresh initializes its own state.
5. The fixture's `deliver()` publishes ordinary messages through `send_manifests` and `prepared_recipients`, with no Lazy recipient or invitation. `advance_ordinary_publications` (`src/store/invitation_content.rs:658`) only increments passive generation/mod version for an exact addressed Lazy recipient plus a published `delivery_mode='lazy'` message. Therefore successful refresh legitimately returns version zero here; it cannot restore the initial pending fingerprint.
6. The fixture's final operation advances only A's binding cursor. Legacy `mod_binding_view` measures `binding_unread_stats`, so only A's fingerprint changes. The new zeroed view never observes that cursor; invitation/Lazy publication refresh does not observe it either. Correcting the earlier publication's wake version cannot restore this final oracle.
7. `ModStoreReads::presentation_seat_view` defaults to `mod_binding_view`, and `presentation_refresh` defaults to `Ok(None)` (`src/ports.rs:2611-2641`). The fake implements the old view and inherits these defaults. Its passing test exercises legacy semantics that the actual SQLite pass no longer supplies.

The separate ordinary-send worker test performs an empty initial pass and then manually moves version 5 to 6. Its changed shared version can trigger the zeroed fingerprint after initialization. That does not cover initial pending state or unchanged-version binding-only transitions.

## Production compatibility boundary

The CLI explicitly retains historical behavior when presentation capability is absent or Accepted has no valid lease (`src/cli/watch.rs:395-455`). However `WatchRequest` contains only `claim` (`src/protocol/watch.rs:261`), and registry `Entry` has no client presentation-mode field (`src/service/mod_channels.rs:75`). Every channel gets a presentation lease. Capability discovery chooses the CLI loop, but the daemon's worker cannot distinguish that choice. Consequently a historical channel on this daemon also receives the new zeroed-fingerprint/refresh behavior.

This is a confirmed fallback semantics mismatch. It is **not yet a demonstrated lost initial body**: legacy `watch_loop` drains immediately on Accepted (`src/cli/watch.rs:505`), before waiting for Attention. The original unit failure alone must not be described as lost end-to-end delivery. A cursor-only notification can also be redundant for already-emitted ordinary content. A runtime control should establish the desired observable boundary before any production fix.

Adding fake Lazy rows, directly editing private `mod_version`, forcing notification, or removing the final A-only assertion would conceal the mismatch. An unconditional switch back to `mod_binding_view` would restore the old oracle while reintroducing the legacy digest walk into the bounded capable path. Neither is an appropriate blind fix.

## Concrete boundary proposal

If byte-exact legacy fallback is required, select the worker path per channel rather than globally. A negotiated presentation opt-in, absent by default for old request bytes and sent only after the existing V2 capability probe, can be carried through registration into the entry. Legacy channels retain the historical fingerprint/read and output behavior; opted-in channels retain the bounded presentation view and private passive refresh. Keep admission A2-canonical and preserve exact Live lease/binding/generation/session ownership. This is a proposed design, not an implemented or approved wire change.

An optional wire opt-in requires a compatibility control: old request serialization stays byte-identical when absent; new clients never send it to capability-absent daemons with strict unknown-field decoding. Merely treating lease presence as opt-in is insufficient because leases are currently issued to all channels. Merely switching mode after the first V2 pull leaves registration/pre-pull work on the legacy digest path and needs a separate boundedness argument.

No new global output version, publication audience fanout, wake_work write, native wake or health reset should be introduced to make the test green. Passive Lazy/ordinary notice behavior must retain its selected lane's contract. Existing A7/A8 restrictions and V2 binding-owned state remain intact. Dinv content must continue through its bounded private source rather than old digest fields. Read TRUST-POLICY A2/A7/A8 and the presentation amendments for this analysis; no admission or wake changes were made.

## Parent-owned validation before production changes

The existing actual failure is already RED. Useful exact selectors, not a full suite:

```text
test(=service::mod_channels::tests::domain::each_channel_receives_only_its_bindings_deliveries)
test(=service::mod_channels::tests::each_channel_receives_only_its_bindings_deliveries)
test(=service::mod_channels::tests::domain::the_worker_pass_pushes_attention_for_a_real_send_and_closes_on_rebind)
test(=cli::watch::tests::presentation_watch_requires_both_capability_and_valid_accepted_lease)
test(=service::delivery_pull::tests::presentation_registry_pass_drives_bounded_passive_refresh_before_full_pull)
test(=service::delivery_pull::tests::presentation_quiet_channel_observes_new_lazy_ordinary_without_wake)
test(=service::mod_channels::tests::passive_invitation_drain_does_not_arm_attention_stall_history)
```

Check the CLI module's exact inventory name before using its selector. Existing tests establish negotiation fallback output and capable passive behavior, but the fallback test uses a scripted transport, not the real registry/store.

The most meaningful new pre-fix control is a real-SQLite sibling-channel test that explicitly elects the historical lane, preserves the full original `1/1 -> 2/2 -> 3/2` frame oracle, and verifies binding A's canonical cursor movement does not alter B. A companion elected-capable control must use the same store and verify a bounded private invitation/Lazy change requests one drain without native/health changes and without legacy digest access. The fake should either implement the same explicit mode split or refuse any forbidden legacy read in that capable control; the current default cannot prove this property. A third actual legacy transport control should drain on Accepted, then exercise a canonical producer after initial drain and compare exact old output rather than equating registry Attention with model output.

The parent alone runs Cargo and manages scoped leak checks. No execution claim is made for these proposed controls. Findings were cross-checked directly with `triage_v090_mod_integration`, which independently reached the same zero-fingerprint/ordinary-refresh conclusion.
