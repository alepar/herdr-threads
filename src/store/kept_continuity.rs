//! Kept continuity claims (ht-dtx0, TRUST-POLICY C1).
//!
//! Responsibility (kept_continuity)
//! - Owns: the `kept_continuity_claims` table: [`keep`], [`due`] and [`forget`].
//! - Hides: the SQL, the per-session key, the same-process supersession, the
//!   count cap and the age bound.
//! - Must not: decide anything. A kept row is the session's own claim, stored
//!   verbatim; only `seats::decide_continuity`, run by the observation worker
//!   against the canonical view (A2), reattaches a seat.
//! - Role: written by `Identity::continuity` when a claim is refused only for
//!   host evidence that is not ready yet, read and pruned by the observation
//!   worker after each completed reconciliation pass.

use rusqlite::{Connection, Transaction, params};

use super::connection::{api_error, store_error};
use crate::ports::HostId;
use crate::protocol::commands::ContinuityCheckIn;
use crate::protocol::results::{ApiError, ErrorCode};

/// At most this many kept claims in all; a claim beyond it is answered but not kept.
pub const MAX_KEPT_CLAIMS: i64 = 256;
/// A kept claim is dropped this long after it was received, decided or not.
pub const KEPT_CLAIM_TTL_MS: i64 = 60 * 60 * 1000;
/// At most this many kept claims are re-submitted per reconciliation pass.
pub const KEPT_CLAIMS_PER_PASS: i64 = 16;

/// One kept claim, oldest first in [`due`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeptClaim {
    pub command: ContinuityCheckIn,
    pub received_at: i64,
}

/// Keep `command` for `host`, replacing the same session's earlier claim on the
/// same target (its age still runs from the first one: a session that keeps
/// claiming does not outlive [`KEPT_CLAIM_TTL_MS`]) and superseding the
/// process's claims for other sessions there ([`supersede`]). `false` when the
/// cap is reached (the claim is not kept).
pub fn keep(
    tx: &Transaction<'_>,
    host: HostId,
    command: &ContinuityCheckIn,
    now: i64,
) -> Result<bool, ApiError> {
    let json = serde_json::to_string(command)
        .map_err(|e| api_error(ErrorCode::StoreCorrupt, format!("kept claim encode: {e}")))?;
    // Before the cap, so a superseding claim the cap refuses still drops the old one.
    supersede(tx, host, command)?;
    let key = params![
        host.get(),
        command.target.as_str(),
        command.harness.as_str(),
        command.native_session.as_str()
    ];
    let exists: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM kept_continuity_claims WHERE host_id=?1 AND target_id=?2 AND harness=?3 AND native_session=?4)",
            key,
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if !exists {
        let count: i64 = tx
            .query_row("SELECT count(*) FROM kept_continuity_claims", [], |r| {
                r.get(0)
            })
            .map_err(store_error)?;
        if count >= MAX_KEPT_CLAIMS {
            return Ok(false);
        }
    }
    tx.execute(
        "INSERT INTO kept_continuity_claims(host_id,target_id,harness,native_session,command_json,received_at) \
         VALUES (?1,?2,?3,?4,?5,?6) \
         ON CONFLICT(host_id,target_id,harness,native_session) DO UPDATE SET \
           command_json=excluded.command_json",
        params![
            host.get(),
            command.target.as_str(),
            command.harness.as_str(),
            command.native_session.as_str(),
            json,
            now
        ],
    )
    .map_err(store_error)?;
    Ok(true)
}

