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

/// Modern unread plus the unchanged top-level legacy-pending branch. The modern branch excludes
/// the rows this binding displayed out of order above its cursor (`binding_row_displays`, written
/// by `read_cursors::advance_through_displayed_with_change`), so every binding-level unread or
/// pending predicate, and `seat_unread_sql!` through it, inherits the exclusion (two point lookups:
/// the message by (thread, sequence), then its mark by primary key; `CROSS JOIN` fixes that order). The named
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
            " AND NOT EXISTS(SELECT 1 FROM messages rdm CROSS JOIN binding_row_displays rd ON rd.message_id=rdm.id WHERE rd.binding_id=",
            $binding,
            " AND rdm.thread_id=",
            $thread,
            " AND rdm.sequence=",
            $sequence,
            ")) OR (",
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
/// finds it unread (`binding_lazy_unread_sql!`). The seat inbox filter, `attention::pending_lazy` and the mod
/// stall tail (`STALL_ROWS_TAIL`) use it as a post-filter; the ordered seat-level seeks and the wake seeks read each connected top-level
/// binding's pending view (`binding_pending`) plus the legacy rows and apply it to each
/// candidate. Display marking uses `seat_owes_display_sql!` instead.
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

/// A lazy recipient row `r` (with its message `m`) is still owed a display at seat level when it is
/// legacy pending or some open top-level binding of the seat that can still read it finds it
/// unread: a connected one, or one disconnected because its process exited (`liveness_process_exit`,
/// which reconnects on its next call). A binding disconnected as superseded, by an end event,
/// retirement, unresolve, replacement or migration owes nothing, so it neither keeps the seat-wide
/// `state` `pending` nor (through `seat_unread_sql!`, which counts connected bindings only) holds the
/// wake frontier open. Decides only the seat-wide `state` (`MARK_DISPLAYED_SQL`), which no delivery or wake decision reads.
macro_rules! seat_owes_display_sql {
    () => {
        concat!(
            "(r.legacy_pending=1 OR EXISTS(SELECT 1 FROM occupant_bindings b WHERE b.seat_id=r.seat_id AND b.role='top_level' AND ",
            "(b.ended_at IS NULL AND (b.liveness='connected' OR b.disconnect_reason='liveness_process_exit'))",
            " AND ",
            $crate::store::lazy_delivery::binding_lazy_unread_sql!(),
            "))"
        )
    };
}

/// A binding (alias `$b`) that holds a pending view (`binding_pending`): open and connected, any role (a
/// subagent reads its own stream). Any disconnect, process exit included, drops the view
/// (`bindings::disconnect_with_hooks`) and the connected transition re-seeds it (`bindings::connect`), so a
/// disconnected binding costs only its cursors and display marks. The v47 triggers `binding_pending_publish`
/// and `binding_pending_legacy_set` spell the same predicate for alias `b` (pinned by
/// `the_view_triggers_spell_the_held_predicate`); `binding_pending_publish` also spells the reader scope
/// (`binding_lazy_scope_sql!`), so no view row is written that the binding can never read.
///
/// A subagent's view is held on purpose (main review b6ad43fe M12, ht-yha.67): its own inbox reads it
/// (`seek_pending` walks the reading binding's own view for `RowFilter::Binding` / `ThreadBinding`, which
/// `inbox_v2::order` uses for a bound reader), its rows leave as its cursor or a display mark passes them,
/// and every disconnect drops it. No wake path reads it, which is correct, since a subagent is never woken.
/// Pinned by `a_subagent_reads_its_own_view`.
macro_rules! view_held_sql {
    ($b:literal) => {
        concat!(
            "(",
            $b,
            ".ended_at IS NULL AND ",
            $b,
            ".liveness='connected')"
        )
    };
}

/// The published rows a held (connected) binding (`view_held_sql!`) finds unread now
/// (`binding_lazy_unread_sql!`): the backfill seed of every binding's pending view (`binding_pending`). The
/// triggers of v47 keep the view after that; a reconnect re-seeds one binding with `SEED_BINDING_VIEW_SQL`.
macro_rules! seed_view_sql {
    ($filter:literal) => {
        concat!(
            "INSERT OR IGNORE INTO binding_pending(binding_id,ordinal,thread_id,priority,decision_seq) SELECT b.binding_id,r.ordinal,r.thread_id,r.priority,r.decision_seq FROM occupant_bindings b JOIN lazy_recipients r ON r.seat_id=b.seat_id JOIN messages m ON m.id=r.message_id WHERE ",
            $filter,
            " AND ",
            view_held_sql!("b"),
            " AND r.decision_seq IS NOT NULL AND ",
            binding_lazy_unread_sql!()
        )
    };
}

