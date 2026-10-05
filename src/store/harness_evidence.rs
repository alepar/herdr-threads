//! Harness version evidence (ht-xoc.4): what hook payloads showed about each
//! (harness, version, contract id) and the latest reason a payload could not
//! be attributed to a version.
//!
//! The rows are advisory data about the harness, never authority: the daemon
//! writes them from its own process, with no seat or caller identity. A row is
//! verified once a lifecycle payload and a tool payload both matched the
//! contract; the first violation is sticky and only a different version or
//! contract id (a different row) starts clean.
//!
//! Retention: a row may be pruned once its `last_seen_at` is older than
//! [`EVIDENCE_RETENTION_MS`] (30 days; Health's window is 24 hours and doctor's
//! history keeps a month). The newest [`EVIDENCE_KEEP_PER_HARNESS`] rows of
//! each harness (by `last_seen_at`, across contract ids) are never pruned,
//! whatever their age. Pruning runs inside `record()`'s write transaction and
//! only when a row was created (the only time the table grows). [`all`] reads
//! at most [`EVIDENCE_READ_CAP`] rows of one harness, the newest by
//! `last_seen_at`.

use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};

use super::connection::{StoreContext, store_error};
pub use crate::harness::contract::EventClass;
use crate::protocol::{results::ApiError, time::UtcMillis};

/// One evidence row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceRow {
    pub harness: String,
    pub version: String,
    pub contract_id: String,
    pub first_seen_at: u64,
    pub lifecycle_ok_at: Option<u64>,
    pub tool_ok_at: Option<u64>,
    pub violation_at: Option<u64>,
    pub violation_event: Option<String>,
    pub violation_field: Option<String>,
    pub last_seen_at: u64,
}

impl EvidenceRow {
    /// A lifecycle payload and a tool payload both matched the contract.
    pub fn verified(&self) -> bool {
        self.lifecycle_ok_at.is_some() && self.tool_ok_at.is_some()
    }
}

/// What one payload showed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvidenceOutcome {
    Ok,
    Violation { field: String },
    Malformed,
}

/// One payload's evidence for a (harness, version, contract id).
#[derive(Debug, Clone, Copy)]
pub struct EvidenceRecord<'a> {
    pub harness: &'a str,
    pub version: &'a str,
    pub contract_id: &'a str,
    pub event: &'a str,
    pub class: EventClass,
    pub outcome: &'a EvidenceOutcome,
}

/// The result of recording: whether the row is new, whether this call set its
/// sticky violation, and the row afterwards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorded {
    pub created: bool,
    pub fresh_violation: bool,
    pub row: EvidenceRow,
}

/// A row unseen for longer than this may be pruned (30 days).
pub const EVIDENCE_RETENTION_MS: i64 = 30 * 24 * 60 * 60 * 1000;
/// The newest rows of each harness that pruning never removes.
pub const EVIDENCE_KEEP_PER_HARNESS: i64 = 64;
/// The most rows `all` returns for one harness.
pub const EVIDENCE_READ_CAP: i64 = 256;

const COLUMNS: &str = "harness, version, contract_id, first_seen_at, lifecycle_ok_at, tool_ok_at, \
     violation_at, violation_event, violation_field, last_seen_at";

