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

/// Candidates include staged rows (no message yet): publication is tested after the seek.
pub const PENDING_CANDIDATES_SQL: &str = concat!(
    "SELECT r.ordinal,r.preparation_id,r.message_id,r.thread_id,r.seat_id FROM lazy_recipients r INDEXED BY lazy_recipients_seat_ordinal LEFT JOIN messages m ON m.id=r.message_id WHERE r.seat_id=?1 AND (m.id IS NULL OR ",
    seat_unread_sql!(),
    ") AND r.ordinal>?2 AND r.ordinal<=?3 ORDER BY r.ordinal LIMIT ?4"
);
/// The binding-keyed twin of `PENDING_CANDIDATES_SQL` (`?5` is the reading binding).
pub const BINDING_PENDING_CANDIDATES_SQL: &str = concat!(
    "SELECT r.ordinal,r.preparation_id,r.message_id,r.thread_id,r.seat_id FROM lazy_recipients r INDEXED BY lazy_recipients_seat_ordinal LEFT JOIN messages m ON m.id=r.message_id JOIN occupant_bindings b ON b.binding_id=?5 AND b.seat_id=r.seat_id WHERE r.seat_id=?1 AND (m.id IS NULL OR ",
    binding_lazy_unread_sql!(),
    ") AND r.ordinal>?2 AND r.ordinal<=?3 ORDER BY r.ordinal LIMIT ?4"
);
/// `PENDING_CANDIDATES_SQL` restricted to one thread (`?5`; ht-br0 thread-scoped page).
pub const THREAD_PENDING_CANDIDATES_SQL: &str = concat!(
    "SELECT r.ordinal,r.preparation_id,r.message_id,r.thread_id,r.seat_id FROM lazy_recipients r INDEXED BY lazy_recipients_seat_ordinal LEFT JOIN messages m ON m.id=r.message_id WHERE r.seat_id=?1 AND r.thread_id=?5 AND (m.id IS NULL OR ",
    seat_unread_sql!(),
    ") AND r.ordinal>?2 AND r.ordinal<=?3 ORDER BY r.ordinal LIMIT ?4"
);
/// `BINDING_PENDING_CANDIDATES_SQL` restricted to one thread (`?6`; ht-br0 thread-scoped page)
/// and to messages below `?7`, the binding's first ordinary walk stop in that thread
/// (`read_cursors::first_ordinary_walk_stop`): a lazy row above it could be shown but not passed
/// by the display walk, so the next hook or inbox would show it again.
pub const BINDING_THREAD_PENDING_CANDIDATES_SQL: &str = concat!(
    "SELECT r.ordinal,r.preparation_id,r.message_id,r.thread_id,r.seat_id FROM lazy_recipients r INDEXED BY lazy_recipients_seat_ordinal LEFT JOIN messages m ON m.id=r.message_id JOIN occupant_bindings b ON b.binding_id=?5 AND b.seat_id=r.seat_id WHERE r.seat_id=?1 AND r.thread_id=?6 AND (m.id IS NULL OR (m.sequence<?7 AND ",
    binding_lazy_unread_sql!(),
    ")) AND r.ordinal>?2 AND r.ordinal<=?3 ORDER BY r.ordinal LIMIT ?4"
);
pub const ADDRESSED_SQL: &str = "SELECT EXISTS(SELECT 1 FROM lazy_recipients r JOIN send_manifests sm ON sm.preparation_id=r.preparation_id AND sm.message_id=r.message_id WHERE r.seat_id=?1 AND r.message_id=?2 AND sm.instance_id=?3)";
pub const CLEANUP_CANDIDATES_SQL: &str = "SELECT ordinal FROM lazy_recipients INDEXED BY lazy_recipients_preparation_ordinal WHERE preparation_id=?1 AND ordinal>?2 AND ordinal<=?3 ORDER BY ordinal LIMIT ?4";

/// One current invitation point reached through the exact membership, immutable
/// pair and seat. No latest-invitation/history search or lifecycle-time inference.
pub(crate) const ACTIVE_DIRECT_EPISODE_SQL: &str = "SELECT li.id,li.episode,li.created_decision_seq \
     FROM memberships lm JOIN direct_threads ld ON ld.thread_id=lm.thread_id \
     JOIN seats ls ON ls.id=lm.seat_id \
     JOIN invitations li ON li.thread_id=lm.thread_id AND li.seat_id=lm.seat_id AND li.episode=lm.episode \
     WHERE lm.thread_id=?1 AND lm.seat_id=?2 AND ld.instance_id=?3 AND ls.instance_id=?3 \
       AND (ls.id=ld.seat_a OR ls.id=ld.seat_b) AND ls.state<>'retired' AND ls.retired_at IS NULL \
       AND lm.state='invited' AND COALESCE(lm.voluntary_state,lm.state)='invited' AND li.state='pending'";

