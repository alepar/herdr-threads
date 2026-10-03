//! Retention (nested spec D3): bounded pruning of superseded snapshot
//! generations, completed work jobs and abandoned send preparation bodies.
//!
//! One pass runs snapshot, work-job and unpublished-preparation transactions, each
//! touching at most [`RETENTION_BATCH_ROWS`] rows and stopping early once
//! [`RETENTION_QUANTUM_MS`] has elapsed. Candidates are found first on a query
//! connection; a write transaction opens only when a row qualifies, so an idle
//! daemon makes no Retention-origin commit.
//!
//! Snapshot keep set, per instance (never pruned): the active generation, the
//! previous published one (greatest `admission_sequence` below the active),
//! the recovery baseline, every generation an unresolved seat references,
//! and every in-flight stage (`building`/`sealed` with `admission_sequence >
//! observation_decided_sequence`). The current baseline's
//! `recovery_baseline_releases` ride with the baseline generation: both tables
//! survived schema v11 (`migrations/0011_cooperative_only.sql`), so both
//! clauses are present. Prunable: older superseded published generations,
//! discarded generations, and dead stages (`admission_sequence <= decided`,
//! which publish rejects).
use super::{SqliteStore, connection::store_error};
pub use crate::ports::PruneProgress;
use crate::protocol::{
    results::ApiError,
    time::{CallBudget, Clock},
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

/// Most rows one retention transaction may change.
pub const RETENTION_BATCH_ROWS: usize = 256;
/// A retention transaction stops starting new units after this many ms.
pub const RETENTION_QUANTUM_MS: u64 = 5;
/// A completed work job of a pruned kind is deleted this long after completion.
pub const WORK_JOB_RETENTION_MS: i64 = 24 * 60 * 60 * 1000;
/// Kinds of completed jobs retention deletes. `preparation_cleanup` is kept
/// (its completion row is the marker `messages.rs` reads); wake work,
/// warnings, receipts and retired seats are not work jobs.
pub const PRUNED_JOB_KINDS: [&str; 3] = [
    "send_attention",
    "warning_attribution",
    "receipt_timer_materialization",
];

/// The pruning predicate over `g` (a generation), `h` (its host instance) and
/// `a` (the active generation, possibly absent). `{index}` is the index hint
/// for the scan and is empty for a by-id re-check.
const CANDIDATE_SQL: &str = "\
SELECT g.id, g.status \
FROM snapshot_generations g {index} \
JOIN host_instances h ON h.id = g.instance_id \
LEFT JOIN snapshot_generations a ON a.id = h.active_snapshot_id \
WHERE (g.status = 'discarded' \
    OR (g.status IN ('building','sealed') AND g.admission_sequence <= h.observation_decided_sequence) \
    OR (g.status = 'published' AND a.id IS NOT NULL AND g.admission_sequence < a.admission_sequence)) \
  AND g.id IS NOT h.active_snapshot_id \
  AND g.id IS NOT h.recovery_baseline_generation_id \
  AND NOT EXISTS (SELECT 1 FROM seats s WHERE s.unresolved_from_generation_id = g.id) \
  AND (g.status != 'published' OR g.id IS NOT ( \
        SELECT p.id FROM snapshot_generations p INDEXED BY snapshot_generations_retention \
        WHERE p.instance_id = g.instance_id AND p.status = 'published' \
          AND p.admission_sequence < a.admission_sequence \
        ORDER BY p.admission_sequence DESC, p.rowid DESC LIMIT 1))";

fn scan_sql() -> String {
    format!(
        "{} ORDER BY g.instance_id, g.admission_sequence LIMIT ?1",
        CANDIDATE_SQL.replace("{index}", "INDEXED BY snapshot_generations_retention")
    )
}

fn recheck_sql() -> String {
    format!("{} AND g.id = ?1", CANDIDATE_SQL.replace("{index}", ""))
}

/// Deletes at most `limit` rows of `table` whose `key` is returned by
/// `select` (bound to `owner`, then the limit as its last parameter).
fn delete_limited(
    tx: &Transaction<'_>,
    table: &str,
    key: &str,
    select: &str,
    owner: &[&dyn rusqlite::ToSql],
    limit: usize,
) -> Result<usize, ApiError> {
    if limit == 0 {
        return Ok(0);
    }
    let limit = limit as i64;
    let mut args: Vec<&dyn rusqlite::ToSql> = owner.to_vec();
    args.push(&limit);
    tx.execute(
        &format!("DELETE FROM {table} WHERE {key} IN ({select})"),
        args.as_slice(),
    )
    .map_err(store_error)
}

fn candidates(conn: &Connection) -> Result<Vec<(String, String)>, ApiError> {
    let mut stmt = conn.prepare(&scan_sql()).map_err(store_error)?;
    stmt.query_map([RETENTION_BATCH_ROWS as i64], |r| {
        Ok((r.get(0)?, r.get(1)?))
    })
    .map_err(store_error)?
    .collect::<Result<Vec<_>, _>>()
    .map_err(store_error)
}

#[derive(Default)]
struct SnapshotBatch {
    generations: u32,
    targets: u32,
    has_more: bool,
}

/// Prunes `candidates` inside `tx`, changing at most `RETENTION_BATCH_ROWS`
/// rows: one for marking a generation discarded, one per release or target
/// deleted, and one for deleting the emptied generation.
fn prune_candidates(
    tx: &Transaction<'_>,
    found: &[(String, String)],
    clock: &dyn Clock,
) -> Result<SnapshotBatch, ApiError> {
    let started = clock.monotonic_now().0;
    let recheck = recheck_sql();
    let mut batch = SnapshotBatch::default();
    let mut rows_left = RETENTION_BATCH_ROWS;
    for (index, (id, _)) in found.iter().enumerate() {
        if rows_left == 0 {
            batch.has_more = true;
            break;
        }
        // Re-check every pin inside the write transaction: a seat may have
        // become unresolved, or a stage published, since the scan.
        let current: Option<String> = tx
            .query_row(&recheck, [id], |r| r.get(1))
            .optional()
            .map_err(store_error)?;
        let Some(status) = current else { continue };
        let instance: String = tx
            .query_row(
                "SELECT instance_id FROM snapshot_generations WHERE id = ?1",
                [id],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        if status != "discarded" {
            tx.execute(
                "UPDATE snapshot_generations SET status = 'discarded' WHERE id = ?1",
                [id],
            )
            .map_err(store_error)?;
            rows_left -= 1;
        }
        let released = delete_limited(
            tx,
            "recovery_baseline_releases",
            "rowid",
            "SELECT rowid FROM recovery_baseline_releases \
             WHERE instance_id = ?1 AND baseline_generation_id = ?2 LIMIT ?3",
            &[&instance, id],
            rows_left.saturating_sub(1),
        )?;
        rows_left -= released;
        let removed = delete_limited(
            tx,
            "snapshot_targets",
            "ordinal",
            "SELECT ordinal FROM snapshot_targets WHERE generation_id = ?1 \
             ORDER BY ordinal LIMIT ?2",
            &[id],
            rows_left.saturating_sub(1),
        )?;
        rows_left -= removed;
        batch.targets += removed as u32;
        let leftover: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM snapshot_targets WHERE generation_id = ?1) \
                 OR EXISTS(SELECT 1 FROM recovery_baseline_releases \
                           WHERE instance_id = ?2 AND baseline_generation_id = ?1)",
                params![id, instance],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        if leftover || rows_left == 0 {
            batch.has_more = true;
            break;
        }
        tx.execute("DELETE FROM snapshot_generations WHERE id = ?1", [id])
            .map_err(store_error)?;
        rows_left -= 1;
        batch.generations += 1;
        if clock.monotonic_now().0.saturating_sub(started) >= RETENTION_QUANTUM_MS {
            batch.has_more = index + 1 < found.len();
            break;
        }
    }
    if found.len() == RETENTION_BATCH_ROWS {
        // The scan was full: more candidates may follow.
        batch.has_more = true;
    }
    Ok(batch)
}

/// A token that changes whenever any committed row change could have become
/// visible: this process's writer generation (its own commits, which SQLite's
/// `data_version` does not report on the same connection) and the writer
/// connection's `data_version` (commits by any other connection).
fn change_mark(store: &SqliteStore, budget: &CallBudget) -> Result<(u64, i64), ApiError> {
    let generation = store.hooks.generation();
    let turn = store.writer(budget)?;
    let version = turn
        .query_row("PRAGMA data_version", [], |r| r.get(0))
        .map_err(store_error)?;
    Ok((generation, version))
}

fn prune_snapshots(store: &SqliteStore, budget: &CallBudget) -> Result<SnapshotBatch, ApiError> {
    // Unchanged tables cannot have grown a candidate: skip the rescan. The
    // mark is read before the scan, so a commit that races it only forces
    // another scan.
    let mark = change_mark(store, budget)?;
    if *store.retention_idle() == Some(mark) {
        return Ok(SnapshotBatch::default());
    }
    let found = store.retention_read(budget, candidates)?;
    if found.is_empty() {
        *store.retention_idle() = Some(mark);
        return Ok(SnapshotBatch::default());
    }
    let mut turn = store.writer(budget)?;
    let tx = turn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(store_error)?;
    let batch = prune_candidates(&tx, &found, store.context.clock())?;
    tx.commit().map_err(store_error)?;
    Ok(batch)
}

fn due_kinds(conn: &Connection, cutoff: i64) -> Result<Vec<&'static str>, ApiError> {
    let mut stmt = conn.prepare(DUE_JOBS_SQL).map_err(store_error)?;
    let mut due = Vec::new();
    for kind in PRUNED_JOB_KINDS {
        let any: bool = stmt
            .query_row(params![kind, cutoff], |r| r.get(0))
            .map_err(store_error)?;
        if any {
            due.push(kind);
        }
    }
    Ok(due)
}

