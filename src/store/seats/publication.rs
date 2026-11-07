//! Seat publication: the per-transaction queue and the materialization of the seat kinds
//! (`seat.created`, `seat.retired`, `seat.retired_reversible`, `seat.unretired`).

use crate::store::seats::LOCAL_SEAT;
use crate::{
    protocol::{
        ids::{SeatId, SeatName},
        results::{ApiError, ErrorCode},
    },
    store::{
        connection::{DecisionInstant, StoreContext, api_error, store_error},
        journal,
        materialize::MaterializeInput,
        schema,
        seats::reversible,
    },
    sync::facts::{
        FactKind,
        payload::SeatKind,
        seat::{
            SeatCreated, SeatCreatedPayload, SeatRetiredPayload, SeatRetiredReversible,
            SeatRetiredReversiblePayload, SeatUnretired, SeatUnretiredPayload,
        },
    },
};
use rusqlite::{OptionalExtension, Transaction, params};

// Seats whose first occupant binding was enrolled in the running write
// transaction and that are not yet published (`seat.created`).
//
// `bindings::enroll_with_hooks` takes a bare connection and has no store
// context to author with, so it only queues the seat here; the write runner
// (`StoreContext::execute_decision`) publishes the queue in the same
// transaction before it commits (`PublicationScope::publish`).
thread_local! {
    static PENDING_PUBLICATION: std::cell::RefCell<Option<Vec<SeatId>>> =
        const { std::cell::RefCell::new(None) };
}

/// The publication queue of one write transaction.
///
/// Responsibility
/// - Owns: the thread-local queue `bindings::enroll_with_hooks` fills
///   ([`queue_publication`]) and its drain into [`ensure_published`] at the end
///   of the transaction's apply step.
/// - Must not: author outside the transaction that queued, keep a seat past
///   its transaction (dropping the scope clears the queue, so a failed
///   transaction publishes nothing), or decide the kind (that is
///   [`ensure_published`]).
/// - Role: opened by the write runners in `store::connection`; a write outside
///   them (a plain-connection unit test) queues nothing and publishes nothing.
pub(crate) struct PublicationScope {
    owner: bool,
}

impl PublicationScope {
    /// Opens the queue; a nested scope joins the outer one.
    pub(crate) fn begin() -> Self {
        let owner = PENDING_PUBLICATION.with(|q| {
            let mut q = q.borrow_mut();
            if q.is_some() {
                false
            } else {
                *q = Some(Vec::new());
                true
            }
        });
        Self { owner }
    }

    /// Publishes every queued seat (each through [`ensure_published`], so an
    /// already published seat authors nothing). A store without an open
    /// journal publishes nothing.
    pub(crate) fn publish(
        &self,
        tx: &Transaction<'_>,
        ctx: &StoreContext,
        decision: DecisionInstant,
    ) -> Result<(), ApiError> {
        if !self.owner {
            return Ok(());
        }
        let queued = PENDING_PUBLICATION
            .with(|q| q.borrow_mut().as_mut().map(std::mem::take))
            .unwrap_or_default();
        if !ctx.journal_open() {
            return Ok(());
        }
        for seat in queued {
            ensure_published(tx, ctx, decision, &seat)?;
        }
        Ok(())
    }
}

impl Drop for PublicationScope {
    fn drop(&mut self) {
        if self.owner {
            PENDING_PUBLICATION.with(|q| *q.borrow_mut() = None);
        }
    }
}

/// Queues `seat` for publication when a [`PublicationScope`] is open.
pub(crate) fn queue_publication(seat: &SeatId) {
    PENDING_PUBLICATION.with(|q| {
        if let Some(queue) = q.borrow_mut().as_mut() {
            queue.push(seat.clone());
        }
    });
}

