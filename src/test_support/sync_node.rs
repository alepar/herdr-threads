//! In-process sync nodes for multi-node convergence scenarios.
//!
//! Responsibility
//! - Owns: [`SyncNode`], one node's sync runtime assembled around an
//!   already-open `SqliteStore`: the real `SyncCoordinator`, the real
//!   `IncarnationsSink` as its `ForkSink`, the real `IncarnationsWorker`
//!   thread and an apply pump; and [`SyncNode::link`], which joins two nodes'
//!   coordinators over a `tokio::io::duplex`.
//! - Hides: the wiring `app.rs` does (own-history hold, `FairWriter`, pacer,
//!   worker deps), the first-frame hand-off the transport does for an inbound
//!   link, and the pump that stands in for the Apply lane and the table
//!   kicks.
//! - Must not: spawn a process, dial anything real (no ssh; the only
//!   transport is a duplex), implement a second frame shuttle, or know any
//!   kind's semantics. Scenario setup (stores, authoring, snapshots) is the
//!   caller's: a store is opened elsewhere and handed in.
//! - Role: test harness for `tests/sync/incarnations_scenarios.rs`. A node
//!   is stopped with [`SyncNode::stop`] and restarted by calling
//!   [`SyncNode::start`] on the same store (the sink's and worker's memory
//!   starts empty, as after a daemon restart).
//!
//! The runtime clock is wall time (monotonic ms since the node started),
//! kept apart from the store's own clock, which stays whatever the caller
//! gave the `StoreContext`.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use tokio::io::duplex;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::daemon::transport::{self, MAX_FRAME_BYTES};
use crate::node::{NodeIdentity, NodeUuid};
use crate::protocol::time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis};
use crate::service::fair_writer::FairWriter;
use crate::service::pacer::Pacer;
use crate::service::workers::WorkerStatus;
use crate::store::SqliteStore;
use crate::store::connection::OwnHistoryHold;
use crate::store::sync_dial_targets::TargetId;
use crate::sync::coordinator::{
    CoordinatorDeps, CoordinatorOptions, DIAL_REPORT_SLOTS, DialReport, DialedStream, LogFn,
    PeerIntake, PeerStream, SyncCoordinator, SyncHandle,
};
use crate::sync::dialer::DialConfig;
use crate::sync::incarnations::{IncarnationsSink, IncarnationsWorker, WorkerDeps};
use crate::sync::journal_port::JournalPort;
use crate::sync::sync_store::MemorySyncStore;
use crate::sync::{LogKey, Vector};

/// Duplex buffer of one link.
const LINK_BUFFER: usize = 4 << 20;
/// How often the pump applies pending entries and kicks the sync lane.
const PUMP_EVERY: Duration = Duration::from_millis(20);

/// Wall-time monotonic clock, UTC pinned to a fixed epoch plus elapsed time.
struct RunClock(Instant);

impl Clock for RunClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(
            1_700_000_000_000 + i64::try_from(self.0.elapsed().as_millis()).unwrap_or(i64::MAX),
        )
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(u64::try_from(self.0.elapsed().as_millis()).unwrap_or(u64::MAX))
    }
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Cancellation::default(),
    }
}

/// How a [`SyncNode`] differs from the default.
#[derive(Clone, Copy, Debug)]
pub struct SyncNodeOptions {
    /// Advertise `envelope.opaque.v1` in the hello (default). Without it the
    /// node stands in for a release that cannot take opaque entries: peers
    /// withhold them from it.
    pub advertise_opaque: bool,
}

impl Default for SyncNodeOptions {
    fn default() -> Self {
        Self {
            advertise_opaque: true,
        }
    }
}

/// A dialed link's report stream and retained terminal result.
struct DialedReports {
    receiver: mpsc::Receiver<DialReport>,
    ended: Option<String>,
}

