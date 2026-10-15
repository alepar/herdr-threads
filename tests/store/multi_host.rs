//! One store with two attached Herdr servers (ht-5fr.2.4): target ids are unique
//! only within one server, so the seats, observation rows and recovery rows of
//! each server are keyed by `host_id`, and a coherent snapshot of one server
//! never touches another server's seats.

use herdr_threads::{
    ports::{
        EnumerationEvidence, EvidenceKind, GuardedSeatTransition, HostId, HostSnapshot,
        IncarnationEvidence, ReconciliationAction, ReconciliationOutcome, SnapshotHeader,
        StorePort,
    },
    protocol::{
        ids::{HostBootId, HostLocator, HostTargetId, SeatId},
        time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis},
    },
    store::{SqliteStore, StoreSettings, connection::StoreContext},
};
use rusqlite::{Connection, params};
use std::sync::Arc;

const NODE: &str = "34750f98bd59fcfc946da45aaabe933b";

struct Fixed;
impl Clock for Fixed {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(100)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(100)
    }
}

struct Directory(std::path::PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct TwoHosts {
    store: SqliteStore,
    db: Connection,
    h1: HostId,
    h2: HostId,
    _directory: Directory,
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Cancellation::default(),
    }
}

fn two_hosts() -> TwoHosts {
    let directory =
        Directory(std::env::temp_dir().join(format!("multi-host-{}", uuid::Uuid::new_v4())));
    std::fs::create_dir(&directory.0).unwrap();
    let path = directory.0.join("store.db");
    let store = SqliteStore::new(
        StoreContext::new(path.clone(), Arc::new(Fixed)),
        NODE,
        StoreSettings::default(),
    )
    .unwrap();
    let attach = |socket: &str| {
        store
            .attach_host(&HostLocator::parse(socket).unwrap(), 0, &budget())
            .unwrap()
            .0
    };
    let h1 = attach("/tmp/ht-multi-host-a.sock");
    let h2 = attach("/tmp/ht-multi-host-b.sock");
    assert_ne!(h1, h2, "two servers, two host rows");
    let db = Connection::open(&path).unwrap();
    db.pragma_update(None, "foreign_keys", "ON").unwrap();
    TwoHosts {
        store,
        db,
        h1,
        h2,
        _directory: directory,
    }
}

/// The `herdr_hosts.host_id` of the server at `socket`.
fn host_row(db: &Connection, socket: &str) -> i64 {
    db.query_row(
        "SELECT host_id FROM herdr_hosts WHERE endpoint=?1",
        [socket],
        |r| r.get(0),
    )
    .unwrap()
}

fn insert_resolved(db: &Connection, seat: &str, host: i64, target: &str) {
    db.execute(
        "INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) \
         VALUES (?1,?2,'resolved','native',?3,1,1,0,?2,1,?4)",
        params![seat, NODE, target, host],
    )
    .unwrap();
}

