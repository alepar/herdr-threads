//! TRUST-POLICY C2 restored-pane retirement (ht-wpq4) against the real store:
//! the watch, the closure, the claim guards and the observed grace.

use super::*;
use crate::identity::reconcile::plan_page;
use crate::ports::{
    EnumerationEvidence, EvidenceKind, ExecutionEvidence, GuardedSeatTransition, HostObservation,
    HostSnapshot, HostUiState, IncarnationEvidence, ObservationProvenance,
    OrdinaryResolutionAttempt, OrdinaryResolutionGuard, OrdinaryResolutionOutcome,
    ReconciliationAction, ReconciliationOutcome, SnapshotHeader, StructuralOccupancy,
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
    /// Record the reconciliation marker after a pass with no refused
    /// transition, as the observation worker does.
    record_marker: bool,
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
        record_marker: true,
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
            .collect::<Vec<_>>();
        if self.record_marker
            && !outcomes
                .iter()
                .any(|outcome| matches!(outcome, ReconciliationOutcome::Stale(_)))
        {
            record_reconciliation_pass(&self.context, &mut self.conn, publication, &budget)
                .unwrap();
        }
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
    // The new recovery's first pass precedes its marker: nothing counts.
    assert_eq!(fx.publish_and_pass(CLOSED), RestoredPane::Watching);
    assert_eq!(fx.watch().unwrap().2, STEP);
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
    assert!(matches!(
        apply(&mut fx, retire.clone()),
        ReconciliationOutcome::Stale(_)
    ));
    assert_eq!(fx.state(), "unresolved");
    // Decided or pruned, the claim no longer holds it off.
    fx.conn
        .execute("DELETE FROM kept_continuity_claims", [])
        .unwrap();

    // Another target than the seat's.
    let mut elsewhere = retire.clone();
    elsewhere.action = ReconciliationAction::RetireRestoredPane {
        absent_target: HostTargetId::new("w1:p2"),
    };
    assert!(matches!(
        apply(&mut fx, elsewhere),
        ReconciliationOutcome::Stale(_)
    ));
    assert_eq!(fx.state(), "unresolved");

    // The seat's generation moved (a rebind or unresolve meanwhile).
    fx.conn
        .execute("UPDATE seats SET generation=generation+1 WHERE id='s1'", [])
        .unwrap();
    assert!(matches!(
        apply(&mut fx, retire.clone()),
        ReconciliationOutcome::Stale(_)
    ));
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

/// Review of ht-wpq4: while reconciliation lags, `kept_continuity::due` never
/// prunes, so an old kept row may be a live session's only claim. It blocks
/// whatever its age, and no closure time counts until the recovery's marker
/// is current again; then the grace runs from there.
#[test]
fn an_expired_kept_claim_blocks_and_a_lagging_marker_counts_nothing() {
    let mut fx = fixture();
    fx.publish_and_pass(OPEN);
    let expired = fx.now() - crate::store::kept_continuity::KEPT_CLAIM_TTL_MS - 1;
    fx.conn.execute(
        "INSERT INTO kept_continuity_claims(host_id,target_id,harness,native_session,command_json,received_at) VALUES ((SELECT min(host_id) FROM herdr_hosts),'w1:p7','claude','SA','{}',?1)",
        [expired],
    ).unwrap();
    for _ in 0..2 * RESTORED_PANE_GRACE_MS / STEP {
        fx.advance(STEP);
        assert_eq!(fx.publish_and_pass(CLOSED), RestoredPane::Watching);
    }
    assert_eq!(fx.state(), "unresolved");
    fx.conn
        .execute("DELETE FROM kept_continuity_claims", [])
        .unwrap();

    // Reconciliation stops completing: the marker no longer names the
    // current recovery.
    fx.record_marker = false;
    fx.conn
        .execute(
            "UPDATE herdr_hosts SET reconciled_boot=NULL,reconciled_epoch=NULL",
            [],
        )
        .unwrap();
    for _ in 0..2 * RESTORED_PANE_GRACE_MS / STEP {
        fx.advance(STEP);
        assert_eq!(fx.publish_and_pass(CLOSED), RestoredPane::Watching);
    }
    assert_eq!(fx.state(), "unresolved");
    assert_eq!(fx.watch().unwrap().2, 0, "no closure time counted");

    // Caught up again: the full grace is still owed.
    fx.record_marker = true;
    fx.publish_and_pass(CLOSED);
    let mut passes = 0;
    while fx.state() == "unresolved" {
        fx.advance(STEP);
        fx.publish_and_pass(CLOSED);
        passes += 1;
    }
    assert!(passes >= RESTORED_PANE_GRACE_MS / STEP, "{passes}");
}

/// Seat `s2` resolved on `w1:p2`, its structural proof in the current incarnation.
fn seed_live_seat(fx: &Fx) {
    fx.conn.execute(&format!("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,structural_terminal_id,structural_incarnation,structural_incarnation_kind,structural_host_boot,structural_host_epoch,structural_connection_epoch,structural_observation_sequence,created_at,home_node,is_local,host_id) VALUES ('s2','{INSTANCE}','resolved','native','w1:p2',1,1,'term-b','inc-2','coherent_enumeration','b2',1,1,1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))"), []).unwrap();
    fx.conn.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,terminal_id,incarnation,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,liveness) VALUES ('s2',1,'w1:p2','term-b','inc-2','b2',1,1,'claude','SB','execution-2','cooperative_top_level',100,'connected')", []).unwrap();
}

