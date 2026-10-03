//! D1 flatness: the observation walk, its publish probe and work discovery
//! visit live rows only, so per-pass cost does not grow with retired seats,
//! superseded generations or completed jobs (ht-p03.12.4).
use super::wake_tests::old_walk;
use super::*;
use crate::ports::{HostInvalidationFence, HostObservationAdmission};
use crate::protocol::time::{Cancellation, MonoInstant};
use crate::store::{WAKE_RECOVERY_WALK_SQL, schema};
use crate::test_support::isolation::{CostCounter, count_vm_units};
use rusqlite::params;
use std::sync::Arc;

struct FixedClock;
impl Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(100)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(1_000)
    }
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(1_000_000),
        cancellation: Cancellation::default(),
    }
}

/// Slack for per-pass fixed work (statement setup, one extra page probe). One
/// unit is 10 VM instructions, so 500 units is 5,000 instructions; the
/// regressions this guards grow by roughly 10x of a 1,000-row scan, far above it.
const CONSTANT_UNITS: u64 = 500;

fn units<T>(db: &Connection, f: impl FnOnce() -> T) -> (T, u64) {
    let counter = crate::test_support::isolation::CostCounter::default();
    let value = crate::test_support::isolation::count_vm_units(db, &counter, f);
    (value, counter.units())
}

fn assert_flat(what: &str, small: u64, big: u64) {
    eprintln!("{what}: small={small} big={big} units");
    assert!(
        big <= small + small / 10 + CONSTANT_UNITS,
        "{what} grew with dead rows: small={small} big={big}"
    );
}

struct Fixture {
    path: std::path::PathBuf,
    context: StoreContext,
}

impl Fixture {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("herdr-discovery-{}.db", uuid::Uuid::new_v4()));
        let context = StoreContext::new(path.clone(), Arc::new(FixedClock));
        let db = context.open_writer().unwrap();
        db.execute(
            "INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'host',1,1)",
            [],
        )
        .unwrap();
        Self { path, context }
    }
    fn reader(&self) -> Connection {
        Connection::open(&self.path).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.path.display()));
        }
    }
}

fn insert_seats(db: &Connection, prefix: &str, count: u64, retired: bool) {
    let (state, target, retired_at, retired_seq) = if retired {
        ("retired", "NULL".to_owned(), "1", "1")
    } else {
        ("resolved", format!("'pane-{prefix}'||x"), "NULL", "NULL")
    };
    db.execute_batch(&format!(
        "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<{count}) \
         INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,retired_at,retired_seq) \
         SELECT '{prefix}'||x,'i','{state}','native',{target},1,1,0,{retired_at},{retired_seq} FROM n"
    ))
    .unwrap();
}

/// Twenty live seats with `retired` retired seats split before and after
/// them, plus `generations` superseded published generations with targets and
/// one active publication (retention has not run).
fn observation_fixture(retired: u64, generations: u64) -> Fixture {
    let fixture = Fixture::new();
    let db = fixture.context.open_writer().unwrap();
    insert_seats(&db, "rb", retired / 2, true);
    insert_seats(&db, "live", 20, false);
    insert_seats(&db, "ra", retired - retired / 2, true);
    db.execute_batch(&format!(
        "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<{generations}) \
         INSERT INTO snapshot_generations(id,instance_id,host_boot,epoch,observation_sequence,incarnation,expected_targets,staged_targets,status,captured_lifecycle_revision,captured_invalidation_revision,published_invalidation_revision,created_at) \
         SELECT 'old'||x,'i','host',1,x,'inc',2,2,'published',0,0,0,0 FROM n;\
         INSERT INTO snapshot_targets(generation_id,target_id,generation,observation_sequence,occupancy,ui_state,observed_at) \
         SELECT g.id,t.t,0,g.observation_sequence,'unknown','unknown',0 FROM snapshot_generations g, (SELECT 'a' AS t UNION ALL SELECT 'b') t;\
         INSERT INTO snapshot_generations(id,instance_id,host_boot,epoch,observation_sequence,incarnation,expected_targets,staged_targets,status,captured_lifecycle_revision,captured_invalidation_revision,published_invalidation_revision,created_at) \
         VALUES ('active','i','host',1,{},'inc',0,0,'published',0,0,0,0);\
         UPDATE host_instances SET active_snapshot_id='active' WHERE id='i';",
        generations + 1
    ))
    .unwrap();
    fixture
}

