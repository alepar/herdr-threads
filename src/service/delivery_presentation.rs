//! Canonically admitted, serial invitation presentation. This bookkeeping is
//! independent of ordinary receipt ACK and invitation acceptance.
use super::{
    delivery_pull::attention_version,
    dispatch::{DomainService, error},
};
use crate::{
    ports::PresentationInput,
    protocol::{
        authority::PeerIdentity,
        delivery::{
            DeliveryPresentationConfirmV2, DeliveryPullReplyV2, DeliveryPullV2,
            DeliveryRefusalReason, PresentationConsumer,
        },
        ids::SeatId,
        results::{ApiError, CommandResult, ErrorCode},
        time::CallBudget,
    },
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard, TryLockError, Weak},
    time::Duration,
};

#[derive(Default)]
pub(super) struct PresentationState {
    permits: Mutex<HashMap<SeatId, Weak<Mutex<()>>>>,
    #[cfg(test)]
    pub(super) last_work: Mutex<Option<crate::ports::PresentationWork>>,
}
impl PresentationState {
    pub(super) fn permit(&self, seat: &SeatId) -> Result<Arc<Mutex<()>>, ApiError> {
        let mut permits = self.permits.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(permit) = permits.get(seat).and_then(Weak::upgrade) {
            return Ok(permit);
        }
        // This is a fixed-size weak owner cache, never held during canonical
        // store or socket work. A full live cache refuses rather than evicts.
        if permits.len() >= 1024 {
            permits.retain(|_, v| v.strong_count() > 0);
        }
        if permits.len() >= 1024 {
            return Err(error(ErrorCode::ServiceBusy, "presentation owner capacity"));
        }
        let permit = Arc::new(Mutex::new(()));
        permits.insert(seat.clone(), Arc::downgrade(&permit));
        Ok(permit)
    }
}
impl DomainService {
    fn acquire_presentation_owner<'a>(
        &self,
        permit: &'a Mutex<()>,
        budget: &CallBudget,
    ) -> Result<MutexGuard<'a, ()>, ApiError> {
        loop {
            if budget.is_exhausted(self.clock.as_ref()) {
                return Err(super::workers::budget_error(budget));
            }
            let owner = match permit.try_lock() {
                Ok(owner) => Some(owner),
                Err(TryLockError::Poisoned(error)) => Some(error.into_inner()),
                Err(TryLockError::WouldBlock) => None,
            };
            if let Some(owner) = owner {
                // A budget may expire while the mutex is being acquired. Never
                // begin admission with an exhausted budget, even after poison.
                if budget.is_exhausted(self.clock.as_ref()) {
                    return Err(super::workers::budget_error(budget));
                }
                return Ok(owner);
            }
            let remaining = budget
                .deadline
                .0
                .saturating_sub(self.clock.monotonic_now().0);
            // Cancellation wakes this wait directly; bounded polling observes
            // owner release and the caller's clock without holding any lock.
            budget
                .cancellation
                .wait_blocking(Duration::from_millis(remaining.min(10)));
        }
    }

    pub(super) fn presentation_pull(
        &self,
        pull: &DeliveryPullV2,
        peer: PeerIdentity,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        pull.validate().map_err(ApiError::invalid_request)?;
        let permit = match self.mod_channels().presentation_owner(&pull.claim.seat)? {
            Some(permit) => permit,
            None => self.presentation.permit(&pull.claim.seat)?,
        };
        let _owner = self.acquire_presentation_owner(&permit, budget)?;
        let admitted = match pull.consumer {
            PresentationConsumer::Hook => self.hook_presentation_view(&pull.claim, peer, budget),
            PresentationConsumer::Watch => self.watch_presentation_view(&pull.claim, budget),
        };
        let (view, binding) = match admitted {
            Ok(v) => v,
            Err(reason) => {
                return Ok(CommandResult::DeliveryPulledV2(
                    DeliveryPullReplyV2::Refused { reason },
                ));
            }
        };
        if pull.consumer == PresentationConsumer::Watch
            && !self.mod_channels().presentation_owner_live(
                &pull.claim.seat,
                &binding.binding_id,
                pull.claim.binding_generation,
                pull.claim.native_session.as_str(),
                pull.presentation_lease.as_deref().unwrap_or_default(),
            )
        {
            return Ok(CommandResult::DeliveryPulledV2(
                DeliveryPullReplyV2::Refused {
                    reason: DeliveryRefusalReason::SessionMismatch,
                },
            ));
        }
        use crate::{
            delivery::presentation::{
                FrozenPresentationRender, PresentationRenderProfile as Profile, descriptor_key,
            },
            protocol::delivery::{
                PresentationRenderRequest as Request, PresentationRenderSelection as Selection,
            },
        };
        let registry = crate::harness::registry::builtins();
        let adapter = registry
            .agent(&binding.harness)
            .ok()
            .and_then(|id| registry.by_id(id).ok());
        let profile = match &pull.render {
            Request::WatchJsonLinesV1 => Profile::WatchJsonLinesV1,
            Request::Hook {
                confirmable_selector,
                passive_selector,
            } => {
                let codecs = adapter.and_then(|a| {
                    Some((
                        a.presentation_codec(*confirmable_selector)?,
                        a.presentation_codec(*passive_selector)?,
                    ))
                });
                let Some((confirmable, passive)) = codecs else {
                    return Ok(CommandResult::DeliveryPulledV2(
                        DeliveryPullReplyV2::Refused {
                            reason: DeliveryRefusalReason::NoHookPull,
                        },
                    ));
                };
                Profile::Hook {
                    confirmable_selector: *confirmable_selector,
                    passive_selector: *passive_selector,
                    confirmable,
                    passive,
                }
            }
        };
        if !profile.validate() {
            return Ok(CommandResult::DeliveryPulledV2(
                DeliveryPullReplyV2::Refused {
                    reason: DeliveryRefusalReason::NoHookPull,
                },
            ));
        }
        let identity =
            self.store
                .presentation_render_selection(&pull.claim, pull.consumer, budget)?;
        let frozen_render = if let Some(identity) = identity {
            let codec = match identity.selection {
                Selection::WatchJsonLinesV1 => None,
                Selection::Hook { selector } => {
                    let Some(codec) = adapter.and_then(|a| a.presentation_codec(selector)) else {
                        return Ok(CommandResult::DeliveryPulledV2(
                            DeliveryPullReplyV2::Refused {
                                reason: DeliveryRefusalReason::NoHookPull,
                            },
                        ));
                    };
                    if descriptor_key(codec) != identity.descriptor_key {
                        return Ok(CommandResult::DeliveryPulledV2(
                            DeliveryPullReplyV2::Refused {
                                reason: DeliveryRefusalReason::NoHookPull,
                            },
                        ));
                    }
                    Some(codec)
                }
            };
            Some(FrozenPresentationRender { identity, codec })
        } else {
            None
        };
        let now = self.clock.utc_now();
        let (known, blocked, slots, due_snapshot) = {
            let mut ledger = self.delivery_pull.lock();
            let Some(entry) =
                ledger.binding_mut(&pull.claim.seat, &binding.binding_id, binding.generation)
            else {
                return Ok(CommandResult::DeliveryPulledV2(
                    DeliveryPullReplyV2::Refused {
                        reason: DeliveryRefusalReason::Busy,
                    },
                ));
            };
            (
                entry.offers.keys().cloned().collect(),
                entry
                    .offers
                    .iter()
                    .filter(|(_, v)| !v.is_due(now))
                    .map(|(id, _)| id.clone())
                    .collect(),
                crate::delivery::ledger::MAX_OFFERS_PER_BINDING.saturating_sub(entry.offers.len()),
                entry.due_snapshot(pull.consumer),
            )
        };
        let owner_replay = (pull.consumer == PresentationConsumer::Watch)
            .then(|| {
                self.mod_channels().presentation_replay_snapshot(
                    &pull.claim,
                    pull.presentation_lease.as_deref().unwrap_or_default(),
                    &due_snapshot,
                )
            })
            .flatten();
        let version = attention_version(&view.fingerprint, view.newest_pending_notice);
        let output = self.store.presentation_pull(
            PresentationInput {
                binding_id: binding.binding_id.clone(),
                owner_replay,
                ordinary_due_snapshot: Some(due_snapshot),
                render_profile: profile,
                frozen_render,
                pull: pull.clone(),
                attention_version: version,
                ordinary_version: attention_version(&view.fingerprint, None),
                with_attention: view.fingerprint.other_pending > 0,
                retry_messages: true,
                known_messages: known,
                blocked_messages: blocked,
                new_message_slots: slots,
            },
            budget,
        )?;
        #[cfg(test)]
        {
            *self
                .presentation
                .last_work
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = Some(output.work);
        }
        if matches!(output.reply, DeliveryPullReplyV2::Refused { .. }) {
            return Ok(CommandResult::DeliveryPulledV2(output.reply));
        }
        let current = {
            let mut ledger = self.delivery_pull.lock();
            if let Some(entry) =
                ledger.binding_mut(&pull.claim.seat, &binding.binding_id, binding.generation)
            {
                // Remove only the canonical terminal delta. Another route may
                // have added offers since our snapshot; those must survive.
                for id in &output.reclaimed_messages {
                    entry.remove(id);
                }
                if let DeliveryPullReplyV2::Page(page) = &output.reply {
                    debug_assert!(
                        output
                            .owner_replay
                            .emitted
                            .keys()
                            .all(|id| page.offered.contains(id) || page.truncated.contains(id))
                    );
                    debug_assert!(
                        output.owner_replay.emitted.is_empty()
                            || output.owner_replay.source
                                == crate::ports::OrdinaryPresentationSource::OwnerReplay
                    );
                    for id in &page.offered {
                        if !output.owner_replay.emitted.contains_key(id) {
                            entry.record(id, now, false);
                        }
                    }
                    for id in &page.truncated {
                        if !output.owner_replay.emitted.contains_key(id) {
                            entry.record(id, now, true);
                        }
                    }
                    // Passive invitation progress must never consume an open
                    // canonical warning's eligibility or create a native wake.
                    // Only post-emission exact control confirmation may advance seen attention.
                    if !output.owner_replay.source.preserves_native_outlook() {
                        entry.more_pending =
                            page.more && (!page.offered.is_empty() || !page.truncated.is_empty());
                    }
                }
                if let Some(update) = output.ordinary_due_update {
                    entry.apply_due_update(pull.consumer, update);
                }
                output
                    .owner_replay
                    .snapshot
                    .as_ref()
                    .map(|_| entry.due_snapshot(pull.consumer))
            } else {
                None
            }
        };
        // Never nest the registry and ledger locks. The owner permit continues
        // to fence replacement through this successful progress installation.
        if let (Some(snapshot), Some(current)) = (output.owner_replay.snapshot, current) {
            self.mod_channels().apply_presentation_replay(
                snapshot,
                &output.owner_replay.completed,
                &current,
            );
        }
        Ok(CommandResult::DeliveryPulledV2(output.reply))
    }
    pub(super) fn presentation_confirm(
        &self,
        request: &DeliveryPresentationConfirmV2,
        peer: PeerIdentity,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        request.validate().map_err(ApiError::invalid_request)?;
        let permit = match self
            .mod_channels()
            .presentation_owner(&request.claim.seat)?
        {
            Some(permit) => permit,
            None => self.presentation.permit(&request.claim.seat)?,
        };
        let _owner = self.acquire_presentation_owner(&permit, budget)?;
        let admitted = match request.consumer {
            PresentationConsumer::Hook => self.hook_presentation_view(&request.claim, peer, budget),
            PresentationConsumer::Watch => self.watch_presentation_view(&request.claim, budget),
        };
        if admitted.is_err() {
            return Ok(CommandResult::DeliveryPresentationConfirmedV2(
                crate::protocol::delivery::DeliveryPresentationConfirmationReplyV2::empty(
                    crate::protocol::delivery::DeliveryPresentationConfirmationV2::StaleBinding,
                ),
            ));
        }
        let (_, binding) = admitted.expect("checked presentation admission");
        let live = request.consumer != PresentationConsumer::Watch
            || self.mod_channels().presentation_owner_live(
                &request.claim.seat,
                &binding.binding_id,
                request.claim.binding_generation,
                request.claim.native_session.as_str(),
                request.presentation_lease.as_deref().unwrap_or_default(),
            );
        if !live {
            return Ok(CommandResult::DeliveryPresentationConfirmedV2(
                crate::protocol::delivery::DeliveryPresentationConfirmationReplyV2::empty(
                    crate::protocol::delivery::DeliveryPresentationConfirmationV2::StaleBinding,
                ),
            ));
        }

        let reply = self.store.presentation_confirm(request, budget)?;
        reply
            .validate_for(&request.emitted_controls)
            .map_err(|message| error(ErrorCode::StoreCorrupt, message))?;
        // The allocator emits at most ONE control per page. Together with
        // noncontrol retirement on the first confirmation, fresh Confirmed
        // identifies the first applicable Attention confirmation. A future
        // multi-control allocator must revisit this no-reset dependency.
        if request.consumer == PresentationConsumer::Watch
            && reply.status
                == crate::protocol::delivery::DeliveryPresentationConfirmationV2::Confirmed
            && reply.acknowledged_controls.iter().any(|stamp| {
                matches!(
                    stamp,
                    crate::protocol::delivery::PresentationControlStampV2::Attention { .. }
                )
            })
        {
            self.mod_channels().record_presentation_attention_push(
                &request.claim.seat,
                &binding.binding_id,
                binding.generation,
                binding.native_session.as_str(),
                request.presentation_lease.as_deref().unwrap_or_default(),
                self.clock.utc_now(),
            );
        }
        if !reply.acknowledged_controls.is_empty() {
            let mut ledger = self.delivery_pull.lock();
            if let Some(entry) =
                ledger.binding_mut(&request.claim.seat, &binding.binding_id, binding.generation)
            {
                for stamp in &reply.acknowledged_controls {
                    match stamp {
                        crate::protocol::delivery::PresentationControlStampV2::Attention {
                            version,
                            ..
                        } => entry.read_attention = Some(*version),
                        crate::protocol::delivery::PresentationControlStampV2::Cleared {
                            ..
                        } => entry.read_attention = None,
                    }
                }
            }
        }
        Ok(CommandResult::DeliveryPresentationConfirmedV2(reply))
    }
}
