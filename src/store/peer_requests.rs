//! Peer-requested launches (`peer_requests`, remote launch, TRUST-POLICY N15).
//!
//! Responsibility
//! - Owns: the one row per request id a linked peer sent, recorded before
//!   this node acts, its progress (`running` with the pane once a tab exists,
//!   then `launched`, `refused` or `failed`), the restart rule (a `running`
//!   row is failed `interrupted`, never resumed) and the 7-day retention.
//!   For a peer launch onto a herdr-core, the row also holds the launch gate's
//!   gate-only record (`gate_attempt`, `gate_created`) and its read, `gate_view`.
//! - Hides: the SQL and the plan's JSON form.
//! - Must not: decide whether to launch (the peer launcher does), or record
//!   anything for a seat as its actor: every row is the requesting node's.
//!
//! Core and harness-neutral.

use rusqlite::{Connection, OptionalExtension, params};

use crate::sync::frame::{LaunchStaged, LaunchState};

/// How a recorded detail of a start that may have happened begins: the row's
/// own marker, read back only by [`PeerRequest::answer`] on this node.
pub const UNCONFIRMED: &str = "start unconfirmed";

/// Longest serialized plan the table takes (its CHECK), in bytes.
pub const MAX_PLAN_BYTES: usize = 131_072;

/// Rows older than this are deleted (ms).
pub const RETENTION_MS: i64 = 7 * 24 * 3_600_000;

/// One recorded request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerRequest {
    pub id: String,
    pub node_uuid: String,
    pub requester_seat: Option<String>,
    pub kind: String,
    pub plan: String,
    pub state: String,
    pub pane: Option<String>,
    pub seat: Option<String>,
    pub detail: Option<String>,
    /// The pair thread and message a handoff staged here (set together), else `None`.
    pub handoff_thread: Option<String>,
    pub handoff_message: Option<String>,
    /// The handoff prompt frozen at decide, else `None`.
    pub handoff_prompt: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl PeerRequest {
    /// The state a repeat of this request is answered with.
    ///
    /// The only place a recorded `Failed` is built: the worker's failure, `settle_at_start`'s
    /// interrupted rows and repeat ids all read it back, so `staged` is set exactly when both
    /// handoff columns are, whatever the pane and the unconfirmed marker say.
    pub fn answer(&self) -> LaunchState {
        let detail = || self.detail.clone().unwrap_or_default();
        match self.state.as_str() {
            "running" => LaunchState::Running {
                pane: self.pane.clone(),
            },
            "launched" => LaunchState::Launched {
                seat: self.seat.clone().unwrap_or_default(),
                pane: self.pane.clone().unwrap_or_default(),
            },
            "refused" => LaunchState::Refused { detail: detail() },
            _ => LaunchState::Failed {
                detail: detail(),
                pane: self.pane.clone(),
                unconfirmed: self
                    .detail
                    .as_deref()
                    .is_some_and(|d| d.starts_with(UNCONFIRMED)),
                staged: match (&self.handoff_thread, &self.handoff_message) {
                    (Some(thread), Some(message)) => Some(LaunchStaged {
                        thread: thread.clone(),
                        message: message.clone(),
                    }),
                    _ => None,
                },
            },
        }
    }
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<PeerRequest> {
    Ok(PeerRequest {
        id: r.get(0)?,
        node_uuid: r.get(1)?,
        requester_seat: r.get(2)?,
        kind: r.get(3)?,
        plan: r.get(4)?,
        state: r.get(5)?,
        pane: r.get(6)?,
        seat: r.get(7)?,
        detail: r.get(8)?,
        created_at: r.get(9)?,
        updated_at: r.get(10)?,
        handoff_thread: r.get(11)?,
        handoff_message: r.get(12)?,
        handoff_prompt: r.get(13)?,
    })
}

const COLUMNS: &str = "id,node_uuid,requester_seat,kind,plan,state,pane,seat,detail,created_at,updated_at,\
     handoff_thread,handoff_message,handoff_prompt";

pub fn get(conn: &Connection, id: &str) -> rusqlite::Result<Option<PeerRequest>> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM peer_requests WHERE id=?1"),
        [id],
        row,
    )
    .optional()
}