/// Each recognized event key is UNIQUE; three calls are a fixed envelope even
/// with arbitrarily much retained invitation/thread history.
pub(crate) const DIRECT_EPISODE_EVENT_SQL: &str = "SELECT instance_id,thread_id,kind,source_invitation_id,decision_seq,sequence \
     FROM messages WHERE event_key=?1";

struct DirectEpisode {
    invitation: InvitationId,
    created_decision: i64,
}

struct InvitationEvent {
    instance: String,
    thread: String,
    kind: String,
    source_invitation: Option<String>,
    decision: i64,
    sequence: i64,
}

fn direct_episode_error(error: rusqlite::Error) -> ApiError {
    match error {
        rusqlite::Error::InvalidColumnType(..) => {
            api_error(ErrorCode::StoreCorrupt, "invalid direct invitation episode")
        }
        other => store_error(other),
    }
}

fn active_direct_episode(
    db: &Connection,
    instance: &str,
    seat: &SeatId,
    thread: &ThreadId,
) -> Result<Option<DirectEpisode>, ApiError> {
    let episode: Option<(String, i64, i64)> = db
        .query_row(
            ACTIVE_DIRECT_EPISODE_SQL,
            params![thread.as_str(), seat.as_str(), instance],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(direct_episode_error)?;
    episode
        .map(|(invitation, episode, created_decision)| {
            if episode <= 0 || created_decision <= 0 {
                return Err(api_error(
                    ErrorCode::StoreCorrupt,
                    "invalid direct invitation episode",
                ));
            }
            Ok(DirectEpisode {
                invitation: InvitationId::new(invitation),
                created_decision,
            })
        })
        .transpose()
}

fn episode_floor(
    db: &Connection,
    instance: &str,
    thread: &ThreadId,
    episode: &DirectEpisode,
) -> Result<u64, ApiError> {
    let invitation = episode.invitation.as_str();
    let mut floor: Option<u64> = None;
    for key in [
        format!("invite:{invitation}"),
        format!("operator_invite:{invitation}"),
        format!("service_invite:{invitation}:ordinary"),
    ] {
        let row = db
            .query_row(DIRECT_EPISODE_EVENT_SQL, [key], |r| {
                Ok(InvitationEvent {
                    instance: r.get(0)?,
                    thread: r.get(1)?,
                    kind: r.get(2)?,
                    source_invitation: r.get(3)?,
                    decision: r.get(4)?,
                    sequence: r.get(5)?,
                })
            })
            .optional()
            .map_err(direct_episode_error)?;
        let Some(event) = row else {
            continue;
        };
        if event.instance != instance
            || event.thread != thread.as_str()
            || event.kind != "info"
            || event.source_invitation.as_deref() != Some(invitation)
            || event.decision <= episode.created_decision
            || event.sequence <= 0
        {
            return Err(api_error(
                ErrorCode::StoreCorrupt,
                "invalid invitation timeline event",
            ));
        }
        let sequence = u64::try_from(event.sequence)
            .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative message sequence"))?;
        floor = Some(floor.map_or(sequence, |current| current.min(sequence)));
    }
    floor.ok_or_else(|| api_error(ErrorCode::StoreCorrupt, "invitation timeline event missing"))
}

/// Capture the exact active pair episode before acceptance changes its state.
/// Generic acceptance keeps the established head-seeding policy.
pub(super) fn direct_acceptance_floor(
    db: &Connection,
    instance: &str,
    seat: &SeatId,
    thread: &ThreadId,
    invitation: &InvitationId,
) -> Result<Option<u64>, ApiError> {
    let pair: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM direct_threads WHERE thread_id=?1)",
            [thread.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if !pair {
        return Ok(None);
    }
    let episode = active_direct_episode(db, instance, seat, thread)?
        .ok_or_else(|| api_error(ErrorCode::StoreCorrupt, "invalid direct invitation episode"))?;
    if episode.invitation != *invitation {
        return Err(api_error(
            ErrorCode::StoreCorrupt,
            "invalid direct invitation episode",
        ));
    }
    episode_floor(db, instance, thread, &episode).map(Some)
}

/// Nonjoined AdvanceCursors admission is per exact published addressed Lazy
/// pair ID. Generic left-thread behavior and ordinary admission are unchanged.
pub(crate) const DIRECT_LAZY_DISPLAY_ADMISSION_SQL: &str = concat!(
    "SELECT EXISTS(SELECT 1 FROM messages m ",
    "JOIN lazy_recipients r ON r.message_id=m.id AND r.thread_id=m.thread_id ",
    "JOIN send_manifests sm ON sm.preparation_id=r.preparation_id AND sm.message_id=m.id AND sm.thread_id=m.thread_id ",
    "JOIN occupant_bindings b ON b.binding_id=?3 AND b.seat_id=r.seat_id ",
    "JOIN direct_threads ld0 ON ld0.thread_id=m.thread_id AND ld0.instance_id=m.instance_id ",
    "WHERE r.seat_id=?1 AND m.id=?2 AND m.instance_id=?4 AND m.kind='ordinary' AND m.delivery_mode='lazy' ",
    "AND sm.instance_id=m.instance_id AND sm.decision_seq=m.decision_seq ",
    "AND (r.seat_id=ld0.seat_a OR r.seat_id=ld0.seat_b) AND ",
    binding_lazy_scope_sql! { binding: "b.binding_id", seat: "r.seat_id", thread: "m.thread_id", instance: "m.instance_id", publication: "m.decision_seq" },
    ")"
);

pub(super) fn direct_lazy_display_admitted(
    db: &Connection,
    instance: &str,
    seat: &SeatId,
    binding: &BindingId,
    message: &MessageId,
) -> Result<bool, ApiError> {
    db.query_row(
        DIRECT_LAZY_DISPLAY_ADMISSION_SQL,
        params![seat.as_str(), message.as_str(), binding.as_str(), instance],
        |r| r.get(0),
    )
    .map_err(store_error)
}

/// Only a cursorless display of currently eligible exact addressed content can
/// use the virtual episode floor. Historical CompleteInboxDelivery remains on
/// its existing zero-start walk when no supplied body qualifies for this rule.
pub(super) fn displayed_episode_floor(
    db: &Connection,
    binding: &BindingId,
    seat: &SeatId,
    thread: &ThreadId,
    displayed: &[u64],
) -> Result<Option<u64>, ApiError> {
    if displayed.is_empty() {
        return Ok(None);
    }
    let instance: String = db
        .query_row(
            "SELECT instance_id FROM seats WHERE id=?1",
            [seat.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    let Some(episode) = active_direct_episode(db, &instance, seat, thread)? else {
        return Ok(None);
    };
    for sequence in displayed {
        let sequence = i64::try_from(*sequence)
            .map_err(|_| api_error(ErrorCode::InvalidRequest, "cursor out of range"))?;
        let eligible: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM messages m \
             JOIN lazy_recipients r ON r.message_id=m.id AND r.thread_id=m.thread_id \
             JOIN send_manifests sm ON sm.preparation_id=r.preparation_id AND sm.message_id=m.id AND sm.thread_id=m.thread_id \
             JOIN occupant_bindings b ON b.binding_id=?6 AND b.seat_id=r.seat_id \
             WHERE m.thread_id=?1 AND m.sequence=?2 AND m.instance_id=?3 AND r.seat_id=?4 \
               AND m.kind='ordinary' AND m.delivery_mode='lazy' AND sm.instance_id=m.instance_id \
               AND sm.decision_seq=m.decision_seq AND m.decision_seq>?5)",
            params![thread.as_str(), sequence, instance, seat.as_str(), episode.created_decision, binding.as_str()], |r| r.get(0),
        ).map_err(store_error)?;
        if eligible {
            return episode_floor(db, &instance, thread, &episode).map(Some);
        }
    }
    Ok(None)
}

/// Exact canonical ID only; foreign-instance IDs are absent, never inferred.
pub fn recorded_mode(
    conn: &Connection,
    instance: &str,
    message: &MessageId,
) -> Result<Option<DeliveryMode>, ApiError> {
    let mode: Option<String> = conn
        .query_row(
            "SELECT delivery_mode FROM messages WHERE instance_id=?1 AND id=?2",
            params![instance, message.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    mode.map(|mode| match mode.as_str() {
        "ordinary" => Ok(DeliveryMode::Ordinary),
        "lazy" => Ok(DeliveryMode::Lazy),
        _ => Err(api_error(
            ErrorCode::StoreCorrupt,
            "invalid recorded delivery mode",
        )),
    })
    .transpose()
}

/// With a `binding`, the high water counts only rows unread for that binding (staged rows still
/// count); without one, the seat form.
pub fn capture_high_water(
    conn: &Connection,
    instance: &str,
    seat: &SeatId,
    binding: Option<&BindingId>,
) -> Result<HighWater, ApiError> {
    let publication_decision = conn
        .query_row(
            "SELECT decision_seq FROM local_node WHERE id=?1",
            [instance],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    let belongs: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM seats WHERE id=?1 AND instance_id=?2)",
            params![seat.as_str(), instance],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if !belongs {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "lazy recipient instance mismatch",
        ));
    }
    let recipient_ordinal = match binding {
        None => conn
            .query_row(
                concat!(
                    "SELECT coalesce(max(r.ordinal),0) FROM lazy_recipients r INDEXED BY lazy_recipients_seat_ordinal LEFT JOIN messages m ON m.id=r.message_id WHERE r.seat_id=?1 AND (m.id IS NULL OR ",
                    seat_unread_sql!(),
                    ")"
                ),
                [seat.as_str()],
                |r| r.get(0),
            )
            .map_err(store_error)?,
        Some(binding) => conn
            .query_row(
                concat!(
                    "SELECT coalesce(max(r.ordinal),0) FROM lazy_recipients r INDEXED BY lazy_recipients_seat_ordinal LEFT JOIN messages m ON m.id=r.message_id JOIN occupant_bindings b ON b.binding_id=?2 AND b.seat_id=r.seat_id WHERE r.seat_id=?1 AND (m.id IS NULL OR ",
                    binding_lazy_unread_sql!(),
                    ")"
                ),
                params![seat.as_str(), binding.as_str()],
                |r| r.get(0),
            )
            .map_err(store_error)?,
    };
    Ok(HighWater {
        recipient_ordinal,
        publication_decision,
    })
}

pub fn publication_eligible(
    conn: &Connection,
    instance: &str,
    recipient: &Recipient,
    publication_decision: i64,
) -> Result<bool, ApiError> {
    conn.query_row("SELECT EXISTS(SELECT 1 FROM send_manifests sm JOIN messages m ON m.id=sm.message_id WHERE sm.preparation_id=?1 AND sm.instance_id=?2 AND sm.message_id=?3 AND sm.thread_id=?4 AND sm.decision_seq<=?5 AND m.delivery_mode='lazy')", params![recipient.preparation_id,instance,recipient.message.as_str(),recipient.thread.as_str(),publication_decision], |r| r.get(0)).map_err(store_error)
}

/// Seek candidates first, then test publication. Excluded rows consume work and
/// advance the cursor, even when a page returns no visible recipient. With a `binding` the
/// candidates are that binding's own unread lazy rows; without one, the seat form.
pub fn pending_page(
    conn: &Connection,
    instance: &str,
    seat: &SeatId,
    binding: Option<&BindingId>,
    after: i64,
    high_water: HighWater,
    work_limit: usize,
) -> Result<PendingPage, ApiError> {
    pending_page_in(
        conn, instance, seat, binding, None, after, high_water, work_limit,
    )
}

/// [`pending_page`], with `thread` restricting the candidates to that one thread (ht-br0
/// thread-scoped page); rows of other threads are neither returned nor counted as work.
#[allow(clippy::too_many_arguments)]
pub fn pending_page_in(
    conn: &Connection,
    instance: &str,
    seat: &SeatId,
    binding: Option<&BindingId>,
    thread: Option<&ThreadId>,
    after: i64,
    high_water: HighWater,
    work_limit: usize,
) -> Result<PendingPage, ApiError> {
    validate_bounds(after, high_water.recipient_ordinal, work_limit)?;
    if high_water.publication_decision < 0 {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid lazy publication bound",
        ));
    }
    let belongs: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM seats WHERE id=?1 AND instance_id=?2)",
            params![seat.as_str(), instance],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if !belongs {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "lazy recipient instance mismatch",
        ));
    }
    let read_row = |r: &rusqlite::Row<'_>| {
        Ok(Recipient {
            ordinal: r.get(0)?,
            preparation_id: r.get(1)?,
            message: MessageId::new(r.get::<_, String>(2)?),
            thread: ThreadId::new(r.get::<_, String>(3)?),
            seat: SeatId::new(r.get::<_, String>(4)?),
        })
    };
    let candidates = match (binding, thread) {
        (None, Some(thread)) => conn
            .prepare(THREAD_PENDING_CANDIDATES_SQL)
            .map_err(store_error)?
            .query_map(
                params![
                    seat.as_str(),
                    after,
                    high_water.recipient_ordinal,
                    work_limit as i64,
                    thread.as_str()
                ],
                read_row,
            )
            .map_err(store_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(store_error)?,
        (Some(binding), Some(thread)) => {
            let stop = read_cursors::first_ordinary_walk_stop(conn, binding, seat, thread)?
                .map_or(Ok(i64::MAX), i64::try_from)
                .map_err(|_| api_error(ErrorCode::StoreCorrupt, "message sequence out of range"))?;
            conn.prepare(BINDING_THREAD_PENDING_CANDIDATES_SQL)
                .map_err(store_error)?
                .query_map(
                    params![
                        seat.as_str(),
                        after,
                        high_water.recipient_ordinal,
                        work_limit as i64,
                        binding.as_str(),
                        thread.as_str(),
                        stop
                    ],
                    read_row,
                )
                .map_err(store_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(store_error)?
        }
        (None, None) => conn
            .prepare(PENDING_CANDIDATES_SQL)
            .map_err(store_error)?
            .query_map(
                params![
                    seat.as_str(),
                    after,
                    high_water.recipient_ordinal,
                    work_limit as i64
                ],
                read_row,
            )
            .map_err(store_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(store_error)?,
        (Some(binding), None) => conn
            .prepare(BINDING_PENDING_CANDIDATES_SQL)
            .map_err(store_error)?
            .query_map(
                params![
                    seat.as_str(),
                    after,
                    high_water.recipient_ordinal,
                    work_limit as i64,
                    binding.as_str()
                ],
                read_row,
            )
            .map_err(store_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(store_error)?,
    };
    let inspected = candidates.len();
    let last_inspected = candidates.last().map_or(after, |r| r.ordinal);
    let mut recipients = Vec::new();
    for recipient in candidates {
        if publication_eligible(conn, instance, &recipient, high_water.publication_decision)? {
            recipients.push(recipient);
        }
    }
    Ok(PendingPage {
        recipients,
        high_water,
        last_inspected,
        inspected,
        has_more: inspected == work_limit && last_inspected < high_water.recipient_ordinal,
    })
}

/// Canonical caller validation and contiguous display proof belong to the
/// completion handler. Updates only published addressed progress, returning true
/// for both pending and already displayed addressed rows (idempotent eligibility).
pub fn complete_addressed(
    tx: &Transaction<'_>,
    instance: &str,
    seat: &SeatId,
    message: &MessageId,
) -> Result<bool, ApiError> {
    let addressed: bool = tx
        .query_row(
            ADDRESSED_SQL,
            params![seat.as_str(), message.as_str(), instance],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if addressed {
        tx.execute("UPDATE lazy_recipients SET state='displayed' WHERE seat_id=?1 AND message_id=?2 AND state='pending'",params![seat.as_str(),message.as_str()]).map_err(store_error)?;
    }
    Ok(addressed)
}

/// Where a lazy row stands for one seat (read-only helper for mod acks).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LazyCompletion {
    /// Published and addressed to the seat; display not yet completed.
    Pending,
    /// Published, addressed, and already completed as displayed.
    Completed,
    /// Not a published row addressed to the seat in this instance.
    NotAddressed,
}

/// With a `binding`, a published addressed row is pending until that binding's cursor passes
/// it (or, for a top-level binding, until its legacy pending flag clears); without one (a
/// pre-binding caller) the seat-wide `state` column decides, as before V34.
pub fn lazy_completion_state(
    conn: &Connection,
    instance: &str,
    seat: &SeatId,
    binding: Option<&BindingId>,
    message: &MessageId,
) -> Result<LazyCompletion, ApiError> {
    let row: Option<(String, i64)> = conn
        .query_row(
            "SELECT r.state,r.ordinal FROM lazy_recipients r JOIN send_manifests sm ON sm.preparation_id=r.preparation_id AND sm.message_id=r.message_id WHERE r.seat_id=?1 AND r.message_id=?2 AND sm.instance_id=?3",
            params![seat.as_str(), message.as_str(), instance],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(store_error)?;
    let Some((state, ordinal)) = row else {
        return Ok(LazyCompletion::NotAddressed);
    };
    let pending = match binding {
        Some(binding) => conn
            .query_row(
                concat!(
                    "SELECT EXISTS(SELECT 1 FROM occupant_bindings b JOIN lazy_recipients r ON r.seat_id=b.seat_id JOIN messages m ON m.id=r.message_id WHERE b.binding_id=?1 AND r.ordinal=?2 AND ",
                    binding_lazy_unread_sql!(),
                    ")"
                ),
                params![binding.as_str(), ordinal],
                |r| r.get::<_, bool>(0),
            )
            .map_err(store_error)?,
        None => state != "displayed",
    };
    Ok(if pending {
        LazyCompletion::Pending
    } else {
        LazyCompletion::Completed
    })
}

/// A published lazy message that is unread for one binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LazyUnread {
    pub ordinal: i64,
    pub message: MessageId,
    pub thread: ThreadId,
    pub sequence: u64,
}

/// A lazy message is unread when its own cursor is short and its established or joined
/// stream permits it, or it is newly published in its current direct invitation episode.
/// The top-level legacy-pending exception remains independent of modern scope.
const BINDING_UNREAD_WHERE: &str = concat!("b.binding_id=?1 AND ", binding_lazy_unread_sql!());
const BINDING_UNREAD_FROM: &str = "FROM occupant_bindings b JOIN lazy_recipients r ON r.seat_id=b.seat_id JOIN send_manifests sm ON sm.preparation_id=r.preparation_id AND sm.message_id=r.message_id JOIN messages m ON m.id=r.message_id AND m.delivery_mode='lazy'";

pub fn binding_lazy_unread(
    db: &Connection,
    binding: &BindingId,
    limit: usize,
) -> Result<Vec<LazyUnread>, ApiError> {
    let mut stmt = db
        .prepare(&format!(
            "SELECT r.ordinal,r.message_id,r.thread_id,m.sequence {BINDING_UNREAD_FROM} WHERE {BINDING_UNREAD_WHERE} ORDER BY r.ordinal LIMIT ?2"
        ))
        .map_err(store_error)?;
    stmt.query_map(params![binding.as_str(), limit as i64], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
        ))
    })
    .map_err(store_error)?
    .map(|row| {
        let (ordinal, message, thread, sequence) = row.map_err(store_error)?;
        Ok(LazyUnread {
            ordinal,
            message: MessageId::new(message),
            thread: ThreadId::new(thread),
            sequence: u64::try_from(sequence)
                .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative message sequence"))?,
        })
    })
    .collect()
}

