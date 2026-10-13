//! Send-eligibility revision across observation-lane snapshot publications.
//!
//! The 5 s observation lane republishes the host namespace even when nothing
//! changed. A publication that changes no send eligibility must not bump
//! `host_instances.send_eligibility_revision` (shared contract amendment §8:
//! "A new evidence observation that changes no eligibility/proof/binding does
//! not require a bump"; "A stable finite snapshot remains resumable"), or every
//! send whose preparation spans one is refused with "send preparation snapshot
//! changed". Every publication that can change eligibility still bumps it
//! exactly once.

use crate::{
    ports::{
        DurableWorkAdmission, EnumerationEvidence, EvidenceKind, ExecutionEvidence,
        HostInvalidationReason, HostObservation, HostSnapshot, HostUiState, IncarnationEvidence,
        ObservationProvenance, SendPreparationProgress, SnapshotHeader, StructuralOccupancy,
    },
    protocol::{
        authority::{CallerClaim, CallerRole, Harness, MutationPermit, ObligationRef},
        commands::SendMessage,
        ids::*,
        results::{CommandResult, ErrorCode},
        time::{CallBudget, Clock, MonoInstant, UtcMillis},
    },
    store::{connection::StoreContext, messages, seats},
    test_support::isolation::TestIsolation,
};
use rusqlite::Connection;
use std::sync::Arc;

struct FixedClock;
impl Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(1000)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(100)
    }
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    }
}

/// One captured host namespace. `sequence` is assigned per publication.
#[derive(Clone)]
struct Capture {
    boot: &'static str,
    epoch: u64,
    incarnation: &'static str,
    targets: Vec<Target>,
}

#[derive(Clone)]
struct Target {
    id: String,
    generation: u64,
    occupancy: StructuralOccupancy,
    ui: HostUiState,
    observed_at: i64,
}

fn target(id: &str) -> Target {
    Target {
        id: id.into(),
        generation: 1,
        occupancy: StructuralOccupancy::Occupied,
        ui: HostUiState::Idle,
        observed_at: 100,
    }
}

fn capture(ids: &[&str]) -> Capture {
    Capture {
        boot: "b",
        epoch: 1,
        incarnation: "incarnation",
        targets: ids.iter().map(|id| target(id)).collect(),
    }
}

fn host_snapshot(capture: &Capture, sequence: u64) -> HostSnapshot {
    HostSnapshot {
        boot: HostBootId::new(capture.boot),
        epoch: capture.epoch,
        observation_sequence: sequence,
        complete: true,
        enumeration: EnumerationEvidence::CoherentVerified,
        incarnation: IncarnationEvidence::Verified {
            identity: capture.incarnation.into(),
            evidence_kind: EvidenceKind::CoherentEnumeration,
        },
        targets: capture
            .targets
            .iter()
            .map(|target| HostObservation {
                focused: false,
                target: HostTargetId::new(target.id.as_str()),
                host_boot: HostBootId::new(capture.boot),
                epoch: capture.epoch,
                generation: target.generation,
                observed_at_utc: UtcMillis(target.observed_at),
                observed_at_mono: MonoInstant(100),
                provenance: ObservationProvenance::CoherentEnumeration,
                occupant: None,
                ui: target.ui,
                terminal: None,
                occupancy: target.occupancy,
                incarnation: IncarnationEvidence::Unknown,
                execution: ExecutionEvidence::Unknown,
                call_id: HostCallId::new(format!("capture-{}-{sequence}", target.id)),
                connection_epoch: 1,
                observation_sequence: sequence,
                started_at_mono: MonoInstant(100),
                completed_at_mono: MonoInstant(100),
            })
            .collect(),
    }
}

/// Stages, seals and publishes one capture through the production stage API,
/// in slices of at most 16 targets as the observation lane does.
fn publish(context: &StoreContext, conn: &mut Connection, capture: &Capture, sequence: u64) {
    publish_from(context, context, conn, capture, sequence);
}

