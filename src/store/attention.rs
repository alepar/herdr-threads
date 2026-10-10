//! Seat attention reads: the digest producer (root adoption, wave-1 fix2
//! (a)) and the canonical pending sets that check-in and inbox count
//! (wave-2 (a)), under the bounded-walk invariant (wave-2 fix1 root decision
//! (a), digest fix5).
//!
//! Bounded-walk invariant. Every read here is a fixed set of per-source walks,
//! each an indexed `LIMIT cap+1` walk (`WINDOW`) over a pending-only access
//! path: a schema v8 projection (a pure pending set kept by triggers in the
//! writers' transactions), a pending-only partial index, or the outstanding
//! projection backlog. Candidates are judged by the canonical predicates only
//! inside a walk's window, never after collecting an unbounded set. A walk
//! whose window fills marks the set `saturated`: counts then saturate at
//! `ATTENTION_COUNT_CAP` with `has_more`. Per-request work is therefore
//! O(`WINDOW` x sources), independent of retained history and of the size of
//! the pending backlog.
//!
//! Walks run newest-first by logical publication key where the projection
//! holds it (invitations, materialized manifest receipts), otherwise by
//! insertion ordinal (informational warnings, above the occupant's offered
//! notice frontier; staged manifest receipts, physical receipts,
//! open-condition warnings, the backlog), which follows publication order
//! except for a preparation staged before, and published after, a full
//! window of newer rows for the same seat and source.
//!
//! Sources, per class:
//! - invitations: `digest_pending_invitations` (pending, not cancelled);
//! - receipts: `digest_pending_manifest_receipts` rows materialized as a
//!   pending `receipt_state` (keyed by decision sequence) and still staged
//!   (not yet materialized, published only), plus pending physical rows, each
//!   judged by `effective_receipt`;
//! - warnings:
//!   - informational warnings (programmatic service notices and canonical
//!     condition transitions) above the seat's current occupant's
//!     offered notice frontier (`digest_programmatic_warnings` rows whose
//!     projection ordinal exceeds `digest_notice_offer.offered_ordinal`, the
//!     occupant-scoped monotone frontier; wave-2 fix2 root decision (a)). A
//!     check-in offer settles only the capped page of notices it actually
//!     carries (`notice_offer_page`, `settle_offered_notices`): one O(1)
//!     frontier write, never a delete of the backlog;
//!   - attributed recipients of legacy open-condition warnings
//!     (`digest_open_warning_recipients`), and open-condition warnings naming
//!     the seat as affected (`digest_open_warnings`), each judged by
//!     `warning_condition_actionable`;
//!   - the projection backlog (`warning_backlog`): warnings whose attribution
//!     job is outstanding, and the staged recipients of publications whose
//!     `send_attention` job is outstanding, each judged by
//!     `is_warning_recipient` and `warning_condition_actionable`.
use super::{
    catch_up,
    connection::{api_error, store_error},
    effective::{self, EffectiveReceiptState, ReceiptSource},
};
use crate::{
    ports::{LogicalAttentionFrontier, LogicalPublicationKey},
    protocol::{
        attention::{
            AttentionClass, AttentionDigest, AttentionRef, AttentionRequirement, AttentionToken,
            DIGEST_VERSION, MAX_DIGEST_IDS,
        },
        ids::{MessageId, SeatId, ThreadId},
        results::{ApiError, ErrorCode, MAX_PENDING_WARNING_COUNT, WarningRef},
    },
};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::BTreeMap;

/// Every pending count (digest classes, check-in, inbox) saturates here.
pub const ATTENTION_COUNT_CAP: u64 = MAX_PENDING_WARNING_COUNT;
/// Rows one source walk may visit: the cap plus one, so that a full window
/// proves "more than the cap may be pending".
pub const WINDOW: usize = ATTENTION_COUNT_CAP as usize + 1;

/// Digest plus the number of indexed rows it visited. The count is the
/// flat-cost witness: it is bounded by `WINDOW` per source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestRun {
    pub digest: AttentionDigest,
    pub frontier: LogicalAttentionFrontier,
    pub work_steps: u64,
    /// The pending-warning set behind `digest.warnings`, unnarrowed (a
    /// caller that needs the waking subset applies `wake_warnings`).
    pub warnings: PendingSet,
}

/// One pending item with its logical publication key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingItem {
    pub id: String,
    pub thread_id: String,
    pub key: (i64, i64),
}

/// The distinct pending items the bounded walks found, newest first, whether
/// any walk filled its window, and the rows visited (the flat-cost witness).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PendingSet {
    pub items: Vec<PendingItem>,
    pub saturated: bool,
    pub work_steps: u64,
}

impl PendingSet {
    /// The presented count: at most `ATTENTION_COUNT_CAP`, with `has_more`
    /// when more than the cap may be pending.
    pub fn count(&self) -> (u64, bool) {
        let found = self.items.len() as u64;
        (
            found.min(ATTENTION_COUNT_CAP),
            self.saturated || found > ATTENTION_COUNT_CAP,
        )
    }

    /// The same set restricted to one thread (the backlog, read once per
    /// request, is shared by every thread of an inbox page).
    pub fn in_thread(&self, thread: &str) -> Self {
        Self {
            items: self
                .items
                .iter()
                .filter(|item| item.thread_id == thread)
                .cloned()
                .collect(),
            saturated: self.saturated,
            work_steps: 0,
        }
    }
}

#[derive(Default)]
struct Gather {
    items: BTreeMap<String, PendingItem>,
    saturated: bool,
    steps: u64,
}

impl Gather {
    /// Account for one walk's window of `rows` visited rows.
    fn window(&mut self, rows: usize) {
        self.steps += rows as u64;
        if rows >= WINDOW {
            self.saturated = true;
        }
    }
    fn add(&mut self, item: PendingItem) {
        match self.items.get_mut(&item.id) {
            Some(existing) if existing.key >= item.key => {}
            Some(existing) => *existing = item,
            None => {
                self.items.insert(item.id.clone(), item);
            }
        }
    }
    /// Union another set's items (its visited rows are accounted by the
    /// caller that read it).
    fn merge(&mut self, set: &PendingSet) {
        self.saturated |= set.saturated;
        for item in &set.items {
            self.add(item.clone());
        }
    }
    fn finish(self) -> PendingSet {
        let mut items: Vec<PendingItem> = self.items.into_values().collect();
        items.sort_by(|a, b| b.key.cmp(&a.key).then_with(|| a.id.cmp(&b.id)));
        PendingSet {
            items,
            saturated: self.saturated,
            work_steps: self.steps,
        }
    }
}

