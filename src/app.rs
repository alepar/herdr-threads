//! Elected production storage and local service composition.

use crate::{
    daemon::{
        control::{ControlService, StopController},
        health::{ComponentStatus, HarnessStatus, HealthInputs, RetirementHealth},
        ownership::EndpointDescriptor,
        paths::InstancePaths,
        run_elected_with_diagnostics,
    },
    ports::{HostPort, LocalService, StorePort},
    protocol::{
        results::{CapabilityState, HealthSettings},
        time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis},
    },
    service::{
        config::ServiceConfig,
        dispatch::DomainService,
        fair_writer::FairWriter,
        host_evidence::HostEvidenceStatus,
        kicks::{CommitKicks, Lane},
        pacer::Pacer,
        workers::{
            WorkerStatus, admission_handle, start_admission_observer, start_deadline_worker,
            start_observation_worker, start_retention_worker, start_wake_worker,
        },
    },
    store::{SqliteStore, connection::StoreContext},
};
use std::{
    io,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

/// UTC is used for durable deadlines; monotonic elapsed time is used for
/// request budgets and wake spacing. Neither domain is derived from the other.
pub struct SystemClock {
    started: Instant,
}
impl SystemClock {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
        }
    }
}
impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}
impl Clock for SystemClock {
    fn utc_now(&self) -> UtcMillis {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        UtcMillis(i64::try_from(millis).unwrap_or(i64::MAX))
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX))
    }
}

/// Upper bound on the retirement-summary read inside one Health request.
pub(crate) const HEALTH_SUMMARY_LIMIT_MS: u64 = 100;

/// The retirement-summary budget for one Health request: the request's own
/// cancellation, and the earlier of the request deadline and
/// `now + HEALTH_SUMMARY_LIMIT_MS`. Client disconnect, request expiry and
/// shutdown therefore stop the read; Health never mints its own cancellation.
pub(crate) fn health_summary_budget(request: &CallBudget, clock: &dyn Clock) -> CallBudget {
    let cap = clock
        .monotonic_now()
        .0
        .saturating_add(HEALTH_SUMMARY_LIMIT_MS);
    CallBudget {
        deadline: MonoInstant(request.deadline.0.min(cap)),
        cancellation: request.cancellation.clone(),
    }
}

/// The production elected Health provider. `run_elected` and the
/// production-seam tests both build Health through this one function, so the
/// store wiring they exercise is the wiring the daemon runs. The provider's
/// only entry point takes the current request budget, and the retirement
/// read receives `health_summary_budget` of it: request cancellation (client
/// disconnect, shutdown) and request expiry both stop a blocked read.
/// `workers` is the lane statuses in `Lane::ALL` order: deadline, wake and
/// observation first, then (when present) retention and the admission observer.
/// `host` supplies the host fields: they come only from evidence the
/// observation lane observed, under the adapter's incarnation witness.
pub(crate) fn elected_health_provider(
    instance: Uuid,
    boot: Uuid,
    settings: HealthSettings,
    clock: Arc<dyn Clock>,
    store: Arc<dyn StorePort>,
    workers: impl Into<Vec<Arc<WorkerStatus>>>,
    host: ElectedHostEvidence,
) -> impl Fn(&CallBudget) -> HealthInputs + Send + Sync + 'static {
    let workers = workers.into();
    let health = ElectedHealth {
        instance,
        boot,
        settings,
        clock,
        store,
        workers,
        host,
    };
    move |request: &CallBudget| health.inputs(request)
}

/// The host evidence the elected Health provider reports: the status the
/// observation worker records into, and the adapter's static incarnation
/// witness (`HostPort::incarnation_witness`), read once at composition.
pub(crate) struct ElectedHostEvidence {
    pub(crate) status: Arc<HostEvidenceStatus>,
    pub(crate) incarnation_witness: CapabilityState,
    /// The adapter's static cooperative wake-prompt capability
    /// (`HostPort::safe_prompt_capability`), read once at composition.
    pub(crate) safe_prompt: CapabilityState,
    /// The installed `claude` and `codex` observed on the daemon's `PATH`
    /// at boot; Unknown until the bounded background observation completes.
    pub(crate) harnesses: Arc<Mutex<HarnessObservations>>,
}

/// The daemon's boot observation of each installed harness.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct HarnessObservations {
    pub(crate) claude: HarnessStatus,
    pub(crate) codex: HarnessStatus,
}

struct ElectedHealth {
    instance: Uuid,
    boot: Uuid,
    settings: HealthSettings,
    clock: Arc<dyn Clock>,
    store: Arc<dyn StorePort>,
    workers: Vec<Arc<WorkerStatus>>,
    host: ElectedHostEvidence,
}

