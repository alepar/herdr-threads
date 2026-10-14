# Cross-node sync: multi-host daemon (HostSessions) — design

Parent: [root spec](2026-10-14-cross-node-sync-design.md) · Bead: ht-5fr.2

## Goal

One node daemon serves every Herdr server of its machine user at once, one `HostSession` each, and accepts a caller from a pane on any of them when its claim names the server it sits on. Wakes, observation and archival sampling run per server, so a server that stops answering or was never attached freezes only its own seats while the other servers, store-only commands and sync keep working.

## Problem description

After ht-5fr.1 the node daemon attaches at most one Herdr server, the *primary host*. That host is held in three places: `Option<HostSession>` in `run_elected_impl`, `StoreContext.primary_host` (an `OnceLock<i64>` read by every Herdr-scope query), and the descriptor's `primary_host`. A client-side interim guard refuses a claim from any other server. Everything below the daemon still assumes one server. `CallerClaim` (`src/protocol/authority.rs`) names a pane `target` but not the server it belongs to, and target ids are only unique within one server. `seats::cooperative_instance` (`src/store/seats.rs`) checks `claim.instance` against the store instance. The observation, wake and archival workers (`src/service/workers.rs`, `src/service/archival.rs`) are each spawned once with one `Arc<dyn HostPort>` and one `HostReachability`. The wake lane freezes as a whole when that one host is down. The Herdr-scope child tables (`snapshot_generations`, `observed_targets`, `recovery_*`, `allocation_decisions`, `archival_instances`, `seat_archival`) are still keyed by `instance_id`, which is now a per-node constant. The bootstrap runtime has one `HandoffNamespace`, and Health reports one `host` component. Root §3 requires one daemon to serve N servers, with `HostSessions` as the one routing point keyed by `host_id`.

## Main challenges

Target ids are unique only within one Herdr server. Every query that matches a pane by `target_id` must therefore also match the server, and reconciliation must never close a seat on server A because a coherent snapshot of server B lacks its target. The single primary-host seam must be cut away everywhere: each query that read the primary host takes its host from the claim, the seat row or the worker that runs the query. Observation, wakes and archival sampling are host-bound, but `CommitKicks` has one pacer slot per lane and a thread's members can sit on different servers, so the design must decide which lanes become per host and which stay node-wide. Archival coherence today is one global flag and epoch, and an outage on one server must not reset quiet periods for threads that live entirely on another. Lazy attach happens on the request path: it must be bounded, happen once under concurrent first claims, never expose a `host_id` to the store before that host's workers run, and leave every spawned worker stoppable at shutdown. Seat continuity (TRUST-POLICY C1 and operator rebinds) must keep a seat's host attributable, and a seat's host must never change silently.

## Key decisions made

The wire gets one new value type, `HostLocator`, which is the canonical Herdr socket path. `CallerClaim` carries it as `host`, and so does every target-bearing command. `HostId` is the node-local `herdr_hosts.host_id` and never leaves the daemon. `HostSessions` (in `src/service/host_sessions.rs`) is a registry of at most 16 `HostSession`s. It attaches sessions at three moments: at startup (the `--host-endpoint` hint plus every remembered row whose socket exists), lazily in dispatch when a command names an unattached locator, and from a 30 s sweep. Sessions are never detached. A small shared `HostIndex` (`src/store/hosts.rs`) replaces `StoreContext.primary_host`. HostSessions publishes a session into the index only after the session's workers run. The store reads the index. The store entry check `cooperative_instance` becomes `cooperative_claim`, which returns the claim's `HostId`. It requires the claimed seat to be local and to sit on that host. Observation and the wake dispatcher become **per-host workers** owned by the session. `CommitKicks` fans a lane kick out to every registered pacer. Archival stays **one node-wide lane**: the session owns the *sampler* (its port plus its archival evidence), and per-host evidence rows (`archival_hosts`) make an outage affect only seats on that host. Every Herdr-scope child table is re-keyed to `host_id` in the editable baseline. Local seats always carry a `host_id`. A rebind sets `host_id` together with `target_id`, so a seat can move between servers of its own node only by an explicit rebind (C1 or operator). The bootstrap runtime and its `HandoffNamespace` are per session. Health reports one entry per attached host. ht-5fr.1's interim client guard, the descriptor's `primary_host` and `HostChoice` are deleted.

## Decision points

### 1. Host identity types: `HostLocator` on the wire, `HostId` in the daemon

