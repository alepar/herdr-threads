//! The seam between "who applied this fact" and "what rows it writes".
//!
//! Responsibility
//! - Owns: [`FactRoute`] (the journal-or-local-only choice and the one emit),
//!   [`MaterializeInput`] (the apply instant, the entry's one
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
use crate::protocol::ids::{SeatId, ThreadId, prefix, public_id_parts};
use crate::protocol::results::{ApiError, ErrorCode};
use crate::store::connection::{DecisionInstant, StoreContext, api_error, store_error};
use crate::store::journal::{self, Authored};
use crate::store::{schema, seats};
use crate::sync::EntityKey;
use crate::sync::facts::FactKind;
use crate::sync::facts::payload::ReferencedSeats;
use crate::sync::hlc::HlcStamp;

/// The suppression query: whether the settlement named by the key already
/// exists, so a delivery for it must be skipped.
pub type SuppressFn<'a> = dyn Fn(&Transaction<'_>, &EntityKey) -> Result<bool, ApiError> + 'a;

/// The test-support write-scope marker a [`local_only`] input carries so the
/// `WriteGuard` authorizer lets its writes through (`()` outside test-support
/// builds; every other constructor leaves it default).
#[cfg(feature = "test-support")]
pub type WriteScope = Option<crate::test_support::write_guard::ScopeGuard>;
/// See the test-support alias.
#[cfg(not(feature = "test-support"))]
pub type WriteScope = ();

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
    /// The entry's envelope HLC: the stamp a register write carries. For a
    /// [`local_only`] input the decision instant at counter 0.
    pub origin_hlc: HlcStamp,
    /// The entry's counter in its origin log (`0` for a [`local_only`] input,
    /// which has no journal row).
    pub origin_counter: u64,
    /// The `journal` row that carried the entry (`thread_registers`
    /// `src_ingest_seq`); `None` for a [`local_only`] input.
    pub ingest_seq: Option<i64>,
    /// Catch-up suppression for a settlement key (`ApplyContext::delivery_suppressed`).
    pub suppress: &'a SuppressFn<'a>,
    /// Test-support only: held for the input's lifetime by [`local_only`].
    pub scope: WriteScope,
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
        origin_hlc: HlcStamp::from_parts(u64::try_from(decision.utc.0).unwrap_or(0), 0),
        origin_counter: 0,
        ingest_seq: None,
        suppress: &never_suppress,
        #[cfg(feature = "test-support")]
        scope: Some(crate::test_support::write_guard::enter_replicated_write_scope()),
        #[cfg(not(feature = "test-support"))]
        scope: (),
    })
}

/// The kinds the restore-window authoring hold refuses for a local seat
/// (fact-kinds design section 2; root section 7). Each is a one-shot fact on a
/// same-key entity, where the node's unpulled old tail may already hold a
/// different fact for the key. [`FactRoute`] owns this list; callers never
/// match kind strings.
pub const HELD_KINDS: &[&str] = &[
    "receipt.settled",
    "membership.joined",
    "membership.left",
    "invitation.answered",
    "seat.retired",
];

/// The `detail` of the refusal [`FactRoute::emit`] returns while the restore-window
/// authoring hold is active.
pub const OWN_HISTORY_HOLD_DETAIL: &str = "own history catching up";

/// Whether `error` is the restore-window authoring hold's refusal, so a display
/// entry point can skip the settlement instead of failing the delivery.
pub fn is_own_history_hold(error: &ApiError) -> bool {
    error.code == ErrorCode::ServiceBusy && error.detail == OWN_HISTORY_HOLD_DETAIL
}

/// The route of a fact in `thread` whose entity `creates` names (the thread
/// id for a send, a membership or a register write, the message id for a
/// settlement): the one place a command decides between the journal and a
/// node-local write. `journal_open` is whether the store's journal is open.
///
/// | thread | journal | creating fact | route |
/// |---|---|---|---|
/// | service-managed (`journaled = 0`) | any | any | `LocalOnly` |
/// | journaled | open | present | `Journal` |
/// | journaled | open | absent | `StoreCorrupt` |
/// | journaled | not open | any | `LocalOnly` in a test or `test-support` build (a fixture store that never opened the journal creates journaled threads node-locally), else `Unsupported` |
///
/// A clean-start store authors every journaled thread through `thread.created`
/// or `thread.direct` and every send through `message.sent`, and journal rows
/// are never deleted, so a journaled entity without its creating fact is
/// corruption; a node-local write for it would never replicate. The daemon
/// opens the journal before serving, so the last row is unreachable in
/// production.
pub(crate) fn journaled_route(
    tx: &Transaction<'_>,
    journal_open: bool,
    thread: &ThreadId,
    creates: &str,
) -> Result<FactRoute, ApiError> {
    let route = FactRoute::for_thread(tx, thread)?;
    if route == FactRoute::LocalOnly {
        return Ok(route);
    }
    if !journal_open {
        #[cfg(any(test, feature = "test-support"))]
        return Ok(FactRoute::LocalOnly);
        #[cfg(not(any(test, feature = "test-support")))]
        return Err(api_error(ErrorCode::Unsupported, "journal not open"));
    }
    let created: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM journal WHERE creates=?1)",
            [creates],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if created {
        Ok(FactRoute::Journal)
    } else {
        Err(api_error(
            ErrorCode::StoreCorrupt,
            format!("journaled entity {creates} has no creating fact"),
        ))
    }
}

