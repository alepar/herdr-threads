//! The peer wire vocabulary: preamble, hello limits and the closed
//! [`PeerFrame`] / [`CloseReason`] enums, framed as u32 BE length plus JSON by
//! the daemon transport's `read_frame_with_limit` / `write_frame`.
//!
//! Core and harness-neutral. Inert: nothing outside tests builds or reads
//! these types yet; the link, coordinator and transport leaves of ht-5fr.5
//! build on them. Versionless: no number is compared anywhere except the
//! envelope generation in the hello; unknown frames and fields are tolerated.

use std::io;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite};
use uuid::Uuid;

use super::LogKey;
use super::envelope::{
    Envelope, EnvelopeError, OpaqueEntry, OpaqueHeader, OpaqueTooLarge, StoredEntry, WireEnvelope,
    check_opaque_size, hex_decode, hex_decode_vec, hex_encode,
};
use crate::daemon::transport::{self, MAX_FRAME_BYTES};
use crate::node::{NodeTag, NodeUuid};

/// The line the remote pump writes once, before any frame.
pub const PREAMBLE: &[u8; 8] = b"HTPEER1\n";
/// How many bytes of remote noise the dialer skips while looking for the
/// preamble.
pub const PREAMBLE_NOISE_WINDOW: usize = 64 * 1024;
/// How long the dialer waits for the preamble.
pub const PREAMBLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// Simultaneous peer connections a daemon admits.
pub const MAX_PEER_CONNECTIONS: usize = 16;
/// The capability every `sync.v1` frame belongs to.
pub const BASE_CAP: &str = "sync.v1";
/// Longest encoded hello frame, in bytes.
pub const MAX_HELLO_BYTES: usize = 8 * 1024;
/// Most capabilities a hello may list.
pub const MAX_HELLO_CAPS: usize = 32;
/// Longest single capability name, in bytes.
pub const MAX_CAP_BYTES: usize = 64;
/// The capability the fork frames (`branch_*`, `Have.forks`) belong to.
pub const FORK_CAP: &str = "fork.v1";
/// Capability: this node accepts `{"opaque": ...}` items in `entries` and
/// `branch_entries` (entries whose signed layout the receiver may not
/// understand). A sender never sends one to a peer that did not advertise it.
pub const OPAQUE_CAP: &str = "envelope.opaque.v1";
/// Capability: this node sends and reads [`PeerFrame::NodeInfo`].
pub const NODE_INFO_CAP: &str = "node.info.v1";
/// Longest `node_info.version` kept, in bytes; a longer one is ignored.
pub const MAX_NODE_VERSION_BYTES: usize = 32;
/// Capability: this node sends, reads and relays [`PeerFrame::SeatStatus`].
pub const SEAT_STATUS_CAP: &str = "seat.status.v1";
/// Most seats one `seat_status` frame carries.
pub const MAX_SEAT_STATUS_SEATS: usize = 512;
/// Longest seat label (workspace, tab, pane) kept, in bytes.
pub const MAX_SEAT_LABEL_BYTES: usize = 64;
/// Longest harness id kept, in bytes.
pub const MAX_SEAT_HARNESS_BYTES: usize = 16;
/// Capability: this node answers [`PeerFrame::AddrResolve`] with
/// [`PeerFrame::AddrResolved`] (remote addresses, `SEAT@MACHINE`). A node
/// sends a request only to a peer that advertised it.
pub const ADDR_RESOLVE_CAP: &str = "addr.resolve.v1";
/// Capability: this node answers an [`AddrTarget::Check`] (whether one of its
/// seats can take a handoff now). A node sends one only to a peer that
/// advertised it.
pub const ADDR_CHECK_CAP: &str = "addr.check.v1";
/// Longest address text (a location or one label) in an `addr_resolve`, in
/// bytes; a longer one is answered `refused`.
pub const MAX_ADDR_TEXT_BYTES: usize = 512;
/// Most candidates one `addr_resolved` lists; the rest are counted.
pub const MAX_ADDR_CANDIDATES: usize = 32;
/// Longest `refused.detail`, path or pane text in an `addr_resolved`, in
/// bytes.
pub const MAX_ADDR_DETAIL_BYTES: usize = 256;
/// Most fork references in one `have` frame.
pub const MAX_HAVE_FORKS: usize = 64;
/// Longest `close.detail`, in bytes.
pub const MAX_CLOSE_DETAIL_BYTES: usize = 256;
/// Most logs in one `have` frame.
pub const MAX_HAVE_LOGS: usize = 4096;
/// Most ages in one `seen` frame.
pub const MAX_SEEN_AGES: usize = 4096;
/// Most entries in one `entries` frame.
pub const MAX_FRAME_ENTRIES: usize = 256;
/// Least `max_frame` (and `credit`) a peer hello may declare; a smaller one
/// is refused `ProtocolError`, so frame halving always terminates with at
/// most one frame per entry.
pub const MIN_PEER_MAX_FRAME: u32 = 4_096;

/// Which side opened the connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Dialer,
    Dialed,
}

/// What a side can accept, offered in its hello; the effective value is the
/// minimum of both sides.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerLimits {
    pub max_frame: u32,
    pub batch_entries: u32,
    pub batch_bytes: u32,
    pub credit: u32,
}

/// The first frame each side sends. Binary fields are lowercase hex.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerHello {
    /// Envelope generation; 1 today.
    pub envelope: u32,
    pub node: NodeUuid,
    pub tag: NodeTag,
    /// Ed25519 public key, 64 hex characters.
    pub pubkey: String,
    /// Random per daemon process; tells a self link from a clone.
    pub boot: Uuid,
    pub caps: Vec<String>,
    pub limits: PeerLimits,
    /// 32 random bytes, 64 hex characters.
    pub nonce: String,
    pub role: Role,
}

/// One log's head in a `have` frame.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HaveLog {
    pub node: NodeUuid,
    pub inc: u32,
    pub counter: u64,
    /// Head hash, 64 hex characters.
    pub head: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frozen_at: Option<u64>,
}

/// A log position: a counter and the hash of the entry at it (64 hex
/// characters).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TipRef {
    pub counter: u64,
    pub hash: String,
}

/// The body of a `branch_want` frame; also what a fork sink queues to send.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchWant {
    pub node: NodeUuid,
    pub inc: u32,
    /// The conflicting branch's tip the sender wants entries of.
    pub tip: TipRef,
    /// First counter wanted, `1 <= from <= tip.counter`.
    pub from: u64,
}

/// Fork evidence carried in a `have` frame behind [`FORK_CAP`]: the sender
/// holds `held` on this log and knows of a conflicting branch ending at
/// `conflicting`, frozen at counter `frozen_at`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForkRef {
    pub node: NodeUuid,
    pub inc: u32,
    pub held: TipRef,
    pub conflicting: TipRef,
    pub frozen_at: u64,
}

