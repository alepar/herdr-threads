//! Reversible retirement (ht-ma6i, spec 4.3, TRUST-POLICY *Reversible retirement*).
//!
//! Responsibility (reversible)
//! - Owns: `reversible_retirements` and `reversible_windows` (each un-retired episode's window);
//!   the reversible to unretired and reversible or unretired to final transitions
//!   (`UnretireSeat`, `FinalizeRetirement`, the operator's finalization); the deadline shift, at
//!   the un-retire and for a late arrival ([`window_shift_ms`]); the SQL fragments for "frozen",
//!   "waiting with no pane" and "held open".
//! - Hides: which seats are frozen or waiting. The fragments are macros, because the receipt scans
//!   build their SQL with `concat!`.
//! - Must not: place a seat, touch holds, open a binding, or settle an obligation (finalization
//!   inserts the job and `advance_retirement` settles as today). It adds no predicate to the
//!   retired read sites.
//! - Role: stateless functions over the caller's transaction. The fence itself is
//!   `control::retirement_transition`; the row lives from the fence until final retirement or
//!   rebind (`rebind_unresolved_seat` deletes it). A wrong author is `Unauthorized`, a seat in the
//!   wrong state is `NotFound`, and the journal hold's `ServiceBusy` passes through unchanged.
//!   The facts (`seats::materialize_seat_retired_reversible`, `seats::materialize_seat_unretired`,
//!   `seats::materialize_seat_retired`) reach the same transitions through [`insert_fact_row`],
//!   [`unretire_by_fact`] and [`finalize_by_fact`], with no author to check. A peer's row has no
//!   closure inputs and never reads `unretired`. A copy whose episode took effect here is marked on
//!   its journal row ([`mark_episode_effect`]), and [`episode_effect_recorded`] makes a re-issued
//!   copy of such an episode a no-op; a copy that changed nothing is never marked. A
//!   `seat.unretired` that arrived before its retire is kept ([`kept_unretire`]) and reverses that
//!   retire when it lands.

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::{
    ports::RetirementInputs,
    protocol::{
        ids::{SeatId, ServiceAuthorId},
        results::{ApiError, ContinuityStatus, ErrorCode, SeatRetirementState},
        service::{SeatUnretired as SeatUnretiredResult, ServiceResult, UnretireSeat},
        time::UtcMillis,
    },
    store::{
        connection::{DecisionInstant, StoreContext, api_error, store_error},
        continuity_keys, control,
        effective::{LOGICAL_PENDING_AVAILABLE_AT_SQL, LOGICAL_PENDING_DEADLINE_SQL},
        schema,
    },
    sync::facts::{
        FactKind,
        seat::{RetireReason, SeatUnretired, SeatUnretiredPayload},
    },
};

/// A seat retired reversibly: what it owes is frozen, so a due scan skips it.
macro_rules! frozen_seat_sql {
    ($seat:literal) => {
        concat!(
            "EXISTS(SELECT 1 FROM reversible_retirements rr WHERE rr.seat_id=",
            $seat,
            " AND rr.state='reversible')"
        )
    };
}
pub(crate) use frozen_seat_sql;

/// A seat un-retired and waiting with no pane: it protects nothing, so hold lift, repair guidance,
/// health and the reconciliation plan leave it out.
macro_rules! waiting_seat_sql {
    ($seat:literal) => {
        concat!(
            "EXISTS(SELECT 1 FROM reversible_retirements rr WHERE rr.seat_id=",
            $seat,
            " AND rr.state='unretired')"
        )
    };
}
pub(crate) use waiting_seat_sql;

/// A seat frozen or waiting: it can still return, so it stays a live member and an origin.
macro_rules! held_open_seat_sql {
    ($seat:literal) => {
        concat!(
            "EXISTS(SELECT 1 FROM reversible_retirements rr WHERE rr.seat_id=",
            $seat,
            ")"
        )
    };
}
pub(crate) use held_open_seat_sql;

