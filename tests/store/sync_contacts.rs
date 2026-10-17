//! `store::sync_contacts` and the `SyncStorePort` contract over both of its
//! implementations.
use herdr_threads::{
    node::NodeUuid,
    protocol::{
        results::ErrorCode,
        time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis},
    },
    service::fair_writer::FairWriter,
    store::{
        SqliteStore, StoreSettings,
        connection::StoreContext,
        sync_contacts::{
            self as contacts, ContactVia, NodeContact, NodeReach, UNREACHABLE_AFTER_MS,
        },
        sync_dial_targets::{
            self as targets, DialSource, DialState, DialTransition, StopReason, TargetId,
        },
    },
    sync::sync_store::{MemorySyncStore, SyncStorePort},
    test_support::isolation::TestIsolation,
};
use rusqlite::Connection;
use std::sync::Arc;

fn node(n: u128) -> NodeUuid {
    NodeUuid::from_u128(n)
}

fn contact(n: u128, at: u64, via: ContactVia) -> NodeContact {
    NodeContact {
        node: node(n),
        last_contact_at: at,
        via,
    }
}

fn record(db: &mut Connection, rows: &[NodeContact], now: u64) {
    let tx = db.transaction().unwrap();
    contacts::record(&tx, rows, now).unwrap();
    tx.commit().unwrap();
}

#[test]
fn record_keeps_max_last_contact() {
    let mut db = super::channel_archival::fixture();
    record(&mut db, &[contact(2, 1_000, ContactVia::Direct)], 5);
    // An older contact changes nothing but is not an error.
    record(&mut db, &[contact(2, 900, ContactVia::Relayed)], 6);
    assert_eq!(
        contacts::get(&db, &node(2)).unwrap(),
        Some(contact(2, 1_000, ContactVia::Direct))
    );
    // A newer one replaces the time and the via.
    record(
        &mut db,
        &[
            contact(2, 1_100, ContactVia::Relayed),
            contact(3, 50, ContactVia::Direct),
        ],
        7,
    );
    assert_eq!(
        contacts::get(&db, &node(2)).unwrap(),
        Some(contact(2, 1_100, ContactVia::Relayed))
    );
    assert_eq!(
        contacts::get(&db, &node(3)).unwrap(),
        Some(contact(3, 50, ContactVia::Direct))
    );
    // An equal time keeps the stored via.
    record(&mut db, &[contact(2, 1_100, ContactVia::Direct)], 8);
    assert_eq!(
        contacts::get(&db, &node(2)).unwrap().unwrap().via,
        ContactVia::Relayed
    );
    let updated: i64 = db
        .query_row(
            "SELECT updated_at FROM sync_node_contact WHERE node_uuid = ?1",
            [node(2).to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(updated, 8, "updated_at is the write time");
    assert_eq!(contacts::get(&db, &node(4)).unwrap(), None);
}

#[test]
fn reach_thresholds_and_since() {
    let mut db = super::channel_archival::fixture();
    record(&mut db, &[contact(2, 1_000, ContactVia::Direct)], 1_000);
    let at = |now| contacts::reach(&db, &node(2), now).unwrap();
    assert_eq!(at(1_000), NodeReach::Reachable);
    assert_eq!(
        at(1_000 + UNREACHABLE_AFTER_MS),
        NodeReach::Reachable,
        "exactly at the threshold"
    );
    assert_eq!(
        at(1_000 + UNREACHABLE_AFTER_MS + 1),
        NodeReach::Unreachable { since: Some(1_000) }
    );
    assert_eq!(
        at(500),
        NodeReach::Reachable,
        "a contact ahead of now is not stale"
    );
    assert_eq!(UNREACHABLE_AFTER_MS, 180_000);
}

#[test]
fn reach_no_row_and_self() {
    let db = super::channel_archival::fixture();
    assert_eq!(
        contacts::reach(&db, &node(2), 1_000_000).unwrap(),
        NodeReach::Unreachable { since: None }
    );
    let me: NodeUuid = herdr_threads::test_support::test_node().parse().unwrap();
    assert_eq!(
        contacts::reach(&db, &me, u64::MAX).unwrap(),
        NodeReach::Reachable,
        "self, with no row"
    );
}

#[test]
fn sync_node_contact_checks() {
    let db = super::channel_archival::fixture();
    for sql in [
        "INSERT INTO sync_node_contact VALUES ('short',1,'direct',1)",
        "INSERT INTO sync_node_contact VALUES (printf('%032d',1),1,'other',1)",
    ] {
        assert!(db.execute(sql, []).is_err(), "{sql}");
    }
}

// ---- SyncStorePort contract ------------------------------------------------

struct Clocked;
impl Clock for Clocked {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(777)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(1)
    }
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Cancellation::default(),
    }
}