fn ms(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

fn opt_ms(value: Option<i64>) -> Option<u64> {
    value.map(ms)
}

fn row_of(row: &Row<'_>) -> rusqlite::Result<EvidenceRow> {
    Ok(EvidenceRow {
        harness: row.get(0)?,
        version: row.get(1)?,
        contract_id: row.get(2)?,
        first_seen_at: ms(row.get(3)?),
        lifecycle_ok_at: opt_ms(row.get(4)?),
        tool_ok_at: opt_ms(row.get(5)?),
        violation_at: opt_ms(row.get(6)?),
        violation_event: row.get(7)?,
        violation_field: row.get(8)?,
        last_seen_at: ms(row.get(9)?),
    })
}

fn now_ms(context: &StoreContext) -> i64 {
    let UtcMillis(now) = context.clock().utc_now();
    now
}

/// Records one payload's evidence in one write transaction.
pub fn record(
    context: &StoreContext,
    writer: &mut Connection,
    record: &EvidenceRecord<'_>,
) -> Result<Recorded, ApiError> {
    let now = now_ms(context);
    let tx = writer
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(store_error)?;
    let key = params![record.harness, record.version, record.contract_id];
    let created = tx
        .execute(
            "INSERT OR IGNORE INTO harness_version_evidence \
             (harness, version, contract_id, first_seen_at, last_seen_at) \
             VALUES (?1, ?2, ?3, ?4, ?4)",
            params![record.harness, record.version, record.contract_id, now],
        )
        .map_err(store_error)?
        == 1;
    if created {
        tx.execute(
            "DELETE FROM harness_version_evidence \
              WHERE harness = ?1 AND last_seen_at < ?2 \
                AND (harness, version, contract_id) NOT IN ( \
                  SELECT harness, version, contract_id FROM harness_version_evidence \
                   WHERE harness = ?1 ORDER BY last_seen_at DESC LIMIT ?3)",
            params![
                record.harness,
                now.saturating_sub(EVIDENCE_RETENTION_MS),
                EVIDENCE_KEEP_PER_HARNESS
            ],
        )
        .map_err(store_error)?;
    }
    let mut fresh_violation = false;
    tx.execute(
        "UPDATE harness_version_evidence SET last_seen_at = MAX(last_seen_at, ?4) \
         WHERE harness = ?1 AND version = ?2 AND contract_id = ?3",
        params![record.harness, record.version, record.contract_id, now],
    )
    .map_err(store_error)?;
    match record.outcome {
        EvidenceOutcome::Ok => {
            let column = match record.class {
                EventClass::Lifecycle => Some("lifecycle_ok_at"),
                EventClass::Tool => Some("tool_ok_at"),
                EventClass::Other => None,
            };
            if let Some(column) = column {
                tx.execute(
                    &format!(
                        "UPDATE harness_version_evidence SET {column} = COALESCE({column}, ?4) \
                         WHERE harness = ?1 AND version = ?2 AND contract_id = ?3"
                    ),
                    params![record.harness, record.version, record.contract_id, now],
                )
                .map_err(store_error)?;
            }
        }
        EvidenceOutcome::Violation { field } => {
            fresh_violation = tx
                .execute(
                    "UPDATE harness_version_evidence \
                     SET violation_at = ?4, violation_event = ?5, violation_field = ?6 \
                     WHERE harness = ?1 AND version = ?2 AND contract_id = ?3 \
                       AND violation_at IS NULL",
                    params![
                        record.harness,
                        record.version,
                        record.contract_id,
                        now,
                        record.event,
                        field
                    ],
                )
                .map_err(store_error)?
                == 1;
        }
        EvidenceOutcome::Malformed => {}
    }
    let row = tx
        .query_row(
            &format!(
                "SELECT {COLUMNS} FROM harness_version_evidence \
                 WHERE harness = ?1 AND version = ?2 AND contract_id = ?3"
            ),
            key,
            row_of,
        )
        .map_err(store_error)?;
    tx.commit().map_err(store_error)?;
    Ok(Recorded {
        created,
        fresh_violation,
        row,
    })
}

pub fn get(
    db: &Connection,
    harness: &str,
    version: &str,
    contract_id: &str,
) -> Result<Option<EvidenceRow>, ApiError> {
    db.query_row(
        &format!(
            "SELECT {COLUMNS} FROM harness_version_evidence \
             WHERE harness = ?1 AND version = ?2 AND contract_id = ?3"
        ),
        params![harness, version, contract_id],
        row_of,
    )
    .optional()
    .map_err(store_error)
}

/// Rows of both harnesses with `last_seen_at >= since_ms`.
pub fn since(db: &Connection, since_ms: u64) -> Result<Vec<EvidenceRow>, ApiError> {
    let since = i64::try_from(since_ms).unwrap_or(i64::MAX);
    collect(
        db,
        &format!(
            "SELECT {COLUMNS} FROM harness_version_evidence WHERE last_seen_at >= ?1 \
             ORDER BY harness, first_seen_at, version, contract_id"
        ),
        params![since],
    )
}

/// The newest `EVIDENCE_READ_CAP` rows of one harness (by `last_seen_at`),
/// ordered by `first_seen_at, version, contract_id`.
pub fn all(db: &Connection, harness: &str) -> Result<Vec<EvidenceRow>, ApiError> {
    collect(
        db,
        &format!(
            "SELECT {COLUMNS} FROM (SELECT {COLUMNS} FROM harness_version_evidence \
             WHERE harness = ?1 ORDER BY last_seen_at DESC LIMIT ?2) \
             ORDER BY first_seen_at, version, contract_id"
        ),
        params![harness, EVIDENCE_READ_CAP],
    )
}

fn collect(
    db: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<Vec<EvidenceRow>, ApiError> {
    let mut stmt = db.prepare(sql).map_err(store_error)?;
    stmt.query_map(params, row_of)
        .map_err(store_error)?
        .collect::<Result<_, _>>()
        .map_err(store_error)
}

/// Keeps the newest reason a payload of `harness` could not be attributed.
pub fn record_unattributed(
    context: &StoreContext,
    writer: &mut Connection,
    harness: &str,
    reason: &str,
) -> Result<(), ApiError> {
    writer
        .execute(
            "INSERT INTO harness_unattributed(harness, reason, at) VALUES (?1, ?2, ?3) \
             ON CONFLICT(harness) DO UPDATE SET reason = excluded.reason, at = excluded.at",
            params![harness, reason, now_ms(context)],
        )
        .map_err(store_error)?;
    Ok(())
}

pub fn last_unattributed(
    db: &Connection,
    harness: &str,
) -> Result<Option<(String, u64)>, ApiError> {
    db.query_row(
        "SELECT reason, at FROM harness_unattributed WHERE harness = ?1",
        [harness],
        |row| Ok((row.get::<_, String>(0)?, ms(row.get(1)?))),
    )
    .optional()
    .map_err(store_error)
}

/// Fixed producer scope: an unavailable-runtime violation under one hook contract.
/// No diagnostic row can qualify a runtime, capability, receipt or binding.
#[derive(Debug, Clone, Copy)]
pub struct DiagnosticRecord<'a> {
    pub harness: &'a str,
    pub session_id: &'a str,
    pub contract_id: &'a str,
    pub event: &'a str,
    pub field: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticRow {
    pub harness: String,
    pub session_id: String,
    pub contract_id: String,
    pub event: String,
    pub field: String,
    pub first_seen_at: u64,
    pub last_seen_at: u64,
}

impl DiagnosticRow {
    pub fn line(&self) -> String {
        format!(
            "harness {} contract input failure: {}/{}; runtime metadata unavailable",
            self.harness, self.event, self.field
        )
    }
}

/// Hard storage cap per harness, separate from historical version retention.
pub const DIAGNOSTIC_KEEP_PER_HARNESS: i64 = 256;
/// At most this many scoped failures are read/projected, newest first.
pub const DIAGNOSTIC_READ_CAP: i64 = 20;
/// Expired failures leave the projection even without another write.
pub const DIAGNOSTIC_RETENTION_MS: i64 = 30 * 24 * 60 * 60 * 1000;

/// Preserve the first event/field; only another violation touches last_seen_at.
/// Creation prunes expiry and evicts deterministically in the same transaction.
pub fn record_diagnostic(
    context: &StoreContext,
    writer: &mut Connection,
    record: &DiagnosticRecord<'_>,
) -> Result<(), ApiError> {
    let now = now_ms(context);
    let tx = writer
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(store_error)?;
    tx.execute(
        "DELETE FROM harness_contract_diagnostics WHERE harness=?1 AND last_seen_at < ?2",
        params![record.harness, now.saturating_sub(DIAGNOSTIC_RETENTION_MS)],
    )
    .map_err(store_error)?;
    tx.execute("INSERT INTO harness_contract_diagnostics
        (harness,session_id,contract_id,event,field,first_seen_at,last_seen_at)
        VALUES (?1,?2,?3,?4,?5,?6,?6)
        ON CONFLICT(harness,session_id,contract_id) DO UPDATE SET last_seen_at=MAX(last_seen_at,excluded.last_seen_at)",
        params![record.harness,record.session_id,record.contract_id,record.event,record.field,now]).map_err(store_error)?;
    tx.execute("DELETE FROM harness_contract_diagnostics WHERE harness=?1 AND (session_id,contract_id) NOT IN (
        SELECT session_id,contract_id FROM harness_contract_diagnostics WHERE harness=?1
        ORDER BY last_seen_at DESC,first_seen_at DESC,session_id,contract_id LIMIT ?2)",
        params![record.harness, DIAGNOSTIC_KEEP_PER_HARNESS]).map_err(store_error)?;
    tx.commit().map_err(store_error)
}

pub fn diagnostics(
    db: &Connection,
    harness: &str,
    now: UtcMillis,
) -> Result<Vec<DiagnosticRow>, ApiError> {
    let mut stmt = db
        .prepare(
            "SELECT harness,session_id,contract_id,event,field,first_seen_at,last_seen_at
        FROM harness_contract_diagnostics WHERE harness=?1 AND last_seen_at >= ?2
        ORDER BY last_seen_at DESC,first_seen_at DESC,session_id,contract_id LIMIT ?3",
        )
        .map_err(store_error)?;
    stmt.query_map(
        params![
            harness,
            now.0.saturating_sub(DIAGNOSTIC_RETENTION_MS),
            DIAGNOSTIC_READ_CAP
        ],
        |row| {
            Ok(DiagnosticRow {
                harness: row.get(0)?,
                session_id: row.get(1)?,
                contract_id: row.get(2)?,
                event: row.get(3)?,
                field: row.get(4)?,
                first_seen_at: ms(row.get(5)?),
                last_seen_at: ms(row.get(6)?),
            })
        },
    )
    .map_err(store_error)?
    .collect::<Result<_, _>>()
    .map_err(store_error)
}

#[cfg(test)]
#[path = "../../tests/store/harness_evidence.rs"]
mod tests;
