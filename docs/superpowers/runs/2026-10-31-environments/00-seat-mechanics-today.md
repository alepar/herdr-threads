# How pane-bound seat identity works today (investigation, 2026-10-31)

Read-only survey of the herdr-threads tree at f4ddaeffa. File:line refs are approximate entry points.

## Schema (migrations/node/0001_node_baseline.sql)
- `seats` (:414-457): state resolved/unresolved/retired/remote; `target_id` nullable; `host_id REFERENCES herdr_hosts`;
  `CHECK((is_local=1) = (host_id IS NOT NULL))`; no name column. STRICT tables, `schema::verify_baseline`.
- `occupant_bindings` (:1311-1344): `target_id NOT NULL`, `host_boot NOT NULL`, host_epoch, terminal_id, incarnation;
  `observation_provenance` has no CHECK; liveness columns (liveness, disconnected_at, liveness_pid, liveness_start).
- Host-keyed: `herdr_hosts` (:54, keyed by endpoint), `observed_targets`/`snapshot_targets`, `allocation_decisions`
  (host_id/target_id/host_boot NOT NULL), `recovery_holds`, `restored_pane_watch`, `remote_seat_status` (ws/tab/pane labels).
  In-memory `HostIndex` (src/store/hosts.rs:147-189).
- Only pane-less seat kind today: remote seats (is_local=0). Person seats are pane seats with harness='human'.
  Graph system identity is a `service_authors` row, not a seat.

## Caller claim
- CLI: `run()` reads HERDR_PANE_ID (src/cli/mod.rs:311); `derive_caller` (:3538) → `derive_selection` (:3815) →
  `located_pane_seat` (:3766, Seats{target: pane} query; moved-pane fallback via Herdr current_pane). Agent callers:
  `caller::derive` (src/cli/caller.rs) + `src/harness/caller_env.rs:277-338`. `CooperativeSelection{seat, target: HostTargetId, harness, role}`
  (src/cli/commands.rs:378). `IntentScope::Cooperative{instance, host, seat}`.
- Hooks: `HookEnv::from_process` (src/cli/hook.rs:255); `connect_pane` (:1783, needs HERDR_ENV=1 and host); `check_in` (:1890)
  → `launch::resolve_seat(host, target)`; `report_binding_event` (:3855); `foreign_session` (:4390) no-ops without pane.
- `CallerClaim` (src/protocol/authority.rs:8-29): host: HostLocator (absolute canonical path, src/protocol/ids.rs:314),
  target: HostTargetId, seat, binding_generation, role, harness, native_session, execution, agent_id?, liveness?;
  `deny_unknown_fields` (local socket only).
- Daemon: `handle_with_output` (src/service/dispatch.rs:1523) resolves host locator, lazy attach (src/service/host_sessions.rs:577).
  Store `cooperative_claim` (src/store/seats.rs:4904) → `attached_id`; `cooperative_mapping` (:5131-5307) needs resolved seat,
  target match, generation, no hold, and `effective::effective_observation(host, target)` matching boot/epoch;
  `binding_evidence` (:4878) needs terminal + incarnation from a host observation.
- Creation: `ResolveSeat` → `attached_host` (dispatch.rs:1692) → `OrdinaryIdentity::resolve` (src/identity/repair.rs:254)
  → live `observe_current_target`.
- Many SQL predicates `b.target_id = claim.target` (receipts.rs:75, invitation_content.rs:285, attention.rs:1102).

## Provenance
- Constants in src/protocol/authority.rs:98-168 (cooperative_top_level, cooperative_mod_channel, cooperative_hook_*,
  cooperative_env_session, operator_human, cooperative_continuity, managed_launch, liveness_*); sets
  CARRIED_BINDING_PROVENANCES, AGENT_BINDING_PROVENANCES. Normative table TRUST-POLICY.md A3 (:378-400). Not on peer wire.

## Liveness
- `LivenessFingerprint{pid,start}` from OS process table (caller_env.rs:249-266, 341, 352); worker src/service/liveness.rs
  node-wide; `liveness_candidates` (bindings.rs:700-732) JOINs herdr_hosts.

## Discovery (src/cli/instance.rs)
- State dir: flag → HERDR_PLUGIN_STATE_DIR → default (must exist) → `herdr plugin list --json` subprocess. Fails with no Herdr and no dir.
- Host: flag → HERDR_SOCKET_PATH → default socket → `herdr status server --json` subprocess; None tolerated.
- Daemon runs store-only with zero hosts (src/app.rs:2040); MAX_ATTACHED_HOSTS=16; idle exit 10 min (src/daemon/idle_exit.rs).
  `operator_host` (dispatch.rs:385) needs any attached host for `seat retire` and orphan invite.

## Wake
- Per-host discovery: `wake_candidates(host)` (src/store/mod.rs:1275, 3198), BATCH_SEATS_SQL `s.host_id=?4` (src/store/wake.rs:1271).
  `wake_frontier` (src/store/lazy_delivery/wake.rs:68). `DeliveryPolicy` (src/harness/adapter.rs:485-497) mechanism PushChannel/HookPull,
  idle_wake Native/Channel. `Registry::has_idle_wake` (src/harness/registry.rs:1447). Native wake typing: src/host/native.rs.
- Sender `wake=no_wake` once per (sender, seat) (src/cli/wake_warning.rs, dispatch.rs:248-301). Overdue `wake=` from
  `overdue_wake` (src/store/schema.rs:1393). `unreceived` (wake.rs:1041). Scheduler src/scheduler/mod.rs.

## Peer wire
- `seat.created` {seat, kind: SeatKind{Agent,Human} (no serde(other)), display_name: Option (reserved, null)} (src/sync/facts/seat.rs:20).
- `seat_status` frame cap SEAT_STATUS_CAP (src/sync/frame.rs:52), `SeatStatusEntry{seat,presence,harness?,activity_age_ms?,workspace?,tab?,pane?}`.
- Remote addressing: `split_machine` (src/protocol/address.rs:791), `seat_resolution::resolve_one` (src/cli/seat_resolution.rs:538),
  `AddrResolve` cap addr.resolve.v1; home-node `RemoteResolver` (src/service/remote_resolver.rs) answers from attached hosts' live topology.

## Display / retire
- `seat list` (src/cli/human.rs:208-302) labels via `NativeCli::seat_labels` (Herdr api snapshot); participants src/cli/peer_locations.rs.
  `SeatSummary` (src/protocol/results.rs:1085) target: Option, no name.
- Pane close → BeginRetirement (src/store/seats.rs:2950), src/store/control/retirement.rs; operator_retire (seats.rs:4759).
