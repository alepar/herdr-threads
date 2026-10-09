//! The pre-D2 canonical seat-attention scan, kept as the test oracle for
//! `store::attention::wake_seat_attention` (ht-p03.12.5). It walks a seat's
//! complete history source by source; production wake discovery no longer
//! calls it.

use crate::ports::{LogicalAttentionFrontier, LogicalPublicationKey};
use crate::protocol::results::{ApiError, ErrorCode};
use crate::store::{
    connection::{api_error, store_error},
    effective::{
        EffectiveReceiptState, EffectiveSeatAttention, ReceiptScanPosition, ReceiptScanScope,
        effective_warning_by_id, is_warning_recipient, scan_effective_receipts,
        warning_condition_actionable,
    },
};
use rusqlite::{Connection, OptionalExtension, params};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatAttentionPosition {
    pub seat_id: String,
    pub decision_seq: i64,
    pub invitation_after_seq: i64,
    pub invitation_after_ordinal: i64,
    pub invitations_done: bool,
    pub has_pending_invitation: bool,
    pub invitation_frontier: Option<(i64, i64)>,
    pub receipts: Option<ReceiptScanPosition>,
    pub receipts_done: bool,
    pub has_pending_receipt: bool,
    pub receipt_frontier_seq: Option<i64>,
    pub physical_warning_after: i64,
    pub physical_warning_high_water: i64,
    pub manifest_warning_after: i64,
    pub manifest_warning_high_water: i64,
    pub next_manifest_warning: bool,
    pub latest_warning_seq: Option<i64>,
    pub latest_warning_offset: Option<i64>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveSeatAttentionSlice {
    /// Present only when every frozen candidate source was examined.
    pub attention: Option<EffectiveSeatAttention>,
    pub position: SeatAttentionPosition,
    pub visited: u16,
    pub has_more: bool,
}

/// Bounded exact seat attention. A caller must retain `position` and continue
/// while `has_more`; partial slices never expose false-negative booleans.
/// Logical manifests are visited before projection, and physical projection
/// can only duplicate a warning ID/decision sequence already represented.
pub fn scan_effective_seat_attention(
    db: &Connection,
    seat_id: &str,
    position: Option<SeatAttentionPosition>,
    max_candidates: u16,
) -> Result<EffectiveSeatAttentionSlice, ApiError> {
    if max_candidates == 0 || max_candidates > 100 {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invalid seat attention bound",
        ));
    }
    let decision_seq: i64 = db.query_row(
        "SELECT h.decision_seq FROM seats s JOIN host_instances h ON h.id=s.instance_id WHERE s.id=?1",
        [seat_id],|r|r.get(0),
    ).optional().map_err(store_error)?
        .ok_or_else(||api_error(ErrorCode::NotFound,"seat missing"))?;
    let mut position = match position {
        Some(position) => {
            if position.seat_id != seat_id || position.decision_seq != decision_seq {
                return Err(api_error(ErrorCode::CursorStale, "seat attention changed"));
            }
            if position.physical_warning_after < 0
                || position.physical_warning_after > position.physical_warning_high_water
                || position.manifest_warning_after < 0
                || position.manifest_warning_after > position.manifest_warning_high_water
                || position.invitation_after_seq < 0
                || position.invitation_after_ordinal < 0
                || (position.invitation_after_seq == 0) != (position.invitation_after_ordinal == 0)
            {
                return Err(api_error(
                    ErrorCode::InvalidCursor,
                    "invalid seat attention position",
                ));
            }
            position
        }
        None => {
            let physical_warning_high_water: i64 = db
                .query_row(
                    "SELECT COALESCE(MAX(ordinal),0) FROM messages WHERE kind='warn'",
                    [],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            let manifest_warning_high_water: i64 = db
                .query_row(
                    "SELECT COALESCE(MAX(ordinal),0) FROM prepared_unavailable_warnings",
                    [],
                    |r| r.get(0),
                )
                .map_err(store_error)?;
            SeatAttentionPosition {
                seat_id: seat_id.into(),
                decision_seq,
                invitation_after_seq: 0,
                invitation_after_ordinal: 0,
                invitations_done: false,
                has_pending_invitation: false,
                invitation_frontier: None,
                receipts: None,
                receipts_done: false,
                has_pending_receipt: false,
                receipt_frontier_seq: None,
                physical_warning_after: 0,
                physical_warning_high_water,
                manifest_warning_after: 0,
                manifest_warning_high_water,
                next_manifest_warning: false,
                latest_warning_seq: None,
                latest_warning_offset: None,
            }
        }
    };
    let mut visited = 0u16;
    let mut hold = crate::store::catch_up::HoldCache::new(seat_id);
    while !position.invitations_done && visited < max_candidates {
        let row:Option<(i64,i64,String)> = db.query_row(
            "SELECT i.created_decision_seq,i.ordinal,CASE WHEN EXISTS(SELECT 1 FROM invitation_rejections rejection WHERE rejection.invitation_id=i.id) THEN 'rejected' WHEN c.invitation_id IS NOT NULL THEN 'cancelled' ELSE i.state END FROM invitations i LEFT JOIN invitation_cancellations c ON c.invitation_id=i.id WHERE i.seat_id=?1 AND (i.created_decision_seq,i.ordinal)>(?2,?3) AND i.created_decision_seq<=?4 ORDER BY i.created_decision_seq,i.ordinal LIMIT 1",
            params![seat_id,position.invitation_after_seq,position.invitation_after_ordinal,decision_seq],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
        ).optional().map_err(store_error)?;
        let Some((seq, ordinal, state)) = row else {
            position.invitations_done = true;
            break;
        };
        if seq <= 0 || ordinal <= 0 {
            return Err(api_error(
                ErrorCode::StoreCorrupt,
                "invalid invitation publication key",
            ));
        }
        position.invitation_after_seq = seq;
        position.invitation_after_ordinal = ordinal;
        visited += 1;
        if state == "pending" {
            position.has_pending_invitation = true;
            position.invitation_frontier = Some(
                position
                    .invitation_frontier
                    .map_or((seq, ordinal), |prior| prior.max((seq, ordinal))),
            );
        }
    }
    if position.invitations_done && !position.receipts_done && visited < max_candidates {
        let slice = scan_effective_receipts(
            db,
            &ReceiptScanScope::Seat(seat_id.into()),
            position.receipts.take(),
            max_candidates - visited,
        )?;
        visited = visited.saturating_add(slice.visited);
        position.receipts = Some(slice.position);
        for receipt in slice.items {
            if receipt.state == EffectiveReceiptState::Pending {
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
                // The oracle applies the same catch-up hold and release key as
                // the digest and wake source (spec §7), through the same
                // helper.
                let Some((seq, _)) = hold.attention_key(
                    db,
                    &receipt.thread_id,
                    &receipt.message_id,
                    receipt.sequence,
                    (seq, 0),
                )?
                else {
                    continue;
                };
                position.has_pending_receipt = true;
                position.receipt_frontier_seq = Some(
                    position
                        .receipt_frontier_seq
                        .map_or(seq, |prior| prior.max(seq)),
                );
            }
        }
        position.receipts_done = !slice.has_more;
    }
    while position.invitations_done && position.receipts_done && visited < max_candidates {
        let physical_open = position.physical_warning_after < position.physical_warning_high_water;
        let manifest_open = position.manifest_warning_after < position.manifest_warning_high_water;
        if !physical_open && !manifest_open {
            break;
        }
        let manifest = if physical_open && manifest_open {
            position.next_manifest_warning
        } else {
            manifest_open
        };
        position.next_manifest_warning = !manifest;
        let row: Option<(i64, String)> = if manifest {
            db.query_row(
                "SELECT ordinal,warning_id FROM prepared_unavailable_warnings WHERE ordinal>?1 AND ordinal<=?2 ORDER BY ordinal LIMIT 1",
                params![position.manifest_warning_after,position.manifest_warning_high_water],
                |r|Ok((r.get(0)?,r.get(1)?)),
            ).optional().map_err(store_error)?
        } else {
            db.query_row(
                "SELECT ordinal,id FROM messages WHERE kind='warn' AND ordinal>?1 AND ordinal<=?2 ORDER BY ordinal LIMIT 1",
                params![position.physical_warning_after,position.physical_warning_high_water],
                |r|Ok((r.get(0)?,r.get(1)?)),
            ).optional().map_err(store_error)?
        };
        let Some((ordinal, warning_id)) = row else {
            if manifest {
                position.manifest_warning_after = position.manifest_warning_high_water;
            } else {
                position.physical_warning_after = position.physical_warning_high_water;
            }
            continue;
        };
        if manifest {
            position.manifest_warning_after = ordinal;
        } else {
            position.physical_warning_after = ordinal;
        }
        visited += 1;
        // The digest rule: every recipient's transition counts. Wake further
        // narrows transitions to the affected seat (`warning_wakes_seat`), so
        // compare wake against this oracle only on fixtures without
        // `warning_conditions` rows for non-affected seats.
        if let Some(warning) = effective_warning_by_id(db, &warning_id)?
            && is_warning_recipient(db, &warning_id, seat_id)?
            && warning_condition_actionable(db, &warning)?
        {
            let offset: Option<i64> = db
                .query_row(
                    "SELECT warning_offset FROM prepared_unavailable_warnings WHERE warning_id=?1",
                    [&warning_id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(store_error)?;
            let offset = match offset {
                Some(offset) => offset,
                None => db
                    .query_row(
                        "SELECT event_offset FROM messages WHERE id=?1 AND kind='warn'",
                        [&warning_id],
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
            let key = (warning.event_seq, offset);
            if position
                .latest_warning_seq
                .zip(position.latest_warning_offset)
                .is_none_or(|prior| key > prior)
            {
                position.latest_warning_seq = Some(key.0);
                position.latest_warning_offset = Some(key.1);
            }
        }
    }
    let has_more = !position.invitations_done
        || !position.receipts_done
        || position.physical_warning_after < position.physical_warning_high_water
        || position.manifest_warning_after < position.manifest_warning_high_water;
    let key = |value: Option<(i64, i64)>| -> Result<Option<LogicalPublicationKey>, ApiError> {
        value
            .map(|(seq, offset)| {
                Ok(LogicalPublicationKey {
                    decision_seq: u64::try_from(seq).map_err(|_| {
                        api_error(ErrorCode::StoreCorrupt, "negative attention sequence")
                    })?,
                    event_offset: u64::try_from(offset).map_err(|_| {
                        api_error(ErrorCode::StoreCorrupt, "negative attention offset")
                    })?,
                })
            })
            .transpose()
    };
    let frontier = LogicalAttentionFrontier {
        invitation: key(position.invitation_frontier)?,
        addressed_receipt: key(position.receipt_frontier_seq.map(|seq| (seq, 0)))?,
        actionable_warning: key(position
            .latest_warning_seq
            .zip(position.latest_warning_offset))?,
    };
    let attention = (!has_more).then_some(EffectiveSeatAttention {
        has_pending_invitation: position.has_pending_invitation,
        has_pending_receipt: position.has_pending_receipt,
        latest_warning_seq: position.latest_warning_seq,
        frontier,
    });
    Ok(EffectiveSeatAttentionSlice {
        attention,
        position,
        visited,
        has_more,
    })
}