**Recommended.**
- `pub struct HostLocator(String)` lives in `src/protocol/ids.rs` and serializes as a string. It is the Herdr server socket path in the form `RuntimeContext::explicit` canonicalizes it (`fs::canonicalize`). `HostLocator::parse` accepts 1..=4096 bytes, which matches the `herdr_hosts.endpoint` CHECK from ht-5fr.1. The path must be absolute, contain no NUL, no empty, `.` or `..` component, and no trailing `/`. Anything else is `InvalidRequest("host locator is not a canonical absolute path")`. It has `as_path()` and a `From<&HostLocator> for PathBuf`.
- `pub struct HostId(i64)` lives in `src/ports.rs` (`Copy`, `Ord`). It is `herdr_hosts.host_id`. It is never serialized to clients, never printed to agents, and never replicated.
- **`CallerClaim`** gains `pub host: HostLocator`. The field is required, so `deny_unknown_fields` stays and clean start allows the change. Every claim builder takes it from `RuntimeContext::require_host()` (ht-5fr.1), which already canonicalizes. The builders are in `cli/hook.rs`, `cli/mod.rs`, `cli/handoff*.rs`, `cli/direct_send.rs`, `cli/topology_runtime.rs`, `cli/lazy_display.rs` and `cli/journal.rs`; `grep -n "CallerClaim {" src` lists them. `cli/journal.rs`'s claim digest (`digest.update(claim.instance…)`) also hashes `host` as length-prefixed bytes after `instance`.
- **Target-bearing commands** gain `pub host: HostLocator` next to `target`: `ResolveSeat`, `ContinuityCheckIn`, `OperatorRebind`, `OperatorFreshSeat` and `OperatorReplace` (`src/protocol/commands.rs`). `OperatorRetire` and `OperatorOrphanInvite` name no target and stay host-free.
- `Command::host_locator(&self) -> Option<&HostLocator>` is the one place that returns the claim's host or the command's explicit `host`. Dispatch uses it (§3).
- `PROTOCOL_VERSION` (`src/protocol/wire.rs`) goes up by one, so an older CLI fails on the existing version check and never sends a claim without a host.
- `CooperativeDecisionFence` gains `pub host: HostLocator`. `MutationPermit::consume_cooperative` adds `|| claim.host != fence.host` to its "cooperative context changed" test. `DecisionFence` (native) is unchanged: its `host_boot`/`host_epoch` come from the seat's host row (§5).
- **Handoff.** The handoff namespace already names its server (`HandoffNamespace.host_endpoint`). Every existing `claim.instance != namespace.instance` comparison also requires `claim.host.as_path() == namespace.host_endpoint`. The sites are `protocol/handoff.rs` (the plan and payload checks), `protocol/handoff/automatic.rs`, `cli/handoff_delivery.rs`, `cli/direct_send.rs`, `cli/handoff.rs` and `cli/journal.rs`'s `IntentScope::Cooperative` match. `IntentScope::Cooperative` gains `host` as well.

**Considered.**
- Putting `HostId` on the wire: the client cannot know it before the daemon attaches the host, and it is node-local state that must not leak.
- A hash of the endpoint as the locator: this adds a mapping table and still needs the path for attach.
- `Option<HostLocator>` in the claim: every claim comes from a pane, and an optional field would bring back the ambiguity this subepic removes.
- Bundling `{host, target}` into one `PaneRef` type: cleaner, but it churns every target consumer for no behaviour change.

### 2. `HostSession` and `HostSessions`

**Recommended.** A new core module `src/service/host_sessions.rs`. It composes service workers, so it lives in `service` and not in `host`.

```rust
pub struct HostSession {
    host_id: HostId,
    locator: HostLocator,
    port: Arc<dyn HostPort>,                 // NativeCli in production
    observer: Arc<dyn BootstrapObserver>,
    reachability: Arc<HostReachability>,
    identity: Arc<OrdinaryIdentity>,         // per-host preparation/repair
    bootstrap: BootstrapRuntime,             // HandoffNamespace{instance: node_uuid, state_dir, host_endpoint: locator}
    evidence: Arc<HostEvidenceStatus>,
    observation: LaneHandle,                 // Pacer + WorkerStatus
    wakes: LaneHandle,
    stop: Cancellation,
}
pub struct HostSessions {
    sessions: RwLock<BTreeMap<HostId, Arc<HostSession>>>,
    index: Arc<HostIndex>,                   // shared with StoreContext (§4)
    factory: HostFactory,
    attach_lock: Mutex<()>,
    deps: SessionDeps,                       // store, writer, clock, kicks, harness/mod/pull sources, worker slot, log
}
pub struct HostFactory {
    pub build: Arc<dyn Fn(&HostLocator) -> (Arc<dyn HostPort>, Arc<dyn BootstrapObserver>) + Send + Sync>,
    pub exists: Arc<dyn Fn(&Path) -> bool + Send + Sync>,   // production: FileTypeExt::is_socket
}
pub const MAX_ATTACHED_HOSTS: usize = 16;
```

- **Read API.** `get(HostId) -> Option<Arc<HostSession>>`, `by_locator(&HostLocator)`, `attached() -> Vec<Arc<HostSession>>`, `attached_count()`, `any_reachable() -> Option<Arc<HostSession>>` (ht-5fr.7 discovery runs `herdr machine list` through it), and `health_entries()` (§10).
- **Attach API.** `resolve(&HostLocator, &CallBudget) -> Result<Arc<HostSession>, ApiError>` returns an attached session or attaches it now (§3). `attach_startup(hint: Option<HostLocator>)` and `stop_all()` complete the API.
- `HostPort` keeps its single-host shape: each session holds its own port (root §3, the per-host handle option). Nothing outside a session holds a node-wide `HostPort`.
- `HostChoice` (ht-5fr.1) is deleted. `run_elected_guarded`/`run_elected_impl` take `HostAttachPlan { hint: Option<HostLocator>, factory: HostFactory }`. `main.rs` passes a `NativeCli` factory with `exists = is_socket`. Tests pass a factory whose `exists` and `build` read a map of fake locators to `FakeHost`s. That map is also the attach path the multi-node harness (ht-5fr.10) consumes. It is a struct of two closures, not a trait: there is one production implementation.
- The node-wide lanes stay single instances, unchanged: deadline, retention, liveness, mod channels, admission observer and (from other subepics) Apply/Sync. Only observation and wakes become per session (§6, §7). Archival is one node-wide lane that routes samples to sessions (§8).

**Considered.**
- One wake, observation and archival worker each, iterating over hosts. Observation is inherently per server (its own cadence, backoff and capture), and a single wake loop would need per-host freeze bookkeeping inside one Scheduler that today holds one dispatcher. Per-session workers reuse today's loops unchanged except for the host filter.
- A `HostRouter` trait in front of `HostPort`. It adds a second dispatch layer when the port is already per session.

### 3. Attaching: startup, lazy, sweep; no detach

