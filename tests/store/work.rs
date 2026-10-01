use super::*;
use crate::store::schema;
use rusqlite::Connection;

#[test]
fn durable_work_cursor_commits_only_a_bounded_prefix() {
    let mut db = Connection::open_in_memory().unwrap();
    schema::initialize(&db).unwrap();
    db.execute("INSERT INTO work_jobs(id, kind, subject_id, high_water) VALUES ('j', 'warning_attribution', 'warning', 40)", []).unwrap();
    let found = discover_work(&db, WorkKind::WarningAttribution, 0, 1).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].position, 0);
    assert!(found[0].has_more);

    let tx = db.transaction().unwrap();
    let progress = commit_work_prefix(&tx, "j", 0, 16, 16, true, None).unwrap();
    assert_eq!(progress.completed_units, 16);
    assert_eq!(progress.next_position, 16);
    assert!(progress.has_more);
    tx.commit().unwrap();

    let tx = db.transaction().unwrap();
    assert!(commit_work_prefix(&tx, "j", 0, 1, 17, true, None).is_err());
    tx.rollback().unwrap();
    assert!(WorkAdmission::new(17).is_err());
    assert_eq!(
        discover_work(&db, WorkKind::WarningAttribution, 0, 1).unwrap()[0].position,
        16
    );
}

#[test]
fn sparse_physical_ordinals_advance_without_counting_gaps_as_units() {
    let mut db = Connection::open_in_memory().unwrap();
    schema::initialize(&db).unwrap();
    db.execute("INSERT INTO work_jobs(id, kind, subject_id, high_water) VALUES ('sparse', 'receipt_timer_materialization', 'seat', 1000)", []).unwrap();
    let tx = db.transaction().unwrap();
    let progress = commit_work_prefix(&tx, "sparse", 0, 2, 700, true, None).unwrap();
    assert_eq!(progress.completed_units, 2);
    assert_eq!(progress.next_position, 700);
    tx.commit().unwrap();
    let tx = db.transaction().unwrap();
    assert!(commit_work_prefix(&tx, "sparse", 700, 1, 699, true, None).is_err());
}

#[test]
fn a_non_cursor_finalize_unit_can_commit_without_moving_physical_position() {
    let mut db = Connection::open_in_memory().unwrap();
    schema::initialize(&db).unwrap();
    db.execute("INSERT INTO work_jobs(id, kind, subject_id, high_water) VALUES ('finalize', 'send_attention', 'prep', 2)", []).unwrap();
    let tx = db.transaction().unwrap();
    let progress = commit_work_prefix(&tx, "finalize", 0, 1, 0, true, None).unwrap();
    assert_eq!(progress.completed_units, 1);
    assert_eq!(progress.next_position, 0);
    tx.commit().unwrap();
}

#[test]
fn work_prefix_reports_units_committed_by_this_call_not_the_running_total() {
    // Kills: `commit_work_prefix` reporting `processed_this_turn: 0` (the
    // deadline lane then never sees committed work progress and cannot clear
    // a work job's failure state), or reporting the cumulative
    // `completed_units` as this call's units.
    let mut db = Connection::open_in_memory().unwrap();
    schema::initialize(&db).unwrap();
    db.execute("INSERT INTO work_jobs(id, kind, subject_id, high_water) VALUES ('j', 'warning_attribution', 'warning', 40)", []).unwrap();
    let tx = db.transaction().unwrap();
    let first = commit_work_prefix(&tx, "j", 0, 16, 16, true, None).unwrap();
    assert_eq!(first.processed_this_turn, 16);
    tx.commit().unwrap();
    let tx = db.transaction().unwrap();
    let second = commit_work_prefix(&tx, "j", 16, 3, 19, true, None).unwrap();
    assert_eq!(
        (second.completed_units, second.processed_this_turn),
        (19, 3)
    );
    tx.commit().unwrap();
}
