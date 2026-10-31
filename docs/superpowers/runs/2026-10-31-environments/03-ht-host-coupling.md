# herdr-threads coupling to Herdr TUI (investigation, 2026-10-31)

Read-only survey of `.worktrees/external-seat` at badccf481 (f4ddaeffa plus the named-seats design doc).
Input to abstracting "the environment ht runs in" (Herdr TUI pane, plain process with
`HERDR_THREADS_SEAT_NAME`, herdr-core/herdr-app terminal). Builds on `00-seat-mechanics-today.md`.
File:line refs are entry points; "(inferred)" marks claims not traced to the end.

Categories: (a) identity/caller location, (b) topology/observation, (c) wake/input typing, (d) labels/display,
(e) launch/handoff, (f) install/plugin registration/state-dir discovery, (g) liveness/process, (h) peer wire/addresses.

## 1. Existing seams (what is already behind a trait)

| Seam | Where | Covers | Production impl |
|---|---|---|---|
| `HostPort` | src/ports.rs:3907-4130 | observe target / snapshot (`enumerate_targets`), wake target + `submit_prompt*`, composer read/clear/stash/restore, `pane_agent_state`, `foreground_pids`, `launch_native*`, `seat_labels`, `live_topology`, `observe_pane_agent`, epoch/incarnation/release/health capabilities | `NativeCli` src/host/native.rs:1249 (only one) |
| `BootstrapObserver` | src/ports.rs:1435 | observe a freshly created pane (handoff/topology) | `NativeCli` src/host/native.rs:42 |
| `CreateTabPort` | src/ports.rs:5536 ("Future required typed topology boundary, deliberately separate from HostPort") | `tab.create` | `NativeCli` src/host/native.rs:85 |
| `HostFactory{build,exists}` | src/service/host_sessions.rs:61-77 | daemon's only constructor of `HostPort`; `exists` = "is a unix socket" | closures in src/main.rs:67-80 |
| `HostSessions` | src/service/host_sessions.rs:397 (MAX_ATTACHED_HOSTS=16 :42) | per-host session: port, observer, observation + wake lanes, identity | n/a (core) |
| `PaneNameSource` | src/cli/follow.rs:173 | pane names, current pane, seat labels for follow/irc nicks | `NativeCli` follow.rs:184 |
| `PaneShellProbe` | src/harness/launch.rs:20 | the pane shell's exported env for launch | `SystemShellProbe` launch.rs:112 |
| `HandoffLauncher` | src/cli/handoff.rs:341 | handoff launch driver | `NativeLauncher` handoff.rs:2015 (holds `&dyn HostPort`, `LaunchParts` cli/launch.rs:573-583) |
| `ArchivalHosts` | src/service/archival.rs:32 | host lookup for archival | `HostSessions` archival.rs:43 |
| `MachineLister` | src/sync/discovery.rs:119 | peer machine discovery | `HerdrCli` (`herdr machine list --json`) discovery.rs:123-175 |
| `ProcessInfo` / `ProcessInfoProvider` | src/ports.rs:4177; src/host/continuity.rs:55 | OS process table (not Herdr) | `HostProcessTable` process_table.rs:15; `KernelProcessInfo` continuity.rs:59 |
| `ComposerPolicy` (per harness) | src/harness/adapter.rs:1379 | parses Herdr `agent.read` detection text / styled screen into a composer read | harness adapters |

Not behind any trait: everything in section 3 (CLI-side `NativeCli::new` construction, env-var reads,
`herdr` subprocess discovery, install scripts, the `HostLocator`-is-a-socket-path rule, schema).

## 2. Map by category

### (a) Identity / caller location

