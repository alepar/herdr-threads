//! ht-5fr.6.14: the `sync resync` rebuild, end to end.
//!
//! - `request` (called directly: the operator CLI is ht-5fr.8's) refuses with
//!   no path to the peer, and otherwise writes the marker and triggers the
//!   daemon's graceful stop.
//! - Two real daemons linked with `sync.dial_command` (never ssh): the node
//!   to rebuild is stopped, given a `requested` marker, and restarted. The
//!   restart backs the store up and moves it aside, bootstraps a new
//!   incarnation, restores the dial target, salvages a fact authored while the
//!   link was down, pulls the peer's facts and removes the marker.
//! - A crash after the `moved` marker write is a failpoint armed on an
//!   in-process daemon start (failpoints cannot be armed in a spawned
//!   daemon); the next start resumes without a second backup.
//!
//! Own target: it runs daemons, and the crash case redirects this process's
//! stdio and installs a panic hook.

use herdr_threads::{
    app::{LaneProbe, SystemClock, run_elected_probed},
    daemon::paths::NodePaths,
    node::{NodeIdentity, NodeTag, NodeUuid, tag_for},
    protocol::{
        results::{ApiError, ErrorCode},
        time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis},
    },
    service::workers::WorkerStatus,
    service::{config::ServiceConfig, fair_writer::FairWriter, pacer::Pacer},
    store::connection::OwnHistoryHold,
    store::sync_dial_targets::DialTarget,
    sync::{
        EntityKey,
        coordinator::{CoordinatorDeps, LogOnlyForkSink, SyncCoordinator},
        dialer::DialConfig,
        facts::{ApplyContext, FactError, FactKind},
        memory_journal::MemoryJournal,
        resync::{
            MARKER_FILE, RequestEnv, ResyncMarker, ResyncPhase, ResyncStore, ResyncTarget, request,
        },
        sync_store::MemorySyncStore,
    },
    test_support::{
        FakeHostWorld, TEST_NODE_SEED,
        failpoints::Failpoint,
        isolation::TestIsolation,
        multinode::{Cluster, ClusterOptions},
        resolve_seeded,
    },
};
use rusqlite::{Connection, OpenFlags, Transaction};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");
const WAIT: Duration = Duration::from_secs(90);
const OPERATOR: &str = "operator:local-user:501";

/// Serializes the in-process daemons of this binary (each redirects the
/// process's stdio and installs a panic hook).
static ONE_DAEMON: Mutex<()> = Mutex::new(());

// ---- a fact kind only the authoring store knows ---------------------------

#[derive(Serialize, Deserialize)]
struct Note {
    text: String,
}

struct NoteKind;

impl FactKind for NoteKind {
    const NAME: &'static str = "resync.note";
    const MAJOR: u32 = 1;
    type Payload = Note;
    fn creates(_: &Note) -> Option<EntityKey> {
        None
    }
    fn needs(_: &Note) -> Vec<EntityKey> {
        Vec::new()
    }
    fn apply(_: &Transaction<'_>, _: &mut ApplyContext<'_>, _: Note) -> Result<(), FactError> {
        Ok(())
    }
}

fn note(text: &str) -> Note {
    Note { text: text.into() }
}

// ---- request: the preconditions and the marker ------------------------------

struct WallClock(Instant);
impl Clock for WallClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(1_700_000_000_000 + self.0.elapsed().as_millis() as i64)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.elapsed().as_millis() as u64)
    }
}

/// A known peer reached by `targets` (none: no path).
struct Directory {
    targets: Vec<DialTarget>,
}

impl ResyncStore for Directory {
    fn peer_tag(&self, node: NodeUuid, _: &CallBudget) -> Result<Option<NodeTag>, ApiError> {
        Ok(Some(tag_for(node, 0)))
    }
    fn targets_reaching(&self, _: NodeUuid, _: &CallBudget) -> Result<Vec<DialTarget>, ApiError> {
        Ok(self.targets.clone())
    }
}

/// A running coordinator with nothing linked: the daemon `request` talks to.
struct Coordinator {
    handle: herdr_threads::sync::coordinator::SyncHandle,
    cancel: Cancellation,
    join: tokio::task::JoinHandle<()>,
    clock: Arc<dyn Clock>,
}

