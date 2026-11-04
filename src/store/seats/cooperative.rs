//! Cooperative mapping and enrolment, `register_available`, hook reconnect and mod seat views.

use crate::{
    ports::HostId,
    protocol::{
        authority::{MutationPermit, ObligationRef, OperatorActor},
        commands::CheckIn,
        ids::{BindingId, SeatId},
        results::{ApiError, CheckInResult, CommandResult, ErrorCode},
        time::{CallBudget, UtcMillis},
    },
    store::{
        bindings, catch_up,
        connection::{DecisionInstant, StoreContext, api_error, store_error},
        effective::{self},
        schema,
    },
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::*;

/// Local mapping witness. These boot/epoch values describe durable mapping;
/// they are never native caller provenance or execution attestation.
#[derive(Debug, Clone)]
pub(crate) struct CooperativeMapping {
    pub boot: String,
    pub epoch: u64,
    pub revision: u64,
    pub invalidation_revision: u64,
    pub generation: u64,
    /// Terminal and host incarnation from the same verified effective
    /// observation that proved this mapping. A binding written from this
    /// mapping stores them so snapshot reconciliation can later reconfirm it.
    pub terminal: Option<String>,
    pub incarnation: Option<String>,
}

impl CooperativeMapping {
    /// The reconfirmation evidence a new or re-registered binding must carry.
    pub(crate) fn reconfirmation_evidence(&self) -> Result<(&str, &str), ApiError> {
        binding_evidence(self.terminal.as_deref(), self.incarnation.as_deref())
    }
}

/// A binding is stored only with the terminal and host incarnation that a
/// later complete snapshot must match to reconfirm it after invalidation.
/// Without them the seat could never recover automatically, so registration
/// fails closed and actionable instead of storing an unrecoverable binding.
pub(crate) fn binding_evidence<'a>(
    terminal: Option<&'a str>,
    incarnation: Option<&'a str>,
) -> Result<(&'a str, &'a str), ApiError> {
    match (terminal, incarnation) {
        (Some(terminal), Some(incarnation)) if !terminal.is_empty() && !incarnation.is_empty() => {
            Ok((terminal, incarnation))
        }
        _ => Err(api_error(
            ErrorCode::StaleHostObservation,
            "host observation lacks terminal/incarnation reconfirmation evidence; retry after the next verified host observation",
        )),
    }
}

/// The one store entry check of a cooperative call: node, declared role, the attached Herdr
/// server the claim names, a local seat, and that seat sitting on that server. Returns the
/// `HostId` the caller keeps for the rest of its transaction (mapping, holds, observation,
/// seat writes).
///
/// Owns: the decision whether a claim may act (A2, decided inside the deciding transaction from
/// the canonical view). Does not own: the claim's wire shape, the host's reachability or session,
/// or what the call then does. Error split: `CallerUnverified` for a claim that does not match
/// the store (node or role, a seat that is not local, a seat on another server), `HostUnavailable`
/// when the named server is not attached (retryable once it is). C1 continuity and the operator
/// rebinds do not go through here (they carry no claim, or move a seat between servers).
pub(crate) fn cooperative_claim(
    db: &Connection,
    ctx: &StoreContext,
    node: &str,
    claim: &crate::protocol::authority::CallerClaim,
) -> Result<HostId, ApiError> {
    claim_shape(claim, node)?;
    let host = ctx.hosts().attached_id(&claim.host)?;
    let seat_host = local_seat_host(db, &claim.seat, node)?;
    if seat_host != host {
        return Err(api_error(
            ErrorCode::CallerUnverified,
            "cooperative seat sits on another Herdr server",
        ));
    }
    Ok(host)
}

/// The claim's node and declared role against the store's node. A subagent claim carries its
/// agent id and resolves to a binding attached to its parent (ht-54r.3); refusing it a write is
/// `Command::validate`'s and dispatch's job (ht-54r.20).
fn claim_shape(
    claim: &crate::protocol::authority::CallerClaim,
    node: &str,
) -> Result<(), ApiError> {
    use crate::protocol::authority::CallerRole;
    let role_matches = match claim.role {
        CallerRole::TopLevel => claim.agent_id.is_none(),
        CallerRole::Subagent => claim.agent_id.is_some(),
    };
    if claim.instance != node || !role_matches {
        return Err(api_error(
            ErrorCode::CallerUnverified,
            "cooperative node or declared role mismatch",
        ));
    }
    Ok(())
}