/// Whether what `seat` owes is frozen: a fact applied now writes `pending`, not a terminal state.
pub(crate) fn is_frozen(conn: &Connection, seat: &SeatId) -> Result<bool, ApiError> {
    conn.query_row(
        concat!("SELECT ", frozen_seat_sql!("?1")),
        [seat.as_str()],
        |r| r.get(0),
    )
    .map_err(store_error)
}

/// `SeatInspection.retirement_state` for a seat reading `continuity`: present only for a seat that
/// reads retired and is frozen. A final (or pre-branch) retirement and an unretired seat are
/// absent, so an older client decodes every retired seat's inspection.
pub(crate) fn retirement_state(
    conn: &Connection,
    seat: &SeatId,
    continuity: ContinuityStatus,
) -> Result<Option<SeatRetirementState>, ApiError> {
    if continuity != ContinuityStatus::Retired || !is_frozen(conn, seat)? {
        return Ok(None);
    }
    Ok(Some(SeatRetirementState::Reversible))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowState {
    Reversible,
    Unretired,
}

/// Who finalizes: the service that owns the seat's key, or the operator under this audit label.
#[derive(Debug, Clone, Copy)]
pub(crate) enum FinalizeBy<'a> {
    Service(&'a ServiceAuthorId),
    Operator(&'a str),
}

struct Row {
    state: RowState,
    cutover_at: i64,
    reason: RetireReason,
    inputs: RetirementInputs,
}

pub(crate) fn row_state(conn: &Connection, seat: &SeatId) -> Result<Option<RowState>, ApiError> {
    conn.query_row(
        "SELECT state FROM reversible_retirements WHERE seat_id=?1",
        [seat.as_str()],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .map_err(store_error)?
    .map(|state| parse_state(&state))
    .transpose()
}

/// The episode a current `reversible` row names, for a fact's `retired_at` to match.
fn frozen_episode(conn: &Connection, seat: &SeatId) -> Result<Option<i64>, ApiError> {
    conn.query_row(
        "SELECT cutover_at FROM reversible_retirements WHERE seat_id=?1 AND state='reversible'",
        [seat.as_str()],
        |r| r.get(0),
    )
    .optional()
    .map_err(store_error)
}

fn parse_state(state: &str) -> Result<RowState, ApiError> {
    match state {
        "reversible" => Ok(RowState::Reversible),
        "unretired" => Ok(RowState::Unretired),
        _ => Err(api_error(
            ErrorCode::StoreCorrupt,
            "invalid reversible retirement state",
        )),
    }
}

fn read_row(conn: &Connection, seat: &SeatId) -> Result<Option<Row>, ApiError> {
    type Columns = (
        String,
        i64,
        String,
        Option<String>,
        Option<i64>,
        Option<String>,
        Option<i64>,
        Option<String>,
        Option<String>,
    );
    let columns: Option<Columns> = conn
        .query_row(
            "SELECT state,cutover_at,reason,closure_boot,closure_epoch,closure_target,closure_generation,closure_evidence,absence_evidence FROM reversible_retirements WHERE seat_id=?1",
            [seat.as_str()],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                ))
            },
        )
        .optional()
        .map_err(store_error)?;
    let Some((state, cutover_at, reason, boot, epoch, target, generation, closure, absence)) =
        columns
    else {
        return Ok(None);
    };
    let corrupt = |what| api_error(ErrorCode::StoreCorrupt, what);
    Ok(Some(Row {
        state: parse_state(&state)?,
        cutover_at,
        reason: match reason.as_str() {
            "closed" => RetireReason::Closed,
            "operator" => RetireReason::Operator,
            _ => return Err(corrupt("invalid reversible retirement reason")),
        },
        inputs: RetirementInputs {
            // A row without closure inputs (a store restored from its own log) yields the empty
            // inputs the operator retire path uses for a seat with no host evidence.
            boot: boot.unwrap_or_default(),
            epoch: epoch.unwrap_or_default(),
            target: target.unwrap_or_default(),
            generation: generation.unwrap_or_default(),
            closure_evidence: match closure.as_deref() {
                None => None,
                Some("restored_pane_closed") => Some("restored_pane_closed"),
                Some(_) => return Err(corrupt("invalid reversible closure evidence")),
            },
            absence_evidence: match absence.as_deref() {
                None => None,
                Some("restored_target_absent") => Some("restored_target_absent"),
                Some(_) => return Err(corrupt("invalid reversible absence evidence")),
            },
        },
    }))
}

