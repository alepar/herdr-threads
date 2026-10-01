use super::*;
use crate::ports::{
    EnumerationEvidence, EvidenceKind, ExecutionEvidence, HostObservation, HostSnapshot,
    HostUiState, IncarnationEvidence, NativeOccupant, ObservationProvenance, StructuralOccupancy,
};
use crate::protocol::authority::Harness;
use crate::protocol::ids::{
    ExecutionId, HostBootId, HostCallId, HostTargetId, NativeSessionId, SeatId, TerminalId,
};
use crate::protocol::time::{MonoInstant, UtcMillis};

fn target(id: &str, terminal: &str, execution: Option<&str>, sequence: u64) -> HostObservation {
    let execution_id = execution.map(ExecutionId::new);
    HostObservation {
        target: HostTargetId::new(id),
        host_boot: HostBootId::new("boot-a"),
        epoch: 1,
        generation: 1,
        observed_at_utc: UtcMillis(1),
        observed_at_mono: MonoInstant(1),
        provenance: ObservationProvenance::FreshCurrentTarget,
        occupant: execution_id.clone().map(|execution| NativeOccupant {
            harness: Harness::Codex,
            session: NativeSessionId::new("conversation"),
            execution,
            is_top_level: true,
        }),
        ui: HostUiState::Idle,
        terminal: Some(TerminalId::new(terminal)),
        occupancy: StructuralOccupancy::Occupied,
        incarnation: IncarnationEvidence::Verified {
            identity: "inc-a".into(),
            evidence_kind: EvidenceKind::CoherentEnumeration,
        },
        execution: execution_id
            .map(|execution| ExecutionEvidence::Verified {
                execution,
                evidence_kind: EvidenceKind::NativeInvocation,
            })
            .unwrap_or(ExecutionEvidence::Unknown),
        call_id: HostCallId::new(format!("call-{sequence}")),
        connection_epoch: 1,
        observation_sequence: sequence,
        started_at_mono: MonoInstant(1),
        completed_at_mono: MonoInstant(1),
    }
}

fn snapshot(sequence: u64, targets: Vec<HostObservation>) -> HostSnapshot {
    HostSnapshot {
        boot: HostBootId::new("boot-a"),
        epoch: 1,
        observation_sequence: sequence,
        complete: true,
        enumeration: EnumerationEvidence::CoherentVerified,
        incarnation: IncarnationEvidence::Verified {
            identity: "inc-a".into(),
            evidence_kind: EvidenceKind::CoherentEnumeration,
        },
        targets,
    }
}

fn structural_proof(
    address: &str,
    terminal: &str,
    sequence: u64,
) -> crate::ports::DurableStructuralProof {
    crate::ports::DurableStructuralProof::from_persisted(
        HostTargetId::new(address),
        TerminalId::new(terminal),
        "inc-a".into(),
        EvidenceKind::CoherentEnumeration,
        HostBootId::new("boot-a"),
        1,
        1,
        sequence,
        1,
    )
    .unwrap()
}

#[test]
fn event_hints_and_periodic_due_never_apply_lifecycle_payloads() {
    let mut lane = ObservationLane::default();
    assert!(lane.snapshot_due(MonoInstant(0)));
    let accepted = snapshot(2, vec![target("address", "terminal-a", None, 2)]);
    let first_ticket = lane.begin_observation().unwrap();
    lane.accept_applied(&accepted, first_ticket, MonoInstant(100))
        .unwrap();
    assert!(!lane.snapshot_due(MonoInstant(200)));
    lane.mark_event_dirty();
    assert!(lane.snapshot_due(MonoInstant(200)));
    assert!(!lane.is_available());
    assert!(lane.check_snapshot(&accepted).is_err());
    let newer = snapshot(3, vec![target("address", "terminal-a", None, 3)]);
    let second_ticket = lane.begin_observation().unwrap();
    lane.accept_applied(&newer, second_ticket, MonoInstant(300))
        .unwrap();
    assert!(lane.is_available());
    assert!(!lane.snapshot_due(MonoInstant(301)));
    assert!(lane.snapshot_due(MonoInstant(5300)));
    let mut reset_without_epoch = snapshot(1, vec![target("address", "terminal-a", None, 1)]);
    assert!(lane.check_snapshot(&reset_without_epoch).is_err());
    reset_without_epoch.epoch = 2;
    reset_without_epoch.targets[0].epoch = 2;
    assert!(lane.check_snapshot(&reset_without_epoch).is_ok());
}

#[test]
fn event_during_delayed_snapshot_keeps_lane_dirty_after_apply() {
    let mut lane = ObservationLane::default();
    let ticket = lane.begin_observation().unwrap();
    lane.mark_event_dirty();
    let accepted = snapshot(2, vec![target("address", "terminal-a", None, 2)]);
    lane.accept_applied(&accepted, ticket, MonoInstant(100))
        .unwrap();
    assert!(lane.snapshot_due(MonoInstant(101)));
    assert!(!lane.is_available());
    let later = lane.begin_observation().unwrap();
    lane.accept_applied(&snapshot(3, vec![]), later, MonoInstant(102))
        .unwrap();
    assert!(lane.is_available());
}

#[test]
fn discarded_late_result_and_wrong_ticket_cannot_apply() {
    let mut lane = ObservationLane::default();
    let old = lane.begin_observation().unwrap();
    assert!(lane.begin_observation().is_err());
    let untrusted = snapshot(0, vec![target("p", "t", None, 0)]);
    assert!(
        lane.accept_applied(&untrusted, old, MonoInstant(10))
            .is_err()
    );
    lane.discard(old).unwrap();
    let next = lane.begin_observation().unwrap();
    let mut fresh = snapshot(1, vec![target("p", "t", None, 1)]);
    fresh.epoch = 2;
    fresh.targets[0].epoch = 2;
    assert!(lane.accept_applied(&fresh, old, MonoInstant(20)).is_err());
    lane.accept_applied(&fresh, next, MonoInstant(20)).unwrap();
}

#[test]
fn failed_capture_marks_local_lane_unavailable_until_new_publication() {
    let mut lane = ObservationLane::default();
    let first = lane.begin_observation().unwrap();
    lane.accept_applied(&snapshot(2, vec![]), first, MonoInstant(10))
        .unwrap();
    assert!(lane.is_available());
    lane.mark_unavailable();
    assert!(!lane.is_available());
    assert!(lane.snapshot_due(MonoInstant(11)));
    let next = lane.begin_observation().unwrap();
    lane.accept_applied(&snapshot(3, vec![]), next, MonoInstant(12))
        .unwrap();
    assert!(lane.is_available());
}

#[test]
fn baseline_holds_every_unclaimed_restored_target_in_bounded_pages() {
    let targets = (0..20)
        .map(|n| target(&format!("pane-{n}"), &format!("terminal-{n}"), None, 2))
        .collect();
    let baseline = snapshot(2, targets);
    let first =
        plan_recovery_baseline_page(&baseline, 0, true, |target| target.as_str() == "pane-3")
            .unwrap();
    assert_eq!(first.targets.len(), 16);
    assert_eq!(first.next_offset, Some(16));
    assert_eq!(
        first.targets[3].disposition,
        crate::ports::RecoveryDisposition::AlreadyOwned
    );
    assert!(
        first.targets.iter().enumerate().all(|(n, item)| n == 3
            || item.disposition == crate::ports::RecoveryDisposition::HeldForRepair)
    );
    let second = plan_recovery_baseline_page(&baseline, 16, true, |_| false).unwrap();
    assert_eq!(second.targets.len(), 4);
    assert_eq!(second.next_offset, None);
    assert!(
        second
            .targets
            .iter()
            .all(|item| item.disposition == crate::ports::RecoveryDisposition::HeldForRepair)
    );
    let mut untrusted = baseline.clone();
    untrusted.enumeration = EnumerationEvidence::CompleteUnverified;
    assert!(plan_recovery_baseline_page(&untrusted, 0, true, |_| false).is_err());
}

#[test]
fn indexed_saved_seat_page_plans_move_without_namespace_scan() {
    use crate::ports::{
        PublishedSnapshot, SeatState, SnapshotGenerationId, SnapshotSavedSeat, SnapshotSeatPage,
        SnapshotTargetMatch,
    };
    let publication = PublishedSnapshot {
        id: SnapshotGenerationId::store_issued("published-a".into()),
        instance: "instance-a".into(),
        boot: HostBootId::new("boot-a"),
        epoch: 1,
        observation_sequence: 2,
        incarnation: "inc-a".into(),
        target_count: 100_000,
        invalidation_revision: 0,
    };
    let page = SnapshotSeatPage {
        publication,
        high_water_ordinal: 1,
        after_ordinal: 1,
        visited: 1,
        has_more: false,
        seats: vec![SnapshotSavedSeat {
            ordinal: 1,
            seat: SeatId::new("seat-a"),
            state: SeatState::Resolved,
            unresolved_reason: None,
            prior_published_observation: None,
            structural_proof: Some(structural_proof("old-address", "terminal-a", 1)),
            target: Some(HostTargetId::new("old-address")),
            terminal: Some(TerminalId::new("terminal-a")),
            binding_generation: 3,
            binding_execution: None,
            active_binding_execution: None,
            bound_boot: Some(HostBootId::new("boot-a")),
            bound_incarnation: Some("inc-a".into()),
            latest_binding_evidence: None,
            observed_match: Some(SnapshotTargetMatch {
                target: HostTargetId::new("new-address"),
                terminal: Some(TerminalId::new("terminal-a")),
                structural_generation: 1,
                observation_sequence: 2,
                connection_epoch: Some(1),
                incarnation_source: Some(EvidenceKind::CoherentEnumeration),
                occupancy: StructuralOccupancy::Occupied,
                verified_execution: None,
                top_level_occupant: false,
            }),
        }],
    };
    let transitions = plan_page(&page).unwrap();
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0].expected_binding_generation, 3);
    assert!(matches!(
        transitions[0].action,
        crate::ports::ReconciliationAction::Move { .. }
    ));

    let mut unchanged_shell = page.clone();
    unchanged_shell.seats[0].target = Some(HostTargetId::new("new-address"));
    unchanged_shell.seats[0].binding_execution = None;
    unchanged_shell.seats[0].bound_boot = None;
    unchanged_shell.seats[0].bound_incarnation = None;
    unchanged_shell.seats[0].structural_proof =
        Some(structural_proof("new-address", "terminal-a", 1));
    unchanged_shell.seats[0]
        .observed_match
        .as_mut()
        .unwrap()
        .occupancy = StructuralOccupancy::EmptyShell;
    assert!(plan_page(&unchanged_shell).unwrap().is_empty());
    unchanged_shell.seats[0].structural_proof = None;
    assert!(matches!(
        plan_page(&unchanged_shell).unwrap()[0].action,
        ReconciliationAction::MarkUnresolved
    ));
    unchanged_shell.seats[0].structural_proof =
        Some(structural_proof("new-address", "terminal-a", 3));
    assert!(
        plan_page(&unchanged_shell).unwrap().is_empty(),
        "an older publication cannot invalidate a later allocation"
    );
    unchanged_shell.seats[0].structural_proof =
        Some(structural_proof("another-address", "terminal-a", 1));
    assert!(matches!(
        plan_page(&unchanged_shell).unwrap()[0].action,
        ReconciliationAction::MarkUnresolved
    ));

    let mut changed = page.clone();
    changed.seats[0].observed_match = None;
    assert!(matches!(
        plan_page(&changed).unwrap()[0].action,
        crate::ports::ReconciliationAction::BeginRetirement { .. }
    ));
    changed.publication.incarnation = "other-incarnation".into();
    assert!(matches!(
        plan_page(&changed).unwrap()[0].action,
        crate::ports::ReconciliationAction::MarkUnresolved
    ));
    changed.seats[0].state = SeatState::Retired;
    assert!(plan_page(&changed).unwrap().is_empty());

    let mut replacement = page;
    replacement.seats[0]
        .observed_match
        .as_mut()
        .unwrap()
        .verified_execution = Some(ExecutionId::new("new-execution"));
    replacement.seats[0]
        .observed_match
        .as_mut()
        .unwrap()
        .top_level_occupant = true;
    assert!(matches!(
        plan_page(&replacement).unwrap()[0].action,
        crate::ports::ReconciliationAction::Replace { .. }
    ));
    replacement.seats[0]
        .observed_match
        .as_mut()
        .unwrap()
        .top_level_occupant = false;
    assert!(matches!(
        plan_page(&replacement).unwrap()[0].action,
        crate::ports::ReconciliationAction::Move { .. }
    ));
}

