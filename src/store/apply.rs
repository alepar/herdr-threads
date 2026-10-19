//! ApplyEngine: the context a fact handler runs with, the single-entry apply
//! path and the passes that drive it.
//!
//! This file holds the handler-facing contract ([`ApplyContext`],
//! [`Provenance`]), the single-entry path ([`apply_one`], [`apply_authored`]:
//! registry lookup, the `needs` check, a savepoint, one local position, the
//! handler, the recorded outcome and the follow-ups a handler queued), the
//! causal pass over ingested entries ([`apply_pass`]) and replay
//! ([`replay`]). Journal and
//! this module call each other by design: Journal authors and hands the new
//! row here in the same transaction, and a follow-up is authored back through
//! Journal so its deps come after its trigger.
//!
//! Responsibility
//! - Owns: readiness (a log's head entry is ready when every `deps` triple is
//!   applied, capped at `frozen_at`), turning one stored `journal` row into an
//!   outcome (`applied`, `parked_unknown`, `parked_dep`, `failed`), the row's
//!   `apply_*` columns, `sync_logs.applied_through`/`blocked_on`, the one local
//!   position per applied entry, re-attempting `parked_dep` entries whose
//!   creator applied, or that wait for a host once one is attached (by the
//!   indexed `creates` query, no in-memory map) and
//!   the startup and operator replay passes.
//! - Must not: verify or append envelopes (Journal), know what any kind means
//!   (FactRegistry handlers do), record `failed` for a locally authored entry
//!   (the caller rolls back instead), or let a handler author directly.
//! - Role: the one place a fact is materialized, for local and remote entries
//!   alike. New apply rules (readiness, outcomes, positions, suppression) go
//!   here; handlers never decide them and commands never materialize outside
//!   this module.

use std::collections::HashSet;
use std::marker::PhantomData;

use rusqlite::{OptionalExtension, Transaction, params};

use crate::node::NodeUuid;
use crate::protocol::ids::SeatId;
use crate::protocol::results::{ApiError, ErrorCode};
use crate::protocol::time::{Clock, MonoInstant};
use crate::store::connection::{DecisionInstant, StoreContext, SyncState, api_error, store_error};
use crate::store::journal;
use crate::store::seats;
use crate::sync::envelope::Deps;
use crate::sync::facts::{FactError, FactKind};
use crate::sync::hlc::HlcStamp;
use crate::sync::journal_port::ApplyProgress;
use crate::sync::{EntityKey, LogKey};

/// Where an applied entry came from, as handlers and attribution see it.
///
/// Closed set. Any incarnation of this node's own log is `Local`; only an
/// entry authored by another node is `ReplicatedFrom`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provenance {
    Local,
    ReplicatedFrom(NodeUuid),
}

impl Provenance {
    /// The TRUST-POLICY value: `replicated_from:<node uuid hex>` for another
    /// node's fact, `local_journal` for this node's own journal (the policy's
    /// value for an own entry materialized with no staged local attribution).
    pub fn as_str(&self) -> String {
        match self {
            Self::Local => "local_journal".to_owned(),
            Self::ReplicatedFrom(node) => format!("replicated_from:{node}"),
        }
    }
}

