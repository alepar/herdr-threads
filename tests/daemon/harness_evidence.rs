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
