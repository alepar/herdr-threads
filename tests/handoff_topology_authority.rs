//! Public dispatch composition, with canonical SQLite decisions and local controls.
use herdr_threads::{
    app::SystemClock,
    ports::{LocalService, StorePort},
    protocol::{
        commands::Command,
        handoff::*,
        results::CommandResult,
        time::{CallBudget, Cancellation, Clock, MonoInstant},
    },
    service::{dispatch::DomainService, fair_writer::FairWriter},
    store::{SqliteStore, StoreSettings, connection::StoreContext},
};
use std::sync::Arc;

struct Fixture {
    service: DomainService,
    store: Arc<SqliteStore>,
    db: rusqlite::Connection,
    clock: Arc<dyn Clock>,
    directory: super::handoff_fences::Directory,
}
impl Fixture {
    fn new() -> Self {
        let directory = super::handoff_fences::Directory(
            std::env::temp_dir().join(format!("ht-authority-{}", uuid::Uuid::new_v4())),
        );
        std::fs::create_dir(&directory.0).unwrap();
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let context = StoreContext::new(directory.0.join("store.db"), clock.clone());
        let db = rusqlite::Connection::open(directory.0.join("store.db")).unwrap();
        herdr_threads::store::schema::initialize(&db, || clock.utc_now()).unwrap();
        db.execute_batch("INSERT INTO host_instances(id,created_at) VALUES('i',0)")
            .unwrap();
        super::channel_archival::thread(&db);
        super::channel_archival::joined_agent(&db);
        drop(db);
        let db = context.open_writer().unwrap();
        let concrete = Arc::new(SqliteStore::new(context, "i", StoreSettings::default()).unwrap());
        let store: Arc<dyn StorePort> = concrete.clone();
        let host = Arc::new(herdr_threads::host::native::NativeCli::new(
            "/host.sock".into(),
            clock.clone(),
        ));
        let writer = Arc::new(FairWriter::new(32));
        let identity = Arc::new(herdr_threads::identity::repair::OrdinaryIdentity::new(
            "i".into(),
            store.clone(),
            host.clone(),
            clock.clone(),
            writer.clone(),
        ));
        let service = DomainService::with_identity("i".into(), store, clock.clone(), identity)
            .with_operator_owner(501)
            .with_cooperative_owner(501, writer)
            .with_bootstrap_runtime(super::topology_handoff::namespace(), host, concrete.clone())
            .unwrap();
        Self {
            service,
            store: concrete,
            db,
            clock,
            directory,
        }
    }
    fn call(
        &self,
        command: Command,
    ) -> Result<CommandResult, herdr_threads::protocol::results::ApiError> {
        self.service.handle(
            command,
            herdr_threads::test_support::peer_identity(501),
            &CallBudget {
                deadline: MonoInstant(self.clock.monotonic_now().0 + 5_000),
                cancellation: Cancellation::default(),
            },
        )
    }
}

