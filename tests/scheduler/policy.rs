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
        last_reserved_at_utc: None,
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
        focused: false,
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

fn thread_ids(n: usize) -> Vec<ThreadId> {
    (1..=n).map(|i| ThreadId::new(format!("t{i}"))).collect()
}

#[test]
fn soft_point_math() {
    assert_eq!(soft_point(1_000_000, 100_000, 0.6), 960_000);
    assert_eq!(soft_point(1_000_000, 100_000, 0.5), 950_000);
    // An extension moves the effective deadline; the soft point follows.
    assert_eq!(soft_point(1_100_000, 100_000, 0.6), 1_060_000);
    // Never later than the deadline itself, never earlier than the window allows.
    assert_eq!(soft_point(1_000, 0, 0.6), 1_000);
}

#[test]
fn poke_text_coalesces() {
    assert_eq!(
        poke_text(40_000, &thread_ids(1)),
        "herdr-threads: receipt due in 40s on t1; run herdr-threads inbox"
    );
    assert_eq!(
        poke_text(40_001, &thread_ids(1)),
        "herdr-threads: receipt due in 41s on t1; run herdr-threads inbox"
    );
    assert_eq!(
        poke_text(-5, &thread_ids(2)),
        "herdr-threads: receipt due in 0s on t1, t2; run herdr-threads inbox"
    );
    assert_eq!(
        poke_text(10_000, &thread_ids(8)),
        "herdr-threads: receipt due in 10s on t1, t2, t3, t4, t5, t6, t7, t8; run herdr-threads inbox"
    );
    assert_eq!(
        poke_text(10_000, &thread_ids(10)),
        "herdr-threads: receipt due in 10s on t1, t2, t3, t4, t5, t6, t7, t8 +2 more; run herdr-threads inbox"
    );
}

fn poke_observation(ui: HostUiState, focused: bool) -> HostObservation {
    HostObservation {
        focused,
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
        ui,
        terminal: None,
        occupancy: StructuralOccupancy::Occupied,
        incarnation: IncarnationEvidence::Unknown,
        execution: ExecutionEvidence::Unknown,
        call_id: HostCallId::new("test-call"),
        connection_epoch: 0,
        observation_sequence: 0,
        started_at_mono: MonoInstant(1),
        completed_at_mono: MonoInstant(1),
    }
}

#[test]
fn eligibility_matrix() {
    use crate::harness::recipe::NativeSupport::{Supported, Unsupported};
    let states = [
        HostUiState::Idle,
        HostUiState::ActiveTurn,
        HostUiState::HumanInput,
        HostUiState::ApprovalOrQuestion,
        HostUiState::Unknown,
    ];
    let mut submits = 0;
    let mut stashes = 0;
    for ui in states {
        for focused in [false, true] {
            for during_turn in [Unsupported, Supported] {
                for stash in [Unsupported, Supported] {
                    let caps = PokeCapabilities {
                        composer_stash: stash,
                        poke_during_turn: during_turn,
                    };
                    let decision =
                        poke_eligibility(&poke_observation(ui, focused), true, true, caps);
                    let expected = if focused {
                        None
                    } else {
                        match ui {
                            HostUiState::Idle => Some(PokeDecision::Submit),
                            HostUiState::ActiveTurn => {
                                (during_turn == Supported).then_some(PokeDecision::Submit)
                            }
                            HostUiState::HumanInput => {
                                (stash == Supported).then_some(PokeDecision::Stash)
                            }
                            HostUiState::ApprovalOrQuestion | HostUiState::Unknown => None,
                        }
                    };
                    match expected {
                        Some(expected) => assert_eq!(decision, expected, "{ui:?} {focused}"),
                        None => assert!(
                            matches!(decision, PokeDecision::Skip(_)),
                            "{ui:?} focused={focused} {during_turn:?} {stash:?} -> {decision:?}"
                        ),
                    }
                    submits += usize::from(decision == PokeDecision::Submit);
                    stashes += usize::from(decision == PokeDecision::Stash);
                }
            }
        }
    }
    // Idle unfocused (4 capability combos) + ActiveTurn unfocused with the
    // capability (2); HumanInput unfocused with the capability (2).
    assert_eq!((submits, stashes), (6, 2));
}

