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
//! - Serving and ingest (ht-5fr.5.13): `serve` answers `want` from committed
//!   rows (halved to the link's `max_frame`), re-pushes armed subscriptions
//!   on `CreditFreed` and on a local vector change, and sends throttled
//!   `have` deltas; `ingest` drops unsolicited `entries`, runs the one
//!   sequential ingest worker and acts on every `IngestOutcome`. New
//!   outcomes are handled there; fork policy goes in a [`ForkSink`]
//!   implementation. The Dialer
//!   (`super::dialer`, ht-5fr.5.7) is constructed here: [`SyncHandle::reload_targets`]
//!   and [`SyncHandle::enabled_target_count`] reach it, a pacer kick re-diffs
//!   its targets, and shutdown cancels its tasks.
//! - Fork wiring (ht-5fr.6.8): `fork.v1` is advertised in the local caps; the
//!   coordinator only moves fork frames. It hands every `have` (logs and fork
//!   refs) to [`ForkSink::on_have`], serves `branch_want` from the journal
//!   (no subscription, at most 4 outstanding per link, `branch_none` for the
//!   rest), routes `branch_entries`/`branch_none` to [`ForkSink::on_branch`]
//!   after a shape check, and on the 1 s tick drains [`ForkSink::outbox`] to
//!   the `fork.v1` links. What a fork means, and which branches to ask for,
//!   is the sink's policy, never the coordinator's.
//! - Reachability and rounds (ht-5fr.5.6): the actor owns the `ContactBook`
//!   (direct contact from every inbound frame, `seen` every 10 s per active
//!   link, a flush through `SyncStorePort::record_contacts` on the 1 s tick and
//!   a final one at shutdown) and writes each active link's round to
//!   [`SyncSignals`], which [`SyncHandle::signals`] hands out. The
//!   completion rule lives in `Actor::update_rounds`.
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
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::{Instant, MissedTickBehavior, timeout};
use uuid::Uuid;

use super::dialer::{DialConfig, DialShared, Dialer, DialerDeps};
use super::envelope::{GENESIS_KIND, StoredEntry, hex_decode, hex_encode};
use super::facts::GenesisPayload;
use super::frame::{
    BranchWant, CloseReason, FORK_CAP, ForkRef, HaveLog, MAX_HAVE_FORKS, MAX_HAVE_LOGS,
    MAX_NODE_VERSION_BYTES, NODE_INFO_CAP, NodeInfo, OPAQUE_CAP, PeerFrame, Role, SEAT_STATUS_CAP,
    SeatStatusEntry, SeatStatusFrame, SeenAge, WireItem, write_peer_frame,
};
use super::journal_port::{ForkEvidence, IngestOutcome, JournalPort};
use super::link::{
    CLOSE_WRITE_TIMEOUT, Established, HANDSHAKE_TIMEOUT, LinkError, LinkEvent, LinkHandle,
    LocalHello, PeerInfo, SendError, handshake,
};
use super::signals::{ContactBook, LinkRound, SyncSignals};
use super::sync_store::SyncStorePort;
use super::{BatchLimits, LogHead, LogKey, LogState, Vector};
use crate::node::{NodeIdentity, NodeTag, NodeUuid};
use crate::protocol::results::{ApiError, ErrorCode};
use crate::protocol::time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis};
use crate::service::fair_writer::FairWriter;
use crate::service::pacer::{Pacer, Wake};
use crate::service::workers::WorkerStatus;
use crate::store::connection::{HoldSource, OwnHistoryHold};
use crate::store::sync_contacts::NodeContact;
use crate::store::sync_dial_targets::TargetId;
use crate::store::sync_peer_status::PeerStatus;

mod ingest;
mod peer_status;
mod routing;
mod seat_status;
mod serve;

use ingest::{IngestDone, IngestJob};
pub use routing::ResyncPin;
use routing::{DupKey, SourceView};
use serve::{BranchDone, ServeDone};
pub use serve::{GateCut, gate_batch};

/// Offers the transport may have waiting for the actor.
pub const PEER_INTAKE_SLOTS: usize = 16;
/// Dialed streams the Dialer may have waiting for the actor.
const DIALED_SLOTS: usize = 16;
/// Link events the actor may have waiting.
const LINK_EVENT_SLOTS: usize = 256;
/// Tokens a link's control-frame bucket holds: a full `want` per wantable log
/// (`MAX_HAVE_LOGS`) plus a little slack.
const CONTROL_BUCKET: u64 = MAX_HAVE_LOGS as u64 + 64;
/// Tokens the bucket regains per second.
const CONTROL_REFILL_PER_S: u64 = 1_024;
/// Control frames dropped on one link before it is closed `ProtocolError`.
const MAX_THROTTLED: u64 = 10_000;
/// Requests (snapshots) the actor may have waiting.
const REQUEST_SLOTS: usize = 16;
/// Most links a snapshot lists.
pub const SNAPSHOT_MAX_LINKS: usize = 64;
/// Most logs a snapshot lists an advertised counter for.
pub const SNAPSHOT_MAX_ADVERTISED: usize = 1024;
/// Capacity of a dialed link's report channel: `Established` and `LinkEnded`.
pub const DIAL_REPORT_SLOTS: usize = 2;
/// How long the actor waits on its pacer before looping.
const PACER_TICK: Duration = Duration::from_secs(1);
/// Most unheld own-log claims tracked per link; claims beyond never count.
const MAX_OWN_CLAIMS: usize = 64;
/// The housekeeping interval.
const HOUSEKEEPING_TICK: Duration = Duration::from_secs(1);
/// Each active link gets a `seen` this often.
const SEEN_EVERY_MS: u64 = 10_000;
/// How long the in-flight `entries` of a source a log was just moved away
/// from are dropped without counting as unsolicited.
const REPLACED_GRACE_MS: u64 = 5_000;
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
        entries: Vec<WireItem>,
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
    /// Set by a `sync resync` restart: the pinned logs are pulled only from
    /// the named peer until a round with it completes or ten minutes pass.
    pub resync_pin: Option<ResyncPin>,
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
    /// `entries` frames dropped because this link was not the log's rx
    /// source (their credit was granted back).
    pub unsolicited: u64,
    /// Control frames (`want`, `unwant`, `have`, `branch_want`) dropped
    /// because the link's token bucket was empty.
    pub throttled: u64,
    /// Logs the peer has armed a subscription for.
    pub subscriptions: u32,
    /// Logs parked on this link: the peer lacks [`OPAQUE_CAP`] and the next
    /// entry it wants of the log is opaque (not a count of entries).
    pub withheld_logs: u64,
    pub last_error: Option<String>,
    /// The last inbound frame of any kind, ping/pong included.
    pub last_rx: UtcMillis,
    /// The last frame the peer's coordinator sent (not the link's own
    /// ping/pong/credit): application-level contact; `None` before the first.
    pub app_rx: Option<UtcMillis>,
    /// The peer's self-reported daemon version (`node_info`); display only.
    pub peer_version: Option<String>,
    /// `(acked, head)`: the peer holds `acked` of our active log (from its
    /// `have`); `None` off an active link or before its first full `have`.
    pub tx: Option<(u64, u64)>,
    /// `(have, head)`: we hold `have` of the peer's active log, which it
    /// advertises at `head`.
    pub rx: Option<(u64, u64)>,
    pub stalled: Stall,
}

impl Stall {
    /// `app`, `tx`, `rx`: the parts that stalled, in that order.
    pub fn names(self) -> Vec<&'static str> {
        [("app", self.app), ("tx", self.tx), ("rx", self.rx)]
            .into_iter()
            .filter_map(|(name, on)| on.then_some(name))
            .collect()
    }

    pub fn any(self) -> bool {
        self.app || self.tx || self.rx
    }
}

/// Which parts of an active link stopped making progress (diagnostic only;
/// no link, rank or dial decision reads it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stall {
    /// No frame from the peer's coordinator for `APP_SILENT_AFTER_MS`,
    /// although the link itself is alive.
    pub app: bool,
    /// Our active log is ahead of what the peer acknowledges and its
    /// acknowledgement has not advanced for `STALL_AFTER_MS`.
    pub tx: bool,
    /// The peer advertises its active log ahead of what we hold and we have
    /// not ingested any of it for `STALL_AFTER_MS`.
    pub rx: bool,
}

/// The coordinator's state for `sync status`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncSnapshot {
    /// At most [`SNAPSHOT_MAX_LINKS`], by link id.
    pub links: Vec<LinkStatus>,
    /// Logs the fork sink holds (no `want` for them).
    pub paused_logs: Vec<LogKey>,
    /// Every want is paused until then (no disk headroom).
    pub headroom_until: Option<UtcMillis>,
    /// The highest counter any active link advertises per log (at most
    /// [`SNAPSHOT_MAX_ADVERTISED`] logs): `sync status` shows the lag behind
    /// it.
    pub advertised: BTreeMap<LogKey, u64>,
}

enum Request {
    Snapshot(oneshot::Sender<SyncSnapshot>),
    /// This node's own seats, to publish on `seat_status` (ht-xg88).
    PublishSeats(Vec<SeatStatusEntry>),
}

/// The cloneable facade over the coordinator actor.
///
/// Responsibility: see the module documentation (SyncCoordinator / SyncHandle).
/// It owns only channel ends and one atomic; it never touches actor state.
#[derive(Clone)]
pub struct SyncHandle {
    links: Arc<AtomicUsize>,
    /// Some active link negotiated [`SEAT_STATUS_CAP`] (written by the actor).
    seat_status_wanted: Arc<AtomicBool>,
    requests: mpsc::Sender<Request>,
    dialed: mpsc::Sender<DialedStream>,
    dial: Arc<DialShared>,
    clock: Arc<dyn Clock>,
    signals: Arc<SyncSignals>,
}

impl SyncHandle {
    /// The round-complete signal (written only by the actor).
    pub fn signals(&self) -> Arc<SyncSignals> {
        self.signals.clone()
    }

    /// Whether some active link negotiated `seat.status.v1`, so publishing
    /// this node's seats reaches anyone.
    pub fn seat_status_wanted(&self) -> bool {
        self.seat_status_wanted.load(Ordering::Acquire)
    }

    /// Hand this node's own seats to the actor for `seat_status` (ht-xg88).
    /// Never waits: `false` when the request queue was full and nothing was
    /// handed over (the caller tries again).
    pub fn publish_seat_status(&self, seats: Vec<SeatStatusEntry>) -> bool {
        self.requests.try_send(Request::PublishSeats(seats)).is_ok()
    }

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

    /// Whether the Dialer finished its first target reload, so
    /// [`Self::enabled_target_count`] is not the initial 0.
    pub fn dial_targets_loaded(&self) -> bool {
        self.dial.targets_loaded()
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

/// How a coordinator differs from the default; for tests that stand in for a
/// node of another release.
#[derive(Clone, Copy, Debug)]
pub struct CoordinatorOptions {
    /// Advertise [`OPAQUE_CAP`] in the hello (default). Without it, peers
    /// withhold opaque entries from this node.
    pub advertise_opaque: bool,
    /// The stall thresholds of node status.
    pub timings: StallTimings,
}

impl Default for CoordinatorOptions {
    fn default() -> Self {
        Self {
            advertise_opaque: true,
            timings: StallTimings::default(),
        }
    }
}

impl CoordinatorOptions {
    /// The daemon's options. Test-support builds read shortened stall
    /// thresholds and `seen` period from `HT_TEST_STALL_AFTER_MS`,
    /// `HT_TEST_APP_SILENT_AFTER_MS` and `HT_TEST_SEEN_EVERY_MS` (multinode
    /// tests).
    pub fn for_daemon() -> Self {
        #[allow(unused_mut)]
        let mut options = Self::default();
        #[cfg(feature = "test-support")]
        {
            let ms = |name: &str| std::env::var(name).ok().and_then(|v| v.parse::<u64>().ok());
            if let Some(v) = ms("HT_TEST_STALL_AFTER_MS") {
                options.timings.stall_after_ms = v;
            }
            if let Some(v) = ms("HT_TEST_APP_SILENT_AFTER_MS") {
                options.timings.app_silent_after_ms = v;
            }
            if let Some(v) = ms("HT_TEST_SEEN_EVERY_MS") {
                options.timings.seen_every_ms = v;
            }
        }
        options
    }
}

/// When a link counts as stalled (see [`Stall`]), and the `seen` period the
/// app threshold is measured against, in ms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StallTimings {
    pub stall_after_ms: u64,
    pub app_silent_after_ms: u64,
    pub seen_every_ms: u64,
}

impl Default for StallTimings {
    fn default() -> Self {
        Self {
            stall_after_ms: peer_status::STALL_AFTER_MS,
            app_silent_after_ms: peer_status::APP_SILENT_AFTER_MS,
            seen_every_ms: SEEN_EVERY_MS,
        }
    }
}

/// Starts the coordinator actor.
pub struct SyncCoordinator;

impl SyncCoordinator {
    /// Spawn the actor on the current tokio runtime. The `JoinHandle` ends
    /// after `deps.cancel` fires and every link has had its `close{shutdown}`
    /// (bounded at one second).
    pub fn start(deps: CoordinatorDeps) -> (SyncHandle, PeerIntake, JoinHandle<()>) {
        Self::start_with(deps, CoordinatorOptions::default())
    }

