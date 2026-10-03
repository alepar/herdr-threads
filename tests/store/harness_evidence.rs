//! Harness version evidence store (ht-xoc.4): verified gate, sticky violation,
//! flags, last_seen_at and unattributed reasons.
use super::*;
use crate::{
    ports::StorePort,
    protocol::time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis},
    store::{StoreSettings, connection::StoreContext},
    test_support::isolation::TestIsolation,
};
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

const HOUR_MS: i64 = 60 * 60 * 1000;
const T0: i64 = 1_000 * HOUR_MS;
const CONTRACT: &str = "0123456789abcdef";
const OTHER_CONTRACT: &str = "fedcba9876543210";

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

struct Fx {
    _iso: TestIsolation,
    store: crate::store::SqliteStore,
    clock: Arc<TestClock>,
}

impl Fx {
    fn new(label: &str) -> Self {
        let iso = TestIsolation::new(label);
        let clock = Arc::new(TestClock(AtomicI64::new(T0)));
        let path = iso.state_root().join("store.db");
        let store = crate::store::SqliteStore::new(
            StoreContext::new(path, clock.clone()),
            "i",
            StoreSettings::default(),
        )
        .unwrap();
        Self {
            _iso: iso,
            store,
            clock,
        }
    }

    fn advance(&self, ms: i64) {
        self.clock.0.fetch_add(ms, Ordering::SeqCst);
    }

    fn record(
        &self,
        version: &str,
        contract: &str,
        event: &str,
        class: EventClass,
        outcome: EvidenceOutcome,
    ) -> Recorded {
        self.store
            .record_harness_evidence(
                &EvidenceRecord {
                    harness: "claude",
                    version,
                    contract_id: contract,
                    event,
                    class,
                    outcome: &outcome,
                },
                &budget(),
            )
            .unwrap()
    }

    fn row(&self, version: &str, contract: &str) -> Option<EvidenceRow> {
        self.store
            .harness_evidence("claude", version, contract, &budget())
            .unwrap()
    }
}

#[test]
fn verified_after_one_lifecycle_and_one_tool_ok() {
    let fx = Fx::new("he-verified");
    let first = fx.record(
        "2.1.286",
        CONTRACT,
        "SessionStart",
        EventClass::Lifecycle,
        EvidenceOutcome::Ok,
    );
    assert!(!first.row.verified(), "lifecycle alone is not verified");
    assert_eq!(first.row.lifecycle_ok_at, Some(T0 as u64));
    assert_eq!(first.row.tool_ok_at, None);
    fx.advance(1000);
    let second = fx.record(
        "2.1.286",
        CONTRACT,
        "PreToolUse",
        EventClass::Tool,
        EvidenceOutcome::Ok,
    );
    assert!(second.row.verified());
    assert_eq!(second.row.tool_ok_at, Some(T0 as u64 + 1000));
    assert_eq!(
        second.row.lifecycle_ok_at,
        Some(T0 as u64),
        "the first lifecycle time is kept"
    );
    assert_eq!(fx.row("2.1.286", CONTRACT), Some(second.row));
}

#[test]
fn two_tool_oks_do_not_verify_without_lifecycle() {
    let fx = Fx::new("he-two-tool");
    fx.record(
        "2.1.286",
        CONTRACT,
        "PreToolUse",
        EventClass::Tool,
        EvidenceOutcome::Ok,
    );
    let again = fx.record(
        "2.1.286",
        CONTRACT,
        "PostToolUse",
        EventClass::Tool,
        EvidenceOutcome::Ok,
    );
    assert!(!again.row.verified());
}

#[test]
fn ok_other_event_does_not_verify() {
    let fx = Fx::new("he-other");
    fx.record(
        "2.1.286",
        CONTRACT,
        "SessionStart",
        EventClass::Lifecycle,
        EvidenceOutcome::Ok,
    );
    let other = fx.record(
        "2.1.286",
        CONTRACT,
        "Notification",
        EventClass::Other,
        EvidenceOutcome::Ok,
    );
    assert!(!other.row.verified());
    assert_eq!(other.row.tool_ok_at, None);
}