#[test]
fn eligibility_requires_bound_recognized_fresh_observation() {
    let caps = PokeCapabilities::NONE;
    let idle = poke_observation(HostUiState::Idle, false);
    assert_eq!(
        poke_eligibility(&idle, true, true, caps),
        PokeDecision::Submit
    );
    assert!(matches!(
        poke_eligibility(&idle, false, true, caps),
        PokeDecision::Skip(_)
    ));
    assert!(matches!(
        poke_eligibility(&idle, true, false, caps),
        PokeDecision::Skip(_)
    ));
    for provenance in [
        ObservationProvenance::CoherentEnumeration,
        ObservationProvenance::UncharacterizedCache,
    ] {
        let stale = HostObservation {
            provenance,
            ..idle.clone()
        };
        assert!(matches!(
            poke_eligibility(&stale, true, true, caps),
            PokeDecision::Skip(_)
        ));
    }
}

mod refusal_backoff {
    use crate::notification::dispatch::DispatchState;
    use crate::notification::policy::{DurableRetry, RetryConfig};
    use crate::ports::{RefusalCause, WakeOutcome};
    use crate::protocol::{
        ids::{SeatId, WakeAttemptId},
        time::MonoInstant,
    };

    const REFUSED: WakeOutcome = WakeOutcome::Refused(RefusalCause::Unavailable);

    fn boot() -> uuid::Uuid {
        uuid::Uuid::parse_str("00000000-0000-4000-8000-000000000001").unwrap()
    }
    fn reserved_state(seat: &SeatId, at: u64) -> DispatchState {
        let mut state =
            DispatchState::new(RetryConfig::default(), MonoInstant(0), boot()).with_refusal_seed(7);
        state
            .restore(
                seat.clone(),
                DurableRetry {
                    retry_step: 0,
                    minimum_delay_ms: 0,
                    effective_delay_ms: 0,
                    ever_reserved: false,
                },
            )
            .unwrap();
        let attempt = WakeAttemptId::new("a");
        state
            .reserved(seat.clone(), attempt.clone(), boot(), MonoInstant(at))
            .unwrap();
        assert!(
            state
                .finish(seat, &attempt, &boot(), MonoInstant(at))
                .unwrap()
        );
        state
    }

    #[test]
    fn can_reserve_requires_ladder_and_refusal_eligibility() {
        let seat = SeatId::new("seat");
        let mut state = reserved_state(&seat, 100_000);
        // Ladder: reserved at 100 s, so the 30 s floor ends at 130 s.
        assert!(!state.can_reserve(&seat, MonoInstant(129_999)));
        assert!(state.can_reserve(&seat, MonoInstant(130_000)));
        // A refusal at 200 s adds its own wait (100 ms +-20 %) on top of an
        // eligible ladder: only the refusal blocks.
        state.record_outcome(&seat, REFUSED, MonoInstant(200_000));
        let due = state
            .next_due_at(MonoInstant(200_000))
            .expect("refusal instant");
        assert!(
            (200_080..=200_120).contains(&due.0),
            "first refusal waits 100 ms +-20 %, got {due:?}"
        );
        assert!(!state.can_reserve(&seat, MonoInstant(due.0 - 1)));
        assert!(state.can_reserve(&seat, due));
        // An elapsed refusal cannot override an unexpired ladder either.
        let mut ladder_blocked = reserved_state(&seat, 100_000);
        ladder_blocked.record_outcome(&seat, REFUSED, MonoInstant(100_000));
        assert!(!ladder_blocked.can_reserve(&seat, MonoInstant(129_999)));
        assert!(ladder_blocked.can_reserve(&seat, MonoInstant(130_000)));
    }

    #[test]
    fn refusal_backoff_doubles_resets_on_submitted_and_ignores_unknown_outcomes() {
        let seat = SeatId::new("seat");
        let mut state = reserved_state(&seat, 0);
        let mut gaps = Vec::new();
        for _ in 0..4 {
            state.record_outcome(&seat, REFUSED, MonoInstant(1_000_000));
            gaps.push(state.next_due_at(MonoInstant(1_000_000)).unwrap().0 - 1_000_000);
        }
        for (n, gap) in gaps.iter().enumerate() {
            let nominal = 100u64 << n;
            assert!(
                *gap * 5 >= nominal * 4 && *gap * 5 <= nominal * 6,
                "refusal {n}: {gap} ms vs nominal {nominal} ms"
            );
        }
        assert_eq!(state.refusal_attempts(&seat), Some(4));
        // An unsent prompt (OutcomeUnknown) neither resets nor advances it.
        for outcome in [
            WakeOutcome::OutcomeUnknown,
            WakeOutcome::TimedOut,
            WakeOutcome::Cancelled,
        ] {
            state.record_outcome(&seat, outcome, MonoInstant(2_000_000));
            assert_eq!(state.refusal_attempts(&seat), Some(4), "{outcome:?}");
        }
        state.record_outcome(&seat, WakeOutcome::Submitted, MonoInstant(2_000_000));
        assert_eq!(state.refusal_attempts(&seat), Some(0));
        assert!(state.can_reserve(&seat, MonoInstant(2_000_000)));
    }

