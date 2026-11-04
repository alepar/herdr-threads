//! The sender node's ledger of signed handoff requests (`handoff_obligations`, spec §2.3, Q4).
//!
//! Responsibility
//! - Owns: the one row per request id, recorded before the first send with the signed
//!   `LaunchRequest` frozen in `request_bytes` (every send replays the winning row's bytes),
//!   the guarded monotonic transitions (`sent -> launched`, `sent|launched -> linked`,
//!   `sent|launched -> expired`, `sent -> failed`; `linked`, `expired` and `failed` are
//!   terminal), the overdue columns (`overdue_seq`, `thread_warned_at`, `alert_shown_at`:
//!   overdue is never a state), expiry, the re-send and due queries, the due step and pass
//!   (the overdue columns and the one in-thread `overdue:handoff:` warning), the `peer list`
//!   ledger rows, the seat-addressed alert predicate (Q4) and pruning.
//! - Hides: every `handoff_obligations` SQL statement. A guard miss is `Ok(false)`, never an
//!   error.
//! - Must not: sign, send frames, render text or write warnings itself (the due step goes
//!   through `schema::record_overdue_if_pending`), or replicate anything: the table is
//!   node-local.
//!
//! Core and harness-neutral.

use rusqlite::{Connection, OptionalExtension, params};

use crate::ports::TimeBasis;
use crate::protocol::authority::ObligationRef;
use crate::protocol::ids::{SeatId, direct_thread_id};
use crate::protocol::results::ApiError;
use crate::protocol::sync_admin::{HandoffLedgerRow, RemoteLaunchAnswer};
use crate::protocol::time::UtcMillis;
use crate::store::connection::store_error;
use crate::store::delivery_tail::DeliveryTail;
use crate::store::{peer_requests, schema};

/// Reason bit for new attention, the one `DeliveryTail` gives a new message or invitation.
const REASON_NEW_ATTENTION: i64 = 1;

/// How long a request stays in the re-send set before it expires (ms).
pub const OBLIGATION_TTL_MS: i64 = 6 * 24 * 3_600_000;
const _: () = assert!(
    OBLIGATION_TTL_MS < peer_requests::RETENTION_MS,
    "OBLIGATION_TTL_MS must stay below peer_requests::RETENTION_MS"
);

/// Terminal rows are deleted this long after `updated_at` (ms).
pub const PRUNE_AFTER_MS: i64 = 7 * 24 * 3_600_000;

/// The alert predicate (Q4): overdue, the in-thread warning not written yet, not yet shown, and
/// not linked (after linking the receipt's own overdue takes over). It reads `state` only for
/// that last term: the pair thread cannot be tested in SQL (its id is a hash), and
/// `thread_warned_at` is exactly "the in-thread warning was written". Written as the partial
/// index `handoff_obligations_alert` spells it so the planner can use that index.
pub const ALERT_PENDING_SQL: &str = "o.overdue_seq IS NOT NULL AND o.thread_warned_at IS NULL AND o.alert_shown_at IS NULL \
     AND o.state!='linked' \
     AND EXISTS(SELECT 1 FROM seats s WHERE s.id=o.author AND s.state!='retired')";

/// A ledger row's state; the closed set of the table's CHECK.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Sent,
    Launched,
    Linked,
    Expired,
    Failed,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Sent => "sent",
            State::Launched => "launched",
            State::Linked => "linked",
            State::Expired => "expired",
            State::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Option<State> {
        Some(match s {
            "sent" => State::Sent,
            "launched" => State::Launched,
            "linked" => State::Linked,
            "expired" => State::Expired,
            "failed" => State::Failed,
            _ => return None,
        })
    }
}

/// One ledger row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Obligation {
    pub request: String,
    pub target_node: String,
    pub author: String,
    pub message: String,
    pub requested_at: i64,
    pub deadline_at: Option<i64>,
    pub request_bytes: Vec<u8>,
    pub seat: Option<String>,
    pub state: State,
    pub answer: Option<String>,
    pub overdue_seq: Option<i64>,
    pub thread_warned_at: Option<i64>,
    pub alert_shown_at: Option<i64>,
    pub created_at: i64,
    pub expires_at: i64,
    pub updated_at: i64,
}

