//! Deliberately a copy of the v2.1.1 (fd959d7aa) peer wire types from
//! `src/sync/frame.rs` and `src/sync/envelope.rs`: the decoder a 2.1.1 node
//! runs, kept so CI can prove what the current code emits still decodes there.
//!
//! Edit rule: never edit in place. A new baseline release gets a sibling file
//! `legacy_wire_v<maj><min><patch>.rs`. Derive and serde attributes are copied
//! verbatim from the tag; each deviation is commented with the tag line it
//! departs from. There is no deviation today: the `#[serde(other)] Unknown`
//! variants of `CloseReason` and `PeerFrame` are in the tag itself
//! (`src/sync/frame.rs` lines 152 and 256 of v2.1.1).
//!
//! `frozen_copy_matches_tag_source` compares each item below with the tag's
//! text, item by item, whenever `git` and the tag are available.
//!
//! Only the items a decoder needs are copied (no `CloseReason::class`,
//! `decode_hello` or the async readers). `NodeUuid` and `NodeTag` are the
//! crate's: `src/node/` is byte-identical to v2.1.1, so their serde form is
//! the tag's.
// Part of a test target that uses a subset of the copy.
#![allow(dead_code)]

use herdr_threads::node::{NodeTag, NodeUuid};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use uuid::Uuid;