/// One in-process node's sync runtime. See the module documentation.
pub struct SyncNode {
    pub store: Arc<SqliteStore>,
    pub uuid: NodeUuid,
    pub sink: Arc<IncarnationsSink>,
    /// The sync lane's health (the coordinator and the worker report to it).
    pub status: Arc<WorkerStatus>,
    pub handle: SyncHandle,
    lines: Arc<Mutex<Vec<String>>>,
    intake: PeerIntake,
    cancel: Cancellation,
    worker_cancel: Cancellation,
    coordinator: JoinHandle<()>,
    worker: std::thread::JoinHandle<()>,
    pump: JoinHandle<()>,
    /// Reports of the links this node dialed, oldest first.
    reports: Mutex<Vec<DialedReports>>,
    next_target: Mutex<i64>,
}

impl SyncNode {
    /// Starts the runtime of the node whose key is `seed` over `store` (which
    /// must have been opened by `journal::open` with that key). `boot` tells
    /// two daemons of one key apart.
    ///
    /// Must be called inside a multi-thread tokio runtime.
    pub fn start(store: Arc<SqliteStore>, seed: [u8; 32], boot: u128) -> Self {
        let journal: Arc<dyn JournalPort> = store.clone();
        Self::start_serving(store, journal, seed, boot)
    }

    /// [`start`](Self::start) with `journal` as what the coordinator reads and
    /// serves from (a decorator over `store`, to misbehave toward peers); the
    /// worker and the sink keep reading `store`.
    pub fn start_serving(
        store: Arc<SqliteStore>,
        journal: Arc<dyn JournalPort>,
        seed: [u8; 32],
        boot: u128,
    ) -> Self {
        Self::start_with(store, journal, seed, boot, SyncNodeOptions::default())
    }

    /// [`start_serving`](Self::start_serving) with `options`.
    pub fn start_with(
        store: Arc<SqliteStore>,
        journal: Arc<dyn JournalPort>,
        seed: [u8; 32],
        boot: u128,
        options: SyncNodeOptions,
    ) -> Self {
        let identity = Arc::new(NodeIdentity::from_seed(seed));
        let uuid = identity.uuid();
        let clock: Arc<dyn Clock> = Arc::new(RunClock(Instant::now()));
        let cancel = Cancellation::default();
        let worker_cancel = Cancellation::default();
        let lines: Arc<Mutex<Vec<String>>> = Arc::default();
        let log: LogFn = {
            let lines = lines.clone();
            Arc::new(move |line| {
                lines
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(line.to_owned());
            })
        };
        let status = Arc::new(WorkerStatus::default());
        let writer = Arc::new(FairWriter::new(64));
        let pacer = Arc::new(Pacer::new("sync", clock.clone(), cancel.clone()));
        let own_history = store
            .context()
            .sync()
            .map(|state| state.own_history_hold().clone())
            .unwrap_or_else(|_| Arc::new(OwnHistoryHold::new(Arc::new(AtomicBool::new(false)))));
        let sink = Arc::new(IncarnationsSink::new(
            store.clone(),
            clock.clone(),
            log.clone(),
        ));
        let (handle, intake, coordinator) = SyncCoordinator::start_with(
            CoordinatorDeps {
                journal,
                sync_store: Arc::new(MemorySyncStore::new()),
                identity,
                boot: Uuid::from_u128(boot),
                writer: writer.clone(),
                clock: clock.clone(),
                pacer: pacer.clone(),
                status: status.clone(),
                log: log.clone(),
                fork_sink: sink.clone(),
                own_history: own_history.clone(),
                dial: DialConfig::default(),
                resync_pin: None,
                cancel: cancel.clone(),
            },
            CoordinatorOptions {
                advertise_opaque: options.advertise_opaque,
            },
        );
        let worker = IncarnationsWorker::start(WorkerDeps {
            sink: sink.clone(),
            journal: store.clone(),
            store: store.clone(),
            writer,
            status: status.clone(),
            own_history,
            signals: handle.signals(),
            clock,
            log,
            sync_pacer: None,
            cancel: worker_cancel.clone(),
        })
        .expect("spawn the incarnations worker");
        let pump = tokio::spawn(pump(store.clone(), pacer, cancel.clone()));
        Self {
            store,
            uuid,
            sink,
            status,
            handle,
            lines,
            intake,
            cancel,
            worker_cancel,
            coordinator,
            worker,
            pump,
            reports: Mutex::new(Vec::new()),
            next_target: Mutex::new(1),
        }
    }