impl Obligation {
    /// Whether the seat-addressed alert is pending on this row (the predicate's row part; the
    /// author's retirement is read by the queries).
    pub(crate) fn alert_pending(&self) -> bool {
        self.overdue_seq.is_some()
            && self.thread_warned_at.is_none()
            && self.alert_shown_at.is_none()
            && self.state != State::Linked
    }
}

/// What a new request records.
pub struct NewObligation<'a> {
    pub request: &'a str,
    pub target_node: &'a str,
    pub author: &'a str,
    pub message: &'a str,
    pub requested_at: i64,
    pub deadline_at: Option<i64>,
    pub request_bytes: &'a [u8],
}

const COLUMNS: &str = "o.request,o.target_node,o.author,o.message,o.requested_at,o.deadline_at,o.request_bytes,\
     o.seat,o.state,o.answer,o.overdue_seq,o.thread_warned_at,o.alert_shown_at,o.created_at,o.expires_at,o.updated_at";

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Obligation> {
    let state: String = r.get(8)?;
    Ok(Obligation {
        request: r.get(0)?,
        target_node: r.get(1)?,
        author: r.get(2)?,
        message: r.get(3)?,
        requested_at: r.get(4)?,
        deadline_at: r.get(5)?,
        request_bytes: r.get(6)?,
        seat: r.get(7)?,
        state: State::parse(&state).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                8,
                rusqlite::types::Type::Text,
                format!("unknown handoff obligation state {state:?}").into(),
            )
        })?,
        answer: r.get(9)?,
        overdue_seq: r.get(10)?,
        thread_warned_at: r.get(11)?,
        alert_shown_at: r.get(12)?,
        created_at: r.get(13)?,
        expires_at: r.get(14)?,
        updated_at: r.get(15)?,
    })
}

fn query(
    conn: &Connection,
    tail: &str,
    args: impl rusqlite::Params,
) -> rusqlite::Result<Vec<Obligation>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM handoff_obligations o {tail}"
    ))?;
    stmt.query_map(args, row)?.collect()
}

pub fn get(conn: &Connection, request: &str) -> rusqlite::Result<Option<Obligation>> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM handoff_obligations o WHERE o.request=?1"),
        [request],
        row,
    )
    .optional()
}

pub fn by_message(conn: &Connection, message: &str) -> rusqlite::Result<Option<Obligation>> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM handoff_obligations o WHERE o.message=?1"),
        [message],
        row,
    )
    .optional()
}

/// Records a new request as `sent`, or returns the row already recorded for its id; the
/// returned row's `request_bytes` are what every send uses. The flag says whether this call
/// inserted it.
pub fn insert_or_get(
    conn: &Connection,
    new: &NewObligation<'_>,
) -> rusqlite::Result<(Obligation, bool)> {
    let inserted = conn.execute(
        "INSERT INTO handoff_obligations(request,target_node,author,message,requested_at,deadline_at,\
         request_bytes,state,created_at,expires_at,updated_at) \
         VALUES (?1,?2,?3,?4,?5,?6,?7,'sent',?5,?5+?8,?5) ON CONFLICT(request) DO NOTHING",
        params![
            new.request,
            new.target_node,
            new.author,
            new.message,
            new.requested_at,
            new.deadline_at,
            new.request_bytes,
            OBLIGATION_TTL_MS
        ],
    )?;
    let stored = get(conn, new.request)?.ok_or(rusqlite::Error::QueryReturnedNoRows)?;
    Ok((stored, inserted == 1))
}

fn bump(conn: &Connection, author: &str) -> Result<(), ApiError> {
    DeliveryTail::attention_unless_retired(conn, author, REASON_NEW_ATTENTION)
}

/// Wakes the author once when the row's alert predicate differs from `before` (an entry or an
/// exit); a write that left it unchanged bumps nothing. A deleted row counts as not pending.
fn bump_if_alert_changed(
    conn: &Connection,
    before: &Obligation,
    request: &str,
) -> Result<(), ApiError> {
    let after = get(conn, request).map_err(store_error)?;
    if before.alert_pending() != after.is_some_and(|row| row.alert_pending()) {
        bump(conn, &before.author)?;
    }
    Ok(())
}

