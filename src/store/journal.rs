//! The journal: the per-origin hash chain of signed facts, in SQLite.
//!
//! Responsibility
//! - Owns: `sync_logs`, `journal` and `journal_forked` as their single writer.
//!   In this file that is startup ([`open`]: invariants, HLC resume, genesis
//!   bootstrap through [`begin_incarnation`]), local authoring ([`author`],
//!   [`author_above`]: tick, next counter, `prev_hash`, deps, seal, insert,
//!   then hand to ApplyEngine in the same transaction), the applied vector
//!   ([`applied_vector`]) and the commit guard ([`check_self_applied`]).
//!   Batch ingest, serving, the full vector, `store_forked` and `freeze` land
//!   in ht-5fr.3.7.
//! - Hides: the per-log chain rules, and that nothing here reads a payload
//!   beyond handing its bytes to the registry.
//! - Must not: materialize anything (ApplyEngine, `store::apply`), choose
//!   peers (SyncCoordinator) or resolve forks (Incarnations). An authoring
//!   failure aborts the command as an `ApiError` and is never recorded as
//!   `failed`.
//! - Role: callers never write `journal`/`sync_logs` rows or materialize facts
//!   directly; a command authors through [`author`] inside its decide
//!   closure, and the two base writers refuse to commit an unapplied own
//!   entry. New journal tables or checks go in this module.
//!
//! Journal and `store::apply` call each other on purpose: Journal appends a
//! row, ApplyEngine applies it in the same transaction.

use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

pub use crate::sync::facts::Predecessor;

use crate::node::{NodeIdentity, NodeUuid};
use crate::protocol::results::{ApiError, ErrorCode};
use crate::store::apply;
use crate::store::connection::{
    DecisionInstant, INGEST_HEADROOM_BYTES, StoreContext, SyncState, api_error, store_error,
};
use crate::sync::envelope::{Deps, GENESIS_KIND, PAYLOAD_MAX, UnsignedEnvelope, hex_encode};
use crate::sync::facts::{FactKind, FactRegistry, GenesisPayload, NodeGenesis};
use crate::sync::hlc::{Hlc, HlcStamp};
use crate::sync::{EntityKey, LogKey};

/// Longest genesis `host_label`, in bytes.
const HOST_LABEL_MAX: usize = 255;

/// What `open` found or did, logged once at daemon start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenReport {
    /// The log this node authors into.
    pub self_log: LogKey,
    /// The highest counter of that log.
    pub counter: u64,
    /// True when this open wrote the bootstrap genesis.
    pub bootstrapped: bool,
    pub parked_unknown: u64,
    pub parked_dep: u64,
    pub failed: u64,
}

/// A freshly authored and applied entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Authored {
    pub ingest_seq: i64,
    pub counter: u64,
    pub hlc: HlcStamp,
    pub hash: [u8; 32],
}

fn corrupt(detail: impl Into<String>) -> ApiError {
    api_error(ErrorCode::StoreCorrupt, detail)
}

fn log_label(node: &str, incarnation: i64) -> String {
    format!("{}/{incarnation}", node.get(..8).unwrap_or(node))
}

fn hash_from(bytes: Vec<u8>, what: &str) -> Result<[u8; 32], ApiError> {
    bytes
        .try_into()
        .map_err(|_| corrupt(format!("journal invariant: {what} is not 32 bytes")))
}

/// The log this node authors into.
struct ActiveLog {
    log_ix: i64,
    key: LogKey,
    contiguous: u64,
    head_hash: [u8; 32],
}

