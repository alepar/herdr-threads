//! IncarnationStore: fork repair state and the first repair decision that
//! turns proofs into rows (incarnations design sections 2, 5 and 6).
//!
//! Owns: `record_fork` (store the missing proof entries, insert the evidence
//! row, freeze lower-only), `owner_rotate`/`restore_rotate` (the clone gate,
//! the in-transaction re-check, and rotation before freeze so I1 holds),
//! `sync_clone_evidence` with `record_clone_evidence`, `clone_suspect` and
//! `clear_clone_evidence` (`own_unknown_newer` recording in ht-5fr.6.12),
//! `restore_successor`, `next_incarnation`, `upsert_branch_tip`,
//! `branch_progress` (chain checks, skipping common entries, the exponential
//! back-search, lowering the freeze to the first divergence), `reissue_pass`
//! (candidate order, branch readiness, `waiting` counts, idempotence through
//! `sync_reissues`), `adopt_declared_freeze`, `status` (including
//! `own_unknown_newer`), and invariants I2 and I3. Landed so far:
//! `record_fork`, `next_incarnation`, `reissue_pass`, `owner_rotate`,
//! `restore_rotate`, `restore_successor`, the clone gate, `upsert_branch_tip`,
//! `due_branches` and `branch_progress`. It is the only owner of `sync_forks`,
//! `sync_branches`, `sync_reissues` and `sync_clone_evidence`, and the single
//! place where repair decisions become rows.
//!
//! Hides: the four tables, their triggers and the candidate query's
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

use rusqlite::Connection;

use crate::node::NodeUuid;
use crate::protocol::results::{ApiError, ErrorCode};
use crate::protocol::time::UtcMillis;
use crate::store::connection::DecisionInstant;
use crate::store::connection::{StoreContext, api_error, store_error};
use crate::store::journal::{self, ReissueSource};
use crate::sync::coordinator::{CloneKind, CloneReport};
use crate::sync::envelope::{Envelope, GENESIS_KIND};
use crate::sync::facts::{GenesisPayload, Predecessor};
use crate::sync::incarnations::{AheadClass, Proof, classify_ahead_of_self};
use crate::sync::{BatchLimits, LogKey};

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
/// zero limit cannot stall the job. A clone suspect (`clone_suspect`) gets
/// `Held{clone}` and nothing is written.
pub fn reissue_pass(
    tx: &Transaction<'_>,
    ctx: &StoreContext,
    decision: DecisionInstant,
    limits: ReissueLimits,
) -> Result<Repair<ReissueProgress>, ApiError> {
    if clone_suspect(tx)? {
        return Ok(Repair::Held { clone: true });
    }
    reissue_pass_ungated(tx, ctx, decision, limits).map(Repair::Done)
}

fn reissue_pass_ungated(
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

// ---- Clone gate (ht-5fr.6.7) --------------------------------------------------

/// What a gated repair did: the work, or "held by the clone gate" with nothing
/// written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repair<T> {
    Done(T),
    /// This node holds uncleared clone evidence about itself (`clone`), so no
    /// automatic identity action runs until `clear_clone_evidence`.
    Held {
        clone: bool,
    },
}

impl<T> Repair<T> {
    /// The work, if the gate let it run.
    pub fn done(self) -> Option<T> {
        match self {
            Self::Done(work) => Some(work),
            Self::Held { .. } => None,
        }
    }
}

/// The two clone-evidence kinds that name a log rather than a report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogEvidence {
    /// An own log that is not active, not on the restore chain and newer.
    OwnUnknownNewer,
    /// Two own geneses name one predecessor; the log is that predecessor.
    LineageBranch,
}

impl LogEvidence {
    fn as_str(self) -> &'static str {
        match self {
            Self::OwnUnknownNewer => "own_unknown_newer",
            Self::LineageBranch => "lineage_branch",
        }
    }
}

fn clone_kind_str(kind: CloneKind) -> &'static str {
    match kind {
        CloneKind::Handshake => "handshake",
        CloneKind::PeerClose => "peer_close",
        CloneKind::ThirdParty => "third_party",
    }
}

/// Whether this node holds an uncleared clone-evidence row about itself.
pub fn clone_suspect(conn: &Connection) -> Result<bool, ApiError> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sync_clone_evidence e JOIN local_node n ON n.id=e.subject \
         WHERE e.cleared_at IS NULL)",
        [],
        |r| r.get(0),
    )
    .map_err(store_error)
}

