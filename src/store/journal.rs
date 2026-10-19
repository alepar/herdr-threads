//! The journal: the per-origin hash chain of signed facts, in SQLite.
//!
//! Responsibility
//! - Owns: `sync_logs`, `journal` and `journal_forked` as their single writer.
//!   In this file that is startup ([`open`]: invariants, HLC resume, genesis
//!   bootstrap through [`begin_incarnation`]), local authoring ([`author`],
//!   [`author_above`]: tick, next counter, `prev_hash`, deps, seal, insert,
//!   then hand to ApplyEngine in the same transaction; [`reissue`], the second
//!   entry point, which copies a frozen log's entry into the active log), the applied vector
//!   ([`applied_vector`]) and the commit guard ([`check_self_applied`]);
//!   verified batch ingest of other nodes' entries ([`ingest`]) and the
//!   committed-range read side ([`serve`], [`vector`], [`stored_hash`]); and
//!   the fork bookkeeping Incarnations calls ([`store_forked`], [`freeze`]).
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

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64};

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use sha2::{Digest, Sha256};

pub use crate::sync::facts::Predecessor;

use crate::node::{NodeIdentity, NodeUuid};
use crate::protocol::results::{ApiError, ErrorCode};
use crate::protocol::time::{CallBudget, UtcMillis};
use crate::store::apply;
use crate::store::connection::{
    DecisionInstant, INGEST_HEADROOM_BYTES, OwnHistoryHold, StoreContext, SyncState, api_error,
    store_error,
};
use crate::sync::chain::{ChainStep, HeldLog, check_next};
use crate::sync::envelope::{
    Deps, Envelope, EnvelopeError, GENESIS_KIND, OpaqueEntry, OpaqueHeader, PAYLOAD_MAX,
    StoredEntry, UnsignedEnvelope, hex_encode,
};
use crate::sync::facts::{FactKind, FactRegistry, GenesisPayload, NodeGenesis};
use crate::sync::hlc::{Fold, Hlc, HlcStamp};
use crate::sync::journal_port::{ApplyProgress, ForkEvidence, IngestOutcome, LogMeta};
use crate::sync::{BatchLimits, EntityKey, LogHead, LogKey, LogState, Vector};

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
    /// The tag counter this entry or a follow-up it applied raised, if any.
    pub self_tag_k: Option<u32>,
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

/// Startup invariants over every log, in one pass. Read-only: `sync verify`
/// runs it on a query connection.
pub(crate) fn verify_invariants(tx: &Connection) -> Result<(), ApiError> {
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
    let apply_hint = Arc::new(AtomicBool::new(true));
    let state = SyncState {
        apply_epochs: Arc::new(super::connection::ApplyEpochs::new()),
        identity,
        hlc: std::sync::Mutex::new(Hlc::resume(
            wall,
            self_max
                .and_then(|h| u64::try_from(h).ok())
                .map(HlcStamp::from_u64),
        )),
        registry,
        headroom_bytes: AtomicU64::new(INGEST_HEADROOM_BYTES),
        branch_walk_bound: AtomicU64::new(BRANCH_WALK_MAX_STEPS),
        incarnation_rng: std::sync::Mutex::new(ctx.new_incarnation_rng()),
        skew_logged: std::sync::Mutex::default(),
        promote_logged: std::sync::Mutex::default(),
        // The first Apply pass after start replays and drains regardless.
        own_history_hold: Arc::new(OwnHistoryHold::new(Arc::clone(&apply_hint))),
        apply_hint,
        rotated_since_start: AtomicBool::new(false),
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
        let incarnation = crate::store::incarnations::draw_unused_incarnation(&tx, &state)?;
        begin_incarnation_with(&tx, &state, decision, incarnation, None, host_label)?;
    }
    // A build that has learned a layout promotes its parked entries at start.
    promote_parked(&tx, &state.promote_logged)?;
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
    let sync = ctx.sync()?;
    let authored =
        begin_incarnation_with(tx, sync, decision, incarnation, predecessor, host_label)?;
    sync.rotated_since_start
        .store(true, std::sync::atomic::Ordering::Release);
    Ok(authored)
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
    let self_tag_k = apply::apply_authored_with(tx, sync, decision, ingest_seq, 0)?;
    Ok(Authored {
        ingest_seq,
        counter: 1,
        hlc,
        hash: envelope.hash,
        self_tag_k,
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
        "INSERT INTO journal(log_ix,counter,prev_hash,hash,hlc,kind,kind_version,deps,creates,needs,payload,signature,ingested_at,apply_state,raw) \
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,'pending',?14)",
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
            envelope.canonical_bytes(),
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
    let mut needs = K::needs(payload);
    needs.sort();
    needs.dedup();
    author_encoded(
        tx,
        sync,
        decision,
        (K::NAME, K::MAJOR),
        bytes,
        K::creates(payload),
        needs,
        floor,
        0,
    )
}

/// Authors a follow-up a handler queued: the same path as [`author`], over the
/// kind, payload bytes, `creates` and `needs` captured when it was queued.
/// `depth` is the follow-up level it is authored at (its trigger's plus one);
/// past [`apply::MAX_FOLLOW_UP_DEPTH`] nothing is authored.
pub(crate) fn author_queued(
    tx: &Transaction<'_>,
    sync: &SyncState,
    decision: DecisionInstant,
    fact: &apply::QueuedFact,
    depth: u8,
) -> Result<Authored, ApiError> {
    if depth > apply::MAX_FOLLOW_UP_DEPTH {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            format!("follow-up chain deeper than {}", apply::MAX_FOLLOW_UP_DEPTH),
        ));
    }
    author_encoded(
        tx,
        sync,
        decision,
        (fact.kind, fact.major),
        fact.payload.clone(),
        fact.creates.clone(),
        fact.needs.clone(),
        None,
        depth,
    )
}

#[allow(clippy::too_many_arguments)]
fn author_encoded(
    tx: &Transaction<'_>,
    sync: &SyncState,
    decision: DecisionInstant,
    (kind, major): (&str, u32),
    bytes: Vec<u8>,
    creates: Option<EntityKey>,
    needs: Vec<EntityKey>,
    floor: Option<HlcStamp>,
    depth: u8,
) -> Result<Authored, ApiError> {
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
    let unsigned = UnsignedEnvelope {
        log: active.key,
        counter,
        prev_hash: active.head_hash,
        hlc,
        kind: kind.to_owned(),
        kind_version: major,
        deps,
        creates,
        needs,
        payload: bytes,
    };
    unsigned
        .validate_shape()
        .map_err(|e| api_error(ErrorCode::InvalidRequest, format!("fact {kind}: {e}")))?;
    let envelope = unsigned.seal(&sync.identity);
    let ingest_seq = insert_pending(tx, active.log_ix, &envelope, decision)?;
    tx.execute(
        "UPDATE sync_logs SET contiguous_counter=?2, head_hash=?3 WHERE log_ix=?1",
        params![active.log_ix, counter as i64, envelope.hash.as_slice()],
    )
    .map_err(store_error)?;
    let self_tag_k = apply::apply_authored_with(tx, sync, decision, ingest_seq, depth)?;
    Ok(Authored {
        ingest_seq,
        counter,
        hlc,
        hash: envelope.hash,
        self_tag_k,
    })
}

/// Journal's second authoring entry point: re-authors `source`, an entry of a
/// frozen own log or of one of its branches, as the next entry of the active
/// self log, and applies it under [`apply::ApplyMode::Reissue`].
///
/// The steps: read the active self log (next counter, previous hash), take
/// `deps` as the element-wise maximum of the applied vector and the source's
/// own deps without the active log (so a receiver never applies the copy
/// before facts the original causally followed), copy `kind`, `kind_version`,
/// `hlc`, `creates`, `needs` and the payload bytes unchanged, seal, insert
/// `pending`, advance the head, then `apply_one`.
///
/// The HLC is not ticked: a re-issued register fact keeps its `(hlc, node)`
/// and so loses last-writer-wins to itself rather than beating concurrent
/// later writes it never saw, and `sent_at` stays stable.
///
/// Unlike [`author`], a handler rejection (or a park) never aborts: the fact
/// was validly authored once and other nodes need the copy, so the outcome is
/// recorded (`failed`, `parked_*`) and the commit guard still holds, because
/// every recorded outcome counts the head as processed. Only a store error
/// returns `Err`.
pub fn reissue(
    tx: &Transaction<'_>,
    ctx: &StoreContext,
    decision: DecisionInstant,
    source: &Envelope,
) -> Result<Authored, ApiError> {
    let sync = ctx.sync()?;
    let active = active_self_log(tx)?;
    let counter = active.contiguous + 1;
    let mut merged: BTreeMap<LogKey, u64> = applied_vector(tx)?.into_iter().collect();
    for (log, at) in source.deps.entries() {
        let slot = merged.entry(*log).or_insert(0);
        *slot = (*slot).max(*at);
    }
    merged.remove(&active.key);
    let deps = Deps::new(merged.into_iter().collect(), &active.key)
        .map_err(|e| api_error(ErrorCode::InvalidRequest, format!("reissue deps: {e}")))?;
    let unsigned = UnsignedEnvelope {
        log: active.key,
        counter,
        prev_hash: active.head_hash,
        hlc: source.hlc,
        kind: source.kind.clone(),
        kind_version: source.kind_version,
        deps,
        creates: source.creates.clone(),
        needs: source.needs.clone(),
        payload: source.payload.clone(),
    };
    unsigned.validate_shape().map_err(|e| {
        api_error(
            ErrorCode::InvalidRequest,
            format!("reissue {}: {e}", source.kind),
        )
    })?;
    let envelope = unsigned.seal(&sync.identity);
    let ingest_seq = insert_pending(tx, active.log_ix, &envelope, decision)?;
    tx.execute(
        "UPDATE sync_logs SET contiguous_counter=?2, head_hash=?3 WHERE log_ix=?1",
        params![active.log_ix, counter as i64, envelope.hash.as_slice()],
    )
    .map_err(store_error)?;
    apply::apply_one(
        tx,
        ctx,
        decision,
        ingest_seq,
        apply::ApplyMode::Reissue { of: source.log },
    )?;
    Ok(Authored {
        ingest_seq,
        counter,
        hlc: source.hlc,
        hash: envelope.hash,
        self_tag_k: None,
    })
}