/// The row a replicated `seat.retired_reversible` leaves: a peer's, or a local row replayed from its
/// own log. Neither knows the closure inputs, and a key is never restored.
pub(crate) fn insert_fact_row(
    tx: &Transaction<'_>,
    seat: &SeatId,
    retired_at: i64,
    reason: RetireReason,
) -> Result<(), ApiError> {
    tx.execute(
        "INSERT INTO reversible_retirements(seat_id,state,cutover_at,reason) VALUES (?1,'reversible',?2,?3)",
        params![seat.as_str(), retired_at, reason_column(reason)],
    )
    .map_err(store_error)?;
    Ok(())
}

fn reason_column(reason: RetireReason) -> &'static str {
    match reason {
        RetireReason::Closed => "closed",
        RetireReason::Operator => "operator",
    }
}

/// The fence's row: the job's inputs, kept until finalization.
pub(crate) fn insert_row(
    tx: &Transaction<'_>,
    seat: &SeatId,
    cutover: UtcMillis,
    reason: RetireReason,
    inputs: &RetirementInputs,
) -> Result<(), ApiError> {
    tx.execute(
        "INSERT INTO reversible_retirements(seat_id,state,cutover_at,reason,closure_boot,closure_epoch,closure_target,closure_generation,closure_evidence,absence_evidence) VALUES (?1,'reversible',?2,?3,?4,?5,?6,?7,?8,?9)",
        params![
            seat.as_str(),
            cutover.0,
            reason_column(reason),
            inputs.boot,
            inputs.epoch,
            inputs.target,
            inputs.generation,
            inputs.closure_evidence,
            inputs.absence_evidence
        ],
    )
    .map_err(store_error)?;
    Ok(())
}

/// A rebind ends the window, whichever state it was in.
pub(crate) fn delete_row(tx: &Transaction<'_>, seat: &SeatId) -> Result<(), ApiError> {
    tx.execute(
        "DELETE FROM reversible_retirements WHERE seat_id=?1",
        [seat.as_str()],
    )
    .map_err(store_error)?;
    Ok(())
}

/// A retiring seat is no longer waiting.
pub(crate) fn delete_unretired_row(tx: &Transaction<'_>, seat: &SeatId) -> Result<(), ApiError> {
    tx.execute(
        "DELETE FROM reversible_retirements WHERE seat_id=?1 AND state='unretired'",
        [seat.as_str()],
    )
    .map_err(store_error)?;
    Ok(())
}

/// A key's owner acts on its seat. No key row is `Unauthorized`: keys are local, so a store
/// restored from its own log has reversible seats only an operator can finalize.
fn require_key_owner(
    conn: &Connection,
    seat: &SeatId,
    author: &ServiceAuthorId,
) -> Result<(), ApiError> {
    match continuity_keys::key_of_seat(conn, seat)? {
        Some(row) if row.owner == *author => Ok(()),
        _ => Err(api_error(
            ErrorCode::Unauthorized,
            "seat's continuity key is not this service's",
        )),
    }
}

fn no_retirement() -> ApiError {
    api_error(
        ErrorCode::NotFound,
        "seat has no retirement in this state to act on",
    )
}

