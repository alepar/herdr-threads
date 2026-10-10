use super::*;
use crate::{
    protocol::{
        authority::{CallerClaim, Harness, MutationPermit, ObligationRef},
        commands::{Accept, CreateThread, Invite, Leave, ThreadMutation},
        ids::*,
        results::CommandResult,
        time::{Clock, MonoInstant, UtcMillis},
    },
    store::connection::StoreContext,
};
use rusqlite::Connection;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
};

struct FixedClock(AtomicI64);
impl Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(self.0.load(Ordering::SeqCst))
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.load(Ordering::SeqCst) as u64)
    }
}

fn fixture(at: i64) -> (StoreContext, Connection, PathBuf, Arc<FixedClock>) {
    let path = std::env::temp_dir().join(format!("herdr-control-{}.db", uuid::Uuid::new_v4()));
    let clock = Arc::new(FixedClock(AtomicI64::new(at)));
    let context = StoreContext::new(path.clone(), clock.clone());
    let conn = context.open_writer().unwrap();
    conn.execute(
        "INSERT INTO host_instances(id, created_at, host_boot, host_epoch) VALUES ('i', 0, 'b', 1)",
        [],
    )
    .unwrap();
    for seat in ["s1", "s2"] {
        conn.execute("INSERT INTO seats(id, instance_id, state, role, target_id, generation, target_generation, created_at) VALUES (?1, 'i', 'resolved', 'native', ?1, 1, 1, 0)", [seat]).unwrap();
        conn.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i',?1,'b',1,1,0,'fresh','term-'||?1,'inc','coherent_enumeration',1)", [seat]).unwrap();
    }
    (context, conn, path, clock)
}

/// `fixture` plus a live, not-yet-registered cooperative occupant binding for
/// each acting seat, so cooperative permits can be issued for them.
fn bound_fixture(at: i64) -> (StoreContext, Connection, PathBuf, Arc<FixedClock>) {
    bound_fixture_recipient_harness(at, Harness::Codex)
}
fn bound_fixture_recipient_harness(
    at: i64,
    recipient: Harness,
) -> (StoreContext, Connection, PathBuf, Arc<FixedClock>) {
    let fixture = fixture(at);
    for seat in ["s1", "s2"] {
        let harness = if seat == "s2" {
            recipient
        } else {
            Harness::Codex
        };
        fixture.1.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) VALUES (?1,1,?1,'b',1,1,?2,'n','00000000-0000-4000-8000-000000000001',?3,0,'term-'||?1,'inc')", rusqlite::params![seat,harness.as_str(),harness.cooperative_provenance()]).unwrap();
    }
    fixture
}

fn snapshot_for_test(sequence: u64, targets: &[&str]) -> crate::ports::HostSnapshot {
    use crate::ports::{
        EnumerationEvidence, EvidenceKind, ExecutionEvidence, HostObservation, HostSnapshot,
        HostUiState, IncarnationEvidence, ObservationProvenance, StructuralOccupancy,
    };
    HostSnapshot {
        boot: HostBootId::new("b"),
        epoch: 1,
        observation_sequence: sequence,
        complete: true,
        enumeration: EnumerationEvidence::CoherentVerified,
        incarnation: IncarnationEvidence::Verified {
            identity: "test-incarnation".into(),
            evidence_kind: EvidenceKind::CoherentEnumeration,
        },
        targets: targets
            .iter()
            .map(|target| HostObservation {
                focused: false,
                target: HostTargetId::new(*target),
                host_boot: HostBootId::new("b"),
                epoch: 1,
                generation: 1,
                observed_at_utc: UtcMillis(100),
                observed_at_mono: MonoInstant(100),
                provenance: ObservationProvenance::CoherentEnumeration,
                occupant: None,
                ui: HostUiState::Idle,
                terminal: None,
                occupancy: StructuralOccupancy::EmptyShell,
                incarnation: IncarnationEvidence::Unknown,
                execution: ExecutionEvidence::Unknown,
                call_id: HostCallId::new(format!("capture-{target}-{sequence}")),
                connection_epoch: 1,
                observation_sequence: sequence,
                started_at_mono: MonoInstant(100),
                completed_at_mono: MonoInstant(100),
            })
            .collect(),
    }
}

fn staged_test_snapshot(
    context: &StoreContext,
    conn: &mut Connection,
    admission: crate::ports::HostObservationAdmission,
    snapshot: &crate::ports::HostSnapshot,
    budget: &crate::protocol::time::CallBudget,
) -> crate::ports::SnapshotGenerationId {
    use crate::ports::{DurableWorkAdmission, SnapshotHeader};
    use crate::store::seats;
    let header = SnapshotHeader::from_captured(admission, snapshot).unwrap();
    let stage = seats::begin_snapshot_stage(context, conn, header, budget).unwrap();
    for (chunk, targets) in snapshot.targets.chunks(16).enumerate() {
        seats::stage_snapshot_targets(
            context,
            conn,
            &stage.id,
            (chunk * 16) as u64,
            targets,
            DurableWorkAdmission::new(16).unwrap(),
            budget,
        )
        .unwrap();
    }
    seats::seal_snapshot_stage(context, conn, &stage.id, budget).unwrap();
    stage.id
}