impl ElectedHealth {
    fn inputs(&self, request: &CallBudget) -> HealthInputs {
        let mut inputs = HealthInputs::unknown(self.instance, self.boot);
        inputs.settings = Some(self.settings.clone());
        inputs.database = ComponentStatus::Ready;
        inputs.schema = ComponentStatus::Ready;
        let summary_budget = health_summary_budget(request, self.clock.as_ref());
        match self.store.retirement_summary(&summary_budget) {
            Ok(summary) => {
                inputs.retirement = RetirementHealth {
                    pending: summary.pending,
                    degraded: summary.degraded,
                }
            }
            Err(_) => {
                inputs.database = ComponentStatus::Degraded("retirement status unavailable".into())
            }
        }
        // Host reachability, coherent enumeration and reconciliation
        // time come only from evidence the observation lane observed.
        let observed = self.host.status.health(self.host.incarnation_witness);
        inputs.host = observed.host;
        inputs.coherent_enumeration = observed.coherent_enumeration;
        inputs.last_reconciliation_at = observed.last_reconciliation_at;
        // Recomputed on every Health read, so an operator repair or a
        // re-registration is reflected without a daemon restart.
        // The first unavailable read names the database degradation.
        let degrade = |inputs: &mut HealthInputs, detail: &str| {
            if inputs.database == ComponentStatus::Ready {
                inputs.database = ComponentStatus::Degraded(detail.into());
            }
        };
        match self.store.binding_evidence_current(&summary_budget) {
            Ok(report) => inputs.binding_evidence = report,
            Err(_) => {
                inputs.binding_evidence = self.store.binding_evidence_startup();
                degrade(&mut inputs, "binding evidence status unavailable");
            }
        }
        match self.store.unresolved_seat_summary(&summary_budget) {
            Ok(summary) => inputs.unresolved = Some(summary),
            Err(_) => degrade(&mut inputs, "unresolved seat status unavailable"),
        }
        inputs.current_execution = CapabilityState::Unsupported;
        inputs.safe_prompt = self.host.safe_prompt;
        inputs.receipt_registration = CapabilityState::Unsupported;
        // Each lane reports only its typed redacted status (class and code,
        // no free text, no seat/attempt identity); exact detail stays in the
        // lane's private diagnostics and durable inspect rows.
        inputs.scheduler = scheduler_status(&self.workers);
        inputs.degraded_lanes = degraded_lanes(&self.workers);
        inputs.transitions_refused = self.host.status.transitions_refused();
        inputs.last_scheduler_tick_at = self.workers.first().and_then(|status| status.last_tick());
        if let Ok(observed) = self.host.harnesses.lock() {
            inputs.claude = observed.claude.clone();
            inputs.codex = observed.codex.clone();
        }
        inputs
    }
}

/// Every lane that recorded a failure after its last success (a poisoned
/// status mutex counts), in `Lane::ALL` order. Health folds more than two of
/// them into one line and points at the daemon log.
fn degraded_lanes(workers: &[Arc<WorkerStatus>]) -> Vec<crate::daemon::health::LaneDegradation> {
    workers
        .iter()
        .zip(Lane::ALL)
        .filter_map(|(status, lane)| {
            status
                .health()
                .map(|health| crate::daemon::health::LaneDegradation {
                    lane: lane.name(),
                    summary: health.summary(),
                    class: health
                        .failure
                        .as_ref()
                        .and_then(crate::service::workers::RedactedFailure::code)
                        .and_then(crate::protocol::results::ErrorCode::default_class),
                })
        })
        .collect()
}

/// A lane whose thread ended without a requested stop outranks any typed
/// failure: it is no longer making progress at all.
/// `workers` follows `Lane::ALL` order, so a prefix of it names its lanes.
fn scheduler_status(workers: &[Arc<WorkerStatus>]) -> ComponentStatus {
    let dead: Vec<&str> = workers
        .iter()
        .zip(Lane::ALL)
        .filter(|(status, _)| status.lane_dead())
        .map(|(_, lane)| lane.name())
        .collect();
    if !dead.is_empty() {
        return ComponentStatus::Degraded(format!(
            "scheduler lane stopped unexpectedly: {}",
            dead.join(", ")
        ));
    }
    match workers.iter().find_map(|status| status.health()) {
        Some(health) => ComponentStatus::Degraded(health.summary()),
        None => ComponentStatus::Ready,
    }
}

/// `supported` requires BOTH a native-verified receipt (the recipe tables'
/// capability) AND a version some recipe lists; a schema-matched or
/// optimistic version never reaches it, however the tables read.
pub(crate) fn supports_native_receipt(native: CapabilityState, listed: bool) -> bool {
    native == CapabilityState::Supported && listed
}

/// Classify the admission of the installed `codex`: cooperative (or
/// supported, when the recipes declare native receipt) when a recipe admits
/// it, unsupported when absent or refused.
pub(crate) fn codex_status(
    admission: &crate::harness::codex::InstalledAdmission,
    native: CapabilityState,
) -> HarnessStatus {
    use crate::harness::codex::{Admission, InstalledRefusal};
    match &admission.result {
        Err(InstalledRefusal::NotFound) => {
            HarnessStatus::NotInstalled("no executable `codex` on the daemon's PATH".into())
        }
        Err(InstalledRefusal::Refused(_)) => HarnessStatus::Refused(admission.line()),
        Ok(version) => match version.admission() {
            Admission::Optimistic {
                admission: optimistic,
                ..
            } => HarnessStatus::Optimistic(format!(
                "codex {}: {}",
                version.as_str(),
                crate::harness::optimistic_label(optimistic, false)
            )),
            Admission::SchemaMatched { .. } => HarnessStatus::Cooperative {
                detail: admission.line(),
                live_unverified: true,
            },
            Admission::Listed if supports_native_receipt(native, true) => {
                HarnessStatus::Supported(admission.line())
            }
            Admission::Listed => HarnessStatus::Cooperative {
                detail: admission.line(),
                live_unverified: false,
            },
        },
    }
}

/// Classify the installed `claude` (`None` when no executable is on `PATH`;
/// otherwise its `--version` observation, admitted only by a recipe).
pub(crate) fn claude_status(
    observed: Option<Result<String, crate::harness::codex::VersionError>>,
    native: CapabilityState,
) -> HarnessStatus {
    claude_status_in(crate::harness::claude::admission_table(), observed, native)
}