/// The capability the fork frames (`branch_*`, `Have.forks`) belong to.
pub const FORK_CAP: &str = "fork.v1";
/// Most fork references in one `have` frame.
pub const MAX_HAVE_FORKS: usize = 64;
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
        entries: Vec<WireEnvelope>,
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
        entries: Vec<WireEnvelope>,
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
        #[derive(Deserialize)]
        struct EntriesBody {
            node: NodeUuid,
            inc: u32,
            entries: Vec<WireEnvelope>,
        }
        #[derive(Deserialize)]
        struct BranchEntriesBody {
            node: NodeUuid,
            inc: u32,
            tip_hash: String,
            entries: Vec<WireEnvelope>,
            more: bool,
        }
        let tag = serde_json::from_slice::<Tag>(bytes).ok();
        match tag.as_ref().map(|tag| tag.t.as_str()) {
            Some("entries") => {
                let EntriesBody { node, inc, entries } = serde_json::from_slice(bytes)?;
                return Ok(Self::Entries { node, inc, entries });
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
                    entries: body.entries,
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
}

/// The serde form of an [`Envelope`] inside peer frames: binary fields are
/// lowercase hex and `payload` is the original JSON text, never re-encoded.
#[derive(Debug, Serialize, Deserialize)]
pub struct WireEnvelope {
    pub node: String,
    pub inc: u32,
    pub counter: u64,
    pub prev: String,
    pub hlc: u64,
    pub kind: String,
    pub ver: u32,
    pub deps: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creates: Option<String>,
    pub needs: Vec<String>,
    pub payload: Box<RawValue>,
    pub hash: String,
    pub sig: String,
}

/// Decode one frame body the way a 2.1.1 node does.
pub fn decode(bytes: &[u8]) -> Result<PeerFrame, serde_json::Error> {
    PeerFrame::decode(bytes)
}

// Everything above this line is the frozen copy; the check below is not.
#[cfg(test)]
mod tag_check {
    use std::process::Command;

    const OWN: &str = include_str!("legacy_wire_v211.rs");

    /// Every copied item: the tag file, the exact line that opens it and the
    /// indent of the line that closes it.
    const ITEMS: &[(&str, &str, &str)] = &[
        (
            "src/sync/frame.rs",
            "pub const FORK_CAP: &str = \"fork.v1\";",
            "",
        ),
        (
            "src/sync/frame.rs",
            "pub const MAX_HAVE_FORKS: usize = 64;",
            "",
        ),
        (
            "src/sync/frame.rs",
            "pub const MAX_FRAME_ENTRIES: usize = 256;",
            "",
        ),
        ("src/sync/frame.rs", "pub enum Role {", ""),
        ("src/sync/frame.rs", "pub struct PeerLimits {", ""),
        ("src/sync/frame.rs", "pub struct PeerHello {", ""),
        ("src/sync/frame.rs", "pub struct HaveLog {", ""),
        ("src/sync/frame.rs", "pub struct TipRef {", ""),
        ("src/sync/frame.rs", "pub struct BranchWant {", ""),
        ("src/sync/frame.rs", "pub struct ForkRef {", ""),
        ("src/sync/frame.rs", "pub struct SeenAge {", ""),
        ("src/sync/frame.rs", "pub enum CloseReason {", ""),
        ("src/sync/frame.rs", "pub enum PeerFrame {", ""),
        (
            "src/sync/frame.rs",
            "    pub fn encode(&self) -> Result<Vec<u8>, serde_json::Error> {",
            "    ",
        ),
        (
            "src/sync/frame.rs",
            "    pub fn decode(bytes: &[u8]) -> Result<Self, serde_json::Error> {",
            "    ",
        ),
        ("src/sync/envelope.rs", "pub struct WireEnvelope {", ""),
    ];

    /// The text of the item opened by the line `marker`: its attribute and
    /// doc lines above it, through its closing line. Lines a deviation
    /// comment (`// DEVIATION`) adds are dropped, so only the tag's text is
    /// compared.
    fn item_text(source: &str, marker: &str, indent: &str) -> Option<String> {
        let lines: Vec<&str> = source
            .lines()
            .filter(|l| !l.trim_start().starts_with("// DEVIATION"))
            .collect();
        let open = lines.iter().position(|l| *l == marker)?;
        let mut start = open;
        while start > 0 {
            let above = lines[start - 1].trim_start();
            if above.starts_with("#[") || above.starts_with("///") {
                start -= 1;
            } else {
                break;
            }
        }
        let close = if marker.ends_with(';') {
            open
        } else {
            let closing = format!("{indent}}}");
            open + lines[open..].iter().position(|l| *l == closing)?
        };
        Some(lines[start..=close].join("\n"))
    }

    fn git(args: &[&str]) -> Option<String> {
        let out = Command::new("git")
            .args(args)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// `Ok(true)`: compare; `Ok(false)`: skip locally; `Err`: the tag is
    /// missing where `HT_REQUIRE_TAGS=1` says CI must have it.
    fn tag_gate(available: bool, require: Option<&str>) -> Result<bool, String> {
        if available {
            Ok(true)
        } else if require == Some("1") {
            Err(
                "HT_REQUIRE_TAGS=1 but git or the v2.1.1 tag is not available \
                 (CI must fetch the tags before the test job)"
                    .to_owned(),
            )
        } else {
            Ok(false)
        }
    }

    #[test]
    fn tag_gate_fails_only_when_required() {
        assert_eq!(tag_gate(true, Some("1")), Ok(true));
        assert_eq!(tag_gate(false, None), Ok(false));
        assert_eq!(tag_gate(false, Some("0")), Ok(false));
        assert!(tag_gate(false, Some("1")).is_err());
    }

    #[test]
    fn frozen_copy_matches_tag_source() {
        let available = git(&["cat-file", "-e", "v2.1.1^{commit}"]).is_some();
        match tag_gate(available, std::env::var("HT_REQUIRE_TAGS").ok().as_deref()) {
            Ok(true) => {}
            Ok(false) => {
                eprintln!("skipped: git or the v2.1.1 tag is not available here");
                return;
            }
            Err(why) => panic!("{why}"),
        }
        let mut sources = std::collections::BTreeMap::new();
        for (file, marker, indent) in ITEMS {
            let tag_source = sources.entry(*file).or_insert_with(|| {
                git(&["show", &format!("v2.1.1:{file}")]).expect("git show of the tag file")
            });
            let theirs = item_text(tag_source, marker, indent)
                .unwrap_or_else(|| panic!("{marker:?} not found in {file} at v2.1.1"));
            let ours = item_text(OWN, marker, indent)
                .unwrap_or_else(|| panic!("{marker:?} not found in the frozen copy"));
            assert_eq!(ours, theirs, "{marker:?} differs from {file} at v2.1.1");
        }
    }

    #[test]
    fn item_text_names_a_difference() {
        // The comparison is not decoration: a copy that drops one attribute
        // yields different text.
        let tag = "#[serde(other)]\npub enum A {\n    B,\n}";
        let copy = "pub enum A {\n    B,\n}";
        assert_ne!(
            item_text(tag, "pub enum A {", ""),
            item_text(copy, "pub enum A {", "")
        );
        let with_note = "// DEVIATION: x\n#[serde(other)]\npub enum A {\n    B,\n}";
        assert_eq!(
            item_text(tag, "pub enum A {", ""),
            item_text(with_note, "pub enum A {", "")
        );
    }
}
