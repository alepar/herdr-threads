//! The one builder of synthetic unknown-layout (`HTJRNL99`) entries. Tests of
//! ht-j87z.3, .5, .10, .12 and .13 use it and never build their own.
//!
//! A synthetic entry is `HTJRNL99 || <the HTJRNL01 canonical bytes after the
//! magic>`: it honours the cross-generation prefix invariant and carries a
//! decodable tail, so the test-only [`parse_synthetic`] can stand in for a
//! build that has learned the layout.

use crate::node::NodeIdentity;
use crate::sync::envelope::{
    Deps, Envelope, EnvelopeError, OpaqueEntry, OpaqueHeader, UnderstoodLayoutGuard,
    UnsignedEnvelope, mark_layout_understood, verify_opaque,
};
use crate::sync::hlc::HlcStamp;
use crate::sync::{EntityKey, LogKey};
use sha2::{Digest, Sha256};

/// The magic of the synthetic layout; never a shipped one.
pub const SYNTHETIC_MAGIC: [u8; 8] = *b"HTJRNL99";

/// `SYNTHETIC_MAGIC || unsigned.canonical_bytes()[8..]`.
pub fn synthetic_raw(unsigned: &UnsignedEnvelope) -> Vec<u8> {
    let mut raw = SYNTHETIC_MAGIC.to_vec();
    raw.extend_from_slice(&unsigned.canonical_bytes()[8..]);
    raw
}

/// Header and raw bytes signed by `identity` (hash = SHA-256(raw)); unverified,
/// so tests can tamper with either.
pub fn synthetic_parts(
    identity: &NodeIdentity,
    unsigned: &UnsignedEnvelope,
) -> (OpaqueHeader, Vec<u8>) {
    let raw = synthetic_raw(unsigned);
    let hash: [u8; 32] = Sha256::digest(&raw).into();
    let header = OpaqueHeader {
        log: unsigned.log,
        counter: unsigned.counter,
        prev_hash: unsigned.prev_hash,
        hash,
        signature: identity.sign(&hash),
    };
    (header, raw)
}

/// A verified [`OpaqueEntry`] (panics if the inputs break the shape rules).
pub fn synthetic_entry(identity: &NodeIdentity, unsigned: &UnsignedEnvelope) -> OpaqueEntry {
    let (header, raw) = synthetic_parts(identity, unsigned);
    verify_opaque(header, raw, &identity.public_key()).expect("synthetic entry verifies")
}

/// A verified HTJRNL99 entry whose body [`parse_synthetic`] refuses (one
/// trailing byte): a valid prefix and signature over a raw that a build which
/// registered the layout still cannot parse.
pub fn synthetic_malformed_entry(
    identity: &NodeIdentity,
    unsigned: &UnsignedEnvelope,
) -> OpaqueEntry {
    let (header, mut raw) = synthetic_parts(identity, unsigned);
    raw.push(0);
    let hash: [u8; 32] = Sha256::digest(&raw).into();
    let header = OpaqueHeader {
        hash,
        signature: identity.sign(&hash),
        ..header
    };
    verify_opaque(header, raw, &identity.public_key()).expect("malformed entry verifies")
}

/// A non-genesis entry in `identity`'s log `incarnation` at `counter` (>= 2)
/// after `prev_hash`: kind `test.opaque` v1, empty deps and needs, payload
/// `{"n":<counter>}`, fixed HLC.
pub fn synthetic_unsigned(
    identity: &NodeIdentity,
    incarnation: u32,
    counter: u64,
    prev_hash: [u8; 32],
) -> UnsignedEnvelope {
    UnsignedEnvelope {
        log: LogKey {
            node: identity.uuid(),
            incarnation,
        },
        counter,
        prev_hash,
        hlc: HlcStamp::from_parts(1_700_000_000_000, 1),
        kind: "test.opaque".into(),
        kind_version: 1,
        deps: Deps::empty(),
        creates: None,
        needs: vec![],
        payload: format!(r#"{{"n":{counter}}}"#).into_bytes(),
    }
}

/// Cursor over the tail; every read fails as `Malformed` when short.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize, field: &'static str) -> Result<&'a [u8], EnvelopeError> {
        if self.0.len() < n {
            return Err(EnvelopeError::Malformed { field });
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(head)
    }

    fn u16(&mut self, field: &'static str) -> Result<usize, EnvelopeError> {
        let b = self.take(2, field)?;
        Ok(usize::from(u16::from_be_bytes([b[0], b[1]])))
    }

    fn u32(&mut self, field: &'static str) -> Result<u32, EnvelopeError> {
        Ok(u32::from_be_bytes(
            self.take(4, field)?.try_into().expect("4 bytes"),
        ))
    }

    fn u64(&mut self, field: &'static str) -> Result<u64, EnvelopeError> {
        Ok(u64::from_be_bytes(
            self.take(8, field)?.try_into().expect("8 bytes"),
        ))
    }

    fn key(&mut self, field: &'static str) -> Result<EntityKey, EnvelopeError> {
        let len = self.u16(field)?;
        let text = std::str::from_utf8(self.take(len, field)?)
            .map_err(|_| EnvelopeError::Malformed { field })?;
        EntityKey::new(text).map_err(|_| EnvelopeError::Malformed { field })
    }
}

