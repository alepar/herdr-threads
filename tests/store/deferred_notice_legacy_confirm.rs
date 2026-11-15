//! Frozen pre-seam confirmation mutation, for real old-reader/new-reader compatibility.
//! Source: 89788571d:src/store/invitation_content.rs::confirm.
//! Original function SHA256: 9e6a0f7b7e9b5751f2c91c879cff1b4c6213ddcdee41096c25a83eb10e580db8.
//! Only the function name changes. Its called helpers retain their historical mutation semantics:
//! binding_scope checks canonical ownership; load now additionally validates well-formed dn1 tags;
//! checked_masks/control_stamp/Controls/retire_outbox/save are unchanged. No helper advances notice
//! frontiers; this fixture must execute the old descriptor-retirement mutation in a transaction.
use super::*;

pub(super) fn historical_confirm(
    db: &Connection,
    request: &DeliveryPresentationConfirmV2,
) -> Result<crate::protocol::delivery::DeliveryPresentationConfirmationReplyV2, ApiError> {
    use crate::protocol::delivery::DeliveryPresentationConfirmationReplyV2 as Reply;
    request.validate().map_err(ApiError::invalid_request)?;
    let Some(binding) = binding_scope(db, &request.claim)? else {
        return Ok(Reply::empty(Confirmation::StaleBinding));
    };
    let mut state = load(db, &request.claim, &binding, request.consumer)?;
    let current = state.outbox_token.as_deref() == Some(&request.token);
    let last = state.last_token.as_deref() == Some(&request.token);
    if !current && !last {
        return Ok(Reply::empty(Confirmation::NotCurrentPage));
    }
    let mut controls: Controls = decode(&state.control_state)?;
    controls.checked()?;
    if last && !current {
        let stamps: Vec<crate::protocol::delivery::PresentationControlStampV2> =
            decode(&state.last_descriptors)?;
        crate::protocol::delivery::validate_control_stamps(&stamps).map_err(corrupt)?;
        let mask = u32::try_from(state.last_confirmed_mask)
            .map_err(|_| corrupt("last presentation mask"))?;
        if mask & !descriptor_mask(stamps.len())? != 0 {
            return Err(corrupt("last presentation mask"));
        }
        if request
            .emitted_controls
            .iter()
            .any(|stamp| !stamps.contains(stamp))
        {
            return Err(ApiError::invalid_request("foreign presentation control"));
        }
        let acknowledged_controls = stamps
            .into_iter()
            .enumerate()
            .filter_map(|(slot, stamp)| {
                (mask & (1u32 << slot) != 0
                    && request.emitted_controls.contains(&stamp)
                    && controls.applicable(&stamp))
                .then_some(stamp)
            })
            .collect();
        return Ok(Reply {
            status: Confirmation::AlreadyConfirmed,
            acknowledged_controls,
        });
    }
    let descriptors: Vec<FrozenPresentation> = decode(&state.outbox_descriptors)?;
    let (mut retired, mut confirmed) = checked_masks(
        &descriptors,
        if current {
            state.outbox_retired_mask
        } else {
            i64::from(descriptor_mask(descriptors.len())?)
        },
        if current {
            state.outbox_confirmed_mask
        } else {
            state.last_confirmed_mask
        },
    )?;
    // Complete subset validation precedes all state/episode effects.
    for stamp in &request.emitted_controls {
        if !descriptors
            .iter()
            .any(|d| control_stamp(&d.item).as_ref() == Some(stamp))
        {
            return Err(ApiError::invalid_request("foreign presentation control"));
        }
    }
    let mut acknowledged_controls = Vec::new();
    let mut changed = false;
    for (slot, descriptor) in descriptors.iter().enumerate() {
        let bit = 1u32 << slot;
        if let Some(stamp) = control_stamp(&descriptor.item) {
            if !request.emitted_controls.contains(&stamp) {
                continue;
            }
            if retired & bit == 0 {
                retired |= bit;
                changed = true;
                if controls.applicable(&stamp) {
                    controls.confirm(&stamp);
                    confirmed |= bit;
                }
            }
            if confirmed & bit != 0 && controls.applicable(&stamp) {
                acknowledged_controls.push(stamp);
            }
        } else if retired & bit == 0 {
            retired |= bit;
            confirmed |= bit;
            changed = true;
        }
    }
    if current {
        state.outbox_retired_mask = i64::from(retired);
        state.outbox_confirmed_mask = i64::from(confirmed);
        state.control_state = encode(&controls)?;
        if retired == descriptor_mask(descriptors.len())? {
            retire_outbox(&mut state)?;
        }
        if changed {
            save(db, &request.claim, &binding, request.consumer, &state)?;
        }
    }
    Ok(Reply {
        status: if changed {
            Confirmation::Confirmed
        } else {
            Confirmation::AlreadyConfirmed
        },
        acknowledged_controls,
    })
}

