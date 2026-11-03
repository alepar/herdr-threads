# Named seats: code design

Bead: ht-kqrt. Behaviour source of truth: `docs/superpowers/specs/2026-10-31-named-seats-design.md`
(approved). This document is how the code is laid out to implement it; it changes no decision. Where
the source contradicts the spec, the conflict is flagged in section 0 with a proposed resolution that
keeps the spec's observable behaviour. Tree surveyed: `.worktrees/external-seat` at badccf481.
Paths are repo-relative; "(inferred)" marks claims not traced end to end.

## 0. Conflicts with the spec (need a ruling, or are implementation-only)

| # | Spec text | What the code requires | Proposed resolution |
|---|---|---|---|
| C1 | "terminal id and incarnation are derived from the name" | The incarnation is host-wide: a current-target publish is refused unless `proof.incarnation()` equals the active snapshot's incarnation (src/store/seats.rs:570-575); snapshot staging requires every target to carry the capture's single incarnation (seats.rs:852-866, per seam report); reconcile marks a mismatch unresolved (src/identity/reconcile.rs:241-265). | Terminal id derived from the name (unique per name, needed so the moved-pane refusal, seats.rs:3794-3818, never fires). Incarnation and boot are one fixed value per node (derived from the node uuid). Observable behaviour unchanged. Implementation-only. |
| C2 | Named host implied to need no worker ("wake worker is never started") | A host with no published coherent snapshot can never resolve a seat: publish refuses with no `active_snapshot_id` (seats.rs:~564) and identity reads wait for a lane capture (src/identity/repair.rs:609-633). A complete snapshot that omits a live seat retires it (reconcile.rs:331-334). | The named host runs the **observation** lane, not the wake lane. Its `enumerate_targets` lists every non-retired local named seat read from the store. Implementation-only. |
| C3 | "`scripts/check-wire-compat` baseline grows only by that field" | Baseline policy forbids baselining a new field of an existing type (scripts/wire-compat-baseline.txt:8-10; rule R4 in scripts/check-wire-compat). | `SeatStatusEntry.name` gets `#[serde(default, skip_serializing_if = "Option::is_none")]` and **no** baseline line. Note: the field is redundant, because `remote_seat_status` accepts entries only for seats `seat.created` already materialized (src/store/remote_seat_status.rs:88-92) and names never change, so `display_name` suffices. Ruling wanted: keep (spec) or drop. Design keeps it, receive-side sanitized. |
| C4 | A3 provenance `cooperative_named_seat` = "binding observed through the named host" | Adapters key turn delivery and wake text on `observation_provenance == COOPERATIVE_TOP_LEVEL_PROVENANCE` (src/harness/codex.rs:1457/1466, claude.rs:963, hermes.rs:823). Storing a new value there flips a hook-registered named Codex to "no turn delivery" and changes wake text, and would need a branch in every adapter. | Keep `occupant_bindings.observation_provenance` as the caller's provenance. Record `cooperative_named_seat` as the provenance of the named-host **observation** (host kind `named`, observation source in `observed_targets`/snapshot rows), and define it in A3 as such. Binding "named-ness" is derived from `seats.host_id` → host kind. Ruling wanted (wording of the A3 row only). |
| C5 | `wake_status` reports `NoWake` for a Native-wake harness, no block for Channel | `has_idle_wake` is harness-only and true for Claude, Codex, Hermes and agy (src/harness/registry.rs:1447-1459; Claude has both a composer policy and `IdleWake::Channel`). A hooks-only Claude named seat (no mod) is classed Channel but nothing wakes it. | Wake classification takes the host's composer capability and a live mod channel (section 5). A named Claude seat without a live channel reports `NoWake`. This matches the spec's "Claude with the mod" wording; flagged because "harness idle wake is Channel" read literally would not warn. |
| C6 | Self-retire "recorded with provenance `cooperative_self_retire`" | `RetireReason` on the wire is closed `{Closed, Operator}` with no `serde(other)` (src/sync/facts/seat.rs:72-79); `allocation_decisions.kind` CHECK has no self-retire value and changing it is a table rebuild (per seam report, baseline.sql:873). | Wire: `RetireReason::Operator`. Local: `allocation_decisions(kind='operator_retire', operator_label='cooperative_self_retire:<binding>')`. Provenance is recorded locally, honestly, without a wire change. Implementation-only. |
| C7 | "Selector resolution ... reads it like any host topology" | True only for the daemon's `RemoteResolver`. The CLI selector stage reads the Herdr topology client-side through `NativeCli` (src/cli/mod.rs:866-877; src/cli/seat_resolution.rs:646-666). | CLI selectors pool a daemon-supplied named topology with the Herdr one (section 6). Implementation-only. |

## 1. Shape of the change

Two new seams, everything else is plumbing into existing paths:

1. **`NamedHost`** (daemon): a second, in-process `HostPort` implementation, always attached,
   owned by `HostSessions` outside its Herdr map. Seats, bindings, claims, mapping, liveness,
   operator retire and `RemoteResolver` run unchanged over it.
