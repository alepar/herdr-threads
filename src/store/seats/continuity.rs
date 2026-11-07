//! Continuity candidates and `decide_continuity`.

use crate::{
    protocol::{
        authority::DecisionFence,
        ids::{ContinuityKey, ExecutionId, HostBootId, HostTargetId, SeatId},
        results::{ApiError, CommandResult, ContinuityConflictKind, ErrorCode},
        time::CallBudget,
    },
    store::{
        bindings, catch_up,
        connection::{StoreContext, api_error, store_error},
        continuity_keys,
        effective::{self, EffectiveObservationSource},
        schema,
    },
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::key_verdict::{self, BoundSeatState, KeyFacts, KeyState, KeyVerdict};
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
/// two, `prefer` first when it is a candidate, then in seat-id order (the caller
/// only needs to tell zero, one and several apart, and a key's bound seat is
/// never lost behind the cut-off, ht-ma6i.47).
pub(crate) fn continuity_candidates(
    tx: &Transaction<'_>,
    instance: &str,
    harness: &str,
    native_session: &str,
    prefer: Option<&SeatId>,
) -> Result<Vec<SeatId>, ApiError> {
    let mut statement = tx
        .prepare(&format!(
            "SELECT s.id, COALESCE(MAX(b.disconnect_reason IS 'seat_unresolved'),0) FROM seats s JOIN occupant_bindings b ON b.seat_id=s.id WHERE s.instance_id=?1 AND s.{LOCAL_SEAT} AND s.state='unresolved' AND s.name IS NULL AND {C1_BINDING} GROUP BY s.id ORDER BY (s.id IS ?4) DESC, s.id",
        ))
        .map_err(store_error)?;
    let rows: Vec<(String, bool)> = statement
        .query_map(
            params![
                instance,
                harness,
                native_session,
                prefer.map(SeatId::as_str)
            ],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
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

/// Schema deltas 12 (ht-hsyg) and 16 (ht-ma6i.5): copy every seat-history decision into the table
/// that admits the new kinds. Ordinals are kept, so the AUTOINCREMENT sequence follows.
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
            "SELECT s.id,s.generation,b.binding_id,s.target_id,h.endpoint,b.harness,b.native_session,b.execution_id,b.liveness_pid,b.liveness_start,b.liveness_boot \
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
                    r.get::<_, Option<String>>(10)?,
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
    Option<String>,
);

/// One candidate row; `None` for a harness this build does not know (it cannot be reattached
/// as one).
fn process_continuity_candidate(
    (seat, generation, binding, target, endpoint, harness, session, execution, pid, start, boot): ProcessContinuityRow,
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
        liveness_boot: boot,
    }))
}