    /// [`start`](Self::start) with `options`.
    pub fn start_with(
        deps: CoordinatorDeps,
        options: CoordinatorOptions,
    ) -> (SyncHandle, PeerIntake, JoinHandle<()>) {
        let links = Arc::new(AtomicUsize::new(0));
        let signals = Arc::new(SyncSignals::new());
        let contacts = ContactBook::new(deps.identity.uuid());
        let (intake, intake_rx) = PeerIntake::with_slots(PEER_INTAKE_SLOTS);
        let (dialed_tx, dialed_rx) = mpsc::channel(DIALED_SLOTS);
        let (requests_tx, requests_rx) = mpsc::channel(REQUEST_SLOTS);
        let (events_tx, events_rx) = mpsc::channel(LINK_EVENT_SLOTS);
        let (ingest_tx, ingest_rx) = mpsc::channel(LINK_EVENT_SLOTS);
        // Bounded by credit (at most 16 links of 1 MiB), so unbounded here.
        let (jobs_tx, jobs_rx) = mpsc::unbounded_channel();
        let worker = ingest::spawn_worker(
            ingest::WorkerDeps {
                journal: deps.journal.clone(),
                writer: deps.writer.clone(),
                clock: deps.clock.clone(),
                cancel: deps.cancel.clone(),
            },
            jobs_rx,
            ingest_tx,
        );
        let dialer = Dialer::start(
            DialerDeps {
                store: deps.sync_store.clone(),
                clock: deps.clock.clone(),
                log: deps.log.clone(),
                dialed: dialed_tx.clone(),
                signals: signals.clone(),
            },
            deps.dial.clone(),
        );
        let seat_status_wanted = Arc::new(AtomicBool::new(false));
        let handle = SyncHandle {
            links: links.clone(),
            seat_status_wanted: seat_status_wanted.clone(),
            requests: requests_tx,
            dialed: dialed_tx,
            dial: dialer.shared(),
            clock: deps.clock.clone(),
            signals: signals.clone(),
        };
        let pin = deps.resync_pin.clone().map(|pin| ActivePin {
            pin,
            since: deps.clock.monotonic_now(),
        });
        let mut hello = LocalHello::new(deps.identity.clone(), deps.boot);
        hello.caps.push(FORK_CAP.to_owned());
        if options.advertise_opaque {
            hello.caps.push(OPAQUE_CAP.to_owned());
        }
        hello.caps.push(NODE_INFO_CAP.to_owned());
        hello.caps.push(SEAT_STATUS_CAP.to_owned());
        let me = Arc::new(hello);
        let actor = Actor {
            me,
            deps,
            count: links,
            links: BTreeMap::new(),
            by_node: BTreeMap::new(),
            rx: BTreeMap::new(),
            replaced: BTreeMap::new(),
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
            serve_jobs: JoinSet::new(),
            serving: BTreeSet::new(),
            serve_retry: BTreeSet::new(),
            branch_jobs: JoinSet::new(),
            ingest_jobs: jobs_tx,
            worker: Some(worker),
            retry_logs: BTreeSet::new(),
            oversize_logs: BTreeSet::new(),
            signals,
            contacts,
            flushes: JoinSet::new(),
            pin,
            timings: options.timings,
            peer_flushed: BTreeMap::new(),
            peer_flushed_at: None,
            peer_flushes: JoinSet::new(),
            peer_flush: peer_status::FlushHealth::default(),
            seats: seat_status::SeatStatusState::new(seat_status_wanted),
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

/// The source a log is being pulled from. `last_progress` is set when the
/// source is chosen or re-asked and by the ingest outcome `Stored`;
/// `requested_from` is what the last `Want` asked for (a `Gap` that names the
/// same counter is a stale batch).
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
    /// When the link came up on the monotonic clock, for `up_for`.
    started: MonoInstant,
    mode: LinkMode,
    /// In the duplicate order. False while a clone probe of an older link
    /// decides this link's fate.
    ranked: bool,
    /// `close` has been queued; the link no longer counts.
    closing: bool,
    /// What the peer advertises. Only an active link keeps one.
    peer_vector: Vector,
    /// Own logs this node does not hold that the peer advertises: when the
    /// link first advertised each. A `Have` is unsigned; such a claim holds
    /// the own history only within [`routing::OWN_CLAIM_PROOF_MS`] of this
    /// time unless the log's genesis arrives. Cleared with `peer_vector`.
    own_claims: BTreeMap<LogKey, OwnClaim>,
    /// A `have` is owed to the peer (`Some(ask)`) once `local` is fresh.
    have_pending: Option<bool>,
    /// This activation's first `have` has gone out; until then the link is
    /// no rx source, so a `want` never precedes the `have`.
    announced: bool,
    /// Penalty-until per log on this link (written by the failover and by
    /// the ingest outcomes).
    errors: BTreeMap<LogKey, MonoInstant>,
    /// What the peer wants from us: the next counter to push per log.
    subs_out: BTreeMap<LogKey, u64>,
    /// The log served last, so the next push starts after it.
    push_cursor: Option<LogKey>,
    /// Logs parked by the Opaque capability gate: the counter at which the
    /// batch was cut empty. Not served again on this link (its capabilities
    /// are fixed) unless the peer wants from before it.
    withheld_logs: BTreeMap<LogKey, u64>,
    /// `entries` frames dropped as unsolicited.
    unsolicited: u64,
    /// Inbound control frames dropped by the token bucket.
    throttled: u64,
    /// The bucket's tokens, in thousandths (a frame costs 1000).
    bucket: u64,
    /// When the bucket was last refilled.
    bucket_at: MonoInstant,
    /// The `subs_out` cap was already logged for this link.
    subs_capped_logged: bool,
    /// `branch_entries`/`branch_none` frames dropped for a shape error.
    bad_branch: u64,
    /// `branch_want`s the peer has outstanding, oldest first; the front is
    /// the one being served while `branch_busy`.
    branch_queue: VecDeque<BranchWant>,
    /// A `serve_branch` job for the front of `branch_queue` is in flight.
    branch_busy: bool,
    /// The heads the peer was last told of (a delta carries the difference).
    last_have_sent: Vector,
    /// When the last `have` went out.
    last_have_at: Option<MonoInstant>,
    /// The full `have` of more than [`MAX_HAVE_LOGS`] logs was logged as
    /// truncated for this link.
    have_truncation_logged: AtomicBool,
    /// The current sync round; only an active link has one.
    round: Option<RoundTrack>,
    /// When the last `seen` went out.
    seen_at: Option<MonoInstant>,
    /// The link's inbound frame count at the last contact poll (the link
    /// answers `ping`/`pong` itself, so only the count tells they arrived).
    polled_inbound: u64,
    /// Wall time of the last inbound frame of any kind.
    last_rx_wall: u64,
    /// The last frame the peer's coordinator sent (see `peer_status`).
    app_rx: MonoInstant,
    /// Wall time of the last coordinator frame; `None` before the first.
    app_rx_wall: Option<u64>,
    /// The peer's `node_info` version.
    peer_version: Option<String>,
    /// A full `have` arrived in this activation, so `peer_vector` is the
    /// peer's whole vector (unlike the round flag, a new round keeps it).
    vector_known: bool,
    /// This activation was sent every held seat-status snapshot.
    seat_status_sent: bool,
    /// The held counter last seen growing in each direction, and when it
    /// last made progress (see `peer_status::track_progress`).
    tx_mark: Option<(LogKey, u64)>,
    tx_progress_at: MonoInstant,
    rx_mark: Option<(LogKey, u64)>,
    rx_progress_at: MonoInstant,
    /// Held so the transport's slot is released when the link is.
    _permit: Option<PeerPermit>,
}

/// One unheld own-log claim of a link (see `LinkEntry::own_claims`).
struct OwnClaim {
    first_seen: MonoInstant,
    /// The expiry has been logged.
    logged: bool,
}

impl LinkEntry {
    /// The peer negotiated [`FORK_CAP`] on this link.
    fn fork_cap(&self) -> bool {
        self.handle.peer().caps.iter().any(|cap| cap == FORK_CAP)
    }

    /// The peer negotiated [`OPAQUE_CAP`] on this link: opaque entries may be
    /// sent to it (asked only through the gate, `serve::gate_batch`).
    fn opaque_cap(&self) -> bool {
        self.handle.peer().caps.iter().any(|cap| cap == OPAQUE_CAP)
    }

    /// Fork content may flow: the node's active, live link with the cap.
    fn fork_active(&self) -> bool {
        self.mode == LinkMode::Active && self.ranked && !self.closing && self.fork_cap()
    }
}

/// A link's round as the actor tracks it. It completes once a full `have`
/// arrived since the start and the peer advertises nothing more that is
/// wantable (see [`Actor::update_rounds`]).
#[derive(Clone, Copy)]
struct RoundTrack {
    started: MonoInstant,
    full_have: bool,
    complete: bool,
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
    /// Sources `apply_pick` just moved a log away from, with the end of the
    /// grace in which their in-flight `entries` are not counted unsolicited.
    replaced: BTreeMap<(LogKey, u64), MonoInstant>,
    /// The last read of `journal.vector()`.
    local: Vector,
    /// Logs not to be pulled (a fork the sink holds).
    paused: BTreeSet<LogKey>,
    /// All wants are paused until then (`Headroom`).
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
    /// `journal.serve` reads in flight (at most one per link).
    serve_jobs: JoinSet<ServeDone>,
    /// Links with a serve job in flight.
    serving: BTreeSet<u64>,
    /// Links whose last serve job failed; pushed again on the next tick.
    serve_retry: BTreeSet<u64>,
    /// `journal.serve_branch` reads in flight (at most one per link).
    branch_jobs: JoinSet<BranchDone>,
    /// The ingest worker's FIFO.
    ingest_jobs: mpsc::UnboundedSender<IngestJob>,
    /// The worker task; ended at shutdown.
    worker: Option<JoinHandle<()>>,
    /// Logs whose batch failed to store; asked again at the pacer's backoff.
    retry_logs: BTreeSet<LogKey>,
    /// Logs stalled on this node by an opaque entry over `OPAQUE_MAX_BYTES`:
    /// never wanted again until restart.
    oversize_logs: BTreeSet<LogKey>,
    /// The round-complete signal this actor writes.
    signals: Arc<SyncSignals>,
    /// Reachability: direct contacts, gossiped ages and the flush schedule.
    contacts: ContactBook,
    /// `record_contacts` calls in flight (at most one).
    flushes: JoinSet<FlushDone>,
    /// The resync rx pin while it holds.
    pin: Option<ActivePin>,
    /// Node status stall thresholds.
    timings: StallTimings,
    /// The peer-status rows the store holds, by node.
    peer_flushed: BTreeMap<NodeUuid, PeerStatus>,
    /// When the last peer-status write started.
    peer_flushed_at: Option<MonoInstant>,
    /// `record_peer_status` calls in flight (at most one).
    peer_flushes: JoinSet<peer_status::PeerFlushDone>,
    /// Failure backoff and stuck-write reporting of those calls.
    peer_flush: peer_status::FlushHealth,
    /// Own and relayed seat-status snapshots (ht-xg88).
    seats: seat_status::SeatStatusState,
}

/// A [`ResyncPin`] and when it took hold (the coordinator's monotonic
/// clock).
struct ActivePin {
    pin: ResyncPin,
    since: MonoInstant,
}

/// A contact flush joined from the actor's `JoinSet`.
struct FlushDone {
    rows: Vec<NodeContact>,
    result: Result<(), ApiError>,
}

impl Actor {
    async fn run(mut self, mut inputs: Inputs) {
        let _liveness = self.deps.status.lane_guard(&self.deps.cancel);
        let cancel = self.deps.cancel.clone();
        let pacer = self.deps.pacer.clone();
        let signals = self.signals.clone();
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
                () = signals.requested() => self.on_round_request(),
                Some(joined) = self.flushes.join_next(), if !self.flushes.is_empty() => {
                    if let Ok(done) = joined {
                        self.on_flushed(done);
                    }
                }
                Some(joined) = self.peer_flushes.join_next(), if !self.peer_flushes.is_empty() => {
                    self.on_peer_status_joined(joined);
                }
                Some(joined) = self.seats.writes.join_next(), if !self.seats.writes.is_empty() => {
                    self.on_seat_status_written(joined);
                }
                Some(done) = inputs.ingest.recv() => self.on_ingested(done),
                Some(joined) = self.serve_jobs.join_next(), if !self.serve_jobs.is_empty() => {
                    if let Ok(done) = joined {
                        self.on_served(done);
                    }
                }
                Some(joined) = self.branch_jobs.join_next(), if !self.branch_jobs.is_empty() => {
                    if let Ok(done) = joined {
                        self.on_branch_served(done);
                    }
                }
                wake = pacer.wait(PACER_TICK) => {
                    // A latched kick survives the loop re-creating this wait.
                    // A due backoff (`RetryDue`) clears the latched kicks, so
                    // it does their work too; `on_retry_due` always moves the
                    // backoff on, so a due pacer cannot spin the loop.
                    if wake == Wake::RetryDue {
                        self.on_retry_due();
                    }
                    if matches!(wake, Wake::Kicked | Wake::RetryDue) {
                        // A table or commit kick: re-diff the dial targets
                        // (the Dialer limits this to once a second) and
                        // re-read the local vector, whose arrival pushes the
                        // armed subscriptions and the `have` deltas.
                        self.dialer.kick();
                        self.request_local();
                    }
                }
                _ = tick.tick() => self.housekeeping(),
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
                (self.deps.log)(&format!(
                    "sync: link up {} ({})",
                    uuid8(&node),
                    match &origin {
                        Origin::Inbound => "inbound",
                        Origin::Dialed { .. } => "dialed",
                    }
                ));
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
                let now = self.mono();
                self.links.insert(
                    link_id,
                    LinkEntry {
                        handle,
                        origin,
                        since: self.deps.clock.utc_now(),
                        started: now,
                        mode: LinkMode::Standby,
                        ranked: false,
                        closing: false,
                        peer_vector: Vector::default(),
                        own_claims: BTreeMap::new(),
                        have_pending: None,
                        announced: false,
                        errors: BTreeMap::new(),
                        subs_out: BTreeMap::new(),
                        push_cursor: None,
                        withheld_logs: BTreeMap::new(),
                        unsolicited: 0,
                        throttled: 0,
                        bucket: CONTROL_BUCKET * 1_000,
                        bucket_at: now,
                        subs_capped_logged: false,
                        bad_branch: 0,
                        branch_queue: VecDeque::new(),
                        branch_busy: false,
                        last_have_sent: Vector::default(),
                        last_have_at: None,
                        have_truncation_logged: AtomicBool::new(false),
                        round: None,
                        seen_at: None,
                        polled_inbound: 0,
                        last_rx_wall: self.wall(),
                        app_rx: now,
                        app_rx_wall: None,
                        peer_version: None,
                        vector_known: false,
                        seat_status_sent: false,
                        tx_mark: None,
                        tx_progress_at: now,
                        rx_mark: None,
                        rx_progress_at: now,
                        _permit: permit,
                    },
                );
                self.send_node_info(link_id);
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

    /// How long the link has been up on the monotonic clock, like the link
    /// task's own `up_for` (`on_ended` adds the wall time).
    fn up_for(&self, link_id: u64) -> Duration {
        self.links.get(&link_id).map_or(Duration::ZERO, |entry| {
            Duration::from_millis(self.mono().0.saturating_sub(entry.started.0))
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
        self.update_rounds();
    }

    /// The link becomes the node's standby: it keeps ping/pong only.
    fn demote(&mut self, link_id: u64) {
        let Some(entry) = self.links.get_mut(&link_id) else {
            return;
        };
        entry.mode = LinkMode::Standby;
        entry.peer_vector = Vector::default();
        entry.vector_known = false;
        entry.own_claims.clear();
        entry.have_pending = None;
        entry.announced = false;
        entry.errors.clear();
        entry.subs_out.clear();
        entry.push_cursor = None;
        entry.branch_queue.clear();
        entry.last_have_sent = Vector::default();
        entry.last_have_at = None;
        entry.round = None;
        entry.seen_at = None;
        self.drop_rx_of(link_id);
        self.update_rounds();
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
        let started = self.mono();
        let Some(entry) = self.links.get_mut(&link_id) else {
            return;
        };
        entry.mode = LinkMode::Active;
        entry.announced = false;
        entry.have_pending = Some(ask);
        entry.seen_at = None;
        entry.app_rx = started;
        entry.vector_known = false;
        entry.seat_status_sent = false;
        entry.tx_mark = None;
        entry.tx_progress_at = started;
        entry.rx_mark = None;
        entry.rx_progress_at = started;
        entry.round = Some(RoundTrack {
            started,
            full_have: false,
            complete: false,
        });
        self.request_local();
        self.update_rounds();
    }

    /// A `have` or `want` arrived on a standby link: the peer promoted it, so
    /// it is active here too and the formerly active link is demoted. The
    /// activation asks for the peer's vector unless `vector_known` (the
    /// promoting frame is a full `have`, which carries it).
    fn promote_on_traffic(&mut self, link_id: u64, vector_known: bool) {
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
        self.activate(link_id, !vector_known);
    }

    fn on_link_event(&mut self, event: LinkEvent) {
        match event {
            LinkEvent::Ended {
                link_id,
                why,
                up_for,
            } => self.on_ended(link_id, why, up_for),
            LinkEvent::Frame {
                link_id,
                frame,
                wire_len,
            } => self.on_frame(link_id, frame, wire_len),
            LinkEvent::CreditFreed { link_id } => self.push_link(link_id),
        }
    }

    fn on_frame(&mut self, link_id: u64, frame: PeerFrame, wire_len: usize) {
        let Some(entry) = self.links.get(&link_id) else {
            return;
        };
        // Any frame of an established link, active or standby, is direct
        // contact with its node.
        let (node, usable) = (entry.handle.peer().node, entry.ranked && !entry.closing);
        let wall = self.wall();
        self.contacts.record_direct(node, wall);
        let now = self.mono();
        if let Some(entry) = self.links.get_mut(&link_id) {
            // Every frame that reaches the actor came from the peer's
            // coordinator: the link answers ping/pong/credit itself.
            entry.app_rx = now;
            entry.app_rx_wall = Some(wall);
            entry.last_rx_wall = wall;
        }
        if matches!(
            frame,
            PeerFrame::Want { .. }
                | PeerFrame::Unwant { .. }
                | PeerFrame::Have { .. }
                | PeerFrame::BranchWant(_)
        ) && !self.admit_control(link_id)
        {
            return;
        }
        // A `branch_want` is answered even by a link that is not the node's
        // active one (`branch_none`), never silently dropped.
        let frame = match frame {
            PeerFrame::BranchWant(want) => {
                self.on_branch_want(link_id, want);
                return;
            }
            PeerFrame::NodeInfo(info) => {
                self.on_node_info(link_id, info);
                return;
            }
            other => other,
        };
        if !usable {
            return;
        }
        let entry = &self.links[&link_id];
        if let PeerFrame::Seen { ages } = &frame {
            self.contacts.merge_seen(ages, wall);
            return;
        }
        if entry.mode == LinkMode::Standby {
            // A standby carries ping/pong (the link's own) and `seen` only
            // (handled above); a `have` or `want` is the peer's promotion.
            if matches!(frame, PeerFrame::Have { .. } | PeerFrame::Want { .. }) {
                self.promote_on_traffic(
                    link_id,
                    matches!(frame, PeerFrame::Have { full: true, .. }),
                );
            } else {
                if matches!(
                    frame,
                    PeerFrame::Entries { .. } | PeerFrame::BranchEntries { .. }
                ) {
                    // Never wanted here: hand its credit back.
                    if let Some(entry) = self.links.get_mut(&link_id) {
                        entry.unsolicited += 1;
                        let _ = entry
                            .handle
                            .grant(u32::try_from(wire_len).unwrap_or(u32::MAX));
                    }
                }
                return;
            }
        }
        // `seen` is recorded by ht-5fr.5.6; the branch frames are ht-5fr.6's.
        match frame {
            PeerFrame::Have {
                full,
                ask,
                logs,
                forks,
            } => self.on_have(link_id, full, ask, &logs, &forks),
            PeerFrame::Want { node, inc, from } => self.on_want(
                link_id,
                LogKey {
                    node,
                    incarnation: inc,
                },
                from,
            ),
            PeerFrame::Unwant { node, inc } => self.on_unwant(
                link_id,
                &LogKey {
                    node,
                    incarnation: inc,
                },
            ),
            PeerFrame::Entries { node, inc, entries } => {
                self.on_entries(link_id, node, inc, entries, wire_len);
            }
            reply @ (PeerFrame::BranchEntries { .. } | PeerFrame::BranchNone { .. }) => {
                self.on_branch_reply(link_id, reply, wire_len);
            }
            PeerFrame::SeatStatus(frame) => self.on_seat_status(link_id, frame),
            _ => {}
        }
    }

    /// Spend one token of the link's control bucket (refilled
    /// [`CONTROL_REFILL_PER_S`] a second). With none left the frame is
    /// dropped and counted; past [`MAX_THROTTLED`] drops the link is closed.
    fn admit_control(&mut self, link_id: u64) -> bool {
        let now = self.mono();
        let Some(entry) = self.links.get_mut(&link_id) else {
            return false;
        };
        if entry.closing {
            return false;
        }
        let elapsed = now.0.saturating_sub(entry.bucket_at.0);
        entry.bucket_at = now;
        // One token is 1000 units and `CONTROL_REFILL_PER_S` a second is that
        // many units per millisecond.
        entry.bucket = entry
            .bucket
            .saturating_add(elapsed.saturating_mul(CONTROL_REFILL_PER_S))
            .min(CONTROL_BUCKET * 1_000);
        if entry.bucket >= 1_000 {
            entry.bucket -= 1_000;
            return true;
        }
        entry.throttled += 1;
        if entry.throttled > MAX_THROTTLED {
            entry.closing = true;
            let _ = entry.handle.close(CloseReason::ProtocolError);
            (self.deps.log)(&format!(
                "sync: link {link_id} flooded control frames; closing"
            ));
        }
        false
    }

    /// Merge a peer's `have` into its `peer_vector` and re-pick the rx source
    /// of every log it touched.
    fn on_have(
        &mut self,
        link_id: u64,
        full: bool,
        ask: bool,
        logs: &[HaveLog],
        forks: &[ForkRef],
    ) {
        let now = self.mono();
        let Some(entry) = self.links.get_mut(&link_id) else {
            return;
        };
        if full && let Some(round) = entry.round.as_mut() {
            round.full_have = true;
        }
        entry.vector_known |= full;
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
        // The sink sees every `have` after the merge; fork refs only from a
        // peer that negotiated the cap (anything else it sent is ignored).
        if let Some(entry) = self.links.get(&link_id) {
            let peer = entry.handle.peer();
            let fork_cap = peer.caps.iter().any(|cap| cap == FORK_CAP);
            let forks = if fork_cap { forks } else { &[] };
            self.deps
                .fork_sink
                .on_have(peer.node, fork_cap, logs, forks);
        }
        self.update_rounds();
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
                let now = self.mono();
                for (id, ask) in owed {
                    self.send_have(id, ask);
                    if let Some(entry) = self.links.get_mut(&id) {
                        entry.have_pending = None;
                        entry.announced = true;
                        entry.last_have_sent = self.local.clone();
                        entry.last_have_at = Some(now);
                    }
                    // The first `seen` of an activation follows its `have`.
                    self.send_seen_if_due(id, now);
                }
                self.flush_have_deltas(now);
            }
            Err(e) => (self.deps.log)(&format!("sync: local vector read failed: {e}")),
        }
        self.reselect_all();
        self.update_own_history();
        self.update_rounds();
        // The vector moved: serve what armed subscriptions now have waiting.
        self.push_all();
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
            .map(|(key, head)| serve::have_log(key, head))
            .collect();
        if self.local.0.len() > MAX_HAVE_LOGS
            && !entry.have_truncation_logged.swap(true, Ordering::AcqRel)
        {
            // The frame has no field for a marker; the peer learns the rest
            // through deltas and `want`.
            (self.deps.log)(&format!(
                "sync: have to {} lists {MAX_HAVE_LOGS} of {} logs",
                uuid8(&entry.handle.peer().node),
                self.local.0.len()
            ));
        }
        let forks = if entry.fork_cap() {
            self.fork_refs()
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
        self.paused.contains(log)
            || self.oversize_logs.contains(log)
            || self.headroom_until.is_some_and(|until| until.0 > now.0)
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
        let pinned_to = self
            .pin
            .as_ref()
            .filter(|active| active.pin.covers(log))
            .filter(|active| now.0.saturating_sub(active.since.0) < routing::PIN_LIFT_MS)
            .map(|active| active.pin.from);
        routing::reselect(
            log,
            self.local.0.get(log),
            &views,
            self.log_paused(log, now),
            current,
            now,
            pinned_to,
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
                    let until = MonoInstant(self.mono().0 + REPLACED_GRACE_MS);
                    self.replaced.insert((log, old), until);
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
            let before = self.links.get_mut(&link_id).and_then(|entry| {
                entry
                    .errors
                    .insert(log, MonoInstant(now.0 + routing::STALL_PENALTY_MS))
            });
            match self.pick(&log) {
                Some(new) => self.apply_pick(log, Some(new)),
                None => {
                    // The sole candidate: no penalty, or the next reselect
                    // would drop it. Ask again and judge it after another
                    // stall window (`start_rx` resets `last_progress`).
                    if let Some(entry) = self.links.get_mut(&link_id) {
                        match before {
                            Some(until) => entry.errors.insert(log, until),
                            None => entry.errors.remove(&log),
                        };
                    }
                    self.start_rx(log, link_id);
                }
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
        self.replaced.retain(|_, until| until.0 > now.0);
        self.lift_pauses(now);
        self.expire_pin(now);
        self.failover(now);
        self.flush_have_deltas(now);
        self.drain_fork_outbox();
        for id in std::mem::take(&mut self.serve_retry) {
            self.push_link(id);
        }
        self.poll_contacts();
        let ids: Vec<u64> = self.links.keys().copied().collect();
        for id in ids {
            self.send_seen_if_due(id, now);
        }
        self.flush_contacts();
        self.track_progress();
        self.flush_peer_status();
        self.seat_status_tick();
        self.update_own_history();
        self.update_rounds();
        if self.refresh.is_empty() && self.links.values().any(|l| l.have_pending.is_some()) {
            // A read that failed leaves `have`s owed: try again.
            self.request_local();
        }
    }

    /// Set the own-history `Links` flag from the active links' vectors and
    /// the local one (the pure [`routing::own_history_on`]).
    ///
    /// An own log a peer advertises that this node does not hold is an
    /// unsigned claim: its link gets a proof window from the first
    /// advertisement, and once it has run out without the genesis the claim
    /// is ignored for the link's lifetime.
    fn update_own_history(&mut self) {
        let own = self.deps.identity.uuid();
        let now = self.mono();
        let mut expired: Vec<(NodeUuid, LogKey)> = Vec::new();
        for entry in self.links.values_mut() {
            if entry.mode != LinkMode::Active {
                entry.own_claims.clear();
                continue;
            }
            let local = &self.local;
            let unheld = |log: &LogKey, adv: &LogHead| {
                log.node == own
                    && adv.counter > 0
                    && local.0.get(log).is_none_or(|head| head.counter == 0)
            };
            // A claim whose log arrived is forgotten. One the peer later
            // omits stays, so restating it cannot open a fresh window.
            entry
                .own_claims
                .retain(|log, _| local.0.get(log).is_none_or(|head| head.counter == 0));
            for (log, adv) in &entry.peer_vector.0 {
                if unheld(log, adv)
                    && !entry.own_claims.contains_key(log)
                    && entry.own_claims.len() < MAX_OWN_CLAIMS
                {
                    entry.own_claims.insert(
                        *log,
                        OwnClaim {
                            first_seen: now,
                            logged: false,
                        },
                    );
                }
            }
            for (log, claim) in &mut entry.own_claims {
                if now.0.saturating_sub(claim.first_seen.0) >= routing::OWN_CLAIM_PROOF_MS
                    && !claim.logged
                {
                    claim.logged = true;
                    expired.push((entry.handle.peer().node, *log));
                }
            }
        }
        for (peer, log) in expired {
            (self.deps.log)(&format!(
                "sync: {} advertises own log {} it cannot serve; ignored",
                uuid8(&peer),
                log.incarnation
            ));
        }
        let on = routing::own_history_on(
            own,
            &self.local,
            self.links
                .iter()
                .filter(|(_, l)| l.mode == LinkMode::Active && l.ranked && !l.closing)
                .map(|(id, l)| (*id, &l.peer_vector)),
            |id, log| {
                self.links
                    .get(id)
                    .and_then(|l| l.own_claims.get(log))
                    .is_some_and(|claim| {
                        now.0.saturating_sub(claim.first_seen.0) < routing::OWN_CLAIM_PROOF_MS
                    })
            },
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
        let wall =
            u64::try_from(self.deps.clock.utc_now().0.saturating_sub(entry.since.0)).unwrap_or(0);
        (self.deps.log)(&format!(
            "sync: link down {} after {}: {}",
            uuid8(&node),
            super::link::up_for_text(up_for, Duration::from_millis(wall)),
            why
        ));
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
        self.update_rounds();
    }

    /// The local wall clock in unix ms (contacts are wall times).
    fn wall(&self) -> u64 {
        u64::try_from(self.deps.clock.utc_now().0).unwrap_or(0)
    }

    /// Record direct contact for every link whose inbound frame count moved
    /// since the last poll: the link answers `ping`/`pong` itself, so a
    /// standby's only traffic is visible nowhere else.
    fn poll_contacts(&mut self) {
        let wall = self.wall();
        for entry in self.links.values_mut() {
            let inbound = entry.handle.inbound_frames();
            if inbound > entry.polled_inbound {
                entry.polled_inbound = inbound;
                entry.last_rx_wall = wall;
                self.contacts.record_direct(entry.handle.peer().node, wall);
            }
        }
    }

    /// `seen` on an active, announced link: right after its first `have` and
    /// then every `timings.seen_every_ms` ([`SEEN_EVERY_MS`] in production).
    fn send_seen_if_due(&mut self, link_id: u64, now: MonoInstant) {
        let wall = self.wall();
        let Some(entry) = self.links.get_mut(&link_id) else {
            return;
        };
        if entry.mode != LinkMode::Active || !entry.ranked || entry.closing || !entry.announced {
            return;
        }
        if entry
            .seen_at
            .is_some_and(|at| now.0.saturating_sub(at.0) < self.timings.seen_every_ms)
        {
            return;
        }
        let ages = self.contacts.seen_ages(&entry.handle.peer().node, wall);
        if entry.handle.send(PeerFrame::Seen { ages }).is_ok() {
            entry.seen_at = Some(now);
        }
    }

    /// Write the due contact rows in one background call; at most one is in
    /// flight, and a failed one is due again at the next flush.
    fn flush_contacts(&mut self) {
        if !self.flushes.is_empty() {
            return;
        }
        let rows = self.contacts.due_for_flush(self.wall());
        if rows.is_empty() {
            return;
        }
        let store = self.deps.sync_store.clone();
        self.flushes.spawn_blocking(move || {
            let result = store.record_contacts(&rows, &unbounded_budget());
            FlushDone { rows, result }
        });
    }

    fn on_flushed(&mut self, done: FlushDone) {
        match done.result {
            Ok(()) => self.contacts.flushed(&done.rows),
            Err(e) => (self.deps.log)(&format!(
                "sync: contact flush failed: {:?}: {:?}",
                e.code, e.detail
            )),
        }
    }

    /// Best effort at shutdown: finish the flush in flight, then write every
    /// row the store does not hold yet. Each step is bounded.
    async fn final_flush(&mut self) {
        let _ = timeout(CLOSE_WRITE_TIMEOUT, async {
            while let Some(joined) = self.flushes.join_next().await {
                if let Ok(done) = joined {
                    self.on_flushed(done);
                }
            }
        })
        .await;
        self.poll_contacts();
        let rows = self.contacts.unflushed();
        if rows.is_empty() {
            return;
        }
        let store = self.deps.sync_store.clone();
        let call = tokio::task::spawn_blocking(move || {
            let result = store.record_contacts(&rows, &unbounded_budget());
            FlushDone { rows, result }
        });
        if let Ok(Ok(done)) = timeout(CLOSE_WRITE_TIMEOUT, call).await {
            self.on_flushed(done);
        }
    }

    /// A round was requested: every active link starts a new one and is sent
    /// `Have{full, ask: true}` (once `local` is fresh).
    fn on_round_request(&mut self) {
        let Some(at) = self.signals.take_request() else {
            return;
        };
        let started = self.mono().max(at);
        let mut any = false;
        for entry in self.links.values_mut() {
            if entry.mode == LinkMode::Active && entry.ranked && !entry.closing {
                entry.round = Some(RoundTrack {
                    started,
                    full_have: false,
                    complete: false,
                });
                entry.have_pending = Some(true);
                any = true;
            }
        }
        if any {
            self.request_local();
        }
        self.update_rounds();
    }

    /// Whether `link_id`'s peer advertises a log we could still pull from it:
    /// ahead of what is held, and neither paused, frozen nor penalized.
    fn advertises_wantable(&self, link_id: u64, now: MonoInstant) -> bool {
        self.links.get(&link_id).is_some_and(|entry| {
            entry.peer_vector.0.iter().any(|(log, adv)| {
                routing::wantable(self.local.0.get(log), adv, self.log_paused(log, now))
                    && entry.errors.get(log).is_none_or(|until| until.0 <= now.0)
            })
        })
    }

    /// Ends the resync pin and re-picks every log: `why` is logged.
    fn lift_pin(&mut self, why: &str) {
        if let Some(active) = self.pin.take() {
            (self.deps.log)(&format!(
                "sync: resync pin to {} lifted: {why}",
                uuid8(&active.pin.from)
            ));
            self.reselect_all();
        }
    }

    /// The pin's 10-minute bound.
    fn expire_pin(&mut self, now: MonoInstant) {
        if self
            .pin
            .as_ref()
            .is_some_and(|active| now.0.saturating_sub(active.since.0) >= routing::PIN_LIFT_MS)
        {
            self.lift_pin("10 minutes passed");
        }
    }

    /// The pin lifts once a round with the pinned peer has completed.
    fn lift_pin_after_round(&mut self) {
        let Some(from) = self.pin.as_ref().map(|active| active.pin.from) else {
            return;
        };
        let done = self.links.values().any(|l| {
            l.mode == LinkMode::Active
                && l.ranked
                && !l.closing
                && l.handle.peer().node == from
                && l.round.is_some_and(|r| r.complete)
        });
        if done {
            self.lift_pin("round with it completed");
        }
    }

    /// Complete the rounds whose rule holds (a full `have` since the start and
    /// nothing wantable left to pull), then publish every active link's round
    /// to [`SyncSignals`]. Called wherever the inputs of the rule change.
    fn update_rounds(&mut self) {
        let now = self.mono();
        let open: Vec<u64> = self
            .links
            .iter()
            .filter(|(_, l)| l.mode == LinkMode::Active && l.ranked && !l.closing)
            .filter(|(_, l)| l.round.is_some_and(|r| r.full_have && !r.complete))
            .map(|(id, _)| *id)
            .collect();
        for id in open {
            if !self.advertises_wantable(id, now)
                && let Some(round) = self.links.get_mut(&id).and_then(|l| l.round.as_mut())
            {
                round.complete = true;
            }
        }
        self.lift_pin_after_round();
        let rounds = self
            .links
            .iter()
            .filter(|(_, l)| l.mode == LinkMode::Active && l.ranked && !l.closing)
            .filter_map(|(id, l)| {
                l.round.map(|r| {
                    (
                        *id,
                        LinkRound {
                            node: l.handle.peer().node,
                            started: r.started,
                            complete: r.complete,
                        },
                    )
                })
            })
            .collect();
        self.signals.publish(rounds);
    }

    fn on_request(&mut self, request: Request) {
        match request {
            Request::PublishSeats(seats) => self.on_publish_seats(seats),
            Request::Snapshot(reply) => {
                let mono_now = self.mono();
                let links = self
                    .links
                    .iter()
                    .take(SNAPSHOT_MAX_LINKS)
                    .map(|(id, entry)| {
                        let peer: &PeerInfo = entry.handle.peer();
                        let (tx, rx) = self.link_positions(entry);
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
                            unsolicited: entry.unsolicited,
                            throttled: entry.throttled,
                            subscriptions: u32::try_from(entry.subs_out.len()).unwrap_or(u32::MAX),
                            withheld_logs: entry.withheld_logs.len() as u64,
                            last_error: None,
                            last_rx: UtcMillis(
                                i64::try_from(entry.last_rx_wall).unwrap_or(i64::MAX),
                            ),
                            app_rx: entry
                                .app_rx_wall
                                .map(|at| UtcMillis(i64::try_from(at).unwrap_or(i64::MAX))),
                            peer_version: entry.peer_version.clone(),
                            tx: tx.map(|d| (d.held, d.head)),
                            rx: rx.map(|d| (d.held, d.head)),
                            stalled: self.stall_of(entry, tx, rx, mono_now),
                        }
                    })
                    .collect();
                let now = self.mono();
                let headroom_until =
                    self.headroom_until
                        .filter(|until| until.0 > now.0)
                        .map(|until| {
                            let left = i64::try_from(until.0 - now.0).unwrap_or(i64::MAX);
                            UtcMillis(self.deps.clock.utc_now().0.saturating_add(left))
                        });
                let mut advertised = BTreeMap::<LogKey, u64>::new();
                for entry in self
                    .links
                    .values()
                    .filter(|entry| entry.mode == LinkMode::Active && !entry.closing)
                {
                    for (log, head) in &entry.peer_vector.0 {
                        if advertised.len() >= SNAPSHOT_MAX_ADVERTISED
                            && !advertised.contains_key(log)
                        {
                            continue;
                        }
                        let slot = advertised.entry(*log).or_insert(0);
                        *slot = (*slot).max(head.counter);
                    }
                }
                let _ = reply.send(SyncSnapshot {
                    links,
                    paused_logs: self.paused.iter().copied().collect(),
                    headroom_until,
                    advertised,
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
        // Each in-flight handshake counted itself in at start and would have
        // counted out on completion; aborting skips that.
        self.count
            .fetch_sub(self.handshakes.len(), Ordering::AcqRel);
        self.handshakes.abort_all();
        self.serve_jobs.abort_all();
        self.branch_jobs.abort_all();
        // Closing the FIFO ends the worker once its current batch is done.
        let (closed, _) = mpsc::unbounded_channel();
        drop(std::mem::replace(&mut self.ingest_jobs, closed));
        if let Some(worker) = self.worker.take() {
            let _ = timeout(CLOSE_WRITE_TIMEOUT, worker).await;
        }
        self.final_flush().await;
        self.final_peer_status_flush().await;
        self.final_seat_status_flush().await;
        self.dialer.shutdown().await;
        let leftover: Vec<u64> = self.links.keys().copied().collect();
        for link_id in leftover {
            self.on_ended(link_id, LinkError::Shutdown, Duration::ZERO);
        }
    }
}

fn unbounded_budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Cancellation::default(),
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
    let refuse = match held {
        Ok(Ok(Some(key))) if key != established.peer.pubkey => CloseReason::AuthFailed,
        Ok(Ok(_)) => return Ok(established),
        // Fail closed: an impostor must not get in because the lookup broke.
        // `busy` is a retry class, so the peer dials again later.
        Ok(Err(e)) => {
            log(&format!(
                "sync: held key lookup for {} failed: {:?}; link refused",
                uuid8(&node),
                e.code
            ));
            CloseReason::Busy
        }
        Err(_) => {
            log(&format!(
                "sync: held key lookup for {} panicked; link refused",
                uuid8(&node)
            ));
            CloseReason::Busy
        }
    };
    let close = PeerFrame::Close {
        reason: refuse,
        detail: None,
    };
    let _ = timeout(CLOSE_WRITE_TIMEOUT, async {
        write_peer_frame(&mut established.writer, &close).await?;
        established.writer.flush().await
    })
    .await;
    Err(LinkError::Refused(refuse))
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
    // Counter 1 is always a known HTJRNL01 genesis; an opaque entry has none.
    Ok(first
        .into_iter()
        .filter_map(|entry| entry.as_known().cloned())
        .find(|entry| entry.kind == GENESIS_KIND)
        .and_then(|entry| serde_json::from_slice::<GenesisPayload>(&entry.payload).ok())
        .and_then(|payload| hex_decode::<32>(&payload.pubkey)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::transport::{self, MAX_FRAME_BYTES};
    use crate::store::sync_contacts::ContactVia;
    use crate::store::sync_dial_targets::{DialTarget, DialTransition};
    use crate::sync::envelope::{Deps, Envelope, UnsignedEnvelope, hex_encode};
    use crate::sync::frame::{OpaqueWire, read_peer_frame};
    use crate::sync::hlc::HlcStamp;
    use crate::sync::journal_port::{ApplyProgress, IngestOutcome};
    use crate::sync::memory_journal::MemoryJournal;
    use crate::sync::signals::RoundState;
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

    /// The log-only sink, recording every clone report and fork too. `ack`
    /// is what `on_fork` answers; `resume` is handed out by `resumed()`.
    struct RecordingSink {
        inner: LogOnlyForkSink,
        clones: Mutex<Vec<CloneReport>>,
        forks: Mutex<Vec<(LogKey, NodeUuid)>>,
        ack: Mutex<ForkAck>,
        resume: Mutex<Vec<LogKey>>,
        /// Every `on_have`: source, fork cap, logs, fork refs.
        haves: Mutex<Vec<RecordedHave>>,
        /// Every `on_branch`.
        branches: Mutex<Vec<(NodeUuid, BranchReply)>>,
        /// What the next `outbox()` hands out.
        outbox: Mutex<ForkOutbox>,
        /// What `fork_refs()` answers.
        refs: Mutex<Vec<ForkRef>>,
    }

    type RecordedHave = (NodeUuid, bool, Vec<HaveLog>, Vec<ForkRef>);

    impl ForkSink for RecordingSink {
        fn on_fork(&self, log: LogKey, evidence: ForkEvidence, source: NodeUuid) -> ForkAck {
            self.inner.on_fork(log, evidence, source);
            self.forks.lock().unwrap().push((log, source));
            *self.ack.lock().unwrap()
        }
        fn resumed(&self) -> Vec<LogKey> {
            std::mem::take(&mut *self.resume.lock().unwrap())
        }
        fn on_clone(&self, report: CloneReport) {
            self.inner.on_clone(report.clone());
            self.clones.lock().unwrap().push(report);
        }
        fn on_have(&self, source: NodeUuid, fork_cap: bool, logs: &[HaveLog], forks: &[ForkRef]) {
            self.haves
                .lock()
                .unwrap()
                .push((source, fork_cap, logs.to_vec(), forks.to_vec()));
        }
        fn on_branch(&self, source: NodeUuid, reply: BranchReply) {
            self.branches.lock().unwrap().push((source, reply));
        }
        fn outbox(&self) -> ForkOutbox {
            std::mem::take(&mut *self.outbox.lock().unwrap())
        }
        fn fork_refs(&self) -> Vec<ForkRef> {
            self.refs.lock().unwrap().clone()
        }
    }

    /// A `MemorySyncStore` that records every `record_contacts` call (when it
    /// ran and with which rows) and can be told to fail them.
    struct CountingStore {
        inner: MemorySyncStore,
        clock: Arc<dyn Clock>,
        calls: Mutex<Vec<(i64, Vec<NodeContact>)>>,
        fail: AtomicBool,
        /// `record_peer_status` calls, and whether they fail.
        peer_calls: AtomicUsize,
        peer_fail: AtomicBool,
    }

    impl CountingStore {
        fn calls(&self) -> Vec<(i64, Vec<NodeContact>)> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl SyncStorePort for CountingStore {
        fn enabled_targets(&self, budget: &CallBudget) -> Result<Vec<DialTarget>, ApiError> {
            self.inner.enabled_targets(budget)
        }
        fn record_transition(
            &self,
            target: TargetId,
            transition: &DialTransition,
            now: UtcMillis,
            budget: &CallBudget,
        ) -> Result<(), ApiError> {
            self.inner
                .record_transition(target, transition, now, budget)
        }
        fn record_contacts(
            &self,
            contacts: &[NodeContact],
            budget: &CallBudget,
        ) -> Result<(), ApiError> {
            self.calls
                .lock()
                .unwrap()
                .push((self.clock.utc_now().0, contacts.to_vec()));
            if self.fail.load(Ordering::SeqCst) {
                return Err(ApiError::new(
                    ErrorCode::StoreBusy,
                    "scripted flush failure",
                ));
            }
            self.inner.record_contacts(contacts, budget)
        }
        fn record_peer_status(
            &self,
            rows: &[crate::store::sync_peer_status::PeerStatus],
            budget: &CallBudget,
        ) -> Result<(), ApiError> {
            self.peer_calls.fetch_add(1, Ordering::SeqCst);
            if self.peer_fail.load(Ordering::SeqCst) {
                return Err(ApiError::new(
                    ErrorCode::StoreBusy,
                    "scripted peer status failure",
                ));
            }
            self.inner.record_peer_status(rows, budget)
        }
        fn record_remote_seat_status(
            &self,
            updates: &[crate::store::remote_seat_status::RemoteSeatUpdate],
            budget: &CallBudget,
        ) -> Result<(), ApiError> {
            self.inner.record_remote_seat_status(updates, budget)
        }
    }

    struct Rig {
        store: Arc<CountingStore>,
        handle: SyncHandle,
        intake: PeerIntake,
        join: JoinHandle<()>,
        cancel: Cancellation,
        clock: Arc<dyn Clock>,
        pacer: Arc<Pacer>,
        own_history: Arc<OwnHistoryHold>,
        sink: Arc<RecordingSink>,
        status: Arc<WorkerStatus>,
        lines: Lines,
    }

    impl Rig {
        fn clones(&self) -> Vec<CloneReport> {
            self.sink.clones.lock().unwrap().clone()
        }
    }

    fn start_with(journal: Arc<dyn JournalPort>) -> Rig {
        start_node(1, journal)
    }

    fn start_node(n: u8, journal: Arc<dyn JournalPort>) -> Rig {
        start_node_pinned(n, journal, None)
    }

    fn start_node_pinned(
        n: u8,
        journal: Arc<dyn JournalPort>,
        resync_pin: Option<ResyncPin>,
    ) -> Rig {
        start_node_full(n, journal, resync_pin, CoordinatorOptions::default())
    }

    fn start_node_full(
        n: u8,
        journal: Arc<dyn JournalPort>,
        resync_pin: Option<ResyncPin>,
        options: CoordinatorOptions,
    ) -> Rig {
        let clock: Arc<dyn Clock> = Arc::new(TokioClock(Instant::now()));
        let cancel = Cancellation::default();
        let (log, lines) = capture();
        let pacer = Arc::new(Pacer::new("sync", clock.clone(), cancel.clone()));
        let own_history = Arc::new(OwnHistoryHold::new(Arc::new(AtomicBool::new(false))));
        let sink = Arc::new(RecordingSink {
            inner: LogOnlyForkSink::new(log.clone(), clock.clone()),
            clones: Mutex::new(Vec::new()),
            forks: Mutex::new(Vec::new()),
            ack: Mutex::new(ForkAck::Paused),
            resume: Mutex::new(Vec::new()),
            haves: Mutex::new(Vec::new()),
            branches: Mutex::new(Vec::new()),
            outbox: Mutex::new(ForkOutbox::default()),
            refs: Mutex::new(Vec::new()),
        });
        let status = Arc::new(WorkerStatus::default());
        let store = Arc::new(CountingStore {
            inner: MemorySyncStore::new(),
            clock: clock.clone(),
            calls: Mutex::new(Vec::new()),
            fail: AtomicBool::new(false),
            peer_calls: AtomicUsize::new(0),
            peer_fail: AtomicBool::new(false),
        });
        let (handle, intake, join) = SyncCoordinator::start_with(
            CoordinatorDeps {
                journal,
                sync_store: store.clone(),
                identity: identity(n),
                boot: Uuid::from_u128(u128::from(n)),
                writer: Arc::new(FairWriter::new(8)),
                clock: clock.clone(),
                pacer: pacer.clone(),
                status: status.clone(),
                log,
                fork_sink: sink.clone(),
                own_history: own_history.clone(),
                dial: DialConfig::default(),
                resync_pin,
                cancel: cancel.clone(),
            },
            options,
        );
        Rig {
            store,
            handle,
            intake,
            join,
            cancel,
            clock,
            pacer,
            own_history,
            sink,
            status,
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
        ) -> Result<Vec<StoredEntry>, ApiError> {
            Ok(vec![StoredEntry::from(self.forged.clone())])
        }
        fn serve_branch(
            &self,
            _: &LogKey,
            _: (u64, [u8; 32]),
            _: u64,
            _: BatchLimits,
            _: &CallBudget,
        ) -> Result<Option<Vec<StoredEntry>>, ApiError> {
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
            _: Vec<StoredEntry>,
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

    /// A journal whose `vector()` is replaced: it fails, or lists `many`
    /// logs; everything else is the memory journal's.
    struct VectorStub {
        inner: MemoryJournal,
        stub: Result<Vector, ErrorCode>,
    }

    impl JournalPort for VectorStub {
        fn vector(&self, _: &CallBudget) -> Result<Vector, ApiError> {
            match &self.stub {
                Ok(vector) => Ok(vector.clone()),
                Err(code) => Err(ApiError::new(code.clone(), "stubbed vector failure")),
            }
        }
        fn stored_hash(
            &self,
            log: &LogKey,
            counter: u64,
            budget: &CallBudget,
        ) -> Result<Option<[u8; 32]>, ApiError> {
            self.inner.stored_hash(log, counter, budget)
        }
        fn serve(
            &self,
            log: &LogKey,
            from: u64,
            limits: BatchLimits,
            budget: &CallBudget,
        ) -> Result<Vec<StoredEntry>, ApiError> {
            self.inner.serve(log, from, limits, budget)
        }
        fn serve_branch(
            &self,
            log: &LogKey,
            tip: (u64, [u8; 32]),
            from: u64,
            limits: BatchLimits,
            budget: &CallBudget,
        ) -> Result<Option<Vec<StoredEntry>>, ApiError> {
            self.inner.serve_branch(log, tip, from, limits, budget)
        }
        fn log_meta(
            &self,
            log: &LogKey,
            budget: &CallBudget,
        ) -> Result<Option<crate::sync::journal_port::LogMeta>, ApiError> {
            self.inner.log_meta(log, budget)
        }
        fn ingest(
            &self,
            log: &LogKey,
            entries: Vec<StoredEntry>,
            budget: &CallBudget,
        ) -> Result<IngestOutcome, ApiError> {
            self.inner.ingest(log, entries, budget)
        }
        fn apply_pending(&self, budget: &CallBudget) -> Result<ApplyProgress, ApiError> {
            self.inner.apply_pending(budget)
        }
    }

    #[tokio::test(start_paused = true)]
    async fn shutdown_releases_in_flight_handshake_counts() {
        let rig = start();
        // The remote end never answers: the handshake stays in flight.
        let _d = dial(&rig, 5, "d-host").await;
        until(|| rig.handle.link_count() == 1).await;
        rig.cancel.cancel();
        rig.join.await.unwrap();
        assert_eq!(rig.handle.link_count(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn held_key_lookup_failure_refuses_the_link() {
        let journal = VectorStub {
            inner: MemoryJournal::new(identity(1)),
            stub: Err(ErrorCode::StoreBusy),
        };
        let rig = start_with(Arc::new(journal));
        let peer = peer_hello(2);

        let mut est = connect_inbound(&rig, &peer).await;
        assert_eq!(next_close(&mut est).await, CloseReason::Busy);
        until(|| rig.handle.link_count() == 0).await;

        let mut d = dial(&rig, 5, "d-host").await;
        let mut est = answer_dialed(&mut d.remote, &peer).await;
        assert_eq!(next_close(&mut est).await, CloseReason::Busy);
        match d.report.recv().await.expect("one report") {
            DialReport::LinkEnded { why, peer_node, .. } => {
                assert!(
                    matches!(why, LinkError::Refused(CloseReason::Busy)),
                    "{why:?}"
                );
                assert_eq!(peer_node, None);
            }
            other => panic!("expected only LinkEnded, got {other:?}"),
        }
        assert_eq!(rig.handle.link_count(), 0);
        assert_eq!(snapshot(&rig).await.links.len(), 0, "nothing admitted");
        let lines = rig.lines.lock().unwrap().clone();
        assert!(
            lines
                .iter()
                .any(|l| l.contains("held key lookup") && l.contains("link refused")),
            "{lines:?}"
        );
        rig.cancel.cancel();
        rig.join.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn full_have_truncation_is_logged() {
        let total = MAX_HAVE_LOGS + 1;
        let mut vector = Vector::default();
        for i in 0..total {
            vector.0.insert(
                LogKey {
                    node: NodeUuid::from_u128(0x1000 + i as u128),
                    incarnation: 1,
                },
                crate::sync::LogHead {
                    counter: 1,
                    head_hash: [0; 32],
                    state: crate::sync::LogState::Active,
                    frozen_at: None,
                },
            );
        }
        let journal = VectorStub {
            inner: MemoryJournal::new(identity(1)),
            stub: Ok(vector),
        };
        let rig = start_with(Arc::new(journal));
        let mut b = connect(&rig, 0, &peer_hello(2)).await;
        let (full, _, logs) = b.expect_have().await;
        assert!(full);
        assert_eq!(logs.len(), MAX_HAVE_LOGS);
        let lines = rig.lines.lock().unwrap().clone();
        let wanted = format!("lists {MAX_HAVE_LOGS} of {total} logs");
        assert_eq!(
            lines.iter().filter(|l| l.contains(&wanted)).count(),
            1,
            "{lines:?}"
        );
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
            seat_status_wanted: Arc::default(),
            requests,
            dialed: dialed_tx,
            dial: rig.handle.dial.clone(),
            clock: rig.clock.clone(),
            signals: rig.handle.signals(),
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
            entry: Box::new(StoredEntry::from(
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
            )),
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

        /// The next frame that is neither a link-level `ping` nor a `seen`.
        async fn next(&mut self) -> PeerFrame {
            loop {
                match self.next_raw().await {
                    PeerFrame::Ping { .. } | PeerFrame::Seen { .. } => continue,
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
                    matches!(
                        frame,
                        PeerFrame::Ping { .. } | PeerFrame::Pong { .. } | PeerFrame::Seen { .. }
                    ),
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

    /// [`two_links`] with the fork capability on both.
    async fn two_fork_links(rig: &Rig) -> (TestPeer, TestPeer) {
        let mut first = connect(rig, 0, &fork_hello(2)).await;
        first.expect_have().await;
        let mut second = connect(rig, 1, &fork_hello(2)).await;
        listed(rig, 2).await;
        let order = expected_order(&[&first, &second]);
        if order[0] == 0 {
            second.quiet().await;
            (first, second)
        } else {
            second.expect_have().await;
            first.quiet().await;
            (second, first)
        }
    }

    // -- Serve resource bounds (ht-5fr.41) ----------------------------------

    fn want_of(i: u128) -> PeerFrame {
        PeerFrame::Want {
            node: NodeUuid::from_u128(0x1000 + i),
            inc: 1,
            from: 1,
        }
    }

    #[tokio::test(start_paused = true)]
    async fn want_flood_is_capped() {
        let (rig, _journal) = node(1, 0, 0);
        let mut peer = connect(&rig, 0, &peer_hello(2)).await;
        peer.expect_have().await;
        for i in 0..5_000u128 {
            peer.send(want_of(i)).await;
        }
        peer.pump(500).await;
        let snap = snapshot(&rig).await;
        assert_eq!(snap.links[0].subscriptions, 4_096, "capped, still up");
        assert!(
            rig.lines
                .lock()
                .unwrap()
                .iter()
                .filter(|l| l.contains("wants more than 4096 logs"))
                .count()
                == 1,
            "logged once: {:?}",
            rig.lines.lock().unwrap()
        );
        // An already armed log is re-armed, and the coordinator answers
        // (after the bucket refilled).
        peer.pump(2_000).await;
        peer.send(want_of(0)).await;
        peer.send(PeerFrame::Unwant {
            node: NodeUuid::from_u128(0x1000),
            inc: 1,
        })
        .await;
        peer.pump(500).await;
        assert_eq!(snapshot(&rig).await.links[0].subscriptions, 4_095);
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn inbound_control_rate_is_throttled() {
        let (rig, _journal) = node(1, 0, 0);
        let mut peer = connect(&rig, 0, &peer_hello(2)).await;
        peer.expect_have().await;
        // The bucket holds 4 160 frames; the rest of a burst is dropped.
        for _ in 0..4_300 {
            peer.send(want_of(0)).await;
        }
        peer.pump(100).await;
        let throttled = snapshot(&rig).await.links[0].throttled;
        assert!(
            (100..1_000).contains(&throttled),
            "{throttled} frames throttled"
        );
        // Far past 10 000 drops the link is closed.
        for _ in 0..12_000 {
            peer.send(want_of(0)).await;
        }
        assert_eq!(peer.expect_close().await, CloseReason::ProtocolError);
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn serve_job_deadline_fires() {
        let (rig, journal) = node(1, 2, 0);
        let own = journal.inner.self_log();
        let mut peer = connect(&rig, 0, &peer_hello(2)).await;
        peer.expect_have().await;
        // The job is held in the journal; it carries a finite deadline, and
        // when that runs out it fails like any serve error.
        journal.serve_gate.store(true, Ordering::SeqCst);
        journal
            .serve_script
            .lock()
            .unwrap()
            .push_back(ErrorCode::DeadlineExceeded);
        let before = rig.clock.monotonic_now().0;
        peer.send(PeerFrame::Want {
            node: own.node,
            inc: own.incarnation,
            from: 1,
        })
        .await;
        // Real-time poll: paused time does not advance while a blocking job
        // is in flight.
        for _ in 0..5_000 {
            if !journal.serve_deadlines.lock().unwrap().is_empty() {
                break;
            }
            tokio::task::yield_now().await;
            std::thread::sleep(Duration::from_millis(1));
        }
        let after = rig.clock.monotonic_now().0;
        let deadline = journal.serve_deadlines.lock().unwrap()[0].0;
        assert!(
            (before + 5_000..=after + 5_000).contains(&deadline),
            "deadline {deadline} is not now + 5 s ({before}..{after})"
        );
        journal.serve_gate.store(false, Ordering::SeqCst);
        // The failed job ends; the next tick pushes the subscription again.
        let (log, counters) = peer.next_entries().await;
        assert_eq!(log, own);
        assert_eq!(counters, [1, 2, 3]);
        assert_eq!(journal.serve_deadlines.lock().unwrap().len(), 2);
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn serve_error_is_retried_on_the_tick() {
        let (rig, journal) = node(1, 2, 0);
        let own = journal.inner.self_log();
        let mut peer = connect(&rig, 0, &peer_hello(2)).await;
        peer.expect_have().await;
        journal
            .serve_script
            .lock()
            .unwrap()
            .push_back(ErrorCode::StoreBusy);
        peer.send(PeerFrame::Want {
            node: own.node,
            inc: own.incarnation,
            from: 1,
        })
        .await;
        until(|| journal.serve_deadlines.lock().unwrap().len() == 1).await;
        let started = tokio::time::Instant::now();
        let (_, counters) = peer.next_entries().await;
        assert_eq!(counters, [1, 2, 3]);
        assert!(
            started.elapsed() <= Duration::from_millis(1_500),
            "retried within a tick: {:?}",
            started.elapsed()
        );
        assert_eq!(
            journal.serve_deadlines.lock().unwrap().len(),
            2,
            "one failed call, one retry, no hot loop"
        );
        stop(rig).await;
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
    async fn sole_stalled_source_stays_the_rx_source() {
        let (rig, _journal) = rig_with_self_log();
        let mut b = connect(&rig, 0, &peer_hello(2)).await;
        b.expect_have().await;
        let log_node = node_of(9);
        b.send_have(true, false, vec![have_log(log_node, 1, 10)])
            .await;
        b.expect_want(log_node, 1, 1).await;
        tokio::time::sleep(Duration::from_secs(31)).await;
        b.expect_want(log_node, 1, 1).await;

        // The next reselect must not see a penalty and drop the source.
        b.send_have(true, false, vec![have_log(log_node, 1, 10)])
            .await;
        b.quiet().await;
        assert_eq!(
            snapshot(&rig).await.links[0].rx_logs,
            1,
            "still the rx source"
        );

        // It is judged again only after another stall window.
        tokio::time::sleep(Duration::from_secs(31)).await;
        b.expect_want(log_node, 1, 1).await;
        rig.cancel.cancel();
        rig.join.await.unwrap();
    }

    fn pinned_rig(logs: Option<Vec<NodeUuid>>) -> Rig {
        let journal = Arc::new(MemoryJournal::new(identity(1)));
        journal.bootstrap();
        start_node_pinned(
            1,
            journal,
            Some(ResyncPin {
                from: node_of(2),
                logs,
            }),
        )
    }

    #[tokio::test(start_paused = true)]
    async fn resync_pin_restricts_candidates_until_round_completes() {
        let rig = pinned_rig(None);
        let mut b = connect(&rig, 0, &peer_hello(2)).await;
        b.expect_have().await;
        let mut c = connect(&rig, 1, &peer_hello(3)).await;
        c.expect_have().await;
        let log_node = node_of(9);

        // C alone advertises the log; B, the pinned peer, has not spoken.
        c.send_have(true, false, vec![have_log(log_node, 1, 5)])
            .await;
        c.quiet().await;
        b.quiet().await;

        // B's full have advertises nothing wantable: its round completes,
        // the pin lifts and C is wanted.
        b.send_have(true, false, Vec::new()).await;
        c.expect_want(log_node, 1, 1).await;
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn resync_pin_wants_from_the_pinned_peer_despite_a_higher_counter() {
        let rig = pinned_rig(None);
        let mut b = connect(&rig, 0, &peer_hello(2)).await;
        b.expect_have().await;
        let mut c = connect(&rig, 1, &peer_hello(3)).await;
        c.expect_have().await;
        let log_node = node_of(9);
        c.send_have(true, false, vec![have_log(log_node, 1, 50)])
            .await;
        b.send_have(true, false, vec![have_log(log_node, 1, 10)])
            .await;
        b.expect_want(log_node, 1, 1).await;
        c.quiet().await;
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn resync_pin_leaves_logs_of_other_nodes_free() {
        // Only the logs of node 8 are pinned; node 9's log comes from C.
        let rig = pinned_rig(Some(vec![node_of(8)]));
        let mut b = connect(&rig, 0, &peer_hello(2)).await;
        b.expect_have().await;
        let mut c = connect(&rig, 1, &peer_hello(3)).await;
        c.expect_have().await;
        c.send_have(
            true,
            false,
            vec![have_log(node_of(9), 1, 5), have_log(node_of(8), 1, 5)],
        )
        .await;
        c.expect_want(node_of(9), 1, 1).await;
        c.quiet().await;
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn resync_pin_lifts_after_10_min() {
        let rig = pinned_rig(None);
        let mut b = connect(&rig, 0, &peer_hello(2)).await;
        b.expect_have().await;
        let mut c = connect(&rig, 1, &peer_hello(3)).await;
        c.expect_have().await;
        let log_node = node_of(9);
        c.send_have(true, false, vec![have_log(log_node, 1, 5)])
            .await;
        // B never completes a round; C stays unwanted for the whole bound.
        // Both links answer pings, so only the pin holds C back.
        let wanted = |frames: &[PeerFrame]| {
            frames
                .iter()
                .any(|f| matches!(f, PeerFrame::Want { node, .. } if *node == log_node))
        };
        for _ in 0..1_160 {
            let (fb, fc) = (b.pump(250).await, c.pump(250).await);
            assert!(!wanted(&fb) && !wanted(&fc), "no want before the pin lifts");
        }
        let mut lifted = false;
        for _ in 0..100 {
            let (fb, fc) = (b.pump(250).await, c.pump(250).await);
            assert!(!wanted(&fb), "B does not advertise the log");
            if wanted(&fc) {
                lifted = true;
                break;
            }
        }
        assert!(lifted, "C is wanted once 10 minutes passed");
        stop(rig).await;
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

    /// The first `have` on `peer`, skipping the entries a `want` was served with.
    async fn first_have(peer: &mut TestPeer) -> (bool, bool, Vec<HaveLog>) {
        loop {
            match peer.next().await {
                PeerFrame::Have {
                    full, ask, logs, ..
                } => return (full, ask, logs),
                PeerFrame::Entries { .. } | PeerFrame::BranchEntries { .. } => continue,
                other => panic!("expected have, got {other:?}"),
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn want_on_standby_promotes_it_and_asks_for_the_peer_vector() {
        let (rig, journal) = rig_with_self_log();
        let own = journal.self_log();
        let (mut active, mut standby) = two_links(&rig).await;
        standby
            .send(PeerFrame::Want {
                node: own.node,
                inc: own.incarnation,
                from: 1,
            })
            .await;
        let (full, ask, _) = first_have(&mut standby).await;
        assert!(full && ask, "a want carries no vector: the promotion asks");
        active.quiet().await;
        let snap = snapshot(&rig).await;
        let modes: Vec<LinkMode> = snap.links.iter().map(|l| l.mode).collect();
        let mut expected = [LinkMode::Standby, LinkMode::Standby];
        expected[standby.idx] = LinkMode::Active;
        assert_eq!(modes, expected);
        let log_node = node_of(9);
        standby
            .send_have(true, false, vec![have_log(log_node, 1, 5)])
            .await;
        // The entries the first want was served may still be in flight.
        loop {
            match standby.next().await {
                PeerFrame::Entries { .. } | PeerFrame::BranchEntries { .. } => continue,
                PeerFrame::Want { node, inc, from } => {
                    assert_eq!((node, inc, from), (log_node, 1, 1));
                    break;
                }
                other => panic!("expected want, got {other:?}"),
            }
        }
        rig.cancel.cancel();
        rig.join.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn delta_have_on_standby_asks_for_the_peer_vector() {
        let (rig, _journal) = rig_with_self_log();
        let (_active, mut standby) = two_links(&rig).await;
        standby
            .send_have(false, false, vec![have_log(node_of(9), 1, 5)])
            .await;
        let (full, ask, _) = first_have(&mut standby).await;
        assert!(full && ask, "a delta is not the vector: the promotion asks");
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
        loop {
            match older.next_raw().await {
                PeerFrame::Seen { .. } => continue,
                PeerFrame::Ping { n } => {
                    older.send(PeerFrame::Pong { n }).await;
                    break;
                }
                other => panic!("expected the probe ping, got {other:?}"),
            }
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
                Ok(Some(
                    PeerFrame::Ping { .. } | PeerFrame::Seen { .. } | PeerFrame::Close { .. },
                )) => continue,
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

    // Kills: a Links hold that an unsigned `Have` can keep on with an own log
    // the node cannot be served (no genesis), and a window that never ends or
    // never starts.
    #[tokio::test(start_paused = true)]
    async fn unproven_own_log_claim_lifts_the_hold() {
        let (rig, journal) = rig_with_self_log();
        let own = journal.self_log();
        let mut peer = connect(&rig, 0, &peer_hello(2)).await;
        peer.expect_have().await;
        assert!(!hold_on(&rig));
        // An own incarnation this node has never held, counter 50; the peer
        // never serves its genesis.
        peer.send_have(true, false, vec![have_log(own.node, 9, 50)])
            .await;
        until(|| hold_on(&rig)).await;
        tokio::time::sleep(Duration::from_secs(9)).await;
        assert!(hold_on(&rig), "inside the 10 s proof window");
        tokio::time::sleep(Duration::from_secs(3)).await;
        assert!(!hold_on(&rig), "the unproven claim no longer holds");
        let ignored = rig
            .lines
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.contains("advertises own log 9 it cannot serve; ignored"))
            .count();
        assert_eq!(ignored, 1, "logged once");
        // The same claim restated does not reopen the window.
        peer.send_have(false, false, vec![have_log(own.node, 9, 51)])
            .await;
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert!(!hold_on(&rig));
        rig.cancel.cancel();
        rig.join.await.unwrap();
    }

    // An unproven claim stays expired when the peer omits the log from a
    // full Have and restates it later.
    #[tokio::test(start_paused = true)]
    async fn omitted_then_restated_own_log_claim_stays_expired() {
        let (rig, journal) = rig_with_self_log();
        let own = journal.self_log();
        let mut peer = connect(&rig, 0, &peer_hello(2)).await;
        peer.expect_have().await;
        peer.send_have(true, false, vec![have_log(own.node, 9, 50)])
            .await;
        until(|| hold_on(&rig)).await;
        tokio::time::sleep(Duration::from_secs(12)).await;
        assert!(!hold_on(&rig));
        // A full Have without the log, then a full Have restating it.
        peer.send_have(true, false, vec![]).await;
        tokio::time::sleep(Duration::from_secs(1)).await;
        peer.send_have(true, false, vec![have_log(own.node, 9, 60)])
            .await;
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert!(!hold_on(&rig), "restating does not reopen the window");
        rig.cancel.cancel();
        rig.join.await.unwrap();
    }

    // The accepted residual: a peer that really holds one of this node's own
    // logs can overstate its counter; the hold lasts until the 5 min cap.
    #[tokio::test(start_paused = true)]
    async fn held_own_log_claim_holds_until_the_cap() {
        let (rig, journal) = rig_with_self_log();
        let own = journal.self_log();
        let mut peer = connect(&rig, 0, &peer_hello(2)).await;
        peer.expect_have().await;
        peer.send_have(true, false, vec![have_log(own.node, own.incarnation, 500)])
            .await;
        until(|| hold_on(&rig)).await;
        // The peer stays alive with pings; it never serves the entries.
        for n in 0..5 {
            tokio::time::sleep(Duration::from_secs(30)).await;
            peer.send(PeerFrame::Ping { n }).await;
        }
        assert!(hold_on(&rig), "a held log's claim is not a proof window");
        for n in 5..11 {
            tokio::time::sleep(Duration::from_secs(30)).await;
            peer.send(PeerFrame::Ping { n }).await;
        }
        assert!(!hold_on(&rig), "the 5 min cap ends it");
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

    // -----------------------------------------------------------------------
    // Serving, live tail, ingest outcomes and Have deltas (ht-5fr.5.13)
    // -----------------------------------------------------------------------

    use std::collections::VecDeque;

    fn open_budget() -> CallBudget {
        CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: Cancellation::default(),
        }
    }

    /// What `TestJournal::ingest` answers instead of ingesting.
    enum Scripted {
        Outcome(IngestOutcome),
        Fail,
    }

    /// A `MemoryJournal` that counts what it stores and can answer `ingest`
    /// from a script (headroom, a fork, a store error) before it falls back
    /// to the real chain rules.
    struct TestJournal {
        inner: MemoryJournal,
        script: Mutex<VecDeque<Scripted>>,
        stored: AtomicUsize,
        ingests: AtomicUsize,
        /// While set, `serve_branch` waits (holds requests outstanding).
        branch_gate: AtomicBool,
        /// While set, `serve` waits.
        serve_gate: AtomicBool,
        /// Errors the next `serve` calls return, front first.
        serve_script: Mutex<VecDeque<ErrorCode>>,
        /// The budget deadline of every `serve` call.
        serve_deadlines: Mutex<Vec<MonoInstant>>,
        /// The `from` counter of every `serve` call.
        serve_froms: Mutex<Vec<u64>>,
    }

    impl TestJournal {
        fn new(n: u8) -> Arc<Self> {
            Arc::new(Self {
                inner: MemoryJournal::new(identity(n)),
                script: Mutex::default(),
                stored: AtomicUsize::new(0),
                ingests: AtomicUsize::new(0),
                branch_gate: AtomicBool::new(false),
                serve_gate: AtomicBool::new(false),
                serve_script: Mutex::default(),
                serve_deadlines: Mutex::default(),
                serve_froms: Mutex::default(),
            })
        }

        fn script(&self, step: Scripted) {
            self.script.lock().unwrap().push_back(step);
        }

        fn view(&self) -> Vector {
            self.inner.vector(&open_budget()).unwrap()
        }
    }

    impl JournalPort for TestJournal {
        fn vector(&self, budget: &CallBudget) -> Result<Vector, ApiError> {
            self.inner.vector(budget)
        }
        fn stored_hash(
            &self,
            log: &LogKey,
            counter: u64,
            budget: &CallBudget,
        ) -> Result<Option<[u8; 32]>, ApiError> {
            self.inner.stored_hash(log, counter, budget)
        }
        fn serve(
            &self,
            log: &LogKey,
            from: u64,
            limits: BatchLimits,
            budget: &CallBudget,
        ) -> Result<Vec<StoredEntry>, ApiError> {
            self.serve_deadlines.lock().unwrap().push(budget.deadline);
            self.serve_froms.lock().unwrap().push(from);
            while self.serve_gate.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(1));
            }
            if let Some(code) = self.serve_script.lock().unwrap().pop_front() {
                return Err(ApiError::new(code, "scripted serve error"));
            }
            self.inner.serve(log, from, limits, budget)
        }
        fn serve_branch(
            &self,
            log: &LogKey,
            tip: (u64, [u8; 32]),
            from: u64,
            limits: BatchLimits,
            budget: &CallBudget,
        ) -> Result<Option<Vec<StoredEntry>>, ApiError> {
            while self.branch_gate.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(1));
            }
            self.inner.serve_branch(log, tip, from, limits, budget)
        }
        fn log_meta(
            &self,
            log: &LogKey,
            budget: &CallBudget,
        ) -> Result<Option<crate::sync::journal_port::LogMeta>, ApiError> {
            self.inner.log_meta(log, budget)
        }
        fn ingest(
            &self,
            log: &LogKey,
            entries: Vec<StoredEntry>,
            budget: &CallBudget,
        ) -> Result<IngestOutcome, ApiError> {
            self.ingests.fetch_add(1, Ordering::SeqCst);
            let scripted = self.script.lock().unwrap().pop_front();
            match scripted {
                Some(Scripted::Outcome(outcome)) => return Ok(outcome),
                Some(Scripted::Fail) => {
                    return Err(ApiError::new(ErrorCode::StoreBusy, "scripted store error"));
                }
                None => {}
            }
            let outcome = self.inner.ingest(log, entries, budget)?;
            if let IngestOutcome::Stored { stored, .. } = &outcome {
                self.stored.fetch_add(*stored, Ordering::SeqCst);
            }
            Ok(outcome)
        }
        fn apply_pending(&self, budget: &CallBudget) -> Result<ApplyProgress, ApiError> {
            self.inner.apply_pending(budget)
        }
    }

    fn payload(i: usize, pad: usize) -> Vec<u8> {
        format!("{{\"i\":{i},\"pad\":\"{}\"}}", "x".repeat(pad)).into_bytes()
    }

    /// A node whose own log holds a genesis and `entries` more entries.
    fn node(n: u8, entries: usize, pad: usize) -> (Rig, Arc<TestJournal>) {
        let journal = TestJournal::new(n);
        journal.inner.bootstrap();
        for i in 0..entries {
            journal
                .inner
                .author_raw("test.kind", payload(i, pad))
                .unwrap();
        }
        (start_node(n, journal.clone()), journal)
    }

    async fn stop(rig: Rig) {
        rig.cancel.cancel();
        rig.join.await.unwrap();
    }

    /// `a` dials `b` over a duplex, the way the Dialer and the transport
    /// would (the first frame is read off before the offer).
    async fn link_nodes(a: &Rig, b: &Rig) -> mpsc::Receiver<DialReport> {
        let (ours, mut theirs) = duplex(BIG);
        let (stdout, stdin) = tokio::io::split(ours);
        let (report_tx, report) = mpsc::channel(DIAL_REPORT_SLOTS);
        a.handle
            .hand_off_dialed(DialedStream {
                target_id: TargetId(1),
                dest: "b".into(),
                stdout: Box::new(stdout),
                stdin: Box::new(stdin),
                report: report_tx,
            })
            .await
            .ok()
            .expect("the actor is running");
        let first = transport::read_frame_with_limit(&mut theirs, MAX_FRAME_BYTES)
            .await
            .unwrap();
        b.intake
            .offer(PeerStream {
                stream: Box::new(theirs),
                first_frame: first,
                permit: Box::new(()),
            })
            .ok()
            .expect("intake has room");
        report
    }

    /// Wait until every journal shows the same, non-empty vector.
    async fn converge(journals: &[&Arc<TestJournal>]) {
        for _ in 0..1200 {
            let first = journals[0].view();
            if !first.0.is_empty() && journals.iter().all(|j| j.view() == first) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!(
            "no convergence: {:?}",
            journals
                .iter()
                .map(|j| j.view().0.values().map(|h| h.counter).collect::<Vec<_>>())
                .collect::<Vec<_>>()
        );
    }

    /// Another node's log with `extra` entries after its genesis, held by no
    /// rig: the source of valid `entries` frames for a manual peer to send.
    struct Foreign {
        log: LogKey,
        envs: Vec<Envelope>,
    }

    fn foreign(n: u8, extra: usize) -> Foreign {
        let journal = MemoryJournal::new(identity(n));
        journal.bootstrap();
        for i in 0..extra {
            journal.author_raw("test.kind", payload(i, 0)).unwrap();
        }
        let log = journal.self_log();
        let envs = journal
            .serve(&log, 1, BatchLimits::default(), &open_budget())
            .unwrap()
            .into_iter()
            .map(|entry| entry.as_known().cloned().expect("a known entry"))
            .collect();
        Foreign { log, envs }
    }

    fn entries_frame(log: &LogKey, envs: &[Envelope]) -> PeerFrame {
        PeerFrame::Entries {
            node: log.node,
            inc: log.incarnation,
            entries: envs.iter().map(WireItem::from).collect(),
        }
    }

    fn wire_len(frame: &PeerFrame) -> u32 {
        u32::try_from(frame.encode().unwrap().len()).unwrap()
    }

    fn open_vector_counter(journal: &TestJournal, log: &LogKey) -> u64 {
        journal.view().0.get(log).map_or(0, |h| h.counter)
    }

    impl TestPeer {
        /// Let `ms` of virtual time pass in short steps, answering the
        /// coordinator's pings so the link stays alive; returns every other
        /// frame that arrived.
        async fn pump(&mut self, ms: u64) -> Vec<PeerFrame> {
            let mut out = Vec::new();
            let mut left = ms;
            while left > 0 {
                let step = left.min(250);
                tokio::time::sleep(Duration::from_millis(step)).await;
                left -= step;
                while let Ok(frame) = self.frames.try_recv() {
                    match frame {
                        PeerFrame::Ping { n } => self.send(PeerFrame::Pong { n }).await,
                        other => out.push(other),
                    }
                }
            }
            out
        }

        /// The next frame `pick` accepts; everything else is skipped.
        async fn find<T>(&mut self, mut pick: impl FnMut(&PeerFrame) -> Option<T>) -> T {
            loop {
                let frame = self.next().await;
                if let Some(found) = pick(&frame) {
                    return found;
                }
            }
        }

        async fn find_want(&mut self, node: NodeUuid, inc: u32, from: u64) {
            self.find(|f| match f {
                PeerFrame::Want {
                    node: n,
                    inc: i,
                    from: fr,
                } if (*n, *i, *fr) == (node, inc, from) => Some(()),
                _ => None,
            })
            .await;
        }

        async fn find_unwant(&mut self, node: NodeUuid, inc: u32) {
            self.find(|f| match f {
                PeerFrame::Unwant { node: n, inc: i } if (*n, *i) == (node, inc) => Some(()),
                _ => None,
            })
            .await;
        }

        async fn find_credit(&mut self) -> u32 {
            self.find(|f| match f {
                PeerFrame::Credit { bytes } => Some(*bytes),
                _ => None,
            })
            .await
        }

        /// Counters of the next `entries` frame.
        async fn next_entries(&mut self) -> (LogKey, Vec<u64>) {
            self.find(|f| match f {
                PeerFrame::Entries { node, inc, entries } => Some((
                    LogKey {
                        node: *node,
                        incarnation: *inc,
                    },
                    entries.iter().map(WireItem::counter).collect(),
                )),
                _ => None,
            })
            .await
        }
    }

    fn wants_in(frames: &[PeerFrame]) -> Vec<(NodeUuid, u32, u64)> {
        frames
            .iter()
            .filter_map(|f| match f {
                PeerFrame::Want { node, inc, from } => Some((*node, *inc, *from)),
                _ => None,
            })
            .collect()
    }

    #[tokio::test(start_paused = true)]
    async fn two_nodes_converge() {
        let (a, ja) = node(1, 3, 0);
        let (b, jb) = node(2, 2, 0);
        let _report = link_nodes(&a, &b).await;
        converge(&[&ja, &jb]).await;
        let vector = ja.view();
        assert_eq!(vector.0.len(), 2, "each node holds both logs");
        let mut counters: Vec<u64> = vector.0.values().map(|h| h.counter).collect();
        counters.sort_unstable();
        assert_eq!(counters, [3, 4], "genesis plus the authored entries");
        // The pulled entries are the served ones, byte for byte.
        let log_a = ja.inner.self_log();
        let on_a = ja
            .inner
            .serve(&log_a, 1, BatchLimits::default(), &open_budget());
        let on_b = jb
            .inner
            .serve(&log_a, 1, BatchLimits::default(), &open_budget());
        assert_eq!(on_a.unwrap(), on_b.unwrap());
        stop(a).await;
        stop(b).await;
    }

    #[tokio::test(start_paused = true)]
    async fn line_relays_and_live_tail_reaches_c_without_new_want() {
        let (a, ja) = node(1, 2, 0);
        let jb = TestJournal::new(2);
        let b = start_node(2, jb.clone());
        let _report = link_nodes(&a, &b).await;
        // C is a manual peer on B that asks for A's log exactly once, before
        // B holds it: the want stays armed.
        let mut c = connect(&b, 0, &peer_hello(3)).await;
        c.expect_have().await;
        let log_a = ja.inner.self_log();
        c.send(PeerFrame::Want {
            node: log_a.node,
            inc: log_a.incarnation,
            from: 1,
        })
        .await;
        let mut seen = Vec::new();
        while seen.len() < 3 {
            let (log, counters) = c.next_entries().await;
            assert_eq!(log, log_a);
            seen.extend(counters);
        }
        assert_eq!(seen, [1, 2, 3], "relayed in order from B's copy");

        // An entry authored on A after convergence reaches C through the
        // live tail: C sends nothing more.
        converge(&[&ja, &jb]).await;
        ja.inner.author_raw("test.kind", payload(9, 0)).unwrap();
        a.pacer.kick();
        let (log, counters) = c.next_entries().await;
        assert_eq!((log, counters), (log_a, vec![4]));
        assert_eq!(open_vector_counter(&jb, &log_a), 4);
        stop(a).await;
        stop(b).await;
    }

    #[tokio::test(start_paused = true)]
    async fn triangle_ingests_each_entry_once_per_node() {
        let (a, ja) = node(1, 2, 0);
        let (b, jb) = node(2, 2, 0);
        let (c, jc) = node(3, 2, 0);
        let _r1 = link_nodes(&a, &b).await;
        let _r2 = link_nodes(&b, &c).await;
        let _r3 = link_nodes(&a, &c).await;
        converge(&[&ja, &jb, &jc]).await;
        // Let any late duplicate traffic settle, then count.
        tokio::time::sleep(Duration::from_secs(5)).await;
        converge(&[&ja, &jb, &jc]).await;
        for (name, journal) in [("a", &ja), ("b", &jb), ("c", &jc)] {
            assert_eq!(
                journal.stored.load(Ordering::SeqCst),
                6,
                "node {name} stores the two foreign logs' 3 entries each exactly once"
            );
        }
        stop(a).await;
        stop(b).await;
        stop(c).await;
    }

    #[tokio::test(start_paused = true)]
    async fn oversized_want_served_in_halved_frames_within_credit() {
        const MAX_FRAME: u32 = 4_096;
        const CREDIT: u32 = 8_192;
        let (rig, journal) = node(1, 12, 700);
        let own = journal.inner.self_log();
        let mut hello = peer_hello(2);
        hello.limits.max_frame = MAX_FRAME;
        hello.limits.credit = CREDIT;
        let mut peer = connect(&rig, 0, &hello).await;
        peer.expect_have().await;
        peer.send(PeerFrame::Want {
            node: own.node,
            inc: own.incarnation,
            from: 1,
        })
        .await;

        // Without credit back, no more than the window arrives.
        let mut frames: Vec<PeerFrame> = Vec::new();
        for frame in peer.pump(1_000).await {
            if matches!(frame, PeerFrame::Entries { .. }) {
                frames.push(frame);
            }
        }
        let first_wave: u32 = frames.iter().map(wire_len).sum();
        assert!(frames.len() >= 2, "the batch was split: {}", frames.len());
        assert!(
            first_wave <= CREDIT,
            "{first_wave} bytes outstanding exceed the {CREDIT} window"
        );
        // Every frame fits `max_frame`; granting the credit back releases the
        // rest, in order and complete.
        let mut counters: Vec<u64> = Vec::new();
        let mut granted = 0u32;
        for _ in 0..200 {
            for frame in &frames[granted as usize..] {
                assert!(wire_len(frame) <= MAX_FRAME, "frame over max_frame");
                if let PeerFrame::Entries { entries, .. } = frame {
                    counters.extend(entries.iter().map(WireItem::counter));
                }
                peer.send(PeerFrame::Credit {
                    bytes: wire_len(frame),
                })
                .await;
            }
            granted = u32::try_from(frames.len()).unwrap();
            if counters.len() >= 13 {
                break;
            }
            for frame in peer.pump(250).await {
                if matches!(frame, PeerFrame::Entries { .. }) {
                    frames.push(frame);
                }
            }
        }
        assert_eq!(counters, (1..=13).collect::<Vec<u64>>());
        assert!(frames.len() >= 4, "{} frames", frames.len());
        stop(rig).await;
    }

    /// A node holding a foreign log `[known 1, known 2, opaque 3, known 4]`,
    /// the log, and the opaque entry's counter.
    fn node_with_opaque_log() -> (Rig, Arc<TestJournal>, LogKey) {
        let (rig, journal) = node(1, 0, 0);
        let log = ingest_opaque_log(&journal.inner, 9);
        rig.pacer.kick();
        (rig, journal, log)
    }

    /// Ingests the foreign log `[known 1, 2, opaque 3, known 4]` of `seed`.
    fn ingest_opaque_log(journal: &MemoryJournal, seed: u8) -> LogKey {
        use crate::test_support::opaque_entry::{synthetic_entry, synthetic_unsigned};
        let f = foreign(seed, 1);
        let signer = identity(seed);
        let opaque = synthetic_entry(
            &signer,
            &synthetic_unsigned(&signer, f.log.incarnation, 3, f.envs[1].hash),
        );
        let four = synthetic_unsigned(&signer, f.log.incarnation, 4, opaque.hash()).seal(&signer);
        let mut batch: Vec<StoredEntry> = f.envs.iter().cloned().map(StoredEntry::from).collect();
        batch.push(StoredEntry::from(opaque));
        batch.push(StoredEntry::from(four));
        let outcome = journal.ingest(&f.log, batch, &open_budget()).unwrap();
        assert!(
            matches!(outcome, IngestOutcome::Stored { stored: 4, .. }),
            "{outcome:?}"
        );
        f.log
    }

    /// A node holding a foreign log `[known 1, 2, 3, opaque 4]` (or, with
    /// `opaque_first`, `[known 1, 2, opaque 3, known 4]`): the log and its
    /// head `(counter, hash)`.
    fn node_with_branch(opaque_first: bool) -> (Rig, Arc<TestJournal>, LogKey, (u64, [u8; 32])) {
        use crate::test_support::opaque_entry::{synthetic_entry, synthetic_unsigned};
        let (rig, journal) = node(1, 0, 0);
        let f = foreign(9, 1);
        let signer = identity(9);
        let inc = f.log.incarnation;
        let mut batch: Vec<StoredEntry> = f.envs.iter().cloned().map(StoredEntry::from).collect();
        let head = if opaque_first {
            let opaque = synthetic_entry(
                &signer,
                &synthetic_unsigned(&signer, inc, 3, f.envs[1].hash),
            );
            let four = synthetic_unsigned(&signer, inc, 4, opaque.hash()).seal(&signer);
            batch.push(StoredEntry::from(opaque));
            batch.push(StoredEntry::from(four.clone()));
            (4, four.hash)
        } else {
            let three = synthetic_unsigned(&signer, inc, 3, f.envs[1].hash).seal(&signer);
            let opaque = synthetic_entry(&signer, &synthetic_unsigned(&signer, inc, 4, three.hash));
            batch.push(StoredEntry::from(three));
            batch.push(StoredEntry::from(opaque.clone()));
            (4, opaque.hash())
        };
        let outcome = journal.inner.ingest(&f.log, batch, &open_budget()).unwrap();
        assert!(
            matches!(outcome, IngestOutcome::Stored { stored: 4, .. }),
            "{outcome:?}"
        );
        (rig, journal, f.log, head)
    }

    fn branch_want_of(log: LogKey, head: (u64, [u8; 32]), from: u64) -> BranchWant {
        BranchWant {
            node: log.node,
            inc: log.incarnation,
            tip: crate::sync::frame::TipRef {
                counter: head.0,
                hash: hex_encode(&head.1),
            },
            from,
        }
    }

    // A branch that ends in an opaque entry is served up to the opaque one:
    // the known prefix, `more: false` (nothing further can follow it here).
    #[tokio::test(start_paused = true)]
    async fn branch_entries_cut_before_opaque_toward_peer() {
        let (rig, _journal, log, head) = node_with_branch(false);
        let mut peer = connect(&rig, 0, &fork_hello(2)).await;
        peer.expect_have().await;
        let want = branch_want_of(log, head, 3);
        peer.send(PeerFrame::BranchWant(want.clone())).await;
        let (tip_hash, counters, more) = next_branch_entries(&mut peer).await;
        assert_eq!(tip_hash, want.tip.hash);
        assert_eq!(counters, [3], "the known prefix of [3, opaque 4]");
        assert!(!more);
        stop(rig).await;
    }

    // A branch whose first entry is opaque has no known prefix: `branch_none`.
    #[tokio::test(start_paused = true)]
    async fn branch_all_opaque_answers_branch_none() {
        let (rig, _journal, log, head) = node_with_branch(true);
        let mut peer = connect(&rig, 0, &fork_hello(2)).await;
        peer.expect_have().await;
        let want = branch_want_of(log, head, 3);
        peer.send(PeerFrame::BranchWant(want.clone())).await;
        let none = peer
            .find(|f| match f {
                PeerFrame::BranchNone { tip_hash, .. } => Some(tip_hash.clone()),
                PeerFrame::BranchEntries { .. } => panic!("an opaque entry was served"),
                _ => None,
            })
            .await;
        assert_eq!(none, want.tip.hash);
        stop(rig).await;
    }

    fn entry_counters(frames: &[PeerFrame]) -> Vec<u64> {
        frames
            .iter()
            .filter_map(|f| match f {
                PeerFrame::Entries { entries, .. } => Some(entries.iter().map(WireItem::counter)),
                _ => None,
            })
            .flatten()
            .collect()
    }

    // The serve-side cut before the first opaque entry is interim: ht-j87z.4's
    // capability gate owns what a link may be served.
    #[tokio::test(start_paused = true)]
    async fn serve_stops_before_opaque_entry() {
        let (rig, journal, log) = node_with_opaque_log();
        let mut peer = connect(&rig, 0, &peer_hello(2)).await;
        peer.expect_have().await;
        peer.send(PeerFrame::Want {
            node: log.node,
            inc: log.incarnation,
            from: 2,
        })
        .await;
        let frames = peer.pump(2_000).await;
        assert_eq!(
            entry_counters(&frames),
            [2],
            "the known prefix, not past the opaque 3: {frames:?}"
        );
        // The cursor moved past what was sent: the next read starts at 3.
        let froms = journal.serve_froms.lock().unwrap().clone();
        assert_eq!(froms.first(), Some(&2), "{froms:?}");
        assert!(froms.contains(&3), "{froms:?}");
        stop(rig).await;
    }

    // The gate parks a log whose whole batch is cut: capabilities are fixed
    // per link, so reading it again would only repeat the cut.
    #[tokio::test(start_paused = true)]
    async fn all_opaque_batch_sends_nothing_and_parks_the_log() {
        let (rig, journal, log) = node_with_opaque_log();
        let mut peer = connect(&rig, 0, &peer_hello(2)).await;
        peer.expect_have().await;
        peer.send(PeerFrame::Want {
            node: log.node,
            inc: log.incarnation,
            from: 3,
        })
        .await;
        let frames = peer.pump(2_000).await;
        assert_eq!(entry_counters(&frames), Vec::<u64>::new(), "{frames:?}");
        let reads = journal.serve_froms.lock().unwrap().clone();
        assert_eq!(reads, [3], "the batch was read once");
        rig.pacer.kick();
        let frames = peer.pump(2_000).await;
        assert_eq!(entry_counters(&frames), Vec::<u64>::new(), "{frames:?}");
        assert_eq!(*journal.serve_froms.lock().unwrap(), [3], "not read again");
        stop(rig).await;
    }

    fn opaque_hello(n: u8) -> LocalHello {
        let mut hello = fork_hello(n);
        hello.caps.push(OPAQUE_CAP.to_owned());
        hello
    }

    /// The items of every `entries` frame in `frames`, in order.
    fn entry_items(frames: &[PeerFrame]) -> Vec<&WireItem> {
        frames
            .iter()
            .filter_map(|f| match f {
                PeerFrame::Entries { entries, .. } => Some(entries.iter()),
                _ => None,
            })
            .flatten()
            .collect()
    }

    /// The opaque entry the journal holds at `counter` of `log`.
    fn held_opaque(journal: &TestJournal, log: &LogKey, counter: u64) -> StoredEntry {
        let one = BatchLimits {
            entries: 1,
            bytes: usize::MAX,
        };
        journal
            .inner
            .serve(log, counter, one, &open_budget())
            .unwrap()
            .remove(0)
    }

    #[tokio::test(start_paused = true)]
    async fn hello_advertises_opaque_cap() {
        let rig = start();
        let peer = connect(&rig, 0, &opaque_hello(2)).await;
        assert!(
            peer.info.caps.iter().any(|c| c == OPAQUE_CAP),
            "{:?}",
            peer.info.caps
        );
        stop(rig).await;

        // Without the option the cap is not offered, whatever the peer says.
        let rig = start_node_full(
            1,
            Arc::new(MemoryJournal::new(identity(1))),
            None,
            CoordinatorOptions {
                advertise_opaque: false,
                ..CoordinatorOptions::default()
            },
        );
        let peer = connect(&rig, 0, &opaque_hello(2)).await;
        assert!(
            peer.info.caps.iter().all(|c| c != OPAQUE_CAP),
            "{:?}",
            peer.info.caps
        );
        assert!(peer.info.caps.iter().any(|c| c == FORK_CAP));
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn opaque_item_sent_only_to_cap_peer() {
        let (rig, journal, log) = node_with_opaque_log();
        let want = || PeerFrame::Want {
            node: log.node,
            inc: log.incarnation,
            from: 1,
        };
        let mut cap = connect(&rig, 0, &opaque_hello(2)).await;
        cap.expect_have().await;
        let mut plain = connect(&rig, 1, &fork_hello(3)).await;
        plain.expect_have().await;
        cap.send(want()).await;
        plain.send(want()).await;

        let frames = cap.pump(2_000).await;
        let items = entry_items(&frames);
        let shape: Vec<(u64, bool)> = items
            .iter()
            .map(|i| (i.counter(), matches!(i, WireItem::Opaque(_))))
            .collect();
        assert_eq!(
            shape,
            [(1, false), (2, false), (3, true), (4, false)],
            "{frames:?}"
        );
        let held = held_opaque(&journal, &log, 3);
        let stored = items[2].clone().into_stored().unwrap();
        assert_eq!(stored, held, "the raw bytes travel unchanged");

        let frames = plain.pump(2_000).await;
        assert_eq!(entry_counters(&frames), [1, 2], "{frames:?}");
        assert!(
            !frames.iter().any(|f| matches!(f, PeerFrame::Close { .. })),
            "withholding is not an error: {frames:?}"
        );
        let status = snapshot(&rig).await;
        // Two entries (3 and 4) were withheld, one log is parked.
        assert_eq!(status.links[plain.idx].withheld_logs, 1);
        assert_eq!(status.links[cap.idx].withheld_logs, 0);
        stop(rig).await;
    }

    // `withheld_logs` counts parked logs, not withheld entries: two logs, each
    // with two entries the plain peer never gets, count 2 (ht-j87z.33).
    #[tokio::test(start_paused = true)]
    async fn withheld_logs_counts_parked_logs_not_entries() {
        let (rig, journal, p) = node_with_opaque_log();
        let q = ingest_opaque_log(&journal.inner, 10);
        rig.pacer.kick();
        let mut plain = connect(&rig, 0, &fork_hello(2)).await;
        plain.expect_have().await;
        for log in [&p, &q] {
            plain
                .send(PeerFrame::Want {
                    node: log.node,
                    inc: log.incarnation,
                    from: 1,
                })
                .await;
        }
        let frames = plain.pump(2_000).await;
        let mut got: Vec<(NodeUuid, u64)> = Vec::new();
        for frame in &frames {
            if let PeerFrame::Entries { node, entries, .. } = frame {
                got.extend(entries.iter().map(|i| (*node, i.counter())));
            }
        }
        got.sort();
        let mut want = vec![(p.node, 1), (p.node, 2), (q.node, 1), (q.node, 2)];
        want.sort();
        assert_eq!(got, want, "{frames:?}");
        // Four entries (3 and 4 of each log) were withheld, two logs parked.
        let status = snapshot(&rig).await;
        assert_eq!(status.links[plain.idx].withheld_logs, 2);
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn gate_parks_log_after_empty_cut() {
        let (rig, journal, log) = node_with_opaque_log();
        let own = journal.inner.self_log();
        let mut plain = connect(&rig, 0, &fork_hello(2)).await;
        plain.expect_have().await;
        let want_foreign = || PeerFrame::Want {
            node: log.node,
            inc: log.incarnation,
            from: 3,
        };
        plain.send(want_foreign()).await;
        plain.pump(2_000).await;
        let reads = journal.serve_froms.lock().unwrap().len();
        assert_eq!(reads, 1);

        // Ticks, kicks and the same want again do not read the log again.
        for _ in 0..3 {
            rig.pacer.kick();
            plain.send(want_foreign()).await;
            plain.pump(2_000).await;
        }
        assert_eq!(journal.serve_froms.lock().unwrap().len(), reads);

        // Other logs keep flowing on the link.
        plain
            .send(PeerFrame::Want {
                node: own.node,
                inc: own.incarnation,
                from: 1,
            })
            .await;
        let frames = plain.pump(2_000).await;
        assert_eq!(entry_counters(&frames), [1], "{frames:?}");
        journal
            .inner
            .author_raw("test.kind", payload(1, 0))
            .unwrap();
        rig.pacer.kick();
        let frames = plain.pump(2_000).await;
        assert_eq!(entry_counters(&frames), [2], "{frames:?}");
        let foreign_reads = journal
            .serve_froms
            .lock()
            .unwrap()
            .iter()
            .filter(|from| **from == 3)
            .count();
        assert_eq!(foreign_reads, 1, "the parked log stayed parked");

        // A want from before the cut serves the known entries again, once.
        plain
            .send(PeerFrame::Want {
                node: log.node,
                inc: log.incarnation,
                from: 1,
            })
            .await;
        let frames = plain.pump(2_000).await;
        assert_eq!(entry_counters(&frames), [1, 2], "{frames:?}");
        assert_eq!(snapshot(&rig).await.links[plain.idx].withheld_logs, 1);
        stop(rig).await;
    }

    // A park ends the link's serve job like any other: the next armed log is
    // served at once, not on some later event (roast-pr-1, ht-j87z.30).
    #[tokio::test(start_paused = true)]
    async fn park_pushes_the_links_next_wanted_log() {
        let (rig, journal, log) = node_with_opaque_log();
        let own = journal.inner.self_log();
        let mut plain = connect(&rig, 0, &fork_hello(2)).await;
        plain.expect_have().await;
        // Hold the first serve job so the second want lands while it runs.
        journal.serve_gate.store(true, Ordering::SeqCst);
        plain
            .send(PeerFrame::Want {
                node: log.node,
                inc: log.incarnation,
                from: 3,
            })
            .await;
        // Real-time poll: paused time does not advance while a blocking job is in flight.
        for _ in 0..5_000 {
            if !journal.serve_froms.lock().unwrap().is_empty() {
                break;
            }
            tokio::task::yield_now().await;
            std::thread::sleep(Duration::from_millis(1));
        }
        plain
            .send(PeerFrame::Want {
                node: own.node,
                inc: own.incarnation,
                from: 1,
            })
            .await;
        for _ in 0..200 {
            tokio::task::yield_now().await;
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            *journal.serve_froms.lock().unwrap(),
            [3],
            "one job per link: the second want waits"
        );
        journal.serve_gate.store(false, Ordering::SeqCst);
        // No kick, no new entry, no further frame from the peer.
        let frames = plain.pump(2_000).await;
        assert_eq!(entry_counters(&frames), [1], "{frames:?}");
        let reads = journal.serve_froms.lock().unwrap().clone();
        assert_eq!(
            reads.iter().filter(|f| **f == 3).count(),
            1,
            "the parked log is not read again: {reads:?}"
        );
        assert_eq!(snapshot(&rig).await.links[plain.idx].withheld_logs, 1);
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn branch_entries_gated_by_cap() {
        // `[known 1, 2, opaque 3, known 4]`: the branch from 3 starts opaque.
        let (rig, journal, log, head) = node_with_branch(true);
        let want = branch_want_of(log, head, 3);
        let mut plain = connect(&rig, 0, &fork_hello(2)).await;
        plain.expect_have().await;
        plain.send(PeerFrame::BranchWant(want.clone())).await;
        let none = plain
            .find(|f| match f {
                PeerFrame::BranchNone { tip_hash, .. } => Some(tip_hash.clone()),
                PeerFrame::BranchEntries { .. } => panic!("an opaque entry was served"),
                _ => None,
            })
            .await;
        assert_eq!(none, want.tip.hash);

        let mut cap = connect(&rig, 1, &opaque_hello(3)).await;
        cap.expect_have().await;
        cap.send(PeerFrame::BranchWant(want.clone())).await;
        let items = cap
            .find(|f| match f {
                PeerFrame::BranchEntries { entries, more, .. } => {
                    assert!(!more);
                    Some(entries.clone())
                }
                _ => None,
            })
            .await;
        let shape: Vec<(u64, bool)> = items
            .iter()
            .map(|i| (i.counter(), matches!(i, WireItem::Opaque(_))))
            .collect();
        assert_eq!(shape, [(3, true), (4, false)]);
        assert_eq!(
            items[0].clone().into_stored().unwrap(),
            held_opaque(&journal, &log, 3)
        );
        stop(rig).await;
    }

    // Entries a cap peer sends are stored, opaque ones included, and served
    // on byte-identical.
    #[tokio::test(start_paused = true)]
    async fn opaque_items_ingested_and_stored() {
        use crate::test_support::opaque_entry::{synthetic_entry, synthetic_unsigned};
        let (rig, journal) = node(1, 0, 0);
        let f = foreign(9, 1);
        let signer = identity(9);
        let opaque = synthetic_entry(
            &signer,
            &synthetic_unsigned(&signer, f.log.incarnation, 3, f.envs[1].hash),
        );
        let four = synthetic_unsigned(&signer, f.log.incarnation, 4, opaque.hash()).seal(&signer);
        let mut peer = connect(&rig, 0, &opaque_hello(2)).await;
        peer.expect_have().await;
        peer.send_have(
            true,
            false,
            vec![have_log(f.log.node, f.log.incarnation, 4)],
        )
        .await;
        peer.find_want(f.log.node, f.log.incarnation, 1).await;
        let mut items: Vec<WireItem> = f.envs.iter().map(WireItem::from).collect();
        items.push(WireItem::from(&opaque));
        items.push(WireItem::from(&four));
        peer.send(PeerFrame::Entries {
            node: f.log.node,
            inc: f.log.incarnation,
            entries: items,
        })
        .await;
        until(|| open_vector_counter(&journal, &f.log) == 4).await;
        assert_eq!(
            held_opaque(&journal, &f.log, 3),
            StoredEntry::Opaque(opaque)
        );
        assert!(
            !rig.lines
                .lock()
                .unwrap()
                .iter()
                .any(|l| l.contains("corrupt")),
            "{:?}",
            rig.lines.lock().unwrap()
        );
        stop(rig).await;
    }

    // Two cap nodes: the opaque entry crosses the link byte-exact and the
    // successor follows it into the puller's journal.
    #[tokio::test(start_paused = true)]
    async fn opaque_entries_relay_between_cap_nodes() {
        let (a, ja, log) = node_with_opaque_log();
        let jb = TestJournal::new(2);
        let b = start_node(2, jb.clone());
        let _report = link_nodes(&a, &b).await;
        until(|| open_vector_counter(&jb, &log) == 4).await;
        assert_eq!(held_opaque(&jb, &log, 3), held_opaque(&ja, &log, 3));
        assert!(held_opaque(&jb, &log, 3).is_opaque());
        assert_eq!(snapshot(&a).await.links[0].withheld_logs, 0);
        stop(a).await;
        stop(b).await;
    }

    // A node that did not advertise the cap keeps the known prefix, stays
    // linked, and the serving side parks the log instead of re-reading it.
    #[tokio::test(start_paused = true)]
    async fn capless_node_converges_to_the_known_prefix_only() {
        let (a, ja, log) = node_with_opaque_log();
        let jb = TestJournal::new(2);
        let b = start_node_full(
            2,
            jb.clone(),
            None,
            CoordinatorOptions {
                advertise_opaque: false,
                ..CoordinatorOptions::default()
            },
        );
        let mut report = link_nodes(&a, &b).await;
        until(|| open_vector_counter(&jb, &log) == 2).await;
        until(|| {
            let ja = ja.serve_froms.lock().unwrap();
            ja.contains(&3)
        })
        .await;
        tokio::time::sleep(Duration::from_secs(30)).await;
        assert_eq!(open_vector_counter(&jb, &log), 2, "stuck at the cut");
        let status = snapshot(&a).await;
        assert_eq!(status.links.len(), 1, "the link stays up");
        assert_eq!(status.links[0].withheld_logs, 1);
        assert!(
            report
                .try_recv()
                .map_or(true, |r| !matches!(r, DialReport::LinkEnded { .. })),
            "no link ended"
        );
        let reads_of_cut = ja
            .serve_froms
            .lock()
            .unwrap()
            .iter()
            .filter(|f| **f == 3)
            .count();
        assert_eq!(reads_of_cut, 1, "the cut was read once, not per tick");
        assert!(
            !a.lines
                .lock()
                .unwrap()
                .iter()
                .any(|l| l.contains("corrupt")),
            "{:?}",
            a.lines.lock().unwrap()
        );
        stop(a).await;
        stop(b).await;
    }

    // A fork whose proof carries an opaque side crosses a link between cap
    // nodes as a `branch_entries` item the receiving fork sink gets intact.
    #[tokio::test(start_paused = true)]
    async fn opaque_fork_branch_flows_to_cap_peer() {
        let (a, ja, log, head) = node_with_branch(true);
        let jb = TestJournal::new(2);
        let b = start_node(2, jb.clone());
        let _report = link_nodes(&a, &b).await;
        let want = branch_want_of(log, head, 3);
        *b.sink.outbox.lock().unwrap() = ForkOutbox {
            sends: vec![(node_of(1), want.clone())],
            refs_changed: false,
        };
        until(|| !b.sink.branches.lock().unwrap().is_empty()).await;
        let (source, reply) = b.sink.branches.lock().unwrap().remove(0);
        assert_eq!(source, node_of(1));
        let BranchReply::Entries {
            entries, tip_hash, ..
        } = reply
        else {
            panic!("a branch_none");
        };
        assert_eq!(tip_hash, want.tip.hash);
        let shape: Vec<(u64, bool)> = entries
            .iter()
            .map(|i| (i.counter(), matches!(i, WireItem::Opaque(_))))
            .collect();
        assert_eq!(shape, [(3, true), (4, false)]);
        assert_eq!(
            entries[0].clone().into_stored().unwrap(),
            held_opaque(&ja, &log, 3)
        );
        stop(a).await;
        stop(b).await;
    }

    fn oversize_item(log: &LogKey, counter: u64, prev: [u8; 32]) -> WireItem {
        use crate::sync::envelope::OPAQUE_MAX_BYTES;
        WireItem::Opaque(OpaqueWire {
            node: hex_encode(log.node.as_bytes()),
            inc: log.incarnation,
            counter,
            prev: hex_encode(&prev),
            hash: "11".repeat(32),
            sig: "22".repeat(64),
            raw: "00".repeat(OPAQUE_MAX_BYTES + 1),
        })
    }

    /// A validly signed oversize opaque item of `signer`'s log, plus its
    /// parts so tests can tamper with them.
    fn signed_oversize_item(
        signer: &NodeIdentity,
        log: &LogKey,
        counter: u64,
        prev: [u8; 32],
    ) -> (WireItem, crate::sync::envelope::OpaqueHeader, Vec<u8>) {
        use crate::sync::envelope::OPAQUE_MAX_BYTES;
        use crate::test_support::opaque_entry::{synthetic_parts, synthetic_unsigned};
        let mut unsigned = synthetic_unsigned(signer, log.incarnation, counter, prev);
        // The entry names `log` whoever signs it.
        unsigned.log = *log;
        unsigned.payload = format!(r#"{{"pad":"{}"}}"#, "x".repeat(OPAQUE_MAX_BYTES)).into_bytes();
        let (header, raw) = synthetic_parts(signer, &unsigned);
        let item = oversize_wire(&header, &raw);
        (item, header, raw)
    }

    fn oversize_wire(header: &crate::sync::envelope::OpaqueHeader, raw: &[u8]) -> WireItem {
        WireItem::Opaque(OpaqueWire {
            node: hex_encode(header.log.node.as_bytes()),
            inc: header.log.incarnation,
            counter: header.counter,
            prev: hex_encode(&header.prev_hash),
            hash: hex_encode(&header.hash),
            sig: hex_encode(&header.signature),
            raw: hex_encode(raw),
        })
    }

    #[tokio::test(start_paused = true)]
    async fn oversize_opaque_item_stores_prefix_and_stalls_without_penalty() {
        use crate::sync::envelope::OPAQUE_MAX_BYTES;
        let (rig, journal) = node(1, 0, 0);
        let f = foreign(9, 1);
        let mut peer = connect(&rig, 0, &opaque_hello(2)).await;
        peer.expect_have().await;
        peer.send_have(
            true,
            false,
            vec![have_log(f.log.node, f.log.incarnation, 5)],
        )
        .await;
        peer.find_want(f.log.node, f.log.incarnation, 1).await;
        let mut items: Vec<WireItem> = f.envs.iter().map(WireItem::from).collect();
        let (item, _, raw) = signed_oversize_item(&identity(9), &f.log, 3, f.envs[1].hash);
        let size = raw.len();
        items.push(item);
        peer.send(PeerFrame::Entries {
            node: f.log.node,
            inc: f.log.incarnation,
            entries: items,
        })
        .await;
        until(|| open_vector_counter(&journal, &f.log) == 2).await;
        peer.find_unwant(f.log.node, f.log.incarnation).await;

        // Stalled: nothing is asked again, however long the peer keeps
        // advertising the log, and past the corrupt-source penalty window.
        peer.send_have(
            true,
            false,
            vec![have_log(f.log.node, f.log.incarnation, 5)],
        )
        .await;
        let frames = peer.pump(ingest::CORRUPT_PENALTY_MS + 60_000).await;
        assert!(wants_in(&frames).is_empty(), "{frames:?}");
        assert_eq!(open_vector_counter(&journal, &f.log), 2);
        let lines = rig.lines.lock().unwrap().clone();
        let stalled: Vec<&String> = lines.iter().filter(|l| l.contains("stalled")).collect();
        assert_eq!(
            stalled,
            [&format!(
                "sync: log {}/{} entry 3 is an opaque entry of {} bytes over {OPAQUE_MAX_BYTES}; log stalled on this node",
                uuid8(&f.log.node),
                f.log.incarnation,
                size
            )]
        );
        assert!(
            !lines.iter().any(|l| l.contains("penalized")),
            "a size refusal is not a corrupt source: {lines:?}"
        );
        // The link itself is fine.
        assert_eq!(snapshot(&rig).await.links.len(), 1);
        stop(rig).await;
    }

    /// Peer 0 is wanted for the foreign log (five entries) and sends its first
    /// two plus `bad`; peer 1 also advertises the log. Returns after peer 0
    /// was unwanted and peer 1 was asked for the entry after the stored prefix.
    async fn oversize_refusal_scenario(
        bad: fn(&Foreign) -> WireItem,
    ) -> (Rig, Arc<TestJournal>, Foreign, TestPeer) {
        let (rig, journal) = node(1, 0, 0);
        let f = foreign(9, 4);
        let (node_id, inc) = (f.log.node, f.log.incarnation);
        let mut a = connect(&rig, 0, &opaque_hello(2)).await;
        a.expect_have().await;
        let mut b = connect(&rig, 1, &opaque_hello(3)).await;
        b.expect_have().await;
        a.send_have(true, false, vec![have_log(node_id, inc, 5)])
            .await;
        a.find_want(node_id, inc, 1).await;
        b.send_have(true, false, vec![have_log(node_id, inc, 5)])
            .await;
        let mut items: Vec<WireItem> = f.envs[..2].iter().map(WireItem::from).collect();
        items.push(bad(&f));
        a.send(PeerFrame::Entries {
            node: node_id,
            inc,
            entries: items,
        })
        .await;
        until(|| open_vector_counter(&journal, &f.log) == 2).await;
        a.find_unwant(node_id, inc).await;
        // The other source is asked, from the entry after the stored prefix.
        b.find_want(node_id, inc, 3).await;
        let lines = rig.lines.lock().unwrap().clone();
        assert!(
            !lines.iter().any(|l| l.contains("stalled")),
            "an unverified item must not stall the log: {lines:?}"
        );
        let penalized: Vec<&String> = lines.iter().filter(|l| l.contains("penalized")).collect();
        assert_eq!(penalized.len(), 1, "{lines:?}");
        assert!(
            penalized[0].contains("unverified oversize opaque entry"),
            "{penalized:?}"
        );
        assert_eq!(snapshot(&rig).await.links.len(), 2);
        // The log goes on from the other source.
        b.send(PeerFrame::Entries {
            node: node_id,
            inc,
            entries: f.envs[2..].iter().map(WireItem::from).collect(),
        })
        .await;
        until(|| open_vector_counter(&journal, &f.log) == 5).await;
        (rig, journal, f, a)
    }

    #[tokio::test(start_paused = true)]
    async fn oversize_item_for_other_log_or_counter_does_not_stall() {
        let (rig, ..) = oversize_refusal_scenario(|f| {
            let other = LogKey {
                node: identity(8).uuid(),
                incarnation: 1,
            };
            signed_oversize_item(&identity(8), &other, 3, f.envs[1].hash).0
        })
        .await;
        stop(rig).await;
        let (rig, ..) = oversize_refusal_scenario(|f| {
            signed_oversize_item(&identity(9), &f.log, 4, f.envs[1].hash).0
        })
        .await;
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn unverified_oversize_refuses_only_that_source() {
        // Right position, wrong author: the signature does not verify.
        let (rig, ..) = oversize_refusal_scenario(|f| {
            signed_oversize_item(&identity(7), &f.log, 3, f.envs[1].hash).0
        })
        .await;
        stop(rig).await;
    }

    // The refused source is wanted again when its penalty expires.
    #[tokio::test(start_paused = true)]
    async fn refused_oversize_source_is_asked_again_after_the_penalty() {
        let (rig, journal) = node(1, 0, 0);
        let f = foreign(9, 4);
        let (node_id, inc) = (f.log.node, f.log.incarnation);
        let mut a = connect(&rig, 0, &opaque_hello(2)).await;
        a.expect_have().await;
        a.send_have(true, false, vec![have_log(node_id, inc, 5)])
            .await;
        a.find_want(node_id, inc, 1).await;
        let mut items: Vec<WireItem> = f.envs[..2].iter().map(WireItem::from).collect();
        items.push(signed_oversize_item(&identity(7), &f.log, 3, f.envs[1].hash).0);
        a.send(PeerFrame::Entries {
            node: node_id,
            inc,
            entries: items,
        })
        .await;
        until(|| open_vector_counter(&journal, &f.log) == 2).await;
        a.find_unwant(node_id, inc).await;
        let early = a.pump(ingest::CORRUPT_PENALTY_MS / 2).await;
        assert!(wants_in(&early).is_empty(), "{early:?}");
        a.send_have(true, false, vec![have_log(node_id, inc, 5)])
            .await;
        let late = a.pump(ingest::CORRUPT_PENALTY_MS / 2 + 5_000).await;
        assert!(wants_in(&late).contains(&(node_id, inc, 3)), "{late:?}");
        stop(rig).await;
    }

    fn worker_parts() -> (Arc<FairWriter>, Arc<dyn Clock>) {
        (
            Arc::new(FairWriter::new(8)),
            Arc::new(TokioClock(Instant::now())),
        )
    }

    // The batch worker's verdict on an oversize claim: only one authenticated
    // as the log's next entry stalls the log; anything else is refused for its
    // source alone, with the stored prefix kept.
    #[tokio::test]
    async fn ingest_batch_authenticates_oversize_claims() {
        use crate::sync::envelope::OPAQUE_MAX_BYTES;
        let f = foreign(9, 3);
        let (writer, clock) = worker_parts();
        let claim = |item: WireItem| -> ingest::OversizeClaim {
            let WireItem::Opaque(wire) = item else {
                panic!("an opaque item")
            };
            ingest::OversizeClaim {
                len: wire.raw.len() / 2,
                wire,
            }
        };
        let known = |range: std::ops::Range<usize>| -> Vec<StoredEntry> {
            f.envs[range]
                .iter()
                .cloned()
                .map(StoredEntry::from)
                .collect()
        };
        // A fresh journal per case, holding the log's first `held` entries;
        // `batch` more arrive in the same frame as the claim.
        let run = |held: usize, batch: usize, item: WireItem| -> ingest::IngestVerdict {
            let journal = MemoryJournal::new(identity(1));
            journal.bootstrap();
            if held > 0 {
                journal
                    .ingest(&f.log, known(0..held), &open_budget())
                    .unwrap();
            }
            ingest::ingest_batch(
                &journal,
                &writer,
                clock.as_ref(),
                Cancellation::default(),
                &f.log,
                known(held..held + batch),
                Some(claim(item)),
            )
            .unwrap()
        };
        let signer = identity(9);
        let refused =
            |verdict: ingest::IngestVerdict, batch: usize, head_at: usize, why: &str| match verdict
            {
                ingest::IngestVerdict::OversizeRefused {
                    stored,
                    head,
                    why: w,
                    ..
                } => {
                    assert_eq!((stored, w), (batch, why));
                    assert_eq!(
                        head.map(|h| (h.counter, h.head_hash)),
                        (batch > 0)
                            .then(|| (f.envs[head_at - 1].counter, f.envs[head_at - 1].hash))
                    );
                }
                other => panic!("expected a refusal ({why}), got {other:?}"),
            };

        // Verified, after a stored prefix of 2 (log held to 1, so 3 is next).
        let (item, ..) = signed_oversize_item(&signer, &f.log, 3, f.envs[1].hash);
        let size = match &item {
            WireItem::Opaque(w) => w.raw.len() / 2,
            WireItem::Known(_) => unreachable!(),
        };
        assert!(size > OPAQUE_MAX_BYTES);
        assert_eq!(
            run(0, 2, item),
            ingest::IngestVerdict::Port(IngestOutcome::Oversize {
                stored: 2,
                counter: 3,
                len: size
            })
        );
        // Verified as the first item: the held log's head is entry 2.
        let (item, ..) = signed_oversize_item(&signer, &f.log, 3, f.envs[1].hash);
        assert_eq!(
            run(2, 0, item),
            ingest::IngestVerdict::Port(IngestOutcome::Oversize {
                stored: 0,
                counter: 3,
                len: size
            })
        );
        // Another log's validly signed item.
        let other_log = LogKey {
            node: identity(8).uuid(),
            incarnation: 1,
        };
        let (item, ..) = signed_oversize_item(&identity(8), &other_log, 3, f.envs[1].hash);
        refused(run(0, 2, item), 2, 2, "names another log");
        // Validly signed, but not the next position.
        let (item, ..) = signed_oversize_item(&signer, &f.log, 4, f.envs[1].hash);
        refused(run(0, 2, item), 2, 2, "not the next position");
        // As the first item the same refusal holds (the former unchecked arm).
        let (item, ..) = signed_oversize_item(&signer, &f.log, 4, f.envs[1].hash);
        refused(run(2, 0, item), 0, 2, "not the next position");
        // prev is not the head hash.
        let (item, ..) = signed_oversize_item(&signer, &f.log, 3, [7; 32]);
        refused(run(0, 2, item), 2, 2, "prev does not follow the head");
        // Raw tampered after signing.
        let (_, header, mut raw) = signed_oversize_item(&signer, &f.log, 3, f.envs[1].hash);
        let last = raw.len() - 1;
        raw[last] ^= 1;
        refused(
            run(0, 2, oversize_wire(&header, &raw)),
            2,
            2,
            "hash mismatch",
        );
        // Signed by another identity.
        let (item, ..) = signed_oversize_item(&identity(7), &f.log, 3, f.envs[1].hash);
        refused(run(0, 2, item), 2, 2, "bad signature");
        // Unsigned garbage.
        refused(
            run(0, 2, oversize_item(&f.log, 3, f.envs[1].hash)),
            2,
            2,
            "prefix does not match",
        );
        // A log the journal does not hold (nothing stored, nothing known).
        let journal = MemoryJournal::new(identity(1));
        journal.bootstrap();
        let (item, ..) = signed_oversize_item(&signer, &f.log, 1, [0; 32]);
        let verdict = ingest::ingest_batch(
            &journal,
            &writer,
            clock.as_ref(),
            Cancellation::default(),
            &f.log,
            Vec::new(),
            Some(claim(item)),
        )
        .unwrap();
        refused(verdict, 0, 0, "unknown log");
    }

    #[tokio::test(start_paused = true)]
    async fn have_deltas_at_most_1_per_s() {
        let (rig, journal) = node(1, 0, 0);
        let own = journal.inner.self_log();
        let mut peer = connect(&rig, 0, &peer_hello(2)).await;
        peer.expect_have().await;
        let mut at: Vec<tokio::time::Instant> = Vec::new();
        let mut last_counter = 0;
        for step in 0..80 {
            if step % 10 == 0 {
                journal
                    .inner
                    .author_raw("test.kind", payload(step, 0))
                    .unwrap();
                rig.pacer.kick();
            }
            for frame in peer.pump(50).await {
                if let PeerFrame::Have {
                    full: false, logs, ..
                } = frame
                {
                    at.push(tokio::time::Instant::now());
                    last_counter = logs
                        .iter()
                        .find(|l| l.node == own.node)
                        .map_or(last_counter, |l| l.counter);
                }
            }
        }
        assert!(at.len() >= 2, "deltas flow: {}", at.len());
        assert!(at.len() <= 5, "at most one a second over 4 s: {}", at.len());
        for pair in at.windows(2) {
            assert!(
                pair[1] - pair[0] >= Duration::from_millis(900),
                "two deltas {:?} apart",
                pair[1] - pair[0]
            );
        }
        // The throttled leftovers still go out.
        for frame in peer.pump(2_000).await {
            if let PeerFrame::Have {
                full: false, logs, ..
            } = frame
            {
                last_counter = logs
                    .iter()
                    .find(|l| l.node == own.node)
                    .map_or(last_counter, |l| l.counter);
            }
        }
        assert_eq!(last_counter, open_vector_counter(&journal, &own));
        stop(rig).await;
    }

    /// A rig plus manual peers B (node 2) and C (node 3) that both advertise
    /// the foreign log `f`: B has more, so B is the rx source and has been
    /// sent the first `Want`.
    async fn two_sources(rig: &Rig, f: &Foreign) -> (TestPeer, TestPeer) {
        let mut b = connect(rig, 0, &peer_hello(2)).await;
        b.expect_have().await;
        let mut c = connect(rig, 1, &peer_hello(3)).await;
        c.expect_have().await;
        let top = f.envs.len() as u64;
        b.send_have(true, false, vec![have_log(f.log.node, 1, top)])
            .await;
        c.send_have(true, false, vec![have_log(f.log.node, 1, top - 1)])
            .await;
        b.find_want(f.log.node, 1, 1).await;
        (b, c)
    }

    #[tokio::test(start_paused = true)]
    async fn corrupt_penalizes_and_switches_source() {
        let (rig, journal) = node(1, 0, 0);
        let f = foreign(9, 3);
        let (mut b, mut c) = two_sources(&rig, &f).await;

        // B serves a forged second entry: nothing is stored.
        let mut forged = f.envs[..2].to_vec();
        forged[1].payload = payload(99, 0);
        let frame = entries_frame(&f.log, &forged);
        let len = wire_len(&frame);
        b.send(frame).await;
        assert_eq!(b.find_credit().await, len, "the frame's credit comes back");
        b.find_unwant(f.log.node, 1).await;
        c.find_want(f.log.node, 1, 1).await;
        assert_eq!(open_vector_counter(&journal, &f.log), 0);

        // The penalty holds B off the log for minutes.
        let frames = b.pump(5 * 60_000).await;
        assert!(
            wants_in(&frames).is_empty(),
            "B is wanted again: {frames:?}"
        );
        assert!(
            rig.lines
                .lock()
                .unwrap()
                .iter()
                .any(|l| l.contains("corrupt"))
        );
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn headroom_pauses_every_want_60s() {
        let (rig, journal) = node(1, 0, 0);
        let (f9, f8) = (foreign(9, 2), foreign(8, 2));
        let mut b = connect(&rig, 0, &peer_hello(2)).await;
        b.expect_have().await;
        b.send_have(
            true,
            false,
            vec![have_log(f9.log.node, 1, 3), have_log(f8.log.node, 1, 3)],
        )
        .await;
        let mut wanted = Vec::new();
        while wanted.len() < 2 {
            wanted.push(
                b.find(|f| match f {
                    PeerFrame::Want { node, from: 1, .. } => Some(*node),
                    _ => None,
                })
                .await,
            );
        }
        wanted.sort();
        let mut both = [f9.log.node, f8.log.node];
        both.sort();
        assert_eq!(wanted, both);

        journal.script(Scripted::Outcome(IngestOutcome::Headroom));
        let frame = entries_frame(&f9.log, &f9.envs[..2]);
        let len = wire_len(&frame);
        b.send(frame).await;
        assert_eq!(b.find_credit().await, len);
        let mut unwanted = Vec::new();
        while unwanted.len() < 2 {
            unwanted.push(
                b.find(|f| match f {
                    PeerFrame::Unwant { node, .. } => Some(*node),
                    _ => None,
                })
                .await,
            );
        }
        unwanted.sort();
        assert_eq!(unwanted, both, "every rx subscription is unwanted");
        let snap = snapshot(&rig).await;
        let until = snap.headroom_until.expect("the pause is reported");
        let left = until.0 - rig.clock.utc_now().0;
        assert!((55_000..=60_000).contains(&left), "{left} ms left");

        // Fresh news during the pause draws no want.
        let frames = b.pump(30_000).await;
        assert!(wants_in(&frames).is_empty(), "{frames:?}");
        b.send_have(
            true,
            false,
            vec![have_log(f9.log.node, 1, 3), have_log(f8.log.node, 1, 3)],
        )
        .await;
        let frames = b.pump(25_000).await;
        assert!(wants_in(&frames).is_empty(), "{frames:?}");

        // After the 60 s every log is wanted again.
        let frames = b.pump(8_000).await;
        let wants = wants_in(&frames);
        assert!(wants.contains(&(f9.log.node, 1, 1)), "{frames:?}");
        assert!(wants.contains(&(f8.log.node, 1, 1)), "{frames:?}");
        assert_eq!(snapshot(&rig).await.headroom_until, None);
        stop(rig).await;
    }

    fn evidence(f: &Foreign) -> ForkEvidence {
        ForkEvidence::Conflict {
            held: Box::new(StoredEntry::from(f.envs[1].clone())),
            conflicting: Box::new(StoredEntry::from(f.envs[2].clone())),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn fork_pauses_log_and_resumed_rewants() {
        let (rig, journal) = node(1, 0, 0);
        let f = foreign(9, 2);
        let mut b = connect(&rig, 0, &peer_hello(2)).await;
        b.expect_have().await;
        b.send_have(true, false, vec![have_log(f.log.node, 1, 3)])
            .await;
        b.find_want(f.log.node, 1, 1).await;

        journal.script(Scripted::Outcome(IngestOutcome::Fork {
            stored: 0,
            evidence: evidence(&f),
        }));
        let frame = entries_frame(&f.log, &f.envs[..1]);
        let len = wire_len(&frame);
        b.send(frame).await;
        assert_eq!(b.find_credit().await, len);
        b.find_unwant(f.log.node, 1).await;
        assert_eq!(
            *rig.sink.forks.lock().unwrap(),
            vec![(f.log, node_of(2))],
            "the sink got the fork with the source node"
        );
        assert_eq!(snapshot(&rig).await.paused_logs, vec![f.log]);

        // Paused: more news draws no want.
        b.send_have(true, false, vec![have_log(f.log.node, 1, 5)])
            .await;
        let frames = b.pump(5_000).await;
        assert!(wants_in(&frames).is_empty(), "{frames:?}");

        // The sink reports it resumed: the next tick wants it again.
        rig.sink.resume.lock().unwrap().push(f.log);
        let frames = b.pump(2_000).await;
        assert_eq!(wants_in(&frames), vec![(f.log.node, 1, 1)], "{frames:?}");
        assert!(snapshot(&rig).await.paused_logs.is_empty());
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn fork_ack_corrupt_penalizes_and_switches() {
        let (rig, journal) = node(1, 0, 0);
        let f = foreign(9, 3);
        *rig.sink.ack.lock().unwrap() = ForkAck::Corrupt;
        let (mut b, mut c) = two_sources(&rig, &f).await;

        journal.script(Scripted::Outcome(IngestOutcome::Fork {
            stored: 0,
            evidence: evidence(&f),
        }));
        b.send(entries_frame(&f.log, &f.envs[..1])).await;
        b.find_unwant(f.log.node, 1).await;
        c.find_want(f.log.node, 1, 1).await;
        assert_eq!(rig.sink.forks.lock().unwrap().len(), 1);
        assert!(
            snapshot(&rig).await.paused_logs.is_empty(),
            "an unproven fork pauses nothing"
        );
        let frames = b.pump(2 * 60_000).await;
        assert!(
            wants_in(&frames).is_empty(),
            "B stays penalized: {frames:?}"
        );
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn err_records_sync_lane_failure_and_rewants_after_backoff() {
        let (rig, journal) = node(1, 0, 0);
        let own = journal.inner.self_log();
        let f = foreign(9, 2);
        let mut b = connect(&rig, 0, &peer_hello(2)).await;
        b.expect_have().await;
        // B also wants this node's own log, to observe the live tail.
        b.send(PeerFrame::Want {
            node: own.node,
            inc: own.incarnation,
            from: 2,
        })
        .await;
        b.send_have(true, false, vec![have_log(f.log.node, 1, 3)])
            .await;
        b.find_want(f.log.node, 1, 1).await;

        journal.script(Scripted::Fail);
        let frame = entries_frame(&f.log, &f.envs[..2]);
        let len = wire_len(&frame);
        b.send(frame).await;
        assert_eq!(
            b.find_credit().await,
            len,
            "credit returns on a store error"
        );
        until(|| rig.status.last_error().is_some()).await;
        assert!(
            rig.status.last_error().unwrap().contains("sync"),
            "{:?}",
            rig.status.last_error()
        );
        assert_eq!(rig.pacer.attempts(), 1, "the backoff is armed");

        // A kick that lands during the backoff is latched, not lost.
        journal
            .inner
            .author_raw("test.kind", payload(1, 0))
            .unwrap();
        rig.pacer.kick();
        let early = b.pump(40).await;
        assert!(
            wants_in(&early).is_empty(),
            "not before the backoff: {early:?}"
        );
        let frames = b.pump(600).await;
        assert_eq!(wants_in(&frames), vec![(f.log.node, 1, 1)], "{frames:?}");
        let tail: Vec<u64> = frames
            .iter()
            .filter_map(|f| match f {
                PeerFrame::Entries { entries, .. } => Some(entries.iter().map(WireItem::counter)),
                _ => None,
            })
            .flatten()
            .collect();
        assert_eq!(tail, vec![2], "the latched kick pushed the new own entry");

        // The retried batch stores: the failure and the backoff clear.
        b.send(entries_frame(&f.log, &f.envs[..2])).await;
        until(|| open_vector_counter(&journal, &f.log) == 2).await;
        until(|| rig.status.last_error().is_none()).await;
        until(|| rig.pacer.attempts() == 0).await;

        // A due pacer must not spin the loop.
        let before = rig.pacer.wakes();
        b.pump(5_000).await;
        let woke = rig.pacer.wakes() - before;
        assert!(woke <= 10, "{woke} pacer wakes in 5 s of quiet");
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn unsolicited_entries_dropped_credit_granted() {
        let (rig, journal) = node(1, 0, 0);
        let f = foreign(9, 2);
        let mut b = connect(&rig, 0, &peer_hello(2)).await;
        b.expect_have().await;

        // A log nobody wanted.
        let frame = entries_frame(&f.log, &f.envs[..2]);
        let len = wire_len(&frame);
        b.send(frame).await;
        assert_eq!(b.find_credit().await, len);
        let snap = snapshot(&rig).await;
        assert_eq!(snap.links[0].unsolicited, 1);
        assert_eq!(journal.ingests.load(Ordering::SeqCst), 0, "never ingested");
        assert_eq!(open_vector_counter(&journal, &f.log), 0);

        // A wanted log, from a link that is not its source.
        let mut c = connect(&rig, 1, &peer_hello(3)).await;
        c.expect_have().await;
        b.send_have(true, false, vec![have_log(f.log.node, 1, 3)])
            .await;
        b.find_want(f.log.node, 1, 1).await;
        let frame = entries_frame(&f.log, &f.envs[..2]);
        let len = wire_len(&frame);
        c.send(frame).await;
        assert_eq!(c.find_credit().await, len);
        let snap = snapshot(&rig).await;
        assert_eq!(snap.links[1].unsolicited, 1);
        assert_eq!(snap.links[0].unsolicited, 1);
        assert_eq!(journal.ingests.load(Ordering::SeqCst), 0);
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn entries_from_a_just_replaced_source_are_not_unsolicited() {
        let (rig, journal) = node(1, 0, 0);
        let f = foreign(9, 2);
        let mut b = connect(&rig, 0, &peer_hello(2)).await;
        b.expect_have().await;
        let mut c = connect(&rig, 1, &peer_hello(3)).await;
        c.expect_have().await;
        b.send_have(true, false, vec![have_log(f.log.node, 1, 3)])
            .await;
        c.send_have(true, false, vec![have_log(f.log.node, 1, 2)])
            .await;
        b.find_want(f.log.node, 1, 1).await;

        // B stalls; the log moves to C.
        b.pump(31_000).await;
        c.find_want(f.log.node, 1, 1).await;

        // A frame B had in flight arrives: dropped, credit granted, not counted.
        let frame = entries_frame(&f.log, &f.envs[..2]);
        let len = wire_len(&frame);
        b.send(frame).await;
        assert_eq!(b.find_credit().await, len);
        assert_eq!(snapshot(&rig).await.links[b.idx].unsolicited, 0);

        // Past the grace the same frame is unsolicited.
        b.pump(REPLACED_GRACE_MS + 1_000).await;
        let frame = entries_frame(&f.log, &f.envs[..2]);
        b.send(frame).await;
        assert_eq!(b.find_credit().await, len);
        assert_eq!(snapshot(&rig).await.links[b.idx].unsolicited, 1);
        assert_eq!(journal.ingests.load(Ordering::SeqCst), 0);
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn gap_rewants_from_expected_and_frozen_unwants() {
        let (rig, journal) = node(1, 0, 0);
        let f = foreign(9, 4);
        let mut b = connect(&rig, 0, &peer_hello(2)).await;
        b.expect_have().await;
        b.send_have(true, false, vec![have_log(f.log.node, 1, 5)])
            .await;
        b.find_want(f.log.node, 1, 1).await;
        b.send(entries_frame(&f.log, &f.envs[..2])).await;
        until(|| open_vector_counter(&journal, &f.log) == 2).await;
        // The next batch skips counter 3.
        b.send(entries_frame(&f.log, &f.envs[3..])).await;
        b.find_want(f.log.node, 1, 3).await;

        journal.script(Scripted::Outcome(IngestOutcome::Frozen {
            stored: 0,
            frozen_at: 2,
        }));
        b.send(entries_frame(&f.log, &f.envs[2..4])).await;
        b.find_unwant(f.log.node, 1).await;
        stop(rig).await;
    }
    // -- fork wiring (ht-5fr.6.8) ------------------------------------------

    fn fork_hello(n: u8) -> LocalHello {
        let mut hello = peer_hello(n);
        hello.caps.push(FORK_CAP.to_owned());
        hello
    }

    fn fork_ref(n: u8) -> ForkRef {
        use crate::sync::frame::TipRef;
        ForkRef {
            node: node_of(n),
            inc: 1,
            held: TipRef {
                counter: 3,
                hash: "aa".repeat(32),
            },
            conflicting: TipRef {
                counter: 4,
                hash: "bb".repeat(32),
            },
            frozen_at: 2,
        }
    }

    /// A `branch_want` for `journal`'s own log, ending at its head.
    fn want_own(journal: &TestJournal, from: u64) -> BranchWant {
        let log = journal.inner.self_log();
        let head = journal.view().0[&log];
        BranchWant {
            node: log.node,
            inc: log.incarnation,
            tip: crate::sync::frame::TipRef {
                counter: head.counter,
                hash: hex_encode(&head.head_hash),
            },
            from,
        }
    }

    /// Counters of the next `branch_entries` frame, with its `more` flag.
    async fn next_branch_entries(peer: &mut TestPeer) -> (String, Vec<u64>, bool) {
        peer.find(|f| match f {
            PeerFrame::BranchEntries {
                tip_hash,
                entries,
                more,
                ..
            } => Some((
                tip_hash.clone(),
                entries.iter().map(WireItem::counter).collect(),
                *more,
            )),
            _ => None,
        })
        .await
    }

    #[tokio::test(start_paused = true)]
    async fn on_have_called_per_have() {
        let (rig, _journal) = node(1, 0, 0);
        let mut fork = connect(&rig, 0, &fork_hello(2)).await;
        fork.expect_have().await;
        let mut plain = connect(&rig, 1, &peer_hello(3)).await;
        plain.expect_have().await;

        fork.send_have(true, false, vec![have_log(node_of(9), 1, 2)])
            .await;
        fork.send(PeerFrame::Have {
            full: false,
            ask: false,
            logs: Vec::new(),
            forks: vec![fork_ref(9)],
        })
        .await;
        // A peer without the cap that sends fork refs anyway.
        plain
            .send(PeerFrame::Have {
                full: true,
                ask: false,
                logs: vec![have_log(node_of(8), 1, 1)],
                forks: vec![fork_ref(8)],
            })
            .await;
        until(|| rig.sink.haves.lock().unwrap().len() == 3).await;
        let haves = rig.sink.haves.lock().unwrap().clone();
        let by = |n: u8| {
            haves
                .iter()
                .filter(|h| h.0 == node_of(n))
                .collect::<Vec<_>>()
        };
        let from_fork = by(2);
        assert_eq!(from_fork.len(), 2, "one call per have");
        assert!(from_fork.iter().all(|h| h.1), "the cap is reported");
        assert_eq!(from_fork[0].2.len(), 1);
        assert!(from_fork[0].3.is_empty());
        assert_eq!(from_fork[1].3, vec![fork_ref(9)]);
        let from_plain = by(3);
        assert_eq!(from_plain.len(), 1);
        assert!(!from_plain[0].1, "no cap on that link");
        assert_eq!(from_plain[0].2.len(), 1, "its logs are still handed over");
        assert!(
            from_plain[0].3.is_empty(),
            "refs without the cap are ignored"
        );
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn branch_want_served() {
        let (rig, journal) = node(1, 5, 0);
        let mut peer = connect(&rig, 0, &fork_hello(2)).await;
        peer.expect_have().await;
        let want = want_own(&journal, 2);
        assert_eq!(want.tip.counter, 6);
        peer.send(PeerFrame::BranchWant(want.clone())).await;
        let (tip_hash, counters, more) = next_branch_entries(&mut peer).await;
        assert_eq!(tip_hash, want.tip.hash);
        assert_eq!(counters, [2, 3, 4, 5, 6]);
        assert!(!more, "the batch reaches the tip");

        // One request, one reply: later entries are not pushed.
        journal
            .inner
            .author_raw("test.kind", payload(9, 0))
            .unwrap();
        let frames = peer.pump(3_000).await;
        assert!(
            !frames.iter().any(|f| matches!(
                f,
                PeerFrame::Entries { .. } | PeerFrame::BranchEntries { .. }
            )),
            "no subscription: {frames:?}"
        );
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn branch_want_over_4_gets_branch_none() {
        let (rig, journal) = node(1, 6, 0);
        let mut peer = connect(&rig, 0, &fork_hello(2)).await;
        peer.expect_have().await;
        journal.branch_gate.store(true, Ordering::SeqCst);
        for from in 1..=6 {
            peer.send(PeerFrame::BranchWant(want_own(&journal, from)))
                .await;
        }
        // Two requests over the cap: answered at once, while the first four
        // are still waiting for the journal.
        for _ in 0..2 {
            let none = peer
                .find(|f| match f {
                    PeerFrame::BranchNone { tip_hash, .. } => Some(tip_hash.clone()),
                    PeerFrame::BranchEntries { .. } => panic!("served before the gate opened"),
                    _ => None,
                })
                .await;
            assert_eq!(none, want_own(&journal, 1).tip.hash);
        }
        journal.branch_gate.store(false, Ordering::SeqCst);
        let mut firsts = Vec::new();
        for _ in 0..4 {
            firsts.push(next_branch_entries(&mut peer).await.1[0]);
        }
        assert_eq!(firsts, [1, 2, 3, 4], "FIFO");
        let frames = peer.pump(2_000).await;
        assert!(
            !frames.iter().any(|f| matches!(
                f,
                PeerFrame::BranchEntries { .. } | PeerFrame::BranchNone { .. }
            )),
            "the over-cap requests got no second answer: {frames:?}"
        );
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn branch_none_when_journal_cannot_serve() {
        let (rig, journal) = node(1, 3, 0);
        let mut peer = connect(&rig, 0, &fork_hello(2)).await;
        peer.expect_have().await;
        let mut wrong_hash = want_own(&journal, 1);
        wrong_hash.tip.hash = "11".repeat(32);
        let mut unknown_log = want_own(&journal, 1);
        unknown_log.inc = 77;
        let mut not_hex = want_own(&journal, 1);
        not_hex.tip.hash = "zz".into();
        for want in [&wrong_hash, &unknown_log, &not_hex] {
            peer.send(PeerFrame::BranchWant(want.clone())).await;
            let (hash, inc) = peer
                .find(|f| match f {
                    PeerFrame::BranchNone { tip_hash, inc, .. } => Some((tip_hash.clone(), *inc)),
                    PeerFrame::BranchEntries { .. } => panic!("served an unservable branch"),
                    _ => None,
                })
                .await;
            assert_eq!((hash, inc), (want.tip.hash.clone(), want.inc));
        }
        // The link still serves a good request afterwards.
        peer.send(PeerFrame::BranchWant(want_own(&journal, 1)))
            .await;
        assert_eq!(next_branch_entries(&mut peer).await.1, [1, 2, 3, 4]);
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn branch_replies_reach_on_branch_with_credit_granted() {
        let (rig, _journal) = node(1, 0, 0);
        let f = foreign(9, 2);
        let mut peer = connect(&rig, 0, &fork_hello(2)).await;
        peer.expect_have().await;
        let frame = PeerFrame::BranchEntries {
            node: f.log.node,
            inc: f.log.incarnation,
            tip_hash: "cc".repeat(32),
            entries: f.envs.iter().map(WireItem::from).collect(),
            more: true,
        };
        let len = wire_len(&frame);
        peer.send(frame).await;
        assert_eq!(
            peer.find_credit().await,
            len,
            "the frame's credit comes back"
        );
        peer.send(PeerFrame::BranchNone {
            node: f.log.node,
            inc: f.log.incarnation,
            tip_hash: "dd".repeat(32),
        })
        .await;
        until(|| rig.sink.branches.lock().unwrap().len() == 2).await;
        {
            let branches = rig.sink.branches.lock().unwrap();
            match &branches[0] {
                (
                    source,
                    BranchReply::Entries {
                        node,
                        inc,
                        tip_hash,
                        entries,
                        more,
                    },
                ) => {
                    assert_eq!(*source, node_of(2));
                    assert_eq!((*node, *inc), (f.log.node, f.log.incarnation));
                    assert_eq!(tip_hash, &"cc".repeat(32));
                    assert_eq!(entries.len(), 3);
                    assert!(*more);
                }
                other => panic!("expected entries, got {other:?}"),
            }
            match &branches[1] {
                (source, BranchReply::None { tip_hash, .. }) => {
                    assert_eq!(*source, node_of(2));
                    assert_eq!(tip_hash, &"dd".repeat(32));
                }
                other => panic!("expected none, got {other:?}"),
            }
        }
        stop(rig).await;
    }

    // -- Reachability and the round-complete signal (ht-5fr.5.6) ----------

    async fn round_is(rig: &Rig, since: MonoInstant, want: RoundState) {
        let signals = rig.handle.signals();
        for _ in 0..200 {
            if signals.round_state(since) == want {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!(
            "round state stayed {:?}, wanted {want:?}",
            signals.round_state(since)
        );
    }

    /// The round state holds `want` through a stretch of virtual time.
    async fn round_stays(rig: &Rig, since: MonoInstant, want: RoundState) {
        for _ in 0..10 {
            assert_eq!(rig.handle.signals().round_state(since), want);
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    fn wall_ms(rig: &Rig) -> u64 {
        u64::try_from(rig.clock.utc_now().0).unwrap()
    }

    fn stored_contact(rig: &Rig, node: NodeUuid) -> Option<NodeContact> {
        rig.store
            .inner
            .contacts()
            .into_iter()
            .find(|c| c.node == node)
    }

    const ZERO: MonoInstant = MonoInstant(0);

    #[tokio::test(start_paused = true)]
    async fn round_pending_then_complete() {
        let (rig, _journal) = rig_with_self_log();
        let mut peer = connect(&rig, 0, &peer_hello(2)).await;
        peer.expect_have().await;
        listed(&rig, 1).await;
        round_stays(&rig, ZERO, RoundState::Pending { links: 1 }).await;
        peer.send_have(true, false, Vec::new()).await;
        round_is(&rig, ZERO, RoundState::Complete).await;
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn a_delta_have_does_not_complete_a_round() {
        let (rig, _journal) = rig_with_self_log();
        let mut peer = connect(&rig, 0, &peer_hello(2)).await;
        peer.expect_have().await;
        peer.send_have(false, false, Vec::new()).await;
        round_stays(&rig, ZERO, RoundState::Pending { links: 1 }).await;
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn request_round_resets_to_pending_until_new_round() {
        let (rig, _journal) = rig_with_self_log();
        let mut peer = connect(&rig, 0, &peer_hello(2)).await;
        peer.expect_have().await;
        peer.send_have(true, false, Vec::new()).await;
        round_is(&rig, ZERO, RoundState::Complete).await;

        tokio::time::sleep(Duration::from_millis(500)).await;
        let at = rig.clock.monotonic_now();
        rig.handle.signals().request_round(at);
        assert_eq!(
            rig.handle.signals().round_state(at),
            RoundState::Pending { links: 1 },
            "the old round started before the request"
        );
        let (full, ask, _) = peer.expect_have().await;
        assert!(full && ask, "the request sends Have{{full, ask: true}}");
        round_stays(&rig, at, RoundState::Pending { links: 1 }).await;
        assert_eq!(
            rig.handle.signals().round_state(ZERO),
            RoundState::Pending { links: 1 },
            "the reset also reopens older readers"
        );
        peer.send_have(true, false, Vec::new()).await;
        round_is(&rig, at, RoundState::Complete).await;
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn no_links_reports_no_links() {
        let (rig, _journal) = rig_with_self_log();
        assert_eq!(rig.handle.signals().round_state(ZERO), RoundState::NoLinks);
        let mut peer = connect(&rig, 0, &peer_hello(2)).await;
        peer.expect_have().await;
        peer.send_have(true, false, Vec::new()).await;
        round_is(&rig, ZERO, RoundState::Complete).await;
        peer.end();
        round_is(&rig, ZERO, RoundState::NoLinks).await;
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn only_the_active_link_of_a_node_counts() {
        let (rig, _journal) = rig_with_self_log();
        let (_active, _standby) = two_links(&rig).await;
        round_stays(&rig, ZERO, RoundState::Pending { links: 1 }).await;
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn request_round_before_any_link_cold_start() {
        let (rig, _journal) = rig_with_self_log();
        let at = rig.clock.monotonic_now();
        rig.handle.signals().request_round(at);
        round_stays(&rig, at, RoundState::NoLinks).await;
        let mut peer = connect(&rig, 0, &peer_hello(2)).await;
        let (full, ask, _) = peer.expect_have().await;
        assert!(full && !ask, "activation sends the ordinary full have");
        listed(&rig, 1).await;
        round_stays(&rig, at, RoundState::Pending { links: 1 }).await;
        peer.send_have(true, false, Vec::new()).await;
        round_is(&rig, at, RoundState::Complete).await;
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn round_waits_for_an_advertised_ahead_log() {
        let (rig, journal) = node(1, 0, 0);
        let f = foreign(9, 3);
        let mut peer = connect(&rig, 0, &peer_hello(2)).await;
        peer.expect_have().await;
        let top = f.envs.len() as u64;
        peer.send_have(true, false, vec![have_log(f.log.node, 1, top)])
            .await;
        peer.find_want(f.log.node, 1, 1).await;
        round_stays(&rig, ZERO, RoundState::Pending { links: 1 }).await;
        peer.send(entries_frame(&f.log, &f.envs[..2])).await;
        until(|| open_vector_counter(&journal, &f.log) == 2).await;
        round_stays(&rig, ZERO, RoundState::Pending { links: 1 }).await;
        peer.send(entries_frame(&f.log, &f.envs[2..])).await;
        until(|| open_vector_counter(&journal, &f.log) == top).await;
        round_is(&rig, ZERO, RoundState::Complete).await;
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn a_penalized_ahead_log_does_not_hold_the_round() {
        let (rig, journal) = node(1, 0, 0);
        let f = foreign(9, 3);
        let mut peer = connect(&rig, 0, &peer_hello(2)).await;
        peer.expect_have().await;
        let top = f.envs.len() as u64;
        peer.send_have(true, false, vec![have_log(f.log.node, 1, top)])
            .await;
        peer.find_want(f.log.node, 1, 1).await;
        round_stays(&rig, ZERO, RoundState::Pending { links: 1 }).await;
        // A forged second entry penalizes the only source of the log.
        let mut forged = f.envs[..2].to_vec();
        forged[1].payload = payload(99, 0);
        peer.send(entries_frame(&f.log, &forged)).await;
        round_is(&rig, ZERO, RoundState::Complete).await;
        assert_eq!(
            open_vector_counter(&journal, &f.log),
            0,
            "the log is still behind: it is unwantable, not caught up"
        );
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn a_paused_ahead_log_does_not_hold_the_round() {
        let (rig, journal) = node(1, 0, 0);
        let f = foreign(9, 2);
        let mut peer = connect(&rig, 0, &peer_hello(2)).await;
        peer.expect_have().await;
        peer.send_have(true, false, vec![have_log(f.log.node, 1, 3)])
            .await;
        peer.find_want(f.log.node, 1, 1).await;
        round_stays(&rig, ZERO, RoundState::Pending { links: 1 }).await;
        // The sink takes a fork on the log: it is paused, not caught up.
        journal.script(Scripted::Outcome(IngestOutcome::Fork {
            stored: 0,
            evidence: evidence(&f),
        }));
        peer.send(entries_frame(&f.log, &f.envs[..1])).await;
        round_is(&rig, ZERO, RoundState::Complete).await;
        assert_eq!(snapshot(&rig).await.paused_logs, vec![f.log]);
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn seen_relative_ages_give_c_relayed_contact_for_a() {
        let (rig, _journal) = rig_with_self_log();
        let (a, b) = (node_of(9), node_of(2));
        let mut link_b = connect(&rig, 0, &peer_hello(2)).await;
        link_b.expect_have().await;
        // B reports itself at 0 and A as last heard 5 s ago.
        let before = wall_ms(&rig);
        link_b
            .send(PeerFrame::Seen {
                ages: vec![
                    SeenAge { node: b, age_ms: 0 },
                    SeenAge {
                        node: a,
                        age_ms: 5_000,
                    },
                ],
            })
            .await;
        tokio::time::sleep(Duration::from_secs(2)).await;
        let after = wall_ms(&rig);
        let for_a = stored_contact(&rig, a).expect("C holds a contact for A");
        assert_eq!(for_a.via, ContactVia::Relayed);
        assert!(
            (before - 5_000..=after - 5_000).contains(&for_a.last_contact_at),
            "A was heard about 5 s before the report: {} not in {}..={}",
            for_a.last_contact_at,
            before - 5_000,
            after - 5_000
        );
        let for_b = stored_contact(&rig, b).expect("C holds a contact for B");
        assert_eq!(for_b.via, ContactVia::Direct, "B's own frame is direct");

        // What C tells B: itself at 0, A by age, never B.
        let frames = link_b.pump(10_000).await;
        let ages = frames
            .iter()
            .find_map(|f| match f {
                PeerFrame::Seen { ages } if ages.iter().any(|age| age.node == a) => {
                    Some(ages.clone())
                }
                _ => None,
            })
            .expect("C sends a seen that lists A within 10 s");
        let me = node_of(1);
        assert_eq!(
            ages[0],
            SeenAge {
                node: me,
                age_ms: 0
            }
        );
        assert!(ages.iter().all(|age| age.node != b), "B is left out");
        let a_age = ages.iter().find(|age| age.node == a).expect("A is listed");
        assert!(
            a_age.age_ms >= 5_000,
            "the age keeps growing from the report: {a_age:?}"
        );
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn line_gives_c_a_relayed_contact_for_a() {
        let (a, ja) = node(1, 1, 0);
        let jb = TestJournal::new(2);
        let b = start_node(2, jb.clone());
        let jc = TestJournal::new(3);
        let c = start_node(3, jc.clone());
        let _ab = link_nodes(&a, &b).await;
        let _bc = link_nodes(&b, &c).await;
        converge(&[&ja, &jb]).await;
        tokio::time::sleep(Duration::from_secs(25)).await;
        assert_eq!(snapshot(&c).await.links.len(), 1, "C has no link to A");
        let for_a = stored_contact(&c, node_of(1)).expect("C holds a contact for A");
        assert_eq!(for_a.via, ContactVia::Relayed);
        assert_eq!(
            stored_contact(&c, node_of(2)).map(|c| c.via),
            Some(ContactVia::Direct)
        );
        assert_eq!(
            stored_contact(&a, node_of(2)).map(|c| c.via),
            Some(ContactVia::Direct)
        );
        let for_c = stored_contact(&a, node_of(3)).expect("A holds a contact for C");
        assert_eq!(for_c.via, ContactVia::Relayed);
        stop(a).await;
        stop(b).await;
        stop(c).await;
    }

    #[tokio::test(start_paused = true)]
    async fn contact_flush_only_on_advance_or_crossing_at_most_every_10s() {
        let (rig, _journal) = rig_with_self_log();
        let mut p2 = connect(&rig, 0, &peer_hello(2)).await;
        p2.expect_have().await;
        let seen = || PeerFrame::Seen { ages: Vec::new() };
        p2.send(seen()).await;
        let first_contact = wall_ms(&rig);
        p2.pump(2_000).await;
        let calls = rig.store.calls();
        assert_eq!(calls.len(), 1, "a never-written row is due at once");
        assert_eq!(calls[0].1.len(), 1);
        assert_eq!(calls[0].1[0].node, node_of(2));
        let c0 = calls[0].1[0].last_contact_at;
        assert!(
            (first_contact..=first_contact + 2_000).contains(&c0),
            "the row carries the frame's time: {c0} vs {first_contact}"
        );

        // Contacts at +10 s and +20 s: under the 30 s advance, nothing is
        // written.
        for _ in 0..2 {
            p2.pump(10_000).await;
            p2.send(seen()).await;
        }
        p2.pump(5_000).await;
        assert_eq!(rig.store.calls().len(), 1, "a 20 s advance is not enough");

        // +31 s: the advance rule fires on the next tick.
        p2.pump(6_000).await;
        p2.send(seen()).await;
        p2.pump(2_000).await;
        let calls = rig.store.calls();
        assert_eq!(calls.len(), 2);
        assert!(calls[1].1[0].last_contact_at - c0 >= 30_000);

        // A second node due right after the flush waits out the 10 s spacing.
        let mut p3 = connect(&rig, 1, &peer_hello(3)).await;
        p3.expect_have().await;
        p3.send(seen()).await;
        p3.pump(5_000).await;
        assert_eq!(rig.store.calls().len(), 2, "within 10 s of the last flush");
        p3.pump(10_000).await;
        let calls = rig.store.calls();
        assert_eq!(calls.len(), 3);
        assert!(calls[2].0 - calls[1].0 >= 10_000, "at most every 10 s");
        assert_eq!(calls[2].1.len(), 1, "only the due row is written");
        assert_eq!(calls[2].1[0].node, node_of(3));
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_contact_flush_retries_at_the_next_flush() {
        let (rig, _journal) = rig_with_self_log();
        rig.store.fail.store(true, Ordering::SeqCst);
        let mut p2 = connect(&rig, 0, &peer_hello(2)).await;
        p2.expect_have().await;
        p2.send(PeerFrame::Seen { ages: Vec::new() }).await;
        p2.pump(3_000).await;
        assert_eq!(rig.store.calls().len(), 1);
        assert!(rig.store.inner.contacts().is_empty(), "the write failed");
        assert!(
            rig.lines
                .lock()
                .unwrap()
                .iter()
                .any(|l| l.contains("contact flush failed")),
            "the failure is logged"
        );
        rig.store.fail.store(false, Ordering::SeqCst);
        p2.pump(12_000).await;
        let calls = rig.store.calls();
        assert_eq!(calls.len(), 2, "retried once the 10 s spacing passed");
        assert!(calls[1].0 - calls[0].0 >= 10_000);
        assert!(stored_contact(&rig, node_of(2)).is_some());
        p2.pump(10_000).await;
        assert_eq!(rig.store.calls().len(), 2, "written, so not due again");
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn malformed_branch_envelope_dropped_and_counted() {
        let (rig, _journal) = node(1, 0, 0);
        let f = foreign(9, 2);
        let mut peer = connect(&rig, 0, &fork_hello(2)).await;
        peer.expect_have().await;
        let mut entries: Vec<WireItem> = f.envs.iter().map(WireItem::from).collect();
        let WireItem::Known(second) = &mut entries[1] else {
            panic!("a known item");
        };
        second.sig = "zz".into();
        let frame = PeerFrame::BranchEntries {
            node: f.log.node,
            inc: f.log.incarnation,
            tip_hash: "cc".repeat(32),
            entries,
            more: false,
        };
        let len = wire_len(&frame);
        peer.send(frame).await;
        assert_eq!(peer.find_credit().await, len, "credit still comes back");
        peer.pump(1_000).await;
        assert!(
            rig.sink.branches.lock().unwrap().is_empty(),
            "the sink never saw the frame"
        );
        assert!(
            rig.lines
                .lock()
                .unwrap()
                .iter()
                .any(|l| l.contains("malformed branch entry") && l.contains("dropped")),
            "logged once on the link: {:?}",
            rig.lines.lock().unwrap()
        );
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn malformed_branch_reply_does_not_penalize_the_rx_source() {
        let (rig, _journal) = node(1, 0, 0);
        let f = foreign(9, 2);
        let mut peer = connect(&rig, 0, &fork_hello(2)).await;
        peer.expect_have().await;
        peer.send_have(
            true,
            false,
            vec![have_log(f.log.node, f.log.incarnation, 3)],
        )
        .await;
        peer.find_want(f.log.node, f.log.incarnation, 1).await;
        let mut entries: Vec<WireItem> = f.envs.iter().map(WireItem::from).collect();
        let WireItem::Known(second) = &mut entries[1] else {
            panic!("a known item");
        };
        second.sig = "zz".into();
        peer.send(PeerFrame::BranchEntries {
            node: f.log.node,
            inc: f.log.incarnation,
            tip_hash: "cc".repeat(32),
            entries,
            more: false,
        })
        .await;
        let frames = peer.pump(2_000).await;
        assert!(
            !frames.iter().any(|f| matches!(f, PeerFrame::Unwant { .. })),
            "the rx subscription was dropped: {frames:?}"
        );
        assert_eq!(snapshot(&rig).await.links[0].rx_logs, 1, "still the source");
        assert!(
            !rig.lines
                .lock()
                .unwrap()
                .iter()
                .any(|l| l.contains("source penalized")),
            "{:?}",
            rig.lines.lock().unwrap()
        );
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn branch_want_on_a_standby_fork_link_gets_branch_none() {
        let (rig, journal) = node(1, 3, 0);
        let (mut active, mut standby) = two_fork_links(&rig).await;
        active.pump(500).await;
        standby.pump(500).await;
        let want = want_own(&journal, 1);
        standby.send(PeerFrame::BranchWant(want.clone())).await;
        let hash = standby
            .find(|f| match f {
                PeerFrame::BranchNone { tip_hash, .. } => Some(tip_hash.clone()),
                PeerFrame::BranchEntries { .. } => panic!("a standby link served a branch"),
                _ => None,
            })
            .await;
        assert_eq!(hash, want.tip.hash);
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn partial_branch_entries_send_no_branch_none() {
        // 40 entries of about 600 bytes at a 4 KiB frame: far more frames than
        // the data queue and the credit window take at once.
        let (rig, journal) = node(1, 40, 600);
        let mut hello = fork_hello(2);
        hello.limits.max_frame = 4_096;
        hello.limits.credit = 8_192;
        let mut peer = connect(&rig, 0, &hello).await;
        peer.expect_have().await;
        peer.send(PeerFrame::BranchWant(want_own(&journal, 1)))
            .await;
        let frames = peer.pump(3_000).await;
        let entries: Vec<bool> = frames
            .iter()
            .filter_map(|f| match f {
                PeerFrame::BranchEntries { more, .. } => Some(*more),
                _ => None,
            })
            .collect();
        assert!(!entries.is_empty(), "some frames were sent: {frames:?}");
        assert_eq!(entries.last(), Some(&true), "the last frame asks for more");
        assert!(
            !frames
                .iter()
                .any(|f| matches!(f, PeerFrame::BranchNone { .. })),
            "branch_none after partial branch_entries: {frames:?}"
        );
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn outbox_sends_only_to_fork_cap_links() {
        let (rig, journal) = node(1, 2, 0);
        let mut fork = connect(&rig, 0, &fork_hello(2)).await;
        fork.expect_have().await;
        let mut plain = connect(&rig, 1, &peer_hello(3)).await;
        plain.expect_have().await;
        let to_fork = want_own(&journal, 1);
        let to_plain = want_own(&journal, 2);
        let to_nobody = want_own(&journal, 3);
        *rig.sink.outbox.lock().unwrap() = ForkOutbox {
            sends: vec![
                (node_of(2), to_fork.clone()),
                (node_of(3), to_plain),
                (node_of(7), to_nobody),
            ],
            refs_changed: false,
        };
        let frames = fork.pump(2_000).await;
        let wants: Vec<&BranchWant> = frames
            .iter()
            .filter_map(|f| match f {
                PeerFrame::BranchWant(w) => Some(w),
                _ => None,
            })
            .collect();
        assert_eq!(wants, [&to_fork]);
        let frames = plain.pump(500).await;
        assert!(
            !frames.iter().any(|f| matches!(f, PeerFrame::BranchWant(_))),
            "a link without fork.v1 got a branch_want: {frames:?}"
        );
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn standby_frames_record_direct_contact() {
        let (rig, _journal) = rig_with_self_log();
        let (mut active, mut standby) = two_links(&rig).await;
        active.pump(3_000).await;
        assert!(
            rig.store.inner.contacts().is_empty(),
            "no frame has arrived yet"
        );
        standby.send(PeerFrame::Seen { ages: Vec::new() }).await;
        standby.pump(3_000).await;
        let held = stored_contact(&rig, node_of(2)).expect("the standby's frame is contact");
        assert_eq!(held.via, ContactVia::Direct);
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn refs_changed_one_delta() {
        let (rig, _journal) = node(1, 0, 0);
        let mut fork = connect(&rig, 0, &fork_hello(2)).await;
        fork.expect_have().await;
        let mut plain = connect(&rig, 1, &peer_hello(3)).await;
        plain.expect_have().await;
        fork.pump(1_500).await;
        plain.pump(100).await;

        *rig.sink.refs.lock().unwrap() = vec![fork_ref(9)];
        rig.sink.outbox.lock().unwrap().refs_changed = true;
        let deltas = |frames: &[PeerFrame]| -> Vec<Vec<ForkRef>> {
            frames
                .iter()
                .filter_map(|f| match f {
                    PeerFrame::Have {
                        full: false,
                        ask: false,
                        logs,
                        forks,
                    } if logs.is_empty() => Some(forks.clone()),
                    _ => None,
                })
                .collect()
        };
        let frames = fork.pump(5_000).await;
        assert_eq!(deltas(&frames), [vec![fork_ref(9)]], "exactly one delta");
        let frames = plain.pump(100).await;
        assert!(
            !frames.iter().any(|f| matches!(f, PeerFrame::Have { .. })),
            "a link without fork.v1 got a delta: {frames:?}"
        );
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn a_standby_ping_is_contact_too() {
        let (rig, _journal) = rig_with_self_log();
        let (mut active, mut standby) = two_links(&rig).await;
        active.pump(3_000).await;
        assert!(rig.store.inner.contacts().is_empty());
        standby.send(PeerFrame::Ping { n: 7 }).await;
        standby.pump(3_000).await;
        assert!(
            stored_contact(&rig, node_of(2)).is_some(),
            "the link answers the ping itself; the frame count still shows it"
        );
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn full_have_carries_fork_refs_only_to_fork_cap_peers() {
        let (rig, _journal) = node(1, 0, 0);
        *rig.sink.refs.lock().unwrap() = vec![fork_ref(9)];
        let mut fork = connect(&rig, 0, &fork_hello(2)).await;
        let mut plain = connect(&rig, 1, &peer_hello(3)).await;
        let forks_of = |frame: PeerFrame| match frame {
            PeerFrame::Have { full, forks, .. } => {
                assert!(full);
                forks
            }
            other => panic!("expected have, got {other:?}"),
        };
        assert_eq!(forks_of(fork.next().await), vec![fork_ref(9)]);
        assert!(forks_of(plain.next().await).is_empty());
        stop(rig).await;
    }

    #[tokio::test(start_paused = true)]
    async fn final_flush_on_cancel_writes_what_the_spacing_held_back() {
        let (rig, _journal) = rig_with_self_log();
        let seen = || PeerFrame::Seen { ages: Vec::new() };
        let mut p2 = connect(&rig, 0, &peer_hello(2)).await;
        p2.expect_have().await;
        p2.send(seen()).await;
        p2.pump(1_500).await;
        assert_eq!(rig.store.calls().len(), 1);
        let mut p3 = connect(&rig, 1, &peer_hello(3)).await;
        p3.expect_have().await;
        p3.send(seen()).await;
        p3.pump(1_000).await;
        assert_eq!(rig.store.calls().len(), 1, "held back by the spacing");
        assert!(stored_contact(&rig, node_of(3)).is_none());
        let store = rig.store.clone();
        stop(rig).await;
        assert_eq!(store.calls().len(), 2, "the final flush made one call");
        assert!(
            store.inner.contacts().iter().any(|c| c.node == node_of(3)),
            "node 3 reached the store at shutdown"
        );
    }

    // -- Node status (ht-xg88) ----------------------------------------------

    fn info_hello(n: u8) -> LocalHello {
        let mut hello = peer_hello(n);
        hello.caps.push(NODE_INFO_CAP.to_owned());
        hello
    }

    // Kills: node_info sent to a peer without the cap (an older peer sees a
    // new frame), not sent to one with it, a peer version kept without the
    // cap, and a version that never reaches the snapshot.
    #[tokio::test(start_paused = true)]
    async fn node_info_goes_only_to_a_peer_with_the_cap() {
        let (rig, _journal) = node(1, 0, 0);
        let mut with = connect(&rig, 0, &info_hello(2)).await;
        match with.next().await {
            PeerFrame::NodeInfo(info) => assert_eq!(info.version, env!("CARGO_PKG_VERSION")),
            other => panic!("expected node_info, got {other:?}"),
        }
        with.expect_have().await;
        // Not a version: ignored. The first plausible one is kept, a later
        // one is not.
        for version in ["2.4 node X linked", "9.9.9", "1.1.1"] {
            with.send(PeerFrame::NodeInfo(NodeInfo {
                version: version.into(),
            }))
            .await;
        }
        let mut without = connect(&rig, 1, &peer_hello(3)).await;
        // The first frame after the handshake is the activation have.
        without.expect_have().await;
        without
            .send(PeerFrame::NodeInfo(NodeInfo {
                version: "1.0.0".into(),
            }))
            .await;
        with.pump(500).await;
        assert!(
            without
                .pump(500)
                .await
                .iter()
                .all(|f| !matches!(f, PeerFrame::NodeInfo(_)))
        );
        let snap = snapshot(&rig).await;
        let version = |n: u8| {
            snap.links
                .iter()
                .find(|l| l.node == node_of(n))
                .and_then(|l| l.peer_version.clone())
        };
        assert_eq!(version(2).as_deref(), Some("9.9.9"));
        assert_eq!(version(3), None, "ignored without the cap");
        stop(rig).await;
    }

    // Kills: a stall reported while the peer is caught up or before the
    // threshold, app contact advanced by ping/pong (a wedged coordinator
    // behind a live link reads healthy), a tx or rx stall that never clears
    // on progress, and positions missing from the snapshot.
    #[tokio::test(start_paused = true)]
    async fn stalls_are_reported_and_clear_on_progress() {
        let journal = TestJournal::new(1);
        let own = journal.inner.bootstrap();
        journal
            .inner
            .author_raw("test.kind", payload(0, 0))
            .unwrap();
        let rig = start_node_full(
            1,
            journal.clone(),
            None,
            CoordinatorOptions {
                timings: StallTimings {
                    stall_after_ms: 6_000,
                    app_silent_after_ms: 3_000,
                    ..StallTimings::default()
                },
                ..CoordinatorOptions::default()
            },
        );
        let head = open_vector_counter(&journal, &own);
        assert!(head >= 2);
        let mut peer = connect(&rig, 0, &peer_hello(2)).await;
        peer.expect_have().await;
        // The peer holds none of our log and advertises 5 of its own.
        peer.send_have(true, false, vec![have_log(node_of(2), 1, 5)])
            .await;
        peer.pump(1_000).await;
        let link = snapshot(&rig).await.links[0].clone();
        assert_eq!(link.tx, Some((0, head)));
        assert_eq!(link.rx, Some((0, 5)));
        assert_eq!(link.stalled, Stall::default());

        // Only ping/pong for longer than the app threshold (pump answers
        // pings): the transport is alive, the coordinator is not.
        peer.pump(3_500).await;
        let link = snapshot(&rig).await.links[0].clone();
        assert!(link.stalled.app, "{link:?}");
        assert!(!link.stalled.tx && !link.stalled.rx, "{link:?}");
        assert!(
            link.app_rx.is_some_and(|app| link.last_rx.0 > app.0),
            "pings move last_rx only"
        );

        // Coordinator traffic again, but no acknowledgement and nothing
        // ingested past the stall threshold.
        for _ in 0..3 {
            peer.send(PeerFrame::Seen { ages: vec![] }).await;
            peer.pump(1_000).await;
        }
        let link = snapshot(&rig).await.links[0].clone();
        assert_eq!(
            link.stalled,
            Stall {
                app: false,
                tx: true,
                rx: true
            }
        );

        // Partial progress: the peer acknowledges some of our log, still
        // behind; tx is no longer stalled (rx still is).
        peer.send_have(
            true,
            false,
            vec![
                have_log(own.node, own.incarnation, 1),
                have_log(node_of(2), 1, 5),
            ],
        )
        .await;
        peer.pump(1_500).await;
        let link = snapshot(&rig).await.links[0].clone();
        assert_eq!(link.tx, Some((1, head)));
        assert!(!link.stalled.tx && link.stalled.rx, "{link:?}");

        // The peer acknowledges our head and stops advertising its log.
        peer.send_have(true, false, vec![have_log(own.node, own.incarnation, head)])
            .await;
        peer.pump(1_500).await;
        let link = snapshot(&rig).await.links[0].clone();
        assert_eq!(link.tx, Some((head, head)));
        assert_eq!(link.stalled, Stall::default(), "{link:?}");
        stop(rig).await;
    }

    // Kills: no peer-status row for a linked node, a row lost or its version
    // wiped when the link ends, and a failing store retried every tick or
    // logged on every retry.
    #[tokio::test(start_paused = true)]
    async fn peer_status_rows_follow_the_link_and_back_off() {
        let (rig, _journal) = node(1, 0, 0);
        rig.store.peer_fail.store(true, Ordering::SeqCst);
        let mut peer = connect(&rig, 0, &info_hello(2)).await;
        peer.next().await;
        peer.expect_have().await;
        peer.send(PeerFrame::NodeInfo(NodeInfo {
            version: "2.4.0".into(),
        }))
        .await;
        peer.pump(5_000).await;
        assert_eq!(
            rig.store.peer_calls.load(Ordering::SeqCst),
            1,
            "one try, then the backoff"
        );
        let failures = || {
            rig.lines
                .lock()
                .unwrap()
                .iter()
                .filter(|l| l.contains("peer status flush failed"))
                .count()
        };
        peer.pump(10_000).await;
        assert_eq!(rig.store.peer_calls.load(Ordering::SeqCst), 2);
        assert_eq!(failures(), 1, "logged once per run of failures");
        rig.store.peer_fail.store(false, Ordering::SeqCst);
        peer.pump(11_000).await;
        let row = |store: &CountingStore| {
            store
                .inner
                .peer_status()
                .into_iter()
                .find(|r| r.node == node_of(2))
        };
        let up = row(&rig.store).expect("a row once the store takes it");
        assert_eq!(up.version.as_deref(), Some("2.4.0"));
        assert!(
            rig.lines
                .lock()
                .unwrap()
                .iter()
                .any(|l| l.contains("peer status writes recovered"))
        );
        peer.end();
        for _ in 0..10 {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let down = row(&rig.store).unwrap();
        assert_eq!(
            down.version.as_deref(),
            Some("2.4.0"),
            "kept after the link"
        );
        stop(rig).await;
    }

    // -- Seat status (ht-xg88 phase 2) --------------------------------------

    fn seat_hello(n: u8) -> LocalHello {
        let mut hello = peer_hello(n);
        hello.caps.push(SEAT_STATUS_CAP.to_owned());
        hello
    }

    fn seat_entry(seat: &str, presence: &str) -> SeatStatusEntry {
        SeatStatusEntry {
            seat: seat.into(),
            presence: presence.into(),
            harness: Some("claude".into()),
            activity_age_ms: Some(5_000),
            workspace: Some("office".into()),
            tab: Some("main".into()),
            pane: Some("claude".into()),
        }
    }

    fn seat_frame(origin: NodeUuid, boot: u128, rev: u64, age_ms: u64) -> PeerFrame {
        PeerFrame::SeatStatus(SeatStatusFrame {
            origin,
            boot: Uuid::from_u128(boot),
            rev,
            age_ms,
            version: "2.4.0".into(),
            seats: vec![seat_entry("sQ0VnTDasAbY", "idle")],
        })
    }

    fn seat_frames(frames: &[PeerFrame]) -> Vec<SeatStatusFrame> {
        frames
            .iter()
            .filter_map(|f| match f {
                PeerFrame::SeatStatus(frame) => Some(frame.clone()),
                _ => None,
            })
            .collect()
    }

    // Kills: own seats sent to a peer without the cap, not sent to a newly
    // ready link, a publish that reuses a rev (receivers would drop it as a
    // repeat), and `seat_status_wanted` never set.
    #[tokio::test(start_paused = true)]
    async fn own_seats_go_to_cap_links_with_a_rev_per_change() {
        let (rig, _journal) = node(1, 0, 0);
        let mut with = connect(&rig, 0, &seat_hello(2)).await;
        with.expect_have().await;
        let mut without = connect(&rig, 1, &peer_hello(3)).await;
        without.expect_have().await;
        with.pump(1_500).await;
        assert!(rig.handle.seat_status_wanted());
        rig.handle
            .publish_seat_status(vec![seat_entry("snSGbl096JZZ", "busy")]);
        let first = seat_frames(&with.pump(1_500).await);
        assert_eq!(first.len(), 1, "{first:?}");
        assert_eq!((first[0].origin, first[0].rev), (node_of(1), 1));
        assert_eq!(first[0].seats[0].presence, "busy");
        rig.handle
            .publish_seat_status(vec![seat_entry("snSGbl096JZZ", "busy")]);
        rig.handle
            .publish_seat_status(vec![seat_entry("snSGbl096JZZ", "idle")]);
        let revs: Vec<u64> = seat_frames(&with.pump(1_500).await)
            .iter()
            .map(|f| f.rev)
            .collect();
        assert_eq!(
            revs,
            [2, 3],
            "every publish, a refresh too, is the next rev"
        );
        assert!(seat_frames(&without.pump(500).await).is_empty());
        // A link that becomes ready later gets the current snapshot.
        let mut late = connect(&rig, 2, &seat_hello(4)).await;
        late.expect_have().await;
        let got = seat_frames(&late.pump(1_500).await);
        assert_eq!(got.iter().map(|f| f.rev).collect::<Vec<_>>(), [3]);
        stop(rig).await;
    }

    // Kills: a snapshot of this node's own origin accepted, an older or
    // repeated one accepted or relayed, a newer one not stored or not
    // relayed, a relay back to the sender or to the origin's own node, and
    // a second live boot not recorded.
    #[tokio::test(start_paused = true)]
    async fn relayed_snapshots_are_ordered_relayed_and_stored() {
        let (rig, _journal) = node(1, 0, 0);
        let mut a = connect(&rig, 0, &seat_hello(2)).await;
        a.expect_have().await;
        let mut b = connect(&rig, 1, &seat_hello(3)).await;
        b.expect_have().await;
        a.pump(1_500).await;
        b.pump(100).await;
        let origin = node_of(9);
        // An origin this node has never heard of is not taken.
        a.send(seat_frame(origin, 1, 5, 2_000)).await;
        assert!(seat_frames(&b.pump(1_500).await).is_empty());
        a.send(PeerFrame::Seen {
            ages: vec![SeenAge {
                node: origin,
                age_ms: 0,
            }],
        })
        .await;
        a.send(seat_frame(origin, 1, 5, 2_000)).await;
        a.send(seat_frame(origin, 1, 4, 0)).await; // older rev: dropped
        a.send(seat_frame(origin, 1, 5, 2_000)).await; // repeat: dropped
        a.send(seat_frame(node_of(1), 1, 9, 0)).await; // our own origin
        let relayed = seat_frames(&b.pump(1_500).await);
        assert_eq!(relayed.len(), 1, "{relayed:?}");
        assert_eq!((relayed[0].origin, relayed[0].rev), (origin, 5));
        assert!(
            relayed[0].age_ms >= 2_000,
            "age accumulates: {}",
            relayed[0].age_ms
        );
        assert!(seat_frames(&a.pump(200).await).is_empty(), "never back");
        let updates = rig.store.inner.seat_updates();
        assert_eq!(updates.len(), 1, "{updates:?}");
        assert_eq!(updates[0].origin, origin);
        assert_eq!(updates[0].rev, 5);
        assert_eq!(updates[0].seats[0].seat, "sQ0VnTDasAbY");
        assert_eq!(updates[0].other_boot_at, None);
        // A restart: the new boot, built later, wins, and the old boot (all
        // of whose builds precede it) is no clone.
        a.send(seat_frame(origin, 2, 1, 0)).await;
        b.pump(1_500).await;
        let updates = rig.store.inner.seat_updates();
        let last = updates.last().unwrap();
        assert_eq!((last.boot, last.rev), (Uuid::from_u128(2), 1));
        assert_eq!(last.other_boot_at, None, "{last:?}");
        // A stale copy of the old boot arriving late: dropped, no clone.
        a.send(seat_frame(origin, 1, 6, 60_000)).await;
        b.pump(1_500).await;
        assert_eq!(rig.store.inner.seat_updates().len(), updates.len());
        // The old boot alive again, built after the new one started: a
        // clone; the newest build wins and carries the suspicion.
        a.send(seat_frame(origin, 1, 7, 0)).await;
        b.pump(1_500).await;
        let last = rig.store.inner.seat_updates().last().unwrap().clone();
        assert_eq!((last.boot, last.rev), (Uuid::from_u128(1), 7));
        assert!(last.other_boot_at.is_some(), "{last:?}");
        stop(rig).await;
    }

    // Kills: a snapshot sent over a link's negotiated frame size (the peer
    // would close the link) instead of truncated.
    #[tokio::test(start_paused = true)]
    async fn own_snapshot_fits_the_link_frame_size() {
        let (rig, _journal) = node(1, 0, 0);
        let mut hello = seat_hello(2);
        hello.limits.max_frame = crate::sync::frame::MIN_PEER_MAX_FRAME;
        let mut peer = connect(&rig, 0, &hello).await;
        peer.expect_have().await;
        peer.pump(1_500).await;
        let big: Vec<SeatStatusEntry> = (0..200)
            .map(|i| SeatStatusEntry {
                workspace: Some("w".repeat(64)),
                tab: Some("t".repeat(64)),
                pane: Some("p".repeat(64)),
                ..seat_entry(&format!("s{i:011}"), "idle")
            })
            .collect();
        rig.handle.publish_seat_status(big);
        let got = seat_frames(&peer.pump(1_500).await);
        assert_eq!(got.len(), 1);
        let size = PeerFrame::SeatStatus(got[0].clone())
            .encode()
            .unwrap()
            .len();
        assert!(
            size <= crate::sync::frame::MIN_PEER_MAX_FRAME as usize,
            "{size}"
        );
        assert!(!got[0].seats.is_empty() && got[0].seats.len() < 200);
        stop(rig).await;
    }
}