/// A fact queued by a handler through [`ApplyContext::follow_up`]: kind name,
/// major, the serialized payload and the `creates`/`needs` keys taken from the
/// typed payload when it was queued. ApplyEngine authors it after the
/// triggering entry is recorded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueuedFact {
    pub kind: &'static str,
    pub major: u32,
    pub payload: Vec<u8>,
    pub creates: Option<EntityKey>,
    /// Sorted and unique.
    pub needs: Vec<EntityKey>,
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
    own_old_incarnation: bool,
    reissue_of: Option<LogKey>,
    /// The entry's signed `creates` and `needs` keys, set by the engine for a
    /// stored entry; `None` for a context built outside the engine.
    pub(crate) entry_keys: Option<(Option<String>, Vec<String>)>,
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
            own_old_incarnation: false,
            reissue_of: None,
            entry_keys: None,
            _entry: PhantomData,
        }
    }

    /// Records the entry's signed keys, which `erased_apply` checks against
    /// the typed payload.
    #[must_use]
    pub(crate) fn with_entry_keys(mut self, creates: Option<String>, needs: Vec<String>) -> Self {
        self.entry_keys = Some((creates, needs));
        self
    }

    /// The origin's "sent at", in Unix ms.
    pub fn origin_ms(&self) -> u64 {
        self.hlc.ms()
    }

    /// True when the entry's origin node is this node, in any incarnation.
    pub fn is_local_origin(&self) -> bool {
        self.origin.node == self.local_node
    }

    /// The only source of `replicated_from` attribution: `ReplicatedFrom` of
    /// the origin node when it is another node, `Local` for any incarnation of
    /// this node. Handlers record this value and never derive their own; its
    /// spelling is defined in TRUST-POLICY.md.
    pub fn provenance(&self) -> Provenance {
        if self.is_local_origin() {
            Provenance::Local
        } else {
            Provenance::ReplicatedFrom(self.origin.node)
        }
    }

    /// Marks the entry as read from one of this node's older incarnations
    /// (a local-origin log other than the active self log). ApplyEngine sets
    /// it; a bare `new` context is not own-old.
    pub(crate) fn with_own_old_incarnation(mut self, own_old: bool) -> Self {
        self.own_old_incarnation = own_old;
        self
    }

    /// Marks the entry as running under [`ApplyMode::Reissue`] for the source
    /// log `of`.
    pub(crate) fn with_reissue_of(mut self, of: Option<LogKey>) -> Self {
        self.reissue_of = of;
        self
    }

    /// True when the entry is this node's own earlier history: an entry of an
    /// own old incarnation, or one being re-issued into the active log
    /// ([`ApplyMode::Reissue`]). Attribution reads it to record
    /// `local_journal` when no local attribution was staged. Provenance is
    /// unaffected: both cases stay [`Provenance::Local`].
    pub fn is_own_history(&self) -> bool {
        self.own_old_incarnation || self.reissue_of.is_some()
    }

    /// Whether `seat` exists and its home is this node (delegates to
    /// [`seats::is_local_seat`]); an unknown seat is not local.
    pub fn is_local_seat(&self, tx: &Transaction<'_>, seat: &SeatId) -> Result<bool, ApiError> {
        seats::is_local_seat(tx, seat)
    }

    /// Catch-up suppression: true exactly when a `journal` or `journal_forked`
    /// row with `creates = settle_key` exists in a log of this node, in any
    /// apply state and whatever the entry being applied came from. The rule is
    /// kind-agnostic: a handler asks with the key that settles its obligation
    /// and never decides suppression itself. An entry from an own old
    /// incarnation does not suppress by itself; only a settlement does, so an
    /// unsettled obligation in a pulled-back own tail is still offered once
    /// the own-history hold lifts.
    pub fn delivery_suppressed(
        &self,
        tx: &Transaction<'_>,
        settle_key: &EntityKey,
    ) -> Result<bool, ApiError> {
        tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM journal j JOIN sync_logs l ON l.log_ix=j.log_ix \
                 WHERE j.creates=?1 AND l.node_uuid=?2) \
             OR EXISTS(SELECT 1 FROM journal_forked j JOIN sync_logs l ON l.log_ix=j.log_ix \
                 WHERE j.creates=?1 AND l.node_uuid=?2)",
            params![settle_key.as_str(), self.local_node.to_string()],
            |r| r.get(0),
        )
        .map_err(store_error)
    }

    /// Queues a typed fact. Only queues: ApplyEngine authors it after the
    /// triggering entry is recorded, so its deps include the trigger.
    pub fn follow_up<K: FactKind>(
        &mut self,
        payload: &K::Payload,
    ) -> Result<(), serde_json::Error> {
        let mut needs = K::needs(payload);
        needs.sort();
        needs.dedup();
        self.follow_ups.push(QueuedFact {
            kind: K::NAME,
            major: K::MAJOR,
            payload: serde_json::to_vec(payload)?,
            creates: K::creates(payload),
            needs,
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
    /// Journal's `reissue`: the entry was just authored as the head of the
    /// active self log from an entry of the log `of` (a frozen own log or one
    /// of its branches). Readiness is not checked (its deps may exceed the
    /// owner's applied vector) and `needs` parking applies as usual; like a
    /// head, any recorded outcome counts as processed. Suppression is the same
    /// settled-only rule as every other origin
    /// ([`ApplyContext::delivery_suppressed`]): this mode adds none of its
    /// own, so an unsettled obligation from a lost branch is still offered.
    Reissue { of: LogKey },
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

/// How many follow-up levels a trigger may chain: a follow-up authored at a
/// deeper level fails its trigger instead of recursing. v1 kinds queue one
/// level at most, so an honest chain never reaches it.
pub(crate) const MAX_FOLLOW_UP_DEPTH: u8 = 4;

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
    apply_entry(tx, ctx.sync()?, decision, ingest_seq, mode, false, 0)
        .map(|entered| entered.outcome)
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
    apply_authored_with(tx, ctx.sync()?, decision, ingest_seq, 0).map(|_| ())
}

/// [`apply_authored`] over an explicit runtime, for `journal::open`, which
/// applies the bootstrap genesis before the runtime is installed. Returns the
/// tag counter the entry (or a follow-up it queued) raised, for the caller to
/// pass on as [`ApplyProgress::self_tag_k`]. `depth` is the follow-up level
/// of the entry (0 for a trigger).
pub(crate) fn apply_authored_with(
    tx: &Transaction<'_>,
    sync: &SyncState,
    decision: DecisionInstant,
    ingest_seq: i64,
    depth: u8,
) -> Result<Option<u32>, ApiError> {
    apply_entry(tx, sync, decision, ingest_seq, ApplyMode::Head, true, depth)
        .map(|entered| entered.self_tag_k)
}

struct EntryRow {
    log_ix: i64,
    origin: LogKey,
    counter: u64,
    hlc: u64,
    kind: String,
    major: u32,
    creates: Option<String>,
    needs: String,
    payload: Vec<u8>,
}

#[allow(clippy::type_complexity)]
fn load_entry(tx: &Transaction<'_>, ingest_seq: i64) -> Result<EntryRow, ApiError> {
    let corrupt = |what: &str| {
        api_error(
            ErrorCode::StoreCorrupt,
            format!("journal row {ingest_seq}: {what}"),
        )
    };
    let (log_ix, node, incarnation, counter, hlc, kind, major, creates, needs, payload): (
        i64,
        String,
        i64,
        i64,
        i64,
        String,
        i64,
        Option<String>,
        String,
        Vec<u8>,
    ) = tx
        .query_row(
            "SELECT j.log_ix,l.node_uuid,l.incarnation,j.counter,j.hlc,j.kind,j.kind_version,j.creates,j.needs,j.payload \
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
        creates,
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
    if matches!(mode, ApplyMode::Head | ApplyMode::Reissue { .. }) {
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

/// What a handler left behind for the engine to finish.
struct Handled {
    decision_seq: i64,
    follow_ups: Vec<QueuedFact>,
    self_tag_k: Option<u32>,
}

/// An applied entry's outcome plus the tag change its handler asked for.
struct Entered {
    outcome: ApplyOutcome,
    self_tag_k: Option<u32>,
}

impl Entered {
    fn plain(outcome: ApplyOutcome) -> Self {
        Self {
            outcome,
            self_tag_k: None,
        }
    }
}

/// Runs the handler inside the open savepoint. The outer `Err` is an engine
/// store error; the inner one is the handler's.
fn run_handler(
    tx: &Transaction<'_>,
    sync: &SyncState,
    decision: DecisionInstant,
    ingest_seq: i64,
    row: &EntryRow,
    mode: ApplyMode,
) -> Result<Result<Handled, FactError>, ApiError> {
    let (local, active_log_ix): (String, Option<i64>) = tx
        .query_row("SELECT id,active_log_ix FROM local_node", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
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
    )
    .with_entry_keys(
        row.creates.clone(),
        row.needs
            .split('\n')
            .filter(|key| !key.is_empty())
            .map(str::to_owned)
            .collect(),
    )
    .with_own_old_incarnation(row.origin.node == local_node && Some(row.log_ix) != active_log_ix)
    .with_reissue_of(match mode {
        ApplyMode::Reissue { of } => Some(of),
        _ => None,
    });
    if let Err(error) = handler(tx, &mut cx, &row.payload) {
        return Ok(Err(error));
    }
    Ok(Ok(Handled {
        decision_seq,
        self_tag_k: cx.self_tag_k(),
        follow_ups: cx.take_follow_ups(),
    }))
}

fn apply_entry(
    tx: &Transaction<'_>,
    sync: &SyncState,
    decision: DecisionInstant,
    ingest_seq: i64,
    mode: ApplyMode,
    authored: bool,
    depth: u8,
) -> Result<Entered, ApiError> {
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
        return Ok(Entered::plain(ApplyOutcome::ParkedUnknown));
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
        return Ok(Entered::plain(ApplyOutcome::ParkedDep {
            on: key.to_owned(),
        }));
    }

    tx.execute_batch("SAVEPOINT apply_entry")
        .map_err(store_error)?;
    // The write guard lets replicated tables change only while this is held.
    #[cfg(feature = "test-support")]
    let _write_scope = crate::test_support::write_guard::enter_replicated_write_scope();
    let handled = match run_handler(tx, sync, decision, ingest_seq, &row, mode) {
        Ok(handled) => handled,
        Err(error) => {
            rollback_savepoint(tx)?;
            return Err(error);
        }
    };
    match handled {
        Ok(handled) => {
            let marked = tx
                .execute(
                    "UPDATE journal SET apply_state='applied', applied_seq=?2, apply_error=NULL, parked_on=NULL \
                     WHERE ingest_seq=?1",
                    params![ingest_seq, handled.decision_seq],
                )
                .map_err(store_error)
                .and_then(|_| advance_head(tx, &row, mode));
            if let Err(error) = marked {
                rollback_savepoint(tx)?;
                return Err(error);
            }
            // Follow-ups go in after the trigger's own update, still inside
            // its savepoint, so their deps include the trigger.
            let mut self_tag_k = handled.self_tag_k;
            for fact in &handled.follow_ups {
                let error = match journal::author_queued(tx, sync, decision, fact, depth + 1) {
                    Ok(authored) => {
                        self_tag_k = self_tag_k.max(authored.self_tag_k);
                        continue;
                    }
                    Err(error) => error,
                };
                rollback_savepoint(tx)?;
                if authored || matches!(FactError::from_api(error.clone()), FactError::Store(_)) {
                    return Err(if authored {
                        api_error(
                            error.code,
                            format!("follow-up {}: {}", fact.kind, error.detail),
                        )
                    } else {
                        error
                    });
                }
                let reason = format!("follow-up {}: {}", fact.kind, error_text(&error));
                record_unapplied(tx, &row, ingest_seq, mode, "failed", None, &reason)?;
                return Ok(Entered::plain(ApplyOutcome::Failed));
            }
            tx.execute_batch("RELEASE apply_entry")
                .map_err(store_error)?;
            Ok(Entered {
                outcome: ApplyOutcome::Applied,
                self_tag_k,
            })
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
            Ok(Entered::plain(ApplyOutcome::Failed))
        }
        Err(FactError::Store(error)) => {
            rollback_savepoint(tx)?;
            Err(error)
        }
        Err(FactError::Wait { on, error }) => {
            rollback_savepoint(tx)?;
            if authored {
                return Err(error);
            }
            record_unapplied(
                tx,
                &row,
                ingest_seq,
                mode,
                "parked_dep",
                Some(on.parked_on()),
                &error_text(&error),
            )?;
            Ok(Entered::plain(ApplyOutcome::ParkedDep {
                on: on.parked_on().to_owned(),
            }))
        }
    }
}

/// How much one [`apply_pass`] or [`replay`] may do before it reports
/// `has_more`: attempts, and monotonic milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ApplyLimits {
    pub entries: usize,
    pub wall_ms: u64,
}

impl Default for ApplyLimits {
    fn default() -> Self {
        Self {
            entries: 64,
            wall_ms: 100,
        }
    }
}

/// An unapplied state a replay may re-attempt. `pending` is not one: a pending
/// entry is not yet causally ready and only [`apply_pass`] may take it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplyState {
    ParkedUnknown,
    ParkedDep,
    Failed,
}

impl ApplyState {
    fn as_sql(self) -> &'static str {
        match self {
            ApplyState::ParkedUnknown => "parked_unknown",
            ApplyState::ParkedDep => "parked_dep",
            ApplyState::Failed => "failed",
        }
    }
}

/// Which entries a [`replay`] re-attempts. Closed set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplayScope<'a> {
    /// Daemon start: `parked_unknown` entries the registry now knows, plus
    /// `parked_dep` entries whose creator has applied or that wait for a host
    /// while one is attached.
    Startup,
    /// `sync unapplied replay`: every entry in the named states.
    Unapplied { states: &'a [ApplyState] },
}

/// The attempt budget and tally of one pass.
struct Run<'a> {
    clock: &'a dyn Clock,
    started: MonoInstant,
    limits: ApplyLimits,
    attempts: usize,
    progress: ApplyProgress,
    attempted: HashSet<i64>,
}

