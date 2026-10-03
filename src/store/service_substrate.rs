//! Durable graph service identity and requirement read boundaries.
//! Control transitions are performed by the service handlers, not here.

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use sha2::{Digest, Sha256};

use crate::protocol::{
    ids::{InvitationId, MessageId, RequirementId, SeatId, ServiceAuthorId, ThreadId},
    results::{ApiError, ErrorCode},
    service::{EventAuthor, RequiredMembership, RequirementState, VoluntaryMembershipState},
    time::UtcMillis,
};

use super::connection::{api_error, store_error};

/// The reserved identity is stable across daemon registrations and restarts.
pub fn ensure_reserved_author(
    tx: &Transaction<'_>,
    instance: &str,
    created_at: UtcMillis,
) -> Result<ServiceAuthorId, ApiError> {
    let id = reserved_author_id(instance);
    tx.execute(
        "INSERT INTO service_authors(id, instance_id, created_at) VALUES (?1, ?2, ?3) \
         ON CONFLICT(instance_id) DO NOTHING",
        params![id.as_str(), instance, created_at.0],
    )
    .map_err(store_error)?;
    let stored: String = tx
        .query_row(
            "SELECT id FROM service_authors WHERE instance_id=?1 AND reserved_name='herdr-graph'",
            [instance],
            |row| row.get(0),
        )
        .map_err(store_error)?;
    ServiceAuthorId::parse(stored).map_err(|why| api_error(ErrorCode::StoreCorrupt, why))
}

pub fn reserved_author_id(instance: &str) -> ServiceAuthorId {
    let digest = Sha256::digest(instance.as_bytes());
    ServiceAuthorId::new(format!("graph:{digest:x}"))
}

pub fn managed_owner(
    db: &Connection,
    thread: &ThreadId,
) -> Result<Option<ServiceAuthorId>, ApiError> {
    let value: Option<String> = db
        .query_row(
            "SELECT managed_owner_author_id FROM threads WHERE id=?1",
            [thread.as_str()],
            |row| row.get(0),
        )
        .optional()
        .map_err(store_error)?
        .flatten();
    value
        .map(|id| ServiceAuthorId::parse(id).map_err(|why| api_error(ErrorCode::StoreCorrupt, why)))
        .transpose()
}

pub fn current_requirement(
    db: &Connection,
    thread: &ThreadId,
    seat: &SeatId,
) -> Result<Option<RequiredMembership>, ApiError> {
    let record = db
        .query_row(
            "SELECT r.id, r.revision, r.invitation_id, r.issuer_author_id, r.state, \
                r.accepted_by_seat_id, r.accepted_at \
         FROM requirement_episodes r JOIN seats s ON s.id=r.seat_id \
         WHERE r.thread_id=?1 AND r.seat_id=?2 AND r.state IN ('pending','accepted') AND s.state!='retired'",
            params![thread.as_str(), seat.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                ))
            },
        )
        .optional()
        .map_err(store_error)?;
    let Some((id, revision, invitation, issuer, state, accepted_by, accepted_at)) = record else {
        return Ok(None);
    };
    let state = match state.as_str() {
        "pending" => RequirementState::Pending,
        "accepted" => RequirementState::Accepted,
        _ => {
            return Err(api_error(
                ErrorCode::StoreCorrupt,
                "invalid effective requirement state",
            ));
        }
    };
    Ok(Some(RequiredMembership {
        requirement: RequirementId::parse(id)
            .map_err(|why| api_error(ErrorCode::StoreCorrupt, why))?,
        revision: u64::try_from(revision)
            .map_err(|_| api_error(ErrorCode::StoreCorrupt, "invalid requirement revision"))?,
        invitation: InvitationId::parse(invitation)
            .map_err(|why| api_error(ErrorCode::StoreCorrupt, why))?,
        thread: thread.clone(),
        seat: seat.clone(),
        issuer: ServiceAuthorId::parse(issuer)
            .map_err(|why| api_error(ErrorCode::StoreCorrupt, why))?,
        state,
        accepted_by: accepted_by
            .map(|id| SeatId::parse(id).map_err(|why| api_error(ErrorCode::StoreCorrupt, why)))
            .transpose()?,
        accepted_at: accepted_at.map(UtcMillis),
    }))
}