fn rows<T>(
    db: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
    map: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
) -> Result<Vec<T>, ApiError> {
    let mut statement = db.prepare_cached(sql).map_err(store_error)?;
    let mapped = statement.query_map(params, map).map_err(store_error)?;
    mapped
        .collect::<rusqlite::Result<Vec<T>>>()
        .map_err(store_error)
}

const WINDOW_SQL: i64 = WINDOW as i64;

fn class(set: &PendingSet) -> Result<AttentionClass, ApiError> {
    listed_class(set, set.items.iter().take(MAX_DIGEST_IDS))
}

/// A class over `set` listing exactly `listed` (at most `MAX_DIGEST_IDS`).
fn listed_class<'a>(
    set: &PendingSet,
    listed: impl Iterator<Item = &'a PendingItem>,
) -> Result<AttentionClass, ApiError> {
    let items = listed
        .map(|item| {
            Ok(AttentionRef {
                id: item.id.clone(),
                thread: ThreadId::parse(item.thread_id.clone())
                    .map_err(|_| api_error(ErrorCode::StoreCorrupt, "invalid thread id"))?,
                requirement: None,
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    let (count, count_has_more) = set.count();
    Ok(AttentionClass {
        count,
        has_more: count > items.len() as u64,
        count_has_more,
        items,
    })
}

/// The invitation class, listing first the pending invitation of each thread
/// that holds a listed receipt (native-codex-matrix-2 P6: a require-ACK
/// handoff's own invitation is usually older than a burst of newer
/// invitations, and the hook must still name its accept), then the newest
/// others, `MAX_DIGEST_IDS` in all. A receipt thread whose invitation lies
/// outside a saturated seat walk gets one `WINDOW` walk of that thread: at most
/// `MAX_DIGEST_IDS` extra walks, so the cost stays flat. Returns the rows
/// those extra walks visited.
fn invitation_class(
    db: &Connection,
    seat_id: &str,
    through: i64,
    invitations: &PendingSet,
    receipts: &AttentionClass,
) -> Result<(AttentionClass, u64), ApiError> {
    let mut listed: Vec<PendingItem> = Vec::new();
    let mut steps = 0;
    for receipt in &receipts.items {
        let thread = receipt.thread.as_str();
        if listed.len() == MAX_DIGEST_IDS || listed.iter().any(|item| item.thread_id == thread) {
            continue;
        }
        let mut found = invitations
            .items
            .iter()
            .find(|item| item.thread_id == thread)
            .cloned();
        if found.is_none() && invitations.saturated {
            let set = pending_invitations(db, seat_id, Some(thread), through)?;
            steps += set.work_steps;
            found = set.items.into_iter().next();
        }
        listed.extend(found);
    }
    let rest: Vec<PendingItem> = invitations
        .items
        .iter()
        .filter(|item| !listed.iter().any(|chosen| chosen.id == item.id))
        .take(MAX_DIGEST_IDS - listed.len())
        .cloned()
        .collect();
    listed.extend(rest);
    Ok((listed_class(invitations, listed.iter())?, steps))
}

/// Attach the pending required membership each listed invitation carries: at
/// most `MAX_DIGEST_IDS` point reads of the effective-requirement unique index
/// (one row per thread and seat), so the cost does not grow with backlog.
fn with_requirements(
    db: &Connection,
    seat_id: &str,
    mut class: AttentionClass,
) -> Result<AttentionClass, ApiError> {
    for item in &mut class.items {
        let row: Option<(String, i64, String, String)> = db
            .prepare_cached(
                "SELECT id,revision,invitation_id,state FROM requirement_episodes WHERE thread_id=?1 AND seat_id=?2 AND state IN ('pending','accepted')",
            )
            .map_err(store_error)?
            .query_row(params![item.thread.as_str(), seat_id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .optional()
            .map_err(store_error)?;
        if let Some((id, revision, invitation, state)) = row
            && invitation == item.id
            && state == "pending"
        {
            item.requirement = Some(AttentionRequirement {
                id,
                revision: u64::try_from(revision).map_err(|_| {
                    api_error(ErrorCode::StoreCorrupt, "negative requirement revision")
                })?,
            });
        }
    }
    Ok(class)
}

fn frontier_key(set: &PendingSet) -> Result<Option<LogicalPublicationKey>, ApiError> {
    set.items
        .first()
        .map(|item| {
            let (seq, offset) = item.key;
            Ok(LogicalPublicationKey {
                decision_seq: u64::try_from(seq).map_err(|_| {
                    api_error(ErrorCode::StoreCorrupt, "negative attention sequence")
                })?,
                event_offset: u64::try_from(offset)
                    .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative attention offset"))?,
            })
        })
        .transpose()
}

/// Compute the digest inside the caller's read transaction. `check_budget` is
/// polled between walks so an exhausted request budget stops the read.
pub fn seat_digest(
    db: &Connection,
    instance: &str,
    seat: &SeatId,
    check_budget: &dyn Fn() -> Result<(), ApiError>,
) -> Result<DigestRun, ApiError> {
    let seat_id = seat.as_str();
    let (decision_seq, episode, open): (i64, i64, i64) = db
        .query_row(
            "SELECT h.decision_seq,s.unavailability_episode,s.unavailability_open FROM seats s JOIN host_instances h ON h.id=s.instance_id WHERE s.id=?1 AND s.instance_id=?2",
            params![seat_id, instance],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(store_error)?
        .ok_or_else(|| api_error(ErrorCode::NotFound, "seat not found"))?;
    let invitations = pending_invitations(db, seat_id, None, decision_seq)?;
    check_budget()?;
    let receipts = pending_receipts(db, seat_id, None)?;
    check_budget()?;
    let warnings = seat_pending_warnings(db, seat_id, check_budget)?;
    check_budget()?;
    let receipt_class = class(&receipts)?;
    let (invitation_class, receipt_thread_steps) =
        invitation_class(db, seat_id, decision_seq, &invitations, &receipt_class)?;
    check_budget()?;
    let frontier = LogicalAttentionFrontier {
        invitation: frontier_key(&invitations)?,
        addressed_receipt: frontier_key(&receipts)?,
        actionable_warning: frontier_key(&warnings)?,
    };
    let pair = |key: Option<LogicalPublicationKey>| key.map(|k| (k.decision_seq, k.event_offset));
    let digest = AttentionDigest {
        version: DIGEST_VERSION,
        seat: seat.clone(),
        token: AttentionToken {
            invitation: pair(frontier.invitation),
            receipt: pair(frontier.addressed_receipt),
            warning: pair(frontier.actionable_warning),
            unavailability_episode: u64::try_from(episode)
                .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative episode"))?,
            lazy: None,
        },
        invitations: with_requirements(db, seat_id, invitation_class)?,
        receipts: receipt_class,
        warnings: class(&warnings)?,
        unavailability_open: open != 0,
        lazy: None,
        mod_channel_live: false,
    };
    Ok(DigestRun {
        digest,
        frontier,
        work_steps: invitations.work_steps
            + receipts.work_steps
            + warnings.work_steps
            + receipt_thread_steps,
        warnings,
    })
}

/// The seat's pending published lazy rows: one `WINDOW` walk of the pending
/// seat/ordinal index, newest ordinal first, taken before any publication
/// filter so an unpublished backlog cannot stretch it. Each windowed row
/// counts only when its manifest is published in this instance up to the
/// host decision sequence `through`; its key is (publication decision
/// sequence, recipient ordinal). Lazy rows never enter the wake frontier
/// (`wake_seat_attention` does not read them).
pub fn pending_lazy(
    db: &Connection,
    instance: &str,
    seat_id: &str,
    through: i64,
) -> Result<PendingSet, ApiError> {
    let window = rows(
        db,
        "SELECT w.ordinal,w.message_id,w.thread_id,(SELECT sm.decision_seq FROM send_manifests sm JOIN messages m ON m.id=sm.message_id WHERE sm.preparation_id=w.preparation_id AND sm.message_id=w.message_id AND sm.instance_id=?3 AND sm.decision_seq<=?4 AND m.delivery_mode='lazy') FROM (SELECT ordinal,preparation_id,message_id,thread_id FROM lazy_recipients INDEXED BY lazy_recipients_pending_seat_ordinal WHERE seat_id=?1 AND state='pending' ORDER BY ordinal DESC LIMIT ?2) w",
        params![seat_id, WINDOW_SQL, instance, through],
        |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<i64>>(3)?,
            ))
        },
    )?;
    let mut gather = Gather::default();
    gather.window(window.len());
    for (ordinal, id, thread_id, published) in window {
        if let Some(decision) = published {
            gather.add(PendingItem {
                id,
                thread_id,
                key: (decision, ordinal),
            });
        }
    }
    Ok(gather.finish())
}

/// Add the lazy class and the token's lazy key to a digest that asked for
/// them (`AttentionDigestQuery.lazy`).
pub fn add_lazy(
    db: &Connection,
    instance: &str,
    digest: &mut AttentionDigest,
) -> Result<(), ApiError> {
    let through: i64 = db
        .query_row(
            "SELECT decision_seq FROM host_instances WHERE id=?1",
            [instance],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    let lazy = pending_lazy(db, instance, digest.seat.as_str(), through)?;
    digest.token.lazy = frontier_key(&lazy)?.map(|k| (k.decision_seq, k.event_offset));
    digest.lazy = Some(class(&lazy)?);
    Ok(())
}

/// The seat's pending invitations (optionally in one thread): one `WINDOW`
/// walk of `digest_pending_invitations`, newest first by (decision sequence,
/// ordinal), up to the host decision sequence `through`. Each windowed row is
/// checked against the source row (still pending, not cancelled, seat not
/// retired).
pub fn pending_invitations(
    db: &Connection,
    seat_id: &str,
    thread: Option<&str>,
    through: i64,
) -> Result<PendingSet, ApiError> {
    const SEAT: &str = "SELECT w.invitation_id,w.thread_id,w.created_decision_seq,w.ordinal,(i.state='pending' AND s.state!='retired' AND NOT EXISTS(SELECT 1 FROM invitation_cancellations c WHERE c.invitation_id=w.invitation_id) AND NOT EXISTS(SELECT 1 FROM invitation_rejections rejection WHERE rejection.invitation_id=w.invitation_id)) FROM (SELECT invitation_id,thread_id,created_decision_seq,ordinal FROM digest_pending_invitations INDEXED BY digest_pending_invitations_seat WHERE seat_id=?1 AND created_decision_seq<=?2 ORDER BY created_decision_seq DESC,ordinal DESC LIMIT ?3) w LEFT JOIN invitations i ON i.id=w.invitation_id LEFT JOIN seats s ON s.id=?1";
    const THREAD: &str = "SELECT w.invitation_id,w.thread_id,w.created_decision_seq,w.ordinal,(i.state='pending' AND s.state!='retired' AND NOT EXISTS(SELECT 1 FROM invitation_cancellations c WHERE c.invitation_id=w.invitation_id) AND NOT EXISTS(SELECT 1 FROM invitation_rejections rejection WHERE rejection.invitation_id=w.invitation_id)) FROM (SELECT invitation_id,thread_id,created_decision_seq,ordinal FROM digest_pending_invitations INDEXED BY digest_pending_invitations_thread WHERE seat_id=?1 AND thread_id=?4 AND created_decision_seq<=?2 ORDER BY created_decision_seq DESC,ordinal DESC LIMIT ?3) w LEFT JOIN invitations i ON i.id=w.invitation_id LEFT JOIN seats s ON s.id=?1";
    let map = |r: &rusqlite::Row<'_>| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, Option<bool>>(4)?.unwrap_or(false),
        ))
    };
    let window = match thread {
        None => rows(db, SEAT, params![seat_id, through, WINDOW_SQL], map)?,
        Some(thread) => rows(
            db,
            THREAD,
            params![seat_id, through, WINDOW_SQL, thread],
            map,
        )?,
    };
    let mut gather = Gather::default();
    gather.window(window.len());
    for (id, thread, seq, ordinal, valid) in window {
        if seq <= 0 || ordinal <= 0 {
            return Err(api_error(
                ErrorCode::StoreCorrupt,
                "invalid invitation publication key",
            ));
        }
        if valid {
            gather.add(PendingItem {
                id,
                thread_id: thread,
                key: (seq, ordinal),
            });
        }
    }
    Ok(gather.finish())
}

/// The seat's pending addressed receipts (optionally in one thread): three
/// `WINDOW` walks (materialized manifest receipts newest first by decision
/// sequence, still-staged manifest receipts newest first by staging ordinal,
/// pending physical rows newest first by ordinal), each windowed candidate
/// judged by `effective_receipt`. A physical row whose logical receipt is
/// manifest-backed is left to the manifest walks.
pub fn pending_receipts(
    db: &Connection,
    seat_id: &str,
    thread: Option<&str>,
) -> Result<PendingSet, ApiError> {
    const MATERIALIZED: &str = "SELECT sm.message_id FROM (SELECT preparation_id FROM digest_pending_manifest_receipts INDEXED BY digest_pending_manifest_receipts_seat WHERE seat_id=?1 AND decision_seq>0 ORDER BY decision_seq DESC,ordinal DESC LIMIT ?2) w LEFT JOIN send_manifests sm ON sm.preparation_id=w.preparation_id";
    const MATERIALIZED_THREAD: &str = "SELECT sm.message_id FROM (SELECT preparation_id FROM digest_pending_manifest_receipts INDEXED BY digest_pending_manifest_receipts_seat_thread WHERE seat_id=?1 AND thread_id=?3 AND decision_seq>0 ORDER BY decision_seq DESC,ordinal DESC LIMIT ?2) w LEFT JOIN send_manifests sm ON sm.preparation_id=w.preparation_id";
    const STAGED: &str = "SELECT sm.message_id FROM (SELECT preparation_id FROM digest_pending_manifest_receipts INDEXED BY digest_pending_manifest_receipts_seat WHERE seat_id=?1 AND decision_seq IS NULL ORDER BY ordinal DESC LIMIT ?2) w LEFT JOIN send_manifests sm ON sm.preparation_id=w.preparation_id";
    const STAGED_THREAD: &str = "SELECT sm.message_id FROM (SELECT preparation_id FROM digest_pending_manifest_receipts INDEXED BY digest_pending_manifest_receipts_seat_thread WHERE seat_id=?1 AND thread_id=?3 AND decision_seq IS NULL ORDER BY ordinal DESC LIMIT ?2) w LEFT JOIN send_manifests sm ON sm.preparation_id=w.preparation_id";
    const PHYSICAL: &str = "SELECT message_id FROM receipts INDEXED BY receipts_required_seat_pending WHERE seat_id=?1 AND state='pending' AND ack_required=1 ORDER BY ordinal DESC LIMIT ?2";
    const PHYSICAL_THREAD: &str = "SELECT message_id FROM receipts INDEXED BY receipts_required_thread_pending WHERE seat_id=?1 AND thread_id=?3 AND state='pending' AND ack_required=1 ORDER BY ordinal DESC LIMIT ?2";
    let mut gather = Gather::default();
    let mut hold = catch_up::HoldCache::new(seat_id);
    for (source, seat_sql, thread_sql) in [
        (ReceiptSource::Manifest, MATERIALIZED, MATERIALIZED_THREAD),
        (ReceiptSource::Manifest, STAGED, STAGED_THREAD),
        (ReceiptSource::Physical, PHYSICAL, PHYSICAL_THREAD),
    ] {
        let map = |r: &rusqlite::Row<'_>| r.get::<_, Option<String>>(0);
        let window = match thread {
            None => rows(db, seat_sql, params![seat_id, WINDOW_SQL], map)?,
            Some(thread) => rows(db, thread_sql, params![seat_id, WINDOW_SQL, thread], map)?,
        };
        gather.window(window.len());
        for message in window.into_iter().flatten() {
            gather.steps += 1;
            let Some(receipt) = effective::effective_receipt(db, &message, seat_id)? else {
                continue;
            };
            if receipt.source != source || receipt.state != EffectiveReceiptState::Pending {
                continue;
            }
            let seq = receipt.decision_seq.ok_or_else(|| {
                api_error(
                    ErrorCode::StoreCorrupt,
                    "pending receipt lacks logical publication key",
                )
            })?;
            if seq <= 0 {
                return Err(api_error(
                    ErrorCode::StoreCorrupt,
                    "invalid receipt publication key",
                ));
            }
            // Catch-up hold (spec §7): ordinary messages above an active
            // row's frontier are not pushed; a released range takes the row's
            // release key. Explicit reads never come through here.
            let Some(key) = hold.attention_key(
                db,
                &receipt.thread_id,
                &receipt.message_id,
                receipt.sequence,
                (seq, 0),
            )?
            else {
                continue;
            };
            gather.add(PendingItem {
                id: receipt.message_id,
                thread_id: receipt.thread_id,
                key,
            });
        }
    }
    Ok(gather.finish())
}

/// A warning's logical publication key: its event sequence and its offset
/// (the manifest warning offset, else the warn message's event offset).
fn judged_warning(
    db: &Connection,
    warning_id: &str,
    seat_id: Option<&str>,
) -> Result<Option<PendingItem>, ApiError> {
    // Attributed transitions use only the bounded notice delivery page.
    // Legacy condition/backlog walks must not revive already carried events.
    if let Some(seat_id) = seat_id {
        let projected_transition: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM digest_programmatic_warnings d WHERE d.seat_id=?1 AND d.warning_id=?2 AND EXISTS(SELECT 1 FROM warning_conditions c WHERE c.open_warning_id=d.warning_id OR c.clear_warning_id=d.warning_id))",
            params![seat_id,warning_id], |r| r.get(0),
        ).map_err(store_error)?;
        if projected_transition
            || (effective::is_invitation_rejection_notice(db, warning_id)?
                && db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM digest_programmatic_warnings WHERE seat_id=?1 AND warning_id=?2)",
                    params![seat_id, warning_id], |r| r.get::<_, bool>(0),
                ).map_err(store_error)?)
        {
            return Ok(None);
        }
    }
    let Some(warning) = effective::effective_warning_by_id(db, warning_id)? else {
        return Ok(None);
    };
    if let Some(seat_id) = seat_id
        && !effective::is_warning_recipient(db, warning_id, seat_id)?
    {
        return Ok(None);
    }
    if !effective::warning_condition_actionable(db, &warning)? {
        return Ok(None);
    }
    let offset: Option<i64> = db
        .query_row(
            "SELECT warning_offset FROM prepared_unavailable_warnings WHERE warning_id=?1",
            [warning_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    let offset = match offset {
        Some(offset) => offset,
        None => db
            .query_row(
                "SELECT event_offset FROM messages WHERE id=?1 AND kind='warn'",
                [warning_id],
                |r| r.get(0),
            )
            .map_err(store_error)?,
    };
    if warning.event_seq <= 0 || offset < 0 {
        return Err(api_error(
            ErrorCode::StoreCorrupt,
            "invalid warning publication key",
        ));
    }
    Ok(Some(PendingItem {
        id: warning.id,
        thread_id: warning.thread_id,
        key: (warning.event_seq, offset),
    }))
}

/// The seat's current occupant's offered notice frontier: the projection
/// ordinal of the last informational notice carried by a committed check-in
/// offer to that exact occupant (binding generation and execution), else 0.
/// A frontier recorded for another occupant (a predecessor) covers nothing.
pub fn notice_frontier(db: &Connection, seat_id: &str) -> Result<i64, ApiError> {
    let frontier: Option<i64> = db
        .query_row(
            "SELECT o.offered_ordinal FROM digest_notice_offer o JOIN seats s ON s.id=o.seat_id JOIN occupant_bindings b ON b.seat_id=s.id AND b.generation=s.generation AND b.ended_at IS NULL WHERE o.seat_id=?1 AND o.binding_generation=s.generation AND o.execution_id=b.execution_id",
            [seat_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    Ok(frontier.unwrap_or(0))
}

/// Informational warnings settle only when their exact attributed delivery
/// row was carried. Missing projection remains pending; legacy warnings keep
/// their original condition/offer semantics (`None`).
pub fn informational_notice_pending(
    db: &Connection,
    seat_id: &str,
    warning_id: &str,
) -> Result<Option<bool>, ApiError> {
    let informational: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM warning_conditions c WHERE c.open_warning_id=?1 OR c.clear_warning_id=?1) OR EXISTS(SELECT 1 FROM messages m JOIN service_notification_publications p ON p.message_id=m.id WHERE m.id=?1 AND m.kind='warn' AND m.author_kind='programmatic')",
        [warning_id], |r| r.get(0),
    ).map_err(store_error)?;
    if !informational && !effective::is_invitation_rejection_notice(db, warning_id)? {
        return Ok(None);
    }
    let ordinal: Option<i64> = db
        .query_row(
            "SELECT ordinal FROM digest_programmatic_warnings WHERE seat_id=?1 AND warning_id=?2",
            params![seat_id, warning_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    let frontier = notice_frontier(db, seat_id)?;
    Ok(Some(ordinal.is_none_or(|ordinal| ordinal > frontier)))
}

/// One seat-leading indexed probe of attributed notices above the exact
/// current occupant's carried frontier. Projection backlog is intentionally
/// absent: it cannot be carried by a CheckIn until attribution publishes it.
pub fn seat_has_pending_notices(db: &Connection, seat_id: &str) -> Result<bool, ApiError> {
    let frontier = notice_frontier(db, seat_id)?;
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM digest_programmatic_warnings INDEXED BY digest_programmatic_warnings_seat WHERE seat_id=?1 AND ordinal>?2)",
        params![seat_id,frontier], |r| r.get(0),
    ).map_err(store_error)
}

/// Is any attributed notice above the current occupant's carried frontier
/// one that wakes the seat (`warning_wakes_seat`)? One `WINDOW` walk, newest
/// first; a full window with no waking notice answers true, as the unnarrowed
/// probe did, so a large unoffered backlog never hides a waking notice.
pub fn seat_has_pending_wake_notices(db: &Connection, seat_id: &str) -> Result<bool, ApiError> {
    let frontier = notice_frontier(db, seat_id)?;
    let window: Vec<String> = rows(
        db,
        "SELECT warning_id FROM digest_programmatic_warnings INDEXED BY digest_programmatic_warnings_seat WHERE seat_id=?1 AND ordinal>?2 ORDER BY ordinal DESC LIMIT ?3",
        params![seat_id, frontier, WINDOW_SQL],
        |r| r.get(0),
    )?;
    let saturated = window.len() >= WINDOW;
    for warning in window {
        if warning_wakes_seat(db, seat_id, &warning)? {
            return Ok(true);
        }
    }
    Ok(saturated)
}

/// One informational notice of a check-in's offered page, with its
/// projection ordinal (the frontier key).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfferedNotice {
    pub notice: WarningRef,
    pub ordinal: i64,
}