/// One full observation pass: the published walk, the invalidation walk, and
/// the publish path's nonretired probe, each to completion.
fn observation_pass(fixture: &Fixture, db: &Connection) -> (Vec<String>, Vec<String>, bool) {
    let published = crate::ports::SnapshotGenerationId::store_issued("active".to_owned());
    let budget = budget();
    let mut walked = Vec::new();
    let mut after = 0;
    let mut high = None;
    loop {
        let page =
            seats::saved_seats_page(&fixture.context, db, &published, after, high, 16, &budget)
                .unwrap();
        walked.extend(page.seats.iter().map(|s| s.seat.as_str().to_owned()));
        if !page.has_more {
            break;
        }
        after = page.after_ordinal;
        high = Some(page.high_water_ordinal);
    }
    let fence = HostInvalidationFence {
        admission: HostObservationAdmission {
            instance: "i".to_owned(),
            sequence: 0,
            expected_active: None,
            expected_boot: None,
            expected_epoch: 0,
            lifecycle_revision: 0,
            invalidation_revision: 0,
        },
        invalidation_revision: 0,
    };
    let mut invalidated = Vec::new();
    let mut after = 0;
    let mut high = None;
    loop {
        let page = seats::saved_seats_page_for_invalidation(
            &fixture.context,
            db,
            &fence,
            after,
            high,
            16,
            &budget,
        )
        .unwrap();
        invalidated.extend(page.seats.iter().map(|s| s.seat.as_str().to_owned()));
        if !page.has_more {
            break;
        }
        after = page.after_ordinal;
        high = Some(page.high_water_ordinal);
    }
    (
        walked,
        invalidated,
        seats::has_nonretired_seat(db, "i").unwrap(),
    )
}

#[test]
fn observation_walk_is_flat_in_retired_seats_and_superseded_generations() {
    let measure = |retired, generations| {
        let fixture = observation_fixture(retired, generations);
        let db = fixture.reader();
        let ((walked, invalidated, nonretired), cost) =
            units(&db, || observation_pass(&fixture, &db));
        assert_eq!(walked.len(), 20, "live seats walked");
        assert_eq!(invalidated, walked);
        assert!(nonretired);
        assert!(walked.iter().all(|id| id.starts_with("live")), "{walked:?}");
        cost
    };
    let small = measure(1_000, 100);
    let big = measure(10_000, 1_000);
    assert_flat("observation pass", small, big);
}

fn insert_jobs(db: &Connection, prefix: &str, count: u64, status: &str) {
    db.execute_batch(&format!(
        "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<{count}) \
         INSERT INTO work_jobs(id,kind,subject_id,position,high_water,status) \
         SELECT '{prefix}'||x,'warning_attribution','{prefix}'||x,0,4,'{status}' FROM n"
    ))
    .unwrap();
}

/// Fifty pending/failed jobs with `completed` completed jobs split before,
/// between and after them.
fn work_fixture(completed: u64) -> Fixture {
    let fixture = Fixture::new();
    let db = fixture.context.open_writer().unwrap();
    insert_jobs(&db, "cb", completed / 3, "complete");
    insert_jobs(&db, "pa", 25, "pending");
    insert_jobs(&db, "cm", completed / 3, "complete");
    insert_jobs(&db, "fa", 25, "failed");
    insert_jobs(&db, "ce", completed - 2 * (completed / 3), "complete");
    fixture
}

/// Work discovery to completion, summing each page's VM units on the
/// connection that ran it. Returns the job ids in discovery order.
fn work_pass(fixture: &Fixture, limit: u16) -> (Vec<String>, u64) {
    let mut ids = Vec::new();
    let mut total = 0;
    let mut cursor: Option<WorkPageCursor> = None;
    loop {
        let budget = budget();
        let db = fixture.context.open_query(budget.clone()).unwrap();
        let (page, cost) = units(&db, || {
            pending_work_page_on(
                &fixture.context,
                &db,
                "i",
                PageRequest {
                    limit,
                    ..PageRequest::default()
                },
                cursor.take(),
                &budget,
            )
        });
        let page = page.unwrap();
        total += cost;
        ids.extend(page.items.iter().map(|job| job.id.clone()));
        match page.next_cursor {
            Some(next) => cursor = Some(WorkPageCursor::decode(&next, "i").unwrap()),
            None => break,
        }
    }
    (ids, total)
}

#[test]
fn work_discovery_is_flat_in_completed_jobs() {
    let measure = |completed| {
        let fixture = work_fixture(completed);
        let (ids, cost) = work_pass(&fixture, 10);
        assert_eq!(ids.len(), 50, "pending and failed jobs discovered");
        assert!(
            ids.iter()
                .all(|id| id.starts_with("pa") || id.starts_with("fa"))
        );
        // Ascending ordinal order: the 25 pending first, then the 25 failed.
        assert!(ids[..25].iter().all(|id| id.starts_with("pa")), "{ids:?}");
        cost
    };
    let small = measure(1_000);
    let big = measure(10_000);
    assert_flat("work discovery pass", small, big);
}

/// A continuation issued by the pre-change code (encoded once from
/// `WorkPageCursor { after_ordinal: 40, high_water_ordinal: 90 }` for instance
/// `i`), frozen here so the query change cannot invalidate cursors in flight.
const PRE_CHANGE_WORK_CURSOR: &str = "c3:EgABKDI0KdeVv3U";

