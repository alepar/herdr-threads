//! The harness evidence recorder (ht-xoc.4): the held SessionStart, the
//! manifest triggers and the unattributed path.
use super::*;
use crate::{
    harness::manifest::{
        CurlFetcher, FetchError, FetchOutcome, Fetcher, ManifestPolicy, ManifestService,
    },
    protocol::time::{Cancellation, MonoInstant, UtcMillis},
    store::{SqliteStore, StoreSettings, connection::StoreContext},
    test_support::isolation::TestIsolation,
};
use std::{
    sync::atomic::{AtomicI64, Ordering},
    time::{Duration, Instant},
};

const HOUR_MS: i64 = 60 * 60 * 1000;
const T0: i64 = 1_000 * HOUR_MS;
const CONTRACT: &str = "0123456789abcdef";
const HELD_CONTRACT: &str = "aaaaaaaaaaaaaaaa";

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

#[derive(Default)]
struct Triggers(Mutex<Vec<(String, FetchReason)>>);
impl ManifestTrigger for Triggers {
    fn ensure_manifest(&self, harness: &str, reason: FetchReason) {
        self.0.lock().unwrap().push((harness.to_owned(), reason));
    }
}
impl Triggers {
    fn calls(&self) -> Vec<(String, FetchReason)> {
        self.0.lock().unwrap().clone()
    }
}

struct Fx {
    _iso: TestIsolation,
    store: Arc<SqliteStore>,
    clock: Arc<TestClock>,
    triggers: Arc<Triggers>,
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
        let triggers = Arc::new(Triggers::default());
        let recorder = HarnessEvidenceRecorder::new(
            store.clone(),
            Some(triggers.clone() as Arc<dyn ManifestTrigger>),
            clock.clone(),
        );
        Self {
            _iso: iso,
            store,
            clock,
            triggers,
            recorder,
        }
    }

    fn advance(&self, ms: i64) {
        self.clock.0.fetch_add(ms, Ordering::SeqCst);
    }

    fn row(
        &self,
        version: &str,
        contract: &str,
    ) -> Option<crate::store::harness_evidence::EvidenceRow> {
        self.store
            .harness_evidence("claude", version, contract, &budget())
            .unwrap()
    }
}

fn note(
    version: Option<&str>,
    event: &str,
    outcome: HarnessEvidenceOutcome,
    session: Option<&str>,
) -> HarnessEvidence {
    HarnessEvidence {
        harness: "claude".into(),
        version: version.map(str::to_owned),
        unattributed_reason: version
            .is_none()
            .then(|| "resume before first entry".into()),
        contract_id: CONTRACT.into(),
        event: event.into(),
        outcome,
        session_id: session.map(str::to_owned),
    }
}

fn ok() -> HarnessEvidenceOutcome {
    HarnessEvidenceOutcome::Ok
}

fn unseen(version: &str) -> FetchReason {
    FetchReason::UnseenVersion {
        version: version.into(),
    }
}

#[test]
fn buffered_session_start_attributed_on_first_tool_event() {
    let fx = Fx::new("her-buffered");
    let mut start = note(None, "SessionStart", ok(), Some("sess-1"));
    start.contract_id = HELD_CONTRACT.into();
    assert!(!fx.recorder.record(&start, &budget()).unwrap());
    assert!(
        fx.store
            .harness_evidence_all("claude", &budget())
            .unwrap()
            .is_empty(),
        "an unattributed SessionStart records no row"
    );
    fx.advance(1000);
    let verified = fx
        .recorder
        .record(
            &note(Some("2.1.286"), "PreToolUse", ok(), Some("sess-1")),
            &budget(),
        )
        .unwrap();
    assert!(
        !verified,
        "the tool row is under the hook's current contract"
    );
    let held = fx.row("2.1.286", HELD_CONTRACT).expect("held row");
    assert_eq!(held.lifecycle_ok_at, Some(T0 as u64 + 1000));
    // The held start is credited under the contract it was sent with.
    let mut tool = note(Some("2.1.286"), "PostToolUse", ok(), Some("sess-1"));
    tool.contract_id = HELD_CONTRACT.into();
    assert!(
        fx.recorder.record(&tool, &budget()).unwrap(),
        "lifecycle (buffered) plus tool under one contract verifies"
    );
    let row = fx.row("2.1.286", HELD_CONTRACT).unwrap();
    assert!(row.lifecycle_ok_at.is_some() && row.tool_ok_at.is_some());
}

#[test]
fn buffered_session_start_credits_the_same_contract_row() {
    let fx = Fx::new("her-buffered-same");
    fx.recorder
        .record(&note(None, "SessionStart", ok(), Some("s")), &budget())
        .unwrap();
    let verified = fx
        .recorder
        .record(
            &note(Some("2.1.286"), "PreToolUse", ok(), Some("s")),
            &budget(),
        )
        .unwrap();
    assert!(verified, "the first attributed event completes the pair");
    let row = fx.row("2.1.286", CONTRACT).unwrap();
    assert!(row.lifecycle_ok_at.is_some() && row.tool_ok_at.is_some());
}

#[test]
fn buffered_session_start_is_consumed_once_and_per_session() {
    let fx = Fx::new("her-buffered-once");
    fx.recorder
        .record(&note(None, "SessionStart", ok(), Some("a")), &budget())
        .unwrap();
    assert!(
        !fx.recorder
            .record(
                &note(Some("2.1.286"), "PreToolUse", ok(), Some("other")),
                &budget()
            )
            .unwrap(),
        "another session's tool event does not take it"
    );
    assert!(
        fx.recorder
            .record(
                &note(Some("2.1.286"), "PreToolUse", ok(), Some("a")),
                &budget()
            )
            .unwrap()
    );
    let first = fx.row("2.1.286", CONTRACT).unwrap().lifecycle_ok_at;
    fx.advance(10);
    fx.recorder
        .record(
            &note(Some("2.1.287"), "PreToolUse", ok(), Some("a")),
            &budget(),
        )
        .unwrap();
    assert_eq!(
        fx.row("2.1.287", CONTRACT).unwrap().lifecycle_ok_at,
        None,
        "the held start was spent on the first attributed event"
    );
    assert!(first.is_some());
}

#[test]
fn buffered_session_start_expires_after_24h() {
    let fx = Fx::new("her-expiry");
    fx.recorder
        .record(&note(None, "SessionStart", ok(), Some("s")), &budget())
        .unwrap();
    fx.advance(24 * HOUR_MS);
    let verified = fx
        .recorder
        .record(
            &note(Some("2.1.286"), "PreToolUse", ok(), Some("s")),
            &budget(),
        )
        .unwrap();
    assert!(!verified);
    assert_eq!(fx.row("2.1.286", CONTRACT).unwrap().lifecycle_ok_at, None);
}

#[test]
fn buffered_session_start_within_24h_still_counts() {
    let fx = Fx::new("her-expiry-edge");
    fx.recorder
        .record(&note(None, "SessionStart", ok(), Some("s")), &budget())
        .unwrap();
    fx.advance(24 * HOUR_MS - 1);
    assert!(
        fx.recorder
            .record(
                &note(Some("2.1.286"), "PreToolUse", ok(), Some("s")),
                &budget()
            )
            .unwrap()
    );
}

#[test]
fn pending_is_bounded() {
    let fx = Fx::new("her-bounded");
    for i in 0..PENDING_MAX_ENTRIES + 5 {
        fx.advance(1);
        fx.recorder
            .record(
                &note(None, "SessionStart", ok(), Some(&format!("s{i}"))),
                &budget(),
            )
            .unwrap();
    }
    assert_eq!(fx.recorder.pending().0.len(), PENDING_MAX_ENTRIES);
    // The oldest five were evicted, the newest kept.
    assert!(
        !fx.recorder
            .record(
                &note(Some("2.1.286"), "PreToolUse", ok(), Some("s0")),
                &budget()
            )
            .unwrap(),
        "s0 was evicted"
    );
    assert!(
        fx.recorder
            .record(
                &note(
                    Some("2.1.286"),
                    "PreToolUse",
                    ok(),
                    Some(&format!("s{}", PENDING_MAX_ENTRIES + 4))
                ),
                &budget()
            )
            .unwrap(),
        "the newest start is still held"
    );
}

#[test]
fn only_session_start_is_held() {
    let fx = Fx::new("her-only-start");
    fx.recorder
        .record(&note(None, "PreToolUse", ok(), Some("s")), &budget())
        .unwrap();
    assert_eq!(fx.recorder.pending().0.len(), 0);
    fx.recorder
        .record(&note(None, "SessionStart", ok(), None), &budget())
        .unwrap();
    assert_eq!(
        fx.recorder.pending().0.len(),
        0,
        "no session id: nothing to key the held start by"
    );
}

#[test]
fn codex_resume_session_start_is_never_held() {
    let fx = Fx::new("her-codex-resume");
    let mut resume = note(None, "SessionStart", ok(), Some("s"));
    resume.harness = "codex".into();
    resume.unattributed_reason = Some("codex resume: rollout version is the creating CLI's".into());
    resume.outcome = HarnessEvidenceOutcome::Violation {
        field: "source".into(),
    };
    assert!(!fx.recorder.record(&resume, &budget()).unwrap());
    assert_eq!(fx.recorder.pending().0.len(), 0, "not held");
    assert_eq!(
        fx.store
            .last_unattributed("codex", &budget())
            .unwrap()
            .map(|(reason, _)| reason)
            .as_deref(),
        Some("codex resume: rollout version is the creating CLI's"),
        "the reason is still recorded"
    );
    // A hook that lost its gate file attributes the next event from the head.
    let mut tool = note(Some("0.159.3"), "PreToolUse", ok(), Some("s"));
    tool.harness = "codex".into();
    fx.recorder.record(&tool, &budget()).unwrap();
    let row = fx
        .store
        .harness_evidence("codex", "0.159.3", CONTRACT, &budget())
        .unwrap()
        .expect("the tool row");
    assert_eq!(row.lifecycle_ok_at, None, "the held start was not flushed");
    assert_eq!(row.violation_at, None);
    // A Claude resume SessionStart is still held and credited later.
    fx.recorder
        .record(&note(None, "SessionStart", ok(), Some("c")), &budget())
        .unwrap();
    assert_eq!(fx.recorder.pending().0.len(), 1);
    fx.recorder
        .record(
            &note(Some("2.1.286"), "PreToolUse", ok(), Some("c")),
            &budget(),
        )
        .unwrap();
    assert!(
        fx.row("2.1.286", CONTRACT)
            .unwrap()
            .lifecycle_ok_at
            .is_some()
    );
}

#[test]
fn ensure_manifest_called_for_first_seen_version_whatever_the_outcome() {
    let fx = Fx::new("her-ensure");
    for (version, outcome) in [
        ("2.1.286", ok()),
        (
            "2.1.287",
            HarnessEvidenceOutcome::Violation {
                field: "tool_name".into(),
            },
        ),
        ("2.1.288", HarnessEvidenceOutcome::Malformed),
    ] {
        let before = fx.triggers.calls().len();
        fx.recorder
            .record(&note(Some(version), "PreToolUse", outcome, None), &budget())
            .unwrap();
        let calls = fx.triggers.calls();
        assert!(
            calls[before..].contains(&("claude".to_owned(), unseen(version))),
            "{version}: {calls:?}"
        );
    }
    let unseen_calls = fx
        .triggers
        .calls()
        .into_iter()
        .filter(|(_, reason)| matches!(reason, FetchReason::UnseenVersion { .. }))
        .count();
    assert_eq!(unseen_calls, 3, "one UnseenVersion call per fresh version");
    // A known version asks for nothing.
    let before = fx.triggers.calls().len();
    fx.recorder
        .record(&note(Some("2.1.286"), "PostToolUse", ok(), None), &budget())
        .unwrap();
    assert_eq!(fx.triggers.calls().len(), before);
}

