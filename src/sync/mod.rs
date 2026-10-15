//! Cross-node synchronisation: the per-origin journal, its envelopes and the
//! clocks that order them.
//!
//! Core and harness-neutral: nothing here names a harness, and nothing in this
//! tree touches the store directly. Store-facing code lives in `src/store/`.

pub mod envelope;
pub mod facts;
pub mod hlc;

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::node::NodeUuid;

pub use envelope::EnvelopeError;

/// The name of one origin log.
///
/// Owns: `LogKey { node, incarnation }` as the only identity of a log,
/// ordered by `(node bytes, incarnation)` (the order of `Deps`). Hides: the
/// local `log_ix` interning; only Journal maps a `LogKey` to a `log_ix`, and
/// it never leaves `src/store/journal.rs` and `src/store/apply.rs`. Must not:
/// carry an "is self" meaning; self-ness comes from the local node row, so an
/// older own incarnation is an ordinary log. Role: leaf value type under
/// Envelope, chain rules, the journal port and the sync coordinator. New
/// per-log replicated state goes in [`LogHead`], not here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LogKey {
    pub node: NodeUuid,
    pub incarnation: u32,
}

/// Whether a log still accepts new entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogState {
    Active,
    Frozen,
}

/// The replication state of one origin log: the contiguous prefix held, the
/// hash of its last entry, and whether the log is closed.
///
/// Owns: the per-log head fields (see [`LogKey`] for the log's identity).
/// Must not: say whether the log is the local node's own. New per-log
/// replicated state is a field here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LogHead {
    /// Highest contiguous counter held; 0 when nothing is held.
    pub counter: u64,
    pub head_hash: [u8; 32],
    pub state: LogState,
    /// Unix ms when the log froze; `None` while active.
    pub frozen_at: Option<i64>,
}

/// A node's full per-log head map.
///
/// Owns: the vector as a value (all logs the node knows, own and foreign).
/// Must not: compute heads; Journal does. Ordered by [`LogKey`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Vector(pub BTreeMap<LogKey, LogHead>);

/// Per-request bounds on a batch of served entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BatchLimits {
    pub entries: usize,
    pub bytes: usize,
}

impl BatchLimits {
    pub const ENTRIES: usize = 256;
    pub const BYTES: usize = 256 * 1024;
}

impl Default for BatchLimits {
    fn default() -> Self {
        Self {
            entries: Self::ENTRIES,
            bytes: Self::BYTES,
        }
    }
}

/// Longest [`EntityKey`], in bytes.
pub const ENTITY_KEY_MAX: usize = 160;

/// The kind-agnostic name of something a fact establishes or depends on.
///
/// Owns: the 1..160-byte `[0-9A-Za-z._:/-]` grammar, its validating
/// constructor, its byte `Ord` (sorted, unique `needs`) and the `/`-joined
/// composite spelling for settlement records (`m<id>/s<id>`, `i<id>/answer`).
/// Hides: nothing beyond the grammar. Must not: be parsed by the engine to
/// learn a kind or an entity type; ApplyEngine and Journal compare keys as
/// opaque strings and never split them. Only the kinds that build composite
/// keys know their parts. Role: produced by fact kinds' `creates`/`needs`,
/// stored in `journal.creates` and `journal.needs`, read by readiness and by
/// catch-up suppression. New entity or settlement keys are new spellings built
/// in their kind, never new engine logic.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntityKey(String);

impl EntityKey {
    /// Validate `text` against the key grammar.
    pub fn new(text: impl Into<String>) -> Result<Self, EnvelopeError> {
        let text = text.into();
        let ok = (1..=ENTITY_KEY_MAX).contains(&text.len())
            && text.bytes().all(|b| {
                b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'/' | b'-')
            });
        if ok {
            Ok(Self(text))
        } else {
            Err(EnvelopeError::Malformed {
                field: "entity_key",
            })
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for EntityKey {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for EntityKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::new(text).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_key_grammar() {
        assert!(EntityKey::new("").is_err());
        assert!(EntityKey::new("a").is_ok());
        assert!(EntityKey::new("a".repeat(160)).is_ok());
        assert!(EntityKey::new("a".repeat(161)).is_err());
        for bad in ["a b", "a#b", "é", "a\n", "a,b"] {
            assert_eq!(
                EntityKey::new(bad),
                Err(EnvelopeError::Malformed {
                    field: "entity_key"
                }),
                "{bad:?}"
            );
        }
        for good in ["m1.x_y:z-/9", "Az09._:/-"] {
            assert!(EntityKey::new(good).is_ok(), "{good}");
        }
        let composite = EntityKey::new("mK3qAbCdEfGh/sK3qAbCdEfGh").unwrap();
        assert_eq!(composite.as_str(), "mK3qAbCdEfGh/sK3qAbCdEfGh");
        assert!(EntityKey::new("a").unwrap() < EntityKey::new("b").unwrap());
        let json = serde_json::to_string(&composite).unwrap();
        assert_eq!(json, "\"mK3qAbCdEfGh/sK3qAbCdEfGh\"");
        assert_eq!(serde_json::from_str::<EntityKey>(&json).unwrap(), composite);
        assert!(serde_json::from_str::<EntityKey>("\"a b\"").is_err());
    }

    #[test]
    fn log_key_order_is_node_then_incarnation() {
        let lo = NodeUuid::from_bytes([1; 16]);
        let hi = NodeUuid::from_bytes([2; 16]);
        let key = |node, incarnation| LogKey { node, incarnation };
        assert!(key(lo, 9) < key(hi, 0));
        assert!(key(lo, 1) < key(lo, 2));
        let mut keys = vec![key(hi, 0), key(lo, 2), key(lo, 1)];
        keys.sort();
        assert_eq!(keys, vec![key(lo, 1), key(lo, 2), key(hi, 0)]);
    }
}