    #[test]
    fn restore_prior_guard_keeps_the_original_anchor() {
        // Kills: a restore that re-anchors at the refusal instant, which would
        // delay the retry by a fresh 30 s ladder floor per refusal.
        let seat = SeatId::new("seat");
        let mut state = DispatchState::new(RetryConfig::default(), MonoInstant(0), boot());
        state
            .restore(
                seat.clone(),
                DurableRetry {
                    retry_step: 0,
                    minimum_delay_ms: 30_000,
                    effective_delay_ms: 30_000,
                    ever_reserved: true,
                },
            )
            .unwrap();
        // Boot anchor 0: eligible at 30 s. Reserve at 30 s, refuse at 31 s.
        let attempt = WakeAttemptId::new("a");
        state
            .reserved(seat.clone(), attempt.clone(), boot(), MonoInstant(30_000))
            .unwrap();
        state
            .finish(&seat, &attempt, &boot(), MonoInstant(31_000))
            .unwrap();
        // Advanced guard (fence miss): next ladder instant is 31 s + 60 s.
        assert!(!state.can_reserve(&seat, MonoInstant(60_000)));
        state.restore_prior_guard(&seat);
        // Restored guard: still the boot anchor, so the ladder is satisfied.
        assert!(state.can_reserve(&seat, MonoInstant(31_000)));
    }
}

#[test]
fn batch_window_uses_elapsed_time_and_restart_caps_backward_wall_jump() {
    use crate::notification::policy::BatchGuard;
    use crate::protocol::time::{MonoInstant, UtcMillis};
    let guard = BatchGuard::restore(UtcMillis(30_100), 30_000, UtcMillis(100), MonoInstant(7));
    assert!(!guard.eligible(MonoInstant(30_006)));
    assert!(guard.eligible(MonoInstant(30_007)));
    let restart = BatchGuard::restore(
        UtcMillis(30_100),
        30_000,
        UtcMillis(20_100),
        MonoInstant(500),
    );
    assert!(!restart.eligible(MonoInstant(10_499)));
    assert!(restart.eligible(MonoInstant(10_500)));
    let backward = BatchGuard::restore(
        UtcMillis(30_100),
        30_000,
        UtcMillis(-90_000),
        MonoInstant(5),
    );
    assert!(backward.eligible(MonoInstant(30_005)));
    let mature = BatchGuard::restore(UtcMillis(30_100), 30_000, UtcMillis(31_000), MonoInstant(4));
    assert!(mature.eligible(MonoInstant(4)));
    let zero = BatchGuard::restore(UtcMillis(30_100), 0, UtcMillis(100), MonoInstant(4));
    assert!(zero.eligible(MonoInstant(4)));
}

#[test]
fn batch_arrivals_do_not_reset_elapsed_guard_and_bypass_preserves_retry_spacing() {
    use crate::notification::dispatch::DispatchState;
    let mut state =
        DispatchState::new(RetryConfig::default(), MonoInstant(0), uuid::Uuid::new_v4());
    let seat = SeatId::new("batch-seat");
    state
        .restore(
            seat.clone(),
            DurableRetry {
                retry_step: 0,
                minimum_delay_ms: 30_000,
                effective_delay_ms: 30_000,
                ever_reserved: true,
            },
        )
        .unwrap();
    let window = Some((UtcMillis(30_000), 30_000));
    assert!(!state.batch_eligible(&seat, window, UtcMillis(0), MonoInstant(0)));
    // Later attention and a backward wall jump keep the original elapsed timer.
    assert!(!state.batch_eligible(&seat, window, UtcMillis(-90_000), MonoInstant(29_999)));
    assert!(state.batch_eligible(&seat, window, UtcMillis(-90_000), MonoInstant(30_000)));
    assert!(state.batch_eligible(&seat, None, UtcMillis(0), MonoInstant(1)));
    assert!(!state.can_reserve(&seat, MonoInstant(1)));
    assert!(state.can_reserve(&seat, MonoInstant(30_000)));
}
