//! ht-xoc.5: Health renders the version verdicts (`harness::state`) from the
//! evidence store and the manifest: nothing for working or new, one broken
//! line per harness, scoped to the newest contract and the last 24 hours.
//! Each test names the mutation it kills.

use super::health_budget::ready_inputs;
use super::*;
use crate::{
    app::claude_status,
    daemon::{
        harness_evidence::{HarnessEvidenceRecorder, ManifestTrigger},
        harness_states::HarnessStatesProvider,
    },
    harness::manifest::{FetchError, FetchOutcome, Fetcher, ManifestPolicy, ManifestService},
    harness::{
        claude,
        codex::VersionError,
        manifest::{self, FetchReason, Manifest},
        recipe::Version,
    },
    ports::StorePort,
    protocol::{
        commands::{HarnessEvidence, HarnessEvidenceOutcome},
        time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis},
    },
    store::{SqliteStore, StoreSettings, connection::StoreContext},
    test_support::isolation::TestIsolation,
};
use serde_json::json;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicI64, Ordering},
};

const HOUR_MS: i64 = 60 * 60 * 1000;
const T0: i64 = 1_000 * HOUR_MS;
const CONTRACT: &str = "0123456789abcdef";
const NEW_CONTRACT: &str = "fedcba9876543210";

struct TestClock(AtomicI64);
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(self.0.load(Ordering::SeqCst))
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(1)
    }
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(1_000_000),
        cancellation: Cancellation::default(),
    }
}

struct NoTrigger;
impl ManifestTrigger for NoTrigger {
    fn ensure_manifest(&self, _harness: &str, _reason: FetchReason) {}
}

struct Fx {
    _iso: TestIsolation,
    clock: Arc<TestClock>,
    store: Arc<SqliteStore>,
    manifest: Arc<Mutex<Arc<Manifest>>>,
    recorder: HarnessEvidenceRecorder,
}

impl Fx {
    fn new(label: &str) -> Self {
        let iso = TestIsolation::new(label);
        let clock = Arc::new(TestClock(AtomicI64::new(T0)));
        let store = Arc::new(
            SqliteStore::new(
                StoreContext::new(iso.state_root().join("store.db"), clock.clone()),
                "i",
                StoreSettings::default(),
            )
            .unwrap(),
        );
        let recorder = HarnessEvidenceRecorder::new(
            store.clone(),
            Some(Arc::new(NoTrigger) as Arc<dyn ManifestTrigger>),
            clock.clone(),
        );
        Self {
            _iso: iso,
            clock,
            store,
            manifest: Arc::new(Mutex::new(Arc::new(Manifest::default()))),
            recorder,
        }
    }

    fn advance(&self, ms: i64) {
        self.clock.0.fetch_add(ms, Ordering::SeqCst);
    }

    fn plant_manifest(&self, rows: serde_json::Value) {
        let bytes = serde_json::to_vec(&json!({
            "schema_version": 2, "generated_at": null, "latest_release": null,
            "contracts": {}, "rows": rows,
        }))
        .unwrap();
        *self.manifest.lock().unwrap() = Arc::new(manifest::parse(&bytes).unwrap());
    }

    /// One payload's evidence for claude (or codex) through the real recorder.
    fn record(
        &self,
        harness: &str,
        version: &str,
        contract: &str,
        event: &str,
        outcome: HarnessEvidenceOutcome,
    ) {
        self.recorder
            .record(
                &HarnessEvidence {
                    harness: harness.into(),
                    version: Some(version.into()),
                    unattributed_reason: None,
                    contract_id: contract.into(),
                    event: event.into(),
                    outcome,
                    session_id: None,
                },
                &budget(),
            )
            .unwrap();
    }

    fn verify(&self, harness: &str, version: &str, contract: &str) {
        self.record(
            harness,
            version,
            contract,
            "SessionStart",
            HarnessEvidenceOutcome::Ok,
        );
        self.record(
            harness,
            version,
            contract,
            "PreToolUse",
            HarnessEvidenceOutcome::Ok,
        );
    }

    fn violate(&self, harness: &str, version: &str, contract: &str) {
        self.record(
            harness,
            version,
            contract,
            "PreToolUse",
            HarnessEvidenceOutcome::Violation {
                field: "tool_input.command".into(),
            },
        );
    }

    fn provider(&self) -> HarnessStatesProvider {
        let manifest = Arc::clone(&self.manifest);
        HarnessStatesProvider::new(
            self.store.clone() as Arc<dyn StorePort>,
            Box::new(move || Arc::clone(&manifest.lock().unwrap())),
            self.clock.clone(),
            Box::new(|_| None),
            None,
        )
    }

    /// Health as the daemon assembles it from this store and manifest.
    fn health(&self) -> crate::protocol::results::Health {
        let mut inputs = ready_inputs();
        inputs.harness_version_lines = self.provider().health_lines(&budget()).unwrap();
        inputs.assemble()
    }
}

fn newer_version() -> String {
    let max = claude::RECIPES
        .iter()
        .map(|recipe| recipe.max_version())
        .max()
        .unwrap();
    format!("{}.{}.{}", max.major, max.minor, max.patch + 13)
}

fn listed_version() -> String {
    claude::RECIPES[0].max_version().to_string()
}

fn below_floor_version() -> String {
    let min = claude::RECIPES
        .iter()
        .map(|recipe| recipe.min_version())
        .min()
        .unwrap();
    Version::new(min.major, min.minor.saturating_sub(1), 0).to_string()
}

fn version_lines(health: &crate::protocol::results::Health) -> Vec<&String> {
    health
        .limitations
        .iter()
        .filter(|line| line.contains("contract input failure"))
        .collect()
}

/// Kills: a Health line for a working version (listed or locally verified) or
/// a new one (seen, not yet verified), and a degraded state for either.
#[test]
fn no_health_line_for_working_or_new() {
    let fx = Fx::new("hhs-working-new");
    let listed = listed_version();
    fx.verify("claude", &listed, CONTRACT);
    let unlisted = newer_version();
    // Seen but not verified: new. Then a second unlisted version, verified.
    fx.record(
        "claude",
        &unlisted,
        CONTRACT,
        "SessionStart",
        HarnessEvidenceOutcome::Ok,
    );
    fx.verify("codex", "999.0.0", CONTRACT);
    let health = fx.health();
    assert_eq!(
        health.state,
        HealthState::Healthy,
        "{:#?}",
        health.limitations
    );
    assert!(
        version_lines(&health).is_empty(),
        "{:#?}",
        health.limitations
    );
    assert!(
        health.notes.iter().all(|line| !line.contains(&unlisted)),
        "{:#?}",
        health.notes
    );
}