| file:line | Needs from host | Behind trait? |
|---|---|---|
| src/cli/mod.rs:311 `run()` reads `HERDR_PANE_ID` | the calling pane id (opaque `HostTargetId`) | No: raw env read |
| src/cli/hook.rs:248-262 `HookEnv::from_process` (`HERDR_ENV==1`, `HERDR_PANE_ID`) | "am I in a Herdr pane" + pane id | No |
| src/cli/hook.rs:1463, :1790 (`connect_pane`), :3485, :4019, :4392-4400 (`foreign_session`) | gate: every hook path no-ops / quiet-fails without `HERDR_ENV` + pane | No: hard-coded Herdr gate, text "not running inside a Herdr pane" |
| src/cli/hook.rs:4364-4368 `hook_runtime_context` reads `HERDR_SOCKET_PATH` | claim host = pane's own server socket | No |
| src/cli/instance.rs:342-349 `claim_host(explicit, pane_env, installed)` | precedence flag > pane `HERDR_SOCKET_PATH` > installed argv | No (pure fn) |
| src/cli/mod.rs:3766-3795 `located_pane_seat` -> `NativeCli::new(..).current_pane` (`pane.current`) | moved-pane follow: old pane id -> current id | No: concrete `NativeCli` |
| src/cli/hook.rs:1620-1639 same moved-pane fallback in hooks | same | No: concrete `NativeCli` |
| src/cli/mod.rs:392-412 `pane_agent` (`me init`) `NativeCli::observe_pane_agent` (`agent.get`) | Herdr's agent record for the pane | Trait method, concrete construction |
| src/protocol/authority.rs:8-29 `CallerClaim{host: HostLocator, target: HostTargetId, ..}` | claim names a host socket + pane | Type-level: shape is Herdr-shaped |
| src/protocol/ids.rs:300-345 `HostLocator::parse` | host identity == canonical absolute filesystem path ("Hides: that a server is identified by a filesystem path") | Leaf type; rule is socket-path-specific |
| src/service/dispatch.rs:1523 + src/service/host_sessions.rs:526-600 `resolve`/`attach_locked` | lazy attach of claim's host via `HostFactory` | Yes (`HostFactory`) |
| src/store/seats.rs:4904 `cooperative_claim`, :5131-5307 `cooperative_mapping` (from 00) | claim target must match `effective_observation(host,target)` boot/epoch | Store logic; needs host observations to exist |
| src/identity/repair.rs:254, :452, :502, :548, :568 `OrdinaryIdentity` | `observe_current_target`, `observe_pane_agent`, `observe_bootstrap_target` for seat creation | Yes (`HostPort`, `BootstrapObserver`) |
| integrations/hermes/__init__.py:37, :340-350 `PANE_ENV=("HERDR_PANE_ID","HERDR_ENV")` | Hermes gateway forwards the client's pane env | No (Python, Herdr names) |
| integrations/skill/SKILL.md:3, :27, :185 | model-facing contract: "Requires running inside your own Herdr pane (HERDR_PANE_ID)" | Docs |
| src/harness/caller_env.rs:277-338 `derive` | harness session from process ancestry + harness env | OS, not Herdr (`ProcessInfo`) |

### (b) Topology / observation

| file:line | Needs | Behind trait? |
|---|---|---|
| src/identity/reconcile.rs:538 `host.enumerate_targets` (observation lane, src/service/workers.rs:2435) | coherent full snapshot (`session.snapshot`) of targets with boot/epoch/sequence/terminal/incarnation | Yes (`HostPort`) |
| src/ports.rs:735-750 `HostObservation{host_boot, epoch, generation, terminal, occupancy, ui, focused,..}`; :897-907 `HostSnapshot{boot, epoch, observation_sequence, complete, enumeration, incarnation}` | host must supply boot id, monotone epoch/sequence, terminal id + incarnation | Trait types, but semantics are Herdr's server model |
| src/host/native.rs:1031-1130 `dispatch` method table | Herdr JSON API: `pane.current`, `pane.get`, `pane.process_info`, `session.snapshot`, `agent.get`, `agent.read`, `pane.send_keys`, `pane.send_text`, `pane.read`, `agent.prompt`, `agent.send_keys`, `agent.start` (+ `tab.create`, `ping` in transport.rs:186, :226, :406) | Inside the adapter |
| src/host/observation.rs:12-563 `normalize_*` (snapshot, pane, names, seat labels, pane agent, topology, current pane) | Herdr response JSON shapes; `agent_status` in idle/working/blocked/done/unknown (:129-132) | Inside the adapter |
| src/host/compatibility.rs:1-20 | Herdr release floor = `min_herdr_version` (herdr-plugin.toml:4 `0.9.1`) | Inside adapter |
| src/host/continuity.rs:19-30, :107-127 `LocalEndpointWitness` (SO_PEERCRED / proc_pidinfo on the socket peer) | server incarnation witnessed from a unix socket peer | Adapter-internal, but `CreateTabRequest.expected_witness` (ports.rs:5552) exposes it in a port type |
| migrations/node/0001_node_baseline.sql:54-66 `herdr_hosts` keyed by `endpoint`; `observed_targets`, `snapshot_targets`, `occupant_bindings.host_boot NOT NULL` (from 00) | every local seat/binding hangs off a host row with boot/epoch | Schema (41 refs in src/store/seats.rs, 15 in topology_handoff/attempts.rs, 10 hosts.rs, ...) |
| Pane close -> retirement src/store/seats.rs:2950, src/store/control/retirement.rs (from 00) | absence from a complete snapshot = target closed | Driven by observation lane |
| src/cli/mod.rs:765-780, :870-874 `host()?.topology(..)` closures into `seat_resolution` (src/cli/seat_resolution.rs:469-671) | live topology for `--space/--tab/--pane` selectors | Closure-injected, concrete `NativeCli` (mod.rs:705-712) |
| src/cli/topology_runtime.rs:388, :438, :482, :547, :954, :1047 | topology + bootstrap observation for topology handoff/recover | Concrete `NativeCli` |

