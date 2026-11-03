# Unified code design: Herdr TUI panes, named seats and herdr-core terminals behind one host seam

Inputs: 02 (named seats code design), 06 (herdr-core code design), 03 (coupling survey), 00, 01, 04 (real-install
spikes; they override research inferences), 05 (vision draft), the approved named-seats spec
(`docs/superpowers/specs/2026-10-31-named-seats-design.md`), and the two unification proposals A (minimal change)
and B (clean abstraction). Tree: `.worktrees/external-seat` at badccf481. Paths are repo-relative. "(inferred)"
marks a claim not traced end to end. This document starts from A and grafts B's best ideas; section 1 records the
judgement.

Revised after the roast (2026-10-31). The confirmed findings and how each one was resolved are listed in
`docs/superpowers/specs/2026-10-31-environments-proposal.md`, Part 4. The main changes:

- An inherited seat name is refused.
- The named enumeration never closes a seat.
- Restore and named continuity get explicit rules.
- hc composer wake is gated on an upstream draft-safe primitive.
- hc closure is keyed on the finish reason.
- The kind sniff now runs before the host row is written.
- G4 is redefined as "this build has a constructor for the kind".

## 1. Judgement of the two proposals

Scores are 1 (poor) to 5 (best).

| Criterion | A (minimal) | B (clean) | Notes |
|---|---|---|---|
| Environment-unawareness of core | 3 | 5 | A keeps `HostKind` readable in two places and an enum `CallerLocator` in the CLI. B confines every family name to `src/host/<family>/` and a registry. |
| Total change size (5 = smallest) | 4 | 2 | B splits `HostPort` into a core plus five capability traits. There are 40 `impl ... HostPort for` blocks across 27 files (grep: src/ports.rs, src/host/native.rs, src/cli/launch.rs, src/service/host_sessions.rs, src/test_support/fake_hosts.rs and 22 test files). Every one of them changes, for no visible behaviour. |
| Risk to trust / delivery / wire invariants (5 = lowest) | 4 | 3 | B puts the named host inside `sessions`, so every `attached()` consumer becomes load-bearing at once. B's `wake() -> None` lets a forgotten accessor make wake tests pass vacuously (B's own R2). B keeps `SeatSummary.name` on a `deny_unknown_fields` struct (src/protocol/results.rs:1083-1085). B's claim that address aliases skip empty ids is not in the code: src/protocol/address.rs:461-466 checks `tab_has_one_pane` / `space_has_one_tab_with_one_pane` and has no empty-id guard. A has no peer-wire change, and its new display field sits in skew-safe `SeatDirectory` (no `deny_unknown_fields`, results.rs:1111-1112, verified). |
| Testability | 4 | 3 | A's no-regression gate is one table test, `idle_wake_on(h, HERDR_TUI, _) == has_idle_wake(h)`. B needs a forwarding macro over every fake, and a "fake exposes wake()" assertion to catch silent `None`. |
| Fit with the code | 5 | 3 | A mirrors the existing seams: `HostFactory` closures (src/service/host_sessions.rs:61-77), defaulted `HostPort` methods with the no-`Unsupported` self-test (src/ports.rs:4533-4572), and the enum rationale 02 already gave. B's `HostFamily` registry imitates harness `builtins()`, but with three built-in hosts it is indirection without a second implementer. |
| **Total** | **20** | **16** | |

**Base: A.** B's main contribution is the discipline, more than the type split: name every node-wide consumer and
say why it includes or skips a host. The following ideas are grafted from B:

- G1: an explicit, verified audit of every `attached()` consumer (section 5.3). A had only a partial list. B's was partly inferred.
- G2: a fixed reserved named-host locator, so no endpoint rewrite is needed (B X6). It is an open decision (O2), recommended.
- G3: one `ENV_VARS` const per host module, used both for caller location and for the daemon's spawn scrub (B's `owned_env_vars`, without the registry).
- G4: a host row is attachable only when this build has a constructor for its kind. A static implemented-kinds set grows by phase: `{herdr}` in F, `+named` in N, `+hc` in H1. Every other value maps to `Unknown`, which the reattach sweep, restore and rebind skip and `doctor` names. A `HostKind` variant, field or method lands in the phase that implements it, not in F. Otherwise an F binary reading a row with kind `hc` would treat the kind as known and dial the core as Herdr (roast U-A3).
- G5: neutral operator error text ("no host attached"), with no "Herdr" in it (B 4.4).
- G6: an optional follow-up check, `scripts/check-host-neutral`, the analogue of `check-harness-neutral` (B R11). It is not in v1.
- G7: B's per-environment capability table format (section 4).

The following B ideas are rejected:

- the `HostPort` split into capability objects (churn, and the silent-`None` risk);
- the `HostFamily` registry and `HostTraits` (no second implementer of the abstraction);
- `dyn CallerSite` / `LocatorChain` (three built-in variants, matched in about six small methods);
- the store-side `HostTraitsLookup` (it gives the store a dependency on a code registry at open);
- named host inside `sessions`.

Compared with the original 02 and 06, the whole unified design has:

- no peer-wire change;
- one migration;
- one topology query;
- one wake-capability type;
- one caller-location resolver;
- one wake-lane rule shared by named seats and hc phase 1.

## 2. Goal

An agent session gets a durable seat in any of three environments and works through the same CLI, hooks, mod
channel, daemon, store and peer wire in all of them: join, send, inbox, ACK, turn-hook and channel delivery, idle
wake as far as the host allows, and messaging across environments and linked nodes. The three environments are:

1. a Herdr TUI pane;
2. a plain process with `HERDR_THREADS_SEAT_NAME`;
3. a herdr-core terminal (`hc run`, or a pane of the Herdr app).

Core code reads host capability answers and never branches on the environment for policy. Two exceptions read the
persisted host kind, and both are listed in section 3.1.

Acceptance criteria for the whole design:

- The Herdr TUI wake e2e is unchanged. `idle_wake_on(h, HERDR_TUI, _)` equals `has_idle_wake(h)` for every builtin. `has_idle_wake` is kept as the harness-only oracle (`pub(crate)`) and is not deleted.
- A named-host enumeration never retires or unresolves a seat (test: a seat missing from it stays resolved).
- A `HERDR_THREADS_SEAT_NAME` that the host server process also carries is refused and never bound.
- `attached()` never contains the named host.
- Labels of two hosts that share a target id (two cores, both with `t1`) never cross.
- With both `HERDR_PANE_ID` and `HC_TERMINAL_ID` set, a seat binds only to a verified candidate, otherwise refuses.
- There is no new peer-wire field, frame or capability. `scripts/check-wire-compat` passes with no baseline line.
- `scripts/check-harness-neutral` passes with no new allowlist entry.

## 3. Environment model