/// [`claude_status`] against an explicit admission table.
pub(crate) fn claude_status_in(
    table: &'static [crate::harness::claude::ClaudeRecipe],
    observed: Option<Result<String, crate::harness::codex::VersionError>>,
    native: CapabilityState,
) -> HarnessStatus {
    use crate::harness::codex::VersionError;
    let recipes = crate::harness::recipe::describe(crate::harness::claude::RECIPES);
    match observed {
        None => HarnessStatus::NotInstalled("no executable `claude` on the daemon's PATH".into()),
        Some(Ok(version)) => {
            use crate::harness::claude::{ClaudeAdmission, admit_in};
            let detail = match admit_in(table, &version) {
                Ok(admitted) => {
                    if let ClaudeAdmission::Optimistic(optimistic) = &admitted.admission {
                        return HarnessStatus::Optimistic(format!(
                            "claude {version}: {}",
                            crate::harness::optimistic_label(optimistic, false)
                        ));
                    }
                    format!("claude {version}: listed: recipe {}", admitted.recipe.id)
                }
                Err(_) => format!("claude {version}: listed"),
            };
            if supports_native_receipt(native, true) {
                HarnessStatus::Supported(detail)
            } else {
                HarnessStatus::Cooperative {
                    detail,
                    live_unverified: false,
                }
            }
        }
        Some(Err(VersionError::Unsupported(version))) => HarnessStatus::Refused(format!(
            "claude {version}: no recipe admits it; supported recipes: {recipes}"
        )),
        Some(Err(VersionError::KnownBroken {
            version,
            range,
            newest_working,
        })) => HarnessStatus::Refused(crate::harness::known_broken_label(
            "claude",
            &version,
            &range,
            newest_working,
        )),
        Some(Err(_)) => HarnessStatus::Refused(format!(
            "claude --version could not be observed or recognized; supported recipes: {recipes}"
        )),
    }
}

/// One harness observation: the installed `claude` or `codex` on `path`,
/// classified for Health. A listed version costs one `--version` run; an
/// unlisted Codex is admitted only when its embedded hook schemas hash-match
/// a recipe (schema-matched, live-unverified). A refused or absent harness is
/// an observation, not a failed pass: the status carries the refusal. The
/// admission-observer lane (`start_admission_observer`) runs
/// [`AdmissionReobserver::pass`] on its Pacer; Health never waits for it.
fn observe_claude(
    path: Option<&std::ffi::OsStr>,
    timeout: Duration,
    cancel: &Cancellation,
) -> HarnessStatus {
    claude_status(
        crate::cli::hook::resolve_on_path("claude", path).map(|binary| {
            crate::harness::claude::observe_installed_version_cancellable(&binary, timeout, cancel)
        }),
        crate::harness::claude::health_capability(),
    )
}

fn observe_codex(
    path: Option<&std::ffi::OsStr>,
    timeout: Duration,
    cancel: &Cancellation,
) -> HarnessStatus {
    let admission = crate::harness::codex::InstalledAdmission::observe_on_path_cancellable(
        path, timeout, cancel,
    );
    codex_status(
        &admission,
        crate::harness::codex::DECLARATION.health_capability(),
    )
}

impl HarnessStatus {
    /// The admission word a binary-change log line states.
    fn admission_word(&self) -> &'static str {
        match self {
            HarnessStatus::Unknown => "unknown",
            HarnessStatus::NotInstalled(_) => "not_found",
            HarnessStatus::Refused(_) => "refused",
            HarnessStatus::Cooperative {
                live_unverified: true,
                ..
            } => crate::harness::codex::SCHEMA_MATCHED_LABEL,
            HarnessStatus::Cooperative { .. } | HarnessStatus::Supported(_) => "listed",
            HarnessStatus::Optimistic(_) => crate::harness::codex::OPTIMISTIC_LABEL,
        }
    }

    /// An observation a later pass may reuse while the binary is unchanged:
    /// an admitted harness (a refusal or an unknown may have been a transient
    /// failure to run `--version`, so it is observed again).
    fn reusable(&self) -> bool {
        matches!(
            self,
            HarnessStatus::Cooperative { .. }
                | HarnessStatus::Supported(_)
                | HarnessStatus::Optimistic(_)
        )
    }
}

/// One harness's last observation and the binary it was observed from.
#[derive(Default)]
struct ObservedBinary {
    identity: Option<crate::harness::BinaryIdentity>,
    status: HarnessStatus,
}

/// The admission-observer pass with re-observation (root spec B6 D3, Wave
/// 25): each pass resolves the harness binaries on `PATH` and compares their
/// identity (canonical path, inode, size, mtime). An admitted harness whose
/// binary is unchanged keeps its observation without another `--version` run;
/// a changed, new or vanished binary is observed again and the change is
/// logged. The pass returns the whole pair, which the lane stores in its slot
/// in one replacement, so Health never reads a half-updated pair.
pub(crate) struct AdmissionReobserver {
    path: Option<std::ffi::OsString>,
    timeout: Duration,
    log: Arc<dyn Fn(&str) + Send + Sync>,
    previous: Mutex<Option<[ObservedBinary; 2]>>,
}

impl AdmissionReobserver {
    pub(crate) fn new(
        path: Option<std::ffi::OsString>,
        timeout: Duration,
        log: Arc<dyn Fn(&str) + Send + Sync>,
    ) -> Self {
        Self {
            path,
            timeout,
            log,
            previous: Mutex::new(None),
        }
    }

