//! Commit-change lane kicks (spec D1): the writer's update/commit/rollback
//! hooks, the table to lane map and the per-origin commit counter.
use crate::{
    protocol::{
        results::ApiError,
        time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis},
    },
    service::{
        kicks::{
            CommitKicks, KNOWN_UNMAPPED, Lane, LaneSet, enter_lane, lanes_for_table, mapped_tables,
        },
        pacer::{Pacer, Wake},
    },
    store::{SqliteStore, StoreSettings, connection::StoreContext},
    test_support::isolation::TestIsolation,
};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

struct Fixed;
impl Clock for Fixed {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(100)
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

type Flushed = Arc<Mutex<Vec<(LaneSet, Option<Lane>)>>>;

struct Fixture {
    _iso: TestIsolation,
    path: PathBuf,
    kicks: Arc<CommitKicks>,
    store: Arc<SqliteStore>,
    flushed: Flushed,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let iso = TestIsolation::new(label);
        let path = iso.state_root().join("store.db");
        let kicks = Arc::new(CommitKicks::default());
        let store = SqliteStore::new(
            StoreContext::new(path.clone(), Arc::new(Fixed)),
            "i",
            StoreSettings::default(),
        )
        .unwrap()
        .with_commit_kicks(Arc::clone(&kicks));
        // The raw rows below skip parents and the immutability triggers: the
        // subject here is the update hook, not the schema's own guards.
        {
            let db = store.writer(&budget()).unwrap();
            db.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
            let triggers: Vec<String> = db
                .prepare("SELECT name FROM sqlite_master WHERE type='trigger'")
                .unwrap()
                .query_map([], |row| row.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            for name in triggers {
                db.execute_batch(&format!("DROP TRIGGER {name}")).unwrap();
            }
        }
        let flushed: Flushed = Arc::default();
        let sink = Arc::clone(&flushed);
        store.set_kick_sink(Box::new(move |lanes, origin| {
            sink.lock().unwrap().push((lanes, origin));
        }));
        // Setup commits above carry no rows; start from a clean slate anyway.
        flushed.lock().unwrap().clear();
        Self {
            _iso: iso,
            path,
            kicks,
            store: Arc::new(store),
            flushed,
        }
    }

    /// One writer turn running `sql`; the turn's kicks flush when it drops.
    fn run(&self, sql: &str) {
        self.try_run(sql).unwrap();
    }

    fn try_run(&self, sql: &str) -> Result<(), ApiError> {
        let db = self.store.writer(&budget())?;
        db.execute_batch(sql).unwrap();
        Ok(())
    }

    fn flushed(&self) -> Vec<(LaneSet, Option<Lane>)> {
        std::mem::take(&mut *self.flushed.lock().unwrap())
    }