| | Herdr TUI pane | Named seat | herdr-core terminal |
|---|---|---|---|
| Host | Herdr server, dialled over its socket | built-in, in-process, never dialled | hc core, dialled over JSON-RPC (04 step 5) |
| Locator | canonical server socket | reserved path (O2) | canonical core socket (`HC_TERMINAL_SOCKET`) |
| Target | pane id (`w..:p..`) | the name | `tN` (stable for the terminal's life, 04 step 3) |
| Boot / incarnation | server process | fixed per node (02 C1) | `hc-core:<core.json id>` (06 V2; id stable across restart and stop/start, 04 D) |
| Caller env | `HERDR_ENV`, `HERDR_PANE_ID`, `HERDR_SOCKET_PATH` | `HERDR_THREADS_SEAT_NAME` | `HC_TERMINAL_ID`, `HC_TERMINAL_SOCKET` (`HC_SCOPE` is scrubbed, never read) |
| Wake | composer typing (Native) | none (Channel only, with a live mod) | none (Channel only). Composer typing only once `hello.features` reports an upstream draft-safe send (10.3) |
| Closure | absence from a complete snapshot | explicit retire only (non-closing enumeration) | `killed`/`removed` of a resolved seat retires it; exit, signal, `server_stop` and `host_shutdown` make it unresolved; `lost`/`restarted` change nothing |
| Launch | in place (`agent.start`) and new tab | refused | new terminal through a gate shim (phase H3); in place refused |

What all three share, unchanged:

- seats, bindings and `CallerClaim{host, target}` (src/protocol/authority.rs:8-29);
- the cooperative mapping against the effective observation (src/store/seats.rs:5131-5307);
- the OS liveness fingerprint (src/harness/caller_env.rs:240-266);
- the observation lane and reconcile (src/identity/reconcile.rs);
- `RemoteResolver`, receipts, delivery, the peer wire.

Each host plugs in by implementing `HostPort` (src/ports.rs:3907). Today its only production impl is `NativeCli`.

### 3.1 The one structural difference: dialled vs built-in

Dialled hosts (Herdr TUI, hc) live in `HostSessions.sessions`. There they are capped by `MAX_ATTACHED_HOSTS`
(host_sessions.rs:42), swept for reattach, counted by idle exit (`attached_count`, src/daemon/idle_exit.rs:38) and
listed by Health. The built-in named host lives in its own slot and is reached only by readers that ask for it
(`all_for_reads`).

`kind()` is read in exactly two places:

- the persisted `herdr_hosts.kind` and the cross-kind rebind refusal (store);
- the peer-discovery gate. `any_reachable` (host_sessions.rs:500) feeds `herdr machine list` (src/app.rs:2279-2282), and with only hc attached it would otherwise fork `herdr`.

## 4. Shared abstractions (Rust sketches)

### 4.1 Host port additions (src/ports.rs, on `HostPort`)

```rust
/// Persisted host family: storage, diagnostics, the cross-kind rebind refusal and the
/// peer-discovery gate only. Policy reads capabilities, never this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKind { Herdr, Named /* N */, Hc /* H1 */, Unknown }   // variant per phase (G4); Unknown = no constructor

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeCapability {
    /// Nothing can be typed. No wake lane; `wake=no_wake` unless the harness wakes itself.
    None,
    /// The host types into a composer. Idle, draft and submit gates stay inside the port.
    Composer(ComposerTraits),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComposerTraits {
    pub submit_budget: Duration, // TUI 2 s (src/host/native.rs:57). focus_known/stash land with the hc wake (H2), not in F
}
impl ComposerTraits {
    pub const HERDR_TUI: Self = Self { submit_budget: Duration::from_secs(2) };
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchivalEvidence { Supported, NoEvidence }   // NoEvidence never reads as "empty composer"
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosureAuthority { Closing, NonClosing }     // NonClosing: snapshots publish targets, never close them

pub trait HostPort: Send + Sync {
    // ... existing ~30 methods unchanged ...
    fn kind(&self) -> HostKind { HostKind::Herdr }
    fn wake_capability(&self) -> WakeCapability { WakeCapability::Composer(ComposerTraits::HERDR_TUI) }
    /// Root pid of a target, for LocateCaller. None = no evidence, never "verified".
    fn target_root_process(&self, _t: &HostTargetId, _cx: &HostCallContext) -> Result<Option<u32>, ApiError> { Ok(None) }
    /// Pid of the host server process (socket peer pid of the dialled connection; inferred, spike E1).
    /// Used only by the inherited-seat-name refusal (4.4). None = no check.
    fn server_process(&self) -> Option<u32> { None }
    fn archival_evidence(&self) -> ArchivalEvidence { ArchivalEvidence::Supported }
    fn closure_authority(&self) -> ClosureAuthority { ClosureAuthority::Closing }
    // foreground_contains (06 2.4) is added with the hc wake (H2), not in F.
}
```

- Every default returns a value, never `ErrorCode::Unsupported`, as the self-test at src/ports.rs:4533-4572 requires.
  The 40 existing impls compile unchanged.
- The required methods that a built-in host cannot serve are listed below. NamedHost and HcHost answer them with a
  non-Unsupported `Err`. Each is genuinely unreachable because a capability answer refuses first:
  - `observe_current_target_for_archival`: archival filters on `archival_evidence() == Supported`, and both
    NamedHost and HcHost return `NoEvidence`;
  - `safe_wake_target` and `submit_prompt`: no wake lane, because `wake_capability() == None`;
  - `launch_native`: `native_launch_capability()` keeps meaning *in-place* launch, and both hosts return
    `Unsupported` in every phase. hc creation is a separate answer on `CreateTargetPort` (H3). Without this,
    in-place launch and the per-session `BootstrapRuntime` gate (host_sessions.rs:693-703) would pass for hc and
    reach the `Err` (roast U-A2).
- NamedHost returns `closure_authority() == NonClosing`. Reconcile skips the absence-closure step for such hosts
  (reconcile.rs:325-335), so a named enumeration can never produce `BeginRetirement` or `MarkUnresolved`
  (roast N-T2, N-A2, U-T5). A store guard also refuses `RetireReason::Closed` for a seat on a `named` host.
- Fields and methods are added in the phase that reads them (G4). `submit_budget` is read by the dispatcher
  budget (src/notification/dispatch.rs:39).
- `NativeCli::target_root_process` is `pane.process_info.shell_pid` (06 6.1 step 5). It is used by the phase-F guard.
- `ProcessInfo::process_group(pid) -> Option<u32>`, defaulted `None` (src/ports.rs:4177; getpgid in
  src/host/process_table.rs), arrives in H2.

### 4.2 Host sessions: dialled map plus built-in slot (src/service/host_sessions.rs)

```rust
pub struct HostSessions {
    sessions: RwLock<BTreeMap<HostId, Arc<HostSession>>>, // dialled: Herdr TUI, hc
    builtin: OnceLock<Arc<HostSession>>,                   // named host; never dialled, never capped
    /* rest unchanged */
}
pub struct HostSession { /* existing */ wake: Option<LaneHandle>, /* ... */ }
impl HostSessions {
    /// Daemon start. Skips factory.exists, the canonical-on-disk check and the cap (host_sessions.rs:575-597).
    pub fn attach_builtin(&self, port: Arc<dyn HostPort>, observer: Arc<dyn BootstrapObserver>,
                          locator: HostLocator) -> Result<(), ApiError>;
    /// Dialled sessions plus the built-in one. Only for the readers listed in 5.3.
    pub fn all_for_reads(&self) -> Vec<Arc<HostSession>>;
    // by_locator / resolve: builtin first, then the map.
}
```

The wake lane rule replaces 02's `Lanes` flag. `start_session` (host_sessions.rs:655-787) wraps the
`start_wake_worker_named` call (:731-749) in `if port.wake_capability() != WakeCapability::None`. The capability is
read once at attach; an hc capability change (branch A detection, 7.3) needs a session reattach (inferred acceptable).

The consumers of `session.wake()` must take `Option`:

- `first_host_pacer` and `Lane::Wakes` (src/app.rs:1028-1043, verified);
- `reachability.attach_wake_pacer` (host_sessions.rs:710);
- Health lane listing.

`HostFactory.build` (host_sessions.rs:74-77; closures in src/main.rs:67-80) stays the only constructor of dialled
hosts. In H1 it splits into `probe(locator, budget) -> Result<HostKind, ApiError>` and
`build(kind, locator) -> (port, observer)`.

- The probe sends one JSON-RPC `hello` line with a short deadline. A reply with `core.id` means `Hc`; anything else
  falls back to the Herdr `ping` (spike D1: the TUI socket must not hang).
- The probe runs **before** `store.attach_host` (today attach_host runs first, host_sessions.rs:600-608), and
  `attach_host` inserts the probed kind. The column default `'herdr'` is never trusted: a row written by default
  or migration carries `kind_probed_at IS NULL` and is re-probed on reattach.
- On a mismatch between the stored kind and the probe, the attach is refused and `doctor` reports it.
- The probe is bounded by the caller's budget and holds `attach_lock` (host_sessions.rs:575-597). That cost is
  documented and paid only on the first attach of a locator.
- The probe also reads the core id, which becomes the boot (06 V2). A known locator answering with a different core
  id is a new boot, so its seats get C2 holds and are never matched by `tN`.

### 4.3 Wake classification (src/harness/registry.rs)

```rust
pub enum IdleWakeOn { Composer, Channel, None }
/// Generalises has_idle_wake (registry.rs:1446-1459), which stays as the pub(crate) oracle for the table test.
/// channel_live = ModChannels::mod_suppressed(seat, now) = wake_suppressed && !stalled (scheduler/mod.rs:671-678),
/// so a stalled channel on a WakeCapability::None host classifies as None and is warned once.
pub fn idle_wake_on(&self, harness: &str, host: WakeCapability, channel_live: bool) -> Option<IdleWakeOn> {
    let adapter = /* lookup; None for unknown ids and `human`, as today */;
    let channel = adapter.delivery_policy().is_some_and(|p| p.idle_wake() == IdleWake::Channel);
    Some(match host {
        // Identical to has_idle_wake: the no-regression table test.
        WakeCapability::Composer(_) if adapter.composer_policy().is_some() => IdleWakeOn::Composer,
        WakeCapability::Composer(_) if channel => IdleWakeOn::Channel,
        WakeCapability::None if channel && channel_live => IdleWakeOn::Channel,
        _ => IdleWakeOn::None,
    })
}
```

`wake_status` (src/service/dispatch.rs:262-300) makes three changes:

- It takes the row's own host (store rows gain `host_id`), replacing today's `find_map` over every attached host (:278-281).
- It looks the host up through `all_for_reads`.
- It maps `None` to `WakeBlock::NoWake`. For a seat whose host has `WakeCapability::None`, the report is
  **persisted** as `(observer, seat)` in the store (6.4) and is never cleared by a channel flap or a daemon restart.
  So the warning fires once ever per pair, and again only after a retire and a re-mint. The in-memory `first` set
  (dispatch.rs:118-127, 284-299) stays for Composer hosts (roast N-D2, U-D1).

`Composer` keeps today's `wake_block` call. No `ModChannels::has_live` is added; the existing grace- and
stall-aware `mod_suppressed` is reused (roast U-D2).

`seat threshold` on a seat whose host has `WakeCapability::None` still records the value. It also prints one line,
`herdr-threads: <seat> has no idle wake; threshold has no effect` (about 60 bytes plus the seat).

### 4.4 Caller location (src/cli/caller_location.rs, new)

```rust
/// The only reader of caller-location env. Empty values are filtered (fixes src/cli/mod.rs:311,
/// which today does not filter, unlike hook.rs:258-260). Var names come from each host module's
/// ENV_VARS const (G3): host::native::ENV_VARS, host::named::ENV_VARS, host::hc::ENV_VARS.
pub struct CallerEnv { herdr_env: bool, pane: Option<String>, pane_host: Option<PathBuf>,
                       seat_name: Option<String>, hc_terminal: Option<String>, hc_socket: Option<PathBuf> }
pub enum EnvMode { Cli, Hook }
pub enum CallerLocator { Pane(HostTargetId), Named(SeatName), Terminal { socket: PathBuf, id: HostTargetId } }
pub enum Located { One(CallerLocator), Ambiguous(Vec<CallerLocator>) }
impl CallerEnv {
    pub fn from_process() -> Self;
    pub fn from_lookup(get: &dyn Fn(&str) -> Option<String>) -> Self;     // tests
    pub fn locate(&self, mode: EnvMode) -> Result<Option<Located>, LocatorError>;
}
pub struct CallerSite { pub host: HostLocator, pub target: HostTargetId, locator: CallerLocator }
impl CallerSite {
    pub fn moved_target(&self, ..) -> Option<HostTargetId>; // Pane only (NativeCli pane.current, mod.rs:3784, hook.rs:1631)
    pub fn relative_pane(&self) -> Option<&HostTargetId>;   // Pane only (mod.rs:766-780)
    pub fn person_origin_allowed(&self) -> bool;            // Pane only (me init, A4 pane_agent probe)
    pub fn enrolls_on_first_use(&self) -> bool;             // Named only
    pub fn attempts_continuity(&self) -> bool;              // Pane, Terminal
    pub fn host_scoped_lookup(&self) -> bool;               // Named, Terminal: send SeatsQuery.host
    pub fn cache_key(&self) -> String;                      // pane id | "name:<n>" | "<core>:<tN>"
}
```

Precedence and verification in `locate`:

1. **Named seat.** A non-empty `HERDR_THREADS_SEAT_NAME` that parses as a `SeatName` replaces the caller's
   *target*. It does not replace instance discovery (4.4.1). An invalid name is a hard error naming the variable,
   never a fallback.
   - **Inherited-name refusal.** If a host candidate (Pane or Terminal) is also present, the claim carries the
     candidate as a `locator_hint`. The daemon reads the environment of that host's `server_process()`
     (`/proc/<pid>/environ` on Linux, `KERN_PROCARGS2` on macOS; same user; spike E1). If it carries the same
     name, the name was inherited from the host, for example a core autostarted from a named shell (04 step 3).
     The call is refused with one line, `HERDR_THREADS_SEAT_NAME=<n> inherited from the <host> server; set it per
     command or restart the server without it` (about 110 bytes plus the name). There is no fallback to the host
     candidate. A name set per command or per terminal, which the server does not carry, is honoured. This is a
     structural consistency check (A1), not attestation (roast N-T1, N-D1, N-A1, H-T3, U-T1).
   - **Observability.** Each new binding of a named seat records its `locator_hint` (`HERDR_PANE_ID`, or
     `<core>:<tN>`). The first time a named seat gains a binding from a second distinct locator, the daemon
     records that once per (seat, locator) and returns one stderr line, `herdr-threads: seat <n> now bound from
     <loc1> and <loc2>` (about 60 bytes). `seat inspect` lists bindings by locator. This replaces the per-process
     leak notice, which a one-shot CLI would have printed on every call (roast U-D5).
   - **Pane-form refusal.** Pane-form allocation (`send --seat <pane>`, `invite --pane`, `--require-ack-pane`) of a
     pane whose current named binding recorded that pane as its hint is refused with
     `pane <id> runs named seat <n>; address it by name` (about 50 bytes). The refusal is gated on the hint and
     never moves or merges a seat (roast N-T8).
   - **Scrub.** Everything herdr-threads launches, hands off to or spawns (`spawn_detached`, the H3 gate shim, any
     core it would start) drops `HERDR_THREADS_SEAT_NAME`. The skill and `setup` text say to set the name per
     command, never in a shell that starts servers.
2. **One family.** `One(Pane)` or `One(Terminal)` is accepted unverified, at zero extra cost. This is A1 as it is
   today. In hook mode a pane still needs `HERDR_ENV=1` (hook.rs:255-262). `Terminal` without
   `HC_TERMINAL_SOCKET` is refused.
3. **Both families.** `Ambiguous` → the daemon's `LocateCaller`:
   - the walk starts at the subject (the claim's liveness pid, else the connection's peer pid) and goes up through `ProcessInfo` parents;
   - the first pid equal to a candidate's `target_root_process` wins, nearest ancestor first (06 6.1). No family is ranked above another: this replaces 02 §9;
   - a stale `HERDR_PANE_ID` inside an hc terminal is the common case (04 step 3, B), so a fixed rank would be wrong in nested setups.
4. **Neither verifies,** or a host is unreachable: refuse. Never fall back.
5. **Phase F guard** (before `HcHost` exists): with both families present, verify only the Herdr candidate. If it
   verifies, the pane wins. Otherwise the CLI prints the "not supported yet" error.

#### 4.4.1 Instance discovery with a name (roast N-T7, N-D6, N-A7)

The name never bypasses discovery of the command group. The order is:

1. the `--state-dir` flag;
2. `HERDR_PLUGIN_STATE_DIR`;
3. the hook-baked `--state-dir` of the installed hooks;
4. when Herdr env is present (`HERDR_ENV`, `HERDR_SESSION`, `HERDR_CONFIG_PATH`), today's discovery, including the
   `herdr plugin list` query for a non-default server (instance.rs:168-198);
5. the existing default dir;
6. the default dir is **created** only when no Herdr is configured or on PATH. Otherwise there is one
   `herdr plugin list` query, or a refusal telling the user to pass `--state-dir`.

`doctor` flags two live stores for one user. A beside-Herdr test runs Herdr under a non-default session and checks
that pane-to-named messaging goes through one daemon.

`CallerSite::env_assignments() -> Vec<String>` is the only builder of caller-site env:

| Site | Variables |
|---|---|
| Pane | `HERDR_PANE_ID` |
| Named | `HERDR_THREADS_SEAT_NAME` |
| Terminal | `HC_TERMINAL_ID` and `HC_TERMINAL_SOCKET` |

The three handoff recovery-hint emitters (handoff.rs:1930-1933, handoff_delivery.rs:1397-1400 and :1816-1822)
build their lines through it from the frozen claim's site, not with `HERDR_PANE_ID=<target>`. A test checks that a
named caller's retry line resolves back to the same seat (roast U-T3).

`CallerSite::site()` maps the locator to `(host, target)`:

- Pane: `rt.claim_host()`, with today's errors (src/cli/instance.rs:342-349);
- Named: the reserved locator (O2);
- Terminal: the canonical `HC_TERMINAL_SOCKET`.

`CallerClaim` is unchanged in all three environments.

### 4.5 Labels and topology served by the daemon (fixes K4)

```rust
// protocol/commands.rs
Command::HostTopologies          // -> Vec<{ host: HostLocator, topology: HostTopology }> from all_for_reads()
// protocol/results.rs: SeatDirectory has no deny_unknown_fields (results.rs:1111-1112), so this is skew-safe:
#[serde(default, skip_serializing_if = "Option::is_none")] pub name: Option<SeatName>,
// SeatSummary.directory and ParticipantLocation.remote (wire name kept) are now filled for LOCAL seats too,
// from seat_publisher::read_labels keyed (host_id, target) (src/service/seat_publisher.rs:195-226).
```

- **Selectors.** CLI selectors pool the `HostTopologies` entries. `Selector::resolve_with` already pools and reports
  `Ambiguous` (src/protocol/address.rs:350-378). Herdr's client-side read is the fallback only while the claim host
  is not yet attached (inferred).
- **Names do not break selectors.** A bare token reads as a seat name only when no pane-path reading matches, so a
  stray seat named like a tab never turns `send review` into an ambiguity error. Exact seat ids always work.
  Auto-creation of a named seat happens only on seat-acting calls (hooks, send, join, ack), never on read-only ones
  (roast N-A6). **[decide D4]**
- **hc labels.** hc labels are `<HC_SESSION>:tN`, or bare `tN` only when exactly one core is attached. Labels are
  keyed `(host_id, target)`. Topology served to the RemoteResolver carries the session, and an ambiguous bare `t1`
  is refused with the list of candidates (roast H-T7).
- **Skew.** `HostTopologies` is a new local command. An older daemon returns the existing upgrade error. Its result
  type has no `deny_unknown_fields`.
- **Synthetic ids.** Named and hc topologies use reserved space and tab ids that no selector segment can match. This
  follows 02, not B's empty-id guard, because no such guard exists (address.rs:461-466).
- **Named labels.** `NamedHost::seat_labels` returns pane label = name, so displays need no name branch.
- **What it removes.** Four CLI `NativeCli::new` sites go away: src/cli/mod.rs:706, :986, :995 and follow.rs:230.
  In total there are 13 direct sites in src/cli (grep; 03 counted 15, including the typed thread-local holder).

### 4.6 Launch

- **Capability answer.** `native_launch_capability()` (src/ports.rs:2084) is the existing answer.
  - NamedHost returns `Unsupported`, and the CLI refuses before any create with the spec text "named seat <name> has
    no pane to launch into" (about 45 bytes plus name). The text comes from the port refusal, not a kind branch.
  - HcHost returns `Unsupported` in every phase, because this answer means in-place launch. Creation of a new hc
    terminal is answered by `CreateTargetPort` (H3).
- **Bootstrap selection.** The production gate is the per-session `BootstrapRuntime` (host_sessions.rs:693-703),
  which already keys on `native_launch_capability() == HostGuardedStart`. hc and named never pass it.
  `with_prebuilt_bootstrap` (dispatch.rs:213-219) is a test fixture builder (called only from tests/) and is not a
  production consumer. Each of these gets a two-host test (TUI and fake hc).
- **Peer placement.** `place()` (src/service/peer_launcher.rs:258-300) skips hosts without a creation capability.
  It also loses the "no Herdr server" text (G5).
- **Phase H3.** H3 adds `CreateTargetPort` (06 2.6) beside `CreateTabPort` (src/ports.rs:5536). `terminal.run` uses
  `no_parent: true` (or `outlive_parent` plus `no_notify`), with `caller` omitted. hc's defaults would make the
  worker die with its parent and type `<hc>` notes into the coordinator (api/run.rs:109-139; notes.rs:56,
  :166-176). The prompt is delivered through herdr-threads, never `first_message`. Its parts:
  - `Placement{NewTab, NewTerminal}`;
  - `HostWitness{HerdrSocket, HcCore}`;
  - `CreatedTarget` with V3 journal records, leaving V1/V2 bytes unchanged (06 K3);
  - the `internal launch-gate` shim that scrubs env, waits for the daemon to authorize its `(core, tN)`, then `exec`s the harness (06 V6, keeps TRUST-POLICY's two-phase launch invariant);
  - `src/cli/host_client.rs`, for the CLI launch sites that remain.

### 4.7 Closure and liveness

- **Liveness** is unchanged in every environment: the OS process fingerprint, never a host report (06 V5).
- **Closure** uses only the existing publication-absence path (src/store/seats.rs:2950-2963), not the constructor-less
  `ClosureEvidence` (06 K8). The `enumerate_targets` contract carries closure per host:
  - Herdr: a complete `session.snapshot`.
  - Named: an enumeration of every non-retired local seat **on the named host** (`host_id = named`, not keyed on the
    name, so no row can be missing). It publishes targets but never closes them (`ClosureAuthority::NonClosing`). It
    is not `CoherentVerified`, because it is the store reading itself, not host evidence (roast U-T5). Snapshot
    rows from it carry `cooperative_named_seat`, and every C1 decision that would treat them as structural checks
    that tag. A named seat closes only by self-retire or operator retire.
  - hc: `hello` and a one-shot `events.subscribe` snapshot on one connection, keyed on the same core id. Events are
    dirty hints that trigger a capture; closure is decided from the capture. Addresses containing `/` (remote
    mirrors) are ignored. Finish-reason rules:

    | Finish | Effect on the seat |
    |---|---|
    | `killed`, or `removed` of a seat still resolved | retires it through the pane-close fence |
    | program exit, signal, `server_stop`, `host_shutdown` | makes it unresolved: bindings end `seat_unresolved`, and C1 resume reattaches it |
    | `lost` / `restarted` | nothing |

    A failed `hello` gives `CompleteUnverified`, which never closes anything (roast H-T1, H-T5, H-A4).

## 5. Per-environment implementations

### 5.1 Capability table (G7)

| Answer | Herdr TUI (`NativeCli`) | Named (`NamedHost`, built-in) | hc (`HcHost`, dialled) |
|---|---|---|---|
| `kind` | Herdr | Named | Hc |
| `wake_capability` | `Composer(HERDR_TUI)` | `None` | `None`; `Composer` only when the upstream draft-safe send is detected (H2) |
| `closure_authority` | Closing | **NonClosing** | Closing (finish-reason rules, 4.7) |
| `archival_evidence` | Supported | NoEvidence | NoEvidence |
| `server_process` | socket peer pid (inferred, E1) | `None` | socket peer pid of `serve` (inferred, E1) |
| boot / incarnation | server process (unchanged) | fixed, from node uuid | `hc-core:<core id>` |
| epoch / sequence | unchanged | own counters, `resume_after_epoch` = max(e, p+1) | own counters (06 K2) |
| `enumerate_targets` | `session.snapshot` | store read of live seats with `host_id = named` (non-closing) | `hello` + `events.subscribe` snapshot |
| `observe_current_target` | `pane.get` | name parses → present, generation 1 | `hello` + `terminal.get`, never with `caller` |
| `live_topology` / `seat_labels` | unchanged | one pane per name, label = name | `herdr.space`/`herdr.tab` labels + doc `herdr-app` names |
| `native_launch_capability` (in place) | unchanged | Unsupported | Unsupported (creation is `CreateTargetPort`, H3) |
| `target_root_process` | `pane.process_info.shell_pid` | `None` | `terminal.get.pid` |
| `CallerLocator` | `Pane` | `Named` | `Terminal` |
| moved target / continuity | yes / yes | no / no (name equality is continuity) | no / yes |
| `ENV_VARS` (G3) | `HERDR_ENV`, `HERDR_PANE_ID`, `HERDR_SOCKET_PATH` | `HERDR_THREADS_SEAT_NAME` | `HC_TERMINAL_ID`, `HC_TERMINAL_SOCKET`, `HC_SCOPE` |

### 5.2 Environment-specific code (stays in its own module)

**Named (`src/host/named.rs`, 02):**

- `SeatName` (src/protocol/ids.rs), `NamedSeatSource`;
- `seats.name` with `seats_live_name`, and the resolve path's rebind of an unresolved row of the same name. This
  is a fourth C1 mapping path, **named continuity**: an ordinary resolve on the named host whose name equals an
  unresolved named seat's name, with no live seat of that name. It is recorded as `cooperative_named_seat`
  (9, item 3);
- `Command::SelfRetire`, with `retire_seat` factored out of `operator_retire` (seats.rs:4759-4852):
  - Wire `RetireReason::Operator`; locally `operator_label='cooperative_self_retire:<binding>'` (02 C6). Remote
    nodes see an operator retire, which is an accepted limit (9).
  - `seat inspect` and `doctor` show the label, not "operator".
  - It is **refused while another connected top-level binding of the seat exists**, unless `--force`. The reply
    names the other bindings (roast N-T4);
- `display_name` publish and remote materialize;
- own-seat restore (roast N-T3, N-A5, U-T2):
  - A row with `display_name` goes onto the named host, **unresolved**, at target = name.
  - If a live seat of that name already exists, the restored seat stays unresolved for operator repair, with no
    merge and no automatic pick.
  - A row without `display_name` is placed automatically only when every dialled host attached or remembered on
    the node has one kind. Otherwise it is held unresolved for the operator ("holds and asks").
  - The always-attached named host never satisfies `wait:host`.
- optional read-only bindings (D3): `HERDR_THREADS_SEAT_READONLY=1` records the binding with a `readonly` flag.
  That session reads and sends, but its display never ACKs receipt-bearing rows and turn hooks skip them;
- the cross-kind rebind refusal;
- `operator_host` fallback to the built-in host (dispatch.rs:383-396);
- `ensure_default_state_dir` and the lazy host in `src/cli/instance.rs`.

**hc (`src/host/hc/{mod,rpc,wire}.rs`, 06):**

- tolerant wire types (no `deny_unknown_fields`), the kind sniff and `HcDiscovery`. Discovery reads `core.json`
  and sends `hello`, and never spawns `hc` (06 V7);
- the standalone install marker and `LocateCaller` (`src/service/locate.rs`, with the transport peer pid next to
  the uid at src/daemon/transport.rs:430);
- H2, gated on the upstream draft-safe send (10.3): `foreground_contains`, `process_group`,
  `WakeBlock::{NoStatus,StatusDone}`, the 4b outcome table of 05, a wake budget of at least 10 s, and a timeout
  after send treated as "submitted, unknown";
- H3: `CreateTargetPort` and V3, the launch gate, `HostClient`;
- `src/test_support/fake_hc.rs`.

**Herdr TUI:** no logic change, beyond the new defaults and the moved CLI label and topology reads.

### 5.3 Node-wide `attached()` consumers (G1, verified)

| Consumer | Site | Ruling |
|---|---|---|
| idle exit | src/daemon/idle_exit.rs:38-39 `attached_count` | dialled hosts with at least one resolved local seat. Named-only node is store-only for idle exit (spec). An hc core with no resolved seat does not keep the daemon up (roast H-A7). |
| cap, reattach sweep | host_sessions.rs:42, :913, :954 | dialled only (unchanged) |
| Health entries, `observed_release` | host_sessions.rs:508-513; src/app.rs:184-194 | dialled only (unchanged); the built-in host adds no entry (inferred acceptable: it has no failure mode) |
| `startup_primary` (Health, archival, advertised bootstrap capability) | host_sessions.rs:847-852 | fallback `attached().next()` filtered to `native_launch_capability() == HostGuardedStart` (hc can be the lowest id) |
| `first_host_pacer`, `host_down` | src/app.rs:1036, :1050-1053 | first dialled host whose wake lane exists for `Lane::Wakes`; `host_down` unchanged |
| archival evidence | src/service/archival.rs:113, :126 | filter on `archival_evidence() == Supported`. Named and hc return `NoEvidence`, which is never read as an empty composer |
| bootstrap runtime (production) | host_sessions.rs:693-703 | already gated on `native_launch_capability() == HostGuardedStart`; hc and named return `Unsupported` |
| `with_prebuilt_bootstrap` | dispatch.rs:213-219 | test fixture only (callers in tests/); no ruling needed |
| `operator_host` fallback | dispatch.rs:383-396 | dialled first, else built-in; text "no host attached" (G5) |
| `wake_status` | dispatch.rs:268-283 | per-row host via `all_for_reads` |
| `RemoteResolver::answer` | src/service/remote_resolver.rs:140 | `all_for_reads` |
| seat publisher `read_labels` | src/service/seat_publisher.rs:201 | `all_for_reads` |
| peer launch `place()` | src/service/peer_launcher.rs:263 | dialled, filtered by creation capability |
| peer discovery gate `any_reachable` | host_sessions.rs:500; src/app.rs:2279-2282 | `kind()==Herdr` sessions only (it forks `herdr machine list`) |
| `HostTopologies` (new) | dispatch | `all_for_reads` |

There is a regression test for "`attached()` never contains the named host" and one for "idle exit with only the
named host exits".

## 6. API changes

### 6.1 CLI

- **Env read:** `HERDR_THREADS_SEAT_NAME`, `HC_TERMINAL_ID`, `HC_TERMINAL_SOCKET`. No new global flag.
- **New commands:**
  - `seat retire --self` (named): one row per abandoned item (about 30 bytes each) plus `retired <seat>`;
  - `handoff`/`launch --new-terminal`, `--space` into hc (H3);
  - `internal launch-gate`, `internal peer-target-create` (H3; `peer-tab-create` kept as an alias, open decision O6).
- **Lines** (all stderr; size stated per token rule (f)):

  | Line | Size |
  |---|---|
  | creation `herdr-threads: created seat <id> (<name>)` | about 45 bytes + name |
  | named launch refusal | about 45 bytes + name |
  | both-family "neither verified" | about 130 bytes |
  | phase guard "runs in herdr-core terminal tN; not supported yet; set HERDR_THREADS_SEAT_NAME" | about 95 bytes |
  | inherited seat name refusal (4.4) | about 110 bytes + name; an error, not a notice |
  | second-locator notice, once per (seat, locator), daemon-remembered | about 60 bytes |
  | pane-form refusal for a named agent's pane | about 50 bytes |
  | `seat threshold` inert on a no-wake host | about 60 bytes |
  | daemon too old for named/terminal sites (6.2) | about 60 bytes |
  | hooks: guard line | about 110 bytes. It rides lifecycle check-ins and compact re-entry only, never the `guidance_every_turn` section. Hermes (no lifecycle event) gets it on its first delivering hook per binding, remembered by the daemon (roast U-D5) |

- **Explicit flags with a name:** explicit cooperative flags together with `HERDR_THREADS_SEAT_NAME` are refused
  with one line (02 3.1).

### 6.2 Local protocol (daemon socket)

| Item | Who | Compatibility |
|---|---|---|
| `SeatsQuery.host: Option<HostLocator>`, `ResolveThreadQuery` caller host | named, hc | `SeatsQuery` is `deny_unknown_fields` (src/protocol/commands.rs:412-414). The field is sent only when `host_scoped_lookup()`, so Herdr panes keep today's bytes and an old daemon never sees it from a pane. |
| `Command::HostTopologies` | all | new command; an old daemon gives the existing upgrade error |
| `SeatDirectory.name`; directory filled for local seats | all | skew-safe (no `deny_unknown_fields`); old CLIs ignore the field and keep their own label lookup |
| `Command::SelfRetire{claim, operation}` | named | new; in `Command::accountable_claim` (commands.rs:1277); idempotency scope `self-retire:<seat>:<operation>` checked before the live-seat precheck (spike S4) |
| `Command::LocateCaller{candidates, subject}` → `Verified{index, root}` / `Unverified{per_candidate}` | hc (phase F: Herdr candidate only) | sent only for `Ambiguous` |
| `WakeBlock::{NoStatus, StatusDone}` | hc H2 | `#[serde(other)] Unknown` exists (06, results.rs:1745) |
| `CallerClaim` | all | **unchanged**: host is the reserved named locator, a core socket or a Herdr socket; target is the name, `tN` or a pane. The `locator_hint` travels in the resolve command, not in the claim |
| local capability `seat.host_sites_v1` | named (N), hc (H1 extends) | the CLI checks it before the first daemon call from a Named or Terminal site. If it is missing, the CLI prints `herdr-threads: daemon predates named/terminal seats; restart it` (about 60 bytes) instead of a decode error or "Herdr server not reachable" (roast U-A5) |

### 6.3 Daemon

- `attach_builtin` at daemon start, and `all_for_reads`.
- The wake lane is gated on capability.
- `wake_status` uses the per-row host and `idle_wake_on`.
- Persisted no-wake reports for `WakeCapability::None` hosts. `ModChannels::has_live` is not added.
- The audit filters in 5.3.
- The kind probe before `attach_host` (H1, 4.2).
- `spawn_detached` (src/daemon/lifecycle.rs:356-390) removes the union of `ENV_VARS` (G3), plus `HC_SESSION` (06 K5).
  Today it removes only `HERDR_BIN_PATH` (:385-389). This is a fix, not hygiene: hc sweeps every process carrying
  the terminal's `HC_SCOPE` when the terminal ends.

### 6.4 Store schema (one NodeDelta)

```sql
ALTER TABLE herdr_hosts ADD COLUMN kind TEXT NOT NULL DEFAULT 'herdr';   -- no CHECK; validated in code (no rebuild later)
CREATE UNIQUE INDEX herdr_hosts_one_named ON herdr_hosts(kind) WHERE kind='named';
ALTER TABLE seats ADD COLUMN name TEXT CHECK(name IS NULL OR (length(CAST(name AS BLOB)) BETWEEN 1 AND 64
  AND name NOT GLOB '*[^A-Za-z0-9._-]*'));
CREATE UNIQUE INDEX seats_live_name ON seats(host_id, name)
  WHERE name IS NOT NULL AND is_local=1 AND state<>'retired';
```

```sql
ALTER TABLE herdr_hosts ADD COLUMN kind_probed_at INTEGER;   -- NULL = kind came from the default; re-probe on reattach
ALTER TABLE occupant_bindings ADD COLUMN locator_hint TEXT;  -- named bindings: pane id or <core>:<tN> (hint, never authority)
ALTER TABLE occupant_bindings ADD COLUMN readonly INTEGER NOT NULL DEFAULT 0;   -- only if D3 is accepted
CREATE TABLE no_wake_reports (observer TEXT NOT NULL, seat_id TEXT NOT NULL, PRIMARY KEY (observer, seat_id));
```

- **Unknown kind (G4).** A row whose kind is not in this build's implemented-kinds set is attached by nobody. Rebind
  and restore never place on it, and `doctor` names it.
- **`seats_live_name`** covers `unresolved` rows, which `seats_live_target` does not (baseline.sql:1670). This keeps
  a named seat that went unresolved from being split by the next resolve of its name.
- **Provenance.** `occupant_bindings.observation_provenance` does not change (02 C4). The hc handoff journal adds V3
  records; that journal is local, not schema.

### 6.5 Peer wire

- `seat.created.display_name` (already baselined, src/sync/facts/seat.rs:20-28) is set from `seats.name`. On
  receive it is validated; an invalid value is stored as NULL, never rejected.
- `SeatStatusEntry.name` (02 C3) is **dropped**. `remote_seat_status` accepts entries only for seats that
  `seat.created` has already materialized (src/store/remote_seat_status.rs:88-92), and names never change. The
  remote node's `seat_labels` publishes pane = name in the existing `SeatStatusEntry.pane` (src/sync/frame.rs:412-430).
- No new frame, item variant or capability in v1. A later `LaunchTarget::NewTerminal` would carry
  `// cap: PEER_LAUNCH_TERMINAL_CAP` with a `pub const` in src/sync/frame.rs (06 3.4). `RetireReason` is unchanged.
- Spec delta: the named-seats spec's "SeatStatusEntry gains name" line needs a ruling (O1).

## 7. OOP boundaries and module layout

```
src/ports.rs                 HostPort (+kind, wake_capability, target_root_process, foreground_contains),
                             HostKind, WakeCapability, ComposerTraits, CreateTargetPort (H3)
src/host/native.rs           NativeCli (Herdr TUI)            ENV_VARS
src/host/named.rs            NamedHost, NamedSeatSource       ENV_VARS
src/host/hc/{mod,rpc,wire}.rs HcHost, HcDiscovery             ENV_VARS
src/service/host_sessions.rs dialled map + builtin slot; capability-gated wake lane
src/service/locate.rs        LocateCaller (nearest ancestor) + cache keyed (candidates, subject, boot/epoch)
src/cli/caller_location.rs   CallerEnv -> Located -> CallerSite (the only env reader)
src/cli/host_client.rs       H3 only: CLI-side handle for launch/handoff
```

Boundaries:

- Policy (wake, launch, archival, discovery) reads port answers.
- Host modules own protocol and env names.
- The CLI reads env only through `CallerEnv`.
- The store knows the built-in literal `'named'` in two places only: the unique index and the restore and rebind rules (G4 for anything else).

## 8. What changes in 02 and 06 (deduplicated)

**02 (named seats):**

- *Keeps:*
  - SeatName, NamedHost semantics (C1, C2), NamedSeatSource;
  - the built-in host outside `sessions`;
  - `seats.name` + `seats_live_name` and the unresolved rebind by name;
  - self-retire (C6), C4 provenance stance, `display_name`, the restore fix, the cross-kind refusal, the operator fallback;
  - lazy discovery, `ensure_default_state_dir`, `pane_selectors` collapsing to bare argv;
  - the display rules, spikes S1-S6.
- *Changes:*
  - `has_composer` → `wake_capability`;
  - `Lanes` → derived from capability;
  - `idle_wake_on(h, bool)` → three-argument form;
  - `NamedTopology` / `PooledTopology{herdr, named}` → `HostTopologies`;
  - the CHECK on kind → validated in code;
  - the state-dir locator → reserved locator (O2).
- *Drops:*
  - `SeatStatusEntry.name`;
  - `SeatSummary.name` and `ParticipantLocation.name` (→ `SeatDirectory.name`);
  - the CLI name branches in human.rs and peer_locations.rs (→ daemon labels);
  - §9's rank of `HC_TERMINAL_ID` over the pane.

**06 (hc):**

- *Keeps:*
  - HcHost, RPC, wire, the kind sniff, boot = core id (V2), the closure rule (V1);
  - LocateCaller (V3), the `Terminal` locator, `WakeCapability`;
  - the pre-read wake sequence (V4, 5.1), `foreground_contains` / `process_group`;
  - CreateTargetPort + V3 + launch-gate (V6), HcDiscovery (V7), names from doc `herdr-app` (V8);
  - K1-K8, `fake_hc`, the spikes.
- *Changes:*
  - "phase 0" becomes the shared foundation F;
  - phase 1 "capability None" is the same rule as named seats;
  - the kind audit becomes the capability filters of 5.3;
  - the `HostClient` enum is deferred to H3.
- *Drops / defers:*
  - `take_observation_hint` and the events lane (polling every 5 s is enough, K6) become an optional H2 item;
  - the `peer-tab-create` alias moves to O6.

## 9. TRUST-POLICY changes (same commit as the code each covers)

1. **Vocabulary** (TRUST-POLICY.md:121-123, verified). "Herdr boot / incarnation: one Herdr server process" becomes
   "host boot / incarnation", per host:
   - Herdr: a server process;
   - named: one fixed value per node;
   - hc: one core identity (`core.json` id), where a core process change is a newer epoch.

   "Target: a Herdr pane address" (:115) becomes "a host target: pane, name or terminal". (F for the wording;
   N and H1 add their rows.) A known hc locator that answers with a different core id is a new boot: its seats get
   C2 holds and are never matched by `tN`.
2. **A3 provenance** (N). `cooperative_named_seat` is recorded on the named-host observation; binding provenance
   stays the caller's (02 C4). `cooperative_self_retire` is a local `operator_label` (C6).
3. **Named rule** (N). A named-host observation is a recorded cooperative claim, never host evidence. It never moves,
   rebinds or merges a pane or terminal seat, and the converse holds; this is enforced by the cross-kind refusal and
   the host-scoped lookup. The named enumeration never authorizes absence closure. **New C1 row, named
   continuity:** an ordinary resolve on the named host whose name equals an unresolved named seat's name, with no
   live seat of that name, reattaches it (`cooperative_named_seat`). Retire ends continuity for good. **Restore
   rule:** see 5.2. **Role semantics:** concurrent bindings share the role's ACK obligations, and the first display
   discharges them. Parallel workers should use distinct names. A read-only binding (D3) never ACKs.
4. **A1 addendum** (F for the guard, H1 in full). When the env names targets on more than one host, the daemon picks
   the target whose host-reported root process is the nearest ancestor of the caller. With none, the call is refused
   and never falls back. This is cooperative correction of stale env, not anti-spoofing. Accepted limits:
   - a single-family stale env is accepted unverified;
   - a double-forked process cannot be verified when both families are present (04 2A(3));
   - a seat name inherited from a host server is refused (4.4). A name inherited through a parent that is not a
     host we can read is accepted and only made observable: tmux, an IDE, cron, or a long-lived shell that
     spawns terminals. The second-locator notice and `seat inspect` show it;
   - any same-user process with the variable acts as that seat, and a typo mints a stray seat;
   - `seat retire --self` replicates to peers as `RetireReason::Operator` (closed enum, seat.rs:72-79). The local
     `operator_label` is authoritative. A capability-gated `RetireReason::SelfRetired` is a possible follow-up.
5. **Closure** (H1). Pane-close retirement covers hc `killed`, and `removed` of a resolved seat. Exit, signal,
   `server_stop` and `host_shutdown` unresolve the seat (C1 resume reattaches it). `lost` and `restarted` change
   nothing. The C1 structural row covers kept `tN` across a core stop and start, for hc only. There is no new
   `closure_evidence` value. Accepted limit: a wiped or cloned core puts saved seats under C2 holds.
6. **A4 wake.**
   - Named (N) and hc (H1): accepted limit, there is no composer wake. Delivery rules 5 and 6 are degraded: p0
     items, ACK requests, invitations and actionable warnings arrive on the next turn, and nobody wakes the seat
     without a live mod channel. Pokes and recipient-side deadline reminders are absent, and `seat threshold` is
     inert.
   - hc composer wake (H2) exists only behind the upstream draft-safe send (10.3). When it lands:
     - the wake target is the terminal whose foreground process group contains the fingerprint pid;
     - status is self-reported by the program;
     - there is no focus signal;
     - `done` is not wake-ready unless upstream accepts `if:[idle,done]` (accepted limit with an overdue `wake=` rendering).
7. **Launch** (H3). `managed_launch` is extended to "host-guarded launch observed by its host", meaning the
   gate-shim launch with the terminal's root pid fingerprint. C4 bootstrap staging gains the hc launch. The
   `ht.bootstrap` label is a recovery hint only and never reclassifies a recorded unknown.
8. **AGENTS.md** (N, H1). The delivery-contract rules 5 and 6 gain the exception for hosts with no wake, with the
   same wording as item 6.

No adversarial verification is added. Every decision is made by the daemon against the canonical view (A2).

## 10. Delivery-contract check

| Rule | Herdr TUI | Named | hc |
|---|---|---|---|
| 1-2 priority / threshold | unchanged | unchanged (never waking does not break rule 2). `seat threshold` is inert, and setting it prints one line | unchanged; same as named until H2 |
| 3 in-turn delivery | unchanged | unchanged: hooks resolve the `Named` site, binding provenance unchanged (C4) | unchanged: hooks resolve `Terminal` |
| 4 nudge only when idle | unchanged | no nudge unless Channel | `if:idle` is a compare-and-type inside the core. It holds **only if** the harness reports `working` during tools (spike W3); one redundant wake is possible inside the read-to-type window (accepted limit) |
| 5 p0 for ACK/invite/warnings | unchanged | **degraded**: p0 items arrive on the next turn; no pokes or recipient deadline reminders | **degraded** as named until H2 |
| 6 who wakes | Native | Channel only (live, unstalled mod, `mod_suppressed`); otherwise nobody (**degraded**) | nobody until H2, Channel as named; then "the **host** types the wake marker into an empty composer" (AGENTS.md and TRUST-POLICY) |

Hooks-only Claude (no mod) on a named or hc seat classifies as `None`, gets `wake=no_wake`, and nothing wakes it.
That is the same as Codex.

- **Warnings.** No-wake reports for `WakeCapability::None` hosts are persisted (4.3), so a sender is warned once
  per seat, across restarts and channel flaps.
- **Overdue text.** Overdue warnings for such seats read `wake=no_wake`, not `wake=none` (schema.rs:1388-1428).
  The skill meaning of `none` becomes "no native wake: the seat wakes itself through a mod channel, or its host
  cannot type".
- **Skill sentence.** N adds one Warnings sentence to SKILL.md for `wake=no_wake` (about 90 bytes, once per
  session), saying the item arrives on the seat's next turn and to tell your user if it cannot wait.
- **Tripwire.** The registry test (registry.rs:1600-1619) becomes a tripwire: if any built-in host returns
  `WakeCapability::None`, SKILL.md must mention `wake=no_wake`. `has_idle_wake` stays as its oracle (roast U-D3,
  U-A6).

### 10.3 Draft safety on hc (both branches of the composer question)

The task text calls the spike pending, but 04 records it: `hc send --if idle` appends to a typed draft and submits
it. Claude and Codex report `idle` with a draft in the composer (04 Spike 2 C). The design covers both branches; it
is built on B and treats A as a detected capability.

- **Branch B (observed): no composer wake on hc.** A screen pre-read followed by `terminal.send{if:"idle"}` is
  rejected (roast U-A1, H-D1, H-A1, N-D5). The gap is not a narrow race:
  - `send` sleeps until 3 s after the last human keystroke, with no upper bound. It then re-checks only the status
    word and pastes and presses Enter (herdr-core input.rs:105-118, :152-205).
  - So a person who starts typing after our read is waited out, and their draft is submitted with "wake up" at
    their first pause.
  - The plain-text `terminal.read` also cannot tell Claude's prompt suggestion from a draft (01 R5).

  HcHost therefore returns `WakeCapability::None` (the H1 behaviour). Senders get `wake=no_wake` once, and Claude
  with the mod keeps Channel wake. Two upstream asks are filed: an atomic draft-safe send (`send{if_input_empty}`
  or `quiet:"refuse"`, checked inside `prepare`) and `if:[idle,done]`.
- **Branch A (a future hc with the draft-safe send). This is H2.**
  - Detected from `hello.features` / `rpc.discover`, never from a version (AGENTS.md "Harness and Herdr versions").
  - HcHost returns `Composer{submit_budget: 15 s}`. The wake budget covers quiet, settle and the started window. A
    transport timeout after the request was sent counts as "submitted, unknown" (`AwaitingReceipt`, never retyped),
    and `started:false` is a candidate for `unreceived`.
  - The 05 4b outcome table applies.
  - Detection runs at attach, so the change applies on the next reattach.
  - The side-draft e2e case (draft untouched, sender gets the overdue `wake=` warning) is the acceptance test.
- **Fallback if W1 fails** (screen text does not parse under a policy). The pre-read returns `Unreadable` for
  that harness, so its seats get no wake (`TargetUnsafe`). The host's `wake_capability` is unchanged, and only that
  harness's wake on hc degrades.

## 11. Harness-neutral check

- No harness id appears in core. These are all neutral:
  - `HostKind`, `WakeCapability`, `ComposerTraits`, `idle_wake_on`, `CallerEnv`/`CallerSite`;
  - `LocateCaller`, `CreateTargetPort`;
  - the host modules' `ENV_VARS`.
- Per-harness inputs are existing adapter methods (`composer_policy`, `delivery_policy`, `clear_key`, `CallerEnvShape`)
  or neutral defaults added in H3 (`child_scrub_vars()`, `host_launch_argv(kind)` with default none, 06 9).
- The Codex `ManagedDaemon` refusal text "has no pane" (src/harness/codex.rs:1427-1438) stays.
- The Hermes bridge's `PANE_ENV` (integrations/hermes/__init__.py:37) must also pass `HERDR_THREADS_SEAT_NAME` and
  `HC_TERMINAL_*`. This is adapter-owned, so it is in scope of N/H1 (inferred; verify).
- `scripts/check-harness-neutral` needs no allowlist entry (inferred).
- The host-name analogue (`check-host-neutral`, G6) is a follow-up.

## 12. Change inventory

Sizes: S < 150 lines, M < 600, L more, tests included (inferred).

| Module | Phase | Change | Size |
|---|---|---|---|
| src/ports.rs | F | `HostKind`, `WakeCapability`, `ComposerTraits`, 4 defaulted methods | S |
| src/service/host_sessions.rs | F | built-in slot, `attach_builtin`, `all_for_reads`, optional wake lane, `startup_primary` filter | M |
| src/app.rs | F | `first_host_pacer`/`Lane::Wakes` on `Option`, discovery gate by kind | S |
| src/harness/registry.rs | F | `idle_wake_on`, table test against `has_idle_wake` | S |
| src/service/dispatch.rs, archival.rs, peer_launcher.rs | F | `wake_status` per-row host; audit filters (5.3); operator text; `HostTopologies` | M |
| src/store (wake rows `host_id`, `SeatsQuery.host`, local directory, kind column, fail-closed kind) | F | | M |
| migrations/node + src/store/schema.rs | F | one NodeDelta | S |
| src/service/seat_publisher.rs, remote_resolver.rs | F | `all_for_reads`, local labels | S |
| src/cli/caller_location.rs (new) | F | CallerEnv/Located/CallerSite, phase guard | M |
| src/cli/mod.rs, hook.rs, watch.rs | F | consume `CallerSite`; HERDR_ENV gates (hook.rs:1463, :1790, :3485, :4019, :4392) via `locate(Hook)`; never `NoPane` for a found site | L (hook.rs bulk; TRUST-POLICY-sensitive) |
| src/cli/human.rs, peer_locations.rs, follow.rs, seat_resolution.rs | F | labels from `SeatDirectory`, selectors from `HostTopologies` | M |
| src/cli/instance.rs, setup.rs | F | lazy host for non-Pane sites, `ensure_default_state_dir` | M |
| src/service/locate.rs (Herdr candidate only), transport peer pid | F | phase guard | S-M |
| src/daemon/lifecycle.rs | F | env scrub (union of `ENV_VARS`) | S |
| src/protocol/{ids,commands,results}.rs | F/N | `SeatName`, `SeatsQuery.host`, `HostTopologies`, `SeatDirectory.name`, `SelfRetire`, `LocateCaller` | S-M |
| src/host/named.rs (new) | N | NamedHost | M |
| src/store/seats.rs | N | name on allocation, rebind by name, restore fix, cross-kind refusal, `retire_seat` + self-retire | M-L |
| src/sync (`ensure_published`, materialize) | N | `display_name` | S |
| src/host/hc/{mod,rpc,wire}.rs (new), factory sniff | H1 | observation, labels, topology | L |
| src/service/locate.rs (full), HcDiscovery, standalone marker, scripts/install.sh, src/harness/setup.rs | H1 | | M |
| src/test_support/fake_hc.rs (new) | H1 | in-thread fake core | M |
| src/notification/dispatch.rs, `foreground_contains`, `process_group`, HcHost wake, `WakeBlock` variants | H2 (gated on upstream) | | M |
| src/service/locate.rs inherited-name check (`server_process` environ), binding `locator_hint`, second-locator notice, pane-form refusal | N | | M |
| src/store no_wake_reports, kind_probed_at, restore/continuity rules, `RetireReason::Closed` guard on named | F/N | | M |
| src/cli handoff recovery hints via `env_assignments` | N | | S |
| src/test_support isolated-core helper, `check-no-leaked-processes` (`herdr-core serve`/`hold`) | H1 | | S |
| `CreateTargetPort`, protocol/handoff V3, cli/launch_gate.rs, topology_handoff*, peer_launch, cli/host_client.rs | H3 | | L |
| TRUST-POLICY.md, AGENTS.md rule 6, integrations/skill/SKILL.md | F/N/H1/H2/H3 | section 9; environment-neutral skill text (about 160 bytes once per session) | S each |

## 13. Phasing

1. **F, the shared foundation, Herdr-only behaviour.** Seams, audit, migration, daemon labels (fixes K4 on its own),
   env scrub (K5), empty-pane filter, phase guard. Exit gate:
   - acceptance criteria in section 2;
   - the full suite and leak check;
   - the Herdr wake e2e. The hook.rs refactor changes the attribution path, so "no observable change" has to be
     shown, not assumed.
2. **N, named seats.** NamedHost, built-in attach, self-retire, `display_name`, discovery without Herdr. Spikes S1
   and S2 before merge.
3. **H1, hc observation and messaging.** HcHost with capability `None`, so it is observation-only through F's lane
   rule. Also `Terminal` sites, full LocateCaller, lazy attach, finish-reason closure, `<session>:tN` labels,
   discovery, install, skill and the isolated-core test helper. Spikes D1, E1 and H-C1 (Codex hooks in hc with and
   without `--no-daemon`) before merge.
4. **H2, hc wake.** Blocked on the upstream draft-safe send. Once it is detected, flip `HcHost::wake_capability` to
   `Composer` and add `focus_known`/`stash` with their readers. Spikes W1-W5, with W2 covering the post-turn status
   of both harnesses.
5. **H3, launch.** Spikes L1-L4.

Which lands first and what ships independently:

- F lands first and ships on its own: it fixes K4 and K5 and adds the TUI guard (vision decision 12).
- N and H1 are independent after F and can be developed in parallel. N is recommended to merge first: it is smaller,
  it has an approved spec, and it settles the store migration's named rules.
- H2 and H3 each depend only on H1.

## 14. Risks

1. **The hook.rs gate refactor in F** touches every hook path and TRUST-POLICY-governed attribution. Mitigation: a
   precedence matrix test (name × pane × `HERDR_ENV` × hc × empty values × missing socket × invalid name) and the
   Herdr wake e2e.
2. **Snapshot completeness.** An enumeration that misses a live seat retires it: named through the store read plus
   the fence (S1), hc through the finished/`machine/tN` filtering. Both get the `FakeHostWorld` reconcile style.
3. **Conditional wake lane.** Every `session.wake()` user must take `Option` (5.3, src/app.rs:1030-1041). A missed
   one is a compile error, not a silent gap, because the field type changes.
4. **The built-in host leaking into first-host code** through misuse of `all_for_reads`. Guarded by the `attached()` test.
5. **hc as lowest host id** in dialled-only consumers (`startup_primary`, `first_host_pacer`, `place()`, the discovery
   gate). Covered by the filters in 5.3; each gets a two-host test (TUI + fake hc).
6. **Kind sniff** on a TUI socket (D1). It needs a short deadline, and the persisted kind avoids a re-sniff.
7. **`deny_unknown_fields` skew** on `SeatsQuery`. Mitigated by sending `host` only for non-Pane sites.
8. **hc wake depends on upstream.** Until herdr-core ships a draft-safe send, hc seats have no composer wake (10.3).
   After that, the `done` and tool-time `idle` gaps (W2, W3) remain, and rule 4 depends on W3.
11. **Inherited-name check depends on reading the server env** (E1). If the peer pid or environ is unreadable on
    some platform, `server_process()` returns `None` and the check is skipped. The limit then falls back to
    observability only.
9. **hc archival after H2** (5.3). It needs a check that "no evidence" never reads as "empty composer".
10. **Product direction** (herdr-app#63). If the TUI moves onto core, `NativeCli` and `HcHost` stay independent
    behind `HostPort`, and either can be retired.

## 15. Open decisions for the user

Superseded by the combined proposal's Part 5 (`docs/superpowers/specs/2026-10-31-environments-proposal.md`). The
table below is the pre-roast list, kept for traceability.

| # | Decision | Recommendation |
|---|---|---|
| O1 | Drop `SeatStatusEntry.name` from the approved named-seats spec (02 C3; a spec text change) | Drop: `display_name` carries it, no wire change |
| O2 | Named locator: `<state_dir>/node/named-host` (spec, 02) or a fixed reserved path such as `/.herdr-threads/named-host` (B X6; `HostLocator::parse` never stats, src/protocol/ids.rs:298-339 verified) | Fixed path: no kind-match plus endpoint rewrite when a store opens under another path. The spec's locator line is updated. |
| O3 | A3 wording: `cooperative_named_seat` names the named-host observation, not the binding (02 C4) | Accept |
| O4 | Vision decisions 3, 7, 11 and the A1 addendum (verification only when two families are present) | Accept as in 06 |
| O5 | Person seats (`me init`) under a named site: refused in v1 | Refuse; revisit on demand |
| O6 | Keep the `peer-tab-create` alias for one release when `peer-target-create` lands (H3) | Keep |
| O7 | `focus_known=false` on hc: wake a focused app pane without the TUI's 60 s focused-empty hold (spike W5) | Accept pending W5 |
| O8 | Adopt `scripts/check-host-neutral` (G6) after H1 | Yes, as a follow-up bead |
| O9 | Whether the hc wake e2e joins the pre-release wake e2e once H2 ships | Decide at H2; opt-in until then |