impl<'a> Run<'a> {
    fn new(clock: &'a dyn Clock, limits: ApplyLimits) -> Self {
        Self {
            clock,
            started: clock.monotonic_now(),
            limits,
            attempts: 0,
            progress: ApplyProgress::default(),
            attempted: HashSet::new(),
        }
    }

    fn exhausted(&self) -> bool {
        self.attempts >= self.limits.entries
            || self.clock.monotonic_now().0.saturating_sub(self.started.0) >= self.limits.wall_ms
    }

    /// Applies one entry and counts its outcome. A replayed attempt also
    /// counts under `replayed`.
    fn attempt(
        &mut self,
        tx: &Transaction<'_>,
        sync: &SyncState,
        decision: DecisionInstant,
        ingest_seq: i64,
        mode: ApplyMode,
    ) -> Result<(), ApiError> {
        self.attempts += 1;
        self.attempted.insert(ingest_seq);
        let entered = apply_entry(tx, sync, decision, ingest_seq, mode, false, 0)?;
        match entered.outcome {
            ApplyOutcome::Applied => self.progress.applied += 1,
            ApplyOutcome::ParkedUnknown | ApplyOutcome::ParkedDep { .. } => {
                self.progress.parked += 1;
            }
            ApplyOutcome::Failed => self.progress.failed += 1,
        }
        if mode == ApplyMode::Replay {
            self.progress.replayed += 1;
        }
        if let Some(k) = entered.self_tag_k {
            self.progress.self_tag_k = Some(self.progress.self_tag_k.map_or(k, |old| old.max(k)));
        }
        Ok(())
    }
}

