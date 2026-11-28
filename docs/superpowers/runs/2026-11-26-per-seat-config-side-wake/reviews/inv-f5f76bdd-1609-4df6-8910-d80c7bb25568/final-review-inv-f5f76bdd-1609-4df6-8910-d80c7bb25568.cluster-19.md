# Final review: cluster 19 (src-protocol), epic ht-z41e

Source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head c3e9ab0becd137adb68f23a1c9e1a87669d03168 (both verified with `git rev-parse --verify <sha>^{commit}`). The caller checkout was at the head SHA with a clean status during the review. Inputs: slices 000099 to 000106 (authority.rs, capabilities.rs, commands.rs, handoff.rs, mod.rs, results.rs, seat_config.rs, service.rs) and digest.md, read against the design doc docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-26-per-seat-config-side-wake-design.md (the spec ht-z41e references).

## Scope and method

The cluster is the wire and registry surface: the closed `REGISTRY` in `src/protocol/seat_config.rs`, the three commands `GetSeatConfig`, `SetSeatConfig` and `UnsetSeatConfig`, the result types `SeatConfigView`, `SeatConfigEntry` and `SeatConfigWrite`, the two capabilities `seat.config_v1` and `seat.auxiliary_session_v1`, the `PermitMutation` conversion, and the new `CallerClaim.auxiliary_session` field. The rest of the slices only add `auxiliary_session: false` to existing test fixtures.

I read every slice, the full `seat_config.rs` at head, and the parts of `commands.rs` that hold `accountable_claim`, `host_claim`, `validate` and `TryFrom<Command> for PermitMutation`. To trace producers and consumers of the protocol surface, I opened `src/cli/config_seat.rs`, `src/service/dispatch.rs` (`seat_config_write`), `src/store/mod.rs` (`seat_config_operator`) and `src/store/seat_config.rs` (`agent_write`). In the caller checkout I ran the read-only `scripts/check-wire-compat` (ok: 0 violations, 0 stale) and `scripts/check-harness-neutral` (ok: 0 violations). I did not build or run the Rust tests. The evidence logs under evidence/ht-z41e.* hold the implementers' runs.

## Registry against the spec

- Keys match the spec table exactly. `nudge_threshold` is an IntRange -1..9 with default 0, agent tier, and the `NudgeThresholdColumn` backing. `programmatic_wake` is `on|off` with default `off`, human tier, and `Table` backing. S1 (closed typed registry) holds: `key()` refuses an unknown name and lists every key, and `validate` matches exactly (`true`, `ON` and `1` are refused and the error names the valid set).
- Sizes: the `programmatic_wake` description is 103 bytes, so it meets the doc comment's "under 110". The spec's list line `<key> <values> default=<d> tier=<t> — <description>` comes to 155 bytes for that key, under the 160-byte budget. The human-tier refusal at full length (`seat config key `programmatic_wake` needs `ht human config seat set programmatic_wake on``) is 89 bytes, under 100.
- `IntRange` validation rejects `+1`, leading spaces, a bare `-` and overflow. It accepts leading zeros and `-0` and rewrites them to canonical text (`007` becomes `7`). That is harmless, because the value is stored canonically.

## Commands, tiers and permits

- `SetSeatConfig` and `UnsetSeatConfig` carry `seat?`, `key`, `value` (set only), `tier`, `operation` and `claim?`. `GetSeatConfig` carries `seat` and `key?`. That is the spec's shape. None of the new structs uses `deny_unknown_fields`, and the optional fields have `#[serde(default)]`. These are local-protocol commands, not peer-wire frames. `check-wire-compat` scans only `src/sync`, so no `// cap:` annotation applies, and gating is by the `seat.config_v1` probe in the CLI (`gate(client, ..)` in config_seat.rs).
- `PermitMutation::try_from` accepts a seat-config write only when it is agent tier and has a claim. Dispatch routes by tier first. A claimless agent write is refused with InvalidRequest, and the store's `agent_write` checks the same thing again, along with a tier mismatch, a subagent role, a foreign `seat` and a human-tier key. A human-tier write goes to `seat_config_operator` as an `OperatorActor` decision and ignores any claim. `accountable_claim` still returns an attached claim, so a subagent claim on the human path is refused early. That matches the brief ("Human tier with a subagent claim attached is refused early"). The CLI sends `claim: None` on the human path and `seat: None` with a claim on the agent path.
- Result types: `SeatConfigEntry` reports `is_default`, `tier`, `writer` and `updated_at`, which is what `get` and the audit-row display need. `SeatConfigWrite.changed` follows the spec's "effective value or set/unset state changed". Both are new `CommandResult` variants, returned only for the new commands.