    /// One pass over both harnesses. Once `cancel` fires the in-flight
    /// `--version` run is killed, no further harness is observed, and the
    /// previous observations are kept (a half-observed pair is never stored);
    /// the caller discards the returned value.
    pub(crate) fn pass(&self, cancel: &Cancellation) -> HarnessObservations {
        let path = self.path.clone();
        let timeout = self.timeout;
        let mut slot = self.previous.lock().unwrap_or_else(|e| e.into_inner());
        let first = slot.is_none();
        let mut previous = slot.take().unwrap_or_default();
        let mut next: [ObservedBinary; 2] = Default::default();
        let mut reused = [false; 2];
        for (index, name) in ["claude", "codex"].into_iter().enumerate() {
            if cancel.is_cancelled() {
                break;
            }
            let identity = crate::cli::hook::resolve_on_path(name, path.as_deref())
                .and_then(|binary| crate::harness::BinaryIdentity::observe(&binary));
            let before = &mut previous[index];
            let unchanged = identity.is_some() && identity == before.identity;
            let status = if unchanged && before.status.reusable() {
                reused[index] = true;
                std::mem::take(&mut before.status)
            } else {
                let status = match index {
                    0 => observe_claude(path.as_deref(), timeout, cancel),
                    _ => observe_codex(path.as_deref(), timeout, cancel),
                };
                if cancel.is_cancelled() {
                    // The run was cut short: its refusal is not an observation.
                    break;
                }
                if !first && identity != before.identity {
                    let describe = |identity: &Option<crate::harness::BinaryIdentity>| {
                        identity.as_ref().map_or_else(
                            || "none".to_owned(),
                            |identity| identity.path().display().to_string(),
                        )
                    };
                    (self.log)(&format!(
                        "{name} binary changed: {} \u{2192} {}; admission {}",
                        describe(&before.identity),
                        describe(&identity),
                        status.admission_word()
                    ));
                }
                status
            };
            next[index] = ObservedBinary { identity, status };
        }
        if cancel.is_cancelled() {
            for index in 0..2 {
                if reused[index] {
                    previous[index].status = std::mem::take(&mut next[index].status);
                }
            }
            *slot = (!first).then_some(previous);
            return HarnessObservations::default();
        }
        let observations = HarnessObservations {
            claude: next[0].status.clone(),
            codex: next[1].status.clone(),
        };
        *slot = Some(next);
        observations
    }
}

/// Starts the retention lane as the elected factory does: its Pacer is
/// registered with the commit-kick registry (the lane's kick set is empty by
/// decision: no table maps to `Lane::Retention`, so it runs on its tick and on
/// `has_more`) and attached to `status` so Health shows its backoff.
pub(crate) fn start_retention_lane(
    kicks: &CommitKicks,
    clock: Arc<dyn Clock>,
    stop: Cancellation,
    store: Arc<dyn StorePort>,
    writer: Arc<FairWriter>,
    status: Arc<WorkerStatus>,
) -> io::Result<std::thread::JoinHandle<()>> {
    let pacer = Arc::new(Pacer::new(Lane::Retention.name(), clock, stop.clone()));
    kicks.register(Lane::Retention, Arc::clone(&pacer));
    start_retention_worker(store, writer, pacer, stop, status)
}

/// Test-only view of one elected daemon's lanes: the store's per-origin commit
/// counts and flushed kick log, and each registered lane Pacer. It observes
/// only; it adds no behaviour to the daemon. In ordinary builds it is an empty
/// type and every method on it is a no-op.
#[derive(Clone, Default)]
pub struct LaneProbe {
    #[cfg(any(test, feature = "test-support"))]
    inner: Arc<Mutex<ProbeState>>,
}

/// One flushed commit kick: the lanes kicked, the committing thread's origin
/// lane (`None` for a request thread) and when the kick flushed (just after
/// the writer guard was released, before the Pacers were kicked).
#[cfg(any(test, feature = "test-support"))]
pub type KickRecord = (crate::service::kicks::LaneSet, Option<Lane>, Instant);

/// Per-lane injected failures and how often each fired.
#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
struct LaneFaults {
    codes: Mutex<[Option<crate::protocol::results::ErrorCode>; 5]>,
    hits: [std::sync::atomic::AtomicU64; 5],
}