/// Node schema 5 row copy (ht-76u6): moves every `journal` and `journal_forked`
/// row of a schema-4 store into `journal_new` / `journal_forked_new`, computing
/// `raw` as the canonical bytes of the stored projection and refusing
/// (`StoreCorrupt`) any row whose SHA-256 is not its stored hash. DML only: it
/// runs inside the delta transaction, and the schema audit replays only the
/// delta SQL. `ingest_seq` / `forked_ix` are preserved (`thread_registers` and
/// `sync_reissues` point at them) and so are the AUTOINCREMENT high-water marks.
pub(crate) fn copy_rows_with_raw(conn: &Connection) -> Result<(), ApiError> {
    // Columns 3..=13 are the eleven `RawEnvelope::read_at` columns.
    let mut read = conn
        .prepare(
            "SELECT j.ingest_seq, l.node_uuid, l.incarnation, j.counter, j.prev_hash, j.hlc, \
             j.kind, j.kind_version, j.deps, j.creates, j.needs, j.payload, j.hash, j.signature, \
             j.log_ix, j.ingested_at, j.apply_state, j.applied_seq, j.parked_on, j.apply_error \
             FROM journal j JOIN sync_logs l ON l.log_ix=j.log_ix ORDER BY j.ingest_seq",
        )
        .map_err(store_error)?;
    let mut write = conn
        .prepare(
            "INSERT INTO journal_new(ingest_seq,log_ix,counter,prev_hash,hash,signature,raw,hlc,kind,\
             kind_version,deps,creates,needs,payload,ingested_at,apply_state,applied_seq,parked_on,apply_error) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)",
        )
        .map_err(store_error)?;
    let mut rows = read.query([]).map_err(store_error)?;
    while let Some(r) = rows.next().map_err(store_error)? {
        let ingest_seq: i64 = r.get(0).map_err(store_error)?;
        let raw = recompute_raw(r, "journal", ingest_seq)?;
        write
            .execute(params![
                ingest_seq,
                r.get::<_, i64>(14).map_err(store_error)?,
                r.get::<_, i64>(3).map_err(store_error)?,
                r.get::<_, Vec<u8>>(4).map_err(store_error)?,
                r.get::<_, Vec<u8>>(12).map_err(store_error)?,
                r.get::<_, Vec<u8>>(13).map_err(store_error)?,
                raw,
                r.get::<_, i64>(5).map_err(store_error)?,
                r.get::<_, String>(6).map_err(store_error)?,
                r.get::<_, i64>(7).map_err(store_error)?,
                r.get::<_, Vec<u8>>(8).map_err(store_error)?,
                r.get::<_, Option<String>>(9).map_err(store_error)?,
                r.get::<_, String>(10).map_err(store_error)?,
                r.get::<_, Vec<u8>>(11).map_err(store_error)?,
                r.get::<_, i64>(15).map_err(store_error)?,
                r.get::<_, String>(16).map_err(store_error)?,
                r.get::<_, Option<i64>>(17).map_err(store_error)?,
                r.get::<_, Option<String>>(18).map_err(store_error)?,
                r.get::<_, Option<String>>(19).map_err(store_error)?,
            ])
            .map_err(store_error)?;
    }
    drop(rows);

    let mut read = conn
        .prepare(
            "SELECT j.forked_ix, l.node_uuid, l.incarnation, j.counter, j.prev_hash, j.hlc, \
             j.kind, j.kind_version, j.deps, j.creates, j.needs, j.payload, j.hash, j.signature, \
             j.log_ix, j.source_node, j.received_at \
             FROM journal_forked j JOIN sync_logs l ON l.log_ix=j.log_ix ORDER BY j.forked_ix",
        )
        .map_err(store_error)?;
    let mut write = conn
        .prepare(
            "INSERT INTO journal_forked_new(forked_ix,log_ix,counter,prev_hash,hash,signature,raw,hlc,kind,\
             kind_version,deps,creates,needs,payload,source_node,received_at) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
        )
        .map_err(store_error)?;
    let mut rows = read.query([]).map_err(store_error)?;
    while let Some(r) = rows.next().map_err(store_error)? {
        let forked_ix: i64 = r.get(0).map_err(store_error)?;
        let raw = recompute_raw(r, "journal_forked", forked_ix)?;
        write
            .execute(params![
                forked_ix,
                r.get::<_, i64>(14).map_err(store_error)?,
                r.get::<_, i64>(3).map_err(store_error)?,
                r.get::<_, Vec<u8>>(4).map_err(store_error)?,
                r.get::<_, Vec<u8>>(12).map_err(store_error)?,
                r.get::<_, Vec<u8>>(13).map_err(store_error)?,
                raw,
                r.get::<_, i64>(5).map_err(store_error)?,
                r.get::<_, String>(6).map_err(store_error)?,
                r.get::<_, i64>(7).map_err(store_error)?,
                r.get::<_, Vec<u8>>(8).map_err(store_error)?,
                r.get::<_, Option<String>>(9).map_err(store_error)?,
                r.get::<_, String>(10).map_err(store_error)?,
                r.get::<_, Vec<u8>>(11).map_err(store_error)?,
                r.get::<_, Option<String>>(15).map_err(store_error)?,
                r.get::<_, i64>(16).map_err(store_error)?,
            ])
            .map_err(store_error)?;
    }
    drop(rows);

    // An explicit-rowid insert already moves the high-water mark to the maximum;
    // this keeps any gap the old table's counter had.
    for (old, new) in [
        ("journal", "journal_new"),
        ("journal_forked", "journal_forked_new"),
    ] {
        conn.execute(
            "UPDATE sqlite_sequence SET seq=(SELECT seq FROM sqlite_sequence WHERE name=?1) \
             WHERE name=?2 AND seq < (SELECT seq FROM sqlite_sequence WHERE name=?1)",
            params![old, new],
        )
        .map_err(store_error)?;
    }
    Ok(())
}

/// The canonical bytes of one schema-4 row read in [`copy_rows_with_raw`]'s
/// layout (log node and incarnation at 1 and 2, the eleven envelope columns from
/// 3), checked against the stored hash.
fn recompute_raw(r: &rusqlite::Row<'_>, table: &str, row_id: i64) -> Result<Vec<u8>, ApiError> {
    let node: String = r.get(1).map_err(store_error)?;
    let incarnation: i64 = r.get(2).map_err(store_error)?;
    let label = log_label(&node, incarnation);
    let bad = |what: &str| {
        corrupt(format!(
            "schema delta 5: {table} row {row_id} ({label}): {what}"
        ))
    };
    let log = LogKey {
        node: node.parse().map_err(|_| bad("malformed node uuid"))?,
        incarnation: u32::try_from(incarnation).map_err(|_| bad("incarnation"))?,
    };
    let stored = RawEnvelope::read_at(r, 3).map_err(store_error)?;
    let counter = stored.counter;
    let hash = stored.hash.clone();
    let envelope = stored.decode(&log)?;
    let raw = envelope.canonical_bytes();
    if Sha256::digest(&raw).as_slice() != hash.as_slice() {
        return Err(bad(&format!(
            "counter {counter} does not re-encode to its hash"
        )));
    }
    Ok(raw)
}

/// What [`freeze`] did to a log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FreezeOutcome {
    /// The log was active and is now frozen at the requested counter.
    Frozen,
    /// The log was frozen higher; its freeze counter was lowered.
    Lowered,
    /// The log is already frozen at or below the requested counter.
    Unchanged,
}

/// The stored columns of one journal row, read before decoding so a decode
/// failure is a typed `StoreCorrupt` and not a SQLite error.
struct RawEnvelope {
    counter: i64,
    prev_hash: Vec<u8>,
    hlc: i64,
    kind: String,
    kind_version: u32,
    deps: Vec<u8>,
    creates: Option<String>,
    needs: String,
    payload: Vec<u8>,
    hash: Vec<u8>,
    signature: Vec<u8>,
}

impl RawEnvelope {
    /// Reads the eleven columns that start at `at`: `counter, prev_hash, hlc, kind,
    /// kind_version, deps, creates, needs, payload, hash, signature`.
    fn read_at(r: &rusqlite::Row<'_>, at: usize) -> rusqlite::Result<Self> {
        Ok(Self {
            counter: r.get(at)?,
            prev_hash: r.get(at + 1)?,
            hlc: r.get(at + 2)?,
            kind: r.get(at + 3)?,
            kind_version: r.get(at + 4)?,
            deps: r.get(at + 5)?,
            creates: r.get(at + 6)?,
            needs: r.get(at + 7)?,
            payload: r.get(at + 8)?,
            hash: r.get(at + 9)?,
            signature: r.get(at + 10)?,
        })
    }