    fn count(&self, key: &str) -> u64 {
        self.store.commit_counts()[key]
    }
}

fn set(lanes: &[Lane]) -> LaneSet {
    lanes
        .iter()
        .fold(LaneSet::EMPTY, |set, lane| set.with(*lane))
}

/// A minimal valid row per mapped table (foreign keys and triggers are off).
const INSERTS: &[(&str, &str)] = &[
    ("wake_work", "INSERT INTO wake_work(seat_id) VALUES ('s1')"),
    (
        "seats",
        "INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('s1','i','unresolved','native',0,0)",
    ),
    (
        "occupant_bindings",
        "INSERT INTO occupant_bindings(seat_id,generation,target_id,terminal_id,incarnation,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at) VALUES ('s1',0,'t','term','inc','b',0,'codex','n','e','p',0)",
    ),
    (
        "seat_availability",
        "INSERT INTO seat_availability(seat_id,decision_seq,decision_at,binding_generation,observation_provenance) VALUES ('s1',1,0,0,'p')",
    ),
    (
        "warning_recipients",
        "INSERT INTO warning_recipients(warning_id,seat_id,generation) VALUES ('w','s1',1)",
    ),
    (
        "warning_offer",
        "INSERT INTO warning_offer(seat_id,binding_generation,execution_id) VALUES ('s1',0,'e')",
    ),
    (
        "work_jobs",
        "INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES ('j','send_attention','sub',0)",
    ),
    (
        "warning_jobs",
        "INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,condition_kind,condition_id) VALUES ('w',1,'t',0,'invitation','c')",
    ),
    (
        "retirements",
        "INSERT INTO retirements(id,seat_id,cutover_at,closure_boot,closure_epoch,closure_target,closure_generation) VALUES ('r','s1',0,'b',0,'t',0)",
    ),
    (
        "invitations",
        "INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at) VALUES ('inv','t','s1',1,'pending',1,0,1,0)",
    ),
    (
        "invitation_cancellations",
        "INSERT INTO invitation_cancellations(invitation_id,requirement_id,cancelled_at) VALUES ('inv','req',0)",
    ),
    (
        "receipts",
        "INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES ('m','t','s1','pending',1)",
    ),
    (
        "receipt_state",
        "INSERT INTO receipt_state(message_id,seat_id,state) VALUES ('m','s1','pending')",
    ),
];

const WAKE_INSERT: &str = "INSERT INTO wake_work(seat_id) VALUES ('s1')";
const SEAT_INSERT: &str = "INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('s1','i','unresolved','native',0,0)";
const JOB_INSERT: &str =
    "INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES ('j','send_attention','sub',0)";
const GENERATION_INSERT: &str = "INSERT INTO snapshot_generations(id,instance_id,host_boot,epoch,observation_sequence,incarnation,expected_targets,status,captured_lifecycle_revision,captured_invalidation_revision,created_at) VALUES ('g','i','b',1,1,'x',0,'building',0,0,0)";
const TARGET_INSERT: &str = "INSERT INTO snapshot_targets(generation_id,target_id,generation,observation_sequence,occupancy,ui_state,observed_at) VALUES ('g','t',0,1,'unknown','unknown',0)";
const OPERATION_INSERT: &str = "INSERT INTO operations(actor_scope,operation_key,digest,result_json,decided_at) VALUES ('a',hex(randomblob(8)),zeroblob(32),'{}',0)";

#[test]
fn every_table_is_classified() {
    let fixture = Fixture::new("kicks-classify");
    let db = fixture.store.writer(&budget()).unwrap();
    let tables: Vec<String> = db
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(tables.len() > 30, "{tables:?}");
    let mapped: BTreeSet<&str> = mapped_tables().collect();
    for table in &tables {
        assert!(
            mapped.contains(table.as_str()) || KNOWN_UNMAPPED.contains(&table.as_str()),
            "table {table} is not classified in service::kicks"
        );
    }
    // The lists carry no stale names, and a name is never both.
    for name in KNOWN_UNMAPPED
        .iter()
        .filter(|name| **name != "sqlite_sequence")
    {
        assert!(tables.iter().any(|t| t == name), "stale unmapped {name}");
        assert!(!mapped.contains(name), "{name} is mapped and unmapped");
    }
    for name in &mapped {
        assert!(tables.iter().any(|t| t == name), "stale mapped {name}");
    }
    for table in ["snapshot_generations", "snapshot_targets", "host_instances"] {
        assert_eq!(lanes_for_table(table), LaneSet::EMPTY, "{table}");
        assert!(KNOWN_UNMAPPED.contains(&table));
    }
    assert_eq!(lanes_for_table("wake_work"), set(&[Lane::Wakes]));
    assert_eq!(lanes_for_table("work_jobs"), set(&[Lane::Deadlines]));
}

#[test]
fn mapped_commit_kicks_after_visibility() {
    let fixture = Fixture::new("kicks-visible");
    let pacer = Arc::new(Pacer::new("wake", Arc::new(Fixed), Cancellation::default()));
    fixture.kicks.register(Lane::Wakes, Arc::clone(&pacer));
    let seen: Arc<Mutex<Vec<i64>>> = Arc::default();
    let (path, seen_sink) = (fixture.path.clone(), Arc::clone(&seen));
    fixture.store.set_kick_sink(Box::new(move |_, _| {
        let reader = rusqlite::Connection::open(&path).unwrap();
        let rows = reader
            .query_row("SELECT count(*) FROM wake_work", [], |row| row.get(0))
            .unwrap();
        seen_sink.lock().unwrap().push(rows);
    }));
    fixture.run(WAKE_INSERT);
    assert_eq!(*seen.lock().unwrap(), vec![1], "row visible at kick time");
    // The registered Pacer really latched the kick.
    assert_eq!(pacer.wait_blocking(Duration::from_secs(5)), Wake::Kicked);
}

#[test]
fn rollback_and_noop_commits_kick_nothing() {
    let fixture = Fixture::new("kicks-rollback");
    fixture.run("BEGIN; INSERT INTO wake_work(seat_id) VALUES ('s1'); ROLLBACK;");
    fixture.run("BEGIN; UPDATE wake_work SET seat_id=seat_id WHERE 0; COMMIT;");
    assert_eq!(fixture.flushed(), vec![]);
    assert_eq!(fixture.count("request"), 0);
    // The rolled-back insert left no pending set behind for a later commit.
    fixture.run(OPERATION_INSERT);
    assert_eq!(fixture.flushed(), vec![]);
    assert_eq!(fixture.count("request"), 1);
    fixture.run(WAKE_INSERT);
    assert_eq!(fixture.flushed(), vec![(set(&[Lane::Wakes]), None)]);
}

#[test]
fn own_origin_is_not_self_kicked() {
    let fixture = Fixture::new("kicks-origin");
    let _wakes = enter_lane(Lane::Wakes);
    fixture.run(WAKE_INSERT);
    assert_eq!(
        fixture.flushed(),
        vec![],
        "a Wakes commit does not kick Wakes"
    );
    fixture.run(JOB_INSERT);
    assert_eq!(
        fixture.flushed(),
        vec![(set(&[Lane::Deadlines]), Some(Lane::Wakes))]
    );
}

#[test]
fn each_table_kicks_exactly_its_lanes() {
    let fixture = Fixture::new("kicks-tables");
    assert_eq!(
        INSERTS
            .iter()
            .map(|(table, _)| *table)
            .collect::<BTreeSet<_>>(),
        mapped_tables().collect::<BTreeSet<_>>(),
        "every mapped table has an insert fixture"
    );
    for (table, insert) in INSERTS {
        let expected = lanes_for_table(table);
        assert_ne!(expected, LaneSet::EMPTY, "{table}");
        let first_column: String = fixture
            .store
            .writer(&budget())
            .unwrap()
            .query_row(
                &format!("SELECT name FROM pragma_table_info('{table}') LIMIT 1"),
                [],
                |row| row.get(0),
            )
            .unwrap();
        for (verb, sql) in [
            ("insert", insert.to_string()),
            (
                "update",
                format!("UPDATE {table} SET {first_column}={first_column} WHERE 1"),
            ),
            ("delete", format!("DELETE FROM {table} WHERE 1")),
        ] {
            fixture.run(&sql);
            assert_eq!(
                fixture.flushed(),
                vec![(expected, None)],
                "{verb} on {table} kicks exactly {expected:?}"
            );
        }
    }
    // One unmapped table kicks nothing but still counts as a commit.
    let before = fixture.count("request");
    fixture.run(OPERATION_INSERT);
    assert_eq!(fixture.flushed(), vec![]);
    assert_eq!(fixture.count("request"), before + 1);
}

#[test]
fn two_thread_interleaving_keeps_the_request_kick() {
    let fixture = Arc::new(Fixture::new("kicks-interleave"));
    fixture.run(SEAT_INSERT);
    fixture.flushed();
    let armed = Arc::new(AtomicBool::new(true));
    let (paused_tx, paused_rx) = mpsc::channel::<()>();
    let (resume_tx, resume_rx) = mpsc::channel::<()>();
    let resume_rx = Mutex::new(resume_rx);
    let hook_armed = Arc::clone(&armed);
    fixture.store.set_kick_pause(Box::new(move || {
        // Only the request-origin turn pauses, and only once.
        if crate::service::kicks::current_origin().is_none()
            && hook_armed.swap(false, Ordering::SeqCst)
        {
            paused_tx.send(()).unwrap();
            resume_rx.lock().unwrap().recv().unwrap();
        }
    }));
    let request = {
        let fixture = Arc::clone(&fixture);
        std::thread::spawn(move || fixture.run(WAKE_INSERT))
    };
    paused_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("request turn paused after releasing the writer");
    let lane = {
        let fixture = Arc::clone(&fixture);
        std::thread::spawn(move || {
            let _origin = enter_lane(Lane::Wakes);
            // Wakes-mapped (seats) and Deadlines-mapped (work_jobs) tables.
            fixture.run("UPDATE seats SET generation=generation; INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES ('j','send_attention','sub',0);");
        })
    };
    lane.join().unwrap();
    assert_eq!(
        fixture.flushed(),
        vec![(set(&[Lane::Deadlines]), Some(Lane::Wakes))],
        "the Wakes-origin turn kicks Deadlines only, before the request turn resumes"
    );
    resume_tx.send(()).unwrap();
    request.join().unwrap();
    assert_eq!(
        fixture.flushed(),
        vec![(set(&[Lane::Wakes]), None)],
        "the request turn's own Wakes kick survives the other turn's commit"
    );
}

#[test]
fn per_origin_counter_counts_commits() {
    let fixture = Fixture::new("kicks-counter");
    let keys = [
        "deadline",
        "wake",
        "observation",
        "retention",
        "admission-observer",
        "request",
    ];
    assert!(keys.iter().all(|key| fixture.count(key) == 0));
    let mut expected = 0;
    let origins: Vec<Option<Lane>> = Lane::ALL
        .into_iter()
        .map(Some)
        .chain(std::iter::once(None))
        .collect();
    for origin in origins {
        let _guard = origin.map(enter_lane);
        fixture.run(OPERATION_INSERT);
        expected += 1;
        let key = origin.map_or("request", Lane::name);
        assert_eq!(fixture.count(key), 1, "{key}");
        assert_eq!(
            keys.iter().map(|key| fixture.count(key)).sum::<u64>(),
            expected,
            "only {key} advanced"
        );
        // A no-op transaction increments nothing.
        fixture.run("BEGIN; UPDATE operations SET decided_at=1 WHERE 0; COMMIT;");
        assert_eq!(
            keys.iter().map(|key| fixture.count(key)).sum::<u64>(),
            expected
        );
    }
}

#[test]
fn retention_origin_kicks_nothing() {
    let fixture = Fixture::new("kicks-retention");
    fixture.run(GENERATION_INSERT);
    let _retention = enter_lane(Lane::Retention);
    fixture.run("DELETE FROM snapshot_generations WHERE id='g'");
    fixture.run(WAKE_INSERT);
    fixture.run(JOB_INSERT);
    assert_eq!(fixture.flushed(), vec![], "retention commits kick no lane");
    assert_eq!(fixture.count("retention"), 3);
}

#[test]
fn snapshot_tables_kick_no_lane_under_any_origin() {
    let fixture = Fixture::new("kicks-snapshots");
    let origins: Vec<Option<Lane>> = std::iter::once(None)
        .chain(Lane::ALL.into_iter().map(Some))
        .collect();
    for origin in origins {
        let _guard = origin.map(enter_lane);
        fixture.run(GENERATION_INSERT);
        fixture.run(TARGET_INSERT);
        fixture.run("UPDATE snapshot_generations SET staged_targets=0 WHERE 1");
        fixture.run("UPDATE snapshot_targets SET observed_at=1 WHERE 1");
        fixture
            .run("DELETE FROM snapshot_targets WHERE 1; DELETE FROM snapshot_generations WHERE 1");
        assert_eq!(fixture.flushed(), vec![], "{origin:?}");
    }
}

#[test]
fn spawn_blocking_after_lane_task_has_no_origin() {
    let fixture = Arc::new(Fixture::new("kicks-blocking"));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        let lane_fixture = Arc::clone(&fixture);
        tokio::task::spawn_blocking(move || {
            let _origin = enter_lane(Lane::Deadlines);
            lane_fixture.run(OPERATION_INSERT);
        })
        .await
        .unwrap();
        let request_fixture = Arc::clone(&fixture);
        tokio::task::spawn_blocking(move || {
            assert_eq!(crate::service::kicks::current_origin(), None);
            request_fixture.run(OPERATION_INSERT);
        })
        .await
        .unwrap();
    });
    assert_eq!(fixture.count("deadline"), 1);
    assert_eq!(fixture.count("request"), 1);
}

