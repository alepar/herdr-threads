//! The apply side of the journal: the context a fact handler runs with, and
//! the single-entry apply path.
//!
//! This file holds the handler-facing contract ([`ApplyContext`],
//! [`Provenance`]) and ApplyEngine's single-entry path ([`apply_one`],
//! [`apply_authored`]): registry lookup, the `needs` check, a savepoint, one
//! local position, the handler and the recorded outcome. The pass loop,
//! replay and the DB-backed context methods (`provenance`, `is_local_seat`,
//! `delivery_suppressed`) land with ht-5fr.3.8. Journal and this module call
//! each other by design: Journal authors and hands the new row here in the
//! same transaction; this module never inserts or reads chain state beyond the
//! entry it applies.
//!
//! Responsibility (single-entry path)
//! - Owns: turning one stored `journal` row into an outcome (`applied`,
//!   `parked_unknown`, `parked_dep`, `failed`), the row's `apply_*` columns
//!   and, for a head entry, `sync_logs.applied_through`/`blocked_on`.
//! - Must not: append or alter envelopes (Journal), record `failed` for a
//!   locally authored entry (the caller rolls back instead), or let a handler
//!   author directly.
//! - Role: the one place a fact is materialized. New outcome kinds or apply
//!   modes go here.

use std::marker::PhantomData;

use rusqlite::{OptionalExtension, Transaction, params};

use crate::node::NodeUuid;
use crate::protocol::results::{ApiError, ErrorCode};
use crate::store::connection::{DecisionInstant, StoreContext, SyncState, api_error, store_error};
use crate::sync::LogKey;
use crate::sync::facts::{FactError, FactKind};
use crate::sync::hlc::HlcStamp;

/// Where an applied entry came from, as handlers and attribution see it.
///
/// Closed set. Any incarnation of this node's own log is `Local`; only an
/// entry authored by another node is `ReplicatedFrom`. The `as_str` text
/// lands with ApplyEngine (ht-5fr.3.8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provenance {
    Local,
    ReplicatedFrom(NodeUuid),
}

/// A fact queued by a handler through [`ApplyContext::follow_up`]: kind name,
/// major and the serialized payload. ApplyEngine authors it after the
/// triggering entry is recorded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueuedFact {
    pub kind: &'static str,
    pub major: u32,
    pub payload: Vec<u8>,
}

/// What a fact handler may read and do while its entry is applied.
///
/// Owns: the facts about the entry being applied (origin log, counter,
/// `ingest_seq`, the origin's `hlc`, the local `decision` instant, the one
/// local `decision_seq` allocated for this entry, the local node), the
/// follow-up queue and the pending self-tag change. Handlers read these
/// fields and nothing else about the engine. Local positions come from
/// `decision_seq`, never from `next_decision_seq`: ApplyEngine allocates one
/// per entry, so replay and local order stay reproducible. Must not: let a
/// handler author through the journal directly (it queues with
/// [`follow_up`](Self::follow_up)), or carry a local position into a payload.
/// Role: passed `&mut` to `FactKind::apply`; built by ApplyEngine per entry.
pub struct ApplyContext<'a> {
    pub origin: LogKey,
    pub counter: u64,
    pub ingest_seq: i64,
    pub hlc: HlcStamp,
    pub decision: DecisionInstant,
    pub decision_seq: i64,
    pub local_node: NodeUuid,
    follow_ups: Vec<QueuedFact>,
    self_tag_k: Option<u32>,
    _entry: PhantomData<&'a ()>,
}

