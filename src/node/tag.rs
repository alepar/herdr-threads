//! Node names and the retag rule. A change to tag derivation or to the retag
//! rule goes here; callers never hash keys or compare uuid strings themselves.
use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

use crate::protocol::ids::base62_digits;

/// A node's stable public name: `SHA-256(public key)[0..16]`.
///
/// Owns: the 32-lowercase-hex form (Display/FromStr/serde) and the byte `Ord`
/// that defines "higher uuid" everywhere. Hides: the hash over the public key.
/// Must not: be built from or printed as a hyphenated `uuid::Uuid`, read the
/// store or keep state. Role: leaf value type; `NodeIdentity`, the schema
/// seed, `node_directory` and the descriptor depend on it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeUuid([u8; 16]);

impl NodeUuid {
    pub fn from_public_key(public: &[u8; 32]) -> Self {
        let digest = Sha256::digest(public);
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(&digest[..16]);
        Self(bytes)
    }

    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl fmt::Display for NodeUuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for NodeUuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// A text form that is not a canonical node uuid or tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseNodeNameError(&'static str);

impl fmt::Display for ParseNodeNameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for ParseNodeNameError {}

impl FromStr for NodeUuid {
    type Err = ParseNodeNameError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let raw = text.as_bytes();
        if raw.len() != 32 || !raw.iter().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(ParseNodeNameError(
                "node uuid must be 32 lowercase hex characters",
            ));
        }
        let nibble = |b: u8| if b <= b'9' { b - b'0' } else { b - b'a' + 10 };
        let mut bytes = [0u8; 16];
        for (slot, pair) in bytes.iter_mut().zip(raw.chunks_exact(2)) {
            *slot = (nibble(pair[0]) << 4) | nibble(pair[1]);
        }
        Ok(Self(bytes))
    }
}

impl Serialize for NodeUuid {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for NodeUuid {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// A node's short display name: three base62 characters derived from the
/// uuid and the retag counter `k` ([`tag_for`]).
///
/// Owns: the 3-character base62 form (Display/FromStr/serde). Hides: how the
/// characters are derived. Must not: be unique by itself (collisions are
/// resolved by [`on_collision`]) or keep state. Role: leaf value type, the
/// prefix of public ids minted on this node.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeTag([u8; 3]);

impl NodeTag {
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0).expect("base62 is ASCII")
    }
}

impl fmt::Display for NodeTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Debug for NodeTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl FromStr for NodeTag {
    type Err = ParseNodeNameError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let raw = text.as_bytes();
        if raw.len() != 3 || !raw.iter().all(u8::is_ascii_alphanumeric) {
            return Err(ParseNodeNameError("node tag must be 3 base62 characters"));
        }
        Ok(Self([raw[0], raw[1], raw[2]]))
    }
}

impl Serialize for NodeTag {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for NodeTag {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// The tag of `uuid` at retag counter `k`: `SHA-256(uuid16 || k as u32 BE)`,
/// first 16 bytes as a big-endian u128, three base62 digits (first emitted
/// first).
pub fn tag_for(uuid: NodeUuid, k: u32) -> NodeTag {
    let mut hasher = Sha256::new();
    hasher.update(uuid.as_bytes());
    hasher.update(k.to_be_bytes());
    let digest = hasher.finalize();
    let mut head = [0u8; 16];
    head.copy_from_slice(&digest[..16]);
    let digits = base62_digits(u128::from_be_bytes(head), 3);
    digits.parse().expect("three base62 digits")
}

/// What a node does on learning another node's tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetagDecision {
    Keep,
    MoveTo { k: u32, tag: NodeTag },
}

/// The retag rule: only the node with the higher uuid moves, and only when
/// the other node's (different) uuid carries the tag this node currently has.
/// A `self_k` at `u32::MAX` keeps (overflow is unreachable in practice).
pub fn on_collision(
    self_uuid: NodeUuid,
    self_k: u32,
    other_uuid: NodeUuid,
    other_tag: NodeTag,
) -> RetagDecision {
    if other_uuid == self_uuid || self_uuid <= other_uuid || other_tag != tag_for(self_uuid, self_k)
    {
        return RetagDecision::Keep;
    }
    match self_k.checked_add(1) {
        Some(k) => RetagDecision::MoveTo {
            k,
            tag: tag_for(self_uuid, k),
        },
        None => RetagDecision::Keep,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uuids() -> [NodeUuid; 3] {
        let mut seq = [0u8; 16];
        for (i, b) in seq.iter_mut().enumerate() {
            *b = i as u8;
        }
        [
            NodeUuid::from_bytes([0; 16]),
            NodeUuid::from_bytes([0xff; 16]),
            NodeUuid::from_bytes(seq),
        ]
    }

    #[test]
    fn node_uuid_hex_round_trip_and_rejects_bad_input() {
        let id = uuids()[2];
        let text = id.to_string();
        assert_eq!(text, "000102030405060708090a0b0c0d0e0f");
        assert_eq!(text.parse::<NodeUuid>().unwrap(), id);
        assert_eq!(format!("{id:?}"), text);
        for bad in [
            "000102030405060708090a0b0c0d0e0",
            "000102030405060708090a0b0c0d0e0f0",
            "000102030405060708090A0B0C0D0E0F",
            "00010203-0405-0607-0809-0a0b0c0d0e0f",
            "000102030405060708090a0b0c0d0e0g",
            "",
        ] {
            assert!(bad.parse::<NodeUuid>().is_err(), "{bad}");
        }
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{text}\""));
        assert_eq!(serde_json::from_str::<NodeUuid>(&json).unwrap(), id);
        assert!(serde_json::from_str::<NodeUuid>("\"ABC\"").is_err());
    }