fn active_self_log(tx: &Connection) -> Result<ActiveLog, ApiError> {
    let row: Option<(i64, String, i64, i64, Vec<u8>)> = tx
        .query_row(
            "SELECT l.log_ix,l.node_uuid,l.incarnation,l.contiguous_counter,l.head_hash \
             FROM local_node n JOIN sync_logs l ON l.log_ix=n.active_log_ix",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()
        .map_err(store_error)?;
    let Some((log_ix, node, incarnation, contiguous, head)) = row else {
        return Err(api_error(ErrorCode::Unsupported, "journal not open"));
    };
    Ok(ActiveLog {
        log_ix,
        key: LogKey {
            node: node
                .parse()
                .map_err(|_| corrupt("journal invariant: malformed node uuid"))?,
            incarnation: u32::try_from(incarnation)
                .map_err(|_| corrupt("journal invariant: incarnation"))?,
        },
        contiguous: u64::try_from(contiguous)
            .map_err(|_| corrupt("journal invariant: contiguous_counter"))?,
        head_hash: hash_from(head, "head_hash")?,
    })
}

/// The commit guard: refuses to commit while the active self log holds an
/// entry that is not accounted for by ApplyEngine.
///
/// Owns: the one primary-key check `applied_through = contiguous_counter` for
/// the active self log, run by the two base writers after `apply` and before
/// `commit`. A local entry can therefore never commit without its
/// materialization (parked and failed head entries also count as processed).
/// Must not: repair anything or read the journal rows; it only compares the
/// two counters. It is a no-op while the store has no active self log (before
/// `open` bootstraps). Role: last line of defence behind `author`; a trip is
/// a programming error reported as `StoreCorrupt "self journal entry left
/// unapplied"`, and the transaction rolls back.
pub fn check_self_applied(tx: &Connection) -> Result<(), ApiError> {
    let counters: Option<(i64, i64)> = tx
        .query_row(
            "SELECT l.applied_through,l.contiguous_counter \
             FROM local_node n JOIN sync_logs l ON l.log_ix=n.active_log_ix",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(store_error)?;
    match counters {
        Some((applied, contiguous)) if applied != contiguous => {
            Err(corrupt("self journal entry left unapplied"))
        }
        _ => Ok(()),
    }
}

/// `(log, applied_through)` for every log with something applied, ordered by
/// `(node, incarnation)` as `Deps` requires.
pub fn applied_vector(conn: &Connection) -> Result<Vec<(LogKey, u64)>, ApiError> {
    let mut statement = conn
        .prepare(
            "SELECT node_uuid,incarnation,applied_through FROM sync_logs \
             WHERE applied_through>0 ORDER BY node_uuid,incarnation",
        )
        .map_err(store_error)?;
    let rows = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })
        .map_err(store_error)?;
    let mut out = Vec::new();
    for row in rows {
        let (node, incarnation, applied) = row.map_err(store_error)?;
        let node: NodeUuid = node
            .parse()
            .map_err(|_| corrupt("journal invariant: malformed node uuid"))?;
        out.push((
            LogKey {
                node,
                incarnation: u32::try_from(incarnation)
                    .map_err(|_| corrupt("journal invariant: incarnation"))?,
            },
            u64::try_from(applied).map_err(|_| corrupt("journal invariant: applied_through"))?,
        ));
    }
    Ok(out)
}

/// Startup invariants over every log, in one pass.
fn verify_invariants(tx: &Transaction<'_>) -> Result<(), ApiError> {
    let mut statement = tx
        .prepare(
            "SELECT l.log_ix,l.node_uuid,l.incarnation,l.contiguous_counter,l.head_hash,l.applied_through, \
             (l.log_ix IS (SELECT active_log_ix FROM local_node)), \
             COALESCE((SELECT MAX(counter) FROM journal j WHERE j.log_ix=l.log_ix),0), \
             (SELECT hash FROM journal j WHERE j.log_ix=l.log_ix AND j.counter=l.contiguous_counter) \
             FROM sync_logs l ORDER BY l.log_ix",
        )
        .map_err(store_error)?;
    let rows = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, Vec<u8>>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, bool>(6)?,
                r.get::<_, i64>(7)?,
                r.get::<_, Option<Vec<u8>>>(8)?,
            ))
        })
        .map_err(store_error)?;
    for row in rows {
        let (node, incarnation, contiguous, head, applied, active, stored_max, stored_hash) =
            row.map_err(store_error)?;
        let label = log_label(&node, incarnation);
        if contiguous != stored_max {
            return Err(corrupt(format!(
                "journal invariant: {label} contiguous_counter {contiguous} != stored max {stored_max}"
            )));
        }
        let expected = stored_hash.unwrap_or_else(|| vec![0u8; 32]);
        if head != expected {
            return Err(corrupt(format!(
                "journal invariant: {label} head_hash does not match the stored hash at counter {contiguous}"
            )));
        }
        if active && applied != contiguous {
            return Err(corrupt(format!(
                "journal invariant: {label} applied_through {applied} != contiguous_counter {contiguous} (active self log)"
            )));
        }
    }
    Ok(())
}

