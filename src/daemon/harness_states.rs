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

pub const HARNESSES: [&str; 2] = ["claude", "codex"];

/// The version of `harness` found on the daemon's `PATH` (canonical `X.Y.Z`),
/// as the admission observer last observed it.
pub type DetectedVersions = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// The manifest verdicts are derived against, read on every call (a fetch
/// may replace it).
pub type ManifestSource = Box<dyn Fn() -> Arc<Manifest> + Send + Sync>;

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
        }
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
        for harness in HARNESSES {
            all.push((harness, self.store.harness_evidence_all(harness, budget)?));
        }
        Ok(self.with_manifest(|manifest| {
            all.iter()
                .filter_map(|(harness, rows)| {
                    state::roll_up(harness, rows, manifest, env!("CARGO_PKG_VERSION"), now)
                        .health_line()
                })
                .collect()
        }))
    }

    /// The `harness.states` report.
    pub fn report(&self, budget: &CallBudget) -> Result<HarnessStatesReport, ApiError> {
        let now = self.now_ms();
        let mut harnesses = Vec::new();
        for harness in HARNESSES {
            let rows = self.store.harness_evidence_all(harness, budget)?;
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
            let detected =
                (self.detected)(harness).and_then(|raw| normalize_version(harness, &raw));
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