/// The server a local seat of `node` sits on; a missing or remote seat is `CallerUnverified`.
fn local_seat_host(db: &Connection, seat: &SeatId, node: &str) -> Result<HostId, ApiError> {
    if !is_local_seat(db, seat)? {
        return Err(api_error(
            ErrorCode::CallerUnverified,
            "cooperative seat is not local",
        ));
    }
    let owner: String = db
        .query_row(
            "SELECT instance_id FROM seats WHERE id=?1",
            [seat.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if owner != node {
        return Err(api_error(
            ErrorCode::CallerUnverified,
            "cooperative seat belongs to another instance",
        ));
    }
    seat_host(db, seat)
}

/// The mapping of the claim's seat on the server the seat itself sits on, for a revalidation
/// inside a transaction (or a permit issue) whose entry already ran [`cooperative_claim`]: it
/// checks the claim's node, role and the seat's locality, not `claim.host` against the seat.
/// The entry points hold the one A2 claim-host check; the deep validators of handoff and
/// bootstrap call this.
pub(crate) fn seat_mapping(
    db: &Connection,
    claim: &crate::protocol::authority::CallerClaim,
    mode: Option<crate::protocol::commands::CheckInMode>,
) -> Result<CooperativeMapping, ApiError> {
    claim_shape(claim, &claim.instance)?;
    let host = local_seat_host(db, &claim.seat, &claim.instance)?;
    cooperative_mapping(db, host, claim, mode)
}

/// [`seat_mapping`] plus: the claim's own binding is open (see [`cooperative_bound_mapping`]).
pub(crate) fn seat_bound_mapping(
    db: &Connection,
    claim: &crate::protocol::authority::CallerClaim,
) -> Result<CooperativeMapping, ApiError> {
    claim_shape(claim, &claim.instance)?;
    let host = local_seat_host(db, &claim.seat, &claim.instance)?;
    cooperative_bound_mapping(db, host, claim, None)
}

/// [`cooperative_claim`] then [`cooperative_mapping`] on the host it returned, for a decision that
/// needs the mapping and not the host afterwards.
pub(crate) fn claimed_mapping(
    db: &Connection,
    ctx: &StoreContext,
    node: &str,
    claim: &crate::protocol::authority::CallerClaim,
    mode: Option<crate::protocol::commands::CheckInMode>,
) -> Result<CooperativeMapping, ApiError> {
    let host = cooperative_claim(db, ctx, node, claim)?;
    cooperative_mapping(db, host, claim, mode)
}

/// Refuses a seat that exists and is not local: a remote seat is never occupied on this node.
/// The same error as [`cooperative_claim`]'s, for the paths that name a seat without a claim
/// (bindings, check-in, resolve, operator actions). An unknown seat is left to the caller's own
/// not-found handling. Checked before any write.
pub(crate) fn require_local_seat(conn: &Connection, seat: &SeatId) -> Result<(), ApiError> {
    let remote: bool = conn
        .query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM seats WHERE id=?1 AND NOT ({LOCAL_SEAT}))"),
            [seat.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if remote {
        Err(api_error(
            ErrorCode::CallerUnverified,
            "cooperative seat is not local",
        ))
    } else {
        Ok(())
    }
}

/// Refuses a seat homed on another node for the node-local nudge threshold (reconcile item 18:
/// never replicated, a remote row's value is never read). The detail names the home node's tag so
/// the operator knows where to set it. An unknown seat is left to the caller's not-found handling.
pub(crate) fn require_local_threshold_seat(
    conn: &Connection,
    seat: &SeatId,
) -> Result<(), ApiError> {
    let home: Option<String> = conn
        .query_row(
            &format!("SELECT home_node FROM seats WHERE id=?1 AND NOT ({LOCAL_SEAT})"),
            [seat.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    let Some(home) = home else {
        return Ok(());
    };
    let tag = home
        .parse::<crate::node::NodeUuid>()
        .ok()
        .and_then(|uuid| super::super::node_directory::get(conn, uuid).ok().flatten())
        .map(|row| row.tag.to_string())
        .unwrap_or_else(|| home.chars().take(8).collect());
    Err(api_error(
        ErrorCode::CallerUnverified,
        format!(
            "seat {} is homed on node {tag}; its nudge threshold is set there",
            seat.as_str()
        ),
    ))
}

/// Delivery's current recipient predicate uses canonical continuity without
/// allocating, binding or requiring a working session on an unbound seat.
pub(crate) fn eligible_delivery_recipient(
    db: &Connection,
    instance: &str,
    seat: &SeatId,
) -> Result<(), ApiError> {
    type Row = (
        String,
        Option<String>,
        i64,
        Option<String>,
        i64,
        Option<i64>,
        Option<i64>,
        i64,
    );
    // A remote seat has no host, target or observation: it is eligible iff its
    // row is `remote` (a retired one is not). Local seats keep the check below.
    let remote_state: Option<String> = db
        .query_row(
            &format!(
                "SELECT state FROM seats WHERE id=?1 AND instance_id=?2 AND NOT ({LOCAL_SEAT})"
            ),
            params![seat.as_str(), instance],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    if let Some(state) = remote_state {
        return if state == "remote" {
            Ok(())
        } else {
            Err(api_error(
                ErrorCode::TargetUnresolved,
                "delivery recipient unresolved, held or invalidated",
            ))
        };
    }
    let row: Option<Row> = db.query_row("SELECT s.state,s.target_id,s.target_generation,h.host_boot,h.host_epoch,s.retired_at,s.retired_seq,h.host_id FROM seats s JOIN herdr_hosts h ON h.host_id=s.host_id WHERE s.id=?1 AND s.instance_id=?2", params![seat.as_str(), instance], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).optional().map_err(store_error)?;
    let Some((state, Some(target), generation, boot, epoch, retired_at, retired_seq, host_id)) =
        row
    else {
        return Err(api_error(
            ErrorCode::TargetUnresolved,
            "delivery recipient missing or foreign",
        ));
    };
    let host = HostId::from_row(host_id);
    let held: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM recovery_holds WHERE host_id=?1 AND target_id=?2 AND released_at IS NULL)", params![host.get(), target], |r| r.get(0)).map_err(store_error)?;
    let observation = effective::effective_observation(db, host, &target)?;
    if state != "resolved"
        || retired_at.is_some()
        || retired_seq.is_some()
        || held
        || observation.as_ref().is_none_or(|o| {
            o.structural_generation != generation
                || Some(o.host_boot.as_str()) != boot.as_deref()
                || o.epoch != epoch
        })
    {
        return Err(api_error(
            ErrorCode::TargetUnresolved,
            "delivery recipient unresolved, held or invalidated",
        ));
    }
    Ok(())
}

/// The claim seat's canonical mapping on `host`, the server [`cooperative_claim`] resolved for
/// the claim (or the seat's own server for a claim the store builds itself).
pub(crate) fn cooperative_mapping(
    db: &Connection,
    host: HostId,
    claim: &crate::protocol::authority::CallerClaim,
    mode: Option<crate::protocol::commands::CheckInMode>,
) -> Result<CooperativeMapping, ApiError> {
    if !uuid::Uuid::parse_str(claim.execution.as_str())
        .is_ok_and(|execution| execution.hyphenated().to_string() == claim.execution.as_str())
    {
        return Err(api_error(
            ErrorCode::CallerUnverified,
            "cooperative execution must be a canonical lowercase hyphenated UUID",
        ));
    }
    type RowColumns = Option<(
        String,
        Option<String>,
        i64,
        i64,
        Option<String>,
        i64,
        i64,
        i64,
    )>;
    let row: RowColumns = db.query_row(
        "SELECT s.state,s.target_id,s.generation,s.target_generation,h.host_boot,h.host_epoch,h.invalidation_revision,h.host_id FROM seats s JOIN herdr_hosts h ON h.host_id=s.host_id WHERE s.id=?1",
        [claim.seat.as_str()], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).optional().map_err(store_error)?;
    let Some((state, target, generation, revision, boot, epoch, invalidation, host_id)) = row
    else {
        return Err(api_error(
            ErrorCode::CallerUnverified,
            "cooperative seat missing",
        ));
    };
    if host_id != host.get() {
        return Err(api_error(
            ErrorCode::CallerUnverified,
            "cooperative seat sits on another Herdr server",
        ));
    }
    if state != "resolved" || target.as_deref() != Some(claim.target.as_str()) {
        return Err(api_error(
            ErrorCode::CallerUnverified,
            "cooperative mapping or generation changed",
        ));
    }
    if matches!(
        mode,
        Some(crate::protocol::commands::CheckInMode::Fork { .. })
    ) && (claim.is_subagent() || claim.harness == crate::protocol::authority::Harness::Human)
    {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "independent fork requires a top-level agent",
        ));
    }
    let lifecycle_mode = matches!(
        mode,
        Some(
            crate::protocol::commands::CheckInMode::Lifecycle { .. }
                | crate::protocol::commands::CheckInMode::Fork { .. }
        )
    );
    let claimed = checked_host_number(claim.binding_generation)?;
    // A `managed_launch` placeholder neither bumps the seat generation nor is replaced by a
    // generation allowance: a launched agent's check-in matches the seat generation like any other.
    if generation != claimed {
        let code = if lifecycle_mode {
            ErrorCode::Conflict
        } else {
            ErrorCode::CallerUnverified
        };
        return Err(api_error(code, "cooperative binding generation changed"));
    }
    let held: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM recovery_holds WHERE host_id=?1 AND target_id=?2 AND released_at IS NULL)",
        params![host.get(),claim.target.as_str()], |r|r.get(0)).map_err(store_error)?;
    let observation = effective::effective_observation(db, host, claim.target.as_str())?;
    if held
        || observation.as_ref().is_none_or(|o| {
            o.structural_generation != revision
                || Some(o.host_boot.as_str()) != boot.as_deref()
                || o.epoch != epoch
        })
    {
        return Err(api_error(
            ErrorCode::CallerUnverified,
            "mapping held or known invalidated",
        ));
    }
    match mode {
        Some(
            crate::protocol::commands::CheckInMode::Lifecycle {
                expected_binding_generation,
            }
            | crate::protocol::commands::CheckInMode::Fork {
                expected_binding_generation,
            },
        ) => {
            if expected_binding_generation != claim.binding_generation {
                return Err(api_error(
                    ErrorCode::Conflict,
                    "lifecycle expected generation differs from context",
                ));
            }
            // Another session's execution is refused; a session's own lifecycle events share the
            // one execution its claims carry (its binding's `execution_id`, which its later
            // claims must match).
            let agent = claim.agent_id.as_ref().map_or("", |agent| agent.as_str());
            let policy =
                crate::harness::registry::builtins().binding_policy(claim.harness.as_str());
            // 2.0.3 child-start hooks stored the parent's execution on attached children.
            // Recognize only that exact canonical shape; do not rewrite history or relax
            // collisions for another session, harness, agent, or unrelated parent.
            let legacy_parent = !claim.is_subagent()
                && claim.harness != crate::protocol::authority::Harness::Human
                && claim.execution
                    == bindings::derived_execution(claim, claim.native_session.as_str(), None)?;
            let child_provenance = if policy.hook_agent_id {
                crate::protocol::authority::COOPERATIVE_HOOK_AGENT_PROVENANCE
            } else {
                crate::protocol::authority::COOPERATIVE_ENV_SESSION_PROVENANCE
            };
            let used: bool = db
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM occupant_bindings child
                 WHERE child.seat_id=?1 AND child.execution_id=?2
                 AND NOT (child.harness=?3 AND child.native_session=?4 AND child.agent_id=?5)
                 AND NOT (?6 AND child.role='subagent' AND child.agent_id<>''
                     AND child.harness=?3 AND child.observation_provenance=?7
                     AND (child.native_session=?4 OR (?8 AND child.native_session=child.agent_id))
                     AND EXISTS(SELECT 1 FROM occupant_bindings parent
                         WHERE parent.binding_id=child.parent_binding AND parent.seat_id=?1
                         AND parent.harness=?3 AND parent.native_session=?4
                         AND parent.agent_id='' AND parent.role='top_level'
                         AND parent.execution_id=?2)))",
                    params![
                        claim.seat.as_str(),
                        claim.execution.as_str(),
                        claim.harness.as_str(),
                        claim.native_session.as_str(),
                        agent,
                        legacy_parent,
                        child_provenance,
                        policy.registered_child_sessions
                    ],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            if used {
                return Err(api_error(
                    ErrorCode::CallerUnverified,
                    "lifecycle execution UUID already used",
                ));
            }
        }
        _ => {
            // Which binding the claim names is the daemon's resolution inside the deciding
            // transaction (`decide_cooperative`); there is no single occupant to compare to.
            let harness = claim.harness.as_str();
            if harness != "human" && crate::harness::registry::builtins().agent(harness).is_err() {
                return Err(api_error(
                    ErrorCode::Unsupported,
                    "claim harness has no registered adapter",
                ));
            }
        }
    }
    Ok(CooperativeMapping {
        boot: boot.unwrap(),
        epoch: epoch as u64,
        revision: revision as u64,
        invalidation_revision: invalidation as u64,
        generation: generation as u64,
        terminal: observation.as_ref().and_then(|o| o.terminal_id.clone()),
        incarnation: observation.and_then(|o| o.incarnation),
    })
}