## Auxiliary-session claim field

- `CallerClaim` keeps its existing `deny_unknown_fields`. The new field has `#[serde(default, skip_serializing_if = Not::not)]`, so a default claim's bytes are unchanged, which tests/protocol/capabilities.rs asserts. A `true` value would break an older daemon, so it is sent only under `seat.auxiliary_session_v1`. `src/cli/site_capability.rs` (`drop_unadvertised_auxiliary`) strips it, and `src/cli/mod.rs:3642` and `src/harness/bridge.rs:573` reset it on echoed and frozen claims. `CallerClaim` does not appear in `src/sync`, so the field never reaches the peer wire.
- Producers exist at head: the Codex decode (`src/harness/codex.rs:1273`) feeds the bridge, which sets the bit on the check-in claim (`bridge.rs:430,513`). The store persists it sticky (`src/store/bindings.rs:1483`), and queue selection reads it (`selection.rs`). That is the spec's "neutral field on the check-in / claim". The doc comment's mention of Codex in neutral core passes `check-harness-neutral`.

## Requirement baseline trace (cluster share)

- B2/S1 (generic closed key=value registry): delivered in this cluster (REGISTRY, `key`, `validate`, string-typed wire).
- B3/B6/B7 (read/write CLI, agent-writable nudge_threshold, human-only gated keys, reads open to all): this cluster supplies the wire half. Commands are tier-tagged, `GetSeatConfig` needs no claim, and the permit conversion admits only agent-tier writes. The CLI producer (config_seat.rs) and the daemon consumers (dispatch, store) exist at head and enforce the tiers. The end-to-end trace belongs to the CLI, service and store cluster reviewers and the seam reviewer.
- B4 (list with descriptions): the registry carries description, values label, default and tier. Rendering is in the CLI cluster.
- B8 (root routing beside /side): this cluster supplies the `auxiliary_session` claim field and its capability. Routing and receipt belong to the harness, store and scheduler clusters (seam).
- B1, B5, B9, B10: outside this cluster's files, so the seam reviewer and the other clusters handle them.

## Findings

- nit: `src/protocol/commands.rs` puts `use crate::protocol::seat_config::ConfigTier;` in the middle of the file, between `enum PermitMutation` and its `TryFrom` impl. The struct fields in the same file spell out `crate::protocol::seat_config::ConfigTier` in full. Moving the import to the top would match the file's style.
- nit: the `ConfigKey.description` doc comment in `src/protocol/seat_config.rs` says "under 110 bytes", but `names_are_unique_defaults_validate_and_descriptions_fit_a_list_line` asserts `< 130`. At 130 bytes, a future key's list line could exceed the spec's 160-byte budget, so the test does not enforce either stated bound.

## Must fix before landing (cluster view)

- none

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- No unit test exercises the `TryFrom<Command> for PermitMutation` guards directly (an agent-tier write without a claim, or a human-tier write with a claim). They are reached only through the service routing tests (tests/service/seat_config.rs), and the store repeats the claim and tier checks.
- I ran no Rust tests for this review; I checked only `scripts/check-wire-compat` and `scripts/check-harness-neutral` at head.

## Deferred OK
- `use crate::protocol::seat_config::ConfigTier;` sits mid-file in src/protocol/commands.rs; moving it to the top is style only.
- The registry description test bound (< 130) is looser than the documented 110 bytes and the spec's 160-byte list line; tightening it to 110 would enforce the stated budget.