2. **`CallerSite`** (CLI + hooks): one resolved "where is the caller" value replacing the
   `(caller_pane: Option<&str>, RuntimeContext::claim_host())` pair that is threaded separately
   today (src/cli/mod.rs:311, :466, :1489-1504; src/cli/hook.rs:250-262, :1803-1806). Downstream code
   sees `host + target + capability answers` and never branches on Herdr vs named.

Module map:

| Module | Change |
|---|---|
| `src/protocol/ids.rs` | `SeatName` newtype (validation). |
| `src/host/named.rs` (new) | `NamedHost: HostPort`, topology/snapshot synthesis. |
| `src/ports.rs` | `HostPort::kind()` and `HostPort::has_composer()` with neutral defaults; `NamedSeatSource` read trait. |
| `src/service/host_sessions.rs` | `named: Option<Arc<HostSession>>`; `start_session` takes `Lanes`; `resolve`/`by_locator` answer named first; `NamedHostPlan` in `HostAttachPlan`. |
| `src/store/hosts.rs`, `src/store/schema.rs`, `migrations/node/00NN_named_seats.sql` | `herdr_hosts.kind`, `seats.name`, unique index; `HostIndex` gains the named entry. |
| `src/store/seats.rs` | name on allocation; own-seat restore excludes named host; operator rebind refuses cross-kind; `ensure_published` sets `display_name`; remote materialize stores it; self-retire body. |
| `src/service/dispatch.rs` | `wake_status` per-row host capability; `operator_host` falls back to named; `SelfRetire` routing. |
| `src/cli/caller_location.rs` (new) | `CallerEnv`, `CallerLocator`, `CallerSite`. |
| `src/cli/mod.rs`, `hook.rs`, `watch.rs`, `instance.rs`, `seat_resolution.rs`, `human.rs`, `peer_locations.rs` | consume `CallerSite`; discovery for named; displays. |
| `src/sync/frame.rs`, `src/sync/coordinator/seat_status.rs` | `SeatStatusEntry.name` + sanitize. |
| `src/protocol/results.rs`, `commands.rs` | `SeatSummary.name`, `SeatDirectory.name`, `ParticipantLocation.name`; `Command::SelfRetire`; `SeatsQuery.host`. |
| `TRUST-POLICY.md`, `integrations/skill/SKILL.md` | A3 rows, named-host rule, accepted limits; `wake=no_wake` note. |

## 2. Types and traits

### 2.1 `SeatName` (src/protocol/ids.rs)

```rust
/// A named seat's name: 1-64 bytes of [A-Za-z0-9._-], not starting with '-' or '.',
/// not parseable as a SeatId. Also a valid HostTargetId (ids.rs: 1-128 ASCII graphic).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SeatName(String);
impl SeatName {
    pub fn parse(s: &str) -> Result<Self, SeatNameError>;
    pub fn as_target(&self) -> HostTargetId;   // infallible by construction
}
```

The grammar excludes `:`, `/` and `@`. Real Herdr pane ids contain `:` (`w1:p1`, and
`looks_like_pane_id` requires `w..:p..`, src/protocol/address.rs:859-869), so named target ids and
pane target ids are disjoint even where a query filters by target only (inferred: assumes every Herdr
target id contains `:`; section 9 adds a host filter so correctness does not rest on it).

### 2.2 `HostPort` additions (src/ports.rs:3907)

Two defaulted methods, no `ErrorCode::Unsupported` in any default body (the self-test at
src/ports.rs:4533-4572 rejects that):

```rust
/// Host family, for node-wide code that must not treat this host as a Herdr server.
fn kind(&self) -> HostKind { HostKind::Herdr }
/// Whether this host can type into a composer (Native wake, poke, archival).
fn has_composer(&self) -> bool { true }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKind { Herdr, Named }   // stored as herdr_hosts.kind
```

`HostKind` is the one discriminator node-wide code reads (section 4.3). It is neutral (no harness
names), so `scripts/check-harness-neutral` is unaffected.

### 2.3 `NamedHost` (src/host/named.rs)

```rust
pub struct NamedHost {
    boot: HostBootId,                 // fixed per node: derived from local node uuid
    incarnation: String,              // fixed per node (C1), e.g. "named-host:node=<uuid>"
    epoch: AtomicU64,                 // resume_after_epoch(persisted) => persisted + 1
    sequence: AtomicU64,              // strictly increasing per capture and per read
    seats: Arc<dyn NamedSeatSource>,  // store read: non-retired local named seats
    clock: Arc<dyn Clock>,
}

/// Read-only view the named host needs from the store. Implemented by SqliteStore.
pub trait NamedSeatSource: Send + Sync {
    fn live_named(&self, host: HostId, budget: &CallBudget) -> Result<Vec<SeatName>, ApiError>;
}
```

Port behaviour (template: `ServerHost`, tests/daemon/multi_host.rs:139-260):

