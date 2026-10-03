//! Catch-up rows (spec §7): one per (seat, thread). Contract surface from
//! ht-1ip.1; the lifecycle and the pushed-attention hold are ht-1ip.6. Every
//! `extension_until` write is ht-1ip.7, made through the hooks in `receipts`:
//! nothing here computes or writes an extension value.
//!
//! Release is a push: every row end (ready, stalled, superseded) allocates a
//! decision sequence stored as `release_seq`, re-publishes the held range
//! (receipts above the frontier take `(release_seq, 0)` as their attention
//! key) and re-derives the seat's wake reasons in the same transaction.
use super::{
    connection::{api_error, store_error},
    receipts, schema, summary, wake,
};
use crate::protocol::{
    ids::{ExecutionId, SeatId, ThreadId},
    results::{ApiError, ErrorCode},
    summary::{AuthorRole, SummarySettings, is_priority},
    time::UtcMillis,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use std::collections::HashMap;

pub struct CatchUpEntry<'a> {
    pub seat: &'a SeatId,
    pub thread: &'a ThreadId,
    /// F computed by the summary handler (published head at decision time).
    pub frontier_seq: u64,
    pub binding_generation: u64,
    pub execution: &'a ExecutionId,
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

/// The seat's open binding as `(generation, execution_id)`.
fn open_binding(conn: &Connection, seat: &SeatId) -> Result<Option<(i64, String)>, ApiError> {
    conn.query_row(
        "SELECT generation, execution_id FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL",
        [seat.as_str()],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
    .map_err(store_error)
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

/// Opens the (seat, thread) row or keeps an active one, in the deciding
/// transaction of a `Summary` that returns Work. Returns the frontier in
/// force: an active row's F, unchanged, else `entry.frontier_seq`. The entry
/// claim must equal the seat's open binding (generation and execution); there
/// is no child-role gate (spec §7, r3). The entry extension hook runs unless
/// this is a re-entry after a stall with no block stored since (spec §8,
/// ht-hqg).
pub fn enter_or_keep(
    tx: &Transaction<'_>,
    entry: &CatchUpEntry<'_>,
    settings: &SummarySettings,
) -> Result<u64, ApiError> {
    let current = open_binding(tx, entry.seat)?;
    if current.as_ref().is_none_or(|(generation, execution)| {
        u64::try_from(*generation).ok() != Some(entry.binding_generation)
            || execution != entry.execution.as_str()
    }) {
        return Err(api_error(
            ErrorCode::CallerUnverified,
            "catch-up entry requires the seat's current binding",
        ));
    }
    let active: Option<(i64, i64, String)> = tx
        .query_row(
            "SELECT frontier_seq, binding_generation, execution_id FROM catch_up WHERE seat_id=?1 AND thread_id=?2 AND state='active'",
            params![entry.seat.as_str(), entry.thread.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(store_error)?;
    if let Some((frontier, generation, execution)) = active {
        if u64::try_from(generation).ok() == Some(entry.binding_generation)
            && execution == entry.execution.as_str()
        {
            return u64::try_from(frontier)
                .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative catch-up frontier"));
        }
        end_row(
            tx,
            entry.seat,
            entry.thread,
            CatchUpEndReason::Superseded,
            entry.now,
        )?;
    }
    // The entry extension is withheld only on re-entry after a stall with no
    // progress since: the most recent row for this (seat, thread) ended
    // `stalled` and no summary block has been stored for the thread since it
    // ended. A first entry, or one after a ready or superseded row, earns one
    // fresh p99 (spec §8, ht-hqg). Read before the upsert below overwrites
    // the ended row; a superseded active row (just ended above) is eligible.
    let stalled_without_progress: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM catch_up c WHERE c.seat_id=?1 AND c.thread_id=?2 AND c.state='ended' AND c.end_reason='stalled' \
             AND NOT EXISTS (SELECT 1 FROM summary_blocks b WHERE b.thread_id=c.thread_id AND b.created_at>=c.ended_at))",
            params![entry.seat.as_str(), entry.thread.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    let frontier = i64::try_from(entry.frontier_seq)
        .map_err(|_| api_error(ErrorCode::InvalidRequest, "catch-up frontier out of range"))?;
    let generation = i64::try_from(entry.binding_generation)
        .map_err(|_| api_error(ErrorCode::InvalidRequest, "binding generation out of range"))?;
    tx.execute(
        "INSERT INTO catch_up(seat_id,thread_id,frontier_seq,binding_generation,execution_id,entered_at,last_progress_at,state,end_reason,ended_at) VALUES (?1,?2,?3,?4,?5,?6,NULL,'active',NULL,NULL) \
         ON CONFLICT(seat_id,thread_id) DO UPDATE SET frontier_seq=excluded.frontier_seq, binding_generation=excluded.binding_generation, execution_id=excluded.execution_id, entered_at=excluded.entered_at, last_progress_at=NULL, state='active', end_reason=NULL, ended_at=NULL",
        params![
            entry.seat.as_str(),
            entry.thread.as_str(),
            frontier,
            generation,
            entry.execution.as_str(),
            entry.now.0
        ],
    )
    .map_err(store_error)?;
    if !stalled_without_progress {
        let instance = seat_instance(tx, entry.seat)?;
        let p99 = summary::p99_job_duration(tx, &instance, settings)?;
        receipts::extension_on_entry(tx, entry.seat, entry.thread, entry.now, p99)?;
    }
    Ok(entry.frontier_seq)
}

/// Ends the seat's active row on `thread` with `end_reason = ready` when a
/// Ready answer goes to the same top-level binding; returns whether a row
/// ended. The exit-grace extension is written by the contract hook.
pub fn on_ready(
    tx: &Transaction<'_>,
    seat: &SeatId,
    thread: &ThreadId,
    binding_generation: u64,
    now: UtcMillis,
    settings: &SummarySettings,
) -> Result<bool, ApiError> {
    let active: Option<i64> = tx
        .query_row(
            "SELECT binding_generation FROM catch_up WHERE seat_id=?1 AND thread_id=?2 AND state='active'",
            params![seat.as_str(), thread.as_str()],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    if active.and_then(|g| u64::try_from(g).ok()) != Some(binding_generation) {
        return Ok(false);
    }
    end_row(tx, seat, thread, CatchUpEndReason::Ready, now)?;
    receipts::extension_on_exit(tx, seat, thread, now, settings.exit_grace_ms)?;
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
/// A row that is not active is left alone (returns false).
fn end_row(
    tx: &Transaction<'_>,
    seat: &SeatId,
    thread: &ThreadId,
    reason: CatchUpEndReason,
    now: UtcMillis,
) -> Result<bool, ApiError> {
    let active: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM catch_up WHERE seat_id=?1 AND thread_id=?2 AND state='active')",
            params![seat.as_str(), thread.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if !active {
        return Ok(false);
    }
    let instance = seat_instance(tx, seat)?;
    let release = schema::next_decision_seq(tx, &instance)?;
    let release = i64::try_from(release)
        .map_err(|_| api_error(ErrorCode::SequenceExhausted, "decision sequence exhausted"))?;
    tx.execute(
        "UPDATE catch_up SET state='ended', end_reason=?1, ended_at=?2, release_seq=?3 WHERE seat_id=?4 AND thread_id=?5 AND state='active'",
        params![reason.as_str(), now.0, release, seat.as_str(), thread.as_str()],
    )
    .map_err(store_error)?;
    wake::note_new_attention(tx, seat)?;
    schema::bump_filter_revision(tx, &instance, "inbox", seat.as_str())?;
    Ok(true)
}

/// Ends (`superseded`) and releases every active row of `seat` whose binding
/// is no longer the seat's open binding (all of them when none is open).
/// Called in the transaction that ends or replaces a binding; never touches
/// bindings or seats.
pub fn supersede_stale(
    tx: &Transaction<'_>,
    seat: &SeatId,
    now: UtcMillis,
) -> Result<u32, ApiError> {
    let threads: Vec<String> = {
        let mut stmt = tx
            .prepare(
                "SELECT c.thread_id FROM catch_up c WHERE c.seat_id=?1 AND c.state='active' AND NOT EXISTS (SELECT 1 FROM occupant_bindings b WHERE b.seat_id=c.seat_id AND b.ended_at IS NULL AND b.generation=c.binding_generation AND b.execution_id=c.execution_id) ORDER BY c.thread_id",
            )
            .map_err(store_error)?;
        stmt.query_map([seat.as_str()], |r| r.get(0))
            .map_err(store_error)?
            .collect::<Result<_, _>>()
            .map_err(store_error)?
    };
    let mut ended = 0;
    for thread in threads {
        if end_row(
            tx,
            seat,
            &ThreadId::new(&thread),
            CatchUpEndReason::Superseded,
            now,
        )? {
            ended += 1;
        }
    }
    Ok(ended)
}

/// Ends (`stalled`) and releases up to `limit` active rows whose
/// `extension_until` has passed, oldest first. Returns the rows ended.
pub fn stall_scan(tx: &Transaction<'_>, now: UtcMillis, limit: u16) -> Result<u16, ApiError> {
    let due: Vec<(String, String)> = {
        let mut stmt = tx
            .prepare(
                "SELECT seat_id, thread_id FROM catch_up INDEXED BY catch_up_extension_until WHERE extension_until IS NOT NULL AND extension_until<=?1 AND state='active' ORDER BY extension_until, seat_id, thread_id LIMIT ?2",
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
    for (seat, thread) in due {
        if end_row(
            tx,
            &SeatId::new(&seat),
            &ThreadId::new(&thread),
            CatchUpEndReason::Stalled,
            now,
        )? {
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

pub fn hold_view(conn: &Connection, seat: &str, thread: &str) -> Result<HoldView, ApiError> {
    let row: Option<(String, i64, Option<i64>)> = conn
        .query_row(
            "SELECT state, frontier_seq, release_seq FROM catch_up WHERE seat_id=?1 AND thread_id=?2",
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
        let (mut db, seat, thread) = seeded();
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
        receipts::extension_on_entry(&tx, &seat, &thread, UtcMillis(1), 90_000).unwrap();
        receipts::extension_on_progress(&tx, &thread, UtcMillis(2), 90_000).unwrap();
        receipts::extension_on_exit(&tx, &seat, &thread, UtcMillis(3), 60_000).unwrap();
        tx.commit().unwrap();
        assert_eq!(catch_up_rows(&db), 0);
    }
}

#[cfg(test)]
#[path = "../../tests/store/catch_up.rs"]
mod lifecycle_tests;