    fn decode(self, log: &LogKey) -> Result<Envelope, ApiError> {
        let bad = |what: &str| corrupt(format!("journal invariant: stored entry {what}"));
        Ok(Envelope {
            log: *log,
            counter: u64::try_from(self.counter).map_err(|_| bad("counter"))?,
            prev_hash: hash_from(self.prev_hash, "prev_hash")?,
            hlc: HlcStamp::from_u64(u64::try_from(self.hlc).map_err(|_| bad("hlc"))?),
            kind: self.kind,
            kind_version: self.kind_version,
            deps: Deps::decode(&self.deps, log).map_err(|_| bad("deps"))?,
            creates: self
                .creates
                .map(EntityKey::new)
                .transpose()
                .map_err(|_| bad("creates"))?,
            needs: self
                .needs
                .split('\n')
                .filter(|key| !key.is_empty())
                .map(EntityKey::new)
                .collect::<Result<_, _>>()
                .map_err(|_| bad("needs"))?,
            payload: self.payload,
            hash: hash_from(self.hash, "hash")?,
            signature: self
                .signature
                .try_into()
                .map_err(|_| corrupt("journal invariant: signature is not 64 bytes"))?,
        })
    }
}

/// One `sync_logs` row as ingest and the read side see it.
struct LogRow {
    log_ix: i64,
    held: HeldLog,
}

impl LogRow {
    fn head(&self) -> LogHead {
        LogHead {
            counter: self.held.contiguous,
            head_hash: self.held.head_hash,
            state: self.held.state,
            frozen_at: self
                .held
                .frozen_at
                .map(|at| i64::try_from(at).unwrap_or(i64::MAX)),
        }
    }
}

fn parse_state(state: &str) -> Result<LogState, ApiError> {
    match state {
        "active" => Ok(LogState::Active),
        "frozen" => Ok(LogState::Frozen),
        _ => Err(corrupt("journal invariant: unknown log state")),
    }
}

fn load_log(conn: &Connection, log: &LogKey) -> Result<Option<LogRow>, ApiError> {
    type Raw = (i64, Vec<u8>, i64, Vec<u8>, String, Option<i64>, bool);
    let raw: Option<Raw> = conn
        .query_row(
            "SELECT log_ix,pubkey,contiguous_counter,head_hash,state,frozen_at_counter, \
             (log_ix IS (SELECT active_log_ix FROM local_node)) \
             FROM sync_logs WHERE node_uuid=?1 AND incarnation=?2",
            params![log.node.to_string(), log.incarnation],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            },
        )
        .optional()
        .map_err(store_error)?;
    let Some((log_ix, pubkey, contiguous, head, state, frozen_at, active)) = raw else {
        return Ok(None);
    };
    Ok(Some(LogRow {
        log_ix,
        held: HeldLog {
            pubkey: pubkey
                .try_into()
                .map_err(|_| corrupt("journal invariant: pubkey is not 32 bytes"))?,
            contiguous: u64::try_from(contiguous)
                .map_err(|_| corrupt("journal invariant: contiguous_counter"))?,
            head_hash: hash_from(head, "head_hash")?,
            state: parse_state(&state)?,
            frozen_at: frozen_at.and_then(|at| u64::try_from(at).ok()),
            is_active_self: active,
            stored_hash: None,
        },
    }))
}

/// Whether `(counter, hash)` of `log_ix` is a chain position, or with
/// `include_branches` also a fork-branch row.
pub(crate) fn held_at(
    conn: &Connection,
    log_ix: i64,
    counter: u64,
    hash: &[u8; 32],
    include_branches: bool,
) -> Result<bool, ApiError> {
    let Ok(counter) = i64::try_from(counter) else {
        return Ok(false);
    };
    let branches = if include_branches {
        " OR EXISTS(SELECT 1 FROM journal_forked b WHERE b.log_ix=?1 AND b.counter=?2 AND b.hash=?3)"
    } else {
        ""
    };
    conn.query_row(
        &format!(
            "SELECT EXISTS(SELECT 1 FROM journal c WHERE c.log_ix=?1 AND c.counter=?2 AND c.hash=?3){branches}"
        ),
        params![log_ix, counter, hash.as_slice()],
        |r| r.get(0),
    )
    .map_err(store_error)
}

/// The `prev_hash` of the fork-branch row `(counter, hash)`, if held.
pub(crate) fn branch_prev(
    conn: &Connection,
    log_ix: i64,
    counter: u64,
    hash: &[u8; 32],
) -> Result<Option<[u8; 32]>, ApiError> {
    conn.query_row(
        "SELECT prev_hash FROM journal_forked b WHERE b.log_ix=?1 AND b.counter=?2 AND b.hash=?3",
        params![
            log_ix,
            i64::try_from(counter).unwrap_or(i64::MAX),
            hash.as_slice()
        ],
        |r| r.get::<_, Vec<u8>>(0),
    )
    .optional()
    .map_err(store_error)?
    .map(|prev| hash_from(prev, "prev_hash"))
    .transpose()
}

fn hash_at(conn: &Connection, log_ix: i64, counter: u64) -> Result<Option<[u8; 32]>, ApiError> {
    let Ok(counter) = i64::try_from(counter) else {
        return Ok(None);
    };
    conn.query_row(
        "SELECT hash FROM journal c WHERE c.log_ix=?1 AND c.counter=?2",
        params![log_ix, counter],
        |r| r.get::<_, Vec<u8>>(0),
    )
    .optional()
    .map_err(store_error)?
    .map(|hash| hash_from(hash, "hash"))
    .transpose()
}

/// How far a chain read goes past the entries this node understands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Rows {
    /// Every row, known and opaque.
    All,
    /// The known (HTJRNL01) rows before the first non-HTJRNL01 one; that row is
    /// not decoded and ends the read. What the store-level `Envelope` readers
    /// (`serve`, `serve_branch`) serve.
    KnownPrefix,
}

/// SQL truth of "this row's `raw` is an HTJRNL01 entry" (design §4, roast r3):
/// the chain, verify, fork, re-issue and serve paths classify by it, never by
/// projection presence. Only apply/materialize use projection presence.
pub(crate) const KNOWN_SQL: &str = "substr(raw,1,8)=CAST('HTJRNL01' AS BLOB)";

/// [`KNOWN_SQL`] on a table alias (`known_sql("j")`).
pub(crate) fn known_sql(alias: &str) -> String {
    KNOWN_SQL.replacen("substr(raw", &format!("substr({alias}.raw"), 1)
}

/// The select list of every chain or branch read over `table` (`journal` or
/// `journal_forked`, aliased `t`); callers append their `WHERE` and `ORDER BY`.
/// Columns, in [`StoredRow::read`] order: `counter, prev_hash, hash, signature,
/// known`, the row's raw bytes, then the seven projection columns. The raw
/// bytes are read only for a non-HTJRNL01 row under [`Rows::All`]; a known row
/// is built from its projection and never reads `raw`, and a prefix read never
/// reads a foreign row's blobs. A foreign row's projection (promoted or not) is
/// never read.
fn row_select(table: &str, rows: Rows) -> String {
    let known = known_sql("t");
    let raw = match rows {
        Rows::All => format!("CASE WHEN {known} THEN NULL ELSE t.raw END"),
        Rows::KnownPrefix => "NULL".to_owned(),
    };
    let when = |column: &str| format!("CASE WHEN {known} THEN t.{column} END");
    format!(
        "SELECT t.counter,t.prev_hash,t.hash,t.signature,({known}) AS known,{raw},{},{},{},{},{},{},{} FROM {table} t",
        when("hlc"),
        when("kind"),
        when("kind_version"),
        when("deps"),
        when("creates"),
        when("needs"),
        when("payload"),
    )
}

/// The stored projection of an HTJRNL01 row.
struct Projection {
    hlc: i64,
    kind: String,
    kind_version: u32,
    deps: Vec<u8>,
    creates: Option<String>,
    needs: String,
    payload: Vec<u8>,
}

/// One row of a chain or branch read, as stored, read before decoding so a
/// decode failure is a typed `StoreCorrupt` and not a SQLite error.
struct StoredRow {
    counter: i64,
    prev_hash: Vec<u8>,
    hash: Vec<u8>,
    signature: Vec<u8>,
    /// [`KNOWN_SQL`] held: `raw` is an HTJRNL01 entry.
    known: bool,
    raw: Option<Vec<u8>>,
    projection: Option<Projection>,
}

impl StoredRow {
    /// Reads the columns of [`row_select`].
    fn read(r: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        let known: bool = r.get(4)?;
        let projection = match r.get::<_, Option<i64>>(6)? {
            Some(hlc) if known => Some(Projection {
                hlc,
                kind: r.get(7)?,
                kind_version: r.get(8)?,
                deps: r.get(9)?,
                creates: r.get(10)?,
                needs: r.get(11)?,
                payload: r.get(12)?,
            }),
            _ => None,
        };
        Ok(Self {
            counter: r.get(0)?,
            prev_hash: r.get(1)?,
            hash: r.get(2)?,
            signature: r.get(3)?,
            known,
            raw: r.get(5)?,
            projection,
        })
    }