/// Whether a copy of this episode's `kind` already took effect on this node (TRUST-POLICY N7
/// re-issue); a no-op copy is never marked, so it never guards its episode. A copy re-issued by fork
/// repair carries the same seat and `retired_at`, and the mark ([`mark_episode_effect`]) is on the
/// append-only journal, so the guard survives the row and seat state the episode's end removed.
/// `None` for `this_row` (no journal row) is `false`. The query scans the journal, once per apply of
/// these two rare kinds. `retired_at` names the episode, so a later episode of the seat is never
/// guarded by this one unless it starts in the same millisecond.
pub(crate) fn episode_effect_recorded(
    tx: &Transaction<'_>,
    kind: &str,
    seat: &SeatId,
    retired_at: i64,
    this_row: Option<i64>,
) -> Result<bool, ApiError> {
    let Some(this_row) = this_row else {
        return Ok(false);
    };
    tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM journal WHERE kind=?1 AND episode_effect=1 AND ingest_seq<>?2 \
           AND json_extract(CAST(payload AS TEXT),'$.seat')=?3 \
           AND json_extract(CAST(payload AS TEXT),'$.retired_at')=?4)",
        params![kind, this_row, seat.as_str(), retired_at],
        |r| r.get(0),
    )
    .map_err(store_error)
}

/// Records that the copy on journal row `this_row` took its episode's effect on this node, for
/// [`episode_effect_recorded`]. `None` (no journal row) does nothing.
pub(crate) fn mark_episode_effect(
    tx: &Transaction<'_>,
    this_row: Option<i64>,
) -> Result<(), ApiError> {
    if let Some(this_row) = this_row {
        tx.execute(
            "UPDATE journal SET episode_effect=1 WHERE ingest_seq=?1",
            [this_row],
        )
        .map_err(store_error)?;
    }
    Ok(())
}

/// The journal rows of every applied `seat.unretired` copy of the episode (any mark).
const KEPT_UNRETIRES_SQL: &str = "FROM journal WHERE kind=?1 AND apply_state='applied' \
     AND json_extract(CAST(payload AS TEXT),'$.seat')=?2 \
     AND json_extract(CAST(payload AS TEXT),'$.retired_at')=?3";

/// The oldest applied `seat.unretired` of this episode other than `this_row`: an un-retire that
/// arrived before its episode's `seat.retired_reversible` changed nothing and was kept in the
/// journal, so that retire reverses itself when it lands.
pub(crate) fn kept_unretire(
    tx: &Transaction<'_>,
    seat: &SeatId,
    retired_at: i64,
    this_row: Option<i64>,
) -> Result<Option<SeatUnretiredPayload>, ApiError> {
    let payload: Option<Vec<u8>> = tx
        .query_row(
            &format!(
                "SELECT payload {KEPT_UNRETIRES_SQL} AND ingest_seq IS NOT ?4 ORDER BY ingest_seq LIMIT 1"
            ),
            params![SeatUnretired::NAME, seat.as_str(), retired_at, this_row],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    payload
        .map(|payload| {
            serde_json::from_slice(&payload).map_err(|_| {
                api_error(
                    ErrorCode::StoreCorrupt,
                    "undecodable seat.unretired payload",
                )
            })
        })
        .transpose()
}

/// Marks every kept `seat.unretired` of the episode once its retire reversed itself.
pub(crate) fn mark_kept_unretires(
    tx: &Transaction<'_>,
    seat: &SeatId,
    retired_at: i64,
) -> Result<(), ApiError> {
    tx.execute(
        &format!(
            "UPDATE journal SET episode_effect=1 WHERE ingest_seq IN (SELECT ingest_seq {KEPT_UNRETIRES_SQL})"
        ),
        params![SeatUnretired::NAME, seat.as_str(), retired_at],
    )
    .map_err(store_error)?;
    Ok(())
}

/// Whether the seat is frozen in the episode `retired_at` names.
pub(crate) fn frozen_in(
    conn: &Connection,
    seat: &SeatId,
    retired_at: i64,
) -> Result<bool, ApiError> {
    Ok(frozen_episode(conn, seat)? == Some(retired_at))
}

/// Whether a local row waits after the un-retire of episode `retired_at` at `unretired_at`: the
/// home's own `UnretireSeat` (or a reversal) already made that fact's transition.
pub(crate) fn waiting_in(
    conn: &Connection,
    seat: &SeatId,
    retired_at: i64,
    unretired_at: i64,
) -> Result<bool, ApiError> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM reversible_retirements WHERE seat_id=?1 AND state='unretired' \
           AND cutover_at=?2 AND unretired_at=?3)",
        params![seat.as_str(), retired_at, unretired_at],
        |r| r.get(0),
    )
    .map_err(store_error)
}