const DUE_JOBS_SQL: &str = "SELECT EXISTS(SELECT 1 FROM work_jobs INDEXED BY work_jobs_retention \
     WHERE kind = ?1 AND status = 'complete' AND (completed_at IS NULL OR completed_at <= ?2))";

const DELETE_JOBS_SQL: &str = "DELETE FROM work_jobs WHERE ordinal IN ( \
     SELECT ordinal FROM work_jobs INDEXED BY work_jobs_retention \
     WHERE kind = ?1 AND status = 'complete' AND (completed_at IS NULL OR completed_at <= ?2) \
     LIMIT ?3)";

fn prune_jobs(store: &SqliteStore, budget: &CallBudget) -> Result<(u32, bool), ApiError> {
    let cutoff = store
        .context
        .clock()
        .utc_now()
        .0
        .saturating_sub(WORK_JOB_RETENTION_MS);
    let due = store.retention_read(budget, |conn| due_kinds(conn, cutoff))?;
    if due.is_empty() {
        return Ok((0, false));
    }
    let mut turn = store.writer(budget)?;
    let tx = turn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(store_error)?;
    let mut rows_left = RETENTION_BATCH_ROWS;
    let mut deleted = 0usize;
    let mut has_more = false;
    for kind in due {
        let n = tx
            .execute(DELETE_JOBS_SQL, params![kind, cutoff, rows_left as i64])
            .map_err(store_error)?;
        deleted += n;
        rows_left -= n;
        if rows_left == 0 {
            has_more = true;
            break;
        }
    }
    tx.commit().map_err(store_error)?;
    Ok((deleted as u32, has_more))
}