/// `publish`, with the stage built under `staging` and published under
/// `publishing` (two contexts model a restart or another process).
fn publish_from(
    staging: &StoreContext,
    publishing: &StoreContext,
    conn: &mut Connection,
    capture: &Capture,
    sequence: u64,
) {
    let context = staging;
    let budget = budget();
    let admission = seats::begin_host_observation(context, conn, "i", &budget).unwrap();
    let snapshot = host_snapshot(capture, sequence);
    let header = SnapshotHeader::from_captured(admission, &snapshot).unwrap();
    let stage = seats::begin_snapshot_stage(context, conn, header, &budget).unwrap();
    let mut offset = 0u64;
    for chunk in snapshot.targets.chunks(16) {
        let progress = seats::stage_snapshot_targets(
            context,
            conn,
            &stage.id,
            offset,
            chunk,
            DurableWorkAdmission::new(16).unwrap(),
            &budget,
        )
        .unwrap();
        offset = progress.stage.staged_targets;
    }
    seats::seal_snapshot_stage(context, conn, &stage.id, &budget).unwrap();
    let published = seats::publish_snapshot_stage(publishing, conn, &stage.id, &budget).unwrap();
    assert_eq!(published.observation_sequence, sequence);
}

fn revision(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT send_eligibility_revision FROM host_instances WHERE id='i'",
        [],
        |r| r.get(0),
    )
    .unwrap()
}

fn host_store(iso: &TestIsolation) -> (StoreContext, Connection) {
    let context = StoreContext::new(iso.path("store.db"), Arc::new(FixedClock));
    let conn = context.open_writer().unwrap();
    conn.execute(
        "INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'b',1,1)",
        [],
    )
    .unwrap();
    (context, conn)
}

// Kills: an unconditional bump on every observation-lane publication. Three
// identical captures (one spanning several 16-target slices) and a capture
// that differs only in volatile, non-eligibility fields (UI state, observed
// time, sequence) leave the revision where the first publication set it.
#[test]
fn unchanged_snapshot_publications_keep_the_send_eligibility_revision() {
    let iso = TestIsolation::new("snapshot-eligibility-unchanged");
    let (context, mut conn) = host_store(&iso);
    let ids: Vec<String> = (0..40).map(|n| format!("p{n}")).collect();
    let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
    let base = capture(&ids);
    assert_eq!(revision(&conn), 0);
    publish(&context, &mut conn, &base, 1);
    assert_eq!(revision(&conn), 1, "the first publication bumps once");
    publish(&context, &mut conn, &base, 2);
    publish(&context, &mut conn, &base, 3);
    assert_eq!(revision(&conn), 1, "identical publications do not bump");
    let mut volatile = base.clone();
    for target in &mut volatile.targets {
        target.ui = HostUiState::ActiveTurn;
        target.observed_at = 900;
    }
    publish(&context, &mut conn, &volatile, 4);
    assert_eq!(revision(&conn), 1, "UI state and observed time do not bump");
}

/// Publishes the base capture twice (asserting the second does not bump),
/// applies `change`, then publishes `next` and asserts exactly one bump.
fn assert_change_bumps_once(
    label: &str,
    change: impl FnOnce(&StoreContext, &mut Connection),
    next: impl Fn(&Capture) -> Capture,
) {
    let iso = TestIsolation::new(&format!("snapshot-eligibility-{label}"));
    let (context, mut conn) = host_store(&iso);
    let base = capture(&["p1", "p2", "p3"]);
    publish(&context, &mut conn, &base, 1);
    publish(&context, &mut conn, &base, 2);
    assert_eq!(revision(&conn), 1, "{label}: baseline is stable");
    change(&context, &mut conn);
    let before = revision(&conn);
    publish(&context, &mut conn, &next(&base), 3);
    assert_eq!(revision(&conn), before + 1, "{label}: bumps exactly once");
    // The changed state is the new baseline: republishing it is stable.
    let after = revision(&conn);
    let latest = next(&base);
    publish(&context, &mut conn, &latest, 4);
    assert_eq!(revision(&conn), after, "{label}: republication is stable");
}

