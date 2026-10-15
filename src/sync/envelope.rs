//! The signed journal entry: canonical bytes, hash, signature, deps and wire
//! form (journal-core design, section 2).

use std::fmt;

use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use sha2::{Digest, Sha256};

use super::{EntityKey, LogKey, hlc::HlcStamp};
use crate::node::{NodeIdentity, NodeUuid};

/// Magic opening the canonical bytes; it also separates this signature domain
/// from the handshake signatures. A layout change is a new magic.
pub const MAGIC: &[u8; 8] = b"HTJRNL01";
/// The kind of the entry that opens a log, and the only one allowed at counter 1.
pub const GENESIS_KIND: &str = "node.genesis";
pub const KIND_MAX: usize = 64;
pub const NEEDS_MAX: usize = 256;
pub const DEPS_MAX: usize = 4096;
pub const PAYLOAD_MAX: usize = 524_288;
const DEP_LEN: usize = 28;

/// Why an envelope was refused. Shape errors stay distinct from the
/// cryptographic ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvelopeError {
    Malformed { field: &'static str },
    HashMismatch,
    BadSignature,
    UuidMismatch,
    NotGenesis,
}

impl fmt::Display for EnvelopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed { field } => write!(f, "malformed envelope field {field}"),
            Self::HashMismatch => f.write_str("envelope hash mismatch"),
            Self::BadSignature => f.write_str("envelope signature invalid"),
            Self::UuidMismatch => f.write_str("genesis pubkey does not hash to the node uuid"),
            Self::NotGenesis => f.write_str("envelope is not a node.genesis v1 at counter 1"),
        }
    }
}

impl std::error::Error for EnvelopeError {}

fn malformed(field: &'static str) -> EnvelopeError {
    EnvelopeError::Malformed { field }
}

/// An entry's dependencies: the author's applied vector at write time, as
/// sorted unique `(log, counter)` pairs, the entry's own log excluded.
///
/// Owns: the compact binary form (u16 count, then per log `node_uuid 16 ||
/// incarnation u32 || counter u64`, strictly ascending by log, counters >= 1),
/// which is unique, so `journal.deps` stores exactly these bytes. Must not:
/// know the payload or the store.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Deps(Vec<(LogKey, u64)>);

impl Deps {
    pub fn empty() -> Self {
        Self::default()
    }

    /// Validate `entries` for an entry written in log `own`.
    pub fn new(entries: Vec<(LogKey, u64)>, own: &LogKey) -> Result<Self, EnvelopeError> {
        if entries.len() > usize::from(u16::MAX) {
            return Err(malformed("deps"));
        }
        let mut prev: Option<&LogKey> = None;
        for (key, counter) in &entries {
            if *counter == 0 || key == own || prev.is_some_and(|p| p >= key) {
                return Err(malformed("deps"));
            }
            prev = Some(key);
        }
        Ok(Self(entries))
    }

    pub fn entries(&self) -> &[(LogKey, u64)] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(2 + self.0.len() * DEP_LEN);
        out.extend_from_slice(&len_u16(self.0.len(), "deps").to_be_bytes());
        for (key, counter) in &self.0 {
            out.extend_from_slice(key.node.as_bytes());
            out.extend_from_slice(&key.incarnation.to_be_bytes());
            out.extend_from_slice(&counter.to_be_bytes());
        }
        out
    }

    /// Strict inverse of [`Self::encode`]: rejects unsorted or duplicate logs,
    /// a zero counter, trailing or missing bytes and the `own` log.
    pub fn decode(bytes: &[u8], own: &LogKey) -> Result<Self, EnvelopeError> {
        let (count, rest) = bytes.split_first_chunk::<2>().ok_or(malformed("deps"))?;
        let count = usize::from(u16::from_be_bytes(*count));
        if rest.len() != count * DEP_LEN {
            return Err(malformed("deps"));
        }
        let entries = rest
            .chunks_exact(DEP_LEN)
            .map(|chunk| {
                let mut node = [0u8; 16];
                node.copy_from_slice(&chunk[..16]);
                let incarnation = u32::from_be_bytes(chunk[16..20].try_into().expect("4 bytes"));
                let counter = u64::from_be_bytes(chunk[20..28].try_into().expect("8 bytes"));
                (
                    LogKey {
                        node: NodeUuid::from_bytes(node),
                        incarnation,
                    },
                    counter,
                )
            })
            .collect();
        Self::new(entries, own)
    }
}