fn store_sources() -> Vec<(String, String)> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/store");
    std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .map(|path| {
            (
                path.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read_to_string(path).unwrap(),
            )
        })
        .collect()
}

#[test]
fn no_mapped_table_is_without_rowid() {
    let fixture = Fixture::new("kicks-rowid");
    let db = fixture.store.writer(&budget()).unwrap();
    for table in mapped_tables() {
        let sql: String = db
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='table' AND name=?1",
                [table],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            !sql.to_ascii_uppercase().contains("WITHOUT ROWID"),
            "{table} has no update hook"
        );
    }
    // No `DELETE FROM <mapped table>` without a WHERE: SQLite's truncate
    // optimization skips the update hook.
    for (file, text) in store_sources() {
        for table in mapped_tables() {
            let needle = format!("DELETE FROM {table}");
            for (at, _) in text.match_indices(&needle) {
                let rest = &text[at + needle.len()..];
                if rest.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_') {
                    continue;
                }
                let head: String = rest.chars().take(24).collect();
                assert!(
                    head.trim_start().to_ascii_uppercase().starts_with("WHERE"),
                    "{file}: unqualified DELETE FROM {table}: {head:?}"
                );
            }
        }
    }
}

#[test]
fn writer_guard_is_private() {
    // Writer access compiles only through `SqliteStore::writer()`, which
    // returns `WriterTurn`. Structural check (no compile-fail harness is a
    // dev-dependency): the writer mutex is locked in exactly one place, and
    // the only `MutexGuard<Connection>` is `WriterTurn`'s private field.
    let sources = store_sources();
    let all: String = sources.iter().map(|(_, text)| text.as_str()).collect();
    assert_eq!(all.matches("self.writer.try_lock()").count(), 1);
    assert_eq!(all.matches("writer.lock()").count(), 0);
    let guard_mentions: Vec<_> = all
        .lines()
        .filter(|line| line.contains("MutexGuard<") && line.contains("Connection>"))
        .collect();
    assert_eq!(guard_mentions.len(), 1, "{guard_mentions:?}");
    assert!(
        guard_mentions[0].trim_start().starts_with("guard: Option<"),
        "{guard_mentions:?}"
    );
    assert!(
        all.contains("fn writer(&self, budget: &CallBudget) -> Result<WriterTurn<'_>, ApiError>")
    );
}

