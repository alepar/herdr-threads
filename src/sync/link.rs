//! `PeerLink`: the link runtime over any byte stream.
//!
//! Owns: the preamble search, the mutually signed handshake, framing on both
//! halves, the byte credit window for `entries` frames, ping/pong liveness and
//! the single end-of-link report. It handles `ping`, `pong`, `credit` and
//! `unknown` itself and forwards every other frame upward as a
//! [`LinkEvent::Frame`]. Hides: the control/data queues, both credit counters
//! (sending window and pending grants) and the timers. Must not: route frames, decide what a frame means, dial or
//! accept connections, or touch the store (known peer keys come in through
//! [`LocalHello::known_pubkey`]). Where new behaviour goes: a new frame is a
//! new `PeerFrame` variant behind a capability; this runtime forwards it to
//! the coordinator, which owns what it means. Role: the transport leaf under
//! the sync coordinator and the dialer; generic over
//! `AsyncRead + AsyncWrite + Unpin + Send` (ssh child stdio, accepted
//! `UnixStream` halves, `tokio::io::duplex` in tests), never a trait object.
//!
//! Core and harness-neutral. No version is compared anywhere; the only number
//! in the hello that is checked is the envelope generation.

use std::fmt;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::sync::{Notify, mpsc};
use tokio::time::{Instant, sleep_until, timeout, timeout_at};
use uuid::Uuid;

use super::envelope::{hex_decode, hex_encode};
use super::frame::{
    BASE_CAP, CloseClass, CloseReason, MAX_CAP_BYTES, MAX_FRAME_ENTRIES, MAX_HELLO_BYTES,
    MAX_HELLO_CAPS, MIN_PEER_MAX_FRAME, PREAMBLE, PREAMBLE_NOISE_WINDOW, PeerFrame, PeerHello,
    PeerLimits, Role, write_peer_frame,
};
use crate::daemon::transport::{self, MAX_FRAME_BYTES};
use crate::node::{NodeIdentity, NodeTag, NodeUuid};
use crate::protocol::time::{Cancellation, Clock};

/// The handshake must finish within this long.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
/// Silence before a side sends a `ping`.
pub const PING_AFTER: Duration = Duration::from_secs(30);
/// Silence after which the link is dead.
pub const DEAD_AFTER: Duration = Duration::from_secs(90);
/// How long a closing side gets to write its `close`.
pub const CLOSE_WRITE_TIMEOUT: Duration = Duration::from_secs(1);
/// The envelope generation this build speaks.
pub const ENVELOPE_GENERATION: u32 = 1;
/// Largest credit window, in bytes.
pub const MAX_CREDIT: u32 = 1 << 20;
/// Largest `entries` batch in bytes.
pub const MAX_BATCH_BYTES: u32 = 256 * 1024;
/// Queued control frames per link.
pub const CONTROL_QUEUE: usize = 256;
/// Queued pre-encoded `entries` frames per link.
pub const DATA_QUEUE: usize = 4;

const AUTH_DOMAIN: &[u8] = b"HTPEER1-AUTH\0";

/// Why a link or a handshake ended. Closed: callers match it.
#[derive(Debug)]
pub enum LinkError {
    Io(io::Error),
    /// A frame that did not decode or exceeded the negotiated size.
    Decode(String),
    PreambleNoise,
    PreambleTimeout,
    PreambleEof,
    HandshakeTimeout,
    Dead,
    /// The peer sent `close`.
    PeerClosed(CloseReason),
    /// This side sent `close` (or closed on purpose).
    Refused(CloseReason),
    /// This side shut down (cancel, dropped handle or closed receiver).
    Shutdown,
}

impl LinkError {
    /// What a dialer does next: the close variants delegate to
    /// [`CloseReason::class`], everything else retries.
    pub fn class(&self) -> CloseClass {
        match self {
            Self::PeerClosed(reason) | Self::Refused(reason) => reason.class(),
            _ => CloseClass::Retry,
        }
    }
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "peer link i/o: {e}"),
            Self::Decode(e) => write!(f, "peer link decode: {e}"),
            Self::PreambleNoise => f.write_str("no peer preamble in the first 64 KiB"),
            Self::PreambleTimeout => f.write_str("no peer preamble in time"),
            Self::PreambleEof => f.write_str("stream ended before the peer preamble"),
            Self::HandshakeTimeout => f.write_str("peer handshake timed out"),
            Self::Dead => f.write_str("peer link silent for too long"),
            Self::PeerClosed(r) => write!(f, "peer closed the link: {r:?}"),
            Self::Refused(r) => write!(f, "link closed locally: {r:?}"),
            Self::Shutdown => f.write_str("peer link shut down"),
        }
    }
}

impl std::error::Error for LinkError {}

impl From<io::Error> for LinkError {
    fn from(e: io::Error) -> Self {
        if e.kind() == io::ErrorKind::InvalidData {
            Self::Decode(e.to_string())
        } else {
            Self::Io(e)
        }
    }
}

/// Read bytes one at a time until the `HTPEER1\n` line has gone by; returns
/// how many noise bytes came before it (the dialer logs a non-zero count
/// once). Reads nothing past the line.
pub async fn read_preamble<R: AsyncRead + Unpin>(
    reader: &mut R,
    deadline: Instant,
) -> Result<usize, LinkError> {
    let scan = async {
        let mut tail = [0u8; PREAMBLE.len()];
        let mut read = 0usize;
        loop {
            let byte = match reader.read_u8().await {
                Ok(b) => b,
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                    return Err(LinkError::PreambleEof);
                }
                Err(e) => return Err(LinkError::Io(e)),
            };
            tail.rotate_left(1);
            tail[PREAMBLE.len() - 1] = byte;
            read += 1;
            if read >= PREAMBLE.len() && &tail == PREAMBLE {
                return Ok(read - PREAMBLE.len());
            }
            if read >= PREAMBLE_NOISE_WINDOW + PREAMBLE.len() {
                return Err(LinkError::PreambleNoise);
            }
        }
    };
    timeout_at(deadline, scan)
        .await
        .unwrap_or(Err(LinkError::PreambleTimeout))
}

/// Everything this side contributes to a handshake.
#[derive(Clone)]
pub struct LocalHello {
    pub identity: Arc<NodeIdentity>,
    /// A random id drawn once per daemon process. The `self_link` versus
    /// `node_collision` split assumes each copy of a cloned node starts its
    /// own daemon process after the copy; memory-preserving clones (a live
    /// VM snapshot, a fork of a running daemon) keep the boot id and are out
    /// of scope (peer-link design section 5, root section 12).
    pub boot: Uuid,
    pub caps: Vec<String>,
    pub limits: PeerLimits,
    /// The public key already known for a node (its genesis key), if any.
    /// The daemon wires this from the node directory; this module stays
    /// store-free. A hello whose key differs from a known one is refused.
    pub known_pubkey: Arc<dyn Fn(NodeUuid) -> Option<[u8; 32]> + Send + Sync>,
}

impl LocalHello {
    /// The default offer: the base capability and the largest limits.
    pub fn new(identity: Arc<NodeIdentity>, boot: Uuid) -> Self {
        Self {
            identity,
            boot,
            caps: vec![BASE_CAP.to_owned()],
            limits: PeerLimits {
                max_frame: MAX_FRAME_BYTES as u32,
                batch_entries: MAX_FRAME_ENTRIES as u32,
                batch_bytes: MAX_BATCH_BYTES,
                credit: MAX_CREDIT,
            },
            known_pubkey: Arc::new(|_| None),
        }
    }
}