#[test]
fn violation_sticks() {
    let fx = Fx::new("he-sticks");
    let first = fx.record(
        "2.1.286",
        CONTRACT,
        "PreToolUse",
        EventClass::Tool,
        EvidenceOutcome::Violation {
            field: "tool_name".into(),
        },
    );
    assert_eq!(first.row.violation_field.as_deref(), Some("tool_name"));
    assert_eq!(first.row.violation_event.as_deref(), Some("PreToolUse"));
    fx.advance(500);
    let ok = fx.record(
        "2.1.286",
        CONTRACT,
        "PreToolUse",
        EventClass::Tool,
        EvidenceOutcome::Ok,
    );
    assert_eq!(ok.row.violation_at, Some(T0 as u64), "ok never clears it");
    fx.advance(500);
    let second = fx.record(
        "2.1.286",
        CONTRACT,
        "SessionStart",
        EventClass::Lifecycle,
        EvidenceOutcome::Violation {
            field: "session_id".into(),
        },
    );
    assert!(!second.fresh_violation);
    assert_eq!(second.row.violation_field.as_deref(), Some("tool_name"));
    assert_eq!(second.row.violation_event.as_deref(), Some("PreToolUse"));
    assert_eq!(second.row.violation_at, Some(T0 as u64));
}

#[test]
fn violation_after_verification_is_recorded() {
    let fx = Fx::new("he-after-verified");
    fx.record(
        "2.1.286",
        CONTRACT,
        "SessionStart",
        EventClass::Lifecycle,
        EvidenceOutcome::Ok,
    );
    fx.record(
        "2.1.286",
        CONTRACT,
        "PreToolUse",
        EventClass::Tool,
        EvidenceOutcome::Ok,
    );
    fx.advance(10);
    let violated = fx.record(
        "2.1.286",
        CONTRACT,
        "PostToolUse",
        EventClass::Tool,
        EvidenceOutcome::Violation {
            field: "tool_response".into(),
        },
    );
    assert!(violated.fresh_violation);
    assert!(violated.row.verified(), "verification facts are not erased");
    assert_eq!(violated.row.violation_at, Some(T0 as u64 + 10));
    assert_eq!(
        violated.row.violation_field.as_deref(),
        Some("tool_response")
    );
}

#[test]
fn new_version_or_contract_has_no_violation() {
    let fx = Fx::new("he-new-key");
    fx.record(
        "2.1.286",
        CONTRACT,
        "PreToolUse",
        EventClass::Tool,
        EvidenceOutcome::Violation { field: "x".into() },
    );
    let version = fx.record(
        "2.1.287",
        CONTRACT,
        "PreToolUse",
        EventClass::Tool,
        EvidenceOutcome::Ok,
    );
    assert!(version.created);
    assert_eq!(version.row.violation_at, None);
    let contract = fx.record(
        "2.1.286",
        OTHER_CONTRACT,
        "PreToolUse",
        EventClass::Tool,
        EvidenceOutcome::Ok,
    );
    assert!(contract.created);
    assert_eq!(contract.row.violation_at, None);
    assert!(
        fx.row("2.1.286", CONTRACT).unwrap().violation_at.is_some(),
        "the original row keeps its violation"
    );
}

#[test]
fn malformed_records_a_sighting_only() {
    let fx = Fx::new("he-malformed");
    let recorded = fx.record(
        "2.1.286",
        CONTRACT,
        "unknown",
        EventClass::Other,
        EvidenceOutcome::Malformed,
    );
    assert!(recorded.created);
    assert!(!recorded.fresh_violation);
    assert_eq!(recorded.row.violation_at, None);
    assert!(!recorded.row.verified());
}

#[test]
fn created_and_fresh_violation_flags() {
    let fx = Fx::new("he-flags");
    let first = fx.record(
        "2.1.286",
        CONTRACT,
        "SessionStart",
        EventClass::Lifecycle,
        EvidenceOutcome::Ok,
    );
    assert!(first.created && !first.fresh_violation);
    let second = fx.record(
        "2.1.286",
        CONTRACT,
        "PreToolUse",
        EventClass::Tool,
        EvidenceOutcome::Ok,
    );
    assert!(!second.created);
    let violation = fx.record(
        "2.1.286",
        CONTRACT,
        "PreToolUse",
        EventClass::Tool,
        EvidenceOutcome::Violation { field: "f".into() },
    );
    assert!(!violation.created && violation.fresh_violation);
    let again = fx.record(
        "2.1.286",
        CONTRACT,
        "PreToolUse",
        EventClass::Tool,
        EvidenceOutcome::Violation { field: "f".into() },
    );
    assert!(!again.fresh_violation);
    let new_row_violation = fx.record(
        "9.9.9",
        CONTRACT,
        "PreToolUse",
        EventClass::Tool,
        EvidenceOutcome::Violation { field: "f".into() },
    );
    assert!(
        new_row_violation.created && new_row_violation.fresh_violation,
        "a violation on a brand-new row is both"
    );
}