impl ApplyContext<'_> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        origin: LogKey,
        counter: u64,
        ingest_seq: i64,
        hlc: HlcStamp,
        decision: DecisionInstant,
        decision_seq: i64,
        local_node: NodeUuid,
    ) -> Self {
        Self {
            origin,
            counter,
            ingest_seq,
            hlc,
            decision,
            decision_seq,
            local_node,
            follow_ups: Vec::new(),
            self_tag_k: None,
            _entry: PhantomData,
        }
    }

    /// The origin's "sent at", in Unix ms.
    pub fn origin_ms(&self) -> u64 {
        self.hlc.ms()
    }

    /// True when the entry's origin node is this node, in any incarnation.
    pub fn is_local_origin(&self) -> bool {
        self.origin.node == self.local_node
    }

    /// Queues a typed fact. Only queues: ApplyEngine authors it after the
    /// triggering entry is recorded, so its deps include the trigger.
    pub fn follow_up<K: FactKind>(
        &mut self,
        payload: &K::Payload,
    ) -> Result<(), serde_json::Error> {
        self.follow_ups.push(QueuedFact {
            kind: K::NAME,
            major: K::MAJOR,
            payload: serde_json::to_vec(payload)?,
        });
        Ok(())
    }

    /// The queued follow-ups, in queue order.
    pub fn follow_ups(&self) -> &[QueuedFact] {
        &self.follow_ups
    }

    /// Takes the queued follow-ups, leaving the queue empty.
    pub fn take_follow_ups(&mut self) -> Vec<QueuedFact> {
        std::mem::take(&mut self.follow_ups)
    }

    /// Carries a tag change out to be applied after commit.
    pub fn set_self_tag_k(&mut self, k: u32) {
        self.self_tag_k = Some(k);
    }

    pub fn self_tag_k(&self) -> Option<u32> {
        self.self_tag_k
    }
}

/// How an entry's `applied_through` bookkeeping is handled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplyMode {
    /// The entry is the head of its log: a recorded outcome (even parked or
    /// failed) counts as processed and advances `applied_through`.
    Head,
    /// A re-attempt of an entry already counted in `applied_through`.
    Replay,
}

/// What applying one entry did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplyOutcome {
    Applied,
    ParkedUnknown,
    ParkedDep { on: String },
    Failed,
}

/// Longest `apply_error` kept, in bytes.
const APPLY_ERROR_MAX: usize = 512;

/// Applies the `journal` row `ingest_seq` and records the outcome.
///
/// A registry miss parks the entry (`parked_unknown`); an unapplied creator of
/// a needed key parks it (`parked_dep`, `parked_on` the first such key); a
/// handler rejection records `failed` (rolling the handler's writes back to
/// the savepoint); a store error rolls back to the savepoint and returns
/// `Err`, for the caller to retry. Needs `journal::open` to have run.
pub fn apply_one(
    tx: &Transaction<'_>,
    ctx: &StoreContext,
    decision: DecisionInstant,
    ingest_seq: i64,
    mode: ApplyMode,
) -> Result<ApplyOutcome, ApiError> {
    apply_entry(tx, ctx.sync()?, decision, ingest_seq, mode, false)
}

/// Applies the just-authored head entry `ingest_seq`. Readiness holds by
/// construction. Any outcome other than `applied` is returned as an error and
/// nothing is recorded (never `failed`): the caller's transaction rolls back.
pub fn apply_authored(
    tx: &Transaction<'_>,
    ctx: &StoreContext,
    decision: DecisionInstant,
    ingest_seq: i64,
) -> Result<(), ApiError> {
    apply_authored_with(tx, ctx.sync()?, decision, ingest_seq)
}

/// [`apply_authored`] over an explicit runtime, for `journal::open`, which
/// applies the bootstrap genesis before the runtime is installed.
pub(crate) fn apply_authored_with(
    tx: &Transaction<'_>,
    sync: &SyncState,
    decision: DecisionInstant,
    ingest_seq: i64,
) -> Result<(), ApiError> {
    apply_entry(tx, sync, decision, ingest_seq, ApplyMode::Head, true).map(|_| ())
}

struct EntryRow {
    log_ix: i64,
    origin: LogKey,
    counter: u64,
    hlc: u64,
    kind: String,
    major: u32,
    needs: String,
    payload: Vec<u8>,
}