/// How long ago a node was last heard from, in a `seen` frame.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeenAge {
    pub node: NodeUuid,
    pub age_ms: u64,
}

/// Why a side ends the link. Closed on send; a newer peer's reason decodes to
/// [`CloseReason::Unknown`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseReason {
    EnvelopeUnsupported,
    NodeCollision,
    SelfLink,
    AuthFailed,
    Busy,
    Shutdown,
    ProtocolError,
    #[serde(other)]
    Unknown,
}

/// What a dialer does after receiving a close.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseClass {
    /// Only an operator can repair the cause; stop dialing this target.
    StopDialing,
    Retry,
}

impl CloseReason {
    pub fn class(self) -> CloseClass {
        match self {
            Self::EnvelopeUnsupported | Self::SelfLink | Self::AuthFailed | Self::NodeCollision => {
                CloseClass::StopDialing
            }
            Self::Busy | Self::Shutdown | Self::ProtocolError | Self::Unknown => CloseClass::Retry,
        }
    }
}

/// The wire form of an opaque entry; binary fields are lowercase hex.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpaqueWire {
    pub node: String,
    pub inc: u32,
    pub counter: u64,
    pub prev: String,
    pub hash: String,
    pub sig: String,
    pub raw: String,
}

/// One item of `entries` / `branch_entries`.
///
/// Owns: the item alternative on the wire and its hand-written decode.
/// `Known` encodes exactly as a bare [`WireEnvelope`] (byte-identical to
/// 2.1.1); `Opaque` encodes as `{"opaque":{...}}`. Decoded by hand: an object
/// with an `opaque` key is `Opaque`, anything else a [`WireEnvelope`] (serde
/// `untagged` cannot hold the `RawValue` payload). Callers never write serde
/// `untagged` around a `RawValue`.
#[derive(Debug, Clone)]
pub enum WireItem {
    Known(WireEnvelope),
    // cap: OPAQUE_CAP
    Opaque(OpaqueWire),
}

impl Serialize for WireItem {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap as _;
        match self {
            Self::Known(wire) => wire.serialize(serializer),
            Self::Opaque(wire) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("opaque", wire)?;
                map.end()
            }
        }
    }
}

/// Why a [`WireItem`] did not become a [`StoredEntry`]. A size refusal is not
/// evidence of a corrupt source, so it stays apart from the shape errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireItemError {
    Envelope(EnvelopeError),
    TooLarge(OpaqueTooLarge),
}

impl std::fmt::Display for WireItemError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Envelope(e) => e.fmt(f),
            Self::TooLarge(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for WireItemError {}

impl From<EnvelopeError> for WireItemError {
    fn from(e: EnvelopeError) -> Self {
        Self::Envelope(e)
    }
}

impl WireItem {
    /// Decode one item from its JSON text.
    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        #[derive(Deserialize)]
        struct Probe {
            #[serde(default)]
            opaque: Option<OpaqueWire>,
        }
        match serde_json::from_str::<Probe>(text)
            .ok()
            .and_then(|p| p.opaque)
        {
            Some(opaque) => Ok(Self::Opaque(opaque)),
            None => serde_json::from_str::<WireEnvelope>(text).map(Self::Known),
        }
    }

    /// The item's counter, as the sender declares it.
    pub fn counter(&self) -> u64 {
        match self {
            Self::Known(wire) => wire.counter,
            Self::Opaque(wire) => wire.counter,
        }
    }

    /// Decode into the entry the journal port takes. An opaque item is size
    /// checked before its hex is decoded, then shape checked against its
    /// header (the signature is checked by the store's `check_next`).
    pub fn into_stored(self) -> Result<StoredEntry, WireItemError> {
        match self {
            Self::Known(wire) => Ok(StoredEntry::Known(Envelope::try_from(wire)?)),
            Self::Opaque(wire) => {
                let malformed = |field| WireItemError::Envelope(EnvelopeError::Malformed { field });
                check_opaque_size(wire.raw.len() / 2).map_err(WireItemError::TooLarge)?;
                let log = LogKey {
                    node: crate::node::NodeUuid::from_bytes(
                        hex_decode::<16>(&wire.node).ok_or(malformed("node"))?,
                    ),
                    incarnation: wire.inc,
                };
                let header = OpaqueHeader {
                    log,
                    counter: wire.counter,
                    prev_hash: hex_decode(&wire.prev).ok_or(malformed("prev"))?,
                    hash: hex_decode(&wire.hash).ok_or(malformed("hash"))?,
                    signature: hex_decode(&wire.sig).ok_or(malformed("sig"))?,
                };
                let raw = hex_decode_vec(&wire.raw).ok_or(malformed("raw"))?;
                Ok(StoredEntry::Opaque(OpaqueEntry::new(header, raw)?))
            }
        }
    }
}

impl From<WireEnvelope> for WireItem {
    fn from(wire: WireEnvelope) -> Self {
        Self::Known(wire)
    }
}

impl From<&Envelope> for WireItem {
    fn from(envelope: &Envelope) -> Self {
        Self::Known(WireEnvelope::from(envelope))
    }
}

impl From<&OpaqueEntry> for WireItem {
    fn from(entry: &OpaqueEntry) -> Self {
        Self::Opaque(OpaqueWire {
            node: hex_encode(entry.log().node.as_bytes()),
            inc: entry.log().incarnation,
            counter: entry.counter(),
            prev: hex_encode(&entry.prev_hash()),
            hash: hex_encode(&entry.hash()),
            sig: hex_encode(&entry.signature()),
            raw: hex_encode(entry.raw()),
        })
    }
}

impl From<&StoredEntry> for WireItem {
    fn from(entry: &StoredEntry) -> Self {
        match entry {
            StoredEntry::Known(envelope) => Self::from(envelope),
            StoredEntry::Opaque(opaque) => Self::from(opaque),
        }
    }
}

/// What [`PeerFrame::NodeInfo`] carries. Never compared or gated on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeInfo {
    #[serde(default)]
    pub version: String,
}

/// What [`PeerFrame::SeatStatus`] carries: the origin node's advisory status
/// of its own seats (ht-xg88). Display data only: nothing routes, wakes or
/// attributes by it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatStatusFrame {
    /// The node whose seats these are.
    #[serde(default = "nil_node")]
    pub origin: NodeUuid,
    /// The origin daemon's boot.
    #[serde(default)]
    pub boot: Uuid,
    /// Per boot, one more for each changed snapshot.
    #[serde(default)]
    pub rev: u64,
    /// How long ago the origin built it, accumulated by every relay.
    #[serde(default)]
    pub age_ms: u64,
    /// The origin daemon's version (relayed nodes learn it here).
    #[serde(default)]
    pub version: String,
    /// At most [`MAX_SEAT_STATUS_SEATS`].
    #[serde(default)]
    pub seats: Vec<SeatStatusEntry>,
}

