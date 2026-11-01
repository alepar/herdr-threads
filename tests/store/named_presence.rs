//! Named presence (N3-presence, SA17): the named host never holds, a named seat goes offline only
//! when its bindings end, and its live bindings stay available across a daemon restart. The named
//! snapshot is produced by the real `NamedHost` and the reconcile stamp, then published and
//! reconciled through the production store paths.
use super::*;
use crate::{
    host::named::NamedHost,
    identity::reconcile::{plan_page_for, stamp_fixed_structure},
    ports::{
        ClosureAuthority, DurableWorkAdmission, GuardedSeatTransition, HostCallContext, HostKind,
        HostPort, PublishedSnapshot, ReconciliationAction, SendPreparationProgress, SnapshotHeader,
    },
    protocol::{
        authority::{CallerClaim, CallerRole, Harness, MutationPermit, ObligationRef},
        commands::{DeliveryMode, ResolveSeat, SendMessage},
        ids::{
            ExecutionId, HostLocator, HostTargetId, NAMED_HOST_LOCATOR, NativeSessionId,
            OperationId, SeatId, SeatName, TerminalId, ThreadId,
        },
        priority::DeliveryPriority,
        results::CommandResult,
        time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis},
    },
    store::{
        bindings::{
            DisconnectReason, LivenessDecision, SeatAvailability, apply_liveness,
            liveness_candidates, seat_availability,
        },
        messages,
        named_seats::NamedSeatSource,
    },
};
use rusqlite::Connection;
use std::sync::{Arc, Mutex};

const INSTANCE: &str = "34750f98bd59fcfc946da45aaabe933b";
const NODE_UUID: &str = "node-1";
const BOOT: &str = "named-host:node=node-1";

struct FixedClock;
impl Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(100)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(100)
    }
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(10_000),
        cancellation: Cancellation::default(),
    }
}

/// The names the host enumerates: refreshed from the store before each capture, as the daemon's
/// store read does.
#[derive(Default)]
struct Names(Mutex<Vec<(SeatId, SeatName)>>);
impl NamedSeatSource for Names {
    fn live_named(
        &self,
        _: &HostId,
        _: &CallBudget,
    ) -> Result<Vec<(SeatId, SeatName)>, crate::protocol::results::ApiError> {
        Ok(self.0.lock().unwrap().clone())
    }
}

struct Fx {
    context: StoreContext,
    conn: Connection,
    host: HostId,
    names: Arc<Names>,
    named: NamedHost,
}

fn named_host(host: HostId, names: &Arc<Names>, persisted_epoch: u64) -> NamedHost {
    let port = NamedHost::new(NODE_UUID, host, names.clone(), Arc::new(FixedClock));
    port.resume_after_epoch(persisted_epoch);
    port
}

/// A store with the pane test host (the sender's) and the named host attached.
fn fixture() -> Fx {
    let path = crate::test_support::scratch::temp_path(format!(
        "herdr-named-presence-{}.db",
        uuid::Uuid::new_v4()
    ));
    let context = StoreContext::new(path, Arc::new(FixedClock)).with_test_host();
    let mut conn = context.open_writer().unwrap();
    let locator = HostLocator::parse(NAMED_HOST_LOCATOR).unwrap();
    let (host, _) =
        crate::store::hosts::attach_host(&mut conn, &locator, 0, HostKind::Named).unwrap();
    conn.execute_batch(
        "UPDATE herdr_hosts SET host_boot='b',host_epoch=1 WHERE kind<>'named'; UPDATE local_node SET decision_seq=1",
    )
    .unwrap();
    let names = Arc::new(Names::default());
    let named = named_host(host, &names, 0);
    Fx {
        context,
        conn,
        host,
        names,
        named,
    }
}

impl Fx {
    /// A named seat by a seat-acting resolve, with a registered, connected binding at the host's
    /// current epoch (the shape a cooperative registration leaves).
    fn seat(&mut self, name: &str) -> SeatId {
        let request = ResolveSeat {
            target: HostTargetId::new(name),
            host: HostLocator::parse(NAMED_HOST_LOCATOR).unwrap(),
            operation: OperationId::new(format!("resolve-{name}")),
            locator_hint: None,
            tmux_hint: None,
        };
        let CommandResult::NamedSeatResolved { seat, .. } = resolve_named_seat(
            &self.context,
            &mut self.conn,
            INSTANCE,
            self.host,
            &request,
            &budget(),
        )
        .unwrap() else {
            panic!("not a named resolve")
        };
        seat
    }