fn load_entry(tx: &Transaction<'_>, ingest_seq: i64) -> Result<EntryRow, ApiError> {
    let corrupt = |what: &str| {
        api_error(
            ErrorCode::StoreCorrupt,
            format!("journal row {ingest_seq}: {what}"),
        )
    };
    let (log_ix, node, incarnation, counter, hlc, kind, major, needs, payload): (
        i64,
        String,
        i64,
        i64,
        i64,
        String,
        i64,
        String,
        Vec<u8>,
    ) = tx
        .query_row(
            "SELECT j.log_ix,l.node_uuid,l.incarnation,j.counter,j.hlc,j.kind,j.kind_version,j.needs,j.payload \
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
                ))
            },
        )
        .optional()
        .map_err(store_error)?
        .ok_or_else(|| corrupt("missing"))?;
    Ok(EntryRow {
        log_ix,
        origin: LogKey {
            node: node.parse().map_err(|_| corrupt("malformed node uuid"))?,
            incarnation: u32::try_from(incarnation).map_err(|_| corrupt("incarnation"))?,
        },
        counter: u64::try_from(counter).map_err(|_| corrupt("counter"))?,
        hlc: u64::try_from(hlc).map_err(|_| corrupt("hlc"))?,
        kind,
        major: u32::try_from(major).map_err(|_| corrupt("kind_version"))?,
        needs,
        payload,
    })
}