### (c) Wake / input typing

| file:line | Needs | Behind trait? |
|---|---|---|
| src/notification/dispatch.rs:126, :239, :324, :373-375, :605, :618 | `pane_agent_state`, `observe_current_target`, `foreground_pids`, `safe_wake_target`/`safe_poke_target`, `submit_prompt_if_owed`, `clear_own_wake_text` | Yes (`HostPort`) |
| src/host/native.rs:53 `WAKE_READY_STATUSES=["idle","done"]`; :1845 `focused_empty_ready`; :1907 `submit_prompt_mode`; :2130-2230 composer observation | Herdr agent status + composer reads before typing | Inside adapter |
| src/host/native.rs:1478 composer via `agent read --source detection`; :2295 styled `recent`; :2339 clear; :1485-1611 stash/restore | screen text of the agent's pane | Adapter, parsed by per-harness `ComposerPolicy` (adapter.rs:1379) |
| src/host/native.rs:1302 `foreground_pids` (`pane.process_info`) | pids in the pane's foreground (composer wake of seats with wake targets) | Yes |
| src/harness/adapter.rs:485-497 `DeliveryPolicy` idle_wake Native/Channel; src/harness/registry.rs:1447 `has_idle_wake` | whether wake is typed by the host (Native) or self-submitted (Channel, Claude mod) | Yes (harness policy); `Native` presumes a host that can type |
| src/store/mod.rs:1275 `wake_candidates(host)`; src/store/wake.rs:1271 `BATCH_SEATS_SQL s.host_id=?4` (from 00) | wake scheduling is per attached host | Store; per-host lanes in `HostSession::wake` (host_sessions.rs:281) |
| src/service/dispatch.rs:281 `host.port().wake_block(..)` | last wake block diagnostic | Yes |

### (d) Labels / display

| file:line | Needs | Behind trait? |
|---|---|---|
| src/cli/mod.rs:981-995 `NativeCli::new(..).seat_labels` for `seat list`; `peer_source` NativeCli for participant locations | workspace/tab/pane labels (`session.snapshot`) | Trait method exists; concrete construction |
| src/cli/peer_locations.rs:3-46 thread-local `Option<NativeCli>`, `participant_labels` | labels for participant rows | No: typed to `NativeCli` |
| src/cli/follow.rs:184-192, :230 `NickCache` | pane names -> nicks | `PaneNameSource` trait; `NativeCli` built in `NickCache::new` |
| src/cli/mod.rs:3990-4000 launch: agent named after pane label (`pane_names`) | pane label | Concrete `NativeCli` |
| src/cli/handoff.rs:2264, :2287 handoff `seat_labels` | labels | Concrete `NativeCli` |
| src/host/observation.rs:283-292 `SeatHostLabels{workspace_id, workspace_label, tab_id, tab_label, pane_label, terminal, incarnation}` | three-level Herdr hierarchy | Port type is Herdr-shaped |
| src/service/seat_publisher.rs:1-17, :202 `session.port().seat_labels` -> peer `seat_status` | labels published to peers | Yes (`HostPort`) |
| src/cli/human.rs:208-302 (from 00), :270, :293 | renders `space/tab/pane` | Display only |