#[cfg(any(test, feature = "test-support"))]
impl LaneFaults {
    /// The error `lane`'s next access fails with, if one is injected.
    fn fire(&self, lane: Lane) -> Option<crate::protocol::results::ApiError> {
        let code = self.codes.lock().unwrap_or_else(|e| e.into_inner())[lane as usize].clone()?;
        self.hits[lane as usize].fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Some(crate::protocol::results::ApiError::new(
            code,
            "injected lane failure",
        ))
    }
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
struct ProbeState {
    store: Option<Arc<SqliteStore>>,
    pacers: Vec<(Lane, Arc<Pacer>)>,
    kicks: Arc<Mutex<Vec<KickRecord>>>,
    /// The daemon's commit-kick registry and its lane statuses (in
    /// `Lane::ALL` order), as the factory wired them.
    registry: Option<Arc<CommitKicks>>,
    statuses: Vec<Arc<WorkerStatus>>,
    /// Injected failures, by lane (see [`LaneProbe::fail_lane`]).
    faults: Arc<LaneFaults>,
    /// Where the lane error log writes instead of the process stderr.
    lane_log_path: Option<std::path::PathBuf>,
}

#[cfg(any(test, feature = "test-support"))]
impl LaneProbe {
    fn state(&self) -> std::sync::MutexGuard<'_, ProbeState> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn attach_store(&self, store: &Arc<SqliteStore>) {
        let mut state = self.state();
        let log = Arc::clone(&state.kicks);
        store.set_kick_sink(Box::new(move |lanes, origin| {
            log.lock()
                .unwrap_or_else(|e| e.into_inner())
                .push((lanes, origin, Instant::now()));
        }));
        let faults = Arc::clone(&state.faults);
        store.set_lane_fault(Some(Arc::new(move |origin| faults.fire(origin?))));
        state.store = Some(Arc::clone(store));
    }
    fn attach_registry(&self, kicks: &Arc<CommitKicks>, statuses: Vec<Arc<WorkerStatus>>) {
        let mut state = self.state();
        state.registry = Some(Arc::clone(kicks));
        state.statuses = statuses;
    }
    /// The injected failure for the admission observer, which holds no store.
    fn admission_fault(
        &self,
    ) -> impl Fn() -> Option<crate::protocol::results::ApiError> + Send + 'static {
        let faults = Arc::clone(&self.state().faults);
        move || faults.fire(Lane::AdmissionObserver)
    }
    fn attach_pacer(&self, lane: Lane, pacer: &Arc<Pacer>) {
        self.state().pacers.push((lane, Arc::clone(pacer)));
    }
    /// Whether the daemon's store and both worker-lane Pacers are attached.
    pub fn attached(&self) -> bool {
        let state = self.state();
        state.store.is_some() && state.pacers.len() >= 2
    }
    /// Commits that changed rows so far, per origin lane name (`request` when
    /// the committing thread holds no lane origin).
    pub fn commit_counts(&self) -> std::collections::BTreeMap<String, u64> {
        self.state()
            .store
            .as_ref()
            .map(|store| store.commit_counts())
            .unwrap_or_default()
    }
    /// Every flushed kick so far, oldest first.
    pub fn kick_log(&self) -> Vec<KickRecord> {
        self.state()
            .kicks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    /// Kicks `lane` as a store commit to one of its mapped tables would. For a
    /// test that writes such a row on a separate connection (which fires no
    /// commit hook) and still wants the lane to see it at once.
    pub fn kick(&self, lane: Lane) {
        // The newest registration wins (a restarted daemon re-registers).
        let pacer = self
            .state()
            .pacers
            .iter()
            .rev()
            .find(|(candidate, _)| *candidate == lane)
            .map(|(_, pacer)| Arc::clone(pacer));
        if let Some(pacer) = pacer {
            pacer.kick();
        }
    }
    /// Whether the elected factory registered `lane`'s Pacer with the commit-kick
    /// registry, the one the store's commit hooks kick through.
    pub fn registered(&self, lane: Lane) -> bool {
        self.state()
            .registry
            .as_ref()
            .is_some_and(|kicks| kicks.registered(lane))
    }
    /// Whether `lane`'s `WorkerStatus` (the one Health reads) has its Pacer
    /// attached; false until the lane thread started.
    pub fn status_has_pacer(&self, lane: Lane) -> bool {
        self.state()
            .statuses
            .get(lane as usize)
            .is_some_and(|status| status.has_pacer())
    }
    /// The Health summary of `lane`'s status; `None` while it reports no failure.
    pub fn lane_health(&self, lane: Lane) -> Option<String> {
        self.state().statuses.get(lane as usize)?.last_error()
    }
    /// How many passes `lane`'s registered Pacer has finished (entered a wait).
    pub fn registered_idle_events(&self, lane: Lane) -> u64 {
        let registry = self.state().registry.clone();
        registry
            .and_then(|kicks| kicks.pacer(lane))
            .map_or(0, |pacer| pacer.idle_events())
    }
    /// Kicks `lane` through the commit-kick registry, as a store commit does.
    pub fn kick_registered(&self, lane: Lane) {
        let registry = self.state().registry.clone();
        if let Some(kicks) = registry {
            kicks.kick(crate::service::kicks::LaneSet::EMPTY.with(lane));
        }
    }
    /// Makes every pass `lane` runs fail with `code` until [`heal_lane`]. The
    /// store lanes fail on their next writer turn or query connection; the
    /// admission observer fails its observation.
    ///
    /// [`heal_lane`]: LaneProbe::heal_lane
    pub fn fail_lane(&self, lane: Lane, code: crate::protocol::results::ErrorCode) {
        self.state()
            .faults
            .codes
            .lock()
            .unwrap_or_else(|e| e.into_inner())[lane as usize] = Some(code);
    }
    /// How many accesses an injected failure has failed on `lane` so far.
    pub fn lane_fault_hits(&self, lane: Lane) -> u64 {
        self.state().faults.hits[lane as usize].load(std::sync::atomic::Ordering::SeqCst)
    }
    /// Consecutive failed passes `lane`'s registered Pacer is backing off from.
    pub fn registered_attempts(&self, lane: Lane) -> u32 {
        let registry = self.state().registry.clone();
        registry
            .and_then(|kicks| kicks.pacer(lane))
            .map_or(0, |pacer| pacer.attempts())
    }
    /// Ends an injected failure.
    pub fn heal_lane(&self, lane: Lane) {
        self.state()
            .faults
            .codes
            .lock()
            .unwrap_or_else(|e| e.into_inner())[lane as usize] = None;
    }
    /// Routes the daemon's lane error log to `path` (appended, created on
    /// demand) instead of the process stderr, which only the detached child
    /// routes into `daemon.log`. Call before the daemon starts.
    pub fn log_lane_errors_to(&self, path: std::path::PathBuf) {
        self.state().lane_log_path = Some(path);
    }
    fn lane_log(&self, clock: Arc<dyn Clock>) -> crate::daemon::logs::RateLimitedLaneLog {
        use crate::daemon::logs::RateLimitedLaneLog;
        let Some(path) = self.state().lane_log_path.clone() else {
            return RateLimitedLaneLog::to_stderr(clock);
        };
        RateLimitedLaneLog::new(
            clock,
            Arc::new(move |line: &str| {
                use std::io::Write;
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if let Ok(mut file) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                {
                    // One append write per line: lanes failing together must not
                    // interleave (`writeln!` on a `File` is two writes, unlike the
                    // production stderr sink, which holds the stderr lock).
                    let _ = file.write_all(format!("{line}\n").as_bytes());
                }
            }),
        )
    }
    /// How many times `lane`'s thread has entered a wait (finished a pass).
    pub fn idle_events(&self, lane: Lane) -> u64 {
        self.state()
            .pacers
            .iter()
            .find(|(candidate, _)| *candidate == lane)
            .map_or(0, |(_, pacer)| pacer.idle_events())
    }
}

