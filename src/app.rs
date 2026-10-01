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
        host_evidence::HostEvidenceStatus,
        workers::{
            FairWriter, WorkerStatus, start_deadline_worker, start_observation_worker,
            start_wake_worker,
        },
    },
    store::{SqliteStore, connection::StoreContext},
};
use std::{
    io,
    sync::{Arc, Mutex},
    time::{Instant, SystemTime, UNIX_EPOCH},
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
/// `workers` is the deadline, wake and observation worker status, in that order.
/// `host` supplies the host fields: they come only from evidence the
/// observation lane observed, under the adapter's incarnation witness.
pub(crate) fn elected_health_provider(
    instance: Uuid,
    boot: Uuid,
    settings: HealthSettings,
    clock: Arc<dyn Clock>,
    store: Arc<dyn StorePort>,
    workers: [Arc<WorkerStatus>; 3],
    host: ElectedHostEvidence,
) -> impl Fn(&CallBudget) -> HealthInputs + Send + Sync + 'static {
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
    workers: [Arc<WorkerStatus>; 3],
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
        inputs.last_scheduler_tick_at = self.workers[0].last_tick();
        if let Ok(observed) = self.host.harnesses.lock() {
            inputs.claude = observed.claude.clone();
            inputs.codex = observed.codex.clone();
        }
        inputs
    }
}

const LANE_NAMES: [&str; 3] = ["deadline", "wake", "observation"];

/// A lane whose thread ended without a requested stop outranks any typed
/// failure: it is no longer making progress at all.
fn scheduler_status(workers: &[Arc<WorkerStatus>; 3]) -> ComponentStatus {
    let dead: Vec<&str> = workers
        .iter()
        .zip(LANE_NAMES)
        .filter(|(status, _)| status.lane_dead())
        .map(|(_, name)| name)
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
        Ok(_) if native == CapabilityState::Supported => HarnessStatus::Supported(admission.line()),
        Ok(version) => HarnessStatus::Cooperative {
            detail: admission.line(),
            live_unverified: matches!(version.admission(), Admission::SchemaMatched { .. }),
        },
    }
}

/// Classify the installed `claude` (`None` when no executable is on `PATH`;
/// otherwise its `--version` observation, admitted only by a recipe).
pub(crate) fn claude_status(
    observed: Option<Result<String, crate::harness::codex::VersionError>>,
    native: CapabilityState,
) -> HarnessStatus {
    use crate::harness::codex::VersionError;
    let recipes = crate::harness::recipe::describe(crate::harness::claude::RECIPES);
    match observed {
        None => HarnessStatus::NotInstalled("no executable `claude` on the daemon's PATH".into()),
        Some(Ok(version)) => {
            let detail = match crate::harness::claude::recipe_for(&version) {
                Ok(recipe) => format!("claude {version}: listed: recipe {}", recipe.id),
                Err(_) => format!("claude {version}: listed"),
            };
            if native == CapabilityState::Supported {
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
        Some(Err(_)) => HarnessStatus::Refused(format!(
            "claude --version could not be observed or recognized; supported recipes: {recipes}"
        )),
    }
}

/// Observe the installed `claude` and `codex` on the daemon's `PATH` once, on
/// a detached thread bounded by the version-observation deadline, and publish
/// each harness's status for Health. A listed version costs one `--version`
/// run; an unlisted Codex is admitted only when its embedded hook schemas
/// hash-match a recipe (schema-matched, live-unverified). Health never waits
/// for it.
fn observe_harness_admissions() -> Arc<Mutex<HarnessObservations>> {
    let slot = Arc::new(Mutex::new(HarnessObservations::default()));
    let writer = Arc::clone(&slot);
    let path = std::env::var_os("PATH");
    let _ = std::thread::Builder::new()
        .name("harness-admission".into())
        .spawn(move || {
            let timeout = crate::harness::codex::VERSION_TIMEOUT;
            let claude = claude_status(
                crate::cli::hook::resolve_on_path("claude", path.as_deref()).map(|binary| {
                    crate::harness::claude::observe_installed_version(&binary, timeout)
                }),
                crate::harness::claude::health_capability(),
            );
            if let Ok(mut observed) = writer.lock() {
                observed.claude = claude;
            }
            let admission = crate::harness::codex::InstalledAdmission::observe_on_path(
                path.as_deref(),
                timeout,
            );
            let codex = codex_status(
                &admission,
                crate::harness::codex::DECLARATION.health_capability(),
            );
            if let Ok(mut observed) = writer.lock() {
                observed.codex = codex;
            }
        });
    slot
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
    let database_path = paths.database_path.clone();
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
    let host_evidence = Arc::new(HostEvidenceStatus::default());
    let factory_host_evidence = Arc::clone(&host_evidence);
    run_elected_with_diagnostics(
        paths,
        clock,
        shutdown,
        move |instance, boot, cancellation| {
            let context = StoreContext::new(database_path, Arc::clone(&factory_clock));
            let store: Arc<dyn StorePort> = Arc::new(
                SqliteStore::new(context, instance.to_string(), config.store_settings(boot))
                    .map_err(|error| {
                        io::Error::other(format!(
                            "store startup: {:?}: {}",
                            error.code, error.detail
                        ))
                    })?,
            );
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
            let mut workers = factory_slot
                .lock()
                .map_err(|_| io::Error::other("worker slot poisoned"))?;
            let worker = start_deadline_worker(
                Arc::clone(&store),
                Arc::clone(&writer),
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
                factory_stop.clone(),
                Arc::clone(&factory_wake_status),
            )?);
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
                .with_service_cancellation(cancellation.clone()),
            );
            workers.push(start_observation_worker(
                Arc::clone(&identity),
                Arc::clone(&store),
                Arc::clone(&writer),
                factory_stop,
                Arc::clone(&factory_observation_status),
                Arc::clone(&factory_host_evidence),
            )?);
            drop(workers);
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
            let health = elected_health_provider(
                instance,
                boot,
                config.health_settings(),
                health_clock,
                health_store,
                [
                    factory_status.clone(),
                    factory_wake_status.clone(),
                    factory_observation_status.clone(),
                ],
                ElectedHostEvidence {
                    status: factory_host_evidence.clone(),
                    incarnation_witness,
                    safe_prompt,
                    harnesses: observe_harness_admissions(),
                },
            );
            Ok(Arc::new(ControlService::new(stop, health, domain)) as Arc<dyn LocalService>)
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
