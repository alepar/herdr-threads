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
            .with_bootstrap_runtime(super::topology_handoff::namespace(), host, concrete)
            .unwrap();
        Self {
            service,
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
        thread: request.plan.payload.channel.thread().unwrap().clone(),
        body: request.plan.payload.body.clone(),
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
                thread: request.plan.payload.channel.thread().unwrap().clone(),
                seat: request.plan.recipient.clone(),
                deadline_millis: None,
                claim: request.claim.clone(),
                operation: request.plan.payload.keys.invite.clone(),
            })
        } else {
            DeliveryAction::Send(herdr_threads::protocol::commands::SendMessage {
                thread: request.plan.payload.channel.thread().unwrap().clone(),
                body: request.plan.payload.body.clone(),
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