/// Everything in an envelope except its hash and signature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnsignedEnvelope {
    pub log: LogKey,
    pub counter: u64,
    pub prev_hash: [u8; 32],
    pub hlc: HlcStamp,
    pub kind: String,
    pub kind_version: u32,
    pub deps: Deps,
    pub creates: Option<EntityKey>,
    pub needs: Vec<EntityKey>,
    pub payload: Vec<u8>,
}

/// One journal entry's signed wire and storage form.
///
/// Owns: the fixed length-prefixed canonical bytes (big-endian, magic
/// `HTJRNL01`), the SHA-256 `hash`, signature verification against a given
/// public key, the genesis self-check (`SHA-256(pubkey)[0..16] == node uuid`)
/// and the shape limits of envelope v1. Hides: the byte layout; JSON
/// canonicalization is never used. Must not: look at the payload's meaning
/// (beyond "valid JSON" and the genesis `pubkey`) or touch the store.
/// Role: value type shared by Journal, peer-link frames and the fact registry
/// (which reads the payload bytes). Envelope fields are added only here;
/// callers never hash JSON or verify ad hoc. A layout change is a new magic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Envelope {
    pub log: LogKey,
    pub counter: u64,
    pub prev_hash: [u8; 32],
    pub hlc: HlcStamp,
    pub kind: String,
    pub kind_version: u32,
    pub deps: Deps,
    pub creates: Option<EntityKey>,
    pub needs: Vec<EntityKey>,
    pub payload: Vec<u8>,
    pub hash: [u8; 32],
    pub signature: [u8; 64],
}

/// Borrowed view of the signed fields, shared by the sealed and unsealed forms.
struct Body<'a> {
    log: &'a LogKey,
    counter: u64,
    prev_hash: &'a [u8; 32],
    hlc: HlcStamp,
    kind: &'a str,
    kind_version: u32,
    deps: &'a Deps,
    creates: Option<&'a EntityKey>,
    needs: &'a [EntityKey],
    payload: &'a [u8],
}

fn len_u16(len: usize, what: &str) -> u16 {
    u16::try_from(len)
        .unwrap_or_else(|_| panic!("{what} length {len} exceeds u16: validate_shape first"))
}

fn len_u32(len: usize, what: &str) -> u32 {
    u32::try_from(len)
        .unwrap_or_else(|_| panic!("{what} length {len} exceeds u32: validate_shape first"))
}