pub fn continuity_digest(
    instance: &str,
    command: &crate::protocol::commands::ContinuityCheckIn,
) -> Result<[u8; 32], ApiError> {
    command
        .validate()
        .map_err(|detail| api_error(ErrorCode::InvalidRequest, detail))?;
    // A keyed claim has its own domain, so a keyless digest stays byte-identical to every committed
    // one. The domain does not encode the claim kind, so the source is hashed with it.
    if let Some(key) = &command.continuity_key {
        return schema::canonical_digest(&(
            "keyed_continuity_check_in",
            &command.source,
            instance,
            &command.target,
            &command.harness,
            &command.native_session,
            &command.execution,
            key,
        ));
    }
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

/// What one `decide_continuity` rests on, read once from its evidence: the provenance it records
/// and the seat and generation it is pinned to. The one match on `ContinuityEvidence`; both
/// transaction phases select through it.
struct ContinuitySelection {
    decision_kind: &'static str,
    pinned: Option<(SeatId, i64)>,
    keyed: bool,
}

/// What a transaction phase selected. Each phase turns a miss into its own refusal.
enum SelectedSeat {
    One(SeatId),
    /// The claim's key names this seat: the key is the evidence, whatever the session.
    Keyed(SeatId),
    None,
    Several(Vec<SeatId>),
}

impl ContinuitySelection {
    fn of(evidence: &crate::ports::ContinuityEvidence) -> Result<Self, ApiError> {
        Ok(match evidence {
            crate::ports::ContinuityEvidence::Cooperative => Self {
                decision_kind: crate::protocol::authority::COOPERATIVE_CONTINUITY_PROVENANCE,
                pinned: None,
                keyed: false,
            },
            crate::ports::ContinuityEvidence::Key => Self {
                decision_kind: crate::protocol::authority::COOPERATIVE_CONTINUITY_PROVENANCE,
                pinned: None,
                keyed: true,
            },
            crate::ports::ContinuityEvidence::Process {
                seat,
                seat_generation,
            } => Self {
                decision_kind: crate::protocol::authority::STRUCTURAL_PROCESS_CONTINUITY_PROVENANCE,
                pinned: Some((seat.clone(), checked_host_number(*seat_generation)?)),
                keyed: false,
            },
        })
    }

    /// A keyed claim's judged key; its verdict reattaches a seat or leaves the claim to decide as a
    /// keyless one. A refusal comes before the target's own guards, because the verdict decides an
    /// owned pane too. `None`: no key to judge.
    fn keyed_seat(
        &self,
        tx: &Transaction<'_>,
        instance: &str,
        host: crate::ports::HostId,
        command: &crate::protocol::commands::ContinuityCheckIn,
        conflict: &KeyConflictStash,
    ) -> Result<Option<KeyDecision>, ApiError> {
        let (true, Some(key)) = (self.keyed, &command.continuity_key) else {
            return Ok(None);
        };
        let session = C1Session {
            instance,
            harness: command.harness.as_str(),
            native_session: command.native_session.as_str(),
        };
        let decision = key_decision(
            tx,
            host,
            command.target.as_str(),
            key,
            KeyCaller::Claim(Some(session)),
        )?;
        decision.refuse(host, command.target.as_str(), conflict)?;
        Ok(Some(decision))
    }

    fn select(
        &self,
        tx: &Transaction<'_>,
        instance: &str,
        host: crate::ports::HostId,
        command: &crate::protocol::commands::ContinuityCheckIn,
        keyed: Option<&KeyDecision>,
    ) -> Result<SelectedSeat, ApiError> {
        if let Some(KeyDecision {
            verdict: KeyVerdict::Reattach(seat),
            ..
        }) = keyed
        {
            return Ok(SelectedSeat::Keyed(seat.clone()));
        }
        let mut candidates = continuity_candidates(
            tx,
            instance,
            command.harness.as_str(),
            command.native_session.as_str(),
            None,
        )?;
        match candidates.len() {
            0 => Ok(SelectedSeat::None),
            1 => {
                let seat = candidates.remove(0);
                if let Some((judged, generation)) = &self.pinned {
                    guard_continuity_pin(
                        tx,
                        host,
                        command.target.as_str(),
                        &seat,
                        judged,
                        *generation,
                    )?;
                }
                Ok(SelectedSeat::One(seat))
            }
            _ => Ok(SelectedSeat::Several(candidates)),
        }
    }
}

/// A conflict a refused transaction judged, recorded after that transaction rolled back.
pub(super) type KeyConflictStash = std::cell::RefCell<Option<continuity_keys::ConflictFacts>>;

/// The C1 facts of a claim's native session.
#[derive(Clone, Copy)]
pub(super) struct C1Session<'a> {
    pub(super) instance: &'a str,
    pub(super) harness: &'a str,
    pub(super) native_session: &'a str,
}

/// Who presents a key: a caller with a claim, with its session when C1 can still decide, or one
/// with no evidence to offer.
#[derive(Clone, Copy)]
pub(super) enum KeyCaller<'a> {
    Claim(Option<C1Session<'a>>),
    Claimless,
}

/// One judged key, with the facts a conflict row records.
pub(super) struct KeyDecision {
    key: ContinuityKey,
    pub(super) verdict: KeyVerdict,
    seat: Option<SeatId>,
    owner: Option<SeatId>,
}

impl KeyDecision {
    fn conflict_facts(
        &self,
        host: crate::ports::HostId,
        target: &str,
        kind: ContinuityConflictKind,
    ) -> continuity_keys::ConflictFacts {
        continuity_keys::ConflictFacts {
            host,
            target: target.to_owned(),
            key: self.key.clone(),
            kind,
            seat: self.seat.clone(),
            owner: self.owner.clone(),
        }
    }

    /// The refusal of a verdict that refuses, a conflict stashed for after the rollback. A pane
    /// with an owner answers `TargetAlreadyOwned` as a keyless claim does, so the owner's check-in
    /// proceeds; a hold-bearing conflict is the definitive `TargetUnresolved`.
    pub(super) fn refuse(
        &self,
        host: crate::ports::HostId,
        target: &str,
        conflict: &KeyConflictStash,
    ) -> Result<(), ApiError> {
        match &self.verdict {
            KeyVerdict::NotReady => Err(api_error(
                ErrorCode::ServiceBusy,
                "recovery reconciliation of the current host epoch has not finished; retry",
            )),
            KeyVerdict::Pending => Err(api_error(
                ErrorCode::ContinuityKeyPending,
                "continuity key is bound to an unresolved seat; no seat minted",
            )),
            KeyVerdict::Conflict(kind) => {
                conflict.replace(Some(self.conflict_facts(host, target, *kind)));
                let code = if key_verdict::only_alerts(*kind) {
                    ErrorCode::TargetAlreadyOwned
                } else {
                    ErrorCode::TargetUnresolved
                };
                Err(api_error(
                    code,
                    format!("{}; no seat moved", key_conflict_cause(*kind, target)?),
                ))
            }
            KeyVerdict::Ignore
            | KeyVerdict::Reattach(_)
            | KeyVerdict::NoOp
            | KeyVerdict::Bind
            | KeyVerdict::EnrollThenBind => Ok(()),
        }
    }
}

