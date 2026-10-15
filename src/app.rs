//! Elected production storage and local service composition.

use crate::{
    daemon::{
        control::{ControlService, StopController},
        health::{ComponentStatus, HarnessStatus, HealthInputs, RetirementHealth},
        ownership::EndpointDescriptor,
        paths::NodePaths,
        run_elected_late_host,
    },
    node::{NodeIdentity, NodeKeyError},
    ports::{LocalService, StorePort},
    protocol::{
        authority::Harness,
        results::{CapabilityState, HealthSettings},
        time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis},
    },
    service::{
        config::ServiceConfig,
        dispatch::DomainService,
        fair_writer::FairWriter,
        host_evidence::HostEvidenceStatus,
        host_sessions::{HostAttachPlan, HostSession, HostSessions, SessionDeps},
        kicks::{CommitKicks, Lane},
        mod_channels::ModChannelRegistry,
        pacer::Pacer,
        workers::{
            ApplyPostCommit, WorkerStatus, admission_handle, start_admission_observer,
            start_apply_worker, start_deadline_worker, start_mod_channel_worker,
            start_retention_worker,
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
    instance: crate::node::NodeUuid,
    boot: Uuid,
    settings: HealthSettings,
    clock: Arc<dyn Clock>,
    store: Arc<dyn StorePort>,
    workers: impl Into<Vec<Arc<WorkerStatus>>>,
    host: ElectedHostEvidence,
) -> impl Fn(&CallBudget) -> HealthInputs + Send + Sync + 'static {
    let workers = workers.into();
    let started = clock.monotonic_now();
    let health = ElectedHealth {
        instance,
        boot,
        settings,
        started,
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
    /// The adapter, read on each Health for the Herdr release it last saw.
    pub(crate) release: HostReleaseSource,
}

impl ElectedHostEvidence {
    /// The evidence of a composed daemon: the startup session's evidence
    /// status and static adapter capabilities (`fallback` and the neutral
    /// answers when no server is attached), and a release source over every
    /// attached session.
    pub(crate) fn of_sessions(
        sessions: &Arc<HostSessions>,
        primary: Option<&HostSession>,
        fallback: Arc<HostEvidenceStatus>,
        harnesses: Arc<Mutex<HarnessObservations>>,
    ) -> Self {
        let (status, incarnation_witness, safe_prompt) = match primary {
            Some(session) => (
                Arc::clone(session.evidence()),
                session.port().incarnation_witness(),
                session.port().safe_prompt_capability(),
            ),
            None => (
                fallback,
                CapabilityState::Unknown,
                CapabilityState::Unsupported,
            ),
        };
        Self {
            status,
            incarnation_witness,
            safe_prompt,
            harnesses,
            release: HostReleaseSource(Some(Arc::clone(sessions))),
        }
    }
}

/// Where Health reads the attached Herdr servers and their observed release;
/// the default has none.
#[derive(Default)]
pub(crate) struct HostReleaseSource(Option<Arc<HostSessions>>);

impl HostReleaseSource {
    /// Herdr servers attached: one per session, 0 store-only.
    fn attached(&self) -> u32 {
        self.0.as_ref().map_or(0, |sessions| {
            u32::try_from(sessions.attached_count()).unwrap_or(u32::MAX)
        })
    }

    fn observed(&self) -> Option<crate::host::compatibility::HostRelease> {
        self.0.as_ref().and_then(|sessions| {
            sessions
                .attached()
                .iter()
                .find_map(|session| session.port().observed_release())
        })
    }
}

/// The daemon's boot observation of each installed harness.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct HarnessObservations {
    pub(crate) entries:
        std::collections::BTreeMap<String, crate::harness::adapter::DaemonObservation>,
}

/// Ordinary observation supplies no safe current-runtime qualifier. Metadata
/// and historical recipe declarations alone cannot grant richer poke behavior.
pub(crate) struct ObservedPokeCapabilities;

impl ObservedPokeCapabilities {
    pub(crate) fn new(_observed: Arc<Mutex<HarnessObservations>>) -> Self {
        Self
    }
}

impl crate::ports::PokeCapabilitySource for ObservedPokeCapabilities {
    fn capabilities(&self, _: Harness) -> crate::harness::recipe::PokeCapabilities {
        crate::harness::recipe::PokeCapabilities::NONE
    }
}

impl HarnessObservations {
    pub(crate) fn status(&self, harness: &str) -> HarnessStatus {
        self.entries
            .get(harness)
            .map(|entry| entry.status.clone())
            .unwrap_or_default()
    }

    /// The detected version of `harness` (`claude` or `codex`).
    pub(crate) fn detected_version(&self, harness: &str) -> Option<String> {
        self.entries
            .get(harness)
            .and_then(|entry| entry.identity.as_ref())
            .and_then(|identity| identity.release().map(str::to_owned))
    }
}

struct ElectedHealth {
    instance: crate::node::NodeUuid,
    boot: Uuid,
    settings: HealthSettings,
    clock: Arc<dyn Clock>,
    store: Arc<dyn StorePort>,
    workers: Vec<Arc<WorkerStatus>>,
    host: ElectedHostEvidence,
    /// The provider's construction at daemon boot, on `clock`: Health's
    /// startup bound (`starting`) is measured from it.
    started: MonoInstant,
}

