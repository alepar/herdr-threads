//! Hook-pull delivery handlers (delivery-policy DP4-DP6): `DeliveryPull` pages
//! a seat's pending items into one framed text and records them in the offered
//! ledger; `DeliveryConfirm` settles the ids the hook found in the session's
//! transcript. Both decide against the canonical A2 view of the seat, never the
//! claim; the ledger is process-local (a restart re-offers).

use super::dispatch::{DomainService, error};
use crate::{
    delivery::{
        self, Assembler, DeliveryItem, DeliveryKind, DrainEvent, FrameItem, MAX_DRAIN_PAGES,
        PAGE_MAX_BYTES, PAGE_MAX_ITEMS, PageError, PageSource, Stop, drain_pages, frame,
        ledger::{Offer, OfferedLedger},
    },
    harness::adapter::DeliveryMechanism,
    ports::{ModBindingView, ModFingerprint, ModSeatView, ReadContext, StorePort},
    protocol::{
        authority::{
            COOPERATIVE_TOP_LEVEL_PROVENANCE, CallerClaim, CallerRole, Harness, PeerIdentity,
        },
        commands::{Command, InboxQuery, PermitMutation},
        delivery::{
            DeliveryConfirm, DeliveryConfirmItem, DeliveryConfirmOutcome, DeliveryConfirmReason,
            DeliveryConfirmReport, DeliveryPage, DeliveryPull, DeliveryPullReply,
            DeliveryRefusalReason,
        },
        ids::MessageId,
        pagination::{Page, PageRequest},
        results::{ApiError, CommandResult, ErrorCode, InboxBatchV2Item},
        time::CallBudget,
    },
};
use std::collections::{BTreeSet, HashMap};
use std::sync::Mutex;

/// A page stops after this many blocks (delivery-policy DP5 step 6).
pub const PAGE_MAX_BLOCKS: usize = 32;
/// A page stops once its framed text would pass this many bytes (always
/// keeping at least one block).
pub const PAGE_MAX_TEXT_BYTES: usize = 64 * 1024;

/// FNV-1a 64 over the seat's `ModFingerprint` fields (delivery-policy DP4). The
/// fingerprint moves with every pending source (receipts, lazy rows, waking
/// warnings and invitations, the binding generation), so the version moves
/// whenever the pending set does, which the bare `wake_work.attention_version`
/// counter does not on settlement.
///
/// `newest_notice` (the newest unoffered informational notice's ordinal) is
/// appended so an `Unchanged` reply never hides a pending notice. Notices are
/// not part of `ModFingerprint`: they never wake and the mod channel's change
/// detector must not see them.
pub fn attention_version(fingerprint: &ModFingerprint, newest_notice: Option<i64>) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    let mut feed = |value: u64| {
        for byte in value.to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(PRIME);
        }
    };
    feed(fingerprint.attention_version);
    feed(fingerprint.pending_receipts);
    feed(fingerprint.max_pending_ordinal as u64);
    feed(fingerprint.lazy_pending);
    feed(fingerprint.max_lazy_rowid as u64);
    feed(fingerprint.other_pending);
    match fingerprint.binding_generation {
        Some(generation) => {
            feed(1);
            feed(generation);
        }
        None => feed(0),
    }
    match newest_notice {
        Some(ordinal) => {
            feed(1);
            feed(ordinal as u64);
        }
        None => feed(0),
    }
    hash
}

/// The process-local offered ledger behind its lock. The lock is taken only
/// for short in-memory steps, never across a store read.
#[derive(Default)]
pub struct DeliveryPullState {
    ledger: Mutex<OfferedLedger>,
}

impl DeliveryPullState {
    fn lock(&self) -> std::sync::MutexGuard<'_, OfferedLedger> {
        self.ledger
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

impl crate::ports::HookPullOutlook for DeliveryPullState {
    fn next_pull(
        &self,
        seat: &crate::protocol::ids::SeatId,
        binding_generation: u64,
        attention_version: u64,
        now: crate::protocol::time::UtcMillis,
    ) -> crate::ports::NextPull {
        self.lock()
            .next_pull(seat, binding_generation, attention_version, now)
    }