    /// `None` for a non-HTJRNL01 row under [`Rows::KnownPrefix`]: the cut.
    fn decode(self, log: &LogKey, rows: Rows) -> Result<Option<StoredEntry>, ApiError> {
        if self.known {
            let Some(projection) = self.projection else {
                return Err(corrupt(
                    "journal invariant: HTJRNL01 row without projection",
                ));
            };
            return Ok(Some(StoredEntry::Known(
                RawEnvelope {
                    counter: self.counter,
                    prev_hash: self.prev_hash,
                    hlc: projection.hlc,
                    kind: projection.kind,
                    kind_version: projection.kind_version,
                    deps: projection.deps,
                    creates: projection.creates,
                    needs: projection.needs,
                    payload: projection.payload,
                    hash: self.hash,
                    signature: self.signature,
                }
                .decode(log)?,
            )));
        }
        if rows == Rows::KnownPrefix {
            return Ok(None);
        }
        // No signature check on read, as the known arm does none.
        let bad = || corrupt("journal invariant: stored opaque entry");
        let header = OpaqueHeader {
            log: *log,
            counter: u64::try_from(self.counter).map_err(|_| bad())?,
            prev_hash: self.prev_hash.try_into().map_err(|_| bad())?,
            hash: self.hash.try_into().map_err(|_| bad())?,
            signature: self.signature.try_into().map_err(|_| bad())?,
        };
        let raw = self.raw.ok_or_else(bad)?;
        OpaqueEntry::new(header, raw)
            .map(|entry| Some(StoredEntry::Opaque(entry)))
            .map_err(|_| bad())
    }
}

/// The chain position `counter` of `log` as a stored entry, known or opaque.
fn entry_at_position(
    conn: &Connection,
    log_ix: i64,
    log: &LogKey,
    counter: u64,
) -> Result<StoredEntry, ApiError> {
    let mut batch = Batch::new(BatchLimits {
        entries: 1,
        bytes: usize::MAX,
    });
    entries_from(
        conn,
        log_ix,
        log,
        counter,
        Some(counter),
        Rows::All,
        &mut batch,
    )?;
    batch
        .out
        .pop()
        .ok_or_else(|| corrupt("journal invariant: held entry missing"))
}

/// The head of every log this node holds: `sync_logs` rows in `(node,
/// incarnation)` order, own logs included. Reads committed state only.
pub fn vector(conn: &Connection) -> Result<Vector, ApiError> {
    let mut statement = conn
        .prepare(
            "SELECT node_uuid,incarnation,contiguous_counter,head_hash,state,frozen_at_counter \
             FROM sync_logs",
        )
        .map_err(store_error)?;
    let rows = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, Vec<u8>>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, Option<i64>>(5)?,
            ))
        })
        .map_err(store_error)?;
    let mut heads = BTreeMap::new();
    for row in rows {
        let (node, incarnation, counter, head, state, frozen_at) = row.map_err(store_error)?;
        let key = LogKey {
            node: node
                .parse()
                .map_err(|_| corrupt("journal invariant: malformed node uuid"))?,
            incarnation: u32::try_from(incarnation)
                .map_err(|_| corrupt("journal invariant: incarnation"))?,
        };
        heads.insert(
            key,
            LogHead {
                counter: u64::try_from(counter)
                    .map_err(|_| corrupt("journal invariant: contiguous_counter"))?,
                head_hash: hash_from(head, "head_hash")?,
                state: parse_state(&state)?,
                frozen_at,
            },
        );
    }
    Ok(Vector(heads))
}

/// The stored hash at `counter` of `log`, if held (for the Have-time
/// comparison of ht-5fr.6).
pub fn stored_hash(
    conn: &Connection,
    log: &LogKey,
    counter: u64,
) -> Result<Option<[u8; 32]>, ApiError> {
    match load_log(conn, log)? {
        Some(row) => hash_at(conn, row.log_ix, counter),
        None => Ok(None),
    }
}

/// Committed entries of `log` from counter `from`, on a query connection.
///
/// Stops at `limits.entries` entries, or before the entry that would take the
/// canonical size past `limits.bytes`; but always returns at least one entry
/// when `from <= contiguous`, even an oversized one, so a peer can always make
/// progress. An unknown log, or `from` beyond the contiguous head, gives an
/// empty vec. Stops before the first opaque row without decoding it, so the
/// result is a known prefix; [`serve_entries`] serves opaque rows too.
pub fn serve(
    conn: &Connection,
    log: &LogKey,
    from: u64,
    limits: BatchLimits,
) -> Result<Vec<Envelope>, ApiError> {
    known_only(serve_rows(conn, log, from, limits, Rows::KnownPrefix)?)
}

/// [`serve`] over every chain position, known and opaque, byte-exact.
pub fn serve_entries(
    conn: &Connection,
    log: &LogKey,
    from: u64,
    limits: BatchLimits,
) -> Result<Vec<StoredEntry>, ApiError> {
    serve_rows(conn, log, from, limits, Rows::All)
}

fn serve_rows(
    conn: &Connection,
    log: &LogKey,
    from: u64,
    limits: BatchLimits,
    rows: Rows,
) -> Result<Vec<StoredEntry>, ApiError> {
    let Some(row) = load_log(conn, log)? else {
        return Ok(Vec::new());
    };
    let from = from.max(1);
    if from > row.held.contiguous {
        return Ok(Vec::new());
    }
    let mut batch = Batch::new(limits);
    entries_from(conn, row.log_ix, log, from, None, rows, &mut batch)?;
    Ok(batch.out)
}

/// The known entries of a [`Rows::KnownPrefix`] read.
fn known_only(entries: Vec<StoredEntry>) -> Result<Vec<Envelope>, ApiError> {
    entries
        .into_iter()
        .map(|entry| match entry {
            StoredEntry::Known(envelope) => Ok(envelope),
            StoredEntry::Opaque(_) => Err(corrupt("journal invariant: opaque entry in known read")),
        })
        .collect()
}

/// Most `journal_forked` rows [`serve_branch`] walks before it answers `None`.
pub const BRANCH_WALK_MAX_STEPS: u64 = 65_536;

/// Accumulates a served batch under [`BatchLimits`]: at most `entries`
/// entries, stopping before the entry that would take the canonical size past
/// `bytes`, but always at least one entry, even an oversized one. Once it
/// refuses an entry it refuses every later one, so a batch never skips a
/// position.
pub(crate) struct Batch {
    limits: BatchLimits,
    out: Vec<StoredEntry>,
    bytes: usize,
    closed: bool,
}

impl Batch {
    fn new(limits: BatchLimits) -> Self {
        Self {
            limits,
            out: Vec::new(),
            bytes: 0,
            closed: false,
        }
    }

    /// Adds `entry` unless a limit is hit; `false` means the batch is full.
    fn push(&mut self, entry: StoredEntry) -> bool {
        if self.closed || self.out.len() >= self.limits.entries.max(1) {
            self.closed = true;
            return false;
        }
        let size = entry.size();
        if !self.out.is_empty() && self.bytes.saturating_add(size) > self.limits.bytes {
            self.closed = true;
            return false;
        }
        self.bytes = self.bytes.saturating_add(size);
        self.out.push(entry);
        true
    }
}

/// The Chain-row reader: adds the chain positions `from..=to` (or to the end)
/// of `log_ix` to `batch`, in counter order, as [`StoredEntry`], until the
/// batch is full. With [`Rows::KnownPrefix`] it also stops, closing the
/// batch, at the first opaque row, before reading that row's blobs.
///
/// The only functions in this file that read a foreign row's raw bytes are this
/// one, [`entry_at_position`] (through it) and [`branch_entry`]; the writers are
/// [`insert_parked_layout`] and `insert_forked`.
pub(crate) fn entries_from(
    conn: &Connection,
    log_ix: i64,
    log: &LogKey,
    from: u64,
    to: Option<u64>,
    rows: Rows,
    batch: &mut Batch,
) -> Result<(), ApiError> {
    let mut statement = conn
        .prepare(&format!(
            "{} WHERE t.log_ix=?1 AND t.counter>=?2 AND t.counter<=?3 ORDER BY t.counter",
            row_select("journal", rows)
        ))
        .map_err(store_error)?;
    let found = statement
        .query_map(
            params![
                log_ix,
                i64::try_from(from).unwrap_or(i64::MAX),
                to.map_or(i64::MAX, |to| i64::try_from(to).unwrap_or(i64::MAX))
            ],
            StoredRow::read,
        )
        .map_err(store_error)?;
    for raw in found {
        let Some(entry) = raw.map_err(store_error)?.decode(log, rows)? else {
            batch.closed = true;
            break;
        };
        if !batch.push(entry) {
            break;
        }
    }
    Ok(())
}

/// What [`branch_entry`] found.
enum BranchRead {
    Entry(Box<StoredEntry>),
    /// An opaque row under [`Rows::KnownPrefix`].
    Cut,
    Missing,
}

/// The fork-branch row `(counter, hash)` of `log_ix`: a `journal_forked` row
/// row, known or opaque.
fn branch_entry(
    conn: &Connection,
    log_ix: i64,
    log: &LogKey,
    counter: u64,
    hash: &[u8; 32],
    rows: Rows,
) -> Result<BranchRead, ApiError> {
    let raw = conn
        .query_row(
            &format!(
                "{} WHERE t.log_ix=?1 AND t.counter=?2 AND t.hash=?3",
                row_select("journal_forked", rows)
            ),
            params![
                log_ix,
                i64::try_from(counter).unwrap_or(i64::MAX),
                hash.as_slice()
            ],
            StoredRow::read,
        )
        .optional()
        .map_err(store_error)?;
    Ok(match raw {
        None => BranchRead::Missing,
        Some(raw) => raw
            .decode(log, rows)?
            .map_or(BranchRead::Cut, |entry| BranchRead::Entry(Box::new(entry))),
    })
}

