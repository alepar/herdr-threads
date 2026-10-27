//! Kept continuity claims (ht-dtx0): per-session keying, the count cap and
//! the age bound. Mounted from `src/store/kept_continuity.rs`.
use super::*;
use crate::protocol::authority::Harness;
use crate::protocol::ids::{ExecutionId, HostTargetId, NativeSessionId, OperationId};

fn claim(target: &str, session: &str) -> ContinuityCheckIn {
    ContinuityCheckIn {
        host: crate::test_support::test_host_locator(),
        target: HostTargetId::new(target),
        harness: Harness::Claude,
        native_session: NativeSessionId::new(session),
        source: ContinuityCheckIn::RESUME_SOURCE.into(),
        operation: OperationId::new(format!("op-{target}-{session}")),
        execution: ExecutionId::new("e"),
        liveness: None,
    }
}

fn store() -> (rusqlite::Connection, HostId) {
    let db = crate::store::bindings::tests::db();
    let host: i64 = db
        .query_row("SELECT min(host_id) FROM herdr_hosts", [], |r| r.get(0))
        .unwrap();
    (db, HostId::from_row(host))
}

fn keep_at(
    db: &mut rusqlite::Connection,
    host: HostId,
    command: &ContinuityCheckIn,
    at: i64,
) -> bool {
    let tx = db.transaction().unwrap();
    let kept = keep(&tx, host, command, at).unwrap();
    tx.commit().unwrap();
    kept
}

fn due_at(db: &mut rusqlite::Connection, host: HostId, at: i64) -> Vec<KeptClaim> {
    let tx = db.transaction().unwrap();
    let due = due(&tx, host, at).unwrap();
    tx.commit().unwrap();
    due
}

/// Kills: keying by target alone (a `/clear` successor's claim on the same
/// pane would overwrite the claim that can reattach the seat); a repeat that
/// restarts the age; a drop that removes a newer claim than the one loaded.
#[test]
fn claims_of_two_sessions_on_one_target_both_stay_and_a_repeat_replaces_its_own() {
    let (mut db, host) = store();
    assert!(keep_at(&mut db, host, &claim("w1:p1", "S"), 10));
    assert!(keep_at(&mut db, host, &claim("w1:p1", "S2"), 20));
    let loaded = due_at(&mut db, host, 25);
    let mut repeat = claim("w1:p1", "S");
    repeat.operation = OperationId::new("op-again");
    assert!(keep_at(&mut db, host, &repeat, 30));
    let due = due_at(&mut db, host, 40);
    let sessions: Vec<_> = due
        .iter()
        .map(|kept| {
            (
                kept.command.native_session.as_str().to_owned(),
                kept.received_at,
            )
        })
        .collect();
    assert_eq!(sessions, vec![("S".into(), 10), ("S2".into(), 20)]);
    assert_eq!(due[0].command.operation.as_str(), "op-again");
    // The row loaded before the repeat is not the row now: dropping it keeps the repeat.
    forget(&db, host, &loaded[0].command, Some(&loaded[0])).unwrap();
    assert_eq!(due_at(&mut db, host, 40).len(), 2);
    forget(&db, host, &claim("w1:p1", "S2"), None).unwrap();
    assert_eq!(due_at(&mut db, host, 40).len(), 1);
}

/// Kills: an unbounded table; a cap that refuses a session's own repeat.
#[test]
fn the_cap_refuses_new_claims_but_not_a_repeat() {
    let (mut db, host) = store();
    for n in 0..MAX_KEPT_CLAIMS {
        assert!(keep_at(&mut db, host, &claim(&format!("w1:p{n}"), "S"), 1));
    }
    assert!(!keep_at(&mut db, host, &claim("w9:p1", "S"), 2));
    assert!(keep_at(&mut db, host, &claim("w1:p0", "S"), 3));
    let count: i64 = db
        .query_row("SELECT count(*) FROM kept_continuity_claims", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, MAX_KEPT_CLAIMS);
    assert_eq!(due_at(&mut db, host, 3).len() as i64, KEPT_CLAIMS_PER_PASS);
}

/// Kills: no age bound (a claim nobody can decide is retried forever).
#[test]
fn a_claim_expires_an_hour_after_it_was_received() {
    let (mut db, host) = store();
    assert!(keep_at(&mut db, host, &claim("w1:p1", "S"), 1_000));
    assert_eq!(due_at(&mut db, host, 1_000 + KEPT_CLAIM_TTL_MS).len(), 1);
    assert!(due_at(&mut db, host, 1_001 + KEPT_CLAIM_TTL_MS).is_empty());
    let count: i64 = db
        .query_row("SELECT count(*) FROM kept_continuity_claims", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 0, "the expired row is gone");
}
