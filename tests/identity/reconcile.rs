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
        focused: false,
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
fn discarded_attempt_waits_for_the_pacer_retry_time_but_event_hints_do_not() {
    let mut lane = ObservationLane::default();
    let ticket = lane.begin_observation().unwrap();
    lane.mark_unavailable();
    lane.discard(ticket).unwrap();
    // Gated by the Pacer's next_retry_at(), not immediately due.
    assert!(!lane.snapshot_due_gated(MonoInstant(100), Some(MonoInstant(400))));
    assert!(lane.snapshot_due_gated(MonoInstant(400), Some(MonoInstant(400))));
    // Host-event hints still set dirty and bypass the gate.
    lane.mark_event_dirty();
    assert!(lane.snapshot_due_gated(MonoInstant(100), Some(MonoInstant(400))));
}

#[test]
fn explicit_capture_request_opens_the_gate_once() {
    let mut lane = ObservationLane::default();
    let ticket = lane.begin_observation().unwrap();
    lane.mark_unavailable();
    lane.discard(ticket).unwrap();
    assert!(!lane.snapshot_due_gated(MonoInstant(100), Some(MonoInstant(400))));
    lane.request_explicit_capture();
    assert!(lane.snapshot_due_gated(MonoInstant(100), Some(MonoInstant(400))));
    assert!(!lane.is_available(), "a request is not evidence");
    // Consumed by the next observation's start.
    let ticket = lane.begin_observation().unwrap();
    lane.mark_unavailable();
    lane.discard(ticket).unwrap();
    assert!(!lane.snapshot_due_gated(MonoInstant(100), Some(MonoInstant(400))));
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
            bound_epoch: None,
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
    // A different verified execution is not a replacement: the production
    // adapter never reports one, so only the structural move is planned.
    assert!(matches!(
        plan_page(&replacement).unwrap()[0].action,
        crate::ports::ReconciliationAction::Move { .. }
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
fn occupant_evidence_never_unseats_or_replaces_the_active_occupant() {
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
            bound_epoch: None,
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
    // Not even positive absence evidence unseats the occupant: a non-top-level
    // occupant or an observed empty shell on the same terminal and target plans
    // nothing, and the same terminal at a new address only moves structure.
    // Kills: a planner that emits any occupant-loss action.
    page.seats[0]
        .observed_match
        .as_mut()
        .unwrap()
        .top_level_occupant = false;
    assert!(
        plan_page(&page).unwrap().is_empty(),
        "a non-top-level occupant on the same terminal is not a transition"
    );
    page.seats[0].observed_match.as_mut().unwrap().occupancy = StructuralOccupancy::EmptyShell;
    assert!(
        plan_page(&page).unwrap().is_empty(),
        "an observed empty shell on the same terminal is not a transition"
    );
    page.seats[0].observed_match.as_mut().unwrap().target = HostTargetId::new("pane-b");
    assert!(matches!(
        &plan_page(&page).unwrap()[0].action,
        ReconciliationAction::Move { target, .. } if target.as_str() == "pane-b"
    ));
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
    assert!(
        matches!(
            plan_page(&page).unwrap()[0].action,
            ReconciliationAction::Move { .. }
        ),
        "a different verified execution never plans a replacement"
    );
}

#[test]
fn host_invalidation_never_reconfirms_from_execution_or_prior_empty_shell_evidence() {
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
            bound_epoch: None,
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
    // A verified execution on the same terminal is not continuity evidence by
    // itself: with no stored binding or structural proof the seat stays
    // unresolved, whatever execution the publication reports.
    assert!(matches!(
        &plan_page(&page).unwrap()[0].action,
        crate::ports::ReconciliationAction::MarkUnresolved
    ));
    page.seats[0]
        .observed_match
        .as_mut()
        .unwrap()
        .verified_execution = Some(ExecutionId::new("execution-new"));
    assert!(matches!(
        &plan_page(&page).unwrap()[0].action,
        crate::ports::ReconciliationAction::MarkUnresolved
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
    // A prior published empty shell on the same terminal no longer bridges
    // a host invalidation: only structural binding or proof evidence does.
    assert!(matches!(
        &plan_page(&page).unwrap()[0].action,
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
            bound_epoch: None,
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
    // Verified execution evidence is not consulted: the structural bridge decides.
    let mut verified = page.clone();
    let observed = verified.seats[0].observed_match.as_mut().unwrap();
    observed.occupancy = StructuralOccupancy::Occupied;
    observed.top_level_occupant = true;
    observed.verified_execution = Some(ExecutionId::new("execution-new"));
    assert!(matches!(
        action(&verified),
        ReconciliationAction::ReconfirmStructure { .. }
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
            bound_epoch: None,
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
fn ordinary_empty_allocation_survives_a_later_coherent_publication() {
    use crate::ports::{
        DurableWorkAdmission, OrdinaryResolutionAttempt, OrdinaryResolutionGuard,
        OrdinaryResolutionOutcome, SnapshotHeader,
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
    let guard = || OrdinaryResolutionGuard::try_new(&request, fresh.clone(), &admission).unwrap();
    let OrdinaryResolutionOutcome::Resolved(seat) = seats::resolve_seat(
        &context,
        &mut conn,
        "i",
        request.clone(),
        OrdinaryResolutionAttempt::Observed(guard()),
        &budget,
    )
    .unwrap() else {
        panic!("seat missing")
    };
    assert_eq!(
        seats::resolve_seat(
            &context,
            &mut conn,
            "i",
            request.clone(),
            OrdinaryResolutionAttempt::Observed(guard()),
            &budget,
        )
        .unwrap(),
        OrdinaryResolutionOutcome::Resolved(seat.clone())
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
    assert_eq!(
        seats::resolve_seat(
            &context,
            &mut conn,
            "i",
            request,
            OrdinaryResolutionAttempt::ReplayOnly,
            &budget,
        )
        .unwrap(),
        OrdinaryResolutionOutcome::Resolved(seat.clone())
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

    let closed = publish(&context, &mut conn, &budget, snapshot(111, vec![]));
    let closed_page =
        seats::saved_seats_page(&context, &conn, &closed.id, 0, None, 16, &budget).unwrap();
    let close_action = plan_page(&closed_page).unwrap();
    assert!(
        matches!(&close_action[0].action, ReconciliationAction::BeginRetirement { absent_target } if absent_target.as_str() == "pane-b")
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
        let store = Arc::new(
            SqliteStore::new(
                StoreContext::new(path.clone(), Arc::new(FixedClock)),
                "i",
                StoreSettings {
                    daemon_boot: Some(uuid::Uuid::new_v4()),
                    ..StoreSettings::default()
                },
            )
            .unwrap(),
        );
        let ports = crate::service::workers::ScheduledStore::new(
            store.clone(),
            Arc::new(crate::service::fair_writer::FairWriter::new(32)),
        );
        let result = reconcile_published_page(&ports, &moved, 0, None, &budget);
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
        drop(ports);
        drop(store);
        let _ = std::fs::remove_file(path);
    }
}

mod fake_port {
    use super::*;
    use crate::identity::reconcile::observation_store::ObservationStore;
    use crate::ports::*;
    use crate::protocol::{
        ids::RetirementJobId,
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
        /// When non-empty, `saved_seats_page_for_invalidation` serves these
        /// seats one per page for whatever fence is asked.
        invalidation_seats: Vec<SnapshotSavedSeat>,
        /// Serving the page after this ordinal fails (page k of the pass).
        fail_invalidation_page_after: Option<u64>,
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
    fn stage_id() -> SnapshotGenerationId {
        SnapshotGenerationId::store_issued("stage-a".into())
    }
    impl ObservationStore for FakeStore {
        fn clock(&self) -> &dyn Clock {
            &self.clock
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
            after: u64,
            _: Option<u64>,
            _: u8,
            _: &CallBudget,
        ) -> Result<InvalidationSeatPage, ApiError> {
            {
                let state = self.state.lock().unwrap();
                if state.fail_invalidation_page_after == Some(after) {
                    return Err(stale("injected marking page failure"));
                }
                if !state.invalidation_seats.is_empty() {
                    let remaining: Vec<_> = state
                        .invalidation_seats
                        .iter()
                        .filter(|seat| seat.ordinal > after)
                        .cloned()
                        .collect();
                    let seat = remaining.first().cloned().expect("page past the end");
                    return Ok(InvalidationSeatPage {
                        fence: fence.clone(),
                        high_water_ordinal: state.invalidation_seats.len() as u64,
                        after_ordinal: seat.ordinal,
                        visited: 1,
                        has_more: remaining.len() > 1,
                        seats: vec![seat],
                    });
                }
            }
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
        fn pane_agent_state(
            &self,
            _target: &SafeWakeTarget,
            _context: &HostCallContext,
        ) -> Result<crate::ports::AgentComposerState, ApiError> {
            Ok(crate::ports::AgentComposerState::Submitted)
        }

        fn launch_native(
            &self,
            _: NativeLaunchRequest,
            _: &HostCallContext,
        ) -> Result<NativeLaunchOutcome, ApiError> {
            panic!("unused")
        }
        fn send_submit_key(
            &self,
            _: &crate::ports::SafeWakeTarget,
            _: &crate::ports::HostCallContext,
        ) -> Result<(), crate::protocol::results::ApiError> {
            Ok(())
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
    fn late_coherent_result_freezes_without_invalidating() {
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
        // A coherent answer past its budget is unavailability: frozen, not
        // invalidated (ht-yms).
        assert!(matches!(
            outcome,
            ObservationOutcome::Frozen {
                reason: HostInvalidationReason::HostUnavailable,
                ..
            }
        ));
        assert!(!lane.is_available());
        assert_eq!(store.state.lock().unwrap().calls, ["admit"]);
    }

    #[test]
    fn invalidation_write_failure_remains_error_and_locally_unavailable() {
        let store = Arc::new(FakeStore::new());
        store.state.lock().unwrap().fail_invalidation = true;
        let host = FakeHost {
            store: store.clone(),
            capture: partial(),
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
    fn coherent_capture_stage_failure_freezes_the_old_publication() {
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
        // A daemon-side staging failure is not host evidence: frozen, the old
        // publication stays (ht-yms).
        assert!(matches!(
            outcome,
            ObservationOutcome::Frozen {
                reason: HostInvalidationReason::PublicationFailed,
                ..
            }
        ));
        assert!(!lane.is_available());
        assert_eq!(
            store.state.lock().unwrap().calls,
            ["admit", "begin", "stage:0:1", "discard"]
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
                bound_epoch: None,
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

    fn saved_seat(ordinal: u64, name: &str) -> SnapshotSavedSeat {
        SnapshotSavedSeat {
            ordinal,
            seat: SeatId::new(name),
            state: SeatState::Resolved,
            unresolved_reason: None,
            prior_published_observation: None,
            structural_proof: None,
            target: Some(HostTargetId::new("p-old")),
            terminal: Some(TerminalId::new("t-old")),
            binding_generation: 1,
            binding_execution: None,
            active_binding_execution: None,
            bound_epoch: None,
            bound_boot: Some(HostBootId::new("boot-a")),
            bound_incarnation: Some("inc-a".into()),
            latest_binding_evidence: None,
            observed_match: None,
        }
    }

    fn context() -> HostCallContext {
        HostCallContext {
            budget: budget(),
            expected_boot: None,
            expected_epoch: None,
        }
    }

    /// One capture attempt through `observe_and_publish` with the given host
    /// result.
    fn observe(
        store: &Arc<FakeStore>,
        lane: &mut ObservationLane,
        capture: Result<HostSnapshot, ApiError>,
    ) -> ObservationOutcome {
        let host = FakeHost {
            store: store.clone(),
            capture,
            cancel_on_return: false,
        };
        observe_and_publish(
            &host,
            store.as_ref(),
            lane,
            "instance-a",
            &context(),
            &budget(),
        )
        .unwrap()
    }

    /// Runs the whole unresolved-marking continuation of an `Invalidated`
    /// outcome the way the lane loop does, feeding the lane each page result.
    fn run_marking_pass(
        store: &Arc<FakeStore>,
        lane: &mut ObservationLane,
        outcome: &ObservationOutcome,
    ) -> Result<(), ApiError> {
        let ObservationOutcome::Invalidated { fence, reason, .. } = outcome else {
            panic!("not an invalidation: {outcome:?}");
        };
        let (mut after, mut high) = (0, None);
        loop {
            let page = reconcile_invalidated_page(store.as_ref(), fence, after, high, &budget());
            lane.note_invalidation_page(*reason, &page);
            let page = page?;
            match page.next_after_ordinal {
                Some(next) => (after, high) = (next, Some(page.high_water_ordinal)),
                None => return Ok(()),
            }
        }
    }

    fn invalidations(store: &FakeStore) -> usize {
        let state = store.state.lock().unwrap();
        state
            .calls
            .iter()
            .filter(|call| call.starts_with("invalidate:"))
            .count()
    }

    /// A capture that still invalidates (evidence the host view is not
    /// coherently known): an incomplete enumeration.
    fn partial() -> Result<HostSnapshot, ApiError> {
        let mut partial = snapshot(2, vec![]);
        partial.complete = false;
        Ok(partial)
    }

    /// Herdr unavailable (ht-yms, TRUST-POLICY C4): a failed capture writes no
    /// invalidation and arms no marking pass, however often it repeats, so
    /// seats and bindings stay frozen. Kills: invalidating (or marking) on a
    /// host that merely did not answer.
    #[test]
    fn unavailable_host_freezes_without_invalidating_or_marking() {
        let store = Arc::new(FakeStore::new());
        store.state.lock().unwrap().invalidation_seats = vec![saved_seat(1, "seat-a")];
        let mut lane = ObservationLane::default();
        for detail in ["host down", "host still down"] {
            let outcome = observe(&store, &mut lane, Err(stale(detail)));
            assert!(
                matches!(
                    outcome,
                    ObservationOutcome::Frozen {
                        reason: HostInvalidationReason::HostUnavailable,
                        cause: Some(_)
                    }
                ),
                "{outcome:?}"
            );
            assert!(!lane.is_available());
        }
        assert_eq!(invalidations(&store), 0);
        assert!(store.state.lock().unwrap().invalidated_seats.is_empty());
        assert_eq!(store.state.lock().unwrap().calls, ["admit", "admit"]);
    }

    #[test]
    fn repeat_failure_with_same_reason_after_completed_marking_writes_no_invalidation() {
        let store = Arc::new(FakeStore::new());
        store.state.lock().unwrap().invalidation_seats = vec![saved_seat(1, "seat-a")];
        let mut lane = ObservationLane::default();
        let first = observe(&store, &mut lane, partial());
        assert!(matches!(first, ObservationOutcome::Invalidated { .. }));
        // Marker is set only once the marking pass's last page commits.
        assert_eq!(lane.last_invalidation_reason(), None);
        run_marking_pass(&store, &mut lane, &first).unwrap();
        assert_eq!(
            lane.last_invalidation_reason(),
            Some(HostInvalidationReason::PartialEnumeration)
        );
        let second = observe(&store, &mut lane, partial());
        assert!(matches!(
            second,
            ObservationOutcome::InvalidationRepeated {
                reason: HostInvalidationReason::PartialEnumeration,
                cause: None
            }
        ));
        // The admission (fence) commit stays; the invalidation is not rewritten.
        assert_eq!(
            store.state.lock().unwrap().calls,
            ["admit", "invalidate:PartialEnumeration", "admit"]
        );
        assert_eq!(lane.repeat_failures(), 1);
        assert!(!lane.is_available());
    }

    #[test]
    fn repeat_before_marking_completes_invalidates_again() {
        let store = Arc::new(FakeStore::new());
        let mut lane = ObservationLane::default();
        observe(&store, &mut lane, partial());
        // No marking page ran: the pass has not completed.
        let second = observe(&store, &mut lane, partial());
        assert!(matches!(second, ObservationOutcome::Invalidated { .. }));
        assert_eq!(invalidations(&store), 2);
    }

    #[test]
    fn different_reason_invalidates_again() {
        let store = Arc::new(FakeStore::new());
        store.state.lock().unwrap().invalidation_seats = vec![saved_seat(1, "seat-a")];
        let mut lane = ObservationLane::default();
        let first = observe(&store, &mut lane, partial());
        run_marking_pass(&store, &mut lane, &first).unwrap();
        let mut unknown = snapshot(2, vec![]);
        unknown.incarnation = crate::ports::IncarnationEvidence::Unknown;
        let second = observe(&store, &mut lane, Ok(unknown));
        assert!(matches!(
            second,
            ObservationOutcome::Invalidated {
                reason: HostInvalidationReason::UnknownIncarnation,
                ..
            }
        ));
        assert_eq!(invalidations(&store), 2);
        assert_eq!(lane.repeat_failures(), 0);
    }

    #[test]
    fn publication_in_between_invalidates_again() {
        let store = Arc::new(FakeStore::new());
        store.state.lock().unwrap().invalidation_seats = vec![saved_seat(1, "seat-a")];
        let mut lane = ObservationLane::default();
        let first = observe(&store, &mut lane, partial());
        run_marking_pass(&store, &mut lane, &first).unwrap();
        let published = observe(&store, &mut lane, Ok(snapshot(2, vec![])));
        assert!(matches!(published, ObservationOutcome::Published(_)));
        assert_eq!(lane.last_invalidation_reason(), None);
        let again = observe(&store, &mut lane, partial());
        assert!(matches!(again, ObservationOutcome::Invalidated { .. }));
        assert_eq!(invalidations(&store), 2);
    }

    #[test]
    fn interrupted_first_marking_pass_reruns_from_ordinal_zero() {
        let store = Arc::new(FakeStore::new());
        {
            let mut state = store.state.lock().unwrap();
            state.invalidation_seats = (1..=4)
                .map(|n| saved_seat(n, &format!("seat-{n}")))
                .collect();
            // Page k = the third page (after ordinal 2) fails.
            state.fail_invalidation_page_after = Some(2);
        }
        let mut lane = ObservationLane::default();
        let first = observe(&store, &mut lane, partial());
        assert!(run_marking_pass(&store, &mut lane, &first).is_err());
        assert_eq!(lane.last_invalidation_reason(), None);
        assert_eq!(
            store.state.lock().unwrap().invalidated_seats,
            [SeatId::new("seat-1"), SeatId::new("seat-2")]
        );
        // Same reason again: not skipped, and the pass restarts at ordinal 0.
        store.state.lock().unwrap().fail_invalidation_page_after = None;
        store.state.lock().unwrap().invalidated_seats.clear();
        let second = observe(&store, &mut lane, partial());
        assert!(matches!(second, ObservationOutcome::Invalidated { .. }));
        run_marking_pass(&store, &mut lane, &second).unwrap();
        assert_eq!(
            store.state.lock().unwrap().invalidated_seats,
            (1..=4)
                .map(|n| SeatId::new(format!("seat-{n}")))
                .collect::<Vec<_>>()
        );
        assert_eq!(invalidations(&store), 2);
    }

    #[test]
    fn skip_returns_invalidation_repeated_and_keeps_health_invalidated() {
        use crate::service::workers::{RedactedFailure, WorkerStatus};
        let store = Arc::new(FakeStore::new());
        store.state.lock().unwrap().invalidation_seats = vec![saved_seat(1, "seat-a")];
        let mut lane = ObservationLane::default();
        let status = WorkerStatus::default();
        let first = observe(&store, &mut lane, partial());
        status.observe_capture(&Ok(Some(first.clone())));
        run_marking_pass(&store, &mut lane, &first).unwrap();
        let pages_before = store.state.lock().unwrap().invalidated_seats.len();
        let second = observe(&store, &mut lane, partial());
        assert!(matches!(
            second,
            ObservationOutcome::InvalidationRepeated { .. }
        ));
        status.observe_capture(&Ok(Some(second)));
        // Health keeps ObservationInvalidated(reason): not cleared, and not
        // replaced by ObservationCapture.
        assert_eq!(
            status.health().unwrap().failure,
            Some(RedactedFailure::ObservationInvalidated(
                HostInvalidationReason::PartialEnumeration
            ))
        );
        // No continuation page ran for the skipped failure.
        assert_eq!(
            store.state.lock().unwrap().invalidated_seats.len(),
            pages_before
        );
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
                bound_epoch: None,
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

    /// An already-unresolved seat (here `other`) is skipped, not re-marked:
    /// the pass goes on to mark the resolved seat after it and completes.
    /// Kills: re-marking it (the store refuses that as Stale, which aborted
    /// the pass with CursorStale, so it never completed and every retry wrote
    /// a new invalidation: ht-zo4,
    /// elected_daemon_reports_startup_binding_evidence_counts_in_health_and_log).
    #[test]
    fn invalidation_skips_already_unresolved_seats_and_marks_the_rest() {
        let store = FakeStore::new();
        let admission = store
            .begin_host_observation("instance-a", &budget())
            .unwrap();
        let fence = HostInvalidationFence {
            admission,
            invalidation_revision: 1,
        };
        let seat = |ordinal: u64, id: &str, state: SeatState| SnapshotSavedSeat {
            ordinal,
            seat: SeatId::new(id),
            state,
            unresolved_reason: (state == SeatState::Unresolved).then_some(UnresolvedReason::Other),
            prior_published_observation: None,
            structural_proof: None,
            target: Some(HostTargetId::new(format!("p-{id}"))),
            terminal: Some(TerminalId::new(format!("t-{id}"))),
            binding_generation: 3,
            binding_execution: None,
            active_binding_execution: None,
            bound_epoch: None,
            bound_boot: Some(HostBootId::new("boot-a")),
            bound_incarnation: Some("inc-a".into()),
            latest_binding_evidence: None,
            observed_match: None,
        };
        store.state.lock().unwrap().invalidation_page = Some(InvalidationSeatPage {
            fence: fence.clone(),
            high_water_ordinal: 2,
            after_ordinal: 2,
            visited: 2,
            has_more: false,
            seats: vec![
                seat(1, "stuck", SeatState::Unresolved),
                seat(2, "live", SeatState::Resolved),
            ],
        });
        let progress = reconcile_invalidated_page(&store, &fence, 0, None, &budget()).unwrap();
        assert_eq!(progress.transition_count, 1);
        assert_eq!(progress.next_after_ordinal, None, "the pass completes");
        assert_eq!(
            store.state.lock().unwrap().invalidated_seats,
            [SeatId::new("live")]
        );
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
                bound_epoch: None,
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

/// C4/O1 over a real store: a daemon restart resumes a higher host epoch of
/// the same Herdr boot and incarnation, so an open registered binding lags the
/// publication. The carry-forward moves it in place.
mod carry_forward {
    use super::*;
    use crate::ports::{
        DurableWorkAdmission, GuardedSeatTransition, PublishedSnapshot, ReconciliationAction,
        ReconciliationOutcome, SnapshotHeader,
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

    struct Fixture {
        context: StoreContext,
        conn: rusqlite::Connection,
        budget: CallBudget,
        path: std::path::PathBuf,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    type BindingRow = (
        i64,
        i64,
        String,
        String,
        String,
        String,
        i64,
        i64,
        String,
        String,
        i64,
    );
    const BINDING_SQL: &str = "SELECT host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,registered_at,observed_at,terminal_id,incarnation,generation FROM occupant_bindings WHERE seat_id='s' AND ordinal=1";
    fn binding(conn: &rusqlite::Connection) -> BindingRow {
        conn.query_row(BINDING_SQL, [], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
                r.get(8)?,
                r.get(9)?,
                r.get(10)?,
            ))
        })
        .unwrap()
    }

    fn production(epoch: u64, sequence: u64, incarnation: &str) -> HostSnapshot {
        let mut observation = target("pane", "terminal-a", None, sequence);
        observation.provenance = ObservationProvenance::CoherentEnumeration;
        observation.occupant = None;
        observation.ui = HostUiState::Unknown;
        observation.occupancy = StructuralOccupancy::Unknown;
        observation.execution = ExecutionEvidence::Unknown;
        observation.epoch = epoch;
        observation.incarnation = IncarnationEvidence::Verified {
            identity: incarnation.into(),
            evidence_kind: EvidenceKind::CoherentEnumeration,
        };
        let mut capture = snapshot(sequence, vec![observation]);
        capture.epoch = epoch;
        capture.incarnation = IncarnationEvidence::Verified {
            identity: incarnation.into(),
            evidence_kind: EvidenceKind::CoherentEnumeration,
        };
        capture
    }

    fn publish(f: &mut Fixture, capture: HostSnapshot) -> PublishedSnapshot {
        let admission =
            seats::begin_host_observation(&f.context, &mut f.conn, "i", &f.budget).unwrap();
        let header = SnapshotHeader::from_captured(admission, &capture).unwrap();
        let stage =
            seats::begin_snapshot_stage(&f.context, &mut f.conn, header, &f.budget).unwrap();
        seats::stage_snapshot_targets(
            &f.context,
            &mut f.conn,
            &stage.id,
            0,
            &capture.targets,
            DurableWorkAdmission::new(16).unwrap(),
            &f.budget,
        )
        .unwrap();
        seats::seal_snapshot_stage(&f.context, &mut f.conn, &stage.id, &f.budget).unwrap();
        seats::publish_snapshot_stage(&f.context, &mut f.conn, &stage.id, &f.budget).unwrap()
    }

    /// Seat `s` on `pane`/`terminal-a`, host epoch 3, open registered binding
    /// of `provenance` at epoch 3. `unresolved` makes it host-invalidated the
    /// way `mark_seat_unresolved` leaves a seat: generation bumped, binding
    /// ended.
    fn fixture(provenance: &str, unresolved: bool) -> Fixture {
        let path = std::env::temp_dir().join(format!("herdr-carry-{}.db", uuid::Uuid::new_v4()));
        let context = StoreContext::new(path.clone(), Arc::new(FixedClock));
        let conn = context.open_writer().unwrap();
        let budget = CallBudget {
            deadline: MonoInstant(1_000),
            cancellation: Cancellation::default(),
        };
        conn.execute(
            "INSERT INTO host_instances(id,created_at,host_boot,host_epoch) VALUES ('i',0,'boot-a',3)",
            [],
        )
        .unwrap();
        let kind = structural_proof("pane", "terminal-a", 1).source_spelling();
        let (state, reason) = if unresolved {
            ("unresolved", Some("host_invalidation"))
        } else {
            ("resolved", None)
        };
        conn.execute(
            "INSERT INTO seats(id,instance_id,state,unresolved_reason,role,target_id,generation,target_generation,structural_terminal_id,structural_incarnation,structural_incarnation_kind,structural_host_boot,structural_host_epoch,structural_connection_epoch,structural_observation_sequence,created_at,unavailability_open) VALUES ('s','i',?1,?2,'native','pane',?4,1,'terminal-a','inc-a',?3,'boot-a',3,1,1,0,1)",
            rusqlite::params![state, reason, kind, if unresolved { 2 } else { 1 }],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES ('s',1,1,'pane','boot-a',3,'claude','plugin_context:x','exec-x',?1,7,8,'terminal-a','inc-a')",
            [provenance],
        )
        .unwrap();
        if unresolved {
            conn.execute(
                "UPDATE occupant_bindings SET ended_at=9 WHERE seat_id='s'",
                [],
            )
            .unwrap();
        }
        Fixture {
            context,
            conn,
            budget,
            path,
        }
    }

    fn plan(f: &Fixture, publication: &PublishedSnapshot) -> Vec<GuardedSeatTransition> {
        let page =
            seats::saved_seats_page(&f.context, &f.conn, &publication.id, 0, None, 16, &f.budget)
                .unwrap();
        plan_page(&page).unwrap()
    }

    fn apply(f: &mut Fixture, transition: GuardedSeatTransition) -> ReconciliationOutcome {
        seats::apply_reconciliation_transition(&f.context, &mut f.conn, transition, &f.budget)
            .unwrap()
    }

    fn observed_generation(f: &Fixture) -> i64 {
        f.conn
            .query_row(
                "SELECT t.generation FROM snapshot_targets t JOIN host_instances h ON h.active_snapshot_id=t.generation_id WHERE t.target_id='pane'",
                [],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn assert_carried(f: &Fixture, provenance: &str, before: &BindingRow) {
        let after = binding(&f.conn);
        // Moved: epoch and structural generation.
        assert_eq!(after.0, 4);
        assert_eq!(after.1, observed_generation(f));
        // Untouched: every other column, including provenance.
        assert_eq!(after.2, before.2);
        assert_eq!(after.3, before.3);
        assert_eq!(after.4, before.4);
        assert_eq!(after.5, provenance);
        assert_eq!((after.6, after.7), (before.6, before.7));
        assert_eq!(after.8, before.8);
        assert_eq!(after.9, before.9);
        assert_eq!(after.10, before.10);
        let ended: Option<i64> = f
            .conn
            .query_row(
                "SELECT ended_at FROM occupant_bindings WHERE seat_id='s' AND ordinal=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(ended, None);
        assert_eq!(
            crate::store::schema::effective_registered_availability(&f.conn, "s", None).unwrap(),
            Some(provenance.to_owned())
        );
    }

    /// A host-invalidated seat on the same terminal is reconfirmed resolved;
    /// the invalidation ended its binding, so nothing is carried and the
    /// agent's next lifecycle check-in registers again (ht-0b8: the carry on
    /// this path could never match and was removed).
    #[test]
    fn reconfirm_structure_after_invalidation_resolves_with_no_binding_to_carry() {
        let mut f = fixture("cooperative_top_level", true);
        let publication = publish(&mut f, production(4, 3, "inc-a"));
        let transitions = plan(&f, &publication);
        assert_eq!(transitions.len(), 1);
        assert_eq!(
            transitions[0].action,
            ReconciliationAction::ReconfirmStructure {
                target: HostTargetId::new("pane"),
                terminal: TerminalId::new("terminal-a"),
            }
        );
        assert_eq!(
            apply(&mut f, transitions[0].clone()),
            ReconciliationOutcome::Applied
        );
        let state: String = f
            .conn
            .query_row("SELECT state FROM seats WHERE id='s'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(state, "resolved");
        let open: i64 = f
            .conn
            .query_row(
                "SELECT count(*) FROM occupant_bindings WHERE seat_id='s' AND ended_at IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(open, 0);
    }

    #[test]
    fn resolved_seat_with_lagging_binding_epoch_gets_carry_forward_transition() {
        let mut f = fixture("cooperative_top_level", false);
        let before = binding(&f.conn);
        let publication = publish(&mut f, production(4, 3, "inc-a"));
        let transitions = plan(&f, &publication);
        assert_eq!(transitions.len(), 1);
        assert_eq!(
            transitions[0].action,
            ReconciliationAction::CarryForward {
                target: HostTargetId::new("pane"),
                terminal: TerminalId::new("terminal-a"),
            }
        );
        assert_eq!(
            apply(&mut f, transitions[0].clone()),
            ReconciliationOutcome::Applied
        );
        assert_carried(&f, "cooperative_top_level", &before);
        let open: i64 = f
            .conn
            .query_row(
                "SELECT unavailability_open FROM seats WHERE id='s'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(open, 0, "the carried binding closes the outage marker");
        // Carried: the next publication at the same epoch plans nothing.
        let next = publish(&mut f, production(4, 4, "inc-a"));
        assert!(plan(&f, &next).is_empty());
    }

    #[test]
    fn current_binding_epoch_plans_no_carry_forward() {
        let mut f = fixture("cooperative_top_level", false);
        let publication = publish(&mut f, production(3, 3, "inc-a"));
        assert!(plan(&f, &publication).is_empty());
    }

    #[test]
    fn operator_human_binding_is_carried_forward_too() {
        let mut f = fixture("operator_human", false);
        let before = binding(&f.conn);
        let publication = publish(&mut f, production(4, 3, "inc-a"));
        let transitions = plan(&f, &publication);
        assert!(matches!(
            transitions[0].action,
            ReconciliationAction::CarryForward { .. }
        ));
        assert_eq!(
            apply(&mut f, transitions[0].clone()),
            ReconciliationOutcome::Applied
        );
        assert_carried(&f, "operator_human", &before);
    }

    #[test]
    fn incarnation_change_still_marks_unresolved_and_carries_nothing() {
        let mut f = fixture("cooperative_top_level", false);
        let publication = publish(&mut f, production(4, 3, "inc-new"));
        let transitions = plan(&f, &publication);
        assert_eq!(transitions[0].action, ReconciliationAction::MarkUnresolved);
        assert_eq!(
            apply(&mut f, transitions[0].clone()),
            ReconciliationOutcome::Applied
        );
        let state: String = f
            .conn
            .query_row("SELECT state FROM seats WHERE id='s'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(state, "unresolved");
        let (epoch, generation): (i64, i64) = f
            .conn
            .query_row(
                "SELECT host_epoch,target_generation FROM occupant_bindings WHERE seat_id='s' AND ordinal=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            (epoch, generation),
            (3, 1),
            "no carry on incarnation change"
        );
        // A forged carry in the changed incarnation is refused.
        let mut forged = transitions[0].clone();
        forged.expected_binding_generation = 1;
        forged.action = ReconciliationAction::CarryForward {
            target: HostTargetId::new("pane"),
            terminal: TerminalId::new("terminal-a"),
        };
        assert_eq!(apply(&mut f, forged), ReconciliationOutcome::Stale);
    }

    #[test]
    fn carry_forward_is_stale_when_binding_changed() {
        let mut f = fixture("cooperative_top_level", false);
        let publication = publish(&mut f, production(4, 3, "inc-a"));
        let transitions = plan(&f, &publication);
        assert!(matches!(
            transitions[0].action,
            ReconciliationAction::CarryForward { .. }
        ));
        f.conn
            .execute("UPDATE seats SET generation=2 WHERE id='s'", [])
            .unwrap();
        assert_eq!(
            apply(&mut f, transitions[0].clone()),
            ReconciliationOutcome::Stale
        );
        let epoch: i64 = f
            .conn
            .query_row(
                "SELECT host_epoch FROM occupant_bindings WHERE seat_id='s'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(epoch, 3);
    }

    fn hand_built_carry(publication: &PublishedSnapshot) -> GuardedSeatTransition {
        GuardedSeatTransition {
            publication: publication.clone(),
            seat: SeatId::new("s"),
            expected_binding_generation: 1,
            expected_target: Some(HostTargetId::new("pane")),
            expected_terminal: Some(TerminalId::new("terminal-a")),
            action: ReconciliationAction::CarryForward {
                target: HostTargetId::new("pane"),
                terminal: TerminalId::new("terminal-a"),
            },
        }
    }

    fn count(f: &Fixture, sql: &str) -> i64 {
        f.conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    // Kills: a planner or applier that carries a native binding (a different
    // provenance set from the one carry_binding_forward moves), which would
    // re-plan the seat on every pass without ever moving a row.
    #[test]
    fn native_binding_is_never_planned_for_carry_forward() {
        let mut f = fixture("verified_current_target", false);
        let publication = publish(&mut f, production(4, 3, "inc-a"));
        assert!(
            plan(&f, &publication).is_empty(),
            "a native binding yields no bound epoch, so no CarryForward"
        );
        let revision = |f: &Fixture| {
            count(
                f,
                "SELECT lifecycle_revision FROM host_instances WHERE id='i'",
            )
        };
        let before = revision(&f);
        let anchors = count(&f, "SELECT COUNT(*) FROM seat_availability");
        assert_eq!(
            apply(&mut f, hand_built_carry(&publication)),
            ReconciliationOutcome::Unchanged
        );
        assert_eq!(revision(&f), before, "an unmoved carry bumps no revision");
        assert_eq!(count(&f, "SELECT COUNT(*) FROM seat_availability"), anchors);
        assert_eq!(binding(&f.conn).0, 3, "the native binding stays put");
        // A second pass plans nothing for it either.
        let next = publish(&mut f, production(4, 4, "inc-a"));
        assert!(plan(&f, &next).is_empty());
    }

    // Kills: the planner and applier reading different provenance sets: the
    // saved-seat snapshot must expose a bound epoch only for a carried one.
    #[test]
    fn carry_forward_requires_bound_epoch_from_a_carried_provenance() {
        let mut native = fixture("verified_current_target", false);
        let publication = publish(&mut native, production(4, 3, "inc-a"));
        let page = seats::saved_seats_page(
            &native.context,
            &native.conn,
            &publication.id,
            0,
            None,
            16,
            &native.budget,
        )
        .unwrap();
        assert_eq!(page.seats[0].bound_epoch, None);
        assert!(
            page.seats[0].active_binding_execution.is_some(),
            "occupant-unavailable planning still sees the native active binding"
        );
        let mut coop = fixture("cooperative_top_level", false);
        let publication = publish(&mut coop, production(4, 3, "inc-a"));
        let mut page = seats::saved_seats_page(
            &coop.context,
            &coop.conn,
            &publication.id,
            0,
            None,
            16,
            &coop.budget,
        )
        .unwrap();
        assert_eq!(page.seats[0].bound_epoch, Some(3));
        assert!(matches!(
            plan_page(&page).unwrap()[0].action,
            ReconciliationAction::CarryForward { .. }
        ));
        page.seats[0].bound_epoch = None;
        assert!(plan_page(&page).unwrap().is_empty());
    }

    // Kills: a carry that moves the binding without the availability anchor
    // and receipt-timer job a fresh registration writes.
    #[test]
    fn carried_binding_writes_availability_anchor_and_timer_job() {
        let mut f = fixture("cooperative_top_level", false);
        f.conn.pragma_update(None, "foreign_keys", "OFF").unwrap();
        // A recipient row staged while the seat was not yet available.
        f.conn
            .execute(
                "INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot,availability_provenance) VALUES ('prep-1','t','s',1,1000,0,NULL)",
                [],
            )
            .unwrap();
        let staged = count(&f, "SELECT MAX(ordinal) FROM prepared_recipients");
        let publication = publish(&mut f, production(4, 3, "inc-a"));
        let transitions = plan(&f, &publication);
        assert_eq!(
            apply(&mut f, transitions[0].clone()),
            ReconciliationOutcome::Applied
        );
        let (generation, provenance): (i64, String) = f
            .conn
            .query_row(
                "SELECT binding_generation,observation_provenance FROM seat_availability WHERE seat_id='s'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            (generation, provenance.as_str()),
            (1, "cooperative_top_level")
        );
        assert_eq!(
            count(
                &f,
                "SELECT COUNT(*) FROM seat_availability WHERE seat_id='s'"
            ),
            1
        );
        let (subject, high_water): (String, i64) = f
            .conn
            .query_row(
                "SELECT subject_id,high_water FROM work_jobs WHERE kind='receipt_timer_materialization'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        let anchor = count(
            &f,
            "SELECT ordinal FROM seat_availability WHERE seat_id='s'",
        );
        assert_eq!(subject, anchor.to_string());
        assert_eq!(
            high_water, staged,
            "the job covers the row staged before the carry"
        );
        assert_eq!(
            count(&f, "SELECT unavailability_open FROM seats WHERE id='s'"),
            0
        );
        // Idempotent: the same transition again moves nothing and anchors nothing.
        assert_eq!(
            apply(&mut f, transitions[0].clone()),
            ReconciliationOutcome::Unchanged
        );
        assert_eq!(
            count(
                &f,
                "SELECT COUNT(*) FROM seat_availability WHERE seat_id='s'"
            ),
            1
        );
    }
}