/// Kills: B6's optimistic note surviving in Health, and an unlisted newer
/// version refused (reported broken) because the ladder code is in place.
#[test]
fn unlisted_new_version_adds_no_health_line() {
    let version = newer_version();
    let mut inputs = ready_inputs();
    inputs.claude = claude_status(
        Some(Ok(version.clone())),
        crate::protocol::results::CapabilityState::Unsupported,
    );
    assert!(matches!(inputs.claude, HarnessStatus::Optimistic(_)));
    // No evidence at all: the version verdict for the PATH version is
    // doctor-only, so Health has no version line.
    let fx = Fx::new("hhs-unlisted");
    inputs.harness_version_lines = fx.provider().health_lines(&budget()).unwrap();
    let health = inputs.assemble();
    assert_eq!(health.state, HealthState::Healthy);
    assert!(health.limitations.is_empty(), "{:#?}", health.limitations);
    assert!(
        health.notes.iter().all(|line| !line.contains(&version)),
        "{:#?}",
        health.notes
    );
    // With an unverified row for it: still no line (new).
    fx.record(
        "claude",
        &version,
        CONTRACT,
        "SessionStart",
        HarnessEvidenceOutcome::Ok,
    );
    assert!(fx.health().limitations.is_empty());
}

/// Kills: a version refusal (known broken, below the floor) that still
/// degrades Health through the PATH observation instead of evidence.
#[test]
fn version_refusal_is_not_a_health_limitation() {
    let mut inputs = ready_inputs();
    inputs.claude = claude_status(
        Some(Err(VersionError::Unsupported(below_floor_version()))),
        crate::protocol::results::CapabilityState::Unsupported,
    );
    assert!(matches!(inputs.claude, HarnessStatus::VersionRefused(_)));
    let health = inputs.assemble();
    assert_eq!(
        health.state,
        HealthState::Healthy,
        "{:#?}",
        health.limitations
    );
    assert!(health.limitations.is_empty(), "{:#?}", health.limitations);
    // An unobservable binary stays a limitation: it blocks the hook.
    let mut inputs = ready_inputs();
    inputs.claude = claude_status(
        Some(Err(VersionError::Unavailable)),
        crate::protocol::results::CapabilityState::Unsupported,
    );
    let health = inputs.assemble();
    assert_eq!(health.state, HealthState::Degraded);
    assert!(
        health
            .limitations
            .iter()
            .any(|l| l.starts_with("harness claude unsupported: "))
    );
}

fn broken_manifest_row(version: &str, extra: serde_json::Value) -> serde_json::Value {
    let mut row = json!({
        "harness": "claude", "version": version, "status": "known_broken",
        "evidence": "live", "contract_id": CONTRACT, "source": "canary",
        "broken_event": "PreToolUse", "broken_field": "tool_input.command",
    });
    for (key, value) in extra.as_object().unwrap() {
        row[key] = value.clone();
    }
    row
}

// Historical manifest advice stays in the report and cannot degrade core Health.
#[test]
fn manifest_history_is_advisory_for_every_release_pointer() {
    let version = newer_version();
    for extra in [
        json!({"last_working": "2.1.283"}),
        json!({"issue_url": "https://example.test/issues/9"}),
    ] {
        let fx = Fx::new("hhs-advisory-manifest");
        fx.record(
            "claude",
            &version,
            CONTRACT,
            "SessionStart",
            HarnessEvidenceOutcome::Ok,
        );
        fx.plant_manifest(json!([broken_manifest_row(&version, extra)]));
        assert!(fx.health().limitations.is_empty());
        assert_eq!(fx.health().state, HealthState::Healthy);
        assert_eq!(
            fx.provider().report(&budget()).unwrap().harnesses[0].versions[0].state,
            "broken"
        );
    }
}

/// Kills: a local violation that is not a Health line, one that is shown for
/// a version with no session in the last 24 hours, and a window that never
/// reopens when the version is used again.
#[test]
fn old_version_without_session_in_24h_drops_out() {
    let fx = Fx::new("hhs-window");
    let version = newer_version();
    fx.record(
        "claude",
        &version,
        CONTRACT,
        "SessionStart",
        HarnessEvidenceOutcome::Ok,
    );
    fx.violate("claude", &version, CONTRACT);
    let health = fx.health();
    assert_eq!(version_lines(&health).len(), 1, "{:#?}", health.limitations);
    assert!(version_lines(&health)[0].starts_with(&format!(
        "harness claude {version} contract input failure: PreToolUse/tool_input.command"
    )));
    fx.advance(24 * HOUR_MS + 1);
    assert!(
        version_lines(&fx.health()).is_empty(),
        "{:#?}",
        fx.health().limitations
    );
    // The row is still there (doctor lists it) and a new session brings it back.
    fx.record(
        "claude",
        &version,
        CONTRACT,
        "PostToolUse",
        HarnessEvidenceOutcome::Ok,
    );
    assert_eq!(version_lines(&fx.health()).len(), 1);
}

/// Kills: Health reading an older contract's rows after a contract change.
#[test]
fn newest_contract_row_decides_after_contract_change() {
    let fx = Fx::new("hhs-contract");
    let version = newer_version();
    // Old contract: broken. Later the harness' hooks move to a new contract
    // that verifies the same version.
    fx.violate("claude", &version, CONTRACT);
    assert_eq!(version_lines(&fx.health()).len(), 1);
    fx.advance(HOUR_MS);
    fx.verify("claude", &version, NEW_CONTRACT);
    let health = fx.health();
    assert!(
        version_lines(&health).is_empty(),
        "{:#?}",
        health.limitations
    );
    assert_eq!(health.state, HealthState::Healthy);
    // And the other way round: a violation under the newest contract decides.
    fx.advance(HOUR_MS);
    fx.violate("claude", &version, "aaaaaaaaaaaaaaaa");
    assert_eq!(version_lines(&fx.health()).len(), 1);
}

/// Kills: a downgrade that leaves the newer contract's rows deciding Health
/// (contract choice by first_seen instead of most recent use).
#[test]
fn downgrade_back_to_an_older_contract_decides_health() {
    let fx = Fx::new("hhs-downgrade");
    let version = newer_version();
    fx.verify("claude", &version, NEW_CONTRACT);
    assert!(version_lines(&fx.health()).is_empty());
    fx.advance(HOUR_MS);
    // The older binary's hooks send the older contract again and it breaks.
    fx.violate("claude", &version, CONTRACT);
    assert_eq!(version_lines(&fx.health()).len(), 1);
}

// A below-floor historical version is advisory; its actual callbacks still count.
#[test]
fn below_floor_evidence_does_not_degrade_core_health() {
    let fx = Fx::new("hhs-floor");
    fx.record(
        "claude",
        &below_floor_version(),
        CONTRACT,
        "SessionStart",
        HarnessEvidenceOutcome::Ok,
    );
    let health = fx.health();
    assert!(health.limitations.is_empty());
    assert_eq!(health.state, HealthState::Healthy);
    health.validate().unwrap();
}

