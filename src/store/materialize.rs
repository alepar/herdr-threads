//! The seam between "who applied this fact" and "what rows it writes".
//!
//! Responsibility
//! - Owns: [`MaterializeInput`] (the apply instant, the entry's one
//!   `decision_seq`, the origin node and wall-clock ms, `local_origin`, the
//!   provenance string and the suppression query), [`local_only`] (the one
//!   other way to build it) and [`require_author`] (the origin-authority
//!   check every kind runs first).
//! - Must not: allocate positions beyond `local_only`'s one
//!   `next_decision_seq`, carry kind data (the payload stays a separate
//!   argument), or read `ApplyContext` (handlers build the input from it in
//!   `sync::facts`; `src/store` never reads the context).
//! - Role: every store materializer takes a `&MaterializeInput`, whether a
//!   `FactKind` handler or a local-only writer called it, so each materializer
//!   exists once. New apply-time inputs are fields here.

use rusqlite::{OptionalExtension, Transaction};

use crate::node::{NodeUuid, tag_for};
use crate::protocol::ids::{SeatId, prefix, public_id_parts};
use crate::protocol::results::{ApiError, ErrorCode};
use crate::store::connection::{DecisionInstant, api_error, store_error};
use crate::store::schema;
use crate::sync::EntityKey;

/// The suppression query: whether the settlement named by the key already
/// exists, so a delivery for it must be skipped.
pub type SuppressFn<'a> = dyn Fn(&Transaction<'_>, &EntityKey) -> Result<bool, ApiError> + 'a;

/// What a materializer needs to know about the apply it runs in.
///
/// Owns: the local apply instant, the entry's one `decision_seq`, the origin
/// node and its wall-clock ms, whether the origin is this node (any
/// incarnation), the provenance string (`local`, `local_journal` or
/// `replicated_from:<uuid>`) and the suppression query. Hides: whether the
/// caller is ApplyEngine or a local-only writer. Must not: allocate positions
/// itself or carry kind data. Lifecycle: a borrowed value for one apply or one
/// local-only write.
pub struct MaterializeInput<'a> {
    /// Local apply instant.
    pub decision: DecisionInstant,
    /// The entry's one local position.
    pub decision_seq: u64,
    pub origin_node: NodeUuid,
    /// The origin's "sent at" in Unix ms (envelope HLC ms).
    pub origin_ms: i64,
    /// The origin node is this node, in any incarnation.
    pub local_origin: bool,
    /// The entry is this node's own earlier history (`ApplyContext::is_own_history`):
    /// an own old incarnation, or a re-issue. `attribution::take` records
    /// `local_journal` for it when nothing was staged.
    pub own_history: bool,
    pub provenance: &'a str,
    /// Catch-up suppression for a settlement key (`ApplyContext::delivery_suppressed`).
    pub suppress: &'a SuppressFn<'a>,
}

fn never_suppress(_: &Transaction<'_>, _: &EntityKey) -> Result<bool, ApiError> {
    Ok(false)
}

/// The input of a local-only write (a service-managed thread, never
/// journaled): one fresh `decision_seq` from `next_decision_seq`, this node as
/// origin, `local_origin = true`, provenance `local`, and a suppression query
/// that is always false.
///
/// Owns: that one allocation. Must not: be used on a journaled thread (those
/// go through `journal::author`, whose apply builds the input from the
/// context) or skip the materializer. Role: the only constructor besides the
/// handler helper in `sync::facts`.
pub fn local_only(
    tx: &Transaction<'_>,
    decision: DecisionInstant,
) -> Result<MaterializeInput<'static>, ApiError> {
    let local: String = tx
        .query_row("SELECT id FROM local_node", [], |r| r.get(0))
        .map_err(store_error)?;
    let origin_node: NodeUuid = local
        .parse()
        .map_err(|_| api_error(ErrorCode::StoreCorrupt, "local_node id is not a node uuid"))?;
    let decision_seq = schema::next_decision_seq(tx, &local)?;
    Ok(MaterializeInput {
        decision,
        decision_seq,
        origin_node,
        origin_ms: decision.utc.0,
        local_origin: true,
        // `local` is never a stored value: with nothing staged this write is
        // recorded as `local_journal` too.
        own_history: true,
        provenance: "local",
        suppress: &never_suppress,
    })
}

/// Who may author a kind (spec section 17; root section 6 "Author authority
/// is not author view"). Closed set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthorRule {
    /// The origin is the seat's home node.
    SeatHome(SeatId),
    /// The origin is the home node of either seat (`thread.direct`).
    EitherSeatHome(SeatId, SeatId),
    /// The seat ID's tag is `tag_for(origin, k)` for some `k` from 0 to the
    /// origin's current `tag_k` (`seat.created`).
    OwnTag(SeatId),
    /// Any origin (`thread.register`, the `node.*` kinds).
    Any,
}

fn subject(rule: &AuthorRule) -> String {
    match rule {
        AuthorRule::SeatHome(s) | AuthorRule::OwnTag(s) => s.as_str().to_owned(),
        AuthorRule::EitherSeatHome(a, b) => format!("{}/{}", a.as_str(), b.as_str()),
        AuthorRule::Any => "-".to_owned(),
    }
}

fn is_home(tx: &Transaction<'_>, seat: &SeatId, origin: NodeUuid) -> Result<bool, ApiError> {
    let home: Option<String> = tx
        .query_row(
            "SELECT home_node FROM seats WHERE id=?1",
            [seat.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    Ok(home.is_some_and(|h| h == origin.to_string()))
}

fn owns_tag(tx: &Transaction<'_>, seat: &SeatId, origin: NodeUuid) -> Result<bool, ApiError> {
    let Some((tag, _)) = public_id_parts(prefix::SEAT, seat.as_str()) else {
        return Ok(false);
    };
    let current: Option<u32> = tx
        .query_row(
            "SELECT tag_k FROM node_directory WHERE node_uuid=?1",
            [origin.to_string()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    Ok(current.is_some_and(|k| (0..=k).any(|k| tag_for(origin, k) == tag)))
}

/// The origin-authority check, called first by each kind's handler. Reads only
/// replicated identity (`seats.home_node`, `node_directory.tag_k`), so every
/// node rejects the same facts. A local-origin input (`local_only`, or an own
/// old incarnation whose facts this node already authorized) passes.
///
/// Returns `InvalidRequest "origin <uuid8> is not the authorized author of
/// <kind> for <id>"` (`FactError::from_api` maps it to `Rejected`), where
/// `<id>` is the rule's seat ID (`a/b` for a pair, `-` for `Any`).
pub fn require_author(
    tx: &Transaction<'_>,
    m: &MaterializeInput<'_>,
    kind: &str,
    rule: &AuthorRule,
) -> Result<(), ApiError> {
    if m.local_origin {
        return Ok(());
    }
    let ok = match rule {
        AuthorRule::Any => true,
        AuthorRule::SeatHome(seat) => is_home(tx, seat, m.origin_node)?,
        AuthorRule::EitherSeatHome(a, b) => {
            is_home(tx, a, m.origin_node)? || is_home(tx, b, m.origin_node)?
        }
        AuthorRule::OwnTag(seat) => owns_tag(tx, seat, m.origin_node)?,
    };
    if ok {
        return Ok(());
    }
    let origin = m.origin_node.to_string();
    Err(api_error(
        ErrorCode::InvalidRequest,
        format!(
            "origin {} is not the authorized author of {kind} for {}",
            &origin[..8],
            subject(rule)
        ),
    ))
}

#[cfg(test)]
#[path = "../../tests/store/materialize.rs"]
mod tests;