#[test]
fn old_decoder_and_real_old_confirmation_leave_new_default_reoffer_lossless() {
    use crate::protocol::delivery::{DeliveryPullReplyV2, PresentationItemV2};
    let (mut f, claim, _, _, _) = super::notice_replay_tests::published_notice();
    let binding = super::notice_replay_tests::binding_for_claim(&f, &claim);
    let DeliveryPullReplyV2::Page(page) = f
        .store
        .presentation_pull(watch_input(&f, &claim, &binding, true), &budget())
        .unwrap()
        .reply
    else {
        panic!("deferred page")
    };
    assert!(
        page.items
            .iter()
            .any(|i| matches!(i, PresentationItemV2::Notice { .. }))
    );
    let frontier =
        || super::super::attention::binding_notice_frontier(&f.db, binding.as_str()).unwrap();
    assert_eq!(frontier(), 0);
    let descriptors:String=f.db.query_row("SELECT outbox_descriptors FROM invitation_presentation_consumers WHERE binding_id=?1 AND consumer='watch'",[binding.as_str()],|r|r.get(0)).unwrap();
    // This is the historical deny-unknown-fields wrapper, not a permissive Value decoder.
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct OldFrozen {
        item: PresentationItemV2,
        context_key: Option<String>,
    }
    let old: Vec<OldFrozen> = serde_json::from_str(&descriptors).unwrap();
    assert!(old.iter().any(|d| {
        matches!(d.item, PresentationItemV2::Notice { .. })
            && d.context_key
                .as_deref()
                .is_some_and(|v| v.starts_with("dn1:"))
    }));
    let request = DeliveryPresentationConfirmV2 {
        claim: claim.clone(),
        consumer: PresentationConsumer::Watch,
        presentation_lease: Some("0123456789abcdef0123456789abcdef".into()),
        token: page.presentation_token.clone().unwrap(),
        emitted_controls: page.emitted_controls,
    };
    let tx = f.db.transaction().unwrap();
    assert_eq!(
        historical_confirm(&tx, &request).unwrap().status,
        Confirmation::Confirmed
    );
    tx.commit().unwrap();
    assert_eq!(
        super::super::attention::binding_notice_frontier(&f.db, binding.as_str()).unwrap(),
        0
    );
    let DeliveryPullReplyV2::Page(reoffered) = f
        .store
        .presentation_pull(watch_input(&f, &claim, &binding, false), &budget())
        .unwrap()
        .reply
    else {
        panic!("new default reoffer")
    };
    assert!(
        reoffered
            .items
            .iter()
            .any(|i| matches!(i, PresentationItemV2::Notice { .. }))
    );
    assert_ne!(reoffered.presentation_token, page.presentation_token);
    assert!(super::super::attention::binding_notice_frontier(&f.db, binding.as_str()).unwrap() > 0);
}
fn budget() -> crate::protocol::time::CallBudget {
    crate::protocol::time::CallBudget {
        deadline: crate::protocol::time::MonoInstant(1000),
        cancellation: Default::default(),
    }
}

#[test]
fn dn1_canonical_integer_boundaries_and_kind_separation() {
    use crate::protocol::{delivery::PresentationItemV2, ids::MessageId};
    let make = |key: &str| FrozenPresentation {
        item: PresentationItemV2::Notice {
            warning: MessageId::new("n"),
            thread: ThreadId::new("t"),
            body: String::new(),
        },
        context_key: Some(key.into()),
    };
    let descriptor = make("dn1:18446744073709551615:18446744073709551615:9223372036854775807");
    assert_eq!(descriptor.context_key.as_ref().unwrap().len(), 65);
    let parsed = deferred_notice(&descriptor).unwrap().unwrap();
    assert_eq!(parsed.notice.sequence, u64::MAX);
    assert_eq!(parsed.notice.event_seq, u64::MAX);
    assert_eq!(parsed.ordinal, i64::MAX);
    assert_eq!(
        deferred_notice_key(&parsed),
        descriptor.context_key.unwrap()
    );
    assert_eq!(
        deferred_notice(&make("dn1:0:0:1"))
            .unwrap()
            .unwrap()
            .notice
            .sequence,
        0
    );
    assert!(
        deferred_notice(&FrozenPresentation {
            item: PresentationItemV2::AttentionCleared {
                epoch: "1".into(),
                attention_version: 1
            },
            context_key: None
        })
        .unwrap()
        .is_none()
    );
}

