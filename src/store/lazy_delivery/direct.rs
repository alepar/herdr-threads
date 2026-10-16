use super::*;

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
pub(in crate::store) fn direct_acceptance_floor(
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
    "WHERE r.seat_id=?1 AND m.id=?2 AND m.instance_id=?4 AND m.kind='ordinary' ",
    "AND sm.instance_id=m.instance_id AND sm.decision_seq=m.decision_seq ",
    "AND (r.seat_id=ld0.seat_a OR r.seat_id=ld0.seat_b) AND ",
    binding_lazy_scope_sql! { binding: "b.binding_id", seat: "r.seat_id", thread: "m.thread_id", instance: "m.instance_id", publication: "m.decision_seq" },
    ")"
);

pub(in crate::store) fn direct_lazy_display_admitted(
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
pub(in crate::store) fn displayed_episode_floor(
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
               AND m.kind='ordinary' AND sm.instance_id=m.instance_id \
               AND sm.decision_seq=m.decision_seq AND m.decision_seq>?5)",
            params![thread.as_str(), sequence, instance, seat.as_str(), episode.created_decision, binding.as_str()], |r| r.get(0),
        ).map_err(store_error)?;
        if eligible {
            return episode_floor(db, &instance, thread, &episode).map(Some);
        }
    }
    Ok(None)
}