/// The next page of informational notices not yet offered to the seat's
/// current occupant: one indexed walk of at most `limit` rows above the
/// occupant's frontier, oldest first by projection ordinal. Only a prefix of
/// this page that a check-in actually carries may be settled.
pub fn notice_offer_page(
    db: &Connection,
    seat_id: &str,
    limit: usize,
) -> Result<Vec<OfferedNotice>, ApiError> {
    const PAGE: &str = "SELECT w.ordinal,w.warning_id,w.thread_id,COALESCE(m.sequence,(SELECT sm.base_sequence+pw.warning_offset FROM prepared_unavailable_warnings pw JOIN send_manifests sm ON sm.preparation_id=pw.preparation_id WHERE pw.warning_id=w.warning_id)),w.event_seq FROM (SELECT ordinal,warning_id,thread_id,event_seq FROM digest_programmatic_warnings INDEXED BY digest_programmatic_warnings_seat WHERE seat_id=?1 AND ordinal>?2 ORDER BY ordinal LIMIT ?3) w LEFT JOIN messages m ON m.id=w.warning_id ORDER BY w.ordinal";
    let frontier = notice_frontier(db, seat_id)?;
    let limit = i64::try_from(limit)
        .map_err(|_| api_error(ErrorCode::InvalidRequest, "invalid notice page limit"))?;
    let page = rows(db, PAGE, params![seat_id, frontier, limit], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, i64>(4)?,
        ))
    })?;
    page.into_iter()
        .map(|(ordinal, warning, thread, sequence, event_seq)| {
            let corrupt = || api_error(ErrorCode::StoreCorrupt, "invalid notice publication key");
            Ok(OfferedNotice {
                notice: WarningRef {
                    warning: MessageId::new(&warning),
                    thread: ThreadId::parse(thread).map_err(|_| corrupt())?,
                    sequence: u64::try_from(sequence).map_err(|_| corrupt())?,
                    event_seq: u64::try_from(event_seq).map_err(|_| corrupt())?,
                },
                ordinal,
            })
        })
        .collect()
}