**Recommended.**
- **Attach sequence** (`HostSessions::attach_locked`, under `attach_lock`, bounded by the caller's budget):
  1. Check that `(factory.exists)(locator)` holds. If it does not, return `HostUnavailable("Herdr server <locator> not reachable")`.
  2. Check that `fs::canonicalize(locator) == locator` (production only; the test factory skips it). A mismatch is `InvalidRequest("host locator is not canonical")`.
  3. Refuse past `MAX_ATTACHED_HOSTS` with `HostUnavailable("too many Herdr servers attached (16)")`.
  4. Take a foreground writer turn and call `StorePort::attach_host(&locator, now) -> (HostId, persisted_epoch)`. This is one `BEGIN IMMEDIATE`: `INSERT … ON CONFLICT(endpoint) DO UPDATE SET last_attached_at`, then read `host_epoch`. It replaces ht-5fr.1's attach upsert and `persisted_host_epoch(instance)`.
  5. `(port, observer) = (factory.build)(locator)`, then `port.resume_after_epoch(persisted_epoch)`.
  6. Build the reachability, the `OrdinaryIdentity` (now taking `host_id`), the bootstrap runtime and the evidence status. Register the session's wake pacer with `CommitKicks` (§7). Spawn the observation and wake workers with `stop`, pushing their `JoinHandle`s into the daemon's existing worker slot.
  7. Publish: insert into `sessions`, then into `HostIndex`.

  If step 6 fails (thread spawn), the code cancels `stop`, unregisters the pacer, joins whatever started, and returns the io error as `HostUnavailable`. The `herdr_hosts` row stays (harmless), and the sweep retries.
- **Concurrency.** `resolve` first checks `by_locator` under the read lock (the fast path). It takes `attach_lock` only on a miss and re-checks after acquiring it, so concurrent first claims attach exactly once. The lock is taken with `try_lock` in a 10 ms poll bounded by the request deadline. A deadline that expires returns `StoreBusy`, the existing retryable code.
- **Startup** (`attach_startup`, in `run_elected_impl` after `Journal::open` and before the listener serves): first the hint, if given, then every `herdr_hosts` row ordered by `last_attached_at DESC` whose endpoint exists, up to the cap. A failure is logged as one line (`host attach <locator>: <error>`) and is never fatal. With no attached host the daemon is store-only, exactly as in ht-5fr.1.
- **Lazy.** At the top of `DomainService::dispatch`, for any command with `Command::host_locator() == Some(l)`, the dispatcher calls `self.hosts.resolve(l, budget)?` before any store call. ht-5fr.1's `attached_host()` helper becomes `attached_host(&HostLocator, &CallBudget) -> Result<Arc<HostSession>, ApiError>`, implemented by `resolve`. Its callers (`ResolveSeat`, the operator actions, `ContinuityCheckIn` and the automatic bootstrap paths) pass the command's locator. The automatic bootstrap paths pass the namespace's `host_endpoint`.
- **Sweep.** A `herdr-host-sweep` thread (owned by HostSessions, stopped by `stop_all`) runs every 30 s. It takes up to 64 `herdr_hosts` rows per tick, round-robin by `host_id`, and attaches any unattached row whose endpoint exists. This brings seats on a restarted server back to wake service without waiting for a claim.
- **No detach.** A session lives until the daemon stops (root Key abstractions). A server that disappears only goes unreachable (§7). `stop_all()` cancels every session's `stop` and is called from the daemon's stop path next to `factory_stop`. Every handle is in the worker slot, so the existing join-on-shutdown covers them.
- **Deleted from ht-5fr.1:** the client-side interim single-host guard (its one function), `EndpointDescriptor.primary_host`, `HostChoice`, `StoreContext.primary_host` and its accessor. `lifecycle::spawn_detached` keeps passing `--host-endpoint` as the startup hint.

**Considered.**
- Attaching only at startup: a second Herdr server started later would stay unusable until a daemon restart.
- An explicit `AttachHost` request from `ensure_running`: redundant, because every pane-bound command already carries the locator.
- Detaching idle sessions: it saves one thread pair per dead server at the cost of a re-attach race. The cap bounds the cost instead.
- A fatal start on a bad hint: one dead server would then block the node and sync.

### 4. Store entry check: `cooperative_claim` and `HostIndex`

**Recommended.**
- `src/store/hosts.rs` (new) owns the `herdr_hosts` table: `attach_host`, `host_row(host_id)`, `remembered(limit)`, and the moved per-host reads. It also holds `pub struct HostIndex { by_locator: RwLock<BTreeMap<HostLocator, HostId>> }` with `attached_id(&HostLocator) -> Result<HostId, ApiError>` (`HostUnavailable("Herdr server not attached")` on a miss), `contains(HostId)` and `ids()`. Only HostSessions publishes into the index.
- `StoreContext::new` takes `Arc<HostIndex>`. `primary_host` is removed.
- `seats::cooperative_instance(db, instance, claim)` becomes `seats::cooperative_claim(db, ctx: &StoreContext, node: &str, claim) -> Result<HostId, ApiError>`:
  1. `claim.instance == node` (the store's `node_uuid`), and the role matches (today's rule). Otherwise `CallerUnverified("cooperative node or declared role mismatch")`.
  2. `host = ctx.hosts().attached_id(&claim.host)?`.
  3. `SELECT is_local, host_id FROM seats WHERE id=?`. A missing row, or `is_local = 0`, is `CallerUnverified("cooperative seat is not local")`. A row whose `host_id != host` is `CallerUnverified("cooperative seat sits on another Herdr server")`.
  4. Return `host`.

  All call sites switch to it and keep the returned `HostId` for the rest of their transaction. They are in `store/seats.rs`, `control.rs`, `messages.rs`, `handoff.rs`, `handoff/automatic.rs` and `topology_handoff/automatic.rs`; `grep -n cooperative_instance src/store` lists them. `cooperative_mapping` takes the `HostId` and joins `herdr_hosts h ON h.host_id=s.host_id` in place of the primary-host read. Its `recovery_holds`/`effective_observation` lookups use `(host_id, target)`.
- `execute_accountable_transaction` and `execute_budgeted_idempotent_transaction_with_constraints` need no new parameter. The check runs inside their decide closures through `cooperative_claim`, as `cooperative_instance` did (root Key abstractions, "Store transactions").
- **Error choice.** Not attached is `HostUnavailable`: it is retryable after the server comes up, and the same code ht-5fr.1 used for its guard. A seat on another server, or a non-local seat, is `CallerUnverified`, the code today's foreign-instance case uses.

**Considered.**
- Resolving `claim.host` only through `herdr_hosts` in SQL, with no in-memory index: the store would then accept claims for a known but unattached host, whose seats have no running observation.
- Passing `HostId` from dispatch into every store method: it widens ~40 `StorePort` signatures, while the index keeps the A2 decision inside the transaction.

### 5. Per-host state in the schema and the queries

**Recommended.** These edits go into `migrations/node/0001_node_baseline.sql`, which stays editable in place until the first release, per ht-5fr.1:
- **Re-key to `host_id INTEGER NOT NULL REFERENCES herdr_hosts(host_id)`**, replacing `instance_id`, with PKs, UNIQUEs and indexes rewritten from `(instance_id, …)` to `(host_id, …)`: `snapshot_generations` (`UNIQUE(host_id,host_boot,epoch,observation_sequence)`, index `(host_id,status,id)`), `observed_targets` (`PK(host_id,target_id)`), `recovery_baseline_releases` (`PK(host_id,baseline_generation_id,target_id)`), `recovery_baseline_targets` (`PK(host_id,target_id)`), `recovery_holds` (`PK(host_id,target_id)`) and `allocation_decisions` (index `(host_id,target_id,ordinal)`). The triggers `host_active_snapshot_published` and `host_baseline_snapshot_published` on `herdr_hosts` compare `g.host_id=NEW.host_id`. The re-keying task lists every other table with an `(instance_id, target_id)` key or index from `grep -n "target_id" migrations/node/0001_node_baseline.sql` and re-keys it the same way. The baseline header comment from ht-5fr.1 that records this boundary is removed.
- **seats.** The CHECK tightens to `CHECK((is_local=1) = (host_id IS NOT NULL))`, so a local seat always has a host and a remote seat never has one. When a seat goes unresolved, `host_id` is kept as the last server it sat on (target NULL). `seats_live_target(host_id,target_id)` stays as ht-5fr.1 defined it.
- **Archival tables**: see §8.
- **Query rule.** Each former `herdr_hosts WHERE host_id=?primary` read takes its host from one of three sources:
  - the claim's `HostId` (cooperative paths, §4);
  - the seat row: `JOIN herdr_hosts h ON h.host_id=s.host_id`, which replaces `JOIN host_instances h ON h.id=s.instance_id` (`eligible_delivery_recipient`, `effective.rs`, `wake.rs`, `attention.rs`, `queries.rs`, `messages.rs`, `bindings.rs`, `catch_up.rs`, `service_events.rs`, `service_send.rs`, `lazy_delivery.rs`, `invitation_content.rs`, `summary.rs`, `retention.rs`);
  - an explicit `HostId` argument passed by the worker that owns the host (observation, wake scope, archival sampling, `attach_host`).
- **Signatures that change** (compiler-driven):
  - `effective::effective_observation(db, host: HostId, target)`;
  - `StorePort::persisted_host_epoch` is removed (folded into `attach_host`);
  - every observation-store method (`identity::reconcile::observation_store::ObservationStore`, snapshot admission, stage, seal and publish, reconciliation pages, recovery baseline) takes `HostId`. `HostObservationAdmission` and `SnapshotHeader` gain `host_id: HostId`, and the `header.admission.instance != header.instance` guard (`seats.rs`) also compares `host_id`.
- **Seat inserts** (allocation, rebind or fresh seat in `seats.rs`) write `host_id` from the claim's or command's resolved host. They no longer use ht-5fr.1's primary accessor.

**Considered.**
- Keeping the child tables keyed by `instance_id` and adding a `host_id` column beside it: two keys that must agree, for no benefit, since `instance_id` is a per-node constant now.
- Allowing local seats with `host_id IS NULL`: per-host wake and observation filters would silently skip them.

### 6. Observation and identity per host

**Recommended.**
- Each session runs today's `spawn_observation_loop` (`src/service/workers.rs`) with its own `OrdinaryIdentity` (constructed with `host_id`), its own `Pacer` named `observation`, its own `WorkerStatus` and its own `HostReachability`. The thread is named `herdr-observations-<host_id>`.
- **Reconciliation scope invariant.** Every reconciliation, absence-closure, allocation, retirement-by-observation and recovery-baseline query adds `s.host_id = ?` (and `is_local = 1`, via ht-5fr.1's `LOCAL_SEAT`). A coherent snapshot of host H can resolve, unresolve or retire only seats whose `host_id = H`. A seat on host H′ whose target id also appears on H is never touched.
- `CommitKicks` keeps no table-driven kick for `Lane::Observation` (none exists today). The per-host observation pacer is reached through the session: `OrdinaryIdentity::with_observation_pacer` gets its own session's pacer.
- `HostEvidenceStatus` (last reconciliation, capture timing) is per session and feeds §10.
- A seat rebound to another host by C1 or an operator action (§11) is afterwards reconciled only by its new host's worker. The rebind's transaction updates `host_id` and `target_id` together.

### 7. Wake dispatcher per host

**Recommended.**
- Each session runs `start_wake_worker_with_liveness` with its own port, reachability, `WorkerStatus` and `Pacer`, and with a `ScheduledStore::for_host(host_id)`. The thread is named `herdr-wakes-<host_id>`. The liveness worker, mod-channel registry, poke capabilities and hook-pull outlook stay node-wide and are shared by `Arc`.
- **Wake scope.** `ScheduledStore::for_host` holds `host: HostId` and passes it to every seat-selecting wake call: `wake_candidates`, `wake_batch_seats`, the poke drive's due-seat scan and completion retries. The store adds `AND s.host_id=?` (`src/store/wake.rs`, the `StorePort` wake methods and `scheduler::WakePort`). Per-seat calls (reserve, record refusal, complete, clear batch) re-check `s.host_id = scope`. On a mismatch (the seat was rebound to another server mid-pass) they return `Conflict`, which the scheduler treats as "skip this seat", so the new host's worker owns it from then on.
- **Freeze is per host.** The loop's `reachability.is_down()` freeze, the recovery count and `clear_wake_refusals` are unchanged, but each is per session. A down host freezes only its own worker: it reserves nothing, records no refusal and makes no Herdr call (TRUST-POLICY C4 as today). Other sessions' workers run normally. Seats whose `host_id` names a host with no session (never attached, or its socket is gone) are selected by no worker, so the same holds for them: no reservation and no refusal. They are served once the sweep or a claim attaches the host.
- **Kick fan-out.** `CommitKicks` slots become `Vec<Arc<Pacer>>` per lane. `register(lane, pacer)` appends, a new `unregister(lane, &pacer)` is used only by a failed attach (§3), and `kick` kicks every registered pacer. `registered(lane)` means "at least one". `ModChannelRegistry`'s wake kick and every table-driven `Lane::Wakes` kick therefore reach every host's wake worker. Each one runs a host-filtered candidate scan, so the extra cost is N−1 cheap empty scans (N ≤ 16). `LaneProbe` (test support) gains `host_pacer(&HostLocator, Lane)`, and its existing single-lane accessors operate on the first registered pacer.
- The delivery contract is unchanged: one seat belongs to exactly one wake worker, so a nudge is still coalesced and dispatched once, and it is still sent only to an idle session.

**Considered.**
- One node-wide wake worker that skips seats of down hosts: one host's slow Herdr call (up to the 5 s budget) would then delay wakes on every other host.
- Per-host lanes in the `Lane` enum: the enum is closed and the hosts are dynamic.

### 8. Archival: node-wide lane, per-host evidence, session samplers

**Recommended.** Threads are node-scope, so the decision lane stays one worker (`Lane::Archival`, `src/service/archival.rs`). What is per host is the evidence and the sampling.
- **Schema** (baseline edits):
  - `archival_instances` keeps only the node-wide fields: `runtime_boot`, `policy_ms`, `last_mono`, `last_utc`, `evidence_epoch` (bumped by boot, policy or clock discontinuity), `mutation_revision`, `bootstrap_veto`, `bootstrap_source`, `seed_cursor` and `seat_seed_cursor`.
  - New `archival_hosts(host_id INTEGER PRIMARY KEY REFERENCES herdr_hosts(host_id), evidence_epoch INTEGER NOT NULL DEFAULT 0, host_generation INTEGER NOT NULL DEFAULT 0, coherent_since INTEGER, validating_snapshot TEXT, snapshot_checked INTEGER NOT NULL DEFAULT 0, snapshot_cursor TEXT NOT NULL DEFAULT '')` takes the moved per-host fields. It is added to `ARCHIVAL_TABLES` in `service/kicks.rs`, and the `archival_host_invalidation` trigger (on `herdr_hosts`) updates the row for `NEW.host_id`.
  - `seat_archival` gains `host_id INTEGER NOT NULL` and `host_evidence_epoch INTEGER NOT NULL`. Its due index becomes `(next_mono, seat_id)`, and its target index `(host_id, target_id)`.
- **Runtime.** `store::archival::Runtime` replaces `host_generation`, `coherent` and `valid_until_mono` with `hosts: Vec<HostArchivalEvidence { host_id, generation, coherent, valid_until_mono }>`, built from `HostSessions::attached()` and each session's `HostReachability::archival_state`. Hosts with no session are absent, which counts as incoherent.
- **Reconcile.** Node-wide discontinuity is handled as today on `archival_instances`. For each host in `rt.hosts`, a generation change, loss of coherence or a gap bumps that host's `archival_hosts.evidence_epoch`, sets or clears `coherent_since`, and bumps the node `mutation_revision`. The revision bump makes in-flight channel scans restart; it does not reset any thread's quiet period.
- **Decision.** The old node-wide `coherent_since` gate in `advance_channel` moves into `scan_members`. Each active member's qualification query joins `archival_hosts ah ON ah.host_id=s.host_id` and `herdr_hosts h ON h.host_id=s.host_id`, and requires `ah.coherent_since <= rt.mono - after_ms` and `a.host_evidence_epoch = ah.evidence_epoch`. A member on an incoherent or unattached host blocks only that thread (defer). The early return `if !rt.coherent` becomes "no host in `rt.hosts` is coherent". `validate_snapshot_page` runs per coherent host on its `archival_hosts` row.
- **Sampling.** `observation_ticket` returns the seat's `host_id`. The worker samples through `sessions.get(ticket.host_id)` with `observe_current_target_for_archival`. A missing session, or a failed sample, calls `mark_archival_uncertain()` on *that* session's reachability only.
- The archival lane's `ContinuationContext.host` becomes `None`: the lane is node-wide, and each legacy hint's own namespace names its server.
- Root §8's rule that automatic archival needs every member local is ht-5fr.4's. It composes with this design as one more `scan_members` condition.

**Considered.**
- One archival lane per host: a thread with members on two servers would have two deciders.
- A node-wide coherence flag (any host down means all incoherent): an outage on one server would restart every thread's quiet period, which breaks "an unreachable host affects only itself".

### 9. Handoff namespace and the bootstrap runtime per host

**Recommended.**
- `DomainService.bootstrap: Option<BootstrapRuntime>` and `with_bootstrap_runtime` are removed. Each session builds its runtime at attach: `HandoffNamespace { instance: node_uuid, state_dir, host_endpoint: locator }`, its `BootstrapObserver`, and the bootstrap store.
- Dispatch selects the runtime by the request's namespace: `hosts.resolve(&HostLocator::parse(namespace.host_endpoint)?, budget)?.bootstrap()`. A namespace whose `instance` is not this node is refused as today (`canonical.instance != self.instance`).
- `bootstrap_available()` and `automatic_available()` take the locator and are true when that host is attachable and its session has a runtime. In production every session has one.
- `DomainService.current_target: Option<Arc<OrdinaryIdentity>>` is replaced by `hosts: Arc<HostSessions>`. ResolveSeat, the operator actions and ContinuityCheckIn use `attached_host(&cmd.host, budget)?.identity()`.

### 10. Health and doctor per host

**Recommended.**
- `protocol::results::Health` gains `hosts: Vec<AttachedHostHealth>`, bounded at 16 and in `host_id` order. Each entry is `{ endpoint: String (≤128 bytes, keeping the tail, prefixed with "…" when truncated), reachability, version, coherent_enumeration, safe_prompt, current_execution, last_reconciliation_at }`. ht-5fr.1's `hosts` count is this list's length. Doctor's `store-only` label is keyed on an empty list.
- `Health.host` stays as the aggregate: the worst `reachability` and the weakest capability state over the entries, or `Unknown` with the `store-only` note when the list is empty. `Health.last_reconciliation_at` is the minimum over hosts, so the first-reconciliation logic still waits for every attached host.
- `HealthInputs` (`src/daemon/health.rs`) replaces the single host fields with `hosts: Vec<HostHealthInput>` from `HostSessions::health_entries()`. `assemble` adds one limitation per unreachable host, `Herdr server <endpoint> unreachable`. More than `LANE_LINES_BEFORE_FOLD` such lines fold into `<n> Herdr servers unreachable`. The `healthy` rule requires every entry to be Ready and Supported.
- Per-session lane statuses report as `observation[<host_id>]` and `wake[<host_id>]` in `degraded_lanes`. This is operator-facing text only, never agent-facing. The provider reads statuses from HostSessions on every Health call, so hosts attached after boot appear.
- `ElectedHostEvidence` and `HostReleaseSource(Option<Arc<dyn HostPort>>)` move into the session (one per host).
- No agent-facing text changes. Health and doctor are operator surfaces. The per-host lines are about 40 bytes each, bounded by the existing fold.

### 11. Seats moving between servers, and the TRUST-POLICY dependency

**Recommended.**
- A seat's `host_id` is where it currently sits. It changes only in the same transaction that sets a new `target_id`, by one of the existing explicit rebinds: C1 continuity reattach (`ContinuityCheckIn`, keyed on the resumed session id, with no host restriction, because the session id is the evidence), `OperatorRebind`, `OperatorReplace` or `OperatorFreshSeat`. Observation never changes `host_id`. A seat never changes `home_node` (ht-5fr.1's trigger). Remote seats have no host.
- `allocation_decisions` rows record the host (`host_id`) of each decision, so `doctor` and audits show a cross-server rebind.
- **TRUST-POLICY (consumed from ht-5fr.9).** The amendment must state:
  - the caller claim's host locator is cooperative context like `target`, not attestation;
  - the daemon accepts a claim only when the claimed seat is a local seat currently on the attached server the claim names (A2, decided in the store);
  - a rebind under C1 or by an operator may move a local seat between Herdr servers of the same node, and never between nodes;
  - an unreachable or unattached server freezes wakes only for its own seats.

  The first task of this subepic that changes `cooperative_claim` checks that this text exists in `TRUST-POLICY.md`, and adds it in the same commit if ht-5fr.9 has not (AGENTS.md trust rule).

### 12. Interfaces other subepics consume

- **ht-5fr.5 (idle-exit):** `HostSessions::attached_count()` is the "no attached host" input.
- **ht-5fr.7 (discovery):** `HostSessions::any_reachable()` provides the attached-host check and a port to run `herdr machine list` through.
- **ht-5fr.10 (multi-node harness):** `HostAttachPlan { hint, factory }` with a fake-locator factory is the FakeHost attach path. One node can carry several fake hosts.
- **ht-5fr.4 (fact kinds):** the `message.sent` and `invitation.created` handlers route local recipients' wakes through table writes only. The per-host wake workers pick them up by `seats.host_id`, so handlers never touch sessions.

## Testing

- **Unit, `src/protocol/ids.rs`:** `HostLocator::parse` accepts a canonical absolute path. It rejects relative paths, `..`, `.`, empty components, a trailing slash, NUL and lengths of 0 or 4097. Serde round-trips it as a string.
- **Unit, `src/protocol/authority.rs`:** `consume_cooperative` refuses a fence whose `host` differs. A claim without `host` fails to deserialize (`deny_unknown_fields` plus the required field).
- **Unit, `src/service/kicks.rs`:** `kick` reaches every pacer registered for a lane, and `unregister` removes exactly one.
- **Unit, `src/service/host_sessions.rs`** (fake factory):
  - `resolve` attaches once under 8 concurrent first calls;
  - a missing socket gives `HostUnavailable`;
  - a non-canonical locator gives `InvalidRequest`;
  - the 17th host is refused;
  - a failed worker spawn (injected) unregisters the pacer and publishes nothing to `HostIndex`;
  - `stop_all` joins every session thread.
- **Store, `tests/store/multi_host.rs`** (new, module of `tests/combined.rs`; one store with two `herdr_hosts` rows):
  - two resolved seats on `(h1,"t1")` and `(h2,"t1")` coexist;
  - a coherent snapshot of h1 lacking `t1` unresolves only the h1 seat and leaves the h2 seat and its recovery and baseline rows untouched;
  - `cooperative_claim` accepts the h2 seat with `claim.host = h2`, refuses it with `h1` ("sits on another Herdr server"), refuses an unattached locator (`HostUnavailable`) and refuses a remote seat ("not local");
  - `wake_candidates` scoped to h1 never returns the h2 seat;
  - a per-seat wake call with a mismatched scope gives `Conflict`;
  - a local seat insert with `host_id NULL` aborts on the CHECK;
  - operator rebind from h1 to h2 sets `host_id` and `target_id` together and records the host in `allocation_decisions`.
- **Store, `tests/store/channel_archival.rs`** (extended):
  - a thread whose members are all on h2 archives while h1 is incoherent;
  - a thread with one member on h1 defers;
  - an h1 coherence loss bumps only h1's `evidence_epoch` and the node `mutation_revision`, and leaves h2 members' samples valid.
- **Daemon integration, `tests/daemon/multi_host.rs`** (new, module of `tests/integration.rs`, test-support; daemon spawned through `test_support::spawn` with a fake-locator factory, stopped at test end):
  - with no hint, a claim from fake host A attaches A lazily, and Health then lists one host;
  - a claim from B attaches B, and seats on both receive a nudge;
  - marking A unavailable freezes A's wake worker (no `wake_work` reservation or refusal rows for A's seat) while B's idle seat is still woken; A's recovery kicks A's worker and its seat is woken;
  - Health is `degraded` with `Herdr server <A> unreachable` while A is down;
  - a restart with both rows remembered re-attaches both at startup;
  - a row whose socket appears after start is attached by the sweep (tick shortened in test-support);
  - after stop, `scripts/check-no-leaked-processes` finds nothing.
