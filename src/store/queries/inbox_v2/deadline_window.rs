//! The deadline-class window: a bounded, sorted batch of pending invitations, pre-upgrade
//! binding-unread rows and explicit-ACK receipts.
//!
//! Owns: window collection from the three existing walks, key building (`DeliveryOrderKey`) for
//! each item from immutable facts only (an item's key never changes between pages: an unread
//! row's key deadline is its receipt's stored deadline, else its send-time projection, whatever
//! the receipt's state; ht-yha.71), the sort, and the
//! re-collected-vs-fresh skip rule (a re-collected window skips keys <= the last key emitted from
//! that same window, `DeadlineCursor::window_last`; a fresh window skips nothing; ht-yha.63),
//! bounded by `DEADLINE_WINDOW` items and
//! `WINDOW_VISITS` visits so every page advances. The three walks seek only pending, published
//! items (invitations and receipts through their pending projections, unread through its unread
//! predicate), so settled and unpublished history costs no visit.
//! Hides: how the walks step and where they stop; a better deadline ordering changes only this file.
//! Must not: persist items (only walk positions go into the cursor), sort or resume deadline items
//! anywhere else, or read `messages.priority` (every deadline-class item is priority 0).

use super::*;
use crate::protocol::{
    ids::InvitationId,
    pagination::{DeadlineCursor, DeadlineHighWaters, DeadlineWalks},
    priority::{DeliveryOrderKey, DeliveryPriority, SourceRank},
};
use crate::store::{catch_up::HoldCache, effective::ReceiptScanPosition};

