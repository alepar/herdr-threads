//! Durable bounded work admission and committed-prefix progress.

pub use crate::ports::{
    DurableWorkAdmission as WorkAdmission, WorkCandidate, WorkKind, WorkProgress,
};
use crate::protocol::results::{ApiError, ErrorCode};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::connection::{api_error, store_error};

impl WorkKind {
    fn as_sql(self) -> &'static str {
        match self {
            Self::WarningAttribution => "warning_attribution",
            Self::SendAttention => "send_attention",
            Self::ReceiptTimerMaterialization => "receipt_timer_materialization",
            Self::PreparationCleanup => "preparation_cleanup",
        }
    }
}

pub fn discover_work(
    db: &Connection,
    kind: WorkKind,
    after_ordinal: u64,
    limit: u16,
) -> Result<Vec<WorkCandidate>, ApiError> {
    if limit == 0 || limit > 100 {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid work page limit",
        ));
    }
    let mut statement = db
        .prepare(
            "SELECT id, position, high_water FROM work_jobs WHERE kind=?1 AND ordinal>?2 \
         AND status IN ('pending','failed') ORDER BY ordinal LIMIT ?3",
        )
        .map_err(store_error)?;
    let after = sql_integer(after_ordinal)?;
    let rows = statement
        .query_map(params![kind.as_sql(), after, limit], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })
        .map_err(store_error)?;
    rows.map(|row| {
        let (id, position, high_water) = row.map_err(store_error)?;
        Ok(WorkCandidate {
            id,
            kind,
            position: nonnegative(position)?,
            high_water: nonnegative(high_water)?,
            has_more: true,
        })
    })
    .collect()
}

pub fn commit_work_prefix(
    tx: &Transaction<'_>,
    id: &str,
    expected_position: u64,
    units: u8,
    next_position: u64,
    has_more: bool,
    last_error: Option<&str>,
) -> Result<WorkProgress, ApiError> {
    WorkAdmission::new(units).map_err(|detail| api_error(ErrorCode::InvalidRequest, detail))?;
    if last_error.is_some_and(|e| e.len() > 512) {
        return Err(api_error(ErrorCode::InvalidRequest, "work error too large"));
    }
    let row: Option<(i64, i64, i64)> = tx.query_row(
        "SELECT position, high_water, completed_units FROM work_jobs WHERE id=?1 AND status IN ('pending','failed')",
        [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    ).optional().map_err(store_error)?;
    let Some((position, high_water, prior_units)) = row else {
        return Err(api_error(
            ErrorCode::Conflict,
            "work job missing or complete",
        ));
    };
    let (position, high_water, prior_units) = (
        nonnegative(position)?,
        nonnegative(high_water)?,
        nonnegative(prior_units)?,
    );
    if position != expected_position
        || next_position < position
        || next_position > high_water
        || (!has_more && next_position != high_water)
    {
        return Err(api_error(
            ErrorCode::Conflict,
            "work prefix does not match durable cursor",
        ));
    }
    let completed_units = prior_units
        .checked_add(u64::from(units))
        .ok_or_else(|| api_error(ErrorCode::StoreCorrupt, "work unit count exhausted"))?;
    let status = if !has_more {
        "complete"
    } else if last_error.is_some() {
        "failed"
    } else {
        "pending"
    };
    tx.execute("UPDATE work_jobs SET position=?1, completed_units=?2, status=?3, last_error=?4 WHERE id=?5 AND position=?6",
        params![sql_integer(next_position)?, sql_integer(completed_units)?, status, last_error, id, sql_integer(expected_position)?]).map_err(store_error)?;
    Ok(WorkProgress {
        completed_units,
        processed_this_turn: units,
        has_more,
        next_position,
        last_error: last_error.map(str::to_owned),
    })
}

fn sql_integer(value: u64) -> Result<i64, ApiError> {
    i64::try_from(value).map_err(|_| {
        api_error(
            ErrorCode::InvalidRequest,
            "work position exceeds SQLite range",
        )
    })
}

fn nonnegative(value: i64) -> Result<u64, ApiError> {
    u64::try_from(value)
        .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative durable work position"))
}

#[cfg(test)]
#[path = "../../tests/store/work.rs"]
mod tests;
