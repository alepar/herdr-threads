//! Wake scope per host (ht-5fr.2.5): one store with two attached Herdr servers.
//! Every seat-selecting wake call returns only the scoped host's seats, and a
//! per-seat wake write for a seat on another host answers `Conflict` and
//! changes nothing.

use herdr_threads::{
    ports::{HostId, StorePort, WakeCandidate, WakeOutcome},
    protocol::{
        ids::{HostLocator, SeatId, WakeAttemptId},
        pagination::PageRequest,
        results::ErrorCode,
        time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis},
    },
    store::{SqliteStore, StoreSettings, connection::StoreContext},
};
use rusqlite::Connection;
use std::sync::Arc;

const NODE: &str = "34750f98bd59fcfc946da45aaabe933b";
const BOOT: &str = "00000000-0000-0000-0000-000000000001";

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
        deadline: MonoInstant(10_000),
        cancellation: Cancellation::default(),
    }
}

/// A seat `seat` on the server at `socket` with a pending invitation: a due
/// wake candidate.
fn due_seat(db: &Connection, seat: &str, socket: &str, pane: &str) {
    let host_row: i64 = db
        .query_row(
            "SELECT host_id FROM herdr_hosts WHERE endpoint=?1",
            [socket],
            |r| r.get(0),
        )
        .unwrap();
    db.execute(
        "INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) VALUES (?1,?2,'resolved','native',?3,1,1,0,(SELECT id FROM local_node),1,?4)",
        rusqlite::params![seat, NODE, pane, host_row],
    )
    .unwrap();
    db.execute(
        "INSERT INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch,occupancy,ui_state,verified_execution,top_level_occupant) VALUES (?1,?2,'host',1,1,0,'fresh','term-'||?2,'inc','coherent_enumeration',1,'occupied','idle','exec',1)",
        rusqlite::params![host_row, pane],
    )
    .unwrap();
    db.execute(
        "INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES (?1,1,1,?2,'host',1,'codex','session','exec','fresh',0,0,'term-'||?2,'inc')",
        rusqlite::params![seat, pane],
    )
    .unwrap();
    let thread = format!("t-{seat}");
    db.execute(
        "INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,journaled) VALUES (?1,?2,'topic','goal',0,0,0)",
        rusqlite::params![thread, NODE],
    )
    .unwrap();
    db.execute(
        "INSERT INTO memberships(thread_id,seat_id,state) VALUES (?1,?2,'invited')",
        rusqlite::params![thread, seat],
    )
    .unwrap();
    db.execute(
        "INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms,origin_node) VALUES (?1,?2,?3,1,'pending',0,1,100,100,(SELECT id FROM local_node))",
        rusqlite::params![format!("inv-{seat}"), thread, seat],
    )
    .unwrap();
}

fn two_hosts() -> TwoHosts {
    let directory = Directory(herdr_threads::test_support::scratch::temp_path(format!(
        "multi-host-wake-{}",
        uuid::Uuid::new_v4()
    )));
    std::fs::create_dir(&directory.0).unwrap();
    let path = directory.0.join("store.db");
    let settings = StoreSettings {
        daemon_boot: Some(uuid::Uuid::parse_str(BOOT).unwrap()),
        minimum_wake_delay_ms: 120_000,
        ..StoreSettings::default()
    };
    let store = SqliteStore::new(
        StoreContext::new(path.clone(), Arc::new(Fixed)),
        NODE,
        settings,
    )
    .unwrap();
    let attach = |socket: &str| {
        store
            .attach_host(&HostLocator::parse(socket).unwrap(), 0, &budget())
            .unwrap()
            .0
    };
    let h1 = attach("/tmp/ht-multi-wake-a.sock");
    let h2 = attach("/tmp/ht-multi-wake-b.sock");
    assert_ne!(h1, h2);
    let db = Connection::open(&path).unwrap();
    db.pragma_update(None, "foreign_keys", "ON").unwrap();
    db.execute_batch(
        "UPDATE herdr_hosts SET host_boot='host',host_epoch=1; UPDATE local_node SET decision_seq=1",
    )
    .unwrap();
    due_seat(&db, "s1", "/tmp/ht-multi-wake-a.sock", "pane");
    due_seat(&db, "s2", "/tmp/ht-multi-wake-b.sock", "pane");
    TwoHosts {
        store,
        db,
        h1,
        h2,
        _directory: directory,
    }
}

fn candidates(fx: &TwoHosts, host: HostId) -> Vec<WakeCandidate> {
    fx.store
        .wake_candidates(host, PageRequest::default(), &budget())
        .unwrap()
        .items
}

fn seats(items: &[WakeCandidate]) -> Vec<&str> {
    items.iter().map(|c| c.seat.as_str()).collect()
}

fn reserved(fx: &TwoHosts, seat: &str) -> bool {
    fx.db
        .query_row(
            "SELECT COUNT(*) FROM wake_work WHERE seat_id=?1 AND reservation_id IS NOT NULL",
            [seat],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
        == 1
}

// Kills: a wake scan that is not filtered by host (the same pane id on two
// servers would wake the wrong server's seat).
#[test]
fn wake_candidates_scoped_to_host() {
    let fx = two_hosts();
    assert_eq!(seats(&candidates(&fx, fx.h1)), ["s1"]);
    assert_eq!(seats(&candidates(&fx, fx.h2)), ["s2"]);
}

// Kills: a per-seat write that trusts the candidate instead of the worker's
// scope: a seat of host 2 reserved by host 1's worker.
#[test]
fn per_seat_wake_call_with_wrong_scope_is_conflict() {
    let fx = two_hosts();
    let foreign = candidates(&fx, fx.h2).remove(0);
    let error = fx
        .store
        .reserve_wake(fx.h1, &foreign, &budget())
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict);
    assert!(!reserved(&fx, "s2"), "a refused scope reserves nothing");
    let error = fx
        .store
        .clear_wake_batch_if_empty(fx.h1, &SeatId::new("s2"), &budget())
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict);
    // The matching scope still reserves, and only the matching scope settles it.
    let reservation = fx
        .store
        .reserve_wake(fx.h2, &foreign, &budget())
        .unwrap()
        .expect("own host reserves");
    assert!(reserved(&fx, "s2"));
    let error = fx
        .store
        .complete_wake(
            fx.h1,
            reservation.attempt.clone(),
            WakeOutcome::Submitted,
            None,
            &budget(),
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict);
    assert!(reserved(&fx, "s2"), "a refused completion settles nothing");
    fx.store
        .complete_wake(
            fx.h2,
            reservation.attempt,
            WakeOutcome::Submitted,
            None,
            &budget(),
        )
        .unwrap();
    assert!(!reserved(&fx, "s2"));
    // An unknown attempt is not a scope error.
    assert!(
        !fx.store
            .complete_wake(
                fx.h1,
                WakeAttemptId::new("none"),
                WakeOutcome::Submitted,
                None,
                &budget(),
            )
            .unwrap()
    );
}