/// Records a clone report from the peer link. A repeat report of an open row
/// (same subject and kind) only advances `last_seen_at`; returns whether a new
/// row was inserted.
pub fn record_clone_evidence(
    tx: &Transaction<'_>,
    report: &CloneReport,
    now: UtcMillis,
) -> Result<bool, ApiError> {
    let detail = if report.boots.is_empty() {
        String::new()
    } else {
        format!(
            "boots={}",
            report
                .boots
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(",")
        )
    };
    upsert_evidence(
        tx,
        report.subject,
        clone_kind_str(report.kind),
        None,
        &detail,
        Some(report.source),
        now,
    )
}

/// Records `own_unknown_newer` or `lineage_branch` evidence about this node's
/// `log` (`detail` is the boots, source node or genesis HLC, at most 512
/// bytes). A repeat of an open row only advances `last_seen_at`.
pub fn record_log_clone_evidence(
    tx: &Transaction<'_>,
    kind: LogEvidence,
    log: &LogKey,
    detail: &str,
    now: UtcMillis,
) -> Result<bool, ApiError> {
    let log_ix: i64 = tx
        .query_row(
            "SELECT log_ix FROM sync_logs WHERE node_uuid=?1 AND incarnation=?2",
            params![log.node.to_string(), log.incarnation],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?
        .ok_or_else(|| api_error(ErrorCode::NotFound, "unknown log"))?;
    upsert_evidence(tx, log.node, kind.as_str(), Some(log_ix), detail, None, now)
}

fn upsert_evidence(
    tx: &Transaction<'_>,
    subject: NodeUuid,
    kind: &str,
    log_ix: Option<i64>,
    detail: &str,
    source: Option<NodeUuid>,
    now: UtcMillis,
) -> Result<bool, ApiError> {
    let subject = subject.to_string();
    let advanced = tx
        .execute(
            "UPDATE sync_clone_evidence SET last_seen_at=MAX(last_seen_at,?4) \
             WHERE subject=?1 AND kind=?2 AND log_ix IS ?3 AND cleared_at IS NULL",
            params![subject, kind, log_ix, now.0],
        )
        .map_err(store_error)?;
    if advanced > 0 {
        return Ok(false);
    }
    let detail: String = detail.chars().take(512).collect();
    tx.execute(
        "INSERT INTO sync_clone_evidence(subject,kind,log_ix,detail,source_node,detected_at,last_seen_at) \
         VALUES (?1,?2,?3,?4,?5,?6,?6)",
        params![
            subject,
            kind,
            log_ix,
            detail,
            source.map(|node| node.to_string()),
            now.0
        ],
    )
    .map_err(store_error)?;
    Ok(true)
}

/// Clears every open clone-evidence row, recording `operator`
/// (`operator:local-user:<uid>`, A3) as the one-time clearer, and returns how
/// many rows it cleared. Any other provenance is `InvalidRequest`.
pub fn clear_clone_evidence(
    tx: &Transaction<'_>,
    operator: &str,
    now: UtcMillis,
) -> Result<u32, ApiError> {
    if !operator.starts_with("operator:local-user:") || operator.len() > 128 {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "clone evidence is cleared with operator:local-user:<uid> provenance",
        ));
    }
    let cleared = tx
        .execute(
            "UPDATE sync_clone_evidence SET cleared_at=?1, cleared_by=?2 WHERE cleared_at IS NULL",
            params![now.0, operator],
        )
        .map_err(store_error)?;
    Ok(u32::try_from(cleared).unwrap_or(u32::MAX))
}

// ---- Rotation (ht-5fr.6.7) ----------------------------------------------------

/// What [`owner_rotate`] or [`restore_rotate`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rotation {
    /// A fork was recorded. `rotated` is the new active log when the forked
    /// log was the active one; `branch_ix` is the conflicting branch's
    /// `sync_branches` row.
    Forked {
        rotated: Option<LogKey>,
        fork: ForkRecord,
        branch_ix: i64,
    },
    /// A restore: `new_log` is active now and names `predecessor`; nothing is
    /// frozen.
    Restored {
        new_log: LogKey,
        predecessor: LogKey,
    },
    /// The re-check found nothing left to repair.
    Stale,
}

/// The active self log, as the rotations read it.
struct ActiveSelf {
    log: LogKey,
    pubkey: [u8; 32],
    contiguous: u64,
    head_hash: [u8; 32],
}

