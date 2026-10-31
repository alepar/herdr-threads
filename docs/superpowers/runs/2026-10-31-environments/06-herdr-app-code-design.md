# herdr-threads on herdr-core / the Herdr app: code design

Behaviour source: `05-herdr-app-vision.md` (draft; its **[decide]** items are still recommendations).
This document lays out the code that implements it. It is shaped like `02-named-seats-code-design.md`
and builds on it, so the two can be merged; section 9 lists the merge points. Inputs: 00, 01, 03, 04
(the hc 0.1.15 / herdr-app 0.1.10 real-install spikes override research inferences), and the four seam
studies (observation, wake, caller location, launch/labels/discovery).
Trees: herdr-threads `.worktrees/external-seat` at badccf481 (`src/...`, repo-relative); herdr-core
dc276b7 (`C:` = `herdr-core/crates/herdr-core/src/`). "(inferred)" marks claims not traced end to end.

The composer question is **settled**, not open: spike 2C (04-mac-spikes.md, "Spike 2" C) found that
`hc send --if idle` appends the wake text to a typed draft and submits both, because Claude and Codex report
`idle` while a draft sits in the composer. The design below is built on that outcome (branch B). Branch A, a
future hc that refuses or holds drafts, changes only one accepted limit (section 5.4).

## 0. Conflicts

### 0.1 With the vision (rulings wanted)

| # | Vision text | What code or spikes require | Proposed resolution |
|---|---|---|---|
| V1 | D1: "`removed` (or `finished` of a non-kept terminal) is closure evidence" | `finished` is final for every terminal. A kept terminal that exits with a code or is killed is `finished` too; only holder loss, restart, stop and shutdown keep it `live` (C:terminals.rs:20-21, :323-344). `removed` always follows `finished` (C:api.rs:288-300; C:terminals.rs:368-395). | Closure = the terminal is `finished`, or absent from a complete capture of the same core id. Kept or not does not matter. |
| V2 | D1/R6 (01): incarnation keyed on `HC_SCOPE` or pid | `HC_SCOPE` changes on core stop+start (04 step 3), and the TUI-shaped incarnation check unresolves every seat when it differs (src/identity/reconcile.rs:241-265, C2). | Boot = incarnation = `hc-core:<core.json id>`. The binding, not the seat, tracks the process (liveness fingerprint, C5). Keep recovery keeps the seat; the old binding ends on liveness. |
| V3 | D3: verification by "tty/pid evidence" for every hc caller | hc exposes no tty (04 spike 2A); hooks have no controlling tty (01 TL;DR 4); TRUST-POLICY forbids adversarial verification (AGENTS.md "Trust policy"). | Verify only when env names targets on **two** hosts (both `HERDR_PANE_ID` and `HC_TERMINAL_ID`). Evidence = the host-reported root pid of the target is the nearest ancestor of the caller's harness process. A single-family env stays unverified, as A1 accepts today (section 6.1). |
| V4 | D4: `terminal.send if:idle` is "race-free" | Race-free against `working`/`blocked`, not against drafts (spike 2C). `if` accepts only `idle`/`working`, not `done` (C:api/types.rs:68-71). | Pre-send composer read from `terminal.read` text through the harness `ComposerPolicy`; refuse on a draft or an unreadable screen. The read-to-type window is a TRUST-POLICY accepted limit. `done` gets no wake (diagnostic `StatusDone`) pending spike W2. |
| V5 | D5: "status and liveness from events" | Producer contract forbids serving enumeration identity from a cache (src/ports.rs:760-764). Binding liveness is the OS fingerprint (src/harness/caller_env.rs:240-266), never host-reported. | Status is read on demand (`terminal.get`) at wake time. Snapshots are one-shot captures. Events are only dirty hints, and they come in phase 2. Liveness stays OS-based. |
| V6 | D7: handoff via `terminal.run` with labels | A launched harness must not start before its recipient seat exists (TRUST-POLICY.md:324, the bootstrap Send is staged before the launch). `terminal.run` cannot unset leaked variables (C:platform/unix.rs:42-47 `env.extend`). | `terminal.run` starts a **gate shim** (`herdr-threads internal launch-gate`). The shim scrubs the env, waits for the daemon to authorize its (core, tN), then `exec`s the harness. In-place launch into an existing terminal is unsupported (section 7). |
| V7 | D2: locator also "discovered under `~/.local/state/herdr-core/*/`" | `hc server status` autostarts a core on macOS (04 step 1). A core started by ht freezes ht's env into every future terminal (04 step 3). | Discovery reads `core.json` and the socket, then sends `hello`. It never runs `hc`. It is used only when no caller site names a core (plain-shell handoff, peer `place()`). |
| V8 | D6: names "from the app" | `where` is ambiguous when a name contains " / " (04 E). | Ids from `herdr.space`/`herdr.tab` labels; names from doc `herdr-app` (tolerant parse, cached by `doc` version). `where` is only a fallback. |

### 0.2 With the code (implementation-only)

