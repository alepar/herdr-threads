//! Passive delivery bookkeeping. These primitives do not validate caller claims;
//! completion callers must do so in their canonical deciding transaction (A2).
use super::{
    bindings,
    connection::{api_error, store_error},
    read_cursors,
};
use crate::protocol::{
    commands::DeliveryMode,
    ids::{BindingId, InvitationId, MessageId, SeatId, ThreadId},
    results::{ApiError, ErrorCode},
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

/// Both bounds are frozen in the same read transaction as the traversal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HighWater {
    pub recipient_ordinal: i64,
    pub publication_decision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recipient {
    pub ordinal: i64,
    pub preparation_id: String,
    pub message: MessageId,
    pub thread: ThreadId,
    pub seat: SeatId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingPage {
    pub recipients: Vec<Recipient>,
    pub high_water: HighWater,
    pub last_inspected: i64,
    pub inspected: usize,
    /// A full work page may require an empty final page to prove exhaustion.
    pub has_more: bool,
}

/// Reader scope from validated canonical scalars. A retained own cursor keeps its
/// established stream; only a cursorless unjoined reader needs a current direct
/// invitation episode and a publication after its creation decision. The existing
/// membership point contains the three extra exact pair/seat/invitation points.
/// This fragment does not test sequence-unread or the legacy-pending exception,
/// so captured continuations can reuse scope without reinterpreting their offset.
macro_rules! binding_lazy_scope_sql {
    (binding: $binding:literal, seat: $seat:literal, thread: $thread:literal,
     instance: $instance:literal, publication: $publication:literal $(,)?) => {
        concat!(
            "(EXISTS(SELECT 1 FROM binding_cursors lc WHERE lc.binding_id=", $binding,
            " AND lc.thread_id=", $thread,
            ") OR EXISTS(SELECT 1 FROM memberships lm WHERE lm.thread_id=", $thread,
            " AND lm.seat_id=", $seat,
            " AND (lm.state='joined' OR (lm.state='invited' AND COALESCE(lm.voluntary_state,lm.state)='invited' AND EXISTS(",
            "SELECT 1 FROM direct_threads ld JOIN seats ls ON ls.id=", $seat,
            " JOIN invitations li ON li.thread_id=ld.thread_id AND li.seat_id=ls.id AND li.episode=lm.episode ",
            "WHERE ld.thread_id=", $thread, " AND ld.instance_id=", $instance,
            " AND (ls.id=ld.seat_a OR ls.id=ld.seat_b) AND ls.instance_id=", $instance,
            " AND ls.state<>'retired' AND ls.retired_at IS NULL AND li.state='pending' AND ", $publication,
            ">li.created_decision_seq)))))"
        )
    };
}
pub(crate) use binding_lazy_scope_sql;

/// Modern unread plus the unchanged top-level legacy-pending branch. The named
/// form accepts scalar inputs, with no required message alias; callers validate
/// message/recipient/manifest identity against their canonical message row first.
/// The historical aliases remain available through the zero-argument wrapper.
macro_rules! binding_lazy_unread_sql {
    () => {
        $crate::store::lazy_delivery::binding_lazy_unread_sql! {
            binding: "b.binding_id", seat: "r.seat_id", thread: "r.thread_id",
            instance: "m.instance_id", sequence: "m.sequence", publication: "m.decision_seq",
            legacy: "r.legacy_pending", role: "b.role",
        }
    };
    (binding: $binding:literal, seat: $seat:literal, thread: $thread:literal,
     instance: $instance:literal, sequence: $sequence:literal, publication: $publication:literal,
     legacy: $legacy:literal, role: $role:literal $(,)?) => {
        concat!(
            "((",
            $sequence,
            ">COALESCE((SELECT lc.seq FROM binding_cursors lc WHERE lc.binding_id=",
            $binding,
            " AND lc.thread_id=",
            $thread,
            "),0) AND ",
            $crate::store::lazy_delivery::binding_lazy_scope_sql! {
                binding: $binding, seat: $seat, thread: $thread,
                instance: $instance, publication: $publication,
            },
            ") OR (",
            $legacy,
            "=1 AND ",
            $role,
            "='top_level'))"
        )
    };
}
pub(crate) use binding_lazy_unread_sql;

/// A lazy recipient row `r` (with its message `m`, possibly absent) is unread at seat level when
/// it is legacy pending (carried from before V34) or some connected top-level binding of the seat
/// finds it unread (`binding_lazy_unread_sql!`). Post-V34 delivery predicates never read
/// `lazy_recipients.state`; only `read_cursors::init_for_join` does, to skip displayed rows.
macro_rules! seat_unread_sql {
    () => {
        concat!(
            "(r.legacy_pending=1 OR EXISTS(SELECT 1 FROM occupant_bindings b WHERE b.seat_id=r.seat_id AND b.ended_at IS NULL AND b.role='top_level' AND b.liveness='connected' AND ",
            $crate::store::lazy_delivery::binding_lazy_unread_sql!(),
            "))"
        )
    };
}
pub(crate) use seat_unread_sql;

mod completion;
mod direct;
mod inbox;
mod staging;
mod wake;

pub(in crate::store) use self::direct::*;
pub use self::{completion::*, inbox::*, staging::*, wake::*};

fn validate_bounds(after: i64, through: i64, limit: usize) -> Result<(), ApiError> {
    if after < 0 || through < after || !(1..=100).contains(&limit) {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid lazy work bounds",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../../tests/store/binding_lazy.rs"]
mod binding_lazy_tests;