/// The cause text of a key conflict, its own and never the restore-hold text.
pub(super) fn key_conflict_cause(
    kind: ContinuityConflictKind,
    target: &str,
) -> Result<String, ApiError> {
    let pane = shlex::try_quote(target)
        .map_err(|_| api_error(ErrorCode::InvalidRequest, "invalid target"))?;
    Ok(format!(
        "{}{} on {pane}",
        crate::protocol::results::KEY_CONFLICT_CAUSE,
        continuity_keys::kind_name(&kind)?
    ))
}

/// Gather the facts of one presented key at one pane and judge them. A host that is not a Herdr
/// host ignores every key: a named seat's continuity is its name, a herdr-core terminal's its core.
pub(super) fn key_decision(
    tx: &Transaction<'_>,
    host: crate::ports::HostId,
    target: &str,
    key: &ContinuityKey,
    caller: KeyCaller<'_>,
) -> Result<KeyDecision, ApiError> {
    let kind: Option<String> = tx
        .query_row(
            "SELECT kind FROM herdr_hosts WHERE host_id=?1",
            [host.get()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    let row = if kind
        .as_deref()
        .map(crate::ports::HostKind::from_stored)
        .is_some_and(crate::ports::HostKind::restores_saved_targets)
    {
        continuity_keys::lookup_key(tx, key)?
    } else {
        None
    };
    let seat = row.as_ref().and_then(|row| row.seat.clone());
    let state = seat
        .as_ref()
        .map(|seat| bound_seat_state(tx, seat))
        .transpose()?;
    let owner: Option<SeatId> = tx
        .query_row(
            "SELECT id FROM seats WHERE host_id=?1 AND target_id=?2 AND state='resolved' LIMIT 1",
            params![host.get(), target],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(store_error)?
        .map(SeatId::new);
    let owner_holds_other_key = match &owner {
        Some(owner) => {
            continuity_keys::key_of_seat(tx, owner)?.is_some_and(|holder| holder.key != *key)
        }
        None => false,
    };
    let candidates = match caller {
        KeyCaller::Claim(Some(session)) => continuity_candidates(
            tx,
            session.instance,
            session.harness,
            session.native_session,
            seat.as_ref(),
        )?,
        KeyCaller::Claim(None) | KeyCaller::Claimless => Vec::new(),
    };
    let c1_match = candidates
        .iter()
        .find(|candidate| Some(*candidate) == seat.as_ref())
        .or(candidates.first());
    let verdict = key_verdict::judge(&KeyFacts {
        key: match (&row, &seat, state) {
            (None, _, _) => KeyState::Unreserved,
            (Some(_), Some(seat), Some(state)) => KeyState::Bound { seat, state },
            (Some(_), _, _) => KeyState::Unbound,
        },
        pane_owner: owner.as_ref(),
        pane_owner_holds_other_key: owner_holds_other_key,
        c1_match,
        has_claim: matches!(caller, KeyCaller::Claim(_)),
        lag: reconciliation_lags(tx, host)?,
    });
    Ok(KeyDecision {
        key: key.clone(),
        verdict,
        seat,
        owner,
    })
}

fn bound_seat_state(tx: &Transaction<'_>, seat: &SeatId) -> Result<BoundSeatState, ApiError> {
    let state: String = tx
        .query_row(
            "SELECT state FROM seats WHERE id=?1",
            [seat.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    Ok(match state.as_str() {
        "unresolved" => BoundSeatState::Unresolved,
        "retired" => BoundSeatState::Retired,
        // A seat that handed off to another node is live elsewhere too.
        _ => BoundSeatState::Resolved,
    })
}

/// Record a conflict a refused transaction judged, in its own short transaction on the same
/// connection, so it survives the rollback. The refusal stands whatever this does: a full table or
/// a failed write is only logged.
pub(super) fn record_stashed_conflict(
    context: &StoreContext,
    conn: &mut Connection,
    conflict: KeyConflictStash,
) {
    let Some(facts) = conflict.into_inner() else {
        return;
    };
    let recorded = context.execute_decision(
        conn,
        |_| Ok(()),
        |tx, at, ()| continuity_keys::record_conflict(tx, &facts, at.utc),
    );
    log_conflict_record(&facts, recorded);
}

/// What a transaction that commits owes the key it was presented: a free key binds to the seat with
/// `check_in` evidence, and a conflict is recorded in place while the caller proceeds unbound.
pub(super) fn settle_key(
    tx: &Transaction<'_>,
    at: crate::protocol::time::UtcMillis,
    host: crate::ports::HostId,
    target: &str,
    seat: &SeatId,
    decision: &KeyDecision,
) -> Result<(), ApiError> {
    match &decision.verdict {
        KeyVerdict::Bind | KeyVerdict::EnrollThenBind => continuity_keys::bind(
            tx,
            &decision.key,
            seat,
            crate::protocol::results::ContinuityKeyBinding::CheckIn,
            at,
        ),
        KeyVerdict::Conflict(kind) => {
            let facts = decision.conflict_facts(host, target, *kind);
            let recorded = continuity_keys::record_conflict(tx, &facts, at)?;
            log_conflict_record(&facts, Ok(recorded));
            Ok(())
        }
        KeyVerdict::Ignore
        | KeyVerdict::Reattach(_)
        | KeyVerdict::NoOp
        | KeyVerdict::Pending
        | KeyVerdict::NotReady => Ok(()),
    }
}

fn log_conflict_record(
    facts: &continuity_keys::ConflictFacts,
    recorded: Result<continuity_keys::ConflictRecord, ApiError>,
) {
    let target = &facts.target;
    match recorded {
        Ok(continuity_keys::ConflictRecord::TableFull) => eprintln!(
            "herdr-threads: continuity key conflict on {target} not recorded: the conflict table is full"
        ),
        Ok(_) => {}
        Err(error) => eprintln!(
            "herdr-threads: continuity key conflict on {target} not recorded: {}",
            error.detail
        ),
    }
}

/// C1 pin (ht-hsyg): evidence about one seat at one generation. The selected seat must be that
/// seat, unmoved; a kept claim on the target is a session's own word and is decided by its own path
/// first. Shared by every decision that rests on such evidence.
pub(super) fn guard_continuity_pin(
    tx: &Transaction<'_>,
    host: crate::ports::HostId,
    target: &str,
    selected: &SeatId,
    judged: &SeatId,
    generation: i64,
) -> Result<(), ApiError> {
    let current: Option<i64> = tx
        .query_row(
            "SELECT generation FROM seats WHERE id=?1 AND state='unresolved'",
            [judged.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    if selected != judged || current != Some(generation) {
        return Err(api_error(
            ErrorCode::Conflict,
            "process continuity evidence no longer names the matching seat; no seat moved",
        ));
    }
    let kept: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM kept_continuity_claims WHERE host_id=?1 AND target_id=?2)",
            params![host.get(), target],
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
    let selection = ContinuitySelection::of(&request.evidence)?;
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
    let expected_boot = request.guard.host_boot().clone();
    let expected_epoch = request.guard.epoch();
    let expected_generation = request.guard.generation();
    let structural_proof = request.guard.structural_proof().cloned();
    let expected_epoch_sql = checked_host_number(expected_epoch)?;
    let expected_generation_sql = checked_host_number(expected_generation)?;
    let digest = continuity_digest(elected_instance, &command)?;
    let harness = command.harness.as_str();
    let session = command.native_session.as_str();
    let conflict = KeyConflictStash::default();
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
            let keyed = selection.keyed_seat(tx, &instance, host, &command, &conflict)?;
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
            let seat = match selection.select(tx, &instance, host, &command, keyed.as_ref())? {
                SelectedSeat::One(seat) | SelectedSeat::Keyed(seat) => seat,
                SelectedSeat::None => {
                    // Until the first reconciliation pass of the current
                    // recovery epoch has run, "no unresolved seat" only says the
                    // daemon has not yet rebuilt the seats a Herdr restart
                    // unresolved: pending, not final. A pass only unresolves
                    // seats, never adds bindings, so a session no seat's
                    // binding names can never become a candidate: its answer
                    // is final now, whatever the lag (ht-kaw9).
                    return Err(
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
                        },
                    );
                }
                SelectedSeat::Several(candidates) => {
                    let pane = shlex::try_quote(target.as_str())
                        .map_err(|_| api_error(ErrorCode::InvalidRequest, "invalid target"))?;
                    let choices = candidates.iter().map(|seat| {
                        let seat = shlex::try_quote(seat.as_str()).map_err(|_| api_error(ErrorCode::StoreCorrupt,"invalid seat id"))?;
                        Ok(format!("inspect `herdr-threads seat inspect {seat}`; if this is its pane, `herdr-threads human seat rebind {seat} --pane {pane} --operator`"))
                    }).collect::<Result<Vec<_>,ApiError>>()?.join("; ");
                    return Err(api_error(
                        ErrorCode::Conflict,
                        format!(
                            "native session matches several unresolved seats (up to two shown); {choices}; no seat moved"
                        ),
                    ));
                }
            };
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
            let keyed = selection.keyed_seat(tx, &instance, host, &command, &conflict)?;
            let selected = selection.select(tx, &instance, host, &command, keyed.as_ref())?;
            // ht-ma6i.47: a session reattach binds a free presented key now, not at a later
            // check-in, when the seat holds no key; a seat holding another is judged at its next
            // check-in through the ordinary path.
            let bind_free_key = matches!(selected, SelectedSeat::One(_))
                && keyed.as_ref().is_some_and(|decision| {
                    matches!(
                        decision.verdict,
                        KeyVerdict::EnrollThenBind | KeyVerdict::Bind
                    )
                });
            let (seat, decision_kind) = match selected {
                SelectedSeat::One(seat) => (seat, selection.decision_kind),
                SelectedSeat::Keyed(seat) => (
                    seat,
                    crate::protocol::authority::COOPERATIVE_CONTINUITY_KEY_PROVENANCE,
                ),
                SelectedSeat::None | SelectedSeat::Several(_) => {
                    return Err(api_error(ErrorCode::Conflict, "continuity match changed"));
                }
            };
            let seat = &seat;
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
            if bind_free_key
                && let Some(decision) = &keyed
                && continuity_keys::key_of_seat(tx, seat)?.is_none()
            {
                settle_key(tx, at.utc, host, target.as_str(), seat, decision)?;
            }
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
    );
    if result.is_err() {
        record_stashed_conflict(context, conn, conflict);
    }
    let result = result?;
    validate_continuity_result(&result)?;
    Ok(result)
}

/// The seats a shell row is written for and judged on: local, unnamed, on a Herdr host (a named
/// seat's continuity is its name, a herdr-core terminal's its core).
const SHELL_SEAT_SCOPE: &str =
    "s.is_local=1 AND s.name IS NULL AND s.target_id IS NOT NULL AND h.kind='herdr'";

/// Not a person seat: the seat's newest top-level binding, if any, is no `human` one.
const NOT_A_PERSON_SEAT: &str = "COALESCE((SELECT b.harness FROM occupant_bindings b WHERE b.seat_id=s.id AND b.role='top_level' AND b.agent_id='' ORDER BY b.ordinal DESC LIMIT 1),'')<>'human'";

/// TRUST-POLICY C1 shell continuity (ht-ma6i.1): the resolved seats of `host` with no fingerprint
/// for their current (boot, incarnation, terminal). A seat whose proof names another incarnation
/// than the published one (a resolved seat a refused transition left on a dead incarnation) is not
/// listed: its pane's shell would be recorded against the wrong incarnation. Each gap carries the
/// published snapshot's boot and epoch, which fence the shell read.
pub fn shell_fingerprint_gaps(
    context: &StoreContext,
    instance: &str,
    host: crate::ports::HostId,
    after: Option<&SeatId>,
    budget: &CallBudget,
) -> Result<Vec<crate::ports::ShellFingerprintGap>, ApiError> {
    let db = context.open_query(budget.clone())?;
    let limit = i64::try_from(crate::ports::SHELL_FINGERPRINT_READS_PER_PASS).unwrap_or(i64::MAX);
    let mut statement = db
        .prepare(&format!(
            "SELECT s.id,s.generation,s.target_id,s.structural_terminal_id,s.structural_incarnation,g.host_boot,g.epoch \
             FROM seats s JOIN herdr_hosts h ON h.host_id=s.host_id \
             JOIN snapshot_generations g ON g.id=h.active_snapshot_id AND g.status='published' \
             WHERE s.instance_id=?1 AND s.host_id=?2 AND s.state='resolved' AND {SHELL_SEAT_SCOPE} \
             AND s.structural_terminal_id IS NOT NULL AND s.structural_host_boot IS NOT NULL AND s.structural_incarnation=g.incarnation \
             AND NOT EXISTS(SELECT 1 FROM seat_shell_fingerprints f WHERE f.seat_id=s.id AND f.host_boot=s.structural_host_boot AND f.incarnation=s.structural_incarnation AND f.terminal_id=s.structural_terminal_id) \
             AND (?3 IS NULL OR s.id>?3) ORDER BY s.id LIMIT ?4",
        ))
        .map_err(|e| db.map_error(e))?;
    let rows = statement
        .query_map(
            params![instance, host.get(), after.map(SeatId::as_str), limit],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, i64>(6)?,
                ))
            },
        )
        .map_err(|e| db.map_error(e))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| db.map_error(e))?;
    let corrupt = |what: &str| api_error(ErrorCode::StoreCorrupt, format!("invalid {what}"));
    rows.into_iter()
        .map(
            |(seat, generation, target, terminal, incarnation, boot, epoch)| {
                Ok(crate::ports::ShellFingerprintGap {
                    seat: SeatId::parse(seat).map_err(|_| corrupt("seat id"))?,
                    seat_generation: u64::try_from(generation)
                        .map_err(|_| corrupt("seat generation"))?,
                    target: HostTargetId::parse(target).map_err(|_| corrupt("target id"))?,
                    terminal: crate::protocol::ids::TerminalId::parse(terminal)
                        .map_err(|_| corrupt("terminal id"))?,
                    incarnation,
                    host_boot: HostBootId::new(boot),
                    host_epoch: u64::try_from(epoch).map_err(|_| corrupt("host epoch"))?,
                })
            },
        )
        .collect()
}