/// How a command turns a decided payload into rows: through the journal, or
/// as a node-local write for a service-managed thread.
///
/// Responsibility
/// - Owns: the route choice, made once per command from `threads.journaled`
///   ([`FactRoute::for_thread`]); for `Journal`, the restore-window authoring
///   hold (while `SyncState.own_history` is active a [`HELD_KINDS`] kind for a
///   local seat is refused as retryable before anything is written), then the
///   publication of every local seat the payload references
///   (`seats::ensure_published`), then `journal::author` / `author_above`; for
///   `LocalOnly`, the refusal of any remote seat (`Unsupported "service
///   threads are node-local"`) and the call of the kind's materializer with
///   [`local_only`].
/// - Hides: the managed-thread exception, so no converted command branches on
///   `journaled` or calls `ensure_published`.
/// - Must not: skip the journal for a journaled thread, publish a seat on the
///   local-only route, or decide anything the command decides (validation,
///   IDs, audience). Authoring errors pass through unchanged (including a
///   parked dependency `Conflict`).
/// - Role: commands call [`FactRoute::emit`] in their fixed per-command order.
///   A new kind needs only its `ReferencedSeats` impl and its materializer; new
///   routing behaviour goes here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FactRoute {
    Journal,
    LocalOnly,
}

impl FactRoute {
    /// The route of a thread whose `journaled` flag is already known.
    pub fn from_journaled(journaled: bool) -> Self {
        if journaled {
            Self::Journal
        } else {
            Self::LocalOnly
        }
    }

    /// The route of `thread`, read from `threads.journaled`; `NotFound` for an
    /// unknown thread.
    pub fn for_thread(tx: &Transaction<'_>, thread: &ThreadId) -> Result<Self, ApiError> {
        let journaled: Option<bool> = tx
            .query_row(
                "SELECT journaled FROM threads WHERE id=?1",
                [thread.as_str()],
                |r| r.get(0),
            )
            .optional()
            .map_err(store_error)?;
        journaled
            .map(Self::from_journaled)
            .ok_or_else(|| api_error(ErrorCode::NotFound, "thread not found"))
    }

    /// Emits fact `K` for `payload`. `floor` is the register stamp a
    /// last-writer-wins write must exceed (`author_above`). `materialize_local`
    /// is the kind's materializer, called only on the local-only route.
    /// Returns the authored entry (`None` on the local-only route).
    pub fn emit<K: FactKind>(
        self,
        tx: &Transaction<'_>,
        ctx: &StoreContext,
        decision: DecisionInstant,
        payload: &K::Payload,
        floor: Option<HlcStamp>,
        materialize_local: impl FnOnce(
            &Transaction<'_>,
            &MaterializeInput<'_>,
            &K::Payload,
        ) -> Result<(), ApiError>,
    ) -> Result<Option<Authored>, ApiError>
    where
        K::Payload: ReferencedSeats,
    {
        let seats = payload.referenced_seats();
        match self {
            Self::Journal => {
                if HELD_KINDS.contains(&K::NAME)
                    && ctx.sync()?.own_history_hold().active(decision.monotonic)
                {
                    for seat in &seats {
                        if seats::is_local_seat(tx, seat)? {
                            return Err(api_error(ErrorCode::ServiceBusy, OWN_HISTORY_HOLD_DETAIL));
                        }
                    }
                }
                for seat in &seats {
                    seats::ensure_published(tx, ctx, decision, seat)?;
                }
                let authored = match floor {
                    Some(floor) => journal::author_above::<K>(tx, ctx, decision, payload, floor)?,
                    None => journal::author::<K>(tx, ctx, decision, payload)?,
                };
                Ok(Some(authored))
            }
            Self::LocalOnly => {
                for seat in &seats {
                    let remote: bool = tx
                        .query_row(
                            "SELECT EXISTS(SELECT 1 FROM seats WHERE id=?1 AND is_local=0)",
                            [seat.as_str()],
                            |r| r.get(0),
                        )
                        .map_err(store_error)?;
                    if remote {
                        return Err(api_error(
                            ErrorCode::Unsupported,
                            "service threads are node-local",
                        ));
                    }
                }
                let input = local_only(tx, decision)?;
                materialize_local(tx, &input, payload)?;
                Ok(None)
            }
        }
    }
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

/// Whether `seat`'s replicated home node is `origin`; an unknown seat has
/// none. The `AuthorRule::SeatHome` test, also used for a register's actor.
pub(crate) fn is_home(
    tx: &Transaction<'_>,
    seat: &SeatId,
    origin: NodeUuid,
) -> Result<bool, ApiError> {
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
