//! The sync coordinator: one actor that owns every peer link, plus the three
//! seams around it ([`PeerIntake`], [`ForkSink`] and the [`SyncHandle`] facade).
//!
//! Responsibility (SyncCoordinator / SyncHandle)
//! - Owns: one tokio actor holding `links`, `by_node`, `rx` and `local`
//!   (state shapes per the peer-link design, section 8); [`CoordinatorDeps`]
//!   as its one composition point; the two link entry points (an inbound
//!   [`PeerStream`] offer, a [`DialedStream`] hand-off) with their handshake,
//!   the held-genesis key check and the dialed-link [`DialReport`]s; the
//!   shutdown that writes `close{shutdown}` on every link; and the cloneable
//!   [`SyncHandle`] (`link_count`, `snapshot`).
//! - Hides: the actor's channels, its `JoinSet` of handshakes and the link
//!   ids. Callers see only the handle and the two hand-off types.
//! - Must not: touch SQLite (every journal call goes through [`JournalPort`],
//!   every sync-table call through `SyncStorePort`), name a harness, resolve a
//!   fork (that is the [`ForkSink`]'s), or dial (the Dialer leaf owns that).
//! - Where new behaviour goes: frame routing, duplicate-link ranking and clone
//!   detection are ht-5fr.5.12; serving, live tail, ingest and Have deltas
//!   are ht-5fr.5.13. Each fills the marked stub arm of [`Actor::run`] and the
//!   state fields marked below; `signals()` is ht-5fr.5.6's. The Dialer
//!   (`super::dialer`, ht-5fr.5.7) is constructed here: [`SyncHandle::reload_targets`]
//!   and [`SyncHandle::enabled_target_count`] reach it, a pacer kick re-diffs
//!   its targets, and shutdown cancels its tasks.
//! - Role: the single owner of per-link and per-log sync state, so routing
//!   decisions that span links need no locks.
//!
//! Every `JournalPort` and `SyncStorePort` call is synchronous and takes
//! writer turns, so an actor that makes one runs it in `spawn_blocking`; this
//! skeleton makes one such call, the held-genesis lookup at admission.
//!
//! Core and harness-neutral.

use std::any::Any;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::{Instant, MissedTickBehavior, timeout};
use uuid::Uuid;

use super::dialer::{DialConfig, DialShared, Dialer, DialerDeps};
use super::envelope::{GENESIS_KIND, WireEnvelope, hex_decode};
use super::facts::GenesisPayload;
use super::frame::{BranchWant, CloseReason, ForkRef, HaveLog, PeerFrame, Role, write_peer_frame};
use super::journal_port::{ForkEvidence, JournalPort};
use super::link::{
    CLOSE_WRITE_TIMEOUT, Established, HANDSHAKE_TIMEOUT, LinkError, LinkEvent, LinkHandle,
    LocalHello, PeerInfo, handshake,
};
use super::sync_store::SyncStorePort;
use super::{BatchLimits, LogKey, Vector};
use crate::node::{NodeIdentity, NodeTag, NodeUuid};
use crate::protocol::results::{ApiError, ErrorCode};
use crate::protocol::time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis};
use crate::service::fair_writer::FairWriter;
use crate::service::pacer::{Pacer, Wake};
use crate::service::workers::WorkerStatus;
use crate::store::sync_dial_targets::TargetId;

/// Offers the transport may have waiting for the actor.
pub const PEER_INTAKE_SLOTS: usize = 16;
/// Dialed streams the Dialer may have waiting for the actor.
const DIALED_SLOTS: usize = 16;
/// Link events the actor may have waiting.
const LINK_EVENT_SLOTS: usize = 256;
/// Requests (snapshots) the actor may have waiting.
const REQUEST_SLOTS: usize = 16;
/// Most links a snapshot lists.
pub const SNAPSHOT_MAX_LINKS: usize = 64;
/// Capacity of a dialed link's report channel: `Established` and `LinkEnded`.
pub const DIAL_REPORT_SLOTS: usize = 2;
/// How long the actor waits on its pacer before looping.
const PACER_TICK: Duration = Duration::from_secs(1);
/// The housekeeping interval.
const HOUSEKEEPING_TICK: Duration = Duration::from_secs(1);
/// `LogOnlyForkSink` logs at most once per log per this many milliseconds.
const FORK_LOG_EVERY_MS: u64 = 3_600_000;

/// Any byte stream a link can run over (ssh child stdio, an accepted socket,
/// a test duplex).
pub trait PeerIo: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> PeerIo for T {}

/// A boxed [`PeerIo`]: the hand-off types cross a channel, so they are not
/// generic.
pub type BoxedIo = Box<dyn PeerIo>;

/// What the transport's `ConnectionGauge` hands out per peer connection. The
/// coordinator never looks inside; dropping it releases the slot, so it lives
/// as long as the link.
pub type PeerPermit = Box<dyn Any + Send>;

/// The coordinator's log hook: one line per call, no newline.
pub type LogFn = Arc<dyn Fn(&str) + Send + Sync>;

// ---------------------------------------------------------------------------
// ForkSink
// ---------------------------------------------------------------------------

/// What a [`ForkSink`] answers to a reported fork.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForkAck {
    /// The sink took the fork; the coordinator pauses the log until the sink
    /// reports it resumed.
    Paused,
    /// The evidence is not a proven fork; the coordinator treats the batch as
    /// corrupt.
    Corrupt,
}

/// Where the coordinator reports a fork and learns that a paused log may
/// resume.
///
/// Responsibility (ForkSink)
/// - Owns: the contract `on_fork(log, evidence, source) -> ForkAck` and
///   `resumed()` (polled on the 1 s tick), and the fork.v1 hooks, all with
///   no-op defaults: `on_have` (a peer's `have` logs and fork refs),
///   `on_branch(BranchReply)` (a `branch_entries` or `branch_none` frame),
///   `outbox()` (`branch_want` frames to send and whether the advertised
///   refs changed) and `fork_refs()` (the refs to advertise).
/// - Hides: fork and clone policy entirely; the coordinator only pauses and
///   unpauses wants.
/// - Must not: be called for anything but `IngestOutcome::Fork` (and later the
///   fork frames and clone reports); the coordinator never freezes, re-issues
///   or acts on fork evidence itself. Every method is non-blocking.
/// - Role: [`LogOnlyForkSink`] now, ht-5fr.6's incarnations sink next, chosen
///   once in [`CoordinatorDeps`]. New fork handling is a new implementation;
///   further hooks are new methods with no-op defaults.
///
/// Errors: none; implementations log their own failures, and an unresolved
/// fork stays paused.
pub trait ForkSink: Send + Sync {
    /// A validly signed entry contradicts the held chain of `log`, learned
    /// from `source`.
    fn on_fork(&self, log: LogKey, evidence: ForkEvidence, source: NodeUuid) -> ForkAck;

