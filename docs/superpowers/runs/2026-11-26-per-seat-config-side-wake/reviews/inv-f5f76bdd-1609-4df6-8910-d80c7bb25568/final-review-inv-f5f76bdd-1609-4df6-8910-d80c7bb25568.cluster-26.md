# Final review: cluster 26 (tests-daemon)

Source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head c3e9ab0becd137adb68f23a1c9e1a87669d03168 (both verified with `git rev-parse --verify <sha>^{commit}`). Review source bound through `dispatch-text.mjs bind-review-source` (exit 0). Caller checkout observed at HEAD c3e9ab0be with a clean status; it was not switched or modified.

## Scope

Four slices (000148-000151), one hunk each, in:

- tests/daemon/launch_gate_guards.rs (`claim`)
- tests/daemon/multi_host.rs (`Rig::claim`-style helper)
- tests/daemon/self_retire.rs (`claim`)
- tests/daemon/watch_connection.rs (`claim`)

Each adds `auxiliary_session: false,` to a `CallerClaim` struct literal.

## Analysis

- `CallerClaim.auxiliary_session` (src/protocol/authority.rs at head) is a new `bool` field with `#[serde(default, skip_serializing_if = "std::ops::Not::not")]`, sent only to a daemon advertising `seat.auxiliary_session_v1`. These test literals need the field to compile, and `false` is the value an ordinary top-level or subagent claim carries in production (hook.rs, mod.rs, bridge.rs default it to `false` unless the Codex decoder reports a null `transcript_path`).
- None of these four suites exercises queue root selection or `/side` routing, so `false` keeps each suite's existing behavior. `false` is also skipped on serialization, so wire bytes in these tests stay the same as at base.
- The `true` path is tested outside this cluster: the daemon check-in round trip through `tests/fixtures/wake_intent.rs` (the payload's evidence is carried on the claim into the real check-in), store persistence in `tests/store/cooperative_checkin.rs`, wire decode and capability gating in `tests/protocol/capabilities.rs`, and Codex decode in `tests/harness/codex.rs`. Whether those compose into the B8 delivery path is for the seam reviewer.

## Requirement baseline trace

- The root spec (`docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-26-per-seat-config-side-wake-design.md`) has a Requirement baseline (B1-B10, S1). This cluster has no production code and delivers no baseline row alone. It only keeps four daemon suites compiling after the claim shape changed. Tracing B1-B10 from production entry points (per-seat config CLI, the nudge-threshold key, Codex `/side` queue routing, native wake deferral on a side chat) leaves this cluster and is for the seam reviewer and the owning clusters. This cluster neither blocks nor weakens any row.

## Findings

- none

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- none

## Deferred OK
- none