/// The batch of the branch of `log` that ends at `tip`, starting at counter
/// `from`, on a query connection. Read-only.
///
/// 1. If `journal` holds `tip.0` with hash `tip.1`, the answer is the
///    `journal` rows `from..=tip.0`.
/// 2. Else, if `journal_forked` holds `(tip.0, tip.1)`, the branch is walked
///    backward by `prev_hash`: at each step the predecessor is a
///    `journal_forked` row or a `journal` row whose hash is the `prev_hash`.
///    The walk ends at `from` or where the branch joins `journal`; the answer
///    is the ascending branch from `from`. The walk reads at most
///    [`BRANCH_WALK_MAX_STEPS`] forked rows.
/// 3. Else `None`.
///
/// Either way the batch obeys `limits` like [`serve`] (the first batch; at
/// least one entry when the range is not empty). `None` means "cannot serve
/// this branch": an unknown log, an unknown tip, a broken chain or a walk
/// over the bound. `Some(vec![])` means the branch is known and nothing lies
/// in range (`from` beyond the tip). Like [`serve`], it stops before the
/// first opaque row, chain position or branch row; [`serve_branch_entries`]
/// serves those too.
pub fn serve_branch(
    conn: &Connection,
    log: &LogKey,
    tip: (u64, [u8; 32]),
    from: u64,
    limits: BatchLimits,
) -> Result<Option<Vec<Envelope>>, ApiError> {
    serve_branch_within(
        conn,
        log,
        tip,
        from,
        limits,
        BRANCH_WALK_MAX_STEPS,
        Rows::KnownPrefix,
    )?
    .map(known_only)
    .transpose()
}

/// [`serve_branch`] over known and opaque rows, byte-exact.
pub fn serve_branch_entries(
    conn: &Connection,
    log: &LogKey,
    tip: (u64, [u8; 32]),
    from: u64,
    limits: BatchLimits,
) -> Result<Option<Vec<StoredEntry>>, ApiError> {
    serve_branch_within(
        conn,
        log,
        tip,
        from,
        limits,
        BRANCH_WALK_MAX_STEPS,
        Rows::All,
    )
}

/// [`serve_branch`] with the walk bound and the row scope given.
fn serve_branch_within(
    conn: &Connection,
    log: &LogKey,
    tip: (u64, [u8; 32]),
    from: u64,
    limits: BatchLimits,
    max_steps: u64,
    rows: Rows,
) -> Result<Option<Vec<StoredEntry>>, ApiError> {
    let Some(row) = load_log(conn, log)? else {
        return Ok(None);
    };
    let (tip_counter, tip_hash) = tip;
    let from = from.max(1);
    let mut batch = Batch::new(limits);
    if hash_at(conn, row.log_ix, tip_counter)? == Some(tip_hash) {
        if from <= tip_counter {
            entries_from(
                conn,
                row.log_ix,
                log,
                from,
                Some(tip_counter),
                rows,
                &mut batch,
            )?;
        }
        return Ok(Some(batch.out));
    }
    let forked_prev = |counter: u64, hash: &[u8; 32]| branch_prev(conn, row.log_ix, counter, hash);
    // The forked part of the branch, tip first, as (counter, hash).
    let mut path: Vec<(u64, [u8; 32])> = Vec::new();
    let (mut counter, mut hash) = (tip_counter, tip_hash);
    // The last `journal` counter the branch shares, if the walk reaches it.
    let mut join: Option<u64> = None;
    loop {
        let Some(prev) = forked_prev(counter, &hash)? else {
            // The tip itself, or a link below it, is unknown.
            return Ok(None);
        };
        if path.len() as u64 >= max_steps {
            return Ok(None);
        }
        path.push((counter, hash));
        if counter <= from {
            break;
        }
        if hash_at(conn, row.log_ix, counter - 1)? == Some(prev) {
            join = Some(counter - 1);
            break;
        }
        (counter, hash) = (counter - 1, prev);
    }
    if let Some(join) = join {
        entries_from(conn, row.log_ix, log, from, Some(join), rows, &mut batch)?;
    }
    for (counter, hash) in path.into_iter().rev().filter(|(c, _)| *c >= from) {
        match branch_entry(conn, row.log_ix, log, counter, &hash, rows)? {
            BranchRead::Entry(entry) => {
                if !batch.push(*entry) {
                    break;
                }
            }
            BranchRead::Cut => break,
            BranchRead::Missing => {
                return Err(corrupt("journal invariant: branch row missing"));
            }
        }
    }
    Ok(Some(batch.out))
}

/// What a node knows about one log, from `sync_logs` and `local_node`, on a
/// query connection. Read-only. `None` for a log the store does not hold, as
/// opposed to a held log with nothing in it.
pub fn log_meta(conn: &Connection, log: &LogKey) -> Result<Option<LogMeta>, ApiError> {
    let Some(row) = load_log(conn, log)? else {
        return Ok(None);
    };
    let own: bool = conn
        .query_row(
            "SELECT is_self FROM sync_logs WHERE log_ix=?1",
            params![row.log_ix],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    Ok(Some(LogMeta {
        pubkey: row.held.pubkey,
        own,
        active: row.held.is_active_self,
        state: row.held.state,
        frozen_at: row.held.frozen_at,
        contiguous: row.held.contiguous,
        head: row.held.head_hash,
    }))
}

/// Free bytes on the filesystem holding `dir` (`f_bavail * f_frsize`), or
/// `None` when it cannot be read (then ingest proceeds; a real shortage
/// surfaces as SQLite `DiskFull`).
fn free_bytes(dir: &std::path::Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::zeroed();
    // SAFETY: `path` is a valid NUL-terminated string and `stats` is a
    // properly sized, writable `statvfs`; it is read only after the call
    // reports success.
    let status = unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) };
    if status != 0 {
        return None;
    }
    // SAFETY: `statvfs` returned 0, so it initialised the struct.
    let stats = unsafe { stats.assume_init() };
    #[allow(clippy::unnecessary_cast)]
    Some((stats.f_bavail as u64).saturating_mul(stats.f_frsize as u64))
}

/// Longest ingest batch, in entries.
const INGEST_BATCH_MAX: usize = 256;

/// Seconds between two skew lines for one log, in ms.
const SKEW_LOG_INTERVAL_MS: i64 = 3_600_000;

fn malformed_batch(log: &LogKey, counter: u64, field: &'static str) -> IngestOutcome {
    IngestOutcome::Corrupt {
        log: *log,
        counter,
        reason: EnvelopeError::Malformed { field },
    }
}

/// Verifies and stores a batch of `log`'s entries from a peer.
///
/// The caller holds a `FairWriter` background turn and passes the writer
/// connection. Outcomes, by what is left in the store:
/// - nothing stored: [`IngestOutcome::Corrupt`] (a damaged or forged entry,
///   a batch over 256 entries, an entry for another log; the whole
///   transaction rolls back, even entries before the bad one) and
///   [`IngestOutcome::Headroom`] (free disk space below the floor, checked
///   before any transaction opens; or SQLite `DiskFull` inside it, which also
///   rolls back);
/// - prefix kept: [`IngestOutcome::Gap`], [`IngestOutcome::Fork`] and
///   [`IngestOutcome::Frozen`] commit the entries stored before the stopping
///   one and report how many; [`IngestOutcome::Stored`] is the whole batch
///   stored or already held;
/// - `Err` is reserved for store errors (`StoreBusy`, `StoreCorrupt`,
///   cancellation), rolled back with nothing advanced.
///
/// Stored entries are `pending` and are applied later by the Apply lane.
/// Payloads are never parsed, except a genesis's `pubkey`; unknown kinds are
/// stored unchanged. Forks are reported, never resolved or recorded here
/// (`store_forked` and `freeze` are for ht-5fr.6).
pub fn ingest(
    ctx: &StoreContext,
    conn: &mut Connection,
    log: &LogKey,
    entries: Vec<Envelope>,
    budget: &CallBudget,
) -> Result<IngestOutcome, ApiError> {
    let entries = entries.into_iter().map(StoredEntry::from).collect();
    ingest_entries(ctx, conn, log, entries, budget)
}

/// [`ingest`] over [`StoredEntry`]: an unknown-layout append is stored in
/// `journal` with an empty projection and `apply_state = 'parked_layout'`, moves
/// the head and does not fold the HLC. Fork evidence carries both sides as
/// stored, known or opaque.
pub fn ingest_entries(
    ctx: &StoreContext,
    conn: &mut Connection,
    log: &LogKey,
    entries: Vec<StoredEntry>,
    budget: &CallBudget,
) -> Result<IngestOutcome, ApiError> {
    let sync = ctx.sync()?;
    if budget.is_exhausted(ctx.clock()) {
        return Err(if budget.cancellation.is_cancelled() {
            api_error(ErrorCode::Cancelled, "ingest cancelled")
        } else {
            api_error(ErrorCode::DeadlineExceeded, "ingest deadline exceeded")
        });
    }
    if let Some(free) = free_bytes(&ctx.database_dir())
        && free < sync.headroom_bytes()
    {
        return Ok(IngestOutcome::Headroom);
    }
    if entries.len() > INGEST_BATCH_MAX {
        return Ok(malformed_batch(log, 0, "batch"));
    }
    if let Some(stray) = entries.iter().find(|entry| entry.log() != *log) {
        return Ok(malformed_batch(log, stray.counter(), "log"));
    }
    match ingest_batch(ctx, sync, conn, log, &entries) {
        Err(error) if error.code == ErrorCode::StoreFull => Ok(IngestOutcome::Headroom),
        other => other,
    }
}