    fn holds_parked(&self, seat: &crate::protocol::ids::SeatId, binding_generation: u64) -> bool {
        self.lock().holds_parked(seat, binding_generation)
    }
}

/// `InboxBatchV2` pages read straight from the store.
struct StorePageSource<'a> {
    store: &'a dyn StorePort,
    read: &'a ReadContext,
    budget: &'a CallBudget,
    seat: &'a crate::protocol::ids::SeatId,
}

impl PageSource for StorePageSource<'_> {
    fn page(&mut self, cursor: Option<String>) -> Result<Page<InboxBatchV2Item>, PageError> {
        let command = Command::InboxBatchV2(InboxQuery {
            seat: Some(self.seat.clone()),
            page: PageRequest {
                cursor,
                limit: PAGE_MAX_ITEMS as u16,
                max_bytes: PAGE_MAX_BYTES as u32,
            },
        });
        match self.store.query(&command, self.read, self.budget) {
            Ok(CommandResult::InboxBatchV2(page)) => Ok(page),
            Ok(_) => Err(PageError::Protocol),
            Err(_) => Err(PageError::Api),
        }
    }
}

fn frame_item(item: &DeliveryItem) -> FrameItem {
    let message = &item.message;
    FrameItem {
        kind: match item.kind {
            DeliveryKind::Message => "message",
            DeliveryKind::Lazy => "lazy",
        }
        .to_owned(),
        id: item.id.as_str().to_owned(),
        thread: Some(message.thread.as_str().to_owned()),
        thread_name: message.thread_name.clone(),
        sender: message.sender.as_ref().map(|seat| seat.as_str().to_owned()),
        sender_name: message.sender_name.clone(),
        author_role: message.author_role.map(|role| role.as_str().to_owned()),
        relays_user: message.relays_user,
        user_intent: message.user_intent.map(|intent| intent.as_str().to_owned()),
        body: message.body.clone(),
    }
}

/// Bytes a block adds to a framed text (the block and its blank-line separator).
fn block_bytes(item: &FrameItem) -> usize {
    frame(std::slice::from_ref(item)).len() - delivery::HEADER.len() - 2 + 2
}

fn refused(reason: DeliveryRefusalReason) -> Result<CommandResult, ApiError> {
    Ok(CommandResult::DeliveryPulled(DeliveryPullReply::Refused {
        reason,
    }))
}

impl DomainService {
    /// The A2 decision shared by pull and confirm (delivery-policy DP6): one
    /// store read through `mod_seat_view`, refused in the table's order.
    fn hook_pull_view(
        &self,
        claim: &CallerClaim,
        budget: &CallBudget,
    ) -> Result<(ModSeatView, ModBindingView), DeliveryRefusalReason> {
        use DeliveryRefusalReason as Refusal;
        if claim.role != CallerRole::TopLevel || claim.instance != self.instance {
            return Err(Refusal::SessionMismatch);
        }
        let view = self
            .store
            .mod_seat_view(&claim.seat, budget)
            .map_err(|_| Refusal::Busy)?;
        let Some(view) = view else {
            return Err(Refusal::Unresolved);
        };
        if view.retired || !view.continuity_resolved {
            return Err(Refusal::Unresolved);
        }
        if view.held {
            return Err(Refusal::Held);
        }
        let Some(binding) = view.binding.clone() else {
            return Err(Refusal::NoBinding);
        };
        if binding.provenance != COOPERATIVE_TOP_LEVEL_PROVENANCE {
            return Err(Refusal::NoBinding);
        }
        if !delivery::declares(&binding.harness, DeliveryMechanism::HookPull) {
            return Err(Refusal::NoHookPull);
        }
        if binding.native_session != claim.native_session.as_str() {
            return Err(Refusal::SessionMismatch);
        }
        Ok((view, binding))
    }