/// ht-p03.9.4: a committed `work_jobs` insert (request origin) wakes the
/// deadline lane at once, and the pass it triggers is not skipped by the tick
/// gate. The clock never moves, so only the kick can start the second pass:
/// the 5 s safety tick cannot elapse and the gate stays closed until
/// `after_committed_change` reopens it. The job's row is not a real
/// preparation, so its quantum fails; that failure showing up in the lane's
/// diagnostic is the proof the driver reached the job.
/// Kills: a lane that waits for its tick after a commit, and one that is
/// kicked but whose driver skips the pass because the gate is still closed.
#[test]
fn kick_runs_after_committed_change() {
    use crate::service::{
        fair_writer::FairWriter,
        workers::{WorkerStatus, start_deadline_worker},
    };
    let fixture = Fixture::new("kicks-deadline-pass");
    let cancel = Cancellation::default();
    let pacer = Arc::new(Pacer::new("deadline", Arc::new(Fixed), cancel.clone()));
    fixture.kicks.register(Lane::Deadlines, Arc::clone(&pacer));
    let status = Arc::new(WorkerStatus::default());
    let worker = start_deadline_worker(
        fixture.store.clone(),
        Arc::new(FairWriter::new(16)),
        Arc::clone(&pacer),
        cancel.clone(),
        Arc::clone(&status),
    )
    .unwrap();
    let wait = |what: &str, done: &dyn Fn() -> bool| {
        let until = std::time::Instant::now() + Duration::from_secs(5);
        while !done() {
            assert!(std::time::Instant::now() < until, "timed out: {what}");
            std::thread::sleep(Duration::from_millis(1));
        }
    };
    wait("the boot pass", &|| pacer.idle_events() >= 1);
    assert_eq!(status.last_diagnostic(), None, "nothing to drive yet");
    let started = std::time::Instant::now();
    fixture.run(
        "INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES ('j','preparation_cleanup','sub',0)",
    );
    wait("the lane to drive the new job", &|| {
        status
            .last_diagnostic()
            .is_some_and(|text| text.starts_with("work job"))
    });
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "the job waited for a tick: {:?}",
        started.elapsed()
    );
    wait("the lane to block again", &|| pacer.idle_events() >= 2);
    assert_eq!(
        pacer.idle_events(),
        2,
        "one kicked pass, then blocked again"
    );
    cancel.cancel();
    worker.join().unwrap();
}