fn report(tx: &Connection, bootstrapped: bool) -> Result<OpenReport, ApiError> {
    let log = active_self_log(tx)?;
    let count = |state: &str| -> Result<u64, ApiError> {
        tx.query_row(
            "SELECT COUNT(*) FROM journal WHERE apply_state=?1",
            [state],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| u64::try_from(n).unwrap_or(0))
        .map_err(store_error)
    };
    Ok(OpenReport {
        self_log: log.key,
        counter: log.contiguous,
        bootstrapped,
        parked_unknown: count("parked_unknown")?,
        parked_dep: count("parked_dep")?,
        failed: count("failed")?,
    })
}

/// Opens the journal for this daemon: verifies the startup invariants, resumes
/// the HLC, bootstraps the genesis when the store has no active self log and
/// installs the [`SyncState`], all in one `BEGIN IMMEDIATE`.
///
/// A violated invariant is `StoreCorrupt "journal invariant: <log> <what>"`
/// and refuses the start. Invariant I1: the active self log is never frozen.
/// A second `open` on the same context changes nothing. The bootstrap
/// incarnation is drawn at random through the context's incarnation source,
/// never a fixed 1, so a wiped store with its key intact starts a new log.
pub fn open(
    ctx: &StoreContext,
    conn: &mut Connection,
    identity: Arc<NodeIdentity>,
    host_label: &str,
    registry: &'static FactRegistry,
) -> Result<OpenReport, ApiError> {
    if ctx.journal_open() {
        return report(conn, false);
    }
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(store_error)?;
    let local_id: String = tx
        .query_row("SELECT id FROM local_node", [], |r| r.get(0))
        .map_err(store_error)?;
    if local_id != identity.uuid().to_string() {
        return Err(corrupt(
            "journal invariant: node identity does not match local_node",
        ));
    }
    verify_invariants(&tx)?;
    let frozen: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM local_node n JOIN sync_logs l ON l.log_ix=n.active_log_ix \
             WHERE l.state='frozen')",
            [],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if frozen {
        return Err(corrupt("journal invariant: active self log frozen"));
    }
    let self_max: Option<i64> = tx
        .query_row(
            "SELECT MAX(j.hlc) FROM sync_logs l JOIN journal j \
             ON j.log_ix=l.log_ix AND j.counter=l.contiguous_counter WHERE l.is_self=1",
            [],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    let wall = ctx.clock().utc_now();
    let state = SyncState {
        identity,
        hlc: std::sync::Mutex::new(Hlc::resume(
            wall,
            self_max
                .and_then(|h| u64::try_from(h).ok())
                .map(HlcStamp::from_u64),
        )),
        registry,
        headroom_bytes: AtomicU64::new(INGEST_HEADROOM_BYTES),
        incarnation_rng: std::sync::Mutex::new(ctx.new_incarnation_rng()),
    };
    let active: Option<i64> = tx
        .query_row("SELECT active_log_ix FROM local_node", [], |r| r.get(0))
        .map_err(store_error)?;
    let bootstrapped = active.is_none();
    if bootstrapped {
        let decision = DecisionInstant {
            utc: wall,
            monotonic: ctx.clock().monotonic_now(),
        };
        let incarnation = state
            .incarnation_rng
            .lock()
            .map_err(|_| corrupt("incarnation source poisoned"))?
            .draw_incarnation();
        begin_incarnation_with(&tx, &state, decision, incarnation, None, host_label)?;
    }
    let opened = report(&tx, bootstrapped)?;
    tx.commit().map_err(store_error)?;
    // Installed after commit: a failed open leaves the context closed.
    ctx.install_sync(state);
    Ok(opened)
}