fn blob32(blob: Vec<u8>, what: &str) -> Result<[u8; 32], ApiError> {
    blob.try_into().map_err(|_| {
        api_error(
            ErrorCode::StoreCorrupt,
            format!("stored {what} is not 32 bytes"),
        )
    })
}

fn active_self(conn: &Connection) -> Result<ActiveSelf, ApiError> {
    let (node, incarnation, pubkey, contiguous, head_hash) = conn
        .query_row(
            "SELECT l.node_uuid,l.incarnation,l.pubkey,l.contiguous_counter,l.head_hash \
             FROM local_node n JOIN sync_logs l ON l.log_ix=n.active_log_ix",
            [],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, u32>(1)?,
                    r.get::<_, Vec<u8>>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, Vec<u8>>(4)?,
                ))
            },
        )
        .optional()
        .map_err(store_error)?
        .ok_or_else(|| api_error(ErrorCode::Unsupported, "no active self log"))?;
    Ok(ActiveSelf {
        log: LogKey {
            node: node
                .parse()
                .map_err(|_| api_error(ErrorCode::StoreCorrupt, "malformed own node uuid"))?,
            incarnation,
        },
        pubkey: blob32(pubkey, "pubkey")?,
        contiguous: u64::try_from(contiguous).unwrap_or(0),
        head_hash: blob32(head_hash, "head hash")?,
    })
}

/// This store's entry of `log` at `counter`, if it holds one.
fn entry_at(conn: &Connection, log: &LogKey, counter: u64) -> Option<Envelope> {
    journal::serve(
        conn,
        log,
        counter,
        BatchLimits {
            entries: 1,
            bytes: 1 << 20,
        },
    )
    .ok()
    .and_then(|entries| entries.into_iter().next())
    .filter(|entry| entry.counter == counter)
}

fn own_host_label(tx: &Transaction<'_>, ctx: &StoreContext) -> Result<String, ApiError> {
    Ok(
        crate::store::node_directory::get(tx, ctx.sync()?.identity.uuid())
            .map_err(store_error)?
            .and_then(|row| row.host_label)
            .unwrap_or_default(),
    )
}

/// Rotates away from a forked own log (or records a fork of a non-active own
/// one), in the caller's transaction. `source` is the node the proof came
/// from, `None` for a local detection.
///
/// A clone suspect gets `Held{clone}` and nothing is written. The re-check
/// reclassifies the proof inside the transaction: it must still be two
/// validly signed entries at one counter, with `held` the entry this store
/// holds; otherwise the result is `Stale`. If `proof.log` is the active self
/// log, a new incarnation is begun first (its genesis declares
/// `frozen_at = x - 1`, so I1 holds), then [`record_fork`] freezes the old log
/// and [`upsert_branch_tip`] queues the conflicting branch for a pull. For a
/// non-active own log the same runs without rotating.
pub fn owner_rotate(
    tx: &Transaction<'_>,
    ctx: &StoreContext,
    decision: DecisionInstant,
    proof: &Proof,
    source: Option<NodeUuid>,
) -> Result<Repair<Rotation>, ApiError> {
    if clone_suspect(tx)? {
        return Ok(Repair::Held { clone: true });
    }
    let active = active_self(tx)?;
    let pubkey: Option<(bool, Vec<u8>)> = tx
        .query_row(
            "SELECT is_self,pubkey FROM sync_logs WHERE node_uuid=?1 AND incarnation=?2",
            params![proof.log.node.to_string(), proof.log.incarnation],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(store_error)?;
    let Some((own, pubkey)) = pubkey else {
        return Err(api_error(ErrorCode::NotFound, "unknown log"));
    };
    if !own {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "owner rotation names a log of another node",
        ));
    }
    let pubkey = blob32(pubkey, "pubkey")?;
    let held_here =
        journal::stored_hash(tx, &proof.log, proof.held.counter)? == Some(proof.held.hash);
    if !held_here {
        return Ok(Repair::Done(Rotation::Stale));
    }
    let crate::sync::incarnations::Classified::Proof(proof) =
        crate::sync::incarnations::classify_pair(&pubkey, &proof.held, &proof.conflicting)
    else {
        return Ok(Repair::Done(Rotation::Stale));
    };
    rotate_for_fork(tx, ctx, decision, &active, &proof, source).map(Repair::Done)
}