pub(super) fn watch_input(
    f: &super::notice_replay_tests::Fixture,
    claim: &CallerClaim,
    binding: &BindingId,
    deferred: bool,
) -> crate::ports::PresentationInput {
    use crate::{
        delivery::presentation::{FrozenPresentationRender, PresentationRenderProfile},
        ports::PresentationInput,
        protocol::delivery::{DeliveryPullV2, PresentationRenderRequest},
    };
    let frozen = f
        .store
        .presentation_render_selection(claim, PresentationConsumer::Watch, &budget())
        .unwrap()
        .map(|identity| FrozenPresentationRender {
            identity,
            codec: None,
        });
    PresentationInput {
        binding_id: binding.clone(),
        owner_replay: None,
        ordinary_due_snapshot: None,
        frozen_render: frozen,
        render_profile: PresentationRenderProfile::WatchJsonLinesV1,
        pull: DeliveryPullV2 {
            defer_notice_settlement: deferred,
            claim: claim.clone(),
            consumer: PresentationConsumer::Watch,
            render: PresentationRenderRequest::WatchJsonLinesV1,
            presentation_lease: Some("0123456789abcdef0123456789abcdef".into()),
            marker_prefix: vec![],
        },
        attention_version: 0,
        ordinary_version: 0,
        with_attention: false,
        retry_messages: true,
        blocked_messages: vec![],
        known_messages: vec![],
        new_message_slots: 256,
    }
}

#[test]
fn deferred_warning_frontier_retains_canonical_notice_until_presentation() {
    use crate::protocol::delivery::DeliveryPullReplyV2;
    let (f, claim, _, _, _) = super::notice_replay_tests::published_notice();
    let binding = super::notice_replay_tests::binding_for_claim(&f, &claim);
    assert!(
        !super::super::attention::notice_offer_page_for(&f.db, claim.seat.as_str(), &binding, 32)
            .unwrap()
            .is_empty()
    );
    let DeliveryPullReplyV2::Page(page) = f
        .store
        .presentation_pull(watch_input(&f, &claim, &binding, true), &budget())
        .unwrap()
        .reply
    else {
        panic!("warning page")
    };
    assert!(
        !super::super::attention::notice_offer_page_for(&f.db, claim.seat.as_str(), &binding, 32)
            .unwrap()
            .is_empty()
    );
    let request = DeliveryPresentationConfirmV2 {
        claim: claim.clone(),
        consumer: PresentationConsumer::Watch,
        token: page.presentation_token.unwrap(),
        presentation_lease: Some("0123456789abcdef0123456789abcdef".into()),
        emitted_controls: page.emitted_controls,
    };
    assert_eq!(
        f.store
            .presentation_confirm(&request, &budget())
            .unwrap()
            .status,
        Confirmation::Confirmed
    );
    assert!(
        super::super::attention::notice_offer_page_for(&f.db, claim.seat.as_str(), &binding, 32)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn deferred_prefix_refusal_is_atomic_for_actual_mixed_nonordinary_page() {
    use crate::protocol::delivery::{DeliveryPullReplyV2, PresentationItemV2};
    let (f, claim, _, _, _) = super::notice_replay_tests::published_notice();
    let binding = super::notice_replay_tests::binding_for_claim(&f, &claim);
    let mut input = watch_input(&f, &claim, &binding, true);
    input.with_attention = true;
    let DeliveryPullReplyV2::Page(page) =
        f.store.presentation_pull(input, &budget()).unwrap().reply
    else {
        panic!("mixed page")
    };
    assert!(
        page.items
            .iter()
            .any(|i| matches!(i, PresentationItemV2::Notice { .. }))
    );
    assert!(
        page.items
            .iter()
            .any(|i| matches!(i, PresentationItemV2::Invitation { .. }))
    );
    assert!(
        !page.emitted_controls.is_empty(),
        "actual canonical attention must participate"
    );
    let mut state = load(&f.db, &claim, &binding, PresentationConsumer::Watch).unwrap();
    let mut descriptors: Vec<FrozenPresentation> = decode(&state.outbox_descriptors).unwrap();
    let notice = descriptors
        .iter_mut()
        .find(|d| matches!(d.item, PresentationItemV2::Notice { .. }))
        .unwrap();
    let mut capture = deferred_notice(notice).unwrap().unwrap();
    capture.notice.event_seq += 1;
    notice.context_key = Some(deferred_notice_key(&capture));
    state.outbox_descriptors = encode(&descriptors).unwrap();
    save(&f.db, &claim, &binding, PresentationConsumer::Watch, &state).unwrap();
    let before =
        encode(&load(&f.db, &claim, &binding, PresentationConsumer::Watch).unwrap()).unwrap();
    let request = DeliveryPresentationConfirmV2 {
        claim: claim.clone(),
        consumer: PresentationConsumer::Watch,
        token: page.presentation_token.unwrap(),
        presentation_lease: Some("0123456789abcdef0123456789abcdef".into()),
        emitted_controls: page.emitted_controls,
    };
    let reply = f.store.presentation_confirm(&request, &budget()).unwrap();
    assert_eq!(reply.status, Confirmation::NotCurrentPage);
    assert!(reply.acknowledged_controls.is_empty());
    assert_eq!(
        encode(&load(&f.db, &claim, &binding, PresentationConsumer::Watch).unwrap()).unwrap(),
        before
    );
    assert_eq!(
        super::super::attention::binding_notice_frontier(&f.db, binding.as_str()).unwrap(),
        0
    );
}
