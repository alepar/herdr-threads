//! First named enrollment must not depend on when the observation worker next runs.
use crate::{
    app::SystemClock,
    host::named::NamedHost,
    identity::OrdinaryIdentity,
    ports::{HostId, HostKind, LocalService},
    protocol::{
        authority::{CallerClaim, CallerRole, Harness, LivenessFingerprint, PeerIdentity},
        commands::{CheckIn, CheckInMode, Command, OperatorRetire, ResolveSeat},
        ids::*,
        results::{ApiError, CommandResult, ErrorCode},
        time::{CallBudget, Cancellation, Clock, MonoInstant},
    },
    service::{
        dispatch::DomainService,
        fair_writer::FairWriter,
        host_sessions::{HostSession, HostSessions},
        pacer::Pacer,
        workers::ScheduledStore,
    },
    store::{SqliteStore, StoreSettings, connection::StoreContext, hosts::HostIndex},
    test_support::isolation::TestIsolation,
};
use std::{sync::Arc, time::Duration};

struct Rig {
    service: DomainService,
    identity: Arc<OrdinaryIdentity>,
    store: Arc<SqliteStore>,
    writer: Arc<FairWriter>,
    clock: Arc<dyn Clock>,
    node: String,
    host: HostId,
    isolation: TestIsolation,
}
impl Rig {
    fn new() -> Self {
        let isolation = TestIsolation::new("named-startup");
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let node = uuid::Uuid::new_v4().simple().to_string();
        let index = Arc::new(HostIndex::new());
        let store = Arc::new(
            SqliteStore::new(
                StoreContext::with_host_index(
                    isolation.path("store.db"),
                    clock.clone(),
                    index.clone(),
                )
                .with_node_seed(crate::test_support::node_seed_for(&node)),
                node.clone(),
                StoreSettings::default(),
            )
            .unwrap(),
        );
        let locator = HostLocator::parse(NAMED_HOST_LOCATOR).unwrap();
        let (host, _) = crate::store::hosts::attach_host(
            &mut store.context().open_writer().unwrap(),
            &locator,
            0,
            HostKind::Named,
        )
        .unwrap();
        index.publish_for_test(locator.clone(), host);
        let writer = Arc::new(FairWriter::new(32));
        let pacer = Arc::new(Pacer::new(
            "observation",
            clock.clone(),
            Cancellation::default(),
        ));
        let named = Arc::new(NamedHost::new(&node, host, store.clone(), clock.clone()));
        let identity = Arc::new(
            OrdinaryIdentity::new(
                node.clone(),
                host,
                store.clone(),
                named,
                clock.clone(),
                writer.clone(),
            )
            .with_observation_pacer(pacer),
        );
        // Deliberately gate background scheduling: the real lane can only capture when this
        // fixture releases it through capture(). A kick alone cannot publish anything.
        let session = HostSession::prebuilt(locator, identity.clone(), clock.clone(), None);
        let sessions = HostSessions::prebuilt(session, store.clone(), clock.clone(), node.clone());
        let service = DomainService::new(node.clone(), store.clone(), clock.clone())
            .with_sessions(sessions)
            .with_operator_owner(501)
            .with_cooperative_owner(501, writer.clone());
        Self {
            service,
            identity,
            store,
            writer,
            clock,
            node,
            host,
            isolation,
        }
    }
    fn budget(&self, millis: u64) -> CallBudget {
        CallBudget {
            deadline: MonoInstant(self.clock.monotonic_now().0 + millis),
            cancellation: Cancellation::default(),
        }
    }
    fn call(&self, command: Command, millis: u64) -> Result<CommandResult, ApiError> {
        self.service.handle(
            command,
            PeerIdentity::from_kernel(501),
            &self.budget(millis),
        )
    }
    fn request(&self) -> ResolveSeat {
        ResolveSeat {
            target: HostTargetId::new("new-agent"),
            host: HostLocator::parse(NAMED_HOST_LOCATOR).unwrap(),
            operation: OperationId::new("resolve-new-agent"),
            locator_hint: None,
            tmux_hint: None,
            continuity_key: None,
        }
    }
    fn resolved(&self) -> SeatId {
        let CommandResult::NamedSeatResolved { seat, created, .. } = self
            .call(Command::ResolveSeat(self.request()), 1_000)
            .unwrap()
        else {
            panic!("named resolution")
        };
        assert!(
            created,
            "same operation retains its historical allocation result"
        );
        seat
    }
    fn capture(&self) {
        let scheduled = ScheduledStore::new(self.store.clone(), self.writer.clone());
        assert!(matches!(
            self.identity
                .capture_if_due(&scheduled, &self.budget(1_000), &self.budget(1_000))
                .unwrap(),
            Some(crate::identity::reconcile::ObservationOutcome::Published(_))
        ));
    }
    fn check_in(&self, seat: SeatId) -> Result<CommandResult, ApiError> {
        self.call(
            Command::CheckIn(CheckIn {
                readonly: false,
                lineage_parent: None,
                mode: CheckInMode::Lifecycle {
                    expected_binding_generation: 1,
                },
                claim: CallerClaim {
                    host: HostLocator::parse(NAMED_HOST_LOCATOR).unwrap(),
                    instance: self.node.clone(),
                    seat,
                    binding_generation: 1,
                    role: CallerRole::TopLevel,
                    harness: Harness::Codex,
                    native_session: NativeSessionId::new("new-session"),
                    execution: ExecutionId::new("00000000-0000-4000-8000-000000000001"),
                    target: self.request().target,
                    agent_id: None,
                    liveness: Some(LivenessFingerprint::TEST),
                },
                operation: OperationId::new("initial-check-in"),
                continuity_key: None,
            }),
            1_000,
        )
    }
    fn sql(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.isolation.path("store.db")).unwrap()
    }
}

