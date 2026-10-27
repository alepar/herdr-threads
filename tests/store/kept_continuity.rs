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

fn by(mut command: ContinuityCheckIn, pid: u32) -> ContinuityCheckIn {
    command.liveness = Some(crate::protocol::authority::LivenessFingerprint { pid, start: 1 });
    command
}

/// Seat `s` gets a binding naming `session`: a session some seat already had.
fn bind(db: &rusqlite::Connection, session: &str) {
    use crate::store::bindings::*;
    let (key, place) = (
        crate::store::bindings::tests::key(session, None),
        crate::store::bindings::tests::placement(),
    );
    let request = EnrollRequest {
        key: &key,
        role: BindingRole::TopLevel,
        parent: None,
        lineage: None,
        fingerprint: Some(crate::protocol::authority::LivenessFingerprint { pid: 99, start: 9 }),
        placement: &place,
        trigger: EnrollTrigger::Lifecycle,
    };
    enroll_with_hooks(db, &request, crate::protocol::time::UtcMillis(5), &[]).unwrap();
}

fn sessions(db: &mut rusqlite::Connection, host: HostId) -> Vec<String> {
    due_at(db, host, 100)
        .into_iter()
        .map(|kept| kept.command.native_session.as_str().to_owned())
        .collect()
}

/// Kills: a process that `/resume`d into another seat's session T keeping its
/// older claim for S, so S's row reattaches S's seat to the pane now running T
/// (T's next check-in then acts as that seat); and supersession by a fresh
/// `/clear` successor, which would strand the pane's seat.
#[test]
fn a_known_session_of_the_same_process_supersedes_its_older_claim() {
    let (mut db, host) = store();
    bind(&db, "T");
    // Fresh successor: no binding names S2, both stay.
    assert!(keep_at(&mut db, host, &by(claim("w1:p1", "S"), 7), 10));
    assert!(keep_at(&mut db, host, &by(claim("w1:p1", "S2"), 7), 20));
    assert_eq!(sessions(&mut db, host), ["S", "S2"]);
    // Another process's claim, and the same process on another target, stay.
    assert!(keep_at(&mut db, host, &by(claim("w1:p1", "O"), 8), 25));
    assert!(keep_at(&mut db, host, &by(claim("w1:p2", "S"), 7), 26));
    // T is another seat's session: the process's other claims on w1:p1 go.
    assert!(keep_at(&mut db, host, &by(claim("w1:p1", "T"), 7), 30));
    assert_eq!(sessions(&mut db, host), ["O", "S", "T"]);
    assert_eq!(
        due_at(&mut db, host, 100)
            .iter()
            .map(|kept| kept.command.target.as_str().to_owned())
            .collect::<Vec<_>>(),
        ["w1:p1", "w1:p2", "w1:p1"]
    );
    // A claim the daemon decided at intake supersedes too.
    assert!(keep_at(&mut db, host, &by(claim("w1:p3", "S"), 7), 40));
    forget(&db, host, &by(claim("w1:p3", "T"), 7), None).unwrap();
    assert_eq!(sessions(&mut db, host), ["O", "S", "T"]);
}
