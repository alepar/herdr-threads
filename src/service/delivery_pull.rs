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
        ledger::{MAX_OFFERS_PER_SEAT, Offer, OfferedLedger},
    },
    harness::adapter::DeliveryMechanism,
    ports::{ModBindingView, ModFingerprint, ModSeatView, PulledNotice, ReadContext, StorePort},
    protocol::{
        authority::{CallerClaim, CallerRole, Harness, PeerIdentity},
        commands::{AdvanceCursors, Command, InboxQuery, PermitMutation},
        delivery::{
            DeliveryConfirm, DeliveryConfirmItem, DeliveryConfirmOutcome, DeliveryConfirmReason,
            DeliveryConfirmReport, DeliveryPage, DeliveryPull, DeliveryPullReply,
            DeliveryRefusalReason,
        },
        ids::{MessageId, OperationId},
        pagination::{Page, PageRequest},
        results::{ApiError, CommandResult, ErrorCode, InboxBatchV2Item, MAX_NOTICE_PAGE_ITEMS},
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
    #[cfg(any(test, feature = "test-support"))]
    probe: PullProbe,
}

/// Runs once on the ledger just before a pull records its page (a rotation racing the pull).
#[cfg(any(test, feature = "test-support"))]
type LedgerRace = Box<dyn FnOnce(&mut OfferedLedger) + Send>;