/// One implementation under test plus the way to seed and inspect it.
trait Subject {
    fn port(&self) -> &dyn SyncStorePort;
    fn seed(&self, dest: &str) -> TargetId;
    fn forget(&self, dest: &str);
    fn all_targets(&self) -> Vec<targets::DialTarget>;
    fn all_contacts(&self) -> Vec<NodeContact>;
}

struct Memory(MemorySyncStore);
impl Subject for Memory {
    fn port(&self) -> &dyn SyncStorePort {
        &self.0
    }
    fn seed(&self, dest: &str) -> TargetId {
        self.0.add_target(dest)
    }
    fn forget(&self, dest: &str) {
        self.0.disable(dest)
    }
    fn all_targets(&self) -> Vec<targets::DialTarget> {
        self.0.targets()
    }
    fn all_contacts(&self) -> Vec<NodeContact> {
        self.0.contacts()
    }
}

struct Sqlite {
    _iso: TestIsolation,
    path: std::path::PathBuf,
    store: SqliteStore,
}
impl Sqlite {
    fn new(label: &str, admission: Option<Arc<FairWriter>>) -> Self {
        let iso = TestIsolation::new(label);
        let path = iso.state_root().join("store.db");
        let mut store = SqliteStore::new(
            StoreContext::new(path.clone(), Arc::new(Clocked)).with_test_host(),
            herdr_threads::test_support::test_node(),
            StoreSettings::default(),
        )
        .unwrap();
        if let Some(writer) = admission {
            store = store.with_sync_admission(writer);
        }
        Self {
            _iso: iso,
            path,
            store,
        }
    }
    fn conn(&self) -> Connection {
        let conn = Connection::open(&self.path).unwrap();
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        conn
    }
}
impl Subject for Sqlite {
    fn port(&self) -> &dyn SyncStorePort {
        &self.store
    }
    fn seed(&self, dest: &str) -> TargetId {
        let mut conn = self.conn();
        let tx = conn.transaction().unwrap();
        let id = targets::add_explicit(&tx, dest, None, 0).unwrap();
        tx.commit().unwrap();
        id
    }
    fn forget(&self, dest: &str) {
        let mut conn = self.conn();
        let tx = conn.transaction().unwrap();
        targets::forget(&tx, dest, 0).unwrap();
        tx.commit().unwrap();
    }
    fn all_targets(&self) -> Vec<targets::DialTarget> {
        targets::list(&self.conn()).unwrap()
    }
    fn all_contacts(&self) -> Vec<NodeContact> {
        let conn = self.conn();
        let nodes: Vec<String> = conn
            .prepare("SELECT node_uuid FROM sync_node_contact ORDER BY node_uuid")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        nodes
            .into_iter()
            .map(|n| contacts::get(&conn, &n.parse().unwrap()).unwrap().unwrap())
            .collect()
    }
}