/// Settle exactly the informational notices a committed check-in offer
/// carried to the seat's occupant (`generation`, `execution`): advance that
/// occupant's monotone frontier to the last carried notice's projection
/// ordinal (wave-2 fix2 root decision (a)). `offered` must be the carried
/// prefix of `notice_offer_page`, read in the same transaction. One frontier
/// upsert: no projection row is deleted and nothing beyond the carried page
/// is covered. A frontier left by another occupant is replaced, never
/// inherited. Returns the rows written (0 when nothing was carried).
pub fn settle_offered_notices(
    tx: &Connection,
    seat_id: &str,
    generation: i64,
    execution: &str,
    offered: &[OfferedNotice],
) -> Result<usize, ApiError> {
    let Some(last) = offered.last() else {
        return Ok(0);
    };
    tx.execute(
        "INSERT INTO digest_notice_offer(seat_id,binding_generation,execution_id,offered_ordinal) VALUES (?1,?2,?3,?4) ON CONFLICT(seat_id) DO UPDATE SET offered_ordinal=CASE WHEN digest_notice_offer.binding_generation=excluded.binding_generation AND digest_notice_offer.execution_id=excluded.execution_id THEN MAX(digest_notice_offer.offered_ordinal,excluded.offered_ordinal) ELSE excluded.offered_ordinal END,binding_generation=excluded.binding_generation,execution_id=excluded.execution_id",
        params![seat_id, generation, execution, last.ordinal],
    )
    .map_err(store_error)
}

