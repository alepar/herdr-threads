//! SyncAdmin tests: real `SqliteStore`s, a real coordinator thread and, for
//! the clone-clearance acceptance, a real incarnations worker thread. The
//! peer is played by the test (it answers the worker's `branch_want`).

use super::*;

use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use rusqlite::Transaction;
use serde::{Deserialize, Serialize};

use crate::node::{NodeIdentity, NodeUuid};
use crate::protocol::authority::PeerIdentity;
use crate::protocol::sync_admin::{SyncReply as Reply, UnappliedRow};
use crate::protocol::time::{MonoInstant, UtcMillis};
use crate::service::pacer::Pacer;
use crate::service::workers::WorkerStatus;
use crate::store::StoreSettings;
use crate::store::connection::{DecisionInstant, StoreContext};
use crate::store::journal;
use crate::sync::EntityKey;
use crate::sync::coordinator::{
    BranchReply, CloneKind, CloneReport, CoordinatorDeps, ForkSink, LogOnlyForkSink,
    SyncCoordinator,
};
use crate::sync::dialer::DialConfig;
use crate::sync::envelope::{Envelope, StoredEntry, hex_encode};
use crate::sync::facts::{ApplyContext, FactError, FactKind, FactRegistry, Predecessor};
use crate::sync::frame::{HaveLog, WireItem};
use crate::sync::incarnations::{HELD_LINE, IncarnationsWorker, Job, WorkerDeps};
use crate::sync::journal_port::{IngestOutcome, JournalPort};
use crate::sync::{BatchLimits, LogKey};
use crate::test_support::isolation::TestIsolation;

const SEED_A: [u8; 32] = [1; 32];
const T0: i64 = 1_700_000_000_000;
const UID: u32 = 501;

#[derive(Serialize, Deserialize)]
struct Put {
    id: String,
}

struct PutKind;
impl FactKind for PutKind {
    const NAME: &'static str = "test.put";
    const MAJOR: u32 = 1;
    type Payload = Put;
    fn creates(p: &Put) -> Option<EntityKey> {
        EntityKey::new(format!("e:{}", p.id)).ok()
    }
    fn needs(_: &Put) -> Vec<EntityKey> {
        Vec::new()
    }
    fn apply(_: &Transaction<'_>, _: &mut ApplyContext<'_>, _: Put) -> Result<(), FactError> {
        Ok(())
    }
}

fn registry() -> &'static FactRegistry {
    FactRegistry::builtins_plus(|b| b.with::<PutKind>())
}

struct ManualClock {
    mono: AtomicU64,
    utc: AtomicI64,
}
impl ManualClock {
    fn new(utc: i64) -> Arc<Self> {
        Arc::new(Self {
            mono: AtomicU64::new(1_000),
            utc: AtomicI64::new(utc),
        })
    }
    fn advance(&self, ms: u64) {
        self.mono.fetch_add(ms, Ordering::SeqCst);
        self.utc
            .fetch_add(i64::try_from(ms).unwrap(), Ordering::SeqCst);
    }
    fn set_utc(&self, utc: i64) {
        self.utc.store(utc, Ordering::SeqCst);
    }
}
impl Clock for ManualClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(self.utc.load(Ordering::SeqCst))
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.mono.load(Ordering::SeqCst))
    }
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Cancellation::default(),
    }
}

fn actor() -> OperatorActor {
    OperatorActor::from_peer(PeerIdentity::from_kernel(UID), UID).expect("the owner's peer")
}