/// The SQL spelling of the time an obligation created at `created` spent frozen in the window
/// from `retired` to `unretired`: `unretired - max(retired, created)`. A caller applies it only
/// where `created <= unretired`. [`shift_pending_deadlines`] and [`window_shift_ms`] share it.
fn frozen_in_window_sql(retired: &str, unretired: &str, created: &str) -> String {
    format!("({unretired}-MAX({retired},{created}))")
}

/// The time an obligation of `seat` created at `created_at` (origin clock) spent frozen, summed
/// over the seat's un-retired windows in `reversible_windows`: what a late arrival (an obligation
/// first written after an un-retire it predates) adds to its deadline, so it reads as every node's
/// copy that was present at the un-retire does.
pub(crate) fn window_shift_ms(
    conn: &Connection,
    seat: &SeatId,
    created_at: i64,
) -> Result<i64, ApiError> {
    conn.query_row(
        &format!(
            "SELECT COALESCE(SUM({}),0) FROM reversible_windows WHERE seat_id=?1 AND unretired_at>=?2",
            frozen_in_window_sql("retired_at", "unretired_at", "?2")
        ),
        params![seat.as_str(), created_at],
        |r| r.get(0),
    )
    .map_err(store_error)
}

/// Records the window from `retired_at` to `unretired_at` (clamped to no earlier than
/// `retired_at`) and moves what the seat owes forward by its frozen time. Every un-retire on a node
/// takes this: home `UnretireSeat`, a peer's apply and an own-log replay.
fn resume_obligations(
    tx: &Transaction<'_>,
    seat: &SeatId,
    retired_at: i64,
    unretired_at: i64,
) -> Result<(), ApiError> {
    let unretired_at = unretired_at.max(retired_at);
    tx.execute(
        "INSERT OR IGNORE INTO reversible_windows(seat_id,retired_at,unretired_at) VALUES (?1,?2,?3)",
        params![seat.as_str(), retired_at, unretired_at],
    )
    .map_err(store_error)?;
    shift_pending_deadlines(tx, seat, retired_at, unretired_at)
}