/// A process claiming a session some binding already names (an in-process
/// `/resume` to another seat's session) has left the sessions it claimed
/// before on the same target: drop its kept claims for them, so an older
/// claim never reattaches its seat to a pane now running another seat's
/// session. A claim for a session no binding names (a `/clear` successor,
/// which no seat can match) supersedes nothing, so the old session's claim
/// can still reattach the pane's seat.
fn supersede(conn: &Connection, host: HostId, command: &ContinuityCheckIn) -> Result<(), ApiError> {
    let Some(liveness) = command.liveness else {
        return Ok(());
    };
    let known: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM occupant_bindings WHERE harness=?1 AND native_session=?2)",
            params![command.harness.as_str(), command.native_session.as_str()],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if !known {
        return Ok(());
    }
    let rows: Vec<(String, String, String)> = conn
        .prepare(
            "SELECT harness,native_session,command_json FROM kept_continuity_claims \
             WHERE host_id=?1 AND target_id=?2 AND native_session<>?3",
        )
        .map_err(store_error)?
        .query_map(
            params![
                host.get(),
                command.target.as_str(),
                command.native_session.as_str()
            ],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(store_error)?
        .collect::<Result<_, _>>()
        .map_err(store_error)?;
    for (harness, session, json) in rows {
        let same_process = serde_json::from_str::<ContinuityCheckIn>(&json)
            .is_ok_and(|kept| kept.liveness == Some(liveness));
        if same_process {
            conn.execute(
                "DELETE FROM kept_continuity_claims WHERE host_id=?1 AND target_id=?2 AND harness=?3 AND native_session=?4",
                params![host.get(), command.target.as_str(), harness, session],
            )
            .map_err(store_error)?;
        }
    }
    Ok(())
}

/// Drop `host`'s claims older than [`KEPT_CLAIM_TTL_MS`] (and any row this
/// binary cannot decode), then return up to [`KEPT_CLAIMS_PER_PASS`] of the
/// rest, oldest first.
pub fn due(tx: &Transaction<'_>, host: HostId, now: i64) -> Result<Vec<KeptClaim>, ApiError> {
    tx.execute(
        "DELETE FROM kept_continuity_claims WHERE host_id=?1 AND received_at<?2",
        params![host.get(), now.saturating_sub(KEPT_CLAIM_TTL_MS)],
    )
    .map_err(store_error)?;
    let rows: Vec<(String, String, String, String, i64)> = tx
        .prepare(
            "SELECT target_id,harness,native_session,command_json,received_at FROM kept_continuity_claims \
             WHERE host_id=?1 ORDER BY received_at, target_id, harness, native_session LIMIT ?2",
        )
        .map_err(store_error)?
        .query_map(params![host.get(), KEPT_CLAIMS_PER_PASS], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .map_err(store_error)?
        .collect::<Result<_, _>>()
        .map_err(store_error)?;
    let mut claims = Vec::with_capacity(rows.len());
    for (target, harness, session, json, received_at) in rows {
        match serde_json::from_str::<ContinuityCheckIn>(&json) {
            Ok(command) => claims.push(KeptClaim {
                command,
                received_at,
            }),
            Err(_) => {
                tx.execute(
                    "DELETE FROM kept_continuity_claims WHERE host_id=?1 AND target_id=?2 AND harness=?3 AND native_session=?4",
                    params![host.get(), target, harness, session],
                )
                .map_err(store_error)?;
            }
        }
    }
    Ok(claims)
}

/// Drop the kept claim of `command`'s session on its target, if any; with
/// `loaded`, only while the row is still the one loaded (a newer claim of the
/// session that replaced it meanwhile stays). Without `loaded` (a claim the
/// session just made and the daemon decided) it also supersedes the process's
/// claims for other sessions there, as [`keep`] does.
pub fn forget(
    conn: &Connection,
    host: HostId,
    command: &ContinuityCheckIn,
    loaded: Option<&KeptClaim>,
) -> Result<(), ApiError> {
    let loaded_json = loaded
        .map(|claim| serde_json::to_string(&claim.command))
        .transpose()
        .map_err(|e| api_error(ErrorCode::StoreCorrupt, format!("kept claim encode: {e}")))?;
    conn.execute(
        "DELETE FROM kept_continuity_claims WHERE host_id=?1 AND target_id=?2 AND harness=?3 AND native_session=?4 \
         AND (?5 IS NULL OR command_json=?5)",
        params![
            host.get(),
            command.target.as_str(),
            command.harness.as_str(),
            command.native_session.as_str(),
            loaded_json
        ],
    )
    .map_err(store_error)?;
    if loaded.is_none() {
        supersede(conn, host, command)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/store/kept_continuity.rs"]
mod tests;