- **CLI, `tests/cli/hook.rs`:** an old installed argv with `--host-endpoint H` produces a claim whose `host` is canonical `H`. The ht-5fr.1 interim-guard test is deleted.
- **Regressions:** the existing single-host suites (`tests/store/*`, `tests/scheduler/*`, `tests/integration/*`) pass with one attached fake host. Their fixtures gain the `host` field, and `effective_observation` and the observation-store calls take `HostId`.
- **Checks:** `nice cargo clippy --locked --all-targets --all-features -- -D warnings`; `scripts/check-harness-neutral` (`src/service/host_sessions.rs` and `src/store/hosts.rs` are core and name no harness); `nice scripts/check-default-features`; focused `nice cargo test --locked --all-features multi_host`.

Suggested task cut:
1. `HostLocator`/`HostId`, claim and command fields, fences, handoff comparisons, protocol version: `src/protocol/{ids,authority,commands,wire,handoff}.rs`, `src/protocol/handoff/automatic.rs`, the `src/cli/*` claim builders.
2. Baseline re-key, `archival_hosts`, `seat_archival.host_id`, the seats CHECK: `migrations/node/0001_node_baseline.sql`.
3. `HostIndex`, `store/hosts.rs`, `cooperative_claim`, the query rewrite, `HostId` parameters: `src/store/{hosts,mod,connection,seats,control,messages,handoff,effective,wake,queries,…}.rs`, `src/ports.rs`, `src/identity/*`.
4. `CommitKicks` fan-out: `src/service/kicks.rs`.
5. HostSession/HostSessions, attach/startup/sweep, per-host observation and wake workers, `app.rs` composition, deleting `HostChoice`, the guard and the descriptor field: `src/service/host_sessions.rs`, `src/service/workers.rs`, `src/app.rs`, `src/main.rs`, `src/host/native.rs` (constructor only), `src/daemon/ownership.rs`, `src/cli/instance.rs`.
6. DomainService routing and the per-host bootstrap runtime: `src/service/dispatch.rs`.
7. Archival per-host evidence and sampler routing: `src/store/archival.rs`, `src/service/archival.rs`, `src/service/host_reachability.rs`.
8. Health and doctor per host: `src/daemon/health.rs`, `src/protocol/results.rs`, `src/cli/doctor.rs`.
9. Integration tests: `tests/daemon/multi_host.rs`, `tests/store/multi_host.rs`.

