//! Seat publication: the per-transaction queue and the `seat.created` / `seat.retired` materialization.

use crate::{
    protocol::{
        ids::SeatId,
        results::{ApiError, ErrorCode},
    },
    store::{
        connection::{DecisionInstant, StoreContext, api_error, store_error},
        journal,
        materialize::MaterializeInput,
        schema,
    },
    sync::facts::{
        payload::SeatKind,
        seat::{SeatCreated, SeatCreatedPayload, SeatRetiredPayload},
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
///   `published = 0`), the published kind (`schema::open_binding_role`:
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
    let row: Option<(bool, bool)> = tx
        .query_row(
            "SELECT is_local, published FROM seats WHERE id=?1",
            [seat.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(store_error)?;
    if row != Some((true, false)) {
        return Ok(false);
    }
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
            display_name: None,
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
/// - Row absent, origin is another node: insert the remote row.
/// - Row absent, origin is this node (an own old incarnation pulled back after
///   a wipe): insert a local `unresolved` seat on the `herdr_hosts` row (never
///   the named host) with the greatest `last_attached_at`; with no such row it is
///   `HostUnavailable "own seat restore needs an attached host"`, which the
///   handler maps to `FactError::Wait { on: Host }`: the entry parks on
///   `wait:host` and is re-attempted at startup and when the first host
///   attaches.
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
            tx.execute(
                "INSERT INTO seats(id,instance_id,state,role,generation,created_at,home_node,is_local,remote_kind,published) \
                 VALUES(?1,(SELECT id FROM local_node),'remote','native',0,?2,?3,0,?4,1)",
                params![id, m.origin_ms, m.origin_node.to_string(), kind],
            )
            .map_err(store_error)?;
        }
        None => {
            let host: Option<i64> = tx
                .query_row(
                    "SELECT host_id FROM herdr_hosts WHERE kind<>'named' ORDER BY last_attached_at DESC, host_id DESC LIMIT 1",
                    [],
                    |r| r.get(0),
                )
                .optional()
                .map_err(store_error)?;
            let Some(host) = host else {
                return Err(api_error(
                    ErrorCode::HostUnavailable,
                    "own seat restore needs an attached host",
                ));
            };
            tx.execute(
                "INSERT INTO seats(id,instance_id,state,unresolved_reason,role,generation,created_at,home_node,is_local,host_id,published) \
                 VALUES(?1,(SELECT id FROM local_node),'unresolved','other','native',1,?2,(SELECT id FROM local_node),1,?3,1)",
                params![id, m.origin_ms, host],
            )
            .map_err(store_error)?;
        }
    }
    Ok(())
}

/// Materializes `seat.retired` (fact-kinds design section 4), shared by the
/// handler for both the authoring node and every other node.
///
/// - Local origin: a no-op. The authoring node's retirement fence already
///   retired the row, and its job writes the audit; an own old incarnation or
///   a re-issued copy changes nothing, and a local-origin fact never retires a
///   remote seat (it is not the seat's home).
/// - Remote seat, not yet retired: `state = 'retired'`, `retired_at` from the
///   payload, `retired_seq = decision_seq`; the send-eligibility and directory
///   revisions move and a `retirements` row with `origin = 'replicated'` lets
///   the retirement job (`advance_retirement`) settle the seat's pending
///   invitations and receipts as `recipient_retired` and close its memberships.
/// - Remote seat already retired: a no-op (no second row).
/// - Unknown seat: `InvalidRequest` (`needs` makes this the overflow case).
pub fn materialize_seat_retired(
    tx: &Transaction<'_>,
    m: &MaterializeInput<'_>,
    payload: &SeatRetiredPayload,
) -> Result<(), ApiError> {
    if m.local_origin {
        return Ok(());
    }
    let id = payload.seat.as_str();
    let row: Option<bool> = tx
        .query_row("SELECT is_local FROM seats WHERE id=?1", [id], |r| r.get(0))
        .optional()
        .map_err(store_error)?;
    match row {
        None => {
            return Err(api_error(ErrorCode::InvalidRequest, "unknown seat"));
        }
        // `require_author` already refused a non-home origin; a local seat is
        // retired by its own fence.
        Some(true) => return Ok(()),
        Some(false) => {}
    }
    let instance: String = tx
        .query_row("SELECT id FROM local_node", [], |r| r.get(0))
        .map_err(store_error)?;
    let retired_seq = i64::try_from(m.decision_seq)
        .map_err(|_| api_error(ErrorCode::StoreCorrupt, "decision_seq out of range"))?;
    let changed = schema::apply_eligibility_transition(tx, &instance, |tx| {
        let changed = tx
            .execute(
                "UPDATE seats SET state='retired',retired_at=?1,retired_seq=?2 WHERE id=?3 AND state!='retired'",
                params![payload.retired_at, retired_seq, id],
            )
            .map_err(store_error)?;
        Ok(changed == 1)
    })?;
    if !changed {
        return Ok(());
    }
    crate::store::no_wake_reports::delete_for_retired(tx, &payload.seat)?;
    schema::bump_filter_revision(tx, &instance, "directory", "all")?;
    crate::store::control::bump_member_directory(tx, &instance, &payload.seat)?;
    let job = crate::protocol::ids::RetirementJobId::new(crate::store::public_ids::fresh(
        tx,
        crate::protocol::ids::prefix::RETIREMENT,
        &[(
            "retirements",
            "id",
            crate::protocol::ids::prefix::RETIREMENT,
        )],
    )?);
    tx.execute(
        "INSERT INTO retirements(id,seat_id,cutover_at,origin) VALUES (?1,?2,?3,'replicated')",
        params![job.as_str(), id, payload.retired_at],
    )
    .map_err(store_error)?;
    Ok(())
}