/// Settle the notices a check-in offer carries (`carried`, the offer's
/// `notices.items`) for the seat's current occupant (`generation`,
/// `execution`), in the offering transaction. The carried notices must be
/// exactly the oldest-first prefix of the occupant's unoffered page (one walk
/// of `carried.len()` rows); anything else is refused, so an offer can never
/// settle a notice it did not carry.
pub fn settle_carried_notices(
    tx: &Connection,
    seat_id: &str,
    generation: i64,
    execution: &str,
    carried: &[WarningRef],
) -> Result<usize, ApiError> {
    if carried.is_empty() {
        return Ok(0);
    }
    let page = notice_offer_page(tx, seat_id, carried.len())?;
    if page.len() != carried.len()
        || page
            .iter()
            .zip(carried)
            .any(|(offered, carried)| offered.notice != *carried)
    {
        return Err(api_error(
            ErrorCode::StoreCorrupt,
            "offered notices differ from the unoffered notice page",
        ));
    }
    settle_offered_notices(tx, seat_id, generation, execution, &page)
}

/// The seat's pending warnings still in the projection backlog, read once
/// per request: `WINDOW` walks (newest first) of outstanding warning
/// attribution jobs and of outstanding `send_attention` jobs. A job names a
/// candidate only while its projection has not reached the seat: an
/// unattributed warning (judged by `is_warning_recipient` and
/// `warning_condition_actionable`), a staged recipient of a published
/// programmatic warn notice not yet projected (so never carried by an offer
/// page), or a published manifest's unavailable warning whose attribution
/// job does not exist yet (judged). Staged candidates across all send jobs
/// are bounded by one further `WINDOW`.
pub fn warning_backlog(
    db: &Connection,
    seat_id: &str,
    check_budget: &dyn Fn() -> Result<(), ApiError>,
) -> Result<PendingSet, ApiError> {
    let mut gather = Gather::default();
    for status in ["pending", "failed"] {
        let jobs: Vec<String> = rows(
            db,
            "SELECT warning_id FROM warning_jobs INDEXED BY warning_jobs_ready WHERE status=?1 ORDER BY ordinal DESC LIMIT ?2",
            params![status, WINDOW_SQL],
            |r| r.get(0),
        )?;
        gather.window(jobs.len());
        for warning in jobs {
            gather.steps += 1;
            if let Some(item) = judged_warning(db, &warning, Some(seat_id))? {
                gather.add(item);
            }
        }
        check_budget()?;
    }
    let mut staged = 0usize;
    for status in ["pending", "failed"] {
        let jobs: Vec<String> = rows(
            db,
            "SELECT subject_id FROM work_jobs INDEXED BY work_jobs_ready WHERE status=?1 AND kind='send_attention' ORDER BY ordinal DESC LIMIT ?2",
            params![status, WINDOW_SQL],
            |r| r.get(0),
        )?;
        gather.window(jobs.len());
        for subject in jobs {
            if staged >= WINDOW {
                gather.saturated = true;
                break;
            }
            if let Some(preparation) = subject.strip_prefix("service-notify:") {
                let notice: Option<(String, String, i64, i64)> = db
                    .query_row(
                        "SELECT m.id,m.thread_id,m.decision_seq,m.event_offset FROM service_notification_recipients r JOIN service_notification_publications p ON p.preparation_id=r.preparation_id JOIN messages m ON m.id=p.message_id WHERE r.preparation_id=?1 AND r.seat_id=?2 AND m.kind='warn' AND m.author_kind='programmatic' AND NOT EXISTS(SELECT 1 FROM warning_recipients wr WHERE wr.warning_id=m.id AND wr.seat_id=?2)",
                        params![preparation, seat_id],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                    )
                    .optional()
                    .map_err(store_error)?;
                staged += 1;
                gather.steps += 1;
                // Not yet projected, so never carried by an offer page: pending.
                if let Some((id, thread, seq, offset)) = notice {
                    gather.add(PendingItem {
                        id,
                        thread_id: thread,
                        key: (seq, offset),
                    });
                }
                continue;
            }
            let remaining = (WINDOW - staged) as i64;
            let warnings: Vec<String> = rows(
                db,
                "SELECT w.warning_id FROM prepared_unavailable_warnings w WHERE w.preparation_id=?1 AND NOT EXISTS(SELECT 1 FROM warning_jobs j WHERE j.warning_id=w.warning_id) ORDER BY w.warning_offset LIMIT ?2",
                params![subject, remaining],
                |r| r.get(0),
            )?;
            staged += warnings.len().max(1);
            for warning in warnings {
                gather.steps += 1;
                if let Some(item) = judged_warning(db, &warning, Some(seat_id))? {
                    gather.add(item);
                }
            }
        }
        check_budget()?;
    }
    Ok(gather.finish())
}