fn rotate_for_fork(
    tx: &Transaction<'_>,
    ctx: &StoreContext,
    decision: DecisionInstant,
    active: &ActiveSelf,
    proof: &Proof,
    source: Option<NodeUuid>,
) -> Result<Rotation, ApiError> {
    let bound = proof
        .x
        .checked_sub(1)
        .ok_or_else(|| api_error(ErrorCode::InvalidRequest, "fork proof counter is zero"))?;
    let rotated = if proof.log == active.log {
        let incarnation = next_incarnation(tx, ctx)?;
        let host_label = own_host_label(tx, ctx)?;
        journal::begin_incarnation(
            tx,
            ctx,
            decision,
            incarnation,
            Some(Predecessor {
                incarnation: proof.log.incarnation,
                frozen_at: Some(bound),
            }),
            &host_label,
        )?;
        Some(LogKey {
            node: proof.log.node,
            incarnation,
        })
    } else {
        None
    };
    let fork = record_fork(tx, proof, source)?;
    let branch_ix = upsert_branch_tip(
        tx,
        &proof.log,
        proof.conflicting.counter,
        proof.conflicting.hash,
        source,
    )?;
    Ok(Rotation::Forked {
        rotated,
        fork,
        branch_ix,
    })
}

/// Rotates away from a restored active log, in the caller's transaction.
/// `entry` is the peer's entry of the active log that triggered the restore
/// (the one at or beyond this store's head).
///
/// A clone suspect gets `Held{clone}`. The re-check reclassifies `entry`
/// against the head read inside the transaction: still a restore (it extends
/// the head) begins a new incarnation whose genesis names
/// [`restore_successor`]'s log when there is one, else the active log, with
/// `frozen_at = None` (no old log is frozen); a restore this node has since
/// authored past has become a fork and takes the [`owner_rotate`] path; an
/// entry that proves nothing is `Stale`.
pub fn restore_rotate(
    tx: &Transaction<'_>,
    ctx: &StoreContext,
    decision: DecisionInstant,
    entry: &Envelope,
    source: Option<NodeUuid>,
) -> Result<Repair<Rotation>, ApiError> {
    if clone_suspect(tx)? {
        return Ok(Repair::Held { clone: true });
    }
    let active = active_self(tx)?;
    if entry.log != active.log {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "restore rotation names a log other than the active self log",
        ));
    }
    let class = classify_ahead_of_self(
        (active.contiguous, active.head_hash),
        |counter| entry_at(tx, &active.log, counter),
        &active.pubkey,
        entry,
    );
    match class {
        AheadClass::Corrupt(err) => Err(api_error(
            ErrorCode::InvalidRequest,
            format!("restore entry fails verification: {err}"),
        )),
        AheadClass::NotFork => Ok(Repair::Done(Rotation::Stale)),
        AheadClass::Fork(proof) => {
            rotate_for_fork(tx, ctx, decision, &active, &proof, source).map(Repair::Done)
        }
        AheadClass::Restore => {
            let predecessor = restore_successor(tx, ctx)?.unwrap_or(active.log);
            let incarnation = next_incarnation(tx, ctx)?;
            let host_label = own_host_label(tx, ctx)?;
            journal::begin_incarnation(
                tx,
                ctx,
                decision,
                incarnation,
                Some(Predecessor {
                    incarnation: predecessor.incarnation,
                    frozen_at: None,
                }),
                &host_label,
            )?;
            Ok(Repair::Done(Rotation::Restored {
                new_log: LogKey {
                    node: active.log.node,
                    incarnation,
                },
                predecessor,
            }))
        }
    }
}

/// One own `node.genesis` entry, as the lineage rules read it.
struct OwnGenesis {
    incarnation: u32,
    hlc: i64,
    predecessor: Option<u32>,
}

fn own_geneses(conn: &Connection) -> Result<Vec<OwnGenesis>, ApiError> {
    let mut statement = conn
        .prepare(
            "SELECT l.incarnation,j.hlc,j.payload FROM sync_logs l \
             JOIN journal j ON j.log_ix=l.log_ix AND j.counter=1 \
             WHERE l.is_self=1 AND j.kind=?1",
        )
        .map_err(store_error)?;
    let rows = statement
        .query_map([GENESIS_KIND], |r| {
            Ok((
                r.get::<_, u32>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, Vec<u8>>(2)?,
            ))
        })
        .map_err(store_error)?;
    let mut out = Vec::new();
    for row in rows {
        let (incarnation, hlc, payload) = row.map_err(store_error)?;
        // A genesis whose payload does not parse names no predecessor.
        let Ok(payload) = serde_json::from_slice::<GenesisPayload>(&payload) else {
            continue;
        };
        out.push(OwnGenesis {
            incarnation,
            hlc,
            predecessor: payload.predecessor.map(|p| p.incarnation),
        });
    }
    Ok(out)
}

