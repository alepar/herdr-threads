//! Bounded invitation deadline scans.

use crate::{
    ports::TimeBasis,
    protocol::{
        authority::ObligationRef,
        ids::InvitationId,
        results::{ApiError, ErrorCode},
    },
};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashMap;

use super::{
    connection::{StoreContext, api_error, store_error},
    schema::{next_decision_seq, record_overdue_with_decision_seq},
};

pub const MAX_INVITATION_DUE_BATCH: u16 = 100;

/// Immutable deadline order, capped by the invitation ordinal observed at the
/// start of a traversal. A new traversal picks up later invitations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvitationDueCursor {
    pub high_water_ordinal: i64,
    pub after_deadline: i64,
    pub after_ordinal: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvitationDueBatch {
    pub inspected: u16,
    pub warnings_added: u16,
    pub next: Option<InvitationDueCursor>,
}

#[derive(Clone, Debug)]
pub(crate) struct AdvisoryInvitation {
    id: InvitationId,
    deadline: i64,
    ordinal: i64,
}

pub(crate) struct AdvisoryPage {
    candidates: Vec<AdvisoryInvitation>,
    has_more: bool,
    high_water_ordinal: i64,
}

/// Advisory selection does not authorize any transition. It reads at most the
/// admitted number of physical rows from the pending-unwarned index. The
/// ordinal high water is checked after this bounded index slice.
pub(crate) fn select_invitation_due_candidates(
    conn: &Connection,
    after: Option<InvitationDueCursor>,
    limit: u16,
) -> Result<AdvisoryPage, ApiError> {
    if limit == 0 || limit > MAX_INVITATION_DUE_BATCH {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "invitation due batch limit must be 1..=100",
        ));
    }
    let high_water_ordinal = match after {
        Some(cursor) if cursor.high_water_ordinal >= 0 && cursor.after_ordinal >= 0 => {
            cursor.high_water_ordinal
        }
        Some(_) => {
            return Err(api_error(
                ErrorCode::InvalidRequest,
                "invalid invitation due cursor",
            ));
        }
        None => conn
            .query_row(
                "SELECT coalesce(max(ordinal), 0) FROM invitations",
                [],
                |r| r.get(0),
            )
            .map_err(store_error)?,
    };
    let (after_deadline, after_ordinal) = after
        .map(|cursor| (cursor.after_deadline, cursor.after_ordinal))
        .unwrap_or((i64::MIN, 0));
    let mut statement = conn.prepare_cached("SELECT i.id, i.deadline_at, i.ordinal FROM invitations i INDEXED BY invitations_pending_unwarned WHERE i.state='pending' AND i.warning_message_id IS NULL AND NOT EXISTS (SELECT 1 FROM invitation_cancellations c WHERE c.invitation_id=i.id) AND (i.deadline_at, i.ordinal)>(?1, ?2) ORDER BY i.deadline_at, i.ordinal LIMIT ?3")
        .map_err(store_error)?;
    let rows = statement
        .query_map(
            params![after_deadline, after_ordinal, i64::from(limit)],
            |r| {
                Ok(AdvisoryInvitation {
                    id: InvitationId::new(r.get::<_, String>(0)?),
                    deadline: r.get(1)?,
                    ordinal: r.get(2)?,
                })
            },
        )
        .map_err(store_error)?;
    let candidates = rows.collect::<Result<Vec<_>, _>>().map_err(store_error)?;
    // A full slice may be the last one. Keep its cursor and let the next
    // bounded call establish completion without visiting a preview row.
    let has_more = candidates.len() == usize::from(limit);
    Ok(AdvisoryPage {
        candidates,
        has_more,
        high_water_ordinal,
    })
}

/// The page may have been selected before acceptance or retirement. The shared
/// helper rereads each obligation and its effective seat fence under the write
/// lock; only the decision sampled in this transaction classifies deadlines.
pub(crate) fn apply_invitation_due_candidates(
    context: &StoreContext,
    conn: &mut Connection,
    page: AdvisoryPage,
) -> Result<InvitationDueBatch, ApiError> {
    context.execute_decision(
        conn,
        |_| Ok(()),
        |tx, decision, _| {
            let mut inspected = 0u16;
            let mut warnings_added = 0u16;
            let mut last = None;
            let mut encountered_future = false;
            let mut decision_sequences = HashMap::<String, u64>::new();
            for candidate in &page.candidates {
                inspected += 1;
                last = Some((candidate.deadline, candidate.ordinal));
                // Newly inserted invitations belong to a later traversal.
                if candidate.ordinal > page.high_water_ordinal {
                    continue;
                }
                // Deadline and state are reread; advisory rows can become terminal.
                let current: Option<(String, i64, String, Option<String>, String)> = tx
                    .query_row(
                        "SELECT t.instance_id, i.deadline_at, CASE WHEN c.invitation_id IS NOT NULL THEN 'cancelled' ELSE i.state END, i.warning_message_id, s.state FROM invitations i JOIN threads t ON t.id=i.thread_id JOIN seats s ON s.id=i.seat_id LEFT JOIN invitation_cancellations c ON c.invitation_id=i.id WHERE i.id=?1",
                        [candidate.id.as_str()],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                    )
                    .optional()
                    .map_err(store_error)?;
                if current.as_ref().is_some_and(|row| row.1 > decision.utc.0) {
                    encountered_future = true;
                    break;
                }
                if let Some((instance, _, state, marker, seat_state)) = current {
                    if state != "pending" || marker.is_some() || seat_state == "retired" {
                        continue;
                    }
                    let decision_seq = match decision_sequences.get(&instance) {
                        Some(seq) => *seq,
                        None => {
                            let seq = next_decision_seq(tx, &instance)?;
                            decision_sequences.insert(instance, seq);
                            seq
                        }
                    };
                    let outcome = record_overdue_with_decision_seq(
                        tx,
                        &ObligationRef::Invitation(candidate.id.clone()),
                        &TimeBasis::Decision,
                        decision.utc,
                        decision_seq,
                    )?;
                    warnings_added += u16::from(outcome.inserted);
                }
            }
            let next = if page.has_more && !encountered_future {
                last.map(|(after_deadline, after_ordinal)| InvitationDueCursor {
                    high_water_ordinal: page.high_water_ordinal,
                    after_deadline,
                    after_ordinal,
                })
            } else {
                None
            };
            Ok(InvitationDueBatch {
                inspected,
                warnings_added,
                next,
            })
        },
    )
}

/// One call inspects at most `limit` physical candidates. The caller
/// retains `next` between fair scheduler turns; a full rescan starts at None.
pub fn scan_invitation_due_batch(
    context: &StoreContext,
    conn: &mut Connection,
    after: Option<InvitationDueCursor>,
    limit: u16,
) -> Result<InvitationDueBatch, ApiError> {
    let page = select_invitation_due_candidates(conn, after, limit)?;
    apply_invitation_due_candidates(context, conn, page)
}

#[cfg(test)]
#[path = "../../tests/store/invitation_due.rs"]
mod tests;
