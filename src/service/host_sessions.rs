//! The multi-host registry: one session per attached Herdr server.
//!
//! The elected daemon composes one [`HostSessions`] from a [`HostAttachPlan`]
//! (`app::run_elected_impl`): startup attach, the sweep, and `stop_all` in its
//! stop path.

use crate::daemon::health::{ComponentStatus, HostHealthInput, LaneDegradation};
use crate::daemon::logs::LaneErrorLog;
use crate::host::probe::PROBE_BUDGET;
use crate::identity::repair::OrdinaryIdentity;
use crate::ports::{
    BootstrapObserver, HookPullOutlook, HostId, HostKind, HostPort, ModChannels,
    NativeLaunchCapability, PokeCapabilitySource, StorePort, WakeCapability,
};
use crate::protocol::{
    handoff::HandoffNamespace,
    ids::HostLocator,
    results::{ApiError, CapabilityState, ErrorCode},
    time::{CallBudget, CancelSubscription, Cancellation, Clock, MonoInstant},
};
use crate::service::{
    fair_writer::FairWriter,
    host_evidence::HostEvidenceStatus,
    host_reachability::HostReachability,
    kicks::{CommitKicks, Lane},
    liveness::LivenessWorker,
    pacer::{Pacer, Wake},
    workers::{WorkerStatus, start_observation_worker_named, start_wake_worker_named},
};
use crate::store::hosts::{HostIndex, HostRow};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock, RwLock, TryLockError,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle, ThreadId},
    time::Duration,
};

/// The most Herdr servers one node attaches.
pub const MAX_ATTACHED_HOSTS: usize = 16;
/// Rows the sweep considers per tick.
const SWEEP_ROWS_PER_TICK: usize = 64;
/// The sweep's production period.
pub const DEFAULT_SWEEP_INTERVAL: Duration = Duration::from_secs(30);
/// How often a blocked `resolve` retries the attach lock.
const ATTACH_POLL: Duration = Duration::from_millis(10);
/// The budget of a startup or sweep attach.
const BACKGROUND_ATTACH_MS: u64 = 5_000;

type BuildHost = Arc<
    dyn Fn(HostKind, &HostLocator) -> (Arc<dyn HostPort>, Arc<dyn BootstrapObserver>) + Send + Sync,
>;
type ProbeHost = Arc<dyn Fn(&HostLocator, Duration) -> Result<HostKind, ApiError> + Send + Sync>;
type SocketExists = Arc<dyn Fn(&Path) -> bool + Send + Sync>;
/// The `herdr_hosts` rows this node attached before, most recent first.
pub type RememberedHosts = Arc<dyn Fn(&CallBudget) -> Result<Vec<HostRow>, ApiError> + Send + Sync>;
/// The hosts that hold a resolved local seat, read through the store.
pub type SeatedHosts = Arc<dyn Fn(&CallBudget) -> Result<BTreeSet<HostId>, ApiError> + Send + Sync>;
type LogLine = Arc<dyn Fn(&str) + Send + Sync>;
/// Runs once with the first attached session.
pub type FirstAttachHook = Box<dyn FnOnce(&Arc<HostSession>) + Send>;

/// HostFactory: how a host's port and observer are built, which kind answers
/// on a socket, and whether the socket exists.
///
/// Owns: three closures, `exists(path)`, `probe(locator, budget) -> kind` and
/// `build(kind, locator) -> (port, observer)`. `HostSessions` calls them in
/// that order, `probe` under `attach_lock` and before the `herdr_hosts` row is
/// written (H1b), so a crash never leaves a wrongly kinded row.
/// Hides: `NativeCli` / `HcHost` construction, the `hello`-then-`ping` probe and
/// the socket test (production) versus the fake-locator map (tests).
/// Must not: attach, write the store or spawn workers (`HostSessions` does all
/// three).
/// Role: built in `main.rs` or a test, carried by [`HostAttachPlan`] and
/// consumed by `HostSessions`. Closures, not a trait: one production
/// implementation, and the fake is data. A new way to build a host is a new
/// closure; nothing else constructs a `HostPort`.
#[derive(Clone)]
pub struct HostFactory {
    pub build: BuildHost,
    pub probe: ProbeHost,
    pub exists: SocketExists,
}

/// HostAttachPlan: the startup instruction for `HostSessions`.
///
/// Owns: the optional `--host-endpoint` hint and the [`HostFactory`].
/// Hides: whether the hint came from a flag or the environment.
/// Must not: attach, write the store or spawn workers.
/// Role: built by the caller of the elected daemon, passed into
/// `run_elected_impl` and consumed by `HostSessions`. It replaced the
/// single-host start choice.
#[derive(Clone)]
pub struct HostAttachPlan {
    pub hint: Option<HostLocator>,
    pub factory: HostFactory,
}

/// The node-wide pieces every session shares by `Arc`, and the knobs of the
/// registry.
///
/// Owns: nothing a session owns. The liveness worker, mod-channel registry,
/// poke capabilities and hook-pull outlook are node-wide and shared here;
/// per-host state never goes in.
pub struct SessionDeps {
    pub store: Arc<dyn StorePort>,
    pub writer: Arc<FairWriter>,
    pub kicks: Arc<CommitKicks>,
    pub clock: Arc<dyn Clock>,
    /// The node uuid as text: the handoff instance and the store instance.
    pub node_uuid: String,
    pub boot: uuid::Uuid,
    /// The handoff state directory; `None` runs sessions without a bootstrap runtime.
    pub state_dir: Option<PathBuf>,
    pub retry: crate::notification::policy::RetryConfig,
    pub liveness: Option<Arc<LivenessWorker>>,
    pub mod_channels: Arc<dyn ModChannels>,
    pub poke_capabilities: Arc<dyn PokeCapabilitySource>,
    pub hook_pull: Arc<dyn HookPullOutlook>,
    /// Where worker handles go; the daemon joins this slot at shutdown.
    pub worker_slot: Arc<Mutex<Vec<JoinHandle<()>>>>,
    /// The daemon's stop token; every session and the sweep stop with it.
    pub stop: Cancellation,
    pub log: LogLine,
    /// Where the lanes of every session report failures.
    pub lane_log: Option<Arc<dyn LaneErrorLog>>,
    /// The remembered `herdr_hosts` rows, read through the store.
    pub remembered: RememberedHosts,
    /// The hosts holding a resolved local seat: what keeps a dialled host from idling out.
    pub seated: SeatedHosts,
    pub sweep_interval: Duration,
    /// Each session's wake-lane safety tick and observation capture cadence
    /// (production: `WAKE_SAFETY_TICK_MILLIS` and `OBSERVATION_CADENCE`; see
    /// `ServiceConfig::lane_tick`).
    pub wake_safety_tick: Duration,
    pub observation_cadence: Duration,
    /// Production attaches only a locator that is its own canonical path on
    /// disk; a fake factory's locators are not on disk.
    pub check_canonical: bool,
}

impl SessionDeps {
    /// Node-wide pieces for a registry nothing runs in: no handoff state, no
    /// liveness worker, no mod channels, no remembered rows, nothing logged.
    /// Fixtures that need a registry without a daemon build on it.
    pub fn detached(store: Arc<dyn StorePort>, clock: Arc<dyn Clock>, node_uuid: String) -> Self {
        Self {
            store,
            writer: Arc::new(FairWriter::new(32)),
            kicks: Arc::new(CommitKicks::default()),
            clock,
            node_uuid,
            boot: uuid::Uuid::new_v4(),
            state_dir: None,
            retry: Default::default(),
            liveness: None,
            mod_channels: Arc::new(crate::ports::NoModChannels),
            poke_capabilities: Arc::new(crate::ports::NoPokeCapabilities),
            hook_pull: Arc::new(crate::ports::NoHookPullOutlook),
            worker_slot: Arc::new(Mutex::new(Vec::new())),
            stop: Cancellation::default(),
            log: Arc::new(|_: &str| {}),
            lane_log: None,
            remembered: Arc::new(|_: &CallBudget| Ok(Vec::new())),
            seated: Arc::new(|_: &CallBudget| Ok(BTreeSet::new())),
            sweep_interval: DEFAULT_SWEEP_INTERVAL,
            wake_safety_tick: Duration::from_millis(
                crate::service::workers::WAKE_SAFETY_TICK_MILLIS,
            ),
            observation_cadence: crate::service::workers::OBSERVATION_CADENCE,
            check_canonical: false,
        }
    }
}

/// One session lane: its status and its Pacer, for Health and archival.
#[derive(Clone)]
pub struct LaneHandle {
    lane: Lane,
    status: Arc<WorkerStatus>,
    pacer: Arc<Pacer>,
}

impl LaneHandle {
    pub fn lane(&self) -> Lane {
        self.lane
    }
    pub fn status(&self) -> &Arc<WorkerStatus> {
        &self.status
    }
    pub fn pacer(&self) -> &Arc<Pacer> {
        &self.pacer
    }
}

/// One attached server's Health inputs and its failed lanes. Operator-only
/// (see [`HostSessions::health_entries`]).
pub struct HostHealthEntry {
    pub input: HostHealthInput,
    pub lanes: Vec<LaneDegradation>,
    /// Whether `input` joins `Health.hosts` and its aggregate. False for the
    /// built-in named host: only its failed lanes reach Health, because its
    /// capability answers (no cooperative Herdr prompt) are not a dialled
    /// server's and would skew the aggregate and its wake limitation.
    pub aggregated: bool,
}

/// `lane` as a Health degradation when its worker recorded a failure after its
/// last success, or its thread ended without a requested stop.
pub(crate) fn lane_degradation(lane: String, status: &WorkerStatus) -> Option<LaneDegradation> {
    if status.lane_dead() {
        return Some(LaneDegradation {
            lane,
            summary: "lane stopped unexpectedly".into(),
            class: None,
        });
    }
    status.health().map(|health| LaneDegradation {
        lane,
        summary: health.summary(),
        class: health
            .failure
            .as_ref()
            .and_then(crate::service::workers::RedactedFailure::code)
            .and_then(ErrorCode::default_class),
    })
}

/// The guarded-bootstrap runtime of one session.
#[derive(Clone)]
pub struct BootstrapRuntime {
    pub canonical: HandoffNamespace,
    pub observer: Arc<dyn BootstrapObserver>,
}