/// Write a read fingerprint if the seat still stands as `gap` listed it: its generation, target,
/// terminal and incarnation, and its host's active publication is still the listing's (boot,
/// epoch, incarnation), so a restart reconciled between the read and the write records nothing.
/// Returns whether it did. Replaces the row of the seat's last terminal.
pub fn record_shell_fingerprint(
    conn: &Connection,
    gap: &crate::ports::ShellFingerprintGap,
    fingerprint: &crate::identity::shell_continuity::ShellFingerprint,
    at: crate::protocol::time::UtcMillis,
) -> Result<bool, ApiError> {
    let (Ok(start), Ok(tty)) = (
        i64::try_from(fingerprint.start),
        i64::try_from(fingerprint.tty),
    ) else {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "shell fingerprint out of range",
        ));
    };
    let written = conn
        .execute(
            "INSERT OR REPLACE INTO seat_shell_fingerprints(seat_id,host_id,target_id,host_boot,incarnation,terminal_id,pid,start,session,tty,kernel_boot,recorded_at) \
             SELECT s.id,s.host_id,s.target_id,s.structural_host_boot,s.structural_incarnation,s.structural_terminal_id,?6,?7,?8,?9,?13,?10 \
             FROM seats s JOIN herdr_hosts h ON h.host_id=s.host_id \
             JOIN snapshot_generations g ON g.id=h.active_snapshot_id AND g.status='published' \
             WHERE s.id=?1 AND s.state='resolved' AND s.generation=?2 AND s.target_id=?3 AND s.structural_terminal_id=?4 AND s.structural_incarnation=?5 AND s.structural_host_boot IS NOT NULL \
             AND g.host_boot=?11 AND g.epoch=?12 AND g.incarnation=?5",
            params![
                gap.seat.as_str(),
                checked_host_number(gap.seat_generation)?,
                gap.target.as_str(),
                gap.terminal.as_str(),
                gap.incarnation,
                fingerprint.pid,
                start,
                fingerprint.session,
                tty,
                at.0,
                gap.host_boot.as_str(),
                checked_host_number(gap.host_epoch)?,
                fingerprint.kernel_boot.as_str()
            ],
        )
        .map_err(store_error)?;
    Ok(written == 1)
}