#[test]
fn current_binding_loss_preserves_structural_seat_and_uses_fenced_revocation() {
    use crate::ports::{
        PublishedSnapshot, SeatState, SnapshotGenerationId, SnapshotSavedSeat, SnapshotSeatPage,
        SnapshotTargetMatch,
    };
    let mut page = SnapshotSeatPage {
        publication: PublishedSnapshot {
            id: SnapshotGenerationId::store_issued("binding-loss".into()),
            instance: "instance-a".into(),
            boot: HostBootId::new("boot-a"),
            epoch: 1,
            observation_sequence: 3,
            incarnation: "inc-a".into(),
            target_count: 1,
            invalidation_revision: 0,
        },
        high_water_ordinal: 1,
        after_ordinal: 1,
        visited: 1,
        has_more: false,
        seats: vec![SnapshotSavedSeat {
            ordinal: 1,
            seat: SeatId::new("seat-a"),
            state: SeatState::Resolved,
            unresolved_reason: None,
            prior_published_observation: None,
            structural_proof: Some(structural_proof("pane-a", "terminal-a", 2)),
            target: Some(HostTargetId::new("pane-a")),
            terminal: Some(TerminalId::new("terminal-a")),
            binding_generation: 7,
            binding_execution: Some(ExecutionId::new("execution-old")),
            active_binding_execution: Some(ExecutionId::new("execution-old")),
            bound_boot: Some(HostBootId::new("boot-a")),
            bound_incarnation: Some("inc-a".into()),
            latest_binding_evidence: None,
            observed_match: Some(SnapshotTargetMatch {
                target: HostTargetId::new("pane-a"),
                terminal: Some(TerminalId::new("terminal-a")),
                structural_generation: 1,
                observation_sequence: 3,
                connection_epoch: Some(1),
                incarnation_source: Some(EvidenceKind::CoherentEnumeration),
                occupancy: StructuralOccupancy::Unknown,
                verified_execution: None,
                top_level_occupant: false,
            }),
        }],
    };
    // native-claude-demo-1 P2. The production Herdr adapter reports
    // occupancy and execution Unknown on every snapshot: that is absence of
    // evidence, and the same terminal in the same verified incarnation keeps
    // the active occupant. Kills: marking unavailable on `occupancy !=
    // Occupied` (Unknown) or on missing execution proof (every ~5 s snapshot
    // opened an unavailability episode and nulled `registered_at`).
    assert!(
        plan_page(&page).unwrap().is_empty(),
        "unknown occupancy on the same terminal must not unseat the occupant"
    );
    // A structurally occupied top-level root that merely lacks execution
    // proof is also not evidence of loss.
    page.seats[0].observed_match.as_mut().unwrap().occupancy = StructuralOccupancy::Occupied;
    page.seats[0]
        .observed_match
        .as_mut()
        .unwrap()
        .top_level_occupant = true;
    assert!(plan_page(&page).unwrap().is_empty());
    // Positive evidence still unseats. Kills: a planner that never marks
    // unavailable (dropping the branch) or ignores an observed empty shell /
    // non-top-level occupant.
    page.seats[0]
        .observed_match
        .as_mut()
        .unwrap()
        .top_level_occupant = false;
    let action = &plan_page(&page).unwrap()[0].action;
    assert!(
        matches!(action, ReconciliationAction::MarkOccupantUnavailable {
        target, terminal, expected_execution
    } if target.as_str()=="pane-a" && terminal.as_str()=="terminal-a" && expected_execution.as_str()=="execution-old"),
        "a non-top-level occupant is positive loss evidence"
    );
    page.seats[0].observed_match.as_mut().unwrap().occupancy = StructuralOccupancy::EmptyShell;
    assert!(
        matches!(plan_page(&page).unwrap().first().map(|t| &t.action),
        Some(ReconciliationAction::MarkOccupantUnavailable { expected_execution, .. })
            if expected_execution.as_str() == "execution-old"),
        "an observed empty shell is positive loss evidence"
    );
    page.seats[0].observed_match.as_mut().unwrap().target = HostTargetId::new("pane-b");
    let action = &plan_page(&page).unwrap()[0].action;
    assert!(
        matches!(action, ReconciliationAction::MarkOccupantUnavailable {
        target, expected_execution, ..
    } if target.as_str()=="pane-b" && expected_execution.as_str()=="execution-old")
    );
    // An observed move with Unknown occupancy follows the terminal and keeps
    // the occupant.
    page.seats[0].observed_match.as_mut().unwrap().occupancy = StructuralOccupancy::Unknown;
    assert!(matches!(
        plan_page(&page).unwrap()[0].action,
        ReconciliationAction::Move { .. }
    ));

    page.seats[0].active_binding_execution = None;
    assert!(matches!(
        plan_page(&page).unwrap()[0].action,
        ReconciliationAction::Move { .. }
    ));
    page.seats[0].observed_match.as_mut().unwrap().target = HostTargetId::new("pane-a");
    assert!(plan_page(&page).unwrap().is_empty());
    page.seats[0].observed_match.as_mut().unwrap().target = HostTargetId::new("pane-b");
    page.seats[0].active_binding_execution = Some(ExecutionId::new("execution-old"));
    let observed = page.seats[0].observed_match.as_mut().unwrap();
    observed.occupancy = StructuralOccupancy::Occupied;
    observed.top_level_occupant = true;
    observed.verified_execution = Some(ExecutionId::new("execution-old"));
    assert!(matches!(
        plan_page(&page).unwrap()[0].action,
        ReconciliationAction::Move { .. }
    ));
    page.seats[0]
        .observed_match
        .as_mut()
        .unwrap()
        .verified_execution = Some(ExecutionId::new("execution-new"));
    assert!(matches!(
        plan_page(&page).unwrap()[0].action,
        ReconciliationAction::Replace { .. }
    ));
}

#[test]
fn host_invalidation_reconfirms_only_proven_same_terminal_and_execution() {
    use crate::ports::{
        PriorPublishedTarget, PublishedSnapshot, SeatState, SnapshotGenerationId,
        SnapshotSavedSeat, SnapshotSeatPage, SnapshotTargetMatch, UnresolvedReason,
    };
    let publication = PublishedSnapshot {
        id: SnapshotGenerationId::store_issued("recovery-publication".into()),
        instance: "instance-a".into(),
        boot: HostBootId::new("boot-a"),
        epoch: 1,
        observation_sequence: 7,
        incarnation: "inc-a".into(),
        target_count: 1,
        invalidation_revision: 2,
    };
    let mut page = SnapshotSeatPage {
        publication,
        high_water_ordinal: 1,
        after_ordinal: 1,
        visited: 1,
        has_more: false,
        seats: vec![SnapshotSavedSeat {
            ordinal: 1,
            seat: SeatId::new("seat-a"),
            state: SeatState::Unresolved,
            unresolved_reason: Some(UnresolvedReason::HostInvalidation),
            prior_published_observation: None,
            structural_proof: None,
            target: Some(HostTargetId::new("old-address")),
            terminal: Some(TerminalId::new("terminal-a")),
            binding_generation: 5,
            binding_execution: Some(ExecutionId::new("execution-old")),
            active_binding_execution: None,
            bound_boot: Some(HostBootId::new("boot-a")),
            bound_incarnation: Some("inc-a".into()),
            latest_binding_evidence: None,
            observed_match: Some(SnapshotTargetMatch {
                target: HostTargetId::new("new-address"),
                terminal: Some(TerminalId::new("terminal-a")),
                structural_generation: 3,
                observation_sequence: 7,
                connection_epoch: Some(1),
                incarnation_source: Some(EvidenceKind::CoherentEnumeration),
                occupancy: StructuralOccupancy::Occupied,
                verified_execution: Some(ExecutionId::new("execution-old")),
                top_level_occupant: true,
            }),
        }],
    };
    assert!(matches!(
        &plan_page(&page).unwrap()[0].action,
        crate::ports::ReconciliationAction::Reconfirm { target, terminal, verified_execution }
            if target.as_str() == "new-address"
                && terminal.as_str() == "terminal-a"
                && verified_execution.as_ref().map(ExecutionId::as_str) == Some("execution-old")
    ));

    page.seats[0]
        .observed_match
        .as_mut()
        .unwrap()
        .verified_execution = Some(ExecutionId::new("execution-new"));
    assert!(matches!(
        &plan_page(&page).unwrap()[0].action,
        crate::ports::ReconciliationAction::Reconfirm { verified_execution, .. }
            if verified_execution.as_ref().map(ExecutionId::as_str) == Some("execution-new")
    ));

    page.seats[0].unresolved_reason = Some(UnresolvedReason::Other);
    assert!(matches!(
        plan_page(&page).unwrap()[0].action,
        crate::ports::ReconciliationAction::MarkUnresolved
    ));
    page.seats[0].unresolved_reason = None;
    assert!(matches!(
        plan_page(&page).unwrap()[0].action,
        crate::ports::ReconciliationAction::MarkUnresolved
    ));
    page.seats[0].unresolved_reason = Some(UnresolvedReason::HostInvalidation);
    page.seats[0].bound_incarnation = Some("restored-incarnation".into());
    assert!(matches!(
        plan_page(&page).unwrap()[0].action,
        crate::ports::ReconciliationAction::MarkUnresolved
    ));
    page.seats[0].bound_incarnation = Some("inc-a".into());
    page.seats[0]
        .observed_match
        .as_mut()
        .unwrap()
        .top_level_occupant = false;
    assert!(matches!(
        plan_page(&page).unwrap()[0].action,
        crate::ports::ReconciliationAction::MarkUnresolved
    ));

    page.seats[0].binding_execution = None;
    let current = page.seats[0].observed_match.as_mut().unwrap();
    current.occupancy = StructuralOccupancy::EmptyShell;
    current.verified_execution = None;
    page.seats[0].prior_published_observation = Some(PriorPublishedTarget {
        generation_id: SnapshotGenerationId::store_issued("prior-publication".into()),
        host_boot: HostBootId::new("boot-a"),
        host_epoch: 1,
        incarnation: "inc-a".into(),
        prior_binding_generation: 4,
        target: SnapshotTargetMatch {
            target: HostTargetId::new("old-address"),
            terminal: Some(TerminalId::new("terminal-a")),
            structural_generation: 2,
            observation_sequence: 6,
            connection_epoch: Some(1),
            incarnation_source: Some(EvidenceKind::CoherentEnumeration),
            occupancy: StructuralOccupancy::EmptyShell,
            verified_execution: None,
            top_level_occupant: false,
        },
    });
    assert!(matches!(
        &plan_page(&page).unwrap()[0].action,
        crate::ports::ReconciliationAction::Reconfirm {
            verified_execution: None,
            ..
        }
    ));
    let proven_shell = page.clone();
    page.seats[0]
        .prior_published_observation
        .as_mut()
        .unwrap()
        .target
        .terminal = Some(TerminalId::new("other-terminal"));
    assert!(matches!(
        plan_page(&page).unwrap()[0].action,
        crate::ports::ReconciliationAction::MarkUnresolved
    ));

    let mut wrong_generation = proven_shell.clone();
    wrong_generation.seats[0]
        .prior_published_observation
        .as_mut()
        .unwrap()
        .prior_binding_generation = 3;
    assert!(matches!(
        plan_page(&wrong_generation).unwrap()[0].action,
        crate::ports::ReconciliationAction::MarkUnresolved
    ));
    let mut occupied_before = proven_shell.clone();
    occupied_before.seats[0]
        .prior_published_observation
        .as_mut()
        .unwrap()
        .target
        .occupancy = StructuralOccupancy::Occupied;
    assert!(matches!(
        plan_page(&occupied_before).unwrap()[0].action,
        crate::ports::ReconciliationAction::MarkUnresolved
    ));
    let mut unknown_now = proven_shell.clone();
    unknown_now.seats[0]
        .observed_match
        .as_mut()
        .unwrap()
        .occupancy = StructuralOccupancy::Unknown;
    assert!(matches!(
        plan_page(&unknown_now).unwrap()[0].action,
        crate::ports::ReconciliationAction::MarkUnresolved
    ));
    let mut no_prior = proven_shell;
    no_prior.seats[0].prior_published_observation = None;
    assert!(matches!(
        plan_page(&no_prior).unwrap()[0].action,
        crate::ports::ReconciliationAction::MarkUnresolved
    ));
}

/// Root decision wave-2 fix1 (b): a host-invalidated seat is reconfirmed
/// structurally when the fresh publication shows the same terminal ID in the
/// same verified host boot/incarnation as its latest binding's stored
/// evidence, with occupancy and execution Unknown exactly as the production
/// Herdr 0.9.1 adapter reports them.
/// Kills: removing `same_terminal_binding_bridge` (the production-shaped page
/// stays MarkUnresolved forever), and dropping any one of its comparisons
/// (binding terminal, boot, incarnation, qualified per-target incarnation
/// evidence), each pinned below by a variant that must stay MarkUnresolved.
#[test]
fn host_invalidated_seat_reconfirms_structurally_from_binding_evidence_with_unknown_occupancy() {
    use crate::ports::{
        BindingEvidence, PublishedSnapshot, ReconciliationAction, SeatState, SnapshotGenerationId,
        SnapshotSavedSeat, SnapshotSeatPage, SnapshotTargetMatch, UnresolvedReason,
    };
    let page = SnapshotSeatPage {
        publication: PublishedSnapshot {
            id: SnapshotGenerationId::store_issued("publication".into()),
            instance: "instance".into(),
            boot: HostBootId::new("boot-a"),
            epoch: 1,
            observation_sequence: 7,
            incarnation: "inc-a".into(),
            target_count: 1,
            invalidation_revision: 2,
        },
        high_water_ordinal: 1,
        after_ordinal: 0,
        visited: 1,
        has_more: false,
        seats: vec![SnapshotSavedSeat {
            ordinal: 1,
            seat: SeatId::new("seat-a"),
            state: SeatState::Unresolved,
            unresolved_reason: Some(UnresolvedReason::HostInvalidation),
            prior_published_observation: None,
            structural_proof: None,
            target: Some(HostTargetId::new("pane")),
            terminal: Some(TerminalId::new("terminal-a")),
            binding_generation: 3,
            binding_execution: Some(ExecutionId::new("execution-old")),
            active_binding_execution: None,
            bound_boot: Some(HostBootId::new("boot-a")),
            bound_incarnation: Some("inc-a".into()),
            latest_binding_evidence: Some(BindingEvidence {
                terminal: TerminalId::new("terminal-a"),
                host_boot: HostBootId::new("boot-a"),
                incarnation: "inc-a".into(),
            }),
            // Production adapter shape: structure only.
            observed_match: Some(SnapshotTargetMatch {
                target: HostTargetId::new("pane"),
                terminal: Some(TerminalId::new("terminal-a")),
                structural_generation: 1,
                observation_sequence: 7,
                connection_epoch: Some(1),
                incarnation_source: Some(EvidenceKind::CoherentEnumeration),
                occupancy: StructuralOccupancy::Unknown,
                verified_execution: None,
                top_level_occupant: false,
            }),
        }],
    };
    let action = |page: &SnapshotSeatPage| plan_page(page).unwrap()[0].action.clone();
    assert_eq!(
        action(&page),
        ReconciliationAction::ReconfirmStructure {
            target: HostTargetId::new("pane"),
            terminal: TerminalId::new("terminal-a"),
        }
    );
    // An agent that exited during the outage does not block the seat: the
    // binding already ended at invalidation and its successor registers anew.
    let mut empty = page.clone();
    empty.seats[0].observed_match.as_mut().unwrap().occupancy = StructuralOccupancy::EmptyShell;
    assert!(matches!(
        action(&empty),
        ReconciliationAction::ReconfirmStructure { .. }
    ));
    // Verified execution evidence keeps its own (unchanged) reconfirmation.
    let mut verified = page.clone();
    let observed = verified.seats[0].observed_match.as_mut().unwrap();
    observed.occupancy = StructuralOccupancy::Occupied;
    observed.top_level_occupant = true;
    observed.verified_execution = Some(ExecutionId::new("execution-new"));
    assert!(matches!(
        action(&verified),
        ReconciliationAction::Reconfirm { verified_execution: Some(ref execution), .. }
            if execution.as_str() == "execution-new"
    ));
    let unresolved = |label: &str, page: &SnapshotSeatPage| {
        assert_eq!(
            action(page),
            ReconciliationAction::MarkUnresolved,
            "{label} must stay fail-closed"
        );
    };
    let mut missing = page.clone();
    missing.seats[0].latest_binding_evidence = None;
    unresolved("binding without evidence", &missing);
    let mut other_terminal = page.clone();
    other_terminal.seats[0]
        .latest_binding_evidence
        .as_mut()
        .unwrap()
        .terminal = TerminalId::new("terminal-b");
    unresolved("binding evidence for another terminal", &other_terminal);
    let mut other_incarnation = page.clone();
    other_incarnation.seats[0]
        .latest_binding_evidence
        .as_mut()
        .unwrap()
        .incarnation = "inc-old".into();
    unresolved("binding from another incarnation", &other_incarnation);
    let mut other_boot = page.clone();
    other_boot.seats[0]
        .latest_binding_evidence
        .as_mut()
        .unwrap()
        .host_boot = HostBootId::new("boot-old");
    unresolved("binding from another host boot", &other_boot);
    let mut unqualified = page.clone();
    unqualified.seats[0]
        .observed_match
        .as_mut()
        .unwrap()
        .connection_epoch = None;
    unresolved("target without qualified incarnation", &unqualified);
    let mut unsourced = page.clone();
    unsourced.seats[0]
        .observed_match
        .as_mut()
        .unwrap()
        .incarnation_source = None;
    unresolved("target without incarnation source", &unsourced);
    // Only a host-invalidated seat is recoverable at all.
    let mut other_reason = page.clone();
    other_reason.seats[0].unresolved_reason = Some(UnresolvedReason::Other);
    unresolved("unresolved for another reason", &other_reason);
}