/// Kills: more than one line per harness, the older broken version winning,
/// and one harness hiding the other's line.
#[test]
fn one_line_per_harness() {
    let fx = Fx::new("hhs-one-line");
    let first = newer_version();
    let second = Version::parse(&first).map(|v| Version::new(v.major, v.minor, v.patch + 1));
    let second = second.unwrap().to_string();
    fx.violate("claude", &first, CONTRACT);
    fx.advance(HOUR_MS);
    fx.violate("claude", &second, CONTRACT);
    fx.violate("codex", "999.0.0", CONTRACT);
    let health = fx.health();
    let lines = version_lines(&health);
    assert_eq!(lines.len(), 2, "{lines:#?}");
    assert!(
        lines[0].starts_with(&format!("harness claude {second} contract input failure: ")),
        "{lines:#?}"
    );
    assert!(
        lines[1].starts_with("harness codex 999.0.0 contract input failure: "),
        "{lines:#?}"
    );
}

/// Kills: version lines that push the worst case past the budget or are
/// folded away before the lower-priority lines.
#[test]
fn worst_case_budget_holds_with_two_broken_lines() {
    let mut inputs = super::health_budget::worst_case_inputs();
    inputs.harness_version_lines = vec![
        format!("harness claude 9.9.9 broken: {}", "x".repeat(400)),
        format!("harness codex 9.9.9 broken: {}", "y".repeat(400)),
    ];
    let health = inputs.assemble();
    assert!(
        health.limitations.len() <= HEALTH_LINE_BUDGET,
        "{:#?}",
        health.limitations
    );
    health.validate().expect("the wire cap still holds");
    for harness in ["claude", "codex"] {
        assert!(
            health
                .limitations
                .iter()
                .any(|line| line.starts_with(&format!("harness {harness} 9.9.9 broken: "))),
            "{:#?}",
            health.limitations
        );
    }
}

struct CountingFetcher(Mutex<Vec<String>>);
impl Fetcher for CountingFetcher {
    fn fetch(&self, url: &str, _etag: Option<&str>) -> Result<FetchOutcome, FetchError> {
        self.0.lock().unwrap().push(url.to_owned());
        Err(FetchError::Offline)
    }
}

/// Kills: a detected version that never asks for a manifest (the first fetch
/// would wait for a payload), one that asks although a row already exists
/// (evidence under any contract, or a manifest row), a missing version that
/// asks, and an observer that waits for the fetch.
#[test]
fn admission_observer_trigger_fetches_only_for_an_unseen_version() {
    let fx = Fx::new("hhs-trigger");
    let fetcher = Arc::new(CountingFetcher(Mutex::new(Vec::new())));
    let cache = fx._iso.state_root().join("harness-manifest");
    let service = Arc::new(ManifestService::new(
        cache,
        ManifestPolicy::Auto,
        fetcher.clone(),
        fx.clock.clone(),
        Arc::new(|_: &str| {}),
    ));
    let store = fx.store.clone();
    let trigger = |detected: &[(&str, Option<String>)]| {
        crate::daemon::harness_states::trigger_unseen_versions(
            store.as_ref(),
            service.as_ref(),
            &service.current(),
            detected,
            &budget(),
        );
        assert!(service.wait_idle(std::time::Duration::from_secs(20)));
    };
    let fetches = || fetcher.0.lock().unwrap().len();

    trigger(&[("claude", None), ("codex", None)]);
    assert_eq!(fetches(), 0, "no detected version: nothing to ask about");

    // A version with a local row (a different contract counts too).
    fx.record(
        "claude",
        "7.7.7",
        NEW_CONTRACT,
        "SessionStart",
        HarnessEvidenceOutcome::Ok,
    );
    trigger(&[("claude", Some("7.7.7".into()))]);
    assert_eq!(fetches(), 0, "an evidence row exists");

    // A version the embedded manifest has a row for.
    let embedded_version = manifest::embedded()
        .rows
        .iter()
        .find(|row| row.harness == "claude")
        .map(|row| row.version.clone())
        .expect("the embedded manifest lists claude versions");
    trigger(&[("claude", Some(embedded_version))]);
    assert_eq!(fetches(), 0, "the manifest has a row");

    trigger(&[("claude", Some("8.8.8".into()))]);
    assert_eq!(fetches(), 1, "an unseen version asks once");
}

/// Kills: a report that lacks the verdict source, the unattributed reason or
/// the parse-failure count, lists more than 20 versions, orders them by
/// version instead of recency, or evaluates a PATH version that already has a
/// row (doctor would print it twice).
#[test]
fn report_carries_sources_unattributed_reason_and_caps_versions() {
    use crate::daemon::logs::{HookParseFailures, RateLimitedLaneLog};
    let fx = Fx::new("hhs-report");
    let version = newer_version();
    fx.record(
        "claude",
        &version,
        CONTRACT,
        "SessionStart",
        HarnessEvidenceOutcome::Ok,
    );
    fx.advance(HOUR_MS);
    fx.violate("claude", &version, CONTRACT);
    for minor in 0..25 {
        fx.advance(1);
        fx.verify("claude", &format!("3.{minor}.0"), CONTRACT);
    }
    fx.store
        .record_unattributed("claude", "no transcript entry yet", &budget())
        .unwrap();
    let failures = Arc::new(HookParseFailures::new(Arc::new(RateLimitedLaneLog::new(
        fx.clock.clone(),
        Arc::new(|_: &str| {}),
    ))));
    failures.record("claude", "Invalid");
    failures.record("claude", "Invalid");
    let manifest = Arc::clone(&fx.manifest);
    let version_for_detected = version.clone();
    let provider = HarnessStatesProvider::new(
        fx.store.clone() as Arc<dyn StorePort>,
        Box::new(move || Arc::clone(&manifest.lock().unwrap())),
        fx.clock.clone(),
        Box::new(move |harness| (harness == "claude").then(|| version_for_detected.clone())),
        Some(failures),
    );
    let report = provider.report(&budget()).unwrap();
    let claude = &report.harnesses[0];
    assert_eq!(
        claude.versions.len(),
        crate::protocol::results::HARNESS_STATE_VERSIONS
    );
    assert!(claude.detected.is_none(), "the PATH version has a row");
    assert_eq!(claude.hook_parse_failures, 2);
    let unattributed = claude.unattributed.as_ref().unwrap();
    assert_eq!(unattributed.reason, "no transcript entry yet");
    assert!(
        claude
            .versions
            .windows(2)
            .all(|pair| pair[0].last_seen_at >= pair[1].last_seen_at),
        "newest last_seen first"
    );
    let newest = &claude.versions[0];
    assert_eq!(newest.state, "working");
    assert_eq!(newest.source, "local evidence (lifecycle + tool payloads)");
    assert_eq!(
        newest.line,
        format!(
            "claude {}: working \u{2014} {}",
            newest.version, newest.source
        )
    );
    // The violated version is older than the 25 verified ones, so it is cut
    // by the cap; ask for it alone on a fresh harness to see its verdict.
    let fx2 = Fx::new("hhs-report-broken");
    fx2.record(
        "claude",
        &version,
        CONTRACT,
        "SessionStart",
        HarnessEvidenceOutcome::Ok,
    );
    fx2.violate("claude", &version, CONTRACT);
    let report = fx2.provider().report(&budget()).unwrap();
    let broken = &report.harnesses[0].versions[0];
    assert_eq!(broken.state, "broken");
    assert_eq!(
        broken.source,
        "local evidence: violation in PreToolUse/tool_input.command"
    );
    assert!(broken.line.starts_with(&format!(
        "harness claude {version} contract input failure: "
    )));
    assert!(broken.in_health_window);
    assert_eq!(
        broken.issue_url.as_deref(),
        Some(crate::harness::admission::ISSUES_URL)
    );
}