/// The newest own log X that succeeds the active log P after a restore from
/// a backup older than a rotation (incarnations design 6, "Restore
/// successor"), else `None`. All four rules must hold:
/// 1. the own `node.genesis` predecessor chain leads from X back to P, or
///    (nothing names P) a single bootstrap genesis with no predecessor and a
///    newer HLC than P's genesis follows P;
/// 2. no log on that chain, P included, is named by two own geneses;
/// 3. every `journal` entry of P has an HLC below the genesis HLC of the
///    chain's first log after P;
/// 4. this process has not rotated since `open`
///    ([`SyncState::rotated_since_start`](crate::store::connection::SyncState::rotated_since_start)).
///
/// More than one candidate bootstrap genesis is ambiguous and gives `None`.
pub fn restore_successor(
    conn: &Connection,
    ctx: &StoreContext,
) -> Result<Option<LogKey>, ApiError> {
    if ctx.sync()?.rotated_since_start() {
        return Ok(None);
    }
    let active = active_self(conn)?;
    let geneses = own_geneses(conn)?;
    let Some(base) = geneses
        .iter()
        .find(|g| g.incarnation == active.log.incarnation)
    else {
        return Ok(None);
    };
    let named_by = |incarnation: u32| {
        geneses
            .iter()
            .filter(move |g| g.predecessor == Some(incarnation))
            .collect::<Vec<_>>()
    };
    let first = match named_by(active.log.incarnation).as_slice() {
        [one] => *one,
        [] => {
            let mut bootstraps = geneses.iter().filter(|g| {
                g.predecessor.is_none()
                    && g.incarnation != active.log.incarnation
                    && g.hlc > base.hlc
            });
            match (bootstraps.next(), bootstraps.next()) {
                (Some(one), None) => one,
                _ => return Ok(None),
            }
        }
        _ => return Ok(None),
    };
    let mut chain = vec![first.incarnation];
    let mut tail = first;
    loop {
        match named_by(tail.incarnation).as_slice() {
            [] => break,
            [next]
                if !chain.contains(&next.incarnation)
                    && next.incarnation != active.log.incarnation =>
            {
                chain.push(next.incarnation);
                tail = next;
            }
            _ => return Ok(None),
        }
    }
    let newest_of_p: Option<i64> = conn
        .query_row(
            "SELECT MAX(j.hlc) FROM journal j JOIN sync_logs l ON l.log_ix=j.log_ix \
             WHERE l.node_uuid=?1 AND l.incarnation=?2",
            params![active.log.node.to_string(), active.log.incarnation],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if newest_of_p.is_some_and(|hlc| hlc >= first.hlc) {
        return Ok(None);
    }
    Ok(Some(LogKey {
        node: active.log.node,
        incarnation: tail.incarnation,
    }))
}

// ---- Branch pulls (ht-5fr.6.7) ------------------------------------------------

/// First `search_span` once a back-search starts (the span doubles from here).
const MIN_SEARCH_SPAN: u64 = 256;

/// The `sync_branches` row for a branch tip of a frozen own log, created
/// `pulling` with `next_from = frozen_at + 1`. A tip already known keeps its
/// progress, except that an `unavailable` row returns to `pulling`. Returns the
/// row's `branch_ix`. `_source` is the advertiser; the sink keeps sources in
/// memory, the table has no column for it. Not gated by the clone gate.
pub fn upsert_branch_tip(
    tx: &Transaction<'_>,
    log: &LogKey,
    counter: u64,
    hash: [u8; 32],
    _source: Option<NodeUuid>,
) -> Result<i64, ApiError> {
    let (log_ix, frozen_at): (i64, Option<i64>) = tx
        .query_row(
            "SELECT log_ix,frozen_at_counter FROM sync_logs WHERE node_uuid=?1 AND incarnation=?2",
            params![log.node.to_string(), log.incarnation],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(store_error)?
        .ok_or_else(|| api_error(ErrorCode::NotFound, "unknown log"))?;
    let Some(frozen_at) = frozen_at else {
        return Err(api_error(
            ErrorCode::Conflict,
            "branch tip names a log that is not frozen",
        ));
    };
    let counter = i64::try_from(counter)
        .ok()
        .filter(|c| *c >= 1)
        .ok_or_else(|| api_error(ErrorCode::InvalidRequest, "branch tip counter out of range"))?;
    tx.execute(
        "INSERT INTO sync_branches(log_ix,tip_counter,tip_hash,next_from,state,first_seen_at,updated_at) \
         VALUES (?1,?2,?3,?4,'pulling',CAST(strftime('%s','now') AS INTEGER)*1000,CAST(strftime('%s','now') AS INTEGER)*1000) \
         ON CONFLICT(log_ix,tip_hash) DO UPDATE SET \
            state=CASE WHEN state='unavailable' THEN 'pulling' ELSE state END, \
            next_attempt_at=CASE WHEN state='unavailable' THEN NULL ELSE next_attempt_at END, \
            updated_at=CAST(strftime('%s','now') AS INTEGER)*1000",
        params![log_ix, counter, hash.as_slice(), frozen_at + 1],
    )
    .map_err(store_error)?;
    tx.query_row(
        "SELECT branch_ix FROM sync_branches WHERE log_ix=?1 AND tip_hash=?2",
        params![log_ix, hash.as_slice()],
        |r| r.get(0),
    )
    .map_err(store_error)
}

/// A `pulling` branch whose `next_attempt_at` has passed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DueBranch {
    pub branch_ix: i64,
    pub log: LogKey,
    pub tip: (u64, [u8; 32]),
    pub next_from: u64,
}

/// The `pulling` branches that are due at `now` (no `next_attempt_at`, or one
/// at or before `now`), oldest first; a read on a query connection, the one
/// the worker's 1 s timer uses.
pub fn due_branches(conn: &Connection, now: UtcMillis) -> Result<Vec<DueBranch>, ApiError> {
    let mut statement = conn
        .prepare(
            "SELECT b.branch_ix,l.node_uuid,l.incarnation,b.tip_counter,b.tip_hash,b.next_from \
             FROM sync_branches b JOIN sync_logs l ON l.log_ix=b.log_ix \
             WHERE b.state='pulling' AND (b.next_attempt_at IS NULL OR b.next_attempt_at<=?1) \
             ORDER BY b.first_seen_at,b.branch_ix",
        )
        .map_err(store_error)?;
    let rows = statement
        .query_map([now.0], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, u32>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, Vec<u8>>(4)?,
                r.get::<_, i64>(5)?,
            ))
        })
        .map_err(store_error)?;
    let mut out = Vec::new();
    for row in rows {
        let (branch_ix, node, incarnation, tip_counter, tip_hash, next_from) =
            row.map_err(store_error)?;
        out.push(DueBranch {
            branch_ix,
            log: LogKey {
                node: node
                    .parse()
                    .map_err(|_| api_error(ErrorCode::StoreCorrupt, "malformed node uuid"))?,
                incarnation,
            },
            tip: (
                u64::try_from(tip_counter).unwrap_or(0),
                blob32(tip_hash, "tip hash")?,
            ),
            next_from: u64::try_from(next_from).unwrap_or(1),
        });
    }
    Ok(out)
}