#[test]
fn structural_generation_change_bumps_once() {
    assert_change_bumps_once(
        "generation",
        |_, _| {},
        |base| {
            let mut next = base.clone();
            next.targets[1].generation = 2;
            next
        },
    );
}

#[test]
fn occupancy_change_bumps_once() {
    assert_change_bumps_once(
        "occupancy",
        |_, _| {},
        |base| {
            let mut next = base.clone();
            next.targets[0].occupancy = StructuralOccupancy::EmptyShell;
            next
        },
    );
}

#[test]
fn removed_target_bumps_once() {
    assert_change_bumps_once(
        "removed",
        |_, _| {},
        |base| {
            let mut next = base.clone();
            next.targets.remove(2);
            next
        },
    );
}

#[test]
fn added_target_bumps_once() {
    assert_change_bumps_once(
        "added",
        |_, _| {},
        |base| {
            let mut next = base.clone();
            next.targets.push(target("p4"));
            next
        },
    );
}

#[test]
fn replaced_target_with_equal_count_bumps_once() {
    assert_change_bumps_once(
        "replaced",
        |_, _| {},
        |base| {
            let mut next = base.clone();
            next.targets[2] = target("p9");
            next
        },
    );
}

#[test]
fn new_host_epoch_bumps_once() {
    assert_change_bumps_once(
        "epoch",
        |_, _| {},
        |base| Capture {
            epoch: 2,
            ..base.clone()
        },
    );
}

#[test]
fn new_host_boot_bumps_once() {
    assert_change_bumps_once(
        "boot",
        |_, _| {},
        |base| Capture {
            boot: "b2",
            ..base.clone()
        },
    );
}

#[test]
fn incarnation_change_bumps_once() {
    assert_change_bumps_once(
        "incarnation",
        |_, _| {},
        |base| Capture {
            incarnation: "other-incarnation",
            ..base.clone()
        },
    );
}

#[test]
fn baseline_hold_change_bumps_once() {
    assert_change_bumps_once(
        "hold",
        |_, conn| {
            // An unresolved saved seat makes the next publication hold the
            // unclaimed baseline targets (hold 0 -> 1).
            conn.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('u','i','unresolved','native',0,0)", []).unwrap();
        },
        Capture::clone,
    );
}

#[test]
fn recovery_after_invalidation_bumps_once() {
    assert_change_bumps_once(
        "invalidation",
        |context, conn| {
            let budget = budget();
            let admission = seats::begin_host_observation(context, conn, "i", &budget).unwrap();
            assert!(
                seats::invalidate_host_observation(
                    context,
                    conn,
                    &admission,
                    HostInvalidationReason::CoherenceLost,
                    &budget,
                )
                .unwrap()
                .is_some()
            );
        },
        Capture::clone,
    );
}

#[test]
fn publication_after_a_current_target_read_bumps_once() {
    assert_change_bumps_once(
        "fresh-read",
        |context, conn| {
            // A fresh current-target read newer than the active publication
            // decides the target until a later publication supersedes it.
            let budget = budget();
            let admission = seats::begin_host_observation(context, conn, "i", &budget).unwrap();
            let mut observation = host_snapshot(&capture(&["p2"]), 2).targets.remove(0);
            observation.provenance = ObservationProvenance::FreshCurrentTarget;
            observation.generation = 7;
            observation.observation_sequence = 3;
            assert!(
                seats::publish_current_target_observation(
                    context,
                    conn,
                    &admission,
                    &observation,
                    &budget,
                )
                .unwrap()
            );
        },
        Capture::clone,
    );
}