/// A seat's fingerprint goes when it retires finally or is un-retired: a row left behind would be
/// evidence about a shell the seat no longer has any claim on.
pub(crate) fn delete_shell_fingerprint(
    tx: &Transaction<'_>,
    seat: &SeatId,
) -> Result<(), ApiError> {
    tx.execute(
        "DELETE FROM seat_shell_fingerprints WHERE seat_id=?1",
        [seat.as_str()],
    )
    .map_err(store_error)?;
    Ok(())
}

/// TRUST-POLICY C1 shell continuity (ht-ma6i.1): the unresolved seats of `host` that a new
/// incarnation left at a saved target with a fingerprint of an earlier incarnation there. Reason
/// `other` only; an `unretired` seat has no target, and a person seat stays an operator case.
pub fn shell_continuity_candidates(
    context: &StoreContext,
    instance: &str,
    host: crate::ports::HostId,
    after: Option<&SeatId>,
    budget: &CallBudget,
) -> Result<Vec<crate::ports::ShellContinuityCandidate>, ApiError> {
    let db = context.open_query(budget.clone())?;
    let limit =
        i64::try_from(crate::ports::SHELL_CONTINUITY_CANDIDATES_PER_PASS).unwrap_or(i64::MAX);
    let mut statement = db
        .prepare(&format!(
            "SELECT s.id,s.generation,s.target_id,f.pid,f.start,f.session,f.tty,f.kernel_boot \
             FROM seats s JOIN seat_shell_fingerprints f ON f.seat_id=s.id AND f.host_id=s.host_id AND f.target_id=s.target_id \
             JOIN herdr_hosts h ON h.host_id=s.host_id \
             JOIN snapshot_generations g ON g.id=h.active_snapshot_id AND g.status='published' \
             WHERE s.instance_id=?1 AND s.host_id=?2 AND s.state='unresolved' AND s.unresolved_reason='other' AND {SHELL_SEAT_SCOPE} \
             AND f.incarnation<>g.incarnation AND {NOT_A_PERSON_SEAT} \
             AND (?3 IS NULL OR s.id>?3) ORDER BY s.id LIMIT ?4",
        ))
        .map_err(|e| db.map_error(e))?;
    let rows = statement
        .query_map(
            params![instance, host.get(), after.map(SeatId::as_str), limit],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, i64>(6)?,
                    r.get::<_, String>(7)?,
                ))
            },
        )
        .map_err(|e| db.map_error(e))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| db.map_error(e))?;
    let corrupt = |what: &str| api_error(ErrorCode::StoreCorrupt, format!("invalid {what}"));
    rows.into_iter()
        .map(
            |(seat, generation, target, pid, start, session, tty, kernel_boot)| {
                Ok(crate::ports::ShellContinuityCandidate {
                    seat: SeatId::parse(seat).map_err(|_| corrupt("seat id"))?,
                    seat_generation: u64::try_from(generation)
                        .map_err(|_| corrupt("seat generation"))?,
                    target: HostTargetId::parse(target).map_err(|_| corrupt("target id"))?,
                    fingerprint: crate::identity::shell_continuity::ShellFingerprint {
                        pid: u32::try_from(pid).map_err(|_| corrupt("fingerprint pid"))?,
                        start: u64::try_from(start).map_err(|_| corrupt("fingerprint start"))?,
                        session: u32::try_from(session)
                            .map_err(|_| corrupt("fingerprint session"))?,
                        tty: u64::try_from(tty).map_err(|_| corrupt("fingerprint tty"))?,
                        kernel_boot,
                    },
                })
            },
        )
        .collect()
}

