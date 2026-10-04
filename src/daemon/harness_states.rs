//! The daemon's version verdicts (ht-xoc.5): Health's one broken line per
//! harness and the `harness.states` report doctor prints, both derived from
//! the evidence store and the manifest by `harness::state`.
use std::sync::Arc;

use crate::{
    daemon::{harness_evidence::ManifestTrigger, logs::HookParseFailures},
    harness::{
        contract::normalize_version,
        manifest::{self, FetchReason, Manifest, ManifestService},
        state::{self, HarnessRollup, State},
    },
    ports::StorePort,
    protocol::{
        results::{
            ApiError, DetectedVersion, HARNESS_STATE_VERSIONS, HarnessStateReport,
            HarnessStatesReport, UnattributedReport, VersionStateReport,
        },
        time::{CallBudget, Clock},
    },
    store::harness_evidence::EvidenceRow,
};

/// The version of `harness` found on the daemon's `PATH` (canonical `X.Y.Z`),
/// as the admission observer last observed it.
pub type DetectedVersions = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// The manifest verdicts are derived against, read on every call (a fetch
/// may replace it).
pub type CachedHarnessObservations = Box<
    dyn Fn() -> Result<
            std::collections::BTreeMap<String, crate::harness::adapter::DaemonObservation>,
            ApiError,
        > + Send
        + Sync,
>;

pub type ManifestSource = Box<dyn Fn() -> Arc<Manifest> + Send + Sync>;

type BrokenRuntimeRollup = std::collections::BTreeMap<String, (u64, String)>;

/// The daemon's manifest service as a [`ManifestSource`].
pub fn service_source(service: Arc<ManifestService>) -> ManifestSource {
    Box::new(move || service.current())
}

/// The embedded manifest as a [`ManifestSource`] (no service).
pub fn embedded_source() -> ManifestSource {
    Box::new(|| Arc::new(manifest::embedded().clone()))
}

pub struct HarnessStatesProvider {
    store: Arc<dyn StorePort>,
    manifest: ManifestSource,
    clock: Arc<dyn Clock>,
    detected: DetectedVersions,
    parse_failures: Option<Arc<HookParseFailures>>,
    registry: &'static crate::harness::registry::Registry,
    observations: Option<CachedHarnessObservations>,
}

impl HarnessStatesProvider {
    pub fn new(
        store: Arc<dyn StorePort>,
        manifest: ManifestSource,
        clock: Arc<dyn Clock>,
        detected: DetectedVersions,
        parse_failures: Option<Arc<HookParseFailures>>,
    ) -> Self {
        Self {
            store,
            manifest,
            clock,
            detected,
            parse_failures,
            registry: crate::harness::registry::builtins(),
            observations: None,
        }
    }

    pub fn with_registry(mut self, registry: &'static crate::harness::registry::Registry) -> Self {
        self.registry = registry;
        self
    }
    pub fn with_observations(mut self, observations: CachedHarnessObservations) -> Self {
        self.observations = Some(observations);
        self
    }
    pub fn has_observations(&self) -> bool {
        self.observations.is_some()
    }
    /// Reads only cached installation facts and bounded store/manifest data.
    /// It never calls an adapter observation/status method or native process.
    pub fn report_v2(
        &self,
        budget: &CallBudget,
    ) -> Result<crate::protocol::results::HarnessHealthV2Report, ApiError> {
        self.report_v2_with_broken_rollup(budget)
            .map(|(report, _)| report)
    }