/// A log's head entry awaiting readiness.
struct Head {
    log_ix: i64,
    log: LogKey,
    blocked_on: Option<String>,
    ingest_seq: i64,
    deps: Vec<u8>,
}

/// The first unmet dependency of `head`, as the `blocked_on` text, or `None`
/// when every deps triple is covered. A triple `(L, c)` is covered when `L`
/// has a row and `applied(L) >= min(c, frozen_at(L))`.
fn unmet_dep(tx: &Transaction<'_>, head: &Head) -> Result<Option<String>, ApiError> {
    let deps = Deps::decode(&head.deps, &head.log).map_err(|e| {
        api_error(
            ErrorCode::StoreCorrupt,
            format!("journal row {}: deps: {e}", head.ingest_seq),
        )
    })?;
    for (dep, counter) in deps.entries() {
        let held: Option<(i64, Option<i64>)> = tx
            .query_row(
                "SELECT applied_through,frozen_at_counter FROM sync_logs \
                 WHERE node_uuid=?1 AND incarnation=?2",
                params![dep.node.to_string(), dep.incarnation],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(store_error)?;
        let needed = i64::try_from(*counter).unwrap_or(i64::MAX);
        let met = held.is_some_and(|(applied, frozen_at)| {
            applied >= frozen_at.map_or(needed, |frozen| needed.min(frozen))
        });
        if !met {
            let node = dep.node.to_string();
            return Ok(Some(format!(
                "deps {}/{}@{counter}",
                &node[..node.len().min(8)],
                dep.incarnation
            )));
        }
    }
    Ok(None)
}

/// The `sync_logs.blocked_on` text of a log whose next chain position holds
/// an entry of a signed layout this node does not understand. It is the
/// parking record: the log applies nothing past that position, its
/// `applied_through` stays, and later entries stay `pending` (not
/// `parked_unknown`). Dependents of the stalled position block on the
/// ordinary `deps <node8>/<inc>@<c>` text.
pub const OPAQUE_LAYOUT: &str = "opaque_layout";

/// Records [`OPAQUE_LAYOUT`] on every log whose head position is opaque,
/// writing only on change. Under the own-history hold only the active self
/// log is touched, like the known heads.
fn mark_opaque_heads(
    tx: &Transaction<'_>,
    active_log: Option<i64>,
    only_self_log: bool,
) -> Result<(), ApiError> {
    let mut statement = tx
        .prepare(
            // Apply classifies a head by projection presence (`kind IS NULL`:
            // a `parked_layout` row, or any row not yet promoted).
            "SELECT l.log_ix FROM sync_logs l \
             JOIN journal j ON j.log_ix=l.log_ix AND j.counter=l.applied_through+1 \
             WHERE l.applied_through<l.contiguous_counter AND j.kind IS NULL \
               AND l.blocked_on IS NOT ?1",
        )
        .map_err(store_error)?;
    let stalled = statement
        .query_map(params![OPAQUE_LAYOUT], |r| r.get::<_, i64>(0))
        .map_err(store_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(store_error)?;
    drop(statement);
    for log_ix in stalled {
        if only_self_log && Some(log_ix) != active_log {
            continue;
        }
        tx.execute(
            "UPDATE sync_logs SET blocked_on=?2 WHERE log_ix=?1",
            params![log_ix, OPAQUE_LAYOUT],
        )
        .map_err(store_error)?;
    }
    Ok(())
}

/// The ready head with the smallest `ingest_seq`, if any. Heads passed over
/// on the way get their `blocked_on` written, only when it changed.
fn next_ready_head(tx: &Transaction<'_>, only_self_log: bool) -> Result<Option<i64>, ApiError> {
    let active_log: Option<i64> = if only_self_log {
        tx.query_row("SELECT active_log_ix FROM local_node", [], |r| r.get(0))
            .map_err(store_error)?
    } else {
        None
    };
    mark_opaque_heads(tx, active_log, only_self_log)?;
    let mut statement = tx
        .prepare(
            // An empty-projection head (`kind IS NULL`) stops its log and is
            // skipped here; `mark_opaque_heads` records
            // `blocked_on = opaque_layout` for it. A promoted row has its
            // projection filled and applies like any other.
            "SELECT l.log_ix,l.node_uuid,l.incarnation,l.blocked_on,j.ingest_seq,j.deps \
             FROM sync_logs l \
             JOIN journal j ON j.log_ix=l.log_ix AND j.counter=l.applied_through+1 \
             WHERE l.applied_through<l.contiguous_counter AND j.kind IS NOT NULL \
             ORDER BY j.ingest_seq",
        )
        .map_err(store_error)?;
    let heads = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, Vec<u8>>(5)?,
            ))
        })
        .map_err(store_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(store_error)?;
    drop(statement);
    for (log_ix, node, incarnation, blocked_on, ingest_seq, deps) in heads {
        // Own-history hold: every log but the active self log stays pending,
        // with no `blocked_on` write.
        if only_self_log && Some(log_ix) != active_log {
            continue;
        }
        let corrupt = |what: &str| {
            api_error(
                ErrorCode::StoreCorrupt,
                format!("sync_logs row {log_ix}: {what}"),
            )
        };
        let head = Head {
            log_ix,
            log: LogKey {
                node: node.parse().map_err(|_| corrupt("malformed node uuid"))?,
                incarnation: u32::try_from(incarnation).map_err(|_| corrupt("incarnation"))?,
            },
            blocked_on,
            ingest_seq,
            deps,
        };
        match unmet_dep(tx, &head)? {
            None => return Ok(Some(head.ingest_seq)),
            Some(text) => {
                if head.blocked_on.as_deref() != Some(text.as_str()) {
                    tx.execute(
                        "UPDATE sync_logs SET blocked_on=?2 WHERE log_ix=?1",
                        params![head.log_ix, text],
                    )
                    .map_err(store_error)?;
                }
            }
        }
    }
    Ok(None)
}