/// Kills: a store read error leaving Health healthy and silent about the
/// missing version evidence.
#[test]
fn failing_evidence_store_shows_the_unavailable_limitation() {
    use crate::daemon::health::{HARNESS_EVIDENCE_UNAVAILABLE_LINE, harness_version_lines};
    use crate::protocol::results::{ApiError, ErrorCode};

    let lines = harness_version_lines(&Err(ApiError::new(ErrorCode::StoreBusy, "boom")));
    assert_eq!(lines, vec![HARNESS_EVIDENCE_UNAVAILABLE_LINE.to_owned()]);
    let mut inputs = ready_inputs();
    inputs.harness_version_lines = lines;
    let health = inputs.assemble();
    assert!(
        health
            .limitations
            .iter()
            .any(|line| line == HARNESS_EVIDENCE_UNAVAILABLE_LINE),
        "{:#?}",
        health.limitations
    );
    assert_eq!(health.state, HealthState::Degraded);

    // Provider level: a real store whose evidence table cannot be read.
    let fx = Fx::new("hhs-unavailable");
    fx.verify("claude", &listed_version(), CONTRACT);
    let db = rusqlite::Connection::open(fx._iso.state_root().join("store.db")).unwrap();
    db.execute_batch("DROP TABLE harness_version_evidence")
        .unwrap();
    let answer = fx.provider().health_lines(&budget());
    assert!(answer.is_err(), "{answer:?}");
    assert_eq!(
        harness_version_lines(&answer),
        vec![HARNESS_EVIDENCE_UNAVAILABLE_LINE.to_owned()]
    );

    // And a readable store passes its lines through unchanged.
    let ok: Result<Vec<String>, ApiError> = Ok(vec!["x".into()]);
    assert_eq!(harness_version_lines(&ok), vec!["x".to_owned()]);
}

// Catches missing v2 provider, cached install becoming profile callback evidence,
// loss of exact build identity, and stale/local violation verdict promotion.
#[test]
fn health_v2_cached_install_and_exact_runtime_scopes_remain_separate() {
    use crate::{
        harness::{
            registry,
            runtime::{RuntimeDescriptor, RuntimeIdentity},
        },
        protocol::results::{HarnessHealthScope, RuntimeEvidenceState},
        store::harness_evidence::{EvidenceOutcome, EvidenceRecordV2},
    };
    let fx = Fx::new("hhs-v2-exact");
    let registration = registry::builtins()
        .by_id(registry::builtins().agent("claude").unwrap())
        .unwrap();
    let descriptor = &registration.contracts()[0];
    let identity = RuntimeIdentity::build(RuntimeDescriptor {
        release_version: Some("2.1.286".into()),
        source: "fixture".into(),
        base_version: Some("2.1.286".into()),
        derived_version: Some("2.1.286+7.gabcdef0".into()),
        commit: Some("a".repeat(40)),
        dirty: Some(false),
        distance: Some(7),
    })
    .unwrap();
    for event in ["SessionStart", "PreToolUse"] {
        fx.store
            .record_harness_evidence_v2(
                &EvidenceRecordV2 {
                    identity: &identity,
                    descriptor,
                    event,
                    outcome: &EvidenceOutcome::Ok,
                    qualified: true,
                },
                &budget(),
            )
            .unwrap();
    }
    let provider = fx.provider().with_observations(Box::new(|| {
        Ok([(
            "claude".into(),
            crate::harness::adapter::DaemonObservation {
                status: HarnessStatus::Cooperative {
                    detail: "installed listed".into(),
                    live_unverified: false,
                },
                ..Default::default()
            },
        )]
        .into())
    }));
    let report = provider
        .report_v2(&budget())
        .expect("implemented rich health provider");
    let entry = &report.harnesses["claude"];
    assert_eq!(entry.scope, HarnessHealthScope::daemon_default());
    assert_eq!(
        entry.enablement.state,
        crate::protocol::results::EnablementState::Unknown
    );
    assert_eq!(
        entry.callback_observation.state,
        crate::protocol::results::CallbackObservationState::Unknown,
        "aggregate runtime evidence cannot prove default-profile callback"
    );
    assert_eq!(entry.runtime_evidence.len(), 1);
    let row = &entry.runtime_evidence[0];
    assert_eq!(row.identity, identity);
    assert_eq!(row.state, RuntimeEvidenceState::Working);
    assert_eq!(row.scope, HarnessHealthScope::all_runtime_scopes());
    assert!(
        row.source.contains("unclassified"),
        "local milestone success cannot manufacture no_model/live stage"
    );
    assert!(
        provider.report(&budget()).unwrap().harnesses[0]
            .versions
            .is_empty(),
        "build-only rich row leaked into legacy semver collection"
    );
    fx.store
        .record_harness_evidence_v2(
            &EvidenceRecordV2 {
                identity: &identity,
                descriptor,
                event: "PreToolUse",
                outcome: &EvidenceOutcome::Violation {
                    field: "tool_input.command".into(),
                },
                qualified: true,
            },
            &budget(),
        )
        .unwrap();
    assert_eq!(
        provider.report_v2(&budget()).unwrap().harnesses["claude"].runtime_evidence[0].state,
        RuntimeEvidenceState::Broken
    );
    let lines = provider.health_lines(&budget()).unwrap();
    assert_eq!(
        lines.len(),
        1,
        "rich broken runtime must reach actual overall Health lines"
    );
    assert!(lines[0].contains(&identity.key));
    let mut overall = ready_inputs();
    overall.harness_version_lines = lines;
    assert_eq!(overall.assemble().state, HealthState::Degraded);
}