    fn report_v2_with_broken_rollup(
        &self,
        budget: &CallBudget,
    ) -> Result<
        (
            crate::protocol::results::HarnessHealthV2Report,
            BrokenRuntimeRollup,
        ),
        ApiError,
    > {
        use crate::daemon::health::HarnessStatus;
        use crate::protocol::results::*;
        use std::collections::BTreeMap;
        let observations = self.observations.as_ref().ok_or_else(|| {
            ApiError::new(
                ErrorCode::Unsupported,
                "harness health observations unavailable",
            )
        })?;
        let cached = observations()?;
        let manifest = (self.manifest)();
        let now = self.now_ms();
        let counts = self
            .parse_failures
            .as_ref()
            .map(|f| f.snapshot())
            .unwrap_or_default();
        let mut harnesses = BTreeMap::new();
        let mut broken_rollup = BTreeMap::new();
        for registration in self.registry.registrations() {
            if budget.cancellation.is_cancelled() {
                return Err(ApiError::cancelled("harness health cancelled"));
            }
            if self.clock.monotonic_now() >= budget.deadline {
                return Err(ApiError::new(
                    ErrorCode::DeadlineExceeded,
                    "harness health budget expired",
                ));
            }
            let id = registration.metadata().id;
            let observed = cached.get(id).cloned().unwrap_or_default();
            let mut limitations = Vec::new();
            let mut notes = Vec::new();
            let detail = match &observed.status {
                HarnessStatus::Unknown => None,
                HarnessStatus::Cooperative { detail, .. }
                | HarnessStatus::NotInstalled(detail)
                | HarnessStatus::Refused(detail)
                | HarnessStatus::VersionRefused(detail)
                | HarnessStatus::Supported(detail)
                | HarnessStatus::Optimistic(detail) => Some(health_text(detail, 256)),
            };
            let (installation, admission) = match &observed.status {
                HarnessStatus::Unknown => {
                    limitations.push("installed runtime has not been observed".into());
                    (InstallationState::Unknown, AdmissionState::Unknown)
                }
                HarnessStatus::NotInstalled(_) => {
                    notes.push("executable not found in the daemon environment".into());
                    (InstallationState::NotFound, AdmissionState::Unsupported)
                }
                HarnessStatus::Refused(_) => {
                    limitations.push(
                        detail
                            .clone()
                            .unwrap_or_else(|| "installed runtime unavailable".into()),
                    );
                    (InstallationState::Unavailable, AdmissionState::Refused)
                }
                HarnessStatus::VersionRefused(_) => {
                    (InstallationState::Present, AdmissionState::Refused)
                }
                HarnessStatus::Cooperative {
                    live_unverified: true,
                    ..
                } => (InstallationState::Present, AdmissionState::SchemaMatched),
                HarnessStatus::Cooperative { .. } | HarnessStatus::Supported(_) => {
                    (InstallationState::Present, AdmissionState::Listed)
                }
                HarnessStatus::Optimistic(_) => {
                    (InstallationState::Present, AdmissionState::Optimistic)
                }
            };
            let rows = self.store.harness_evidence_v2_all(id, 0, budget)?;
            // The historical collection supplies candidate identity/timestamps only.
            // derive_runtime gates usable facts with current exact descriptors.
            let mut candidates: BTreeMap<_, RuntimeCandidate> = BTreeMap::new();
            for row in manifest
                .runtime_rows()
                .iter()
                .filter(|row| row.harness == id)
            {
                let candidate = RuntimeCandidate {
                    identity: row.identity.clone(),
                    domain: row.domain.clone(),
                    origin: row.origin,
                    contract: row.contract_id.clone(),
                    at: row.last_seen_at,
                    local: None,
                };
                candidates.insert(candidate.key(), candidate);
            }
            for row in rows {
                let candidate = RuntimeCandidate {
                    identity: row.identity.clone(),
                    domain: row.domain.clone(),
                    origin: row.origin,
                    contract: row.contract_id.clone(),
                    at: row.last_seen_at,
                    local: Some(row),
                };
                let key = candidate.key();
                let at = candidates
                    .get(&key)
                    .map_or(candidate.at, |old| old.at.max(candidate.at));
                candidates.insert(key, RuntimeCandidate { at, ..candidate });
            }
            let mut candidates: Vec<_> = candidates.into_values().collect();
            candidates.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| a.key().cmp(&b.key())));
            let runtime_evidence = candidates
                .into_iter()
                .map(|candidate| {
                    let derived = state::derive_runtime(
                        registration,
                        &candidate.identity,
                        &candidate.domain,
                        candidate.origin,
                        &candidate.contract,
                        candidate.local.as_ref(),
                        &manifest,
                    );
                    RuntimeEvidenceHealth {
                        identity: candidate.identity,
                        domain: candidate.domain,
                        origin: candidate.origin,
                        contract_id: candidate.contract,
                        state: derived.state,
                        source: health_text(&derived.source, 256),
                        line: health_text(&derived.line, 256),
                        notes: derived
                            .notes
                            .iter()
                            .take(16)
                            .map(|s| health_text(s, 256))
                            .collect(),
                        issue_url: derived.issue_url.as_deref().map(|s| health_text(s, 256)),
                        last_seen_at: candidate.at,
                        in_health_window: now.saturating_sub(candidate.at)
                            <= state::HEALTH_WINDOW_MS,
                        scope: HarnessHealthScope::all_runtime_scopes(),
                    }
                })
                .collect::<Vec<_>>();
            // The bounded diagnostic input must retain its newest broken row
            // even when newer non-broken rows exhaust the public display cap.
            if let Some(row) = runtime_evidence
                .iter()
                .find(|row| row.in_health_window && row.state == RuntimeEvidenceState::Broken)
            {
                broken_rollup.insert(id.into(), (row.last_seen_at, row.line.clone()));
            }
            let runtime_evidence = runtime_evidence.into_iter().take(20).collect();
            let mut unattributed = Vec::new();
            for descriptor in registration.contracts() {
                if let Some((reason, at)) = self.store.last_unattributed_v2(
                    id,
                    descriptor.domain_id,
                    descriptor.origin,
                    budget,
                )? {
                    unattributed.push(UnattributedHealthV2 {
                        domain: descriptor.domain_id.into(),
                        origin: descriptor.origin,
                        reason: health_text(&reason, 256),
                        at,
                    });
                }
            }
            let mut enablement = observed.enablement;
            enablement.detail = enablement.detail.as_deref().map(|s| health_text(s, 256));
            let mut callback_observation = observed.callback_observation;
            callback_observation.detail = callback_observation
                .detail
                .as_deref()
                .map(|s| health_text(s, 256));
            harnesses.insert(
                id.into(),
                AdapterHealthV2Report {
                    scope: HarnessHealthScope::daemon_default(),
                    installation: HealthAxis {
                        state: installation,
                        detail: detail.clone(),
                    },
                    admission: HealthAxis {
                        state: admission,
                        detail,
                    },
                    enablement,
                    callback_observation,
                    receipt_basis: observed
                        .receipt_basis
                        .as_deref()
                        .map(|s| health_text(s, 128))
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "unknown".into()),
                    limitations,
                    notes,
                    runtime_evidence,
                    unattributed,
                    hook_parse_failures: counts
                        .iter()
                        .find(|(name, _)| name == id)
                        .map_or(0, |(_, n)| *n),
                },
            );
        }
        let report = HarnessHealthV2Report { harnesses };
        report
            .validate()
            .map_err(|reason| ApiError::new(ErrorCode::InvalidRequest, reason))?;
        Ok((report, broken_rollup))
    }

    fn now_ms(&self) -> u64 {
        u64::try_from(self.clock.utc_now().0).unwrap_or(0)
    }

    fn with_manifest<T>(&self, run: impl FnOnce(&Manifest) -> T) -> T {
        run(&(self.manifest)())
    }

    /// Health's lines: at most one per harness (the broken version seen most
    /// recently within 24 hours, under the contract id the harness's hooks send now).
    pub fn health_lines(&self, budget: &CallBudget) -> Result<Vec<String>, ApiError> {
        let now = self.now_ms();
        let mut all = Vec::new();
        for registration in self.registry.registrations() {
            let harness = registration.metadata().id;
            if registration.legacy_contract_id().is_some() {
                all.push((harness, self.store.harness_evidence_all(harness, budget)?));
            }
        }
        let mut lines: BrokenRuntimeRollup = self.with_manifest(|manifest| {
            all.iter()
                .filter_map(|(harness, rows)| {
                    let rollup =
                        state::roll_up(harness, rows, manifest, env!("CARGO_PKG_VERSION"), now);
                    let line = rollup.health_line()?;
                    let at = rollup
                        .versions
                        .iter()
                        .filter(|v| {
                            v.in_health_window && matches!(v.derived.state, state::State::Broken(_))
                        })
                        .map(|v| v.last_seen_at)
                        .max()
                        .unwrap_or(0);
                    Some(((*harness).into(), (at, line)))
                })
                .collect()
        });
        if self.has_observations() {
            for (id, (at, line)) in self.report_v2_with_broken_rollup(budget)?.1 {
                if lines.get(&id).is_none_or(|(old_at, _)| at >= *old_at) {
                    lines.insert(id, (at, line));
                }
            }
        }
        Ok(self
            .registry
            .registrations()
            .iter()
            .filter_map(|r| lines.remove(r.metadata().id).map(|(_, line)| line))
            .collect())
    }

    /// The `harness.states` report.
    pub fn report(&self, budget: &CallBudget) -> Result<HarnessStatesReport, ApiError> {
        let now = self.now_ms();
        let mut harnesses = Vec::new();
        for registration in self.registry.registrations() {
            let harness = registration.metadata().id;
            let legacy = registration.legacy_contract_id().is_some();
            let rows = if legacy {
                self.store.harness_evidence_all(harness, budget)?
            } else {
                vec![]
            };
            let unattributed = self
                .store
                .last_unattributed(harness, budget)?
                .map(|(reason, at)| UnattributedReport { reason, at });
            let failures = self
                .parse_failures
                .as_ref()
                .map(|failures| failures.snapshot())
                .unwrap_or_default()
                .into_iter()
                .find(|(name, _)| name == harness)
                .map_or(0, |(_, count)| count);
            let detected = legacy
                .then(|| (self.detected)(harness))
                .flatten()
                .and_then(|raw| normalize_version(harness, &raw));
            if !legacy {
                harnesses.push(HarnessStateReport {
                    harness: harness.into(),
                    contract_id: None,
                    detected: None,
                    versions: vec![],
                    unattributed,
                    hook_parse_failures: failures,
                });
                continue;
            }
            harnesses.push(self.with_manifest(|manifest| {
                harness_report(
                    harness,
                    &rows,
                    manifest,
                    now,
                    detected,
                    unattributed,
                    failures,
                )
            }));
        }
        Ok(HarnessStatesReport { harnesses })
    }
}