### (e) Launch / handoff (creating panes/tabs)

| file:line | Needs | Behind trait? |
|---|---|---|
| src/harness/launch.rs:754, :807, :876, :910, :999 | `observe_pane_agent`, `observe_current_target`, `launch_native_with_evidence` (`agent.start --kind --pane`) into an existing shell pane | Yes (`HostPort` via `LaunchParts.host: &dyn HostPort`, cli/launch.rs:575) |
| src/ports.rs:2084 `NativeLaunchCapability{HostGuardedStart, Unsupported}` | static launch capability | Yes |
| src/harness/adapter.rs:504 `host_kinds`, :1377 `expected_host_kinds`; src/harness/registry.rs:1538 `by_host_kind` | Herdr agent `kind` strings per harness | Harness metadata keyed by Herdr's kind vocabulary |
| src/harness/launch.rs:20-40 `PaneShellProbe` ("Herdr's `agent.start` carries no environment") | pane shell env | Trait; rationale Herdr-specific |
| src/cli/topology_handoff.rs:749-1027, src/cli/topology_handoff/automatic.rs:332-531 `native.create_tab` (`N: CreateTabPort`) | create tab in a workspace, then bootstrap-observe its root pane | Yes (generic over `CreateTabPort`), built as `NativeCli` in topology_runtime.rs:388 |
| src/cli/handoff.rs:2254-2280 `NativeLauncher` with `NativeCli::new(claim_host)` | handoff to new/existing pane | Trait use, concrete construction |
| src/cli/peer_launch.rs:38-60 `request_witnessed(endpoint,"ping")` then `NativeCli::create_tab` | peer-requested new tab; raw Herdr transport call outside the adapter type | No: calls `crate::host::request_witnessed` directly |
| src/service/peer_launcher.rs:582-605 `tab_args` / `launch_args`; :777-800 child `herdr-threads --state-dir --host-endpoint ...` | runs local CLI with an explicit Herdr socket | Peer launch requires an attached Herdr host (:258, :270 `live_topology`) |
| src/ports.rs:5544-5560 `CreateTabRequest{workspace, cwd, label, focus, env, expected_witness}` | Herdr tab semantics + socket witness | Port type, Herdr-shaped |

### (f) Install / plugin registration / state-dir discovery

| file:line | Needs | Behind trait? |
|---|---|---|
| herdr-plugin.toml:1-55 | Herdr plugin manifest: build, `startup` daemon ensure, workspace actions, `operator` overlay pane | Herdr-only artifact |
| scripts/view.sh:4-20 | requires `HERDR_PLUGIN_STATE_DIR`, `HERDR_SOCKET_PATH`; `open` execs `$HERDR_BIN_PATH plugin pane open` | Herdr-only |
| scripts/install.sh:29, :161-220, :520-537, :850-869 | `herdr plugin list/link/unlink/action invoke/log list`; `--no-herdr` opt-out (src/cli/update.rs:310, :356) | Shell; Herdr optional via `--no-herdr` |
| src/cli/instance.rs:1-25, :70-82 `InstanceInputs::from_process` (`HERDR_PLUGIN_STATE_DIR`, `HERDR_SOCKET_PATH`, `HERDR_SESSION`/`HERDR_CONFIG_PATH` => non-default server) | where the node lives and which server | No |
| src/cli/instance.rs:99-107 `plugin_state_dir` = `$XDG_STATE_HOME/herdr/plugins/herdr-threads` | state dir is under Herdr's plugin root | No |
| src/cli/instance.rs:246-282 `plugin_registry` reads `~/.config/herdr/plugins.json`; :287 `leftover_state_dir` | plugin installed/enabled check | No |
| src/cli/instance.rs:307-321 `default_host_endpoint` = `~/.config/herdr/herdr.sock` (must be a socket) | default host | No |
| src/cli/setup.rs:254-339 `herdr_json`: `herdr plugin list --json`, `herdr status server --json` | fallback discovery subprocesses | No |
| src/cli/instance.rs:384-390 error "use --state-dir or HERDR_PLUGIN_STATE_DIR"; state dir is required, host optional (:391-394) | | No |
| src/daemon/paths.rs:123-145 `RuntimeContext::from_environment` (`HERDR_PLUGIN_STATE_DIR`, `HERDR_SOCKET_PATH`, `HERDR_BIN_PATH`) | hook-scoped context | No |
| src/cli/hook.rs:449-456 `hook_state_dir` | `--state-dir` else `HERDR_PLUGIN_STATE_DIR` | No |
| src/harness/setup.rs:255-331; `cli::hook::installed_argv` | installed hook argv bakes `--state-dir`/`--host-endpoint` | No (argv format) |
| src/main.rs:45, src/daemon/lifecycle.rs:386-388 | `HERDR_BIN_PATH` forwarded to the daemon | No |
| src/app.rs:2040-2051 | daemon runs "store-only" when no Herdr server attached | Already tolerated |