| # | Fact | Resolution |
|---|---|---|
| K1 | TRUST-POLICY vocabulary: "Herdr boot / incarnation: one Herdr server process" (TRUST-POLICY.md:121-123). `tN` is durable across core processes (C:terminals.rs:15-16, :203-250; 04 step 3). | Vocabulary becomes per host kind. For hc, a core restart is "same boot, newer epoch", giving `CarryForward` (reconcile.rs:309-328), not C2. |
| K2 | `Events.seq` is in memory and resets to 0 per core process (C:events.rs:16-20). `has_coherent_order` needs a sequence (src/ports.rs:909). | `observation_sequence` is the adapter's own counter, as in NativeCli (src/host/native.rs:822). Core seq is used only to drop stale events. |
| K3 | `CreatedTab::validate` / `BootstrapPaneObservation::try_new` hard-code `herdr-server:` incarnations and `<ws>:` ids (src/ports.rs:~1380-1425, :5571-5600). Journaled records use `deny_unknown_fields` with frozen bytes. | New V3 sibling records (`CreatedTarget`, `HostWitness`). V1/V2 bytes stay unchanged. |
| K4 | CLI joins labels by **target only** (src/cli/human.rs:52-61), and `SeatSummary` has no host (src/protocol/results.rs:1085-1106). Two cores both own `t1`. | Labels come from the daemon, keyed `(host_id, target)`, through the already-present `SeatDirectory` (results.rs:1101-1105). This fixes a latent defect even before hc exists. |
| K5 | A daemon autostarted inside an hc terminal inherits `HC_SCOPE` (spawn keeps env, src/daemon/lifecycle.rs:356-390). hc sweeps every process carrying the terminal's scope when it ends; `setsid` does not escape (C:sweep.rs:1-4, :95-131). | `spawn_detached` removes `HC_SCOPE`, `HC_TERMINAL_ID`, `HC_TERMINAL_SOCKET` (and `HERDR_PANE_ID`, R8b in 01). Phase 0 fix. |
| K6 | `ObservationLane::mark_event_dirty` has no production caller (src/identity/reconcile.rs:795). The lane polls every 5 s (src/service/workers.rs:52). | Polling is enough for v1. Phase 2 adds a neutral `take_observation_hint` that the worker calls. |
| K7 | Wake submit budget is 2 s (src/host/native.rs:57; src/notification/dispatch.rs:39). An hc send can block about 13 s: settle 5 s + quiet 3 s + 0.2 s gap + started window 5 s (C:input.rs:19, :64-71, :190-210). | The budget becomes a host wake capability field (section 2.3). |
| K8 | `ClosureEvidence`/`control::begin_retirement` (src/ports.rs:662-667; src/store/control/retirement.rs:19-110) has no production constructor (inferred). | Do not build hc closure on it. Use the publication-absence path (src/store/seats.rs:2950-2963). |

## 1. Shape of the change

An hc host is a third host kind behind the existing seams. The daemon dials it, observes it, wakes
through it and labels from it. It goes in through `HostFactory`, so every daemon consumer that already
uses `Arc<dyn HostPort>` (03 §4) runs unchanged. There are four new seams:

1. **`HcHost`** (daemon, `src/host/hc/`): a second dialled `HostPort` + `BootstrapObserver` +
   `CreateTargetPort` implementation over hc's JSON-RPC socket. It lives in `HostSessions.sessions`
   (unlike the named host), so idle exit, the reattach sweep and the cap count it.
2. **`CallerSite` extension** (CLI + hooks): `CallerLocator::Terminal{socket, id}` and the
   multi-family `Located::Ambiguous`, resolved by the daemon (`LocateCaller`).
3. **`WakeCapability`** (ports): replaces named-seats' `has_composer() -> bool` with the traits
   the dispatcher and registry need (focus known, stash support, submit budget).
4. **`CreateTargetPort`** (ports): a host-neutral successor to `CreateTabPort` for launch/handoff,
   with V3 journal records.

Module map:

| Module | Change |
|---|---|
| `src/host/hc/mod.rs` (new) | `HcHost: HostPort + BootstrapObserver + CreateTargetPort` |
| `src/host/hc/rpc.rs` (new) | JSON-RPC 2.0 line client: `hello`, `terminal.get/read/send/keys/run/list`, `events.subscribe`, `doc.get`; one connection per call; the peer witness reuses `LocalEndpointWitness` (src/host/continuity.rs:107-127) |
| `src/host/hc/wire.rs` (new) | tolerant serde types (`HcTerminal`, `HcStatus`, `HcNotify`, `HelloReply`, `AppDoc`), no `deny_unknown_fields` |
| `src/host/hc/events.rs` (new, phase 2) | long-lived subscription per host, producing hints |
| `src/ports.rs` | `HostKind::Hc`; `WakeCapability`; `foreground_contains`; `target_root_process`; `take_observation_hint`; `CreateTargetPort` + `Placement` + `HostWitness` + `CreatedTarget`; `ProcessInfo::process_group` |
| `src/host/process_table.rs` | `process_group(pid)` (getpgid) |
| `src/service/host_sessions.rs`, `src/main.rs` | `HostFactory.build` sniffs the kind (Herdr `ping` vs hc `hello`), persists `herdr_hosts.kind` |
| `src/service/locate.rs` (new) | `LocateCaller` verification + cache |
| `src/service/dispatch.rs` | `wake_status` via `idle_wake_on(harness, WakeCapability, channel_live)`; `LocateCaller` arm; Herdr-only "first host" consumers filtered by kind |
| `src/notification/dispatch.rs` | budget from `WakeCapability`; `composer_choice` through `foreground_contains` |
| `src/daemon/transport.rs` | expose the peer pid (tokio `UCred::pid()`) next to uid (:430) |
| `src/daemon/lifecycle.rs` | scrub `HC_*`/`HERDR_PANE_ID` in `spawn_detached` (K5) |
| `src/cli/caller_location.rs` | `HC_TERMINAL_ID`/`HC_TERMINAL_SOCKET`; `Located`; phase-0 guard |
| `src/cli/mod.rs`, `hook.rs`, `watch.rs`, `instance.rs`, `human.rs`, `peer_locations.rs`, `follow.rs` | consume `CallerSite`; labels from the daemon; discovery without `herdr` |
| `src/cli/host_client.rs` (new) | `HostClient` enum {Herdr(NativeCli), Hc(HcClient)} for the remaining CLI-side host uses (launch, handoff, peer target create) |
| `src/cli/launch_gate.rs` (new) | `internal launch-gate` shim |
| `src/cli/topology_handoff*.rs`, `topology_runtime.rs`, `handoff.rs`, `peer_launch.rs`, `src/service/peer_launcher.rs` | generic over `CreateTargetPort`; `peer-target-create` |
| `src/protocol/handoff.rs` | `BootstrapPlan` V3 carrying `CreatedTarget` |
| `src/protocol/results.rs`, `commands.rs` | `Command::LocateCaller`, `Command::HostTopologies`; `WakeBlock::{NoStatus, StatusDone}`; `SeatDirectory` filled for local seats |
| `src/harness/registry.rs` | `idle_wake_on` takes `WakeCapability` |
| `src/harness/adapter.rs` | neutral defaults: `child_scrub_vars()`, `host_launch_argv(kind)` |
| `src/test_support/fake_hc.rs` (new) | in-process fake core (section 10) |
| `scripts/install.sh`, `src/harness/setup.rs`, `integrations/skill/SKILL.md` | standalone install; environment-neutral skill text |
| `TRUST-POLICY.md`, `AGENTS.md` (delivery contract rule 6 wording) | section 8 |