/// [`cooperative_mapping`] plus: the claim's own binding (seat, harness, session, agent id) is
/// open. For validators that re-check authority a bound session already holds (a handoff, an
/// observation ticket) and never enroll; a claim resolved by `decide_cooperative` may enroll
/// instead and uses `cooperative_mapping`.
pub(crate) fn cooperative_bound_mapping(
    db: &Connection,
    host: HostId,
    claim: &crate::protocol::authority::CallerClaim,
    mode: Option<crate::protocol::commands::CheckInMode>,
) -> Result<CooperativeMapping, ApiError> {
    let mapping = cooperative_mapping(db, host, claim, mode)?;
    if super::super::queries::open_binding_for(db, &bindings::BindingKey::from_claim(claim))?
        .is_none()
    {
        return Err(api_error(
            ErrorCode::CallerUnverified,
            "cooperative occupant context changed",
        ));
    }
    Ok(mapping)
}

/// Parent Begin is canonically retained separately from the legacy child Begin
/// which deliberately shares its frozen operation key. Never consult the child ledger.
pub(crate) fn issue_bootstrap_begin_permit(
    context: &StoreContext,
    db: &Connection,
    instance: &str,
    canonical: &crate::protocol::handoff::HandoffNamespace,
    command: &crate::protocol::handoff::BeginBootstrap,
    budget: &CallBudget,
) -> Result<MutationPermit, ApiError> {
    crate::protocol::commands::Command::BeginBootstrap(Box::new(command.clone()))
        .validate()
        .map_err(ApiError::invalid_request)?;
    super::super::topology_handoff::encode_identity(canonical, &command.identity)?;
    let host = cooperative_claim(db, context, instance, &command.identity.claim)?;
    let current = super::super::topology_handoff::current(db, canonical, &command.identity)?;
    let mapping = if current
        .is_some_and(|v| v.state == crate::protocol::handoff::BootstrapState::Completed)
    {
        // Historical replay cannot consume a live deciding fence.
        (0, 0)
    } else {
        let mapping = cooperative_mapping(db, host, &command.identity.claim, None)?;
        (mapping.revision, mapping.invalidation_revision)
    };
    let request = crate::store::cooperative_permit_request(
        &crate::protocol::commands::PermitMutation::BeginBootstrap(Box::new(command.clone())),
    )?;
    Ok(MutationPermit::cooperative(
        request.claim,
        request.operation,
        request.obligation,
        request.payload_hash,
        context.clock().monotonic_now(),
        mapping,
        budget.clone(),
    ))
}

pub(crate) fn issue_cooperative_permit(
    context: &StoreContext,
    db: &Connection,
    instance: &str,
    mut request: crate::ports::CooperativePermitRequest,
    budget: &CallBudget,
) -> Result<MutationPermit, ApiError> {
    let host = cooperative_claim(db, context, instance, &request.claim)?;
    let stored: Option<Vec<u8>> = db
        .query_row(
            "SELECT digest FROM operations WHERE actor_scope IN (?1,?2) AND operation_key=?3",
            params![
                format!("seat:{}", request.claim.seat.as_str()),
                // A self-retired seat's replay is stored under its own scope (S4): the live-seat
                // mapping below would refuse it, so the stored record is looked up first.
                format!("self-retire:{}", request.claim.seat.as_str()),
                request.operation.as_str()
            ],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    let (revision, invalidation) = if let Some(digest) = stored {
        if digest != request.payload_hash {
            return Err(api_error(
                ErrorCode::OperationPayloadMismatch,
                "operation key reused with different payload",
            ));
        }
        // Historical replay is not authority to mutate. Decision validation still
        // rejects this stale context if no exact operation record is found.
        (0, 0)
    } else {
        if let ObligationRef::AcceptCurrent(thread) = &request.obligation {
            let invitation =
                super::control::current_invitation_for_accept(db, thread, &request.claim.seat)?
                    .ok_or_else(|| api_error(ErrorCode::NotFound, "current invitation missing"))?;
            request.obligation = ObligationRef::Invitation(invitation);
        }
        let mapping = cooperative_mapping(db, host, &request.claim, request.check_in_mode)?;
        (mapping.revision, mapping.invalidation_revision)
    };
    Ok(MutationPermit::cooperative(
        request.claim,
        request.operation,
        request.obligation,
        request.payload_hash,
        context.clock().monotonic_now(),
        (revision, invalidation),
        budget.clone(),
    ))
}

pub fn check_in_payload(command: &CheckIn) -> impl serde::Serialize + '_ {
    ("check_in", &command.mode, &command.claim)
}

// Allowed: one deciding mutation: transaction, permit, claim, mode and the obligation it proves.
#[allow(clippy::too_many_arguments)]
pub(crate) fn decide_cooperative(
    ctx: &StoreContext,
    tx: &Transaction<'_>,
    at: DecisionInstant,
    instance: &str,
    permit: &mut MutationPermit,
    claim: &crate::protocol::authority::CallerClaim,
    mode: Option<crate::protocol::commands::CheckInMode>,
    operation: &crate::protocol::ids::OperationId,
    obligation: &ObligationRef,
    digest: &[u8; 32],
) -> Result<(CooperativeMapping, bindings::Resolution), ApiError> {
    let mapping = decide_cooperative_fence(
        ctx, tx, at, instance, permit, claim, mode, operation, obligation, digest,
    )?;
    let resolution = resolve_cooperative(tx, ctx.processes(), at, claim, mode, &mapping)?;
    Ok((mapping, resolution))
}

/// The deciding fence of a cooperative call: the mapping against the canonical view (A2) and the
/// permit it consumes. Resolves no binding; callers that must act between the fence and the
/// binding write (`register_available`) call this and [`resolve_cooperative`] separately.
#[allow(clippy::too_many_arguments)]
pub(crate) fn decide_cooperative_fence(
    ctx: &StoreContext,
    tx: &Transaction<'_>,
    at: DecisionInstant,
    instance: &str,
    permit: &mut MutationPermit,
    claim: &crate::protocol::authority::CallerClaim,
    mode: Option<crate::protocol::commands::CheckInMode>,
    operation: &crate::protocol::ids::OperationId,
    obligation: &ObligationRef,
    digest: &[u8; 32],
) -> Result<CooperativeMapping, ApiError> {
    let host = cooperative_claim(tx, ctx, instance, claim)?;
    let mapping = cooperative_mapping(tx, host, claim, mode)?;
    let fence = crate::protocol::authority::CooperativeDecisionFence {
        now: at.monotonic,
        instance: instance.into(),
        host: claim.host.clone(),
        seat: claim.seat.clone(),
        target: claim.target.clone(),
        mapping_revision: mapping.revision,
        binding_generation: mapping.generation,
        invalidation_revision: mapping.invalidation_revision,
        known_invalidated: false,
    };
    let actor = permit
        .consume_cooperative(&fence, operation, obligation, digest)
        .map_err(|e| api_error(ErrorCode::CallerUnverified, e))?;
    if actor != claim {
        return Err(api_error(
            ErrorCode::CallerUnverified,
            "cooperative permit context differs from command",
        ));
    }
    Ok(mapping)
}