    /// Logs whose fork is now handled and may be wanted again.
    fn resumed(&self) -> Vec<LogKey>;

    /// A `have` from `source`: its logs and, when `fork_cap` (the link
    /// negotiated [`FORK_CAP`](super::frame::FORK_CAP)), its fork refs. Default: ignored.
    fn on_have(&self, _source: NodeUuid, _fork_cap: bool, _logs: &[HaveLog], _forks: &[ForkRef]) {}

    /// A branch reply from `source`. Default: ignored.
    fn on_branch(&self, _source: NodeUuid, _reply: BranchReply) {}

    /// Take what the sink wants sent: `branch_want` frames per peer, and
    /// whether [`Self::fork_refs`] changed since the last call. Default:
    /// nothing.
    fn outbox(&self) -> ForkOutbox {
        ForkOutbox::default()
    }

    /// The fork refs to advertise in `have`, at most [`MAX_HAVE_FORKS`](super::frame::MAX_HAVE_FORKS),
    /// newest first. Default: none.
    fn fork_refs(&self) -> Vec<ForkRef> {
        Vec::new()
    }
}

/// A branch frame handed to [`ForkSink::on_branch`].
#[derive(Debug)]
pub enum BranchReply {
    /// A `branch_entries` batch.
    Entries {
        node: NodeUuid,
        inc: u32,
        tip_hash: String,
        entries: Vec<WireEnvelope>,
        more: bool,
    },
    /// A `branch_none`: the peer no longer has that branch.
    None {
        node: NodeUuid,
        inc: u32,
        tip_hash: String,
    },
}

/// What [`ForkSink::outbox`] hands the coordinator.
#[derive(Debug, Default)]
pub struct ForkOutbox {
    /// `branch_want` frames, each for the peer node it is addressed to.
    pub sends: Vec<(NodeUuid, BranchWant)>,
    /// The advertised fork refs changed: send a fresh `have`.
    pub refs_changed: bool,
}

/// The sink until incarnations land: one log line per log per hour, and a
/// fork stays paused forever.
pub struct LogOnlyForkSink {
    log: LogFn,
    clock: Arc<dyn Clock>,
    last_logged: Mutex<BTreeMap<LogKey, MonoInstant>>,
}

impl LogOnlyForkSink {
    pub fn new(log: LogFn, clock: Arc<dyn Clock>) -> Self {
        Self {
            log,
            clock,
            last_logged: Mutex::new(BTreeMap::new()),
        }
    }
}

pub(super) fn uuid8(node: &NodeUuid) -> String {
    node.to_string().chars().take(8).collect()
}

impl ForkSink for LogOnlyForkSink {
    fn on_fork(&self, log: LogKey, _evidence: ForkEvidence, source: NodeUuid) -> ForkAck {
        let now = self.clock.monotonic_now();
        let due = {
            let mut last = self
                .last_logged
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let due = last
                .get(&log)
                .is_none_or(|at| now.0.saturating_sub(at.0) >= FORK_LOG_EVERY_MS);
            if due {
                last.insert(log, now);
            }
            due
        };
        if due {
            (self.log)(&format!(
                "sync: fork on {}/{} from {}; log paused",
                uuid8(&log.node),
                log.incarnation,
                uuid8(&source)
            ));
        }
        ForkAck::Paused
    }

    fn resumed(&self) -> Vec<LogKey> {
        Vec::new()
    }
}

// ---------------------------------------------------------------------------
// Entry points: PeerIntake and DialedStream
// ---------------------------------------------------------------------------

/// A sniffed inbound peer connection: the stream, the dialer's hello frame
/// already read from it, and the transport's permit.
pub struct PeerStream {
    pub stream: BoxedIo,
    pub first_frame: Vec<u8>,
    pub permit: PeerPermit,
}

/// A refused [`PeerIntake::offer`]: the stream comes back so the transport can
/// write `close{busy}` on it.
pub struct Busy(pub PeerStream);

/// The bounded hand-off from the daemon transport to the coordinator for
/// sniffed inbound peer connections.
///
/// Responsibility (PeerIntake)
/// - Owns: the 16-slot `mpsc` sender, and the [`PeerStream`] value that
///   carries the peer permit with the stream into the coordinator's `JoinSet`.
/// - Hides: that the receiver is an actor; the transport only offers and
///   either hands over or gets [`Busy`].
/// - Must not: run the handshake or hold the stream after a successful offer;
///   the transport task returns at once, so the 250 ms drain never waits on a
///   link. Inbound admission rules go here and in `peer_connection::hand_off`.
/// - Relationships: created by [`SyncCoordinator::start`], passed as `Option`
///   into `serve_with_gate`; `peer_connection::hand_off` is its only caller.
///   `None` (coordinator not started or stopped) and a full or closed channel
///   all end in `close{busy}`.
/// - Lifecycle: lives as long as the coordinator; the transport holds a clone
///   until it stops serving.
/// - Errors: a refused offer is one outcome (busy), not an error the
///   transport tells apart further.
#[derive(Clone)]
pub struct PeerIntake {
    tx: mpsc::Sender<PeerStream>,
}

impl PeerIntake {
    pub(crate) fn with_slots(slots: usize) -> (Self, mpsc::Receiver<PeerStream>) {
        let (tx, rx) = mpsc::channel(slots);
        (Self { tx }, rx)
    }

    /// Hand `stream` to the coordinator, or get it back as [`Busy`] when the
    /// channel is full or the coordinator has stopped. Never waits.
    pub fn offer(&self, stream: PeerStream) -> Result<(), Busy> {
        self.tx.try_send(stream).map_err(|e| match e {
            mpsc::error::TrySendError::Full(s) | mpsc::error::TrySendError::Closed(s) => Busy(s),
        })
    }
}

/// What the actor tells a DialTask about its link. A dropped sender without a
/// `LinkEnded` (the actor stopped) reads as `LinkEnded { why: Shutdown }`.
#[derive(Debug)]
pub enum DialReport {
    /// The handshake completed (sent once, right after it, before the link is
    /// ranked).
    Established { node_uuid: NodeUuid },
    /// The link is over (sent exactly once, last). A handshake that failed, or
    /// was refused on the held-genesis check, carries `peer_node: None`.
    LinkEnded {
        target_id: TargetId,
        why: LinkError,
        up_for: Duration,
        peer_node: Option<NodeUuid>,
    },
}