/// The seat's projected pending warnings (optionally in one thread), apart
/// from the backlog: `WINDOW` walks of informational notices above the current
/// occupant's offered notice frontier (newest first by projection ordinal),
/// of attributed
/// legacy open-condition warning recipients and of open-condition warnings naming
/// the seat as affected (each newest first by source ordinal, per source,
/// judged by `warning_condition_actionable`).
pub fn projected_pending_warnings(
    db: &Connection,
    seat_id: &str,
    thread: Option<&str>,
    check_budget: &dyn Fn() -> Result<(), ApiError>,
) -> Result<PendingSet, ApiError> {
    const PROGRAMMATIC: &str = "SELECT warning_id,thread_id,event_seq,event_offset FROM digest_programmatic_warnings INDEXED BY digest_programmatic_warnings_seat WHERE seat_id=?1 AND ordinal>?4 ORDER BY ordinal DESC LIMIT ?2";
    const PROGRAMMATIC_THREAD: &str = "SELECT warning_id,thread_id,event_seq,event_offset FROM digest_programmatic_warnings INDEXED BY digest_programmatic_warnings_thread WHERE seat_id=?1 AND thread_id=?3 AND ordinal>?4 ORDER BY ordinal DESC LIMIT ?2";
    const RECIPIENT: &str = "SELECT warning_id FROM digest_open_warning_recipients INDEXED BY digest_open_warning_recipients_seat WHERE seat_id=?1 AND source=?4 ORDER BY source_ordinal DESC LIMIT ?2";
    const RECIPIENT_THREAD: &str = "SELECT warning_id FROM digest_open_warning_recipients INDEXED BY digest_open_warning_recipients_thread WHERE seat_id=?1 AND thread_id=?3 AND source=?4 ORDER BY source_ordinal DESC LIMIT ?2";
    const AFFECTED: &str = "SELECT warning_id FROM digest_open_warnings INDEXED BY digest_open_warnings_affected WHERE affected_seat_id=?1 AND source=?4 ORDER BY source_ordinal DESC LIMIT ?2";
    const AFFECTED_THREAD: &str = "SELECT warning_id FROM digest_open_warnings INDEXED BY digest_open_warnings_affected_thread WHERE affected_seat_id=?1 AND thread_id=?3 AND source=?4 ORDER BY source_ordinal DESC LIMIT ?2";
    let mut gather = Gather::default();
    let map = |r: &rusqlite::Row<'_>| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
        ))
    };
    let frontier = notice_frontier(db, seat_id)?;
    let programmatic = match thread {
        None => rows(
            db,
            PROGRAMMATIC,
            params![seat_id, WINDOW_SQL, None::<String>, frontier],
            map,
        )?,
        Some(thread) => rows(
            db,
            PROGRAMMATIC_THREAD,
            params![seat_id, WINDOW_SQL, thread, frontier],
            map,
        )?,
    };
    gather.window(programmatic.len());
    for (id, thread, seq, offset) in programmatic {
        if seq <= 0 || offset < 0 {
            return Err(api_error(
                ErrorCode::StoreCorrupt,
                "invalid warning publication key",
            ));
        }
        gather.add(PendingItem {
            id,
            thread_id: thread,
            key: (seq, offset),
        });
    }
    check_budget()?;
    // An attributed recipient row is recipient evidence, and the affected
    // seat is a recipient by the canonical rule: only the condition is judged.
    for (seat_sql, thread_sql) in [(RECIPIENT, RECIPIENT_THREAD), (AFFECTED, AFFECTED_THREAD)] {
        for source in ["job", "prepared"] {
            let window: Vec<String> = match thread {
                None => rows(
                    db,
                    seat_sql,
                    params![seat_id, WINDOW_SQL, None::<String>, source],
                    |r| r.get(0),
                )?,
                Some(thread) => rows(
                    db,
                    thread_sql,
                    params![seat_id, WINDOW_SQL, thread, source],
                    |r| r.get(0),
                )?,
            };
            gather.window(window.len());
            for warning in window {
                gather.steps += 1;
                if let Some(item) = judged_warning(db, &warning, Some(seat_id))? {
                    gather.add(item);
                }
            }
            check_budget()?;
        }
    }
    Ok(gather.finish())
}