    /// `DeliveryPull` (delivery-policy DP5): the seat's next page of pending
    /// items, or `Unchanged` when nothing new needs showing.
    pub(super) fn delivery_pull(
        &self,
        pull: DeliveryPull,
        budget: &CallBudget,
        read: &ReadContext,
    ) -> Result<CommandResult, ApiError> {
        pull.validate()
            .map_err(|why| error(ErrorCode::InvalidRequest, why))?;
        let (view, binding) = match self.hook_pull_view(&pull.claim, budget) {
            Ok(decided) => decided,
            Err(reason) => return refused(reason),
        };
        let seat = &pull.claim.seat;
        let now = self.clock.utc_now();
        let version = attention_version(&view.fingerprint, view.newest_pending_notice);
        let seen_current = pull.seen_attention_version == Some(version);
        let known: HashMap<MessageId, Offer> = {
            let mut ledger = self.delivery_pull.lock();
            let entry = ledger.seat_mut(seat, binding.generation, &binding.native_session);
            if seen_current
                && entry.read_attention.is_some()
                && !entry.more_pending
                && !entry.notices_pending
                && !entry.any_due(now)
            {
                return Ok(CommandResult::DeliveryPulled(
                    DeliveryPullReply::Unchanged {
                        attention_version: version,
                    },
                ));
            }
            entry
                .offers
                .iter()
                .map(|(id, offer)| (id.clone(), *offer))
                .collect()
        };

        let mut source = StorePageSource {
            store: self.store.as_ref(),
            read,
            budget,
            seat,
        };
        let mut assembler = Assembler::new(None).with_command_prefix(pull.marker_prefix.clone());
        let mut blocks: Vec<FrameItem> = Vec::new();
        let mut shown: Vec<(MessageId, bool)> = Vec::new();
        let mut pending: BTreeSet<MessageId> = BTreeSet::new();
        let mut text_bytes = delivery::HEADER.len();
        let mut more = false;
        let drained = drain_pages(
            &mut source,
            &mut assembler,
            version,
            MAX_DRAIN_PAGES,
            |event| -> Result<Stop, std::convert::Infallible> {
                let DrainEvent::Item(item) = event else {
                    return Ok(Stop::Continue);
                };
                pending.insert(item.id.clone());
                if known.get(&item.id).is_some_and(|offer| !offer.is_due(now)) {
                    return Ok(Stop::Continue);
                }
                let block = frame_item(&item);
                let bytes = block_bytes(&block);
                if !blocks.is_empty()
                    && (blocks.len() >= PAGE_MAX_BLOCKS || text_bytes + bytes > PAGE_MAX_TEXT_BYTES)
                {
                    more = true;
                    return Ok(Stop::Halt);
                }
                text_bytes += bytes;
                shown.push((item.id.clone(), item.message.truncated));
                blocks.push(block);
                Ok(Stop::Continue)
            },
        );
        match drained {
            Ok(()) => {}
            Err(delivery::DrainPagesError::Page(_)) => return refused(DeliveryRefusalReason::Busy),
            Err(
                delivery::DrainPagesError::Assemble(_) | delivery::DrainPagesError::TooManyPages,
            ) => {
                return Err(error(
                    ErrorCode::StoreCorrupt,
                    "inbox paging contract broken",
                ));
            }
            Err(delivery::DrainPagesError::Sink(never)) => match never {},
        }
        // Step 5: one attention block when the version moved and something that
        // wakes the seat (an invitation or a waking warning) is pending.
        let with_attention = !seen_current && view.fingerprint.other_pending > 0;
        // Informational notices (never wake, shown once): carried after the
        // message blocks, settled before the reply by the frontier rule.
        let mut notice_blocks: Vec<FrameItem> = Vec::new();
        let mut notices_settled = false;
        let mut notices_has_more = false;
        let room = PAGE_MAX_BLOCKS.saturating_sub(blocks.len() + usize::from(with_attention));
        let limit = room.min(crate::protocol::results::MAX_NOTICE_PAGE_ITEMS);
        if !more && limit > 0 && view.newest_pending_notice.is_some() {
            let mut page = match self.store.pull_notice_page(seat, limit + 1, budget) {
                Ok(page) => page,
                Err(error) => {
                    eprintln!(
                        "herdr-threads: debug: delivery pull: notice page for seat {} unavailable ({:?}: {}); retried on the next pull",
                        seat.as_str(),
                        error.code,
                        error.detail
                    );
                    Vec::new()
                }
            };
            notices_has_more = page.len() > limit;
            page.truncate(limit);
            let mut kept = 0;
            let mut bytes_used = text_bytes;
            for pulled in &page {
                let block = FrameItem {
                    kind: "notice".to_owned(),
                    id: pulled.notice.warning.as_str().to_owned(),
                    thread: Some(pulled.notice.thread.as_str().to_owned()),
                    thread_name: delivery::assemble::label(&pulled.topic),
                    sender: pulled.author.as_ref().map(|a| a.as_str().to_owned()),
                    sender_name: Some(
                        pulled
                            .author
                            .as_ref()
                            .map_or_else(|| "service".to_owned(), |a| a.as_str().to_owned()),
                    ),
                    body: crate::protocol::watch::notice_marker_for(
                        &pull.marker_prefix,
                        &pulled.notice.warning,
                    ),
                    ..FrameItem::default()
                };
                let bytes = block_bytes(&block);
                if bytes_used + bytes > PAGE_MAX_TEXT_BYTES {
                    break;
                }
                bytes_used += bytes;
                notice_blocks.push(block);
                kept += 1;
            }
            if kept < page.len() {
                notices_has_more = true;
            }
            page.truncate(kept);
            let carried: Vec<_> = page.iter().map(|p| p.notice.clone()).collect();
            notices_settled = match self.store.settle_pulled_notices(
                seat,
                binding.generation,
                &binding.native_session,
                &carried,
                budget,
            ) {
                Ok(settled) => settled,
                Err(error) => {
                    eprintln!(
                        "herdr-threads: debug: delivery pull: notice settle for seat {} failed ({:?}: {}); retried on the next pull",
                        seat.as_str(),
                        error.code,
                        error.detail
                    );
                    false
                }
            };
            if notices_settled {
                blocks.append(&mut notice_blocks);
            }
        }
        let carried_all = notices_settled && !notices_has_more;
        let notices_left = view.newest_pending_notice.is_some() && !carried_all;
        let reply_version = attention_version(
            &view.fingerprint,
            if carried_all {
                None
            } else {
                view.newest_pending_notice
            },
        );
        if with_attention {
            blocks.push(FrameItem {
                kind: "attention".to_owned(),
                id: version.to_string(),
                body: crate::protocol::watch::attention_marker_for(&pull.marker_prefix),
                ..FrameItem::default()
            });
        }
        {
            let mut ledger = self.delivery_pull.lock();
            let entry = ledger.seat_mut(seat, binding.generation, &binding.native_session);
            for (id, truncated) in &shown {
                entry.record(id, now, *truncated);
            }
            if !more {
                // The whole pending set was read: what is no longer in it left.
                entry.retain_pending(&pending);
            }
            entry.more_pending = more;
            entry.notices_pending = notices_left;
            entry.read_attention = (!more).then_some(view.fingerprint.attention_version);
        }
        if blocks.is_empty() {
            return Ok(CommandResult::DeliveryPulled(
                DeliveryPullReply::Unchanged {
                    attention_version: reply_version,
                },
            ));
        }
        let (truncated, offered): (Vec<_>, Vec<_>) =
            shown.into_iter().partition(|(_, truncated)| *truncated);
        Ok(CommandResult::DeliveryPulled(DeliveryPullReply::Page(
            DeliveryPage {
                attention_version: reply_version,
                text: frame(&blocks),
                offered: offered.into_iter().map(|(id, _)| id).collect(),
                truncated: truncated.into_iter().map(|(id, _)| id).collect(),
                more,
            },
        )))
    }

