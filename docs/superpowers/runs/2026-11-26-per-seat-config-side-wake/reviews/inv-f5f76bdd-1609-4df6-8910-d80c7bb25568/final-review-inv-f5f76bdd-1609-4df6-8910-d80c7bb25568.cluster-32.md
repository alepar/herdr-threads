# Final review: cluster 32 (tests-protocol), epic ht-z41e

Source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head c3e9ab0becd137adb68f23a1c9e1a87669d03168 (both verified with `git rev-parse --verify`). Caller checkout observed at HEAD c3e9ab0be with a clean `git status --short`, before and after reading.

Cluster files: tests/protocol/capabilities.rs, tests/protocol/delivery.rs, tests/protocol/priority.rs, tests/protocol/watch.rs (slices 000178-000181).

## What the cluster changes

- delivery.rs, priority.rs, watch.rs and most of capabilities.rs: mechanical `auxiliary_session: false` additions to `CallerClaim` literals. `CallerClaim.auxiliary_session` at head is `#[serde(default, skip_serializing_if = "std::ops::Not::not")]` (src/protocol/authority.rs), so with this value the bytes of existing claims stay the same. These edits are correct and carry no behavior.
- `capability_constants_are_stable` and `bare_daemon_advertises_legacy_capabilities_without_v2_recorder`: both new constants are pinned (`seat.config_v1`, `seat.auxiliary_session_v1`), and the ordered lists match `ADVERTISED` at head: `SEAT_AUXILIARY_SESSION_V1` comes after `SEAT_CONTINUITY_KEY_V1` and `SEAT_CONFIG_V1` comes after `PEER_HANDOFF_STAGE_V1` (checked against src/protocol/capabilities.rs).
- `every_advertised_capability_has_a_handler` gains two arms:
  - `SEAT_CONFIG_V1` decodes minimal JSON for `get_seat_config`, `set_seat_config` and `unset_seat_config` (tier `human`), validates them, round-trips the writes and asserts that ControlService forwards them to the domain (`NotFound` from the `NoDomain` stub, not `InvalidRequest` or an unknown-command error).
  - `SEAT_AUXILIARY_SESSION_V1` decodes a `check_in` claim with `auxiliary_session: true`, round-trips it, checks that a claim without the field decodes as false and serializes without the key, and asserts that the daemon refusal is not `InvalidRequest`.

## Spec conformance

- The design (docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-26-per-seat-config-side-wake-design.md, "Commands") requires the three commands to be gated by `seat.config_v1` and requires new fields to default. The probe covers the wire decode, the advertisement and the routing for both. `GetSeatConfig`, `SetSeatConfig` and `UnsetSeatConfig` are new types. Their optional fields (`key`, `seat`, `claim`) carry `#[serde(default)]`, and the probe's minimal payloads, which have no claim and no seat, decode.
- Section 2 requires the claim to carry auxiliary-session evidence that an older daemon never sees and that a default claim never serializes. The `skip_serializing_if` assertion covers the second half. The first half (the CLI strips the field when the capability is missing) lives in src/cli/site_capability.rs, which is outside this cluster.

## Requirement baseline tracing

The baseline rows B1-B10 (CLI tiers, nudge_threshold on the non-human path, /side routing, the native side-composer defer) are delivered by production code outside this cluster: src/cli/config_seat.rs, src/store/seat_config.rs, src/service/dispatch.rs, src/store/wake_intents/selection.rs and src/host/native.rs. This cluster only pins the wire and capability contract those rows depend on. Tracing each row from its production entry point is the seam reviewer's job. Inside this cluster, nothing contradicts any row. I checked one named risk outside the cluster: `seat_config_write` in src/service/dispatch.rs:1628 routes the agent tier through `PermitMutation` (claim required) and the human tier through the elected-owner operator check. The store refuses a human-tier write with no seat, which explains why the probe's seatless human payload passes `validate()`. That behavior is correct.

## Findings

- tests/protocol/capabilities.rs, `SEAT_CONFIG_V1` arm: the comment says the daemon "routes the read (an unknown seat is NotFound)". The handler is `ControlService` over `NoDomain`, which answers `NotFound` for every command, so the assertion proves only that the read reaches the domain, not unknown-seat semantics. The comment is misleading, but the test is sound for what it actually checks (routing). Real unknown-seat handling is tested in tests/service/seat_config.rs.
- tests/protocol/capabilities.rs, `SEAT_CONFIG_V1` arm: only `tier: "human"` write payloads are probed at the capability boundary. The agent-tier wire shape (`tier: "agent"` with a `claim`, which `PermitMutation::try_from` accepts) is not decoded or round-tripped here. It is covered in tests/cli/config_seat.rs (lines 526 and 573) and tests/service/seat_config.rs, so this is a gap in this probe, not in the suite.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- No test run of this cluster at head c3e9ab0be is on record. The last recorded run is task 11's focused `protocol::capabilities` run at its own task head, and the full-suite sweep is deferred to the caller (progress.md Recovery record). The capability-list assertions break if a later merge reorders `ADVERTISED`, so they need the caller's sweep at head.
- The `SEAT_CONFIG_V1` capability probe does not decode or round-trip the agent-tier `set_seat_config`/`unset_seat_config` payload with a claim. Other targets cover it.

## Deferred OK
- The `SEAT_CONFIG_V1` probe comment in tests/protocol/capabilities.rs says "an unknown seat is NotFound", but the `NoDomain` stub answers NotFound for everything. It should be reworded to say the command reaches the domain.