| Method | Named host |
|---|---|
| `kind` / `has_composer` | `Named` / `false` |
| `native_launch_capability` | `Unsupported` (no `BootstrapRuntime`, host_sessions.rs:~720) |
| `observe_current_target(t)` | if `t` parses as `SeatName`: present, `FreshCurrentTarget`, non-empty `call_id`, terminal = `named:<name>`, `Verified{incarnation, NativeCurrentTarget}`, generation 1, `ui: Unknown`, `focused: false`, sequence `+1`, observed time inside the call window (all of src/ports.rs:860-889 `verified_structural_proof`). Else `Err(InvalidArgument)`. |
| `enumerate_targets` | `complete`, `CoherentVerified`, incarnation kind `CoherentEnumeration` (`authorizes_absence_closure`, ports.rs:908-931); one target per `live_named()` row; sequence `+1`. |
| `live_topology` | one `TopologyPane{target=name, label=Some(name), space/tab = reserved unmatchable ids}` per live name; never `Err` (remote_resolver.rs:145-154 fails the whole answer on any host error). |
| `seat_labels` | one `SeatHostLabels` per name, pane label = name, workspace/tab empty. |
| `safe_wake_target` | `None`. |
| `submit_prompt`, `send_submit_key`, `launch_native`, `pane_agent_state`, `observe_current_target_for_archival` | `Err` (a non-Unsupported code, e.g. `InvalidArgument "named host has no pane"`); never reached because the wake lane is not started and launch is refused earlier. |
| `resume_after_epoch(e)` | `epoch = max(epoch, e + 1)`, as NativeCli (src/host/native.rs:817-820). |
| `incarnation_witness`, `observed_release` | defaults (Unknown / None). |

Race (inferred safe): a first `observe_current_target(name)` publishes a fresh `observed_targets` row
for a name the next enumeration may not yet include; the snapshot carries the lifecycle fence
(`publish_snapshot_stage` fence after `insert_ordinary_allocation` bumps the lifecycle revision, per
seam report seats.rs:~1011-1020, 3211-3234), so the stale snapshot is discarded, not used to retire.
**Spike S1** confirms this with the reconcile tests.

Boot is fixed per node so a daemon restart is a `CarryForward` (same terminal, newer epoch,
reconcile.rs:303-321), never a recovery baseline. The first-ever publication runs `new_recovery` with
no seats; **spike S2** confirms it completes without leaving `baseline_hold_unclaimed`
(src/store/bindings.rs:700-732 skips held hosts in `liveness_candidates`).

### 2.4 HostSessions (src/service/host_sessions.rs)

```rust
pub struct HostSessions {
    /* existing */ sessions: ..., index: Arc<HostIndex>, factory: HostFactory, ...
    named: Option<Arc<HostSession>>,          // new; never in `sessions`
}
enum Lanes { ObservationAndWake, ObservationOnly }
fn start_session(&self, id: HostId, locator: HostLocator, port, observer, lanes: Lanes) -> ...;
pub fn attach_named(&self, plan: &NamedHostPlan) -> Result<(), ApiError>;   // daemon start
pub fn named(&self) -> Option<Arc<HostSession>>;
```

- `attach_named` = `store.attach_host(locator, HostKind::Named)` → `NamedHost::new` → `resume_after_epoch`
  → `start_session(.., Lanes::ObservationOnly)` → `index.publish`. It skips `factory.exists`, the
  canonical-on-disk check and the cap (host_sessions.rs:575-597); the locator is reserved, not dialled.
- `by_locator` and `resolve` (host_sessions.rs:478, :526) answer `named` before the map.
- Because `named` is outside `sessions`, `attached()`, `attached_count()`, `any_reachable()`
  (host_sessions.rs:500), `MAX_ATTACHED_HOSTS` (:42), the reattach sweep (:864-947) and every
  "first attached host" consumer are unchanged, and therefore Herdr-only:
  idle exit (src/daemon/idle_exit.rs:39), discovery gate (src/app.rs:2280-2283), `startup_primary`
  and `on_first_attach`/archival (app.rs:2040-2052, :2466-2490), `first_host_pacer`/`host_down`
  (app.rs:1035, :1050), Health (app.rs:187-194), archival iteration (src/service/archival.rs:113-126),
  bootstrap identity selection (src/service/dispatch.rs:213-219). This is the spec's "outside idle
  exit's count" for free.
- Consumers that **must** see the named host get it explicitly through a new
  `fn all_for_reads(&self) -> Vec<Arc<HostSession>>` (Herdr sessions + named): `RemoteResolver::answer`
  (src/service/remote_resolver.rs:132-157), seat publisher `read_labels`
  (src/service/seat_publisher.rs:195-226), `wake_status` (dispatch.rs:268-271).
- `HostFactory` (host_sessions.rs:74-77) is unchanged; the named host is not built per locator.
  `HostAttachPlan` gains `named: NamedHostPlan { locator, seats: Arc<dyn NamedSeatSource> }`.

### 2.5 `CallerSite` (src/cli/caller_location.rs)

