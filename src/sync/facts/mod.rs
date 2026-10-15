//! Replicated fact kinds: what each journal entry means and how it applies.
//!
//! A fact kind is one module in this directory plus one `.with::<K>()` entry
//! in [`FactRegistry::builtins`]. Callers never `match` on kind strings; they
//! author through a typed [`FactKind`] and ApplyEngine dispatches through the
//! registry. A registry miss (unknown kind, or a newer major than this node
//! knows) is the park decision, not an error.
//!
//! # Version rules
//!
//! - An additive payload field is a minor change: new fields use
//!   `#[serde(default)]`, and payload structs never use `deny_unknown_fields`.
//! - A breaking change adds a new `MAJOR` handler next to the old one.
//! - Old majors stay registered forever, because the journal is never deleted.
//!
//! # Review rules for every kind
//!
//! 1. Payloads carry fact IDs and origin data only, never local positions.
//! 2. `needs` names only keys created by some fact's `creates`. Entities not
//!    yet converted to facts are left out.
//! 3. Handlers never author through `journal::author` directly; they use
//!    [`ApplyContext::follow_up`].
//! 4. Handlers never recompute audiences or reject facts based on the local
//!    membership view.
//! 5. Handlers never write `wake_work` and the other delivery tables
//!    directly; they use DeliveryTail.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use rusqlite::Transaction;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::protocol::results::{ApiError, ErrorCode};
use crate::sync::EntityKey;

pub use crate::store::apply::{ApplyContext, Provenance};

/// A handler's failure, split by what the engine does next.
///
/// `Rejected` is a fact-level rejection: the entry is recorded `failed` and
/// counted as processed. `Store` is a store error: the savepoint and the
/// quantum abort and are retried. Callers keep the two apart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FactError {
    Rejected(ApiError),
    Store(ApiError),
}

impl FactError {
    /// The one classifier of `ApiError` codes. Store-health codes (`StoreBusy`,
    /// `StoreFull`, `StoreCorrupt`, `IncompatibleSchema`, `Cancelled`,
    /// `DeadlineExceeded`) are `Store`; every other code, `Conflict` included
    /// (a constraint violation on a remote fact), is `Rejected`.
    pub fn from_api(error: ApiError) -> Self {
        match error.code {
            ErrorCode::StoreBusy
            | ErrorCode::StoreFull
            | ErrorCode::StoreCorrupt
            | ErrorCode::IncompatibleSchema
            | ErrorCode::Cancelled
            | ErrorCode::DeadlineExceeded => FactError::Store(error),
            _ => FactError::Rejected(error),
        }
    }
}

/// One replicated fact kind.
///
/// Owns, per kind: the name, the major (`kind_version`), the typed payload,
/// `creates`/`needs` extraction and the apply handler. Hides: the payload
/// schema (callers see only the type) and "unknown kind or newer major means
/// park". Must not: recompute audiences, reject a fact the author allowed from
/// the local membership view, or emit wakes directly. A new kind is one module
/// plus one registry entry; no code outside the registry matches on kind
/// strings.
pub trait FactKind: 'static {
    /// The envelope `kind`, e.g. `"node.retag"`.
    const NAME: &'static str;
    /// The envelope `kind_version`.
    const MAJOR: u32;
    type Payload: Serialize + DeserializeOwned;

    /// The entity this fact establishes, if any.
    fn creates(payload: &Self::Payload) -> Option<EntityKey>;
    /// The entities this fact depends on; sorted and unique by the author.
    fn needs(payload: &Self::Payload) -> Vec<EntityKey>;
    /// Materializes the fact inside the apply transaction.
    fn apply(
        tx: &Transaction<'_>,
        ctx: &mut ApplyContext<'_>,
        payload: Self::Payload,
    ) -> Result<(), FactError>;
}

/// The decode-and-apply function of one kind, over payload bytes.
pub type ErasedApply = fn(&Transaction<'_>, &mut ApplyContext<'_>, &[u8]) -> Result<(), FactError>;

