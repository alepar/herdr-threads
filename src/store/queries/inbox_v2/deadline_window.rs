//! The deadline-class window: a bounded, sorted batch of pending invitations, pre-upgrade
//! binding-unread rows and explicit-ACK receipts.
//!
//! Owns: window collection from the three existing walks, key building (`DeliveryOrderKey`) for
//! each item, the sort, and the re-collected-vs-fresh skip rule (a re-collected window skips keys
//! <= the last emitted key, a fresh window skips nothing), bounded by `DEADLINE_WINDOW` items and
//! `WINDOW_VISITS` visits so every page advances.
//! Hides: how the walks step and where they stop; a better deadline ordering changes only this file.
//! Must not: persist items (only walk positions go into the cursor), sort or resume deadline items
//! anywhere else, or read `messages.priority` (every deadline-class item is priority 0).

use super::*;
use crate::protocol::{
    ids::InvitationId,
    pagination::{DeadlineCursor, DeadlineHighWaters, DeadlineWalks},
    priority::{DeliveryOrderKey, DeliveryPriority, SourceRank},
};
use crate::store::effective::ReceiptScanPosition;

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

fn unavailable(detail: &str) -> ApiError {
    invalid(detail)
}

impl DeadlineWindow {
    /// Collect one window from `cursor`: re-collect `window_start..window_end` when
    /// `cursor.window_end` is `Some` (skipping keys <= `last`), otherwise a fresh window from
    /// `cursor.window_start` (skipping nothing). Every visited candidate is added to `*visits`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn collect(
        db: &Connection,
        seat: &SeatId,
        reading: Option<&BindingId>,
        current_agent: bool,
        cursor: &DeadlineCursor,
        decision: u64,
        last: Option<&DeliveryOrderKey>,
        visits: &mut usize,
    ) -> Result<Self, ApiError> {
        let high = &cursor.high;
        let mut at = cursor.window_start;
        let stop = cursor.window_end;
        let mut spent = 0usize;
        let mut items: Vec<DeadlineItem> = Vec::new();
        let more = |items: &[DeadlineItem], spent: usize| {
            items.len() < DEADLINE_WINDOW && spent < WINDOW_VISITS
        };

        // Invitations.
        while more(&items, spent)
            && at.invitation_after < high.invitation
            && stop.is_none_or(|e| at.invitation_after < e.invitation_after)
        {
            let Some(step) =
                next_invitation(db, seat, at.invitation_after, high.invitation, decision)?
            else {
                at.invitation_after = high.invitation;
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
            let stop_unread = match &stop {
                Some(e) => Some(
                    e.unread_after
                        .ok_or_else(|| unavailable("missing unread position"))?,
                ),
                None => None,
            };
            while more(&items, spent)
                && after < high_unread
                && stop_unread.is_none_or(|e| after < e)
            {
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
                let receipt = crate::store::effective::effective_receipt(db, &id, seat.as_str())?
                    .filter(|r| {
                        r.state == EffectiveReceiptState::Pending
                            && r.decision_seq.unwrap_or(0) <= decision as i64
                    });
                items.push(DeadlineItem {
                    key: DeliveryOrderKey {
                        priority: DeliveryPriority::p0(),
                        deadline: receipt.as_ref().and_then(|r| r.deadline_at).map(UtcMillis),
                        decision_seq: decision_seq.max(0) as u64,
                        rank: SourceRank::Receipt,
                        ordinal: ordinal.max(0) as u64,
                    },
                    item: DeadlineRef::Unread {
                        message: MessageId::new(id),
                        ordinal,
                        ack: current_agent && receipt.is_some(),
                    },
                });
            }
            at.unread_after = Some(after);
        }

        // Receipts.
        while more(&items, spent)
            && (at.physical_after < high.physical || at.manifest_after < high.manifest)
            && stop.is_none_or(|e| {
                at.physical_after < e.physical_after || at.manifest_after < e.manifest_after
            })
        {
            let step = next_receipt(
                db,
                seat,
                reading,
                ReceiptScanPosition {
                    physical_after: at.physical_after,
                    manifest_after: at.manifest_after,
                    physical_high_water: high.physical,
                    manifest_high_water: high.manifest,
                    next_manifest: at.next_manifest,
                },
                decision,
            )?;
            at.physical_after = step.position.physical_after;
            at.manifest_after = step.position.manifest_after;
            at.next_manifest = step.position.next_manifest;
            if step.visited == 0 {
                at.physical_after = high.physical;
                at.manifest_after = high.manifest;
                break;
            }
            spent += step.visited;
            if let Some(receipt) = step.receipt {
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
        // Keys are total (ordinals are unique per source), so an unstable-free stable sort is exact.
        items.sort_by(|a, b| a.key.cmp(&b.key));
        if stop.is_some()
            && let Some(last) = last
        {
            items.retain(|i| i.key > *last);
        }
        Ok(Self {
            items,
            next: 0,
            walks_exhausted: exhausted(&at, high),
            end: at,
        })
    }

    pub(super) fn head(&self) -> Option<&DeadlineItem> {
        self.items.get(self.next)
    }

    pub(super) fn advance(&mut self) {
        if self.next < self.items.len() {
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
    /// `window_end` (and sets `done` when the walks are exhausted); a partly emitted one stores
    /// `window_end = end` so the next call re-collects exactly this window.
    pub(super) fn persist(&self, cursor: &mut DeadlineCursor) {
        if self.next >= self.items.len() {
            cursor.window_start = self.end;
            cursor.window_end = None;
            cursor.done = self.walks_exhausted;
        } else {
            cursor.window_end = Some(self.end);
        }
    }

    /// Write back a window whose head is only partly shown: re-collect exactly this window, even
    /// when the head is its last item.
    pub(super) fn persist_partial(&self, cursor: &mut DeadlineCursor) {
        cursor.window_end = Some(self.end);
    }
}

#[cfg(test)]
#[path = "../../../../tests/store/deadline_window.rs"]
mod tests;