/// The seat's pending warnings (optionally in one thread): the projected
/// walks plus the request's `backlog` (read once by `warning_backlog`). This
/// is the canonical pending-warning set that the digest, check-in and inbox
/// count (wave-2 (a); an informational notice settles when a committed offer
/// carries it, wave-2 fix2 (a)).
pub fn pending_warnings(
    db: &Connection,
    seat_id: &str,
    thread: Option<&str>,
    backlog: &PendingSet,
    check_budget: &dyn Fn() -> Result<(), ApiError>,
) -> Result<PendingSet, ApiError> {
    let projected = projected_pending_warnings(db, seat_id, thread, check_budget)?;
    let mut gather = Gather {
        steps: projected.work_steps,
        ..Gather::default()
    };
    gather.merge(&projected);
    match thread {
        None => gather.merge(backlog),
        Some(thread) => gather.merge(&backlog.in_thread(thread)),
    }
    Ok(gather.finish())
}

/// The seat's whole pending-warning set: backlog plus projected walks.
pub fn seat_pending_warnings(
    db: &Connection,
    seat_id: &str,
    check_budget: &dyn Fn() -> Result<(), ApiError>,
) -> Result<PendingSet, ApiError> {
    let backlog = warning_backlog(db, seat_id, check_budget)?;
    let mut set = pending_warnings(db, seat_id, None, &backlog, check_budget)?;
    set.work_steps += backlog.work_steps;
    Ok(set)
}

/// One O(1) existence probe of a pending access path: the index it must use
/// and the statement. `?1` is the seat; the programmatic probe also takes
/// `?2`, the seat's offered notice frontier.
pub struct WakeProbe {
    pub index: &'static str,
    pub sql: &'static str,
}