/// Every deadline the seat owes moves forward by the time that obligation spent frozen in the
/// window from `retired_at` to `unretired_at`, `unretired_at - max(retired_at, created)`, so the
/// remaining time resumes; one already past stays past by the same margin. Set-based, so a large
/// backlog fits the call budget. Touches nothing the seat is owed: its own outgoing requests keep
/// their deadlines. A logical receipt gets a `receipt_state` row carrying its shifted deadline;
/// `receipt_due_sparse` follows `receipt_state` through its triggers. Only an obligation created by
/// `unretired_at` moves: a replayed `seat.unretired` runs after obligations the un-retire never
/// froze. Every path passes the fact's `unretired_at` (home `UnretireSeat`, a peer's apply and an
/// own-log replay, through [`resume_obligations`]), read against the origin's clock; a replicated
/// message's `decision_at` is the origin's `created_at`, so every node's copy of an obligation
/// shifts or stays alike. One that arrives later shifts when first written ([`window_shift_ms`]).
pub(crate) fn shift_pending_deadlines(
    conn: &Connection,
    seat: &SeatId,
    retired_at: i64,
    unretired_at: i64,
) -> Result<(), ApiError> {
    // The un-retire decision itself, on the seat's home node: a legal replicated write.
    #[cfg(feature = "test-support")]
    let _scope = crate::test_support::write_guard::enter_replicated_write_scope();
    let frozen_message = frozen_in_window_sql("?2", "?3", "m.decision_at");
    for table in ["receipts", "receipt_state"] {
        conn.execute(
            &format!(
                "UPDATE {table} SET deadline_at=deadline_at+(SELECT {frozen_message} FROM messages m WHERE m.id={table}.message_id) \
                 WHERE seat_id=?1 AND state='pending' AND deadline_at IS NOT NULL \
                 AND EXISTS(SELECT 1 FROM messages m WHERE m.id={table}.message_id AND m.decision_at<=?3)"
            ),
            params![seat.as_str(), retired_at, unretired_at],
        )
        .map_err(store_error)?;
    }
    // The stored deadline of a logical receipt is its send-time projection, the shared SQL
    // spelling of `effective_receipt_with_key_deadline` for a pending, unretired seat.
    conn.execute(
        &format!(
            "INSERT INTO receipt_state(message_id,seat_id,state,available_at,deadline_at) \
             SELECT sm.message_id,pr.seat_id,'pending',{LOGICAL_PENDING_AVAILABLE_AT_SQL},{LOGICAL_PENDING_DEADLINE_SQL}+{} \
             FROM prepared_recipients pr JOIN send_manifests sm ON sm.preparation_id=pr.preparation_id \
             LEFT JOIN receipt_send_time_cutovers cut ON cut.instance_id=sm.instance_id \
             WHERE pr.seat_id=?1 AND pr.ack_required=1 AND sm.decision_at<=?3 \
             ON CONFLICT(message_id,seat_id) DO UPDATE SET deadline_at=excluded.deadline_at \
             WHERE receipt_state.state='pending' AND receipt_state.deadline_at IS NULL",
            frozen_in_window_sql("?2", "?3", "sm.decision_at")
        ),
        params![seat.as_str(), retired_at, unretired_at],
    )
    .map_err(store_error)?;
    conn.execute(
        &format!(
            "UPDATE invitations SET deadline_at=deadline_at+{} WHERE seat_id=?1 AND state='pending' AND created_at<=?3",
            frozen_in_window_sql("?2", "?3", "created_at")
        ),
        params![seat.as_str(), retired_at, unretired_at],
    )
    .map_err(store_error)?;
    Ok(())
}

/// `UnretireSeat`: the owner undoes a reversible retirement. The seat comes back unresolved with no
/// target and no binding, and waits; nothing is placed and no hold moves. What it held was never
/// released, so memberships and requirement episodes are effective again as they stand.
pub(crate) fn unretire(
    tx: &Transaction<'_>,
    context: &StoreContext,
    instance: &str,
    author: &ServiceAuthorId,
    request: &UnretireSeat,
    at: DecisionInstant,
) -> Result<ServiceResult, ApiError> {
    let seat = &request.seat;
    require_key_owner(tx, seat, author)?;
    let row = read_row(tx, seat)?
        .filter(|row| row.state == RowState::Reversible)
        .ok_or_else(no_retirement)?;
    // Bounded by the published `unretired_at`, as every peer and an own-log replay bound it.
    let generation = unretire_local(tx, instance, seat, row.cutover_at, at.utc)?;
    allocation_decision(tx, seat, "service_unretire", at, &row.inputs, None)?;
    control::emit_if_published::<SeatUnretired>(
        tx,
        context,
        at,
        seat,
        &SeatUnretiredPayload {
            seat: seat.clone(),
            retired_at: row.cutover_at,
            unretired_at: at.utc.0,
        },
    )?;
    Ok(ServiceResult::SeatUnretired(SeatUnretiredResult {
        seat: seat.clone(),
        generation: u64::try_from(generation)
            .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative seat generation"))?,
    }))
}