#[test]
fn publication_after_a_read_of_an_unpublished_target_bumps_once() {
    assert_change_bumps_once(
        "fresh-read-absent",
        |context, conn| {
            // The read target is in neither publication: only the read's
            // lifecycle transition shows that this publication supersedes it.
            let budget = budget();
            let admission = seats::begin_host_observation(context, conn, "i", &budget).unwrap();
            let mut observation = host_snapshot(&capture(&["p7"]), 2).targets.remove(0);
            observation.provenance = ObservationProvenance::FreshCurrentTarget;
            observation.observation_sequence = 3;
            assert!(
                seats::publish_current_target_observation(
                    context,
                    conn,
                    &admission,
                    &observation,
                    &budget,
                )
                .unwrap()
            );
        },
        Capture::clone,
    );
}

// Kills: trusting an absent comparison. A stage whose match evidence this
// context never recorded (a restart, or another process staging) bumps.
#[test]
fn publication_without_staging_evidence_bumps() {
    let iso = TestIsolation::new("snapshot-eligibility-no-evidence");
    let (context, mut conn) = host_store(&iso);
    let base = capture(&["p1", "p2"]);
    publish(&context, &mut conn, &base, 1);
    publish(&context, &mut conn, &base, 2);
    assert_eq!(revision(&conn), 1);
    let other = StoreContext::new(iso.path("store.db"), Arc::new(FixedClock));
    publish_from(&context, &other, &mut conn, &base, 3);
    assert_eq!(revision(&conn), 2, "unproven content bumps");
    publish(&context, &mut conn, &base, 4);
    assert_eq!(revision(&conn), 2);
}

// --- send preparation across an unchanged publication ---------------------

/// The host store plus a joined cooperative sender `a` on target `pa` and a
/// joined, registered recipient `r` on target `pr` in thread `t`.
fn send_store(iso: &TestIsolation) -> (StoreContext, Connection) {
    let (context, conn) = host_store(iso);
    for (seat, target, execution) in [
        ("a", "pa", "00000000-0000-4000-8000-0000000000aa"),
        ("r", "pr", "00000000-0000-4000-8000-0000000000bb"),
    ] {
        conn.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,unavailability_episode) VALUES (?1,'i','resolved','native',?2,1,1,0,1)", [seat, target]).unwrap();
        conn.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation,registered_at) VALUES (?1,1,?2,'b',1,1,'codex','n',?3,'cooperative_top_level',0,'term-'||?2,'inc',0)", [seat, target, execution]).unwrap();
    }
    conn.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0)", []).unwrap();
    for seat in ["a", "r"] {
        conn.execute(
            "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t',?1,'joined')",
            [seat],
        )
        .unwrap();
        conn.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t',?1,1,1)", [seat]).unwrap();
    }
    (context, conn)
}

fn send_request() -> SendMessage {
    SendMessage {
        delivery_mode: crate::protocol::commands::DeliveryMode::Ordinary,
        thread: ThreadId::new("t"),
        body: "hello".into(),
        direct_recipient: None,
        invited_recipients: vec![],
        deadline_millis: None,
        operation: OperationId::new("op"),
        claim: CallerClaim {
            instance: "i".into(),
            seat: SeatId::new("a"),
            binding_generation: 1,
            role: CallerRole::TopLevel,
            harness: Harness::Codex,
            native_session: NativeSessionId::new("n"),
            execution: ExecutionId::new("00000000-0000-4000-8000-0000000000aa"),
            target: HostTargetId::new("pa"),
            agent_id: None,
            liveness: Some(crate::protocol::authority::LivenessFingerprint::TEST),
        },
        relays_user: false,
        user_intent: None,
    }
}

fn permit(request: &SendMessage) -> MutationPermit {
    let digest = crate::store::schema::canonical_digest(&messages::send_payload(request)).unwrap();
    MutationPermit::cooperative(
        request.claim.clone(),
        request.operation.clone(),
        ObligationRef::Control(request.thread.clone()),
        digest,
        MonoInstant(50),
        (1, 0),
        CallBudget {
            deadline: MonoInstant(u64::MAX),
            cancellation: Default::default(),
        },
    )
}

