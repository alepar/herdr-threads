# Phase 6 sweep: Accepted presentation lease diagnosis

2026-10-12. Read-only source/fixture/contract investigation using the current systematic-debugging skill. Candidate identity supplied by parent: HEAD d8427582, tree 53d0c453. No Git re-attestation, Cargo, test runtime, process control, tracker, Herdr, delegation, source edit or patch occurred. This report is the only write. Parent owns reproduction and the single unspent phase-6 sweep-fix pass.

## Observed failure

Actual run `/private/tmp/ht-final-C-d8427582/nextest-run.log:149–168` records `daemon::transport::watch_connection::tests::presentation_owner_accepted_wire_carries_the_registered_lease` failing at `tests/daemon/watch_connection.rs:614`: actual `Accepted { presentation_lease: None, attention_version: 7 }`; expected the known `Some("0123456789abcdef0123456789abcdef")` and the same attention version. This is actual baseline evidence on the parent-attributed candidate, not a conjectured pre-existing or flaky failure. No independent reproduction was run by this agent.

## Caller and data flow

1. `tests/daemon/watch_connection.rs:148–161` defines `watch_request` with `presentation_v2: false`. This is intentionally the historical request helper, used throughout the transport fixture.
2. The failing test (`604–613`) populates `WatchFake.presentation_lease` with a known token but calls that helper without changing the request mode. `WatchFake::accepting(7)` ignores request admission semantics, returns version 7, stores the request/sink, and its `watch_presentation_lease` returns the injected token regardless of channel/mode (`43–126`). The fake therefore permits an impossible real-registry combination: legacy request plus an available presentation token.
3. `start` passes a clone of this unchanged request directly into the real `serve` transport on a socket pair (`187–205`). Matching fake instance/boot clear the transport identity gates. The fake accepts registration, so attention version 7 is unchanged.
4. `src/daemon/transport/watch_connection.rs:160–178` builds Accepted using `advertised_presentation_lease(request.watch.presentation_v2 && all_four_advertised, handler.watch_presentation_lease(channel))`. All four advertised capabilities are present in `src/protocol/capabilities.rs:105–111`, but request opt-in is false. Thus the helper deterministically selects None. No timer, cache, registry race or changed attention version is needed to explain the outcome.
5. The test's expected value checks only advertisement of `DELIVERY_PRESENTATION_V2`, then invokes the same low-level helper with that true flag (`611–613`). It omits the decisive request election and the other capability prerequisites. The source/test disagreement is already fully explained before serialization; `WatchAccepted` then omits None by serde policy (`src/protocol/watch.rs:291–294`).

Real service behavior independently confirms the distinction. `src/service/dispatch.rs:1034–1050` selects Legacy for false and PresentationV2 for true, with separate canonical store views. `src/service/mod_channels.rs:861–865` mints a lease only for PresentationV2; `699–708` returns one only for the exact live PresentationV2 channel. `DomainService::watch_presentation_lease` simply delegates (`dispatch.rs:1090–1092`). The production CLI's `probe_mod_watch` (`src/cli/watch.rs:358–370`) requires mod-watch support plus all three presentation/cursor prerequisites before setting election true; `open_watch` carries that elected value into the request.

## Selected contract and working references

The authoritative Q3 design is `docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/resume-v090-q3-channel-design.md`, especially Requirements R1/R2, Selected negotiation, and Service and registry split. It explicitly requires historical claim-only/false requests to omit leases and prohibits advertisement or generated tokens from silently promoting legacy channels. Explicit negotiated true yields the mode-bound Accepted lease. The current normative `TRUST-POLICY.md:1872–1876` states the same per-channel election and capable-Live-owner condition. The Q3 specification and quality review files in this WS contain their later scoped PASS appendices; source and normative contract, not those report labels, decide this diagnosis.

Working references read in full for the relevant fixture paths:

- `tests/service/mod_channels.rs:1475–1480`: real DomainService legacy registration is live but returns no lease.
- `tests/service/mod_channels.rs:1539–1655`: mixed same-seat modes positively admit explicitly capable owners under their own leases, refuse stale/borrowed authority, and preserve no-lease legacy/reconnect behavior.
- `tests/cli/watch.rs:1403–1514`: raw peer fixture returns even an unsolicited valid lease, yet missing capability legs preserve historical wire/consumer; the all-capabilities control requires request true and use of the exact lease.
- `tests/protocol/watch.rs`, `accepted_presentation_lease_survives_new_decoder_and_old_bytes_stay_omitted`: additive lease round trip and old bytes remain correct.
- `tests/daemon/watch_connection.rs:216–233`: existing general Accepted control expects None from the default legacy request, but its fake has no lease, so it does not itself prove that transport suppresses an unsolicited fake lease.

The actual final focused log already records PASS for the protocol round-trip control (line 298), real legacy registration (522), mixed modes/replacement (561), and CLI all-capability/absent-capability controls (112–123). The general transport Accepted control is absent from that run selection; do not claim it passed there.

## Testable hypothesis and minimum correction scope

Hypothesis: changing only the failing test's request election to true will make its known injected lease survive the production transport, because all four capabilities are advertised. Leaving election false with exactly the same injected fake lease must continue to yield None and byte-exact historical Accepted output. Parent should confirm this counterfactual on the actual baseline/corrected candidate.

Recommended source scope: **only `tests/daemon/watch_connection.rs`**, local to this test (or a neighboring focused transport control). Preserve the shared helper's false default and all production gates. Set true explicitly for the positive presentation-owner request; require the full capability prerequisite set, not merely the generic presentation capability. Prefer a direct known-token expected value once those prerequisites are positively asserted, instead of deriving expected behavior through the same production helper. Exercise a real `serve` false-request counterpart with the fake token still populated and check the resulting serialized Accepted omits the lease. Existing helper(false)/helper(true) assertions alone do not test request-to-wire negotiation. Keep client drop and task join for each served connection.

Do not fix by removing the request gate, automatically promoting all helper requests, changing the expected positive value to None, weakening the real legacy tests, changing capabilities, or altering service/registry admission. Those changes either violate selected Q3 or erase the intended positive transport coverage. No production bug is demonstrated by this failure.

## Exact parent-owned selectors

Use nextest equality selectors against binary id `herdr-threads` (all are library tests). Primary actual-baseline selector:

```text
binary(=herdr-threads) & test(=daemon::transport::watch_connection::tests::presentation_owner_accepted_wire_carries_the_registered_lease)
```

Minimal useful comparison selection, six current tests, with the same primary selector before and after the narrow test correction:

```text
binary(=herdr-threads) & (test(=daemon::transport::watch_connection::tests::presentation_owner_accepted_wire_carries_the_registered_lease) | test(=daemon::transport::watch_connection::tests::accepted_reply_carries_attention_version_and_the_claim) | test(=protocol::watch::tests::accepted_presentation_lease_survives_new_decoder_and_old_bytes_stay_omitted) | test(=service::mod_channels::tests::domain::q3_legacy_claim_only_registration_has_no_presentation_lease) | test(=service::mod_channels::tests::domain::q3_mixed_same_seat_modes_and_replacement_cannot_borrow_old_leases) | test(=cli::watch::tests::q3_all_watch_mode_capabilities_send_true_and_use_the_accepted_lease))
```

If the false-request fake-token transport countercontrol is split into a new test, add its exact selected name to the corrected selection rather than assume the six-test selector covers it. The existing three missing-prerequisite CLI controls are additional established controls if parent elects to include them: `cli::watch::tests::q3_missing_watch_mode_capability_keeps_old_wire_and_consumer`, `cli::watch::tests::q3_missing_generic_presentation_capability_keeps_old_wire_and_consumer`, and `cli::watch::tests::q3_missing_binding_cursors_capability_keeps_old_wire_and_consumer`.

This investigation claims a source-proven admission/oracle mismatch and records existing actual run evidence. It does not claim a new passing runtime result, a pre-Q3 baseline result, completion of the sweep-fix pass, or clearance of the other two sweep failures.
