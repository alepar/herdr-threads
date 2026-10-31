## Goals

### ht-kqrt (root)
An agent in a Herdr TUI pane, an agent in a plain or tmux process started with `HERDR_THREADS_SEAT_NAME=<name>`,
and an agent in a herdr-core terminal (`hc run` or a Herdr desktop-app pane) each get a durable seat with full
messaging: join, send, inbox, ACK, turn-hook and channel delivery, and cross-environment and cross-node addressing.
All three go through one environment-agnostic core, where routing, delivery, receipts and binding logic never learn
which environment a seat lives in. Idle wake works as far as each host allows.

## Task tree

- ht-kqrt · Environments: named seats, tmux wake and herdr-core terminals behind one host seam (epic) · Root epic of super-auto run 2026-10-31-environments-named-seats-herdr-app (branch super-auto/environments-named-seats-herdr-app). · deps: none
  - ht-kqrt.1 · Bring main into the run branch · Merge local `main` (contains ht-tod8 5e19e9bce: `lifecycle::scrub_env`, `host::PANE_SCOPED_ENV_VARS`) into the run branch so every later leaf starts from that b · deps: none
      owns: main merged into the run branch (base for every leaf)
  - ht-kqrt.2 · K5 env scrub: child_env_scrub_list() and spawn_detached · Make `scrub_env` `pub(crate)`, add `host::child_env_scrub_list()` (union of `PANE_SCOPED_ENV_VARS`, the named scrub-only list `HERDR_THREADS_SEAT_NAME`/`HERDR_T · deps: ht-kqrt.1
      owns: host::child_env_scrub_list() (Env scrub list); host scrub-only ENV_VARS consts for named and hc in src/host/mod.rs
      consumes: lifecycle::scrub_env and host::PANE_SCOPED_ENV_VARS from main (F1)
  - ht-kqrt.3 · HostPort capability answers: HostKind{Herdr,Unknown}, six defaulted methods, G4 implemented-kinds set · In src/ports.rs add `HostKind{Herdr, Unknown}`, `WakeCapability{None, Composer(ComposerTraits{submit_budget})}` with `ComposerTraits::HERDR_TUI`, `ArchivalEvide · deps: ht-kqrt.1
      owns: HostPort capability answers; HostKind implemented set; HostKind, WakeCapability, ComposerTraits::HERDR_TUI, ArchivalEvidence, ClosureAuthority enums
  - ht-kqrt.4 · NodeDelta 0012_host_environments.sql, Herdr kind writes, unknown-kind fail-closed · Add `migrations/node/0012_host_environments.sql` (next free slot after F1; renumber rule in run.md) registered in `NODE_DELTAS`, carrying every column of 07 §6. · deps: ht-kqrt.3
      owns: NodeDelta 0012 schema (herdr_hosts.kind/kind_probed_at, seats.name, seats_live_name, herdr_hosts_one_named, occupant_bindings.locator_hint/readonly, no_wake_reports)
      consumes: HostKind and the implemented-kinds set (F2)
  - ht-kqrt.5 · HostSessions builtin slot, attach_builtin, all_for_reads, Option wake lane · Extend src/service/host_sessions.rs: `builtin: OnceLock<Arc<HostSession>>`, `attach_builtin(port, observer, locator)` (skips `factory.exists`, the canonical che · deps: ht-kqrt.3
      owns: HostSessions::attach_builtin, all_for_reads, built-in slot; HostSession.wake: Option<LaneHandle>
      consumes: HostPort capability answers (F2)
  - ht-kqrt.6 · 07 §5.3 attached()/all_for_reads consumer audit (G1) and operator_host text · Apply the verified consumer table in 07 §5.3 at each call site: idle exit counts only dialled hosts holding a resolved local seat; cap and reattach sweep dialle · deps: ht-kqrt.5
      owns: dialled-only consumer filters per 07 §5.3 (idle exit, cap, reattach sweep, operator_host text)
      consumes: all_for_reads/attached() (F4); HostPort capability answers (F2, via F4)
  - ht-kqrt.7 · HarnessRegistry::idle_wake_on and its table test · Add `HarnessRegistry::idle_wake_on(harness, WakeCapability, channel_live) -> Option<IdleWakeOn{Composer, Channel, None}>` in src/harness/registry.rs. · deps: ht-kqrt.3
      owns: HarnessRegistry::idle_wake_on and IdleWakeOn
      consumes: WakeCapability (F2)
  - ht-kqrt.8 · wake_status per-row host through all_for_reads · `wake_status` (src/service/dispatch.rs:252) takes each row's own host (join on `seats.host_id`, not a column), looks it up through `all_for_reads`, and classifi · deps: ht-kqrt.5, ht-kqrt.7
      owns: WakeStatus row host id field; per-row host lookup in wake_status
      consumes: all_for_reads (F4); idle_wake_on (F5)
  - ht-kqrt.9 · HostTopologies (+capability), SeatDirectory.name, K4 label keying, CLI label NativeCli sites removed · Fix K4: `Command::HostTopologies` gated on local capability `topology.host_topologies_v1` (src/protocol/capabilities.rs, advertised from F); `#[serde(default)]  · deps: ht-kqrt.5
      owns: Command::HostTopologies + topology.host_topologies_v1; SeatDirectory.name field; (host_id, target) label keying
      consumes: all_for_reads (F4)
  - ht-kqrt.10 · caller_location.rs: CallerEnv/Located/CallerSite (Pane), env-read scan, recovery hints, hook gates · Create src/cli/caller_location.rs, the CLI's only reader of caller-location env: `CallerEnv::{from_process, from_lookup}`, `locate(EnvMode::{Cli,Hook})`, `Locat · deps: ht-kqrt.2
      owns: CallerEnv / Located / CallerSite; host::native::ENV_VARS usage as the Pane family names
      consumes: host scrub-only ENV_VARS consts (F1-scrub)
  - ht-kqrt.11 · LocateCaller (Herdr candidate), transport peer pid, TUI leak guard M1/M3 · Add `Command::LocateCaller{candidates, subject}` → `LocateReply::{Verified{index, root}, Unverified{per_candidate}}` in src/service/locate.rs (new): walk from t · deps: ht-kqrt.10, ht-kqrt.3, ht-kqrt.5
      owns: LocateCaller; transport peer pid; M1/M3 guard and the per-binding M3 memory
      consumes: CallerEnv / Located / CallerSite (F7); HostPort capability answers (F2); all_for_reads (F4)
  - ht-kqrt.12 · SeatsQuery.host and ResolveThreadQuery caller host (unsent); CLI site-capability check mechanism · Add `SeatsQuery.host: Option<HostLocator>` and the caller host on `ResolveThreadQuery` (both `deny_unknown_fields`), sent only when `host_scoped_lookup()` (neve · deps: ht-kqrt.1
      owns: SeatsQuery.host and ResolveThreadQuery caller-host fields; CLI site-capability gate mechanism
  - ht-kqrt.13 · F behavioural gate: herdr-release-smoke and wake-e2e self-test evidence · Before N and H1 build on F, run `nice scripts/herdr-release-smoke` (hook-running stand-in Codex in an isolated Herdr server) against the newest release in `test · deps: ht-kqrt.10, ht-kqrt.11, ht-kqrt.12, ht-kqrt.2, ht-kqrt.4, ht-kqrt.6, ht-kqrt.73, ht-kqrt.74, ht-kqrt.8, ht-kqrt.9
  - ht-kqrt.14 · Split file: src/store/seats.rs · Move-only, behaviour-preserving split of src/store/seats.rs (6.5k lines) into a `src/store/seats/` module along its existing concerns so the N store leaves and  · deps: ht-kqrt.1
      owns: file split of src/store/seats.rs into src/store/seats/
  - ht-kqrt.15 · NamedHost, NamedSeatSource, SeatName, HostKind::Named, attach_builtin at daemon start · Add `SeatName` (src/protocol/ids.rs: 1-64 bytes of `[A-Za-z0-9._-]`, no leading `-`/`.`, not seat-id-shaped) with the corpus test against the 0012 CHECK. · deps: ht-kqrt.13, ht-kqrt.4, ht-kqrt.5
      owns: NamedHost; NamedSeatSource; SeatName; HostKind::Named + implemented-set entry; host::named::ENV_VARS; reserved named-host locator
      consumes: HostPort capability answers (F2); attach_builtin (F4); NodeDelta 0012 schema (F3)
  - ht-kqrt.16 · Named caller site: CallerLocator::Named site answers, named resolve command shape, seat.host_sites_v1 gate (M13) · Add `CallerLocator::Named` to caller_location.rs (precedence rule 1: a valid name replaces only the target). · deps: ht-kqrt.10, ht-kqrt.11, ht-kqrt.12, ht-kqrt.15
      owns: CallerLocator::Named site answers; seat.host_sites_v1 capability and its CLI gate; named resolve command shape (name as target at the reserved locator)
      consumes: CallerEnv / Located / CallerSite (F7); NamedHost / SeatName (N1-host); LocateCaller M1/M3 (F8); SeatsQuery.host + site-capability mechanism (F9)
  - ht-kqrt.17 · Named site remainder: M14/M15/M16 refusals, watch S3, Hermes bridge, skill K1, AGENTS.md line · Refusals on the Named site: invalid name → M16; explicit cooperative flags + name → M14; `me init` under a name → M15. · deps: ht-kqrt.16
      owns: M14/M15/M16 refusals; watch on the Named site; Hermes bridge name handling (PANE_ENV); skill K1; AGENTS.md named-seat line; Claude mod channel on the Named site (register.js spawn env forwards HERDR_THREADS_SEAT_NAME/HERDR_THREADS_SEAT_READONLY; the mod does not stop on no_pane when a name is set)
      consumes: CallerLocator::Named site answers and named resolve command shape (N1-site); CallerEnv / Located / CallerSite (F7)
  - ht-kqrt.18 · Instance discovery for named sites (SA1), default state dir, lazy Herdr endpoint, doctor two stores · Implement SA1's discovery order for non-Pane sites in src/cli/instance.rs (`--state-dir`, `HERDR_PLUGIN_STATE_DIR`, hook-baked `--state-dir`, today's discovery  · deps: ht-kqrt.16
      owns: named-site instance discovery and default state dir creation; lazy Herdr endpoint for non-Pane sites; the default standalone state dir path function (single definition used by discovery and by Herdr-less setup baking)
      consumes: CallerEnv / Located / CallerSite incl. Named (N1-site)
  - ht-kqrt.19 · Standalone (Herdr-less) install: install.sh setup, install.json marker, baked --state-dir, leftover check · scripts/install.sh runs `setup` whenever it can (today it skips unless the plugin is linked, :909-911) and keeps `herdr plugin link` only when `herdr` is presen · deps: ht-kqrt.1, ht-kqrt.18
      owns: <state>/node/install.json standalone marker; Herdr-less setup baking --state-dir
      consumes: default state dir definition (N1-disc)
  - ht-kqrt.20 · Store: named allocation on the named host with the M8 creation flag; reads create nothing · In src/store/seats/ (after the split): allocate a named seat with `seats.name` on the named host and return a creation flag the CLI renders as M8; reads create  · deps: ht-kqrt.14, ht-kqrt.15, ht-kqrt.16, ht-kqrt.4
      owns: named seat allocation and M8 creation flag
      consumes: NamedHost (N1-host); NodeDelta 0012 schema (F3); HostKind implemented set (F2); named resolve command shape (N1-site)
  - ht-kqrt.21 · Store: named continuity (C1), cross-kind rebind refusal, Closed guard, wait:host, S1 · Add the fourth C1 path "named continuity" (an ordinary resolve whose name equals an unresolved named seat's name, with no live seat of that name, reattaches it, · deps: ht-kqrt.14, ht-kqrt.20
      owns: named continuity C1 path; cross-kind rebind refusal; Closed guard on named seats; wait:host exclusion of the named host; TRUST-POLICY N rows: C1 named continuity path, cooperative_named_seat provenance, cross-kind rebind refusal (same commit)
      consumes: named seat allocation (N3-alloc); NamedHost (N1-host); HostKind implemented set (F2)
  - ht-kqrt.22 · Store: restore rules for named and unnamed seats (SA4, D12) · Change the local-origin restore in `materialize_seat_created`: a row with `display_name` goes onto the named host, unresolved, at target = name; with a live sea · deps: ht-kqrt.14, ht-kqrt.20
      owns: restore placement rules; TRUST-POLICY D12 restore placement row (same commit)
      consumes: named allocation (N3-alloc); HostKind implemented set (F2); seat.created.display_name semantics (N7: set from seats.name, SeatName-validated on receive, NULL when invalid), through a fixture fact carrying display_name
  - ht-kqrt.23 · Store: live-holder guard (SA16, D19) and M7 · In the enroll transaction, when a named seat would gain or reconnect a top-level binding, check the seat's other connected top-level bindings with `LivenessFing · deps: ht-kqrt.14, ht-kqrt.20
      owns: live-holder guard and seat_name_in_use; TRUST-POLICY D19 live-holder guard row and M7 (same commit)
      consumes: named allocation (N3-alloc)
  - ht-kqrt.24 · Read-only bindings (D3): HERDR_THREADS_SEAT_READONLY · `readonly=1` is recorded on the binding when the caller env has `HERDR_THREADS_SEAT_READONLY=1` (read in caller_location.rs, carried in the resolve command). · deps: ht-kqrt.16, ht-kqrt.20
      owns: occupant_bindings.readonly semantics; readonly field on the resolve command (#[serde(default)], read from HERDR_THREADS_SEAT_READONLY in caller_location.rs); TRUST-POLICY read-only binding receipt semantics (a readonly display never ACKs) (same commit)
      consumes: NodeDelta 0012 readonly column (F3); CallerEnv / Located / CallerSite (N1-site)
  - ht-kqrt.25 · Named presence (SA17): no baseline/recovery holds, structural boot carry, recipient_unavailable · Named presence follows the pane precedent: a named seat goes offline only when its bindings end and then shows "inactive, unavailable since T" in seat list/part · deps: ht-kqrt.14, ht-kqrt.20
      owns: named presence semantics; one unresolved named row never silences liveness of other named bindings on the shared named host
      consumes: NamedHost fixed boot (N1-host); named allocation (N3-alloc)
  - ht-kqrt.26 · Self-retire: retire_seat, Command::SelfRetire, seat retire --self (SA6, D5) · Factor `retire_seat(tx, seat, wire, label)` out of `operator_retire`. · deps: ht-kqrt.14, ht-kqrt.16, ht-kqrt.20
      owns: retire_seat; Command::SelfRetire; seat retire --self; TRUST-POLICY row for cooperative_self_retire and its RetireReason::Operator replication (same commit)
      consumes: named allocation (N3-alloc); CallerEnv / Located / CallerSite Named (N1-site)
  - ht-kqrt.27 · Inherited-name refusal (SA2, D1): server environ reader, resolve-command locator_hint, M6 · When a host candidate sits beside the name, the resolve command carries it as `locator_hint`; the daemon reads the environ of the host's `server_process()` (Lin · deps: ht-kqrt.11, ht-kqrt.16, ht-kqrt.20
      owns: server environ reader; locator_hint field on the named resolve command; M6; TRUST-POLICY D1 inherited-name refusal row (same commit)
      consumes: LocateCaller (F8); HostPort server_process answer (F2); named resolve command shape (N1-site); named allocation (N3-alloc)
  - ht-kqrt.28 · Persisted locator_hint, M9 second-locator notice, seat inspect by locator · Each named binding records `occupant_bindings.locator_hint`; a binding with a hint no earlier row of the seat carries, while another distinct hint exists, retur · deps: ht-kqrt.14, ht-kqrt.20, ht-kqrt.72
      owns: occupant_bindings.locator_hint recording; M9; seat inspect by-locator listing; TRUST-POLICY M9 second-locator notice row (same commit)
      consumes: locator_hint on the resolve command (N4-inherit); named allocation (N3-alloc)
  - ht-kqrt.29 · Pane-form allocation refusal M10 (SA9) · Pane-form allocation of a pane whose current named binding hinted it is refused with M10 (SA9). · deps: ht-kqrt.14, ht-kqrt.20, ht-kqrt.28
      owns: M10 pane-form allocation refusal
      consumes: persisted locator_hint (N4-hint); named allocation (N3-alloc)
  - ht-kqrt.30 · Named display and selectors (SA14, D4), handoff out of a named seat · Fill `SeatDirectory.name`; `thread show`, delivery `from` headers, `seat list`, participants and `seat inspect` render the name through the pane label with work · deps: ht-kqrt.16, ht-kqrt.18, ht-kqrt.20, ht-kqrt.9
      owns: named label rendering and name selector reading; Named-site CallerSite::env_assignments handoff recovery hint (HERDR_THREADS_SEAT_NAME=<n>); M11 handoff-into-named-seat refusal (NamedHost port answer)
      consumes: HostTopologies/SeatDirectory.name (F6); NamedHost seat_labels (N1-host); named allocation (N3-alloc); CallerLocator::Named site answers (N1-site); NamedHost native_launch_capability answer (N1-host) for M11
  - ht-kqrt.31 · No-wake reports: persisted no_wake_reports, WakeBlock::NoWake, wake=no_wake rendering · `wake_status` maps an effective capability `None` to `WakeBlock::NoWake`, persisted once per (sender, seat) per no-wake episode in `no_wake_reports` (row writte · deps: ht-kqrt.15, ht-kqrt.20, ht-kqrt.4, ht-kqrt.8
      owns: no_wake_reports semantics and WakeBlock::NoWake; wake=no_wake rendering
      consumes: idle_wake_on/wake_status (F5, F5w); NamedHost (N1-host); named allocation (N3-alloc)
  - ht-kqrt.32 · No-wake remainder: inert threshold M12, M19 offline with poke/reminder suppression, S6 doctor, host-side tripwire, K2/K3, rules 5/6 docs · ` offline` suffix (M19) on `wake=no_wake` for named/hc seats whose bindings ended. · deps: ht-kqrt.15, ht-kqrt.20, ht-kqrt.31
      owns: host-side no-wake tripwire; M12, M19, K2, K3; poke/reminder suppression for offline no-wake seats; rules 5/6 degradation docs (TRUST-POLICY A4, AGENTS.md rules 5/6)
      consumes: no_wake_reports / WakeBlock::NoWake / wake=no_wake rendering (N6); named allocation (N3-alloc); NamedHost test-only constructor (N1-host)
  - ht-kqrt.33 · Sync: display_name publish and remote materialize; name@node · `ensure_published` sets `seat.created.display_name` from `seats.name`; on receive it is validated with `SeatName` and an invalid value is stored as NULL. · deps: ht-kqrt.14, ht-kqrt.20, ht-kqrt.30
      owns: seat.created.display_name semantics: set from seats.name in ensure_published, validated with SeatName on receive, NULL when invalid (existing baselined field, no new wire line)
      consumes: SeatName (N1-host); named allocation (N3-alloc); name selector (N5)
  - ht-kqrt.34 · Named seats end to end through one daemon · Integration suite (own `[[test]]` target with `required-features = ["test-support"]`, in-process or spawned daemon) proving use case 1 and mixed messaging: name · deps: ht-kqrt.13, ht-kqrt.16, ht-kqrt.17, ht-kqrt.18, ht-kqrt.19, ht-kqrt.20, ht-kqrt.21, ht-kqrt.23, ht-kqrt.24, ht-kqrt.25, ht-kqrt.26, ht-kqrt.28, ht-kqrt.29, ht-kqrt.30, ht-kqrt.31, ht-kqrt.32, ht-kqrt.33, ht-kqrt.6, ht-kqrt.65, ht-kqrt.71, ht-kqrt.72
      consumes: Named site (N1-site); named allocation (N3-alloc); discovery (N1-disc); display (N5); no-wake reports (N6); all N leaves (phase gate)
  - ht-kqrt.35 · HostPort::target_wake_capability and per-target no-wake reports · Add defaulted `HostPort::target_wake_capability(&HostTargetId) -> WakeCapability` (default `self.wake_capability()`). · deps: ht-kqrt.31, ht-kqrt.32
      owns: HostPort::target_wake_capability
      consumes: HostPort capability answers (F2); no-wake reports (N6); host-side tripwire (N6-rest)
  - ht-kqrt.36 · turn_state_edge adapter method, Claude report-only Stop, daemon per-binding turn state · Add neutral defaulted `HarnessAdapter::turn_state_edge(event) -> Option<TurnEdge>` (default `None` = no tmux wake for that harness). · deps: ht-kqrt.1, ht-kqrt.10, ht-kqrt.11, ht-kqrt.16
      owns: HarnessAdapter::turn_state_edge and TurnEdge; per-binding hook turn state in the daemon; turn_state_edge implementations for Codex and agy over their existing hook events (src/harness/codex, src/harness/agy/ only)
      consumes: hook.rs caller-location and hook-gate refactor (F7, F8, N1-site)
  - ht-kqrt.37 · ComposerPolicy::screen_blocked default and harness recognizers · Add defaulted `ComposerPolicy::screen_blocked(&str) -> Option<bool>` (default `None` = no wake), implemented by harnesses whose captured fixtures support recogn · deps: ht-kqrt.1
      owns: ComposerPolicy::screen_blocked
  - ht-kqrt.38 · tmux hint: TMUX/TMUX_PANE read, resolve-command hint, verification, per-target answer, tmux test helper · caller_location.rs reads `TMUX` (socket, server pid) and `TMUX_PANE` into `tmux:<socket>:<server_pid>:<pane>`, carried in the resolve command (not `CallerClaim` · deps: ht-kqrt.15, ht-kqrt.16, ht-kqrt.28, ht-kqrt.35
      owns: tmux hint format and per-binding verification; private tmux test helper; scripts/check-no-leaked-processes recognition of tmux servers started with -L ht-test-; tmux hint as its own resolve-command field (#[serde(default)]), held in daemon memory per binding and lost on restart (accepted limit), never stored as occupant_bindings.locator_hint and never triggers M9
      consumes: CallerEnv / Located / CallerSite (N1-site); NamedHost (N1-host); target_wake_capability (N2-target); occupant_bindings.locator_hint recording and M9 (N4-hint)
  - ht-kqrt.39 · tmux composer wake: built-in wake lane, capture-pane read, send-keys typing, wake condition · `NamedHost::wake_capability()` becomes `Composer(ComposerTraits::TMUX{submit_budget: 2 s})`, so the lane starts; the built-in session gets its own wake lane at  · deps: ht-kqrt.35, ht-kqrt.36, ht-kqrt.37, ht-kqrt.38
      owns: ComposerTraits::TMUX; built-in session wake lane registration; delivery-contract audit table in the commit message; AGENTS.md rule 6 Native wording and the tmux "What we do today" line; TRUST-POLICY N2 A4 row (tmux wake + accepted limits); T1-T5 results in the run report
      consumes: NamedHost (N1-host); turn_state_edge (N2-turn); screen_blocked (N2-blocked); target_wake_capability (N2-target); HostSession.wake: Option (F4); tmux hint as its own resolve-command field held in daemon memory per binding (N2-hint)
  - ht-kqrt.40 · hc JSON-RPC client and wire types, fake_hc (in-thread + fake-hc bin), vendored schema and contract test · Create src/host/hc/rpc.rs (JSON-RPC 2.0 line client, one connection per call: `hello`, `terminal.get`, `events.subscribe` one-shot snapshot, `doc.get`; never `c · deps: ht-kqrt.1, ht-kqrt.13
      owns: hc JSON-RPC client and wire types; fake_hc (in-thread and fake-hc bin); vendored hc 0.1.15 schema fixture; isolated-core helper (scripts/lib/isolated-core.sh); leak-check recognition of herdr-core serve/hold
  - ht-kqrt.41 · HcHost observation, HcBoot, HostKind::Hc, enumerate_targets, target_root_process, ENV_VARS · Create `HcHost` in src/host/hc/mod.rs: `HostKind::Hc` plus implemented-set entry; boot = incarnation = `hc-core:<core.id>` (`HcBoot::from_hello`, ASCII ≤128 B); · deps: ht-kqrt.14, ht-kqrt.3, ht-kqrt.40
      owns: HcHost / HcBoot; HostKind::Hc + implemented-set entry; enumerate_targets; target_root_process; host::hc::ENV_VARS; hc label vocabulary (herdr.space, herdr.tab, ht.bootstrap as hint-only, ht.label; writers ht-kqrt.55/ht-kqrt.57, reader ht-kqrt.43) and the <session>:tN label/selector format; TRUST-POLICY H1 vocabulary hc row (core id boot, new id = new boot = C2 holds) and C1 structural row (same commit)
      consumes: hc rpc client and fake_hc (H1a); HostPort capability answers (F2)
  - ht-kqrt.42 · HcHost finish-reason closure, C2 holds on core-id rotation, status null → Unknown · Finish-reason closure (D10) through the store publication-absence path over a coherent capture of the same core id: `killed`/`removed` retire, exit/signal/`serv · deps: ht-kqrt.14, ht-kqrt.41
      owns: finish-reason closure mapping; C2 holds on core-id rotation; status: null → Unknown and the doctor status source; TRUST-POLICY H1 D10 closure table, C2 on core-id rotation and the wiped/cloned-core accepted limit (same commit)
      consumes: HcHost / HcBoot / enumerate_targets (H1c); fake_hc knobs (H1a)
  - ht-kqrt.43 · hc labels <session>:tN, herdr-app doc names, ambiguous bare tN refusal · `HcHost::seat_labels`/`live_topology` take workspace from `herdr.space`, tab from `herdr.tab`, target `tN`; names from doc `herdr-app` parsed tolerantly and cac · deps: ht-kqrt.41, ht-kqrt.9
      owns: HcHost seat_labels/live_topology output; herdr-app doc-name parsing and cache; ambiguous bare tN refusal
      consumes: HcHost (H1c); HostTopologies (F6); hc label vocabulary and <session>:tN format (H1c)
  - ht-kqrt.44 · HostFactory {exists, probe, build(kind, locator)}; probe before attach_host · `HostFactory` (host_sessions.rs:61-77, closures in src/main.rs:67-80) becomes `{exists, probe, build}`. · deps: ht-kqrt.4, ht-kqrt.40, ht-kqrt.41
      owns: HostFactory::probe and build(kind, locator)
      consumes: HcHost (H1c); hc rpc client (H1a); NodeDelta 0012 kind columns (F3)
  - ht-kqrt.45 · HcDiscovery: core.json scan, socket derivation, hello id match · `HcDiscovery` in src/host/hc/ reads core state and never runs `hc`: list `$XDG_STATE_HOME|~/.local/state/herdr-core/*/core.json`; derive each socket by the core · deps: ht-kqrt.40, ht-kqrt.41
      owns: HcDiscovery API and core state paths ($XDG_STATE_HOME|~/.local/state/herdr-core//core.json, socket derivation, hello id match)
      consumes: HcHost (H1c); implements the discovery part of the HcHost abstraction in src/host/hc/discovery.rs and updates its responsibility comment; hc rpc client (H1a)
  - ht-kqrt.46 · CallerLocator::Terminal, full LocateCaller, M4 and M17; remove M1/M2/M3 · Add `CallerLocator::Terminal{socket,id}` (`Terminal` without `HC_TERMINAL_SOCKET` → M17). · deps: ht-kqrt.11, ht-kqrt.16, ht-kqrt.18, ht-kqrt.41, ht-kqrt.44
      owns: CallerLocator::Terminal site answers; M4, M17; TRUST-POLICY H1 A1 addendum in full with the double-fork and Codex-daemon-mode limits and the single-family stale-env limit (same commit)
      consumes: CallerEnv / Located / CallerSite (F7, N1-site); LocateCaller (F8); HcHost (H1c); HostFactory probe (H1b)
  - ht-kqrt.47 · HcHost::server_process and the hc inherited-name check (E1 hc path) · `HcHost::server_process` = the core socket's peer pid from the `LocalEndpointWitness` the client records. · deps: ht-kqrt.40, ht-kqrt.41, ht-kqrt.46, ht-kqrt.72
      owns: HcHost::server_process; LocalEndpointWitness peer-pid recording in the hc client
      consumes: server environ reader (N4-inherit); HcHost (H1c); fake-hc bin (H1a); Terminal site (H1e)
  - ht-kqrt.48 · seat.terminal_sites_v1 and hc messaging end to end with wake=no_wake; skill K4 · Advertise `seat.terminal_sites_v1` and gate Terminal sites on it in the CLI (M13 against an N-only daemon). · deps: ht-kqrt.12, ht-kqrt.16, ht-kqrt.17, ht-kqrt.26, ht-kqrt.31, ht-kqrt.44, ht-kqrt.46
      owns: seat.terminal_sites_v1 capability; mod watch/channel on a Terminal site (register.js spawn env carries HC_TERMINAL_ID/HC_TERMINAL_SOCKET; never stops on no_pane); TRUST-POLICY H1 A4 row (hc has no composer wake until upstream) and the AGENTS.md H1 hc "What we do today" line (same commit); M20 refusal of seat retire --self from a Terminal site
      consumes: Terminal site (H1e); HostFactory probe (H1b); no-wake reports (N6); self-retire (N3-retire); watch on the Named site (N1-site-rest); CLI site-capability gate mechanism (F9, ht-kqrt.12)
  - ht-kqrt.49 · hc-only setup (no TUI --host-endpoint) and Hermes PANE_ENV terminal vars · On an hc-only machine (a core state dir present, no `herdr` on PATH) `setup` bakes `--state-dir` and no TUI `--host-endpoint` (src/harness/setup.rs). · deps: ht-kqrt.17, ht-kqrt.18, ht-kqrt.19, ht-kqrt.45
      consumes: Hermes bridge PANE_ENV (N1-site-rest); standalone install (N-install); HcDiscovery state paths (H1d); named-site discovery (N1-disc)
  - ht-kqrt.50 · Two-host dialled-only consumer tests (TUI fake + fake hc), two-kind restore, seatless core idle exit · Each dialled-only consumer gets a two-host test where hc holds the lowest host id: `startup_primary`, `first_host_pacer`, `place()`, the discovery gate, archiva · deps: ht-kqrt.41, ht-kqrt.44, ht-kqrt.6, ht-kqrt.71
      consumes: HostFactory probe (H1b); HcHost (H1c); consumer filters (F4-audit); restore rules (N3-restore)
  - ht-kqrt.51 · hc schema-conformance test (always shipped) · `tests/host/hc_schema_conformance.rs` in tests/combined.rs validates every fake_hc reply against the result schema of its method in the vendored `hc-0.1.15.open · deps: ht-kqrt.40
      consumes: fake_hc and vendored schema (H1a)
  - ht-kqrt.52 · scripts/hc-smoke real-core smoke (opt-in, only with a verified pin) · Ship `scripts/hc-smoke` plus `tests/herdr-core-releases.tsv` only if the implementer can fetch and verify the herdr-core 0.1.15 manifest from `https://releases. · deps: ht-kqrt.2, ht-kqrt.40, ht-kqrt.43, ht-kqrt.44, ht-kqrt.67
      consumes: HcHost (H1c); finish-reason closure (H1c-closure); HostFactory probe (H1b); hc labels (H1c-labels); isolated-core helper and herdr-core serve/hold leak recognition (H1a)
  - ht-kqrt.53 · Seam contract: CreateTargetPort, Placement, HostWitness, CreatedTarget, NativeCli delegate, V3 journal records · Add `CreateTargetPort` beside `CreateTabPort` (src/ports.rs:5536) with `CreateTargetRequest`, `Placement{NewTab{space}, NewTerminal{parent}}`, `HostWitness{Herd · deps: ht-kqrt.1
      owns: CreateTargetPort; Placement, HostWitness, CreatedTarget; V3 bootstrap journal records; launch-gate authorization command and reply (bootstrap <compound>:<attempt>, (core, tN) witness, bounded timeout, refusal), its local capability, and the rule that the bootstrap Send is staged before the reply
  - ht-kqrt.54 · Launch-gate shim: internal launch-gate, env blanking, child_scrub_vars, host_launch_argv, Codex --no-daemon · src/cli/launch_gate.rs (new): `herdr-threads internal launch-gate --bootstrap <compound>:<attempt> -- <argv>` reads `HC_TERMINAL_ID`/`HC_TERMINAL_SOCKET` throug · deps: ht-kqrt.10, ht-kqrt.2, ht-kqrt.41, ht-kqrt.53
      owns: internal launch-gate shim; HarnessAdapter::child_scrub_vars and host_launch_argv; TRUST-POLICY H3 managed_launch host-guarded launch row (gate shim, root-pid fingerprint; same commit)
      consumes: Env scrub list (F1-scrub); CallerEnv (F7); HostKind::Hc (H1c); launch-gate authorization command and reply (H3a)
  - ht-kqrt.55 · HcHost CreateTargetPort via terminal.run; fake_hc terminal.run · Add `terminal.run` to the hc client and fake_hc. · deps: ht-kqrt.40, ht-kqrt.41, ht-kqrt.53
      owns: terminal.run client method and its parameter set (no_notify, outlive_parent, no_parent, labels; no caller, no first_message; each checked against the vendored hc-0.1.15 schema), plus the fake_hc terminal.run; TRUST-POLICY H3 row: the ht.bootstrap label is hint-only (same commit)
      consumes: CreateTargetPort (H3a); HcHost (H1c); fake_hc (H1a); hc label vocabulary (H1c)
  - ht-kqrt.56 · Herdr launch and handoff through CreateTargetPort (generic refactor) · Make topology_handoff*, topology_runtime, handoff and launch code generic over `CreateTargetPort`; F's launch/handoff `NativeCli::new` sites (src/cli/handoff.rs · deps: ht-kqrt.53
      owns: src/cli/host_client.rs launch handle
      consumes: CreateTargetPort (H3a)
  - ht-kqrt.57 · hc handoff/launch CLI: --space/--new-tab/--new-terminal, in-place refusal, gate-shim handoff end to end · `handoff`/`launch --space <app space>`, `--new-tab`, `--new-terminal` (no app labels, parent = caller) select a hc core from the caller's Terminal site or `HcDi · deps: ht-kqrt.45, ht-kqrt.46, ht-kqrt.54, ht-kqrt.55, ht-kqrt.56
      owns: TRUST-POLICY H3 C4 staging row for the hc launch (same commit as the behaviour); daemon LaunchGate handler (matches the shim's HC_TERMINAL_ID/(core, tN) against the CreatedTarget/HostWitness and HcHost terminal.get, stages the bootstrap Send before the reply, refuses after a bounded timeout)
      consumes: CreateTargetPort (H3a via H3d1); gate shim (H3b); HcHost terminal.run (H3c); HcDiscovery (H1d); Terminal site (H1e); hc label vocabulary (H1c); launch-gate authorization command and reply, bootstrap Send staged before the reply (H3a)
  - ht-kqrt.58 · Peer launch: NewTab onto hc spaces, cap-gated LaunchTarget::NewTerminal, peer-target-create alias · Make peer_launch and peer_launcher generic over `CreateTargetPort` (src/cli/peer_launch.rs:48 site). · deps: ht-kqrt.53, ht-kqrt.55
      owns: LaunchTarget::NewTerminal + PEER_LAUNCH_TERMINAL_CAP; internal peer-target-create
      consumes: CreateTargetPort (H3a); HcHost terminal.run (H3c)
  - ht-kqrt.59 · Configuration smoke: Herdr pane, named, named+tmux and hc terminal sites x claude/codex/hermes/agy hook stand-ins · One integration target that exercises each enumerated configuration once: for each caller site (Herdr pane in an isolated Herdr session; `HERDR_THREADS_SEAT_NAM · deps: ht-kqrt.16, ht-kqrt.34, ht-kqrt.43, ht-kqrt.48, ht-kqrt.69
      consumes: Named site (N1-site); hc messaging (H1g); tmux wake (N2-wake)
  - ht-kqrt.60 · hc seats across nodes: remote publication and addressing of hc terminal seats · Publish hc terminal seats' labels to peers through the existing `SeatStatusEntry.pane`/`seat_labels` path (the same path N7 uses for named seats), so a peer nod · deps: ht-kqrt.33, ht-kqrt.43, ht-kqrt.48
      owns: remote publication of hc seat labels through the existing SeatStatusEntry.pane/seat_labels path; <session>:tN@node remote selector reading
      consumes: hc label vocabulary and <session>:tN format (H1c, ht-kqrt.41); hc labels (H1c-labels); remote seat_labels publish and name@node selector (N7); seat.terminal_sites_v1 and Terminal-site messaging (H1g)
  - ht-kqrt.61 · Opt-in real-agent wake-e2e case: named seat in a private tmux server (tmux-named) · Add an opt-in case to scripts/wake-e2e.py (`--cases tmux-named/send-p0`, plus `tmux-named/send-p1` as the silent control) that runs a named seat (`HERDR_THREADS · deps: ht-kqrt.69, ht-kqrt.70
      owns: tmux-named/ opt-in wake-e2e cases and their cleanup
      consumes: tmux composer wake (N2-wake); per-binding turn state and turn_state_edge (N2-turn); private tmux helper and -L ht-test- leak recognition (N2-hint)
  - ht-kqrt.62 · N2 phase gate: unfiltered suite, leak checks, Herdr-pane regression proof, T1-T5 evidence · Before phase N2 merges into the run branch, run the per-phase gate on the tree with every N2 leaf merged and record the evidence (commands, SHA, results) in the · deps: ht-kqrt.61, ht-kqrt.65, ht-kqrt.69
      consumes: all N2 leaves (phase gate)
  - ht-kqrt.63 · H1 phase gate: unfiltered suite, leak checks, D1/E1/S5/H-C1 evidence, hc-smoke status · Before phase H1 merges into the run branch, run the per-phase gate on the tree with every H1 leaf merged and record the evidence (commands, SHA, results) in the · deps: ht-kqrt.13, ht-kqrt.45, ht-kqrt.47, ht-kqrt.49, ht-kqrt.50, ht-kqrt.51, ht-kqrt.52, ht-kqrt.60, ht-kqrt.65, ht-kqrt.67
      consumes: all H1 leaves (phase gate)
  - ht-kqrt.64 · Final gate (whole merged run branch, also H3 phase gate): unfiltered suite, leak checks, Herdr regression proofs, L1-L4 evidence · This is the final gate. · deps: ht-kqrt.57, ht-kqrt.58, ht-kqrt.59, ht-kqrt.62, ht-kqrt.63, ht-kqrt.65, ht-kqrt.68
      consumes: all H3 leaves (phase gate); N2 gate, H1 gate and configuration smoke (whole-tree gate)
  - ht-kqrt.65 · Host-neutrality source scan: no HostKind::Named/Hc or host env branch in routing, delivery, receipts, binding · Add a source-scan test (a `tests/combined.rs` `#[path]` module, no process-wide state) that enforces the goal's "routing, delivery, receipts and binding never l · deps: ht-kqrt.10, ht-kqrt.3
      owns: host-neutrality source scan (test module) and its allowlist file
      consumes: HostKind and the four permitted kind() readers rule (F2); src/cli/caller_location.rs as the only caller-location env reader (F7)
  - ht-kqrt.66 · Integration sweep: three environments behind one host seam · Scope-bounded root integration sweep, run after every leaf has merged into the run branch (after the final gate ht-kqrt.64). · deps: ht-kqrt.1, ht-kqrt.10, ht-kqrt.11, ht-kqrt.12, ht-kqrt.13, ht-kqrt.14, ht-kqrt.15, ht-kqrt.16, ht-kqrt.17, ht-kqrt.18, ht-kqrt.19, ht-kqrt.2, ht-kqrt.20, ht-kqrt.21, ht-kqrt.23, ht-kqrt.24, ht-kqrt.25, ht-kqrt.26, ht-kqrt.28, ht-kqrt.29, ht-kqrt.3, ht-kqrt.30, ht-kqrt.31, ht-kqrt.32, ht-kqrt.33, ht-kqrt.34, ht-kqrt.35, ht-kqrt.37, ht-kqrt.38, ht-kqrt.4, ht-kqrt.40, ht-kqrt.41, ht-kqrt.43, ht-kqrt.44, ht-kqrt.45, ht-kqrt.46, ht-kqrt.47, ht-kqrt.48, ht-kqrt.49, ht-kqrt.5, ht-kqrt.50, ht-kqrt.51, ht-kqrt.52, ht-kqrt.53, ht-kqrt.54, ht-kqrt.55, ht-kqrt.56, ht-kqrt.57, ht-kqrt.58, ht-kqrt.59, ht-kqrt.6, ht-kqrt.60, ht-kqrt.61, ht-kqrt.62, ht-kqrt.63, ht-kqrt.64, ht-kqrt.65, ht-kqrt.67, ht-kqrt.68, ht-kqrt.69, ht-kqrt.7, ht-kqrt.70, ht-kqrt.71, ht-kqrt.72, ht-kqrt.73, ht-kqrt.74, ht-kqrt.8, ht-kqrt.9
      consumes: all leaves (integration sweep)
  - ht-kqrt.67 · HcHost D10 finality closure over a capture with finished records · Owns: `enumerate_targets` keeping finished local records with `reason` and `keep` (machine mirrors still dropped, only live records are present targets); the D1 · deps: ht-kqrt.14, ht-kqrt.40, ht-kqrt.41
  - ht-kqrt.68 · Launched hc worker retires on exit (H3 keep:false finality) · Owns: the H3 acceptance that a gate-shim-launched worker (`keep: false`, harness exec'd as terminal root) retires its seat through D10 when it exits, so no laun · deps: ht-kqrt.54, ht-kqrt.55, ht-kqrt.67
  - ht-kqrt.69 · tmux composer wake: built-in lane, idle-exit keep-alive, pane-mode gate, capture-pane read, send-keys typing · Owns: `ComposerTraits::TMUX`; the built-in session wake lane (pacer, reachability, Health); the new `ActivityProbe::active` term counting wake-armed named bindi · deps: ht-kqrt.35, ht-kqrt.37, ht-kqrt.38, ht-kqrt.5, ht-kqrt.70
  - ht-kqrt.70 · tmux turn state: turn_state_edge with every turn-ending edge, Claude report-only Stop and StopFailure, screen-idle clear · Owns: `HarnessAdapter::turn_state_edge` and `TurnEdge` (including `ScreenIdle`); per-binding hook turn state in the daemon (busy on turn start; idle on `Stop`,  · deps: ht-kqrt.1, ht-kqrt.10, ht-kqrt.11, ht-kqrt.16
  - ht-kqrt.71 · Store: restore rules with nameless hold on a name collision (SA4, D12) · Owns: the local-origin restore in `materialize_seat_created` (named row onto the named host unresolved at target = name; on collision with a non-retired seat of · deps: ht-kqrt.14, ht-kqrt.20, ht-kqrt.26, ht-kqrt.3, ht-kqrt.33
  - ht-kqrt.72 · Inherited-name refusal (SA2, D1) with the same-name limit and the revised M6 · Owns: server environ reader (Linux `/proc`, macOS `KERN_PROCARGS2` behind cfg); `locator_hint` field on the named resolve command; M6 with the text "use another · deps: ht-kqrt.11, ht-kqrt.16, ht-kqrt.20, ht-kqrt.3
  - ht-kqrt.73 · spawn_detached reparents the daemon (double fork through internal daemon-detach) · Owns: `herdr-threads internal daemon-detach` (setsid, spawn the daemon with the scrubbed env and the same args/stdio, report its pid on stdout, exit at once); ` · deps: ht-kqrt.2
  - ht-kqrt.74 · LocateCaller terminal-root wall and start-stamped cache key · Owns: `host::foreign_terminal_process_names()` (F: a const list in src/host/mod.rs with `herdr-core` and `hc`, moving into `host::hc` in H1); the walk rule that · deps: ht-kqrt.11
