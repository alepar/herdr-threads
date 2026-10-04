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

const DAY_MS: i64 = 24 * HOUR_MS;

impl Fx {
    /// A lifecycle-ok record for claude version `vN` under `CONTRACT`.
    fn seen(&self, n: usize) -> Recorded {
        self.record(
            &format!("v{n:04}"),
            CONTRACT,
            "SessionStart",
            EventClass::Lifecycle,
            EvidenceOutcome::Ok,
        )
    }

    fn all_claude(&self) -> Vec<EvidenceRow> {
        self.store
            .harness_evidence_all("claude", &budget())
            .unwrap()
    }
}

/// Kills: a table that grows forever, and a prune that drops the newest rows.
#[test]
fn rows_unseen_for_thirty_days_are_pruned_beyond_the_newest_64() {
    let fx = Fx::new("he-prune");
    for n in 0..70 {
        fx.seen(n);
        fx.advance(1);
    }
    fx.advance(31 * DAY_MS);
    fx.seen(70);
    let rows = fx.all_claude();
    assert_eq!(rows.len(), 64);
    let versions: Vec<_> = rows.iter().map(|r| r.version.clone()).collect();
    let expected: Vec<_> = (7..71).map(|n| format!("v{n:04}")).collect();
    assert_eq!(versions, expected, "the newest 64 by last_seen_at survive");
}

/// Kills: pruning by count alone (rows within retention removed).
#[test]
fn recent_rows_are_never_pruned() {
    let fx = Fx::new("he-no-prune");
    for n in 0..70 {
        fx.seen(n);
        fx.advance(1);
    }
    fx.advance(DAY_MS);
    fx.seen(70);
    assert_eq!(fx.all_claude().len(), 71);
}

/// Kills: pruning on every record instead of only when a row is created.
#[test]
fn pruning_happens_only_on_a_new_row() {
    let fx = Fx::new("he-prune-new-only");
    for n in 0..70 {
        fx.seen(n);
        fx.advance(1);
    }
    fx.advance(31 * DAY_MS);
    // Touch an existing, now very old, row: no row is created.
    let touched = fx.seen(69);
    assert!(!touched.created);
    assert_eq!(
        fx.all_claude().len(),
        70,
        "nothing pruned without a new row"
    );
}

/// Kills: Health reading every row of a harness.
#[test]
fn the_harness_read_is_bounded_to_the_newest_rows() {
    let fx = Fx::new("he-read-cap");
    for n in 0..300 {
        fx.seen(n);
        fx.advance(1);
    }
    let rows = fx.all_claude();
    assert_eq!(rows.len(), EVIDENCE_READ_CAP as usize);
    let versions: Vec<_> = rows.iter().map(|r| r.version.clone()).collect();
    let expected: Vec<_> = (44..300).map(|n| format!("v{n:04}")).collect();
    assert_eq!(
        versions, expected,
        "the newest 256, ordered by first_seen_at"
    );
}

#[test]
fn v2_evidence_exact_domains_transaction_milestones_retention_and_legacy_rows() {
    use crate::harness::{adapter::HarnessAdapter, runtime::RuntimeIdentity};
    let fx = Fx::new("he-v2-exact");
    fx.record(
        "2.1.286",
        CONTRACT,
        "SessionStart",
        EventClass::Lifecycle,
        EvidenceOutcome::Ok,
    );
    let legacy = fx.row("2.1.286", CONTRACT).unwrap();
    let descriptor = crate::harness::claude::ClaudeAdapter.contracts()[0];
    let identity = RuntimeIdentity::stable_release("2.1.286", "native_transcript").unwrap();
    let mut writer = fx.store.writer(&budget()).unwrap();
    let record = |event| EvidenceRecordV2 {
        identity: &identity,
        descriptor: &descriptor,
        event,
        outcome: &EvidenceOutcome::Ok,
        qualified: true,
    };
    let first = record_v2(&fx.store.context, &mut writer, &record("SessionStart")).unwrap();
    assert!(first.created);
    assert!(!first.row.verified(&descriptor));
    assert_eq!(first.row.milestones.get("lifecycle"), Some(&(T0 as u64)));
    fx.advance(1000);
    let second = record_v2(&fx.store.context, &mut writer, &record("PreToolUse")).unwrap();
    assert!(second.row.verified(&descriptor));
    assert_eq!(
        second.row.milestones.get("tool"),
        Some(&((T0 + 1000) as u64))
    );
    drop(writer);
    assert_eq!(
        fx.store
            .harness_evidence_v2(
                "claude",
                &identity.key,
                descriptor.domain_id,
                descriptor.origin,
                &descriptor.contract_id_v2().unwrap(),
                &budget()
            )
            .unwrap(),
        Some(second.row)
    );
    assert_eq!(fx.row("2.1.286", CONTRACT), Some(legacy));
}