```rust
/// The only reader of HERDR_PANE_ID / HERDR_ENV / HERDR_SOCKET_PATH / HERDR_THREADS_SEAT_NAME.
#[derive(Debug, Clone, Default)]
pub struct CallerEnv {
    pub herdr_env: bool,             // HERDR_ENV == "1"
    pub pane: Option<String>,        // HERDR_PANE_ID, non-empty
    pub pane_host: Option<PathBuf>,  // HERDR_SOCKET_PATH, non-empty
    pub seat_name: Option<String>,   // HERDR_THREADS_SEAT_NAME, non-empty
}
impl CallerEnv {
    pub fn from_process() -> Self;
    pub fn from_lookup(get: &dyn Fn(&str) -> Option<String>) -> Self;   // tests
    /// Name (non-empty) wins and the pane is ignored; invalid name is a hard error
    /// naming the variable, never a pane fallback. Hook mode keeps today's HERDR_ENV gate for panes.
    pub fn locator(&self, mode: EnvMode) -> Result<Option<CallerLocator>, LocatorError>;
}
pub enum EnvMode { Cli, Hook }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallerLocator { Pane(HostTargetId), Named(SeatName) }

#[derive(Debug, Clone)]
pub struct CallerSite { pub host: HostLocator, pub target: HostTargetId, kind: SiteKind }
enum SiteKind { Pane, Named }

impl CallerLocator {
    /// Pane: rt.require_host() + rt.claim_host(), today's errors unchanged (src/daemon/paths.rs:96-115).
    /// Named: named_host_locator(&rt.state_dir); needs no Herdr.
    pub fn site(&self, rt: &RuntimeContext) -> Result<CallerSite, LocatorError>;
}
impl CallerSite {
    pub fn moved_target(&self, clock: &Arc<dyn Clock>, budget: &CallBudget) -> Option<HostTargetId>; // Named: None
    pub fn relative_pane(&self) -> Option<&HostTargetId>;        // Named: None
    pub fn enrolls_on_first_use(&self) -> bool;                  // Named: true (CLI)
    pub fn attempts_continuity(&self) -> bool;                   // Named: false
    pub fn pane_for_agent_probe(&self) -> Option<&HostTargetId>; // Named: None (A4 probe)
    pub fn cache_key(&self) -> String;                           // pane id, or "name:<n>"
}
pub fn named_host_locator(state_dir: &Path) -> HostLocator;     // <state_dir>/node/named-host
```

Enum over trait: two environments, both built in; a trait object pays off only when a host is not
built into core. Extracting `trait CallerEnvironment` from `impl CallerSite` later is mechanical.

`CallerClaim` (src/protocol/authority.rs:8-18) needs no new field: `host` is the named locator,
`target` the name. `HostLocator::parse` accepts any canonical absolute path (src/protocol/ids.rs:298-351),
but its doc ("the wire name of one Herdr server, its canonical socket path") is updated; the named
locator is not required to exist on disk, so the parse must not stat it (verify; inferred that it does not).

## 3. API changes

### 3.1 CLI

- Env `HERDR_THREADS_SEAT_NAME` (new). No new global flag.
- `herdr-threads seat retire --self` (new, agent-facing). Output: one row per abandoned item
  (`ack <id> in <thread>`, `thread <id>`), then `retired <seat>`; model-visible size: about 30 bytes
  per row plus one 20-30 byte summary line (token rule (f), stated in the commit).
- Creation line on stderr: `herdr-threads: created seat <id> (<name>)` (about 45 bytes plus name).
- Handoff/launch whose target resolves to a named seat: refused `named seat <name> has no pane to launch into`.
- `--cooperative-target` cannot express a named seat. Proposal: the env name overrides an explicit
  cooperative selection's host/target when both are present is **not** done; instead explicit
  cooperative flags with the name set are refused with one line. (Inferred gap; no spec text.
  Ruling wanted only if a scripted caller needs the explicit form.)

### 3.2 Local protocol (daemon socket)

| Item | Change | Compatibility |
|---|---|---|
| `Command::SelfRetire { claim: CallerClaim, operation: OperationId }` | new; listed in `Command::accountable_claim` (src/protocol/commands.rs:1277) so the subagent refusal and lazy host resolve apply. New `PermitMutation::SelfRetire`. | Older daemon rejects the unknown command: CLI prints the daemon-upgrade error (existing behaviour). |
| `SeatsQuery.host: Option<HostLocator>` | `#[serde(default, skip_serializing_if = "Option::is_none")]` (commands.rs:412-429). Used by `pane_seat`/`find_seat_for` so seat-by-target is host-scoped. | `SeatsQuery` is `deny_unknown_fields`: a new CLI against an old daemon fails when it sets the field. Set it only for named sites (old daemons have no named seats anyway). |
| `ResolveThreadQuery` caller target | adds the same optional host. | as above |
| `SeatSummary.name`, `SeatDirectory.name`, `ParticipantLocation.name: Option<SeatName>` | `#[serde(default, skip_serializing_if = "Option::is_none")]` (results.rs:1083-1140, :1769-1784). | These are `deny_unknown_fields`: an older CLI fails to decode a page containing a named seat (inferred). Same precedent as `directory` (results.rs:1104). Accepted: daemon and CLI ship as one binary. |
| `NamedTopology` query (or `SeatsQuery.filter.named_only`) | returns live local named seats for CLI selectors (section 6). | new command; same fallback as `SelfRetire`. |