/// Starts the log of `incarnation` for this node and makes it the active self
/// log: builds and seals the `node.genesis` entry at counter 1, inserts the
/// `sync_logs` and `journal` rows, points `local_node.active_log_ix` at it and
/// applies it. `predecessor` names the log it replaces, if any. Refuses
/// (`Conflict`) when a log for `(self, incarnation)` already exists. Public
/// for ht-5fr.6's rotation.
pub fn begin_incarnation(
    tx: &Transaction<'_>,
    ctx: &StoreContext,
    decision: DecisionInstant,
    incarnation: u32,
    predecessor: Option<Predecessor>,
    host_label: &str,
) -> Result<Authored, ApiError> {
    begin_incarnation_with(
        tx,
        ctx.sync()?,
        decision,
        incarnation,
        predecessor,
        host_label,
    )
}

fn begin_incarnation_with(
    tx: &Transaction<'_>,
    sync: &SyncState,
    decision: DecisionInstant,
    incarnation: u32,
    predecessor: Option<Predecessor>,
    host_label: &str,
) -> Result<Authored, ApiError> {
    if incarnation == 0 {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "incarnation must be at least 1",
        ));
    }
    let (id, tag, tag_k): (String, String, u32) = tx
        .query_row("SELECT id,tag,tag_k FROM local_node", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .map_err(store_error)?;
    let exists: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sync_logs WHERE node_uuid=?1 AND incarnation=?2)",
            params![id, incarnation],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if exists {
        return Err(api_error(
            ErrorCode::Conflict,
            format!("log {}/{incarnation} already exists", &id[..8]),
        ));
    }
    let mut label = host_label.to_owned();
    if label.len() > HOST_LABEL_MAX {
        let mut end = HOST_LABEL_MAX;
        while !label.is_char_boundary(end) {
            end -= 1;
        }
        label.truncate(end);
    }
    let payload = serde_json::to_vec(&GenesisPayload {
        pubkey: hex_encode(&sync.identity.public_key()),
        tag,
        tag_k,
        incarnation,
        predecessor,
        host_label: label,
    })
    .map_err(|e| api_error(ErrorCode::InvalidRequest, format!("genesis payload: {e}")))?;
    let log = LogKey {
        node: sync.identity.uuid(),
        incarnation,
    };
    let hlc = sync
        .hlc
        .lock()
        .map_err(|_| corrupt("hlc poisoned"))?
        .tick(decision.utc);
    let unsigned = UnsignedEnvelope {
        log,
        counter: 1,
        prev_hash: [0u8; 32],
        hlc,
        kind: GENESIS_KIND.to_owned(),
        kind_version: NodeGenesis::MAJOR,
        deps: Deps::empty(),
        creates: None,
        needs: Vec::new(),
        payload,
    };
    unsigned
        .validate_shape()
        .map_err(|e| api_error(ErrorCode::InvalidRequest, format!("genesis envelope: {e}")))?;
    let envelope = unsigned.seal(&sync.identity);
    tx.execute(
        "INSERT INTO sync_logs(node_uuid,incarnation,is_self,pubkey,genesis_hash,contiguous_counter,head_hash,first_seen_at) \
         VALUES (?1,?2,1,?3,?4,1,?4,?5)",
        params![
            id,
            incarnation,
            sync.identity.public_key().as_slice(),
            envelope.hash.as_slice(),
            decision.utc.0
        ],
    )
    .map_err(store_error)?;
    let log_ix = tx.last_insert_rowid();
    let ingest_seq = insert_pending(tx, log_ix, &envelope, decision)?;
    tx.execute("UPDATE local_node SET active_log_ix=?1", [log_ix])
        .map_err(store_error)?;
    apply::apply_authored_with(tx, sync, decision, ingest_seq)?;
    Ok(Authored {
        ingest_seq,
        counter: 1,
        hlc,
        hash: envelope.hash,
    })
}

fn insert_pending(
    tx: &Transaction<'_>,
    log_ix: i64,
    envelope: &crate::sync::envelope::Envelope,
    decision: DecisionInstant,
) -> Result<i64, ApiError> {
    let needs = envelope
        .needs
        .iter()
        .map(EntityKey::as_str)
        .collect::<Vec<_>>()
        .join("\n");
    tx.execute(
        "INSERT INTO journal(log_ix,counter,prev_hash,hash,hlc,kind,kind_version,deps,creates,needs,payload,signature,ingested_at,apply_state) \
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,'pending')",
        params![
            log_ix,
            envelope.counter as i64,
            envelope.prev_hash.as_slice(),
            envelope.hash.as_slice(),
            i64::try_from(envelope.hlc.as_u64()).unwrap_or(i64::MAX),
            envelope.kind,
            envelope.kind_version,
            envelope.deps.encode(),
            envelope.creates.as_ref().map(EntityKey::as_str),
            needs,
            envelope.payload,
            envelope.signature.as_slice(),
            decision.utc.0,
        ],
    )
    .map_err(store_error)?;
    Ok(tx.last_insert_rowid())
}