/// The un-retire transition of a local row, without authorization: `UnretireSeat` and an own
/// `seat.unretired` replayed from the log. `unretired_at` is the fact's, and it bounds the shift
/// as in [`shift_pending_deadlines`]. Returns the seat's new generation.
fn unretire_local(
    tx: &Transaction<'_>,
    instance: &str,
    seat: &SeatId,
    retired_at: i64,
    unretired_at: UtcMillis,
) -> Result<i64, ApiError> {
    let mut generation = 0_i64;
    // The seat first: the terminal trigger is relaxed only while the row reads `reversible`.
    schema::apply_eligibility_transition(tx, instance, |tx| {
        #[cfg(feature = "test-support")]
        let _scope = crate::test_support::write_guard::enter_replicated_write_scope();
        generation = tx
            .query_row(
                "UPDATE seats SET state='unresolved',unresolved_reason='other',target_id=NULL,\
                 structural_terminal_id=NULL,structural_incarnation=NULL,structural_incarnation_kind=NULL,\
                 structural_host_boot=NULL,structural_host_epoch=NULL,structural_connection_epoch=NULL,\
                 structural_observation_sequence=NULL,retired_at=NULL,retired_seq=NULL,generation=generation+1 \
                 WHERE id=?1 AND state='retired' RETURNING generation",
                [seat.as_str()],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        Ok(true)
    })?;
    tx.execute(
        "UPDATE reversible_retirements SET state='unretired',unretired_at=?2 WHERE seat_id=?1",
        params![seat.as_str(), unretired_at.0],
    )
    .map_err(store_error)?;
    // An unretired seat has no target to come back to: its old shell proves nothing.
    super::delete_shell_fingerprint(tx, seat)?;
    resume_obligations(tx, seat, retired_at, unretired_at.0)?;
    // The fence swept the warnings of the episode it found open: this absence is a new one.
    tx.execute(
        "UPDATE seats SET unavailability_open=0 WHERE id=?1",
        [seat.as_str()],
    )
    .map_err(store_error)?;
    schema::ensure_unavailability_episode(tx, seat)?;
    control::settle_local_seat(tx, seat)?;
    schema::bump_filter_revision(tx, instance, "directory", "all")?;
    control::bump_member_directory(tx, instance, seat)?;
    Ok(generation)
}

/// `seat.unretired` on this node. A no-op unless the fact undoes the current episode. A local row
/// (own-log replay) takes [`unretire_local`]; a remote row reads `remote` again, its deadlines on
/// this node shift by the same amount, and its row goes, because a peer has no waiting state.
/// Both bound the shift by the payload's `unretired_at`, as the home `UnretireSeat` does. Returns
/// whether it took either transition.
pub(crate) fn unretire_by_fact(
    tx: &Transaction<'_>,
    instance: &str,
    local_row: bool,
    payload: &SeatUnretiredPayload,
) -> Result<bool, ApiError> {
    let seat = &payload.seat;
    if frozen_episode(tx, seat)? != Some(payload.retired_at) {
        return Ok(false);
    }
    let unretired_at = UtcMillis(payload.unretired_at);
    if local_row {
        unretire_local(tx, instance, seat, payload.retired_at, unretired_at)?;
        return Ok(true);
    }
    // The seat first: the terminal trigger is relaxed only while the row reads `reversible`.
    schema::apply_eligibility_transition(tx, instance, |tx| {
        tx.execute(
            "UPDATE seats SET state='remote',retired_at=NULL,retired_seq=NULL WHERE id=?1 AND state='retired'",
            [seat.as_str()],
        )
        .map(|changed| changed == 1)
        .map_err(store_error)
    })?;
    delete_row(tx, seat)?;
    resume_obligations(tx, seat, payload.retired_at, payload.unretired_at)?;
    schema::bump_filter_revision(tx, instance, "directory", "all")?;
    control::bump_member_directory(tx, instance, seat)?;
    Ok(true)
}

/// `seat.retired` on a seat that already reads retired: when it finalizes the current episode, the
/// deferred job with `retired_seq` advanced to this fact, so the job also settles what a window
/// fact wrote. Returns whether it did: any other retired row, or another episode, is a no-op. A row
/// with no closure inputs (a peer's, or one replayed from the own log) yields the replicated job.
/// It acts on state alone (a final row is terminal), so a re-issued copy, or one arriving after the
/// seat was finally retired, changes nothing and writes no second job.
pub(crate) fn finalize_by_fact(
    tx: &Transaction<'_>,
    instance: &str,
    seat: &SeatId,
    retired_at: i64,
    retired_seq: i64,
) -> Result<bool, ApiError> {
    if frozen_episode(tx, seat)? != Some(retired_at) {
        return Ok(false);
    }
    let row = read_row(tx, seat)?.ok_or_else(no_retirement)?;
    delete_row(tx, seat)?;
    schema::apply_eligibility_transition(tx, instance, |tx| {
        tx.execute(
            "UPDATE seats SET retired_seq=?2 WHERE id=?1 AND state='retired'",
            params![seat.as_str(), retired_seq],
        )
        .map_err(store_error)?;
        Ok(true)
    })?;
    let cutover = UtcMillis(retired_at);
    if row.inputs.boot.is_empty() {
        control::insert_replicated_retirement_job(tx, seat, cutover)?;
    } else {
        control::insert_retirement_job(tx, seat, cutover, &row.inputs)?;
    }
    Ok(true)
}

/// `FinalizeRetirement` and the operator's `seat retire` on a seat with a row. Reversible: the job
/// the fence deferred, with the original cutover and evidence; `retired_seq` advances to this
/// decision so the job's fence also covers what was applied during the window. Unretired: an
/// ordinary final retirement, with reason `Operator` so it cannot go reversible again. Either way
/// `insert_retirement_job` releases the key.
pub(crate) fn finalize(
    tx: &Transaction<'_>,
    context: &StoreContext,
    instance: &str,
    by: FinalizeBy<'_>,
    seat: &SeatId,
    at: DecisionInstant,
) -> Result<(), ApiError> {
    if let FinalizeBy::Service(author) = by {
        require_key_owner(tx, seat, author)?;
    }
    let row = read_row(tx, seat)?.ok_or_else(no_retirement)?;
    delete_row(tx, seat)?;
    match row.state {
        RowState::Reversible => {
            let retired_seq = schema::next_decision_seq(tx, instance)? as i64;
            schema::apply_eligibility_transition(tx, instance, |tx| {
                #[cfg(feature = "test-support")]
                let _scope = crate::test_support::write_guard::enter_replicated_write_scope();
                tx.execute(
                    "UPDATE seats SET retired_seq=?2 WHERE id=?1 AND state='retired'",
                    params![seat.as_str(), retired_seq],
                )
                .map_err(store_error)?;
                Ok(true)
            })?;
            let cutover = UtcMillis(row.cutover_at);
            control::insert_retirement_job(tx, seat, cutover, &row.inputs)?;
            control::emit_seat_retired(tx, context, at, seat.clone(), row.reason, cutover)?;
        }
        RowState::Unretired => {
            control::begin_retirement_authored(
                tx,
                context,
                at,
                RetireReason::Operator,
                seat.clone(),
                instance,
                RetirementInputs {
                    closure_evidence: None,
                    absence_evidence: None,
                    ..row.inputs.clone()
                },
            )?;
        }
    }
    let (kind, label) = match by {
        FinalizeBy::Service(_) => ("service_finalize", None),
        FinalizeBy::Operator(label) => ("operator_retire", Some(label)),
    };
    allocation_decision(tx, seat, kind, at, &row.inputs, label)
}

fn allocation_decision(
    tx: &Transaction<'_>,
    seat: &SeatId,
    kind: &str,
    at: DecisionInstant,
    inputs: &RetirementInputs,
    label: Option<&str>,
) -> Result<(), ApiError> {
    tx.execute(
        "INSERT INTO allocation_decisions(host_id,target_id,seat_id,kind,decided_at,host_boot,epoch,generation,operator_label) \
         SELECT host_id,?1,id,?2,?3,?4,?5,?6,?7 FROM seats WHERE id=?8",
        params![
            inputs.target,
            kind,
            at.utc.0,
            inputs.boot,
            inputs.epoch,
            inputs.generation,
            label,
            seat.as_str()
        ],
    )
    .map_err(store_error)?;
    Ok(())
}

#[cfg(test)]
#[path = "../../../tests/store/reversible_retirement.rs"]
mod tests;
