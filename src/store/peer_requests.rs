//! Peer-requested launches (`peer_requests`, remote launch, TRUST-POLICY N15).
//!
//! Responsibility
//! - Owns: the one row per request id a linked peer sent, recorded before
//!   this node acts, its progress (`running` with the pane once a tab exists,
//!   then `launched`, `refused` or `failed`), the restart rule (a `running`
//!   row is failed `interrupted`, never resumed) and the 7-day retention.
//! - Hides: the SQL and the plan's JSON form.
//! - Must not: decide whether to launch (the peer launcher does), or record
//!   anything for a seat as its actor: every row is the requesting node's.
//!
//! Core and harness-neutral.

use rusqlite::{Connection, OptionalExtension, params};

use crate::sync::frame::LaunchState;

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
    pub created_at: i64,
    pub updated_at: i64,
}

impl PeerRequest {
    /// The state a repeat of this request is answered with.
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
    })
}

const COLUMNS: &str =
    "id,node_uuid,requester_seat,kind,plan,state,pane,seat,detail,created_at,updated_at";

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
        "INSERT INTO peer_requests(id,node_uuid,requester_seat,kind,plan,state,pane,seat,detail,created_at,updated_at) \
         VALUES (?1,?2,?3,?4,?5,'running',NULL,NULL,NULL,?6,?6) ON CONFLICT(id) DO NOTHING",
        params![new.id, new.node_uuid, new.requester_seat, new.kind, new.plan, new.created_at],
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
    conn.execute(
        "UPDATE peer_requests SET state='failed', detail='interrupted: the daemon restarted', \
         updated_at=?1 WHERE state='running'",
        [now],
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
            let count: i64 = r.get(11)?;
            Ok((
                request.node_uuid.clone(),
                u64::try_from(count).unwrap_or(0),
                request,
            ))
        })?
        .collect()
}