/// The target launched the request: `sent -> launched`, recording the seat. The alert predicate
/// does not read the seat: a pending alert stays until the in-thread warning is written.
pub fn launched(conn: &Connection, request: &str, seat: &str, now: i64) -> Result<bool, ApiError> {
    let before = get(conn, request).map_err(store_error)?;
    let changed = conn
        .execute(
            "UPDATE handoff_obligations SET state='launched', seat=?2, updated_at=?3 \
             WHERE request=?1 AND state='sent'",
            params![request, seat, now],
        )
        .map_err(store_error)?;
    if changed == 1
        && let Some(before) = before
    {
        bump_if_alert_changed(conn, &before, request)?;
    }
    Ok(changed == 1)
}

/// The message replicated: `sent|launched -> linked` by message id, recording the recipient.
/// The materializer calls it inside the `message.sent` transaction.
pub fn link(conn: &Connection, message: &str, recipient: &str, now: i64) -> Result<bool, ApiError> {
    let before = by_message(conn, message).map_err(store_error)?;
    let changed = conn
        .execute(
            "UPDATE handoff_obligations SET state='linked', seat=?2, updated_at=?3 \
             WHERE message=?1 AND state IN ('sent','launched')",
            params![message, recipient, now],
        )
        .map_err(store_error)?;
    if changed == 1
        && let Some(before) = before
    {
        bump_if_alert_changed(conn, &before, &before.request)?;
    }
    Ok(changed == 1)
}

/// The target refused or failed without staging: `sent -> failed`, storing the answer.
/// `shown` says a polling CLI received it, which marks the alert shown. Does not clear a
/// pending alert (the predicate reads no state), so it bumps nothing.
pub fn fail(
    conn: &Connection,
    request: &str,
    answer: &str,
    shown: bool,
    now: i64,
) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "UPDATE handoff_obligations SET state='failed', answer=?2, \
         alert_shown_at=CASE WHEN ?4 THEN ?3 ELSE alert_shown_at END, updated_at=?3 \
         WHERE request=?1 AND state='sent'",
        params![request, answer, now, shown],
    )?;
    Ok(changed == 1)
}

/// Records that the request is overdue: allocates `overdue_seq` once (any state). When that
/// makes the alert pending it wakes its author, in the same transaction.
pub fn mark_overdue(tx: &Connection, request: &str, _now: i64) -> Result<bool, ApiError> {
    let Some(before) = get(tx, request).map_err(store_error)? else {
        return Ok(false);
    };
    let changed = allocate_overdue(tx, &before)?;
    if changed {
        bump_if_alert_changed(tx, &before, request)?;
    }
    Ok(changed)
}

/// Allocates `overdue_seq` once, without waking anyone.
fn allocate_overdue(tx: &Connection, current: &Obligation) -> Result<bool, ApiError> {
    let request = current.request.as_str();
    if current.overdue_seq.is_some() {
        return Ok(false);
    }
    let node: String = tx
        .query_row("SELECT id FROM local_node", [], |r| r.get(0))
        .map_err(store_error)?;
    let seq = schema::next_decision_seq(tx, &node)?;
    let changed = tx
        .execute(
            "UPDATE handoff_obligations SET overdue_seq=?2 WHERE request=?1 AND overdue_seq IS NULL",
            params![request, seq as i64],
        )
        .map_err(store_error)?;
    Ok(changed == 1)
}

/// Records that the in-thread overdue warning was written, once.
pub fn mark_thread_warned(conn: &Connection, request: &str, now: i64) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "UPDATE handoff_obligations SET thread_warned_at=?2 WHERE request=?1 AND thread_warned_at IS NULL",
        params![request, now],
    )?;
    Ok(changed == 1)
}

/// What one due step changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DueOutcome {
    /// `overdue_seq` was newly set or the in-thread warning was written: the row moved.
    pub advanced: bool,
    /// The in-thread warning was inserted.
    pub warned: bool,
}

