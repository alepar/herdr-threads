//! TRUST-POLICY C2 restored-pane retirement (ht-wpq4) against the real store:
//! the watch, the closure, the claim guards and the observed grace.

use super::*;
use crate::identity::reconcile::plan_page;
use crate::ports::{
    EnumerationEvidence, EvidenceKind, ExecutionEvidence, GuardedSeatTransition, HostObservation,
    HostSnapshot, HostUiState, IncarnationEvidence, ObservationProvenance, ReconciliationAction,
    ReconciliationOutcome, SnapshotHeader, StructuralOccupancy,
};
use crate::protocol::ids::HostCallId;
use crate::protocol::time::{Cancellation, Clock, MonoInstant};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

const INSTANCE: &str = "34750f98bd59fcfc946da45aaabe933b";

struct TestClock(AtomicI64);
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(self.0.load(Ordering::SeqCst))
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.load(Ordering::SeqCst) as u64)
    }
}

struct Fx {
    context: StoreContext,
    conn: Connection,
    clock: Arc<TestClock>,
    sequence: u64,
    epoch: u64,
    boot: &'static str,
    incarnation: &'static str,
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(u64::MAX / 2),
        cancellation: Cancellation::default(),
    }
}

/// Seat `s1` was resolved on `w1:p1` in an older incarnation and a new one
/// left it unresolved (reason `other`); its session was claude `SA`.
fn fixture() -> Fx {
    let path = crate::test_support::scratch::temp_path(format!(
        "herdr-restored-pane-{}.db",
        uuid::Uuid::new_v4()
    ));
    let clock = Arc::new(TestClock(AtomicI64::new(1_790_000_000_000)));
    let context = StoreContext::new(path, clock.clone()).with_test_host();
    let mut conn = context.open_writer().unwrap();
    crate::store::journal::open(
        &context,
        &mut conn,
        Arc::new(crate::node::NodeIdentity::from_seed(
            crate::test_support::TEST_NODE_SEED,
        )),
        "test",
        crate::sync::facts::FactRegistry::builtins(),
    )
    .unwrap();
    conn.execute("UPDATE herdr_hosts SET host_boot='b-old',host_epoch=1", [])
        .unwrap();
    conn.execute(&format!("INSERT INTO seats(id,instance_id,state,unresolved_reason,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) VALUES ('s1','{INSTANCE}','unresolved','other','native','w1:p1',2,1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))"), []).unwrap();
    conn.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,terminal_id,incarnation,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,ended_at,liveness,disconnected_at,disconnect_reason) VALUES ('s1',1,'w1:p1','term-old','inc-old','b-old',1,1,'claude','SA','execution-1','cooperative_top_level',100,200,'disconnected',200,'seat_unresolved')", []).unwrap();
    Fx {
        context,
        conn,
        clock,
        sequence: 0,
        boot: "b2",
        incarnation: "inc-2",
        epoch: 1,
    }
}