## 2. Types and traits

### 2.1 Host kind (merges with 02 §2.2)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKind { Herdr, Named, Hc }   // herdr_hosts.kind: 'herdr' | 'named' | 'hc'
```

Node-wide code that is Herdr-only reads `kind()` and skips hc sessions. Today that is MachineLister
(`herdr machine list`, src/sync/discovery.rs:119-175), `startup_primary`/archival
(src/app.rs:2040-2052, :2466-2490; src/service/archival.rs:113-126) and bootstrap identity selection
(src/service/dispatch.rs:213-219) (inferred list; audit in phase 1). The Health and idle-exit counts include hc.

### 2.2 `HcHost` (src/host/hc/mod.rs)

```rust
pub struct HcHost {
    socket: PathBuf,                       // HostLocator, canonical
    boot: Mutex<Option<HcBoot>>,           // last core id seen
    witness: Mutex<Option<LocalEndpointWitness>>, // `hc serve` peer pid/start
    epoch: AtomicU64,                      // bumped on transport failure, stale answer, witness change
    sequence: AtomicU64,                   // adapter counter (K2)
    started: Mutex<HashMap<HostTargetId, bool>>, // last send's `started`, for pane_agent_state
    labels: Mutex<Option<(u64, AppDoc)>>,  // doc "herdr-app" by version
    procs: Arc<dyn ProcessInfo>,
    clock: Arc<dyn Clock>,
}
struct HcBoot(String);                     // "hc-core:<core.id>", HostBootId::parse (ASCII, ≤128)
impl HcBoot { fn from_hello(h: &HelloReply) -> Result<Self, ApiError>; }
```

Port behaviour:

| Method | hc host |
|---|---|
| `kind` | `Hc` |
| `enumerate_targets` | Fresh connection: `hello` (core id → boot) then a one-shot `events.subscribe{}` snapshot, then close. The snapshot and subscription are taken under one lock, so the capture is coherent (C:api.rs:547-551). Keep local addresses (no `/`) with `state=="live"` (this includes lost/spawn_pending); drop finished ones (closure, V1) and remote `machine/tN` mirrors (C:events.rs:71-75; `here_only` is not set for us, C:api.rs:530). Result: `complete`, `CoherentVerified`, `Verified{boot, CoherentEnumeration}` (`authorizes_absence_closure`, src/ports.rs:919-931). A failed `hello` or an unparseable id gives `CompleteUnverified`/`Unknown`, which never closes anything (C4 freeze). Never `terminal.list`: it prunes, fans out to machines with 3 s timeouts and is unsequenced (C:api.rs:231-270). |
| `observe_current_target(tN)` | `hello` + `terminal.get{terminal}` on one connection, **never** with `caller` (that mutates hc's parent bookkeeping, C:api.rs:272-275). Check boot after the response, as `check_boot` does (native.rs:2453-2469). `not_found`/finished: not present. terminal = `tN`, generation 1, occupancy/execution Unknown (C5). |
| `observe` UI | from `status.state`: idle/done → Idle, working → ActiveTurn, blocked → ApprovalOrQuestion, else or null → Unknown; `focused=false` |
| `resume_after_epoch(e)` | `epoch = max(epoch, e+1)` (native.rs:817-820) |
| `seat_labels` / `live_topology` | section 6.4 |
| `target_root_process(t)` | `terminal.get.pid` when live (C:terminals.rs:37) |
| `foreground_contains(t, pid)` | `process_group(pid) == terminal.get.foreground.pid` (pgrp leader, C:foreground.rs:12-20) |
| `wake_capability` | section 2.3 |
| `safe_wake_target`, `submit_prompt_if_owed`, `pane_agent_state`, `clear_own_wake_text` | section 5 |
| `stash_composer`/`restore_composer` | default `Unsupported` (hc cannot type without submitting, 04 C) |
| `native_launch_capability` | `HostGuardedStart` for **creation** only (section 7) |
| `launch_native` (in place) | `Err(InvalidArgument "hc terminal tN: launch creates a new terminal; use --new-tab/--space")` |
| `observe_pane_agent` | default `None` (hc has no agent kind) |

### 2.3 `WakeCapability` (replaces 02's `has_composer`)

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WakeCapability {
    None,                                  // named host
    Composer { focus_known: bool, stash: bool, submit_budget: Duration },
}
fn wake_capability(&self) -> WakeCapability {          // HostPort, neutral default = Herdr TUI
    WakeCapability::Composer { focus_known: true, stash: true, submit_budget: Duration::from_secs(2) }
}
// HcHost: Composer { focus_known: false, stash: false, submit_budget: 15 s }
// NamedHost: None
```

The idle gate, the composer read and the submit evidence stay inside each host's
`submit_prompt_if_owed`. They are host mechanics, not dispatcher policy, so they get no enum. The
seam study's six-field struct was cut to the three fields a caller reads.

### 2.4 Small neutral port additions (src/ports.rs)

```rust
// HostPort, all defaulted (no ErrorCode::Unsupported bodies; self-test src/ports.rs:4533-4572)
fn foreground_contains(&self, t: &HostTargetId, pid: u32, cx: &HostCallContext)
    -> Result<Option<bool>, ApiError> { /* default: foreground_pids(t).map(|v| v.contains(&pid)) */ }
fn target_root_process(&self, t: &HostTargetId, cx: &HostCallContext)
    -> Result<Option<u32>, ApiError> { Ok(None) }      // None = no evidence, never "verified"
fn take_observation_hint(&self) -> bool { false }      // phase 2

// ProcessInfo (src/ports.rs:4177), defaulted
fn process_group(&self, pid: u32) -> Option<u32> { None }  // getpgid; inferred permitted cross-session on macOS (spike W4)
```

`composer_choice` (src/ports.rs:2249-2279) asks `foreground_contains(fingerprint_pid)`
instead of scanning `foreground_pids`. TUI behaviour is the same (the default is the scan). On hc this
finds a fingerprint pid under a wrapper leader (aisw `cx`, node shims) and keeps A4's "exact pid in the
foreground process group" rule.

### 2.5 Caller location (extends 02 §2.5)