/// The row's due step (spec §2.3, "Overdue before linking"), idempotent: once its deadline
/// has passed it allocates `overdue_seq` if unset (an open row with no seat also gets one
/// `log` line) and, with the seat known and the pair thread held, records the one in-thread
/// `overdue:handoff:<request>` warning and sets `thread_warned_at`. A seat known without the
/// pair thread is retried on the next pass, its seat-addressed alert pending meanwhile. The
/// author is woken once if the alert predicate changed. Only the sender node has ledger rows,
/// so the target never reaches here.
pub fn due_step(
    tx: &Connection,
    row: &Obligation,
    now: i64,
    log: &mut dyn FnMut(&str),
) -> Result<DueOutcome, ApiError> {
    let Some(row) = get(tx, &row.request).map_err(store_error)? else {
        return Ok(DueOutcome::default());
    };
    if row.deadline_at.is_none_or(|deadline| deadline > now) {
        return Ok(DueOutcome::default());
    }
    let open = matches!(row.state, State::Sent | State::Launched);
    let unseen_failure = row.state == State::Failed && row.alert_shown_at.is_none();
    let mut outcome = DueOutcome::default();
    if (open || unseen_failure) && allocate_overdue(tx, &row)? {
        outcome.advanced = true;
        if open && row.seat.is_none() {
            log(&format!(
                "handoff {} to {} overdue: not confirmed by that machine",
                request_prefix(&row.request),
                target_label(tx, &row.target_node)?
            ));
        }
    }
    if let Some(seat) = row.seat.as_deref().filter(|_| open)
        && row.thread_warned_at.is_none()
        && pair_thread_held(tx, &row.author, seat)?
    {
        let inserted = schema::record_overdue_if_pending(
            tx,
            &ObligationRef::Handoff(row.request.clone()),
            &TimeBasis::Decision,
            UtcMillis(now),
        )?
        .inserted;
        mark_thread_warned(tx, &row.request, now).map_err(store_error)?;
        outcome.advanced = true;
        outcome.warned = inserted;
    }
    if outcome.advanced {
        bump_if_alert_changed(tx, &row, &row.request)?;
    }
    Ok(outcome)
}

/// Whether the author's node holds the pair thread of `author` and `seat`, and the seat.
fn pair_thread_held(tx: &Connection, author: &str, seat: &str) -> Result<bool, ApiError> {
    let Ok(thread) = direct_thread_id(&SeatId::new(author), &SeatId::new(seat)) else {
        return Ok(false);
    };
    tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM threads WHERE id=?1) AND EXISTS(SELECT 1 FROM seats WHERE id=?2)",
        params![thread.as_str(), seat],
        |r| r.get(0),
    )
    .map_err(store_error)
}

/// The first 8 characters of a request id, as every line names it.
fn request_prefix(request: &str) -> &str {
    request.get(..8).unwrap_or(request)
}