Order: 1 → 2 → 3 → 4 → 5 → (6, 7, 8) → 9.

## Key abstractions

**HostLocator / HostId**: new value types. `HostLocator` (`protocol::ids`) is the canonical socket path the claim and the target-bearing commands carry. `HostId` (`ports`) is the node-local `herdr_hosts.host_id`.
- Owns: the locator's parse rules (canonical absolute path, 1..=4096 bytes) and its string serde. `HostId`'s ordering.
- Hides: that a Herdr server is identified by its socket path.
- Must not: put `HostId` on the wire, in agent-visible text, or in a replicated payload.
- Relationships: leaf types. `CallerClaim`, the fences, `HostIndex` and HostSessions depend on them.
- Errors: `parse` returns `InvalidRequest` with one fixed message.

**HostSession / HostSessions**: extended (single-host `Option` in ht-5fr.1 → registry). Module `src/service/host_sessions.rs`.
- Owns: per host, the port, bootstrap observer, reachability, `OrdinaryIdentity`, bootstrap runtime (handoff namespace), evidence, observation worker and wake worker. Registry-wide, it owns the attach sequence (startup, lazy `resolve`, the 30 s sweep), the 16-host cap, publishing into `HostIndex`, `stop_all`, health entries, and the archival sampler lookup by `host_id`.
- Hides: how many Herdr servers exist, how they are attached, and that each has its own worker threads.
- Must not: detach a session; let one host's reachability affect another's workers; be addressed for remote seats; publish a `HostId` before its workers run.
- Relationships: composes `HostFactory` (two closures: `build`, `exists`), the node-wide store, writer, kicks and harness sources. DomainService, the archival lane, Health, idle-exit (ht-5fr.5) and discovery (ht-5fr.7) read it.
- Lifecycle: created in `run_elected_impl` after `Journal::open` and alive for the daemon's life. Sessions live until `stop_all`, and their thread handles join through the daemon's worker slot.
- Errors: `resolve` returns `HostUnavailable` (no socket, cap reached, spawn failed), `InvalidRequest` (non-canonical locator) or `StoreBusy` (attach lock wait past the deadline). Callers keep them apart.