/// Whether another non-retired seat's fingerprint names the same process: the shell could then be
/// either seat's, so nothing is decided (seats are never chosen between on evidence that fits two).
/// A fingerprint of another kernel boot names no process of this one and is no rival.
pub fn shell_rival_exists(
    conn: &Connection,
    seat: &SeatId,
    fingerprint: &crate::identity::shell_continuity::ShellFingerprint,
) -> Result<bool, ApiError> {
    let start = i64::try_from(fingerprint.start)
        .map_err(|_| api_error(ErrorCode::InvalidRequest, "shell fingerprint out of range"))?;
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM seat_shell_fingerprints f JOIN seats s ON s.id=f.seat_id WHERE f.seat_id<>?1 AND f.pid=?2 AND f.start=?3 AND f.kernel_boot=?4 AND s.state<>'retired')",
        params![
            seat.as_str(),
            fingerprint.pid,
            start,
            fingerprint.kernel_boot.as_str()
        ],
        |r| r.get(0),
    )
    .map_err(store_error)
}

/// The judged seat is still the candidate the listing named, at this target with this fingerprint.
fn shell_candidate_stands(
    tx: &Transaction<'_>,
    host: crate::ports::HostId,
    target: &str,
    seat: &SeatId,
    fingerprint: &crate::identity::shell_continuity::ShellFingerprint,
) -> Result<bool, ApiError> {
    let start = i64::try_from(fingerprint.start)
        .map_err(|_| api_error(ErrorCode::InvalidRequest, "shell fingerprint out of range"))?;
    tx.query_row(
        &format!(
            "SELECT EXISTS(SELECT 1 FROM seats s JOIN seat_shell_fingerprints f ON f.seat_id=s.id AND f.host_id=s.host_id AND f.target_id=s.target_id \
             JOIN herdr_hosts h ON h.host_id=s.host_id \
             JOIN snapshot_generations g ON g.id=h.active_snapshot_id AND g.status='published' \
             WHERE s.id=?1 AND s.host_id=?2 AND s.target_id=?3 AND s.state='unresolved' AND s.unresolved_reason='other' AND {SHELL_SEAT_SCOPE} \
             AND f.pid=?4 AND f.start=?5 AND f.kernel_boot=?6 AND f.incarnation<>g.incarnation AND {NOT_A_PERSON_SEAT})"
        ),
        params![
            seat.as_str(),
            host.get(),
            target,
            fingerprint.pid,
            start,
            fingerprint.kernel_boot.as_str()
        ],
        |r| r.get(0),
    )
    .map_err(store_error)
}