// Catches the public display cap hiding a retained current-window rich violation.
#[test]
fn health_v2_broken_twenty_first_row_still_degrades_overall_health() {
    use crate::{
        harness::{registry, runtime::RuntimeIdentity},
        protocol::results::RuntimeEvidenceState,
        store::harness_evidence::{EvidenceOutcome, EvidenceRecordV2},
    };
    let fx = Fx::new("hhs-v2-broken-outside-cap");
    let registration = registry::builtins()
        .by_id(registry::builtins().agent("claude").unwrap())
        .unwrap();
    let descriptor = &registration.contracts()[0];
    let broken = RuntimeIdentity::stable_release("9.0.0", "fixture").unwrap();
    fx.store
        .record_harness_evidence_v2(
            &EvidenceRecordV2 {
                identity: &broken,
                descriptor,
                event: "PreToolUse",
                outcome: &EvidenceOutcome::Violation {
                    field: "tool_input.command".into(),
                },
                qualified: true,
            },
            &budget(),
        )
        .unwrap();
    for minor in 1..=20 {
        fx.advance(1);
        let identity = RuntimeIdentity::stable_release(&format!("9.{minor}.0"), "fixture").unwrap();
        fx.store
            .record_harness_evidence_v2(
                &EvidenceRecordV2 {
                    identity: &identity,
                    descriptor,
                    event: "PreToolUse",
                    outcome: &EvidenceOutcome::Ok,
                    qualified: true,
                },
                &budget(),
            )
            .unwrap();
    }
    let provider = fx
        .provider()
        .with_observations(Box::new(|| Ok(Default::default())));
    let report = provider.report_v2(&budget()).unwrap();
    let rows = &report.harnesses["claude"].runtime_evidence;
    assert_eq!(rows.len(), 20, "public report must remain capped");
    assert!(
        rows.iter()
            .all(|row| row.state != RuntimeEvidenceState::Broken)
    );
    assert!(
        provider.report(&budget()).unwrap().harnesses[0]
            .versions
            .is_empty()
    );
    let mut overall = ready_inputs();
    overall.harness_version_lines = provider.health_lines(&budget()).unwrap();
    let health = overall.assemble();
    assert_eq!(
        health.state,
        HealthState::Degraded,
        "display truncation must not erase a retained rich violation"
    );
    assert!(
        health
            .limitations
            .iter()
            .any(|line| line.contains("9.0.0") && line.contains("broken"))
    );
}

// Catches a generic registered installation failure silently leaving old Health healthy.
#[test]
fn generic_cached_install_failure_degrades_overall_health_with_bounded_note() {
    let mut inputs = ready_inputs();
    inputs.additional_harnesses = vec![("fourth".into(), HarnessStatus::Refused("x".repeat(2000)))];
    let health = inputs.assemble();
    assert_eq!(
        health.state,
        HealthState::Degraded,
        "registered adapter failure cannot be omitted from overall health"
    );
    assert!(
        health
            .limitations
            .iter()
            .any(|line| line.starts_with("harness fourth unsupported: "))
    );
    health.validate().unwrap();
    let mut inputs = ready_inputs();
    inputs.additional_harnesses = vec![(
        "fourth".into(),
        HarnessStatus::NotInstalled("absent".into()),
    )];
    let health = inputs.assemble();
    assert_eq!(health.state, HealthState::Healthy);
    assert!(
        health
            .notes
            .iter()
            .any(|line| line.starts_with("harness fourth not installed:"))
    );
}

// Catches fixed-pair iteration, native discovery in a health request, cross-domain
// milestone credit and converting rich-only build metadata into old semver wires.
#[test]
fn health_v2_fourth_adapter_is_cached_and_domains_have_no_legacy_projection() {
    use crate::{
        app::{AdmissionReobserver, health_v2_observer_tests::counting_with_contracts},
        harness::{
            adapter::{ContractDomain, InstallEnvironment},
            evidence::EvidenceOrigin,
            registry::{Registration, Registry},
            runtime::{RuntimeDescriptor, RuntimeIdentity},
        },
        protocol::{
            results::{CommandResult, RuntimeEvidenceState},
            wire::{PROTOCOL_VERSION, WireResponse},
        },
        store::harness_evidence::{EvidenceOutcome, EvidenceRecordV2},
    };
    let fx = Fx::new("hhs-v2-fourth");
    let original = crate::harness::registry::builtins().registrations()[0].contracts()[0];
    let mut native = original;
    native.contract = Box::leak(Box::new(crate::harness::contract::HarnessContract {
        harness: "fourth",
        ..*original.contract
    }));
    native.domain_id = "native_shape";
    native.origin = EvidenceOrigin::NativeShapeObservation;
    let mut bridge = native;
    bridge.domain_id = "bridge_envelope";
    bridge.origin = EvidenceOrigin::BridgeEnvelope;
    bridge.domain = ContractDomain::Bridge;
    let contracts = Box::leak(vec![native, bridge].into_boxed_slice());
    let adapter = counting_with_contracts("fourth", contracts);
    let registry = Box::leak(Box::new(
        Registry::new(Box::leak(
            vec![Registration::new(adapter)].into_boxed_slice(),
        ))
        .unwrap(),
    ));
    let observer = AdmissionReobserver::with_registry(
        registry,
        InstallEnvironment {
            clock: fx.clock.clone(),
            path: None,
            config_root: None,
            state_dir: None,
        },
        std::time::Duration::from_millis(50),
        Arc::new(|_| {}),
    );
    let cached = observer.pass(&Cancellation::default()).entries;
    let identity = RuntimeIdentity::build(RuntimeDescriptor {
        release_version: Some("2.1.286".into()),
        source: "fixture".into(),
        base_version: None,
        derived_version: None,
        commit: None,
        dirty: None,
        distance: None,
    })
    .unwrap();
    for (descriptor, events) in [
        (&contracts[0], &["SessionStart", "PreToolUse"][..]),
        (&contracts[1], &["SessionStart"][..]),
    ] {
        for event in events {
            fx.store
                .record_harness_evidence_v2(
                    &EvidenceRecordV2 {
                        identity: &identity,
                        descriptor,
                        event,
                        outcome: &EvidenceOutcome::Ok,
                        qualified: true,
                    },
                    &budget(),
                )
                .unwrap();
        }
    }
    let provider = fx
        .provider()
        .with_registry(registry)
        .with_observations(Box::new(move || Ok(cached.clone())));
    let report = provider.report_v2(&budget()).unwrap();
    provider.report_v2(&budget()).unwrap();
    assert_eq!(
        adapter.calls.load(Ordering::SeqCst),
        1,
        "actual health requests must never invoke native observation"
    );
    let entry = &report.harnesses["fourth"];
    let native = entry
        .runtime_evidence
        .iter()
        .find(|r| r.domain == "native_shape")
        .unwrap();
    let bridge = entry
        .runtime_evidence
        .iter()
        .find(|r| r.domain == "bridge_envelope")
        .unwrap();
    assert_eq!(native.state, RuntimeEvidenceState::Working);
    assert_eq!(bridge.state, RuntimeEvidenceState::New);
    assert_eq!(
        entry.callback_observation.state,
        crate::protocol::results::CallbackObservationState::Unknown
    );
    let legacy = provider.report(&budget()).unwrap();
    assert_eq!(legacy.harnesses.len(), 1);
    assert_eq!(
        serde_json::to_value(&legacy).unwrap(),
        json!({"harnesses":[{"harness":"fourth","contract_id":null,"detected":null,"versions":[],"unattributed":null,"hook_parse_failures":0}]})
    );
    let response = WireResponse {
        version: PROTOCOL_VERSION,
        request_id: "fourth".into(),
        instance: uuid::Uuid::new_v4().to_string(),
        daemon_boot: uuid::Uuid::new_v4().to_string(),
        result: Ok(CommandResult::HarnessHealthV2(report)),
    };
    assert_eq!(
        serde_json::from_str::<WireResponse>(&serde_json::to_string(&response).unwrap()).unwrap(),
        response
    );
}