```rust
pub struct CallerEnv {                // still the only env reader
    pub herdr_env: bool, pub pane: Option<String>, pub pane_host: Option<PathBuf>,
    pub seat_name: Option<String>,
    pub hc_terminal: Option<String>,  // HC_TERMINAL_ID, non-empty
    pub hc_socket: Option<PathBuf>,   // HC_TERMINAL_SOCKET, non-empty
}
pub enum CallerLocator { Pane(HostTargetId), Named(SeatName), Terminal { socket: PathBuf, id: HostTargetId } }
pub enum Located { One(CallerLocator), Ambiguous(Vec<CallerLocator>) }
impl CallerEnv { pub fn locate(&self, mode: EnvMode) -> Result<Option<Located>, LocatorError>; }
```

Precedence (section 6.1) lives in `locate`. `Terminal` without `HC_TERMINAL_SOCKET` is refused; it
never guesses from `HC_SESSION` (hc itself trusts the id only with a matching socket, C:client.rs:245-253).

### 2.6 `CreateTargetPort` (src/ports.rs, beside `CreateTabPort`)

```rust
pub trait CreateTargetPort: Send + Sync {
    fn create_target(&self, r: &CreateTargetRequest, cx: &HostCallContext) -> CreateTargetOutcome;
}
#[serde(deny_unknown_fields)] pub struct CreateTargetRequest {   // local journal, not peer wire
    pub correlation: HostCallId, pub placement: Placement,
    pub cwd: PathBuf, pub label: String, pub focus: bool,
    pub env: BTreeMap<String, String>,
    pub argv: Option<Vec<String>>,          // hc: gate-shim argv; Herdr: None (shell pane)
    pub directory_context: Option<LaunchDirectoryContext>,
    pub expected_witness: HostWitness,
}
#[serde(tag = "k")] pub enum Placement { NewTab { space: HostTargetId }, NewTerminal { parent: Option<HostTargetId> } }
#[serde(tag = "k")] pub enum HostWitness {
    HerdrSocket(LocalEndpointWitness),
    HcCore { core_id: String, peer: LocalEndpointWitness },
}
pub struct CreatedTarget { pub correlation: HostCallId, pub placement: Placement,
    pub target: HostTargetId, pub tab: Option<HostTargetId>, pub terminal: TerminalId,
    pub host_incarnation: HostBootId, pub witness: HostWitness }
impl CreatedTarget { pub fn validate(&self) -> Result<(), &'static str>; } // dispatch on witness kind
```

NativeCli implements it by delegating to `create_tab`, so new plans use one port. `BootstrapPlan`
V3 carries `CreatedTarget`; `BootstrapPaneObservation::try_new` takes `HostWitness`.

## 3. API changes

### 3.1 CLI

- Env: `HC_TERMINAL_ID`, `HC_TERMINAL_SOCKET` are read; no new flag.
- Errors (stderr, CLI):
  - both families present and neither verifies: `herdr-threads: caller is in Herdr pane P and herdr-core terminal tN; neither verified: set HERDR_THREADS_SEAT_NAME or unset the stale one` (about 130 bytes);
  - phase 0: `herdr-threads: runs in herdr-core terminal tN; not supported yet; set HERDR_THREADS_SEAT_NAME` (about 95 bytes).
- Hooks: exit 0, one stderr diagnostic, plus one SessionStart context line (same text, about 110 bytes);
  nothing on per-turn hooks (token rule (f)).
- Seat-name leak notice (section 6.1): `herdr-threads: HERDR_THREADS_SEAT_NAME may be inherited from <host>; unset it there`, stderr, once per process, CLI only.
- `handoff`/`launch` into hc: `--space <app space>` (`herdr.space` label), `--new-tab`, and a new
  `--new-terminal` (no app labels, parent = caller). In-place `--pane tN` is refused (2.2).
- `internal launch-gate --bootstrap <compound>:<attempt> -- <argv>` and `internal peer-target-create`
  (replaces `peer-tab-create`; the old name stays as an alias for one release, inferred need).
- `human handoff recover ... --created-pane tN`: shows the `ht.bootstrap` correlation label as a hint (section 7).

### 3.2 Local protocol (daemon socket)

