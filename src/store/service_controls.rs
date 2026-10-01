//! Managed thread and requirement decisions for a registered service.

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::{
    ports::{
        ServiceAuthorityGate, ServiceConnectionAuthority, ServiceDecisionStartError,
        ServiceDecisionTransaction,
    },
    protocol::{
        ids::{InvitationId, RequirementId, SeatId, ThreadId, prefix},
        pagination::{Consistency, Cursor, CursorDirection, CursorScope, Page, StopReason},
        results::{ApiError, ErrorCode},
        service::{
            EventAuthor, InvitationConstraint, ManagedThread, RequirementState, ServiceInvitation,
            ServiceMembership, ServiceMembershipQuery, ServiceOperation, ServiceResult,
            VoluntaryMembershipState,
        },
        time::{CallBudget, UtcMillis},
    },
};

use super::{
    connection::{StoreContext, api_error, store_error},
    schema::{self, EventInput},
    service_substrate,
};

// Allowed: service control entry: request, connection authority, gate, budget and default.
#[allow(clippy::too_many_arguments)]
pub fn operate(
    context: &StoreContext,
    writer: &mut Connection,
    instance: &str,
    operation: ServiceOperation,
    connection: &ServiceConnectionAuthority,
    gate: &dyn ServiceAuthorityGate,
    budget: &CallBudget,
    installation_default_ms: Option<u64>,
) -> Result<ServiceResult, ApiError> {
    if connection.instance() != instance {
        return Err(api_error(
            ErrorCode::InstanceMismatch,
            "service connection instance mismatch",
        ));
    }
    if budget.is_exhausted(context.clock()) {
        return Err(api_error(
            ErrorCode::DeadlineExceeded,
            "service store budget exhausted",
        ));
    }
    let decision = ServiceDecisionTransaction::begin(writer, gate, connection).map_err(
        |error| match error {
            ServiceDecisionStartError::Database(error) => store_error(error),
            ServiceDecisionStartError::Authority(error) => error,
        },
    )?;
    let tx = decision.transaction();
    let author = decision.author().clone();
    if author != *connection.author() {
        return Err(api_error(
            ErrorCode::StaleServiceGeneration,
            "service author changed",
        ));
    }
    if let ServiceOperation::Membership(query) = &operation {
        let result = ServiceResult::Membership(membership_query(tx, instance, query)?);
        decision.rollback().map_err(store_error)?;
        return Ok(result);
    }
    let key = operation
        .operation_key()
        .ok_or_else(|| api_error(ErrorCode::InvalidRequest, "operation key required"))?;
    let digest = schema::canonical_digest(&operation)?;
    let scope = format!("service:{instance}:{}", author.as_str());
    if let Some((old_digest, json)) = tx
        .query_row(
            "SELECT digest,result_json FROM operations WHERE actor_scope=?1 AND operation_key=?2",
            params![scope, key.as_str()],
            |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(store_error)?
    {
        if old_digest != digest {
            return Err(api_error(
                ErrorCode::OperationPayloadMismatch,
                "operation key reused with different payload",
            ));
        }
        let result: ServiceResult = serde_json::from_str(&json).map_err(|e| {
            api_error(
                ErrorCode::StoreCorrupt,
                format!("invalid service replay: {e}"),
            )
        })?;
        decision.rollback().map_err(store_error)?;
        return Ok(result);
    }
    let reserved =
        service_substrate::ensure_reserved_author(tx, instance, context.clock().utc_now())?;
    if reserved != author {
        return Err(api_error(
            ErrorCode::Unauthorized,
            "registered service author does not match reserved instance identity",
        ));
    }
    let at = context.clock().utc_now();
    let result = match &operation {
        ServiceOperation::EnsureThread(v) => ensure_thread(tx, instance, &author, v, at)?,
        ServiceOperation::Invite(v) => {
            invite(tx, instance, &author, v, at, installation_default_ms)?
        }
        ServiceOperation::SetTopic(v) => {
            require_owner(tx, instance, &v.thread, &author)?;
            tx.execute("UPDATE threads SET topic=?1,topic_revision=topic_revision+1,updated_at=?2 WHERE id=?3 AND topic!=?1",
                params![v.topic,at.0,v.thread.as_str()]).map_err(store_error)?;
            schema::bump_filter_revision(tx, instance, "topic", v.thread.as_str())?;
            schema::bump_filter_revision(tx, instance, "topic", "all")?;
            schema::bump_filter_revision(tx, instance, "directory", "all")?;
            ServiceResult::TopicChanged(managed_thread(tx, &v.thread, &author)?)
        }
        ServiceOperation::Archive(v) | ServiceOperation::Reopen(v) => {
            require_owner(tx, instance, &v.thread, &author)?;
            let archived = matches!(&operation, ServiceOperation::Archive(_));
            tx.execute("UPDATE threads SET archived=?1,updated_at=?2,directory_revision=directory_revision+1 WHERE id=?3 AND archived!=?1",
                params![archived,at.0,v.thread.as_str()]).map_err(store_error)?;
            schema::bump_filter_revision(tx, instance, "directory", "all")?;
            let thread = managed_thread(tx, &v.thread, &author)?;
            if archived {
                ServiceResult::Archived(thread)
            } else {
                ServiceResult::Reopened(thread)
            }
        }
        ServiceOperation::ReleaseRequirement(v) => release(tx, instance, &author, v, at)?,
        ServiceOperation::Notify(_) => {
            return Err(api_error(
                ErrorCode::Unsupported,
                "notification publication belongs to C3",
            ));
        }
        ServiceOperation::Membership(_) => unreachable!(),
    };
    let json = serde_json::to_string(&result).map_err(|e| {
        api_error(
            ErrorCode::StoreCorrupt,
            format!("cannot encode service result: {e}"),
        )
    })?;
    tx.execute("INSERT INTO operations(actor_scope,operation_key,digest,result_json,decided_at) VALUES (?1,?2,?3,?4,?5)",
        params![scope,key.as_str(),digest.as_slice(),json,at.0]).map_err(store_error)?;
    decision.commit().map_err(store_error)?;
    Ok(result)
}

fn managed_thread(
    tx: &Transaction<'_>,
    thread: &ThreadId,
    owner: &crate::protocol::ids::ServiceAuthorId,
) -> Result<ManagedThread, ApiError> {
    let archived: bool = tx
        .query_row(
            "SELECT archived FROM threads WHERE id=?1",
            [thread.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    Ok(ManagedThread {
        thread: thread.clone(),
        owner: owner.clone(),
        archived,
    })
}

fn require_owner(
    tx: &Transaction<'_>,
    instance: &str,
    thread: &ThreadId,
    author: &crate::protocol::ids::ServiceAuthorId,
) -> Result<(), ApiError> {
    let owner: Option<(String, Option<String>)> = tx
        .query_row(
            "SELECT instance_id,managed_owner_author_id FROM threads WHERE id=?1",
            [thread.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(store_error)?;
    match owner {
        Some((stored, Some(id))) if stored == instance && id == author.as_str() => Ok(()),
        Some(_) => Err(api_error(
            ErrorCode::IncompatibleOwnership,
            "managed thread belongs to another owner",
        )),
        None => Err(api_error(ErrorCode::NotFound, "thread missing")),
    }
}

/// Invitation episodes remain monotonic even when a required-only membership
/// keeps its older voluntary episode or a cancellation retains the invitation.
pub(crate) fn next_invitation_episode(
    tx: &Transaction<'_>,
    thread: &ThreadId,
    seat: &SeatId,
) -> Result<i64, ApiError> {
    let last: i64 = tx
        .query_row(
            "SELECT max(
        coalesce((SELECT episode FROM memberships WHERE thread_id=?1 AND seat_id=?2),0),
        coalesce((SELECT max(episode) FROM invitations WHERE thread_id=?1 AND seat_id=?2),0))",
            params![thread.as_str(), seat.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    last.checked_add(1)
        .ok_or_else(|| api_error(ErrorCode::SequenceExhausted, "invitation episode exhausted"))
}

fn ensure_thread(
    tx: &Transaction<'_>,
    instance: &str,
    author: &crate::protocol::ids::ServiceAuthorId,
    v: &crate::protocol::service::EnsureManagedThread,
    at: UtcMillis,
) -> Result<ServiceResult, ApiError> {
    if v.topic.is_empty() || v.topic.len() > 1024 || v.goal.is_empty() || v.goal.len() > 1024 {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid managed thread topic or goal",
        ));
    }
    let existing: Option<(String, Option<String>)> = tx
        .query_row(
            "SELECT instance_id,managed_owner_author_id FROM threads WHERE id=?1",
            [v.thread.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(store_error)?;
    if existing.is_some() {
        require_owner(tx, instance, &v.thread, author)?;
        return Ok(ServiceResult::ThreadEnsured(managed_thread(
            tx, &v.thread, author,
        )?));
    }
    tx.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,managed_owner_author_id) VALUES (?1,?2,?3,?4,?5,?5,?6)",
        params![v.thread.as_str(),instance,v.topic,v.goal,at.0,author.as_str()]).map_err(store_error)?;
    schema::bump_filter_revision(tx, instance, "directory", "all")?;
    Ok(ServiceResult::ThreadEnsured(managed_thread(
        tx, &v.thread, author,
    )?))
}

fn invite(
    tx: &Transaction<'_>,
    instance: &str,
    author: &crate::protocol::ids::ServiceAuthorId,
    v: &crate::protocol::service::ServiceInvite,
    at: UtcMillis,
    default_ms: Option<u64>,
) -> Result<ServiceResult, ApiError> {
    let (thread_instance, owner): (String, Option<String>) = tx
        .query_row(
            "SELECT instance_id,managed_owner_author_id FROM threads WHERE id=?1",
            [v.thread.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(store_error)?
        .ok_or_else(|| api_error(ErrorCode::NotFound, "thread missing"))?;
    if thread_instance != instance {
        return Err(api_error(
            ErrorCode::InstanceMismatch,
            "thread outside service instance",
        ));
    }
    if v.constraint == InvitationConstraint::Required && owner.as_deref() != Some(author.as_str()) {
        return Err(api_error(
            ErrorCode::RequiredInvitationNeedsManagedThread,
            "required invitation needs a thread managed by this service",
        ));
    }
    let seat_instance: Option<String> = tx
        .query_row(
            "SELECT instance_id FROM seats WHERE id=?1 AND state!='retired'",
            [v.seat.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    if seat_instance.as_deref() != Some(instance) {
        return Err(api_error(
            ErrorCode::TargetUnresolved,
            "invite target missing, retired, or outside instance",
        ));
    }
    let prior: Option<(i64, String)> = tx
        .query_row(
            "SELECT episode,state FROM memberships WHERE thread_id=?1 AND seat_id=?2",
            params![v.thread.as_str(), v.seat.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(store_error)?;
    let current = service_substrate::current_requirement(tx, &v.thread, &v.seat)?;
    let voluntary = service_substrate::effective_membership(tx, &v.thread, &v.seat)?.voluntary;
    if v.constraint == InvitationConstraint::Required {
        if let Some(current) = current {
            return Ok(ServiceResult::Invitation(ServiceInvitation {
                invitation: current.invitation.clone(),
                requirement: Some(current),
            }));
        }
    } else if voluntary == VoluntaryMembershipState::Joined {
        return Ok(ServiceResult::AlreadyJoined(
            crate::protocol::results::AlreadyJoined {
                thread: v.thread.clone(),
                seat: v.seat.clone(),
            },
        ));
    }
    let pending:Option<(String,i64,i64)>=tx.query_row(
        "SELECT i.id,i.episode,i.created_decision_seq FROM invitations i WHERE i.thread_id=?1 AND i.seat_id=?2 AND i.state='pending' AND NOT EXISTS (SELECT 1 FROM invitation_cancellations c WHERE c.invitation_id=i.id) AND NOT EXISTS (SELECT 1 FROM requirement_episodes r WHERE r.invitation_id=i.id AND r.created_decision_seq=i.created_decision_seq) ORDER BY i.episode DESC LIMIT 1",
        params![v.thread.as_str(),v.seat.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(store_error)?;
    if v.constraint == InvitationConstraint::Ordinary
        && let Some((id, _, _)) = pending
    {
        let requirement = service_substrate::current_requirement(tx, &v.thread, &v.seat)?
            .filter(|current| current.invitation.as_str() == id);
        return Ok(ServiceResult::Invitation(ServiceInvitation {
            invitation: InvitationId::new(id),
            requirement,
        }));
    }
    let duration = i64::try_from(v.deadline_millis.or(default_ms).unwrap_or(300_000))
        .map_err(|_| api_error(ErrorCode::InvalidRequest, "invitation duration too large"))?;
    if duration <= 0 {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invitation duration must be positive",
        ));
    }
    let new_invitation = pending.is_none() || voluntary == VoluntaryMembershipState::Joined;
    let (invitation, episode, created_seq) = if new_invitation {
        let episode = next_invitation_episode(tx, &v.thread, &v.seat)?;
        if v.constraint == InvitationConstraint::Required {
            if prior.is_none() {
                tx.execute("INSERT INTO memberships(thread_id,seat_id,episode,state,voluntary_state) VALUES (?1,?2,?3,'invited','absent')",
                    params![v.thread.as_str(),v.seat.as_str(),episode]).map_err(store_error)?;
            } else if prior.as_ref().is_some_and(|(_, state)| state == "left") {
                tx.execute("UPDATE memberships SET state='invited',voluntary_state='left' WHERE thread_id=?1 AND seat_id=?2",
                    params![v.thread.as_str(),v.seat.as_str()]).map_err(store_error)?;
            }
        } else if prior.is_none() {
            tx.execute("INSERT INTO memberships(thread_id,seat_id,episode,state,voluntary_state) VALUES (?1,?2,?3,'invited','invited')",
                params![v.thread.as_str(),v.seat.as_str(),episode]).map_err(store_error)?;
        } else {
            tx.execute("UPDATE memberships SET state='invited',episode=?1,voluntary_state='invited',joined_at=NULL,left_at=NULL WHERE thread_id=?2 AND seat_id=?3",
                params![episode,v.thread.as_str(),v.seat.as_str()]).map_err(store_error)?;
        }
        let id = InvitationId::new(crate::store::public_ids::fresh(
            tx,
            prefix::INVITATION,
            &[("invitations", "id", prefix::INVITATION)],
        )?);
        let seq = schema::next_decision_seq(tx, instance)? as i64;
        let deadline = schema::checked_deadline(at, duration)?;
        tx.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,frozen_duration_ms,deadline_at) VALUES (?1,?2,?3,?4,'pending',?5,?6,?7,?8)",
            params![id.as_str(),v.thread.as_str(),v.seat.as_str(),episode,at.0,seq,duration,deadline.0]).map_err(store_error)?;
        (id, episode, seq)
    } else {
        let (id, episode, seq) = pending.expect("existing pending invitation");
        (InvitationId::new(id), episode, seq)
    };
    let requirement = if v.constraint == InvitationConstraint::Required {
        let id = RequirementId::new(crate::store::public_ids::fresh(
            tx,
            prefix::REQUIREMENT,
            &[("requirement_episodes", "id", prefix::REQUIREMENT)],
        )?);
        let seq = if new_invitation {
            created_seq
        } else {
            schema::next_decision_seq(tx, instance)? as i64
        };
        tx.execute("INSERT INTO requirement_episodes(id,thread_id,seat_id,issuer_author_id,invitation_id,state,created_decision_seq,created_at) VALUES (?1,?2,?3,?4,?5,'pending',?6,?7)",
            params![id.as_str(),v.thread.as_str(),v.seat.as_str(),author.as_str(),invitation.as_str(),seq,at.0]).map_err(store_error)?;
        service_substrate::current_requirement(tx, &v.thread, &v.seat)?
    } else {
        None
    };
    let payload = serde_json::json!({"action":"service_invite","issuer":author.as_str(),
        "seat":v.seat.as_str(),"invitation":invitation.as_str(),"required":requirement.is_some(),
        "episode":episode})
    .to_string();
    schema::append_attributed_event_once(
        tx,
        EventInput {
            thread: &v.thread,
            key: &format!(
                "service_invite:{}:{}",
                invitation.as_str(),
                requirement
                    .as_ref()
                    .map_or("ordinary", |r| r.requirement.as_str())
            ),
            kind: "info",
            payload_json: &payload,
            decision_at: at,
            source_message: None,
            source_invitation: Some(&invitation),
        },
        EventAuthor::Programmatic(author.clone()),
    )?;
    tx.execute("INSERT INTO wake_work(seat_id,reason_bits,attention_version) VALUES (?1,1,1) ON CONFLICT(seat_id) DO UPDATE SET reason_bits=reason_bits|1,attention_version=attention_version+1",
        [v.seat.as_str()]).map_err(store_error)?;
    schema::bump_membership_revision(tx, &v.thread)?;
    schema::bump_filter_revision(tx, instance, "directory", "all")?;
    schema::bump_filter_revision(
        tx,
        instance,
        "directory",
        &format!("member:{}", v.seat.as_str()),
    )?;
    schema::bump_filter_revision(tx, instance, "inbox", v.seat.as_str())?;
    Ok(ServiceResult::Invitation(ServiceInvitation {
        invitation,
        requirement,
    }))
}

fn release(
    tx: &Transaction<'_>,
    instance: &str,
    author: &crate::protocol::ids::ServiceAuthorId,
    v: &crate::protocol::service::ReleaseRequirement,
    at: UtcMillis,
) -> Result<ServiceResult, ApiError> {
    require_owner(tx, instance, &v.thread, author)?;
    let current = service_substrate::current_requirement(tx, &v.thread, &v.seat)?
        .ok_or_else(|| api_error(ErrorCode::NotFound, "effective requirement missing"))?;
    if current.requirement != v.requirement {
        return Err(api_error(
            ErrorCode::Conflict,
            "requirement episode changed",
        ));
    }
    tx.execute("UPDATE requirement_episodes SET state='released',revision=revision+1,released_at=?1 WHERE id=?2 AND state IN ('pending','accepted')",
        params![at.0,v.requirement.as_str()]).map_err(store_error)?;
    if current.state == RequirementState::Pending {
        tx.execute("INSERT INTO invitation_cancellations(invitation_id,requirement_id,cancelled_at) SELECT i.id,r.id,?1 FROM requirement_episodes r JOIN invitations i ON i.id=r.invitation_id WHERE r.id=?2 AND r.created_decision_seq=i.created_decision_seq AND i.state='pending'",
            params![at.0,v.requirement.as_str()]).map_err(store_error)?;
    }
    schema::bump_membership_revision(tx, &v.thread)?;
    schema::bump_filter_revision(tx, instance, "directory", "all")?;
    schema::bump_filter_revision(
        tx,
        instance,
        "directory",
        &format!("member:{}", v.seat.as_str()),
    )?;
    schema::bump_filter_revision(tx, instance, "inbox", v.seat.as_str())?;
    let mut released = current;
    released.state = RequirementState::Released;
    released.revision += 1;
    let payload = serde_json::json!({"action":"release_requirement","issuer":author.as_str(),
        "requirement":v.requirement.as_str(),"seat":v.seat.as_str()})
    .to_string();
    schema::append_attributed_event_once(
        tx,
        EventInput {
            thread: &v.thread,
            key: &format!("release_requirement:{}", v.requirement.as_str()),
            kind: "info",
            payload_json: &payload,
            decision_at: at,
            source_message: None,
            source_invitation: Some(&released.invitation),
        },
        EventAuthor::Programmatic(author.clone()),
    )?;
    Ok(ServiceResult::RequirementReleased(released))
}

fn membership_query(
    tx: &Transaction<'_>,
    instance: &str,
    q: &ServiceMembershipQuery,
) -> Result<Page<ServiceMembership>, ApiError> {
    q.page
        .validate()
        .map_err(|why| api_error(ErrorCode::InvalidRequest, why))?;
    let thread_instance: Option<String> = tx
        .query_row(
            "SELECT instance_id FROM threads WHERE id=?1",
            [q.thread.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    if thread_instance.as_deref() != Some(instance) {
        return Err(api_error(ErrorCode::NotFound, "thread missing"));
    }
    if let Some(seat) = &q.seat {
        let membership: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM memberships WHERE thread_id=?1 AND seat_id=?2)",
                params![q.thread.as_str(), seat.as_str()],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        if !membership {
            if q.page.cursor.is_some() {
                return Err(api_error(
                    ErrorCode::InvalidCursor,
                    "absent membership has no continuation",
                ));
            }
            let seat_state: Option<String> = tx
                .query_row(
                    "SELECT state FROM seats WHERE id=?1 AND instance_id=?2",
                    params![seat.as_str(), instance],
                    |r| r.get(0),
                )
                .optional()
                .map_err(store_error)?;
            let state = match seat_state.as_deref() {
                Some("retired") => VoluntaryMembershipState::Retired,
                Some(_) => VoluntaryMembershipState::Absent,
                None => {
                    return Err(api_error(
                        ErrorCode::NotFound,
                        "seat missing from service instance",
                    ));
                }
            };
            let page = Page {
                items: vec![ServiceMembership {
                    thread: q.thread.clone(),
                    seat: seat.clone(),
                    voluntary_state: state,
                    requirement: None,
                }],
                next_cursor: None,
                next_argv: None,
                high_water_ordinal: 0,
                scope_revision: None,
                has_more: false,
                stop_reason: StopReason::Complete,
                consistency: Consistency::BoundedLive,
            };
            let bytes = serde_json::to_vec(&ServiceResult::Membership(page.clone()))
                .map_err(|e| api_error(ErrorCode::StoreCorrupt, e.to_string()))?;
            if bytes.len() > q.page.max_bytes as usize {
                return Err(api_error(
                    ErrorCode::InvalidBudget,
                    "service membership page exceeds byte budget",
                ));
            }
            return Ok(page);
        }
    }
    let filter = if let Some(seat) = &q.seat {
        format!("seat:{}", seat.as_str())
    } else {
        "all".into()
    };
    let scope_key = q.thread.as_str();
    let cursor = q
        .page
        .cursor
        .as_deref()
        .map(|raw| {
            Cursor::decode_for(
                raw,
                instance,
                CursorScope::ServiceMembership,
                scope_key,
                &filter,
                CursorDirection::Ascending,
                1,
            )
        })
        .transpose()
        .map_err(|why| api_error(ErrorCode::InvalidCursor, why))?;
    if let Some(cursor) = &cursor {
        cursor
            .validate_for(
                instance,
                CursorScope::ServiceMembership,
                scope_key,
                &filter,
                CursorDirection::Ascending,
                1,
            )
            .map_err(|why| api_error(ErrorCode::InvalidCursor, why))?;
    }
    let high = cursor.as_ref().map_or_else(
        || {
            tx.query_row(
                "SELECT coalesce(max(ordinal),0) FROM memberships WHERE thread_id=?1",
                [q.thread.as_str()],
                |r| r.get::<_, i64>(0),
            )
            .map(|v| v as u64)
            .map_err(store_error)
        },
        |c| Ok(c.high_water_ordinal),
    )?;
    let mut after = cursor.as_ref().map_or(0, |c| c.after_ordinal);
    let mut items = Vec::new();
    let mut positions = Vec::new();
    let mut reason = StopReason::Complete;
    while items.len() < q.page.limit as usize {
        let row:Option<(i64,String)>=tx.query_row(
            "SELECT m.ordinal,m.seat_id FROM memberships m WHERE m.thread_id=?1 AND m.ordinal>?2 AND m.ordinal<=?3 AND (?4 IS NULL OR m.seat_id=?4) ORDER BY m.ordinal LIMIT 1",
            params![q.thread.as_str(),after as i64,high as i64,q.seat.as_ref().map(|s|s.as_str())],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(store_error)?;
        let Some((ordinal, seat)) = row else {
            break;
        };
        let prior = after;
        after = ordinal as u64;
        let seat = SeatId::new(seat);
        let state = service_substrate::effective_membership(tx, &q.thread, &seat)?.voluntary;
        let requirement = service_substrate::latest_requirement(tx, &q.thread, &seat)?;
        items.push(ServiceMembership {
            thread: q.thread.clone(),
            seat,
            voluntary_state: state,
            requirement,
        });
        positions.push(prior);
        if serde_json::to_vec(&items)
            .map_err(|e| api_error(ErrorCode::StoreCorrupt, e.to_string()))?
            .len()
            > q.page.max_bytes as usize
        {
            items.pop();
            positions.pop();
            after = prior;
            if items.is_empty() {
                return Err(api_error(
                    ErrorCode::InvalidBudget,
                    "service membership row exceeds page byte budget",
                ));
            }
            reason = StopReason::Bytes;
            break;
        }
    }
    loop {
        let more:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM memberships WHERE thread_id=?1 AND ordinal>?2 AND ordinal<=?3 AND (?4 IS NULL OR seat_id=?4))",
            params![q.thread.as_str(),after as i64,high as i64,q.seat.as_ref().map(|s|s.as_str())],|r|r.get(0)).map_err(store_error)?;
        let stop = if more && reason == StopReason::Complete {
            StopReason::Rows
        } else {
            reason
        };
        let next = if more {
            Some(
                Cursor {
                    instance: instance.into(),
                    scope: CursorScope::ServiceMembership,
                    scope_key: scope_key.into(),
                    filter_digest: filter.clone(),
                    direction: CursorDirection::Ascending,
                    order_version: 1,
                    last_examined_key: None,
                    after_ordinal: after,
                    high_water_ordinal: high,
                    scope_revision: None,
                    filter_revision: None,
                    search: None,
                    inbox: None,
                    attention: None,
                    binding: None,
                }
                .encode()
                .map_err(|why| api_error(ErrorCode::InvalidCursor, why))?,
            )
        } else {
            None
        };
        let page = Page {
            items: items.clone(),
            next_cursor: next.clone(),
            next_argv: next.as_ref().map(|cursor| {
                let mut argv = vec![
                    "service".into(),
                    "membership".into(),
                    "--thread".into(),
                    q.thread.as_str().into(),
                ];
                if let Some(seat) = &q.seat {
                    argv.extend(["--seat".into(), seat.as_str().into()]);
                }
                argv.extend(["--cursor".into(), cursor.clone()]);
                argv
            }),
            high_water_ordinal: high,
            scope_revision: None,
            has_more: more,
            stop_reason: stop,
            consistency: Consistency::BoundedLive,
        };
        let encoded = serde_json::to_vec(&ServiceResult::Membership(page.clone()))
            .map_err(|e| api_error(ErrorCode::StoreCorrupt, e.to_string()))?;
        if items.is_empty() && more {
            return Err(api_error(
                ErrorCode::InvalidBudget,
                "service membership row exceeds page byte budget",
            ));
        }
        if encoded.len() <= q.page.max_bytes as usize {
            return Ok(page);
        }
        if items.pop().is_none() {
            return Err(api_error(
                ErrorCode::InvalidBudget,
                "service membership page exceeds byte budget",
            ));
        }
        after = positions.pop().expect("one position per membership row");
        reason = StopReason::Bytes;
    }
}
