//! `node.genesis` v1: the fact that opens every log.
//!
//! One genesis sits at counter 1 of each `(node, incarnation)` log. It
//! announces the log's public key (the envelope check ties it to the node
//! uuid), the node's tag at that time, the incarnation, the predecessor
//! incarnation it replaces (if any) and a free-text host label. Applying it
//! records the node in the node directory. It creates and needs nothing.
//!
//! A genesis from another node also runs the tag-collision check
//! (`node_retag::collision_check`). The adoption of a predecessor freeze
//! belongs to later work (ht-5fr.6).

use rusqlite::Transaction;
use serde::{Deserialize, Serialize};

use super::node_retag::collision_check;
use super::{ApplyContext, FactError, FactKind};
use crate::node::{NodeTag, tag_for};
use crate::protocol::results::{ApiError, ErrorCode};
use crate::store::connection::store_error;
use crate::store::node_directory::{self, NodeDirectoryRow};
use crate::sync::EntityKey;

/// The log a new incarnation replaces: its incarnation number and, when the
/// old log was frozen (forked), the counter it was frozen at. `None`
/// `frozen_at` means the node rotated away after a restore and the old log
/// was not forked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Predecessor {
    pub incarnation: u32,
    #[serde(default)]
    pub frozen_at: Option<u64>,
}

/// The `node.genesis` v1 payload. New fields are additive with
/// `#[serde(default)]`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenesisPayload {
    /// Lowercase hex of the 32-byte Ed25519 public key.
    pub pubkey: String,
    pub tag: String,
    pub tag_k: u32,
    pub incarnation: u32,
    #[serde(default)]
    pub predecessor: Option<Predecessor>,
    #[serde(default)]
    pub host_label: String,
}

/// The `node.genesis` v1 kind.
pub struct NodeGenesis;

impl FactKind for NodeGenesis {
    const NAME: &'static str = crate::sync::envelope::GENESIS_KIND;
    const MAJOR: u32 = 1;
    type Payload = GenesisPayload;

    fn creates(_: &GenesisPayload) -> Option<EntityKey> {
        None
    }

    fn needs(_: &GenesisPayload) -> Vec<EntityKey> {
        Vec::new()
    }

    fn apply(
        tx: &Transaction<'_>,
        ctx: &mut ApplyContext<'_>,
        payload: GenesisPayload,
    ) -> Result<(), FactError> {
        let reject = |detail: &str| {
            Err(FactError::Rejected(ApiError::new(
                ErrorCode::InvalidRequest,
                format!("node.genesis: {detail}"),
            )))
        };
        if payload.incarnation != ctx.origin.incarnation {
            return reject("incarnation does not match the log");
        }
        let Ok(tag) = payload.tag.parse::<NodeTag>() else {
            return reject("malformed tag");
        };
        if tag != tag_for(ctx.origin.node, payload.tag_k) {
            return reject("tag does not match tag_for(node, tag_k)");
        }
        let hlc = i64::try_from(ctx.hlc.as_u64()).unwrap_or(i64::MAX);
        let known = node_directory::get(tx, ctx.origin.node).map_err(from_sqlite)?;
        let newer = known
            .as_ref()
            .is_none_or(|row| row.tag_hlc.is_none_or(|stored| hlc > stored));
        if newer {
            node_directory::upsert(
                tx,
                &NodeDirectoryRow {
                    node_uuid: ctx.origin.node,
                    tag,
                    tag_k: payload.tag_k,
                    host_label: (!payload.host_label.is_empty()).then_some(payload.host_label),
                    first_seen_at: ctx.decision.utc.0,
                    tag_hlc: Some(hlc),
                },
            )
            .map_err(from_sqlite)?;
            if !ctx.is_local_origin() {
                collision_check(tx, ctx, ctx.origin.node, tag)?;
            }
        }
        Ok(())
    }
}

fn from_sqlite(error: rusqlite::Error) -> FactError {
    FactError::from_api(store_error(error))
}