    /// `DeliveryConfirm` (delivery-policy DP6): every id gets a result. Ids the
    /// ledger did not offer to the canonical generation are answered here; the
    /// rest are settled by the store, which decides each again against A2.
    pub(super) fn delivery_confirm(
        &self,
        confirm: DeliveryConfirm,
        peer: PeerIdentity,
        budget: &CallBudget,
        read: ReadContext,
    ) -> Result<CommandResult, ApiError> {
        confirm
            .validate()
            .map_err(|why| error(ErrorCode::InvalidRequest, why))?;
        let report = |outcome: DeliveryConfirmOutcome, reason: Option<DeliveryConfirmReason>| {
            Ok(CommandResult::DeliveryConfirmed(DeliveryConfirmReport {
                results: confirm
                    .ids
                    .iter()
                    .map(|id| DeliveryConfirmItem {
                        id: id.clone(),
                        result: outcome,
                        reason,
                    })
                    .collect(),
            }))
        };
        let (_, binding) = match self.hook_pull_view(&confirm.claim, budget) {
            Ok(decided) => decided,
            Err(DeliveryRefusalReason::NoHookPull) => {
                return report(
                    DeliveryConfirmOutcome::RefusedTerminal,
                    Some(DeliveryConfirmReason::NoHookPull),
                );
            }
            Err(DeliveryRefusalReason::SessionMismatch) => {
                return report(DeliveryConfirmOutcome::StaleGeneration, None);
            }
            Err(DeliveryRefusalReason::Busy) => {
                return report(
                    DeliveryConfirmOutcome::Retryable,
                    Some(DeliveryConfirmReason::Busy),
                );
            }
            // No open binding yet, an unresolved seat or a held target: nothing
            // was offered that could be confirmed now; the hook may retry.
            Err(_) => return report(DeliveryConfirmOutcome::Retryable, None),
        };
        let seat = &confirm.claim.seat;
        let claim_is_older = confirm.claim.binding_generation < binding.generation;
        let forwarded: Vec<MessageId> = {
            let mut ledger = self.delivery_pull.lock();
            let entry = ledger.seat_mut(seat, binding.generation, &binding.native_session);
            confirm
                .ids
                .iter()
                .filter(|id| entry.offers.contains_key(*id))
                .cloned()
                .collect()
        };
        let mut settled_by_store: HashMap<MessageId, DeliveryConfirmItem> = HashMap::new();
        if !forwarded.is_empty() {
            let Ok(agent) = crate::harness::registry::builtins().agent(&binding.harness) else {
                return report(
                    DeliveryConfirmOutcome::RefusedTerminal,
                    Some(DeliveryConfirmReason::NoHookPull),
                );
            };
            // Decided against the canonical binding, never the client's hint.
            let effective = DeliveryConfirm {
                claim: CallerClaim {
                    instance: self.instance.clone(),
                    seat: seat.clone(),
                    binding_generation: binding.generation,
                    role: CallerRole::TopLevel,
                    harness: Harness::Agent(agent),
                    native_session: confirm.claim.native_session.clone(),
                    execution: confirm.claim.execution.clone(),
                    target: confirm.claim.target.clone(),
                },
                ids: forwarded,
                operation: confirm.operation.clone(),
            };
            let CommandResult::DeliveryConfirmed(stored) = self.cooperative_mutation(
                PermitMutation::DeliveryConfirm(effective),
                peer,
                budget,
                read,
                false,
            )?
            else {
                return Err(error(
                    ErrorCode::StoreCorrupt,
                    "delivery confirm returned another result",
                ));
            };
            let mut ledger = self.delivery_pull.lock();
            let entry = ledger.seat_mut(seat, binding.generation, &binding.native_session);
            for item in stored.results {
                if matches!(
                    item.result,
                    DeliveryConfirmOutcome::Settled | DeliveryConfirmOutcome::AlreadySettled
                ) {
                    entry.remove(&item.id);
                }
                settled_by_store.insert(item.id.clone(), item);
            }
        }
        let results = confirm
            .ids
            .iter()
            .map(|id| {
                settled_by_store
                    .remove(id)
                    .unwrap_or_else(|| DeliveryConfirmItem {
                        id: id.clone(),
                        result: if claim_is_older {
                            DeliveryConfirmOutcome::StaleGeneration
                        } else {
                            DeliveryConfirmOutcome::NotOffered
                        },
                        reason: None,
                    })
            })
            .collect();
        Ok(CommandResult::DeliveryConfirmed(DeliveryConfirmReport {
            results,
        }))
    }
}

#[cfg(all(test, feature = "test-support"))]
#[path = "../../tests/service/delivery_pull.rs"]
mod tests;

#[cfg(all(test, feature = "test-support"))]
#[path = "../../tests/service/lazy_hook_pull.rs"]
mod lazy_hook_pull_tests;