/// Latest episode, including release and retirement, for status inspection.
pub fn latest_requirement(
    db: &Connection,
    thread: &ThreadId,
    seat: &SeatId,
) -> Result<Option<RequiredMembership>, ApiError> {
    type RowColumns = Option<(
        String,
        i64,
        String,
        String,
        String,
        Option<String>,
        Option<i64>,
    )>;
    let row: RowColumns = db
        .query_row(
            "SELECT id,revision,invitation_id,issuer_author_id,state,accepted_by_seat_id,accepted_at \
             FROM requirement_episodes WHERE thread_id=?1 AND seat_id=?2 ORDER BY ordinal DESC LIMIT 1",
            params![thread.as_str(),seat.as_str()],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?)),
        ).optional().map_err(store_error)?;
    let Some((id, revision, invitation, issuer, state, accepted_by, accepted_at)) = row else {
        return Ok(None);
    };
    let mut state = match state.as_str() {
        "pending" => RequirementState::Pending,
        "accepted" => RequirementState::Accepted,
        "released" => RequirementState::Released,
        "retired" => RequirementState::Retired,
        _ => {
            return Err(api_error(
                ErrorCode::StoreCorrupt,
                "invalid requirement state",
            ));
        }
    };
    let retired: bool = db
        .query_row(
            "SELECT state='retired' FROM seats WHERE id=?1",
            [seat.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if retired
        && matches!(
            state,
            RequirementState::Pending | RequirementState::Accepted
        )
    {
        state = RequirementState::Retired;
    }
    Ok(Some(RequiredMembership {
        requirement: RequirementId::parse(id)
            .map_err(|why| api_error(ErrorCode::StoreCorrupt, why))?,
        revision: u64::try_from(revision)
            .map_err(|_| api_error(ErrorCode::StoreCorrupt, "invalid requirement revision"))?,
        invitation: InvitationId::parse(invitation)
            .map_err(|why| api_error(ErrorCode::StoreCorrupt, why))?,
        thread: thread.clone(),
        seat: seat.clone(),
        issuer: ServiceAuthorId::parse(issuer)
            .map_err(|why| api_error(ErrorCode::StoreCorrupt, why))?,
        state,
        accepted_by: accepted_by
            .map(|id| SeatId::parse(id).map_err(|why| api_error(ErrorCode::StoreCorrupt, why)))
            .transpose()?,
        accepted_at: accepted_at.map(UtcMillis),
    }))
}

/// One projection of independent voluntary state and the live requirement.
/// Internal compatibility rows may remain `invited` after cancellation, but
/// callers must use this view to decide what the participant actually holds.
pub struct MembershipProjection {
    pub voluntary: VoluntaryMembershipState,
    pub native_state: Option<&'static str>,
    pub requirement: Option<RequiredMembership>,
}

pub fn effective_membership(
    db: &Connection,
    thread: &ThreadId,
    seat: &SeatId,
) -> Result<MembershipProjection, ApiError> {
    let raw: Option<(String, Option<String>)> = db
        .query_row(
            "SELECT state,voluntary_state FROM memberships WHERE thread_id=?1 AND seat_id=?2",
            params![thread.as_str(), seat.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(store_error)?;
    let seat_state: Option<String> = db
        .query_row(
            "SELECT state FROM seats WHERE id=?1",
            [seat.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    let voluntary = if seat_state.as_deref() == Some("retired") {
        VoluntaryMembershipState::Retired
    } else {
        match raw
            .as_ref()
            .map(|(state, marker)| marker.as_deref().unwrap_or(state))
            .unwrap_or("absent")
        {
            "absent" => VoluntaryMembershipState::Absent,
            "invited" => VoluntaryMembershipState::Invited,
            "joined" => VoluntaryMembershipState::Joined,
            "left" => VoluntaryMembershipState::Left,
            "retired" => VoluntaryMembershipState::Retired,
            _ => {
                return Err(api_error(
                    ErrorCode::StoreCorrupt,
                    "invalid voluntary membership state",
                ));
            }
        }
    };
    let requirement = current_requirement(db, thread, seat)?;
    let native_state = match voluntary {
        VoluntaryMembershipState::Retired => Some("retired"),
        VoluntaryMembershipState::Joined => Some("joined"),
        _ if requirement
            .as_ref()
            .is_some_and(|r| r.state == RequirementState::Accepted) =>
        {
            Some("joined")
        }
        _ if requirement
            .as_ref()
            .is_some_and(|r| r.state == RequirementState::Pending) =>
        {
            Some("invited")
        }
        VoluntaryMembershipState::Invited => Some("invited"),
        VoluntaryMembershipState::Left => Some("left"),
        VoluntaryMembershipState::Absent => None,
    };
    Ok(MembershipProjection {
        voluntary,
        native_state,
        requirement,
    })
}

pub fn message_author(db: &Connection, message: &MessageId) -> Result<EventAuthor, ApiError> {
    let (kind, seat, service): (Option<String>, Option<String>, Option<String>) = db
        .query_row(
            "SELECT author_kind, actor_seat_id, author_service_id FROM messages WHERE id=?1",
            [message.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(store_error)?;
    match (kind.as_deref(), seat, service) {
        (Some("programmatic"), None, Some(id)) => ServiceAuthorId::parse(id)
            .map(EventAuthor::Programmatic)
            .map_err(|why| api_error(ErrorCode::StoreCorrupt, why)),
        (Some("native") | None, Some(id), None) => SeatId::parse(id)
            .map(EventAuthor::Native)
            .map_err(|why| api_error(ErrorCode::StoreCorrupt, why)),
        (Some("built_in") | None, None, None) => Ok(EventAuthor::BuiltIn),
        _ => Err(api_error(
            ErrorCode::StoreCorrupt,
            "invalid message author attribution",
        )),
    }
}

/// The actor label every service-authored message row carries.
pub const SERVICE_ACTOR_LABEL: &str = "herdr-graph";

/// The summary of a service-authored message, as history reads it back
/// (`queries::message_summary`): built on the write path, where the decided
/// row is not yet readable through a query connection.
// Allowed: the message row's parts, each from the caller's transaction.
#[allow(clippy::too_many_arguments)]
pub fn service_message_summary(
    message: MessageId,
    thread: ThreadId,
    author: ServiceAuthorId,
    kind: crate::protocol::results::MessageKind,
    sequence: u64,
    created_at: UtcMillis,
    source: &str,
    preview_detail_argv: Option<Vec<String>>,
) -> crate::protocol::results::MessageSummary {
    let snippet: String = source
        .chars()
        .take(crate::protocol::output::PREVIEW_SNIPPET_CHARS)
        .collect();
    crate::protocol::results::MessageSummary {
        message,
        thread,
        author: None,
        event_author: Some(EventAuthor::Programmatic(author)),
        author_role: Some(crate::protocol::summary::AuthorRole::Service),
        relays_user: false,
        author_role_backfilled: false,
        kind,
        sequence,
        created_at,
        actor_label: Some(SERVICE_ACTOR_LABEL.into()),
        preview_omitted: snippet.len() < source.len(),
        preview_data: snippet,
        preview_detail_argv,
    }
}