/// A `parked_dep` row `p` may be re-attempted: its creator applied, or it
/// waits for a host and `herdr_hosts` holds a row (the materializer's own
/// condition, so a re-attempt never re-parks on the host).
const PARKED_READY: &str = "(EXISTS(SELECT 1 FROM journal c WHERE c.creates=p.parked_on AND c.apply_state='applied') \
     OR (p.parked_on='wait:host' AND EXISTS(SELECT 1 FROM herdr_hosts)))";

/// The `parked_dep` entry with the smallest `ingest_seq` that may be
/// re-attempted ([`PARKED_READY`]).
fn next_reattemptable(tx: &Transaction<'_>) -> Result<Option<i64>, ApiError> {
    tx.query_row(
        &format!(
            "SELECT p.ingest_seq FROM journal p WHERE p.apply_state='parked_dep' AND {PARKED_READY} \
             ORDER BY p.ingest_seq LIMIT 1"
        ),
        [],
        |r| r.get(0),
    )
    .optional()
    .map_err(store_error)
}

/// Re-attempts `parked_dep` entries whose creator applied, until none is
/// left. Returns true when the limit stopped it with work remaining.
fn reattempt_parked(
    tx: &Transaction<'_>,
    sync: &SyncState,
    decision: DecisionInstant,
    run: &mut Run<'_>,
) -> Result<bool, ApiError> {
    while let Some(ingest_seq) = next_reattemptable(tx)? {
        if run.exhausted() {
            return Ok(true);
        }
        run.attempt(tx, sync, decision, ingest_seq, ApplyMode::Replay)?;
    }
    Ok(false)
}