/// Gives `seat` a continuity key, as a service's reserve-with-seat does.
fn hold_key(fx: &mut Fx, seat: &str) {
    let tx = fx.conn.transaction().unwrap();
    let author =
        crate::store::service_substrate::ensure_reserved_author(&tx, INSTANCE, UtcMillis(0))
            .unwrap();
    let key = crate::protocol::ids::ContinuityKey::new(format!("{seat}-key"));
    crate::store::continuity_keys::reserve(
        &tx,
        &author,
        &crate::protocol::service::ReserveContinuityKey {
            key,
            seat: Some(SeatId::new(seat)),
            operation: crate::protocol::ids::OperationId::new("reserve"),
        },
        UtcMillis(1),
    )
    .unwrap();
    tx.commit().unwrap();
}

fn reversible_count(fx: &Fx, seat: &str) -> i64 {
    fx.conn
        .query_row(
            "SELECT count(*) FROM reversible_retirements WHERE seat_id=?1",
            [seat],
            |r| r.get(0),
        )
        .unwrap()
}

/// The two plan-driven `Closed` routes of a key holder. Kills: a reconcile path that still inserts
/// a job (and so an outcome that advances one), or loses the closure evidence.
#[test]
fn closed_retirement_of_key_holder_by_the_plan_writes_a_reversible_row() {
    // The same-incarnation close: the pane of a proved seat is gone from a coherent publication.
    let mut fx = fixture();
    fx.publish_and_pass(OPEN);
    seed_live_seat(&fx);
    hold_key(&mut fx, "s2");
    let publication = fx.publish(&[("w1:p1", "term-a")]);
    let (_, outcomes) = fx.pass(&publication);
    assert!(
        outcomes
            .iter()
            .any(|o| matches!(o, ReconciliationOutcome::RetiredReversibly)),
        "{outcomes:?}"
    );
    assert!(
        !outcomes
            .iter()
            .any(|o| matches!(o, ReconciliationOutcome::RetirementStarted(_)))
    );
    let state: String = fx
        .conn
        .query_row("SELECT state FROM seats WHERE id='s2'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(state, "retired");
    assert_eq!(reversible_count(&fx, "s2"), 1);

    // The restored-pane close: the watch's grace elapses for a key holder.
    let mut fx = fixture();
    hold_key(&mut fx, "s1");
    fx.publish_and_pass(OPEN);
    fx.publish_and_pass(CLOSED);
    for _ in 0..RESTORED_PANE_GRACE_MS / STEP {
        fx.advance(STEP);
        fx.publish_and_pass(CLOSED);
    }
    assert_eq!(fx.state(), "retired");
    assert_eq!(fx.watch(), None);
    let evidence: String = fx
        .conn
        .query_row(
            "SELECT closure_evidence FROM reversible_retirements WHERE seat_id='s1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(evidence, "restored_pane_closed");
    let jobs: i64 = fx
        .conn
        .query_row(
            "SELECT count(*) FROM retirements WHERE seat_id='s1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(jobs, 0, "RetireRestoredPane no longer updates a job row");
}

/// A fresh current-target read of `target` in the fixture's current boot.
fn fresh_observation(fx: &Fx, target: &str, terminal: Option<&str>) -> HostObservation {
    HostObservation {
        focused: false,
        target: HostTargetId::new(target),
        host_boot: HostBootId::new(fx.boot),
        epoch: fx.epoch,
        generation: 1,
        observed_at_utc: UtcMillis(fx.now()),
        observed_at_mono: MonoInstant(fx.now() as u64),
        provenance: ObservationProvenance::FreshCurrentTarget,
        occupant: None,
        ui: HostUiState::Idle,
        terminal: terminal.map(TerminalId::new),
        occupancy: StructuralOccupancy::EmptyShell,
        incarnation: IncarnationEvidence::Verified {
            identity: fx.incarnation.into(),
            evidence_kind: EvidenceKind::NativeCurrentTarget,
        },
        execution: ExecutionEvidence::Unknown,
        call_id: HostCallId::new(format!("relaunch-{target}")),
        connection_epoch: fx.epoch,
        observation_sequence: fx.sequence + 1,
        started_at_mono: MonoInstant(fx.now() as u64),
        completed_at_mono: MonoInstant(fx.now() as u64),
    }
}

/// Gives `seat` a joined membership and a pending requirement episode in thread `t`.
fn seed_standing(fx: &mut Fx, seat: &str) {
    let tx = fx.conn.transaction().unwrap();
    let author =
        crate::store::service_substrate::ensure_reserved_author(&tx, INSTANCE, UtcMillis(0))
            .unwrap();
    tx.execute(
        &format!("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence,journaled,managed_owner_author_id) VALUES ('t','{INSTANCE}','topic','goal',0,0,1,0,'{}')", author.as_str()),
        [],
    )
    .unwrap();
    tx.execute(
        "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t',?1,'joined')",
        [seat],
    )
    .unwrap();
    tx.execute(
        "INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at,origin_node) VALUES ('inv',?1,?2,1,'pending',1,1,100,9223372036854770000,(SELECT id FROM local_node))",
        ["t", seat],
    )
    .unwrap();
    tx.execute(
        "INSERT INTO requirement_episodes(id,thread_id,seat_id,issuer_author_id,invitation_id,state,created_decision_seq,created_at) VALUES ('req','t',?1,?2,'inv','pending',1,1)",
        [seat, author.as_str()],
    )
    .unwrap();
    tx.commit().unwrap();
}

/// The seeded membership and requirement episode both still effective.
fn assert_standing_effective(fx: &Fx, seat: &str, keyed: bool) {
    let thread = crate::protocol::ids::ThreadId::new("t");
    let seat = SeatId::new(seat);
    let membership =
        crate::store::service_substrate::effective_membership(&fx.conn, &thread, &seat).unwrap();
    assert_eq!(
        membership.voluntary,
        crate::protocol::service::VoluntaryMembershipState::Joined,
        "keyed={keyed}"
    );
    let requirement =
        crate::store::service_substrate::current_requirement(&fx.conn, &thread, &seat).unwrap();
    assert_eq!(
        requirement.map(|current| current.requirement),
        Some(crate::protocol::ids::RequirementId::new("req")),
        "keyed={keyed}"
    );
}

/// B3: an agent exiting with its pane open leaves the seat and its standing alone, keyless or
/// key-holding, in the same incarnation and after a restart. Kills: agent exit read as a close,
/// reversible or final, or costing a membership or a requirement episode.
#[test]
fn agent_exit_with_pane_open_keeps_the_seat() {
    for keyed in [false, true] {
        // Same incarnation: the binding ended, the pane still shown.
        let mut fx = fixture();
        fx.publish_and_pass(OPEN);
        seed_live_seat(&fx);
        if keyed {
            hold_key(&mut fx, "s2");
        }
        seed_standing(&mut fx, "s2");
        assert_standing_effective(&fx, "s2", keyed);
        let exit_at = UtcMillis(fx.now());
        let tx = fx.conn.transaction().unwrap();
        let bindings =
            crate::store::bindings::list_open_top_level(&tx, &SeatId::new("s2")).unwrap();
        assert_eq!(bindings.len(), 1, "keyed={keyed}");
        assert!(
            crate::store::bindings::disconnect_with_hooks(
                &tx,
                &bindings[0],
                crate::store::bindings::DisconnectReason::ProcessExit,
                exit_at,
                &[],
            )
            .unwrap(),
            "keyed={keyed}"
        );
        tx.commit().unwrap();
        for _ in 0..3 {
            fx.advance(STEP);
            fx.publish_and_pass(OPEN);
        }
        let (state, retirements): (String, i64) = fx
            .conn
            .query_row(
                "SELECT state,(SELECT count(*) FROM retirements WHERE seat_id='s2') FROM seats WHERE id='s2'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            (state.as_str(), retirements),
            ("resolved", 0),
            "keyed={keyed}"
        );
        assert_eq!(reversible_count(&fx, "s2"), 0, "keyed={keyed}");
        assert_standing_effective(&fx, "s2", keyed);
        // The relaunch keeps the seat: the pane's resolved owner answers, with or without its key.
        for continuity_key in [
            None,
            keyed.then(|| crate::protocol::ids::ContinuityKey::new("s2-key")),
        ] {
            let request = crate::protocol::commands::ResolveSeat {
                target: HostTargetId::new("w1:p2"),
                host: crate::test_support::test_host_locator(),
                operation: crate::protocol::ids::OperationId::new(format!(
                    "relaunch-{}",
                    uuid::Uuid::new_v4()
                )),
                locator_hint: None,
                tmux_hint: None,
                continuity_key,
            };
            fx.sequence += 1;
            let observation = fresh_observation(&fx, "w1:p2", Some("term-b"));
            let budget = budget();
            let admission = begin_host_observation(
                &fx.context,
                &mut fx.conn,
                INSTANCE,
                crate::test_support::test_host_id(),
                &budget,
            )
            .unwrap();
            assert!(
                publish_current_target_observation(
                    &fx.context,
                    &mut fx.conn,
                    &admission,
                    &observation,
                    &budget
                )
                .unwrap()
            );
            let guard =
                OrdinaryResolutionGuard::try_new(&request, observation, &admission).unwrap();
            let outcome = resolve_seat(
                &fx.context,
                &mut fx.conn,
                INSTANCE,
                request,
                OrdinaryResolutionAttempt::Observed(guard),
                &budget,
            )
            .unwrap();
            assert_eq!(
                outcome,
                OrdinaryResolutionOutcome::Resolved(SeatId::new("s2")),
                "keyed={keyed}"
            );
        }
        assert_standing_effective(&fx, "s2", keyed);

        // After a restart: unresolved, watched, never closed while the pane is shown.
        let mut fx = fixture();
        if keyed {
            hold_key(&mut fx, "s1");
        }
        seed_standing(&mut fx, "s1");
        for _ in 0..=(RESTORED_PANE_GRACE_MS / STEP) + 1 {
            fx.advance(STEP);
            fx.publish_and_pass(OPEN);
        }
        assert_eq!(fx.state(), "unresolved", "keyed={keyed}");
        assert_eq!(reversible_count(&fx, "s1"), 0, "keyed={keyed}");
        assert_standing_effective(&fx, "s1", keyed);
        // The relaunch keeps the seat: its resumed session reattaches it to that pane.
        let command = crate::protocol::commands::ContinuityCheckIn {
            host: crate::test_support::test_host_locator(),
            target: HostTargetId::new("w1:p1"),
            harness: crate::protocol::authority::Harness::Claude,
            native_session: crate::protocol::ids::NativeSessionId::new("SA"),
            source: crate::protocol::commands::ContinuityCheckIn::RESUME_SOURCE.into(),
            operation: crate::protocol::ids::OperationId::new("relaunch"),
            execution: crate::protocol::ids::ExecutionId::new("execution-new"),
            liveness: Some(crate::protocol::authority::LivenessFingerprint { pid: 42, start: 7 }),
            continuity_key: keyed.then(|| crate::protocol::ids::ContinuityKey::new("s1-key")),
        };
        let observation = fresh_observation(&fx, "w1:p1", Some("term-a"));
        let budget = budget();
        let admission = begin_host_observation(
            &fx.context,
            &mut fx.conn,
            INSTANCE,
            crate::test_support::test_host_id(),
            &budget,
        )
        .unwrap();
        assert!(
            publish_current_target_observation(
                &fx.context,
                &mut fx.conn,
                &admission,
                &observation,
                &budget
            )
            .unwrap()
        );
        let guard = crate::ports::ContinuityTargetGuard::try_new(
            INSTANCE,
            crate::test_support::test_host_id(),
            &command,
            observation,
        )
        .unwrap();
        decide_continuity(
            &fx.context,
            &mut fx.conn,
            INSTANCE,
            crate::ports::ContinuityRequest {
                command,
                guard,
                diagnostic: "match",
                evidence: crate::ports::ContinuityEvidence::Cooperative,
            },
        )
        .unwrap();
        assert_eq!(fx.state(), "resolved", "keyed={keyed}");
        assert_eq!(reversible_count(&fx, "s1"), 0, "keyed={keyed}");
        assert_standing_effective(&fx, "s1", keyed);
    }
}

/// An unretired seat waits with no pane. Kills: the plan acting on it, or its row keeping the hold.
#[test]
fn an_unretired_seat_is_outside_the_reconciliation_plan() {
    let mut fx = fixture();
    fx.conn.execute_batch(
        "UPDATE seats SET target_id=NULL WHERE id='s1';
         INSERT INTO reversible_retirements(seat_id,state,cutover_at,reason,unretired_at) VALUES ('s1','unretired',1,'closed',2);",
    ).unwrap();
    let publication = fx.publish(OPEN);
    let budget = budget();
    let page =
        saved_seats_page(&fx.context, &fx.conn, &publication.id, 0, None, 16, &budget).unwrap();
    assert!(page.seats.is_empty());
    assert!(!page.has_more);
    assert!(plan_page(&page).unwrap().is_empty());
    fx.pass(&publication);
    assert_eq!(fx.state(), "unresolved");
    let hold: i64 = fx
        .conn
        .query_row(
            "SELECT baseline_hold_unclaimed FROM herdr_hosts WHERE host_id=(SELECT min(host_id) FROM herdr_hosts)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(hold, 0, "the waiting seat holds nothing");
}

// G5 (ht-ma6i.4): an unresolved seat whose saved target never returns.

// The fixture's saved target `w1:p1` is absent from `CLOSED`, the baseline included.
impl Fx {
    /// The absence ledger's (boot, seen, observed) for `s1`.
    fn absence(&self) -> Option<(String, Option<i64>, i64)> {
        self.conn
            .query_row(
                "SELECT host_boot,absent_seen_at,absent_observed_ms FROM restored_target_absence WHERE seat_id='s1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .unwrap()
    }
    fn observed(&self) -> i64 {
        self.absence().unwrap().2
    }
    fn hold(&self) -> i64 {
        self.conn
            .query_row(
                "SELECT baseline_hold_unclaimed FROM herdr_hosts WHERE host_id=(SELECT min(host_id) FROM herdr_hosts)",
                [],
                |r| r.get(0),
            )
            .unwrap()
    }
    fn keep_claim(&self, harness: &str, session: &str, key: Option<&str>) {
        self.conn.execute(
            "INSERT INTO kept_continuity_claims(host_id,target_id,harness,native_session,command_json,received_at,continuity_key) VALUES ((SELECT min(host_id) FROM herdr_hosts),'w1:p7',?1,?2,'{}',0,?3)",
            params![harness, session, key],
        ).unwrap();
    }
    /// Baseline without the target, marker recorded, the count started at zero.
    fn absence_started(&mut self) {
        assert_eq!(self.publish_and_pass(CLOSED), RestoredPane::None);
        assert_eq!(self.publish_and_pass(CLOSED), RestoredPane::TargetAbsent);
        assert_eq!(self.observed(), 0);
    }
    /// Runs twice the grace of passes, each `STEP` apart, all answering `expected`.
    fn hold_for_twice_the_grace(&mut self, expected: RestoredPane) {
        for _ in 0..2 * RESTORED_PANE_GRACE_MS / STEP {
            self.advance(STEP);
            assert_eq!(self.publish_and_pass(CLOSED), expected);
        }
        assert_eq!(self.state(), "unresolved");
    }
}

/// Kills: no step cap, a grace off by one pass in either direction.
#[test]
fn absent_target_retires_after_ten_observed_minutes_with_step_cap() {
    let mut fx = fixture();
    fx.absence_started();
    // A suspend between two passes counts one step.
    fx.advance(100 * STEP);
    assert_eq!(fx.publish_and_pass(CLOSED), RestoredPane::TargetAbsent);
    assert_eq!(fx.observed(), STEP);
    for pass in 2..RESTORED_PANE_GRACE_MS / STEP {
        fx.advance(STEP);
        assert_eq!(fx.publish_and_pass(CLOSED), RestoredPane::TargetAbsent);
        assert_eq!(fx.observed(), pass * STEP);
        assert_eq!(fx.state(), "unresolved");
    }
    fx.advance(STEP);
    assert_eq!(fx.publish_and_pass(CLOSED), RestoredPane::TargetGone);
    assert_eq!(fx.state(), "retired");
}

/// Kills: evidence left off the job, `closure_evidence` set instead, a ledger row left behind, a
/// hold not lifted.
#[test]
fn absence_evidence_is_recorded_and_holds_lift() {
    let mut fx = fixture();
    fx.absence_started();
    assert_eq!(fx.hold(), 1, "the unresolved seat holds the baseline");
    for _ in 1..RESTORED_PANE_GRACE_MS / STEP {
        fx.advance(STEP);
        fx.publish_and_pass(CLOSED);
    }
    fx.advance(STEP);
    let publication = fx.publish(CLOSED);
    let (evidence, outcomes) = fx.pass(&publication);
    assert_eq!(evidence, RestoredPane::TargetGone);
    assert!(
        outcomes
            .iter()
            .any(|o| matches!(o, ReconciliationOutcome::RetirementStarted(_))),
        "{outcomes:?}"
    );
    type JobRow = (Option<String>, Option<String>, String, i64, String, i64);
    let row: JobRow = fx
        .conn
        .query_row(
            "SELECT absence_evidence,closure_evidence,closure_boot,closure_epoch,closure_target,closure_generation FROM retirements WHERE seat_id='s1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )
        .unwrap();
    assert_eq!(
        row,
        (
            Some("restored_target_absent".into()),
            None,
            "b2".into(),
            1,
            "w1:p1".into(),
            1
        )
    );
    assert_eq!(fx.absence(), None);
    assert_eq!(fx.hold(), 0);
}

/// Kills: a path around the disposition that writes a job or drops the evidence or the key.
#[test]
fn key_holder_retires_reversibly() {
    let mut fx = fixture();
    hold_key(&mut fx, "s1");
    fx.absence_started();
    for _ in 1..RESTORED_PANE_GRACE_MS / STEP {
        fx.advance(STEP);
        fx.publish_and_pass(CLOSED);
    }
    fx.advance(STEP);
    let publication = fx.publish(CLOSED);
    let (_, outcomes) = fx.pass(&publication);
    assert!(
        outcomes
            .iter()
            .any(|o| matches!(o, ReconciliationOutcome::RetiredReversibly)),
        "{outcomes:?}"
    );
    assert_eq!(fx.state(), "retired");
    let row: (Option<String>, Option<String>) = fx
        .conn
        .query_row(
            "SELECT absence_evidence,closure_evidence FROM reversible_retirements WHERE seat_id='s1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(row, (Some("restored_target_absent".into()), None));
    let (jobs, keys): (i64, i64) = fx
        .conn
        .query_row(
            "SELECT (SELECT count(*) FROM retirements WHERE seat_id='s1'),(SELECT count(*) FROM continuity_keys WHERE seat_id='s1')",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((jobs, keys), (0, 1));
    assert_eq!(fx.absence(), None);
}

/// Kills: a shown target that keeps the count (either path), and a reset that also ends the watch
/// rule's turn.
#[test]
fn target_reappearing_resets_and_hands_over_to_the_watch() {
    // The target is not in the baseline: the row resets and the watch rule watches nothing.
    let mut fx = fixture();
    fx.absence_started();
    for _ in 0..4 {
        fx.advance(STEP);
        fx.publish_and_pass(CLOSED);
    }
    assert_eq!(fx.observed(), 4 * STEP);
    assert_eq!(fx.publish_and_pass(OPEN), RestoredPane::TargetShown);
    assert_eq!(fx.absence(), None);
    assert_eq!(fx.publish_and_pass(OPEN), RestoredPane::None);
    // Absent again: the full grace is owed from zero.
    assert_eq!(fx.publish_and_pass(CLOSED), RestoredPane::TargetAbsent);
    assert_eq!(fx.observed(), 0);

    // The baseline held it but no pass saw it: the watch rule takes over, with precedence, and
    // the counted absence goes with it.
    let mut fx = fixture();
    fx.publish(OPEN);
    fx.absence_started();
    fx.advance(STEP);
    fx.publish_and_pass(CLOSED);
    assert_eq!(fx.observed(), STEP);
    assert_eq!(fx.publish_and_pass(OPEN), RestoredPane::Observe);
    assert_eq!(fx.absence(), None);
    assert_eq!(fx.watch(), Some(("term-a".into(), None, 0)));
    assert_eq!(fx.publish_and_pass(OPEN), RestoredPane::Watching);
    assert_eq!(fx.publish_and_pass(CLOSED), RestoredPane::Absent);
    assert_eq!(fx.absence(), None, "the watch owns the closure now");
}

impl Fx {
    /// Seat `s3`'s structural proof names `term-a` in `incarnation`.
    fn s3_proof_names_term_a(&self, incarnation: &str) {
        self.conn.execute(&format!("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id,structural_terminal_id,structural_incarnation,structural_incarnation_kind,structural_host_boot,structural_host_epoch,structural_connection_epoch,structural_observation_sequence) VALUES ('s3','{INSTANCE}','resolved','native','w1:p9',1,1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts),'term-a','{incarnation}','coherent_enumeration','b2',1,1,1)"), []).unwrap();
    }
    /// Seat `s4`, live at `w1:p1` where `term-a` shows: its structural proof and
    /// its open binding name `term-a` in `inc-2`. A resolved seat with no proof of
    /// the current incarnation is unresolved by the first pass, which ends its
    /// binding, so a live binding comes with its seat's proof.
    fn s4_binding_names_term_a(&self) {
        self.conn.execute(&format!("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id,structural_terminal_id,structural_incarnation,structural_incarnation_kind,structural_host_boot,structural_host_epoch,structural_connection_epoch,structural_observation_sequence) VALUES ('s4','{INSTANCE}','resolved','native','w1:p1',1,1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts),'term-a','inc-2','coherent_enumeration','b2',1,1,1)"), []).unwrap();
        self.conn.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,terminal_id,incarnation,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,ended_at,liveness,disconnected_at,disconnect_reason) VALUES ('s4',1,'w1:p1','term-a','inc-2','b2',1,1,'claude','SB','execution-4','cooperative_top_level',100,NULL,'connected',NULL,NULL)", []).unwrap();
    }
    /// Seat `s3`, retired, keeps its structural proof naming `term-a` in `inc-2`
    /// (a retirement updates only state, generation, retired_at and retired_seq).
    fn retired_s3_proof_names_term_a(&self) {
        self.conn.execute(&format!("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id,structural_terminal_id,structural_incarnation,structural_incarnation_kind,structural_host_boot,structural_host_epoch,structural_connection_epoch,structural_observation_sequence,retired_at) VALUES ('s3','{INSTANCE}','retired','native','w1:p9',1,1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts),'term-a','inc-2','coherent_enumeration','b2',1,1,1,50)"), []).unwrap();
    }
    /// Seat `s4`'s binding named `term-a` in `inc-2` and has ended (its session left).
    fn ended_s4_binding_names_term_a(&self) {
        self.conn.execute(&format!("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) VALUES ('s4','{INSTANCE}','resolved','native','w1:p9',1,1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))"), []).unwrap();
        self.conn.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,terminal_id,incarnation,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,ended_at,liveness,disconnected_at,disconnect_reason) VALUES ('s4',1,'w1:p9','term-a','inc-2','b2',1,1,'claude','SB','execution-4','cooperative_top_level',100,300,'disconnected',300,'seat_unresolved')", []).unwrap();
    }
}

/// ht-ma6i.41. Kills: a shown target whose pane another seat's proof or binding names that
/// waits forever (neither watched nor counted), and a count that skips the grace or the step cap.
#[test]
fn a_target_showing_another_seats_pane_counts_as_absent() {
    let claims: [fn(&Fx); 2] = [
        |fx| fx.s3_proof_names_term_a("inc-2"),
        |fx| fx.s4_binding_names_term_a(),
    ];
    for (index, claim) in claims.into_iter().enumerate() {
        let mut fx = fixture();
        claim(&fx);
        // First pass: no marker yet, nothing counts; the claimed pane is never watched.
        assert_eq!(fx.publish_and_pass(OPEN), RestoredPane::None, "{index}");
        assert_eq!(
            fx.publish_and_pass(OPEN),
            RestoredPane::TargetAbsent,
            "{index}"
        );
        assert_eq!(fx.observed(), 0, "{index}");
        assert_eq!(fx.watch(), None, "{index}");
        // A suspend between two passes counts one step.
        fx.advance(100 * STEP);
        assert_eq!(
            fx.publish_and_pass(OPEN),
            RestoredPane::TargetAbsent,
            "{index}"
        );
        assert_eq!(fx.observed(), STEP, "{index}");
        for pass in 2..RESTORED_PANE_GRACE_MS / STEP {
            fx.advance(STEP);
            assert_eq!(
                fx.publish_and_pass(OPEN),
                RestoredPane::TargetAbsent,
                "{index}"
            );
            assert_eq!(fx.observed(), pass * STEP, "{index}");
            assert_eq!(fx.state(), "unresolved", "{index}");
        }
        fx.advance(STEP);
        assert_eq!(
            fx.publish_and_pass(OPEN),
            RestoredPane::TargetGone,
            "{index}"
        );
        assert_eq!(fx.state(), "retired", "{index}");
        let evidence: Option<String> = fx
            .conn
            .query_row(
                "SELECT absence_evidence FROM retirements WHERE seat_id='s1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            evidence.as_deref(),
            Some("restored_target_absent"),
            "{index}"
        );
    }
}

/// ht-ma6i.41. Kills: a claim of another incarnation read as this one's, and a count that
/// ignores the kept-claim block on the claimed-pane path.
#[test]
fn another_incarnations_claim_is_watched_and_a_kept_claim_still_blocks() {
    // A proof from the old incarnation names nothing in inc-2: the pane is watched as before.
    let mut fx = fixture();
    fx.s3_proof_names_term_a("inc-old");
    assert_eq!(fx.publish_and_pass(OPEN), RestoredPane::Observe);
    assert_eq!(fx.watch().map(|w| w.0), Some("term-a".into()));

    // A kept C1 claim for the seat's session blocks the claimed-pane count like any absence.
    let mut fx = fixture();
    fx.s3_proof_names_term_a("inc-2");
    fx.keep_claim("claude", "SA", None);
    fx.publish_and_pass(OPEN);
    for _ in 0..2 * RESTORED_PANE_GRACE_MS / STEP {
        fx.advance(STEP);
        assert_eq!(fx.publish_and_pass(OPEN), RestoredPane::None);
    }
    assert_eq!(fx.state(), "unresolved");
    assert_eq!(fx.absence(), None, "nothing counted");
}

/// ht-ma6i.42. Kills: an ended binding or a retired seat's retained proof read as a live
/// claim on the absence path (the seat retired, finally when keyless, on stale evidence), and
/// a narrowing that also lets such a pane be watched.
#[test]
fn a_departed_claimant_leaves_the_seat_waiting() {
    let claims: [fn(&Fx); 2] = [
        |fx| fx.retired_s3_proof_names_term_a(),
        |fx| fx.ended_s4_binding_names_term_a(),
    ];
    for (index, claim) in claims.into_iter().enumerate() {
        let mut fx = fixture();
        claim(&fx);
        for _ in 0..2 * RESTORED_PANE_GRACE_MS / STEP {
            assert_eq!(fx.publish_and_pass(OPEN), RestoredPane::None, "{index}");
            fx.advance(STEP);
        }
        assert_eq!(fx.publish_and_pass(OPEN), RestoredPane::None, "{index}");
        assert_eq!(fx.absence(), None, "{index}: nothing counted");
        assert_eq!(fx.watch(), None, "{index}: the watch stays withheld");
        assert_eq!(fx.state(), "unresolved", "{index}");
    }
}

/// ht-ma6i.42. The predicate itself, per row kind: the absence path (`live_only`) counts a
/// live seat's proof and an open binding only; the watch counts every row. Kills: a narrowing
/// that drops the open-binding arm (the pass above cannot keep a proofless binding open) or
/// that reaches the watch.
#[test]
fn only_a_live_claimant_counts_on_the_absence_path() {
    let open_binding = |fx: &Fx| {
        fx.conn.execute(&format!("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) VALUES ('s4','{INSTANCE}','resolved','native','w1:p9',1,1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))"), []).unwrap();
        fx.conn.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,terminal_id,incarnation,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,ended_at,liveness,disconnected_at,disconnect_reason) VALUES ('s4',1,'w1:p9','term-a','inc-2','b2',1,1,'claude','SB','execution-4','cooperative_top_level',100,NULL,'connected',NULL,NULL)", []).unwrap();
    };
    /// A claimant's rows, and whether they are live.
    type Claim = (fn(&Fx), bool);
    let claims: [Claim; 4] = [
        (|fx| fx.s3_proof_names_term_a("inc-2"), true),
        (|fx| fx.retired_s3_proof_names_term_a(), false),
        (open_binding, true),
        (|fx| fx.ended_s4_binding_names_term_a(), false),
    ];
    let seat = SeatId::new("s1");
    let host = crate::test_support::test_host_id();
    for (index, (claim, live)) in claims.into_iter().enumerate() {
        let fx = fixture();
        claim(&fx);
        let claimed = |live_only| {
            terminal_claimed_elsewhere(&fx.conn, &seat, host, "term-a", "inc-2", live_only).unwrap()
        };
        assert_eq!(claimed(true), live, "{index}: absence path");
        assert!(claimed(false), "{index}: the watch counts every row");
    }
}

/// Kills: a block on any kept row, or none, and a block that forgets the count.
#[test]
fn kept_c1_claim_blocks_the_count() {
    let mut fx = fixture();
    fx.absence_started();
    fx.advance(STEP);
    fx.publish_and_pass(CLOSED);
    // Another session's claim names nothing of this seat.
    fx.keep_claim("claude", "SX", None);
    fx.advance(STEP);
    assert_eq!(fx.publish_and_pass(CLOSED), RestoredPane::TargetAbsent);
    assert_eq!(fx.observed(), 2 * STEP);
    // The seat's own top-level session, of any age.
    fx.keep_claim("claude", "SA", None);
    fx.hold_for_twice_the_grace(RestoredPane::None);
    assert_eq!(fx.observed(), 2 * STEP, "nothing counted");
    fx.conn
        .execute("DELETE FROM kept_continuity_claims", [])
        .unwrap();
    fx.advance(STEP);
    assert_eq!(fx.publish_and_pass(CLOSED), RestoredPane::TargetAbsent);
    assert_eq!(fx.observed(), 3 * STEP);
}

/// Kills: a key test that matches any keyed claim, and a watch that does not share it.
#[test]
fn kept_claim_with_the_seats_key_blocks_the_count() {
    let mut fx = fixture();
    hold_key(&mut fx, "s1");
    fx.absence_started();
    fx.keep_claim("codex", "SZ", Some("another-key"));
    fx.advance(STEP);
    assert_eq!(fx.publish_and_pass(CLOSED), RestoredPane::TargetAbsent);
    assert_eq!(fx.observed(), STEP);
    fx.keep_claim("codex", "SY", Some("s1-key"));
    fx.hold_for_twice_the_grace(RestoredPane::None);
    assert_eq!(fx.observed(), STEP, "nothing counted");

    // The ht-wpq4 watch shares the guard.
    let mut fx = fixture();
    hold_key(&mut fx, "s1");
    fx.publish_and_pass(OPEN);
    fx.keep_claim("codex", "SY", Some("s1-key"));
    assert_eq!(fx.publish_and_pass(CLOSED), RestoredPane::Watching);
}

/// Kills: a count while the marker lags, and a catch-up that credits the lag beyond one step.
#[test]
fn lagging_marker_adds_nothing() {
    let mut fx = fixture();
    fx.absence_started();
    fx.record_marker = false;
    fx.conn
        .execute(
            "UPDATE herdr_hosts SET reconciled_boot=NULL,reconciled_epoch=NULL",
            [],
        )
        .unwrap();
    fx.hold_for_twice_the_grace(RestoredPane::None);
    assert_eq!(fx.observed(), 0, "no absence time counted");
    fx.record_marker = true;
    fx.publish_and_pass(CLOSED);
    fx.advance(STEP);
    assert_eq!(fx.publish_and_pass(CLOSED), RestoredPane::TargetAbsent);
    assert!(fx.observed() <= 2 * STEP, "{}", fx.observed());
    assert_eq!(fx.state(), "unresolved");
}

/// Kills: the rule on a host kind with no absence authority, or on a seat with no target.
#[test]
fn named_hc_and_unretired_seats_are_never_retired_this_way() {
    for kind in ["named", "hc"] {
        let mut fx = fixture();
        fx.conn
            .execute("UPDATE herdr_hosts SET kind=?1", [kind])
            .unwrap();
        assert_eq!(fx.publish_and_pass(CLOSED), RestoredPane::None, "{kind}");
        fx.hold_for_twice_the_grace(RestoredPane::None);
        assert_eq!(fx.absence(), None, "{kind}");
    }

    // An unretired seat has no target: no page row, and a forged action is refused.
    let mut fx = fixture();
    fx.conn.execute_batch(
        "UPDATE seats SET target_id=NULL WHERE id='s1';
         INSERT INTO reversible_retirements(seat_id,state,cutover_at,reason,unretired_at) VALUES ('s1','unretired',1,'closed',2);",
    ).unwrap();
    fx.publish_and_pass(CLOSED);
    fx.hold_for_twice_the_grace(RestoredPane::None);
    assert_eq!(fx.absence(), None);
    let publication = fx.publish(CLOSED);
    for expected_target in [None, Some(HostTargetId::new("w1:p1"))] {
        let outcome = apply_reconciliation_transition(
            &fx.context,
            &mut fx.conn,
            GuardedSeatTransition {
                publication: publication.clone(),
                seat: SeatId::new("s1"),
                expected_binding_generation: 2,
                expected_target,
                expected_terminal: None,
                action: ReconciliationAction::RetireAbsentTarget {
                    absent_target: HostTargetId::new("w1:p1"),
                },
            },
            &budget(),
        )
        .unwrap();
        assert!(matches!(outcome, ReconciliationOutcome::Stale(_)));
    }
    assert_eq!(fx.state(), "unresolved");
}

/// Kills: a row of another boot, incarnation or generation counted, or left in place.
#[test]
fn stale_row_of_another_boot_is_replaced() {
    let mut fx = fixture();
    fx.absence_started();
    for _ in 0..4 {
        fx.advance(STEP);
        fx.publish_and_pass(CLOSED);
    }
    assert_eq!(fx.observed(), 4 * STEP);
    // A new boot: that row is not this count.
    fx.boot = "b3";
    fx.incarnation = "inc-3";
    fx.sequence = 0;
    fx.publish_and_pass(CLOSED);
    fx.advance(STEP);
    assert_eq!(fx.publish_and_pass(CLOSED), RestoredPane::TargetAbsent);
    let (boot, _, observed) = fx.absence().unwrap();
    assert_eq!((boot.as_str(), observed), ("b3", 0));

    // Another mapping generation voids the row too.
    fx.advance(STEP);
    fx.publish_and_pass(CLOSED);
    assert_eq!(fx.observed(), STEP);
    fx.conn
        .execute("UPDATE seats SET generation=generation+1 WHERE id='s1'", [])
        .unwrap();
    fx.advance(STEP);
    assert_eq!(fx.publish_and_pass(CLOSED), RestoredPane::TargetAbsent);
    assert_eq!(fx.observed(), 0);
    let generation: i64 = fx
        .conn
        .query_row(
            "SELECT seat_generation FROM restored_target_absence WHERE seat_id='s1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(generation, 3);
}

/// Kills: an applier that trusts the plan's evidence instead of re-deriving it.
#[test]
fn the_applier_refuses_an_absent_retirement_whose_evidence_changed_after_the_page() {
    let mut fx = fixture();
    fx.absence_started();
    for _ in 1..RESTORED_PANE_GRACE_MS / STEP {
        fx.advance(STEP);
        fx.publish_and_pass(CLOSED);
    }
    fx.advance(STEP);
    let publication = fx.publish(CLOSED);
    let budget = budget();
    let page =
        saved_seats_page(&fx.context, &fx.conn, &publication.id, 0, None, 16, &budget).unwrap();
    let retire = plan_page(&page)
        .unwrap()
        .into_iter()
        .find(|t| t.seat.as_str() == "s1")
        .unwrap();
    assert!(matches!(
        retire.action,
        ReconciliationAction::RetireAbsentTarget { .. }
    ));
    fx.keep_claim("claude", "SA", None);
    let outcome =
        apply_reconciliation_transition(&fx.context, &mut fx.conn, retire, &budget).unwrap();
    assert!(matches!(outcome, ReconciliationOutcome::Stale(_)));
    assert_eq!(fx.state(), "unresolved");
}