| Item | Change | Compatibility |
|---|---|---|
| `Command::LocateCaller { candidates: Vec<CandidateSite>, subject: Option<LivenessFingerprint> }` → `LocateReply::{Verified{index, root}, Unverified{per_candidate: Vec<CandidateVerdict>}}` | new; `CandidateVerdict = NotAncestor \| HostUnreachable \| TargetGone` | sent only when there are ≥2 candidates, so an old daemon is affected only in the conflict case (the existing upgrade error) |
| `Command::HostTopologies` | live topology of every attached host plus named, each tagged with its host locator. CLI selectors pool them (generalizes 02's `NamedTopology`). | new command |
| `SeatSummary.directory` / `ParticipantLocation` labels | filled for **local** seats from `read_labels` by `(host_id, target)` (src/service/seat_publisher.rs:195-225) | field exists (results.rs:1101-1105); old CLIs keep their own lookup |
| `WakeBlock::{NoStatus, StatusDone}` | new variants | `WakeBlock` has `#[serde(other)] Unknown` (results.rs:1745) |
| `CallerClaim` | no change: `host` = core socket, `target` = `tN` | — |

### 3.3 Store schema

- `herdr_hosts.kind` (named-seats' NodeDelta 12) admits `'hc'` **from the start**: define the CHECK
  as `kind IN ('herdr','named','hc')`, or with no CHECK and validation in code (02 §9). Either way, no
  later table rebuild.
- No other column. `retirements.closure_evidence` stays `restored_pane_closed` only (migration 0009);
  hc closure is the plain `Closed` path. Location evidence is not persisted in v1 (no authority, no reader).
- Handoff journal: V3 `BootstrapPlan`/`BootstrapProgress`/`BootstrapAttachment` with `CreatedTarget`;
  V1/V2 decode unchanged.

### 3.4 Peer wire

v1: **no change.** `SeatStatusEntry{workspace,tab,pane}` (src/sync/frame.rs:412-430) carries app
space/tab/pane labels. `LaunchTarget::NewTab{space,label}` (frame.rs:560-600) maps to `herdr.space=<space>`
plus a fresh `herdr.tab`. A later `LaunchTarget::NewTerminal{label}` would need
`// cap: PEER_LAUNCH_TERMINAL_CAP` and a `pub const` in frame.rs (scripts/check-wire-compat); not in v1.
Remote hc terminals (`machine/tN`) are never attached; they belong to that machine's node (vision D8).

## 4. Daemon flow

### 4.1 Attach and kind

`resolve(claim.host)` → `HostFactory.build(locator)`: connect and send one line `{"jsonrpc":"2.0","method":"hello"}`.
A reply with `core.id` and `generation` means `HcHost`; otherwise it falls back to today's Herdr `ping`
(inferred: a TUI server answers a JSON-RPC line with an error, not a hang; spike D1). The kind is persisted
on first attach and trusted on reattach, with a re-sniff if the socket stops answering its kind. `exists`
(is a unix socket) is unchanged. `MAX_ATTACHED_HOSTS` = 16 covers several cores (04 D: several per user).

### 4.2 Resolve, claims, mapping, liveness

Unchanged code over a new port:

1. `ResolveSeat{host=core socket, target=tN}` → `OrdinaryIdentity::resolve` (src/identity/repair.rs:254-282) → `HcHost::observe_current_target`;
2. publish → allocation;
3. `cooperative_mapping` (src/store/seats.rs:5131-5307) against the lane snapshot;
4. liveness through the OS fingerprint.

`moved_target` is `None` for hc: `tN` never moves, so `Move` is unreachable (inferred).

### 4.3 Reconcile outcomes

| Event | Outcome |
|---|---|
| core restart / `server restart` / stop+start / `hc update` | same boot, newer epoch → `CarryForward` |
| keep recovery (fresh shell, same tN) | target present → no structural change; the old binding ends on liveness |
| terminal finished / removed | absent from a complete capture → `BeginRetirement{Closed}` (seats.rs:2950-2963) |
| core wiped/cloned (new core id, `t1` reused, C:server.rs:91-113) | new boot → C2 `MarkUnresolved`; restored-pane chain applies (accepted limit: it watches unrelated reused `tN`, section 8) |
| core down / socket refused | missing evidence → frozen (C4) |

## 5. Wake and the delivery contract

### 5.1 `submit_prompt_if_owed` on hc (in order)

1. `terminal.get` gives the fence: core id, `tN`, status, `turn` T0. If status is not reported: `NoStatus`. If `done`: `StatusDone`. If `blocked`/`working`: refuse.
2. `terminal.read` → bound harness `ComposerPolicy::read` (src/harness/adapter.rs:1379). Proceed only on `Empty`. `Text`/`Unreadable`: refuse (`TargetUnsafe`), nothing typed.
3. `still_owed()`.
4. `terminal.send{text: wake_text, if: "idle"}`. Never `force`.
5. Map the result:
   - `status_mismatch`/`blocked`: refusal ("nothing was typed" is the core's guarantee, C:input.rs:162-189);
   - timeout or disconnect after issue: `OutcomeUnknown`;
   - success: cache `started`;
   - `reply.turn != T0`: diagnostic only (rule-4 gap, 5.3).

Other methods:
- `safe_wake_target`: `Some` only when status is reported and idle.
- `pane_agent_state`: `started=true` → `Submitted`. `started=false` → reread the composer: exactly the wake text → `HoldingPrompt`, otherwise `Unknown`.
- `clear_own_wake_text`: when the policy opts in and a fresh read holds exactly the wake text, `terminal.keys [clear_key]` (hc accepts `ctrl+u`, C:keys.rs:26-58).
- Pokes: skipped (`stash=false`).
- `AwaitingReceipt`/`unreceived` (src/scheduler/mod.rs:785-801) is unchanged. `started=true` plus `unreceived` points precisely at hooks that failed to resolve the seat.

The A4 kind check (`cooperative_wake_ready`, native.rs:1795-1840) has no hc analogue. The evidence is
`foreground_contains(fingerprint_pid)`, which is process-based and stronger than a kind string, and needs no harness
vocabulary in core. The OSC 7501 `app` field is diagnostics only.

### 5.2 Classification (merges with 02 §5.2)

```rust
pub fn idle_wake_on(&self, harness: &str, host: WakeCapability, channel_live: bool) -> Option<IdleWakeOn>;
// Channel iff the policy's idle wake is Channel and channel_live;
// Composer iff host is Composer(_) and the adapter has a composer_policy; else None (wake=no_wake once).
```

Claude with the mod wakes itself on hc (Channel, host-independent, A4 :683-684). Hooks-only Claude,
Codex, Hermes and agy use the hc composer wake.

### 5.3 Contract check

| Rule | hc |
|---|---|
| 1-3 | unchanged (store decisions; turn hooks deliver in-turn because hooks resolve the `Terminal` site) |
| 4 | `if:idle` is a compare-and-type inside the core, after its quiet wait, under the per-terminal input queue (C:input.rs:111-120, :179-189). Typing into `working`/`blocked` is impossible **provided the harness reports `working` during tool runs** (spike W3). Gap: `still_owed` runs before the RPC, and the core's idle compare comes up to ~8 s later. A turn that starts and ends inside that window gets one redundant content-free wake. Detected via `turn`; closed only by upstream `if_turn` (6.6). Accepted limit. |
| 5 | unchanged |
| 6 | wording: "Native: the **host** types the wake marker into an empty composer" (AGENTS.md and TRUST-POLICY). hc is a second Native implementation. |

hc's own notes and `hc send` can still type into working agents. That is outside our contract (vision
D11). Our launches set `no_notify` (section 7). Double delivery under user-enabled notes is documented, not fought.

### 5.4 Draft safety (spike 2C outcome, both branches)

- **Branch B (observed):** the pre-read in 5.1 step 2 is the only guard. Residual race: a person who starts
  typing between our read and the core's `prepare` is held by the quiet gate, then gets the wake typed into
  their draft once they pause for 3 s (C:input.rs:113-120). The window is normally one RPC. TRUST-POLICY
  accepted limit, plus an upstream ask for `send{quiet:"refuse"}`.
- **Branch A (a future hc that refuses or holds drafts):** detected from `hello.features`/`rpc.discover`,
  never from a version. The pre-read stays (it is cheap and also catches pollution). The accepted limit is dropped.
- Focus: hc has no focus signal, so every hc target is treated as unfocused (`focus_known=false`). The
  alternative, the TUI's 60 s focused-empty hold on every wake, breaks "wake within seconds". Spike W5 checks a focused app pane.

## 6. CLI and hooks

### 6.1 Caller location precedence (`CallerEnv::locate`)

1. Valid non-empty `HERDR_THREADS_SEAT_NAME` wins (approved spec). Guard: if a host locator is also present
   and verifies, print the leak notice (3.1). The name is still honoured, because seats are never moved on heuristics.
2. **Exactly one host family**: `One(Pane)` or `One(Terminal)`, unverified, at zero added cost (A1 as today).
3. **Both families**: `Ambiguous([..])` → `LocateCaller`. The daemon walks from the subject (the
   `claim.liveness` harness pid, else the connection's peer pid) through `ProcessInfo` parents (cycle guard as
   `caller_env::walk`). The first pid equal to a candidate's `target_root_process` wins. Nearest wins, so
   nested cases resolve (a Herdr server autostarted inside an hc terminal). Ancestor-or-self covers `exec claude`.
4. Neither verifies, or a needed host is unreachable: refuse; never fall back to the other family. An
   unreachable hc core is `retry` (holders keep PTYs across a core restart).
5. **Phase 0** (no `HcHost` yet): with both families present, verify only the Herdr candidate
   (`pane.process_info.shell_pid`). Verified → the TUI pane wins. Otherwise → the phase-0 error.

Daemon cache (src/service/locate.rs): keyed by (candidates, subject {pid,start}, host boot/epoch). It is
cleared on epoch change and, in phase 2, on hc `restarted|finished|removed` events. After the first turn,
a per-turn hook pays one in-memory lookup.

Also fix: src/cli/mod.rs:311 does not filter an empty `HERDR_PANE_ID` (the hook does, hook.rs:258-260).
`CallerEnv` filters both the same way.

### 6.2 Hooks (on top of 02 §6.2)

The `HERDR_ENV` gates (hook.rs:1463, :1790, :3485, :4019, :4392) become `locate(Hook)` is
`Ok(Some(_))`. An hc site is `Terminal`, which has no `HERDR_ENV` requirement. `attempts_continuity()` is true for
`Terminal` (same terminal, newer session, as in a pane). `moved_target()` is `None`.

The mod watch must never answer `NoPane` for a `Terminal` site (register.js:284-312 stops permanently).

### 6.3 Discovery and install (src/cli/instance.rs, setup.rs, scripts/install.sh)

- **One state dir for every environment:** `$XDG_STATE_HOME/herdr/plugins/herdr-threads`. A TUI plugin,
  hc terminals and named seats on one machine share one node. If it is missing, create it 0700 for `Terminal`
  and `Named` sites (`ensure_default_state_dir`, 02 §6.3).
- Leftover check (instance.rs:271-290): `NotInstalled` is a leftover only without a standalone marker
  `<state>/node/install.json {"mode":"standalone"}`, written by the installer or `setup`. `Disabled` still refuses.
- Claim host: `CallerSite.host` replaces `claim_host`'s `pane_env` (instance.rs:342-349). The host is
  resolved lazily, and never by `herdr status server` for a `Terminal` site.
- `HcDiscovery` (only when no caller site names a core):
  1. list `$XDG_STATE_HOME|~/.local/state/herdr-core/*/core.json`;
  2. derive each socket with the core's rules: `HC_SOCKET`/config `socket`, else `$XDG_RUNTIME_DIR/herdr-core/<session>.sock`, else `<state>/<session>.sock` (C:paths.rs:17-35);
  3. `hello`, and require `core.id` == core.json id;
  4. dedupe by id.

  Never spawn `hc` (V7). If no core is live: `no herdr-core running`.
- install.sh:906-911 runs harness setup only when the plugin is linked. Change: run `setup` whenever it
  can. Keep `herdr plugin link` only when `herdr` is present. Write the marker. Detect hc only to print next steps.
- `setup` bakes `--state-dir`, and on hc-only machines no TUI `--host-endpoint` (src/harness/setup.rs:255-331).
- No daemon in an `hc run --keep` terminal: it would inherit `HC_TERMINAL_ID`, and keep restores a shell,
  not the daemon (04 step 1). The lazy CLI/hook start is enough (with the K5 scrub).
- Skill (integrations/skill/SKILL.md:3, :27, :185): environment-neutral "runs in a Herdr pane, a herdr-core
  terminal or with HERDR_THREADS_SEAT_NAME", plus one line on the split (hc runs and attaches terminals,
  herdr-threads keeps durable threads and receipts). About 160 bytes, once per session.

### 6.4 Labels and topology

- `HcHost::seat_labels`/`live_topology` keep the three-level shape (src/host/observation.rs:283-292, :448-471):
  - workspace = `herdr.space`, tab = `herdr.tab`, target = `tN`;
  - names come from doc `herdr-app` (`{spaces:[{id,name,tabs:[{id,name,root}]}]}`, 04 E), parsed tolerantly;
  - an unnamed tab is shown by its 1-based position (as the app does);
  - pane label = hc `name` or `None` (so displays show `tN`). Never `title`, which is the program's OSC title;
  - unlabeled terminals have empty space/tab.
- The doc is cached by version. In v1 it is re-read when a capture sees changed labels. In phase 2 it is
  re-read on a `doc` event (doc.get can be 1 MiB and has no conditional read, C:api/doc.rs:4-7). Never `doc.set`: the app owns that schema.
- The CLI-side label sources go away: src/cli/mod.rs:981-995, `with_seat_labels` human.rs:52-61,
  thread-local NativeCli peer_locations.rs:14-46, follow.rs:230. Labels come from the daemon (K4).
  Selectors use `HostTopologies` (3.2). That takes 6 of the 15 CLI `NativeCli` sites (03 §3) off the list.

## 7. Launch and handoff (phase 3)

One `terminal.run` per handoff:

- argv `[herdr-threads, internal, launch-gate, --bootstrap <compound>:<attempt>, --, <composed harness argv>]`.
- `cwd`: the frozen cwd.
- `env`: only what ht must set (adapter `native_options_env`, config roots).
- labels `{herdr.space, herdr.tab: <fresh>|<existing>, ht.bootstrap: <HostCallId>, ht.label: <label>}`.
- `no_notify: true`, `outlive_parent: true` (the hc defaults would kill the child with its coordinator and type `<hc>` notes into it, C:api/run.rs:137-139), `keep: false`, no hc `name` (names must be unique and ≤32 chars).

The shim:
1. reads `HC_TERMINAL_ID`/`HC_TERMINAL_SOCKET`;
2. removes leaked `HERDR_*` and each adapter's `CallerEnvShape` session/pid vars plus `child_scrub_vars()` (new neutral adapter method, default empty);
3. sets the caller's config roots, so a leaked `CLAUDE_CONFIG_DIR` cannot win;
4. waits (bounded) until the daemon has resolved its own (core, tN) as the frozen recipient (`ResolveBootstrapSeat`, src/cli/topology_handoff.rs:1174-1265);
5. `exec`s the harness.

`exec` keeps the pid, so `terminal.pid` and the liveness fingerprint follow the harness (spike L2).
This keeps the two-phase invariant: the seat exists, and the bootstrap Send is staged, before the harness runs (TRUST-POLICY.md:324).

Outcome classes:
- A JSON-RPC error before the record exists: `NotSubmitted`.
- `spawn_failed`/timeout/disconnect: `OutcomeUnknown` (a kept record can survive as `spawn_pending`, C:api/run.rs:121-190).
- The `ht.bootstrap` label is a recovery **hint** only. Using it as automatic creation evidence would reclassify a recorded unknown, which TRUST-POLICY.md:2537-2543 forbids.

Placement:
- `--space` → `herdr.space=<id>` + fresh `herdr.tab` (spike L1: how the app treats an unknown tab id).
- `--new-terminal` → no app labels, parent = caller (a paneless sub-agent in the app).
- Tab names cannot be set through the core. Optional phase 4: best-effort `herdr-app do tab.rename` over the app socket.
- Codex `--no-daemon` under hc (04 step 6, unverified): if real, adapter-owned `host_launch_argv(HostKind::Hc)` (neutral default: none), never a core branch.

Peer launch:
- `internal peer-target-create` dispatches on the host kind of `--host-endpoint`. The `[from <tag>]` suffix goes into `ht.label`.
- `place()` (src/service/peer_launcher.rs:258-300) already resolves against every host's `live_topology`.

## 8. TRUST-POLICY touch points (same commit as the code they cover)

1. **Vocabulary** (:121-123): host boot / incarnation per host kind. Herdr TUI: one server process. hc: one core
   identity (`core.json` id). A core process change is a new epoch. C2's "old ids prove nothing" applies across a core-id change.
2. **Closure** for hc: the target is finished, or absent from a complete capture of the same core id. No new `closure_evidence` value.
3. **Accepted limit**: a wiped or cloned core puts saved seats under C2 holds. The restored-pane watch then
   tracks unrelated reused `tN`, and may retire after 10 minutes; the operator path remains. There is no
   retire-on-new-core rule ("a new incarnation never retires").
4. **A1 addendum**: "When the caller env names targets on more than one host, the location is the target
   whose host-reported root process is the nearest ancestor of the caller's harness process (or the connection
   peer). None: refused, never a fallback. Decided by the daemon (A2) from host reports and the process table;
   the client only lists candidates." It is cooperative correction of stale environments, not anti-spoofing.
   Accepted limits:
   - (a) a single-family stale env is accepted unverified;
   - (b) a process that double-forks out of its terminal cannot be verified when both families are present (04 spike 2A(3));
   - (c) a leaked `HERDR_THREADS_SEAT_NAME` is warned about, not blocked.