// Catches an unreadable bounded store or poisoned cache being fabricated as empty Working.
#[test]
fn health_v2_store_cache_and_expired_budget_fail_explicitly() {
    let fx = Fx::new("hhs-v2-unavailable");
    let provider = fx
        .provider()
        .with_observations(Box::new(|| Ok(Default::default())));
    let mut expired = budget();
    expired.deadline = MonoInstant(1);
    assert_eq!(
        provider.report_v2(&expired).unwrap_err().code,
        crate::protocol::results::ErrorCode::DeadlineExceeded
    );
    let poisoned = fx.provider().with_observations(Box::new(|| {
        Err(crate::protocol::results::ApiError::new(
            crate::protocol::results::ErrorCode::StoreBusy,
            "cached observations unavailable",
        ))
    }));
    assert!(poisoned.report_v2(&budget()).is_err());
    let db = rusqlite::Connection::open(fx._iso.state_root().join("store.db")).unwrap();
    db.execute_batch("DROP TABLE harness_contract_evidence_v2")
        .unwrap();
    assert!(provider.report_v2(&budget()).is_err());
    assert!(provider.health_lines(&budget()).is_err());
}

fn health_runtime_manifest(
    identity: &crate::harness::runtime::RuntimeIdentity,
    stale: bool,
    stage: &str,
) -> Manifest {
    let descriptor = crate::harness::registry::builtins().registrations()[0].contracts()[0];
    let contract = if stale {
        "0000000000000000".into()
    } else {
        descriptor.contract_id_v2().unwrap()
    };
    let known_broken = stage == "source_captured";
    manifest::parse(&serde_json::to_vec(&json!({
        "schema_version":2, "generated_at":null, "latest_release":null, "contracts":{}, "rows":[],
        "runtime_contracts":{"claude":[{"domain":descriptor.domain_id,"origin":descriptor.origin,"id":contract,
            "events":descriptor.events.iter().map(|e| json!({"event":e.native_event,"milestone":e.milestone,"always_send":e.always_send})).collect::<Vec<_>>(), "required_milestones":descriptor.required_milestones}]},
        "runtime_rows":[{"harness":"claude","identity":identity,"domain":descriptor.domain_id,"origin":descriptor.origin,"contract_id":contract,
            "status":if known_broken {"known_broken"} else {"verified"}, "evidence_stage":stage,"source":"manual", "required_milestones":descriptor.required_milestones,"successful_milestones":if known_broken {vec![]} else {descriptor.required_milestones.to_vec()}, "broken_event":known_broken.then_some("PreToolUse"),"broken_field":known_broken.then_some("tool_name"),"supported_since":null,"issue_url":null,"last_seen_at":T0 as u64}]
    })).unwrap()).unwrap()
}

// Catches historical manifest collection being used as an authority, losing stage/source,
// or promoting exact build release text into recipe/legacy facts.
#[test]
fn health_v2_manifest_verdicts_require_current_descriptor_and_keep_evidence_stage() {
    use crate::{
        harness::runtime::{RuntimeDescriptor, RuntimeIdentity},
        protocol::results::RuntimeEvidenceState,
    };
    let fx = Fx::new("hhs-v2-manifest");
    let identity = RuntimeIdentity::build(RuntimeDescriptor {
        release_version: Some("2.1.286".into()),
        source: "fixture".into(),
        base_version: None,
        derived_version: None,
        commit: None,
        dirty: None,
        distance: None,
    })
    .unwrap();
    let provider = fx
        .provider()
        .with_observations(Box::new(|| Ok(Default::default())));
    for (stage, expected) in [
        ("source_captured", RuntimeEvidenceState::Broken),
        ("no_model", RuntimeEvidenceState::Working),
        ("live", RuntimeEvidenceState::Working),
    ] {
        *fx.manifest.lock().unwrap() = Arc::new(health_runtime_manifest(&identity, false, stage));
        let report = provider.report_v2(&budget()).unwrap();
        let row = &report.harnesses["claude"].runtime_evidence[0];
        assert_eq!(row.identity, identity);
        assert_eq!(row.state, expected);
        assert_eq!(row.source, format!("manifest manual ({stage})"));
    }
    *fx.manifest.lock().unwrap() = Arc::new(health_runtime_manifest(&identity, true, "no_model"));
    let report = provider.report_v2(&budget()).unwrap();
    let row = &report.harnesses["claude"].runtime_evidence[0];
    assert_eq!(
        row.state,
        RuntimeEvidenceState::Unavailable,
        "historical/stale descriptor row cannot be Working"
    );
    assert_eq!(row.identity, identity);
    assert!(
        provider.report(&budget()).unwrap().harnesses[0]
            .versions
            .is_empty()
    );
}

// Catches unbounded rollup, time-order reversal and unstable equal-time tie ordering.
#[test]
fn health_v2_runtime_rows_are_bounded_newest_then_identity_domain_contract() {
    use crate::{
        harness::{registry, runtime::RuntimeIdentity},
        store::harness_evidence::{EvidenceOutcome, EvidenceRecordV2},
    };
    let fx = Fx::new("hhs-v2-cap-rows");
    let descriptor = &registry::builtins().registrations()[0].contracts()[0];
    let mut newest = String::new();
    for patch in 0..25 {
        let identity =
            RuntimeIdentity::stable_release(&format!("999.0.{patch}"), "native_transcript")
                .unwrap();
        fx.advance(1);
        fx.store
            .record_harness_evidence_v2(
                &EvidenceRecordV2 {
                    identity: &identity,
                    descriptor,
                    event: "SessionStart",
                    outcome: &EvidenceOutcome::Ok,
                    qualified: true,
                },
                &budget(),
            )
            .unwrap();
        newest = identity.key;
    }
    let provider = fx
        .provider()
        .with_observations(Box::new(|| Ok(Default::default())));
    let report = provider.report_v2(&budget()).unwrap();
    let rows = &report.harnesses["claude"].runtime_evidence;
    assert_eq!(rows.len(), 20);
    assert_eq!(rows[0].identity.key, newest);
    assert_eq!(rows[19].identity.key, "release:999.0.5");
    for version in ["999.1.1", "999.1.0"] {
        let identity = RuntimeIdentity::stable_release(version, "native_transcript").unwrap();
        fx.store
            .record_harness_evidence_v2(
                &EvidenceRecordV2 {
                    identity: &identity,
                    descriptor,
                    event: "SessionStart",
                    outcome: &EvidenceOutcome::Ok,
                    qualified: true,
                },
                &budget(),
            )
            .unwrap();
    }
    let report = provider.report_v2(&budget()).unwrap();
    let rows = &report.harnesses["claude"].runtime_evidence;
    assert_eq!(rows[0].identity.key, "release:999.0.24");
    assert_eq!(rows[1].identity.key, "release:999.1.0");
    assert_eq!(rows[2].identity.key, "release:999.1.1");
}