### 3.3 Store schema (`NodeDelta` 12, src/store/schema.rs:93-136)

```sql
ALTER TABLE herdr_hosts ADD COLUMN kind TEXT NOT NULL DEFAULT 'herdr'
  CHECK(kind IN ('herdr','named'));
CREATE UNIQUE INDEX herdr_hosts_one_named ON herdr_hosts(kind) WHERE kind='named';
ALTER TABLE seats ADD COLUMN name TEXT CHECK(name IS NULL OR
  (length(CAST(name AS BLOB)) BETWEEN 1 AND 64 AND name NOT GLOB '*[^A-Za-z0-9._-]*'));
CREATE UNIQUE INDEX seats_live_name ON seats(host_id, name)
  WHERE name IS NOT NULL AND is_local=1 AND state<>'retired';
```

- `kind` over endpoint-string compare: the endpoint depends on `--state-dir`; the kind column survives
  a store opened under another path, and `attach_host` for the named host matches by kind and rewrites
  the endpoint (inferred design).
- `seats_live_name` also covers `unresolved` rows, which `seats_live_target` (resolved only,
  migrations/node/0001_node_baseline.sql:1670) does not. A named seat that went unresolved would
  otherwise be split by the next resolve of its name, because `ordinary_resolution_apply` matches only
  `state='resolved'` (seats.rs:3778-3788). With the index, that insert fails closed. The resolve path
  for a named host also first rebinds an `unresolved` row of the same name (name equality is the
  continuity evidence, spec). This is the one place named seats behave differently in the store, gated
  on host kind.
- `remote` rows store `name` from `seat.created.display_name` (target_id is NULL on remote rows by CHECK).
- No change to `occupant_bindings.observation_provenance` (no CHECK there; C4).
- Retire reuse after retire: `ordinary_resolution_apply` ignores retired rows, so a new seat is minted
  with no code change; the partial index permits it.

### 3.4 Peer wire

- `SeatCreatedPayload.display_name` (src/sync/facts/seat.rs:20-28, already baselined): set from
  `seats.name` in `ensure_published` (src/store/seats.rs:222, now `None`). Old nodes ignore it.
- `materialize_seat_created` remote branch stores it as `seats.name` after grammar validation
  (invalid: stored NULL, never rejected; one remote bad name must not stop the stream).
- `SeatStatusEntry.name` (src/sync/frame.rs:411-430): `#[serde(default, skip_serializing_if =
  "Option::is_none")]`, no baseline line (C3). `sanitize` (src/sync/coordinator/seat_status.rs:104-135)
  filters it to the grammar like `harness`.
- No new frame, item variant or capability; `RetireReason` unchanged (C6).
- Own-seat restore (`materialize_seat_created`, local origin, seats.rs:270-296) picks
  `herdr_hosts ORDER BY last_attached_at DESC`; the named host is stamped every start and would win.
  Change: for a restored seat with `display_name` set, place on the named host with `target_id=name`,
  `state='unresolved'`; otherwise pick among `kind='herdr'` only. If a live seat of that name already
  exists locally, the restored one stays unresolved with `name` NULL and a warning (inferred; **spike S5**).

## 4. Daemon flow

### 4.1 Resolve (first use)

`ResolveSeat{host=named locator, target=name}` → `HostSessions::resolve` returns the named session →
`OrdinaryIdentity::resolve` (src/identity/repair.rs:254-282) → `NamedHost::observe_current_target` →
`publish_current_target_observation` (needs an active snapshot: the lane published one at start) →
`ordinary_resolution_apply` → `insert_ordinary_allocation` sets `seats.name` when the host kind is
`named` (seats.rs:3211-3234). The reply carries `created: bool`; the CLI prints the creation line.

### 4.2 Claims and mapping

`cooperative_claim` → `HostIndex::attached_id(&claim.host)` (the named entry is published) →
`cooperative_mapping` (seats.rs:5131-5307): needs `effective_observation` (src/store/effective.rs:55-123),
satisfied by the lane's snapshot listing every live name. `binding_evidence` needs terminal and
incarnation: always set. Liveness (`liveness_candidates`, bindings.rs:700-732) works with the named
row's `node_id` = local and no hold.

### 4.3 Node-wide guards keyed on `HostKind`

- `operator_host` (src/service/dispatch.rs:383-396): fallback to `named` when no Herdr host is attached,
  so `human seat retire --operator` works on a Herdr-less node.