#[test]
fn last_seen_at_touched_on_every_record() {
    let fx = Fx::new("he-last-seen");
    let first = fx.record(
        "2.1.286",
        CONTRACT,
        "SessionStart",
        EventClass::Lifecycle,
        EvidenceOutcome::Ok,
    );
    assert_eq!(first.row.first_seen_at, T0 as u64);
    assert_eq!(first.row.last_seen_at, T0 as u64);
    for (i, outcome) in [
        EvidenceOutcome::Ok,
        EvidenceOutcome::Malformed,
        EvidenceOutcome::Violation { field: "f".into() },
    ]
    .into_iter()
    .enumerate()
    {
        fx.advance(100);
        let recorded = fx.record(
            "2.1.286",
            CONTRACT,
            "Notification",
            EventClass::Other,
            outcome,
        );
        assert_eq!(recorded.row.last_seen_at, T0 as u64 + 100 * (i as u64 + 1));
        assert_eq!(recorded.row.first_seen_at, T0 as u64);
    }
}

#[test]
fn heartbeat_keeps_a_long_session_in_window() {
    let fx = Fx::new("he-heartbeat");
    fx.record(
        "2.1.286",
        CONTRACT,
        "SessionStart",
        EventClass::Lifecycle,
        EvidenceOutcome::Ok,
    );
    fx.advance(25 * HOUR_MS);
    let now = (T0 + 25 * HOUR_MS) as u64;
    let window = now - 24 * HOUR_MS as u64;
    assert!(
        fx.store
            .harness_evidence_since(window, &budget())
            .unwrap()
            .is_empty(),
        "without a heartbeat the row left the 24 h window"
    );
    fx.record(
        "2.1.286",
        CONTRACT,
        "PostToolUse",
        EventClass::Tool,
        EvidenceOutcome::Ok,
    );
    let rows = fx.store.harness_evidence_since(window, &budget()).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].last_seen_at, now);
}

#[test]
fn since_and_all_cover_both_harnesses() {
    let fx = Fx::new("he-since-all");
    fx.record(
        "2.1.286",
        CONTRACT,
        "SessionStart",
        EventClass::Lifecycle,
        EvidenceOutcome::Ok,
    );
    fx.store
        .record_harness_evidence(
            &EvidenceRecord {
                harness: "codex",
                version: "0.5.0",
                contract_id: OTHER_CONTRACT,
                event: "SessionStart",
                class: EventClass::Lifecycle,
                outcome: &EvidenceOutcome::Ok,
            },
            &budget(),
        )
        .unwrap();
    let rows = fx
        .store
        .harness_evidence_since(T0 as u64, &budget())
        .unwrap();
    let harnesses: Vec<_> = rows.iter().map(|r| r.harness.as_str()).collect();
    assert_eq!(harnesses, ["claude", "codex"]);
    assert_eq!(
        fx.store
            .harness_evidence_all("codex", &budget())
            .unwrap()
            .len(),
        1
    );
    assert!(
        fx.store
            .harness_evidence_since(T0 as u64 + 1, &budget())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn unattributed_reason_persists_and_records_no_row() {
    let fx = Fx::new("he-unattributed");
    assert_eq!(
        fx.store.last_unattributed("claude", &budget()).unwrap(),
        None
    );
    fx.store
        .record_unattributed("claude", "transcript not found", &budget())
        .unwrap();
    fx.advance(50);
    fx.store
        .record_unattributed("claude", "resume before first entry", &budget())
        .unwrap();
    assert_eq!(
        fx.store.last_unattributed("claude", &budget()).unwrap(),
        Some(("resume before first entry".to_owned(), T0 as u64 + 50))
    );
    assert_eq!(
        fx.store.last_unattributed("codex", &budget()).unwrap(),
        None,
        "the record is per harness"
    );
    assert!(
        fx.store
            .harness_evidence_all("claude", &budget())
            .unwrap()
            .is_empty(),
        "an unattributed payload writes no evidence row"
    );
}