impl ElectedHealth {
    fn inputs(&self, request: &CallBudget) -> HealthInputs {
        let mut inputs = HealthInputs::unknown(self.instance, self.boot);
        inputs.settings = Some(self.settings.clone());
        inputs.hosts = self.host.release.attached();
        inputs.since_start_ms = Some(self.clock.monotonic_now().0.saturating_sub(self.started.0));
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
        if let Some(release) = self.host.release.observed() {
            inputs.host_version = Some(release.summary());
            inputs.host_release_warning = release.warning();
        }
        // Each lane reports only its typed redacted status (class and code,
        // no free text, no seat/attempt identity); exact detail stays in the
        // lane's private diagnostics and durable inspect rows.
        inputs.scheduler = scheduler_status(&self.workers);
        inputs.degraded_lanes = degraded_lanes(&self.workers);
        inputs.transitions_refused = self.host.status.transitions_refused();
        inputs.last_scheduler_tick_at = self.workers.first().and_then(|status| status.last_tick());
        if let Ok(observed) = self.host.harnesses.lock() {
            // A harness with a legacy contract always reports (`Unknown` until
            // observed); any other only once observed: an optional harness the
            // observer has not reached yet is not an unknown core harness.
            inputs.harnesses = crate::harness::registry::builtins()
                .registrations()
                .iter()
                .filter_map(|r| {
                    let id = r.metadata().id;
                    if r.legacy_contract_id().is_some() {
                        Some((id.to_owned(), observed.status(id)))
                    } else {
                        observed
                            .entries
                            .get(id)
                            .map(|entry| (id.to_owned(), entry.status.clone()))
                    }
                })
                .collect();
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
#[cfg(test)]
pub(crate) fn supports_native_receipt(native: CapabilityState, listed: bool) -> bool {
    native == CapabilityState::Supported && listed
}

/// Classify the admission of the installed `codex`: cooperative (or
/// supported, when the recipes declare native receipt) when a recipe admits
/// it, unsupported when absent or refused.
#[cfg(test)]
pub(crate) fn codex_status(
    admission: &crate::harness::codex::InstalledAdmission,
    native: CapabilityState,
) -> HarnessStatus {
    use crate::harness::codex::{Admission, InstalledRefusal};
    match &admission.result {
        Err(InstalledRefusal::NotFound) => {
            HarnessStatus::NotInstalled("no executable `codex` on the daemon's PATH".into())
        }
        Err(InstalledRefusal::Refused(
            crate::harness::codex::VersionError::Unsupported(_)
            | crate::harness::codex::VersionError::KnownBroken { .. },
        )) => HarnessStatus::VersionRefused(admission.line()),
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
#[cfg(test)]
pub(crate) fn claude_status(
    observed: Option<Result<String, crate::harness::codex::VersionError>>,
    native: CapabilityState,
) -> HarnessStatus {
    claude_status_in(crate::harness::claude::admission_table(), observed, native)
}

/// [`claude_status`] against an explicit admission table.
#[cfg(test)]
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
        Some(Err(VersionError::Unsupported(version))) => HarnessStatus::VersionRefused(format!(
            "claude {version}: no recipe admits it; supported recipes: {recipes}"
        )),
        Some(Err(VersionError::KnownBroken {
            version,
            range,
            newest_working,
        })) => HarnessStatus::VersionRefused(crate::harness::known_broken_label(
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
/// The `--version` the installed harness reported travels with its status:
/// poke capabilities follow the recipe of exactly that version.
impl HarnessStatus {
    /// The admission word a binary-change log line states.
    pub(crate) fn admission_word(&self) -> &'static str {
        match self {
            HarnessStatus::Unknown => "unknown",
            HarnessStatus::NotInstalled(_) => "not_found",
            HarnessStatus::PresentUnqualified { .. } => "unqualified",
            HarnessStatus::Refused(_) | HarnessStatus::VersionRefused(_) => "refused",
            HarnessStatus::Cooperative {
                live_unverified: true,
                ..
            } => crate::harness::recipe::SCHEMA_MATCHED_LABEL,
            HarnessStatus::ContractDeclared { .. } => "contract_declared",
            HarnessStatus::Cooperative { .. } => "listed",
            HarnessStatus::Supported(_) => "listed",
            HarnessStatus::Optimistic(_) => crate::harness::recipe::OPTIMISTIC_LABEL,
        }
    }

    /// An observation a later pass may reuse while the binary is unchanged:
    /// an admitted harness (a refusal or an unknown may have been a transient
    /// failure to run `--version`, so it is observed again).
    pub(crate) fn reusable(&self) -> bool {
        matches!(
            self,
            HarnessStatus::Cooperative { .. }
                | HarnessStatus::Supported(_)
                | HarnessStatus::Optimistic(_)
        )
    }
}

#[derive(Clone)]
struct ObservedBinary {
    fingerprint: Option<String>,
    binary: Option<crate::harness::BinaryIdentity>,
    observation: crate::harness::adapter::DaemonObservation,
}
fn observation_snapshot(
    rows: &Option<std::collections::BTreeMap<String, ObservedBinary>>,
) -> HarnessObservations {
    HarnessObservations {
        entries: rows
            .as_ref()
            .into_iter()
            .flat_map(|rows| rows.iter())
            .map(|(id, row)| (id.clone(), row.observation.clone()))
            .collect(),
    }
}
/// An atomic registry snapshot. Cancellation never publishes part of a pass.
pub(crate) struct AdmissionReobserver {
    registry: &'static crate::harness::registry::Registry,
    environment: crate::harness::adapter::InstallEnvironment,
    timeout: Duration,
    log: Arc<dyn Fn(&str) + Send + Sync>,
    previous: Mutex<Option<std::collections::BTreeMap<String, ObservedBinary>>>,
}
impl AdmissionReobserver {
    pub(crate) fn new(
        path: Option<std::ffi::OsString>,
        timeout: Duration,
        log: Arc<dyn Fn(&str) + Send + Sync>,
    ) -> Self {
        Self::with_registry(
            crate::harness::registry::builtins(),
            crate::harness::adapter::InstallEnvironment {
                clock: Arc::new(SystemClock::new()),
                path,
                config_root: None,
                state_dir: None,
            },
            timeout,
            log,
        )
    }
    pub(crate) fn with_registry(
        registry: &'static crate::harness::registry::Registry,
        environment: crate::harness::adapter::InstallEnvironment,
        timeout: Duration,
        log: Arc<dyn Fn(&str) + Send + Sync>,
    ) -> Self {
        Self {
            registry,
            environment,
            timeout,
            log,
            previous: Mutex::new(None),
        }
    }
    pub(crate) fn pass(&self, cancel: &Cancellation) -> HarnessObservations {
        let mut previous = self.previous.lock().unwrap_or_else(|e| e.into_inner());
        let mut next = std::collections::BTreeMap::new();
        let mut messages = Vec::new();
        for registration in self.registry.registrations() {
            if cancel.is_cancelled() {
                return observation_snapshot(&previous);
            }
            let id = registration.metadata().id;
            let fingerprint = |registration: &crate::harness::registry::Registration| {
                registration
                    .observation_fingerprint(&self.environment)
                    .filter(|s| !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control))
            };
            let before = fingerprint(registration);
            let binary = match registration.metadata().executable {
                crate::harness::adapter::ExecutableLookup::Path(name) => {
                    crate::cli::hook::resolve_on_path(name, self.environment.path.as_deref())
                        .and_then(|p| crate::harness::BinaryIdentity::observe(&p))
                }
                crate::harness::adapter::ExecutableLookup::Unsupported => None,
            };
            let old = previous.as_ref().and_then(|rows| rows.get(id));
            let reused = old.is_some_and(|old| {
                before.is_some() && before == old.fingerprint && old.observation.status.reusable()
            });
            let mut entry = if reused {
                old.expect("checked prior observation").observation.clone()
            } else {
                let budget = CallBudget {
                    deadline: crate::protocol::time::MonoInstant(
                        self.environment.clock.monotonic_now().0.saturating_add(
                            self.timeout.as_millis().try_into().unwrap_or(u64::MAX),
                        ),
                    ),
                    cancellation: cancel.clone(),
                };
                registration.observe_daemon(&self.environment, &budget)
            };
            if cancel.is_cancelled() {
                return observation_snapshot(&previous);
            }
            let after = fingerprint(registration);
            let stable = before == after;
            if !stable {
                entry = crate::harness::adapter::DaemonObservation {
                    status: HarnessStatus::Refused(
                        "observation inputs changed during the pass; retry required".into(),
                    ),
                    ..Default::default()
                };
            }
            if let Some(old) = old
                && !reused
                && old.fingerprint != before
            {
                if old.binary != binary {
                    let describe = |i: &Option<crate::harness::BinaryIdentity>| {
                        i.as_ref()
                            .map_or_else(|| "none".into(), |i| i.path().display().to_string())
                    };
                    messages.push(format!(
                        "{id} binary changed: {} → {}; admission {}",
                        describe(&old.binary),
                        describe(&binary),
                        entry.status.admission_word()
                    ));
                } else {
                    messages.push(format!(
                        "{id} observation inputs changed; admission {}",
                        entry.status.admission_word()
                    ));
                }
            }
            next.insert(
                id.into(),
                ObservedBinary {
                    fingerprint: stable.then_some(after).flatten(),
                    binary,
                    observation: entry,
                },
            );
        }
        if cancel.is_cancelled() {
            return observation_snapshot(&previous);
        }
        *previous = Some(next);
        let snapshot = observation_snapshot(&previous);
        for message in messages {
            (self.log)(&message);
        }
        snapshot
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

/// Starts the Apply lane as the elected factory does: its Pacer is registered
/// with the commit-kick registry (`journal` commits kick it) and attached to
/// `status` so Health shows its backoff.
pub(crate) fn start_apply_lane(
    kicks: &CommitKicks,
    clock: Arc<dyn Clock>,
    stop: Cancellation,
    store: Arc<SqliteStore>,
    post_commit: ApplyPostCommit,
    writer: Arc<FairWriter>,
    status: Arc<WorkerStatus>,
) -> io::Result<std::thread::JoinHandle<()>> {
    let pacer = Arc::new(Pacer::new(Lane::Apply.name(), clock, stop.clone()));
    kicks.register(Lane::Apply, Arc::clone(&pacer));
    start_apply_worker(store, post_commit, writer, pacer, stop, status)
}

/// Test-only view of one elected daemon's lanes: the store's per-origin commit
/// counts and flushed kick log, and each registered lane Pacer. It observes;
/// it changes the daemon only through its explicit test knobs (injected lane
/// faults, `relax_commit_durability`). In ordinary builds it is an empty type
/// and every method on it is a no-op.
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
    codes: Mutex<[Option<crate::protocol::results::ErrorCode>; Lane::COUNT]>,
    hits: [std::sync::atomic::AtomicU64; Lane::COUNT],
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

/// Exclusive registration for a test-only producer commit observer. Dropping
/// it unregisters the callback and waits for an in-flight callback to finish.
/// Drop outside the observer: the observer must never reenter the writer,
/// register observers, or drop its own guard.
#[cfg(any(test, feature = "test-support"))]
pub struct LaneCommitObserverGuard {
    registry: std::sync::Weak<Mutex<CommitObservers>>,
    lane: Lane,
    entry: Arc<CommitObserver>,
}

#[cfg(any(test, feature = "test-support"))]
impl Drop for LaneCommitObserverGuard {
    fn drop(&mut self) {
        // Synchronize before unregistering: a dispatcher that already cloned
        // the entry must see inactive before it can invoke the callback.
        self.entry
            .execution
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .active = false;
        if let Some(registry) = self.registry.upgrade() {
            let mut registry = registry.lock().unwrap_or_else(|e| e.into_inner());
            if registry.entries[self.lane as usize]
                .as_ref()
                .is_some_and(|entry| Arc::ptr_eq(entry, &self.entry))
            {
                registry.entries[self.lane as usize] = None;
            }
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
struct CommitObservers {
    entries: [Option<Arc<CommitObserver>>; Lane::COUNT],
}

#[cfg(any(test, feature = "test-support"))]
struct CommitObserver {
    execution: Mutex<CommitObserverExecution>,
    /// Legacy watchers own only their returned slot. Once captured or dropped,
    /// that slot must not prevent a subsequent registration.
    one_shot: Option<std::sync::Weak<Mutex<Option<Instant>>>>,
}

#[cfg(any(test, feature = "test-support"))]
struct CommitObserverExecution {
    active: bool,
    seen: u64,
    callback: Box<dyn Fn(Instant) + Send + Sync>,
}

#[cfg(any(test, feature = "test-support"))]
impl CommitObserver {
    fn legacy_finished(&self) -> bool {
        self.one_shot.as_ref().is_some_and(|slot| {
            slot.upgrade()
                .is_none_or(|slot| slot.lock().unwrap_or_else(|e| e.into_inner()).is_some())
        })
    }
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
struct ProbeState {
    store: Option<Arc<SqliteStore>>,
    commit_observers: Arc<Mutex<CommitObservers>>,
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
    /// The daemon's multi-host registry; the host lanes' Pacers and the
    /// reachability the observation lane feeds the wake lane live in its sessions.
    sessions: Option<std::sync::Weak<HostSessions>>,
    /// The sweep period the daemon is composed with (default: production's).
    host_sweep_interval: Option<Duration>,
    /// The store commits without fsync (see [`LaneProbe::relax_commit_durability`]).
    relaxed_durability: bool,
}

#[cfg(any(test, feature = "test-support"))]
impl LaneProbe {
    fn state(&self) -> std::sync::MutexGuard<'_, ProbeState> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn attach_store(&self, store: &Arc<SqliteStore>) {
        let mut state = self.state();
        assert!(
            state
                .commit_observers
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .entries
                .iter()
                .all(|entry| entry.as_ref().is_none_or(|entry| entry.legacy_finished())),
            "cannot reattach a store while commit observers are active"
        );
        // Each store gets its own registry. An old store's weak dispatcher
        // can never reach registrations made after this attachment.
        let observers = Arc::new(Mutex::new(CommitObservers::default()));
        state.commit_observers = Arc::clone(&observers);
        let log = Arc::clone(&state.kicks);
        store.set_kick_sink(Box::new(move |lanes, origin| {
            log.lock()
                .unwrap_or_else(|e| e.into_inner())
                .push((lanes, origin, Instant::now()));
        }));
        let faults = Arc::clone(&state.faults);
        store.set_lane_fault(Some(Arc::new(move |origin| faults.fire(origin?))));
        let weak_observers = Arc::downgrade(&observers);
        let weak_store = Arc::downgrade(store);
        // Install one dispatcher for this probe/store. Registrations never
        // replace the store's pause hook or another origin's observer.
        store.set_kick_pause(Box::new(move || {
            let at = Instant::now();
            let Some(lane) = crate::service::kicks::current_origin() else {
                return;
            };
            let (Some(store), Some(observers)) = (weak_store.upgrade(), weak_observers.upgrade())
            else {
                return;
            };
            let count = store.commit_counts().get(lane.name()).copied().unwrap_or(0);
            let entry =
                observers.lock().unwrap_or_else(|e| e.into_inner()).entries[lane as usize].clone();
            let Some(entry) = entry else { return };
            {
                let mut execution = entry.execution.lock().unwrap_or_else(|e| e.into_inner());
                if !execution.active || count <= execution.seen {
                    return;
                }
                execution.seen = count;
                (execution.callback)(at);
                if entry.one_shot.is_none() {
                    return;
                }
                execution.active = false;
            }
            let mut observers = observers.lock().unwrap_or_else(|e| e.into_inner());
            if observers.entries[lane as usize]
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &entry))
            {
                observers.entries[lane as usize] = None;
            }
        }));
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
    fn attach_host_sessions(&self, sessions: &Arc<HostSessions>) {
        self.state().sessions = Some(Arc::downgrade(sessions));
    }
    /// The period of the daemon's host sweep (set by
    /// [`LaneProbe::set_host_sweep_interval`]); call the setter before the
    /// daemon starts.
    fn host_sweep_interval(&self) -> Duration {
        self.state()
            .host_sweep_interval
            .unwrap_or(crate::service::host_sessions::DEFAULT_SWEEP_INTERVAL)
    }
    /// Shortens the host sweep so a test sees a socket that appears after
    /// start attached within milliseconds. Call before the daemon starts.
    pub fn set_host_sweep_interval(&self, interval: Duration) {
        self.state().host_sweep_interval = Some(interval);
    }
    /// The session of the attached server `locator`, once the daemon attached it.
    fn session_of(&self, locator: &crate::protocol::ids::HostLocator) -> Option<Arc<HostSession>> {
        self.host_sessions()?.by_locator(locator)
    }
    /// The host-lane Pacer of the server `locator`: its observation or wake
    /// worker's. `None` for any other lane, or while the server is unattached.
    pub fn host_pacer(
        &self,
        locator: &crate::protocol::ids::HostLocator,
        lane: Lane,
    ) -> Option<Arc<Pacer>> {
        let session = self.session_of(locator)?;
        match lane {
            Lane::Observation => Some(Arc::clone(session.observation().pacer())),
            Lane::Wakes => Some(Arc::clone(session.wake().pacer())),
            _ => None,
        }
    }
    /// The first attached server's Pacer for a host lane.
    fn first_host_pacer(&self, lane: Lane) -> Option<Arc<Pacer>> {
        let session = self.host_sessions()?.attached().into_iter().next()?;
        match lane {
            Lane::Observation => Some(Arc::clone(session.observation().pacer())),
            Lane::Wakes => Some(Arc::clone(session.wake().pacer())),
            _ => None,
        }
    }
    /// The daemon's host registry while the daemon lives (the probe holds it
    /// weakly, so a stopped daemon's hosts are released).
    pub fn host_sessions(&self) -> Option<Arc<HostSessions>> {
        self.state().sessions.as_ref()?.upgrade()
    }
    /// Whether the first attached server's observation lane has it marked
    /// down (its wake lane is frozen); false before the daemon is attached.
    pub fn host_down(&self) -> bool {
        self.host_sessions()
            .and_then(|sessions| sessions.attached().into_iter().next())
            .is_some_and(|session| session.reachability().is_down())
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
    /// Observe changed commits from one lane on its producer after writer
    /// release, before the kick log and host handling. `at` precedes the
    /// callback's bounded read-only work; it is not an exact SQLite timestamp.
    /// The daemon has one producer per lane. No-op and other-origin turns are
    /// ignored. An occupied lane returns AlreadyExists without replacement.
    /// The callback must not reenter the writer/register/drop its own guard.
    pub fn observe_lane_commits(
        &self,
        lane: Lane,
        callback: Box<dyn Fn(Instant) + Send + Sync>,
    ) -> io::Result<LaneCommitObserverGuard> {
        let (registry, entry) = self.register_commit_observer(lane, callback, None)?;
        Ok(LaneCommitObserverGuard {
            registry: Arc::downgrade(&registry),
            lane,
            entry,
        })
    }

    fn register_commit_observer(
        &self,
        lane: Lane,
        callback: Box<dyn Fn(Instant) + Send + Sync>,
        one_shot: Option<std::sync::Weak<Mutex<Option<Instant>>>>,
    ) -> io::Result<(Arc<Mutex<CommitObservers>>, Arc<CommitObserver>)> {
        let state = self.state();
        let store = state.store.as_ref().expect("attached store");
        let registry = Arc::clone(&state.commit_observers);
        let baseline = store.commit_counts().get(lane.name()).copied().unwrap_or(0);
        let mut observers = registry.lock().unwrap_or_else(|e| e.into_inner());
        if observers.entries[lane as usize]
            .as_ref()
            .is_some_and(|entry| !entry.legacy_finished())
        {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "lane commit observer already registered",
            ));
        }
        let entry = Arc::new(CommitObserver {
            execution: Mutex::new(CommitObserverExecution {
                active: true,
                seen: baseline,
                callback,
            }),
            one_shot,
        });
        observers.entries[lane as usize] = Some(Arc::clone(&entry));
        drop(observers);
        drop(state);
        Ok((registry, entry))
    }

    /// Observe the next row-changing commit from `lane` on its writer thread,
    /// after guard release. Observer scheduling must not alter this instant.
    /// Shares exclusive per-origin ownership with observe_lane_commits; an
    /// active conflict panics without replacing the existing observer. A
    /// captured or dropped result slot releases this one-shot registration.
    pub fn next_commit_instant(&self, lane: Lane) -> Arc<Mutex<Option<Instant>>> {
        let instant = Arc::new(Mutex::new(None));
        let observed = Arc::downgrade(&instant);
        self.register_commit_observer(
            lane,
            Box::new(move |at| {
                if let Some(slot) = observed.upgrade() {
                    slot.lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .get_or_insert(at);
                }
            }),
            Some(Arc::downgrade(&instant)),
        )
        .expect("exclusive lane commit observer");
        instant
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
        let registered = self
            .state()
            .pacers
            .iter()
            .rev()
            .find(|(candidate, _)| *candidate == lane)
            .map(|(_, pacer)| Arc::clone(pacer));
        let pacer = registered.or_else(|| self.first_host_pacer(lane));
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
    /// Observe a completed pass before the registered worker evaluates its
    /// next wait. The callback must not reenter or replace its own idle hook.
    pub fn set_registered_idle_hook(&self, lane: Lane, hook: Box<dyn Fn(u64) + Send + Sync>) {
        let registry = self.state().registry.clone();
        let pacer = registry
            .and_then(|kicks| kicks.pacer(lane))
            .expect("registered lane");
        pacer.set_idle_hook(hook);
    }
    /// Observe each pass start (a wait ending) of the registered worker, with
    /// the reason the wait ended. The callback must not reenter the Pacer.
    pub fn set_registered_wake_hook(
        &self,
        lane: Lane,
        hook: Box<dyn Fn(crate::service::pacer::Wake) + Send + Sync>,
    ) {
        let registry = self.state().registry.clone();
        let pacer = registry
            .and_then(|kicks| kicks.pacer(lane))
            .expect("registered lane");
        pacer.set_wake_hook(hook);
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
    /// Latency fixtures: the daemon's store commits with `synchronous=NORMAL`,
    /// so a commit-to-wake budget measures the lane chain rather than fsync
    /// time on a disk shared with the rest of the test suite. Production
    /// always commits with `FULL`. Call before the daemon starts.
    pub fn relax_commit_durability(&self) {
        self.state().relaxed_durability = true;
    }
    fn configure_store(&self, context: &StoreContext) {
        if self.state().relaxed_durability {
            context.relax_commit_durability();
        }
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
        let registered = self
            .state()
            .pacers
            .iter()
            .find(|(candidate, _)| *candidate == lane)
            .map(|(_, pacer)| Arc::clone(pacer));
        registered
            .or_else(|| self.first_host_pacer(lane))
            .map_or(0, |pacer| pacer.idle_events())
    }
}

#[cfg(not(any(test, feature = "test-support")))]
impl LaneProbe {
    fn configure_store(&self, _: &StoreContext) {}
    fn attach_store(&self, _: &Arc<SqliteStore>) {}
    fn attach_registry(&self, _: &Arc<CommitKicks>, _: Vec<Arc<WorkerStatus>>) {}
    fn attach_pacer(&self, _: Lane, _: &Arc<Pacer>) {}
    fn attach_host_sessions(&self, _: &Arc<HostSessions>) {}
    fn host_sweep_interval(&self) -> Duration {
        crate::service::host_sessions::DEFAULT_SWEEP_INTERVAL
    }
    fn lane_log(&self, clock: Arc<dyn Clock>) -> crate::daemon::logs::RateLimitedLaneLog {
        crate::daemon::logs::RateLimitedLaneLog::to_stderr(clock)
    }
}

/// The lifecycle calls the factory only after ownership election and socket
/// bind. The matching instance and boot are injected into SQLite and control.
pub async fn run_elected<R>(
    paths: &NodePaths,
    clock: Arc<dyn Clock>,
    shutdown: Cancellation,
    config: ServiceConfig,
    plan: HostAttachPlan,
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
        plan,
        None,
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
    paths: &NodePaths,
    clock: Arc<dyn Clock>,
    shutdown: Cancellation,
    config: ServiceConfig,
    plan: HostAttachPlan,
    probe: LaneProbe,
    on_ready: R,
) -> io::Result<bool>
where
    R: FnOnce(&EndpointDescriptor) -> io::Result<()>,
{
    run_elected_impl(paths, clock, shutdown, config, plan, None, probe, on_ready).await
}

/// Production native composition: `plan.factory` builds each attached host,
/// and a host with `HostGuardedStart` retains its required scoped observer for
/// guarded bootstrap.
pub async fn run_elected_guarded<R>(
    paths: &NodePaths,
    context: &crate::daemon::paths::RuntimeContext,
    clock: Arc<dyn Clock>,
    shutdown: Cancellation,
    config: ServiceConfig,
    plan: HostAttachPlan,
    on_ready: R,
) -> io::Result<bool>
where
    R: FnOnce(&EndpointDescriptor) -> io::Result<()>,
{
    let selected_paths = NodePaths::resolve_read_only(&context.state_dir)?;
    if selected_paths.node_dir.as_os_str() != paths.node_dir.as_os_str() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "guarded runtime paths differ from selected context",
        ));
    }
    run_elected_impl(
        paths,
        clock,
        shutdown,
        config,
        plan,
        Some(context.state_dir.clone()),
        LaneProbe::default(),
        on_ready,
    )
    .await
}

/// The longest `host_label` the journal records, in bytes.
const HOST_LABEL_MAX_BYTES: usize = 255;

/// This machine's hostname, lossy UTF-8, for the genesis `host_label`.
fn host_label() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: the buffer is valid for `buf.len()` bytes; gethostname writes at
    // most that many and may omit the terminator when it truncates.
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
    if rc != 0 {
        return String::new();
    }
    let len = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
    truncate_label(&String::from_utf8_lossy(&buf[..len]))
}

/// `text` cut to at most [`HOST_LABEL_MAX_BYTES`] bytes at a char boundary.
fn truncate_label(text: &str) -> String {
    let mut end = text.len().min(HOST_LABEL_MAX_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

/// Load or create the node key and publish the `node.json` hint. A refused
/// key is a one-line start failure naming the `NodeKeyError` variant.
fn load_node_identity(paths: &NodePaths) -> io::Result<NodeIdentity> {
    paths.prepare_node_dir()?;
    let identity =
        NodeIdentity::load_or_create(paths, paths.database_path.exists()).map_err(|error| {
            let kind = match error {
                NodeKeyError::Unreadable { .. } => io::ErrorKind::Other,
                _ => io::ErrorKind::InvalidData,
            };
            io::Error::new(
                kind,
                format!("node key refused ({}): {error}", error.variant()),
            )
        })?;
    publish_node_json(paths, &identity)?;
    Ok(identity)
}

/// Writes `node.json` from the identity's current tag and `tag_k`, keeping the
/// recorded `created_at` of an earlier hint for the same node.
fn publish_node_json(paths: &NodePaths, identity: &NodeIdentity) -> io::Result<()> {
    crate::node::json::publish_identity(&paths.node_json_path, identity).map(|_| ())
}

/// The start check of the node's identity against its store (Daemon start
/// order step 3): the store's `local_node.id` must be the key's uuid, else the
/// start is refused with `NodeKeyError::IdentityMismatch`; the stored `tag_k`
/// becomes the identity's.
fn check_store_identity(
    conn: &rusqlite::Connection,
    identity: &NodeIdentity,
    key_path: &std::path::Path,
) -> io::Result<()> {
    // A database without a node table or row (a file the store has not
    // initialized yet) is not this store's to judge: opening it creates or
    // reports it. Any other failure (busy, corrupt) fails the start.
    let (stored, tag_k): (String, u32) =
        match conn.query_row("SELECT id, tag_k FROM local_node", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        }) {
            Ok(found) => found,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(()),
            Err(rusqlite::Error::SqliteFailure(_, Some(message)))
                if message.starts_with("no such table: local_node") =>
            {
                return Ok(());
            }
            Err(error) => {
                return Err(io::Error::other(format!(
                    "store startup: identity check: {error}"
                )));
            }
        };
    let stored_uuid: crate::node::NodeUuid = stored.parse().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("store startup: local_node id {stored:?} is not a node uuid"),
        )
    })?;
    if stored_uuid != identity.uuid() {
        let error = NodeKeyError::IdentityMismatch {
            path: key_path.to_path_buf(),
            store: stored_uuid,
            key: identity.uuid(),
        };
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("node key refused ({}): {error}", error.variant()),
        ));
    }
    identity.set_tag_k(tag_k);
    Ok(())
}

/// The daemon's registry and the links that stop it with the service.
struct ComposedSessions {
    sessions: Arc<HostSessions>,
    _links: Vec<crate::protocol::time::CancelSubscription>,
}

/// The running coordinator: its facade (idle exit and the deadline lane's
/// `signals()` reach it here) and the actor task the stop path awaits.
struct ComposedSync {
    handle: crate::sync::coordinator::SyncHandle,
    join: tokio::task::JoinHandle<()>,
}

/// How long shutdown waits for the coordinator to close its links.
const SYNC_STOP_BOUND: Duration = Duration::from_secs(3);

#[allow(clippy::too_many_arguments)]
async fn run_elected_impl<R>(
    paths: &NodePaths,
    clock: Arc<dyn Clock>,
    shutdown: Cancellation,
    config: ServiceConfig,
    // The startup hint and how hosts are built (spec §3 Startup).
    plan: HostAttachPlan,
    // Set by `run_elected_guarded`: the state dir of the guarded handoff
    // namespace, used when the attached host has `HostGuardedStart`.
    guarded_state_dir: Option<std::path::PathBuf>,
    probe: LaneProbe,
    on_ready: R,
) -> io::Result<bool>
where
    R: FnOnce(&EndpointDescriptor) -> io::Result<()>,
{
    // The node's identity comes from its key, loaded (or created for a new
    // node) before election; the descriptor and the store `instance` string
    // carry its uuid. node.json is a derived hint, written here so clients
    // can find the daemon; it is rewritten after the store's identity check.
    let identity = Arc::new(load_node_identity(paths)?);
    // Instance settings and the offline switch are read once, here: the
    // manifest fetch policy is fixed for this daemon's lifetime. An invalid
    // settings file refuses the start with a message naming the file.
    let instance_settings = crate::daemon::settings::load(&paths.node_dir)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    let manifest_policy = crate::harness::manifest::policy_from(
        &instance_settings,
        std::env::var_os("HERDR_THREADS_OFFLINE").as_deref(),
    );
    let manifest_cache_dir = crate::harness::manifest::cache_dir(&paths.node_dir);
    let mod_delivery = instance_settings.mod_delivery;
    let database_path = paths.database_path.clone();
    let archival_paths = paths.clone();
    let identity_paths = paths.clone();
    let identity_db_path = paths.database_path.clone();
    let identity_key_path = paths.key_path.clone();
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
    let factory_apply_status = Arc::new(WorkerStatus::default());
    let factory_archival_status = Arc::new(WorkerStatus::default());
    let factory_sync_status = Arc::new(WorkerStatus::default());
    let admission_status = Arc::new(WorkerStatus::default());
    let factory_admission_status = Arc::clone(&admission_status);
    // One rate-limited logger for the daemon; every lane's status reports to it.
    let rate_limited_log = Arc::new(probe.lane_log(Arc::clone(&clock)));
    let lane_log: Arc<dyn crate::daemon::logs::LaneErrorLog> = rate_limited_log.clone();
    let factory_lane_log = Arc::clone(&lane_log);
    let HostAttachPlan {
        hint: factory_hint,
        factory: mut host_factory,
    } = plan;
    // The startup hint is the operator's explicit choice: it attaches whether
    // or not its socket exists yet (the session then reports the server down
    // and the observation lane reconnects), as the single-host daemon always
    // did. Every other server attaches only once its socket exists.
    if let Some(hint) = factory_hint.clone() {
        let exists = Arc::clone(&host_factory.exists);
        host_factory.exists =
            Arc::new(move |path: &std::path::Path| path == hint.as_path() || exists(path));
    }
    // Filled by the factory; the stop path takes it to stop every session.
    let sessions_slot: Arc<Mutex<Option<ComposedSessions>>> = Arc::new(Mutex::new(None));
    // The sync coordinator: filled by the factory, awaited by the stop path.
    // The intake and gauge reach the accept loop through `peers`.
    let sync_slot: Arc<Mutex<Option<ComposedSync>>> = Arc::new(Mutex::new(None));
    let factory_sync_slot = Arc::clone(&sync_slot);
    let peers = crate::daemon::transport::PeerWiring::default();
    let factory_peers = peers.clone();
    let factory_sessions = Arc::clone(&sessions_slot);
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
        &factory_archival_status,
        &admission_status,
        &factory_apply_status,
        &factory_sync_status,
    ] {
        status.set_error_log(Arc::clone(&lane_log));
    }
    let host_evidence = Arc::new(HostEvidenceStatus::default());
    let factory_host_evidence = Arc::clone(&host_evidence);
    let factory_probe = probe;
    // Filled by the factory once the host is attached (after the store
    // opens); read when the descriptor is published.
    let primary_host = crate::daemon::ownership::PrimaryHost::default();
    let factory_primary_host = primary_host.clone();
    run_elected_late_host(
        paths,
        primary_host,
        peers,
        clock,
        shutdown,
        move |instance, boot, cancellation| {
            if instance != identity.uuid() {
                return Err(io::Error::other(
                    "node.json hint does not match the loaded node key",
                ));
            }
            // Created before the context; HostSessions will publish into it.
            let host_index = Arc::new(crate::store::hosts::HostIndex::new());
            let context = StoreContext::with_host_index(
                database_path,
                Arc::clone(&factory_clock),
                Arc::clone(&host_index),
            )
            .with_node_seed(crate::store::schema::NodeSeed {
                uuid: identity.uuid(),
                tag: crate::node::tag_for(identity.uuid(), 0),
                tag_k: 0,
            });
            // Daemon start order step 3-4: an existing store's node row must
            // belong to the key (checked before anything is written to it, and
            // on a read-only connection so the writer's startup evidence
            // backfill stays with the store's own open); adopt its tag_k and
            // regenerate node.json with it. A new store is created from the
            // identity itself.
            if identity_db_path.exists() {
                let conn = rusqlite::Connection::open_with_flags(
                    &identity_db_path,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                )
                .map_err(|error| io::Error::other(format!("store startup: {error}")))?;
                conn.busy_timeout(std::time::Duration::from_secs(5))
                    .map_err(|error| io::Error::other(format!("store startup: {error}")))?;
                check_store_identity(&conn, &identity, &identity_key_path)?;
            }
            publish_node_json(&identity_paths, &identity)?;
            // Daemon start order: the journal opens once, after the node.json
            // step and before any host attaches. A refused open (a violated
            // journal invariant) refuses the start.
            {
                let mut conn = context.open_writer().map_err(|error| {
                    io::Error::other(format!(
                        "store startup: journal open: {:?}: {}",
                        error.code, error.detail
                    ))
                })?;
                context.pin_binding_evidence();
                let report = crate::store::journal::open(
                    &context,
                    &mut conn,
                    Arc::clone(&identity),
                    &host_label(),
                    crate::sync::facts::FactRegistry::builtins(),
                )
                .map_err(|error| {
                    io::Error::other(format!(
                        "store startup: journal open: {:?}: {}",
                        error.code, error.detail
                    ))
                })?;
                // Once per start: a local upgrade resets targets stopped by
                // an older version (a foreground writer turn).
                {
                    let sync_error = |error: crate::protocol::results::ApiError| {
                        io::Error::other(format!(
                            "store startup: sync dial targets: {:?}: {}",
                            error.code, error.detail
                        ))
                    };
                    let tx = conn
                        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                        .map_err(|error| io::Error::other(format!("store startup: {error}")))?;
                    crate::store::sync_dial_targets::clear_stale_stops(
                        &tx,
                        env!("CARGO_PKG_VERSION"),
                    )
                    .map_err(sync_error)?;
                    tx.commit()
                        .map_err(|error| io::Error::other(format!("store startup: {error}")))?;
                }
                // A crash between a retag's commit and its node.json
                // regeneration left the identity behind the committed tag.
                let budget = crate::protocol::time::CallBudget {
                    deadline: crate::protocol::time::MonoInstant(
                        context.clock().monotonic_now().0.saturating_add(5_000),
                    ),
                    cancellation: crate::protocol::time::Cancellation::default(),
                };
                crate::sync::facts::publish_committed_tag(
                    &context,
                    &identity,
                    &identity_paths.node_json_path,
                    &budget,
                )
                .map_err(|error| {
                    io::Error::other(format!("store startup: publish committed tag: {error}"))
                })?;
                rate_limited_log.write_line(&format!(
                    "journal open: log {:?} counter {} bootstrapped {} parked_unknown {} parked_dep {} failed {}",
                    report.self_log,
                    report.counter,
                    report.bootstrapped,
                    report.parked_unknown,
                    report.parked_dep,
                    report.failed
                ));
            }
            factory_probe.configure_store(&context);
            // One registry per elected daemon: the store's commit hooks kick
            // through it and every lane registers its Pacer here at start.
            let kicks = Arc::new(CommitKicks::default());
            // The one background-turn admission: the store's sync-table writes
            // wait on it like every other lane's.
            let writer = Arc::new(FairWriter::new(32));
            let sqlite = Arc::new(
                SqliteStore::new(context, instance.to_string(), config.store_settings(boot))
                    .map_err(|error| {
                        io::Error::other(format!(
                            "store startup: {:?}: {}",
                            error.code, error.detail
                        ))
                    })?
                    .with_commit_kicks(Arc::clone(&kicks))
                    .with_sync_admission(Arc::clone(&writer)),
            );
            factory_probe.attach_store(&sqlite);
            let remembered_store = Arc::clone(&sqlite);
            let bootstrap_store = sqlite.clone();
            let apply_store = sqlite.clone();
            let apply_identity = Arc::clone(&identity);
            let sync_identity = Arc::clone(&identity);
            let sync_store = sqlite.clone();
            let apply_node_json = identity_paths.node_json_path.clone();
            let store: Arc<dyn StorePort> = sqlite;
            // The manifest policy, logged after election (stderr is the daemon
            // log from here on).
            rate_limited_log.write_line(&format!(
                "harness manifest: {}",
                match manifest_policy {
                    crate::harness::manifest::ManifestPolicy::Auto => "auto",
                    crate::harness::manifest::ManifestPolicy::Off(
                        crate::harness::manifest::OffReason::Settings,
                    ) => "off (settings.json)",
                    crate::harness::manifest::ManifestPolicy::Off(
                        crate::harness::manifest::OffReason::OfflineEnv,
                    ) => "off (HERDR_THREADS_OFFLINE=1)",
                }
            ));
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
            // The admission observer fills this slot; the wake lane reads poke
            // capabilities from the versions it observed.
            let harnesses = Arc::new(Mutex::new(HarnessObservations::default()));
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
            // HostSessions hands its own worker handles to this slot, so the
            // lock is held only for a push.
            let push_worker = |handle: std::thread::JoinHandle<()>| -> io::Result<()> {
                factory_slot
                    .lock()
                    .map_err(|_| io::Error::other("worker slot poisoned"))?
                    .push(handle);
                Ok(())
            };
            push_worker(start_deadline_worker(
                Arc::clone(&store),
                Arc::clone(&writer),
                register_lane(Lane::Deadlines),
                factory_stop.clone(),
                Arc::clone(&factory_status),
            )?)?;
            // The one registry of live mod delivery channels (spec D2, D7),
            // shared by the wake lane (routing), the domain service (digest
            // flag and watch registration) and setup-status. It is the sole
            // owner of channel liveness and of the disconnect kick of the
            // wake lane; commits touching an attention source wake its worker.
            let mod_registry = Arc::new(
                ModChannelRegistry::new(
                    Arc::clone(&factory_clock),
                    Arc::clone(&store) as Arc<dyn crate::ports::ModStoreReads>,
                    instance.to_string(),
                    {
                        let kicks = Arc::clone(&kicks);
                        Arc::new(move || {
                            kicks.kick(crate::service::kicks::LaneSet::EMPTY.with(Lane::Wakes))
                        })
                    },
                    mod_delivery,
                )
                .with_connection_cap(config.watch_connection_cap())
                .with_log({
                    let log = Arc::clone(&rate_limited_log);
                    Arc::new(move |line: &str| log.write_line(line))
                }),
            );
            kicks.set_mod_observer(mod_registry.observer());
            let mod_channels: Arc<dyn crate::ports::ModChannels> = mod_registry.clone();
            let delivery_pull =
                Arc::new(crate::service::delivery_pull::DeliveryPullState::default());
            // Binding liveness (spec §3, decision 9): the host process table backs
            // both the periodic re-check thread and the re-check before each
            // wake decision of every session's wake worker.
            let liveness = Arc::new(crate::service::liveness::LivenessWorker::new(
                Arc::clone(&store) as Arc<dyn crate::ports::StorePort>,
                Arc::clone(&writer),
                Arc::new(crate::host::process_table::HostProcessTable),
            ));
            // The multi-host registry (spec §2, §3): built after the journal
            // opened and before the listener serves. Startup attaches the plan's
            // hint, then every remembered server whose socket exists, and starts
            // the sweep; a failure is one log line and never fatal.
            // Sessions stop when the service shuts down (their identities'
            // detached compensation never holds the owner's shutdown) and when
            // the daemon drains.
            let session_stop = Cancellation::default();
            let links = vec![
                cancellation.on_cancel({
                    let session_stop = session_stop.clone();
                    Arc::new(move || session_stop.cancel())
                }),
                factory_stop.on_cancel({
                    let session_stop = session_stop.clone();
                    Arc::new(move || session_stop.cancel())
                }),
            ];
            let host_sessions = Arc::new(HostSessions::new(
                Arc::clone(&host_index),
                host_factory,
                SessionDeps {
                    store: Arc::clone(&store),
                    writer: Arc::clone(&writer),
                    kicks: Arc::clone(&kicks),
                    clock: Arc::clone(&factory_clock),
                    node_uuid: instance.to_string(),
                    boot,
                    state_dir: guarded_state_dir.clone(),
                    retry: config.retry_config(),
                    liveness: Some(Arc::clone(&liveness)),
                    mod_channels: Arc::clone(&mod_channels),
                    poke_capabilities: Arc::new(ObservedPokeCapabilities::new(Arc::clone(
                        &harnesses,
                    ))),
                    hook_pull: Arc::clone(&delivery_pull)
                        as Arc<dyn crate::ports::HookPullOutlook>,
                    worker_slot: Arc::clone(&factory_slot),
                    stop: session_stop,
                    log: {
                        let log = Arc::clone(&rate_limited_log);
                        Arc::new(move |line: &str| log.write_line(line))
                    },
                    lane_log: Some(Arc::clone(&factory_lane_log)),
                    remembered: Arc::new(move |budget: &CallBudget| {
                        let db = remembered_store.context().open_query(budget.clone())?;
                        crate::store::hosts::remembered(&db, usize::MAX)
                    }),
                    sweep_interval: factory_probe.host_sweep_interval(),
                    // Production attaches only a locator that is its own
                    // canonical path on disk; fake locators are not on disk.
                    check_canonical: cfg!(not(any(test, feature = "test-support"))),
                },
            ));
            *factory_sessions
                .lock()
                .map_err(|_| io::Error::other("host sessions slot poisoned"))? =
                Some(ComposedSessions {
                    sessions: Arc::clone(&host_sessions),
                    _links: links,
                });
            factory_probe.attach_host_sessions(&host_sessions);
            host_sessions.attach_startup(factory_hint.as_ref())?;
            // Interim until ht-5fr.2.12 routes by claim host: the service,
            // Health and archival follow the startup session (`None` when no
            // server is attached at startup: store-only).
            let primary = host_sessions.startup_primary(factory_hint.as_ref());
            factory_primary_host
                .set(primary.as_ref().map(|session| std::path::PathBuf::from(session.locator())));
            rate_limited_log.write_line(&match &primary {
                Some(session) => format!(
                    "herdr host attached: {} (host {})",
                    session.locator().as_str(),
                    session.host_id().get()
                ),
                None => "no Herdr server attached: store-only".to_owned(),
            });
            if primary.is_some() {
                push_worker(crate::service::liveness::start_liveness_worker(
                    liveness,
                    Arc::new(Pacer::new(
                        "liveness",
                        Arc::clone(&factory_clock),
                        factory_stop.clone(),
                    )),
                    factory_stop.clone(),
                    {
                        let log = Arc::clone(&rate_limited_log);
                        Arc::new(move |line: &str| log.write_line(line))
                    },
                )?)?;
            }
            push_worker(start_mod_channel_worker(
                Arc::clone(&mod_registry),
                Arc::new(Pacer::new(
                    "mod-channels",
                    Arc::clone(&factory_clock),
                    factory_stop.clone(),
                )),
                Arc::clone(&factory_clock),
                factory_stop.clone(),
                {
                    let log = Arc::clone(&rate_limited_log);
                    Arc::new(move |line: &str| log.write_line(line))
                },
            )?)?;
            // The identity resolver is the service's only way to the host
            // (`DomainService::attached_host`); a store-only daemon has none.
            let identity = primary.as_ref().map(|session| Arc::clone(session.identity()));
            push_worker(start_retention_lane(
                &kicks,
                Arc::clone(&factory_clock),
                factory_stop.clone(),
                Arc::clone(&store),
                Arc::clone(&writer),
                Arc::clone(&factory_retention_status),
            )?)?;
            // The Apply lane applies ingested journal entries. Its post-commit
            // hook brings the identity cache and node.json up to a tag the
            // pass moved (the node kinds own the effect).
            let post_commit: ApplyPostCommit = {
                let store = Arc::clone(&apply_store);
                let identity = Arc::clone(&apply_identity);
                let node_json = apply_node_json.clone();
                let clock = Arc::clone(&factory_clock);
                let log = Arc::clone(&rate_limited_log);
                Box::new(move |progress| {
                    if progress.self_tag_k.is_none() {
                        return;
                    }
                    let budget = CallBudget {
                        deadline: MonoInstant(clock.monotonic_now().0.saturating_add(5_000)),
                        cancellation: Cancellation::default(),
                    };
                    if let Err(error) = crate::sync::facts::publish_committed_tag(
                        store.context(),
                        &identity,
                        &node_json,
                        &budget,
                    ) {
                        log.write_line(&format!("apply: publish committed tag: {error}"));
                    }
                })
            };
            push_worker(start_apply_lane(
                &kicks,
                Arc::clone(&factory_clock),
                factory_stop.clone(),
                apply_store,
                post_commit,
                Arc::clone(&writer),
                Arc::clone(&factory_apply_status),
            )?)?;
            // The sync runtime (peer-link spec §15): after the journal, the
            // host sessions and the Apply lane, before the listener serves.
            // The coordinator builds its Dialer; the intake goes to the
            // accept loop through the wiring.
            let sync_pacer = register_lane(Lane::Sync);
            factory_sync_status.attach_pacer(Arc::clone(&sync_pacer));
            let sync_log: crate::sync::coordinator::LogFn = {
                let log = Arc::clone(&rate_limited_log);
                Arc::new(move |line: &str| log.write_line(line))
            };
            let own_history = Arc::clone(
                sync_store
                    .context()
                    .sync()
                    .map_err(|error| {
                        io::Error::other(format!("store startup: sync state: {}", error.detail))
                    })?
                    .own_history_hold(),
            );
            let (sync_handle, peer_intake, sync_join) = crate::sync::coordinator::SyncCoordinator::start(
                crate::sync::coordinator::CoordinatorDeps {
                    journal: sync_store.clone(),
                    sync_store,
                    identity: sync_identity,
                    boot,
                    writer: Arc::clone(&writer),
                    clock: Arc::clone(&factory_clock),
                    pacer: sync_pacer,
                    status: Arc::clone(&factory_sync_status),
                    log: sync_log.clone(),
                    fork_sink: Arc::new(crate::sync::coordinator::LogOnlyForkSink::new(
                        sync_log,
                        Arc::clone(&factory_clock),
                    )),
                    own_history,
                    dial: crate::sync::dialer::DialConfig::from(&config),
                    cancel: factory_stop.clone(),
                },
            );
            factory_peers.set_intake(peer_intake);
            // Idle exit (spec §15): a store-only daemon with no host, link,
            // target or client stops through the same shutdown as `Stop`.
            {
                let idle_stop = Cancellation::default();
                let subscriptions = [&cancellation, &factory_stop].map(|token| {
                    let idle_stop = idle_stop.clone();
                    token.on_cancel(Arc::new(move || idle_stop.cancel()))
                });
                let idle = crate::daemon::idle_exit::IdleExit::new(
                    crate::daemon::idle_exit::ActivityProbe {
                        hosts: Arc::clone(&host_sessions),
                        sync: sync_handle.clone(),
                        gauge: factory_peers.gauge(),
                    },
                    Arc::clone(&factory_clock),
                    config.idle_exit_after(),
                    cancellation.clone(),
                    {
                        let log = Arc::clone(&rate_limited_log);
                        Arc::new(move |line: &str| log.write_line(line))
                    },
                );
                std::thread::Builder::new()
                    .name("idle-exit".into())
                    .spawn(move || {
                        idle.run(&idle_stop);
                        drop(subscriptions);
                    })
                    .map_err(|error| io::Error::other(format!("idle exit monitor: {error}")))?;
            }
            *factory_sync_slot
                .lock()
                .map_err(|_| io::Error::other("sync slot poisoned"))? = Some(ComposedSync {
                handle: sync_handle,
                join: sync_join,
            });
            // The harness version manifest: cache, embedded fallback and the
            // detached fetch policy. One log line per fetch outcome (at most
            // one fetch per harness per day, so no extra rate limit).
            let manifest = Arc::new(crate::harness::manifest::ManifestService::new(
                manifest_cache_dir.clone(),
                manifest_policy,
                Arc::new(crate::harness::manifest::CurlFetcher::new(
                    manifest_cache_dir,
                )),
                Arc::clone(&factory_clock),
                {
                    let log = Arc::clone(&rate_limited_log);
                    Arc::new(move |line: &str| log.write_line(line))
                },
            ));
            let states_harnesses = Arc::clone(&harnesses);
            let observer_manifest = Arc::clone(&manifest);
            let observer_store = Arc::clone(&store);
            let reobserver = AdmissionReobserver::new(
                std::env::var_os("PATH"),
                crate::harness::process::VERSION_TIMEOUT,
                {
                    let log = Arc::clone(&rate_limited_log);
                    Arc::new(move |line: &str| log.write_line(line))
                },
            );
            #[cfg(any(test, feature = "test-support"))]
            let admission_fault = factory_probe.admission_fault();
            if let Some(handle) = admission_handle(
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
                        // A detected version nobody has a row for asks for a
                        // manifest fetch (detached; never waits).
                        crate::daemon::harness_states::trigger_unseen_versions(
                            observer_store.as_ref(),
                            observer_manifest.as_ref(),
                            &observer_manifest.current(),
                            &crate::harness::registry::builtins()
                                .registrations()
                                .iter()
                                .filter(|r| r.legacy_contract_id().is_some())
                                .map(|r| {
                                    let id = r.metadata().id;
                                    (id, observed.detected_version(id))
                                })
                                .collect::<Vec<_>>(),
                            budget,
                        );
                        Ok(observed)
                    },
                    Arc::clone(&harnesses),
                    register_lane(Lane::AdmissionObserver),
                    Arc::clone(&factory_clock),
                    factory_stop.clone(),
                    Arc::clone(&factory_admission_status),
                ),
                &factory_admission_status,
            ) {
                push_worker(handle)?;
            }
            if let Some(session) = &primary {
                let archival_context = crate::protocol::output::ContinuationContext {
                    state_dir: Some(archival_paths.state_dir.to_string_lossy().into_owned()),
                    host: Some(session.locator().as_str().to_owned()),
                };
                push_worker(crate::service::archival::start(
                    crate::service::archival::ArchivalWorker {
                        store: Arc::clone(&store),
                        host: Arc::clone(session.port()),
                        writer: Arc::clone(&writer),
                        reachability: Arc::clone(session.reachability()),
                        source: crate::archival_legacy::Source::new(
                            &archival_paths,
                            instance.to_string(),
                            archival_context,
                        ),
                        boot: boot.to_string(),
                        after_ms: config.archive_after_ms(),
                        cancellation: factory_stop.clone(),
                    },
                    register_lane(Lane::Archival),
                    Arc::clone(&factory_archival_status),
                )?)?;
            }
            // In `Lane::ALL` order. A lane that never started (host lanes of a
            // store-only daemon) has an untouched status and reports ready; the
            // host lanes report the startup session's (per-host Health: ht-5fr.2.10).
            let lane_statuses = vec![
                factory_status.clone(),
                primary.as_ref().map_or_else(
                    || factory_wake_status.clone(),
                    |session| Arc::clone(session.wake().status()),
                ),
                primary.as_ref().map_or_else(
                    || factory_observation_status.clone(),
                    |session| Arc::clone(session.observation().status()),
                ),
                factory_retention_status.clone(),
                factory_admission_status.clone(),
                factory_archival_status.clone(),
                factory_apply_status.clone(),
                factory_sync_status.clone(),
            ];
            factory_probe.attach_registry(&kicks, lane_statuses.clone());
            let health_store = Arc::clone(&store);
            let evidence_store = Arc::clone(&store);
            let states_store = Arc::clone(&store);
            let health_clock = Arc::clone(&factory_clock);
            // Service host seam: with a host the service is composed with its
            // identity resolver; store-only it has none and `attached_host`
            // answers `HostUnavailable`.
            let mut domain = match identity {
                Some(identity) => DomainService::with_identity(
                    instance.to_string(),
                    store,
                    Arc::clone(&factory_clock),
                    identity,
                ),
                None => DomainService::new(instance.to_string(), store, Arc::clone(&factory_clock)),
            }
            .with_operator_owner(crate::daemon::paths::effective_uid())
            .with_cooperative_owner(crate::daemon::paths::effective_uid(), writer)
            .with_mod_channels(Arc::clone(&mod_channels))
            .with_delivery_pull(Arc::clone(&delivery_pull));
            // Guarded bootstrap needs an attached host with `HostGuardedStart`.
            let bootstrap = primary
                .as_ref()
                .and_then(|session| session.bootstrap().cloned());
            if let Some(runtime) = &bootstrap {
                domain = domain
                    .with_bootstrap_runtime(
                        runtime.canonical.clone(),
                        Arc::clone(&runtime.observer),
                        bootstrap_store,
                    )
                    .map_err(|error| io::Error::other(error.detail))?;
            }
            let stop = StopController::new(instance, boot, cancellation);
            let provider = elected_health_provider(
                instance,
                boot,
                config.health_settings(),
                health_clock,
                health_store,
                lane_statuses,
                ElectedHostEvidence::of_sessions(
                    &host_sessions,
                    primary.as_deref(),
                    factory_host_evidence.clone(),
                    harnesses,
                ),
            );
            let log_path = factory_log_path.clone();
            let health_harnesses = Arc::clone(&states_harnesses);
            let states = Arc::new(
                crate::daemon::harness_states::HarnessStatesProvider::new(
                    states_store,
                    crate::daemon::harness_states::service_source(Arc::clone(&manifest)),
                    Arc::clone(&factory_clock),
                    Box::new(move |harness: &str| {
                        states_harnesses
                            .lock()
                            .ok()
                            .and_then(|observed| observed.detected_version(harness))
                    }),
                    Some(Arc::clone(&factory_parse_failures)),
                )
                .with_observations(Box::new(move || {
                    health_harnesses
                        .lock()
                        .map(|snapshot| snapshot.entries.clone())
                        .map_err(|_| {
                            crate::protocol::results::ApiError::new(
                                crate::protocol::results::ErrorCode::StoreBusy,
                                "cached harness observation unavailable",
                            )
                        })
                }))
                .with_mod_channels(mod_channels),
            );
            let health_states = Arc::clone(&states);
            let health_log = Arc::clone(&rate_limited_log);
            let health = move |request: &CallBudget| {
                let mut inputs = provider(request);
                inputs.log_path = Some(log_path.clone());
                // A store read failure is logged (rate limited) and Health
                // shows the evidence-unavailable limitation (degraded).
                let answer = health_states.health_lines(request);
                if let Err(error) = &answer {
                    health_log.record_harness_states_unavailable(&error.detail);
                }
                inputs.harness_version_lines =
                    crate::daemon::health::harness_version_lines(&answer);
                inputs
            };
            let harness_evidence = Arc::new(
                crate::daemon::harness_evidence::HarnessEvidenceRecorder::new(
                    evidence_store.clone(),
                    Some(Arc::clone(&manifest)
                        as Arc<dyn crate::daemon::harness_evidence::ManifestTrigger>),
                    Arc::clone(&factory_clock),
                ),
            );
            let harness_evidence_v2 = Arc::new(
                crate::daemon::harness_evidence::HarnessEvidenceRecorderV2::new(
                    evidence_store,
                    Some(manifest.clone()
                        as Arc<dyn crate::daemon::harness_evidence::ManifestTrigger>),
                    factory_clock.clone(),
                )
                .with_legacy_pending(&harness_evidence)
                .with_rich_manifest_source(Arc::clone(&manifest)
                    as Arc<dyn crate::daemon::harness_evidence::RichManifestSource>),
            );
            let control = if bootstrap.is_some() {
                ControlService::new_guarded(stop, health, domain)
                    .map_err(|error| io::Error::other(error.detail))?
            } else {
                ControlService::new(stop, health, domain)
            };
            Ok(Arc::new(
                control
                    .with_hook_parse_failures(factory_parse_failures)
                    .with_harness_evidence(harness_evidence)
                    .with_harness_evidence_v2(harness_evidence_v2)
                    .with_harness_health_v2(Arc::clone(&states))
                    .with_harness_states(states)
                    .with_harness_manifest(manifest),
            ) as Arc<dyn LocalService>)
        },
        on_ready,
        move || async move {
            worker_stop.cancel();
            // The coordinator closes its links (1 s), cancels its dial tasks
            // and flushes contacts on the cancel; a task still running at the
            // bound is aborted and logged.
            let sync = sync_slot
                .lock()
                .map_err(|_| io::Error::other("sync slot poisoned"))?
                .take();
            if let Some(ComposedSync { handle, mut join }) = sync {
                drop(handle);
                if tokio::time::timeout(SYNC_STOP_BOUND, &mut join).await.is_err() {
                    join.abort();
                    let _ = join.await;
                    eprintln!(
                        "sync: coordinator still running after {} s; aborted",
                        SYNC_STOP_BOUND.as_secs()
                    );
                }
            }
            let sessions = sessions_slot
                .lock()
                .map_err(|_| io::Error::other("host sessions slot poisoned"))?
                .take();
            let workers = std::mem::take(
                &mut *worker_slot
                    .lock()
                    .map_err(|_| io::Error::other("worker slot poisoned"))?,
            );
            let joined = tokio::task::spawn_blocking(move || {
                // Sessions stop and join their own threads first; the slot
                // then holds only the daemon's lanes.
                if let Some(composed) = sessions {
                    composed.sessions.stop_all();
                }
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

#[cfg(test)]
mod poke_capability_source_tests {
    use super::*;
    use crate::{
        harness::recipe::{NativeSupport, PokeCapabilities},
        ports::PokeCapabilitySource,
    };

    fn source(claude: Option<&str>, codex: Option<&str>) -> ObservedPokeCapabilities {
        ObservedPokeCapabilities::new(Arc::new(Mutex::new(HarnessObservations {
            entries: [("claude", claude), ("codex", codex)]
                .into_iter()
                .map(|(name, version)| {
                    (
                        name.into(),
                        crate::harness::adapter::DaemonObservation {
                            identity: version.map(|v| {
                                crate::harness::runtime::RuntimeIdentity::stable_release(
                                    v,
                                    "installed_probe",
                                )
                                .unwrap()
                            }),
                            ..Default::default()
                        },
                    )
                })
                .collect(),
        })))
    }

    // Optional metadata alone is not captured current-runtime qualification.
    #[test]
    fn task3_versionless_optional_metadata_does_not_grant_rich_poke_capabilities() {
        let observed = Arc::new(Mutex::new(HarnessObservations {
            entries: std::collections::BTreeMap::from([(
                "claude".into(),
                crate::harness::adapter::DaemonObservation {
                    status: HarnessStatus::Cooperative {
                        detail: "contract_declared".into(),
                        live_unverified: false,
                    },
                    identity: Some(
                        crate::harness::runtime::RuntimeIdentity::stable_release(
                            "2.1.287",
                            "installed_probe",
                        )
                        .unwrap(),
                    ),
                    ..Default::default()
                },
            )]),
        }));
        let source = ObservedPokeCapabilities::new(observed);
        assert_eq!(source.capabilities(Harness::Claude), PokeCapabilities::NONE);
        // The historical capture declaration itself remains intact.
        assert_eq!(
            crate::harness::recipe::poke_capabilities(Harness::Claude, Some("2.1.287")),
            PokeCapabilities {
                composer_stash: NativeSupport::Supported
            }
        );
    }

    // Metadata/cache changes cannot substitute for safe current qualification.
    #[test]
    fn observed_metadata_and_unobserved_runtime_both_leave_richer_paths_unavailable() {
        for versions in [
            (Some("2.1.287"), Some("0.160.0")),
            (Some("2.1.286"), None),
            (None, None),
        ] {
            let source = source(versions.0, versions.1);
            for harness in [Harness::Claude, Harness::Codex, Harness::Human] {
                assert_eq!(source.capabilities(harness), PokeCapabilities::NONE);
            }
        }
        let slot = Arc::new(Mutex::new(HarnessObservations::default()));
        let late = ObservedPokeCapabilities::new(Arc::clone(&slot));
        assert_eq!(late.capabilities(Harness::Claude), PokeCapabilities::NONE);
        slot.lock().unwrap().entries.insert(
            "claude".into(),
            crate::harness::adapter::DaemonObservation {
                identity: Some(
                    crate::harness::runtime::RuntimeIdentity::stable_release(
                        "2.1.287",
                        "installed_probe",
                    )
                    .unwrap(),
                ),
                ..Default::default()
            },
        );
        assert_eq!(late.capabilities(Harness::Claude), PokeCapabilities::NONE);
    }

    /// Kills: a source that declares poke capabilities for a detected
    /// version the admission ladder refused (recorded for doctor only).
    #[test]
    fn refused_status_declares_no_poke_capabilities() {
        for status in [
            HarnessStatus::VersionRefused("known broken".into()),
            HarnessStatus::Refused("unrecognized".into()),
        ] {
            let refused =
                ObservedPokeCapabilities::new(Arc::new(Mutex::new(HarnessObservations {
                    entries: [(
                        "claude".into(),
                        crate::harness::adapter::DaemonObservation {
                            status,
                            identity: Some(
                                crate::harness::runtime::RuntimeIdentity::stable_release(
                                    "2.1.287",
                                    "installed_probe",
                                )
                                .unwrap(),
                            ),
                            ..Default::default()
                        },
                    )]
                    .into(),
                })));
            assert_eq!(
                refused.capabilities(Harness::Claude),
                PokeCapabilities::NONE
            );
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/lane_commit_observer.rs"]
mod lane_commit_observer_tests;

#[cfg(test)]
pub(crate) mod health_v2_observer_tests {
    use super::*;

    use crate::harness::adapter::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    pub(crate) struct CountingAdapter {
        metadata: &'static AdapterMetadata,
        pub(crate) calls: AtomicUsize,
        contracts: &'static [ContractDescriptor],
        input: AtomicUsize,
        refuse: AtomicBool,
        fingerprint: AtomicBool,
        mutate: AtomicBool,
        cancel: Mutex<Option<Cancellation>>,
    }
    impl HarnessAdapter for CountingAdapter {
        type Admission = ();
        fn metadata(&self) -> &'static AdapterMetadata {
            self.metadata
        }
        fn contracts(&self) -> &'static [ContractDescriptor] {
            self.contracts
        }
        fn observation_fingerprint(&self, _: &InstallEnvironment) -> Option<String> {
            self.fingerprint.load(Ordering::SeqCst).then(|| {
                format!(
                    "same-binary:profile-assets-{}",
                    self.input.load(Ordering::SeqCst)
                )
            })
        }
        fn observe_daemon(&self, _: &InstallEnvironment, _: &CallBudget) -> DaemonObservation {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.mutate.swap(false, Ordering::SeqCst) {
                self.input.fetch_add(1, Ordering::SeqCst);
            }
            if let Some(cancel) = self.cancel.lock().unwrap().take() {
                cancel.cancel();
            }
            DaemonObservation {
                status: if self.refuse.load(Ordering::SeqCst) {
                    HarnessStatus::Refused("transient fixture".into())
                } else {
                    HarnessStatus::Cooperative {
                        detail: format!("assets {}", self.input.load(Ordering::SeqCst)),
                        live_unverified: false,
                    }
                },
                ..Default::default()
            }
        }
        fn observe_install(&self, _: &InstallEnvironment, _: &CallBudget) -> InstallObservation {
            panic!("health may only use owned observation")
        }
        fn admit(&self, _: &AdmissionRequest, _: &CallBudget) -> AdmissionDecision<()> {
            panic!("unused")
        }
        fn version_ladder(&self, _: &RuntimeIdentity) -> Ladder {
            Ladder::Listed
        }
        fn classify(&self, _: &HookInput) -> ContractObservation {
            panic!("unused")
        }
        fn decode(&self, _: &(), _: &HookInput) -> Result<DecodedEvent, DecodeFailure> {
            panic!("unused")
        }
        fn encode(
            &self,
            _: &(),
            _: &DecodedEvent,
            _: &NeutralOffer,
        ) -> Result<EncodedOutput, EncodeFailure> {
            panic!("unused")
        }
        fn attribute_runtime(&self, _: &HookInput, _: &CallBudget) -> RuntimeAttribution {
            panic!("unused")
        }
        fn setup(&self, _: &SetupRequest, _: &CallBudget) -> Result<SetupOutcome, SetupFailure> {
            panic!("unused")
        }
        fn status(&self, _: &StatusRequest, _: &CallBudget) -> SetupStatus {
            panic!("unused")
        }
        fn unsetup(
            &self,
            _: &UnsetupRequest,
            _: &CallBudget,
        ) -> Result<RemovalOutcome, SetupFailure> {
            panic!("unused")
        }
    }
    pub(crate) fn counting(id: &'static str) -> &'static CountingAdapter {
        counting_with_contracts(id, &[])
    }
    pub(crate) fn counting_with_contracts(
        id: &'static str,
        contracts: &'static [ContractDescriptor],
    ) -> &'static CountingAdapter {
        Box::leak(Box::new(CountingAdapter {
            metadata: Box::leak(Box::new(AdapterMetadata {
                agent_env_markers: &[],
                id,
                display_label: id,
                context_spelling: id,
                context_aliases: &[],
                executable: ExecutableLookup::Unsupported,
                host_kinds: Box::leak(vec![id].into_boxed_slice()),
                setup_scopes: &[SetupScopeKind::Profile],
                budget: EventBudgetPolicy {
                    lifecycle_ms: 10,
                    observer_ms: 10,
                },
                runtime_sources: &["fixture"],
            })),
            contracts,
            calls: AtomicUsize::new(0),
            input: AtomicUsize::new(0),
            refuse: AtomicBool::new(false),
            fingerprint: AtomicBool::new(true),
            mutate: AtomicBool::new(false),
            cancel: Mutex::new(None),
        }))
    }
    fn counting_observer(adapters: &[&'static CountingAdapter]) -> AdmissionReobserver {
        let registrations = Box::leak(
            adapters
                .iter()
                .map(|a| crate::harness::registry::Registration::new(*a))
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        );
        let registry = Box::leak(Box::new(
            crate::harness::registry::Registry::new(registrations).unwrap(),
        ));
        AdmissionReobserver::with_registry(
            registry,
            InstallEnvironment {
                clock: Arc::new(SystemClock::new()),
                path: None,
                config_root: None,
                state_dir: None,
            },
            Duration::from_millis(50),
            Arc::new(|_| {}),
        )
    }
    // Catches binary-only caching, ignored registry entries and transient refusal reuse.
    #[test]
    fn health_v2_adapter_fingerprint_reuses_only_unchanged_declared_inputs() {
        let first = counting("first");
        let extra = counting("third");
        let observer = counting_observer(&[first, extra]);
        let snapshot = observer.pass(&Cancellation::default());
        assert_eq!(
            snapshot
                .entries
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["first", "third"]
        );
        observer.pass(&Cancellation::default());
        assert_eq!(
            extra.calls.load(Ordering::SeqCst),
            1,
            "unchanged reusable adapter must not be probed again"
        );
        extra.input.store(1, Ordering::SeqCst);
        observer.pass(&Cancellation::default());
        assert_eq!(
            extra.calls.load(Ordering::SeqCst),
            2,
            "changed assets on the same binary require observation"
        );
        extra.refuse.store(true, Ordering::SeqCst);
        extra.input.store(2, Ordering::SeqCst);
        observer.pass(&Cancellation::default());
        observer.pass(&Cancellation::default());
        assert_eq!(
            extra.calls.load(Ordering::SeqCst),
            4,
            "unchanged transient refusals must retry"
        );
        extra.refuse.store(false, Ordering::SeqCst);
        extra.fingerprint.store(false, Ordering::SeqCst);
        observer.pass(&Cancellation::default());
        observer.pass(&Cancellation::default());
        assert_eq!(
            extra.calls.load(Ordering::SeqCst),
            6,
            "None fingerprint must never permit reuse"
        );
    }
    // Catches caching a probe under inputs that changed during the probe.
    #[test]
    fn health_v2_changed_inputs_during_probe_are_unavailable_and_not_reused() {
        let adapter = counting("racy");
        adapter.mutate.store(true, Ordering::SeqCst);
        let observer = counting_observer(&[adapter]);
        let snapshot = observer.pass(&Cancellation::default());
        assert!(
            matches!(snapshot.status("racy"), HarnessStatus::Refused(_)),
            "changed input pass was trusted"
        );
        observer.pass(&Cancellation::default());
        assert_eq!(adapter.calls.load(Ordering::SeqCst), 2);
    }
    // Catches partial publication after a reused entry and a later cancelled probe.
    #[test]
    fn health_v2_cancel_after_reuse_preserves_every_registry_entry() {
        let first = counting("first");
        let second = counting("second");
        let observer = counting_observer(&[first, second]);
        let before = observer.pass(&Cancellation::default());
        second.input.store(1, Ordering::SeqCst);
        let cancel = Cancellation::default();
        *second.cancel.lock().unwrap() = Some(cancel.clone());
        assert_eq!(observer.pass(&cancel), before);
        assert_eq!(
            first.calls.load(Ordering::SeqCst),
            1,
            "first unchanged entry should have been reused"
        );
        assert_eq!(
            observer.pass(&Cancellation::default()).status("first"),
            before.status("first")
        );
        assert_eq!(
            second.calls.load(Ordering::SeqCst),
            3,
            "cancelled replacement must not enter the cache"
        );
    }

    // Catches fixed-brand rendering or false admission/native-receipt claims for an injected adapter.
    #[test]
    fn health_wording_uses_supplied_registry_and_unknown_optional_metadata() {
        let adapter = counting("fourth");
        let registrations = Box::leak(
            vec![crate::harness::registry::Registration::new(adapter)].into_boxed_slice(),
        );
        let registry = crate::harness::registry::Registry::new(registrations).unwrap();
        let receipt = crate::daemon::health::cooperative_receipt_line_for(&registry);
        assert!(
            receipt.contains("fourth admission unknown"),
            "missing honest adapter metadata: {receipt}"
        );
        assert!(!receipt.contains("admitted: claude"));
        let wake = crate::daemon::health::cooperative_wake_line_for(&registry);
        assert!(
            wake.contains("idle/done fourth agent"),
            "missing supplied registered host kind: {wake}"
        );
        assert!(!wake.contains("during a turn"));
        let builtins = crate::harness::registry::builtins();
        let legacy =
            crate::harness::registry::Registry::new(&builtins.registrations()[..2]).unwrap();
        assert_eq!(
            crate::daemon::health::cooperative_wake_line_for(&legacy),
            crate::daemon::health::COOPERATIVE_WAKE_LINE
        );
        let expected = if cfg!(feature = "test-support") {
            "wake cooperative: prompts only Herdr's detected idle/done claude, codex, hermes, agy or synthetic_fourth agent in the seat's terminal, rechecked immediately before submission; native execution and composer contents are unverified"
        } else {
            "wake cooperative: prompts only Herdr's detected idle/done claude, codex, hermes or agy agent in the seat's terminal, rechecked immediately before submission; native execution and composer contents are unverified"
        };
        assert_eq!(
            crate::daemon::health::cooperative_wake_line_for(builtins),
            expected
        );
    }

    // Catches cancellation erasing a successful snapshot rather than retaining it whole.
    #[test]
    fn health_v2_registry_snapshot_atomic_cancel_and_frozen_legacy_projection() {
        let iso = crate::test_support::isolation::TestIsolation::new("health-v2-cancel");
        crate::harness::stub_binaries::write_stub_harness(iso.state_root(), "claude", "2.1.286");
        let observer = AdmissionReobserver::new(
            Some(iso.state_root().as_os_str().to_owned()),
            Duration::from_secs(2),
            Arc::new(|_| {}),
        );
        let before = observer.pass(&Cancellation::default());
        assert!(matches!(
            before.status("claude"),
            HarnessStatus::ContractDeclared { .. }
        ));
        let cancel = Cancellation::default();
        cancel.cancel();
        assert_eq!(
            observer.pass(&cancel),
            before,
            "cancelled pass must preserve the entire last snapshot"
        );
        assert_eq!(observer.pass(&Cancellation::default()), before);
        let legacy = crate::protocol::results::HarnessHealth {
            claude: before.status("claude").state(),
            codex: before.status("codex").state(),
        };
        assert_eq!(
            serde_json::to_string(&legacy).unwrap(),
            r#"{"codex":"unsupported","claude":"cooperative"}"#
        );
    }
}

#[cfg(test)]
mod store_identity_tests {
    #[test]
    fn host_label_truncates_at_char_boundary() {
        // 'é' is two bytes: 128 of them are 256 bytes, so the cut at 255 falls
        // inside the last char and must back up to 254.
        let text = "é".repeat(128);
        let cut = super::truncate_label(&text);
        assert_eq!(cut.len(), 254);
        assert!(text.starts_with(&cut));
        assert_eq!(super::truncate_label("lab"), "lab");
        assert!(super::host_label().len() <= 255);
    }

    use super::*;
    use crate::{
        node::tag_for,
        protocol::time::UtcMillis,
        store::schema::{NodeSeed, initialize},
    };

    fn store_of(seed: &NodeSeed) -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        initialize(&conn, || UtcMillis(1), seed).unwrap();
        conn
    }

    // Kills: starting a daemon on a store that belongs to another node's key
    // (the check accepting or skipping a `local_node.id` that differs).
    #[test]
    fn restart_with_foreign_local_node_id_fails_identity_mismatch() {
        let store_node = crate::test_support::test_node_seed();
        let conn = store_of(&store_node);
        let key = NodeIdentity::from_seed([7; 32]);
        assert_ne!(key.uuid(), store_node.uuid);
        let key_path = std::path::Path::new("/node/node.key");
        let error = check_store_identity(&conn, &key, key_path).unwrap_err();
        let text = error.to_string();
        assert!(
            text.starts_with("node key refused (IdentityMismatch): "),
            "{text}"
        );
        assert!(text.contains(&store_node.uuid.to_string()), "{text}");
        assert!(text.contains(&key.uuid().to_string()), "{text}");
        assert!(text.contains("/node/node.key"), "{text}");
        assert_eq!(key.tag_k(), 0, "a refused start adopts nothing");
    }

    // Kills: an identity check that swallows a real read failure (here a
    // missing column) as "nothing to check".
    #[test]
    fn unreadable_node_row_fails_start_but_absent_table_or_row_skips() {
        let key = NodeIdentity::from_seed([7; 32]);
        let path = std::path::Path::new("/node/node.key");
        let empty = rusqlite::Connection::open_in_memory().unwrap();
        check_store_identity(&empty, &key, path).unwrap();
        empty
            .execute_batch("CREATE TABLE local_node (id TEXT, tag_k INTEGER)")
            .unwrap();
        check_store_identity(&empty, &key, path).unwrap();
        let broken = rusqlite::Connection::open_in_memory().unwrap();
        broken
            .execute_batch("CREATE TABLE local_node (other TEXT)")
            .unwrap();
        let error = check_store_identity(&broken, &key, path).unwrap_err();
        assert!(error.to_string().contains("identity check"), "{error}");
    }

    // Kills: a daemon that keeps `tag_k = 0` after a retag, or leaves node.json
    // carrying the pre-store tag.
    #[test]
    fn node_json_regenerated_with_stored_tag() {
        let key = NodeIdentity::from_seed([7; 32]);
        let seed = NodeSeed {
            uuid: key.uuid(),
            tag: tag_for(key.uuid(), 0),
            tag_k: 0,
        };
        let conn = store_of(&seed);
        conn.execute("UPDATE local_node SET tag_k=2", []).unwrap();
        let iso = crate::test_support::isolation::TestIsolation::new("store-identity-json");
        let paths = crate::test_support::resolve_seeded(iso.state_root());
        // The pre-store write, as load_node_identity does.
        publish_node_json(&paths, &key).unwrap();
        let before = crate::node::json::read_hint(&paths.node_json_path).unwrap();
        assert_eq!((before.tag_k, before.tag), (0, tag_for(key.uuid(), 0)));
        check_store_identity(&conn, &key, &paths.key_path).unwrap();
        assert_eq!(key.tag_k(), 2);
        publish_node_json(&paths, &key).unwrap();
        let after = crate::node::json::read_hint(&paths.node_json_path).unwrap();
        assert_eq!(after.node_uuid, key.uuid());
        assert_eq!((after.tag_k, after.tag), (2, tag_for(key.uuid(), 2)));
        assert_eq!(after.created_at, before.created_at);
    }
}