/// The cases both implementations must agree on.
fn port_contract(subject: &dyn Subject) {
    let port = subject.port();
    let b = &budget();
    assert!(port.enabled_targets(b).unwrap().is_empty());

    let first = subject.seed("alpha");
    let second = subject.seed("beta");
    let third = subject.seed("gamma");
    subject.forget("gamma");
    let enabled = port.enabled_targets(b).unwrap();
    assert_eq!(
        enabled
            .iter()
            .map(|t| (t.target_id, t.ssh_dest.as_str()))
            .collect::<Vec<_>>(),
        vec![(first, "alpha"), (second, "beta")],
        "enabled targets only, by id"
    );
    assert!(
        enabled
            .iter()
            .all(|t| t.source == DialSource::Explicit && t.state == DialState::Idle)
    );

    let now = UtcMillis(1_000);
    for (target, transition) in [
        (first, DialTransition::Dialing),
        (first, DialTransition::Linked { node: node(5) }),
        (
            second,
            DialTransition::Backoff {
                attempts: 2,
                next_at: 9_000,
                error: "ssh: refused".into(),
            },
        ),
        (third, DialTransition::NotInstalled),
    ] {
        port.record_transition(target, &transition, now, b).unwrap();
    }
    port.record_transition(
        second,
        &DialTransition::Stopped {
            reason: StopReason::SelfLink,
            version: "9.9.9".into(),
        },
        UtcMillis(2_000),
        b,
    )
    .unwrap();
    let rows = subject.all_targets();
    let by = |id: TargetId| rows.iter().find(|t| t.target_id == id).unwrap();
    assert_eq!(by(first).state, DialState::Linked);
    assert_eq!(by(first).learned_node, Some(node(5)));
    assert_eq!(by(first).last_linked_at, Some(1_000));
    assert_eq!(by(second).state, DialState::Stopped);
    assert_eq!(
        by(second).stopped,
        Some((StopReason::SelfLink, Some("9.9.9".into())))
    );
    assert_eq!(by(second).attempts, 2);
    assert_eq!(by(second).last_error.as_deref(), Some("ssh: refused"));
    assert_eq!(by(second).updated_at, 2_000);
    assert_eq!(
        by(third).state,
        DialState::NotInstalled,
        "a forgotten row still records"
    );
    assert!(!by(third).enabled);

    let err = port
        .record_transition(TargetId(4242), &DialTransition::Dialing, now, b)
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidRequest);

    port.record_contacts(&[], b).unwrap();
    assert!(subject.all_contacts().is_empty());
    port.record_contacts(
        &[
            contact(2, 5_000, ContactVia::Direct),
            contact(3, 4_000, ContactVia::Relayed),
        ],
        b,
    )
    .unwrap();
    port.record_contacts(
        &[
            contact(2, 4_500, ContactVia::Relayed),
            contact(3, 6_000, ContactVia::Direct),
        ],
        b,
    )
    .unwrap();
    let mut held = subject.all_contacts();
    held.sort_by_key(|c| c.node);
    assert_eq!(
        held,
        vec![
            contact(2, 5_000, ContactVia::Direct),
            contact(3, 6_000, ContactVia::Direct)
        ],
        "the later contact wins per node"
    );
}

#[test]
fn sync_store_port_contract_memory() {
    port_contract(&Memory(MemorySyncStore::new()));
}

#[test]
fn sync_store_port_contract_sqlite() {
    port_contract(&Sqlite::new("sync-port-sqlite", None));
}

#[test]
fn sync_store_port_contract_sqlite_with_admission() {
    port_contract(&Sqlite::new(
        "sync-port-admission",
        Some(Arc::new(FairWriter::new(8))),
    ));
}

#[test]
fn sqlite_writes_wait_for_a_background_turn() {
    let writer = Arc::new(FairWriter::new(8));
    let subject = Sqlite::new("sync-port-turn", Some(Arc::clone(&writer)));
    let target = subject.seed("alpha");
    let held = writer.enter_background(&budget(), &Clocked).unwrap();

    // While another background turn is running, both writes wait; a cancelled
    // budget ends the wait with an error and writes nothing.
    for write in [0, 1] {
        let cancel = Cancellation::default();
        let blocked = CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: cancel.clone(),
        };
        let admission = Arc::clone(&writer);
        let canceller = std::thread::spawn(move || {
            // Start the timer only after this write really queued behind the
            // held turn; a descheduled caller must not start already cancelled.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while admission.waiting() == (0, 0) && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            let queued = admission.waiting() != (0, 0);
            std::thread::sleep(std::time::Duration::from_millis(100));
            cancel.cancel();
            queued
        });
        let started = std::time::Instant::now();
        let result = if write == 0 {
            subject
                .port()
                .record_contacts(&[contact(2, 5, ContactVia::Direct)], &blocked)
        } else {
            subject.port().record_transition(
                target,
                &DialTransition::Dialing,
                UtcMillis(5),
                &blocked,
            )
        };
        assert!(canceller.join().unwrap(), "write {write} entered admission");
        assert!(
            result.is_err(),
            "write {write} must not bypass the held turn"
        );
        assert!(started.elapsed() >= std::time::Duration::from_millis(90));
    }
    assert!(subject.all_contacts().is_empty());
    assert_eq!(subject.all_targets()[0].state, DialState::Idle);

    drop(held);
    subject
        .port()
        .record_contacts(&[contact(2, 5, ContactVia::Direct)], &budget())
        .unwrap();
    subject
        .port()
        .record_transition(target, &DialTransition::Dialing, UtcMillis(5), &budget())
        .unwrap();
    assert_eq!(subject.all_contacts().len(), 1);
    assert_eq!(subject.all_targets()[0].state, DialState::Dialing);
}