#[test]
fn fresh_violation_triggers_ensure_manifest_once() {
    let fx = Fx::new("her-fresh-violation");
    fx.recorder
        .record(
            &note(Some("2.1.286"), "SessionStart", ok(), None),
            &budget(),
        )
        .unwrap();
    let violation = || {
        note(
            Some("2.1.286"),
            "PreToolUse",
            HarnessEvidenceOutcome::Violation { field: "f".into() },
            None,
        )
    };
    fx.recorder.record(&violation(), &budget()).unwrap();
    fx.recorder.record(&violation(), &budget()).unwrap();
    let fresh = fx
        .triggers
        .calls()
        .into_iter()
        .filter(|(_, reason)| *reason == FetchReason::FreshViolation)
        .count();
    assert_eq!(fresh, 1, "only the first violation is fresh");
}

#[test]
fn ambiguous_attribution_records_nothing() {
    let fx = Fx::new("her-ambiguous");
    let verified = fx
        .recorder
        .record(&note(None, "PreToolUse", ok(), Some("s")), &budget())
        .unwrap();
    assert!(!verified);
    assert!(
        fx.store
            .harness_evidence_all("claude", &budget())
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        fx.store.last_unattributed("claude", &budget()).unwrap(),
        Some(("resume before first entry".to_owned(), T0 as u64))
    );
    assert!(fx.triggers.calls().is_empty(), "no fetch for no version");
}

#[test]
fn verified_reply_tracks_the_row_state() {
    let fx = Fx::new("her-verified-reply");
    let a = fx
        .recorder
        .record(
            &note(Some("2.1.286"), "SessionStart", ok(), None),
            &budget(),
        )
        .unwrap();
    let b = fx
        .recorder
        .record(&note(Some("2.1.286"), "PreToolUse", ok(), None), &budget())
        .unwrap();
    assert!((a, b) == (false, true));
}

/// A real `curl` fetch of an unreachable loopback URL.
struct UnreachableCurl(CurlFetcher);
impl Fetcher for UnreachableCurl {
    fn fetch(&self, _url: &str, etag: Option<&str>) -> Result<FetchOutcome, FetchError> {
        self.0
            .fetch("http://127.0.0.1:9/harness-versions.json", etag)
    }
}

#[test]
fn unreachable_manifest_url_does_not_delay_recording() {
    let iso = TestIsolation::new("her-unreachable");
    let clock = Arc::new(TestClock(AtomicI64::new(T0)));
    let store = Arc::new(
        SqliteStore::new(
            StoreContext::new(iso.state_root().join("store.db"), clock.clone()),
            "i",
            StoreSettings::default(),
        )
        .unwrap(),
    );
    let cache = iso.state_root().join("harness-manifest");
    let manifest = Arc::new(ManifestService::new(
        cache.clone(),
        ManifestPolicy::Auto,
        Arc::new(UnreachableCurl(CurlFetcher::new(cache))),
        clock.clone(),
        Arc::new(|_: &str| {}),
    ));
    let recorder = HarnessEvidenceRecorder::new(
        store.clone(),
        Some(manifest.clone() as Arc<dyn ManifestTrigger>),
        clock,
    );
    // Warm the store connections so the timing measures the recorder.
    store.last_unattributed("claude", &budget()).unwrap();
    let started = Instant::now();
    recorder
        .record(
            &note(Some("9.9.9"), "SessionStart", ok(), Some("s")),
            &budget(),
        )
        .unwrap();
    let elapsed = started.elapsed();
    assert!(elapsed < Duration::from_millis(50), "{elapsed:?}");
    let row = store
        .harness_evidence("claude", "9.9.9", CONTRACT, &budget())
        .unwrap();
    assert!(row.is_some(), "the evidence row exists");
    assert!(manifest.wait_idle(Duration::from_secs(20)));
}

/// The recorder's store, failing on cue: each `true` popped from a script
/// fails that write with `store_busy`; an empty script lets writes
/// through to the real store.
struct FlakyWrites {
    inner: Arc<SqliteStore>,
    evidence: Mutex<VecDeque<bool>>,
    unattributed: Mutex<VecDeque<bool>>,
}

impl FlakyWrites {
    fn fail(script: &Mutex<VecDeque<bool>>) -> bool {
        script.lock().unwrap().pop_front().unwrap_or(false)
    }
}

impl EvidenceWrites for FlakyWrites {
    fn record_harness_evidence(
        &self,
        record: &EvidenceRecord<'_>,
        budget: &CallBudget,
    ) -> Result<crate::store::harness_evidence::Recorded, ApiError> {
        if Self::fail(&self.evidence) {
            return Err(ApiError::store_busy("injected evidence write failure"));
        }
        self.inner.record_harness_evidence(record, budget)
    }

    fn record_contract_diagnostic(
        &self,
        record: &DiagnosticRecord<'_>,
        budget: &CallBudget,
    ) -> Result<(), ApiError> {
        self.inner.record_contract_diagnostic(record, budget)
    }

    fn record_unattributed(
        &self,
        harness: &str,
        reason: &str,
        budget: &CallBudget,
    ) -> Result<(), ApiError> {
        if Self::fail(&self.unattributed) {
            return Err(ApiError::store_busy("injected reason write failure"));
        }
        self.inner.record_unattributed(harness, reason, budget)
    }
}

/// An [`Fx`] whose recorder writes through [`FlakyWrites`].
fn flaky_fx(label: &str, evidence: &[bool], unattributed: &[bool]) -> Fx {
    let mut fx = Fx::new(label);
    let writes = Arc::new(FlakyWrites {
        inner: fx.store.clone(),
        evidence: Mutex::new(evidence.iter().copied().collect()),
        unattributed: Mutex::new(unattributed.iter().copied().collect()),
    });
    fx.recorder = HarnessEvidenceRecorder::with_writes(
        writes,
        Some(fx.triggers.clone() as Arc<dyn ManifestTrigger>),
        fx.clock.clone(),
    );
    fx
}

fn held_start(session: &str, outcome: HarnessEvidenceOutcome) -> HarnessEvidence {
    let mut start = note(None, "SessionStart", outcome, Some(session));
    start.contract_id = HELD_CONTRACT.into();
    start
}

#[test]
fn held_session_start_survives_a_failed_flush() {
    let fx = flaky_fx("her-flush-fail", &[true], &[]);
    assert!(
        !fx.recorder
            .record(&held_start("sess-1", ok()), &budget())
            .unwrap()
    );
    let tool = note(Some("2.1.286"), "PreToolUse", ok(), Some("sess-1"));
    assert!(fx.recorder.record(&tool, &budget()).is_err());
    assert!(fx.row("2.1.286", HELD_CONTRACT).is_none());
    assert!(fx.row("2.1.286", CONTRACT).is_none());
    fx.advance(1000);
    fx.recorder.record(&tool, &budget()).unwrap();
    assert!(
        fx.row("2.1.286", HELD_CONTRACT)
            .unwrap()
            .lifecycle_ok_at
            .is_some()
    );
    assert!(fx.row("2.1.286", CONTRACT).unwrap().tool_ok_at.is_some());
}

#[test]
fn stored_held_start_is_not_replayed_when_the_event_write_fails() {
    let fx = flaky_fx("her-no-replay", &[false, true], &[]);
    let violation = HarnessEvidenceOutcome::Violation {
        field: "session_id".into(),
    };
    fx.recorder
        .record(&held_start("sess-1", violation), &budget())
        .unwrap();
    let tool = note(Some("2.1.286"), "PreToolUse", ok(), Some("sess-1"));
    assert!(fx.recorder.record(&tool, &budget()).is_err());
    assert!(fx.row("2.1.286", HELD_CONTRACT).is_some());
    assert!(fx.row("2.1.286", CONTRACT).is_none());
    fx.recorder.record(&tool, &budget()).unwrap();
    let fresh = fx
        .triggers
        .calls()
        .into_iter()
        .filter(|(_, reason)| *reason == FetchReason::FreshViolation)
        .count();
    assert_eq!(fresh, 1, "the held violation was written once");
}

#[test]
fn unattributed_session_start_is_held_even_when_the_reason_write_fails() {
    let fx = flaky_fx("her-reason-fail", &[], &[true]);
    assert!(
        fx.recorder
            .record(&held_start("sess-1", ok()), &budget())
            .is_err()
    );
    fx.recorder
        .record(
            &note(Some("2.1.286"), "PreToolUse", ok(), Some("sess-1")),
            &budget(),
        )
        .unwrap();
    assert!(
        fx.row("2.1.286", HELD_CONTRACT)
            .unwrap()
            .lifecycle_ok_at
            .is_some()
    );
}

#[test]
fn unattributed_event_reason_write_failure_is_returned() {
    let fx = flaky_fx("her-reason-event", &[], &[true]);
    assert!(
        fx.recorder
            .record(&note(None, "PreToolUse", ok(), Some("sess-1")), &budget())
            .is_err()
    );
    fx.recorder
        .record(
            &note(Some("2.1.286"), "PreToolUse", ok(), Some("sess-1")),
            &budget(),
        )
        .unwrap();
    let row = fx.row("2.1.286", CONTRACT).unwrap();
    assert!(row.tool_ok_at.is_some());
    assert!(row.lifecycle_ok_at.is_none(), "nothing was held");
}

#[test]
fn event_write_failure_without_a_held_start_is_returned() {
    let fx = flaky_fx("her-event-fail", &[true], &[]);
    let tool = note(Some("2.1.286"), "PreToolUse", ok(), Some("sess-1"));
    assert!(fx.recorder.record(&tool, &budget()).is_err());
    assert!(fx.row("2.1.286", CONTRACT).is_none());
    fx.recorder.record(&tool, &budget()).unwrap();
    assert!(fx.row("2.1.286", CONTRACT).unwrap().tool_ok_at.is_some());
}

fn held(session: &str, received_ms: i64, contract: &str) -> Held {
    Held {
        harness: "claude".into(),
        session_id: session.into(),
        received_ms,
        contract_id: contract.into(),
        event: "SessionStart".into(),
        outcome: EvidenceOutcome::Ok,
    }
}

#[test]
fn restore_yields_to_a_newer_hold() {
    let mut pending = PendingSessionStarts::default();
    pending.hold(held("s", T0, "a"), T0);
    let a = pending.take("claude", "s", T0).unwrap();
    pending.hold(held("s", T0 + 1, "b"), T0 + 1);
    pending.restore(a, T0 + 2);
    assert_eq!(
        pending.take("claude", "s", T0 + 2).unwrap().received_ms,
        T0 + 1
    );
    assert!(pending.take("claude", "s", T0 + 2).is_none());

    // An expired entry is dropped.
    pending.restore(held("old", T0, "a"), T0 + PENDING_MAX_AGE_MS);
    assert!(pending.0.is_empty());

    // Restoring into a full queue keeps the bound and the order.
    for i in 0..PENDING_MAX_ENTRIES {
        pending.hold(held(&format!("s{i}"), T0 + 10 + i as i64, "a"), T0 + 10);
    }
    pending.restore(held("back", T0 + 5, "a"), T0 + 20);
    assert_eq!(pending.0.len(), PENDING_MAX_ENTRIES);
    assert!(
        pending.take("claude", "back", T0 + 20).is_none(),
        "the oldest was evicted"
    );
    assert!(pending.take("claude", "s0", T0 + 20).is_some());
}