#[test]
fn host_admission_orders_late_success_and_failure_without_false_absence() {
    use crate::ports::HostInvalidationReason;
    use crate::protocol::time::{CallBudget, Cancellation};
    use crate::store::{effective, seats};
    let (context, mut conn, path, _) = fixture(100);
    let budget = CallBudget {
        deadline: MonoInstant(1_000),
        cancellation: Cancellation::default(),
    };
    let older = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let newer = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let first = snapshot_for_test(2, &["s1"]);
    let newer_stage = staged_test_snapshot(&context, &mut conn, newer, &first, &budget);
    let publication =
        seats::publish_snapshot_stage(&context, &mut conn, &newer_stage, &budget).unwrap();
    assert!(
        seats::invalidate_host_observation(
            &context,
            &mut conn,
            &older,
            HostInvalidationReason::HostUnavailable,
            &budget,
        )
        .unwrap()
        .is_none()
    );
    assert_eq!(
        effective::effective_observation(&conn, "i", "s1")
            .unwrap()
            .unwrap()
            .source,
        effective::EffectiveObservationSource::Published
    );

    let late_success = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let newer_failure = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let second = snapshot_for_test(3, &["s1"]);
    let old_stage = staged_test_snapshot(&context, &mut conn, late_success, &second, &budget);
    let invalidation = seats::invalidate_host_observation(
        &context,
        &mut conn,
        &newer_failure,
        HostInvalidationReason::PartialEnumeration,
        &budget,
    )
    .unwrap()
    .unwrap();
    assert!(invalidation.invalidation_revision() > publication.invalidation_revision);
    assert!(
        effective::effective_observation(&conn, "i", "s1")
            .unwrap()
            .is_none()
    );
    assert_eq!(
        seats::publish_snapshot_stage(&context, &mut conn, &old_stage, &budget)
            .unwrap_err()
            .code,
        ErrorCode::StaleHostObservation
    );
    let seat_state: String = conn
        .query_row("SELECT state FROM seats WHERE id='s1'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(seat_state, "resolved");
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn saved_seats_page_counts_physical_rows_and_uses_published_index() {
    use crate::protocol::time::{CallBudget, Cancellation};
    use crate::store::seats;
    let (context, mut conn, path, _) = fixture(100);
    for index in 3..=18 {
        let seat = format!("s{index}");
        conn.execute(
            "INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES (?1,'i','resolved','native',?1,1,1,0)",
            [&seat],
        )
        .unwrap();
    }
    let budget = CallBudget {
        deadline: MonoInstant(1_000),
        cancellation: Cancellation::default(),
    };
    let admission = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let snapshot = snapshot_for_test(2, &["s1", "s2", "s3"]);
    let stage = staged_test_snapshot(&context, &mut conn, admission, &snapshot, &budget);
    let publication = seats::publish_snapshot_stage(&context, &mut conn, &stage, &budget).unwrap();
    let first = seats::saved_seats_page(&context, &conn, &stage, 0, None, 16, &budget).unwrap();
    assert_eq!(first.publication, publication);
    assert_eq!(first.visited, 16);
    assert_eq!(first.seats.len(), 16);
    assert!(first.has_more);
    assert_eq!(
        first.seats[0]
            .observed_match
            .as_ref()
            .unwrap()
            .target
            .as_str(),
        "s1"
    );
    assert!(first.seats[3].observed_match.is_none());
    let second = seats::saved_seats_page(
        &context,
        &conn,
        &stage,
        first.after_ordinal,
        Some(first.high_water_ordinal),
        16,
        &budget,
    )
    .unwrap();
    assert_eq!(second.visited, 2);
    assert!(!second.has_more);
    assert_eq!(second.seats.len(), 2);
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn discarded_stage_cleans_up_in_bounded_reopenable_quanta() {
    use crate::ports::{DurableWorkAdmission, SnapshotHeader};
    use crate::protocol::time::{CallBudget, Cancellation};
    use crate::store::seats;
    let (context, mut conn, path, _) = fixture(100);
    let budget = CallBudget {
        deadline: MonoInstant(1_000),
        cancellation: Cancellation::default(),
    };
    let names: Vec<_> = (0..17).map(|n| format!("discard-{n}")).collect();
    let names: Vec<_> = names.iter().map(String::as_str).collect();
    let snapshot = snapshot_for_test(2, &names);
    let admission = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let stage = seats::begin_snapshot_stage(
        &context,
        &mut conn,
        SnapshotHeader::from_captured(admission, &snapshot).unwrap(),
        &budget,
    )
    .unwrap();
    for (offset, chunk) in snapshot.targets.chunks(16).enumerate() {
        seats::stage_snapshot_targets(
            &context,
            &mut conn,
            &stage.id,
            (offset * 16) as u64,
            chunk,
            DurableWorkAdmission::new(16).unwrap(),
            &budget,
        )
        .unwrap();
    }
    let first = seats::discard_snapshot_stage(
        &context,
        &mut conn,
        &stage.id,
        DurableWorkAdmission::new(16).unwrap(),
        &budget,
    )
    .unwrap();
    assert_eq!(first.visited, 16);
    assert!(!first.complete);
    drop(conn);
    let mut reopened = context.open_writer().unwrap();
    let second = seats::discard_snapshot_stage(
        &context,
        &mut reopened,
        &stage.id,
        DurableWorkAdmission::new(16).unwrap(),
        &budget,
    )
    .unwrap();
    assert_eq!(second.visited, 1);
    assert!(second.complete);
    let remaining: i64 = reopened
        .query_row(
            "SELECT COUNT(*) FROM snapshot_targets WHERE generation_id=?1",
            [stage.id.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(remaining, 0);
    drop(reopened);
    let _ = std::fs::remove_file(path);
}

#[test]
fn denied_observation_marks_saved_seat_unresolved_without_retirement() {
    use crate::ports::{
        GuardedInvalidationTransition, HostInvalidationReason, ReconciliationOutcome,
    };
    use crate::protocol::time::{CallBudget, Cancellation};
    use crate::store::seats;
    let (context, mut conn, path, _) = fixture(100);
    let budget = CallBudget {
        deadline: MonoInstant(1_000),
        cancellation: Cancellation::default(),
    };
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
    let outcome = seats::mark_unresolved_from_invalidation(
        &context,
        &mut conn,
        GuardedInvalidationTransition {
            fence,
            seat: SeatId::new("s1"),
            expected_binding_generation: 1,
            expected_target: Some(HostTargetId::new("s1")),
            expected_terminal: None,
        },
        &budget,
    )
    .unwrap();
    assert_eq!(outcome, ReconciliationOutcome::Applied);
    let state: (String, Option<String>, i64) = conn
        .query_row(
            "SELECT state,target_id,generation FROM seats WHERE id='s1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(state, ("unresolved".into(), Some("s1".into()), 2));
    let retirements: i64 = conn
        .query_row("SELECT COUNT(*) FROM retirements", [], |r| r.get(0))
        .unwrap();
    assert_eq!(retirements, 0);
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn guarded_reconciliation_rejects_stale_publication_and_binding() {
    use crate::ports::{GuardedSeatTransition, ReconciliationAction, ReconciliationOutcome};
    use crate::protocol::time::{CallBudget, Cancellation};
    use crate::store::seats;
    let (context, mut conn, path, _) = fixture(100);
    let budget = CallBudget {
        deadline: MonoInstant(1_000),
        cancellation: Cancellation::default(),
    };
    let first_admission = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let first = snapshot_for_test(2, &["s1"]);
    let first_stage = staged_test_snapshot(&context, &mut conn, first_admission, &first, &budget);
    let first_publication =
        seats::publish_snapshot_stage(&context, &mut conn, &first_stage, &budget).unwrap();
    let newer_admission = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let newer = snapshot_for_test(3, &["s1"]);
    let newer_stage = staged_test_snapshot(&context, &mut conn, newer_admission, &newer, &budget);
    seats::publish_snapshot_stage(&context, &mut conn, &newer_stage, &budget).unwrap();
    let stale = seats::apply_reconciliation_transition(
        &context,
        &mut conn,
        GuardedSeatTransition {
            publication: first_publication,
            seat: SeatId::new("s1"),
            expected_binding_generation: 1,
            expected_target: Some(HostTargetId::new("s1")),
            expected_terminal: None,
            action: ReconciliationAction::MarkUnresolved,
        },
        &budget,
    )
    .unwrap();
    assert_eq!(stale, ReconciliationOutcome::Stale);
    let current =
        seats::saved_seats_page(&context, &conn, &newer_stage, 0, None, 16, &budget).unwrap();
    let stale_binding = seats::apply_reconciliation_transition(
        &context,
        &mut conn,
        GuardedSeatTransition {
            publication: current.publication,
            seat: SeatId::new("s1"),
            expected_binding_generation: 2,
            expected_target: Some(HostTargetId::new("s1")),
            expected_terminal: None,
            action: ReconciliationAction::MarkUnresolved,
        },
        &budget,
    )
    .unwrap();
    assert_eq!(stale_binding, ReconciliationOutcome::Stale);
    let state: String = conn
        .query_row("SELECT state FROM seats WHERE id='s1'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(state, "resolved");
    let current =
        seats::saved_seats_page(&context, &conn, &newer_stage, 0, None, 16, &budget).unwrap();
    assert_eq!(
        seats::apply_reconciliation_transition(
            &context,
            &mut conn,
            GuardedSeatTransition {
                publication: current.publication,
                seat: SeatId::new("s1"),
                expected_binding_generation: 1,
                expected_target: Some(HostTargetId::new("s1")),
                expected_terminal: None,
                action: ReconciliationAction::MarkUnresolved,
            },
            &budget
        )
        .unwrap(),
        ReconciliationOutcome::Applied
    );
    let cause: (Option<String>, Option<String>) = conn
        .query_row(
            "SELECT unresolved_reason,unresolved_from_generation_id FROM seats WHERE id='s1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(cause, (Some("other".into()), None));
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn invalidation_seat_page_is_bounded_and_rejects_a_newer_host_decision() {
    use crate::ports::HostInvalidationReason;
    use crate::protocol::time::{CallBudget, Cancellation};
    use crate::store::seats;
    let (context, mut conn, path, _) = fixture(100);
    let budget = CallBudget {
        deadline: MonoInstant(1_000),
        cancellation: Cancellation::default(),
    };
    let admission = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let fence = seats::invalidate_host_observation(
        &context,
        &mut conn,
        &admission,
        HostInvalidationReason::UnknownIncarnation,
        &budget,
    )
    .unwrap()
    .unwrap();
    let page =
        seats::saved_seats_page_for_invalidation(&context, &conn, &fence, 0, None, 16, &budget)
            .unwrap();
    assert_eq!(page.visited, 2);
    assert_eq!(page.seats.len(), 2);
    assert!(page.seats.iter().all(|seat| seat.observed_match.is_none()));
    let newer = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    seats::invalidate_host_observation(
        &context,
        &mut conn,
        &newer,
        HostInvalidationReason::HostUnavailable,
        &budget,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        seats::saved_seats_page_for_invalidation(
            &context,
            &conn,
            &fence,
            0,
            Some(page.high_water_ordinal),
            16,
            &budget,
        )
        .unwrap_err()
        .code,
        ErrorCode::CursorStale
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn guarded_absence_retirement_commits_one_fence_and_rejects_invalidation() {
    use crate::ports::{
        GuardedSeatTransition, HostInvalidationReason, ReconciliationAction, ReconciliationOutcome,
    };
    use crate::protocol::time::{CallBudget, Cancellation};
    use crate::store::seats;
    let (context, mut conn, path, _) = fixture(100);
    conn.execute(
        "INSERT INTO occupant_bindings(seat_id,generation,target_id,terminal_id,incarnation,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at) VALUES ('s1',1,'s1','terminal-1','test-incarnation','b',1,1,'codex','native-1','execution-1','verified_current_target',100)",
        [],
    )
    .unwrap();
    let budget = CallBudget {
        deadline: MonoInstant(1_000),
        cancellation: Cancellation::default(),
    };
    let admission = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let empty = snapshot_for_test(2, &[]);
    let stage = staged_test_snapshot(&context, &mut conn, admission, &empty, &budget);
    let publication = seats::publish_snapshot_stage(&context, &mut conn, &stage, &budget).unwrap();
    let transition = GuardedSeatTransition {
        publication,
        seat: SeatId::new("s1"),
        expected_binding_generation: 1,
        expected_target: Some(HostTargetId::new("s1")),
        expected_terminal: Some(TerminalId::new("terminal-1")),
        action: ReconciliationAction::BeginRetirement {
            absent_target: HostTargetId::new("s1"),
        },
    };
    let denied = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    seats::invalidate_host_observation(
        &context,
        &mut conn,
        &denied,
        HostInvalidationReason::HostUnavailable,
        &budget,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        seats::apply_reconciliation_transition(&context, &mut conn, transition.clone(), &budget)
            .unwrap(),
        ReconciliationOutcome::Stale
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM retirements", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let recovered = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let new_empty = snapshot_for_test(3, &[]);
    let recovered_stage = staged_test_snapshot(&context, &mut conn, recovered, &new_empty, &budget);
    let recovered_publication =
        seats::publish_snapshot_stage(&context, &mut conn, &recovered_stage, &budget).unwrap();
    let outcome = seats::apply_reconciliation_transition(
        &context,
        &mut conn,
        GuardedSeatTransition {
            publication: recovered_publication,
            ..transition
        },
        &budget,
    )
    .unwrap();
    assert!(matches!(
        outcome,
        ReconciliationOutcome::RetirementStarted(_)
    ));
    let state: (String, i64) = conn
        .query_row(
            "SELECT state,generation FROM seats WHERE id='s1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, ("retired".into(), 2));
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM retirements", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn guarded_move_preserves_binding() {
    use crate::ports::{
        EvidenceKind, ExecutionEvidence, GuardedSeatTransition, NativeOccupant,
        ReconciliationAction, ReconciliationOutcome,
    };
    use crate::protocol::time::{CallBudget, Cancellation};
    use crate::store::seats;
    let (context, mut conn, path, _) = fixture(100);
    conn.execute(
        "INSERT INTO occupant_bindings(seat_id,generation,target_id,terminal_id,incarnation,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at) VALUES ('s1',1,'s1','terminal-1','test-incarnation','b',1,1,'codex','native-1','execution-1','verified_current_target',100)",
        [],
    )
    .unwrap();
    let budget = CallBudget {
        deadline: MonoInstant(1_000),
        cancellation: Cancellation::default(),
    };
    let observed = |sequence: u64, execution: &str| {
        let mut snapshot = snapshot_for_test(sequence, &["moved-target"]);
        snapshot.targets[0].terminal = Some(TerminalId::new("terminal-1"));
        snapshot.targets[0].occupancy = crate::ports::StructuralOccupancy::Occupied;
        snapshot.targets[0].execution = ExecutionEvidence::Verified {
            execution: ExecutionId::new(execution),
            evidence_kind: EvidenceKind::NativeCurrentTarget,
        };
        snapshot.targets[0].occupant = Some(NativeOccupant {
            harness: Harness::Codex,
            session: NativeSessionId::new("native-1"),
            execution: ExecutionId::new(execution),
            is_top_level: true,
        });
        snapshot
    };
    let admission = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let first = observed(2, "execution-1");
    let stage = staged_test_snapshot(&context, &mut conn, admission, &first, &budget);
    let publication = seats::publish_snapshot_stage(&context, &mut conn, &stage, &budget).unwrap();
    let moved = seats::apply_reconciliation_transition(
        &context,
        &mut conn,
        GuardedSeatTransition {
            publication,
            seat: SeatId::new("s1"),
            expected_binding_generation: 1,
            expected_target: Some(HostTargetId::new("s1")),
            expected_terminal: Some(TerminalId::new("terminal-1")),
            action: ReconciliationAction::Move {
                target: HostTargetId::new("moved-target"),
                terminal: TerminalId::new("terminal-1"),
            },
        },
        &budget,
    )
    .unwrap();
    assert_eq!(moved, ReconciliationOutcome::Applied);
    let proof: (String, i64, i64) = conn.query_row(
        "SELECT structural_terminal_id,structural_observation_sequence,target_generation FROM seats WHERE id='s1'", [],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    ).unwrap();
    assert_eq!(proof, ("terminal-1".into(), 2, 1));
    let moved_state: (String, i64, String) = conn
        .query_row(
            "SELECT s.target_id,s.generation,b.target_id FROM seats s JOIN occupant_bindings b ON b.seat_id=s.id AND b.ended_at IS NULL WHERE s.id='s1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        moved_state,
        ("moved-target".into(), 1, "moved-target".into())
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

/// A guarded Move without a verified execution, on the same terminal, when
/// the published snapshot shows the occupant absent (an empty shell). Returns
/// the outcome and the seat's (target, generation, binding target, binding
/// ended) afterwards.
fn move_with_absent_occupant_evidence(
    registered: bool,
) -> (
    crate::ports::ReconciliationOutcome,
    (String, i64, String, Option<i64>),
) {
    use crate::ports::{GuardedSeatTransition, ReconciliationAction, StructuralOccupancy};
    use crate::protocol::time::{CallBudget, Cancellation};
    use crate::store::seats;
    let (context, mut conn, path, _) = fixture(100);
    conn.execute(
        "INSERT INTO occupant_bindings(seat_id,generation,target_id,terminal_id,incarnation,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,registered_at) VALUES ('s1',1,'s1','terminal-1','test-incarnation','b',1,1,'codex','native-1','execution-1','verified_current_target',100,?1)",
        [registered.then_some(100_i64)],
    )
    .unwrap();
    let budget = CallBudget {
        deadline: MonoInstant(1_000),
        cancellation: Cancellation::default(),
    };
    // Same terminal at a moved address, no verified execution, and positive
    // absence evidence: an empty shell with no top-level occupant.
    let mut absent = snapshot_for_test(2, &["moved-target"]);
    absent.targets[0].terminal = Some(TerminalId::new("terminal-1"));
    absent.targets[0].occupancy = StructuralOccupancy::EmptyShell;
    let admission = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let stage = staged_test_snapshot(&context, &mut conn, admission, &absent, &budget);
    let publication = seats::publish_snapshot_stage(&context, &mut conn, &stage, &budget).unwrap();
    let outcome = seats::apply_reconciliation_transition(
        &context,
        &mut conn,
        GuardedSeatTransition {
            publication,
            seat: SeatId::new("s1"),
            expected_binding_generation: 1,
            expected_target: Some(HostTargetId::new("s1")),
            expected_terminal: Some(TerminalId::new("terminal-1")),
            action: ReconciliationAction::Move {
                target: HostTargetId::new("moved-target"),
                terminal: TerminalId::new("terminal-1"),
            },
        },
        &budget,
    )
    .unwrap();
    let state = conn
        .query_row(
            "SELECT s.target_id,s.generation,b.target_id,b.ended_at FROM seats s JOIN occupant_bindings b ON b.seat_id=s.id WHERE s.id='s1' ORDER BY b.ordinal DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    drop(conn);
    let _ = std::fs::remove_file(path);
    (outcome, state)
}

/// W5-4: a registered binding is never carried by a Move when the snapshot
/// positively shows its occupant absent. The seat keeps its old target and
/// its live binding. Kills: dropping `registered && shows_occupant_absent()`
/// from the Move arm's unknown-execution branch.
#[test]
fn guarded_move_refuses_registered_binding_with_absent_occupant_evidence() {
    let (outcome, state) = move_with_absent_occupant_evidence(true);
    assert_eq!(outcome, crate::ports::ReconciliationOutcome::Stale);
    assert_eq!(state, ("s1".into(), 1, "s1".into(), None));
}

/// W5-4: an unregistered binding (never checked in) carries no registration
/// that absence could contradict, so the same-terminal Move in the same
/// verified incarnation carries the seat, its generation and its binding to
/// the new address (unknown execution never unseats a binding, D2). Pins
/// that documented outcome.
#[test]
fn guarded_move_carries_unregistered_binding_despite_absent_occupant_evidence() {
    let (outcome, state) = move_with_absent_occupant_evidence(false);
    assert_eq!(outcome, crate::ports::ReconciliationOutcome::Applied);
    assert_eq!(
        state,
        ("moved-target".into(), 1, "moved-target".into(), None)
    );
}

#[test]
fn first_published_baseline_hold_survives_later_snapshots_and_operator_release() {
    use crate::ports::{OperatorRequest, OperatorTargetGuard};
    use crate::protocol::{
        authority::{OperatorActor, PeerIdentity},
        commands::{OperatorCommand, OperatorFreshSeat},
        time::{CallBudget, Cancellation},
    };
    use crate::store::{effective, seats};
    let (context, mut conn, path, _) = fixture(100);
    conn.execute(
        "UPDATE seats SET state='unresolved',target_id=NULL WHERE id='s1'",
        [],
    )
    .unwrap();
    let budget = CallBudget {
        deadline: MonoInstant(1_000),
        cancellation: Cancellation::default(),
    };
    let admission = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let first = snapshot_for_test(1, &["new-target"]);
    let baseline = staged_test_snapshot(&context, &mut conn, admission, &first, &budget);
    seats::publish_snapshot_stage(&context, &mut conn, &baseline, &budget).unwrap();
    assert_eq!(
        effective::effective_recovery_disposition(&conn, "i", "new-target").unwrap(),
        effective::EffectiveRecoveryDisposition::BaselineHeld
    );
    let admission = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let second = snapshot_for_test(2, &["new-target", "later-target"]);
    let stage = staged_test_snapshot(&context, &mut conn, admission, &second, &budget);
    seats::publish_snapshot_stage(&context, &mut conn, &stage, &budget).unwrap();
    let retained: String = conn
        .query_row(
            "SELECT recovery_baseline_generation_id FROM host_instances WHERE id='i'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(retained, baseline.as_str());
    assert_eq!(
        effective::effective_recovery_disposition(&conn, "i", "new-target").unwrap(),
        effective::EffectiveRecoveryDisposition::BaselineHeld
    );
    assert_eq!(
        effective::effective_recovery_disposition(&conn, "i", "later-target").unwrap(),
        effective::EffectiveRecoveryDisposition::CreatedAfterBaseline
    );
    let mut fresh = first.targets[0].clone();
    fresh.provenance = crate::ports::ObservationProvenance::FreshCurrentTarget;
    fresh.observation_sequence = 3;
    conn.execute(
        "INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','new-target','b',1,1,3,100,'fresh','term-'||'new-target','inc','coherent_enumeration',1)",
        [],
    )
    .unwrap();
    let command = OperatorFreshSeat {
        target: HostTargetId::new("new-target"),
        operation: OperationId::new("release-baseline"),
    };
    let guard =
        OperatorTargetGuard::try_new("i", &OperatorCommand::FreshSeat(command.clone()), fresh)
            .unwrap();
    let actor = OperatorActor::from_peer(PeerIdentity::from_kernel(501), 501).unwrap();
    let CommandResult::OperatorFreshSeat(new_seat) = seats::mutate_operator(
        &context,
        &mut conn,
        "i",
        OperatorRequest::FreshSeat(command, guard),
        actor,
        None,
    )
    .unwrap() else {
        panic!("operator fresh seat result missing")
    };
    let release_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM recovery_baseline_releases WHERE instance_id='i' AND baseline_generation_id=?1 AND target_id='new-target'",
            [baseline.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(release_count, 1);
    conn.execute(
        "UPDATE seats SET state='retired',retired_at=100 WHERE id=?1",
        [new_seat.as_str()],
    )
    .unwrap();
    assert_eq!(
        effective::effective_recovery_disposition(&conn, "i", "new-target").unwrap(),
        effective::EffectiveRecoveryDisposition::Released
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn large_snapshot_resumes_after_reopen_and_duplicate_rows_roll_back_the_quantum() {
    use crate::ports::{DurableWorkAdmission, SnapshotHeader};
    use crate::protocol::time::{CallBudget, Cancellation};
    use crate::store::seats;
    let (context, mut conn, path, _) = fixture(100);
    let budget = CallBudget {
        deadline: MonoInstant(1_000),
        cancellation: Cancellation::default(),
    };
    let names: Vec<_> = (0..205).map(|n| format!("large-{n}")).collect();
    let names: Vec<_> = names.iter().map(String::as_str).collect();
    let snapshot = snapshot_for_test(2, &names);
    let admission = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let stage = seats::begin_snapshot_stage(
        &context,
        &mut conn,
        SnapshotHeader::from_captured(admission, &snapshot).unwrap(),
        &budget,
    )
    .unwrap();
    for (chunk, targets) in snapshot.targets.chunks(16).enumerate() {
        let progress = seats::stage_snapshot_targets(
            &context,
            &mut conn,
            &stage.id,
            (chunk * 16) as u64,
            targets,
            DurableWorkAdmission::new(16).unwrap(),
            &budget,
        )
        .unwrap();
        assert_eq!(progress.visited as usize, targets.len());
        if chunk == 2 {
            drop(conn);
            conn = context.open_writer().unwrap();
        }
    }
    seats::seal_snapshot_stage(&context, &mut conn, &stage.id, &budget).unwrap();
    let published = seats::publish_snapshot_stage(&context, &mut conn, &stage.id, &budget).unwrap();
    assert_eq!(published.target_count, 205);
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM snapshot_targets WHERE generation_id=?1",
            [stage.id.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 205);

    let duplicate = snapshot_for_test(3, &["duplicate", "duplicate"]);
    let admission = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let duplicate_stage = seats::begin_snapshot_stage(
        &context,
        &mut conn,
        SnapshotHeader::from_captured(admission, &duplicate).unwrap(),
        &budget,
    )
    .unwrap();
    assert_eq!(
        seats::stage_snapshot_targets(
            &context,
            &mut conn,
            &duplicate_stage.id,
            0,
            &duplicate.targets,
            DurableWorkAdmission::new(16).unwrap(),
            &budget,
        )
        .unwrap_err()
        .code,
        ErrorCode::InvalidRequest
    );
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM snapshot_targets WHERE generation_id=?1",
            [duplicate_stage.id.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    let mut duplicate_terminal = snapshot_for_test(4, &["one", "two"]);
    duplicate_terminal.targets[0].terminal = Some(TerminalId::new("same-terminal"));
    duplicate_terminal.targets[1].terminal = Some(TerminalId::new("same-terminal"));
    let admission = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let terminal_stage = seats::begin_snapshot_stage(
        &context,
        &mut conn,
        SnapshotHeader::from_captured(admission, &duplicate_terminal).unwrap(),
        &budget,
    )
    .unwrap();
    assert_eq!(
        seats::stage_snapshot_targets(
            &context,
            &mut conn,
            &terminal_stage.id,
            0,
            &duplicate_terminal.targets,
            DurableWorkAdmission::new(16).unwrap(),
            &budget,
        )
        .unwrap_err()
        .code,
        ErrorCode::InvalidRequest
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn old_fresh_row_cannot_authorize_direct_retirement_after_empty_publication() {
    use crate::ports::ClosureEvidence;
    use crate::protocol::time::{CallBudget, Cancellation};
    use crate::store::{control, seats};
    let (context, mut conn, path, _) = fixture(100);
    let budget = CallBudget {
        deadline: MonoInstant(1_000),
        cancellation: Cancellation::default(),
    };
    let admission = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let empty = snapshot_for_test(2, &[]);
    let stage = staged_test_snapshot(&context, &mut conn, admission, &empty, &budget);
    seats::publish_snapshot_stage(&context, &mut conn, &stage, &budget).unwrap();
    assert_eq!(
        control::begin_retirement(
            &context,
            &mut conn,
            SeatId::new("s1"),
            ClosureEvidence {
                host_boot: HostBootId::new("b"),
                epoch: 1,
                target: HostTargetId::new("s1"),
                generation: 1,
            },
        )
        .unwrap_err()
        .code,
        ErrorCode::StaleHostObservation
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM retirements", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn invalidated_seat_reconfirms_moved_terminal_without_restoring_registration() {
    use crate::ports::{
        EvidenceKind, ExecutionEvidence, GuardedInvalidationTransition, GuardedSeatTransition,
        HostInvalidationReason, NativeOccupant, ReconciliationAction, ReconciliationOutcome,
    };
    use crate::protocol::time::{CallBudget, Cancellation};
    use crate::store::seats;
    let (context, mut conn, path, _) = fixture(100);
    conn.execute(
        "INSERT INTO occupant_bindings(seat_id,generation,target_id,terminal_id,incarnation,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,registered_at) VALUES ('s1',1,'s1','terminal-1','test-incarnation','b',1,1,'codex','native-1','execution-1','verified_current_target',100,100)",
        [],
    )
    .unwrap();
    let budget = CallBudget {
        deadline: MonoInstant(1_000),
        cancellation: Cancellation::default(),
    };
    let observed = |sequence: u64, target: &str, execution: &str| {
        let mut snapshot = snapshot_for_test(sequence, &[target]);
        snapshot.targets[0].terminal = Some(TerminalId::new("terminal-1"));
        snapshot.targets[0].occupancy = crate::ports::StructuralOccupancy::Occupied;
        snapshot.targets[0].execution = ExecutionEvidence::Verified {
            execution: ExecutionId::new(execution),
            evidence_kind: EvidenceKind::NativeCurrentTarget,
        };
        snapshot.targets[0].occupant = Some(NativeOccupant {
            harness: Harness::Codex,
            session: NativeSessionId::new("native-1"),
            execution: ExecutionId::new(execution),
            is_top_level: true,
        });
        snapshot
    };
    let admission = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let first = observed(2, "s1", "execution-1");
    let stage = staged_test_snapshot(&context, &mut conn, admission, &first, &budget);
    seats::publish_snapshot_stage(&context, &mut conn, &stage, &budget).unwrap();
    let failed = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let fence = seats::invalidate_host_observation(
        &context,
        &mut conn,
        &failed,
        HostInvalidationReason::HostUnavailable,
        &budget,
    )
    .unwrap()
    .unwrap();
    let outcome = seats::mark_unresolved_from_invalidation(
        &context,
        &mut conn,
        GuardedInvalidationTransition {
            fence,
            seat: SeatId::new("s1"),
            expected_binding_generation: 1,
            expected_target: Some(HostTargetId::new("s1")),
            expected_terminal: Some(TerminalId::new("terminal-1")),
        },
        &budget,
    )
    .unwrap();
    assert_eq!(outcome, ReconciliationOutcome::Applied);
    let held: (String, Option<String>, i64) = conn
        .query_row(
            "SELECT state,unresolved_reason,generation FROM seats WHERE id='s1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        held,
        ("unresolved".into(), Some("host_invalidation".into()), 2)
    );
    let admission = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let moved = observed(3, "moved-target", "execution-2");
    let stage = staged_test_snapshot(&context, &mut conn, admission, &moved, &budget);
    let publication = seats::publish_snapshot_stage(&context, &mut conn, &stage, &budget).unwrap();
    let saved = seats::saved_seats_page(&context, &conn, &stage, 0, None, 16, &budget).unwrap();
    let old = saved
        .seats
        .iter()
        .find(|seat| seat.seat.as_str() == "s1")
        .unwrap();
    assert_eq!(old.target.as_ref().unwrap().as_str(), "s1");
    assert_eq!(old.terminal.as_ref().unwrap().as_str(), "terminal-1");
    assert_eq!(
        old.observed_match.as_ref().unwrap().target.as_str(),
        "moved-target"
    );
    let transition = GuardedSeatTransition {
        publication,
        seat: SeatId::new("s1"),
        expected_binding_generation: 2,
        expected_target: Some(HostTargetId::new("s1")),
        expected_terminal: Some(TerminalId::new("terminal-1")),
        action: ReconciliationAction::ReconfirmStructure {
            target: HostTargetId::new("moved-target"),
            terminal: TerminalId::new("terminal-1"),
        },
    };
    conn.execute(
        "UPDATE seats SET unresolved_reason='other',unresolved_from_generation_id=NULL,unresolved_prior_binding_generation=NULL WHERE id='s1'",
        [],
    )
    .unwrap();
    assert_eq!(
        seats::apply_reconciliation_transition(&context, &mut conn, transition.clone(), &budget)
            .unwrap(),
        ReconciliationOutcome::Stale
    );
    conn.execute(
        "UPDATE seats SET unresolved_reason='host_invalidation',unresolved_from_generation_id=?1,unresolved_prior_binding_generation=?2 WHERE id='s1'",
        rusqlite::params![old.prior_published_observation.as_ref().unwrap().generation_id.as_str(), 1],
    )
    .unwrap();
    let outcome =
        seats::apply_reconciliation_transition(&context, &mut conn, transition, &budget).unwrap();
    assert_eq!(outcome, ReconciliationOutcome::Applied);
    let resolved: (String, Option<String>, String, i64) = conn
        .query_row(
            "SELECT state,unresolved_reason,target_id,generation FROM seats WHERE id='s1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        resolved,
        ("resolved".into(), None, "moved-target".into(), 2)
    );
    let active: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM occupant_bindings WHERE seat_id='s1' AND ended_at IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(active, 0);
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn fresh_coherent_absence_retires_a_host_invalidated_seat() {
    use crate::ports::{
        GuardedInvalidationTransition, GuardedSeatTransition, HostInvalidationReason,
        ReconciliationAction, ReconciliationOutcome,
    };
    use crate::protocol::time::{CallBudget, Cancellation};
    use crate::store::seats;
    let (context, mut conn, path, _) = fixture(100);
    conn.execute(
        "INSERT INTO occupant_bindings(seat_id,generation,target_id,terminal_id,incarnation,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at) VALUES ('s1',1,'s1','terminal-1','test-incarnation','b',1,1,'codex','native-1','execution-1','verified_current_target',100)",
        [],
    ).unwrap();
    let budget = CallBudget {
        deadline: MonoInstant(1_000),
        cancellation: Cancellation::default(),
    };
    let admission = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let mut first = snapshot_for_test(2, &["s1"]);
    first.targets[0].terminal = Some(TerminalId::new("terminal-1"));
    let stage = staged_test_snapshot(&context, &mut conn, admission, &first, &budget);
    seats::publish_snapshot_stage(&context, &mut conn, &stage, &budget).unwrap();
    let failed = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let fence = seats::invalidate_host_observation(
        &context,
        &mut conn,
        &failed,
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
                seat: SeatId::new("s1"),
                expected_binding_generation: 1,
                expected_target: Some(HostTargetId::new("s1")),
                expected_terminal: Some(TerminalId::new("terminal-1")),
            },
            &budget
        )
        .unwrap(),
        ReconciliationOutcome::Applied
    );
    let admission = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let empty = snapshot_for_test(3, &[]);
    let stage = staged_test_snapshot(&context, &mut conn, admission, &empty, &budget);
    let publication = seats::publish_snapshot_stage(&context, &mut conn, &stage, &budget).unwrap();
    let outcome = seats::apply_reconciliation_transition(
        &context,
        &mut conn,
        GuardedSeatTransition {
            publication,
            seat: SeatId::new("s1"),
            expected_binding_generation: 2,
            expected_target: Some(HostTargetId::new("s1")),
            expected_terminal: Some(TerminalId::new("terminal-1")),
            action: ReconciliationAction::BeginRetirement {
                absent_target: HostTargetId::new("s1"),
            },
        },
        &budget,
    )
    .unwrap();
    assert!(matches!(
        outcome,
        ReconciliationOutcome::RetirementStarted(_)
    ));
    let state: (String, Option<String>, Option<String>) = conn
        .query_row(
            "SELECT state,unresolved_reason,unresolved_from_generation_id FROM seats WHERE id='s1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(state, ("retired".into(), None, None));
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn snapshot_stage_is_hidden_bounded_and_allows_a_foreground_writer() {
    use crate::ports::{
        DurableWorkAdmission, EnumerationEvidence, EvidenceKind, ExecutionEvidence,
        HostObservation, HostSnapshot, HostUiState, IncarnationEvidence, ObservationProvenance,
        SnapshotHeader, StructuralOccupancy,
    };
    use crate::protocol::time::{CallBudget, Cancellation};
    use crate::store::{effective, seats};

    let (context, mut conn, path, _) = fixture(100);
    let budget = CallBudget {
        deadline: MonoInstant(1_000),
        cancellation: Cancellation::default(),
    };
    let targets: Vec<_> = (0..17)
        .map(|index| HostObservation {
            focused: false,
            target: HostTargetId::new(format!("stage-{index}")),
            host_boot: HostBootId::new("b"),
            epoch: 1,
            generation: 1,
            observed_at_utc: UtcMillis(100),
            observed_at_mono: MonoInstant(100),
            provenance: ObservationProvenance::CoherentEnumeration,
            occupant: None,
            ui: HostUiState::Idle,
            terminal: None,
            occupancy: StructuralOccupancy::EmptyShell,
            incarnation: IncarnationEvidence::Unknown,
            execution: ExecutionEvidence::Unknown,
            call_id: HostCallId::new(format!("stage-call-{index}")),
            connection_epoch: 1,
            observation_sequence: 2,
            started_at_mono: MonoInstant(100),
            completed_at_mono: MonoInstant(100),
        })
        .collect();
    let snapshot = HostSnapshot {
        boot: HostBootId::new("b"),
        epoch: 1,
        observation_sequence: 2,
        complete: true,
        enumeration: EnumerationEvidence::CoherentVerified,
        incarnation: IncarnationEvidence::Verified {
            identity: "stage-incarnation".into(),
            evidence_kind: EvidenceKind::CoherentEnumeration,
        },
        targets: targets.clone(),
    };
    let admission = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let header = SnapshotHeader::from_captured(admission, &snapshot).unwrap();
    let stage = seats::begin_snapshot_stage(&context, &mut conn, header, &budget).unwrap();
    let admission = DurableWorkAdmission::new(16).unwrap();
    let first = seats::stage_snapshot_targets(
        &context,
        &mut conn,
        &stage.id,
        0,
        &targets[..16],
        admission,
        &budget,
    )
    .unwrap();
    assert_eq!(first.visited, 16);
    assert_eq!(first.stage.staged_targets, 16);
    assert_eq!(
        seats::seal_snapshot_stage(&context, &mut conn, &stage.id, &budget)
            .unwrap_err()
            .code,
        ErrorCode::StaleHostObservation
    );
    assert!(
        effective::effective_observation(&conn, "i", "stage-0")
            .unwrap()
            .is_none()
    );
    let foreground = context.open_writer().unwrap();
    foreground
        .execute(
            "UPDATE host_instances SET duration_config_revision=duration_config_revision+1 WHERE id='i'",
            [],
        )
        .unwrap();
    let second = seats::stage_snapshot_targets(
        &context,
        &mut conn,
        &stage.id,
        16,
        &targets[16..],
        admission,
        &budget,
    )
    .unwrap();
    assert_eq!(second.visited, 1);
    seats::seal_snapshot_stage(&context, &mut conn, &stage.id, &budget).unwrap();
    assert!(
        effective::effective_observation(&conn, "i", "stage-0")
            .unwrap()
            .is_none()
    );
    let published = seats::publish_snapshot_stage(&context, &mut conn, &stage.id, &budget).unwrap();
    assert_eq!(published.target_count, 17);
    assert_eq!(
        effective::effective_observation(&conn, "i", "stage-0")
            .unwrap()
            .unwrap()
            .structural_generation,
        1
    );
    drop(foreground);
    drop(conn);
    let _ = std::fs::remove_file(path);
}

fn claim(seat: &str) -> CallerClaim {
    CallerClaim {
        instance: "i".into(),
        seat: SeatId::new(seat),
        binding_generation: 1,
        role: crate::protocol::authority::CallerRole::TopLevel,
        harness: Harness::Codex,
        native_session: NativeSessionId::new("n"),
        execution: ExecutionId::new("00000000-0000-4000-8000-000000000001"),
        target: HostTargetId::new(seat),
    }
}
fn permit(
    seat: &str,
    operation: &str,
    obligation: ObligationRef,
    hash: [u8; 32],
    at: i64,
) -> MutationPermit {
    MutationPermit::cooperative(
        claim(seat),
        OperationId::new(operation),
        obligation,
        hash,
        MonoInstant(at as u64),
        (1, 0),
        crate::protocol::time::CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: Default::default(),
        },
    )
}

struct GraphGate(ServiceAuthorId);
struct GraphGuard(ServiceAuthorId);
impl crate::ports::ServiceDecisionGuard for GraphGuard {
    fn author(&self) -> &ServiceAuthorId {
        &self.0
    }
}
impl crate::ports::ServiceAuthorityGate for GraphGate {
    fn register(
        &self,
        instance: &str,
        boot: &str,
        author: ServiceAuthorId,
    ) -> Result<crate::ports::ServiceConnectionAuthority, crate::protocol::results::ApiError> {
        Ok(crate::ports::ServiceConnectionAuthority::new(
            instance.into(),
            boot.into(),
            1,
            author,
        ))
    }
    fn decision_guard<'a>(
        &'a self,
        _proof: &crate::ports::ServiceWriteTransactionProof,
        connection: &crate::ports::ServiceConnectionAuthority,
    ) -> Result<Box<dyn crate::ports::ServiceDecisionGuard + 'a>, crate::protocol::results::ApiError>
    {
        if connection.author() != &self.0 {
            return Err(api_error(
                ErrorCode::StaleServiceGeneration,
                "wrong service author",
            ));
        }
        Ok(Box::new(GraphGuard(self.0.clone())))
    }
    fn revoke_exact(&self, _connection: &crate::ports::ServiceConnectionAuthority) -> bool {
        true
    }
}

#[test]
fn required_then_independent_ordinary_acceptance_keeps_one_join_interval() {
    use crate::ports::ServiceAuthorityGate;
    use crate::protocol::{commands::AcceptRequired, service::*};
    use sha2::Digest;
    let (context, mut conn, path, clock) = bound_fixture(100);
    let budget = crate::protocol::time::CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    };
    let author = ServiceAuthorId::new(format!("graph:{:x}", sha2::Sha256::digest(b"i")));
    let gate = GraphGate(author.clone());
    let connection = gate.register("i", "boot", author).unwrap();
    let operate = |conn: &mut Connection, operation: ServiceOperation| {
        crate::store::service_controls::operate(
            &context,
            conn,
            "i",
            operation,
            &connection,
            &gate,
            &budget,
            None,
        )
        .unwrap()
    };
    let thread = ThreadId::new("independent-two-acceptances");
    operate(
        &mut conn,
        ServiceOperation::EnsureThread(EnsureManagedThread {
            thread: thread.clone(),
            topic: "topic".into(),
            goal: "goal".into(),
            operation: OperationId::new("ensure-two-acceptances"),
        }),
    );
    let ServiceResult::Invitation(required) = operate(
        &mut conn,
        ServiceOperation::Invite(ServiceInvite {
            thread: thread.clone(),
            seat: SeatId::new("s2"),
            constraint: InvitationConstraint::Required,
            deadline_millis: Some(500),
            operation: OperationId::new("required-two-acceptances"),
        }),
    ) else {
        panic!()
    };
    let ServiceResult::Invitation(ordinary) = operate(
        &mut conn,
        ServiceOperation::Invite(ServiceInvite {
            thread: thread.clone(),
            seat: SeatId::new("s2"),
            constraint: InvitationConstraint::Ordinary,
            deadline_millis: Some(15),
            operation: OperationId::new("ordinary-two-acceptances"),
        }),
    ) else {
        panic!()
    };
    assert_ne!(required.invitation, ordinary.invitation);
    let requirement = required.requirement.unwrap();
    clock.0.store(110, Ordering::SeqCst);
    let required_command = AcceptRequired {
        thread: thread.clone(),
        invitation: required.invitation.clone(),
        requirement: requirement.requirement,
        expected_revision: requirement.revision,
        operation: OperationId::new("accept-required-two"),
        claim: CallerClaim {
            seat: SeatId::new("s2"),
            ..claim("s2")
        },
    };
    let required_hash = cooperative_payload_hash("accept_required", &required_command).unwrap();
    assert!(matches!(
        accept_required(
            &context,
            &mut conn,
            &budget,
            &required_command,
            permit(
                "s2",
                "accept-required-two",
                ObligationRef::Invitation(required.invitation.clone()),
                required_hash,
                110
            )
        )
        .unwrap(),
        CommandResult::RequiredAccepted(_)
    ));
    assert_eq!(
        conn.query_row(
            "SELECT state,accepted_at,accepted_actor_seat_id FROM invitations WHERE id=?1",
            [required.invitation.as_str()],
            |r| Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?
            ))
        )
        .unwrap(),
        ("accepted".into(), 110, "s2".into())
    );
    let original:(i64,i64,i64)=conn.query_row("SELECT episode,joined_seq,(SELECT joined_at FROM memberships WHERE thread_id=?1 AND seat_id='s2') FROM membership_intervals WHERE thread_id=?1 AND seat_id='s2' AND left_seq IS NULL",[thread.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT state FROM invitations WHERE id=?1",
            [ordinary.invitation.as_str()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "pending"
    );
    let ServiceResult::Membership(membership) = operate(
        &mut conn,
        ServiceOperation::Membership(ServiceMembershipQuery {
            thread: thread.clone(),
            seat: Some(SeatId::new("s2")),
            page: Default::default(),
        }),
    ) else {
        panic!()
    };
    assert_eq!(
        membership.items[0].voluntary_state,
        VoluntaryMembershipState::Joined
    );
    let directory =
        crate::protocol::commands::Command::Directory(crate::protocol::commands::DirectoryQuery {
            recent: false,
            membership: Some(SeatId::new("s2")),
            membership_filter: crate::protocol::commands::DirectoryMembership::Joined,
            topic_contains: None,
            page: Default::default(),
        });
    let CommandResult::Directory(threads) =
        crate::store::queries::query(&context, "i", &directory, &budget).unwrap()
    else {
        panic!()
    };
    assert!(threads.items.iter().any(|item| item.thread == thread));
    let inbox = crate::protocol::commands::Command::Inbox(crate::protocol::commands::InboxQuery {
        seat: Some(SeatId::new("s2")),
        page: Default::default(),
    });
    let CommandResult::Inbox(before) =
        crate::store::queries::query(&context, "i", &inbox, &budget).unwrap()
    else {
        panic!()
    };
    assert_eq!(
        before
            .items
            .iter()
            .find(|item| item.thread == thread)
            .unwrap()
            .invitations,
        1
    );
    clock.0.store(120, Ordering::SeqCst);
    let due =
        crate::store::invitation_due::scan_invitation_due_batch(&context, &mut conn, None, 10)
            .unwrap();
    assert_eq!(due.warnings_added, 1);
    assert!(
        conn.query_row(
            "SELECT warning_message_id IS NOT NULL FROM invitations WHERE id=?1",
            [ordinary.invitation.as_str()],
            |r| r.get::<_, bool>(0)
        )
        .unwrap()
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM receipts WHERE thread_id=?1",
            [thread.as_str()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    clock.0.store(130, Ordering::SeqCst);
    let ordinary_command = Accept {
        thread: thread.clone(),
        operation: OperationId::new("accept-ordinary-two"),
        claim: claim("s2"),
    };
    let ordinary_hash = cooperative_payload_hash("accept", &ordinary_command).unwrap();
    assert_eq!(
        accept(
            &context,
            &mut conn,
            &budget,
            &ordinary_command,
            permit(
                "s2",
                "accept-ordinary-two",
                ObligationRef::Invitation(ordinary.invitation.clone()),
                ordinary_hash,
                130
            )
        )
        .unwrap(),
        CommandResult::Accepted(ordinary.invitation.clone().into())
    );
    assert_eq!(conn.query_row("SELECT count(*) FROM membership_intervals WHERE thread_id=?1 AND seat_id='s2' AND left_seq IS NULL",[thread.as_str()],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(conn.query_row("SELECT episode,joined_seq,(SELECT joined_at FROM memberships WHERE thread_id=?1 AND seat_id='s2') FROM membership_intervals WHERE thread_id=?1 AND seat_id='s2' AND left_seq IS NULL",[thread.as_str()],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?))).unwrap(),original);
    assert_eq!(
        conn.query_row(
            "SELECT state,accepted_at,accepted_actor_seat_id FROM invitations WHERE id=?1",
            [ordinary.invitation.as_str()],
            |r| Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?
            ))
        )
        .unwrap(),
        ("accepted".into(), 130, "s2".into())
    );
    let CommandResult::Inbox(after) =
        crate::store::queries::query(&context, "i", &inbox, &budget).unwrap()
    else {
        panic!()
    };
    // Acceptance settles the obligation; unread open and clear warning events
    // remain visible as history until the seat consumes them.
    let item = after
        .items
        .iter()
        .find(|item| item.thread == thread)
        .unwrap();
    assert_eq!(item.invitations, 0);
    assert_eq!(item.pending_receipts, 0);
    assert_eq!(item.warnings, 2);
    assert_eq!(
        conn.query_row(
            "SELECT count(*),count(clear_warning_id) FROM warning_conditions WHERE thread_id=?1 AND condition_kind='invitation'",
            [thread.as_str()],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        ).unwrap(),
        (1, 1),
        "one overdue condition, closed by acceptance",
    );
    assert_eq!(
        crate::store::invitation_due::scan_invitation_due_batch(&context, &mut conn, None, 10)
            .unwrap()
            .warnings_added,
        0
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM receipts WHERE thread_id=?1",
            [thread.as_str()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        accept(
            &context,
            &mut conn,
            &budget,
            &ordinary_command,
            permit(
                "s2",
                "accept-ordinary-two",
                ObligationRef::Invitation(ordinary.invitation.clone()),
                ordinary_hash,
                130
            )
        )
        .unwrap(),
        CommandResult::Accepted(ordinary.invitation.clone().into())
    );
    let fresh_repeat = Accept {
        operation: OperationId::new("accept-ordinary-two-again"),
        ..ordinary_command.clone()
    };
    assert_eq!(
        accept(
            &context,
            &mut conn,
            &budget,
            &fresh_repeat,
            permit(
                "s2",
                "accept-ordinary-two-again",
                ObligationRef::Invitation(ordinary.invitation.clone()),
                cooperative_payload_hash("accept", &fresh_repeat).unwrap(),
                130
            )
        )
        .unwrap(),
        CommandResult::Accepted(ordinary.invitation.clone().into())
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM messages WHERE thread_id=?1 AND event_key LIKE 'accept:%'",
            [thread.as_str()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );

    let reverse = ThreadId::new("independent-reverse-acceptances");
    operate(
        &mut conn,
        ServiceOperation::EnsureThread(EnsureManagedThread {
            thread: reverse.clone(),
            topic: "reverse".into(),
            goal: "goal".into(),
            operation: OperationId::new("ensure-reverse"),
        }),
    );
    let ServiceResult::Invitation(reverse_required) = operate(
        &mut conn,
        ServiceOperation::Invite(ServiceInvite {
            thread: reverse.clone(),
            seat: SeatId::new("s2"),
            constraint: InvitationConstraint::Required,
            deadline_millis: Some(500),
            operation: OperationId::new("required-reverse"),
        }),
    ) else {
        panic!()
    };
    let ServiceResult::Invitation(reverse_ordinary) = operate(
        &mut conn,
        ServiceOperation::Invite(ServiceInvite {
            thread: reverse.clone(),
            seat: SeatId::new("s2"),
            constraint: InvitationConstraint::Ordinary,
            deadline_millis: Some(500),
            operation: OperationId::new("ordinary-reverse"),
        }),
    ) else {
        panic!()
    };
    clock.0.store(140, Ordering::SeqCst);
    let reverse_ordinary_command = Accept {
        thread: reverse.clone(),
        operation: OperationId::new("accept-ordinary-reverse"),
        claim: claim("s2"),
    };
    let reverse_ordinary_hash =
        cooperative_payload_hash("accept", &reverse_ordinary_command).unwrap();
    assert_eq!(
        accept(
            &context,
            &mut conn,
            &budget,
            &reverse_ordinary_command,
            permit(
                "s2",
                "accept-ordinary-reverse",
                ObligationRef::Invitation(reverse_ordinary.invitation.clone()),
                reverse_ordinary_hash,
                140
            )
        )
        .unwrap(),
        CommandResult::Accepted(reverse_ordinary.invitation.clone().into())
    );
    let reverse_interval:(i64,i64,i64)=conn.query_row("SELECT episode,joined_seq,(SELECT joined_at FROM memberships WHERE thread_id=?1 AND seat_id='s2') FROM membership_intervals WHERE thread_id=?1 AND seat_id='s2' AND left_seq IS NULL",[reverse.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    let reverse_requirement = reverse_required.requirement.unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT state FROM requirement_episodes WHERE id=?1",
            [reverse_requirement.requirement.as_str()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "pending"
    );
    clock.0.store(150, Ordering::SeqCst);
    let reverse_required_command = AcceptRequired {
        thread: reverse.clone(),
        invitation: reverse_required.invitation.clone(),
        requirement: reverse_requirement.requirement,
        expected_revision: reverse_requirement.revision,
        operation: OperationId::new("accept-required-reverse"),
        claim: CallerClaim {
            seat: SeatId::new("s2"),
            ..claim("s2")
        },
    };
    let reverse_required_hash =
        cooperative_payload_hash("accept_required", &reverse_required_command).unwrap();
    assert!(matches!(
        accept_required(
            &context,
            &mut conn,
            &budget,
            &reverse_required_command,
            permit(
                "s2",
                "accept-required-reverse",
                ObligationRef::Invitation(reverse_required.invitation.clone()),
                reverse_required_hash,
                150
            )
        )
        .unwrap(),
        CommandResult::RequiredAccepted(_)
    ));
    assert_eq!(conn.query_row("SELECT episode,joined_seq,(SELECT joined_at FROM memberships WHERE thread_id=?1 AND seat_id='s2') FROM membership_intervals WHERE thread_id=?1 AND seat_id='s2' AND left_seq IS NULL",[reverse.as_str()],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?))).unwrap(),reverse_interval);
    assert_eq!(conn.query_row("SELECT count(*) FROM membership_intervals WHERE thread_id=?1 AND seat_id='s2' AND left_seq IS NULL",[reverse.as_str()],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(
        conn.query_row(
            "SELECT state,accepted_at,accepted_actor_seat_id FROM invitations WHERE id=?1",
            [reverse_required.invitation.as_str()],
            |r| Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?
            ))
        )
        .unwrap(),
        ("accepted".into(), 150, "s2".into())
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM receipts WHERE thread_id=?1",
            [reverse.as_str()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn native_ordinary_offer_after_required_offer_can_be_accepted_after_join() {
    use crate::ports::ServiceAuthorityGate;
    use crate::protocol::{commands::AcceptRequired, service::*};
    use sha2::Digest;
    let (context, mut conn, path, clock) = bound_fixture(100);
    let budget = crate::protocol::time::CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    };
    let author = ServiceAuthorId::new(format!("graph:{:x}", sha2::Sha256::digest(b"i")));
    let gate = GraphGate(author.clone());
    let connection = gate.register("i", "boot", author).unwrap();
    let operate = |conn: &mut Connection, operation: ServiceOperation| {
        crate::store::service_controls::operate(
            &context,
            conn,
            "i",
            operation,
            &connection,
            &gate,
            &budget,
            None,
        )
        .unwrap()
    };
    let thread = ThreadId::new("native-independent-offer");
    operate(
        &mut conn,
        ServiceOperation::EnsureThread(EnsureManagedThread {
            thread: thread.clone(),
            topic: "topic".into(),
            goal: "goal".into(),
            operation: OperationId::new("ensure-native-offer"),
        }),
    );
    let ServiceResult::Invitation(creator_invite) = operate(
        &mut conn,
        ServiceOperation::Invite(ServiceInvite {
            thread: thread.clone(),
            seat: SeatId::new("s1"),
            constraint: InvitationConstraint::Ordinary,
            deadline_millis: Some(1000),
            operation: OperationId::new("invite-creator"),
        }),
    ) else {
        panic!()
    };
    let creator_accept = Accept {
        thread: thread.clone(),
        operation: OperationId::new("accept-creator"),
        claim: claim("s1"),
    };
    let creator_hash = cooperative_payload_hash("accept", &creator_accept).unwrap();
    accept(
        &context,
        &mut conn,
        &budget,
        &creator_accept,
        permit(
            "s1",
            "accept-creator",
            ObligationRef::Invitation(creator_invite.invitation),
            creator_hash,
            100,
        ),
    )
    .unwrap();
    let ServiceResult::Invitation(required) = operate(
        &mut conn,
        ServiceOperation::Invite(ServiceInvite {
            thread: thread.clone(),
            seat: SeatId::new("s2"),
            constraint: InvitationConstraint::Required,
            deadline_millis: Some(1000),
            operation: OperationId::new("required-native-offer"),
        }),
    ) else {
        panic!()
    };
    let native_invite = Invite {
        thread: thread.clone(),
        seat: SeatId::new("s2"),
        deadline_millis: Some(1000),
        operation: OperationId::new("native-independent-offer"),
        claim: claim("s1"),
    };
    let invite_hash = cooperative_payload_hash("invite", &native_invite).unwrap();
    let CommandResult::Invitation(ordinary) = invite(
        &context,
        &mut conn,
        &budget,
        &native_invite,
        permit(
            "s1",
            "native-independent-offer",
            ObligationRef::Control(thread.clone()),
            invite_hash,
            100,
        ),
        None,
    )
    .unwrap() else {
        panic!()
    };
    assert_ne!(required.invitation, ordinary);
    clock.0.store(110, Ordering::SeqCst);
    let req = required.requirement.unwrap();
    let required_accept = AcceptRequired {
        thread: thread.clone(),
        invitation: required.invitation.clone(),
        requirement: req.requirement,
        expected_revision: req.revision,
        operation: OperationId::new("accept-required-native-offer"),
        claim: CallerClaim {
            seat: SeatId::new("s2"),
            ..claim("s2")
        },
    };
    let required_hash = cooperative_payload_hash("accept_required", &required_accept).unwrap();
    accept_required(
        &context,
        &mut conn,
        &budget,
        &required_accept,
        permit(
            "s2",
            "accept-required-native-offer",
            ObligationRef::Invitation(required.invitation),
            required_hash,
            110,
        ),
    )
    .unwrap();
    let original:i64=conn.query_row("SELECT joined_seq FROM membership_intervals WHERE thread_id=?1 AND seat_id='s2' AND left_seq IS NULL",[thread.as_str()],|r|r.get(0)).unwrap();
    clock.0.store(120, Ordering::SeqCst);
    let ordinary_accept = Accept {
        thread: thread.clone(),
        operation: OperationId::new("accept-native-offer"),
        claim: claim("s2"),
    };
    let ordinary_hash = cooperative_payload_hash("accept", &ordinary_accept).unwrap();
    assert_eq!(
        accept(
            &context,
            &mut conn,
            &budget,
            &ordinary_accept,
            permit(
                "s2",
                "accept-native-offer",
                ObligationRef::Invitation(ordinary.clone()),
                ordinary_hash,
                120
            )
        )
        .unwrap(),
        CommandResult::Accepted(ordinary.clone().into())
    );
    assert_eq!(conn.query_row("SELECT joined_seq FROM membership_intervals WHERE thread_id=?1 AND seat_id='s2' AND left_seq IS NULL",[thread.as_str()],|r|r.get::<_,i64>(0)).unwrap(),original);
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM membership_intervals WHERE thread_id=?1 AND seat_id='s2'",
            [thread.as_str()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    drop(conn);
    let reopened = context.open_writer().unwrap();
    assert_eq!(
        reopened
            .query_row(
                "SELECT state,accepted_at FROM invitations WHERE id=?1",
                [ordinary.as_str()],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            )
            .unwrap(),
        ("accepted".into(), 120)
    );
    drop(reopened);
    let _ = std::fs::remove_file(path);
}

#[test]
fn managed_required_upgrade_preserves_deadline_and_demands_native_revision() {
    use crate::ports::ServiceAuthorityGate;
    use crate::protocol::{commands::AcceptRequired, service::*};
    use sha2::Digest;
    let (context, mut conn, path, _) = bound_fixture(100);
    let budget = crate::protocol::time::CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    };
    let author = ServiceAuthorId::new(format!("graph:{:x}", sha2::Sha256::digest(b"i")));
    let gate = GraphGate(author.clone());
    let connection = gate.register("i", "boot", author.clone()).unwrap();
    let operate = |conn: &mut Connection, operation: ServiceOperation| {
        crate::store::service_controls::operate(
            &context,
            conn,
            "i",
            operation,
            &connection,
            &gate,
            &budget,
            None,
        )
        .unwrap()
    };
    let thread = ThreadId::new("managed-1");
    operate(
        &mut conn,
        ServiceOperation::EnsureThread(EnsureManagedThread {
            thread: thread.clone(),
            topic: "same topic".into(),
            goal: "goal".into(),
            operation: OperationId::new("ensure-1"),
        }),
    );
    let ordinary = operate(
        &mut conn,
        ServiceOperation::Invite(ServiceInvite {
            thread: thread.clone(),
            seat: SeatId::new("s2"),
            constraint: InvitationConstraint::Ordinary,
            deadline_millis: Some(1000),
            operation: OperationId::new("invite-ordinary"),
        }),
    );
    let ServiceResult::Invitation(ordinary) = ordinary else {
        panic!("ordinary invitation expected")
    };
    let original_deadline: i64 = conn
        .query_row(
            "SELECT deadline_at FROM invitations WHERE id=?1",
            [ordinary.invitation.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    let upgraded = operate(
        &mut conn,
        ServiceOperation::Invite(ServiceInvite {
            thread: thread.clone(),
            seat: SeatId::new("s2"),
            constraint: InvitationConstraint::Required,
            deadline_millis: Some(9000),
            operation: OperationId::new("upgrade"),
        }),
    );
    let ServiceResult::Invitation(upgraded) = upgraded else {
        panic!("required invitation expected")
    };
    assert_eq!(upgraded.invitation, ordinary.invitation);
    let required = upgraded.requirement.clone().unwrap();
    assert_eq!(
        original_deadline,
        conn.query_row(
            "SELECT deadline_at FROM invitations WHERE id=?1",
            [ordinary.invitation.as_str()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap()
    );
    let repeated = operate(
        &mut conn,
        ServiceOperation::Invite(ServiceInvite {
            thread: thread.clone(),
            seat: SeatId::new("s2"),
            constraint: InvitationConstraint::Required,
            deadline_millis: Some(5000),
            operation: OperationId::new("upgrade-again"),
        }),
    );
    assert_eq!(repeated, ServiceResult::Invitation(upgraded.clone()));
    let ordinary_again = operate(
        &mut conn,
        ServiceOperation::Invite(ServiceInvite {
            thread: thread.clone(),
            seat: SeatId::new("s2"),
            constraint: InvitationConstraint::Ordinary,
            deadline_millis: None,
            operation: OperationId::new("ordinary-after-upgrade"),
        }),
    );
    assert_eq!(ordinary_again, ServiceResult::Invitation(upgraded.clone()));
    let mut native_claim = claim("s2");
    native_claim.seat = SeatId::new("s2");
    let stale = Accept {
        thread: thread.clone(),
        operation: OperationId::new("stale-ordinary"),
        claim: native_claim.clone(),
    };
    let stale_hash = cooperative_payload_hash("accept", &stale).unwrap();
    let stale_error = accept(
        &context,
        &mut conn,
        &budget,
        &stale,
        permit(
            "s2",
            "stale-ordinary",
            ObligationRef::Invitation(ordinary.invitation.clone()),
            stale_hash,
            100,
        ),
    )
    .unwrap_err();
    assert_eq!(stale_error.code, ErrorCode::StaleRequirementAcceptance);
    assert!(stale_error.detail.contains("reread"));
    let accepted = AcceptRequired {
        thread: thread.clone(),
        invitation: ordinary.invitation.clone(),
        requirement: required.requirement.clone(),
        expected_revision: required.revision,
        operation: OperationId::new("native-required"),
        claim: native_claim,
    };
    let accepted_hash = cooperative_payload_hash("accept_required", &accepted).unwrap();
    let result = accept_required(
        &context,
        &mut conn,
        &budget,
        &accepted,
        permit(
            "s2",
            "native-required",
            ObligationRef::Invitation(ordinary.invitation.clone()),
            accepted_hash,
            100,
        ),
    )
    .unwrap();
    let CommandResult::RequiredAccepted(accepted_state) = result else {
        panic!("required acceptance expected")
    };
    assert_eq!(accepted_state.state, RequirementState::Accepted);
    assert_eq!(accepted_state.accepted_by, Some(SeatId::new("s2")));
    assert_eq!(accepted_state.revision, required.revision + 1);
    let acceptance_message: String = conn
        .query_row(
            "SELECT id FROM messages WHERE event_key=?1",
            [format!("accept_required:{}", required.requirement.as_str())],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        crate::store::service_substrate::message_author(&conn, &MessageId::new(acceptance_message))
            .unwrap(),
        EventAuthor::Native(SeatId::new("s2"))
    );
    let repeated_accept = AcceptRequired {
        operation: OperationId::new("native-required-again"),
        ..accepted.clone()
    };
    let repeated_hash = cooperative_payload_hash("accept_required", &repeated_accept).unwrap();
    let same = accept_required(
        &context,
        &mut conn,
        &budget,
        &repeated_accept,
        permit(
            "s2",
            "native-required-again",
            ObligationRef::Invitation(ordinary.invitation.clone()),
            repeated_hash,
            100,
        ),
    )
    .unwrap();
    assert_eq!(
        same,
        CommandResult::RequiredAccepted(accepted_state.clone())
    );
    assert_eq!(conn.query_row("SELECT count(*) FROM messages WHERE thread_id=?1 AND event_key LIKE 'accept_required:%'",
        [thread.as_str()],|r|r.get::<_,i64>(0)).unwrap(),1);
    let left = Leave {
        thread: thread.clone(),
        operation: OperationId::new("leave-required"),
        claim: claim("s2"),
    };
    let leave_hash = cooperative_payload_hash("leave", &left).unwrap();
    assert_eq!(
        leave(
            &context,
            &mut conn,
            &budget,
            &left,
            permit(
                "s2",
                "leave-required",
                ObligationRef::Control(thread.clone()),
                leave_hash,
                100
            )
        )
        .unwrap_err()
        .code,
        ErrorCode::MembershipRequired
    );
    let released = operate(
        &mut conn,
        ServiceOperation::ReleaseRequirement(ReleaseRequirement {
            thread: thread.clone(),
            seat: SeatId::new("s2"),
            requirement: required.requirement.clone(),
            operation: OperationId::new("release"),
        }),
    );
    assert!(
        matches!(released,ServiceResult::RequirementReleased(ref state) if state.state==RequirementState::Released)
    );
    let left = Leave {
        operation: OperationId::new("leave-after-release"),
        ..left
    };
    assert!(matches!(
        leave(
            &context,
            &mut conn,
            &budget,
            &left,
            permit(
                "s2",
                "leave-after-release",
                ObligationRef::Control(thread.clone()),
                cooperative_payload_hash("leave", &left).unwrap(),
                100
            )
        )
        .unwrap(),
        CommandResult::Left(_)
    ));
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn managed_release_preserves_upgraded_ordinary_invitation_and_its_deadline() {
    use crate::ports::ServiceAuthorityGate;
    use crate::protocol::service::*;
    use sha2::Digest;
    let (context, mut conn, _, _) = fixture(100);
    let budget = crate::protocol::time::CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    };
    let author = ServiceAuthorId::new(format!("graph:{:x}", sha2::Sha256::digest(b"i")));
    let gate = GraphGate(author.clone());
    let connection = gate.register("i", "boot", author).unwrap();
    let operate = |conn: &mut Connection, operation: ServiceOperation| {
        crate::store::service_controls::operate(
            &context,
            conn,
            "i",
            operation,
            &connection,
            &gate,
            &budget,
            None,
        )
        .unwrap()
    };
    let thread = ThreadId::new("managed-release-upgrade");
    operate(
        &mut conn,
        ServiceOperation::EnsureThread(EnsureManagedThread {
            thread: thread.clone(),
            topic: "topic".into(),
            goal: "goal".into(),
            operation: OperationId::new("ensure-release-upgrade"),
        }),
    );
    let ordinary_op = ServiceOperation::Invite(ServiceInvite {
        thread: thread.clone(),
        seat: SeatId::new("s2"),
        constraint: InvitationConstraint::Ordinary,
        deadline_millis: Some(1000),
        operation: OperationId::new("ordinary-release-upgrade"),
    });
    let ServiceResult::Invitation(ordinary) = operate(&mut conn, ordinary_op.clone()) else {
        panic!()
    };
    let deadline: i64 = conn
        .query_row(
            "SELECT deadline_at FROM invitations WHERE id=?1",
            [ordinary.invitation.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    let ServiceResult::Invitation(upgraded) = operate(
        &mut conn,
        ServiceOperation::Invite(ServiceInvite {
            thread: thread.clone(),
            seat: SeatId::new("s2"),
            constraint: InvitationConstraint::Required,
            deadline_millis: Some(9000),
            operation: OperationId::new("required-release-upgrade"),
        }),
    ) else {
        panic!()
    };
    assert_eq!(upgraded.invitation, ordinary.invitation);
    let requirement = upgraded.requirement.unwrap();
    operate(
        &mut conn,
        ServiceOperation::ReleaseRequirement(ReleaseRequirement {
            thread: thread.clone(),
            seat: SeatId::new("s2"),
            requirement: requirement.requirement,
            operation: OperationId::new("release-upgrade"),
        }),
    );
    assert_eq!(
        operate(
            &mut conn,
            ServiceOperation::Invite(ServiceInvite {
                thread: thread.clone(),
                seat: SeatId::new("s2"),
                constraint: InvitationConstraint::Ordinary,
                deadline_millis: Some(10000),
                operation: OperationId::new("ordinary-after-release-upgrade")
            })
        ),
        ServiceResult::Invitation(ordinary.clone())
    );
    assert_eq!(
        conn.query_row(
            "SELECT deadline_at FROM invitations WHERE id=?1",
            [ordinary.invitation.as_str()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        deadline
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM invitation_cancellations WHERE invitation_id=?1",
            [ordinary.invitation.as_str()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        operate(&mut conn, ordinary_op),
        ServiceResult::Invitation(ordinary)
    );
}

#[test]
fn managed_required_controls_preserve_left_history_and_independent_ordinary_invite() {
    use crate::ports::ServiceAuthorityGate;
    use crate::protocol::service::*;
    use sha2::Digest;
    let (context, mut conn, _, _) = fixture(100);
    let budget = crate::protocol::time::CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    };
    let author = ServiceAuthorId::new(format!("graph:{:x}", sha2::Sha256::digest(b"i")));
    let gate = GraphGate(author.clone());
    let connection = gate.register("i", "boot", author).unwrap();
    let operate = |conn: &mut Connection, operation: ServiceOperation| {
        crate::store::service_controls::operate(
            &context,
            conn,
            "i",
            operation,
            &connection,
            &gate,
            &budget,
            None,
        )
        .unwrap()
    };
    let thread = ThreadId::new("managed-independent");
    operate(
        &mut conn,
        ServiceOperation::EnsureThread(EnsureManagedThread {
            thread: thread.clone(),
            topic: "topic".into(),
            goal: "goal".into(),
            operation: OperationId::new("ensure-independent"),
        }),
    );
    conn.execute("INSERT INTO memberships(thread_id,seat_id,episode,state,voluntary_state,joined_at,left_at) VALUES (?1,'s1',1,'left','left',10,42)",[thread.as_str()]).unwrap();
    conn.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq,left_seq) VALUES (?1,'s1',1,1,2)",[thread.as_str()]).unwrap();
    let ServiceResult::Invitation(left_invite) = operate(
        &mut conn,
        ServiceOperation::Invite(ServiceInvite {
            thread: thread.clone(),
            seat: SeatId::new("s1"),
            constraint: InvitationConstraint::Required,
            deadline_millis: Some(100),
            operation: OperationId::new("required-left"),
        }),
    ) else {
        panic!()
    };
    assert_eq!(
        conn.query_row(
            "SELECT voluntary_state,left_at FROM memberships WHERE thread_id=?1 AND seat_id='s1'",
            [thread.as_str()],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        )
        .unwrap(),
        ("left".into(), 42)
    );
    operate(
        &mut conn,
        ServiceOperation::ReleaseRequirement(ReleaseRequirement {
            thread: thread.clone(),
            seat: SeatId::new("s1"),
            requirement: left_invite.requirement.unwrap().requirement,
            operation: OperationId::new("release-left"),
        }),
    );
    let ServiceResult::Membership(left) = operate(
        &mut conn,
        ServiceOperation::Membership(ServiceMembershipQuery {
            thread: thread.clone(),
            seat: Some(SeatId::new("s1")),
            page: Default::default(),
        }),
    ) else {
        panic!()
    };
    assert_eq!(
        left.items[0].voluntary_state,
        VoluntaryMembershipState::Left
    );
    let native_participants = crate::protocol::commands::Command::Participants(
        crate::protocol::commands::ParticipantsQuery {
            thread: thread.clone(),
            page: Default::default(),
            caller: None,
        },
    );
    let CommandResult::Participants(participants) =
        crate::store::queries::query(&context, "i", &native_participants, &budget).unwrap()
    else {
        panic!()
    };
    let previous = participants
        .items
        .iter()
        .find(|item| item.seat == SeatId::new("s1"))
        .unwrap();
    assert_eq!(
        previous.physical_state,
        crate::protocol::results::MembershipStatus::Left
    );
    assert_eq!(
        previous.effective_state,
        crate::protocol::results::MembershipStatus::Left
    );
    assert_eq!(previous.left_at, Some(crate::protocol::time::UtcMillis(42)));
    let ServiceResult::Invitation(required) = operate(
        &mut conn,
        ServiceOperation::Invite(ServiceInvite {
            thread: thread.clone(),
            seat: SeatId::new("s2"),
            constraint: InvitationConstraint::Required,
            deadline_millis: Some(100),
            operation: OperationId::new("required-first"),
        }),
    ) else {
        panic!()
    };
    let ServiceResult::Invitation(ordinary) = operate(
        &mut conn,
        ServiceOperation::Invite(ServiceInvite {
            thread: thread.clone(),
            seat: SeatId::new("s2"),
            constraint: InvitationConstraint::Ordinary,
            deadline_millis: Some(500),
            operation: OperationId::new("ordinary-second"),
        }),
    ) else {
        panic!()
    };
    assert_ne!(required.invitation, ordinary.invitation);
    let deadline: i64 = conn
        .query_row(
            "SELECT deadline_at FROM invitations WHERE id=?1",
            [ordinary.invitation.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    operate(
        &mut conn,
        ServiceOperation::ReleaseRequirement(ReleaseRequirement {
            thread: thread.clone(),
            seat: SeatId::new("s2"),
            requirement: required.requirement.unwrap().requirement,
            operation: OperationId::new("release-first"),
        }),
    );
    assert_eq!(
        operate(
            &mut conn,
            ServiceOperation::Invite(ServiceInvite {
                thread: thread.clone(),
                seat: SeatId::new("s2"),
                constraint: InvitationConstraint::Ordinary,
                deadline_millis: Some(9999),
                operation: OperationId::new("ordinary-third")
            })
        ),
        ServiceResult::Invitation(ordinary.clone())
    );
    assert_eq!(
        conn.query_row(
            "SELECT deadline_at FROM invitations WHERE id=?1",
            [ordinary.invitation.as_str()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        deadline
    );
    let ServiceResult::Membership(current) = operate(
        &mut conn,
        ServiceOperation::Membership(ServiceMembershipQuery {
            thread,
            seat: Some(SeatId::new("s2")),
            page: Default::default(),
        }),
    ) else {
        panic!()
    };
    assert_eq!(
        current.items[0].voluntary_state,
        VoluntaryMembershipState::Invited
    );
}

#[test]
fn managed_release_cancels_only_required_invitation_and_replay_retains_history() {
    use crate::ports::ServiceAuthorityGate;
    use crate::protocol::service::*;
    use sha2::Digest;
    let (context, mut conn, path, clock) = fixture(100);
    let budget = crate::protocol::time::CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    };
    let author = ServiceAuthorId::new(format!("graph:{:x}", sha2::Sha256::digest(b"i")));
    let gate = GraphGate(author.clone());
    let connection = gate.register("i", "boot", author.clone()).unwrap();
    let operate = |conn: &mut Connection, operation: ServiceOperation| {
        crate::store::service_controls::operate(
            &context,
            conn,
            "i",
            operation,
            &connection,
            &gate,
            &budget,
            None,
        )
    };
    let thread = ThreadId::new("managed-2");
    let ensure = ServiceOperation::EnsureThread(EnsureManagedThread {
        thread: thread.clone(),
        topic: "topic".into(),
        goal: "goal".into(),
        operation: OperationId::new("ensure-2"),
    });
    let first = operate(&mut conn, ensure.clone()).unwrap();
    assert_eq!(operate(&mut conn, ensure.clone()).unwrap(), first);
    assert!(matches!(
        operate(
            &mut conn,
            ServiceOperation::EnsureThread(EnsureManagedThread {
                thread: ThreadId::new("managed-same-topic"),
                topic: "topic".into(),
                goal: "other goal".into(),
                operation: OperationId::new("ensure-same-topic")
            })
        )
        .unwrap(),
        ServiceResult::ThreadEnsured(_)
    ));
    let mut changed = ensure;
    if let ServiceOperation::EnsureThread(v) = &mut changed {
        v.topic = "changed".into();
    }
    assert_eq!(
        operate(&mut conn, changed).unwrap_err().code,
        ErrorCode::OperationPayloadMismatch
    );
    let ordinary = ThreadId::new("ordinary-2");
    conn.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES (?1,'i','topic','goal',100,100)",
        [ordinary.as_str()]).unwrap();
    let ordinary_invite = ServiceOperation::Invite(ServiceInvite {
        thread: ordinary.clone(),
        seat: SeatId::new("s1"),
        constraint: InvitationConstraint::Ordinary,
        deadline_millis: None,
        operation: OperationId::new("ordinary-invite"),
    });
    assert!(matches!(
        operate(&mut conn, ordinary_invite).unwrap(),
        ServiceResult::Invitation(_)
    ));
    assert_eq!(
        operate(
            &mut conn,
            ServiceOperation::Invite(ServiceInvite {
                thread: ordinary.clone(),
                seat: SeatId::new("s2"),
                constraint: InvitationConstraint::Required,
                deadline_millis: None,
                operation: OperationId::new("bad-required")
            })
        )
        .unwrap_err()
        .code,
        ErrorCode::RequiredInvitationNeedsManagedThread
    );
    assert_eq!(
        operate(
            &mut conn,
            ServiceOperation::EnsureThread(EnsureManagedThread {
                thread: ordinary.clone(),
                topic: "topic".into(),
                goal: "goal".into(),
                operation: OperationId::new("collision")
            })
        )
        .unwrap_err()
        .code,
        ErrorCode::IncompatibleOwnership
    );
    let invite = ServiceOperation::Invite(ServiceInvite {
        thread: thread.clone(),
        seat: SeatId::new("s2"),
        constraint: InvitationConstraint::Required,
        deadline_millis: Some(100),
        operation: OperationId::new("required-2"),
    });
    let ServiceResult::Invitation(created) = operate(&mut conn, invite.clone()).unwrap() else {
        panic!()
    };
    let required = created.requirement.as_ref().unwrap();
    let deadline: i64 = conn
        .query_row(
            "SELECT deadline_at FROM invitations WHERE id=?1",
            [created.invitation.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    clock.0.store(200, Ordering::SeqCst);
    assert_eq!(
        operate(&mut conn, invite).unwrap(),
        ServiceResult::Invitation(created.clone())
    );
    assert_eq!(
        deadline,
        conn.query_row(
            "SELECT deadline_at FROM invitations WHERE id=?1",
            [created.invitation.as_str()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap()
    );
    let release = ServiceOperation::ReleaseRequirement(ReleaseRequirement {
        thread: thread.clone(),
        seat: SeatId::new("s2"),
        requirement: required.requirement.clone(),
        operation: OperationId::new("release-2"),
    });
    let released = operate(&mut conn, release.clone()).unwrap();
    assert_eq!(operate(&mut conn, release).unwrap(), released);
    let cancellations: i64 = conn
        .query_row(
            "SELECT count(*) FROM invitation_cancellations WHERE invitation_id=?1",
            [created.invitation.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(cancellations, 1);
    let release_message: String = conn
        .query_row(
            "SELECT id FROM messages WHERE event_key=?1",
            [format!(
                "release_requirement:{}",
                required.requirement.as_str()
            )],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        crate::store::service_substrate::message_author(&conn, &MessageId::new(release_message))
            .unwrap(),
        EventAuthor::Programmatic(author.clone())
    );
    let query = ServiceMembershipQuery {
        thread: thread.clone(),
        seat: Some(SeatId::new("s2")),
        page: Default::default(),
    };
    let ServiceResult::Membership(page) =
        operate(&mut conn, ServiceOperation::Membership(query)).unwrap()
    else {
        panic!()
    };
    assert_eq!(page.items.len(), 1);
    assert_eq!(
        page.items[0].requirement.as_ref().unwrap().state,
        RequirementState::Released
    );
    assert_eq!(
        page.items[0].voluntary_state,
        VoluntaryMembershipState::Absent
    );
    let native_directory =
        crate::protocol::commands::Command::Directory(crate::protocol::commands::DirectoryQuery {
            recent: false,
            membership: Some(SeatId::new("s2")),
            membership_filter: crate::protocol::commands::DirectoryMembership::Invited,
            topic_contains: None,
            page: Default::default(),
        });
    let CommandResult::Directory(directory) =
        crate::store::queries::query(&context, "i", &native_directory, &budget).unwrap()
    else {
        panic!()
    };
    assert!(!directory.items.iter().any(|item| item.thread == thread));
    let native_participants = crate::protocol::commands::Command::Participants(
        crate::protocol::commands::ParticipantsQuery {
            thread: thread.clone(),
            page: Default::default(),
            caller: None,
        },
    );
    let CommandResult::Participants(participants) =
        crate::store::queries::query(&context, "i", &native_participants, &budget).unwrap()
    else {
        panic!()
    };
    assert!(
        !participants
            .items
            .iter()
            .any(|item| item.seat == SeatId::new("s2"))
    );
    let native_thread =
        crate::protocol::commands::Command::Thread(crate::protocol::commands::ThreadQuery {
            thread: thread.clone(),
            page: Default::default(),
            caller: None,
        });
    let CommandResult::Thread(details) =
        crate::store::queries::query(&context, "i", &native_thread, &budget).unwrap()
    else {
        panic!()
    };
    assert_eq!(details.participant_count, 0);
    operate(
        &mut conn,
        ServiceOperation::Invite(ServiceInvite {
            thread: thread.clone(),
            seat: SeatId::new("s1"),
            constraint: InvitationConstraint::Ordinary,
            deadline_millis: None,
            operation: OperationId::new("managed-ordinary-s1"),
        }),
    )
    .unwrap();
    let first_query = ServiceMembershipQuery {
        thread: thread.clone(),
        seat: None,
        page: crate::protocol::pagination::PageRequest {
            cursor: None,
            limit: 1,
            max_bytes: 4096,
        },
    };
    let mut too_small = first_query.clone();
    too_small.page.max_bytes = 256;
    assert_eq!(
        operate(&mut conn, ServiceOperation::Membership(too_small))
            .unwrap_err()
            .code,
        ErrorCode::InvalidBudget
    );
    let ServiceResult::Membership(first_page) =
        operate(&mut conn, ServiceOperation::Membership(first_query.clone())).unwrap()
    else {
        panic!()
    };
    assert_eq!(first_page.items.len(), 1);
    assert!(first_page.has_more);
    assert!(
        serde_json::to_vec(&ServiceResult::Membership(first_page.clone()))
            .unwrap()
            .len()
            <= 4096
    );
    let second_query = ServiceMembershipQuery {
        page: crate::protocol::pagination::PageRequest {
            cursor: first_page.next_cursor.clone(),
            ..first_query.page
        },
        ..first_query
    };
    let ServiceResult::Membership(second_page) =
        operate(&mut conn, ServiceOperation::Membership(second_query)).unwrap()
    else {
        panic!()
    };
    assert_eq!(second_page.items.len(), 1);
    assert!(!second_page.has_more);
    drop(conn);
    let reopened = context.open_writer().unwrap();
    assert_eq!(
        reopened
            .query_row(
                "SELECT count(*) FROM invitation_cancellations WHERE invitation_id=?1",
                [created.invitation.as_str()],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    drop(reopened);
    let _ = std::fs::remove_file(path);
}

#[test]
fn joined_voluntary_requirement_waits_for_new_acceptance_and_can_leave_first() {
    use crate::ports::ServiceAuthorityGate;
    use crate::protocol::{commands::AcceptRequired, service::*};
    use sha2::Digest;
    let (context, mut conn, path, _) = bound_fixture(100);
    let budget = crate::protocol::time::CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    };
    let author = ServiceAuthorId::new(format!("graph:{:x}", sha2::Sha256::digest(b"i")));
    let gate = GraphGate(author.clone());
    let connection = gate.register("i", "boot", author).unwrap();
    let operate = |conn: &mut Connection, operation: ServiceOperation| {
        crate::store::service_controls::operate(
            &context,
            conn,
            "i",
            operation,
            &connection,
            &gate,
            &budget,
            None,
        )
        .unwrap()
    };
    let thread = ThreadId::new("joined-upgrade");
    operate(
        &mut conn,
        ServiceOperation::EnsureThread(EnsureManagedThread {
            thread: thread.clone(),
            topic: "joined".into(),
            goal: "goal".into(),
            operation: OperationId::new("ensure-joined"),
        }),
    );
    operate(
        &mut conn,
        ServiceOperation::Invite(ServiceInvite {
            thread: thread.clone(),
            seat: SeatId::new("s2"),
            constraint: InvitationConstraint::Ordinary,
            deadline_millis: None,
            operation: OperationId::new("ordinary-joined"),
        }),
    );
    let accept_cmd = Accept {
        thread: thread.clone(),
        operation: OperationId::new("accept-ordinary"),
        claim: claim("s2"),
    };
    let accept_hash = cooperative_payload_hash("accept", &accept_cmd).unwrap();
    let ordinary_invitation = InvitationId::new(
        conn.query_row(
            "SELECT id FROM invitations WHERE thread_id=?1 AND seat_id='s2'",
            [thread.as_str()],
            |r| r.get::<_, String>(0),
        )
        .unwrap(),
    );
    assert!(matches!(
        accept(
            &context,
            &mut conn,
            &budget,
            &accept_cmd,
            permit(
                "s2",
                "accept-ordinary",
                ObligationRef::Invitation(ordinary_invitation),
                accept_hash,
                100
            )
        )
        .unwrap(),
        CommandResult::Accepted(_)
    ));
    let native_archive = ThreadMutation {
        thread: thread.clone(),
        operation: OperationId::new("native-archive-managed"),
        claim: claim("s2"),
    };
    let archive_hash = cooperative_payload_hash("archive", &native_archive).unwrap();
    assert_eq!(
        archive(
            &context,
            &mut conn,
            &budget,
            &native_archive,
            permit(
                "s2",
                "native-archive-managed",
                ObligationRef::Control(thread.clone()),
                archive_hash,
                100
            )
        )
        .unwrap_err()
        .code,
        ErrorCode::Unauthorized
    );
    let required = operate(
        &mut conn,
        ServiceOperation::Invite(ServiceInvite {
            thread: thread.clone(),
            seat: SeatId::new("s2"),
            constraint: InvitationConstraint::Required,
            deadline_millis: None,
            operation: OperationId::new("confirm-joined"),
        }),
    );
    let ServiceResult::Invitation(invitation) = required else {
        panic!()
    };
    let requirement = invitation.requirement.unwrap();
    assert_eq!(requirement.state, RequirementState::Pending);
    assert_eq!(
        conn.query_row(
            "SELECT state FROM memberships WHERE thread_id=?1 AND seat_id='s2'",
            [thread.as_str()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "joined"
    );
    let leave_cmd = Leave {
        thread: thread.clone(),
        operation: OperationId::new("leave-before-consent"),
        claim: claim("s2"),
    };
    let leave_hash = cooperative_payload_hash("leave", &leave_cmd).unwrap();
    assert!(matches!(
        leave(
            &context,
            &mut conn,
            &budget,
            &leave_cmd,
            permit(
                "s2",
                "leave-before-consent",
                ObligationRef::Control(thread.clone()),
                leave_hash,
                100
            )
        )
        .unwrap(),
        CommandResult::Left(_)
    ));
    let current =
        crate::store::service_substrate::current_requirement(&conn, &thread, &SeatId::new("s2"))
            .unwrap()
            .unwrap();
    assert_eq!(current.state, RequirementState::Pending);
    let mut native_claim = claim("s2");
    native_claim.seat = SeatId::new("s2");
    let command = AcceptRequired {
        thread: thread.clone(),
        invitation: invitation.invitation.clone(),
        requirement: requirement.requirement,
        expected_revision: requirement.revision,
        operation: OperationId::new("consent-after-leave"),
        claim: native_claim,
    };
    let hash = cooperative_payload_hash("accept_required", &command).unwrap();
    let accepted = accept_required(
        &context,
        &mut conn,
        &budget,
        &command,
        permit(
            "s2",
            "consent-after-leave",
            ObligationRef::Invitation(invitation.invitation),
            hash,
            100,
        ),
    )
    .unwrap();
    assert!(
        matches!(accepted,CommandResult::RequiredAccepted(ref state) if state.state==RequirementState::Accepted)
    );
    assert_eq!(
        conn.query_row(
            "SELECT state FROM memberships WHERE thread_id=?1 AND seat_id='s2'",
            [thread.as_str()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "joined"
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn retirement_cutover_makes_pending_requirement_terminal_before_bounded_cleanup() {
    use crate::ports::ServiceAuthorityGate;
    use crate::protocol::service::*;
    use sha2::Digest;
    let (context, mut conn, path, _) = fixture(100);
    let budget = crate::protocol::time::CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    };
    let author = ServiceAuthorId::new(format!("graph:{:x}", sha2::Sha256::digest(b"i")));
    let gate = GraphGate(author.clone());
    let connection = gate.register("i", "boot", author).unwrap();
    let operate = |conn: &mut Connection, operation: ServiceOperation| {
        crate::store::service_controls::operate(
            &context,
            conn,
            "i",
            operation,
            &connection,
            &gate,
            &budget,
            None,
        )
    };
    let thread = ThreadId::new("retirement-requirement");
    operate(
        &mut conn,
        ServiceOperation::EnsureThread(EnsureManagedThread {
            thread: thread.clone(),
            topic: "retirement".into(),
            goal: "goal".into(),
            operation: OperationId::new("ensure-retirement"),
        }),
    )
    .unwrap();
    let ServiceResult::Invitation(invited) = operate(
        &mut conn,
        ServiceOperation::Invite(ServiceInvite {
            thread: thread.clone(),
            seat: SeatId::new("s2"),
            constraint: InvitationConstraint::Required,
            deadline_millis: None,
            operation: OperationId::new("required-retirement"),
        }),
    )
    .unwrap() else {
        panic!()
    };
    let requirement = invited.requirement.unwrap();
    // The retirement fence changes the seat before its bounded per-thread
    // cleanup reaches the requirement row.
    conn.execute(
        "UPDATE seats SET state='retired',retired_at=101 WHERE id='s2'",
        [],
    )
    .unwrap();
    assert!(
        crate::store::service_substrate::current_requirement(&conn, &thread, &SeatId::new("s2"))
            .unwrap()
            .is_none()
    );
    let query = ServiceMembershipQuery {
        thread: thread.clone(),
        seat: Some(SeatId::new("s2")),
        page: Default::default(),
    };
    let ServiceResult::Membership(page) =
        operate(&mut conn, ServiceOperation::Membership(query)).unwrap()
    else {
        panic!()
    };
    assert_eq!(
        page.items[0].voluntary_state,
        VoluntaryMembershipState::Retired
    );
    assert_eq!(
        page.items[0].requirement.as_ref().unwrap().state,
        RequirementState::Retired
    );
    assert_eq!(
        operate(
            &mut conn,
            ServiceOperation::ReleaseRequirement(ReleaseRequirement {
                thread: thread.clone(),
                seat: SeatId::new("s2"),
                requirement: requirement.requirement,
                operation: OperationId::new("release-retired")
            })
        )
        .unwrap_err()
        .code,
        ErrorCode::NotFound
    );
    assert_eq!(
        operate(
            &mut conn,
            ServiceOperation::Invite(ServiceInvite {
                thread,
                seat: SeatId::new("s2"),
                constraint: InvitationConstraint::Required,
                deadline_millis: None,
                operation: OperationId::new("reinvite-retired")
            })
        )
        .unwrap_err()
        .code,
        ErrorCode::TargetUnresolved
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn create_thread_persists_joined_creator_and_creation_audit() {
    let (context, mut conn, path, _) = bound_fixture(100);
    let command = CreateThread {
        name: None,
        topic: "topic".into(),
        goal: "goal".into(),
        operation: OperationId::new("o1"),
        claim: claim("s1"),
    };
    let hash = cooperative_payload_hash("create_thread", &command).unwrap();
    let result = create_thread(
        &context,
        &mut conn,
        &crate::protocol::time::CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: Default::default(),
        },
        &command,
        permit(
            "s1",
            "o1",
            ObligationRef::CheckIn(SeatId::new("s1")),
            hash,
            100,
        ),
    )
    .unwrap();
    let CommandResult::ThreadCreated(thread) = result else {
        panic!("wrong result")
    };
    assert!(
        is_short_public_id(prefix::THREAD, thread.as_str()),
        "new thread IDs are short: {}",
        thread.as_str()
    );
    let inbox_revision: i64 = conn.query_row("SELECT COALESCE((SELECT revision FROM filter_revisions WHERE instance_id='i' AND scope_kind='inbox' AND scope_key='s1'),0)",[],|r|r.get(0)).unwrap();
    assert_eq!(inbox_revision, 1);
    let state: String = conn
        .query_row(
            "SELECT state FROM memberships WHERE thread_id=?1 AND seat_id='s1'",
            [thread.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "joined");
    let interval: (i64, Option<i64>) = conn.query_row(
        "SELECT joined_seq,left_seq FROM membership_intervals WHERE thread_id=?1 AND seat_id='s1'",
        [thread.as_str()], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert!(interval.0 > 0);
    assert_eq!(interval.1, None);
    let goal: String = conn
        .query_row(
            "SELECT goal FROM threads WHERE id=?1",
            [thread.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(goal, "goal");
    let history: i64 = conn
        .query_row(
            "SELECT count(*) FROM messages WHERE thread_id=?1 AND kind='info'",
            [thread.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(history, 1);
    conn.execute(
        "UPDATE seats SET state='retired',retired_at=100 WHERE id='s1'",
        [],
    )
    .unwrap();
    let replay = create_thread(
        &context,
        &mut conn,
        &crate::protocol::time::CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: Default::default(),
        },
        &command,
        permit(
            "s1",
            "o1",
            ObligationRef::CheckIn(SeatId::new("s1")),
            hash,
            100,
        ),
    )
    .unwrap();
    assert_eq!(replay, CommandResult::ThreadCreated(thread));
    assert_eq!(conn.query_row("SELECT revision FROM filter_revisions WHERE instance_id='i' AND scope_kind='inbox' AND scope_key='s1'",[],|r|r.get::<_,i64>(0)).unwrap(),inbox_revision);
    let changed = CreateThread {
        name: None,
        topic: "changed".into(),
        ..command.clone()
    };
    let changed_hash = cooperative_payload_hash("create_thread", &changed).unwrap();
    assert_eq!(
        create_thread(
            &context,
            &mut conn,
            &crate::protocol::time::CallBudget {
                deadline: MonoInstant(u64::MAX),
                cancellation: Default::default()
            },
            &changed,
            permit(
                "s1",
                "o1",
                ObligationRef::CheckIn(SeatId::new("s1")),
                changed_hash,
                100
            )
        )
        .unwrap_err()
        .code,
        crate::protocol::results::ErrorCode::OperationPayloadMismatch
    );
    let new_operation = CreateThread {
        operation: OperationId::new("new-after-retire"),
        ..command.clone()
    };
    assert!(
        create_thread(
            &context,
            &mut conn,
            &crate::protocol::time::CallBudget {
                deadline: MonoInstant(u64::MAX),
                cancellation: Default::default()
            },
            &new_operation,
            permit(
                "s1",
                "new-after-retire",
                ObligationRef::CheckIn(SeatId::new("s1")),
                hash,
                100
            )
        )
        .is_err()
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn joined_member_changes_topic_with_scoped_revision_and_audit() {
    use crate::protocol::commands::SetTopic;
    let (context, mut conn, path, _) = bound_fixture(100);
    let create = CreateThread {
        name: None,
        topic: "old".into(),
        goal: "goal".into(),
        operation: OperationId::new("create-topic"),
        claim: claim("s1"),
    };
    let hash = cooperative_payload_hash("create_thread", &create).unwrap();
    let CommandResult::ThreadCreated(thread) = create_thread(
        &context,
        &mut conn,
        &crate::protocol::time::CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: Default::default(),
        },
        &create,
        permit(
            "s1",
            "create-topic",
            ObligationRef::CheckIn(SeatId::new("s1")),
            hash,
            100,
        ),
    )
    .unwrap() else {
        panic!()
    };
    let command = SetTopic {
        thread: thread.clone(),
        topic: "new".into(),
        operation: OperationId::new("set-topic"),
        claim: claim("s1"),
    };
    let hash = cooperative_payload_hash("set_topic", &command).unwrap();
    assert_eq!(
        set_topic(
            &context,
            &mut conn,
            &crate::protocol::time::CallBudget {
                deadline: MonoInstant(u64::MAX),
                cancellation: Default::default()
            },
            &command,
            permit(
                "s1",
                "set-topic",
                ObligationRef::Control(thread.clone()),
                hash,
                100
            )
        )
        .unwrap(),
        CommandResult::TopicChanged(thread.clone())
    );
    let (topic, revision): (String, i64) = conn
        .query_row(
            "SELECT topic,topic_revision FROM threads WHERE id=?1",
            [thread.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(topic, "new");
    assert!(revision > 0);
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM messages WHERE thread_id=?1 AND event_key LIKE 'set_topic:%'",
            [thread.as_str()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn pending_reinvite_keeps_first_deadline_and_new_invites_use_precedence() {
    let (context, mut conn, path, _) = bound_fixture(100);
    let create = CreateThread {
        name: None,
        topic: "topic".into(),
        goal: "goal".into(),
        operation: OperationId::new("create"),
        claim: claim("s1"),
    };
    let digest = cooperative_payload_hash("create_thread", &create).unwrap();
    let CommandResult::ThreadCreated(thread) = create_thread(
        &context,
        &mut conn,
        &crate::protocol::time::CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: Default::default(),
        },
        &create,
        permit(
            "s1",
            "create",
            ObligationRef::CheckIn(SeatId::new("s1")),
            digest,
            100,
        ),
    )
    .unwrap() else {
        panic!("create failed")
    };
    let initial_revision: i64 = conn
        .query_row(
            "SELECT membership_revision FROM threads WHERE id=?1",
            [thread.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    let first = Invite {
        thread: thread.clone(),
        seat: SeatId::new("s2"),
        deadline_millis: None,
        operation: OperationId::new("invite1"),
        claim: claim("s1"),
    };
    let hash = cooperative_payload_hash("invite", &first).unwrap();
    let CommandResult::Invitation(invitation) = invite(
        &context,
        &mut conn,
        &crate::protocol::time::CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: Default::default(),
        },
        &first,
        permit(
            "s1",
            "invite1",
            ObligationRef::Control(thread.clone()),
            hash,
            100,
        ),
        Some(120_000),
    )
    .unwrap() else {
        panic!("invite failed")
    };
    let invited_revision: i64 = conn
        .query_row(
            "SELECT membership_revision FROM threads WHERE id=?1",
            [thread.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(invited_revision > initial_revision);
    let saved: (i64, i64) = conn
        .query_row(
            "SELECT frozen_duration_ms, deadline_at FROM invitations WHERE id=?1",
            [invitation.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(saved, (120_000, 120_100));
    let again = Invite {
        deadline_millis: Some(500_000),
        operation: OperationId::new("invite2"),
        ..first
    };
    let hash = cooperative_payload_hash("invite", &again).unwrap();
    let result = invite(
        &context,
        &mut conn,
        &crate::protocol::time::CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: Default::default(),
        },
        &again,
        permit("s1", "invite2", ObligationRef::Control(thread), hash, 100),
        Some(120_000),
    )
    .unwrap();
    assert_eq!(result, CommandResult::Invitation(invitation.clone()));
    let after: (i64, i64) = conn
        .query_row(
            "SELECT frozen_duration_ms, deadline_at FROM invitations WHERE id=?1",
            [invitation.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(after, saved);
    assert_eq!(
        conn.query_row(
            "SELECT membership_revision FROM threads WHERE id=?1",
            [again.thread.as_str()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        invited_revision
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn operator_orphan_invite_uses_configured_duration_and_keeps_pending_deadline() {
    use crate::protocol::{
        authority::{OperatorActor, PeerIdentity},
        commands::OperatorOrphanInvite,
    };
    let (context, mut conn, path, _) = fixture(100);
    conn.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,archived) VALUES ('orphan','i','topic','goal',0,0,1)",[]).unwrap();
    let actor = OperatorActor::from_peer(PeerIdentity::from_kernel(501), 501).unwrap();
    let command = OperatorOrphanInvite {
        thread: ThreadId::new("orphan"),
        seat: SeatId::new("s2"),
        deadline_millis: None,
        operation: OperationId::new("operator-invite-1"),
    };
    let CommandResult::OperatorInvited(id) = operator_orphan_invite(
        &context,
        &mut conn,
        "i",
        &command,
        actor.clone(),
        Some(120_000),
    )
    .unwrap() else {
        panic!()
    };
    assert!(
        is_short_public_id(prefix::INVITATION, id.as_str()),
        "new invitation IDs are short: {}",
        id.as_str()
    );
    let first: (i64, i64) = conn
        .query_row(
            "SELECT frozen_duration_ms,deadline_at FROM invitations WHERE id=?1",
            [id.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(first, (120_000, 120_100));
    let again = OperatorOrphanInvite {
        deadline_millis: Some(500_000),
        operation: OperationId::new("operator-invite-2"),
        ..command
    };
    let result = operator_orphan_invite(&context, &mut conn, "i", &again, actor, None).unwrap();
    assert_eq!(result, CommandResult::OperatorInvited(id.clone()));
    assert_eq!(
        conn.query_row("SELECT archived FROM threads WHERE id='orphan'", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap(),
        1
    );
    assert_eq!(
        conn.query_row(
            "SELECT frozen_duration_ms,deadline_at FROM invitations WHERE id=?1",
            [id.as_str()],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        )
        .unwrap(),
        first
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn late_accept_in_archived_thread_warns_once_and_leave_keeps_receipts() {
    let (context, mut conn, path, clock) = bound_fixture(100);
    let create = CreateThread {
        name: None,
        topic: "topic".into(),
        goal: "goal".into(),
        operation: OperationId::new("c"),
        claim: claim("s1"),
    };
    let hash = cooperative_payload_hash("create_thread", &create).unwrap();
    let CommandResult::ThreadCreated(thread) = create_thread(
        &context,
        &mut conn,
        &crate::protocol::time::CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: Default::default(),
        },
        &create,
        permit(
            "s1",
            "c",
            ObligationRef::CheckIn(SeatId::new("s1")),
            hash,
            100,
        ),
    )
    .unwrap() else {
        panic!()
    };
    let invitation_request = Invite {
        thread: thread.clone(),
        seat: SeatId::new("s2"),
        deadline_millis: None,
        operation: OperationId::new("i"),
        claim: claim("s1"),
    };
    let hash = cooperative_payload_hash("invite", &invitation_request).unwrap();
    let CommandResult::Invitation(invitation) = invite(
        &context,
        &mut conn,
        &crate::protocol::time::CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: Default::default(),
        },
        &invitation_request,
        permit("s1", "i", ObligationRef::Control(thread.clone()), hash, 100),
        None,
    )
    .unwrap() else {
        panic!()
    };
    let archive_request = ThreadMutation {
        thread: thread.clone(),
        operation: OperationId::new("a"),
        claim: claim("s1"),
    };
    let hash = cooperative_payload_hash("archive", &archive_request).unwrap();
    archive(
        &context,
        &mut conn,
        &crate::protocol::time::CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: Default::default(),
        },
        &archive_request,
        permit("s1", "a", ObligationRef::Control(thread.clone()), hash, 100),
    )
    .unwrap();
    clock.0.store(300_100, Ordering::SeqCst);
    let accept_request = Accept {
        thread: thread.clone(),
        operation: OperationId::new("accept"),
        claim: claim("s2"),
    };
    let hash = cooperative_payload_hash("accept", &accept_request).unwrap();
    assert_eq!(
        accept(
            &context,
            &mut conn,
            &crate::protocol::time::CallBudget {
                deadline: MonoInstant(u64::MAX),
                cancellation: Default::default()
            },
            &accept_request,
            permit(
                "s2",
                "accept",
                ObligationRef::Invitation(invitation.clone()),
                hash,
                300_100
            )
        )
        .unwrap(),
        CommandResult::Accepted(invitation.clone().into())
    );
    let joined_invite = Invite {
        thread: thread.clone(),
        seat: SeatId::new("s2"),
        deadline_millis: Some(1),
        operation: OperationId::new("invite-accepted"),
        claim: claim("s1"),
    };
    let joined_hash = cooperative_payload_hash("invite", &joined_invite).unwrap();
    assert_eq!(
        invite(
            &context,
            &mut conn,
            &crate::protocol::time::CallBudget {
                deadline: MonoInstant(u64::MAX),
                cancellation: Default::default()
            },
            &joined_invite,
            permit(
                "s1",
                "invite-accepted",
                ObligationRef::Control(thread.clone()),
                joined_hash,
                300_100
            ),
            None,
        )
        .unwrap(),
        CommandResult::AlreadyJoined(crate::protocol::results::AlreadyJoined {
            thread: thread.clone(),
            seat: SeatId::new("s2"),
        })
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM invitations WHERE thread_id=?1 AND seat_id='s2'",
            [thread.as_str()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    let warnings: i64 = conn
        .query_row(
            "SELECT count(*) FROM messages WHERE thread_id=?1 AND kind='warn'",
            [thread.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(warnings, 2, "one open and one clear warning event");
    assert_eq!(
        conn.query_row(
            "SELECT count(*),count(clear_warning_id) FROM warning_conditions WHERE thread_id=?1 AND condition_kind='invitation'",
            [thread.as_str()],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        ).unwrap(),
        (1, 1),
        "late acceptance opens and closes exactly one condition",
    );
    let state: String = conn
        .query_row(
            "SELECT state FROM memberships WHERE thread_id=?1 AND seat_id='s2'",
            [thread.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "joined");
    let (joined_seq, left_seq): (i64, Option<i64>) = conn.query_row(
        "SELECT joined_seq,left_seq FROM membership_intervals WHERE thread_id=?1 AND seat_id='s2'",
        [thread.as_str()], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert!(joined_seq > 0);
    assert_eq!(left_seq, None);
    conn.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('i','m',?1,100,'ordinary','body',0,100)", [thread.as_str()]).unwrap();
    conn.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES ('m',?1,'s2','pending',300000)", [thread.as_str()]).unwrap();
    let leave_request = Leave {
        thread: thread.clone(),
        operation: OperationId::new("leave"),
        claim: claim("s2"),
    };
    let hash = cooperative_payload_hash("leave", &leave_request).unwrap();
    let inbox_before_leave: i64 = conn.query_row("SELECT revision FROM filter_revisions WHERE instance_id='i' AND scope_kind='inbox' AND scope_key='s2'",[],|r|r.get(0)).unwrap();
    leave(
        &context,
        &mut conn,
        &crate::protocol::time::CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: Default::default(),
        },
        &leave_request,
        permit(
            "s2",
            "leave",
            ObligationRef::Control(thread.clone()),
            hash,
            300_100,
        ),
    )
    .unwrap();
    assert_eq!(conn.query_row("SELECT revision FROM filter_revisions WHERE instance_id='i' AND scope_kind='inbox' AND scope_key='s2'",[],|r|r.get::<_,i64>(0)).unwrap(),inbox_before_leave+1);
    let closed_seq: i64 = conn
        .query_row(
            "SELECT left_seq FROM membership_intervals WHERE thread_id=?1 AND seat_id='s2'",
            [thread.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(closed_seq > joined_seq);
    let receipt: String = conn
        .query_row(
            "SELECT state FROM receipts WHERE message_id='m' AND seat_id='s2'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(receipt, "pending");
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn recovery_snapshot_holds_all_unclaimed_targets_until_explicit_fresh_choice() {
    use crate::{
        ports::{
            EnumerationEvidence, EvidenceKind, ExecutionEvidence, HostObservation, HostSnapshot,
            HostUiState, IncarnationEvidence, ObservationProvenance, OperatorRequest,
            OperatorTargetGuard, StructuralOccupancy,
        },
        protocol::{
            authority::{OperatorActor, PeerIdentity},
            commands::{OperatorFreshSeat, ResolveSeat},
        },
        store::seats,
    };
    let (context, mut conn, path, _) = fixture(100);
    conn.execute(
        "UPDATE seats SET state='unresolved', target_id=NULL WHERE id='s1'",
        [],
    )
    .unwrap();
    let mut observation = HostObservation {
        focused: false,
        target: HostTargetId::new("p1"),
        host_boot: HostBootId::new("b"),
        epoch: 2,
        generation: 3,
        observed_at_utc: UtcMillis(100),
        observed_at_mono: MonoInstant(100),
        provenance: ObservationProvenance::FreshCurrentTarget,
        occupant: None,
        ui: HostUiState::Idle,
        terminal: Some(TerminalId::new("terminal-p1")),
        occupancy: StructuralOccupancy::EmptyShell,
        incarnation: IncarnationEvidence::Verified {
            identity: "incarnation".into(),
            evidence_kind: EvidenceKind::NativeCurrentTarget,
        },
        execution: ExecutionEvidence::Unknown,
        call_id: HostCallId::new("call"),
        connection_epoch: 1,
        observation_sequence: 1,
        started_at_mono: MonoInstant(100),
        completed_at_mono: MonoInstant(100),
    };
    let snapshot = HostSnapshot {
        boot: HostBootId::new("b"),
        epoch: 2,
        observation_sequence: 1,
        complete: true,
        enumeration: EnumerationEvidence::CoherentVerified,
        incarnation: IncarnationEvidence::Verified {
            identity: "incarnation".into(),
            evidence_kind: EvidenceKind::CoherentEnumeration,
        },
        targets: vec![observation.clone()],
    };
    let partial = HostSnapshot {
        enumeration: EnumerationEvidence::Partial,
        ..snapshot.clone()
    };
    assert!(seats::record_snapshot(&context, &mut conn, "i", partial).is_err());
    let budget = crate::protocol::time::CallBudget {
        deadline: MonoInstant(1000),
        cancellation: crate::protocol::time::Cancellation::default(),
    };
    let admission = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let stage = staged_test_snapshot(&context, &mut conn, admission, &snapshot, &budget);
    seats::publish_snapshot_stage(&context, &mut conn, &stage, &budget).unwrap();
    observation.observation_sequence = 2;
    let admission = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    assert!(
        seats::publish_current_target_observation(
            &context,
            &mut conn,
            &admission,
            &observation,
            &budget
        )
        .unwrap()
    );
    let eligibility_revision: i64 = conn
        .query_row(
            "SELECT send_eligibility_revision FROM host_instances WHERE id='i'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(eligibility_revision > 0);
    assert_eq!(
        conn.query_row("SELECT count(*) FROM seats", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        crate::store::effective::effective_recovery_disposition(&conn, "i", "p1").unwrap(),
        crate::store::effective::EffectiveRecoveryDisposition::BaselineHeld
    );
    drop(conn);
    let mut conn = context.open_writer().unwrap();
    let ordinary = ResolveSeat {
        target: HostTargetId::new("p1"),
        operation: OperationId::new("resolve"),
    };
    let guard =
        crate::ports::OrdinaryResolutionGuard::try_new(&ordinary, observation.clone(), &admission)
            .unwrap();
    assert_eq!(
        seats::resolve_seat(
            &context,
            &mut conn,
            "i",
            ordinary,
            crate::ports::OrdinaryResolutionAttempt::Observed(guard),
            &budget,
        )
        .unwrap_err()
        .code,
        ErrorCode::TargetUnresolved
    );
    let fresh = OperatorFreshSeat {
        target: HostTargetId::new("p1"),
        operation: OperationId::new("fresh"),
    };
    let guard = OperatorTargetGuard::try_new(
        "i",
        &crate::protocol::commands::OperatorCommand::FreshSeat(fresh.clone()),
        observation.clone(),
    )
    .unwrap();
    let actor = OperatorActor::from_peer(PeerIdentity::from_kernel(501), 501).unwrap();
    let result = seats::mutate_operator(
        &context,
        &mut conn,
        "i",
        OperatorRequest::FreshSeat(fresh, guard),
        actor,
        None,
    )
    .unwrap();
    let CommandResult::OperatorFreshSeat(new_seat) = result else {
        panic!("fresh seat not returned")
    };
    assert_eq!(
        conn.query_row(
            "SELECT target_generation FROM seats WHERE id=?1",
            [new_seat.as_str()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        3
    );
    assert_eq!(
        conn.query_row(
            "SELECT structural_terminal_id,structural_observation_sequence FROM seats WHERE id=?1",
            [new_seat.as_str()],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        )
        .unwrap(),
        ("terminal-p1".into(), 2)
    );
    let after_repair_revision: i64 = conn
        .query_row(
            "SELECT send_eligibility_revision FROM host_instances WHERE id='i'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(after_repair_revision > eligibility_revision);
    assert_eq!(
        conn.query_row(
            "SELECT state FROM seats WHERE id=?1",
            [new_seat.as_str()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "resolved"
    );
    assert_eq!(
        conn.query_row("SELECT state FROM seats WHERE id='s1'", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "unresolved"
    );
    conn.execute(
        "INSERT INTO host_instances(id,created_at,host_boot,host_epoch) VALUES ('j',0,'b',2)",
        [],
    )
    .unwrap();
    let mut cross_snapshot = snapshot_for_test(1, &["p1"]);
    cross_snapshot.epoch = 2;
    cross_snapshot.incarnation = IncarnationEvidence::Verified {
        identity: "incarnation".into(),
        evidence_kind: EvidenceKind::CoherentEnumeration,
    };
    cross_snapshot.targets = vec![HostObservation {
        observation_sequence: 1,
        ..observation.clone()
    }];
    let admission = seats::begin_host_observation(&context, &mut conn, "j", &budget).unwrap();
    let stage = staged_test_snapshot(&context, &mut conn, admission, &cross_snapshot, &budget);
    seats::publish_snapshot_stage(&context, &mut conn, &stage, &budget).unwrap();
    let admission = seats::begin_host_observation(&context, &mut conn, "j", &budget).unwrap();
    assert!(
        seats::publish_current_target_observation(
            &context,
            &mut conn,
            &admission,
            &observation,
            &budget
        )
        .unwrap()
    );
    let cross = crate::protocol::commands::OperatorFreshSeat {
        target: HostTargetId::new("p1"),
        operation: OperationId::new("fresh"),
    };
    let cross_guard = OperatorTargetGuard::try_new(
        "j",
        &crate::protocol::commands::OperatorCommand::FreshSeat(cross.clone()),
        observation,
    )
    .unwrap();
    let cross_actor = OperatorActor::from_peer(PeerIdentity::from_kernel(501), 501).unwrap();
    let CommandResult::OperatorFreshSeat(cross_seat) = seats::mutate_operator(
        &context,
        &mut conn,
        "j",
        OperatorRequest::FreshSeat(cross, cross_guard),
        cross_actor,
        None,
    )
    .unwrap() else {
        panic!("cross-instance operator allocation missing")
    };
    assert_eq!(
        conn.query_row(
            "SELECT instance_id FROM seats WHERE id=?1",
            [cross_seat.as_str()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "j"
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn ordinary_allocation_replay_returns_original_seat_after_retirement() {
    use crate::{
        ports::{
            EvidenceKind, ExecutionEvidence, HostObservation, HostUiState, IncarnationEvidence,
            ObservationProvenance, OrdinaryResolutionAttempt as Attempt, OrdinaryResolutionGuard,
            OrdinaryResolutionOutcome as Outcome, StructuralOccupancy,
        },
        protocol::commands::ResolveSeat,
        protocol::time::{CallBudget, Cancellation},
        store::seats,
    };
    let (context, mut conn, path, _) = fixture(100);
    // Remove this fixture's derived archival children before its seeded seats.
    conn.execute("DELETE FROM seat_archival", []).unwrap();
    conn.execute("DELETE FROM seats", []).unwrap();
    let observation = HostObservation {
        focused: false,
        target: HostTargetId::new("p3"),
        host_boot: HostBootId::new("b"),
        epoch: 1,
        generation: 1,
        observed_at_utc: UtcMillis(100),
        observed_at_mono: MonoInstant(100),
        provenance: ObservationProvenance::FreshCurrentTarget,
        occupant: None,
        ui: HostUiState::Idle,
        terminal: Some(TerminalId::new("terminal-p3")),
        occupancy: StructuralOccupancy::EmptyShell,
        incarnation: IncarnationEvidence::Verified {
            identity: "test-incarnation".into(),
            evidence_kind: EvidenceKind::NativeCurrentTarget,
        },
        execution: ExecutionEvidence::Unknown,
        call_id: HostCallId::new("call"),
        connection_epoch: 1,
        observation_sequence: 2,
        started_at_mono: MonoInstant(100),
        completed_at_mono: MonoInstant(100),
    };
    let budget = CallBudget {
        deadline: MonoInstant(1_000),
        cancellation: Cancellation::default(),
    };
    let admission = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let captured = snapshot_for_test(1, &["p3"]);
    let stage = staged_test_snapshot(&context, &mut conn, admission, &captured, &budget);
    seats::publish_snapshot_stage(&context, &mut conn, &stage, &budget).unwrap();
    let admission = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    assert!(
        seats::publish_current_target_observation(
            &context,
            &mut conn,
            &admission,
            &observation,
            &budget
        )
        .unwrap()
    );
    let request = ResolveSeat {
        target: HostTargetId::new("p3"),
        operation: OperationId::new("resolve-once"),
    };
    let guard =
        || OrdinaryResolutionGuard::try_new(&request, observation.clone(), &admission).unwrap();
    let Outcome::Resolved(seat) = seats::resolve_seat(
        &context,
        &mut conn,
        "i",
        request.clone(),
        Attempt::Observed(guard()),
        &budget,
    )
    .unwrap() else {
        panic!("seat missing")
    };
    conn.execute(
        "UPDATE seats SET state='retired',retired_at=100 WHERE id=?1",
        [seat.as_str()],
    )
    .unwrap();
    assert_eq!(
        seats::resolve_seat(
            &context,
            &mut conn,
            "i",
            request.clone(),
            Attempt::Observed(guard()),
            &budget,
        )
        .unwrap(),
        Outcome::Resolved(seat.clone())
    );
    assert_eq!(
        conn.query_row(
            "SELECT state FROM seats WHERE id=?1",
            [seat.as_str()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "retired"
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn ordinary_empty_allocation_retains_structural_identity_without_occupant_binding() {
    use crate::{
        ports::{
            EvidenceKind, ExecutionEvidence, GuardedSeatTransition, HostObservation, HostUiState,
            IncarnationEvidence, ObservationProvenance, OrdinaryResolutionAttempt,
            OrdinaryResolutionGuard, OrdinaryResolutionOutcome, ReconciliationAction,
            ReconciliationOutcome, StructuralOccupancy,
        },
        protocol::commands::ResolveSeat,
        protocol::time::{CallBudget, Cancellation},
        store::seats,
    };
    let (context, mut conn, path, _) = fixture(100);
    // Remove this fixture's derived archival children before its seeded seats.
    conn.execute("DELETE FROM seat_archival", []).unwrap();
    conn.execute("DELETE FROM seats", []).unwrap();
    let budget = CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Cancellation::default(),
    };
    let mut first = snapshot_for_test(1, &["p3"]);
    first.targets[0].terminal = Some(TerminalId::new("terminal-p3"));
    let admission = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let stage = staged_test_snapshot(&context, &mut conn, admission, &first, &budget);
    seats::publish_snapshot_stage(&context, &mut conn, &stage, &budget).unwrap();
    let request = ResolveSeat {
        target: HostTargetId::new("p3"),
        operation: OperationId::new("ordinary-empty-proof"),
    };
    let observation = HostObservation {
        focused: false,
        target: request.target.clone(),
        host_boot: HostBootId::new("b"),
        epoch: 1,
        generation: 1,
        observed_at_utc: UtcMillis(100),
        observed_at_mono: MonoInstant(100),
        provenance: ObservationProvenance::FreshCurrentTarget,
        occupant: None,
        ui: HostUiState::Idle,
        terminal: Some(TerminalId::new("terminal-p3")),
        occupancy: StructuralOccupancy::EmptyShell,
        incarnation: IncarnationEvidence::Verified {
            identity: "test-incarnation".into(),
            evidence_kind: EvidenceKind::CoherentEnumeration,
        },
        execution: ExecutionEvidence::Unknown,
        call_id: HostCallId::new("allocation-proof"),
        connection_epoch: 1,
        observation_sequence: 2,
        started_at_mono: MonoInstant(100),
        completed_at_mono: MonoInstant(100),
    };
    let admission = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    assert!(
        seats::publish_current_target_observation(
            &context,
            &mut conn,
            &admission,
            &observation,
            &budget
        )
        .unwrap()
    );
    let guard = OrdinaryResolutionGuard::try_new(&request, observation, &admission).unwrap();
    let OrdinaryResolutionOutcome::Resolved(seat) = seats::resolve_seat(
        &context,
        &mut conn,
        "i",
        request,
        OrdinaryResolutionAttempt::Observed(guard),
        &budget,
    )
    .unwrap() else {
        panic!("seat missing")
    };
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM occupant_bindings WHERE seat_id=?1",
            [seat.as_str()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    drop(conn);
    let mut conn = context.open_writer().unwrap();
    let mut second = snapshot_for_test(3, &["p3"]);
    second.targets[0].terminal = Some(TerminalId::new("terminal-p3"));
    let admission = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let stage = staged_test_snapshot(&context, &mut conn, admission, &second, &budget);
    seats::publish_snapshot_stage(&context, &mut conn, &stage, &budget).unwrap();
    let page = seats::saved_seats_page(&context, &conn, &stage, 0, None, 16, &budget).unwrap();
    let saved = page.seats.iter().find(|saved| saved.seat == seat).unwrap();
    assert_eq!(
        saved.terminal.as_ref().map(TerminalId::as_str),
        Some("terminal-p3")
    );
    assert_eq!(saved.bound_boot.as_ref().map(HostBootId::as_str), Some("b"));
    assert_eq!(saved.bound_incarnation.as_deref(), Some("test-incarnation"));
    assert!(saved.binding_execution.is_none());
    let mut moved_snapshot = snapshot_for_test(4, &["moved-p3"]);
    moved_snapshot.targets[0].terminal = Some(TerminalId::new("terminal-p3"));
    let admission = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let stage = staged_test_snapshot(&context, &mut conn, admission, &moved_snapshot, &budget);
    let publication = seats::publish_snapshot_stage(&context, &mut conn, &stage, &budget).unwrap();
    assert_eq!(
        seats::apply_reconciliation_transition(
            &context,
            &mut conn,
            GuardedSeatTransition {
                publication,
                seat: seat.clone(),
                expected_binding_generation: 1,
                expected_target: Some(HostTargetId::new("p3")),
                expected_terminal: Some(TerminalId::new("terminal-p3")),
                action: ReconciliationAction::Move {
                    target: HostTargetId::new("moved-p3"),
                    terminal: TerminalId::new("terminal-p3"),
                },
            },
            &budget,
        )
        .unwrap(),
        ReconciliationOutcome::Applied
    );
    assert_eq!(
        conn.query_row(
            "SELECT target_id FROM seats WHERE id=?1",
            [seat.as_str()],
            |r| { r.get::<_, String>(0) }
        )
        .unwrap(),
        "moved-p3"
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM occupant_bindings WHERE seat_id=?1",
            [seat.as_str()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let admission = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let absent = snapshot_for_test(5, &[]);
    let stage = staged_test_snapshot(&context, &mut conn, admission, &absent, &budget);
    let publication = seats::publish_snapshot_stage(&context, &mut conn, &stage, &budget).unwrap();
    assert!(matches!(
        seats::apply_reconciliation_transition(
            &context,
            &mut conn,
            GuardedSeatTransition {
                publication,
                seat: seat.clone(),
                expected_binding_generation: 1,
                expected_target: Some(HostTargetId::new("moved-p3")),
                expected_terminal: Some(TerminalId::new("terminal-p3")),
                action: ReconciliationAction::BeginRetirement {
                    absent_target: HostTargetId::new("moved-p3"),
                },
            },
            &budget,
        )
        .unwrap(),
        ReconciliationOutcome::RetirementStarted(_)
    ));
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn coherent_target_change_updates_structural_generation() {
    use crate::{
        ports::{
            EnumerationEvidence, EvidenceKind, ExecutionEvidence, HostObservation, HostSnapshot,
            HostUiState, IncarnationEvidence, ObservationProvenance, StructuralOccupancy,
        },
        store::seats,
    };
    let (context, mut conn, path, _) = fixture(100);
    let observation = HostObservation {
        focused: false,
        target: HostTargetId::new("s2"),
        host_boot: HostBootId::new("b"),
        epoch: 1,
        generation: 2,
        observed_at_utc: UtcMillis(100),
        observed_at_mono: MonoInstant(100),
        provenance: ObservationProvenance::FreshCurrentTarget,
        occupant: None,
        ui: HostUiState::Idle,
        terminal: None,
        occupancy: StructuralOccupancy::EmptyShell,
        incarnation: IncarnationEvidence::Unknown,
        execution: ExecutionEvidence::Unknown,
        call_id: HostCallId::new("call-change"),
        connection_epoch: 1,
        observation_sequence: 2,
        started_at_mono: MonoInstant(100),
        completed_at_mono: MonoInstant(100),
    };
    seats::record_snapshot(
        &context,
        &mut conn,
        "i",
        HostSnapshot {
            boot: HostBootId::new("b"),
            epoch: 1,
            observation_sequence: 2,
            complete: true,
            enumeration: EnumerationEvidence::CoherentVerified,
            incarnation: IncarnationEvidence::Verified {
                identity: "incarnation".into(),
                evidence_kind: EvidenceKind::CoherentEnumeration,
            },
            targets: vec![observation],
        },
    )
    .unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT target_generation FROM seats WHERE id='s2'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    assert_eq!(
        conn.query_row("SELECT generation FROM seats WHERE id='s2'", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn coherent_snapshots_keep_per_target_order_and_accept_ordered_empty_enumeration() {
    use crate::{
        ports::{
            EnumerationEvidence, EvidenceKind, ExecutionEvidence, HostObservation, HostSnapshot,
            HostUiState, IncarnationEvidence, ObservationProvenance, StructuralOccupancy,
        },
        store::seats,
    };
    let (context, mut conn, path, _) = fixture(100);
    let target = |id: &str, generation: u64, sequence: u64| HostObservation {
        focused: false,
        target: HostTargetId::new(id),
        host_boot: HostBootId::new("b"),
        epoch: 1,
        generation,
        observed_at_utc: UtcMillis(100),
        observed_at_mono: MonoInstant(100),
        provenance: ObservationProvenance::FreshCurrentTarget,
        occupant: None,
        ui: HostUiState::Idle,
        terminal: None,
        occupancy: StructuralOccupancy::EmptyShell,
        incarnation: IncarnationEvidence::Unknown,
        execution: ExecutionEvidence::Unknown,
        call_id: HostCallId::new(format!("call-{id}-{sequence}")),
        connection_epoch: 1,
        observation_sequence: sequence,
        started_at_mono: MonoInstant(100),
        completed_at_mono: MonoInstant(100),
    };
    let snapshot = |sequence: u64, targets: Vec<HostObservation>| HostSnapshot {
        boot: HostBootId::new("b"),
        epoch: 1,
        observation_sequence: sequence,
        complete: true,
        enumeration: EnumerationEvidence::CoherentVerified,
        incarnation: IncarnationEvidence::Verified {
            identity: "incarnation".into(),
            evidence_kind: EvidenceKind::CoherentEnumeration,
        },
        targets,
    };
    seats::record_snapshot(
        &context,
        &mut conn,
        "i",
        snapshot(5, vec![target("s1", 1, 4), target("s2", 2, 5)]),
    )
    .unwrap();
    seats::record_snapshot(
        &context,
        &mut conn,
        "i",
        snapshot(6, vec![target("s1", 2, 6), target("s2", 99, 4)]),
    )
    .unwrap();
    let saved:(i64,i64)=conn.query_row("SELECT generation,observation_sequence FROM observed_targets WHERE instance_id='i' AND target_id='s2'",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(saved, (2, 5));
    assert_eq!(
        conn.query_row(
            "SELECT target_generation FROM seats WHERE id='s1'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    seats::record_snapshot(&context, &mut conn, "i", snapshot(7, vec![])).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT observation_sequence FROM host_instances WHERE id='i'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        7
    );
    assert!(seats::record_snapshot(&context, &mut conn, "i", snapshot(6, vec![])).is_err());
    assert!(seats::record_snapshot(&context, &mut conn, "i", snapshot(7, vec![])).is_err());
    let mut new_boot_target = target("s2", 4, 1);
    new_boot_target.host_boot = HostBootId::new("c");
    let mut new_boot = snapshot(1, vec![new_boot_target]);
    new_boot.boot = HostBootId::new("c");
    seats::record_snapshot(&context, &mut conn, "i", new_boot).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT host_boot,observation_sequence FROM host_instances WHERE id='i'",
            [],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        )
        .unwrap(),
        ("c".to_owned(), 1)
    );
    assert_eq!(conn.query_row("SELECT generation,observation_sequence FROM observed_targets WHERE instance_id='i' AND target_id='s2'",[],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?))).unwrap(),(4,1));
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn restoring_current_target_observation_bumps_send_eligibility_revision() {
    use crate::{
        ports::{
            EnumerationEvidence, EvidenceKind, ExecutionEvidence, HostObservation, HostSnapshot,
            HostUiState, IncarnationEvidence, ObservationProvenance, StructuralOccupancy,
        },
        store::seats,
    };
    let (context, mut conn, path, _) = fixture(100);
    conn.execute(
        "DELETE FROM observed_targets WHERE instance_id='i' AND target_id='s2'",
        [],
    )
    .unwrap();
    let observation = HostObservation {
        focused: false,
        target: HostTargetId::new("s2"),
        host_boot: HostBootId::new("b"),
        epoch: 1,
        generation: 1,
        observed_at_utc: UtcMillis(100),
        observed_at_mono: MonoInstant(100),
        provenance: ObservationProvenance::FreshCurrentTarget,
        occupant: None,
        ui: HostUiState::Idle,
        terminal: None,
        occupancy: StructuralOccupancy::EmptyShell,
        incarnation: IncarnationEvidence::Unknown,
        execution: ExecutionEvidence::Unknown,
        call_id: HostCallId::new("fresh-s2"),
        connection_epoch: 1,
        observation_sequence: 1,
        started_at_mono: MonoInstant(100),
        completed_at_mono: MonoInstant(100),
    };
    let initial: i64 = conn
        .query_row(
            "SELECT send_eligibility_revision FROM host_instances WHERE id='i'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    seats::record_snapshot(
        &context,
        &mut conn,
        "i",
        HostSnapshot {
            boot: HostBootId::new("b"),
            epoch: 1,
            observation_sequence: 1,
            complete: true,
            enumeration: EnumerationEvidence::CoherentVerified,
            incarnation: IncarnationEvidence::Verified {
                identity: "incarnation".into(),
                evidence_kind: EvidenceKind::CoherentEnumeration,
            },
            targets: vec![observation],
        },
    )
    .unwrap();
    let latest: i64 = conn
        .query_row(
            "SELECT send_eligibility_revision FROM host_instances WHERE id='i'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(latest > initial);
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn snapshot_rejects_nonrepresentable_host_numbers_without_changing_state() {
    use crate::ports::{
        EnumerationEvidence, EvidenceKind, ExecutionEvidence, HostObservation, HostSnapshot,
        HostUiState, IncarnationEvidence, ObservationProvenance, StructuralOccupancy,
    };
    use crate::store::seats;
    let (context, mut conn, path, _) = fixture(100);
    let before: (String, i64, i64) = conn
        .query_row(
            "SELECT host_boot,host_epoch,observation_sequence FROM host_instances WHERE id='i'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    let make = |epoch, generation, sequence| HostSnapshot {
        boot: HostBootId::new("overflow-boot"),
        epoch,
        observation_sequence: sequence,
        complete: true,
        enumeration: EnumerationEvidence::CoherentVerified,
        incarnation: IncarnationEvidence::Verified {
            identity: "overflow-incarnation".into(),
            evidence_kind: EvidenceKind::CoherentEnumeration,
        },
        targets: vec![HostObservation {
            focused: false,
            target: HostTargetId::new("overflow-target"),
            host_boot: HostBootId::new("overflow-boot"),
            epoch,
            generation,
            observed_at_utc: UtcMillis(100),
            observed_at_mono: MonoInstant(100),
            provenance: ObservationProvenance::FreshCurrentTarget,
            occupant: None,
            ui: HostUiState::Idle,
            terminal: None,
            occupancy: StructuralOccupancy::EmptyShell,
            incarnation: IncarnationEvidence::Unknown,
            execution: ExecutionEvidence::Unknown,
            call_id: HostCallId::new("overflow-call"),
            connection_epoch: 1,
            observation_sequence: sequence,
            started_at_mono: MonoInstant(100),
            completed_at_mono: MonoInstant(100),
        }],
    };
    for snapshot in [
        make(u64::MAX, 1, 1),
        make(1, u64::MAX, 1),
        make(1, 1, u64::MAX),
    ] {
        assert_eq!(
            seats::record_snapshot(&context, &mut conn, "i", snapshot)
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
        let after: (String, i64, i64) = conn
            .query_row(
                "SELECT host_boot,host_epoch,observation_sequence FROM host_instances WHERE id='i'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(after, before);
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM observed_targets WHERE target_id='overflow-target'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }
    seats::record_snapshot(
        &context,
        &mut conn,
        "i",
        make(i64::MAX as u64, i64::MAX as u64, i64::MAX as u64),
    )
    .unwrap();
    let accepted: (i64, i64, i64) = conn
        .query_row(
            "SELECT h.host_epoch,h.observation_sequence,o.generation FROM host_instances h JOIN observed_targets o ON o.instance_id=h.id WHERE h.id='i' AND o.target_id='overflow-target'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(accepted, (i64::MAX, i64::MAX, i64::MAX));
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn inviting_directly_joined_creator_is_truthful_replayable_noop() {
    let (context, mut conn, path, _) = bound_fixture(100);
    let create = CreateThread {
        name: None,
        topic: "topic".into(),
        goal: "goal".into(),
        operation: OperationId::new("create-creator"),
        claim: claim("s1"),
    };
    let create_hash = cooperative_payload_hash("create_thread", &create).unwrap();
    let CommandResult::ThreadCreated(thread) = create_thread(
        &context,
        &mut conn,
        &crate::protocol::time::CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: Default::default(),
        },
        &create,
        permit(
            "s1",
            "create-creator",
            ObligationRef::CheckIn(SeatId::new("s1")),
            create_hash,
            100,
        ),
    )
    .unwrap() else {
        panic!()
    };
    let request = Invite {
        thread: thread.clone(),
        seat: SeatId::new("s1"),
        deadline_millis: Some(42),
        operation: OperationId::new("invite-creator"),
        claim: claim("s1"),
    };
    let digest = cooperative_payload_hash("invite", &request).unwrap();
    let expected = CommandResult::AlreadyJoined(crate::protocol::results::AlreadyJoined {
        thread: thread.clone(),
        seat: SeatId::new("s1"),
    });
    for _ in 0..2 {
        let result = invite(
            &context,
            &mut conn,
            &crate::protocol::time::CallBudget {
                deadline: MonoInstant(u64::MAX),
                cancellation: Default::default(),
            },
            &request,
            permit(
                "s1",
                "invite-creator",
                ObligationRef::Control(thread.clone()),
                digest,
                100,
            ),
            None,
        )
        .unwrap();
        assert_eq!(result, expected);
    }
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM invitations WHERE thread_id=?1",
            [thread.as_str()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM messages WHERE thread_id=?1",
            [thread.as_str()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        conn.query_row(
            "SELECT state FROM memberships WHERE thread_id=?1 AND seat_id='s1'",
            [thread.as_str()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "joined"
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn retirement_rejects_nonrepresentable_closure_evidence_before_fence() {
    use crate::ports::ClosureEvidence;
    let (context, mut conn, path, _) = fixture(100);
    let error = begin_retirement(
        &context,
        &mut conn,
        SeatId::new("s2"),
        ClosureEvidence {
            host_boot: HostBootId::new("b"),
            epoch: u64::MAX,
            target: HostTargetId::new("s2"),
            generation: 1,
        },
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert_eq!(
        conn.query_row("SELECT state FROM seats WHERE id='s2'", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "resolved"
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM retirements", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn retirement_rejects_exhausted_binding_generation_before_fence() {
    use crate::ports::ClosureEvidence;
    let (context, mut conn, path, _) = fixture(100);
    conn.execute("UPDATE seats SET generation=?1 WHERE id='s2'", [i64::MAX])
        .unwrap();
    let error = begin_retirement(
        &context,
        &mut conn,
        SeatId::new("s2"),
        ClosureEvidence {
            host_boot: HostBootId::new("b"),
            epoch: 1,
            target: HostTargetId::new("s2"),
            generation: 1,
        },
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::SequenceExhausted);
    assert_eq!(
        conn.query_row("SELECT state FROM seats WHERE id='s2'", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "resolved"
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM retirements", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn registration_rejects_exhausted_binding_generation_without_anchor() {
    use crate::{
        protocol::commands::{CheckIn, CheckInMode},
        store::seats,
    };
    let (context, mut conn, path, _) = fixture(100);
    conn.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) VALUES ('s2',?1,'s2','b',1,1,'codex','old','old','cooperative_top_level',0,'term-'||'s2','inc')",[i64::MAX]).unwrap();
    let command = CheckIn {
        mode: CheckInMode::Lifecycle {
            expected_binding_generation: 1,
        },
        claim: claim("s2"),
        operation: OperationId::new("exhausted-check-in"),
    };
    let digest =
        crate::store::schema::canonical_digest(&seats::check_in_payload(&command)).unwrap();
    let error = seats::register_available(
        &context,
        &mut conn,
        &command,
        None,
        permit(
            "s2",
            "exhausted-check-in",
            ObligationRef::CheckIn(SeatId::new("s2")),
            digest,
            100,
        ),
        &crate::protocol::time::CallBudget {
            deadline: crate::protocol::time::MonoInstant(u64::MAX),
            cancellation: Default::default(),
        },
        |_tx, _seat, _seq| panic!("offer must not run after exhausted generation"),
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::SequenceExhausted);
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM seat_availability WHERE seat_id='s2'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM occupant_bindings WHERE seat_id='s2' AND ended_at IS NULL",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn operator_rebind_rejects_exhausted_binding_generation() {
    use crate::{
        ports::{
            ExecutionEvidence, HostObservation, HostUiState, IncarnationEvidence,
            ObservationProvenance, OperatorRequest, OperatorTargetGuard, StructuralOccupancy,
        },
        protocol::{
            authority::OperatorActor,
            authority::PeerIdentity,
            commands::{OperatorCommand, OperatorRebind},
        },
        store::seats,
    };
    let (context, mut conn, path, _) = fixture(100);
    conn.execute(
        "UPDATE seats SET state='unresolved',target_id=NULL,generation=?1 WHERE id='s1'",
        [i64::MAX],
    )
    .unwrap();
    conn.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','free','b',1,1,1,100,'fresh','term-'||'free','inc','coherent_enumeration',1)",[]).unwrap();
    let command = OperatorRebind {
        seat: SeatId::new("s1"),
        target: HostTargetId::new("free"),
        operation: OperationId::new("rebind-exhausted"),
    };
    let observation = HostObservation {
        focused: false,
        target: HostTargetId::new("free"),
        host_boot: HostBootId::new("b"),
        epoch: 1,
        generation: 1,
        observed_at_utc: UtcMillis(100),
        observed_at_mono: MonoInstant(100),
        provenance: ObservationProvenance::FreshCurrentTarget,
        occupant: None,
        ui: HostUiState::Idle,
        terminal: None,
        occupancy: StructuralOccupancy::EmptyShell,
        incarnation: IncarnationEvidence::Unknown,
        execution: ExecutionEvidence::Unknown,
        call_id: HostCallId::new("rebind-exhausted-call"),
        connection_epoch: 1,
        observation_sequence: 1,
        started_at_mono: MonoInstant(100),
        completed_at_mono: MonoInstant(100),
    };
    let guard =
        OperatorTargetGuard::try_new("i", &OperatorCommand::Rebind(command.clone()), observation)
            .unwrap();
    let actor = OperatorActor::from_peer(PeerIdentity::from_kernel(501), 501).unwrap();
    let error = seats::mutate_operator(
        &context,
        &mut conn,
        "i",
        OperatorRequest::Rebind(command, guard),
        actor,
        None,
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::SequenceExhausted);
    assert_eq!(
        conn.query_row("SELECT state FROM seats WHERE id='s1'", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "unresolved"
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM allocation_decisions WHERE target_id='free'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn mixed_retirement_preserves_committed_prefix_across_failed_quantum_and_reopen() {
    use crate::{
        ports::{ClosureEvidence, WorkAdmission},
        protocol::time::{CallBudget, Cancellation},
    };
    let (context, mut conn, path, clock) = fixture(100);
    for thread in ["retire-a", "retire-b"] {
        conn.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES (?1,'i','topic','goal',0,0)", [thread]).unwrap();
    }
    conn.execute("INSERT INTO memberships(thread_id,seat_id,state,joined_at) VALUES ('retire-a','s2','joined',0)", []).unwrap();
    conn.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('retire-a','s2',1,1)", []).unwrap();
    conn.execute(
        "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('retire-b','s2','invited')",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,frozen_duration_ms,deadline_at,created_decision_seq) VALUES ('retire-invite','retire-b','s2',1,'pending',0,90,90,1)", []).unwrap();
    for n in 0..18 {
        let id = format!("retire-message-{n}");
        conn.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('i',?1,'retire-a',?2,'ordinary','body',0,?2)", rusqlite::params![id,n+1]).unwrap();
        conn.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at,ack_actor_seat_id,ack_generation,ack_observation,acked_at) VALUES (?1,'retire-a','s2',?2,90,0,?3,?4,?5,?6,?7)",
            rusqlite::params![id,if n==0 {"acked"} else {"pending"},if n==17 {101} else {90},if n==0 {Some("s2")} else {None},if n==0 {Some(1)} else {None},if n==0 {Some("verified")} else {None},if n==0 {Some(50)} else {None}]).unwrap();
    }
    conn.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('i','retire-b-message','retire-b',1,'ordinary','body',0,19)", []).unwrap();
    conn.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) VALUES ('retire-b-message','retire-b','s2','pending',90,0,90)", []).unwrap();
    for (n, (name, duration, eligible)) in [
        ("overdue", 90, 1),
        ("equality", 100, 1),
        ("predeadline", 101, 1),
        ("unstarted", 90, 0),
        ("acked", 90, 1),
    ]
    .into_iter()
    .enumerate()
    {
        let prep = format!("retire-prep-{name}");
        let message = format!("retire-logical-{name}");
        conn.execute("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,recipient_count,status) VALUES (?1,'i','seat:s1',?2,zeroblob(32),'retire-a',0,0,0,0,0,0,0,1,'sealed')",
            rusqlite::params![prep,format!("retire-op-{name}")]).unwrap();
        conn.execute("INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES (?1,'retire-a','s2',1,?2,?3)",
            rusqlite::params![prep,duration,eligible]).unwrap();
        conn.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('i',?1,'retire-a',?2,'ordinary','body',0,?3)",
            rusqlite::params![message,19+n as i64,20+n as i64]).unwrap();
        conn.execute("INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('i',?1,?2,'retire-a',?3,0,?4,0,1,0)",
            rusqlite::params![prep,message,20+n as i64,19+n as i64]).unwrap();
    }
    conn.execute("INSERT INTO receipt_state(message_id,seat_id,state,ack_actor_seat_id,ack_generation,ack_observation,acked_at) VALUES ('retire-logical-acked','s2','acked','s2',1,'verified',50)",[]).unwrap();
    conn.execute(
        "UPDATE threads SET next_sequence=24 WHERE id='retire-a'",
        [],
    )
    .unwrap();
    conn.execute("UPDATE threads SET next_sequence=2 WHERE id='retire-b'", [])
        .unwrap();
    conn.execute("UPDATE host_instances SET decision_seq=30 WHERE id='i'", [])
        .unwrap();
    let job = begin_retirement(
        &context,
        &mut conn,
        SeatId::new("s2"),
        ClosureEvidence {
            host_boot: HostBootId::new("b"),
            epoch: 1,
            target: HostTargetId::new("s2"),
            generation: 1,
        },
    )
    .unwrap();
    assert_eq!(
        crate::store::effective::effective_receipt(&conn, "retire-logical-overdue", "s2")
            .unwrap()
            .unwrap()
            .state,
        crate::store::effective::EffectiveReceiptState::RecipientRetired
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM receipt_state WHERE state='recipient_retired'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    clock.0.store(200, Ordering::SeqCst);
    let budget = CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Cancellation::default(),
    };
    // Fail the first durable thread-selection/cursor publication, after SQL effects
    // have run, to prove they roll back with the initial progress pointer.
    conn.execute_batch("CREATE TRIGGER fail_retire_selection BEFORE UPDATE ON retirements WHEN OLD.phase='select_thread' AND OLD.processed_units=0 BEGIN SELECT RAISE(ABORT,'injected selection cursor failure'); END;").unwrap();
    assert!(
        advance_retirement(
            &context,
            &mut conn,
            job.id.clone(),
            WorkAdmission::Background,
            &budget
        )
        .is_err()
    );
    assert_eq!(
        conn.query_row(
            "SELECT phase,thread_ordinal,processed_units FROM retirements WHERE id=?1",
            [job.id.as_str()],
            |r| Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?
            ))
        )
        .unwrap(),
        ("select_thread".into(), 0, 0)
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM receipts WHERE state='recipient_retired'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM messages WHERE kind='warn'", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(conn);
    let mut conn = context.open_writer().unwrap();
    conn.execute_batch("DROP TRIGGER fail_retire_selection;")
        .unwrap();
    let first = advance_retirement(
        &context,
        &mut conn,
        job.id.clone(),
        WorkAdmission::Background,
        &budget,
    )
    .unwrap();
    assert_eq!(first.processed_this_turn, 16);
    assert!(!first.complete);
    let saved: (i64,String,i64,i64) = conn.query_row("SELECT thread_ordinal,phase,obligation_ordinal,processed_units FROM retirements WHERE id=?1",[job.id.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    let retired_before: i64 = conn
        .query_row(
            "SELECT count(*) FROM receipts WHERE state='recipient_retired'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(retired_before > 0);
    conn.execute_batch("CREATE TRIGGER fail_retire_receipt BEFORE UPDATE OF state ON receipts WHEN OLD.message_id='retire-message-16' AND NEW.state='recipient_retired' BEGIN SELECT RAISE(ABORT,'injected receipt failure'); END;").unwrap();
    assert!(
        advance_retirement(
            &context,
            &mut conn,
            job.id.clone(),
            WorkAdmission::Background,
            &budget
        )
        .is_err()
    );
    let after_receipt_failure: (i64,String,i64,i64) = conn.query_row("SELECT thread_ordinal,phase,obligation_ordinal,processed_units FROM retirements WHERE id=?1",[job.id.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(after_receipt_failure, saved);
    conn.execute_batch("DROP TRIGGER fail_retire_receipt;")
        .unwrap();
    conn.execute_batch("CREATE TRIGGER fail_retire_invite BEFORE UPDATE OF state ON invitations WHEN OLD.id='retire-invite' AND NEW.state='recipient_retired' BEGIN SELECT RAISE(ABORT,'injected retirement failure'); END;").unwrap();
    assert!(
        advance_retirement(
            &context,
            &mut conn,
            job.id.clone(),
            WorkAdmission::Background,
            &budget
        )
        .is_err()
    );
    let after: (i64,String,i64,i64) = conn.query_row("SELECT thread_ordinal,phase,obligation_ordinal,processed_units FROM retirements WHERE id=?1",[job.id.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(after, saved);
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM receipts WHERE state='recipient_retired'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        retired_before
    );
    conn.execute_batch("DROP TRIGGER fail_retire_invite;")
        .unwrap();
    conn.execute_batch("CREATE TRIGGER fail_retire_logical BEFORE INSERT ON receipt_state WHEN NEW.message_id='retire-logical-overdue' BEGIN SELECT RAISE(ABORT,'injected logical receipt failure'); END;").unwrap();
    let mut logical_failure_seen = false;
    for _ in 0..8 {
        let before: (i64,String,i64,i64) = conn.query_row("SELECT thread_ordinal,phase,obligation_ordinal,processed_units FROM retirements WHERE id=?1",[job.id.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        match advance_retirement(
            &context,
            &mut conn,
            job.id.clone(),
            WorkAdmission::Background,
            &budget,
        ) {
            Ok(progress) => assert!(progress.processed_this_turn <= 16),
            Err(_) => {
                let after: (i64,String,i64,i64) = conn.query_row("SELECT thread_ordinal,phase,obligation_ordinal,processed_units FROM retirements WHERE id=?1",[job.id.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
                assert_eq!(after, before);
                logical_failure_seen = true;
                break;
            }
        }
    }
    assert!(logical_failure_seen);
    conn.execute_batch("DROP TRIGGER fail_retire_logical;")
        .unwrap();
    conn.execute_batch("CREATE TRIGGER fail_retire_cursor BEFORE UPDATE ON retirements WHEN NEW.processed_units>OLD.processed_units BEGIN SELECT RAISE(ABORT,'injected retirement cursor failure'); END;").unwrap();
    let mut cursor_failure_seen = false;
    for _ in 0..8 {
        let before: (i64,String,i64,i64) = conn.query_row("SELECT thread_ordinal,phase,obligation_ordinal,processed_units FROM retirements WHERE id=?1",[job.id.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        let before_effects: (i64,i64) = conn.query_row("SELECT (SELECT count(*) FROM receipt_state WHERE state='recipient_retired'),(SELECT count(*) FROM messages WHERE kind='warn')",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        match advance_retirement(
            &context,
            &mut conn,
            job.id.clone(),
            WorkAdmission::Background,
            &budget,
        ) {
            Ok(progress) => assert!(progress.processed_this_turn <= 16),
            Err(_) => {
                let after: (i64,String,i64,i64) = conn.query_row("SELECT thread_ordinal,phase,obligation_ordinal,processed_units FROM retirements WHERE id=?1",[job.id.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
                let after_effects: (i64,i64) = conn.query_row("SELECT (SELECT count(*) FROM receipt_state WHERE state='recipient_retired'),(SELECT count(*) FROM messages WHERE kind='warn')",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
                assert_eq!(after, before);
                assert_eq!(after_effects, before_effects);
                cursor_failure_seen = true;
                break;
            }
        }
    }
    assert!(cursor_failure_seen);
    conn.execute_batch("DROP TRIGGER fail_retire_cursor;")
        .unwrap();
    conn.execute_batch("CREATE TRIGGER fail_retire_audit BEFORE INSERT ON retirement_audits WHEN NEW.thread_id='retire-b' BEGIN SELECT RAISE(ABORT,'injected audit failure'); END;").unwrap();
    drop(conn);
    let mut conn = context.open_writer().unwrap();
    let mut audit_failure_seen = false;
    for _ in 0..8 {
        let before: (i64,String,i64,i64) = conn.query_row("SELECT thread_ordinal,phase,obligation_ordinal,processed_units FROM retirements WHERE id=?1",[job.id.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        match advance_retirement(
            &context,
            &mut conn,
            job.id.clone(),
            WorkAdmission::Background,
            &budget,
        ) {
            Ok(progress) => {
                assert!(progress.processed_this_turn <= 16);
                assert!(!progress.complete);
            }
            Err(_) => {
                let after: (i64,String,i64,i64) = conn.query_row("SELECT thread_ordinal,phase,obligation_ordinal,processed_units FROM retirements WHERE id=?1",[job.id.as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
                assert_eq!(after, before);
                audit_failure_seen = true;
                break;
            }
        }
    }
    assert!(audit_failure_seen);
    conn.execute_batch("DROP TRIGGER fail_retire_audit;")
        .unwrap();
    drop(conn);
    let mut conn = context.open_writer().unwrap();
    let mut turns = 0;
    loop {
        let progress = advance_retirement(
            &context,
            &mut conn,
            job.id.clone(),
            WorkAdmission::Background,
            &budget,
        )
        .unwrap();
        assert!(progress.processed_this_turn <= 16);
        turns += 1;
        if progress.complete {
            break;
        }
        assert!(turns < 8);
    }
    assert_eq!(conn.query_row("SELECT count(*) FROM invitations WHERE id='retire-invite' AND state='recipient_retired'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM receipts WHERE state='acked'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM messages WHERE kind='warn'", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        20
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM receipt_state WHERE state='recipient_retired'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        4
    );
    assert_eq!(
        conn.query_row(
            "SELECT ack_actor_seat_id FROM receipt_state WHERE message_id='retire-logical-acked'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "s2"
    );
    assert_eq!(conn.query_row("SELECT count(*) FROM messages WHERE source_message_id='retire-logical-predeadline' OR source_message_id='retire-logical-unstarted'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM retirement_audits WHERE job_id=?1",
            [job.id.as_str()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    let warning_b:i64=conn.query_row("SELECT sequence FROM messages WHERE thread_id='retire-b' AND kind='warn' ORDER BY sequence DESC LIMIT 1",[],|r|r.get(0)).unwrap();
    let audit_b:i64=conn.query_row("SELECT sequence FROM messages WHERE thread_id='retire-b' AND event_key LIKE 'retirement:%'",[],|r|r.get(0)).unwrap();
    assert!(warning_b < audit_b);
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn retirement_fence_allows_orphan_invite_before_membership_cleanup() {
    use crate::ports::ClosureEvidence;
    use crate::protocol::{
        authority::{OperatorActor, PeerIdentity},
        commands::OperatorOrphanInvite,
        results::ErrorCode,
    };
    let (context, mut conn, path, _) = fixture(100);
    for thread in ["new-orphan", "still-joined"] {
        conn.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES (?1,'i','topic','goal',0,0)",[thread]).unwrap();
        conn.execute("INSERT INTO memberships(thread_id,seat_id,state,joined_at) VALUES (?1,'s2','joined',0)",[thread]).unwrap();
        conn.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES (?1,'s2',1,1)",[thread]).unwrap();
    }
    conn.execute("INSERT INTO memberships(thread_id,seat_id,state,joined_at) VALUES ('still-joined','s1','joined',0)",[]).unwrap();
    conn.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('still-joined','s1',1,1)",[]).unwrap();
    conn.execute("UPDATE host_instances SET decision_seq=1 WHERE id='i'", [])
        .unwrap();
    let actor = OperatorActor::from_peer(PeerIdentity::from_kernel(501), 501).unwrap();
    let invite = OperatorOrphanInvite {
        thread: ThreadId::new("new-orphan"),
        seat: SeatId::new("s1"),
        deadline_millis: Some(90),
        operation: OperationId::new("after-fence"),
    };
    assert_eq!(
        operator_orphan_invite(&context, &mut conn, "i", &invite, actor.clone(), None)
            .unwrap_err()
            .code,
        ErrorCode::ThreadNotOrphaned
    );
    let job = begin_retirement(
        &context,
        &mut conn,
        SeatId::new("s2"),
        ClosureEvidence {
            host_boot: HostBootId::new("b"),
            epoch: 1,
            target: HostTargetId::new("s2"),
            generation: 1,
        },
    )
    .unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT state FROM memberships WHERE thread_id='new-orphan' AND seat_id='s2'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "joined"
    );
    assert_eq!(
        conn.query_row(
            "SELECT status FROM retirements WHERE id=?1",
            [job.id.as_str()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "pending"
    );
    assert!(matches!(
        operator_orphan_invite(&context, &mut conn, "i", &invite, actor.clone(), None).unwrap(),
        CommandResult::OperatorInvited(_)
    ));
    let guarded = OperatorOrphanInvite {
        thread: ThreadId::new("still-joined"),
        operation: OperationId::new("guarded"),
        ..invite
    };
    assert_eq!(
        operator_orphan_invite(&context, &mut conn, "i", &guarded, actor, None)
            .unwrap_err()
            .code,
        ErrorCode::ThreadNotOrphaned
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn retirement_fence_is_immediate_and_cleanup_is_bounded_with_ordered_warnings() {
    use crate::{
        ports::{ClosureEvidence, WorkAdmission},
        protocol::time::{CallBudget, Cancellation},
    };
    let (context, mut conn, path, _) = fixture(100);
    conn.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0)", []).unwrap();
    conn.execute(
        "INSERT INTO memberships(thread_id,seat_id,state,joined_at) VALUES ('t','s2','joined',0)",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t','s2',1,1)",[]).unwrap();
    conn.execute("UPDATE host_instances SET decision_seq=1 WHERE id='i'", [])
        .unwrap();
    for n in 0..25 {
        let id = format!("m{n}");
        conn.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('i',?1,'t',?2,'ordinary','body',0,?2)", rusqlite::params![id,n+1]).unwrap();
        conn.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) VALUES (?1,'t','s2','pending',100,0,?2)",
            rusqlite::params![id, if n < 12 { 100 } else { 101 }]).unwrap();
    }
    conn.execute("UPDATE threads SET next_sequence=26 WHERE id='t'", [])
        .unwrap();
    let job = begin_retirement(
        &context,
        &mut conn,
        SeatId::new("s2"),
        ClosureEvidence {
            host_boot: HostBootId::new("b"),
            epoch: 1,
            target: HostTargetId::new("s2"),
            generation: 1,
        },
    )
    .unwrap();
    let (cutover_seq, eligibility_revision): (Option<i64>, i64) = conn.query_row(
        "SELECT s.retired_seq,h.send_eligibility_revision FROM seats s JOIN host_instances h ON h.id=s.instance_id WHERE s.id='s2'",
        [], |r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert!(cutover_seq.unwrap_or_default() > 0);
    assert!(eligibility_revision > 0);
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM receipts WHERE state='pending'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        25
    );
    assert_eq!(
        crate::store::effective::effective_receipt(&conn, "m0", "s2")
            .unwrap()
            .unwrap()
            .state,
        crate::store::effective::EffectiveReceiptState::RecipientRetired
    );
    drop(conn);
    let budget = CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Cancellation::default(),
    };
    let mut turns = 0;
    loop {
        let mut reopened = context.open_writer().unwrap();
        let progress = advance_retirement(
            &context,
            &mut reopened,
            job.id.clone(),
            WorkAdmission::Background,
            &budget,
        )
        .unwrap();
        assert!(progress.processed_this_turn <= 16);
        turns += 1;
        if progress.complete {
            break;
        }
        assert!(turns < 10);
    }
    assert!(turns >= 2);
    let conn = context.open_writer().unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM receipts WHERE state='recipient_retired'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        25
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM messages WHERE kind='warn'", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        12
    );
    let audit_sequence: i64 = conn
        .query_row(
            "SELECT sequence FROM messages WHERE event_key LIKE 'retirement:%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let last_warning: i64 = conn
        .query_row(
            "SELECT max(sequence) FROM messages WHERE kind='warn'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(last_warning < audit_sequence);
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM receipts WHERE state='acked'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        conn.query_row(
            "SELECT left_seq FROM membership_intervals WHERE thread_id='t' AND seat_id='s2'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn retirement_materializes_unprojected_manifest_receipts_in_bounded_turns() {
    use crate::{
        ports::{ClosureEvidence, WorkAdmission},
        protocol::time::{CallBudget, Cancellation},
    };
    let (context, mut conn, path, _) = fixture(100);
    conn.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('logical','i','topic','goal',0,0)",[]).unwrap();
    conn.execute("INSERT INTO memberships(thread_id,seat_id,state,joined_at) VALUES ('logical','s2','joined',0)",[]).unwrap();
    for n in 0..20 {
        let prep = format!("p{n}");
        let message = format!("lm{n}");
        let op = format!("op{n}");
        conn.execute("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,recipient_count,status) VALUES (?1,'i','seat:s1',?2,zeroblob(32),'logical',0,0,0,0,0,0,0,1,'sealed')",
            rusqlite::params![prep,op]).unwrap();
        conn.execute("INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES (?1,'logical','s2',1,100,1)",[prep.as_str()]).unwrap();
        conn.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('i',?1,'logical',?2,'ordinary','body',0,?2)",rusqlite::params![message,n+1]).unwrap();
        conn.execute("INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('i',?1,?2,'logical',?3,0,?3,0,1,0)",rusqlite::params![prep,message,n+1]).unwrap();
    }
    conn.execute("INSERT INTO receipt_state(message_id,seat_id,state,acked_at) VALUES ('lm0','s2','acked',50)",[]).unwrap();
    conn.execute("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,recipient_count,status) VALUES ('unstarted','i','seat:s1','unstarted-op',zeroblob(32),'logical',0,0,0,0,0,0,0,1,'sealed')",[]).unwrap();
    conn.execute("INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES ('unstarted','logical','s2',1,100,1)",[]).unwrap();
    conn.execute("UPDATE threads SET next_sequence=21 WHERE id='logical'", [])
        .unwrap();
    conn.execute("UPDATE host_instances SET decision_seq=30 WHERE id='i'", [])
        .unwrap();
    let job = begin_retirement(
        &context,
        &mut conn,
        SeatId::new("s2"),
        ClosureEvidence {
            host_boot: HostBootId::new("b"),
            epoch: 1,
            target: HostTargetId::new("s2"),
            generation: 1,
        },
    )
    .unwrap();
    assert_eq!(
        crate::store::effective::effective_receipt(&conn, "lm1", "s2")
            .unwrap()
            .unwrap()
            .state,
        crate::store::effective::EffectiveReceiptState::RecipientRetired
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM receipt_state", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    let budget = CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Cancellation::default(),
    };
    let mut turns = 0;
    loop {
        let progress = advance_retirement(
            &context,
            &mut conn,
            job.id.clone(),
            WorkAdmission::Background,
            &budget,
        )
        .unwrap();
        assert!(progress.processed_this_turn <= 16);
        turns += 1;
        if progress.complete {
            break;
        }
        assert!(turns < 10);
    }
    assert!(turns >= 2);
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM receipt_state WHERE state='recipient_retired'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        19
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM receipt_state WHERE state='acked'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM receipt_state WHERE message_id='unstarted'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM messages WHERE thread_id='logical' AND kind='warn'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        19
    );
    let audit_sequence:i64=conn.query_row("SELECT sequence FROM messages WHERE thread_id='logical' AND event_key LIKE 'retirement:%'",[],|r|r.get(0)).unwrap();
    let warning_sequence: i64 = conn
        .query_row(
            "SELECT MAX(sequence) FROM messages WHERE thread_id='logical' AND kind='warn'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(warning_sequence < audit_sequence);
    drop(conn);
    let _ = std::fs::remove_file(path);
}

fn seed_structural_continuity_obligations(conn: &Connection) {
    conn.execute_batch("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('fix3-thread','i','topic','goal',0,0);
        INSERT INTO memberships(thread_id,seat_id,state) VALUES ('fix3-thread','s1','invited');
        INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES ('fix3-invite','fix3-thread','s1',1,'pending',0,(SELECT decision_seq+1 FROM host_instances WHERE id='i'),300,300);
        UPDATE host_instances SET decision_seq=decision_seq+1 WHERE id='i';
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_seq,decision_at) VALUES ('fix3-message','i','fix3-thread',1,'ordinary','body',(SELECT decision_seq+1 FROM host_instances WHERE id='i'),0);
        UPDATE host_instances SET decision_seq=decision_seq+1 WHERE id='i';
        UPDATE threads SET next_sequence=2 WHERE id='fix3-thread';
        INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES ('fix3-message','fix3-thread','s1','pending',300);").unwrap();
}

/// Mutate one durable fence at a time, then restore it before the real allowed
/// production chain continues. These are deciding-store rejection controls.
fn reject_changed_structural_continuity(
    context: &StoreContext,
    conn: &mut Connection,
    transition: &crate::ports::GuardedSeatTransition,
    budget: &crate::protocol::time::CallBudget,
) {
    use crate::ports::ReconciliationOutcome;
    use crate::store::seats;
    for (change, restore) in [
        (
            "UPDATE seats SET structural_incarnation='changed-incarnation' WHERE id='s1'",
            None,
        ),
        (
            "UPDATE seats SET structural_observation_sequence=999 WHERE id='s1'",
            None,
        ),
        (
            "UPDATE seats SET structural_connection_epoch=0 WHERE id='s1'",
            None,
        ),
        (
            "UPDATE seats SET generation=generation+1 WHERE id='s1'",
            Some("UPDATE seats SET generation=generation-1 WHERE id='s1'"),
        ),
        (
            "UPDATE seats SET state='unresolved',unresolved_reason='other' WHERE id='s1'",
            Some("UPDATE seats SET state='resolved',unresolved_reason=NULL WHERE id='s1'"),
        ),
    ] {
        let proof: (String,i64,i64) = conn.query_row("SELECT structural_incarnation,structural_observation_sequence,structural_connection_epoch FROM seats WHERE id='s1'", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        conn.execute(change, []).unwrap();
        assert_eq!(
            seats::apply_reconciliation_transition(context, conn, transition.clone(), budget)
                .unwrap(),
            ReconciliationOutcome::Stale,
            "{change}"
        );
        if let Some(restore) = restore {
            conn.execute(restore, []).unwrap();
        }
        conn.execute("UPDATE seats SET structural_incarnation=?1,structural_observation_sequence=?2,structural_connection_epoch=?3 WHERE id='s1'", rusqlite::params![proof.0,proof.1,proof.2]).unwrap();
    }
    let proof: (String,String,String,String,i64,i64,i64) = conn.query_row("SELECT structural_terminal_id,structural_incarnation,structural_incarnation_kind,structural_host_boot,structural_host_epoch,structural_connection_epoch,structural_observation_sequence FROM seats WHERE id='s1'", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).unwrap();
    conn.execute_batch("UPDATE seats SET structural_terminal_id=NULL,structural_incarnation=NULL,structural_incarnation_kind=NULL,structural_host_boot=NULL,structural_host_epoch=NULL,structural_connection_epoch=NULL,structural_observation_sequence=NULL WHERE id='s1';").unwrap();
    assert_eq!(
        seats::apply_reconciliation_transition(context, conn, transition.clone(), budget).unwrap(),
        ReconciliationOutcome::Stale
    );
    conn.execute("UPDATE seats SET structural_terminal_id=?1,structural_incarnation=?2,structural_incarnation_kind=?3,structural_host_boot=?4,structural_host_epoch=?5,structural_connection_epoch=?6,structural_observation_sequence=?7 WHERE id='s1'", rusqlite::params![proof.0,proof.1,proof.2,proof.3,proof.4,proof.5,proof.6]).unwrap();
    let mut stale = transition.clone();
    stale.publication.observation_sequence += 1;
    assert_eq!(
        seats::apply_reconciliation_transition(context, conn, stale, budget).unwrap(),
        ReconciliationOutcome::Stale
    );
    let mut stale = transition.clone();
    stale.expected_terminal = Some(TerminalId::new("changed-terminal"));
    assert_eq!(
        seats::apply_reconciliation_transition(context, conn, stale, budget).unwrap(),
        ReconciliationOutcome::Stale
    );
}

#[test]
fn review_operator_rebind_new_incarnation_then_absence() {
    use crate::ports::*;
    use crate::protocol::{
        authority::{OperatorActor, PeerIdentity},
        commands::{OperatorCommand, OperatorRebind},
        time::{CallBudget, Cancellation},
    };
    use crate::store::seats;
    let (context, mut conn, path, _) = fixture(100);
    conn.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,terminal_id,incarnation,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,ended_at) VALUES ('s1',1,'s1','old-terminal','old-incarnation','b',1,1,'codex','old-native','old-execution','verified_current_target',50,50,90)",[]).unwrap();
    conn.execute(
        "UPDATE seats SET state='unresolved',unresolved_reason='other',generation=2 WHERE id='s1'",
        [],
    )
    .unwrap();
    let budget = CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Cancellation::default(),
    };
    let mut capture = snapshot_for_test(2, &["repair-target"]);
    capture.epoch = 2;
    capture.incarnation = IncarnationEvidence::Verified {
        identity: "new-incarnation".into(),
        evidence_kind: EvidenceKind::CoherentEnumeration,
    };
    capture.targets[0].epoch = 2;
    capture.targets[0].terminal = Some(TerminalId::new("new-terminal"));
    let a = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let stage = staged_test_snapshot(&context, &mut conn, a, &capture, &budget);
    seats::publish_snapshot_stage(&context, &mut conn, &stage, &budget).unwrap();
    let mut fresh = capture.targets[0].clone();
    fresh.provenance = ObservationProvenance::FreshCurrentTarget;
    fresh.observation_sequence = 3;
    fresh.incarnation = IncarnationEvidence::Verified {
        identity: "new-incarnation".into(),
        evidence_kind: EvidenceKind::NativeCurrentTarget,
    };
    let a = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    assert!(
        seats::publish_current_target_observation(&context, &mut conn, &a, &fresh, &budget)
            .unwrap()
    );
    let command = OperatorRebind {
        seat: SeatId::new("s1"),
        target: fresh.target.clone(),
        operation: OperationId::new("review-rebind"),
    };
    let guard = OperatorTargetGuard::try_new("i", &OperatorCommand::Rebind(command.clone()), fresh)
        .unwrap();
    let actor = OperatorActor::from_peer(PeerIdentity::from_kernel(501), 501).unwrap();
    assert!(matches!(
        seats::mutate_operator(
            &context,
            &mut conn,
            "i",
            OperatorRequest::Rebind(command, guard),
            actor,
            None
        )
        .unwrap(),
        CommandResult::OperatorRebound(_)
    ));
    assert_eq!(
        conn.query_row(
            "SELECT structural_incarnation FROM seats WHERE id='s1'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "new-incarnation"
    );
    seed_structural_continuity_obligations(&conn);
    let mut absent = snapshot_for_test(4, &[]);
    absent.epoch = 2;
    absent.incarnation = capture.incarnation.clone();
    let a = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let stage = staged_test_snapshot(&context, &mut conn, a, &absent, &budget);
    seats::publish_snapshot_stage(&context, &mut conn, &stage, &budget).unwrap();
    let page = seats::saved_seats_page(&context, &conn, &stage, 0, None, 16, &budget).unwrap();
    let ts = crate::identity::reconcile::plan_page(&page).unwrap();
    let t = ts.into_iter().find(|t| t.seat.as_str() == "s1").unwrap();
    assert!(matches!(
        t.action,
        ReconciliationAction::BeginRetirement { .. }
    ));
    reject_changed_structural_continuity(&context, &mut conn, &t, &budget);
    drop(conn);
    let mut conn = context.open_writer().unwrap();
    let outcome =
        seats::apply_reconciliation_transition(&context, &mut conn, t.clone(), &budget).unwrap();
    assert!(
        matches!(outcome, ReconciliationOutcome::RetirementStarted(_)),
        "authoritative absence after qualified repair returned {outcome:?}"
    );
    assert_eq!(
        seats::apply_reconciliation_transition(&context, &mut conn, t, &budget).unwrap(),
        ReconciliationOutcome::Stale
    );
    assert_eq!(
        crate::store::effective::effective_receipt(&conn, "fix3-message", "s1")
            .unwrap()
            .unwrap()
            .state,
        crate::store::effective::EffectiveReceiptState::RecipientRetired
    );
    assert_eq!(
        conn.query_row(
            "SELECT state FROM receipts WHERE message_id='fix3-message'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "pending"
    );
    assert_eq!(
        conn.query_row(
            "SELECT incarnation,registered_at,ended_at FROM occupant_bindings WHERE seat_id='s1'",
            [],
            |r| Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?
            ))
        )
        .unwrap(),
        ("old-incarnation".into(), 50, 90)
    );
    let ReconciliationOutcome::RetirementStarted(job) = outcome else {
        unreachable!()
    };
    drop(conn);
    let mut conn = context.open_writer().unwrap();
    let mut completed = false;
    for _ in 0..16 {
        let progress = advance_retirement(
            &context,
            &mut conn,
            job.id.clone(),
            crate::ports::WorkAdmission::Background,
            &budget,
        )
        .unwrap();
        assert!(progress.processed_this_turn <= 16);
        if progress.complete {
            completed = true;
            break;
        }
    }
    assert!(completed);
    assert_eq!(
        conn.query_row(
            "SELECT state,acked_at FROM receipts WHERE message_id='fix3-message'",
            [],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?))
        )
        .unwrap(),
        ("recipient_retired".into(), None)
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}
#[test]
fn ordinary_resolution_guard_rejects_unqualified_or_mismatched_read() {
    use crate::ports::{
        EvidenceKind, IncarnationEvidence, ObservationProvenance, OrdinaryResolutionGuard,
    };
    use crate::protocol::{
        commands::ResolveSeat,
        time::{CallBudget, Cancellation},
    };
    let (context, mut conn, path, _) = fixture(100);
    let budget = CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Cancellation::default(),
    };
    let admission =
        crate::store::seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let request = ResolveSeat {
        target: HostTargetId::new("pane-new"),
        operation: OperationId::new("resolve"),
    };
    let mut observation = snapshot_for_test(2, &["pane-new"]).targets.remove(0);
    observation.terminal = Some(TerminalId::new("terminal-new"));
    observation.incarnation = IncarnationEvidence::Verified {
        identity: "test-incarnation".into(),
        evidence_kind: EvidenceKind::NativeCurrentTarget,
    };
    assert!(OrdinaryResolutionGuard::try_new(&request, observation.clone(), &admission).is_err());
    observation.provenance = ObservationProvenance::FreshCurrentTarget;
    assert!(OrdinaryResolutionGuard::try_new(&request, observation.clone(), &admission).is_ok());
    observation.target = HostTargetId::new("different-target");
    assert!(OrdinaryResolutionGuard::try_new(&request, observation.clone(), &admission).is_err());
    observation.target = request.target.clone();
    observation.incarnation = IncarnationEvidence::Verified {
        identity: "test-incarnation".into(),
        evidence_kind: EvidenceKind::NativeInvocation,
    };
    assert!(OrdinaryResolutionGuard::try_new(&request, observation, &admission).is_err());
    drop(conn);
    let _ = std::fs::remove_file(path);
}

fn ordinary_resolution_fixture() -> (
    StoreContext,
    rusqlite::Connection,
    std::path::PathBuf,
    Arc<crate::store::SqliteStore>,
    crate::ports::HostObservation,
    crate::protocol::time::CallBudget,
) {
    use crate::protocol::time::{CallBudget, Cancellation};
    let (context, mut conn, path, clock) = fixture(100);
    // Remove this fixture's derived archival children before its seeded seats.
    conn.execute("DELETE FROM seat_archival", []).unwrap();
    conn.execute("DELETE FROM seats", []).unwrap();
    let budget = CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Cancellation::default(),
    };
    let mut snapshot = snapshot_for_test(1, &["pane-new"]);
    snapshot.targets[0].terminal = Some(TerminalId::new("terminal-new"));
    let admission =
        crate::store::seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let stage = staged_test_snapshot(&context, &mut conn, admission, &snapshot, &budget);
    crate::store::seats::publish_snapshot_stage(&context, &mut conn, &stage, &budget).unwrap();
    let store = Arc::new(
        crate::store::SqliteStore::new(
            StoreContext::new(path.clone(), clock),
            "i",
            crate::store::StoreSettings::default(),
        )
        .unwrap(),
    );
    let mut observation = snapshot.targets.remove(0);
    observation.provenance = crate::ports::ObservationProvenance::FreshCurrentTarget;
    observation.incarnation = crate::ports::IncarnationEvidence::Verified {
        identity: "test-incarnation".into(),
        evidence_kind: crate::ports::EvidenceKind::NativeCurrentTarget,
    };
    (context, conn, path, store, observation, budget)
}

fn ordinary_resolution_read(
    store: &crate::store::SqliteStore,
    observation: &crate::ports::HostObservation,
    sequence: u64,
    budget: &crate::protocol::time::CallBudget,
) -> (
    crate::ports::HostObservation,
    crate::ports::HostObservationAdmission,
) {
    use crate::ports::StorePort;
    let admission = store.begin_host_observation("i", budget).unwrap();
    let observation = crate::ports::HostObservation {
        observation_sequence: sequence,
        ..observation.clone()
    };
    assert!(
        store
            .publish_current_target_observation(&admission, &observation, budget)
            .unwrap()
    );
    (observation, admission)
}

#[test]
fn ordinary_resolution_returns_existing_owner_and_replays_through_host_loss_and_retirement() {
    use crate::ports::{
        ClosureEvidence, HostInvalidationReason, OrdinaryResolutionAttempt as Attempt,
        OrdinaryResolutionGuard, OrdinaryResolutionOutcome as Outcome, StorePort,
    };
    use crate::protocol::{commands::ResolveSeat, results::ErrorCode};
    let (context, mut conn, path, store, observation, budget) = ordinary_resolution_fixture();
    let first = ResolveSeat {
        target: observation.target.clone(),
        operation: OperationId::new("first-resolve"),
    };
    assert_eq!(
        store
            .resolve_seat(first.clone(), Attempt::ReplayOnly, &budget)
            .unwrap(),
        Outcome::NeedsObservation
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM operations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let (current, admission) = ordinary_resolution_read(&store, &observation, 2, &budget);
    let guard = OrdinaryResolutionGuard::try_new(&first, current.clone(), &admission).unwrap();
    let Outcome::Resolved(seat) = store
        .resolve_seat(first.clone(), Attempt::Observed(guard), &budget)
        .unwrap()
    else {
        panic!("seat missing")
    };
    let second = ResolveSeat {
        operation: OperationId::new("second-resolve"),
        ..first.clone()
    };
    let (current, admission) = ordinary_resolution_read(&store, &observation, 3, &budget);
    let before: (i64,i64,i64) = conn.query_row("SELECT send_eligibility_revision,lifecycle_revision,(SELECT count(*) FROM allocation_decisions) FROM host_instances WHERE id='i'", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    let guard = OrdinaryResolutionGuard::try_new(&second, current.clone(), &admission).unwrap();
    assert_eq!(
        store
            .resolve_seat(second, Attempt::Observed(guard), &budget)
            .unwrap(),
        Outcome::Resolved(seat.clone())
    );

    let after: (i64,i64,i64) = conn.query_row("SELECT send_eligibility_revision,lifecycle_revision,(SELECT count(*) FROM allocation_decisions) FROM host_instances WHERE id='i'", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(before, after);
    for table in [
        "occupant_bindings",
        "seat_availability",
        "memberships",
        "receipts",
        "warning_offer",
        "work_jobs",
    ] {
        assert_eq!(
            conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    begin_retirement(
        &context,
        &mut conn,
        seat.clone(),
        ClosureEvidence {
            host_boot: HostBootId::new("b"),
            epoch: 1,
            target: observation.target.clone(),
            generation: 1,
        },
    )
    .unwrap();
    let failed = store.begin_host_observation("i", &budget).unwrap();
    store
        .invalidate_host_observation(&failed, HostInvalidationReason::HostUnavailable, &budget)
        .unwrap();
    assert_eq!(
        store
            .resolve_seat(first.clone(), Attempt::ReplayOnly, &budget)
            .unwrap(),
        Outcome::Resolved(seat.clone())
    );
    // The deciding observed invocation must recheck replay before rejecting the
    // now-invalid evidence, covering a commit between initial miss and decision.
    assert_eq!(
        store
            .resolve_seat(
                first.clone(),
                Attempt::Observed(
                    OrdinaryResolutionGuard::try_new(&first, current, &admission).unwrap()
                ),
                &budget
            )
            .unwrap(),
        Outcome::Resolved(seat.clone())
    );
    let different = ResolveSeat {
        target: HostTargetId::new("different-target"),
        ..first
    };
    assert_eq!(
        store
            .resolve_seat(different, Attempt::ReplayOnly, &budget)
            .unwrap_err()
            .code,
        ErrorCode::OperationPayloadMismatch
    );
    assert_eq!(
        conn.query_row(
            "SELECT state FROM seats WHERE id=?1",
            [seat.as_str()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "retired"
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM seats", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(store);
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn ordinary_resolution_two_keys_share_allocation_and_operation_namespace() {
    use crate::ports::{
        OrdinaryResolutionAttempt as Attempt, OrdinaryResolutionGuard,
        OrdinaryResolutionOutcome as Outcome, StorePort,
    };
    use crate::protocol::commands::ResolveSeat;
    let (context, mut conn, path, store, observation, budget) = ordinary_resolution_fixture();
    let (current, admission) = ordinary_resolution_read(&store, &observation, 2, &budget);
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let mut workers = Vec::new();
    for key in ["concurrent-a", "concurrent-b"] {
        let store = store.clone();
        let barrier = barrier.clone();
        let budget = budget.clone();
        let request = ResolveSeat {
            target: current.target.clone(),
            operation: OperationId::new(key),
        };
        let guard =
            OrdinaryResolutionGuard::try_new(&request, current.clone(), &admission).unwrap();
        workers.push(std::thread::spawn(move || {
            barrier.wait();
            store
                .resolve_seat(request, Attempt::Observed(guard), &budget)
                .unwrap()
        }));
    }
    barrier.wait();
    let first = workers.remove(0).join().unwrap();
    assert_eq!(workers.remove(0).join().unwrap(), first);
    let Outcome::Resolved(seat) = first else {
        panic!("seat missing")
    };
    assert_eq!(
        conn.query_row("SELECT count(*) FROM seats", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM allocation_decisions", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM operations WHERE actor_scope='service-allocation:i'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    let replay = ResolveSeat {
        target: current.target.clone(),
        operation: OperationId::new("concurrent-a"),
    };
    assert_eq!(
        crate::store::seats::resolve_seat(
            &context,
            &mut conn,
            "i",
            replay,
            Attempt::ReplayOnly,
            &budget,
        )
        .unwrap(),
        Outcome::Resolved(seat)
    );
    drop(store);
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn final_resolved_target_check_rejects_hold_retirement_and_changed_structure() {
    use crate::ports::{
        ClosureEvidence, OrdinaryResolutionAttempt as Attempt, OrdinaryResolutionGuard,
        OrdinaryResolutionOutcome as Outcome, ResolvedTargetCheck, StorePort,
    };
    use crate::protocol::{commands::ResolveSeat, results::ErrorCode};
    for change in [
        "hold",
        "retire",
        "unresolved",
        "generation",
        "binding",
        "wrong-seat",
        "superseded",
    ] {
        let (context, mut conn, path, store, observation, budget) = ordinary_resolution_fixture();
        let request = ResolveSeat {
            target: observation.target.clone(),
            operation: OperationId::new("resolve"),
        };
        let (current, admission) = ordinary_resolution_read(&store, &observation, 2, &budget);
        let Outcome::Resolved(seat) = store
            .resolve_seat(
                request.clone(),
                Attempt::Observed(
                    OrdinaryResolutionGuard::try_new(&request, current, &admission).unwrap(),
                ),
                &budget,
            )
            .unwrap()
        else {
            panic!("seat missing")
        };
        let (current, admission) = ordinary_resolution_read(&store, &observation, 3, &budget);
        let guard =
            || OrdinaryResolutionGuard::try_new(&request, current.clone(), &admission).unwrap();
        store
            .check_resolved_target(
                ResolvedTargetCheck {
                    expected_seat: seat.clone(),
                    guard: guard(),
                },
                &budget,
            )
            .unwrap();
        let expected = match change {
            "hold" => {
                conn.execute("INSERT INTO recovery_holds(instance_id,target_id,baseline_boot,baseline_epoch,reason) VALUES ('i','pane-new','b',1,'explicit operator hold')",[]).unwrap();
                ErrorCode::TargetUnresolved
            }
            "retire" => {
                begin_retirement(
                    &context,
                    &mut conn,
                    seat.clone(),
                    ClosureEvidence {
                        host_boot: HostBootId::new("b"),
                        epoch: 1,
                        target: observation.target.clone(),
                        generation: 1,
                    },
                )
                .unwrap();
                ErrorCode::TargetUnresolved
            }
            "unresolved" => {
                conn.execute(
                    "UPDATE seats SET state='unresolved',unresolved_reason='other' WHERE id=?1",
                    [seat.as_str()],
                )
                .unwrap();
                conn.execute("UPDATE host_instances SET lifecycle_revision=lifecycle_revision+1 WHERE id='i'", []).unwrap();
                ErrorCode::TargetUnresolved
            }
            "generation" => {
                conn.execute(
                    "UPDATE seats SET target_generation=2 WHERE id=?1",
                    [seat.as_str()],
                )
                .unwrap();
                ErrorCode::TargetUnresolved
            }
            "binding" => {
                conn.execute(
                    "UPDATE seats SET generation=generation+1 WHERE id=?1",
                    [seat.as_str()],
                )
                .unwrap();
                conn.execute("UPDATE host_instances SET lifecycle_revision=lifecycle_revision+1 WHERE id='i'", []).unwrap();
                ErrorCode::StaleHostObservation
            }
            "wrong-seat" => ErrorCode::TargetUnresolved,
            "superseded" => {
                store.begin_host_observation("i", &budget).unwrap();
                ErrorCode::StaleHostObservation
            }
            _ => unreachable!(),
        };
        let checked_seat = if change == "wrong-seat" {
            SeatId::new("different-owner")
        } else {
            seat.clone()
        };
        assert_eq!(
            store
                .check_resolved_target(
                    ResolvedTargetCheck {
                        expected_seat: checked_seat,
                        guard: guard()
                    },
                    &budget
                )
                .unwrap_err()
                .code,
            expected,
            "{change}"
        );
        let count: i64 = conn
            .query_row("SELECT count(*) FROM operations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(
            store
                .resolve_seat(request.clone(), Attempt::ReplayOnly, &budget)
                .unwrap(),
            Outcome::Resolved(seat.clone())
        );
        let fresh_request = ResolveSeat {
            operation: OperationId::new("new-key"),
            ..request
        };
        let fresh_result = store.resolve_seat(
            fresh_request.clone(),
            Attempt::Observed(
                OrdinaryResolutionGuard::try_new(&fresh_request, current, &admission).unwrap(),
            ),
            &budget,
        );
        if change == "wrong-seat" || change == "binding" {
            assert_eq!(fresh_result.unwrap(), Outcome::Resolved(seat));
        } else {
            assert!(fresh_result.is_err(), "{change}");
            assert_eq!(
                conn.query_row("SELECT count(*) FROM operations", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                1
            );
        }
        drop(store);
        drop(conn);
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn ordinary_resolution_enforces_durable_allocation_classes_after_reopen() {
    use crate::ports::{
        OrdinaryResolutionAttempt as Attempt, OrdinaryResolutionGuard,
        OrdinaryResolutionOutcome as Outcome, StorePort,
    };
    use crate::protocol::commands::ResolveSeat;
    for state in [
        "unknown-baseline",
        "held-baseline",
        "released",
        "created-after",
    ] {
        let (context, conn, path, store, mut observation, budget) = ordinary_resolution_fixture();
        if state == "created-after" {
            observation.target = HostTargetId::new("created-after");
            observation.terminal = Some(TerminalId::new("terminal-created-after"));
        }
        let request = ResolveSeat {
            target: observation.target.clone(),
            operation: OperationId::new("resolve"),
        };
        let (current, admission) = ordinary_resolution_read(&store, &observation, 2, &budget);
        match state {
            "unknown-baseline" => {
                conn.execute(
                    "UPDATE host_instances SET recovery_baseline_generation_id=NULL WHERE id='i'",
                    [],
                )
                .unwrap();
            }
            "held-baseline" => {
                conn.execute(
                    "UPDATE host_instances SET baseline_hold_unclaimed=1 WHERE id='i'",
                    [],
                )
                .unwrap();
            }
            "released" => {
                conn.execute("INSERT INTO recovery_baseline_releases(instance_id,baseline_generation_id,target_id,decision_seq) SELECT 'i',recovery_baseline_generation_id,'pane-new',1 FROM host_instances WHERE id='i'",[]).unwrap();
            }
            _ => (),
        }
        drop(conn);
        let conn = context.open_writer().unwrap();
        let result = store.resolve_seat(
            request.clone(),
            Attempt::Observed(
                OrdinaryResolutionGuard::try_new(&request, current, &admission).unwrap(),
            ),
            &budget,
        );
        if state == "created-after" {
            assert!(matches!(result.unwrap(), Outcome::Resolved(_)));
        } else {
            assert!(result.is_err(), "{state}");
            assert_eq!(
                conn.query_row("SELECT count(*) FROM seats", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                0
            );
            assert_eq!(
                conn.query_row("SELECT count(*) FROM operations", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                0
            );
        }
        drop(store);
        drop(conn);
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn ordinary_resolution_cancelled_writer_wait_has_no_seat_or_operation_effect() {
    use crate::ports::{OrdinaryResolutionAttempt as Attempt, OrdinaryResolutionGuard, StorePort};
    use crate::protocol::{commands::ResolveSeat, results::ErrorCode};
    let (_context, conn, path, store, observation, budget) = ordinary_resolution_fixture();
    let request = ResolveSeat {
        target: observation.target.clone(),
        operation: OperationId::new("cancelled-resolve"),
    };
    let (current, admission) = ordinary_resolution_read(&store, &observation, 2, &budget);
    let guard = OrdinaryResolutionGuard::try_new(&request, current, &admission).unwrap();
    conn.execute_batch("BEGIN IMMEDIATE").unwrap();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let task_store = store.clone();
    let task_budget = budget.clone();
    let worker = std::thread::spawn(move || {
        ready_tx.send(()).unwrap();
        task_store.resolve_seat(request, Attempt::Observed(guard), &task_budget)
    });
    ready_rx.recv().unwrap();
    budget.cancellation.cancel();
    conn.execute_batch("ROLLBACK").unwrap();
    let error = worker.join().unwrap().unwrap_err();
    assert_eq!(error.code, ErrorCode::Cancelled, "{error:?}");
    assert_eq!(
        conn.query_row("SELECT count(*) FROM seats", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM operations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(store);
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn retirement_quantum_failure_retains_bounded_error_until_successful_progress() {
    use crate::{
        ports::{ClosureEvidence, WorkAdmission},
        protocol::time::{CallBudget, Cancellation},
    };
    let (context, mut conn, path, clock) = fixture(100);
    conn.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence) VALUES ('fail-thread','i','topic','goal',0,0,3)", []).unwrap();
    conn.execute("INSERT INTO memberships(thread_id,seat_id,state,joined_at) VALUES ('fail-thread','s2','joined',0)", []).unwrap();
    conn.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('fail-thread','s2',1,1)", []).unwrap();
    for n in 1..=2 {
        let id = format!("fail-message-{n}");
        conn.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('i',?1,'fail-thread',?2,'ordinary','body',0,?2)", rusqlite::params![id, n]).unwrap();
        conn.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) VALUES (?1,'fail-thread','s2','pending',90,0,90)", [&id]).unwrap();
    }
    conn.execute("UPDATE host_instances SET decision_seq=10 WHERE id='i'", [])
        .unwrap();
    let job = begin_retirement(
        &context,
        &mut conn,
        SeatId::new("s2"),
        ClosureEvidence {
            host_boot: HostBootId::new("b"),
            epoch: 1,
            target: HostTargetId::new("s2"),
            generation: 1,
        },
    )
    .unwrap();
    clock.0.store(200, Ordering::SeqCst);
    let budget = CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Cancellation::default(),
    };
    // A real SQLite constraint abort on the obligation write, not a seeded
    // last_error. Its message is longer than the persisted diagnostic bound.
    let injected = format!("injected receipt retirement failure {}", "é".repeat(400));
    conn.execute_batch(&format!(
        "CREATE TRIGGER fail_retire_receipt BEFORE UPDATE ON receipts WHEN NEW.state='recipient_retired' BEGIN SELECT RAISE(ABORT,'{injected}'); END;"
    ))
    .unwrap();
    let durable = |conn: &Connection| -> (String, String, i64, Option<String>) {
        conn.query_row(
            "SELECT status,phase,processed_units,last_error FROM retirements WHERE id=?1",
            [job.id.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap()
    };
    let count =
        |conn: &Connection, sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };

    let failed = advance_retirement(
        &context,
        &mut conn,
        job.id.clone(),
        WorkAdmission::Background,
        &budget,
    )
    .unwrap_err();
    // The caller still receives the real storage failure.
    assert_eq!(failed.code, ErrorCode::Conflict);
    assert!(
        failed
            .detail
            .contains("injected receipt retirement failure")
    );
    // The whole quantum rolled back: cursor and counters did not move, and
    // only the bounded diagnostic committed.
    let (status, phase, processed, stored) = durable(&conn);
    assert_eq!(
        (status.as_str(), phase.as_str(), processed),
        ("pending", "select_thread", 0)
    );
    let error = stored.clone().expect("failed quantum retains diagnostic");
    assert!(error.starts_with("SQLite: injected receipt retirement failure"));
    assert!(failed.detail.starts_with(&error));
    assert_eq!(error.chars().count(), 256);
    assert!(error.len() <= 512);
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM receipts WHERE state='recipient_retired'"
        ),
        0
    );
    assert_eq!(
        count(&conn, "SELECT count(*) FROM messages WHERE kind='warn'"),
        0
    );
    // Logical retirement is never undone by a failed cleanup quantum.
    assert_eq!(
        conn.query_row("SELECT state FROM seats WHERE id='s2'", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "retired"
    );

    // A repeated failure keeps the cursor and the retained diagnostic.
    assert!(
        advance_retirement(
            &context,
            &mut conn,
            job.id.clone(),
            WorkAdmission::Background,
            &budget,
        )
        .is_err()
    );
    assert_eq!(
        durable(&conn),
        (status.clone(), phase.clone(), 0, stored.clone())
    );

    // The error survives reopen, and a later successful quantum clears it.
    drop(conn);
    let mut conn = context.open_writer().unwrap();
    assert_eq!(durable(&conn).3.as_deref(), Some(error.as_str()));
    conn.execute_batch("DROP TRIGGER fail_retire_receipt;")
        .unwrap();
    let recovered = advance_retirement(
        &context,
        &mut conn,
        job.id.clone(),
        WorkAdmission::Background,
        &budget,
    )
    .unwrap();
    assert!(recovered.complete);
    assert!(recovered.last_error.is_none());
    let (status, phase, processed, stored) = durable(&conn);
    assert_eq!((status.as_str(), phase.as_str()), ("complete", "complete"));
    assert_eq!(processed, i64::from(recovered.processed_this_turn));
    assert_eq!(stored, None);
    assert_eq!(
        count(
            &conn,
            "SELECT count(*) FROM receipts WHERE state='recipient_retired'"
        ),
        2
    );
    assert_eq!(
        count(&conn, "SELECT count(*) FROM messages WHERE kind='warn'"),
        2
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn partial_retirement_progress_after_failure_clears_error_before_completion() {
    use crate::{
        ports::{ClosureEvidence, WorkAdmission},
        protocol::time::{CallBudget, Cancellation},
    };
    let (context, mut conn, path, clock) = fixture(100);
    conn.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence) VALUES ('long-thread','i','topic','goal',0,0,31)", []).unwrap();
    conn.execute("INSERT INTO memberships(thread_id,seat_id,state,joined_at) VALUES ('long-thread','s2','joined',0)", []).unwrap();
    conn.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('long-thread','s2',1,1)", []).unwrap();
    for n in 1..=30 {
        let id = format!("long-message-{n}");
        conn.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('i',?1,'long-thread',?2,'ordinary','body',0,?2)", rusqlite::params![id, n]).unwrap();
        conn.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) VALUES (?1,'long-thread','s2','pending',90,0,90)", [&id]).unwrap();
    }
    conn.execute("UPDATE host_instances SET decision_seq=40 WHERE id='i'", [])
        .unwrap();
    let job = begin_retirement(
        &context,
        &mut conn,
        SeatId::new("s2"),
        ClosureEvidence {
            host_boot: HostBootId::new("b"),
            epoch: 1,
            target: HostTargetId::new("s2"),
            generation: 1,
        },
    )
    .unwrap();
    clock.0.store(200, Ordering::SeqCst);
    let budget = CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Cancellation::default(),
    };
    conn.execute_batch("CREATE TRIGGER fail_retire_receipt BEFORE UPDATE ON receipts WHEN NEW.state='recipient_retired' BEGIN SELECT RAISE(ABORT,'transient receipt failure'); END;").unwrap();
    assert!(
        advance_retirement(
            &context,
            &mut conn,
            job.id.clone(),
            WorkAdmission::Background,
            &budget,
        )
        .is_err()
    );
    let retained: Option<String> = conn
        .query_row(
            "SELECT last_error FROM retirements WHERE id=?1",
            [job.id.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        retained.as_deref(),
        Some("SQLite: transient receipt failure")
    );
    conn.execute_batch("DROP TRIGGER fail_retire_receipt;")
        .unwrap();
    let progressed = advance_retirement(
        &context,
        &mut conn,
        job.id.clone(),
        WorkAdmission::Background,
        &budget,
    )
    .unwrap();
    assert_eq!(progressed.processed_this_turn, 16);
    assert!(!progressed.complete);
    assert!(progressed.last_error.is_none());
    let row: (String, Option<String>) = conn
        .query_row(
            "SELECT status,last_error FROM retirements WHERE id=?1",
            [job.id.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(row, ("pending".into(), None));
    drop(conn);
    let _ = std::fs::remove_file(path);
}

/// Each monotonic read advances past the retirement work window, so a quantum
/// that starts cleanly ends before its first unit.
struct IdleQuantumClock(std::sync::atomic::AtomicU64);
impl Clock for IdleQuantumClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(200)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.fetch_add(
            crate::ports::RETIREMENT_WORK_MILLIS,
            std::sync::atomic::Ordering::SeqCst,
        ))
    }
}

#[test]
fn idle_retirement_quantum_returns_and_keeps_retained_error() {
    // Kills: `let last_error = if progressed { None } else { prior_error }`
    // mutated to always `None` (the idle quantum then reports no error), and
    // the durable `last_error=CASE WHEN ?11 THEN NULL ELSE last_error END`
    // mutated to always clear (the retained diagnostic then disappears).
    use crate::{
        ports::{ClosureEvidence, WorkAdmission},
        protocol::time::{CallBudget, Cancellation},
    };
    let (context, mut conn, path, clock) = fixture(100);
    conn.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence) VALUES ('idle-thread','i','topic','goal',0,0,3)", []).unwrap();
    conn.execute("INSERT INTO memberships(thread_id,seat_id,state,joined_at) VALUES ('idle-thread','s2','joined',0)", []).unwrap();
    conn.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('idle-thread','s2',1,1)", []).unwrap();
    for n in 1..=2 {
        let id = format!("idle-message-{n}");
        conn.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('i',?1,'idle-thread',?2,'ordinary','body',0,?2)", rusqlite::params![id, n]).unwrap();
        conn.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) VALUES (?1,'idle-thread','s2','pending',90,0,90)", [&id]).unwrap();
    }
    conn.execute("UPDATE host_instances SET decision_seq=10 WHERE id='i'", [])
        .unwrap();
    let job = begin_retirement(
        &context,
        &mut conn,
        SeatId::new("s2"),
        ClosureEvidence {
            host_boot: HostBootId::new("b"),
            epoch: 1,
            target: HostTargetId::new("s2"),
            generation: 1,
        },
    )
    .unwrap();
    clock.0.store(200, Ordering::SeqCst);
    conn.execute_batch("CREATE TRIGGER fail_retire_receipt BEFORE UPDATE ON receipts WHEN NEW.state='recipient_retired' BEGIN SELECT RAISE(ABORT,'retained receipt failure'); END;").unwrap();
    assert!(
        advance_retirement(
            &context,
            &mut conn,
            job.id.clone(),
            WorkAdmission::Background,
            &CallBudget {
                deadline: MonoInstant(1000),
                cancellation: Cancellation::default(),
            },
        )
        .is_err()
    );
    // The idle quantum below is not caused by the failure injection.
    conn.execute_batch("DROP TRIGGER fail_retire_receipt;")
        .unwrap();
    let durable = |conn: &Connection| -> (String, String, i64, Option<String>) {
        conn.query_row(
            "SELECT status,phase,processed_units,last_error FROM retirements WHERE id=?1",
            [job.id.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap()
    };
    let before = durable(&conn);
    assert_eq!(
        before.3.as_deref(),
        Some("SQLite: retained receipt failure")
    );
    drop(conn);

    let idle_context = StoreContext::new(
        path.clone(),
        Arc::new(IdleQuantumClock(std::sync::atomic::AtomicU64::new(1_000))),
    );
    let mut idle_conn = idle_context.open_writer().unwrap();
    let idle = advance_retirement(
        &idle_context,
        &mut idle_conn,
        job.id.clone(),
        WorkAdmission::Background,
        &CallBudget {
            deadline: MonoInstant(1 << 40),
            cancellation: Cancellation::default(),
        },
    )
    .unwrap();
    assert_eq!(idle.processed_this_turn, 0, "the quantum must be idle");
    assert!(!idle.complete);
    assert_eq!(
        idle.last_error.as_ref().map(|error| error.as_str()),
        Some("SQLite: retained receipt failure")
    );
    assert_eq!(durable(&idle_conn), before);
    drop(idle_conn);
    let _ = std::fs::remove_file(path);
}

/// Kills: a registration stored without the terminal/incarnation of its
/// verified effective observation (unreconfirmable after invalidation), or a
/// registration that silently proceeds when that evidence is missing.
#[test]
fn registration_requires_and_records_reconfirmation_evidence() {
    use crate::{
        protocol::commands::{CheckIn, CheckInMode},
        store::seats,
    };
    let (context, mut conn, path, _) = fixture(100);
    let command = CheckIn {
        mode: CheckInMode::Lifecycle {
            expected_binding_generation: 1,
        },
        claim: claim("s2"),
        operation: OperationId::new("check-in-1"),
    };
    let digest =
        crate::store::schema::canonical_digest(&seats::check_in_payload(&command)).unwrap();
    let budget = crate::protocol::time::CallBudget {
        deadline: crate::protocol::time::MonoInstant(u64::MAX),
        cancellation: Default::default(),
    };
    fn empty<T>() -> crate::protocol::pagination::Page<T> {
        crate::protocol::pagination::Page {
            items: vec![],
            next_cursor: None,
            next_argv: None,
            high_water_ordinal: 0,
            scope_revision: None,
            has_more: false,
            stop_reason: crate::protocol::pagination::StopReason::Complete,
            consistency: crate::protocol::pagination::Consistency::BoundedLive,
        }
    }
    let offer = |_tx: &rusqlite::Transaction<'_>, seat: &SeatId, seq: u64| {
        Ok(crate::protocol::results::CheckInResult {
            context_disposition: crate::protocol::results::CheckInContextDisposition::Current,
            context: claim(seat.as_str()),
            seat: seat.clone(),
            offered_through: Some(seq.to_string()),
            warning_count: 0,
            warning_count_has_more: false,
            warnings: empty(),
            inbox: empty(),
            notices: crate::protocol::results::NoticeOffer::default(),
        })
    };
    conn.execute("UPDATE observed_targets SET terminal_id=NULL,incarnation=NULL,incarnation_source_kind=NULL,connection_epoch=NULL WHERE target_id='s2'", []).unwrap();
    let error = seats::register_available(
        &context,
        &mut conn,
        &command,
        None,
        permit(
            "s2",
            "check-in-1",
            ObligationRef::CheckIn(SeatId::new("s2")),
            digest,
            100,
        ),
        &budget,
        offer,
    )
    .unwrap_err();
    assert_eq!(
        error.code,
        crate::protocol::results::ErrorCode::StaleHostObservation,
        "{error:?}"
    );
    let bindings = |conn: &Connection| -> Vec<(Option<String>, Option<String>)> {
        let mut stmt = conn
            .prepare("SELECT terminal_id,incarnation FROM occupant_bindings WHERE seat_id='s2'")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert!(bindings(&conn).is_empty());
    conn.execute("UPDATE observed_targets SET terminal_id='term-s2',incarnation='inc',incarnation_source_kind='coherent_enumeration',connection_epoch=1 WHERE target_id='s2'", []).unwrap();
    seats::register_available(
        &context,
        &mut conn,
        &command,
        None,
        permit(
            "s2",
            "check-in-1",
            ObligationRef::CheckIn(SeatId::new("s2")),
            digest,
            100,
        ),
        &budget,
        offer,
    )
    .unwrap();
    assert_eq!(
        bindings(&conn),
        vec![(Some("term-s2".into()), Some("inc".into()))]
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

// Demo-1 P5: accept wrote `harness:"Claude"` + `native_session`, ACK and send
// wrote `harness:"claude"` + `session`. Kills: a writer that keeps debug
// casing or `native_session`, a reader that leaves legacy accept rows in the
// old shape, and a reader that rewrites current or opaque rows.
#[test]
fn accountable_observation_has_one_shape_and_legacy_rows_read_back_in_it() {
    let actor = AccountableActor {
        harness: crate::protocol::authority::Harness::Claude,
        native_session: crate::protocol::ids::NativeSessionId::new("sess-1"),
        execution: crate::protocol::ids::ExecutionId::new("exec-1"),
        host_boot: crate::protocol::ids::HostBootId::new("boot-1"),
        target_generation: 3,
        binding_generation: 2,
        observed_at_utc: UtcMillis(10),
        provenance: "cooperative_top_level",
    };
    let written = actor.observation(11);
    let value: serde_json::Value = serde_json::from_str(&written).unwrap();
    assert_eq!(value["harness"], "claude");
    assert_eq!(value["session"], "sess-1");
    assert!(value.get("native_session").is_none());
    assert_eq!(value["decided_at"], 11);
    assert_eq!(value["provenance"], "cooperative_top_level");
    // Current rows (and opaque fixtures) are returned byte for byte.
    assert_eq!(canonical_observation(&written), written);
    assert_eq!(canonical_observation("obs"), "obs");
    // A legacy accept row reads back in the current shape, keeping its values.
    let legacy = r#"{"harness":"Claude","native_session":"sess-1","execution":"exec-1","host_boot":"boot-1","observed_at":10,"provenance":"cooperative_top_level"}"#;
    let read: serde_json::Value = serde_json::from_str(&canonical_observation(legacy)).unwrap();
    assert_eq!(read["harness"], "claude");
    assert_eq!(read["session"], "sess-1");
    assert!(read.get("native_session").is_none());
    assert_eq!(read["execution"], "exec-1");
    assert_eq!(read["provenance"], "cooperative_top_level");
}

// ---- B5 trust guards (ht-rzi.1): hold lift, retire, replace ----

fn b5_budget() -> crate::protocol::time::CallBudget {
    crate::protocol::time::CallBudget {
        deadline: MonoInstant(1_000),
        cancellation: crate::protocol::time::Cancellation::default(),
    }
}

fn b5_actor() -> crate::protocol::authority::OperatorActor {
    use crate::protocol::authority::{OperatorActor, PeerIdentity};
    OperatorActor::from_peer(PeerIdentity::from_kernel(501), 501).unwrap()
}

fn b5_fresh_observation(target: &str) -> crate::ports::HostObservation {
    use crate::ports::{
        ExecutionEvidence, HostObservation, HostUiState, IncarnationEvidence,
        ObservationProvenance, StructuralOccupancy,
    };
    HostObservation {
        focused: false,
        target: HostTargetId::new(target),
        host_boot: HostBootId::new("b"),
        epoch: 1,
        generation: 1,
        observed_at_utc: UtcMillis(100),
        observed_at_mono: MonoInstant(100),
        provenance: ObservationProvenance::FreshCurrentTarget,
        occupant: None,
        ui: HostUiState::Idle,
        terminal: None,
        occupancy: StructuralOccupancy::EmptyShell,
        incarnation: IncarnationEvidence::Unknown,
        execution: ExecutionEvidence::Unknown,
        call_id: HostCallId::new(format!("b5-call-{target}")),
        connection_epoch: 1,
        observation_sequence: 1,
        started_at_mono: MonoInstant(100),
        completed_at_mono: MonoInstant(100),
    }
}

/// Publish a baseline holding `targets` with `s1` unresolved; returns the
/// published snapshot. Leaves the restore flag set and the marker NULL.
fn b5_published_baseline(
    context: &StoreContext,
    conn: &mut Connection,
    targets: &[&str],
) -> crate::ports::PublishedSnapshot {
    use crate::store::seats;
    conn.execute(
        "UPDATE seats SET state='unresolved',target_id=NULL WHERE id='s1'",
        [],
    )
    .unwrap();
    let budget = b5_budget();
    let admission = seats::begin_host_observation(context, conn, "i", &budget).unwrap();
    let snapshot = snapshot_for_test(1, targets);
    let stage = staged_test_snapshot(context, conn, admission, &snapshot, &budget);
    seats::publish_snapshot_stage(context, conn, &stage, &budget).unwrap()
}

fn b5_hold_state(conn: &Connection) -> (i64, i64) {
    conn.query_row(
        "SELECT baseline_hold_unclaimed,(SELECT count(*) FROM recovery_holds WHERE instance_id='i' AND released_at IS NULL) FROM host_instances WHERE id='i'",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .unwrap()
}

fn b5_set_marker_to_recovery(conn: &Connection) {
    conn.execute(
        "UPDATE host_instances SET reconciled_boot=recovery_boot,reconciled_epoch=recovery_epoch WHERE id='i'",
        [],
    )
    .unwrap();
}

fn b5_lift(context: &StoreContext, conn: &mut Connection) -> bool {
    let _ = context;
    let tx = conn.transaction().unwrap();
    let lifted =
        crate::store::seats::lift_baseline_hold_if_clear(&tx, "i", UtcMillis(100)).unwrap();
    tx.commit().unwrap();
    lifted
}

fn b5_add_hold(conn: &Connection, instance: &str, target: &str) {
    conn.execute(
        "INSERT INTO recovery_holds(instance_id,target_id,baseline_boot,baseline_epoch,reason) VALUES (?1,?2,'b',1,'unresolved saved seat continuity')",
        rusqlite::params![instance, target],
    )
    .unwrap();
}

#[test]
fn lift_requires_reconciliation_marker_for_current_recovery_boot_epoch() {
    let (context, mut conn, path, _) = fixture(100);
    b5_published_baseline(&context, &mut conn, &["new-target"]);
    conn.execute(
        "INSERT INTO host_instances(id, created_at, host_boot, host_epoch, baseline_hold_unclaimed) VALUES ('other', 0, 'b', 1, 1)",
        [],
    )
    .unwrap();
    b5_add_hold(&conn, "i", "new-target");
    b5_add_hold(&conn, "other", "other-target");
    conn.execute(
        "UPDATE seats SET state='retired',retired_at=1 WHERE id='s1'",
        [],
    )
    .unwrap();
    assert_eq!(b5_hold_state(&conn), (1, 1));
    // Marker NULL: nothing is lifted although no unresolved seat remains.
    assert!(!b5_lift(&context, &mut conn));
    assert_eq!(b5_hold_state(&conn), (1, 1));
    // Marker for another epoch: still lagging.
    conn.execute(
        "UPDATE host_instances SET reconciled_boot=recovery_boot,reconciled_epoch=recovery_epoch+1 WHERE id='i'",
        [],
    )
    .unwrap();
    assert!(!b5_lift(&context, &mut conn));
    assert_eq!(b5_hold_state(&conn), (1, 1));
    b5_set_marker_to_recovery(&conn);
    assert!(b5_lift(&context, &mut conn));
    assert_eq!(b5_hold_state(&conn), (0, 0));
    let other: (i64, i64) = conn
        .query_row(
            "SELECT baseline_hold_unclaimed,(SELECT count(*) FROM recovery_holds WHERE instance_id='other' AND released_at IS NULL) FROM host_instances WHERE id='other'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(other, (1, 1), "other instances are untouched");
    // Nothing left to lift: the helper reports false the second time.
    assert!(!b5_lift(&context, &mut conn));
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn lift_does_not_fire_while_an_unresolved_seat_remains() {
    let (context, mut conn, path, _) = fixture(100);
    b5_published_baseline(&context, &mut conn, &["new-target"]);
    b5_add_hold(&conn, "i", "new-target");
    b5_set_marker_to_recovery(&conn);
    assert!(!b5_lift(&context, &mut conn), "s1 is still unresolved");
    assert_eq!(b5_hold_state(&conn), (1, 1));
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn explicit_hold_pane_resolves_after_last_unresolved_seat_is_retired() {
    use crate::store::{effective, effective::EffectiveRecoveryDisposition as Disposition, seats};
    let (context, mut conn, path, _) = fixture(100);
    b5_published_baseline(&context, &mut conn, &["pane-h"]);
    b5_add_hold(&conn, "i", "pane-h");
    b5_set_marker_to_recovery(&conn);
    assert_eq!(
        effective::effective_recovery_disposition(&conn, "i", "pane-h").unwrap(),
        Disposition::ExplicitHold
    );
    conn.execute(
        "INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','pane-h','b',1,1,1,100,'fresh','term-pane-h','inc','coherent_enumeration',1)",
        [],
    )
    .unwrap();
    // Retire the last unresolved seat through the operator command.
    let command = crate::protocol::commands::OperatorRetire {
        seat: SeatId::new("s1"),
        operation: OperationId::new("retire-last"),
    };
    seats::mutate_operator(
        &context,
        &mut conn,
        "i",
        crate::ports::OperatorRequest::Retire(command),
        b5_actor(),
        None,
    )
    .unwrap();
    assert_eq!(b5_hold_state(&conn), (0, 0));
    let disposition = effective::effective_recovery_disposition(&conn, "i", "pane-h").unwrap();
    assert!(
        !matches!(
            disposition,
            Disposition::ExplicitHold | Disposition::BaselineHeld
        ),
        "{disposition:?}"
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn operator_fresh_seat_lifts_when_no_unresolved_remains() {
    use crate::{
        ports::{OperatorRequest, OperatorTargetGuard},
        protocol::commands::{OperatorCommand, OperatorFreshSeat},
        store::seats,
    };
    let (context, mut conn, path, _) = fixture(100);
    b5_published_baseline(&context, &mut conn, &["new-target"]);
    b5_add_hold(&conn, "i", "new-target");
    b5_set_marker_to_recovery(&conn);
    // The unresolved seat is retired by other means; the hold lingers.
    conn.execute(
        "UPDATE seats SET state='retired',retired_at=1 WHERE id='s1'",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','new-target','b',1,1,3,100,'fresh','term-new-target','inc','coherent_enumeration',1)",
        [],
    )
    .unwrap();
    let command = OperatorFreshSeat {
        target: HostTargetId::new("new-target"),
        operation: OperationId::new("fresh-lift"),
    };
    let guard = OperatorTargetGuard::try_new(
        "i",
        &OperatorCommand::FreshSeat(command.clone()),
        b5_fresh_observation("new-target"),
    )
    .unwrap();
    seats::mutate_operator(
        &context,
        &mut conn,
        "i",
        OperatorRequest::FreshSeat(command, guard),
        b5_actor(),
        None,
    )
    .unwrap();
    assert_eq!(b5_hold_state(&conn), (0, 0));
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn operator_rebind_of_last_unresolved_seat_lifts_holds_in_same_transaction() {
    use crate::{
        ports::{OperatorRequest, OperatorTargetGuard},
        protocol::commands::{OperatorCommand, OperatorRebind},
        store::seats,
    };
    let (context, mut conn, path, _) = fixture(100);
    b5_published_baseline(&context, &mut conn, &["new-target", "held-elsewhere"]);
    b5_add_hold(&conn, "i", "held-elsewhere");
    // Marker lags: the rebind must not lift.
    conn.execute(
        "INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','new-target','b',1,1,3,100,'fresh','term-new-target','inc','coherent_enumeration',1)",
        [],
    )
    .unwrap();
    let rebind = |operation: &str| OperatorRebind {
        seat: SeatId::new("s1"),
        target: HostTargetId::new("new-target"),
        operation: OperationId::new(operation),
    };
    let command = rebind("rebind-lag");
    let guard = OperatorTargetGuard::try_new(
        "i",
        &OperatorCommand::Rebind(command.clone()),
        b5_fresh_observation("new-target"),
    )
    .unwrap();
    seats::mutate_operator(
        &context,
        &mut conn,
        "i",
        OperatorRequest::Rebind(command, guard),
        b5_actor(),
        None,
    )
    .unwrap();
    assert_eq!(
        b5_hold_state(&conn),
        (1, 1),
        "rebind with a lagging marker keeps the hold"
    );
    // Same shape with the marker current: the rebind itself lifts.
    let (context, mut conn2, path2, _) = fixture(100);
    b5_published_baseline(&context, &mut conn2, &["new-target", "held-elsewhere"]);
    b5_add_hold(&conn2, "i", "held-elsewhere");
    b5_set_marker_to_recovery(&conn2);
    conn2.execute(
        "INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','new-target','b',1,1,3,100,'fresh','term-new-target','inc','coherent_enumeration',1)",
        [],
    )
    .unwrap();
    let command = rebind("rebind-lift");
    let guard = OperatorTargetGuard::try_new(
        "i",
        &OperatorCommand::Rebind(command.clone()),
        b5_fresh_observation("new-target"),
    )
    .unwrap();
    seats::mutate_operator(
        &context,
        &mut conn2,
        "i",
        OperatorRequest::Rebind(command, guard),
        b5_actor(),
        None,
    )
    .unwrap();
    assert_eq!(b5_hold_state(&conn2), (0, 0));
    drop(conn);
    drop(conn2);
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(path2);
}

#[test]
fn record_reconciliation_pass_writes_marker_only_for_current_recovery_publication() {
    use crate::store::seats;
    let (context, mut conn, path, _) = fixture(100);
    let first = b5_published_baseline(&context, &mut conn, &["t1"]);
    let marker = |conn: &Connection| -> (Option<String>, Option<i64>) {
        conn.query_row(
            "SELECT reconciled_boot,reconciled_epoch FROM host_instances WHERE id='i'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    };
    let budget = b5_budget();
    // A publication of another epoch is not the current recovery baseline.
    let mut other_epoch = first.clone();
    other_epoch.epoch += 1;
    assert!(
        !seats::record_reconciliation_pass(&context, &mut conn, &other_epoch, &budget).unwrap()
    );
    assert_eq!(marker(&conn), (None, None));
    // The current publication writes the marker; s1 is still unresolved, so
    // the lift inside the same transaction leaves the hold alone.
    assert!(seats::record_reconciliation_pass(&context, &mut conn, &first, &budget).unwrap());
    assert_eq!(marker(&conn), (Some("b".into()), Some(1)));
    assert_eq!(b5_hold_state(&conn).0, 1);
    // With nothing unresolved the next recorded pass lifts the hold.
    conn.execute(
        "UPDATE seats SET state='retired',retired_at=1 WHERE id='s1'",
        [],
    )
    .unwrap();
    b5_add_hold(&conn, "i", "t1");
    conn.execute(
        "UPDATE host_instances SET reconciled_boot=NULL,reconciled_epoch=NULL WHERE id='i'",
        [],
    )
    .unwrap();
    assert!(seats::record_reconciliation_pass(&context, &mut conn, &first, &budget).unwrap());
    assert_eq!(b5_hold_state(&conn), (0, 0));
    // A superseded publication writes nothing.
    let admission = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let second = snapshot_for_test(2, &["t1"]);
    let stage = staged_test_snapshot(&context, &mut conn, admission, &second, &budget);
    seats::publish_snapshot_stage(&context, &mut conn, &stage, &budget).unwrap();
    conn.execute(
        "UPDATE host_instances SET reconciled_boot=NULL,reconciled_epoch=NULL WHERE id='i'",
        [],
    )
    .unwrap();
    assert!(!seats::record_reconciliation_pass(&context, &mut conn, &first, &budget).unwrap());
    assert_eq!(marker(&conn), (None, None));
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn daemon_start_clears_stuck_flag_and_open_holds_when_marker_matches() {
    use crate::store::{SqliteStore, StoreSettings};
    for (marker_matches, expected) in [(true, (0, 0)), (false, (1, 1))] {
        let (context, mut conn, path, clock) = fixture(100);
        b5_published_baseline(&context, &mut conn, &["t1"]);
        conn.execute(
            "UPDATE seats SET state='retired',retired_at=1 WHERE id='s1'",
            [],
        )
        .unwrap();
        b5_add_hold(&conn, "i", "t1");
        if marker_matches {
            b5_set_marker_to_recovery(&conn);
        }
        assert_eq!(b5_hold_state(&conn), (1, 1));
        let store = SqliteStore::new(
            StoreContext::new(path.clone(), clock),
            "i",
            StoreSettings::default(),
        )
        .unwrap();
        assert_eq!(
            b5_hold_state(&conn),
            expected,
            "marker_matches={marker_matches}"
        );
        drop(store);
        drop(context);
        drop(conn);
        let _ = std::fs::remove_file(path);
    }
    // A fresh store without the instance row starts cleanly.
    let (context, conn, path, _) = fixture(100);
    // Remove this fixture's derived archival children before its seeded seats.
    conn.execute("DELETE FROM seat_archival", []).unwrap();
    conn.execute("DELETE FROM seats", []).unwrap();
    conn.execute("DELETE FROM observed_targets", []).unwrap();
    conn.execute("DELETE FROM host_instances", []).unwrap();
    SqliteStore::new(context, "i", StoreSettings::default()).unwrap();
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn reconciliation_retirement_of_last_unresolved_seat_lifts() {
    use crate::ports::{GuardedSeatTransition, ReconciliationAction, ReconciliationOutcome};
    use crate::store::seats;
    let (context, mut conn, path, _) = fixture(100);
    conn.execute(
        "INSERT INTO occupant_bindings(seat_id,generation,target_id,terminal_id,incarnation,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at) VALUES ('s1',1,'s1','terminal-1','test-incarnation','b',1,1,'codex','native-1','execution-1','verified_current_target',100)",
        [],
    )
    .unwrap();
    let budget = b5_budget();
    let admission = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let empty = snapshot_for_test(2, &[]);
    let stage = staged_test_snapshot(&context, &mut conn, admission, &empty, &budget);
    let publication = seats::publish_snapshot_stage(&context, &mut conn, &stage, &budget).unwrap();
    // Stuck state: a flag and a hold with no unresolved seat, marker current.
    conn.execute(
        "UPDATE host_instances SET baseline_hold_unclaimed=1 WHERE id='i'",
        [],
    )
    .unwrap();
    b5_add_hold(&conn, "i", "s1");
    b5_set_marker_to_recovery(&conn);
    assert_eq!(b5_hold_state(&conn), (1, 1));
    let outcome = seats::apply_reconciliation_transition(
        &context,
        &mut conn,
        GuardedSeatTransition {
            publication,
            seat: SeatId::new("s1"),
            expected_binding_generation: 1,
            expected_target: Some(HostTargetId::new("s1")),
            expected_terminal: Some(TerminalId::new("terminal-1")),
            action: ReconciliationAction::BeginRetirement {
                absent_target: HostTargetId::new("s1"),
            },
        },
        &budget,
    )
    .unwrap();
    assert!(matches!(
        outcome,
        ReconciliationOutcome::RetirementStarted(_)
    ));
    assert_eq!(b5_hold_state(&conn), (0, 0));
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn reconfirm_structure_of_last_unresolved_seat_lifts_and_not_before() {
    use crate::ports::{
        GuardedInvalidationTransition, GuardedSeatTransition, HostInvalidationReason, HostUiState,
        ReconciliationAction, ReconciliationOutcome, StructuralOccupancy,
    };
    use crate::store::seats;
    let (context, mut conn, path, _) = fixture(100);
    for seat in ["s1", "s2"] {
        conn.execute(
            "INSERT INTO occupant_bindings(seat_id,generation,target_id,terminal_id,incarnation,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,registered_at) VALUES (?1,1,?1,'term-'||?1,'test-incarnation','b',1,1,'codex','plugin_context:x','e-'||?1,'cooperative_top_level',1,1)",
            [seat],
        )
        .unwrap();
    }
    let budget = b5_budget();
    let production = |sequence: u64| {
        let mut snapshot = snapshot_for_test(sequence, &["s1", "s2"]);
        for target in &mut snapshot.targets {
            target.terminal = Some(TerminalId::new(format!("term-{}", target.target.as_str())));
            target.ui = HostUiState::Unknown;
            target.occupancy = StructuralOccupancy::Unknown;
        }
        snapshot
    };
    let admission = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let stage = staged_test_snapshot(&context, &mut conn, admission, &production(2), &budget);
    seats::publish_snapshot_stage(&context, &mut conn, &stage, &budget).unwrap();
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
    for seat in ["s1", "s2"] {
        assert_eq!(
            seats::mark_unresolved_from_invalidation(
                &context,
                &mut conn,
                GuardedInvalidationTransition {
                    fence: fence.clone(),
                    seat: SeatId::new(seat),
                    expected_binding_generation: 1,
                    expected_target: Some(HostTargetId::new(seat)),
                    expected_terminal: Some(TerminalId::new(format!("term-{seat}"))),
                },
                &budget
            )
            .unwrap(),
            ReconciliationOutcome::Applied
        );
    }
    let admission = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let stage = staged_test_snapshot(&context, &mut conn, admission, &production(3), &budget);
    let publication = seats::publish_snapshot_stage(&context, &mut conn, &stage, &budget).unwrap();
    b5_add_hold(&conn, "i", "s1");
    b5_set_marker_to_recovery(&conn);
    assert_eq!(b5_hold_state(&conn), (1, 1));
    let reconfirm = |seat: &str, conn: &mut Connection| {
        seats::apply_reconciliation_transition(
            &context,
            conn,
            GuardedSeatTransition {
                publication: publication.clone(),
                seat: SeatId::new(seat),
                expected_binding_generation: 2,
                expected_target: Some(HostTargetId::new(seat)),
                expected_terminal: Some(TerminalId::new(format!("term-{seat}"))),
                action: ReconciliationAction::ReconfirmStructure {
                    target: HostTargetId::new(seat),
                    terminal: TerminalId::new(format!("term-{seat}")),
                },
            },
            &budget,
        )
        .unwrap()
    };
    assert_eq!(reconfirm("s1", &mut conn), ReconciliationOutcome::Applied);
    assert_eq!(b5_hold_state(&conn), (1, 1), "s2 is still unresolved");
    assert_eq!(reconfirm("s2", &mut conn), ReconciliationOutcome::Applied);
    assert_eq!(b5_hold_state(&conn), (0, 0), "no hold after full reconfirm");
    drop(conn);
    let _ = std::fs::remove_file(path);
}

fn b5_seed_obligations(conn: &Connection, seat: &str, tag: &str) {
    let (joined, invited) = (format!("{tag}-joined"), format!("{tag}-invited"));
    for thread in [&joined, &invited] {
        conn.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES (?1,'i','topic','goal',0,0)", [thread]).unwrap();
    }
    conn.execute(
        "INSERT INTO memberships(thread_id,seat_id,state,joined_at) VALUES (?1,?2,'joined',0)",
        [&joined, seat],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES (?1,?2,1,1)",
        [&joined, seat],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO memberships(thread_id,seat_id,state) VALUES (?1,?2,'invited')",
        [&invited, seat],
    )
    .unwrap();
    conn.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,frozen_duration_ms,deadline_at,created_decision_seq) VALUES (?1,?2,?3,1,'pending',0,90,90,1)",
        [&format!("{tag}-invite"), &invited, seat]).unwrap();
    let message = format!("{tag}-message");
    conn.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('i',?1,?2,1,'ordinary','body',0,?3)", rusqlite::params![message, joined, if tag == "new" { 2 } else { 3 }]).unwrap();
    conn.execute("UPDATE host_instances SET decision_seq=30 WHERE id='i'", [])
        .unwrap();
    conn.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) VALUES (?1,?2,?3,'pending',90,0,90)", [&message, &joined, seat]).unwrap();
    conn.execute("UPDATE threads SET next_sequence=2 WHERE id=?1", [&joined])
        .unwrap();
}

fn b5_obligation_states(conn: &Connection, seat: &str) -> (String, String) {
    let invitation: String = conn
        .query_row(
            "SELECT state FROM invitations WHERE seat_id=?1",
            [seat],
            |r| r.get(0),
        )
        .unwrap();
    let receipt: String = conn
        .query_row("SELECT state FROM receipts WHERE seat_id=?1", [seat], |r| {
            r.get(0)
        })
        .unwrap();
    (invitation, receipt)
}

fn b5_drain_retirement(
    context: &StoreContext,
    conn: &mut Connection,
    clock: &FixedClock,
    seat: &str,
) {
    use crate::ports::WorkAdmission;
    clock.0.store(200, Ordering::SeqCst);
    let job: String = conn
        .query_row("SELECT id FROM retirements WHERE seat_id=?1", [seat], |r| {
            r.get(0)
        })
        .unwrap();
    for _ in 0..16 {
        let progress = advance_retirement(
            context,
            conn,
            RetirementJobId::new(job.clone()),
            WorkAdmission::Background,
            &b5_budget(),
        )
        .unwrap();
        if progress.complete {
            return;
        }
    }
    panic!("retirement did not complete");
}

#[test]
fn operator_retire_retires_settles_and_audits() {
    use crate::{ports::OperatorRequest, protocol::commands::OperatorRetire, store::seats};
    let (context, mut conn, path, clock) = fixture(100);
    b5_seed_obligations(&conn, "s2", "rt");
    conn.execute(
        "INSERT INTO occupant_bindings(seat_id,generation,target_id,terminal_id,incarnation,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at) VALUES ('s2',1,'s2','terminal-2','inc','b',1,1,'codex','native-2','execution-2','verified_current_target',100)",
        [],
    )
    .unwrap();
    let command = OperatorRetire {
        seat: SeatId::new("s2"),
        operation: OperationId::new("retire-s2"),
    };
    let run = |conn: &mut Connection, command: &OperatorRetire| {
        seats::mutate_operator(
            &context,
            conn,
            "i",
            OperatorRequest::Retire(command.clone()),
            b5_actor(),
            None,
        )
    };
    assert_eq!(
        run(&mut conn, &command).unwrap(),
        CommandResult::OperatorRetired(SeatId::new("s2"))
    );
    let seat: (String, i64) = conn
        .query_row(
            "SELECT state,(SELECT count(*) FROM occupant_bindings WHERE seat_id='s2' AND ended_at IS NULL) FROM seats WHERE id='s2'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(seat, ("retired".into(), 0));
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM retirements WHERE seat_id='s2'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        b5_obligation_states(&conn, "s2"),
        ("pending".into(), "pending".into()),
        "settlement is the retirement worker's bounded work"
    );
    b5_drain_retirement(&context, &mut conn, &clock, "s2");
    assert_eq!(
        b5_obligation_states(&conn, "s2"),
        ("recipient_retired".into(), "recipient_retired".into())
    );
    let audit: Vec<(String, Option<String>, String)> = conn
        .prepare("SELECT kind,operator_label,seat_id FROM allocation_decisions")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        audit,
        vec![(
            "operator_retire".to_owned(),
            Some("operator:local-user:501".to_owned()),
            "s2".to_owned()
        )]
    );
    let labelled: i64 = conn
        .query_row(
            "SELECT (SELECT count(*) FROM messages WHERE body LIKE '%operator:local-user%' OR coalesce(event_json,'') LIKE '%operator:local-user%')+(SELECT count(*) FROM receipts WHERE coalesce(ack_actor_seat_id,'') LIKE '%operator:local-user%')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(labelled, 0, "the label is audit-only, never on receipts");
    // Replay with the same key returns the stored result and writes nothing.
    assert_eq!(
        run(&mut conn, &command).unwrap(),
        CommandResult::OperatorRetired(SeatId::new("s2"))
    );
    // A different key for the retired seat is refused without a new row.
    let again = OperatorRetire {
        operation: OperationId::new("retire-s2-again"),
        ..command.clone()
    };
    assert_eq!(
        run(&mut conn, &again).unwrap_err().code,
        ErrorCode::NotFound
    );
    let unknown = OperatorRetire {
        seat: SeatId::new("nobody"),
        operation: OperationId::new("retire-nobody"),
    };
    assert_eq!(
        run(&mut conn, &unknown).unwrap_err().code,
        ErrorCode::NotFound
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM allocation_decisions", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

fn b5_replace_fixture() -> (StoreContext, Connection, PathBuf, Arc<FixedClock>) {
    let (context, conn, path, clock) = fixture(100);
    // OLD = s1, unresolved. NEW = s2, resolved on its own pane "s2".
    conn.execute(
        "UPDATE seats SET state='unresolved',target_id=NULL WHERE id='s1'",
        [],
    )
    .unwrap();
    conn.execute(
        "UPDATE observed_targets SET observation_sequence=3 WHERE target_id='s2'",
        [],
    )
    .unwrap();
    b5_seed_obligations(&conn, "s2", "new");
    b5_seed_obligations(&conn, "s1", "old");
    (context, conn, path, clock)
}

fn b5_replace_request(
    old: &str,
    pane: &str,
    new: &str,
    operation: &str,
) -> crate::ports::OperatorRequest {
    use crate::{
        ports::{OperatorRequest, OperatorTargetGuard},
        protocol::commands::{OperatorCommand, OperatorReplace},
    };
    let command = OperatorReplace {
        seat: SeatId::new(old),
        target: HostTargetId::new(pane),
        replace: SeatId::new(new),
        operation: OperationId::new(operation),
    };
    let guard = OperatorTargetGuard::try_new(
        "i",
        &OperatorCommand::Replace(command.clone()),
        b5_fresh_observation(pane),
    )
    .unwrap();
    OperatorRequest::Replace(command, guard)
}

#[test]
fn operator_replace_retires_new_and_rebinds_old_atomically() {
    use crate::store::seats;
    let (context, mut conn, path, clock) = b5_replace_fixture();
    let old_before = (
        b5_obligation_states(&conn, "s1"),
        conn.query_row(
            "SELECT count(*) FROM memberships WHERE seat_id='s1'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap(),
    );
    let result = seats::mutate_operator(
        &context,
        &mut conn,
        "i",
        b5_replace_request("s1", "s2", "s2", "replace-1"),
        b5_actor(),
        None,
    )
    .unwrap();
    assert_eq!(result, CommandResult::OperatorRebound(SeatId::new("s1")));
    let seat_row = |conn: &Connection, seat: &str| -> (String, Option<String>) {
        conn.query_row(
            "SELECT state,target_id FROM seats WHERE id=?1",
            [seat],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    };
    assert_eq!(
        seat_row(&conn, "s1"),
        ("resolved".into(), Some("s2".into()))
    );
    assert_eq!(seat_row(&conn, "s2").0, "retired");
    // The pane is owned by exactly one live seat, and it is OLD: a concurrent
    // resolve serializes behind the decision and can only see OLD.
    let owners: Vec<String> = conn
        .prepare("SELECT id FROM seats WHERE target_id='s2' AND state='resolved'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(owners, vec!["s1".to_owned()]);
    assert_eq!(
        b5_obligation_states(&conn, "s2"),
        ("pending".into(), "pending".into())
    );
    b5_drain_retirement(&context, &mut conn, &clock, "s2");
    assert_eq!(
        b5_obligation_states(&conn, "s2"),
        ("recipient_retired".into(), "recipient_retired".into())
    );
    // Nothing moved from NEW to OLD.
    let old_after = (
        b5_obligation_states(&conn, "s1"),
        conn.query_row(
            "SELECT count(*) FROM memberships WHERE seat_id='s1'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap(),
    );
    assert_eq!(old_after, old_before);
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM memberships WHERE seat_id='s1' AND thread_id LIKE 'new-%'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let audit: Vec<(String, String, Option<String>)> = conn
        .prepare("SELECT kind,seat_id,operator_label FROM allocation_decisions ORDER BY ordinal")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let label = Some("operator:local-user:501".to_owned());
    assert_eq!(
        audit,
        vec![
            ("operator_retire".to_owned(), "s2".to_owned(), label.clone()),
            ("operator_rebind".to_owned(), "s1".to_owned(), label),
        ]
    );
    // Replay returns the stored result and writes nothing more.
    assert_eq!(
        seats::mutate_operator(
            &context,
            &mut conn,
            "i",
            b5_replace_request("s1", "s2", "s2", "replace-1"),
            b5_actor(),
            None,
        )
        .unwrap(),
        CommandResult::OperatorRebound(SeatId::new("s1"))
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM allocation_decisions", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn operator_replace_refuses_when_new_does_not_own_the_pane_and_writes_nothing() {
    use crate::store::seats;
    let (context, mut conn, path, _) = b5_replace_fixture();
    conn.execute("INSERT INTO seats(id, instance_id, state, role, target_id, generation, target_generation, created_at) VALUES ('s3', 'i', 'resolved', 'native', 's3', 1, 1, 0)", []).unwrap();
    for (new, operation) in [("s3", "replace-wrong-owner"), ("s1", "replace-self")] {
        let error = seats::mutate_operator(
            &context,
            &mut conn,
            "i",
            b5_replace_request("s1", "s2", new, operation),
            b5_actor(),
            None,
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::TargetAlreadyOwned, "{new}");
    }
    let written: (i64, i64, String, String) = conn
        .query_row(
            "SELECT (SELECT count(*) FROM allocation_decisions),(SELECT count(*) FROM retirements),(SELECT state FROM seats WHERE id='s1'),(SELECT state FROM seats WHERE id='s2')",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(written, (0, 0, "unresolved".into(), "resolved".into()));
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn rebind_onto_owned_target_lists_both_resolutions() {
    use crate::{
        ports::{OperatorRequest, OperatorTargetGuard},
        protocol::commands::{OperatorCommand, OperatorRebind},
        store::seats,
    };
    let (context, mut conn, path, _) = b5_replace_fixture();
    let command = OperatorRebind {
        seat: SeatId::new("s1"),
        target: HostTargetId::new("s2"),
        operation: OperationId::new("rebind-owned"),
    };
    let guard = OperatorTargetGuard::try_new(
        "i",
        &OperatorCommand::Rebind(command.clone()),
        b5_fresh_observation("s2"),
    )
    .unwrap();
    let error = seats::mutate_operator(
        &context,
        &mut conn,
        "i",
        OperatorRequest::Rebind(command, guard),
        b5_actor(),
        None,
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::TargetAlreadyOwned);
    assert!(
        error
            .detail
            .contains("herdr-threads human seat retire s1 --operator"),
        "{}",
        error.detail
    );
    assert!(
        error
            .detail
            .contains("herdr-threads human seat rebind s1 --pane s2 --replace s2 --operator"),
        "{}",
        error.detail
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

// ---- TRUST-POLICY C1 cooperative continuity (ht-rzi.2) ----

fn c1_observe(conn: &Connection, target: &str) {
    conn.execute(
        "INSERT OR REPLACE INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i',?1,'b',1,1,3,100,'fresh','term-'||?1,'inc','coherent_enumeration',1)",
        [target],
    )
    .unwrap();
}

fn c1_bind(conn: &Connection, seat: &str, generation: i64, harness: &str, session: &str) {
    conn.execute(
        "INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,ended_at) VALUES (?1,?2,1,'old-pane','b',1,?3,?4,'exec-'||?1||'-'||?2,'cooperative_top_level',10,10,50)",
        rusqlite::params![seat, generation, harness, session],
    )
    .unwrap();
}

fn c1_unresolved_seat(conn: &Connection, seat: &str, harness: &str, session: &str) {
    conn.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES (?1,'i','unresolved','native',NULL,1,1,0)", [seat]).unwrap();
    c1_bind(conn, seat, 1, harness, session);
}

fn c1_command(
    target: &str,
    session: &str,
    operation: &str,
) -> crate::protocol::commands::ContinuityCheckIn {
    crate::protocol::commands::ContinuityCheckIn {
        target: HostTargetId::new(target),
        harness: Harness::Claude,
        native_session: NativeSessionId::new(session),
        source: "resume".into(),
        operation: OperationId::new(operation),
        execution: ExecutionId::new(format!("exec-{operation}")),
    }
}

fn c1_decide(
    context: &StoreContext,
    conn: &mut Connection,
    command: &crate::protocol::commands::ContinuityCheckIn,
    diagnostic: &'static str,
) -> Result<CommandResult, crate::protocol::results::ApiError> {
    let guard = crate::ports::ContinuityTargetGuard::try_new(
        "i",
        command,
        b5_fresh_observation(command.target.as_str()),
    )
    .unwrap();
    crate::store::seats::decide_continuity(
        context,
        conn,
        "i",
        crate::ports::ContinuityRequest {
            command: command.clone(),
            guard,
            diagnostic,
        },
    )
}

/// Seat `s1` unresolved with a latest `claude`/`sess-1` binding, `held-pane`
/// observed and held. Marker left NULL (no lift unless a test sets it).
fn c1_fixture() -> (StoreContext, Connection, PathBuf) {
    let (context, mut conn, path, _) = fixture(100);
    b5_published_baseline(&context, &mut conn, &["held-pane"]);
    c1_bind(&conn, "s1", 1, "claude", "sess-1");
    b5_add_hold(&conn, "i", "held-pane");
    c1_observe(&conn, "held-pane");
    (context, conn, path)
}

fn c1_tables_mentioning(conn: &Connection, needle: &str) -> Vec<String> {
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let mut found = Vec::new();
    for table in tables {
        let columns: Vec<String> = conn
            .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let any = columns
            .iter()
            .map(|c| format!("CAST(\"{c}\" AS TEXT) LIKE '%{needle}%'"))
            .collect::<Vec<_>>()
            .join(" OR ");
        let hit: bool = conn
            .query_row(
                &format!("SELECT EXISTS(SELECT 1 FROM \"{table}\" WHERE {any})"),
                [],
                |r| r.get(0),
            )
            .unwrap();
        if hit {
            found.push(table);
        }
    }
    found
}

#[test]
fn continuity_reattaches_unique_session_match_on_held_target() {
    let (context, mut conn, path) = c1_fixture();
    let command = c1_command("held-pane", "sess-1", "c1-op");
    let result = c1_decide(&context, &mut conn, &command, "match").unwrap();
    assert_eq!(
        result,
        CommandResult::ContinuityReattached(crate::protocol::results::ContinuityReattachment {
            seat: SeatId::new("s1"),
            binding_generation: 2,
        })
    );
    let seat: (String, Option<String>, i64) = conn
        .query_row(
            "SELECT state,target_id,generation FROM seats WHERE id='s1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(seat, ("resolved".into(), Some("held-pane".into()), 2));
    let open: i64 = conn
        .query_row(
            "SELECT count(*) FROM occupant_bindings WHERE seat_id='s1' AND ended_at IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        open, 1,
        "the deciding transaction opens the successor binding"
    );
    let released: i64 = conn
        .query_row(
            "SELECT count(*) FROM recovery_holds WHERE target_id='held-pane' AND released_at IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(released, 1);
    let rows: Vec<(String, String, Option<String>, Option<String>)> = conn
        .prepare(
            "SELECT kind,seat_id,operator_label,continuity_diagnostic FROM allocation_decisions",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        rows,
        vec![(
            "cooperative_continuity".into(),
            "s1".into(),
            None,
            Some("match".into())
        )]
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn continuity_opens_successor_cooperative_binding_in_the_deciding_transaction() {
    let (context, mut conn, path) = c1_fixture();
    let command = c1_command("held-pane", "sess-1", "c1-open");
    let result = c1_decide(&context, &mut conn, &command, "match").unwrap();
    let CommandResult::ContinuityReattached(reply) = result else {
        panic!("wrong result");
    };
    assert_eq!(reply.seat.as_str(), "s1");
    let seat: (String, Option<String>, i64, i64) = conn
        .query_row(
            "SELECT state,target_id,generation,unavailability_open FROM seats WHERE id='s1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        seat,
        (
            "resolved".into(),
            Some("held-pane".into()),
            reply.binding_generation as i64,
            0
        )
    );
    let open: Vec<(String, String, String, String, i64, bool, String)> = conn
        .prepare("SELECT observation_provenance,harness,native_session,execution_id,generation,registered_at IS NOT NULL,target_id FROM occupant_bindings WHERE seat_id='s1' AND ended_at IS NULL")
        .unwrap()
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        open,
        vec![(
            "cooperative_top_level".into(),
            "claude".into(),
            "sess-1".into(),
            "exec-c1-open".into(),
            reply.binding_generation as i64,
            true,
            "held-pane".into()
        )]
    );
    let decisions: i64 = conn
        .query_row(
            "SELECT count(*) FROM allocation_decisions WHERE kind='cooperative_continuity' AND seat_id='s1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(decisions, 1);
    assert_eq!(
        crate::store::schema::effective_registered_availability(&conn, "s1", Some("i")).unwrap(),
        Some("cooperative_top_level".into())
    );
    let anchors: i64 = conn
        .query_row(
            "SELECT count(*) FROM seat_availability WHERE seat_id='s1' AND binding_generation=?1",
            [reply.binding_generation as i64],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(anchors, 1);
    let jobs: i64 = conn
        .query_row(
            "SELECT count(*) FROM work_jobs WHERE kind='receipt_timer_materialization'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(jobs, 1);
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn continuity_without_match_is_retryable_until_reconciled() {
    use crate::protocol::results::ErrorCode;
    let (context, mut conn, path) = c1_fixture();
    conn.execute(
        "UPDATE host_instances SET recovery_boot='b',recovery_epoch=2,reconciled_boot='b',reconciled_epoch=1 WHERE id='i'",
        [],
    )
    .unwrap();
    let command = c1_command("held-pane", "sess-other", "c1-lag");
    let lagging = c1_decide(&context, &mut conn, &command, "match").unwrap_err();
    assert_eq!(lagging.code, ErrorCode::ServiceBusy);
    assert!(
        lagging.detail.contains("has not finished"),
        "{}",
        lagging.detail
    );
    let stored: i64 = conn
        .query_row(
            "SELECT count(*) FROM operations WHERE operation_key='c1-lag'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        stored, 0,
        "a retryable refusal stores nothing under the key"
    );
    conn.execute(
        "UPDATE host_instances SET reconciled_boot=recovery_boot,reconciled_epoch=recovery_epoch WHERE id='i'",
        [],
    )
    .unwrap();
    let settled = c1_decide(&context, &mut conn, &command, "match").unwrap_err();
    assert_eq!(settled.code, ErrorCode::NotFound);
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn continuity_on_unowned_target_reattaches() {
    let (context, mut conn, path, _) = fixture(100);
    b5_published_baseline(&context, &mut conn, &["free-pane"]);
    c1_bind(&conn, "s1", 1, "claude", "sess-1");
    c1_observe(&conn, "free-pane");
    let command = c1_command("free-pane", "sess-1", "c1-free");
    let result = c1_decide(&context, &mut conn, &command, "absent").unwrap();
    assert!(matches!(
        result,
        CommandResult::ContinuityReattached(ref r) if r.seat.as_str() == "s1"
    ));
    let state: (String, Option<String>) = conn
        .query_row("SELECT state,target_id FROM seats WHERE id='s1'", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(state, ("resolved".into(), Some("free-pane".into())));
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn continuity_refuses_zero_and_multiple_matches() {
    use crate::protocol::results::ErrorCode;
    let (context, mut conn, path) = c1_fixture();
    // Reconciliation of the current epoch has finished, so "no unresolved
    // seat" is final (while it lags it is the retryable ServiceBusy).
    b5_set_marker_to_recovery(&conn);
    // Zero: no unresolved seat holds this session.
    let none = c1_decide(
        &context,
        &mut conn,
        &c1_command("held-pane", "sess-other", "c1-zero"),
        "match",
    )
    .unwrap_err();
    assert_eq!(none.code, ErrorCode::NotFound);
    // Several: a second unresolved seat carries the same last session.
    c1_unresolved_seat(&conn, "s3", "claude", "sess-1");
    let many = c1_decide(
        &context,
        &mut conn,
        &c1_command("held-pane", "sess-1", "c1-many"),
        "match",
    )
    .unwrap_err();
    assert_eq!(many.code, ErrorCode::Conflict);
    assert!(many.detail.contains("seat rebind --operator"));
    let after: (String, String, i64, i64) = conn
        .query_row(
            "SELECT (SELECT state FROM seats WHERE id='s1'),(SELECT state FROM seats WHERE id='s3'),(SELECT count(*) FROM recovery_holds WHERE target_id='held-pane' AND released_at IS NULL),(SELECT count(*) FROM allocation_decisions)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(after, ("unresolved".into(), "unresolved".into(), 1, 0));
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn continuity_never_matches_retired_sentinel_human_or_empty_sessions() {
    use crate::store::seats::continuity_candidates;
    let (_context, mut conn, path) = c1_fixture();
    // Retired seat with the session.
    conn.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,retired_at,retired_seq) VALUES ('s-retired','i','retired','native',NULL,1,1,0,5,1)", []).unwrap();
    c1_bind(&conn, "s-retired", 1, "claude", "sess-r");
    // Sentinel, human and empty values on unresolved seats.
    c1_unresolved_seat(&conn, "s-sentinel", "claude", "plugin_context:abc");
    c1_unresolved_seat(&conn, "s-human", "human", "sess-h");
    c1_unresolved_seat(&conn, "s-empty", "claude", "");
    // A different harness with the same id, and a superseded (not latest) binding.
    c1_unresolved_seat(&conn, "s-codex", "codex", "sess-c");
    c1_unresolved_seat(&conn, "s-old", "claude", "sess-old");
    c1_bind(&conn, "s-old", 2, "claude", "sess-new");
    let tx = conn.transaction().unwrap();
    let ask = |harness: &str, session: &str| {
        continuity_candidates(&tx, "i", harness, session)
            .unwrap()
            .into_iter()
            .map(|seat| seat.as_str().to_owned())
            .collect::<Vec<_>>()
    };
    assert!(ask("claude", "sess-r").is_empty(), "retired seat");
    assert!(ask("claude", "plugin_context:abc").is_empty(), "sentinel");
    assert!(ask("human", "sess-h").is_empty(), "human occupant");
    assert!(ask("claude", "").is_empty(), "empty");
    assert!(
        ask("claude", "sess-c").is_empty(),
        "other harness's session"
    );
    assert!(ask("claude", "sess-old").is_empty(), "superseded binding");
    // Controls: the live matches are found, so the empties above are real.
    assert_eq!(ask("codex", "sess-c"), vec!["s-codex"]);
    assert_eq!(ask("claude", "sess-new"), vec!["s-old"]);
    assert_eq!(ask("claude", "sess-1"), vec!["s1"]);
    drop(tx);
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn continuity_refuses_owned_target() {
    use crate::protocol::results::ErrorCode;
    let (context, mut conn, path) = c1_fixture();
    // Reconciliation of the current epoch has finished, so the resolved
    // owner is current and the refusal is final.
    b5_set_marker_to_recovery(&conn);
    // s2 resolved on target "s2" (fixture): never taken.
    c1_observe(&conn, "s2");
    let error = c1_decide(
        &context,
        &mut conn,
        &c1_command("s2", "sess-1", "c1-owned"),
        "match",
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::TargetAlreadyOwned);
    let state: String = conn
        .query_row("SELECT state FROM seats WHERE id='s1'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(state, "unresolved");
    drop(conn);
    let _ = std::fs::remove_file(path);
}

/// ht-p63: a restored Herdr that kept the pane's id, before the first
/// reconciliation pass of the recovery epoch. The target's resolved owner may
/// be a stale mapping the pass will unresolve, so the refusal is the
/// retryable `ServiceBusy` (nothing stored under the key, nothing taken);
/// once reconciled with the owner still resolved it is final.
#[test]
fn continuity_on_owned_target_is_retryable_until_reconciled() {
    use crate::protocol::results::ErrorCode;
    let (context, mut conn, path) = c1_fixture();
    conn.execute(
        "UPDATE host_instances SET recovery_boot='b',recovery_epoch=2,reconciled_boot='b',reconciled_epoch=1 WHERE id='i'",
        [],
    )
    .unwrap();
    c1_observe(&conn, "s2");
    let command = c1_command("s2", "sess-1", "c1-owned-lag");
    let lagging = c1_decide(&context, &mut conn, &command, "match").unwrap_err();
    assert_eq!(lagging.code, ErrorCode::ServiceBusy);
    assert!(
        lagging.detail.contains("has not finished"),
        "{}",
        lagging.detail
    );
    let (stored, s1, s2): (i64, String, String) = conn
        .query_row(
            "SELECT (SELECT count(*) FROM operations WHERE operation_key='c1-owned-lag'),(SELECT state FROM seats WHERE id='s1'),(SELECT state FROM seats WHERE id='s2')",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        (stored, s1.as_str(), s2.as_str()),
        (0, "unresolved", "resolved")
    );
    b5_set_marker_to_recovery(&conn);
    let settled = c1_decide(&context, &mut conn, &command, "match").unwrap_err();
    assert_eq!(settled.code, ErrorCode::TargetAlreadyOwned);
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn continuity_of_last_unresolved_seat_lifts_hold() {
    let (context, mut conn, path) = c1_fixture();
    b5_set_marker_to_recovery(&conn);
    assert_eq!(b5_hold_state(&conn), (1, 1));
    c1_decide(
        &context,
        &mut conn,
        &c1_command("held-pane", "sess-1", "c1-lift"),
        "mismatch",
    )
    .unwrap();
    assert_eq!(b5_hold_state(&conn), (0, 0));
    drop(conn);
    let _ = std::fs::remove_file(path);
    // With another unresolved seat left (or the marker lagging) the baseline
    // flag stays: only the target's own hold is released.
    let (context, mut conn, path) = c1_fixture();
    b5_set_marker_to_recovery(&conn);
    c1_unresolved_seat(&conn, "s3", "claude", "sess-3");
    c1_decide(
        &context,
        &mut conn,
        &c1_command("held-pane", "sess-1", "c1-nolift"),
        "match",
    )
    .unwrap();
    assert_eq!(b5_hold_state(&conn), (1, 0));
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn continuity_replay_returns_recorded_seat_and_generation() {
    let (context, mut conn, path) = c1_fixture();
    let command = c1_command("held-pane", "sess-1", "c1-replay");
    let first = c1_decide(&context, &mut conn, &command, "match").unwrap();
    let replayed =
        crate::store::seats::replay_continuity(&context, "i", &command, &b5_budget()).unwrap();
    assert_eq!(replayed, Some(first.clone()));
    // The same key and payload decided again is the recorded result.
    let again = c1_decide(&context, &mut conn, &command, "absent").unwrap();
    assert_eq!(again, first);
    let rows: (i64, Option<String>) = conn
        .query_row(
            "SELECT count(*),max(continuity_diagnostic) FROM allocation_decisions WHERE kind='cooperative_continuity'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        rows,
        (1, Some("match".into())),
        "no second row, first diagnostic kept"
    );
    let bindings: i64 = conn
        .query_row(
            "SELECT count(*) FROM occupant_bindings WHERE seat_id='s1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(bindings, 2, "replay adds no binding row");
    // Another payload under the same key is refused, not replayed.
    let other = c1_command("held-pane", "sess-2", "c1-replay");
    assert_eq!(
        crate::store::seats::replay_continuity(&context, "i", &other, &b5_budget())
            .unwrap_err()
            .code,
        crate::protocol::results::ErrorCode::OperationPayloadMismatch
    );
    assert_eq!(
        crate::store::seats::replay_continuity(
            &context,
            "i",
            &c1_command("held-pane", "sess-1", "c1-never-used"),
            &b5_budget()
        )
        .unwrap(),
        None
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn continuity_diagnostic_values_round_trip() {
    for diagnostic in ["match", "mismatch", "absent", "read_error"] {
        let (context, mut conn, path) = c1_fixture();
        c1_decide(
            &context,
            &mut conn,
            &c1_command("held-pane", "sess-1", "c1-diag"),
            diagnostic,
        )
        .unwrap();
        let stored: Option<String> = conn
            .query_row(
                "SELECT continuity_diagnostic FROM allocation_decisions",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored.as_deref(), Some(diagnostic));
        drop(conn);
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn continuity_never_writes_receipts() {
    let (context, mut conn, path) = c1_fixture();
    c1_decide(
        &context,
        &mut conn,
        &c1_command("held-pane", "sess-1", "c1-receipts"),
        "match",
    )
    .unwrap();
    // The value exists in seat history only: no receipt, binding or other
    // table carries it (A3).
    assert_eq!(
        c1_tables_mentioning(&conn, "cooperative_continuity"),
        vec!["allocation_decisions".to_owned()]
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

/// Every monotonic read steps 10 ms: each snapshot writer turn's quantum is
/// spent by its first unit.
struct SteppingClock(std::sync::atomic::AtomicU64);
impl Clock for SteppingClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(100)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.fetch_add(10, std::sync::atomic::Ordering::SeqCst))
    }
}

/// A writer turn whose 5 ms quantum is spent (CPU load, a stall) stages its
/// committed prefix, at least one target, and returns progress; the rest
/// goes in later turns and the snapshot publishes. Kills: failing the slice
/// with DeadlineExceeded (the lane then recorded PublicationFailed, a host
/// invalidation that unresolved seats and ended bindings on no host
/// evidence: ht-zo4), and a turn that stages nothing (no progress).
#[test]
fn spent_snapshot_quantum_commits_its_prefix_and_staging_resumes() {
    use crate::ports::{DurableWorkAdmission, SnapshotHeader};
    use crate::protocol::time::{CallBudget, Cancellation};
    use crate::store::seats;
    let (context, mut conn, path, _) = fixture(100);
    let budget = CallBudget {
        deadline: MonoInstant(u64::MAX / 2),
        cancellation: Cancellation::default(),
    };
    let snapshot = snapshot_for_test(2, &["q-1", "q-2", "q-3"]);
    let admission = seats::begin_host_observation(&context, &mut conn, "i", &budget).unwrap();
    let stage = seats::begin_snapshot_stage(
        &context,
        &mut conn,
        SnapshotHeader::from_captured(admission, &snapshot).unwrap(),
        &budget,
    )
    .unwrap();
    let stepping = StoreContext::new(
        path.clone(),
        Arc::new(SteppingClock(std::sync::atomic::AtomicU64::new(0))),
    );
    let mut offset = 0usize;
    while offset < snapshot.targets.len() {
        let progress = seats::stage_snapshot_targets(
            &stepping,
            &mut conn,
            &stage.id,
            offset as u64,
            &snapshot.targets[offset..],
            DurableWorkAdmission::new(16).unwrap(),
            &budget,
        )
        .unwrap();
        assert_eq!(progress.visited, 1, "one unit per spent quantum");
        offset += 1;
        assert_eq!(progress.stage.staged_targets, offset as u64);
    }
    seats::seal_snapshot_stage(&context, &mut conn, &stage.id, &budget).unwrap();
    let published = seats::publish_snapshot_stage(&context, &mut conn, &stage.id, &budget).unwrap();
    assert_eq!(published.target_count, 3);
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn thread_names_create_rename_clear_and_replay_keep_topic_and_authority() {
    let (context, mut conn, path, _) = bound_fixture(100);
    let command: CreateThread = serde_json::from_value(serde_json::json!({
        "topic":"topic", "goal":"goal", "name":"team café", "operation":"names-create", "claim":claim("s1")
    })).expect("named create must decode");
    let budget = crate::protocol::time::CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Default::default(),
    };
    let hash = cooperative_payload_hash("create_thread", &command).unwrap();
    let result = create_thread(
        &context,
        &mut conn,
        &budget,
        &command,
        permit(
            "s1",
            "names-create",
            ObligationRef::CheckIn(SeatId::new("s1")),
            hash,
            100,
        ),
    )
    .unwrap();
    let CommandResult::ThreadCreated(thread) = result else {
        panic!("create")
    };
    let stored: Option<String> = conn
        .query_row(
            "SELECT name FROM threads WHERE id=?1",
            [thread.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stored.as_deref(), Some("team café"));
    // All name writes exercise the same joined-caller rule as topic changes.
    for (operation, name) in [("names-rename", Some("new name")), ("names-clear", None)] {
        let wire: crate::protocol::commands::Command = serde_json::from_value(serde_json::json!({
            "kind":"set_thread_name", "args":{"thread":thread,"name":name,"operation":operation,"claim":claim("s1")}
        })).unwrap();
        let crate::protocol::commands::Command::SetThreadName(command) = wire else {
            panic!("name")
        };
        let hash = cooperative_payload_hash("set_thread_name", &command).unwrap();
        for _ in 0..2 {
            let result = set_thread_name(
                &context,
                &mut conn,
                &budget,
                &command,
                permit(
                    "s1",
                    operation,
                    ObligationRef::Control(thread.clone()),
                    hash,
                    100,
                ),
            )
            .unwrap();
            assert_eq!(result, CommandResult::ThreadNameChanged(thread.clone()));
        }
        let stored: Option<String> = conn
            .query_row(
                "SELECT name FROM threads WHERE id=?1",
                [thread.as_str()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(stored.as_deref(), name);
        let mut foreign = command.clone();
        foreign.claim = claim("s2");
        foreign.operation = OperationId::new(format!("foreign-{operation}"));
        let hash = cooperative_payload_hash("set_thread_name", &foreign).unwrap();
        assert_eq!(
            set_thread_name(
                &context,
                &mut conn,
                &budget,
                &foreign,
                permit(
                    "s2",
                    foreign.operation.as_str(),
                    ObligationRef::Control(thread.clone()),
                    hash,
                    100
                )
            )
            .unwrap_err()
            .code,
            crate::protocol::results::ErrorCode::Unauthorized
        );
    }
    assert_eq!(
        conn.query_row(
            "SELECT topic || '/' || goal FROM threads WHERE id=?1",
            [thread.as_str()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "topic/goal"
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn thread_names_store_validates_before_writes_and_allows_duplicate_create() {
    let (context, mut conn, path, _) = bound_fixture(100);
    let budget = crate::protocol::time::CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Default::default(),
    };
    for invalid in [
        "".to_owned(),
        "x".repeat(129),
        "é".repeat(65),
        "bad\nname".into(),
        "bad\u{7f}".into(),
    ] {
        let invalid_create = CreateThread {
            name: Some(invalid.clone()),
            topic: "topic".into(),
            goal: "goal".into(),
            operation: OperationId::new("invalid-create-name"),
            claim: claim("s1"),
        };
        let hash = cooperative_payload_hash("create_thread", &invalid_create).unwrap();
        assert_eq!(
            create_thread(
                &context,
                &mut conn,
                &budget,
                &invalid_create,
                permit(
                    "s1",
                    "invalid-create-name",
                    ObligationRef::CheckIn(SeatId::new("s1")),
                    hash,
                    100
                )
            )
            .unwrap_err()
            .code,
            crate::protocol::results::ErrorCode::InvalidRequest
        );
        let invalid_change = crate::protocol::commands::SetThreadName {
            thread: ThreadId::new("missing"),
            name: Some(invalid),
            operation: OperationId::new("invalid-set-name"),
            claim: claim("s1"),
        };
        let hash = cooperative_payload_hash("set_thread_name", &invalid_change).unwrap();
        assert_eq!(
            set_thread_name(
                &context,
                &mut conn,
                &budget,
                &invalid_change,
                permit(
                    "s1",
                    "invalid-set-name",
                    ObligationRef::Control(ThreadId::new("missing")),
                    hash,
                    100
                )
            )
            .unwrap_err()
            .code,
            crate::protocol::results::ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        conn.query_row("SELECT count(*) FROM threads", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    for operation in ["duplicate-one", "duplicate-two"] {
        let command = CreateThread {
            name: Some("é".repeat(64)),
            topic: "topic".into(),
            goal: "goal".into(),
            operation: OperationId::new(operation),
            claim: claim("s1"),
        };
        let hash = cooperative_payload_hash("create_thread", &command).unwrap();
        create_thread(
            &context,
            &mut conn,
            &budget,
            &command,
            permit(
                "s1",
                operation,
                ObligationRef::CheckIn(SeatId::new("s1")),
                hash,
                100,
            ),
        )
        .unwrap();
    }
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM threads WHERE name=?1",
            ["é".repeat(64)],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

fn rejection_offer() -> (
    StoreContext,
    Connection,
    PathBuf,
    Arc<FixedClock>,
    ThreadId,
    InvitationId,
) {
    rejection_offer_with_recipient_harness(Harness::Codex)
}
fn rejection_offer_with_recipient_harness(
    recipient: Harness,
) -> (
    StoreContext,
    Connection,
    PathBuf,
    Arc<FixedClock>,
    ThreadId,
    InvitationId,
) {
    let (context, mut conn, path, clock) = bound_fixture_recipient_harness(100, recipient);
    let budget = crate::protocol::time::CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Default::default(),
    };
    let create = CreateThread {
        name: None,
        topic: "review".into(),
        goal: "review only".into(),
        operation: OperationId::new("create"),
        claim: claim("s1"),
    };
    let hash = cooperative_payload_hash("create_thread", &create).unwrap();
    let CommandResult::ThreadCreated(thread) = create_thread(
        &context,
        &mut conn,
        &budget,
        &create,
        permit(
            "s1",
            "create",
            ObligationRef::CheckIn(SeatId::new("s1")),
            hash,
            100,
        ),
    )
    .unwrap() else {
        panic!()
    };
    let offer = Invite {
        thread: thread.clone(),
        seat: SeatId::new("s2"),
        deadline_millis: Some(10),
        operation: OperationId::new("invite"),
        claim: claim("s1"),
    };
    let hash = cooperative_payload_hash("invite", &offer).unwrap();
    let CommandResult::Invitation(invitation) = invite(
        &context,
        &mut conn,
        &budget,
        &offer,
        permit(
            "s1",
            "invite",
            ObligationRef::Control(thread.clone()),
            hash,
            100,
        ),
        None,
    )
    .unwrap() else {
        panic!()
    };
    (context, conn, path, clock, thread, invitation)
}

fn rejection_call(
    context: &StoreContext,
    conn: &mut Connection,
    command: &crate::protocol::commands::Reject,
    at: i64,
) -> Result<CommandResult, ApiError> {
    let hash = cooperative_payload_hash("reject", command).unwrap();
    reject(
        context,
        conn,
        &crate::protocol::time::CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: Default::default(),
        },
        command,
        MutationPermit::cooperative(
            command.claim.clone(),
            command.operation.clone(),
            ObligationRef::Invitation(command.invitation.clone()),
            hash,
            MonoInstant(at as u64),
            (1, 0),
            crate::protocol::time::CallBudget {
                deadline: MonoInstant(u64::MAX),
                cancellation: Default::default(),
            },
        ),
    )
}

#[test]
fn rejection_retains_reason_provenance_and_settles_only_invitation() {
    let (context, mut conn, path, clock, thread, invitation) = rejection_offer();
    // Deadlines classify lateness; they do not expire an invitation.
    clock.0.store(200, Ordering::SeqCst);
    let command = crate::protocol::commands::Reject {
        thread: thread.clone(),
        invitation: invitation.clone(),
        reason: "Outside my review role\nPeer data: <script>".into(),
        operation: OperationId::new("reject"),
        claim: claim("s2"),
    };
    let CommandResult::Rejected(record) =
        rejection_call(&context, &mut conn, &command, 200).unwrap()
    else {
        panic!()
    };
    assert_eq!(record.reason, command.reason);
    assert_eq!(record.actor.as_str(), "s2");
    assert_eq!(record.generation, 1);
    assert!(record.observation.contains("cooperative_top_level"));
    assert_eq!(record.rejected_at, UtcMillis(200));
    assert_eq!(
        rejection_call(&context, &mut conn, &command, 200).unwrap(),
        CommandResult::Rejected(record.clone())
    );
    let mut retry = command.clone();
    retry.operation = OperationId::new("retry");
    assert_eq!(
        rejection_call(&context, &mut conn, &retry, 200).unwrap(),
        CommandResult::Rejected(record)
    );
    retry.operation = OperationId::new("changed");
    retry.reason = "changed reason".into();
    assert_eq!(
        rejection_call(&context, &mut conn, &retry, 200)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    let pending: i64 = conn
        .query_row(
            "SELECT count(*) FROM digest_pending_invitations WHERE invitation_id=?1",
            [invitation.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(pending, 0);
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM membership_intervals WHERE seat_id='s2'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let event: String = conn
        .query_row(
            "SELECT event_json FROM messages WHERE event_key=?1",
            [format!("reject:{}", invitation.as_str())],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&event).unwrap()["reason"],
        command.reason
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

// Catches rejection being retained only as an info audit, dropped during
// attribution, revived after an offer, or duplicated by either replay path.
#[test]
fn invitation_rejection_delivers_reason_once_as_a_thread_warning() {
    let (context, mut conn, path, clock, thread, invitation) = rejection_offer();
    let command = crate::protocol::commands::Reject {
        thread: thread.clone(),
        invitation: invitation.clone(),
        reason: "Outside this seat's mail-only remit".into(),
        operation: OperationId::new("reject-warning"),
        claim: claim("s2"),
    };
    let original = rejection_call(&context, &mut conn, &command, 100).unwrap();
    let (id, kind, actor, event): (String, String, String, String) = conn
        .query_row(
            "SELECT id,kind,actor_seat_id,event_json FROM messages WHERE event_key=?1",
            [format!("reject:{}", invitation.as_str())],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(kind, "warn");
    assert_eq!(actor, "s2");
    let event: serde_json::Value = serde_json::from_str(&event).unwrap();
    assert_eq!(event["reason"], command.reason);
    assert_eq!(event["invitation"], invitation.as_str());
    assert_eq!(event["seat"], "s2");
    let pending = |db: &Connection| {
        crate::store::attention::seat_pending_warnings(db, "s1", &|| Ok(()))
            .unwrap()
            .items
            .into_iter()
            .map(|item| item.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        pending(&conn),
        vec![id.clone()],
        "visible before projection"
    );
    assert!(!crate::store::attention::warning_wakes_seat(&conn, "s1", &id).unwrap());
    for seat in ["s1", "s2"] {
        let wake = crate::store::attention::wake_seat_attention(&conn, seat).unwrap();
        assert!(!wake.attention.has_pending_invitation);
        assert!(!wake.attention.has_pending_receipt);
        assert_eq!(wake.attention.latest_warning_seq, None);
    }
    let budget = crate::protocol::time::CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Default::default(),
    };
    let CommandResult::InboxBatch(batch) = crate::store::queries::query(
        &context,
        "i",
        &crate::protocol::commands::Command::InboxBatch(crate::protocol::commands::InboxQuery {
            seat: Some(SeatId::new("s1")),
            page: Default::default(),
        }),
        &budget,
    )
    .unwrap() else {
        panic!()
    };
    assert!(
        batch.items.iter().any(|item| matches!(item,
            crate::protocol::results::InboxBatchItem::Warning { warning, .. }
            if warning.as_str() == id
        )),
        "reason must reach the thread member's inbox: {batch:?}"
    );

    let CommandResult::Message(detail) = crate::store::queries::query(
        &context,
        "i",
        &crate::protocol::commands::Command::Message(crate::protocol::commands::MessageQuery {
            message: MessageId::new(&id),
            body: crate::protocol::commands::BodyReadRequest {
                cursor: None,
                offset: None,
                max_bytes: 4096,
            },
        }),
        &budget,
    )
    .unwrap() else {
        panic!()
    };
    assert_eq!(
        detail.summary.event_author,
        Some(crate::protocol::service::EventAuthor::Native(SeatId::new(
            "s2"
        )))
    );
    assert!(matches!(detail.content,
        crate::protocol::results::MessageContent::System { event, .. }
        if event.kind == crate::protocol::results::MessageKind::Warn
            && event.event_json["reason"] == command.reason
            && event.source_invitation.as_ref() == Some(&invitation)
    ));
    let job: String = conn
        .query_row(
            "SELECT id FROM work_jobs WHERE kind='warning_attribution' AND subject_id=?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    let budget = crate::protocol::time::CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Default::default(),
    };
    while crate::store::materialization::advance_work(
        &mut conn,
        &job,
        crate::ports::DurableWorkAdmission::new(1).unwrap(),
        &budget,
        clock.as_ref(),
    )
    .unwrap()
    .has_more
    {}
    assert_eq!(pending(&conn), vec![id.clone()], "visible after projection");
    for seat in ["s1", "s2"] {
        assert_eq!(
            crate::store::attention::wake_seat_attention(&conn, seat)
                .unwrap()
                .attention
                .latest_warning_seq,
            None
        );
    }
    let tx = conn.transaction().unwrap();
    let page = crate::store::attention::notice_offer_page(&tx, "s1", 16).unwrap();
    assert_eq!(page.len(), 1);
    let execution: String = tx
        .query_row(
            "SELECT execution_id FROM occupant_bindings WHERE seat_id='s1' AND ended_at IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    crate::store::attention::settle_offered_notices(&tx, "s1", 1, &execution, &page).unwrap();
    tx.commit().unwrap();
    assert!(
        pending(&conn).is_empty(),
        "offered warning must stay settled"
    );
    assert_eq!(
        rejection_call(&context, &mut conn, &command, 100).unwrap(),
        original
    );
    let mut replay = command.clone();
    replay.operation = OperationId::new("reject-warning-again");
    assert_eq!(
        rejection_call(&context, &mut conn, &replay, 100).unwrap(),
        original
    );
    assert!(pending(&conn).is_empty(), "replay must not redeliver");
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM messages WHERE event_key=?1",
            [format!("reject:{}", invitation.as_str())],
            |r| r.get::<_, i64>(0),
        )
        .unwrap(),
        1
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

// Catches validation moving after durable rejection/event publication, or
// selecting a caller's invitation without checking its exact thread.
#[test]
fn invitation_rejection_invalid_reason_or_wrong_thread_has_no_effects() {
    for (reason, wrong_thread, wrong_invitation) in [
        ("", false, false),
        (" \n\t", false, false),
        ("Outside my remit", true, false),
        ("Outside my remit", false, true),
    ] {
        let (context, mut conn, path, _, thread, invitation) = rejection_offer();
        let command = crate::protocol::commands::Reject {
            thread: if wrong_thread {
                ThreadId::new("unrelated")
            } else {
                thread
            },
            invitation: if wrong_invitation {
                InvitationId::new("wrong-invitation")
            } else {
                invitation.clone()
            },
            reason: reason.into(),
            operation: OperationId::new("invalid-reject"),
            claim: claim("s2"),
        };
        let before: i64 = conn
            .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            rejection_call(&context, &mut conn, &command, 100)
                .unwrap_err()
                .code,
            if wrong_thread || wrong_invitation {
                ErrorCode::Unauthorized
            } else {
                ErrorCode::InvalidRequest
            }
        );
        assert_eq!(
            conn.query_row("SELECT count(*) FROM messages", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            before
        );
        assert_eq!(
            conn.query_row("SELECT count(*) FROM invitation_rejections", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            conn.query_row(
                "SELECT count(*) FROM digest_pending_invitations WHERE invitation_id=?1",
                [invitation.as_str()],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        drop(conn);
        let _ = std::fs::remove_file(path);
    }
}

// Catches a failed projection advancing the fanout cursor, or a global offer
// watermark swallowing attribution that was not actually carried.
#[test]
fn invitation_rejection_projection_retry_and_late_offer_keep_exact_delivery() {
    let (context, mut conn, path, clock, thread, invitation) = rejection_offer();
    let command = crate::protocol::commands::Reject {
        thread: thread.clone(),
        invitation: invitation.clone(),
        reason: "Outside remit".into(),
        operation: OperationId::new("reject-delayed"),
        claim: claim("s2"),
    };
    rejection_call(&context, &mut conn, &command, 100).unwrap();
    let (id, seq): (String, i64) = conn
        .query_row(
            "SELECT id,decision_seq FROM messages WHERE event_key=?1",
            [format!("reject:{}", invitation.as_str())],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    conn.execute("INSERT INTO warning_offer(seat_id,binding_generation,execution_id,offered_through_seq) SELECT seat_id,generation,execution_id,?1 FROM occupant_bindings WHERE seat_id='s1'", [seq+100]).unwrap();
    conn.execute_batch("CREATE TEMP TRIGGER reject_projection_failure BEFORE INSERT ON digest_programmatic_warnings BEGIN SELECT RAISE(ABORT,'injected rejection projection failure'); END;").unwrap();
    let budget = crate::protocol::time::CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Default::default(),
    };
    let job = format!("work:{id}");
    let failed = crate::store::materialization::advance_work(
        &mut conn,
        &job,
        crate::ports::DurableWorkAdmission::new(1).unwrap(),
        &budget,
        clock.as_ref(),
    )
    .unwrap();
    assert!(failed.last_error.is_some());
    assert_eq!(failed.processed_this_turn, 0);
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM warning_recipients WHERE warning_id=?1",
            [&id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        conn.query_row("SELECT position FROM work_jobs WHERE id=?1", [&job], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap(),
        0
    );
    assert_eq!(
        crate::store::attention::informational_notice_pending(&conn, "s1", &id).unwrap(),
        Some(true)
    );
    conn.execute_batch("DROP TRIGGER reject_projection_failure;")
        .unwrap();
    while crate::store::materialization::advance_work(
        &mut conn,
        &job,
        crate::ports::DurableWorkAdmission::new(1).unwrap(),
        &budget,
        clock.as_ref(),
    )
    .unwrap()
    .has_more
    {}
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM digest_programmatic_warnings WHERE warning_id=?1",
            [&id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    assert_eq!(
        crate::store::attention::notice_offer_page(&conn, "s1", 1)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        crate::store::attention::informational_notice_pending(&conn, "s1", &id).unwrap(),
        Some(true)
    );
    assert!(!crate::store::attention::warning_wakes_seat(&conn, "s2", &id).unwrap());
    drop(conn);
    let _ = std::fs::remove_file(path);
}

// Catches worker-time membership replacing the rejection's frozen audience,
// or an offered prefix settling uncarried notices or a successor binding.
#[test]
fn invitation_rejection_frozen_members_and_binding_scoped_prefix() {
    let (context, mut conn, path, clock, thread, invitation) = rejection_offer();
    let command = crate::protocol::commands::Reject {
        thread: thread.clone(),
        invitation: invitation.clone(),
        reason: "Outside remit".into(),
        operation: OperationId::new("reject-frozen"),
        claim: claim("s2"),
    };
    rejection_call(&context, &mut conn, &command, 100).unwrap();
    let (id, seq): (String, i64) = conn
        .query_row(
            "SELECT id,decision_seq FROM messages WHERE event_key=?1",
            [format!("reject:{}", invitation.as_str())],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    conn.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('later','i','resolved','native','later',1,1,100)", []).unwrap();
    conn.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES (?1,'later',1,?2)", params![thread.as_str(),seq+1]).unwrap();
    conn.execute(
        "UPDATE membership_intervals SET left_seq=?1 WHERE thread_id=?2 AND seat_id='s1'",
        params![seq + 1, thread.as_str()],
    )
    .unwrap();
    assert!(!crate::store::effective::is_warning_recipient(&conn, &id, "later").unwrap());
    assert!(crate::store::effective::is_warning_recipient(&conn, &id, "s1").unwrap());
    let budget = crate::protocol::time::CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Default::default(),
    };
    while crate::store::materialization::advance_work(
        &mut conn,
        &format!("work:{id}"),
        crate::ports::DurableWorkAdmission::new(1).unwrap(),
        &budget,
        clock.as_ref(),
    )
    .unwrap()
    .has_more
    {}
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM warning_recipients WHERE warning_id=?1 AND seat_id='later'",
            [&id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    // A second exact invitation can be independently refused, producing a new notice.
    conn.execute("INSERT INTO invitations(id,thread_id,seat_id,state,created_at,deadline_at,episode,created_decision_seq,frozen_duration_ms) SELECT 'second-refusal',thread_id,seat_id,'pending',created_at,deadline_at,episode+1,created_decision_seq,frozen_duration_ms FROM invitations WHERE id=?1", [invitation.as_str()]).unwrap();
    let mut second = command.clone();
    second.invitation = InvitationId::new("second-refusal");
    second.operation = OperationId::new("second-refusal");
    rejection_call(&context, &mut conn, &second, 100).unwrap();
    let second_id: String = conn
        .query_row(
            "SELECT id FROM messages WHERE event_key='reject:second-refusal'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    while crate::store::materialization::advance_work(
        &mut conn,
        &format!("work:{second_id}"),
        crate::ports::DurableWorkAdmission::new(1).unwrap(),
        &budget,
        clock.as_ref(),
    )
    .unwrap()
    .has_more
    {}
    let page = crate::store::attention::notice_offer_page(&conn, "s2", 1).unwrap();
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].notice.warning.as_str(), id);
    let execution: String = conn
        .query_row(
            "SELECT execution_id FROM occupant_bindings WHERE seat_id='s2' AND ended_at IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    crate::store::attention::settle_offered_notices(&conn, "s2", 1, &execution, &page).unwrap();
    assert_eq!(
        crate::store::attention::informational_notice_pending(&conn, "s2", &id).unwrap(),
        Some(false)
    );
    assert_eq!(
        crate::store::attention::informational_notice_pending(&conn, "s2", &second_id).unwrap(),
        Some(true)
    );
    // A stale predecessor frontier does not cover the current occupant.
    conn.execute(
        "UPDATE digest_notice_offer SET execution_id='predecessor' WHERE seat_id='s2'",
        [],
    )
    .unwrap();
    assert_eq!(
        crate::store::attention::notice_offer_page(&conn, "s2", 16)
            .unwrap()
            .len(),
        2
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

// Catches replay upgrading historical audit events into deliverable warnings.
#[test]
fn invitation_rejection_historical_info_replay_stays_unchanged() {
    let (context, mut conn, path, _, thread, invitation) = rejection_offer();
    let command = crate::protocol::commands::Reject {
        thread: thread.clone(),
        invitation: invitation.clone(),
        reason: "Historical refusal".into(),
        operation: OperationId::new("historical-replay"),
        claim: claim("s2"),
    };
    conn.execute("INSERT INTO invitation_rejections(invitation_id,reason,rejected_at,actor_seat_id,generation,observation) VALUES (?1,?2,100,'s2',1,'cooperative_top_level:old')",params![invitation.as_str(),command.reason]).unwrap();
    let tx = conn.transaction().unwrap();
    let payload=serde_json::json!({"action":"reject","seat":"s2","invitation":invitation.as_str(),"reason":command.reason}).to_string();
    let (id, _) = schema::append_attributed_event_once(
        &tx,
        schema::EventInput {
            thread: &thread,
            key: &format!("reject:{}", invitation.as_str()),
            kind: "info",
            payload_json: &payload,
            decision_at: UtcMillis(100),
            source_message: None,
            source_invitation: Some(&invitation),
        },
        crate::protocol::service::EventAuthor::Native(SeatId::new("s2")),
    )
    .unwrap();
    tx.commit().unwrap();
    let first = rejection_call(&context, &mut conn, &command, 100).unwrap();
    assert_eq!(
        rejection_call(&context, &mut conn, &command, 100).unwrap(),
        first
    );
    let mut fresh = command.clone();
    fresh.operation = OperationId::new("historical-fresh");
    assert_eq!(
        rejection_call(&context, &mut conn, &fresh, 100).unwrap(),
        first
    );
    assert_eq!(
        conn.query_row(
            "SELECT kind FROM messages WHERE id=?1",
            [id.as_str()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "info"
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM warning_jobs WHERE warning_id=?1",
            [id.as_str()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert!(
        crate::store::attention::seat_pending_warnings(&conn, "s1", &|| Ok(()))
            .unwrap()
            .items
            .is_empty()
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

// Catches passive classification trusting peer JSON instead of the exact
// retained source, event key and honest native author.
#[test]
fn invitation_rejection_notice_classification_requires_canonical_event() {
    for variant in [
        "wrong-key",
        "missing-source",
        "wrong-actor",
        "built-in",
        "wrong-json",
        "missing-ledger",
    ] {
        let (_context, mut conn, path, _, thread, invitation) = rejection_offer();
        if variant != "missing-ledger" {
            conn.execute("INSERT INTO invitation_rejections(invitation_id,reason,rejected_at,actor_seat_id,generation,observation) VALUES (?1,'Outside remit',100,'s2',1,'cooperative_top_level:old')", [invitation.as_str()]).unwrap();
        }
        let tx = conn.transaction().unwrap();
        let payload=serde_json::json!({"action":"reject","seat":"s2","invitation":invitation.as_str(),"reason":if variant=="wrong-json" {"unretained peer text"} else {"Outside remit"}}).to_string();
        let key = if variant == "wrong-key" {
            "other-event".into()
        } else {
            format!("reject:{}", invitation.as_str())
        };
        let author = if variant == "built-in" {
            crate::protocol::service::EventAuthor::BuiltIn
        } else {
            crate::protocol::service::EventAuthor::Native(SeatId::new(
                if variant == "wrong-actor" { "s1" } else { "s2" },
            ))
        };
        let (id, _) = schema::append_attributed_event_once(
            &tx,
            schema::EventInput {
                thread: &thread,
                key: &key,
                kind: "warn",
                payload_json: &payload,
                decision_at: UtcMillis(100),
                source_message: None,
                source_invitation: if variant == "missing-source" {
                    None
                } else {
                    Some(&invitation)
                },
            },
            author,
        )
        .unwrap();
        tx.commit().unwrap();
        assert!(
            !crate::store::effective::is_invitation_rejection_notice(&conn, id.as_str()).unwrap(),
            "{variant}"
        );
        assert_eq!(
            crate::store::attention::informational_notice_pending(&conn, "s1", id.as_str())
                .unwrap(),
            None,
            "{variant}"
        );
        assert!(
            crate::store::attention::warning_wakes_seat(&conn, "s1", id.as_str()).unwrap(),
            "{variant}"
        );
        drop(conn);
        let _ = std::fs::remove_file(path);
    }
}

fn rejection_accept(
    context: &StoreContext,
    conn: &mut Connection,
    thread: &ThreadId,
    invitation: &InvitationId,
) -> Result<CommandResult, ApiError> {
    let command = Accept {
        thread: thread.clone(),
        operation: OperationId::new("accept"),
        claim: claim("s2"),
    };
    let hash = cooperative_payload_hash("accept", &command).unwrap();
    accept(
        context,
        conn,
        &crate::protocol::time::CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: Default::default(),
        },
        &command,
        permit(
            "s2",
            "accept",
            ObligationRef::Invitation(invitation.clone()),
            hash,
            100,
        ),
    )
}

#[test]
fn invitation_rejection_accept_races_have_one_winner_and_reinvite_is_fresh() {
    for accept_first in [true, false] {
        let (context, mut conn, path, _, thread, invitation) = rejection_offer();
        let command = crate::protocol::commands::Reject {
            thread: thread.clone(),
            invitation: invitation.clone(),
            reason: "Wrong topic".into(),
            operation: OperationId::new("reject"),
            claim: claim("s2"),
        };
        if accept_first {
            rejection_accept(&context, &mut conn, &thread, &invitation).unwrap();
            assert_eq!(
                rejection_call(&context, &mut conn, &command, 100)
                    .unwrap_err()
                    .code,
                ErrorCode::Conflict
            );
            assert_eq!(
                conn.query_row("SELECT count(*) FROM invitation_rejections", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                0
            );
        } else {
            let original = rejection_call(&context, &mut conn, &command, 100).unwrap();
            assert_eq!(
                rejection_accept(&context, &mut conn, &thread, &invitation)
                    .unwrap_err()
                    .code,
                ErrorCode::Conflict
            );
            let offer = Invite {
                thread: thread.clone(),
                seat: SeatId::new("s2"),
                deadline_millis: Some(50),
                operation: OperationId::new("reinvite"),
                claim: claim("s1"),
            };
            let hash = cooperative_payload_hash("invite", &offer).unwrap();
            let CommandResult::Invitation(fresh) = invite(
                &context,
                &mut conn,
                &crate::protocol::time::CallBudget {
                    deadline: MonoInstant(u64::MAX),
                    cancellation: Default::default(),
                },
                &offer,
                permit(
                    "s1",
                    "reinvite",
                    ObligationRef::Control(thread.clone()),
                    hash,
                    100,
                ),
                None,
            )
            .unwrap() else {
                panic!()
            };
            assert_ne!(fresh, invitation);
            // Delayed replay may report the original decision, but never settles the fresh episode.
            assert_eq!(
                rejection_call(&context, &mut conn, &command, 100).unwrap(),
                original
            );
            let mut delayed = command.clone();
            delayed.operation = OperationId::new("delayed");
            assert_eq!(
                rejection_call(&context, &mut conn, &delayed, 100).unwrap(),
                original
            );
            assert_eq!(
                conn.query_row(
                    "SELECT count(*) FROM digest_pending_invitations WHERE invitation_id=?1",
                    [fresh.as_str()],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                1
            );
            rejection_accept(&context, &mut conn, &thread, &fresh).unwrap();
        }
        drop(conn);
        let _ = std::fs::remove_file(path);
    }
}

fn rejection_required_upgrade(
    conn: &mut Connection,
    thread: &ThreadId,
    invitation: &InvitationId,
    required_only: bool,
) {
    conn.execute(
        "INSERT INTO service_authors(id,instance_id,created_at) VALUES ('owner','i',0)",
        [],
    )
    .unwrap();
    conn.execute(
        "UPDATE threads SET managed_owner_author_id='owner' WHERE id=?1",
        [thread.as_str()],
    )
    .unwrap();
    conn.execute("INSERT INTO requirement_episodes(id,thread_id,seat_id,issuer_author_id,invitation_id,state,created_decision_seq,created_at) SELECT 'requirement',thread_id,seat_id,'owner',id,'pending',created_decision_seq+?2,created_at FROM invitations WHERE id=?1", rusqlite::params![invitation.as_str(),i64::from(!required_only)]).unwrap();
}

#[test]
fn invitation_rejection_refuses_required_upgrade_and_cancelled_or_retired_episode() {
    for state in ["required", "required-accepted", "cancelled", "retired"] {
        let (context, mut conn, path, _, thread, invitation) = rejection_offer();
        let command = crate::protocol::commands::Reject {
            thread: thread.clone(),
            invitation: invitation.clone(),
            reason: "Outside scope".into(),
            operation: OperationId::new("reject"),
            claim: claim("s2"),
        };
        match state {
            "required" => rejection_required_upgrade(&mut conn, &thread, &invitation, false),
            "required-accepted" => {
                rejection_required_upgrade(&mut conn, &thread, &invitation, false);
                conn.execute(
                    "UPDATE invitations SET state='accepted',accepted_at=100,accepted_actor_seat_id='s2',accepted_generation=1,accepted_observation='cooperative_top_level:test' WHERE id=?1",
                    [invitation.as_str()],
                )
                .unwrap();
                conn.execute("UPDATE requirement_episodes SET state='accepted',accepted_at=100,accepted_by_seat_id='s2',accepted_generation=1,accepted_observation='cooperative_top_level:test',revision=revision+1",[]).unwrap();
            }
            "cancelled" => {
                rejection_required_upgrade(&mut conn, &thread, &invitation, true);
                conn.execute("UPDATE requirement_episodes SET state='released',revision=revision+1,released_at=100 WHERE id='requirement'", []).unwrap();
                conn.execute("INSERT INTO invitation_cancellations(invitation_id,requirement_id,cancelled_at) VALUES (?1,'requirement',100)", [invitation.as_str()]).unwrap();
            }
            "retired" => {
                conn.execute(
                    "UPDATE invitations SET state='recipient_retired',retired_at=100 WHERE id=?1",
                    [invitation.as_str()],
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
        let error = rejection_call(&context, &mut conn, &command, 100).unwrap_err();
        if state.starts_with("required") {
            assert_eq!(error.code, ErrorCode::MembershipRequired);
            assert!(
                error.detail.contains("owner")
                    && error.detail.contains("release requirement requirement")
            );
            assert_eq!(
                conn.query_row("SELECT state FROM requirement_episodes", [], |r| r
                    .get::<_, String>(0))
                    .unwrap(),
                if state == "required" {
                    "pending"
                } else {
                    "accepted"
                }
            );
        } else {
            assert_eq!(error.code, ErrorCode::Conflict);
        }
        assert_eq!(
            conn.query_row("SELECT count(*) FROM invitation_rejections", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        drop(conn);
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn invitation_rejection_leaves_message_receipts_and_deadlines_unchanged() {
    let (context, mut conn, path, _, thread, invitation) = rejection_offer();
    conn.execute("INSERT INTO messages(instance_id,id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('i','owed',?1,100,'ordinary','explicitly addressed',0,100)", [thread.as_str()]).unwrap();
    conn.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) VALUES ('owed',?1,'s2','pending',300000,100,300100)", [thread.as_str()]).unwrap();
    let command = crate::protocol::commands::Reject {
        thread: thread.clone(),
        invitation,
        reason: "Outside role".into(),
        operation: OperationId::new("reject"),
        claim: claim("s2"),
    };
    rejection_call(&context, &mut conn, &command, 100).unwrap();
    let receipt: (String,i64,Option<i64>,Option<String>) = conn.query_row("SELECT state,deadline_at,acked_at,ack_observation FROM receipts WHERE message_id='owed'", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(receipt, ("pending".into(), 300100, None, None));
    assert_eq!(
        conn.query_row("SELECT count(*) FROM human_receipt_waivers", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn invitation_rejection_history_does_not_expand_due_scan_work() {
    let (context, mut conn, path, _, thread, invitation) = rejection_offer();
    // Fill retained terminal history without involving clocks or external processes.
    conn.execute("WITH RECURSIVE n(v) AS (SELECT 2 UNION ALL SELECT v+1 FROM n WHERE v<2000) INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,frozen_duration_ms,deadline_at) SELECT 'history-'||v,?1,'s2',v,'pending',100,v,10,110 FROM n", [thread.as_str()]).unwrap();
    conn.execute("INSERT INTO invitation_rejections(invitation_id,reason,rejected_at,actor_seat_id,generation,observation) SELECT id,'outside role',100,'s2',1,'cooperative_top_level:test' FROM invitations WHERE thread_id=?1",[thread.as_str()]).unwrap();
    let mut statement = conn
        .prepare(crate::store::invitation_due::INVITATION_DUE_SELECTION)
        .unwrap();
    let rows = statement
        .query_map(rusqlite::params![i64::MIN, 0, 1], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(rows.is_empty());
    let steps = statement.get_status(rusqlite::StatementStatus::VmStep);
    assert!(
        steps < 1000,
        "terminal invitation history expanded due work: {steps} instructions"
    );
    drop(statement);
    let selected =
        crate::store::invitation_due::select_invitation_due_candidates(&conn, None, 1).unwrap();
    let batch = crate::store::invitation_due::apply_invitation_due_candidates(
        &context, &mut conn, selected,
    )
    .unwrap();
    assert_eq!(batch.inspected, 0);
    assert_eq!(batch.warnings_added, 0);
    assert!(
        conn.execute(
            "UPDATE invitation_rejections SET reason='rewritten' WHERE invitation_id=?1",
            [invitation.as_str()]
        )
        .is_err()
    );
    assert!(
        conn.execute(
            "DELETE FROM invitation_rejections WHERE invitation_id=?1",
            [invitation.as_str()]
        )
        .is_err()
    );
    assert!(
        conn.execute(
            "UPDATE invitations SET reject_recorded=0 WHERE id=?1",
            [invitation.as_str()]
        )
        .is_err()
    );
    assert_eq!(conn.query_row("SELECT count(*) FROM invitations WHERE reject_recorded<>EXISTS(SELECT 1 FROM invitation_rejections r WHERE r.invitation_id=invitations.id)",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[test]
fn invitation_rejection_requires_recipient_current_top_level_binding() {
    for invalid in ["subagent", "generation", "recipient", "target"] {
        let (context, mut conn, path, _, thread, invitation) = rejection_offer();
        let mut command = crate::protocol::commands::Reject {
            thread,
            invitation,
            reason: "Outside role".into(),
            operation: OperationId::new("reject"),
            claim: claim("s2"),
        };
        match invalid {
            "subagent" => command.claim.role = crate::protocol::authority::CallerRole::Subagent,
            "generation" => command.claim.binding_generation += 1,
            "recipient" => command.claim = claim("s1"),
            "target" => command.claim.target = HostTargetId::new("unbound"),
            _ => unreachable!(),
        }
        assert!(
            rejection_call(&context, &mut conn, &command, 100).is_err(),
            "accepted {invalid}"
        );
        assert_eq!(
            conn.query_row("SELECT count(*) FROM invitation_rejections", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        drop(conn);
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn invitation_rejection_restores_left_only_from_recorded_leave_evidence() {
    for retain_evidence in [true, false] {
        let (context, mut conn, path, _, thread, invitation) = rejection_offer();
        rejection_accept(&context, &mut conn, &thread, &invitation).unwrap();
        let budget = crate::protocol::time::CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: Default::default(),
        };
        let command = Leave {
            thread: thread.clone(),
            operation: OperationId::new("leave"),
            claim: claim("s2"),
        };
        let hash = cooperative_payload_hash("leave", &command).unwrap();
        leave(
            &context,
            &mut conn,
            &budget,
            &command,
            permit(
                "s2",
                "leave",
                ObligationRef::Control(thread.clone()),
                hash,
                100,
            ),
        )
        .unwrap();
        if !retain_evidence {
            // A legacy closed interval without its matching leave event is insufficient.
            conn.execute("UPDATE membership_intervals SET left_seq=left_seq+1000 WHERE thread_id=?1 AND seat_id='s2'",[thread.as_str()]).unwrap();
        }
        let offer = Invite {
            thread: thread.clone(),
            seat: SeatId::new("s2"),
            deadline_millis: Some(50),
            operation: OperationId::new("reinvite"),
            claim: claim("s1"),
        };
        let hash = cooperative_payload_hash("invite", &offer).unwrap();
        let CommandResult::Invitation(fresh) = invite(
            &context,
            &mut conn,
            &budget,
            &offer,
            permit(
                "s1",
                "reinvite",
                ObligationRef::Control(thread.clone()),
                hash,
                100,
            ),
            None,
        )
        .unwrap() else {
            panic!()
        };
        let before: i64 = conn
            .query_row("SELECT count(*) FROM membership_intervals", [], |r| {
                r.get(0)
            })
            .unwrap();
        let rejection = crate::protocol::commands::Reject {
            thread: thread.clone(),
            invitation: fresh,
            reason: "Outside current role".into(),
            operation: OperationId::new("reject"),
            claim: claim("s2"),
        };
        rejection_call(&context, &mut conn, &rejection, 100).unwrap();
        let state:(String,Option<i64>) = conn.query_row("SELECT voluntary_state,left_at FROM memberships WHERE thread_id=?1 AND seat_id='s2'",[thread.as_str()],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(
            state,
            if retain_evidence {
                ("left".into(), Some(100))
            } else {
                ("absent".into(), None)
            }
        );
        assert_eq!(
            conn.query_row("SELECT count(*) FROM membership_intervals", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            before
        );
        drop(conn);
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn invitation_rejection_human_retains_declared_human_provenance() {
    let (context, mut conn, path, _, thread, invitation) =
        rejection_offer_with_recipient_harness(Harness::Human);
    let mut human = claim("s2");
    human.harness = Harness::Human;
    let command = crate::protocol::commands::Reject {
        thread,
        invitation,
        reason: "Outside my topic".into(),
        operation: OperationId::new("reject-human"),
        claim: human,
    };
    let CommandResult::Rejected(result) =
        rejection_call(&context, &mut conn, &command, 100).unwrap()
    else {
        panic!()
    };
    let observation: serde_json::Value = serde_json::from_str(&result.observation).unwrap();
    assert_eq!(observation["provenance"], "operator_human");
    assert_eq!(observation["harness"], "human");
    assert_eq!(result.actor, SeatId::new("s2"));
    drop(conn);
    let _ = std::fs::remove_file(path);
}

#[path = "topology_attachment_guard.rs"]
mod topology_attachment;

#[test]
fn directory_name_mutations_stale_filtered_cursors_but_replay_and_noop_do_not() {
    use crate::{
        ports::StorePort,
        protocol::{
            commands::{DirectoryMembership, DirectoryQuery, PermitMutation, SetThreadName},
            pagination::PageRequest,
        },
        store::{SqliteStore, StoreSettings},
    };
    for recent in [false, true] {
        for changed in ["first", "later"] {
            let (context, conn, path, clock) = bound_fixture(100);
            conn.execute_batch("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('first','i','needle','goal',0,0),('later','i','needle','goal',0,0);
                INSERT INTO memberships(thread_id,seat_id,state) VALUES ('first','s1','joined'),('later','s1','joined');").unwrap();
            conn.execute(
                "UPDATE threads SET topic='unrelated' WHERE id=?1",
                [changed],
            )
            .unwrap();
            conn.execute_batch("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('anchor-a','i','needle','goal',0,0),('anchor-b','i','needle','goal',0,0);").unwrap();
            let store = SqliteStore::new(
                StoreContext::new(path.clone(), clock),
                "i",
                StoreSettings::default(),
            )
            .unwrap();
            let budget = crate::protocol::time::CallBudget {
                deadline: MonoInstant(u64::MAX),
                cancellation: Default::default(),
            };
            let directory = |cursor, filtered: bool| {
                crate::protocol::commands::Command::Directory(DirectoryQuery {
                    recent,
                    membership: None,
                    membership_filter: DirectoryMembership::All,
                    topic_contains: filtered.then(|| "needle".into()),
                    page: PageRequest {
                        cursor,
                        limit: 1,
                        max_bytes: 65536,
                    },
                })
            };
            let first = |filtered| {
                let CommandResult::Directory(page) = super::super::queries::query(
                    &context,
                    "i",
                    &directory(None, filtered),
                    &budget,
                )
                .unwrap() else {
                    panic!()
                };
                page.next_cursor.expect("continuation")
            };
            let mutate = |operation: &str, name: Option<&str>| {
                let mutation = PermitMutation::SetThreadName(SetThreadName {
                    thread: ThreadId::new(changed),
                    name: name.map(str::to_owned),
                    operation: OperationId::new(operation),
                    claim: claim("s1"),
                });
                let permit = store
                    .issue_cooperative_permit(
                        crate::store::cooperative_permit_request(&mutation).unwrap(),
                        &budget,
                    )
                    .unwrap();
                store.mutate(mutation, permit, &budget).unwrap();
            };
            for (op, name, expected_match) in [
                ("set", Some("needle"), true),
                ("rename-out", Some("elsewhere"), false),
                ("rename-in", Some("needle again"), true),
                ("clear", None, false),
            ] {
                let cursor = first(true);
                let unfiltered = first(false);
                mutate(op, name);
                let error = super::super::queries::query(
                    &context,
                    "i",
                    &directory(Some(cursor), true),
                    &budget,
                )
                .unwrap_err();
                assert_eq!(
                    error.code,
                    crate::protocol::results::ErrorCode::CursorStale,
                    "{recent} {changed} {op}"
                );
                let argv = error.restart_argv.unwrap();
                assert!(argv.windows(2).any(|pair| pair == ["--search", "needle"]));
                if !recent {
                    assert!(
                        super::super::queries::query(
                            &context,
                            "i",
                            &directory(Some(unfiltered.clone()), false),
                            &budget
                        )
                        .is_ok()
                    );
                }
                if recent {
                    assert_eq!(
                        super::super::queries::query(
                            &context,
                            "i",
                            &directory(Some(unfiltered), false),
                            &budget
                        )
                        .unwrap_err()
                        .code,
                        crate::protocol::results::ErrorCode::CursorStale
                    );
                }
                let mut fresh = directory(None, true);
                if let crate::protocol::commands::Command::Directory(q) = &mut fresh {
                    q.page.limit = 10;
                }
                let CommandResult::Directory(matches) =
                    super::super::queries::query(&context, "i", &fresh, &budget).unwrap()
                else {
                    panic!()
                };
                assert_eq!(
                    matches
                        .items
                        .iter()
                        .any(|item| item.thread.as_str() == changed),
                    expected_match
                );
                let cursor = first(true);
                mutate(op, name); // exact historical replay
                assert!(
                    super::super::queries::query(
                        &context,
                        "i",
                        &directory(Some(cursor), true),
                        &budget
                    )
                    .is_ok()
                );
                let cursor = first(true);
                mutate(&format!("noop-{op}"), name);
                assert!(
                    super::super::queries::query(
                        &context,
                        "i",
                        &directory(Some(cursor), true),
                        &budget
                    )
                    .is_ok()
                );
            }
            assert_eq!(conn.query_row("SELECT revision FROM filter_revisions WHERE instance_id='i' AND scope_kind='directory' AND scope_key='name/all'",[],|r|r.get::<_,i64>(0)).unwrap(),4);
            if !recent {
                let cursor = first(true);
                let mutation = PermitMutation::Join(crate::protocol::commands::Join {
                    thread: ThreadId::new("first"),
                    operation: OperationId::new("unrelated-join"),
                    claim: claim("s2"),
                });
                let permit = store
                    .issue_cooperative_permit(
                        crate::store::cooperative_permit_request(&mutation).unwrap(),
                        &budget,
                    )
                    .unwrap();
                store.mutate(mutation, permit, &budget).unwrap();
                assert!(
                    super::super::queries::query(
                        &context,
                        "i",
                        &directory(Some(cursor), true),
                        &budget
                    )
                    .is_ok()
                );
            }
            let cursor = first(true);
            let mutation = PermitMutation::SetTopic(crate::protocol::commands::SetTopic {
                thread: ThreadId::new(changed),
                topic: "new topic".into(),
                operation: OperationId::new("topic-change"),
                claim: claim("s1"),
            });
            let permit = store
                .issue_cooperative_permit(
                    crate::store::cooperative_permit_request(&mutation).unwrap(),
                    &budget,
                )
                .unwrap();
            store.mutate(mutation, permit, &budget).unwrap();
            assert_eq!(
                super::super::queries::query(
                    &context,
                    "i",
                    &directory(Some(cursor), true),
                    &budget
                )
                .unwrap_err()
                .code,
                crate::protocol::results::ErrorCode::CursorStale
            );

            drop(store);
            drop(conn);
            drop(context);
            let _ = std::fs::remove_file(path);
        }
    }
}