    #[test]
    fn node_uuid_order_matches_hex_order() {
        let all = uuids();
        let mut extra = [0u8; 16];
        extra[15] = 1;
        let all = [all[0], all[1], all[2], NodeUuid::from_bytes(extra)];
        for a in all {
            for b in all {
                assert_eq!(a.cmp(&b), a.to_string().cmp(&b.to_string()));
            }
        }
    }

    #[test]
    fn node_uuid_is_sha256_prefix_of_public_key() {
        let key = [9u8; 32];
        let digest = Sha256::digest(key);
        assert_eq!(NodeUuid::from_public_key(&key).as_bytes(), &digest[..16]);
    }

    /// SHA-256 and base62 recomputed without `tag_for`/`base62_digits`.
    fn independent_tag(uuid: NodeUuid, k: u32) -> String {
        let mut input = uuid.as_bytes().to_vec();
        input.extend_from_slice(&k.to_be_bytes());
        let digest = Sha256::digest(&input);
        let mut n = u128::from_be_bytes(digest[..16].try_into().unwrap());
        let alphabet = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
        let mut out = String::new();
        for _ in 0..3 {
            out.push(alphabet.as_bytes()[(n % 62) as usize] as char);
            n /= 62;
        }
        out
    }

    #[test]
    fn tag_for_golden_vectors() {
        let [zero, ff, seq] = uuids();
        let golden = [
            (zero, 0, "xIl"),
            (zero, 1, "f8Z"),
            (ff, 0, "vX9"),
            (ff, 1, "w5Z"),
            (seq, 0, "4t5"),
            (seq, 1, "MdY"),
        ];
        for (uuid, k, want) in golden {
            assert_eq!(tag_for(uuid, k).as_str(), want, "{uuid} k={k}");
        }
        for (uuid, k, want) in golden {
            assert_eq!(independent_tag(uuid, k), want);
        }
    }

    #[test]
    fn tag_for_is_three_base62_chars() {
        for uuid in uuids() {
            for k in 0..200 {
                let tag = tag_for(uuid, k);
                assert_eq!(tag.as_str().len(), 3);
                assert!(tag.as_str().bytes().all(|b| b.is_ascii_alphanumeric()));
                assert_eq!(tag.as_str().parse::<NodeTag>().unwrap(), tag);
            }
        }
    }

    #[test]
    fn node_tag_parse_and_serde() {
        assert!("ab".parse::<NodeTag>().is_err());
        assert!("abcd".parse::<NodeTag>().is_err());
        assert!("a-c".parse::<NodeTag>().is_err());
        let tag: NodeTag = "aZ9".parse().unwrap();
        let json = serde_json::to_string(&tag).unwrap();
        assert_eq!(json, "\"aZ9\"");
        assert_eq!(serde_json::from_str::<NodeTag>(&json).unwrap(), tag);
    }

    #[test]
    fn on_collision_rules() {
        let low = NodeUuid::from_bytes([1; 16]);
        let high = NodeUuid::from_bytes([2; 16]);
        for k in [0, 3] {
            let high_tag = tag_for(high, k);
            // Lower uuid never moves.
            assert_eq!(
                on_collision(low, k, high, tag_for(low, k)),
                RetagDecision::Keep
            );
            // Equal uuid keeps.
            assert_eq!(on_collision(high, k, high, high_tag), RetagDecision::Keep);
            // Different tag keeps.
            let other: NodeTag = if tag_for(low, k).as_str() == "000" {
                "111".parse().unwrap()
            } else {
                "000".parse().unwrap()
            };
            assert_eq!(on_collision(high, k, low, other), RetagDecision::Keep);
            // Higher uuid with the same tag moves.
            assert_eq!(
                on_collision(high, k, low, high_tag),
                RetagDecision::MoveTo {
                    k: k + 1,
                    tag: tag_for(high, k + 1)
                }
            );
        }
        assert_eq!(
            on_collision(high, u32::MAX, low, tag_for(high, u32::MAX)),
            RetagDecision::Keep
        );
    }
}