impl Body<'_> {
    fn canonical_bytes(&self) -> Vec<u8> {
        let deps = self.deps.encode();
        let mut out = Vec::with_capacity(160 + deps.len() + self.payload.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(self.log.node.as_bytes());
        out.extend_from_slice(&self.log.incarnation.to_be_bytes());
        out.extend_from_slice(&self.counter.to_be_bytes());
        out.extend_from_slice(self.prev_hash);
        out.extend_from_slice(&self.hlc.as_u64().to_be_bytes());
        out.extend_from_slice(&len_u16(self.kind.len(), "kind").to_be_bytes());
        out.extend_from_slice(self.kind.as_bytes());
        out.extend_from_slice(&self.kind_version.to_be_bytes());
        out.extend_from_slice(&len_u32(deps.len(), "deps").to_be_bytes());
        out.extend_from_slice(&deps);
        match self.creates {
            None => out.push(0),
            Some(key) => {
                out.push(1);
                push_key(&mut out, key);
            }
        }
        out.extend_from_slice(&len_u16(self.needs.len(), "needs").to_be_bytes());
        for key in self.needs {
            push_key(&mut out, key);
        }
        out.extend_from_slice(&len_u32(self.payload.len(), "payload").to_be_bytes());
        out.extend_from_slice(self.payload);
        out
    }

    fn validate_shape(&self) -> Result<(), EnvelopeError> {
        if self.counter == 0 || self.counter > i64::MAX as u64 {
            return Err(malformed("counter"));
        }
        if !valid_kind(self.kind) {
            return Err(malformed("kind"));
        }
        if self.kind_version == 0 || self.kind_version > u32::from(u16::MAX) {
            return Err(malformed("kind_version"));
        }
        let genesis = self.kind == GENESIS_KIND;
        if (self.counter == 1) != genesis || (genesis && self.kind_version != 1) {
            return Err(malformed("kind"));
        }
        if self.counter == 1 && *self.prev_hash != [0u8; 32] {
            return Err(malformed("prev_hash"));
        }
        if self.deps.len() > DEPS_MAX || self.deps.entries().iter().any(|(k, _)| k == self.log) {
            return Err(malformed("deps"));
        }
        if self.needs.len() > NEEDS_MAX || self.needs.windows(2).any(|w| w[0] >= w[1]) {
            return Err(malformed("needs"));
        }
        let edge_ws = |b: Option<&u8>| b.is_none_or(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r'));
        if self.payload.len() > PAYLOAD_MAX
            || edge_ws(self.payload.first())
            || edge_ws(self.payload.last())
            || serde_json::from_slice::<serde::de::IgnoredAny>(self.payload).is_err()
        {
            return Err(malformed("payload"));
        }
        Ok(())
    }
}

fn push_key(out: &mut Vec<u8>, key: &EntityKey) {
    out.extend_from_slice(&len_u16(key.as_str().len(), "entity key").to_be_bytes());
    out.extend_from_slice(key.as_str().as_bytes());
}

/// `^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)+$`, 1..=64 bytes.
fn valid_kind(kind: &str) -> bool {
    if kind.is_empty() || kind.len() > KIND_MAX {
        return false;
    }
    let mut segments = 0;
    for segment in kind.split('.') {
        segments += 1;
        let mut bytes = segment.bytes();
        if !bytes.next().is_some_and(|b| b.is_ascii_lowercase())
            || !bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            return false;
        }
    }
    segments >= 2
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

impl UnsignedEnvelope {
    fn body(&self) -> Body<'_> {
        Body {
            log: &self.log,
            counter: self.counter,
            prev_hash: &self.prev_hash,
            hlc: self.hlc,
            kind: &self.kind,
            kind_version: self.kind_version,
            deps: &self.deps,
            creates: self.creates.as_ref(),
            needs: &self.needs,
            payload: &self.payload,
        }
    }

    /// The envelope v1 limits; see [`Envelope::validate_shape`].
    pub fn validate_shape(&self) -> Result<(), EnvelopeError> {
        self.body().validate_shape()
    }

    pub fn canonical_bytes(&self) -> Vec<u8> {
        self.body().canonical_bytes()
    }

    /// Hash the canonical bytes and sign the hash. The caller has run
    /// [`Self::validate_shape`]; a field too long to length-prefix panics.
    pub fn seal(self, identity: &NodeIdentity) -> Envelope {
        let hash = sha256(&self.canonical_bytes());
        let signature = identity.sign(&hash);
        Envelope {
            log: self.log,
            counter: self.counter,
            prev_hash: self.prev_hash,
            hlc: self.hlc,
            kind: self.kind,
            kind_version: self.kind_version,
            deps: self.deps,
            creates: self.creates,
            needs: self.needs,
            payload: self.payload,
            hash,
            signature,
        }
    }
}