    /// Joins `self` (the dialer) and `other` (the accepting side) with one
    /// duplex link, the way the Dialer and the transport would.
    pub async fn link(&self, other: &SyncNode) {
        let (ours, mut theirs) = duplex(LINK_BUFFER);
        let (stdout, stdin) = tokio::io::split(ours);
        let (report_tx, report) = mpsc::channel(DIAL_REPORT_SLOTS);
        let target = {
            let mut next = self
                .next_target
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let id = *next;
            *next += 1;
            id
        };
        self.handle
            .hand_off_dialed(DialedStream {
                target_id: TargetId(target),
                dest: format!("test:{}", other.uuid),
                stdout: Box::new(stdout),
                stdin: Box::new(stdin),
                report: report_tx,
            })
            .await
            .ok()
            .expect("the dialing coordinator is running");
        self.reports
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(DialedReports {
                receiver: report,
                ended: None,
            });
        let first = transport::read_frame_with_limit(&mut theirs, MAX_FRAME_BYTES)
            .await
            .expect("the dialer's first frame");
        other
            .intake
            .offer(PeerStream {
                stream: Box::new(theirs),
                first_frame: first,
                permit: Box::new(()),
            })
            .ok()
            .expect("the accepting coordinator has room for the link");
    }

    /// The `DialReport::LinkEnded` reasons of the links this node dialed so
    /// far (`Debug` of the `LinkError`), oldest dial first. Repeated calls
    /// retain terminal reports for convergence polls and failure diagnostics.
    pub fn ended_links(&self) -> Vec<String> {
        let mut reports = self.reports.lock().unwrap_or_else(PoisonError::into_inner);
        for report in reports.iter_mut() {
            while let Ok(item) = report.receiver.try_recv() {
                if let DialReport::LinkEnded { why, .. } = item {
                    report.ended = Some(format!("{why:?}"));
                }
            }
        }
        reports
            .iter()
            .filter_map(|report| report.ended.clone())
            .collect()
    }

    /// Every log line the coordinator, sink and worker wrote.
    pub fn lines(&self) -> Vec<String> {
        self.lines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The store's vector (`log -> head`).
    pub fn vector(&self) -> Vector {
        self.store.vector(&budget()).expect("read the vector")
    }

    /// `(counter, head hash)` of `log`, `None` when the log is not held.
    pub fn head_of(&self, log: &LogKey) -> Option<(u64, [u8; 32])> {
        self.vector()
            .0
            .get(log)
            .map(|head| (head.counter, head.head_hash))
    }

    /// Stops the pump, the coordinator (every link gets `close{shutdown}`)
    /// and the worker thread. The store stays open for the caller.
    pub async fn stop(self) {
        self.cancel.cancel();
        self.worker_cancel.cancel();
        self.pump.abort();
        self.coordinator
            .await
            .expect("the coordinator did not panic");
        let worker = self.worker;
        tokio::task::spawn_blocking(move || worker.join())
            .await
            .expect("join task")
            .expect("the worker did not panic");
    }
}

/// Stands in for the Apply lane and the table kicks: applies pending entries
/// and wakes the sync lane every [`PUMP_EVERY`].
async fn pump(store: Arc<SqliteStore>, pacer: Arc<Pacer>, cancel: Cancellation) {
    loop {
        if cancel.is_cancelled() {
            return;
        }
        let store = store.clone();
        let more = tokio::task::spawn_blocking(move || {
            store
                .apply_pending(&budget())
                .map(|progress| progress.has_more)
                .unwrap_or(false)
        })
        .await
        .unwrap_or(false);
        pacer.kick();
        if !more {
            tokio::time::sleep(PUMP_EVERY).await;
        }
    }
}

/// Polls `cond` every 25 ms until it holds (true) or `timeout` passes
/// (false).
pub async fn wait_for(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if cond() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// [`wait_for`], panicking with `what` on timeout.
pub async fn wait_until(what: &str, timeout: Duration, cond: impl FnMut() -> bool) {
    assert!(
        wait_for(timeout, cond).await,
        "timed out waiting for: {what}"
    );
}