/// What the handshake learned about the other side. `caps` and `limits` are
/// the effective (intersected / minimum) values.
#[derive(Clone, Debug)]
pub struct PeerInfo {
    pub node: NodeUuid,
    pub tag: NodeTag,
    pub pubkey: [u8; 32],
    pub boot: Uuid,
    pub caps: Vec<String>,
    pub limits: PeerLimits,
    pub role: Role,
    pub nonce: [u8; 32],
    /// The node that dialed this link: the peer when `role` is `Dialer`,
    /// this node otherwise. Both ends derive the same value, so the
    /// duplicate-link order (peer-link design section 8) needs no frame.
    pub dialer_node: NodeUuid,
    /// The dialer's hello nonce (this side's own nonce when it dialed).
    pub dialer_nonce: [u8; 32],
}

/// A finished handshake: the peer and the two stream halves, ready for
/// [`spawn`].
pub struct Established<T> {
    pub peer: PeerInfo,
    pub reader: ReadHalf<T>,
    pub writer: WriteHalf<T>,
}

fn auth_digest(own_hello: &[u8], peer_hello: &[u8], peer_nonce: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(AUTH_DOMAIN);
    hasher.update((own_hello.len() as u32).to_be_bytes());
    hasher.update(own_hello);
    hasher.update((peer_hello.len() as u32).to_be_bytes());
    hasher.update(peer_hello);
    hasher.update(peer_nonce);
    hasher.finalize().into()
}

async fn write_raw<W: AsyncWrite + Unpin>(writer: &mut W, body: &[u8]) -> io::Result<()> {
    transport::write_frame(writer, body).await?;
    writer.flush().await
}

async fn write_frame_flushed<W: AsyncWrite + Unpin>(
    writer: &mut W,
    frame: &PeerFrame,
) -> io::Result<()> {
    write_peer_frame(writer, frame).await?;
    writer.flush().await
}

/// Send `close` (best effort, bounded) and report the local refusal.
async fn refuse<W: AsyncWrite + Unpin>(writer: &mut W, reason: CloseReason) -> LinkError {
    let close = PeerFrame::Close {
        reason,
        detail: None,
    };
    let _ = timeout(CLOSE_WRITE_TIMEOUT, write_frame_flushed(writer, &close)).await;
    LinkError::Refused(reason)
}

/// Read the next handshake frame; a `close` becomes `PeerClosed`.
async fn read_handshake_frame<R: AsyncRead + Unpin>(
    reader: &mut R,
    limit: usize,
) -> Result<(Vec<u8>, PeerFrame), LinkError> {
    let body = transport::read_frame_with_limit(reader, limit).await?;
    let frame = PeerFrame::decode(&body).map_err(|e| LinkError::Decode(e.to_string()))?;
    if let PeerFrame::Close { reason, .. } = frame {
        return Err(LinkError::PeerClosed(reason));
    }
    Ok((body, frame))
}

fn random_nonce() -> io::Result<[u8; 32]> {
    let mut nonce = [0u8; 32];
    getrandom::fill(&mut nonce).map_err(io::Error::other)?;
    Ok(nonce)
}

/// Run the mutually signed handshake over `io` within `deadline`.
///
/// The dialer sends its hello first; the dialed side passes the dialer's
/// already-sniffed hello frame as `first_frame` and answers. Both sides check
/// the peer hello (envelope generation, key-to-node binding, known key, self
/// versus clone, hello shape), exchange `Auth` signatures over both raw hello
/// frames and the peer's nonce, then exchange `Accept`. A refusal sends
/// `close` and returns `Refused`; a `close` from the peer returns
/// `PeerClosed`. Roles are not enforced beyond who speaks first.
pub async fn handshake<T>(
    io: T,
    me: &LocalHello,
    role: Role,
    first_frame: Option<Vec<u8>>,
    deadline: Instant,
) -> Result<Established<T>, LinkError>
where
    T: AsyncRead + AsyncWrite + Unpin + Send,
{
    let (mut reader, mut writer) = tokio::io::split(io);
    let run = run_handshake(&mut reader, &mut writer, me, role, first_frame);
    match timeout_at(deadline, run).await {
        Ok(Ok(peer)) => Ok(Established {
            peer,
            reader,
            writer,
        }),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(LinkError::HandshakeTimeout),
    }
}

async fn run_handshake<R, W>(
    reader: &mut R,
    writer: &mut W,
    me: &LocalHello,
    role: Role,
    first_frame: Option<Vec<u8>>,
) -> Result<PeerInfo, LinkError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let own_nonce = random_nonce()?;
    let own_hello = PeerHello {
        envelope: ENVELOPE_GENERATION,
        node: me.identity.uuid(),
        tag: me.identity.tag(),
        pubkey: hex_encode(&me.identity.public_key()),
        boot: me.boot,
        caps: me.caps.clone(),
        limits: me.limits.clone(),
        nonce: hex_encode(&own_nonce),
        role,
    };
    let own_bytes = PeerFrame::Hello(own_hello)
        .encode()
        .map_err(|e| LinkError::Decode(e.to_string()))?;

    if role == Role::Dialer {
        write_raw(writer, &own_bytes).await?;
    }
    let peer_bytes = match first_frame {
        Some(bytes) => bytes,
        None => transport::read_frame_with_limit(reader, MAX_FRAME_BYTES).await?,
    };
    let (hello, peer_pubkey, peer_nonce) = match check_peer_hello(me, &peer_bytes) {
        Ok(checked) => checked,
        Err(HelloRefusal::PeerClosed(reason)) => return Err(LinkError::PeerClosed(reason)),
        Err(HelloRefusal::Refuse(reason)) => return Err(refuse(writer, reason).await),
    };
    if role == Role::Dialed {
        write_raw(writer, &own_bytes).await?;
    }

    // Sign the peer's nonce over both exact hellos, then verify theirs.
    let digest = auth_digest(&own_bytes, &peer_bytes, &peer_nonce);
    let sig = hex_encode(&me.identity.sign(&digest));
    write_frame_flushed(writer, &PeerFrame::Auth { sig }).await?;

    let (_, frame) = read_handshake_frame(reader, MAX_FRAME_BYTES).await?;
    let PeerFrame::Auth { sig } = frame else {
        return Err(refuse(writer, CloseReason::ProtocolError).await);
    };
    let expected = auth_digest(&peer_bytes, &own_bytes, &own_nonce);
    let verified = hex_decode::<64>(&sig).is_some_and(|raw| {
        VerifyingKey::from_bytes(&peer_pubkey)
            .and_then(|key| key.verify_strict(&expected, &Signature::from_bytes(&raw)))
            .is_ok()
    });
    if !verified {
        return Err(refuse(writer, CloseReason::AuthFailed).await);
    }

    write_frame_flushed(writer, &PeerFrame::Accept).await?;
    let (_, frame) = read_handshake_frame(reader, MAX_FRAME_BYTES).await?;
    if !matches!(frame, PeerFrame::Accept) {
        return Err(refuse(writer, CloseReason::ProtocolError).await);
    }

    Ok(PeerInfo {
        node: hello.node,
        tag: hello.tag,
        pubkey: peer_pubkey,
        boot: hello.boot,
        caps: me
            .caps
            .iter()
            .filter(|cap| hello.caps.contains(cap))
            .cloned()
            .collect(),
        limits: effective_limits(&me.limits, &hello.limits),
        role: hello.role,
        nonce: peer_nonce,
        dialer_node: if role == Role::Dialer {
            me.identity.uuid()
        } else {
            hello.node
        },
        dialer_nonce: if role == Role::Dialer {
            own_nonce
        } else {
            peer_nonce
        },
    })
}

enum HelloRefusal {
    PeerClosed(CloseReason),
    Refuse(CloseReason),
}

