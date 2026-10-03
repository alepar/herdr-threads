//! Canonical logical receipt and warning projection.

use crate::ports::LogicalAttentionFrontier;
use crate::protocol::{
    pagination::InboxCursorState,
    results::{ApiError, ErrorCode},
    time::{CallBudget, Clock, UtcMillis},
};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

use super::{
    connection::{api_error, store_error},
    schema::checked_deadline,
};

/// The one read path for a target after a hidden snapshot publishes. A fresh
/// current-target observation wins only within the same boot/epoch and with a
/// strictly later trusted sequence. Missing data stays missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectiveObservationSource {
    Published,
    NewerCurrentTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveObservation {
    pub source: EffectiveObservationSource,
    pub target_id: String,
    pub host_boot: String,
    pub epoch: i64,
    pub structural_generation: i64,
    pub observation_sequence: i64,
    pub connection_epoch: Option<i64>,
    pub incarnation: Option<String>,
    pub incarnation_source_kind: Option<String>,
    pub terminal_id: Option<String>,
    pub occupancy: String,
    pub ui_state: String,
    pub verified_execution: Option<String>,
    pub top_level_occupant: bool,
}

pub fn effective_observation(
    db: &Connection,
    instance: &str,
    target: &str,
) -> Result<Option<EffectiveObservation>, ApiError> {
    type CurrentColumns = Option<(Option<String>, Option<String>, i64, i64, i64)>;
    let current: CurrentColumns = db.query_row(
        "SELECT active_snapshot_id,host_boot,host_epoch,observation_sequence,invalidation_revision FROM host_instances WHERE id=?1",
        [instance], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
    ).optional().map_err(store_error)?;
    let Some((active_id, host_boot, host_epoch, publication_seq, invalidation_revision)) = current
    else {
        return Ok(None);
    };
    // A denied or partial host capture invalidates the prior publication at
    // one scalar write; per-seat projection may follow in bounded quanta.
    if invalidation_revision > 0 {
        let published_revision: Option<i64> = match active_id.as_deref() {
            Some(id) => db.query_row(
                "SELECT published_invalidation_revision FROM snapshot_generations WHERE id=?1 AND status='published'",
                [id], |r| r.get(0),
            ).optional().map_err(store_error)?.flatten(),
            None => None,
        };
        if published_revision.is_none_or(|revision| revision < invalidation_revision) {
            return Ok(None);
        }
    }
    let published = if let Some(active_id) = active_id.as_deref() {
        db.query_row(
            "SELECT g.host_boot,g.epoch,t.generation,t.observation_sequence,t.terminal_id,t.occupancy,t.ui_state,t.verified_execution,t.top_level_occupant,t.connection_epoch,CASE WHEN t.incarnation_source_kind IS NOT NULL THEN g.incarnation END,t.incarnation_source_kind FROM snapshot_targets t JOIN snapshot_generations g ON g.id=t.generation_id WHERE t.generation_id=?1 AND t.target_id=?2 AND g.status='published'",
            params![active_id,target], |r| Ok(EffectiveObservation {
                source: EffectiveObservationSource::Published,
                target_id: target.into(), host_boot:r.get(0)?, epoch:r.get(1)?,
                structural_generation:r.get(2)?, observation_sequence:r.get(3)?,
                terminal_id:r.get(4)?, occupancy:r.get(5)?, ui_state:r.get(6)?,
                verified_execution:r.get(7)?, top_level_occupant:r.get::<_,i64>(8)? != 0,
                connection_epoch:r.get(9)?,incarnation:r.get(10)?,incarnation_source_kind:r.get(11)?,
            }),
        ).optional().map_err(store_error)?
    } else {
        None
    };
    let newer = db.query_row(
        "SELECT host_boot,epoch,generation,observation_sequence,terminal_id,occupancy,ui_state,verified_execution,top_level_occupant,connection_epoch,incarnation,incarnation_source_kind FROM observed_targets WHERE instance_id=?1 AND target_id=?2 AND provenance='fresh'",
        params![instance,target], |r| Ok(EffectiveObservation {
            source: EffectiveObservationSource::NewerCurrentTarget,
            target_id:target.into(), host_boot:r.get(0)?, epoch:r.get(1)?,
            structural_generation:r.get(2)?, observation_sequence:r.get(3)?,
            terminal_id:r.get(4)?, occupancy:r.get(5)?, ui_state:r.get(6)?,
            verified_execution:r.get(7)?, top_level_occupant:r.get::<_,i64>(8)? != 0,
            connection_epoch:r.get(9)?,incarnation:r.get(10)?,incarnation_source_kind:r.get(11)?,
        }),
    ).optional().map_err(store_error)?;
    let Some(newer) = newer else {
        return Ok(published);
    };
    if host_boot.as_deref() == Some(newer.host_boot.as_str())
        && host_epoch == newer.epoch
        && (active_id.is_none() || newer.observation_sequence > publication_seq)
    {
        Ok(Some(newer))
    } else {
        Ok(published)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectiveRecoveryDisposition {
    Owned(String),
    ExplicitHold,
    BaselineHeld,
    Released,
    UnambiguousUnclaimed,
    CreatedAfterBaseline,
    Unknown,
}

/// Ownership and operator repair are durable overlays. A retained baseline
/// hold applies only to members of the first complete recovery generation.
pub fn effective_recovery_disposition(
    db: &Connection,
    instance: &str,
    target: &str,
) -> Result<EffectiveRecoveryDisposition, ApiError> {
    let owner: Option<String> = db.query_row(
        "SELECT id FROM seats WHERE instance_id=?1 AND target_id=?2 AND state='resolved' LIMIT 1",
        params![instance,target], |r| r.get(0),
    ).optional().map_err(store_error)?;
    if let Some(owner) = owner {
        return Ok(EffectiveRecoveryDisposition::Owned(owner));
    }
    type RowColumns = Option<(Option<String>, i64, Option<String>, Option<String>, i64)>;
    let row: RowColumns = db.query_row(
        "SELECT host_boot,host_epoch,recovery_baseline_generation_id,active_snapshot_id,baseline_hold_unclaimed FROM host_instances WHERE id=?1",
        [instance],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
    ).optional().map_err(store_error)?;
    let Some((boot, epoch, baseline, active, hold_unclaimed)) = row else {
        return Ok(EffectiveRecoveryDisposition::Unknown);
    };
    let explicit: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM recovery_holds WHERE instance_id=?1 AND target_id=?2 AND baseline_boot=?3 AND baseline_epoch=?4 AND released_at IS NULL)",
        params![instance,target,boot,epoch],|r|r.get(0),
    ).map_err(store_error)?;
    if explicit {
        return Ok(EffectiveRecoveryDisposition::ExplicitHold);
    }
    let Some(observation) = effective_observation(db, instance, target)? else {
        return Ok(EffectiveRecoveryDisposition::Unknown);
    };
    if observation.host_boot != boot.as_deref().unwrap_or("") || observation.epoch != epoch {
        return Ok(EffectiveRecoveryDisposition::Unknown);
    }
    let Some(baseline) = baseline else {
        return Ok(EffectiveRecoveryDisposition::Unknown);
    };
    let member: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM snapshot_targets WHERE generation_id=?1 AND target_id=?2)",
            params![baseline, target],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if member {
        let released: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM recovery_baseline_releases WHERE instance_id=?1 AND baseline_generation_id=?2 AND target_id=?3)",
            params![instance,baseline,target],|r|r.get(0),
        ).map_err(store_error)?;
        if released {
            return Ok(EffectiveRecoveryDisposition::Released);
        }
        return Ok(if hold_unclaimed != 0 {
            EffectiveRecoveryDisposition::BaselineHeld
        } else {
            EffectiveRecoveryDisposition::UnambiguousUnclaimed
        });
    }
    if active.is_some() {
        Ok(EffectiveRecoveryDisposition::CreatedAfterBaseline)
    } else {
        Ok(EffectiveRecoveryDisposition::Unknown)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectiveReceiptState {
    Pending,
    Acknowledged,
    RecipientRetired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveReceipt {
    pub source: ReceiptSource,
    pub message_id: String,
    pub thread_id: String,
    pub seat_id: String,
    pub sequence: i64,
    pub source_ordinal: i64,
    pub decision_seq: Option<i64>,
    pub decision_at: i64,
    pub frozen_duration_ms: i64,
    pub state: EffectiveReceiptState,
    pub available_at: Option<i64>,
    pub deadline_at: Option<i64>,
    pub warning_message_id: Option<String>,
    pub ack_actor_seat_id: Option<String>,
    pub ack_generation: Option<i64>,
    pub ack_observation: Option<String>,
    pub acked_at: Option<i64>,
    pub retired_at: Option<i64>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptSource {
    Manifest,
    Physical,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiptScanScope {
    /// Scan a published message's complete logical recipient set.
    Message(String),
    Seat(String),
    /// Pending physical rows plus still-unsettled staged recipients (the v8
    /// pending projection). Settled receipts are never candidates; callers
    /// that need settled history use `Seat` or `Message`.
    Thread(String),
    /// Global ordinal traversal counts rows belonging to other instances too.
    Instance(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiptScanPosition {
    pub physical_after: i64,
    pub manifest_after: i64,
    pub physical_high_water: i64,
    pub manifest_high_water: i64,
    pub next_manifest: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveReceiptSlice {
    pub items: Vec<EffectiveReceipt>,
    pub position: ReceiptScanPosition,
    /// Count of indexed candidates examined, including filtered/deduplicated rows.
    pub visited: u16,
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveSeatAttention {
    pub has_pending_invitation: bool,
    pub has_pending_receipt: bool,
    /// Greatest logical warning decision sequence reachable by this seat.
    pub latest_warning_seq: Option<i64>,
    /// Complete immutable publication maxima, available only after every
    /// frozen indexed source has been examined.
    pub frontier: LogicalAttentionFrontier,
}

fn receipt_high_water(
    db: &Connection,
    scope: &ReceiptScanScope,
    manifest: bool,
) -> Result<i64, ApiError> {
    let (sql, key): (&str, Option<&str>) = match (scope, manifest) {
        (ReceiptScanScope::Seat(seat), false) => (
            "SELECT COALESCE(MAX(ordinal),0) FROM receipts WHERE seat_id=?1",
            Some(seat),
        ),
        (ReceiptScanScope::Seat(seat), true) => (
            "SELECT COALESCE(MAX(ordinal),0) FROM prepared_recipients WHERE seat_id=?1",
            Some(seat),
        ),
        (ReceiptScanScope::Thread(thread), false) => (
            "SELECT COALESCE(MAX(ordinal),0) FROM receipts WHERE thread_id=?1 AND state='pending'",
            Some(thread),
        ),
        (ReceiptScanScope::Thread(thread), true) => (
            "SELECT COALESCE(MAX(ordinal),0) FROM prepared_recipients WHERE thread_id=?1",
            Some(thread),
        ),
        (ReceiptScanScope::Message(message), false) => (
            "SELECT COALESCE(MAX(ordinal),0) FROM receipts WHERE message_id=?1",
            Some(message),
        ),
        (ReceiptScanScope::Message(message), true) => (
            "SELECT COALESCE(MAX(pr.ordinal),0) FROM send_manifests sm JOIN prepared_recipients pr ON pr.preparation_id=sm.preparation_id WHERE sm.message_id=?1",
            Some(message),
        ),
        (ReceiptScanScope::Instance(_), false) => {
            ("SELECT COALESCE(MAX(ordinal),0) FROM receipts", None)
        }
        (ReceiptScanScope::Instance(_), true) => (
            "SELECT COALESCE(MAX(ordinal),0) FROM prepared_recipients",
            None,
        ),
    };
    match key {
        Some(key) => db.query_row(sql, [key], |r| r.get(0)).map_err(store_error),
        None => db.query_row(sql, [], |r| r.get(0)).map_err(store_error),
    }
}

fn receipt_candidate(
    db: &Connection,
    scope: &ReceiptScanScope,
    manifest: bool,
    after: i64,
    high_water: i64,
) -> Result<Option<(String, String, i64)>, ApiError> {
    let (sql, key): (&str, Option<&str>) = match (scope, manifest) {
        (ReceiptScanScope::Seat(seat), false) => (
            "SELECT message_id,seat_id,ordinal FROM receipts WHERE seat_id=?1 AND ordinal>?2 AND ordinal<=?3 ORDER BY ordinal LIMIT 1",
            Some(seat),
        ),
        (ReceiptScanScope::Seat(seat), true) => (
            "SELECT sm.message_id,pr.seat_id,pr.ordinal FROM prepared_recipients pr LEFT JOIN send_manifests sm ON sm.preparation_id=pr.preparation_id WHERE pr.seat_id=?1 AND pr.ordinal>?2 AND pr.ordinal<=?3 ORDER BY pr.ordinal LIMIT 1",
            Some(seat),
        ),
        (ReceiptScanScope::Thread(thread), false) => (
            "SELECT message_id,seat_id,ordinal FROM receipts WHERE thread_id=?1 AND state='pending' AND ordinal>?2 AND ordinal<=?3 ORDER BY ordinal LIMIT 1",
            Some(thread),
        ),
        // Pending-only: the v8 projection holds every staged recipient until
        // its terminal settlement, keyed by the staged row's thread and
        // ordinal, so settled history is never visited (digest fix3).
        (ReceiptScanScope::Thread(thread), true) => (
            "SELECT sm.message_id,d.seat_id,d.ordinal FROM digest_pending_manifest_receipts d LEFT JOIN send_manifests sm ON sm.preparation_id=d.preparation_id WHERE d.thread_id=?1 AND d.ordinal>?2 AND d.ordinal<=?3 ORDER BY d.ordinal LIMIT 1",
            Some(thread),
        ),
        (ReceiptScanScope::Message(message), false) => (
            "SELECT message_id,seat_id,ordinal FROM receipts WHERE message_id=?1 AND ordinal>?2 AND ordinal<=?3 ORDER BY ordinal LIMIT 1",
            Some(message),
        ),
        (ReceiptScanScope::Message(message), true) => (
            "SELECT sm.message_id,pr.seat_id,pr.ordinal FROM send_manifests sm JOIN prepared_recipients pr ON pr.preparation_id=sm.preparation_id WHERE sm.message_id=?1 AND pr.ordinal>?2 AND pr.ordinal<=?3 ORDER BY pr.ordinal LIMIT 1",
            Some(message),
        ),
        (ReceiptScanScope::Instance(_), false) => (
            "SELECT message_id,seat_id,ordinal FROM receipts WHERE ordinal>?2 AND ordinal<=?3 ORDER BY ordinal LIMIT 1",
            None,
        ),
        (ReceiptScanScope::Instance(_), true) => (
            "SELECT sm.message_id,pr.seat_id,pr.ordinal FROM prepared_recipients pr LEFT JOIN send_manifests sm ON sm.preparation_id=pr.preparation_id WHERE pr.ordinal>?2 AND pr.ordinal<=?3 ORDER BY pr.ordinal LIMIT 1",
            None,
        ),
    };
    db.query_row(sql, params![key, after, high_water], |r| {
        Ok((
            r.get::<_, Option<String>>(0)?.unwrap_or_default(),
            r.get(1)?,
            r.get(2)?,
        ))
    })
    .optional()
    .map_err(store_error)
}

/// Visit at most `max_candidates` indexed physical and staged-recipient rows.
/// Published manifest recipients are canonical; physical projection rows for
/// the same logical receipt are counted but omitted. The two captured source
/// high waters prevent a later projection from moving a cursor's identity.
pub fn scan_effective_receipts(
    db: &Connection,
    scope: &ReceiptScanScope,
    position: Option<ReceiptScanPosition>,
    max_candidates: u16,
) -> Result<EffectiveReceiptSlice, ApiError> {
    if max_candidates == 0 || max_candidates > 100 {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid receipt scan bound",
        ));
    }
    let mut position = match position {
        Some(position) => position,
        None => ReceiptScanPosition {
            physical_after: 0,
            manifest_after: 0,
            physical_high_water: receipt_high_water(db, scope, false)?,
            manifest_high_water: receipt_high_water(db, scope, true)?,
            next_manifest: false,
        },
    };
    if position.physical_after < 0
        || position.manifest_after < 0
        || position.physical_after > position.physical_high_water
        || position.manifest_after > position.manifest_high_water
    {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid receipt scan position",
        ));
    }
    let mut items = Vec::new();
    let mut visited = 0;
    while visited < max_candidates {
        let preferred = position.next_manifest;
        let candidate = receipt_candidate(
            db,
            scope,
            preferred,
            if preferred {
                position.manifest_after
            } else {
                position.physical_after
            },
            if preferred {
                position.manifest_high_water
            } else {
                position.physical_high_water
            },
        )?;
        let (manifest, candidate) = if let Some(candidate) = candidate {
            (preferred, candidate)
        } else {
            let other = !preferred;
            let Some(candidate) = receipt_candidate(
                db,
                scope,
                other,
                if other {
                    position.manifest_after
                } else {
                    position.physical_after
                },
                if other {
                    position.manifest_high_water
                } else {
                    position.physical_high_water
                },
            )?
            else {
                break;
            };
            (other, candidate)
        };
        position.next_manifest = !manifest;
        if manifest {
            position.manifest_after = candidate.2;
        } else {
            position.physical_after = candidate.2;
        }
        visited += 1;
        if candidate.0.is_empty() {
            continue;
        } // unpublished preparation
        let Some(receipt) = effective_receipt(db, &candidate.0, &candidate.1)? else {
            continue;
        };
        if !manifest && receipt.source == ReceiptSource::Manifest {
            continue;
        }
        if let ReceiptScanScope::Instance(instance) = scope {
            let belongs: bool = db
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM threads WHERE id=?1 AND instance_id=?2)",
                    params![receipt.thread_id, instance],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            if !belongs {
                continue;
            }
        }
        items.push(receipt);
    }
    let has_more = receipt_candidate(
        db,
        scope,
        false,
        position.physical_after,
        position.physical_high_water,
    )?
    .is_some()
        || receipt_candidate(
            db,
            scope,
            true,
            position.manifest_after,
            position.manifest_high_water,
        )?
        .is_some();
    Ok(EffectiveReceiptSlice {
        items,
        position,
        visited,
        has_more,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveWarning {
    pub id: String,
    pub thread_id: String,
    pub sequence: i64,
    pub event_seq: i64,
    pub event_json: String,
    pub source_message_id: Option<String>,
    pub affected_seat_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectiveTimelineEntry {
    Physical {
        id: String,
        sequence: i64,
        kind: String,
    },
    PublishedWarning(EffectiveWarning),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelinePosition {
    pub after_sequence: i64,
    pub high_water_sequence: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveTimelineSlice {
    pub entries: Vec<EffectiveTimelineEntry>,
    pub position: TimelinePosition,
    pub visited: u16,
    pub has_more: bool,
}

/// Global publication order within one host instance. Every supported
/// producer records a non-null decision sequence; direct events in one batch
/// have distinct offsets, and a send's ordinary message uses offset zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobalLogicalKinds {
    Ordinary,
    Warnings,
    All,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobalLogicalSource {
    Physical,
    PublishedWarning,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalLogicalCandidate {
    pub id: String,
    pub thread_id: String,
    pub decision_seq: i64,
    pub event_offset: i64,
    pub kind: String,
    pub source: GlobalLogicalSource,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalLogicalPosition {
    pub after_decision_seq: i64,
    pub after_event_offset: i64,
    pub high_water_decision_seq: i64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalLogicalSlice {
    pub candidates: Vec<GlobalLogicalCandidate>,
    pub position: GlobalLogicalPosition,
    /// Indexed rows returned by the physical and manifest seeks, including
    /// duplicate probes and filtered rows.
    pub visited: u16,
    pub has_more: bool,
}

fn next_physical_logical(
    db: &Connection,
    instance: &str,
    kinds: GlobalLogicalKinds,
    position: &GlobalLogicalPosition,
) -> Result<Option<GlobalLogicalCandidate>, ApiError> {
    let sql = match kinds {
        GlobalLogicalKinds::Ordinary => {
            "SELECT id,thread_id,decision_seq,event_offset,kind FROM messages WHERE instance_id=?1 AND kind='ordinary' AND (decision_seq,event_offset)>(?2,?3) AND decision_seq<=?4 ORDER BY decision_seq,event_offset LIMIT 1"
        }
        GlobalLogicalKinds::Warnings => {
            "SELECT id,thread_id,decision_seq,event_offset,kind FROM messages WHERE instance_id=?1 AND kind='warn' AND (decision_seq,event_offset)>(?2,?3) AND decision_seq<=?4 ORDER BY decision_seq,event_offset LIMIT 1"
        }
        GlobalLogicalKinds::All => {
            "SELECT id,thread_id,decision_seq,event_offset,kind FROM messages WHERE instance_id=?1 AND (decision_seq,event_offset)>(?2,?3) AND decision_seq<=?4 ORDER BY decision_seq,event_offset LIMIT 1"
        }
    };
    db.query_row(
        sql,
        params![
            instance,
            position.after_decision_seq,
            position.after_event_offset,
            position.high_water_decision_seq
        ],
        |r| {
            Ok(GlobalLogicalCandidate {
                id: r.get(0)?,
                thread_id: r.get(1)?,
                decision_seq: r.get(2)?,
                event_offset: r.get(3)?,
                kind: r.get(4)?,
                source: GlobalLogicalSource::Physical,
            })
        },
    )
    .optional()
    .map_err(store_error)
}

fn next_manifest_warning_logical(
    db: &Connection,
    instance: &str,
    position: &GlobalLogicalPosition,
) -> Result<Option<GlobalLogicalCandidate>, ApiError> {
    let current: Option<(String,String,i64,i64)> = db.query_row(
        "SELECT preparation_id,thread_id,decision_seq,warning_count FROM send_manifests WHERE instance_id=?1 AND decision_seq=?2 AND warning_count>?3",
        params![instance,position.after_decision_seq,position.after_event_offset],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
    ).optional().map_err(store_error)?;
    let (preparation, thread, seq, offset) = if let Some((preparation, thread, seq, _)) = current {
        (
            preparation,
            thread,
            seq,
            position.after_event_offset.checked_add(1).ok_or_else(|| {
                api_error(ErrorCode::SequenceExhausted, "warning offset exhausted")
            })?,
        )
    } else {
        let next:Option<(String,String,i64)>=db.query_row(
            "SELECT preparation_id,thread_id,decision_seq FROM send_manifests WHERE instance_id=?1 AND decision_seq>?2 AND decision_seq<=?3 AND warning_count>0 ORDER BY decision_seq LIMIT 1",
            params![instance,position.after_decision_seq,position.high_water_decision_seq],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
        ).optional().map_err(store_error)?;
        let Some((preparation, thread, seq)) = next else {
            return Ok(None);
        };
        (preparation, thread, seq, 1)
    };
    let id:String=db.query_row(
        "SELECT warning_id FROM prepared_unavailable_warnings WHERE preparation_id=?1 AND warning_offset=?2",
        params![preparation,offset],|r|r.get(0),
    ).optional().map_err(store_error)?
        .ok_or_else(||api_error(ErrorCode::StoreCorrupt,"published warning offset missing"))?;
    Ok(Some(GlobalLogicalCandidate {
        id,
        thread_id: thread,
        decision_seq: seq,
        event_offset: offset,
        kind: "warn".into(),
        source: GlobalLogicalSource::PublishedWarning,
    }))
}

/// Visit at most `max_candidates` indexed physical rows across both logical
/// sources. A merge probe counts even when that row loses ordering and is
/// revisited next call. Callers advance only after fully examining a returned
/// candidate; an empty final slice is a valid bounded completion.
pub fn scan_global_logical_candidates(
    db: &Connection,
    instance: &str,
    kinds: GlobalLogicalKinds,
    position: Option<GlobalLogicalPosition>,
    max_candidates: u16,
) -> Result<GlobalLogicalSlice, ApiError> {
    let merging = kinds != GlobalLogicalKinds::Ordinary;
    if max_candidates == 0 || max_candidates > 100 || (merging && max_candidates < 2) {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid global logical scan bound",
        ));
    }
    let high_water: i64 = db
        .query_row(
            "SELECT decision_seq FROM host_instances WHERE id=?1",
            [instance],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?
        .ok_or_else(|| api_error(ErrorCode::NotFound, "host instance missing"))?;
    let mut position = position.unwrap_or(GlobalLogicalPosition {
        after_decision_seq: 0,
        after_event_offset: -1,
        high_water_decision_seq: high_water,
    });
    if position.after_decision_seq < 0
        || position.after_decision_seq > position.high_water_decision_seq
        || position.after_event_offset < -1
        || position.high_water_decision_seq > high_water
    {
        return Err(api_error(
            ErrorCode::InvalidCursor,
            "invalid global logical position",
        ));
    }
    let mut visited = 0u16;
    let mut candidates = Vec::new();
    let mut exhausted = false;
    while visited + if merging { 2 } else { 1 } <= max_candidates {
        let physical = next_physical_logical(db, instance, kinds, &position)?;
        let manifest = if merging {
            next_manifest_warning_logical(db, instance, &position)?
        } else {
            None
        };
        visited = visited
            .checked_add(u16::from(physical.is_some()) + u16::from(manifest.is_some()))
            .ok_or_else(|| api_error(ErrorCode::SequenceExhausted, "candidate count exhausted"))?;
        let chosen = match (physical, manifest) {
            (None, None) => {
                exhausted = true;
                break;
            }
            (Some(p), None) => p,
            (None, Some(m)) => m,
            (Some(p), Some(m)) => {
                match (p.decision_seq, p.event_offset).cmp(&(m.decision_seq, m.event_offset)) {
                    std::cmp::Ordering::Less => p,
                    std::cmp::Ordering::Greater => m,
                    std::cmp::Ordering::Equal => {
                        if p.id != m.id || p.kind != "warn" {
                            return Err(api_error(
                                ErrorCode::StoreCorrupt,
                                "logical publication key collision",
                            ));
                        }
                        m
                    }
                }
            }
        };
        position.after_decision_seq = chosen.decision_seq;
        position.after_event_offset = chosen.event_offset;
        candidates.push(chosen);
    }
    Ok(GlobalLogicalSlice {
        candidates,
        position,
        visited,
        has_more: !exhausted,
    })
}

/// The inbox cursor's publication checkpoint is scoped to a seat and frozen
/// thread prefix. The baseline remains fixed while a page advances; only the
/// validation positions move through later logical publications.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxCapture {
    pub seat_revision: u64,
    pub lifecycle_revision: u64,
    pub baseline_decision_seq: u64,
    pub manifest_after_seq: u64,
    pub warning_after_seq: u64,
    pub warning_after_offset: i64,
}
impl InboxCapture {
    pub fn to_cursor_state(&self) -> InboxCursorState {
        InboxCursorState {
            seat_revision: self.seat_revision,
            lifecycle_revision: self.lifecycle_revision,
            baseline_decision_seq: self.baseline_decision_seq,
            manifest_after_seq: self.manifest_after_seq,
            warning_after_seq: self.warning_after_seq,
            warning_after_offset: self.warning_after_offset,
        }
    }
    pub fn from_cursor_state(state: InboxCursorState) -> Self {
        Self {
            seat_revision: state.seat_revision,
            lifecycle_revision: state.lifecycle_revision,
            baseline_decision_seq: state.baseline_decision_seq,
            manifest_after_seq: state.manifest_after_seq,
            warning_after_seq: state.warning_after_seq,
            warning_after_offset: state.warning_after_offset,
        }
    }
    /// A newly examined thread can contain a publication previously ignored
    /// while it was beyond the checked prefix. Revalidate from the original
    /// capture on the next page, with resumable bounded progress.
    pub fn reset_for_next_thread_prefix(&mut self) {
        self.manifest_after_seq = self.baseline_decision_seq;
        self.warning_after_seq = self.baseline_decision_seq;
        self.warning_after_offset = i64::MAX;
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InboxValidation {
    Current { token: InboxCapture, visited: u16 },
    More { token: InboxCapture, visited: u16 },
    Changed,
}

fn inbox_revision(db: &Connection, instance: &str, seat: &str) -> Result<i64, ApiError> {
    db.query_row("SELECT COALESCE((SELECT revision FROM filter_revisions WHERE instance_id=?1 AND scope_kind='inbox' AND scope_key=?2),0)",params![instance,seat],|r|r.get(0)).map_err(store_error)
}
fn inbox_host_revisions(db: &Connection, instance: &str) -> Result<(i64, i64), ApiError> {
    db.query_row(
        "SELECT lifecycle_revision,decision_seq FROM host_instances WHERE id=?1",
        [instance],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
    .map_err(store_error)?
    .ok_or_else(|| api_error(ErrorCode::NotFound, "host instance missing"))
}
fn checked_u64(value: i64) -> Result<u64, ApiError> {
    u64::try_from(value)
        .map_err(|_| api_error(ErrorCode::StoreCorrupt, "negative durable revision"))
}
fn check_inbox_budget(budget: &CallBudget, clock: &dyn Clock) -> Result<(), ApiError> {
    if budget.cancellation.is_cancelled() {
        Err(api_error(
            ErrorCode::Cancelled,
            "inbox validation cancelled",
        ))
    } else if clock.monotonic_now() >= budget.deadline {
        Err(api_error(
            ErrorCode::ReadBudgetExhausted,
            "inbox validation budget exhausted",
        ))
    } else {
        Ok(())
    }
}
/// Capture under the same short read/registration transaction as the first
/// inbox page or check-in offer. A hidden preparation has no manifest and does
/// not change this token.
pub fn capture_inbox_token(
    db: &Connection,
    instance: &str,
    seat: &str,
) -> Result<InboxCapture, ApiError> {
    let owned: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM seats WHERE id=?1 AND instance_id=?2)",
            params![seat, instance],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if !owned {
        return Err(api_error(ErrorCode::NotFound, "seat missing from instance"));
    }
    let (lifecycle, decision) = inbox_host_revisions(db, instance)?;
    let baseline = checked_u64(decision)?;
    Ok(InboxCapture {
        seat_revision: checked_u64(inbox_revision(db, instance, seat)?)?,
        lifecycle_revision: checked_u64(lifecycle)?,
        baseline_decision_seq: baseline,
        manifest_after_seq: baseline,
        warning_after_seq: baseline,
        warning_after_offset: i64::MAX,
    })
}

/// Validate all published receipt and warning inclusion changes since a page
/// captured its thread range. A returned More is never evidence of unchanged
/// inclusion; callers emit a same-thread-position work continuation. The
/// caller holds a single read transaction and finite SQLite progress guard.
// Allowed: token validation needs the query position, token, bound, budget and clock together.
#[allow(clippy::too_many_arguments)]
pub fn validate_inbox_token(
    db: &Connection,
    instance: &str,
    seat: &str,
    after_thread_ordinal: u64,
    mut token: InboxCapture,
    max_candidates: u16,
    budget: &CallBudget,
    clock: &dyn Clock,
) -> Result<InboxValidation, ApiError> {
    if !(4..=100).contains(&max_candidates) {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid inbox validation bound",
        ));
    }
    check_inbox_budget(budget, clock)?;
    let owned: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM seats WHERE id=?1 AND instance_id=?2)",
            params![seat, instance],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if !owned {
        return Err(api_error(ErrorCode::NotFound, "seat missing from instance"));
    }
    let (lifecycle, current) = inbox_host_revisions(db, instance)?;
    let current = checked_u64(current)?;
    if token.seat_revision != checked_u64(inbox_revision(db, instance, seat)?)?
        || token.lifecycle_revision != checked_u64(lifecycle)?
    {
        return Ok(InboxValidation::Changed);
    }
    if token.baseline_decision_seq > current
        || token.manifest_after_seq < token.baseline_decision_seq
        || token.manifest_after_seq > current
        || token.warning_after_seq < token.baseline_decision_seq
        || token.warning_after_seq > current
        || token.warning_after_offset < -1
        || after_thread_ordinal > i64::MAX as u64
    {
        return Err(api_error(
            ErrorCode::InvalidCursor,
            "invalid inbox publication position",
        ));
    }
    let mut visited = 0u16;
    let manifest_quota = max_candidates / 2;
    while token.manifest_after_seq < current && visited < manifest_quota {
        check_inbox_budget(budget, clock)?;
        let row:Option<(i64,String,i64)>=db.query_row(
            "SELECT sm.decision_seq,sm.preparation_id,t.ordinal FROM send_manifests sm JOIN threads t ON t.id=sm.thread_id WHERE sm.instance_id=?1 AND sm.decision_seq>?2 AND sm.decision_seq<=?3 ORDER BY sm.decision_seq LIMIT 1",
            params![instance,token.manifest_after_seq as i64,current as i64],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
        ).optional().map_err(store_error)?;
        let Some((seq, prep, thread_ordinal)) = row else {
            token.manifest_after_seq = current;
            break;
        };
        token.manifest_after_seq = checked_u64(seq)?;
        visited += 1;
        if thread_ordinal <= after_thread_ordinal as i64 {
            let addressed:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM prepared_recipients WHERE preparation_id=?1 AND seat_id=?2)",params![prep,seat],|r|r.get(0)).map_err(store_error)?;
            if addressed {
                return Ok(InboxValidation::Changed);
            }
        }
    }
    let warning_budget = max_candidates - visited;
    if !(token.warning_after_seq == current && token.warning_after_offset == i64::MAX)
        && warning_budget >= 2
    {
        check_inbox_budget(budget, clock)?;
        let position = GlobalLogicalPosition {
            after_decision_seq: token.warning_after_seq as i64,
            after_event_offset: token.warning_after_offset,
            high_water_decision_seq: current as i64,
        };
        let slice = scan_global_logical_candidates(
            db,
            instance,
            GlobalLogicalKinds::Warnings,
            Some(position),
            warning_budget,
        )?;
        visited = visited
            .checked_add(slice.visited)
            .ok_or_else(|| api_error(ErrorCode::SequenceExhausted, "inbox visited overflow"))?;
        for warning in &slice.candidates {
            check_inbox_budget(budget, clock)?;
            let thread_ordinal: i64 = db
                .query_row(
                    "SELECT ordinal FROM threads WHERE id=?1",
                    [&warning.thread_id],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            if thread_ordinal <= after_thread_ordinal as i64
                && is_warning_recipient(db, &warning.id, seat)?
            {
                return Ok(InboxValidation::Changed);
            }
        }
        if slice.has_more {
            token.warning_after_seq = checked_u64(slice.position.after_decision_seq)?;
            token.warning_after_offset = slice.position.after_event_offset;
        } else {
            token.warning_after_seq = current;
            token.warning_after_offset = i64::MAX;
        }
    }
    if token.manifest_after_seq == current
        && token.warning_after_seq == current
        && token.warning_after_offset == i64::MAX
    {
        Ok(InboxValidation::Current { token, visited })
    } else {
        Ok(InboxValidation::More { token, visited })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveWarningSlice {
    pub warnings: Vec<EffectiveWarning>,
    pub position: TimelinePosition,
    /// Timeline slots examined before filtering, including ordinary/info rows.
    pub visited: u16,
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarningRecipientPosition {
    pub warning_id: String,
    pub interval_after: i64,
    pub interval_high_water: i64,
    pub affected_done: bool,
    pub ledger_after: i64,
    pub ledger_high_water: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarningRecipientSlice {
    pub seats: Vec<String>,
    pub position: WarningRecipientPosition,
    /// Physical interval candidates plus the direct affected-seat candidate.
    pub visited: u16,
    pub has_more: bool,
}

/// Enumerate the decision-frozen interval set and direct affected recipient
/// under one physical candidate bound. Attribution projection does not change
/// this page's historical answer. Retirement warnings have no fanout recipe.
pub fn scan_effective_warning_recipients(
    db: &Connection,
    warning_id: &str,
    position: Option<WarningRecipientPosition>,
    max_candidates: u16,
) -> Result<WarningRecipientSlice, ApiError> {
    if max_candidates == 0 || max_candidates > 100 {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid warning recipient bound",
        ));
    }
    let warning = effective_warning_by_id(db, warning_id)?
        .ok_or_else(|| api_error(ErrorCode::NotFound, "warning missing"))?;
    let service_preparation: Option<String> = db
        .query_row(
            "SELECT preparation_id FROM service_notification_publications WHERE message_id=?1",
            [warning_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    if let Some(preparation) = service_preparation {
        let high: i64 = db.query_row(
            "SELECT coalesce(max(ordinal),0) FROM service_notification_recipients WHERE preparation_id=?1",
            [preparation.as_str()], |r| r.get(0)).map_err(store_error)?;
        let mut position = position.unwrap_or(WarningRecipientPosition {
            warning_id: warning_id.into(),
            interval_after: 0,
            interval_high_water: 0,
            affected_done: true,
            ledger_after: 0,
            ledger_high_water: high,
        });
        if position.warning_id != warning_id
            || position.interval_after != 0
            || position.interval_high_water != 0
            || !position.affected_done
            || position.ledger_after < 0
            || position.ledger_after > position.ledger_high_water
            || position.ledger_high_water != high
        {
            return Err(api_error(
                ErrorCode::InvalidCursor,
                "invalid service warning recipient position",
            ));
        }
        let mut seats = Vec::new();
        while seats.len() < usize::from(max_candidates) {
            let row: Option<(i64,String)> = db.query_row(
                "SELECT ordinal,seat_id FROM service_notification_recipients WHERE preparation_id=?1 AND ordinal>?2 AND ordinal<=?3 ORDER BY ordinal LIMIT 1",
                params![preparation,position.ledger_after,high], |r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(store_error)?;
            let Some((ordinal, seat)) = row else { break };
            position.ledger_after = ordinal;
            seats.push(seat);
        }
        let has_more: bool=db.query_row(
            "SELECT EXISTS(SELECT 1 FROM service_notification_recipients WHERE preparation_id=?1 AND ordinal>?2 AND ordinal<=?3)",
            params![preparation,position.ledger_after,high],|r|r.get(0)).map_err(store_error)?;
        return Ok(WarningRecipientSlice {
            visited: seats.len() as u16,
            seats,
            position,
            has_more,
        });
    }
    let recipe: Option<(i64,Option<String>)> = db.query_row(
        "SELECT sm.interval_high_water,w.affected_seat_id FROM prepared_unavailable_warnings w JOIN send_manifests sm ON sm.preparation_id=w.preparation_id WHERE w.warning_id=?1",
        [warning_id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(store_error)?
        .or(db.query_row("SELECT interval_high_water,affected_seat_id FROM warning_jobs WHERE warning_id=?1",[warning_id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(store_error)?);
    if recipe.is_none() {
        // A physical system/retirement warning may have only the immutable
        // projected recipient ledger, with no fanout recipe. Traverse its
        // warning-leading ordinal index instead of claiming an empty set.
        let ledger_high_water: i64 = db
            .query_row(
                "SELECT COALESCE(MAX(ordinal),0) FROM warning_recipients WHERE warning_id=?1",
                [warning_id],
                |r| r.get(0),
            )
            .map_err(store_error)?;
        let mut position = position.unwrap_or(WarningRecipientPosition {
            warning_id: warning_id.to_owned(),
            interval_after: 0,
            interval_high_water: 0,
            affected_done: true,
            ledger_after: 0,
            ledger_high_water,
        });
        if position.warning_id != warning_id
            || position.interval_after != 0
            || position.interval_high_water != 0
            || !position.affected_done
            || position.ledger_high_water > ledger_high_water
            || position.ledger_after < 0
            || position.ledger_after > ledger_high_water
        {
            return Err(api_error(
                ErrorCode::InvalidRequest,
                "invalid warning ledger position",
            ));
        }
        let mut seats = Vec::new();
        while seats.len() < usize::from(max_candidates) {
            let row:Option<(i64,String)>=db.query_row(
                "SELECT ordinal,seat_id FROM warning_recipients WHERE warning_id=?1 AND ordinal>?2 AND ordinal<=?3 ORDER BY ordinal LIMIT 1",
                params![warning_id,position.ledger_after,position.ledger_high_water],
                |r|Ok((r.get(0)?,r.get(1)?)),
            ).optional().map_err(store_error)?;
            let Some((ordinal, seat)) = row else { break };
            position.ledger_after = ordinal;
            seats.push(seat);
        }
        let has_more:bool=db.query_row(
            "SELECT EXISTS(SELECT 1 FROM warning_recipients WHERE warning_id=?1 AND ordinal>?2 AND ordinal<=?3)",
            params![warning_id,position.ledger_after,position.ledger_high_water],|r|r.get(0),
        ).map_err(store_error)?;
        return Ok(WarningRecipientSlice {
            visited: seats.len() as u16,
            seats,
            position,
            has_more,
        });
    }
    let (high_water, affected) = recipe.expect("checked recipe");
    let mut position = position.unwrap_or(WarningRecipientPosition {
        warning_id: warning_id.to_owned(),
        interval_after: 0,
        interval_high_water: high_water,
        affected_done: affected.is_none(),
        ledger_after: 0,
        ledger_high_water: 0,
    });
    if position.warning_id != warning_id
        || position.interval_after < 0
        || position.interval_after > position.interval_high_water
        || position.interval_high_water != high_water
        || position.ledger_after != 0
        || position.ledger_high_water != 0
    {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid warning recipient position",
        ));
    }
    let mut seats = Vec::new();
    let mut visited = 0;
    while visited < max_candidates {
        type IntervalColumns = Option<(i64, String, i64, Option<i64>, Option<i64>)>;
        let interval: IntervalColumns = db.query_row(
            "SELECT mi.ordinal,mi.seat_id,mi.joined_seq,mi.left_seq,s.retired_seq FROM membership_intervals mi JOIN seats s ON s.id=mi.seat_id WHERE mi.thread_id=?1 AND mi.ordinal>?2 AND mi.ordinal<=?3 ORDER BY mi.ordinal LIMIT 1",
            params![warning.thread_id,position.interval_after,position.interval_high_water],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(store_error)?;
        if let Some((ordinal, seat, joined, left, retired)) = interval {
            position.interval_after = ordinal;
            visited += 1;
            if joined <= warning.event_seq
                && left.is_none_or(|left| left > warning.event_seq)
                && retired.is_none_or(|retired| retired > warning.event_seq)
            {
                seats.push(seat);
            }
            continue;
        }
        if !position.affected_done {
            position.affected_done = true;
            visited += 1;
            if let Some(seat) = affected.as_deref() {
                let already_joined: bool=db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM membership_intervals mi JOIN seats s ON s.id=mi.seat_id WHERE mi.thread_id=?1 AND mi.seat_id=?2 AND mi.ordinal<=?3 AND mi.joined_seq<=?4 AND (mi.left_seq IS NULL OR mi.left_seq>?4) AND (s.retired_seq IS NULL OR s.retired_seq>?4))",
                    params![warning.thread_id,seat,position.interval_high_water,warning.event_seq],|r|r.get(0)).map_err(store_error)?;
                if !already_joined && is_warning_recipient(db, warning_id, seat)? {
                    seats.push(seat.to_owned());
                }
            }
            continue;
        }
        break;
    }
    let has_interval: bool=db.query_row("SELECT EXISTS(SELECT 1 FROM membership_intervals WHERE thread_id=?1 AND ordinal>?2 AND ordinal<=?3)",params![warning.thread_id,position.interval_after,position.interval_high_water],|r|r.get(0)).map_err(store_error)?;
    let has_more = has_interval || !position.affected_done;
    Ok(WarningRecipientSlice {
        seats,
        position,
        visited,
        has_more,
    })
}

/// One finite read slice for exact historical warning discovery/counts. Keep
/// scanning while `has_more`; a partial count is not an exact zero or total.
/// Only the thread's warning positions are visited (see
/// `scan_effective_warning_timeline`), never its ordinary messages.
pub fn scan_effective_warnings_for_seat(
    db: &Connection,
    thread_id: &str,
    seat_id: &str,
    position: Option<TimelinePosition>,
    max_candidates: u16,
) -> Result<EffectiveWarningSlice, ApiError> {
    let timeline = scan_effective_warning_timeline(db, thread_id, position, max_candidates)?;
    let mut warnings = Vec::new();
    for entry in timeline.entries {
        let warning = match entry {
            EffectiveTimelineEntry::Physical { id, kind, .. } if kind == "warn" => {
                effective_warning_by_id(db, &id)?
            }
            EffectiveTimelineEntry::PublishedWarning(warning) => Some(warning),
            _ => None,
        };
        if let Some(warning) = warning
            && is_warning_recipient(db, &warning.id, seat_id)?
        {
            warnings.push(warning);
        }
    }
    Ok(EffectiveWarningSlice {
        warnings,
        position: timeline.position,
        visited: timeline.visited,
        has_more: timeline.has_more,
    })
}

/// Timeline sequence positions are immutable and gap-free at publication. A
/// point lookup first sees a materialized row; otherwise the predecessor
/// manifest and its exact warning offset supply the same logical identity.
/// Projection after this page cannot move or duplicate that identity.
pub fn scan_effective_timeline(
    db: &Connection,
    thread_id: &str,
    position: Option<TimelinePosition>,
    max_candidates: u16,
) -> Result<EffectiveTimelineSlice, ApiError> {
    if max_candidates == 0 || max_candidates > 100 {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid timeline scan bound",
        ));
    }
    let mut position = match position {
        Some(position) => position,
        None => {
            let next: i64 = db
                .query_row(
                    "SELECT next_sequence FROM threads WHERE id=?1",
                    [thread_id],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            TimelinePosition {
                after_sequence: 0,
                high_water_sequence: next.checked_sub(1).ok_or_else(|| {
                    api_error(ErrorCode::StoreCorrupt, "invalid timeline sequence")
                })?,
            }
        }
    };
    if position.after_sequence < 0 || position.after_sequence > position.high_water_sequence {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid timeline position",
        ));
    }
    let mut entries = Vec::new();
    while entries.len() < usize::from(max_candidates)
        && position.after_sequence < position.high_water_sequence
    {
        let sequence = position.after_sequence.checked_add(1).ok_or_else(|| {
            api_error(ErrorCode::SequenceExhausted, "timeline sequence exhausted")
        })?;
        let entry = timeline_entry(db, thread_id, sequence)?;
        entries.push(entry);
        position.after_sequence = sequence;
    }
    Ok(EffectiveTimelineSlice {
        visited: entries.len() as u16,
        has_more: position.after_sequence < position.high_water_sequence,
        entries,
        position,
    })
}

/// The single effective entry at one published timeline position: the
/// materialized row when present, otherwise the manifest warning that owns
/// the position.
fn timeline_entry(
    db: &Connection,
    thread_id: &str,
    sequence: i64,
) -> Result<EffectiveTimelineEntry, ApiError> {
    let physical: Option<(String, String)> = db
        .query_row(
            "SELECT id,kind FROM messages WHERE thread_id=?1 AND sequence=?2",
            params![thread_id, sequence],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(store_error)?;
    let entry = if let Some((id, kind)) = physical {
        EffectiveTimelineEntry::Physical { id, sequence, kind }
    } else {
        let manifest: Option<(String,i64,i64)> = db.query_row("SELECT preparation_id,base_sequence,warning_count FROM send_manifests WHERE thread_id=?1 AND base_sequence<?2 ORDER BY base_sequence DESC LIMIT 1",params![thread_id,sequence],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(store_error)?;
        let Some((preparation, base, count)) = manifest else {
            return Err(api_error(
                ErrorCode::StoreCorrupt,
                "unbacked timeline position",
            ));
        };
        let offset = sequence
            .checked_sub(base)
            .ok_or_else(|| api_error(ErrorCode::StoreCorrupt, "invalid warning offset"))?;
        if offset <= 0 || offset > count {
            return Err(api_error(
                ErrorCode::StoreCorrupt,
                "unbacked timeline position",
            ));
        }
        let warning_id: String = db.query_row("SELECT warning_id FROM prepared_unavailable_warnings WHERE preparation_id=?1 AND warning_offset=?2",params![preparation,offset],|r|r.get(0)).map_err(store_error)?;
        let warning = effective_warning_by_id(db, &warning_id)?
            .ok_or_else(|| api_error(ErrorCode::StoreCorrupt, "missing published warning"))?;
        if warning.thread_id != thread_id || warning.sequence != sequence {
            return Err(api_error(
                ErrorCode::StoreCorrupt,
                "warning timeline mismatch",
            ));
        }
        EffectiveTimelineEntry::PublishedWarning(warning)
    };
    Ok(entry)
}

/// The next published warning position in `(after, high]`: the lesser of the
/// next materialized warn row (`messages_thread_warning`) and the next
/// manifest-reserved warning position (`send_manifests_thread_warning`), both
/// single indexed seeks that never touch ordinary messages.
fn next_warning_sequence(
    db: &Connection,
    thread_id: &str,
    after: i64,
    high: i64,
) -> Result<Option<i64>, ApiError> {
    let physical: Option<i64> = db
        .query_row(
            "SELECT sequence FROM messages WHERE thread_id=?1 AND kind='warn' AND sequence>?2 AND sequence<=?3 ORDER BY sequence LIMIT 1",
            params![thread_id, after, high],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    let covering: Option<(i64, i64)> = db
        .query_row(
            "SELECT base_sequence,warning_count FROM send_manifests WHERE thread_id=?1 AND warning_count>0 AND base_sequence<=?2 ORDER BY base_sequence DESC LIMIT 1",
            params![thread_id, after],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(store_error)?;
    let within = covering.and_then(|(base, count)| {
        base.checked_add(count)
            .filter(|end| *end > after)
            .and_then(|_| after.checked_add(1))
    });
    let manifest = match within {
        Some(next) => Some(next),
        None => db
            .query_row(
                "SELECT base_sequence FROM send_manifests WHERE thread_id=?1 AND warning_count>0 AND base_sequence>?2 AND base_sequence<?3 ORDER BY base_sequence LIMIT 1",
                params![thread_id, after, high],
                |r| r.get::<_, i64>(0),
            )
            .optional()
            .map_err(store_error)?
            .map(|base| {
                base.checked_add(1).ok_or_else(|| {
                    api_error(ErrorCode::SequenceExhausted, "timeline sequence exhausted")
                })
            })
            .transpose()?,
    }
    .filter(|next| *next <= high);
    Ok(match (physical, manifest) {
        (Some(p), Some(m)) => Some(p.min(m)),
        (p, m) => p.or(m),
    })
}

/// The warning-only projection of `scan_effective_timeline`: the same entries
/// at the same positions, in the same order and with the same position
/// encoding, minus every ordinary (non-warning) entry, which is never read.
/// Cost is proportional to the thread's warnings, not to its messages.
pub fn scan_effective_warning_timeline(
    db: &Connection,
    thread_id: &str,
    position: Option<TimelinePosition>,
    max_candidates: u16,
) -> Result<EffectiveTimelineSlice, ApiError> {
    if max_candidates == 0 || max_candidates > 100 {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid timeline scan bound",
        ));
    }
    let mut position = match position {
        Some(position) => position,
        None => {
            let next: i64 = db
                .query_row(
                    "SELECT next_sequence FROM threads WHERE id=?1",
                    [thread_id],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            TimelinePosition {
                after_sequence: 0,
                high_water_sequence: next.checked_sub(1).ok_or_else(|| {
                    api_error(ErrorCode::StoreCorrupt, "invalid timeline sequence")
                })?,
            }
        }
    };
    if position.after_sequence < 0 || position.after_sequence > position.high_water_sequence {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid timeline position",
        ));
    }
    let mut entries = Vec::new();
    while position.after_sequence < position.high_water_sequence {
        let Some(sequence) = next_warning_sequence(
            db,
            thread_id,
            position.after_sequence,
            position.high_water_sequence,
        )?
        else {
            position.after_sequence = position.high_water_sequence;
            break;
        };
        if entries.len() == usize::from(max_candidates) {
            break;
        }
        entries.push(timeline_entry(db, thread_id, sequence)?);
        position.after_sequence = sequence;
    }
    Ok(EffectiveTimelineSlice {
        visited: entries.len() as u16,
        has_more: position.after_sequence < position.high_water_sequence,
        entries,
        position,
    })
}

/// Full immutable unavailable-episode identity, scoped to one instance/thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnavailableWarningKey {
    pub instance: String,
    pub thread_id: String,
    pub affected_seat_id: String,
    pub unavailability_episode: u64,
}

pub fn canonical_warning_key(key: &UnavailableWarningKey) -> Result<String, ApiError> {
    if [
        key.instance.as_str(),
        key.thread_id.as_str(),
        key.affected_seat_id.as_str(),
    ]
    .iter()
    .any(|value| {
        value.is_empty() || value.len() > 128 || value.bytes().any(|b| !b.is_ascii_graphic())
    }) || key.unavailability_episode == 0
    {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid unavailable warning key",
        ));
    }
    serde_json::to_string(&(
        "herdr-unavailable-warning-v1",
        &key.instance,
        &key.thread_id,
        &key.affected_seat_id,
        key.unavailability_episode,
    ))
    .map_err(|_| api_error(ErrorCode::StoreCorrupt, "cannot encode warning key"))
}

/// Domain-separated SHA-256 truncated to an RFC 9562 version-8 UUID.
fn warning_uuid(key: &UnavailableWarningKey) -> Result<uuid::Uuid, ApiError> {
    let full_key = canonical_warning_key(key)?;
    let mut hash = Sha256::new();
    hash.update(b"herdr-warning-id-v1\0");
    hash.update(full_key.as_bytes());
    let digest = hash.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(uuid::Uuid::from_bytes(bytes))
}

/// Stable compact ID for a new unavailable-recipient warning.
pub fn canonical_warning_id(key: &UnavailableWarningKey) -> Result<String, ApiError> {
    Ok(format!("w{}", warning_uuid(key)?))
}

fn legacy_warning_id(key: &UnavailableWarningKey) -> Result<String, ApiError> {
    Ok(format!("warning-{}", warning_uuid(key)?))
}

/// Direct events and published manifests share one canonical lookup. Hidden
/// preparation rows are excluded by the manifest join.
pub fn effective_warning_by_key(
    db: &Connection,
    key: &UnavailableWarningKey,
) -> Result<Option<EffectiveWarning>, ApiError> {
    let encoded = canonical_warning_key(key)?;
    let expected_id = canonical_warning_id(key)?;
    let legacy_id = legacy_warning_id(key)?;
    let published: Option<String> = db.query_row(
        "SELECT w.warning_id FROM prepared_unavailable_warnings w JOIN send_manifests sm ON sm.preparation_id=w.preparation_id WHERE w.warning_key=?1 LIMIT 1",
        [&encoded], |r| r.get(0),
    ).optional().map_err(store_error)?;
    let direct: Option<String> = db
        .query_row(
            "SELECT id FROM messages WHERE event_key=?1 AND kind='warn'",
            [&encoded],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?;
    let recorded = published.as_deref().or(direct.as_deref());
    if recorded.is_some_and(|id| id != expected_id.as_str() && id != legacy_id.as_str())
        || (published.is_some() && direct.is_some() && published.as_deref() != direct.as_deref())
    {
        return Err(api_error(
            ErrorCode::StoreCorrupt,
            "warning key and deterministic ID disagree",
        ));
    }
    recorded.map_or(Ok(None), |id| effective_warning_by_id(db, id))
}

pub fn effective_warning_by_id(
    db: &Connection,
    id: &str,
) -> Result<Option<EffectiveWarning>, ApiError> {
    let published: Option<EffectiveWarning> = db.query_row(
        "SELECT w.warning_id, sm.thread_id, sm.base_sequence+w.warning_offset, sm.decision_seq, \
         w.event_json, sm.message_id, w.affected_seat_id \
         FROM prepared_unavailable_warnings w JOIN send_manifests sm ON sm.preparation_id=w.preparation_id \
         WHERE w.warning_id=?1",
        [id], |r| Ok(EffectiveWarning {
            id: r.get(0)?, thread_id: r.get(1)?, sequence: r.get(2)?, event_seq: r.get(3)?,
            event_json: r.get(4)?, source_message_id: r.get(5)?, affected_seat_id: r.get(6)?,
        }),
    ).optional().map_err(store_error)?;
    if published.is_some() {
        return Ok(published);
    }
    db.query_row(
        "SELECT m.id, m.thread_id, m.sequence, m.decision_seq, m.event_json, m.source_message_id, w.affected_seat_id \
         FROM messages m LEFT JOIN warning_jobs w ON w.warning_id=m.id WHERE m.id=?1 AND m.kind='warn'",
        [id], |r| Ok(EffectiveWarning {
            id: r.get(0)?, thread_id: r.get(1)?, sequence: r.get(2)?, event_seq: r.get(3)?,
            event_json: r.get(4)?, source_message_id: r.get(5)?, affected_seat_id: r.get(6)?,
        }),
    ).optional().map_err(store_error)
}

pub fn is_warning_recipient(db: &Connection, id: &str, seat_id: &str) -> Result<bool, ApiError> {
    let Some(warning) = effective_warning_by_id(db, id)? else {
        return Ok(false);
    };
    let service: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM service_notification_publications p JOIN service_notification_recipients r ON r.preparation_id=p.preparation_id WHERE p.message_id=?1 AND r.seat_id=?2)",
        params![id,seat_id], |r|r.get(0)).map_err(store_error)?;
    if service {
        return Ok(true);
    }
    let projected: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM warning_recipients WHERE warning_id=?1 AND seat_id=?2)",
            params![id, seat_id],
            |r| r.get(0),
        )
        .map_err(store_error)?;
    if projected || warning.affected_seat_id.as_deref() == Some(seat_id) {
        return Ok(true);
    }
    let high_water: Option<i64> = db
        .query_row(
            "SELECT sm.interval_high_water FROM prepared_unavailable_warnings w \
         JOIN send_manifests sm ON sm.preparation_id=w.preparation_id WHERE w.warning_id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(store_error)?
        .or(db
            .query_row(
                "SELECT interval_high_water FROM warning_jobs WHERE warning_id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()
            .map_err(store_error)?);
    let Some(high_water) = high_water else {
        return Ok(false);
    };
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM membership_intervals mi JOIN seats s ON s.id=mi.seat_id \
         WHERE mi.thread_id=?1 AND mi.seat_id=?2 AND mi.ordinal<=?3 AND mi.joined_seq<=?4 \
         AND (mi.left_seq IS NULL OR mi.left_seq>?4) AND (s.retired_seq IS NULL OR s.retired_seq>?4))",
        params![warning.thread_id, seat_id, high_water, warning.event_seq], |r| r.get(0),
    ).map_err(store_error)
}

/// Historical recipients remain discoverable after source settlement, while
/// warning-only wake uses the exact source condition frozen by the event.
pub fn warning_condition_actionable(
    db: &Connection,
    warning: &EffectiveWarning,
) -> Result<bool, ApiError> {
    let programmatic: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM messages m JOIN service_notification_publications p ON p.message_id=m.id WHERE m.id=?1 AND m.kind='warn' AND m.author_kind='programmatic')",
        [&warning.id],|r|r.get(0)).map_err(store_error)?;
    if programmatic {
        return Ok(true);
    }
    let unavailable: Option<(String,i64)> = db.query_row(
        "SELECT w.affected_seat_id,w.unavailability_episode FROM prepared_unavailable_warnings w JOIN send_manifests sm ON sm.preparation_id=w.preparation_id WHERE w.warning_id=?1",
        [&warning.id],|r|Ok((r.get(0)?,r.get(1)?)),
    ).optional().map_err(store_error)?;
    if let Some((seat, episode)) = unavailable {
        return db.query_row(
            "SELECT EXISTS(SELECT 1 FROM seats WHERE id=?1 AND state!='retired' AND unavailability_open=1 AND unavailability_episode=?2)",
            params![seat,episode],|r|r.get(0),
        ).map_err(store_error);
    }
    let source:Option<(String,String,Option<String>)>=db.query_row(
        "SELECT condition_kind,condition_id,affected_seat_id FROM warning_jobs WHERE warning_id=?1",
        [&warning.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    ).optional().map_err(store_error)?;
    let Some((kind, id, affected)) = source else {
        return Ok(false);
    };
    match kind.as_str() {
        "invitation" => db.query_row(
            "SELECT EXISTS(SELECT 1 FROM invitations i JOIN seats s ON s.id=i.seat_id WHERE i.id=?1 AND i.state='pending' AND s.state!='retired' AND NOT EXISTS (SELECT 1 FROM invitation_cancellations c WHERE c.invitation_id=i.id))",
            [id],|r|r.get(0),
        ).map_err(store_error),
        "receipt" => {
            let Some(message)=warning.source_message_id.as_deref() else { return Err(api_error(ErrorCode::StoreCorrupt,"receipt warning lacks source message")) };
            let Some(seat)=affected.as_deref() else { return Err(api_error(ErrorCode::StoreCorrupt,"receipt warning lacks affected seat")) };
            Ok(effective_receipt(db,message,seat)?.is_some_and(|receipt|receipt.state==EffectiveReceiptState::Pending))
        }
        "unavailable" => Err(api_error(ErrorCode::StoreCorrupt,"unavailable warning lacks published episode")),
        _ => Err(api_error(ErrorCode::StoreCorrupt,format!("unknown warning condition {kind}"))),
    }
}

pub fn effective_receipt(
    db: &Connection,
    message_id: &str,
    seat_id: &str,
) -> Result<Option<EffectiveReceipt>, ApiError> {
    type LogicalColumns = Option<(
        String,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<i64>,
        Option<String>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
    )>;
    let logical: LogicalColumns = db.query_row(
        "SELECT sm.thread_id, sm.base_sequence, pr.ordinal, sm.decision_seq, sm.decision_at, pr.frozen_duration_ms, pr.eligible_at_snapshot, rs.state, rs.warning_message_id, rs.ack_actor_seat_id, rs.ack_generation, rs.ack_observation, rs.acked_at, s.retired_at, s.retired_seq \
         FROM send_manifests sm JOIN prepared_recipients pr ON pr.preparation_id=sm.preparation_id \
         JOIN seats s ON s.id=pr.seat_id \
         LEFT JOIN receipt_state rs ON rs.message_id=sm.message_id AND rs.seat_id=pr.seat_id \
         WHERE sm.message_id=?1 AND pr.seat_id=?2",
        params![message_id, seat_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?, r.get(9)?, r.get(10)?, r.get(11)?, r.get(12)?, r.get(13)?, r.get(14)?)),
    ).optional().map_err(store_error)?;
    if let Some((
        thread_id,
        sequence,
        ordinal,
        send_seq,
        send_at,
        duration,
        eligible,
        state,
        warning_message_id,
        ack_actor_seat_id,
        ack_generation,
        ack_observation,
        acked_at,
        retired_at,
        retired_seq,
    )) = logical
    {
        if retired_at.is_some() && retired_seq.is_none() {
            return Err(api_error(
                ErrorCode::StoreCorrupt,
                "retired seat lacks decision cutover",
            ));
        }
        let start =
            if eligible == 1 {
                Some(send_at)
            } else {
                db.query_row(
                "SELECT decision_at FROM seat_availability WHERE seat_id=?1 AND decision_seq>?2 \
                 AND (?3 IS NULL OR decision_seq<?3) ORDER BY decision_seq LIMIT 1",
                params![seat_id, send_seq, retired_seq], |r| r.get(0),
            ).optional().map_err(store_error)?
            };
        let deadline = start
            .map(|at| checked_deadline(UtcMillis(at), duration).map(|d| d.0))
            .transpose()?;
        return Ok(Some(EffectiveReceipt {
            source: ReceiptSource::Manifest,
            message_id: message_id.to_owned(),
            thread_id,
            seat_id: seat_id.to_owned(),
            sequence,
            source_ordinal: ordinal,
            decision_seq: Some(send_seq),
            decision_at: send_at,
            frozen_duration_ms: duration,
            state: receipt_state(state.as_deref(), retired_at.is_some())?,
            available_at: start,
            deadline_at: deadline,
            warning_message_id,
            ack_actor_seat_id,
            ack_generation,
            ack_observation,
            acked_at,
            retired_at,
        }));
    }
    type PhysicalColumns = Option<(
        String,
        i64,
        i64,
        Option<i64>,
        i64,
        i64,
        String,
        Option<i64>,
        Option<i64>,
        Option<String>,
        Option<String>,
        Option<i64>,
        Option<String>,
        Option<i64>,
        Option<i64>,
    )>;
    let physical: PhysicalColumns = db
        .query_row(
            "SELECT r.thread_id, m.sequence, r.ordinal, m.decision_seq, m.decision_at, r.frozen_duration_ms, r.state, r.available_at, r.deadline_at, r.warning_message_id, r.ack_actor_seat_id, r.ack_generation, r.ack_observation, r.acked_at, s.retired_at \
         FROM receipts r JOIN seats s ON s.id=r.seat_id JOIN messages m ON m.id=r.message_id WHERE r.message_id=?1 AND r.seat_id=?2",
            params![message_id, seat_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?, r.get(9)?, r.get(10)?, r.get(11)?, r.get(12)?, r.get(13)?, r.get(14)?)),
        )
        .optional()
        .map_err(store_error)?;
    physical
        .map(
            |(
                thread_id,
                sequence,
                ordinal,
                decision_seq,
                decision_at,
                duration,
                state,
                available_at,
                deadline_at,
                warning_message_id,
                ack_actor_seat_id,
                ack_generation,
                ack_observation,
                acked_at,
                retired_at,
            )| {
                Ok(EffectiveReceipt {
                    source: ReceiptSource::Physical,
                    message_id: message_id.to_owned(),
                    thread_id,
                    seat_id: seat_id.to_owned(),
                    sequence,
                    source_ordinal: ordinal,
                    decision_seq,
                    decision_at,
                    frozen_duration_ms: duration,
                    state: receipt_state(Some(&state), retired_at.is_some())?,
                    available_at,
                    deadline_at,
                    warning_message_id,
                    ack_actor_seat_id,
                    ack_generation,
                    ack_observation,
                    acked_at,
                    retired_at,
                })
            },
        )
        .transpose()
}

/// The physical row a receipt's `soft_poked_at` lives on (spec §10): a
/// manifest receipt's `receipt_state` row, or a legacy `receipts` row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoftPoked {
    /// The effective receipt has no physical row of that source yet (a staged
    /// manifest receipt that is not materialized): it cannot carry the mark.
    NoRow,
    Unpoked,
    Poked(i64),
}

/// `soft_poked_at` through the effective receipt projection (spec §10; r3):
/// `source` is the effective receipt's own `source`, so the read always lands
/// on the row `poke::mark_soft_poked` writes.
pub fn receipt_soft_poked_at(
    db: &Connection,
    message_id: &str,
    seat_id: &str,
    source: ReceiptSource,
) -> Result<SoftPoked, ApiError> {
    let sql = match source {
        ReceiptSource::Manifest => {
            "SELECT soft_poked_at FROM receipt_state WHERE message_id=?1 AND seat_id=?2"
        }
        ReceiptSource::Physical => {
            "SELECT soft_poked_at FROM receipts WHERE message_id=?1 AND seat_id=?2"
        }
    };
    let row: Option<Option<i64>> = db
        .query_row(sql, params![message_id, seat_id], |r| r.get(0))
        .optional()
        .map_err(store_error)?;
    Ok(match row {
        None => SoftPoked::NoRow,
        Some(None) => SoftPoked::Unpoked,
        Some(Some(at)) => SoftPoked::Poked(at),
    })
}

fn receipt_state(state: Option<&str>, retired: bool) -> Result<EffectiveReceiptState, ApiError> {
    match state.unwrap_or("pending") {
        "acked" => Ok(EffectiveReceiptState::Acknowledged),
        "recipient_retired" => Ok(EffectiveReceiptState::RecipientRetired),
        "pending" if retired => Ok(EffectiveReceiptState::RecipientRetired),
        "pending" => Ok(EffectiveReceiptState::Pending),
        _ => Err(api_error(ErrorCode::StoreCorrupt, "unknown receipt state")),
    }
}

#[cfg(test)]
#[path = "../../tests/store/effective.rs"]
mod tests;