/// HostSession: one attached Herdr server and everything that is per server.
///
/// Owns: the server's `Arc<dyn HostPort>`, bootstrap observer, `HostReachability`,
/// `OrdinaryIdentity` (constructed with its `host_id`), bootstrap runtime with
/// its `HandoffNamespace`, `HostEvidenceStatus`, and its own observation and
/// wake workers (their `LaneHandle`s and stop token).
/// Hides: how many Herdr servers exist; callers hold a session, never a
/// node-wide `HostPort`.
/// Must not: affect sync or other hosts when its server is unreachable
/// (unreachable only marks it), or be addressed for remote seats.
/// Role: built only by `HostSessions`, which publishes it after its workers
/// run. It lives until the daemon stops. A new per-host worker goes here: a
/// field for its handle, started in `HostSessions::attach_locked`.
pub struct HostSession {
    host_id: HostId,
    locator: HostLocator,
    port: Arc<dyn HostPort>,
    observer: Arc<dyn BootstrapObserver>,
    reachability: Arc<HostReachability>,
    identity: Arc<OrdinaryIdentity>,
    bootstrap: Option<BootstrapRuntime>,
    /// The bootstrap runtime of a host that creates its own targets (V3 automatic bootstrap).
    /// Kept apart from `bootstrap` so the V1/V2 guarded path, Send preparation and the
    /// advertised guarded capabilities, which read `bootstrap`, answer as before.
    target_bootstrap: Option<BootstrapRuntime>,
    evidence: Arc<HostEvidenceStatus>,
    observation: LaneHandle,
    /// `None` when the port's `wake_capability()` is `WakeCapability::None`: no wake
    /// lane runs for such a host.
    wake: Option<LaneHandle>,
    stop: Cancellation,
    /// Keeps the link from the daemon's stop token to `stop` alive.
    _link: CancelSubscription,
}

impl HostSession {
    pub fn host_id(&self) -> HostId {
        self.host_id
    }
    pub fn locator(&self) -> &HostLocator {
        &self.locator
    }
    pub fn port(&self) -> &Arc<dyn HostPort> {
        &self.port
    }
    pub fn observer(&self) -> &Arc<dyn BootstrapObserver> {
        &self.observer
    }
    pub fn reachability(&self) -> &Arc<HostReachability> {
        &self.reachability
    }
    pub fn identity(&self) -> &Arc<OrdinaryIdentity> {
        &self.identity
    }
    /// `None` when the node has no handoff state directory or the server
    /// cannot start guarded.
    pub fn bootstrap(&self) -> Option<&BootstrapRuntime> {
        self.bootstrap.as_ref()
    }
    /// `None` unless the port creates its own targets and the node has a handoff state
    /// directory.
    pub fn target_bootstrap(&self) -> Option<&BootstrapRuntime> {
        self.target_bootstrap.as_ref()
    }
    pub fn evidence(&self) -> &Arc<HostEvidenceStatus> {
        &self.evidence
    }
    pub fn observation(&self) -> &LaneHandle {
        &self.observation
    }
    /// The wake lane; `None` for a host whose port answers `WakeCapability::None`.
    pub fn wake(&self) -> Option<&LaneHandle> {
        self.wake.as_ref()
    }
    /// Asks this session's workers to stop. Their handles join through the
    /// daemon's worker slot.
    pub fn stop(&self) {
        self.stop.cancel();
    }
    pub fn is_stopped(&self) -> bool {
        self.stop.is_cancelled()
    }

    fn health_entry(&self) -> HostHealthEntry {
        let evidence = self.evidence.health(self.port.incarnation_witness());
        let status = match evidence.host {
            ComponentStatus::Ready | ComponentStatus::Unknown if self.reachability.is_down() => {
                ComponentStatus::Unavailable("host marked down by the observation lane".into())
            }
            other => other,
        };
        let id = self.host_id.get();
        let lanes = std::iter::once(&self.observation)
            .chain(self.wake.as_ref())
            .filter_map(|handle| {
                lane_degradation(format!("{}[{id}]", handle.lane().name()), handle.status())
            })
            .collect();
        HostHealthEntry {
            input: HostHealthInput {
                host_id: id,
                endpoint: self.locator.as_str().to_owned(),
                status,
                version: self
                    .port
                    .observed_release()
                    .map(|release| release.summary()),
                current_execution: CapabilityState::Unsupported,
                coherent_enumeration: evidence.coherent_enumeration,
                safe_prompt: self.port.safe_prompt_capability(),
                last_reconciliation_at: evidence.last_reconciliation_at,
            },
            lanes,
            aggregated: true,
        }
    }

    /// A session over a caller-built identity, with no workers, for services
    /// composed around one fixture host ([`HostSessions::prebuilt`]). Its
    /// port is the identity's. A session without `bootstrap` refuses guarded
    /// bootstrap observations.
    pub fn prebuilt(
        locator: HostLocator,
        identity: Arc<OrdinaryIdentity>,
        clock: Arc<dyn Clock>,
        bootstrap: Option<BootstrapRuntime>,
    ) -> Arc<Self> {
        let stop = Cancellation::default();
        let link = stop.on_cancel(Arc::new(|| {}));
        // A host that creates its own targets keeps the runtime in the V3 slot only.
        let creates_targets = identity.host_port().creates_targets_itself();
        let observer = bootstrap.as_ref().map_or_else(
            || Arc::new(RefusedBootstrapObserver) as Arc<dyn BootstrapObserver>,
            |runtime| Arc::clone(&runtime.observer),
        );
        let lane = |lane: Lane| LaneHandle {
            lane,
            status: Arc::new(WorkerStatus::default()),
            pacer: Arc::new(Pacer::new(lane.name(), Arc::clone(&clock), stop.clone())),
        };
        let (bootstrap_slot, target_slot) = if creates_targets {
            (None, bootstrap)
        } else {
            (bootstrap, None)
        };
        Arc::new(Self {
            host_id: identity.host_id(),
            locator,
            port: Arc::clone(identity.host_port()),
            observer,
            reachability: Arc::new(HostReachability::default()),
            identity,
            bootstrap: bootstrap_slot,
            target_bootstrap: target_slot,
            evidence: Arc::new(HostEvidenceStatus::default()),
            observation: lane(Lane::Observation),
            wake: Some(lane(Lane::Wakes)),
            stop,
            _link: link,
        })
    }
}

/// The bootstrap observer of a prebuilt session that has no bootstrap
/// runtime: every observation is refused.
struct RefusedBootstrapObserver;

impl BootstrapObserver for RefusedBootstrapObserver {
    fn observe_bootstrap_target(
        &self,
        _: &crate::protocol::ids::HostTargetId,
        _: &crate::ports::HostCallContext,
    ) -> Result<crate::ports::BootstrapPaneObservation, ApiError> {
        Err(ApiError::unsupported("bootstrap observation unavailable"))
    }
}

/// HostSessions: the registry of attached Herdr servers.
///
/// Owns: one built-in session slot ([`HostSessions::attach_builtin`]: set once,
/// never dialled, outside the cap and [`HostSessions::attached`], visible to
/// [`HostSessions::all_for_reads`], `by_locator` and `resolve`) and up to
/// [`MAX_ATTACHED_HOSTS`] dialled sessions keyed by `HostId`, attached at
/// startup, lazily through [`HostSessions::resolve`] on the request path (one
/// attach under concurrent first claims, bounded by the request budget,
/// `StoreBusy` past it) and by a periodic sweep; never detached. Each session
/// owns its port, reachability, identity, bootstrap runtime, evidence and its
/// own observation and wake workers; publication into `HostIndex` happens only
/// after those workers run. A failed attach unregisters its Pacers and
/// publishes nothing. `stop_all` cancels every session. Also owns the one-shot
/// first-attach hook node-wide lanes start from ([`HostSessions::on_first_attach`]).
/// Hides: how many Herdr servers exist; it is the one place that routes by a
/// seat's `host_id`.
/// Must not: decide reachability for the store (that is `HostIndex` plus the
/// session), or let one host's failure touch another's.
/// Errors kept apart: `HostUnavailable` (no socket, cap, spawn failure),
/// `InvalidRequest` (non-canonical), `StoreBusy` (lock wait past deadline).
/// Role: lives for the daemon; `DomainService` reaches every host through it
/// (`DomainService::attached_host`). New per-host workers go into `HostSession`.
pub struct HostSessions {
    sessions: RwLock<BTreeMap<HostId, Arc<HostSession>>>,
    /// The built-in session: set once, never in `sessions`.
    builtin: OnceLock<Arc<HostSession>>,
    index: Arc<HostIndex>,
    factory: HostFactory,
    attach_lock: Mutex<()>,
    deps: SessionDeps,
    /// Threads this registry pushed into the worker slot.
    threads: Mutex<Vec<ThreadId>>,
    sweep_stop: Cancellation,
    sweep_cursor: Mutex<Option<HostId>>,
    /// The first server `attach_startup` attached (see [`HostSessions::startup_primary`]).
    startup_first: Mutex<Option<HostId>>,
    /// The one-shot hook node-wide lanes start from (see [`HostSessions::on_first_attach`]).
    first_attach: Mutex<Option<FirstAttachHook>>,
    /// Set by [`HostSessions::prebuilt`]: the one session every locator names.
    sole: Option<Arc<HostSession>>,
    stopped: AtomicBool,
    _sweep_link: Mutex<Option<CancelSubscription>>,
}

fn unavailable(detail: impl Into<String>) -> ApiError {
    ApiError::new(ErrorCode::HostUnavailable, detail.into())
}

/// Whether `path` is its own canonical form on disk.
fn is_canonical_on_disk(path: &Path) -> bool {
    std::fs::canonicalize(path).is_ok_and(|real| real == path)
}

impl HostSessions {
    pub fn new(index: Arc<HostIndex>, factory: HostFactory, deps: SessionDeps) -> Self {
        let sweep_stop = Cancellation::default();
        let link = {
            let sweep_stop = sweep_stop.clone();
            deps.stop.on_cancel(Arc::new(move || sweep_stop.cancel()))
        };
        Self {
            sessions: RwLock::new(BTreeMap::new()),
            builtin: OnceLock::new(),
            index,
            factory,
            attach_lock: Mutex::new(()),
            deps,
            threads: Mutex::new(Vec::new()),
            sweep_stop,
            sweep_cursor: Mutex::new(None),
            startup_first: Mutex::new(None),
            first_attach: Mutex::new(None),
            sole: None,
            stopped: AtomicBool::new(false),
            _sweep_link: Mutex::new(Some(link)),
        }
    }

