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
//! `record_fork`, `next_incarnation` and `reissue_pass`. It is the only owner of
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
use crate::store::connection::DecisionInstant;
use crate::store::connection::{StoreContext, api_error, store_error};
use crate::store::journal::{self, ReissueSource};
use crate::sync::LogKey;
use crate::sync::envelope::GENESIS_KIND;
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

/// What one [`reissue_pass`] may do before it reports `has_more`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReissueLimits {
    /// Entries re-issued per pass.
    pub entries: usize,
    /// Monotonic milliseconds per pass.
    pub wall_ms: u64,
}

impl Default for ReissueLimits {
    fn default() -> Self {
        Self {
            entries: 64,
            wall_ms: 100,
        }
    }
}

/// The tally of one [`reissue_pass`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReissueProgress {
    /// Entries re-issued this pass, including those whose handler rejected or
    /// parked (the outcome is recorded; the entry is still re-issued).
    pub reissued: usize,
    /// `journal_forked` candidates seen but not yet ready: their predecessor
    /// is neither a `journal` row nor an already re-issued forked row. Only
    /// candidates reached before a limit stopped the pass are counted.
    pub waiting: usize,
    /// A limit stopped the pass while candidates remained: run it again.
    pub has_more: bool,
}

/// One re-issue candidate: an entry of a frozen own log above its freeze, or
/// any `journal_forked` entry of one.
struct Candidate {
    log: LogKey,
    log_ix: i64,
    counter: i64,
    hash: Vec<u8>,
    prev_hash: Vec<u8>,
    forked: bool,
}

/// Every not-yet-re-issued candidate, interleaved: by the log's first sight,
/// then `log_ix`, then counter (so each branch keeps its own order and the
/// branches of one counter are adjacent), the `journal` branch before the
/// forked ones, and a forked branch's rows by arrival. `node.genesis` never
/// qualifies.
const CANDIDATES: &str = "\
SELECT log_ix,node_uuid,incarnation,counter,hash,prev_hash,src FROM (\
 SELECT l.first_seen_at AS seen,l.log_ix AS log_ix,l.node_uuid AS node_uuid,l.incarnation AS incarnation,\
        j.counter AS counter,j.hash AS hash,j.prev_hash AS prev_hash,0 AS src,j.ingest_seq AS seq \
 FROM sync_logs l JOIN journal j ON j.log_ix=l.log_ix \
 WHERE l.is_self=1 AND l.state='frozen' AND j.counter>l.frozen_at_counter AND j.kind<>?1 \
   AND NOT EXISTS(SELECT 1 FROM sync_reissues r WHERE r.source_log_ix=j.log_ix AND r.source_counter=j.counter AND r.source_hash=j.hash) \
 UNION ALL \
 SELECT l.first_seen_at,l.log_ix,l.node_uuid,l.incarnation,f.counter,f.hash,f.prev_hash,1,f.forked_ix \
 FROM sync_logs l JOIN journal_forked f ON f.log_ix=l.log_ix \
 WHERE l.is_self=1 AND l.state='frozen' AND f.kind<>?1 \
   AND NOT EXISTS(SELECT 1 FROM sync_reissues r WHERE r.source_log_ix=f.log_ix AND r.source_counter=f.counter AND r.source_hash=f.hash)\
) ORDER BY seen,log_ix,counter,src,seq";