/// A dial the Dialer hands over: the ssh child's stdio and the channel for
/// [`DialReport`]s (bounded to [`DIAL_REPORT_SLOTS`]).
pub struct DialedStream {
    pub target_id: TargetId,
    pub dest: String,
    pub stdout: Box<dyn AsyncRead + Unpin + Send>,
    pub stdin: Box<dyn AsyncWrite + Unpin + Send>,
    pub report: mpsc::Sender<DialReport>,
}

// ---------------------------------------------------------------------------
// Deps and handle
// ---------------------------------------------------------------------------

/// Everything the coordinator is composed from (`src/app.rs` builds it once).
pub struct CoordinatorDeps {
    pub journal: Arc<dyn JournalPort>,
    pub sync_store: Arc<dyn SyncStorePort>,
    pub identity: Arc<NodeIdentity>,
    /// Random per daemon process (tells a self link from a clone).
    pub boot: Uuid,
    pub writer: Arc<FairWriter>,
    pub clock: Arc<dyn Clock>,
    /// The `Lane::Sync` pacer.
    pub pacer: Arc<Pacer>,
    /// The sync lane's health status.
    pub status: Arc<WorkerStatus>,
    pub log: LogFn,
    pub fork_sink: Arc<dyn ForkSink>,
    /// The Dialer's transport template and test jitter seed.
    pub dial: DialConfig,
    pub cancel: Cancellation,
}

/// How a link takes part in replication. Duplicate-link ranking is
/// ht-5fr.5.12's; until then every link reports `Active`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LinkMode {
    #[default]
    Active,
    Standby,
}

/// One link in a [`SyncSnapshot`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkStatus {
    pub node: NodeUuid,
    pub tag: NodeTag,
    /// The peer's role on this link (`Dialer` when it connected to us).
    pub role: Role,
    pub mode: LinkMode,
    /// The ssh destination of a dialed link.
    pub dest: Option<String>,
    pub since: UtcMillis,
    /// Filled by ht-5fr.5.13.
    pub rx_logs: u32,
    /// Filled by ht-5fr.5.13.
    pub unsolicited: u64,
    pub last_error: Option<String>,
}

/// The coordinator's state for `sync status`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncSnapshot {
    /// At most [`SNAPSHOT_MAX_LINKS`], by link id.
    pub links: Vec<LinkStatus>,
    /// Filled by ht-5fr.5.13.
    pub paused_logs: Vec<LogKey>,
    /// Filled by ht-5fr.5.13.
    pub headroom_until: Option<UtcMillis>,
}

enum Request {
    Snapshot(oneshot::Sender<SyncSnapshot>),
}

/// The cloneable facade over the coordinator actor.
///
/// Responsibility: see the module documentation (SyncCoordinator / SyncHandle).
/// It owns only channel ends and one atomic; it never touches actor state.
#[derive(Clone)]
pub struct SyncHandle {
    links: Arc<AtomicUsize>,
    requests: mpsc::Sender<Request>,
    dialed: mpsc::Sender<DialedStream>,
    dial: Arc<DialShared>,
    clock: Arc<dyn Clock>,
}

impl SyncHandle {
    /// Links that are established or handshaking, inbound or dialed.
    pub fn link_count(&self) -> usize {
        self.links.load(Ordering::Acquire)
    }

    /// Ask the Dialer to re-read the enabled dial targets now (new targets
    /// get a task, removed ones are cancelled). Never waits.
    pub fn reload_targets(&self) {
        self.dial.request_reload();
    }

    /// Enabled dial targets after the Dialer's last reload diff (0 before the
    /// first). The idle exit reads it.
    pub fn enabled_target_count(&self) -> usize {
        self.dial.enabled_targets()
    }

    /// The actor's links, answered within `budget` or `StoreBusy`.
    pub async fn snapshot(&self, budget: &CallBudget) -> Result<SyncSnapshot, ApiError> {
        let busy = || ApiError::new(ErrorCode::StoreBusy, "sync coordinator did not answer");
        let remaining = budget
            .deadline
            .0
            .saturating_sub(self.clock.monotonic_now().0);
        if remaining == 0 || budget.cancellation.is_cancelled() {
            return Err(busy());
        }
        let (tx, rx) = oneshot::channel();
        let ask = async {
            self.requests
                .send(Request::Snapshot(tx))
                .await
                .map_err(|_| busy())?;
            rx.await.map_err(|_| busy())
        };
        tokio::select! {
            result = timeout(Duration::from_millis(remaining), ask) => {
                result.unwrap_or_else(|_| Err(busy()))
            }
            () = budget.cancellation.cancelled() => Err(busy()),
        }
    }

    /// Hand a dialed stream to the actor; it comes back when the actor has
    /// stopped. For the Dialer leaf and tests.
    pub async fn hand_off_dialed(&self, dialed: DialedStream) -> Result<(), DialedStream> {
        self.dialed.send(dialed).await.map_err(|e| e.0)
    }
}

// ---------------------------------------------------------------------------
// The actor
// ---------------------------------------------------------------------------

/// Starts the coordinator actor.
pub struct SyncCoordinator;

impl SyncCoordinator {
    /// Spawn the actor on the current tokio runtime. The `JoinHandle` ends
    /// after `deps.cancel` fires and every link has had its `close{shutdown}`
    /// (bounded at one second).
    pub fn start(deps: CoordinatorDeps) -> (SyncHandle, PeerIntake, JoinHandle<()>) {
        let links = Arc::new(AtomicUsize::new(0));
        let (intake, intake_rx) = PeerIntake::with_slots(PEER_INTAKE_SLOTS);
        let (dialed_tx, dialed_rx) = mpsc::channel(DIALED_SLOTS);
        let (requests_tx, requests_rx) = mpsc::channel(REQUEST_SLOTS);
        let (events_tx, events_rx) = mpsc::channel(LINK_EVENT_SLOTS);
        let (ingest_tx, ingest_rx) = mpsc::channel(LINK_EVENT_SLOTS);
        let dialer = Dialer::start(
            DialerDeps {
                store: deps.sync_store.clone(),
                clock: deps.clock.clone(),
                log: deps.log.clone(),
                dialed: dialed_tx.clone(),
            },
            deps.dial.clone(),
        );
        let handle = SyncHandle {
            links: links.clone(),
            requests: requests_tx,
            dialed: dialed_tx,
            dial: dialer.shared(),
            clock: deps.clock.clone(),
        };
        let me = Arc::new(LocalHello::new(deps.identity.clone(), deps.boot));
        let actor = Actor {
            me,
            deps,
            count: links,
            links: BTreeMap::new(),
            by_node: BTreeMap::new(),
            rx: BTreeMap::new(),
            local: Vector::default(),
            next_link_id: 1,
            handshakes: JoinSet::new(),
            events_tx,
            link_cancel: Cancellation::default(),
            dialer,
            _ingest_tx: ingest_tx,
        };
        let join = tokio::spawn(actor.run(Inputs {
            intake: intake_rx,
            dialed: dialed_rx,
            requests: requests_rx,
            events: events_rx,
            ingest: ingest_rx,
        }));
        (handle, intake, join)
    }
}