fn prepare(
    context: &StoreContext,
    conn: &mut Connection,
    request: &SendMessage,
    max_units: u8,
) -> Result<SendPreparationProgress, crate::protocol::results::ApiError> {
    messages::prepare_send_step(
        context,
        conn,
        request,
        messages::MessageLimits::default(),
        &budget(),
        DurableWorkAdmission { max_units },
    )
}

fn decide(
    context: &StoreContext,
    conn: &mut Connection,
    request: &SendMessage,
) -> Result<CommandResult, crate::protocol::results::ApiError> {
    messages::publish_send(
        context,
        conn,
        request,
        &mut permit(request),
        &budget(),
        || messages::MAX_BODY_BYTES,
    )
}

// Kills: a ready send refused at its decision because an unchanged
// observation-lane publication landed between preparation and decision (the
// hosted mod_delivery failure). A publication that does change a recipient's
// eligibility still refuses the stale preparation.
#[test]
fn ready_send_survives_an_unchanged_publication_but_not_a_changed_one() {
    let iso = TestIsolation::new("snapshot-eligibility-send");
    let (context, mut conn) = send_store(&iso);
    let base = capture(&["pa", "pr"]);
    publish(&context, &mut conn, &base, 1);
    let request = send_request();
    assert!(matches!(
        prepare(&context, &mut conn, &request, 16).unwrap(),
        SendPreparationProgress::Ready { .. }
    ));
    assert!(
        conn.query_row(
            "SELECT eligible_at_snapshot FROM prepared_recipients WHERE seat_id='r'",
            [],
            |r| r.get::<_, bool>(0)
        )
        .unwrap(),
        "the recipient is available at preparation"
    );
    publish(&context, &mut conn, &base, 2);
    let sent = decide(&context, &mut conn, &request).unwrap();
    assert!(matches!(sent, CommandResult::MessageSent(_)), "{sent:?}");

    let mut changed_request = send_request();
    changed_request.operation = OperationId::new("op-2");
    changed_request.body = "second".into();
    assert!(matches!(
        prepare(&context, &mut conn, &changed_request, 16).unwrap(),
        SendPreparationProgress::Ready { .. }
    ));
    // The recipient's pane is replaced: its structural generation changes,
    // so the staged availability is stale.
    let mut changed = base.clone();
    changed.targets[1].generation = 2;
    publish(&context, &mut conn, &changed, 3);
    let error = decide(&context, &mut conn, &changed_request).unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict, "{}", error.detail);
    assert!(
        error
            .detail
            .starts_with("send preparation snapshot changed"),
        "{}",
        error.detail
    );
}

// Kills: a preparation that spans several writer turns restarted by an
// unchanged publication in between (a send needing longer than the 5 s
// cadence could never complete on a stable host).
#[test]
fn multi_step_preparation_completes_across_unchanged_publications() {
    let iso = TestIsolation::new("snapshot-eligibility-multistep");
    let (context, mut conn) = send_store(&iso);
    for seat in ["x", "y"] {
        conn.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES (?1,'i','unresolved','native',0,0)", [seat]).unwrap();
        conn.execute(
            "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t',?1,'joined')",
            [seat],
        )
        .unwrap();
        conn.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t',?1,1,1)", [seat]).unwrap();
    }
    let base = capture(&["pa", "pr"]);
    publish(&context, &mut conn, &base, 1);
    let stable = revision(&conn);
    let request = send_request();
    let mut sequence = 1;
    let mut steps = 0;
    loop {
        steps += 1;
        assert!(steps < 10, "preparation does not converge");
        match prepare(&context, &mut conn, &request, 1).unwrap() {
            SendPreparationProgress::Ready { .. } => break,
            SendPreparationProgress::More { .. } => {
                sequence += 1;
                publish(&context, &mut conn, &base, sequence);
            }
            other => panic!("unexpected preparation progress {other:?}"),
        }
    }
    assert!(steps > 2, "the preparation spans several turns");
    assert_eq!(revision(&conn), stable);
    let sent = decide(&context, &mut conn, &request).unwrap();
    assert!(matches!(sent, CommandResult::MessageSent(_)), "{sent:?}");
}