/// Polls `condition` for up to `secs` seconds.
fn wait_for(secs: u64, what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    while !condition() {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// One node: a real store with its journal open.
struct Node {
    iso: TestIsolation,
    store: Arc<SqliteStore>,
    clock: Arc<ManualClock>,
    sink: Arc<IncarnationsSink>,
    lines: Arc<Mutex<Vec<String>>>,
}

impl Node {
    fn new(seed: [u8; 32], incarnation_seed: u64) -> Self {
        let clock = ManualClock::new(T0);
        let iso = TestIsolation::new("sync-admin");
        let ctx = StoreContext::new(iso.state_root().join("store.db"), clock.clone())
            .with_journal_seed(seed);
        ctx.seed_incarnations(incarnation_seed);
        let id = Arc::new(NodeIdentity::from_seed(seed));
        {
            let mut writer = ctx.open_writer().expect("open a writer");
            journal::open(&ctx, &mut writer, id.clone(), "host", registry())
                .expect("open the journal");
        }
        let store = Arc::new(
            SqliteStore::new(ctx, id.uuid().to_string(), StoreSettings::default())
                .expect("open the store"),
        );
        let lines = Arc::new(Mutex::new(Vec::new()));
        let log = {
            let lines = lines.clone();
            Arc::new(move |line: &str| lines.lock().unwrap().push(line.to_owned())) as LogFn
        };
        let sink = Arc::new(IncarnationsSink::new(store.clone(), clock.clone(), log));
        Self {
            iso,
            store,
            clock,
            sink,
            lines,
        }
    }

    fn put(&self, id: &str) {
        let ctx = self.store.context();
        let mut conn = ctx.open_writer().expect("open a writer");
        ctx.execute_decision(
            &mut conn,
            |_| Ok(()),
            |tx, d, ()| journal::author::<PutKind>(tx, ctx, d, &Put { id: id.into() }),
        )
        .expect("author a put");
    }

    fn rotate_to(&self, incarnation: u32, predecessor: Option<Predecessor>) {
        let ctx = self.store.context();
        let mut conn = ctx.open_writer().expect("open a writer");
        ctx.execute_decision(
            &mut conn,
            |_| Ok(()),
            |tx, d, ()| journal::begin_incarnation(tx, ctx, d, incarnation, predecessor, "host"),
        )
        .expect("begin an incarnation");
    }

    fn active_log(&self) -> LogKey {
        let db = self.store.context().open_query(budget()).unwrap();
        let (node, incarnation): (String, u32) = db
            .query_row(
                "SELECT l.node_uuid,l.incarnation FROM local_node n \
                 JOIN sync_logs l ON l.log_ix=n.active_log_ix",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        LogKey {
            node: node.parse().unwrap(),
            incarnation,
        }
    }

    fn entries(&self, log: &LogKey, from: u64) -> Vec<Envelope> {
        self.store
            .serve(log, from, BatchLimits::default(), &budget())
            .unwrap()
            .into_iter()
            .map(|entry| entry.as_known().cloned().expect("a known entry"))
            .collect()
    }

    fn ingest(&self, from: &Node, log: &LogKey, first: u64) {
        let entries = from.entries(log, first);
        let entries = entries.into_iter().map(StoredEntry::from).collect();
        match self.store.ingest(log, entries, &budget()).unwrap() {
            IngestOutcome::Stored { .. } => {}
            other => panic!("ingest of {log:?}: {other:?}"),
        }
    }

    fn count(&self, sql: &str) -> i64 {
        let db = self.store.context().open_query(budget()).unwrap();
        db.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    fn quantum<R>(
        &self,
        f: impl FnOnce(&Transaction<'_>, &StoreContext, DecisionInstant) -> Result<R, ApiError>,
    ) -> R {
        self.store.sync_operator_quantum(&budget(), f).unwrap()
    }
}

/// A node with a running coordinator and the admin built over it.
struct Rig {
    node: Node,
    admin: SyncAdmin,
    coordinator_stop: Cancellation,
    join: Option<tokio::task::JoinHandle<()>>,
    // Dropped last: the coordinator and admin await through it.
    _runtime: tokio::runtime::Runtime,
}

impl Rig {
    fn new(node: Node) -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let clock: Arc<dyn Clock> = node.clock.clone();
        let coordinator_stop = Cancellation::default();
        let writer = Arc::new(FairWriter::new(8));
        let log: LogFn = Arc::new(|_| {});
        let hold = node
            .store
            .context()
            .sync()
            .unwrap()
            .own_history_hold()
            .clone();
        let _enter = runtime.enter();
        let (handle, _intake, join) = SyncCoordinator::start(CoordinatorDeps {
            journal: node.store.clone(),
            sync_store: node.store.clone(),
            identity: Arc::new(NodeIdentity::from_seed(SEED_A)),
            boot: uuid::Uuid::from_u128(9),
            writer: writer.clone(),
            clock: clock.clone(),
            pacer: Arc::new(Pacer::new("sync", clock.clone(), coordinator_stop.clone())),
            status: Arc::new(WorkerStatus::default()),
            log: log.clone(),
            fork_sink: Arc::new(LogOnlyForkSink::new(log.clone(), clock.clone())),
            own_history: hold,
            dial: DialConfig::default(),
            cancel: coordinator_stop.clone(),
            resync_pin: None,
        });
        let state = node.iso.state_root().join("state");
        std::fs::create_dir_all(&state).unwrap();
        let paths = NodePaths::resolve(&state).unwrap();
        std::fs::create_dir_all(&paths.node_dir).unwrap();
        let lines = node.lines.clone();
        let admin = SyncAdmin {
            store: node.store.clone(),
            sync: handle,
            sink: node.sink.clone(),
            clock,
            paths,
            shutdown: Cancellation::default(),
            writer,
            log: Arc::new(move |line: &str| lines.lock().unwrap().push(line.to_owned())),
            runtime: runtime.handle().clone(),
            discovery: None,
        };
        Self {
            node,
            admin,
            coordinator_stop,
            join: Some(join),
            _runtime: runtime,
        }
    }

    fn mutate(&self, command: SyncCommand) -> Result<Reply, ApiError> {
        self.admin.run(command, Some(&actor()), &budget())
    }

    fn read(&self, command: SyncCommand) -> Result<Reply, ApiError> {
        self.admin.run(command, None, &budget())
    }

    fn status(&self) -> StatusReply {
        match self.read(SyncCommand::Status { log: None }).unwrap() {
            Reply::Status(status) => *status,
            other => panic!("{other:?}"),
        }
    }

    fn peers(&self) -> Vec<crate::protocol::sync_admin::PeerRow> {
        match self.read(SyncCommand::PeerList).unwrap() {
            Reply::Peers { rows, .. } => rows,
            other => panic!("{other:?}"),
        }
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.coordinator_stop.cancel();
        if let Some(join) = self.join.take() {
            let _ = self._runtime.block_on(join);
        }
    }
}

fn text(reply: Reply) -> String {
    let bytes = crate::protocol::output::encode_selected(
        &crate::protocol::results::CommandResult::Sync(reply),
        &crate::protocol::output::OutputSpec {
            format: crate::protocol::output::OutputFormat::Text,
            context: Default::default(),
        },
    )
    .unwrap();
    String::from_utf8(bytes).unwrap()
}

// Kills: a peer add that does not reach the table, a list that hides the row,
// a forget that deletes instead of disabling, a forget by node that matches
// nothing, and a forget of nothing that reports success.
#[test]
fn peer_add_list_forget_round_trip() {
    let rig = Rig::new(Node::new(SEED_A, 1));
    let added = rig
        .mutate(SyncCommand::PeerAdd {
            ssh_dest: "alice@host".into(),
            remote_command: Some("ht --state-dir /x".into()),
        })
        .unwrap();
    let Reply::PeerAdded { ssh_dest, .. } = added else {
        panic!("{added:?}")
    };
    assert_eq!(ssh_dest, "alice@host");
    let rows = rig.peers();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].enabled);
    assert_eq!(rows[0].remote_command.as_deref(), Some("ht --state-dir /x"));
    assert_eq!(rows[0].source, "explicit");

    let forgotten = rig
        .mutate(SyncCommand::PeerForget {
            target: "alice@host".into(),
        })
        .unwrap();
    assert_eq!(forgotten, Reply::PeerForgotten { disabled: 1 });
    let rows = rig.peers();
    assert_eq!(rows.len(), 1, "forgetting deletes nothing");
    assert!(!rows[0].enabled);
    let again = rig
        .mutate(SyncCommand::PeerForget {
            target: "alice@host".into(),
        })
        .unwrap_err();
    assert_eq!(again.code, ErrorCode::NotFound);

    // By node: a learned node's tag forgets its target.
    rig.mutate(SyncCommand::PeerAdd {
        ssh_dest: "bob@host".into(),
        remote_command: None,
    })
    .unwrap();
    let peer = "ee".repeat(16);
    let db = rig.node.store.context().open_writer().unwrap();
    db.execute(
        "UPDATE sync_dial_targets SET learned_node_uuid=?1 WHERE ssh_dest='bob@host'",
        [&peer],
    )
    .unwrap();
    db.execute(
        "INSERT INTO node_directory(node_uuid,tag,tag_k,first_seen_at) VALUES (?1,'Pq7',0,0)",
        [&peer],
    )
    .unwrap();
    drop(db);
    let by_node = rig
        .mutate(SyncCommand::PeerForget {
            target: "Pq7".into(),
        })
        .unwrap();
    assert_eq!(by_node, Reply::PeerForgotten { disabled: 1 });
    assert!(rig.peers().iter().all(|row| !row.enabled));
}

// Kills: control characters or a leading dash reaching the table, a 65th
// target, and a re-add of an enabled target counted against the cap.
#[test]
fn peer_add_refuses_control_chars_and_cap() {
    let rig = Rig::new(Node::new(SEED_A, 1));
    for bad in ["a\nb@host", "a\u{1b}[31m@host", ""] {
        let error = rig
            .mutate(SyncCommand::PeerAdd {
                ssh_dest: bad.into(),
                remote_command: None,
            })
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest, "{bad:?}");
    }
    let error = rig
        .mutate(SyncCommand::PeerAdd {
            ssh_dest: "-oProxyCommand=x".into(),
            remote_command: None,
        })
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    let error = rig
        .mutate(SyncCommand::PeerAdd {
            ssh_dest: "ok@host".into(),
            remote_command: Some("x\ny".into()),
        })
        .unwrap_err();
    assert_eq!(
        error.code,
        ErrorCode::InvalidRequest,
        "command control chars"
    );
    assert!(rig.peers().is_empty(), "nothing was stored");

    for n in 0..64 {
        rig.mutate(SyncCommand::PeerAdd {
            ssh_dest: format!("u{n}@host"),
            remote_command: None,
        })
        .unwrap();
    }
    let over = rig
        .mutate(SyncCommand::PeerAdd {
            ssh_dest: "u64@host".into(),
            remote_command: None,
        })
        .unwrap_err();
    assert_eq!(over.code, ErrorCode::InvalidRequest);
    assert_eq!(over.detail, "too many dial targets (64)");
    rig.mutate(SyncCommand::PeerAdd {
        ssh_dest: "u0@host".into(),
        remote_command: None,
    })
    .expect("re-adding an enabled target does not grow the count");
    assert_eq!(rig.peers().len(), 64);
}

// Kills: a mutating command that runs without the operator actor, and a read
// that demands one.
#[test]
fn mutating_commands_need_the_operator_actor() {
    let rig = Rig::new(Node::new(SEED_A, 1));
    let mutating = [
        SyncCommand::PeerAdd {
            ssh_dest: "a@h".into(),
            remote_command: None,
        },
        SyncCommand::PeerForget {
            target: "a@h".into(),
        },
        SyncCommand::UnappliedReplay,
        SyncCommand::Resync {
            from: "Pq7".into(),
            logs: Vec::new(),
        },
        SyncCommand::CloneCleared,
    ];
    for command in mutating {
        assert!(command.is_mutating());
        let error = rig.read(command.clone()).unwrap_err();
        assert_eq!(error.code, ErrorCode::Unauthorized, "{command:?}");
    }
    assert!(rig.peers().is_empty(), "the refused add stored nothing");
    for command in [
        SyncCommand::PeerList,
        SyncCommand::Status { log: None },
        SyncCommand::Verify,
        SyncCommand::UnappliedList {
            after: None,
            limit: 8,
        },
    ] {
        assert!(!command.is_mutating());
        rig.read(command).expect("reads need no operator");
    }
}

fn restored_with_unknown_newer() -> (Node, Node) {
    let orig = Node::new(SEED_A, 600);
    orig.clock.advance(1_000);
    orig.rotate_to(501, None);
    let restored = Node::new(SEED_A, 600);
    let newer = LogKey {
        node: orig.active_log().node,
        incarnation: 501,
    };
    restored.ingest(&orig, &newer, 1);
    // The restored node moved on after the rotation: no restore successor.
    restored.clock.set_utc(T0 + 100_000);
    restored.put("late");
    (orig, restored)
}

// Kills: a status that omits an open own_unknown_newer row, one that prints
// one remedy instead of both, evidence shown as cleared, and a status that
// shows remedies on a healthy store.
#[test]
fn sync_status_open_own_unknown_newer_prints_both_remedies() {
    let (_orig, restored) = restored_with_unknown_newer();
    let recorded =
        restored.quantum(|tx, ctx, d| incarnations::record_own_unknown_newer(tx, ctx, d.utc));
    assert!(recorded >= 1, "{recorded}");
    let rig = Rig::new(restored);
    let status = rig.status();
    assert!(
        status
            .evidence
            .iter()
            .any(|row| row.kind == "own_unknown_newer"),
        "{:?}",
        status.evidence
    );
    assert_eq!(status.own_unknown_newer.len(), 1);
    assert_eq!(status.remedies.len(), 2);
    assert!(
        status.remedies[0].starts_with("two live copies: move one copy's <state_dir>/node/ aside")
    );
    assert!(status.remedies[0].contains("sync clone-cleared"));
    assert!(status.remedies[0].contains("peer add"));
    assert!(
        status.remedies[1].starts_with("one restored copy: run `herdr-threads sync clone-cleared`")
    );
    assert!(!status.coordinator_unavailable);
    assert_eq!(
        status.logs.iter().filter(|log| log.is_self).count(),
        2,
        "both own incarnations are listed"
    );

    let rendered = text(Reply::Status(Box::new(status)));
    assert!(rendered.starts_with("sync_status\n"), "{rendered}");
    assert_eq!(
        rendered
            .lines()
            .filter(|l| l.starts_with("remedy: "))
            .count(),
        2,
        "{rendered}"
    );
    assert!(rendered.contains("two live copies"), "{rendered}");
    assert!(rendered.contains("one restored copy"), "{rendered}");
    assert!(
        rendered
            .lines()
            .any(|l| l.starts_with("clone_evidence ") && l.contains("own_unknown_newer")),
        "{rendered}"
    );
    assert!(
        rendered
            .lines()
            .any(|l| l.starts_with("own_unknown_newer ")),
        "{rendered}"
    );

    let healthy = Rig::new(Node::new(SEED_A, 1));
    let status = healthy.status();
    assert!(status.remedies.is_empty());
    assert!(status.evidence.is_empty());
}

// Kills: a blocked_on or cannot_reissue field missing from the JSON or the
// text, one printed on an unblocked log, and a decode that needs the new
// fields (an older daemon's reply has neither).
#[test]
fn status_json_carries_blocked_on() {
    use crate::protocol::sync_admin::{LogRow, RepairRow};
    let rig = Rig::new(Node::new(SEED_A, 1));
    let mut status = rig.status();
    let plain = text(Reply::Status(Box::new(status.clone())));
    assert!(!plain.contains(" blocked="), "{plain}");
    assert!(!plain.contains("cannot_reissue"), "{plain}");

    status.logs[0].blocked_on = Some("opaque_layout".into());
    status.repairs.push(RepairRow {
        log: "aaaaaaaa/1".into(),
        frozen_at: Some(2),
        origin: None,
        evidence: 0,
        branches: 0,
        reissued: 1,
        pending: 0,
        cannot_reissue: 3,
    });
    let json = serde_json::to_value(&status).unwrap();
    assert_eq!(json["logs"][0]["blocked_on"], "opaque_layout");
    assert_eq!(json["repairs"][0]["cannot_reissue"], 3);
    let rendered = text(Reply::Status(Box::new(status.clone())));
    let log_line = rendered.lines().find(|l| l.starts_with("log ")).unwrap();
    assert!(log_line.ends_with(" blocked=opaque_layout"), "{log_line}");
    let repair_line = rendered.lines().find(|l| l.starts_with("repair ")).unwrap();
    assert!(repair_line.ends_with(" cannot_reissue=3"), "{repair_line}");

    // An older reply without the fields still decodes; None is not serialized.
    let mut old = serde_json::to_value(&status).unwrap();
    old["logs"][0].as_object_mut().unwrap().remove("blocked_on");
    old["repairs"][0]
        .as_object_mut()
        .unwrap()
        .remove("cannot_reissue");
    let decoded: crate::protocol::sync_admin::StatusReply = serde_json::from_value(old).unwrap();
    let row: &LogRow = &decoded.logs[0];
    assert_eq!(row.blocked_on, None);
    assert_eq!(decoded.repairs[0].cannot_reissue, 0);
    assert!(
        !serde_json::to_string(row).unwrap().contains("blocked_on"),
        "None is omitted"
    );
}

// Kills: a parked_layout count missing from the text or JSON, one printed on a
// log with none, and a decode that needs the field (an older daemon's reply
// has none).
#[test]
fn status_reports_parked_layout() {
    let rig = Rig::new(Node::new(SEED_A, 1));
    let mut status = rig.status();
    let plain = text(Reply::Status(Box::new(status.clone())));
    assert!(!plain.contains("parked_layout"), "{plain}");
    assert!(
        serde_json::to_value(&status).unwrap()["logs"][0]
            .get("parked_layout")
            .is_none()
    );

    status.logs[0].parked_layout = 4;
    let rendered = text(Reply::Status(Box::new(status.clone())));
    let log_line = rendered.lines().find(|l| l.starts_with("log ")).unwrap();
    assert!(log_line.ends_with(" parked_layout=4"), "{log_line}");
    let mut json = serde_json::to_value(&status).unwrap();
    assert_eq!(json["logs"][0]["parked_layout"], 4);
    json["logs"][0]
        .as_object_mut()
        .unwrap()
        .remove("parked_layout");
    let decoded: crate::protocol::sync_admin::StatusReply = serde_json::from_value(json).unwrap();
    assert_eq!(decoded.logs[0].parked_layout, 0);
}

// Kills: a withheld_logs count missing from the JSON or the link line, one
// printed on a link that parked nothing, a decode that needs the field (an
// older daemon's reply has none) and one that drops the pre-rename `withheld`
// key.
#[test]
fn status_reports_link_withheld_logs() {
    use crate::protocol::sync_admin::LinkRow;
    let rig = Rig::new(Node::new(SEED_A, 1));
    let mut status = rig.status();
    let link = |withheld_logs| LinkRow {
        node: "aB3".into(),
        mode: "active".into(),
        role: "dialer".into(),
        rx_logs: 1,
        unsolicited: 0,
        withheld_logs,
        dest: None,
        last_error: None,
        since: None,
        last_rx: None,
        app_rx: None,
        subscriptions: 0,
        peer_lag: None,
        stalled: vec![],
    };
    status.links = vec![link(0)];
    let plain = text(Reply::Status(Box::new(status.clone())));
    let line = plain.lines().find(|l| l.starts_with("link ")).unwrap();
    assert_eq!(line, "link aB3 active dialer rx_logs=1 unsolicited=0");

    status.links = vec![link(2)];
    let rendered = text(Reply::Status(Box::new(status.clone())));
    let line = rendered.lines().find(|l| l.starts_with("link ")).unwrap();
    assert_eq!(
        line,
        "link aB3 active dialer rx_logs=1 unsolicited=0 withheld_logs=2"
    );
    let json = serde_json::to_value(&status).unwrap();
    assert_eq!(json["links"][0]["withheld_logs"], 2);

    let mut old = json.clone();
    old["links"][0]
        .as_object_mut()
        .unwrap()
        .remove("withheld_logs");
    let decoded: crate::protocol::sync_admin::StatusReply = serde_json::from_value(old).unwrap();
    assert_eq!(decoded.links[0].withheld_logs, 0);

    let mut renamed = json;
    let link = renamed["links"][0].as_object_mut().unwrap();
    let count = link.remove("withheld_logs").unwrap();
    link.insert("withheld".into(), count);
    let decoded: crate::protocol::sync_admin::StatusReply =
        serde_json::from_value(renamed).unwrap();
    assert_eq!(decoded.links[0].withheld_logs, 2);
}

// Kills: a lineage-branch row without its remedy lines.
#[test]
fn sync_status_open_lineage_branch_prints_both_remedies() {
    let node = Node::new(SEED_A, 600);
    node.put("p");
    let i1 = node.active_log();
    node.clock.advance(1_000);
    node.rotate_to(
        501,
        Some(Predecessor {
            incarnation: i1.incarnation,
            frozen_at: None,
        }),
    );
    node.clock.advance(1_000);
    node.rotate_to(
        502,
        Some(Predecessor {
            incarnation: i1.incarnation,
            frozen_at: None,
        }),
    );
    let recorded =
        node.quantum(|tx, ctx, d| incarnations::record_own_unknown_newer(tx, ctx, d.utc));
    assert_eq!(recorded, 1, "two geneses name I1");
    let rig = Rig::new(node);
    let status = rig.status();
    assert_eq!(status.lineage_branches.len(), 1);
    assert_eq!(status.evidence.len(), 1);
    assert_eq!(status.evidence[0].kind, "lineage_branch");
    assert_eq!(status.remedies.len(), 2);
    let rendered = text(Reply::Status(Box::new(status)));
    assert!(
        rendered.lines().any(|l| l.starts_with("lineage_branch ")),
        "{rendered}"
    );
    assert_eq!(
        rendered
            .lines()
            .filter(|l| l.starts_with("remedy: "))
            .count(),
        2
    );
}

// Kills: a verify that flags a healthy store, and one that misses a seeded
// I2 violation.
#[test]
fn sync_verify_healthy_and_seeded_violation() {
    let rig = Rig::new(Node::new(SEED_A, 1));
    rig.node.put("x");
    assert_eq!(
        rig.read(SyncCommand::Verify).unwrap(),
        Reply::Verified {
            violations: Vec::new()
        }
    );
    assert_eq!(text(rig.read(SyncCommand::Verify).unwrap()), "verify ok\n");

    // A foreign log frozen at 1 with a forked row at counter 1 (I2).
    let db = rig.node.store.context().open_writer().unwrap();
    db.execute(
        "INSERT INTO sync_logs(node_uuid,incarnation,is_self,pubkey,genesis_hash,contiguous_counter, \
         head_hash,applied_through,state,frozen_at_counter,first_seen_at) \
         VALUES (?1,1,0,zeroblob(32),zeroblob(32),1,zeroblob(32),1,'frozen',1,0)",
        [&"cc".repeat(16)],
    )
    .unwrap();
    let log_ix = db.last_insert_rowid();
    db.execute(
        "INSERT INTO journal(log_ix,counter,prev_hash,hash,hlc,kind,kind_version,deps,needs, \
         payload,signature,ingested_at,apply_state,applied_seq,raw) \
         VALUES (?1,1,zeroblob(32),zeroblob(32),1,'test.kind',1,x'0000','',x'7b7d',zeroblob(64),0,'applied',777777,x'48544a524e4c3031000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000')",
        [log_ix],
    )
    .unwrap();
    db.execute(
        "INSERT INTO journal_forked(log_ix,counter,prev_hash,hash,hlc,kind,kind_version,deps,needs, \
         payload,signature,received_at,raw) \
         VALUES (?1,1,zeroblob(32),x'0101010101010101010101010101010101010101010101010101010101010101',1,'test.kind',1,x'','',x'7b7d',zeroblob(64),0,x'48544a524e4c3031000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000')",
        [log_ix],
    )
    .unwrap();
    drop(db);
    let Reply::Verified { violations } = rig.read(SyncCommand::Verify).unwrap() else {
        panic!()
    };
    // The seeded entry's hash is made up, so J3 reports it after I2.
    assert_eq!(violations.len(), 2, "{violations:?}");
    assert_eq!(violations[0].invariant, "I2");
    assert_eq!(violations[1].invariant, "J3");
    assert_eq!(violations[1].log.as_deref(), Some("cccccccc/1"));
    assert_eq!(violations[1].counter, Some(1));
    let rendered = text(Reply::Verified { violations });
    assert!(
        rendered.starts_with("verify violations=2\nviolation I2 cccccccc/1 counter=1: "),
        "{rendered}"
    );
    assert!(
        rendered.contains("\nviolation J3 cccccccc/1 counter=1: hash does not match"),
        "{rendered}"
    );
}

// Kills: a verify that checks only linkage and so passes an entry whose
// stored hash is not the hash of its canonical bytes.
#[test]
fn sync_verify_reports_a_tampered_entry() {
    let rig = Rig::new(Node::new(SEED_A, 1));
    rig.node.put("x");
    assert_eq!(text(rig.read(SyncCommand::Verify).unwrap()), "verify ok\n");
    let db = rig.node.store.context().open_writer().unwrap();
    db.execute(
        "INSERT INTO sync_logs(node_uuid,incarnation,is_self,pubkey,genesis_hash,contiguous_counter, \
         head_hash,applied_through,state,first_seen_at) \
         VALUES (?1,1,0,zeroblob(32),zeroblob(32),1,x'0909090909090909090909090909090909090909090909090909090909090909',0,'active',0)",
        [&"ee".repeat(16)],
    )
    .unwrap();
    let log_ix = db.last_insert_rowid();
    db.execute(
        "INSERT INTO journal(log_ix,counter,prev_hash,hash,hlc,kind,kind_version,deps,needs, \
         payload,signature,ingested_at,apply_state,raw) \
         VALUES (?1,1,zeroblob(32),x'0909090909090909090909090909090909090909090909090909090909090909',1,'test.kind',1,x'0000','',x'7b7d',zeroblob(64),0,'failed',x'48544a524e4c3031000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000')",
        [log_ix],
    )
    .unwrap();
    drop(db);
    let Reply::Verified { violations } = rig.read(SyncCommand::Verify).unwrap() else {
        panic!()
    };
    assert_eq!(violations.len(), 1, "{violations:?}");
    assert_eq!(violations[0].invariant, "J3");
    assert_eq!(violations[0].log.as_deref(), Some("eeeeeeee/1"));
    assert_eq!(violations[0].counter, Some(1));
}

// Kills: a peer list or status that drops the discovery worker's state.
#[test]
fn peer_list_and_status_carry_the_discovery_state() {
    use crate::sync::discovery::{DiscoveryState, DiscoveryStatus};
    let mut rig = Rig::new(Node::new(SEED_A, 1));
    let status = Arc::new(DiscoveryStatus::default());
    status.set_for_test(DiscoveryState::NotAvailable);
    rig.admin.discovery = Some(status);
    let peers = rig.read(SyncCommand::PeerList).unwrap();
    let Reply::Peers { discovery, .. } = &peers else {
        panic!("{peers:?}")
    };
    assert_eq!(discovery.as_deref(), Some("not_available"));
    assert_eq!(text(peers).lines().nth(1), Some("discovery not_available"));
    assert_eq!(rig.status().discovery.as_deref(), Some("not_available"));
    let rendered = text(rig.read(SyncCommand::Status { log: None }).unwrap());
    assert_eq!(rendered.lines().nth(1), Some("discovery not_available"));
}

fn seed_unknown_kind_entries(node: &Node) {
    let db = node.store.context().open_writer().unwrap();
    db.execute(
        "INSERT INTO sync_logs(node_uuid,incarnation,is_self,pubkey,genesis_hash,contiguous_counter, \
         head_hash,applied_through,state,first_seen_at) \
         VALUES (?1,1,0,zeroblob(32),zeroblob(32),2,x'0202020202020202020202020202020202020202020202020202020202020202',0,'active',0)",
        ["dd".repeat(16)],
    )
    .unwrap();
    let log_ix = db.last_insert_rowid();
    for n in 1..=2i64 {
        db.execute(
            "INSERT INTO journal(log_ix,counter,prev_hash,hash,hlc,kind,kind_version,deps,needs, \
             payload,signature,ingested_at,apply_state,raw) \
             VALUES (?1,?2,?3,?4,1,'unknown.kind',1,x'','',x'7b7d',zeroblob(64),0,'parked_unknown',x'48544a524e4c3031000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000')",
            rusqlite::params![log_ix, n, vec![(n - 1) as u8; 32], vec![n as u8; 32]],
        )
        .unwrap();
    }
}

// Kills: a list that ignores the page bound or the continuation, and a replay
// that does not attempt the parked entries (or that fakes success for a kind
// the registry still does not know).
#[test]
fn sync_unapplied_list_and_replay() {
    let rig = Rig::new(Node::new(SEED_A, 1));
    seed_unknown_kind_entries(&rig.node);
    let Reply::Unapplied(first) = rig
        .read(SyncCommand::UnappliedList {
            after: None,
            limit: 1,
        })
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(first.rows.len(), 1);
    assert_eq!(first.parked_unknown, 2);
    let next = first.next_after.expect("one more entry");
    let UnappliedRow {
        counter,
        kind,
        state,
        ..
    } = &first.rows[0];
    assert_eq!(
        (*counter, kind.as_str(), state.as_str()),
        (1, "unknown.kind", "parked_unknown")
    );
    let rendered = text(Reply::Unapplied(first));
    assert!(
        rendered.starts_with("unapplied parked_unknown=2 parked_dep=0 failed=0\n"),
        "{rendered}"
    );
    assert!(
        rendered.contains(" #1 unknown.kind parked_unknown"),
        "{rendered}"
    );
    assert!(
        rendered.contains(&format!("sync unapplied list --after {next}")),
        "{rendered}"
    );

    let Reply::Replayed(replayed) = rig.mutate(SyncCommand::UnappliedReplay).unwrap() else {
        panic!()
    };
    assert_eq!(replayed.attempted, 2, "both entries were attempted");
    assert_eq!(replayed.applied, 0, "an unknown kind stays parked");
    assert!(!replayed.has_more);
    assert_eq!(
        rig.node
            .count("SELECT COUNT(*) FROM journal WHERE apply_state='parked_unknown'"),
        2
    );
}

// Kills: a resync that stops the daemon for an unknown node or one with no
// path, and one that, once allowed, writes no marker or does not stop.
#[test]
fn sync_resync_refusals_and_request() {
    let rig = Rig::new(Node::new(SEED_A, 1));
    let unknown = rig
        .mutate(SyncCommand::Resync {
            from: "Zz9".into(),
            logs: Vec::new(),
        })
        .unwrap_err();
    assert_eq!(unknown.code, ErrorCode::InvalidRequest);
    assert_eq!(unknown.detail, "unknown node");

    // A known node nothing reaches.
    let peer = "ee".repeat(16);
    let db = rig.node.store.context().open_writer().unwrap();
    db.execute(
        "INSERT INTO node_directory(node_uuid,tag,tag_k,first_seen_at) VALUES (?1,'Pq7',0,0)",
        [&peer],
    )
    .unwrap();
    let no_path = rig
        .mutate(SyncCommand::Resync {
            from: "Pq7".into(),
            logs: Vec::new(),
        })
        .unwrap_err();
    assert_eq!(no_path.code, ErrorCode::HostUnavailable);
    assert_eq!(no_path.detail, "no path to Pq7");
    assert!(
        !rig.admin.shutdown.is_cancelled(),
        "a refusal stops nothing"
    );
    assert!(
        crate::sync::resync::ResyncMarker::read(&rig.admin.paths)
            .unwrap()
            .is_none(),
        "a refusal writes no marker"
    );

    // With a dial target that learned the node there is a path.
    rig.mutate(SyncCommand::PeerAdd {
        ssh_dest: "pq@host".into(),
        remote_command: None,
    })
    .unwrap();
    db.execute(
        "UPDATE sync_dial_targets SET learned_node_uuid=?1 WHERE ssh_dest='pq@host'",
        [&peer],
    )
    .unwrap();
    drop(db);
    let reply = rig
        .mutate(SyncCommand::Resync {
            from: "Pq7".into(),
            logs: Vec::new(),
        })
        .unwrap();
    assert_eq!(reply, Reply::ResyncRequested { from: "Pq7".into() });
    let marker = crate::sync::resync::ResyncMarker::read(&rig.admin.paths)
        .unwrap()
        .expect("the marker is written");
    assert_eq!(marker.from.to_string(), peer);
    assert_eq!(marker.operator, "operator:local-user:501");
    assert!(
        rig.admin.shutdown.is_cancelled(),
        "the daemon is asked to stop"
    );
}

/// The restore-older-than-a-rotation fixture: `orig` rotated I1 to I2, the
/// restored node holds only I1 (and learned I2's genesis).
struct RestoreWorld {
    orig: Node,
    restored: Node,
    i1: LogKey,
    tail: Envelope,
}

fn restore_world() -> RestoreWorld {
    let orig = Node::new(SEED_A, 600);
    orig.put("p");
    let i1 = orig.active_log();
    orig.clock.advance(1_000);
    orig.rotate_to(
        501,
        Some(Predecessor {
            incarnation: i1.incarnation,
            frozen_at: None,
        }),
    );
    let i2 = orig.active_log();
    assert_ne!(i1, i2);
    let restored = Node::new(SEED_A, 600);
    assert_eq!(restored.active_log(), i1, "the backup holds the first log");
    restored.ingest(&orig, &i2, 1);
    let tail = orig.entries(&i1, 2).remove(0);
    RestoreWorld {
        orig,
        restored,
        i1,
        tail,
    }
}

// Acceptance (root section 10, single restored copy): clone evidence about this
// node holds the restore rotation; `sync clone-cleared` alone (no move-aside)
// clears the rows with the operator label, signals the worker, and the
// rotation then runs.
//
// Kills: a clearance that does not record the operator, one that leaves the
// rows open, one that never signals the worker (the rotation stays held), and
// a rotation that ran while the evidence was open.
#[test]
fn sync_clone_cleared_single_restored_store_clears_and_rotation_runs() {
    let w = restore_world();
    let source = NodeUuid::from_bytes([11; 16]);
    let report = CloneReport {
        subject: w.i1.node,
        kind: CloneKind::Handshake,
        boots: Vec::new(),
        source,
    };
    w.restored
        .quantum(|tx, _, d| incarnations::record_clone_evidence(tx, &report, d.utc));
    assert_eq!(
        w.restored
            .count("SELECT COUNT(*) FROM sync_clone_evidence WHERE cleared_at IS NULL"),
        1
    );
    let restored_sink = w.restored.sink.clone();
    let restored_store = w.restored.store.clone();
    let restored_clock = w.restored.clock.clone();
    let rig = Rig::new(w.restored);

    // Queue both inputs before starting the worker. Otherwise its startup
    // reissue pass can emit HELD_LINE before this Restore is handled, letting
    // the test clear the gate too early: Restore then rotates directly and
    // never emits the branch request the test waits for below.
    restored_sink.on_have(
        source,
        true,
        &[HaveLog {
            node: w.i1.node,
            inc: w.i1.incarnation,
            counter: 2,
            head: hex_encode(&w.tail.hash),
            frozen_at: None,
        }],
        &[],
    );
    assert!(restored_sink.enqueue(Job::Restore {
        log: w.i1,
        source,
        entry: Box::new(w.tail.clone().into()),
    }));

    // The worker, on the same store and sink the admin signals.
    let worker_stop = Cancellation::default();
    let worker = {
        let lines = rig.node.lines.clone();
        IncarnationsWorker::start(WorkerDeps {
            sink: restored_sink.clone(),
            journal: restored_store.clone(),
            store: restored_store.clone(),
            writer: Arc::new(FairWriter::new(8)),
            status: Arc::new(WorkerStatus::default()),
            own_history: restored_store
                .context()
                .sync()
                .unwrap()
                .own_history_hold()
                .clone(),
            signals: rig.admin.sync.signals(),
            sync_pacer: None,
            clock: restored_clock,
            log: Arc::new(move |line: &str| lines.lock().unwrap().push(line.to_owned())),
            cancel: worker_stop.clone(),
        })
        .unwrap()
    };
    let stop_worker = |worker: std::thread::JoinHandle<()>| {
        worker_stop.cancel();
        worker.join().unwrap();
    };

    wait_for(10, "the worker to hold the restore", || {
        rig.node
            .lines
            .lock()
            .unwrap()
            .iter()
            .any(|l| l == HELD_LINE)
    });
    assert_eq!(
        rig.node.active_log(),
        w.i1,
        "no restore rotation while the evidence is open"
    );
    let before = rig.status();
    assert_eq!(before.remedies.len(), 2);

    // The operator clears it, alone.
    let Reply::CloneCleared { cleared } = rig.mutate(SyncCommand::CloneCleared).unwrap() else {
        panic!()
    };
    assert_eq!(cleared, 1);
    let (by, at): (String, i64) = {
        let db = rig.node.store.context().open_query(budget()).unwrap();
        db.query_row(
            "SELECT cleared_by,cleared_at FROM sync_clone_evidence",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    };
    assert_eq!(by, "operator:local-user:501");
    assert_eq!(at, T0, "cleared at the decision time");
    let after = rig.status();
    assert!(
        after.evidence.is_empty() && after.remedies.is_empty(),
        "{after:?}"
    );

    // The worker re-derives the held restore: it asks the peer for the entry
    // that extends its head; the test answers as the peer.
    let mut want = None;
    wait_for(10, "the worker to ask for the restore entry", || {
        want = restored_sink.outbox().sends.into_iter().next();
        want.is_some()
    });
    let (to, want) = want.unwrap();
    assert_eq!(to, source);
    assert_eq!(
        (want.node, want.inc, want.from),
        (w.i1.node, w.i1.incarnation, 2)
    );
    restored_sink.on_branch(
        source,
        BranchReply::Entries {
            node: want.node,
            inc: want.inc,
            tip_hash: want.tip.hash.clone(),
            entries: vec![WireItem::from(&w.tail)],
            more: false,
        },
    );
    wait_for(10, "the restore rotation to run", || {
        rig.node.active_log() != w.i1
    });
    stop_worker(worker);
    drop(w.orig);
}

// -- Node status (ht-xg88) --------------------------------------------------

fn peer_node() -> NodeUuid {
    NodeUuid::from_u128(0xabcdef)
}

fn seed_peer_node(node: &Node, tag: &str) {
    let db = node.store.context().open_writer().unwrap();
    crate::store::node_directory::upsert(
        &db,
        &crate::store::node_directory::NodeDirectoryRow {
            node_uuid: peer_node(),
            tag: tag.parse().unwrap(),
            tag_k: 0,
            host_label: Some("laptop".into()),
            first_seen_at: 0,
            tag_hlc: None,
        },
    )
    .unwrap();
}

fn record_peer(node: &Node, row: crate::store::sync_peer_status::PeerStatus) {
    let mut db = node.store.context().open_writer().unwrap();
    let tx = db.transaction().unwrap();
    crate::store::sync_peer_status::record(&tx, &[row], 1).unwrap();
    tx.commit().unwrap();
}

fn peer_status(version: Option<&str>, boot: u128) -> crate::store::sync_peer_status::PeerStatus {
    crate::store::sync_peer_status::PeerStatus {
        node: peer_node(),
        version: version.map(str::to_owned),
        boot: uuid::Uuid::from_u128(boot),
        tx: Some((7, 9)),
        rx: Some((3, 5)),
        app_seen_at: Some(4),
    }
}

fn node_lines(rig: &Rig) -> Vec<String> {
    text(rig.read(SyncCommand::PeerList).unwrap())
        .lines()
        .filter(|l| l.starts_with("node "))
        .map(str::to_owned)
        .collect()
}

// Kills: a known peer node missing from `peer list` while it has no link, a
// lost last-known version or position, a never-heard node shown reachable,
// a relayed-fresh node shown linked (no live link), and open clone evidence
// not marked.
#[test]
fn peer_list_shows_last_known_node_status() {
    let rig = Rig::new(Node::new(SEED_A, 1));
    seed_peer_node(&rig.node, "Zq7");
    record_peer(&rig.node, peer_status(Some("2.3.9"), 1));
    // A row without a version or positions keeps the stored ones.
    record_peer(
        &rig.node,
        crate::store::sync_peer_status::PeerStatus {
            version: None,
            tx: None,
            rx: None,
            ..peer_status(None, 1)
        },
    );
    let lines = node_lines(&rig);
    assert_eq!(lines, ["node Zq7 unreachable never v=2.3.9 rx=3/5 tx=7/9"]);
    assert_eq!(lines[0].len(), 48);

    // A newer boot without a version replaces the version.
    record_peer(&rig.node, peer_status(None, 2));
    assert_eq!(
        node_lines(&rig),
        ["node Zq7 unreachable never v=- rx=3/5 tx=7/9"]
    );

    // Fresh contact but no live link: reachable (relayed), never linked.
    let now = crate::protocol::time::Clock::utc_now(rig.node.clock.as_ref()).0;
    let db = rig.node.store.context().open_writer().unwrap();
    db.execute(
        "INSERT INTO sync_node_contact(node_uuid,last_contact_at,via,updated_at) VALUES (?1,?2,'relayed',?2)",
        rusqlite::params![peer_node().to_string(), now - 4_000],
    )
    .unwrap();
    db.execute(
        "INSERT INTO sync_clone_evidence(subject,kind,detail,source_node,detected_at,last_seen_at) \
         VALUES (?1,'third_party','two boots',NULL,0,0)",
        [peer_node().to_string()],
    )
    .unwrap();
    let lines = crate::protocol::output::with_render_now(UtcMillis(now), || node_lines(&rig));
    // The stored app contact is at unix ms 4: days ago.
    assert_eq!(
        lines,
        [format!(
            "node Zq7 reachable seen=4s app={} v=- rx=3/5 tx=7/9 clone_suspected",
            age_of(now, 4)
        )]
    );
}

fn age_of(now: i64, at: i64) -> String {
    let secs = (now - at) / 1000;
    match secs {
        0..=119 => format!("{secs}s"),
        120..=7_199 => format!("{}m", secs / 60),
        7_200..=172_799 => format!("{}h", secs / 3_600),
        _ => format!("{}d", secs / 86_400),
    }
}

// Kills: node rows rendered with the wrong state words, the stall parts
// missing, a linked row without its ages or link start, and an older
// daemon's reply (no nodes) printing node lines.
#[test]
fn node_rows_render_compactly() {
    use crate::protocol::sync_admin::NodeRow;
    let now = 1_793_092_323_481; // 2026-10-27T09:12:03.481Z
    let row = |state: &str, stalled: &[&str]| NodeRow {
        node: "Q0V".into(),
        host: None,
        state: state.into(),
        stalled: stalled.iter().map(|s| (*s).to_owned()).collect(),
        last_contact_at: Some(now - 4_000),
        app_seen_at: Some(now - 6_000),
        linked_since: Some(now - 3 * 3_600_000),
        version: Some("2.4.0".into()),
        tx: Some((1180, 1234)),
        rx: Some((567, 567)),
        clone_suspected: false,
    };
    let render = |nodes: Vec<NodeRow>| {
        crate::protocol::output::with_render_now(UtcMillis(now), || {
            text(Reply::Peers {
                rows: vec![],
                discovery: None,
                nodes,
            })
        })
    };
    let linked = render(vec![row("linked", &[])]);
    assert_eq!(
        linked,
        "peers\nnode Q0V linked seen=4s app=6s v=2.4.0 rx=567/567 tx=1180/1234 since=06:12Z\n"
    );
    let stalled = render(vec![NodeRow {
        clone_suspected: true,
        ..row("stalled", &["app", "tx"])
    }]);
    let line = stalled.lines().nth(1).unwrap();
    assert_eq!(
        line,
        "node Q0V stalled app,tx seen=4s app=6s v=2.4.0 rx=567/567 tx=1180/1234 since=06:12Z clone_suspected"
    );
    assert_eq!(line.len(), 99);
    let unreachable = render(vec![NodeRow {
        linked_since: None,
        ..row("unreachable", &[])
    }]);
    assert_eq!(
        unreachable.lines().nth(1).unwrap(),
        "node Q0V unreachable since 09:11Z v=2.4.0 rx=567/567 tx=1180/1234"
    );
    // The machine name an `@MACHINE` address matches, escaped to one token.
    let named = render(vec![NodeRow {
        host: Some("laptop.local".into()),
        ..row("linked", &[])
    }]);
    assert!(
        named
            .lines()
            .nth(1)
            .unwrap()
            .ends_with(" since=06:12Z host=laptop.local"),
        "{named}"
    );
    // `empty` only when there is neither a target nor a node; an older
    // daemon's reply has no `nodes`: no node lines.
    let old: Reply =
        serde_json::from_value(serde_json::json!({"op": "peers", "rows": []})).unwrap();
    assert_eq!(text(old), "peers\nempty\n");
}

// Kills: the link liveness fields missing from a current daemon's link row,
// or printed (as dashes) for an older daemon's reply that has none.
#[test]
fn status_link_row_shows_liveness() {
    use crate::protocol::sync_admin::LinkRow;
    let now = 1_793_092_323_481;
    let rig = Rig::new(Node::new(SEED_A, 1));
    let mut status = rig.status();
    status.links = vec![LinkRow {
        node: "Q0V".into(),
        mode: "active".into(),
        role: "dialer".into(),
        rx_logs: 3,
        unsolicited: 0,
        withheld_logs: 0,
        dest: None,
        last_error: None,
        since: Some(now - 60_000),
        last_rx: Some(now - 2_000),
        app_rx: Some(now - 6_000),
        subscriptions: 3,
        peer_lag: Some(54),
        stalled: vec!["tx".into()],
    }];
    let rendered = crate::protocol::output::with_render_now(UtcMillis(now), || {
        text(Reply::Status(Box::new(status)))
    });
    let line = rendered.lines().find(|l| l.starts_with("link ")).unwrap();
    assert_eq!(
        line,
        "link Q0V active dialer rx_logs=3 unsolicited=0 since=09:11Z last_rx=2s app_rx=6s subs=3 peer_lag=54 stalled=tx"
    );
    assert_eq!(line.len(), 110);
}

// Kills: a new boot that keeps the old boot's positions or app contact, an
// app contact that moves backwards or turns NULL into 0, and a counter past
// i64::MAX that fails the write instead of being clamped.
#[test]
fn peer_status_merge_rule() {
    use crate::store::sync_peer_status::{PeerStatus, list};
    let rig = Rig::new(Node::new(SEED_A, 1));
    let stored = |node: &Node| list(&node.store.context().open_writer().unwrap()).unwrap();
    let base = PeerStatus {
        app_seen_at: None,
        ..peer_status(Some("2.4.0"), 1)
    };
    record_peer(&rig.node, base.clone());
    assert_eq!(
        stored(&rig.node),
        std::slice::from_ref(&base),
        "NULL app contact stays NULL"
    );
    // Same boot: missing values keep the stored ones, app contact only grows.
    record_peer(
        &rig.node,
        PeerStatus {
            version: None,
            tx: None,
            rx: Some((4, 5)),
            app_seen_at: Some(50),
            ..base.clone()
        },
    );
    record_peer(
        &rig.node,
        PeerStatus {
            app_seen_at: Some(20),
            ..base.clone()
        },
    );
    let row = &stored(&rig.node)[0];
    assert_eq!(row.version.as_deref(), Some("2.4.0"));
    assert_eq!(
        (row.tx, row.rx, row.app_seen_at),
        (Some((7, 9)), Some((3, 5)), Some(50))
    );
    // A new boot replaces everything, absent values included.
    record_peer(
        &rig.node,
        PeerStatus {
            version: None,
            tx: None,
            rx: None,
            app_seen_at: None,
            ..peer_status(None, 2)
        },
    );
    let row = &stored(&rig.node)[0];
    assert_eq!(
        (row.version.clone(), row.tx, row.rx, row.app_seen_at),
        (None, None, None, None)
    );
    // Past i64::MAX: stored clamped, the write succeeds.
    record_peer(
        &rig.node,
        PeerStatus {
            rx: Some((1, u64::MAX)),
            ..peer_status(None, 2)
        },
    );
    assert_eq!(stored(&rig.node)[0].rx, Some((1, i64::MAX as u64)));
}

// Kills: a stored unreachable contact overriding a live active link (a
// just-linked node shown `unreachable since <now>`), and a stall not shown.
#[test]
fn node_state_prefers_the_live_link() {
    use crate::store::sync_contacts::NodeReach;
    use crate::store::sync_status::{LiveNode, NodeState, node_state};
    use crate::sync::coordinator::Stall;
    let live = |stalled: Stall| LiveNode {
        node: peer_node(),
        linked_since: 0,
        last_rx: 0,
        app_seen_at: None,
        version: None,
        tx: None,
        rx: None,
        stalled,
    };
    let never = NodeReach::Unreachable { since: None };
    assert_eq!(
        node_state(never, Some(&live(Stall::default()))),
        NodeState::Linked
    );
    let tx = Stall {
        tx: true,
        ..Stall::default()
    };
    assert_eq!(node_state(never, Some(&live(tx))), NodeState::Stalled);
    assert_eq!(node_state(never, None), NodeState::Unreachable);
    assert_eq!(node_state(NodeReach::Reachable, None), NodeState::Reachable);
}

// -- Seat status store (ht-xg88 phase 2) -------------------------------------

fn seed_remote_seat(node: &Node, seat: &str, home: NodeUuid) {
    let db = node.store.context().open_writer().unwrap();
    db.execute(
        "INSERT INTO seats(id,instance_id,state,role,generation,created_at,home_node,is_local,remote_kind,host_id,published) \
         VALUES (?1,(SELECT id FROM local_node),'remote','native',0,0,?2,0,'agent',NULL,1)",
        rusqlite::params![seat, home.to_string()],
    )
    .unwrap();
}

fn remote_seat(seat: &str, presence: &str) -> crate::store::remote_seat_status::RemoteSeat {
    crate::store::remote_seat_status::RemoteSeat {
        seat: seat.into(),
        presence: presence.into(),
        harness: Some("claude".into()),
        activity_at: Some(900),
        workspace: Some("office".into()),
        tab: Some("main".into()),
        pane: Some("claude".into()),
    }
}

fn record_seats(node: &Node, update: crate::store::remote_seat_status::RemoteSeatUpdate) {
    let mut db = node.store.context().open_writer().unwrap();
    let tx = db.transaction().unwrap();
    crate::store::remote_seat_status::record(&tx, &[update], 1).unwrap();
    tx.commit().unwrap();
}

// Kills: an entry kept for a seat whose home node is not the snapshot's
// origin (or that this node does not know), a newer snapshot that keeps a
// seat it no longer lists, and a refresh that wipes the seats.
#[test]
fn remote_seat_snapshots_replace_by_origin_with_the_home_guard() {
    use crate::store::remote_seat_status::{RemoteSeatUpdate, get};
    let rig = Rig::new(Node::new(SEED_A, 1));
    let other = NodeUuid::from_u128(0x5eed);
    seed_remote_seat(&rig.node, "sQ0VnTDasAbY", peer_node());
    seed_remote_seat(&rig.node, "sQ0Vx1aB2cD3", peer_node());
    seed_remote_seat(&rig.node, "sK7m0aaaBBBB", other);
    let update = |rev, seats: Vec<_>| RemoteSeatUpdate {
        origin: peer_node(),
        boot: uuid::Uuid::from_u128(1),
        rev,
        built_at: 1_000 * rev,
        version: Some("2.4.0".into()),
        other_boot_at: None,
        seats,
    };
    record_seats(
        &rig.node,
        update(
            1,
            vec![
                remote_seat("sQ0VnTDasAbY", "idle"),
                remote_seat("sQ0Vx1aB2cD3", "busy"),
                remote_seat("sK7m0aaaBBBB", "busy"), // another node's seat
                remote_seat("sUnknown0000", "busy"), // unknown here
            ],
        ),
    );
    let db = rig.node.store.context().open_writer().unwrap();
    let count = |db: &rusqlite::Connection| -> i64 {
        db.query_row("SELECT count(*) FROM remote_seat_status", [], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(count(&db), 2);
    assert_eq!(get(&db, "sK7m0aaaBBBB").unwrap(), None);
    record_seats(
        &rig.node,
        update(2, vec![remote_seat("sQ0VnTDasAbY", "busy")]),
    );
    assert_eq!(count(&db), 1, "a seat the newer snapshot omits is gone");
    // A refresh (same content, next rev) keeps the rows and moves the build.
    record_seats(
        &rig.node,
        update(3, vec![remote_seat("sQ0VnTDasAbY", "busy")]),
    );
    let row = get(&db, "sQ0VnTDasAbY")
        .unwrap()
        .expect("kept by a refresh");
    assert_eq!((row.seat.presence.as_str(), row.built_at), ("busy", 3_000));
    // A seat the store learns of after the snapshot that named it is taken
    // by the next refresh.
    seed_remote_seat(&rig.node, "sQ0VlateSeat", peer_node());
    record_seats(
        &rig.node,
        update(
            4,
            vec![
                remote_seat("sQ0VnTDasAbY", "busy"),
                remote_seat("sQ0VlateSeat", "idle"),
            ],
        ),
    );
    assert_eq!(count(&db), 2);
}

// Kills: presence shown for a seat whose home node is unreachable, a
// snapshot past the reach window shown fresh, a clone suspicion not shown,
// and a live link overridden by the stored reach.
#[test]
fn remote_presence_is_only_as_fresh_as_its_node() {
    use crate::store::remote_seat_status::{RemotePresence, RemoteSeatRow, presence_of};
    use crate::store::sync_contacts::NodeReach;
    let now = 1_000_000;
    let row = |built_at, other_boot_at| RemoteSeatRow {
        origin: peer_node(),
        seat: remote_seat("sQ0VnTDasAbY", "idle"),
        built_at,
        other_boot_at,
    };
    let fresh = row(now - 30_000, None);
    let gone = NodeReach::Unreachable { since: Some(5) };
    assert_eq!(
        presence_of(Some(&fresh), gone, false, now),
        RemotePresence::Unreachable { since: Some(5) }
    );
    assert!(matches!(
        presence_of(Some(&fresh), gone, true, now),
        RemotePresence::Fresh { .. }
    ));
    assert_eq!(
        presence_of(Some(&fresh), NodeReach::Reachable, false, now),
        RemotePresence::Fresh {
            presence: "idle".into(),
            activity_at: Some(900)
        }
    );
    assert_eq!(
        presence_of(
            Some(&row(now - 181_000, None)),
            NodeReach::Reachable,
            false,
            now
        ),
        RemotePresence::Stale {
            built_at: now - 181_000
        }
    );
    assert_eq!(
        presence_of(
            Some(&row(now - 1_000, Some(now - 10_000))),
            NodeReach::Reachable,
            true,
            now
        ),
        RemotePresence::CloneSuspected
    );
    assert_eq!(
        presence_of(None, NodeReach::Reachable, true, now),
        RemotePresence::Unknown
    );
}

// -- Seat directory (ht-xg88 phase 3) ----------------------------------------

fn render(result: crate::protocol::results::CommandResult) -> String {
    let bytes = crate::protocol::output::encode_selected(
        &result,
        &crate::protocol::output::OutputSpec {
            format: crate::protocol::output::OutputFormat::Text,
            context: Default::default(),
        },
    )
    .unwrap();
    String::from_utf8(bytes).unwrap()
}

fn seats_of(
    rig: &Rig,
    filter: crate::protocol::commands::SeatFilter,
) -> Vec<crate::protocol::results::SeatSummary> {
    use crate::ports::{ReadContext, StorePort};
    let result = rig
        .node
        .store
        .query(
            &crate::protocol::commands::Command::Seats(crate::protocol::commands::SeatsQuery {
                page: crate::protocol::pagination::PageRequest::default(),
                target: None,
                include_retired: false,
                filter,
            }),
            &ReadContext {
                instance: rig
                    .node
                    .store
                    .context()
                    .open_writer()
                    .unwrap()
                    .query_row("SELECT id FROM local_node", [], |r| r.get(0))
                    .unwrap(),
                output: Default::default(),
                operation_scope: None,
            },
            &budget(),
        )
        .unwrap();
    let crate::protocol::results::CommandResult::Seats(page) = result else {
        panic!("{result:?}")
    };
    page.items
}

// Kills: a remote seat listed without its home node's presence and labels,
// one on an unreachable node shown with a presence, filters that do not
// narrow (node, machine, harness, label, local) or that drop matching rows.
#[test]
fn seat_list_carries_the_directory_and_filters_it() {
    use crate::protocol::commands::SeatFilter;
    let rig = Rig::new(Node::new(SEED_A, 1));
    seed_peer_node(&rig.node, "Zq7");
    seed_remote_seat(&rig.node, "sQ0VnTDasAbY", peer_node());
    seed_remote_seat(&rig.node, "sQ0Vx1aB2cD3", peer_node());
    let now = crate::protocol::time::Clock::utc_now(rig.node.clock.as_ref()).0 as u64;
    let mut second = remote_seat("sQ0Vx1aB2cD3", "busy");
    second.harness = Some("codex".into());
    second.workspace = Some("lab".into());
    record_seats(
        &rig.node,
        crate::store::remote_seat_status::RemoteSeatUpdate {
            origin: peer_node(),
            boot: uuid::Uuid::from_u128(1),
            rev: 1,
            built_at: now - 1_000,
            version: None,
            other_boot_at: None,
            seats: vec![remote_seat("sQ0VnTDasAbY", "idle"), second],
        },
    );
    // No contact yet: the home node is unreachable, never a presence.
    let all = seats_of(&rig, SeatFilter::default());
    let remote: Vec<_> = all
        .iter()
        .filter_map(|s| s.directory.as_deref())
        .filter(|d| d.node == "Zq7")
        .collect();
    assert_eq!(remote.len(), 2);
    assert!(
        remote.iter().all(|d| d.state == "unreachable"),
        "{remote:?}"
    );
    assert_eq!(
        remote[0].pane.as_deref(),
        Some("claude"),
        "last-known labels"
    );
    // Fresh contact: the published presence.
    let db = rig.node.store.context().open_writer().unwrap();
    db.execute(
        "INSERT INTO sync_node_contact(node_uuid,last_contact_at,via,updated_at) VALUES (?1,?2,'direct',?2)",
        rusqlite::params![peer_node().to_string(), now as i64],
    )
    .unwrap();
    let states = |filter: SeatFilter| -> Vec<(String, String)> {
        seats_of(&rig, filter)
            .into_iter()
            .map(|s| {
                (
                    s.seat.as_str().to_owned(),
                    s.directory.map_or_else(String::new, |d| d.state),
                )
            })
            .collect()
    };
    let mut fresh = states(SeatFilter {
        node: Some("zq7".into()),
        ..SeatFilter::default()
    });
    fresh.sort();
    assert_eq!(
        fresh,
        [
            ("sQ0VnTDasAbY".to_owned(), "idle".to_owned()),
            ("sQ0Vx1aB2cD3".to_owned(), "busy".to_owned())
        ]
    );
    let only = |filter| {
        states(filter)
            .into_iter()
            .map(|(s, _)| s)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        only(SeatFilter {
            harness: Some("CODEX".into()),
            ..SeatFilter::default()
        }),
        ["sQ0Vx1aB2cD3"]
    );
    assert_eq!(
        only(SeatFilter {
            label: Some("LAB".into()),
            ..SeatFilter::default()
        }),
        ["sQ0Vx1aB2cD3"]
    );
    assert_eq!(
        only(SeatFilter {
            machine: Some("laptop".into()),
            harness: Some("claude".into()),
            ..SeatFilter::default()
        }),
        ["sQ0VnTDasAbY"]
    );
    assert!(
        only(SeatFilter {
            local: true,
            ..SeatFilter::default()
        })
        .iter()
        .all(|s| !s.starts_with("sQ0V"))
    );
    // A filtered page continues with its filter: the `next:` command keeps
    // it (as `--flag=VALUE`) and its cursor resumes after the first row.
    use crate::ports::{ReadContext, StorePort};
    let query = |cursor: Option<String>| {
        crate::protocol::commands::Command::Seats(crate::protocol::commands::SeatsQuery {
            page: crate::protocol::pagination::PageRequest {
                cursor,
                limit: 1,
                ..Default::default()
            },
            target: None,
            include_retired: false,
            filter: SeatFilter {
                node: Some("zq7".into()),
                ..SeatFilter::default()
            },
        })
    };
    let instance: String = rig
        .node
        .store
        .context()
        .open_writer()
        .unwrap()
        .query_row("SELECT id FROM local_node", [], |r| r.get(0))
        .unwrap();
    let read = |command| {
        let result = rig
            .node
            .store
            .query(
                &command,
                &ReadContext {
                    instance: instance.clone(),
                    output: Default::default(),
                    operation_scope: None,
                },
                &budget(),
            )
            .unwrap();
        let crate::protocol::results::CommandResult::Seats(page) = result else {
            panic!("{result:?}")
        };
        page
    };
    let first = read(query(None));
    assert_eq!(first.items.len(), 1);
    let argv = first.next_argv.clone().expect("a next page");
    assert!(argv.iter().any(|a| a == "--node=zq7"), "{argv:?}");
    let second = read(query(first.next_cursor.clone()));
    assert_eq!(second.items.len(), 1);
    assert_ne!(second.items[0].seat, first.items[0].seat);
}

// Kills: rows, suffixes and lines of the directory printed with the wrong
// words or order, peer labels outside the `: ` tail, and byte sizes over
// what the spec states.
#[test]
fn directory_rows_render_compactly() {
    use crate::protocol::results::{CommandResult, SeatDirectory, SeatSummary};
    let now = 1_793_092_323_481; // 2026-10-27T09:12:03.481Z
    let directory = |state: &str, since: Option<i64>| SeatDirectory {
        node: "Q0V".into(),
        machine: Some("laptop".into()),
        state: state.into(),
        since: since.map(UtcMillis),
        harness: Some("claude".into()),
        activity_at: Some(UtcMillis(now - 12 * 60_000)),
        seen_at: Some(UtcMillis(now - 4_000)),
        workspace: Some("office".into()),
        tab: Some("main".into()),
        pane: Some("claude".into()),
    };
    let summary = |seat: &str, d: SeatDirectory| SeatSummary {
        seat: crate::protocol::ids::SeatId::new(seat),
        continuity: crate::protocol::results::ContinuityStatus::Remote,
        target: None,
        generation: 0,
        created_at: UtcMillis(0),
        retired_at: None,
        active: false,
        bindings: vec![],
        availability: None,
        directory: Some(Box::new(d)),
    };
    let page = crate::protocol::pagination::Page {
        items: vec![
            summary("sQ0VnTDasAbY", directory("idle", None)),
            summary(
                "sQ0Vx1aB2cD3",
                directory("unreachable", Some(now - 30 * 60_000)),
            ),
            summary(
                "sQ0Vtab0nTab",
                SeatDirectory {
                    tab: Some("a: b\nc".into()),
                    ..directory("stale", Some(now - 5 * 60_000))
                },
            ),
        ],
        next_cursor: None,
        next_argv: None,
        high_water_ordinal: 0,
        scope_revision: None,
        has_more: false,
        stop_reason: crate::protocol::pagination::StopReason::Complete,
        consistency: crate::protocol::pagination::Consistency::BoundedLive,
    };
    let text = crate::protocol::output::with_render_now(UtcMillis(now), || {
        render(CommandResult::Seats(page))
    });
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "seats");
    assert_eq!(
        lines[1],
        "sQ0VnTDasAbY Q0V idle claude last=12m: office / main / claude"
    );
    assert_eq!(lines[1].len(), 61);
    assert_eq!(
        lines[2],
        "sQ0Vx1aB2cD3 Q0V unreachable since 08:42Z claude last=12m: office / main / claude"
    );
    assert_eq!(lines[2].len(), 81);
    assert_eq!(
        lines[3],
        "sQ0Vtab0nTab Q0V stale 09:07Z claude last=12m: office / a: b\\nc / claude"
    );
    let line = crate::protocol::output::with_render_now(UtcMillis(now), || {
        crate::protocol::output::inspect_directory_line_for_test(&directory("idle", None))
    });
    assert_eq!(
        line,
        "directory: Q0V idle claude last=12m seen=4s: office / main / claude\n"
    );
    assert_eq!(line.trim_end().len(), 67);
}

// Kills: a directory filter that does not narrow (each criterion, alone, on
// an entry that fails only it), `--on` that misses the node tag, and
// `--local` that keeps a remote seat.
#[test]
fn seat_filter_matches_each_criterion() {
    use crate::protocol::commands::SeatFilter;
    use crate::protocol::results::SeatDirectory;
    use crate::store::seat_directory::matches;
    let entry = SeatDirectory {
        node: "Q0V".into(),
        machine: Some("laptop".into()),
        state: "idle".into(),
        since: None,
        harness: Some("claude".into()),
        activity_at: None,
        seen_at: None,
        workspace: Some("Office".into()),
        tab: None,
        pane: None,
    };
    let f = |filter: SeatFilter| matches(&filter, Some(&entry), false);
    assert!(f(SeatFilter::default()));
    assert!(f(SeatFilter {
        node: Some("q0v".into()),
        ..Default::default()
    }));
    assert!(!f(SeatFilter {
        node: Some("blk".into()),
        ..Default::default()
    }));
    assert!(f(SeatFilter {
        machine: Some("LAPTOP".into()),
        ..Default::default()
    }));
    assert!(
        f(SeatFilter {
            machine: Some("Q0V".into()),
            ..Default::default()
        }),
        "the tag too"
    );
    assert!(!f(SeatFilter {
        machine: Some("desk".into()),
        ..Default::default()
    }));
    assert!(!f(SeatFilter {
        harness: Some("codex".into()),
        ..Default::default()
    }));
    assert!(f(SeatFilter {
        label: Some("offi".into()),
        ..Default::default()
    }));
    assert!(!f(SeatFilter {
        label: Some("lab".into()),
        ..Default::default()
    }));
    assert!(!f(SeatFilter {
        local: true,
        ..Default::default()
    }));
    assert!(matches(
        &SeatFilter {
            local: true,
            ..Default::default()
        },
        Some(&entry),
        true
    ));
}

// Kills: a local row without its state word or continuity marker, a
// participant suffix shown for an unreachable or unknown node (or a word
// only, without harness and age, for a current one), and a remote location
// hint not escaped part by part or without its "last known" mark.
#[test]
fn local_rows_participant_suffix_and_location_hint() {
    use crate::protocol::results::{
        CommandResult, ContinuityStatus, MembershipStatus, Participant, SeatDirectory, SeatSummary,
    };
    let now = 1_793_092_323_481;
    let entry = |state: &str| SeatDirectory {
        node: "blk".into(),
        machine: None,
        state: state.into(),
        since: None,
        harness: Some("claude".into()),
        activity_at: Some(UtcMillis(now - 4_000)),
        seen_at: None,
        workspace: None,
        tab: None,
        pane: None,
    };
    let summary = |continuity, d: SeatDirectory| SeatSummary {
        seat: crate::protocol::ids::SeatId::new("snSGbl096JZZ"),
        continuity,
        target: None,
        generation: 0,
        created_at: UtcMillis(0),
        retired_at: None,
        active: true,
        bindings: vec![],
        availability: None,
        directory: Some(Box::new(d)),
    };
    fn page<T>(items: Vec<T>) -> crate::protocol::pagination::Page<T> {
        crate::protocol::pagination::Page {
            items,
            next_cursor: None,
            next_argv: None,
            high_water_ordinal: 0,
            scope_revision: None,
            has_more: false,
            stop_reason: crate::protocol::pagination::StopReason::Complete,
            consistency: crate::protocol::pagination::Consistency::BoundedLive,
        }
    }
    let text = crate::protocol::output::with_render_now(UtcMillis(now), || {
        render(CommandResult::Seats(page(vec![
            summary(ContinuityStatus::Resolved, entry("busy")),
            summary(ContinuityStatus::Unresolved, entry("offline")),
        ])))
    });
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[1], "snSGbl096JZZ blk busy claude last=4s");
    assert_eq!(lines[1].len(), 36);
    assert_eq!(
        lines[2],
        "snSGbl096JZZ blk offline claude last=4s unresolved"
    );

    let participant = |d: Option<SeatDirectory>| Participant {
        seat: crate::protocol::ids::SeatId::new("sQ0VnTDasAbY"),
        is_self: false,
        requirement: None,
        episode: 1,
        joined: true,
        retired: false,
        physical_state: MembershipStatus::Joined,
        effective_state: MembershipStatus::Joined,
        joined_at: None,
        left_at: None,
        retirement_cutover: None,
        cleanup_state: None,
        accepted_invitation: None,
        active: false,
        bindings: vec![],
        availability: Some(crate::protocol::results::PresenceAvailability::Remote),
        home_unreachable: None,
        directory: d.map(Box::new),
    };
    let rows = crate::protocol::output::with_render_now(UtcMillis(now), || {
        render(CommandResult::Participants(page(vec![
            participant(Some(entry("idle"))),
            participant(Some(entry("unreachable"))),
            participant(Some(entry("stale"))),
            participant(Some(entry("-"))),
            participant(None),
        ])))
    });
    let rows: Vec<&str> = rows.lines().skip(1).collect();
    assert_eq!(rows[0], "sQ0VnTDasAbY joined remote idle claude last=4s");
    assert_eq!(rows[0].len() - "sQ0VnTDasAbY joined remote".len(), 20);
    assert_eq!(rows[1], "sQ0VnTDasAbY joined remote");
    assert_eq!(rows[2], "sQ0VnTDasAbY joined remote stale");
    assert_eq!(rows[3], "sQ0VnTDasAbY joined remote");
    assert_eq!(rows[4], "sQ0VnTDasAbY joined remote");

    let labeled = SeatDirectory {
        workspace: Some("a / b".into()),
        pane: Some("p\nq".into()),
        ..entry("idle")
    };
    assert_eq!(
        labeled
            .labels(|part| crate::cli::human::one_line(part, false, 64))
            .unwrap(),
        "a / b / - / p\\nq"
    );
}