/// Everything a snapshot of the other server must leave alone: the server's
/// seat, its recovery hold and baseline rows, its observed target and its host
/// row.
fn untouched_by_other_host(db: &Connection, host: i64) -> String {
    let rows = |sql: &str| -> Vec<String> {
        let mut statement = db.prepare(sql).unwrap();
        let columns = statement.column_count();
        statement
            .query_map([host], |r| {
                (0..columns)
                    .map(|i| {
                        r.get::<_, rusqlite::types::Value>(i)
                            .map(|v| format!("{v:?}"))
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map(|cells| cells.join("|"))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    [
        rows("SELECT id,state,unresolved_reason,target_id,generation,target_generation,host_id FROM seats WHERE host_id=?1 ORDER BY id"),
        rows("SELECT target_id,baseline_boot,baseline_epoch,reason,released_at FROM recovery_holds WHERE host_id=?1 ORDER BY target_id"),
        rows("SELECT target_id,baseline_boot,baseline_epoch,disposition FROM recovery_baseline_targets WHERE host_id=?1 ORDER BY target_id"),
        rows("SELECT target_id,host_boot,epoch,generation,observation_sequence,provenance FROM observed_targets WHERE host_id=?1 ORDER BY target_id"),
        rows("SELECT host_id,active_snapshot_id,host_boot,host_epoch,lifecycle_revision,invalidation_revision,baseline_hold_unclaimed,reconciled_boot FROM herdr_hosts WHERE host_id=?1"),
    ]
    .iter()
    .map(|section| section.join("\n"))
    .collect::<Vec<_>>()
    .join("\n--\n")
}

#[test]
fn same_target_on_two_hosts_coexists() {
    let f = two_hosts();
    insert_resolved(
        &f.db,
        "s-h1",
        host_row(&f.db, "/tmp/ht-multi-host-a.sock"),
        "t1",
    );
    insert_resolved(
        &f.db,
        "s-h2",
        host_row(&f.db, "/tmp/ht-multi-host-b.sock"),
        "t1",
    );
    let resolved: Vec<(String, i64)> =
        f.db.prepare(
            "SELECT id,host_id FROM seats WHERE target_id='t1' AND state='resolved' ORDER BY id",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        resolved,
        vec![
            (
                "s-h1".to_owned(),
                host_row(&f.db, "/tmp/ht-multi-host-a.sock")
            ),
            (
                "s-h2".to_owned(),
                host_row(&f.db, "/tmp/ht-multi-host-b.sock")
            ),
        ]
    );
    // The same target on the SAME server is still one owner.
    let error = f
        .db
        .execute(
            "INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) \
             VALUES ('s-dup',?1,'resolved','native','t1',1,1,0,?1,1,?2)",
            params![NODE, host_row(&f.db, "/tmp/ht-multi-host-a.sock")],
        )
        .unwrap_err();
    assert!(error.to_string().contains("UNIQUE"), "{error}");
    // Observation rows follow the same key.
    for socket in ["/tmp/ht-multi-host-a.sock", "/tmp/ht-multi-host-b.sock"] {
        f.db.execute(
            "INSERT INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observed_at,provenance) VALUES (?1,'t1','b',1,1,0,'fresh')",
            [host_row(&f.db, socket)],
        )
        .unwrap();
    }
    let observed: i64 =
        f.db.query_row(
            "SELECT count(*) FROM observed_targets WHERE target_id='t1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        observed, 2,
        "one observed row per server for the same target id"
    );
}

#[test]
fn snapshot_of_h1_unresolves_only_h1_seat() {
    let f = two_hosts();
    let (a, b) = (
        host_row(&f.db, "/tmp/ht-multi-host-a.sock"),
        host_row(&f.db, "/tmp/ht-multi-host-b.sock"),
    );
    // The second server's seat has the LOWER ordinal, inside any high-water mark
    // of the first server's page.
    insert_resolved(&f.db, "s-h2", b, "t1");
    insert_resolved(&f.db, "s-h1", a, "t1");
    // The second server's recovery and baseline rows for the same target id.
    f.db.execute(
        "INSERT INTO recovery_holds(host_id,target_id,baseline_boot,baseline_epoch,reason) VALUES (?1,'t1','b',1,'repair')",
        [b],
    )
    .unwrap();
    f.db.execute(
        "INSERT INTO recovery_baseline_targets(host_id,target_id,baseline_boot,baseline_epoch,captured_at,disposition) VALUES (?1,'t1','b',1,0,'already_owned')",
        [b],
    )
    .unwrap();
    f.db.execute(
        "INSERT INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observed_at,provenance) VALUES (?1,'t1','b',1,1,0,'fresh')",
        [b],
    )
    .unwrap();
    let before = untouched_by_other_host(&f.db, b);

    // A coherent, complete snapshot of the first server that lacks `t1`.
    let admission = f
        .store
        .begin_host_observation(NODE, f.h1, &budget())
        .unwrap();
    assert_eq!(admission.host_id(), f.h1);
    let snapshot = HostSnapshot {
        boot: HostBootId::new("b"),
        epoch: 1,
        observation_sequence: 1,
        complete: true,
        enumeration: EnumerationEvidence::CoherentVerified,
        incarnation: IncarnationEvidence::Verified {
            identity: "inc".into(),
            evidence_kind: EvidenceKind::CoherentEnumeration,
        },
        targets: Vec::new(),
    };
    let header = SnapshotHeader::from_captured(admission, &snapshot).unwrap();
    assert_eq!(header.host_id, f.h1);
    let stage = f.store.begin_snapshot_stage(header, &budget()).unwrap();
    f.store.seal_snapshot_stage(&stage.id, &budget()).unwrap();
    let published = f
        .store
        .publish_snapshot_stage(&stage.id, &budget())
        .unwrap();
    assert_eq!(published.host_id, f.h1);

    // The page names only the first server's seat.
    let page = f
        .store
        .saved_seats_page(&published.id, 0, None, 16, &budget())
        .unwrap();
    let paged: Vec<&str> = page.seats.iter().map(|s| s.seat.as_str()).collect();
    assert_eq!(paged, ["s-h1"]);

    // Even handed the other server's seat, the first server's publication
    // refuses: nothing changes.
    let foreign = GuardedSeatTransition {
        publication: published.clone(),
        seat: SeatId::new("s-h2"),
        expected_binding_generation: 1,
        expected_target: Some(HostTargetId::new("t1")),
        expected_terminal: None,
        action: ReconciliationAction::MarkUnresolved,
    };
    assert_eq!(
        f.store
            .apply_reconciliation_transition(foreign, &budget())
            .unwrap(),
        ReconciliationOutcome::Stale
    );
    assert_eq!(untouched_by_other_host(&f.db, b), before);

    // The planned page unresolves the first server's seat and keeps its host.
    let plan = herdr_threads::identity::reconcile::plan_page(&page).unwrap();
    assert_eq!(plan.len(), 1);
    for transition in plan {
        assert_eq!(
            f.store
                .apply_reconciliation_transition(transition, &budget())
                .unwrap(),
            ReconciliationOutcome::Applied
        );
    }
    let (state, host): (String, i64) =
        f.db.query_row("SELECT state,host_id FROM seats WHERE id='s-h1'", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(
        (state.as_str(), host),
        ("unresolved", a),
        "unresolve keeps host_id"
    );
    assert_eq!(
        untouched_by_other_host(&f.db, b),
        before,
        "the second server's seat, recovery and baseline rows are untouched"
    );
    // The second server's own observation order is still at its first admission.
    let second = f
        .store
        .begin_host_observation(NODE, f.h2, &budget())
        .unwrap();
    assert_eq!((second.host_id(), second.sequence()), (f.h2, 1));
}

#[test]
fn local_seat_with_null_host_aborts() {
    let f = two_hosts();
    let error = f
        .db
        .execute(
            "INSERT INTO seats(id,instance_id,state,role,generation,created_at,home_node,is_local,host_id) \
             VALUES ('nohost',?1,'unresolved','native',1,0,?1,1,NULL)",
            [NODE],
        )
        .unwrap_err();
    assert!(
        error.to_string().contains("CHECK constraint failed"),
        "{error}"
    );
    let count: i64 =
        f.db.query_row("SELECT count(*) FROM seats", [], |r| r.get(0))
            .unwrap();
    assert_eq!(count, 0, "the aborted insert left no row");
}