### (g) Liveness / process

| file:line | Needs | Behind trait? |
|---|---|---|
| src/harness/caller_env.rs:240-266 `fingerprint_for` -> `LivenessFingerprint{pid,start}` | OS process table only | Yes (`ProcessInfo`), Herdr-free |
| src/service/liveness.rs (node-wide worker); src/store/bindings.rs:709 `liveness_candidates` JOIN `herdr_hosts` | binding liveness; selection requires a host row | Store predicate host-keyed |
| src/host/native.rs:1302 `foreground_pids`; :724 `started_process`, :754 `early_exit_output` (`pane.read`) | process facts reported by Herdr for the pane | Yes / adapter |
| src/daemon/idle_exit.rs:29-39 `active()` = `hosts.attached_count()!=0` || links || clients | an attached Herdr host keeps the daemon up | Core, counts `HostSessions` |
| src/host/continuity.rs:107-127 peer witness of the Herdr server process | Herdr server identity across restarts | Adapter |

### (h) Peer wire / addresses

| file:line | Needs | Behind trait? |
|---|---|---|
| src/sync/frame.rs:412-430 `SeatStatusEntry{workspace?, tab?, pane?}` (cap SEAT_STATUS_CAP :52-55) | Herdr 3-level labels on the wire | Wire (optional fields, version-tolerant) |
| src/sync/frame.rs:560-600 `LaunchPlan` / `LaunchTarget::{Pane{space,tab,pane}, NewTab{space,label}}` | remote launch addresses Herdr panes/tabs | Wire (`#[serde(other)] Unknown`) |
| src/protocol/address.rs:1-40 readings `space/tab/pane`, `split_machine` (:791 per 00) | address grammar is Herdr hierarchy | Pure; grammar Herdr-shaped |
| src/service/remote_resolver.rs:131-160 `session.port().live_topology` (`AddrResolver` :294) | answer `addr.resolve.v1` from attached hosts' live topology | Yes (`HostPort`) |
| src/service/peer_launcher.rs:258-270 target must exist on one attached host | | `HostPort.live_topology` |
| src/sync/discovery.rs:119-175 `HerdrCli` `herdr machine list --json` (`HERDR_BIN_PATH`) | peer machine discovery from Herdr | Yes (`MachineLister`), Herdr-only impl; `NotAvailable` tolerated |
| src/sync/facts/seat.rs:20 `seat.created{display_name}` reserved null (from 00) | named seats need it | Wire |

## 3. Concrete `NativeCli` construction outside the daemon factory (the bypasses)

The daemon builds hosts only through `HostFactory` (src/main.rs:71). The CLI constructs `NativeCli` directly
from `RuntimeContext::require_host()` at these sites, so a non-Herdr environment needs a CLI-side factory too:

| Site | Purpose | Category |
|---|---|---|
| src/cli/mod.rs:398 | `me init` pane agent read | a |
| src/cli/mod.rs:705-712 | topology + current pane for seat selectors | a, b |
| src/cli/mod.rs:986, :995 | seat labels, participant locations | d |
| src/cli/mod.rs:3784 | moved-pane follow (CLI) | a |
| src/cli/mod.rs:3990 | `launch`: pane label naming + launch | d, e |
| src/cli/hook.rs:1631 | moved-pane follow (hooks) | a |
| src/cli/handoff.rs:2264 | handoff launcher + labels | d, e |
| src/cli/peer_launch.rs:38-48 | peer tab create (+ raw `request_witnessed` ping) | e, h |
| src/cli/topology_runtime.rs:388, :954, :1047 | topology handoff / recover | b, e |
| src/cli/follow.rs:230 | follow/irc nick cache | d |
| src/cli/peer_locations.rs:15-23 | thread-local typed `NativeCli` | d |

Herdr subprocesses (not via the socket): `herdr plugin list --json`, `herdr status server --json`
(src/cli/setup.rs:284, :325), `herdr machine list --json` (src/sync/discovery.rs:154), `herdr plugin pane open`
(scripts/view.sh:12), installer `herdr plugin link/unlink/list/action/log` (scripts/install.sh).

## 4. Observations for the abstraction

- The daemon core is already host-neutral at the call level: every daemon-side consumer goes through
  `Arc<dyn HostPort>`/`BootstrapObserver` from `HostSessions`. A second host kind (named host, herdr-app
  terminal) can enter via `HostFactory.build` if `HostLocator` can name it; today `HostLocator::parse`
  admits only an absolute path and `exists` tests for a unix socket (src/main.rs:76-79).
- The CLI/hook side is not: Herdr presence is detected by env (`HERDR_ENV`, `HERDR_PANE_ID`,
  `HERDR_SOCKET_PATH`) at src/cli/mod.rs:311 and src/cli/hook.rs:255-262, and every hook path gates on it
  (hook.rs:1463, :1790, :3485, :4019, :4393). One "caller environment" resolver returning
  `{host locator, target, kind}` would replace these reads.
- `HostPort` mixes three concerns that environments support unevenly: identity/observation
  (all), typed wake/composer (Herdr TUI only; named host: none, per named-seats design), and topology
  creation/labels (Herdr TUI; herdr-app maybe (inferred)). Most methods already have inert defaults
  (`seat_labels`, `foreground_pids`, `wake_block`, `stash_composer`, `observe_pane_agent`); the required ones
  for a non-typing host are `observe_current_target`, `enumerate_targets`, `live_topology`,
  `observe_current_target_for_archival`, `safe_wake_target`, `submit_prompt`, `launch_native`,
  `pane_agent_state`, `send_submit_key`, `native_launch_capability` (src/ports.rs:3907-4130).
- Herdr-shaped types leak into ports and wire: `SeatHostLabels` (workspace/tab/pane), `HostTopology`
  (spaces/tabs/panes), `CreateTabRequest.expected_witness` (socket peer witness), `LaunchTarget` and
  `SeatStatusEntry` labels. A non-Herdr host can return empty/None for these (inferred: renderers already
  tolerate absent labels, src/cli/peer_locations.rs, src/cli/human.rs).
- Store invariants assume every local seat/binding has a `herdr_hosts` row with boot/epoch and
  observations (`occupant_bindings.host_boot NOT NULL`, `cooperative_mapping` effective observation match).
  The named-seats design satisfies this by giving the named host a row and synthesized observations.
- State-dir discovery is Herdr-plugin-rooted (`$XDG_STATE_HOME/herdr/plugins/herdr-threads`), but works
  without Herdr when the default directory exists or `--state-dir` is given; host discovery returns None
  without Herdr (src/cli/instance.rs:391-394) and the daemon runs store-only (src/app.rs:2040-2051).
- Liveness is OS-based and Herdr-free except for the `herdr_hosts` JOIN in `liveness_candidates`.
- Harness metadata uses Herdr's agent `kind` vocabulary (`host_kinds`, `by_host_kind`) for launch and
  composer evidence; an environment without `agent.start` reports `NativeLaunchCapability::Unsupported`.
