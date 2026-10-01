use super::*;
use crate::ports::{
    ExecutionEvidence, HostObservation, HostUiState, IncarnationEvidence, NativeOccupant,
    ObservationProvenance, StructuralOccupancy, WakeCandidate,
};
use crate::protocol::{
    authority::Harness,
    ids::*,
    time::{MonoInstant, UtcMillis},
};

fn candidate() -> WakeCandidate {
    WakeCandidate {
        seat: SeatId::new("seat"),
        attention_witness: None,
        effectively_retired: false,
        continuity_resolved: true,
        binding_generation: Some(1),
        binding_execution: None,
        target: None,
        reason_bits: 0,
        has_pending_invitation: false,
        has_pending_receipt: false,
        actionable_warning_generation: Some(9),
        actionable_warning_seq: None,
        warning_offer: None,
        attention_version: 0,
        checkpoint_version: 0,
        retry_step: 0,
        reservation_id: None,
        reservation_boot: None,
        last_reservation_id: None,
        last_reservation_boot: None,
        last_reserved_frontier: Default::default(),
        minimum_delay_ms: 0,
        effective_delay_ms: 0,
        last_outcome: None,
    }
}

#[test]
fn successor_occupant_sees_warning_offered_to_predecessor() {
    let mut successor = candidate();
    successor.binding_generation = Some(2);
    successor.binding_execution = Some(ExecutionId::new("B"));
    successor.actionable_warning_seq = Some(9);
    successor.warning_offer = Some(crate::ports::WarningOfferFrontier {
        generation: 1,
        execution: ExecutionId::new("A"),
        offered_through_seq: 10,
    });
    assert!(
        AttentionSnapshot::from_candidate(&successor)
            .select()
            .unwrap()
            .warnings
    );
    successor.warning_offer = Some(crate::ports::WarningOfferFrontier {
        generation: 2,
        execution: ExecutionId::new("B"),
        offered_through_seq: 10,
    });
    assert_eq!(AttentionSnapshot::from_candidate(&successor).select(), None);
    successor.binding_execution = Some(ExecutionId::new("C"));
    assert!(
        AttentionSnapshot::from_candidate(&successor)
            .select()
            .unwrap()
            .warnings
    );
}

#[test]
fn delayed_warning_uses_event_sequence_and_current_occupant_offer() {
    let mut current = candidate();
    current.binding_execution = Some(ExecutionId::new("A"));
    current.actionable_warning_seq = Some(7);
    current.warning_offer = Some(crate::ports::WarningOfferFrontier {
        generation: 1,
        execution: ExecutionId::new("A"),
        offered_through_seq: 10,
    });
    assert_eq!(AttentionSnapshot::from_candidate(&current).select(), None);
    current.actionable_warning_seq = Some(11);
    assert!(
        AttentionSnapshot::from_candidate(&current)
            .select()
            .unwrap()
            .warnings
    );
    current.actionable_warning_seq = Some(7);
    current.binding_generation = Some(2);
    assert!(
        AttentionSnapshot::from_candidate(&current)
            .select()
            .unwrap()
            .warnings
    );
}

#[test]
fn logical_pending_booleans_survive_projection_lag_and_warning_offer() {
    let mut current = candidate();
    current.binding_execution = Some(ExecutionId::new("A"));
    current.actionable_warning_seq = Some(7);
    current.warning_offer = Some(crate::ports::WarningOfferFrontier {
        generation: 1,
        execution: ExecutionId::new("A"),
        offered_through_seq: 10,
    });
    current.reason_bits = 0;
    current.has_pending_invitation = true;
    current.has_pending_receipt = true;
    assert_eq!(
        AttentionSnapshot::from_candidate(&current).select(),
        Some(SelectedAttention {
            invites: true,
            ordinary: true,
            warnings: false,
        })
    );
}