/// What one [`branch_progress`] reply did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BranchVerdict {
    /// Entries were verified and admitted (or skipped as common).
    Progress {
        /// Entries newly stored in `journal_forked`.
        stored: usize,
        /// Entries that were already this store's own chain.
        skipped: usize,
        /// The branch is `complete`.
        complete: bool,
        /// The next counter to request.
        next_from: u64,
        /// The log's freeze counter afterwards.
        frozen_at: u64,
    },
    /// The first entry did not chain on a known predecessor: the divergence
    /// lies below `next_from` and the back-search moved earlier.
    NeedsEarlier { next_from: u64, search_span: u64 },
    /// A bad signature, a broken chain or a reply that does not fit the
    /// branch; the caller penalizes the source. Nothing was admitted.
    Corrupt { reason: String },
}

struct BranchRow {
    log_ix: i64,
    log: LogKey,
    pubkey: [u8; 32],
    frozen_at: u64,
    tip_counter: u64,
    tip_hash: [u8; 32],
    next_from: u64,
    search_span: u64,
}

fn load_branch(tx: &Transaction<'_>, branch_ix: i64) -> Result<BranchRow, ApiError> {
    let row = tx
        .query_row(
            "SELECT b.log_ix,l.node_uuid,l.incarnation,l.pubkey,l.frozen_at_counter,\
                    b.tip_counter,b.tip_hash,b.next_from,b.search_span \
             FROM sync_branches b JOIN sync_logs l ON l.log_ix=b.log_ix WHERE b.branch_ix=?1",
            [branch_ix],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, u32>(2)?,
                    r.get::<_, Vec<u8>>(3)?,
                    r.get::<_, Option<i64>>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, Vec<u8>>(6)?,
                    r.get::<_, i64>(7)?,
                    r.get::<_, i64>(8)?,
                ))
            },
        )
        .optional()
        .map_err(store_error)?
        .ok_or_else(|| api_error(ErrorCode::NotFound, "unknown branch"))?;
    let unsigned = |v: i64| u64::try_from(v).unwrap_or(0);
    Ok(BranchRow {
        log_ix: row.0,
        log: LogKey {
            node: row
                .1
                .parse()
                .map_err(|_| api_error(ErrorCode::StoreCorrupt, "malformed node uuid"))?,
            incarnation: row.2,
        },
        pubkey: blob32(row.3, "pubkey")?,
        frozen_at: unsigned(row.4.ok_or_else(|| {
            api_error(
                ErrorCode::StoreCorrupt,
                "branch of a log that is not frozen",
            )
        })?),
        tip_counter: unsigned(row.5),
        tip_hash: blob32(row.6, "tip hash")?,
        next_from: unsigned(row.7).max(1),
        search_span: unsigned(row.8),
    })
}