/// An unpublished preparation expires after a day without a successful quantum.
pub const PREPARATION_RETENTION_MS: i64 = 24 * 60 * 60 * 1000;
const PREPARATION_CANDIDATES_SQL: &str = "SELECT p.id,p.status,p.prepared_at FROM send_preparations p INDEXED BY send_preparations_retention WHERE p.prepared_at IS NOT NULL AND p.status IN ('building','sealed') AND p.prepared_at<=?1 AND NOT EXISTS(SELECT 1 FROM send_manifests WHERE preparation_id=p.id) ORDER BY p.prepared_at,p.id LIMIT ?2";

type PreparationCandidate = (String, String, i64);

fn preparation_candidates(
    conn: &Connection,
    cutoff: i64,
) -> Result<Vec<PreparationCandidate>, ApiError> {
    let mut statement = conn
        .prepare(PREPARATION_CANDIDATES_SQL)
        .map_err(store_error)?;
    statement
        .query_map(params![cutoff, (RETENTION_BATCH_ROWS / 2) as i64], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .map_err(store_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(store_error)
}

fn discard_expired_preparations(
    tx: &Transaction<'_>,
    found: &[PreparationCandidate],
    clock: &dyn Clock,
    budget: &CallBudget,
) -> Result<(u32, bool), ApiError> {
    let started = clock.monotonic_now().0;
    let mut discarded = 0;
    let mut more = found.len() == RETENTION_BATCH_ROWS / 2;
    for (id, status, progress) in found {
        if budget.cancellation.is_cancelled() {
            return Err(super::connection::api_error(
                crate::protocol::results::ErrorCode::Cancelled,
                "retention cancelled",
            ));
        }
        if budget.deadline_passed(clock) {
            return Err(super::connection::api_error(
                crate::protocol::results::ErrorCode::DeadlineExceeded,
                "retention deadline exceeded",
            ));
        }
        if clock.monotonic_now().0.saturating_sub(started) >= RETENTION_QUANTUM_MS {
            more = true;
            break;
        }
        // The ID fences the generation. Status and progress fence publication,
        // rebuilding and a successful quantum after the read-only scan.
        let eligible: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM send_preparations p WHERE id=?1 AND status=?2 AND status IN ('building','sealed') AND prepared_at=?3 AND prepared_at<=?4 AND NOT EXISTS(SELECT 1 FROM send_manifests WHERE preparation_id=p.id))",params![id,status,progress,clock.utc_now().0.saturating_sub(PREPARATION_RETENTION_MS)],|r|r.get(0)).map_err(store_error)?;
        if eligible {
            super::messages::discard_preparation(tx, id)?;
            discarded += 1;
        }
    }
    Ok((discarded, more))
}

fn prune_preparations(store: &SqliteStore, budget: &CallBudget) -> Result<(u32, bool), ApiError> {
    let cutoff = store
        .context
        .clock()
        .utc_now()
        .0
        .saturating_sub(PREPARATION_RETENTION_MS);
    let found = store.retention_read(budget, |conn| preparation_candidates(conn, cutoff))?;
    if found.is_empty() {
        return Ok((0, false));
    }
    let mut turn = store.writer(budget)?;
    let tx = turn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(store_error)?;
    // Each candidate changes at most two rows: header + cleanup enqueue.
    let batch = discard_expired_preparations(&tx, &found, store.context.clock(), budget)?;
    if batch.0 == 0 {
        tx.rollback().map_err(store_error)?;
    } else {
        tx.commit().map_err(store_error)?;
    }
    Ok(batch)
}

/// Snapshot, work-job and unpublished-preparation transactions, each at most
/// [`RETENTION_BATCH_ROWS`] rows; no write transaction opens when no row
/// qualifies.
pub fn prune_once(store: &SqliteStore, budget: &CallBudget) -> Result<PruneProgress, ApiError> {
    store.live_budget(budget)?;
    let snapshots = prune_snapshots(store, budget)?;
    store.live_budget(budget)?;
    let (jobs, jobs_more) = prune_jobs(store, budget)?;
    store.live_budget(budget)?;
    let (preparations, preparations_more) = prune_preparations(store, budget)?;
    Ok(PruneProgress {
        preparations,
        generations: snapshots.generations,
        targets: snapshots.targets,
        jobs,
        has_more: snapshots.has_more || jobs_more || preparations_more,
    })
}

impl SqliteStore {
    /// Runs one retention pass (see [`prune_once`]).
    pub fn prune_retention(&self, budget: &CallBudget) -> Result<PruneProgress, ApiError> {
        prune_once(self, budget)
    }

    fn retention_idle(&self) -> std::sync::MutexGuard<'_, Option<(u64, i64)>> {
        self.retention_idle
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// Runs `read` on a fresh query connection, under the test cost counter
    /// when one is installed.
    fn retention_read<T>(
        &self,
        budget: &CallBudget,
        read: impl FnOnce(&Connection) -> Result<T, ApiError>,
    ) -> Result<T, ApiError> {
        let conn = self.context.open_query(budget.clone())?;
        #[cfg(any(test, feature = "test-support"))]
        if let Some(counter) = self.cost_probe() {
            return crate::test_support::isolation::count_vm_units(&conn, &counter, || read(&conn));
        }
        read(&conn)
    }
}

#[cfg(test)]
#[path = "../../tests/store/retention.rs"]
mod tests;