    fn bind(&self, seat: &SeatId, name: &str, epoch: i64) {
        self.conn.execute(
            "INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES (?1,1,?2,?3,?4,1,'codex',?2,?5,'cooperative_top_level',0,0,?6,?3)",
            rusqlite::params![seat.as_str(), name, BOOT, epoch, format!("00000000-0000-4000-8000-0000000000{:02x}", name.as_bytes()[0]), format!("named:{name}")],
        ).unwrap();
    }

    /// One named observation-lane publication: the host's capture, the reconcile stamp, the
    /// production stage and publish.
    fn publish(&mut self) -> PublishedSnapshot {
        *self.names.0.lock().unwrap() =
            crate::store::named_seats::live_named(&self.conn, &self.host).unwrap();
        let context = HostCallContext {
            budget: budget(),
            expected_boot: None,
            expected_epoch: None,
        };
        let snapshot = stamp_fixed_structure(self.named.enumerate_targets(&context).unwrap());
        let admission = begin_host_observation(
            &self.context,
            &mut self.conn,
            INSTANCE,
            self.host,
            &budget(),
        )
        .unwrap();
        let header = SnapshotHeader::from_captured(admission, &snapshot).unwrap();
        let stage = begin_snapshot_stage(&self.context, &mut self.conn, header, &budget()).unwrap();
        crate::test_support::snapshot::stage_all(&snapshot.targets, |offset, targets| {
            stage_snapshot_targets(
                &self.context,
                &mut self.conn,
                &stage.id,
                offset,
                targets,
                DurableWorkAdmission::new(16).unwrap(),
                &budget(),
            )
        })
        .unwrap();
        seal_snapshot_stage(&self.context, &mut self.conn, &stage.id, &budget()).unwrap();
        publish_snapshot_stage(&self.context, &mut self.conn, &stage.id, &budget()).unwrap()
    }

    /// The saved-seat pass of `published` as the lane runs it for a non-closing host; returns the
    /// number of transitions it planned.
    fn reconcile(&mut self, published: &PublishedSnapshot) -> usize {
        let page = saved_seats_page(
            &self.context,
            &self.conn,
            &published.id,
            0,
            None,
            16,
            &budget(),
        )
        .unwrap();
        let plan = plan_page_for(&page, ClosureAuthority::NonClosing).unwrap();
        for transition in &plan {
            apply_reconciliation_transition(
                &self.context,
                &mut self.conn,
                transition.clone(),
                &budget(),
            )
            .unwrap();
        }
        assert!(
            record_reconciliation_pass(&self.context, &mut self.conn, published, &budget())
                .unwrap()
        );
        plan.len()
    }

    /// The plan the way the lane applies it, but for an unresolve of `seat` (what an operator or
    /// a restored row leaves): through the production transition.
    fn unresolve(&mut self, published: &PublishedSnapshot, seat: &SeatId, name: &str) {
        apply_reconciliation_transition(
            &self.context,
            &mut self.conn,
            GuardedSeatTransition {
                publication: published.clone(),
                seat: seat.clone(),
                expected_binding_generation: 1,
                expected_target: Some(HostTargetId::new(name)),
                expected_terminal: Some(TerminalId::new(format!("named:{name}"))),
                action: ReconciliationAction::MarkUnresolved,
            },
            &budget(),
        )
        .unwrap();
    }

    /// The liveness worker's verdict for a seat whose process is gone.
    fn process_exit(&self, seat: &SeatId, at: i64) -> usize {
        let decisions: Vec<LivenessDecision> = liveness_candidates(&self.conn, None)
            .unwrap()
            .into_iter()
            .filter(|c| &c.binding.seat == seat)
            .map(|c| LivenessDecision {
                binding: c.binding.binding_id,
                fingerprint: c.binding.fingerprint,
                reason: DisconnectReason::ProcessExit,
            })
            .collect();
        apply_liveness(&self.conn, &decisions, UtcMillis(at)).unwrap()
    }

