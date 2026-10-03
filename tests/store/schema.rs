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
    db.execute("INSERT INTO snapshot_generations(id,instance_id,host_boot,epoch,observation_sequence,incarnation,expected_targets,staged_targets,status,captured_lifecycle_revision,captured_invalidation_revision,created_at) VALUES ('g1','i','b',1,1,'inc',2,0,'building',0,0,0)", []).unwrap();
    db.execute("INSERT INTO snapshot_targets(generation_id,target_id,terminal_id,generation,observation_sequence,occupancy,ui_state,observed_at) VALUES ('g1','p1','t1',1,1,'empty_shell','idle',0)", []).unwrap();
    db.execute("INSERT INTO snapshot_targets(generation_id,target_id,terminal_id,generation,observation_sequence,occupancy,ui_state,observed_at) VALUES ('g1','p2','t2',1,1,'occupied','idle',0)", []).unwrap();
    assert!(db.execute("INSERT INTO snapshot_targets(generation_id,target_id,terminal_id,generation,observation_sequence,occupancy,ui_state,observed_at) VALUES ('g1','p3','t2',1,1,'unknown','unknown',0)", []).is_err());
    let active: Option<String> = db
        .query_row(
            "SELECT active_snapshot_id FROM host_instances WHERE id='i'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(active, None);
    assert!(
        db.execute(
            "UPDATE host_instances SET active_snapshot_id='g1' WHERE id='i'",
            []
        )
        .is_err()
    );
    db.execute(
        "UPDATE snapshot_generations SET staged_targets=2,status='published' WHERE id='g1'",
        [],
    )
    .unwrap();
    db.execute("UPDATE host_instances SET active_snapshot_id='g1',recovery_baseline_generation_id='g1',baseline_hold_unclaimed=1 WHERE id='i'", []).unwrap();
    let active: String = db
        .query_row(
            "SELECT active_snapshot_id FROM host_instances WHERE id='i'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(active, "g1");
    assert!(
        db.execute(
            "UPDATE host_instances SET active_snapshot_id='missing' WHERE id='i'",
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
    db.execute("INSERT INTO snapshot_generations(id,instance_id,host_boot,epoch,observation_sequence,incarnation,expected_targets,staged_targets,status,captured_lifecycle_revision,captured_invalidation_revision,created_at) VALUES ('prior','i','boot',1,1,'inc',0,0,'published',0,0,0)", []).unwrap();
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

fn seeded_db(utc: i64) -> (StoreContext, Connection, Arc<TestClock>) {
    let clock = Arc::new(TestClock::new(utc));
    let context = StoreContext::new(db_path(), clock.clone());
    let db = context.open_writer().unwrap();
    db.execute(
        "INSERT INTO host_instances(id, created_at) VALUES ('i', 0)",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO seats(id, instance_id, state, role, generation, created_at) VALUES ('s', 'i', 'resolved', 'native', 1, 0)", []).unwrap();
    db.execute("INSERT INTO threads(id, instance_id, topic, goal, created_at, updated_at) VALUES ('t', 'i', 'topic', 'purpose', 0, 0)", []).unwrap();
    (context, db, clock)
}

#[test]
fn v5_upgrade_adds_index_for_failed_pending_retirements() {
    let db = Connection::open_in_memory().unwrap();
    for migration in [
        include_str!("../../migrations/0001_initial.sql"),
        include_str!("../../migrations/0002_service_substrate.sql"),
        include_str!("../../migrations/0003_invitation_cancellations.sql"),
        include_str!("../../migrations/0004_voluntary_membership.sql"),
        include_str!("../../migrations/0005_service_notifications.sql"),
    ] {
        db.execute_batch(migration).unwrap();
    }
    db.pragma_update(None, "user_version", 5).unwrap();
    schema::initialize(&db).unwrap();
    assert_eq!(
        db.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        12
    );
    let plan: Vec<String> = db.prepare(
        "EXPLAIN QUERY PLAN SELECT EXISTS(SELECT 1 FROM retirements r INDEXED BY retirements_failed_pending JOIN seats s ON s.id=r.seat_id WHERE r.status='pending' AND r.last_error IS NOT NULL AND s.instance_id='i')",
    ).unwrap().query_map([], |row| row.get(3)).unwrap().collect::<Result<_, _>>().unwrap();
    assert!(
        plan.iter()
            .any(|step| step.contains("retirements_failed_pending")),
        "{plan:?}"
    );
}

fn v6_database() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    for migration in [
        include_str!("../../migrations/0001_initial.sql"),
        include_str!("../../migrations/0002_service_substrate.sql"),
        include_str!("../../migrations/0003_invitation_cancellations.sql"),
        include_str!("../../migrations/0004_voluntary_membership.sql"),
        include_str!("../../migrations/0005_service_notifications.sql"),
        include_str!("../../migrations/0006_retirement_health.sql"),
    ] {
        db.execute_batch(migration).unwrap();
    }
    db.pragma_update(None, "user_version", 6).unwrap();
    db
}

// Kills: a v6 database left without the digest indexes (the producer names
// them with INDEXED BY and would fail on every query), or a v7 migration that
// rewrites existing rows.
#[test]
fn v6_upgrade_adds_seat_and_thread_leading_digest_indexes() {
    let db = v6_database();
    db.execute_batch("INSERT INTO host_instances(id,created_at) VALUES ('i',0); INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('s','i','resolved','native',1,0); INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0); INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t','s',1,1);").unwrap();
    schema::initialize(&db).unwrap();
    assert_eq!(
        db.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        12
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM membership_intervals WHERE seat_id='s'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    let plan: Vec<String> = db
        .prepare("EXPLAIN QUERY PLAN SELECT DISTINCT thread_id FROM membership_intervals WHERE seat_id='s'")
        .unwrap()
        .query_map([], |row| row.get(3))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(
        plan.iter()
            .any(|step| step.contains("membership_intervals_seat_thread")),
        "{plan:?}"
    );
    // A second startup is a verified no-op.
    schema::initialize(&db).unwrap();
}

// Kills: accepting a v7 database whose digest index is missing or altered
// (for example a partial index missing its predicate, which would change the
// producer's plan or fail its INDEXED BY).
#[test]
fn startup_rejects_missing_or_altered_digest_index() {
    for tamper in [
        "DROP INDEX messages_thread_warning",
        "DROP INDEX send_manifests_thread_warning; CREATE INDEX send_manifests_thread_warning ON send_manifests(thread_id, base_sequence)",
    ] {
        let db = v6_database();
        schema::initialize(&db).unwrap();
        db.execute_batch(tamper).unwrap();
        let error = schema::initialize(&db).unwrap_err();
        assert_eq!(error.code, ErrorCode::IncompatibleSchema, "{tamper}");
        assert!(
            error.detail.contains("attention digest index"),
            "{}",
            error.detail
        );
    }
}

fn v7_database() -> Connection {
    let db = v6_database();
    db.execute_batch(include_str!("../../migrations/0007_attention_digest.sql"))
        .unwrap();
    db.pragma_update(None, "user_version", 7).unwrap();
    db
}

// Digest fix2: a v7 database upgrades to v8 with every pending-only projection
// backfilled from existing rows (pending rows in, settled rows out), and a
// second startup is a verified no-op. Kills: a missing 7 => upgrade arm, a
// backfill that omits pending rows or copies ACKed/accepted ones, and an
// upgrade that rewrites existing history.
#[test]
fn v7_upgrade_backfills_only_pending_rows_into_the_digest_projections() {
    let db = v7_database();
    db.execute_batch("\
        INSERT INTO host_instances(id,created_at) VALUES ('i',0);\
        INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('s','i','resolved','native',1,0);\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0);\
        INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES ('open','t','s',1,'pending',0,1,100,100);\
        INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms,accepted_at,accepted_actor_seat_id,accepted_generation,accepted_observation) VALUES ('done','t','s',2,'accepted',0,1,100,100,1,'s',1,'obs');\
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('m1','i','t',1,'ordinary','b',0,2),('m2','i','t',2,'ordinary','b',0,3);\
        INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES ('p1','i','a','1',zeroblob(32),'t',0,0,0,0,0,0,1,'sealed'),('p2','i','a','2',zeroblob(32),'t',0,0,0,0,0,0,1,'sealed');\
        INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES ('p1','t','s',1,100,1),('p2','t','s',1,100,1);\
        INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('p1','m1','i','t',2,0,1,0,1,0),('p2','m2','i','t',3,0,2,0,1,0);\
        INSERT INTO receipt_state(message_id,seat_id,state,ack_actor_seat_id,ack_generation,ack_observation,acked_at) VALUES ('m1','s','acked','s',1,'obs',1);\
    ").unwrap();
    let history = |db: &Connection| -> Vec<i64> {
        [
            "invitations",
            "messages",
            "prepared_recipients",
            "receipt_state",
        ]
        .iter()
        .map(|t| {
            db.query_row(&format!("SELECT count(*) FROM {t}"), [], |r| r.get(0))
                .unwrap()
        })
        .collect()
    };
    let before = history(&db);
    schema::initialize(&db).unwrap();
    assert_eq!(
        db.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        12
    );
    assert_eq!(history(&db), before);
    let rows = |sql: &str| -> Vec<String> {
        db.prepare(sql)
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(
        rows("SELECT invitation_id FROM digest_pending_invitations"),
        ["open"]
    );
    assert_eq!(
        rows("SELECT preparation_id FROM digest_pending_manifest_receipts"),
        ["p2"]
    );
    schema::initialize(&db).unwrap();
}

// Wave-2 fix2 (a): upgrading carries each seat's existing offer state into
// the occupant-scoped notice frontier. `s` and `o` each received notices n1
// (seq 3) and n2 (seq 5). `s`'s current occupant was offered through seq 3,
// so only n2 stays pending for it; `o`'s `warning_offer` belongs to another
// execution than its current occupant, so it settles nothing there. The
// projection is ordered by publication, whatever the recipient insertion
// order. Kills: a backfill without the frontier (n1 pending for `s`).
// `o`'s result needs occupant scoping in the backfill or in the frontier
// read; each alone suffices here, so neither single mutation fails this
// test (demonstrated). Read-time scoping is killed by
// `programmatic_notices_settle_page_by_page_for_the_current_occupant`.
#[test]
fn v6_upgrade_carries_the_current_occupant_offer_into_the_notice_frontier() {
    let db = v6_database();
    // Seed only the rows the backfill reads (the publications' preparation
    // rows are not needed for it).
    db.pragma_update(None, "foreign_keys", "OFF").unwrap();
    db.execute_batch("\
        INSERT INTO host_instances(id,created_at,decision_seq) VALUES ('i',0,10);\
        INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('s','i','resolved','native',1,0),('o','i','resolved','native',1,0);\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0);\
        INSERT INTO service_authors(id,instance_id,created_at) VALUES ('svc','i',0);\
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,decision_seq,event_json,decision_at,author_kind,author_service_id) VALUES ('n1','i','t',1,'warn',3,'{}',0,'programmatic','svc'),('n2','i','t',2,'warn',5,'{}',0,'programmatic','svc');\
        INSERT INTO service_notification_publications(preparation_id,message_id,decision_seq,recipient_count) VALUES ('p1','n1',3,2),('p2','n2',5,2);\
        INSERT INTO warning_recipients(warning_id,seat_id,generation) VALUES ('n2','s',5),('n2','o',5),('n1','s',3),('n1','o',3);\
        INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at) VALUES ('s',1,'ps','b',1,'codex','ns','exec-s','fresh',0),('o',1,'po','b',1,'codex','no','exec-o','fresh',0);\
        INSERT INTO warning_offer(seat_id,binding_generation,execution_id,offered_through_seq) VALUES ('s',1,'exec-s',3),('o',1,'predecessor',5);\
    ").unwrap();
    schema::initialize(&db).unwrap();
    let pending = |seat: &str| -> Vec<String> {
        crate::store::attention::seat_pending_warnings(&db, seat, &|| Ok(()))
            .unwrap()
            .items
            .into_iter()
            .map(|item| item.id)
            .collect()
    };
    assert_eq!(pending("s"), ["n2"]);
    assert_eq!(pending("o"), ["n2", "n1"]);
    let order: Vec<String> = db
        .prepare("SELECT warning_id FROM digest_programmatic_warnings WHERE seat_id='s' ORDER BY ordinal")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(order, ["n1", "n2"]);
}

// Digest fix2: startup verifies every v8 projection table, index and
// maintaining trigger against the migration text. Kills: accepting a database
// whose settlement trigger is missing (the projection would keep settled rows
// and the digest cost would grow with history again) or whose insertion
// trigger lost its predicate or was dropped (pending attention would never
// reach the projection and be silently hidden).
#[test]
fn startup_rejects_missing_or_altered_digest_projection() {
    for tamper in [
        "DROP TRIGGER digest_receipt_state_settled_update",
        "DROP TRIGGER digest_manifest_receipt_staged",
        "DROP INDEX digest_open_warnings_affected",
        "DROP INDEX digest_programmatic_warnings_seat",
        "DROP TRIGGER digest_programmatic_warning_projected",
        "DROP TABLE digest_notice_offer",
        "DROP TRIGGER digest_manifest_receipt_materialized",
        "DROP TRIGGER digest_open_warning_closed",
        "DROP TRIGGER digest_invitation_created; CREATE TRIGGER digest_invitation_created AFTER INSERT ON invitations BEGIN SELECT 1; END",
    ] {
        let db = v6_database();
        schema::initialize(&db).unwrap();
        db.execute_batch(tamper).unwrap();
        let error = schema::initialize(&db).unwrap_err();
        assert_eq!(error.code, ErrorCode::IncompatibleSchema, "{tamper}");
        assert!(
            error.detail.contains("attention digest projection"),
            "{tamper}: {}",
            error.detail
        );
    }
}

#[test]
fn v1_history_migrates_once_with_native_and_builtin_authors_intact() {
    let db = Connection::open_in_memory().unwrap();
    db.pragma_update(None, "foreign_keys", "ON").unwrap();
    db.execute_batch(include_str!("../../migrations/0001_initial.sql"))
        .unwrap();
    db.pragma_update(None, "user_version", 1).unwrap();
    db.execute_batch("\
        INSERT INTO host_instances(id,created_at) VALUES ('i',0);\
        INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('s','i','resolved','native',1,0);\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0);\
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,decision_seq,body,decision_at) VALUES ('native','i','t',1,'ordinary','s',1,'hello',1);\
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_key,decision_seq,event_json,decision_at) VALUES ('builtin','i','t',2,'info','old:event',2,'{}',2);\
    ").unwrap();
    schema::initialize(&db).unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        12
    );
    assert_eq!(
        db.query_row("SELECT id FROM seats", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "s"
    );
    assert_eq!(
        db.query_row("SELECT id FROM threads", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "t"
    );
    assert_eq!(
        db.query_row(
            "SELECT author_kind FROM messages WHERE id='native'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "native"
    );
    assert_eq!(
        db.query_row(
            "SELECT author_kind FROM messages WHERE id='builtin'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "built_in"
    );
    assert_eq!(
        db.query_row("SELECT managed_owner_author_id FROM threads", [], |r| {
            r.get::<_, Option<String>>(0)
        })
        .unwrap(),
        None
    );
    assert_eq!(
        crate::store::service_substrate::message_author(&db, &MessageId::new("native")).unwrap(),
        crate::protocol::service::EventAuthor::Native(SeatId::new("s"))
    );
    assert_eq!(
        crate::store::service_substrate::message_author(&db, &MessageId::new("builtin")).unwrap(),
        crate::protocol::service::EventAuthor::BuiltIn
    );
    schema::initialize(&db).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM messages", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn service_requirement_substrate_enforces_owner_link_and_one_effective_episode() {
    let (_context, mut db, _clock) = seeded_db(100);
    let tx = db.transaction().unwrap();
    let author =
        crate::store::service_substrate::ensure_reserved_author(&tx, "i", UtcMillis(100)).unwrap();
    assert_eq!(
        crate::store::service_substrate::ensure_reserved_author(&tx, "i", UtcMillis(200)).unwrap(),
        author
    );
    tx.commit().unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at) VALUES ('other',0)",
        [],
    )
    .unwrap();
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
    let author =
        crate::store::service_substrate::ensure_reserved_author(&tx, "i", UtcMillis(100)).unwrap();
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

    let service_message = "INSERT INTO messages(id,instance_id,thread_id,sequence,kind,decision_seq,event_json,decision_at,author_kind,author_service_id) VALUES (?1,'i','t',?2,'info',?3,'{}',200,?4,?5)";
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
    db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,decision_seq,body,decision_at) VALUES ('native2','i','t',2,'ordinary','s',3,'hello',201)", []).unwrap();
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
        schema::verify_existing(&db).unwrap_err().code,
        ErrorCode::IncompatibleSchema
    );
}

fn pending_requirement_fixture() -> Connection {
    let (_context, mut db, _clock) = seeded_db(100);
    let tx = db.transaction().unwrap();
    let author =
        crate::store::service_substrate::ensure_reserved_author(&tx, "i", UtcMillis(100)).unwrap();
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
    db.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('other','i','resolved','native',1,0)", []).unwrap();
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
            schema::initialize(&db).unwrap_err().code,
            ErrorCode::IncompatibleSchema
        );
        assert_eq!(
            db.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            12
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
        schema::initialize(&db).unwrap_err().code,
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
        schema::initialize(&db).unwrap();
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
            schema::initialize(&db).unwrap_err().code,
            ErrorCode::IncompatibleSchema
        );
        let after: (String, i64, Option<String>) = db.query_row("SELECT state,revision,accepted_by_seat_id FROM requirement_episodes WHERE id='req'", [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
        assert_eq!(after, before);
        assert_eq!(
            db.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            12
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
    for mask in 0..16 {
        let db = pending_requirement_fixture();
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
    assert!(db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,observed_at,provenance,incarnation) VALUES ('i','p','b',1,1,2,0,'fresh','inc')",[]).is_err());
    db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,observed_at,provenance,terminal_id,connection_epoch,incarnation,incarnation_source_kind) VALUES ('i','p','b',1,1,2,0,'fresh','term',3,'inc','native_current_target')",[]).unwrap();
    db.execute("INSERT INTO snapshot_generations(id,instance_id,host_boot,epoch,observation_sequence,incarnation,expected_targets,staged_targets,status,captured_lifecycle_revision,captured_invalidation_revision,created_at) VALUES ('g','i','b',1,2,'inc',1,1,'published',0,0,0)",[]).unwrap();
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
fn fresh_database_has_durable_settings_constraints_and_read_only_queries() {
    let path = db_path();
    let store = StoreContext::new(path.clone(), Arc::new(FixedClock));
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
        12
    );
    assert!(db.execute("INSERT INTO seats(id, instance_id, state, role, generation, created_at) VALUES ('s', 'missing', 'resolved', 'native', 1, 0)", []).is_err());
    db.execute(
        "INSERT INTO host_instances(id, created_at) VALUES ('i', 0)",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO seats(id, instance_id, state, role, target_id, generation, created_at) VALUES ('s1', 'i', 'resolved', 'native', 'pane', 1, 0)", []).unwrap();
    assert!(db.execute("INSERT INTO seats(id, instance_id, state, role, target_id, generation, created_at) VALUES ('s2', 'i', 'resolved', 'native', 'pane', 1, 0)", []).is_err());
    let query = store
        .open_query(CallBudget {
            deadline: MonoInstant(200),
            cancellation: Cancellation::default(),
        })
        .unwrap();
    assert!(
        query
            .execute(
                "INSERT INTO host_instances(id, created_at) VALUES ('i', 0)",
                []
            )
            .is_err()
    );
    drop(query);
    drop(db);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
    let _ = std::fs::remove_file(path);
}

#[test]
fn amended_v1_requires_goal_and_rejects_an_old_v1_file_without_rewriting_it() {
    let path = db_path();
    let old = Connection::open(&path).unwrap();
    old.execute_batch(
        "CREATE TABLE threads(id TEXT PRIMARY KEY, topic TEXT NOT NULL); PRAGMA user_version=1;",
    )
    .unwrap();
    old.execute(
        "INSERT INTO threads(id, topic) VALUES ('kept', 'history')",
        [],
    )
    .unwrap();
    drop(old);
    let before = std::fs::read(&path).unwrap();
    let store = StoreContext::new(path.clone(), Arc::new(FixedClock));
    assert_eq!(
        store.open_writer().err().unwrap().code,
        ErrorCode::IncompatibleSchema
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);

    let fresh = StoreContext::new(db_path(), Arc::new(FixedClock));
    let db = fresh.open_writer().unwrap();
    db.execute(
        "INSERT INTO host_instances(id, created_at) VALUES ('i', 0)",
        [],
    )
    .unwrap();
    assert!(db.execute("INSERT INTO threads(id, instance_id, topic, created_at, updated_at) VALUES ('missing-goal', 'i', 'topic', 0, 0)", []).is_err());
    db.execute("INSERT INTO threads(id, instance_id, topic, goal, created_at, updated_at) VALUES ('t', 'i', 'topic', 'purpose', 0, 0)", []).unwrap();
    let goal: String = db
        .query_row("SELECT goal FROM threads WHERE id='t'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(goal, "purpose");
}

#[test]
fn amended_v1_has_provenance_and_hidden_publication_tables() {
    let db = StoreContext::new(db_path(), Arc::new(FixedClock))
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
        ("host_instances", "decision_seq"),
        ("host_instances", "send_eligibility_revision"),
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
        .open_writer()
        .unwrap();
    db.execute_batch("DROP TABLE work_jobs").unwrap();
    assert_eq!(
        schema::verify_existing(&db).unwrap_err().code,
        ErrorCode::IncompatibleSchema
    );
}

#[test]
fn decision_and_eligibility_revisions_commit_and_rollback_together() {
    let (_, mut db, _) = seeded_db(0);
    let tx = db.transaction().unwrap();
    assert_eq!(schema::next_decision_seq(&tx, "i").unwrap(), 1);
    assert!(
        schema::apply_eligibility_transition(&tx, "i", |tx| {
            tx.execute("UPDATE seats SET generation=2 WHERE id='s'", [])
                .map_err(super::store_error)?;
            Ok(true)
        })
        .unwrap()
    );
    schema::bump_membership_revision(&tx, &ThreadId::new("t")).unwrap();
    schema::bump_timeline_revision(&tx, &ThreadId::new("t")).unwrap();
    tx.commit().unwrap();
    let revisions: (i64, i64, i64, i64) = db.query_row("SELECT h.decision_seq, h.send_eligibility_revision, t.membership_revision, t.timeline_revision FROM host_instances h JOIN threads t ON t.instance_id=h.id WHERE h.id='i'", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(revisions, (1, 1, 1, 1));
    let tx = db.transaction().unwrap();
    assert_eq!(schema::next_decision_seq(&tx, "i").unwrap(), 2);
    schema::apply_eligibility_transition(&tx, "i", |tx| {
        tx.execute("UPDATE seats SET generation=3 WHERE id='s'", [])
            .map_err(super::store_error)?;
        Ok(true)
    })
    .unwrap();
    tx.rollback().unwrap();
    let revisions: (i64, i64) = db
        .query_row(
            "SELECT decision_seq, send_eligibility_revision FROM host_instances WHERE id='i'",
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
        "UPDATE host_instances SET host_boot='b', host_epoch=1 WHERE id='i'",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance) VALUES ('i','p','b',1,1,1,'fresh')", []).unwrap();
    db.execute("INSERT INTO occupant_bindings(seat_id, generation, target_generation, target_id, host_boot, host_epoch, harness, native_session, execution_id, observation_provenance, observed_at, registered_at,terminal_id,incarnation) VALUES ('s',1,1,'p','b',1,'codex','n','e','verified',1,1,'term-'||'p','inc')", []).unwrap();
    db.execute("UPDATE seats SET unavailability_open=0 WHERE id='s'", [])
        .unwrap();
    let tx = db.transaction().unwrap();
    assert!(schema::ensure_unavailability_episode(&tx, &SeatId::new("s")).is_err());
    tx.rollback().unwrap();
    db.execute("UPDATE host_instances SET host_epoch=2 WHERE id='i'", [])
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
    db.execute("UPDATE host_instances SET host_epoch=3 WHERE id='i'", [])
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
    db.execute_batch("UPDATE seats SET target_id='p', target_generation=1, unavailability_open=0 WHERE id='s'; UPDATE host_instances SET host_boot='b', host_epoch=1 WHERE id='i'; INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance) VALUES ('i','p','b',1,1,1,'fresh'); INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES ('s',1,1,'p','b',1,'codex','n','e','verified',1,1,'term-'||'p','inc');").unwrap();
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
    db.execute_batch("UPDATE seats SET target_id='p', target_generation=1, unavailability_open=0 WHERE id='s'; UPDATE host_instances SET host_boot='b', host_epoch=1 WHERE id='i'; INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance) VALUES ('i','p','b',1,1,1,'fresh'); INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES ('s',1,1,'p','b',1,'codex','n','e','verified',1,1,'term-'||'p','inc');").unwrap();
    db.execute(
        "UPDATE host_instances SET invalidation_revision=1 WHERE id='i'",
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
            let seq = schema::next_decision_seq(tx, "i")?;
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
        "SELECT warning_message_id, (SELECT count(*) FROM messages), (SELECT count(*) FROM warning_jobs), (SELECT decision_seq FROM host_instances WHERE id='i') FROM invitations WHERE id='v1'",
        [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    ).unwrap();
    assert_eq!(empty, (None, 0, 0, 0));
    let first = context
        .execute_decision(
            &mut db,
            |_| Ok(()),
            |tx, at, _| {
                let seq = schema::next_decision_seq(tx, "i")?;
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
        db.execute("INSERT INTO messages(instance_id,id, thread_id, sequence, kind, body, decision_at,decision_seq) VALUES ('i',?1, 't', ?2, 'ordinary', 'body', 0,?2)", params![id, sequence]).unwrap();
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
    let store = StoreContext::new(path.clone(), Arc::new(FixedClock));
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
    let broken = StoreContext::new(broken_path.clone(), Arc::new(FixedClock));
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
    db.execute("INSERT INTO seats(id, instance_id, state, role, generation, created_at) VALUES ('other', 'i', 'resolved', 'native', 1, 0)", []).unwrap();
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
    db.execute("INSERT INTO threads(id, instance_id, topic, goal, created_at, updated_at) VALUES ('t2', 'i', 'topic', 'purpose', 0, 0)", []).unwrap();
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
    db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t2','i','topic','goal',0,0)", []).unwrap();
    let tx = db.transaction().unwrap();
    let seq = schema::next_decision_seq(&tx, "i").unwrap();
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
    let offsets:Vec<i64>=db.prepare("SELECT event_offset FROM messages WHERE instance_id='i' AND decision_seq=?1 ORDER BY event_offset")
        .unwrap().query_map([seq as i64],|r|r.get(0)).unwrap().map(Result::unwrap).collect();
    assert_eq!(offsets, vec![0, 1]);
}

#[test]
fn logical_message_keys_are_instance_scoped_and_thread_bound() {
    let (_, db, _) = seeded_db(0);
    db.execute(
        "INSERT INTO host_instances(id,created_at) VALUES ('other',0)",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('other-thread','other','topic','goal',0,0)", []).unwrap();
    db.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('i','m1','t',1,'ordinary','body',0,1)", []).unwrap();
    db.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('other','m2','other-thread',1,'ordinary','body',0,1)", []).unwrap();
    assert!(db.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('i','bad','other-thread',2,'ordinary','body',0,2)", []).is_err());
    assert!(db.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('i','collision','t',2,'ordinary','body',0,1)", []).is_err());
    assert!(db.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at) VALUES ('i','null-key','t',2,'ordinary','body',0)", []).is_err());
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
    db.execute("INSERT INTO messages(instance_id,id, thread_id, sequence, kind, body, decision_at,decision_seq) VALUES ('i','m', 't', 1, 'ordinary', 'original', 10,1)", []).unwrap();
    db.execute("INSERT INTO messages(instance_id,id, thread_id, sequence, kind, event_key, event_json, decision_at,decision_seq) VALUES ('i','e', 't', 2, 'info', 'key', '{}', 10,2)", []).unwrap();
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
    db.execute("INSERT INTO threads(id, instance_id, topic, goal, created_at, updated_at) VALUES ('other', 'i', 'topic', 'purpose', 0, 0)", []).unwrap();
    db.execute("INSERT INTO messages(instance_id,id, thread_id, sequence, kind, body, decision_at,decision_seq) VALUES ('i','m', 't', 1, 'ordinary', 'body', 0,1)", []).unwrap();
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
    let broken = StoreContext::new(broken_path, clock);
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
    let store = StoreContext::new(path.clone(), Arc::new(FixedClock));
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
    db.execute("INSERT INTO recovery_baseline_targets(instance_id, target_id, baseline_boot, baseline_epoch, captured_at, disposition) VALUES ('i', 'pane', 'old', 1, 10, 'held_for_repair')", []).unwrap();
    db.execute("INSERT INTO observed_targets(instance_id, target_id, host_boot, epoch, generation, observed_at, provenance) VALUES ('i', 'pane', 'old', 1, 1, 10, 'enumeration')", []).unwrap();
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
    db.execute("INSERT INTO seats(id, instance_id, state, role, generation, created_at) VALUES ('b:c', 'i', 'resolved', 'native', 1, 0)", []).unwrap();
    for (id, sequence, seat) in [("a:b", 1, "c"), ("a", 2, "b:c")] {
        if seat == "c" {
            db.execute("INSERT INTO seats(id, instance_id, state, role, generation, created_at) VALUES ('c', 'i', 'resolved', 'native', 1, 0)", []).unwrap();
        }
        db.execute("INSERT INTO messages(instance_id,id, thread_id, sequence, kind, body, decision_at,decision_seq) VALUES ('i',?1, 't', ?2, 'ordinary', 'body', 0,?2)", params![id, sequence]).unwrap();
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
        db.execute("INSERT INTO messages(instance_id,id, thread_id, sequence, kind, body, decision_at,decision_seq) VALUES ('i',?1, 't', ?2, 'ordinary', 'body', 0,?2)", params![id, sequence]).unwrap();
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
                schema::bump_filter_revision(tx, "i", "directory", "t")?;
                schema::bump_filter_revision(tx, "i", "directory", "t")?;
                schema::bump_filter_revision(tx, "i", "topic", "t")?;
                Ok(())
            },
        )
        .unwrap();
    let revisions: (i64, i64) = db.query_row("SELECT (SELECT revision FROM filter_revisions WHERE instance_id='i' AND scope_kind='directory' AND scope_key='t'), (SELECT revision FROM filter_revisions WHERE instance_id='i' AND scope_kind='topic' AND scope_key='t')", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert_eq!(revisions, (2, 1));
}

#[test]
fn v2_database_migrates_to_additive_invitation_cancellations_and_voluntary_state_without_rewriting_history()
 {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(include_str!("../../migrations/0001_initial.sql"))
        .unwrap();
    db.execute_batch(include_str!("../../migrations/0002_service_substrate.sql"))
        .unwrap();
    db.pragma_update(None, "user_version", 2).unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at) VALUES ('i',0)",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('s','i','resolved','native',1,0)",[]).unwrap();
    db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0)",[]).unwrap();
    db.execute(
        "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t','s','invited')",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at) VALUES ('old','t','s',1,'pending',1,10,100,110)",[]).unwrap();
    schema::initialize(&db).unwrap();
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        12
    );
    assert_eq!(
        db.query_row(
            "SELECT deadline_at FROM invitations WHERE id='old'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        110
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM invitation_cancellations", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
    schema::initialize(&db).unwrap();
}

#[test]
fn v4_database_adds_notification_schema_without_changing_existing_history() {
    let db = Connection::open_in_memory().unwrap();
    for migration in [
        include_str!("../../migrations/0001_initial.sql"),
        include_str!("../../migrations/0002_service_substrate.sql"),
        include_str!("../../migrations/0003_invitation_cancellations.sql"),
        include_str!("../../migrations/0004_voluntary_membership.sql"),
    ] {
        db.execute_batch(migration).unwrap();
    }
    db.pragma_update(None, "user_version", 4).unwrap();
    db.execute_batch("INSERT INTO host_instances(id,created_at) VALUES ('i',0);
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0);
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_key,decision_seq,event_json,decision_at)
            VALUES ('old','i','t',1,'info','old',1,'{\"event\":\"old\"}',0);")
        .unwrap();

    schema::initialize(&db).unwrap();
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        12
    );
    assert_eq!(
        db.query_row("SELECT event_json FROM messages WHERE id='old'", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        "{\"event\":\"old\"}"
    );
    for table in [
        "service_notification_preparations",
        "service_notification_recipients",
        "service_notification_publications",
    ] {
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [table],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }
    schema::initialize(&db).unwrap();
}

#[test]
fn v5_startup_rejects_missing_or_weakened_notification_objects_without_mutation() {
    let canonical = include_str!("../../migrations/0005_service_notifications.sql");
    let cases = [
        (
            "missing required ordinal index",
            "CREATE INDEX requirement_episodes_thread_ordinal\n    ON requirement_episodes(thread_id,ordinal);",
            "",
        ),
        (
            "wrong required ordinal index order",
            "ON requirement_episodes(thread_id,ordinal)",
            "ON requirement_episodes(ordinal,thread_id)",
        ),
        (
            "wrong required ordinal index collation",
            "ON requirement_episodes(thread_id,ordinal)",
            "ON requirement_episodes(thread_id COLLATE NOCASE,ordinal)",
        ),
        (
            "missing paging index",
            "CREATE INDEX service_notification_recipients_page\n    ON service_notification_recipients(preparation_id,ordinal);",
            "",
        ),
        (
            "wrong paging order",
            "ON service_notification_recipients(preparation_id,ordinal)",
            "ON service_notification_recipients(ordinal,preparation_id)",
        ),
        (
            "wrong paging collation",
            "ON service_notification_recipients(preparation_id,ordinal)",
            "ON service_notification_recipients(preparation_id COLLATE NOCASE,ordinal)",
        ),
        (
            "weakened preparation key",
            "    id TEXT NOT NULL UNIQUE,\n    instance_id",
            "    id TEXT NOT NULL,\n    instance_id",
        ),
        (
            "weakened preparation check",
            "digest BLOB NOT NULL CHECK(length(digest)=32)",
            "digest BLOB NOT NULL",
        ),
        (
            "changed case-sensitive status literal",
            "status IN ('building','sealed','discarded','published')",
            "status IN ('BUILDING','sealed','discarded','published')",
        ),
        (
            "weakened recipient uniqueness",
            "UNIQUE(preparation_id,seat_id)",
            "UNIQUE(preparation_id,ordinal)",
        ),
        (
            "weakened recipient foreign key",
            "seat_id TEXT NOT NULL REFERENCES seats(id)",
            "seat_id TEXT NOT NULL",
        ),
        (
            "weakened publication key",
            "message_id TEXT NOT NULL UNIQUE REFERENCES messages(id)",
            "message_id TEXT NOT NULL REFERENCES messages(id)",
        ),
        (
            "weakened publication check",
            "decision_seq INTEGER NOT NULL CHECK(decision_seq>0)",
            "decision_seq INTEGER NOT NULL",
        ),
        (
            "wrong operation index",
            "ON service_notification_preparations(instance_id,author_id,operation_key,ordinal)",
            "ON service_notification_preparations(instance_id,operation_key,author_id,ordinal)",
        ),
    ];
    for (name, original, replacement) in cases {
        assert_eq!(canonical.matches(original).count(), 1, "{name}");
        let db = Connection::open_in_memory().unwrap();
        for migration in [
            include_str!("../../migrations/0001_initial.sql"),
            include_str!("../../migrations/0002_service_substrate.sql"),
            include_str!("../../migrations/0003_invitation_cancellations.sql"),
            include_str!("../../migrations/0004_voluntary_membership.sql"),
        ] {
            db.execute_batch(migration).unwrap();
        }
        db.execute_batch(&canonical.replacen(original, replacement, 1))
            .unwrap();
        db.pragma_update(None, "user_version", 5).unwrap();
        let before: Vec<(String, String)> = db
            .prepare("SELECT name,coalesce(sql,'') FROM sqlite_master ORDER BY name")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            schema::initialize(&db).unwrap_err().code,
            ErrorCode::IncompatibleSchema,
            "{name}"
        );
        let after: Vec<(String, String)> = db
            .prepare("SELECT name,coalesce(sql,'') FROM sqlite_master ORDER BY name")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(after, before, "{name}");
        assert_eq!(
            db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            5,
            "{name}"
        );
    }
}

#[test]
fn v3_required_only_shadow_recovers_prior_left_only_from_exact_leave_audit() {
    fn legacy(with_leave_audit: bool) -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(include_str!("../../migrations/0001_initial.sql"))
            .unwrap();
        db.execute_batch(include_str!("../../migrations/0002_service_substrate.sql"))
            .unwrap();
        db.execute_batch(include_str!(
            "../../migrations/0003_invitation_cancellations.sql"
        ))
        .unwrap();
        db.pragma_update(None, "user_version", 3).unwrap();
        db.execute_batch("INSERT INTO host_instances(id,created_at) VALUES ('i',0);
            INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('s','i','resolved','native',1,0);
            INSERT INTO service_authors(id,instance_id,created_at) VALUES ('graph:i','i',0);
            INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,managed_owner_author_id)
                VALUES ('t','i','topic','goal',0,0,'graph:i');
            INSERT INTO memberships(thread_id,seat_id,episode,state,left_at) VALUES ('t','s',2,'invited',NULL);
            INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq,left_seq) VALUES ('t','s',1,1,2);
            INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at)
                VALUES ('required','t','s',2,'pending',3,30,100,130);
            INSERT INTO requirement_episodes(id,thread_id,seat_id,issuer_author_id,invitation_id,state,created_decision_seq,created_at)
                VALUES ('req','t','s','graph:i','required','pending',3,30);").unwrap();
        if with_leave_audit {
            db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_key,decision_seq,event_json,decision_at)
                VALUES ('leave','i','t',1,'info','leave:1',2,'{\"action\":\"leave\",\"seat\":\"s\"}',20)",[]).unwrap();
        }
        db
    }
    let missing = legacy(false);
    assert_eq!(
        schema::initialize(&missing).unwrap_err().code,
        ErrorCode::IncompatibleSchema
    );
    assert_eq!(
        missing
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
    assert_eq!(
        missing
            .query_row("SELECT state FROM memberships", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "invited"
    );
    let recovered = legacy(true);
    schema::initialize(&recovered).unwrap();
    assert_eq!(
        recovered
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        12
    );
    assert_eq!(
        recovered
            .query_row("SELECT voluntary_state,left_at FROM memberships", [], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })
            .unwrap(),
        ("left".into(), 20)
    );
    assert_eq!(
        recovered
            .query_row("SELECT deadline_at FROM invitations", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        130
    );
    schema::initialize(&recovered).unwrap();
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
    let context = StoreContext::new(path.clone(), clock.clone());
    drop(context.open_writer().unwrap());
    {
        // A pre-invariant store: raw connection, no guard installed.
        let db = Connection::open(&path).unwrap();
        db.execute(
            "INSERT INTO host_instances(id,created_at,host_boot,host_epoch) VALUES ('i',0,'b',1)",
            [],
        )
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
                &format!("INSERT INTO seats(id,instance_id,state,unresolved_reason,role,target_id,generation,target_generation,created_at) VALUES ('{seat}','i','{state}',{reason},'native','p-{seat}',2,1,0)"),
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

fn v8_database() -> Connection {
    let db = v7_database();
    db.execute_batch(include_str!(
        "../../migrations/0008_digest_pending_paths.sql"
    ))
    .unwrap();
    db.pragma_update(None, "user_version", 8).unwrap();
    db
}

// ht-4is.8.9: a v8 database upgrades to v9 by rebuilding occupant_bindings
// with identical rows, ordinals, AUTOINCREMENT high water and indexes; only
// the accepted harness set grows to include a person's `human` occupant.
// Kills: a missing 8 => upgrade arm, a rebuild that drops or renumbers
// history, loses an index, or still refuses the human harness.
#[test]
fn v8_upgrade_rebuilds_occupant_bindings_to_accept_a_human_occupant() {
    let db = v8_database();
    db.execute_batch("\
        INSERT INTO host_instances(id,created_at) VALUES ('i',0);\
        INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('s','i','resolved','native',2,0);\
        INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,ended_at,terminal_id,incarnation) VALUES ('s',1,'w1:p1','b',0,'claude','n1','e1','cooperative_top_level',1,1,5,'t','inc');\
        INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES ('s',2,'w1:p1','b',0,'codex','n2','e2','cooperative_top_level',6,6,'t','inc');\
    ").unwrap();
    assert!(
        db.execute(
            "UPDATE occupant_bindings SET harness='human' WHERE generation=2",
            []
        )
        .is_err(),
        "v8 refuses the human harness"
    );
    let rows = |db: &Connection| -> Vec<(i64, String, String, Option<i64>)> {
        db.prepare(
            "SELECT ordinal,harness,execution_id,ended_at FROM occupant_bindings ORDER BY ordinal",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
    };
    let before = rows(&db);
    schema::initialize(&db).unwrap();
    assert_eq!(
        db.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        12
    );
    assert_eq!(rows(&db), before);
    for index in [
        "occupant_bindings_current",
        "occupant_bindings_history",
        "occupant_bindings_execution",
    ] {
        let exists: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='index' AND name=?1 AND tbl_name='occupant_bindings')",
                [index],
                |r| r.get(0),
            )
            .unwrap();
        assert!(exists, "{index}");
    }
    db.execute(
        "UPDATE occupant_bindings SET ended_at=7 WHERE generation=2",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES ('s',3,'w1:p1','b',0,'human','plugin_context:x','e3','operator_human',8,8,'t','inc')", [])
        .unwrap();
    let ordinal: i64 = db
        .query_row(
            "SELECT ordinal FROM occupant_bindings WHERE generation=3",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        ordinal, 3,
        "AUTOINCREMENT continues past the copied history"
    );
    assert!(
        db.execute(
            "UPDATE occupant_bindings SET harness='robot' WHERE generation=3",
            []
        )
        .is_err()
    );
    // A second startup is a verified no-op.
    schema::initialize(&db).unwrap();
}

fn v9_database() -> Connection {
    let db = v8_database();
    db.execute_batch(include_str!("../../migrations/0009_human_occupant.sql"))
        .unwrap();
    db.pragma_update(None, "user_version", 9).unwrap();
    db
}

/// Tables dropped by the v11 migration (B4; v10 is B5's trust guards): those only the deleted pre-cooperative
/// verification layer used. The mechanical sweep (`rg -w <table> src/`) found
/// none: allocation_decisions, recovery_baseline_releases and
/// recovery_baseline_targets are still written or read by ordinary resolution,
/// operator repair and the effective recovery disposition.
const DROPPED_IN_V11: &[&str] = &[];

// ht-p03.2: a v9 database upgrades (through B5's v10) to v11 by dropping only
// the tables the removed verification layer used; cooperative tables and rows
// are intact. Kills: a missing 9 => upgrade arm, a v11 not stamped as version 11,
// and a migration that drops or alters a cooperative table or its rows.
#[test]
fn v9_upgrade_to_v11_drops_removed_only_tables() {
    let db = v9_database();
    db.execute_batch("\
        INSERT INTO host_instances(id,created_at) VALUES ('i',0);\
        INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('s','i','resolved','native',1,0);\
        INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES ('s',1,'w1:p1','b',0,'claude','n1','e1','cooperative_top_level',1,1,'t','inc');\
    ").unwrap();
    schema::initialize(&db).unwrap();
    let version: i64 = db
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, 12);
    for dropped in DROPPED_IN_V11 {
        let exists: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name=?1)",
                [dropped],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!exists, "{dropped} still present");
    }
    for kept in [
        "seats",
        "occupant_bindings",
        "threads",
        "messages",
        "receipts",
        "recovery_holds",
        "wake_work",
        "work_jobs",
        "allocation_decisions",
        "recovery_baseline_releases",
        "recovery_baseline_targets",
    ] {
        let exists: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                [kept],
                |r| r.get(0),
            )
            .unwrap();
        assert!(exists, "{kept} dropped");
    }
    let rows: i64 = db
        .query_row("SELECT count(*) FROM occupant_bindings", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 1);
    schema::initialize(&db).unwrap(); // second startup is a verified no-op
}

/// A v11 store (the previous release's shape) with rows in existing tables.
fn v11_populated_database() -> Connection {
    let db = v9_database();
    db.execute_batch("\
        INSERT INTO host_instances(id,created_at) VALUES ('i',0);\
        INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('s','i','resolved','native',1,0);\
        INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES ('s',1,'w1:p1','b',0,'claude','n1','e1','cooperative_top_level',1,1,'t','inc');\
    ").unwrap();
    schema::initialize(&db).unwrap();
    db.execute_batch(
        "DROP INDEX harness_version_evidence_seen; DROP TABLE harness_version_evidence; DROP TABLE harness_unattributed; PRAGMA user_version=11;",
    )
    .unwrap();
    db
}

// ht-xoc.4: a populated v11 store upgrades to v12 keeping its rows and gaining
// both evidence tables (empty, usable); a fresh store lands at v12 with them.
// Kills: a missing 11 => upgrade arm, a v12 not stamped as 12, a migration that
// loses existing rows, and an audit that does not check the new tables.
#[test]
fn migration_applies_on_a_populated_store() {
    let db = v11_populated_database();
    schema::initialize(&db).unwrap();
    let version: i64 = db
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, 12);
    let bindings: i64 = db
        .query_row("SELECT count(*) FROM occupant_bindings", [], |r| r.get(0))
        .unwrap();
    assert_eq!(bindings, 1, "existing rows survive");
    for table in ["harness_version_evidence", "harness_unattributed"] {
        let rows: i64 = db
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0, "{table} starts empty");
    }
    db.execute(
        "INSERT INTO harness_version_evidence(harness,version,contract_id,first_seen_at,last_seen_at) VALUES ('claude','2.1.286','0123456789abcdef',1,1)",
        [],
    )
    .unwrap();
    assert!(
        db.execute(
            "INSERT INTO harness_version_evidence(harness,version,contract_id,first_seen_at,last_seen_at) VALUES ('human','1.0.0','0123456789abcdef',1,1)",
            [],
        )
        .is_err(),
        "harness is constrained to claude and codex"
    );
    schema::initialize(&db).unwrap(); // second startup is a verified no-op
}

#[test]
fn fresh_store_lands_at_v12_with_the_evidence_tables() {
    let db = Connection::open_in_memory().unwrap();
    schema::initialize(&db).unwrap();
    let version: i64 = db
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, 12);
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
    let db = Connection::open_in_memory().unwrap();
    schema::initialize(&db).unwrap();
    db.execute_batch("DROP TABLE harness_unattributed").unwrap();
    assert_eq!(
        schema::verify_existing(&db).unwrap_err().code,
        ErrorCode::IncompatibleSchema
    );
    db.execute_batch(
        "CREATE TABLE harness_unattributed(harness TEXT PRIMARY KEY, reason TEXT NOT NULL, at INTEGER NOT NULL) WITHOUT ROWID",
    )
    .unwrap();
    assert_eq!(
        schema::verify_existing(&db).unwrap_err().code,
        ErrorCode::IncompatibleSchema,
        "a table without the harness CHECK is not the migration's table"
    );
}

const B1_V11: &[(&str, &str)] = &[
    (
        "seats_live_ordinal",
        "CREATE INDEX seats_live_ordinal ON seats(instance_id, ordinal) WHERE state!='retired'",
    ),
    (
        "work_jobs_live",
        "CREATE INDEX work_jobs_live ON work_jobs(ordinal) WHERE status IN ('pending','failed')",
    ),
    (
        "work_jobs_retention",
        "CREATE INDEX work_jobs_retention ON work_jobs(kind, completed_at) WHERE status='complete'",
    ),
    (
        "snapshot_generations_retention",
        "CREATE INDEX snapshot_generations_retention ON snapshot_generations(instance_id, admission_sequence)",
    ),
    (
        "wake_work_reserved",
        "CREATE INDEX wake_work_reserved ON wake_work(seat_id) WHERE reservation_id IS NOT NULL",
    ),
];

fn normalize_sql(sql: &str) -> String {
    sql.trim()
        .trim_end_matches(';')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

// ht-p03.12.1: a v9 database gains the five B1 indexes with exactly the
// contract SQL and work_jobs.completed_at; pre-v11 completed rows keep NULL.
// Kills: a missing or reworded index, a missing column, a backfilled stamp.
#[test]
fn v9_upgrade_creates_b1_indexes_and_completed_at() {
    let db = v9_database();
    db.execute_batch("INSERT INTO work_jobs(id,kind,subject_id,high_water,status) VALUES ('done','send_attention','p1',1,'complete'),('todo','send_attention','p2',1,'pending');").unwrap();
    schema::initialize(&db).unwrap();
    for (name, sql) in B1_V11 {
        let installed: String = db
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='index' AND name=?1",
                [name],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(normalize_sql(&installed), normalize_sql(sql), "{name}");
    }
    let has_column: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('work_jobs') WHERE name='completed_at')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(has_column);
    let stamped: i64 = db
        .query_row(
            "SELECT count(*) FROM work_jobs WHERE completed_at IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stamped, 0);
    assert!(
        db.execute("UPDATE work_jobs SET completed_at=-1 WHERE id='done'", [])
            .is_err(),
        "negative completed_at violates the CHECK"
    );
}

#[test]
fn startup_verification_rejects_a_tampered_b1_index() {
    let db = v9_database();
    schema::initialize(&db).unwrap();
    db.execute_batch("DROP INDEX work_jobs_live; CREATE INDEX work_jobs_live ON work_jobs(ordinal) WHERE status='pending';").unwrap();
    let error = schema::initialize(&db).unwrap_err();
    assert_eq!(error.code, ErrorCode::IncompatibleSchema);
    assert!(error.detail.contains("work_jobs_live"), "{}", error.detail);
}

// Every index is usable by name with its own predicate (INDEXED BY fails to
// prepare when the index is missing or cannot serve the probe), including the
// seven access paths ht-p03.12.5 relies on.
#[test]
fn b1_indexes_are_usable_by_name() {
    let db = v9_database();
    schema::initialize(&db).unwrap();
    for probe in [
        "SELECT ordinal FROM seats INDEXED BY seats_live_ordinal WHERE instance_id=?1 AND state!='retired' AND ordinal>?2 ORDER BY ordinal LIMIT 1",
        "SELECT ordinal FROM work_jobs INDEXED BY work_jobs_live WHERE status IN ('pending','failed') AND ordinal>?1 ORDER BY ordinal LIMIT 1",
        "SELECT ordinal FROM work_jobs INDEXED BY work_jobs_retention WHERE status='complete' AND kind=?1 AND completed_at<?2 ORDER BY completed_at LIMIT 1",
        "SELECT admission_sequence FROM snapshot_generations INDEXED BY snapshot_generations_retention WHERE instance_id=?1 AND admission_sequence<?2 ORDER BY admission_sequence LIMIT 1",
        "SELECT seat_id FROM wake_work INDEXED BY wake_work_reserved WHERE reservation_id IS NOT NULL AND seat_id>?1 LIMIT 1",
        "SELECT ordinal FROM digest_pending_invitations INDEXED BY digest_pending_invitations_seat WHERE seat_id=?1 AND created_decision_seq>?2 ORDER BY created_decision_seq, ordinal LIMIT 1",
        "SELECT ordinal FROM digest_pending_manifest_receipts INDEXED BY digest_pending_manifest_receipts_seat WHERE seat_id=?1 AND decision_seq>?2 ORDER BY decision_seq, ordinal LIMIT 1",
        "SELECT ordinal FROM receipts INDEXED BY receipts_seat_state_ordinal WHERE seat_id=?1 AND state=?2 AND ordinal>?3 ORDER BY ordinal LIMIT 1",
        "SELECT source_ordinal FROM digest_open_warning_recipients INDEXED BY digest_open_warning_recipients_seat WHERE seat_id=?1 AND source=?2 AND source_ordinal>?3 ORDER BY source_ordinal LIMIT 1",
        "SELECT source_ordinal FROM digest_open_warnings INDEXED BY digest_open_warnings_affected WHERE affected_seat_id=?1 AND source=?2 AND source_ordinal>?3 ORDER BY source_ordinal LIMIT 1",
        "SELECT ordinal FROM digest_programmatic_warnings INDEXED BY digest_programmatic_warnings_seat WHERE seat_id=?1 AND ordinal>?2 ORDER BY ordinal LIMIT 1",
        "SELECT ordinal FROM occupant_bindings INDEXED BY occupant_bindings_current WHERE seat_id=?1 AND ended_at IS NULL",
    ] {
        db.prepare(probe).unwrap_or_else(|e| panic!("{probe}: {e}"));
    }
}

// ht-p03.12.6 pin criterion: after v11 the baseline and recovery tables below
// all still exist (ht-p03.2 dropped no table; b4-removed-symbols.txt).
#[test]
fn v11_survivors_recorded() {
    let db = v9_database();
    schema::initialize(&db).unwrap();
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

// B5 (ht-rzi.1): a populated v9 store upgrades through v10 (rebuilding
// allocation_decisions) to the current v11. Kills: a rebuild that renumbers or drops history,
// loses an index, or seeds the new marker/diagnostic columns with non-NULL.
#[test]
fn v9_store_with_rows_migrates_to_v10_preserving_allocation_history() {
    let db = v9_database();
    db.execute_batch("\
        INSERT INTO host_instances(id,created_at) VALUES ('i',0);\
        INSERT INTO allocation_decisions(ordinal,instance_id,target_id,seat_id,kind,decided_at,host_boot,epoch,generation,operator_label) VALUES (3,'i','w1:p1','s1','ordinary',10,'b',0,1,NULL);\
        INSERT INTO allocation_decisions(ordinal,instance_id,target_id,seat_id,kind,decided_at,host_boot,epoch,generation,operator_label) VALUES (7,'i','w1:p2','s2','operator_rebind',11,'b',1,2,'operator:local-user:501');\
    ").unwrap();
    let rows = |db: &Connection| -> Vec<(i64, String, String, String, i64, Option<String>)> {
        db.prepare("SELECT ordinal,target_id,seat_id,kind,epoch,operator_label FROM allocation_decisions ORDER BY ordinal")
            .unwrap()
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    let before = rows(&db);
    assert_eq!(before.len(), 2);
    schema::initialize(&db).unwrap();
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        12
    );
    assert_eq!(rows(&db), before);
    let diagnostics: i64 = db
        .query_row(
            "SELECT count(*) FROM allocation_decisions WHERE continuity_diagnostic IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(diagnostics, 0);
    let marker: (Option<String>, Option<i64>) = db
        .query_row(
            "SELECT reconciled_boot,reconciled_epoch FROM host_instances WHERE id='i'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(marker, (None, None));
    for index in [
        "allocation_decisions_target",
        "allocation_decisions_seat_history",
    ] {
        let exists: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='index' AND name=?1 AND tbl_name='allocation_decisions')",
                [index],
                |r| r.get(0),
            )
            .unwrap();
        assert!(exists, "{index}");
    }
    db.execute("INSERT INTO allocation_decisions(instance_id,target_id,seat_id,kind,decided_at,host_boot,epoch,generation) VALUES ('i','w1:p3','s3','ordinary',12,'b',0,1)", [])
        .unwrap();
    let next: i64 = db
        .query_row("SELECT max(ordinal) FROM allocation_decisions", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(next, 8, "AUTOINCREMENT continues past the copied history");
    schema::initialize(&db).unwrap();
}

#[test]
fn v10_allocation_decisions_accepts_b5_kinds() {
    let db = v9_database();
    schema::initialize(&db).unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at) VALUES ('i',0)",
        [],
    )
    .unwrap();
    let insert = |kind: &str, diagnostic: Option<&str>| {
        db.execute(
            "INSERT INTO allocation_decisions(instance_id,target_id,seat_id,kind,decided_at,host_boot,epoch,generation,continuity_diagnostic) VALUES ('i','t','s',?1,1,'b',0,1,?2)",
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

/// A store at main's v10 (B5 trust guards, before the B4/B1 cooperative-only
/// migration was renumbered to v11).
fn v10_database() -> Connection {
    let db = v9_database();
    db.execute_batch(include_str!("../../migrations/0010_b5_trust_guards.sql"))
        .unwrap();
    db.pragma_update(None, "user_version", 10).unwrap();
    db
}

// Merge of main (B5, v10) into the remaining-findings run (B1/B4, renumbered
// to v11): a populated main-v10 store upgrades to v11 keeping every B5 column,
// kind and row, and gains exactly the B1 indexes and work_jobs.completed_at.
// Kills: a missing 10 => upgrade arm, a v11 that re-runs or skips the B5
// migration, a v10 store verified against the v11 shape before upgrading, and
// a second startup that is not a verified no-op.
#[test]
fn main_v10_store_upgrades_to_v11_with_both_migrations() {
    let db = v10_database();
    db.execute_batch("\
        INSERT INTO host_instances(id,created_at,reconciled_boot,reconciled_epoch) VALUES ('i',0,'b',3);\
        INSERT INTO allocation_decisions(ordinal,instance_id,target_id,seat_id,kind,decided_at,host_boot,epoch,generation,continuity_diagnostic) VALUES (4,'i','w1:p1','s1','cooperative_continuity',10,'b',3,2,'match');\
        INSERT INTO work_jobs(id,kind,subject_id,high_water,status) VALUES ('done','send_attention','p1',1,'complete');\
    ").unwrap();
    schema::initialize(&db).unwrap();
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        12
    );
    let marker: (String, i64) = db
        .query_row(
            "SELECT reconciled_boot,reconciled_epoch FROM host_instances WHERE id='i'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(marker, ("b".to_owned(), 3));
    let decision: (i64, String, Option<String>) = db
        .query_row(
            "SELECT ordinal,kind,continuity_diagnostic FROM allocation_decisions",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        decision,
        (
            4,
            "cooperative_continuity".to_owned(),
            Some("match".to_owned())
        )
    );
    for (name, sql) in B1_V11 {
        let installed: String = db
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='index' AND name=?1",
                [name],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(normalize_sql(&installed), normalize_sql(sql), "{name}");
    }
    let stamped: i64 = db
        .query_row(
            "SELECT count(*) FROM work_jobs WHERE completed_at IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stamped, 0, "pre-v11 completed rows keep NULL");
    schema::initialize(&db).unwrap(); // second startup is a verified no-op
}

// A fresh store and a main-v10 store end in the same v11 shape.
// Kills: a fresh path that skips either migration.
#[test]
fn fresh_and_main_v10_stores_share_the_v11_shape() {
    let shape = |db: &Connection| -> Vec<(String, String, Option<String>)> {
        db.prepare(
            "SELECT type,name,sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY type,name",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
    };
    let fresh = Connection::open_in_memory().unwrap();
    schema::initialize(&fresh).unwrap();
    let upgraded = v10_database();
    schema::initialize(&upgraded).unwrap();
    for db in [&fresh, &upgraded] {
        assert_eq!(
            db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                .unwrap(),
            12
        );
    }
    let normalize =
        |rows: Vec<(String, String, Option<String>)>| -> Vec<(String, String, Option<String>)> {
            rows.into_iter()
                .map(|(t, n, sql)| (t, n, sql.map(|sql| normalize_sql(&sql))))
                .collect()
        };
    assert_eq!(normalize(shape(&fresh)), normalize(shape(&upgraded)));
}