#[test]
fn first_named_resolution_admits_immediate_lifecycle_while_background_capture_is_paused() {
    let rig = Rig::new();
    rig.capture(); // A real empty baseline, published before the new name exists.
    let seat = rig.resolved();
    // No observation, timer, retry or worker scheduling between resolve and check-in.
    let checked = rig.check_in(seat.clone());
    assert!(
        matches!(checked, Ok(CommandResult::CheckedIn(_))),
        "{checked:?}"
    );
    rig.capture(); // Releasing background capture must preserve the same allocation and binding.
    assert_eq!(rig.resolved(), seat);
    assert_eq!(
        rig.sql()
            .query_row("SELECT count(*) FROM seats", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        rig.sql()
            .query_row("SELECT count(*) FROM occupant_bindings", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn first_named_resolution_without_a_snapshot_is_bounded_and_replays_same_allocation_after_capture()
{
    let rig = Rig::new();
    let at = std::time::Instant::now();
    let result = rig.call(Command::ResolveSeat(rig.request()), 40);
    assert!(
        matches!(result, Err(ref error) if matches!(error.code, ErrorCode::DeadlineExceeded | ErrorCode::StaleHostObservation)),
        "{result:?}"
    );
    assert!(at.elapsed() < Duration::from_secs(1));
    let original: String = rig
        .sql()
        .query_row("SELECT id FROM seats", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        rig.sql()
            .query_row("SELECT count(*) FROM occupant_bindings", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    rig.capture();
    let replay = rig.resolved();
    assert_eq!(replay.as_str(), original);
    assert!(matches!(
        rig.check_in(replay),
        Ok(CommandResult::CheckedIn(_))
    ));
}

#[test]
fn replayed_named_allocation_does_not_admit_a_retired_owner_or_replace_its_successor() {
    let rig = Rig::new();
    rig.capture();
    let old = rig.resolved();
    rig.call(
        Command::OperatorRetire(OperatorRetire {
            seat: old.clone(),
            operation: OperationId::new("retire"),
        }),
        1_000,
    )
    .unwrap();
    let mut successor = rig.request();
    successor.operation = OperationId::new("successor");
    let CommandResult::NamedSeatResolved { seat: new, .. } =
        rig.call(Command::ResolveSeat(successor), 1_000).unwrap()
    else {
        panic!("successor")
    };
    assert_ne!(old, new);
    let replay = rig.call(Command::ResolveSeat(rig.request()), 1_000);
    assert!(
        matches!(replay, Err(ref error) if error.code == ErrorCode::TargetUnresolved),
        "{replay:?}"
    );
    assert_eq!(
        rig.sql()
            .query_row(
                "SELECT id FROM seats WHERE host_id=?1 AND state='resolved'",
                [rig.host.get()],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        new.as_str()
    );
}

// S2: existing named callers must not invalidate global audience/cursor fences simply by
// resolving their unchanged name. A historical allocation replay has the same requirement.
#[test]
fn unchanged_named_resolution_preserves_publication_and_eligibility_fences() {
    use crate::{
        ports::{DurableWorkAdmission, SendPreparationProgress},
        protocol::{
            authority::{MutationPermit, ObligationRef},
            commands::{
                CreateThread, DeliveryMode, InvitationConfirmation, Invite, ParticipantsQuery,
                SendMessage,
            },
            pagination::PageRequest,
        },
        store::messages,
    };
    let rig = Rig::new();
    rig.capture();
    let seat = rig.resolved();
    let CommandResult::CheckedIn(checked) = rig.check_in(seat.clone()).unwrap() else {
        panic!("check-in")
    };
    let claim = checked.context;
    let CommandResult::ThreadCreated(thread) = rig
        .call(
            Command::CreateThread(CreateThread {
                name: None,
                topic: "stable named audience".into(),
                goal: "stable pagination and send".into(),
                operation: OperationId::new("stable-thread"),
                claim: claim.clone(),
            }),
            1_000,
        )
        .unwrap()
    else {
        panic!("thread")
    };
    let mut other = rig.request();
    other.target = HostTargetId::new("other-agent");
    other.operation = OperationId::new("other-agent");
    let CommandResult::NamedSeatResolved { seat: other, .. } =
        rig.call(Command::ResolveSeat(other), 1_000).unwrap()
    else {
        panic!("other seat")
    };
    rig.call(
        Command::Invite(Invite {
            confirmation: Some(InvitationConfirmation::Lazy),
            thread: thread.clone(),
            seat: other.clone(),
            deadline_millis: None,
            operation: OperationId::new("invite-other"),
            claim: claim.clone(),
        }),
        1_000,
    )
    .unwrap();
    rig.capture();
    let mut participants = ParticipantsQuery {
        thread: thread.clone(),
        page: PageRequest {
            limit: 1,
            ..Default::default()
        },
        caller: None,
    };
    let CommandResult::Participants(first) = rig
        .call(Command::Participants(participants.clone()), 1_000)
        .unwrap()
    else {
        panic!("participants")
    };
    participants.page.cursor = Some(
        first
            .next_cursor
            .expect("two participants yield a continuation"),
    );
    let send = SendMessage {
        priority: None,
        delivery_mode: DeliveryMode::Ordinary,
        thread: thread.clone(),
        body: "sealed before unchanged resolve".into(),
        direct_recipient: None,
        invited_recipients: vec![other],
        deadline_millis: None,
        operation: OperationId::new("stable-send"),
        claim: claim.clone(),
        relays_user: false,
        user_intent: None,
    };
    let mut db = rig.store.context().open_writer().unwrap();
    loop {
        if matches!(
            messages::prepare_send_step(
                rig.store.context(),
                &mut db,
                &send,
                messages::MessageLimits::default(),
                &rig.budget(1_000),
                DurableWorkAdmission::new(16).unwrap()
            )
            .unwrap(),
            SendPreparationProgress::Ready { .. }
        ) {
            break;
        }
    }
    let fences = || {
        rig.sql()
            .query_row(
                "SELECT h.lifecycle_revision,n.send_eligibility_revision,h.observation_admission_sequence,h.observation_decided_sequence,h.observation_sequence FROM herdr_hosts h JOIN local_node n ON n.id=h.node_id WHERE h.host_id=?1",
                [rig.host.get()],
                |r| Ok((r.get::<_, i64>(0)?,r.get::<_, i64>(1)?,r.get::<_, i64>(2)?,r.get::<_, i64>(3)?,r.get::<_, i64>(4)?)),
            )
            .unwrap()
    };
    let before = fences();
    let mut request = rig.request();
    request.operation = OperationId::new("unchanged-new-operation");
    assert!(matches!(
        rig.call(Command::ResolveSeat(request), 1_000),
        Ok(CommandResult::NamedSeatResolved { seat: current, created: false, .. }) if current == seat
    ));
    assert_eq!(fences(), before, "an unchanged resolve published again");
    assert_eq!(rig.resolved(), seat);
    assert_eq!(fences(), before, "a historical replay published again");
    let resumed = rig.call(Command::Participants(participants), 1_000);
    assert!(
        matches!(resumed, Ok(CommandResult::Participants(_))),
        "cursor invalidated: {resumed:?}"
    );
    let digest = crate::store::schema::canonical_digest(&messages::send_payload(&send)).unwrap();
    let mut permit = MutationPermit::cooperative(
        claim,
        send.operation.clone(),
        ObligationRef::Control(thread),
        digest,
        rig.clock.monotonic_now(),
        (1, 0),
        rig.budget(1_000),
    );
    let sent = messages::publish_send(
        rig.store.context(),
        &mut db,
        &send,
        &mut permit,
        &rig.budget(1_000),
        || messages::MAX_BODY_BYTES,
    )
    .unwrap();
    assert!(matches!(sent, CommandResult::MessageSent(_)));
    let scheduled = ScheduledStore::new(rig.store.clone(), rig.writer.clone());
    assert!(
        rig.identity
            .capture_if_due(&scheduled, &rig.budget(1_000), &rig.budget(1_000))
            .unwrap()
            .is_none(),
        "unchanged resolve requested an explicit capture"
    );
}