    fn candidate_seats(&self) -> Vec<String> {
        let mut seats: Vec<String> = liveness_candidates(&self.conn, None)
            .unwrap()
            .into_iter()
            .map(|c| c.binding.seat.as_str().to_owned())
            .collect();
        seats.sort();
        seats
    }

    fn available(&self, seat: &SeatId) -> bool {
        crate::store::schema::effective_registered_availability(
            &self.conn,
            seat.as_str(),
            Some(INSTANCE),
        )
        .unwrap()
        .is_some()
    }

    fn scalar(&self, sql: &str) -> i64 {
        self.conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    fn holds(&self) -> (i64, i64) {
        (
            self.scalar("SELECT baseline_hold_unclaimed FROM herdr_hosts WHERE kind='named'"),
            self.scalar("SELECT count(*) FROM recovery_holds WHERE released_at IS NULL"),
        )
    }
}

// Kills: an unresolved named row that arms `baseline_hold_unclaimed` (or a recovery hold) for the
// whole named host, which silences the liveness of every other named binding. The control flips
// the same host to a pane kind and the same transition arms the hold, so the assertion has teeth.
#[test]
fn unresolved_named_row_does_not_silence_others() {
    let mut f = fixture();
    let alpha = f.seat("alpha");
    let beta = f.seat("beta");
    let gamma = f.seat("gamma");
    f.bind(&alpha, "alpha", 1);
    f.bind(&beta, "beta", 1);
    let published = f.publish();
    f.reconcile(&published);
    f.unresolve(&published, &gamma, "gamma");
    assert_eq!(
        f.scalar("SELECT count(*) FROM seats WHERE name='gamma' AND state='unresolved'"),
        1
    );
    assert_eq!(f.holds(), (0, 0), "the named host holds nothing");
    // Seat ids are random, so `candidate_seats()` (sorted) is compared with a sorted expectation.
    let mut expected = vec![alpha.as_str().to_owned(), beta.as_str().to_owned()];
    expected.sort();
    assert_eq!(
        f.candidate_seats(),
        expected,
        "the live bindings stay judged by liveness"
    );

    let mut control = fixture();
    let a = control.seat("alpha");
    let g = control.seat("gamma");
    control.bind(&a, "alpha", 1);
    let published = control.publish();
    control.reconcile(&published);
    control
        .conn
        .execute(
            "UPDATE herdr_hosts SET kind='herdr' WHERE host_id=?1",
            [control.host.get()],
        )
        .unwrap();
    control.unresolve(&published, &g, "gamma");
    assert_eq!(
        control.scalar("SELECT baseline_hold_unclaimed FROM herdr_hosts WHERE host_id=(SELECT host_id FROM seats WHERE name='gamma')"),
        1,
        "a pane host arms the hold for the same transition"
    );
    assert!(
        control.candidate_seats().is_empty(),
        "and its live binding is silenced"
    );
}

// S2. Kills: a first named publication over live seats that arms `baseline_hold_unclaimed`, or
// leaves a hold a former state armed. Control: a pane host in the same state keeps its hold.
#[test]
fn s2_first_empty_named_publication_clears_holds() {
    let arm = |f: &mut Fx| {
        f.seat("alpha");
        f.conn
            .execute_batch(
                "UPDATE seats SET state='unresolved',unresolved_reason='other' WHERE name='alpha'",
            )
            .unwrap();
        f.conn
            .execute(
                "UPDATE herdr_hosts SET baseline_hold_unclaimed=1 WHERE host_id=?1",
                [f.host.get()],
            )
            .unwrap();
        f.conn
            .execute(
                "INSERT INTO recovery_holds(host_id,target_id,baseline_boot,baseline_epoch,reason) VALUES (?1,'alpha',?2,1,'former')",
                rusqlite::params![f.host.get(), BOOT],
            )
            .unwrap();
        // Nothing is enumerated: the one live row is unresolved and shows no target.
        f.conn
            .execute_batch("UPDATE seats SET name=NULL WHERE name='alpha'")
            .unwrap();
    };
    let mut f = fixture();
    arm(&mut f);
    let published = f.publish();
    assert_eq!(published.target_count, 0, "the first publication is empty");
    assert_eq!(f.holds(), (0, 0), "holds are cleared, none armed");

    let mut control = fixture();
    arm(&mut control);
    control
        .conn
        .execute(
            "UPDATE herdr_hosts SET kind='herdr' WHERE host_id=?1",
            [control.host.get()],
        )
        .unwrap();
    control.publish();
    assert_eq!(
        control.scalar("SELECT baseline_hold_unclaimed FROM herdr_hosts WHERE host_id=(SELECT host_id FROM seats WHERE state='unresolved')"),
        1,
        "a pane host over an unresolved row holds"
    );
}

// Kills: a seat whose bindings ended that stays "active" or "available" (no offline word, no
// unavailable-since time), or one that goes offline while a binding is live.
#[test]
fn ended_bindings_show_inactive_unavailable_since() {
    let mut f = fixture();
    let alpha = f.seat("alpha");
    let beta = f.seat("beta");
    f.bind(&alpha, "alpha", 1);
    f.bind(&beta, "beta", 1);
    let published = f.publish();
    f.reconcile(&published);
    assert!(f.available(&alpha) && f.available(&beta));
    let live = crate::store::local_seat_status::local_seat(&f.conn, alpha.as_str())
        .unwrap()
        .unwrap();
    assert_eq!(crate::store::local_seat_status::presence(&live), "idle");

    assert_eq!(f.process_exit(&alpha, 700), 1);
    let ended = crate::store::local_seat_status::local_seat(&f.conn, alpha.as_str())
        .unwrap()
        .unwrap();
    assert!(!ended.connected);
    assert_eq!(
        crate::store::local_seat_status::presence(&ended),
        "offline",
        "peers see the ended named seat offline"
    );
    assert_eq!(
        seat_availability(&f.conn, &alpha).unwrap(),
        SeatAvailability::Unavailable {
            since: UtcMillis(700)
        }
    );
    assert!(f.available(&beta), "the other seat stays available");
    assert_eq!(
        seat_availability(&f.conn, &beta).unwrap(),
        SeatAvailability::Available
    );
}

// Kills: a daemon restart (the named epoch moves past the persisted one, the boot is fixed) that
// leaves live named bindings on the old epoch, unavailable until they re-register. Without the
// stamped structural proof the planner plans no carry and the assertion after the pass fails.
#[test]
fn restart_keeps_live_named_bindings_available() {
    let mut f = fixture();
    let alpha = f.seat("alpha");
    let beta = f.seat("beta");
    f.bind(&alpha, "alpha", 1);
    f.bind(&beta, "beta", 1);
    let first = f.publish();
    assert_eq!(first.epoch, 1);
    f.reconcile(&first);
    assert!(f.available(&alpha) && f.available(&beta));

    // The daemon restarts: a new NamedHost over the persisted epoch, the same fixed boot.
    f.named = named_host(f.host, &f.names, 1);
    let second = f.publish();
    assert_eq!(second.epoch, 2, "the restart moves the host epoch");
    assert_eq!(second.incarnation, BOOT);
    assert!(
        !f.available(&alpha),
        "before the pass the bindings lag the epoch"
    );
    assert_eq!(f.reconcile(&second), 2, "one carry per live named seat");
    assert!(f.available(&alpha) && f.available(&beta));
    assert_eq!(
        f.scalar("SELECT count(*) FROM occupant_bindings WHERE host_epoch=2 AND ended_at IS NULL"),
        2
    );
    assert_eq!(
        f.scalar("SELECT count(*) FROM seats WHERE state<>'resolved' AND name IS NOT NULL"),
        0
    );
}

// ---- sends ----

fn sender(f: &Fx) {
    f.conn.execute_batch(
        "INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,journaled) VALUES ('t','34750f98bd59fcfc946da45aaabe933b','topic','goal',0,0,0);
         INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,unavailability_episode,home_node,is_local,host_id) VALUES ('a','34750f98bd59fcfc946da45aaabe933b','resolved','native','pa',1,1,0,1,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts));
         INSERT INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ((SELECT min(host_id) FROM herdr_hosts),'pa','b',1,1,0,'fresh','term-pa','inc','coherent_enumeration',1);
         INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES ('a',1,'pa','b',1,1,'codex','n','00000000-0000-4000-8000-0000000000aa','cooperative_top_level',0,0,'term-pa','inc');
         INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t','a','joined');
         INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t','a',1,1);",
    ).unwrap();
}