**HostIndex**: new struct (`src/store/hosts.rs`). It is the store's read view of which hosts are attached.
- Owns: the `HostLocator → HostId` map of attached hosts and `attached_id`, which returns `HostUnavailable` on a miss.
- Hides: the in-memory synchronization.
- Must not: be written by anything except HostSessions, or contain a host whose workers are not running.
- Relationships: it replaces ht-5fr.1's `StoreContext.primary_host` accessor and is held by `StoreContext`. `cooperative_claim` is its main reader.

**`seats::cooperative_claim`**: extended from `cooperative_instance`. It is the store entry check for every cooperative write.
- Owns: the node, role, attached-host, local-seat and seat-on-this-host checks, and returning the claim's `HostId` for the rest of the transaction.
- Must not: attach hosts (dispatch does) or consult reachability.
- Errors: `CallerUnverified` (node or role mismatch, seat not local, seat on another server) is kept apart from `HostUnavailable` (not attached).

**CallerClaim**: extended with `host: HostLocator`. `CooperativeDecisionFence` gains `host`. Every handoff namespace comparison also compares the host. `Command::host_locator()` is the one accessor dispatch uses.

**CommitKicks (`service/kicks.rs`)**: extended. A lane slot holds every registered pacer, and a kick reaches all of them. `unregister` exists only for a failed attach.