/// The placement a binding written by `claim` records, from the deciding mapping. Reconfirmation
/// evidence is needed only to write a placement: a plain touch of an open binding writes none, so
/// a missing observation does not refuse it.
fn cooperative_placement(
    tx: &Transaction<'_>,
    at: DecisionInstant,
    claim: &crate::protocol::authority::CallerClaim,
    lifecycle: bool,
    mapping: &CooperativeMapping,
) -> Result<bindings::BindingPlacement, ApiError> {
    let key = bindings::BindingKey::from_claim(claim);
    let placement = match mapping.reconfirmation_evidence() {
        Ok((terminal, incarnation)) => (terminal.to_owned(), incarnation.to_owned()),
        Err(error) => {
            if lifecycle || super::super::queries::open_binding_for(tx, &key)?.is_none() {
                return Err(error);
            }
            (String::new(), String::new())
        }
    };
    Ok(bindings::BindingPlacement {
        generation: mapping.generation,
        target_generation: mapping.revision,
        target: claim.target.as_str().to_owned(),
        terminal: placement.0,
        incarnation: placement.1,
        host_boot: mapping.boot.clone(),
        host_epoch: mapping.epoch,
        execution: claim.execution.as_str().to_owned(),
        provenance: claim.harness.cooperative_provenance().to_owned(),
        observed_at: at.utc,
        registered_at: Some(at.utc),
    })
}

/// The ancestors of `pid` (nearest first), bounded so a cyclic table cannot loop.
fn ancestor_pids(processes: &dyn crate::ports::ProcessInfo, pid: u32) -> Vec<u32> {
    let mut chain = Vec::new();
    let mut next = processes.process(pid).and_then(|p| p.parent);
    while let Some(parent) = next {
        if chain.len() >= 64 || chain.contains(&parent) {
            break;
        }
        chain.push(parent);
        next = processes.process(parent).and_then(|p| p.parent);
    }
    chain
}