/// The six pending access paths wake discovery probes per live non-human
/// seat (spec D2), in probe order. Each is an `INDEXED BY` seat-leading
/// existence lookup. `wake_work` is deliberately not probed.
pub const WAKE_PROBES: [WakeProbe; 6] = [
    WakeProbe {
        index: "digest_pending_invitations_seat",
        sql: "SELECT EXISTS(SELECT 1 FROM digest_pending_invitations INDEXED BY digest_pending_invitations_seat WHERE seat_id=?1)",
    },
    WakeProbe {
        index: "digest_pending_manifest_receipts_seat",
        sql: "SELECT EXISTS(SELECT 1 FROM digest_pending_manifest_receipts INDEXED BY digest_pending_manifest_receipts_seat WHERE seat_id=?1)",
    },
    WakeProbe {
        index: "receipts_required_seat_pending",
        sql: "SELECT EXISTS(SELECT 1 FROM receipts INDEXED BY receipts_required_seat_pending WHERE seat_id=?1 AND state='pending' AND ack_required=1)",
    },
    WakeProbe {
        index: "digest_open_warning_recipients_seat",
        sql: "SELECT EXISTS(SELECT 1 FROM digest_open_warning_recipients INDEXED BY digest_open_warning_recipients_seat WHERE seat_id=?1)",
    },
    WakeProbe {
        index: "digest_open_warnings_affected",
        sql: "SELECT EXISTS(SELECT 1 FROM digest_open_warnings INDEXED BY digest_open_warnings_affected WHERE affected_seat_id=?1)",
    },
    WakeProbe {
        index: "digest_programmatic_warnings_seat",
        sql: "SELECT EXISTS(SELECT 1 FROM digest_programmatic_warnings INDEXED BY digest_programmatic_warnings_seat WHERE seat_id=?1 AND ordinal>?2)",
    },
];

/// Does any pending access path hold a row for the seat? At most six O(1)
/// probes, stopping at the first hit. A hit is only a reason to examine the
/// seat: the canonical judges still decide whether the work is actionable.
pub fn seat_has_pending_rows(db: &Connection, seat_id: &str) -> Result<bool, ApiError> {
    for probe in &WAKE_PROBES {
        let hit: bool = if probe.index == "digest_programmatic_warnings_seat" {
            let frontier = notice_frontier(db, seat_id)?;
            db.prepare_cached(probe.sql)
                .map_err(store_error)?
                .query_row(params![seat_id, frontier], |r| r.get(0))
        } else {
            db.prepare_cached(probe.sql)
                .map_err(store_error)?
                .query_row([seat_id], |r| r.get(0))
        }
        .map_err(store_error)?;
        if hit {
            return Ok(true);
        }
    }
    Ok(false)
}

/// A seat's wake attention: the canonical judgement of what is pending for
/// it, with the decision-sequence position it was read at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WakeSeatAttention {
    pub attention: effective::EffectiveSeatAttention,
    /// `host_instances.decision_seq`, read once before the walks. It is the
    /// position `reserve` re-reads and refuses on any change; it is not
    /// derived from the walked rows (an ACK that settles a receipt adds no
    /// newer row).
    pub decision_seq: i64,
}

/// May this pending warning wake `seat_id`? A canonical condition transition
/// is an informational notice for every member (TRUST-POLICY A7): it wakes
/// only the affected seat of a still-open condition, the seat that owes the
/// overdue ACK or invitation answer. Other members, and every clear, receive
/// it on their next check-in offer instead of a prompt whose inbox has
/// nothing for them. Other warnings keep their judged actionability. Unique
/// index probes only.
pub fn warning_wakes_seat(
    db: &Connection,
    seat_id: &str,
    warning_id: &str,
) -> Result<bool, ApiError> {
    if effective::is_invitation_rejection_notice(db, warning_id)? {
        return Ok(false);
    }
    db.query_row(
        "SELECT CASE WHEN EXISTS(SELECT 1 FROM warning_conditions WHERE open_warning_id=?1) OR EXISTS(SELECT 1 FROM warning_conditions WHERE clear_warning_id=?1) THEN EXISTS(SELECT 1 FROM warning_conditions WHERE open_warning_id=?1 AND affected_seat_id=?2 AND clear_warning_id IS NULL) ELSE 1 END",
        params![warning_id, seat_id],
        |r| r.get(0),
    )
    .map_err(store_error)
}

/// Narrow a seat's pending-warning set to the warnings that wake it
/// (`warning_wakes_seat`). A saturated walk can hide an older waking notice (a
/// service notice behind a full window of other seats' transitions).
/// Narrowing it to nothing would turn that unknown into "no warning" and skip
/// the conservative offer probe, so a saturated set with no waking item keeps
/// the unnarrowed answer instead. Native wake (`wake_seat_attention`) and the
/// mod channel's attention fingerprint (`seats::mod_seat_view`) share it.
pub fn wake_warnings(
    db: &Connection,
    seat_id: &str,
    mut warnings: PendingSet,
) -> Result<PendingSet, ApiError> {
    let mut wakes = Vec::with_capacity(warnings.items.len());
    for item in &warnings.items {
        if warning_wakes_seat(db, seat_id, &item.id)? {
            wakes.push(item.clone());
        }
    }
    if !(wakes.is_empty() && warnings.saturated) {
        warnings.items = wakes;
    }
    Ok(warnings)
}

/// The wake attention of one seat, from the newest-first per-source walks
/// and canonical judges (`effective_receipt`, `is_warning_recipient`,
/// `warning_condition_actionable`), with warnings narrowed to those that
/// wake the seat (`warning_wakes_seat`): O(`WINDOW` x sources), independent
/// of retained history. Call it inside the caller's read transaction; the
/// decision sequence is read first, so the walks see at least that state.
pub fn wake_seat_attention(db: &Connection, seat_id: &str) -> Result<WakeSeatAttention, ApiError> {
    let decision_seq: i64 = db
        .query_row(
            "SELECT h.decision_seq FROM seats s JOIN host_instances h ON h.id=s.instance_id WHERE s.id=?1",
            [seat_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?
        .ok_or_else(|| api_error(ErrorCode::NotFound, "seat missing"))?;
    let invitations = pending_invitations(db, seat_id, None, decision_seq)?;
    let receipts = pending_receipts(db, seat_id, None)?;
    let warnings = wake_warnings(db, seat_id, seat_pending_warnings(db, seat_id, &|| Ok(()))?)?;
    let latest_warning_seq = warnings.items.first().map(|item| item.key.0);
    Ok(WakeSeatAttention {
        attention: effective::EffectiveSeatAttention {
            has_pending_invitation: !invitations.items.is_empty(),
            has_pending_receipt: !receipts.items.is_empty(),
            latest_warning_seq,
            frontier: LogicalAttentionFrontier {
                invitation: frontier_key(&invitations)?,
                addressed_receipt: frontier_key(&receipts)?,
                actionable_warning: frontier_key(&warnings)?,
            },
        },
        decision_seq,
    })
}

#[cfg(test)]
#[path = "../../tests/store/attention.rs"]
mod tests;