fn start_coordinator() -> Coordinator {
    let clock: Arc<dyn Clock> = Arc::new(WallClock(Instant::now()));
    let cancel = Cancellation::default();
    let log: herdr_threads::sync::coordinator::LogFn = Arc::new(|_| {});
    let (handle, _intake, join) = SyncCoordinator::start(CoordinatorDeps {
        journal: Arc::new(MemoryJournal::new(Arc::new(NodeIdentity::from_seed(
            TEST_NODE_SEED,
        )))),
        sync_store: Arc::new(MemorySyncStore::new()),
        identity: Arc::new(NodeIdentity::from_seed(TEST_NODE_SEED)),
        boot: uuid::Uuid::from_u128(7),
        writer: Arc::new(FairWriter::new(8)),
        clock: clock.clone(),
        pacer: Arc::new(Pacer::new("sync", clock.clone(), cancel.clone())),
        status: Arc::new(WorkerStatus::default()),
        log: log.clone(),
        fork_sink: Arc::new(LogOnlyForkSink::new(log, clock.clone())),
        own_history: Arc::new(OwnHistoryHold::new(Arc::default())),
        dial: DialConfig::default(),
        resync_pin: None,
        cancel: cancel.clone(),
    });
    Coordinator {
        handle,
        cancel,
        join,
        clock,
    }
}

impl Coordinator {
    async fn stop(self) {
        self.cancel.cancel();
        self.join.await.unwrap();
    }
}

fn peer() -> NodeUuid {
    NodeUuid::from_bytes([7; 16])
}