#[test]
fn coalesces_current_reasons_and_silences_info_and_offered_warnings() {
    let mut reasons = candidate();
    reasons.has_pending_invitation = true;
    reasons.has_pending_receipt = true;
    reasons.binding_execution = Some(ExecutionId::new("A"));
    reasons.actionable_warning_seq = Some(9);
    reasons.warning_offer = Some(crate::ports::WarningOfferFrontier {
        generation: 1,
        execution: ExecutionId::new("A"),
        offered_through_seq: 8,
    });
    assert_eq!(
        AttentionSnapshot::from_candidate(&reasons).select(),
        Some(SelectedAttention {
            invites: true,
            ordinary: true,
            warnings: true
        })
    );
    assert_eq!(
        MARKER,
        "herdr-threads: attention pending; run herdr-threads inbox"
    );
    assert_eq!(
        AttentionSnapshot::from_candidate(&WakeCandidate {
            has_pending_invitation: false,
            has_pending_receipt: false,
            warning_offer: Some(crate::ports::WarningOfferFrontier {
                offered_through_seq: 9,
                ..reasons.warning_offer.clone().unwrap()
            }),
            ..reasons.clone()
        })
        .select(),
        None
    );
    assert_eq!(
        AttentionSnapshot::from_candidate(&WakeCandidate {
            actionable_warning_seq: None,
            has_pending_invitation: false,
            has_pending_receipt: false,
            reason_bits: u64::MAX,
            ..reasons.clone()
        })
        .select(),
        None
    );
    assert_eq!(
        AttentionSnapshot::from_candidate(&WakeCandidate {
            warning_offer: Some(crate::ports::WarningOfferFrontier {
                offered_through_seq: 9,
                ..reasons.warning_offer.clone().unwrap()
            }),
            ..reasons.clone()
        })
        .select(),
        Some(SelectedAttention {
            invites: true,
            ordinary: true,
            warnings: false
        })
    );
    assert_eq!(
        AttentionSnapshot::from_candidate(&WakeCandidate {
            effectively_retired: true,
            ..reasons
        })
        .select(),
        None
    );
}

#[test]
fn target_requires_fresh_resolved_native_idle_without_known_input() {
    let observation = HostObservation {
        target: HostTargetId::new("pane"),
        host_boot: HostBootId::new("boot"),
        epoch: 1,
        generation: 7,
        observed_at_utc: UtcMillis(1),
        observed_at_mono: MonoInstant(1),
        provenance: ObservationProvenance::FreshCurrentTarget,
        occupant: Some(NativeOccupant {
            harness: Harness::Codex,
            session: NativeSessionId::new("session"),
            execution: ExecutionId::new("exec"),
            is_top_level: true,
        }),
        ui: HostUiState::Idle,
        terminal: None,
        occupancy: StructuralOccupancy::Unknown,
        incarnation: IncarnationEvidence::Unknown,
        execution: ExecutionEvidence::Unknown,
        call_id: HostCallId::new("test-call"),
        connection_epoch: 0,
        observation_sequence: 0,
        started_at_mono: MonoInstant(1),
        completed_at_mono: MonoInstant(1),
    };
    let state = TargetState {
        observation: &observation,
        resolved: true,
        held: false,
        available: true,
        execution_recognized: true,
        expected_target: &observation.target,
        expected_boot: &observation.host_boot,
        expected_epoch: 1,
        expected_generation: 7,
    };
    assert!(state.may_hint()); // An unregistered native root can recover by hint.
    for ui in [
        HostUiState::ActiveTurn,
        HostUiState::ApprovalOrQuestion,
        HostUiState::HumanInput,
        HostUiState::Unknown,
    ] {
        assert!(
            !TargetState {
                observation: &HostObservation {
                    ui,
                    ..observation.clone()
                },
                ..state
            }
            .may_hint()
        );
    }
    assert!(
        !TargetState {
            held: true,
            ..state
        }
        .may_hint()
    );
    assert!(
        !TargetState {
            available: false,
            ..state
        }
        .may_hint()
    );
    assert!(
        !TargetState {
            execution_recognized: false,
            ..state
        }
        .may_hint()
    );
    assert!(
        !TargetState {
            resolved: false,
            ..state
        }
        .may_hint()
    );
    assert!(
        !TargetState {
            expected_generation: 8,
            ..state
        }
        .may_hint()
    );
    assert!(
        !TargetState {
            expected_epoch: 2,
            ..state
        }
        .may_hint()
    );
    assert!(
        !TargetState {
            expected_target: &HostTargetId::new("other"),
            ..state
        }
        .may_hint()
    );
    assert!(
        !TargetState {
            observation: &HostObservation {
                provenance: ObservationProvenance::UncharacterizedCache,
                ..observation.clone()
            },
            ..state
        }
        .may_hint()
    );
    assert!(
        !TargetState {
            observation: &HostObservation {
                occupant: None,
                ..observation.clone()
            },
            ..state
        }
        .may_hint()
    );
    assert!(
        !TargetState {
            observation: &HostObservation {
                occupant: Some(NativeOccupant {
                    is_top_level: false,
                    ..observation.occupant.clone().unwrap()
                }),
                ..observation.clone()
            },
            ..state
        }
        .may_hint()
    );
}