- Operator rebind / replace / fresh-seat targeting (seats.rs:4176 `rebind_unresolved_seat` rewrites
  `host_id`): refuse when source and destination host kinds differ ("a pane observation never moves a
  named seat", and the converse).
- Handoff/launch target resolution: a target on a `Named` host is refused with the spec text before any
  `CreateTabPort` call.

### 4.4 Self-retire

Dispatch: cooperative arm (dispatch.rs:1648-1676) through `cooperative_mutation`
(dispatch.rs:1053-1196), which already enforces owner-peer, instance and `role != TopLevel` refusal.
`claim_bootstrap_runtime` yields `None` for the named host (no `HostGuardedStart`).

Store, one deciding transaction:
1. `cooperative_claim` + host kind `named` (pane seat: refused `pane seats retire on pane close or by
   the operator`).
2. `cooperative_bound_mapping` (seats.rs:5313): the caller's own open binding.
3. Read abandoned rows (open ACK obligations, joined threads) for the reply.
4. Shared retire body factored out of `operator_retire` (seats.rs:4759-4852):
   `fn retire_seat(tx, seat, wire: RetireReason, label: RetireLabel)` → `begin_retirement_authored`
   (src/store/control/retirement.rs:128-170), allocation decision, `lift_baseline_hold_if_clear`.
Idempotency scope: `self-retire:<seat>:<operation>`, checked before the live-seat precheck so a replay
after success returns the stored result, not `NotFound` (**spike S4**).

## 5. Wake and the delivery contract

### 5.1 Mechanism

- No wake lane for the named host (`Lanes::ObservationOnly`). Composer wake and pokes run only in the
  wake worker (src/service/workers.rs:2081-2113, :2201), whose candidates are host-filtered
  (src/store/wake.rs:1271), so named seats are never typed into. `wake_work` rows are still upserted
  per seat (src/store/delivery_tail.rs:109-111): bounded, one per seat (inferred harmless; **spike S6**
  checks doctor does not flag them as stuck).
- Channel wake (Claude mod) is independent of the host: the mod submits `wake up` from watch-line
  `wakes` flags (integrations/claude/mod/hooks/register.js:556-669). `watch` inherits the session env
  (inferred; **spike S3**) so it resolves the same named seat, and must never return
  `WatchStatusReason::NoPane` for a named site, which stops the mod permanently (register.js:284-312).

### 5.2 Classification (`wake_status`, dispatch.rs:252-301)

Registry gains a neutral function of harness and host capability:

```rust
// src/harness/registry.rs
pub fn idle_wake_on(&self, harness: &str, host_has_composer: bool) -> Option<IdleWakeOn>;
pub enum IdleWakeOn { Composer, Channel, None }
```

- `host_has_composer = true`: today's `has_idle_wake` result (no behaviour change for pane seats).
- `false`: `Channel` iff `delivery_policy().idle_wake() == Channel`, else `None`.

The store's `WakeStatus` rows gain `host_id` (src/store/queries.rs:~6515; `SeatWakeStatus`,
src/protocol/results.rs:1760-1768, gains no wire field: the host is looked up daemon-side).
`wake_status` then:
- `None` → `NoWake`;
- `Channel` on a composer-less host → `NoWake` unless `mod_channels` (dispatch.rs:114) reports a live
  channel for the seat (C5; needs a `ModChannels::has_live(seat)` query, defaulted `false`, inferred
  that no such method exists today);
- `Composer` → existing `wake_block` scan.
The existing `(observer, seat)` `first` bookkeeping (dispatch.rs:286-299) gives the one-time
`wake=no_wake`. No harness id appears in core.

### 5.3 Contract check

1-3 unchanged (priorities, threshold, in-turn hook/channel delivery). 4: a named seat gets no nudge
unless its harness wakes itself. 5: p0 items still go through delivery; for a NoWake seat they arrive
on the next turn. 6: Channel unchanged; Native is the single degraded capability, keyed on the observed
host capability, reported once. Invariant 2 does not hold for named Native-wake seats: TRUST-POLICY
accepted limit + one SKILL.md sentence under Warnings (size: about 120 bytes, once per session). The
`idle_wake_classification_of_the_builtins` test comment (registry.rs:1600-1619) and
src/cli/wake_warning.rs:1-11 state no production seat produces `no_wake`; both are corrected.

Overdue warnings for named seats read `wake=none` (src/store/schema.rs:1393-1428 fallback), never
`unreceived`/`refused`, matching "no retries".

Turn delivery is untouched because binding provenance stays the caller's (C4):
`binding_turn_delivery`/`binding_wake_text` keep their inputs.

## 6. CLI and hooks

### 6.1 Wiring

- `cli::run` (src/cli/mod.rs:311) builds `CallerEnv::from_process()` and resolves `locator(Cli)` once,
  before the watch precheck (src/cli/watch.rs:162-177 takes `locator.is_some()`).
- `run_in_pane`/`run_parsed_in_pane` take `Option<&CallerLocator>`; after `resolve_context`, `site()` is
  computed lazily so Pane errors stay at the point of use.
- `located_pane_seat` → `locate_caller_seat(client, site)` (mod.rs:3766-3796): `pane_seat(site)` with
  host filter → `site.moved_target()` → if `enrolls_on_first_use()`, `ResolveSeat` and the creation
  line. The "never allocates" doc invariant (mod.rs:3534) becomes "never allocates a pane seat".
  `launch::resolve_seat` takes the concrete `LocalSocketClient` (src/cli/launch.rs:287-317); generalize
  it to `C: LocalClient`.
- `derive_selection` (mod.rs:3815-3925): agent origin uses `site.target`/`site.host`. Person origin with
  a named site is refused (spec covers agent sessions only; `me init` and the A4 `pane_agent` probe are
  pane-only). Ruling wanted only if persons need named seats.