fn v2_note(runtime: bool, event: &str) -> crate::protocol::commands::HarnessEvidenceV2 {
    use crate::harness::adapter::HarnessAdapter;
    let descriptor = crate::harness::claude::ClaudeAdapter.contracts()[0];
    crate::protocol::commands::HarnessEvidenceV2 {
        harness: "claude".into(),
        domain: "native_payload".into(),
        origin: descriptor.origin,
        runtime: runtime.then(|| {
            crate::harness::runtime::RuntimeIdentity::stable_release("2.1.286", "native_transcript")
                .unwrap()
        }),
        unavailable_reason: (!runtime).then(|| "awaiting transcript".into()),
        contract_id: descriptor.contract_id_v2().unwrap(),
        event: event.into(),
        outcome: crate::protocol::commands::HarnessEvidenceOutcomeV2::Ok,
        session_id: Some("v2-session".into()),
        qualifications: vec![],
    }
}
#[test]
fn v2_recorder_records_required_milestones_without_legacy_rows() {
    let fx = Fx::new("v2-recorder");
    let recorder = HarnessEvidenceRecorderV2::new(
        fx.store.clone(),
        Some(fx.triggers.clone()),
        fx.clock.clone(),
    );
    assert!(
        !recorder
            .record(&v2_note(true, "SessionStart"), &budget())
            .unwrap()
    );
    assert!(
        recorder
            .record(&v2_note(true, "PreToolUse"), &budget())
            .unwrap()
    );
    assert!(
        fx.store
            .harness_evidence_all("claude", &budget())
            .unwrap()
            .is_empty()
    );
    let rows = fx
        .store
        .harness_evidence_v2_all("claude", 0, &budget())
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0]
            .milestones
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["lifecycle", "tool"]
    );
}

#[test]
fn v2_holds_credit_only_exact_contract_without_stale_fallback() {
    let fx = Fx::new("v2-holds");
    let recorder = HarnessEvidenceRecorderV2::new(fx.store.clone(), None, fx.clock.clone());
    assert!(
        !recorder
            .record(&v2_note(false, "SessionStart"), &budget())
            .unwrap()
    );
    let mut stale = v2_note(true, "PreToolUse");
    stale.contract_id = "ffffffffffffffff".into();
    assert_eq!(
        recorder.record(&stale, &budget()).unwrap_err().code,
        crate::protocol::results::ErrorCode::Unsupported
    );
    assert!(
        recorder
            .record(&v2_note(true, "PreToolUse"), &budget())
            .unwrap(),
        "held lifecycle must survive unrelated stale observation"
    );
}
#[test]
fn v2_codex_resumed_suppression_is_sticky_for_session() {
    use crate::harness::adapter::HarnessAdapter;
    let fx = Fx::new("v2-codex-resume");
    let recorder = HarnessEvidenceRecorderV2::new(fx.store.clone(), None, fx.clock.clone());
    let d = crate::harness::codex::CodexAdapter.contracts()[0];
    let mut start = v2_note(false, "SessionStart");
    start.harness = "codex".into();
    start.contract_id = d.contract_id_v2().unwrap();
    recorder.record(&start, &budget()).unwrap();
    start.unavailable_reason = d.resumed_unavailable_reason.map(str::to_owned);
    recorder.record(&start, &budget()).unwrap();
    start.runtime = Some(
        crate::harness::runtime::RuntimeIdentity::stable_release("0.159.3", "native_transcript")
            .unwrap(),
    );
    start.unavailable_reason = None;
    recorder.record(&start, &budget()).unwrap();
    start.event = "PreToolUse".into();
    assert!(
        !recorder.record(&start, &budget()).unwrap(),
        "resumed creating-CLI lifecycle must never receive later credit"
    );
    let rows = fx
        .store
        .harness_evidence_v2_all("codex", 0, &budget())
        .unwrap();
    assert!(!rows[0].milestones.contains_key("lifecycle"));
}