/// One bounded apply pass over ingested entries.
///
/// Repeatedly takes the ready log head with the smallest `ingest_seq` (ready
/// per [`unmet_dep`]) and applies it, advancing `applied_through` whatever the
/// outcome; a head that is not ready gets `blocked_on` set and is skipped.
/// While the own-history hold is active ([`crate::store::connection::OwnHistoryHold`]) only the active
/// self log's head is considered; other heads stay `pending`.
/// Between heads, `parked_dep` entries whose creator has applied (or that
/// wait for a host, once one is attached) are re-attempted. Stops when nothing is ready, or at `limits` with
/// `has_more` set. A `Store` error aborts the pass: the caller rolls the whole
/// quantum back. `applied` counts every entry that applied, `replayed` the
/// re-attempts among all outcomes.
pub fn apply_pass(
    tx: &Transaction<'_>,
    ctx: &StoreContext,
    decision: DecisionInstant,
    limits: ApplyLimits,
) -> Result<ApplyProgress, ApiError> {
    let sync = ctx.sync()?;
    // Entries of a layout this build learned since they were parked.
    journal::promote_parked(tx, &sync.promote_logged)?;
    let mut run = Run::new(ctx.clock(), limits);
    let hold = sync.own_history_hold.active(ctx.clock().monotonic_now());
    loop {
        if reattempt_parked(tx, sync, decision, &mut run)? {
            run.progress.has_more = true;
            break;
        }
        let Some(ingest_seq) = next_ready_head(tx, hold)? else {
            break;
        };
        if run.exhausted() {
            run.progress.has_more = true;
            break;
        }
        run.attempt(tx, sync, decision, ingest_seq, ApplyMode::Head)?;
    }
    Ok(run.progress)
}