- `run_caller_scoped_deferred` (mod.rs:1489-1504), `body_reader_claim` (:1792-1830),
  `watch::locate_claim` (watch.rs:1166/1171): claim host from `site.host`.
- Relative selector closures (mod.rs:766-780, :839-844): `site.relative_pane()`.
- Update-notice and nick-cache keys: `site.cache_key()` (mod.rs:1282, src/cli/follow.rs:1077-1082).

### 6.2 Hooks (src/cli/hook.rs)

- `HookEnv` (hook.rs:248-262) becomes `CallerEnv`; `foreign_session` and the gates at :1463, :3485,
  :4019, :4482 become `env.locator(Hook)` is `Ok(Some(_))`.
- `connect_pane` → `connect_caller` returning `Connected{.., site}`; the unconditional `require_host`
  (:1803-1806) moves into `site()` for Pane.
- `check_in` (:1890-2060): skip C1 `reattach_by_continuity` when `!site.attempts_continuity()`;
  `resolve_seat(.., &site.host, &site.target)`. `PaneCall` gets `host: &HostLocator` replacing the four
  `context.claim_host()` calls (:2362, :2560, :2854, :2921).
- `report_binding_event` (:3913) and HookStep (:3648) use `site.host`.
- `pane_context_journal`/`record_observer_reset` (:3779-3848) match host and target.
- `pane_selectors` (:1293-1317): for a named site, `state_agrees` alone collapses to bare argv, so
  ready commands carry no `--state-dir` (saves the bytes; token rule).

### 6.3 Discovery (src/cli/instance.rs)

`InstanceInputs` gains `named: bool`.
- `resolve_state_dir` with `named` and no flag/env: `default_state_dir_to_create` (instance.rs:192-199),
  create mode 0700, factored out of `resolve_peer_context` (:204-235) as `ensure_default_state_dir`.
  `detect_state_dir` (`herdr plugin list`, src/cli/setup.rs:279-316) is never reached.