/// Items per window.
pub(super) const DEADLINE_WINDOW: usize = 64;
/// Visit share of one window: a re-collected window always costs less than one page's work.
pub(super) const WINDOW_VISITS: usize = CANDIDATE_LIMIT / 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DeadlineRef {
    Invitation {
        id: InvitationId,
        ordinal: i64,
    },
    Unread {
        message: MessageId,
        ordinal: i64,
        ack: bool,
    },
    Receipt {
        message: MessageId,
        ordinal: i64,
        ack: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DeadlineItem {
    pub(super) key: DeliveryOrderKey,
    pub(super) item: DeadlineRef,
}

pub(super) struct DeadlineWindow {
    items: Vec<DeadlineItem>,
    next: usize,
    /// The last key emitted from this window (carried over from the cursor on a re-collect).
    last: Option<DeliveryOrderKey>,
    end: DeadlineWalks,
    walks_exhausted: bool,
}

fn exhausted(walks: &DeadlineWalks, high: &DeadlineHighWaters) -> bool {
    walks.invitation_after >= high.invitation
        && walks
            .unread_after
            .zip(high.unread)
            .is_none_or(|(a, h)| a >= h)
        && walks.physical_after >= high.physical
        && walks.manifest_after >= high.manifest
}

/// Whether a pushed page holds this ordinary message back (catch-up hold, spec section 7): the
/// attention digest's rule, through the same `HoldCache`. An unpushed page holds nothing.
fn held(
    hold: &mut Option<HoldCache<'_>>,
    db: &Connection,
    thread: &str,
    message: &str,
    sequence: i64,
    decision_seq: i64,
) -> Result<bool, ApiError> {
    match hold {
        Some(hold) => Ok(hold
            .attention_key(db, thread, message, sequence, (decision_seq, 0))?
            .is_none()),
        None => Ok(false),
    }
}

fn unavailable(detail: &str) -> ApiError {
    invalid(detail)
}

impl DeadlineWindow {
    /// Collect one window from `cursor`: re-collect `window_start..window_end` when
    /// `cursor.window_end` is `Some` (skipping keys <= `cursor.window_last`, the last key emitted
    /// from this window), otherwise a fresh window from `cursor.window_start` (skipping nothing).
    /// Every visited candidate is added to `*visits`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn collect(
        db: &Connection,
        seat: &SeatId,
        reading: Option<&BindingId>,
        current_agent: bool,
        cursor: &DeadlineCursor,
        decision: u64,
        visits: &mut usize,
        hold: &mut Option<HoldCache<'_>>,
    ) -> Result<Self, ApiError> {
        let high = &cursor.high;
        let mut at = cursor.window_start;
        let stop = cursor.window_end;
        let mut spent = 0usize;
        let mut items: Vec<DeadlineItem> = Vec::new();
        let more = |items: &[DeadlineItem], spent: usize| {
            items.len() < DEADLINE_WINDOW && spent < WINDOW_VISITS
        };
        // A re-collected window walks exactly its original range: each walk is bounded by the
        // window's end, not only by the high water, so a row that left a walk since the first
        // collection (displayed, settled) never pulls a later row into this window (ht-yha.63).
        let (invitation_high, physical_high, manifest_high) = match &stop {
            Some(e) => (e.invitation_after, e.physical_after, e.manifest_after),
            None => (high.invitation, high.physical, high.manifest),
        };

        // Invitations.
        while more(&items, spent) && at.invitation_after < invitation_high {
            let Some(step) =
                next_invitation(db, seat, at.invitation_after, invitation_high, decision)?
            else {
                at.invitation_after = invitation_high;
                break;
            };
            spent += 1;
            at.invitation_after = step.ordinal;
            if !step.pending {
                continue;
            }
            let (_, deadline) = invitation_confirmation_metadata(db, &step.id)?;
            items.push(DeadlineItem {
                key: DeliveryOrderKey {
                    priority: DeliveryPriority::p0(),
                    deadline,
                    decision_seq: step.decision_seq.max(0) as u64,
                    rank: SourceRank::Invitation,
                    ordinal: step.ordinal.max(0) as u64,
                },
                item: DeadlineRef::Invitation {
                    id: InvitationId::new(step.id),
                    ordinal: step.ordinal,
                },
            });
        }

        // Binding-unread rows (bound reads only).
        if let Some(binding) = reading {
            let high_unread = high
                .unread
                .ok_or_else(|| unavailable("missing unread capture"))?;
            let mut after = at
                .unread_after
                .ok_or_else(|| unavailable("missing unread position"))?;
            let high_unread = match &stop {
                Some(e) => e
                    .unread_after
                    .ok_or_else(|| unavailable("missing unread position"))?,
                None => high_unread,
            };
            while more(&items, spent) && after < high_unread {
                let Some((ordinal, id, decision_seq)) = next_unread(
                    db,
                    seat,
                    binding,
                    after as i64,
                    high_unread as i64,
                    decision,
                )?
                else {
                    after = high_unread;
                    break;
                };
                spent += 1;
                after = ordinal as u64;
                if hold.is_some() {
                    let (thread, sequence): (String, i64) = db
                        .query_row(
                            "SELECT thread_id,sequence FROM messages WHERE id=?1",
                            [&id],
                            |r| Ok((r.get(0)?, r.get(1)?)),
                        )
                        .map_err(store_error)?;
                    if held(hold, db, &thread, &id, sequence, decision_seq)? {
                        continue;
                    }
                }
                // The key's deadline is the receipt's stored deadline, else its send-time
                // projection, whatever its state (not its effective deadline, which a settle can
                // move), so an item ACKed between pages keeps its key; only the ACK offer needs a
                // pending receipt.
                let (receipt, key_deadline) =
                    crate::store::effective::effective_receipt_with_key_deadline(
                        db,
                        &id,
                        seat.as_str(),
                    )?
                    .map_or((None, None), |(r, key)| (Some(r), key));
                let pending = receipt.as_ref().is_some_and(|r| {
                    r.state == EffectiveReceiptState::Pending
                        && r.decision_seq.unwrap_or(0) <= decision as i64
                });
                items.push(DeadlineItem {
                    key: DeliveryOrderKey {
                        priority: DeliveryPriority::p0(),
                        deadline: key_deadline.map(UtcMillis),
                        decision_seq: decision_seq.max(0) as u64,
                        rank: SourceRank::Unread,
                        ordinal: ordinal.max(0) as u64,
                    },
                    item: DeadlineRef::Unread {
                        message: MessageId::new(id),
                        ordinal,
                        ack: current_agent && pending,
                    },
                });
            }
            at.unread_after = Some(after);
        }

        // Receipts.
        while more(&items, spent)
            && (at.physical_after < physical_high || at.manifest_after < manifest_high)
        {
            let step = next_receipt(
                db,
                seat,
                reading,
                ReceiptScanPosition {
                    physical_after: at.physical_after,
                    manifest_after: at.manifest_after,
                    physical_high_water: physical_high,
                    manifest_high_water: manifest_high,
                    next_manifest: at.next_manifest,
                },
                decision,
            )?;
            at.physical_after = step.position.physical_after;
            at.manifest_after = step.position.manifest_after;
            at.next_manifest = step.position.next_manifest;
            if step.visited == 0 {
                at.physical_after = physical_high;
                at.manifest_after = manifest_high;
                break;
            }
            spent += step.visited;
            if let Some(receipt) = step.receipt {
                if held(
                    hold,
                    db,
                    &receipt.thread_id,
                    &receipt.message_id,
                    receipt.sequence,
                    receipt.decision_seq.unwrap_or(0),
                )? {
                    continue;
                }
                items.push(DeadlineItem {
                    key: DeliveryOrderKey {
                        priority: DeliveryPriority::p0(),
                        deadline: receipt.deadline_at.map(UtcMillis),
                        decision_seq: receipt.decision_seq.unwrap_or(0).max(0) as u64,
                        rank: SourceRank::Receipt,
                        ordinal: receipt.source_ordinal.max(0) as u64,
                    },
                    item: DeadlineRef::Receipt {
                        message: MessageId::new(receipt.message_id),
                        ordinal: receipt.source_ordinal,
                        ack: current_agent,
                    },
                });
            }
        }

        *visits += spent;
        // Keys are total (ordinals are unique per source and each source has its own rank), so
        // the sort is exact.
        items.sort_by_key(|a| a.key);
        // Only a re-collected window skips, and only what it emitted itself.
        let last = stop.and(cursor.window_last);
        if let Some(last) = &last {
            items.retain(|i| i.key > *last);
        }
        Ok(Self {
            items,
            next: 0,
            last,
            walks_exhausted: exhausted(&at, high),
            end: at,
        })
    }

    pub(super) fn head(&self) -> Option<&DeadlineItem> {
        self.items.get(self.next)
    }

    pub(super) fn advance(&mut self) {
        if let Some(head) = self.items.get(self.next) {
            self.last = Some(head.key);
            self.next += 1;
        }
    }

    /// Remove the first remaining item for `message` (a body continuation emitted it directly).
    /// The head is advanced past like any emitted item; a deeper item is only dropped. Returns the
    /// head's key when it was the head.
    pub(super) fn take_message(&mut self, message: &MessageId) -> Option<DeliveryOrderKey> {
        let at = (self.next..self.items.len()).find(|&n| match &self.items[n].item {
            DeadlineRef::Unread { message: m, .. } | DeadlineRef::Receipt { message: m, .. } => {
                m == message
            }
            DeadlineRef::Invitation { .. } => false,
        })?;
        if at == self.next {
            let key = self.items[at].key;
            self.last = Some(key);
            self.next += 1;
            Some(key)
        } else {
            self.items.remove(at);
            None
        }
    }

    /// No item left in this window and the walks are exhausted.
    #[cfg(test)]
    pub(super) fn done(&self) -> bool {
        self.next >= self.items.len() && self.walks_exhausted
    }

    /// Write back: a fully emitted window moves `window_start` to its end and clears
    /// `window_end` and `window_last` (and sets `done` when the walks are exhausted); a partly
    /// emitted one stores `window_end = end` and the last key emitted from it, so the next call
    /// re-collects exactly this window and skips exactly what it already emitted.
    pub(super) fn persist(&self, cursor: &mut DeadlineCursor) {
        if self.next >= self.items.len() {
            cursor.window_start = self.end;
            cursor.window_end = None;
            cursor.window_last = None;
            cursor.done = self.walks_exhausted;
        } else {
            cursor.window_end = Some(self.end);
            cursor.window_last = self.last;
        }
    }

    /// Write back a window whose head is only partly shown into `cursor`, the state before that
    /// head: re-collect exactly this window, even when the head is its last item. `window_last`
    /// stays the cursor's own (the head is not emitted yet).
    pub(super) fn persist_partial(&self, cursor: &mut DeadlineCursor) {
        cursor.window_end = Some(self.end);
    }
}

#[cfg(test)]
#[path = "../../../../tests/store/deadline_window.rs"]
mod tests;