#[cfg(not(any(test, feature = "test-support")))]
impl LaneProbe {
    fn attach_store(&self, _: &Arc<SqliteStore>) {}
    fn attach_registry(&self, _: &Arc<CommitKicks>, _: Vec<Arc<WorkerStatus>>) {}
    fn attach_pacer(&self, _: Lane, _: &Arc<Pacer>) {}
    fn lane_log(&self, clock: Arc<dyn Clock>) -> crate::daemon::logs::RateLimitedLaneLog {
        crate::daemon::logs::RateLimitedLaneLog::to_stderr(clock)
    }
}

/// The lifecycle calls the factory only after ownership election and socket
/// bind. The matching instance and boot are injected into SQLite and control.
pub async fn run_elected<R>(
    paths: &InstancePaths,
    clock: Arc<dyn Clock>,
    shutdown: Cancellation,
    config: ServiceConfig,
    host: Arc<dyn HostPort>,
    on_ready: R,
) -> io::Result<bool>
where
    R: FnOnce(&EndpointDescriptor) -> io::Result<()>,
{
    run_elected_impl(
        paths,
        clock,
        shutdown,
        config,
        host,
        LaneProbe::default(),
        on_ready,
    )
    .await
}

/// [`run_elected`] with a [`LaneProbe`] attached to the daemon's store and
/// lane Pacers, so a test can read per-origin commit counts, the flushed kick
/// log and each lane's idle count.
#[cfg(any(test, feature = "test-support"))]
pub async fn run_elected_probed<R>(
    paths: &InstancePaths,
    clock: Arc<dyn Clock>,
    shutdown: Cancellation,
    config: ServiceConfig,
    host: Arc<dyn HostPort>,
    probe: LaneProbe,
    on_ready: R,
) -> io::Result<bool>
where
    R: FnOnce(&EndpointDescriptor) -> io::Result<()>,
{
    run_elected_impl(paths, clock, shutdown, config, host, probe, on_ready).await
}