struct FourthEvidenceAdapter;
impl crate::harness::adapter::HarnessAdapter for FourthEvidenceAdapter {
    type Admission = ();
    fn metadata(&self) -> &'static crate::harness::adapter::AdapterMetadata {
        use crate::harness::adapter::*;
        static META: AdapterMetadata = AdapterMetadata {
            id: "fourth",
            display_label: "Fourth fixture",
            context_spelling: "Fourth",
            context_aliases: &[],
            executable: ExecutableLookup::Unsupported,
            host_kinds: &["fourth"],
            setup_scopes: &[],
            budget: EventBudgetPolicy {
                lifecycle_ms: 5000,
                observer_ms: 1500,
            },
            runtime_sources: &["fixture"],
        };
        &META
    }
    fn contracts(&self) -> &'static [crate::harness::adapter::ContractDescriptor] {
        use crate::harness::{
            adapter::{ContractDescriptor, ContractDomain},
            contract::{EventClass, EventContract, FieldSpec, HarnessContract, JsonType},
            evidence::{AttributionHolding, EvidenceEvent, EvidenceOrigin},
        };
        static CONTRACT: HarnessContract = HarnessContract {
            harness: "fourth",
            discriminator: "event",
            events: &[
                EventContract {
                    event: "turn_started",
                    class: EventClass::Lifecycle,
                    fields: &[FieldSpec {
                        path: "session_id",
                        ty: JsonType::String,
                        required: true,
                    }],
                },
                EventContract {
                    event: "tool_finished",
                    class: EventClass::Tool,
                    fields: &[FieldSpec {
                        path: "session_id",
                        ty: JsonType::String,
                        required: true,
                    }],
                },
            ],
        };
        static EVENTS: &[EvidenceEvent] = &[
            EvidenceEvent {
                native_event: "turn_started",
                milestone: Some("turn"),
                always_send: true,
            },
            EvidenceEvent {
                native_event: "tool_finished",
                milestone: Some("post_tool"),
                always_send: false,
            },
        ];
        static DOMAINS: [ContractDescriptor; 2] = [
            ContractDescriptor {
                domain_id: "native_shape",
                origin: EvidenceOrigin::NativeShapeObservation,
                events: EVENTS,
                required_milestones: &["turn", "post_tool"],
                qualifications: &["same_runtime", "observer"],
                holding: AttributionHolding::UntilAttributed,
                resumed_unavailable_reason: None,
                domain: ContractDomain::Native,
                contract: &CONTRACT,
            },
            ContractDescriptor {
                domain_id: "bridge",
                origin: EvidenceOrigin::BridgeEnvelope,
                events: EVENTS,
                required_milestones: &["turn", "post_tool"],
                qualifications: &["same_runtime", "observer"],
                holding: AttributionHolding::Never,
                resumed_unavailable_reason: None,
                domain: ContractDomain::Bridge,
                contract: &CONTRACT,
            },
        ];
        &DOMAINS
    }
    fn observe_install(
        &self,
        _: &crate::harness::adapter::InstallEnvironment,
        _: &CallBudget,
    ) -> crate::harness::adapter::InstallObservation {
        panic!("evidence must not probe")
    }
    fn admit(
        &self,
        _: &crate::harness::adapter::AdmissionRequest,
        _: &CallBudget,
    ) -> crate::harness::adapter::AdmissionDecision<()> {
        panic!("evidence grants no admission")
    }
    fn version_ladder(
        &self,
        _: &crate::harness::runtime::RuntimeIdentity,
    ) -> crate::harness::adapter::Ladder {
        panic!("evidence is not release admission")
    }
    fn classify(
        &self,
        _: &crate::harness::adapter::HookInput,
    ) -> crate::harness::adapter::ContractObservation {
        panic!("wire supplies projected outcome")
    }
    fn decode(
        &self,
        _: &(),
        _: &crate::harness::adapter::HookInput,
    ) -> Result<crate::harness::adapter::DecodedEvent, crate::harness::adapter::DecodeFailure> {
        panic!("no decode")
    }
    fn encode(
        &self,
        _: &(),
        _: &crate::harness::adapter::DecodedEvent,
        _: &crate::harness::adapter::NeutralOffer,
    ) -> Result<crate::harness::adapter::EncodedOutput, crate::harness::adapter::EncodeFailure>
    {
        panic!("no offers")
    }
    fn attribute_runtime(
        &self,
        _: &crate::harness::adapter::HookInput,
        _: &CallBudget,
    ) -> crate::harness::adapter::RuntimeAttribution {
        panic!("no runtime execution")
    }
    fn setup(
        &self,
        _: &crate::harness::adapter::SetupRequest,
        _: &CallBudget,
    ) -> Result<crate::harness::adapter::SetupOutcome, crate::harness::adapter::SetupFailure> {
        panic!("no setup")
    }
    fn status(
        &self,
        _: &crate::harness::adapter::StatusRequest,
        _: &CallBudget,
    ) -> crate::harness::adapter::SetupStatus {
        panic!("no status")
    }
    fn unsetup(
        &self,
        _: &crate::harness::adapter::UnsetupRequest,
        _: &CallBudget,
    ) -> Result<crate::harness::adapter::RemovalOutcome, crate::harness::adapter::SetupFailure>
    {
        panic!("no unsetup")
    }
}
fn fourth_registry() -> &'static crate::harness::registry::Registry {
    static REG: std::sync::OnceLock<crate::harness::registry::Registry> =
        std::sync::OnceLock::new();
    REG.get_or_init(|| {
        crate::harness::registry::Registry::new(Box::leak(
            vec![crate::harness::registry::Registration::new(
                &FourthEvidenceAdapter,
            )]
            .into_boxed_slice(),
        ))
        .unwrap()
    })
}
fn fourth_note(
    domain: usize,
    runtime: bool,
    event: &str,
) -> crate::protocol::commands::HarnessEvidenceV2 {
    use crate::harness::adapter::HarnessAdapter;
    let d = FourthEvidenceAdapter.contracts()[domain];
    let mut n = v2_note(runtime, event);
    n.harness = "fourth".into();
    n.domain = d.domain_id.into();
    n.origin = d.origin;
    n.contract_id = d.contract_id_v2().unwrap();
    n.qualifications = vec!["same_runtime".into(), "observer".into()];
    n
}
#[test]
fn v2_fourth_harness_observer_domains_are_independent_and_do_not_create_authority() {
    let fx = Fx::new("v2-fourth");
    let recorder = HarnessEvidenceRecorderV2::new(
        fx.store.clone(),
        Some(fx.triggers.clone()),
        fx.clock.clone(),
    )
    .with_registry(fourth_registry());
    recorder
        .record(&fourth_note(0, false, "turn_started"), &budget())
        .unwrap();
    assert!(
        !recorder
            .record(&fourth_note(1, true, "tool_finished"), &budget())
            .unwrap()
    );
    assert!(
        recorder
            .record(&fourth_note(0, true, "tool_finished"), &budget())
            .unwrap()
    );
    let rows = fx
        .store
        .harness_evidence_v2_all("fourth", 0, &budget())
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter()
            .find(|r| r.domain == "native_shape")
            .unwrap()
            .milestones
            .contains_key("turn")
    );
    assert!(
        !rows
            .iter()
            .find(|r| r.domain == "bridge")
            .unwrap()
            .milestones
            .contains_key("turn")
    );
    let connection = rusqlite::Connection::open(fx._iso.state_root().join("store.db")).unwrap();
    for table in [
        "seats",
        "occupant_bindings",
        "receipts",
        "warning_offer",
        "digest_notice_offer",
        "digest_open_warnings",
        "memberships",
        "messages",
    ] {
        let count: i64 = connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "no authority side effect in {table}");
    }
}
#[test]
fn v2_held_write_failure_is_restored_for_exact_retry() {
    let fx = Fx::new("v2-failed-hold");
    let recorder = HarnessEvidenceRecorderV2::new(fx.store.clone(), None, fx.clock.clone());
    recorder
        .record(&v2_note(false, "SessionStart"), &budget())
        .unwrap();
    let db = rusqlite::Connection::open(fx._iso.state_root().join("store.db")).unwrap();
    db.execute_batch("CREATE TRIGGER reject_v2 BEFORE INSERT ON harness_contract_evidence_v2 BEGIN SELECT RAISE(ABORT,'injected evidence failure'); END;").unwrap();
    assert!(
        recorder
            .record(&v2_note(true, "PreToolUse"), &budget())
            .is_err()
    );
    db.execute_batch("DROP TRIGGER reject_v2;").unwrap();
    assert!(
        recorder
            .record(&v2_note(true, "PreToolUse"), &budget())
            .unwrap()
    );
}
#[test]
fn v2_rejects_undeclared_violation_field_without_poisoning_domain() {
    let fx = Fx::new("v2-bad-field");
    let recorder = HarnessEvidenceRecorderV2::new(fx.store.clone(), None, fx.clock.clone());
    let mut n = v2_note(true, "SessionStart");
    n.outcome = crate::protocol::commands::HarnessEvidenceOutcomeV2::Violation {
        field: "user_content".into(),
    };
    assert_eq!(
        recorder.record(&n, &budget()).unwrap_err().code,
        crate::protocol::results::ErrorCode::Unsupported
    );
    assert!(
        fx.store
            .harness_evidence_v2_all("claude", 0, &budget())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn v2_build_and_release_first_seen_fetches_use_exact_domain_reason() {
    for build in [false, true] {
        let fx = Fx::new("v2-exact-fetch");
        let recorder = HarnessEvidenceRecorderV2::new(
            fx.store.clone(),
            Some(fx.triggers.clone()),
            fx.clock.clone(),
        );
        let mut n = v2_note(true, "SessionStart");
        if build {
            n.runtime = Some(
                crate::harness::runtime::RuntimeIdentity::build(
                    crate::harness::runtime::RuntimeDescriptor {
                        release_version: Some("2.1.286".into()),
                        source: "fixture".into(),
                        base_version: Some("2.1.286".into()),
                        derived_version: Some("2.1.286+dev".into()),
                        commit: None,
                        dirty: Some(true),
                        distance: None,
                    },
                )
                .unwrap(),
            );
        }
        recorder.record(&n, &budget()).unwrap();
        assert_eq!(
            fx.triggers.calls(),
            vec![(
                "claude".into(),
                FetchReason::UnseenRuntime {
                    identity: n.runtime.clone().unwrap(),
                    domain: "native_payload".into(),
                    origin: n.origin,
                    contract_id: n.contract_id.clone()
                }
            )]
        );
        recorder.record(&n, &budget()).unwrap();
        assert_eq!(fx.triggers.calls().len(), 1);
    }
}

#[test]
fn v2_pending_ttl_capacity_and_failed_restore_are_deterministic() {
    let mut pending = PendingV2::default();
    let held = |session: String, at: i64| {
        let mut note = v2_note(false, "SessionStart");
        note.session_id = Some(session.clone());
        HeldV2 {
            key: V2HoldKey {
                harness: "claude".into(),
                session,
                domain: "native_payload".into(),
                origin: note.origin,
                contract: note.contract_id.clone(),
            },
            received_ms: at,
            note,
        }
    };
    for i in 0..=PENDING_MAX_ENTRIES {
        pending.push(held(format!("s{i}"), T0 + i as i64), T0 + i as i64);
    }
    assert_eq!(pending.0.len(), 1024);
    assert_eq!(pending.0.front().unwrap().key.session, "s1");
    let failed = pending.0.remove(3).unwrap();
    let failed_key = failed.key.clone();
    let received = failed.received_ms;
    pending.restore(failed, T0 + 1024);
    assert_eq!(pending.0[3].key.session, failed_key.session);
    pending.expire(received + PENDING_MAX_AGE_MS);
    assert!(pending.0.iter().all(|h| h.received_ms > received));
    let failed = held("retry".into(), T0);
    pending.push(held("retry".into(), T0 + 1), T0 + 1);
    pending.restore(failed, T0 + 1);
    assert_eq!(
        pending
            .0
            .iter()
            .filter(|h| h.key.session == "retry")
            .count(),
        1
    );
    assert_eq!(
        pending
            .0
            .iter()
            .find(|h| h.key.session == "retry")
            .unwrap()
            .received_ms,
        T0 + 1
    );
}
#[test]
fn v2_missing_qualification_malformed_and_violation_are_not_success() {
    let fx = Fx::new("v2-qualified");
    let recorder = HarnessEvidenceRecorderV2::new(fx.store.clone(), None, fx.clock.clone())
        .with_registry(fourth_registry());
    let mut n = fourth_note(0, true, "turn_started");
    n.qualifications.clear();
    recorder.record(&n, &budget()).unwrap();
    n = fourth_note(0, true, "tool_finished");
    n.outcome = crate::protocol::commands::HarnessEvidenceOutcomeV2::Malformed;
    assert!(!recorder.record(&n, &budget()).unwrap());
    let rows = fx
        .store
        .harness_evidence_v2_all("fourth", 0, &budget())
        .unwrap();
    assert!(rows[0].milestones.is_empty());
    assert!(rows[0].violation_at.is_none());
    n.outcome = crate::protocol::commands::HarnessEvidenceOutcomeV2::Violation {
        field: "session_id".into(),
    };
    recorder.record(&n, &budget()).unwrap();
    n.outcome = crate::protocol::commands::HarnessEvidenceOutcomeV2::Ok;
    assert!(!recorder.record(&n, &budget()).unwrap());
    n.event = "turn_started".into();
    assert!(!recorder.record(&n, &budget()).unwrap());
    let rows = fx
        .store
        .harness_evidence_v2_all("fourth", 0, &budget())
        .unwrap();
    assert_eq!(rows[0].violation_field.as_deref(), Some("session_id"));
    assert_eq!(rows[0].milestones.len(), 2);
}

struct CachedExactDomain;
impl RichManifestSource for CachedExactDomain {
    fn contains(
        &self,
        harness: &str,
        identity: &crate::harness::runtime::RuntimeIdentity,
        domain: &str,
        origin: crate::harness::evidence::EvidenceOrigin,
        contract_id: &str,
    ) -> bool {
        harness == "claude"
            && identity.key == "release:2.1.286"
            && identity.source == "native_transcript"
            && domain == "native_payload"
            && origin == crate::harness::evidence::EvidenceOrigin::NativePayload
            && contract_id == "c4c4b249584b3578"
    }
}
#[test]
fn v2_cached_rich_row_skips_refresh_without_granting_local_verification() {
    let fx = Fx::new("v2-rich-cache");
    let recorder = HarnessEvidenceRecorderV2::new(
        fx.store.clone(),
        Some(fx.triggers.clone()),
        fx.clock.clone(),
    )
    .with_rich_manifest_source(Arc::new(CachedExactDomain));
    assert!(
        !recorder
            .record(&v2_note(true, "SessionStart"), &budget())
            .unwrap()
    );
    assert!(fx.triggers.calls().is_empty());
    let rows = fx
        .store
        .harness_evidence_v2_all("claude", 0, &budget())
        .unwrap();
    assert_eq!(rows[0].milestones.len(), 1);
    assert!(
        recorder
            .record(&v2_note(true, "PreToolUse"), &budget())
            .unwrap()
    );
}
#[test]
fn v2_unknown_identity_domain_origin_event_or_qualification_never_falls_back() {
    let fx = Fx::new("v2-refuse");
    let recorder = HarnessEvidenceRecorderV2::new(fx.store.clone(), None, fx.clock.clone());
    for which in [
        "harness",
        "domain",
        "origin",
        "contract",
        "event",
        "qualification",
    ] {
        let mut n = v2_note(true, "SessionStart");
        match which {
            "harness" => n.harness = "missing".into(),
            "domain" => n.domain = "other".into(),
            "origin" => n.origin = crate::harness::evidence::EvidenceOrigin::BridgeEnvelope,
            "contract" => n.contract_id = "ffffffffffffffff".into(),
            "event" => n.event = "turn_started".into(),
            "qualification" => n.qualifications = vec!["invented".into()],
            _ => unreachable!(),
        }
        assert_eq!(
            recorder.record(&n, &budget()).unwrap_err().code,
            crate::protocol::results::ErrorCode::Unsupported,
            "{which}"
        );
    }
    assert!(
        fx.store
            .harness_evidence_v2_all("claude", 0, &budget())
            .unwrap()
            .is_empty()
    );
    let mut forged = v2_note(true, "SessionStart");
    forged.runtime.as_mut().unwrap().key = "release:9.9.9".into();
    assert_eq!(
        recorder.record(&forged, &budget()).unwrap_err().code,
        crate::protocol::results::ErrorCode::InvalidRequest
    );
}

// Catch independent legacy/v2 caps: alternating paths must evict the oldest
// legacy start when their combined count reaches 1025.
#[test]
fn mixed_pending_paths_share_one_capacity_and_oldest_eviction() {
    let fx = Fx::new("mixed-pending-bound");
    let recorder = HarnessEvidenceRecorderV2::new(fx.store.clone(), None, fx.clock.clone())
        .with_legacy_pending(&fx.recorder);
    fx.recorder
        .record(
            &note(None, "SessionStart", ok(), Some("oldest-legacy")),
            &budget(),
        )
        .unwrap();
    for i in 0..PENDING_MAX_ENTRIES {
        fx.advance(1);
        if i % 2 == 0 {
            let mut n = v2_note(false, "SessionStart");
            n.session_id = Some(format!("mixed-v2-{i}"));
            recorder.record(&n, &budget()).unwrap();
        } else {
            fx.recorder
                .record(
                    &note(
                        None,
                        "SessionStart",
                        ok(),
                        Some(&format!("mixed-legacy-{i}")),
                    ),
                    &budget(),
                )
                .unwrap();
        }
    }
    assert!(
        !fx.recorder
            .record(
                &note(Some("2.1.286"), "PreToolUse", ok(), Some("oldest-legacy")),
                &budget()
            )
            .unwrap(),
        "aggregate capacity must evict oldest across both paths"
    );
    let mut tool = v2_note(true, "PreToolUse");
    tool.session_id = Some("mixed-v2-1022".into());
    assert!(
        recorder.record(&tool, &budget()).unwrap(),
        "recent v2 hold remains eligible"
    );
    assert!(
        fx.recorder
            .record(
                &note(
                    Some("2.1.286"),
                    "PreToolUse",
                    ok(),
                    Some("mixed-legacy-1023")
                ),
                &budget()
            )
            .unwrap(),
        "recent legacy hold remains eligible"
    );
}

fn codex_note(
    runtime: bool,
    event: &str,
    session: &str,
) -> crate::protocol::commands::HarnessEvidenceV2 {
    use crate::harness::adapter::HarnessAdapter;
    let d = crate::harness::codex::CodexAdapter.contracts()[0];
    let mut n = v2_note(runtime, event);
    n.harness = "codex".into();
    n.contract_id = d.contract_id_v2().unwrap();
    n.session_id = Some(session.into());
    n.runtime = runtime.then(|| {
        crate::harness::runtime::RuntimeIdentity::stable_release("0.159.3", "native_transcript")
            .unwrap()
    });
    n.unavailable_reason = (!runtime).then(|| d.resumed_unavailable_reason.unwrap().into());
    n
}

// Catch suppression-marker expiry/eviction granting creating-CLI lifecycle.
fn assert_codex_suppression_after_forgetting(expire: bool) {
    let fx = Fx::new(if expire {
        "codex-sticky-expiry"
    } else {
        "codex-sticky-eviction"
    });
    let recorder = HarnessEvidenceRecorderV2::new(fx.store.clone(), None, fx.clock.clone());
    recorder
        .record(&codex_note(false, "SessionStart", "resumed-old"), &budget())
        .unwrap();
    if expire {
        fx.advance(PENDING_MAX_AGE_MS);
    } else {
        for i in 0..PENDING_MAX_ENTRIES {
            fx.advance(1);
            let mut n = v2_note(false, "SessionStart");
            n.session_id = Some(format!("unrelated-{i}"));
            recorder.record(&n, &budget()).unwrap();
        }
    }
    recorder
        .record(&codex_note(true, "SessionStart", "resumed-old"), &budget())
        .unwrap();
    assert!(
        !recorder
            .record(&codex_note(true, "PreToolUse", "resumed-old"), &budget())
            .unwrap(),
        "forgotten suppression must not credit creating CLI"
    );
    let rows = fx
        .store
        .harness_evidence_v2_all("codex", 0, &budget())
        .unwrap();
    assert!(!rows[0].milestones.contains_key("lifecycle"));
    assert!(
        recorder
            .record(
                &codex_note(true, "SessionStart", "genuinely-new"),
                &budget()
            )
            .unwrap(),
        "an unrelated fresh lifecycle remains eligible"
    );
}
#[test]
fn v2_resumed_suppression_survives_pending_ttl() {
    assert_codex_suppression_after_forgetting(true);
}
#[test]
fn v2_resumed_suppression_survives_pending_capacity_eviction() {
    assert_codex_suppression_after_forgetting(false);
}

#[test]
fn v2_saturated_suppression_filter_withholds_only_lifecycle_credit() {
    let fx = Fx::new("codex-filter-saturated");
    let recorder = HarnessEvidenceRecorderV2::new(fx.store.clone(), None, fx.clock.clone());
    // Worst-case collision/saturation fixture, not a native observation.
    recorder.pending.lock().unwrap().2.0.fill(u64::MAX);
    recorder
        .record(
            &codex_note(true, "SessionStart", "new-but-colliding"),
            &budget(),
        )
        .unwrap();
    assert!(
        !recorder
            .record(
                &codex_note(true, "PreToolUse", "new-but-colliding"),
                &budget()
            )
            .unwrap()
    );
    let rows = fx
        .store
        .harness_evidence_v2_all("codex", 0, &budget())
        .unwrap();
    assert!(!rows[0].milestones.contains_key("lifecycle"));
    assert!(
        rows[0].milestones.contains_key("tool"),
        "filter cannot suppress independent tool observations"
    );
}

#[test]
fn mixed_pending_paths_can_evict_v2_and_expire_both_lanes() {
    let fx = Fx::new("mixed-pending-v2-oldest");
    let recorder = HarnessEvidenceRecorderV2::new(fx.store.clone(), None, fx.clock.clone())
        .with_legacy_pending(&fx.recorder);
    recorder
        .record(&v2_note(false, "SessionStart"), &budget())
        .unwrap();
    for i in 0..PENDING_MAX_ENTRIES {
        fx.advance(1);
        fx.recorder
            .record(
                &note(None, "SessionStart", ok(), Some(&format!("legacy-{i}"))),
                &budget(),
            )
            .unwrap();
    }
    assert!(
        !recorder
            .record(&v2_note(true, "PreToolUse"), &budget())
            .unwrap(),
        "legacy entries evict oldest v2 hold"
    );
    recorder
        .record(&v2_note(false, "SessionStart"), &budget())
        .unwrap();
    fx.advance(PENDING_MAX_AGE_MS);
    assert!(
        !fx.recorder
            .record(
                &note(Some("2.1.286"), "PreToolUse", ok(), Some("legacy-1023")),
                &budget()
            )
            .unwrap(),
        "shared legacy hold expires"
    );
    assert!(
        !recorder
            .record(&v2_note(true, "PreToolUse"), &budget())
            .unwrap(),
        "shared v2 hold expires"
    );
}

#[test]
fn v2_suppression_collision_cannot_flush_an_older_held_lifecycle() {
    let fx = Fx::new("codex-filter-held-collision");
    let recorder = HarnessEvidenceRecorderV2::new(fx.store.clone(), None, fx.clock.clone());
    let mut held = codex_note(false, "SessionStart", "held-before-collision");
    held.unavailable_reason = Some("awaiting transcript".into());
    recorder.record(&held, &budget()).unwrap();
    // Other suppression marks can make an already-held key collide.
    recorder.pending.lock().unwrap().2.0.fill(u64::MAX);
    assert!(
        !recorder
            .record(
                &codex_note(true, "PreToolUse", "held-before-collision"),
                &budget()
            )
            .unwrap(),
        "conservative filter also applies to held lifecycle flush"
    );
    let rows = fx
        .store
        .harness_evidence_v2_all("codex", 0, &budget())
        .unwrap();
    assert!(!rows[0].milestones.contains_key("lifecycle"));
    assert!(rows[0].milestones.contains_key("tool"));
}

// Exercises the real parser/cache source and recorder together; a cached domain
// skips refresh but still cannot supply the missing local tool milestone.
#[test]
fn v2_real_runtime_manifest_lookup_skips_refresh_without_local_credit() {
    let fx = Fx::new("v2-real-manifest");
    let cache = fx._iso.state_root().join("rich-manifest");
    std::fs::create_dir_all(&cache).unwrap();
    let mut d: serde_json::Value = serde_json::from_str(include_str!(
        "../harness/testdata/manifest/runtime-schema2.json"
    ))
    .unwrap();
    d.as_object_mut().unwrap().remove("generated_at");
    std::fs::write(cache.join("harness-versions.json"), d.to_string()).unwrap();
    let service = Arc::new(ManifestService::new(
        cache,
        ManifestPolicy::Off(crate::harness::manifest::OffReason::Settings),
        Arc::new(NeverFetch),
        fx.clock.clone(),
        Arc::new(|_| {}),
    ));
    let recorder = HarnessEvidenceRecorderV2::new(
        fx.store.clone(),
        Some(fx.triggers.clone()),
        fx.clock.clone(),
    )
    .with_legacy_pending(&fx.recorder)
    .with_rich_manifest_source(service);
    let mut n = v2_note(true, "SessionStart");
    n.runtime = Some(serde_json::from_value(d["runtime_rows"][0]["identity"].clone()).unwrap());
    assert!(!recorder.record(&n, &budget()).unwrap());
    assert!(fx.triggers.calls().is_empty());
    let rows = fx
        .store
        .harness_evidence_v2_all("claude", 0, &budget())
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].milestones.len(), 1);
    n.event = "PreToolUse".into();
    assert!(recorder.record(&n, &budget()).unwrap());
}
struct NeverFetch;
impl Fetcher for NeverFetch {
    fn fetch(&self, _: &str, _: Option<&str>) -> Result<FetchOutcome, FetchError> {
        panic!("offline manifest test must not fetch")
    }
}