/// Records a failed reply on the branch row (attempt counted, error kept).
fn note_failure(tx: &Transaction<'_>, branch_ix: i64, error: &str) -> Result<(), ApiError> {
    let error: String = error.chars().take(512).collect();
    tx.execute(
        "UPDATE sync_branches SET attempts=attempts+1, last_error=?2, \
         updated_at=CAST(strftime('%s','now') AS INTEGER)*1000 WHERE branch_ix=?1",
        params![branch_ix, error],
    )
    .map_err(store_error)?;
    Ok(())
}

/// Checks one reply against the branch: signatures, contiguity from
/// `next_from`, bounds by the tip. `Err(reason)` is a `Corrupt` verdict.
fn verify_reply(branch: &BranchRow, entries: &[Envelope]) -> Result<(), String> {
    let mut expected = branch.next_from;
    let mut previous: Option<&Envelope> = None;
    for entry in entries {
        if entry.log != branch.log {
            return Err("entry names another log".into());
        }
        entry
            .verify(&branch.pubkey)
            .map_err(|e| format!("bad entry {}: {e}", entry.counter))?;
        if entry.counter != expected {
            return Err(format!(
                "entry {} where {expected} was requested",
                entry.counter
            ));
        }
        if let Some(previous) = previous
            && entry.prev_hash != previous.hash
        {
            return Err(format!("broken chain at {}", entry.counter));
        }
        if entry.counter > branch.tip_counter {
            return Err(format!("entry {} is beyond the tip", entry.counter));
        }
        if entry.counter == branch.tip_counter && entry.hash != branch.tip_hash {
            return Err("branch does not end at the advertised tip".into());
        }
        expected += 1;
        previous = Some(entry);
    }
    Ok(())
}

/// Whether the first entry chains on a predecessor this store knows: the zero
/// hash at counter 1, or a `journal` or `journal_forked` row at `counter - 1`.
fn chains_on_known(
    tx: &Transaction<'_>,
    branch: &BranchRow,
    first: &Envelope,
) -> Result<bool, ApiError> {
    if first.counter == 1 {
        return Ok(first.prev_hash == [0u8; 32]);
    }
    let counter = i64::try_from(first.counter - 1).unwrap_or(i64::MAX);
    tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM journal WHERE log_ix=?1 AND counter=?2 AND hash=?3) \
             OR EXISTS(SELECT 1 FROM journal_forked WHERE log_ix=?1 AND counter=?2 AND hash=?3)",
        params![branch.log_ix, counter, first.prev_hash.as_slice()],
        |r| r.get(0),
    )
    .map_err(store_error)
}