/// Authors fact `K` into the active self log and applies it in `tx`.
///
/// The steps: tick the HLC, read the active self log (next counter, previous
/// hash), take the applied vector minus the self log as `deps`, encode the
/// payload (over 512 KiB is `InvalidRequest`), extract `creates`/`needs`,
/// validate and seal, insert `pending`, then `apply::apply_authored`. A handler
/// rejection, or any outcome other than `applied` (`Conflict "fact <kind> not
/// applicable: <reason>"` for a parked one), is returned as an error, so the
/// caller's transaction rolls back and nothing is recorded `failed`.
pub fn author<K: FactKind>(
    tx: &Transaction<'_>,
    ctx: &StoreContext,
    decision: DecisionInstant,
    payload: &K::Payload,
) -> Result<Authored, ApiError> {
    author_with::<K>(tx, ctx.sync()?, decision, payload, None)
}

/// [`author`] for a last-writer-wins write: the entry's HLC is above `floor`
/// (the register's current stamp), so it wins against what the author saw.
pub fn author_above<K: FactKind>(
    tx: &Transaction<'_>,
    ctx: &StoreContext,
    decision: DecisionInstant,
    payload: &K::Payload,
    floor: HlcStamp,
) -> Result<Authored, ApiError> {
    author_with::<K>(tx, ctx.sync()?, decision, payload, Some(floor))
}

fn author_with<K: FactKind>(
    tx: &Transaction<'_>,
    sync: &SyncState,
    decision: DecisionInstant,
    payload: &K::Payload,
    floor: Option<HlcStamp>,
) -> Result<Authored, ApiError> {
    let bytes = serde_json::to_vec(payload).map_err(|e| {
        api_error(
            ErrorCode::InvalidRequest,
            format!("fact payload encode: {e}"),
        )
    })?;
    if bytes.len() > PAYLOAD_MAX {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "fact payload exceeds 512 KiB",
        ));
    }
    let active = active_self_log(tx)?;
    let counter = active.contiguous + 1;
    let hlc = {
        let mut clock = sync.hlc.lock().map_err(|_| corrupt("hlc poisoned"))?;
        match floor {
            None => clock.tick(decision.utc),
            Some(floor) => clock.tick_above(decision.utc, floor),
        }
    };
    let deps = Deps::new(
        applied_vector(tx)?
            .into_iter()
            .filter(|(log, _)| *log != active.key)
            .collect(),
        &active.key,
    )
    .map_err(|e| api_error(ErrorCode::InvalidRequest, format!("fact deps: {e}")))?;
    let mut needs = K::needs(payload);
    needs.sort();
    needs.dedup();
    let unsigned = UnsignedEnvelope {
        log: active.key,
        counter,
        prev_hash: active.head_hash,
        hlc,
        kind: K::NAME.to_owned(),
        kind_version: K::MAJOR,
        deps,
        creates: K::creates(payload),
        needs,
        payload: bytes,
    };
    unsigned
        .validate_shape()
        .map_err(|e| api_error(ErrorCode::InvalidRequest, format!("fact {}: {e}", K::NAME)))?;
    let envelope = unsigned.seal(&sync.identity);
    let ingest_seq = insert_pending(tx, active.log_ix, &envelope, decision)?;
    tx.execute(
        "UPDATE sync_logs SET contiguous_counter=?2, head_hash=?3 WHERE log_ix=?1",
        params![active.log_ix, counter as i64, envelope.hash.as_slice()],
    )
    .map_err(store_error)?;
    apply::apply_authored_with(tx, sync, decision, ingest_seq)?;
    Ok(Authored {
        ingest_seq,
        counter,
        hlc,
        hash: envelope.hash,
    })
}

#[cfg(test)]
#[path = "../../tests/store/journal.rs"]
mod tests;