/// The same structural reconfirmation over a real store: a registered
/// cooperative binding, a host invalidation, then a production-shaped
/// publication (occupant None, occupancy/UI/execution Unknown).
/// Kills: the store's `ReconfirmStructure` prepare accepting a binding
/// whose stored evidence names another incarnation (forged transition below
/// must be Stale), and the apply arm not restoring the seat.
#[test]
fn real_store_reconfirms_host_invalidated_seat_from_production_shaped_publication() {
    use crate::ports::{
        DurableWorkAdmission, GuardedInvalidationTransition, HostInvalidationReason,
        ReconciliationAction, ReconciliationOutcome, SnapshotHeader,
    };
    use crate::protocol::time::{CallBudget, Cancellation, Clock};
    use crate::store::{connection::StoreContext, seats};
    use std::sync::Arc;

    struct FixedClock;
    impl Clock for FixedClock {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(100)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(100)
        }
    }
    fn production(sequence: u64) -> HostSnapshot {
        let mut observation = target("pane", "terminal-a", None, sequence);
        observation.provenance = ObservationProvenance::CoherentEnumeration;
        observation.occupant = None;
        observation.ui = HostUiState::Unknown;
        observation.occupancy = StructuralOccupancy::Unknown;
        observation.execution = ExecutionEvidence::Unknown;
        snapshot(sequence, vec![observation])
    }
    fn publish(
        context: &StoreContext,
        conn: &mut rusqlite::Connection,
        budget: &CallBudget,
        capture: HostSnapshot,
    ) -> crate::ports::SnapshotGenerationId {
        let admission = seats::begin_host_observation(context, conn, "i", budget).unwrap();
        let header = SnapshotHeader::from_captured(admission, &capture).unwrap();
        let stage = seats::begin_snapshot_stage(context, conn, header, budget).unwrap();
        seats::stage_snapshot_targets(
            context,
            conn,
            &stage.id,
            0,
            &capture.targets,
            DurableWorkAdmission::new(16).unwrap(),
            budget,
        )
        .unwrap();
        seats::seal_snapshot_stage(context, conn, &stage.id, budget).unwrap();
        seats::publish_snapshot_stage(context, conn, &stage.id, budget).unwrap();
        stage.id
    }
    for binding_incarnation in ["inc-a", "inc-old"] {
        let path =
            std::env::temp_dir().join(format!("herdr-reconcile-{}.db", uuid::Uuid::new_v4()));
        let context = StoreContext::new(path.clone(), Arc::new(FixedClock));
        let mut conn = context.open_writer().unwrap();
        let budget = CallBudget {
            deadline: MonoInstant(1_000),
            cancellation: Cancellation::default(),
        };
        conn.execute(
            "INSERT INTO host_instances(id,created_at,host_boot,host_epoch) VALUES ('i',0,'boot-a',1)",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('s','i','resolved','native','pane',1,1,0)", []).unwrap();
        conn.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance) VALUES ('i','pane','boot-a',1,1,0,'fresh')", []).unwrap();
        conn.execute(
            "INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES ('s',1,1,'pane','boot-a',1,'codex','plugin_context:x','e','cooperative_top_level',1,1,'terminal-a',?1)",
            [binding_incarnation],
        )
        .unwrap();
        publish(&context, &mut conn, &budget, production(2));
        let admission = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
        let fence = seats::invalidate_host_observation(
            &context,
            &mut conn,
            &admission,
            HostInvalidationReason::HostUnavailable,
            &budget,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            seats::mark_unresolved_from_invalidation(
                &context,
                &mut conn,
                GuardedInvalidationTransition {
                    fence,
                    seat: SeatId::new("s"),
                    expected_binding_generation: 1,
                    expected_target: Some(HostTargetId::new("pane")),
                    expected_terminal: Some(TerminalId::new("terminal-a")),
                },
                &budget
            )
            .unwrap(),
            ReconciliationOutcome::Applied
        );
        let current = publish(&context, &mut conn, &budget, production(3));
        let page =
            seats::saved_seats_page(&context, &conn, &current, 0, None, 16, &budget).unwrap();
        let actions = plan_page(&page).unwrap();
        let state = |conn: &rusqlite::Connection| -> (String, i64) {
            conn.query_row("SELECT state,generation FROM seats WHERE id='s'", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap()
        };
        if binding_incarnation == "inc-a" {
            assert_eq!(
                actions[0].action,
                ReconciliationAction::ReconfirmStructure {
                    target: HostTargetId::new("pane"),
                    terminal: TerminalId::new("terminal-a"),
                }
            );
            assert_eq!(
                seats::apply_reconciliation_transition(
                    &context,
                    &mut conn,
                    actions[0].clone(),
                    &budget
                )
                .unwrap(),
                ReconciliationOutcome::Applied
            );
            // Seat restored at the invalidation's generation; the ended
            // registration is not revived.
            assert_eq!(state(&conn), ("resolved".into(), 2));
            let live: i64 = conn
                .query_row(
                    "SELECT count(*) FROM occupant_bindings WHERE seat_id='s' AND ended_at IS NULL",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(live, 0);
            assert_eq!(
                seats::apply_reconciliation_transition(
                    &context,
                    &mut conn,
                    actions[0].clone(),
                    &budget
                )
                .unwrap(),
                ReconciliationOutcome::Stale
            );
        } else {
            assert_eq!(actions[0].action, ReconciliationAction::MarkUnresolved);
            let mut forged = actions[0].clone();
            forged.action = ReconciliationAction::ReconfirmStructure {
                target: HostTargetId::new("pane"),
                terminal: TerminalId::new("terminal-a"),
            };
            assert_eq!(
                seats::apply_reconciliation_transition(&context, &mut conn, forged, &budget)
                    .unwrap(),
                ReconciliationOutcome::Stale
            );
            assert_eq!(state(&conn), ("unresolved".into(), 2));
        }
        drop(conn);
        std::fs::remove_file(path).unwrap();
    }
}

/// Wave-2 fix2 (b): a seat resolved before its agent started and never
/// registered has no binding row, so no binding evidence. Its own verified
/// structural proof bridges it under the same rule (same terminal, host boot
/// and verified incarnation; qualified target), with the production
/// adapter's shape (occupancy/execution Unknown).
/// Kills: the planner not consulting the structural proof for a seat with no
/// binding (fix2 review S1: the seat then stays unresolved forever on the
/// real adapter), and each of the proof's terminal, boot, incarnation, epoch
/// and qualified-target conditions, and the bridge accepting a seat that has
/// any binding row.
#[test]
fn never_registered_host_invalidated_seat_reconfirms_from_its_structural_proof() {
    use crate::ports::{
        BindingEvidence, DurableStructuralProof, PublishedSnapshot, ReconciliationAction,
        SeatState, SnapshotGenerationId, SnapshotSavedSeat, SnapshotSeatPage, SnapshotTargetMatch,
        UnresolvedReason,
    };
    let proof = |terminal: &str, incarnation: &str, boot: &str, epoch: u64, sequence: u64| {
        DurableStructuralProof::from_persisted(
            HostTargetId::new("pane"),
            TerminalId::new(terminal),
            incarnation.into(),
            EvidenceKind::CoherentEnumeration,
            HostBootId::new(boot),
            epoch,
            1,
            sequence,
            1,
        )
        .unwrap()
    };
    let page = SnapshotSeatPage {
        publication: PublishedSnapshot {
            id: SnapshotGenerationId::store_issued("publication".into()),
            instance: "instance".into(),
            boot: HostBootId::new("boot-a"),
            epoch: 2,
            observation_sequence: 7,
            incarnation: "inc-a".into(),
            target_count: 1,
            invalidation_revision: 2,
        },
        high_water_ordinal: 1,
        after_ordinal: 0,
        visited: 1,
        has_more: false,
        seats: vec![SnapshotSavedSeat {
            ordinal: 1,
            seat: SeatId::new("seat-u"),
            state: SeatState::Unresolved,
            unresolved_reason: Some(UnresolvedReason::HostInvalidation),
            prior_published_observation: None,
            structural_proof: Some(proof("terminal-u", "inc-a", "boot-a", 1, 3)),
            target: Some(HostTargetId::new("pane")),
            terminal: Some(TerminalId::new("terminal-u")),
            binding_generation: 2,
            binding_execution: None,
            active_binding_execution: None,
            bound_boot: Some(HostBootId::new("boot-a")),
            bound_incarnation: Some("inc-a".into()),
            latest_binding_evidence: None,
            observed_match: Some(SnapshotTargetMatch {
                target: HostTargetId::new("pane"),
                terminal: Some(TerminalId::new("terminal-u")),
                structural_generation: 1,
                observation_sequence: 7,
                connection_epoch: Some(1),
                incarnation_source: Some(EvidenceKind::CoherentEnumeration),
                occupancy: StructuralOccupancy::Unknown,
                verified_execution: None,
                top_level_occupant: false,
            }),
        }],
    };
    let action = |page: &SnapshotSeatPage| plan_page(page).unwrap()[0].action.clone();
    assert_eq!(
        action(&page),
        ReconciliationAction::ReconfirmStructure {
            target: HostTargetId::new("pane"),
            terminal: TerminalId::new("terminal-u"),
        }
    );
    // Same epoch: the proof must not be newer than the publication.
    let mut same_epoch = page.clone();
    same_epoch.seats[0].structural_proof = Some(proof("terminal-u", "inc-a", "boot-a", 2, 7));
    assert!(matches!(
        action(&same_epoch),
        ReconciliationAction::ReconfirmStructure { .. }
    ));
    let unresolved = |label: &str, page: &SnapshotSeatPage| {
        assert_eq!(
            action(page),
            ReconciliationAction::MarkUnresolved,
            "{label} must stay fail-closed"
        );
    };
    let mut newer = page.clone();
    newer.seats[0].structural_proof = Some(proof("terminal-u", "inc-a", "boot-a", 2, 8));
    unresolved("proof newer than the publication", &newer);
    let mut later_epoch = page.clone();
    later_epoch.seats[0].structural_proof = Some(proof("terminal-u", "inc-a", "boot-a", 3, 1));
    unresolved("proof from a later host epoch", &later_epoch);
    // The bridge's own incarnation and boot checks (bound_* agree with the
    // publication here, so only the bridge can refuse).
    let mut other_incarnation = page.clone();
    other_incarnation.seats[0].structural_proof =
        Some(proof("terminal-u", "inc-old", "boot-a", 1, 3));
    unresolved("proof from another incarnation", &other_incarnation);
    let mut other_boot = page.clone();
    other_boot.seats[0].structural_proof = Some(proof("terminal-u", "inc-a", "boot-old", 1, 3));
    unresolved("proof from another host boot", &other_boot);
    let mut no_proof = page.clone();
    no_proof.seats[0].structural_proof = None;
    unresolved("seat without a structural proof", &no_proof);
    let mut unqualified = page.clone();
    unqualified.seats[0]
        .observed_match
        .as_mut()
        .unwrap()
        .connection_epoch = None;
    unresolved("target without qualified incarnation", &unqualified);
    let mut unsourced = page.clone();
    unsourced.seats[0]
        .observed_match
        .as_mut()
        .unwrap()
        .incarnation_source = None;
    unresolved("target without incarnation source", &unsourced);
    // Any binding row, even an ended legacy one without evidence, keeps the
    // binding-evidence rule: the structural proof never stands in for it.
    let mut legacy = page.clone();
    legacy.seats[0].binding_execution = Some(ExecutionId::new("execution-legacy"));
    unresolved("seat with a binding lacking evidence", &legacy);
    let mut mismatched = page.clone();
    mismatched.seats[0].binding_execution = Some(ExecutionId::new("execution-old"));
    mismatched.seats[0].latest_binding_evidence = Some(BindingEvidence {
        terminal: TerminalId::new("terminal-u"),
        host_boot: HostBootId::new("boot-a"),
        incarnation: "inc-old".into(),
    });
    unresolved("seat whose binding evidence mismatches", &mismatched);
    let mut other_reason = page.clone();
    other_reason.seats[0].unresolved_reason = Some(UnresolvedReason::Other);
    unresolved("unresolved for another reason", &other_reason);
}

/// The never-registered bridge over a real store: a seat allocated with a
/// verified structural proof and no binding, a host invalidation, then a
/// production-shaped publication.
/// Kills: the store's `ReconfirmStructure` prepare accepting a seat with no
/// binding without re-checking its structural proof (the forged transition
/// for a proof from another incarnation must be Stale), and the apply arm
/// not restoring the seat.
#[test]
fn real_store_reconfirms_never_registered_seat_from_its_structural_proof() {
    use crate::ports::{
        DurableWorkAdmission, GuardedInvalidationTransition, HostInvalidationReason,
        ReconciliationAction, ReconciliationOutcome, SnapshotHeader,
    };
    use crate::protocol::time::{CallBudget, Cancellation, Clock};
    use crate::store::{connection::StoreContext, seats};
    use std::sync::Arc;

    struct FixedClock;
    impl Clock for FixedClock {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(100)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(100)
        }
    }
    fn production(sequence: u64) -> HostSnapshot {
        let mut observation = target("pane", "terminal-u", None, sequence);
        observation.provenance = ObservationProvenance::CoherentEnumeration;
        observation.occupant = None;
        observation.ui = HostUiState::Unknown;
        observation.occupancy = StructuralOccupancy::Unknown;
        observation.execution = ExecutionEvidence::Unknown;
        snapshot(sequence, vec![observation])
    }
    fn publish(
        context: &StoreContext,
        conn: &mut rusqlite::Connection,
        budget: &CallBudget,
        capture: HostSnapshot,
    ) -> crate::ports::SnapshotGenerationId {
        let admission = seats::begin_host_observation(context, conn, "i", budget).unwrap();
        let header = SnapshotHeader::from_captured(admission, &capture).unwrap();
        let stage = seats::begin_snapshot_stage(context, conn, header, budget).unwrap();
        seats::stage_snapshot_targets(
            context,
            conn,
            &stage.id,
            0,
            &capture.targets,
            DurableWorkAdmission::new(16).unwrap(),
            budget,
        )
        .unwrap();
        seats::seal_snapshot_stage(context, conn, &stage.id, budget).unwrap();
        seats::publish_snapshot_stage(context, conn, &stage.id, budget).unwrap();
        stage.id
    }
    for proof_incarnation in ["inc-a", "inc-old"] {
        let path =
            std::env::temp_dir().join(format!("herdr-reconcile-{}.db", uuid::Uuid::new_v4()));
        let context = StoreContext::new(path.clone(), Arc::new(FixedClock));
        let mut conn = context.open_writer().unwrap();
        let budget = CallBudget {
            deadline: MonoInstant(1_000),
            cancellation: Cancellation::default(),
        };
        conn.execute(
            "INSERT INTO host_instances(id,created_at,host_boot,host_epoch) VALUES ('i',0,'boot-a',1)",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,structural_terminal_id,structural_incarnation,structural_incarnation_kind,structural_host_boot,structural_host_epoch,structural_connection_epoch,structural_observation_sequence) VALUES ('u','i','resolved','native','pane',1,1,0,'terminal-u',?1,'coherent_enumeration','boot-a',1,1,1)", [proof_incarnation]).unwrap();
        conn.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance) VALUES ('i','pane','boot-a',1,1,0,'fresh')", []).unwrap();
        publish(&context, &mut conn, &budget, production(2));
        let admission = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
        let fence = seats::invalidate_host_observation(
            &context,
            &mut conn,
            &admission,
            HostInvalidationReason::HostUnavailable,
            &budget,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            seats::mark_unresolved_from_invalidation(
                &context,
                &mut conn,
                GuardedInvalidationTransition {
                    fence,
                    seat: SeatId::new("u"),
                    expected_binding_generation: 1,
                    expected_target: Some(HostTargetId::new("pane")),
                    expected_terminal: Some(TerminalId::new("terminal-u")),
                },
                &budget
            )
            .unwrap(),
            ReconciliationOutcome::Applied
        );
        let current = publish(&context, &mut conn, &budget, production(3));
        let page =
            seats::saved_seats_page(&context, &conn, &current, 0, None, 16, &budget).unwrap();
        assert_eq!(page.seats[0].binding_execution, None, "never registered");
        let actions = plan_page(&page).unwrap();
        let state = |conn: &rusqlite::Connection| -> (String, i64) {
            conn.query_row("SELECT state,generation FROM seats WHERE id='u'", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap()
        };
        let reconfirm = ReconciliationAction::ReconfirmStructure {
            target: HostTargetId::new("pane"),
            terminal: TerminalId::new("terminal-u"),
        };
        if proof_incarnation == "inc-a" {
            assert_eq!(actions[0].action, reconfirm);
            assert_eq!(
                seats::apply_reconciliation_transition(
                    &context,
                    &mut conn,
                    actions[0].clone(),
                    &budget
                )
                .unwrap(),
                ReconciliationOutcome::Applied
            );
            assert_eq!(state(&conn), ("resolved".into(), 2));
            let bindings: i64 = conn
                .query_row(
                    "SELECT count(*) FROM occupant_bindings WHERE seat_id='u'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(bindings, 0, "reconfirmation registers nothing");
            assert_eq!(
                seats::apply_reconciliation_transition(
                    &context,
                    &mut conn,
                    actions[0].clone(),
                    &budget
                )
                .unwrap(),
                ReconciliationOutcome::Stale
            );
        } else {
            assert_eq!(actions[0].action, ReconciliationAction::MarkUnresolved);
            let mut forged = actions[0].clone();
            forged.action = reconfirm;
            assert_eq!(
                seats::apply_reconciliation_transition(&context, &mut conn, forged, &budget)
                    .unwrap(),
                ReconciliationOutcome::Stale
            );
            assert_eq!(state(&conn), ("unresolved".into(), 2));
        }
        drop(conn);
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn real_invalidation_page_reconfirms_a_prelaunch_shell_once() {
    use crate::ports::{
        DurableWorkAdmission, GuardedInvalidationTransition, HostInvalidationReason,
        ReconciliationAction, ReconciliationOutcome, SnapshotHeader,
    };
    use crate::protocol::time::{CallBudget, Cancellation, Clock};
    use crate::store::{connection::StoreContext, seats};
    use std::sync::Arc;

    struct FixedClock;
    impl Clock for FixedClock {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(100)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(100)
        }
    }

    let path = std::env::temp_dir().join(format!("herdr-reconcile-{}.db", uuid::Uuid::new_v4()));
    let context = StoreContext::new(path.clone(), Arc::new(FixedClock));
    let mut conn = context.open_writer().unwrap();
    conn.execute(
        "INSERT INTO host_instances(id,created_at,host_boot,host_epoch) VALUES ('i',0,'boot-a',1)",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('s','i','resolved','native','old',1,1,0)", []).unwrap();
    conn.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance) VALUES ('i','old','boot-a',1,1,0,'fresh')", []).unwrap();
    let budget = CallBudget {
        deadline: MonoInstant(1_000),
        cancellation: Cancellation::default(),
    };
    let shell = |sequence, address: &str| {
        let mut observation = target(address, "terminal-a", None, sequence);
        observation.occupancy = StructuralOccupancy::EmptyShell;
        snapshot(sequence, vec![observation])
    };
    fn publish(
        context: &StoreContext,
        conn: &mut rusqlite::Connection,
        budget: &CallBudget,
        capture: HostSnapshot,
    ) -> (
        crate::ports::SnapshotGenerationId,
        crate::ports::PublishedSnapshot,
    ) {
        let admission = seats::begin_host_observation(context, conn, "i", budget).unwrap();
        let header = SnapshotHeader::from_captured(admission, &capture).unwrap();
        let stage = seats::begin_snapshot_stage(context, conn, header, budget).unwrap();
        seats::stage_snapshot_targets(
            context,
            conn,
            &stage.id,
            0,
            &capture.targets,
            DurableWorkAdmission::new(16).unwrap(),
            budget,
        )
        .unwrap();
        seats::seal_snapshot_stage(context, conn, &stage.id, budget).unwrap();
        let published = seats::publish_snapshot_stage(context, conn, &stage.id, budget).unwrap();
        (stage.id, published)
    }
    let (prior, _) = publish(&context, &mut conn, &budget, shell(2, "old"));
    let admission = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let fence = seats::invalidate_host_observation(
        &context,
        &mut conn,
        &admission,
        HostInvalidationReason::HostUnavailable,
        &budget,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        seats::mark_unresolved_from_invalidation(
            &context,
            &mut conn,
            GuardedInvalidationTransition {
                fence,
                seat: SeatId::new("s"),
                expected_binding_generation: 1,
                expected_target: Some(HostTargetId::new("old")),
                expected_terminal: None,
            },
            &budget
        )
        .unwrap(),
        ReconciliationOutcome::Applied
    );
    let (current, _) = publish(&context, &mut conn, &budget, shell(3, "new"));
    let page = seats::saved_seats_page(&context, &conn, &current, 0, None, 16, &budget).unwrap();
    assert_eq!(
        page.seats[0]
            .prior_published_observation
            .as_ref()
            .unwrap()
            .generation_id,
        prior
    );
    let actions = plan_page(&page).unwrap();
    assert!(
        matches!(&actions[0].action, ReconciliationAction::Reconfirm {
        target, verified_execution: None, ..
    } if target.as_str() == "new")
    );
    assert_eq!(
        seats::apply_reconciliation_transition(&context, &mut conn, actions[0].clone(), &budget)
            .unwrap(),
        ReconciliationOutcome::Applied
    );
    assert_eq!(
        seats::apply_reconciliation_transition(&context, &mut conn, actions[0].clone(), &budget)
            .unwrap(),
        ReconciliationOutcome::Stale
    );
    let actual: (String, String, i64) = conn
        .query_row(
            "SELECT state,target_id,generation FROM seats WHERE id='s'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(actual, ("resolved".into(), "new".into(), 2));
    let active: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM occupant_bindings WHERE seat_id='s' AND ended_at IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(active, 0);
    drop(conn);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn ordinary_empty_allocation_survives_a_later_coherent_publication() {
    ordinary_allocation_lifecycle(None);
}

#[test]
fn registered_occupant_loss_crosses_real_saved_page_and_planner() {
    for address in ["pane-b", "pane-c"] {
        ordinary_allocation_lifecycle(Some(address));
    }
}

fn ordinary_allocation_lifecycle(loss_address: Option<&str>) {
    use crate::ports::{
        DurableWorkAdmission, OrdinaryAllocationGuard, RecoveryBaseline, RecoveryDisposition,
        SnapshotHeader,
    };
    use crate::protocol::{
        commands::ResolveSeat,
        ids::OperationId,
        time::{CallBudget, Cancellation, Clock},
    };
    use crate::store::{connection::StoreContext, seats};
    use std::sync::Arc;

    struct FixedClock;
    impl Clock for FixedClock {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(100)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(100)
        }
    }

    let path =
        std::env::temp_dir().join(format!("herdr-ordinary-shell-{}.db", uuid::Uuid::new_v4()));
    let context = StoreContext::new(path.clone(), Arc::new(FixedClock));
    let mut conn = context.open_writer().unwrap();
    conn.execute(
        "INSERT INTO host_instances(id,created_at,host_boot,host_epoch) VALUES ('i',0,'boot-a',1)",
        [],
    )
    .unwrap();
    let budget = CallBudget {
        deadline: MonoInstant(1_000),
        cancellation: Cancellation::default(),
    };
    let shell = |sequence| {
        let mut observation = target("pane-a", "terminal-a", None, sequence);
        observation.occupancy = StructuralOccupancy::EmptyShell;
        observation
    };
    fn publish(
        context: &StoreContext,
        conn: &mut rusqlite::Connection,
        budget: &CallBudget,
        capture: HostSnapshot,
    ) -> crate::ports::PublishedSnapshot {
        let admission = seats::begin_host_observation(context, conn, "i", budget).unwrap();
        let header = SnapshotHeader::from_captured(admission, &capture).unwrap();
        let stage = seats::begin_snapshot_stage(context, conn, header, budget).unwrap();
        if !capture.targets.is_empty() {
            seats::stage_snapshot_targets(
                context,
                conn,
                &stage.id,
                0,
                &capture.targets,
                DurableWorkAdmission::new(16).unwrap(),
                budget,
            )
            .unwrap();
        }
        seats::seal_snapshot_stage(context, conn, &stage.id, budget).unwrap();
        seats::publish_snapshot_stage(context, conn, &stage.id, budget).unwrap()
    }

    publish(&context, &mut conn, &budget, snapshot(2, vec![shell(2)]));
    let fresh = shell(3);
    let admission = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    assert!(
        seats::publish_current_target_observation(&context, &mut conn, &admission, &fresh, &budget)
            .unwrap()
    );
    let request = ResolveSeat {
        target: fresh.target.clone(),
        operation: OperationId::new("allocate-shell"),
    };
    let baseline = RecoveryBaseline::new(
        HostBootId::new("boot-a"),
        1,
        fresh.target.clone(),
        RecoveryDisposition::UnambiguousUnclaimed,
    );
    let guard =
        OrdinaryAllocationGuard::try_new(&request, fresh.clone(), baseline.clone()).unwrap();
    let seat = seats::allocate(&context, &mut conn, "i", request.clone(), guard).unwrap();
    let replay_guard =
        OrdinaryAllocationGuard::try_new(&request, fresh.clone(), baseline.clone()).unwrap();
    assert_eq!(
        seats::allocate(&context, &mut conn, "i", request.clone(), replay_guard).unwrap(),
        seat
    );
    conn.execute(
        "INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('handoff','i','handoff','goal',100,100)",
        [],
    )
    .unwrap();
    let operator = crate::protocol::authority::OperatorActor::from_peer(
        crate::protocol::authority::PeerIdentity::from_kernel(501),
        501,
    )
    .unwrap();
    let invitation = crate::protocol::commands::OperatorOrphanInvite {
        thread: crate::protocol::ids::ThreadId::new("handoff"),
        seat: seat.clone(),
        deadline_millis: Some(300),
        operation: OperationId::new("invite-shell"),
    };
    assert!(matches!(
        crate::store::control::operator_orphan_invite(
            &context,
            &mut conn,
            "i",
            &invitation,
            operator,
            None
        )
        .unwrap(),
        crate::protocol::results::CommandResult::OperatorInvited(_)
    ));
    let later = publish(&context, &mut conn, &budget, snapshot(4, vec![shell(4)]));
    let page = seats::saved_seats_page(&context, &conn, &later.id, 0, None, 16, &budget).unwrap();
    assert_eq!(page.seats.len(), 1);
    assert_eq!(page.seats[0].seat, seat);
    let durable = page.seats[0].structural_proof.as_ref().unwrap();
    assert_eq!(durable.target().as_str(), "pane-a");
    assert_eq!(durable.terminal().as_str(), "terminal-a");
    assert_eq!(durable.incarnation(), "inc-a");
    assert_eq!(durable.observation_sequence(), 3);
    assert!(page.seats[0].active_binding_execution.is_none());
    assert!(
        plan_page(&page).unwrap().is_empty(),
        "the live empty shell must retain its resolved seat"
    );
    let saved: (String, i64) = conn
        .query_row(
            "SELECT state,generation FROM seats WHERE id=?1",
            [seat.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(saved, ("resolved".into(), 1));
    drop(conn);
    let mut conn = context.open_writer().unwrap();
    let replay_guard = OrdinaryAllocationGuard::try_new(&request, fresh, baseline).unwrap();
    assert_eq!(
        seats::allocate(&context, &mut conn, "i", request, replay_guard).unwrap(),
        seat
    );
    let repeated = publish(&context, &mut conn, &budget, snapshot(5, vec![shell(5)]));
    let reopened_page =
        seats::saved_seats_page(&context, &conn, &repeated.id, 0, None, 16, &budget).unwrap();
    assert!(plan_page(&reopened_page).unwrap().is_empty());
    let bindings: i64 = conn
        .query_row(
            "SELECT count(*) FROM occupant_bindings WHERE seat_id=?1",
            [seat.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        bindings, 0,
        "allocation replay must not invent occupant history"
    );
    let pending: i64 = conn
        .query_row(
            "SELECT count(*) FROM invitations WHERE seat_id=?1 AND state='pending'",
            [seat.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pending, 1, "reopen retains the prelaunch handoff");
    let mut moved_shell = target("pane-b", "terminal-a", None, 6);
    moved_shell.occupancy = StructuralOccupancy::EmptyShell;
    let moved = publish(&context, &mut conn, &budget, snapshot(6, vec![moved_shell]));
    let moved_page =
        seats::saved_seats_page(&context, &conn, &moved.id, 0, None, 16, &budget).unwrap();
    let move_action = plan_page(&moved_page).unwrap();
    assert!(
        matches!(&move_action[0].action, ReconciliationAction::Move { target, .. } if target.as_str() == "pane-b")
    );
    assert_eq!(
        seats::apply_reconciliation_transition(
            &context,
            &mut conn,
            move_action[0].clone(),
            &budget
        )
        .unwrap(),
        ReconciliationOutcome::Applied
    );
    assert_eq!(
        seats::apply_reconciliation_transition(
            &context,
            &mut conn,
            move_action[0].clone(),
            &budget
        )
        .unwrap(),
        ReconciliationOutcome::Stale
    );
    let moved_saved: (String, i64) = conn
        .query_row(
            "SELECT target_id,generation FROM seats WHERE id=?1",
            [seat.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(moved_saved, ("pane-b".into(), 1));
    let pending_after_move: i64 = conn
        .query_row(
            "SELECT count(*) FROM invitations WHERE seat_id=?1 AND state='pending'",
            [seat.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pending_after_move, 1);

    let mut final_target = "pane-b";
    if let Some(loss_address) = loss_address {
        use crate::protocol::{
            authority::{
                CallerClaim, DecisionFence, MutationPermit, ObligationRef, ReceiptRegistration,
                VerifiedCaller,
            },
            commands::CheckIn,
            pagination::{Consistency, Page, StopReason},
            results::CheckInResult,
        };
        let native = target("pane-b", "terminal-a", Some("execution-a"), 7);
        let admission = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
        assert!(
            seats::publish_current_target_observation(
                &context, &mut conn, &admission, &native, &budget
            )
            .unwrap()
        );
        let command = CheckIn {
            mode: crate::protocol::commands::CheckInMode::Current,
            claim: CallerClaim {
                instance: String::new(),
                seat: SeatId::new("legacy-fixture"),
                binding_generation: 0,
                role: crate::protocol::authority::CallerRole::TopLevel,
                harness: Harness::Codex,
                native_session: NativeSessionId::new("conversation"),
                execution: ExecutionId::new("execution-a"),
                target: native.target.clone(),
            },
            operation: OperationId::new("register-a"),
        };
        // This internal authority fixture exercises the production registration
        // transaction; native attribution remains its independent validation gate.
        let permit = MutationPermit::new(
            VerifiedCaller {
                seat: seat.clone(),
                harness: Harness::Codex,
                native_session: command.claim.native_session.clone(),
                execution: command.claim.execution.clone(),
                host_boot: native.host_boot.clone(),
                target_generation: 1,
                binding_generation: 1,
                observed_at_utc: UtcMillis(100),
            },
            command.operation.clone(),
            ObligationRef::CheckIn(seat.clone()),
            crate::store::schema::canonical_digest(&crate::store::seats::native_check_in_payload(
                &command,
            ))
            .unwrap(),
            MonoInstant(100),
            1,
        );
        let registration = ReceiptRegistration {
            seat: seat.clone(),
            host_boot: native.host_boot.clone(),
            target_generation: 1,
            binding_generation: 1,
            native_session: command.claim.native_session.clone(),
            execution: command.claim.execution.clone(),
        };
        assert!(matches!(
            seats::register_available(
                &context,
                &mut conn,
                &command,
                Some(&registration),
                &crate::protocol::time::CallBudget {
                    deadline: crate::protocol::time::MonoInstant(u64::MAX),
                    cancellation: Default::default()
                },
                permit,
                |_, at| Ok(DecisionFence {
                    now: at.monotonic,
                    host_boot: HostBootId::new("boot-a"),
                    host_epoch: 1,
                    target_generation: 1,
                    binding_generation: 1,
                    known_invalidated: false,
                }),
                |_, seat, sequence| Ok(CheckInResult {
                    context_disposition:
                        crate::protocol::results::CheckInContextDisposition::Current,
                    context: command.claim.clone(),
                    seat: seat.clone(),
                    offered_through: Some(sequence.to_string()),
                    warning_count: 0,
                    warning_count_has_more: false,
                    warnings: Page {
                        items: vec![],
                        next_cursor: None,
                        next_argv: None,
                        high_water_ordinal: 0,
                        scope_revision: None,
                        has_more: false,
                        stop_reason: StopReason::Complete,
                        consistency: Consistency::BoundedLive
                    },
                    notices: Default::default(),
                    inbox: Page {
                        items: vec![],
                        next_cursor: None,
                        next_argv: None,
                        high_water_ordinal: 0,
                        scope_revision: None,
                        has_more: false,
                        stop_reason: StopReason::Complete,
                        consistency: Consistency::BoundedLive
                    },
                }),
            )
            .unwrap(),
            crate::protocol::results::CommandResult::CheckedIn(_)
        ));
        let anchor: (i64, i64, i64) = conn.query_row(
            "SELECT ordinal,decision_seq,binding_generation FROM seat_availability WHERE seat_id=?1",
            [seat.as_str()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))
        ).unwrap();
        assert_eq!(anchor.2, 1);
        // native-claude-demo-1 P2: a production-adapter-shaped observation
        // stream (occupant None, UI/occupancy/execution Unknown, same pane and
        // terminal in the verified incarnation) over many ~5 s snapshots
        // plans nothing, and a forged unavailability transition against such
        // a publication is refused by the store guard. The registered
        // binding, its availability and the seat's episode stay untouched.
        // Kills: planner `occupancy != Occupied || execution unverified` ⇒
        // MarkOccupantUnavailable, and the store guard admitting it for
        // Unknown occupancy.
        let seat_episode = |conn: &rusqlite::Connection| -> (i64, i64) {
            conn.query_row(
                "SELECT unavailability_episode,unavailability_open FROM seats WHERE id=?1",
                [seat.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
        };
        let live_binding = |conn: &rusqlite::Connection| -> (i64, i64) {
            conn.query_row(
                "SELECT count(*),sum(registered_at IS NOT NULL) FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL",
                [seat.as_str()],
                |r| Ok((r.get(0)?, r.get::<_, Option<i64>>(1)?.unwrap_or(0))),
            )
            .unwrap()
        };
        let episode_before = seat_episode(&conn);
        assert_eq!(live_binding(&conn), (1, 1));
        let production = |sequence| {
            let mut observation = target("pane-b", "terminal-a", None, sequence);
            observation.occupant = None;
            observation.ui = HostUiState::Unknown;
            observation.occupancy = StructuralOccupancy::Unknown;
            observation.execution = ExecutionEvidence::Unknown;
            observation
        };
        let mut last_unknown = None;
        for sequence in 8..24 {
            let published = publish(
                &context,
                &mut conn,
                &budget,
                snapshot(sequence, vec![production(sequence)]),
            );
            let unknown_page =
                seats::saved_seats_page(&context, &conn, &published.id, 0, None, 16, &budget)
                    .unwrap();
            assert_eq!(
                unknown_page.seats[0]
                    .active_binding_execution
                    .as_ref()
                    .map(|e| e.as_str()),
                Some("execution-a")
            );
            assert!(
                plan_page(&unknown_page).unwrap().is_empty(),
                "unknown snapshot {sequence} must not unseat the occupant"
            );
            last_unknown = Some(published);
        }
        let forged = GuardedSeatTransition {
            publication: last_unknown.unwrap(),
            seat: seat.clone(),
            expected_binding_generation: 1,
            expected_target: Some(HostTargetId::new("pane-b")),
            expected_terminal: Some(TerminalId::new("terminal-a")),
            action: ReconciliationAction::MarkOccupantUnavailable {
                target: HostTargetId::new("pane-b"),
                terminal: TerminalId::new("terminal-a"),
                expected_execution: ExecutionId::new("execution-a"),
            },
        };
        assert_ne!(
            seats::apply_reconciliation_transition(&context, &mut conn, forged, &budget).unwrap(),
            ReconciliationOutcome::Applied,
            "store guard admitted unavailability on Unknown occupancy"
        );
        assert_eq!(
            seat_episode(&conn),
            episode_before,
            "no unavailability episode opened"
        );
        assert_eq!(
            live_binding(&conn),
            (1, 1),
            "registered_at kept, binding live"
        );

        // Loss needs positive evidence (native-claude-demo-1 P2): an observed
        // empty shell (moved to pane-c) or an observed non-top-level occupant
        // on the same terminal (pane-b). Unknown occupancy is not loss.
        let mut loss = target(loss_address, "terminal-a", None, 108);
        loss.occupancy = StructuralOccupancy::EmptyShell;
        if loss_address == "pane-b" {
            loss.occupancy = StructuralOccupancy::Occupied;
            loss.occupant = native.occupant.clone().map(|mut occupant| {
                occupant.is_top_level = false;
                occupant
            });
        }
        let publication = publish(&context, &mut conn, &budget, snapshot(108, vec![loss]));
        let page = seats::saved_seats_page(&context, &conn, &publication.id, 0, None, 16, &budget)
            .unwrap();
        assert_eq!(
            page.seats[0]
                .active_binding_execution
                .as_ref()
                .unwrap()
                .as_str(),
            "execution-a"
        );
        let actions = plan_page(&page).unwrap();
        assert!(matches!(&actions[0].action,
            ReconciliationAction::MarkOccupantUnavailable { target, expected_execution, .. }
                if target.as_str() == loss_address && expected_execution.as_str() == "execution-a"));
        assert_eq!(
            seats::apply_reconciliation_transition(
                &context,
                &mut conn,
                actions[0].clone(),
                &budget
            )
            .unwrap(),
            ReconciliationOutcome::Applied
        );
        let after = seats::saved_seats_page(&context, &conn, &publication.id, 0, None, 16, &budget)
            .unwrap();
        assert_eq!(after.seats[0].state, crate::ports::SeatState::Resolved);
        assert_eq!(after.seats[0].binding_generation, 1);
        assert!(after.seats[0].active_binding_execution.is_none());
        assert_eq!(
            after.seats[0].binding_execution.as_ref().unwrap().as_str(),
            "execution-a",
            "saved history must remain separate from current authority"
        );
        assert!(plan_page(&after).unwrap().is_empty());
        assert_eq!(conn.query_row(
            "SELECT count(*),sum(ended_at IS NOT NULL),min(execution_id) FROM occupant_bindings WHERE seat_id=?1",
            [seat.as_str()], |r| Ok((r.get::<_, i64>(0)?,r.get::<_, i64>(1)?,r.get::<_, String>(2)?))
        ).unwrap(), (1,1,"execution-a".into()));
        assert_eq!(conn.query_row(
            "SELECT ordinal,decision_seq,binding_generation FROM seat_availability WHERE seat_id=?1",
            [seat.as_str()], |r| Ok((r.get::<_, i64>(0)?,r.get::<_, i64>(1)?,r.get::<_, i64>(2)?))
        ).unwrap(), anchor, "loss retains the original availability evidence");
        assert_eq!(
            seats::apply_reconciliation_transition(
                &context,
                &mut conn,
                actions[0].clone(),
                &budget
            )
            .unwrap(),
            ReconciliationOutcome::Stale
        );
        let mut unknown_again = target(loss_address, "terminal-a", None, 109);
        unknown_again.occupancy = StructuralOccupancy::Unknown;
        let repeated = publish(
            &context,
            &mut conn,
            &budget,
            snapshot(109, vec![unknown_again]),
        );
        let repeated_page =
            seats::saved_seats_page(&context, &conn, &repeated.id, 0, None, 16, &budget).unwrap();
        assert!(
            plan_page(&repeated_page).unwrap().is_empty(),
            "repeated uncertainty must not fabricate replacement or rotate generation"
        );
        let same_native = target(loss_address, "terminal-a", Some("execution-a"), 110);
        let known_again = publish(
            &context,
            &mut conn,
            &budget,
            snapshot(110, vec![same_native]),
        );
        let known_page =
            seats::saved_seats_page(&context, &conn, &known_again.id, 0, None, 16, &budget)
                .unwrap();
        assert!(
            plan_page(&known_page).unwrap().is_empty(),
            "same historical execution must not be mistaken for a successor"
        );
        assert!(
            known_page.seats[0].active_binding_execution.is_none(),
            "snapshot proof must not register native authority"
        );
        assert_eq!(conn.query_row(
            "SELECT ordinal,decision_seq,binding_generation FROM seat_availability WHERE seat_id=?1",
            [seat.as_str()], |r| Ok((r.get::<_, i64>(0)?,r.get::<_, i64>(1)?,r.get::<_, i64>(2)?))
        ).unwrap(), anchor, "same-execution reappearance must not restart the receipt anchor");
        let anchor_jobs: i64 = conn.query_row(
            "SELECT count(*) FROM work_jobs WHERE kind='receipt_timer_materialization' AND subject_id=?1",
            [anchor.0.to_string()], |r| r.get(0)
        ).unwrap();
        assert_eq!(
            anchor_jobs, 1,
            "reappearance creates no extra receipt timer work"
        );
        let accept = crate::protocol::commands::Accept {
            thread: invitation.thread.clone(),
            operation: OperationId::new("stale-accept"),
            claim: CallerClaim {
                target: HostTargetId::new(loss_address),
                ..command.claim.clone()
            },
        };
        let invitation_id: String = conn
            .query_row(
                "SELECT id FROM invitations WHERE seat_id=?1",
                [seat.as_str()],
                |r| r.get(0),
            )
            .unwrap();
        let stale_permit = MutationPermit::new(
            VerifiedCaller {
                seat: seat.clone(),
                harness: Harness::Codex,
                native_session: command.claim.native_session.clone(),
                execution: command.claim.execution.clone(),
                host_boot: HostBootId::new("boot-a"),
                target_generation: 1,
                binding_generation: 1,
                observed_at_utc: UtcMillis(100),
            },
            accept.operation.clone(),
            ObligationRef::Invitation(crate::protocol::ids::InvitationId::new(invitation_id)),
            crate::store::schema::canonical_digest(&("accept", &accept.thread)).unwrap(),
            MonoInstant(100),
            1,
        );
        let rejected = crate::store::control::accept(
            &context,
            &mut conn,
            &crate::protocol::time::CallBudget {
                deadline: crate::protocol::time::MonoInstant(u64::MAX),
                cancellation: Default::default(),
            },
            &accept,
            stale_permit,
            |_, at| {
                Ok(DecisionFence {
                    now: at.monotonic,
                    host_boot: HostBootId::new("boot-a"),
                    host_epoch: 1,
                    target_generation: 1,
                    binding_generation: 1,
                    known_invalidated: false,
                })
            },
        )
        .unwrap_err();
        assert_eq!(rejected.code, ErrorCode::CallerUnverified);
        let pending: i64 = conn
            .query_row(
                "SELECT count(*) FROM invitations WHERE seat_id=?1 AND state='pending'",
                [seat.as_str()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            pending, 1,
            "stale native authority cannot accept the invitation"
        );
        final_target = loss_address;
    }
    let closed = publish(&context, &mut conn, &budget, snapshot(111, vec![]));
    let closed_page =
        seats::saved_seats_page(&context, &conn, &closed.id, 0, None, 16, &budget).unwrap();
    let close_action = plan_page(&closed_page).unwrap();
    assert!(
        matches!(&close_action[0].action, ReconciliationAction::BeginRetirement { absent_target } if absent_target.as_str() == final_target)
    );
    assert!(matches!(
        seats::apply_reconciliation_transition(
            &context,
            &mut conn,
            close_action[0].clone(),
            &budget
        )
        .unwrap(),
        ReconciliationOutcome::RetirementStarted(_)
    ));
    let retired: (String, i64) = conn
        .query_row(
            "SELECT state,generation FROM seats WHERE id=?1",
            [seat.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(retired.0, "retired");
    let after_closure = publish(
        &context,
        &mut conn,
        &budget,
        snapshot(112, vec![shell(112)]),
    );
    let after_closure_page =
        seats::saved_seats_page(&context, &conn, &after_closure.id, 0, None, 16, &budget).unwrap();
    assert!(plan_page(&after_closure_page).unwrap().is_empty());
    drop(conn);
    std::fs::remove_file(path).unwrap();
}

/// D2 (host-recovery R08): a registered seat whose pane moves, observed by
/// the production adapter's shape (occupancy and execution Unknown), follows
/// the move structurally: same terminal in the same verified server
/// incarnation keeps the seat, its generation and its live registered
/// binding. The page publishes without CursorStale, so a later unregistered
/// seat on the same page follows its own move too.
/// Kills: the Move arm demanding a verified execution equal to the bound one
/// (the seat then keeps its dead address and the page aborts every turn),
/// and the unknown-execution move ignoring the bound terminal.
#[test]
fn real_store_registered_seat_follows_observed_move_with_unsupported_execution() {
    use crate::ports::{DurableWorkAdmission, SnapshotHeader};
    use crate::protocol::time::{CallBudget, Cancellation, Clock};
    use crate::store::{SqliteStore, StoreSettings, connection::StoreContext, seats};
    use std::sync::Arc;

    struct FixedClock;
    impl Clock for FixedClock {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(100)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(100)
        }
    }
    fn production(address: &str, terminal: &str, sequence: u64) -> HostObservation {
        let mut observation = target(address, terminal, None, sequence);
        observation.provenance = ObservationProvenance::CoherentEnumeration;
        observation.occupant = None;
        observation.ui = HostUiState::Unknown;
        observation.occupancy = StructuralOccupancy::Unknown;
        observation.execution = ExecutionEvidence::Unknown;
        observation
    }
    fn publish(
        context: &StoreContext,
        conn: &mut rusqlite::Connection,
        budget: &CallBudget,
        capture: HostSnapshot,
    ) -> crate::ports::PublishedSnapshot {
        let admission = seats::begin_host_observation(context, conn, "i", budget).unwrap();
        let header = SnapshotHeader::from_captured(admission, &capture).unwrap();
        let stage = seats::begin_snapshot_stage(context, conn, header, budget).unwrap();
        seats::stage_snapshot_targets(
            context,
            conn,
            &stage.id,
            0,
            &capture.targets,
            DurableWorkAdmission::new(16).unwrap(),
            budget,
        )
        .unwrap();
        seats::seal_snapshot_stage(context, conn, &stage.id, budget).unwrap();
        seats::publish_snapshot_stage(context, conn, &stage.id, budget).unwrap()
    }
    for bound_terminal in ["terminal-b", "terminal-other"] {
        let path =
            std::env::temp_dir().join(format!("herdr-reconcile-{}.db", uuid::Uuid::new_v4()));
        let context = StoreContext::new(path.clone(), Arc::new(FixedClock));
        let mut conn = context.open_writer().unwrap();
        let budget = CallBudget {
            deadline: MonoInstant(1_000),
            cancellation: Cancellation::default(),
        };
        conn.execute(
            "INSERT INTO host_instances(id,created_at,host_boot,host_epoch) VALUES ('i',0,'boot-a',1)",
            [],
        )
        .unwrap();
        let kind = structural_proof("w1:p2", "terminal-b", 1).source_spelling();
        for (seat, address, terminal) in
            [("b", "w1:p2", "terminal-b"), ("d", "w1:p3", "terminal-d")]
        {
            conn.execute(
                "INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,structural_terminal_id,structural_incarnation,structural_incarnation_kind,structural_host_boot,structural_host_epoch,structural_connection_epoch,structural_observation_sequence,created_at) VALUES (?1,'i','resolved','native',?2,1,1,?3,'inc-a',?4,'boot-a',1,1,1,0)",
                rusqlite::params![seat, address, terminal, kind],
            )
            .unwrap();
        }
        // B is registered (cooperative check-in); D never registered.
        conn.execute(
            "INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES ('b',1,1,'w1:p2','boot-a',1,'claude','plugin_context:x','e','cooperative_top_level',1,1,?1,'inc-a')",
            [bound_terminal],
        )
        .unwrap();
        if bound_terminal != "terminal-b" {
            // Without the seat's own structural proof, only the binding's
            // terminal can carry it; a different terminal must not.
            conn.execute(
                "UPDATE seats SET structural_terminal_id=NULL,structural_incarnation=NULL,structural_incarnation_kind=NULL,structural_host_boot=NULL,structural_host_epoch=NULL,structural_connection_epoch=NULL,structural_observation_sequence=NULL WHERE id='b'",
                [],
            )
            .unwrap();
        }
        publish(
            &context,
            &mut conn,
            &budget,
            snapshot(
                2,
                vec![
                    production("w1:p2", "terminal-b", 2),
                    production("w1:p3", "terminal-d", 2),
                ],
            ),
        );
        let moved = publish(
            &context,
            &mut conn,
            &budget,
            snapshot(
                3,
                vec![
                    production("w2:p1", "terminal-b", 3),
                    production("w2:p2", "terminal-d", 3),
                ],
            ),
        );
        if bound_terminal != "terminal-b" {
            // Only a forged transition reaches the store here: the planner
            // never moves a seat without its own structural proof.
            let forged = crate::ports::GuardedSeatTransition {
                publication: moved.clone(),
                seat: SeatId::new("b"),
                expected_binding_generation: 1,
                expected_target: Some(HostTargetId::new("w1:p2")),
                expected_terminal: Some(TerminalId::new("terminal-b")),
                action: crate::ports::ReconciliationAction::Move {
                    target: HostTargetId::new("w2:p1"),
                    terminal: TerminalId::new("terminal-b"),
                },
            };
            assert_eq!(
                seats::apply_reconciliation_transition(&context, &mut conn, forged, &budget)
                    .unwrap(),
                crate::ports::ReconciliationOutcome::Stale,
                "a binding on another terminal must not carry the seat"
            );
            let target: String = conn
                .query_row("SELECT target_id FROM seats WHERE id='b'", [], |r| r.get(0))
                .unwrap();
            assert_eq!(target, "w1:p2");
            drop(conn);
            let _ = std::fs::remove_file(path);
            continue;
        }
        drop(conn);
        let store = SqliteStore::new(
            StoreContext::new(path.clone(), Arc::new(FixedClock)),
            "i",
            StoreSettings {
                daemon_boot: Some(uuid::Uuid::new_v4()),
                ..StoreSettings::default()
            },
        )
        .unwrap();
        let result = reconcile_published_page(&store, &moved, 0, None, &budget);
        let status = crate::service::workers::WorkerStatus::default();
        status.observe_reconciliation(&result);
        assert_eq!(status.health(), None, "Health must not report CursorStale");
        let progress = result.unwrap();
        let conn = context.open_writer().unwrap();
        let seat = |id: &str| -> (String, String, i64) {
            conn.query_row(
                "SELECT state,target_id,generation FROM seats WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap()
        };
        assert_eq!(
            seat("d"),
            ("resolved".into(), "w2:p2".into(), 1),
            "the later unregistered seat follows its own move"
        );
        let binding: (String, Option<String>, bool, bool) = conn
            .query_row(
                "SELECT target_id,terminal_id,registered_at IS NOT NULL,ended_at IS NULL FROM occupant_bindings WHERE seat_id='b'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(progress.transitions_refused, 0);
        assert_eq!(progress.transition_count, 2);
        assert_eq!(
            seat("b"),
            ("resolved".into(), "w2:p1".into(), 1),
            "the registered seat follows its pane with the same generation"
        );
        assert_eq!(
            binding,
            ("w2:p1".into(), Some("terminal-b".into()), true, true),
            "the registered binding moves with the seat and stays live"
        );
        drop(conn);
        drop(store);
        let _ = std::fs::remove_file(path);
    }
}

mod fake_port {
    use super::*;
    use crate::ports::*;
    use crate::protocol::{
        authority::{MutationPermit, OperatorActor},
        commands::{Command, PermitMutation, ResolveSeat, SendMessage},
        ids::{RetirementJobId, WakeAttemptId},
        pagination::{Page, PageRequest},
        results::{CommandResult, RetirementStatus},
        time::{CallBudget, Cancellation, Clock},
    };
    use std::sync::{Arc, Mutex};

    struct TestClock;
    impl Clock for TestClock {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(100)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(100)
        }
    }

    #[derive(Default)]
    struct State {
        header: Option<SnapshotHeader>,
        staged: u64,
        calls: Vec<String>,
        page: Option<SnapshotSeatPage>,
        invalidation_page: Option<InvalidationSeatPage>,
        actions: Vec<ReconciliationAction>,
        invalidated_seats: Vec<SeatId>,
        fail_offset: Option<u64>,
        admission_sequence: u64,
        decided_sequence: u64,
        invalidation_revision: u64,
        fail_invalidation: bool,
        refuse_seats: Vec<SeatId>,
    }
    struct FakeStore {
        clock: TestClock,
        state: Mutex<State>,
    }
    impl FakeStore {
        fn new() -> Self {
            Self {
                clock: TestClock,
                state: Mutex::new(State::default()),
            }
        }
    }
    fn unsupported() -> ApiError {
        stale("unused fake port operation")
    }
    fn stage_id() -> SnapshotGenerationId {
        SnapshotGenerationId::store_issued("stage-a".into())
    }
    impl StorePort for FakeStore {
        fn clock(&self) -> &dyn Clock {
            &self.clock
        }
        fn query(
            &self,
            _: &Command,
            _: &ReadContext,
            _: &CallBudget,
        ) -> Result<CommandResult, ApiError> {
            Err(unsupported())
        }
        fn mutate(
            &self,
            _: PermitMutation,
            _: MutationPermit,
            _: &CallBudget,
        ) -> Result<CommandResult, ApiError> {
            Err(unsupported())
        }
        fn prepare_send_step(
            &self,
            _: &SendMessage,
            _: DurableWorkAdmission,
            _: &CallBudget,
        ) -> Result<SendPreparationProgress, ApiError> {
            Err(unsupported())
        }
        fn abandon_send_preparation(&self, _: &str, _: &CallBudget) -> Result<(), ApiError> {
            Err(unsupported())
        }
        fn allocate_seat(
            &self,
            _: ResolveSeat,
            _: OrdinaryAllocationGuard,
            _: &CallBudget,
        ) -> Result<SeatId, ApiError> {
            Err(unsupported())
        }
        fn mutate_operator(
            &self,
            _: OperatorRequest,
            _: OperatorActor,
            _: &CallBudget,
        ) -> Result<CommandResult, ApiError> {
            Err(unsupported())
        }
        fn register_available(
            &self,
            _: RegisterAvailableRequest,
            _: MutationPermit,
            _: &CallBudget,
        ) -> Result<CommandResult, ApiError> {
            Err(unsupported())
        }
        fn revoke_registration(
            &self,
            _: RegistrationRevocation,
            _: &CallBudget,
        ) -> Result<bool, ApiError> {
            Err(unsupported())
        }
        fn begin_host_observation(
            &self,
            instance: &str,
            _: &CallBudget,
        ) -> Result<HostObservationAdmission, ApiError> {
            let mut state = self.state.lock().unwrap();
            state.admission_sequence += 1;
            state.calls.push("admit".into());
            Ok(HostObservationAdmission {
                instance: instance.into(),
                sequence: state.admission_sequence,
                expected_active: None,
                expected_boot: None,
                expected_epoch: 0,
                lifecycle_revision: 0,
                invalidation_revision: 0,
            })
        }
        fn publish_current_target_observation(
            &self,
            _: &HostObservationAdmission,
            _: &HostObservation,
            _: &CallBudget,
        ) -> Result<bool, ApiError> {
            Err(unsupported())
        }
        fn invalidate_host_observation(
            &self,
            admission: &HostObservationAdmission,
            reason: HostInvalidationReason,
            budget: &CallBudget,
        ) -> Result<Option<HostInvalidationFence>, ApiError> {
            assert!(
                !budget.cancellation.is_cancelled(),
                "invalidation needs a live maintenance budget"
            );
            let mut state = self.state.lock().unwrap();
            if state.fail_invalidation {
                return Err(stale("injected invalidation write failure"));
            }
            if admission.sequence <= state.decided_sequence {
                return Ok(None);
            }
            state.decided_sequence = admission.sequence;
            state.invalidation_revision += 1;
            state.calls.push(format!("invalidate:{reason:?}"));
            Ok(Some(HostInvalidationFence {
                admission: admission.clone(),
                invalidation_revision: state.invalidation_revision,
            }))
        }
        fn mark_unresolved_from_invalidation(
            &self,
            transition: GuardedInvalidationTransition,
            _: &CallBudget,
        ) -> Result<ReconciliationOutcome, ApiError> {
            self.state
                .lock()
                .unwrap()
                .invalidated_seats
                .push(transition.seat);
            Ok(ReconciliationOutcome::Applied)
        }
        fn saved_seats_page_for_invalidation(
            &self,
            fence: &HostInvalidationFence,
            _: u64,
            _: Option<u64>,
            _: u8,
            _: &CallBudget,
        ) -> Result<InvalidationSeatPage, ApiError> {
            let page = self
                .state
                .lock()
                .unwrap()
                .invalidation_page
                .clone()
                .unwrap();
            assert_eq!(&page.fence, fence);
            Ok(page)
        }
        fn begin_snapshot_stage(
            &self,
            header: SnapshotHeader,
            _: &CallBudget,
        ) -> Result<SnapshotStage, ApiError> {
            let mut state = self.state.lock().unwrap();
            if header.admission.sequence <= state.decided_sequence {
                return Err(stale("superseded observation admission"));
            }
            state.calls.push("begin".into());
            state.header = Some(header.clone());
            Ok(SnapshotStage {
                id: stage_id(),
                expected_targets: header.expected_targets,
                staged_targets: 0,
                sealed: false,
            })
        }
        fn stage_snapshot_targets(
            &self,
            stage: &SnapshotGenerationId,
            offset: u64,
            targets: &[HostObservation],
            admission: DurableWorkAdmission,
            _: &CallBudget,
        ) -> Result<SnapshotStageProgress, ApiError> {
            let mut state = self.state.lock().unwrap();
            assert_eq!(*stage, stage_id());
            assert_eq!(offset, state.staged);
            assert!(targets.len() <= usize::from(admission.max_units));
            state
                .calls
                .push(format!("stage:{}:{}", offset, targets.len()));
            if state.fail_offset == Some(offset) {
                return Err(stale("injected staging failure"));
            }
            state.staged += targets.len() as u64;
            Ok(SnapshotStageProgress {
                stage: SnapshotStage {
                    id: stage_id(),
                    expected_targets: state.header.as_ref().unwrap().expected_targets,
                    staged_targets: state.staged,
                    sealed: false,
                },
                visited: targets.len() as u8,
            })
        }
        fn seal_snapshot_stage(
            &self,
            _: &SnapshotGenerationId,
            _: &CallBudget,
        ) -> Result<SnapshotStage, ApiError> {
            let mut state = self.state.lock().unwrap();
            state.calls.push("seal".into());
            let expected = state.header.as_ref().unwrap().expected_targets;
            assert_eq!(state.staged, expected);
            Ok(SnapshotStage {
                id: stage_id(),
                expected_targets: expected,
                staged_targets: state.staged,
                sealed: true,
            })
        }
        fn publish_snapshot_stage(
            &self,
            _: &SnapshotGenerationId,
            _: &CallBudget,
        ) -> Result<PublishedSnapshot, ApiError> {
            let mut state = self.state.lock().unwrap();
            state.calls.push("publish".into());
            let header = state.header.as_ref().unwrap();
            let decision_sequence = header.admission.sequence;
            state.decided_sequence = decision_sequence;
            let header = state.header.as_ref().unwrap();
            Ok(PublishedSnapshot {
                id: stage_id(),
                instance: header.instance.clone(),
                boot: header.boot.clone(),
                epoch: header.epoch,
                observation_sequence: header.observation_sequence,
                incarnation: header.incarnation.clone(),
                target_count: header.expected_targets,
                invalidation_revision: 0,
            })
        }
        fn discard_snapshot_stage(
            &self,
            _: &SnapshotGenerationId,
            admission: DurableWorkAdmission,
            _: &CallBudget,
        ) -> Result<SnapshotCleanupProgress, ApiError> {
            let mut state = self.state.lock().unwrap();
            assert!(admission.max_units <= 16);
            state.calls.push("discard".into());
            Ok(SnapshotCleanupProgress {
                stage: stage_id(),
                visited: 0,
                complete: true,
            })
        }
        fn saved_seats_page(
            &self,
            published: &SnapshotGenerationId,
            _: u64,
            _: Option<u64>,
            limit: u8,
            _: &CallBudget,
        ) -> Result<SnapshotSeatPage, ApiError> {
            assert!(limit <= 16);
            let state = self.state.lock().unwrap();
            let page = state.page.clone().unwrap();
            assert_eq!(page.publication.id, *published);
            Ok(page)
        }
        fn apply_reconciliation_transition(
            &self,
            transition: GuardedSeatTransition,
            _: &CallBudget,
        ) -> Result<ReconciliationOutcome, ApiError> {
            let mut state = self.state.lock().unwrap();
            if state.refuse_seats.contains(&transition.seat) {
                return Ok(ReconciliationOutcome::Stale);
            }
            state.actions.push(transition.action.clone());
            if matches!(
                transition.action,
                ReconciliationAction::BeginRetirement { .. }
            ) {
                Ok(ReconciliationOutcome::RetirementStarted(RetirementJob {
                    id: RetirementJobId::new("retire-a"),
                    seat: transition.seat,
                    retired_at: UtcMillis(100),
                    phase: RetirementPhase::Warnings,
                    after_ordinal: None,
                    high_water_ordinal: 100_000,
                    processed_units: 0,
                }))
            } else {
                Ok(ReconciliationOutcome::Applied)
            }
        }
        fn due_obligations(
            &self,
            _: DueScanRequest,
            _: &CallBudget,
        ) -> Result<DueScanProgress, ApiError> {
            Err(unsupported())
        }
        fn begin_retirement(
            &self,
            _: SeatId,
            _: ClosureEvidence,
            _: &CallBudget,
        ) -> Result<RetirementJob, ApiError> {
            panic!("reconciliation must use guarded transition")
        }
        fn advance_retirement(
            &self,
            _: RetirementJobId,
            _: WorkAdmission,
            _: &CallBudget,
        ) -> Result<RetirementProgress, ApiError> {
            panic!("cleanup must be asynchronous")
        }
        fn pending_retirement_jobs(
            &self,
            _: PageRequest,
            _: &CallBudget,
        ) -> Result<Page<RetirementStatus>, ApiError> {
            Err(unsupported())
        }
        fn wake_candidates(
            &self,
            _: PageRequest,
            _: &CallBudget,
        ) -> Result<Page<WakeCandidate>, ApiError> {
            Err(unsupported())
        }
        fn wake_recovery_candidates(
            &self,
            _: PageRequest,
            _: &CallBudget,
        ) -> Result<Page<WakeRecoveryCandidate>, ApiError> {
            Err(unsupported())
        }
        fn recover_wake_reservation(
            &self,
            _: WakeRecoveryRequest,
            _: &CallBudget,
        ) -> Result<WakeRecoveryOutcome, ApiError> {
            Err(unsupported())
        }
        fn pending_work(
            &self,
            _: PageRequest,
            _: &CallBudget,
        ) -> Result<Page<WorkCandidate>, ApiError> {
            Err(unsupported())
        }
        fn advance_work(
            &self,
            _: &str,
            _: DurableWorkAdmission,
            _: &CallBudget,
        ) -> Result<WorkProgress, ApiError> {
            Err(unsupported())
        }
        fn reserve_wake(
            &self,
            _: &WakeCandidate,
            _: &CallBudget,
        ) -> Result<Option<WakeReservation>, ApiError> {
            Err(unsupported())
        }
        fn validate_wake_reservation(
            &self,
            _: &WakeReservation,
            _: &CallBudget,
        ) -> Result<bool, ApiError> {
            Err(unsupported())
        }
        fn complete_wake(
            &self,
            _: WakeAttemptId,
            _: WakeOutcome,
            _: &CallBudget,
        ) -> Result<(), ApiError> {
            Err(unsupported())
        }
    }

    fn budget() -> CallBudget {
        CallBudget {
            deadline: MonoInstant(1000),
            cancellation: Cancellation::default(),
        }
    }

    struct FakeHost {
        store: Arc<FakeStore>,
        capture: Result<HostSnapshot, ApiError>,
        cancel_on_return: bool,
    }
    impl HostPort for FakeHost {
        fn native_launch_capability(&self) -> NativeLaunchCapability {
            panic!("unused")
        }
        fn observe_current_target(
            &self,
            _: &HostTargetId,
            _: &HostCallContext,
        ) -> Result<HostObservation, ApiError> {
            panic!("unused")
        }
        fn enumerate_targets(&self, context: &HostCallContext) -> Result<HostSnapshot, ApiError> {
            assert_eq!(
                self.store
                    .state
                    .lock()
                    .unwrap()
                    .calls
                    .first()
                    .map(String::as_str),
                Some("admit")
            );
            if self.cancel_on_return {
                context.budget.cancellation.cancel();
            }
            self.capture.clone()
        }
        fn subscribe_lifecycle(
            &self,
            _: &HostCallContext,
        ) -> Result<Box<dyn HostLifecycleSubscription>, ApiError> {
            panic!("unused")
        }
        fn safe_wake_target(&self, _: &SeatId, _: &HostObservation) -> Option<SafeWakeTarget> {
            panic!("unused")
        }
        fn submit_prompt(
            &self,
            _: &SafeWakeTarget,
            _: &str,
            _: &HostCallContext,
        ) -> Result<PromptOutcome, ApiError> {
            panic!("unused")
        }
        fn launch_native(
            &self,
            _: NativeLaunchRequest,
            _: &HostCallContext,
        ) -> Result<NativeLaunchOutcome, ApiError> {
            panic!("unused")
        }
    }

    #[test]
    fn unknown_capture_admits_then_invalidates_without_staging_or_retirement() {
        let store = Arc::new(FakeStore::new());
        let mut unknown = snapshot(2, vec![]);
        unknown.incarnation = IncarnationEvidence::Unknown;
        let host = FakeHost {
            store: store.clone(),
            capture: Ok(unknown),
            cancel_on_return: false,
        };
        let mut lane = ObservationLane::default();
        let outcome = observe_and_publish(
            &host,
            store.as_ref(),
            &mut lane,
            "instance-a",
            &HostCallContext {
                budget: budget(),
                expected_boot: None,
                expected_epoch: None,
            },
            &budget(),
        )
        .unwrap();
        assert!(matches!(
            outcome,
            ObservationOutcome::Invalidated {
                reason: HostInvalidationReason::UnknownIncarnation,
                ..
            }
        ));
        assert!(!lane.is_available());
        assert_eq!(
            store.state.lock().unwrap().calls,
            ["admit", "invalidate:UnknownIncarnation"]
        );
        assert!(store.state.lock().unwrap().actions.is_empty());
    }

    #[test]
    fn partial_capture_never_stages_or_requests_absence_retirement() {
        let store = Arc::new(FakeStore::new());
        let mut partial = snapshot(2, vec![]);
        partial.complete = false;
        partial.enumeration = EnumerationEvidence::Partial;
        let host = FakeHost {
            store: store.clone(),
            capture: Ok(partial),
            cancel_on_return: false,
        };
        let mut lane = ObservationLane::default();
        let outcome = observe_and_publish(
            &host,
            store.as_ref(),
            &mut lane,
            "instance-a",
            &HostCallContext {
                budget: budget(),
                expected_boot: None,
                expected_epoch: None,
            },
            &budget(),
        )
        .unwrap();
        assert!(matches!(
            outcome,
            ObservationOutcome::Invalidated {
                reason: HostInvalidationReason::PartialEnumeration,
                ..
            }
        ));
        assert_eq!(
            store.state.lock().unwrap().calls,
            ["admit", "invalidate:PartialEnumeration"]
        );
        assert!(store.state.lock().unwrap().actions.is_empty());
    }

    #[test]
    fn older_failed_admission_cannot_invalidate_newer_decision() {
        let store = FakeStore::new();
        let old = store
            .begin_host_observation("instance-a", &budget())
            .unwrap();
        let new = store
            .begin_host_observation("instance-a", &budget())
            .unwrap();
        assert!(
            store
                .invalidate_host_observation(
                    &new,
                    HostInvalidationReason::HostUnavailable,
                    &budget()
                )
                .unwrap()
                .is_some()
        );
        assert!(
            store
                .invalidate_host_observation(
                    &old,
                    HostInvalidationReason::HostUnavailable,
                    &budget()
                )
                .unwrap()
                .is_none()
        );
        assert_eq!(store.state.lock().unwrap().invalidation_revision, 1);
    }

    #[test]
    fn late_success_cannot_publish_after_newer_invalidation() {
        let store = FakeStore::new();
        let old = store
            .begin_host_observation("instance-a", &budget())
            .unwrap();
        let newer = store
            .begin_host_observation("instance-a", &budget())
            .unwrap();
        store
            .invalidate_host_observation(&newer, HostInvalidationReason::HostUnavailable, &budget())
            .unwrap();
        let mut lane = ObservationLane::default();
        let ticket = lane.begin_observation().unwrap();
        assert!(
            publish_captured(
                &store,
                &mut lane,
                ticket,
                old,
                &snapshot(2, vec![]),
                &budget()
            )
            .is_err()
        );
        lane.discard(ticket).unwrap();
        assert!(
            !store
                .state
                .lock()
                .unwrap()
                .calls
                .iter()
                .any(|call| call == "publish")
        );
    }

    #[test]
    fn late_coherent_result_uses_maintenance_budget_to_fail_closed() {
        let store = Arc::new(FakeStore::new());
        let host = FakeHost {
            store: store.clone(),
            capture: Ok(snapshot(2, vec![])),
            cancel_on_return: true,
        };
        let mut lane = ObservationLane::default();
        let outcome = observe_and_publish(
            &host,
            store.as_ref(),
            &mut lane,
            "instance-a",
            &HostCallContext {
                budget: budget(),
                expected_boot: None,
                expected_epoch: None,
            },
            &budget(),
        )
        .unwrap();
        assert!(matches!(
            outcome,
            ObservationOutcome::Invalidated {
                reason: HostInvalidationReason::HostUnavailable,
                ..
            }
        ));
        assert!(!lane.is_available());
        assert_eq!(
            store.state.lock().unwrap().calls,
            ["admit", "invalidate:HostUnavailable"]
        );
    }

    #[test]
    fn invalidation_write_failure_remains_error_and_locally_unavailable() {
        let store = Arc::new(FakeStore::new());
        store.state.lock().unwrap().fail_invalidation = true;
        let host = FakeHost {
            store: store.clone(),
            capture: Err(stale("denied host read")),
            cancel_on_return: false,
        };
        let mut lane = ObservationLane::default();
        assert!(
            observe_and_publish(
                &host,
                store.as_ref(),
                &mut lane,
                "instance-a",
                &HostCallContext {
                    budget: budget(),
                    expected_boot: None,
                    expected_epoch: None
                },
                &budget(),
            )
            .is_err()
        );
        assert!(!lane.is_available());
        assert!(lane.snapshot_due(MonoInstant(101)));
        assert!(
            !store
                .state
                .lock()
                .unwrap()
                .calls
                .iter()
                .any(|call| call.starts_with("invalidate:"))
        );
    }

    #[test]
    fn coherent_capture_stage_failure_durably_invalidates_old_publication() {
        let store = Arc::new(FakeStore::new());
        store.state.lock().unwrap().fail_offset = Some(0);
        let host = FakeHost {
            store: store.clone(),
            capture: Ok(snapshot(2, vec![target("p", "t", None, 2)])),
            cancel_on_return: false,
        };
        let mut lane = ObservationLane::default();
        let outcome = observe_and_publish(
            &host,
            store.as_ref(),
            &mut lane,
            "instance-a",
            &HostCallContext {
                budget: budget(),
                expected_boot: None,
                expected_epoch: None,
            },
            &budget(),
        )
        .unwrap();
        assert!(matches!(
            outcome,
            ObservationOutcome::Invalidated {
                reason: HostInvalidationReason::PublicationFailed,
                ..
            }
        ));
        assert!(!lane.is_available());
        assert_eq!(
            store.state.lock().unwrap().calls,
            [
                "admit",
                "begin",
                "stage:0:1",
                "discard",
                "invalidate:PublicationFailed"
            ]
        );
    }

    #[test]
    fn capture_stages_in_bounded_steps_before_publish_and_discards_failed_stage() {
        let store = FakeStore::new();
        let captured = snapshot(
            2,
            (0..20)
                .map(|n| target(&format!("p-{n}"), &format!("t-{n}"), None, 2))
                .collect(),
        );
        let mut lane = ObservationLane::default();
        let ticket = lane.begin_observation().unwrap();
        let host_admission = store
            .begin_host_observation("instance-a", &budget())
            .unwrap();
        let published = publish_captured(
            &store,
            &mut lane,
            ticket,
            host_admission,
            &captured,
            &budget(),
        )
        .unwrap();
        assert_eq!(published.target_count, 20);
        assert_eq!(
            store.state.lock().unwrap().calls,
            [
                "admit",
                "begin",
                "stage:0:16",
                "stage:16:4",
                "seal",
                "publish"
            ]
        );
        assert!(!lane.snapshot_due(MonoInstant(101)));

        let store = FakeStore::new();
        store.state.lock().unwrap().fail_offset = Some(16);
        let mut lane = ObservationLane::default();
        let ticket = lane.begin_observation().unwrap();
        let host_admission = store
            .begin_host_observation("instance-a", &budget())
            .unwrap();
        assert!(
            publish_captured(
                &store,
                &mut lane,
                ticket,
                host_admission,
                &captured,
                &budget()
            )
            .is_err()
        );
        lane.discard(ticket).unwrap();
        assert_eq!(
            store.state.lock().unwrap().calls,
            ["admit", "begin", "stage:0:16", "stage:16:4", "discard"]
        );
        assert!(lane.snapshot_due(MonoInstant(101)));
    }

    #[test]
    fn authoritative_absence_commits_only_retirement_fence_and_releases_page() {
        let store = FakeStore::new();
        let publication = PublishedSnapshot {
            id: stage_id(),
            instance: "instance-a".into(),
            boot: HostBootId::new("boot-a"),
            epoch: 1,
            observation_sequence: 2,
            incarnation: "inc-a".into(),
            target_count: 100_000,
            invalidation_revision: 0,
        };
        store.state.lock().unwrap().page = Some(SnapshotSeatPage {
            publication: publication.clone(),
            high_water_ordinal: 1,
            after_ordinal: 1,
            visited: 1,
            has_more: false,
            seats: vec![SnapshotSavedSeat {
                ordinal: 1,
                seat: SeatId::new("seat-a"),
                state: SeatState::Resolved,
                unresolved_reason: None,
                prior_published_observation: None,
                structural_proof: Some(structural_proof("p-old", "t-old", 1)),
                target: Some(HostTargetId::new("p-old")),
                terminal: Some(TerminalId::new("t-old")),
                binding_generation: 3,
                binding_execution: None,
                active_binding_execution: None,
                bound_boot: Some(HostBootId::new("boot-a")),
                bound_incarnation: Some("inc-a".into()),
                latest_binding_evidence: None,
                observed_match: None,
            }],
        });
        let progress = reconcile_published_page(&store, &publication, 0, None, &budget()).unwrap();
        assert_eq!(progress.retirements_started, 1);
        assert_eq!(progress.transition_count, 1);
        assert_eq!(progress.next_after_ordinal, None);
        assert!(matches!(
            store.state.lock().unwrap().actions[0],
            ReconciliationAction::BeginRetirement { .. }
        ));

        // Cleanup remains pending in the returned job; an unrelated published
        // page can still be handled without invoking advance_retirement.
        store
            .state
            .lock()
            .unwrap()
            .page
            .as_mut()
            .unwrap()
            .seats
            .clear();
        store.state.lock().unwrap().page.as_mut().unwrap().visited = 0;
        let unrelated = reconcile_published_page(&store, &publication, 0, None, &budget()).unwrap();
        assert_eq!(unrelated.transition_count, 0);
    }

    #[test]
    fn invalidation_projects_only_bounded_unresolved_transitions() {
        let store = FakeStore::new();
        let admission = store
            .begin_host_observation("instance-a", &budget())
            .unwrap();
        let fence = HostInvalidationFence {
            admission,
            invalidation_revision: 1,
        };
        store.state.lock().unwrap().invalidation_page = Some(InvalidationSeatPage {
            fence: fence.clone(),
            high_water_ordinal: 1,
            after_ordinal: 1,
            visited: 1,
            has_more: false,
            seats: vec![SnapshotSavedSeat {
                ordinal: 1,
                seat: SeatId::new("seat-a"),
                state: SeatState::Resolved,
                unresolved_reason: None,
                prior_published_observation: None,
                structural_proof: None,
                target: Some(HostTargetId::new("p-old")),
                terminal: Some(TerminalId::new("t-old")),
                binding_generation: 3,
                binding_execution: None,
                active_binding_execution: None,
                bound_boot: Some(HostBootId::new("boot-a")),
                bound_incarnation: Some("inc-a".into()),
                latest_binding_evidence: None,
                observed_match: None,
            }],
        });
        let progress = reconcile_invalidated_page(&store, &fence, 0, None, &budget()).unwrap();
        assert_eq!(progress.transition_count, 1);
        assert_eq!(progress.retirements_started, 0);
        assert_eq!(
            store.state.lock().unwrap().invalidated_seats,
            [SeatId::new("seat-a")]
        );
        assert!(store.state.lock().unwrap().actions.is_empty());
    }

    /// D2: a guarded transition the store refuses for one seat is recorded
    /// and skipped; it never aborts the page, so a later seat on the same
    /// page still follows its move and Health never reports CursorStale.
    /// Kills: `reconcile_published_page` returning CursorStale on the first
    /// `ReconciliationOutcome::Stale` (the pre-fix whole-page abort).
    #[test]
    fn refused_transition_for_one_seat_does_not_block_another_seats_move() {
        let store = FakeStore::new();
        let publication = PublishedSnapshot {
            id: stage_id(),
            instance: "instance-a".into(),
            boot: HostBootId::new("boot-a"),
            epoch: 1,
            observation_sequence: 2,
            incarnation: "inc-a".into(),
            target_count: 2,
            invalidation_revision: 0,
        };
        let moved =
            |ordinal: u64, seat: &str, old: &str, new: &str, terminal: &str| SnapshotSavedSeat {
                ordinal,
                seat: SeatId::new(seat),
                state: SeatState::Resolved,
                unresolved_reason: None,
                prior_published_observation: None,
                structural_proof: Some(structural_proof(old, terminal, 1)),
                target: Some(HostTargetId::new(old)),
                terminal: Some(TerminalId::new(terminal)),
                binding_generation: 1,
                binding_execution: None,
                active_binding_execution: None,
                bound_boot: Some(HostBootId::new("boot-a")),
                bound_incarnation: Some("inc-a".into()),
                latest_binding_evidence: None,
                observed_match: Some(SnapshotTargetMatch {
                    target: HostTargetId::new(new),
                    terminal: Some(TerminalId::new(terminal)),
                    structural_generation: 1,
                    observation_sequence: 2,
                    connection_epoch: Some(1),
                    incarnation_source: Some(EvidenceKind::CoherentEnumeration),
                    occupancy: StructuralOccupancy::Unknown,
                    verified_execution: None,
                    top_level_occupant: false,
                }),
            };
        {
            let mut state = store.state.lock().unwrap();
            state.page = Some(SnapshotSeatPage {
                publication: publication.clone(),
                high_water_ordinal: 2,
                after_ordinal: 2,
                visited: 2,
                has_more: false,
                seats: vec![
                    moved(1, "seat-b", "w1:p2", "w2:p1", "terminal-b"),
                    moved(2, "seat-d", "w1:p3", "w2:p2", "terminal-d"),
                ],
            });
            state.refuse_seats = vec![SeatId::new("seat-b")];
        }
        let result = reconcile_published_page(&store, &publication, 0, None, &budget());
        let status = crate::service::workers::WorkerStatus::default();
        status.observe_reconciliation(&result);
        assert_eq!(
            status.health(),
            None,
            "a per-seat refusal must not reach Health as a reconciliation failure"
        );
        let progress = result.expect("a refused seat must not abort the page");
        assert_eq!(progress.transitions_refused, 1);
        assert_eq!(progress.transition_count, 1);
        assert_eq!(progress.next_after_ordinal, None);
        assert_eq!(
            store.state.lock().unwrap().actions,
            [ReconciliationAction::Move {
                target: HostTargetId::new("w2:p2"),
                terminal: TerminalId::new("terminal-d"),
            }],
            "the later seat still follows its own move"
        );
    }
}