#[test]
fn pre_change_work_cursor_still_validates() {
    let cursor = WorkPageCursor::decode(PRE_CHANGE_WORK_CURSOR, "i").unwrap();
    assert_eq!(
        cursor,
        WorkPageCursor {
            after_ordinal: 40,
            high_water_ordinal: 90
        }
    );
    // It continues: discovery resumes after ordinal 40 without a stale or
    // invalid-cursor error, and returns only jobs past it.
    let fixture = Fixture::new();
    let db = fixture.context.open_writer().unwrap();
    insert_jobs(&db, "j", 100, "pending");
    drop(db);
    let budget = budget();
    let db = fixture.context.open_query(budget.clone()).unwrap();
    let page = pending_work_page_on(
        &fixture.context,
        &db,
        "i",
        PageRequest {
            limit: 100,
            ..PageRequest::default()
        },
        Some(cursor),
        &budget,
    )
    .unwrap();
    assert_eq!(page.items.first().map(|job| job.id.as_str()), Some("j41"));
    assert_eq!(page.items.last().map(|job| job.id.as_str()), Some("j90"));
    assert_eq!(page.items.len(), 50);
    assert_eq!(page.high_water_ordinal, 90);
    assert!(page.next_cursor.is_none());
}

const OLD: &str = "00000000-0000-0000-0000-000000000001";

/// 40 reserved seats; `history` more seats around them that are retired or
/// carry a settled (unreserved) wake_work row.
fn with_history(history: u64) -> Connection {
    let db = Connection::open_in_memory().unwrap();
    schema::initialize(&db, || crate::protocol::time::UtcMillis(0)).unwrap();
    db.execute_batch("INSERT INTO host_instances(id,created_at) VALUES ('i',0)")
        .unwrap();
    db.execute_batch(&format!("\
        WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<{})\
        INSERT INTO seats(id,instance_id,state,role,generation,created_at,retired_at,retired_seq) SELECT 's'||x,'i',IIF(x%40=0 OR x>40,IIF(x%2=0,'retired','resolved'),'resolved'),'native',1,0,IIF(x%40=0 OR x>40,IIF(x%2=0,0,NULL),NULL),IIF(x%40=0 OR x>40,IIF(x%2=0,x,NULL),NULL) FROM n;\
        INSERT INTO wake_work(seat_id,reservation_id,reservation_boot) SELECT id,'r'||ordinal,'{OLD}' FROM seats WHERE ordinal<=40;\
        INSERT INTO wake_work(seat_id) SELECT id FROM seats WHERE ordinal>40 AND ordinal%2=1;\
    ", 40 + history)).unwrap();
    db
}

fn walk_units(db: &Connection, walk: impl Fn(u64) -> Vec<u64>) -> (u64, usize) {
    let counter = CostCounter::default();
    let found = count_vm_units(db, &counter, || {
        let (mut after, mut seen) = (0u64, 0usize);
        loop {
            let page = walk(after);
            let Some(last) = page.last() else { break };
            after = *last;
            seen += page.len();
        }
        seen
    });
    (counter.units(), found)
}

fn new_walk(db: &Connection, after: u64) -> Vec<u64> {
    let high = db
        .query_row("SELECT MAX(ordinal) FROM seats", [], |r| r.get::<_, i64>(0))
        .unwrap();
    let mut statement = db.prepare(WAKE_RECOVERY_WALK_SQL).unwrap();
    statement
        .query_map(params!["i", after as i64, high], |r| r.get::<_, i64>(0))
        .unwrap()
        .map(|r| r.unwrap() as u64)
        .collect()
}

// Kills: a recovery walk whose cost follows all seats (the old LEFT JOIN over
// seats) rather than the reserved set, and a plan that scans wake_work instead
// of using wake_work_reserved. Retired seats and settled wake_work rows grow
// 10x with the reserved set fixed; the walk's VM work stays within 1.1x + C.
#[test]
fn recovery_walk_is_flat_in_retired_and_unreserved_rows() {
    const C: u64 = 20;
    let (small, large) = (with_history(2_000), with_history(20_000));
    let (small_units, small_found) = walk_units(&small, |after| new_walk(&small, after));
    let (large_units, large_found) = walk_units(&large, |after| new_walk(&large, after));
    assert_eq!((small_found, large_found), (40, 40));
    eprintln!("recovery walk vm units/10: {small_units} -> {large_units}");
    assert!(
        large_units <= small_units + small_units / 10 + C,
        "recovery walk grew with history: {small_units} -> {large_units}"
    );
    // The assertion discriminates: the old walk over the same data grows.
    let old_units = |db: &Connection| {
        walk_units(db, |after| {
            let high = db
                .query_row("SELECT MAX(ordinal) FROM seats", [], |r| r.get::<_, i64>(0))
                .unwrap();
            old_walk(db, "i", after, high as u64)
                .into_iter()
                .map(|row| row.0)
                .collect()
        })
        .0
    };
    let (old_small, old_large) = (old_units(&small), old_units(&large));
    assert!(
        old_large > old_small * 5,
        "oracle must grow: {old_small} -> {old_large}"
    );
    let plan: String = small
        .query_row(
            &format!("EXPLAIN QUERY PLAN {WAKE_RECOVERY_WALK_SQL}"),
            params!["i", 0, 1_i64 << 40],
            |r| r.get(3),
        )
        .unwrap();
    assert!(plan.contains("wake_work_reserved"), "{plan}");
}
