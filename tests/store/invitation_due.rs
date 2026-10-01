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
    let mut cursor = first.next;
    assert!(cursor.is_some());

    for i in 0..40 {
        invitation(&conn, &format!("new-{i}"), 3 + i, 200 + i, "pending");
    }

    for _ in 0..40 {
        let batch = scan_invitation_due_batch(&context, &mut conn, cursor, 1).unwrap();
        assert_eq!((batch.inspected, batch.warnings_added), (1, 0));
        let next = batch
            .next
            .expect("physical scan must advance past skipped row");
        assert!(next.after_deadline >= cursor.unwrap().after_deadline);
        cursor = Some(next);
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
