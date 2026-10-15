//! IncarnationStore: fork repair state and the first repair decision that
//! turns proofs into rows (incarnations design sections 2, 5 and 6).
//!
//! Owns: `record_fork` (store the missing proof entries, insert the evidence
//! row, freeze lower-only), `owner_rotate`/`restore_rotate` (the clone gate,
//! the in-transaction re-check, and rotation before freeze so I1 holds),
//! `sync_clone_evidence` with `record_clone_evidence`, `clone_suspect` and
//! `clear_clone_evidence` (land in ht-5fr.6.7; `own_unknown_newer` recording
//! in ht-5fr.6.12), `next_incarnation`, `upsert_branch_tip`,
//! `branch_progress` (chain checks, skipping common entries, the exponential
//! back-search, lowering the freeze to the first divergence), `reissue_pass`
//! (candidate order, branch readiness, `waiting` counts, idempotence through
//! `sync_reissues`), `adopt_declared_freeze`, `status` (including
//! `own_unknown_newer`), and invariants I2 and I3. Landed so far:
//! `record_fork` and `next_incarnation`. It is the only owner of
//! `sync_forks`, `sync_branches` and `sync_reissues`, and the single place
//! where repair decisions become rows.
//!
//! Hides: the three tables, their triggers and the candidate query's
//! ordering.
//!
//! Must not: write `journal`, `journal_forked` or `sync_logs` itself. It goes
//! through Journal's `store_forked`, `freeze`, `begin_incarnation` and
//! `reissue`. It must not hold network or timing state (sources, timeouts and
//! penalties live in the sink), or interpret a payload (re-issue is
//! kind-agnostic).
//!
//! Where new repair state goes: here. New persisted repair state is a table
//! or column this module owns; callers never write those tables or freeze,
//! rotate or re-issue through Journal directly.
//!
//! Role: stateless functions over persisted state, called only by
//! IncarnationsWorker inside background writer quanta, by the `node.genesis`
//! handler (`adopt_declared_freeze`), by `journal::open` (`next_incarnation`
//! at bootstrap) and by ht-5fr.8's status. Every function is one transaction
//! (the caller's), so a crash between quanta resumes from the rows. A
//! `branch_progress` verdict (`Corrupt`, `NeedsEarlier`, progress) is a
//! value, not an `Err`; `Err` stays reserved for store errors, which roll the
//! quantum back. Depends on Journal, ForkProof and `SyncState.incarnation_rng`.

use rusqlite::{OptionalExtension, Transaction, params};

use crate::node::NodeUuid;
use crate::protocol::results::{ApiError, ErrorCode};
use crate::store::connection::{StoreContext, api_error, store_error};
use crate::store::journal;
use crate::sync::incarnations::Proof;

/// What [`record_fork`] decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ForkRecord {
    /// This proof was not recorded before (its evidence row is new).
    pub new: bool,
    /// The log's freeze counter after the call: `x - 1`, or lower if the log
    /// was already frozen lower.
    pub frozen_at: u64,
}

/// Redraws before `next_incarnation` gives up; a u32 space makes a collision
/// run this long a broken source, not bad luck.
const MAX_INCARNATION_DRAWS: usize = 1024;

/// Turns a fork proof about a held log into rows, in the caller's
/// transaction: freezes the log at `x - 1` (lower-only), stores whichever
/// proof entries `journal` does not already hold in `journal_forked`, and
/// inserts the deduplicated `sync_forks` evidence row. `source` is the node
/// the evidence came from, if known.
///
/// The freeze comes first because it refuses the active self log (invariant
/// I1, `Conflict`): an own-active proof is routed to rotation by the caller
/// (`owner_rotate`, ht-5fr.6.7), and the refusal is returned unchanged with
/// nothing written. An unknown log is `NotFound`.
pub fn record_fork(
    tx: &Transaction<'_>,
    proof: &Proof,
    source: Option<NodeUuid>,
) -> Result<ForkRecord, ApiError> {
    let bound = proof
        .x
        .checked_sub(1)
        .ok_or_else(|| api_error(ErrorCode::InvalidRequest, "fork proof counter is zero"))?;
    journal::freeze(tx, &proof.log, bound)?;

    let log_ix: i64 = tx
        .query_row(
            "SELECT log_ix FROM sync_logs WHERE node_uuid=?1 AND incarnation=?2",
            params![proof.log.node.to_string(), proof.log.incarnation],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    let mut missing = Vec::new();
    for entry in [&proof.held, &proof.conflicting] {
        let held: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM journal WHERE log_ix=?1 AND counter=?2 AND hash=?3)",
                params![
                    log_ix,
                    i64::try_from(entry.counter).unwrap_or(i64::MAX),
                    entry.hash.as_slice()
                ],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        if !held {
            missing.push(entry.clone());
        }
    }
    journal::store_forked(tx, &proof.log, &missing, source)?;

    let counter = |c: u64| i64::try_from(c).unwrap_or(i64::MAX);
    let inserted = tx
        .execute(
            "INSERT OR IGNORE INTO sync_forks(log_ix,origin,proof_counter,held_counter,held_hash,conflicting_counter,conflicting_hash,frozen_at,source_node,detected_at) \
             VALUES (?1,'evidence',?2,?3,?4,?5,?6,?7,?8,CAST(strftime('%s','now') AS INTEGER)*1000)",
            params![
                log_ix,
                counter(proof.x),
                counter(proof.held.counter),
                proof.held.hash.as_slice(),
                counter(proof.conflicting.counter),
                proof.conflicting.hash.as_slice(),
                counter(bound),
                source.map(|node| node.to_string()),
            ],
        )
        .map_err(store_error)?;

    let frozen_at: i64 = tx
        .query_row(
            "SELECT frozen_at_counter FROM sync_logs WHERE log_ix=?1",
            [log_ix],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    Ok(ForkRecord {
        new: inserted == 1,
        frozen_at: u64::try_from(frozen_at).unwrap_or(0),
    })
}

/// A uniformly random incarnation in `1..=u32::MAX` that no `sync_logs` row
/// of this node already uses, drawn from `SyncState.incarnation_rng` (so the
/// test-support `seed_incarnations` fixes the sequence). Needs the journal
/// open (`Unsupported "journal not open"` otherwise).
pub fn next_incarnation(tx: &Transaction<'_>, ctx: &StoreContext) -> Result<u32, ApiError> {
    let state = ctx.sync()?;
    for _ in 0..MAX_INCARNATION_DRAWS {
        let draw = state
            .incarnation_rng
            .lock()
            .map_err(|_| api_error(ErrorCode::StoreCorrupt, "incarnation source poisoned"))?
            .draw_incarnation();
        let used: Option<i64> = tx
            .query_row(
                "SELECT 1 FROM sync_logs WHERE is_self=1 AND incarnation=?1",
                [draw],
                |r| r.get(0),
            )
            .optional()
            .map_err(store_error)?;
        if used.is_none() {
            return Ok(draw);
        }
    }
    Err(api_error(
        ErrorCode::Conflict,
        "no unused incarnation after repeated draws",
    ))
}

#[cfg(test)]
#[path = "../../tests/store/incarnations.rs"]
mod tests;
