use super::*;
use crate::{
    protocol::time::{Clock, MonoInstant, UtcMillis},
    store::connection::StoreContext,
};
use rusqlite::{Connection, params};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
};

struct TestClock(AtomicI64);
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(self.0.load(Ordering::SeqCst))
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(0)
    }
}

fn setup(now: i64) -> (StoreContext, Connection, Arc<TestClock>, PathBuf) {
    let path = std::env::temp_dir().join(format!("invitation-due-{}.db", uuid::Uuid::new_v4()));
    let clock = Arc::new(TestClock(AtomicI64::new(now)));
    let context = StoreContext::new(path.clone(), clock.clone());
    let conn = context.open_writer().unwrap();
    conn.execute(
        "INSERT INTO host_instances(id, created_at) VALUES ('host', 0)",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO seats(id, instance_id, state, role, generation, created_at) VALUES ('seat', 'host', 'resolved', 'native', 1, 0)", []).unwrap();
    conn.execute("INSERT INTO threads(id, instance_id, topic, goal, created_at, updated_at) VALUES ('thread', 'host', 'topic', 'purpose', 0, 0)", []).unwrap();
    (context, conn, clock, path)
}

fn invitation(conn: &Connection, id: &str, episode: i64, deadline: i64, state: &str) {
    let decision_seq: i64 = conn.query_row(
        "UPDATE host_instances SET decision_seq=decision_seq+1 WHERE id='host' RETURNING decision_seq",
        [], |row| row.get(0),
    ).unwrap();
    conn.execute("INSERT INTO invitations(id, thread_id, seat_id, episode, state, created_decision_seq, created_at, frozen_duration_ms, deadline_at, accepted_at, accepted_actor_seat_id, accepted_generation, accepted_observation) VALUES (?1, 'thread', 'seat', ?2, ?3, ?6, 0, ?4, ?5, CASE WHEN ?3='accepted' THEN 0 END, CASE WHEN ?3='accepted' THEN 'seat' END, CASE WHEN ?3='accepted' THEN 1 END, CASE WHEN ?3='accepted' THEN 'test-observation' END)",
        params![id, episode, state, deadline, deadline, decision_seq]).unwrap();
}

fn warnings(conn: &Connection) -> i64 {
    conn.query_row("SELECT count(*) FROM messages WHERE kind='warn'", [], |r| {
        r.get(0)
    })
    .unwrap()
}

#[test]
fn pending_warns_once_at_equality_across_reopened_scans() {
    let (context, mut conn, clock, path) = setup(99);
    invitation(&conn, "pending", 1, 100, "pending");
    invitation(&conn, "accepted", 2, 100, "accepted");
    let before = scan_invitation_due_batch(&context, &mut conn, None, 2).unwrap();
    assert_eq!(before.warnings_added, 0);
    assert_eq!(warnings(&conn), 0);
    clock.0.store(100, Ordering::SeqCst);
    let due = scan_invitation_due_batch(&context, &mut conn, None, 2).unwrap();
    assert_eq!(due.warnings_added, 1);
    assert_eq!(warnings(&conn), 1);
    scan_invitation_due_batch(&context, &mut conn, None, 2).unwrap();
    drop(conn);
    let mut reopened = StoreContext::new(path, clock).open_writer().unwrap();
    scan_invitation_due_batch(&context, &mut reopened, None, 2).unwrap();
    assert_eq!(warnings(&reopened), 1);
    let (marker, warning_id): (Option<String>, String) = reopened
        .query_row(
            "SELECT i.warning_message_id, m.id FROM invitations i JOIN messages m ON m.source_invitation_id=i.id WHERE i.id='pending' AND m.kind='warn'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(marker.as_deref(), Some(warning_id.as_str()));
    let jobs: i64 = reopened
        .query_row(
            "SELECT count(*) FROM warning_jobs WHERE warning_id=?1",
            [warning_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(jobs, 1);
    let direct_wake: i64 = reopened
        .query_row(
            "SELECT count(*) FROM wake_work WHERE seat_id='seat'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(direct_wake, 0);
    let payload: String = reopened
        .query_row(
            "SELECT event_json FROM messages WHERE kind='warn'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(value["deadline_at"], 100);
    let frozen: (i64, i64) = reopened
        .query_row(
            "SELECT deadline_at, frozen_duration_ms FROM invitations WHERE id='pending'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(frozen, (100, 100));
}

#[test]
fn accepted_after_advisory_selection_gets_no_retrospective_warning() {
    let (context, mut conn, clock, _) = setup(90);
    invitation(&conn, "accepted-later", 1, 100, "pending");
    let page = select_invitation_due_candidates(&conn, None, 1).unwrap();
    conn.execute(
        "UPDATE invitations SET state='accepted', accepted_at=90, accepted_actor_seat_id='seat', accepted_generation=1, accepted_observation='test-observation' WHERE id='accepted-later'",
        [],
    )
    .unwrap();
    clock.0.store(110, Ordering::SeqCst);
    assert_eq!(
        apply_invitation_due_candidates(&context, &mut conn, page)
            .unwrap()
            .warnings_added,
        0
    );
    assert_eq!(warnings(&conn), 0);
    let decision_seq: i64 = conn
        .query_row(
            "SELECT decision_seq FROM host_instances WHERE id='host'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(decision_seq, 1);
}

#[test]
fn advisory_candidate_fenced_before_decision_never_gains_late_warning() {
    let (context, mut conn, clock, path) = setup(90);
    invitation(&conn, "future-at-cutover", 1, 100, "pending");
    let page = select_invitation_due_candidates(&conn, None, 1).unwrap();
    conn.execute(
        "UPDATE seats SET state='retired', retired_at=90 WHERE id='seat'",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO retirements(id, seat_id, cutover_at, closure_boot, closure_epoch, closure_target, closure_generation) VALUES ('job', 'seat', 90, 'boot', 1, 'target', 1)", []).unwrap();
    clock.0.store(150, Ordering::SeqCst);
    let result = apply_invitation_due_candidates(&context, &mut conn, page).unwrap();
    assert_eq!(result.inspected, 1);
    assert_eq!(result.warnings_added, 0);
    assert_eq!(warnings(&conn), 0);
    drop(conn);
    let mut reopened = StoreContext::new(path, clock).open_writer().unwrap();
    assert_eq!(
        scan_invitation_due_batch(&context, &mut reopened, None, 1)
            .unwrap()
            .warnings_added,
        0
    );
    assert_eq!(warnings(&reopened), 0);
}

#[test]
fn earlier_warning_survives_retirement_and_repeated_scans() {
    let (context, mut conn, clock, _) = setup(100);
    invitation(&conn, "already-overdue", 1, 100, "pending");
    assert_eq!(
        scan_invitation_due_batch(&context, &mut conn, None, 1)
            .unwrap()
            .warnings_added,
        1
    );
    conn.execute(
        "UPDATE seats SET state='retired', retired_at=101 WHERE id='seat'",
        [],
    )
    .unwrap();
    clock.0.store(200, Ordering::SeqCst);
    for _ in 0..2 {
        assert_eq!(
            scan_invitation_due_batch(&context, &mut conn, None, 1)
                .unwrap()
                .warnings_added,
            0
        );
    }
    assert_eq!(warnings(&conn), 1);
    let jobs: i64 = conn
        .query_row("SELECT count(*) FROM warning_jobs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(jobs, 1);
}

#[test]
fn warnings_in_one_scan_share_the_transaction_decision_sequence() {
    let (context, mut conn, _, _) = setup(100);
    invitation(&conn, "first", 1, 100, "pending");
    invitation(&conn, "second", 2, 100, "pending");
    let batch = scan_invitation_due_batch(&context, &mut conn, None, 2).unwrap();
    assert_eq!((batch.inspected, batch.warnings_added), (2, 2));
    let (count, distinct): (i64, i64) = conn
        .query_row(
            "SELECT count(*), count(DISTINCT event_seq) FROM warning_jobs",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((count, distinct), (2, 1));
}

#[test]
fn one_scan_allocates_decision_sequences_per_host_instance() {
    let (context, mut conn, _, _) = setup(100);
    invitation(&conn, "first", 1, 100, "pending");
    conn.execute_batch("INSERT INTO host_instances(id, created_at, decision_seq) VALUES ('other-host', 0, 1); INSERT INTO seats(id, instance_id, state, role, generation, created_at) VALUES ('other-seat', 'other-host', 'resolved', 'native', 1, 0); INSERT INTO threads(id, instance_id, topic, goal, created_at, updated_at) VALUES ('other-thread', 'other-host', 'topic', 'purpose', 0, 0); INSERT INTO invitations(id, thread_id, seat_id, episode, state, created_decision_seq, created_at, frozen_duration_ms, deadline_at) VALUES ('second', 'other-thread', 'other-seat', 1, 'pending', 1, 0, 100, 100);").unwrap();
    let batch = scan_invitation_due_batch(&context, &mut conn, None, 2).unwrap();
    assert_eq!((batch.inspected, batch.warnings_added), (2, 2));
    let sequences: Vec<(String, i64)> = conn
        .prepare("SELECT id, decision_seq FROM host_instances ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        sequences,
        vec![("host".into(), 2), ("other-host".into(), 2)]
    );
}

#[test]
fn bounded_continuation_reaches_later_due_rows() {
    let (context, mut conn, _, _) = setup(300);
    invitation(&conn, "first", 1, 100, "pending");
    invitation(&conn, "second", 2, 200, "pending");
    invitation(&conn, "third", 3, 300, "pending");
    let first = scan_invitation_due_batch(&context, &mut conn, None, 1).unwrap();
    assert_eq!((first.inspected, first.warnings_added), (1, 1));
    let second = scan_invitation_due_batch(&context, &mut conn, first.next, 1).unwrap();
    assert_eq!((second.inspected, second.warnings_added), (1, 1));
    let third = scan_invitation_due_batch(&context, &mut conn, second.next, 1).unwrap();
    assert_eq!((third.inspected, third.warnings_added), (1, 1));
    let final_cursor = third.next.expect("full physical slice must be resumable");
    let complete = scan_invitation_due_batch(&context, &mut conn, Some(final_cursor), 1).unwrap();
    assert_eq!(
        (complete.inspected, complete.warnings_added, complete.next),
        (0, 0, None)
    );
    assert_eq!(warnings(&conn), 3);
    let repeat = scan_invitation_due_batch(&context, &mut conn, None, 1).unwrap();
    assert_eq!((repeat.inspected, repeat.warnings_added), (0, 0));
    assert!(repeat.next.is_none());
}

#[test]
fn already_warned_pending_rows_are_absent_from_physical_due_slices() {
    let (context, mut conn, _, _) = setup(100);
    invitation(&conn, "warned", 1, 100, "pending");
    let warned = scan_invitation_due_batch(&context, &mut conn, None, 1).unwrap();
    assert_eq!((warned.inspected, warned.warnings_added), (1, 1));
    invitation(&conn, "new-due", 2, 100, "pending");
    let next = scan_invitation_due_batch(&context, &mut conn, None, 1).unwrap();
    assert_eq!((next.inspected, next.warnings_added), (1, 1));
    assert!(next.next.is_some());
    assert_eq!(warnings(&conn), 2);
}

#[test]
fn thirty_thousand_settled_rows_do_not_delay_a_new_due_invitation() {
    let (context, mut conn, _, _) = setup(100);
    let tx = conn.transaction().unwrap();
    {
        let mut insert = tx.prepare("INSERT INTO invitations(id, thread_id, seat_id, episode, state, created_decision_seq, created_at, frozen_duration_ms, deadline_at, accepted_at, accepted_actor_seat_id, accepted_generation, accepted_observation) VALUES (?1, 'thread', 'seat', ?2, 'accepted', ?2, 0, 100, 100, 0, 'seat', 1, 'test-observation')").unwrap();
        for n in 1..=30_000 {
            insert.execute(params![format!("settled-{n}"), n]).unwrap();
        }
    }
    tx.execute(
        "UPDATE host_instances SET decision_seq=30000 WHERE id='host'",
        [],
    )
    .unwrap();
    tx.commit().unwrap();
    invitation(&conn, "new-due", 30_001, 100, "pending");
    let batch = scan_invitation_due_batch(&context, &mut conn, None, 1).unwrap();
    assert_eq!((batch.inspected, batch.warnings_added), (1, 1));
    assert!(batch.next.is_some());
    assert_eq!(warnings(&conn), 1);
}

#[test]
fn due_slice_accepts_the_full_hundred_candidate_admission() {
    let (context, mut conn, _, _) = setup(100);
    let tx = conn.transaction().unwrap();
    {
        let mut insert = tx.prepare("INSERT INTO invitations(id, thread_id, seat_id, episode, state, created_decision_seq, created_at, frozen_duration_ms, deadline_at) VALUES (?1, 'thread', 'seat', ?2, 'pending', ?2, 0, 100, 100)").unwrap();
        for n in 1..=101 {
            insert.execute(params![format!("due-{n}"), n]).unwrap();
        }
    }
    tx.execute(
        "UPDATE host_instances SET decision_seq=101 WHERE id='host'",
        [],
    )
    .unwrap();
    tx.commit().unwrap();
    let first = scan_invitation_due_batch(&context, &mut conn, None, 100).unwrap();
    assert_eq!((first.inspected, first.warnings_added), (100, 100));
    let second = scan_invitation_due_batch(&context, &mut conn, first.next, 100).unwrap();
    assert_eq!((second.inspected, second.warnings_added), (1, 1));
}

#[test]
fn interleaved_new_invitations_consume_bounded_slices_and_do_not_starve_old_rows() {
    let (context, mut conn, _, _) = setup(300);
    invitation(&conn, "first", 1, 100, "pending");
    invitation(&conn, "old-second", 2, 300, "pending");
    let first = scan_invitation_due_batch(&context, &mut conn, None, 1).unwrap();
    assert_eq!((first.inspected, first.warnings_added), (1, 1));
    let cursor = first.next;
    assert!(cursor.is_some());

    for i in 0..40 {
        invitation(&conn, &format!("new-{i}"), 3 + i, 200 + i, "pending");
    }

    let old = scan_invitation_due_batch(&context, &mut conn, cursor, 1).unwrap();
    assert_eq!((old.inspected, old.warnings_added), (1, 1));
    assert_eq!(
        scan_invitation_due_batch(&context, &mut conn, old.next, 1)
            .unwrap()
            .inspected,
        0
    );
    assert_eq!(warnings(&conn), 2);
}

#[test]
fn failed_warning_job_write_rolls_back_marker_event_and_sequence() {
    let (context, mut conn, _, _) = setup(100);
    invitation(&conn, "pending", 1, 100, "pending");
    conn.execute_batch("CREATE TRIGGER reject_warning_job BEFORE INSERT ON warning_jobs BEGIN SELECT RAISE(ABORT, 'job failure'); END;").unwrap();
    assert!(scan_invitation_due_batch(&context, &mut conn, None, 1).is_err());
    assert_eq!(warnings(&conn), 0);
    let marker: Option<String> = conn
        .query_row(
            "SELECT warning_message_id FROM invitations WHERE id='pending'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(marker, None);
    let jobs: i64 = conn
        .query_row("SELECT count(*) FROM warning_jobs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(jobs, 0);
    let sequence: i64 = conn
        .query_row(
            "SELECT next_sequence FROM threads WHERE id='thread'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(sequence, 1);
    conn.execute_batch("DROP TRIGGER reject_warning_job")
        .unwrap();
    assert_eq!(
        scan_invitation_due_batch(&context, &mut conn, None, 1)
            .unwrap()
            .warnings_added,
        1
    );
    assert_eq!(warnings(&conn), 1);
}

#[test]
fn failed_page_retries_from_its_original_physical_cursor() {
    let (context, mut conn, _, _) = setup(100);
    invitation(&conn, "first", 1, 100, "pending");
    invitation(&conn, "second", 2, 100, "pending");
    let first = scan_invitation_due_batch(&context, &mut conn, None, 1).unwrap();
    assert_eq!((first.inspected, first.warnings_added), (1, 1));
    let retry_position = first.next.expect("second due row remains");
    conn.execute_batch("CREATE TRIGGER reject_second_job BEFORE INSERT ON warning_jobs BEGIN SELECT RAISE(ABORT, 'job failure'); END;").unwrap();
    assert!(scan_invitation_due_batch(&context, &mut conn, Some(retry_position), 1).is_err());
    assert_eq!(warnings(&conn), 1);
    conn.execute_batch("DROP TRIGGER reject_second_job")
        .unwrap();
    let retried = scan_invitation_due_batch(&context, &mut conn, Some(retry_position), 1).unwrap();
    assert_eq!((retried.inspected, retried.warnings_added), (1, 1));
    assert_eq!(
        scan_invitation_due_batch(&context, &mut conn, retried.next, 1)
            .unwrap()
            .inspected,
        0
    );
    assert_eq!(warnings(&conn), 2);
}

// Fails if lazy source rows enter either due classification or native/mod attention.
#[test]
fn lazy_invitation_is_discoverable_but_never_due_or_waking() {
    let (context, mut conn, _, _) = setup(1_000);
    conn.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('origin','host','resolved','native',1,0)", []).unwrap();
    conn.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at,confirmation,inviter_seat_id) VALUES ('lazy','thread','seat',1,'pending',1,0,100,100,'lazy','origin')", []).unwrap();
    let batch = scan_invitation_due_batch(&context, &mut conn, None, 100).unwrap();
    assert_eq!((batch.inspected, batch.warnings_added), (0, 0));
    let tx = conn.transaction().unwrap();
    let result = crate::store::schema::record_overdue_if_pending(
        &tx,
        &ObligationRef::Invitation(InvitationId::new("lazy")),
        &TimeBasis::Decision,
        UtcMillis(1_000),
    )
    .unwrap();
    assert!(
        !result.inserted,
        "late acceptance/rejection/retirement must not retrospectively warn a lazy invitation"
    );
    tx.commit().unwrap();
    assert_eq!(
        crate::store::seats::mod_seat_view(
            &conn,
            "host",
            &crate::protocol::ids::SeatId::new("seat")
        )
        .unwrap()
        .unwrap()
        .fingerprint
        .other_pending,
        0,
        "lazy invitations must not raise mod attention"
    );
    assert!(
        !crate::store::attention::wake_seat_attention(&conn, "seat")
            .unwrap()
            .attention
            .has_pending_invitation
    );
    assert_eq!(
        crate::store::attention::pending_invitations(&conn, "seat", None, i64::MAX)
            .unwrap()
            .items
            .len(),
        1
    );
}

// Earlier-deadline arrivals must not stretch a finite snapshot's physical scan.
#[test]
fn invitation_snapshot_finishes_with_250_initial_rows_despite_continuing_arrivals() {
    let (context, mut conn, _, _) = setup(1_000);
    for i in 0..250 {
        invitation(
            &conn,
            &format!("initial-{i}"),
            i + 1,
            if i < 100 { 100 } else { 900 },
            "pending",
        );
    }
    let first = scan_invitation_due_batch(&context, &mut conn, None, 100).unwrap();
    assert_eq!((first.inspected, first.warnings_added), (100, 100));
    let mut cursor = first.next;
    for page in 0..2 {
        for i in 0..150 {
            invitation(
                &conn,
                &format!("arrival-{page}-{i}"),
                251 + page * 150 + i,
                200,
                "pending",
            );
        }
        let batch = scan_invitation_due_batch(&context, &mut conn, cursor, 100).unwrap();
        assert!(batch.inspected <= 100);
        cursor = batch.next;
    }
    let initial_warned: i64=conn.query_row("SELECT count(*) FROM invitations WHERE id LIKE 'initial-%' AND warning_message_id IS NOT NULL",[],|r|r.get(0)).unwrap();
    assert_eq!(initial_warned, 250);
    assert!(
        cursor.is_none(),
        "finite high-water snapshot must finish after three bounded pages"
    );
}

#[test]
fn timed_invitation_warns_original_inviter_after_leaving_and_clear_is_passive() {
    let (context, mut conn, _, _) = setup(100);
    conn.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('origin','host','resolved','native',1,0)", []).unwrap();
    conn.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at,confirmation,inviter_seat_id) VALUES ('timed','thread','seat',1,'pending',1,0,100,100,'require_ack','origin')", []).unwrap();
    assert_eq!(
        scan_invitation_due_batch(&context, &mut conn, None, 100)
            .unwrap()
            .warnings_added,
        1
    );
    let warning: String = conn
        .query_row(
            "SELECT warning_message_id FROM invitations WHERE id='timed'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(crate::store::effective::is_warning_recipient(&conn, &warning, "origin").unwrap());
    assert!(crate::store::attention::warning_wakes_seat(&conn, "origin", &warning).unwrap());
    assert!(
        crate::store::attention::wake_seat_attention(&conn, "origin")
            .unwrap()
            .attention
            .latest_warning_seq
            .is_some()
    );
    let tx = conn.transaction().unwrap();
    tx.execute("UPDATE invitations SET state='accepted',accepted_at=150,accepted_actor_seat_id='seat',accepted_generation=1,accepted_observation='test-observation' WHERE id='timed'",[]).unwrap();
    crate::store::schema::clear_warning_condition_for_invitation(&tx, "timed", UtcMillis(150))
        .unwrap();
    tx.commit().unwrap();
    let clear: String = conn
        .query_row(
            "SELECT clear_warning_id FROM warning_conditions WHERE condition_id='timed'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!crate::store::attention::warning_wakes_seat(&conn, "origin", &clear).unwrap());
    let clear_recipient:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM digest_programmatic_warnings WHERE warning_id=?1 AND seat_id='origin')",[&clear],|r|r.get(0)).unwrap();
    assert!(
        clear_recipient,
        "the left inviter must retain its passive clear notice"
    );
    assert!(!crate::store::attention::warning_wakes_seat(&conn, "origin", &warning).unwrap());
}

#[test]
fn invitation_due_index_bounds_physical_work_across_lazy_and_future_backlogs() {
    let (context, mut conn, _, _) = setup(100);
    conn.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('origin','host','resolved','native',1,0)", []).unwrap();
    conn.execute("WITH RECURSIVE n(v) AS (SELECT 1 UNION ALL SELECT v+1 FROM n WHERE v<2000) INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at,confirmation,inviter_seat_id) SELECT 'lazy-'||v,'thread','seat',v,'pending',1,0,1,1,'lazy','origin' FROM n",[]).unwrap();
    for i in 0..250 {
        invitation(&conn, &format!("future-{i}"), 2001 + i, 1_000, "pending");
    }
    invitation(&conn, "last-due", 2251, 100, "pending");
    let mut cursor = None;
    let mut inspected = 0;
    for _ in 0..3 {
        let batch = scan_invitation_due_batch(&context, &mut conn, cursor, 100).unwrap();
        inspected += batch.inspected;
        cursor = batch.next;
    }
    assert_eq!(
        inspected, 251,
        "lazy rows must be absent from the physical due access path"
    );
    assert!(cursor.is_none());
    assert_eq!(
        warnings(&conn),
        1,
        "future rows cannot end a traversal before a later ordinal that is due"
    );
    let mut stmt = conn.prepare(INVITATION_DUE_SELECTION).unwrap();
    let _: Vec<String> = stmt
        .query_map(params![0, i64::MAX, 100], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(
        stmt.get_status(rusqlite::StatementStatus::VmStep) < 10_000,
        "bounded index selection must not scan lazy history or sort the backlog"
    );
    assert_eq!(stmt.get_status(rusqlite::StatementStatus::Sort), 0);
}

#[test]
fn invitation_backlog_restart_reconstructs_progress_without_recipient_activity() {
    let (context, mut conn, clock, path) = setup(1_000);
    for i in 0..250 {
        invitation(&conn, &format!("restart-{i}"), i + 1, 500, "pending");
    }
    assert_eq!(
        scan_invitation_due_batch(&context, &mut conn, None, 100)
            .unwrap()
            .warnings_added,
        100
    );
    drop(conn);
    let restarted = StoreContext::new(path, clock);
    let mut reopened = restarted.open_writer().unwrap();
    for i in 0..250 {
        invitation(&reopened, &format!("future-{i}"), 251 + i, 2_000, "pending");
    }
    let mut cursor = None;
    for _ in 0..4 {
        let batch = scan_invitation_due_batch(&restarted, &mut reopened, cursor, 100).unwrap();
        assert!(batch.inspected <= 100);
        cursor = batch.next;
    }
    assert_eq!(warnings(&reopened), 250);
    assert_eq!(reopened.query_row("SELECT count(*) FROM invitations WHERE id LIKE 'restart-%' AND warning_message_id IS NOT NULL",[],|r|r.get::<_,i64>(0)).unwrap(),250);
    assert_eq!(
        reopened
            .query_row("SELECT count(*) FROM occupant_bindings", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0,
        "discovery never requires registration"
    );
}

#[test]
fn invitation_confirmation_prior_schema_upgrade_preserves_exact_open_origins() {
    let path =
        std::env::temp_dir().join(format!("invitation-origin-v33-{}.db", uuid::Uuid::new_v4()));
    let conn = Connection::open(&path).unwrap();
    conn.pragma_update(None, "foreign_keys", "ON").unwrap();
    let mut migrations: Vec<_> =
        std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/migrations"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name().unwrap().to_str().unwrap()[..4]
                    .parse::<u32>()
                    .is_ok_and(|version| version <= 33)
            })
            .collect();
    migrations.sort();
    assert_eq!(migrations.len(), 33);
    for migration in migrations {
        conn.execute_batch(&std::fs::read_to_string(migration).unwrap())
            .unwrap();
    }
    conn.pragma_update(None, "user_version", 33).unwrap();
    drop(conn);
    let conn = Connection::open(&path).unwrap();
    conn.pragma_update(None, "foreign_keys", "ON").unwrap();
    assert_eq!(
        conn.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        33
    );
    conn.execute(
        "INSERT INTO host_instances(id,created_at,decision_seq) VALUES ('host',0,10)",
        [],
    )
    .unwrap();
    for seat in ["origin", "legacy", "unknown", "seat"] {
        conn.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES (?1,'host','resolved','native',1,0)", [seat]).unwrap();
    }
    conn.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('thread','host','topic','goal',0,0)", []).unwrap();
    for (ordinal, id, recorded, actor) in [
        (1, "stored", Some("origin"), None),
        (2, "historical", None, Some("legacy")),
        (3, "unattributed", None, None),
    ] {
        conn.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at,inviter_seat_id) VALUES (?1,'thread','seat',?2,'pending',1,0,100,100,?3)", params![id,ordinal,recorded]).unwrap();
        if let Some(actor) = actor {
            crate::store::schema::append_event_once(&conn, crate::store::schema::EventInput {
                thread: &crate::protocol::ids::ThreadId::new("thread"), key: &format!("invite:{id}"), kind: "info",
                payload_json: &serde_json::json!({"action":"invite","actor_seat":actor,"seat":"seat","invitation":id}).to_string(), decision_at: UtcMillis(0), source_message: None, source_invitation: Some(&InvitationId::new(id)),
            }).unwrap();
        }
        let (warning, _) = crate::store::schema::append_event_once(
            &conn,
            crate::store::schema::EventInput {
                thread: &crate::protocol::ids::ThreadId::new("thread"),
                key: &format!("overdue:{id}"),
                kind: "warn",
                payload_json: "{}",
                decision_at: UtcMillis(100),
                source_message: None,
                source_invitation: Some(&InvitationId::new(id)),
            },
        )
        .unwrap();
        let seq: i64 = conn
            .query_row(
                "SELECT decision_seq FROM messages WHERE id=?1",
                [warning.as_str()],
                |r| r.get(0),
            )
            .unwrap();
        conn.execute("INSERT INTO warning_conditions(condition_kind,thread_id,condition_id,affected_seat_id,open_warning_id,opened_seq) VALUES ('invitation','thread',?1,'seat',?2,?3)", params![id, warning.as_str(),seq]).unwrap();
    }
    let before: Vec<(String, String)> = {
        let mut query = conn
            .prepare("SELECT id,event_json FROM messages ORDER BY ordinal")
            .unwrap();
        query
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    drop(conn);
    let context = StoreContext::new(path.clone(), Arc::new(TestClock(AtomicI64::new(200))));
    let conn = context.open_writer().unwrap();
    assert_eq!(
        conn.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        34
    );
    for origin in ["origin", "legacy"] {
        assert!(
            crate::store::attention::wake_seat_attention(&conn, origin)
                .unwrap()
                .attention
                .latest_warning_seq
                .is_some(),
            "prior-schema open origin {origin} must survive upgrade"
        );
    }
    assert!(
        crate::store::attention::wake_seat_attention(&conn, "unknown")
            .unwrap()
            .attention
            .latest_warning_seq
            .is_none(),
        "unknown historical evidence cannot attribute a member"
    );
    let after: Vec<(String, String)> = {
        let mut query = conn
            .prepare("SELECT id,event_json FROM messages ORDER BY ordinal")
            .unwrap();
        query
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(before, after, "upgrade never rewrites historical events");
    let mut conn = conn;
    let tx = conn.transaction().unwrap();
    tx.execute("UPDATE invitations SET state='accepted',accepted_at=200,accepted_actor_seat_id='seat',accepted_generation=1,accepted_observation='test-observation' WHERE id='stored'", []).unwrap();
    crate::store::schema::clear_warning_condition_for_invitation(&tx, "stored", UtcMillis(200))
        .unwrap();
    tx.commit().unwrap();
    conn.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at,inviter_seat_id) VALUES ('next-episode','thread','seat',4,'pending',10,100,100,200,'legacy')", []).unwrap();
    assert!(
        scan_invitation_due_batch(&context, &mut conn, None, 100)
            .unwrap()
            .warnings_added
            > 0
    );
    let next_origin: String = conn.query_row("SELECT invitation_originator_seat_id FROM warning_conditions WHERE condition_id='next-episode' AND clear_warning_id IS NULL", [], |r|r.get(0)).unwrap();
    assert_eq!(
        next_origin, "legacy",
        "new open episode resolves its own origin rather than retaining the prior origin"
    );
    drop(conn);
    drop(context);
    let _ = std::fs::remove_file(path);
}

// Exercise both transaction boundaries in the composed chain using real prior32.
#[test]
fn invitation_confirmation_actual32_upgrade_rolls_back_reopens_and_preserves_receipt_progress() {
    for (obstruction, failed_version) in [
        ("invitations_confirmation_pending_seat", 32),
        ("warning_conditions_invitation_origin", 33),
    ] {
        let path =
            std::env::temp_dir().join(format!("invitation-chain-{}.db", uuid::Uuid::new_v4()));
        let db = Connection::open(&path).unwrap();
        let mut paths = std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/migrations"))
            .unwrap()
            .map(|p| p.unwrap().path())
            .collect::<Vec<_>>();
        paths.sort();
        for (index, migration) in paths.iter().take(32).enumerate() {
            db.execute_batch(&std::fs::read_to_string(migration).unwrap())
                .unwrap();
            db.pragma_update(None, "user_version", index as i64 + 1)
                .unwrap();
            if index == 28 {
                db.execute_batch(
                    "INSERT INTO host_instances(id,created_at,decision_seq) VALUES('host',0,12)",
                )
                .unwrap();
            }
        }
        db.execute_batch("UPDATE host_instances SET decision_seq=99; UPDATE receipt_deadline_backfill SET physical_after=2,physical_high_water=7,prepared_after=3,prepared_high_water=9;
            INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES('seat','host','resolved','native',1,0);
            INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES('thread','host','topic','goal',0,0);
            INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at,confirmation,inviter_seat_id) VALUES('retained','thread','seat',1,'pending',1,0,100,100,'require_ack','seat');").unwrap();
        db.execute_batch(&format!("CREATE TABLE {obstruction}(value TEXT)"))
            .unwrap();
        drop(db);
        let db = Connection::open(&path).unwrap();
        db.pragma_update(None, "foreign_keys", true).unwrap();
        assert_eq!(
            db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            32
        );
        assert!(crate::store::schema::initialize(&db, || UtcMillis(200)).is_err());
        assert!(db.is_autocommit());
        assert_eq!(
            db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            failed_version
        );
        assert!(
            db.pragma_query_value(None, "foreign_keys", |r| r.get::<_, bool>(0))
                .unwrap()
        );
        assert_eq!(db.query_row("SELECT count(*) FROM pragma_table_info('warning_conditions') WHERE name LIKE 'invitation_originator_%'", [], |r| r.get::<_,i64>(0)).unwrap(),0);
        if failed_version == 32 {
            assert_eq!(
                db.query_row(
                    "SELECT count(*) FROM sqlite_master WHERE name='invitations_confirmation_scan'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                0
            );
        }
        drop(db);
        let db = Connection::open(&path).unwrap();
        db.pragma_update(None, "foreign_keys", true).unwrap();
        assert_eq!(
            db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            failed_version
        );
        db.execute_batch(&format!("DROP TABLE {obstruction}"))
            .unwrap();
        for _ in 0..2 {
            crate::store::schema::initialize(&db, || UtcMillis(300)).unwrap();
            assert_eq!(
                db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                    .unwrap(),
                34
            );
            assert_eq!(db.query_row("SELECT physical_after,physical_high_water,prepared_after,prepared_high_water,completed FROM receipt_deadline_backfill", [], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?,r.get::<_,i64>(4)?))).unwrap(),(2,7,3,9,0));
            assert_eq!(db.query_row("SELECT historical_decision_seq FROM receipt_send_time_cutovers WHERE instance_id='host'", [], |r| r.get::<_,i64>(0)).unwrap(),12);
            assert_eq!(db.query_row("SELECT confirmation,inviter_seat_id,deadline_at FROM invitations WHERE id='retained'", [], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?))).unwrap(),("require_ack".into(),"seat".into(),100));
        }
        drop(db);
        let db = Connection::open(&path).unwrap();
        crate::store::schema::initialize(&db, || UtcMillis(400)).unwrap();
        assert_eq!(
            db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            34
        );
        drop(db);
        std::fs::remove_file(path).unwrap();
    }
}