fn v2_identity(version: &str) -> crate::harness::runtime::RuntimeIdentity {
    crate::harness::runtime::RuntimeIdentity::stable_release(version, "native_transcript").unwrap()
}
fn v2_descriptor() -> crate::harness::adapter::ContractDescriptor {
    use crate::harness::adapter::HarnessAdapter;
    crate::harness::claude::ClaudeAdapter.contracts()[0]
}
fn v2_write(
    fx: &Fx,
    identity: &crate::harness::runtime::RuntimeIdentity,
    descriptor: &crate::harness::adapter::ContractDescriptor,
    event: &str,
    outcome: EvidenceOutcome,
    qualified: bool,
) -> Result<RecordedV2, crate::protocol::results::ApiError> {
    fx.store.record_harness_evidence_v2(
        &EvidenceRecordV2 {
            identity,
            descriptor,
            event,
            outcome: &outcome,
            qualified,
        },
        &budget(),
    )
}
#[test]
fn v2_domains_origins_contracts_runtime_and_qualification_do_not_share_success() {
    use crate::harness::evidence::EvidenceOrigin;
    let fx = Fx::new("he-v2-isolation");
    let identity = v2_identity("2.1.286");
    let descriptor = v2_descriptor();
    v2_write(
        &fx,
        &identity,
        &descriptor,
        "SessionStart",
        EvidenceOutcome::Ok,
        true,
    )
    .unwrap();
    let mut other = descriptor;
    for dimension in 0..3 {
        match dimension {
            0 => other.domain_id = "bridge",
            1 => {
                other = descriptor;
                other.origin = EvidenceOrigin::BridgeEnvelope;
            }
            _ => {
                other = descriptor;
                other.required_milestones = &["tool"];
            }
        }
        let row = v2_write(
            &fx,
            &identity,
            &other,
            "PreToolUse",
            EvidenceOutcome::Ok,
            true,
        )
        .unwrap()
        .row;
        assert!(!row.milestones.contains_key("lifecycle"));
        assert!(!row.verified(&descriptor));
    }
    let different = v2_identity("2.1.287");
    assert!(
        !v2_write(
            &fx,
            &different,
            &descriptor,
            "PreToolUse",
            EvidenceOutcome::Ok,
            true
        )
        .unwrap()
        .row
        .verified(&descriptor)
    );
    let unqualified = v2_write(
        &fx,
        &identity,
        &descriptor,
        "PreToolUse",
        EvidenceOutcome::Ok,
        false,
    )
    .unwrap();
    assert!(!unqualified.row.verified(&descriptor));
    let empty = crate::harness::adapter::ContractDescriptor {
        required_milestones: &[],
        ..descriptor
    };
    assert!(
        !v2_write(
            &fx,
            &identity,
            &empty,
            "PreToolUse",
            EvidenceOutcome::Ok,
            true
        )
        .unwrap()
        .row
        .verified(&empty)
    );
}
#[test]
fn v2_first_success_and_first_violation_are_sticky_malformed_is_not_violation() {
    let fx = Fx::new("he-v2-sticky");
    let identity = v2_identity("2.1.286");
    let descriptor = v2_descriptor();
    let malformed = v2_write(
        &fx,
        &identity,
        &descriptor,
        "PreToolUse",
        EvidenceOutcome::Malformed,
        true,
    )
    .unwrap();
    assert!(malformed.row.milestones.is_empty());
    assert_eq!(malformed.row.violation_at, None);
    let first = v2_write(
        &fx,
        &identity,
        &descriptor,
        "SessionStart",
        EvidenceOutcome::Ok,
        true,
    )
    .unwrap();
    fx.advance(100);
    let violation = v2_write(
        &fx,
        &identity,
        &descriptor,
        "PreToolUse",
        EvidenceOutcome::Violation {
            field: "tool_name".into(),
        },
        true,
    )
    .unwrap();
    assert!(violation.fresh_violation);
    fx.advance(100);
    let second = v2_write(
        &fx,
        &identity,
        &descriptor,
        "SessionStart",
        EvidenceOutcome::Ok,
        true,
    )
    .unwrap();
    let final_row = v2_write(
        &fx,
        &identity,
        &descriptor,
        "PreToolUse",
        EvidenceOutcome::Ok,
        true,
    )
    .unwrap()
    .row;
    assert_eq!(
        second.row.milestones.get("lifecycle"),
        first.row.milestones.get("lifecycle")
    );
    assert_eq!(final_row.violation_at, Some((T0 + 100) as u64));
    assert_eq!(final_row.violation_field.as_deref(), Some("tool_name"));
    assert!(!final_row.verified(&descriptor));
    let again = v2_write(
        &fx,
        &identity,
        &descriptor,
        "SessionStart",
        EvidenceOutcome::Violation {
            field: "different".into(),
        },
        true,
    )
    .unwrap();
    assert!(!again.fresh_violation);
    assert_eq!(again.row.violation_event.as_deref(), Some("PreToolUse"));
}
#[test]
fn v2_conflicting_descriptor_and_failed_write_rollback_every_side_effect() {
    let fx = Fx::new("he-v2-rollback");
    let identity = v2_identity("2.1.286");
    let descriptor = v2_descriptor();
    v2_write(
        &fx,
        &identity,
        &descriptor,
        "SessionStart",
        EvidenceOutcome::Ok,
        true,
    )
    .unwrap();
    let conflict = v2_identity("2.1.286");
    let mut conflict = conflict;
    conflict.descriptor.source = "different".into();
    assert!(conflict.validate().is_ok());
    fx.advance(100);
    assert!(
        v2_write(
            &fx,
            &conflict,
            &descriptor,
            "PreToolUse",
            EvidenceOutcome::Ok,
            true
        )
        .is_err()
    );
    let writer = fx.store.writer(&budget()).unwrap();
    let at: i64 = writer
        .query_row(
            "SELECT last_seen_at FROM harness_runtime_identities",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(at, T0);
    writer.execute_batch("CREATE TRIGGER fail_v2 BEFORE INSERT ON harness_contract_evidence_v2 BEGIN SELECT RAISE(ABORT,'injected write failure'); END;").unwrap();
    drop(writer);
    let new_identity = v2_identity("2.1.287");
    assert!(
        v2_write(
            &fx,
            &new_identity,
            &descriptor,
            "SessionStart",
            EvidenceOutcome::Ok,
            true
        )
        .is_err()
    );
    let writer = fx.store.writer(&budget()).unwrap();
    let count: i64 = writer
        .query_row(
            "SELECT count(*) FROM harness_runtime_identities WHERE identity_key='release:2.1.287'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}
#[test]
fn v2_reads_have_deterministic_ties_cap_and_retention_prunes_only_unreferenced_descriptors() {
    let fx = Fx::new("he-v2-retention");
    let descriptor = v2_descriptor();
    for n in 0..270 {
        v2_write(
            &fx,
            &v2_identity(&format!("2.1.{n}")),
            &descriptor,
            "SessionStart",
            EvidenceOutcome::Ok,
            true,
        )
        .unwrap();
    }
    use crate::harness::{adapter::HarnessAdapter, evidence::EvidenceOrigin};
    let codex = crate::harness::codex::CodexAdapter.contracts()[0];
    v2_write(
        &fx,
        &v2_identity("2.1.0"),
        &codex,
        "SessionStart",
        EvidenceOutcome::Ok,
        true,
    )
    .unwrap();
    let bridge = crate::harness::adapter::ContractDescriptor {
        domain_id: "bridge",
        origin: EvidenceOrigin::BridgeEnvelope,
        ..descriptor
    };
    v2_write(
        &fx,
        &v2_identity("2.1.0"),
        &bridge,
        "SessionStart",
        EvidenceOutcome::Ok,
        true,
    )
    .unwrap();
    let rows = fx
        .store
        .harness_evidence_v2_all("claude", 0, &budget())
        .unwrap();
    assert_eq!(rows.len(), 256);
    assert_eq!(rows[0].identity.key, "release:2.1.0");
    assert_eq!(rows[1].identity.key, "release:2.1.0");
    assert_eq!(rows[0].domain, "bridge");
    assert_eq!(
        fx.store
            .harness_evidence_v2_since(0, &budget())
            .unwrap()
            .len(),
        256
    );
    fx.advance(EVIDENCE_RETENTION_MS + 1);
    v2_write(
        &fx,
        &v2_identity("3.0.0"),
        &descriptor,
        "SessionStart",
        EvidenceOutcome::Ok,
        true,
    )
    .unwrap();
    let db = fx.store.context.open_query(budget()).unwrap();
    let rows = all_v2(&db, "claude", 0).unwrap();
    assert_eq!(rows.len(), 64);
    assert_eq!(rows[0].identity.key, "release:3.0.0");
    let counts:(i64,i64)=db.query_row("SELECT (SELECT count(*) FROM harness_runtime_identities WHERE harness='claude'),(SELECT count(*) FROM harness_contract_evidence_v2 WHERE harness='claude')",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(counts, (63, 64));
    assert_eq!(
        fx.store
            .harness_evidence_v2_all("codex", 0, &budget())
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        all_v2(&db, "claude", (T0 + EVIDENCE_RETENTION_MS + 1) as u64)
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn v2_unattributed_reasons_are_bounded_latest_per_exact_domain_origin() {
    use crate::harness::evidence::EvidenceOrigin::*;
    let fx = Fx::new("he-v2-reasons");
    let mut writer = fx.store.writer(&budget()).unwrap();
    record_unattributed_v2(
        &fx.store.context,
        &mut writer,
        "claude",
        "native",
        NativePayload,
        "first",
    )
    .unwrap();
    fx.advance(50);
    record_unattributed_v2(
        &fx.store.context,
        &mut writer,
        "claude",
        "native",
        NativePayload,
        "last",
    )
    .unwrap();
    record_unattributed_v2(
        &fx.store.context,
        &mut writer,
        "claude",
        "native",
        BridgeEnvelope,
        "bridge",
    )
    .unwrap();
    assert_eq!(
        last_unattributed_v2(&writer, "claude", "native", NativePayload).unwrap(),
        Some(("last".into(), (T0 + 50) as u64))
    );
    assert_eq!(
        last_unattributed_v2(&writer, "claude", "native", BridgeEnvelope).unwrap(),
        Some(("bridge".into(), (T0 + 50) as u64))
    );
    for reason in ["", "bad\nreason", &"é".repeat(129)] {
        assert!(
            record_unattributed_v2(
                &fx.store.context,
                &mut writer,
                "claude",
                "native",
                NativePayload,
                reason
            )
            .is_err()
        );
    }
}

#[test]
fn v2_noncanonical_milestone_maps_are_refused_not_silently_reinterpreted() {
    let fx = Fx::new("he-v2-corrupt-map");
    let identity = v2_identity("2.1.286");
    let descriptor = v2_descriptor();
    v2_write(
        &fx,
        &identity,
        &descriptor,
        "SessionStart",
        EvidenceOutcome::Ok,
        true,
    )
    .unwrap();
    let writer = fx.store.writer(&budget()).unwrap();
    writer
        .execute(
            "UPDATE harness_contract_evidence_v2 SET milestones_json=?1",
            [r#"{"lifecycle":1,"lifecycle":2}"#],
        )
        .unwrap();
    drop(writer);
    assert!(
        fx.store
            .harness_evidence_v2_all("claude", 0, &budget())
            .is_err(),
        "duplicate milestone timestamp must not be last-wins"
    );
}
