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
//! - Routing (ht-5fr.5.12): duplicate-link ranking and standby, the full
//!   `Have` on activation, `peer_vector` merge, rx-source selection
//!   (`reselect`) with its 30 s failover, clone detection at a third node and
//!   the own-history `Links` flag. The pure decisions live in `routing`
//!   (`duplicate_order`, `reselect`, `wantable`, `own_history_on`): that is
//!   where relay-policy changes go; the actor only applies their answers.
//! - Where new behaviour goes: serving, live tail, ingest and Have deltas
//!   are ht-5fr.5.13. Each fills the marked stub arm of [`Actor::run`] and the
//!   state fields marked below; `signals()` is ht-5fr.5.6's. The Dialer
//!   (`super::dialer`, ht-5fr.5.7) is constructed here: [`SyncHandle::reload_targets`]
//!   and [`SyncHandle::enabled_target_count`] reach it, a pacer kick re-diffs
//!   its targets, and shutdown cancels its tasks.
//! - Role: the single owner of per-link and per-log sync state, so routing
//!   decisions that span links need no locks.
//!
//! Every `JournalPort` and `SyncStorePort` call is synchronous and takes
//! writer turns, so an actor that makes one runs it in `spawn_blocking`: the
//! held-genesis lookup at admission and the `vector()` read that refreshes
//! `local` (a `JoinSet` job whose result the actor joins).
//!
//! Core and harness-neutral.

use std::any::Any;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::{Instant, MissedTickBehavior, timeout};
use uuid::Uuid;

use super::dialer::{DialConfig, DialShared, Dialer, DialerDeps};
use super::envelope::{GENESIS_KIND, WireEnvelope, hex_decode, hex_encode};
use super::facts::GenesisPayload;
use super::frame::{
    BranchWant, CloseReason, FORK_CAP, ForkRef, HaveLog, MAX_HAVE_LOGS, PeerFrame, Role,
    write_peer_frame,
};
use super::journal_port::{ForkEvidence, JournalPort};
use super::link::{
    CLOSE_WRITE_TIMEOUT, Established, HANDSHAKE_TIMEOUT, LinkError, LinkEvent, LinkHandle,
    LocalHello, PeerInfo, handshake,
};
use super::sync_store::SyncStorePort;
use super::{BatchLimits, LogHead, LogKey, LogState, Vector};
use crate::node::{NodeIdentity, NodeTag, NodeUuid};
use crate::protocol::results::{ApiError, ErrorCode};
use crate::protocol::time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis};
use crate::service::fair_writer::FairWriter;
use crate::service::pacer::{Pacer, Wake};
use crate::service::workers::WorkerStatus;
use crate::store::connection::{HoldSource, OwnHistoryHold};
use crate::store::sync_dial_targets::TargetId;

mod routing;

use routing::{DupKey, SourceView};

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
/// `LogOnlyForkSink` logs at most once per log (or clone subject) per this many
/// milliseconds.
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

    /// Two live daemons are suspected to hold one node key. Default: ignored.
    fn on_clone(&self, _report: CloneReport) {}
}

/// How a suspected clone was noticed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloneKind {
    /// A handshake was refused with `node_collision` (this node's key shows
    /// up on the other end with another boot).
    Handshake,
    /// An established peer link ended with the peer's `close{node_collision}`.
    PeerClose,
    /// Two live links carry one third node's uuid with different boots.
    ThirdParty,
}

impl CloneKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Handshake => "handshake",
            Self::PeerClose => "peer_close",
            Self::ThirdParty => "third_party",
        }
    }
}

/// A clone suspicion handed to [`ForkSink::on_clone`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloneReport {
    /// The node whose key is held twice: this node for `Handshake` and
    /// `PeerClose`, the third node for `ThirdParty`.
    pub subject: NodeUuid,
    pub kind: CloneKind,
    /// The two boots seen for `ThirdParty` (`[older, newer]`); empty when the
    /// other copy's boot is not known.
    pub boots: Vec<Uuid>,
    /// Who reported it: this node, except for `PeerClose`, where it is the
    /// peer that closed.
    pub source: NodeUuid,
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
    clone_logged: Mutex<BTreeMap<NodeUuid, MonoInstant>>,
}