async fn run_elected_impl<R>(
    paths: &InstancePaths,
    clock: Arc<dyn Clock>,
    shutdown: Cancellation,
    config: ServiceConfig,
    host: Arc<dyn HostPort>,
    probe: LaneProbe,
    on_ready: R,
) -> io::Result<bool>
where
    R: FnOnce(&EndpointDescriptor) -> io::Result<()>,
{
    let database_path = paths.database_path.clone();
    let factory_log_path = crate::daemon::logs::daemon_log_path(paths);
    let factory_clock = Arc::clone(&clock);
    let worker_slot = Arc::new(Mutex::new(Vec::new()));
    let factory_slot = Arc::clone(&worker_slot);
    let worker_stop = Cancellation::default();
    let factory_stop = worker_stop.clone();
    let worker_status = Arc::new(WorkerStatus::default());
    let factory_status = Arc::clone(&worker_status);
    let observation_status = Arc::new(WorkerStatus::default());
    let factory_observation_status = Arc::clone(&observation_status);
    let wake_status = Arc::new(WorkerStatus::default());
    let factory_wake_status = Arc::clone(&wake_status);
    let factory_retention_status = Arc::new(WorkerStatus::default());
    let admission_status = Arc::new(WorkerStatus::default());
    let factory_admission_status = Arc::clone(&admission_status);
    // One rate-limited logger for the daemon; every lane's status reports to it.
    let rate_limited_log = Arc::new(probe.lane_log(Arc::clone(&clock)));
    let lane_log: Arc<dyn crate::daemon::logs::LaneErrorLog> = rate_limited_log.clone();
    // Hook payloads the optimistic recipes could not parse: counted for
    // Health and logged through the same rate-limited logger.
    let hook_parse_failures = Arc::new(crate::daemon::logs::HookParseFailures::new(Arc::clone(
        &rate_limited_log,
    )));
    let factory_parse_failures = Arc::clone(&hook_parse_failures);
    for status in [
        &worker_status,
        &observation_status,
        &wake_status,
        &factory_retention_status,
        &admission_status,
    ] {
        status.set_error_log(Arc::clone(&lane_log));
    }
    let host_evidence = Arc::new(HostEvidenceStatus::default());
    let factory_host_evidence = Arc::clone(&host_evidence);
    let factory_probe = probe;
    run_elected_with_diagnostics(
        paths,
        clock,
        shutdown,
        move |instance, boot, cancellation| {
            let context = StoreContext::new(database_path, Arc::clone(&factory_clock));
            // One registry per elected daemon: the store's commit hooks kick
            // through it and every lane registers its Pacer here at start.
            let kicks = Arc::new(CommitKicks::default());
            let sqlite = Arc::new(
                SqliteStore::new(context, instance.to_string(), config.store_settings(boot))
                    .map_err(|error| {
                        io::Error::other(format!(
                            "store startup: {:?}: {}",
                            error.code, error.detail
                        ))
                    })?
                    .with_commit_kicks(Arc::clone(&kicks)),
            );
            factory_probe.attach_store(&sqlite);
            let store: Arc<dyn StorePort> = sqlite;
            // The writer's startup invariant verification is logged once per
            // daemon boot (the daemon log) and reported by Health/doctor.
            // Written to the process stderr descriptor itself, which the
            // elected child routes into its rotating log.
            if let Some(report) = store.binding_evidence_startup() {
                use std::io::Write;
                let _ = writeln!(
                    std::io::stderr(),
                    "{}",
                    crate::daemon::health::binding_evidence_line(report)
                );
            }
            // Daemon restart creates a new host connection epoch above the
            // previous boot's durable publication before any host observation.
            let persisted_epoch = store
                .persisted_host_epoch(
                    &instance.to_string(),
                    &CallBudget {
                        deadline: MonoInstant(
                            factory_clock.monotonic_now().0.saturating_add(2_000),
                        ),
                        cancellation: Cancellation::default(),
                    },
                )
                .map_err(|error| {
                    io::Error::other(format!("store startup: {:?}: {}", error.code, error.detail))
                })?;
            host.resume_after_epoch(persisted_epoch);
            let incarnation_witness = host.incarnation_witness();
            let safe_prompt = host.safe_prompt_capability();
            let writer = Arc::new(FairWriter::new(32));
            let register_lane = |lane: Lane| {
                let pacer = Arc::new(Pacer::new(
                    lane.name(),
                    Arc::clone(&factory_clock),
                    factory_stop.clone(),
                ));
                kicks.register(lane, Arc::clone(&pacer));
                factory_probe.attach_pacer(lane, &pacer);
                pacer
            };
            let mut workers = factory_slot
                .lock()
                .map_err(|_| io::Error::other("worker slot poisoned"))?;
            let worker = start_deadline_worker(
                Arc::clone(&store),
                Arc::clone(&writer),
                register_lane(Lane::Deadlines),
                factory_stop.clone(),
                Arc::clone(&factory_status),
            )?;
            workers.push(worker);
            workers.push(start_wake_worker(
                Arc::clone(&store),
                Arc::clone(&writer),
                Arc::clone(&host),
                instance.to_string(),
                boot,
                config.retry_config(),
                register_lane(Lane::Wakes),
                factory_stop.clone(),
                Arc::clone(&factory_wake_status),
            )?);
            let observation_pacer = Arc::new(Pacer::new(
                Lane::Observation.name(),
                Arc::clone(&factory_clock),
                factory_stop.clone(),
            ));
            kicks.register(Lane::Observation, Arc::clone(&observation_pacer));
            let identity = Arc::new(
                crate::identity::repair::OrdinaryIdentity::new(
                    instance.to_string(),
                    Arc::clone(&store),
                    host,
                    Arc::clone(&factory_clock),
                    Arc::clone(&writer),
                )
                // Detached invalidation compensation outlives request cancellation
                // but never the owner: shutdown cancels its bounded wait.
                .with_service_cancellation(cancellation.clone())
                .with_observation_pacer(Arc::clone(&observation_pacer)),
            );
            workers.push(start_retention_lane(
                &kicks,
                Arc::clone(&factory_clock),
                factory_stop.clone(),
                Arc::clone(&store),
                Arc::clone(&writer),
                Arc::clone(&factory_retention_status),
            )?);
            let harnesses = Arc::new(Mutex::new(HarnessObservations::default()));
            let reobserver = AdmissionReobserver::new(
                std::env::var_os("PATH"),
                crate::harness::codex::VERSION_TIMEOUT,
                {
                    let log = Arc::clone(&rate_limited_log);
                    Arc::new(move |line: &str| log.write_line(line))
                },
            );
            #[cfg(any(test, feature = "test-support"))]
            let admission_fault = factory_probe.admission_fault();
            workers.extend(admission_handle(
                start_admission_observer(
                    move |budget| {
                        #[cfg(any(test, feature = "test-support"))]
                        if let Some(error) = admission_fault() {
                            return Err(error);
                        }
                        let observed = reobserver.pass(&budget.cancellation);
                        if budget.cancellation.is_cancelled() {
                            return Err(crate::protocol::results::ApiError::cancelled(
                                "admission pass cancelled",
                            ));
                        }
                        Ok(observed)
                    },
                    Arc::clone(&harnesses),
                    register_lane(Lane::AdmissionObserver),
                    Arc::clone(&factory_clock),
                    factory_stop.clone(),
                    Arc::clone(&factory_admission_status),
                ),
                &factory_admission_status,
            ));
            workers.push(start_observation_worker(
                Arc::clone(&identity),
                Arc::clone(&store),
                Arc::clone(&writer),
                factory_stop,
                Arc::clone(&factory_observation_status),
                Arc::clone(&factory_host_evidence),
                observation_pacer,
            )?);
            drop(workers);
            factory_probe.attach_registry(
                &kicks,
                vec![
                    // `Lane::ALL` order.
                    factory_status.clone(),
                    factory_wake_status.clone(),
                    factory_observation_status.clone(),
                    factory_retention_status.clone(),
                    factory_admission_status.clone(),
                ],
            );
            let health_store = Arc::clone(&store);
            let health_clock = Arc::clone(&factory_clock);
            let domain = DomainService::with_identity(
                instance.to_string(),
                store,
                Arc::clone(&factory_clock),
                identity,
            )
            .with_operator_owner(crate::daemon::paths::effective_uid())
            .with_cooperative_owner(crate::daemon::paths::effective_uid(), writer);
            let stop = StopController::new(instance, boot, cancellation);
            let provider = elected_health_provider(
                instance,
                boot,
                config.health_settings(),
                health_clock,
                health_store,
                vec![
                    factory_status.clone(),
                    factory_wake_status.clone(),
                    factory_observation_status.clone(),
                    // `Lane::ALL` order is the slice order.
                    factory_retention_status.clone(),
                    factory_admission_status.clone(),
                ],
                ElectedHostEvidence {
                    status: factory_host_evidence.clone(),
                    incarnation_witness,
                    safe_prompt,
                    harnesses,
                },
            );
            let log_path = factory_log_path.clone();
            let health_parse_failures = Arc::clone(&factory_parse_failures);
            let health = move |request: &CallBudget| {
                let mut inputs = provider(request);
                inputs.log_path = Some(log_path.clone());
                inputs.hook_parse_failures = health_parse_failures.snapshot();
                inputs
            };
            Ok(Arc::new(
                ControlService::new(stop, health, domain)
                    .with_hook_parse_failures(factory_parse_failures),
            ) as Arc<dyn LocalService>)
        },
        on_ready,
        move || async move {
            worker_stop.cancel();
            let workers = std::mem::take(
                &mut *worker_slot
                    .lock()
                    .map_err(|_| io::Error::other("worker slot poisoned"))?,
            );
            let joined = tokio::task::spawn_blocking(move || {
                let mut failed = false;
                for worker in workers {
                    failed |= worker.join().is_err();
                }
                failed
            })
            .await
            .map_err(|_| io::Error::other("service worker join failed"))?;
            if joined {
                return Err(io::Error::other("service worker panicked"));
            }
            Ok(())
        },
    )
    .await
}