/// The target node's tag, else its uuid prefix (what `peer list` calls the node).
pub(crate) fn target_label(conn: &Connection, node: &str) -> Result<String, ApiError> {
    let tag: Option<String> = conn
        .query_row(
            "SELECT tag FROM node_directory WHERE node_uuid=?1",
            [node],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    Ok(crate::store::sync_status::node_label(tag.as_deref(), node))
}

/// Runs the due step of every due row, one transaction per row, until `limit` rows advanced.
/// Takes the connection of the writer (in autocommit). Pages past rows the step cannot move
/// (a seat known, the pair thread not held yet) so they never starve the others, and logs and
/// skips a row whose step errors. Each line the step logs goes to `daemon.log` (stderr).
/// Returns the number of in-thread warnings it inserted.
pub fn due_pass(conn: &mut Connection, now: i64, limit: usize) -> Result<usize, ApiError> {
    let mut warnings = 0;
    let mut advanced = 0;
    let mut after: Option<(i64, String)> = None;
    while advanced < limit {
        let cursor = after
            .as_ref()
            .map(|(deadline, request)| (*deadline, request.as_str()));
        let page = due_rows(conn, now, cursor, limit).map_err(store_error)?;
        let last_page = page.len() < limit;
        for row in &page {
            after = row
                .deadline_at
                .map(|deadline| (deadline, row.request.clone()));
            let step = conn.transaction().map_err(store_error).and_then(|tx| {
                let outcome = due_step(&tx, row, now, &mut |line| eprintln!("{line}"))?;
                tx.commit().map_err(store_error)?;
                Ok(outcome)
            });
            match step {
                Ok(outcome) => {
                    advanced += usize::from(outcome.advanced);
                    warnings += usize::from(outcome.warned);
                }
                Err(error) => eprintln!(
                    "handoff {} due step failed: {}",
                    request_prefix(&row.request),
                    error.detail
                ),
            }
            if advanced >= limit {
                break;
            }
        }
        if last_page || after.is_none() {
            break;
        }
    }
    Ok(warnings)
}

/// The ledger rows `peer list` shows: unconfirmed overdue (`sent`, overdue, seat unknown),
/// unlinked overdue (`launched`, overdue, not yet warned in the pair thread), `expired` and
/// `failed`, oldest first. `linked` and plain open rows have no line.
pub fn ledger_rows(conn: &Connection) -> Result<Vec<HandoffLedgerRow>, ApiError> {
    let rows = query(
        conn,
        "WHERE (o.state='sent' AND o.overdue_seq IS NOT NULL AND o.seat IS NULL) \
         OR (o.state='launched' AND o.overdue_seq IS NOT NULL AND o.thread_warned_at IS NULL) \
         OR o.state IN ('expired','failed') ORDER BY o.requested_at,o.request",
        [],
    )
    .map_err(store_error)?;
    rows.into_iter()
        .map(|row| {
            Ok(HandoffLedgerRow {
                request: row.request.clone(),
                node: target_label(conn, &row.target_node)?,
                state: row.state.as_str().to_owned(),
                seat: row.seat.clone(),
                overdue: row.overdue_seq.is_some(),
                detail: row.answer.as_deref().map(answer_detail),
            })
        })
        .collect()
}

/// The one-line detail of a stored failed answer (a serialized `RemoteLaunchAnswer`).
fn answer_detail(answer: &str) -> String {
    use crate::sync::frame::LaunchState;
    match serde_json::from_str::<RemoteLaunchAnswer>(answer) {
        Ok(RemoteLaunchAnswer::Answered {
            state: LaunchState::Refused { detail } | LaunchState::Failed { detail, .. },
        }) => detail,
        Ok(RemoteLaunchAnswer::Answered {
            state: LaunchState::Unavailable { reason },
        }) => reason,
        Ok(RemoteLaunchAnswer::NoCap { cap, .. }) => format!(
            "needs {}",
            if cap.is_empty() {
                "peer.launch.v1"
            } else {
                &cap
            }
        ),
        _ => "no detail".to_owned(),
    }
}

/// Expires every `sent|launched` row past its `expires_at`, one savepoint per row. A row whose
/// deadline has passed first runs its due step ([`due_step`]): no deadline-bearing row expires
/// without its overdue. A row whose step errors is rolled back, logged and skipped. Returns
/// the rows it expired, as expired.
pub fn expire_due(tx: &Connection, now: i64) -> Result<Vec<Obligation>, ApiError> {
    let due = query(
        tx,
        "WHERE o.state IN ('sent','launched') AND o.expires_at<=?1 ORDER BY o.expires_at,o.request",
        [now],
    )
    .map_err(store_error)?;
    let mut expired = Vec::new();
    for candidate in due {
        // A named savepoint (not `Connection::savepoint`, which needs `&mut`): the caller holds
        // only the deciding transaction.
        tx.execute_batch("SAVEPOINT handoff_expire")
            .map_err(store_error)?;
        let step = (|| {
            due_step(tx, &candidate, now, &mut |line| eprintln!("{line}"))?;
            let changed = tx
                .execute(
                    "UPDATE handoff_obligations SET state='expired', updated_at=?2 \
                     WHERE request=?1 AND state IN ('sent','launched') AND expires_at<=?2",
                    params![candidate.request, now],
                )
                .map_err(store_error)?;
            if changed == 1 {
                return get(tx, &candidate.request).map_err(store_error);
            }
            Ok(None)
        })();
        match step {
            Ok(row) => {
                tx.execute_batch("RELEASE handoff_expire")
                    .map_err(store_error)?;
                expired.extend(row);
            }
            Err(error) => {
                let _ = tx.execute_batch("ROLLBACK TO handoff_expire; RELEASE handoff_expire");
                eprintln!(
                    "handoff {} expiry failed: {}",
                    request_prefix(&candidate.request),
                    error.detail
                );
            }
        }
    }
    Ok(expired)
}

/// The rows a re-send replays: `sent|launched`, overdue or not, optionally one target node's.
pub fn resend_set(
    conn: &Connection,
    target_node: Option<&str>,
) -> rusqlite::Result<Vec<Obligation>> {
    query(
        conn,
        "WHERE o.state IN ('sent','launched') AND (?1 IS NULL OR o.target_node=?1) \
         ORDER BY o.requested_at,o.request",
        [target_node],
    )
}

/// The rows the due step handles: a past deadline and either an open request not yet marked
/// overdue (or, with its seat known, not yet warned in-thread), or a failed one whose answer
/// no CLI showed and which was not marked overdue yet. `after` is the `(deadline_at, request)`
/// of the last row of the previous page; the page starts past it.
pub fn due_rows(
    conn: &Connection,
    now: i64,
    after: Option<(i64, &str)>,
    limit: usize,
) -> rusqlite::Result<Vec<Obligation>> {
    let (after_deadline, after_request) = after.unzip();
    query(
        conn,
        "WHERE o.deadline_at<=?1 AND ( \
           (o.state IN ('sent','launched') AND (o.overdue_seq IS NULL OR (o.seat IS NOT NULL AND o.thread_warned_at IS NULL))) \
           OR (o.state='failed' AND o.alert_shown_at IS NULL AND o.overdue_seq IS NULL)) \
           AND (?3 IS NULL OR (o.deadline_at,o.request) > (?3,?4)) \
         ORDER BY o.deadline_at,o.request LIMIT ?2",
        params![now, limit as i64, after_deadline, after_request],
    )
}

/// One pending alert: `(request, target node, overdue_seq, launched seat)`.
pub type PendingAlert = (String, String, i64, Option<String>);

/// The author's pending alerts in `overdue_seq` order.
pub fn pending_alerts(conn: &Connection, author: &str) -> rusqlite::Result<Vec<PendingAlert>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT o.request,o.target_node,o.overdue_seq,o.seat FROM handoff_obligations o \
         WHERE o.author=?1 AND {ALERT_PENDING_SQL} ORDER BY o.overdue_seq"
    ))?;
    stmt.query_map([author], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
    })?
    .collect()
}