/// The peer hello's checks, in the spec's order, before any `Auth` is sent.
fn check_peer_hello(
    me: &LocalHello,
    bytes: &[u8],
) -> Result<(PeerHello, [u8; 32], [u8; 32]), HelloRefusal> {
    use HelloRefusal::{PeerClosed, Refuse};
    if bytes.len() > MAX_HELLO_BYTES {
        return Err(Refuse(CloseReason::ProtocolError));
    }
    let Some(hello) = PeerFrame::decode_hello(bytes) else {
        return Err(match PeerFrame::decode(bytes) {
            Ok(PeerFrame::Close { reason, .. }) => PeerClosed(reason),
            _ => Refuse(CloseReason::ProtocolError),
        });
    };
    if hello.envelope != ENVELOPE_GENERATION {
        return Err(Refuse(CloseReason::EnvelopeUnsupported));
    }
    let Some(pubkey) = hex_decode::<32>(&hello.pubkey) else {
        return Err(Refuse(CloseReason::AuthFailed));
    };
    if NodeUuid::from_public_key(&pubkey) != hello.node {
        return Err(Refuse(CloseReason::AuthFailed));
    }
    if (me.known_pubkey)(hello.node).is_some_and(|known| known != pubkey) {
        return Err(Refuse(CloseReason::AuthFailed));
    }
    if hello.node == me.identity.uuid() {
        return Err(Refuse(if hello.boot == me.boot {
            CloseReason::SelfLink
        } else {
            CloseReason::NodeCollision
        }));
    }
    if hello.caps.len() > MAX_HELLO_CAPS || hello.caps.iter().any(|c| c.len() > MAX_CAP_BYTES) {
        return Err(Refuse(CloseReason::ProtocolError));
    }
    let l = &hello.limits;
    if l.max_frame < MIN_PEER_MAX_FRAME
        || l.credit < MIN_PEER_MAX_FRAME
        || l.batch_entries == 0
        || l.batch_bytes == 0
    {
        return Err(Refuse(CloseReason::ProtocolError));
    }
    let Some(nonce) = hex_decode::<32>(&hello.nonce) else {
        return Err(Refuse(CloseReason::ProtocolError));
    };
    Ok((hello, pubkey, nonce))
}

fn effective_limits(own: &PeerLimits, peer: &PeerLimits) -> PeerLimits {
    PeerLimits {
        max_frame: own
            .max_frame
            .min(peer.max_frame)
            .min(MAX_FRAME_BYTES as u32),
        batch_entries: own
            .batch_entries
            .min(peer.batch_entries)
            .min(MAX_FRAME_ENTRIES as u32),
        batch_bytes: own.batch_bytes.min(peer.batch_bytes).min(MAX_BATCH_BYTES),
        credit: own.credit.min(peer.credit).min(MAX_CREDIT),
    }
}

/// What a running link reports upward.
#[derive(Debug)]
pub enum LinkEvent {
    /// A frame the link does not handle itself. `wire_len` is the frame
    /// body's byte length, the figure the receiver returns as `Credit` for an
    /// `entries` frame.
    Frame {
        link_id: u64,
        frame: PeerFrame,
        wire_len: usize,
    },
    /// Room in the data queue (or credit) freed up: try sending again.
    CreditFreed { link_id: u64 },
    /// The link is over; the last event of a link.
    Ended {
        link_id: u64,
        why: LinkError,
        up_for: Duration,
    },
}

/// Why a [`LinkHandle`] call did not queue anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendError {
    /// The queue is full; try again after a [`LinkEvent::CreditFreed`].
    Full,
    /// The link has ended.
    LinkGone,
    /// An `entries` or `branch_entries` body that is empty or over the negotiated `max_frame`.
    BadSize,
}

enum Control {
    Frame(PeerFrame),
    Close(CloseReason),
}

struct Shared {
    credit: u64,
    outstanding: AtomicU64,
    credit_freed: Notify,
    /// Credit granted to the peer and not yet written. The writer drains it
    /// into `credit` frames, so a grant never depends on queue room.
    pending_credit: AtomicU64,
    credit_due: Notify,
    unknown_frames: AtomicU64,
    /// Frames decoded from the peer, of any kind (the coordinator's clone
    /// probe reads it: any growth means the peer is alive).
    inbound_frames: AtomicU64,
}

impl Shared {
    fn release(&self, bytes: u64) {
        let _ = self
            .outstanding
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |out| {
                Some(out.saturating_sub(bytes))
            });
        self.credit_freed.notify_one();
    }
}

/// The sending side of a running link. Calls never block and return
/// [`SendError::LinkGone`] once the link has ended. Dropping the handle ends
/// the link.
///
/// Owns: the two bounded queues into the writer task, control first, then
/// byte-credited `entries`, and the pending-credit counter the writer drains
/// ahead of control frames. Must not: route or interpret frames; that is the
/// coordinator's. See the module documentation for the full responsibility.
pub struct LinkHandle {
    link_id: u64,
    peer: PeerInfo,
    control: mpsc::Sender<Control>,
    data: mpsc::Sender<Vec<u8>>,
    shared: Arc<Shared>,
}

impl LinkHandle {
    pub fn link_id(&self) -> u64 {
        self.link_id
    }

    pub fn peer(&self) -> &PeerInfo {
        &self.peer
    }

    /// Queue a control frame (anything but `entries` and `branch_entries`;
    /// use [`Self::try_send_entries`] for those). `branch_want` and
    /// `branch_none` are control frames. Credit goes through [`Self::grant`].
    pub fn send(&self, frame: PeerFrame) -> Result<(), SendError> {
        map_try(self.control.try_send(Control::Frame(frame)))
    }

    /// Queue one pre-encoded `entries` or `branch_entries` frame body (both
    /// share the credited data queue and the credit counting). It is written
    /// when the credit window allows (`outstanding + len <= credit`, or nothing is
    /// outstanding).
    pub fn try_send_entries(&self, body: Vec<u8>) -> Result<(), SendError> {
        if body.is_empty() || body.len() > self.peer.limits.max_frame as usize {
            return Err(SendError::BadSize);
        }
        map_try(self.data.try_send(body))
    }

    /// Return `bytes` of credit to the peer. Never dropped: grants add to a
    /// pending counter the writer sends as `credit` frames ahead of queued
    /// control frames; `Full` is never returned.
    pub fn grant(&self, bytes: u32) -> Result<(), SendError> {
        // The writer owns the receiver: a closed sender means it is over.
        if self.control.is_closed() {
            return Err(SendError::LinkGone);
        }
        if bytes == 0 {
            return Ok(());
        }
        self.shared
            .pending_credit
            .fetch_add(u64::from(bytes), Ordering::AcqRel);
        self.shared.credit_due.notify_one();
        Ok(())
    }

    /// Write `close` (within one second) and end the link.
    pub fn close(&self, reason: CloseReason) -> Result<(), SendError> {
        map_try(self.control.try_send(Control::Close(reason)))
    }

    /// Unknown frames the link counted and ignored so far.
    pub fn unknown_frames(&self) -> u64 {
        self.shared.unknown_frames.load(Ordering::Relaxed)
    }

    /// Frames received from the peer so far, `pong` and `ping` included.
    pub fn inbound_frames(&self) -> u64 {
        self.shared.inbound_frames.load(Ordering::Relaxed)
    }
}

fn map_try<T>(result: Result<(), mpsc::error::TrySendError<T>>) -> Result<(), SendError> {
    result.map_err(|e| match e {
        mpsc::error::TrySendError::Full(_) => SendError::Full,
        mpsc::error::TrySendError::Closed(_) => SendError::LinkGone,
    })
}

