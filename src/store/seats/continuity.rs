//! Continuity candidates and `decide_continuity`.

use crate::{
    protocol::{
        authority::DecisionFence,
        ids::{ExecutionId, HostBootId, HostTargetId, SeatId},
        results::{ApiError, CommandResult, ErrorCode},
        time::CallBudget,
    },
    store::{
        bindings, catch_up,
        connection::{StoreContext, api_error, store_error},
        effective::{self, EffectiveObservationSource},
        schema,
    },
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::*;

/// TRUST-POLICY C1 candidates: nonretired unresolved **local** unnamed seats (a named seat's
/// continuity is its name, `named_seat_apply`, and it never rebinds onto a pane) of the instance, whatever
/// Herdr server they last sat on (a remote seat is never a candidate, so C1 never moves a seat
/// between nodes), whose bindings include this harness's session `native_session` (any
/// top-level row, open or ended, not only the newest: a one-shot or `/clear`
/// successor enrolled later must not hide the main session, ht-54r.78).
/// Sentinel (`plugin_context:`), empty and human values never match, and a
/// `managed_launch` binding (no session: the agent never checked in)
/// never counts (ht-5n6); NULL
/// never equals anything. When several seats match, only those on which a
/// matching row ended `seat_unresolved` count, if there are any: that reason
/// is written only to a row still connected when its seat became unresolved,
/// so a row that had already disconnected for another reason is recorded
/// evidence that the session had left that seat (ht-54r.91). Returns at most
/// two, in seat-id order (the caller only needs to tell zero, one and
/// several apart).
pub(crate) fn continuity_candidates(
    tx: &Transaction<'_>,
    instance: &str,
    harness: &str,
    native_session: &str,
) -> Result<Vec<SeatId>, ApiError> {
    let mut statement = tx
        .prepare(&format!(
            "SELECT s.id, COALESCE(MAX(b.disconnect_reason IS 'seat_unresolved'),0) FROM seats s JOIN occupant_bindings b ON b.seat_id=s.id WHERE s.instance_id=?1 AND s.{LOCAL_SEAT} AND s.state='unresolved' AND s.name IS NULL AND {C1_BINDING} GROUP BY s.id ORDER BY s.id",
        ))
        .map_err(store_error)?;
    let rows: Vec<(String, bool)> = statement
        .query_map(params![instance, harness, native_session], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .map_err(store_error)?
        .collect::<Result<_, _>>()
        .map_err(store_error)?;
    let live_at_unresolve = rows.iter().any(|(_, live)| *live);
    Ok(rows
        .into_iter()
        .filter(|(_, live)| *live || !live_at_unresolve)
        .map(|(seat, _)| SeatId::new(seat))
        .take(2)
        .collect())
}

/// The C1 candidate binding of seat `s` for harness `?2` and native session
/// `?3`: [`continuity_candidates`] and [`session_may_become_candidate`] share
/// it, so a lagging claim is final exactly when no seat could match it.
const C1_BINDING: &str = "b.harness=?2 AND b.harness<>'human' AND b.role='top_level' AND b.agent_id='' AND b.observation_provenance<>'managed_launch' AND b.native_session=?3 AND b.native_session<>'' AND substr(b.native_session,1,15)<>'plugin_context:'";

/// Whether a reconciliation pass could still make some seat a C1 candidate
/// for this session: a nonretired local seat has a [`C1_BINDING`] row, which
/// [`continuity_candidates`] would match once that seat is unresolved. A pass
/// adds no binding, so without one the no-match is final (ht-kaw9).
fn session_may_become_candidate(
    tx: &Transaction<'_>,
    instance: &str,
    harness: &str,
    native_session: &str,
) -> Result<bool, ApiError> {
    tx.query_row(
        &format!(
            "SELECT EXISTS(SELECT 1 FROM seats s JOIN occupant_bindings b ON b.seat_id=s.id WHERE s.instance_id=?1 AND s.{LOCAL_SEAT} AND s.state<>'retired' AND s.name IS NULL AND {C1_BINDING})",
        ),
        params![instance, harness, native_session],
        |r| r.get(0),
    )
    .map_err(store_error)
}

/// Schema delta 12 (ht-hsyg): copy every seat-history decision into the table that admits the
/// `structural_process_continuity` kind. Ordinals are kept, so the AUTOINCREMENT sequence follows.
pub(crate) fn copy_allocation_decisions(conn: &Connection) -> Result<(), ApiError> {
    conn.execute(
        "INSERT INTO allocation_decisions_new(ordinal,host_id,target_id,seat_id,kind,decided_at,host_boot,epoch,generation,operator_label,continuity_diagnostic) \
         SELECT ordinal,host_id,target_id,seat_id,kind,decided_at,host_boot,epoch,generation,operator_label,continuity_diagnostic FROM allocation_decisions ORDER BY ordinal",
        [],
    )
    .map_err(store_error)?;
    Ok(())
}

/// Candidate rows one C1 process continuity page lists (ht-hsyg); the caller pages on.
pub const PROCESS_CONTINUITY_CANDIDATES_PER_PASS: usize = 32;

/// TRUST-POLICY C1 process continuity (ht-hsyg): the unresolved local seats of `host` that a
/// new incarnation left at a saved target, each with the top-level harness bindings its latest
/// unresolve ended while open (`seat_unresolved`, placed at the generation the unresolve bumped
/// from) under another Herdr incarnation than the host's published one, carrying a liveness
/// fingerprint. The same binding filter as [`continuity_candidates`]: never human,
/// `managed_launch`, a subagent or a `plugin_context:` session. Seat order (each seat's newest
/// binding first), after `page.after`; bounded by [`PROCESS_CONTINUITY_CANDIDATES_PER_PASS`]
/// rows, so a caller pages on, except a listing of the seats saved at `page.target`, which is
/// complete.
pub fn process_continuity_candidates(
    context: &StoreContext,
    instance: &str,
    host: crate::ports::HostId,
    page: &crate::ports::ProcessContinuityPage,
    budget: &CallBudget,
) -> Result<crate::ports::ProcessContinuityListing, ApiError> {
    let db = context.open_query(budget.clone())?;
    // A target listing (the rivals at one pane) is complete: one target's rows, never cut.
    let limit = match page.target {
        Some(_) => -1,
        None => i64::try_from(PROCESS_CONTINUITY_CANDIDATES_PER_PASS).unwrap_or(i64::MAX),
    };
    let mut statement = db
        .prepare(&format!(
            "SELECT s.id,s.generation,b.binding_id,s.target_id,h.endpoint,b.harness,b.native_session,b.execution_id,b.liveness_pid,b.liveness_start \
             FROM seats s JOIN occupant_bindings b ON b.seat_id=s.id JOIN herdr_hosts h ON h.host_id=s.host_id \
             JOIN snapshot_generations g ON g.id=h.active_snapshot_id AND g.status='published' \
             WHERE s.instance_id=?1 AND s.host_id=?2 AND s.{LOCAL_SEAT} AND s.state='unresolved' AND s.unresolved_reason='other' AND s.target_id IS NOT NULL AND s.name IS NULL \
             AND b.disconnect_reason='seat_unresolved' AND b.generation=s.generation-1 AND b.incarnation IS NOT NULL AND b.incarnation<>g.incarnation \
             AND b.harness<>'human' AND b.role='top_level' AND b.agent_id='' AND b.observation_provenance<>'managed_launch' \
             AND b.native_session<>'' AND substr(b.native_session,1,15)<>'plugin_context:' AND b.liveness_pid IS NOT NULL AND b.liveness_start IS NOT NULL \
             AND (?4 IS NULL OR s.id>?4) AND (?5 IS NULL OR s.target_id=?5) \
             ORDER BY s.id, b.ordinal DESC LIMIT ?3",
        ))
        .map_err(|e| db.map_error(e))?;
    let rows = statement
        .query_map(
            params![
                instance,
                host.get(),
                limit,
                page.after.as_ref().map(SeatId::as_str),
                page.target.as_ref().map(HostTargetId::as_str)
            ],
            |r| -> rusqlite::Result<ProcessContinuityRow> {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, String>(7)?,
                    r.get::<_, i64>(8)?,
                    r.get::<_, i64>(9)?,
                ))
            },
        )
        .map_err(|e| db.map_error(e))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| db.map_error(e))?;
    let full = page.target.is_none() && rows.len() >= PROCESS_CONTINUITY_CANDIDATES_PER_PASS;
    let mut candidates = Vec::with_capacity(rows.len());
    for row in rows {
        if let Some(candidate) = process_continuity_candidate(row)? {
            candidates.push(candidate);
        }
    }
    Ok(crate::ports::ProcessContinuityListing { candidates, full })
}

