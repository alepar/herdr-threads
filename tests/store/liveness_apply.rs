//! `apply_liveness` re-validates a `Superseded` verdict against the claim ordering read inside the
//! write transaction (spec §3: "the most recent lifecycle claimer wins").
use super::tests::{db, key, placement};
use super::*;
use crate::protocol::authority::LivenessFingerprint;
use crate::protocol::time::UtcMillis;
use rusqlite::Connection;

const FP: LivenessFingerprint = LivenessFingerprint { pid: 7, start: 9 };

/// Enrolls a connected top-level binding without running the enroll hooks, so two bindings of one
/// process can both be connected (as they are between a check-in and the next liveness pass).
fn enroll_plain(db: &Connection, session: &str, observed_at: i64) -> BindingRow {
    let (k, mut p) = (key(session, None), placement());
    p.observed_at = UtcMillis(observed_at);
    let request = EnrollRequest {
        key: &k,
        role: BindingRole::TopLevel,
        parent: None,
        lineage: None,
        fingerprint: Some(FP),
        placement: &p,
        trigger: EnrollTrigger::Lifecycle,
    };
    match enroll_with_hooks(db, &request, UtcMillis(observed_at), &[]).unwrap() {
        EnrollOutcome::Enrolled { binding, .. } => binding,
        other => panic!("expected Enrolled, got {other:?}"),
    }
}

fn pair(db: &Connection) -> (BindingRow, BindingRow) {
    (enroll_plain(db, "a", 10), enroll_plain(db, "b", 20))
}

fn superseded(binding: &BindingRow) -> LivenessDecision {
    LivenessDecision {
        binding: binding.binding_id.clone(),
        fingerprint: binding.fingerprint,
        reason: DisconnectReason::Superseded,
    }
}

fn liveness(db: &Connection, binding: &BindingRow) -> Liveness {
    get(db, &binding.binding_id).unwrap().unwrap().liveness
}

/// What the worker's unlocked read would judge: the older claimer loses.
fn judged_loser(db: &Connection, a: &BindingRow) -> LivenessDecision {
    let candidates = liveness_candidates(db, None).unwrap();
    assert_eq!(candidates.len(), 2);
    superseded(a)
}

#[test]
fn superseded_verdict_is_dropped_when_the_loser_reclaims_first() {
    let db = db();
    let (a, b) = pair(&db);
    let verdict = judged_loser(&db, &a);
    // A's lifecycle check-in commits in the gap: A is now the latest claimer and the real hook
    // disconnects B.
    db.execute(
        "UPDATE occupant_bindings SET observed_at=30 WHERE binding_id=?1",
        [a.binding_id.as_str()],
    )
    .unwrap();
    let current = get(&db, &a.binding_id).unwrap().unwrap();
    same_pid_supersede(
        &db,
        &EnrollEvent {
            binding: &current,
            trigger: EnrollTrigger::Lifecycle,
            created: false,
        },
        UtcMillis(31),
    )
    .unwrap();
    assert_eq!(liveness(&db, &b), Liveness::Disconnected);
    assert_eq!(apply_liveness(&db, &[verdict], UtcMillis(32)).unwrap(), 0);
    assert_eq!(liveness(&db, &a), Liveness::Connected);
}

#[test]
fn superseded_verdict_still_applies_when_ordering_is_unchanged() {
    let db = db();
    let (a, b) = pair(&db);
    let verdict = judged_loser(&db, &a);
    assert_eq!(apply_liveness(&db, &[verdict], UtcMillis(32)).unwrap(), 1);
    let loser = get(&db, &a.binding_id).unwrap().unwrap();
    assert_eq!(loser.liveness, Liveness::Disconnected);
    assert_eq!(loser.disconnect_reason.as_deref(), Some("superseded"));
    assert_eq!(liveness(&db, &b), Liveness::Connected);
}

#[test]
fn superseded_verdict_is_dropped_when_the_group_shrank_to_one() {
    let db = db();
    let (a, b) = pair(&db);
    let verdict = judged_loser(&db, &a);
    disconnect(
        &db,
        &b.binding_id,
        DisconnectReason::ProcessExit,
        UtcMillis(25),
    )
    .unwrap();
    assert_eq!(apply_liveness(&db, &[verdict], UtcMillis(32)).unwrap(), 0);
    assert_eq!(liveness(&db, &a), Liveness::Connected);
}

#[test]
fn process_exit_verdicts_are_unchanged() {
    let db = db();
    let (a, b) = pair(&db);
    let verdict = LivenessDecision {
        reason: DisconnectReason::ProcessExit,
        ..superseded(&b)
    };
    assert_eq!(apply_liveness(&db, &[verdict], UtcMillis(32)).unwrap(), 1);
    assert_eq!(liveness(&db, &b), Liveness::Disconnected);
    assert_eq!(liveness(&db, &a), Liveness::Connected);
}