/// Re-attempts unapplied entries in `ingest_seq` order, per `scope`.
///
/// A replay never moves `applied_through` (the entries were counted when they
/// were first processed). An entry that applies gets a fresh `applied_seq`, so
/// it takes late local positions. Entries whose creator applies along the way
/// are re-attempted too. Each entry is attempted at most once per call.
pub fn replay(
    tx: &Transaction<'_>,
    ctx: &StoreContext,
    decision: DecisionInstant,
    scope: ReplayScope<'_>,
    limits: ApplyLimits,
) -> Result<ApplyProgress, ApiError> {
    let sync = ctx.sync()?;
    let mut run = Run::new(ctx.clock(), limits);
    for ingest_seq in replay_candidates(tx, sync, scope)? {
        if run.attempted.contains(&ingest_seq) {
            continue;
        }
        if run.exhausted() {
            run.progress.has_more = true;
            return Ok(run.progress);
        }
        run.attempt(tx, sync, decision, ingest_seq, ApplyMode::Replay)?;
        if reattempt_parked(tx, sync, decision, &mut run)? {
            run.progress.has_more = true;
            return Ok(run.progress);
        }
    }
    Ok(run.progress)
}

/// The entries `scope` selects, in `ingest_seq` order.
fn replay_candidates(
    tx: &Transaction<'_>,
    sync: &SyncState,
    scope: ReplayScope<'_>,
) -> Result<Vec<i64>, ApiError> {
    let states = match scope {
        ReplayScope::Startup => "'parked_unknown','parked_dep'".to_owned(),
        ReplayScope::Unapplied { states } => states
            .iter()
            .map(|state| format!("'{}'", state.as_sql()))
            .collect::<Vec<_>>()
            .join(","),
    };
    if states.is_empty() {
        return Ok(Vec::new());
    }
    let mut statement = tx
        .prepare(&format!(
            "SELECT p.ingest_seq,p.apply_state,p.kind,p.kind_version,\
             COALESCE({PARKED_READY}, 0) \
             FROM journal p WHERE p.apply_state IN ({states}) ORDER BY p.ingest_seq"
        ))
        .map_err(store_error)?;
    let rows = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, bool>(4)?,
            ))
        })
        .map_err(store_error)?;
    let mut out = Vec::new();
    for row in rows {
        let (ingest_seq, state, kind, major, ready) = row.map_err(store_error)?;
        let selected = match scope {
            ReplayScope::Unapplied { .. } => true,
            ReplayScope::Startup if state == "parked_unknown" => u32::try_from(major)
                .ok()
                .is_some_and(|major| sync.registry.lookup(&kind, major).is_some()),
            ReplayScope::Startup => ready,
        };
        if selected {
            out.push(ingest_seq);
        }
    }
    Ok(out)
}

#[cfg(test)]
#[path = "../../tests/store/apply.rs"]
mod tests;