type ProcessContinuityRow = (
    String,
    i64,
    String,
    String,
    String,
    String,
    String,
    String,
    i64,
    i64,
);

/// One candidate row; `None` for a harness this build does not know (it cannot be reattached
/// as one).
fn process_continuity_candidate(
    (seat, generation, binding, target, endpoint, harness, session, execution, pid, start): ProcessContinuityRow,
) -> Result<Option<crate::ports::ProcessContinuityCandidate>, ApiError> {
    let corrupt = |what: &str| api_error(ErrorCode::StoreCorrupt, format!("invalid {what}"));
    let Ok(harness) = serde_json::from_value::<crate::protocol::authority::Harness>(
        serde_json::Value::String(harness),
    ) else {
        return Ok(None);
    };
    Ok(Some(crate::ports::ProcessContinuityCandidate {
        seat: SeatId::parse(seat).map_err(|_| corrupt("seat id"))?,
        seat_generation: u64::try_from(generation).map_err(|_| corrupt("seat generation"))?,
        binding_id: binding,
        target: HostTargetId::parse(target).map_err(|_| corrupt("target id"))?,
        host: crate::protocol::ids::HostLocator::parse(endpoint)
            .map_err(|_| corrupt("host endpoint"))?,
        harness,
        native_session: crate::protocol::ids::NativeSessionId::parse(session)
            .map_err(|_| corrupt("native session"))?,
        execution: ExecutionId::parse(execution).map_err(|_| corrupt("execution id"))?,
        liveness: crate::protocol::authority::LivenessFingerprint {
            pid: u32::try_from(pid).map_err(|_| corrupt("liveness pid"))?,
            start: u64::try_from(start).map_err(|_| corrupt("liveness start"))?,
        },
    }))
}