/// Start the reader and writer for an established link. The link's last event
/// is exactly one [`LinkEvent::Ended`]; `cancel` ends it with `close{shutdown}`.
pub fn spawn<T>(
    established: Established<T>,
    link_id: u64,
    events: mpsc::Sender<LinkEvent>,
    clock: Arc<dyn Clock>,
    cancel: Cancellation,
) -> LinkHandle
where
    T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let Established {
        peer,
        reader,
        writer,
    } = established;
    let (control_tx, control_rx) = mpsc::channel(CONTROL_QUEUE);
    let (data_tx, data_rx) = mpsc::channel(DATA_QUEUE);
    let shared = Arc::new(Shared {
        credit: u64::from(peer.limits.credit),
        outstanding: AtomicU64::new(0),
        credit_freed: Notify::new(),
        pending_credit: AtomicU64::new(0),
        credit_due: Notify::new(),
        unknown_frames: AtomicU64::new(0),
        inbound_frames: AtomicU64::new(0),
    });
    let max_frame = peer.limits.max_frame as usize;
    let handle = LinkHandle {
        link_id,
        peer,
        control: control_tx.clone(),
        data: data_tx,
        shared: shared.clone(),
    };

    tokio::spawn(async move {
        let started = clock.monotonic_now();
        let reader_fut = run_reader(
            reader,
            max_frame,
            link_id,
            control_tx.clone(),
            events.clone(),
            shared.clone(),
        );
        let writer_fut = run_writer(writer, control_rx, data_rx, link_id, events.clone(), shared);
        tokio::pin!(reader_fut, writer_fut);
        let why = tokio::select! {
            why = &mut reader_fut => {
                if matches!(why, LinkError::Refused(_) | LinkError::Decode(_)) {
                    // A close was queued; let the writer put it on the wire.
                    let _ = timeout(CLOSE_WRITE_TIMEOUT * 2, &mut writer_fut).await;
                }
                why
            }
            why = &mut writer_fut => why,
            () = cancel.cancelled() => {
                let _ = control_tx.try_send(Control::Close(CloseReason::Shutdown));
                let _ = timeout(CLOSE_WRITE_TIMEOUT * 2, &mut writer_fut).await;
                LinkError::Shutdown
            }
        };
        let up_for = Duration::from_millis(clock.monotonic_now().0.saturating_sub(started.0));
        let _ = events
            .send(LinkEvent::Ended {
                link_id,
                why,
                up_for,
            })
            .await;
    });
    handle
}

async fn read_next<R: AsyncRead + Unpin>(mut reader: R, limit: usize) -> (R, io::Result<Vec<u8>>) {
    let result = transport::read_frame_with_limit(&mut reader, limit).await;
    (reader, result)
}

async fn run_reader<R: AsyncRead + Unpin>(
    reader: R,
    max_frame: usize,
    link_id: u64,
    control: mpsc::Sender<Control>,
    events: mpsc::Sender<LinkEvent>,
    shared: Arc<Shared>,
) -> LinkError {
    let mut last_inbound = Instant::now();
    let mut last_ping: Option<Instant> = None;
    let mut ping_n = 0u64;
    // One read future lives across timer wakeups: a half-read frame is never
    // dropped.
    let mut pending = Box::pin(read_next(reader, max_frame));
    loop {
        let ping_at = last_inbound.max(last_ping.unwrap_or(last_inbound)) + PING_AFTER;
        let dead_at = last_inbound + DEAD_AFTER;
        tokio::select! {
            (reader, result) = &mut pending => {
                let body = match result {
                    Ok(body) => body,
                    Err(e) => return e.into(),
                };
                last_inbound = Instant::now();
                shared.inbound_frames.fetch_add(1, Ordering::Relaxed);
                pending = Box::pin(read_next(reader, max_frame));
                let frame = match PeerFrame::decode(&body) {
                    Ok(frame) => frame,
                    Err(e) => {
                        let _ = control.try_send(Control::Close(CloseReason::ProtocolError));
                        return LinkError::Decode(e.to_string());
                    }
                };
                match frame {
                    PeerFrame::Hello(_) | PeerFrame::Auth { .. } | PeerFrame::Accept => {
                        let _ = control.try_send(Control::Close(CloseReason::ProtocolError));
                        return LinkError::Refused(CloseReason::ProtocolError);
                    }
                    PeerFrame::Close { reason, .. } => return LinkError::PeerClosed(reason),
                    PeerFrame::Ping { n } => {
                        let _ = control.try_send(Control::Frame(PeerFrame::Pong { n }));
                    }
                    PeerFrame::Pong { .. } => {}
                    PeerFrame::Credit { bytes } => shared.release(u64::from(bytes)),
                    PeerFrame::Unknown => {
                        shared.unknown_frames.fetch_add(1, Ordering::Relaxed);
                    }
                    frame => {
                        let event = LinkEvent::Frame { link_id, frame, wire_len: body.len() };
                        if events.send(event).await.is_err() {
                            return LinkError::Shutdown;
                        }
                    }
                }
            }
            () = sleep_until(ping_at) => {
                ping_n += 1;
                last_ping = Some(Instant::now());
                let _ = control.try_send(Control::Frame(PeerFrame::Ping { n: ping_n }));
            }
            () = sleep_until(dead_at) => return LinkError::Dead,
        }
    }
}

async fn run_writer<W: AsyncWrite + Unpin>(
    mut writer: W,
    mut control: mpsc::Receiver<Control>,
    mut data: mpsc::Receiver<Vec<u8>>,
    link_id: u64,
    events: mpsc::Sender<LinkEvent>,
    shared: Arc<Shared>,
) -> LinkError {
    // The head of the data queue, popped but waiting for credit.
    let mut held: Option<Vec<u8>> = None;
    let mut freed_pending = false;
    loop {
        // Granted credit goes out before any queued control frame.
        if let Some(end) = write_due_credit(&mut writer, &shared).await {
            return end;
        }
        // Control frames go before data.
        while let Ok(item) = control.try_recv() {
            if let Some(end) = write_due_credit(&mut writer, &shared).await {
                return end;
            }
            if let Some(end) = write_control(&mut writer, item).await {
                return end;
            }
        }
        if let Some(body) = &held {
            let len = body.len() as u64;
            let outstanding = shared.outstanding.load(Ordering::Acquire);
            if outstanding == 0 || outstanding + len <= shared.credit {
                shared.outstanding.fetch_add(len, Ordering::AcqRel);
                if let Err(e) = write_raw(&mut writer, body).await {
                    return e.into();
                }
                held = None;
                continue;
            }
        }
        tokio::select! {
            biased;
            item = control.recv() => match item {
                Some(item) => {
                    if let Some(end) = write_due_credit(&mut writer, &shared).await {
                        return end;
                    }
                    if let Some(end) = write_control(&mut writer, item).await {
                        return end;
                    }
                }
                None => return LinkError::Shutdown,
            },
            () = shared.credit_due.notified() => {}
            permit = events.reserve(), if freed_pending => {
                if let Ok(permit) = permit {
                    permit.send(LinkEvent::CreditFreed { link_id });
                }
                freed_pending = false;
            }
            body = data.recv(), if held.is_none() => match body {
                Some(body) => {
                    held = Some(body);
                    freed_pending = true;
                }
                None => {
                    // The handle was dropped.
                    return write_control(&mut writer, Control::Close(CloseReason::Shutdown))
                        .await
                        .unwrap_or(LinkError::Shutdown);
                }
            },
            () = shared.credit_freed.notified(), if held.is_some() => {}
        }
    }
}

/// Write the pending grants as `credit` frames. `Some` ends the writer.
async fn write_due_credit<W: AsyncWrite + Unpin>(
    writer: &mut W,
    shared: &Shared,
) -> Option<LinkError> {
    let mut due = shared.pending_credit.swap(0, Ordering::AcqRel);
    while due > 0 {
        let bytes = u32::try_from(due).unwrap_or(u32::MAX);
        if let Err(e) = write_frame_flushed(writer, &PeerFrame::Credit { bytes }).await {
            return Some(e.into());
        }
        due -= u64::from(bytes);
    }
    None
}