// Catches crediting a held Codex startup to later creator metadata.
#[test]
fn task3_versionless_codex_start_is_never_credited_to_creator_metadata() {
    let fx = Fx::new("task3-no-codex-hold");
    let mut start = note(None, "SessionStart", ok(), Some("s"));
    start.harness = "codex".into();
    start.unattributed_reason = Some("runtime metadata unavailable".into());
    assert!(!fx.recorder.record(&start, &budget()).unwrap());
    start.version = Some("0.159.3".into());
    start.event = "PreToolUse".into();
    assert!(
        !fx.recorder.record(&start, &budget()).unwrap(),
        "unknown startup must not verify a later creator row"
    );
}

// Exercises runtime-unavailable V2 producer -> real SQLite -> both projections.
#[test]
fn absorption_v2_unavailable_violation_is_sticky_without_runtime_credit() {
    use crate::daemon::harness_states::{HarnessStatesProvider, embedded_source};
    let fx = Fx::new("absorption-v2-diagnostic");
    let recorder = HarnessEvidenceRecorderV2::new(fx.store.clone(), None, fx.clock.clone());
    let mut note = v2_note(false, "PreToolUse");
    note.outcome = crate::protocol::commands::HarnessEvidenceOutcomeV2::Violation {
        field: "tool_name".into(),
    };
    assert!(!recorder.record(&note, &budget()).unwrap());
    let first = fx.store.contract_diagnostics("claude", &budget()).unwrap();
    assert_eq!(first.len(), 1);
    note.outcome = crate::protocol::commands::HarnessEvidenceOutcomeV2::Ok;
    assert!(!recorder.record(&note, &budget()).unwrap());
    assert_eq!(
        fx.store.contract_diagnostics("claude", &budget()).unwrap(),
        first
    );
    assert!(
        fx.store
            .harness_evidence_v2_all("claude", 0, &budget())
            .unwrap()
            .is_empty()
    );
    let provider = HarnessStatesProvider::new(
        fx.store.clone(),
        embedded_source(),
        fx.clock.clone(),
        Box::new(|_| None),
        None,
    )
    .with_observations(Box::new(|| Ok(Default::default())));
    let report = provider.report_v2(&budget()).unwrap();
    assert!(report.harnesses["claude"].runtime_evidence.is_empty());
    assert!(
        report.harnesses["claude"]
            .limitations
            .iter()
            .any(|line| line.contains("contract input failure"))
    );
    assert!(
        provider
            .health_lines(&budget())
            .unwrap()
            .iter()
            .any(|line| line.contains("contract input failure"))
    );
    assert!(
        provider
            .report(&budget())
            .unwrap()
            .harnesses
            .iter()
            .any(|h| h.harness == "claude"
                && h.unattributed
                    .as_ref()
                    .is_some_and(|u| u.reason.contains("contract input failure")))
    );
    assert!(fx.triggers.calls().is_empty());
    // Store failure must remain observable, rather than an empty success projection.
    let db = rusqlite::Connection::open(fx._iso.state_root().join("store.db")).unwrap();
    db.execute_batch("DROP TABLE harness_contract_diagnostics")
        .unwrap();
    note.outcome = crate::protocol::commands::HarnessEvidenceOutcomeV2::Violation {
        field: "tool_name".into(),
    };
    assert!(recorder.record(&note, &budget()).is_err());
    assert!(provider.report_v2(&budget()).is_err());
    assert!(provider.report(&budget()).is_err());
    assert!(provider.health_lines(&budget()).is_err());
}