fn nil_node() -> NodeUuid {
    NodeUuid::from_bytes([0; 16])
}

/// One seat of a [`SeatStatusFrame`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeatStatusEntry {
    #[serde(default)]
    pub seat: String,
    /// `busy`, `idle` or `offline`; an unknown value renders as `-`.
    #[serde(default)]
    pub presence: String,
    /// The harness id of the seat's binding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    /// How long ago the seat last acted, at build time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity_age_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<String>,
}

/// A request to the peer to resolve an address on its own node (cap
/// [`ADDR_RESOLVE_CAP`]). The peer answers with one [`AddrResolved`] of the
/// same `id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddrResolve {
    /// Correlation id, fresh per request.
    #[serde(default)]
    pub id: Uuid,
    #[serde(default)]
    pub target: AddrTarget,
}

/// What an [`AddrResolve`] asks for. An unknown kind decodes to
/// [`AddrTarget::Unknown`] and is answered `refused`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "k", rename_all = "snake_case")]
pub enum AddrTarget {
    /// A SEAT argument without its `@MACHINE` suffix: an exact seat ID, a
    /// `SPACE/TAB/PANE` name, a nickname or a pane ID.
    Seat {
        #[serde(default)]
        location: String,
    },
    /// A `--space/--tab/--pane` selector without its suffixes.
    Pane {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        space: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane: Option<String>,
    },
    /// Whether this exact seat of the answering node can take a handoff now
    /// (cap [`ADDR_CHECK_CAP`]): resolved, published, nonretired, unheld, its
    /// target observed.
    Check {
        #[serde(default)]
        seat: String,
    },
    #[default]
    #[serde(other)]
    Unknown,
}

/// The answer to an [`AddrResolve`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddrResolved {
    #[serde(default)]
    pub id: Uuid,
    #[serde(default)]
    pub outcome: AddrOutcome,
}

/// How the answering node resolved an address against its own live
/// topology and seat table. An unknown kind decodes to
/// [`AddrOutcome::Unknown`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "k", rename_all = "snake_case")]
pub enum AddrOutcome {
    /// One pane, mapped to this resolved local seat.
    Seat {
        #[serde(default)]
        seat: String,
        #[serde(default)]
        path: String,
        #[serde(default)]
        pane: String,
    },
    /// One pane, which holds no resolved seat.
    NoSeat {
        #[serde(default)]
        path: String,
        #[serde(default)]
        pane: String,
    },
    NoMatch,
    /// At most [`MAX_ADDR_CANDIDATES`]; `omitted` counts the rest.
    Ambiguous {
        #[serde(default)]
        candidates: Vec<AddrCandidate>,
        #[serde(default)]
        omitted: u32,
    },
    /// No attached Herdr host could be read (or one failed and the rest
    /// did not already make the answer ambiguous).
    HostUnavailable,
    /// Not answered, and asking again cannot help (an unsupported or
    /// invalid request); `detail` (at most [`MAX_ADDR_DETAIL_BYTES`]) says
    /// why.
    Refused {
        #[serde(default)]
        detail: String,
    },
    /// The answer to [`AddrTarget::Check`]: the seat can take a handoff.
    Eligible,
    /// The answer to [`AddrTarget::Check`]: it cannot; `reason` is at most
    /// [`MAX_ADDR_DETAIL_BYTES`].
    Ineligible {
        #[serde(default)]
        reason: String,
    },
    /// Not answered now (busy, resolver not ready, a store read failed); a
    /// retry may succeed. `reason` is at most [`MAX_ADDR_DETAIL_BYTES`].
    Unavailable {
        #[serde(default)]
        reason: String,
    },
    #[default]
    #[serde(other)]
    Unknown,
}

/// One candidate of an ambiguous [`AddrOutcome`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddrCandidate {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub pane: String,
    /// The pane's resolved seat, if it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seat: Option<String>,
}

/// One peer wire frame. Every struct tolerates unknown fields and an unknown
/// `t` decodes to [`PeerFrame::Unknown`], so peers add frames and fields
/// compatibly; senders send a non-base frame only to a peer that advertised
/// its capability.
///
/// Decode with [`PeerFrame::decode`] only: serde buffers an internally tagged
/// enum's content, which cannot hold the raw-JSON payloads of
/// [`WireEnvelope`], so `Entries` is skipped by the derived `Deserialize` and
/// decoded from its own struct.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum PeerFrame {
    Hello(PeerHello),
    Auth {
        /// Signature, 128 hex characters.
        sig: String,
    },
    Accept,
    Close {
        reason: CloseReason,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    Have {
        /// `true` replaces the stored peer vector; otherwise a delta.
        full: bool,
        #[serde(default)]
        ask: bool,
        logs: Vec<HaveLog>,
        /// Fork evidence, at most [`MAX_HAVE_FORKS`]; sent only to a peer
        /// that advertised [`FORK_CAP`].
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        forks: Vec<ForkRef>,
    },
    Want {
        node: NodeUuid,
        inc: u32,
        /// First counter wanted, at least 1.
        from: u64,
    },
    Unwant {
        node: NodeUuid,
        inc: u32,
    },
    #[serde(skip_deserializing)]
    Entries {
        node: NodeUuid,
        inc: u32,
        entries: Vec<WireItem>,
    },
    /// Fork cap: ask for the conflicting branch of a frozen log, from
    /// counter `from` (`1 <= from <= tip.counter`, checked by `decode`).
    BranchWant(BranchWant),
    /// Fork cap: a batch of the branch ending at `tip_hash`, 1..=256 entries.
    /// Rides the credited data queue like `Entries`.
    #[serde(skip_deserializing)]
    BranchEntries {
        node: NodeUuid,
        inc: u32,
        tip_hash: String,
        entries: Vec<WireItem>,
        more: bool,
    },
    /// Fork cap: the sender no longer has the branch ending at `tip_hash`.
    BranchNone {
        node: NodeUuid,
        inc: u32,
        tip_hash: String,
    },
    Credit {
        bytes: u32,
    },
    Ping {
        n: u64,
    },
    Pong {
        n: u64,
    },
    Seen {
        ages: Vec<SeenAge>,
    },
    /// The sender's daemon version, for display only; sent once per link
    /// right after it is established.
    // cap: NODE_INFO_CAP
    NodeInfo(NodeInfo),
    /// One origin node's whole advisory seat snapshot, relayed as is.
    // cap: SEAT_STATUS_CAP
    SeatStatus(SeatStatusFrame),
    /// Resolve an address on the receiving node (remote addresses).
    // cap: ADDR_RESOLVE_CAP
    AddrResolve(AddrResolve),
    /// The answer to one `addr_resolve`.
    // cap: ADDR_RESOLVE_CAP
    AddrResolved(AddrResolved),
    #[serde(other)]
    Unknown,
}