5. **A4 Wake**, hc clause:
   - the wake goes to the target whose foreground process group contains the binding's fingerprint pid (no host kind on hc);
   - the idle/blocked status is self-reported by the program (cooperative);
   - no focus signal;
   - the draft race (5.4) is an accepted limit while hc's `send` cannot refuse on recent typing;
   - one redundant content-free wake is possible inside the `still_owed` → type window (5.3).
6. **Launch**: `managed_launch` covers gate-shim launches. The correlation label is hint-only.
7. **Delivery contract** rule 6 wording ("the host types"), in AGENTS.md as well.

No new provenance value: hc observations are host evidence like Herdr's, and binding provenances are unchanged.

## 9. Harness-neutral check and merge with named seats

Harness-neutral:
- `HcHost`, `WakeCapability`, `CallerLocator::Terminal`, `LocateCaller`, `CreateTargetPort` and `HostKind::Hc` carry no harness ids.
- Per-harness inputs are existing or neutral-default adapter methods: `ComposerPolicy`, `clear_key`, `clears_unsubmitted_wake`, `CallerEnvShape`, new `child_scrub_vars()` and `host_launch_argv(kind)`.
- `host_kinds` stays Herdr-launch metadata that `HcHost` ignores.
- `scripts/check-harness-neutral` needs no allowlist entry (inferred).