impl Fx {
    fn now(&self) -> i64 {
        self.clock.0.load(Ordering::SeqCst)
    }
    fn advance(&self, ms: i64) {
        self.clock.0.fetch_add(ms, Ordering::SeqCst);
    }
    /// A coherent publication of `panes` (target, terminal) in the current
    /// boot and incarnation.
    fn publish(&mut self, panes: &[(&str, &str)]) -> PublishedSnapshot {
        self.sequence += 1;
        let sequence = self.sequence;
        let boot = HostBootId::new(self.boot);
        let snapshot = HostSnapshot {
            boot: boot.clone(),
            epoch: self.epoch,
            observation_sequence: sequence,
            complete: true,
            enumeration: EnumerationEvidence::CoherentVerified,
            incarnation: IncarnationEvidence::Verified {
                identity: self.incarnation.into(),
                evidence_kind: EvidenceKind::CoherentEnumeration,
            },
            targets: panes
                .iter()
                .map(|(target, terminal)| HostObservation {
                    focused: false,
                    target: HostTargetId::new(*target),
                    host_boot: boot.clone(),
                    epoch: self.epoch,
                    generation: 1,
                    observed_at_utc: UtcMillis(self.now()),
                    observed_at_mono: MonoInstant(100),
                    provenance: ObservationProvenance::CoherentEnumeration,
                    occupant: None,
                    ui: HostUiState::Idle,
                    terminal: Some(TerminalId::new(*terminal)),
                    occupancy: StructuralOccupancy::Unknown,
                    incarnation: IncarnationEvidence::Unknown,
                    execution: ExecutionEvidence::Unknown,
                    call_id: HostCallId::new(format!("capture-{target}-{sequence}")),
                    connection_epoch: self.epoch,
                    observation_sequence: sequence,
                    started_at_mono: MonoInstant(100),
                    completed_at_mono: MonoInstant(100),
                })
                .collect(),
        };
        let budget = budget();
        let admission = begin_host_observation(
            &self.context,
            &mut self.conn,
            INSTANCE,
            crate::test_support::test_host_id(),
            &budget,
        )
        .unwrap();
        let header = SnapshotHeader::from_captured(admission, &snapshot).unwrap();
        let stage = begin_snapshot_stage(&self.context, &mut self.conn, header, &budget).unwrap();
        crate::test_support::snapshot::stage_all(&snapshot.targets, |offset, targets| {
            stage_snapshot_targets(
                &self.context,
                &mut self.conn,
                &stage.id,
                offset,
                targets,
                DurableWorkAdmission::new(16).unwrap(),
                &budget,
            )
        })
        .unwrap();
        seal_snapshot_stage(&self.context, &mut self.conn, &stage.id, &budget).unwrap();
        publish_snapshot_stage(&self.context, &mut self.conn, &stage.id, &budget).unwrap()
    }
    /// One reconciliation pass over `publication`; the seat's evidence as
    /// planned and every transition's outcome.
    fn pass(
        &mut self,
        publication: &PublishedSnapshot,
    ) -> (RestoredPane, Vec<ReconciliationOutcome>) {
        let budget = budget();
        let page = saved_seats_page(
            &self.context,
            &self.conn,
            &publication.id,
            0,
            None,
            16,
            &budget,
        )
        .unwrap();
        let evidence = page
            .seats
            .iter()
            .find(|seat| seat.seat.as_str() == "s1")
            .map_or(RestoredPane::None, |seat| seat.restored_pane);
        let outcomes = plan_page(&page)
            .unwrap()
            .into_iter()
            .map(|transition| {
                apply_reconciliation_transition(&self.context, &mut self.conn, transition, &budget)
                    .unwrap()
            })
            .collect();
        (evidence, outcomes)
    }
    fn publish_and_pass(&mut self, panes: &[(&str, &str)]) -> RestoredPane {
        let publication = self.publish(panes);
        self.pass(&publication).0
    }
    fn state(&self) -> String {
        self.conn
            .query_row("SELECT state FROM seats WHERE id='s1'", [], |r| r.get(0))
            .unwrap()
    }
    fn watch(&self) -> Option<(String, Option<i64>, i64)> {
        self.conn
            .query_row(
                "SELECT terminal_id,absent_seen_at,absent_observed_ms FROM restored_pane_watch WHERE seat_id='s1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .unwrap()
    }
}

const OPEN: &[(&str, &str)] = &[("w1:p1", "term-a"), ("w1:p2", "term-b")];
const CLOSED: &[(&str, &str)] = &[("w1:p2", "term-b")];
const STEP: i64 = ABSENCE_STEP_CAP_MS;

/// Watch, then the closure observed for exactly the grace: one pass short
/// keeps the seat, the next retires it with its evidence recorded.
#[test]
fn restored_pane_closed_for_the_grace_retires_the_seat() {
    let mut fx = fixture();
    assert_eq!(fx.publish_and_pass(OPEN), RestoredPane::Observe);
    assert_eq!(fx.watch().map(|w| w.0), Some("term-a".into()));
    assert_eq!(fx.publish_and_pass(OPEN), RestoredPane::Watching);
    // The first absent pass counts nothing; each later one at most a step.
    assert_eq!(fx.publish_and_pass(CLOSED), RestoredPane::Absent);
    let passes = RESTORED_PANE_GRACE_MS / STEP;
    for _ in 1..passes {
        fx.advance(STEP);
        assert_eq!(fx.publish_and_pass(CLOSED), RestoredPane::Absent);
    }
    assert_eq!(fx.state(), "unresolved");
    assert_eq!(fx.watch().unwrap().2, RESTORED_PANE_GRACE_MS - STEP);
    fx.advance(STEP);
    let publication = fx.publish(CLOSED);
    let (evidence, outcomes) = fx.pass(&publication);
    assert_eq!(evidence, RestoredPane::Closed);
    assert!(
        outcomes
            .iter()
            .any(|o| matches!(o, ReconciliationOutcome::RetirementStarted(_)))
    );
    assert_eq!(fx.state(), "retired");
    assert_eq!(fx.watch(), None, "a retirement deletes its watch");
    let evidence: Option<String> = fx
        .conn
        .query_row(
            "SELECT closure_evidence FROM retirements WHERE seat_id='s1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(evidence.as_deref(), Some("restored_pane_closed"));
}

/// A suspend, clock step or downtime between two passes counts one step.
#[test]
fn a_gap_between_passes_counts_at_most_one_step() {
    let mut fx = fixture();
    fx.publish_and_pass(OPEN);
    assert_eq!(fx.publish_and_pass(CLOSED), RestoredPane::Absent);
    fx.advance(2 * 60 * 60 * 1000);
    assert_eq!(fx.publish_and_pass(CLOSED), RestoredPane::Absent);
    assert_eq!(fx.watch().unwrap().2, STEP);
    // A clock stepped backwards counts nothing.
    fx.advance(-10 * 60 * 1000);
    assert_eq!(fx.publish_and_pass(CLOSED), RestoredPane::Absent);
    assert_eq!(fx.watch().unwrap().2, STEP);
    assert_eq!(fx.state(), "unresolved");
}

/// A kept claim for one of the seat's sessions keeps it watching however long
/// the pane stays closed.
#[test]
fn a_kept_claim_for_the_seats_session_holds_off_retirement() {
    let mut fx = fixture();
    fx.publish_and_pass(OPEN);
    let now = fx.now();
    fx.conn.execute(
        "INSERT INTO kept_continuity_claims(host_id,target_id,harness,native_session,command_json,received_at) VALUES ((SELECT min(host_id) FROM herdr_hosts),'w1:p7','claude','SA','{}',?1)",
        [now],
    ).unwrap();
    for _ in 0..=(RESTORED_PANE_GRACE_MS / STEP) + 1 {
        fx.advance(STEP);
        assert_eq!(fx.publish_and_pass(CLOSED), RestoredPane::Watching);
    }
    assert_eq!(fx.state(), "unresolved");
    // Another session's kept claim does not hold it off.
    fx.conn
        .execute(
            "UPDATE kept_continuity_claims SET native_session='OTHER'",
            [],
        )
        .unwrap();
    assert_eq!(fx.publish_and_pass(CLOSED), RestoredPane::Absent);
}

/// A pane another seat's proof or binding names is never watched; one moved
/// to another target keeps its terminal and is not a close.
#[test]
fn a_pane_claimed_by_another_seat_or_moved_is_not_a_close() {
    let mut fx = fixture();
    fx.conn.execute(&format!("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id,structural_terminal_id,structural_incarnation,structural_incarnation_kind,structural_host_boot,structural_host_epoch,structural_connection_epoch,structural_observation_sequence) VALUES ('s3','{INSTANCE}','resolved','native','w1:p9',1,1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts),'term-a','inc-2','coherent_enumeration','b2',1,1,1)"), []).unwrap();
    assert_eq!(fx.publish_and_pass(OPEN), RestoredPane::None);
    assert_eq!(fx.watch(), None);

    let mut fx = fixture();
    fx.publish_and_pass(OPEN);
    let moved = &[("w1:p9", "term-a"), ("w1:p2", "term-b")];
    for _ in 0..3 {
        fx.advance(STEP);
        assert_eq!(fx.publish_and_pass(moved), RestoredPane::Watching);
    }
}

/// A target released from the baseline (claimed) is never watched, and a
/// watch from another incarnation is void and replaced.
#[test]
fn released_targets_and_other_incarnations_start_no_closure() {
    let mut fx = fixture();
    let publication = fx.publish(OPEN);
    fx.conn.execute(
        "INSERT INTO recovery_baseline_releases(host_id,baseline_generation_id,target_id,decision_seq) VALUES ((SELECT min(host_id) FROM herdr_hosts),?1,'w1:p1',1)",
        [publication.id.as_str()],
    ).unwrap();
    assert_eq!(fx.pass(&publication).0, RestoredPane::None);

    let mut fx = fixture();
    fx.publish_and_pass(OPEN);
    assert_eq!(fx.publish_and_pass(CLOSED), RestoredPane::Absent);
    // A new Herdr incarnation restores the pane again under a new terminal.
    fx.boot = "b3";
    fx.incarnation = "inc-3";
    fx.sequence = 0;
    assert_eq!(
        fx.publish_and_pass(&[("w1:p1", "term-a3"), ("w1:p2", "term-b3")]),
        RestoredPane::Observe
    );
    assert_eq!(fx.watch(), Some(("term-a3".into(), None, 0)));
}

/// A target the recovery baseline did not hold (the pane appeared later) is
/// never watched, so its closure retires nothing.
#[test]
fn a_target_not_in_the_baseline_is_never_watched() {
    let mut fx = fixture();
    assert_eq!(fx.publish_and_pass(CLOSED), RestoredPane::None);
    assert_eq!(fx.publish_and_pass(OPEN), RestoredPane::None);
    assert_eq!(fx.watch(), None);
}

/// A daemon restart after the closure (a new recovery epoch, its baseline
/// retaken without the closed pane) keeps the watch and its observed time.
#[test]
fn the_watch_survives_a_new_recovery_epoch_after_the_closure() {
    let mut fx = fixture();
    fx.publish_and_pass(OPEN);
    assert_eq!(fx.publish_and_pass(CLOSED), RestoredPane::Absent);
    fx.advance(STEP);
    assert_eq!(fx.publish_and_pass(CLOSED), RestoredPane::Absent);
    let recorded: i64 = fx
        .conn
        .query_row(
            "SELECT recorded_at FROM restored_pane_watch WHERE seat_id='s1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    fx.epoch = 2;
    fx.advance(STEP);
    assert_eq!(fx.publish_and_pass(CLOSED), RestoredPane::Absent);
    assert_eq!(
        fx.watch().unwrap().2,
        2 * STEP,
        "observed time carries over"
    );
    let after: i64 = fx
        .conn
        .query_row(
            "SELECT recorded_at FROM restored_pane_watch WHERE seat_id='s1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(after, recorded, "the same watch, not a new one");
    while fx.state() == "unresolved" {
        fx.advance(STEP);
        fx.publish_and_pass(CLOSED);
    }
    assert_eq!(fx.state(), "retired");
}

/// The applier decides against the store as it is at commit (A2): a kept
/// claim, a generation bump or a different target after the page read
/// refuses the retirement.
#[test]
fn the_applier_refuses_a_retirement_whose_evidence_changed_after_the_page() {
    let mut fx = fixture();
    fx.publish_and_pass(OPEN);
    fx.publish_and_pass(CLOSED);
    for _ in 0..RESTORED_PANE_GRACE_MS / STEP - 1 {
        fx.advance(STEP);
        fx.publish_and_pass(CLOSED);
    }
    fx.advance(STEP);
    let publication = fx.publish(CLOSED);
    let page_budget = budget();
    let planned = || {
        let page = saved_seats_page(
            &fx.context,
            &fx.conn,
            &publication.id,
            0,
            None,
            16,
            &page_budget,
        )
        .unwrap();
        plan_page(&page)
            .unwrap()
            .into_iter()
            .find(|t| t.seat.as_str() == "s1")
            .unwrap()
    };
    let retire = planned();
    assert!(matches!(
        retire.action,
        ReconciliationAction::RetireRestoredPane { .. }
    ));
    let apply = |fx: &mut Fx, transition: GuardedSeatTransition| {
        apply_reconciliation_transition(&fx.context, &mut fx.conn, transition, &budget()).unwrap()
    };

    // A kept claim for the seat's session arrives between page and apply.
    let now = fx.now();
    fx.conn.execute(
        "INSERT INTO kept_continuity_claims(host_id,target_id,harness,native_session,command_json,received_at) VALUES ((SELECT min(host_id) FROM herdr_hosts),'w1:p7','claude','SA','{}',?1)",
        [now],
    ).unwrap();
    assert_eq!(apply(&mut fx, retire.clone()), ReconciliationOutcome::Stale);
    assert_eq!(fx.state(), "unresolved");
    // A kept claim past its TTL no longer holds it off.
    fx.conn
        .execute(
            "UPDATE kept_continuity_claims SET received_at=?1",
            [now - crate::store::kept_continuity::KEPT_CLAIM_TTL_MS - 1],
        )
        .unwrap();

    // Another target than the seat's.
    let mut elsewhere = retire.clone();
    elsewhere.action = ReconciliationAction::RetireRestoredPane {
        absent_target: HostTargetId::new("w1:p2"),
    };
    assert_eq!(apply(&mut fx, elsewhere), ReconciliationOutcome::Stale);
    assert_eq!(fx.state(), "unresolved");

    // The seat's generation moved (a rebind or unresolve meanwhile).
    fx.conn
        .execute("UPDATE seats SET generation=generation+1 WHERE id='s1'", [])
        .unwrap();
    assert_eq!(apply(&mut fx, retire.clone()), ReconciliationOutcome::Stale);
    assert_eq!(fx.state(), "unresolved");
    fx.conn
        .execute("UPDATE seats SET generation=generation-1 WHERE id='s1'", [])
        .unwrap();

    // Unchanged evidence retires.
    assert!(matches!(
        apply(&mut fx, retire),
        ReconciliationOutcome::RetirementStarted(_)
    ));
    assert_eq!(fx.state(), "retired");
}