**Archival (`store/archival.rs`, `service/archival.rs`)**: extended. The decision lane is node-wide. Evidence is per host (`archival_hosts`, `seat_archival.host_id`/`host_evidence_epoch`, `Runtime.hosts`). Member qualification checks its own host's coherence. Samples route through the seat's session.

**Store schema (`0001_node_baseline.sql`)**: extended. The Herdr-scope child tables are keyed by `host_id`. A local seat always has a `host_id`. `archival_hosts` is new.

**Health**: extended with `hosts: Vec<AttachedHostHealth>`. `host` is the aggregate. Unreachable hosts get one limitation each, folded past two.

**HostChoice, interim client guard, `EndpointDescriptor.primary_host`, `StoreContext.primary_host`**: removed. They are replaced by `HostAttachPlan`, daemon-side lazy attach and `HostIndex`.

Variation: the number and identity of Herdr servers vary at runtime, so HostSessions is the one routing point and `HostId` is the one key. Which lanes run per host is fixed in code: observation and wakes are per session, everything else is node-wide, and archival routes samples. How a server is built varies only between production and tests, so it is `HostFactory`, two closures and no trait. Error kinds are fixed codes, and attach outcomes are a closed set.

Walk-through: an agent in a pane of Herdr server B sends while the daemon has only A attached.
1. The CLI builds a claim with `host = canonical(B)`.
2. Dispatch calls `hosts.resolve(B)`. B's socket exists, so the dispatcher takes `attach_lock`. `attach_host` upserts `herdr_hosts` (host_id 2) and returns B's persisted epoch. The `NativeCli` for B resumes after it. B's observation and wake workers start, and B is published into `HostIndex`.
3. The send's transaction runs `cooperative_claim`: the node matches, `HostIndex` gives 2, and the seat is local with `host_id = 2`. The send proceeds through the journal (ht-5fr.3/4).
4. The recipient seat sits on A. The commit kicks `Lane::Wakes`, which reaches both wake workers. B's worker finds no candidate in its scope. A's worker wakes the idle recipient through A's port.
5. Later A's server dies. A's observation marks A down, and A's wake worker freezes. B's seats keep being woken, Health shows `Herdr server <A> unreachable`, and a thread whose members are all on B can still archive.
6. A comes back on the same socket. The observation capture marks A up, the recovery kick resumes A's wake worker, and nothing restarts.

Likely future change: a third per-host worker, such as a per-server discovery poll. It adds one field and one spawn in `HostSession::attach`, plus one Health line. Nothing else routes by host.

Digest:
- HostLocator/HostId: own the wire locator form and the node-local host key. New host addressing goes here. Callers never put `HostId` on the wire or compare raw path strings.
- HostSession/HostSessions: own per-server ports and workers, attach (startup, lazy, sweep), the cap, `HostIndex` publication and shutdown. New per-server workers go into HostSession. Callers never hold a node-wide `HostPort` or attach hosts themselves.
- HostIndex: owns the attached-host view the store reads. Only HostSessions writes it. Store code never reads `herdr_hosts` to decide whether a host is attached.
- cooperative_claim: owns claim-to-seat-to-host acceptance. New claim checks go here. Store entry points never compare `claim.instance` or `claim.host` inline.
- Archival per-host evidence: owns per-host coherence and epochs. Host outages change only their own `archival_hosts` row. The lane never treats one host's outage as global.

## Post-Implementation Notes

> *As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
