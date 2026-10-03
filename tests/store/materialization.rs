use super::*;
use crate::protocol::time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis};
use crate::store::{effective::effective_warning_by_id, schema};
use rusqlite::Connection;
use std::sync::atomic::{AtomicU64, Ordering};

struct TestClock(AtomicU64);
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(999_999)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.fetch_add(1, Ordering::SeqCst))
    }
}
fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(10_000),
        cancellation: Cancellation::default(),
    }
}
fn fixture() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.pragma_update(None, "foreign_keys", "ON").unwrap();
    schema::initialize(&db).unwrap();
    db.execute_batch("\
        INSERT INTO host_instances(id,created_at) VALUES('i',0);\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES('t','i','topic','goal',0,0);\
        INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES('s1','i','resolved','native',1,0),('s2','i','resolved','native',1,0),('s3','i','resolved','native',1,0);\
    ").unwrap();
    db
}
fn count(db: &Connection, table: &str) -> i64 {
    db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

fn file_fixture() -> (Connection, std::path::PathBuf) {
    let source = fixture();
    let path = std::env::temp_dir().join(format!("materialization-{}.db", uuid::Uuid::new_v4()));
    source
        .execute("VACUUM INTO ?1", [path.to_str().unwrap()])
        .unwrap();
    drop(source);
    let db = Connection::open(&path).unwrap();
    db.pragma_update(None, "foreign_keys", "ON").unwrap();
    (db, path)
}

fn long_multibyte_diagnostic() -> String {
    format!("{}{}{}", "é".repeat(210), "🙂".repeat(30), "文".repeat(100))
}

#[test]
fn warning_snapshot_survives_later_leave_join_and_affected_deduplicates() {
    let mut db = fixture();
    db.execute_batch("\
        INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq,left_seq) VALUES('t','s1',1,2,11),('t','s2',1,12,NULL);\
        INSERT INTO messages(instance_id,id,thread_id,sequence,kind,event_key,event_json,decision_at,decision_seq) VALUES('i','w','t',1,'warn','w','{}',10,10);\
        INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id) VALUES('w',10,'t',2,'s1','receipt','m:s1');\
        INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES('j','warning_attribution','w',2);\
    ").unwrap();
    let clock = TestClock(AtomicU64::new(0));
    for _ in 0..8 {
        let p = advance_work(
            &mut db,
            "j",
            WorkAdmission::new(1).unwrap(),
            &budget(),
            &clock,
        )
        .unwrap();
        if !p.has_more {
            break;
        }
    }
    let rows: Vec<(String, i64)> = {
        let mut s = db
            .prepare("SELECT seat_id,generation FROM warning_recipients ORDER BY seat_id")
            .unwrap();
        s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(rows, vec![("s1".into(), 10)]);
    assert_eq!(
        db.query_row("SELECT status FROM work_jobs WHERE id='j'", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "complete"
    );
}

#[test]
fn later_anchor_does_not_restart_timer_or_enqueue_duplicate_attention() {
    let mut db = fixture();
    db.execute_batch("\
        INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,recipient_count,status) VALUES('p','i','actor','op',zeroblob(32),'t',0,0,0,0,0,0,1,1,'sealed');\
        INSERT INTO prepared_recipients(preparation_id,seat_id,thread_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES('p','s1','t',1,300,0);\
        INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES('i','m','t',1,'ordinary','body',1000,10);\
        INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES('i','p','m','t',10,1000,1,0,1,0);\
        INSERT INTO seat_availability(seat_id,decision_seq,decision_at,binding_generation,observation_provenance) VALUES('s1',11,1200,1,'verified'),('s1',12,500,1,'verified');\
        INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES('a1','receipt_timer_materialization','1',1),('a2','receipt_timer_materialization','2',1);\
    ").unwrap();
    let clock = TestClock(AtomicU64::new(0));
    for job in ["a1", "a2"] {
        while advance_work(
            &mut db,
            job,
            WorkAdmission::new(1).unwrap(),
            &budget(),
            &clock,
        )
        .unwrap()
        .has_more
        {}
    }
    let times:(Option<i64>,Option<i64>)=db.query_row("SELECT available_at,deadline_at FROM receipt_state WHERE message_id='m' AND seat_id='s1'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(times, (Some(1200), Some(1500)));
    assert_eq!(
        db.query_row(
            "SELECT attention_version FROM wake_work WHERE seat_id='s1'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[test]
fn failed_unit_keeps_prefix_and_restarts_without_duplicate_recipient() {
    let mut db = fixture();
    db.execute_batch("\
        INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES('t','s1',1,2),('t','s2',1,3);\
        INSERT INTO messages(instance_id,id,thread_id,sequence,kind,event_key,event_json,decision_at,decision_seq) VALUES('i','w','t',1,'warn','w','{}',10,10);\
        INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,condition_kind,condition_id) VALUES('w',10,'t',2,'receipt','m:s1');\
        INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES('j','warning_attribution','w',2);\
        CREATE TRIGGER reject_second BEFORE INSERT ON warning_recipients WHEN NEW.seat_id='s2' BEGIN SELECT RAISE(ABORT,'injected failure'); END;\
    ").unwrap();
    let clock = TestClock(AtomicU64::new(0));
    let p = advance_work(
        &mut db,
        "j",
        WorkAdmission::new(16).unwrap(),
        &budget(),
        &clock,
    )
    .unwrap();
    assert_eq!(p.next_position, 1);
    assert_eq!(p.completed_units, 1);
    assert!(p.last_error.is_some());
    assert_eq!(count(&db, "warning_recipients"), 1);
    db.execute_batch("DROP TRIGGER reject_second").unwrap();
    while advance_work(
        &mut db,
        "j",
        WorkAdmission::new(16).unwrap(),
        &budget(),
        &clock,
    )
    .unwrap()
    .has_more
    {}
    assert_eq!(count(&db, "warning_recipients"), 2);
}

#[test]
fn long_multibyte_second_unit_failure_commits_warning_prefix_across_reopen() {
    let (mut db, path) = file_fixture();
    db.execute_batch("\
        INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES('t','s1',1,2),('t','s2',1,3);\
        INSERT INTO messages(instance_id,id,thread_id,sequence,kind,event_key,event_json,decision_at,decision_seq) VALUES('i','w','t',1,'warn','w','{}',10,10);\
        INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,condition_kind,condition_id) VALUES('w',10,'t',2,'receipt','m:s1');\
        INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES('j','warning_attribution','w',2);\
    ").unwrap();
    let diagnostic = long_multibyte_diagnostic();
    db.execute_batch(&format!("CREATE TRIGGER reject_second BEFORE INSERT ON warning_recipients WHEN NEW.seat_id='s2' BEGIN SELECT RAISE(ABORT,'{diagnostic}'); END;")).unwrap();
    let clock = TestClock(AtomicU64::new(0));
    let progress = advance_work(
        &mut db,
        "j",
        WorkAdmission::new(16).unwrap(),
        &budget(),
        &clock,
    )
    .unwrap();
    assert_eq!(
        (
            progress.completed_units,
            progress.next_position,
            progress.has_more
        ),
        (1, 1, true)
    );
    let error = progress.last_error.unwrap();
    assert!(!error.is_empty() && error.len() <= 512);
    assert!(error.contains('é') && error.contains('🙂'));
    drop(db);
    let mut reopened = Connection::open(&path).unwrap();
    reopened.pragma_update(None, "foreign_keys", "ON").unwrap();
    let job: (i64, i64, String, String) = reopened
        .query_row(
            "SELECT completed_units,position,status,last_error FROM work_jobs WHERE id='j'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(job, (1, 1, "failed".into(), error.clone()));
    let warning: (i64, String, String) = reopened
        .query_row(
            "SELECT cursor_ordinal,status,last_error FROM warning_jobs WHERE warning_id='w'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(warning, (1, "failed".into(), error));
    assert_eq!(count(&reopened, "warning_recipients"), 1);
    reopened
        .execute_batch("DROP TRIGGER reject_second")
        .unwrap();
    while advance_work(
        &mut reopened,
        "j",
        WorkAdmission::new(16).unwrap(),
        &budget(),
        &clock,
    )
    .unwrap()
    .has_more
    {}
    assert_eq!(count(&reopened, "warning_recipients"), 2);
    assert_eq!(
        reopened
            .query_row(
                "SELECT count(DISTINCT seat_id) FROM warning_recipients",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
    drop(reopened);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn long_multibyte_first_unit_failure_retains_warning_error_across_reopen() {
    let (mut db, path) = file_fixture();
    db.execute_batch("\
        INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES('t','s1',1,2);\
        INSERT INTO messages(instance_id,id,thread_id,sequence,kind,event_key,event_json,decision_at,decision_seq) VALUES('i','w','t',1,'warn','w','{}',10,10);\
        INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,condition_kind,condition_id) VALUES('w',10,'t',1,'receipt','m:s1');\
        INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES('j','warning_attribution','w',1);\
    ").unwrap();
    let diagnostic = long_multibyte_diagnostic();
    db.execute_batch(&format!("CREATE TRIGGER reject_first BEFORE INSERT ON warning_recipients BEGIN SELECT RAISE(ABORT,'{diagnostic}'); END;")).unwrap();
    let clock = TestClock(AtomicU64::new(0));
    let progress = advance_work(
        &mut db,
        "j",
        WorkAdmission::new(16).unwrap(),
        &budget(),
        &clock,
    )
    .unwrap();
    assert_eq!(
        (
            progress.completed_units,
            progress.next_position,
            progress.has_more
        ),
        (0, 0, true)
    );
    let error = progress.last_error.unwrap();
    assert!(!error.is_empty() && error.len() <= 512);
    drop(db);
    let mut reopened = Connection::open(&path).unwrap();
    reopened.pragma_update(None, "foreign_keys", "ON").unwrap();
    let job: (i64, i64, String, String) = reopened
        .query_row(
            "SELECT completed_units,position,status,last_error FROM work_jobs WHERE id='j'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(job, (0, 0, "failed".into(), error.clone()));
    let warning: (String, String) = reopened
        .query_row(
            "SELECT status,last_error FROM warning_jobs WHERE warning_id='w'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(warning, ("failed".into(), error));
    assert_eq!(count(&reopened, "warning_recipients"), 0);
    reopened.execute_batch("DROP TRIGGER reject_first").unwrap();
    while advance_work(
        &mut reopened,
        "j",
        WorkAdmission::new(16).unwrap(),
        &budget(),
        &clock,
    )
    .unwrap()
    .has_more
    {}
    assert_eq!(count(&reopened, "warning_recipients"), 1);
    drop(reopened);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn long_multibyte_cleanup_failure_retains_nonwarning_error() {
    let mut db = fixture();
    db.execute_batch("\
        INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES('p','i','actor','op',zeroblob(32),'t',0,0,0,0,0,0,1,'discarded');\
        INSERT INTO prepared_recipients(preparation_id,seat_id,thread_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES('p','s1','t',1,300,0);\
        INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES('cleanup','preparation_cleanup','p',0);\
    ").unwrap();
    let diagnostic = long_multibyte_diagnostic();
    db.execute_batch(&format!("CREATE TRIGGER reject_delete BEFORE DELETE ON prepared_recipients BEGIN SELECT RAISE(ABORT,'{diagnostic}'); END;")).unwrap();
    let clock = TestClock(AtomicU64::new(0));
    let progress = advance_work(
        &mut db,
        "cleanup",
        WorkAdmission::new(16).unwrap(),
        &budget(),
        &clock,
    )
    .unwrap();
    assert_eq!(
        (
            progress.completed_units,
            progress.next_position,
            progress.has_more
        ),
        (0, 0, true)
    );
    let error = progress.last_error.unwrap();
    assert!(!error.is_empty() && error.len() <= 512);
    let job: (String, String) = db
        .query_row(
            "SELECT status,last_error FROM work_jobs WHERE id='cleanup'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(job, ("failed".into(), error));
    assert_eq!(count(&db, "prepared_recipients"), 1);
}

#[test]
fn first_unit_failure_is_durable_and_retried_at_same_cursor() {
    let mut db = fixture();
    db.execute_batch("\
        INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES('t','s1',1,2);\
        INSERT INTO messages(instance_id,id,thread_id,sequence,kind,event_key,event_json,decision_at,decision_seq) VALUES('i','w','t',1,'warn','w','{}',10,10);\
        INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,condition_kind,condition_id) VALUES('w',10,'t',1,'receipt','m:s1');\
        INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES('j','warning_attribution','w',1);\
        CREATE TRIGGER reject_first BEFORE INSERT ON warning_recipients BEGIN SELECT RAISE(ABORT,'injected first failure'); END;\
    ").unwrap();
    let clock = TestClock(AtomicU64::new(0));
    let p = advance_work(
        &mut db,
        "j",
        WorkAdmission::new(16).unwrap(),
        &budget(),
        &clock,
    )
    .unwrap();
    assert_eq!(p.next_position, 0);
    assert_eq!(p.completed_units, 0);
    assert!(p.last_error.is_some());
    assert_eq!(
        db.query_row("SELECT status FROM work_jobs WHERE id='j'", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "failed"
    );
    assert_eq!(
        db.query_row(
            "SELECT status FROM warning_jobs WHERE warning_id='w'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "failed"
    );
    db.execute_batch("DROP TRIGGER reject_first").unwrap();
    while advance_work(
        &mut db,
        "j",
        WorkAdmission::new(16).unwrap(),
        &budget(),
        &clock,
    )
    .unwrap()
    .has_more
    {}
    assert_eq!(count(&db, "warning_recipients"), 1);
}

#[test]
fn expired_call_budget_returns_committed_prefix_with_error() {
    let mut db = fixture();
    db.execute_batch("\
        INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES('t','s1',1,2),('t','s2',1,3);\
        INSERT INTO messages(instance_id,id,thread_id,sequence,kind,event_key,event_json,decision_at,decision_seq) VALUES('i','w','t',1,'warn','w','{}',10,10);\
        INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,condition_kind,condition_id) VALUES('w',10,'t',2,'invitation','missing');\
        INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES('j','warning_attribution','w',2);\
    ").unwrap();
    let clock = TestClock(AtomicU64::new(0));
    let short = CallBudget {
        deadline: MonoInstant(4),
        cancellation: Cancellation::default(),
    };
    let p = advance_work(
        &mut db,
        "j",
        WorkAdmission::new(16).unwrap(),
        &short,
        &clock,
    )
    .unwrap();
    assert_eq!(p.completed_units, 1);
    assert_eq!(p.next_position, 1);
    assert!(p.last_error.is_some());
    assert_eq!(count(&db, "warning_recipients"), 1);
}

#[test]
fn published_manifest_warning_and_first_anchor_project_without_new_decision() {
    let mut db = fixture();
    db.execute_batch("\
        INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,recipient_count,warning_count,status) VALUES('p','i','actor','op',zeroblob(32),'t',0,0,0,0,0,0,1,1,1,'sealed');\
        INSERT INTO prepared_recipients(preparation_id,seat_id,thread_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES('p','s1','t',1,300,0);\
        INSERT INTO prepared_unavailable_warnings(preparation_id,warning_key,warning_id,affected_seat_id,unavailability_episode,warning_offset,event_json) VALUES('p','key','w','s1',1,1,'{}');\
        INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES('i','m','t',1,'ordinary','body',1000,10);\
        INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES('i','p','m','t',10,1000,1,0,1,1);\
        INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES('send','send_attention','p',3);\
        INSERT INTO seat_availability(seat_id,decision_seq,decision_at,binding_generation,observation_provenance) VALUES('s1',11,1200,1,'verified');\
    ").unwrap();
    let before = effective_warning_by_id(&db, "w").unwrap().unwrap();
    let clock = TestClock(AtomicU64::new(0));
    while advance_work(
        &mut db,
        "send",
        WorkAdmission::new(1).unwrap(),
        &budget(),
        &clock,
    )
    .unwrap()
    .has_more
    {}
    assert_eq!(effective_warning_by_id(&db, "w").unwrap().unwrap(), before);
    assert_eq!(count(&db, "warning_jobs"), 1);
    assert_eq!(count(&db, "work_jobs"), 2);
    let (available,deadline): (Option<i64>,Option<i64>) = db.query_row("SELECT available_at,deadline_at FROM receipt_state WHERE message_id='m' AND seat_id='s1'",[],|r| Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!((available, deadline), (Some(1200), Some(1500)));
    assert_eq!(count(&db, "messages"), 1);
}

#[test]
fn warning_job_identity_collision_stops_send_projection_at_same_unit() {
    let mut db = fixture();
    db.execute_batch("\
        INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,warning_count,status) VALUES('p','i','actor','op',zeroblob(32),'t',0,0,0,0,0,0,0,1,'sealed');\
        INSERT INTO prepared_unavailable_warnings(preparation_id,warning_key,warning_id,affected_seat_id,unavailability_episode,warning_offset,event_json) VALUES('p','key','w','s1',1,1,'{}');\
        INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES('i','m','t',1,'ordinary','body',1000,10);\
        INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES('i','p','m','t',10,1000,1,0,0,1);\
        INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id) VALUES('w',9,'t',0,'s2','unavailable','other');\
        INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES('send','send_attention','p',2);\
    ").unwrap();
    let clock = TestClock(AtomicU64::new(0));
    let p = advance_work(
        &mut db,
        "send",
        WorkAdmission::new(16).unwrap(),
        &budget(),
        &clock,
    )
    .unwrap();
    assert_eq!(p.next_position, 0);
    assert!(p.last_error.is_some());
    assert_eq!(count(&db, "warning_jobs"), 1);
}

#[test]
fn existing_warning_recipient_with_wrong_event_generation_fails_closed() {
    let mut db = fixture();
    db.execute_batch("\
        INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES('t','s1',1,2);\
        INSERT INTO messages(instance_id,id,thread_id,sequence,kind,event_key,event_json,decision_at,decision_seq) VALUES('i','w','t',1,'warn','w','{}',10,10);\
        INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,condition_kind,condition_id) VALUES('w',10,'t',1,'invitation','missing');\
        INSERT INTO warning_recipients(warning_id,seat_id,generation) VALUES('w','s1',9);\
        INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES('j','warning_attribution','w',1);\
    ").unwrap();
    let clock = TestClock(AtomicU64::new(0));
    let p = advance_work(
        &mut db,
        "j",
        WorkAdmission::new(16).unwrap(),
        &budget(),
        &clock,
    )
    .unwrap();
    assert_eq!(p.next_position, 0);
    assert!(p.last_error.is_some());
    assert_eq!(
        db.query_row(
            "SELECT generation FROM warning_recipients WHERE warning_id='w' AND seat_id='s1'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        9
    );
}

#[test]
fn abandoned_preparation_cleanup_is_bounded_and_keeps_digest() {
    let mut db = fixture();
    db.execute_batch("\
        INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES('p','i','actor','op',zeroblob(32),'t',0,0,0,0,0,0,20,'discarded');\
        INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES('cleanup','preparation_cleanup','p',0);\
    ").unwrap();
    db.execute("INSERT INTO prepared_recipients(preparation_id,seat_id,thread_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES('p','s1','t',1,300,0)",[]).unwrap();
    db.execute("INSERT INTO prepared_unavailable_warnings(preparation_id,warning_key,warning_id,affected_seat_id,unavailability_episode,warning_offset,event_json) VALUES('p','key','w','s2',1,1,'{}')",[]).unwrap();
    let clock = TestClock(AtomicU64::new(0));
    let p = advance_work(
        &mut db,
        "cleanup",
        WorkAdmission::new(1).unwrap(),
        &budget(),
        &clock,
    )
    .unwrap();
    assert_eq!(
        count(&db, "prepared_recipients") + count(&db, "prepared_unavailable_warnings"),
        1
    );
    assert!(p.has_more);
    while advance_work(
        &mut db,
        "cleanup",
        WorkAdmission::new(1).unwrap(),
        &budget(),
        &clock,
    )
    .unwrap()
    .has_more
    {}
    assert_eq!(
        count(&db, "prepared_recipients") + count(&db, "prepared_unavailable_warnings"),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT status FROM send_preparations WHERE id='p'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "discarded"
    );
}

#[test]
fn cleanup_job_cannot_delete_published_logical_receipts() {
    let mut db = fixture();
    db.execute_batch("\
        INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES('p','i','actor','op',zeroblob(32),'t',0,0,0,0,0,0,1,'sealed');\
        INSERT INTO prepared_recipients(preparation_id,seat_id,thread_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES('p','s1','t',1,300,1);\
        INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES('i','m','t',1,'ordinary','body',1000,10);\
        INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES('i','p','m','t',10,1000,1,0,1,0);\
        INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES('cleanup','preparation_cleanup','p',0);\
    ").unwrap();
    let clock = TestClock(AtomicU64::new(0));
    let p = advance_work(
        &mut db,
        "cleanup",
        WorkAdmission::new(16).unwrap(),
        &budget(),
        &clock,
    )
    .unwrap();
    assert!(p.last_error.is_some());
    assert_eq!(count(&db, "prepared_recipients"), 1);
    assert_eq!(count(&db, "send_manifests"), 1);
}

#[test]
fn wide_warning_resumes_after_reopen_without_skipping_or_duplicate_rows() {
    let (mut db, path) = file_fixture();
    for n in 0..40 {
        db.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES(?1,'i','resolved','native',1,0)",[format!("member{n}")]).unwrap();
        db.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES('t',?1,1,2)",[format!("member{n}")]).unwrap();
    }
    db.execute_batch("\
        INSERT INTO messages(instance_id,id,thread_id,sequence,kind,event_key,event_json,decision_at,decision_seq) VALUES('i','w','t',1,'warn','w','{}',10,10);\
        INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,condition_kind,condition_id) VALUES('w',10,'t',40,'invitation','missing');\
        INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES('j','warning_attribution','w',40);\
    ").unwrap();
    let clock = TestClock(AtomicU64::new(0));
    let first = advance_work(
        &mut db,
        "j",
        WorkAdmission::new(16).unwrap(),
        &budget(),
        &clock,
    )
    .unwrap();
    assert!(first.completed_units > 0 && first.completed_units <= 16 && first.has_more);
    drop(db);
    let mut reopened = Connection::open(&path).unwrap();
    reopened.pragma_update(None, "foreign_keys", "ON").unwrap();
    let mut turns = 0;
    let mut prior_completed = first.completed_units;
    loop {
        let p = advance_work(
            &mut reopened,
            "j",
            WorkAdmission::new(16).unwrap(),
            &budget(),
            &clock,
        )
        .unwrap();
        let delta = p.completed_units - prior_completed;
        assert!(delta > 0 && delta <= 16);
        prior_completed = p.completed_units;
        turns += 1;
        if !p.has_more {
            break;
        }
        assert!(turns < 100);
    }
    assert_eq!(count(&reopened, "warning_recipients"), 40);
    assert_eq!(prior_completed, 43);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn restored_occupant_keeps_warning_history_without_warning_only_attention() {
    let mut db = fixture();
    db.execute_batch("\
        UPDATE host_instances SET host_boot='boot',host_epoch=1 WHERE id='i';\
        UPDATE seats SET unavailability_episode=1,unavailability_open=0 WHERE id='s1';\
        INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at) VALUES('s1',1,'target','boot',1,'codex','native','exec','verified',0,20);\
        INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,recipient_count,warning_count,status) VALUES('p','i','actor','op',zeroblob(32),'t',0,0,0,0,0,0,1,1,1,'sealed');\
        INSERT INTO prepared_recipients(preparation_id,seat_id,thread_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES('p','s1','t',1,300,0);\
        INSERT INTO prepared_unavailable_warnings(preparation_id,warning_key,warning_id,affected_seat_id,unavailability_episode,warning_offset,event_json) VALUES('p','key','w','s1',1,1,'{}');\
        INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES('i','m','t',1,'ordinary','body',1000,10);\
        INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES('i','p','m','t',10,1000,1,0,1,1);\
        INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id,phase) VALUES('w',10,'t',0,'s1','unavailable','key','affected');\
        INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES('j','warning_attribution','w',0);\
    ").unwrap();
    let clock = TestClock(AtomicU64::new(0));
    while advance_work(
        &mut db,
        "j",
        WorkAdmission::new(16).unwrap(),
        &budget(),
        &clock,
    )
    .unwrap()
    .has_more
    {}
    assert_eq!(count(&db, "warning_recipients"), 1);
    assert_eq!(count(&db, "wake_work"), 0);
}

#[test]
fn invalidated_host_binding_does_not_suppress_open_unavailability_warning() {
    let mut db = fixture();
    db.execute_batch("\
        UPDATE host_instances SET host_boot='new',host_epoch=2 WHERE id='i';\
        UPDATE seats SET unavailability_episode=1,unavailability_open=1 WHERE id='s1';\
        INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at) VALUES('s1',1,'target','old',1,'codex','native','exec','verified',0,20);\
        INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,recipient_count,warning_count,status) VALUES('p','i','actor','op',zeroblob(32),'t',0,0,0,0,0,0,1,1,1,'sealed');\
        INSERT INTO prepared_recipients(preparation_id,seat_id,thread_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES('p','s1','t',1,300,0);\
        INSERT INTO prepared_unavailable_warnings(preparation_id,warning_key,warning_id,affected_seat_id,unavailability_episode,warning_offset,event_json) VALUES('p','key','w','s1',1,1,'{}');\
        INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES('i','m','t',1,'ordinary','body',1000,10);\
        INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES('i','p','m','t',10,1000,1,0,1,1);\
        INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id,phase) VALUES('w',10,'t',0,'s1','unavailable','key','affected');\
        INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES('j','warning_attribution','w',0);\
    ").unwrap();
    let clock = TestClock(AtomicU64::new(0));
    while advance_work(
        &mut db,
        "j",
        WorkAdmission::new(16).unwrap(),
        &budget(),
        &clock,
    )
    .unwrap()
    .has_more
    {}
    assert_eq!(count(&db, "warning_recipients"), 1);
    assert_eq!(count(&db, "wake_work"), 1);
}

#[test]
fn ack_before_unavailable_warning_attribution_keeps_history_without_wake() {
    let mut db = fixture();
    db.execute_batch("\
        UPDATE seats SET unavailability_episode=1,unavailability_open=1 WHERE id='s1';\
        INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,recipient_count,warning_count,status) VALUES('p','i','actor','op',zeroblob(32),'t',0,0,0,0,0,0,1,1,1,'sealed');\
        INSERT INTO prepared_recipients(preparation_id,seat_id,thread_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES('p','s1','t',1,300,0);\
        INSERT INTO prepared_unavailable_warnings(preparation_id,warning_key,warning_id,affected_seat_id,unavailability_episode,warning_offset,event_json) VALUES('p','key','w','s1',1,1,'{}');\
        INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES('i','m','t',1,'ordinary','body',1000,10);\
        INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES('i','p','m','t',10,1000,1,0,1,1);\
        INSERT INTO receipt_state(message_id,seat_id,state,acked_at) VALUES('m','s1','acked',1010);\
        INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id,phase) VALUES('w',10,'t',0,'s1','unavailable','key','affected');\
        INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES('j','warning_attribution','w',0);\
    ").unwrap();
    let clock = TestClock(AtomicU64::new(0));
    while advance_work(
        &mut db,
        "j",
        WorkAdmission::new(16).unwrap(),
        &budget(),
        &clock,
    )
    .unwrap()
    .has_more
    {}
    assert_eq!(count(&db, "warning_recipients"), 1);
    assert_eq!(count(&db, "wake_work"), 0);
}

#[test]
fn current_offer_suppresses_projection_wake_but_successor_gets_attention() {
    let mut db = fixture();
    db.execute_batch("\
        UPDATE host_instances SET decision_seq=10 WHERE id='i';\
        INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at) VALUES('v','t','s1',1,'pending',1,0,300,300);\
        INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at) VALUES('s1',1,'target','boot',1,'codex','native','old','verified',0,1);\
        INSERT INTO warning_offer(seat_id,binding_generation,execution_id,offered_through_seq) VALUES('s1',1,'old',10);\
        INSERT INTO messages(instance_id,id,thread_id,sequence,kind,event_key,event_json,decision_at,decision_seq,event_offset) VALUES('i','w1','t',1,'warn','w1','{}',10,10,0),('i','w2','t',2,'warn','w2','{}',10,10,1);\
        INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id,phase) VALUES('w1',10,'t',0,'s1','invitation','v','affected'),('w2',10,'t',0,'s1','invitation','v','affected');\
        INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES('j1','warning_attribution','w1',0),('j2','warning_attribution','w2',0);\
    ").unwrap();
    let clock = TestClock(AtomicU64::new(0));
    while advance_work(
        &mut db,
        "j1",
        WorkAdmission::new(16).unwrap(),
        &budget(),
        &clock,
    )
    .unwrap()
    .has_more
    {}
    assert_eq!(count(&db, "warning_recipients"), 1);
    assert_eq!(count(&db, "wake_work"), 0);
    db.execute_batch("\
        UPDATE occupant_bindings SET ended_at=20 WHERE seat_id='s1' AND generation=1;\
        UPDATE seats SET generation=2 WHERE id='s1';\
        INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at) VALUES('s1',2,'target','boot',1,'codex','native2','new','verified',20,21);\
    ").unwrap();
    while advance_work(
        &mut db,
        "j2",
        WorkAdmission::new(16).unwrap(),
        &budget(),
        &clock,
    )
    .unwrap()
    .has_more
    {}
    assert_eq!(count(&db, "warning_recipients"), 2);
    assert_eq!(
        db.query_row(
            "SELECT attention_version FROM wake_work WHERE seat_id='s1'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

fn completed_at(db: &Connection, job: &str) -> (String, Option<i64>) {
    db.query_row(
        "SELECT status, completed_at FROM work_jobs WHERE id=?1",
        [job],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .unwrap()
}

fn drive(db: &mut Connection, job: &str) {
    let clock = TestClock(AtomicU64::new(0));
    while advance_work(db, job, WorkAdmission::new(1).unwrap(), &budget(), &clock)
        .unwrap()
        .has_more
    {}
}

// ht-p03.12.1: each pruned kind stamps completed_at (the TestClock's UTC
// millis) when its production driver completes the job.
#[test]
fn warning_attribution_completion_stamps_completed_at() {
    let mut db = fixture();
    db.execute_batch("\
        INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq,left_seq) VALUES('t','s1',1,2,NULL);\
        INSERT INTO messages(instance_id,id,thread_id,sequence,kind,event_key,event_json,decision_at,decision_seq) VALUES('i','w','t',1,'warn','w','{}',10,10);\
        INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id) VALUES('w',10,'t',1,'s1','receipt','m:s1');\
        INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES('j','warning_attribution','w',1);\
    ").unwrap();
    assert_eq!(completed_at(&db, "j"), ("pending".into(), None));
    drive(&mut db, "j");
    assert_eq!(completed_at(&db, "j"), ("complete".into(), Some(999_999)));
}

#[test]
fn receipt_timer_completion_stamps_completed_at() {
    let mut db = fixture();
    db.execute_batch("\
        INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,recipient_count,status) VALUES('p','i','actor','op',zeroblob(32),'t',0,0,0,0,0,0,1,1,'sealed');\
        INSERT INTO prepared_recipients(preparation_id,seat_id,thread_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES('p','s1','t',1,300,0);\
        INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES('i','m','t',1,'ordinary','body',1000,10);\
        INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES('i','p','m','t',10,1000,1,0,1,0);\
        INSERT INTO seat_availability(seat_id,decision_seq,decision_at,binding_generation,observation_provenance) VALUES('s1',11,1200,1,'verified');\
        INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES('a1','receipt_timer_materialization','1',1);\
    ").unwrap();
    drive(&mut db, "a1");
    assert_eq!(completed_at(&db, "a1"), ("complete".into(), Some(999_999)));
}

#[test]
fn send_attention_completion_stamps_completed_at() {
    let mut db = fixture();
    db.execute_batch("\
        INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,recipient_count,warning_count,status) VALUES('p','i','actor','op',zeroblob(32),'t',0,0,0,0,0,0,1,1,1,'sealed');\
        INSERT INTO prepared_recipients(preparation_id,seat_id,thread_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES('p','s1','t',1,300,0);\
        INSERT INTO prepared_unavailable_warnings(preparation_id,warning_key,warning_id,affected_seat_id,unavailability_episode,warning_offset,event_json) VALUES('p','key','w','s1',1,1,'{}');\
        INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES('i','m','t',1,'ordinary','body',1000,10);\
        INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES('i','p','m','t',10,1000,1,0,1,1);\
        INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES('send','send_attention','p',3);\
        INSERT INTO seat_availability(seat_id,decision_seq,decision_at,binding_generation,observation_provenance) VALUES('s1',11,1200,1,'verified');\
    ").unwrap();
    drive(&mut db, "send");
    assert_eq!(
        completed_at(&db, "send"),
        ("complete".into(), Some(999_999))
    );
}