- `resolve_host_endpoint` (:351-363): with `named`, stop after the default-socket filesystem check, never
  fork `herdr status server` (setup.rs:320-339). Today `resolve_context_with_selected_paths`
  (:382-394) resolves the host eagerly for every command; with `named` it becomes lazy (spec: "resolved
  only when a command needs a pane target"). The daemon autostart's `--host-endpoint` hint stays an
  attach hint (src/daemon/lifecycle.rs:369-371).
- `setup` with `--state-dir` and no `herdr` on PATH: verify; fix if not (spec item, unexamined).

### 6.4 Selectors

The selector stage (src/cli/mod.rs:866-877) gets a composite topology source:

```rust
trait TopologySource { fn topology(&self, budget: &CallBudget) -> Result<HostTopology, ApiError>; }
struct PooledTopology { herdr: Option<NativeCli>, named: NamedTopologyClient }
```

`PooledTopology` merges Herdr's topology (if reachable) with the daemon's named topology. A Herdr
failure is fatal only when the named part has no match. `Selector::resolve_with` already pools and
returns `Ambiguous` (src/protocol/address.rs:350-378). Remote `name@B` is unchanged: `split_machine` →
home `RemoteResolver`, which reads the named host's `live_topology`. Synthetic space/tab ids on named
panes must be unmatchable, so the space-only and tab-only alias readings (address.rs:461-466) never
hit them.

### 6.5 Displays

- `seat list` (src/cli/human.rs:246-297): name in the PANE column, SPACE/TAB `-`; remote `name@node`.
- `thread participants` (src/cli/peer_locations.rs:131-184): name branch before label lookup.
- Delivery `from` headers: `sender_name = name` (or `name@node`) through the existing slot
  (src/delivery/assemble.rs:328-333; src/delivery/frame.rs:110-125; register.js `label()`). Size:
  replaces a 12-byte seat id with the name (up to 64 bytes; typically shorter); fixtures in
  `tests/fixtures/delivery/frame-*` updated and the size stated in the commit.
- Seat publisher (src/service/seat_publisher.rs:58-77): labels from the named port's `seat_labels`,
  `name` from `seats.name`. Presence for named seats reads `idle`/`offline` only (no `ui_state`),
  inferred acceptable.

## 7. TRUST-POLICY touch points (same commit)

- A3 table: `cooperative_named_seat` (named-host observation: "a caller presented this name";
  recorded on the observation/host, C4) and `cooperative_self_retire` (local `operator_label`, C6).
- New rule: a named-host observation is a recorded cooperative claim, never Herdr evidence; it never
  moves, rebinds or merges a pane seat, and a pane observation never moves a named seat (enforced by
  the cross-kind refusal, 4.3, and disjoint target namespaces + host filter).
- Continuity: name equality on one node is the continuity evidence (the unresolved-rebind in 3.3);
  retire ends it for good (retired is terminal by trigger, baseline.sql:2096-2098, per seam report).
- Accepted limits: any same-user process with the variable acts as that seat; a typo mints a stray seat;
  Native wake absent for named seats (invariant 2).
- No adversarial verification added; the decision stays in the daemon against the canonical view.

## 8. Harness-neutral check

No harness id in core: the named host, `CallerSite`, `HostKind` and `idle_wake_on` are neutral. The
only adapter-owned text touched is the Codex `ManagedDaemon` refusal ("has no pane",
src/harness/codex.rs:1427-1438), which stays as is: the managed daemon is still refused for named seats
(inferred correct; it has no session of its own). The Hermes bridge reads its own `PANE_ENV`
(integrations/hermes/__init__.py:37); it passes the environment through to the CLI, so it needs no
change unless it gates on the pane (verify).

## 9. Third environment (herdr-core / herdr-app terminal host)

General enough as designed:
- `HostKind` gains `Terminal`; `herdr_hosts.kind` CHECK widens (a table rebuild, so better: define the
  column without a CHECK and validate in code, decided at migration time).
- `CallerLocator` gains `Terminal{ socket: PathBuf, id: HostTargetId }` from `HC_TERMINAL_SOCKET` and
  `HC_TERMINAL_ID`; `site()` maps to `host = canonical socket path`, `target = tN`. `HostLocator` as a
  canonical path fits a socket; `CallerClaim` again needs no field.
- `HostFactory.build` dispatches by kind to an `HcHost: HostPort` (RPC client), registered like
  NativeCli. `has_composer()` decides wake (hc `send` waits for typing to stop, per
  04-mac-spikes.md step 4), `kind()` keeps it out of Herdr-only code.
- `PooledTopology` takes a list of sources, not two fields.

Not general yet (follow-ups, not in v1):
- The 15 direct `NativeCli::new` sites in the CLI (03-ht-host-coupling.md §2) need a CLI-side host
  factory; v1 only routes caller location and selectors through new seams.
- Env precedence: the mac spike found hc terminals inherit the core process env, including a stale
  `HERDR_PANE_ID` of whatever pane autostarted the core (04-mac-spikes.md step 3). `CallerEnv::locator`
  must rank `HC_TERMINAL_ID` above `HERDR_PANE_ID` when that variant lands; the named-first precedence
  already shields named seats.
- `BootstrapObserver` hard-codes the Herdr identity format (src/ports.rs:1386-1404).

## 10. Test strategy (seam level)

- Unit: `SeatName` grammar (boundaries 0/1/64/65, leading `-`/`.`, `@`, `/`, `:`, seat-id-shaped);
  `CallerEnv::locator` precedence matrix (name × pane × HERDR_ENV × invalid name); `NamedHost`
  observation satisfies `verified_structural_proof`; snapshot satisfies `authorizes_absence_closure`;
  `idle_wake_on` table for every builtin × composer.
- Store (in-process, `FakeHostWorld` pattern, src/test_support/fake_hosts.rs): named host lane publish
  then resolve; restart with persisted epoch → `CarryForward`, zero transitions (S1/S2); unresolved
  named seat rebinds, never splits; `seats_live_name`; own-seat restore excludes named host; cross-kind
  rebind refused; self-retire idempotent replay.
- Daemon (`tests/daemon/multi_host.rs` style): named host never in `attached()`; idle exit with only the
  named host; `RemoteResolver` answers `reviewer`; `wake_status` NoWake once per pair; channel-live
  Claude reports no block.
- CLI/hook integration (isolated state dir, no Herdr): spec list. Beside-Herdr in an isolated Herdr
  session (scripts/lib/isolated-herdr.sh). Multinode (`test_support::multinode`): `reviewer@B`,
  old-node decode of `display_name` and `SeatStatusEntry.name`.
- New suites with no process-wide state go in `tests/combined.rs`; in-process daemons get their own
  `[[test]]` with `required-features = ["test-support"]`.

## 11. Risks

1. **Snapshot completeness** (C2): a named port enumeration that misses a seat retires it. The store
   read and the lifecycle fence carry correctness; S1 must prove the race safe. Highest risk.
2. **Unresolved split**: any path that unresolves a named seat (invalidation, generation drift) plus a
   later resolve; mitigated by `seats_live_name` and the named rebind, but every `MarkUnresolved` source
   for the named host should be unreachable by construction (fixed boot, generation 1).
3. **Node-wide "first host" code** silently picking the named host: avoided by keeping it out of
   `sessions`; regressions caught by the `attached()` test.
4. **deny_unknown_fields skew** on `SeatsQuery`/`SeatSummary`/`ParticipantLocation`: CLI-daemon version
   skew breaks decoding; accepted as single-binary, but the field is sent only when needed.
5. **Mod watch** stopping on `no_pane` for a named site, and env inheritance into `watch` (S3).
6. **Discovery** eagerly forking `herdr` today; the lazy-host change touches every CLI command's
   context resolution.

Spikes: S1 snapshot/allocation race and restart CarryForward; S2 first publication with no seats
clears holds; S3 mod `process.spawn` env inheritance; S4 self-retire replay scope; S5 restored named
seat vs a newer local seat of the same name; S6 doctor on never-settled `wake_work` of named seats.