/// A published lazy message unread by `binding`, with its publication decision sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedLazyUnread {
    pub ordinal: i64,
    pub message: String,
    pub thread: String,
    pub decision_seq: i64,
}

/// The newest published lazy messages unread by one binding, newest ordinal first: one windowed
/// walk of at most `limit` rows. Published means the manifest is published in `instance` up to
/// the host decision sequence `through` (the same bound `pending_lazy` applies per seat).
pub fn binding_lazy_window(
    db: &Connection,
    instance: &str,
    binding: &BindingId,
    through: i64,
    limit: i64,
) -> Result<Vec<PublishedLazyUnread>, ApiError> {
    let mut stmt = db
        .prepare_cached(&format!(
            "SELECT r.ordinal,r.message_id,r.thread_id,sm.decision_seq {BINDING_UNREAD_FROM} WHERE {BINDING_UNREAD_WHERE} AND sm.instance_id=?3 AND sm.decision_seq<=?4 ORDER BY r.ordinal DESC LIMIT ?2"
        ))
        .map_err(store_error)?;
    stmt.query_map(params![binding.as_str(), limit, instance, through], |r| {
        Ok(PublishedLazyUnread {
            ordinal: r.get(0)?,
            message: r.get(1)?,
            thread: r.get(2)?,
            decision_seq: r.get(3)?,
        })
    })
    .map_err(store_error)?
    .collect::<rusqlite::Result<Vec<_>>>()
    .map_err(store_error)
}