impl Envelope {
    fn body(&self) -> Body<'_> {
        Body {
            log: &self.log,
            counter: self.counter,
            prev_hash: &self.prev_hash,
            hlc: self.hlc,
            kind: &self.kind,
            kind_version: self.kind_version,
            deps: &self.deps,
            creates: self.creates.as_ref(),
            needs: &self.needs,
            payload: &self.payload,
        }
    }

    pub fn canonical_bytes(&self) -> Vec<u8> {
        self.body().canonical_bytes()
    }

    /// SHA-256 of the canonical bytes (not the stored `hash` field).
    pub fn compute_hash(&self) -> [u8; 32] {
        sha256(&self.canonical_bytes())
    }

    /// Recompute the hash (`HashMismatch`), then check the signature over it
    /// with `verify_strict` (`BadSignature`, also for an invalid public key).
    pub fn verify(&self, pubkey: &[u8; 32]) -> Result<(), EnvelopeError> {
        if self.compute_hash() != self.hash {
            return Err(EnvelopeError::HashMismatch);
        }
        let key = VerifyingKey::from_bytes(pubkey).map_err(|_| EnvelopeError::BadSignature)?;
        key.verify_strict(&self.hash, &Signature::from_bytes(&self.signature))
            .map_err(|_| EnvelopeError::BadSignature)
    }

    /// The public key a `node.genesis` v1 entry at counter 1 announces. Parses
    /// only `pubkey` from the payload and checks it hashes to the log's node.
    pub fn genesis_pubkey(&self) -> Result<[u8; 32], EnvelopeError> {
        if self.counter != 1 || self.kind != GENESIS_KIND || self.kind_version != 1 {
            return Err(EnvelopeError::NotGenesis);
        }
        #[derive(Deserialize)]
        struct Genesis {
            pubkey: String,
        }
        let genesis: Genesis =
            serde_json::from_slice(&self.payload).map_err(|_| malformed("payload"))?;
        let pubkey = hex_decode::<32>(&genesis.pubkey).ok_or(malformed("payload"))?;
        if NodeUuid::from_public_key(&pubkey) != self.log.node {
            return Err(EnvelopeError::UuidMismatch);
        }
        Ok(pubkey)
    }

    /// The envelope v1 limits: counter in `1..=i64::MAX`; `kind` grammar and
    /// 1..=64 bytes; `kind_version` 1..=65535; at most 256 sorted unique
    /// `needs`; at most 4096 deps, none in the own log; payload at most
    /// 524288 bytes of JSON with no leading or trailing whitespace (so the
    /// wire form can carry it verbatim); counter 1 is `node.genesis` v1 with a
    /// zero `prev_hash`, and `node.genesis` appears only at counter 1.
    pub fn validate_shape(&self) -> Result<(), EnvelopeError> {
        self.body().validate_shape()
    }
}

pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from(DIGITS[usize::from(b >> 4)]));
        out.push(char::from(DIGITS[usize::from(b & 15)]));
    }
    out
}

fn nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    }
}

/// Lowercase hex only, so each byte string has one spelling.
fn hex_decode_vec(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    bytes
        .chunks_exact(2)
        .map(|pair| Some(nibble(pair[0])? << 4 | nibble(pair[1])?))
        .collect()
}