/// Cuts `text` to at most `max` bytes at a char boundary.
fn truncate_at_boundary(mut text: String, max: usize) -> String {
    if text.len() > max {
        let mut end = max;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

fn error_text(error: &ApiError) -> String {
    let code = serde_json::to_value(&error.code)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{:?}", error.code));
    truncate_at_boundary(format!("{code}: {}", error.detail), APPLY_ERROR_MAX)
}

/// Writes the entry's non-applied outcome and, for a head, counts it as
/// processed.
fn record_unapplied(
    tx: &Transaction<'_>,
    row: &EntryRow,
    ingest_seq: i64,
    mode: ApplyMode,
    state: &str,
    parked_on: Option<&str>,
    error: &str,
) -> Result<(), ApiError> {
    let error = truncate_at_boundary(error.to_owned(), APPLY_ERROR_MAX);
    tx.execute(
        "UPDATE journal SET apply_state=?2, parked_on=?3, apply_error=?4 WHERE ingest_seq=?1",
        params![ingest_seq, state, parked_on, error],
    )
    .map_err(store_error)?;
    advance_head(tx, row, mode)
}

fn advance_head(tx: &Transaction<'_>, row: &EntryRow, mode: ApplyMode) -> Result<(), ApiError> {
    if mode == ApplyMode::Head {
        tx.execute(
            "UPDATE sync_logs SET applied_through=?2, blocked_on=NULL WHERE log_ix=?1 AND applied_through<?2",
            params![row.log_ix, row.counter as i64],
        )
        .map_err(store_error)?;
    }
    Ok(())
}

fn rollback_savepoint(tx: &Transaction<'_>) -> Result<(), ApiError> {
    tx.execute_batch("ROLLBACK TO apply_entry; RELEASE apply_entry;")
        .map_err(store_error)
}

/// Runs the handler inside the open savepoint. The outer `Err` is an engine
/// store error; the inner one is the handler's.
fn run_handler(
    tx: &Transaction<'_>,
    sync: &SyncState,
    decision: DecisionInstant,
    ingest_seq: i64,
    row: &EntryRow,
) -> Result<Result<i64, FactError>, ApiError> {
    let local: String = tx
        .query_row("SELECT id FROM local_node", [], |r| r.get(0))
        .map_err(store_error)?;
    let local_node: NodeUuid = local
        .parse()
        .map_err(|_| api_error(ErrorCode::StoreCorrupt, "local_node id is not a node uuid"))?;
    let decision_seq = i64::try_from(crate::store::schema::next_decision_seq(tx, &local)?)
        .map_err(|_| api_error(ErrorCode::SequenceExhausted, "decision sequence exhausted"))?;
    let Some(handler) = sync.registry.lookup(&row.kind, row.major) else {
        return Err(api_error(
            ErrorCode::StoreCorrupt,
            "registry lookup changed",
        ));
    };
    let mut cx = ApplyContext::new(
        row.origin,
        row.counter,
        ingest_seq,
        HlcStamp::from_u64(row.hlc),
        decision,
        decision_seq,
        local_node,
    );
    if let Err(error) = handler(tx, &mut cx, &row.payload) {
        return Ok(Err(error));
    }
    if !cx.follow_ups().is_empty() {
        return Ok(Err(FactError::Rejected(api_error(
            ErrorCode::Unsupported,
            "follow-up facts are not applied by this engine yet",
        ))));
    }
    Ok(Ok(decision_seq))
}

fn apply_entry(
    tx: &Transaction<'_>,
    sync: &SyncState,
    decision: DecisionInstant,
    ingest_seq: i64,
    mode: ApplyMode,
    authored: bool,
) -> Result<ApplyOutcome, ApiError> {
    let row = load_entry(tx, ingest_seq)?;
    let not_applicable = |reason: String| {
        api_error(
            ErrorCode::Conflict,
            format!("fact {} not applicable: {reason}", row.kind),
        )
    };

    if sync.registry.lookup(&row.kind, row.major).is_none() {
        let reason = format!("unknown kind {} v{}", row.kind, row.major);
        if authored {
            return Err(not_applicable(reason));
        }
        record_unapplied(tx, &row, ingest_seq, mode, "parked_unknown", None, &reason)?;
        return Ok(ApplyOutcome::ParkedUnknown);
    }

    for key in row.needs.split('\n').filter(|key| !key.is_empty()) {
        let satisfied: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM journal WHERE creates=?1 AND apply_state='applied')",
                [key],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        if satisfied {
            continue;
        }
        let creator: Option<String> = tx
            .query_row(
                "SELECT apply_state FROM journal WHERE creates=?1 ORDER BY ingest_seq LIMIT 1",
                [key],
                |r| r.get(0),
            )
            .optional()
            .map_err(store_error)?;
        let reason = format!(
            "needs {key}: creator {}",
            creator.as_deref().unwrap_or("absent")
        );
        if authored {
            return Err(not_applicable(reason));
        }
        record_unapplied(tx, &row, ingest_seq, mode, "parked_dep", Some(key), &reason)?;
        return Ok(ApplyOutcome::ParkedDep { on: key.to_owned() });
    }

    tx.execute_batch("SAVEPOINT apply_entry")
        .map_err(store_error)?;
    let handled = match run_handler(tx, sync, decision, ingest_seq, &row) {
        Ok(handled) => handled,
        Err(error) => {
            rollback_savepoint(tx)?;
            return Err(error);
        }
    };
    match handled {
        Ok(decision_seq) => {
            let marked = tx
                .execute(
                    "UPDATE journal SET apply_state='applied', applied_seq=?2, apply_error=NULL, parked_on=NULL \
                     WHERE ingest_seq=?1",
                    params![ingest_seq, decision_seq],
                )
                .map_err(store_error)
                .and_then(|_| advance_head(tx, &row, mode));
            if let Err(error) = marked {
                rollback_savepoint(tx)?;
                return Err(error);
            }
            tx.execute_batch("RELEASE apply_entry")
                .map_err(store_error)?;
            Ok(ApplyOutcome::Applied)
        }
        Err(FactError::Rejected(error)) => {
            rollback_savepoint(tx)?;
            if authored {
                return Err(error);
            }
            record_unapplied(
                tx,
                &row,
                ingest_seq,
                mode,
                "failed",
                None,
                &error_text(&error),
            )?;
            Ok(ApplyOutcome::Failed)
        }
        Err(FactError::Store(error)) => {
            rollback_savepoint(tx)?;
            Err(error)
        }
    }
}