/// Admits one `branch_entries` reply for branch `branch_ix` of a frozen own
/// log, in the caller's transaction. `more` is the reply's own flag.
///
/// Every entry is verified with the log's public key and must continue the
/// previous one; the first must chain on a known predecessor, else the
/// divergence lies below `next_from` and the back-search moves earlier
/// (`search_span = max(256, 2 x span)`, `NeedsEarlier`). Entries equal to this
/// store's own chain are skipped; the others go to `journal_forked` with
/// `source` as `source_node`, and the first off-chain counter `d + 1` lowers
/// the log's freeze to `min(frozen_at, d)` before any of them is stored, so
/// I2 holds. `next_from` moves past the last entry; the row is `complete` when
/// that entry is the tip, or when `more` is false and it is short of the tip
/// (the source held less). A `Corrupt` reply writes nothing but the
/// `attempts` and `last_error` record. Not gated by the clone gate.
pub fn branch_progress(
    tx: &Transaction<'_>,
    branch_ix: i64,
    source: NodeUuid,
    entries: &[Envelope],
    more: bool,
) -> Result<BranchVerdict, ApiError> {
    let branch = load_branch(tx, branch_ix)?;
    if let Err(reason) = verify_reply(&branch, entries) {
        note_failure(tx, branch_ix, &reason)?;
        return Ok(BranchVerdict::Corrupt { reason });
    }
    let Some(first) = entries.first() else {
        let complete = !more;
        tx.execute(
            "UPDATE sync_branches SET attempts=attempts+1, last_error=NULL, \
             state=CASE WHEN ?2 THEN 'complete' ELSE state END, \
             updated_at=CAST(strftime('%s','now') AS INTEGER)*1000 WHERE branch_ix=?1",
            params![branch_ix, complete],
        )
        .map_err(store_error)?;
        return Ok(BranchVerdict::Progress {
            stored: 0,
            skipped: 0,
            complete,
            next_from: branch.next_from,
            frozen_at: branch.frozen_at,
        });
    };
    if !chains_on_known(tx, &branch, first)? {
        if branch.next_from <= 1 {
            let reason = "first entry does not chain at counter 1".to_owned();
            note_failure(tx, branch_ix, &reason)?;
            return Ok(BranchVerdict::Corrupt { reason });
        }
        let search_span = branch.search_span.saturating_mul(2).max(MIN_SEARCH_SPAN);
        let next_from = branch.next_from.saturating_sub(search_span).max(1);
        tx.execute(
            "UPDATE sync_branches SET search_span=?2, next_from=?3, attempts=attempts+1, \
             last_error='first entry does not chain; searching earlier', \
             updated_at=CAST(strftime('%s','now') AS INTEGER)*1000 WHERE branch_ix=?1",
            params![
                branch_ix,
                i64::try_from(search_span).unwrap_or(i64::MAX),
                i64::try_from(next_from).unwrap_or(i64::MAX)
            ],
        )
        .map_err(store_error)?;
        return Ok(BranchVerdict::NeedsEarlier {
            next_from,
            search_span,
        });
    }

    // Split the reply into the shared prefix and the off-chain rest: a hash
    // chain that differs once never rejoins.
    let mut common = 0;
    for entry in entries {
        if journal::stored_hash(tx, &branch.log, entry.counter)? == Some(entry.hash) {
            common += 1;
        } else {
            break;
        }
    }
    let off_chain = &entries[common..];
    let mut frozen_at = branch.frozen_at;
    if let Some(divergent) = off_chain.first() {
        let divergence = divergent.counter - 1;
        if divergence < frozen_at {
            journal::freeze(tx, &branch.log, divergence)?;
            frozen_at = divergence;
        }
    }
    let stored = journal::store_forked(tx, &branch.log, off_chain, Some(source))?;

    let last = entries.last().map_or(branch.next_from - 1, |e| e.counter);
    let complete = last >= branch.tip_counter || !more;
    tx.execute(
        "UPDATE sync_branches SET next_from=?2, search_span=0, attempts=attempts+1, last_error=NULL, \
         state=CASE WHEN ?3 THEN 'complete' ELSE state END, \
         updated_at=CAST(strftime('%s','now') AS INTEGER)*1000 WHERE branch_ix=?1",
        params![
            branch_ix,
            i64::try_from(last + 1).unwrap_or(i64::MAX),
            complete
        ],
    )
    .map_err(store_error)?;
    Ok(BranchVerdict::Progress {
        stored,
        skipped: common,
        complete,
        next_from: last + 1,
        frozen_at,
    })
}

#[cfg(test)]
#[path = "../../tests/store/incarnations.rs"]
mod tests;