// Creator-only unavailability does not hold lifecycle or suppress future real starts.
#[test]
fn absorption_codex_creator_only_neither_holds_nor_filters_lifecycle() {
    let fx = Fx::new("absorption-creator-only");
    let recorder = HarnessEvidenceRecorderV2::new(fx.store.clone(), None, fx.clock.clone());
    let mut start = codex_note(false, "SessionStart", "fresh-creator");
    start.unavailable_reason = Some(
        crate::harness::attribution::Unattributed::CodexCreatorOnly
            .as_str()
            .into(),
    );
    assert!(!recorder.record(&start, &budget()).unwrap());
    assert_eq!(
        recorder.pending.lock().unwrap().1.0.len(),
        0,
        "creator-only lifecycle held"
    );
    let tool = codex_note(true, "PreToolUse", "fresh-creator");
    assert!(!recorder.record(&tool, &budget()).unwrap());
    let rows = fx
        .store
        .harness_evidence_v2_all("codex", 0, &budget())
        .unwrap();
    assert!(!rows[0].milestones.contains_key("lifecycle"));
    let real_start = codex_note(true, "SessionStart", "fresh-creator");
    assert!(
        recorder.record(&real_start, &budget()).unwrap(),
        "creator-only must not create a sticky resume filter"
    );
    let mut default_start = codex_note(false, "SessionStart", "ordinary-unavailability");
    default_start.unavailable_reason = Some("awaiting transcript".into());
    assert!(!recorder.record(&default_start, &budget()).unwrap());
    assert_eq!(
        recorder.pending.lock().unwrap().1.0.len(),
        1,
        "other declared holding policy changed"
    );
}

struct NonholdingFixture(&'static [&'static str]);
impl crate::harness::adapter::HarnessAdapter for NonholdingFixture {
    type Admission = ();
    fn metadata(&self) -> &'static crate::harness::adapter::AdapterMetadata {
        FourthEvidenceAdapter.metadata()
    }
    fn contracts(&self) -> &'static [crate::harness::adapter::ContractDescriptor] {
        FourthEvidenceAdapter.contracts()
    }
    fn nonholding_unavailable_reasons(
        &self,
        _: &crate::harness::adapter::ContractDescriptor,
    ) -> &'static [&'static str] {
        self.0
    }
    fn observe_install(
        &self,
        e: &crate::harness::adapter::InstallEnvironment,
        b: &CallBudget,
    ) -> crate::harness::adapter::InstallObservation {
        FourthEvidenceAdapter.observe_install(e, b)
    }
    fn admit(
        &self,
        r: &crate::harness::adapter::AdmissionRequest,
        b: &CallBudget,
    ) -> crate::harness::adapter::AdmissionDecision<()> {
        FourthEvidenceAdapter.admit(r, b)
    }
    fn version_ladder(
        &self,
        i: &crate::harness::runtime::RuntimeIdentity,
    ) -> crate::harness::adapter::Ladder {
        FourthEvidenceAdapter.version_ladder(i)
    }
    fn classify(
        &self,
        i: &crate::harness::adapter::HookInput,
    ) -> crate::harness::adapter::ContractObservation {
        FourthEvidenceAdapter.classify(i)
    }
    fn decode(
        &self,
        a: &(),
        i: &crate::harness::adapter::HookInput,
    ) -> Result<crate::harness::adapter::DecodedEvent, crate::harness::adapter::DecodeFailure> {
        FourthEvidenceAdapter.decode(a, i)
    }
    fn encode(
        &self,
        a: &(),
        e: &crate::harness::adapter::DecodedEvent,
        o: &crate::harness::adapter::NeutralOffer,
    ) -> Result<crate::harness::adapter::EncodedOutput, crate::harness::adapter::EncodeFailure>
    {
        FourthEvidenceAdapter.encode(a, e, o)
    }
    fn attribute_runtime(
        &self,
        i: &crate::harness::adapter::HookInput,
        b: &CallBudget,
    ) -> crate::harness::adapter::RuntimeAttribution {
        FourthEvidenceAdapter.attribute_runtime(i, b)
    }
    fn setup(
        &self,
        r: &crate::harness::adapter::SetupRequest,
        b: &CallBudget,
    ) -> Result<crate::harness::adapter::SetupOutcome, crate::harness::adapter::SetupFailure> {
        FourthEvidenceAdapter.setup(r, b)
    }
    fn status(
        &self,
        r: &crate::harness::adapter::StatusRequest,
        b: &CallBudget,
    ) -> crate::harness::adapter::SetupStatus {
        FourthEvidenceAdapter.status(r, b)
    }
    fn unsetup(
        &self,
        r: &crate::harness::adapter::UnsetupRequest,
        b: &CallBudget,
    ) -> Result<crate::harness::adapter::RemovalOutcome, crate::harness::adapter::SetupFailure>
    {
        FourthEvidenceAdapter.unsetup(r, b)
    }
}
#[test]
fn absorption_nonholding_policy_bounds_and_exact_descriptor_membership() {
    use crate::harness::{
        adapter::HarnessAdapter,
        registry::{Registration, Registry},
    };
    for reasons in [
        &[""][..],
        &["same", "same"][..],
        &["bad\nreason"][..],
        &["1", "2", "3", "4", "5", "6", "7", "8", "9"][..],
    ] {
        let adapter = Box::leak(Box::new(NonholdingFixture(reasons)));
        let entries = Box::leak(vec![Registration::new(adapter)].into_boxed_slice());
        assert!(
            Registry::new(entries).is_err(),
            "accepted invalid nonholding policy {reasons:?}"
        );
    }
    let long: &'static str = Box::leak("x".repeat(129).into_boxed_str());
    let reasons = Box::leak(vec![long].into_boxed_slice());
    let entries = Box::leak(
        vec![Registration::new(Box::leak(Box::new(NonholdingFixture(
            reasons,
        ))))]
        .into_boxed_slice(),
    );
    assert!(Registry::new(entries).is_err());
    let entries = Box::leak(
        vec![Registration::new(Box::leak(Box::new(NonholdingFixture(
            &["creator only"],
        ))))]
        .into_boxed_slice(),
    );
    let registry = Registry::new(entries).unwrap();
    let r = registry.by_id(registry.agent("fourth").unwrap()).unwrap();
    let d = &r.contracts()[0];
    let before = d.contract_id_v2().unwrap();
    assert_eq!(r.nonholding_unavailable_reasons(d), &["creator only"]);
    let copy = *d;
    assert!(
        r.nonholding_unavailable_reasons(&copy).is_empty(),
        "policy escaped exact registered descriptor"
    );
    assert_eq!(d.contract_id_v2().unwrap(), before);
    assert!(
        FourthEvidenceAdapter
            .nonholding_unavailable_reasons(d)
            .is_empty()
    );
}

// A delegating StorePort pauses before the real SQLite transaction, never models
// qualification or milestones itself. Unused port methods also delegate unchanged.
mod publication_order {
    use super::*;
    use crate::ports::*;
    use crate::protocol::{
        authority::*, commands::*, ids::*, pagination::*, results::*, service::*, time::*,
    };

