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
        .filter(|line| line.contains(" broken: ") || line.contains("below the supported floor"))
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

/// Kills: a wrong action for each manifest situation: upgrade when another
/// contract's release supports it, pin from last_working, report otherwise.
#[test]
fn broken_line_per_manifest_situation() {
    let version = newer_version();
    // (a) another contract verified it in a newer release: upgrade.
    let fx = Fx::new("hhs-broken-upgrade");
    fx.record(
        "claude",
        &version,
        CONTRACT,
        "SessionStart",
        HarnessEvidenceOutcome::Ok,
    );
    fx.plant_manifest(json!([
        broken_manifest_row(&version, json!({"last_working": "2.1.283"})),
        {"harness": "claude", "version": version, "status": "verified",
         "contract_id": "ffffffffffffffff", "supported_since": "999.0.0"},
    ]));
    let health = fx.health();
    assert_eq!(
        version_lines(&health),
        [&format!(
            "harness claude {version} broken: the canary manifest row reports PreToolUse payload \
             field tool_input.command; upgrade herdr-threads to 999.0.0 (supports claude {version})"
        )]
    );
    assert_eq!(health.state, HealthState::Degraded);

    // (b) no newer release, a last_working pointer: pin.
    let fx = Fx::new("hhs-broken-pin");
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
    assert_eq!(
        version_lines(&fx.health()),
        [&format!(
            "harness claude {version} broken: the canary manifest row reports PreToolUse payload \
             field tool_input.command; pin claude to <= 2.1.283"
        )]
    );

    // (b2) a locally verified older version beats the manifest's last_working.
    let older = listed_version();
    fx.verify("claude", &older, CONTRACT);
    assert!(
        version_lines(&fx.health())[0].ends_with(&format!("pin claude to <= {older}")),
        "{:#?}",
        fx.health().limitations
    );

    // (c) nothing else known: the row's issue URL.
    let fx = Fx::new("hhs-broken-report");
    fx.record(
        "claude",
        &version,
        CONTRACT,
        "SessionStart",
        HarnessEvidenceOutcome::Ok,
    );
    fx.plant_manifest(json!([broken_manifest_row(
        &version,
        json!({"issue_url": "https://example.test/issues/9"})
    )]));
    assert!(
        version_lines(&fx.health())[0].ends_with("; report: https://example.test/issues/9"),
        "{:#?}",
        fx.health().limitations
    );
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
        "harness claude {version} broken: PreToolUse payload field tool_input.command is missing \
         or has the wrong type; "
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

/// Kills: a below-floor version that reaches Health only through the PATH
/// observation (never end to end through evidence), a wrong floor, and the
/// below-floor line losing its `upgrade <harness>` action.
#[test]
fn below_floor_line_rendered_through_evidence() {
    let fx = Fx::new("hhs-floor");
    let version = below_floor_version();
    // The recorder is the real hook path: one lifecycle payload for an old
    // claude is enough.
    fx.record(
        "claude",
        &version,
        CONTRACT,
        "SessionStart",
        HarnessEvidenceOutcome::Ok,
    );
    let min = claude::RECIPES
        .iter()
        .map(|r| r.min_version())
        .min()
        .unwrap();
    let health = fx.health();
    assert_eq!(
        version_lines(&health),
        [&format!(
            "claude {version} is below the supported floor {min}; upgrade claude"
        )]
    );
    assert_eq!(health.state, HealthState::Degraded);
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
        lines[0].starts_with(&format!("harness claude {second} broken: ")),
        "{lines:#?}"
    );
    assert!(
        lines[1].starts_with("harness codex 999.0.0 broken: "),
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
    assert!(
        broken
            .line
            .starts_with(&format!("harness claude {version} broken: "))
    );
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