/// One binding's seed (`?1` binding, `?2` its seat). The candidates are a superset of every row the unread
/// predicate accepts and are found without walking the seat's delivery history: the messages past each of
/// the binding's cursors, the messages of the seat's pending direct-pair invitations the binding has no
/// cursor in, and the seat's legacy rows; each reaches its delivery row by `UNIQUE(message_id, seat_id)`.
/// The unchanged predicate then decides exactly. A pending pair is the only cursorless thread the reader
/// scope (`binding_lazy_scope_sql!`) admits: an open binding has a cursor in every joined thread (V32
/// backfill, `read_cursors::init_for_new_binding` at enrolment, `read_cursors::init_for_join` and
/// `init_for_direct_accept` at every join), and an invited ordinary or declined thread is never readable, so
/// neither is walked (ht-yha.67, M11). The pair's walk is its history while the invitation is pending:
/// rows the binding reads there, plus the pair's creation events.
pub(crate) const SEED_BINDING_VIEW_SQL: &str = concat!(
    "WITH cand(ordinal) AS (",
    "SELECT lr.ordinal FROM binding_cursors c CROSS JOIN messages cm INDEXED BY messages_thread_sequence ON cm.thread_id=c.thread_id AND cm.sequence>c.seq CROSS JOIN lazy_recipients lr ON lr.message_id=cm.id AND lr.seat_id=?2 WHERE c.binding_id=?1 ",
    "UNION SELECT lr.ordinal FROM memberships mb INDEXED BY memberships_seat_ordinal CROSS JOIN messages mm INDEXED BY messages_thread_sequence ON mm.thread_id=mb.thread_id CROSS JOIN lazy_recipients lr ON lr.message_id=mm.id AND lr.seat_id=?2 WHERE mb.seat_id=?2 AND mb.state='invited' AND COALESCE(mb.voluntary_state,mb.state)='invited' AND EXISTS(SELECT 1 FROM direct_threads nd JOIN invitations ni ON ni.thread_id=nd.thread_id AND ni.seat_id=mb.seat_id AND ni.episode=mb.episode WHERE nd.thread_id=mb.thread_id AND ni.state='pending') AND NOT EXISTS(SELECT 1 FROM binding_cursors nc WHERE nc.binding_id=?1 AND nc.thread_id=mb.thread_id) ",
    "UNION SELECT lg.ordinal FROM lazy_recipients lg INDEXED BY lazy_recipients_legacy WHERE lg.seat_id=?2 AND lg.legacy_pending=1) ",
    "INSERT OR IGNORE INTO binding_pending(binding_id,ordinal,thread_id,priority,decision_seq) SELECT b.binding_id,r.ordinal,r.thread_id,r.priority,r.decision_seq FROM occupant_bindings b JOIN lazy_recipients r ON r.ordinal IN (SELECT ordinal FROM cand) JOIN messages m ON m.id=r.message_id WHERE b.binding_id=?1 AND ",
    view_held_sql!("b"),
    " AND r.decision_seq IS NOT NULL AND ",
    binding_lazy_unread_sql!()
);

/// Seed `binding`'s pending view from its cursors, display marks and (top-level) legacy rows; only a
/// connected binding gets rows (`view_held_sql!`). Called by `read_cursors::seed` (enrolment, re-adoption,
/// reopening) and by `bindings::connect` on every disconnected -> connected transition. Cost: the messages
/// past its cursors, the pending direct pairs it has no cursor in and the seat's legacy rows, never the
/// seat's delivery history (`SEED_BINDING_VIEW_SQL`).
pub fn seed_binding_pending(tx: &Connection, binding: &BindingId) -> Result<(), ApiError> {
    let seat: Option<String> = tx
        .query_row(
            "SELECT seat_id FROM occupant_bindings WHERE binding_id=?1",
            [binding.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    let Some(seat) = seat else {
        return Ok(());
    };
    tx.execute(SEED_BINDING_VIEW_SQL, params![binding.as_str(), seat])
        .map_err(store_error)?;
    Ok(())
}

/// Seed the view of every held binding (the v47 migration's backfill).
pub fn seed_all_binding_pending(conn: &Connection) -> Result<(), ApiError> {
    conn.execute(seed_view_sql!("1"), []).map_err(store_error)?;
    Ok(())
}

/// The seat's connected top-level bindings, the readers every seat-level delivery view consults.
pub(crate) fn connected_top_level(conn: &Connection, seat: &str) -> Result<Vec<String>, ApiError> {
    conn.prepare_cached("SELECT binding_id FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL AND role='top_level' AND liveness='connected' ORDER BY binding_id")
        .map_err(store_error)?
        .query_map([seat], |r| r.get(0))
        .map_err(store_error)?
        .collect::<rusqlite::Result<_>>()
        .map_err(store_error)
}

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