/// Records a new request as `running`, or returns the row already recorded
/// for its id (whatever its node: an id is the requester's and unique).
pub fn insert_or_get(
    conn: &Connection,
    new: &PeerRequest,
) -> rusqlite::Result<(PeerRequest, bool)> {
    let inserted = conn.execute(
        "INSERT INTO peer_requests(id,node_uuid,requester_seat,kind,plan,state,pane,seat,detail,created_at,updated_at,handoff_prompt) \
         VALUES (?1,?2,?3,?4,?5,'running',?7,NULL,NULL,?6,?6,?8) ON CONFLICT(id) DO NOTHING",
        params![
            new.id,
            new.node_uuid,
            new.requester_seat,
            new.kind,
            new.plan,
            new.created_at,
            new.pane,
            new.handoff_prompt
        ],
    )?;
    let stored = get(conn, &new.id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)?;
    Ok((stored, inserted == 1))
}

/// Records progress or an outcome of a `running` request; a settled row is
/// never changed again.
pub fn record(
    conn: &Connection,
    id: &str,
    state: &str,
    pane: Option<&str>,
    seat: Option<&str>,
    detail: Option<&str>,
    now: i64,
) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "UPDATE peer_requests SET state=?2, pane=COALESCE(?3,pane), seat=COALESCE(?4,seat), \
         detail=COALESCE(?5,detail), updated_at=?6 WHERE id=?1 AND state='running'",
        params![id, state, pane, seat, detail, now],
    )?;
    Ok(changed == 1)
}

/// The attempt of a peer launch's gate: a request is never retried here (N15).
pub const GATE_ATTEMPT: u32 = 1;

/// Makes `id` a gate-only record, before its terminal exists: the shim is told to wait,
/// not refused as unknown.
pub fn open_gate(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE peer_requests SET gate_attempt=?2 WHERE id=?1",
        params![id, GATE_ATTEMPT],
    )?;
    Ok(())
}

/// The creation, seat and terminal of a gate-only record, and its outcome, in one write:
/// the view releases the gate on this row, so the seat always precedes the gate.
pub fn record_gate_launch(
    conn: &Connection,
    id: &str,
    created: &str,
    pane: &str,
    seat: &str,
    now: i64,
) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "UPDATE peer_requests SET state='launched', gate_created=?2, pane=?3, seat=?4, \
         updated_at=?5 WHERE id=?1 AND state='running' AND gate_attempt IS NOT NULL",
        params![id, created, pane, seat, now],
    )?;
    Ok(changed == 1)
}

/// What staging reads of a handoff request: the row's plan and its staged thread and message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffView {
    pub node_uuid: String,
    pub plan: String,
    pub state: String,
    pub seat: Option<String>,
    pub handoff_thread: Option<String>,
    pub handoff_message: Option<String>,
}

/// The handoff columns of request `id`; `None` for no row.
pub fn handoff_view(conn: &Connection, id: &str) -> rusqlite::Result<Option<HandoffView>> {
    conn.query_row(
        "SELECT node_uuid,plan,state,seat,handoff_thread,handoff_message FROM peer_requests WHERE id=?1",
        [id],
        |r| {
            Ok(HandoffView {
                node_uuid: r.get(0)?,
                plan: r.get(1)?,
                state: r.get(2)?,
                seat: r.get(3)?,
                handoff_thread: r.get(4)?,
                handoff_message: r.get(5)?,
            })
        },
    )
    .optional()
}

/// Records the staged pair thread and message of a `running` handoff request in one write; false
/// when the row is not `running`.
pub fn record_handoff(
    conn: &Connection,
    id: &str,
    thread: &str,
    message: &str,
    now: i64,
) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "UPDATE peer_requests SET handoff_thread=?2, handoff_message=?3, updated_at=?4 \
         WHERE id=?1 AND state='running'",
        params![id, thread, message, now],
    )?;
    Ok(changed == 1)
}