struct Inputs {
    intake: mpsc::Receiver<PeerStream>,
    dialed: mpsc::Receiver<DialedStream>,
    requests: mpsc::Receiver<Request>,
    events: mpsc::Receiver<LinkEvent>,
    ingest: mpsc::Receiver<IngestDone>,
}

/// An ingest-worker result. Filled by ht-5fr.5.13; the worker does not exist
/// yet, so nothing sends these.
struct IngestDone;

/// The source a log is being pulled from. Filled by ht-5fr.5.13.
#[allow(dead_code)]
struct RxSource {
    link: u64,
    requested_from: u64,
    last_progress: MonoInstant,
}

enum Origin {
    Inbound,
    Dialed {
        target_id: TargetId,
        dest: String,
        report: mpsc::Sender<DialReport>,
    },
}

struct LinkEntry {
    handle: LinkHandle,
    origin: Origin,
    since: UtcMillis,
    /// Held so the transport's slot is released when the link is.
    _permit: Option<PeerPermit>,
}

/// A finished handshake attempt, joined from the actor's `JoinSet`.
struct Admission {
    link_id: u64,
    origin: Origin,
    permit: Option<PeerPermit>,
    result: Result<Established<BoxedIo>, LinkError>,
}

struct Actor {
    deps: CoordinatorDeps,
    me: Arc<LocalHello>,
    /// Shared with [`SyncHandle::link_count`]: handshaking plus established.
    count: Arc<AtomicUsize>,
    links: BTreeMap<u64, LinkEntry>,
    /// Links per peer node. Ranking is ht-5fr.5.12's.
    by_node: BTreeMap<NodeUuid, Vec<u64>>,
    /// Filled by ht-5fr.5.13.
    #[allow(dead_code)]
    rx: BTreeMap<LogKey, RxSource>,
    /// The last read of `journal.vector()`. Filled by ht-5fr.5.13.
    #[allow(dead_code)]
    local: Vector,
    next_link_id: u64,
    handshakes: JoinSet<Admission>,
    events_tx: mpsc::Sender<LinkEvent>,
    /// Ends link tasks that outlive the shutdown bound. Never cancelled
    /// earlier: the actor closes links itself.
    link_cancel: Cancellation,
    /// Owns the dial tasks and their children.
    dialer: Dialer,
    /// Keeps the ingest channel open until the worker (ht-5fr.5.13) takes it.
    _ingest_tx: mpsc::Sender<IngestDone>,
}

