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
