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

use super::envelope::WireEnvelope;
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
/// Longest `close.detail`, in bytes.
pub const MAX_CLOSE_DETAIL_BYTES: usize = 256;
/// Most logs in one `have` frame.
pub const MAX_HAVE_LOGS: usize = 4096;
/// Most ages in one `seen` frame.
pub const MAX_SEEN_AGES: usize = 4096;
/// Most entries in one `entries` frame.
pub const MAX_FRAME_ENTRIES: usize = 256;

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
        entries: Vec<WireEnvelope>,
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
    #[serde(other)]
    Unknown,
}

impl PeerFrame {
    /// The frame's JSON body, ready for `write_frame`.
    pub fn encode(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        #[derive(Deserialize)]
        struct Tag {
            t: String,
        }
        #[derive(Deserialize)]
        struct EntriesBody {
            node: NodeUuid,
            inc: u32,
            entries: Vec<WireEnvelope>,
        }
        if serde_json::from_slice::<Tag>(bytes).is_ok_and(|tag| tag.t == "entries") {
            let EntriesBody { node, inc, entries } = serde_json::from_slice(bytes)?;
            return Ok(Self::Entries { node, inc, entries });
        }
        serde_json::from_slice(bytes)
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
                entries: vec![wire_envelope()],
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
        ]
    }

    fn json_of(frame: &PeerFrame) -> Value {
        serde_json::from_slice(&frame.encode().unwrap()).unwrap()
    }

    #[test]
    fn every_variant_round_trips() {
        let frames = all_frames();
        assert_eq!(frames.len(), 13);
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
                "hello", "auth", "accept", "close", "close", "have", "want", "unwant", "entries",
                "credit", "ping", "pong", "seen"
            ]
        );
        // The entry payload keeps its exact bytes.
        let entries = PeerFrame::Entries {
            node: node(1),
            inc: 1,
            entries: vec![wire_envelope()],
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
}