/// The live-holder guard (SA16, D19; TRUST-POLICY N rows): when a named seat would gain or
/// reconnect a top-level binding, refuse with `seat_name_in_use` (M7) if another connected
/// top-level binding of the seat is held by a live process that differs from the caller's with
/// neither in the other's parent chain. Admitted beside a live holder: the same process, an
/// ancestor or descendant, subagents, a `readonly` holder or caller, a Codex fork, and any holder whose
/// pid is gone or reused (its start stamp no longer matches). A claim without a fingerprint is
/// left to the admission check that requires one. Cooperative same-user consistency, not
/// verification.
fn guard_live_holder(
    tx: &Connection,
    processes: &dyn crate::ports::ProcessInfo,
    claim: &crate::protocol::authority::CallerClaim,
    fork: bool,
    readonly: bool,
) -> Result<(), ApiError> {
    let Some(caller) = claim.liveness else {
        return Ok(());
    };
    if fork
        || readonly
        || claim.is_subagent()
        || claim.harness == crate::protocol::authority::Harness::Human
    {
        return Ok(());
    }
    // An open, connected binding of the caller's own key is already a holder: a touch.
    if bindings::match_claim(tx, &bindings::BindingKey::from_claim(claim))?
        .is_some_and(|row| row.liveness == bindings::Liveness::Connected)
    {
        return Ok(());
    }
    let name: Option<String> = tx
        .query_row(
            "SELECT name FROM seats WHERE id=?1",
            [claim.seat.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?
        .flatten();
    let Some(name) = name else {
        return Ok(());
    };
    let mut stmt = tx
        .prepare(
            "SELECT liveness_pid,liveness_start FROM occupant_bindings \
             WHERE seat_id=?1 AND ended_at IS NULL AND liveness='connected' AND role='top_level' \
             AND concurrent_session=0 AND readonly=0 AND liveness_pid IS NOT NULL \
             AND observation_provenance<>?2 \
             AND NOT (harness=?3 AND native_session=?4 AND agent_id='')",
        )
        .map_err(store_error)?;
    let holders = stmt
        .query_map(
            params![
                claim.seat.as_str(),
                crate::protocol::authority::MANAGED_LAUNCH_PROVENANCE,
                claim.harness.as_str(),
                claim.native_session.as_str(),
            ],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        )
        .map_err(store_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(store_error)?;
    let caller_ancestors = ancestor_pids(processes, caller.pid);
    for (pid, start) in holders {
        let (Ok(pid), Ok(start)) = (u32::try_from(pid), u64::try_from(start)) else {
            continue;
        };
        let holder = crate::protocol::authority::LivenessFingerprint { pid, start };
        if holder.pid == caller.pid
            || !holder.is_alive(processes)
            || caller_ancestors.contains(&holder.pid)
            || ancestor_pids(processes, holder.pid).contains(&caller.pid)
        {
            continue;
        }
        return Err(api_error(
            ErrorCode::SeatNameInUse,
            // M7 (59 bytes + fields) after the CLI's `herdr-threads: ` prefix.
            format!("seat {name} is held by live pid {pid}; use another name"),
        ));
    }
    Ok(())
}

/// Resolve the claim to its own binding (ht-54r.3): enroll, reconnect, reopen, attach or touch.
/// Never ends another binding.
pub(crate) fn resolve_cooperative(
    tx: &Transaction<'_>,
    processes: &dyn crate::ports::ProcessInfo,
    at: DecisionInstant,
    claim: &crate::protocol::authority::CallerClaim,
    mode: Option<crate::protocol::commands::CheckInMode>,
    mapping: &CooperativeMapping,
) -> Result<bindings::Resolution, ApiError> {
    resolve_cooperative_with_lineage(tx, processes, at, claim, mode, mapping, None, false)
}

/// [`resolve_cooperative`] with a lineage predecessor hint (Hermes `parent_session_id`, spec §1
/// Lineage). The hint names a session; it resolves to the open top-level binding of that session on
/// the claim's seat and harness, and is honoured only when `bindings::validate_lineage` holds (same
/// seat, harness, top-level, same process fingerprint). A successor not yet enrolled seeds from the
/// predecessor (`ResolveRequest.lineage`); one already enrolled (CLI-first, an accepted limit) links
/// `lineage_from` once and moves no cursor (it keeps its own read position).
// Allowed: a resolve's inputs plus the two facts only a check-in carries.
#[allow(clippy::too_many_arguments)]
pub(crate) fn resolve_cooperative_with_lineage(
    tx: &Transaction<'_>,
    processes: &dyn crate::ports::ProcessInfo,
    at: DecisionInstant,
    claim: &crate::protocol::authority::CallerClaim,
    mode: Option<crate::protocol::commands::CheckInMode>,
    mapping: &CooperativeMapping,
    lineage_parent: Option<&crate::protocol::ids::NativeSessionId>,
    readonly: bool,
) -> Result<bindings::Resolution, ApiError> {
    let lifecycle = matches!(
        mode,
        Some(
            crate::protocol::commands::CheckInMode::Lifecycle { .. }
                | crate::protocol::commands::CheckInMode::Fork { .. }
        )
    );
    guard_live_holder(
        tx,
        processes,
        claim,
        matches!(
            mode,
            Some(crate::protocol::commands::CheckInMode::Fork { .. })
        ),
        readonly,
    )?;
    let placement = cooperative_placement(tx, at, claim, lifecycle, mapping)?;
    let predecessor = match lineage_parent {
        Some(parent)
            if !matches!(
                mode,
                Some(crate::protocol::commands::CheckInMode::Fork { .. })
            ) && *parent != claim.native_session
                && !claim.is_subagent() =>
        {
            bindings::match_claim(
                tx,
                &bindings::BindingKey {
                    seat: claim.seat.clone(),
                    harness: claim.harness.as_str().to_owned(),
                    native_session: parent.clone(),
                    agent_id: None,
                },
            )?
            .filter(|row| row.role == bindings::BindingRole::TopLevel)
            .map(|row| row.binding_id)
        }
        _ => None,
    };
    let resolution = bindings::resolve_claim(
        tx,
        &bindings::ResolveRequest {
            claim,
            lifecycle,
            fork: matches!(
                mode,
                Some(crate::protocol::commands::CheckInMode::Fork { .. })
            ),
            placement: &placement,
            lineage: predecessor.as_ref(),
        },
        at.utc,
    )?;
    if let Some(predecessor) = &predecessor
        && !matches!(resolution, bindings::Resolution::Enrolled { .. })
    {
        let successor = resolution.binding();
        if successor.role == bindings::BindingRole::TopLevel
            && !successor.concurrent_session
            && successor.lineage_from.is_none()
            && bindings::validate_lineage(tx, claim, predecessor)?
        {
            bindings::link_lineage(tx, &successor.binding_id, predecessor)?;
        }
    }
    Ok(resolution)
}

/// The binding a Hermes delegate child's start event registers (ht-54r.11): the child session as
/// a subagent binding of the parent's top-level binding (enrolled first if needed), keyed
/// `(seat, hermes, child, agent_id = child)`. Returns whether the binding was created.
pub(crate) fn attach_registered_child(
    tx: &Transaction<'_>,
    processes: &dyn crate::ports::ProcessInfo,
    at: DecisionInstant,
    parent_claim: &crate::protocol::authority::CallerClaim,
    child: &crate::protocol::ids::NativeSessionId,
    mapping: &CooperativeMapping,
) -> Result<bool, ApiError> {
    let parent = resolve_cooperative(tx, processes, at, parent_claim, None, mapping)?
        .binding()
        .clone();
    let child_claim = bindings::registered_child_claim(parent_claim, child)?;
    let mut placement = cooperative_placement(tx, at, &child_claim, false, mapping)?;
    placement.provenance =
        crate::protocol::authority::COOPERATIVE_ENV_SESSION_PROVENANCE.to_owned();
    let created = matches!(
        bindings::attach_subagent(tx, &child_claim, &parent, &placement, at.utc)?,
        bindings::EnrollOutcome::Enrolled { .. }
    );
    // A child CLI call that beat this start event enrolled a stray top-level plain-call row;
    // the registered subagent row replaces it, whether just created or already there.
    bindings::supersede_plain_child_row(tx, &child_claim, at.utc)?;
    Ok(created)
}

/// A cooperative check-in constructs its bounded offer inside the deciding SQLite
/// transaction. An offer or encoding failure rolls back the anchor and frontier.
pub fn register_available(
    context: &StoreContext,
    conn: &mut Connection,
    command: &CheckIn,
    operator: Option<&OperatorActor>,
    mut permit: MutationPermit,
    budget: &CallBudget,
    build_offer: impl FnOnce(
        &Transaction<'_>,
        &SeatId,
        u64,
        super::super::first_contact::OfferHistory,
        &BindingId,
    ) -> Result<CheckInResult, ApiError>,
) -> Result<CommandResult, ApiError> {
    let claim = &command.claim;
    let digest = schema::canonical_digest(&check_in_payload(command))?;
    // The operator form never shares an idempotency key with the plain one.
    let idempotency_digest = if operator.is_some() {
        schema::canonical_digest(&("operator_check_in", &command.mode, &command.claim))?
    } else {
        digest
    };
    let scope = format!("seat:{}", claim.seat.as_str());
    schema::execute_budgeted_idempotent_transaction(
        context,
        conn,
        budget,
        &scope,
        command.operation.as_str(),
        idempotency_digest,
        |tx| cooperative_claim(tx, context, &claim.instance, claim).map(|_| ()),
        |tx| {
            let host = cooperative_claim(tx, context, &claim.instance, claim)?;
            cooperative_mapping(tx, host, claim, Some(command.mode)).map(|_| ())
        },
        |tx, at| {
            let mapping = decide_cooperative_fence(
                context,
                tx,
                at,
                &claim.instance,
                &mut permit,
                claim,
                Some(command.mode),
                &command.operation,
                &ObligationRef::CheckIn(claim.seat.clone()),
                &digest,
            )?;
            let seat = &claim.seat;
            let (terminal, incarnation) = mapping.reconfirmation_evidence()?;
            // Evaluated before the claim's binding is resolved, so a binding this very call
            // enrolls does not count. Availability follows liveness: only connected bindings count.
            let previously_available:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL AND liveness='connected' AND registered_at IS NOT NULL AND generation=?2 AND host_boot=?3 AND host_epoch=?4 AND target_generation=?5)",
                params![seat.as_str(),mapping.generation as i64,mapping.boot,mapping.epoch as i64,mapping.revision as i64],|r|r.get(0)).map_err(store_error)?;
            if !previously_available {
                schema::ensure_unavailability_episode(tx, seat)?;
            }
            let lifecycle = matches!(
                command.mode,
                crate::protocol::commands::CheckInMode::Lifecycle { .. }
                    | crate::protocol::commands::CheckInMode::Fork { .. }
            );
            let human = claim.harness == crate::protocol::authority::Harness::Human;
            if lifecycle
                && human
                && operator.is_none()
                && let Some(open) = bindings::list_connected(tx, seat)?
                    .into_iter()
                    .find(|binding| binding.harness != "human")
            {
                return Err(api_error(
                    ErrorCode::Unauthorized,
                    format!(
                        "seat {} is bound to a {} agent ({}); a person's check-in never replaces an agent's binding. Run it in your own shell pane, or override as the local account: `herdr-threads human me init --operator`",
                        seat.as_str(),
                        open.harness,
                        if open.provenance == crate::protocol::authority::MANAGED_LAUNCH_PROVENANCE
                        {
                            "launched, not checked in"
                        } else {
                            open.provenance.as_str()
                        }
                    ),
                ));
            }
            let resolution = resolve_cooperative_with_lineage(
                tx,
                context.processes(),
                at,
                claim,
                Some(command.mode),
                &mapping,
                command.lineage_parent.as_ref(),
                command.readonly,
            )?;
            // Read-only is decided from the resolved binding's role (a Hermes registered child
            // resolves to its subagent row); the transaction rolls back on Err.
            super::control::require_top_level_binding(resolution.binding())?;
            let binding_id = resolution.binding().binding_id.clone();
            if command.readonly {
                bindings::set_readonly(tx, &binding_id)?;
            }
            // Spec §2: a new binding's first response carries its history stats (once: only the
            // check-in that enrolls it); a resumed or reconnected one whose unread backlog
            // exceeds one page is fast-forwarded. Never on a plain touch, and never for the
            // binding another check-in superseded (that binding does not run this path).
            let first_contact = match &resolution {
                bindings::Resolution::Enrolled {
                    first_contact: true,
                    ..
                } => Some(super::super::first_contact::first_contact(
                    tx,
                    seat,
                    &|thread| {
                        super::super::read_cursors::cursor_for(tx, &binding_id, thread)
                            .ok()
                            .flatten()
                            .unwrap_or(0)
                    },
                )?),
                _ => None,
            }
            .filter(|fc| !fc.threads.is_empty());
            let fast_forward = match &resolution {
                bindings::Resolution::Existing {
                    reconnected: true, ..
                }
                | bindings::Resolution::Reopened(_) => {
                    super::super::first_contact::fast_forward(tx, &binding_id)?
                }
                _ => None,
            };
            // The seat's mapping generation: check-ins add and refresh bindings, they never
            // bump it (only rebind, unresolve and retire do).
            let generation = mapping.generation as i64;
            if lifecycle {
                // TRUST-POLICY A4: a person and an agent never hold a seat together. An agent's
                // check-in already ended the seat's human binding and any launch placeholder
                // inside `resolve_cooperative`; a person's ends the seat's agent bindings (the
                // connected ones only under the operator override, refused above otherwise).
                if human {
                    bindings::end_agent_bindings(tx, seat, at.utc)?;
                }
                // Only ended bindings' catch-up rows end: another binding's check-in leaves an
                // open binding's catch-up alone (ht-54r.22).
                catch_up::supersede_ended_bindings(tx, seat, at.utc)?;
                if let Some(actor) = operator {
                    tx.execute("INSERT INTO allocation_decisions(host_id,target_id,seat_id,kind,decided_at,host_boot,epoch,generation,operator_label) VALUES ((SELECT host_id FROM seats WHERE id=?2),?1,?2,'operator_human_override',?3,?4,?5,?6,?7)",
                        params![claim.target.as_str(),seat.as_str(),at.utc.0,mapping.boot,mapping.epoch as i64,mapping.revision as i64,actor.audit_label()]).map_err(store_error)?;
                }
                // A lifecycle check-in starts this session's offer frontier over; the seat's other
                // bindings keep theirs (ht-54r.9).
                tx.execute(
                    "DELETE FROM warning_offer WHERE binding_id=?1",
                    [binding_id.as_str()],
                )
                .map_err(store_error)?;
                tx.execute(
                    "DELETE FROM in_turn_offer WHERE binding_id=?1",
                    [binding_id.as_str()],
                )
                .map_err(store_error)?;
            } else {
                // Restoring availability does not replace the occupant or its generation, and a plain
                // call never rewrites the recorded observation provenance (ht-54r.79).
                // The placement write can make the binding count (or re-rank the staged
                // provenance) whatever `previously_available` said: fence it on its own effect.
                schema::fence_seat_availability(tx, seat.as_str(), || {
                    tx.execute("UPDATE occupant_bindings SET registered_at=?1,host_boot=?2,host_epoch=?3,target_generation=?4,terminal_id=?6,incarnation=?7 WHERE binding_id=?5",
                        params![at.utc.0,mapping.boot,mapping.epoch as i64,mapping.revision as i64,binding_id.as_str(),terminal,incarnation]).map_err(store_error)
                        .map(|rows| ((), rows != 0))
                })?;
            }
            let seq = schema::next_decision_seq(tx, &claim.instance)?;
            if lifecycle && claim.harness == crate::protocol::authority::Harness::Human {
                // One durable cutoff waives all still-pending seat obligations
                // at this human check-in. No receipt is marked ACKed.
                tx.execute("INSERT INTO human_receipt_waivers(seat_id,through_decision_seq,human_generation,decided_at) VALUES (?1,?2,?3,?4) ON CONFLICT(seat_id) DO UPDATE SET through_decision_seq=MAX(human_receipt_waivers.through_decision_seq,excluded.through_decision_seq),human_generation=excluded.human_generation,decided_at=excluded.decided_at",
                    params![seat.as_str(),seq as i64,generation,at.utc.0]).map_err(store_error)?;
                schema::clear_waived_receipt_conditions_for_seat(tx, seat.as_str(), at.utc)?;
                let (prepared_high_water, physical_high_water): (i64,i64) = tx.query_row("SELECT COALESCE((SELECT MAX(ordinal) FROM prepared_recipients WHERE seat_id=?1),0),COALESCE((SELECT MAX(ordinal) FROM receipts WHERE seat_id=?1),0)",
                    [seat.as_str()], |r| Ok((r.get(0)?,r.get(1)?))).map_err(store_error)?;
                tx.execute("INSERT INTO human_receipt_reconciliation_bounds(seat_id,prepared_high_water,physical_high_water,decision_seq) VALUES (?1,?2,?3,?4) ON CONFLICT(seat_id) DO UPDATE SET prepared_high_water=excluded.prepared_high_water,physical_high_water=excluded.physical_high_water,decision_seq=excluded.decision_seq",
                    params![seat.as_str(),prepared_high_water,physical_high_water,seq as i64]).map_err(store_error)?;
                tx.execute("INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES (?1,'human_receipt_reconciliation',?2,?3) ON CONFLICT(kind,subject_id) DO UPDATE SET position=0,high_water=excluded.high_water,status='pending',last_error=NULL,completed_at=NULL",
                    params![format!("work:human-receipts:{}",seat.as_str()),seat.as_str(),prepared_high_water.max(physical_high_water)]).map_err(store_error)?;
            }
            if lifecycle || !previously_available {
                tx.execute("INSERT INTO seat_availability(seat_id,decision_seq,decision_at,binding_generation,observation_provenance) VALUES (?1,?2,?3,?4,?5)",
                    params![seat.as_str(),seq as i64,at.utc.0,generation,claim.harness.cooperative_provenance()]).map_err(store_error)?;
                let anchor = tx.last_insert_rowid();
                let high_water: i64 = tx
                    .query_row(
                        "SELECT COALESCE(MAX(ordinal),0) FROM prepared_recipients WHERE seat_id=?1",
                        [seat.as_str()],
                        |r| r.get(0),
                    )
                    .map_err(store_error)?;
                tx.execute("INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES (?1,'receipt_timer_materialization',?2,?3)",params![format!("receipt-timer:{anchor}"),anchor.to_string(),high_water]).map_err(store_error)?;
            }
            schema::clear_open_unavailability_for_seat(tx, seat.as_str(), at.utc)?;
            tx.execute(
                "UPDATE seats SET unavailability_open=0 WHERE id=?1",
                [seat.as_str()],
            )
            .map_err(store_error)?;
            schema::apply_eligibility_transition(tx, &claim.instance, |_| {
                Ok(lifecycle || !previously_available)
            })?;
            let mut offer = build_offer(
                tx,
                seat,
                seq,
                super::super::first_contact::OfferHistory {
                    first_contact,
                    fast_forward,
                },
                &binding_id,
            )?;
            if offer.seat != *seat {
                return Err(api_error(
                    ErrorCode::StoreCorrupt,
                    "check-in offer seat mismatch",
                ));
            }
            offer.context = claim.clone();
            offer.context.binding_generation = generation as u64;
            offer
                .inbox
                .validate()
                .map_err(|e| api_error(ErrorCode::InvalidBudget, e))?;
            tx.execute("INSERT INTO warning_offer(binding_id,seat_id,offered_through_seq) VALUES (?1,?2,?3) ON CONFLICT(binding_id) DO UPDATE SET offered_through_seq=MAX(warning_offer.offered_through_seq,excluded.offered_through_seq)",
                params![binding_id.as_str(),seat.as_str(),seq as i64]).map_err(store_error)?;
            // Lifecycle and Current offers alike settle only the notices
            // they carry (wave-2 fix2 root decision (a)), on this binding's own frontier.
            super::super::attention::settle_carried_notices(
                tx,
                seat.as_str(),
                &binding_id,
                &offer.notices.items,
            )?;
            Ok(CommandResult::CheckedIn(offer))
        },
        present_check_in_result,
    )
}

/// TRUST-POLICY A3 `managed_launch` (ht-5n6): after `launch` observed Herdr's
/// guarded start correlate with its request, open an unregistered occupant
/// binding for the started harness so the seat has the open binding wake
/// discovery requires, before the agent's first check-in. Decided in one
/// transaction against the canonical view (A2): the seat must be resolved
/// onto the launch target with no recovery hold, the effective observation
/// must be current for the instance's host boot and epoch and the seat's
/// target generation, and the launcher's terminal, Herdr incarnation, boot and
/// target generation must equal it. The placeholder is one more binding next to
/// any the seat already holds (ht-54r.4): it keeps the seat generation, carries
/// the started process's liveness fingerprint and placeholder session and
/// execution ids no caller claim can match, has no `registered_at` and starts no
/// availability, receipt timer or wake binding generation: it authorizes
/// nothing but a wake prompt to the bound harness. The launched agent's first
/// lifecycle check-in replaces only the placeholders of its own harness
/// (`bindings::launch_placeholder_replace`).
pub fn record_managed_launch(
    context: &StoreContext,
    conn: &mut Connection,
    instance: &str,
    command: &crate::protocol::commands::RecordManagedLaunch,
) -> Result<CommandResult, ApiError> {
    command
        .validate()
        .map_err(|e| api_error(ErrorCode::InvalidRequest, e))?;
    let target_generation = checked_host_number(command.target_generation)?;
    context.execute_decision(
        conn,
        |tx| {
            type SeatColumns = Option<(String, Option<String>, i64, i64, Option<String>, i64, i64)>;
            let row: SeatColumns = tx
                .query_row(
                    "SELECT s.state,s.target_id,s.generation,s.target_generation,h.host_boot,h.host_epoch,h.host_id FROM seats s JOIN herdr_hosts h ON h.host_id=s.host_id WHERE s.id=?1 AND s.instance_id=?2",
                    params![command.seat.as_str(), instance],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
                )
                .optional()
                .map_err(store_error)?;
            let Some((state, target, generation, seat_target_generation, boot, epoch, host_id)) = row else {
                return Err(api_error(ErrorCode::NotFound, "launched seat not found"));
            };
            if state != "resolved" || target.as_deref() != Some(command.target.as_str()) {
                return Err(api_error(
                    ErrorCode::TargetUnresolved,
                    "launched seat is no longer resolved onto the launch pane",
                ));
            }
            let host = HostId::from_row(host_id);
            let held: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM recovery_holds WHERE host_id=?1 AND target_id=?2 AND released_at IS NULL)",
                    params![host.get(), command.target.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            let observation =
                effective::effective_observation(tx, host, command.target.as_str())?;
            let Some(observation) = observation.filter(|o| {
                !held
                    && o.structural_generation == seat_target_generation
                    && Some(o.host_boot.as_str()) == boot.as_deref()
                    && o.epoch == epoch
            }) else {
                return Err(api_error(
                    ErrorCode::StaleHostObservation,
                    "launch pane is held or its observation is not current",
                ));
            };
            let (terminal, incarnation) = binding_evidence(
                observation.terminal_id.as_deref(),
                observation.incarnation.as_deref(),
            )?;
            if terminal != command.terminal.as_str()
                || incarnation != command.incarnation
                || observation.host_boot != command.host_boot.as_str()
                || observation.structural_generation != target_generation
            {
                return Err(api_error(
                    ErrorCode::StaleHostObservation,
                    "launch evidence differs from the current pane observation",
                ));
            }
            Ok((
                generation,
                seat_target_generation,
                observation.host_boot.clone(),
                epoch,
                terminal.to_owned(),
                incarnation.to_owned(),
            ))
        },
        |tx, at, (generation, seat_target_generation, boot, epoch, terminal, incarnation)| {
            // The seat may already hold agent bindings, connected or not: the placeholder is one
            // more binding, keyed by its own `launch:` session, and changes none of them. The seat
            // generation stays: only the launched agent's own check-in replaces the placeholder.
            let placeholder = crate::protocol::ids::NativeSessionId::new(format!(
                "{}{}",
                crate::protocol::authority::MANAGED_LAUNCH_PLACEHOLDER_PREFIX,
                uuid::Uuid::new_v4()
            ));
            let key = bindings::BindingKey {
                seat: command.seat.clone(),
                harness: command.harness.as_str().to_owned(),
                native_session: placeholder.clone(),
                agent_id: None,
            };
            let placement = bindings::BindingPlacement {
                generation: u64::try_from(generation).map_err(|_| {
                    api_error(ErrorCode::StoreCorrupt, "negative seat generation")
                })?,
                target_generation: u64::try_from(seat_target_generation).map_err(|_| {
                    api_error(ErrorCode::StoreCorrupt, "negative target generation")
                })?,
                target: command.target.as_str().to_owned(),
                terminal,
                incarnation,
                host_boot: boot,
                host_epoch: u64::try_from(epoch)
                    .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative host epoch"))?,
                execution: placeholder.as_str().to_owned(),
                provenance: crate::protocol::authority::MANAGED_LAUNCH_PROVENANCE.to_owned(),
                observed_at: at.utc,
                registered_at: None,
            };
            let outcome = bindings::enroll(
                tx,
                &bindings::EnrollRequest {
                    key: &key,
                    role: bindings::BindingRole::TopLevel,
                    parent: None,
                    lineage: None,
                    fingerprint: Some(command.fingerprint),
                    placement: &placement,
                    trigger: bindings::EnrollTrigger::Lifecycle,
                },
                at.utc,
            )?;
            let (bindings::EnrollOutcome::Enrolled { binding, .. }
            | bindings::EnrollOutcome::Existing(binding)) = outcome;
            Ok(CommandResult::ManagedLaunchRecorded(
                crate::protocol::results::ManagedLaunchRecord {
                    seat: command.seat.clone(),
                    recorded: true,
                    binding_generation: binding.generation,
                    provenance: crate::protocol::authority::MANAGED_LAUNCH_PROVENANCE.into(),
                },
            ))
        },
    )
}

/// The hook pull's plain call (spec §3): resolves `claim` with the plain trigger (the same
/// touch-or-reconnect a CLI `inbox` performs, re-adopting a V32-migrated row with its placement and
/// cursors) and reads the seat's mod view of the claim's binding, in one write transaction.
/// Records no operation: a plain touch-or-reconnect is idempotent on its own.
pub fn hook_plain_reconnect(
    context: &StoreContext,
    conn: &mut Connection,
    instance: &str,
    claim: &crate::protocol::authority::CallerClaim,
    presentation: bool,
) -> Result<Option<crate::ports::ModSeatView>, ApiError> {
    context.execute_decision(
        conn,
        |tx| {
            let host = cooperative_claim(tx, context, instance, claim)?;
            cooperative_mapping(tx, host, claim, None)
        },
        |tx, at, mapping| {
            resolve_cooperative(tx, context.processes(), at, claim, None, &mapping)?;
            let view = if presentation {
                presentation_seat_view
            } else {
                mod_binding_view
            };
            view(
                tx,
                instance,
                &claim.seat,
                crate::ports::ModBindingRef::Claim(claim),
            )
        },
    )
}

/// Bound of the lazy pending count in a mod fingerprint (a change detector,
/// not a count anyone presents).
const MOD_LAZY_COUNT_CAP: i64 = 1000;

/// The seat's A2 state for the mod watch channel (spec D2), read-only, with one of its bindings
/// (spec §3 Wake). Call it inside the caller's read transaction. `None`: no such seat in
/// `instance`. The fingerprint is built from the canonical digest (so it moves with every
/// attention source the digest knows), with warnings narrowed to those that wake the seat
/// (`attention::wake_warnings`), plus the binding's own unread: ordinary mail past its cursors
/// and its lazy rows. `binding` is `None` when the reference resolves to no binding of this seat;
/// an ended or disconnected binding is returned as it is, with no unread.
pub fn mod_binding_view(
    db: &Connection,
    instance: &str,
    seat: &SeatId,
    binding: crate::ports::ModBindingRef<'_>,
) -> Result<Option<crate::ports::ModSeatView>, ApiError> {
    mod_seat_view_mode(db, instance, seat, binding, true)
}
pub fn presentation_seat_view(
    db: &Connection,
    instance: &str,
    seat: &SeatId,
    binding: crate::ports::ModBindingRef<'_>,
) -> Result<Option<crate::ports::ModSeatView>, ApiError> {
    mod_seat_view_mode(db, instance, seat, binding, false)
}
fn mod_seat_view_mode(
    db: &Connection,
    instance: &str,
    seat: &SeatId,
    binding: crate::ports::ModBindingRef<'_>,
    legacy_digest: bool,
) -> Result<Option<crate::ports::ModSeatView>, ApiError> {
    use crate::ports::{ModBindingRef, ModBindingView, ModFingerprint, ModSeatView};
    let row: Option<(String, Option<String>)> = db
        .query_row(
            "SELECT state,target_id FROM seats WHERE id=?1 AND instance_id=?2",
            params![seat.as_str(), instance],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(store_error)?;
    let Some((state, target)) = row else {
        return Ok(None);
    };
    let held = match target {
        Some(target) => db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM recovery_holds WHERE host_id=(SELECT host_id FROM seats WHERE id=?1) AND target_id=?2 AND released_at IS NULL)",
                params![seat.as_str(), target],
                |r| r.get::<_, bool>(0),
            )
            .map_err(store_error)?,
        None => false,
    };
    let row = match binding {
        ModBindingRef::Id(id) => super::bindings::get(db, id)?,
        ModBindingRef::Claim(claim) => {
            super::bindings::match_claim(db, &super::bindings::BindingKey::from_claim(claim))?
        }
    }
    .filter(|row| &row.seat == seat);
    let readonly = match &row {
        Some(row) => super::bindings::is_readonly(db, &row.binding_id)?,
        None => false,
    };
    let binding = row.map(|row| ModBindingView {
        readonly,
        generation: row.generation,
        provenance: row.provenance,
        harness: row.harness,
        native_session: row.native_session,
        top_level: row.role == super::bindings::BindingRole::TopLevel,
        connected: row.liveness == super::bindings::Liveness::Connected,
        ended_at: row.ended_at,
        disconnect_reason: row.disconnect_reason,
        binding_id: row.binding_id,
    });
    let attention_version: i64 = db
        .query_row(
            "SELECT attention_version FROM wake_work WHERE seat_id=?1",
            [seat.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?
        .unwrap_or(0);
    let attention_version = u64::try_from(attention_version)
        .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative attention version"))?;
    if !legacy_digest {
        return Ok(Some(ModSeatView {
            seat: seat.clone(),
            retired: state == "retired",
            continuity_resolved: state == "resolved",
            state,
            held,
            attention_version,
            fingerprint: ModFingerprint {
                attention_version,
                binding_generation: binding.as_ref().map(|b| b.generation),
                pending_receipts: 0,
                max_pending_ordinal: 0,
                lazy_pending: 0,
                max_lazy_rowid: 0,
                unread: 0,
                max_unread_rowid: 0,
                other_pending: 0,
            },
            binding,
            newest_pending_notice: None,
            pending_invitation_version: 0,
        }));
    }
    let run = super::super::attention::seat_digest(db, instance, seat, &|| Ok(()))?;
    // The binding's own unread. Published rows only: a staged row is invisible to the mod and
    // its publication (manifest + message) must change the fingerprint. An ended or
    // disconnected binding holds no channel and counts none.
    let (lazy_pending, max_lazy_rowid, unread, max_unread_rowid) =
        match binding.as_ref().filter(|held| held.holds_channel()) {
            Some(held) => {
                let lazy = super::super::lazy_delivery::binding_lazy_unread(
                    db,
                    &held.binding_id,
                    MOD_LAZY_COUNT_CAP as usize,
                )?;
                let (unread, max_unread_rowid) = super::super::read_cursors::binding_unread_stats(
                    db,
                    &held.binding_id,
                    MOD_LAZY_COUNT_CAP,
                )?;
                (
                    lazy.len() as u64,
                    lazy.iter().map(|row| row.ordinal).max().unwrap_or(0),
                    unread,
                    max_unread_rowid,
                )
            }
            None => (0, 0, 0, 0),
        };
    let digest = &run.digest;
    // Only warnings that would wake the seat natively raise mod attention
    // (TRUST-POLICY A7): another seat's overdue transition, and every clear,
    // is an informational notice that waits for the next check-in. The
    // saturated-walk conservative rule is the native wake's.
    let (waking_warnings, _) =
        super::super::attention::wake_warnings(db, seat.as_str(), run.warnings)?.count();
    let fingerprint = ModFingerprint {
        attention_version,
        pending_receipts: digest.receipts.count,
        max_pending_ordinal: digest
            .token
            .receipt
            .map_or(0, |(seq, _)| i64::try_from(seq).unwrap_or(i64::MAX)),
        lazy_pending,
        max_lazy_rowid,
        unread,
        max_unread_rowid,
        other_pending: super::super::attention::pending_confirming_invitations(
            db,
            seat.as_str(),
            None,
            i64::MAX,
        )?
        .count()
        .0 + waking_warnings
            + super::super::attention::seat_alerts(db, seat.as_str())?.0,
        binding_generation: binding.as_ref().map(|b| b.generation),
    };
    // Notices above the binding's own frontier (spec §2 Hook notifications); the seat view's
    // when the reference resolved to no binding.
    let newest_pending_notice = match binding.as_ref() {
        Some(held) => super::super::attention::binding_newest_pending_notice(db, &held.binding_id)?,
        None => super::super::attention::newest_pending_notice(db, seat.as_str())?,
    };
    Ok(Some(ModSeatView {
        seat: seat.clone(),
        retired: state == "retired",
        continuity_resolved: state == "resolved",
        state,
        held,
        binding,
        attention_version,
        fingerprint,
        newest_pending_notice,
        pending_invitation_version: run.invitations.content_version(),
    }))
}

/// Head of the stall query over the seat's published pending delivery rows (aliases `r`, `m`, `s`), driven
/// from the connected top-level bindings' pending views and the seat's legacy rows; never the seat-wide
/// `state`. The threshold predicate (`THRESHOLD_ADMITS_SQL`, joined by `format!`) and `STALL_ROWS_TAIL`
/// follow.
const STALL_ROWS_HEAD: &str = "SELECT min(m.decision_at) FROM lazy_recipients r JOIN seats s ON s.id=r.seat_id JOIN messages m ON m.id=r.message_id WHERE r.ordinal IN (SELECT p.ordinal FROM occupant_bindings vb JOIN binding_pending p ON p.binding_id=vb.binding_id WHERE vb.seat_id=?1 AND vb.ended_at IS NULL AND vb.role='top_level' AND vb.liveness='connected' UNION SELECT lr.ordinal FROM lazy_recipients lr INDEXED BY lazy_recipients_legacy WHERE lr.seat_id=?1 AND lr.legacy_pending=1) AND r.seat_id=?1 AND r.decision_seq IS NOT NULL AND m.kind='ordinary' AND length(CAST(m.body AS BLOB))<=?2 AND m.decision_at<=?3 AND ";
/// Tail: the row is unread for some connected top-level binding (`seat_unread_sql!`).
const STALL_ROWS_TAIL: &str = concat!(" AND ", crate::store::lazy_delivery::seat_unread_sql!());

/// The oldest publication time among the seat's wake-class pending items:
/// ordinary receipts (the digest's bounded receipt walk, catch-up held items
/// excluded) and delivery rows its current nudge threshold admits. Only items
/// published at or before `at_or_before` whose stored body is at most
/// `body_limit` bytes count (spec D7: a truncated item never counts toward a
/// stall).
pub fn mod_stall_oldest(
    db: &Connection,
    seat: &SeatId,
    at_or_before: UtcMillis,
    body_limit: usize,
) -> Result<Option<UtcMillis>, ApiError> {
    let pending = super::super::attention::pending_receipts(db, seat.as_str(), None)?;
    let mut oldest: Option<i64> = None;
    for item in &pending.items {
        let published: Option<i64> = db
            .query_row(
                "SELECT decision_at FROM messages WHERE id=?1 AND kind='ordinary' AND length(CAST(body AS BLOB))<=?2 AND decision_at<=?3",
                params![
                    item.id,
                    i64::try_from(body_limit).unwrap_or(i64::MAX),
                    at_or_before.0
                ],
                |r| r.get(0),
            )
            .optional()
            .map_err(store_error)?;
        if let Some(at) = published {
            oldest = Some(oldest.map_or(at, |current| current.min(at)));
        }
    }
    let rows: Option<i64> = db
        .query_row(
            &format!("{STALL_ROWS_HEAD}{THRESHOLD_ADMITS_SQL}{STALL_ROWS_TAIL}"),
            params![
                seat.as_str(),
                i64::try_from(body_limit).unwrap_or(i64::MAX),
                at_or_before.0
            ],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    let oldest = match (oldest, rows) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    Ok(oldest.map(UtcMillis))
}

pub(crate) fn present_check_in_result(
    tx: &Transaction<'_>,
    mut result: CommandResult,
) -> Result<CommandResult, ApiError> {
    if let CommandResult::CheckedIn(offer) = &mut result {
        let claim = &offer.context;
        // Current: the seat has not been rebound since the claim (`seats.generation`, the
        // fence) and the claim's own binding, found by key, is open on the seat's target. A
        // seat holds many bindings, so no binding generation or execution is compared
        // (ht-54r.3).
        let current:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM seats s JOIN occupant_bindings b ON b.seat_id=s.id WHERE s.id=?1 AND s.instance_id=?2 AND s.state!='retired' AND s.target_id=?3 AND s.generation=?4 AND b.ended_at IS NULL AND b.target_id=s.target_id AND b.harness=?5 AND b.native_session=?6 AND b.agent_id=?7)",
            params![claim.seat.as_str(),claim.instance,claim.target.as_str(),claim.binding_generation as i64,claim.harness.as_str(),claim.native_session.as_str(),claim.agent_id.as_ref().map_or("",|a|a.as_str())],|r|r.get(0)).map_err(store_error)?;
        offer.context_disposition = if current {
            crate::protocol::results::CheckInContextDisposition::Current
        } else {
            crate::protocol::results::CheckInContextDisposition::Historical
        };
    }
    Ok(result)
}

#[cfg(test)]
#[path = "../../../tests/store/mod_seat_view.rs"]
mod mod_seat_view_tests;

#[cfg(test)]
#[path = "../../../tests/store/live_holder.rs"]
mod live_holder_tests;