    use std::sync::{atomic::AtomicBool, mpsc};
    struct PausedStore {
        inner: Arc<SqliteStore>,
        once: AtomicBool,
        entered: mpsc::Sender<()>,
        release: Mutex<mpsc::Receiver<()>>,
    }
    impl StorePort for PausedStore {
        fn clock(&self) -> &dyn Clock {
            self.inner.clock()
        }
        fn archival_pass(
            &self,
            runtime: &crate::store::archival::Runtime,
            hints: &[crate::archival_legacy::Hint],
            budget: &CallBudget,
        ) -> Result<crate::store::archival::Progress, ApiError> {
            self.inner.archival_pass(runtime, hints, budget)
        }
        fn archival_next(
            &self,
            runtime: &crate::store::archival::Runtime,
            budget: &CallBudget,
        ) -> Result<crate::store::archival::ObservationWork, ApiError> {
            self.inner.archival_next(runtime, budget)
        }
        fn archival_sample(
            &self,
            runtime: &crate::store::archival::Runtime,
            ticket: &crate::store::archival::ObservationTicket,
            sample: Option<&ComposerObservation>,
            budget: &CallBudget,
        ) -> Result<bool, ApiError> {
            self.inner.archival_sample(runtime, ticket, sample, budget)
        }
        fn audit_service_disconnect(
            &self,
            boot: &str,
            generation: u64,
            peer: crate::protocol::authority::PeerIdentity,
            budget: &CallBudget,
        ) -> Result<(), ApiError> {
            self.inner
                .audit_service_disconnect(boot, generation, peer, budget)
        }
        fn service_operation(
            &self,
            operation: ServiceOperation,
            connection: &ServiceConnectionAuthority,
            gate: &dyn ServiceAuthorityGate,
            budget: &CallBudget,
            admission: Option<&crate::service::fair_writer::FairWriter>,
        ) -> Result<ServiceResult, ApiError> {
            self.inner
                .service_operation(operation, connection, gate, budget, admission)
        }
        fn query(
            &self,
            command: &Command,
            read: &ReadContext,
            budget: &CallBudget,
        ) -> Result<CommandResult, ApiError> {
            self.inner.query(command, read, budget)
        }
        fn mutate(
            &self,
            command: PermitMutation,
            permit: MutationPermit,
            budget: &CallBudget,
        ) -> Result<CommandResult, ApiError> {
            self.inner.mutate(command, permit, budget)
        }
        fn prepare_send_step(
            &self,
            request: &SendMessage,
            admission: DurableWorkAdmission,
            budget: &CallBudget,
        ) -> Result<SendPreparationProgress, ApiError> {
            self.inner.prepare_send_step(request, admission, budget)
        }
        fn abandon_send_preparation(
            &self,
            expected_preparation_id: &str,
            budget: &CallBudget,
        ) -> Result<(), ApiError> {
            self.inner
                .abandon_send_preparation(expected_preparation_id, budget)
        }
        fn resolve_seat(
            &self,
            request: ResolveSeat,
            attempt: OrdinaryResolutionAttempt,
            budget: &CallBudget,
        ) -> Result<OrdinaryResolutionOutcome, ApiError> {
            self.inner.resolve_seat(request, attempt, budget)
        }
        fn check_resolved_target(
            &self,
            check: ResolvedTargetCheck,
            budget: &CallBudget,
        ) -> Result<(), ApiError> {
            self.inner.check_resolved_target(check, budget)
        }
        fn replay_operator(
            &self,
            command: OperatorCommand,
            actor: OperatorActor,
            budget: &CallBudget,
        ) -> Result<Option<CommandResult>, ApiError> {
            self.inner.replay_operator(command, actor, budget)
        }
        fn mutate_operator(
            &self,
            command: OperatorRequest,
            actor: OperatorActor,
            budget: &CallBudget,
        ) -> Result<CommandResult, ApiError> {
            self.inner.mutate_operator(command, actor, budget)
        }
        fn replay_continuity(
            &self,
            command: ContinuityCheckIn,
            budget: &CallBudget,
        ) -> Result<Option<CommandResult>, ApiError> {
            self.inner.replay_continuity(command, budget)
        }
        fn decide_continuity(
            &self,
            request: ContinuityRequest,
            budget: &CallBudget,
        ) -> Result<CommandResult, ApiError> {
            self.inner.decide_continuity(request, budget)
        }
        fn record_managed_launch(
            &self,
            command: crate::protocol::commands::RecordManagedLaunch,
            budget: &CallBudget,
        ) -> Result<CommandResult, ApiError> {
            self.inner.record_managed_launch(command, budget)
        }
        fn issue_cooperative_permit(
            &self,
            request: CooperativePermitRequest,
            budget: &CallBudget,
        ) -> Result<MutationPermit, ApiError> {
            self.inner.issue_cooperative_permit(request, budget)
        }
        fn register_available(
            &self,
            request: RegisterAvailableRequest,
            permit: MutationPermit,
            budget: &CallBudget,
        ) -> Result<CommandResult, ApiError> {
            self.inner.register_available(request, permit, budget)
        }
        fn persisted_host_epoch(
            &self,
            _instance: &str,
            _budget: &CallBudget,
        ) -> Result<u64, ApiError> {
            self.inner.persisted_host_epoch(_instance, _budget)
        }
        fn begin_host_observation(
            &self,
            instance: &str,
            budget: &CallBudget,
        ) -> Result<HostObservationAdmission, ApiError> {
            self.inner.begin_host_observation(instance, budget)
        }
        fn publish_current_target_observation(
            &self,
            admission: &HostObservationAdmission,
            observation: &HostObservation,
            budget: &CallBudget,
        ) -> Result<bool, ApiError> {
            self.inner
                .publish_current_target_observation(admission, observation, budget)
        }
        fn invalidate_host_observation(
            &self,
            admission: &HostObservationAdmission,
            reason: HostInvalidationReason,
            budget: &CallBudget,
        ) -> Result<Option<HostInvalidationFence>, ApiError> {
            self.inner
                .invalidate_host_observation(admission, reason, budget)
        }
        fn mark_unresolved_from_invalidation(
            &self,
            transition: GuardedInvalidationTransition,
            budget: &CallBudget,
        ) -> Result<ReconciliationOutcome, ApiError> {
            self.inner
                .mark_unresolved_from_invalidation(transition, budget)
        }
        fn saved_seats_page_for_invalidation(
            &self,
            fence: &HostInvalidationFence,
            after_ordinal: u64,
            high_water_ordinal: Option<u64>,
            limit: u8,
            budget: &CallBudget,
        ) -> Result<InvalidationSeatPage, ApiError> {
            self.inner.saved_seats_page_for_invalidation(
                fence,
                after_ordinal,
                high_water_ordinal,
                limit,
                budget,
            )
        }
        fn begin_snapshot_stage(
            &self,
            header: SnapshotHeader,
            budget: &CallBudget,
        ) -> Result<SnapshotStage, ApiError> {
            self.inner.begin_snapshot_stage(header, budget)
        }
        fn stage_snapshot_targets(
            &self,
            stage: &SnapshotGenerationId,
            offset: u64,
            targets: &[HostObservation],
            admission: DurableWorkAdmission,
            budget: &CallBudget,
        ) -> Result<SnapshotStageProgress, ApiError> {
            self.inner
                .stage_snapshot_targets(stage, offset, targets, admission, budget)
        }
        fn seal_snapshot_stage(
            &self,
            stage: &SnapshotGenerationId,
            budget: &CallBudget,
        ) -> Result<SnapshotStage, ApiError> {
            self.inner.seal_snapshot_stage(stage, budget)
        }
        fn publish_snapshot_stage(
            &self,
            stage: &SnapshotGenerationId,
            budget: &CallBudget,
        ) -> Result<PublishedSnapshot, ApiError> {
            self.inner.publish_snapshot_stage(stage, budget)
        }
        fn discard_snapshot_stage(
            &self,
            stage: &SnapshotGenerationId,
            admission: DurableWorkAdmission,
            budget: &CallBudget,
        ) -> Result<SnapshotCleanupProgress, ApiError> {
            self.inner.discard_snapshot_stage(stage, admission, budget)
        }
        fn prune_retention(&self, budget: &CallBudget) -> Result<PruneProgress, ApiError> {
            self.inner.prune_retention(budget)
        }
        fn record_harness_evidence_v2(
            &self,
            record: &crate::store::harness_evidence::EvidenceRecordV2<'_>,
            budget: &CallBudget,
        ) -> Result<crate::store::harness_evidence::RecordedV2, ApiError> {
            if record.event == "SessionStart" && self.once.swap(false, Ordering::SeqCst) {
                self.entered.send(()).unwrap();
                self.release
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(3))
                    .map_err(|_| ApiError::invalid_request("test pause expired"))?;
            }
            self.inner.record_harness_evidence_v2(record, budget)
        }
        fn harness_evidence_v2(
            &self,
            harness: &str,
            identity: &str,
            domain: &str,
            origin: crate::harness::evidence::EvidenceOrigin,
            contract: &str,
            budget: &CallBudget,
        ) -> Result<Option<crate::store::harness_evidence::EvidenceRowV2>, ApiError> {
            self.inner
                .harness_evidence_v2(harness, identity, domain, origin, contract, budget)
        }
        fn harness_evidence_v2_all(
            &self,
            harness: &str,
            since_ms: u64,
            budget: &CallBudget,
        ) -> Result<Vec<crate::store::harness_evidence::EvidenceRowV2>, ApiError> {
            self.inner
                .harness_evidence_v2_all(harness, since_ms, budget)
        }
        fn harness_evidence_v2_since(
            &self,
            since_ms: u64,
            budget: &CallBudget,
        ) -> Result<Vec<crate::store::harness_evidence::EvidenceRowV2>, ApiError> {
            self.inner.harness_evidence_v2_since(since_ms, budget)
        }
        fn record_unattributed_v2(
            &self,
            harness: &str,
            domain: &str,
            origin: crate::harness::evidence::EvidenceOrigin,
            reason: &str,
            budget: &CallBudget,
        ) -> Result<(), ApiError> {
            self.inner
                .record_unattributed_v2(harness, domain, origin, reason, budget)
        }
        fn last_unattributed_v2(
            &self,
            harness: &str,
            domain: &str,
            origin: crate::harness::evidence::EvidenceOrigin,
            budget: &CallBudget,
        ) -> Result<Option<(String, u64)>, ApiError> {
            self.inner
                .last_unattributed_v2(harness, domain, origin, budget)
        }
        fn record_harness_evidence(
            &self,
            record: &crate::store::harness_evidence::EvidenceRecord<'_>,
            budget: &CallBudget,
        ) -> Result<crate::store::harness_evidence::Recorded, ApiError> {
            self.inner.record_harness_evidence(record, budget)
        }
        fn harness_evidence(
            &self,
            harness: &str,
            version: &str,
            contract_id: &str,
            budget: &CallBudget,
        ) -> Result<Option<crate::store::harness_evidence::EvidenceRow>, ApiError> {
            self.inner
                .harness_evidence(harness, version, contract_id, budget)
        }
        fn harness_evidence_since(
            &self,
            since_ms: u64,
            budget: &CallBudget,
        ) -> Result<Vec<crate::store::harness_evidence::EvidenceRow>, ApiError> {
            self.inner.harness_evidence_since(since_ms, budget)
        }
        fn harness_evidence_all(
            &self,
            harness: &str,
            budget: &CallBudget,
        ) -> Result<Vec<crate::store::harness_evidence::EvidenceRow>, ApiError> {
            self.inner.harness_evidence_all(harness, budget)
        }
        fn record_contract_diagnostic(
            &self,
            record: &crate::store::harness_evidence::DiagnosticRecord<'_>,
            budget: &CallBudget,
        ) -> Result<(), ApiError> {
            self.inner.record_contract_diagnostic(record, budget)
        }
        fn contract_diagnostics(
            &self,
            harness: &str,
            budget: &CallBudget,
        ) -> Result<Vec<crate::store::harness_evidence::DiagnosticRow>, ApiError> {
            self.inner.contract_diagnostics(harness, budget)
        }
        fn record_unattributed(
            &self,
            harness: &str,
            reason: &str,
            budget: &CallBudget,
        ) -> Result<(), ApiError> {
            self.inner.record_unattributed(harness, reason, budget)
        }
        fn last_unattributed(
            &self,
            harness: &str,
            budget: &CallBudget,
        ) -> Result<Option<(String, u64)>, ApiError> {
            self.inner.last_unattributed(harness, budget)
        }
        fn saved_seats_page(
            &self,
            published: &SnapshotGenerationId,
            after_ordinal: u64,
            high_water_ordinal: Option<u64>,
            limit: u8,
            budget: &CallBudget,
        ) -> Result<SnapshotSeatPage, ApiError> {
            self.inner
                .saved_seats_page(published, after_ordinal, high_water_ordinal, limit, budget)
        }
        fn apply_reconciliation_transition(
            &self,
            transition: GuardedSeatTransition,
            budget: &CallBudget,
        ) -> Result<ReconciliationOutcome, ApiError> {
            self.inner
                .apply_reconciliation_transition(transition, budget)
        }
        fn record_reconciliation_pass(
            &self,
            published: &PublishedSnapshot,
            budget: &CallBudget,
        ) -> Result<bool, ApiError> {
            self.inner.record_reconciliation_pass(published, budget)
        }
        fn due_obligations(
            &self,
            request: DueScanRequest,
            budget: &CallBudget,
        ) -> Result<DueScanProgress, ApiError> {
            self.inner.due_obligations(request, budget)
        }
        fn begin_retirement(
            &self,
            seat: SeatId,
            proof: ClosureEvidence,
            budget: &CallBudget,
        ) -> Result<RetirementJob, ApiError> {
            self.inner.begin_retirement(seat, proof, budget)
        }
        fn advance_retirement(
            &self,
            job: RetirementJobId,
            admission: WorkAdmission,
            budget: &CallBudget,
        ) -> Result<RetirementProgress, ApiError> {
            self.inner.advance_retirement(job, admission, budget)
        }
        fn pending_retirement_jobs(
            &self,
            page: PageRequest,
            budget: &CallBudget,
        ) -> Result<Page<RetirementStatus>, ApiError> {
            self.inner.pending_retirement_jobs(page, budget)
        }
        fn binding_evidence_startup(&self) -> Option<BindingEvidenceStartup> {
            self.inner.binding_evidence_startup()
        }
        fn binding_evidence_current(
            &self,
            budget: &CallBudget,
        ) -> Result<Option<BindingEvidenceStartup>, ApiError> {
            self.inner.binding_evidence_current(budget)
        }
        fn unresolved_seat_summary(
            &self,
            budget: &CallBudget,
        ) -> Result<UnresolvedSeatSummary, ApiError> {
            self.inner.unresolved_seat_summary(budget)
        }
        fn retirement_summary(&self, _budget: &CallBudget) -> Result<RetirementSummary, ApiError> {
            self.inner.retirement_summary(_budget)
        }
        fn summary(
            &self,
            _request: &crate::protocol::summary::SummaryRequest,
            _budget: &CallBudget,
        ) -> Result<crate::protocol::summary::SummaryOutcome, ApiError> {
            self.inner.summary(_request, _budget)
        }
        fn summary_job(
            &self,
            _request: &crate::protocol::summary::SummaryJobRequest,
            _budget: &CallBudget,
        ) -> Result<crate::protocol::summary::SummaryJobOutcome, ApiError> {
            self.inner.summary_job(_request, _budget)
        }
        fn summary_submit(
            &self,
            _request: &crate::protocol::summary::SummarySubmitRequest,
            _budget: &CallBudget,
        ) -> Result<crate::protocol::summary::SubmitOutcome, ApiError> {
            self.inner.summary_submit(_request, _budget)
        }
        fn wake_candidates(
            &self,
            page: PageRequest,
            budget: &CallBudget,
        ) -> Result<Page<WakeCandidate>, ApiError> {
            self.inner.wake_candidates(page, budget)
        }
        fn pending_work(
            &self,
            page: PageRequest,
            budget: &CallBudget,
        ) -> Result<Page<WorkCandidate>, ApiError> {
            self.inner.pending_work(page, budget)
        }
        fn advance_work(
            &self,
            job: &str,
            admission: DurableWorkAdmission,
            budget: &CallBudget,
        ) -> Result<WorkProgress, ApiError> {
            self.inner.advance_work(job, admission, budget)
        }
        fn wake_batch_seats(
            &self,
            _after: Option<&SeatId>,
            _limit: u16,
            _budget: &CallBudget,
        ) -> Result<Vec<SeatId>, ApiError> {
            self.inner.wake_batch_seats(_after, _limit, _budget)
        }
        fn clear_wake_batch_if_empty(
            &self,
            _seat: &SeatId,
            _budget: &CallBudget,
        ) -> Result<bool, ApiError> {
            self.inner.clear_wake_batch_if_empty(_seat, _budget)
        }
        fn wake_batch_window(
            &self,
            _candidate: &WakeCandidate,
            _budget: &CallBudget,
        ) -> Result<Option<(UtcMillis, u64)>, ApiError> {
            self.inner.wake_batch_window(_candidate, _budget)
        }
        fn reserve_wake(
            &self,
            candidate: &WakeCandidate,
            budget: &CallBudget,
        ) -> Result<Option<WakeReservation>, ApiError> {
            self.inner.reserve_wake(candidate, budget)
        }
        fn wake_recovery_candidates(
            &self,
            page: PageRequest,
            budget: &CallBudget,
        ) -> Result<Page<WakeRecoveryCandidate>, ApiError> {
            self.inner.wake_recovery_candidates(page, budget)
        }
        fn recover_wake_reservation(
            &self,
            request: WakeRecoveryRequest,
            budget: &CallBudget,
        ) -> Result<WakeRecoveryOutcome, ApiError> {
            self.inner.recover_wake_reservation(request, budget)
        }
        fn validate_wake_reservation(
            &self,
            reservation: &WakeReservation,
            budget: &CallBudget,
        ) -> Result<bool, ApiError> {
            self.inner.validate_wake_reservation(reservation, budget)
        }
        fn complete_wake(
            &self,
            attempt: WakeAttemptId,
            outcome: WakeOutcome,
            refused_restore: Option<&PriorLadder>,
            budget: &CallBudget,
        ) -> Result<bool, ApiError> {
            self.inner
                .complete_wake(attempt, outcome, refused_restore, budget)
        }
        fn poke_candidates(
            &self,
            _limit: u16,
            _budget: &CallBudget,
        ) -> Result<Vec<PokeDue>, ApiError> {
            self.inner.poke_candidates(_limit, _budget)
        }
        fn reserve_poke(
            &self,
            _due: &PokeDue,
            _budget: &CallBudget,
        ) -> Result<Option<PokeReservation>, ApiError> {
            self.inner.reserve_poke(_due, _budget)
        }
        fn complete_poke(
            &self,
            _attempt: WakeAttemptId,
            _outcome: WakeOutcome,
            _receipts: &[PokeReceipt],
            _budget: &CallBudget,
        ) -> Result<(), ApiError> {
            self.inner
                .complete_poke(_attempt, _outcome, _receipts, _budget)
        }
        fn poke_for_wake(
            &self,
            _seat: &SeatId,
            _budget: &CallBudget,
        ) -> Result<Option<PokeDue>, ApiError> {
            self.inner.poke_for_wake(_seat, _budget)
        }
    }

    struct UnlockedCallbacks {
        pending: Arc<Mutex<PendingSessionStarts>>,
        lookups: AtomicI64,
        triggers: AtomicI64,
    }
    impl RichManifestSource for UnlockedCallbacks {
        fn contains(
            &self,
            _: &str,
            _: &crate::harness::runtime::RuntimeIdentity,
            _: &str,
            _: crate::harness::evidence::EvidenceOrigin,
            _: &str,
        ) -> bool {
            assert!(
                self.pending.try_lock().is_ok(),
                "manifest lookup held publication fence"
            );
            self.lookups.fetch_add(1, Ordering::SeqCst);
            false
        }
    }
    impl ManifestTrigger for UnlockedCallbacks {
        fn ensure_manifest(&self, _: &str, _: FetchReason) {
            assert!(
                self.pending.try_lock().is_ok(),
                "manifest trigger held publication fence"
            );
            self.triggers.fetch_add(1, Ordering::SeqCst);
        }
    }
    #[test]
    fn committed_held_manifest_callbacks_unlock_even_when_direct_write_fails() {
        let fx = Fx::new("publication-callbacks");
        let callbacks = Arc::new(UnlockedCallbacks {
            pending: fx.recorder.pending.clone(),
            lookups: AtomicI64::new(0),
            triggers: AtomicI64::new(0),
        });
        let recorder = HarnessEvidenceRecorderV2::new(
            fx.store.clone(),
            Some(callbacks.clone()),
            fx.clock.clone(),
        )
        .with_legacy_pending(&fx.recorder)
        .with_rich_manifest_source(callbacks.clone());
        recorder
            .record(&v2_note(false, "SessionStart"), &budget())
            .unwrap();
        let db = rusqlite::Connection::open(fx._iso.state_root().join("store.db")).unwrap();
        db.execute_batch("CREATE TRIGGER reject_tool BEFORE UPDATE ON harness_contract_evidence_v2 WHEN NEW.milestones_json LIKE '%tool%' BEGIN SELECT RAISE(ABORT,'injected direct failure'); END;").unwrap();
        assert!(
            recorder
                .record(&v2_note(true, "PreToolUse"), &budget())
                .is_err()
        );
        assert_eq!(callbacks.lookups.load(Ordering::SeqCst), 1);
        assert_eq!(callbacks.triggers.load(Ordering::SeqCst), 1);
        let rows = fx
            .store
            .harness_evidence_v2_all("claude", 0, &budget())
            .unwrap();
        assert!(rows[0].milestones.contains_key("lifecycle"));
        assert!(!rows[0].milestones.contains_key("tool"));
        db.execute_batch("DROP TRIGGER reject_tool;").unwrap();
        assert!(
            recorder
                .record(&v2_note(true, "PreToolUse"), &budget())
                .unwrap()
        );
    }
    #[test]
    fn failed_held_publication_then_completed_suppression_cannot_resurrect_hold() {
        let fx = Fx::new("publication-restore");
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let paused = Arc::new(PausedStore {
            inner: fx.store.clone(),
            once: AtomicBool::new(true),
            entered: entered_tx,
            release: Mutex::new(release_rx),
        });
        let recorder = Arc::new(
            HarnessEvidenceRecorderV2::new(paused, None, fx.clock.clone())
                .with_legacy_pending(&fx.recorder),
        );
        let mut initial = codex_note(false, "SessionStart", "failure");
        initial.unavailable_reason = Some("runtime_metadata_missing".into());
        recorder.record(&initial, &budget()).unwrap();
        let db = rusqlite::Connection::open(fx._iso.state_root().join("store.db")).unwrap();
        db.execute_batch("CREATE TRIGGER reject_v2 BEFORE INSERT ON harness_contract_evidence_v2 BEGIN SELECT RAISE(ABORT,'injected held failure'); END;").unwrap();
        let note = codex_note(true, "PreToolUse", "failure");
        let resumed = codex_note(false, "SessionStart", "failure");
        let (attempt_tx, attempt_rx) = mpsc::channel();
        let (entered, attempted, published, suppressed) = std::thread::scope(|scope| {
            let publisher = scope.spawn(|| recorder.record(&note, &budget()));
            let entered = entered_rx.recv_timeout(Duration::from_secs(2));
            let suppressor = scope.spawn(|| {
                attempt_tx.send(()).unwrap();
                recorder.record(&resumed, &budget())
            });
            let attempted = attempt_rx.recv_timeout(Duration::from_secs(2));
            let _ = release_tx.send(());
            (entered, attempted, publisher.join(), suppressor.join())
        });
        entered.unwrap();
        attempted.unwrap();
        assert!(published.unwrap().is_err());
        suppressed.unwrap().unwrap();
        assert!(
            recorder.pending.lock().unwrap().1.0.is_empty(),
            "suppression must remove a restored hold"
        );
        db.execute_batch("DROP TRIGGER reject_v2;").unwrap();
        assert!(!recorder.record(&note, &budget()).unwrap());
        recorder
            .record(&codex_note(true, "SessionStart", "failure"), &budget())
            .unwrap();
        let rows = fx
            .store
            .harness_evidence_v2_all("codex", 0, &budget())
            .unwrap();
        assert!(!rows[0].milestones.contains_key("lifecycle"));
        assert!(rows[0].milestones.contains_key("tool"));
    }

    #[test]
    fn completed_resumed_suppression_prevents_held_and_direct_lifecycle_credit() {
        let mut stale_routes = Vec::new();
        for held in [true, false] {
            let fx = Fx::new("publication-order");
            let (entered_tx, entered_rx) = mpsc::channel();
            let (release_tx, release_rx) = mpsc::channel();
            let paused = Arc::new(PausedStore {
                inner: fx.store.clone(),
                once: AtomicBool::new(true),
                entered: entered_tx,
                release: Mutex::new(release_rx),
            });
            let recorder = Arc::new(
                HarnessEvidenceRecorderV2::new(paused, None, fx.clock.clone())
                    .with_legacy_pending(&fx.recorder),
            );
            let mut initial = codex_note(false, "SessionStart", "racing");
            initial.unavailable_reason = Some("runtime_metadata_missing".into());
            if held {
                recorder.record(&initial, &budget()).unwrap();
            }
            let note = codex_note(
                true,
                if held { "PreToolUse" } else { "SessionStart" },
                "racing",
            );
            let resumed = codex_note(false, "SessionStart", "racing");
            let (done_tx, done_rx) = mpsc::channel();
            let (attempt_tx, attempt_rx) = mpsc::channel();
            // No assertion/panic occurs while workers can be waiting for a release.
            let (entered, attempted, completed_first, published, suppressed) =
                std::thread::scope(|scope| {
                    let publisher = scope.spawn(|| recorder.record(&note, &budget()));
                    let entered = entered_rx.recv_timeout(Duration::from_secs(2));
                    let suppressor = scope.spawn(|| {
                        attempt_tx.send(()).unwrap();
                        let result = recorder.record(&resumed, &budget());
                        done_tx.send(result.is_ok()).unwrap();
                        result
                    });
                    let attempted = attempt_rx.recv_timeout(Duration::from_secs(2));
                    let completed_first = done_rx.recv_timeout(Duration::from_millis(250)).ok();
                    let _ = release_tx.send(());
                    (
                        entered,
                        attempted,
                        completed_first,
                        publisher.join(),
                        suppressor.join(),
                    )
                });
            entered.unwrap();
            attempted.unwrap();
            published.unwrap().unwrap();
            suppressed.unwrap().unwrap();
            let rows = fx
                .store
                .harness_evidence_v2_all("codex", 0, &budget())
                .unwrap();
            let credited = rows[0].milestones.contains_key("lifecycle");
            if completed_first == Some(true) && credited {
                stale_routes.push(if held { "extracted-held" } else { "direct" });
            }
            // A held publication fence makes commit-first legitimate: release before
            // joining suppression, then confirm that suppression actually completed.
            if completed_first.is_none() {
                assert!(credited, "commit-first should retain earlier lifecycle");
            }
        }
        assert!(
            stale_routes.is_empty(),
            "completed suppression followed by stale SQLite lifecycle credit: {stale_routes:?}"
        );
    }
}