/// Test-only probes of the pull (never compiled into ordinary builds).
#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
struct PullProbe {
    /// Pending items the drains read, summed over pulls.
    items_read: std::sync::atomic::AtomicUsize,
    before_record: Mutex<Option<LedgerRace>>,
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
            claim: None,
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

/// An informational notice block: the warning reference and its pinned read-only body command.
fn notice_block(pulled: &PulledNotice, marker_prefix: &[String]) -> FrameItem {
    FrameItem {
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
        body: crate::protocol::watch::notice_marker_for(marker_prefix, &pulled.notice.warning),
        ..FrameItem::default()
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
    /// store read through `mod_binding_view` for the binding the claim's key names (spec §3),
    /// refused in the table's order.
    fn hook_pull_view(
        &self,
        claim: &CallerClaim,
        peer: PeerIdentity,
        budget: &CallBudget,
    ) -> Result<(ModSeatView, ModBindingView), DeliveryRefusalReason> {
        use DeliveryRefusalReason as Refusal;
        if claim.role != CallerRole::TopLevel || claim.instance != self.instance {
            return Err(Refusal::SessionMismatch);
        }
        let (view, binding) = self.hook_binding_read(claim, budget)?;
        // `SessionMismatch` means only a native-session mismatch: a stale conversation.
        if binding.native_session != claim.native_session.as_str() {
            return Err(Refusal::SessionMismatch);
        }
        if binding.holds_channel() {
            return Ok((view, binding));
        }
        // Superseded by a `/clear` in the same process: a stale conversation, like a rotated one.
        if binding.disconnect_reason.as_deref() == Some("superseded") {
            return Err(Refusal::SessionMismatch);
        }
        if binding.ended_at.is_some() || !binding.top_level || binding.connected {
            return Err(Refusal::NoBinding);
        }
        // A same-session disconnected top-level binding: the same plain cooperative call a CLI
        // `inbox` makes. The store reconnects the disconnected binding for a claim that carries a
        // fingerprint (re-adopting a V32-migrated one with its placement and cursors); a claim
        // without one was refused above.
        if claim.liveness.is_none() {
            return Err(Refusal::NoBinding);
        }
        let advance = AdvanceCursors {
            claim: claim.clone(),
            operation: OperationId::new(uuid::Uuid::new_v4().to_string()),
            displayed: Vec::new(),
        };
        match self.advance_cursors(&advance, peer, budget) {
            Ok(_) => {}
            Err(e) if e.code == ErrorCode::ServiceBusy || e.code == ErrorCode::DeadlineExceeded => {
                return Err(Refusal::Busy);
            }
            Err(_) => return Err(Refusal::NoBinding),
        }
        let (view, binding) = self.hook_binding_read(claim, budget)?;
        if binding.native_session != claim.native_session.as_str() {
            return Err(Refusal::SessionMismatch);
        }
        if binding.holds_channel() {
            Ok((view, binding))
        } else {
            Err(Refusal::NoBinding)
        }
    }

    /// One store read of the claim's binding, refused in the table's order up to the harness check.
    fn hook_binding_read(
        &self,
        claim: &CallerClaim,
        budget: &CallBudget,
    ) -> Result<(ModSeatView, ModBindingView), DeliveryRefusalReason> {
        use DeliveryRefusalReason as Refusal;
        let view = self
            .store
            .mod_binding_view(
                &claim.seat,
                crate::ports::ModBindingRef::Claim(claim),
                budget,
            )
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
        if !delivery::declares(&binding.harness, DeliveryMechanism::HookPull) {
            return Err(Refusal::NoHookPull);
        }
        if binding.harness != claim.harness.as_str() {
            return Err(Refusal::NoBinding);
        }
        Ok((view, binding))
    }

    /// `DeliveryPull` (delivery-policy DP5): the seat's next page of pending
    /// items, or `Unchanged` when nothing new needs showing.
    pub(super) fn delivery_pull(
        &self,
        pull: DeliveryPull,
        peer: PeerIdentity,
        budget: &CallBudget,
        read: &ReadContext,
    ) -> Result<CommandResult, ApiError> {
        pull.validate()
            .map_err(|why| error(ErrorCode::InvalidRequest, why))?;
        let (view, binding) = match self.hook_pull_view(&pull.claim, peer, budget) {
            Ok(decided) => decided,
            Err(reason) => return refused(reason),
        };
        let seat = &pull.claim.seat;
        let now = self.clock.utc_now();
        let version = attention_version(&view.fingerprint, view.newest_pending_notice);
        let seen_current = pull.seen_attention_version == Some(version);
        // The pending set this pull reads; notices are read on their own and are not part of it.
        let pending_version = attention_version(&view.fingerprint, None);
        let (known, whole_read_current): (HashMap<MessageId, Offer>, bool) = {
            let mut ledger = self.delivery_pull.lock();
            // A newer generation's entry: this request read the binding before a rotation.
            let Some(entry) = ledger.seat_mut(seat, binding.generation, &binding.native_session)
            else {
                return refused(DeliveryRefusalReason::Busy);
            };
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
            (
                entry
                    .offers
                    .iter()
                    .map(|(id, offer)| (id.clone(), *offer))
                    .collect(),
                entry.whole_read == Some(pending_version),
            )
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
        // Step 5: one attention block when the version moved and something that
        // wakes the seat (an invitation or a waking warning) is pending.
        let with_attention = !seen_current && view.fingerprint.other_pending > 0;
        // Informational notices (never wake, shown once) ride every page, a message backlog included: the first
        // notice page is read before the drain, which holds back one block slot (two with the attention block) and
        // the first notice block's bytes. Carried after the message blocks, settled before the reply by the
        // frontier rule.
        let mut notice_page: Vec<PulledNotice> = if view.newest_pending_notice.is_some() {
            match self.store.pull_notice_page(
                seat,
                Some(&binding.binding_id),
                MAX_NOTICE_PAGE_ITEMS + 1,
                budget,
            ) {
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
            }
        } else {
            Vec::new()
        };
        let mut notice_blocks: Vec<FrameItem> = notice_page
            .iter()
            .map(|pulled| notice_block(pulled, &pull.marker_prefix))
            .collect();
        let (reserved_blocks, reserved_bytes) = notice_blocks.first().map_or((0, 0), |first| {
            (1 + usize::from(with_attention), block_bytes(first))
        });
        let mut room_for_new = MAX_OFFERS_PER_SEAT.saturating_sub(known.len());
        let mut withheld = false;
        // A full table whose last whole read saw this same pending set can show only its due offers: the drain stops
        // once it has met them all (or never starts), so it does not walk the backlog. A moved pending set (an
        // arrival, a settle elsewhere) reads the whole set, and `retain_pending` frees slots.
        let mut due_left = known.values().filter(|offer| offer.is_due(now)).count();
        let skip_drain = whole_read_current && room_for_new == 0 && due_left == 0;
        let mut short = skip_drain;
        let drained = if skip_drain {
            Ok(())
        } else {
            drain_pages(
                &mut source,
                &mut assembler,
                version,
                MAX_DRAIN_PAGES,
                |event| -> Result<Stop, std::convert::Infallible> {
                    let DrainEvent::Item(item) = event else {
                        return Ok(Stop::Continue);
                    };
                    pending.insert(item.id.clone());
                    let offered = known.get(&item.id);
                    let due = offered.is_some_and(|offer| offer.is_due(now));
                    if due {
                        due_left = due_left.saturating_sub(1);
                    }
                    if offered.is_none() && room_for_new == 0 {
                        // A full offer table takes no new id: it stays pending and unshown, parked for `inbox`
                        // (holds_parked).
                        withheld = true;
                    } else if offered.is_none() || due {
                        let block = frame_item(&item);
                        let bytes = block_bytes(&block);
                        if !blocks.is_empty()
                            && (blocks.len() + reserved_blocks >= PAGE_MAX_BLOCKS
                                || text_bytes + bytes + reserved_bytes > PAGE_MAX_TEXT_BYTES)
                        {
                            more = true;
                            return Ok(Stop::Halt);
                        }
                        if offered.is_none() {
                            room_for_new -= 1;
                        }
                        text_bytes += bytes;
                        shown.push((item.id.clone(), item.message.truncated));
                        blocks.push(block);
                    }
                    if whole_read_current && room_for_new == 0 && due_left == 0 {
                        short = true;
                        return Ok(Stop::Halt);
                    }
                    Ok(Stop::Continue)
                },
            )
        };
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
        #[cfg(any(test, feature = "test-support"))]
        self.delivery_pull
            .probe
            .items_read
            .fetch_add(pending.len(), std::sync::atomic::Ordering::SeqCst);
        let mut notices_settled = false;
        let mut notices_has_more = false;
        let notice_start = blocks.len();
        let mut carried_blocks = 0;
        let room = PAGE_MAX_BLOCKS.saturating_sub(blocks.len() + usize::from(with_attention));
        let limit = room.min(MAX_NOTICE_PAGE_ITEMS);
        if limit > 0 && view.newest_pending_notice.is_some() {
            notices_has_more = notice_page.len() > limit;
            notice_page.truncate(limit);
            notice_blocks.truncate(limit);
            let mut kept = 0;
            let mut bytes_used = text_bytes;
            for block in &notice_blocks {
                let bytes = block_bytes(block);
                if bytes_used + bytes > PAGE_MAX_TEXT_BYTES {
                    break;
                }
                bytes_used += bytes;
                kept += 1;
            }
            if kept < notice_page.len() {
                notices_has_more = true;
            }
            notice_page.truncate(kept);
            notice_blocks.truncate(kept);
            let carried: Vec<_> = notice_page.iter().map(|p| p.notice.clone()).collect();
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
                carried_blocks = notice_blocks.len();
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
            #[cfg(any(test, feature = "test-support"))]
            if let Some(race) = self
                .delivery_pull
                .probe
                .before_record
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .take()
            {
                race(&mut ledger);
            }
            let Some(entry) = ledger.seat_mut(seat, binding.generation, &binding.native_session)
            else {
                // Rotated meanwhile: the page belongs to a superseded generation, so no message is recorded or shown
                // (they stay pending for the new one). Notices this pull settled are committed as carried: return
                // them rather than lose them to a refusal.
                if carried_blocks == 0 {
                    return refused(DeliveryRefusalReason::Busy);
                }
                drop(ledger);
                return Ok(CommandResult::DeliveryPulled(DeliveryPullReply::Page(
                    DeliveryPage {
                        attention_version: reply_version,
                        text: frame(&blocks[notice_start..notice_start + carried_blocks]),
                        offered: Vec::new(),
                        truncated: Vec::new(),
                        more: false,
                    },
                )));
            };
            for (id, truncated) in &shown {
                // Never false here unless a concurrent pull of this generation filled the slots first; such an
                // id was shown once unrecorded (at most one extra showing).
                entry.record(id, now, *truncated);
            }
            if !more && !short {
                // The whole pending set was read: what is no longer in it left.
                entry.retain_pending(&pending);
                entry.whole_read = Some(pending_version);
            }
            // An id withheld at capacity while this whole read freed slots is shown by the next pull instead of
            // hiding behind `Unchanged`.
            entry.more_pending = more || (withheld && !entry.is_full());
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
        let (_, binding) = match self.hook_pull_view(&confirm.claim, peer, budget) {
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
            let Some(entry) = ledger.seat_mut(seat, binding.generation, &binding.native_session)
            else {
                return report(
                    DeliveryConfirmOutcome::Retryable,
                    Some(DeliveryConfirmReason::Busy),
                );
            };
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
                    agent_id: None,
                    liveness: confirm.claim.liveness,
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
            let mut entry = ledger.seat_mut(seat, binding.generation, &binding.native_session);
            for item in stored.results {
                if matches!(
                    item.result,
                    DeliveryConfirmOutcome::Settled | DeliveryConfirmOutcome::AlreadySettled
                ) {
                    // A newer generation's entry is not this request's; its next whole read drops settled ids.
                    if let Some(entry) = entry.as_deref_mut() {
                        entry.remove(&item.id);
                    }
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