pub fn continuity_digest(
    instance: &str,
    command: &crate::protocol::commands::ContinuityCheckIn,
) -> Result<[u8; 32], ApiError> {
    command
        .validate()
        .map_err(|detail| api_error(ErrorCode::InvalidRequest, detail))?;
    // Keep every committed resume digest replayable across this upgrade.
    // A new claim kind has its own domain, so changing it cannot replay an
    // operation recorded under the other kind.
    let domain =
        if command.source == crate::protocol::commands::ContinuityCheckIn::SAME_SESSION_SOURCE {
            "same_session_continuity_check_in"
        } else {
            "continuity_check_in"
        };
    schema::canonical_digest(&(
        domain,
        instance,
        &command.target,
        &command.harness,
        &command.native_session,
        &command.execution,
    ))
}

fn continuity_scope(instance: &str) -> String {
    format!("continuity:{instance}")
}

fn validate_continuity_result(result: &CommandResult) -> Result<(), ApiError> {
    if matches!(result, CommandResult::ContinuityReattached(_)) {
        Ok(())
    } else {
        Err(api_error(
            ErrorCode::StoreCorrupt,
            "continuity replay result has wrong type",
        ))
    }
}

/// Historical continuity result for the same operation key and payload. Grants
/// no current authority and touches no durable state.
pub fn replay_continuity(
    context: &StoreContext,
    instance: &str,
    command: &crate::protocol::commands::ContinuityCheckIn,
    budget: &CallBudget,
) -> Result<Option<CommandResult>, ApiError> {
    let expected = continuity_digest(instance, command)?;
    let db = context.open_query(budget.clone())?;
    let stored: Option<(Vec<u8>, String)> = db
        .query_row(
            "SELECT digest, result_json FROM operations WHERE actor_scope=?1 AND operation_key=?2",
            params![continuity_scope(instance), command.operation.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| db.map_error(e))?;
    let Some((digest, json)) = stored else {
        return Ok(None);
    };
    if digest != expected {
        return Err(api_error(
            ErrorCode::OperationPayloadMismatch,
            "operation key reused with different payload",
        ));
    }
    let result: CommandResult = serde_json::from_str(&json).map_err(|e| {
        api_error(
            ErrorCode::StoreCorrupt,
            format!("invalid stored result: {e}"),
        )
    })?;
    validate_continuity_result(&result)?;
    Ok(Some(result))
}

/// TRUST-POLICY C1: reattach the one unresolved seat one of whose bindings holds
/// the native session id onto the (held or unowned) target, in one deciding
/// transaction that is a complete lifecycle check-in. The match is cooperative
/// and structural only: Herdr's `agent_session` (`diagnostic`) is recorded,
/// never consulted. The seat's old binding is ended and the successor
/// `cooperative_top_level` binding (the command's harness, session and
/// execution, a new generation) is opened with its availability anchor in the
/// same transaction; the reply is idempotent under the operation key and a lost
/// reply is recovered by the committed binding. The `cooperative_continuity`
/// value is written to seat history (`allocation_decisions`) only, never to a
/// receipt or the binding. With no matching unresolved seat, or a target that
/// still has a resolved owner, while the host epoch's reconciliation lags its
/// recovery marker, the refusal is the retryable `ServiceBusy`, not `NotFound`
/// or `TargetAlreadyOwned`.
pub fn decide_continuity(
    context: &StoreContext,
    conn: &mut Connection,
    elected_instance: &str,
    mut request: crate::ports::ContinuityRequest,
) -> Result<CommandResult, ApiError> {
    let command = request.command.clone();
    let diagnostic = request.diagnostic;
    let (decision_kind, pinned) = match &request.evidence {
        crate::ports::ContinuityEvidence::Cooperative => (
            crate::protocol::authority::COOPERATIVE_CONTINUITY_PROVENANCE,
            None,
        ),
        crate::ports::ContinuityEvidence::Process {
            seat,
            seat_generation,
        } => (
            crate::protocol::authority::STRUCTURAL_PROCESS_CONTINUITY_PROVENANCE,
            Some((seat.clone(), checked_host_number(*seat_generation)?)),
        ),
    };
    let instance = request.guard.instance().to_owned();
    if instance != elected_instance {
        return Err(api_error(
            ErrorCode::StaleHostObservation,
            "continuity guard belongs to another instance",
        ));
    }
    command
        .validate()
        .map_err(|e| api_error(ErrorCode::InvalidRequest, e))?;
    // C1 does not go through `cooperative_claim`: the resumed session id is its evidence, and the
    // seat may sit on another server of this node (the move C1 exists for). The command's server
    // is the one it reattaches onto; an unattached one is `HostUnavailable`.
    let host = context.hosts().attached_id(&command.host)?;
    if request.guard.host_id() != host {
        return Err(api_error(
            ErrorCode::StaleHostObservation,
            "continuity guard belongs to another Herdr server",
        ));
    }
    let target = command.target.clone();
    // C1 process continuity (ht-hsyg): the evidence is about one seat's process at one
    // generation. The session's unique match must be that seat, unmoved; a kept claim on the
    // target is the session's own word and is decided by its own path first.
    let process_guard = |tx: &Transaction<'_>, seat: &SeatId| -> Result<(), ApiError> {
        let Some((judged, generation)) = &pinned else {
            return Ok(());
        };
        let current: Option<i64> = tx
            .query_row(
                "SELECT generation FROM seats WHERE id=?1 AND state='unresolved'",
                [judged.as_str()],
                |r| r.get(0),
            )
            .optional()
            .map_err(store_error)?;
        if seat != judged || current != Some(*generation) {
            return Err(api_error(
                ErrorCode::Conflict,
                "process continuity evidence no longer names the matching seat; no seat moved",
            ));
        }
        let kept: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM kept_continuity_claims WHERE host_id=?1 AND target_id=?2)",
                params![host.get(), target.as_str()],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        if kept {
            return Err(api_error(
                ErrorCode::ServiceBusy,
                "a kept continuity claim names this target and is decided first; retry",
            ));
        }
        Ok(())
    };
    let expected_boot = request.guard.host_boot().clone();
    let expected_epoch = request.guard.epoch();
    let expected_generation = request.guard.generation();
    let structural_proof = request.guard.structural_proof().cloned();
    let expected_epoch_sql = checked_host_number(expected_epoch)?;
    let expected_generation_sql = checked_host_number(expected_generation)?;
    let digest = continuity_digest(elected_instance, &command)?;
    let harness = command.harness.as_str();
    let session = command.native_session.as_str();
    let result = schema::execute_idempotent_transaction(
        context,
        conn,
        &continuity_scope(&instance),
        command.operation.as_str(),
        digest,
        |tx| {
            // Decision 9 / spec §3: a lifecycle check-in records its fingerprint. Checked here,
            // after the recorded-result lookup, so a replay of a committed reattachment is
            // still answered.
            if command.liveness.is_none() {
                return Err(api_error(
                    ErrorCode::LivenessFingerprintRequired,
                    "a native session's reattachment must carry its liveness fingerprint",
                ));
            }
            if !observed_matches(
                tx,
                host,
                target.as_str(),
                &expected_boot,
                expected_epoch,
                expected_generation,
            )? {
                return Err(api_error(
                    ErrorCode::StaleHostObservation,
                    "continuity target observation changed",
                ));
            }
            // Held or unowned only: a target with a resolved owner is never
            // taken (seats are never merged, nothing moves on a heuristic).
            // Until the first reconciliation pass of the current recovery
            // epoch has run, the owner may be a mapping from the previous
            // Herdr incarnation that the pass will unresolve (a restored
            // pane that kept its id, ht-p63): pending, not final.
            if !target_free(tx, host, target.as_str())? {
                return Err(if reconciliation_lags(tx, host)? {
                    api_error(
                        ErrorCode::ServiceBusy,
                        "recovery reconciliation of the current host epoch has not finished; retry",
                    )
                } else {
                    let owner: String = tx.query_row("SELECT id FROM seats WHERE host_id=?1 AND target_id=?2 AND state='resolved'", params![host.get(),target.as_str()], |row| row.get(0)).map_err(store_error)?;
                    let owner = shlex::try_quote(&owner)
                        .map_err(|_| api_error(ErrorCode::StoreCorrupt, "invalid owner id"))?;
                    let pane = shlex::try_quote(target.as_str())
                        .map_err(|_| api_error(ErrorCode::InvalidRequest, "invalid target"))?;
                    api_error(
                        ErrorCode::TargetAlreadyOwned,
                        format!(
                            "continuity target {pane} already belongs to {owner}; inspect `herdr-threads seat inspect {owner}` before operator repair; no seat moved"
                        ),
                    )
                });
            }
            if let Some(proof) = &structural_proof
                && !structural_proof_matches_current(tx, host, proof)?
            {
                return Err(api_error(
                    ErrorCode::StaleHostObservation,
                    "continuity structural proof differs from current evidence",
                ));
            }
            let candidates = continuity_candidates(tx, &instance, harness, session)?;
            let [seat] = candidates.as_slice() else {
                return Err(if candidates.is_empty() {
                    // Until the first reconciliation pass of the current
                    // recovery epoch has run, "no unresolved seat" only says the
                    // daemon has not yet rebuilt the seats a Herdr restart
                    // unresolved: pending, not final. A pass only unresolves
                    // seats, never adds bindings, so a session no seat's
                    // binding names can never become a candidate: its answer
                    // is final now, whatever the lag (ht-kaw9).
                    if reconciliation_lags(tx, host)?
                        && session_may_become_candidate(tx, &instance, harness, session)?
                    {
                        api_error(
                            ErrorCode::ServiceBusy,
                            "recovery reconciliation of the current host epoch has not finished; retry",
                        )
                    } else {
                        api_error(
                            ErrorCode::NotFound,
                            "no unresolved seat matches this native session",
                        )
                    }
                } else {
                    let pane = shlex::try_quote(target.as_str())
                        .map_err(|_| api_error(ErrorCode::InvalidRequest, "invalid target"))?;
                    let choices = candidates.iter().map(|seat| {
                        let seat = shlex::try_quote(seat.as_str()).map_err(|_| api_error(ErrorCode::StoreCorrupt,"invalid seat id"))?;
                        Ok(format!("inspect `herdr-threads seat inspect {seat}`; if this is its pane, `herdr-threads human seat rebind {seat} --pane {pane} --operator`"))
                    }).collect::<Result<Vec<_>,ApiError>>()?.join("; ");
                    api_error(
                        ErrorCode::Conflict,
                        format!(
                            "native session matches several unresolved seats (up to two shown); {choices}; no seat moved"
                        ),
                    )
                });
            };
            process_guard(tx, seat)?;
            let generation: i64 = tx
                .query_row(
                    "SELECT generation FROM seats WHERE id=?1 AND instance_id=?2",
                    params![seat.as_str(), instance],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            if generation == i64::MAX {
                return Err(api_error(
                    ErrorCode::SequenceExhausted,
                    "seat binding generation exhausted",
                ));
            }
            Ok(())
        },
        |tx, at| {
            let fresh = effective::effective_observation(tx, host, target.as_str())?
                .filter(|observation| {
                    observation.source == EffectiveObservationSource::NewerCurrentTarget
                })
                .ok_or_else(|| {
                    api_error(
                        ErrorCode::StaleHostObservation,
                        "fresh target observation missing",
                    )
                })?;
            request
                .guard
                .consume(
                    &instance,
                    &command,
                    &DecisionFence {
                        now: at.monotonic,
                        host_boot: HostBootId::new(fresh.host_boot),
                        host_epoch: u64::try_from(fresh.epoch).map_err(|_| {
                            api_error(ErrorCode::StoreCorrupt, "negative host epoch")
                        })?,
                        target_generation: u64::try_from(fresh.structural_generation).map_err(
                            |_| api_error(ErrorCode::StoreCorrupt, "negative target generation"),
                        )?,
                        binding_generation: 0,
                        known_invalidated: !observed_matches(
                            tx,
                            host,
                            target.as_str(),
                            &expected_boot,
                            expected_epoch,
                            expected_generation,
                        )?,
                    },
                )
                .map_err(|reason| api_error(ErrorCode::StaleHostObservation, reason))?;
            let candidates = continuity_candidates(tx, &instance, harness, session)?;
            let [seat] = candidates.as_slice() else {
                return Err(api_error(ErrorCode::Conflict, "continuity match changed"));
            };
            process_guard(tx, seat)?;
            if !target_free(tx, host, target.as_str())? {
                return Err(api_error(
                    ErrorCode::TargetAlreadyOwned,
                    "target became owned",
                ));
            }
            rebind_unresolved_seat(
                tx,
                &instance,
                host,
                seat,
                target.as_str(),
                expected_generation_sql,
                at.utc,
            )?;
            if let Some(proof) = &structural_proof {
                update_structural_proof(tx, seat, proof)?;
            } else {
                tx.execute("UPDATE seats SET structural_terminal_id=NULL,structural_incarnation=NULL,structural_incarnation_kind=NULL,structural_host_boot=NULL,structural_host_epoch=NULL,structural_connection_epoch=NULL,structural_observation_sequence=NULL WHERE id=?1", [seat.as_str()]).map_err(store_error)?;
            }
            let new_generation: i64 = tx
                .query_row(
                    "SELECT generation FROM seats WHERE id=?1",
                    [seat.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            // The successor binding opens here, so the reply is a complete
            // check-in: the seat is resolved, bound and available when this
            // transaction commits. Evidence is the structural proof when the
            // guard carries one, else the fresh observation of the same target.
            let (terminal, incarnation) = match &structural_proof {
                Some(proof) => (
                    proof.terminal().as_str().to_owned(),
                    proof.incarnation().to_owned(),
                ),
                None => {
                    let (terminal, incarnation) = binding_evidence(
                        fresh.terminal_id.as_deref(),
                        fresh.incarnation.as_deref(),
                    )?;
                    (terminal.to_owned(), incarnation.to_owned())
                }
            };
            let fingerprint = command.liveness.ok_or_else(|| {
                api_error(
                    ErrorCode::LivenessFingerprintRequired,
                    "a native session's reattachment must carry its liveness fingerprint",
                )
            })?;
            // ht-54r.78: the general resolver. The unresolve ended every binding of the seat, so
            // it reopens the session's own row ended `seat_unresolved` (cursors kept) or enrolls
            // one at the seat's read position; both run the enroll hooks.
            let claim = crate::protocol::authority::CallerClaim {
                instance: instance.clone(),
                host: command.host.clone(),
                seat: seat.clone(),
                binding_generation: u64::try_from(new_generation)
                    .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative seat generation"))?,
                role: crate::protocol::authority::CallerRole::TopLevel,
                harness: command.harness,
                native_session: command.native_session.clone(),
                execution: command.execution.clone(),
                target: target.clone(),
                agent_id: None,
                liveness: Some(fingerprint),
            };
            let placement = bindings::BindingPlacement {
                generation: claim.binding_generation,
                target_generation: expected_generation,
                target: target.as_str().to_owned(),
                terminal,
                incarnation,
                host_boot: expected_boot.as_str().to_owned(),
                host_epoch: expected_epoch,
                execution: command.execution.as_str().to_owned(),
                provenance: crate::protocol::authority::COOPERATIVE_TOP_LEVEL_PROVENANCE.to_owned(),
                observed_at: at.utc,
                registered_at: Some(at.utc),
            };
            let resolution = bindings::resolve_claim(
                tx,
                &bindings::ResolveRequest {
                    claim: &claim,
                    lifecycle: true,
                    fork: false,
                    placement: &placement,
                    lineage: None,
                },
                at.utc,
            )?;
            // Spec §2: a reopened session's cursors stayed where they were, so a backlog past
            // one page is fast-forwarded here, as `register_available` does for a reconnect. An
            // enrolled successor starts at the seat's read position and is not (its first-contact
            // stats are out of scope).
            let fast_forward = match &resolution {
                bindings::Resolution::Reopened(_)
                | bindings::Resolution::Existing {
                    reconnected: true, ..
                } => {
                    super::super::first_contact::fast_forward(tx, &resolution.binding().binding_id)?
                }
                _ => None,
            };
            catch_up::supersede_ended_bindings(tx, seat, at.utc)?;
            tx.execute("UPDATE wake_work SET binding_generation=?1 WHERE seat_id=?2 AND reservation_id IS NULL",
                params![new_generation,seat.as_str()]).map_err(store_error)?;
            let seq = schema::next_decision_seq(tx, &instance)?;
            anchor_seat_availability(
                tx,
                seat,
                seq,
                at.utc,
                new_generation,
                crate::protocol::authority::COOPERATIVE_TOP_LEVEL_PROVENANCE,
            )?;
            schema::clear_open_unavailability_for_seat(tx, seat.as_str(), at.utc)?;
            tx.execute(
                "UPDATE seats SET unavailability_open=0 WHERE id=?1",
                [seat.as_str()],
            )
            .map_err(store_error)?;
            tx.execute("INSERT INTO allocation_decisions(host_id,target_id,seat_id,kind,decided_at,host_boot,epoch,generation,operator_label,continuity_diagnostic) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,NULL,?9)",
                params![host.get(),target.as_str(),seat.as_str(),decision_kind,at.utc.0,expected_boot.as_str(),expected_epoch_sql,expected_generation_sql,diagnostic]).map_err(store_error)?;
            release_claimed_target(tx, &instance, host, target.as_str(), at.utc)?;
            lift_baseline_hold_if_clear(tx, &instance, host, at.utc)?;
            Ok(CommandResult::ContinuityReattached(
                crate::protocol::results::ContinuityReattachment {
                    seat: seat.clone(),
                    binding_generation: u64::try_from(new_generation).map_err(|_| {
                        api_error(ErrorCode::StoreCorrupt, "negative seat generation")
                    })?,
                    fast_forward,
                },
            ))
        },
    )?;
    validate_continuity_result(&result)?;
    Ok(result)
}

#[cfg(test)]
#[path = "../../../tests/store/named_continuity.rs"]
mod named_continuity_tests;
