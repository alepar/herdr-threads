//! Catch-up rows (spec §7): one per (binding, thread) since V34 (ht-54r.22). Contract surface from
//! ht-1ip.1; the lifecycle and the pushed-attention hold are ht-1ip.6. Every
//! `extension_until` write is ht-1ip.7, made through the hooks in `receipts`:
//! nothing here computes or writes an extension value.
//!
//! Release is a push: every row end (ready, stalled, superseded) allocates a
//! decision sequence stored as `release_seq`, re-publishes the held range
//! (receipts above the frontier take `(release_seq, 0)` as their attention
//! key) and re-derives the seat's wake reasons in the same transaction.
use super::{
    bindings,
    connection::{api_error, store_error},
    receipts, schema, summary, wake,
};
use crate::protocol::{
    ids::{BindingId, SeatId, ThreadId},
    results::{ApiError, ErrorCode},
    summary::{AuthorRole, SummarySettings, is_priority},
    time::UtcMillis,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use std::collections::HashMap;

pub struct CatchUpEntry<'a> {
    /// The caller's binding: the row belongs to it, not to the seat.
    pub binding: &'a BindingId,
    pub seat: &'a SeatId,
    pub thread: &'a ThreadId,
    /// F computed by the summary handler (published head at decision time).
    pub frontier_seq: u64,
    pub now: UtcMillis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatchUpEndReason {
    Ready,
    Stalled,
    Superseded,
}
impl CatchUpEndReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Stalled => "stalled",
            Self::Superseded => "superseded",
        }
    }
}

fn seat_instance(tx: &Transaction<'_>, seat: &SeatId) -> Result<String, ApiError> {
    tx.query_row(
        "SELECT instance_id FROM seats WHERE id=?1",
        [seat.as_str()],
        |r| r.get(0),
    )
    .map_err(store_error)
}

fn thread_instance(tx: &Transaction<'_>, thread: &ThreadId) -> Result<String, ApiError> {
    tx.query_row(
        "SELECT instance_id FROM threads WHERE id=?1",
        [thread.as_str()],
        |r| r.get(0),
    )
    .map_err(store_error)
}