/// Re-issues the union of every branch tail of the frozen own logs into the
/// active self log, in the caller's transaction, until `limits` stop it.
///
/// Kind-agnostic and idempotent: the work list is the candidates not yet in
/// `sync_reissues`, and each re-issue and its `sync_reissues` row are written
/// together, so a rolled-back quantum leaves neither and a committed one is
/// never repeated (invariant I3: every `sync_reissues` row names an entry on
/// the active log). A handler rejection is recorded by `journal::reissue` and
/// does not stop the pass.
///
/// A forked candidate is ready only when its `prev_hash` names the `journal`
/// row at `counter - 1` or a forked row already re-issued (this pass or an
/// earlier one); otherwise it is skipped and counted in `waiting`, and the
/// other branches go on. At least one candidate is re-issued per call, so a
/// zero limit cannot stall the job. The clone gate sits in front of this call
/// (ht-5fr.6.7).
pub fn reissue_pass(
    tx: &Transaction<'_>,
    ctx: &StoreContext,
    decision: DecisionInstant,
    limits: ReissueLimits,
) -> Result<ReissueProgress, ApiError> {
    let candidates = {
        let mut statement = tx.prepare(CANDIDATES).map_err(store_error)?;
        let rows = statement
            .query_map([GENESIS_KIND], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, u32>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, Vec<u8>>(4)?,
                    r.get::<_, Vec<u8>>(5)?,
                    r.get::<_, i64>(6)?,
                ))
            })
            .map_err(store_error)?;
        let mut out = Vec::new();
        for row in rows {
            let (log_ix, node, incarnation, counter, hash, prev_hash, src) =
                row.map_err(store_error)?;
            out.push(Candidate {
                log: LogKey {
                    node: node.parse().map_err(|_| {
                        api_error(ErrorCode::StoreCorrupt, "malformed own node uuid")
                    })?,
                    incarnation,
                },
                log_ix,
                counter,
                hash,
                prev_hash,
                forked: src == 1,
            });
        }
        out
    };

    let started = ctx.clock().monotonic_now();
    let mut progress = ReissueProgress::default();
    let total = candidates.len();
    for (seen, candidate) in candidates.into_iter().enumerate() {
        if candidate.forked && !forked_ready(tx, &candidate)? {
            progress.waiting += 1;
            continue;
        }
        let hash: [u8; 32] = candidate
            .hash
            .as_slice()
            .try_into()
            .map_err(|_| api_error(ErrorCode::StoreCorrupt, "stored hash is not 32 bytes"))?;
        let counter = u64::try_from(candidate.counter)
            .map_err(|_| api_error(ErrorCode::StoreCorrupt, "stored counter"))?;
        let source = journal::serve_branch(
            tx,
            &candidate.log,
            (counter, hash),
            counter,
            crate::sync::BatchLimits {
                entries: 1,
                bytes: 1 << 20,
            },
        )?
        .and_then(|mut entries| entries.pop())
        .ok_or_else(|| api_error(ErrorCode::StoreCorrupt, "re-issue source entry missing"))?;
        let authored = journal::reissue(
            tx,
            ctx,
            decision,
            &source,
            if candidate.forked {
                ReissueSource::Forked
            } else {
                ReissueSource::Journal
            },
        )?;
        tx.execute(
            "INSERT INTO sync_reissues(source_log_ix,source_counter,source_hash,target_ingest_seq,reissued_at) \
             VALUES (?1,?2,?3,?4,?5)",
            params![
                candidate.log_ix,
                candidate.counter,
                candidate.hash,
                authored.ingest_seq,
                decision.utc.0,
            ],
        )
        .map_err(store_error)?;
        progress.reissued += 1;
        let spent = ctx.clock().monotonic_now().0.saturating_sub(started.0);
        if seen + 1 < total && (progress.reissued >= limits.entries || spent >= limits.wall_ms) {
            progress.has_more = true;
            break;
        }
    }
    Ok(progress)
}

/// Whether a forked candidate's predecessor is in place: the `journal` row at
/// `counter - 1` with its `prev_hash`, or a forked row of that hash already
/// re-issued.
fn forked_ready(tx: &Transaction<'_>, candidate: &Candidate) -> Result<bool, ApiError> {
    tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM journal WHERE log_ix=?1 AND counter=?2-1 AND hash=?3) \
             OR EXISTS(SELECT 1 FROM journal_forked f JOIN sync_reissues r \
                       ON r.source_log_ix=f.log_ix AND r.source_counter=f.counter AND r.source_hash=f.hash \
                       WHERE f.log_ix=?1 AND f.counter=?2-1 AND f.hash=?3)",
        params![candidate.log_ix, candidate.counter, candidate.prev_hash],
        |r| r.get(0),
    )
    .map_err(store_error)
}

#[cfg(test)]
#[path = "../../tests/store/incarnations.rs"]
mod tests;