// Catches a cooperative note inventing runtime recipe admission.
#[test]
fn task3_versionless_health_cooperation_does_not_claim_runtime_admission() {
    let mut inputs = ready_inputs();
    inputs.claude = HarnessStatus::ContractDeclared {
        detail: "contract_declared; runtime metadata unavailable".into(),
    };
    let health = inputs.assemble();
    assert_eq!(
        health.harness.claude,
        crate::protocol::results::HarnessState::Cooperative
    );
    assert_eq!(health.state, HealthState::Healthy);
    assert!(health.limitations.is_empty());
    let note = health
        .notes
        .iter()
        .find(|line| line.starts_with("receipt cooperative:"))
        .unwrap();
    assert!(note.contains("cooperative_top_level"));
    assert!(
        !note.contains("admitted recipe") && !note.contains("admitted:"),
        "{note}"
    );
    assert!(
        health
            .notes
            .iter()
            .any(|line| line.contains("runtime metadata unavailable"))
    );
}

// Catches manifest/floor diagnoses becoming operational Health refusals.
#[test]
fn task3_versionless_historical_verdicts_are_advisory_but_payload_failures_matter() {
    let fx = Fx::new("task3-advisory");
    let version = below_floor_version();
    fx.record(
        "claude",
        &version,
        CONTRACT,
        "SessionStart",
        HarnessEvidenceOutcome::Ok,
    );
    fx.plant_manifest(json!([broken_manifest_row(
        &version,
        json!({"last_working": "2.1.283"})
    )]));
    assert!(fx.provider().health_lines(&budget()).unwrap().is_empty());
    assert_eq!(
        fx.provider().report(&budget()).unwrap().harnesses[0]
            .versions
            .len(),
        1
    );
    fx.violate("claude", &version, CONTRACT);
    assert!(
        fx.provider()
            .health_lines(&budget())
            .unwrap()
            .iter()
            .any(|line| line.contains("tool_input.command"))
    );
}

// Catches inventing a session key for invalid payloads and accidental verification.
#[test]
fn task3_versionless_missing_empty_session_and_malformed_never_create_identity() {
    let fx = Fx::new("task3-no-identity");
    for session in [None, Some("")] {
        let note = HarnessEvidence {
            harness: "codex".into(),
            version: None,
            unattributed_reason: Some("runtime metadata unavailable".into()),
            contract_id: CONTRACT.into(),
            event: "PreToolUse".into(),
            outcome: HarnessEvidenceOutcome::Violation {
                field: "session_id".into(),
            },
            session_id: session.map(str::to_owned),
        };
        assert!(!fx.recorder.record(&note, &budget()).unwrap());
    }
    let note = HarnessEvidence {
        harness: "codex".into(),
        version: None,
        unattributed_reason: Some("runtime metadata unavailable".into()),
        contract_id: CONTRACT.into(),
        event: "PreToolUse".into(),
        outcome: HarnessEvidenceOutcome::Malformed,
        session_id: Some("s".into()),
    };
    assert!(!fx.recorder.record(&note, &budget()).unwrap());
    let db = rusqlite::Connection::open(fx._iso.state_root().join("store.db")).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM harness_contract_diagnostics",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert!(
        fx.store
            .harness_evidence_all("codex", &budget())
            .unwrap()
            .is_empty()
    );
}

// Catches loss across daemon/store reopen and success overwriting the first failure.
#[test]
fn task3_versionless_violation_survives_reopen_and_success_without_version_rows() {
    let fx = Fx::new("task3-sticky");
    let mut note = HarnessEvidence {
        harness: "codex".into(),
        version: None,
        unattributed_reason: Some("runtime metadata unavailable".into()),
        contract_id: CONTRACT.into(),
        event: "PreToolUse".into(),
        outcome: HarnessEvidenceOutcome::Violation {
            field: "session_id".into(),
        },
        session_id: Some("session-a".into()),
    };
    assert!(!fx.recorder.record(&note, &budget()).unwrap());
    fx.advance(1000);
    note.event = "PostToolUse".into();
    note.outcome = HarnessEvidenceOutcome::Violation {
        field: "tool_input.command".into(),
    };
    assert!(!fx.recorder.record(&note, &budget()).unwrap());
    note.outcome = HarnessEvidenceOutcome::Ok;
    assert!(!fx.recorder.record(&note, &budget()).unwrap());
    let reopened = Arc::new(
        SqliteStore::new(
            StoreContext::new(fx._iso.state_root().join("store.db"), fx.clock.clone()),
            "i",
            StoreSettings::default(),
        )
        .unwrap(),
    );
    let provider = HarnessStatesProvider::new(
        reopened.clone(),
        crate::daemon::harness_states::embedded_source(),
        fx.clock.clone(),
        Box::new(|_| None),
        None,
    );
    assert!(
        reopened
            .harness_evidence_all("codex", &budget())
            .unwrap()
            .is_empty()
    );
    let report = provider.report(&budget()).unwrap();
    let codex = &report.harnesses[1];
    assert!(codex.versions.is_empty());
    assert!(codex.detected.is_none());
    let reason = &codex.unattributed.as_ref().unwrap().reason;
    assert!(
        reason.contains("PreToolUse") && reason.contains("session_id"),
        "{reason}"
    );
    let lines = provider.health_lines(&budget()).unwrap();
    assert!(
        lines
            .iter()
            .any(|line| line.contains("PreToolUse") && line.contains("session_id")),
        "{lines:?}"
    );
}

// Actual observer output must stay distinct from historical listed admission.
#[test]
fn absorption_observer_to_v2_declares_contract_without_runtime_admission() {
    use crate::harness::{
        adapter::{DaemonObservation, InstallEnvironment},
        registry,
    };
    use crate::protocol::results::{AdmissionState, InstallationState};
    let fx = Fx::new("absorption-declared-projection");
    let bin = fx._iso.state_root().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    for name in ["claude", "codex"] {
        crate::harness::stub_binaries::write_stub_harness(
            &bin,
            name,
            if name == "claude" {
                "2.1.286"
            } else {
                "0.158.0"
            },
        );
    }
    let env = InstallEnvironment {
        path: Some(bin.into_os_string()),
        config_root: None,
        state_dir: None,
        clock: fx.clock.clone(),
    };
    let mut cached = std::collections::BTreeMap::new();
    for name in ["claude", "codex"] {
        let r = registry::builtins()
            .by_id(registry::builtins().agent(name).unwrap())
            .unwrap();
        cached.insert(name.into(), r.observe_daemon(&env, &budget()));
    }
    let report = fx
        .provider()
        .with_observations(Box::new(move || Ok(cached.clone())))
        .report_v2(&budget())
        .unwrap();
    for name in ["claude", "codex"] {
        let entry = &report.harnesses[name];
        assert_eq!(entry.installation.state, InstallationState::Present);
        assert_eq!(entry.admission.state, AdmissionState::Unknown);
        assert!(
            entry
                .admission
                .detail
                .as_deref()
                .is_some_and(|d| d.contains("contract_declared"))
        );
        assert!(entry.runtime_evidence.is_empty());
    }
    let doctor = serde_json::json!({"adapter_order":["claude","codex"],"harness_health_v2":report});
    let rendered = crate::cli::doctor::render_debug_text(&doctor);
    assert!(
        rendered.contains("contract_declared"),
        "actual doctor text dropped declared contract detail: {rendered}"
    );
    let legacy = DaemonObservation {
        status: HarnessStatus::Cooperative {
            detail: "historical listed".into(),
            live_unverified: false,
        },
        ..Default::default()
    };
    let report = fx
        .provider()
        .with_observations(Box::new(move || {
            Ok(std::collections::BTreeMap::from([(
                "claude".into(),
                legacy.clone(),
            )]))
        }))
        .report_v2(&budget())
        .unwrap();
    assert_eq!(
        report.harnesses["claude"].admission.state,
        AdmissionState::Listed
    );
}