Merge points with 02:

| 02 item | Change here |
|---|---|
| `HostKind {Herdr, Named}`, CHECK `('herdr','named')` | add `Hc`; widen the CHECK in the same migration (3.3) |
| `has_composer() -> bool`, `idle_wake_on(harness, bool)` | `wake_capability() -> WakeCapability`, `idle_wake_on(harness, WakeCapability, channel_live)` |
| `NamedTopology` query, `PooledTopology{herdr, named}` | `HostTopologies` over all attached hosts; `PooledTopology` takes a list |
| `CallerEnv`/`CallerLocator {Pane, Named}` | add `hc_*` fields, `Terminal`, `Located::Ambiguous` |
| 02 §9 "`HC_TERMINAL_ID` above `HERDR_PANE_ID`" | replaced by verified precedence (6.1): neither family is ranked above the other without process evidence |
| named host outside `sessions` | hc host **inside** `sessions` (a real dialled server) |

## 10. Test strategy

- **Fake core** `test_support::fake_hc` (in-process thread, scratch socket via `test_support::scratch::temp_path`, so no child process can leak). JSON-RPC lines for `hello`, `server.status`, `terminal.get/read/send/keys/run/list`, `events.subscribe` (snapshot, events, `missed`, heartbeat) and `doc.get`.
  - Knobs: per-terminal state/status/turn/pid/foreground/labels/screen text; quiet and started delays; core id rotation (wipe); seq reset (restart); witness change; unknown fields and unknown event types in replies (versionless tolerance); `status_mismatch`/`blocked` refusals; a draft that `send` merges (2C behaviour).