fn ingest_batch(
    ctx: &StoreContext,
    sync: &SyncState,
    conn: &mut Connection,
    log: &LogKey,
    entries: &[StoredEntry],
) -> Result<IngestOutcome, ApiError> {
    let wall = ctx.clock().utc_now();
    let decision = DecisionInstant {
        utc: wall,
        monotonic: ctx.clock().monotonic_now(),
    };
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(store_error)?;
    let mut row = load_log(&tx, log)?;
    let (mut stored, mut duplicates) = (0usize, 0usize);
    let mut max_ahead: Option<u64> = None;
    let mut stop: Option<IngestOutcome> = None;
    for entry in entries {
        let step = {
            let view = match row.as_mut() {
                Some(row) => {
                    row.held.stored_hash = if entry.counter() <= row.held.contiguous {
                        hash_at(&tx, row.log_ix, entry.counter())?
                    } else {
                        None
                    };
                    Some(&row.held)
                }
                None => None,
            };
            check_next(view, entry)
        };
        match step {
            ChainStep::Duplicate => duplicates += 1,
            ChainStep::Corrupt(reason) => {
                return Ok(IngestOutcome::Corrupt {
                    log: *log,
                    counter: entry.counter(),
                    reason,
                });
            }
            ChainStep::AheadOfSelf => {
                let Some(row) = row.as_ref() else {
                    return Err(corrupt("journal invariant: self log without a row"));
                };
                stop = Some(IngestOutcome::Fork {
                    stored,
                    evidence: ForkEvidence::AheadOfSelf {
                        log: *log,
                        my_head: row.head(),
                        entry: Box::new(entry.clone()),
                    },
                });
                break;
            }
            ChainStep::Gap { expected } => {
                stop = Some(IngestOutcome::Gap { stored, expected });
                break;
            }
            ChainStep::BeyondFrozen => {
                let frozen_at = row.as_ref().and_then(|row| row.held.frozen_at).unwrap_or(0);
                stop = Some(IngestOutcome::Frozen { stored, frozen_at });
                break;
            }
            ChainStep::Conflict => {
                let Some(row) = row.as_ref() else {
                    return Err(corrupt("journal invariant: conflict without a held log"));
                };
                let held_counter = if entry.counter() <= row.held.contiguous {
                    entry.counter()
                } else {
                    entry.counter() - 1
                };
                stop = Some(IngestOutcome::Fork {
                    stored,
                    evidence: ForkEvidence::Conflict {
                        held: Box::new(entry_at_position(&tx, row.log_ix, log, held_counter)?),
                        conflicting: Box::new(entry.clone()),
                    },
                });
                break;
            }
            step @ (ChainStep::Genesis | ChainStep::Append) => {
                match entry {
                    StoredEntry::Known(envelope) => {
                        // The chain check proved hash and signature; the shape is the
                        // sender's claim, so refuse what the table cannot hold.
                        if let Err(reason) = envelope.validate_shape() {
                            return Ok(IngestOutcome::Corrupt {
                                log: *log,
                                counter: envelope.counter,
                                reason,
                            });
                        }
                        if step == ChainStep::Genesis {
                            let pubkey = match envelope.genesis_pubkey() {
                                Ok(pubkey) => pubkey,
                                Err(reason) => {
                                    return Ok(IngestOutcome::Corrupt {
                                        log: *log,
                                        counter: envelope.counter,
                                        reason,
                                    });
                                }
                            };
                            row = Some(insert_log(&tx, log, &pubkey, envelope, wall)?);
                        }
                        let Some(row) = row.as_mut() else {
                            return Err(corrupt("journal invariant: append without a log row"));
                        };
                        insert_pending(&tx, row.log_ix, envelope, decision)?;
                        row.held.contiguous = envelope.counter;
                        row.held.head_hash = envelope.hash;
                        let fold = sync
                            .hlc
                            .lock()
                            .map_err(|_| corrupt("hlc poisoned"))?
                            .fold(wall, envelope.hlc);
                        if let Fold::Skewed { ahead_ms } = fold {
                            max_ahead = Some(max_ahead.map_or(ahead_ms, |seen| seen.max(ahead_ms)));
                        }
                    }
                    StoredEntry::Opaque(opaque) => {
                        // `check_next` refuses an opaque genesis, so this is an append.
                        let (ChainStep::Append, Some(row)) = (step, row.as_mut()) else {
                            return Err(corrupt("journal invariant: opaque genesis"));
                        };
                        insert_parked_layout(&tx, row.log_ix, opaque, decision.utc.0)?;
                        row.held.contiguous = opaque.counter();
                        row.held.head_hash = opaque.hash();
                        // No HLC fold: an opaque entry has no readable hlc.
                    }
                }
                stored += 1;
            }
        }
    }
    if let Some(row) = row.as_ref() {
        if stored > 0 {
            tx.execute(
                "UPDATE sync_logs SET contiguous_counter=?2, head_hash=?3 WHERE log_ix=?1",
                params![
                    row.log_ix,
                    i64::try_from(row.held.contiguous).unwrap_or(i64::MAX),
                    row.held.head_hash.as_slice()
                ],
            )
            .map_err(store_error)?;
        }
        let skew = i64::try_from(max_ahead.unwrap_or(0)).unwrap_or(i64::MAX);
        tx.execute(
            "UPDATE sync_logs SET skew_ahead_ms=?2 WHERE log_ix=?1 AND skew_ahead_ms<>?2",
            params![row.log_ix, skew],
        )
        .map_err(store_error)?;
    }
    let head = row.as_ref().map_or(
        LogHead {
            counter: 0,
            head_hash: [0u8; 32],
            state: LogState::Active,
            frozen_at: None,
        },
        LogRow::head,
    );
    tx.commit().map_err(store_error)?;
    if stored > 0 {
        // After the commit, before the writer turn drops and kicks the Apply
        // lane, so the kicked pass sees it. The epoch moves first: a pass that
        // reads the new epoch also sees the committed rows.
        sync.apply_epochs().note_ingested();
        sync.set_apply_hint();
    }
    if let Some(ahead_ms) = max_ahead {
        log_skew(sync, log, ahead_ms, wall);
    }
    Ok(stop.unwrap_or(IngestOutcome::Stored {
        stored,
        duplicates,
        head,
    }))
}

/// Inserts the `sync_logs` row a verified genesis announces, empty, so the
/// entry then goes in through the ordinary store path.
fn insert_log(
    tx: &Transaction<'_>,
    log: &LogKey,
    pubkey: &[u8; 32],
    genesis: &Envelope,
    wall: UtcMillis,
) -> Result<LogRow, ApiError> {
    let local: String = tx
        .query_row("SELECT id FROM local_node", [], |r| r.get(0))
        .map_err(store_error)?;
    let node = log.node.to_string();
    tx.execute(
        "INSERT INTO sync_logs(node_uuid,incarnation,is_self,pubkey,genesis_hash,contiguous_counter,head_hash,first_seen_at) \
         VALUES (?1,?2,?3,?4,?5,0,?6,?7)",
        params![
            node,
            log.incarnation,
            node == local,
            pubkey.as_slice(),
            genesis.hash.as_slice(),
            [0u8; 32].as_slice(),
            wall.0
        ],
    )
    .map_err(store_error)?;
    Ok(LogRow {
        log_ix: tx.last_insert_rowid(),
        held: HeldLog {
            pubkey: *pubkey,
            contiguous: 0,
            head_hash: [0u8; 32],
            state: LogState::Active,
            frozen_at: None,
            is_active_self: false,
            stored_hash: None,
        },
    })
}

/// One daemon-log line per log per hour when ingest refused to absorb a
/// skewed stamp.
fn log_skew(sync: &SyncState, log: &LogKey, ahead_ms: u64, wall: UtcMillis) {
    let Ok(mut logged) = sync.skew_logged.lock() else {
        return;
    };
    if logged
        .get(log)
        .is_some_and(|last| wall.0.saturating_sub(*last) < SKEW_LOG_INTERVAL_MS)
    {
        return;
    }
    logged.insert(*log, wall.0);
    eprintln!(
        "sync: log {} hlc ahead of wall by {ahead_ms} ms; not absorbed",
        log_label(&log.node.to_string(), i64::from(log.incarnation))
    );
}

/// Records verified entries of a forked branch of `log` in `journal_forked`,
/// ignoring exact duplicates (`UNIQUE(log_ix, counter, hash)`), and returns
/// how many were new. `source` is the node they came from, if known. The
/// log must be known (`NotFound` otherwise). For ht-5fr.6; ingest never calls
/// it, because the journal reports forks and does not resolve them.
pub fn store_forked(
    tx: &Transaction<'_>,
    log: &LogKey,
    entries: &[Envelope],
    source: Option<NodeUuid>,
) -> Result<usize, ApiError> {
    let entries: Vec<StoredEntry> = entries.iter().cloned().map(StoredEntry::from).collect();
    store_forked_entries(tx, log, &entries, source)
}