impl LogOnlyForkSink {
    pub fn new(log: LogFn, clock: Arc<dyn Clock>) -> Self {
        Self {
            log,
            clock,
            last_logged: Mutex::new(BTreeMap::new()),
            clone_logged: Mutex::new(BTreeMap::new()),
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

    fn on_clone(&self, report: CloneReport) {
        let now = self.clock.monotonic_now();
        let due = {
            let mut last = self
                .clone_logged
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let due = last
                .get(&report.subject)
                .is_none_or(|at| now.0.saturating_sub(at.0) >= FORK_LOG_EVERY_MS);
            if due {
                last.insert(report.subject, now);
            }
            due
        };
        if due {
            (self.log)(&format!(
                "sync: clone suspected for {} ({})",
                uuid8(&report.subject),
                report.kind.as_str()
            ));
        }
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
    /// The own-history hold in `SyncState`; the coordinator owns its `Links`
    /// flag.
    pub own_history: Arc<OwnHistoryHold>,
    /// The Dialer's transport template and test jitter seed.
    pub dial: DialConfig,
    pub cancel: Cancellation,
}

/// How a link takes part in replication: the one active link per peer node
/// carries `have`/`want`/`entries`; the others only keep `ping`/`pong`.
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
    /// Logs this link is the rx source of.
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
            paused: BTreeSet::new(),
            headroom_until: None,
            probes: BTreeMap::new(),
            refresh: JoinSet::new(),
            refresh_again: false,
            own_history_on: false,
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

/// The source a log is being pulled from. `last_progress` is set when the
/// source is chosen or re-asked and by ht-5fr.5.13's ingest on `Stored`;
/// `requested_from` is read by its gap handling.
#[allow(dead_code)]
struct RxSource {
    link: u64,
    requested_from: u64,
    last_progress: MonoInstant,
}

/// A live check of whether an older link to the same node is still answering,
/// started when a newer link carries the node's uuid with another boot.
struct CloneProbe {
    older: u64,
    /// The older link's inbound frame count when the probe started.
    base: u64,
    deadline: MonoInstant,
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
    mode: LinkMode,
    /// In the duplicate order. False while a clone probe of an older link
    /// decides this link's fate.
    ranked: bool,
    /// `close` has been queued; the link no longer counts.
    closing: bool,
    /// What the peer advertises. Only an active link keeps one.
    peer_vector: Vector,
    /// A `have` is owed to the peer (`Some(ask)`) once `local` is fresh.
    have_pending: Option<bool>,
    /// This activation's first `have` has gone out; until then the link is
    /// no rx source, so a `want` never precedes the `have`.
    announced: bool,
    /// Penalty-until per log on this link (written here by the failover and
    /// by ht-5fr.5.13's ingest outcomes).
    errors: BTreeMap<LogKey, MonoInstant>,
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
    /// Ranked links per peer node.
    by_node: BTreeMap<NodeUuid, Vec<u64>>,
    /// Where each wanted log is pulled from.
    rx: BTreeMap<LogKey, RxSource>,
    /// The last read of `journal.vector()`.
    local: Vector,
    /// Logs not to be pulled (a fork the sink holds). Written by
    /// ht-5fr.5.13's ingest outcomes.
    paused: BTreeSet<LogKey>,
    /// All wants are paused until then. Written by ht-5fr.5.13 on `Headroom`.
    headroom_until: Option<MonoInstant>,
    /// Clone probes by the newer link's id.
    probes: BTreeMap<u64, CloneProbe>,
    /// `journal.vector()` reads in flight (at most one).
    refresh: JoinSet<Result<Vector, String>>,
    /// Another read was asked for while one was in flight.
    refresh_again: bool,
    /// The last value given to the own-history `Links` flag.
    own_history_on: bool,
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
                Some(joined) = self.refresh.join_next(), if !self.refresh.is_empty() => {
                    self.on_local(joined.unwrap_or_else(|e| Err(e.to_string())));
                }
                Some(request) = inputs.requests.recv() => self.on_request(request),
                Some(IngestDone) = inputs.ingest.recv() => {
                    // Ingest results: ht-5fr.5.13 acts on the outcome here.
                }
                wake = pacer.wait(PACER_TICK) => {
                    if wake == Wake::Kicked {
                        // A table or commit kick: re-diff the dial targets
                        // (the Dialer limits this to once a second) and
                        // re-read the local vector.
                        self.dialer.kick();
                        self.request_local();
                    }
                    // Have deltas on a kick: ht-5fr.5.13.
                }
                _ = tick.tick() => {
                    self.housekeeping();
                    // fork_sink.resumed() and the contact flush: ht-5fr.5.13.
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

    fn mono(&self) -> MonoInstant {
        self.deps.clock.monotonic_now()
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
                if matches!(
                    why,
                    LinkError::Refused(CloseReason::NodeCollision)
                        | LinkError::PeerClosed(CloseReason::NodeCollision)
                ) {
                    self.report_clone(CloneKind::Handshake, None);
                }
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
                let boot = established.peer.boot;
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
                // A live older link of this node with another boot: the
                // newer link waits for the probe of it.
                let older = self.by_node.get(&node).and_then(|ids| {
                    ids.iter().copied().find(|id| {
                        self.links
                            .get(id)
                            .is_some_and(|l| !l.closing && l.handle.peer().boot != boot)
                    })
                });
                self.links.insert(
                    link_id,
                    LinkEntry {
                        handle,
                        origin,
                        since: self.deps.clock.utc_now(),
                        mode: LinkMode::Standby,
                        ranked: false,
                        closing: false,
                        peer_vector: Vector::default(),
                        have_pending: None,
                        announced: false,
                        errors: BTreeMap::new(),
                        _permit: permit,
                    },
                );
                match older {
                    Some(older) => self.start_probe(link_id, older),
                    None => self.rank_link(link_id),
                }
            }
        }
    }

    /// Ping `older` and wait for any inbound frame from it before ranking
    /// `newer`. The check runs on the housekeeping tick, so the actor never
    /// blocks on it.
    fn start_probe(&mut self, newer: u64, older: u64) {
        let Some(entry) = self.links.get(&older) else {
            self.rank_link(newer);
            return;
        };
        if entry.handle.send(PeerFrame::Ping { n: u64::MAX }).is_err() {
            self.rank_link(newer);
            return;
        }
        let probe = CloneProbe {
            older,
            base: entry.handle.inbound_frames(),
            deadline: MonoInstant(self.mono().0 + routing::CLONE_PROBE_MS),
        };
        self.probes.insert(newer, probe);
    }

    /// Settle the clone probes whose older link answered, went silent past the
    /// deadline or is gone.
    fn resolve_probes(&mut self, now: MonoInstant) {
        let pending: Vec<u64> = self.probes.keys().copied().collect();
        for newer in pending {
            let Some(probe) = self.probes.get(&newer) else {
                continue;
            };
            let (older, base, deadline) = (probe.older, probe.base, probe.deadline);
            let answered = self
                .links
                .get(&older)
                .map(|l| l.handle.inbound_frames() > base);
            match answered {
                None => {
                    self.probes.remove(&newer);
                    self.rank_link(newer);
                }
                Some(true) => {
                    self.probes.remove(&newer);
                    self.reject_clone(newer, older);
                }
                Some(false) if now.0 >= deadline.0 => {
                    // A leftover of a peer daemon that restarted: end it,
                    // then rank the newer link as if it were alone.
                    self.probes.remove(&newer);
                    let up_for = self.up_for(older);
                    self.on_ended(older, LinkError::Dead, up_for);
                    self.rank_link(newer);
                }
                Some(false) => {}
            }
        }
    }

    fn up_for(&self, link_id: u64) -> Duration {
        self.links.get(&link_id).map_or(Duration::ZERO, |entry| {
            let now = self.deps.clock.utc_now().0;
            Duration::from_millis(u64::try_from(now.saturating_sub(entry.since.0)).unwrap_or(0))
        })
    }

    /// Two live daemons hold one key: keep the older link, close the newer
    /// with `node_collision` and report it.
    fn reject_clone(&mut self, newer: u64, older: u64) {
        let Some(old_boot) = self.links.get(&older).map(|l| l.handle.peer().boot) else {
            return;
        };
        let Some(entry) = self.links.get_mut(&newer) else {
            return;
        };
        entry.closing = true;
        let _ = entry.handle.close(CloseReason::NodeCollision);
        let peer = entry.handle.peer();
        let report = CloneReport {
            subject: peer.node,
            kind: CloneKind::ThirdParty,
            boots: vec![old_boot, peer.boot],
            source: self.deps.identity.uuid(),
        };
        self.deps.fork_sink.on_clone(report);
    }

    /// A clone suspicion about this node itself (`Handshake`, `PeerClose`).
    fn report_clone(&self, kind: CloneKind, peer: Option<NodeUuid>) {
        let me = self.deps.identity.uuid();
        self.deps.fork_sink.on_clone(CloneReport {
            subject: me,
            kind,
            boots: Vec::new(),
            source: peer.unwrap_or(me),
        });
    }

    /// Put `link_id` into the duplicate order of its node.
    fn rank_link(&mut self, link_id: u64) {
        let Some(entry) = self.links.get_mut(&link_id) else {
            return;
        };
        entry.ranked = true;
        let node = entry.handle.peer().node;
        self.by_node.entry(node).or_default().push(link_id);
        self.rank_node(node, false);
    }

    /// Recompute which link of `node` is active (the pure
    /// [`routing::duplicate_order`]), close the newest standby beyond two,
    /// demote the former active link and activate the new one. `after_end`:
    /// the change is the end of a link, so a promotion asks for the peer's
    /// vector.
    fn rank_node(&mut self, node: NodeUuid, after_end: bool) {
        let order = loop {
            let keys: Vec<DupKey> = self
                .by_node
                .get(&node)
                .into_iter()
                .flatten()
                .filter_map(|id| {
                    let entry = self.links.get(id)?;
                    (entry.ranked && !entry.closing).then(|| {
                        let peer = entry.handle.peer();
                        DupKey {
                            link: *id,
                            dialer: peer.dialer_node,
                            nonce: peer.dialer_nonce,
                        }
                    })
                })
                .collect();
            let order = routing::duplicate_order(&keys);
            if order.len() <= routing::MAX_LINKS_PER_NODE {
                break order;
            }
            let Some(newest) = order[1..].iter().copied().max() else {
                break order;
            };
            if let Some(entry) = self.links.get_mut(&newest) {
                entry.closing = true;
                let _ = entry.handle.close(CloseReason::Busy);
            }
        };
        let Some(&active) = order.first() else {
            return;
        };
        let prev = order.iter().copied().find(|id| {
            self.links
                .get(id)
                .is_some_and(|l| l.mode == LinkMode::Active)
        });
        if prev != Some(active) {
            for id in &order {
                if *id != active
                    && let Some(entry) = self.links.get(id)
                    && entry.mode == LinkMode::Active
                {
                    self.demote(*id);
                }
            }
            self.activate(active, prev.is_some() || after_end);
        }
        self.update_own_history();
    }

    /// The link becomes the node's standby: it keeps ping/pong only.
    fn demote(&mut self, link_id: u64) {
        let Some(entry) = self.links.get_mut(&link_id) else {
            return;
        };
        entry.mode = LinkMode::Standby;
        entry.peer_vector = Vector::default();
        entry.have_pending = None;
        entry.announced = false;
        entry.errors.clear();
        self.drop_rx_of(link_id);
    }

    /// Forget every rx source on `link_id` (the link ended or was demoted; no
    /// `unwant` goes out) and pick new ones.
    fn drop_rx_of(&mut self, link_id: u64) {
        let logs: Vec<LogKey> = self
            .rx
            .iter()
            .filter(|(_, src)| src.link == link_id)
            .map(|(log, _)| *log)
            .collect();
        for log in &logs {
            self.rx.remove(log);
        }
        for log in logs {
            self.reselect_log(log);
        }
    }

    /// The link becomes the node's active one: a full `have` goes out as
    /// soon as `local` is fresh (`ask`: and the peer is asked for its own).
    fn activate(&mut self, link_id: u64, ask: bool) {
        let Some(entry) = self.links.get_mut(&link_id) else {
            return;
        };
        entry.mode = LinkMode::Active;
        entry.announced = false;
        entry.have_pending = Some(ask);
        self.request_local();
    }

    /// A `have` or `want` arrived on a standby link: the peer promoted it, so
    /// it is active here too and the formerly active link is demoted.
    fn promote_on_traffic(&mut self, link_id: u64) {
        let Some(node) = self.links.get(&link_id).map(|l| l.handle.peer().node) else {
            return;
        };
        let others: Vec<u64> = self
            .by_node
            .get(&node)
            .into_iter()
            .flatten()
            .copied()
            .filter(|id| *id != link_id)
            .collect();
        for id in others {
            if self
                .links
                .get(&id)
                .is_some_and(|l| l.mode == LinkMode::Active)
            {
                self.demote(id);
            }
        }
        self.activate(link_id, false);
    }

    fn on_link_event(&mut self, event: LinkEvent) {
        match event {
            LinkEvent::Ended {
                link_id,
                why,
                up_for,
            } => self.on_ended(link_id, why, up_for),
            LinkEvent::Frame { link_id, frame, .. } => self.on_frame(link_id, frame),
            LinkEvent::CreditFreed { .. } => {
                // Resuming a blocked push: ht-5fr.5.13.
            }
        }
    }

    fn on_frame(&mut self, link_id: u64, frame: PeerFrame) {
        let Some(entry) = self.links.get(&link_id) else {
            return;
        };
        if !entry.ranked || entry.closing {
            return;
        }
        if entry.mode == LinkMode::Standby {
            // A standby carries ping/pong (the link's own) and `seen`
            // (ht-5fr.5.6) only; a `have` or `want` is the peer's promotion.
            if matches!(frame, PeerFrame::Have { .. } | PeerFrame::Want { .. }) {
                self.promote_on_traffic(link_id);
            } else {
                return;
            }
        }
        // `want`, `unwant`, `entries`, the branch frames and `seen` are
        // served, ingested or recorded by ht-5fr.5.13 and ht-5fr.5.6.
        if let PeerFrame::Have {
            full, ask, logs, ..
        } = frame
        {
            self.on_have(link_id, full, ask, &logs);
        }
    }

    /// Merge a peer's `have` into its `peer_vector` and re-pick the rx source
    /// of every log it touched.
    fn on_have(&mut self, link_id: u64, full: bool, ask: bool, logs: &[HaveLog]) {
        let now = self.mono();
        let Some(entry) = self.links.get_mut(&link_id) else {
            return;
        };
        let before = std::mem::take(&mut entry.peer_vector);
        let mut vector = if full {
            Vector::default()
        } else {
            before.clone()
        };
        let mut changed: BTreeSet<LogKey> = BTreeSet::new();
        for log in logs {
            let Some(head) = have_head(log) else {
                continue;
            };
            let key = LogKey {
                node: log.node,
                incarnation: log.inc,
            };
            vector.0.insert(key, head);
            changed.insert(key);
        }
        if full {
            changed.extend(before.0.keys().copied());
        }
        entry.peer_vector = vector;
        if ask {
            entry.have_pending = Some(entry.have_pending.unwrap_or(false));
        }
        // News from the current source of a caught-up log restarts its
        // progress clock: it is not stalled, it had nothing to do.
        for log in &changed {
            let held = self.local.0.get(log).map_or(0, |h| h.counter);
            let was = before.0.get(log).map_or(0, |h| h.counter);
            let is = self
                .links
                .get(&link_id)
                .and_then(|l| l.peer_vector.0.get(log))
                .map_or(0, |h| h.counter);
            if was <= held
                && is > held
                && let Some(src) = self.rx.get_mut(log)
                && src.link == link_id
            {
                src.last_progress = now;
            }
        }
        for log in changed {
            self.reselect_log(log);
        }
        self.update_own_history();
        if ask {
            self.request_local();
        }
    }

    /// Read `journal.vector()` off the actor; the result arrives as a
    /// `refresh` job.
    fn request_local(&mut self) {
        if !self.refresh.is_empty() {
            self.refresh_again = true;
            return;
        }
        let journal = self.deps.journal.clone();
        self.refresh.spawn_blocking(move || {
            let budget = CallBudget {
                deadline: MonoInstant(u64::MAX),
                cancellation: Cancellation::default(),
            };
            journal
                .vector(&budget)
                .map_err(|e| format!("{:?}: {:?}", e.code, e.detail))
        });
    }

    /// A fresh local vector: answer the owed `have`s, re-pick every source
    /// and refresh the own-history flag.
    fn on_local(&mut self, result: Result<Vector, String>) {
        match result {
            Ok(vector) => {
                self.local = vector;
                let owed: Vec<(u64, bool)> = self
                    .links
                    .iter()
                    .filter(|(_, l)| l.mode == LinkMode::Active && !l.closing)
                    .filter_map(|(id, l)| l.have_pending.map(|ask| (*id, ask)))
                    .collect();
                for (id, ask) in owed {
                    self.send_have(id, ask);
                    if let Some(entry) = self.links.get_mut(&id) {
                        entry.have_pending = None;
                        entry.announced = true;
                    }
                }
            }
            Err(e) => (self.deps.log)(&format!("sync: local vector read failed: {e}")),
        }
        self.reselect_all();
        self.update_own_history();
        if std::mem::take(&mut self.refresh_again) {
            self.request_local();
        }
    }

    /// Send a full `have` of the local vector (the first
    /// [`MAX_HAVE_LOGS`] logs by key) on `link_id`.
    fn send_have(&self, link_id: u64, ask: bool) {
        let Some(entry) = self.links.get(&link_id) else {
            return;
        };
        let logs = self
            .local
            .0
            .iter()
            .take(MAX_HAVE_LOGS)
            .map(|(key, head)| HaveLog {
                node: key.node,
                inc: key.incarnation,
                counter: head.counter,
                head: hex_encode(&head.head_hash),
                frozen_at: match (head.state, head.frozen_at) {
                    (LogState::Frozen, Some(at)) => u64::try_from(at).ok(),
                    _ => None,
                },
            })
            .collect();
        let forks = if entry.handle.peer().caps.iter().any(|cap| cap == FORK_CAP) {
            self.deps.fork_sink.fork_refs()
        } else {
            Vec::new()
        };
        let _ = entry.handle.send(PeerFrame::Have {
            full: true,
            ask,
            logs,
            forks,
        });
    }

    fn send_unwant(&self, link_id: u64, log: &LogKey) {
        if let Some(entry) = self.links.get(&link_id) {
            let _ = entry.handle.send(PeerFrame::Unwant {
                node: log.node,
                inc: log.incarnation,
            });
        }
    }

    /// `Want` the log from `link_id` just above what is held, and record it
    /// as the rx source.
    fn start_rx(&mut self, log: LogKey, link_id: u64) {
        let from = self.local.0.get(&log).map_or(0, |h| h.counter) + 1;
        if let Some(entry) = self.links.get(&link_id) {
            let _ = entry.handle.send(PeerFrame::Want {
                node: log.node,
                inc: log.incarnation,
                from,
            });
        }
        let last_progress = self.mono();
        self.rx.insert(
            log,
            RxSource {
                link: link_id,
                requested_from: from,
                last_progress,
            },
        );
    }

    fn log_paused(&self, log: &LogKey, now: MonoInstant) -> bool {
        self.paused.contains(log) || self.headroom_until.is_some_and(|until| until.0 > now.0)
    }

    /// The pure `reselect` over the actor's current state for `log`.
    fn pick(&self, log: &LogKey) -> Option<u64> {
        let now = self.mono();
        let views: Vec<SourceView> = self
            .links
            .iter()
            .filter(|(_, l)| l.mode == LinkMode::Active && l.ranked && !l.closing && l.announced)
            .map(|(id, l)| SourceView {
                link: *id,
                node: l.handle.peer().node,
                adv: l.peer_vector.0.get(log).copied(),
                penalized: l.errors.get(log).is_some_and(|until| until.0 > now.0),
            })
            .collect();
        let current = self.rx.get(log).map(|src| routing::CurrentSource {
            link: src.link,
            last_progress: src.last_progress,
        });
        routing::reselect(
            log,
            self.local.0.get(log),
            &views,
            self.log_paused(log, now),
            current,
            now,
        )
    }

    fn reselect_log(&mut self, log: LogKey) {
        let pick = self.pick(&log);
        self.apply_pick(log, pick);
    }

    /// Apply a `reselect` answer: a different source gets the `Want` and the
    /// old one an `Unwant`; no candidate leaves a healthy source armed (live
    /// tail) and drops an unhealthy one.
    fn apply_pick(&mut self, log: LogKey, pick: Option<u64>) {
        let current = self.rx.get(&log).map(|src| src.link);
        match pick {
            Some(new) if Some(new) == current => {}
            Some(new) => {
                if let Some(old) = current {
                    self.send_unwant(old, &log);
                }
                self.start_rx(log, new);
            }
            None => {
                let Some(old) = current else {
                    return;
                };
                let now = self.mono();
                let healthy = self.links.get(&old).is_some_and(|l| {
                    l.mode == LinkMode::Active
                        && !l.closing
                        && l.errors.get(&log).is_none_or(|until| until.0 <= now.0)
                }) && !self.log_paused(&log, now);
                if !healthy {
                    self.send_unwant(old, &log);
                    self.rx.remove(&log);
                }
            }
        }
    }

    fn reselect_all(&mut self) {
        let mut logs: BTreeSet<LogKey> = self.rx.keys().copied().collect();
        for entry in self.links.values() {
            if entry.mode == LinkMode::Active {
                logs.extend(entry.peer_vector.0.keys().copied());
            }
        }
        for log in logs {
            self.reselect_log(log);
        }
    }

    /// Whether `link_id` still advertises more of `log` than is held.
    fn advertises_more(&self, link_id: u64, log: &LogKey) -> bool {
        self.links
            .get(&link_id)
            .and_then(|l| l.peer_vector.0.get(log))
            .is_some_and(|adv| routing::wantable(self.local.0.get(log), adv, false))
    }

    /// The 30 s failover: an rx source that has not progressed while it still
    /// advertises more is penalized for the log, and the log re-picked; when
    /// it is the only candidate it is asked again from `contiguous + 1`
    /// (`Want` is idempotent).
    fn failover(&mut self, now: MonoInstant) {
        let stalled: Vec<(LogKey, u64)> = self
            .rx
            .iter()
            .filter(|(log, src)| {
                now.0.saturating_sub(src.last_progress.0) >= routing::RX_STALL_MS
                    && self.advertises_more(src.link, log)
            })
            .map(|(log, src)| (*log, src.link))
            .collect();
        for (log, link_id) in stalled {
            if let Some(entry) = self.links.get_mut(&link_id) {
                entry
                    .errors
                    .insert(log, MonoInstant(now.0 + routing::STALL_PENALTY_MS));
            }
            match self.pick(&log) {
                Some(new) => self.apply_pick(log, Some(new)),
                None => self.start_rx(log, link_id),
            }
        }
    }

    /// Lift expired penalties and re-pick the logs they covered.
    fn expire_penalties(&mut self, now: MonoInstant) {
        let mut lifted: BTreeSet<LogKey> = BTreeSet::new();
        for entry in self.links.values_mut() {
            entry.errors.retain(|log, until| {
                let live = until.0 > now.0;
                if !live {
                    lifted.insert(*log);
                }
                live
            });
        }
        for log in lifted {
            self.reselect_log(log);
        }
    }

    fn housekeeping(&mut self) {
        let now = self.mono();
        self.resolve_probes(now);
        self.expire_penalties(now);
        self.failover(now);
        if self.refresh.is_empty() && self.links.values().any(|l| l.have_pending.is_some()) {
            // A read that failed leaves `have`s owed: try again.
            self.request_local();
        }
    }

    /// Set the own-history `Links` flag from the active links' vectors and
    /// the local one (the pure [`routing::own_history_on`]).
    fn update_own_history(&mut self) {
        let on = routing::own_history_on(
            self.deps.identity.uuid(),
            &self.local,
            self.links
                .values()
                .filter(|l| l.mode == LinkMode::Active && l.ranked && !l.closing)
                .map(|l| &l.peer_vector),
        );
        if on != self.own_history_on {
            self.own_history_on = on;
            self.deps
                .own_history
                .set(HoldSource::Links, on, self.mono());
        }
    }

    fn on_ended(&mut self, link_id: u64, why: LinkError, up_for: Duration) {
        let Some(entry) = self.links.remove(&link_id) else {
            return;
        };
        let node = entry.handle.peer().node;
        self.probes.remove(&link_id);
        let was_ranked = self.by_node.get_mut(&node).is_some_and(|ids| {
            let before = ids.len();
            ids.retain(|id| *id != link_id);
            ids.len() != before
        });
        if self.by_node.get(&node).is_some_and(Vec::is_empty) {
            self.by_node.remove(&node);
        }
        self.count.fetch_sub(1, Ordering::AcqRel);
        if matches!(why, LinkError::PeerClosed(CloseReason::NodeCollision)) {
            self.report_clone(CloneKind::PeerClose, Some(node));
        }
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
        // Probes waiting on this link: it is gone, so the newer link ranks.
        let waiting: Vec<u64> = self
            .probes
            .iter()
            .filter(|(_, p)| p.older == link_id)
            .map(|(newer, _)| *newer)
            .collect();
        for newer in waiting {
            self.probes.remove(&newer);
            self.rank_link(newer);
        }
        if was_ranked {
            self.drop_rx_of(link_id);
            self.rank_node(node, true);
        }
        self.update_own_history();
    }

    fn on_request(&self, request: Request) {
        match request {
            Request::Snapshot(reply) => {
                let links = self
                    .links
                    .iter()
                    .take(SNAPSHOT_MAX_LINKS)
                    .map(|(id, entry)| {
                        let peer: &PeerInfo = entry.handle.peer();
                        LinkStatus {
                            node: peer.node,
                            tag: peer.tag,
                            role: peer.role,
                            mode: entry.mode,
                            dest: match &entry.origin {
                                Origin::Inbound => None,
                                Origin::Dialed { dest, .. } => Some(dest.clone()),
                            },
                            since: entry.since,
                            rx_logs: u32::try_from(
                                self.rx.values().filter(|src| src.link == *id).count(),
                            )
                            .unwrap_or(u32::MAX),
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

/// A peer's advertised head from one `have` row; `None` for a malformed hash.
fn have_head(log: &HaveLog) -> Option<LogHead> {
    let frozen_at = log
        .frozen_at
        .map(|at| i64::try_from(at).unwrap_or(i64::MAX));
    Some(LogHead {
        counter: log.counter,
        head_hash: hex_decode::<32>(&log.head)?,
        state: if frozen_at.is_some() {
            LogState::Frozen
        } else {
            LogState::Active
        },
        frozen_at,
    })
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
    use std::sync::atomic::{AtomicBool, AtomicU64};
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

    /// The log-only sink, recording every clone report too.
    struct RecordingSink {
        inner: LogOnlyForkSink,
        clones: Mutex<Vec<CloneReport>>,
    }

    impl ForkSink for RecordingSink {
        fn on_fork(&self, log: LogKey, evidence: ForkEvidence, source: NodeUuid) -> ForkAck {
            self.inner.on_fork(log, evidence, source)
        }
        fn resumed(&self) -> Vec<LogKey> {
            self.inner.resumed()
        }
        fn on_clone(&self, report: CloneReport) {
            self.inner.on_clone(report.clone());
            self.clones.lock().unwrap().push(report);
        }
    }

    struct Rig {
        handle: SyncHandle,
        intake: PeerIntake,
        join: JoinHandle<()>,
        cancel: Cancellation,
        clock: Arc<dyn Clock>,
        pacer: Arc<Pacer>,
        own_history: Arc<OwnHistoryHold>,
        sink: Arc<RecordingSink>,
        lines: Lines,
    }

    impl Rig {
        fn clones(&self) -> Vec<CloneReport> {
            self.sink.clones.lock().unwrap().clone()
        }
    }

    fn start_with(journal: Arc<dyn JournalPort>) -> Rig {
        let clock: Arc<dyn Clock> = Arc::new(TokioClock(Instant::now()));
        let cancel = Cancellation::default();
        let (log, lines) = capture();
        let pacer = Arc::new(Pacer::new("sync", clock.clone(), cancel.clone()));
        let own_history = Arc::new(OwnHistoryHold::new(Arc::new(AtomicBool::new(false))));
        let sink = Arc::new(RecordingSink {
            inner: LogOnlyForkSink::new(log.clone(), clock.clone()),
            clones: Mutex::new(Vec::new()),
        });
        let (handle, intake, join) = SyncCoordinator::start(CoordinatorDeps {
            journal,
            sync_store: Arc::new(MemorySyncStore::new()),
            identity: identity(1),
            boot: Uuid::from_u128(1),
            writer: Arc::new(FairWriter::new(8)),
            clock: clock.clone(),
            pacer: pacer.clone(),
            status: Arc::new(WorkerStatus::default()),
            log,
            fork_sink: sink.clone(),
            own_history: own_history.clone(),
            dial: DialConfig::default(),
            cancel: cancel.clone(),
        });
        Rig {
            handle,
            intake,
            join,
            cancel,
            clock,
            pacer,
            own_history,
            sink,
            lines,
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

    // -----------------------------------------------------------------------
    // Routing: duplicate links, Have/Want, failover, clones, own history
    // -----------------------------------------------------------------------

    use tokio::io::WriteHalf;

    fn hello_boot(n: u8, boot: u128) -> LocalHello {
        LocalHello::new(identity(n), Uuid::from_u128(boot))
    }

    fn node_of(n: u8) -> NodeUuid {
        identity(n).uuid()
    }

    fn have_log(node: NodeUuid, inc: u32, counter: u64) -> HaveLog {
        HaveLog {
            node,
            inc,
            counter,
            head: "00".repeat(32),
            frozen_at: None,
        }
    }

    /// The test-side end of a link: sends frames, records what the
    /// coordinator wrote. `idx` is the connect order (the actor's link id order).
    struct TestPeer {
        idx: usize,
        info: PeerInfo,
        writer: WriteHalf<DuplexStream>,
        frames: mpsc::UnboundedReceiver<PeerFrame>,
        reader: tokio::task::JoinHandle<()>,
    }

    fn test_peer(idx: usize, est: Established<DuplexStream>) -> TestPeer {
        let Established {
            peer,
            mut reader,
            writer,
        } = est;
        let (tx, frames) = mpsc::unbounded_channel();
        let reader = tokio::spawn(async move {
            while let Ok(frame) = read_peer_frame(&mut reader, MAX_FRAME_BYTES).await {
                if tx.send(frame).is_err() {
                    break;
                }
            }
        });
        TestPeer {
            idx,
            info: peer,
            writer,
            frames,
            reader,
        }
    }

    impl TestPeer {
        async fn send(&mut self, frame: PeerFrame) {
            write_peer_frame(&mut self.writer, &frame).await.unwrap();
            self.writer.flush().await.unwrap();
        }

        async fn send_have(&mut self, full: bool, ask: bool, logs: Vec<HaveLog>) {
            self.send(PeerFrame::Have {
                full,
                ask,
                logs,
                forks: Vec::new(),
            })
            .await;
        }

        /// The next frame, pings included.
        async fn next_raw(&mut self) -> PeerFrame {
            match timeout(Duration::from_secs(5), self.frames.recv()).await {
                Ok(Some(frame)) => frame,
                Ok(None) => panic!("link ended while a frame was expected"),
                Err(_) => panic!("no frame within 5 s"),
            }
        }

        /// The next frame that is not a link-level `ping`.
        async fn next(&mut self) -> PeerFrame {
            loop {
                match self.next_raw().await {
                    PeerFrame::Ping { .. } => continue,
                    frame => return frame,
                }
            }
        }

        async fn expect_have(&mut self) -> (bool, bool, Vec<HaveLog>) {
            match self.next().await {
                PeerFrame::Have {
                    full, ask, logs, ..
                } => (full, ask, logs),
                other => panic!("expected have, got {other:?}"),
            }
        }

        async fn expect_want(&mut self, node: NodeUuid, inc: u32, from: u64) {
            match self.next().await {
                PeerFrame::Want {
                    node: n,
                    inc: i,
                    from: f,
                } => assert_eq!((n, i, f), (node, inc, from)),
                other => panic!("expected want, got {other:?}"),
            }
        }

        async fn expect_unwant(&mut self, node: NodeUuid, inc: u32) {
            match self.next().await {
                PeerFrame::Unwant { node: n, inc: i } => assert_eq!((n, i), (node, inc)),
                other => panic!("expected unwant, got {other:?}"),
            }
        }

        async fn expect_close(&mut self) -> CloseReason {
            match self.next().await {
                PeerFrame::Close { reason, .. } => reason,
                other => panic!("expected close, got {other:?}"),
            }
        }

        /// Nothing but `ping`/`pong` arrives within a short while.
        async fn quiet(&mut self) {
            tokio::time::sleep(Duration::from_millis(200)).await;
            while let Ok(frame) = self.frames.try_recv() {
                assert!(
                    matches!(frame, PeerFrame::Ping { .. } | PeerFrame::Pong { .. }),
                    "link {} should be quiet, got {frame:?}",
                    self.idx
                );
            }
        }

        /// Drop the link from the remote side.
        fn end(self) {
            self.reader.abort();
            drop(self.writer);
        }
    }

    async fn connect(rig: &Rig, idx: usize, hello: &LocalHello) -> TestPeer {
        test_peer(idx, connect_inbound(rig, hello).await)
    }

    async fn snapshot(rig: &Rig) -> SyncSnapshot {
        let budget = CallBudget {
            deadline: MonoInstant(rig.clock.monotonic_now().0 + 5_000),
            cancellation: Cancellation::default(),
        };
        rig.handle.snapshot(&budget).await.unwrap()
    }

    /// Connect-order indexes of `peers` in the duplicate order, best first.
    fn expected_order(peers: &[&TestPeer]) -> Vec<usize> {
        let keys: Vec<DupKey> = peers
            .iter()
            .map(|p| DupKey {
                link: p.idx as u64,
                dialer: p.info.dialer_node,
                nonce: p.info.dialer_nonce,
            })
            .collect();
        routing::duplicate_order(&keys)
            .into_iter()
            .map(|id| id as usize)
            .collect()
    }

    fn rig_with_self_log() -> (Rig, Arc<MemoryJournal>) {
        let journal = Arc::new(MemoryJournal::new(identity(1)));
        journal.bootstrap();
        (start_with(journal.clone()), journal)
    }

    /// Two inbound links from peer 2 with the same boot: returns
    /// `(active, standby)` with the activation `have`s already consumed.
    async fn two_links(rig: &Rig) -> (TestPeer, TestPeer) {
        let mut first = connect(rig, 0, &peer_hello(2)).await;
        first.expect_have().await;
        let mut second = connect(rig, 1, &peer_hello(2)).await;
        listed(rig, 2).await;
        let order = expected_order(&[&first, &second]);
        if order[0] == 0 {
            second.quiet().await;
            (first, second)
        } else {
            let (full, ask, _) = second.expect_have().await;
            assert!(full && ask, "the new minimal link asks for the peer's have");
            first.quiet().await;
            (second, first)
        }
    }

    #[tokio::test(start_paused = true)]
    async fn activation_sends_full_have() {
        let (rig, journal) = rig_with_self_log();
        let mut peer = connect(&rig, 0, &peer_hello(2)).await;
        let (full, ask, logs) = peer.expect_have().await;
        assert!(full && !ask);
        let own = journal.self_log();
        assert_eq!(logs.len(), 1);
        assert_eq!(
            (logs[0].node, logs[0].inc, logs[0].counter),
            (own.node, own.incarnation, 1)
        );
        assert_eq!(logs[0].head.len(), 64);
        rig.cancel.cancel();
        rig.join.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn ask_answered_with_full_have() {
        let (rig, _journal) = rig_with_self_log();
        let mut peer = connect(&rig, 0, &peer_hello(2)).await;
        peer.expect_have().await;
        peer.quiet().await;
        peer.send_have(true, true, Vec::new()).await;
        let (full, ask, logs) = peer.expect_have().await;
        assert!(full && !ask, "the reply is full and does not ask back");
        assert_eq!(logs.len(), 1);
        rig.cancel.cancel();
        rig.join.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn rx_prefers_origin() {
        let (rig, _journal) = rig_with_self_log();
        let mut b = connect(&rig, 0, &peer_hello(2)).await;
        b.expect_have().await;
        let mut c = connect(&rig, 1, &peer_hello(3)).await;
        c.expect_have().await;
        let origin = node_of(3);

        // B advertises C's log with a high counter and is wanted first.
        b.send_have(true, false, vec![have_log(origin, 1, 50)])
            .await;
        b.expect_want(origin, 1, 1).await;
        c.quiet().await;

        // C, the origin, advertises less and still takes over.
        c.send_have(true, false, vec![have_log(origin, 1, 10)])
            .await;
        b.expect_unwant(origin, 1).await;
        c.expect_want(origin, 1, 1).await;
        let snap = snapshot(&rig).await;
        assert_eq!(snap.links[0].rx_logs, 0);
        assert_eq!(snap.links[1].rx_logs, 1);
        rig.cancel.cancel();
        rig.join.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn failover_after_30s_unwant_old_want_new() {
        let (rig, _journal) = rig_with_self_log();
        let mut b = connect(&rig, 0, &peer_hello(2)).await;
        b.expect_have().await;
        let mut c = connect(&rig, 1, &peer_hello(3)).await;
        c.expect_have().await;
        let log_node = node_of(9);
        b.send_have(true, false, vec![have_log(log_node, 1, 10)])
            .await;
        c.send_have(true, false, vec![have_log(log_node, 1, 8)])
            .await;
        b.expect_want(log_node, 1, 1).await;
        c.quiet().await;

        tokio::time::sleep(Duration::from_secs(28)).await;
        b.quiet().await;
        c.quiet().await;

        tokio::time::sleep(Duration::from_secs(3)).await;
        b.expect_unwant(log_node, 1).await;
        c.expect_want(log_node, 1, 1).await;
        rig.cancel.cancel();
        rig.join.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn failover_with_one_candidate_asks_again() {
        let (rig, _journal) = rig_with_self_log();
        let mut b = connect(&rig, 0, &peer_hello(2)).await;
        b.expect_have().await;
        let log_node = node_of(9);
        b.send_have(true, false, vec![have_log(log_node, 1, 10)])
            .await;
        b.expect_want(log_node, 1, 1).await;
        tokio::time::sleep(Duration::from_secs(31)).await;
        b.expect_want(log_node, 1, 1).await;
        rig.cancel.cancel();
        rig.join.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn two_links_only_minimal_carries_have_want() {
        let (rig, _journal) = rig_with_self_log();
        let (mut active, mut standby) = two_links(&rig).await;
        let log_node = node_of(9);
        active
            .send_have(true, false, vec![have_log(log_node, 1, 5)])
            .await;
        active.expect_want(log_node, 1, 1).await;
        standby.quiet().await;
        let snap = snapshot(&rig).await;
        let modes: Vec<LinkMode> = snap.links.iter().map(|l| l.mode).collect();
        let mut expected = [LinkMode::Standby, LinkMode::Standby];
        expected[active.idx] = LinkMode::Active;
        assert_eq!(modes, expected);
        rig.cancel.cancel();
        rig.join.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn ending_active_promotes_standby_without_handshake() {
        let (rig, _journal) = rig_with_self_log();
        let (active, mut standby) = two_links(&rig).await;
        active.end();
        let (full, ask, _) = standby.expect_have().await;
        assert!(full && ask, "promotion sends a full have that asks");
        until(|| rig.handle.link_count() == 1).await;
        let snap = snapshot(&rig).await;
        assert_eq!(snap.links.len(), 1);
        assert_eq!(snap.links[0].mode, LinkMode::Active);
        rig.cancel.cancel();
        rig.join.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn have_on_standby_promotes_it() {
        let (rig, _journal) = rig_with_self_log();
        let (mut active, mut standby) = two_links(&rig).await;
        let log_node = node_of(9);
        standby
            .send_have(true, false, vec![have_log(log_node, 1, 5)])
            .await;
        let (full, ask, _) = standby.expect_have().await;
        assert!(full && !ask);
        standby.expect_want(log_node, 1, 1).await;
        active.quiet().await;
        let snap = snapshot(&rig).await;
        let modes: Vec<LinkMode> = snap.links.iter().map(|l| l.mode).collect();
        let mut expected = [LinkMode::Standby, LinkMode::Standby];
        expected[standby.idx] = LinkMode::Active;
        assert_eq!(
            modes, expected,
            "the standby is active, the former active demoted"
        );
        rig.cancel.cancel();
        rig.join.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn third_link_closed_busy() {
        let (rig, _journal) = rig_with_self_log();
        let mut peers = Vec::new();
        for idx in 0..3 {
            peers.push(connect(&rig, idx, &peer_hello(2)).await);
            if idx < 2 {
                listed(&rig, idx + 1).await;
            }
        }
        let refs: Vec<&TestPeer> = peers.iter().collect();
        let order = expected_order(&refs);
        let newest_standby = order[1..].iter().copied().max().unwrap();
        for peer in &mut peers {
            if peer.idx == newest_standby {
                loop {
                    match peer.next().await {
                        PeerFrame::Close { reason, .. } => {
                            assert_eq!(reason, CloseReason::Busy);
                            break;
                        }
                        PeerFrame::Have { .. } => continue,
                        other => panic!("unexpected {other:?}"),
                    }
                }
            } else {
                while let Ok(frame) = peer.frames.try_recv() {
                    assert!(
                        !matches!(frame, PeerFrame::Close { .. }),
                        "only the newest standby is closed"
                    );
                }
            }
        }
        until(|| rig.handle.link_count() == 2).await;
        rig.cancel.cancel();
        rig.join.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn same_boot_two_links_is_duplicate() {
        let (rig, _journal) = rig_with_self_log();
        let (_active, _standby) = two_links(&rig).await;
        assert_eq!(rig.handle.link_count(), 2);
        assert!(rig.clones().is_empty(), "one boot is no clone");
        let snap = snapshot(&rig).await;
        let mut modes: Vec<LinkMode> = snap.links.iter().map(|l| l.mode).collect();
        modes.sort_by_key(|m| *m == LinkMode::Standby);
        assert_eq!(modes, [LinkMode::Active, LinkMode::Standby]);
        rig.cancel.cancel();
        rig.join.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn third_party_clone_newer_closed_and_reported() {
        let (rig, _journal) = rig_with_self_log();
        let mut older = connect(&rig, 0, &hello_boot(2, 21)).await;
        older.expect_have().await;

        let mut newer = connect(&rig, 1, &hello_boot(2, 22)).await;
        // The older link is probed with a ping; it answers.
        match older.next_raw().await {
            PeerFrame::Ping { n } => older.send(PeerFrame::Pong { n }).await,
            other => panic!("expected the probe ping, got {other:?}"),
        }
        assert_eq!(newer.expect_close().await, CloseReason::NodeCollision);
        until(|| !rig.clones().is_empty()).await;
        assert_eq!(
            rig.clones(),
            vec![CloneReport {
                subject: node_of(2),
                kind: CloneKind::ThirdParty,
                boots: vec![Uuid::from_u128(21), Uuid::from_u128(22)],
                source: node_of(1),
            }]
        );
        assert!(
            rig.lines.lock().unwrap().iter().any(|l| l
                == &format!(
                    "sync: clone suspected for {} (third_party)",
                    uuid8(&node_of(2))
                )),
            "{:?}",
            rig.lines.lock().unwrap()
        );
        until(|| rig.handle.link_count() == 1).await;
        let snap = snapshot(&rig).await;
        assert_eq!(snap.links.len(), 1);
        assert_eq!(
            snap.links[0].mode,
            LinkMode::Active,
            "the older link is kept"
        );
        rig.cancel.cancel();
        rig.join.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn silent_older_link_dead_newer_ranked_no_report() {
        let (rig, _journal) = rig_with_self_log();
        let mut older = connect(&rig, 0, &hello_boot(2, 21)).await;
        older.expect_have().await;

        let mut newer = connect(&rig, 1, &hello_boot(2, 22)).await;
        // The older link never answers its probe ping.
        tokio::time::sleep(Duration::from_secs(11)).await;
        let (full, ask, _) = newer.expect_have().await;
        assert!(full && !ask, "the newer link is ranked as if alone");
        assert!(rig.clones().is_empty(), "a restarted peer is not a clone");
        until(|| rig.handle.link_count() == 1).await;
        let snap = snapshot(&rig).await;
        assert_eq!(snap.links.len(), 1);
        assert_eq!(snap.links[0].mode, LinkMode::Active);
        // The older link's stream is closed (dropping its handle writes
        // `close{shutdown}`).
        loop {
            match timeout(Duration::from_secs(5), older.frames.recv()).await {
                Ok(None) => break,
                Ok(Some(PeerFrame::Ping { .. } | PeerFrame::Close { .. })) => continue,
                other => panic!("expected the older link to end, got {other:?}"),
            }
        }
        rig.cancel.cancel();
        rig.join.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn handshake_node_collision_reports_self() {
        let (rig, _journal) = rig_with_self_log();
        // Another daemon with this node's key dials in: the handshake is refused.
        let (a, mut b) = duplex(BIG);
        let clone_hello = hello_boot(1, 99);
        let dialer = crate::sync::link::handshake(a, &clone_hello, Role::Dialer, None, deadline());
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
        let (result, ()) = tokio::join!(dialer, offer);
        assert!(matches!(
            result,
            Err(LinkError::PeerClosed(CloseReason::NodeCollision))
        ));
        until(|| !rig.clones().is_empty()).await;
        let me = node_of(1);
        assert_eq!(
            rig.clones(),
            vec![CloneReport {
                subject: me,
                kind: CloneKind::Handshake,
                boots: Vec::new(),
                source: me,
            }]
        );

        // The same boot is a self link, not a clone.
        let (a, mut b) = duplex(BIG);
        let self_hello = hello_boot(1, 1);
        let dialer = crate::sync::link::handshake(a, &self_hello, Role::Dialer, None, deadline());
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
        let (result, ()) = tokio::join!(dialer, offer);
        assert!(matches!(
            result,
            Err(LinkError::PeerClosed(CloseReason::SelfLink))
        ));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(rig.clones().len(), 1, "a self link is no clone report");

        // An established peer link closed with node_collision reports too.
        let mut peer = connect(&rig, 0, &peer_hello(2)).await;
        peer.expect_have().await;
        peer.send(PeerFrame::Close {
            reason: CloseReason::NodeCollision,
            detail: None,
        })
        .await;
        until(|| rig.clones().len() == 2).await;
        assert_eq!(
            rig.clones()[1],
            CloneReport {
                subject: me,
                kind: CloneKind::PeerClose,
                boots: Vec::new(),
                source: node_of(2),
            }
        );
        let hits = rig
            .lines
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.starts_with("sync: clone suspected for"))
            .count();
        assert_eq!(hits, 1, "one log line per subject per hour");
        rig.cancel.cancel();
        rig.join.await.unwrap();
    }

    fn hold_on(rig: &Rig) -> bool {
        rig.own_history.active(rig.clock.monotonic_now())
    }

    #[tokio::test(start_paused = true)]
    async fn peer_advertising_own_incarnation_sets_links() {
        let (rig, _journal) = rig_with_self_log();
        let mut peer = connect(&rig, 0, &peer_hello(2)).await;
        peer.expect_have().await;
        assert!(!hold_on(&rig));
        peer.send_have(true, false, vec![have_log(node_of(1), 7, 5)])
            .await;
        until(|| hold_on(&rig)).await;
        rig.cancel.cancel();
        rig.join.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn ingest_or_link_end_clears_links() {
        let (rig, journal) = rig_with_self_log();
        let own = journal.self_log();
        let mut peer = connect(&rig, 0, &peer_hello(2)).await;
        peer.expect_have().await;

        // The peer holds the own log up to counter 4; local holds 1.
        peer.send_have(true, false, vec![have_log(own.node, own.incarnation, 4)])
            .await;
        until(|| hold_on(&rig)).await;
        // Local catches up (ingest up to the advertised counter).
        for _ in 0..3 {
            journal.author_raw("test.kind", b"{}".to_vec()).unwrap();
        }
        rig.pacer.kick();
        until(|| !hold_on(&rig)).await;

        // Another incarnation beyond local sets it again; the link ending
        // clears it.
        peer.send_have(true, false, vec![have_log(own.node, 9, 3)])
            .await;
        until(|| hold_on(&rig)).await;
        peer.end();
        until(|| !hold_on(&rig)).await;
        rig.cancel.cancel();
        rig.join.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn fresh_node_never_sets_links() {
        let rig = start();
        let mut peer = connect(&rig, 0, &peer_hello(2)).await;
        // The peer advertises its own log and a third node's, nothing of ours.
        peer.send_have(
            true,
            false,
            vec![have_log(node_of(2), 1, 9), have_log(node_of(5), 1, 3)],
        )
        .await;
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert!(!hold_on(&rig));
        rig.cancel.cancel();
        rig.join.await.unwrap();
    }
}
