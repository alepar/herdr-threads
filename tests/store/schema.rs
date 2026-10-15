use super::*;

use crate::store::schema::{self, record_overdue_if_pending};
use crate::{
    ports::TimeBasis,
    protocol::{
        authority::ObligationRef,
        ids::{InvitationId, MessageId, RetirementJobId, SeatId, ThreadId},
        results::{CommandResult, ErrorCode},
        time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis},
    },
};
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicI64, AtomicU64, Ordering},
    },
};

struct FixedClock;
impl Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(100)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(100)
    }
}

fn db_path() -> PathBuf {
    std::env::temp_dir().join(format!("herdr-threads-store-{}.db", uuid::Uuid::new_v4()))
}

#[test]
fn snapshot_generations_are_hidden_until_one_active_pointer_switch() {
    let (_context, db, _clock) = seeded_db(0);
    db.execute("INSERT INTO snapshot_generations(id,host_id,host_boot,epoch,observation_sequence,incarnation,expected_targets,staged_targets,status,captured_lifecycle_revision,captured_invalidation_revision,created_at) VALUES ('g1',(SELECT min(host_id) FROM herdr_hosts),'b',1,1,'inc',2,0,'building',0,0,0)", []).unwrap();
    db.execute("INSERT INTO snapshot_targets(generation_id,target_id,terminal_id,generation,observation_sequence,occupancy,ui_state,observed_at) VALUES ('g1','p1','t1',1,1,'empty_shell','idle',0)", []).unwrap();
    db.execute("INSERT INTO snapshot_targets(generation_id,target_id,terminal_id,generation,observation_sequence,occupancy,ui_state,observed_at) VALUES ('g1','p2','t2',1,1,'occupied','idle',0)", []).unwrap();
    assert!(db.execute("INSERT INTO snapshot_targets(generation_id,target_id,terminal_id,generation,observation_sequence,occupancy,ui_state,observed_at) VALUES ('g1','p3','t2',1,1,'unknown','unknown',0)", []).is_err());
    let active: Option<String> = db
        .query_row(
            "SELECT active_snapshot_id FROM herdr_hosts WHERE node_id='34750f98bd59fcfc946da45aaabe933b'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(active, None);
    assert!(
        db.execute(
            "UPDATE herdr_hosts SET active_snapshot_id='g1' WHERE node_id='34750f98bd59fcfc946da45aaabe933b'",
            []
        )
        .is_err()
    );
    db.execute(
        "UPDATE snapshot_generations SET staged_targets=2,status='published' WHERE id='g1'",
        [],
    )
    .unwrap();
    db.execute("UPDATE herdr_hosts SET active_snapshot_id='g1',recovery_baseline_generation_id='g1',baseline_hold_unclaimed=1 WHERE node_id='34750f98bd59fcfc946da45aaabe933b'", []).unwrap();
    let active: String = db
        .query_row(
            "SELECT active_snapshot_id FROM herdr_hosts WHERE node_id='34750f98bd59fcfc946da45aaabe933b'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(active, "g1");
    assert!(
        db.execute(
            "UPDATE herdr_hosts SET active_snapshot_id='missing' WHERE node_id='34750f98bd59fcfc946da45aaabe933b'",
            []
        )
        .is_err()
    );
}

#[test]
fn unresolved_origin_is_explicit_and_cannot_attach_to_a_resolved_seat() {
    let (_context, db, _clock) = seeded_db(0);
    assert!(
        db.execute(
            "UPDATE seats SET unresolved_reason='host_invalidation' WHERE id='s'",
            []
        )
        .is_err()
    );
    db.execute(
        "UPDATE seats SET state='unresolved',unresolved_reason='host_invalidation' WHERE id='s'",
        [],
    )
    .unwrap();
    assert_eq!(
        db.query_row(
            "SELECT unresolved_reason FROM seats WHERE id='s'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "host_invalidation"
    );
    assert!(
        db.execute(
            "UPDATE seats SET unresolved_reason='invented' WHERE id='s'",
            []
        )
        .is_err()
    );
    db.execute("INSERT INTO snapshot_generations(id,host_id,host_boot,epoch,observation_sequence,incarnation,expected_targets,staged_targets,status,captured_lifecycle_revision,captured_invalidation_revision,created_at) VALUES ('prior',(SELECT min(host_id) FROM herdr_hosts),'boot',1,1,'inc',0,0,'published',0,0,0)", []).unwrap();
    db.execute("UPDATE seats SET unresolved_from_generation_id='prior',unresolved_prior_binding_generation=1 WHERE id='s'", []).unwrap();
    assert!(
        db.execute("DELETE FROM snapshot_generations WHERE id='prior'", [])
            .is_err()
    );
    assert!(
        db.execute(
            "UPDATE seats SET unresolved_reason='other' WHERE id='s'",
            []
        )
        .is_err()
    );
    assert!(
        db.execute(
            "UPDATE seats SET state='resolved',unresolved_reason=NULL WHERE id='s'",
            []
        )
        .is_err()
    );
    db.execute("UPDATE seats SET state='resolved',unresolved_reason=NULL,unresolved_from_generation_id=NULL,unresolved_prior_binding_generation=NULL WHERE id='s'", []).unwrap();
    db.execute("DELETE FROM snapshot_generations WHERE id='prior'", [])
        .unwrap();
}

struct TestClock {
    utc: AtomicI64,
    mono: AtomicU64,
    samples: AtomicU64,
}
impl TestClock {
    fn new(utc: i64) -> Self {
        Self {
            utc: AtomicI64::new(utc),
            mono: AtomicU64::new(1),
            samples: AtomicU64::new(0),
        }
    }
    fn set(&self, utc: i64) {
        self.utc.store(utc, Ordering::SeqCst);
    }
}
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        self.samples.fetch_add(1, Ordering::SeqCst);
        UtcMillis(self.utc.load(Ordering::SeqCst))
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.mono.load(Ordering::SeqCst))
    }
}

/// An open binding of seat `s` (created on first use); returns its binding_id.
fn seat_s_binding(db: &Connection) -> String {
    db.execute(
        "INSERT OR IGNORE INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) VALUES ('s',1,'p','b',0,'codex','n','e','cooperative_top_level',0,'tm','inc')",
        [],
    )
    .unwrap();
    db.query_row(
        "SELECT binding_id FROM occupant_bindings WHERE seat_id='s'",
        [],
        |r| r.get(0),
    )
    .unwrap()
}

fn seeded_db(utc: i64) -> (StoreContext, Connection, Arc<TestClock>) {
    let clock = Arc::new(TestClock::new(utc));
    let context = StoreContext::new(db_path(), clock.clone()).with_test_host();
    let db = context.open_writer().unwrap();
    db.execute("INSERT INTO seats(id, instance_id, state, role, generation, created_at,home_node,is_local,host_id) VALUES ('s', '34750f98bd59fcfc946da45aaabe933b', 'resolved', 'native', 1, 0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))", []).unwrap();
    db.execute("INSERT INTO threads(id, instance_id, topic, goal, created_at, updated_at) VALUES ('t', '34750f98bd59fcfc946da45aaabe933b', 'topic', 'purpose', 0, 0)", []).unwrap();
    (context, db, clock)
}

/// A temp store file (and its journal/WAL siblings) removed on drop. Tests
/// that tamper with many copies of one store build it once and `copy` it,
/// instead of replaying every migration per case.
struct ScratchStore(std::path::PathBuf);

impl ScratchStore {
    fn new(prefix: &str) -> Self {
        Self(std::env::temp_dir().join(format!("{prefix}-{}.sqlite3", uuid::Uuid::new_v4())))
    }

    /// A fresh store at the latest schema, seeded from this binary's
    /// fresh-schema template (ordinary creation if no template is available).
    fn fresh(prefix: &str) -> Self {
        let store = Self::new(prefix);
        super::fresh_schema_template::seed(store.path());
        store
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }

    fn copy(&self, prefix: &str) -> Self {
        let copy = Self::new(prefix);
        std::fs::copy(&self.0, &copy.0).unwrap();
        copy
    }
}

impl Drop for ScratchStore {
    fn drop(&mut self) {
        for suffix in ["", "-journal", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.0.display()));
        }
    }
}

/// The serialized `main` database of `db` (an independent byte image).
fn image_of(db: &Connection) -> Vec<u8> {
    use rusqlite::ffi;
    let mut size: ffi::sqlite3_int64 = 0;
    // SAFETY: serialize the main schema of our own open connection; the
    // returned sqlite3_malloc buffer is copied and freed here.
    unsafe {
        let data = ffi::sqlite3_serialize(db.handle(), c"main".as_ptr(), &mut size, 0);
        assert!(!data.is_null(), "serialize store image");
        let image = std::slice::from_raw_parts(data, size as usize).to_vec();
        ffi::sqlite3_free(data.cast());
        image
    }
}

/// A private in-memory database holding a copy of `image`.
fn from_image(image: &[u8]) -> Connection {
    use rusqlite::ffi;
    let db = Connection::open_in_memory().unwrap();
    // SAFETY: SQLite takes ownership of the sqlite3_malloc64 copy (FREEONCLOSE)
    // and may grow it (RESIZEABLE); the connection handle is ours and open.
    unsafe {
        let buffer = ffi::sqlite3_malloc64(image.len() as u64).cast::<u8>();
        assert!(!buffer.is_null(), "allocate store image");
        std::ptr::copy_nonoverlapping(image.as_ptr(), buffer, image.len());
        let size = image.len() as ffi::sqlite3_int64;
        let rc = ffi::sqlite3_deserialize(
            db.handle(),
            c"main".as_ptr(),
            buffer,
            size,
            size,
            (ffi::SQLITE_DESERIALIZE_FREEONCLOSE | ffi::SQLITE_DESERIALIZE_RESIZEABLE) as _,
        );
        assert_eq!(rc, ffi::SQLITE_OK, "deserialize store image");
    }
    db
}

/// A private in-memory fresh store at the latest schema. Fresh creation
/// replays every migration (~75 ms of CPU); this copies one image per test
/// process instead: the binary's fresh-schema template (built by the
/// production fresh-creation path), verified once by `schema::initialize`
/// (which creates it the ordinary way if no template is available), so a
/// tamper case starting from a copy needs no verification of its own.
fn fresh_db() -> Connection {
    static IMAGE: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    let image = IMAGE.get_or_init(|| {
        let store = ScratchStore::fresh("ht-fresh-image");
        let db = Connection::open(store.path()).unwrap();
        schema::initialize(&db, || UtcMillis(0), &crate::test_support::test_node_seed()).unwrap();
        image_of(&db)
    });
    let db = from_image(image);
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        schema::NODE_SCHEMA_VERSION
    );
    db
}

// Kills: accepting a v7 database whose digest index is missing or altered
// (for example a partial index missing its predicate, which would change the
// producer's plan or fail its INDEXED BY).
#[test]
fn startup_rejects_missing_or_altered_digest_index() {
    let snapshot = image_of(&fresh_db());
    for tamper in [
        "DROP INDEX messages_thread_warning",
        "DROP INDEX send_manifests_thread_warning; CREATE INDEX send_manifests_thread_warning ON send_manifests(thread_id, base_sequence)",
    ] {
        let db = from_image(&snapshot);
        db.execute_batch(tamper).unwrap();
        let error = schema::initialize(
            &db,
            || crate::protocol::time::UtcMillis(0),
            &crate::test_support::test_node_seed(),
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::IncompatibleSchema, "{tamper}");
        assert!(error.detail.contains("_thread_warning"), "{}", error.detail);
    }
}

// Digest fix2: startup verifies every v8 projection table, index and
// maintaining trigger against the migration text. Kills: accepting a database
// whose settlement trigger is missing (the projection would keep settled rows
// and the digest cost would grow with history again) or whose insertion
// trigger lost its predicate or was dropped (pending attention would never
// reach the projection and be silently hidden).
#[test]
fn startup_rejects_missing_or_altered_digest_projection() {
    let snapshot = image_of(&fresh_db());
    for tamper in [
        "DROP TRIGGER digest_receipt_state_settled_update",
        "DROP TRIGGER digest_manifest_receipt_staged",
        "DROP INDEX digest_open_warnings_affected",
        "DROP INDEX digest_programmatic_warnings_seat",
        "DROP TRIGGER digest_programmatic_warning_projected",
        "DROP TRIGGER digest_manifest_receipt_materialized",
        "DROP TRIGGER digest_open_warning_closed",
        "DROP TRIGGER digest_invitation_created; CREATE TRIGGER digest_invitation_created AFTER INSERT ON invitations BEGIN SELECT 1; END",
    ] {
        let db = from_image(&snapshot);
        db.execute_batch(tamper).unwrap();
        let error = schema::initialize(
            &db,
            || crate::protocol::time::UtcMillis(0),
            &crate::test_support::test_node_seed(),
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::IncompatibleSchema, "{tamper}");
        assert!(
            error.detail.contains("digest_"),
            "{tamper}: {}",
            error.detail
        );
    }
}

#[test]
fn service_requirement_substrate_enforces_owner_link_and_one_effective_episode() {
    let (_context, mut db, _clock) = seeded_db(100);
    let tx = db.transaction().unwrap();
    let author = crate::store::service_substrate::ensure_reserved_author(
        &tx,
        "34750f98bd59fcfc946da45aaabe933b",
        UtcMillis(100),
    )
    .unwrap();
    assert_eq!(
        crate::store::service_substrate::ensure_reserved_author(
            &tx,
            "34750f98bd59fcfc946da45aaabe933b",
            UtcMillis(200)
        )
        .unwrap(),
        author
    );
    tx.commit().unwrap();
    db.execute("PRAGMA foreign_keys=OFF", []).unwrap();
    let other_tx = db.transaction().unwrap();
    let other_author =
        crate::store::service_substrate::ensure_reserved_author(&other_tx, "other", UtcMillis(100))
            .unwrap();
    other_tx.commit().unwrap();
    assert!(
        db.execute(
            "UPDATE threads SET managed_owner_author_id=?1 WHERE id='t'",
            [other_author.as_str()]
        )
        .is_err()
    );
    db.execute(
        "UPDATE threads SET managed_owner_author_id=?1 WHERE id='t'",
        [author.as_str()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO memberships(thread_id,seat_id,state,joined_at) VALUES ('t','s','joined',50)",
        [],
    )
    .unwrap();
    db.execute_batch("\
        INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at) VALUES ('inv1','t','s',1,'pending',1,100,1000,1100);\
        INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at) VALUES ('inv2','t','s',2,'pending',2,200,1000,1200);\
    ").unwrap();
    let insert = "INSERT INTO requirement_episodes(id,thread_id,seat_id,issuer_author_id,invitation_id,state,created_decision_seq,created_at) VALUES (?1,'t','s',?2,?3,'pending',?4,?5)";
    db.execute(insert, params!["req1", author.as_str(), "inv1", 1, 100])
        .unwrap();
    assert!(
        db.execute(insert, params!["req2", author.as_str(), "inv2", 2, 200])
            .is_err()
    );
    let projected = crate::store::service_substrate::effective_membership(
        &db,
        &ThreadId::new("t"),
        &SeatId::new("s"),
    )
    .unwrap();
    assert_eq!(
        projected.voluntary,
        crate::protocol::service::VoluntaryMembershipState::Joined
    );
    assert_eq!(
        projected.requirement.unwrap().state,
        crate::protocol::service::RequirementState::Pending
    );
    assert!(
        db.execute(
            "UPDATE requirement_episodes SET state='accepted', revision=2 WHERE id='req1'",
            []
        )
        .is_err()
    );
    db.execute("UPDATE requirement_episodes SET state='released', revision=2, released_at=300 WHERE id='req1'", []).unwrap();
    assert!(db.execute("UPDATE requirement_episodes SET state='pending', revision=3, released_at=NULL WHERE id='req1'", []).is_err());
    assert!(
        db.execute("DELETE FROM requirement_episodes WHERE id='req1'", [])
            .is_err()
    );
    assert!(
        db.execute(
            "UPDATE service_authors SET created_at=400 WHERE id=?1",
            [author.as_str()]
        )
        .is_err()
    );
    db.execute(insert, params!["req2", author.as_str(), "inv2", 2, 200])
        .unwrap();
    assert_eq!(
        crate::store::service_substrate::effective_membership(
            &db,
            &ThreadId::new("t"),
            &SeatId::new("s")
        )
        .unwrap()
        .voluntary,
        crate::protocol::service::VoluntaryMembershipState::Joined
    );
    assert!(
        db.execute(
            "UPDATE threads SET managed_owner_author_id=NULL WHERE id='t'",
            []
        )
        .is_err()
    );
    assert_eq!(
        crate::store::service_substrate::managed_owner(&db, &ThreadId::new("t")).unwrap(),
        Some(author)
    );
    assert!(
        db.execute(
            "UPDATE requirement_episodes SET state='retired',revision=2 WHERE id='req2'",
            []
        )
        .is_err()
    );
    db.execute(
        "UPDATE requirement_episodes SET state='retired',revision=2,retired_at=400 WHERE id='req2'",
        [],
    )
    .unwrap();
    assert!(
        crate::store::service_substrate::current_requirement(
            &db,
            &ThreadId::new("t"),
            &SeatId::new("s")
        )
        .unwrap()
        .is_none()
    );
}

#[test]
fn service_author_and_requirement_acceptance_keep_explicit_provenance() {
    let (_context, mut db, _clock) = seeded_db(100);
    let tx = db.transaction().unwrap();
    let author = crate::store::service_substrate::ensure_reserved_author(
        &tx,
        "34750f98bd59fcfc946da45aaabe933b",
        UtcMillis(100),
    )
    .unwrap();
    tx.commit().unwrap();
    db.execute(
        "UPDATE threads SET managed_owner_author_id=?1 WHERE id='t'",
        [author.as_str()],
    )
    .unwrap();
    db.execute_batch("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at) VALUES ('inv','t','s',1,'pending',1,100,1000,1100)").unwrap();
    db.execute("INSERT INTO requirement_episodes(id,thread_id,seat_id,issuer_author_id,invitation_id,state,created_decision_seq,created_at) VALUES ('req','t','s',?1,'inv','pending',1,100)", [author.as_str()]).unwrap();
    assert!(db.execute("UPDATE requirement_episodes SET state='accepted',revision=2,accepted_at=150,accepted_by_seat_id='s' WHERE id='req'", []).is_err());
    db.execute("UPDATE requirement_episodes SET state='accepted',revision=2,accepted_at=150,accepted_by_seat_id='s',accepted_generation=1,accepted_observation='native:current' WHERE id='req'", []).unwrap();
    let required = crate::store::service_substrate::current_requirement(
        &db,
        &ThreadId::new("t"),
        &SeatId::new("s"),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        required.state,
        crate::protocol::service::RequirementState::Accepted
    );
    assert_eq!(required.accepted_by, Some(SeatId::new("s")));
    assert_eq!(required.accepted_at, Some(UtcMillis(150)));
    assert_eq!(required.revision, 2);

    let service_message = "INSERT INTO messages(id,instance_id,thread_id,sequence,kind,decision_seq,event_json,decision_at,author_kind,author_service_id) VALUES (?1,'34750f98bd59fcfc946da45aaabe933b','t',?2,'info',?3,'{}',200,?4,?5)";
    assert!(
        db.execute(
            service_message,
            params!["bad", 1, 2, "programmatic", "missing"]
        )
        .is_err()
    );
    db.execute(
        service_message,
        params!["notice", 1, 2, "programmatic", author.as_str()],
    )
    .unwrap();
    assert_eq!(
        crate::store::service_substrate::message_author(&db, &MessageId::new("notice")).unwrap(),
        crate::protocol::service::EventAuthor::Programmatic(author)
    );
    db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,decision_seq,body,decision_at) VALUES ('native2','34750f98bd59fcfc946da45aaabe933b','t',2,'ordinary','s',3,'hello',201)", []).unwrap();
    assert_eq!(
        crate::store::service_substrate::message_author(&db, &MessageId::new("native2")).unwrap(),
        crate::protocol::service::EventAuthor::Native(SeatId::new("s"))
    );
    db.execute("UPDATE requirement_episodes SET state='released',revision=3,released_at=250 WHERE id='req'", []).unwrap();
    assert!(
        crate::store::service_substrate::current_requirement(
            &db,
            &ThreadId::new("t"),
            &SeatId::new("s")
        )
        .unwrap()
        .is_none()
    );
    let accepted_history: (String, i64) = db
        .query_row(
            "SELECT accepted_by_seat_id,accepted_at FROM requirement_episodes WHERE id='req'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(accepted_history, ("s".to_owned(), 150));
}

#[test]
fn v2_verification_rejects_a_weakened_effective_requirement_index() {
    let (_context, db, _clock) = seeded_db(100);
    db.execute_batch("DROP INDEX requirement_episodes_effective; CREATE INDEX requirement_episodes_effective ON requirement_episodes(thread_id,seat_id)").unwrap();
    assert_eq!(
        schema::verify_baseline(&db).unwrap_err().code,
        ErrorCode::IncompatibleSchema
    );
}

fn pending_requirement_fixture() -> Connection {
    let (_context, mut db, _clock) = seeded_db(100);
    let tx = db.transaction().unwrap();
    let author = crate::store::service_substrate::ensure_reserved_author(
        &tx,
        "34750f98bd59fcfc946da45aaabe933b",
        UtcMillis(100),
    )
    .unwrap();
    tx.commit().unwrap();
    db.execute(
        "UPDATE threads SET managed_owner_author_id=?1 WHERE id='t'",
        [author.as_str()],
    )
    .unwrap();
    db.execute_batch("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at) VALUES ('inv','t','s',1,'pending',1,100,1000,1100)").unwrap();
    db.execute("INSERT INTO requirement_episodes(id,thread_id,seat_id,issuer_author_id,invitation_id,state,created_decision_seq,created_at) VALUES ('req','t','s',?1,'inv','pending',1,100)", [author.as_str()]).unwrap();
    db
}

fn acceptance_tuple(db: &Connection) -> (i64, String, i64, String, i64) {
    db.query_row(
        "SELECT revision,accepted_by_seat_id,accepted_generation,accepted_observation,accepted_at FROM requirement_episodes WHERE id='req'",
        [],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)),
    ).unwrap()
}

#[test]
fn accepted_requirement_provenance_cannot_be_rewritten_on_revision_or_terminal_change() {
    let db = pending_requirement_fixture();
    db.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at,home_node,is_local,host_id) VALUES ('other','34750f98bd59fcfc946da45aaabe933b','resolved','native',1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))", []).unwrap();
    db.execute("UPDATE requirement_episodes SET state='accepted',revision=2,accepted_by_seat_id='s',accepted_generation=1,accepted_observation='native:original',accepted_at=150 WHERE id='req'", []).unwrap();
    let original = acceptance_tuple(&db);
    for change in [
        "accepted_by_seat_id='other'",
        "accepted_generation=99",
        "accepted_observation='native:rewritten'",
        "accepted_at=999",
        "accepted_generation=99,accepted_observation='native:rewritten',accepted_at=999",
        "accepted_generation=NULL",
    ] {
        let sql = format!("UPDATE requirement_episodes SET revision=3,{change} WHERE id='req'");
        assert!(
            db.execute(&sql, []).is_err(),
            "accepted tuple rewrite passed: {change}"
        );
        assert_eq!(acceptance_tuple(&db), original);
    }
    db.execute(
        "UPDATE requirement_episodes SET revision=3 WHERE id='req'",
        [],
    )
    .unwrap();
    assert_eq!(acceptance_tuple(&db).0, 3);
    assert!(db.execute("UPDATE requirement_episodes SET state='released',revision=4,released_at=300,accepted_at=999 WHERE id='req'", []).is_err());
    db.execute("UPDATE requirement_episodes SET state='released',revision=4,released_at=300 WHERE id='req'", []).unwrap();
    let released_tuple = acceptance_tuple(&db);
    assert_eq!(released_tuple.0, 4);
    assert_eq!(
        (
            released_tuple.1,
            released_tuple.2,
            released_tuple.3,
            released_tuple.4
        ),
        (original.1, original.2, original.3, original.4)
    );

    let retired = pending_requirement_fixture();
    retired.execute("UPDATE requirement_episodes SET state='accepted',revision=2,accepted_by_seat_id='s',accepted_generation=1,accepted_observation='native:original',accepted_at=150 WHERE id='req'", []).unwrap();
    assert!(retired.execute("UPDATE requirement_episodes SET state='retired',revision=3,retired_at=300,accepted_observation='native:rewritten' WHERE id='req'", []).is_err());
    retired.execute("UPDATE requirement_episodes SET state='retired',revision=3,retired_at=300 WHERE id='req'", []).unwrap();
    assert_eq!(acceptance_tuple(&retired).1, "s");
    assert_eq!(acceptance_tuple(&retired).2, 1);
    assert_eq!(acceptance_tuple(&retired).3, "native:original");
    assert_eq!(acceptance_tuple(&retired).4, 150);
}

