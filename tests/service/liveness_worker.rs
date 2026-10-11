//! The liveness worker (ht-54r.23): a real store, an injected process table, an injected clock.
use super::*;
use crate::{
    ports::ProcessRecord,
    protocol::time::{Clock, UtcMillis},
    scheduler::WakePort,
    service::workers::ScheduledStore,
    store::{SqliteStore, StoreSettings, connection::StoreContext},
    test_support::isolation::TestIsolation,
};
use rusqlite::Connection;
use std::{collections::HashMap, path::PathBuf, sync::Mutex, time::Duration};

struct FixedClock;
impl Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(5_000)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(1)
    }
}

/// A mutable process table: a pid maps to its start stamp.
#[derive(Default)]
struct Table(Mutex<HashMap<u32, u64>>);
impl Table {
    fn set(&self, pid: u32, start: u64) {
        self.0.lock().unwrap().insert(pid, start);
    }
    fn kill(&self, pid: u32) {
        self.0.lock().unwrap().remove(&pid);
    }
}
impl ProcessInfo for Table {
    fn process(&self, pid: u32) -> Option<ProcessRecord> {
        self.0.lock().unwrap().get(&pid).map(|start| ProcessRecord {
            pid,
            parent: None,
            start: *start,
            argv: Vec::new(),
        })
    }
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Cancellation::default(),
    }
}

struct Fx {
    _iso: TestIsolation,
    path: PathBuf,
    store: Arc<SqliteStore>,
    table: Arc<Table>,
    writer: Arc<FairWriter>,
    worker: Arc<LivenessWorker>,
}

impl Fx {
    fn new(label: &str) -> Self {
        let iso = TestIsolation::new(label);
        let path = iso.state_root().join("store.db");
        let store = Arc::new(
            SqliteStore::new(
                StoreContext::new(path.clone(), Arc::new(FixedClock)),
                "i",
                StoreSettings::default(),
            )
            .unwrap(),
        );
        let table = Arc::new(Table::default());
        let writer = Arc::new(FairWriter::new(32));
        let worker = Arc::new(LivenessWorker::new(
            store.clone(),
            Arc::clone(&writer),
            table.clone(),
        ));
        let fx = Self {
            _iso: iso,
            path,
            store,
            table,
            writer,
            worker,
        };
        let db = fx.raw();
        db.execute(
            "INSERT INTO host_instances(id,created_at,host_boot,host_epoch) VALUES ('i',0,'b',1)",
            [],
        )
        .unwrap();
        fx.seat("s");
        fx
    }

    fn raw(&self) -> Connection {
        let db = Connection::open(&self.path).unwrap();
        db.busy_timeout(Duration::from_secs(5)).unwrap();
        db.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        db
    }

    /// A resolved seat on the observed target `p`, available once a binding is registered.
    fn seat(&self, id: &str) {
        let db = self.raw();
        let target = format!("p-{id}");
        db.execute(
            "INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,unavailability_open) VALUES (?1,'i','resolved','native',?2,0,1,0,0)",
            rusqlite::params![id, target],
        )
        .unwrap();
        db.execute(
            "INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance) VALUES ('i',?1,'b',1,1,1,'fresh')",
            [&target],
        )
        .unwrap();
    }

    /// A connected, registered binding; `fingerprint` None leaves it fingerprint-less.
    fn bind(
        &self,
        seat: &str,
        harness: &str,
        session: &str,
        fingerprint: Option<(u32, u64)>,
        claimed_at: i64,
    ) -> String {
        let db = self.raw();
        db.execute(
            "INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation,liveness_pid,liveness_start) \
             VALUES (?1,0,1,'p-'||?1,'b',1,?2,?3,'e','verified',?4,1,'term-p','inc',?5,?6)",
            rusqlite::params![
                seat,
                harness,
                session,
                claimed_at,
                fingerprint.map(|f| i64::from(f.0)),
                fingerprint.map(|f| f.1 as i64)
            ],
        )
        .unwrap();
        db.query_row(
            "SELECT binding_id FROM occupant_bindings WHERE rowid=last_insert_rowid()",
            [],
            |r| r.get(0),
        )
        .unwrap()
    }