fn erased_apply<K: FactKind>(
    tx: &Transaction<'_>,
    ctx: &mut ApplyContext<'_>,
    bytes: &[u8],
) -> Result<(), FactError> {
    let payload: K::Payload = serde_json::from_slice(bytes).map_err(|e| {
        FactError::Rejected(ApiError::new(
            ErrorCode::InvalidRequest,
            format!("payload decode: {e}"),
        ))
    })?;
    K::apply(tx, ctx, payload)
}

/// The set of kinds this node can apply, keyed by `(kind, major)`.
///
/// Owns: the map from `(kind, major)` to the erased decode-and-apply function.
/// A lookup miss is the park decision (`parked_unknown`). Lifecycle: the
/// builtins live in a `OnceLock` static, immutable, like
/// `harness::registry::builtins()`. [`FactRegistry::builtins`] is the one
/// place kinds are chosen.
pub struct FactRegistry {
    handlers: BTreeMap<(&'static str, u32), ErasedApply>,
}

/// Builds a [`FactRegistry`]; a duplicate `(NAME, MAJOR)` panics.
#[derive(Default)]
pub struct FactRegistryBuilder {
    handlers: BTreeMap<(&'static str, u32), ErasedApply>,
}

impl FactRegistryBuilder {
    /// Registers `K`. Panics when `(K::NAME, K::MAJOR)` is already registered.
    #[must_use]
    pub fn with<K: FactKind>(mut self) -> Self {
        let key = (K::NAME, K::MAJOR);
        let previous = self.handlers.insert(key, erased_apply::<K>);
        assert!(
            previous.is_none(),
            "duplicate fact kind registration: {} v{}",
            K::NAME,
            K::MAJOR
        );
        self
    }

    pub fn build(self) -> FactRegistry {
        FactRegistry {
            handlers: self.handlers,
        }
    }
}

static BUILTINS: OnceLock<FactRegistry> = OnceLock::new();

impl FactRegistry {
    pub fn builder() -> FactRegistryBuilder {
        FactRegistryBuilder::default()
    }

    /// The registry of shipped kinds (empty until kinds land).
    pub fn builtins() -> &'static FactRegistry {
        BUILTINS.get_or_init(|| FactRegistry::builder().build())
    }

    /// A leaked registry holding the builtins plus what `extend` adds, so
    /// tests can add kinds and simulate an upgrade by reopening with more.
    #[cfg(any(test, feature = "test-support"))]
    pub fn builtins_plus(
        extend: impl FnOnce(FactRegistryBuilder) -> FactRegistryBuilder,
    ) -> &'static FactRegistry {
        let seeded = FactRegistryBuilder {
            handlers: Self::builtins().handlers.clone(),
        };
        Box::leak(Box::new(extend(seeded).build()))
    }

    /// The handler for `(kind, major)`; `None` means park.
    pub fn lookup(&self, kind: &str, major: u32) -> Option<ErasedApply> {
        self.handlers
            .iter()
            .find(|((k, m), _)| *k == kind && *m == major)
            .map(|(_, f)| *f)
    }

    pub fn len(&self) -> usize {
        self.handlers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }
}