#[test]
fn pending_terminal_requirement_cannot_invent_acceptance_provenance() {
    for (state, time_column) in [("released", "released_at"), ("retired", "retired_at")] {
        let db = pending_requirement_fixture();
        let full = format!(
            "UPDATE requirement_episodes SET state='{state}',revision=2,{time_column}=300,accepted_by_seat_id='s',accepted_generation=1,accepted_observation='native:invented',accepted_at=250 WHERE id='req'"
        );
        assert!(
            db.execute(&full, []).is_err(),
            "{state} invented acceptance"
        );
        let partial = format!(
            "UPDATE requirement_episodes SET state='{state}',revision=2,{time_column}=300,accepted_generation=1 WHERE id='req'"
        );
        assert!(
            db.execute(&partial, []).is_err(),
            "{state} accepted a partial tuple"
        );
        let legal = format!(
            "UPDATE requirement_episodes SET state='{state}',revision=2,{time_column}=300 WHERE id='req'"
        );
        db.execute(&legal, []).unwrap();
        let history: (String, Option<i64>, Option<i64>) = db.query_row("SELECT state,accepted_at,accepted_generation FROM requirement_episodes WHERE id='req'", [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
        assert_eq!(history, (state.to_owned(), None, None));
    }
}

#[test]
fn startup_rejects_missing_or_weakened_acceptance_guard_without_history_changes() {
    for replacement in [
        None,
        Some(
            "CREATE TRIGGER requirement_episodes_acceptance_provenance BEFORE UPDATE ON requirement_episodes WHEN 0 BEGIN SELECT RAISE(ABORT,'unreachable'); END",
        ),
    ] {
        let db = pending_requirement_fixture();
        let initial: (i64, String) = db
            .query_row(
                "SELECT revision,state FROM requirement_episodes WHERE id='req'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        db.execute_batch("DROP TRIGGER IF EXISTS requirement_episodes_acceptance_provenance")
            .unwrap();
        if let Some(sql) = replacement {
            db.execute_batch(sql).unwrap();
        }
        let before: Option<String> = db.query_row("SELECT sql FROM sqlite_master WHERE type='trigger' AND name='requirement_episodes_acceptance_provenance'", [], |row| row.get(0)).optional().unwrap();
        assert_eq!(
            schema::initialize(
                &db,
                || crate::protocol::time::UtcMillis(0),
                &crate::test_support::test_node_seed()
            )
            .unwrap_err()
            .code,
            ErrorCode::IncompatibleSchema
        );
        assert_eq!(
            db.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            schema::NODE_SCHEMA_VERSION
        );
        assert_eq!(
            db.query_row(
                "SELECT revision,state FROM requirement_episodes WHERE id='req'",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            )
            .unwrap(),
            initial
        );
        let after: Option<String> = db.query_row("SELECT sql FROM sqlite_master WHERE type='trigger' AND name='requirement_episodes_acceptance_provenance'", [], |row| row.get(0)).optional().unwrap();
        assert_eq!(after, before);
    }
}

#[test]
fn startup_rejects_weakened_requirement_table_constraint() {
    let db = pending_requirement_fixture();
    let original: String = db
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='requirement_episodes'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let weakened = original.replace("AND accepted_generation IS NOT NULL", "");
    assert_ne!(weakened, original);
    db.execute_batch("PRAGMA writable_schema=ON").unwrap();
    db.execute(
        "UPDATE sqlite_schema SET sql=?1 WHERE type='table' AND name='requirement_episodes'",
        [&weakened],
    )
    .unwrap();
    db.execute_batch("PRAGMA writable_schema=OFF").unwrap();
    assert_eq!(
        schema::initialize(
            &db,
            || crate::protocol::time::UtcMillis(0),
            &crate::test_support::test_node_seed()
        )
        .unwrap_err()
        .code,
        ErrorCode::IncompatibleSchema
    );
    assert_eq!(
        db.query_row(
            "SELECT state FROM requirement_episodes WHERE id='req'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "pending"
    );
    assert_eq!(
        db.query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='requirement_episodes'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        weakened
    );
}

#[test]
fn accepted_requirement_requires_actor_on_update_and_direct_insert() {
    let db = pending_requirement_fixture();
    let judge_update = "UPDATE requirement_episodes SET state='accepted',revision=2,accepted_generation=1,accepted_observation='native:claimed',accepted_at=100 WHERE id='req'";
    assert!(db.execute(judge_update, []).is_err());
    let unchanged: (String, i64, Option<String>) = db
        .query_row(
            "SELECT state,revision,accepted_by_seat_id FROM requirement_episodes WHERE id='req'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(unchanged, ("pending".to_owned(), 1, None));
    db.execute("UPDATE requirement_episodes SET state='accepted',revision=2,accepted_by_seat_id='s',accepted_generation=1,accepted_observation='native:claimed',accepted_at=100 WHERE id='req'", []).unwrap();
    assert_eq!(
        acceptance_tuple(&db),
        (2, "s".to_owned(), 1, "native:claimed".to_owned(), 100)
    );

    let direct = pending_requirement_fixture();
    direct.execute("UPDATE requirement_episodes SET state='released',revision=2,released_at=200 WHERE id='req'", []).unwrap();
    direct.execute_batch("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at) VALUES ('inv2','t','s',2,'pending',2,200,1000,1200)").unwrap();
    let missing_actor = "INSERT INTO requirement_episodes(id,thread_id,seat_id,issuer_author_id,invitation_id,state,created_decision_seq,created_at,accepted_generation,accepted_observation,accepted_at) SELECT 'req2','t','s',managed_owner_author_id,'inv2','accepted',2,200,1,'native:claimed',200 FROM threads WHERE id='t'";
    assert!(direct.execute(missing_actor, []).is_err());
    assert_eq!(
        direct
            .query_row(
                "SELECT count(*) FROM requirement_episodes WHERE id='req2'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    let complete = "INSERT INTO requirement_episodes(id,thread_id,seat_id,issuer_author_id,invitation_id,state,created_decision_seq,created_at,accepted_by_seat_id,accepted_generation,accepted_observation,accepted_at) SELECT 'req2','t','s',managed_owner_author_id,'inv2','accepted',2,200,'s',1,'native:claimed',200 FROM threads WHERE id='t'";
    direct.execute(complete, []).unwrap();
    let actor: String = direct
        .query_row(
            "SELECT accepted_by_seat_id FROM requirement_episodes WHERE id='req2'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(actor, "s");
}

#[test]
fn startup_rejects_missing_or_weakened_actor_presence_checks() {
    for weaken in [
        (
            "accepted_by_seat_id IS NOT NULL AND accepted_by_seat_id = seat_id",
            "accepted_by_seat_id = seat_id",
        ),
        (
            "accepted_at IS NOT NULL AND accepted_by_seat_id IS NOT NULL",
            "accepted_at IS NOT NULL AND 1=1",
        ),
    ] {
        let db = pending_requirement_fixture();
        schema::initialize(
            &db,
            || crate::protocol::time::UtcMillis(0),
            &crate::test_support::test_node_seed(),
        )
        .unwrap();
        let original: String = db
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='table' AND name='requirement_episodes'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            original.matches("accepted_by_seat_id IS NOT NULL").count(),
            2
        );
        let weakened = original.replacen(weaken.0, weaken.1, 1);
        assert_ne!(weakened, original);
        let before: (String, i64, Option<String>) = db.query_row("SELECT state,revision,accepted_by_seat_id FROM requirement_episodes WHERE id='req'", [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
        db.execute_batch("PRAGMA writable_schema=ON").unwrap();
        db.execute(
            "UPDATE sqlite_schema SET sql=?1 WHERE type='table' AND name='requirement_episodes'",
            [&weakened],
        )
        .unwrap();
        db.execute_batch("PRAGMA writable_schema=OFF").unwrap();
        assert_eq!(
            schema::initialize(
                &db,
                || crate::protocol::time::UtcMillis(0),
                &crate::test_support::test_node_seed()
            )
            .unwrap_err()
            .code,
            ErrorCode::IncompatibleSchema
        );
        let after: (String, i64, Option<String>) = db.query_row("SELECT state,revision,accepted_by_seat_id FROM requirement_episodes WHERE id='req'", [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
        assert_eq!(after, before);
        assert_eq!(
            db.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            schema::NODE_SCHEMA_VERSION
        );
        assert_eq!(
            db.query_row(
                "SELECT sql FROM sqlite_master WHERE type='table' AND name='requirement_episodes'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            weakened
        );
    }
}

#[test]
fn direct_accepted_insert_requires_native_actor() {
    let db = pending_requirement_fixture();
    db.execute("UPDATE requirement_episodes SET state='released',revision=2,released_at=200 WHERE id='req'", []).unwrap();
    db.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at) VALUES ('inv2','t','s',2,'pending',2,200,1000,1200)", []).unwrap();
    let missing_actor = "INSERT INTO requirement_episodes(id,thread_id,seat_id,issuer_author_id,invitation_id,state,created_decision_seq,created_at,accepted_generation,accepted_observation,accepted_at) SELECT 'req2','t','s',managed_owner_author_id,'inv2','accepted',2,200,1,'native:claimed',200 FROM threads WHERE id='t'";
    assert!(db.execute(missing_actor, []).is_err());
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM requirement_episodes WHERE id='req2'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let complete = "INSERT INTO requirement_episodes(id,thread_id,seat_id,issuer_author_id,invitation_id,state,created_decision_seq,created_at,accepted_by_seat_id,accepted_generation,accepted_observation,accepted_at) SELECT 'req2','t','s',managed_owner_author_id,'inv2','accepted',2,200,'s',1,'native:claimed',200 FROM threads WHERE id='t'";
    db.execute(complete, []).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT accepted_by_seat_id FROM requirement_episodes WHERE id='req2'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "s"
    );
}

#[test]
fn requirement_acceptance_field_subsets_and_terminal_insert_shapes() {
    // One fixture serves every update mask: each rejected partial update
    // leaves the requirement pending at revision 1 (asserted below), and the
    // only accepted shape (mask 15) runs last.
    let db = pending_requirement_fixture();
    for mask in 0..16 {
        let actor = if mask & 1 != 0 { "'s'" } else { "NULL" };
        let generation = if mask & 2 != 0 { "1" } else { "NULL" };
        let observation = if mask & 4 != 0 {
            "'native:claimed'"
        } else {
            "NULL"
        };
        let time = if mask & 8 != 0 { "100" } else { "NULL" };
        let update = format!(
            "UPDATE requirement_episodes SET state='accepted',revision=2,accepted_by_seat_id={actor},accepted_generation={generation},accepted_observation={observation},accepted_at={time} WHERE id='req'"
        );
        let result = db.execute(&update, []);
        if mask == 15 {
            result.unwrap();
            assert_eq!(
                acceptance_tuple(&db),
                (2, "s".to_owned(), 1, "native:claimed".to_owned(), 100)
            );
        } else {
            assert!(result.is_err(), "partial update accepted mask {mask:04b}");
            let status: (String, i64) = db
                .query_row(
                    "SELECT state,revision FROM requirement_episodes WHERE id='req'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(status, ("pending".to_owned(), 1));
        }
    }

    let direct = pending_requirement_fixture();
    direct.execute("UPDATE requirement_episodes SET state='released',revision=2,released_at=200 WHERE id='req'", []).unwrap();
    direct.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at) VALUES ('inv2','t','s',2,'pending',2,200,1000,1200)", []).unwrap();
    for mask in 0..16 {
        let actor = if mask & 1 != 0 { "'s'" } else { "NULL" };
        let generation = if mask & 2 != 0 { "1" } else { "NULL" };
        let observation = if mask & 4 != 0 {
            "'native:claimed'"
        } else {
            "NULL"
        };
        let time = if mask & 8 != 0 { "200" } else { "NULL" };
        let insert = format!(
            "INSERT INTO requirement_episodes(id,thread_id,seat_id,issuer_author_id,invitation_id,state,created_decision_seq,created_at,accepted_by_seat_id,accepted_generation,accepted_observation,accepted_at) SELECT 'req2','t','s',managed_owner_author_id,'inv2','accepted',2,200,{actor},{generation},{observation},{time} FROM threads WHERE id='t'"
        );
        let result = direct.execute(&insert, []);
        if mask == 15 {
            result.unwrap();
            let stored: (String, i64, String, i64) = direct.query_row("SELECT accepted_by_seat_id,accepted_generation,accepted_observation,accepted_at FROM requirement_episodes WHERE id='req2'", [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).unwrap();
            assert_eq!(
                stored,
                ("s".to_owned(), 1, "native:claimed".to_owned(), 200)
            );
        } else {
            assert!(
                result.is_err(),
                "partial accepted INSERT passed mask {mask:04b}"
            );
            assert_eq!(
                direct
                    .query_row(
                        "SELECT count(*) FROM requirement_episodes WHERE id='req2'",
                        [],
                        |row| row.get::<_, i64>(0)
                    )
                    .unwrap(),
                0
            );
        }
    }

    for state in ["released", "retired"] {
        let db = pending_requirement_fixture();
        let terminal_time = if state == "released" {
            "released_at"
        } else {
            "retired_at"
        };
        for (ordinal, tuple) in [
            (2, "NULL,NULL,NULL,NULL"),
            (3, "'s',1,'native:accepted',150"),
        ] {
            let invitation = format!("inv{ordinal}");
            db.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at) VALUES (?1,'t','s',?2,'pending',?2,100,1000,1100)", params![invitation, ordinal]).unwrap();
            let sql = format!(
                "INSERT INTO requirement_episodes(id,thread_id,seat_id,issuer_author_id,invitation_id,state,created_decision_seq,created_at,{terminal_time},accepted_by_seat_id,accepted_generation,accepted_observation,accepted_at) SELECT 'req{ordinal}','t','s',managed_owner_author_id,'inv{ordinal}','{state}',{ordinal},100,200,{tuple} FROM threads WHERE id='t'"
            );
            db.execute(&sql, []).unwrap();
        }
        db.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at) VALUES ('inv4','t','s',4,'pending',4,100,1000,1100)", []).unwrap();
        let partial = format!(
            "INSERT INTO requirement_episodes(id,thread_id,seat_id,issuer_author_id,invitation_id,state,created_decision_seq,created_at,{terminal_time},accepted_generation,accepted_observation,accepted_at) SELECT 'req4','t','s',managed_owner_author_id,'inv4','{state}',4,100,200,1,'native:claimed',150 FROM threads WHERE id='t'"
        );
        assert!(
            db.execute(&partial, []).is_err(),
            "{state} inserted partial acceptance"
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM requirement_episodes WHERE id='req4'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }
}

#[test]
fn structural_seat_proof_is_all_or_none_and_keeps_provenance() {
    let (_, db, _) = seeded_db(0);
    assert!(
        db.execute(
            "UPDATE seats SET target_id='p',structural_terminal_id='term' WHERE id='s'",
            []
        )
        .is_err()
    );
    db.execute("UPDATE seats SET target_id='p',target_generation=2,structural_terminal_id='term',structural_incarnation='inc',structural_incarnation_kind='coherent_enumeration',structural_host_boot='boot',structural_host_epoch=1,structural_connection_epoch=3,structural_observation_sequence=4 WHERE id='s'",[]).unwrap();
    let (terminal,kind,sequence):(String,String,i64)=db.query_row("SELECT structural_terminal_id,structural_incarnation_kind,structural_observation_sequence FROM seats WHERE id='s'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(
        (terminal.as_str(), kind.as_str(), sequence),
        ("term", "coherent_enumeration", 4)
    );
    assert!(
        db.execute(
            "UPDATE seats SET structural_incarnation_kind='native_invocation' WHERE id='s'",
            []
        )
        .is_err()
    );
}

#[test]
fn target_observation_provenance_requires_complete_qualified_tuple() {
    let (_, db, _) = seeded_db(0);
    assert!(db.execute("INSERT INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observation_sequence,observed_at,provenance,incarnation) VALUES ((SELECT min(host_id) FROM herdr_hosts),'p','b',1,1,2,0,'fresh','inc')",[]).is_err());
    db.execute("INSERT INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observation_sequence,observed_at,provenance,terminal_id,connection_epoch,incarnation,incarnation_source_kind) VALUES ((SELECT min(host_id) FROM herdr_hosts),'p','b',1,1,2,0,'fresh','term',3,'inc','native_current_target')",[]).unwrap();
    db.execute("INSERT INTO snapshot_generations(id,host_id,host_boot,epoch,observation_sequence,incarnation,expected_targets,staged_targets,status,captured_lifecycle_revision,captured_invalidation_revision,created_at) VALUES ('g',(SELECT min(host_id) FROM herdr_hosts),'b',1,2,'inc',1,1,'published',0,0,0)",[]).unwrap();
    assert!(db.execute("INSERT INTO snapshot_targets(generation_id,target_id,generation,observation_sequence,connection_epoch,occupancy,ui_state,observed_at) VALUES ('g','p',1,2,3,'empty_shell','idle',0)",[]).is_err());
    db.execute("INSERT INTO snapshot_targets(generation_id,target_id,generation,observation_sequence,connection_epoch,incarnation_source_kind,terminal_id,occupancy,ui_state,observed_at) VALUES ('g','p',1,2,3,'coherent_enumeration','term','empty_shell','idle',0)",[]).unwrap();
}

#[test]
fn read_connection_rejects_previous_experimental_schema_marker() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE schema_identity(marker TEXT PRIMARY KEY); INSERT INTO schema_identity(marker) VALUES ('herdr-threads-shared-v1-r6'); PRAGMA user_version=1;").unwrap();
    assert_eq!(
        schema::verify_query_connection(&db).unwrap_err().code,
        ErrorCode::IncompatibleSchema
    );
}

#[test]
fn invitation_publication_key_is_required_positive_and_immutable() {
    let (_, db, _) = seeded_db(0);
    assert!(db.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,frozen_duration_ms,deadline_at) VALUES ('v','t','s',1,'pending',0,300,300)",[]).is_err());
    db.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at) VALUES ('v','t','s',1,'pending',1,0,300,300)",[]).unwrap();
    assert!(
        db.execute(
            "UPDATE invitations SET created_decision_seq=2 WHERE id='v'",
            []
        )
        .is_err()
    );
    let plan:Vec<String>=db.prepare("EXPLAIN QUERY PLAN SELECT ordinal FROM invitations WHERE seat_id='s' AND created_decision_seq>0 ORDER BY created_decision_seq,ordinal LIMIT 16").unwrap().query_map([],|r|r.get(3)).unwrap().map(Result::unwrap).collect();
    assert!(
        plan.iter()
            .any(|line| line.contains("invitations_seat_decision")),
        "{plan:?}"
    );
}

fn add_invitation(db: &Connection, id: &str, deadline: i64, state: &str) {
    let episode = id.trim_start_matches('v').parse::<i64>().unwrap();
    db.execute("INSERT INTO invitations(id, thread_id, seat_id, episode, state, created_decision_seq, created_at, frozen_duration_ms, deadline_at, accepted_at, accepted_actor_seat_id, accepted_generation, accepted_observation) VALUES (?1, 't', 's', ?2, ?3, ?2, 0, 300000, ?4, CASE WHEN ?3='accepted' THEN 1 END, CASE WHEN ?3='accepted' THEN 's' END, CASE WHEN ?3='accepted' THEN 1 END, CASE WHEN ?3='accepted' THEN 'fixture-proof' END)",
        params![id, episode, state, deadline]).unwrap();
}

fn event_count(db: &Connection) -> i64 {
    db.query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
        .unwrap()
}

static WRITER_BUSY_SIGNAL: std::sync::Mutex<Option<std::sync::mpsc::Sender<()>>> =
    std::sync::Mutex::new(None);

fn signal_writer_lock_wait(count: i32) -> bool {
    if count == 0
        && let Some(signal) = WRITER_BUSY_SIGNAL.lock().unwrap().take()
    {
        let _ = signal.send(());
    }
    std::thread::sleep(std::time::Duration::from_millis(1));
    count < 2000
}

#[test]
fn relaxed_commit_durability_is_a_per_context_test_hook() {
    let path = db_path();
    let synchronous = |db: &Connection| {
        db.query_row("PRAGMA synchronous", [], |r| r.get::<_, i64>(0))
            .unwrap()
    };
    // Unit-test stores start relaxed; both start from FULL here, so the
    // relax hook must actually move one, and only that one.
    let relaxed = StoreContext::new(path.clone(), Arc::new(FixedClock)).with_test_host();
    relaxed.require_commit_durability();
    let durable = StoreContext::new(path, Arc::new(FixedClock)).with_test_host();
    durable.require_commit_durability();
    assert_eq!(
        synchronous(&relaxed.open_writer().unwrap()),
        2,
        "FULL first"
    );
    relaxed.relax_commit_durability();
    assert_eq!(synchronous(&relaxed.open_writer().unwrap()), 1, "NORMAL");
    assert_eq!(synchronous(&durable.open_writer().unwrap()), 2, "FULL");
}

#[test]
fn fresh_database_has_durable_settings_constraints_and_read_only_queries() {
    let path = db_path();
    let store = StoreContext::new(path.clone(), Arc::new(FixedClock)).with_test_host();
    store.require_commit_durability();
    let db = store.open_writer().unwrap();
    assert_eq!(
        db.query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "wal"
    );
    assert_eq!(
        db.query_row("PRAGMA synchronous", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        db.query_row("PRAGMA foreign_keys", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        schema::NODE_SCHEMA_VERSION
    );
    assert!(db.execute("INSERT INTO seats(id, instance_id, state, role, generation, created_at,home_node,is_local,host_id) VALUES ('s', 'missing', 'resolved', 'native', 1, 0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))", []).is_err());
    db.execute("INSERT INTO seats(id, instance_id, state, role, target_id, generation, created_at,home_node,is_local,host_id) VALUES ('s1', '34750f98bd59fcfc946da45aaabe933b', 'resolved', 'native', 'pane', 1, 0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))", []).unwrap();
    assert!(db.execute("INSERT INTO seats(id, instance_id, state, role, target_id, generation, created_at,home_node,is_local,host_id) VALUES ('s2', '34750f98bd59fcfc946da45aaabe933b', 'resolved', 'native', 'pane', 1, 0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))", []).is_err());
    let query = store
        .open_query(CallBudget {
            deadline: MonoInstant(200),
            cancellation: Cancellation::default(),
        })
        .unwrap();
    assert!(
        query
            .execute("UPDATE local_node SET decision_seq=decision_seq+1", [])
            .is_err()
    );
    drop(query);
    drop(db);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
    let _ = std::fs::remove_file(path);
}

#[test]
fn amended_v1_has_provenance_and_hidden_publication_tables() {
    let db = StoreContext::new(db_path(), Arc::new(FixedClock))
        .with_test_host()
        .open_writer()
        .unwrap();
    for table in [
        "membership_intervals",
        "seat_availability",
        "send_preparations",
        "prepared_recipients",
        "prepared_unavailable_warnings",
        "send_manifests",
        "receipt_state",
        "warning_jobs",
        "warning_recipients",
        "warning_offer",
        "work_jobs",
    ] {
        let found: i64 = db
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [table],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(found, 1, "missing {table}");
    }
    for (table, column) in [
        ("local_node", "decision_seq"),
        ("local_node", "send_eligibility_revision"),
        ("threads", "membership_revision"),
        ("threads", "timeline_revision"),
        ("seats", "retired_seq"),
        ("seats", "unavailability_episode"),
        ("invitations", "warning_message_id"),
        ("invitations", "accepted_actor_seat_id"),
        ("receipts", "warning_message_id"),
    ] {
        let mut statement = db.prepare(&format!("PRAGMA table_info({table})")).unwrap();
        let found = statement
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .any(|v| v.unwrap() == column);
        assert!(found, "missing {table}.{column}");
    }
}

#[test]
fn writer_audit_rejects_missing_amended_v1_objects() {
    let db = StoreContext::new(db_path(), Arc::new(FixedClock))
        .with_test_host()
        .open_writer()
        .unwrap();
    db.execute_batch("DROP TABLE work_jobs").unwrap();
    assert_eq!(
        schema::verify_baseline(&db).unwrap_err().code,
        ErrorCode::IncompatibleSchema
    );
}

#[test]
fn decision_and_eligibility_revisions_commit_and_rollback_together() {
    let (_, mut db, _) = seeded_db(0);
    let tx = db.transaction().unwrap();
    assert_eq!(
        schema::next_decision_seq(&tx, "34750f98bd59fcfc946da45aaabe933b").unwrap(),
        1
    );
    assert!(
        schema::apply_eligibility_transition(&tx, "34750f98bd59fcfc946da45aaabe933b", |tx| {
            tx.execute("UPDATE seats SET generation=2 WHERE id='s'", [])
                .map_err(super::store_error)?;
            Ok(true)
        })
        .unwrap()
    );
    schema::bump_membership_revision(&tx, &ThreadId::new("t")).unwrap();
    schema::bump_timeline_revision(&tx, &ThreadId::new("t")).unwrap();
    tx.commit().unwrap();
    let revisions: (i64, i64, i64, i64) = db.query_row("SELECT h.decision_seq, h.send_eligibility_revision, t.membership_revision, t.timeline_revision FROM local_node h JOIN threads t ON t.instance_id=h.id WHERE h.id='34750f98bd59fcfc946da45aaabe933b'", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(revisions, (1, 1, 1, 1));
    let tx = db.transaction().unwrap();
    assert_eq!(
        schema::next_decision_seq(&tx, "34750f98bd59fcfc946da45aaabe933b").unwrap(),
        2
    );
    schema::apply_eligibility_transition(&tx, "34750f98bd59fcfc946da45aaabe933b", |tx| {
        tx.execute("UPDATE seats SET generation=3 WHERE id='s'", [])
            .map_err(super::store_error)?;
        Ok(true)
    })
    .unwrap();
    tx.rollback().unwrap();
    let revisions: (i64, i64) = db
        .query_row(
            "SELECT decision_seq, send_eligibility_revision FROM local_node WHERE id='34750f98bd59fcfc946da45aaabe933b'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(revisions, (1, 1));
    assert_eq!(
        db.query_row("SELECT generation FROM seats WHERE id='s'", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn epoch_loss_opens_one_episode_and_later_registration_allows_a_new_one() {
    let (_, mut db, _) = seeded_db(10);
    db.execute(
        "UPDATE seats SET target_id='p', target_generation=1 WHERE id='s'",
        [],
    )
    .unwrap();
    db.execute(
        "UPDATE herdr_hosts SET host_boot='b', host_epoch=1 WHERE node_id='34750f98bd59fcfc946da45aaabe933b'",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observed_at,provenance) VALUES ((SELECT min(host_id) FROM herdr_hosts),'p','b',1,1,1,'fresh')", []).unwrap();
    db.execute("INSERT INTO occupant_bindings(seat_id, generation, target_generation, target_id, host_boot, host_epoch, harness, native_session, execution_id, observation_provenance, observed_at, registered_at,terminal_id,incarnation) VALUES ('s',1,1,'p','b',1,'codex','n','e','verified',1,1,'term-'||'p','inc')", []).unwrap();
    db.execute("UPDATE seats SET unavailability_open=0 WHERE id='s'", [])
        .unwrap();
    let tx = db.transaction().unwrap();
    assert!(schema::ensure_unavailability_episode(&tx, &SeatId::new("s")).is_err());
    tx.rollback().unwrap();
    db.execute(
        "UPDATE herdr_hosts SET host_epoch=2 WHERE node_id='34750f98bd59fcfc946da45aaabe933b'",
        [],
    )
    .unwrap();
    let tx = db.transaction().unwrap();
    assert_eq!(
        schema::ensure_unavailability_episode(&tx, &SeatId::new("s")).unwrap(),
        2
    );
    assert_eq!(
        schema::ensure_unavailability_episode(&tx, &SeatId::new("s")).unwrap(),
        2
    );
    tx.commit().unwrap();
    db.execute("UPDATE seats SET unavailability_open=0 WHERE id='s'", [])
        .unwrap();
    db.execute(
        "UPDATE herdr_hosts SET host_epoch=3 WHERE node_id='34750f98bd59fcfc946da45aaabe933b'",
        [],
    )
    .unwrap();
    let tx = db.transaction().unwrap();
    assert_eq!(
        schema::ensure_unavailability_episode(&tx, &SeatId::new("s")).unwrap(),
        3
    );
    tx.commit().unwrap();
}

#[test]
fn target_generation_change_opens_episode_before_binding_reconciliation() {
    let (_, mut db, _) = seeded_db(10);
    db.execute_batch("UPDATE seats SET target_id='p', target_generation=1, unavailability_open=0 WHERE id='s'; UPDATE herdr_hosts SET host_boot='b', host_epoch=1 WHERE node_id='34750f98bd59fcfc946da45aaabe933b'; INSERT INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observed_at,provenance) VALUES ((SELECT min(host_id) FROM herdr_hosts),'p','b',1,1,1,'fresh'); INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES ('s',1,1,'p','b',1,'codex','n','e','verified',1,1,'term-'||'p','inc');").unwrap();
    let tx = db.transaction().unwrap();
    assert!(schema::ensure_unavailability_episode(&tx, &SeatId::new("s")).is_err());
    tx.rollback().unwrap();
    db.execute(
        "UPDATE observed_targets SET generation=2 WHERE target_id='p'",
        [],
    )
    .unwrap();
    let tx = db.transaction().unwrap();
    assert_eq!(
        schema::ensure_unavailability_episode(&tx, &SeatId::new("s")).unwrap(),
        2
    );
    tx.commit().unwrap();
}

#[test]
fn invalidated_observation_opens_episode_even_with_stale_fresh_target_row() {
    let (_, mut db, _) = seeded_db(10);
    db.execute_batch("UPDATE seats SET target_id='p', target_generation=1, unavailability_open=0 WHERE id='s'; UPDATE herdr_hosts SET host_boot='b', host_epoch=1 WHERE node_id='34750f98bd59fcfc946da45aaabe933b'; INSERT INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observed_at,provenance) VALUES ((SELECT min(host_id) FROM herdr_hosts),'p','b',1,1,1,'fresh'); INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES ('s',1,1,'p','b',1,'codex','n','e','verified',1,1,'term-'||'p','inc');").unwrap();
    db.execute(
        "UPDATE herdr_hosts SET invalidation_revision=1 WHERE node_id='34750f98bd59fcfc946da45aaabe933b'",
        [],
    )
    .unwrap();
    let tx = db.transaction().unwrap();
    assert_eq!(
        schema::ensure_unavailability_episode(&tx, &SeatId::new("s")).unwrap(),
        2
    );
    tx.commit().unwrap();
}

#[test]
fn decision_sample_controls_equality_and_late_warning_even_after_clock_moves() {
    let (context, mut db, clock) = seeded_db(299_999);
    add_invitation(&db, "v1", 300_000, "pending");
    let obligation = ObligationRef::Invitation(InvitationId::new("v1"));
    context
        .execute_decision(
            &mut db,
            |_| Ok(()),
            |tx, at, _| {
                clock.set(301_000);
                assert_eq!(at.utc, UtcMillis(299_999));
                assert!(
                    !record_overdue_if_pending(tx, &obligation, &TimeBasis::Decision, at.utc)?
                        .inserted
                );
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(event_count(&db), 0);
    clock.set(299_999);
    context
        .execute_decision(
            &mut db,
            |_| {
                clock.set(300_000);
                Ok(())
            },
            |tx, at, _| {
                assert_eq!(at.utc, UtcMillis(300_000));
                assert!(
                    record_overdue_if_pending(tx, &obligation, &TimeBasis::Decision, at.utc)?
                        .inserted
                );
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(event_count(&db), 1);
    context
        .execute_decision(
            &mut db,
            |_| Ok(()),
            |tx, at, _| {
                assert!(
                    !record_overdue_if_pending(tx, &obligation, &TimeBasis::Decision, at.utc)?
                        .inserted
                );
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(event_count(&db), 1);
}

#[test]
fn overdue_source_marker_event_and_job_commit_or_rollback_as_one() {
    let (context, mut db, _) = seeded_db(300_000);
    add_invitation(&db, "v1", 300_000, "pending");
    let obligation = ObligationRef::Invitation(InvitationId::new("v1"));
    let failed: Result<(), _> = context.execute_decision(
        &mut db,
        |_| Ok(()),
        |tx, at, _| {
            let seq = schema::next_decision_seq(tx, "34750f98bd59fcfc946da45aaabe933b")?;
            let outcome = schema::record_overdue_with_decision_seq(
                tx,
                &obligation,
                &TimeBasis::Decision,
                at.utc,
                seq,
            )?;
            assert!(outcome.inserted);
            Err(super::api_error(ErrorCode::Conflict, "rollback"))
        },
    );
    assert!(failed.is_err());
    let empty: (Option<String>, i64, i64, i64) = db.query_row(
        "SELECT warning_message_id, (SELECT count(*) FROM messages), (SELECT count(*) FROM warning_jobs), (SELECT decision_seq FROM local_node WHERE id='34750f98bd59fcfc946da45aaabe933b') FROM invitations WHERE id='v1'",
        [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    ).unwrap();
    assert_eq!(empty, (None, 0, 0, 0));
    let first = context
        .execute_decision(
            &mut db,
            |_| Ok(()),
            |tx, at, _| {
                let seq = schema::next_decision_seq(tx, "34750f98bd59fcfc946da45aaabe933b")?;
                schema::record_overdue_with_decision_seq(
                    tx,
                    &obligation,
                    &TimeBasis::Decision,
                    at.utc,
                    seq,
                )
            },
        )
        .unwrap();
    assert!(first.inserted);
    let repeated = context
        .execute_decision(
            &mut db,
            |_| Ok(()),
            |tx, at, _| {
                schema::record_overdue_if_pending(tx, &obligation, &TimeBasis::Decision, at.utc)
            },
        )
        .unwrap();
    assert_eq!(repeated.warning, first.warning);
    assert!(!repeated.inserted);
    let committed: (String, i64, i64, i64) = db.query_row(
        "SELECT warning_message_id, (SELECT count(*) FROM messages), (SELECT count(*) FROM warning_jobs), (SELECT count(*) FROM work_jobs) FROM invitations WHERE id='v1'",
        [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    ).unwrap();
    assert_eq!(
        committed,
        (first.warning.unwrap().as_str().to_owned(), 1, 1, 1)
    );
}

#[test]
fn failed_decision_rolls_back_warning_and_wake_together() {
    let (context, mut db, _) = seeded_db(300_000);
    add_invitation(&db, "v1", 300_000, "pending");
    let obligation = ObligationRef::Invitation(InvitationId::new("v1"));
    let result: Result<(), _> = context.execute_decision(
        &mut db,
        |_| Ok(()),
        |tx, at, _| {
            assert!(
                record_overdue_if_pending(tx, &obligation, &TimeBasis::Decision, at.utc)?.inserted
            );
            Err(super::api_error(ErrorCode::Conflict, "forced failure"))
        },
    );
    assert!(result.is_err());
    assert_eq!(event_count(&db), 0);
    assert_eq!(
        db.query_row("SELECT count(*) FROM wake_work", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn operation_replay_does_not_resample_or_run_apply() {
    let (context, mut db, clock) = seeded_db(5);
    let first = schema::execute_idempotent_transaction(
        &context,
        &mut db,
        "seat:s",
        "op",
        [7; 32],
        |_| Ok(()),
        |_, _| Ok(CommandResult::Left(ThreadId::new("t"))),
    )
    .unwrap();
    assert_eq!(clock.samples.load(Ordering::SeqCst), 1);
    let replay = schema::execute_idempotent_transaction(
        &context,
        &mut db,
        "seat:s",
        "op",
        [7; 32],
        |_| panic!("replay validated"),
        |_, _| panic!("replay applied"),
    )
    .unwrap();
    assert_eq!(first, replay);
    assert_eq!(clock.samples.load(Ordering::SeqCst), 1);
    let mismatch = schema::execute_idempotent_transaction(
        &context,
        &mut db,
        "seat:s",
        "op",
        [8; 32],
        |_| Ok(()),
        |_, _| panic!("mismatch applied"),
    );
    assert_eq!(
        mismatch.unwrap_err().code,
        ErrorCode::OperationPayloadMismatch
    );
}

#[test]
fn retirement_uses_fenced_cutover_and_never_enqueues_wake() {
    let (context, mut db, _) = seeded_db(900_000);
    add_invitation(&db, "v1", 300_000, "pending");
    add_invitation(&db, "v2", 300_001, "pending");
    add_invitation(&db, "v3", 100, "accepted");
    db.execute(
        "UPDATE seats SET state='retired', retired_at=300000 WHERE id='s'",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO retirements(id, seat_id, cutover_at, closure_boot, closure_epoch, closure_target, closure_generation) VALUES ('j', 's', 300000, 'b', 1, 'p', 1)", []).unwrap();
    let job = TimeBasis::Retirement(RetirementJobId::new("j"));
    context
        .execute_decision(
            &mut db,
            |_| Ok(()),
            |tx, at, _| {
                assert!(
                    record_overdue_if_pending(
                        tx,
                        &ObligationRef::Invitation(InvitationId::new("v1")),
                        &job,
                        at.utc
                    )?
                    .inserted
                );
                assert!(
                    !record_overdue_if_pending(
                        tx,
                        &ObligationRef::Invitation(InvitationId::new("v2")),
                        &job,
                        at.utc
                    )?
                    .inserted
                );
                assert!(
                    !record_overdue_if_pending(
                        tx,
                        &ObligationRef::Invitation(InvitationId::new("v3")),
                        &job,
                        at.utc
                    )?
                    .inserted
                );
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(event_count(&db), 1);
    assert_eq!(
        db.query_row("SELECT count(*) FROM wake_work", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    let wrong = context.execute_decision(
        &mut db,
        |_| Ok(()),
        |tx, at, _| {
            record_overdue_if_pending(
                tx,
                &ObligationRef::Invitation(InvitationId::new("v1")),
                &TimeBasis::Retirement(RetirementJobId::new("absent")),
                at.utc,
            )
        },
    );
    assert!(wrong.is_err());
    assert_eq!(event_count(&db), 1);
}

#[test]
fn receipt_cutover_skips_unstarted_future_and_terminal_rows() {
    let (context, mut db, _) = seeded_db(900_000);
    for (id, sequence, deadline, state) in [
        ("m1", 1, Some(300_000), "pending"),
        ("m2", 2, Some(300_001), "pending"),
        ("m3", 3, None, "pending"),
        ("m4", 4, Some(100), "acked"),
    ] {
        db.execute("INSERT INTO messages(instance_id,id, thread_id, sequence, kind, body, decision_at,decision_seq) VALUES ('34750f98bd59fcfc946da45aaabe933b',?1, 't', ?2, 'ordinary', 'body', 0,?2)", params![id, sequence]).unwrap();
        db.execute("INSERT INTO receipts(message_id, thread_id, seat_id, state, frozen_duration_ms, available_at, deadline_at) VALUES (?1, 't', 's', ?2, 300000, ?3, ?4)",
            params![id, state, deadline.map(|_| 0), deadline]).unwrap();
    }
    db.execute("UPDATE threads SET next_sequence=5 WHERE id='t'", [])
        .unwrap();
    db.execute(
        "UPDATE seats SET state='retired', retired_at=300000 WHERE id='s'",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO retirements(id, seat_id, cutover_at, closure_boot, closure_epoch, closure_target, closure_generation) VALUES ('j', 's', 300000, 'b', 1, 'p', 1)", []).unwrap();
    let basis = TimeBasis::Retirement(RetirementJobId::new("j"));
    context
        .execute_decision(
            &mut db,
            |_| Ok(()),
            |tx, at, _| {
                for (id, expected) in [("m1", true), ("m2", false), ("m3", false), ("m4", false)] {
                    let result = record_overdue_if_pending(
                        tx,
                        &ObligationRef::Receipt {
                            message: MessageId::new(id),
                            seat: SeatId::new("s"),
                        },
                        &basis,
                        at.utc,
                    )?;
                    assert_eq!(result.inserted, expected);
                }
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(event_count(&db), 5);
    assert_eq!(
        db.query_row("SELECT count(*) FROM wake_work", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    let payload: String = db
        .query_row(
            "SELECT event_json FROM messages WHERE source_message_id='m1' AND kind='warn'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(value["classified_at"], 300_000);
    assert_eq!(value["materialized_at"], 900_000);
    assert_eq!(value["current_status"], "recipient_retired");
    assert_eq!(value["physical_status"], "pending");
}

#[test]
fn retirement_cursor_and_warning_roll_back_as_one_quantum() {
    let (context, mut db, _) = seeded_db(900_000);
    add_invitation(&db, "v1", 300_000, "pending");
    db.execute(
        "UPDATE seats SET state='retired', retired_at=300000 WHERE id='s'",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO retirements(id, seat_id, cutover_at, closure_boot, closure_epoch, closure_target, closure_generation) VALUES ('j', 's', 300000, 'b', 1, 'p', 1)", []).unwrap();
    let failed: Result<(), _> = context.execute_decision(
        &mut db,
        |_| Ok(()),
        |tx, at, _| {
            record_overdue_if_pending(
                tx,
                &ObligationRef::Invitation(InvitationId::new("v1")),
                &TimeBasis::Retirement(RetirementJobId::new("j")),
                at.utc,
            )?;
            tx.execute(
                "UPDATE retirements SET obligation_ordinal=1, processed_units=1 WHERE id='j'",
                [],
            )
            .unwrap();
            Err(super::api_error(
                ErrorCode::Conflict,
                "forced quantum failure",
            ))
        },
    );
    assert!(failed.is_err());
    assert_eq!(event_count(&db), 0);
    assert_eq!(
        db.query_row(
            "SELECT processed_units FROM retirements WHERE id='j'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn unknown_version_and_non_database_files_are_preserved() {
    let path = db_path();
    let store = StoreContext::new(path.clone(), Arc::new(FixedClock)).with_test_host();
    let db = store.open_writer().unwrap();
    db.pragma_update(None, "user_version", 99).unwrap();
    drop(db);
    assert_eq!(
        store.open_writer().unwrap_err().code,
        ErrorCode::IncompatibleSchema
    );
    let raw = Connection::open(&path).unwrap();
    assert_eq!(
        raw.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        99
    );
    drop(raw);
    let broken_path = db_path();
    let bytes = b"this is not sqlite";
    std::fs::write(&broken_path, bytes).unwrap();
    let broken = StoreContext::new(broken_path.clone(), Arc::new(FixedClock)).with_test_host();
    assert_eq!(
        broken.open_writer().unwrap_err().code,
        ErrorCode::StoreCorrupt
    );
    assert_eq!(std::fs::read(&broken_path).unwrap(), bytes);
}

#[test]
fn query_cancellation_interrupts_only_its_connection() {
    let (context, db, _) = seeded_db(100);
    let cancellation = Cancellation::default();
    let query = context
        .open_query(CallBudget {
            deadline: MonoInstant(500),
            cancellation: cancellation.clone(),
        })
        .unwrap();
    cancellation.cancel();
    let error = query.query_row("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<100000) SELECT sum(x) FROM n", [], |r| r.get::<_, i64>(0)).unwrap_err();
    assert!(matches!(error, rusqlite::Error::SqliteFailure(_, _)));
    db.execute("INSERT INTO seats(id, instance_id, state, role, generation, created_at,home_node,is_local,host_id) VALUES ('other', '34750f98bd59fcfc946da45aaabe933b', 'resolved', 'native', 1, 0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))", []).unwrap();
}

#[test]
fn checked_deadline_rejects_zero_and_overflow() {
    assert_eq!(
        schema::checked_deadline(UtcMillis(10), 5).unwrap(),
        UtcMillis(15)
    );
    assert_eq!(
        schema::checked_deadline(UtcMillis(10), 0).unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    assert_eq!(
        schema::checked_deadline(UtcMillis(i64::MAX), 1)
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
}

#[test]
fn canonical_digest_ignores_object_insertion_order() {
    struct OrderedPairs([(&'static str, i32); 2]);
    impl serde::Serialize for OrderedPairs {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            use serde::ser::SerializeMap;
            let mut map = serializer.serialize_map(Some(2))?;
            for (key, value) in self.0 {
                map.serialize_entry(key, &value)?;
            }
            map.end()
        }
    }
    let a = OrderedPairs([("alpha", 1), ("beta", 2)]);
    let b = OrderedPairs([("beta", 2), ("alpha", 1)]);
    assert_eq!(
        schema::canonical_digest(&a).unwrap(),
        schema::canonical_digest(&b).unwrap()
    );
}

#[test]
fn duplicate_event_key_cannot_suppress_another_threads_event() {
    let (context, mut db, _) = seeded_db(50);
    db.execute("INSERT INTO threads(id, instance_id, topic, goal, created_at, updated_at) VALUES ('t2', '34750f98bd59fcfc946da45aaabe933b', 'topic', 'purpose', 0, 0)", []).unwrap();
    let t1 = ThreadId::new("t");
    let t2 = ThreadId::new("t2");
    context
        .execute_decision(
            &mut db,
            |_| Ok(()),
            |tx, at, _| {
                schema::append_event_once(
                    tx,
                    schema::EventInput {
                        thread: &t1,
                        key: "same",
                        kind: "info",
                        payload_json: "{}",
                        decision_at: at.utc,
                        source_message: None,
                        source_invitation: None,
                    },
                )?;
                Ok(())
            },
        )
        .unwrap();
    let result = context.execute_decision(
        &mut db,
        |_| Ok(()),
        |tx, at, _| {
            schema::append_event_once(
                tx,
                schema::EventInput {
                    thread: &t2,
                    key: "same",
                    kind: "info",
                    payload_json: "{}",
                    decision_at: at.utc,
                    source_message: None,
                    source_invitation: None,
                },
            )
        },
    );
    assert_eq!(result.unwrap_err().code, ErrorCode::Conflict);
    assert_eq!(event_count(&db), 1);
}

#[test]
fn one_decision_batch_allocates_distinct_global_event_offsets() {
    let (_, mut db, _) = seeded_db(50);
    db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t2','34750f98bd59fcfc946da45aaabe933b','topic','goal',0,0)", []).unwrap();
    let tx = db.transaction().unwrap();
    let seq = schema::next_decision_seq(&tx, "34750f98bd59fcfc946da45aaabe933b").unwrap();
    for (thread, key) in [("t", "first"), ("t2", "second")] {
        schema::append_event_once_with_decision_seq(
            &tx,
            schema::EventInput {
                thread: &ThreadId::new(thread),
                key,
                kind: "warn",
                payload_json: "{}",
                decision_at: UtcMillis(50),
                source_message: None,
                source_invitation: None,
            },
            seq,
        )
        .unwrap();
    }
    tx.commit().unwrap();
    let offsets:Vec<i64>=db.prepare("SELECT event_offset FROM messages WHERE instance_id='34750f98bd59fcfc946da45aaabe933b' AND decision_seq=?1 ORDER BY event_offset")
        .unwrap().query_map([seq as i64],|r|r.get(0)).unwrap().map(Result::unwrap).collect();
    assert_eq!(offsets, vec![0, 1]);
}

#[test]
fn logical_message_keys_are_instance_scoped_and_thread_bound() {
    let (_, db, _) = seeded_db(0);
    db.execute("PRAGMA foreign_keys=OFF", []).unwrap();
    db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('other-thread','other','topic','goal',0,0)", []).unwrap();
    db.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('34750f98bd59fcfc946da45aaabe933b','m1','t',1,'ordinary','body',0,1)", []).unwrap();
    db.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('other','m2','other-thread',1,'ordinary','body',0,1)", []).unwrap();
    assert!(db.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('34750f98bd59fcfc946da45aaabe933b','bad','other-thread',2,'ordinary','body',0,2)", []).is_err());
    assert!(db.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('34750f98bd59fcfc946da45aaabe933b','collision','t',2,'ordinary','body',0,1)", []).is_err());
    assert!(db.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at) VALUES ('34750f98bd59fcfc946da45aaabe933b','null-key','t',2,'ordinary','body',0)", []).is_err());
}

#[test]
fn inserting_event_advances_timeline_revision_once() {
    let (context, mut db, _) = seeded_db(50);
    let thread = ThreadId::new("t");
    for _ in 0..2 {
        context
            .execute_decision(
                &mut db,
                |_| Ok(()),
                |tx, at, _| {
                    schema::append_event_once(
                        tx,
                        schema::EventInput {
                            thread: &thread,
                            key: "stable-event",
                            kind: "info",
                            payload_json: "{}",
                            decision_at: at.utc,
                            source_message: None,
                            source_invitation: None,
                        },
                    )
                },
            )
            .unwrap();
    }
    let revision: i64 = db
        .query_row(
            "SELECT timeline_revision FROM threads WHERE id='t'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(revision, 1);
}

#[test]
fn creation_ordinals_and_retirement_cutover_are_immutable() {
    let (_, db, _) = seeded_db(50);
    assert!(
        db.execute("UPDATE seats SET ordinal=999 WHERE id='s'", [])
            .is_err()
    );
    assert!(
        db.execute("UPDATE threads SET ordinal=999 WHERE id='t'", [])
            .is_err()
    );
    db.execute(
        "UPDATE seats SET state='retired', retired_at=50 WHERE id='s'",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO retirements(id, seat_id, cutover_at, closure_boot, closure_epoch, closure_target, closure_generation) VALUES ('j', 's', 50, 'b', 1, 'p', 1)", []).unwrap();
    assert!(
        db.execute("UPDATE retirements SET cutover_at=51 WHERE id='j'", [])
            .is_err()
    );
    assert!(
        db.execute("UPDATE retirements SET closure_target='q' WHERE id='j'", [])
            .is_err()
    );
}

#[test]
fn message_contents_and_history_are_immutable() {
    let (_, db, _) = seeded_db(50);
    db.execute("INSERT INTO messages(instance_id,id, thread_id, sequence, kind, body, decision_at,decision_seq) VALUES ('34750f98bd59fcfc946da45aaabe933b','m', 't', 1, 'ordinary', 'original', 10,1)", []).unwrap();
    db.execute("INSERT INTO messages(instance_id,id, thread_id, sequence, kind, event_key, event_json, decision_at,decision_seq) VALUES ('34750f98bd59fcfc946da45aaabe933b','e', 't', 2, 'info', 'key', '{}', 10,2)", []).unwrap();
    for sql in [
        "UPDATE messages SET body='rewritten' WHERE id='m'",
        "UPDATE messages SET decision_at=11 WHERE id='m'",
        "UPDATE messages SET kind='info', body=NULL, event_json='{}' WHERE id='m'",
        "UPDATE messages SET actor_label='forged' WHERE id='m'",
        r#"UPDATE messages SET event_json='{"changed":true}' WHERE id='e'"#,
        "UPDATE messages SET event_key='replacement' WHERE id='e'",
    ] {
        assert!(db.execute(sql, []).is_err(), "{sql}");
    }
    assert!(db.execute("DELETE FROM messages WHERE id='m'", []).is_err());
    let original: (String, i64) = db
        .query_row(
            "SELECT body, decision_at FROM messages WHERE id='m'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(original, ("original".to_owned(), 10));
}

#[test]
fn receipt_thread_must_match_its_message_thread() {
    let (_, db, _) = seeded_db(50);
    db.execute("INSERT INTO threads(id, instance_id, topic, goal, created_at, updated_at) VALUES ('other', '34750f98bd59fcfc946da45aaabe933b', 'topic', 'purpose', 0, 0)", []).unwrap();
    db.execute("INSERT INTO messages(instance_id,id, thread_id, sequence, kind, body, decision_at,decision_seq) VALUES ('34750f98bd59fcfc946da45aaabe933b','m', 't', 1, 'ordinary', 'body', 0,1)", []).unwrap();
    assert!(db.execute("INSERT INTO receipts(message_id, thread_id, seat_id, state, frozen_duration_ms) VALUES ('m', 'other', 's', 'pending', 300000)", []).is_err());
    db.execute("INSERT INTO receipts(message_id, thread_id, seat_id, state, frozen_duration_ms) VALUES ('m', 't', 's', 'pending', 300000)", []).unwrap();
    assert!(
        db.execute(
            "UPDATE receipts SET thread_id='other' WHERE message_id='m' AND seat_id='s'",
            []
        )
        .is_err()
    );
}

#[test]
fn query_lock_wait_uses_budget_remaining_after_connection_open() {
    let (context, writer, clock) = seeded_db(50);
    // Rollback journaling supplies a read-lock conflict for this callback test;
    // the separate factory settings test verifies production WAL mode.
    writer
        .pragma_update(None, "journal_mode", "DELETE")
        .unwrap();
    clock.mono.store(100, Ordering::SeqCst);
    let query = context
        .open_query(CallBudget {
            deadline: MonoInstant(200),
            cancellation: Cancellation::default(),
        })
        .unwrap();
    clock.mono.store(199, Ordering::SeqCst);
    let (busy_tx, busy_rx) = std::sync::mpsc::channel();
    *query._progress.busy_signal.lock().unwrap() = Some(busy_tx);
    writer.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let worker = std::thread::spawn(move || {
        let error = query
            .query_row("SELECT count(*) FROM threads", [], |r| r.get::<_, i64>(0))
            .unwrap_err();
        query.map_error(error).code
    });
    busy_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("query reached SQLite busy handler");
    clock.mono.store(200, Ordering::SeqCst);
    assert_eq!(worker.join().unwrap(), ErrorCode::DeadlineExceeded);
    writer.execute_batch("COMMIT").unwrap();
}

#[test]
fn query_lock_wait_stops_when_cancelled_after_connection_open() {
    let (context, writer, clock) = seeded_db(50);
    writer
        .pragma_update(None, "journal_mode", "DELETE")
        .unwrap();
    clock.mono.store(100, Ordering::SeqCst);
    let cancellation = Cancellation::default();
    let query = context
        .open_query(CallBudget {
            deadline: MonoInstant(200),
            cancellation: cancellation.clone(),
        })
        .unwrap();
    let (busy_tx, busy_rx) = std::sync::mpsc::channel();
    *query._progress.busy_signal.lock().unwrap() = Some(busy_tx);
    writer.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let worker = std::thread::spawn(move || {
        let error = query
            .query_row("SELECT count(*) FROM threads", [], |r| r.get::<_, i64>(0))
            .unwrap_err();
        query.map_error(error).code
    });
    busy_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("query reached SQLite busy handler");
    cancellation.cancel();
    assert_eq!(worker.join().unwrap(), ErrorCode::Cancelled);
    writer.execute_batch("COMMIT").unwrap();
}

#[test]
fn already_cancelled_query_setup_returns_cancelled() {
    let (context, _, clock) = seeded_db(50);
    clock.mono.store(100, Ordering::SeqCst);
    let cancellation = Cancellation::default();
    cancellation.cancel();
    let result = context.open_query(CallBudget {
        deadline: MonoInstant(200),
        cancellation,
    });
    let error = match result {
        Ok(_) => panic!("cancelled query opened"),
        Err(error) => error,
    };
    assert_eq!(error.code, ErrorCode::Cancelled);
}

#[test]
fn cancelled_during_query_schema_setup_reports_cancelled() {
    let (context, writer, clock) = seeded_db(50);
    writer
        .pragma_update(None, "journal_mode", "DELETE")
        .unwrap();
    clock.mono.store(100, Ordering::SeqCst);
    let cancellation = Cancellation::default();
    let (busy_tx, busy_rx) = std::sync::mpsc::channel();
    *context.setup_busy_signal.lock().unwrap() = Some(busy_tx);
    writer.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let cancel_worker = cancellation.clone();
    let worker = std::thread::spawn(move || {
        match context.open_query(CallBudget {
            deadline: MonoInstant(200),
            cancellation: cancel_worker,
        }) {
            Ok(_) => panic!("locked query setup succeeded"),
            Err(error) => error.code,
        }
    });
    busy_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("setup reached SQLite busy handler");
    cancellation.cancel();
    assert_eq!(worker.join().unwrap(), ErrorCode::Cancelled);
    writer.execute_batch("COMMIT").unwrap();
}

#[test]
fn deadline_during_query_schema_setup_reports_deadline_exceeded() {
    let (context, writer, clock) = seeded_db(50);
    writer
        .pragma_update(None, "journal_mode", "DELETE")
        .unwrap();
    clock.mono.store(100, Ordering::SeqCst);
    let (busy_tx, busy_rx) = std::sync::mpsc::channel();
    *context.setup_busy_signal.lock().unwrap() = Some(busy_tx);
    writer.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let worker = std::thread::spawn(move || {
        match context.open_query(CallBudget {
            deadline: MonoInstant(200),
            cancellation: Cancellation::default(),
        }) {
            Ok(_) => panic!("locked query setup succeeded"),
            Err(error) => error.code,
        }
    });
    busy_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("setup reached SQLite busy handler");
    clock.mono.store(200, Ordering::SeqCst);
    assert_eq!(worker.join().unwrap(), ErrorCode::DeadlineExceeded);
    writer.execute_batch("COMMIT").unwrap();
}

#[test]
fn idle_query_connection_is_reused_under_the_new_calls_budget() {
    let (context, writer, clock) = seeded_db(50);
    writer
        .pragma_update(None, "journal_mode", "DELETE")
        .unwrap();
    clock.mono.store(100, Ordering::SeqCst);
    let budget = |cancellation: &Cancellation| CallBudget {
        deadline: MonoInstant(200),
        cancellation: cancellation.clone(),
    };
    let first = context
        .open_query(budget(&Cancellation::default()))
        .unwrap();
    let handle = unsafe { first.handle() };
    drop(first);
    assert_eq!(context.idle_queries.lock().unwrap().len(), 1);
    let cancellation = Cancellation::default();
    let query = context.open_query(budget(&cancellation)).unwrap();
    assert_eq!(
        unsafe { query.handle() },
        handle,
        "the idle connection is reused"
    );
    assert!(context.idle_queries.lock().unwrap().is_empty());
    assert!(
        query.execute("UPDATE threads SET topic='x'", []).is_err(),
        "a reused connection stays query-only"
    );
    // The busy handler belongs to this call: cancelling it stops a lock wait.
    let (busy_tx, busy_rx) = std::sync::mpsc::channel();
    *query._progress.busy_signal.lock().unwrap() = Some(busy_tx);
    writer.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let worker = std::thread::spawn(move || {
        let error = query
            .query_row("SELECT count(*) FROM threads", [], |r| r.get::<_, i64>(0))
            .unwrap_err();
        query.map_error(error).code
    });
    busy_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("query reached SQLite busy handler");
    cancellation.cancel();
    assert_eq!(worker.join().unwrap(), ErrorCode::Cancelled);
    writer.execute_batch("COMMIT").unwrap();
}

#[test]
fn query_connection_returns_idle_only_outside_a_transaction_and_under_the_cap() {
    let (context, _writer, clock) = seeded_db(50);
    clock.mono.store(100, Ordering::SeqCst);
    let open = || {
        context
            .open_query(CallBudget {
                deadline: MonoInstant(200),
                cancellation: Cancellation::default(),
            })
            .unwrap()
    };
    let in_transaction = open();
    in_transaction.execute_batch("BEGIN").unwrap();
    in_transaction
        .query_row("SELECT count(*) FROM threads", [], |r| r.get::<_, i64>(0))
        .unwrap();
    drop(in_transaction);
    assert!(
        context.idle_queries.lock().unwrap().is_empty(),
        "a connection left inside a transaction is closed, not reused"
    );
    let held: Vec<_> = (0..IDLE_QUERY_CONNECTIONS + 2).map(|_| open()).collect();
    drop(held);
    assert_eq!(
        context.idle_queries.lock().unwrap().len(),
        IDLE_QUERY_CONNECTIONS
    );
}

#[test]
fn query_setup_preserves_schema_and_corruption_errors() {
    let (context, writer, clock) = seeded_db(50);
    clock.mono.store(100, Ordering::SeqCst);
    writer.pragma_update(None, "user_version", 99).unwrap();
    let result = context.open_query(CallBudget {
        deadline: MonoInstant(200),
        cancellation: Cancellation::default(),
    });
    let error = match result {
        Ok(_) => panic!("unknown schema accepted"),
        Err(error) => error,
    };
    assert_eq!(error.code, ErrorCode::IncompatibleSchema);

    let broken_path = db_path();
    std::fs::write(&broken_path, b"not a sqlite database").unwrap();
    let broken = StoreContext::new(broken_path, clock).with_test_host();
    let result = broken.open_query(CallBudget {
        deadline: MonoInstant(200),
        cancellation: Cancellation::default(),
    });
    let error = match result {
        Ok(_) => panic!("corrupt database accepted"),
        Err(error) => error,
    };
    assert_eq!(error.code, ErrorCode::StoreCorrupt);
}

#[test]
fn writer_lock_wait_precedes_decision_sample() {
    let (context, mut writer, clock) = seeded_db(299_999);
    writer.busy_handler(Some(signal_writer_lock_wait)).unwrap();
    let lock = Connection::open(&context.path).unwrap();
    lock.execute_batch("BEGIN IMMEDIATE").unwrap();
    let (busy_tx, busy_rx) = std::sync::mpsc::channel();
    *WRITER_BUSY_SIGNAL.lock().unwrap() = Some(busy_tx);
    let worker = std::thread::spawn(move || {
        context
            .execute_decision(&mut writer, |_| Ok(()), |_, at, _| Ok(at.utc))
            .unwrap()
    });
    if busy_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .is_err()
    {
        lock.execute_batch("COMMIT").unwrap();
        panic!("worker never reached SQLite's BEGIN IMMEDIATE lock wait");
    }
    clock.set(300_000);
    lock.execute_batch("COMMIT").unwrap();
    assert_eq!(worker.join().unwrap(), UtcMillis(300_000));
}

#[test]
fn missing_v1_index_is_rejected_without_replacement() {
    let path = db_path();
    let store = StoreContext::new(path.clone(), Arc::new(FixedClock)).with_test_host();
    let db = store.open_writer().unwrap();
    db.execute_batch("DROP INDEX receipts_seat_thread_state_ordinal")
        .unwrap();
    drop(db);
    assert_eq!(
        store.open_writer().unwrap_err().code,
        ErrorCode::IncompatibleSchema
    );
    let raw = Connection::open(path).unwrap();
    let exists: i64 = raw
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='receipts_seat_thread_state_ordinal'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(exists, 0);
}

#[test]
fn recovery_baseline_survives_new_target_observations_without_allocating_a_seat() {
    let (_, db, _) = seeded_db(50);
    db.execute("INSERT INTO recovery_baseline_targets(host_id, target_id, baseline_boot, baseline_epoch, captured_at, disposition) VALUES ((SELECT min(host_id) FROM herdr_hosts), 'pane', 'old', 1, 10, 'held_for_repair')", []).unwrap();
    db.execute("INSERT INTO observed_targets(host_id, target_id, host_boot, epoch, generation, observed_at, provenance) VALUES ((SELECT min(host_id) FROM herdr_hosts), 'pane', 'old', 1, 1, 10, 'enumeration')", []).unwrap();
    db.execute("UPDATE observed_targets SET host_boot='new', epoch=2, generation=2, observed_at=20 WHERE target_id='pane'", []).unwrap();
    let baseline: (String, i64) = db.query_row("SELECT baseline_boot, baseline_epoch FROM recovery_baseline_targets WHERE target_id='pane'", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert_eq!(baseline, ("old".to_owned(), 1));
    assert_eq!(
        db.query_row("SELECT count(*) FROM seats", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn receipt_warning_keys_distinguish_ids_containing_colons() {
    let (context, mut db, _) = seeded_db(300_000);
    db.execute("INSERT INTO seats(id, instance_id, state, role, generation, created_at,home_node,is_local,host_id) VALUES ('b:c', '34750f98bd59fcfc946da45aaabe933b', 'resolved', 'native', 1, 0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))", []).unwrap();
    for (id, sequence, seat) in [("a:b", 1, "c"), ("a", 2, "b:c")] {
        if seat == "c" {
            db.execute("INSERT INTO seats(id, instance_id, state, role, generation, created_at,home_node,is_local,host_id) VALUES ('c', '34750f98bd59fcfc946da45aaabe933b', 'resolved', 'native', 1, 0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))", []).unwrap();
        }
        db.execute("INSERT INTO messages(instance_id,id, thread_id, sequence, kind, body, decision_at,decision_seq) VALUES ('34750f98bd59fcfc946da45aaabe933b',?1, 't', ?2, 'ordinary', 'body', 0,?2)", params![id, sequence]).unwrap();
        db.execute("INSERT INTO receipts(message_id, thread_id, seat_id, state, frozen_duration_ms, available_at, deadline_at) VALUES (?1, 't', ?2, 'pending', 300000, 0, 300000)", params![id, seat]).unwrap();
    }
    db.execute("UPDATE threads SET next_sequence=3 WHERE id='t'", [])
        .unwrap();
    context
        .execute_decision(
            &mut db,
            |_| Ok(()),
            |tx, at, _| {
                for (message, seat) in [("a:b", "c"), ("a", "b:c")] {
                    assert!(
                        record_overdue_if_pending(
                            tx,
                            &ObligationRef::Receipt {
                                message: MessageId::new(message),
                                seat: SeatId::new(seat)
                            },
                            &TimeBasis::Decision,
                            at.utc
                        )?
                        .inserted
                    );
                }
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(event_count(&db), 4);
}

#[test]
fn receipt_warning_condition_is_quiet_for_second_pending_source() {
    let (context, mut db, _) = seeded_db(300_000);
    for (id, sequence) in [("m1", 1), ("m2", 2)] {
        db.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('34750f98bd59fcfc946da45aaabe933b',?1,'t',?2,'ordinary','body',0,?2)", params![id,sequence]).unwrap();
        db.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) VALUES (?1,'t','s','pending',300000,0,300000)", [id]).unwrap();
    }
    db.execute("UPDATE threads SET next_sequence=3 WHERE id='t'", [])
        .unwrap();
    let warn = |tx: &rusqlite::Transaction<'_>, id: &str, at: UtcMillis| {
        record_overdue_if_pending(
            tx,
            &ObligationRef::Receipt {
                message: MessageId::new(id),
                seat: SeatId::new("s"),
            },
            &TimeBasis::Decision,
            at,
        )
    };
    context
        .execute_decision(
            &mut db,
            |_| Ok(()),
            |tx, at, _| {
                assert!(warn(tx, "m1", at.utc)?.inserted);
                assert!(!warn(tx, "m2", at.utc)?.inserted);
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM warning_jobs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn receipt_warning_condition_survives_writer_reopen() {
    let (context, mut db, _) = seeded_db(300_000);
    for (id, sequence) in [("m1", 1), ("m2", 2)] {
        db.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('34750f98bd59fcfc946da45aaabe933b',?1,'t',?2,'ordinary','body',0,?2)", params![id,sequence]).unwrap();
        db.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) VALUES (?1,'t','s','pending',300000,0,300000)", [id]).unwrap();
    }
    db.execute("UPDATE threads SET next_sequence=3 WHERE id='t'", [])
        .unwrap();
    context
        .execute_decision(
            &mut db,
            |_| Ok(()),
            |tx, at, _| {
                assert!(
                    record_overdue_if_pending(
                        tx,
                        &ObligationRef::Receipt {
                            message: MessageId::new("m1"),
                            seat: SeatId::new("s")
                        },
                        &TimeBasis::Decision,
                        at.utc
                    )?
                    .inserted
                );
                Ok(())
            },
        )
        .unwrap();
    drop(db);
    let mut db = context.open_writer().unwrap();
    context
        .execute_decision(
            &mut db,
            |_| Ok(()),
            |tx, at, _| {
                assert!(
                    !record_overdue_if_pending(
                        tx,
                        &ObligationRef::Receipt {
                            message: MessageId::new("m2"),
                            seat: SeatId::new("s")
                        },
                        &TimeBasis::Decision,
                        at.utc
                    )?
                    .inserted
                );
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM warning_conditions WHERE clear_warning_id IS NULL",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[test]
fn unavailable_recovery_records_one_bounded_close_job_for_many_threads() {
    let (context, mut db, clock) = seeded_db(300_000);
    // Far above the 5-row foreground bound and several 16-unit work turns.
    const CONDITIONS: i64 = 64;
    for number in 0..CONDITIONS {
        let thread = format!("u{number}");
        let warning = format!("e{number}");
        db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES (?1,'34750f98bd59fcfc946da45aaabe933b','topic','goal',0,0)", [&thread]).unwrap();
        db.execute("INSERT INTO warning_conditions(condition_kind,thread_id,condition_id,affected_seat_id,episode,open_warning_id,opened_seq) VALUES ('unavailable',?1,'1:s:1','s',1,?2,1)", params![thread,warning]).unwrap();
    }
    let before: i64 = db
        .query_row("SELECT total_changes()", [], |r| r.get(0))
        .unwrap();
    // Migration 46's presentation_fence_* triggers bump the one-row
    // presentation_warning_fence after each fenced write (seats, work_jobs,
    // warning_close_sweeps); total_changes() counts those trigger updates.
    let fence = |db: &rusqlite::Connection| -> i64 {
        db.query_row(
            "SELECT version FROM presentation_warning_fence WHERE singleton=1",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };
    let fence_before = fence(&db);
    context
        .execute_decision(
            &mut db,
            |_| Ok(()),
            |tx, at, _| {
                assert_eq!(
                    schema::clear_open_unavailability_for_seat(tx, "s", at.utc)?,
                    1
                );
                tx.execute("UPDATE seats SET unavailability_open=0 WHERE id='s'", [])
                    .unwrap();
                Ok(())
            },
        )
        .unwrap();
    let after: i64 = db
        .query_row("SELECT total_changes()", [], |r| r.get(0))
        .unwrap();
    let fence_bumps = fence(&db) - fence_before;
    let rows = after - before - fence_bumps;
    assert!(rows <= 5, "foreground touched {rows} rows");
    // At most one fence bump per fenced foreground write: still independent
    // of the number of open conditions.
    assert!(
        (1..=rows).contains(&fence_bumps),
        "foreground bumped the presentation fence {fence_bumps} times for {rows} rows"
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM warning_conditions WHERE clear_warning_id IS NOT NULL",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(db.query_row("SELECT count(*) FROM work_jobs WHERE kind='warning_condition_close' AND status='pending'", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    let job: String = db
        .query_row(
            "SELECT id FROM work_jobs WHERE kind='warning_condition_close'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let budget = CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Cancellation::default(),
    };
    let first = crate::store::materialization::advance_work(
        &mut db,
        &job,
        crate::ports::DurableWorkAdmission { max_units: 1 },
        &budget,
        clock.as_ref(),
    )
    .unwrap();
    assert!(first.has_more);
    assert_eq!(first.processed_this_turn, 1);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM warning_conditions WHERE clear_warning_id IS NOT NULL",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    drop(db);
    let mut db = context.open_writer().unwrap();
    while crate::store::materialization::advance_work(
        &mut db,
        &job,
        crate::ports::DurableWorkAdmission { max_units: 16 },
        &budget,
        clock.as_ref(),
    )
    .unwrap()
    .has_more
    {}
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM warning_conditions WHERE clear_warning_id IS NOT NULL",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        CONDITIONS
    );
    assert_eq!(db.query_row("SELECT count(*) FROM work_jobs WHERE kind='warning_condition_close' AND status='complete'", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
}

#[test]
fn overlapping_waiver_sweeps_keep_each_conditions_first_close_time() {
    let (context, mut db, clock) = seeded_db(300_000);
    for (thread, warning) in [("t", "eold"), ("t2", "enew")] {
        if thread == "t2" {
            db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t2','34750f98bd59fcfc946da45aaabe933b','topic','goal',0,0)", []).unwrap();
        }
        db.execute("INSERT INTO warning_conditions(condition_kind,thread_id,condition_id,affected_seat_id,open_warning_id,opened_seq) VALUES ('receipt',?1,'s','s',?2,1)", params![thread, warning]).unwrap();
        if thread == "t" {
            context
                .execute_decision(
                    &mut db,
                    |_| Ok(()),
                    |tx, _, _| {
                        assert_eq!(
                            schema::clear_waived_receipt_conditions_for_seat(
                                tx,
                                "s",
                                UtcMillis(100)
                            )?,
                            1
                        );
                        Ok(())
                    },
                )
                .unwrap();
        }
    }
    context
        .execute_decision(
            &mut db,
            |_| Ok(()),
            |tx, _, _| {
                assert_eq!(
                    schema::clear_waived_receipt_conditions_for_seat(tx, "s", UtcMillis(200))?,
                    1
                );
                Ok(())
            },
        )
        .unwrap();
    let jobs: Vec<String> = db
        .prepare("SELECT id FROM work_jobs WHERE kind='warning_condition_close' ORDER BY ordinal")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(jobs.len(), 2);
    let budget = CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Cancellation::default(),
    };
    while crate::store::materialization::advance_work(
        &mut db,
        &jobs[1],
        crate::ports::DurableWorkAdmission { max_units: 16 },
        &budget,
        clock.as_ref(),
    )
    .unwrap()
    .has_more
    {}
    let old: Option<String> = db
        .query_row(
            "SELECT clear_warning_id FROM warning_conditions WHERE open_warning_id='eold'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(old.is_none());
    while crate::store::materialization::advance_work(
        &mut db,
        &jobs[0],
        crate::ports::DurableWorkAdmission { max_units: 16 },
        &budget,
        clock.as_ref(),
    )
    .unwrap()
    .has_more
    {}
    let times: Vec<i64> = db.prepare("SELECT m.decision_at FROM warning_conditions c JOIN messages m ON m.id=c.clear_warning_id ORDER BY c.ordinal").unwrap()
        .query_map([], |r| r.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
    assert_eq!(times, vec![100, 200]);
}

#[test]
fn deferred_clear_recipient_snapshot_excludes_member_joining_after_close() {
    let (context, mut db, clock) = seeded_db(300_000);
    for seat in ["early", "late"] {
        db.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at,home_node,is_local,host_id) VALUES (?1,'34750f98bd59fcfc946da45aaabe933b','resolved','native',1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))", [seat]).unwrap();
    }
    db.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t','early',1,1)", []).unwrap();
    db.execute(
        "UPDATE local_node SET decision_seq=2 WHERE id='34750f98bd59fcfc946da45aaabe933b'",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO warning_conditions(condition_kind,thread_id,condition_id,affected_seat_id,open_warning_id,opened_seq) VALUES ('receipt','t','s','s','eopen',1)", []).unwrap();
    context
        .execute_decision(
            &mut db,
            |_| Ok(()),
            |tx, _, _| {
                schema::clear_waived_receipt_conditions_for_seat(tx, "s", UtcMillis(100))?;
                Ok(())
            },
        )
        .unwrap();
    db.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t','late',1,3)", []).unwrap();
    db.execute(
        "UPDATE local_node SET decision_seq=3 WHERE id='34750f98bd59fcfc946da45aaabe933b'",
        [],
    )
    .unwrap();
    let job: String = db
        .query_row(
            "SELECT id FROM work_jobs WHERE kind='warning_condition_close'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let budget = CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Cancellation::default(),
    };
    while crate::store::materialization::advance_work(
        &mut db,
        &job,
        crate::ports::DurableWorkAdmission { max_units: 16 },
        &budget,
        clock.as_ref(),
    )
    .unwrap()
    .has_more
    {}
    let clear: String = db
        .query_row(
            "SELECT clear_warning_id FROM warning_conditions WHERE open_warning_id='eopen'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(crate::store::effective::is_warning_recipient(&db, &clear, "early").unwrap());
    assert!(!crate::store::effective::is_warning_recipient(&db, &clear, "late").unwrap());
    let fanout = format!("work:{clear}");
    while crate::store::materialization::advance_work(
        &mut db,
        &fanout,
        crate::ports::DurableWorkAdmission { max_units: 16 },
        &budget,
        clock.as_ref(),
    )
    .unwrap()
    .has_more
    {}
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM warning_recipients WHERE warning_id=?1 AND seat_id='early'",
            [&clear],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM warning_recipients WHERE warning_id=?1 AND seat_id='late'",
            [&clear],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn receipt_reopen_publishes_queued_clear_before_new_open() {
    let (context, mut db, _) = seeded_db(300_000);
    db.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('34750f98bd59fcfc946da45aaabe933b','m1','t',1,'ordinary','first',0,1)", []).unwrap();
    db.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) VALUES ('m1','t','s','pending',300000,0,300000)", []).unwrap();
    db.execute("UPDATE threads SET next_sequence=2 WHERE id='t'", [])
        .unwrap();
    context
        .execute_decision(
            &mut db,
            |_| Ok(()),
            |tx, at, _| {
                record_overdue_if_pending(
                    tx,
                    &ObligationRef::Receipt {
                        message: MessageId::new("m1"),
                        seat: SeatId::new("s"),
                    },
                    &TimeBasis::Decision,
                    at.utc,
                )?;
                Ok(())
            },
        )
        .unwrap();
    context.execute_decision(&mut db, |_| Ok(()), |tx, at, _| {
        let cutoff = schema::next_decision_seq(tx, "34750f98bd59fcfc946da45aaabe933b")?;
        tx.execute("INSERT INTO human_receipt_waivers(seat_id,through_decision_seq,human_generation,decided_at) VALUES ('s',?1,1,?2)", params![cutoff as i64,at.utc.0]).unwrap();
        schema::clear_waived_receipt_conditions_for_seat(tx, "s", at.utc)?;
        Ok(())
    }).unwrap();
    let cutoff: i64 = db
        .query_row(
            "SELECT close_decision_seq FROM warning_close_sweeps",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let next_seq = cutoff + 1;
    db.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('34750f98bd59fcfc946da45aaabe933b','m2','t',3,'ordinary','second',0,?1)", [next_seq]).unwrap();
    db.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) VALUES ('m2','t','s','pending',300000,0,300000)", []).unwrap();
    db.execute(
        "UPDATE local_node SET decision_seq=?1 WHERE id='34750f98bd59fcfc946da45aaabe933b'",
        [next_seq],
    )
    .unwrap();
    db.execute("UPDATE threads SET next_sequence=4 WHERE id='t'", [])
        .unwrap();
    context
        .execute_decision(
            &mut db,
            |_| Ok(()),
            |tx, at, _| {
                assert!(
                    record_overdue_if_pending(
                        tx,
                        &ObligationRef::Receipt {
                            message: MessageId::new("m2"),
                            seat: SeatId::new("s")
                        },
                        &TimeBasis::Decision,
                        at.utc
                    )?
                    .inserted
                );
                Ok(())
            },
        )
        .unwrap();
    let rows: Vec<(i64, Option<i64>, Option<i64>)> = db.prepare("SELECT opened_seq,cleared_seq,(SELECT recipient_cutoff_seq FROM warning_jobs WHERE warning_id=c.clear_warning_id) FROM warning_conditions c WHERE condition_kind='receipt' ORDER BY ordinal").unwrap()
        .query_map([], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap().collect::<Result<_,_>>().unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows[0].0 < rows[0].1.unwrap() && rows[0].1.unwrap() < rows[1].0);
    assert_eq!(rows[0].2, Some(cutoff));
}

#[test]
fn human_waiver_closes_warned_receipt_without_acking_it() {
    let (context, mut db, clock) = seeded_db(300_000);
    db.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('34750f98bd59fcfc946da45aaabe933b','m','t',1,'ordinary','body',0,1)", []).unwrap();
    db.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) VALUES ('m','t','s','pending',300000,0,300000)", []).unwrap();
    db.execute("UPDATE threads SET next_sequence=2 WHERE id='t'", [])
        .unwrap();
    context
        .execute_decision(
            &mut db,
            |_| Ok(()),
            |tx, at, _| {
                record_overdue_if_pending(
                    tx,
                    &ObligationRef::Receipt {
                        message: MessageId::new("m"),
                        seat: SeatId::new("s"),
                    },
                    &TimeBasis::Decision,
                    at.utc,
                )?;
                Ok(())
            },
        )
        .unwrap();
    context.execute_decision(&mut db, |_| Ok(()), |tx, at, _| {
        let cutoff = schema::next_decision_seq(tx, "34750f98bd59fcfc946da45aaabe933b")?;
        tx.execute("INSERT INTO human_receipt_waivers(seat_id,through_decision_seq,human_generation,decided_at) VALUES ('s',?1,1,?2)", params![cutoff as i64,at.utc.0]).unwrap();
        assert_eq!(schema::clear_waived_receipt_conditions_for_seat(tx, "s", at.utc)?, 1);
        Ok(())
    }).unwrap();
    let job: String = db
        .query_row(
            "SELECT id FROM work_jobs WHERE kind='warning_condition_close'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    while crate::store::materialization::advance_work(
        &mut db,
        &job,
        crate::ports::DurableWorkAdmission { max_units: 16 },
        &CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: Cancellation::default(),
        },
        clock.as_ref(),
    )
    .unwrap()
    .has_more
    {}
    let (state, clear): (String, Option<String>) = db.query_row("SELECT r.state,c.clear_warning_id FROM receipts r JOIN warning_conditions c ON c.open_warning_id=r.warning_message_id WHERE r.message_id='m'", [], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(state, "pending");
    assert!(clear.is_some());
}

#[test]
fn invitation_warning_clear_is_recorded_once_after_terminal_settlement() {
    let (context, mut db, _) = seeded_db(300_000);
    add_invitation(&db, "v1", 300_000, "pending");
    context
        .execute_decision(
            &mut db,
            |_| Ok(()),
            |tx, at, _| {
                record_overdue_if_pending(
                    tx,
                    &ObligationRef::Invitation(InvitationId::new("v1")),
                    &TimeBasis::Decision,
                    at.utc,
                )?;
                Ok(())
            },
        )
        .unwrap();
    context
        .execute_decision(
            &mut db,
            |_| Ok(()),
            |tx, at, _| {
                tx.execute(
                    "UPDATE invitations SET state='recipient_retired',retired_at=?1 WHERE id='v1'",
                    [at.utc.0],
                )
                .unwrap();
                assert!(schema::clear_warning_condition_for_invitation(
                    tx, "v1", at.utc
                )?);
                assert!(!schema::clear_warning_condition_for_invitation(
                    tx, "v1", at.utc
                )?);
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM warning_jobs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn receipt_warning_condition_clears_once_and_reopens_with_new_source() {
    let (context, mut db, _) = seeded_db(300_000);
    for (id, sequence) in [("m1", 1), ("m2", 2), ("m3", 3)] {
        db.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('34750f98bd59fcfc946da45aaabe933b',?1,'t',?2,'ordinary','body',0,?2)", params![id,sequence]).unwrap();
        db.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) VALUES (?1,'t','s','pending',300000,0,300000)", [id]).unwrap();
    }
    db.execute("UPDATE threads SET next_sequence=4 WHERE id='t'", [])
        .unwrap();
    let warn = |tx: &rusqlite::Transaction<'_>, id: &str, at: UtcMillis| {
        record_overdue_if_pending(
            tx,
            &ObligationRef::Receipt {
                message: MessageId::new(id),
                seat: SeatId::new("s"),
            },
            &TimeBasis::Decision,
            at,
        )
    };
    context
        .execute_decision(
            &mut db,
            |_| Ok(()),
            |tx, at, _| {
                warn(tx, "m1", at.utc)?;
                warn(tx, "m2", at.utc)?;
                Ok(())
            },
        )
        .unwrap();
    context
        .execute_decision(
            &mut db,
            |_| Ok(()),
            |tx, at, _| {
                tx.execute(
                    "UPDATE receipts SET state='acked' WHERE message_id='m1'",
                    [],
                )
                .unwrap();
                assert_eq!(
                    schema::clear_warning_conditions_for_receipts(tx, "t", "s", at.utc)?,
                    0
                );
                tx.execute(
                    "UPDATE receipts SET state='acked' WHERE message_id='m2'",
                    [],
                )
                .unwrap();
                assert_eq!(
                    schema::clear_warning_conditions_for_receipts(tx, "t", "s", at.utc)?,
                    1
                );
                assert_eq!(
                    schema::clear_warning_conditions_for_receipts(tx, "t", "s", at.utc)?,
                    0
                );
                Ok(())
            },
        )
        .unwrap();
    context
        .execute_decision(
            &mut db,
            |_| Ok(()),
            |tx, at, _| {
                assert!(warn(tx, "m3", at.utc)?.inserted);
                Ok(())
            },
        )
        .unwrap();
    let phases: Vec<String> = db.prepare("SELECT CASE WHEN id IN (SELECT clear_warning_id FROM warning_conditions) THEN 'clear' ELSE 'open' END FROM messages WHERE kind='warn' ORDER BY sequence")
        .unwrap().query_map([], |r| r.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
    assert_eq!(phases, ["open", "clear", "open"]);
}

#[test]
fn delayed_warning_attribution_keeps_open_and_clear_actionable_in_order() {
    let (context, mut db, clock) = seeded_db(300_000);
    db.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at,home_node,is_local,host_id) VALUES ('p','34750f98bd59fcfc946da45aaabe933b','resolved','native',1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))", []).unwrap();
    for seat in ["s", "p"] {
        db.execute(
            "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t',?1,'joined')",
            [seat],
        )
        .unwrap();
        db.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t',?1,1,1)", [seat]).unwrap();
    }
    db.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('34750f98bd59fcfc946da45aaabe933b','m','t',1,'ordinary','body',0,1)", []).unwrap();
    db.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) VALUES ('m','t','s','pending',300000,0,300000)", []).unwrap();
    db.execute("UPDATE threads SET next_sequence=2 WHERE id='t'", [])
        .unwrap();
    context
        .execute_decision(
            &mut db,
            |_| Ok(()),
            |tx, at, _| {
                record_overdue_if_pending(
                    tx,
                    &ObligationRef::Receipt {
                        message: MessageId::new("m"),
                        seat: SeatId::new("s"),
                    },
                    &TimeBasis::Decision,
                    at.utc,
                )?;
                Ok(())
            },
        )
        .unwrap();
    context
        .execute_decision(
            &mut db,
            |_| Ok(()),
            |tx, at, _| {
                tx.execute("UPDATE receipts SET state='acked' WHERE message_id='m'", [])
                    .unwrap();
                assert_eq!(
                    schema::clear_warning_conditions_for_receipts(tx, "t", "s", at.utc)?,
                    1
                );
                Ok(())
            },
        )
        .unwrap();
    let warnings: Vec<(String, i64)> = db
        .prepare("SELECT id,sequence FROM messages WHERE kind='warn' ORDER BY sequence")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(warnings.len(), 2);
    assert!(warnings[0].1 < warnings[1].1);
    for (id, _) in &warnings {
        let job = format!("work:{id}");
        while crate::store::materialization::advance_work(
            &mut db,
            &job,
            crate::ports::DurableWorkAdmission { max_units: 16 },
            &CallBudget {
                deadline: MonoInstant(u64::MAX),
                cancellation: Cancellation::default(),
            },
            clock.as_ref(),
        )
        .unwrap()
        .has_more
        {}
    }
    for (id, _) in warnings {
        let recipients: i64 = db
            .query_row(
                "SELECT count(*) FROM warning_recipients WHERE warning_id=?1 AND seat_id='p'",
                [id.as_str()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(recipients, 1);
        let warning = crate::store::effective::effective_warning_by_id(&db, &id)
            .unwrap()
            .unwrap();
        assert!(crate::store::effective::warning_condition_actionable(&db, &warning).unwrap());
    }
}

#[test]
fn event_helper_rejects_invalid_structured_payload() {
    let (context, mut db, _) = seeded_db(50);
    let thread = ThreadId::new("t");
    let result = context.execute_decision(
        &mut db,
        |_| Ok(()),
        |tx, at, _| {
            schema::append_event_once(
                tx,
                schema::EventInput {
                    thread: &thread,
                    key: "bad",
                    kind: "info",
                    payload_json: "not-json",
                    decision_at: at.utc,
                    source_message: None,
                    source_invitation: None,
                },
            )
        },
    );
    assert_eq!(result.unwrap_err().code, ErrorCode::InvalidRequest);
    assert_eq!(event_count(&db), 0);
}

#[test]
fn effective_retirement_fences_pending_rows_but_preserves_settled_provenance() {
    let (context, mut db, _) = seeded_db(50);
    db.execute("INSERT INTO memberships(thread_id, seat_id, state, joined_at) VALUES ('t', 's', 'joined', 1)", []).unwrap();
    add_invitation(&db, "v1", 300_000, "pending");
    add_invitation(&db, "v2", 300_000, "accepted");
    for (id, sequence, state) in [("m1", 1, "pending"), ("m2", 2, "acked")] {
        db.execute("INSERT INTO messages(instance_id,id, thread_id, sequence, kind, body, decision_at,decision_seq) VALUES ('34750f98bd59fcfc946da45aaabe933b',?1, 't', ?2, 'ordinary', 'body', 0,?2)", params![id, sequence]).unwrap();
        db.execute("INSERT INTO receipts(message_id, thread_id, seat_id, state, frozen_duration_ms) VALUES (?1, 't', 's', ?2, 300000)", params![id, state]).unwrap();
    }
    db.execute(
        "UPDATE seats SET state='retired', retired_at=50 WHERE id='s'",
        [],
    )
    .unwrap();
    context
        .execute_decision(
            &mut db,
            |_| Ok(()),
            |tx, _, _| {
                assert_eq!(
                    schema::effective_membership_state(tx, &ThreadId::new("t"), &SeatId::new("s"))?
                        .unwrap()
                        .state,
                    "retired"
                );
                assert_eq!(
                    crate::store::effective::effective_receipt(tx, "m1", "s")?
                        .unwrap()
                        .state,
                    crate::store::effective::EffectiveReceiptState::RecipientRetired
                );
                assert_eq!(
                    crate::store::effective::effective_receipt(tx, "m2", "s")?
                        .unwrap()
                        .state,
                    crate::store::effective::EffectiveReceiptState::Acknowledged
                );
                Ok(())
            },
        )
        .unwrap();
}

#[test]
fn filter_revision_updates_only_its_scope_in_the_transaction() {
    let (context, mut db, _) = seeded_db(50);
    context
        .execute_decision(
            &mut db,
            |_| Ok(()),
            |tx, _, _| {
                schema::bump_filter_revision(
                    tx,
                    "34750f98bd59fcfc946da45aaabe933b",
                    "directory",
                    "t",
                )?;
                schema::bump_filter_revision(
                    tx,
                    "34750f98bd59fcfc946da45aaabe933b",
                    "directory",
                    "t",
                )?;
                schema::bump_filter_revision(tx, "34750f98bd59fcfc946da45aaabe933b", "topic", "t")?;
                Ok(())
            },
        )
        .unwrap();
    let revisions: (i64, i64) = db.query_row("SELECT (SELECT revision FROM filter_revisions WHERE instance_id='34750f98bd59fcfc946da45aaabe933b' AND scope_kind='directory' AND scope_key='t'), (SELECT revision FROM filter_revisions WHERE instance_id='34750f98bd59fcfc946da45aaabe933b' AND scope_kind='topic' AND scope_key='t')", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert_eq!(revisions, (2, 1));
}

/// Kills: removing (or weakening) the writer-connection binding-evidence
/// guard, so a live binding without terminal/incarnation could be stored and
/// later never reconfirmed after a host invalidation.
#[test]
fn writer_guard_rejects_live_bindings_without_reconfirmation_evidence() {
    let (_context, db, _clock) = seeded_db(0);
    let insert = |generation: i64, evidence: &str, ended: &str| {
        db.execute(
            &format!("INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation,ended_at) VALUES ('s',{generation},1,'p','b',1,'codex','n','e{generation}','cooperative_top_level',1,1,{evidence},{ended})"),
            [],
        )
    };
    for missing in [
        "NULL,'inc'",
        "'term',NULL",
        "NULL,NULL",
        "'','inc'",
        "'term',''",
    ] {
        let error = insert(1, missing, "NULL").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("occupant binding lacks reconfirmation evidence"),
            "{missing}: {error}"
        );
    }
    // Ended history is not a live binding and is not reconfirmed.
    insert(1, "NULL,NULL", "5").unwrap();
    insert(2, "'term','inc'", "NULL").unwrap();
    // Re-registering a live binding must keep its evidence.
    db.execute(
        "UPDATE occupant_bindings SET registered_at=NULL WHERE generation=2",
        [],
    )
    .unwrap();
    let error = db
        .execute(
            "UPDATE occupant_bindings SET registered_at=7,incarnation=NULL WHERE generation=2",
            [],
        )
        .unwrap_err();
    assert!(
        error.to_string().contains("reconfirmation evidence"),
        "{error}"
    );
    db.execute(
        "UPDATE occupant_bindings SET registered_at=7 WHERE generation=2",
        [],
    )
    .unwrap();
    let rows: i64 = db
        .query_row(
            "SELECT count(*) FROM occupant_bindings WHERE ended_at IS NULL AND (terminal_id IS NULL OR incarnation IS NULL)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rows, 0);
}

/// Legacy stores (cooperative bindings written without evidence) are repaired
/// at writer startup only from the seat's own matching verified structural
/// proof; everything else is left untouched and fail-closed.
/// Kills: dropping the startup backfill (stuck seats never heal), and a
/// backfill that invents evidence across a different boot, epoch, target
/// generation or terminal, or rewrites an older (non-latest) binding.
#[test]
fn writer_startup_backfills_legacy_binding_evidence_only_from_matching_structural_proof() {
    let path = db_path();
    let clock = Arc::new(TestClock::new(0));
    let context = StoreContext::new(path.clone(), clock.clone()).with_test_host();
    drop(context.open_writer().unwrap());
    {
        // A pre-invariant store: raw connection, no guard installed.
        let db = Connection::open(&path).unwrap();
        db.execute("UPDATE herdr_hosts SET host_boot='b',host_epoch=1", [])
            .unwrap();
        let proof = "structural_terminal_id='term',structural_incarnation='inc',structural_incarnation_kind='coherent_enumeration',structural_host_boot='b',structural_host_epoch=1,structural_connection_epoch=1,structural_observation_sequence=1";
        for (seat, state, reason) in [
            ("ok", "resolved", "NULL"),
            ("stuck", "unresolved", "'host_invalidation'"),
            ("boot", "resolved", "NULL"),
            ("epoch", "resolved", "NULL"),
            ("tgen", "resolved", "NULL"),
            ("term", "resolved", "NULL"),
            ("noproof", "resolved", "NULL"),
        ] {
            db.execute(
                &format!("INSERT INTO seats(id,instance_id,state,unresolved_reason,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) VALUES ('{seat}','34750f98bd59fcfc946da45aaabe933b','{state}',{reason},'native','p-{seat}',2,1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))"),
                [],
            )
            .unwrap();
            if seat != "noproof" {
                db.execute(&format!("UPDATE seats SET {proof} WHERE id='{seat}'"), [])
                    .unwrap();
            }
        }
        let bind = |seat: &str,
                    generation: i64,
                    boot: &str,
                    epoch: i64,
                    tgen: i64,
                    terminal: &str,
                    ended: &str| {
            db.execute(
                &format!("INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,ended_at) VALUES ('{seat}',{generation},{tgen},'p-{seat}','{boot}',{epoch},'codex','n','e','cooperative_top_level',1,1,{terminal},{ended})"),
                [],
            )
            .unwrap();
        };
        bind("ok", 1, "b", 1, 1, "NULL", "NULL");
        // Ended by the invalidation that left the seat stuck.
        bind("stuck", 0, "b", 1, 1, "NULL", "3");
        bind("stuck", 1, "b", 1, 1, "NULL", "4");
        bind("boot", 1, "other", 1, 1, "NULL", "NULL");
        bind("epoch", 1, "b", 2, 1, "NULL", "NULL");
        bind("tgen", 1, "b", 1, 2, "NULL", "NULL");
        bind("term", 1, "b", 1, 1, "'other-term'", "NULL");
        bind("noproof", 1, "b", 1, 1, "NULL", "NULL");
    }
    let db = context.open_writer().unwrap();
    // S1 (wave-2 fix1): the verification result is kept, not discarded:
    // "ok" and "stuck" backfilled; boot, epoch, tgen, term and noproof lack.
    // Kills: `open_writer` dropping `guard_binding_evidence`'s counts.
    assert_eq!(
        context.binding_evidence_startup(),
        Some(crate::ports::BindingEvidenceStartup {
            backfilled: 2,
            still_lacking: 5,
        })
    );
    let evidence = |seat: &str, generation: i64| -> (Option<String>, Option<String>) {
        db.query_row(
            "SELECT terminal_id,incarnation FROM occupant_bindings WHERE seat_id=?1 AND generation=?2",
            params![seat, generation],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    };
    let backed = (Some("term".to_string()), Some("inc".to_string()));
    assert_eq!(evidence("ok", 1), backed);
    assert_eq!(evidence("stuck", 1), backed);
    assert_eq!(
        evidence("stuck", 0),
        (None, None),
        "only the latest binding"
    );
    for seat in ["boot", "epoch", "tgen", "noproof"] {
        assert_eq!(evidence(seat, 1), (None, None), "{seat}");
    }
    assert_eq!(evidence("term", 1), (Some("other-term".into()), None));
    // Idempotent: a second startup changes nothing and reports the remainder.
    assert_eq!(schema::guard_binding_evidence(&db).unwrap(), (0, 5));
    drop(db);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn fresh_store_lands_at_latest_with_the_evidence_tables() {
    let db = Connection::open_in_memory().unwrap();
    schema::initialize(
        &db,
        || crate::protocol::time::UtcMillis(0),
        &crate::test_support::test_node_seed(),
    )
    .unwrap();
    let version: i64 = db
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, schema::NODE_SCHEMA_VERSION);
    for table in ["harness_version_evidence", "harness_unattributed"] {
        let present: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                [table],
                |r| r.get(0),
            )
            .unwrap();
        assert!(present, "{table} missing");
    }
}

#[test]
fn v12_audit_rejects_a_missing_or_altered_evidence_table() {
    let db = fresh_db();
    schema::initialize(
        &db,
        || crate::protocol::time::UtcMillis(0),
        &crate::test_support::test_node_seed(),
    )
    .unwrap();
    db.execute_batch("DROP TABLE harness_unattributed").unwrap();
    assert_eq!(
        schema::verify_baseline(&db).unwrap_err().code,
        ErrorCode::IncompatibleSchema
    );
    db.execute_batch(
        "CREATE TABLE harness_unattributed(harness TEXT PRIMARY KEY, reason TEXT NOT NULL, at INTEGER NOT NULL) WITHOUT ROWID",
    )
    .unwrap();
    assert_eq!(
        schema::verify_baseline(&db).unwrap_err().code,
        ErrorCode::IncompatibleSchema,
        "a table without the harness CHECK is not the migration's table"
    );
}

#[test]
fn startup_verification_rejects_a_tampered_b1_index() {
    let db = fresh_db();
    schema::initialize(
        &db,
        || crate::protocol::time::UtcMillis(0),
        &crate::test_support::test_node_seed(),
    )
    .unwrap();
    db.execute_batch("DROP INDEX work_jobs_live; CREATE INDEX work_jobs_live ON work_jobs(ordinal) WHERE status='pending';").unwrap();
    let error = schema::initialize(
        &db,
        || crate::protocol::time::UtcMillis(0),
        &crate::test_support::test_node_seed(),
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::IncompatibleSchema);
    assert!(error.detail.contains("work_jobs_live"), "{}", error.detail);
}

// Every index is usable by name with its own predicate (INDEXED BY fails to
// prepare when the index is missing or cannot serve the probe), including the
// seven access paths ht-p03.12.5 relies on.
#[test]
fn b1_indexes_are_usable_by_name() {
    let db = fresh_db();
    schema::initialize(
        &db,
        || crate::protocol::time::UtcMillis(0),
        &crate::test_support::test_node_seed(),
    )
    .unwrap();
    for probe in [
        "SELECT ordinal FROM seats INDEXED BY seats_live_ordinal WHERE instance_id=?1 AND state!='retired' AND ordinal>?2 ORDER BY ordinal LIMIT 1",
        "SELECT ordinal FROM work_jobs INDEXED BY work_jobs_live WHERE status IN ('pending','failed') AND ordinal>?1 ORDER BY ordinal LIMIT 1",
        "SELECT ordinal FROM work_jobs INDEXED BY work_jobs_retention WHERE status='complete' AND kind=?1 AND completed_at<?2 ORDER BY completed_at LIMIT 1",
        "SELECT admission_sequence FROM snapshot_generations INDEXED BY snapshot_generations_retention WHERE host_id=?1 AND admission_sequence<?2 ORDER BY admission_sequence LIMIT 1",
        "SELECT seat_id FROM wake_work INDEXED BY wake_work_reserved WHERE reservation_id IS NOT NULL AND seat_id>?1 LIMIT 1",
        "SELECT ordinal FROM digest_pending_invitations INDEXED BY digest_pending_invitations_seat WHERE seat_id=?1 AND created_decision_seq>?2 ORDER BY created_decision_seq, ordinal LIMIT 1",
        "SELECT ordinal FROM digest_pending_manifest_receipts INDEXED BY digest_pending_manifest_receipts_seat WHERE seat_id=?1 AND decision_seq>?2 ORDER BY decision_seq, ordinal LIMIT 1",
        "SELECT ordinal FROM receipts INDEXED BY receipts_seat_state_ordinal WHERE seat_id=?1 AND state=?2 AND ordinal>?3 ORDER BY ordinal LIMIT 1",
        "SELECT source_ordinal FROM digest_open_warning_recipients INDEXED BY digest_open_warning_recipients_seat WHERE seat_id=?1 AND source=?2 AND source_ordinal>?3 ORDER BY source_ordinal LIMIT 1",
        "SELECT source_ordinal FROM digest_open_warnings INDEXED BY digest_open_warnings_affected WHERE affected_seat_id=?1 AND source=?2 AND source_ordinal>?3 ORDER BY source_ordinal LIMIT 1",
        "SELECT ordinal FROM digest_programmatic_warnings INDEXED BY digest_programmatic_warnings_seat WHERE seat_id=?1 AND ordinal>?2 ORDER BY ordinal LIMIT 1",
        "SELECT ordinal FROM occupant_bindings INDEXED BY occupant_bindings_open WHERE seat_id=?1 AND ended_at IS NULL",
    ] {
        db.prepare(probe).unwrap_or_else(|e| panic!("{probe}: {e}"));
    }
}

/// The columns of `index` of `table`, in key order.
fn index_columns(db: &Connection, index: &str) -> Vec<String> {
    let mut statement = db
        .prepare("SELECT name FROM pragma_index_info(?1) ORDER BY seqno")
        .unwrap();
    statement
        .query_map([index], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// ht-5fr.2.4: every Herdr-scope child table is keyed by `host_id` (target ids
/// are unique only within one Herdr server) and carries no `instance_id`.
#[test]
fn herdr_scope_tables_keyed_by_host_id() {
    let db = fresh_db();
    let primary_key = |table: &str| -> Vec<String> {
        let mut statement = db
            .prepare("SELECT name FROM pragma_table_info(?1) WHERE pk>0 ORDER BY pk")
            .unwrap();
        statement
            .query_map([table], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    for (table, key) in [
        ("observed_targets", vec!["host_id", "target_id"]),
        ("recovery_baseline_targets", vec!["host_id", "target_id"]),
        ("recovery_holds", vec!["host_id", "target_id"]),
        (
            "recovery_baseline_releases",
            vec!["host_id", "baseline_generation_id", "target_id"],
        ),
    ] {
        assert_eq!(primary_key(table), key, "{table} primary key");
    }
    // snapshot_generations: UNIQUE(host_id,host_boot,epoch,observation_sequence)
    // is an auto-index; find it by its columns.
    let unique_found: bool = db
        .prepare("SELECT name FROM pragma_index_list('snapshot_generations') WHERE \"unique\"=1 AND origin='u'")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .any(|name| {
            index_columns(&db, &name.unwrap())
                == ["host_id", "host_boot", "epoch", "observation_sequence"]
        });
    assert!(unique_found, "snapshot_generations UNIQUE key");
    assert_eq!(
        index_columns(&db, "snapshot_generations_status"),
        ["host_id", "status", "id"]
    );
    assert_eq!(
        index_columns(&db, "snapshot_generations_retention"),
        ["host_id", "admission_sequence"]
    );
    assert_eq!(
        index_columns(&db, "allocation_decisions_target"),
        ["host_id", "target_id", "ordinal"]
    );
    for table in [
        "snapshot_generations",
        "observed_targets",
        "recovery_baseline_releases",
        "recovery_baseline_targets",
        "recovery_holds",
        "allocation_decisions",
    ] {
        let columns: Vec<(String, i64)> = db
            .prepare("SELECT name,\"notnull\" FROM pragma_table_info(?1)")
            .unwrap()
            .query_map([table], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(
            !columns.iter().any(|(name, _)| name == "instance_id"),
            "{table} still has instance_id"
        );
        assert!(
            columns.contains(&("host_id".to_owned(), 1)),
            "{table} has no NOT NULL host_id"
        );
        let references_hosts: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_foreign_key_list(?1) WHERE \"from\"='host_id' AND \"table\"='herdr_hosts' AND \"to\"='host_id')",
                [table],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            references_hosts,
            "{table}.host_id does not reference herdr_hosts"
        );
    }
}

// ht-p03.12.6 pin criterion: after v11 the baseline and recovery tables below
// all still exist (ht-p03.2 dropped no table; b4-removed-symbols.txt).
#[test]
fn v11_survivors_recorded() {
    let db = fresh_db();
    schema::initialize(
        &db,
        || crate::protocol::time::UtcMillis(0),
        &crate::test_support::test_node_seed(),
    )
    .unwrap();
    let mut statement = db
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND (name LIKE '%baseline%' OR name LIKE '%recovery%' OR name='allocation_decisions') ORDER BY name")
        .unwrap();
    let survivors: Vec<String> = statement
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    println!("v11 baseline/recovery survivors: {survivors:?}");
    for table in [
        "allocation_decisions",
        "recovery_baseline_releases",
        "recovery_baseline_targets",
        "recovery_holds",
    ] {
        assert!(
            survivors.iter().any(|s| s == table),
            "{table} missing: {survivors:?}"
        );
    }
}

#[test]
fn v10_allocation_decisions_accepts_b5_kinds() {
    let db = fresh_db();
    schema::initialize(
        &db,
        || crate::protocol::time::UtcMillis(0),
        &crate::test_support::test_node_seed(),
    )
    .unwrap();
    crate::test_support::attach_test_host_to_connection(&db);
    let insert = |kind: &str, diagnostic: Option<&str>| {
        db.execute(
            "INSERT INTO allocation_decisions(host_id,target_id,seat_id,kind,decided_at,host_boot,epoch,generation,continuity_diagnostic) VALUES ((SELECT min(host_id) FROM herdr_hosts),'t','s',?1,1,'b',0,1,?2)",
            params![kind, diagnostic],
        )
    };
    for kind in [
        "operator_retire",
        "operator_human_override",
        "cooperative_continuity",
    ] {
        insert(kind, None).unwrap();
    }
    assert!(insert("bogus", None).is_err());
    for diagnostic in ["match", "mismatch", "absent", "read_error"] {
        insert("cooperative_continuity", Some(diagnostic)).unwrap();
    }
    assert!(insert("cooperative_continuity", Some("maybe")).is_err());
}

fn object_exists(db: &Connection, kind: &str, name: &str) -> bool {
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type=?1 AND name=?2)",
        [kind, name],
        |r| r.get(0),
    )
    .unwrap()
}

fn column_exists(db: &Connection, table: &str, column: &str) -> bool {
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name=?2)",
        [table, column],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn fresh_database_has_thread_summary_schema() {
    let (_context, db, _clock) = seeded_db(0);
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        schema::NODE_SCHEMA_VERSION
    );
    for table in [
        "summary_blocks",
        "summary_items",
        "summary_transitions",
        "summary_jobs",
        "summary_job_durations",
        "catch_up",
    ] {
        assert!(object_exists(&db, "table", table), "{table}");
    }
    for (table, column) in [
        ("messages", "author_role"),
        ("messages", "relays_user"),
        ("messages", "author_role_backfilled"),
        ("receipts", "soft_poked_at"),
        ("receipt_state", "soft_poked_at"),
    ] {
        assert!(column_exists(&db, table, column), "{table}.{column}");
    }
    for index in [
        "summary_items_fold",
        "summary_transitions_fold",
        "summary_jobs_live_lease",
        "summary_job_durations_recent",
        "catch_up_extension_until",
        "catch_up_thread_active",
    ] {
        assert!(object_exists(&db, "index", index), "{index}");
    }
    let block = |id: &str, idx: i64| {
        db.execute(
            "INSERT INTO summary_blocks(id,instance_id,thread_id,chunking_version,level,idx,first_seq,last_seq,source_hash,narrative,author_seat_id,model,prompt_version,created_at) VALUES (?1,'34750f98bd59fcfc946da45aaabe933b','t','c1',0,?2,1,2,'h','n','s','m','p',0)",
            params![id, idx],
        )
    };
    block("b1", 0).unwrap();
    assert!(
        block("b2", 0).is_err(),
        "same (thread, version, level, idx)"
    );
    block("b3", 1).unwrap();
    assert!(
        db.execute("UPDATE summary_blocks SET narrative='x' WHERE id='b1'", [])
            .unwrap_err()
            .to_string()
            .contains("summary blocks are immutable")
    );
    assert!(
        db.execute("DELETE FROM summary_blocks WHERE id='b1'", [])
            .unwrap_err()
            .to_string()
            .contains("summary blocks are retained")
    );
    let binding = seat_s_binding(&db);
    let catch_up = |state: &str, reason: Option<&str>, ended: Option<i64>| {
        db.execute(
            "INSERT OR REPLACE INTO catch_up(binding_id,seat_id,thread_id,frontier_seq,entered_at,state,end_reason,ended_at) VALUES (?4,'s','t',1,0,?1,?2,?3)",
            params![state, reason, ended, binding],
        )
    };
    assert!(catch_up("active", Some("ready"), Some(1)).is_err());
    assert!(catch_up("ended", None, None).is_err());
    catch_up("active", None, None).unwrap();
    catch_up("ended", Some("stalled"), Some(1)).unwrap();
}

#[test]
fn new_message_rows_cannot_claim_backfill_or_service_relay() {
    let (_context, db, _clock) = seeded_db(0);
    db.execute(
        "INSERT INTO service_authors(id,instance_id,created_at) VALUES ('svc','34750f98bd59fcfc946da45aaabe933b',0)",
        [],
    )
    .unwrap();
    let ordinary = |id: &str, seq: i64, backfilled: i64| {
        db.execute(
            "INSERT INTO messages(instance_id,id,thread_id,sequence,kind,actor_seat_id,decision_seq,body,decision_at,author_kind,author_role,author_role_backfilled) VALUES ('34750f98bd59fcfc946da45aaabe933b',?1,'t',?2,'ordinary','s',?2,'b',1,'native','agent',?3)",
            params![id, seq, backfilled],
        )
    };
    ordinary("ok", 1, 0).unwrap();
    assert!(
        ordinary("forged", 2, 1)
            .unwrap_err()
            .to_string()
            .contains("invalid message summary authorship")
    );
    let service = |id: &str, seq: i64, relays: i64| {
        db.execute(
            "INSERT INTO messages(instance_id,id,thread_id,sequence,kind,decision_seq,body,decision_at,author_kind,author_service_id,author_role,relays_user) VALUES ('34750f98bd59fcfc946da45aaabe933b',?1,'t',?2,'ordinary',?2,'b',1,'programmatic','svc','service',?3)",
            params![id, seq, relays],
        )
    };
    service("svc_ok", 3, 0).unwrap();
    assert!(
        service("svc_relay", 4, 1)
            .unwrap_err()
            .to_string()
            .contains("invalid message summary authorship")
    );
}

#[test]
fn fresh_database_has_catch_up_release_column() {
    let (_context, db, _clock) = seeded_db(0);
    assert!(column_exists(&db, "catch_up", "release_seq"));
    let binding = seat_s_binding(&db);
    let insert = |release: i64| {
        db.execute(
            "INSERT OR REPLACE INTO catch_up(binding_id,seat_id,thread_id,frontier_seq,entered_at,state,end_reason,ended_at,release_seq) VALUES (?2,'s','t',1,0,'ended','ready',1,?1)",
            params![release, binding],
        )
    };
    assert!(insert(0).is_err(), "release_seq must be positive");
    insert(5).unwrap();
}

#[test]
fn wake_batch_schema_audit_rejects_missing_or_altered_clear_trigger() {
    for replacement in [
        None,
        Some(
            "CREATE TRIGGER wake_batches_clear_retired AFTER UPDATE OF state ON seats BEGIN DELETE FROM wake_batches; END;",
        ),
    ] {
        let db = fresh_db();
        db.execute_batch("DROP TRIGGER wake_batches_clear_retired")
            .unwrap();
        if let Some(replacement) = replacement {
            db.execute_batch(replacement).unwrap();
        }
        let error = schema::verify_baseline(&db).unwrap_err();
        assert_eq!(error.code, ErrorCode::IncompatibleSchema);
        assert!(
            error.detail.contains("wake_batches_clear_retired"),
            "{}",
            error.detail
        );
    }
}

#[test]
fn invitation_rejection_schema_audit_rejects_projection_guard_tampering() {
    for replacement in [
        None,
        Some(
            "CREATE TRIGGER invitations_rejection_projection_guard BEFORE UPDATE OF reject_recorded ON invitations BEGIN SELECT 1; END;",
        ),
    ] {
        let db = fresh_db();
        db.execute_batch("DROP TRIGGER invitations_rejection_projection_guard")
            .unwrap();
        if let Some(sql) = replacement {
            db.execute_batch(sql).unwrap();
        }
        let error =
            schema::initialize(&db, || UtcMillis(1), &crate::test_support::test_node_seed())
                .unwrap_err();
        assert_eq!(error.code, ErrorCode::IncompatibleSchema);
        assert!(
            error
                .detail
                .contains("invitations_rejection_projection_guard"),
            "{}",
            error.detail
        );
    }
}

#[test]
fn recent_activity_writer_rejects_missing_or_null_default() {
    let verified = ScratchStore::fresh("ht-activity-default");
    schema::initialize(
        &Connection::open(verified.path()).unwrap(),
        || UtcMillis(0),
        &crate::test_support::test_node_seed(),
    )
    .unwrap();
    for replacement in [
        "last_activity INTEGER NOT NULL",
        "last_activity INTEGER NOT NULL DEFAULT NULL",
    ] {
        let store = verified.copy("ht-activity-default");
        let path = store.path();
        let db = Connection::open(path).unwrap();
        let original: String = db
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE type='table' AND name='threads'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let altered = original.replace("last_activity INTEGER NOT NULL DEFAULT 0", replacement);
        assert_ne!(original, altered);
        db.execute_batch("PRAGMA writable_schema=ON").unwrap();
        db.execute(
            "UPDATE sqlite_schema SET sql=?1 WHERE type='table' AND name='threads'",
            [altered],
        )
        .unwrap();
        db.execute_batch("PRAGMA writable_schema=OFF").unwrap();
        drop(db);
        let db = Connection::open(path).unwrap();
        let default: Option<String> = db
            .query_row(
                "SELECT dflt_value FROM pragma_table_info('threads') WHERE name='last_activity'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(default.is_none() || default.as_deref() == Some("NULL"));
        let result =
            schema::initialize(&db, || UtcMillis(1), &crate::test_support::test_node_seed());
        drop(db);
        assert!(
            matches!(result,Err(ref error) if error.code==ErrorCode::IncompatibleSchema),
            "writer must reject the altered default before creation: {replacement}: {result:?}"
        );
    }
}

// Catches weakened lexical bounds, descriptor/domain separation, missing composite
// foreign keys, and accepting native/bridge evidence with no runtime identity.
#[test]
fn adapter_migration_v2_tables_enforce_bounded_keys_and_references() {
    let db = fresh_db();
    db.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    schema::initialize(&db, || UtcMillis(0), &crate::test_support::test_node_seed()).unwrap();
    let runtime = "INSERT INTO harness_runtime_identities(harness,identity_key,descriptor_json,last_seen_at) VALUES (?1,'release:1.2.3','{}',0)";
    for invalid in [
        "",
        "Human",
        "9agent",
        "agent name",
        "agent/../../",
        "human",
        "agent\0hidden",
    ] {
        assert!(db.execute(runtime, [invalid]).is_err(), "{invalid}");
    }
    assert!(db.execute(runtime, ["future_agent"]).is_ok());
    assert!(db.execute(runtime, ["future_agent"]).is_err(), "runtime PK");
    let evidence = "INSERT INTO harness_contract_evidence_v2(harness,identity_key,domain,origin,contract_id,first_seen_at,last_seen_at,milestones_json) VALUES ('future_agent',?1,'native_turn',?2,'0123456789abcdef',0,0,'{}')";
    assert!(
        db.execute(
            evidence,
            params!["release:9.9.9", "native_shape_observation"]
        )
        .is_err(),
        "runtime FK"
    );
    assert!(
        db.execute(
            evidence,
            params!["release:1.2.3", "native_shape_observation"]
        )
        .is_ok()
    );
    assert!(
        db.execute(evidence, params!["release:1.2.3", "bridge_envelope"])
            .is_ok()
    );
    assert!(
        db.execute(evidence, params!["release:1.2.3", "native_payload"])
            .is_ok()
    );
    assert!(
        db.execute(evidence, params!["release:1.2.3", "made_up"])
            .is_err()
    );
    assert!(
        db.execute("DELETE FROM harness_runtime_identities", [])
            .is_err()
    );
    for invalid in ["", "UPPER", "9domain", "domain-dash"] {
        assert!(db.execute("INSERT INTO harness_unattributed_v2(harness,domain,origin,reason,at) VALUES ('future_agent',?1,'bridge_envelope','unavailable',0)",[invalid]).is_err());
    }
    assert!(db.execute("INSERT INTO harness_unattributed_v2(harness,domain,origin,reason,at) VALUES ('future_agent','native_turn','native_shape_observation',?1,0)",["x".repeat(257)]).is_err());
    assert!(
        db.execute(
            "UPDATE harness_runtime_identities SET descriptor_json=?1",
            ["x".repeat(4097)]
        )
        .is_err()
    );
    assert!(
        db.execute(
            "UPDATE harness_contract_evidence_v2 SET milestones_json=?1",
            ["x".repeat(4097)]
        )
        .is_err()
    );
    assert!(
        db.execute("UPDATE harness_contract_evidence_v2 SET violation_at=1", [])
            .is_err(),
        "partial sticky violation"
    );
}

#[test]
fn adapter_migration_reopen_rejects_weakened_lexical_tables_and_missing_indexes() {
    for statement in [
        "DROP INDEX harness_contract_evidence_v2_seen",
        "DROP TABLE harness_unattributed_v2",
        "DROP INDEX occupant_bindings_history",
        "PRAGMA writable_schema=ON; UPDATE sqlite_master SET sql=replace(sql,'instr(harness,char(0))=0 AND ','') WHERE name='harness_runtime_identities'; PRAGMA writable_schema=OFF",
    ] {
        let db = fresh_db();
        db.execute_batch(statement).unwrap();
        assert_eq!(
            schema::initialize(&db, || UtcMillis(0), &crate::test_support::test_node_seed())
                .unwrap_err()
                .code,
            ErrorCode::IncompatibleSchema
        );
    }
}

#[test]
fn archival_commit_counter_origin_does_not_collide_with_requests() {
    let hooks = KickHooks::default();
    hooks.on_update("threads");
    hooks.on_commit();
    {
        let _origin = kicks::enter_lane(kicks::Lane::Archival);
        hooks.on_update("threads");
        hooks.on_commit();
    }
    assert_eq!(hooks.commit_counts()["request"], 1);
    assert_eq!(
        hooks.commits[kicks::Lane::Archival as usize].load(Ordering::SeqCst),
        1
    );
    assert_eq!(
        hooks.commit_counts().contains_key("archival"),
        crate::protocol::wire::PROTOCOL_VERSION >= 6
    );
}

fn user_intent_seed(db: &Connection) {
    db.execute_batch("INSERT INTO seats(id,instance_id,state,role,generation,created_at,home_node,is_local,host_id) VALUES ('s','34750f98bd59fcfc946da45aaabe933b','resolved','native',1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts));
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','34750f98bd59fcfc946da45aaabe933b','topic','goal',11,12);
        INSERT INTO summary_blocks(id,instance_id,thread_id,chunking_version,level,idx,first_seq,last_seq,source_hash,narrative,author_seat_id,model,prompt_version,created_at) VALUES ('b','34750f98bd59fcfc946da45aaabe933b','t','c1',0,0,1,2,'hash','old summary','s','m1','p1',15);
        INSERT INTO summary_jobs(id,instance_id,thread_id,chunking_version,level,idx,first_seq,last_seq,fetched_at,created_at) VALUES ('j','34750f98bd59fcfc946da45aaabe933b','t','c1',0,0,1,2,42,14);").unwrap();
}

// Catches a CHECK weakened to accept unknown claims or SQL NULL bypassing attribution.
#[test]
fn user_intent_schema22_checks_and_eligibility() {
    let db = fresh_db();
    schema::initialize(&db, || UtcMillis(0), &crate::test_support::test_node_seed()).unwrap();
    crate::test_support::attach_test_host_to_connection(&db);
    user_intent_seed(&db);
    db.execute(
        "INSERT INTO service_authors(id,instance_id,created_at) VALUES ('svc','34750f98bd59fcfc946da45aaabe933b',0)",
        [],
    )
    .unwrap();
    let mut seq = 0;
    for kind in ["ordinary", "info", "warn"] {
        for role in [Some("human"), Some("agent"), Some("service"), None] {
            for relay in [0, 1] {
                for intent in [Some("query"), Some("request"), Some("rule"), None] {
                    seq += 1;
                    // Use canonical programmatic author shape for services.
                    let result=db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,event_json,decision_at,decision_seq,author_role,relays_user,user_intent,author_kind,author_service_id,actor_label) VALUES (?1,'34750f98bd59fcfc946da45aaabe933b','t',?2,?3,?9,?4,?5,0,?2,?6,?7,?8,?10,?11,?12)",params![format!("m{seq}"),seq,kind,if kind=="ordinary"{Some("body")}else{None},if kind=="ordinary"{None}else{Some("{}")},role,relay,intent,if role==Some("service"){None}else{Some("s")},if role==Some("service"){"programmatic"}else{"native"},if role==Some("service"){Some("svc")}else{None},if role==Some("service"){Some("herdr-graph")}else{None}]);
                    let eligible = kind == "ordinary"
                        && (role == Some("human") || (role == Some("agent") && relay == 1));
                    assert_eq!(
                        result.is_ok(),
                        (intent.is_none() || eligible) && !(role == Some("service") && relay == 1),
                        "{kind}/{role:?}/{relay}/{intent:?}: {result:?}"
                    );
                    if let Err(error) = result {
                        if !eligible && intent.is_some() {
                            assert!(
                                error
                                    .to_string()
                                    .contains("invalid user intent attribution")
                            );
                        }
                    } else {
                        assert_eq!(
                            db.query_row(
                                "SELECT user_intent FROM messages WHERE id=?1",
                                [format!("m{seq}")],
                                |r| r.get::<_, Option<String>>(0)
                            )
                            .unwrap()
                            .as_deref(),
                            intent
                        );
                    }
                }
            }
        }
    }
    assert!(db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,decision_at,decision_seq,author_role,user_intent) VALUES ('bad','34750f98bd59fcfc946da45aaabe933b','t',1000,'ordinary','s','body',0,1000,'human','instruction')",[]).is_err());
    for (ordinal, change) in [None, Some("withdrawn"), Some("replaced")]
        .into_iter()
        .enumerate()
    {
        db.execute("INSERT INTO summary_transitions(block_id,ordinal,thread_id,chunking_version,target_id,new_status,cite_seq,rule_change) VALUES ('b',?1,'t','c1','i.1','superseded',2,?2)",params![ordinal as i64,change]).unwrap();
        assert_eq!(
            db.query_row(
                "SELECT rule_change FROM summary_transitions WHERE block_id='b' AND ordinal=?1",
                [ordinal as i64],
                |r| r.get::<_, Option<String>>(0)
            )
            .unwrap()
            .as_deref(),
            change
        );
    }
    assert!(db.execute("INSERT INTO summary_transitions(block_id,ordinal,thread_id,chunking_version,target_id,new_status,cite_seq,rule_change) VALUES ('b',9,'t','c1','i.1','superseded',2,'completed')",[]).is_err());
}

// Catches startup accepting a present but altered column domain, shape, or guard.
#[test]
fn user_intent_schema22_rejects_column_or_guard_tampering() {
    // Column tampering edits the stored schema text, which only a reopen
    // loads: each case starts from a file copy of one fresh store verified
    // here once.
    let verified = ScratchStore::fresh("ht-intent-tamper");
    schema::initialize(
        &Connection::open(verified.path()).unwrap(),
        || UtcMillis(0),
        &crate::test_support::test_node_seed(),
    )
    .unwrap();
    for replacement in [
        None,
        Some(
            "CREATE TRIGGER messages_user_intent_insert BEFORE INSERT ON messages BEGIN SELECT 1; END;",
        ),
    ] {
        let db = fresh_db();
        db.execute_batch("DROP TRIGGER messages_user_intent_insert")
            .unwrap();
        if let Some(sql) = replacement {
            db.execute_batch(sql).unwrap();
        }
        assert_eq!(
            schema::initialize(&db, || UtcMillis(1), &crate::test_support::test_node_seed())
                .unwrap_err()
                .code,
            ErrorCode::IncompatibleSchema
        );
    }
    for (table, column, domain) in [
        (
            "messages",
            "user_intent",
            "CHECK (user_intent IN ('query', 'request', 'rule'))",
        ),
        (
            "summary_transitions",
            "rule_change",
            "CHECK (rule_change IN ('withdrawn', 'replaced'))",
        ),
        ("summary_jobs", "fetched_bundle_json", ""),
    ] {
        let original_declaration = format!(
            "{column} TEXT{}",
            if domain.is_empty() {
                String::new()
            } else {
                format!(" {domain}")
            }
        );
        let mut replacements = vec![
            String::new(),
            format!("{column} BLOB {domain}"),
            format!("{column} TEXT NOT NULL {domain}"),
            format!("{column} TEXT DEFAULT NULL {domain}"),
        ];
        if !domain.is_empty() {
            replacements.push(format!("{column} TEXT"));
            replacements.push(format!(
                "{column} TEXT {}",
                domain.replace(
                    "))",
                    if column == "user_intent" {
                        ", 'instruction'))"
                    } else {
                        ", 'completed'))"
                    }
                )
            ));
        }
        for replacement in replacements {
            let store = verified.copy("ht-intent-tamper");
            let path = store.path();
            let db = Connection::open(path).unwrap();
            let original: String = db
                .query_row(
                    "SELECT sql FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |r| r.get(0),
                )
                .unwrap();
            let normalize = |sql: &str| sql.split_whitespace().collect::<Vec<_>>().join(" ");
            let original = normalize(&original);
            let declaration = normalize(&original_declaration);
            let altered = if replacement.is_empty() {
                original.replace(&format!(", {declaration}"), "")
            } else {
                original.replace(&declaration, &replacement)
            };
            assert_ne!(original, altered, "{table}/{column}/{replacement}");
            db.execute_batch("PRAGMA writable_schema=ON").unwrap();
            db.execute(
                "UPDATE sqlite_schema SET sql=?1 WHERE type='table' AND name=?2",
                params![altered, table],
            )
            .unwrap();
            db.execute_batch("PRAGMA writable_schema=OFF").unwrap();
            drop(db);
            let db = Connection::open(path).unwrap();
            let result =
                schema::initialize(&db, || UtcMillis(1), &crate::test_support::test_node_seed());
            assert!(
                matches!(result,Err(ref e) if e.code==ErrorCode::IncompatibleSchema),
                "{table}/{replacement}: {result:?}"
            );
        }
    }
}

#[test]
fn archival_schema_refuses_missing_or_weakened_guards_before_writing() {
    for change in [
        "DROP TRIGGER channel_handoff_terminal",
        "DROP INDEX seat_archival_due",
        "DROP TRIGGER archival_operation_activity; CREATE TRIGGER archival_operation_activity AFTER INSERT ON operations BEGIN SELECT 1; END",
    ] {
        let db = fresh_db();
        db.execute_batch(change).unwrap();
        assert!(
            matches!(schema::initialize(&db, || UtcMillis(1), &crate::test_support::test_node_seed()), Err(e) if e.code == ErrorCode::IncompatibleSchema)
        );
    }
}

#[test]
fn archival_schema_audit_preserves_case_sensitive_terminal_state_literals() {
    let db = fresh_db();
    schema::initialize(&db, || UtcMillis(0), &crate::test_support::test_node_seed()).unwrap();
    let sql: String = db
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='channel_handoff_terminal'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    db.execute_batch("DROP TRIGGER channel_handoff_terminal")
        .unwrap();
    db.execute_batch(&sql.replace("OLD.state='completed'", "OLD.state='COMPLETED'"))
        .unwrap();
    assert!(
        matches!(schema::initialize(&db,||UtcMillis(1), &crate::test_support::test_node_seed()),Err(e) if e.code==ErrorCode::IncompatibleSchema),
        "uppercasing the state literal disables absorbing completion and must fail the audit"
    );
}

#[test]
fn v32_startup_rejects_missing_or_altered_binding_objects() {
    for alteration in [
        "DROP INDEX occupant_bindings_session_key",
        "DROP INDEX occupant_bindings_open",
        "DROP INDEX binding_cursors_thread",
        "DROP TABLE binding_cursors",
        "DROP INDEX occupant_bindings_session_key; CREATE INDEX occupant_bindings_session_key ON occupant_bindings(seat_id, harness, native_session, agent_id) WHERE ended_at IS NULL",
    ] {
        let db = fresh_db();
        db.execute_batch(alteration).unwrap();
        assert_eq!(
            schema::initialize(&db, || UtcMillis(0), &crate::test_support::test_node_seed())
                .unwrap_err()
                .code,
            ErrorCode::IncompatibleSchema,
            "{alteration}"
        );
    }
}

#[test]
fn v33_startup_rejects_an_altered_archival_trigger() {
    for alteration in [
        // dropped
        "DROP TRIGGER archival_fence_seats_update".to_owned(),
        "DROP TRIGGER archival_fence_memberships_update".to_owned(),
        "DROP TRIGGER archival_fence_occupant_bindings_insert".to_owned(),
        "DROP TRIGGER archival_fence_occupant_bindings_update".to_owned(),
        // widened column list
        "DROP TRIGGER archival_fence_memberships_update; CREATE TRIGGER archival_fence_memberships_update AFTER UPDATE ON memberships BEGIN UPDATE archival_instances SET mutation_revision=mutation_revision+1; END".to_owned(),
        // dropped V23 trigger V33 does not own
        "DROP TRIGGER archival_fence_memberships_insert".to_owned(),
    ] {
        let db = fresh_db();
        db.execute_batch(&alteration).unwrap();
        assert_eq!(
            schema::initialize(&db, || UtcMillis(0), &crate::test_support::test_node_seed()).unwrap_err().code,
            ErrorCode::IncompatibleSchema,
            "{alteration}"
        );
    }
}

#[test]
fn v34_startup_rejects_altered_catch_up_and_lazy_objects() {
    for alteration in [
        "DROP INDEX catch_up_seat",
        "DROP INDEX catch_up_extension_until",
        "DROP INDEX catch_up_thread_active",
        "DROP INDEX lazy_recipients_legacy",
        "DROP INDEX lazy_recipients_seat_ordinal",
        "DROP INDEX catch_up_seat; CREATE INDEX catch_up_seat ON catch_up(seat_id, thread_id)",
        "DROP TRIGGER archival_fence_catch_up_update",
        "DROP TABLE catch_up; CREATE TABLE catch_up(binding_id TEXT NOT NULL, seat_id TEXT NOT NULL, thread_id TEXT NOT NULL, frontier_seq INTEGER NOT NULL, entered_at INTEGER NOT NULL, extension_until INTEGER, last_progress_at INTEGER, state TEXT NOT NULL, end_reason TEXT, ended_at INTEGER, release_seq INTEGER, PRIMARY KEY(binding_id, thread_id)) STRICT",
        "PRAGMA writable_schema=ON; UPDATE sqlite_schema SET sql=replace(sql,'CHECK(legacy_pending IN (0,1))','CHECK(legacy_pending IN (0,1,2))') WHERE type='table' AND name='lazy_recipients'; PRAGMA writable_schema=OFF",
    ] {
        let db = fresh_db();
        db.execute_batch(alteration).unwrap();
        assert_eq!(
            schema::initialize(&db, || UtcMillis(0), &crate::test_support::test_node_seed())
                .unwrap_err()
                .code,
            ErrorCode::IncompatibleSchema,
            "{alteration}"
        );
    }
}

#[test]
fn v35_startup_rejects_altered_attention_frontier_objects() {
    for alteration in [
        "DROP TABLE digest_notice_offer",
        "DROP TABLE warning_offer",
        "DROP INDEX warning_offer_seat",
        "DROP INDEX digest_notice_offer_seat",
        "DROP INDEX warning_offer_seat; CREATE INDEX warning_offer_seat ON warning_offer(binding_id)",
        "DROP TABLE warning_offer; CREATE TABLE warning_offer(seat_id TEXT PRIMARY KEY REFERENCES seats(id), binding_generation INTEGER NOT NULL, execution_id TEXT NOT NULL, offered_through_seq INTEGER NOT NULL DEFAULT 0) STRICT; CREATE INDEX warning_offer_seat ON warning_offer(seat_id)",
        "DROP TABLE digest_notice_offer; CREATE TABLE digest_notice_offer(binding_id TEXT PRIMARY KEY, seat_id TEXT NOT NULL, offered_ordinal INTEGER NOT NULL) STRICT; CREATE INDEX digest_notice_offer_seat ON digest_notice_offer(seat_id)",
    ] {
        let db = fresh_db();
        db.execute_batch(alteration).unwrap();
        assert_eq!(
            schema::initialize(&db, || UtcMillis(0), &crate::test_support::test_node_seed())
                .unwrap_err()
                .code,
            ErrorCode::IncompatibleSchema,
            "{alteration}"
        );
    }
}

#[test]
fn task3_versionless_schema24_rejects_altered_table_and_missing_index() {
    for alteration in [
        "DROP INDEX harness_contract_diagnostics_recent",
        "DROP TABLE harness_contract_diagnostics; CREATE TABLE harness_contract_diagnostics(harness TEXT, session_id TEXT, contract_id TEXT, event TEXT, field TEXT, first_seen_at INTEGER, last_seen_at INTEGER)",
    ] {
        let db = fresh_db();
        db.execute_batch(alteration).unwrap();
        assert!(
            schema::initialize(&db, || UtcMillis(2), &crate::test_support::test_node_seed())
                .is_err()
        );
    }
}

#[test]
fn invitation_confirmation_schema_refuses_missing_policy_guard() {
    let db = Connection::open_in_memory().unwrap();
    schema::initialize(&db, || UtcMillis(0), &crate::test_support::test_node_seed()).unwrap();
    db.execute_batch("DROP TRIGGER invitations_confirmation_immutable")
        .unwrap();
    assert_eq!(
        schema::initialize(&db, || UtcMillis(0), &crate::test_support::test_node_seed())
            .unwrap_err()
            .code,
        ErrorCode::IncompatibleSchema
    );
}

// Node baseline: creation, verification and refusal.

fn incompatible_detail(result: Result<(), crate::protocol::results::ApiError>) -> String {
    let error = result.unwrap_err();
    assert_eq!(error.code, ErrorCode::IncompatibleSchema, "{error:?}");
    error.detail
}

fn reinitialize(db: &Connection) -> Result<(), crate::protocol::results::ApiError> {
    schema::initialize(db, || UtcMillis(0), &crate::test_support::test_node_seed())
}

#[test]
fn fresh_initialize_sets_node_application_id_and_version() {
    let db = Connection::open_in_memory().unwrap();
    schema::initialize(&db, || UtcMillis(0), &crate::test_support::test_node_seed()).unwrap();
    let application_id: i64 = db
        .pragma_query_value(None, "application_id", |r| r.get(0))
        .unwrap();
    let version: i64 = db
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    let marker: String = db
        .query_row("SELECT marker FROM schema_identity", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        (application_id, version, marker.as_str()),
        (0x4854_4E44, 1, "herdr-threads-node-v1")
    );
    assert_eq!(application_id, i64::from(schema::NODE_APPLICATION_ID));
}

#[test]
fn second_open_verifies() {
    let store = ScratchStore::new("ht-second-open");
    let db = Connection::open(store.path()).unwrap();
    schema::initialize(&db, || UtcMillis(0), &crate::test_support::test_node_seed()).unwrap();
    drop(db);
    let db = Connection::open(store.path()).unwrap();
    schema::initialize(&db, || UtcMillis(0), &crate::test_support::test_node_seed()).unwrap();
    schema::verify_baseline(&db).unwrap();
}

#[test]
fn drift_extra_index_is_incompatible_naming_it() {
    let db = fresh_db();
    db.execute_batch("CREATE INDEX drift_ix ON threads(topic)")
        .unwrap();
    let detail = incompatible_detail(reinitialize(&db));
    assert!(detail.contains("drift_ix"), "{detail}");
}

#[test]
fn drift_altered_table_is_incompatible_naming_it() {
    let db = fresh_db();
    db.execute_batch("ALTER TABLE threads ADD COLUMN drift INTEGER")
        .unwrap();
    let detail = incompatible_detail(reinitialize(&db));
    assert!(detail.contains("table threads"), "{detail}");
}

#[test]
fn drift_missing_object_is_incompatible_naming_it() {
    let db = fresh_db();
    let (kind, name): (String, String) = db
        .query_row(
            "SELECT type, name FROM sqlite_master WHERE type='trigger' ORDER BY name LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    db.execute_batch(&format!("DROP TRIGGER \"{name}\""))
        .unwrap();
    let detail = incompatible_detail(reinitialize(&db));
    assert!(
        detail.contains(&format!("missing schema object: {kind} {name}")),
        "{detail}"
    );
}

#[test]
fn test_fault_trigger_is_tolerated_but_other_extra_objects_are_not() {
    let db = fresh_db();
    db.execute_batch(
        "CREATE TRIGGER ht_test_fault_x BEFORE INSERT ON threads BEGIN SELECT RAISE(ABORT,'x'); END",
    )
    .unwrap();
    reinitialize(&db).unwrap();
    db.execute_batch("CREATE INDEX ht_test_fault_ix ON threads(topic)")
        .unwrap();
    let detail = incompatible_detail(reinitialize(&db));
    assert!(detail.contains("ht_test_fault_ix"), "{detail}");
}

#[test]
fn wrong_application_id_refused() {
    let db = fresh_db();
    db.pragma_update(None, "application_id", 1).unwrap();
    let detail = incompatible_detail(reinitialize(&db));
    assert!(detail.contains("application_id 0x1"), "{detail}");
}

#[test]
fn newer_schema_version_refused() {
    let db = fresh_db();
    db.pragma_update(None, "user_version", 2).unwrap();
    let detail = incompatible_detail(reinitialize(&db));
    assert!(detail.contains("newer than this binary"), "{detail}");
}

#[test]
fn legacy_chain_store_refused() {
    // A pre-node store: a nonempty database stamped with the old chain's
    // `user_version` and no `application_id`.
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE host_instances(id TEXT); PRAGMA user_version = 46;")
        .unwrap();
    let detail = incompatible_detail(reinitialize(&db));
    assert!(detail.contains("pre-node schema version 46"), "{detail}");
}

#[test]
fn unversioned_nonempty_refused() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE unrelated(x)").unwrap();
    let detail = incompatible_detail(reinitialize(&db));
    assert!(detail.contains("unversioned nonempty"), "{detail}");
}

#[test]
fn query_connection_accepts_node_store_and_rejects_wrong_marker() {
    let db = fresh_db();
    schema::verify_query_connection(&db).unwrap();
    db.execute_batch("DROP TABLE schema_identity; CREATE TABLE schema_identity(marker TEXT PRIMARY KEY); INSERT INTO schema_identity VALUES ('herdr-threads-shared-v1-r7');")
        .unwrap();
    assert_eq!(
        schema::verify_query_connection(&db).unwrap_err().code,
        ErrorCode::IncompatibleSchema
    );
    let old = Connection::open_in_memory().unwrap();
    old.execute_batch("CREATE TABLE host_instances(id TEXT); PRAGMA user_version = 46;")
        .unwrap();
    assert_eq!(
        schema::verify_query_connection(&old).unwrap_err().code,
        ErrorCode::IncompatibleSchema
    );
}

fn seed_with_retag(k: u32) -> schema::NodeSeed {
    let uuid = crate::node::NodeUuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef);
    schema::NodeSeed {
        uuid,
        tag: crate::node::tag_for(uuid, k),
        tag_k: k,
    }
}

#[test]
fn initialize_inserts_local_node_and_self_directory_row() {
    let db = Connection::open_in_memory().unwrap();
    let seed = seed_with_retag(3);
    schema::initialize(&db, || UtcMillis(1234), &seed).unwrap();
    let node: (String, i64, i64, String, i64, i64) = db
        .query_row(
            "SELECT id, singleton, created_at, tag, tag_k, decision_seq FROM local_node",
            [],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        node,
        (seed.uuid.to_string(), 1, 1234, seed.tag.to_string(), 3, 0)
    );
    let directory: (String, String, i64, Option<String>, i64, Option<i64>) = db
        .query_row(
            "SELECT node_uuid, tag, tag_k, host_label, first_seen_at, tag_hlc FROM node_directory",
            [],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        directory,
        (
            seed.uuid.to_string(),
            seed.tag.to_string(),
            3,
            None,
            1234,
            None
        )
    );
    let hosts: i64 = db
        .query_row("SELECT count(*) FROM herdr_hosts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(hosts, 0, "no host exists until one is attached");
    // An existing store keeps its own identity: a different seed changes nothing.
    schema::initialize(&db, || UtcMillis(9), &crate::test_support::test_node_seed()).unwrap();
    let id: String = db
        .query_row("SELECT id FROM local_node", [], |r| r.get(0))
        .unwrap();
    assert_eq!(id, seed.uuid.to_string());
}

#[test]
fn fresh_store_without_a_node_seed_is_refused() {
    let db = Connection::open_in_memory().unwrap();
    let error = schema::initialize_with(&db, || UtcMillis(0), None).unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest, "{error:?}");
    assert!(error.detail.contains("node seed missing"), "{error:?}");
    let objects: i64 = db
        .query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))
        .unwrap();
    assert_eq!(objects, 0, "nothing is created without an identity");
}

#[test]
fn second_local_node_row_aborts() {
    let db = fresh_db();
    let error = db
        .execute(
            "INSERT INTO local_node(id,created_at,tag,tag_k) VALUES ('00000000000000000000000000000001',0,'abc',0)",
            [],
        )
        .unwrap_err();
    assert!(error.to_string().contains("UNIQUE"), "{error}");
    let rows: i64 = db
        .query_row("SELECT count(*) FROM local_node", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 1);
}

#[test]
fn local_node_id_must_be_hex32() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(schema::NODE_BASELINE).unwrap();
    for bad in [
        "ABCDEF0123456789ABCDEF0123456789",
        "0123456789abcdef0123456789abcde",
        "0123456789abcdef0123456789abcdef0",
        "0123456789abcdef0123456789abcdeg",
    ] {
        let error = db
            .execute(
                "INSERT INTO local_node(id,created_at,tag,tag_k) VALUES (?1,0,'abc',0)",
                [bad],
            )
            .unwrap_err();
        assert!(error.to_string().contains("CHECK"), "{bad}: {error}");
    }
    for bad_tag in ["ab", "abcd", "ab-"] {
        assert!(
            db.execute(
                "INSERT INTO local_node(id,created_at,tag,tag_k) VALUES ('0123456789abcdef0123456789abcdef',0,?1,0)",
                [bad_tag],
            )
            .is_err(),
            "tag {bad_tag:?}"
        );
    }
    db.execute(
        "INSERT INTO local_node(id,created_at,tag,tag_k) VALUES ('0123456789abcdef0123456789abcdef',0,'aZ9',0)",
        [],
    )
    .unwrap();
}

#[test]
fn herdr_hosts_endpoint_unique_and_bounded() {
    let db = fresh_db();
    let node = crate::test_support::test_node();
    let insert = |endpoint: &str| {
        db.execute(
            "INSERT INTO herdr_hosts(node_id,endpoint,created_at) VALUES (?1,?2,0)",
            params![node, endpoint],
        )
    };
    insert("/run/herdr.sock").unwrap();
    assert!(
        insert("/run/herdr.sock")
            .unwrap_err()
            .to_string()
            .contains("UNIQUE")
    );
    assert!(insert("").unwrap_err().to_string().contains("CHECK"));
    assert!(
        insert(&"x".repeat(4097))
            .unwrap_err()
            .to_string()
            .contains("CHECK")
    );
    insert(&"x".repeat(4096)).unwrap();
    // The bound counts bytes, not characters.
    assert!(insert(&"é".repeat(2049)).is_err());
    let hosts: i64 = db
        .query_row("SELECT count(*) FROM herdr_hosts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(hosts, 2);
}

#[test]
fn host_scope_write_without_primary_host_is_host_unavailable() {
    let clock = Arc::new(TestClock::new(0));
    // No `with_test_host`: nothing is attached.
    let context = StoreContext::new(db_path(), clock);
    let mut conn = context.open_writer().unwrap();
    let node = crate::test_support::test_node();
    let budget = CallBudget {
        deadline: MonoInstant(1_000_000),
        cancellation: Cancellation::default(),
    };
    let error = crate::store::seats::begin_host_observation(
        &context,
        &mut conn,
        &node,
        crate::test_support::test_host_id(),
        &budget,
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::HostUnavailable, "{error:?}");
    assert_eq!(
        context.primary_host().unwrap_err().code,
        ErrorCode::HostUnavailable
    );
    // A read of a Herdr-scope column through the same seam fails the same way.
    let read = conn.query_row(
        "SELECT host_epoch FROM herdr_hosts WHERE host_id=primary_host() AND node_id=?1",
        [&node],
        |r| r.get::<_, i64>(0),
    );
    assert_eq!(
        crate::store::connection::store_error(read.unwrap_err()).code,
        ErrorCode::HostUnavailable
    );
    // Node-scope state is untouched by the refusal and still writable.
    assert_eq!(
        schema::next_decision_seq(&conn, &node).unwrap(),
        1,
        "node-scope counters do not need a host"
    );
    // Attaching makes the same write succeed.
    let host_id = context.attach_host("/run/herdr.sock").unwrap();
    assert_eq!(context.primary_host().unwrap(), host_id);
    let admission = crate::store::seats::begin_host_observation(
        &context,
        &mut conn,
        &node,
        crate::test_support::test_host_id(),
        &budget,
    )
    .unwrap();
    assert_eq!(admission.sequence, 1);
}

#[test]
fn attach_host_is_idempotent_and_stamps_last_attached_at() {
    let clock = Arc::new(TestClock::new(5_000));
    let context = StoreContext::new(db_path(), clock.clone());
    let first = context.attach_host("/run/herdr-a.sock").unwrap();
    assert_eq!(context.primary_host().unwrap(), first);
    clock.utc.store(9_000, Ordering::SeqCst);
    assert_eq!(context.attach_host("/run/herdr-a.sock").unwrap(), first);
    let db = context.open_writer().unwrap();
    let row: (String, i64, Option<i64>) = db
        .query_row(
            "SELECT node_id, created_at, last_attached_at FROM herdr_hosts WHERE host_id=?1",
            [first],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        row,
        (crate::test_support::test_node(), 5_000, Some(9_000)),
        "a re-attach stamps last_attached_at and keeps the row"
    );
    // The AUTOINCREMENT sequence is not advanced by the re-attach.
    let next: i64 = db
        .query_row(
            "INSERT INTO herdr_hosts(node_id,endpoint,created_at) SELECT id,'/run/herdr-b.sock',0 FROM local_node RETURNING host_id",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(next, first + 1);
}

// Journal-core guards (ht-5fr.3.4): raw-SQL fixtures valid under every CHECK.
const PEER_NODE: &str = "0123456789abcdef0123456789abcdef";

fn hash_blob(byte: u8) -> Vec<u8> {
    vec![byte; 32]
}

/// A fresh store with one self log (`log_ix` 1) and one peer log (`log_ix` 2).
fn journal_db() -> Connection {
    let db = fresh_db();
    db.execute(
        "INSERT INTO sync_logs(log_ix,node_uuid,incarnation,is_self,pubkey,genesis_hash,contiguous_counter,head_hash,first_seen_at) \
         SELECT 1,id,1,1,zeroblob(32),zeroblob(32),5,zeroblob(32),0 FROM local_node",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO sync_logs(log_ix,node_uuid,incarnation,is_self,pubkey,genesis_hash,contiguous_counter,head_hash,first_seen_at) \
         VALUES (2,?1,1,0,zeroblob(32),zeroblob(32),5,zeroblob(32),0)",
        [PEER_NODE],
    )
    .unwrap();
    db
}

fn insert_journal(
    db: &Connection,
    counter: i64,
    state: &str,
    applied_seq: Option<i64>,
    parked_on: Option<&str>,
) -> rusqlite::Result<usize> {
    db.execute(
        "INSERT INTO journal(log_ix,counter,prev_hash,hash,hlc,kind,kind_version,deps,payload,signature,ingested_at,apply_state,applied_seq,parked_on) \
         VALUES (2,?1,?2,?3,10,'k',1,x'',x'00',zeroblob(64),0,?4,?5,?6)",
        params![counter, hash_blob(1), hash_blob(2), state, applied_seq, parked_on],
    )
}

fn abort_message(result: rusqlite::Result<usize>) -> String {
    result.unwrap_err().to_string()
}

#[test]
fn journal_core_objects_verify() {
    let db = journal_db();
    schema::verify_baseline(&db).unwrap();
    insert_journal(&db, 1, "pending", None, None).unwrap();
    db.execute(
        "INSERT INTO thread_registers(thread_id,field,value,hlc,node_uuid,src_ingest_seq) VALUES ('t','topic','x',1,?1,1)",
        [PEER_NODE],
    )
    .unwrap();
    db.execute("UPDATE local_node SET active_log_ix=1", [])
        .unwrap();
    let ix: Option<i64> = db
        .query_row("SELECT active_log_ix FROM local_node", [], |r| r.get(0))
        .unwrap();
    assert_eq!(ix, Some(1));
}

#[test]
fn journal_payload_update_aborts() {
    let db = journal_db();
    insert_journal(&db, 1, "pending", None, None).unwrap();
    let message = abort_message(db.execute("UPDATE journal SET payload=x'01'", []));
    assert!(message.contains("journal_envelope_immutable"), "{message}");
    let message = abort_message(db.execute("UPDATE journal SET ingested_at=9", []));
    assert!(message.contains("journal_envelope_immutable"), "{message}");
    // The apply bookkeeping columns stay writable while not applied.
    db.execute(
        "UPDATE journal SET apply_state='failed', apply_error='e'",
        [],
    )
    .unwrap();
}

#[test]
fn journal_delete_aborts() {
    let db = journal_db();
    insert_journal(&db, 1, "pending", None, None).unwrap();
    let message = abort_message(db.execute("DELETE FROM journal", []));
    assert!(message.contains("journal_append_only"), "{message}");
}

#[test]
fn journal_applied_to_pending_aborts() {
    let db = journal_db();
    insert_journal(&db, 1, "applied", Some(1), None).unwrap();
    let message = abort_message(db.execute(
        "UPDATE journal SET apply_state='pending', applied_seq=NULL",
        [],
    ));
    assert!(message.contains("journal_applied_terminal"), "{message}");
}

#[test]
fn sync_logs_contiguous_regression_aborts() {
    let db = journal_db();
    let message = abort_message(db.execute(
        "UPDATE sync_logs SET contiguous_counter=4 WHERE log_ix=2",
        [],
    ));
    assert!(message.contains("sync_logs_monotone"), "{message}");
    db.execute(
        "UPDATE sync_logs SET contiguous_counter=6 WHERE log_ix=2",
        [],
    )
    .unwrap();
}

#[test]
fn sync_logs_applied_regression_aborts() {
    let db = journal_db();
    db.execute("UPDATE sync_logs SET applied_through=3 WHERE log_ix=2", [])
        .unwrap();
    let message =
        abort_message(db.execute("UPDATE sync_logs SET applied_through=2 WHERE log_ix=2", []));
    assert!(message.contains("sync_logs_monotone"), "{message}");
}

#[test]
fn sync_logs_unfreeze_aborts() {
    let db = journal_db();
    db.execute(
        "UPDATE sync_logs SET state='frozen', frozen_at_counter=3 WHERE log_ix=2",
        [],
    )
    .unwrap();
    let message = abort_message(db.execute(
        "UPDATE sync_logs SET state='active', frozen_at_counter=NULL WHERE log_ix=2",
        [],
    ));
    assert!(message.contains("sync_logs_monotone"), "{message}");
}

#[test]
fn sync_logs_self_flag_mismatch_aborts() {
    let db = journal_db();
    // A peer uuid flagged self, and the local uuid flagged not-self.
    let message = abort_message(db.execute(
        "INSERT INTO sync_logs(log_ix,node_uuid,incarnation,is_self,pubkey,genesis_hash,contiguous_counter,head_hash,first_seen_at) \
         VALUES (3,?1,2,1,zeroblob(32),zeroblob(32),0,zeroblob(32),0)",
        [PEER_NODE],
    ));
    assert!(message.contains("sync_logs_self_flag"), "{message}");
    let message = abort_message(db.execute(
        "INSERT INTO sync_logs(log_ix,node_uuid,incarnation,is_self,pubkey,genesis_hash,contiguous_counter,head_hash,first_seen_at) \
         SELECT 3,id,2,0,zeroblob(32),zeroblob(32),0,zeroblob(32),0 FROM local_node",
        [],
    ));
    assert!(message.contains("sync_logs_self_flag"), "{message}");
}

#[test]
fn sync_logs_identity_update_aborts() {
    let db = journal_db();
    for assignment in [
        "incarnation=2",
        "is_self=1",
        "pubkey=zeroblob(32)",
        "genesis_hash=zeroblob(32)",
        "node_uuid='fedcba9876543210fedcba9876543210'",
    ] {
        let message = abort_message(db.execute(
            &format!("UPDATE sync_logs SET {assignment} WHERE log_ix=2"),
            [],
        ));
        assert!(
            message.contains("sync_logs_identity_immutable"),
            "{assignment}: {message}"
        );
    }
    let message = abort_message(db.execute("DELETE FROM sync_logs WHERE log_ix=2", []));
    assert!(message.contains("sync_logs_append_only"), "{message}");
}

#[test]
fn thread_registers_regression_aborts() {
    let db = journal_db();
    insert_journal(&db, 1, "pending", None, None).unwrap();
    db.execute(
        "INSERT INTO thread_registers(thread_id,field,value,hlc,node_uuid,src_ingest_seq) VALUES ('t','topic','x',5,?1,1)",
        [PEER_NODE],
    )
    .unwrap();
    // Equal and lower stamps both lose; a higher node_uuid at equal hlc wins.
    for (hlc, node) in [
        (5, PEER_NODE),
        (4, "ffffffffffffffffffffffffffffffff"),
        (5, "00000000000000000000000000000000"),
    ] {
        let message = abort_message(db.execute(
            "UPDATE thread_registers SET value='y', hlc=?1, node_uuid=?2",
            params![hlc, node],
        ));
        assert!(
            message.contains("thread_registers_lww"),
            "{hlc} {node}: {message}"
        );
    }
    db.execute(
        "UPDATE thread_registers SET value='y', hlc=5, node_uuid='ffffffffffffffffffffffffffffffff'",
        [],
    )
    .unwrap();
}

#[test]
fn thread_registers_delete_aborts() {
    let db = journal_db();
    insert_journal(&db, 1, "pending", None, None).unwrap();
    db.execute(
        "INSERT INTO thread_registers(thread_id,field,value,hlc,node_uuid,src_ingest_seq) VALUES ('t','name','x',5,?1,1)",
        [PEER_NODE],
    )
    .unwrap();
    let message = abort_message(db.execute("DELETE FROM thread_registers", []));
    assert!(
        message.contains("thread_registers_append_only"),
        "{message}"
    );
}

fn insert_forked(db: &Connection) {
    db.execute(
        "INSERT INTO journal_forked(log_ix,counter,prev_hash,hash,hlc,kind,kind_version,deps,payload,signature,source_node,received_at) \
         VALUES (2,3,zeroblob(32),zeroblob(32),1,'k',1,x'',x'00',zeroblob(64),NULL,0)",
        [],
    )
    .unwrap();
}

#[test]
fn journal_forked_update_aborts() {
    let db = journal_db();
    insert_forked(&db);
    for assignment in ["payload=x'01'", "received_at=9", "source_node=NULL"] {
        let message =
            abort_message(db.execute(&format!("UPDATE journal_forked SET {assignment}"), []));
        assert!(
            message.contains("journal_forked_immutable"),
            "{assignment}: {message}"
        );
    }
}

#[test]
fn journal_forked_delete_aborts() {
    let db = journal_db();
    insert_forked(&db);
    let message = abort_message(db.execute("DELETE FROM journal_forked", []));
    assert!(message.contains("journal_forked_append_only"), "{message}");
}

#[test]
fn bad_node_uuid_refused() {
    let db = journal_db();
    for bad in [
        "short",
        "0123456789ABCDEF0123456789abcdef",
        "0123456789abcdef0123456789abcdeg",
    ] {
        let result = db.execute(
            "INSERT INTO sync_logs(log_ix,node_uuid,incarnation,is_self,pubkey,genesis_hash,contiguous_counter,head_hash,first_seen_at) \
             VALUES (9,?1,1,0,zeroblob(32),zeroblob(32),0,zeroblob(32),0)",
            [bad],
        );
        let message = abort_message(result);
        assert!(
            message.contains("CHECK constraint failed"),
            "{bad}: {message}"
        );
    }
}

#[test]
fn applied_without_applied_seq_refused() {
    let db = journal_db();
    let message = abort_message(insert_journal(&db, 1, "applied", None, None));
    assert!(message.contains("CHECK constraint failed"), "{message}");
    let message = abort_message(insert_journal(&db, 2, "pending", Some(7), None));
    assert!(message.contains("CHECK constraint failed"), "{message}");
}

#[test]
fn parked_dep_without_parked_on_refused() {
    let db = journal_db();
    let message = abort_message(insert_journal(&db, 1, "parked_dep", None, None));
    assert!(message.contains("CHECK constraint failed"), "{message}");
    let message = abort_message(insert_journal(&db, 2, "pending", None, Some("k")));
    assert!(message.contains("CHECK constraint failed"), "{message}");
    insert_journal(&db, 3, "parked_dep", None, Some("k")).unwrap();
}

const FOREIGN_NODE: &str = "abababababababababababababababab";

/// A fresh store with two attached host rows; the local node is the fixture
/// node.
fn seat_home_db() -> (Connection, String, i64, i64) {
    let db = fresh_db();
    let node = crate::test_support::test_node();
    let first = crate::store::connection::upsert_host(&db, "host-a", 0).unwrap();
    let second = crate::store::connection::upsert_host(&db, "host-b", 0).unwrap();
    (db, node, first, second)
}

#[allow(clippy::too_many_arguments)]
fn insert_seat(
    db: &Connection,
    id: &str,
    node: &str,
    state: &str,
    home: &str,
    is_local: i64,
    remote_kind: Option<&str>,
    host: Option<i64>,
    target: Option<&str>,
) -> rusqlite::Result<usize> {
    db.execute(
        "INSERT INTO seats(id,instance_id,state,role,generation,created_at,home_node,is_local,remote_kind,host_id,target_id,retired_at) \
         VALUES (?1,?2,?3,'native',1,0,?4,?5,?6,?7,?8,CASE WHEN ?3='retired' THEN 1 END)",
        params![id, node, state, home, is_local, remote_kind, host, target],
    )
}

// Kills: a seat flagged remote whose home is this node (locality disagreeing
// with `home_node`).
#[test]
fn remote_seat_with_local_home_aborts() {
    let (db, node, _, _) = seat_home_db();
    let error = insert_seat(
        &db,
        "r",
        &node,
        "remote",
        &node,
        0,
        Some("agent"),
        None,
        None,
    )
    .unwrap_err();
    assert!(error.to_string().contains("seat home mismatch"), "{error}");
}

// Kills: a seat flagged local whose home is another node.
#[test]
fn resolved_seat_with_foreign_home_aborts() {
    let (db, node, host, _) = seat_home_db();
    let error = insert_seat(
        &db,
        "l",
        &node,
        "resolved",
        FOREIGN_NODE,
        1,
        None,
        Some(host),
        Some("p1"),
    )
    .unwrap_err();
    assert!(error.to_string().contains("seat home mismatch"), "{error}");
}

// Kills: moving a seat to another home, or flipping its locality, after insert.
#[test]
fn home_node_update_aborts() {
    let (db, node, host, _) = seat_home_db();
    insert_seat(
        &db,
        "l",
        &node,
        "resolved",
        &node,
        1,
        None,
        Some(host),
        Some("p1"),
    )
    .unwrap();
    insert_seat(
        &db,
        "r",
        &node,
        "remote",
        FOREIGN_NODE,
        0,
        Some("human"),
        None,
        None,
    )
    .unwrap();
    for update in [
        format!("UPDATE seats SET home_node='{FOREIGN_NODE}' WHERE id='l'"),
        "UPDATE seats SET is_local=0,remote_kind='agent' WHERE id='l'".to_owned(),
        format!("UPDATE seats SET home_node='{node}' WHERE id='r'"),
        "UPDATE seats SET is_local=1,remote_kind=NULL WHERE id='r'".to_owned(),
    ] {
        let error = db.execute(&update, []).unwrap_err();
        assert!(
            error.to_string().contains("seat home mismatch"),
            "{update}: {error}"
        );
    }
    let (home, local): (String, i64) = db
        .query_row(
            "SELECT home_node,is_local FROM seats WHERE id='l'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((home, local), (node, 1));
}

// Kills: a remote row shape the baseline must accept, and the CHECKs that keep
// a remote seat free of host, target and a local state.
#[test]
fn valid_remote_seat_row_inserts() {
    let (db, node, host, _) = seat_home_db();
    insert_seat(
        &db,
        "r",
        &node,
        "remote",
        FOREIGN_NODE,
        0,
        Some("agent"),
        None,
        None,
    )
    .unwrap();
    let (state, local, kind, host_id, target): (String, i64, String, Option<i64>, Option<String>) =
        db.query_row(
            "SELECT state,is_local,remote_kind,host_id,target_id FROM seats WHERE id='r'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    assert_eq!(
        (state.as_str(), local, kind.as_str(), host_id, target),
        ("remote", 0, "agent", None, None)
    );
    // A remote seat carries no host, no target and only the remote/retired states.
    assert!(
        insert_seat(
            &db,
            "r2",
            &node,
            "remote",
            FOREIGN_NODE,
            0,
            Some("agent"),
            Some(host),
            None
        )
        .is_err()
    );
    assert!(
        insert_seat(
            &db,
            "r3",
            &node,
            "resolved",
            FOREIGN_NODE,
            0,
            Some("agent"),
            None,
            None
        )
        .is_err()
    );
    // A local seat is never remote-kinded, a remote seat always is; a target needs a host.
    assert!(
        insert_seat(
            &db,
            "r4",
            &node,
            "remote",
            FOREIGN_NODE,
            0,
            None,
            None,
            None
        )
        .is_err()
    );
    assert!(
        insert_seat(
            &db,
            "l1",
            &node,
            "resolved",
            &node,
            1,
            Some("agent"),
            Some(host),
            None
        )
        .is_err()
    );
    assert!(
        insert_seat(
            &db,
            "l2",
            &node,
            "resolved",
            &node,
            1,
            None,
            None,
            Some("p9")
        )
        .is_err()
    );
    // A local state is not a remote seat's.
    assert!(insert_seat(&db, "l3", &node, "remote", &node, 1, None, Some(host), None).is_err());
}

// Kills: a live-target index still keyed by the node alone (two hosts could not
// share a pane id) or by the target alone (one host could bind it twice).
#[test]
fn live_target_unique_per_host() {
    let (db, node, first, second) = seat_home_db();
    insert_seat(
        &db,
        "a",
        &node,
        "resolved",
        &node,
        1,
        None,
        Some(first),
        Some("p1"),
    )
    .unwrap();
    insert_seat(
        &db,
        "b",
        &node,
        "resolved",
        &node,
        1,
        None,
        Some(second),
        Some("p1"),
    )
    .unwrap();
    let error = insert_seat(
        &db,
        "c",
        &node,
        "resolved",
        &node,
        1,
        None,
        Some(first),
        Some("p1"),
    )
    .unwrap_err();
    assert!(error.to_string().contains("UNIQUE"), "{error}");
}

#[test]
fn fork_tables_verify() {
    let db = journal_db();
    schema::verify_baseline(&db).unwrap();
    db.execute(
        "INSERT INTO sync_forks(log_ix,origin,proof_counter,held_counter,held_hash,conflicting_counter,conflicting_hash,frozen_at,detected_at) \
         VALUES (2,'evidence',3,3,zeroblob(32),3,zeroblob(32),2,0)",
        [],
    )
    .unwrap();
    // origin='declared' carries no proof columns; 'evidence' requires them.
    db.execute(
        "INSERT INTO sync_forks(log_ix,origin,frozen_at,detected_at) VALUES (2,'declared',2,0)",
        [],
    )
    .unwrap();
    assert!(
        db.execute(
            "INSERT INTO sync_forks(log_ix,origin,frozen_at,detected_at) VALUES (2,'evidence',2,0)",
            [],
        )
        .is_err()
    );
    db.execute(
        "INSERT INTO sync_branches(log_ix,tip_counter,tip_hash,next_from,state,first_seen_at,updated_at) \
         VALUES (2,3,zeroblob(32),1,'pulling',0,0)",
        [],
    )
    .unwrap();
    assert!(
        db.execute(
            "INSERT INTO sync_branches(log_ix,tip_counter,tip_hash,next_from,state,first_seen_at,updated_at) \
             VALUES (2,3,zeroblob(32),1,'done',0,0)",
            [],
        )
        .is_err()
    );
    insert_journal(&db, 1, "pending", None, None).unwrap();
    db.execute(
        "INSERT INTO sync_reissues(source_log_ix,source_counter,source_hash,target_ingest_seq,reissued_at) \
         VALUES (2,3,zeroblob(32),1,0)",
        [],
    )
    .unwrap();
    let message = abort_message(db.execute("DELETE FROM sync_forks", []));
    assert!(message.contains("sync_forks_append_only"), "{message}");
    let message = abort_message(db.execute("DELETE FROM sync_reissues", []));
    assert!(message.contains("sync_reissues_append_only"), "{message}");
    let message = abort_message(db.execute("UPDATE sync_reissues SET reissued_at=1", []));
    assert!(message.contains("sync_reissues_immutable"), "{message}");
    db.execute(
        "UPDATE sync_logs SET state='frozen', frozen_at_counter=3 WHERE log_ix=2",
        [],
    )
    .unwrap();
    let message = abort_message(db.execute(
        "UPDATE sync_logs SET frozen_at_counter=4 WHERE log_ix=2",
        [],
    ));
    assert!(
        message.contains("sync_logs_freeze_lowers_only"),
        "{message}"
    );
    db.execute(
        "UPDATE sync_logs SET frozen_at_counter=2 WHERE log_ix=2",
        [],
    )
    .unwrap();
}

const PROBE_DELTA: schema::NodeDelta = schema::NodeDelta {
    version: 2,
    sql: "CREATE TABLE delta_probe(x INTEGER)",
};

fn open_with(
    db: &Connection,
    deltas: &[schema::NodeDelta],
) -> Result<(), crate::protocol::results::ApiError> {
    schema::initialize_with_deltas(
        db,
        || UtcMillis(0),
        Some(&crate::test_support::test_node_seed()),
        deltas,
        false,
    )
}

fn user_version(db: &Connection) -> i64 {
    db.pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap()
}

fn table_exists(db: &Connection, name: &str) -> bool {
    db.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
        [name],
        |r| r.get::<_, i64>(0),
    )
    .unwrap()
        == 1
}

#[test]
fn production_delta_list_is_empty_and_numbered_from_two() {
    assert!(schema::NODE_DELTAS.is_empty());
}

#[test]
fn test_delta_upgrades_baseline_store() {
    let db = Connection::open_in_memory().unwrap();
    open_with(&db, &[]).unwrap();
    assert_eq!(user_version(&db), 1);
    assert!(!table_exists(&db, "delta_probe"));
    open_with(&db, &[PROBE_DELTA]).unwrap();
    assert_eq!(user_version(&db), 2);
    assert!(table_exists(&db, "delta_probe"));
    // Applied delta is audited: a binary without it now sees a newer store.
    let detail = incompatible_detail(open_with(&db, &[]));
    assert!(detail.contains("newer than this binary"), "{detail}");
}

#[test]
fn fresh_store_applies_every_delta() {
    let db = Connection::open_in_memory().unwrap();
    open_with(&db, &[PROBE_DELTA]).unwrap();
    assert_eq!(user_version(&db), 2);
    assert!(table_exists(&db, "delta_probe"));
}

#[test]
fn second_delta_open_is_noop() {
    let db = Connection::open_in_memory().unwrap();
    open_with(&db, &[]).unwrap();
    open_with(&db, &[PROBE_DELTA]).unwrap();
    db.execute("INSERT INTO delta_probe(x) VALUES (7)", [])
        .unwrap();
    open_with(&db, &[PROBE_DELTA]).unwrap();
    assert_eq!(user_version(&db), 2);
    let rows: i64 = db
        .query_row("SELECT count(*) FROM delta_probe", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 1, "a second open must not rerun the delta");
}

#[test]
fn failing_delta_rolls_back() {
    let db = Connection::open_in_memory().unwrap();
    open_with(&db, &[]).unwrap();
    let bad = schema::NodeDelta {
        version: 2,
        sql: "CREATE TABLE a(x); CREATE TABLE a(x);",
    };
    open_with(&db, &[bad]).unwrap_err();
    assert_eq!(user_version(&db), 1);
    assert!(
        !table_exists(&db, "a"),
        "the first statement must roll back"
    );
    open_with(&db, &[]).unwrap();
}

#[test]
fn newer_store_refused_naming_versions() {
    let db = Connection::open_in_memory().unwrap();
    open_with(&db, &[]).unwrap();
    db.pragma_update(None, "user_version", 5).unwrap();
    let detail = incompatible_detail(open_with(&db, &[]));
    assert!(detail.contains("newer than this binary"), "{detail}");
    assert!(
        detail.contains("version 5") && detail.contains("newest known 1"),
        "{detail}"
    );
    assert_eq!(user_version(&db), 5, "never downgraded");
}

#[test]
fn altered_applied_delta_object_refused() {
    let db = Connection::open_in_memory().unwrap();
    open_with(&db, &[PROBE_DELTA]).unwrap();
    db.execute_batch("CREATE INDEX extra ON delta_probe(x)")
        .unwrap();
    let detail = incompatible_detail(open_with(&db, &[PROBE_DELTA]));
    assert!(detail.contains("extra"), "{detail}");
}

#[test]
fn drifted_baseline_store_is_not_upgraded() {
    let db = Connection::open_in_memory().unwrap();
    open_with(&db, &[]).unwrap();
    db.execute_batch("CREATE INDEX drift_ix ON threads(topic)")
        .unwrap();
    let detail = incompatible_detail(open_with(&db, &[PROBE_DELTA]));
    assert!(detail.contains("drift_ix"), "{detail}");
    assert_eq!(user_version(&db), 1, "audit precedes any delta");
    assert!(!table_exists(&db, "delta_probe"));
}

#[test]
fn misnumbered_delta_list_refused() {
    let db = Connection::open_in_memory().unwrap();
    let skip = schema::NodeDelta {
        version: 3,
        sql: "CREATE TABLE s(x)",
    };
    let detail = incompatible_detail(open_with(&db, &[skip]));
    assert!(detail.contains("misnumbered"), "{detail}");
}