pub fn alert_count(conn: &Connection, author: &str) -> rusqlite::Result<i64> {
    conn.query_row(
        &format!(
            "SELECT count(*) FROM handoff_obligations o WHERE o.author=?1 AND {ALERT_PENDING_SQL}"
        ),
        [author],
        |r| r.get(0),
    )
}

pub fn latest_alert_seq(conn: &Connection, author: &str) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        &format!(
            "SELECT max(o.overdue_seq) FROM handoff_obligations o WHERE o.author=?1 AND {ALERT_PENDING_SQL}"
        ),
        [author],
        |r| r.get(0),
    )
}

/// Marks the named requests' alerts shown, only on rows of `author` whose alert is pending,
/// and wakes the author once if any left the predicate. Returns the number marked.
pub fn mark_alerts_shown(
    conn: &Connection,
    author: &str,
    requests: &[String],
    now: i64,
) -> Result<usize, ApiError> {
    let mut marked = 0;
    for request in requests {
        marked += conn
            .execute(
                &format!(
                    "UPDATE handoff_obligations AS o SET alert_shown_at=?3 \
                     WHERE o.request=?1 AND o.author=?2 AND {ALERT_PENDING_SQL}"
                ),
                params![request, author, now],
            )
            .map_err(store_error)?;
    }
    if marked > 0 {
        bump(conn, author)?;
    }
    Ok(marked)
}

/// Deletes `linked|expired|failed` rows older than [`PRUNE_AFTER_MS`] past `updated_at`; each
/// author that had an alert pending among them is woken once. Returns the number deleted.
pub fn prune(tx: &Connection, now: i64) -> Result<usize, ApiError> {
    let cutoff = now - PRUNE_AFTER_MS;
    let pending: Vec<String> = {
        let mut stmt = tx
            .prepare(&format!(
                "SELECT DISTINCT o.author FROM handoff_obligations o \
                 WHERE o.state IN ('linked','expired','failed') AND o.updated_at<?1 AND {ALERT_PENDING_SQL}"
            ))
            .map_err(store_error)?;
        stmt.query_map([cutoff], |r| r.get(0))
            .map_err(store_error)?
            .collect::<rusqlite::Result<_>>()
            .map_err(store_error)?
    };
    let deleted = tx
        .execute(
            "DELETE FROM handoff_obligations WHERE state IN ('linked','expired','failed') AND updated_at<?1",
            [cutoff],
        )
        .map_err(store_error)?;
    for author in pending {
        bump(tx, &author)?;
    }
    Ok(deleted)
}

#[cfg(test)]
#[path = "../../tests/store/handoff_obligations.rs"]
mod tests;