    fn state(&self, binding: &str) -> (String, Option<String>) {
        self.raw()
            .query_row(
                "SELECT liveness, disconnect_reason FROM occupant_bindings WHERE binding_id=?1",
                [binding],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    }

    fn connected(&self, binding: &str) -> bool {
        self.state(binding).0 == "connected"
    }

    fn episode_open(&self, seat: &str) -> bool {
        self.raw()
            .query_row(
                "SELECT unavailability_open FROM seats WHERE id=?1",
                [seat],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
            == 1
    }

    fn commits(&self) -> u64 {
        self.store.commit_counts().values().sum()
    }
}

#[test]
fn exited_process_disconnects_its_binding() {
    let fx = Fx::new("liveness-exit");
    fx.table.set(100, 5);
    let b = fx.bind("s", "claude", "A", Some((100, 5)), 10);
    assert_eq!(
        fx.worker.pass(&budget()).unwrap(),
        PassOutcome {
            checked: 1,
            disconnected: 0
        }
    );
    assert!(fx.connected(&b));
    fx.table.kill(100);
    assert_eq!(fx.worker.pass(&budget()).unwrap().disconnected, 1);
    assert_eq!(
        fx.state(&b),
        ("disconnected".into(), Some("liveness_process_exit".into()))
    );
    // The last connected binding left: the seat's unavailability episode opened, and the
    // binding was disconnected, not ended.
    assert!(fx.episode_open("s"));
    let ended: Option<i64> = fx
        .raw()
        .query_row(
            "SELECT ended_at FROM occupant_bindings WHERE binding_id=?1",
            [&b],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(ended, None);
}

#[test]
fn pid_reuse_with_another_start_disconnects() {
    let fx = Fx::new("liveness-reuse");
    fx.table.set(100, 6);
    let b = fx.bind("s", "claude", "A", Some((100, 5)), 10);
    fx.worker.pass(&budget()).unwrap();
    assert_eq!(
        fx.state(&b),
        ("disconnected".into(), Some("liveness_process_exit".into()))
    );
}

#[test]
fn same_fingerprint_newer_session_disconnects_the_older() {
    let fx = Fx::new("liveness-supersede");
    fx.table.set(100, 5);
    fx.table.set(200, 5);
    let older = fx.bind("s", "claude", "A", Some((100, 5)), 10);
    let newer = fx.bind("s", "claude", "B", Some((100, 5)), 20);
    let other_process = fx.bind("s", "claude", "C", Some((200, 5)), 5);
    let other_harness = fx.bind("s", "codex", "D", Some((100, 5)), 5);
    fx.worker.pass(&budget()).unwrap();
    assert_eq!(
        fx.state(&older),
        ("disconnected".into(), Some("superseded".into()))
    );
    assert!(fx.connected(&newer));
    assert!(fx.connected(&other_process));
    assert!(fx.connected(&other_harness));
    assert!(!fx.episode_open("s"));
}

#[test]
fn a_plain_call_only_binding_neither_supersedes_nor_is_superseded() {
    let fx = Fx::new("liveness-plain-call");
    fx.table.set(100, 5);
    let older = fx.bind("s", "claude", "A", Some((100, 5)), 10);
    let newer = fx.bind("s", "claude", "B", Some((100, 5)), 20);
    let plain = |binding: &str| {
        fx.raw()
            .execute(
                "UPDATE occupant_bindings SET observation_provenance='cooperative_env_session' WHERE binding_id=?1",
                [binding],
            )
            .unwrap();
    };
    // The newer binding was enrolled by a plain call only: it does not take the process.
    plain(&newer);
    let candidates = fx.store.liveness_candidates(None, &budget()).unwrap();
    assert_eq!(classify(&candidates, fx.table.as_ref()), vec![]);
    fx.worker.pass(&budget()).unwrap();
    assert!(fx.connected(&older));
    assert!(fx.connected(&newer));
    // And the older plain-call-only binding is not superseded by a newer lifecycle one either.
    plain(&older);
    fx.raw()
        .execute(
            "UPDATE occupant_bindings SET observation_provenance='cooperative_top_level' WHERE binding_id=?1",
            [&newer],
        )
        .unwrap();
    fx.worker.pass(&budget()).unwrap();
    assert!(fx.connected(&older));
    assert!(fx.connected(&newer));
    // Control: both lifecycle-claimed, the older is superseded.
    fx.raw()
        .execute(
            "UPDATE occupant_bindings SET observation_provenance='cooperative_top_level'",
            [],
        )
        .unwrap();
    fx.worker.pass(&budget()).unwrap();
    assert_eq!(
        fx.state(&older),
        ("disconnected".into(), Some("superseded".into()))
    );
    assert!(fx.connected(&newer));
}

#[test]
fn same_fingerprint_tie_goes_to_the_later_enrolled_binding() {
    let fx = Fx::new("liveness-tie");
    fx.table.set(100, 5);
    let first = fx.bind("s", "claude", "A", Some((100, 5)), 10);
    let second = fx.bind("s", "claude", "B", Some((100, 5)), 10);
    fx.worker.pass(&budget()).unwrap();
    assert!(!fx.connected(&first));
    assert!(fx.connected(&second));
}

#[test]
fn subagents_neither_supersede_nor_are_superseded() {
    let fx = Fx::new("liveness-subagent");
    fx.table.set(100, 5);
    let top = fx.bind("s", "claude", "A", Some((100, 5)), 10);
    let db = fx.raw();
    db.execute(
        "INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation,liveness_pid,liveness_start,role,parent_binding,agent_id) \
         VALUES ('s',0,1,'p-s','b',1,'claude','A','e','verified',99,1,'term-p','inc',100,5,'subagent',?1,'sub')",
        [&top],
    )
    .unwrap();
    let sub: String = db
        .query_row(
            "SELECT binding_id FROM occupant_bindings WHERE role='subagent'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    fx.worker.pass(&budget()).unwrap();
    assert!(fx.connected(&top), "a newer subagent never supersedes");
    assert!(fx.connected(&sub));
}

#[test]
fn fingerprintless_bindings_disconnect_on_first_pass() {
    let fx = Fx::new("liveness-nofp");
    let migrated = fx.bind("s", "claude", "A", None, 10);
    let plain = fx.bind("s", "codex", "B", None, 11);
    fx.raw()
        .execute(
            "UPDATE occupant_bindings SET migrated=1 WHERE binding_id=?1",
            [&migrated],
        )
        .unwrap();
    fx.worker.pass(&budget()).unwrap();
    // Decision 9: no zombie bindings. The migrated 24 h grace is archival only (§4), so
    // migrated and plain fingerprint-less bindings alike disconnect, and neither is ended.
    for binding in [&migrated, &plain] {
        assert_eq!(
            fx.state(binding),
            ("disconnected".into(), Some("liveness_process_exit".into()))
        );
        let ended: Option<i64> = fx
            .raw()
            .query_row(
                "SELECT ended_at FROM occupant_bindings WHERE binding_id=?1",
                [binding],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(ended, None);
    }
}

#[test]
fn fingerprintless_verdict_is_void_after_a_fingerprinted_reconnect() {
    let fx = Fx::new("liveness-nofp-void");
    let b = fx.bind("s", "claude", "A", None, 10);
    let candidates = fx.store.liveness_candidates(None, &budget()).unwrap();
    let decisions = classify(&candidates, fx.table.as_ref());
    assert_eq!(decisions.len(), 1);
    // The binding reconnects with a live fingerprint before the write lands.
    fx.table.set(100, 5);
    fx.raw()
        .execute(
            "UPDATE occupant_bindings SET liveness_pid=100, liveness_start=5 WHERE binding_id=?1",
            [&b],
        )
        .unwrap();
    assert_eq!(fx.store.apply_liveness(&decisions, &budget()).unwrap(), 0);
    assert!(fx.connected(&b));
}

#[test]
fn human_bindings_are_never_touched() {
    let fx = Fx::new("liveness-human");
    let human = fx.bind("s", "human", "H", None, 10);
    let dead_pid = fx.bind("s", "human", "H2", Some((300, 1)), 11);
    assert_eq!(fx.worker.pass(&budget()).unwrap().checked, 0);
    assert!(fx.connected(&human));
    assert!(fx.connected(&dead_pid));
}

#[test]
fn held_unresolved_or_retired_seats_are_skipped() {
    let fx = Fx::new("liveness-held");
    // The process table is empty, as after a Herdr restart: every fingerprint looks dead.
    fx.seat("unresolved");
    fx.seat("retired");
    fx.seat("held");
    let on_resolved = fx.bind("s", "claude", "A", Some((100, 5)), 10);
    let on_unresolved = fx.bind("unresolved", "claude", "B", Some((101, 5)), 10);
    let on_retired = fx.bind("retired", "claude", "C", Some((102, 5)), 10);
    let on_held = fx.bind("held", "claude", "D", Some((103, 5)), 10);
    let db = fx.raw();
    db.execute(
        "UPDATE seats SET state='unresolved', unresolved_reason='other' WHERE id='unresolved'",
        [],
    )
    .unwrap();
    db.execute(
        "UPDATE seats SET state='retired', retired_at=1 WHERE id='retired'",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO recovery_holds(instance_id,target_id,baseline_boot,baseline_epoch,reason) VALUES ('i','p-s','b',1,'restore')",
        [],
    )
    .unwrap();
    // The open hold covers the whole instance (including the resolved seat `s`).
    assert_eq!(fx.worker.pass(&budget()).unwrap(), PassOutcome::default());
    for b in [&on_resolved, &on_unresolved, &on_retired, &on_held] {
        assert!(fx.connected(b));
    }
    // Once the hold is released only the resolved seats are judged.
    db.execute("UPDATE recovery_holds SET released_at=2", [])
        .unwrap();
    fx.worker.pass(&budget()).unwrap();
    assert!(!fx.connected(&on_resolved));
    assert!(!fx.connected(&on_held));
    assert!(fx.connected(&on_unresolved));
    assert!(fx.connected(&on_retired));
}

#[test]
fn idle_pass_commits_nothing() {
    let fx = Fx::new("liveness-idle");
    fx.table.set(100, 5);
    fx.table.set(101, 7);
    fx.bind("s", "claude", "A", Some((100, 5)), 10);
    fx.bind("s", "codex", "B", Some((101, 7)), 11);
    let before = fx.commits();
    for _ in 0..3 {
        assert_eq!(
            fx.worker.pass(&budget()).unwrap(),
            PassOutcome {
                checked: 2,
                disconnected: 0
            }
        );
    }
    assert_eq!(fx.commits(), before, "an idle pass writes nothing");
}

#[test]
fn recheck_before_wake_disconnects_a_dead_binding_before_it_is_targeted() {
    let fx = Fx::new("liveness-wake");
    fx.seat("t");
    fx.table.set(100, 5);
    fx.table.set(110, 5);
    let dead = fx.bind("s", "claude", "A", Some((100, 5)), 10);
    let other_seat = fx.bind("t", "claude", "B", Some((110, 5)), 10);
    fx.table.kill(100);
    fx.table.kill(110);
    let port = ScheduledStore::new(fx.store.clone(), Arc::clone(&fx.writer))
        .with_liveness(Some(Arc::clone(&fx.worker)));
    port.before_wake_decision(&SeatId::new("s"), &budget())
        .unwrap();
    assert_eq!(
        fx.state(&dead),
        ("disconnected".into(), Some("liveness_process_exit".into()))
    );
    assert!(
        fx.connected(&other_seat),
        "the pre-wake check judges only the candidate's seat"
    );
    // Without a worker the hook is a no-op.
    let bare = ScheduledStore::new(fx.store.clone(), Arc::clone(&fx.writer));
    bare.before_wake_decision(&SeatId::new("t"), &budget())
        .unwrap();
    assert!(fx.connected(&other_seat));
}

#[test]
fn the_thread_rechecks_on_its_cadence_and_stops_on_cancel() {
    let fx = Fx::new("liveness-thread");
    fx.table.set(100, 5);
    let b = fx.bind("s", "claude", "A", Some((100, 5)), 10);
    let cancel = Cancellation::default();
    let pacer = Arc::new(Pacer::new(
        "liveness",
        Arc::new(FixedClock) as Arc<dyn Clock>,
        cancel.clone(),
    ));
    let handle = start_liveness_worker(
        Arc::clone(&fx.worker),
        pacer.clone(),
        cancel.clone(),
        Arc::new(|_| {}),
    )
    .unwrap();
    // The first pass runs at once and leaves the live binding alone.
    let started = std::time::Instant::now();
    while pacer.idle_events() < 1 {
        assert!(started.elapsed() < Duration::from_secs(20), "first pass");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(fx.connected(&b));
    fx.table.kill(100);
    // A fixed clock never reaches the tick; the clock-advance wake stands in for the cadence.
    pacer.clock_advanced();
    pacer.kick_explicit();
    let started = std::time::Instant::now();
    while fx.connected(&b) {
        assert!(started.elapsed() < Duration::from_secs(20), "second pass");
        std::thread::sleep(Duration::from_millis(1));
    }
    cancel.cancel();
    handle.join().unwrap();
}