fn join(f: &Fx, seat: &SeatId) {
    f.conn
        .execute(
            "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t',?1,'joined')",
            [seat.as_str()],
        )
        .unwrap();
    f.conn
        .execute(
            "INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t',?1,1,1)",
            [seat.as_str()],
        )
        .unwrap();
}

fn request(operation: &str, priority: Option<u8>, ack: &[&SeatId]) -> SendMessage {
    SendMessage {
        priority: priority.map(|p| DeliveryPriority::new(p).unwrap()),
        delivery_mode: DeliveryMode::Ordinary,
        thread: ThreadId::new("t"),
        body: "hello".into(),
        direct_recipient: None,
        invited_recipients: ack.iter().map(|s| (*s).clone()).collect(),
        deadline_millis: None,
        operation: OperationId::new(operation),
        claim: CallerClaim {
            host: crate::test_support::test_host_locator(),
            instance: INSTANCE.into(),
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

fn send(f: &mut Fx, request: &SendMessage) {
    loop {
        let step = messages::prepare_send_step(
            &f.context,
            &mut f.conn,
            request,
            messages::MessageLimits::default(),
            &budget(),
            DurableWorkAdmission::new(16).unwrap(),
        )
        .unwrap();
        if matches!(step, SendPreparationProgress::Ready { .. }) {
            break;
        }
    }
    let digest = crate::store::schema::canonical_digest(&messages::send_payload(request)).unwrap();
    let mut permit = MutationPermit::cooperative(
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
    );
    messages::publish_send(
        &f.context,
        &mut f.conn,
        request,
        &mut permit,
        &budget(),
        || messages::MAX_BODY_BYTES,
    )
    .unwrap();
}

/// `(recipient, episode)` of every `recipient_unavailable` warning prepared so far.
fn warnings(f: &Fx) -> Vec<(String, i64)> {
    let mut stmt = f
        .conn
        .prepare("SELECT affected_seat_id,unavailability_episode FROM prepared_unavailable_warnings ORDER BY warning_offset,preparation_id")
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

// Kills: no warning for an offline named recipient, or one per send instead of one per episode,
// for an ACK send and for a p0 send alike.
#[test]
fn one_recipient_unavailable_per_episode() {
    let mut f = fixture();
    sender(&f);
    let alpha = f.seat("alpha");
    f.bind(&alpha, "alpha", 1);
    join(&f, &alpha);
    let published = f.publish();
    f.reconcile(&published);
    send(&mut f, &request("op-live", None, &[&alpha]));
    assert!(
        warnings(&f).is_empty(),
        "a live named recipient is not warned about"
    );

    assert_eq!(f.process_exit(&alpha, 700), 1);
    send(&mut f, &request("op-ack", None, &[&alpha]));
    let first = warnings(&f);
    assert_eq!(first.len(), 1, "the ACK send warns once: {first:?}");
    assert_eq!(first[0].0, alpha.as_str());
    send(&mut f, &request("op-p0", Some(0), &[]));
    send(&mut f, &request("op-ack-2", None, &[&alpha]));
    assert_eq!(
        warnings(&f),
        first,
        "later ACK and p0 sends of the same episode add none"
    );
}

// Kills: one unresolved named row that keeps another named seat from going offline when its
// bindings end (and so from showing `recipient_unavailable`).
#[test]
fn other_named_seat_goes_offline_with_one_unresolved() {
    let mut f = fixture();
    sender(&f);
    let beta = f.seat("beta");
    let gamma = f.seat("gamma");
    f.bind(&beta, "beta", 1);
    join(&f, &beta);
    let published = f.publish();
    f.reconcile(&published);
    f.unresolve(&published, &gamma, "gamma");
    assert_eq!(f.holds(), (0, 0));

    assert_eq!(
        f.process_exit(&beta, 900),
        1,
        "the unresolved row does not silence the liveness of beta's binding"
    );
    assert!(!f.available(&beta));
    send(&mut f, &request("op-p0", Some(0), &[]));
    assert_eq!(
        warnings(&f)
            .iter()
            .map(|w| w.0.as_str())
            .collect::<Vec<_>>(),
        vec![beta.as_str()],
        "beta shows recipient_unavailable"
    );
}