#[test]
fn public_begin_installs_canonical_fence_and_reserve_replay_never_reauthorizes() {
    let f = Fixture::new();
    let identity = super::topology_handoff::identity();
    let begin = Command::BeginBootstrap(Box::new(BeginBootstrap {
        operation: identity.payload.handoff.keys.begin.clone(),
        identity: identity.clone(),
    }));
    let result = f.call(begin.clone());
    assert!(
        matches!(result, Ok(CommandResult::Bootstrap(ref status)) if status.state == BootstrapState::Prepared),
        "actual public Begin must reach canonical deciding pipeline: {result:?}"
    );
    // Losing the first response leaves only canonical parent/key retention.
    assert_eq!(f.call(begin.clone()).unwrap(), result.unwrap());
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM operations WHERE actor_scope='seat:sender' AND operation_key=?1",
            [identity.payload.handoff.keys.begin.as_str()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let mut changed = identity.clone();
    changed.payload.handoff.body = "changed original".into();
    changed.digest = changed.semantic_digest().unwrap();
    assert!(
        f.call(Command::BeginBootstrap(Box::new(BeginBootstrap {
            operation: changed.payload.handoff.keys.begin.clone(),
            identity: changed
        })))
        .is_err()
    );
    let reserve = Command::ReserveBootstrapAttempt(Box::new(ReserveBootstrapAttempt {
        identity: identity.clone(),
        operation: BootstrapAttempt::first()
            .operation(&identity.compound, "reserve")
            .unwrap(),
        expected_attempt: BootstrapAttempt::first(),
    }));
    assert!(
        matches!(f.call(reserve.clone()).unwrap(), CommandResult::BootstrapReserved(ref result) if matches!(**result, ReserveBootstrapResult::Authorized { .. }))
    );
    assert!(
        matches!(f.call(reserve).unwrap(), CommandResult::BootstrapReserved(ref result) if matches!(**result, ReserveBootstrapResult::Replay { .. }))
    );
    assert_eq!(
        f.db.query_row("SELECT count(*) FROM bootstrap_attempts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(f.directory.0.is_dir());
    let before: i64 =
        f.db.query_row("SELECT count(*) FROM operations", [], |r| r.get(0))
            .unwrap();
    f.db.execute("UPDATE occupant_bindings SET ended_at=1", [])
        .unwrap();
    assert!(
        f.call(begin).is_err(),
        "live parent replay must recheck current caller"
    );
    assert_eq!(
        f.db.query_row("SELECT count(*) FROM operations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        before
    );
}

fn begin(identity: BootstrapIdentity) -> Command {
    Command::BeginBootstrap(Box::new(BeginBootstrap {
        operation: identity.payload.handoff.keys.begin.clone(),
        identity,
    }))
}
fn reserve(identity: BootstrapIdentity) -> Command {
    Command::ReserveBootstrapAttempt(Box::new(ReserveBootstrapAttempt {
        operation: BootstrapAttempt::first()
            .operation(&identity.compound, "reserve")
            .unwrap(),
        expected_attempt: BootstrapAttempt::first(),
        identity,
    }))
}
#[test]
fn public_reservation_loss_and_changed_caller_refuse_final_submission_check() {
    let f = Fixture::new();
    let identity = super::topology_handoff::identity();
    f.call(begin(identity.clone())).unwrap();
    // Discard the first response. The retry must not grant another submission.
    f.call(reserve(identity.clone())).unwrap();
    assert!(
        matches!(f.call(reserve(identity.clone())).unwrap(), CommandResult::BootstrapReserved(ref r) if matches!(**r, ReserveBootstrapResult::Replay { .. }))
    );
    let check = Command::CheckBootstrapSubmission(Box::new(CheckBootstrapSubmission {
        identity: identity.clone(),
        operation: BootstrapAttempt::first()
            .operation(&identity.compound, "check")
            .unwrap(),
        expected_attempt: BootstrapAttempt::first(),
        expected_administrative_revision: 0,
    }));
    assert!(matches!(
        f.call(check.clone()).unwrap(),
        CommandResult::BootstrapSubmissionChecked(_)
    ));
    f.db.execute("UPDATE seats SET generation=generation+1 WHERE id='s'", [])
        .unwrap();
    assert!(
        f.call(check).is_err(),
        "a retained check result cannot bypass current A2"
    );
    assert_eq!(
        f.db.query_row("SELECT count(*) FROM messages", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn public_begin_refuses_copied_namespace_and_subagent_before_mutation() {
    for field in ["state", "endpoint", "instance", "role"] {
        let f = Fixture::new();
        let mut identity = super::topology_handoff::identity();
        match field {
            "state" => identity.payload.handoff.namespace.state_dir = "/foreign-state".into(),
            "endpoint" => identity.payload.handoff.namespace.host_endpoint = "/foreign.sock".into(),
            "instance" => {
                identity.payload.handoff.namespace.instance = "foreign".into();
                identity.claim.instance = "foreign".into();
                identity.scope = herdr_threads::cli::journal::IntentScope::Cooperative {
                    instance: "foreign".into(),
                    seat: identity.claim.seat.clone(),
                };
            }
            _ => identity.claim.role = herdr_threads::protocol::authority::CallerRole::Subagent,
        }
        identity.digest = identity.semantic_digest().unwrap();
        assert!(f.call(begin(identity)).is_err(), "{field}");
        assert_eq!(
            f.db.query_row("SELECT count(*) FROM bootstrap_handoffs", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
#[test]
fn actual_guarded_domain_capability_requires_concrete_composition_and_handler() {
    use herdr_threads::daemon::control::{ControlService, StopController};
    let f = Fixture::new();
    let peer = herdr_threads::test_support::peer_identity(501);
    let budget = CallBudget {
        deadline: MonoInstant(f.clock.monotonic_now().0 + 5_000),
        cancellation: Cancellation::default(),
    };
    let control = ControlService::new_guarded(
        StopController::new(
            uuid::Uuid::new_v4(),
            uuid::Uuid::new_v4(),
            Cancellation::default(),
        ),
        |_| -> herdr_threads::daemon::health::HealthInputs {
            panic!("health outside capability probe")
        },
        f.service,
    )
    .unwrap();
    let CommandResult::Capabilities(caps) = control
        .handle(Command::Capabilities, peer, &budget)
        .unwrap()
    else {
        panic!("capabilities missing")
    };
    assert!(
        caps.capabilities
            .iter()
            .any(|name| name
                == herdr_threads::protocol::capabilities::BOOTSTRAP_GUARDED_RESOLUTION_V1)
    );
    assert!(
        caps.capabilities
            .iter()
            .any(|name| name
                == herdr_threads::protocol::capabilities::BOOTSTRAP_INSPECTED_RECOVERY_V1)
    );
    // Advertising probe must also execute the real typed deciding handler.
    assert!(
        matches!(control.handle(begin(super::topology_handoff::identity()), peer, &budget).unwrap(), CommandResult::Bootstrap(ref status) if status.state == BootstrapState::Prepared)
    );
    let f = Fixture::new();
    let control = ControlService::new(
        StopController::new(
            uuid::Uuid::new_v4(),
            uuid::Uuid::new_v4(),
            Cancellation::default(),
        ),
        |_| -> herdr_threads::daemon::health::HealthInputs {
            panic!("health outside capability probe")
        },
        f.service,
    );
    let CommandResult::Capabilities(caps) = control
        .handle(Command::Capabilities, peer, &budget)
        .unwrap()
    else {
        panic!("capabilities missing")
    };
    assert!(
        !caps
            .capabilities
            .iter()
            .any(|name| name
                == herdr_threads::protocol::capabilities::BOOTSTRAP_GUARDED_RESOLUTION_V1)
    );
    assert!(
        !caps
            .capabilities
            .iter()
            .any(|name| name
                == herdr_threads::protocol::capabilities::BOOTSTRAP_INSPECTED_RECOVERY_V1)
    );
}

fn delivery() -> DeliveryMutation {
    use herdr_threads::cli::journal::{DeliveryPlan, IntentScope, SemanticMutation};
    use sha2::{Digest, Sha256};
    let bootstrap = super::topology_handoff::identity();
    let claim = bootstrap.claim;
    let plan = DeliveryPlan {
        version: 1,
        payload: bootstrap.payload.handoff,
        recipient: claim.seat.clone(),
    };
    let scope = IntentScope::Cooperative {
        instance: claim.instance.clone(),
        seat: claim.seat.clone(),
    };
    let semantic = SemanticMutation::Frozen {
        claim: claim.clone(),
        mutation: Box::new(SemanticMutation::HandoffDelivery(Box::new(plan.clone()))),
    };
    let digest = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&semantic).unwrap())
    );
    let mut request = DeliveryMutation {
        scope,
        claim,
        digest,
        plan,
        action: DeliveryAction::Begin(HandoffMutation {
            identity: super::handoff_fences::identity(),
            operation: herdr_threads::protocol::ids::OperationId::new("placeholder"),
        }),
    };
    request.action = DeliveryAction::Begin(HandoffMutation {
        identity: request.identity(),
        operation: request.plan.payload.keys.begin.clone(),
    });
    request
}
#[test]
fn public_delivery_envelope_reaches_real_deciding_begin_without_launch() {
    let f = Fixture::new();
    let request = delivery();
    request.validate().unwrap();
    let result = f.call(Command::HandoffDelivery(Box::new(request)));
    assert!(
        matches!(result, Ok(CommandResult::Handoff(ref r)) if r.state == HandoffState::Live),
        "actual scoped delivery handler: {result:?}"
    );
    assert_eq!(
        f.db.query_row("SELECT count(*) FROM allocation_decisions", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

fn refresh_delivery(request: &mut DeliveryMutation) {
    use herdr_threads::cli::journal::SemanticMutation;
    use sha2::{Digest, Sha256};
    let semantic = SemanticMutation::Frozen {
        claim: request.claim.clone(),
        mutation: Box::new(SemanticMutation::HandoffDelivery(Box::new(
            request.plan.clone(),
        ))),
    };
    request.digest = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&semantic).unwrap())
    );
    request.action = DeliveryAction::Begin(HandoffMutation {
        identity: request.identity(),
        operation: request.plan.payload.keys.begin.clone(),
    });
}
#[test]
fn public_delivery_scoped_status_and_prepublication_admission_are_read_only() {
    let f = Fixture::new();
    let mut request = delivery();
    request.action = DeliveryAction::Prepare(request.identity());
    assert!(matches!(
        f.call(Command::HandoffDelivery(Box::new(request.clone())))
            .unwrap(),
        CommandResult::SeatResolved(_)
    ));
    assert_eq!(
        f.db.query_row("SELECT count(*) FROM operations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    request.action = DeliveryAction::Begin(HandoffMutation {
        identity: request.identity(),
        operation: request.plan.payload.keys.begin.clone(),
    });
    f.call(Command::HandoffDelivery(Box::new(request.clone())))
        .unwrap();
    request.action = DeliveryAction::Status(request.identity());
    assert!(matches!(
        f.call(Command::HandoffDelivery(Box::new(request.clone())))
            .unwrap(),
        CommandResult::Handoff(_)
    ));
    let live_status = f
        .call(Command::HandoffDelivery(Box::new(request.clone())))
        .unwrap();
    f.db.execute("INSERT INTO recovery_holds(instance_id,target_id,baseline_boot,baseline_epoch,reason) VALUES('i','p','b',1,'pure Status control')", []).unwrap();
    assert_eq!(
        f.call(Command::HandoffDelivery(Box::new(request.clone())))
            .unwrap(),
        live_status,
        "Live Status reads identity without granting current A2 admission"
    );
    let mut prepare = request.clone();
    prepare.action = DeliveryAction::Prepare(prepare.identity());
    assert!(
        f.call(Command::HandoffDelivery(Box::new(prepare))).is_err(),
        "subsequent live preparation must enforce current recipient guard"
    );
    for field in ["state", "endpoint", "spelling"] {
        let mut copied = request.clone();
        match field {
            "state" => copied.plan.payload.namespace.state_dir = "/other".into(),
            "endpoint" => copied.plan.payload.namespace.host_endpoint = "/other.sock".into(),
            _ => copied.plan.payload.namespace.state_dir = "/state//".into(),
        }
        refresh_delivery(&mut copied);
        copied.action = DeliveryAction::Status(copied.identity());
        assert!(
            f.call(Command::HandoffDelivery(Box::new(copied))).is_err(),
            "{field}"
        );
    }
    assert_eq!(
        f.db.query_row("SELECT count(*) FROM operations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
#[test]
fn public_delivery_namespace_and_original_actor_refuse_before_begin_or_replay() {
    for field in ["state", "endpoint", "human", "phase"] {
        let f = Fixture::new();
        let mut request = delivery();
        f.call(Command::HandoffDelivery(Box::new(request.clone())))
            .unwrap();
        let before: String =
            f.db.query_row("SELECT result_json FROM operations", [], |r| r.get(0))
                .unwrap();
        match field {
            "state" => request.plan.payload.namespace.state_dir = "/foreign-state".into(),
            "endpoint" => request.plan.payload.namespace.host_endpoint = "/foreign.sock".into(),
            "human" => request.claim.harness = herdr_threads::protocol::authority::Harness::Human,
            _ => {}
        }
        refresh_delivery(&mut request);
        if field == "phase" {
            let DeliveryAction::Begin(ref mut inner) = request.action else {
                unreachable!()
            };
            inner.identity.recipient = herdr_threads::protocol::ids::SeatId::new("substituted");
        }
        assert!(
            f.call(Command::HandoffDelivery(Box::new(request))).is_err(),
            "{field}"
        );
        assert_eq!(
            f.db.query_row("SELECT result_json FROM operations", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            before
        );
    }
}
#[test]
fn public_delivery_current_recipient_hold_refuses_live_begin_and_send_preparation() {
    let f = Fixture::new();
    let mut request = delivery();
    f.call(Command::HandoffDelivery(Box::new(request.clone())))
        .unwrap();
    f.db.execute("INSERT INTO recovery_holds(instance_id,target_id,baseline_boot,baseline_epoch,reason) VALUES('i','p','b',1,'repair')", []).unwrap();
    assert!(
        f.call(Command::HandoffDelivery(Box::new(request.clone())))
            .is_err()
    );
    request.action = DeliveryAction::Send(herdr_threads::protocol::commands::SendMessage {
        delivery_mode: herdr_threads::protocol::commands::DeliveryMode::Ordinary,
        thread: request.plan.payload.channel.thread().unwrap().clone(),
        body: request.plan.payload.body.clone(),
        direct_recipient: None,
        invited_recipients: vec![request.plan.recipient.clone()],
        deadline_millis: None,
        operation: request.plan.payload.keys.send.clone(),
        claim: request.claim.clone(),
        relays_user: false,
        user_intent: None,
    });
    assert!(f.call(Command::HandoffDelivery(Box::new(request))).is_err());
    assert_eq!(
        f.db.query_row("SELECT count(*) FROM send_preparations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        f.db.query_row("SELECT count(*) FROM messages", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn public_delivery_live_cached_create_replay_rechecks_recipient() {
    live_cached_replay("create");
}
#[test]
fn public_delivery_live_cached_invite_replay_rechecks_recipient() {
    live_cached_replay("invite");
}
#[test]
fn public_delivery_live_cached_send_replay_rechecks_recipient() {
    live_cached_replay("send");
}
fn live_cached_replay(phase: &str) {
    for change in ["hold", "binding", "retirement"] {
        let f = Fixture::new();
        let mut request = delivery();
        if phase == "create" {
            request.plan.payload.channel = HandoffChannel::New {
                name: None,
                topic: "new".into(),
                goal: "new".into(),
            };
            refresh_delivery(&mut request);
        }
        f.call(Command::HandoffDelivery(Box::new(request.clone())))
            .unwrap();
        request.action = if phase == "create" {
            DeliveryAction::Create(herdr_threads::protocol::commands::CreateThread {
                name: None,
                topic: "new".into(),
                goal: "new".into(),
                claim: request.claim.clone(),
                operation: request.plan.payload.keys.create.clone(),
            })
        } else if phase == "invite" {
            DeliveryAction::Invite(herdr_threads::protocol::commands::Invite {
                confirmation: None,
                thread: request.plan.payload.channel.thread().unwrap().clone(),
                seat: request.plan.recipient.clone(),
                deadline_millis: None,
                claim: request.claim.clone(),
                operation: request.plan.payload.keys.invite.clone(),
            })
        } else {
            DeliveryAction::Send(herdr_threads::protocol::commands::SendMessage {
                delivery_mode: herdr_threads::protocol::commands::DeliveryMode::Ordinary,
                thread: request.plan.payload.channel.thread().unwrap().clone(),
                body: request.plan.payload.body.clone(),
                direct_recipient: None,
                invited_recipients: vec![request.plan.recipient.clone()],
                deadline_millis: None,
                relays_user: false,
                user_intent: None,
                claim: request.claim.clone(),
                operation: request.plan.payload.keys.send.clone(),
            })
        };
        let historical = f
            .call(Command::HandoffDelivery(Box::new(request.clone())))
            .unwrap();
        let old = match &request.action {
            DeliveryAction::Create(v) => Command::CreateThread(v.clone()),
            DeliveryAction::Invite(v) => Command::Invite(v.clone()),
            DeliveryAction::Send(v) => Command::SendMessage(v.clone()),
            _ => unreachable!(),
        };
        let retained: Vec<(String, Vec<u8>, String)> =
            f.db.prepare(
                "SELECT operation_key,digest,result_json FROM operations ORDER BY operation_key",
            )
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let count: i64 =
            f.db.query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
                .unwrap();
        match change {
            "hold" => {
                f.db.execute("INSERT INTO recovery_holds(instance_id,target_id,baseline_boot,baseline_epoch,reason) VALUES('i','p','b',1,'repair')", []).unwrap();
            }
            "binding" => {
                f.db.execute("UPDATE seats SET generation=generation+1 WHERE id='s'", [])
                    .unwrap();
            }
            _ => {
                f.db.execute(
                    "UPDATE seats SET state='retired',retired_at=1 WHERE id='s'",
                    [],
                )
                .unwrap();
            }
        }
        let replay = f.call(Command::HandoffDelivery(Box::new(request)));
        assert_eq!(
            f.call(old).unwrap(),
            historical,
            "old None historical replay stays unchanged: {phase}/{change}"
        );
        assert!(
            replay.is_err(),
            "live {phase} replay must check current recipient: {replay:?}"
        );
        let after: Vec<(String, Vec<u8>, String)> =
            f.db.prepare(
                "SELECT operation_key,digest,result_json FROM operations ORDER BY operation_key",
            )
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(after, retained);
        assert_eq!(
            f.db.query_row::<i64, _, _>("SELECT count(*) FROM messages", [], |r| r.get(0))
                .unwrap(),
            count
        );
    }
}

#[test]
fn public_delivery_completed_status_and_keyed_history_bypass_only_live_guards() {
    let f = Fixture::new();
    let mut request = delivery();
    f.call(Command::HandoffDelivery(Box::new(request.clone())))
        .unwrap();
    request.action = DeliveryAction::Complete(HandoffMutation {
        identity: request.identity(),
        operation: request.plan.payload.keys.complete.clone(),
    });
    let completed = f
        .call(Command::HandoffDelivery(Box::new(request.clone())))
        .unwrap();
    f.db.execute_batch("UPDATE threads SET archived=1; UPDATE occupant_bindings SET ended_at=1; UPDATE seats SET state='retired',retired_at=1,generation=generation+1").unwrap();
    assert_eq!(
        f.call(Command::HandoffDelivery(Box::new(request.clone())))
            .unwrap(),
        completed
    );
    request.action = DeliveryAction::Status(request.identity());
    assert_eq!(
        f.call(Command::HandoffDelivery(Box::new(request.clone())))
            .unwrap(),
        completed
    );
    request.plan.payload.namespace.state_dir = "/other".into();
    refresh_delivery(&mut request);
    request.action = DeliveryAction::Status(request.identity());
    assert!(f.call(Command::HandoffDelivery(Box::new(request))).is_err());
}

#[test]
fn actual_main_delivery_phase_preserves_ordinary_and_rejects_lazy_mode() {
    use herdr_threads::protocol::commands::{DeliveryMode, SendMessage};
    let f = Fixture::new();
    let mut request = delivery();
    let mut send = SendMessage {
        delivery_mode: DeliveryMode::Ordinary,
        thread: request.plan.payload.channel.thread().unwrap().clone(),
        body: request.plan.payload.body.clone(),
        direct_recipient: None,
        invited_recipients: vec![request.plan.recipient.clone()],
        deadline_millis: None,
        operation: request.plan.payload.keys.send.clone(),
        claim: request.claim.clone(),
        relays_user: false,
        user_intent: None,
    };
    request.action = DeliveryAction::Send(send.clone());
    request.validate().unwrap();
    let ordinary = serde_json::to_vec(&request).unwrap();
    assert!(!String::from_utf8_lossy(&ordinary).contains("delivery_mode"));
    let restored: DeliveryMutation = serde_json::from_slice(&ordinary).unwrap();
    assert_eq!(serde_json::to_vec(&restored).unwrap(), ordinary);
    restored.validate().unwrap();

    send.delivery_mode = DeliveryMode::Lazy;
    // This existing lower-level refusal already prevents a Lazy+ACK send.
    // The direct envelope control below tests exact frozen-phase matching.
    assert!(Command::SendMessage(send.clone()).validate().is_err());
    request.action = DeliveryAction::Send(send);
    let direct_refusal = request.validate().is_err();
    assert!(f.call(Command::HandoffDelivery(Box::new(request))).is_err());
    for table in ["send_preparations", "messages", "operations"] {
        assert_eq!(
            f.db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0,
        );
    }
    assert!(
        direct_refusal,
        "frozen Ordinary delivery phase must reject a changed Lazy mode"
    );
}

#[test]
fn direct_marker_delivery_refuses_before_preparation_and_preserves_original_send() {
    use herdr_threads::{
        ports::{BootstrapStorePort, DurableWorkAdmission},
        protocol::{
            commands::{DeliveryMode, SendMessage},
            results::ErrorCode,
        },
    };
    let f = Fixture::new();
    let mut request = delivery();
    f.call(Command::HandoffDelivery(Box::new(request.clone())))
        .unwrap();
    let send = SendMessage {
        delivery_mode: DeliveryMode::Ordinary,
        thread: request.plan.payload.channel.thread().unwrap().clone(),
        body: request.plan.payload.body.clone(),
        direct_recipient: None,
        invited_recipients: vec![request.plan.recipient.clone()],
        deadline_millis: None,
        operation: request.plan.payload.keys.send.clone(),
        claim: request.claim.clone(),
        relays_user: false,
        user_intent: None,
    };
    let snapshot = || {
        ["send_preparations", "prepared_recipients", "operations"].map(|table| {
            f.db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap()
        })
    };
    let before = snapshot();
    let mut marked = send.clone();
    marked.direct_recipient = Some(request.plan.recipient.clone());
    request.action = DeliveryAction::Send(marked.clone());
    assert!(
        f.call(Command::HandoffDelivery(Box::new(request.clone())))
            .is_err()
    );
    assert_eq!(
        snapshot(),
        before,
        "unsupported envelope must not stage its altered digest or audience"
    );
    let budget = CallBudget {
        deadline: MonoInstant(f.clock.monotonic_now().0 + 5000),
        cancellation: Cancellation::default(),
    };
    assert_eq!(
        f.store
            .delivery_prepare_send_step(
                &request.plan.payload.namespace,
                &request,
                DurableWorkAdmission::new(16).unwrap(),
                &budget
            )
            .unwrap_err()
            .code,
        ErrorCode::Unsupported
    );
    assert_eq!(
        f.store
            .bootstrap_prepare_send_step(
                &request.plan.payload.namespace,
                &marked,
                DurableWorkAdmission::new(16).unwrap(),
                &budget
            )
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert_eq!(snapshot(), before);
    request.action = DeliveryAction::Send(send);
    assert!(matches!(
        f.call(Command::HandoffDelivery(Box::new(request))).unwrap(),
        CommandResult::MessageSent(_)
    ));
}

#[test]
fn ordinary_unlinked_direct_ensure_send_reaches_selected_bootstrap_runtime() {
    use herdr_threads::protocol::{
        commands::{DeliveryMode, EnsureDirectThread, SendMessage},
        ids::{OperationId, SeatId},
    };
    let f = Fixture::new();
    let caller = super::topology_handoff::identity().claim;
    f.db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES('peer','i','resolved','native','peer-pane',1,1,0)",[]).unwrap();
    f.db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES('i','peer-pane','b',1,1,0,'fresh','term-peer','inc','coherent_enumeration',1)",[]).unwrap();
    let ensure = Command::EnsureDirectThread(EnsureDirectThread {
        creation_policy: None,
        recipient: SeatId::new("peer"),
        operation: OperationId::new("direct-runtime-ensure"),
        claim: caller.clone(),
    });
    let CommandResult::ThreadCreated(t) = f.call(ensure).unwrap() else {
        panic!("missing pair")
    };
    let send = Command::SendMessage(SendMessage {
        delivery_mode: DeliveryMode::Lazy,
        thread: t,
        body: "unlinked direct".into(),
        direct_recipient: Some(SeatId::new("peer")),
        invited_recipients: vec![],
        deadline_millis: None,
        operation: OperationId::new("direct-runtime-send"),
        claim: caller,
        relays_user: false,
        user_intent: None,
    });
    assert!(matches!(
        f.call(send.clone()).unwrap(),
        CommandResult::MessageSent(_)
    ));
    assert!(matches!(
        f.call(send).unwrap(),
        CommandResult::MessageSent(_)
    ));
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM lazy_recipients WHERE seat_id='peer'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM invitations WHERE seat_id='peer' AND state='pending'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[test]
fn automatic_contract_public_dispatch_refuses_every_phase_without_effects() {
    use herdr_threads::{cli::journal::IntentScope, protocol::ids::*};
    let mut f = Fixture::new();
    f.service = DomainService::new("i".into(), f.store.clone(), f.clock.clone());
    let claim = super::handoff_fences::identity().claim;
    let payload = AutomaticHandoffPayload::new(
        super::topology_handoff::namespace(),
        OperationId::new("automatic-compound"),
        "work".into(),
    )
    .unwrap();
    let plan = AutomaticDeliveryPlan::new(payload, SeatId::new("peer")).unwrap();
    let request = AutomaticDeliveryMutation::new(
        IntentScope::Cooperative {
            instance: claim.instance.clone(),
            seat: claim.seat.clone(),
        },
        claim,
        AutomaticOriginal::Delivery(Box::new(plan)),
    )
    .unwrap();
    let counts = || {
        ["operations", "messages", "channel_handoff_fences"].map(|table| {
            f.db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap()
        })
    };
    let before = counts();
    let mut status = request.clone();
    status.action = AutomaticDeliveryAction::Status;
    for phase in [
        status,
        request.clone(),
        request.begin(),
        request.ensure(),
        request.send(ThreadId::new("t")),
        request.complete(),
    ] {
        let result = f.call(Command::AutomaticHandoffDelivery(Box::new(phase)));
        assert!(
            matches!(result, Err(ref error) if error.code == herdr_threads::protocol::results::ErrorCode::Unsupported),
            "{result:?}"
        );
        assert_eq!(
            counts(),
            before,
            "unsupported automatic contract must not mutate canonical state"
        );
    }
}

fn automatic_request(f: &Fixture) -> AutomaticDeliveryMutation {
    use herdr_threads::{cli::journal::IntentScope, protocol::ids::*};
    f.db.execute_batch("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES('peer','i','resolved','native','peer-pane',1,1,0)").unwrap();
    f.db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES('i','peer-pane','b',1,1,0,'fresh','term-peer','inc','coherent_enumeration',1)",[]).unwrap();
    let claim = super::handoff_fences::identity().claim;
    AutomaticDeliveryMutation::new(
        IntentScope::Cooperative {
            instance: claim.instance.clone(),
            seat: claim.seat.clone(),
        },
        claim,
        AutomaticOriginal::Delivery(Box::new(
            AutomaticDeliveryPlan::new(
                AutomaticHandoffPayload::new(
                    super::topology_handoff::namespace(),
                    OperationId::new("automatic-compound"),
                    "work".into(),
                )
                .unwrap(),
                SeatId::new("peer"),
            )
            .unwrap(),
        )),
    )
    .unwrap()
}
fn automatic_call(
    f: &Fixture,
    request: AutomaticDeliveryMutation,
) -> Result<CommandResult, herdr_threads::protocol::results::ApiError> {
    f.call(Command::AutomaticHandoffDelivery(Box::new(request)))
}
#[test]
fn automatic_canonical_ensure_attaches_exact_pair_and_replays_without_reinviting() {
    let f = Fixture::new();
    let request = automatic_request(&f);
    assert!(automatic_call(&f, request.ensure()).is_err());
    assert_eq!(
        f.db.query_row("SELECT count(*) FROM direct_threads", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        automatic_call(&f, request.clone()).unwrap(),
        CommandResult::SeatResolved(request.plan().recipient.clone())
    );
    assert!(
        matches!(automatic_call(&f,request.begin()).unwrap(), CommandResult::Handoff(ref r) if r.thread.is_none() && r.state==HandoffState::Live)
    );
    let result = automatic_call(&f, request.ensure()).unwrap();
    let AutomaticDeliveryAction::Ensure(raw) = request.ensure().action else {
        panic!("Ensure phase")
    };
    assert_eq!(
        f.call(Command::EnsureDirectThread(raw)).unwrap_err().code,
        herdr_threads::protocol::results::ErrorCode::OperationPayloadMismatch
    );
    let thread = automatic_selected_thread(&result).unwrap();
    let attached: String =
        f.db.query_row(
            "SELECT thread_id FROM channel_handoff_fences WHERE compound='automatic-compound'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(attached, thread.as_str());
    let pair: String =
        f.db.query_row(
            "SELECT thread_id FROM direct_threads WHERE seat_a='peer' AND seat_b='s'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(pair, attached);
    f.db.execute(
        "UPDATE memberships SET state='left',left_at=1 WHERE thread_id=?1 AND seat_id='peer'",
        [thread.as_str()],
    )
    .unwrap();
    assert_eq!(automatic_call(&f, request.ensure()).unwrap(), result);
    assert_eq!(
        f.db.query_row("SELECT count(*) FROM invitations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM messages WHERE kind='ordinary'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}
#[test]
fn automatic_canonical_companion_is_immutable_retained_foreign_keyed_and_audited() {
    let f = Fixture::new();
    let request = automatic_request(&f);
    automatic_call(&f, request.begin()).unwrap();
    assert!(
        f.db.execute(
            "INSERT INTO automatic_handoff_companions SELECT * FROM automatic_handoff_companions",
            []
        )
        .is_err()
    );
    assert!(
        f.db.execute(
            "UPDATE automatic_handoff_keys SET role='send' WHERE role='ensure'",
            []
        )
        .is_err()
    );
    assert!(
        f.db.execute("DELETE FROM automatic_handoff_keys", [])
            .is_err()
    );
    assert!(f.db.execute("INSERT INTO automatic_handoff_keys SELECT instance_id,actor_scope,compound,'another-key',role FROM automatic_handoff_keys WHERE role='ensure'",[]).is_err());
    assert!(
        f.db.execute(
            "UPDATE automatic_handoff_companions SET original_json=x'00'",
            []
        )
        .is_err()
    );
    assert!(
        f.db.execute("DELETE FROM automatic_handoff_companions", [])
            .is_err()
    );
    assert!(f.db.execute("INSERT INTO automatic_handoff_companions SELECT instance_id,actor_scope,'missing',state_dir,host_endpoint,original_json,begin_key,ensure_key,send_key,complete_key FROM automatic_handoff_companions",[]).is_err());
    f.db.execute_batch("DROP TRIGGER automatic_handoff_immutable")
        .unwrap();
    assert!(herdr_threads::store::schema::initialize(&f.db, || f.clock.utc_now()).is_err());
}
#[test]
fn automatic_canonical_namespace_requires_exact_frozen_path_spelling() {
    use herdr_threads::protocol::results::ErrorCode;
    for field in ["state", "endpoint"] {
        let f = Fixture::new();
        let mut request = automatic_request(&f);
        let AutomaticOriginal::Delivery(plan) = &mut request.original else {
            panic!()
        };
        match field {
            "state" => plan.payload.namespace.state_dir = "/state//".into(),
            _ => plan.payload.namespace.host_endpoint = "/host.sock/".into(),
        }
        request.digest = request.original.digest(&request.claim).unwrap();
        request.validate().unwrap();
        let prepare = automatic_call(&f, request.clone());
        let begin = automatic_call(&f, request.begin());
        assert!(
            matches!(prepare, Err(ref error) if error.code == ErrorCode::InstanceMismatch),
            "{field} prepare: {prepare:?}"
        );
        assert!(
            matches!(begin, Err(ref error) if error.code == ErrorCode::InstanceMismatch),
            "{field} begin: {begin:?}"
        );
        for table in [
            "operations",
            "channel_handoff_fences",
            "automatic_handoff_companions",
            "automatic_handoff_keys",
            "direct_threads",
            "invitations",
        ] {
            let count: i64 =
                f.db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                    .unwrap();
            assert_eq!(count, 0, "{field}: {table}");
        }
    }
}
#[test]
fn automatic_canonical_forged_original_namespace_claim_keys_and_payload_have_no_effects() {
    use herdr_threads::protocol::results::ErrorCode;
    let f = Fixture::new();
    let request = automatic_request(&f);
    automatic_call(&f, request.begin()).unwrap();
    let before =
        f.db.query_row("SELECT count(*) FROM operations", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap();
    for change in ["body", "namespace", "claim", "key", "recipient"] {
        let mut forged = request.clone();
        let AutomaticOriginal::Delivery(plan) = &mut forged.original else {
            panic!()
        };
        match change {
            "body" => plan.payload.body = "different".into(),
            "namespace" => plan.payload.namespace.state_dir = "/other".into(),
            "claim" => forged.claim.binding_generation += 1,
            "key" => {
                plan.payload.keys.ensure =
                    herdr_threads::protocol::ids::OperationId::new("other-key")
            }
            _ => plan.recipient = herdr_threads::protocol::ids::SeatId::new("other-peer"),
        }
        forged.digest = forged.original.digest(&forged.claim).unwrap();
        let error = automatic_call(&f, forged.ensure()).unwrap_err();
        assert!(
            matches!(
                error.code,
                ErrorCode::OperationPayloadMismatch
                    | ErrorCode::InstanceMismatch
                    | ErrorCode::CallerUnverified
            ),
            "{change}: {error:?}"
        );
        assert_eq!(
            f.db.query_row("SELECT count(*) FROM operations", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            before
        );
        assert_eq!(
            f.db.query_row("SELECT count(*) FROM direct_threads", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    let mut bad = request.ensure();
    if let AutomaticDeliveryAction::Ensure(v) = &mut bad.action {
        v.operation = herdr_threads::protocol::ids::OperationId::new("forged");
    }
    assert!(automatic_call(&f, bad).is_err());
}
#[test]
fn automatic_canonical_attachment_failure_rolls_back_resolver_and_operation() {
    let f = Fixture::new();
    let request = automatic_request(&f);
    automatic_call(&f, request.begin()).unwrap();
    f.db.execute_batch("CREATE TRIGGER deny_automatic_attachment BEFORE UPDATE OF thread_id ON channel_handoff_fences WHEN NEW.compound='automatic-compound' BEGIN SELECT RAISE(ABORT,'attachment failure'); END").unwrap();
    assert!(automatic_call(&f, request.ensure()).is_err());
    assert_eq!(
        f.db.query_row("SELECT count(*) FROM direct_threads", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        f.db.query_row("SELECT count(*) FROM invitations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM operations WHERE operation_key=?1",
            [request.plan().payload.keys.ensure.as_str()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn automatic_canonical_refuses_raw_create_invite_even_with_completed_historical_rows() {
    use herdr_threads::protocol::{
        commands::{CreateThread, Invite},
        ids::ThreadId,
    };
    use sha2::{Digest, Sha256};
    for historical in [false, true] {
        let f = Fixture::new();
        let request = automatic_request(&f);
        automatic_call(&f, request.begin()).unwrap();
        let create = CreateThread {
            name: None,
            topic: "forged".into(),
            goal: "forged".into(),
            operation: request.plan().payload.keys.ensure.clone(),
            claim: request.claim.clone(),
        };
        let invite = Invite {
            confirmation: None,
            thread: ThreadId::new("t"),
            seat: request.plan().recipient.clone(),
            deadline_millis: None,
            operation: request.plan().payload.keys.complete.clone(),
            claim: request.claim.clone(),
        };
        if historical {
            for (key, bytes, result) in [
                (
                    create.operation.as_str(),
                    serde_json::to_vec(&serde_json::to_value(("create_thread", &create)).unwrap())
                        .unwrap(),
                    CommandResult::ThreadCreated(ThreadId::new("t")),
                ),
                (
                    invite.operation.as_str(),
                    serde_json::to_vec(&serde_json::to_value(("invite", &invite)).unwrap())
                        .unwrap(),
                    CommandResult::Invitation(herdr_threads::protocol::ids::InvitationId::new(
                        "historical-invitation",
                    )),
                ),
            ] {
                let digest = Sha256::digest(bytes);
                f.db.execute("INSERT INTO operations(actor_scope,operation_key,digest,result_json,decided_at) VALUES('seat:s',?1,?2,?3,0)",rusqlite::params![key,digest.as_slice(),serde_json::to_string(&result).unwrap()]).unwrap();
            }
            f.db.execute(
                "UPDATE channel_handoff_fences SET thread_id='t',state='completed',completed_at=1",
                [],
            )
            .unwrap();
        }
        // The historical fixtures must pass exact ordinary permit issuance;
        // refusal below must come from canonical automatic-mode admission.
        for raw in [
            herdr_threads::protocol::commands::PermitMutation::CreateThread(create.clone()),
            herdr_threads::protocol::commands::PermitMutation::Invite(invite.clone()),
        ] {
            f.store
                .issue_cooperative_permit(
                    herdr_threads::store::cooperative_permit_request(&raw).unwrap(),
                    &CallBudget {
                        deadline: MonoInstant(f.clock.monotonic_now().0 + 5000),
                        cancellation: Cancellation::default(),
                    },
                )
                .unwrap();
        }
        let before =
            f.db.query_row("SELECT count(*) FROM operations", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap();
        for command in [Command::CreateThread(create), Command::Invite(invite)] {
            let error = f.call(command).unwrap_err();
            assert_eq!(
                error.code,
                herdr_threads::protocol::results::ErrorCode::OperationPayloadMismatch,
                "{historical}: {error:?}"
            );
        }
        assert_eq!(
            f.db.query_row("SELECT count(*) FROM operations", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            before
        );
        assert_eq!(
            f.db.query_row("SELECT count(*) FROM invitations", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn automatic_canonical_selected_attachment_cannot_be_retargeted() {
    let f = Fixture::new();
    let request = automatic_request(&f);
    automatic_call(&f, request.begin()).unwrap();
    automatic_call(&f, request.ensure()).unwrap();
    assert!(
        f.db.execute(
            "UPDATE channel_handoff_fences SET thread_id='t' WHERE compound='automatic-compound'",
            []
        )
        .is_err()
    );
}
#[test]
fn automatic_canonical_ensure_historical_result_must_match_attached_thread() {
    let f = Fixture::new();
    let request = automatic_request(&f);
    automatic_call(&f, request.begin()).unwrap();
    automatic_call(&f, request.ensure()).unwrap();
    f.db.execute(
        "UPDATE operations SET result_json=?2 WHERE operation_key=?1",
        rusqlite::params![
            request.plan().payload.keys.ensure.as_str(),
            serde_json::to_string(&CommandResult::ThreadCreated(
                herdr_threads::protocol::ids::ThreadId::new("t")
            ))
            .unwrap()
        ],
    )
    .unwrap();
    assert!(automatic_call(&f, request.ensure()).is_err());
}
#[test]
fn automatic_canonical_full_outer_original_is_frozen_and_oversize_refuses_before_effects() {
    use herdr_threads::{
        cli::launch::LaunchRequest,
        harness::context::Harness as ContextHarness,
        protocol::{ids::HostTargetId, output::ContinuationContext},
    };
    let f = Fixture::new();
    let request = automatic_request(&f);
    let mut handoff = request.clone();
    handoff.original = AutomaticOriginal::Handoff(Box::new(AutomaticHandoffPlan {
        version: 2,
        delivery: request.plan().clone(),
        launch: LaunchRequest {
            target: HostTargetId::new("peer-pane"),
            harness: ContextHarness::Codex,
            harness_binary: None,
            argv: vec!["--model".into(), "frozen".into()],
            name: None,
            pane_label: None,
        },
        context: ContinuationContext {
            state_dir: Some("/state".into()),
            host: Some("/host.sock".into()),
        },
        startup_input: None,
    }));
    handoff.digest = handoff.original.digest(&handoff.claim).unwrap();
    automatic_call(&f, handoff.begin()).unwrap();
    let mut changed = handoff.clone();
    let AutomaticOriginal::Handoff(plan) = &mut changed.original else {
        panic!()
    };
    plan.launch.argv[1] = "different".into();
    changed.digest = changed.original.digest(&changed.claim).unwrap();
    assert_eq!(
        automatic_call(&f, changed.ensure()).unwrap_err().code,
        herdr_threads::protocol::results::ErrorCode::OperationPayloadMismatch
    );
    let mut oversized = handoff;
    let AutomaticOriginal::Handoff(plan) = &mut oversized.original else {
        panic!()
    };
    plan.launch.argv = vec!["\u{1}".repeat(4096); 16];
    oversized.digest = oversized.original.digest(&oversized.claim).unwrap();
    oversized.original.validate().unwrap();
    assert!(oversized.validate().is_err());
    let before =
        f.db.query_row("SELECT count(*) FROM operations", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap();
    assert_eq!(
        automatic_call(&f, oversized.begin()).unwrap_err().code,
        herdr_threads::protocol::results::ErrorCode::InvalidRequest
    );
    assert_eq!(
        f.db.query_row("SELECT count(*) FROM operations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        before
    );
    assert_eq!(
        f.db.query_row("SELECT count(*) FROM direct_threads", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

struct GenericBootstrap;
impl herdr_threads::ports::BootstrapStorePort for GenericBootstrap {
    fn bootstrap_begin_permit(
        &self,
        _canonical: &herdr_threads::protocol::handoff::HandoffNamespace,
        _command: &herdr_threads::protocol::handoff::BeginBootstrap,
        _budget: &CallBudget,
    ) -> Result<
        herdr_threads::protocol::authority::MutationPermit,
        herdr_threads::protocol::results::ApiError,
    > {
        unreachable!("automatic route reached unrelated legacy adapter")
    }
    fn bootstrap_prepare_send_step(
        &self,
        _canonical: &herdr_threads::protocol::handoff::HandoffNamespace,
        _command: &herdr_threads::protocol::commands::SendMessage,
        _admission: herdr_threads::ports::DurableWorkAdmission,
        _budget: &CallBudget,
    ) -> Result<
        herdr_threads::ports::SendPreparationProgress,
        herdr_threads::protocol::results::ApiError,
    > {
        unreachable!("automatic route reached unrelated legacy adapter")
    }
    fn delivery_query(
        &self,
        _canonical: &herdr_threads::protocol::handoff::HandoffNamespace,
        _request: &herdr_threads::protocol::handoff::DeliveryMutation,
        _budget: &CallBudget,
    ) -> Result<CommandResult, herdr_threads::protocol::results::ApiError> {
        unreachable!("automatic route reached unrelated legacy adapter")
    }
    fn delivery_mutate(
        &self,
        _canonical: &herdr_threads::protocol::handoff::HandoffNamespace,
        _request: &herdr_threads::protocol::handoff::DeliveryMutation,
        _permit: herdr_threads::protocol::authority::MutationPermit,
        _budget: &CallBudget,
    ) -> Result<CommandResult, herdr_threads::protocol::results::ApiError> {
        unreachable!("automatic route reached unrelated legacy adapter")
    }
    fn delivery_prepare_send_step(
        &self,
        _canonical: &herdr_threads::protocol::handoff::HandoffNamespace,
        _request: &herdr_threads::protocol::handoff::DeliveryMutation,
        _admission: herdr_threads::ports::DurableWorkAdmission,
        _budget: &CallBudget,
    ) -> Result<
        herdr_threads::ports::SendPreparationProgress,
        herdr_threads::protocol::results::ApiError,
    > {
        unreachable!("automatic route reached unrelated legacy adapter")
    }
    fn bootstrap_mutate(
        &self,
        _canonical: &herdr_threads::protocol::handoff::HandoffNamespace,
        _command: herdr_threads::protocol::commands::PermitMutation,
        _permit: herdr_threads::protocol::authority::MutationPermit,
        _guard: Option<herdr_threads::ports::BootstrapAttachmentGuard>,
        _budget: &CallBudget,
    ) -> Result<CommandResult, herdr_threads::protocol::results::ApiError> {
        unreachable!("automatic route reached unrelated legacy adapter")
    }
    fn bootstrap_query(
        &self,
        _canonical: &herdr_threads::protocol::handoff::HandoffNamespace,
        _command: &Command,
        _budget: &CallBudget,
    ) -> Result<CommandResult, herdr_threads::protocol::results::ApiError> {
        unreachable!("automatic route reached unrelated legacy adapter")
    }
    fn bootstrap_recovery_replay(
        &self,
        _canonical: &herdr_threads::protocol::handoff::HandoffNamespace,
        _request: &herdr_threads::protocol::handoff::RecoverBootstrap,
        _actor: &herdr_threads::protocol::authority::OperatorActor,
        _budget: &CallBudget,
    ) -> Result<
        Option<herdr_threads::protocol::handoff::BootstrapRecoveryResult>,
        herdr_threads::protocol::results::ApiError,
    > {
        unreachable!("automatic route reached unrelated legacy adapter")
    }
    fn bootstrap_recover(
        &self,
        _canonical: &herdr_threads::protocol::handoff::HandoffNamespace,
        _request: &herdr_threads::protocol::handoff::RecoverBootstrap,
        _actor: &herdr_threads::protocol::authority::OperatorActor,
        _guard: Option<herdr_threads::ports::BootstrapAttachmentGuard>,
        _budget: &CallBudget,
    ) -> Result<CommandResult, herdr_threads::protocol::results::ApiError> {
        unreachable!("automatic route reached unrelated legacy adapter")
    }
}

#[test]
fn automatic_generic_selected_runtime_defaults_refuse_without_effects() {
    let mut f = Fixture::new();
    let request = automatic_request(&f);
    let host = Arc::new(herdr_threads::host::native::NativeCli::new(
        "/host.sock".into(),
        f.clock.clone(),
    ));
    f.service = f
        .service
        .with_bootstrap_runtime(
            super::topology_handoff::namespace(),
            host,
            Arc::new(GenericBootstrap),
        )
        .unwrap();
    let before =
        f.db.query_row("SELECT count(*) FROM operations", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap();
    let mut status = request.clone();
    status.action = AutomaticDeliveryAction::Status;
    for phase in [status, request.clone(), request.begin(), request.ensure()] {
        assert_eq!(
            automatic_call(&f, phase).unwrap_err().code,
            herdr_threads::protocol::results::ErrorCode::Unsupported
        );
    }
    assert_eq!(
        f.db.query_row("SELECT count(*) FROM operations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        before
    );
    assert_eq!(
        f.db.query_row("SELECT count(*) FROM channel_handoff_fences", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        f.db.query_row("SELECT count(*) FROM direct_threads", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn automatic_canonical_historical_ensure_keeps_original_claim_namespace_and_fence() {
    use herdr_threads::protocol::results::ErrorCode;
    let f = Fixture::new();
    let request = automatic_request(&f);
    automatic_call(&f, request.begin()).unwrap();
    let result = automatic_call(&f, request.ensure()).unwrap();
    let thread = automatic_selected_thread(&result).unwrap();
    assert!(
        matches!(automatic_call(&f,request.begin()).unwrap(),CommandResult::Handoff(ref r) if r.thread.as_ref()==Some(thread))
    );
    f.db.execute(
        "UPDATE occupant_bindings SET ended_at=1 WHERE seat_id='s'",
        [],
    )
    .unwrap();
    assert_eq!(automatic_call(&f, request.ensure()).unwrap(), result);
    for completed in [false, true] {
        if completed {
            f.db.execute("UPDATE channel_handoff_fences SET state='completed',completed_at=1 WHERE compound='automatic-compound'",[]).unwrap();
        }
        let mut status = request.clone();
        status.action = AutomaticDeliveryAction::Status;
        assert!(
            matches!(automatic_call(&f,status).unwrap(),CommandResult::Handoff(ref r) if r.thread.as_ref()==Some(thread))
        );
        for field in ["claim", "namespace", "body"] {
            let mut forged = request.clone();
            match field {
                "claim" => forged.claim.binding_generation += 1,
                "namespace" => {
                    let AutomaticOriginal::Delivery(p) = &mut forged.original else {
                        panic!()
                    };
                    p.payload.namespace.state_dir = "/other".into();
                }
                _ => {
                    let AutomaticOriginal::Delivery(p) = &mut forged.original else {
                        panic!()
                    };
                    p.payload.body = "forged body".into();
                }
            }
            forged.digest = forged.original.digest(&forged.claim).unwrap();
            let error = automatic_call(&f, forged.ensure()).unwrap_err();
            assert!(matches!(
                error.code,
                ErrorCode::OperationPayloadMismatch | ErrorCode::InstanceMismatch
            ));
        }
        assert_eq!(automatic_call(&f, request.ensure()).unwrap(), result);
        assert_eq!(
            f.db.query_row("SELECT count(*) FROM invitations", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
}

#[test]
fn automatic_canonical_registration_refuses_cross_role_and_legacy_key_adoption() {
    use herdr_threads::protocol::{commands::CreateThread, ids::OperationId};
    let f = Fixture::new();
    let request = automatic_request(&f);
    automatic_call(&f, request.begin()).unwrap();
    let mut collision = request.clone();
    let AutomaticOriginal::Delivery(plan) = &mut collision.original else {
        panic!()
    };
    plan.payload.keys = AutomaticHandoffKeys::new(OperationId::new("other-compound"));
    plan.payload.keys.ensure = request.plan().payload.keys.send.clone();
    collision.digest = collision.original.digest(&collision.claim).unwrap();
    assert!(automatic_call(&f, collision.begin()).is_err());
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM automatic_handoff_companions",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        f.db.query_row("SELECT count(*) FROM automatic_handoff_keys", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        5
    );
    let mut legacy = request;
    let AutomaticOriginal::Delivery(plan) = &mut legacy.original else {
        panic!()
    };
    plan.payload.keys = AutomaticHandoffKeys::new(OperationId::new("legacy-compound"));
    legacy.digest = legacy.original.digest(&legacy.claim).unwrap();
    f.call(Command::CreateThread(CreateThread {
        name: None,
        topic: "ordinary".into(),
        goal: "manual".into(),
        operation: legacy.plan().payload.keys.ensure.clone(),
        claim: legacy.claim.clone(),
    }))
    .unwrap();
    assert!(automatic_call(&f, legacy.begin()).is_err());
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM automatic_handoff_companions",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        f.db.query_row("SELECT count(*) FROM direct_threads", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn automatic_canonical_complete_requires_published_frozen_send() {
    use herdr_threads::protocol::{
        commands::{OperatorRetire, Reject},
        ids::*,
    };
    let f = Fixture::new();
    let request = automatic_request(&f);
    let peer = automatic_peer_claim(&f, &request);
    automatic_call(&f, request.begin()).unwrap();
    let ensured = automatic_call(&f, request.ensure()).unwrap();
    let thread = automatic_selected_thread(&ensured).unwrap().clone();
    let assert_live_without_completion = || {
        assert_eq!(
            f.db.query_row(
                "SELECT state FROM channel_handoff_fences WHERE compound=?1",
                [request.plan().payload.keys.compound.as_str()],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "live"
        );
        assert_eq!(
            f.db.query_row(
                "SELECT count(*) FROM operations WHERE operation_key=?1",
                [request.plan().payload.keys.complete.as_str()],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    };
    assert!(
        automatic_call(&f, request.complete()).is_err(),
        "Ensure alone must not complete or release the live handoff fence"
    );
    assert_live_without_completion();
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM messages WHERE kind='ordinary'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let AutomaticDeliveryAction::Send(mut unrelated) = request.send(thread.clone()).action else {
        panic!("send")
    };
    unrelated.operation = OperationId::new("unrelated-send");
    unrelated.body = "unrelated work".into();
    assert!(matches!(
        f.call(Command::SendMessage(unrelated)).unwrap(),
        CommandResult::MessageSent(_)
    ));
    assert!(
        automatic_call(&f, request.complete()).is_err(),
        "a published ordinary message on this pair under another key is not the frozen Send"
    );
    assert_live_without_completion();
    assert!(matches!(
        automatic_call(&f, request.send(thread.clone())).unwrap(),
        CommandResult::MessageSent(_)
    ));
    let completed = automatic_call(&f, request.complete()).unwrap();
    assert!(
        matches!(completed, CommandResult::Handoff(ref status) if status.state == HandoffState::Completed)
    );
    let invitation: String =
        f.db.query_row(
            "SELECT id FROM invitations WHERE thread_id=?1",
            [thread.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    f.call(Command::Reject(Reject {
        thread: thread.clone(),
        invitation: InvitationId::new(invitation),
        reason: "declined after publication".into(),
        operation: OperationId::new("reject-after-complete"),
        claim: peer,
    }))
    .unwrap();
    f.call(Command::OperatorRetire(OperatorRetire {
        seat: request.claim.seat.clone(),
        operation: OperationId::new("retire-after-complete"),
    }))
    .unwrap();
    assert_eq!(automatic_call(&f, request.complete()).unwrap(), completed);
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM messages WHERE kind='ordinary'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
}

#[test]
fn automatic_canonical_complete_refuses_corrupt_publication_proof() {
    use herdr_threads::protocol::ids::*;
    for corruption in [
        "digest",
        "result_kind",
        "missing_message",
        "unrelated_message",
    ] {
        let f = Fixture::new();
        let request = automatic_request(&f);
        automatic_call(&f, request.begin()).unwrap();
        let ensured = automatic_call(&f, request.ensure()).unwrap();
        let thread = automatic_selected_thread(&ensured).unwrap().clone();
        let AutomaticDeliveryAction::Send(mut unrelated) = request.send(thread.clone()).action
        else {
            panic!("send")
        };
        unrelated.operation = OperationId::new("unrelated-send");
        unrelated.body = "unrelated work".into();
        let unrelated = f.call(Command::SendMessage(unrelated)).unwrap();
        automatic_call(&f, request.send(thread.clone())).unwrap();
        if corruption == "digest" {
            f.db.execute(
                "UPDATE operations SET digest=zeroblob(32) WHERE operation_key=?1",
                [request.plan().payload.keys.send.as_str()],
            )
            .unwrap();
        } else {
            let result = match corruption {
                "result_kind" => CommandResult::ThreadCreated(thread),
                "missing_message" => CommandResult::MessageSent(MessageId::new("missing-message")),
                "unrelated_message" => unrelated,
                _ => unreachable!(),
            };
            f.db.execute(
                "UPDATE operations SET result_json=?1 WHERE operation_key=?2",
                rusqlite::params![
                    serde_json::to_string(&result).unwrap(),
                    request.plan().payload.keys.send.as_str()
                ],
            )
            .unwrap();
        }
        assert!(
            automatic_call(&f, request.complete()).is_err(),
            "{corruption} must not prove publication of the frozen Send"
        );
        assert_eq!(
            f.db.query_row("SELECT state FROM channel_handoff_fences", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "live"
        );
        assert_eq!(
            f.db.query_row(
                "SELECT count(*) FROM operations WHERE operation_key=?1",
                [request.plan().payload.keys.complete.as_str()],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            f.db.query_row(
                "SELECT count(*) FROM messages WHERE kind='ordinary'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            2
        );
    }
}

#[test]
fn automatic_canonical_send_publishes_once_and_completed_replay_is_historical() {
    let f = Fixture::new();
    let request = automatic_request(&f);
    automatic_call(&f, request.begin()).unwrap();
    let ensured = automatic_call(&f, request.ensure()).unwrap();
    let thread = automatic_selected_thread(&ensured).unwrap().clone();
    let sent = automatic_call(&f, request.send(thread.clone())).unwrap();
    assert!(matches!(sent, CommandResult::MessageSent(_)), "{sent:?}");
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM messages WHERE kind='ordinary' AND body='work'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    let completed = automatic_call(&f, request.complete()).unwrap();
    assert!(matches!(completed, CommandResult::Handoff(ref v) if v.state==HandoffState::Completed));
    f.db.execute(
        "UPDATE memberships SET state='left',left_at=1 WHERE thread_id=?1 AND seat_id='peer'",
        [thread.as_str()],
    )
    .unwrap();
    f.db.execute("UPDATE occupant_bindings SET ended_at=1", [])
        .unwrap();
    assert_eq!(automatic_call(&f, request.send(thread)).unwrap(), sent);
    assert_eq!(automatic_call(&f, request.complete()).unwrap(), completed);
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM messages WHERE kind='ordinary'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

fn automatic_parent_identity() -> AutomaticBootstrapIdentity {
    let old = super::topology_handoff::identity();
    let plan = AutomaticBootstrapPlan {
        version: 2,
        payload: AutomaticBootstrapPayload {
            handoff: AutomaticHandoffPayload::new(
                old.payload.handoff.namespace.clone(),
                herdr_threads::protocol::ids::OperationId::new("automatic-child"),
                "work".into(),
            )
            .unwrap(),
            keys: AutomaticBootstrapKeys::new(herdr_threads::protocol::ids::OperationId::new(
                "automatic-parent",
            )),
            workspace: old.payload.workspace,
            cwd: old.payload.cwd,
            label: old.payload.label,
            focus: false,
            env: Default::default(),
            launch: old.payload.launch,
            startup_input: None,
        },
    };
    AutomaticBootstrapIdentity::new(old.scope, old.claim, plan).unwrap()
}

#[test]
fn automatic_canonical_bootstrap_public_begin_reserve_and_record_created() {
    let f = Fixture::new();
    let identity = automatic_parent_identity();
    let mut request = AutomaticBootstrapMutation {
        identity,
        action: AutomaticBootstrapAction::Begin,
    };
    let begin = f
        .call(Command::AutomaticBootstrap(Box::new(request.clone())))
        .unwrap();
    assert!(
        matches!(begin,CommandResult::AutomaticBootstrap(ref v) if v.state==BootstrapState::Prepared)
    );
    request.action = AutomaticBootstrapAction::Reserve {
        attempt: BootstrapAttempt::first(),
    };
    let reserved = f
        .call(Command::AutomaticBootstrap(Box::new(request.clone())))
        .unwrap();
    assert!(
        matches!(reserved,CommandResult::AutomaticBootstrapReserved(ref v) if v.authorization.is_some())
    );
    let replay = f
        .call(Command::AutomaticBootstrap(Box::new(request.clone())))
        .unwrap();
    assert!(
        matches!(replay,CommandResult::AutomaticBootstrapReserved(ref v) if v.authorization.is_none())
    );
    request.action = AutomaticBootstrapAction::CheckSubmission {
        attempt: BootstrapAttempt::first(),
        administrative_revision: 0,
    };
    assert!(matches!(
        f.call(Command::AutomaticBootstrap(Box::new(request.clone())))
            .unwrap(),
        CommandResult::BootstrapSubmissionChecked(_)
    ));
    request.action = AutomaticBootstrapAction::RecordNotSubmitted {
        attempt: BootstrapAttempt::first(),
    };
    let not_submitted = f
        .call(Command::AutomaticBootstrap(Box::new(request.clone())))
        .unwrap();
    assert!(
        matches!(not_submitted, CommandResult::AutomaticBootstrap(ref status)
        if status.attempt.get() == 2 && status.attempt_state == BootstrapAttemptState::Prepared)
    );
    assert_eq!(
        f.call(Command::AutomaticBootstrap(Box::new(request.clone())))
            .unwrap(),
        not_submitted
    );
    let next = BootstrapAttempt::new(2).unwrap();
    request.action = AutomaticBootstrapAction::Reserve { attempt: next };
    assert!(
        matches!(f.call(Command::AutomaticBootstrap(Box::new(request.clone()))).unwrap(),
        CommandResult::AutomaticBootstrapReserved(ref status) if status.authorization.is_some())
    );
    request.action = AutomaticBootstrapAction::CheckSubmission {
        attempt: next,
        administrative_revision: 0,
    };
    assert!(matches!(
        f.call(Command::AutomaticBootstrap(Box::new(request.clone())))
            .unwrap(),
        CommandResult::BootstrapSubmissionChecked(_)
    ));
    request.action = AutomaticBootstrapAction::RecordCreated {
        attempt: next,
        evidence: super::topology_handoff::created(),
    };
    let created = f
        .call(Command::AutomaticBootstrap(Box::new(request)))
        .unwrap();
    assert!(
        matches!(created,CommandResult::AutomaticBootstrap(ref v) if v.state==BootstrapState::Created && v.creation.is_some())
    );
    let json = serde_json::to_string(&begin).unwrap();
    assert!(!json.contains("invite_key"));
    assert!(!json.contains("create_key"));
}

#[test]
fn automatic_canonical_ordinary_send_uses_selected_bootstrap_preparer() {
    let f = Fixture::new();
    let request = automatic_request(&f);
    automatic_call(&f, request.begin()).unwrap();
    let ensured = automatic_call(&f, request.ensure()).unwrap();
    let thread = automatic_selected_thread(&ensured).unwrap().clone();
    let AutomaticDeliveryAction::Send(send) = request.send(thread).action else {
        panic!("send")
    };
    let result = f.call(Command::SendMessage(send.clone())).unwrap();
    assert!(matches!(result, CommandResult::MessageSent(_)));
    assert_eq!(f.call(Command::SendMessage(send)).unwrap(), result);
}

fn parent_call(
    f: &Fixture,
    id: &AutomaticBootstrapIdentity,
    action: AutomaticBootstrapAction,
) -> Result<CommandResult, herdr_threads::protocol::results::ApiError> {
    f.call(Command::AutomaticBootstrap(Box::new(
        AutomaticBootstrapMutation {
            identity: id.clone(),
            action,
        },
    )))
}
#[test]
fn automatic_canonical_unknown_recovery_is_inspected_one_use_and_operator_attributed() {
    let f = Fixture::new();
    let id = automatic_parent_identity();
    let attempt = BootstrapAttempt::first();
    parent_call(&f, &id, AutomaticBootstrapAction::Begin).unwrap();
    parent_call(&f, &id, AutomaticBootstrapAction::Reserve { attempt }).unwrap();
    let unknown = parent_call(
        &f,
        &id,
        AutomaticBootstrapAction::RecordOutcomeUnknown { attempt },
    )
    .unwrap();
    let CommandResult::AutomaticBootstrap(status) = unknown else {
        panic!("status")
    };
    assert_eq!(status.attempt_state, BootstrapAttemptState::OutcomeUnknown);
    assert!(
        parent_call(
            &f,
            &id,
            AutomaticBootstrapAction::RecordNotSubmitted { attempt }
        )
        .is_err(),
        "unknown submission cannot become transport-proven zero"
    );
    let recover = AutomaticBootstrapAction::Recover {
        attempt,
        inspection: status.recovery_inspection().unwrap(),
        disposition: AutomaticBootstrapRecoveryDisposition::NotCreated {
            quiescence: BootstrapQuiescenceAssertion::InspectedNoncreationAndQuiescence,
        },
    };
    let result = parent_call(&f, &id, recover.clone()).unwrap();
    let CommandResult::AutomaticBootstrap(ref status) = result else {
        panic!("status")
    };
    assert_eq!(status.attempt.get(), 2);
    assert_eq!(status.administrative_revision, 1);
    let saved = status.recovery.as_ref().unwrap();
    assert_eq!(saved.operator_uid, 501);
    assert_eq!(saved.operator_provenance, "operator:local-user:501");
    f.db.execute("UPDATE occupant_bindings SET ended_at=1", [])
        .unwrap();
    assert_eq!(parent_call(&f, &id, recover).unwrap(), result);
    assert!(
        parent_call(
            &f,
            &id,
            AutomaticBootstrapAction::CheckSubmission {
                attempt,
                administrative_revision: 0
            }
        )
        .is_err()
    );
}
#[test]
fn automatic_canonical_parent_blocks_archival_before_child_registration() {
    let f = Fixture::new();
    let id = automatic_parent_identity();
    parent_call(&f, &id, AutomaticBootstrapAction::Begin).unwrap();
    f.db.execute("UPDATE memberships SET state='left'", [])
        .unwrap();
    for at in [0, 60000, 120000] {
        let rt = herdr_threads::store::archival::Runtime {
            boot: "auto-archive".into(),
            mono: at,
            utc: herdr_threads::protocol::time::UtcMillis(at),
            after_ms: 60000,
            host_generation: 0,
            coherent: true,
            valid_until_mono: None,
            legacy_source: Some("covered".into()),
        };
        for _ in 0..20 {
            if !herdr_threads::store::archival::advance(&f.db, "i", &rt)
                .unwrap()
                .has_more
            {
                break;
            }
        }
    }
    assert!(
        !f.db
            .query_row("SELECT archived FROM threads WHERE id='t'", [], |r| r
                .get::<_, bool>(0))
            .unwrap(),
        "unattached original parent must protect possible creation"
    );
}

#[test]
fn automatic_canonical_recovery_public_noncreation_requires_inspected_state() {
    let f = Fixture::new();
    let id = automatic_parent_identity();
    let attempt = BootstrapAttempt::first();
    parent_call(&f, &id, AutomaticBootstrapAction::Begin).unwrap();
    let CommandResult::AutomaticBootstrapReserved(reserved) =
        parent_call(&f, &id, AutomaticBootstrapAction::Reserve { attempt }).unwrap()
    else {
        panic!("reserve")
    };
    let action = AutomaticBootstrapAction::Recover {
        attempt,
        inspection: reserved.status.recovery_inspection().unwrap(),
        disposition: AutomaticBootstrapRecoveryDisposition::NotCreated {
            quiescence: BootstrapQuiescenceAssertion::InspectedNoncreationAndQuiescence,
        },
    };
    let CommandResult::AutomaticBootstrap(status) = parent_call(&f, &id, action).unwrap() else {
        panic!("status")
    };
    assert_eq!(status.attempt.get(), 2);
    assert!(status.recovery.is_some());
}

#[test]
fn automatic_canonical_descriptor_requires_the_concrete_automatic_runtime() {
    use herdr_threads::daemon::control::{ControlService, StopController};
    for generic in [false, true] {
        let mut f = Fixture::new();
        if generic {
            let host = Arc::new(herdr_threads::host::native::NativeCli::new(
                "/host.sock".into(),
                f.clock.clone(),
            ));
            f.service = f
                .service
                .with_bootstrap_runtime(
                    super::topology_handoff::namespace(),
                    host,
                    Arc::new(GenericBootstrap),
                )
                .unwrap();
        }
        let budget = CallBudget {
            deadline: MonoInstant(f.clock.monotonic_now().0 + 5000),
            cancellation: Cancellation::default(),
        };
        let service = ControlService::new_guarded(
            StopController::new(
                uuid::Uuid::new_v4(),
                uuid::Uuid::new_v4(),
                Cancellation::default(),
            ),
            |_| -> herdr_threads::daemon::health::HealthInputs { panic!("health") },
            f.service,
        )
        .unwrap();
        let CommandResult::Capabilities(caps) = service
            .handle(
                Command::Capabilities,
                herdr_threads::test_support::peer_identity(501),
                &budget,
            )
            .unwrap()
        else {
            panic!("capabilities")
        };
        assert_eq!(
            caps.capabilities
                .iter()
                .any(|name| name == herdr_threads::protocol::capabilities::AUTOMATIC_HANDOFF_V1),
            !generic
        );
    }
}

fn automatic_peer_claim(
    f: &Fixture,
    request: &AutomaticDeliveryMutation,
) -> herdr_threads::protocol::authority::CallerClaim {
    use herdr_threads::protocol::ids::*;
    f.db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES('peer',1,'peer-pane','b',1,1,'codex','peer-session','00000000-0000-4000-8000-000000000002','cooperative_top_level',0,0,'term-peer','inc')",[]).unwrap();
    let mut claim = request.claim.clone();
    claim.seat = SeatId::new("peer");
    claim.target = HostTargetId::new("peer-pane");
    claim.native_session = NativeSessionId::new("peer-session");
    claim.execution = ExecutionId::new("00000000-0000-4000-8000-000000000002");
    claim
}
#[test]
fn automatic_canonical_lost_ensure_then_real_reject_or_leave_cannot_publish_or_reinvite() {
    use herdr_threads::protocol::{
        commands::{Accept, Leave, Reject},
        ids::*,
    };
    for phase in ["reject", "leave"] {
        let f = Fixture::new();
        let request = automatic_request(&f);
        let peer = automatic_peer_claim(&f, &request);
        automatic_call(&f, request.begin()).unwrap();
        let ensured = automatic_call(&f, request.ensure()).unwrap();
        let thread = automatic_selected_thread(&ensured).unwrap().clone();
        if phase == "reject" {
            let invitation: String =
                f.db.query_row(
                    "SELECT id FROM invitations WHERE thread_id=?1",
                    [thread.as_str()],
                    |r| r.get(0),
                )
                .unwrap();
            f.call(Command::Reject(Reject {
                thread: thread.clone(),
                invitation: InvitationId::new(invitation),
                reason: "declined".into(),
                operation: OperationId::new("reject-peer"),
                claim: peer,
            }))
            .unwrap();
        } else {
            f.call(Command::Accept(Accept {
                thread: thread.clone(),
                operation: OperationId::new("accept-peer"),
                claim: peer.clone(),
            }))
            .unwrap();
            f.call(Command::Leave(Leave {
                thread: thread.clone(),
                operation: OperationId::new("leave-peer"),
                claim: peer,
            }))
            .unwrap();
        }
        // Reject itself now appends a warning notice. Freeze the all-kind
        // baseline after that ordinary decision so retry gets no false credit.
        let messages_after_lifecycle: i64 =
            f.db.query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
                .unwrap();
        if phase == "reject" {
            assert_eq!(
                f.db.query_row(
                    "SELECT count(*) FROM messages WHERE thread_id=?1 AND kind='warn'",
                    [thread.as_str()],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                1
            );
        }
        assert_eq!(automatic_call(&f, request.ensure()).unwrap(), ensured);
        assert!(automatic_call(&f, request.send(thread)).is_err(), "{phase}");
        assert_eq!(
            f.db.query_row("SELECT count(*) FROM messages", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            messages_after_lifecycle,
            "{phase}: automatic retry must not add a message or notice"
        );
        assert_eq!(
            f.db.query_row("SELECT count(*) FROM invitations", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            f.db.query_row(
                "SELECT count(*) FROM messages WHERE kind='ordinary'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }
}
#[test]
fn automatic_canonical_real_reject_between_preparation_and_publication_refuses() {
    use herdr_threads::{
        ports::{BootstrapStorePort, DurableWorkAdmission, SendPreparationProgress},
        protocol::{
            commands::{PermitMutation, Reject},
            ids::*,
        },
    };
    let f = Fixture::new();
    let request = automatic_request(&f);
    let peer = automatic_peer_claim(&f, &request);
    automatic_call(&f, request.begin()).unwrap();
    let ensured = automatic_call(&f, request.ensure()).unwrap();
    let thread = automatic_selected_thread(&ensured).unwrap().clone();
    let send = request.send(thread.clone());
    let budget = CallBudget {
        deadline: MonoInstant(f.clock.monotonic_now().0 + 5000),
        cancellation: Cancellation::default(),
    };
    loop {
        match f
            .store
            .automatic_delivery_prepare_send_step(
                &request.plan().payload.namespace,
                &send,
                DurableWorkAdmission::new(16).unwrap(),
                &budget,
            )
            .unwrap()
        {
            SendPreparationProgress::Ready { .. } => break,
            SendPreparationProgress::More { .. } => {}
            SendPreparationProgress::Committed(_) => panic!("fresh preparation"),
        }
    }
    let invitation: String =
        f.db.query_row(
            "SELECT id FROM invitations WHERE thread_id=?1",
            [thread.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    f.call(Command::Reject(Reject {
        thread,
        invitation: InvitationId::new(invitation),
        reason: "declined".into(),
        operation: OperationId::new("reject-interleave"),
        claim: peer,
    }))
    .unwrap();
    let AutomaticDeliveryAction::Send(inner) = &send.action else {
        panic!("send")
    };
    let permit = f
        .store
        .issue_cooperative_permit(
            herdr_threads::store::cooperative_permit_request(&PermitMutation::SendMessage(
                inner.clone(),
            ))
            .unwrap(),
            &budget,
        )
        .unwrap();
    assert!(
        f.store
            .automatic_delivery_mutate(&request.plan().payload.namespace, &send, permit, &budget)
            .is_err()
    );
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM messages WHERE kind='ordinary'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}
#[test]
fn automatic_canonical_selected_runtime_keeps_active_unlinked_direct_send() {
    use herdr_threads::protocol::{
        commands::{DeliveryMode, EnsureDirectThread, SendMessage},
        ids::*,
    };
    let f = Fixture::new();
    let request = automatic_request(&f);
    let CommandResult::ThreadCreated(thread) = f
        .call(Command::EnsureDirectThread(EnsureDirectThread {
            creation_policy: None,
            recipient: request.plan().recipient.clone(),
            operation: OperationId::new("unlinked-ensure"),
            claim: request.claim.clone(),
        }))
        .unwrap()
    else {
        panic!("thread")
    };
    let send = SendMessage {
        user_intent: None,
        thread,
        body: "unlinked".into(),
        invited_recipients: vec![],
        direct_recipient: Some(request.plan().recipient.clone()),
        deadline_millis: None,
        operation: OperationId::new("unlinked-send"),
        claim: request.claim,
        delivery_mode: DeliveryMode::Ordinary,
        relays_user: false,
    };
    assert!(matches!(
        f.call(Command::SendMessage(send)).unwrap(),
        CommandResult::MessageSent(_)
    ));
}

#[test]
fn automatic_canonical_first_begin_admits_future_startup_before_any_effect() {
    use herdr_threads::{
        cli::launch::LaunchRequest,
        harness::{adapter::StartupInputTemplate, context::Harness as ContextHarness},
        protocol::{handoff::HandoffPolicy, ids::HostTargetId, output::ContinuationContext},
    };
    let f = Fixture::new();
    let request = automatic_request(&f);
    let mut delivery = request.plan().clone();
    delivery.payload.policy = Some(HandoffPolicy {
        deadline_millis: None,
        invitation_confirmation: herdr_threads::protocol::commands::InvitationConfirmation::Lazy,
        invite_deadline_millis: None,
    });
    let mut template = StartupInputTemplate::positional(0);
    template.max_arg_bytes = 1;
    let original = AutomaticOriginal::Handoff(Box::new(AutomaticHandoffPlan {
        version: 2,
        delivery,
        launch: LaunchRequest {
            target: HostTargetId::new("peer-pane"),
            harness: ContextHarness::Codex,
            harness_binary: None,
            argv: vec![],
            name: None,
            pane_label: None,
        },
        context: ContinuationContext {
            state_dir: Some("/state".into()),
            host: Some("/host.sock".into()),
        },
        startup_input: Some(template),
    }));
    let request = AutomaticDeliveryMutation::new(request.scope, request.claim, original).unwrap();
    let counts = || {
        [
            "operations",
            "channel_handoff_fences",
            "automatic_handoff_companions",
            "direct_threads",
            "invitations",
        ]
        .map(|table| {
            f.db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap()
        })
    };
    let before = counts();
    assert_eq!(
        automatic_call(&f, request.begin()).unwrap_err().code,
        herdr_threads::protocol::results::ErrorCode::InvalidRequest
    );
    assert_eq!(counts(), before);
}