/// Sanitizes diagnostic facts; no native payload is retained here.
fn health_text(text: &str, limit: usize) -> String {
    let clean: String = text.chars().filter(|c| !c.is_control()).collect();
    if clean.len() <= limit {
        return clean;
    }
    let mut end = limit.saturating_sub('…'.len_utf8());
    while !clean.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &clean[..end])
}
struct RuntimeCandidate {
    identity: crate::harness::runtime::RuntimeIdentity,
    domain: String,
    origin: crate::harness::evidence::EvidenceOrigin,
    contract: String,
    at: u64,
    local: Option<crate::store::harness_evidence::EvidenceRowV2>,
}
impl RuntimeCandidate {
    fn key(&self) -> (String, String, String, String) {
        (
            self.identity.key.clone(),
            self.domain.clone(),
            format!("{:?}", self.origin),
            self.contract.clone(),
        )
    }
}

fn line_for(harness: &str, version: &str, derived: &state::Derived) -> String {
    match &derived.state {
        State::Broken(broken) => state::broken_line(harness, version, broken),
        other => format!(
            "{harness} {version}: {} \u{2014} {}",
            state::state_word(other),
            state::source_text(other)
        ),
    }
}

fn harness_report(
    harness: &'static str,
    rows: &[EvidenceRow],
    manifest: &Manifest,
    now: u64,
    detected: Option<String>,
    unattributed: Option<UnattributedReport>,
    hook_parse_failures: u64,
) -> HarnessStateReport {
    let release = env!("CARGO_PKG_VERSION");
    let rollup: HarnessRollup = state::roll_up(harness, rows, manifest, release, now);
    let detected = detected
        .filter(|version| !rollup.versions.iter().any(|row| &row.version == version))
        .map(|version| {
            let (derived, _) = state::derive_detected(
                harness,
                &version,
                rollup.contract_id.as_deref(),
                rows,
                manifest,
                release,
            );
            DetectedVersion {
                state: state::state_word(&derived.state).to_owned(),
                line: state::detected_line(harness, &version, &derived),
                version,
            }
        });
    let versions = rollup
        .versions
        .iter()
        .take(HARNESS_STATE_VERSIONS)
        .map(|verdict| VersionStateReport {
            version: verdict.version.clone(),
            state: state::state_word(&verdict.derived.state).to_owned(),
            source: state::source_text(&verdict.derived.state),
            line: line_for(harness, &verdict.version, &verdict.derived),
            notes: verdict.derived.doctor_notes.clone(),
            issue_url: state::issue_url(&verdict.derived.state),
            last_seen_at: verdict.last_seen_at,
            in_health_window: verdict.in_health_window,
        })
        .collect();
    HarnessStateReport {
        harness: harness.to_owned(),
        contract_id: rollup.contract_id,
        detected,
        versions,
        unattributed,
        hook_parse_failures,
    }
}

/// The admission observer saw `detected` versions on `PATH`: a version with no
/// evidence row (any contract) and no manifest row asks for a manifest fetch.
/// Never blocks (the trigger only schedules a detached fetch); a store read
/// failure triggers nothing.
pub fn trigger_unseen_versions(
    store: &dyn StorePort,
    trigger: &dyn ManifestTrigger,
    manifest: &Manifest,
    detected: &[(&str, Option<String>)],
    budget: &CallBudget,
) {
    for (harness, version) in detected {
        let Some(version) = version else { continue };
        if manifest.has_row(harness, version) {
            continue;
        }
        let Ok(rows) = store.harness_evidence_all(harness, budget) else {
            continue;
        };
        if rows.iter().any(|row| &row.version == version) {
            continue;
        }
        trigger.ensure_manifest(
            harness,
            FetchReason::UnseenVersion {
                version: version.clone(),
            },
        );
    }
}