/// The launch gate's read of a gate-only record; `None` for no row or a row without a gate.
/// A launch-only request has no local Send (N15), so a recorded creation is the release. A
/// handoff request (its plan carries a sender block) releases only on its staged Send: the
/// message `stage` wrote exists, as a local hc handoff releases on its staged Send.
pub fn gate_view(
    db: &Connection,
    id: &str,
    attempt: crate::protocol::handoff::BootstrapAttempt,
) -> Result<Option<crate::ports::LaunchGateView>, crate::protocol::results::ApiError> {
    use crate::store::connection::store_error;
    type Found = (u32, Option<String>, String, String, bool);
    let found: Option<Found> = db
        .query_row(
            "SELECT gate_attempt, gate_created, state, plan, \
             handoff_message IS NOT NULL AND EXISTS(SELECT 1 FROM messages WHERE id=handoff_message) \
             FROM peer_requests WHERE id=?1 AND gate_attempt IS NOT NULL",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()
        .map_err(store_error)?;
    let Some((gate_attempt, created, state, plan, message_exists)) = found else {
        return Ok(None);
    };
    let created = created
        .map(|text| serde_json::from_str::<crate::ports::CreatedTarget>(&text))
        .transpose()
        .map_err(|e| {
            crate::protocol::results::ApiError::store_corrupt(format!(
                "peer launch gate record: {e}"
            ))
        })?;
    // An unreadable plan is a launch-only row: the gate never waits on a message it cannot name.
    let handoff = serde_json::from_str::<crate::sync::frame::LaunchPlan>(&plan)
        .is_ok_and(|plan| plan.handoff().is_some());
    Ok(Some(crate::ports::LaunchGateView {
        send_staged: created.is_some() && (!handoff || message_exists),
        created,
        current_attempt: gate_attempt == attempt.get(),
        cancelled: matches!(state.as_str(), "failed" | "refused"),
    }))
}

/// Rows are pruned only here, at daemon start; the duplicate guard of an id
/// lapses with its row after [`RETENTION_MS`].
///
/// At daemon start: every request still `running` was interrupted (on Linux
/// its child ended with the old daemon; on macOS an orphaned child may still
/// finish); it is failed, never resumed. Old rows are
/// deleted. Returns the ids failed.
pub fn settle_at_start(conn: &Connection, now: i64) -> rusqlite::Result<Vec<String>> {
    let ids: Vec<String> = {
        let mut statement = conn.prepare("SELECT id FROM peer_requests WHERE state='running'")?;
        statement
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?
    };
    // One that already names its pane may have started the agent there.
    conn.execute(
        "UPDATE peer_requests SET state='failed', detail=CASE WHEN pane IS NULL \
         THEN 'interrupted: the daemon restarted' \
         ELSE ?2 || ': interrupted by a daemon restart; inspect pane ' || pane || \
         ' here before launching again' END, updated_at=?1 WHERE state='running'",
        rusqlite::params![now, UNCONFIRMED],
    )?;
    conn.execute(
        "DELETE FROM peer_requests WHERE created_at < ?1",
        [now - RETENTION_MS],
    )?;
    Ok(ids)
}

/// Per requesting node: how many requests it made in the retention window,
/// and the newest one.
pub fn by_node(conn: &Connection) -> rusqlite::Result<Vec<(String, u64, PeerRequest)>> {
    let mut statement = conn.prepare(&format!(
        "SELECT {COLUMNS}, (SELECT COUNT(*) FROM peer_requests p WHERE p.node_uuid=r.node_uuid) \
         FROM peer_requests r WHERE created_at = (SELECT MAX(created_at) FROM peer_requests q \
         WHERE q.node_uuid=r.node_uuid) GROUP BY node_uuid ORDER BY node_uuid"
    ))?;
    statement
        .query_map([], |r| {
            let request = row(r)?;
            let count: i64 = r.get(14)?;
            Ok((
                request.node_uuid.clone(),
                u64::try_from(count).unwrap_or(0),
                request,
            ))
        })?
        .collect()
}