    /// A registry over several prebuilt sessions (distinct host ids), for fixtures that read
    /// every host's labels. Nothing attaches; there is no sole session, so locators resolve
    /// only through the registry's own lookup.
    #[cfg(any(test, feature = "test-support"))]
    pub fn prebuilt_many(
        prebuilt: Vec<Arc<HostSession>>,
        store: Arc<dyn StorePort>,
        clock: Arc<dyn Clock>,
        node_uuid: String,
    ) -> Arc<Self> {
        let deps = SessionDeps::detached(store, clock, node_uuid);
        let factory = HostFactory {
            build: Arc::new(|_, _| unreachable!("a prebuilt registry attaches nothing")),
            probe: Arc::new(|_, _| unreachable!("a prebuilt registry attaches nothing")),
            exists: Arc::new(|_| false),
        };
        let sessions = Self::new(Arc::new(HostIndex::new()), factory, deps);
        {
            let mut map = sessions.sessions.write().unwrap_or_else(|e| e.into_inner());
            for session in prebuilt {
                map.insert(session.host_id, session);
            }
        }
        Arc::new(sessions)
    }

    /// A registry over one prebuilt session, for fixtures: every locator
    /// resolves to it and nothing attaches. The node-wide pieces are inert
    /// (no workers, no remembered rows, nothing reachable to attach).
    pub fn prebuilt(
        session: Arc<HostSession>,
        store: Arc<dyn StorePort>,
        clock: Arc<dyn Clock>,
        node_uuid: String,
    ) -> Arc<Self> {
        let deps = SessionDeps::detached(store, clock, node_uuid);
        let factory = HostFactory {
            build: Arc::new(|_, _| unreachable!("a prebuilt registry attaches nothing")),
            probe: Arc::new(|_, _| unreachable!("a prebuilt registry attaches nothing")),
            exists: Arc::new(|_| false),
        };
        let mut sessions = Self::new(Arc::new(HostIndex::new()), factory, deps);
        sessions
            .sessions
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .insert(session.host_id, Arc::clone(&session));
        sessions.sole = Some(session);
        Arc::new(sessions)
    }

    pub fn get(&self, id: HostId) -> Option<Arc<HostSession>> {
        self.sessions.read().ok()?.get(&id).cloned()
    }

    /// The session of `locator`: the built-in one first, then a dialled one.
    pub fn by_locator(&self, locator: &HostLocator) -> Option<Arc<HostSession>> {
        if let Some(builtin) = self.builtin.get().filter(|b| b.locator == *locator) {
            return Some(Arc::clone(builtin));
        }
        self.sessions
            .read()
            .ok()?
            .values()
            .find(|session| session.locator == *locator)
            .cloned()
    }