/// [`store_forked`] over [`StoredEntry`]: an unknown-layout side goes to
/// `journal_forked` with an empty projection.
pub fn store_forked_entries(
    tx: &Transaction<'_>,
    log: &LogKey,
    entries: &[StoredEntry],
    source: Option<NodeUuid>,
) -> Result<usize, ApiError> {
    let Some(row) = load_log(tx, log)? else {
        return Err(api_error(ErrorCode::NotFound, "unknown log"));
    };
    let source = source.map(|node| node.to_string());
    let mut inserted = 0;
    for entry in entries {
        if entry.log() != *log {
            return Err(api_error(
                ErrorCode::InvalidRequest,
                "forked entry names another log",
            ));
        }
        inserted += usize::from(insert_forked(tx, row.log_ix, entry, source.as_deref())?);
    }
    Ok(inserted)
}

/// Stores one verified fork-branch entry of `log_ix` in `journal_forked`,
/// ignoring an exact duplicate, and returns whether the row is new. A known
/// entry carries its projection and `raw = canonical_bytes()`; an unknown-layout
/// one only its header and `raw`.
fn insert_forked(
    tx: &Transaction<'_>,
    log_ix: i64,
    entry: &StoredEntry,
    source: Option<&str>,
) -> Result<bool, ApiError> {
    let new = match entry {
        StoredEntry::Known(entry) => {
            let needs = entry
                .needs
                .iter()
                .map(EntityKey::as_str)
                .collect::<Vec<_>>()
                .join("\n");
            tx.execute(
                "INSERT OR IGNORE INTO journal_forked(log_ix,counter,prev_hash,hash,hlc,kind,kind_version,deps,creates,needs,payload,signature,source_node,received_at,raw) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,CAST(strftime('%s','now') AS INTEGER)*1000,?14)",
                params![
                    log_ix,
                    i64::try_from(entry.counter).unwrap_or(i64::MAX),
                    entry.prev_hash.as_slice(),
                    entry.hash.as_slice(),
                    i64::try_from(entry.hlc.as_u64()).unwrap_or(i64::MAX),
                    entry.kind,
                    entry.kind_version,
                    entry.deps.encode(),
                    entry.creates.as_ref().map(EntityKey::as_str),
                    needs,
                    entry.payload,
                    entry.signature.as_slice(),
                    source,
                    entry.canonical_bytes(),
                ],
            )
        }
        StoredEntry::Opaque(opaque) => tx.execute(
            "INSERT OR IGNORE INTO journal_forked(log_ix,counter,prev_hash,hash,signature,raw,needs,source_node,received_at) \
             VALUES (?1,?2,?3,?4,?5,?6,NULL,?7,CAST(strftime('%s','now') AS INTEGER)*1000)",
            params![
                log_ix,
                i64::try_from(opaque.counter()).unwrap_or(i64::MAX),
                opaque.prev_hash().as_slice(),
                opaque.hash().as_slice(),
                opaque.signature().as_slice(),
                opaque.raw(),
                source,
            ],
        ),
    }
    .map_err(store_error)?;
    Ok(new == 1)
}

/// Stores a verified unknown-layout entry as chain position `counter` of
/// `log_ix`: a `journal` row with an empty projection and `apply_state =
/// 'parked_layout'`. A plain INSERT (`check_next` proved the position free;
/// `UNIQUE(log_ix, counter)` guards it). `ingested_at` is the ingestion time.
pub(crate) fn insert_parked_layout(
    tx: &Connection,
    log_ix: i64,
    entry: &OpaqueEntry,
    ingested_at: i64,
) -> Result<(), ApiError> {
    tx.execute(
        "INSERT INTO journal(log_ix,counter,prev_hash,hash,signature,raw,needs,ingested_at,apply_state) \
         VALUES (?1,?2,?3,?4,?5,?6,NULL,?7,'parked_layout')",
        params![
            log_ix,
            i64::try_from(entry.counter()).unwrap_or(i64::MAX),
            entry.prev_hash().as_slice(),
            entry.hash().as_slice(),
            entry.signature().as_slice(),
            entry.raw(),
            ingested_at,
        ],
    )
    .map_err(store_error)?;
    Ok(())
}

/// What [`promote`] did to one `journal` row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Promoted {
    /// Projection filled, `parked_layout` -> `pending`.
    Done,
    /// This build has no parser for the row's magic: unchanged.
    NotUnderstood,
    /// Not an empty-projection `parked_layout` row (already promoted, or
    /// known): unchanged.
    NotEligible,
    /// Parse, shape, consistency or trigger failure: rolled back, the row
    /// stays `parked_layout`.
    Failed(String),
}

/// Promotes one `journal` row (never `journal_forked`) in the caller's
/// transaction, inside its own SAVEPOINT so a failure leaves nothing behind.
///
/// Fills the projection from the layout parser this build registered for the
/// row's magic, moves `parked_layout` -> `pending` in the same statement and,
/// when the row is its log's next position, clears `blocked_on =
/// opaque_layout`. `raw`, `hash` and `signature` are never written. A row
/// that is not an empty-projection `parked_layout` row is [`Promoted::NotEligible`],
/// so promotion is idempotent.
pub(crate) fn promote(tx: &Connection, ingest_seq: i64) -> Result<Promoted, ApiError> {
    tx.execute_batch("SAVEPOINT promote").map_err(store_error)?;
    let outcome = match promote_in_savepoint(tx, ingest_seq) {
        Ok(outcome) => outcome,
        Err(e) => {
            let _ = tx.execute_batch("ROLLBACK TO promote; RELEASE promote");
            return Err(e);
        }
    };
    if matches!(outcome, Promoted::Failed(_)) {
        tx.execute_batch("ROLLBACK TO promote")
            .map_err(store_error)?;
    }
    tx.execute_batch("RELEASE promote").map_err(store_error)?;
    Ok(outcome)
}

fn promote_in_savepoint(tx: &Connection, ingest_seq: i64) -> Result<Promoted, ApiError> {
    type Row = (
        i64,
        String,
        i64,
        i64,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        bool,
        String,
        i64,
    );
    let row: Option<Row> = tx
        .query_row(
            "SELECT j.log_ix,l.node_uuid,l.incarnation,j.counter,j.prev_hash,j.hash,j.signature, \
                    j.raw,j.kind IS NULL,j.apply_state,l.applied_through \
             FROM journal j JOIN sync_logs l ON l.log_ix=j.log_ix WHERE j.ingest_seq=?1",
            [ingest_seq],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                    r.get(10)?,
                ))
            },
        )
        .optional()
        .map_err(store_error)?;
    let Some((
        log_ix,
        node,
        incarnation,
        counter,
        prev,
        hash,
        signature,
        raw,
        empty,
        state,
        applied,
    )) = row
    else {
        return Ok(Promoted::NotEligible);
    };
    if !empty || state != "parked_layout" {
        return Ok(Promoted::NotEligible);
    }
    let Some(magic) = raw.first_chunk::<8>() else {
        return Ok(Promoted::Failed("raw shorter than the magic".into()));
    };
    let Some(parser) = crate::sync::envelope::understood_layout(magic) else {
        return Ok(Promoted::NotUnderstood);
    };
    let fail = |what: &str| Ok(Promoted::Failed(what.to_owned()));
    let (Ok(node), Ok(incarnation), Ok(counter_u)) = (
        node.parse::<NodeUuid>(),
        u32::try_from(incarnation),
        u64::try_from(counter),
    ) else {
        return Err(corrupt("journal invariant: malformed log row"));
    };
    let log = LogKey { node, incarnation };
    let (Ok(prev_hash), Ok(hash), Ok(signature)) = (
        <[u8; 32]>::try_from(prev),
        <[u8; 32]>::try_from(hash),
        <[u8; 64]>::try_from(signature),
    ) else {
        return Err(corrupt("journal invariant: hash or signature length"));
    };
    let header = OpaqueHeader {
        log,
        counter: counter_u,
        prev_hash,
        hash,
        signature,
    };
    let entry = match OpaqueEntry::new(header, raw) {
        Ok(entry) => entry,
        Err(e) => return fail(&format!("shape: {e}")),
    };
    let envelope = match parser(&entry) {
        Ok(envelope) => envelope,
        Err(e) => return fail(&format!("parse: {e}")),
    };
    if let Err(e) = envelope.validate_shape() {
        return fail(&format!("envelope: {e}"));
    }
    if envelope.log != log
        || envelope.counter != counter_u
        || envelope.prev_hash != prev_hash
        || envelope.hash != hash
        || envelope.signature != signature
    {
        return fail("parsed envelope does not match the stored header");
    }
    let needs = envelope
        .needs
        .iter()
        .map(EntityKey::as_str)
        .collect::<Vec<_>>()
        .join("\n");
    let updated = tx.execute(
        "UPDATE journal SET hlc=?2,kind=?3,kind_version=?4,deps=?5,creates=?6,needs=?7,payload=?8, \
         apply_state='pending' WHERE ingest_seq=?1 AND kind IS NULL AND apply_state='parked_layout'",
        params![
            ingest_seq,
            i64::try_from(envelope.hlc.as_u64()).unwrap_or(i64::MAX),
            envelope.kind,
            envelope.kind_version,
            envelope.deps.encode(),
            envelope.creates.as_ref().map(EntityKey::as_str),
            needs,
            envelope.payload,
        ],
    );
    match updated {
        Ok(1) => {}
        Ok(_) => return Ok(Promoted::NotEligible),
        Err(e) => return fail(&format!("store: {e}")),
    }
    if counter == applied + 1 {
        tx.execute(
            "UPDATE sync_logs SET blocked_on=NULL WHERE log_ix=?1 AND blocked_on='opaque_layout'",
            [log_ix],
        )
        .map_err(store_error)?;
    }
    Ok(Promoted::Done)
}