#[tokio::test]
async fn request_with_no_path_refuses() {
    let iso = TestIsolation::new("rs-nopath");
    let paths = resolve_seeded(iso.state_root());
    let coordinator = start_coordinator();
    let shutdown = Cancellation::default();
    let error = request(
        &Directory {
            targets: Vec::new(),
        },
        &paths,
        &coordinator.handle,
        peer(),
        None,
        OPERATOR,
        &RequestEnv {
            clock: &coordinator.clock,
            shutdown: &shutdown,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::HostUnavailable);
    assert_eq!(error.detail, format!("no path to {}", tag_for(peer(), 0)));
    assert!(
        !paths.node_dir.join(MARKER_FILE).exists(),
        "a refused request writes nothing"
    );
    assert!(!shutdown.is_cancelled(), "and stops nothing");
    coordinator.stop().await;
}

#[tokio::test]
async fn request_writes_marker_and_stops_daemon() {
    let iso = TestIsolation::new("rs-request");
    let paths = resolve_seeded(iso.state_root());
    let coordinator = start_coordinator();
    let targets = {
        let store = MemorySyncStore::new();
        store.add_target("peer.example");
        store.targets()
    };
    let shutdown = Cancellation::default();
    request(
        &Directory { targets },
        &paths,
        &coordinator.handle,
        peer(),
        Some(vec![peer()]),
        OPERATOR,
        &RequestEnv {
            clock: &coordinator.clock,
            shutdown: &shutdown,
        },
    )
    .await
    .unwrap();
    let marker = ResyncMarker::read(&paths).unwrap().expect("the marker");
    assert_eq!(marker.phase, ResyncPhase::Requested);
    assert_eq!(marker.from, peer());
    assert_eq!(marker.logs, Some(vec![peer()]));
    assert_eq!(marker.operator, OPERATOR);
    assert_eq!(
        marker.targets,
        vec![ResyncTarget {
            ssh_dest: "peer.example".to_owned(),
            remote_command: None,
        }]
    );
    assert!(shutdown.is_cancelled(), "the graceful stop was triggered");
    coordinator.stop().await;
}

// ---- the rebuild, with two real daemons -----------------------------------

fn db_of(state: &Path) -> PathBuf {
    state.join("node").join("herdr-threads.db")
}

fn open_ro(state: &Path) -> Option<Connection> {
    let conn = Connection::open_with_flags(db_of(state), OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    conn.busy_timeout(Duration::from_secs(5)).ok()?;
    Some(conn)
}

/// The incarnations of `node`'s logs that `state`'s store holds, ascending.
/// (`is_self` marks every incarnation of the store's own node, so it does not
/// tell the current one from a salvaged one.)
fn incarnations_of(state: &Path, node: NodeUuid) -> Vec<i64> {
    let Some(conn) = open_ro(state) else {
        return Vec::new();
    };
    let Ok(mut stmt) =
        conn.prepare("SELECT incarnation FROM sync_logs WHERE node_uuid = ?1 ORDER BY incarnation")
    else {
        return Vec::new();
    };
    stmt.query_map([node.to_string()], |r| r.get(0))
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
}

/// Whether the store in `state` holds a journal entry with this hash.
fn holds(state: &Path, hash: &[u8; 32]) -> bool {
    open_ro(state).is_some_and(|conn| {
        conn.query_row(
            "SELECT count(*) FROM journal WHERE hash = ?1",
            [hash.as_slice()],
            |r| r.get::<_, i64>(0),
        )
        .is_ok_and(|n| n > 0)
    })
}

fn dial_targets(state: &Path) -> Vec<(String, String)> {
    let Some(conn) = open_ro(state) else {
        return Vec::new();
    };
    let Ok(mut stmt) = conn.prepare("SELECT ssh_dest, state FROM sync_dial_targets") else {
        return Vec::new();
    };
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
}

fn backup_dirs(state: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(state.join("node").join("backups")) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries.map(|e| e.unwrap().path()).collect();
    dirs.sort();
    dirs
}

fn requested(from: NodeUuid, targets: Vec<ResyncTarget>) -> ResyncMarker {
    ResyncMarker {
        v: 1,
        from,
        logs: None,
        operator: OPERATOR.to_owned(),
        requested_at: 1_700_000_000_000,
        phase: ResyncPhase::Requested,
        backup_dir: None,
        targets,
    }
}

#[test]
fn restart_backs_up_moves_new_incarnation_restores_target_salvages_and_removes_marker() {
    // Node 1 dials node 0, and is the one rebuilt from it.
    let mut cluster = Cluster::start(
        "rs-rebuild",
        BIN,
        2,
        ClusterOptions {
            links: vec![(1, 0)],
            ..Default::default()
        },
    );
    cluster.wait_converged(WAIT);
    let rebuilt = cluster.state_dir(1).to_path_buf();
    let peer_state = cluster.state_dir(0).to_path_buf();
    let (rebuilt_uuid, peer_uuid) = (cluster.node_uuid(1), cluster.node_uuid(0));
    let before = incarnations_of(&rebuilt, rebuilt_uuid);
    assert_eq!(before.len(), 1, "node 1 authors into one incarnation");
    let old_incarnation = before[0];

    // Behind a partition each node authors one fact the other never sees.
    cluster.partition(0, 1);
    let (_, lost) = cluster.author_kind::<NoteKind>(1, &note("authored while the link was down"));
    let (_, theirs) = cluster.author_kind::<NoteKind>(0, &note("the peer's fact"));
    assert!(holds(&rebuilt, &lost));
    assert!(!holds(&peer_state, &lost), "the peer never saw it");
    assert!(!holds(&rebuilt, &theirs));

    // The operator's request, as `request` leaves it: the node is down and
    // the marker names the peer and the way to reach it.
    cluster.stop_node(1);
    let paths = NodePaths::resolve_read_only(&rebuilt).unwrap();
    requested(
        peer_uuid,
        vec![ResyncTarget {
            ssh_dest: peer_state.display().to_string(),
            remote_command: None,
        }],
    )
    .write(&paths)
    .unwrap();
    assert!(backup_dirs(&rebuilt).is_empty());

    cluster.heal(0, 1);
    cluster.start_node(1);

    // The marker ends the rebuild once the salvage ran.
    cluster.wait_until("the marker to be removed", WAIT, || {
        (!rebuilt.join("node").join(MARKER_FILE).exists()).then_some(())
    });
    let backups = backup_dirs(&rebuilt);
    assert_eq!(backups.len(), 1, "{backups:?}");
    assert!(
        backups[0].join("herdr-threads.db").is_file(),
        "the vacuumed copy"
    );
    assert!(
        backups[0].join("orig").join("herdr-threads.db").is_file(),
        "the moved files"
    );

    // A new incarnation, and the old one is an ordinary log now.
    let logs = incarnations_of(&rebuilt, rebuilt_uuid);
    assert_eq!(
        logs.len(),
        2,
        "the new incarnation and the old one: {logs:?}"
    );
    assert!(
        logs.contains(&old_incarnation),
        "the old incarnation was salvaged: {logs:?}"
    );

    // The fact authored while the link was down came back from the backup,
    // the peer's fact was pulled, and the dial target is restored.
    assert!(holds(&rebuilt, &lost), "salvage recovered the lost fact");
    cluster.wait_until("the peer's fact to be pulled", WAIT, || {
        holds(&rebuilt, &theirs).then_some(())
    });
    cluster.wait_until("the dial target to be restored and linked", WAIT, || {
        (dial_targets(&rebuilt) == vec![(peer_state.display().to_string(), "linked".to_owned())])
            .then_some(())
    });
    // The salvaged fact reaches the peer like any fact of an old incarnation.
    cluster.wait_until("the peer to learn the salvaged fact", WAIT, || {
        holds(&peer_state, &lost).then_some(())
    });
}

// ---- a crash after `moved` ----------------------------------------------------

/// One in-process store-only elected daemon.
struct Daemon {
    stop: Cancellation,
    thread: Option<std::thread::JoinHandle<std::io::Result<bool>>>,
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl Daemon {
    /// Starts the daemon; `Err` is the start failure it reported instead of
    /// becoming ready.
    fn start(paths: &NodePaths) -> Result<Self, std::io::Error> {
        let guard = ONE_DAEMON.lock().unwrap_or_else(|e| e.into_inner());
        let paths = paths.clone();
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let stop = Cancellation::default();
        let plan = FakeHostWorld::new().plan(None);
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let daemon_stop = stop.clone();
        let thread = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(run_elected_probed(
                    &paths,
                    clock,
                    daemon_stop,
                    ServiceConfig::default(),
                    plan,
                    LaneProbe::default(),
                    move |descriptor| {
                        ready_tx
                            .send(descriptor.clone())
                            .map_err(std::io::Error::other)
                    },
                ))
        });
        let mut daemon = Self {
            stop,
            thread: Some(thread),
            _guard: guard,
        };
        match ready_rx.recv_timeout(Duration::from_secs(30)) {
            Ok(_) => {
                // The elected daemon installs a quiet panic hook; restore
                // visible failures.
                std::panic::set_hook(Box::new(|info| eprintln!("{info}")));
                Ok(daemon)
            }
            Err(_) => {
                let thread = daemon.thread.take().expect("the daemon thread");
                match thread.join().expect("daemon thread") {
                    Err(error) => Err(error),
                    Ok(_) => panic!("the daemon exited without becoming ready"),
                }
            }
        }
    }

    fn shut_down(&mut self) {
        self.stop.cancel();
        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .expect("daemon thread")
                .expect("daemon drained cleanly");
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        self.shut_down();
    }
}

fn until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn crash_after_moved_resumes_without_second_backup() {
    let iso = TestIsolation::new("rs-crash");
    let paths = resolve_seeded(iso.state_root());

    // A node that has run once, so there is a store to rebuild.
    drop(Daemon::start(&paths).expect("the first start"));
    assert!(paths.database_path.is_file());
    let own = NodeIdentity::from_seed(
        fs::read(&paths.key_path)
            .unwrap()
            .try_into()
            .expect("a 32-byte key"),
    )
    .uuid();
    let old = incarnations_of(iso.state_root(), own);
    assert_eq!(old.len(), 1, "one incarnation so far: {old:?}");

    // No targets: nothing is dialed. The peer is a node nobody serves.
    requested(peer(), Vec::new()).write(&paths).unwrap();

    // The crash: the start dies right after the `moved` marker write.
    let armed = Failpoint::error(
        "resync.after_moved",
        paths.node_dir.display().to_string(),
        ErrorCode::StoreBusy,
    );
    let error = Daemon::start(&paths)
        .err()
        .expect("the armed start must fail");
    assert!(error.to_string().contains("resync"), "{error}");
    assert_eq!(armed.fired(), 1);
    drop(armed);
    let marker = ResyncMarker::read(&paths)
        .unwrap()
        .expect("marker survives");
    assert_eq!(marker.phase, ResyncPhase::Moved);
    assert!(
        !paths.database_path.exists(),
        "the old store was moved aside"
    );
    let backups = backup_dirs(iso.state_root());
    assert_eq!(backups.len(), 1, "{backups:?}");

    // The next start resumes at the plan: no second backup, a new
    // incarnation, the old one salvaged, the marker gone.
    let daemon = Daemon::start(&paths).expect("the resumed start");
    until("the marker to be removed", || {
        ResyncMarker::read(&paths).unwrap().is_none()
    });
    assert_eq!(backup_dirs(iso.state_root()), backups, "still one backup");
    let logs = incarnations_of(iso.state_root(), own);
    assert_eq!(
        logs.len(),
        2,
        "the new incarnation and the salvaged one: {logs:?}"
    );
    assert!(logs.contains(&old[0]), "{logs:?} vs {old:?}");
    drop(daemon);
}