/// TRUST-POLICY C1 shell continuity (ht-ma6i.1): resolve the seat the kernel facts tied to the pane
/// shell, at its saved target, in one transaction under an operation key per (seat, generation,
/// fingerprint). Held or unowned targets only, guarded like C1 process continuity: the seat is
/// pinned at the judged generation and a kept claim on the target is decided first. A key conflict
/// row binds only ordinary resolution, and `rebind_unresolved_seat` clears it. The structural proof
/// makes the next pass reconfirm the seat. It opens no binding: there is no session to attach, so
/// the seat stays unavailable until an agent checks in.
pub fn decide_shell_continuity(
    context: &StoreContext,
    conn: &mut Connection,
    elected_instance: &str,
    request: crate::ports::ShellContinuityRequest,
) -> Result<CommandResult, ApiError> {
    let crate::ports::ShellContinuityRequest {
        seat,
        seat_generation,
        fingerprint,
        mut guard,
    } = request;
    let instance = guard.instance().to_owned();
    if instance != elected_instance {
        return Err(api_error(
            ErrorCode::StaleHostObservation,
            "continuity guard belongs to another instance",
        ));
    }
    let host = guard.host_id();
    let target = guard.target().as_str().to_owned();
    let target = target.as_str();
    let generation = checked_host_number(seat_generation)?;
    let expected_boot = guard.host_boot().clone();
    let expected_epoch = guard.epoch();
    let expected_generation = guard.generation();
    let proof = guard.structural_proof().clone();
    let expected_epoch_sql = checked_host_number(expected_epoch)?;
    let expected_generation_sql = checked_host_number(expected_generation)?;
    let operation = format!(
        "shell-continuity:{}:{}:{}:{}",
        seat.as_str(),
        generation,
        fingerprint.pid,
        fingerprint.start
    );
    let digest = schema::canonical_digest(&(
        crate::protocol::authority::STRUCTURAL_SHELL_CONTINUITY_PROVENANCE,
        &instance,
        seat.as_str(),
        target,
        generation,
        fingerprint.pid,
        fingerprint.start,
    ))?;
    schema::execute_idempotent_transaction(
        context,
        conn,
        &continuity_scope(&instance),
        &operation,
        digest,
        |tx| {
            if !observed_matches(
                tx,
                host,
                target,
                &expected_boot,
                expected_epoch,
                expected_generation,
            )? {
                return Err(api_error(
                    ErrorCode::StaleHostObservation,
                    "continuity target observation changed",
                ));
            }
            if !target_free(tx, host, target)? {
                return Err(if reconciliation_lags(tx, host)? {
                    api_error(
                        ErrorCode::ServiceBusy,
                        "recovery reconciliation of the current host epoch has not finished; retry",
                    )
                } else {
                    api_error(
                        ErrorCode::TargetAlreadyOwned,
                        "continuity target has a resolved owner; no seat moved",
                    )
                });
            }
            if !structural_proof_matches_current(tx, host, &proof)? {
                return Err(api_error(
                    ErrorCode::StaleHostObservation,
                    "continuity structural proof differs from current evidence",
                ));
            }
            guard_continuity_pin(tx, host, target, &seat, &seat, generation)?;
            if !shell_candidate_stands(tx, host, target, &seat, &fingerprint)?
                || shell_rival_exists(tx, &seat, &fingerprint)?
            {
                return Err(api_error(
                    ErrorCode::Conflict,
                    "shell continuity evidence no longer names this seat alone; no seat moved",
                ));
            }
            if generation == i64::MAX {
                return Err(api_error(
                    ErrorCode::SequenceExhausted,
                    "seat binding generation exhausted",
                ));
            }
            Ok(())
        },
        |tx, at| {
            let fresh = effective::effective_observation(tx, host, target)?
                .filter(|observation| {
                    observation.source == EffectiveObservationSource::NewerCurrentTarget
                })
                .ok_or_else(|| {
                    api_error(
                        ErrorCode::StaleHostObservation,
                        "fresh target observation missing",
                    )
                })?;
            guard
                .consume(&DecisionFence {
                    now: at.monotonic,
                    host_boot: HostBootId::new(fresh.host_boot),
                    host_epoch: u64::try_from(fresh.epoch)
                        .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative host epoch"))?,
                    target_generation: u64::try_from(fresh.structural_generation).map_err(
                        |_| api_error(ErrorCode::StoreCorrupt, "negative target generation"),
                    )?,
                    binding_generation: 0,
                    known_invalidated: !observed_matches(
                        tx,
                        host,
                        target,
                        &expected_boot,
                        expected_epoch,
                        expected_generation,
                    )?,
                })
                .map_err(|reason| api_error(ErrorCode::StaleHostObservation, reason))?;
            rebind_unresolved_seat(
                tx,
                &instance,
                host,
                &seat,
                target,
                expected_generation_sql,
                at.utc,
            )?;
            update_structural_proof(tx, &seat, &proof)?;
            let new_generation: i64 = tx
                .query_row(
                    "SELECT generation FROM seats WHERE id=?1",
                    [seat.as_str()],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            tx.execute(
                "INSERT INTO allocation_decisions(host_id,target_id,seat_id,kind,decided_at,host_boot,epoch,generation) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    host.get(),
                    target,
                    seat.as_str(),
                    crate::protocol::authority::STRUCTURAL_SHELL_CONTINUITY_PROVENANCE,
                    at.utc.0,
                    expected_boot.as_str(),
                    expected_epoch_sql,
                    expected_generation_sql
                ],
            )
            .map_err(store_error)?;
            release_claimed_target(tx, &instance, host, target, at.utc)?;
            lift_baseline_hold_if_clear(tx, &instance, host, at.utc)?;
            Ok(CommandResult::ContinuityReattached(
                crate::protocol::results::ContinuityReattachment {
                    seat: seat.clone(),
                    binding_generation: u64::try_from(new_generation).map_err(|_| {
                        api_error(ErrorCode::StoreCorrupt, "negative seat generation")
                    })?,
                    fast_forward: None,
                },
            ))
        },
    )
}

#[cfg(test)]
#[path = "../../../tests/store/named_continuity.rs"]
mod named_continuity_tests;

#[cfg(test)]
#[path = "../../../tests/store/shell_continuity.rs"]
mod shell_continuity_tests;