/// The parked rows in `ingest_seq` order. The redundant `<> 'applied'` lets
/// SQLite use the partial index `journal_unapplied`, which it cannot prove
/// from `= 'parked_layout'` alone.
const PARKED_SQL: &str = "SELECT j.ingest_seq,substr(j.raw,1,8) FROM journal j \
     WHERE j.apply_state<>'applied' AND j.apply_state='parked_layout' ORDER BY j.ingest_seq";

/// Promotes every `parked_layout` row whose magic this build understands, in
/// `ingest_seq` order; returns `(promoted, failed)`. Run at store open and at
/// the start of every apply pass. A row-level failure never fails the caller:
/// the row stays parked and its reason is logged once per process. Production
/// builds understand no foreign layout, so the pass returns before touching the
/// store; otherwise the scan is an index search on the partial
/// `journal_unapplied` ([`PARKED_SQL`]).
pub(crate) fn promote_parked(
    tx: &Connection,
    logged: &std::sync::Mutex<std::collections::HashSet<i64>>,
) -> Result<(usize, usize), ApiError> {
    if !crate::sync::envelope::any_layout_understood() {
        return Ok((0, 0));
    }
    let mut statement = tx.prepare(PARKED_SQL).map_err(store_error)?;
    let parked = statement
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)))
        .map_err(store_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(store_error)?;
    drop(statement);
    let (mut promoted, mut failed) = (0, 0);
    for (ingest_seq, magic) in parked {
        let understood = <[u8; 8]>::try_from(magic.as_slice())
            .ok()
            .is_some_and(|m| crate::sync::envelope::understood_layout(&m).is_some());
        if !understood {
            continue;
        }
        match promote(tx, ingest_seq)? {
            Promoted::Done => promoted += 1,
            Promoted::Failed(reason) => {
                failed += 1;
                log_cannot_promote(tx, logged, ingest_seq, &magic, &reason);
            }
            Promoted::NotUnderstood | Promoted::NotEligible => {}
        }
    }
    Ok((promoted, failed))
}

/// One daemon-log line per row per process for a failed promotion.
fn log_cannot_promote(
    tx: &Connection,
    logged: &std::sync::Mutex<std::collections::HashSet<i64>>,
    ingest_seq: i64,
    magic: &[u8],
    reason: &str,
) {
    let Ok(mut logged) = logged.lock() else {
        return;
    };
    if !logged.insert(ingest_seq) {
        return;
    }
    let at: Option<(String, i64, i64)> = tx
        .query_row(
            "SELECT l.node_uuid,l.incarnation,j.counter FROM journal j \
             JOIN sync_logs l ON l.log_ix=j.log_ix WHERE j.ingest_seq=?1",
            [ingest_seq],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .ok()
        .flatten();
    let (label, counter) = at.map_or_else(
        || (format!("seq{ingest_seq}"), 0),
        |(node, inc, counter)| (log_label(&node, inc), counter),
    );
    let reason: String = reason.chars().take(80).collect();
    eprintln!(
        "sync: cannot promote journal entry {label}@{counter} (layout {}): {reason}",
        String::from_utf8_lossy(magic).escape_debug()
    );
}

/// Freezes `log` at counter `at`, lower-only: an active log becomes frozen
/// ([`FreezeOutcome::Frozen`]), a frozen log's counter is lowered
/// ([`FreezeOutcome::Lowered`]), and a request at or above the current
/// freeze changes nothing ([`FreezeOutcome::Unchanged`]). Refuses the active
/// self log (`Conflict "cannot freeze the active self log"`, invariant I1:
/// the owner rotates before it freezes) and an unknown log (`NotFound`). For
/// ht-5fr.6; ingest never calls it.
pub fn freeze(tx: &Transaction<'_>, log: &LogKey, at: u64) -> Result<FreezeOutcome, ApiError> {
    let Some(row) = load_log(tx, log)? else {
        return Err(api_error(ErrorCode::NotFound, "unknown log"));
    };
    if row.held.is_active_self {
        return Err(api_error(
            ErrorCode::Conflict,
            "cannot freeze the active self log",
        ));
    }
    let at = i64::try_from(at)
        .map_err(|_| api_error(ErrorCode::InvalidRequest, "freeze counter out of range"))?;
    let outcome = match (row.held.state, row.held.frozen_at) {
        (LogState::Active, _) => FreezeOutcome::Frozen,
        (LogState::Frozen, Some(current)) if u64::try_from(at).is_ok_and(|at| at < current) => {
            FreezeOutcome::Lowered
        }
        (LogState::Frozen, _) => return Ok(FreezeOutcome::Unchanged),
    };
    tx.execute(
        "UPDATE sync_logs SET state='frozen', frozen_at_counter=?2 WHERE log_ix=?1",
        params![row.log_ix, at],
    )
    .map_err(store_error)?;
    Ok(outcome)
}

/// The sync layer's view of the store: reads on query connections, writes on
/// the serialized writer. Callers hold a `FairWriter` background turn for
/// `ingest` and `apply_pending`.
impl crate::sync::journal_port::JournalPort for crate::store::SqliteStore {
    fn vector(&self, budget: &CallBudget) -> Result<Vector, ApiError> {
        let db = self.context.open_query(budget.clone())?;
        vector(&db)
    }

    fn stored_hash(
        &self,
        log: &LogKey,
        counter: u64,
        budget: &CallBudget,
    ) -> Result<Option<[u8; 32]>, ApiError> {
        let db = self.context.open_query(budget.clone())?;
        stored_hash(&db, log, counter)
    }

    fn serve(
        &self,
        log: &LogKey,
        from: u64,
        limits: BatchLimits,
        budget: &CallBudget,
    ) -> Result<Vec<StoredEntry>, ApiError> {
        let db = self.context.open_query(budget.clone())?;
        serve_entries(&db, log, from, limits)
    }

    fn serve_branch(
        &self,
        log: &LogKey,
        tip: (u64, [u8; 32]),
        from: u64,
        limits: BatchLimits,
        budget: &CallBudget,
    ) -> Result<Option<Vec<StoredEntry>>, ApiError> {
        let db = self.context.open_query(budget.clone())?;
        let bound = self
            .context
            .sync()
            .map_or(BRANCH_WALK_MAX_STEPS, SyncState::branch_walk_bound);
        serve_branch_within(&db, log, tip, from, limits, bound, Rows::All)
    }

    fn log_meta(&self, log: &LogKey, budget: &CallBudget) -> Result<Option<LogMeta>, ApiError> {
        let db = self.context.open_query(budget.clone())?;
        log_meta(&db, log)
    }

    /// `ingest` opens its own `BEGIN IMMEDIATE` (it keeps a valid prefix on a
    /// stop), so it runs on the writer turn directly rather than inside
    /// `execute_budgeted_decision`.
    fn ingest(
        &self,
        log: &LogKey,
        entries: Vec<StoredEntry>,
        budget: &CallBudget,
    ) -> Result<IngestOutcome, ApiError> {
        let mut writer = self.writer(budget)?;
        ingest_entries(&self.context, &mut writer, log, entries, budget)
    }

    fn apply_pending(&self, budget: &CallBudget) -> Result<ApplyProgress, ApiError> {
        let mut writer = self.writer(budget)?;
        self.context.execute_budgeted_decision(
            &mut writer,
            budget,
            |_| Ok(()),
            |tx, decision, ()| {
                apply::apply_pass(tx, &self.context, decision, apply::ApplyLimits::default())
            },
        )
    }
}

/// What the Apply lane worker needs beyond [`JournalPort`](crate::sync::journal_port::JournalPort):
/// the startup replay, the apply hint and a passive WAL checkpoint. The worker
/// holds the concrete store, not the sync seam. Callers hold a `FairWriter`
/// background turn for `replay_startup` and `checkpoint_passive`.
impl crate::store::SqliteStore {
    /// One bounded `ReplayScope::Startup` pass: `parked_unknown` entries the
    /// registry now knows and `parked_dep` entries whose creator applied or that
    /// wait for a host while one is attached.
    pub fn replay_startup(&self, budget: &CallBudget) -> Result<ApplyProgress, ApiError> {
        let mut writer = self.writer(budget)?;
        self.context.execute_budgeted_decision(
            &mut writer,
            budget,
            |_| Ok(()),
            |tx, decision, ()| {
                apply::replay(
                    tx,
                    &self.context,
                    decision,
                    apply::ReplayScope::Startup,
                    apply::ApplyLimits::default(),
                )
            },
        )
    }

    /// Clears the apply hint and returns whether it was set (`false` before
    /// the journal opens).
    pub fn take_apply_hint(&self) -> bool {
        self.context.sync().is_ok_and(|sync| sync.take_apply_hint())
    }

    /// Marks entries as possibly waiting to apply.
    pub fn set_apply_hint(&self) {
        if let Ok(sync) = self.context.sync() {
            sync.set_apply_hint();
        }
    }

    /// `PRAGMA wal_checkpoint(PASSIVE)` on the writer connection: copies WAL
    /// frames into the database without blocking readers or the writer.
    pub fn checkpoint_passive(&self, budget: &CallBudget) -> Result<(), ApiError> {
        let writer = self.writer(budget)?;
        writer
            .query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |_| Ok(()))
            .map_err(store_error)?;
        #[cfg(any(test, feature = "test-support"))]
        self.checkpoints
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

#[cfg(test)]
#[path = "../../tests/store/journal.rs"]
mod tests;