pub(super) fn hex_decode<const N: usize>(text: &str) -> Option<[u8; N]> {
    hex_decode_vec(text)?.try_into().ok()
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

impl From<&Envelope> for WireEnvelope {
    /// Envelopes are shape-valid by construction (authored through
    /// `validate_shape` or built by `TryFrom<WireEnvelope>`); a payload that is
    /// not JSON text would travel as `null` and fail the hash on arrival.
    fn from(e: &Envelope) -> Self {
        let payload = std::str::from_utf8(&e.payload)
            .ok()
            .and_then(|text| RawValue::from_string(text.to_owned()).ok())
            .unwrap_or_else(|| RawValue::from_string("null".to_owned()).expect("null is JSON"));
        Self {
            node: hex_encode(e.log.node.as_bytes()),
            inc: e.log.incarnation,
            counter: e.counter,
            prev: hex_encode(&e.prev_hash),
            hlc: e.hlc.as_u64(),
            kind: e.kind.clone(),
            ver: e.kind_version,
            deps: hex_encode(&e.deps.encode()),
            creates: e.creates.as_ref().map(|k| k.as_str().to_owned()),
            needs: e.needs.iter().map(|k| k.as_str().to_owned()).collect(),
            payload,
            hash: hex_encode(&e.hash),
            sig: hex_encode(&e.signature),
        }
    }
}

impl TryFrom<WireEnvelope> for Envelope {
    type Error = EnvelopeError;

    /// Decode every field strictly and run `validate_shape`. The payload bytes
    /// are the wire text as received. Hash and signature are not checked here:
    /// that is [`Envelope::verify`].
    fn try_from(w: WireEnvelope) -> Result<Self, EnvelopeError> {
        let log = LogKey {
            node: NodeUuid::from_bytes(hex_decode::<16>(&w.node).ok_or(malformed("node"))?),
            incarnation: w.inc,
        };
        let deps_bytes = hex_decode_vec(&w.deps).ok_or(malformed("deps"))?;
        let creates = w
            .creates
            .map(|k| EntityKey::new(k).map_err(|_| malformed("creates")))
            .transpose()?;
        let needs = w
            .needs
            .into_iter()
            .map(|k| EntityKey::new(k).map_err(|_| malformed("needs")))
            .collect::<Result<Vec<_>, _>>()?;
        let envelope = Envelope {
            counter: w.counter,
            prev_hash: hex_decode(&w.prev).ok_or(malformed("prev"))?,
            hlc: HlcStamp::from_u64(w.hlc),
            kind: w.kind,
            kind_version: w.ver,
            deps: Deps::decode(&deps_bytes, &log)?,
            creates,
            needs,
            payload: w.payload.get().as_bytes().to_vec(),
            hash: hex_decode(&w.hash).ok_or(malformed("hash"))?,
            signature: hex_decode(&w.sig).ok_or(malformed("sig"))?,
            log,
        };
        envelope.validate_shape()?;
        Ok(envelope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(n: u8) -> NodeIdentity {
        NodeIdentity::from_seed([n; 32])
    }

    fn own(id: &NodeIdentity) -> LogKey {
        LogKey {
            node: id.uuid(),
            incarnation: 1,
        }
    }

    fn genesis_payload(id: &NodeIdentity) -> Vec<u8> {
        format!(r#"{{"pubkey":"{}"}}"#, hex_encode(&id.public_key())).into_bytes()
    }

    fn genesis(id: &NodeIdentity) -> UnsignedEnvelope {
        UnsignedEnvelope {
            log: own(id),
            counter: 1,
            prev_hash: [0; 32],
            hlc: HlcStamp::from_parts(1_700_000_000_000, 3),
            kind: GENESIS_KIND.into(),
            kind_version: 1,
            deps: Deps::empty(),
            creates: None,
            needs: vec![],
            payload: genesis_payload(id),
        }
    }

    fn key(s: &str) -> EntityKey {
        EntityKey::new(s).unwrap()
    }

    fn other_log(n: u8, inc: u32) -> LogKey {
        LogKey {
            node: NodeUuid::from_bytes([n; 16]),
            incarnation: inc,
        }
    }

    /// A non-genesis entry exercising every field.
    fn full(id: &NodeIdentity) -> UnsignedEnvelope {
        UnsignedEnvelope {
            log: own(id),
            counter: 7,
            prev_hash: [0xab; 32],
            hlc: HlcStamp::from_parts(1_700_000_000_123, 9),
            kind: "thread.message".into(),
            kind_version: 2,
            deps: Deps::new(vec![(other_log(2, 1), 5), (other_log(3, 4), 9)], &own(id)).unwrap(),
            creates: Some(key("m1")),
            needs: vec![key("a.b"), key("t1")],
            payload: br#"{ "b":1,  "a":[true] }"#.to_vec(),
        }
    }

    #[test]
    fn golden_canonical_bytes_and_hash() {
        let id = identity(1);
        let bytes = full(&id).canonical_bytes();
        assert_eq!(&bytes[..8], b"HTJRNL01");
        assert_eq!(hex_encode(&bytes), GOLDEN_CANONICAL);
        let sealed = full(&id).seal(&id);
        assert_eq!(hex_encode(&sealed.hash), GOLDEN_HASH);
        assert_eq!(sealed.hash, sealed.compute_hash());
    }

    const GOLDEN_CANONICAL: &str = "48544a524e4c303134750f98bd59fcfc946da45aaabe933b000000010000000000000007abababababababababababababababababababababababababababababababab018bcfe5687b0009000e7468726561642e6d657373616765000000020000003a000202020202020202020202020202020202000000010000000000000005030303030303030303030303030303030000000400000000000000090100026d3100020003612e6200027431000000167b202262223a312c20202261223a5b747275655d207d";
    const GOLDEN_HASH: &str = "78f49644fbf31c3c2dbe08a42dbf4fd66ef43229771b12a1fa6d62a5f98b1890";

    #[test]
    fn every_field_mutation_changes_hash() {
        let id = identity(1);
        let base = full(&id).seal(&id);
        type Mutator = Box<dyn Fn(&mut Envelope)>;
        let mutators: Vec<(&str, Mutator)> = vec![
            (
                "node",
                Box::new(|e| e.log.node = NodeUuid::from_bytes([9; 16])),
            ),
            ("incarnation", Box::new(|e| e.log.incarnation += 1)),
            ("counter", Box::new(|e| e.counter += 1)),
            ("prev_hash", Box::new(|e| e.prev_hash[0] ^= 1)),
            (
                "hlc",
                Box::new(|e| e.hlc = HlcStamp::from_u64(e.hlc.as_u64() + 1)),
            ),
            ("kind", Box::new(|e| e.kind = "thread.messagf".into())),
            ("kind_version", Box::new(|e| e.kind_version += 1)),
            (
                "deps",
                Box::new(|e| e.deps = Deps::new(vec![(other_log(2, 1), 6)], &e.log).unwrap()),
            ),
            ("creates_value", Box::new(|e| e.creates = Some(key("m2")))),
            ("creates_none", Box::new(|e| e.creates = None)),
            ("needs_value", Box::new(|e| e.needs[0] = key("a.c"))),
            ("needs_count", Box::new(|e| e.needs.push(key("z")))),
            ("payload", Box::new(|e| e.payload[2] ^= 1)),
        ];
        for (name, mutate) in mutators {
            let mut changed = base.clone();
            mutate(&mut changed);
            assert_ne!(changed.compute_hash(), base.hash, "{name}");
        }
    }

    #[test]
    fn verify_ok_hash_mismatch_bad_signature() {
        let id = identity(1);
        let sealed = full(&id).seal(&id);
        assert_eq!(sealed.verify(&id.public_key()), Ok(()));
        let mut flipped = sealed.clone();
        flipped.payload[3] ^= 1;
        assert_eq!(
            flipped.verify(&id.public_key()),
            Err(EnvelopeError::HashMismatch)
        );
        assert_eq!(
            sealed.verify(&identity(2).public_key()),
            Err(EnvelopeError::BadSignature)
        );
        let mut resigned_hash = sealed.clone();
        resigned_hash.signature[0] ^= 1;
        assert_eq!(
            resigned_hash.verify(&id.public_key()),
            Err(EnvelopeError::BadSignature)
        );
    }

    #[test]
    fn genesis_pubkey_uuid_mismatch_and_not_genesis() {
        let id = identity(1);
        let g = genesis(&id).seal(&id);
        assert_eq!(g.validate_shape(), Ok(()));
        assert_eq!(g.genesis_pubkey(), Ok(id.public_key()));
        assert_eq!(g.verify(&g.genesis_pubkey().unwrap()), Ok(()));

        let mut wrong_node = genesis(&id);
        wrong_node.log.node = identity(2).uuid();
        assert_eq!(
            wrong_node.seal(&id).genesis_pubkey(),
            Err(EnvelopeError::UuidMismatch)
        );

        assert_eq!(
            full(&id).seal(&id).genesis_pubkey(),
            Err(EnvelopeError::NotGenesis)
        );
        let mut late = genesis(&id);
        late.counter = 2;
        assert_eq!(
            late.seal(&id).genesis_pubkey(),
            Err(EnvelopeError::NotGenesis)
        );
        let mut v2 = genesis(&id);
        v2.kind_version = 2;
        assert_eq!(
            v2.seal(&id).genesis_pubkey(),
            Err(EnvelopeError::NotGenesis)
        );
        let mut no_key = genesis(&id);
        no_key.payload = b"{}".to_vec();
        assert_eq!(
            no_key.seal(&id).genesis_pubkey(),
            Err(EnvelopeError::Malformed { field: "payload" })
        );
    }

    #[test]
    fn deps_round_trip_and_rejections() {
        let me = other_log(1, 1);
        let entries = vec![
            (other_log(1, 2), 3),
            (other_log(2, 1), 1),
            (other_log(2, 5), u64::MAX),
        ];
        let deps = Deps::new(entries.clone(), &me).unwrap();
        let bytes = deps.encode();
        assert_eq!(bytes.len(), 2 + 3 * 28);
        assert_eq!(Deps::decode(&bytes, &me).unwrap(), deps);
        assert_eq!(
            Deps::decode(&Deps::empty().encode(), &me).unwrap(),
            Deps::empty()
        );

        let bad = Err(EnvelopeError::Malformed { field: "deps" });
        let raw = |list: &[(LogKey, u64)]| {
            let mut out = (list.len() as u16).to_be_bytes().to_vec();
            for (k, c) in list {
                out.extend_from_slice(k.node.as_bytes());
                out.extend_from_slice(&k.incarnation.to_be_bytes());
                out.extend_from_slice(&c.to_be_bytes());
            }
            out
        };
        // unsorted
        assert_eq!(
            Deps::decode(&raw(&[(other_log(2, 1), 1), (other_log(1, 2), 1)]), &me),
            bad
        );
        // duplicate
        assert_eq!(
            Deps::decode(&raw(&[(other_log(2, 1), 1), (other_log(2, 1), 2)]), &me),
            bad
        );
        // zero counter
        assert_eq!(Deps::decode(&raw(&[(other_log(2, 1), 0)]), &me), bad);
        // own log
        assert_eq!(Deps::decode(&raw(&[(me, 4)]), &me), bad);
        // trailing byte, truncated, empty input
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert_eq!(Deps::decode(&trailing, &me), bad);
        assert_eq!(Deps::decode(&bytes[..bytes.len() - 1], &me), bad);
        assert_eq!(Deps::decode(&[], &me), bad);
        // constructor applies the same rules
        assert_eq!(Deps::new(vec![(me, 1)], &me), bad);
        assert_eq!(Deps::new(vec![(other_log(2, 1), 0)], &me), bad);
        assert_eq!(
            Deps::new(vec![(other_log(2, 1), 1), (other_log(1, 2), 1)], &me),
            bad
        );
    }

    fn shape_of(f: impl FnOnce(&mut UnsignedEnvelope)) -> Result<(), EnvelopeError> {
        let id = identity(1);
        let mut e = full(&id);
        f(&mut e);
        e.validate_shape()
    }

    fn bad(field: &'static str) -> Result<(), EnvelopeError> {
        Err(EnvelopeError::Malformed { field })
    }

    #[test]
    fn shape_limits_at_boundaries() {
        assert_eq!(shape_of(|_| {}), Ok(()));

        // kind grammar and length 0/1/64/65
        let kind_of_len = |n: usize| format!("a.{}", "b".repeat(n - 2));
        assert_eq!(shape_of(|e| e.kind = String::new()), bad("kind"));
        assert_eq!(shape_of(|e| e.kind = "a".into()), bad("kind"));
        assert_eq!(shape_of(|e| e.kind = kind_of_len(3)), Ok(()));
        assert_eq!(shape_of(|e| e.kind = kind_of_len(64)), Ok(()));
        assert_eq!(shape_of(|e| e.kind = kind_of_len(65)), bad("kind"));
        for k in [
            "A.b", "a.B", "a..b", "a.b.", ".a.b", "1a.b", "a.1b", "a-b.c", "a.b c",
        ] {
            assert_eq!(shape_of(|e| e.kind = k.into()), bad("kind"), "{k}");
        }
        assert_eq!(shape_of(|e| e.kind = "a_1.b2.c".into()), Ok(()));

        // kind_version 0/1/65535/65536
        assert_eq!(shape_of(|e| e.kind_version = 0), bad("kind_version"));
        assert_eq!(shape_of(|e| e.kind_version = 1), Ok(()));
        assert_eq!(shape_of(|e| e.kind_version = 65535), Ok(()));
        assert_eq!(shape_of(|e| e.kind_version = 65536), bad("kind_version"));

        // needs 256/257, unsorted, duplicate
        let needs = |n: usize| (0..n).map(|i| key(&format!("k{i:04}"))).collect::<Vec<_>>();
        assert_eq!(shape_of(|e| e.needs = needs(256)), Ok(()));
        assert_eq!(shape_of(|e| e.needs = needs(257)), bad("needs"));
        assert_eq!(
            shape_of(|e| e.needs = vec![key("b"), key("a")]),
            bad("needs")
        );
        assert_eq!(
            shape_of(|e| e.needs = vec![key("a"), key("a")]),
            bad("needs")
        );

        // deps 4096/4097 (distinct logs, own excluded)
        let deps = |n: usize| {
            let me = own(&identity(1));
            let list = (0..n)
                .map(|i| {
                    let mut node = [0u8; 16];
                    node[..4].copy_from_slice(&(i as u32).to_be_bytes());
                    node[15] = 7;
                    (
                        LogKey {
                            node: NodeUuid::from_bytes(node),
                            incarnation: 1,
                        },
                        1,
                    )
                })
                .collect();
            Deps::new(list, &me).unwrap()
        };
        assert_eq!(shape_of(|e| e.deps = deps(4096)), Ok(()));
        assert_eq!(shape_of(|e| e.deps = deps(4097)), bad("deps"));
        // own log smuggled into deps by hand
        assert_eq!(shape_of(|e| e.deps = Deps(vec![(e.log, 1)])), bad("deps"));

        // payload 524288/524289, invalid JSON, edge whitespace, empty
        let string_payload = |n: usize| format!("\"{}\"", "x".repeat(n - 2)).into_bytes();
        assert_eq!(
            shape_of(|e| e.payload = string_payload(PAYLOAD_MAX)),
            Ok(())
        );
        assert_eq!(
            shape_of(|e| e.payload = string_payload(PAYLOAD_MAX + 1)),
            bad("payload")
        );
        for p in ["", "{", "{} x", "nul", " {}", "{} ", "{}\n"] {
            assert_eq!(shape_of(|e| e.payload = p.into()), bad("payload"), "{p:?}");
        }
        assert_eq!(shape_of(|e| e.payload = b"[1, 2]".to_vec()), Ok(()));

        // counter 0 / 2 / i64::MAX / +1
        assert_eq!(shape_of(|e| e.counter = 0), bad("counter"));
        assert_eq!(shape_of(|e| e.counter = i64::MAX as u64), Ok(()));
        assert_eq!(
            shape_of(|e| e.counter = i64::MAX as u64 + 1),
            bad("counter")
        );

        // counter 1 iff node.genesis v1
        let id = identity(1);
        assert_eq!(genesis(&id).validate_shape(), Ok(()));
        let mut g = genesis(&id);
        g.kind = "thread.message".into();
        assert_eq!(g.validate_shape(), bad("kind"));
        let mut g = genesis(&id);
        g.kind_version = 2;
        assert_eq!(g.validate_shape(), bad("kind"));
        let mut g = genesis(&id);
        g.prev_hash = [1; 32];
        assert_eq!(g.validate_shape(), bad("prev_hash"));
        assert_eq!(shape_of(|e| e.kind = GENESIS_KIND.into()), bad("kind"));
        let mut g = genesis(&id);
        g.counter = 2;
        assert_eq!(g.validate_shape(), bad("kind"));
    }

    #[test]
    fn wire_round_trip_preserves_payload_bytes() {
        let id = identity(1);
        let mut unsigned = full(&id);
        unsigned.payload =
            b"{ \"z\" : 1,\n\t\"a\":  [ 1 ,2 ],   \"m\":{\"y\":null,\"b\":\"\\u00e9\"} }".to_vec();
        let sealed = unsigned.seal(&id);
        let text = serde_json::to_string(&WireEnvelope::from(&sealed)).unwrap();
        assert!(
            text.contains("{ \\\"z\\\"") || text.contains("{ \"z\" : 1,"),
            "{text}"
        );
        let back: WireEnvelope = serde_json::from_str(&text).unwrap();
        let round = Envelope::try_from(back).unwrap();
        assert_eq!(round, sealed);
        assert_eq!(round.payload, sealed.payload);
        assert_eq!(round.verify(&id.public_key()), Ok(()));

        let g = genesis(&id).seal(&id);
        let text = serde_json::to_string(&WireEnvelope::from(&g)).unwrap();
        assert!(!text.contains("creates"));
        let round =
            Envelope::try_from(serde_json::from_str::<WireEnvelope>(&text).unwrap()).unwrap();
        assert_eq!(round, g);
    }

    #[test]
    fn wire_rejects_bad_fields() {
        let id = identity(1);
        let sealed = full(&id).seal(&id);
        let wire = || serde_json::to_value(WireEnvelope::from(&sealed)).unwrap();
        let try_with = |field: &str, value: serde_json::Value| {
            let mut v = wire();
            v[field] = value;
            Envelope::try_from(serde_json::from_value::<WireEnvelope>(v).unwrap())
        };
        let upper = hex_encode(&sealed.hash).to_uppercase();
        assert_eq!(try_with("hash", upper.into()), Err(malformed("hash")));
        assert_eq!(try_with("sig", "ab".into()), Err(malformed("sig")));
        assert_eq!(try_with("node", "zz".into()), Err(malformed("node")));
        assert_eq!(try_with("prev", "0".into()), Err(malformed("prev")));
        assert_eq!(try_with("deps", "00".into()), Err(malformed("deps")));
        assert_eq!(try_with("creates", "a b".into()), Err(malformed("creates")));
        assert_eq!(
            try_with("needs", serde_json::json!(["b", "a"])),
            Err(malformed("needs"))
        );
        assert_eq!(try_with("ver", 0.into()), Err(malformed("kind_version")));
        assert!(try_with("counter", 7.into()).is_ok());
    }
}