impl Actor {
    async fn run(mut self, mut inputs: Inputs) {
        let _liveness = self.deps.status.lane_guard(&self.deps.cancel);
        let cancel = self.deps.cancel.clone();
        let pacer = self.deps.pacer.clone();
        let mut tick = tokio::time::interval(HOUSEKEEPING_TICK);
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                biased;
                () = cancel.cancelled() => break,
                Some(event) = inputs.events.recv() => self.on_link_event(event),
                Some(offer) = inputs.intake.recv() => self.start_inbound(offer),
                Some(dialed) = inputs.dialed.recv() => self.start_dialed(dialed),
                Some(joined) = self.handshakes.join_next(), if !self.handshakes.is_empty() => {
                    if let Ok(admission) = joined {
                        self.on_admission(admission);
                    }
                }
                Some(request) = inputs.requests.recv() => self.on_request(request),
                Some(IngestDone) = inputs.ingest.recv() => {
                    // Ingest results: ht-5fr.5.13 acts on the outcome here.
                }
                wake = pacer.wait(PACER_TICK) => {
                    if wake == Wake::Kicked {
                        // A table or commit kick: re-diff the dial targets
                        // (the Dialer limits this to once a second).
                        self.dialer.kick();
                    }
                    // Have deltas on a kick: ht-5fr.5.13.
                }
                _ = tick.tick() => {
                    // Housekeeping (fork_sink.resumed(), penalties, contact
                    // flush): ht-5fr.5.12 / 5.13.
                }
            }
        }
        self.shutdown(&mut inputs).await;
    }

    fn next_id(&mut self) -> u64 {
        let id = self.next_link_id;
        self.next_link_id += 1;
        id
    }

    fn start_inbound(&mut self, offer: PeerStream) {
        let link_id = self.next_id();
        self.count.fetch_add(1, Ordering::AcqRel);
        let PeerStream {
            stream,
            first_frame,
            permit,
        } = offer;
        let me = self.me.clone();
        let journal = self.deps.journal.clone();
        let log = self.deps.log.clone();
        self.handshakes.spawn(async move {
            let result = admit(stream, &me, Role::Dialed, Some(first_frame), &journal, &log).await;
            Admission {
                link_id,
                origin: Origin::Inbound,
                permit: Some(permit),
                result,
            }
        });
    }

    fn start_dialed(&mut self, dialed: DialedStream) {
        let link_id = self.next_id();
        self.count.fetch_add(1, Ordering::AcqRel);
        let DialedStream {
            target_id,
            dest,
            stdout,
            stdin,
            report,
        } = dialed;
        let me = self.me.clone();
        let journal = self.deps.journal.clone();
        let log = self.deps.log.clone();
        self.handshakes.spawn(async move {
            let io: BoxedIo = Box::new(tokio::io::join(stdout, stdin));
            let result = admit(io, &me, Role::Dialer, None, &journal, &log).await;
            Admission {
                link_id,
                origin: Origin::Dialed {
                    target_id,
                    dest,
                    report,
                },
                permit: None,
                result,
            }
        });
    }

    fn on_admission(&mut self, admission: Admission) {
        let Admission {
            link_id,
            origin,
            permit,
            result,
        } = admission;
        match result {
            Err(why) => {
                self.count.fetch_sub(1, Ordering::AcqRel);
                if let Origin::Dialed {
                    target_id, report, ..
                } = origin
                {
                    let _ = report.try_send(DialReport::LinkEnded {
                        target_id,
                        why,
                        up_for: Duration::ZERO,
                        peer_node: None,
                    });
                }
            }
            Ok(established) => {
                let node = established.peer.node;
                let handle = crate::sync::link::spawn(
                    established,
                    link_id,
                    self.events_tx.clone(),
                    self.deps.clock.clone(),
                    self.link_cancel.clone(),
                );
                if let Origin::Dialed { report, .. } = &origin {
                    let _ = report.try_send(DialReport::Established { node_uuid: node });
                }
                self.by_node.entry(node).or_default().push(link_id);
                self.links.insert(
                    link_id,
                    LinkEntry {
                        handle,
                        origin,
                        since: self.deps.clock.utc_now(),
                        _permit: permit,
                    },
                );
            }
        }
    }

    fn on_link_event(&mut self, event: LinkEvent) {
        match event {
            LinkEvent::Ended {
                link_id,
                why,
                up_for,
            } => self.on_ended(link_id, why, up_for),
            LinkEvent::Frame { .. } => {
                // Frame routing (have, want, entries, seen): ht-5fr.5.12 and
                // ht-5fr.5.13. Until then a frame is read and dropped.
            }
            LinkEvent::CreditFreed { .. } => {
                // Resuming a blocked push: ht-5fr.5.13.
            }
        }
    }

    fn on_ended(&mut self, link_id: u64, why: LinkError, up_for: Duration) {
        let Some(entry) = self.links.remove(&link_id) else {
            return;
        };
        let node = entry.handle.peer().node;
        if let Some(ids) = self.by_node.get_mut(&node) {
            ids.retain(|id| *id != link_id);
            if ids.is_empty() {
                self.by_node.remove(&node);
            }
        }
        self.count.fetch_sub(1, Ordering::AcqRel);
        if let Origin::Dialed {
            target_id, report, ..
        } = entry.origin
        {
            let _ = report.try_send(DialReport::LinkEnded {
                target_id,
                why,
                up_for,
                peer_node: Some(node),
            });
        }
    }

    fn on_request(&self, request: Request) {
        match request {
            Request::Snapshot(reply) => {
                let links = self
                    .links
                    .values()
                    .take(SNAPSHOT_MAX_LINKS)
                    .map(|entry| {
                        let peer: &PeerInfo = entry.handle.peer();
                        LinkStatus {
                            node: peer.node,
                            tag: peer.tag,
                            role: peer.role,
                            mode: LinkMode::default(),
                            dest: match &entry.origin {
                                Origin::Inbound => None,
                                Origin::Dialed { dest, .. } => Some(dest.clone()),
                            },
                            since: entry.since,
                            rx_logs: 0,
                            unsolicited: 0,
                            last_error: None,
                        }
                    })
                    .collect();
                let _ = reply.send(SyncSnapshot {
                    links,
                    ..SyncSnapshot::default()
                });
            }
        }
    }

    /// `close{shutdown}` on every link, then wait up to one second for the
    /// link tasks to report their end; stragglers are cancelled.
    async fn shutdown(&mut self, inputs: &mut Inputs) {
        for entry in self.links.values() {
            let _ = entry.handle.close(CloseReason::Shutdown);
        }
        let bound = tokio::time::sleep(CLOSE_WRITE_TIMEOUT);
        tokio::pin!(bound);
        while !self.links.is_empty() {
            tokio::select! {
                Some(event) = inputs.events.recv() => {
                    if let LinkEvent::Ended { link_id, why, up_for } = event {
                        self.on_ended(link_id, why, up_for);
                    }
                }
                () = &mut bound => break,
            }
        }
        self.link_cancel.cancel();
        self.handshakes.abort_all();
        self.dialer.shutdown().await;
        let leftover: Vec<u64> = self.links.keys().copied().collect();
        for link_id in leftover {
            self.on_ended(link_id, LinkError::Shutdown, Duration::ZERO);
        }
    }
}

// ---------------------------------------------------------------------------
// Admission: handshake plus the held-genesis key check
// ---------------------------------------------------------------------------

/// Run the handshake as `role`, then compare the hello's public key with the
/// genesis key this node already holds for that node. A mismatch writes
/// `close{auth_failed}` and refuses the link.
async fn admit(
    io: BoxedIo,
    me: &LocalHello,
    role: Role,
    first_frame: Option<Vec<u8>>,
    journal: &Arc<dyn JournalPort>,
    log: &LogFn,
) -> Result<Established<BoxedIo>, LinkError> {
    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    let mut established = handshake(io, me, role, first_frame, deadline).await?;
    let node = established.peer.node;
    let held = {
        let journal = journal.clone();
        tokio::task::spawn_blocking(move || held_genesis_pubkey(journal.as_ref(), &node)).await
    };
    match held {
        Ok(Ok(Some(key))) if key != established.peer.pubkey => {
            let close = PeerFrame::Close {
                reason: CloseReason::AuthFailed,
                detail: None,
            };
            let _ = timeout(CLOSE_WRITE_TIMEOUT, async {
                write_peer_frame(&mut established.writer, &close).await?;
                established.writer.flush().await
            })
            .await;
            Err(LinkError::Refused(CloseReason::AuthFailed))
        }
        Ok(Ok(_)) => Ok(established),
        Ok(Err(e)) => {
            log(&format!(
                "sync: held key lookup for {} failed: {:?}",
                uuid8(&node),
                e.code
            ));
            Ok(established)
        }
        Err(_) => Ok(established),
    }
}

