//! The multi-host registry: one session per attached Herdr server.
//!
//! The elected daemon composes one [`HostSessions`] from a [`HostAttachPlan`]
//! (`app::run_elected_impl`): startup attach, the sweep, and `stop_all` in its
//! stop path.

use crate::daemon::health::{ComponentStatus, HostHealthInput, LaneDegradation};
use crate::daemon::logs::LaneErrorLog;
use crate::identity::repair::OrdinaryIdentity;
use crate::ports::{
    BootstrapObserver, HookPullOutlook, HostId, HostPort, ModChannels, NativeLaunchCapability,
    PokeCapabilitySource, StorePort,
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
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, RwLock, TryLockError,
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

type BuildHost =
    Arc<dyn Fn(&HostLocator) -> (Arc<dyn HostPort>, Arc<dyn BootstrapObserver>) + Send + Sync>;
type SocketExists = Arc<dyn Fn(&Path) -> bool + Send + Sync>;
/// The `herdr_hosts` rows this node attached before, most recent first.
pub type RememberedHosts = Arc<dyn Fn(&CallBudget) -> Result<Vec<HostRow>, ApiError> + Send + Sync>;
type LogLine = Arc<dyn Fn(&str) + Send + Sync>;
/// Runs once with the first attached session.
pub type FirstAttachHook = Box<dyn FnOnce(&Arc<HostSession>) + Send>;

/// HostFactory: how a Herdr server's port and observer are built and whether
/// its socket exists.
///
/// Owns: two closures, `build(locator) -> (port, observer)` and `exists(path)`.
/// Hides: `NativeCli` construction and the socket test (production) versus the
/// fake-locator map (tests).
/// Must not: attach, write the store or spawn workers (`HostSessions` does all
/// three).
/// Role: built in `main.rs` or a test, carried by [`HostAttachPlan`] and
/// consumed by `HostSessions`. Two closures, not a trait: one production
/// implementation, and the fake is data. A new way to build a host is a new
/// closure pair; nothing else constructs a `HostPort`.
#[derive(Clone)]
pub struct HostFactory {
    pub build: BuildHost,
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
    evidence: Arc<HostEvidenceStatus>,
    observation: LaneHandle,
    wake: LaneHandle,
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
    pub fn evidence(&self) -> &Arc<HostEvidenceStatus> {
        &self.evidence
    }
    pub fn observation(&self) -> &LaneHandle {
        &self.observation
    }
    pub fn wake(&self) -> &LaneHandle {
        &self.wake
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
        let lanes = [&self.observation, &self.wake]
            .into_iter()
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
        let observer = bootstrap.as_ref().map_or_else(
            || Arc::new(RefusedBootstrapObserver) as Arc<dyn BootstrapObserver>,
            |runtime| Arc::clone(&runtime.observer),
        );
        let lane = |lane: Lane| LaneHandle {
            lane,
            status: Arc::new(WorkerStatus::default()),
            pacer: Arc::new(Pacer::new(lane.name(), Arc::clone(&clock), stop.clone())),
        };
        Arc::new(Self {
            host_id: identity.host_id(),
            locator,
            port: Arc::clone(identity.host_port()),
            observer,
            reachability: Arc::new(HostReachability::default()),
            identity,
            bootstrap,
            evidence: Arc::new(HostEvidenceStatus::default()),
            observation: lane(Lane::Observation),
            wake: lane(Lane::Wakes),
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
/// Owns: up to [`MAX_ATTACHED_HOSTS`] sessions keyed by `HostId`, attached at
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
            build: Arc::new(|_| unreachable!("a prebuilt registry attaches nothing")),
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

    pub fn by_locator(&self, locator: &HostLocator) -> Option<Arc<HostSession>> {
        self.sessions
            .read()
            .ok()?
            .values()
            .find(|session| session.locator == *locator)
            .cloned()
    }

    /// Every session, ascending by host id.
    pub fn attached(&self) -> Vec<Arc<HostSession>> {
        self.sessions
            .read()
            .map(|map| map.values().cloned().collect())
            .unwrap_or_default()
    }

    pub fn attached_count(&self) -> usize {
        self.sessions.read().map_or(0, |map| map.len())
    }

    /// Whether any attached server is not marked down.
    pub fn any_reachable(&self) -> bool {
        self.attached()
            .iter()
            .any(|session| !session.reachability.is_down())
    }

    /// One Health entry per attached server, ascending by host id, with the
    /// server's own failed lanes (`observation[<host_id>]`, `wake[<host_id>]`).
    ///
    /// Operator-only surface: it feeds `Health.hosts` and the degraded-lane
    /// lines of `doctor`, which carry the node-local host id and the socket
    /// path; nothing agent-facing reads it. Read on every Health call, so an
    /// outage or recovery shows without a restart. A server the observation
    /// lane marked down reports unreachable even before its evidence says so.
    pub fn health_entries(&self) -> Vec<HostHealthEntry> {
        self.attached()
            .iter()
            .map(|session| session.health_entry())
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
        self.resolve_checked(locator, budget, true)
    }

    /// `resolve` with the canonical-path check optional: the operator's startup
    /// hint is explicit, so it skips the check (its socket may not exist yet).
    fn resolve_checked(
        &self,
        locator: &HostLocator,
        budget: &CallBudget,
        canonical_check: bool,
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
        self.attach_locked(locator, budget, canonical_check)
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
        canonical_check: bool,
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
        if canonical_check && self.deps.check_canonical && !is_canonical_on_disk(locator.as_path())
        {
            return Err(ApiError::invalid_request("host locator is not canonical"));
        }
        if self.attached_count() >= MAX_ATTACHED_HOSTS {
            return Err(unavailable(format!(
                "too many Herdr servers attached ({MAX_ATTACHED_HOSTS})"
            )));
        }
        let (host_id, epoch) = {
            let _turn = self
                .deps
                .writer
                .enter_foreground(budget, self.deps.clock.as_ref())?;
            self.deps
                .store
                .attach_host(locator, self.deps.clock.utc_now().0, budget)?
        };
        let (port, observer) = (self.factory.build)(locator);
        port.resume_after_epoch(u64::try_from(epoch).unwrap_or(0));

        let session = self.start_session(host_id, locator, port, observer)?;
        if let Ok(mut map) = self.sessions.write() {
            map.insert(host_id, Arc::clone(&session));
        }
        self.index.publish(locator.clone(), host_id);
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
        let wake_pacer = Arc::new(Pacer::new(
            Lane::Wakes.name(),
            Arc::clone(&deps.clock),
            stop.clone(),
        ));
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
        let observation_status = Arc::new(WorkerStatus::default());
        let wake_status = Arc::new(WorkerStatus::default());
        if let Some(log) = &deps.lane_log {
            observation_status.set_error_log(Arc::clone(log));
            wake_status.set_error_log(Arc::clone(log));
        }
        reachability.attach_wake_pacer(Arc::clone(&wake_pacer));
        deps.kicks.register(Lane::Wakes, Arc::clone(&wake_pacer));
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
            started.push(start_wake_worker_named(
                format!("herdr-wakes-{id}"),
                Arc::clone(&deps.store),
                Arc::clone(&deps.writer),
                Arc::clone(&port),
                host_id,
                deps.node_uuid.clone(),
                deps.boot,
                deps.retry,
                Arc::clone(&wake_pacer),
                stop.clone(),
                Arc::clone(&wake_status),
                Arc::clone(&deps.poke_capabilities),
                Arc::clone(&reachability),
                Arc::clone(&deps.mod_channels),
                Arc::clone(&deps.hook_pull),
                deps.liveness.clone(),
                deps.wake_safety_tick,
            )?);
            Ok(())
        })();
        if let Err(error) = spawned {
            stop.cancel();
            deps.kicks.unregister(Lane::Wakes, &wake_pacer);
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
            evidence,
            observation: LaneHandle {
                lane: Lane::Observation,
                status: observation_status,
                pacer: observation_pacer,
            },
            wake: LaneHandle {
                lane: Lane::Wakes,
                status: wake_status,
                pacer: wake_pacer,
            },
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
            && let Err(error) = self.resolve_checked(hint, &self.background_budget(), false)
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
    /// attached `host_id`; `None` when no server is attached.
    pub fn startup_primary(&self, hint: Option<&HostLocator>) -> Option<Arc<HostSession>> {
        let first = *self.startup_first.lock().unwrap_or_else(|e| e.into_inner());
        hint.and_then(|hint| self.by_locator(hint))
            .or_else(|| first.and_then(|id| self.get(id)))
            .or_else(|| self.attached().into_iter().next())
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
        let attached: Vec<HostLocator> = self
            .attached()
            .iter()
            .map(|session| session.locator.clone())
            .collect();
        let mut rows: Vec<HostRow> = self
            .remembered_rows()
            .into_iter()
            .filter(|row| !attached.contains(&row.locator))
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
        for session in self.attached() {
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
    struct FakeHost;

    fn refused<T>() -> Result<T, ApiError> {
        Err(ApiError::unsupported("fake host"))
    }

    impl HostPort for FakeHost {
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
                    Arc::new(move |_| {
                        world.built.fetch_add(1, Ordering::SeqCst);
                        (Arc::new(FakeHost), Arc::new(NoObserver))
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
                deadline: MonoInstant(crate::app::SystemClock::default().monotonic_now().0 + ms),
                cancellation: Cancellation::default(),
            }
        }

        fn lines(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
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