/// Publishes a local seat once: authors `seat.created` so other nodes list it.
/// Returns whether it authored.
///
/// Responsibility
/// - Owns: the decision "this local seat is not yet published" (`is_local = 1`,
///   `published = 0`), the published `display_name` (the seat's `seats.name`, `None`
///   for an unnamed seat), the published kind (`schema::open_binding_role`:
///   `Some("human")` is `human`, anything else, no open binding included, is
///   `agent`) and the one `journal::author::<SeatCreated>` call; the handler
///   then sets `published = 1`.
/// - Does not: run for a remote or already published seat (those return
///   `false` and write nothing), run on the local-only route (managed
///   membership never publishes), or allocate a seat (allocation by
///   observation and the operator fresh seat stay local inserts that author
///   nothing).
/// - Role: called by `bindings::enroll_with_hooks` (through the write runner's
///   [`PublicationScope`]) when a seat is resolved to an occupant, and by
///   `FactRoute::Journal` for every referenced local seat as an idempotent
///   backstop. Commands never call it themselves.
///
/// Accepted limit: a seat's published kind is fixed. If its occupant later
/// switches between human and agent, other nodes keep the published kind; only
/// the ACK expectation of remote recipients depends on it.
pub fn ensure_published(
    tx: &Transaction<'_>,
    ctx: &StoreContext,
    decision: DecisionInstant,
    seat: &SeatId,
) -> Result<bool, ApiError> {
    let row: Option<(bool, bool, Option<String>)> = tx
        .query_row(
            "SELECT is_local, published, name FROM seats WHERE id=?1",
            [seat.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(store_error)?;
    let Some((true, false, name)) = row else {
        return Ok(false);
    };
    let kind = match schema::open_binding_role(tx, seat)? {
        Some("human") => SeatKind::Human,
        _ => SeatKind::Agent,
    };
    journal::author::<SeatCreated>(
        tx,
        ctx,
        decision,
        &SeatCreatedPayload {
            seat: seat.clone(),
            kind,
            display_name: name,
        },
    )?;
    Ok(true)
}

/// Materializes `seat.created` (fact-kinds design section 4), shared by the
/// handler for both the authoring node and every other node.
///
/// - Row exists and is local: `published = 1` (the authoring node).
/// - Row exists and is remote: no-op (a second `seat.created` for one seat,
///   even with another kind, keeps the first: the root accepted fork limit).
/// - Row absent, origin is another node: insert the remote row. Its `name` is the
///   payload's `display_name` when that parses as a `SeatName`, else NULL: a peer's
///   value is never rejected, only not stored (the name is display and selector
///   text, never authority).
/// - Row absent, origin is this node (an own old incarnation pulled back after
///   a wipe): [`restore_own_seat`]. A seat with a `display_name` goes
///   unresolved onto the named host at target = name, nameless (held) when a
///   non-retired seat of that name exists; a seat without one goes onto the
///   one host kind of the node, else the entry parks on `wait:host`.
pub fn materialize_seat_created(
    tx: &Transaction<'_>,
    m: &MaterializeInput<'_>,
    payload: &SeatCreatedPayload,
) -> Result<(), ApiError> {
    let id = payload.seat.as_str();
    let existing: Option<bool> = tx
        .query_row("SELECT is_local FROM seats WHERE id=?1", [id], |r| r.get(0))
        .optional()
        .map_err(store_error)?;
    match existing {
        Some(true) => {
            tx.execute(
                "UPDATE seats SET published=1 WHERE id=?1 AND published=0",
                [id],
            )
            .map_err(store_error)?;
        }
        Some(false) => {}
        None if !m.local_origin => {
            let kind = match payload.kind {
                SeatKind::Agent => "agent",
                SeatKind::Human => "human",
            };
            let name = payload
                .display_name
                .as_deref()
                .and_then(|n| SeatName::parse(n).ok());
            tx.execute(
                "INSERT INTO seats(id,instance_id,state,role,generation,created_at,home_node,is_local,remote_kind,published,name) \
                 VALUES(?1,(SELECT id FROM local_node),'remote','native',0,?2,?3,0,?4,1,?5)",
                params![
                    id,
                    m.origin_ms,
                    m.origin_node.to_string(),
                    kind,
                    name.as_ref().map(SeatName::as_str)
                ],
            )
            .map_err(store_error)?;
        }
        None => restore_own_seat(tx, m, payload)?,
    }
    Ok(())
}

/// The local-origin restore of `seat.created` (TRUST-POLICY D12, SA4): an own old incarnation
/// pulled back after a wipe, inserted as a local `unresolved` seat.
///
/// - `display_name` parses as a `SeatName`: the named host, unresolved, `target_id = name`. When
///   a non-retired local seat of that name exists there (resolved or unresolved, the rows
///   `seats_live_name` covers) the row is inserted with `name` NULL instead, so the unique index
///   cannot be violated: a named-host seat with a NULL name is the held marker and its held name
///   is its own `seat.created.display_name` (see [`held_name`]). No merge and no pick: the name
///   keeps resolving to the existing seat until [`hand_back_name`] runs in its retirement.
/// - Own-log order is `created`, `retired`, `created`: the replayed `seat.retired` of an
///   earlier incarnation retires its restored row ([`materialize_seat_retired`]) before a later
///   `seat.created` of the same name is restored, so the successor finds the name free.
/// - No name: the `herdr_hosts` row with the greatest `last_attached_at`, only when every
///   non-named host row of the node has one kind (a mixed node would send the seat to the wrong
///   environment). Otherwise, and with no such row, `HostUnavailable`, which the handler maps to
///   `FactError::Wait { on: Host }`: the entry parks on `wait:host` and is re-attempted at
///   startup and when a host attaches.
fn restore_own_seat(
    tx: &Transaction<'_>,
    m: &MaterializeInput<'_>,
    payload: &SeatCreatedPayload,
) -> Result<(), ApiError> {
    let id = payload.seat.as_str();
    let name = payload
        .display_name
        .as_deref()
        .and_then(|n| SeatName::parse(n).ok());
    if let Some(name) = name {
        let named: Option<i64> = tx
            .query_row(
                "SELECT host_id FROM herdr_hosts WHERE kind='named'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(store_error)?;
        let Some(named) = named else {
            return Err(api_error(
                ErrorCode::HostUnavailable,
                "own named seat restore needs the named host",
            ));
        };
        let taken: bool = tx
            .query_row(
                &format!("SELECT EXISTS(SELECT 1 FROM seats WHERE host_id=?1 AND {LOCAL_SEAT} AND name=?2 AND state<>'retired')"),
                params![named, name.as_str()],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        tx.execute(
            "INSERT INTO seats(id,instance_id,state,unresolved_reason,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id,published,name) \
             VALUES(?1,(SELECT id FROM local_node),'unresolved','other','native',?2,1,1,?3,(SELECT id FROM local_node),1,?4,1,?5)",
            params![
                id,
                name.as_str(),
                m.origin_ms,
                named,
                (!taken).then(|| name.as_str())
            ],
        )
        .map_err(store_error)?;
        return Ok(());
    }
    let single_kind: bool = tx
        .query_row(
            "SELECT count(DISTINCT kind)<=1 FROM herdr_hosts WHERE kind<>'named'",
            [],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    let host: Option<i64> = if single_kind {
        tx.query_row(
            "SELECT host_id FROM herdr_hosts WHERE kind<>'named' ORDER BY last_attached_at DESC, host_id DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?
    } else {
        None
    };
    let Some(host) = host else {
        return Err(api_error(
            ErrorCode::HostUnavailable,
            if single_kind {
                "own seat restore needs an attached host"
            } else {
                "own seat restore is held: attached hosts are of more than one kind"
            },
        ));
    };
    tx.execute(
        "INSERT INTO seats(id,instance_id,state,unresolved_reason,role,generation,created_at,home_node,is_local,host_id,published) \
         VALUES(?1,(SELECT id FROM local_node),'unresolved','other','native',1,?2,(SELECT id FROM local_node),1,?3,1)",
        params![id, m.origin_ms, host],
    )
    .map_err(store_error)?;
    Ok(())
}

/// The name a held seat waits for: the `display_name` of its own `seat.created` fact. `None` for
/// a seat that is not held (a seat with a name, a seat off the named host, a remote or retired
/// seat).
pub(crate) fn held_name(
    conn: &rusqlite::Connection,
    seat: &str,
) -> Result<Option<String>, ApiError> {
    conn.query_row(
        &format!("SELECT {HELD_NAME_SQL} FROM seats s WHERE s.id=?1 AND {HELD_ROW_SQL}"),
        [seat],
        |r| r.get(0),
    )
    .optional()
    .map(Option::flatten)
    .map_err(store_error)
}

/// One held seat and the live seat that keeps its name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeldName {
    pub seat: SeatId,
    pub name: String,
    pub holder: Option<SeatId>,
}

/// Every held seat (oldest first) with the non-retired seat that currently owns its name: what
/// `doctor` lists for operator repair.
pub fn held_names(conn: &rusqlite::Connection) -> Result<Vec<HeldName>, ApiError> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT s.id,{HELD_NAME_SQL},(SELECT o.id FROM seats o WHERE o.host_id=s.host_id AND o.is_local=1 AND o.state<>'retired' AND o.name={HELD_NAME_SQL}) \
             FROM seats s WHERE {HELD_ROW_SQL} ORDER BY s.created_at,s.id"
        ))
        .map_err(store_error)?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(store_error)?;
    let mut held = Vec::new();
    for row in rows {
        let (seat, name, holder) = row.map_err(store_error)?;
        if let Some(name) = name {
            held.push(HeldName {
                seat: SeatId::new(seat),
                name,
                holder: holder.map(SeatId::new),
            });
        }
    }
    Ok(held)
}

/// A held seat: local, off the name index (`name` NULL), live, on the named host.
const HELD_ROW_SQL: &str = "s.is_local=1 AND s.name IS NULL AND s.state<>'retired' \
     AND s.host_id IN (SELECT host_id FROM herdr_hosts WHERE kind='named')";

/// The held name of the row `s`: its own `seat.created` fact's `display_name`.
const HELD_NAME_SQL: &str = "(SELECT json_extract(CAST(j.payload AS TEXT),'$.display_name') FROM journal j WHERE j.kind='seat.created' AND j.creates=s.id LIMIT 1)";

/// The retire-transaction hand-back (TRUST-POLICY D12): when the retired local `seat` owned a
/// name, the oldest held seat of that name (by `created_at`, then id) takes it. The caller has
/// already retired `seat`, so the unique index allows the update; named continuity reattaches the
/// held seat on the next resolve of the name. Retiring a held seat (no name) changes nothing, and
/// no name moves while two live candidates exist.
pub(crate) fn hand_back_name(
    tx: &Transaction<'_>,
    instance: &str,
    seat: &SeatId,
) -> Result<(), ApiError> {
    let owned: Option<(i64, String)> = tx
        .query_row(
            &format!("SELECT host_id,name FROM seats WHERE id=?1 AND {LOCAL_SEAT} AND name IS NOT NULL AND state='retired'"),
            [seat.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(store_error)?;
    let Some((host, name)) = owned else {
        return Ok(());
    };
    let heir: Option<String> = tx
        .query_row(
            &format!("SELECT s.id FROM seats s WHERE s.host_id=?1 AND {HELD_ROW_SQL} AND {HELD_NAME_SQL}=?2 ORDER BY s.created_at,s.id LIMIT 1"),
            params![host, name],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    let Some(heir) = heir else {
        return Ok(());
    };
    tx.execute("UPDATE seats SET name=?1 WHERE id=?2", params![name, heir])
        .map_err(store_error)?;
    schema::bump_filter_revision(tx, instance, "directory", "all")?;
    Ok(())
}

/// Materializes `seat.retired` (fact-kinds design section 4), shared by the
/// handler for both the authoring node and every other node.
///
/// - Local seat: the authoring node's retirement fence already retired the
///   row and its job wrote the audit, so a live echo finds `state = 'retired'`
///   and changes nothing. A local row restored by own-log replay is not yet
///   retired and takes the same transition, then [`hand_back_name`] gives its
///   name to a held row, as the live retire does. A remote-origin fact never
///   retires a local seat (`require_author` refuses it).
/// - Remote seat, not yet retired: `state = 'retired'`, `retired_at` from the
///   payload, `retired_seq = decision_seq`; the send-eligibility and directory
///   revisions move and a `retirements` row with `origin = 'replicated'` lets
///   the retirement job (`advance_retirement`) settle the seat's pending
///   invitations and receipts as `recipient_retired` and close its memberships.
/// - Remote seat already retired: a no-op (no second row).
/// - Seat already retired with a `reversible` row of this `retired_at`: the deferred job
///   ([`reversible::finalize_by_fact`]). Another episode's row: a no-op.
/// - Local seat with an `unretired` row (own-log replay): the row goes with the transition.
/// - Unknown seat: `InvalidRequest` (`needs` makes this the overflow case).
///
/// Per episode the outcome does not depend on order: before its `seat.retired_reversible` it
/// retires finally and that retire is then a no-op, after it the deferred job runs, and a re-issued
/// copy changes nothing; the seat ends finally retired with one job.
pub fn materialize_seat_retired(
    tx: &Transaction<'_>,
    m: &MaterializeInput<'_>,
    payload: &SeatRetiredPayload,
) -> Result<(), ApiError> {
    let Some(local_row) = retirement_fact_row(tx, m, &payload.seat)? else {
        return Ok(());
    };
    let instance = local_instance(tx)?;
    let retired_seq = decision_seq(m)?;
    if !retire_row(
        tx,
        &instance,
        &payload.seat,
        local_row,
        payload.retired_at,
        retired_seq,
    )? {
        reversible::finalize_by_fact(
            tx,
            &instance,
            &payload.seat,
            payload.retired_at,
            retired_seq,
        )?;
        return Ok(());
    }
    crate::store::control::insert_replicated_retirement_job(
        tx,
        &payload.seat,
        crate::protocol::time::UtcMillis(payload.retired_at),
    )?;
    if local_row {
        hand_back_name(tx, &instance, &payload.seat)?;
    }
    Ok(())
}

/// Materializes `seat.retired_reversible` (spec 4.3), shared by the handler for both the authoring
/// node and every other node. The seat reads retired and gets a `reversible` row with no closure
/// inputs and no job. A row already retired changes nothing: finally retired, the live echo of the
/// fence, or another episode. A local row replayed from the own log takes the fence's transition; a
/// key is never restored (TRUST-POLICY, a store restored from its own log).
///
/// Per episode (TRUST-POLICY N7): a copy whose retire took effect here, the transition or the seat
/// found frozen in its episode (the fence's own echo), is marked on its journal row, and a later copy
/// of an episode with a marked copy is a no-op ([`reversible::episode_effect_recorded`]), so an ended
/// episode never runs again; a copy that changed nothing stays unmarked. When the episode's
/// `seat.unretired` arrived first and was kept ([`reversible::kept_unretire`]), the retire is reversed
/// in the same transaction, with the one deadline shift, and the kept copies are marked.
pub fn materialize_seat_retired_reversible(
    tx: &Transaction<'_>,
    m: &MaterializeInput<'_>,
    payload: &SeatRetiredReversiblePayload,
) -> Result<(), ApiError> {
    let Some(local_row) = retirement_fact_row(tx, m, &payload.seat)? else {
        return Ok(());
    };
    // The guard first, so a re-issued copy of an ended episode never reverses anything.
    if reversible::episode_effect_recorded(
        tx,
        SeatRetiredReversible::NAME,
        &payload.seat,
        payload.retired_at,
        m.ingest_seq,
    )? {
        return Ok(());
    }
    let instance = local_instance(tx)?;
    if retire_row(
        tx,
        &instance,
        &payload.seat,
        local_row,
        payload.retired_at,
        decision_seq(m)?,
    )? {
        reversible::insert_fact_row(tx, &payload.seat, payload.retired_at, payload.reason)?;
        if let Some(unretire) =
            reversible::kept_unretire(tx, &payload.seat, payload.retired_at, m.ingest_seq)?
        {
            reversible::unretire_by_fact(tx, &instance, local_row, &unretire)?;
            reversible::mark_kept_unretires(tx, &payload.seat, payload.retired_at)?;
        }
    } else if !reversible::frozen_in(tx, &payload.seat, payload.retired_at)? {
        // Finally retired, or another episode: a no-op, never a guard.
        return Ok(());
    }
    reversible::mark_episode_effect(tx, m.ingest_seq)
}

/// Materializes `seat.unretired` (spec 4.3), shared by the handler for both the authoring node and
/// every other node: [`reversible::unretire_by_fact`], a no-op outside the current episode and for a
/// copy of an episode whose un-retire already took effect here
/// ([`reversible::episode_effect_recorded`]). A copy that took effect is marked on its journal row,
/// and so is the home's echo of its own un-retire (the local row already waits after it). A copy that
/// found nothing to act on stays unmarked and is kept: its episode's `seat.retired_reversible`, when
/// it lands, reverses itself ([`materialize_seat_retired_reversible`]).
pub fn materialize_seat_unretired(
    tx: &Transaction<'_>,
    m: &MaterializeInput<'_>,
    payload: &SeatUnretiredPayload,
) -> Result<(), ApiError> {
    let Some(local_row) = retirement_fact_row(tx, m, &payload.seat)? else {
        return Ok(());
    };
    if reversible::episode_effect_recorded(
        tx,
        SeatUnretired::NAME,
        &payload.seat,
        payload.retired_at,
        m.ingest_seq,
    )? {
        return Ok(());
    }
    let took = reversible::unretire_by_fact(tx, &local_instance(tx)?, local_row, payload)?
        || (local_row
            && reversible::waiting_in(
                tx,
                &payload.seat,
                payload.retired_at,
                payload.unretired_at,
            )?);
    if took {
        reversible::mark_episode_effect(tx, m.ingest_seq)?;
    }
    Ok(())
}

/// Whether the row a retirement fact acts on is local, `None` for a fact with nothing to do.
fn retirement_fact_row(
    tx: &Transaction<'_>,
    m: &MaterializeInput<'_>,
    seat: &SeatId,
) -> Result<Option<bool>, ApiError> {
    let row: Option<bool> = tx
        .query_row(
            "SELECT is_local FROM seats WHERE id=?1",
            [seat.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    match row {
        // An own fact for a seat that never had a row here: nothing to retire.
        None if m.local_origin => Ok(None),
        None => Err(api_error(ErrorCode::InvalidRequest, "unknown seat")),
        // `require_author` already refused a non-home origin; a remote fact
        // never retires a local seat, which its own fence retires.
        Some(true) if !m.local_origin => Ok(None),
        Some(local) => Ok(Some(local)),
    }
}

fn local_instance(tx: &Transaction<'_>) -> Result<String, ApiError> {
    tx.query_row("SELECT id FROM local_node", [], |r| r.get(0))
        .map_err(store_error)
}

fn decision_seq(m: &MaterializeInput<'_>) -> Result<i64, ApiError> {
    i64::try_from(m.decision_seq)
        .map_err(|_| api_error(ErrorCode::StoreCorrupt, "decision_seq out of range"))
}

/// A retirement fact's constant-size transition of a row not yet retired, with no job; returns
/// whether it changed the row.
fn retire_row(
    tx: &Transaction<'_>,
    instance: &str,
    seat: &SeatId,
    local_row: bool,
    retired_at: i64,
    retired_seq: i64,
) -> Result<bool, ApiError> {
    let id = seat.as_str();
    let cutover = crate::protocol::time::UtcMillis(retired_at);
    let changed = schema::apply_eligibility_transition(tx, instance, |tx| {
        // A local row takes the fence's transition in the fence's order; a remote row keeps the
        // bare statements (it holds no bindings, offers or wake work here).
        if local_row {
            schema::clear_open_unavailability_for_seat(tx, id, cutover)?;
        }
        let generation_bump = if local_row {
            ",generation=generation+1"
        } else {
            ""
        };
        let changed = tx
            .execute(
                &format!("UPDATE seats SET state='retired',unresolved_reason=NULL,unresolved_from_generation_id=NULL,unresolved_prior_binding_generation=NULL{generation_bump},retired_at=?1,retired_seq=?2 WHERE id=?3 AND state!='retired'"),
                params![retired_at, retired_seq, id],
            )
            .map_err(store_error)?;
        if local_row && changed == 1 {
            reversible::delete_unretired_row(tx, seat)?;
            crate::store::control::end_local_seat(tx, seat, cutover)?;
        }
        Ok(changed == 1)
    })?;
    if !changed {
        return Ok(false);
    }
    if local_row {
        crate::store::control::settle_local_seat(tx, seat)?;
    } else {
        crate::store::no_wake_reports::delete_for_retired(tx, seat)?;
    }
    schema::bump_filter_revision(tx, instance, "directory", "all")?;
    crate::store::control::bump_member_directory(tx, instance, seat)?;
    Ok(true)
}

#[cfg(test)]
#[path = "../../../tests/store/named_restore.rs"]
mod named_restore_tests;

#[cfg(all(test, feature = "test-support"))]
#[path = "../../../tests/store/reversible_facts.rs"]
mod reversible_facts_tests;