/// Write one control item. `Some` ends the writer: a write failure, or a
/// close that has been written.
async fn write_control<W: AsyncWrite + Unpin>(writer: &mut W, item: Control) -> Option<LinkError> {
    match item {
        Control::Frame(frame) => write_frame_flushed(writer, &frame)
            .await
            .err()
            .map(LinkError::from),
        Control::Close(reason) => {
            let close = PeerFrame::Close {
                reason,
                detail: None,
            };
            let written = timeout(CLOSE_WRITE_TIMEOUT, write_frame_flushed(writer, &close)).await;
            Some(match written {
                Ok(Err(e)) => e.into(),
                _ if reason == CloseReason::Shutdown => LinkError::Shutdown,
                _ => LinkError::Refused(reason),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::time::{MonoInstant, UtcMillis};
    use tokio::io::{DuplexStream, duplex};

    const BIG: usize = 4 << 20;

    struct TokioClock(Instant);
    impl Clock for TokioClock {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(0)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(self.0.elapsed().as_millis() as u64)
        }
    }

    fn clock() -> Arc<dyn Clock> {
        Arc::new(TokioClock(Instant::now()))
    }

    fn identity(n: u8) -> Arc<NodeIdentity> {
        Arc::new(NodeIdentity::from_seed([n; 32]))
    }

    fn local(n: u8, boot: u128) -> LocalHello {
        LocalHello::new(identity(n), Uuid::from_u128(boot))
    }

    fn deadline() -> Instant {
        Instant::now() + HANDSHAKE_TIMEOUT
    }

    async fn recv(io: &mut DuplexStream) -> PeerFrame {
        let body = transport::read_frame_with_limit(io, MAX_FRAME_BYTES)
            .await
            .unwrap();
        PeerFrame::decode(&body).unwrap()
    }

    async fn send(io: &mut DuplexStream, frame: &PeerFrame) {
        write_raw(io, &frame.encode().unwrap()).await.unwrap();
    }

    /// Both sides real. The dialed side gets the dialer's sniffed hello as
    /// its first frame, as the daemon transport does.
    async fn pair(
        dialer: &LocalHello,
        dialed: &LocalHello,
    ) -> (
        Result<Established<DuplexStream>, LinkError>,
        Result<Established<DuplexStream>, LinkError>,
    ) {
        let (a, mut b) = duplex(BIG);
        let dialed_side = async {
            let first = transport::read_frame_with_limit(&mut b, MAX_FRAME_BYTES)
                .await
                .unwrap();
            handshake(b, dialed, Role::Dialed, Some(first), deadline()).await
        };
        tokio::join!(
            handshake(a, dialer, Role::Dialer, None, deadline()),
            dialed_side
        )
    }

    fn manual_hello(id: &NodeIdentity, boot: u128, role: Role) -> (PeerHello, Vec<u8>) {
        let hello = PeerHello {
            envelope: 1,
            node: id.uuid(),
            tag: id.tag(),
            pubkey: hex_encode(&id.public_key()),
            boot: Uuid::from_u128(boot),
            caps: vec![BASE_CAP.into()],
            limits: LocalHello::new(identity(1), Uuid::nil()).limits,
            nonce: hex_encode(&[0x42; 32]),
            role,
        };
        let bytes = PeerFrame::Hello(hello.clone()).encode().unwrap();
        (hello, bytes)
    }

    /// A manual dialed peer with identity `id`: reads the dialer's hello,
    /// answers with `reply` (built from it) and returns the dialer's raw hello.
    async fn manual_answer(io: &mut DuplexStream, reply: &[u8]) -> Vec<u8> {
        let theirs = transport::read_frame_with_limit(io, MAX_FRAME_BYTES)
            .await
            .unwrap();
        write_raw(io, reply).await.unwrap();
        theirs
    }

    fn expect_close(frame: PeerFrame, want: CloseReason) {
        match frame {
            PeerFrame::Close { reason, .. } => assert_eq!(reason, want),
            other => panic!("expected close {want:?}, got {other:?}"),
        }
    }

    // ---- preamble -------------------------------------------------------

    #[tokio::test(start_paused = true)]
    async fn preamble_after_0_and_1000_noise_bytes() {
        for noise in [0usize, 1000] {
            let (mut a, mut b) = duplex(BIG);
            let mut data = vec![b'x'; noise];
            data.extend_from_slice(PREAMBLE);
            data.push(b'!');
            b.write_all(&data).await.unwrap();
            let skipped = read_preamble(&mut a, Instant::now() + Duration::from_secs(30))
                .await
                .unwrap();
            assert_eq!(skipped, noise);
            // Nothing past the preamble line was consumed.
            assert_eq!(a.read_u8().await.unwrap(), b'!');
        }
    }

    #[tokio::test(start_paused = true)]
    async fn preamble_noise_over_64k() {
        let (mut a, mut b) = duplex(BIG);
        let mut data = vec![b'x'; PREAMBLE_NOISE_WINDOW + 1];
        data.extend_from_slice(PREAMBLE);
        b.write_all(&data).await.unwrap();
        let err = read_preamble(&mut a, Instant::now() + Duration::from_secs(30))
            .await
            .unwrap_err();
        assert!(matches!(err, LinkError::PreambleNoise), "{err:?}");
        // Exactly the window of noise is still accepted.
        let (mut a, mut b) = duplex(BIG);
        let mut data = vec![b'x'; PREAMBLE_NOISE_WINDOW];
        data.extend_from_slice(PREAMBLE);
        b.write_all(&data).await.unwrap();
        let skipped = read_preamble(&mut a, Instant::now() + Duration::from_secs(30))
            .await
            .unwrap();
        assert_eq!(skipped, PREAMBLE_NOISE_WINDOW);
    }

    #[tokio::test(start_paused = true)]
    async fn preamble_timeout_30s() {
        let (mut a, _b) = duplex(BIG);
        let start = Instant::now();
        let err = read_preamble(&mut a, start + super::super::frame::PREAMBLE_TIMEOUT)
            .await
            .unwrap_err();
        assert!(matches!(err, LinkError::PreambleTimeout), "{err:?}");
        assert_eq!(start.elapsed(), Duration::from_secs(30));
    }

    #[tokio::test(start_paused = true)]
    async fn preamble_eof() {
        let (mut a, mut b) = duplex(BIG);
        b.write_all(b"HTPEER").await.unwrap();
        drop(b);
        let err = read_preamble(&mut a, Instant::now() + Duration::from_secs(30))
            .await
            .unwrap_err();
        assert!(matches!(err, LinkError::PreambleEof), "{err:?}");
    }

    // ---- handshake ------------------------------------------------------

    #[tokio::test(start_paused = true)]
    async fn handshake_both_roles_caps_intersection_min_limits() {
        let mut dialer = local(1, 10);
        dialer.caps = vec![BASE_CAP.into(), "a.v1".into(), "b.v1".into()];
        dialer.limits = PeerLimits {
            max_frame: 1 << 20,
            batch_entries: 512,
            batch_bytes: 512 * 1024,
            credit: 2 << 20,
        };
        let mut dialed = local(2, 20);
        dialed.caps = vec![BASE_CAP.into(), "b.v1".into(), "c.v1".into()];
        dialed.limits = PeerLimits {
            max_frame: 512 * 1024,
            batch_entries: 128,
            batch_bytes: 1 << 20,
            credit: 4 << 20,
        };
        // The dialed side also knows the dialer's real key: it matches.
        let dialer_key = dialer.identity.public_key();
        let dialer_node = dialer.identity.uuid();
        dialed.known_pubkey = Arc::new(move |n| (n == dialer_node).then_some(dialer_key));

        let (d, e) = pair(&dialer, &dialed).await;
        let (d, e) = (d.unwrap(), e.unwrap());
        let want_limits = PeerLimits {
            max_frame: 512 * 1024,
            batch_entries: 128,
            batch_bytes: 256 * 1024,
            credit: 1 << 20,
        };
        for (est, me, other, other_role, other_boot) in [
            (&d, &dialer, &dialed, Role::Dialed, 20u128),
            (&e, &dialed, &dialer, Role::Dialer, 10u128),
        ] {
            let _ = me;
            assert_eq!(est.peer.node, other.identity.uuid());
            assert_eq!(est.peer.tag, other.identity.tag());
            assert_eq!(est.peer.pubkey, other.identity.public_key());
            assert_eq!(est.peer.boot, Uuid::from_u128(other_boot));
            assert_eq!(est.peer.role, other_role);
            assert_eq!(est.peer.caps, [BASE_CAP.to_owned(), "b.v1".to_owned()]);
            assert_eq!(est.peer.limits, want_limits);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn refuse_envelope_unsupported() {
        let dialer = local(1, 10);
        let (a, mut b) = duplex(BIG);
        let other = NodeIdentity::from_seed([2; 32]);
        let (mut hello, _) = manual_hello(&other, 20, Role::Dialed);
        hello.envelope = 2;
        let reply = PeerFrame::Hello(hello).encode().unwrap();
        let (res, ()) = tokio::join!(
            handshake(a, &dialer, Role::Dialer, None, deadline()),
            async {
                manual_answer(&mut b, &reply).await;
                expect_close(recv(&mut b).await, CloseReason::EnvelopeUnsupported);
            }
        );
        let err = res.err().unwrap();
        assert!(matches!(
            err,
            LinkError::Refused(CloseReason::EnvelopeUnsupported)
        ));
        assert_eq!(err.class(), CloseClass::StopDialing);
    }

    #[tokio::test(start_paused = true)]
    async fn hello_with_tiny_limits_is_refused() {
        let floor = MIN_PEER_MAX_FRAME;
        let tiny: [fn(&mut PeerLimits); 4] = [
            |l| l.max_frame = 100,
            |l| l.credit = 100,
            |l| l.batch_entries = 0,
            |l| l.batch_bytes = 0,
        ];
        for shrink in tiny {
            let dialer = local(1, 10);
            let (a, mut b) = duplex(BIG);
            let other = NodeIdentity::from_seed([2; 32]);
            let (mut hello, _) = manual_hello(&other, 20, Role::Dialed);
            shrink(&mut hello.limits);
            let reply = PeerFrame::Hello(hello).encode().unwrap();
            let (res, ()) = tokio::join!(
                handshake(a, &dialer, Role::Dialer, None, deadline()),
                async {
                    manual_answer(&mut b, &reply).await;
                    expect_close(recv(&mut b).await, CloseReason::ProtocolError);
                }
            );
            assert!(matches!(
                res.err().unwrap(),
                LinkError::Refused(CloseReason::ProtocolError)
            ));
        }
        // Exactly the floor is accepted by the check.
        let me = local(1, 10);
        let other = NodeIdentity::from_seed([2; 32]);
        let (mut hello, _) = manual_hello(&other, 20, Role::Dialed);
        hello.limits.max_frame = floor;
        hello.limits.credit = floor;
        hello.limits.batch_entries = 1;
        hello.limits.batch_bytes = 1;
        let bytes = PeerFrame::Hello(hello).encode().unwrap();
        assert!(check_peer_hello(&me, &bytes).is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn refuse_auth_failed_bad_sig() {
        let dialer = local(1, 10);
        let (a, mut b) = duplex(BIG);
        let other = NodeIdentity::from_seed([2; 32]);
        let (_, reply) = manual_hello(&other, 20, Role::Dialed);
        let (res, ()) = tokio::join!(
            handshake(a, &dialer, Role::Dialer, None, deadline()),
            async {
                manual_answer(&mut b, &reply).await;
                assert!(matches!(recv(&mut b).await, PeerFrame::Auth { .. }));
                // A well-formed signature over the wrong data.
                let sig = hex_encode(&other.sign(&[7; 32]));
                send(&mut b, &PeerFrame::Auth { sig }).await;
                expect_close(recv(&mut b).await, CloseReason::AuthFailed);
            }
        );
        assert!(matches!(
            res.err().unwrap(),
            LinkError::Refused(CloseReason::AuthFailed)
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn refuse_auth_failed_uuid_pubkey_mismatch_no_auth_sent() {
        let dialer = local(1, 10);
        let (a, mut b) = duplex(BIG);
        let other = NodeIdentity::from_seed([2; 32]);
        let (mut hello, _) = manual_hello(&other, 20, Role::Dialed);
        // Claim a node id that this key does not hash to.
        hello.node = NodeUuid::from_u128(0xdead);
        let reply = PeerFrame::Hello(hello).encode().unwrap();
        let (res, ()) = tokio::join!(
            handshake(a, &dialer, Role::Dialer, None, deadline()),
            async {
                manual_answer(&mut b, &reply).await;
                // The very next frame is the close: no Auth was sent.
                expect_close(recv(&mut b).await, CloseReason::AuthFailed);
            }
        );
        assert!(matches!(
            res.err().unwrap(),
            LinkError::Refused(CloseReason::AuthFailed)
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn refuse_auth_failed_known_pubkey_differs() {
        let mut dialer = local(1, 10);
        let (a, mut b) = duplex(BIG);
        let other = NodeIdentity::from_seed([2; 32]);
        let other_node = other.uuid();
        // The genesis key on record for that node is a different key.
        dialer.known_pubkey = Arc::new(move |n| (n == other_node).then_some([9; 32]));
        let (_, reply) = manual_hello(&other, 20, Role::Dialed);
        let (res, ()) = tokio::join!(
            handshake(a, &dialer, Role::Dialer, None, deadline()),
            async {
                manual_answer(&mut b, &reply).await;
                expect_close(recv(&mut b).await, CloseReason::AuthFailed);
            }
        );
        assert!(matches!(
            res.err().unwrap(),
            LinkError::Refused(CloseReason::AuthFailed)
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn unknown_node_proceeds() {
        let mut dialer = local(1, 10);
        let dialed = local(2, 20);
        let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = asked.clone();
        dialer.known_pubkey = Arc::new(move |n| {
            log.lock().unwrap().push(n);
            None
        });
        let (d, e) = pair(&dialer, &dialed).await;
        d.unwrap();
        e.unwrap();
        assert_eq!(*asked.lock().unwrap(), [dialed.identity.uuid()]);
    }

    #[tokio::test(start_paused = true)]
    async fn refuse_self_link_same_boot() {
        let me = local(1, 10);
        let (d, e) = pair(&me, &me).await;
        assert!(matches!(
            e.err().unwrap(),
            LinkError::Refused(CloseReason::SelfLink)
        ));
        let err = d.err().unwrap();
        assert!(matches!(err, LinkError::PeerClosed(CloseReason::SelfLink)));
        assert_eq!(err.class(), CloseClass::StopDialing);
    }

    #[tokio::test(start_paused = true)]
    async fn refuse_node_collision_other_boot() {
        let dialer = local(1, 10);
        let clone = local(1, 11);
        let (d, e) = pair(&dialer, &clone).await;
        assert!(matches!(
            e.err().unwrap(),
            LinkError::Refused(CloseReason::NodeCollision)
        ));
        let err = d.err().unwrap();
        assert!(matches!(
            err,
            LinkError::PeerClosed(CloseReason::NodeCollision)
        ));
        assert_eq!(err.class(), CloseClass::StopDialing);
    }

    #[tokio::test(start_paused = true)]
    async fn refuse_busy() {
        let dialer = local(1, 10);
        let (a, mut b) = duplex(BIG);
        let (res, ()) = tokio::join!(
            handshake(a, &dialer, Role::Dialer, None, deadline()),
            async {
                transport::read_frame_with_limit(&mut b, MAX_FRAME_BYTES)
                    .await
                    .unwrap();
                send(
                    &mut b,
                    &PeerFrame::Close {
                        reason: CloseReason::Busy,
                        detail: None,
                    },
                )
                .await;
            }
        );
        let err = res.err().unwrap();
        assert!(matches!(err, LinkError::PeerClosed(CloseReason::Busy)));
        assert_eq!(err.class(), CloseClass::Retry);
    }

    #[tokio::test(start_paused = true)]
    async fn replayed_auth_fails() {
        let x = NodeIdentity::from_seed([2; 32]);
        let (_, x_hello) = manual_hello(&x, 20, Role::Dialed);

        // Link 1: a real dialer against a manual X that completes honestly.
        let dialer = local(1, 10);
        let (a, mut b) = duplex(BIG);
        let (res, x_auth) = tokio::join!(
            handshake(a, &dialer, Role::Dialer, None, deadline()),
            async {
                let theirs = manual_answer(&mut b, &x_hello).await;
                let PeerFrame::Auth { .. } = recv(&mut b).await else {
                    panic!("expected auth");
                };
                let dialer_nonce =
                    hex_decode::<32>(&PeerFrame::decode_hello(&theirs).unwrap().nonce).unwrap();
                let sig = hex_encode(&x.sign(&auth_digest(&x_hello, &theirs, &dialer_nonce)));
                send(&mut b, &PeerFrame::Auth { sig: sig.clone() }).await;
                // The genuine Auth verifies on its own link: the dialer
                // answers with Accept.
                assert!(matches!(recv(&mut b).await, PeerFrame::Accept));
                send(&mut b, &PeerFrame::Accept).await;
                sig
            }
        );
        res.unwrap();

        // Link 2: the same hello and the same Auth, replayed to a new dialer.
        let (a, mut b) = duplex(BIG);
        let (res, ()) = tokio::join!(
            handshake(a, &dialer, Role::Dialer, None, deadline()),
            async {
                manual_answer(&mut b, &x_hello).await;
                assert!(matches!(recv(&mut b).await, PeerFrame::Auth { .. }));
                send(&mut b, &PeerFrame::Auth { sig: x_auth }).await;
                expect_close(recv(&mut b).await, CloseReason::AuthFailed);
            }
        );
        assert!(matches!(
            res.err().unwrap(),
            LinkError::Refused(CloseReason::AuthFailed)
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn handshake_timeout_15s() {
        let dialer = local(1, 10);
        let (a, _b) = duplex(BIG);
        let start = Instant::now();
        let err = handshake(a, &dialer, Role::Dialer, None, start + HANDSHAKE_TIMEOUT)
            .await
            .err()
            .unwrap();
        assert!(matches!(err, LinkError::HandshakeTimeout), "{err:?}");
        assert_eq!(start.elapsed(), HANDSHAKE_TIMEOUT);
        assert_eq!(err.class(), CloseClass::Retry);
    }

    // ---- running link ---------------------------------------------------

    /// A real link on side A, the raw established halves on side B.
    async fn running() -> (
        LinkHandle,
        mpsc::Receiver<LinkEvent>,
        Established<DuplexStream>,
        Cancellation,
    ) {
        let (d, e) = pair(&local(1, 10), &local(2, 20)).await;
        let (events_tx, events_rx) = mpsc::channel(64);
        let cancel = Cancellation::default();
        let handle = spawn(d.unwrap(), 7, events_tx, clock(), cancel.clone());
        (handle, events_rx, e.unwrap(), cancel)
    }

    async fn read_body(est: &mut Established<DuplexStream>) -> Vec<u8> {
        transport::read_frame_with_limit(&mut est.reader, MAX_FRAME_BYTES)
            .await
            .unwrap()
    }

    async fn read_next_frame(est: &mut Established<DuplexStream>) -> PeerFrame {
        PeerFrame::decode(&read_body(est).await).unwrap()
    }

    async fn nothing_within(est: &mut Established<DuplexStream>, secs: u64) -> bool {
        timeout(Duration::from_secs(secs), read_body(est))
            .await
            .is_err()
    }

    #[tokio::test(start_paused = true)]
    async fn credit_two_400k_go_third_waits() {
        let (handle, _events, mut peer, _cancel) = running().await;
        for _ in 0..3 {
            handle.try_send_entries(vec![b'a'; 400 * 1024]).unwrap();
        }
        assert_eq!(read_body(&mut peer).await.len(), 400 * 1024);
        assert_eq!(read_body(&mut peer).await.len(), 400 * 1024);
        assert!(
            nothing_within(&mut peer, 5).await,
            "third frame needs credit"
        );
        write_peer_frame(&mut peer.writer, &PeerFrame::Credit { bytes: 400 * 1024 })
            .await
            .unwrap();
        assert_eq!(read_body(&mut peer).await.len(), 400 * 1024);
    }

    #[tokio::test(start_paused = true)]
    async fn single_900k_goes_when_nothing_outstanding() {
        let (handle, _events, mut peer, _cancel) = running().await;
        handle.try_send_entries(vec![b'a'; 900 * 1024]).unwrap();
        assert_eq!(read_body(&mut peer).await.len(), 900 * 1024);
        // With 900 KiB outstanding a second one has to wait for credit.
        handle.try_send_entries(vec![b'b'; 900 * 1024]).unwrap();
        assert!(nothing_within(&mut peer, 5).await);
        write_peer_frame(&mut peer.writer, &PeerFrame::Credit { bytes: 900 * 1024 })
            .await
            .unwrap();
        assert_eq!(read_body(&mut peer).await.len(), 900 * 1024);
        // Oversized and empty bodies never queue.
        assert_eq!(
            handle.try_send_entries(vec![b'c'; MAX_FRAME_BYTES + 1]),
            Err(SendError::BadSize)
        );
        assert_eq!(handle.try_send_entries(vec![]), Err(SendError::BadSize));
    }

    #[tokio::test(start_paused = true)]
    async fn ping_answered_while_data_blocked() {
        let (handle, _events, mut peer, _cancel) = running().await;
        for _ in 0..3 {
            handle.try_send_entries(vec![b'a'; 400 * 1024]).unwrap();
        }
        read_body(&mut peer).await;
        read_body(&mut peer).await;
        assert!(nothing_within(&mut peer, 1).await);
        write_peer_frame(&mut peer.writer, &PeerFrame::Ping { n: 5 })
            .await
            .unwrap();
        assert!(matches!(
            read_next_frame(&mut peer).await,
            PeerFrame::Pong { n: 5 }
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn ping_after_30s_dead_at_90s() {
        let (handle, mut events, mut peer, _cancel) = running().await;
        let start = Instant::now();
        let frame = timeout(Duration::from_secs(31), read_next_frame(&mut peer))
            .await
            .unwrap();
        assert!(matches!(frame, PeerFrame::Ping { n: 1 }), "{frame:?}");
        assert_eq!(start.elapsed(), PING_AFTER);
        let LinkEvent::Ended {
            why,
            up_for,
            link_id,
        } = events.recv().await.unwrap()
        else {
            panic!("expected Ended");
        };
        assert_eq!(link_id, 7);
        assert!(matches!(why, LinkError::Dead), "{why:?}");
        assert_eq!(why.class(), CloseClass::Retry);
        assert_eq!(up_for, DEAD_AFTER);
        assert_eq!(handle.send(PeerFrame::Accept), Err(SendError::LinkGone));
        assert_eq!(handle.try_send_entries(vec![1]), Err(SendError::LinkGone));
        assert_eq!(handle.grant(1), Err(SendError::LinkGone));
    }

    #[tokio::test(start_paused = true)]
    async fn handshake_frame_after_establishment_protocol_error() {
        let (handle, mut events, mut peer, _cancel) = running().await;
        write_peer_frame(&mut peer.writer, &PeerFrame::Accept)
            .await
            .unwrap();
        expect_close(read_next_frame(&mut peer).await, CloseReason::ProtocolError);
        let LinkEvent::Ended { why, .. } = events.recv().await.unwrap() else {
            panic!("expected Ended");
        };
        assert!(matches!(
            why,
            LinkError::Refused(CloseReason::ProtocolError)
        ));
        assert_eq!(
            handle.close(CloseReason::Shutdown),
            Err(SendError::LinkGone)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn forwards_frames_counts_unknown_and_applies_close() {
        let (handle, mut events, mut peer, _cancel) = running().await;
        let have = PeerFrame::Have {
            full: true,
            ask: false,
            logs: vec![],
            forks: vec![],
        };
        let have_len = have.encode().unwrap().len();
        write_peer_frame(&mut peer.writer, &have).await.unwrap();
        write_raw(&mut peer.writer, br#"{"t":"fork_offer","x":1}"#)
            .await
            .unwrap();
        write_peer_frame(
            &mut peer.writer,
            &PeerFrame::Want {
                node: NodeUuid::nil(),
                inc: 1,
                from: 1,
            },
        )
        .await
        .unwrap();
        let LinkEvent::Frame {
            link_id,
            frame,
            wire_len,
        } = events.recv().await.unwrap()
        else {
            panic!("expected Frame");
        };
        assert_eq!((link_id, wire_len), (7, have_len));
        assert!(matches!(frame, PeerFrame::Have { full: true, .. }));
        // The unknown frame is not forwarded; the next event is the want.
        let LinkEvent::Frame { frame, .. } = events.recv().await.unwrap() else {
            panic!("expected Frame");
        };
        assert!(matches!(frame, PeerFrame::Want { .. }));
        assert_eq!(handle.unknown_frames(), 1);

        handle.close(CloseReason::Busy).unwrap();
        expect_close(read_next_frame(&mut peer).await, CloseReason::Busy);
        let LinkEvent::Ended { why, .. } = events.recv().await.unwrap() else {
            panic!("expected Ended");
        };
        assert!(matches!(why, LinkError::Refused(CloseReason::Busy)));
    }

    fn branch_entries_body(payload_bytes: usize) -> Vec<u8> {
        let payload = format!(r#"{{"p":"{}"}}"#, "x".repeat(payload_bytes));
        let envelope = crate::sync::envelope::WireEnvelope {
            node: "00".repeat(16),
            inc: 1,
            counter: 2,
            prev: "00".repeat(32),
            hlc: 5,
            kind: "k".into(),
            ver: 1,
            deps: "0000".into(),
            creates: None,
            needs: vec![],
            payload: serde_json::value::RawValue::from_string(payload).unwrap(),
            hash: "00".repeat(32),
            sig: "00".repeat(64),
        };
        PeerFrame::BranchEntries {
            node: NodeUuid::nil(),
            inc: 1,
            tip_hash: "bb".repeat(32),
            entries: vec![envelope.into()],
            more: false,
        }
        .encode()
        .unwrap()
    }

    #[tokio::test(start_paused = true)]
    async fn branch_entries_consume_and_return_credit_like_entries() {
        let (handle, mut events, mut peer, _cancel) = running().await;
        let body = branch_entries_body(400 * 1024);
        assert!(body.len() > 400 * 1024 && body.len() < 410 * 1024);
        let len = body.len();
        // Sending: two frames fill the 1 MiB window, the third waits.
        for _ in 0..3 {
            handle.try_send_entries(body.clone()).unwrap();
        }
        assert_eq!(read_body(&mut peer).await.len(), len);
        assert_eq!(read_body(&mut peer).await.len(), len);
        assert!(
            nothing_within(&mut peer, 5).await,
            "third branch_entries frame needs credit"
        );
        write_peer_frame(&mut peer.writer, &PeerFrame::Credit { bytes: len as u32 })
            .await
            .unwrap();
        let third = read_body(&mut peer).await;
        assert_eq!(third.len(), len);
        assert!(matches!(
            PeerFrame::decode(&third),
            Ok(PeerFrame::BranchEntries { .. })
        ));

        // Receiving: a branch_entries frame reaches the coordinator with its
        // wire length (the figure returned as credit); branch_want and
        // branch_none are forwarded as ordinary frames.
        write_raw(&mut peer.writer, &body).await.unwrap();
        for frame in [
            PeerFrame::BranchWant(crate::sync::frame::BranchWant {
                node: NodeUuid::nil(),
                inc: 1,
                tip: crate::sync::frame::TipRef {
                    counter: 3,
                    hash: "bb".repeat(32),
                },
                from: 2,
            }),
            PeerFrame::BranchNone {
                node: NodeUuid::nil(),
                inc: 1,
                tip_hash: "bb".repeat(32),
            },
        ] {
            write_peer_frame(&mut peer.writer, &frame).await.unwrap();
        }
        let mut seen = Vec::new();
        while seen.len() < 3 {
            if let Some(LinkEvent::Frame {
                frame, wire_len, ..
            }) = events.recv().await
            {
                seen.push((frame, wire_len));
            }
        }
        assert!(matches!(seen[0].0, PeerFrame::BranchEntries { .. }));
        assert_eq!(seen[0].1, len);
        assert!(matches!(seen[1].0, PeerFrame::BranchWant { .. }));
        assert!(matches!(seen[2].0, PeerFrame::BranchNone { .. }));
    }

    #[tokio::test(start_paused = true)]
    async fn grant_is_never_dropped_on_a_full_control_queue() {
        let (handle, _events, mut peer, _cancel) = running().await;
        for n in 0..CONTROL_QUEUE as u64 {
            handle.send(PeerFrame::Pong { n }).unwrap();
        }
        assert_eq!(
            handle.send(PeerFrame::Pong { n: 999 }),
            Err(SendError::Full)
        );
        assert_eq!(handle.grant(1000), Ok(()));
        assert_eq!(handle.grant(234), Ok(()));
        let (mut pongs, mut credit) = (0, 0u64);
        while pongs < CONTROL_QUEUE || credit < 1234 {
            match read_next_frame(&mut peer).await {
                PeerFrame::Pong { .. } => pongs += 1,
                PeerFrame::Credit { bytes } => credit += u64::from(bytes),
                other => panic!("unexpected {other:?}"),
            }
        }
        assert_eq!(credit, 1234);
        assert!(nothing_within(&mut peer, 5).await, "no extra credit");
    }

    #[tokio::test(start_paused = true)]
    async fn grant_zero_sends_nothing() {
        let (handle, _events, mut peer, _cancel) = running().await;
        assert_eq!(handle.grant(0), Ok(()));
        assert!(nothing_within(&mut peer, 5).await);
    }

    async fn next_frame_event(rx: &mut mpsc::Receiver<LinkEvent>) -> (PeerFrame, usize) {
        loop {
            match rx.recv().await.expect("events open") {
                LinkEvent::Frame {
                    frame, wire_len, ..
                } => return (frame, wire_len),
                LinkEvent::CreditFreed { .. } => {}
                LinkEvent::Ended { why, .. } => panic!("link ended: {why:?}"),
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn peer_writer_resumes_after_a_grant_on_a_full_control_queue() {
        let (d, e) = pair(&local(1, 10), &local(2, 20)).await;
        let (tx_a, mut events_a) = mpsc::channel(64);
        let (tx_b, _events_b) = mpsc::channel(64);
        let (cancel_a, cancel_b) = (Cancellation::default(), Cancellation::default());
        let a = spawn(d.unwrap(), 1, tx_a, clock(), cancel_a.clone());
        let b = spawn(e.unwrap(), 2, tx_b, clock(), cancel_b.clone());
        let body = branch_entries_body(400 * 1024);
        for _ in 0..3 {
            b.try_send_entries(body.clone()).unwrap();
        }
        let mut wire_len = 0;
        for _ in 0..2 {
            let (frame, len) = next_frame_event(&mut events_a).await;
            assert!(matches!(frame, PeerFrame::BranchEntries { .. }));
            wire_len = len;
        }
        assert!(
            timeout(Duration::from_secs(5), next_frame_event(&mut events_a))
                .await
                .is_err(),
            "third frame needs credit"
        );
        for n in 0..CONTROL_QUEUE as u64 {
            a.send(PeerFrame::Pong { n }).unwrap();
        }
        assert_eq!(a.send(PeerFrame::Pong { n: 999 }), Err(SendError::Full));
        assert_eq!(a.grant(wire_len as u32), Ok(()));
        let (third, _) = timeout(Duration::from_secs(10), next_frame_event(&mut events_a))
            .await
            .expect("peer writer resumed without a reconnect");
        assert!(matches!(third, PeerFrame::BranchEntries { .. }));
        cancel_a.cancel();
        cancel_b.cancel();
    }
}