- **Contract fixture**: the hc 0.1.15 OpenRPC schema captured as historical evidence. One test asserts every method and param we send exists in it. It is not a gate on a version; a newer schema with extra fields must still pass.
- **Unit**:
  - `HcBoot::from_hello`;
  - capture → `HostSnapshot` satisfies `has_coherent_order` and `authorizes_absence_closure`, drops finished and `machine/tN`;
  - status → `HostUiState` table;
  - `CallerEnv::locate` matrix (name × pane × hc × empty values × missing socket);
  - `verify` nearest-ancestor (nested, self, cycle, double-fork);
  - `idle_wake_on` for every builtin × capability × channel;
  - `AppDoc` tolerant parse.
- **Store/reconcile** (`FakeHostWorld`, src/test_support/fake_hosts.rs):
  - restart → `CarryForward`, zero transitions;
  - finished → `BeginRetirement{Closed}`;
  - core id change → `MarkUnresolved`, no retire;
  - keep recovery → seat kept, binding ended by liveness.
- **Daemon** (`tests/daemon/multi_host.rs` style, own `[[test]]`, `required-features = ["test-support"]`):
  - kind sniff;
  - two fake cores both with `t1`: labels never cross (K4);
  - wake: pre-read refusal on a draft, `if:idle` refusal, `NoStatus`/`StatusDone` blocks, budget > 2 s does not become `OutcomeUnknown`;
  - `LocateCaller` with a TUI fake plus an hc fake.
- **CLI/hook integration** (isolated state dir, fake core, no real Herdr): `inbox`/`check_in` from a `Terminal` site; the phase-0 refusal with both families present; daemon autostart from an hc env carries no `HC_SCOPE` (K5).
- **Opt-in real-install e2e** (`scripts/hc-e2e`, private `HC_SESSION`, scratch XDG dirs, never `hc server status` on the default core, every core it starts is stopped): an hc analogue of the wake matrix. It is **not** a release gate until hc support ships; then the vision decides whether it joins the pre-release wake e2e.
- New suites with no process-wide state go into `tests/combined.rs`. The fake core runs in-thread, so it fits there unless a suite also runs an in-process daemon.

## 11. Risks

1. **Draft clobber** (5.4): the residual race is real and certain once a person pauses for 3 s inside the window. It is accepted only with a TRUST-POLICY limit, and closed only upstream.
2. **`done` parking / tool-time `idle`** (W2, W3): if a harness parks in `done`, hc wakes silently never fire. If it reports `idle` during tools, rule 4 breaks. Both need the spike before phase 2.
3. **Boot choice**: keying boot on the core process instead of the core id would unresolve every hc seat on each autostart, restart or update.
4. **Snapshot filtering**: if finished or remote terminals are not dropped, closed terminals look present or remote ones get staged as local.
5. **Env leaks**: HERDR_*/CLAUDE_CODE_* reach hc terminals (04 B). Without the shim, launched agents bind foreign seats or configs. Without K5, a daemon autostarted from an hc terminal is swept when that terminal ends.
6. **Composer text parity**: `terminal.read` plain text may not parse the same as Herdr detection text (NBSP, wrapping). The failure is safe (Unreadable → no wake), but it costs that harness its wake.
7. **Wake budget**: a 15 s hc send ties up the host's wake lane. One lane per host limits the blast radius (inferred acceptable).
8. **App pane close** is unverified: if it unlabels instead of killing, seats in closed app panes never retire (spike O1).
9. **"First host" code**: Herdr-only consumers (MachineLister, archival, bootstrap identity selection) can pick an hc session, unless the kind audit lands in phase 1.
10. **Product direction** (herdr-app#63): the TUI may move onto core. The two backends stay independent behind one port, so either can be retired.

## 12. Phased rollout

| Phase | Content | Depends on |
|---|---|---|
| 0 | TUI guard (6.1 step 5); K5 daemon spawn scrub; empty-`HERDR_PANE_ID` filter; labels from the daemon by `(host_id, target)` (K4). No hc code. | named seats' `CallerEnv` |
| 1 | `HcHost` observation, identity, `LocateCaller`, `Terminal` sites in CLI/hooks, labels/topology, discovery, standalone install, skill text, TRUST-POLICY 1-4, kind audit. Wake capability `None` → `wake=no_wake`, except Claude mod Channel. Messaging works end to end. | 0, named seats merged |
| 2 | Composer wake (5.1), `WakeBlock::{NoStatus,StatusDone}`, `foreground_contains`/`process_group`, events hint lane + doc cache + locate-cache invalidation, TRUST-POLICY 5, AGENTS.md rule 6 wording | spikes W1-W5 |
| 3 | `CreateTargetPort`, V3 journal, launch-gate shim, handoff `--space/--new-terminal`, peer `NewTab` into app spaces, TRUST-POLICY 6 | spikes L1-L4 |
| 4 (optional) | app tab rename, mod panel, `LaunchTarget::NewTerminal` with a cap | — |

Upstream asks (via the Herdr channel, not gh), each detected as a capability, never a version:
`send{quiet:"refuse"}`; `if` accepting `done` or a list; `if_turn: N`; start the core with a sanitized base env
or strip foreign host identity; public `scope`.

## 13. Spikes

- **W1** `terminal.read` text parses under each `ComposerPolicy` (empty, draft, placeholder, wrap).
- **W2** Claude/Codex status after a completed turn (`idle` or `done`).
- **W3** `working` reported during tool runs.
- **W4** `getpgid` on another session's pid on macOS and Linux; aisw-wrapped leader vs fingerprint pid.
- **W5** Typing in a focused app pane while a wake is due.
- **O1** What app pane close does to its terminal (kill → finished/removed, or unlabel).
- **D1** A Herdr TUI socket's reply to a JSON-RPC `hello` line (kind sniff must not hang).
- **L1** `herdr.space=<sp>` + unknown `herdr.tab` makes a new tab; required id format.
- **L2** The shim's `exec` keeps `terminal.pid`; 7501 `app` shows the harness.
- **L3** `outlive_parent` + `no_notify` stop every `<hc>` note.
- **L4** Codex `--no-daemon` under hc; the Linux socket path of an app-started core.