/// The shipped registry.
pub fn registry() -> &'static FactRegistry {
    FactRegistry::builtins()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::NodeUuid;
    use crate::protocol::time::{MonoInstant, UtcMillis};
    use crate::store::connection::DecisionInstant;
    use crate::sync::LogKey;
    use crate::sync::hlc::HlcStamp;
    use rusqlite::Connection;
    use serde::Deserialize;

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct Ping {
        id: String,
    }

    struct PingKind;
    impl FactKind for PingKind {
        const NAME: &'static str = "test.ping";
        const MAJOR: u32 = 1;
        type Payload = Ping;
        fn creates(p: &Ping) -> Option<EntityKey> {
            EntityKey::new(format!("ping:{}", p.id)).ok()
        }
        fn needs(_: &Ping) -> Vec<EntityKey> {
            Vec::new()
        }
        fn apply(_: &Transaction<'_>, _: &mut ApplyContext<'_>, _: Ping) -> Result<(), FactError> {
            Ok(())
        }
    }

    struct PingKindV2;
    impl FactKind for PingKindV2 {
        const NAME: &'static str = "test.ping";
        const MAJOR: u32 = 2;
        type Payload = Ping;
        fn creates(_: &Ping) -> Option<EntityKey> {
            None
        }
        fn needs(_: &Ping) -> Vec<EntityKey> {
            Vec::new()
        }
        fn apply(_: &Transaction<'_>, _: &mut ApplyContext<'_>, _: Ping) -> Result<(), FactError> {
            Ok(())
        }
    }

    fn context() -> ApplyContext<'static> {
        let node = NodeUuid::from_bytes([7; 16]);
        ApplyContext::new(
            LogKey {
                node,
                incarnation: 1,
            },
            1,
            1,
            HlcStamp::from_parts(1_000, 0),
            DecisionInstant {
                utc: UtcMillis(1_000),
                monotonic: MonoInstant(1),
            },
            1,
            node,
        )
    }

    #[test]
    #[should_panic(expected = "duplicate fact kind registration")]
    fn builder_duplicate_name_major_panics() {
        let _ = FactRegistry::builder()
            .with::<PingKind>()
            .with::<PingKind>()
            .build();
    }

    #[test]
    fn builtins_plus_adds_kind_and_builtins_unchanged() {
        let before = FactRegistry::builtins().len();
        let plus = FactRegistry::builtins_plus(|b| b.with::<PingKind>().with::<PingKindV2>());
        assert_eq!(plus.len(), before + 2);
        assert!(plus.lookup("test.ping", 1).is_some());
        assert!(plus.lookup("test.ping", 2).is_some());
        assert!(plus.lookup("test.ping", 3).is_none());
        assert_eq!(registry().len(), before);
        assert!(registry().lookup("test.ping", 1).is_none());
    }

    #[test]
    fn erased_decode_failure_is_rejected_invalid_request() {
        let plus = FactRegistry::builtins_plus(|b| b.with::<PingKind>());
        let handler = plus.lookup("test.ping", 1).unwrap();
        let mut conn = Connection::open_in_memory().unwrap();
        let tx = conn.transaction().unwrap();
        let mut cx = context();
        let err = handler(&tx, &mut cx, b"{\"id\": 5}").unwrap_err();
        match err {
            FactError::Rejected(e) => {
                assert_eq!(e.code, ErrorCode::InvalidRequest);
                assert!(e.detail.starts_with("payload decode: "), "{}", e.detail);
            }
            other => panic!("expected Rejected, got {other:?}"),
        }
        handler(&tx, &mut cx, b"{\"id\":\"a\"}").unwrap();
    }

    #[test]
    fn from_api_classifies_store_codes() {
        for code in [
            ErrorCode::StoreBusy,
            ErrorCode::StoreFull,
            ErrorCode::StoreCorrupt,
            ErrorCode::IncompatibleSchema,
            ErrorCode::Cancelled,
            ErrorCode::DeadlineExceeded,
        ] {
            assert!(
                matches!(
                    FactError::from_api(ApiError::new(code.clone(), "x")),
                    FactError::Store(_)
                ),
                "{code:?}"
            );
        }
        for code in [ErrorCode::Conflict, ErrorCode::InvalidRequest] {
            assert!(
                matches!(
                    FactError::from_api(ApiError::new(code.clone(), "x")),
                    FactError::Rejected(_)
                ),
                "{code:?}"
            );
        }
    }

    #[test]
    fn follow_up_queue_holds_typed_payloads() {
        let mut cx = context();
        cx.follow_up::<PingKind>(&Ping { id: "a".into() }).unwrap();
        cx.follow_up::<PingKindV2>(&Ping { id: "b".into() })
            .unwrap();
        let queued = cx.follow_ups().to_vec();
        assert_eq!(queued.len(), 2);
        assert_eq!((queued[0].kind, queued[0].major), ("test.ping", 1));
        assert_eq!(queued[1].major, 2);
        let back: Ping = serde_json::from_slice(&queued[1].payload).unwrap();
        assert_eq!(back, Ping { id: "b".into() });
        assert_eq!(cx.take_follow_ups().len(), 2);
        assert!(cx.follow_ups().is_empty());
        assert!(cx.is_local_origin());
        assert_eq!(cx.origin_ms(), 1_000);
        cx.set_self_tag_k(3);
        assert_eq!(cx.self_tag_k(), Some(3));
    }
}