/// The public key in the lowest-incarnation genesis this journal holds for
/// `node`, if any.
fn held_genesis_pubkey(
    journal: &dyn JournalPort,
    node: &NodeUuid,
) -> Result<Option<[u8; 32]>, ApiError> {
    let budget = CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Cancellation::default(),
    };
    let vector = journal.vector(&budget)?;
    let Some(log) = vector.0.keys().find(|log| log.node == *node) else {
        return Ok(None);
    };
    let limits = BatchLimits {
        entries: 1,
        bytes: BatchLimits::BYTES,
    };
    let first = journal.serve(log, 1, limits, &budget)?;
    Ok(first
        .into_iter()
        .find(|entry| entry.kind == GENESIS_KIND)
        .and_then(|entry| serde_json::from_slice::<GenesisPayload>(&entry.payload).ok())
        .and_then(|payload| hex_decode::<32>(&payload.pubkey)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::transport::{self, MAX_FRAME_BYTES};
    use crate::sync::envelope::{Deps, Envelope, UnsignedEnvelope, hex_encode};
    use crate::sync::frame::read_peer_frame;
    use crate::sync::hlc::HlcStamp;
    use crate::sync::journal_port::{ApplyProgress, IngestOutcome};
    use crate::sync::memory_journal::MemoryJournal;
    use crate::sync::sync_store::MemorySyncStore;
    use std::sync::atomic::AtomicU64;
    use tokio::io::{DuplexStream, duplex};

    const BIG: usize = 4 << 20;

    struct TokioClock(Instant);
    impl Clock for TokioClock {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(1_700_000_000_000 + self.0.elapsed().as_millis() as i64)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(self.0.elapsed().as_millis() as u64)
        }
    }

    struct ManualClock(AtomicU64);
    impl Clock for ManualClock {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(0)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(self.0.load(Ordering::SeqCst))
        }
    }

    fn identity(n: u8) -> Arc<NodeIdentity> {
        Arc::new(NodeIdentity::from_seed([n; 32]))
    }

    type Lines = Arc<Mutex<Vec<String>>>;

    fn capture() -> (LogFn, Lines) {
        let lines: Lines = Arc::default();
        let sink = lines.clone();
        let log: LogFn = Arc::new(move |line| sink.lock().unwrap().push(line.to_owned()));
        (log, lines)
    }

    struct Rig {
        handle: SyncHandle,
        intake: PeerIntake,
        join: JoinHandle<()>,
        cancel: Cancellation,
        clock: Arc<dyn Clock>,
    }

    fn start_with(journal: Arc<dyn JournalPort>) -> Rig {
        let clock: Arc<dyn Clock> = Arc::new(TokioClock(Instant::now()));
        let cancel = Cancellation::default();
        let (log, _) = capture();
        let (handle, intake, join) = SyncCoordinator::start(CoordinatorDeps {
            journal,
            sync_store: Arc::new(MemorySyncStore::new()),
            identity: identity(1),
            boot: Uuid::from_u128(1),
            writer: Arc::new(FairWriter::new(8)),
            clock: clock.clone(),
            pacer: Arc::new(Pacer::new("sync", clock.clone(), cancel.clone())),
            status: Arc::new(WorkerStatus::default()),
            log: log.clone(),
            fork_sink: Arc::new(LogOnlyForkSink::new(log, clock.clone())),
            dial: DialConfig::default(),
            cancel: cancel.clone(),
        });
        Rig {
            handle,
            intake,
            join,
            cancel,
            clock,
        }
    }

    fn start() -> Rig {
        start_with(Arc::new(MemoryJournal::new(identity(1))))
    }

    fn peer_hello(n: u8) -> LocalHello {
        LocalHello::new(identity(n), Uuid::from_u128(u128::from(n)))
    }

    fn deadline() -> Instant {
        Instant::now() + HANDSHAKE_TIMEOUT
    }

    /// The remote dialer of an inbound link: handshakes over a duplex whose
    /// other end is offered to the intake the way the transport does (after
    /// sniffing the first frame).
    async fn connect_inbound(rig: &Rig, peer: &LocalHello) -> Established<DuplexStream> {
        let (a, mut b) = duplex(BIG);
        let dialer = crate::sync::link::handshake(a, peer, Role::Dialer, None, deadline());
        let offer = async {
            let first = transport::read_frame_with_limit(&mut b, MAX_FRAME_BYTES)
                .await
                .unwrap();
            rig.intake
                .offer(PeerStream {
                    stream: Box::new(b),
                    first_frame: first,
                    permit: Box::new(()),
                })
                .ok()
                .expect("intake has room");
        };
        let (est, ()) = tokio::join!(dialer, offer);
        est.expect("the dialer side completes the handshake")
    }

    struct DialedRig {
        report: mpsc::Receiver<DialReport>,
        /// The remote end of the ssh child's stdio.
        remote: DuplexStream,
    }

    /// Hand the coordinator a dialed stream; returns the remote end.
    async fn dial(rig: &Rig, target: i64, dest: &str) -> DialedRig {
        let (ours, remote) = duplex(BIG);
        let (stdout, stdin) = tokio::io::split(ours);
        let (report_tx, report) = mpsc::channel(DIAL_REPORT_SLOTS);
        rig.handle
            .hand_off_dialed(DialedStream {
                target_id: TargetId(target),
                dest: dest.to_owned(),
                stdout: Box::new(stdout),
                stdin: Box::new(stdin),
                report: report_tx,
            })
            .await
            .ok()
            .expect("the actor is running");
        DialedRig { report, remote }
    }

    async fn answer_dialed(
        remote: &mut DuplexStream,
        peer: &LocalHello,
    ) -> Established<DuplexStream> {
        let first = transport::read_frame_with_limit(remote, MAX_FRAME_BYTES)
            .await
            .unwrap();
        // Hand the handshake the stream by value through a scratch duplex swap.
        let io = std::mem::replace(remote, duplex(1).0);
        crate::sync::link::handshake(io, peer, Role::Dialed, Some(first), deadline())
            .await
            .expect("the dialed side completes the handshake")
    }

    async fn until(mut cond: impl FnMut() -> bool) {
        for _ in 0..200 {
            if cond() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("condition not reached");
    }

    /// Wait until the actor lists `n` established links (a handshake the peer
    /// has finished may not be registered yet).
    async fn listed(rig: &Rig, n: usize) {
        for _ in 0..200 {
            let budget = CallBudget {
                deadline: MonoInstant(rig.clock.monotonic_now().0 + 5_000),
                cancellation: Cancellation::default(),
            };
            if rig.handle.snapshot(&budget).await.unwrap().links.len() == n {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("links not listed");
    }

    async fn next_close(est: &mut Established<DuplexStream>) -> CloseReason {
        loop {
            match read_peer_frame(&mut est.reader, MAX_FRAME_BYTES).await {
                Ok(PeerFrame::Close { reason, .. }) => return reason,
                Ok(_) => continue,
                Err(e) => panic!("stream ended without a close: {e}"),
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn inbound_peer_stream_link_count_0_1_0() {
        let rig = start();
        assert_eq!(rig.handle.link_count(), 0);
        let est = connect_inbound(&rig, &peer_hello(2)).await;
        until(|| rig.handle.link_count() == 1).await;
        drop(est);
        until(|| rig.handle.link_count() == 0).await;
        rig.cancel.cancel();
        rig.join.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn dialed_stream_reports_established_then_ended_once_each() {
        let rig = start();
        let peer = peer_hello(2);
        let mut d = dial(&rig, 7, "b-host").await;
        let est = answer_dialed(&mut d.remote, &peer).await;
        match d.report.recv().await.expect("established report") {
            DialReport::Established { node_uuid } => {
                assert_eq!(node_uuid, peer.identity.uuid());
            }
            other => panic!("expected Established first, got {other:?}"),
        }
        assert_eq!(rig.handle.link_count(), 1);
        drop(est);
        match d.report.recv().await.expect("ended report") {
            DialReport::LinkEnded {
                target_id,
                peer_node,
                ..
            } => {
                assert_eq!(target_id, TargetId(7));
                assert_eq!(peer_node, Some(peer.identity.uuid()));
            }
            other => panic!("expected LinkEnded last, got {other:?}"),
        }
        assert!(
            d.report.recv().await.is_none(),
            "nothing follows LinkEnded and the sender is released"
        );
        until(|| rig.handle.link_count() == 0).await;
        rig.cancel.cancel();
        rig.join.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn refused_dialed_handshake_only_link_ended_no_peer() {
        let rig = start();
        let mut d = dial(&rig, 3, "c-host").await;
        let _first = transport::read_frame_with_limit(&mut d.remote, MAX_FRAME_BYTES)
            .await
            .unwrap();
        let busy = PeerFrame::Close {
            reason: CloseReason::Busy,
            detail: None,
        };
        write_peer_frame(&mut d.remote, &busy).await.unwrap();
        match d.report.recv().await.expect("one report") {
            DialReport::LinkEnded {
                target_id,
                why,
                peer_node,
                ..
            } => {
                assert_eq!(target_id, TargetId(3));
                assert!(
                    matches!(why, LinkError::PeerClosed(CloseReason::Busy)),
                    "{why:?}"
                );
                assert_eq!(peer_node, None);
            }
            other => panic!("expected only LinkEnded, got {other:?}"),
        }
        assert!(d.report.recv().await.is_none(), "no second report");
        assert_eq!(rig.handle.link_count(), 0);
        rig.cancel.cancel();
        rig.join.await.unwrap();
    }

    /// A journal that claims to hold a genesis for `victim` signed with a
    /// different key.
    struct ForgedGenesis {
        victim: NodeUuid,
        forged: Envelope,
    }

    impl ForgedGenesis {
        fn new(victim: NodeUuid, other: &NodeIdentity) -> Self {
            let log = LogKey {
                node: victim,
                incarnation: 1,
            };
            let payload = GenesisPayload {
                pubkey: hex_encode(&other.public_key()),
                tag: other.tag().to_string(),
                tag_k: 0,
                incarnation: 1,
                predecessor: None,
                host_label: "forged".into(),
            };
            let forged = UnsignedEnvelope {
                log,
                counter: 1,
                prev_hash: [0; 32],
                hlc: HlcStamp::from_parts(1_700_000_000_001, 0),
                kind: GENESIS_KIND.to_owned(),
                kind_version: 1,
                deps: Deps::empty(),
                creates: None,
                needs: Vec::new(),
                payload: serde_json::to_vec(&payload).unwrap(),
            }
            .seal(other);
            Self { victim, forged }
        }
    }

    impl JournalPort for ForgedGenesis {
        fn vector(&self, _: &CallBudget) -> Result<Vector, ApiError> {
            let mut vector = Vector::default();
            vector.0.insert(
                self.forged.log,
                crate::sync::LogHead {
                    counter: 1,
                    head_hash: self.forged.hash,
                    state: crate::sync::LogState::Active,
                    frozen_at: None,
                },
            );
            assert_eq!(self.forged.log.node, self.victim);
            Ok(vector)
        }
        fn stored_hash(
            &self,
            _: &LogKey,
            _: u64,
            _: &CallBudget,
        ) -> Result<Option<[u8; 32]>, ApiError> {
            Ok(None)
        }
        fn serve(
            &self,
            _: &LogKey,
            _: u64,
            _: BatchLimits,
            _: &CallBudget,
        ) -> Result<Vec<Envelope>, ApiError> {
            Ok(vec![self.forged.clone()])
        }
        fn serve_branch(
            &self,
            _: &LogKey,
            _: (u64, [u8; 32]),
            _: u64,
            _: BatchLimits,
            _: &CallBudget,
        ) -> Result<Option<Vec<Envelope>>, ApiError> {
            Ok(None)
        }
        fn log_meta(
            &self,
            _: &LogKey,
            _: &CallBudget,
        ) -> Result<Option<crate::sync::journal_port::LogMeta>, ApiError> {
            Ok(None)
        }
        fn ingest(
            &self,
            _: &LogKey,
            _: Vec<Envelope>,
            _: &CallBudget,
        ) -> Result<IngestOutcome, ApiError> {
            Ok(IngestOutcome::Headroom)
        }
        fn apply_pending(&self, _: &CallBudget) -> Result<ApplyProgress, ApiError> {
            Ok(ApplyProgress::default())
        }
    }

    #[tokio::test(start_paused = true)]
    async fn held_genesis_pubkey_mismatch_auth_failed() {
        let peer = peer_hello(2);
        let journal = ForgedGenesis::new(peer.identity.uuid(), &NodeIdentity::from_seed([9; 32]));
        let rig = start_with(Arc::new(journal));

        // Inbound: the dialer completes the handshake, then reads the close.
        let mut est = connect_inbound(&rig, &peer).await;
        assert_eq!(next_close(&mut est).await, CloseReason::AuthFailed);
        until(|| rig.handle.link_count() == 0).await;

        // Dialed: no Established, one LinkEnded with no peer.
        let mut d = dial(&rig, 5, "d-host").await;
        let mut est = answer_dialed(&mut d.remote, &peer).await;
        assert_eq!(next_close(&mut est).await, CloseReason::AuthFailed);
        match d.report.recv().await.expect("one report") {
            DialReport::LinkEnded { why, peer_node, .. } => {
                assert!(
                    matches!(why, LinkError::Refused(CloseReason::AuthFailed)),
                    "{why:?}"
                );
                assert_eq!(peer_node, None);
            }
            other => panic!("expected only LinkEnded, got {other:?}"),
        }
        assert!(d.report.recv().await.is_none());
        assert_eq!(rig.handle.link_count(), 0);
        rig.cancel.cancel();
        rig.join.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn snapshot_lists_link_and_store_busy_past_budget() {
        let rig = start();
        let peer = peer_hello(2);
        let _inbound = connect_inbound(&rig, &peer).await;
        let mut d = dial(&rig, 4, "e-host").await;
        let other = peer_hello(3);
        let _dialed = answer_dialed(&mut d.remote, &other).await;
        listed(&rig, 2).await;

        let budget = CallBudget {
            deadline: MonoInstant(rig.clock.monotonic_now().0 + 5_000),
            cancellation: Cancellation::default(),
        };
        let snapshot = rig.handle.snapshot(&budget).await.unwrap();
        assert_eq!(snapshot.links.len(), 2);
        let inbound = snapshot
            .links
            .iter()
            .find(|l| l.node == peer.identity.uuid())
            .expect("the inbound link is listed");
        assert_eq!(inbound.tag, peer.identity.tag());
        assert_eq!(inbound.role, Role::Dialer);
        assert_eq!(inbound.dest, None);
        assert_eq!(inbound.last_error, None);
        let dialed = snapshot
            .links
            .iter()
            .find(|l| l.node == other.identity.uuid())
            .expect("the dialed link is listed");
        assert_eq!(dialed.role, Role::Dialed);
        assert_eq!(dialed.dest.as_deref(), Some("e-host"));
        assert!(snapshot.paused_logs.is_empty());
        assert_eq!(snapshot.headroom_until, None);

        // A budget that is already spent never reaches the actor.
        let spent = CallBudget {
            deadline: rig.clock.monotonic_now(),
            cancellation: Cancellation::default(),
        };
        let err = rig.handle.snapshot(&spent).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::StoreBusy);

        // An actor that never answers: the budget ends the wait.
        let (requests, _held_open) = mpsc::channel(1);
        let (dialed_tx, _dialed_rx) = mpsc::channel(1);
        let stalled = SyncHandle {
            links: Arc::default(),
            requests,
            dialed: dialed_tx,
            dial: rig.handle.dial.clone(),
            clock: rig.clock.clone(),
        };
        let short = CallBudget {
            deadline: MonoInstant(rig.clock.monotonic_now().0 + 50),
            cancellation: Cancellation::default(),
        };
        let err = stalled.snapshot(&short).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::StoreBusy);

        rig.cancel.cancel();
        rig.join.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn cancel_sends_close_shutdown_on_every_link() {
        let rig = start();
        let mut inbound = connect_inbound(&rig, &peer_hello(2)).await;
        let mut d = dial(&rig, 8, "f-host").await;
        let mut dialed = answer_dialed(&mut d.remote, &peer_hello(3)).await;
        listed(&rig, 2).await;

        rig.cancel.cancel();
        assert_eq!(next_close(&mut inbound).await, CloseReason::Shutdown);
        assert_eq!(next_close(&mut dialed).await, CloseReason::Shutdown);
        rig.join.await.unwrap();
        assert_eq!(rig.handle.link_count(), 0);
        // The dialed link's report ends with LinkEnded, last.
        let mut last = None;
        while let Some(report) = d.report.recv().await {
            last = Some(report);
        }
        assert!(
            matches!(last, Some(DialReport::LinkEnded { .. })),
            "{last:?}"
        );
    }

    #[test]
    fn peer_intake_offer_full_is_busy() {
        let (intake, _rx) = PeerIntake::with_slots(PEER_INTAKE_SLOTS);
        let stream = || PeerStream {
            stream: Box::new(duplex(8).0),
            first_frame: vec![1],
            permit: Box::new(()),
        };
        for _ in 0..PEER_INTAKE_SLOTS {
            assert!(intake.offer(stream()).is_ok());
        }
        let Err(Busy(returned)) = intake.offer(stream()) else {
            panic!("the 17th offer must be busy");
        };
        assert_eq!(returned.first_frame, vec![1], "the stream comes back");

        let (closed, rx) = PeerIntake::with_slots(1);
        drop(rx);
        assert!(closed.offer(stream()).is_err(), "a stopped actor is busy");
    }

    #[test]
    fn log_only_fork_sink_paused_rate_limited_never_resumes() {
        let (log, lines) = capture();
        let clock = Arc::new(ManualClock(AtomicU64::new(0)));
        let sink = LogOnlyForkSink::new(log, clock.clone());
        let a = LogKey {
            node: NodeUuid::from_bytes([0xaa; 16]),
            incarnation: 1,
        };
        let b = LogKey {
            node: NodeUuid::from_bytes([0xbb; 16]),
            incarnation: 2,
        };
        let source = NodeUuid::from_bytes([0xcc; 16]);
        let evidence = || ForkEvidence::AheadOfSelf {
            log: a,
            my_head: crate::sync::LogHead {
                counter: 1,
                head_hash: [0; 32],
                state: crate::sync::LogState::Active,
                frozen_at: None,
            },
            entry: Box::new(
                UnsignedEnvelope {
                    log: a,
                    counter: 2,
                    prev_hash: [0; 32],
                    hlc: HlcStamp::from_parts(1, 0),
                    kind: "x.y".into(),
                    kind_version: 1,
                    deps: Deps::empty(),
                    creates: None,
                    needs: Vec::new(),
                    payload: b"{}".to_vec(),
                }
                .seal(&NodeIdentity::from_seed([5; 32])),
            ),
        };
        assert_eq!(sink.on_fork(a, evidence(), source), ForkAck::Paused);
        assert_eq!(sink.on_fork(a, evidence(), source), ForkAck::Paused);
        assert_eq!(lines.lock().unwrap().len(), 1, "same log within the hour");
        assert_eq!(
            lines.lock().unwrap()[0],
            "sync: fork on aaaaaaaa/1 from cccccccc; log paused"
        );
        assert_eq!(sink.on_fork(b, evidence(), source), ForkAck::Paused);
        assert_eq!(
            lines.lock().unwrap().len(),
            2,
            "another log logs its own line"
        );
        clock.0.store(FORK_LOG_EVERY_MS - 1, Ordering::SeqCst);
        sink.on_fork(a, evidence(), source);
        assert_eq!(lines.lock().unwrap().len(), 2, "just under the hour");
        clock.0.store(FORK_LOG_EVERY_MS, Ordering::SeqCst);
        sink.on_fork(a, evidence(), source);
        assert_eq!(lines.lock().unwrap().len(), 3, "an hour later logs again");
        assert!(sink.resumed().is_empty(), "never resumes");
    }
}