#[test]
fn reservation_completion_and_new_attention_use_elapsed_anchor() {
    let config = RetryConfig::new(30_000).unwrap();
    let first = RetryGuard::never_reserved(config);
    assert!(first.eligible(MonoInstant(0)));
    let (first, frozen) = first.reserve(MonoInstant(0)).unwrap();
    assert_eq!(
        frozen,
        DurableRetry {
            retry_step: 0,
            minimum_delay_ms: 30_000,
            effective_delay_ms: 30_000,
            ever_reserved: true
        }
    );
    assert!(!first.eligible(MonoInstant(31_999)));
    let after = first
        .complete(MonoInstant(2_000))
        .unwrap()
        .new_attention()
        .unwrap();
    assert!(!after.eligible(MonoInstant(31_999)));
    assert!(after.eligible(MonoInstant(32_000)));
    let (second, frozen) = after.reserve(MonoInstant(32_000)).unwrap();
    assert_eq!(frozen.retry_step, 1);
    assert_eq!(frozen.effective_delay_ms, 60_000);
    assert!(
        !second
            .complete(MonoInstant(33_000))
            .unwrap()
            .eligible(MonoInstant(92_999))
    );
    assert!(
        second
            .complete(MonoInstant(33_000))
            .unwrap()
            .eligible(MonoInstant(93_000))
    );
    let (third, frozen) = second
        .complete(MonoInstant(33_000))
        .unwrap()
        .reserve(MonoInstant(93_000))
        .unwrap();
    assert_eq!((frozen.retry_step, frozen.effective_delay_ms), (2, 120_000));
    let (fourth, frozen) = third
        .complete(MonoInstant(94_000))
        .unwrap()
        .reserve(MonoInstant(214_000))
        .unwrap();
    assert_eq!((frozen.retry_step, frozen.effective_delay_ms), (3, 300_000));
    assert!(
        !fourth
            .new_attention()
            .unwrap()
            .eligible(MonoInstant(244_000))
    );
}

#[test]
fn restart_reanchors_retained_history_even_when_reasons_were_empty() {
    let prior = DurableRetry {
        retry_step: 3,
        minimum_delay_ms: 30_000,
        effective_delay_ms: 300_000,
        ever_reserved: true,
    };
    let config = RetryConfig::new(120_000).unwrap();
    let boot = RetryGuard::from_durable(config, prior, MonoInstant(5_000)).unwrap();
    assert!(!boot.eligible(MonoInstant(304_999)));
    assert!(boot.eligible(MonoInstant(305_000)));
    let shortened = boot.new_attention().unwrap();
    assert!(!shortened.eligible(MonoInstant(124_999)));
    assert!(shortened.eligible(MonoInstant(125_000)));
    let rebooted = RetryGuard::from_durable(config, prior, MonoInstant(600_000)).unwrap();
    assert!(!rebooted.eligible(MonoInstant(899_999)));
    assert!(rebooted.eligible(MonoInstant(900_000)));
}

#[test]
fn new_attention_during_long_attempt_uses_minimum_after_completion() {
    let config = RetryConfig::new(30_000).unwrap();
    for (
        prior_step,
        prior_delay,
        reserved_at,
        expected_step,
        expected_delay,
        completed_at,
        before_minimum,
        at_minimum,
    ) in [
        (1, 60_000, 60_000, 2, 120_000, 65_000, 94_999, 95_000),
        (2, 120_000, 120_000, 3, 300_000, 125_000, 154_999, 155_000),
    ] {
        let prior = DurableRetry {
            retry_step: prior_step,
            minimum_delay_ms: 30_000,
            effective_delay_ms: prior_delay,
            ever_reserved: true,
        };
        let (in_flight, frozen) = RetryGuard::from_durable(config, prior, MonoInstant(0))
            .unwrap()
            .reserve(MonoInstant(reserved_at))
            .unwrap();
        assert_eq!(
            (frozen.retry_step, frozen.effective_delay_ms),
            (expected_step, expected_delay)
        );
        let with_new_work = in_flight.new_attention().unwrap();
        assert!(!with_new_work.eligible(MonoInstant(reserved_at + expected_delay + 30_000)));
        let completed = with_new_work.complete(MonoInstant(completed_at)).unwrap();
        assert!(!completed.eligible(MonoInstant(before_minimum)));
        assert!(completed.eligible(MonoInstant(at_minimum)));
    }
}

#[test]
fn raised_and_lowered_minimums_preserve_prior_frozen_floor() {
    let prior = DurableRetry {
        retry_step: 0,
        minimum_delay_ms: 120_000,
        effective_delay_ms: 120_000,
        ever_reserved: true,
    };
    let lower =
        RetryGuard::from_durable(RetryConfig::new(30_000).unwrap(), prior, MonoInstant(1_000))
            .unwrap()
            .new_attention()
            .unwrap();
    assert!(!lower.eligible(MonoInstant(120_999)));
    assert!(lower.eligible(MonoInstant(121_000)));
    let raised = RetryGuard::from_durable(
        RetryConfig::new(300_000).unwrap(),
        prior,
        MonoInstant(1_000),
    )
    .unwrap();
    assert!(
        !raised
            .new_attention()
            .unwrap()
            .eligible(MonoInstant(300_999))
    );
    assert!(
        raised
            .new_attention()
            .unwrap()
            .eligible(MonoInstant(301_000))
    );
    assert!(RetryConfig::new(29_999).is_err());
}