/// Opens the (binding, thread) row or keeps an active one, in the deciding
/// transaction of a `Summary` that returns Work. Returns the frontier in
/// force: an active row's F, unchanged, else `entry.frontier_seq`. The entry
/// binding must be an open binding of the entry seat (every seat generation
/// bump ends the seat's open bindings, so an open binding is current); there
/// is no child-role gate (spec §7, r3). The entry extension hook runs unless
/// this is a re-entry after a stall with no block stored since (spec §8,
/// ht-hqg).
pub fn enter_or_keep(
    tx: &Transaction<'_>,
    entry: &CatchUpEntry<'_>,
    settings: &SummarySettings,
) -> Result<u64, ApiError> {
    let open = bindings::get(tx, entry.binding)?
        .is_some_and(|b| b.ended_at.is_none() && b.seat == *entry.seat);
    if !open {
        return Err(api_error(
            ErrorCode::CallerUnverified,
            "catch-up entry requires an open binding of the seat",
        ));
    }
    let active: Option<i64> = tx
        .query_row(
            "SELECT frontier_seq FROM catch_up WHERE binding_id=?1 AND thread_id=?2 AND state='active'",
            params![entry.binding.as_str(), entry.thread.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    if let Some(frontier) = active {
        return u64::try_from(frontier)
            .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative catch-up frontier"));
    }
    // The entry extension is withheld only on re-entry after a stall with no
    // progress since: the most recent row for this (binding, thread) ended
    // `stalled` and no summary block has been stored for the thread since it
    // ended. A first entry, or one after a ready or superseded row, earns one
    // fresh p99 (spec §8, ht-hqg). Read before the upsert below overwrites
    // the ended row.
    let stalled_without_progress: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM catch_up c WHERE c.binding_id=?1 AND c.thread_id=?2 AND c.state='ended' AND c.end_reason='stalled' \
             AND NOT EXISTS (SELECT 1 FROM summary_blocks b WHERE b.thread_id=c.thread_id AND b.created_at>=c.ended_at))",
            params![entry.binding.as_str(), entry.thread.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    let frontier = i64::try_from(entry.frontier_seq)
        .map_err(|_| api_error(ErrorCode::InvalidRequest, "catch-up frontier out of range"))?;
    tx.execute(
        "INSERT INTO catch_up(binding_id,seat_id,thread_id,frontier_seq,entered_at,last_progress_at,state,end_reason,ended_at) VALUES (?1,?2,?3,?4,?5,NULL,'active',NULL,NULL) \
         ON CONFLICT(binding_id,thread_id) DO UPDATE SET frontier_seq=excluded.frontier_seq, entered_at=excluded.entered_at, last_progress_at=NULL, state='active', end_reason=NULL, ended_at=NULL",
        params![
            entry.binding.as_str(),
            entry.seat.as_str(),
            entry.thread.as_str(),
            frontier,
            entry.now.0
        ],
    )
    .map_err(store_error)?;
    if !stalled_without_progress {
        let instance = seat_instance(tx, entry.seat)?;
        let p99 = summary::p99_job_duration(tx, &instance, settings)?;
        receipts::extension_on_entry(tx, entry.binding, entry.thread, entry.now, p99)?;
    }
    Ok(entry.frontier_seq)
}

/// Ends the binding's active row on `thread` with `end_reason = ready` when a
/// Ready answer goes to that same binding; returns whether a row ended. The
/// exit-grace extension is written by the contract hook.
pub fn on_ready(
    tx: &Transaction<'_>,
    binding: &BindingId,
    thread: &ThreadId,
    now: UtcMillis,
    settings: &SummarySettings,
) -> Result<bool, ApiError> {
    if end_row(tx, binding, thread, CatchUpEndReason::Ready, now)?.is_none() {
        return Ok(false);
    }
    receipts::extension_on_exit(tx, binding, thread, now, settings.exit_grace_ms)?;
    Ok(true)
}

/// Called once per newly stored block on `thread` (never for an idempotent
/// re-submit): marks progress on every active row of the thread and calls the
/// contract's progress hook once.
pub fn on_progress(
    tx: &Transaction<'_>,
    thread: &ThreadId,
    now: UtcMillis,
    settings: &SummarySettings,
) -> Result<(), ApiError> {
    tx.execute(
        "UPDATE catch_up SET last_progress_at=?1 WHERE thread_id=?2 AND state='active'",
        params![now.0, thread.as_str()],
    )
    .map_err(store_error)?;
    let instance = thread_instance(tx, thread)?;
    let p99 = summary::p99_job_duration(tx, &instance, settings)?;
    receipts::extension_on_progress(tx, thread, now, p99)
}

/// Ends an active row and releases what it held: allocates `release_seq`,
/// re-derives the seat's wake reasons and bumps its inbox filter revision.
/// A row that is not active is left alone (returns `None`); otherwise the
/// row's seat.
fn end_row(
    tx: &Transaction<'_>,
    binding: &BindingId,
    thread: &ThreadId,
    reason: CatchUpEndReason,
    now: UtcMillis,
) -> Result<Option<SeatId>, ApiError> {
    let seat: Option<String> = tx
        .query_row(
            "SELECT seat_id FROM catch_up WHERE binding_id=?1 AND thread_id=?2 AND state='active'",
            params![binding.as_str(), thread.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    let Some(seat) = seat.map(SeatId::new) else {
        return Ok(None);
    };
    let instance = seat_instance(tx, &seat)?;
    let release = schema::next_decision_seq(tx, &instance)?;
    let release = i64::try_from(release)
        .map_err(|_| api_error(ErrorCode::SequenceExhausted, "decision sequence exhausted"))?;
    tx.execute(
        "UPDATE catch_up SET state='ended', end_reason=?1, ended_at=?2, release_seq=?3 WHERE binding_id=?4 AND thread_id=?5 AND state='active'",
        params![reason.as_str(), now.0, release, binding.as_str(), thread.as_str()],
    )
    .map_err(store_error)?;
    wake::note_new_attention(tx, &seat)?;
    schema::bump_filter_revision(tx, &instance, "inbox", seat.as_str())?;
    Ok(Some(seat))
}

fn supersede_threads(
    tx: &Transaction<'_>,
    sql: &str,
    key: &str,
    now: UtcMillis,
) -> Result<u32, ApiError> {
    let rows: Vec<(String, String)> = {
        let mut stmt = tx.prepare(sql).map_err(store_error)?;
        stmt.query_map([key], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(store_error)?
            .collect::<Result<_, _>>()
            .map_err(store_error)?
    };
    let mut ended = 0;
    for (binding, thread) in rows {
        let binding = BindingId::parse(binding)
            .map_err(|_| api_error(ErrorCode::StoreCorrupt, "invalid catch-up binding"))?;
        if end_row(
            tx,
            &binding,
            &ThreadId::new(&thread),
            CatchUpEndReason::Superseded,
            now,
        )?
        .is_some()
        {
            ended += 1;
        }
    }
    Ok(ended)
}

/// Ends (`superseded`) and releases every active row of `binding` when the
/// binding itself has ended (or is gone). Another binding's rows, and a
/// binding that is still open, are never touched: a check-in of one binding
/// leaves the others' catch-up alone. Never touches bindings or seats.
pub fn supersede_stale(
    tx: &Transaction<'_>,
    binding: &BindingId,
    now: UtcMillis,
) -> Result<u32, ApiError> {
    supersede_threads(
        tx,
        "SELECT c.binding_id, c.thread_id FROM catch_up c WHERE c.binding_id=?1 AND c.state='active' AND NOT EXISTS (SELECT 1 FROM occupant_bindings b WHERE b.binding_id=c.binding_id AND b.ended_at IS NULL) ORDER BY c.thread_id",
        binding.as_str(),
        now,
    )
}

/// `supersede_stale` for every binding of `seat` that has ended: the call for
/// a transaction that ends or replaces bindings and does not hold one binding
/// to name (unresolve, rebind, retire, a lifecycle check-in that replaces
/// placeholder bindings). Open bindings' rows survive.
pub fn supersede_ended_bindings(
    tx: &Transaction<'_>,
    seat: &SeatId,
    now: UtcMillis,
) -> Result<u32, ApiError> {
    supersede_threads(
        tx,
        "SELECT c.binding_id, c.thread_id FROM catch_up c WHERE c.seat_id=?1 AND c.state='active' AND NOT EXISTS (SELECT 1 FROM occupant_bindings b WHERE b.binding_id=c.binding_id AND b.ended_at IS NULL) ORDER BY c.binding_id, c.thread_id",
        seat.as_str(),
        now,
    )
}

/// Ends (`stalled`) and releases up to `limit` active rows whose
/// `extension_until` has passed, oldest first. Returns the rows ended.
pub fn stall_scan(tx: &Transaction<'_>, now: UtcMillis, limit: u16) -> Result<u16, ApiError> {
    let due: Vec<(String, String)> = {
        let mut stmt = tx
            .prepare(
                "SELECT binding_id, thread_id FROM catch_up INDEXED BY catch_up_extension_until WHERE extension_until IS NOT NULL AND extension_until<=?1 AND state='active' ORDER BY extension_until, seat_id, thread_id, binding_id LIMIT ?2",
            )
            .map_err(store_error)?;
        stmt.query_map(params![now.0, i64::from(limit)], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .map_err(store_error)?
        .collect::<Result<_, _>>()
        .map_err(store_error)?
    };
    let mut ended = 0;
    for (binding, thread) in due {
        let binding = BindingId::parse(binding)
            .map_err(|_| api_error(ErrorCode::StoreCorrupt, "invalid catch-up binding"))?;
        if end_row(
            tx,
            &binding,
            &ThreadId::new(&thread),
            CatchUpEndReason::Stalled,
            now,
        )?
        .is_some()
        {
            ended += 1;
        }
    }
    Ok(ended)
}

/// F above which ordinary messages on `thread` are held for `seat` while an
/// active row exists.
pub fn held_above(
    conn: &Connection,
    seat: &SeatId,
    thread: &ThreadId,
) -> Result<Option<u64>, ApiError> {
    Ok(hold_view(conn, seat.as_str(), thread.as_str())?.active_frontier)
}

/// What one (seat, thread) pair's catch-up row says about pushed attention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HoldView {
    /// Frontier of the active row, when one exists.
    pub active_frontier: Option<u64>,
    /// `(frontier, release_seq)` of the ended row, when the last row ended.
    pub released: Option<(u64, i64)>,
}

impl HoldView {
    /// Whether a pending receipt at message `sequence` is held back from
    /// pushed attention: active row, above the frontier, not priority.
    pub fn held(&self, sequence: u64, priority: bool) -> bool {
        self.active_frontier.is_some_and(|f| sequence > f) && !priority
    }

    /// The attention key of a receipt that is not held: a receipt above an
    /// ended row's frontier that was published before the release takes
    /// `(release_seq, 0)`, a fresh key above the seat's mark.
    pub fn attention_key(&self, sequence: u64, original: (i64, i64)) -> (i64, i64) {
        match self.released {
            Some((frontier, release)) if sequence > frontier && release > original.0 => {
                (release, 0)
            }
            _ => original,
        }
    }

    /// Whether the message could be affected at all (priority unknown).
    fn may_apply(&self, sequence: u64, original: (i64, i64)) -> bool {
        self.active_frontier.is_some_and(|f| sequence > f)
            || self
                .released
                .is_some_and(|(f, r)| sequence > f && r > original.0)
    }
}

/// Seat-level view over the seat's per-binding rows for the thread: any
/// active row holds (the lowest frontier wins); otherwise the most recently
/// released row decides the attention key.
pub fn hold_view(conn: &Connection, seat: &str, thread: &str) -> Result<HoldView, ApiError> {
    let row: Option<(String, i64, Option<i64>)> = conn
        .query_row(
            "SELECT state, frontier_seq, release_seq FROM catch_up WHERE seat_id=?1 AND thread_id=?2 \
             ORDER BY (state='active') DESC, CASE WHEN state='active' THEN frontier_seq END ASC, release_seq DESC LIMIT 1",
            params![seat, thread],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(store_error)?;
    let Some((state, frontier, release)) = row else {
        return Ok(HoldView::default());
    };
    let frontier = u64::try_from(frontier)
        .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative catch-up frontier"))?;
    Ok(if state == "active" {
        HoldView {
            active_frontier: Some(frontier),
            released: None,
        }
    } else {
        HoldView {
            active_frontier: None,
            released: release.map(|r| (frontier, r)),
        }
    })
}

/// Per-call cache of hold views for one seat, shared by the pushed-attention
/// producers (digest, inbox, wake scan) so they apply the identical rule.
pub struct HoldCache<'a> {
    seat: &'a str,
    views: HashMap<String, HoldView>,
}

impl<'a> HoldCache<'a> {
    pub fn new(seat: &'a str) -> Self {
        Self {
            seat,
            views: HashMap::new(),
        }
    }

    /// The attention key for a pending receipt, or `None` when it is held.
    /// Priority comes from the message row through the contract's
    /// `is_priority`; a priority message keeps its own key.
    pub fn attention_key(
        &mut self,
        db: &Connection,
        thread: &str,
        message_id: &str,
        sequence: i64,
        original: (i64, i64),
    ) -> Result<Option<(i64, i64)>, ApiError> {
        let view = match self.views.get(thread) {
            Some(view) => *view,
            None => {
                let view = hold_view(db, self.seat, thread)?;
                self.views.insert(thread.to_owned(), view);
                view
            }
        };
        let sequence = u64::try_from(sequence)
            .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative message sequence"))?;
        if !view.may_apply(sequence, original) {
            return Ok(Some(original));
        }
        let (role, relays): (Option<String>, i64) = db
            .query_row(
                "SELECT author_role, relays_user FROM messages WHERE id=?1",
                [message_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(store_error)?;
        let priority = is_priority(
            role.as_deref().and_then(AuthorRole::from_column),
            relays != 0,
        );
        if priority {
            return Ok(Some(original));
        }
        if view.held(sequence, false) {
            return Ok(None);
        }
        Ok(Some(view.attention_key(sequence, original)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        protocol::time::{Clock, MonoInstant},
        store::{
            connection::StoreContext,
            effective::{EffectiveReceipt, EffectiveReceiptState, ReceiptSource},
            receipts, summary,
        },
    };
    use std::sync::Arc;

    struct FixedClock;
    impl Clock for FixedClock {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(100)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(100)
        }
    }

    fn seeded() -> (Connection, SeatId, ThreadId) {
        let path = std::env::temp_dir().join(format!("catch-up-{}.db", uuid::Uuid::new_v4()));
        let db = StoreContext::new(path, Arc::new(FixedClock))
            .open_writer()
            .unwrap();
        db.execute(
            "INSERT INTO host_instances(id, created_at) VALUES ('i', 0)",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO seats(id, instance_id, state, role, generation, created_at) VALUES ('s', 'i', 'resolved', 'native', 1, 0)", []).unwrap();
        db.execute("INSERT INTO threads(id, instance_id, topic, goal, created_at, updated_at) VALUES ('t', 'i', 'topic', 'purpose', 0, 0)", []).unwrap();
        (db, SeatId::new("s"), ThreadId::new("t"))
    }

    fn catch_up_rows(db: &Connection) -> i64 {
        db.query_row("SELECT count(*) FROM catch_up", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn end_reasons_have_stable_columns() {
        assert_eq!(CatchUpEndReason::Ready.as_str(), "ready");
        assert_eq!(CatchUpEndReason::Stalled.as_str(), "stalled");
        assert_eq!(CatchUpEndReason::Superseded.as_str(), "superseded");
    }

    #[test]
    fn p99_job_duration_is_cold() {
        let (db, _, _) = seeded();
        let mut settings = SummarySettings::default();
        assert_eq!(
            summary::p99_job_duration(&db, "i", &settings).unwrap(),
            90_000
        );
        settings.p99_cold_ms = 123_000;
        assert_eq!(
            summary::p99_job_duration(&db, "i", &settings).unwrap(),
            123_000
        );
    }

    #[test]
    fn effective_deadline_is_the_frozen_deadline() {
        let (mut db, _seat, thread) = seeded();
        let receipt = |deadline_at| EffectiveReceipt {
            source: ReceiptSource::Physical,
            message_id: "m".into(),
            thread_id: "t".into(),
            seat_id: "s".into(),
            sequence: 1,
            source_ordinal: 1,
            decision_seq: Some(1),
            decision_at: 0,
            frozen_duration_ms: 1_000,
            state: EffectiveReceiptState::Pending,
            available_at: Some(0),
            deadline_at,
            warning_message_id: None,
            ack_actor_seat_id: None,
            ack_generation: None,
            ack_observation: None,
            acked_at: None,
            retired_at: None,
        };
        assert_eq!(
            receipts::effective_deadline(&db, &receipt(Some(1_000))).unwrap(),
            Some(1_000)
        );
        assert_eq!(
            receipts::effective_deadline(&db, &receipt(None)).unwrap(),
            None
        );
        let tx = db.transaction().unwrap();
        let binding = BindingId::new("b");
        receipts::extension_on_entry(&tx, &binding, &thread, UtcMillis(1), 90_000).unwrap();
        receipts::extension_on_progress(&tx, &thread, UtcMillis(2), 90_000).unwrap();
        receipts::extension_on_exit(&tx, &binding, &thread, UtcMillis(3), 60_000).unwrap();
        tx.commit().unwrap();
        assert_eq!(catch_up_rows(&db), 0);
    }
}

#[cfg(test)]
#[path = "../../tests/store/catch_up.rs"]
mod lifecycle_tests;