    /// Every dialled session, ascending by host id. Never the built-in one.
    pub fn attached(&self) -> Vec<Arc<HostSession>> {
        self.sessions
            .read()
            .map(|map| map.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Every session a read may consult: the dialled ones (ascending by host
    /// id), then the built-in one.
    pub fn all_for_reads(&self) -> Vec<Arc<HostSession>> {
        let mut all = self.attached();
        all.extend(self.builtin.get().cloned());
        all
    }

    /// Whether any dialled session's host holds a resolved local seat (07 §5.3 idle exit). A
    /// store that cannot answer counts as holding one, so a read failure never ends the daemon.
    pub fn any_seated(&self) -> bool {
        let attached = self.attached();
        if attached.is_empty() {
            return false;
        }
        match (self.deps.seated)(&self.background_budget()) {
            Ok(seated) => attached
                .iter()
                .any(|session| seated.contains(&session.host_id)),
            Err(_) => true,
        }
    }

    /// The first dialled session's Pacer for a host lane, among the sessions that run the lane
    /// (a host with no wake capability has no wake lane, so it is not "first" for `Wakes`).
    pub fn first_pacer(&self, lane: Lane) -> Option<Arc<Pacer>> {
        self.attached().into_iter().find_map(|session| match lane {
            Lane::Observation => Some(Arc::clone(session.observation().pacer())),
            Lane::Wakes => session.wake().map(|wake| Arc::clone(wake.pacer())),
            _ => None,
        })
    }

    pub fn attached_count(&self) -> usize {
        self.sessions.read().map_or(0, |map| map.len())
    }

    /// The built-in host's wake-armed named bindings (tmux hint verified, process alive), pruning
    /// the dead ones: the idle-exit term for a tmux seat the daemon could wake.
    pub fn wake_armed_bindings(&self) -> usize {
        self.builtin
            .get()
            .map_or(0, |session| session.port().wake_armed_bindings())
    }

    /// Whether any attached Herdr server is not marked down. The peer-discovery
    /// gate asks about Herdr (`machines` come from its CLI), so this is the one
    /// new `kind()` reader: dialled sessions of another kind never open it.
    pub fn any_reachable(&self) -> bool {
        self.attached().iter().any(|session| {
            session.port().kind() == crate::ports::HostKind::Herdr
                && !session.reachability.is_down()
        })
    }

    /// One Health entry per attached server, ascending by host id, with the
    /// server's own failed lanes (`observation[<host_id>]`, `wake[<host_id>]`),
    /// then the built-in named host's entry only while one of its lanes is
    /// degraded, and not aggregated (`HostHealthEntry::aggregated`).
    /// `DaemonHealthInputs.hosts` empty means a store-only daemon, and `assemble`
    /// aggregates over `hosts`, so an always-present built-in input would turn
    /// every store-only daemon's capability fields into the named host's; its
    /// lane failure still reaches Health and doctor.
    ///
    /// Operator-only surface: it feeds `Health.hosts` and the degraded-lane
    /// lines of `doctor`, which carry the node-local host id and the socket
    /// path; nothing agent-facing reads it. Read on every Health call, so an
    /// outage or recovery shows without a restart. A server the observation
    /// lane marked down reports unreachable even before its evidence says so.
    pub fn health_entries(&self) -> Vec<HostHealthEntry> {
        let builtin = self
            .builtin
            .get()
            .map(|session| HostHealthEntry {
                aggregated: false,
                ..session.health_entry()
            })
            .filter(|entry| !entry.lanes.is_empty());
        self.attached()
            .iter()
            .map(|session| session.health_entry())
            .chain(builtin)
            .collect()
    }

    /// The session of `locator`, attaching it first when it is new.
    ///
    /// A read-lock lookup answers an attached host. A miss takes the attach
    /// lock (polled every 10 ms until `budget`'s deadline; `StoreBusy` past
    /// it), looks again, then attaches.
    pub fn resolve(
        &self,
        locator: &HostLocator,
        budget: &CallBudget,
    ) -> Result<Arc<HostSession>, ApiError> {
        self.resolve_checked(locator, budget, false)
    }

    /// `resolve` for the operator's startup hint when `startup_hint`: it is explicit,
    /// so it skips the canonical-path check (its socket may not exist yet) and a
    /// silent probe does not refuse it.
    fn resolve_checked(
        &self,
        locator: &HostLocator,
        budget: &CallBudget,
        startup_hint: bool,
    ) -> Result<Arc<HostSession>, ApiError> {
        if let Some(sole) = &self.sole {
            return Ok(Arc::clone(sole));
        }
        if let Some(session) = self.by_locator(locator) {
            return Ok(session);
        }
        let _attaching = self.lock_attach(budget)?;
        if let Some(session) = self.by_locator(locator) {
            return Ok(session);
        }
        self.attach_locked(locator, budget, startup_hint)
    }

    fn lock_attach(&self, budget: &CallBudget) -> Result<std::sync::MutexGuard<'_, ()>, ApiError> {
        loop {
            match self.attach_lock.try_lock() {
                Ok(guard) => return Ok(guard),
                Err(TryLockError::Poisoned(poisoned)) => return Ok(poisoned.into_inner()),
                Err(TryLockError::WouldBlock) => {}
            }
            if budget.cancellation.wait_blocking(ATTACH_POLL) {
                return Err(ApiError::cancelled("host attach cancelled"));
            }
            if budget.deadline_passed(self.deps.clock.as_ref()) {
                return Err(ApiError::new(
                    ErrorCode::StoreBusy,
                    "host attach is busy".to_owned(),
                ));
            }
        }
    }

    /// The attach sequence, under `attach_lock`, for a locator not yet attached.
    fn attach_locked(
        &self,
        locator: &HostLocator,
        budget: &CallBudget,
        startup_hint: bool,
    ) -> Result<Arc<HostSession>, ApiError> {
        if self.stopped.load(Ordering::SeqCst) {
            return Err(unavailable("host sessions are stopped"));
        }
        if !(self.factory.exists)(locator.as_path()) {
            return Err(unavailable(format!(
                "Herdr server {} not reachable",
                locator.as_str()
            )));
        }
        if !startup_hint && self.deps.check_canonical && !is_canonical_on_disk(locator.as_path()) {
            return Err(ApiError::invalid_request("host locator is not canonical"));
        }
        if self.attached_count() >= MAX_ATTACHED_HOSTS {
            return Err(unavailable(format!(
                "too many Herdr servers attached ({MAX_ATTACHED_HOSTS})"
            )));
        }
        // Probe before the row is written (H1b), still under `attach_lock`.
        let probed = match (self.factory.probe)(locator, PROBE_BUDGET) {
            Ok(kind) => Some(kind),
            // The operator named this server and its socket may not be up yet: it
            // attaches under the kind its row holds (the default for a new row,
            // `kind_probed_at` NULL), the observation lane reports it down and
            // reconnects, and a later attach whose probe answers stamps the kind.
            Err(_) if startup_hint => None,
            Err(error) => return Err(error),
        };
        let (host_id, epoch, kind) = {
            let _turn = self
                .deps
                .writer
                .enter_foreground(budget, self.deps.clock.as_ref())?;
            let now = self.deps.clock.utc_now().0;
            match probed {
                Some(kind) => {
                    let (host_id, epoch) = self
                        .deps
                        .store
                        .attach_host_of_kind(locator, now, kind, budget)?;
                    (host_id, epoch, kind)
                }
                None => self.deps.store.attach_host_unprobed(locator, now, budget)?,
            }
        };
        let (port, observer) = (self.factory.build)(kind, locator);
        port.resume_after_epoch(u64::try_from(epoch).unwrap_or(0));

        let session = self.start_session(host_id, locator, port, observer)?;
        // Index first: a caller that finds the session (the read-lock fast path
        // of `resolve`) must also find its id in the store's index.
        self.index.publish(locator.clone(), host_id);
        if let Ok(mut map) = self.sessions.write() {
            map.insert(host_id, Arc::clone(&session));
        }
        // Taken under its own lock, so a hook installed concurrently by
        // `on_first_attach` runs exactly once, here or there.
        let hook = self
            .first_attach
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(hook) = hook {
            hook(&session);
        }
        Ok(session)
    }

    /// Upserts the host row of `locator` and returns its id, so a built-in
    /// port that needs its own host id (the named host's store read is keyed on
    /// it) can be built before [`HostSessions::attach_builtin`]. Idempotent: the
    /// attach that follows returns the same id.
    pub fn reserve_host_id(
        &self,
        locator: &HostLocator,
        kind: crate::ports::HostKind,
    ) -> Result<HostId, ApiError> {
        let budget = self.background_budget();
        let _turn = self
            .deps
            .writer
            .enter_foreground(&budget, self.deps.clock.as_ref())?;
        self.deps
            .store
            .attach_host_of_kind(locator, self.deps.clock.utc_now().0, kind, &budget)
            .map(|(host_id, _)| host_id)
    }

    /// Attaches the built-in host under `locator`: the one session that is never
    /// dialled. It skips `factory.exists`, the canonical-path check and
    /// [`MAX_ATTACHED_HOSTS`], runs the first-attach hook never, and lives in its
    /// own slot, so [`HostSessions::attached`] never lists it. A second call is
    /// an error.
    pub fn attach_builtin(
        &self,
        kind: crate::ports::HostKind,
        port: Arc<dyn HostPort>,
        observer: Arc<dyn BootstrapObserver>,
        locator: &HostLocator,
    ) -> Result<Arc<HostSession>, ApiError> {
        let budget = self.background_budget();
        let _attaching = self.lock_attach(&budget)?;
        if self.stopped.load(Ordering::SeqCst) {
            return Err(unavailable("host sessions are stopped"));
        }
        if self.builtin.get().is_some() {
            return Err(ApiError::invalid_request(
                "the built-in host is already attached",
            ));
        }
        let (host_id, epoch) = {
            let _turn = self
                .deps
                .writer
                .enter_foreground(&budget, self.deps.clock.as_ref())?;
            self.deps.store.attach_host_of_kind(
                locator,
                self.deps.clock.utc_now().0,
                kind,
                &budget,
            )?
        };
        port.resume_after_epoch(u64::try_from(epoch).unwrap_or(0));
        let session = self.start_session(host_id, locator, port, observer)?;
        self.index.publish(locator.clone(), host_id);
        // Under `attach_lock`, so no second call raced the check above.
        let _ = self.builtin.set(Arc::clone(&session));
        Ok(session)
    }

    /// Runs `hook` once with the first session that attaches: at once (on the
    /// calling thread) when a session is attached already, else on the thread of
    /// the first successful attach (startup, a lazy `resolve` on the request
    /// path, or the sweep), after it is published. A second call replaces a hook
    /// that has not run yet.
    pub fn on_first_attach(&self, hook: FirstAttachHook) {
        let mut slot = self.first_attach.lock().unwrap_or_else(|e| e.into_inner());
        let attached = self.attached();
        let startup_first = *self.startup_first.lock().unwrap_or_else(|e| e.into_inner());
        let first = startup_first
            .and_then(|id| attached.iter().find(|session| session.host_id == id))
            .or_else(|| attached.first())
            .cloned();
        match first {
            Some(session) => {
                drop(slot);
                hook(&session);
            }
            None => *slot = Some(hook),
        }
    }

    /// Builds the session's values and starts its workers. On failure nothing
    /// is left registered or running.
    fn start_session(
        &self,
        host_id: HostId,
        locator: &HostLocator,
        port: Arc<dyn HostPort>,
        observer: Arc<dyn BootstrapObserver>,
    ) -> Result<Arc<HostSession>, ApiError> {
        let deps = &self.deps;
        let stop = Cancellation::default();
        let link = {
            let stop = stop.clone();
            deps.stop.on_cancel(Arc::new(move || stop.cancel()))
        };
        let reachability = Arc::new(HostReachability::default());
        let evidence = Arc::new(HostEvidenceStatus::default());
        let wake_pacer = (port.wake_capability() != WakeCapability::None).then(|| {
            Arc::new(Pacer::new(
                Lane::Wakes.name(),
                Arc::clone(&deps.clock),
                stop.clone(),
            ))
        });
        let observation_pacer = Arc::new(Pacer::new(
            Lane::Observation.name(),
            Arc::clone(&deps.clock),
            stop.clone(),
        ));
        let identity = Arc::new(
            OrdinaryIdentity::new(
                deps.node_uuid.clone(),
                host_id,
                Arc::clone(&deps.store),
                Arc::clone(&port),
                Arc::clone(&deps.clock),
                Arc::clone(&deps.writer),
            )
            .with_service_cancellation(stop.clone())
            .with_observation_pacer(Arc::clone(&observation_pacer)),
        );
        let bootstrap = deps.state_dir.as_ref().and_then(|state_dir| {
            (port.native_launch_capability() == NativeLaunchCapability::HostGuardedStart).then(
                || BootstrapRuntime {
                    canonical: HandoffNamespace {
                        instance: deps.node_uuid.clone(),
                        state_dir: state_dir.clone(),
                        host_endpoint: PathBuf::from(locator),
                    },
                    observer: Arc::clone(&observer),
                },
            )
        });
        let target_bootstrap = deps.state_dir.as_ref().and_then(|state_dir| {
            port.creates_targets_itself().then(|| BootstrapRuntime {
                canonical: HandoffNamespace {
                    instance: deps.node_uuid.clone(),
                    state_dir: state_dir.clone(),
                    host_endpoint: PathBuf::from(locator),
                },
                observer: Arc::clone(&observer),
            })
        });
        let observation_status = Arc::new(WorkerStatus::default());
        let wake_status = Arc::new(WorkerStatus::default());
        if let Some(log) = &deps.lane_log {
            observation_status.set_error_log(Arc::clone(log));
            wake_status.set_error_log(Arc::clone(log));
        }
        if let Some(pacer) = &wake_pacer {
            reachability.attach_wake_pacer(Arc::clone(pacer));
            deps.kicks.register(Lane::Wakes, Arc::clone(pacer));
        }
        deps.kicks
            .register(Lane::Observation, Arc::clone(&observation_pacer));

        let id = host_id.get();
        let mut started: Vec<JoinHandle<()>> = Vec::new();
        let spawned = (|| -> std::io::Result<()> {
            started.push(start_observation_worker_named(
                format!("herdr-observations-{id}"),
                Arc::clone(&identity),
                Arc::clone(&deps.store),
                Arc::clone(&deps.writer),
                stop.clone(),
                Arc::clone(&observation_status),
                Arc::clone(&evidence),
                Arc::clone(&observation_pacer),
                Arc::clone(&reachability),
                deps.observation_cadence,
            )?);
            spawn_fault(locator)?;
            if let Some(wake_pacer) = &wake_pacer {
                started.push(start_wake_worker_named(
                    format!("herdr-wakes-{id}"),
                    Arc::clone(&deps.store),
                    Arc::clone(&deps.writer),
                    Arc::clone(&port),
                    host_id,
                    deps.node_uuid.clone(),
                    deps.boot,
                    deps.retry,
                    Arc::clone(wake_pacer),
                    stop.clone(),
                    Arc::clone(&wake_status),
                    Arc::clone(&deps.poke_capabilities),
                    Arc::clone(&reachability),
                    Arc::clone(&deps.mod_channels),
                    Arc::clone(&deps.hook_pull),
                    deps.liveness.clone(),
                    deps.wake_safety_tick,
                )?);
            }
            Ok(())
        })();
        if let Err(error) = spawned {
            stop.cancel();
            if let Some(pacer) = &wake_pacer {
                deps.kicks.unregister(Lane::Wakes, pacer);
            }
            deps.kicks.unregister(Lane::Observation, &observation_pacer);
            for handle in started {
                let _ = handle.join();
            }
            return Err(unavailable(format!(
                "Herdr server {} workers did not start: {error}",
                locator.as_str()
            )));
        }
        self.hand_to_slot(started);
        Ok(Arc::new(HostSession {
            host_id,
            locator: locator.clone(),
            port,
            observer,
            reachability,
            identity,
            bootstrap,
            target_bootstrap,
            evidence,
            observation: LaneHandle {
                lane: Lane::Observation,
                status: observation_status,
                pacer: observation_pacer,
            },
            wake: wake_pacer.map(|pacer| LaneHandle {
                lane: Lane::Wakes,
                status: wake_status,
                pacer,
            }),
            stop,
            _link: link,
        }))
    }

    fn hand_to_slot(&self, handles: Vec<JoinHandle<()>>) {
        let mut slot = self
            .deps
            .worker_slot
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut ids = self.threads.lock().unwrap_or_else(|e| e.into_inner());
        for handle in handles {
            ids.push(handle.thread().id());
            slot.push(handle);
        }
    }

    fn background_budget(&self) -> CallBudget {
        CallBudget {
            deadline: MonoInstant(
                self.deps
                    .clock
                    .monotonic_now()
                    .0
                    .saturating_add(BACKGROUND_ATTACH_MS),
            ),
            cancellation: self.deps.stop.clone(),
        }
    }

    fn log_attach_failure(&self, locator: &HostLocator, error: &ApiError) {
        (self.deps.log)(&format!(
            "host attach {}: {}",
            locator.as_str(),
            error.detail
        ));
    }

    /// Attaches the startup hint, then the remembered rows whose endpoint
    /// exists, up to the cap, and starts the sweep. A failure is one log line
    /// `host attach <locator>: <error>` and never fatal; only the sweep
    /// thread failing to start is an error.
    pub fn attach_startup(self: &Arc<Self>, hint: Option<&HostLocator>) -> std::io::Result<()> {
        if let Some(hint) = hint
            && let Err(error) = self.resolve_checked(hint, &self.background_budget(), true)
        {
            self.log_attach_failure(hint, &error);
        }
        let first_remembered = self.attach_remembered_live();
        // Attach order is the hint, then remembered most recent first (read
        // before the attaches restamp them): the first session in it leads.
        *self.startup_first.lock().unwrap_or_else(|e| e.into_inner()) = hint
            .and_then(|hint| self.by_locator(hint))
            .map(|session| session.host_id)
            .or(first_remembered);
        self.start_sweep()
    }

    /// The session the node-wide seams without a claim host follow (Health,
    /// archival, the advertised bootstrap capability): the hint's, else the first server startup
    /// attached (the most recently attached remembered one), else the lowest
    /// attached `host_id` whose port has `HostGuardedStart` (never the
    /// built-in host); `None` when there is none.
    pub fn startup_primary(&self, hint: Option<&HostLocator>) -> Option<Arc<HostSession>> {
        let first = *self.startup_first.lock().unwrap_or_else(|e| e.into_inner());
        hint.and_then(|hint| self.by_locator(hint))
            .or_else(|| first.and_then(|id| self.get(id)))
            // Dialled-only fallback, filtered to guarded-start hosts (07 §5.3).
            .or_else(|| {
                self.attached().into_iter().find(|session| {
                    session.port().native_launch_capability()
                        == NativeLaunchCapability::HostGuardedStart
                })
            })
    }

    fn remembered_rows(&self) -> Vec<HostRow> {
        match (self.deps.remembered)(&self.background_budget()) {
            Ok(rows) => rows,
            Err(error) => {
                (self.deps.log)(&format!("host attach: remembered hosts: {}", error.detail));
                Vec::new()
            }
        }
    }

    /// Attaches the remembered rows whose socket exists, most recent first;
    /// returns the first row now attached.
    fn attach_remembered_live(&self) -> Option<HostId> {
        let mut first = None;
        for row in self.remembered_rows() {
            if self.attached_count() >= MAX_ATTACHED_HOSTS {
                break;
            }
            if !row.attachable() {
                continue;
            }
            if let Some(session) = self.by_locator(&row.locator) {
                first.get_or_insert(session.host_id);
                continue;
            }
            if !(self.factory.exists)(row.locator.as_path()) {
                continue;
            }
            match self.resolve(&row.locator, &self.background_budget()) {
                Ok(session) => {
                    first.get_or_insert(session.host_id);
                }
                Err(error) => self.log_attach_failure(&row.locator, &error),
            }
        }
        first
    }

    /// Starts the `herdr-host-sweep` thread: every `sweep_interval` it attaches
    /// remembered rows whose socket now exists.
    pub fn start_sweep(self: &Arc<Self>) -> std::io::Result<()> {
        let registry = Arc::clone(self);
        let pacer = Pacer::new(
            "host-sweep",
            Arc::clone(&self.deps.clock),
            self.sweep_stop.clone(),
        );
        let interval = self.deps.sweep_interval;
        let handle = thread::Builder::new()
            .name("herdr-host-sweep".into())
            .spawn(move || {
                while pacer.wait_blocking(interval) != Wake::Cancelled {
                    registry.sweep_tick();
                }
            })?;
        self.hand_to_slot(vec![handle]);
        Ok(())
    }

    /// One sweep pass: at most 64 unattached rows, round-robin by `host_id`.
    fn sweep_tick(&self) {
        // Dialled-only: the sweep re-attaches remembered rows; the built-in host is never dialled.
        let attached: Vec<HostLocator> = self
            .attached()
            .iter()
            .map(|session| session.locator.clone())
            .collect();
        let mut rows: Vec<HostRow> = self
            .remembered_rows()
            .into_iter()
            .filter(|row| row.attachable() && !attached.contains(&row.locator))
            .collect();
        rows.sort_by_key(|row| row.id);
        let cursor = self
            .sweep_cursor
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .to_owned();
        let split = cursor.map_or(0, |last| rows.partition_point(|row| row.id <= last));
        rows.rotate_left(split);
        rows.truncate(SWEEP_ROWS_PER_TICK);
        if let Some(last) = rows.last() {
            *self.sweep_cursor.lock().unwrap_or_else(|e| e.into_inner()) = Some(last.id);
        }
        for row in rows {
            if self.sweep_stop.is_cancelled() || self.attached_count() >= MAX_ATTACHED_HOSTS {
                break;
            }
            if !(self.factory.exists)(row.locator.as_path()) {
                continue;
            }
            if let Err(error) = self.resolve(&row.locator, &self.background_budget()) {
                self.log_attach_failure(&row.locator, &error);
            }
        }
    }

    /// Cancels every session and the sweep, then joins their threads (taken
    /// from the worker slot). There is no detach: sessions end with the daemon.
    pub fn stop_all(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.sweep_stop.cancel();
        // Waits out an attach in flight, so no session starts after this point.
        drop(self.attach_lock.lock().unwrap_or_else(|e| e.into_inner()));
        for session in self.all_for_reads() {
            session.stop();
        }
        let ours: Vec<ThreadId> =
            std::mem::take(&mut *self.threads.lock().unwrap_or_else(|e| e.into_inner()));
        let handles: Vec<JoinHandle<()>> = {
            let mut slot = self
                .deps
                .worker_slot
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let (mine, others): (Vec<_>, Vec<_>) = std::mem::take(&mut *slot)
                .into_iter()
                .partition(|handle| ours.contains(&handle.thread().id()));
            *slot = others;
            mine
        };
        for handle in handles {
            let _ = handle.join();
        }
    }
}

/// Test-only injected worker-spawn failure, scoped by locator.
#[cfg(any(test, feature = "test-support"))]
fn spawn_fault(locator: &HostLocator) -> std::io::Result<()> {
    crate::test_support::failpoints::hit("host_sessions.spawn_wakes", locator.as_str())
        .map_err(|error| std::io::Error::other(error.detail))
}

#[cfg(not(any(test, feature = "test-support")))]
fn spawn_fault(_: &HostLocator) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::{
        AgentComposerState, ComposerObservation, HostCallContext, HostObservation, HostSnapshot,
        NativeLaunchOutcome, NativeLaunchRequest, PromptOutcome, SafeWakeTarget,
    };
    use crate::protocol::ids::{HostTargetId, SeatId};
    use crate::store::{SqliteStore, StoreSettings, connection::StoreContext};
    use crate::test_support::{failpoints::Failpoint, isolation::TestIsolation};
    use std::{collections::BTreeSet, sync::atomic::AtomicUsize, time::Instant};

    /// Every host call is refused: sessions start, observe nothing and wake nothing.
    struct FakeHost {
        no_wake: bool,
    }

    fn refused<T>() -> Result<T, ApiError> {
        Err(ApiError::unsupported("fake host"))
    }

    impl HostPort for FakeHost {
        fn wake_capability(&self) -> WakeCapability {
            if self.no_wake {
                WakeCapability::None
            } else {
                WakeCapability::Composer(crate::ports::ComposerTraits::HERDR_TUI)
            }
        }
        fn live_topology(
            &self,
            _: &crate::protocol::time::CallBudget,
        ) -> Result<crate::host::observation::HostTopology, crate::protocol::results::ApiError>
        {
            Err(crate::protocol::results::ApiError::unsupported(
                "fixture host has no topology",
            ))
        }
        fn native_launch_capability(&self) -> NativeLaunchCapability {
            NativeLaunchCapability::Unsupported
        }
        fn observe_current_target(
            &self,
            _: &HostTargetId,
            _: &HostCallContext,
        ) -> Result<HostObservation, ApiError> {
            refused()
        }
        fn observe_current_target_for_archival(
            &self,
            _: &HostTargetId,
            _: &HostCallContext,
        ) -> Result<ComposerObservation, ApiError> {
            refused()
        }
        fn enumerate_targets(&self, _: &HostCallContext) -> Result<HostSnapshot, ApiError> {
            refused()
        }
        fn safe_wake_target(&self, _: &SeatId, _: &HostObservation) -> Option<SafeWakeTarget> {
            None
        }
        fn submit_prompt(
            &self,
            _: &SafeWakeTarget,
            _: &str,
            _: &HostCallContext,
        ) -> Result<PromptOutcome, ApiError> {
            refused()
        }
        fn launch_native(
            &self,
            _: NativeLaunchRequest,
            _: &HostCallContext,
        ) -> Result<NativeLaunchOutcome, ApiError> {
            refused()
        }
        fn pane_agent_state(
            &self,
            _: &SafeWakeTarget,
            _: &HostCallContext,
        ) -> Result<AgentComposerState, ApiError> {
            refused()
        }
        fn send_submit_key(&self, _: &SafeWakeTarget, _: &HostCallContext) -> Result<(), ApiError> {
            refused()
        }
    }

    struct NoObserver;
    impl BootstrapObserver for NoObserver {
        fn observe_bootstrap_target(
            &self,
            _: &HostTargetId,
            _: &HostCallContext,
        ) -> Result<crate::ports::BootstrapPaneObservation, ApiError> {
            refused()
        }
    }

    /// The set of locators whose "socket" exists; the factory counts builds.
    #[derive(Default)]
    struct World {
        live: Mutex<BTreeSet<String>>,
        built: AtomicUsize,
        /// The kind each locator's probe answers (`None`: the peer is silent);
        /// Herdr when absent.
        kinds: Mutex<BTreeMap<String, Option<HostKind>>>,
        /// The `herdr_hosts` rows that existed at each probe, in probe order.
        rows_at_probe: Mutex<Vec<i64>>,
        /// The kind each build was asked for, in build order.
        built_kinds: Mutex<Vec<HostKind>>,
    }

    impl World {
        fn set_live(&self, locator: &str, live: bool) {
            let mut set = self.live.lock().unwrap();
            if live {
                set.insert(locator.to_owned());
            } else {
                set.remove(locator);
            }
        }
    }

    struct Rig {
        _isolation: TestIsolation,
        world: Arc<World>,
        sessions: Arc<HostSessions>,
        index: Arc<HostIndex>,
        kicks: Arc<CommitKicks>,
        slot: Arc<Mutex<Vec<JoinHandle<()>>>>,
        log: Arc<Mutex<Vec<String>>>,
        stop: Cancellation,
        store: Arc<SqliteStore>,
    }

    impl Rig {
        fn new(label: &str, check_canonical: bool, sweep: Duration) -> Self {
            let isolation = TestIsolation::new(label);
            let clock: Arc<dyn Clock> = Arc::new(crate::app::SystemClock::default());
            let instance = crate::node::NodeUuid::new_v4().to_string();
            let index = Arc::new(HostIndex::new());
            let store = Arc::new(
                SqliteStore::new(
                    StoreContext::with_host_index(
                        isolation.path("store.db"),
                        Arc::clone(&clock),
                        Arc::clone(&index),
                    )
                    .with_node_seed(crate::test_support::node_seed_for(&instance)),
                    instance.clone(),
                    StoreSettings::default(),
                )
                .unwrap(),
            );
            let kicks = Arc::new(CommitKicks::default());
            let world = Arc::new(World::default());
            let factory = HostFactory {
                build: {
                    let world = Arc::clone(&world);
                    Arc::new(move |kind, _| {
                        world.built.fetch_add(1, Ordering::SeqCst);
                        world.built_kinds.lock().unwrap().push(kind);
                        (Arc::new(FakeHost { no_wake: false }), Arc::new(NoObserver))
                    })
                },
                probe: {
                    let world = Arc::clone(&world);
                    let db_path = isolation.path("store.db");
                    Arc::new(move |locator, _| {
                        let rows = rusqlite::Connection::open(&db_path)
                            .and_then(|db| {
                                db.query_row("SELECT count(*) FROM herdr_hosts", [], |r| r.get(0))
                            })
                            .unwrap_or(0);
                        world.rows_at_probe.lock().unwrap().push(rows);
                        let kinds = world.kinds.lock().unwrap();
                        kinds
                            .get(locator.as_str())
                            .copied()
                            .unwrap_or(Some(HostKind::Herdr))
                            .ok_or_else(|| ApiError::host_unavailable("silent peer"))
                    })
                },
                exists: {
                    let world = Arc::clone(&world);
                    Arc::new(move |path| {
                        world.live.lock().unwrap().contains(path.to_str().unwrap())
                    })
                },
            };
            let slot = Arc::new(Mutex::new(Vec::new()));
            let log = Arc::new(Mutex::new(Vec::new()));
            let stop = Cancellation::default();
            let remembered_store = Arc::clone(&store);
            let deps = SessionDeps {
                store: Arc::clone(&store) as Arc<dyn StorePort>,
                writer: Arc::new(FairWriter::new(32)),
                kicks: Arc::clone(&kicks),
                clock,
                node_uuid: instance,
                boot: uuid::Uuid::new_v4(),
                state_dir: None,
                retry: Default::default(),
                liveness: None,
                mod_channels: Arc::new(crate::ports::NoModChannels),
                poke_capabilities: Arc::new(crate::ports::NoPokeCapabilities),
                hook_pull: Arc::new(crate::ports::NoHookPullOutlook),
                worker_slot: Arc::clone(&slot),
                stop: stop.clone(),
                log: {
                    let log = Arc::clone(&log);
                    Arc::new(move |line: &str| log.lock().unwrap().push(line.to_owned()))
                },
                lane_log: None,
                remembered: Arc::new(move |budget| {
                    let db = remembered_store.context().open_query(budget.clone())?;
                    crate::store::hosts::remembered(&db, usize::MAX)
                }),
                seated: Arc::new(|_| Ok(BTreeSet::new())),
                sweep_interval: sweep,
                wake_safety_tick: Duration::from_millis(
                    crate::service::workers::WAKE_SAFETY_TICK_MILLIS,
                ),
                observation_cadence: crate::service::workers::OBSERVATION_CADENCE,
                check_canonical,
            };
            let sessions = Arc::new(HostSessions::new(Arc::clone(&index), factory, deps));
            Self {
                _isolation: isolation,
                world,
                sessions,
                index,
                kicks,
                slot,
                log,
                stop,
                store,
            }
        }

        fn budget(&self, ms: u64) -> CallBudget {
            CallBudget {
                deadline: MonoInstant(self.store.clock().monotonic_now().0 + ms),
                cancellation: Cancellation::default(),
            }
        }

        fn lines(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }

        /// The stored `(kind, kind_probed_at)` of `endpoint`.
        fn row_kind(&self, endpoint: &str) -> (String, Option<i64>) {
            rusqlite::Connection::open(self._isolation.path("store.db"))
                .unwrap()
                .query_row(
                    "SELECT kind,kind_probed_at FROM herdr_hosts WHERE endpoint=?1",
                    [endpoint],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap()
        }

        fn remember(&self, locator: &str) {
            let budget = self.budget(5_000);
            self.store.attach_host(&loc(locator), 1, &budget).unwrap();
        }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            self.stop.cancel();
            self.sessions.stop_all();
        }
    }

    fn loc(text: &str) -> HostLocator {
        HostLocator::parse(text).unwrap()
    }

    fn wait(what: &str, done: &dyn Fn() -> bool) {
        let until = Instant::now() + Duration::from_secs(20);
        while !done() {
            assert!(Instant::now() < until, "timed out waiting for {what}");
            thread::yield_now();
        }
    }

    fn recording_hook(seen: &Arc<Mutex<Vec<String>>>) -> FirstAttachHook {
        let seen = Arc::clone(seen);
        Box::new(move |session| {
            seen.lock()
                .unwrap()
                .push(session.locator().as_str().to_owned())
        })
    }

    // Kills: a hook that runs on every attach, or never (installed before any attach).
    #[test]
    fn first_attach_hook_runs_once_on_the_first_attach() {
        let rig = Rig::new("hs-first-hook", false, Duration::from_secs(60));
        let (a, b) = (loc("/herdr/a.sock"), loc("/herdr/b.sock"));
        rig.world.set_live(a.as_str(), true);
        rig.world.set_live(b.as_str(), true);
        let seen = Arc::new(Mutex::new(Vec::new()));
        rig.sessions.on_first_attach(recording_hook(&seen));
        assert!(seen.lock().unwrap().is_empty(), "nothing attached yet");
        rig.sessions.resolve(&a, &rig.budget(10_000)).unwrap();
        assert_eq!(*seen.lock().unwrap(), vec![a.as_str().to_owned()]);
        rig.sessions.resolve(&b, &rig.budget(10_000)).unwrap();
        assert_eq!(
            seen.lock().unwrap().len(),
            1,
            "a second host does not rerun it"
        );
    }

    // Kills: a hook stored forever when a session is attached already.
    #[test]
    fn first_attach_hook_runs_at_once_when_a_host_is_attached() {
        let rig = Rig::new("hs-first-hook-now", false, Duration::from_secs(60));
        let (a, b) = (loc("/herdr/a.sock"), loc("/herdr/b.sock"));
        rig.world.set_live(a.as_str(), true);
        rig.world.set_live(b.as_str(), true);
        rig.sessions.resolve(&a, &rig.budget(10_000)).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        rig.sessions.on_first_attach(recording_hook(&seen));
        assert_eq!(*seen.lock().unwrap(), vec![a.as_str().to_owned()]);
        rig.sessions.resolve(&b, &rig.budget(10_000)).unwrap();
        assert_eq!(
            seen.lock().unwrap().len(),
            1,
            "a later attach does not rerun it"
        );
    }

    // Kills: an attach per caller (a check-then-act race outside attach_lock).
    #[test]
    fn resolve_attaches_once_under_8_concurrent_first_calls() {
        let rig = Rig::new("hs-concurrent", false, Duration::from_secs(60));
        let locator = loc("/herdr/one.sock");
        rig.world.set_live(locator.as_str(), true);
        assert!(
            !rig.sessions.any_reachable(),
            "no session, nothing reachable"
        );
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let ids: Vec<HostId> = thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|_| {
                    let barrier = Arc::clone(&barrier);
                    let (sessions, locator, budget) = (&rig.sessions, &locator, rig.budget(10_000));
                    scope.spawn(move || {
                        barrier.wait();
                        sessions.resolve(locator, &budget).unwrap().host_id()
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert!(ids.iter().all(|id| *id == ids[0]));
        assert_eq!(rig.world.built.load(Ordering::SeqCst), 1);
        assert_eq!(rig.sessions.attached_count(), 1);
        assert_eq!(rig.index.ids(), vec![ids[0]]);
        let session = rig.sessions.get(ids[0]).unwrap();
        assert_eq!(session.host_id(), ids[0]);
        assert_eq!(session.locator(), &locator);
        assert_eq!(rig.sessions.by_locator(&locator).unwrap().host_id(), ids[0]);
    }

    #[test]
    fn missing_socket_is_host_unavailable() {
        let rig = Rig::new("hs-missing", false, Duration::from_secs(60));
        let locator = loc("/herdr/gone.sock");
        let error = rig
            .sessions
            .resolve(&locator, &rig.budget(5_000))
            .err()
            .unwrap();
        assert_eq!(error.code, ErrorCode::HostUnavailable);
        assert_eq!(error.detail, "Herdr server /herdr/gone.sock not reachable");
        assert_eq!(rig.sessions.attached_count(), 0);
        assert!(rig.index.ids().is_empty());
    }

    // Kills: accepting a locator that reaches the socket through a symlink.
    #[test]
    fn non_canonical_locator_is_invalid_request() {
        let rig = Rig::new("hs-canonical", true, Duration::from_secs(60));
        let real = rig._isolation.path("real");
        std::fs::create_dir(&real).unwrap();
        let link = rig._isolation.path("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let through_link = loc(link.join("herdr.sock").to_str().unwrap());
        rig.world.set_live(through_link.as_str(), true);
        let error = rig
            .sessions
            .resolve(&through_link, &rig.budget(5_000))
            .err()
            .unwrap();
        assert_eq!(error.code, ErrorCode::InvalidRequest);
        assert_eq!(error.detail, "host locator is not canonical");
        assert_eq!(rig.sessions.attached_count(), 0);
    }

    // Kills: the startup hint failing the canonical check in production
    // (check_canonical on) when its socket does not exist yet.
    #[test]
    fn startup_hint_with_missing_socket_attaches_with_canonical_check_on() {
        let rig = Rig::new("hs-hint-canon", true, Duration::from_secs(60));
        let hint = loc(rig._isolation.path("absent.sock").to_str().unwrap());
        rig.world.set_live(hint.as_str(), true);
        rig.sessions.attach_startup(Some(&hint)).unwrap();
        assert!(rig.lines().is_empty(), "{:?}", rig.lines());
        assert!(rig.sessions.by_locator(&hint).is_some());
        // A non-hint resolve of the same non-existent path stays refused.
        let other = loc(rig._isolation.path("other.sock").to_str().unwrap());
        rig.world.set_live(other.as_str(), true);
        let error = rig
            .sessions
            .resolve(&other, &rig.budget(5_000))
            .err()
            .unwrap();
        assert_eq!(error.code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn seventeenth_host_is_refused() {
        let rig = Rig::new("hs-cap", false, Duration::from_secs(60));
        for n in 0..MAX_ATTACHED_HOSTS {
            let locator = format!("/herdr/h{n}.sock");
            rig.world.set_live(&locator, true);
            rig.sessions
                .resolve(&loc(&locator), &rig.budget(10_000))
                .unwrap();
        }
        assert_eq!(rig.sessions.attached_count(), 16);
        rig.world.set_live("/herdr/extra.sock", true);
        let error = rig
            .sessions
            .resolve(&loc("/herdr/extra.sock"), &rig.budget(5_000))
            .err()
            .unwrap();
        assert_eq!(error.code, ErrorCode::HostUnavailable);
        assert_eq!(error.detail, "too many Herdr servers attached (16)");
        assert_eq!(rig.sessions.attached_count(), 16);
        assert_eq!(rig.index.ids().len(), 16);
    }

    // Kills: a failed attach leaving a registered Pacer, a running thread or an
    // index entry behind.
    #[test]
    fn spawn_failure_unregisters_pacer_and_publishes_nothing() {
        let rig = Rig::new("hs-spawn-fail", false, Duration::from_secs(60));
        let locator = loc("/herdr/fail.sock");
        rig.world.set_live(locator.as_str(), true);
        let fault = Failpoint::error(
            "host_sessions.spawn_wakes",
            locator.as_str(),
            ErrorCode::StoreBusy,
        );
        let error = rig
            .sessions
            .resolve(&locator, &rig.budget(5_000))
            .err()
            .unwrap();
        assert_eq!(error.code, ErrorCode::HostUnavailable);
        assert_eq!(fault.fired(), 1);
        assert!(!rig.kicks.registered(Lane::Wakes));
        assert!(!rig.kicks.registered(Lane::Observation));
        assert!(rig.index.ids().is_empty());
        assert_eq!(rig.sessions.attached_count(), 0);
        assert!(rig.slot.lock().unwrap().is_empty());
        // The failure is not sticky: the next attach starts normally.
        drop(fault);
        rig.sessions.resolve(&locator, &rig.budget(5_000)).unwrap();
        assert!(rig.kicks.registered(Lane::Wakes));
        assert_eq!(rig.index.ids().len(), 1);
    }

    // Kills: stop_all returning while a session thread still runs.
    #[test]
    fn stop_all_joins_every_session_thread() {
        let rig = Rig::new("hs-stop", false, Duration::from_secs(60));
        for n in 0..3 {
            let locator = format!("/herdr/s{n}.sock");
            rig.world.set_live(&locator, true);
            rig.sessions
                .resolve(&loc(&locator), &rig.budget(10_000))
                .unwrap();
        }
        rig.sessions.start_sweep().unwrap();
        let names: Vec<String> = rig
            .slot
            .lock()
            .unwrap()
            .iter()
            .map(|h| h.thread().name().unwrap().to_owned())
            .collect();
        assert_eq!(names.len(), 7, "{names:?}");
        assert!(names.iter().any(|n| n.starts_with("herdr-observations-")));
        assert!(names.iter().any(|n| n.starts_with("herdr-wakes-")));
        assert!(names.contains(&"herdr-host-sweep".to_owned()));
        rig.sessions.stop_all();
        assert!(rig.slot.lock().unwrap().is_empty(), "every handle joined");
        assert!(rig.sessions.attached().iter().all(|s| s.is_stopped()));
        let error = rig
            .sessions
            .resolve(&loc("/herdr/late.sock"), &rig.budget(1_000))
            .err()
            .unwrap();
        assert_eq!(error.code, ErrorCode::HostUnavailable);
    }

    #[test]
    fn attach_startup_skips_missing_hint_and_attaches_remembered_live_rows() {
        let rig = Rig::new("hs-startup", false, Duration::from_secs(60));
        rig.remember("/herdr/live.sock");
        rig.remember("/herdr/dead.sock");
        rig.world.set_live("/herdr/live.sock", true);
        let hint = loc("/herdr/missing.sock");
        rig.sessions.attach_startup(Some(&hint)).unwrap();
        assert_eq!(
            rig.lines(),
            vec!["host attach /herdr/missing.sock: Herdr server /herdr/missing.sock not reachable"]
        );
        let attached: Vec<String> = rig
            .sessions
            .attached()
            .iter()
            .map(|s| s.locator().as_str().to_owned())
            .collect();
        assert_eq!(attached, vec!["/herdr/live.sock"]);
        assert!(rig.sessions.by_locator(&loc("/herdr/dead.sock")).is_none());
    }

    // Kills: a remembered row of a kind outside the implemented set dialled as
    // Herdr (startup, sweep, or an explicit resolve); a Herdr row left
    // un-stamped by its reattach.
    #[test]
    fn reattach_skips_rows_of_an_unimplemented_kind_and_stamps_herdr_rows() {
        let rig = Rig::new("hs-unknown-kind", false, Duration::from_millis(20));
        rig.remember("/herdr/live.sock");
        rig.remember("/herdr/future.sock");
        let db = rusqlite::Connection::open(rig._isolation.path("store.db")).unwrap();
        db.execute(
            "UPDATE herdr_hosts SET kind='future', kind_probed_at=NULL WHERE endpoint='/herdr/future.sock'",
            [],
        )
        .unwrap();
        db.execute(
            "UPDATE herdr_hosts SET kind_probed_at=NULL WHERE endpoint='/herdr/live.sock'",
            [],
        )
        .unwrap();
        rig.world.set_live("/herdr/live.sock", true);
        rig.world.set_live("/herdr/future.sock", true);
        rig.sessions.attach_startup(None).unwrap();
        // Let several sweep ticks pass: the sweep must not pick the row up either.
        Cancellation::default().wait_blocking(Duration::from_millis(100));
        let attached: Vec<String> = rig
            .sessions
            .attached()
            .iter()
            .map(|s| s.locator().as_str().to_owned())
            .collect();
        assert_eq!(attached, vec!["/herdr/live.sock"]);
        assert_eq!(rig.world.built.load(Ordering::SeqCst), 1);
        let explicit = rig
            .sessions
            .resolve(&loc("/herdr/future.sock"), &rig.budget(5_000))
            .err()
            .expect("an explicit locator of an unimplemented kind is refused");
        assert_eq!(explicit.code, ErrorCode::HostUnavailable);
        assert_eq!(rig.world.built.load(Ordering::SeqCst), 1);
        let stamped = |endpoint: &str| -> (String, Option<i64>) {
            db.query_row(
                "SELECT kind,kind_probed_at FROM herdr_hosts WHERE endpoint=?1",
                [endpoint],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
        };
        let (kind, probed) = stamped("/herdr/live.sock");
        assert_eq!(kind, "herdr");
        assert!(probed.is_some(), "the Herdr reattach stamps kind_probed_at");
        assert_eq!(stamped("/herdr/future.sock"), ("future".to_owned(), None));
    }

    // Kills: a primary picked by lowest id when the hint (or the most recently
    // attached remembered server) names another one.
    #[test]
    fn startup_primary_prefers_hint_then_most_recent_remembered() {
        let rig = Rig::new("hs-primary", false, Duration::from_secs(60));
        assert!(rig.sessions.startup_primary(None).is_none());
        for name in ["/herdr/a.sock", "/herdr/b.sock", "/herdr/c.sock"] {
            rig.world.set_live(name, true);
        }
        // Stamps order by attach order: a < b < c.
        for (stamp, name) in [(10, "/herdr/a.sock"), (20, "/herdr/b.sock")] {
            rig.store
                .attach_host(&loc(name), stamp, &rig.budget(5_000))
                .unwrap();
        }
        rig.sessions.attach_startup(None).unwrap();
        assert_eq!(rig.sessions.attached_count(), 2);
        let primary = rig.sessions.startup_primary(None).unwrap();
        assert_eq!(primary.locator().as_str(), "/herdr/b.sock");
        let hint = loc("/herdr/c.sock");
        rig.sessions.resolve(&hint, &rig.budget(5_000)).unwrap();
        let primary = rig.sessions.startup_primary(Some(&hint)).unwrap();
        assert_eq!(primary.locator(), &hint);
        let missing = loc("/herdr/missing.sock");
        let fallback = rig.sessions.startup_primary(Some(&missing)).unwrap();
        assert_ne!(fallback.locator(), &missing);
    }

    // Kills: the probe running after (or without) the row write, so a crash or
    // a refused probe leaves a wrongly kinded row; the probed kind not reaching
    // the stored column or `build`.
    #[test]
    fn probe_runs_before_the_row_and_its_kind_is_stored_and_built() {
        let rig = Rig::new("hs-probe-order", false, Duration::from_secs(60));
        rig.world.set_live("/hc/core.sock", true);
        rig.world
            .kinds
            .lock()
            .unwrap()
            .insert("/hc/core.sock".into(), Some(HostKind::Hc));
        rig.sessions
            .resolve(&loc("/hc/core.sock"), &rig.budget(5_000))
            .unwrap();
        assert_eq!(*rig.world.rows_at_probe.lock().unwrap(), vec![0]);
        assert_eq!(*rig.world.built_kinds.lock().unwrap(), vec![HostKind::Hc]);
        let db = rusqlite::Connection::open(rig._isolation.path("store.db")).unwrap();
        let (kind, probed): (String, Option<i64>) = db
            .query_row(
                "SELECT kind,kind_probed_at FROM herdr_hosts WHERE endpoint='/hc/core.sock'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(kind, "hc");
        assert!(probed.is_some());
    }

    // Kills: a failed probe still writing a row.
    #[test]
    fn failed_probe_writes_no_row() {
        let rig = Rig::new("hs-probe-fail", false, Duration::from_secs(60));
        rig.world.set_live("/herdr/silent.sock", true);
        rig.world
            .kinds
            .lock()
            .unwrap()
            .insert("/herdr/silent.sock".into(), None);
        let error = rig
            .sessions
            .resolve(&loc("/herdr/silent.sock"), &rig.budget(5_000))
            .err()
            .expect("a silent peer is not attached");
        assert_eq!(error.code, ErrorCode::HostUnavailable);
        let db = rusqlite::Connection::open(rig._isolation.path("store.db")).unwrap();
        let rows: i64 = db
            .query_row("SELECT count(*) FROM herdr_hosts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0);
    }

    // Kills: a stored kind overwritten by a probe that disagrees with it.
    #[test]
    fn a_kind_mismatch_refuses_the_attach() {
        let rig = Rig::new("hs-probe-mismatch", false, Duration::from_secs(60));
        rig.world.set_live("/herdr/a.sock", true);
        rig.world
            .kinds
            .lock()
            .unwrap()
            .insert("/herdr/a.sock".into(), Some(HostKind::Hc));
        // Stored as Herdr, the socket now answers as hc.
        rig.remember("/herdr/a.sock");
        let error = rig
            .sessions
            .resolve(&loc("/herdr/a.sock"), &rig.budget(5_000))
            .err()
            .expect("a kind mismatch refuses the attach");
        assert_eq!(error.code, ErrorCode::HostUnavailable);
        assert!(error.detail.contains("stored as kind 'herdr'"), "{error:?}");
        assert_eq!(rig.world.built.load(Ordering::SeqCst), 0);
        assert_eq!(rig.sessions.attached_count(), 0);
        let db = rusqlite::Connection::open(rig._isolation.path("store.db")).unwrap();
        let kind: String = db
            .query_row(
                "SELECT kind FROM herdr_hosts WHERE endpoint='/herdr/a.sock'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(kind, "herdr", "the stored kind is not rewritten");
    }

    // Kills: a startup hint whose server is not up yet being refused (no row, no
    // session, no observation lane) instead of attaching unprobed.
    #[test]
    fn a_hint_whose_probe_fails_attaches_unprobed() {
        let rig = Rig::new("hs-hint-silent", false, Duration::from_secs(60));
        let hint = loc("/herdr/hint.sock");
        rig.world.set_live(hint.as_str(), true);
        rig.world
            .kinds
            .lock()
            .unwrap()
            .insert(hint.as_str().into(), None);
        rig.sessions.attach_startup(Some(&hint)).unwrap();
        assert!(rig.sessions.by_locator(&hint).is_some());
        assert_eq!(
            *rig.world.built_kinds.lock().unwrap(),
            vec![HostKind::Herdr]
        );
        assert_eq!(rig.row_kind("/herdr/hint.sock"), ("herdr".into(), None));
    }

    // Kills: a silent hint rebuilding a stamped hc row as the default kind.
    #[test]
    fn a_silent_hint_keeps_a_stamped_row() {
        let rig = Rig::new("hs-hint-stamped", false, Duration::from_secs(60));
        let hint = loc("/hc/core.sock");
        rig.store
            .attach_host_of_kind(&hint, 1, HostKind::Hc, &rig.budget(5_000))
            .unwrap();
        rig.world.set_live(hint.as_str(), true);
        rig.world
            .kinds
            .lock()
            .unwrap()
            .insert(hint.as_str().into(), None);
        rig.sessions.attach_startup(Some(&hint)).unwrap();
        assert!(rig.sessions.by_locator(&hint).is_some());
        assert_eq!(*rig.world.built_kinds.lock().unwrap(), vec![HostKind::Hc]);
        assert_eq!(rig.row_kind("/hc/core.sock"), ("hc".into(), Some(1)));
    }

    fn builtin_parts(no_wake: bool) -> (Arc<dyn HostPort>, Arc<dyn BootstrapObserver>) {
        (Arc::new(FakeHost { no_wake }), Arc::new(NoObserver))
    }

    // Kills: a built-in session listed by `attached()`, so the sweep, startup
    // primary and `any_reachable` would treat it as a dialled server.
    #[test]
    fn attach_builtin_is_never_in_attached() {
        let rig = Rig::new("hs-builtin", false, Duration::from_secs(60));
        let (port, observer) = builtin_parts(false);
        let session = rig
            .sessions
            .attach_builtin(HostKind::Named, port, observer, &loc("/builtin/named"))
            .unwrap();
        assert!(rig.sessions.attached().is_empty());
        assert_eq!(rig.sessions.attached_count(), 0);
        assert!(rig.sessions.startup_primary(None).is_none());
        let all = rig.sessions.all_for_reads();
        assert_eq!(all.len(), 1);
        assert!(Arc::ptr_eq(&all[0], &session));
        let (port, observer) = builtin_parts(false);
        let error = rig
            .sessions
            .attach_builtin(HostKind::Named, port, observer, &loc("/builtin/other"))
            .err()
            .unwrap();
        assert_eq!(error.code, ErrorCode::InvalidRequest);
        assert_eq!(rig.sessions.all_for_reads().len(), 1);
    }

    // Kills: the built-in attach consulting the socket test or counting
    // against the dialled cap.
    #[test]
    fn attach_builtin_skips_factory_exists_and_cap() {
        let rig = Rig::new("hs-builtin-cap", false, Duration::from_secs(60));
        for n in 0..MAX_ATTACHED_HOSTS {
            let locator = format!("/herdr/s{n}.sock");
            rig.world.set_live(&locator, true);
            rig.sessions
                .resolve(&loc(&locator), &rig.budget(10_000))
                .unwrap();
        }
        assert_eq!(rig.sessions.attached_count(), MAX_ATTACHED_HOSTS);
        let calls = Arc::new(AtomicUsize::new(0));
        let factory = HostFactory {
            build: Arc::new(|_, _| unreachable!("the built-in port is given")),
            probe: Arc::new(|_, _| unreachable!("the built-in host is never probed")),
            exists: {
                let calls = Arc::clone(&calls);
                Arc::new(move |_| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    false
                })
            },
        };
        let sessions = Arc::new(HostSessions::new(
            Arc::clone(&rig.index),
            factory,
            SessionDeps::detached(
                Arc::clone(&rig.store) as Arc<dyn StorePort>,
                Arc::new(crate::app::SystemClock::default()),
                "node".into(),
            ),
        ));
        let (port, observer) = builtin_parts(false);
        sessions
            .attach_builtin(
                HostKind::Named,
                port,
                observer,
                &loc("/builtin/not-on-disk"),
            )
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 0, "exists never consulted");
        sessions.stop_all();
        // The dialled registry is full, and the built-in slot still takes one.
        let (port, observer) = builtin_parts(false);
        rig.sessions
            .attach_builtin(HostKind::Herdr, port, observer, &loc("/builtin/past-cap"))
            .unwrap();
        assert_eq!(rig.sessions.attached_count(), MAX_ATTACHED_HOSTS);
        assert_eq!(rig.sessions.all_for_reads().len(), MAX_ATTACHED_HOSTS + 1);
    }

    // Kills: `resolve` trying to dial (exists is false for it) instead of
    // answering the built-in slot.
    #[test]
    fn builtin_found_by_locator_and_resolve() {
        let rig = Rig::new("hs-builtin-find", false, Duration::from_secs(60));
        rig.world.set_live("/herdr/a.sock", true);
        let dialled = rig
            .sessions
            .resolve(&loc("/herdr/a.sock"), &rig.budget(5_000))
            .unwrap();
        let (port, observer) = builtin_parts(false);
        let builtin = rig
            .sessions
            .attach_builtin(HostKind::Named, port, observer, &loc("/builtin/named"))
            .unwrap();
        let named = loc("/builtin/named");
        assert!(Arc::ptr_eq(
            &rig.sessions.by_locator(&named).unwrap(),
            &builtin
        ));
        assert!(Arc::ptr_eq(
            &rig.sessions.resolve(&named, &rig.budget(5_000)).unwrap(),
            &builtin
        ));
        assert!(Arc::ptr_eq(
            &rig.sessions.by_locator(&loc("/herdr/a.sock")).unwrap(),
            &dialled
        ));
        let all = rig.sessions.all_for_reads();
        assert_eq!(all.len(), 2);
        assert!(Arc::ptr_eq(&all[0], &dialled), "dialled first");
        assert!(Arc::ptr_eq(&all[1], &builtin), "built-in last");
        assert_eq!(rig.sessions.attached().len(), 1);
    }

    // Kills: a wake lane (pacer, kick registration, worker) started for a host
    // that cannot wake.
    #[test]
    fn no_wake_lane_for_wake_none() {
        let rig = Rig::new("hs-no-wake", false, Duration::from_secs(60));
        let before = rig.slot.lock().unwrap().len();
        let (port, observer) = builtin_parts(true);
        let session = rig
            .sessions
            .attach_builtin(HostKind::Named, port, observer, &loc("/builtin/named"))
            .unwrap();
        assert!(session.wake().is_none());
        assert!(!rig.kicks.registered(Lane::Wakes));
        assert!(rig.kicks.registered(Lane::Observation));
        let names: Vec<String> = rig
            .slot
            .lock()
            .unwrap()
            .iter()
            .skip(before)
            .map(|h| h.thread().name().unwrap().to_owned())
            .collect();
        assert_eq!(names.len(), 1, "{names:?}");
        assert!(names[0].starts_with("herdr-observations-"));
        let entry = session.health_entry();
        assert!(
            entry
                .lanes
                .iter()
                .all(|lane| !lane.lane.starts_with("wake"))
        );
        // A waking host keeps its lane.
        rig.world.set_live("/herdr/a.sock", true);
        let dialled = rig
            .sessions
            .resolve(&loc("/herdr/a.sock"), &rig.budget(5_000))
            .unwrap();
        assert!(dialled.wake().is_some());
        assert!(rig.kicks.registered(Lane::Wakes));
    }

    #[test]
    fn sweep_attaches_row_whose_socket_appears() {
        let rig = Rig::new("hs-sweep", false, Duration::from_millis(20));
        rig.remember("/herdr/later.sock");
        rig.sessions.start_sweep().unwrap();
        // Several sweep ticks pass (20 ms period) with the socket absent.
        Cancellation::default().wait_blocking(Duration::from_millis(100));
        assert_eq!(rig.sessions.attached_count(), 0, "no socket, no attach");
        assert!(rig.lines().is_empty(), "a missing socket is not logged");
        rig.world.set_live("/herdr/later.sock", true);
        wait("the sweep to attach the row", &|| {
            rig.sessions.attached_count() == 1
        });
        assert_eq!(rig.index.ids().len(), 1);
    }

    // Kills: a lock wait that never ends: past the request budget the caller
    // gets StoreBusy, not a hang.
    #[test]
    fn resolve_past_its_budget_while_another_attach_holds_the_lock_is_store_busy() {
        let rig = Rig::new("hs-busy", false, Duration::from_secs(60));
        let held = rig.sessions.attach_lock.lock().unwrap();
        let error = rig
            .sessions
            .resolve(&loc("/herdr/busy.sock"), &rig.budget(50))
            .err()
            .unwrap();
        assert_eq!(error.code, ErrorCode::StoreBusy);
        drop(held);
    }
}
