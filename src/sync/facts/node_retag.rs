//! `node.retag` v1: a node moves to the next tag after a collision.
//!
//! Only the node with the higher uuid ever moves (`node::on_collision`), and
//! it authors the retag as a follow-up of the genesis or retag fact that
//! showed it the collision, so the retag's deps include its trigger. Applying
//! a retag records the node's new tag in the node directory; on the node's own
//! origin it also moves `local_node.(tag, tag_k)`. Both then run the same
//! collision check, so chained collisions converge.
//!
//! The identity cache and `node.json` change only after the commit:
//! [`publish_committed_tag`] reads the committed `local_node.tag_k` and
//! updates them. The apply lane calls it after each pass that reports
//! `self_tag_k`, and daemon start calls it once after the journal opens, which
//! covers a crash between a retag's commit and its `node.json` regeneration.
//!
//! Model-visible effect: IDs minted after a retag carry the new 3-character
//! tag (no length change).

use std::io;
use std::path::Path;

use rusqlite::{Connection, Transaction, params};
use serde::{Deserialize, Serialize};

use super::{ApplyContext, FactError, FactKind};
use crate::node::{NodeIdentity, NodeTag, NodeUuid, RetagDecision, on_collision, tag_for};
use crate::protocol::results::{ApiError, ErrorCode};
use crate::protocol::time::CallBudget;
use crate::store::connection::{StoreContext, store_error};
use crate::store::node_directory::{self, NodeDirectoryRow};
use crate::sync::EntityKey;

/// The `node.retag` v1 payload: the new tag counter and the tag it names.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetagPayload {
    pub k: u32,
    pub tag: NodeTag,
}

/// The `node.retag` v1 kind.
pub struct NodeRetag;

impl FactKind for NodeRetag {
    const NAME: &'static str = "node.retag";
    const MAJOR: u32 = 1;
    type Payload = RetagPayload;

    fn creates(_: &RetagPayload) -> Option<EntityKey> {
        None
    }

    fn needs(_: &RetagPayload) -> Vec<EntityKey> {
        Vec::new()
    }

    fn apply(
        tx: &Transaction<'_>,
        ctx: &mut ApplyContext<'_>,
        payload: RetagPayload,
    ) -> Result<(), FactError> {
        let node = ctx.origin.node;
        if payload.k == 0 || payload.tag != tag_for(node, payload.k) {
            return Err(FactError::Rejected(ApiError::new(
                ErrorCode::InvalidRequest,
                "node.retag: tag does not match tag_for(node, k) for k > 0",
            )));
        }
        let hlc = i64::try_from(ctx.hlc.as_u64()).unwrap_or(i64::MAX);
        let known = node_directory::get(tx, node).map_err(from_sqlite)?;
        let newer = known
            .as_ref()
            .is_none_or(|row| row.tag_hlc.is_none_or(|stored| hlc > stored));
        if newer {
            node_directory::upsert(
                tx,
                &NodeDirectoryRow {
                    node_uuid: node,
                    tag: payload.tag,
                    tag_k: payload.k,
                    host_label: None,
                    first_seen_at: ctx.decision.utc.0,
                    tag_hlc: Some(hlc),
                },
            )
            .map_err(from_sqlite)?;
        }
        if ctx.is_local_origin() {
            let moved = tx
                .execute(
                    "UPDATE local_node SET tag=?1, tag_k=?2 WHERE tag_k<?2",
                    params![payload.tag.to_string(), payload.k],
                )
                .map_err(from_sqlite)?;
            if moved == 0 {
                return Ok(());
            }
            ctx.set_self_tag_k(payload.k);
            // Someone may already carry the tag this node moved to.
            for other in node_directory::colliding(tx, &payload.tag).map_err(from_sqlite)? {
                collision_check(tx, ctx, other, payload.tag)?;
            }
        } else if newer {
            collision_check(tx, ctx, node, payload.tag)?;
        }
        Ok(())
    }
}

/// Asks `on_collision` what this node does on learning that `other` carries
/// `other_tag`; on a move, queues the `node.retag` follow-up. Reads
/// `local_node` for the node's own uuid and current `tag_k`, so a second check
/// in the same pass sees the first one's move and converges.
pub(super) fn collision_check(
    tx: &Transaction<'_>,
    ctx: &mut ApplyContext<'_>,
    other: NodeUuid,
    other_tag: NodeTag,
) -> Result<(), FactError> {
    let (id, tag_k): (String, u32) = tx
        .query_row("SELECT id, tag_k FROM local_node", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .map_err(from_sqlite)?;
    let me: NodeUuid = id.parse().map_err(|_| {
        FactError::Store(ApiError::new(
            ErrorCode::StoreCorrupt,
            "local_node id is not a node uuid",
        ))
    })?;
    if let RetagDecision::MoveTo { k, tag } = on_collision(me, tag_k, other, other_tag) {
        ctx.follow_up::<NodeRetag>(&RetagPayload { k, tag })
            .map_err(|error| {
                FactError::Rejected(ApiError::new(
                    ErrorCode::InvalidRequest,
                    format!("node.retag follow-up: {error}"),
                ))
            })?;
    }
    Ok(())
}

/// The committed `local_node.tag_k`.
fn committed_tag_k(conn: &Connection) -> Result<u32, ApiError> {
    conn.query_row("SELECT tag_k FROM local_node", [], |r| r.get(0))
        .map_err(store_error)
}

/// Brings the identity cache and `node.json` up to the committed tag counter.
///
/// Reads `local_node.tag_k` on a query connection (committed data only); when
/// it is above `identity.tag_k()`, sets the identity's counter and regenerates
/// `node.json`. Returns the counter it raised the identity to, `None` when
/// there was nothing to do. Idempotent. Never call it inside a transaction:
/// the identity must not change for a retag that may still roll back.
pub fn publish_committed_tag(
    store: &StoreContext,
    identity: &NodeIdentity,
    node_json_path: &Path,
    budget: &CallBudget,
) -> io::Result<Option<u32>> {
    let to_io = |error: ApiError| io::Error::other(format!("{:?}: {}", error.code, error.detail));
    let db = store.open_query(budget.clone()).map_err(to_io)?;
    let committed = committed_tag_k(&db).map_err(to_io)?;
    drop(db);
    if committed <= identity.tag_k() {
        return Ok(None);
    }
    identity.set_tag_k(committed);
    crate::node::json::publish_identity(node_json_path, identity)?;
    Ok(Some(committed))
}

fn from_sqlite(error: rusqlite::Error) -> FactError {
    FactError::from_api(store_error(error))
}

#[cfg(test)]
#[path = "../../../tests/store/node_facts.rs"]
mod tests;