impl PeerFrame {
    /// The frame's JSON body, ready for `write_frame`.
    pub fn encode(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    /// Decode one frame body. Fork frames are validated here: a
    /// `branch_want` outside `1 <= from <= tip.counter`, a `branch_entries`
    /// with 0 or more than [`MAX_FRAME_ENTRIES`] entries, or a `have` with
    /// more than [`MAX_HAVE_FORKS`] fork refs is an error (the link answers
    /// `protocol_error`).
    pub fn decode(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        use serde::de::Error as _;
        #[derive(Deserialize)]
        struct Tag {
            t: String,
        }
        // Items stay raw text until probed: see [`WireItem`].
        #[derive(Deserialize)]
        struct EntriesBody {
            node: NodeUuid,
            inc: u32,
            entries: Vec<Box<serde_json::value::RawValue>>,
        }
        #[derive(Deserialize)]
        struct BranchEntriesBody {
            node: NodeUuid,
            inc: u32,
            tip_hash: String,
            entries: Vec<Box<serde_json::value::RawValue>>,
            more: bool,
        }
        fn items(
            raw: Vec<Box<serde_json::value::RawValue>>,
        ) -> Result<Vec<WireItem>, serde_json::Error> {
            raw.iter().map(|r| WireItem::from_json(r.get())).collect()
        }
        let tag = serde_json::from_slice::<Tag>(bytes).ok();
        match tag.as_ref().map(|tag| tag.t.as_str()) {
            Some("entries") => {
                let EntriesBody { node, inc, entries } = serde_json::from_slice(bytes)?;
                return Ok(Self::Entries {
                    node,
                    inc,
                    entries: items(entries)?,
                });
            }
            Some("branch_entries") => {
                let body: BranchEntriesBody = serde_json::from_slice(bytes)?;
                if body.entries.is_empty() || body.entries.len() > MAX_FRAME_ENTRIES {
                    return Err(serde_json::Error::custom(
                        "branch_entries needs 1..=256 entries",
                    ));
                }
                return Ok(Self::BranchEntries {
                    node: body.node,
                    inc: body.inc,
                    tip_hash: body.tip_hash,
                    entries: items(body.entries)?,
                    more: body.more,
                });
            }
            _ => {}
        }
        let frame: Self = serde_json::from_slice(bytes)?;
        match &frame {
            Self::BranchWant(BranchWant { tip, from, .. }) if *from < 1 || *from > tip.counter => {
                Err(serde_json::Error::custom(
                    "branch_want from outside 1..=tip.counter",
                ))
            }
            Self::Have { forks, .. } if forks.len() > MAX_HAVE_FORKS => Err(
                serde_json::Error::custom("have carries more than 64 fork refs"),
            ),
            _ => Ok(frame),
        }
    }

    /// The hello in `bytes`, only if it is a JSON object whose `"t"` is
    /// `"hello"` and it is at most [`MAX_HELLO_BYTES`]. Any ordinary or
    /// service request has no `"t":"hello"` and yields `None`.
    pub fn decode_hello(bytes: &[u8]) -> Option<PeerHello> {
        if bytes.len() > MAX_HELLO_BYTES {
            return None;
        }
        let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
        if value.as_object()?.get("t")?.as_str()? != "hello" {
            return None;
        }
        match serde_json::from_value(value).ok()? {
            Self::Hello(hello) => Some(hello),
            _ => None,
        }
    }
}

/// Encode `frame` and write it with the transport's length prefix.
pub async fn write_peer_frame<W: AsyncWrite + Unpin>(
    writer: &mut W,
    frame: &PeerFrame,
) -> io::Result<()> {
    let body = frame.encode().map_err(io::Error::other)?;
    transport::write_frame(writer, &body).await
}

/// Read one length-prefixed frame body of at most `limit` bytes (the
/// negotiated `max_frame`, at most [`MAX_FRAME_BYTES`]) and decode it.
pub async fn read_peer_frame<R: AsyncRead + Unpin>(
    reader: &mut R,
    limit: usize,
) -> io::Result<PeerFrame> {
    let body = transport::read_frame_with_limit(reader, limit.min(MAX_FRAME_BYTES)).await?;
    PeerFrame::decode(&body).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::commands::Command;
    use crate::protocol::service::{ServiceRegister, ServiceRequest, ServiceWireRequest};
    use crate::protocol::wire::WireRequest;
    use serde_json::{Value, json};

    fn node(n: u128) -> NodeUuid {
        NodeUuid::from_u128(n)
    }

    fn hello() -> PeerHello {
        PeerHello {
            envelope: 1,
            node: node(7),
            tag: "aB3".parse().unwrap(),
            pubkey: "11".repeat(32),
            boot: Uuid::from_u128(9),
            caps: vec![BASE_CAP.into()],
            limits: PeerLimits {
                max_frame: 1 << 20,
                batch_entries: 256,
                batch_bytes: 256 * 1024,
                credit: 1 << 20,
            },
            nonce: "22".repeat(32),
            role: Role::Dialer,
        }
    }

    fn wire_envelope() -> WireEnvelope {
        WireEnvelope {
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
            payload: serde_json::value::RawValue::from_string(r#"{ "b":1, "a":2 }"#.into())
                .unwrap(),
            hash: "00".repeat(32),
            sig: "00".repeat(64),
        }
    }

    fn sealed_envelope() -> crate::sync::envelope::Envelope {
        use crate::node::NodeIdentity;
        use crate::sync::LogKey;
        use crate::sync::envelope::{Deps, UnsignedEnvelope};
        use crate::sync::hlc::HlcStamp;
        let id = NodeIdentity::from_seed([1; 32]);
        UnsignedEnvelope {
            log: LogKey {
                node: id.uuid(),
                incarnation: 1,
            },
            counter: 7,
            prev_hash: [0xab; 32],
            hlc: HlcStamp::from_parts(1_700_000_000_123, 9),
            kind: "thread.message".into(),
            kind_version: 2,
            deps: Deps::empty(),
            creates: None,
            needs: vec![],
            payload: br#"{ "b":1,  "a":[true] }"#.to_vec(),
        }
        .seal(&id)
    }

    #[test]
    fn entries_frames_golden_bytes() {
        let mut second = wire_envelope();
        second.counter = 3;
        let sealed = WireEnvelope::from(&sealed_envelope());
        let entries = PeerFrame::Entries {
            node: node(1),
            inc: 2,
            entries: vec![wire_envelope().into(), second.into(), sealed.into()],
        };
        let branch = PeerFrame::BranchEntries {
            node: node(1),
            inc: 2,
            tip_hash: "bb".repeat(32),
            entries: vec![wire_envelope().into()],
            more: true,
        };
        // Captured from the code before `WireItem` existed (ht-j87z.4): a
        // known-layout entry is the same bytes on the wire as in 2.1.1.
        assert_eq!(
            String::from_utf8(entries.encode().unwrap()).unwrap(),
            r##"{"t":"entries","node":"00000000000000000000000000000001","inc":2,"entries":[{"node":"00000000000000000000000000000000","inc":1,"counter":2,"prev":"0000000000000000000000000000000000000000000000000000000000000000","hlc":5,"kind":"k","ver":1,"deps":"0000","needs":[],"payload":{ "b":1, "a":2 },"hash":"0000000000000000000000000000000000000000000000000000000000000000","sig":"00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"},{"node":"00000000000000000000000000000000","inc":1,"counter":3,"prev":"0000000000000000000000000000000000000000000000000000000000000000","hlc":5,"kind":"k","ver":1,"deps":"0000","needs":[],"payload":{ "b":1, "a":2 },"hash":"0000000000000000000000000000000000000000000000000000000000000000","sig":"00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"},{"node":"34750f98bd59fcfc946da45aaabe933b","inc":1,"counter":7,"prev":"abababababababababababababababababababababababababababababababab","hlc":111411200008060937,"kind":"thread.message","ver":2,"deps":"0000","needs":[],"payload":{ "b":1,  "a":[true] },"hash":"5944b87d81a846626e1b4b52f6b810e2d98f983b600299c6fb054a78b9ce749b","sig":"e626f50132e0fdab513a9b8753940e1363c09e46447d2a69b02353218c0309c6216bc2a7563261275fb7b3ee7ff7d2c07273e371d47daed394a484d924a9fe01"}]}"##
        );
        assert_eq!(
            String::from_utf8(branch.encode().unwrap()).unwrap(),
            r##"{"t":"branch_entries","node":"00000000000000000000000000000001","inc":2,"tip_hash":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","entries":[{"node":"00000000000000000000000000000000","inc":1,"counter":2,"prev":"0000000000000000000000000000000000000000000000000000000000000000","hlc":5,"kind":"k","ver":1,"deps":"0000","needs":[],"payload":{ "b":1, "a":2 },"hash":"0000000000000000000000000000000000000000000000000000000000000000","sig":"00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"}],"more":true}"##
        );
    }

    fn all_frames() -> Vec<PeerFrame> {
        vec![
            PeerFrame::Hello(hello()),
            PeerFrame::Auth {
                sig: "33".repeat(64),
            },
            PeerFrame::Accept,
            PeerFrame::Close {
                reason: CloseReason::Busy,
                detail: Some("peer connections full (16)".into()),
            },
            PeerFrame::Close {
                reason: CloseReason::Shutdown,
                detail: None,
            },
            PeerFrame::Have {
                full: true,
                ask: true,
                logs: vec![
                    HaveLog {
                        node: node(1),
                        inc: 2,
                        counter: 3,
                        head: "44".repeat(32),
                        frozen_at: Some(1_700_000_000_000),
                    },
                    HaveLog {
                        node: node(2),
                        inc: 1,
                        counter: 0,
                        head: "00".repeat(32),
                        frozen_at: None,
                    },
                ],
                forks: vec![],
            },
            PeerFrame::Want {
                node: node(1),
                inc: 2,
                from: 1,
            },
            PeerFrame::Unwant {
                node: node(1),
                inc: 2,
            },
            PeerFrame::Entries {
                node: node(1),
                inc: 2,
                entries: vec![wire_envelope().into()],
            },
            PeerFrame::Credit { bytes: 4096 },
            PeerFrame::Ping { n: 1 },
            PeerFrame::Pong { n: 1 },
            PeerFrame::Seen {
                ages: vec![SeenAge {
                    node: node(3),
                    age_ms: 1500,
                }],
            },
            PeerFrame::NodeInfo(NodeInfo {
                version: "2.4.0".to_owned(),
            }),
            PeerFrame::SeatStatus(SeatStatusFrame {
                origin: node(4),
                boot: Uuid::from_u128(7),
                rev: 3,
                age_ms: 1200,
                version: "2.4.0".to_owned(),
                seats: vec![SeatStatusEntry {
                    seat: "sQ0VnTDasAbY".to_owned(),
                    presence: "idle".to_owned(),
                    harness: Some("claude".to_owned()),
                    activity_age_ms: Some(720_000),
                    workspace: Some("office".to_owned()),
                    tab: Some("main".to_owned()),
                    pane: None,
                }],
            }),
            PeerFrame::AddrResolve(AddrResolve {
                id: Uuid::from_u128(11),
                target: AddrTarget::Seat {
                    location: "office/main/claude".to_owned(),
                },
            }),
            PeerFrame::AddrResolved(AddrResolved {
                id: Uuid::from_u128(11),
                outcome: AddrOutcome::Ambiguous {
                    candidates: vec![AddrCandidate {
                        path: "office / main / claude".to_owned(),
                        pane: "w1:p1".to_owned(),
                        seat: Some("snSGabcdefgh".to_owned()),
                    }],
                    omitted: 0,
                },
            }),
        ]
    }

    fn json_of(frame: &PeerFrame) -> Value {
        serde_json::from_slice(&frame.encode().unwrap()).unwrap()
    }

    #[test]
    fn every_variant_round_trips() {
        let frames = all_frames();
        assert_eq!(frames.len(), 17);
        for frame in &frames {
            let bytes = frame.encode().unwrap();
            let back = PeerFrame::decode(&bytes).unwrap();
            assert!(!matches!(back, PeerFrame::Unknown), "{frame:?}");
            // Equality through the wire form; WireEnvelope has no PartialEq.
            assert_eq!(back.encode().unwrap(), bytes, "{frame:?}");
        }
        // The tag spelling is part of the wire contract.
        let tags: Vec<String> = frames
            .iter()
            .map(|f| json_of(f)["t"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            tags,
            [
                "hello",
                "auth",
                "accept",
                "close",
                "close",
                "have",
                "want",
                "unwant",
                "entries",
                "credit",
                "ping",
                "pong",
                "seen",
                "node_info",
                "seat_status",
                "addr_resolve",
                "addr_resolved"
            ]
        );
        // The entry payload keeps its exact bytes.
        let entries = PeerFrame::Entries {
            node: node(1),
            inc: 1,
            entries: vec![wire_envelope().into()],
        }
        .encode()
        .unwrap();
        assert!(
            std::str::from_utf8(&entries)
                .unwrap()
                .contains(r#""payload":{ "b":1, "a":2 }"#)
        );
    }

    #[test]
    fn addr_kinds_round_trip_and_unknown_kinds_decode() {
        for target in [
            AddrTarget::Seat {
                location: "a@b".into(),
            },
            AddrTarget::Pane {
                space: Some("w1".into()),
                tab: None,
                pane: Some("p".into()),
            },
        ] {
            let frame = PeerFrame::AddrResolve(AddrResolve {
                id: Uuid::from_u128(1),
                target: target.clone(),
            });
            let PeerFrame::AddrResolve(back) = PeerFrame::decode(&frame.encode().unwrap()).unwrap()
            else {
                panic!("addr_resolve")
            };
            assert_eq!(back.target, target);
        }
        for outcome in [
            AddrOutcome::Seat {
                seat: "s1".into(),
                path: "a / b / c".into(),
                pane: "w1:p1".into(),
            },
            AddrOutcome::NoSeat {
                path: "a / b / c".into(),
                pane: "w1:p1".into(),
            },
            AddrOutcome::NoMatch,
            AddrOutcome::HostUnavailable,
            AddrOutcome::Refused {
                detail: "busy".into(),
            },
        ] {
            let frame = PeerFrame::AddrResolved(AddrResolved {
                id: Uuid::from_u128(2),
                outcome: outcome.clone(),
            });
            let PeerFrame::AddrResolved(back) =
                PeerFrame::decode(&frame.encode().unwrap()).unwrap()
            else {
                panic!("addr_resolved")
            };
            assert_eq!(back.outcome, outcome);
        }
        let future = br#"{"t":"addr_resolve","id":"00000000-0000-0000-0000-000000000003","target":{"k":"thread","x":1}}"#;
        let PeerFrame::AddrResolve(req) = PeerFrame::decode(future).unwrap() else {
            panic!("addr_resolve")
        };
        assert_eq!(req.target, AddrTarget::Unknown);
        let future = br#"{"t":"addr_resolved","id":"00000000-0000-0000-0000-000000000003","outcome":{"k":"moved","to":"x"}}"#;
        let PeerFrame::AddrResolved(reply) = PeerFrame::decode(future).unwrap() else {
            panic!("addr_resolved")
        };
        assert_eq!(reply.outcome, AddrOutcome::Unknown);
    }

    #[test]
    fn unknown_t_decodes_to_unknown() {
        for text in [
            r#"{"t":"fork_offer","x":1}"#,
            r#"{"t":"hello_v2"}"#,
            r#"{"t":"credit_v2","bytes":"nope"}"#,
        ] {
            assert!(
                matches!(PeerFrame::decode(text.as_bytes()), Ok(PeerFrame::Unknown)),
                "{text}"
            );
        }
        assert!(PeerFrame::decode(br#"{"no_tag":1}"#).is_err());
        assert!(PeerFrame::decode(b"not json").is_err());
    }

    #[test]
    fn unknown_fields_tolerated() {
        for frame in all_frames() {
            let mut value = json_of(&frame);
            value
                .as_object_mut()
                .unwrap()
                .insert("future".into(), json!({"a": [1]}));
            // Nested structs tolerate extras too.
            if let Some(logs) = value.get_mut("logs").and_then(Value::as_array_mut) {
                logs[0]["extra"] = json!(true);
            }
            if let Some(limits) = value.get_mut("limits") {
                limits["extra"] = json!(1);
            }
            if let Some(ages) = value.get_mut("ages").and_then(Value::as_array_mut) {
                ages[0]["extra"] = json!(1);
            }
            let back = PeerFrame::decode(&serde_json::to_vec(&value).unwrap()).unwrap();
            // Compare as values: the test's own Value round trip reorders the payload.
            assert_eq!(json_of(&back), json_of(&frame));
        }
        // Optional fields may be absent.
        let have = PeerFrame::decode(br#"{"t":"have","full":false,"logs":[]}"#).unwrap();
        assert!(matches!(have, PeerFrame::Have { ask: false, .. }));
    }

    fn requests() -> Vec<Vec<u8>> {
        let wire = WireRequest {
            version: 1,
            request_id: "r".into(),
            expected_instance: "i".into(),
            expected_boot: None,
            output: None,
            command: Command::Health,
        };
        let service = ServiceWireRequest {
            version: 1,
            request_id: "r".into(),
            expected_instance: "i".into(),
            service: ServiceRequest::Register(ServiceRegister {
                capability: "c".into(),
            }),
        };
        vec![
            serde_json::to_vec(&wire).unwrap(),
            serde_json::to_vec(&service).unwrap(),
        ]
    }

    #[test]
    fn decode_hello_rejects_non_hello_and_wire_requests() {
        let good = PeerFrame::Hello(hello()).encode().unwrap();
        assert_eq!(PeerFrame::decode_hello(&good), Some(hello()));
        for bytes in requests() {
            assert!(PeerFrame::decode_hello(&bytes).is_none());
        }
        for frame in all_frames().into_iter().skip(1) {
            assert!(PeerFrame::decode_hello(&frame.encode().unwrap()).is_none());
        }
        for text in [
            "[]",
            "null",
            "\"hello\"",
            r#"{"t":"Hello"}"#,
            r#"{"t":"hello"}"#,
            r#"{"t":["hello"]}"#,
            "",
            "{",
        ] {
            assert!(PeerFrame::decode_hello(text.as_bytes()).is_none(), "{text}");
        }
    }

    #[test]
    fn decode_hello_rejects_over_8k() {
        let mut at_limit = hello();
        // Pad caps until the frame is exactly MAX_HELLO_BYTES.
        let base = PeerFrame::Hello(at_limit.clone()).encode().unwrap().len();
        at_limit.caps = vec!["x".repeat(MAX_HELLO_BYTES - base + BASE_CAP.len())];
        let bytes = PeerFrame::Hello(at_limit.clone()).encode().unwrap();
        assert_eq!(bytes.len(), MAX_HELLO_BYTES);
        assert_eq!(PeerFrame::decode_hello(&bytes), Some(at_limit.clone()));
        at_limit.caps[0].push('x');
        let bytes = PeerFrame::Hello(at_limit).encode().unwrap();
        assert_eq!(bytes.len(), MAX_HELLO_BYTES + 1);
        assert!(PeerFrame::decode_hello(&bytes).is_none());
        // Whitespace padding past the limit is refused before parsing.
        let mut padded = PeerFrame::Hello(hello()).encode().unwrap();
        padded.resize(MAX_HELLO_BYTES + 1, b' ');
        assert!(PeerFrame::decode_hello(&padded).is_none());
    }

    #[test]
    fn close_reason_class_table() {
        use CloseClass::*;
        use CloseReason::*;
        for (reason, class) in [
            (EnvelopeUnsupported, StopDialing),
            (NodeCollision, StopDialing),
            (SelfLink, StopDialing),
            (AuthFailed, StopDialing),
            (Busy, Retry),
            (Shutdown, Retry),
            (ProtocolError, Retry),
            (Unknown, Retry),
        ] {
            assert_eq!(reason.class(), class, "{reason:?}");
        }
        for (reason, text) in [
            (EnvelopeUnsupported, "envelope_unsupported"),
            (NodeCollision, "node_collision"),
            (SelfLink, "self_link"),
            (AuthFailed, "auth_failed"),
            (Busy, "busy"),
            (Shutdown, "shutdown"),
            (ProtocolError, "protocol_error"),
            (Unknown, "unknown"),
        ] {
            assert_eq!(serde_json::to_value(reason).unwrap(), json!(text));
            assert_eq!(
                serde_json::from_value::<CloseReason>(json!(text)).unwrap(),
                reason
            );
        }
    }

    #[test]
    fn unknown_reason_is_retry() {
        let frame =
            PeerFrame::decode(br#"{"t":"close","reason":"clock_skew","detail":"x"}"#).unwrap();
        let PeerFrame::Close { reason, detail } = frame else {
            panic!("not a close: {frame:?}");
        };
        assert_eq!(reason, CloseReason::Unknown);
        assert_eq!(reason.class(), CloseClass::Retry);
        assert_eq!(detail.as_deref(), Some("x"));
    }

    #[tokio::test]
    async fn framed_round_trip_over_transport() {
        let (mut a, mut b) = tokio::io::duplex(1 << 16);
        for frame in all_frames() {
            write_peer_frame(&mut a, &frame).await.unwrap();
            let back = read_peer_frame(&mut b, MAX_FRAME_BYTES).await.unwrap();
            assert_eq!(back.encode().unwrap(), frame.encode().unwrap());
        }
        write_peer_frame(&mut a, &PeerFrame::Ping { n: 1 })
            .await
            .unwrap();
        let err = read_peer_frame(&mut b, 4).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    fn tip(counter: u64, byte: &str) -> TipRef {
        TipRef {
            counter,
            hash: byte.repeat(32),
        }
    }

    fn fork_ref(n: u128) -> ForkRef {
        ForkRef {
            node: node(n),
            inc: 2,
            held: tip(5, "aa"),
            conflicting: tip(7, "bb"),
            frozen_at: 4,
        }
    }

    #[test]
    fn branch_frames_round_trip() {
        let frames = [
            PeerFrame::BranchWant(BranchWant {
                node: node(1),
                inc: 2,
                tip: tip(7, "bb"),
                from: 5,
            }),
            PeerFrame::BranchEntries {
                node: node(1),
                inc: 2,
                tip_hash: "bb".repeat(32),
                entries: vec![wire_envelope().into()],
                more: true,
            },
            PeerFrame::BranchNone {
                node: node(1),
                inc: 2,
                tip_hash: "bb".repeat(32),
            },
        ];
        for (frame, tag) in frames
            .iter()
            .zip(["branch_want", "branch_entries", "branch_none"])
        {
            let bytes = frame.encode().unwrap();
            assert_eq!(json_of(frame)["t"], tag);
            let back = PeerFrame::decode(&bytes).unwrap();
            assert!(!matches!(back, PeerFrame::Unknown), "{frame:?}");
            assert_eq!(back.encode().unwrap(), bytes, "{frame:?}");
        }
        // The entry payload keeps its exact bytes.
        let text = String::from_utf8(frames[1].encode().unwrap()).unwrap();
        assert!(text.contains(r#""payload":{ "b":1, "a":2 }"#));
        // 0 and 257 entries are refused.
        for count in [0usize, MAX_FRAME_ENTRIES + 1] {
            let frame = PeerFrame::BranchEntries {
                node: node(1),
                inc: 2,
                tip_hash: "bb".repeat(32),
                entries: (0..count).map(|_| wire_envelope().into()).collect(),
                more: false,
            };
            assert!(
                PeerFrame::decode(&frame.encode().unwrap()).is_err(),
                "{count}"
            );
        }
    }

    #[test]
    fn have_forks_round_trip() {
        let have = PeerFrame::Have {
            full: true,
            ask: false,
            logs: vec![],
            forks: vec![fork_ref(1), fork_ref(2)],
        };
        let bytes = have.encode().unwrap();
        let PeerFrame::Have { forks, .. } = PeerFrame::decode(&bytes).unwrap() else {
            panic!("not a have");
        };
        assert_eq!(forks, vec![fork_ref(1), fork_ref(2)]);
        let value = json_of(&have);
        assert_eq!(value["forks"][0]["held"]["counter"], 5);
        assert_eq!(value["forks"][0]["conflicting"]["hash"], "bb".repeat(32));
        assert_eq!(value["forks"][0]["frozen_at"], 4);
        // An empty list is omitted from the wire; an absent one decodes empty.
        let plain = PeerFrame::Have {
            full: false,
            ask: false,
            logs: vec![],
            forks: vec![],
        };
        assert!(json_of(&plain).get("forks").is_none());
        let PeerFrame::Have { forks, .. } =
            PeerFrame::decode(br#"{"t":"have","full":true,"logs":[]}"#).unwrap()
        else {
            panic!("not a have");
        };
        assert!(forks.is_empty());
        // At most 64 refs.
        let at_limit = PeerFrame::Have {
            full: true,
            ask: false,
            logs: vec![],
            forks: (0..MAX_HAVE_FORKS as u128).map(fork_ref).collect(),
        };
        assert!(PeerFrame::decode(&at_limit.encode().unwrap()).is_ok());
        let over = PeerFrame::Have {
            full: true,
            ask: false,
            logs: vec![],
            forks: (0..=MAX_HAVE_FORKS as u128).map(fork_ref).collect(),
        };
        assert!(PeerFrame::decode(&over.encode().unwrap()).is_err());
    }

    #[test]
    fn capless_reader_ignores_forks_and_decodes_branch_as_unknown() {
        // The frame enum as it stood before fork.v1: no `forks`, no `branch_*`.
        #[derive(Debug, Deserialize)]
        #[serde(tag = "t", rename_all = "snake_case")]
        enum OldFrame {
            Have {
                full: bool,
                #[serde(default)]
                ask: bool,
                logs: Vec<HaveLog>,
            },
            #[serde(other)]
            Unknown,
        }
        let have = PeerFrame::Have {
            full: true,
            ask: true,
            logs: vec![],
            forks: vec![fork_ref(1)],
        };
        let old: OldFrame = serde_json::from_slice(&have.encode().unwrap()).unwrap();
        assert!(matches!(
            old,
            OldFrame::Have {
                full: true,
                ask: true,
                ref logs,
            } if logs.is_empty()
        ));
        for frame in [
            PeerFrame::BranchWant(BranchWant {
                node: node(1),
                inc: 2,
                tip: tip(7, "bb"),
                from: 5,
            }),
            PeerFrame::BranchEntries {
                node: node(1),
                inc: 2,
                tip_hash: "bb".repeat(32),
                entries: vec![wire_envelope().into()],
                more: false,
            },
            PeerFrame::BranchNone {
                node: node(1),
                inc: 2,
                tip_hash: "bb".repeat(32),
            },
        ] {
            let old: OldFrame = serde_json::from_slice(&frame.encode().unwrap()).unwrap();
            assert!(matches!(old, OldFrame::Unknown), "{frame:?}");
        }
    }

    #[test]
    fn branch_want_from_bounds_refused() {
        let want = |from: u64, counter: u64| {
            PeerFrame::BranchWant(BranchWant {
                node: node(1),
                inc: 2,
                tip: tip(counter, "bb"),
                from,
            })
            .encode()
            .unwrap()
        };
        assert!(PeerFrame::decode(&want(0, 7)).is_err());
        assert!(PeerFrame::decode(&want(8, 7)).is_err());
        assert!(PeerFrame::decode(&want(1, 7)).is_ok());
        assert!(PeerFrame::decode(&want(7, 7)).is_ok());
    }

    // -- WireItem (ht-j87z.4) ----------------------------------------------

    fn opaque_entry_at(counter: u64) -> OpaqueEntry {
        use crate::node::NodeIdentity;
        use crate::test_support::opaque_entry::{synthetic_entry, synthetic_unsigned};
        let id = NodeIdentity::from_seed([5; 32]);
        synthetic_entry(&id, &synthetic_unsigned(&id, 1, counter, [0x11; 32]))
    }

    fn opaque_wire(counter: u64) -> OpaqueWire {
        match WireItem::from(&opaque_entry_at(counter)) {
            WireItem::Opaque(wire) => wire,
            other => panic!("not opaque: {other:?}"),
        }
    }

    #[test]
    fn wire_item_known_round_trips_byte_identical() {
        let sealed = sealed_envelope();
        let item = WireItem::from(&sealed);
        let text = serde_json::to_string(&item).unwrap();
        // The bare 2.1.1 form, byte for byte.
        assert_eq!(
            text,
            serde_json::to_string(&WireEnvelope::from(&sealed)).unwrap()
        );
        assert!(text.contains(r#""payload":{ "b":1,  "a":[true] }"#));
        let back = WireItem::from_json(&text).unwrap();
        assert!(matches!(back, WireItem::Known(_)), "{back:?}");
        assert_eq!(serde_json::to_string(&back).unwrap(), text);
        assert_eq!(back.counter(), 7);
        assert_eq!(back.into_stored().unwrap(), StoredEntry::Known(sealed));
    }

    #[test]
    fn wire_item_opaque_round_trip() {
        let entry = opaque_entry_at(3);
        let item = WireItem::from(&entry);
        let text = serde_json::to_string(&item).unwrap();
        assert!(text.starts_with(r#"{"opaque":{"node":""#), "{text}");
        let back = WireItem::from_json(&text).unwrap();
        assert!(matches!(back, WireItem::Opaque(_)), "{back:?}");
        assert_eq!(back.counter(), 3);
        assert_eq!(serde_json::to_string(&back).unwrap(), text);
        // The raw bytes survive untouched.
        assert_eq!(back.into_stored().unwrap(), StoredEntry::Opaque(entry));
    }

    #[test]
    fn opaque_item_with_unknown_fields_decodes() {
        let entry = opaque_entry_at(3);
        let mut value = serde_json::to_value(WireItem::from(&entry)).unwrap();
        value["opaque"]["future"] = json!({"a": [1]});
        value["extra"] = json!(true);
        let item = WireItem::from_json(&value.to_string()).unwrap();
        assert_eq!(item.into_stored().unwrap(), StoredEntry::Opaque(entry));
    }

    #[test]
    fn entries_mixed_items_decode() {
        let known = sealed_envelope();
        let opaque = opaque_entry_at(3);
        let frame = PeerFrame::Entries {
            node: node(1),
            inc: 2,
            entries: vec![
                WireItem::from(&known),
                WireItem::from(&opaque),
                WireItem::from(&known),
            ],
        };
        let bytes = frame.encode().unwrap();
        let PeerFrame::Entries { entries, .. } = PeerFrame::decode(&bytes).unwrap() else {
            panic!("not entries");
        };
        let kinds: Vec<bool> = entries
            .iter()
            .map(|e| matches!(e, WireItem::Opaque(_)))
            .collect();
        assert_eq!(kinds, [false, true, false]);
        let stored: Vec<StoredEntry> = entries
            .into_iter()
            .map(|e| e.into_stored().unwrap())
            .collect();
        assert_eq!(stored[0], StoredEntry::Known(known.clone()));
        assert_eq!(stored[1], StoredEntry::Opaque(opaque));
        assert_eq!(stored[2], StoredEntry::Known(known));
        // A malformed item anywhere fails the frame decode, as before.
        let broken = br#"{"t":"entries","node":"00000000000000000000000000000001","inc":1,"entries":[{"counter":1}]}"#;
        assert!(PeerFrame::decode(broken).is_err());
    }

    #[test]
    fn opaque_item_oversize_is_too_large_not_envelope_error() {
        use crate::sync::envelope::OPAQUE_MAX_BYTES;
        let mut wire = opaque_wire(3);
        wire.raw = "00".repeat(OPAQUE_MAX_BYTES + 1);
        let err = WireItem::Opaque(wire.clone()).into_stored().unwrap_err();
        assert_eq!(
            err,
            WireItemError::TooLarge(OpaqueTooLarge {
                len: OPAQUE_MAX_BYTES + 1
            })
        );
        // At the bound the size check passes; the shape check refuses the
        // all-zero prefix instead.
        wire.raw = "00".repeat(OPAQUE_MAX_BYTES);
        let err = WireItem::Opaque(wire).into_stored().unwrap_err();
        assert!(matches!(err, WireItemError::Envelope(_)), "{err:?}");
    }

    #[test]
    fn opaque_item_bad_hex_is_malformed() {
        for (field, edit) in [
            (
                "node",
                (|w: &mut OpaqueWire| w.node = "zz".into()) as fn(&mut OpaqueWire),
            ),
            ("prev", |w| w.prev = "AB".repeat(32)),
            ("hash", |w| w.hash.truncate(62)),
            ("sig", |w| w.sig = "0".repeat(128) + "0"),
            ("raw", |w| w.raw = "G".repeat(w.raw.len())),
        ] {
            let mut wire = opaque_wire(3);
            edit(&mut wire);
            assert_eq!(
                WireItem::Opaque(wire).into_stored().unwrap_err(),
                WireItemError::Envelope(EnvelopeError::Malformed { field }),
                "{field}"
            );
        }
    }

    #[test]
    fn branch_entries_count_rule_counts_opaque_items() {
        let frame = |count: usize| PeerFrame::BranchEntries {
            node: node(1),
            inc: 2,
            tip_hash: "bb".repeat(32),
            entries: (0..count)
                .map(|_| WireItem::Opaque(opaque_wire(3)))
                .collect(),
            more: false,
        };
        for count in [0, MAX_FRAME_ENTRIES + 1] {
            assert!(
                PeerFrame::decode(&frame(count).encode().unwrap()).is_err(),
                "{count}"
            );
        }
        for count in [1, MAX_FRAME_ENTRIES] {
            assert!(
                PeerFrame::decode(&frame(count).encode().unwrap()).is_ok(),
                "{count}"
            );
        }
    }
}