// Production builtins feed cached consumers; the test-support-only fourth adapter
// is not a shipped harness and supplies no production installation observation.
#[test]
fn hermes_registry_presence_preserves_healthy_legacy_only_health() {
    use crate::{
        app::AdmissionReobserver,
        harness::{adapter::InstallEnvironment, registry},
        protocol::results::{
            AdmissionState, CallbackObservationState, EnablementState, InstallationState,
        },
    };
    use std::os::unix::fs::PermissionsExt;
    let fx = Fx::new("hermes-registry-presence");
    let bin = fx._iso.home().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    for name in ["claude", "codex"] {
        crate::harness::stub_binaries::write_stub_harness(&bin, name, "0.0.0");
    }
    let observer = AdmissionReobserver::with_registry(
        registry::builtins(),
        InstallEnvironment {
            clock: fx.clock.clone(),
            path: Some(bin.clone().into_os_string()),
            config_root: None,
            state_dir: None,
        },
        std::time::Duration::from_millis(50),
        Arc::new(|_| {}),
    );
    let legacy = |cached: &std::collections::BTreeMap<
        String,
        crate::harness::adapter::DaemonObservation,
    >| {
        let mut inputs = ready_inputs();
        inputs.instance = uuid::Uuid::from_u128(1);
        inputs.boot = uuid::Uuid::from_u128(2);
        inputs.claude = cached["claude"].status.clone();
        inputs.codex = cached["codex"].status.clone();
        inputs.additional_harnesses = vec![("hermes".into(), cached["hermes"].status.clone())];
        inputs.harness_version_lines = fx.provider().health_lines(&budget()).unwrap();
        inputs.assemble()
    };
    let absent = observer.pass(&Cancellation::default()).entries;
    assert!(matches!(
        absent["hermes"].status,
        HarnessStatus::NotInstalled(_)
    ));
    let absent_report = fx
        .provider()
        .with_observations(Box::new({
            let cached = absent.clone();
            move || Ok(cached.clone())
        }))
        .report_v2(&budget())
        .unwrap();
    assert_eq!(
        absent_report.harnesses["hermes"].installation.state,
        InstallationState::NotFound
    );
    let absent_health = legacy(&absent);
    assert_eq!(absent_health.state, HealthState::Healthy);
    assert!(
        absent_health
            .notes
            .iter()
            .any(|line| line.starts_with("harness hermes not installed:"))
    );
    let legacy_bytes = serde_json::to_vec(&absent_health.harness).unwrap();
    assert_eq!(
        legacy_bytes,
        br#"{"codex":"cooperative","claude":"cooperative"}"#
    );

    let executable = bin.join("hermes");
    std::fs::write(
        &executable,
        "#!/bin/sh\n: > \"${0%/*}/executed\"\nexit 91\n",
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    let present = observer.pass(&Cancellation::default()).entries;
    let observed = &present["hermes"];
    assert!(observed.identity.is_none());
    assert!(observed.receipt_basis.is_none());
    assert_eq!(observed.enablement.state, EnablementState::Unknown);
    assert_eq!(
        observed.callback_observation.state,
        CallbackObservationState::Unknown
    );
    assert!(!observed.status.reusable());
    assert_eq!(observed.status.admission_word(), "unqualified");
    assert_eq!(observed.status.state(), HarnessState::Unsupported);
    let provider = fx.provider().with_observations(Box::new({
        let cached = present.clone();
        move || Ok(cached.clone())
    }));
    let report = provider.report_v2(&budget()).unwrap();
    let entry = &report.harnesses["hermes"];
    assert_eq!(entry.installation.state, InstallationState::Present);
    assert_eq!(entry.admission.state, AdmissionState::Unknown);
    assert_eq!(entry.enablement.state, EnablementState::Unknown);
    assert_eq!(
        entry.callback_observation.state,
        CallbackObservationState::Unknown
    );
    assert_eq!(entry.receipt_basis, "unknown");
    assert!(entry.runtime_evidence.is_empty());
    assert!(
        entry
            .limitations
            .iter()
            .any(|line| line.contains("qualification unavailable"))
    );
    assert!(entry.admission.detail.as_deref().is_some_and(|d| {
        d.contains("qualification unavailable") && !d.contains("contract_declared")
    }));
    assert!(!bin.join("executed").exists());
    let present_health = legacy(&present);
    assert_eq!(present_health.state, HealthState::Healthy);
    assert_eq!(
        serde_json::to_vec(&present_health.harness).unwrap(),
        legacy_bytes
    );
    assert_eq!(present_health.limitations, absent_health.limitations);
    assert!(
        present_health
            .notes
            .iter()
            .any(|line| line.starts_with("harness hermes:")
                && line.contains("qualification unavailable"))
    );
    let mut unqualified_only = ready_inputs();
    unqualified_only.claude = HarnessStatus::NotInstalled("fixture absent".into());
    unqualified_only.codex = HarnessStatus::NotInstalled("fixture absent".into());
    unqualified_only.additional_harnesses = vec![("hermes".into(), observed.status.clone())];
    let health = unqualified_only.assemble();
    assert_eq!(health.state, HealthState::Healthy);
    assert!(
        !health
            .notes
            .iter()
            .any(|line| line.starts_with("receipt cooperative:"))
    );
    // Cache remains present after the executable disappears; only the next pass
    // observes the filesystem. Health requests cannot secretly rediscover Hermes.
    std::fs::remove_file(&executable).unwrap();
    assert_eq!(provider.report_v2(&budget()).unwrap(), report);
    assert_eq!(legacy(&present), present_health);
    assert!(matches!(
        observer.pass(&Cancellation::default()).entries["hermes"].status,
        HarnessStatus::NotInstalled(_)
    ));
    assert!(!bin.join("executed").exists());
}