/// The test-only parser for [`SYNTHETIC_MAGIC`]: decodes the HTJRNL01-shaped
/// tail (hlc u64, kind u16+bytes, kind_version u32, deps u32+bytes, creates
/// flag+key, needs u16 + keys, payload u32+bytes; no trailing bytes) and
/// returns an [`Envelope`] carrying the opaque entry's hash and signature.
pub fn parse_synthetic(entry: &OpaqueEntry) -> Result<Envelope, EnvelopeError> {
    if entry.magic() != SYNTHETIC_MAGIC {
        return Err(EnvelopeError::Malformed { field: "magic" });
    }
    let mut r = Reader(&entry.raw()[68..]);
    let hlc = HlcStamp::from_u64(r.u64("hlc")?);
    let kind_len = r.u16("kind")?;
    let kind = String::from_utf8(r.take(kind_len, "kind")?.to_vec())
        .map_err(|_| EnvelopeError::Malformed { field: "kind" })?;
    let kind_version = r.u32("kind_version")?;
    let deps_len = r.u32("deps")? as usize;
    let deps = Deps::decode(r.take(deps_len, "deps")?, &entry.log())?;
    let creates = match r.take(1, "creates")?[0] {
        0 => None,
        1 => Some(r.key("creates")?),
        _ => return Err(EnvelopeError::Malformed { field: "creates" }),
    };
    let needs_len = r.u16("needs")?;
    let needs = (0..needs_len)
        .map(|_| r.key("needs"))
        .collect::<Result<Vec<_>, _>>()?;
    let payload_len = r.u32("payload")? as usize;
    let payload = r.take(payload_len, "payload")?.to_vec();
    if !r.0.is_empty() {
        return Err(EnvelopeError::Malformed { field: "raw" });
    }
    Ok(Envelope {
        log: entry.log(),
        counter: entry.counter(),
        prev_hash: entry.prev_hash(),
        hlc,
        kind,
        kind_version,
        deps,
        creates,
        needs,
        payload,
        hash: entry.hash(),
        signature: entry.signature(),
    })
}

/// Registers [`parse_synthetic`] for [`SYNTHETIC_MAGIC`] until dropped.
pub fn understand_synthetic() -> UnderstoodLayoutGuard {
    mark_layout_understood(SYNTHETIC_MAGIC, parse_synthetic)
}

/// 68 bytes: `HTJRNL01` then zeros. For SQL fixtures that insert a projection
/// row directly; it hashes to nothing and is never served as raw.
pub fn known_raw_stub() -> Vec<u8> {
    let mut raw = vec![0u8; 68];
    raw[..8].copy_from_slice(b"HTJRNL01");
    raw
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::envelope::{MAGIC, understood_layout};

    fn identity(n: u8) -> NodeIdentity {
        NodeIdentity::from_seed([n; 32])
    }

    #[test]
    fn synthetic_round_trip() {
        let id = identity(1);
        let mut unsigned = synthetic_unsigned(&id, 3, 5, [0x11; 32]);
        unsigned.creates = Some(EntityKey::new("m1").unwrap());
        unsigned.needs = vec![
            EntityKey::new("a.b").unwrap(),
            EntityKey::new("t1").unwrap(),
        ];
        let entry = synthetic_entry(&id, &unsigned);
        let parsed = parse_synthetic(&entry).unwrap();
        assert_eq!(parsed.log, unsigned.log);
        assert_eq!(parsed.counter, 5);
        assert_eq!(parsed.prev_hash, [0x11; 32]);
        assert_eq!(parsed.hlc, unsigned.hlc);
        assert_eq!(parsed.kind, unsigned.kind);
        assert_eq!(parsed.kind_version, unsigned.kind_version);
        assert_eq!(parsed.deps, unsigned.deps);
        assert_eq!(parsed.creates, unsigned.creates);
        assert_eq!(parsed.needs, unsigned.needs);
        assert_eq!(parsed.payload, unsigned.payload);
        assert_eq!(parsed.hash, entry.hash());
        assert_eq!(parsed.signature, entry.signature());
        assert_eq!(synthetic_raw(&unsigned)[..8], SYNTHETIC_MAGIC);
        assert_eq!(
            synthetic_raw(&unsigned)[8..],
            unsigned.canonical_bytes()[8..]
        );
    }

    #[test]
    fn parse_synthetic_refuses_other_magic_and_trailing_bytes() {
        let id = identity(1);
        let unsigned = synthetic_unsigned(&id, 1, 2, [0x22; 32]);

        let mut other = synthetic_raw(&unsigned);
        other[..8].copy_from_slice(b"HTJRNL98");
        let hash: [u8; 32] = Sha256::digest(&other).into();
        let header = OpaqueHeader {
            hash,
            signature: id.sign(&hash),
            ..synthetic_parts(&id, &unsigned).0
        };
        let entry = verify_opaque(header, other, &id.public_key()).unwrap();
        assert_eq!(
            parse_synthetic(&entry),
            Err(EnvelopeError::Malformed { field: "magic" })
        );

        let mut trailing = synthetic_raw(&unsigned);
        trailing.push(0);
        let hash: [u8; 32] = Sha256::digest(&trailing).into();
        let header = OpaqueHeader {
            hash,
            signature: id.sign(&hash),
            ..synthetic_parts(&id, &unsigned).0
        };
        let entry = verify_opaque(header, trailing, &id.public_key()).unwrap();
        assert_eq!(
            parse_synthetic(&entry),
            Err(EnvelopeError::Malformed { field: "raw" })
        );
    }

    #[test]
    fn understood_layout_hook() {
        let id = identity(1);
        let entry = synthetic_entry(&id, &synthetic_unsigned(&id, 1, 2, [0x33; 32]));
        assert!(understood_layout(&SYNTHETIC_MAGIC).is_none());
        {
            let _guard = understand_synthetic();
            let parser = understood_layout(&SYNTHETIC_MAGIC).expect("registered");
            assert_eq!(parser(&entry).unwrap().counter, 2);
            assert!(understood_layout(MAGIC).is_none());
        }
        assert!(understood_layout(&SYNTHETIC_MAGIC).is_none());
    }
}