#[cfg(test)]
mod lane_liveness_tests {
    use super::*;
    use crate::protocol::time::Cancellation;

    fn lanes() -> [Arc<WorkerStatus>; 3] {
        std::array::from_fn(|_| Arc::new(WorkerStatus::default()))
    }

    #[test]
    fn live_and_cleanly_stopped_lanes_stay_ready() {
        let workers = lanes();
        let cancel = Cancellation::default();
        let _held = workers[0].lane_guard(&cancel);
        assert_eq!(scheduler_status(&workers), ComponentStatus::Ready);
        let stopped = Cancellation::default();
        stopped.cancel();
        drop(workers[1].lane_guard(&stopped));
        assert_eq!(scheduler_status(&workers), ComponentStatus::Ready);
    }

    #[test]
    fn panicked_lane_degrades_health_naming_it() {
        let workers = lanes();
        let cancel = Cancellation::default();
        let status = Arc::clone(&workers[1]);
        let handle = std::thread::spawn(move || {
            let _guard = status.lane_guard(&cancel);
            panic!("simulated wake lane panic");
        });
        assert!(handle.join().is_err());
        match scheduler_status(&workers) {
            ComponentStatus::Degraded(detail) => assert!(detail.contains("wake"), "{detail}"),
            other => panic!("expected degraded, got {other:?}"),
        }
    }

    #[test]
    fn retention_lane_failure_degrades_health_in_one_line_with_retry_suffix() {
        use crate::protocol::results::{ApiError, ErrorCode};
        let workers: [Arc<WorkerStatus>; 4] =
            std::array::from_fn(|_| Arc::new(WorkerStatus::default()));
        let retention = &workers[Lane::Retention as usize];
        let pacer = Arc::new(Pacer::new(
            Lane::Retention.name(),
            Arc::new(SystemClock::new()),
            Cancellation::default(),
        ));
        retention.attach_pacer(Arc::clone(&pacer));
        assert_eq!(scheduler_status(&workers), ComponentStatus::Ready);
        pacer.on_failure();
        retention.record_failure(
            Lane::Retention,
            &ApiError::new(ErrorCode::StoreBusy, "private detail"),
        );
        let ComponentStatus::Degraded(detail) = scheduler_status(&workers) else {
            panic!("a failed retention pass must degrade Health");
        };
        assert!(
            detail.starts_with("lane retention failed: StoreBusy"),
            "{detail}"
        );
        assert!(detail.contains("retrying (attempt 1,"), "{detail}");
        assert!(
            !detail.contains('\n') && !detail.contains("private detail"),
            "{detail}"
        );
        pacer.on_success();
        retention.record_success(crate::protocol::time::UtcMillis(1));
        assert_eq!(scheduler_status(&workers), ComponentStatus::Ready);
    }

    #[test]
    fn early_exit_without_stop_degrades_health_naming_it() {
        let workers = lanes();
        let cancel = Cancellation::default();
        drop(workers[2].lane_guard(&cancel));
        match scheduler_status(&workers) {
            ComponentStatus::Degraded(detail) => {
                assert!(detail.contains("observation"), "{detail}")
            }
            other => panic!("expected degraded, got {other:?}"),
        }
    }
}