pub fn binding_has_unread_lazy(db: &Connection, binding: &BindingId) -> Result<bool, ApiError> {
    db.query_row(
        &format!("SELECT EXISTS(SELECT 1 {BINDING_UNREAD_FROM} WHERE {BINDING_UNREAD_WHERE})"),
        params![binding.as_str()],
        |r| r.get(0),
    )
    .map_err(store_error)
}

/// Seat-level view for seat-keyed consumers (mod fingerprint, kicks, the legacy seat inbox):
/// published lazy messages unread for at least one connected top-level binding of the seat, plus
/// legacy pending rows; a seat with no connected top-level binding counts only legacy rows.
/// Returns `(count, max ordinal)` over the newest `cap` rows.
pub fn seat_lazy_pending(db: &Connection, seat: &SeatId, cap: i64) -> Result<(u64, i64), ApiError> {
    let (count, max): (i64, i64) = db
        .query_row(
            concat!(
                "SELECT count(*),COALESCE(max(ordinal),0) FROM (SELECT r.ordinal FROM lazy_recipients r INDEXED BY lazy_recipients_seat_ordinal JOIN send_manifests sm ON sm.preparation_id=r.preparation_id AND sm.message_id=r.message_id JOIN messages m ON m.id=sm.message_id AND m.delivery_mode='lazy' WHERE r.seat_id=?1 AND ",
                seat_unread_sql!(),
                " ORDER BY r.ordinal DESC LIMIT ?2)"
            ),
            params![seat.as_str(), cap],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(store_error)?;
    Ok((u64::try_from(count).unwrap_or(0), max))
}

/// Displays completed for `binding`: advances the binding's cursor over the displayed messages
/// and, when the binding is top-level, clears the seat's displayed legacy rows. With `None` (a
/// caller that matches no open binding, a pre-binding client) the display counts for the whole
/// seat as before V34: every open top-level binding advances. Also sets `state` to `displayed`,
/// which `read_cursors::init_for_join` reads so a join never re-marks a displayed row. The completion handlers validate the caller and the messages before calling.
pub fn complete_for_binding(
    tx: &Transaction<'_>,
    instance: &str,
    seat: &SeatId,
    binding: Option<&BindingId>,
    messages: &[MessageId],
) -> Result<(), ApiError> {
    complete_displayed_for_binding(tx, instance, seat, binding, messages, &[])
}

/// [`complete_for_binding`] for a display that also showed `shown` ordinary messages whole
/// (settled or already settled by the caller): their sequences join `messages`' in the one
/// cursor walk per thread, so the cursor passes both. A `shown` id with no row is skipped.
pub fn complete_displayed_for_binding(
    tx: &Transaction<'_>,
    instance: &str,
    seat: &SeatId,
    binding: Option<&BindingId>,
    messages: &[MessageId],
    shown: &[MessageId],
) -> Result<(), ApiError> {
    let mut by_thread: std::collections::BTreeMap<String, Vec<u64>> = Default::default();
    let mut displayed = Vec::new();
    for message in messages {
        if !complete_addressed(tx, instance, seat, message)? {
            continue;
        }
        let (thread, sequence): (String, i64) = tx
            .query_row(
                "SELECT thread_id,sequence FROM messages WHERE id=?1",
                [message.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(store_error)?;
        by_thread.entry(thread).or_default().push(
            u64::try_from(sequence)
                .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative message sequence"))?,
        );
        displayed.push(message);
    }
    for message in shown {
        let row: Option<(String, i64)> = tx
            .query_row(
                "SELECT thread_id,sequence FROM messages WHERE id=?1 AND instance_id=?2",
                params![message.as_str(), instance],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(store_error)?;
        let Some((thread, sequence)) = row else {
            continue;
        };
        by_thread.entry(thread).or_default().push(
            u64::try_from(sequence)
                .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative message sequence"))?,
        );
    }
    let targets: Vec<BindingId> = match binding {
        Some(binding) => vec![binding.clone()],
        None => bindings::list_open_top_level(tx, seat)?,
    };
    let clears_legacy = match binding {
        Some(binding) => is_top_level(tx, binding)?,
        None => true,
    };
    if clears_legacy {
        for message in displayed {
            tx.execute(
                "UPDATE lazy_recipients SET legacy_pending=0 WHERE seat_id=?1 AND message_id=?2 AND legacy_pending=1",
                params![seat.as_str(), message.as_str()],
            )
            .map_err(store_error)?;
        }
    }
    for target in &targets {
        let mut reader_changed = false;
        for (thread, sequences) in &by_thread {
            let (_, changed) = read_cursors::advance_through_displayed_with_change(
                tx,
                target,
                &ThreadId::new(thread),
                sequences,
            )?;
            reader_changed |= changed;
        }
        if reader_changed {
            super::presentation_reader::note_binding_reader_change(tx, target)?;
        }
    }
    // `binding_cursors` is WITHOUT ROWID, so SQLite's update hook never reports it. Touch the
    // displayed rows so the commit still kicks the mod observer (`lazy_recipients` is in
    // `MOD_NOTIFY_TABLES`) when a later binding's cursor passes an already-displayed row.
    for message in messages {
        tx.execute(
            "UPDATE lazy_recipients SET legacy_pending=legacy_pending WHERE seat_id=?1 AND message_id=?2",
            params![seat.as_str(), message.as_str()],
        )
        .map_err(store_error)?;
    }
    Ok(())
}

fn is_top_level(db: &Connection, binding: &BindingId) -> Result<bool, ApiError> {
    Ok(bindings::get(db, binding)?.is_some_and(|b| b.role == bindings::BindingRole::TopLevel))
}

/// Bound of the pending-lazy count (a change detector for the mod fingerprint
/// and the wake frontier, not a count anyone presents).
pub const PENDING_SUMMARY_CAP: i64 = 1000;

/// `(count, max ordinal)` of the seat's published pending lazy rows, over the
/// newest `PENDING_SUMMARY_CAP`. Published rows only: a staged row is invisible
/// to the mod and to wake, and its publication (manifest + message) must change
/// both. Shared by `mod_seat_view` and `wake_seat_attention`.
pub fn pending_summary(conn: &Connection, seat: &SeatId) -> Result<(i64, i64), ApiError> {
    // Pending means unread at seat level (`seat_unread_sql!`): post-V34 delivery predicates never
    // read `lazy_recipients.state` (only `init_for_join` does, to skip displayed rows).
    let (count, max) = seat_lazy_pending(conn, seat, PENDING_SUMMARY_CAP)?;
    Ok((i64::try_from(count).unwrap_or(i64::MAX), max))
}

/// The lazy wake frontier of a `wake_on: all` seat: the greatest publication
/// decision sequence (`send_manifests.decision_seq`) of its pending published
/// lazy rows, or `None` when nothing is pending. Publication order only grows,
/// so a row staged early but published late still lies beyond a frontier
/// reserved earlier; the staging ordinal would not. Same window as
/// [`pending_summary`]: the newest `PENDING_SUMMARY_CAP` pending rows by
/// staging ordinal. A late-published row's ordinal is near the top, because
/// staging and publication are seconds apart.
pub fn wake_frontier(conn: &Connection, seat: &str) -> Result<Option<i64>, ApiError> {
    let (count, max_seq): (i64, i64) = conn
        .query_row(
            concat!(
                "SELECT count(*),COALESCE(max(w.decision_seq),0) FROM (SELECT sm.decision_seq FROM lazy_recipients r INDEXED BY lazy_recipients_seat_ordinal JOIN send_manifests sm ON sm.preparation_id=r.preparation_id AND sm.message_id=r.message_id JOIN messages m ON m.id=sm.message_id AND m.delivery_mode='lazy' WHERE r.seat_id=?1 AND ",
                seat_unread_sql!(),
                " ORDER BY r.ordinal DESC LIMIT ?2) w"
            ),
            params![seat, PENDING_SUMMARY_CAP],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(store_error)?;
    Ok((count > 0).then_some(max_seq))
}

/// Publication time of the seat's oldest published pending lazy row; the batch
/// window treats it like an ordinary receipt's publication time.
pub fn oldest_pending_at(conn: &Connection, seat: &str) -> Result<Option<i64>, ApiError> {
    conn.query_row(
        concat!(
            "SELECT m.decision_at FROM lazy_recipients r INDEXED BY lazy_recipients_seat_ordinal JOIN send_manifests sm ON sm.preparation_id=r.preparation_id AND sm.message_id=r.message_id JOIN messages m ON m.id=sm.message_id AND m.delivery_mode='lazy' WHERE r.seat_id=?1 AND ",
                seat_unread_sql!(),
                " ORDER BY r.ordinal LIMIT 1"
        ),
        [seat],
        |r| r.get(0),
    )
    .optional()
    .map_err(store_error)
}

/// Wake discovery probe: the seat is `wake_on: all` and has a lazy recipient row that is
/// staged (no message yet) or unread at seat level, within the newest
/// `PENDING_SUMMARY_CAP` rows. The canonical summary decides whether the row is published.
pub fn has_wake_rows(conn: &Connection, seat: &str) -> Result<bool, ApiError> {
    conn.query_row(
        concat!(
            "SELECT EXISTS(SELECT 1 FROM seats WHERE id=?1 AND wake_on='all') AND EXISTS(SELECT 1 FROM (SELECT r.ordinal FROM lazy_recipients r INDEXED BY lazy_recipients_seat_ordinal LEFT JOIN messages m ON m.id=r.message_id WHERE r.seat_id=?1 AND (m.id IS NULL OR ",
            seat_unread_sql!(),
            ") ORDER BY r.ordinal DESC LIMIT 1000))"
        ),
        [seat],
        |r| r.get(0),
    )
    .map_err(store_error)
}

/// Publication of a lazy send: every `wake_on: all` recipient gets the
/// new-attention write so the dispatcher re-judges it. A `nudge` recipient is
/// untouched. One set-based statement over the preparation's recipients.
pub fn note_published(tx: &Transaction<'_>, preparation: &str) -> Result<(), ApiError> {
    tx.execute(
        "INSERT INTO wake_work(seat_id,reason_bits,attention_version) SELECT r.seat_id,1,1 FROM lazy_recipients r INDEXED BY lazy_recipients_preparation_ordinal JOIN seats s ON s.id=r.seat_id WHERE r.preparation_id=?1 AND s.wake_on='all' AND s.is_local=1 ON CONFLICT(seat_id) DO UPDATE SET reason_bits=reason_bits|1,attention_version=attention_version+1",
        [preparation],
    )
    .map_err(store_error)?;
    Ok(())
}

pub fn stage_recipient(
    tx: &Transaction<'_>,
    preparation: &str,
    message: &MessageId,
    thread: &ThreadId,
    seat: &SeatId,
) -> Result<(), ApiError> {
    tx.execute("INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id) VALUES(?1,?2,?3,?4)",params![preparation,message.as_str(),thread.as_str(),seat.as_str()]).map_err(store_error)?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CleanupPage {
    pub inspected: usize,
    pub deleted: usize,
    pub last_inspected: i64,
    pub has_more: bool,
}

pub fn preparation_high_water(conn: &Connection, preparation: &str) -> Result<i64, ApiError> {
    conn.query_row(
        "SELECT coalesce(max(ordinal),0) FROM lazy_recipients WHERE preparation_id=?1",
        [preparation],
        |r| r.get(0),
    )
    .map_err(store_error)
}

/// Cleanup a selected preparation in bounded units; publication never permits
/// deletion, and each selected candidate is charged before that decision.
pub fn cleanup_unpublished(
    tx: &Transaction<'_>,
    preparation: &str,
    after: i64,
    through: i64,
    work_limit: usize,
) -> Result<CleanupPage, ApiError> {
    validate_bounds(after, through, work_limit)?;
    let mut stmt = tx.prepare(CLEANUP_CANDIDATES_SQL).map_err(store_error)?;
    let candidates = stmt
        .query_map(
            params![preparation, after, through, work_limit as i64],
            |r| r.get::<_, i64>(0),
        )
        .map_err(store_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(store_error)?;
    let inspected = candidates.len();
    let last_inspected = candidates.last().copied().unwrap_or(after);
    let published: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM send_manifests WHERE preparation_id=?1)",
            [preparation],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    let mut deleted = 0;
    if !published {
        for ordinal in candidates {
            deleted += tx
                .execute("DELETE FROM lazy_recipients WHERE ordinal=?1", [ordinal])
                .map_err(store_error)?;
        }
    }
    Ok(CleanupPage {
        inspected,
        deleted,
        last_inspected,
        has_more: inspected == work_limit && last_inspected < through,
    })
}

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
#[path = "../../tests/store/binding_lazy.rs"]
mod binding_lazy_tests;
