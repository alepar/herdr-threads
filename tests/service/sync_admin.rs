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
use crate::sync::envelope::{Envelope, WireEnvelope, hex_encode};
use crate::sync::facts::{ApplyContext, FactError, FactKind, FactRegistry, Predecessor};
use crate::sync::frame::HaveLog;
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
    }

    fn ingest(&self, from: &Node, log: &LogKey, first: u64) {
        let entries = from.entries(log, first);
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
         payload,signature,ingested_at,apply_state,applied_seq) \
         VALUES (?1,1,zeroblob(32),zeroblob(32),1,'test.kind',1,x'0000','',x'7b7d',zeroblob(64),0,'applied',777777)",
        [log_ix],
    )
    .unwrap();
    db.execute(
        "INSERT INTO journal_forked(log_ix,counter,prev_hash,hash,hlc,kind,kind_version,deps,needs, \
         payload,signature,received_at) \
         VALUES (?1,1,zeroblob(32),x'0101010101010101010101010101010101010101010101010101010101010101',1,'test.kind',1,x'','',x'7b7d',zeroblob(64),0)",
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
         payload,signature,ingested_at,apply_state) \
         VALUES (?1,1,zeroblob(32),x'0909090909090909090909090909090909090909090909090909090909090909',1,'test.kind',1,x'0000','',x'7b7d',zeroblob(64),0,'failed')",
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
             payload,signature,ingested_at,apply_state) \
             VALUES (?1,?2,?3,?4,1,'unknown.kind',1,x'','',x'7b7d',zeroblob(64),0,'parked_unknown')",
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

    // The peer advertises I1's tail, and the restore entry is queued: the gate
    // holds it.
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
    restored_sink.enqueue(Job::Restore {
        log: w.i1,
        source,
        entry: Box::new(w.tail.clone()),
    });
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
            entries: vec![WireEnvelope::from(&w.tail)],
            more: false,
        },
    );
    wait_for(10, "the restore rotation to run", || {
        rig.node.active_log() != w.i1
    });
    stop_worker(worker);
    drop(w.orig);
}
