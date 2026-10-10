use herdr_threads::{
    app::{SystemClock, run_elected},
    client::{
        local::LocalSocketClient,
        service::{PersistentServiceClient, ServiceIntentJournal},
    },
    daemon::{
        ownership::OwnerLock,
        paths::{InstancePaths, RuntimeContext},
        transport::{read_frame, write_frame},
    },
    ports::{HostPort, LocalClient},
    protocol::{
        authority::{CallerClaim, CallerRole, Harness},
        commands::{
            AcceptRequired, Ack, CheckIn, CheckInMode, Command, InboxQuery, Leave, SendMessage,
            ServiceDisconnectRequest, SetTopic, ThreadMutation,
        },
        ids::{ExecutionId, HostTargetId, NativeSessionId, OperationId, SeatId, ThreadId},
        pagination::PageRequest,
        results::{ApiError, CommandResult, ErrorCode},
        service::{
            EnsureManagedThread, InvitationConstraint, NotificationSeverity, ReleaseRequirement,
            SERVICE_SESSION_CAPABILITY, ServiceInvite, ServiceNotify, ServiceOperation,
            ServiceRegister, ServiceRequest, ServiceResult, ServiceSetTopic, ServiceThreadMutation,
            ServiceWireRequest, ServiceWireResponse,
        },
        time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis},
        wire::PROTOCOL_VERSION,
    },
    service::config::ServiceConfig,
    store::connection::StoreContext,
};
use std::{
    fs,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use tokio::net::UnixStream;
use uuid::Uuid;

struct PausedHost(Arc<AtomicBool>);
struct TempRoot(PathBuf);
impl Drop for TempRoot {
    fn drop(&mut self) {
        super::print_daemon_logs_if_panicking(&self.0);
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct JournalDeadlineClock {
    root: PathBuf,
    expire_after_record: AtomicBool,
}
struct CompletionFaultClock {
    root: PathBuf,
    armed: AtomicBool,
}
impl Clock for CompletionFaultClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(1)
    }
    fn monotonic_now(&self) -> MonoInstant {
        if self.armed.load(Ordering::SeqCst)
            && fs::read_dir(&self.root)
                .unwrap()
                .flatten()
                .any(|entry| entry.file_name().to_string_lossy().ends_with(".intent"))
            && self.armed.swap(false, Ordering::SeqCst)
        {
            fs::set_permissions(&self.root, fs::Permissions::from_mode(0o500)).unwrap();
        }
        MonoInstant(1)
    }
}
impl Clock for JournalDeadlineClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(1)
    }
    fn monotonic_now(&self) -> MonoInstant {
        let pending = fs::read_dir(&self.root)
            .ok()
            .into_iter()
            .flatten()
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().ends_with(".intent"));
        MonoInstant(
            if self.expire_after_record.load(Ordering::SeqCst) && pending {
                5_001
            } else {
                1
            },
        )
    }
}
impl PausedHost {
    fn wait(&self) -> Result<(), ApiError> {
        while !self.0.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(5));
        }
        Err(ApiError::cancelled("fixture host released"))
    }
}
impl HostPort for PausedHost {
    fn observe_current_target_for_archival(
        &self,
        _: &herdr_threads::protocol::ids::HostTargetId,
        _: &herdr_threads::ports::HostCallContext,
    ) -> Result<herdr_threads::ports::ComposerObservation, herdr_threads::protocol::results::ApiError>
    {
        Err(herdr_threads::protocol::results::ApiError::unsupported(
            "test adapter has no composer-aware archival observation",
        ))
    }
    fn native_launch_capability(&self) -> herdr_threads::ports::NativeLaunchCapability {
        herdr_threads::ports::NativeLaunchCapability::Unsupported
    }
    fn observe_current_target(
        &self,
        _: &HostTargetId,
        _: &herdr_threads::ports::HostCallContext,
    ) -> Result<herdr_threads::ports::HostObservation, ApiError> {
        self.wait()?;
        unreachable!()
    }
    fn enumerate_targets(
        &self,
        _: &herdr_threads::ports::HostCallContext,
    ) -> Result<herdr_threads::ports::HostSnapshot, ApiError> {
        self.wait()?;
        unreachable!()
    }
    fn safe_wake_target(
        &self,
        _: &SeatId,
        _: &herdr_threads::ports::HostObservation,
    ) -> Option<herdr_threads::ports::SafeWakeTarget> {
        None
    }
    fn submit_prompt(
        &self,
        _: &herdr_threads::ports::SafeWakeTarget,
        _: &str,
        _: &herdr_threads::ports::HostCallContext,
    ) -> Result<herdr_threads::ports::PromptOutcome, ApiError> {
        unreachable!()
    }
    fn pane_agent_state(
        &self,
        _target: &herdr_threads::ports::SafeWakeTarget,
        _context: &herdr_threads::ports::HostCallContext,
    ) -> Result<herdr_threads::ports::AgentComposerState, herdr_threads::protocol::results::ApiError>
    {
        Ok(herdr_threads::ports::AgentComposerState::Submitted)
    }

    fn launch_native(
        &self,
        _: herdr_threads::ports::NativeLaunchRequest,
        _: &herdr_threads::ports::HostCallContext,
    ) -> Result<herdr_threads::ports::NativeLaunchOutcome, ApiError> {
        unreachable!()
    }
    fn send_submit_key(
        &self,
        _: &herdr_threads::ports::SafeWakeTarget,
        _: &herdr_threads::ports::HostCallContext,
    ) -> Result<(), herdr_threads::protocol::results::ApiError> {
        Ok(())
    }
}
struct Daemon {
    stop: Cancellation,
    release: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<std::io::Result<bool>>>,
}
impl Drop for Daemon {
    fn drop(&mut self) {
        self.stop.cancel();
        self.release.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Daemon {
    fn finish(mut self) {
        self.stop.cancel();
        self.release.store(true, Ordering::SeqCst);
        assert!(self.worker.take().unwrap().join().unwrap().unwrap());
    }
}

async fn raw_service_call(
    stream: &mut UnixStream,
    request: &ServiceWireRequest,
) -> ServiceWireResponse {
    write_frame(stream, &serde_json::to_vec(request).unwrap())
        .await
        .unwrap();
    let bytes = tokio::time::timeout(Duration::from_secs(3), read_frame(stream))
        .await
        .unwrap()
        .unwrap();
    let response: ServiceWireResponse = serde_json::from_slice(&bytes).unwrap();
    assert!(response.correlates_to(request));
    response
}

#[test]
fn registered_service_and_native_caller_complete_required_flow_with_exact_replay() {
    let _daemon_lock = super::IN_PROCESS_DAEMON
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let root = std::env::temp_dir().join(format!("herdr-graph-d2-{}", Uuid::new_v4()));
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    let _root_cleanup = TempRoot(root.clone());
    let paths = InstancePaths::resolve(
        &RuntimeContext::explicit(root.join("state"), root.join("host.sock"), None).unwrap(),
    )
    .unwrap();
    let owner = OwnerLock::acquire(&paths).unwrap();
    let instance = owner.instance_uuid();
    drop(owner);
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let db = StoreContext::new(paths.database_path.clone(), Arc::clone(&clock))
        .open_writer()
        .unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at,host_boot,host_epoch) VALUES (?1,0,'fixture-host',1)",
        [instance.to_string()],
    )
    .unwrap();
    drop(db);
    let release = Arc::new(AtomicBool::new(false));
    let host: Arc<dyn HostPort> = Arc::new(PausedHost(Arc::clone(&release)));
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let stop = Cancellation::default();
    let thread_stop = stop.clone();
    let daemon_paths = paths.clone();
    let daemon_clock = Arc::clone(&clock);
    let worker = std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(run_elected(
                &daemon_paths,
                daemon_clock,
                thread_stop,
                ServiceConfig::default(),
                host,
                move |descriptor| {
                    ready_tx
                        .send(descriptor.clone())
                        .map_err(std::io::Error::other)
                },
            ))
    });
    let daemon = Daemon {
        stop,
        release,
        worker: Some(worker),
    };
    let descriptor = ready_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("daemon endpoint did not publish");
    let client = PersistentServiceClient::new(
        descriptor.endpoint.clone(),
        Arc::clone(&clock),
        descriptor.instance_uuid,
        Some(descriptor.boot_id),
        ServiceIntentJournal::open(root.join("intents")).unwrap(),
    );
    let ordinary = LocalSocketClient::new(
        descriptor.endpoint.clone(),
        Arc::clone(&clock),
        descriptor.instance_uuid,
        Some(descriptor.boot_id),
    );
    let budget = || CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0 + 5_000),
        cancellation: Cancellation::default(),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let registration = client.register(&budget()).await.unwrap();
        let thread = ThreadId::new("graph-system");
        let (_, created) = client.submit(ServiceOperation::EnsureThread(EnsureManagedThread { thread: thread.clone(), topic: "system".into(), goal: "coordination".into(), operation: OperationId::new("create") }), &budget()).await.unwrap();
        assert!(matches!(created, ServiceResult::ThreadEnsured(_)));
        let db = rusqlite::Connection::open(&paths.database_path).unwrap();
        for index in 0..20 {
            let seat = SeatId::new(format!("member-{index}"));
            db.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES (?1,?2,'resolved','native',1,0)", rusqlite::params![seat.as_str(), instance.to_string()]).unwrap();
            db.execute("INSERT INTO memberships(thread_id,seat_id,state,voluntary_state) VALUES (?1,?2,'joined','joined')", rusqlite::params![thread.as_str(), seat.as_str()]).unwrap();
            db.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES (?1,?2,1,1)", rusqlite::params![thread.as_str(), seat.as_str()]).unwrap();
        }
        db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('pending',?1,'resolved','native','pane-pending',0,0,0)", [instance.to_string()]).unwrap();
        db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,occupancy,ui_state,top_level_occupant,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES (?1,'pane-pending','fixture-host',1,0,1,'fresh','unknown','unknown',0,0,'term-'||'pane-pending','inc','coherent_enumeration',1)", [instance.to_string()]).unwrap();
        let (_, invited) = client.submit(ServiceOperation::Invite(ServiceInvite { thread: thread.clone(), seat: SeatId::new("pending"), constraint: InvitationConstraint::Required, deadline_millis: Some(300_000), operation: OperationId::new("require") }), &budget()).await.unwrap();
        let ServiceResult::Invitation(invitation) = invited else { panic!("missing required invitation"); };
        let required = invitation.requirement.expect("required episode");
        assert_eq!(required.revision, 1);
        assert_eq!(required.state, herdr_threads::protocol::service::RequirementState::Pending);
        let native = CallerClaim {
            instance: instance.to_string(), seat: SeatId::new("pending"), binding_generation: 0,
            role: CallerRole::TopLevel, harness: Harness::Codex,
            native_session: NativeSessionId::new("plugin_context:pending"),
            execution: ExecutionId::new("00000000-0000-4000-8000-000000000111"),
            target: HostTargetId::new("pane-pending"),
        };
        let checked = std::thread::scope(|scope| scope.spawn(|| ordinary.call(Command::CheckIn(CheckIn {
            mode: CheckInMode::Lifecycle { expected_binding_generation: 0 },
            claim: native, operation: OperationId::new("native-check-in"),
        }), &budget())).join().unwrap()).unwrap();
        let CommandResult::CheckedIn(checked) = checked else { panic!("missing native check-in") };
        let native = checked.context;
        let mut service_claim = native.clone();
        service_claim.seat = SeatId::new(registration.author.as_str());
        let mut builtin_claim = native.clone();
        builtin_claim.seat = SeatId::new("built_in");
        for (key, claim) in [("claim-service-author", service_claim), ("claim-built-in-author", builtin_claim)] {
            let denied = std::thread::scope(|scope| scope.spawn(|| ordinary.call(Command::SendMessage(SendMessage {
        delivery_mode: herdr_threads::protocol::commands::DeliveryMode::Ordinary,
                thread: thread.clone(), body: "forged authority".into(), invited_recipients: vec![],
                deadline_millis: None, operation: OperationId::new(key), claim, relays_user: false, user_intent: None,
            }), &budget())).join().unwrap()).unwrap_err();
            assert_eq!(denied.code, ErrorCode::CallerUnverified, "{key}: {}", denied.detail);
            let staged: i64 = db.query_row("SELECT count(*) FROM operations WHERE operation_key=?1", [key], |row| row.get(0)).unwrap();
            assert_eq!(staged, 0, "denied authority claim staged an operation");
        }
        let forged_messages: i64 = db.query_row("SELECT count(*) FROM messages WHERE body='forged authority'", [], |row| row.get(0)).unwrap();
        assert_eq!(forged_messages, 0);
        // Simulate a concurrent authoritative revision advance after the native
        // caller read revision 1; the stale request still crosses the daemon.
        db.execute("UPDATE requirement_episodes SET revision=revision+1 WHERE id=?1 AND state='pending'", [required.requirement.as_str()]).unwrap();
        let stale = std::thread::scope(|scope| scope.spawn(|| ordinary.call(Command::AcceptRequired(AcceptRequired {
            thread: thread.clone(), invitation: invitation.invitation.clone(),
            requirement: required.requirement.clone(), expected_revision: required.revision,
            operation: OperationId::new("native-stale-required-accept"), claim: native.clone(),
        }), &budget())).join().unwrap()).unwrap_err();
        assert_eq!(stale.code, ErrorCode::StaleRequirementAcceptance);
        let (state, revision): (String, i64) = db.query_row("SELECT state,revision FROM requirement_episodes WHERE id=?1", [required.requirement.as_str()], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
        assert_eq!((state.as_str(), revision), ("pending", 2));
        let stale_staged: i64 = db.query_row("SELECT count(*) FROM operations WHERE operation_key='native-stale-required-accept'", [], |row| row.get(0)).unwrap();
        assert_eq!(stale_staged, 0);
        let accepted = std::thread::scope(|scope| scope.spawn(|| ordinary.call(Command::AcceptRequired(AcceptRequired {
            thread: thread.clone(), invitation: invitation.invitation.clone(),
            requirement: required.requirement.clone(), expected_revision: 2,
            operation: OperationId::new("native-required-accept"), claim: native.clone(),
        }), &budget())).join().unwrap()).unwrap();
        assert!(matches!(accepted, CommandResult::RequiredAccepted(ref state) if state.state == herdr_threads::protocol::service::RequirementState::Accepted && state.revision == 3));
        let refused = std::thread::scope(|scope| scope.spawn(|| ordinary.call(Command::Leave(Leave {
            thread: thread.clone(), operation: OperationId::new("native-required-leave"), claim: native.clone(),
        }), &budget())).join().unwrap()).unwrap_err();
        assert_eq!(refused.code, ErrorCode::MembershipRequired);
        for command in [
            Command::SetTopic(SetTopic { thread: thread.clone(), topic: "forged change".into(), operation: OperationId::new("native-managed-topic"), claim: native.clone() }),
            Command::Archive(ThreadMutation { thread: thread.clone(), operation: OperationId::new("native-managed-archive"), claim: native.clone() }),
        ] {
            let denied = std::thread::scope(|scope| scope.spawn(|| ordinary.call(command, &budget())).join().unwrap()).unwrap_err();
            assert_eq!(denied.code, ErrorCode::Unauthorized);
        }
        let (_, upgraded) = client.submit(ServiceOperation::Invite(ServiceInvite { thread: thread.clone(), seat: SeatId::new("member-0"), constraint: InvitationConstraint::Required, deadline_millis: Some(300_000), operation: OperationId::new("upgrade-joined") }), &budget()).await.unwrap();
        let ServiceResult::Invitation(upgraded) = upgraded else { panic!("missing joined upgrade"); };
        let response = std::thread::scope(|scope| scope.spawn(|| ordinary.call(Command::Inbox(InboxQuery { seat: Some(SeatId::new("member-0")), page: PageRequest::default() }), &budget())).join().unwrap()).unwrap();
        let CommandResult::Inbox(inbox) = response else { panic!("missing native inbox"); };
        let row = inbox.items.iter().find(|item| item.thread == thread).expect("joined member pending requirement absent from inbox");
        let pending = row.pending_requirement.as_ref().expect("joined member pending requirement lacks revision");
        assert_eq!(pending.requirement, upgraded.requirement.unwrap().requirement);
        assert_eq!(pending.revision, 1);
        let (_, result) = client.submit(ServiceOperation::Notify(ServiceNotify { thread: thread.clone(), severity: NotificationSeverity::Warn, event_json: serde_json::json!({"changed":"path","author_kind":"built_in","author_service_id":"forged","kind":"receipt_warning"}), operation: OperationId::new("notify") }), &budget()).await.unwrap();
        let ServiceResult::Notification(notice) = result else { panic!("missing notification"); };
        assert_eq!(notice.author, registration.author);
        let (author, kind, author_kind): (String, String, String) = db.query_row("SELECT author_service_id,kind,author_kind FROM messages WHERE id=?1", [notice.summary.message.as_str()], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap();
        assert_eq!(author, registration.author.as_str());
        assert_eq!(kind, "warn");
        assert_eq!(author_kind, "programmatic");
        let recipients: i64 = db.query_row("SELECT recipient_count FROM service_notification_publications WHERE message_id=?1", [notice.summary.message.as_str()], |row| row.get(0)).unwrap();
        assert_eq!(recipients, 21);
        let receipts: i64 = db.query_row("SELECT count(*) FROM receipts WHERE message_id=?1", [notice.summary.message.as_str()], |row| row.get(0)).unwrap();
        assert_eq!(receipts, 0);
        db.execute("UPDATE seats SET target_id='pane-member-1',target_generation=0 WHERE id='member-1'", []).unwrap();
        db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,occupancy,ui_state,top_level_occupant,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES (?1,'pane-member-1','fixture-host',1,0,1,'fresh','unknown','unknown',0,0,'term-'||'pane-member-1','inc','coherent_enumeration',1)", [instance.to_string()]).unwrap();
        let recipient_claim = CallerClaim {
            instance: instance.to_string(), seat: SeatId::new("member-1"), binding_generation: 1,
            role: CallerRole::TopLevel, harness: Harness::Codex,
            native_session: NativeSessionId::new("plugin_context:member-1"),
            execution: ExecutionId::new("00000000-0000-4000-8000-000000000112"),
            target: HostTargetId::new("pane-member-1"),
        };
        let checked_recipient = std::thread::scope(|scope| scope.spawn(|| ordinary.call(Command::CheckIn(CheckIn {
            mode: CheckInMode::Lifecycle { expected_binding_generation: 1 },
            claim: recipient_claim, operation: OperationId::new("recipient-check-in"),
        }), &budget())).join().unwrap()).unwrap();
        let CommandResult::CheckedIn(checked_recipient) = checked_recipient else { panic!("missing recipient check-in") };
        let sent = std::thread::scope(|scope| scope.spawn(|| ordinary.call(Command::SendMessage(SendMessage {
        delivery_mode: herdr_threads::protocol::commands::DeliveryMode::Ordinary,
            thread: thread.clone(), body: "native system mail".into(), invited_recipients: vec![],
            deadline_millis: None, operation: OperationId::new("native-system-send"), claim: native.clone(), relays_user: false, user_intent: None,
        }), &budget())).join().unwrap()).unwrap();
        let CommandResult::MessageSent(sent) = sent else { panic!("missing system message") };
        let (kind, actor, author_kind, author_service): (String, Option<String>, Option<String>, Option<String>) = db.query_row("SELECT kind,actor_seat_id,author_kind,author_service_id FROM messages WHERE id=?1", [sent.as_str()], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))).unwrap();
        assert_eq!((kind.as_str(), actor.as_deref(), author_kind.as_deref(), author_service.as_deref()), ("ordinary", Some("pending"), None, None));
        let manifest_recipients: i64 = db.query_row("SELECT recipient_count FROM send_manifests WHERE message_id=?1", [sent.as_str()], |row| row.get(0)).unwrap();
        assert_eq!(manifest_recipients, 20);
        let pending_override_rows: i64 = db.query_row("SELECT count(*) FROM receipt_state WHERE message_id=?1 AND seat_id='member-1'", [sent.as_str()], |row| row.get(0)).unwrap();
        assert_eq!(pending_override_rows, 0);
        let acked = std::thread::scope(|scope| scope.spawn(|| ordinary.call(Command::Ack(Ack {
            messages: vec![sent.clone()], operation: OperationId::new("native-system-ack"), claim: checked_recipient.context,
        }), &budget())).join().unwrap()).unwrap();
        assert!(matches!(acked, CommandResult::Acknowledged(_)));
        let (receipt_state, ack_actor, ack_observation): (String, Option<String>, Option<String>) = db.query_row("SELECT state,ack_actor_seat_id,ack_observation FROM receipt_state WHERE message_id=?1 AND seat_id='member-1'", [sent.as_str()], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap();
        assert_eq!((receipt_state.as_str(), ack_actor.as_deref()), ("acked", Some("member-1")));
        assert!(ack_observation.is_some());
        let service_receipts: i64 = db.query_row("SELECT count(*) FROM receipt_state WHERE message_id=?1", [notice.summary.message.as_str()], |row| row.get(0)).unwrap();
        assert_eq!(service_receipts, 0);
        db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('ordinary-populated',?1,'ordinary','ordinary',0,0)", [instance.to_string()]).unwrap();
        db.execute("INSERT INTO memberships(thread_id,seat_id,state,voluntary_state) VALUES ('ordinary-populated','member-0','joined','joined')", []).unwrap();
        db.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('ordinary-populated','member-0',1,1)", []).unwrap();
        db.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('ordinary-target',?1,'resolved','native',1,0)", [instance.to_string()]).unwrap();
        let ordinary_thread = ThreadId::new("ordinary-populated");
        let (_, ordinary_invited) = client.submit(ServiceOperation::Invite(ServiceInvite { thread: ordinary_thread.clone(), seat: SeatId::new("ordinary-target"), constraint: InvitationConstraint::Ordinary, deadline_millis: Some(300_000), operation: OperationId::new("ordinary-invite") }), &budget()).await.unwrap();
        assert!(matches!(ordinary_invited, ServiceResult::Invitation(_)));
        let required_denied = client.submit(ServiceOperation::Invite(ServiceInvite { thread: ordinary_thread.clone(), seat: SeatId::new("ordinary-target"), constraint: InvitationConstraint::Required, deadline_millis: Some(300_000), operation: OperationId::new("ordinary-required-denied") }), &budget()).await.unwrap_err();
        assert!(matches!(required_denied, herdr_threads::client::service::ServiceCallError::Api { error, .. } if error.code == ErrorCode::RequiredInvitationNeedsManagedThread));
        for operation in [
            ServiceOperation::SetTopic(ServiceSetTopic { thread: ordinary_thread.clone(), topic: "forged".into(), operation: OperationId::new("ordinary-topic-denied") }),
            ServiceOperation::Archive(ServiceThreadMutation { thread: ordinary_thread.clone(), operation: OperationId::new("ordinary-archive-denied") }),
        ] {
            let denied = client.submit(operation, &budget()).await.unwrap_err();
            assert!(matches!(denied, herdr_threads::client::service::ServiceCallError::Api { error, .. } if error.code == ErrorCode::IncompatibleOwnership));
        }
        client.disconnect().await;
        let disconnected = || std::thread::scope(|scope| scope.spawn(|| ordinary.call(Command::ServiceInspect, &budget())).join().unwrap()).unwrap();
        let until = Instant::now() + Duration::from_secs(3);
        loop {
            let CommandResult::ServiceInspection(state) = disconnected() else { panic!("missing service inspection") };
            if !state.connected { break; }
            assert!(Instant::now() < until, "service connection did not revoke");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let registration_request = ServiceWireRequest {
            version: PROTOCOL_VERSION, request_id: "raw-register".into(), expected_instance: instance.to_string(),
            service: ServiceRequest::Register(ServiceRegister { capability: SERVICE_SESSION_CAPABILITY.into() }),
        };
        let recovery_root = root.join("recovery-intents");
        let deadline_clock = Arc::new(JournalDeadlineClock { root: recovery_root.clone(), expire_after_record: AtomicBool::new(true) });
        let recovery_clock: Arc<dyn Clock> = deadline_clock.clone();
        let recovery_client = PersistentServiceClient::new(
            descriptor.endpoint.clone(), recovery_clock, descriptor.instance_uuid, Some(descriptor.boot_id),
            ServiceIntentJournal::open(&recovery_root).unwrap(),
        );
        let recovery_budget = || CallBudget { deadline: MonoInstant(5_000), cancellation: Cancellation::default() };
        assert_eq!(recovery_client.register(&recovery_budget()).await.unwrap().author, registration.author);
        let lost_operation = OperationId::new("response-lost-ensure");
        let failed = recovery_client.submit(ServiceOperation::EnsureThread(EnsureManagedThread {
            thread: ThreadId::new("response-loss-thread"), topic: "initial".into(), goal: "recovery".into(), operation: lost_operation.clone(),
        }), &recovery_budget()).await.unwrap_err();
        assert_eq!(failed.pending_operation(), Some(&lost_operation));
        assert_eq!(recovery_client.journal().pending().unwrap(), vec![lost_operation.clone()]);
        let saved = recovery_client.journal().inspect(&lost_operation).unwrap();
        assert_eq!(saved.author, registration.author);
        recovery_client.disconnect().await;
        let until = Instant::now() + Duration::from_secs(3);
        loop {
            let CommandResult::ServiceInspection(state) = disconnected() else { panic!("missing service inspection") };
            if !state.connected { break; }
            assert!(Instant::now() < until, "unsent recovery client did not revoke");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let mut raw = UnixStream::connect(&descriptor.endpoint).await.unwrap();
        assert!(matches!(raw_service_call(&mut raw, &registration_request).await.result, Ok(ServiceResult::Registered(ref state)) if state.author == registration.author));
        write_frame(&mut raw, &serde_json::to_vec(&saved.request).unwrap()).await.unwrap();
        let until = Instant::now() + Duration::from_secs(3);
        while db.query_row("SELECT count(*) FROM operations WHERE operation_key=?1", [lost_operation.as_str()], |row| row.get::<_,i64>(0)).unwrap() == 0 {
            assert!(Instant::now() < until, "response-loss mutation did not commit");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        drop(raw); // caller never observes the committed response
        let until = Instant::now() + Duration::from_secs(3);
        loop {
            let CommandResult::ServiceInspection(state) = disconnected() else { panic!("missing service inspection") };
            if !state.connected { break; }
            assert!(Instant::now() < until, "ambiguous socket did not revoke");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        deadline_clock.expire_after_record.store(false, Ordering::SeqCst);
        assert_eq!(recovery_client.register(&recovery_budget()).await.unwrap().author, registration.author);
        let replayed = recovery_client.replay(&lost_operation, &recovery_budget()).await.unwrap();
        assert!(matches!(replayed, ServiceResult::ThreadEnsured(ref state) if state.thread.as_str() == "response-loss-thread"));
        assert_eq!(recovery_client.journal().inspect_completed(&lost_operation).unwrap().saved.request, saved.request);
        assert!(recovery_client.journal().pending().unwrap().is_empty());
        assert_eq!(db.query_row("SELECT count(*) FROM operations WHERE operation_key=?1", [lost_operation.as_str()], |row| row.get::<_,i64>(0)).unwrap(), 1);
        assert_eq!(db.query_row("SELECT count(*) FROM threads WHERE id='response-loss-thread'", [], |row| row.get::<_,i64>(0)).unwrap(), 1);
        recovery_client.disconnect().await;
        let until = Instant::now() + Duration::from_secs(3);
        loop {
            let CommandResult::ServiceInspection(state) = disconnected() else { panic!("missing service inspection") };
            if !state.connected { break; }
            assert!(Instant::now() < until, "replay socket did not revoke");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let completion_root = root.join("completion-fault-intents");
        let completion_clock = Arc::new(CompletionFaultClock { root: completion_root.clone(), armed: AtomicBool::new(false) });
        let completion_client = PersistentServiceClient::new(
            descriptor.endpoint.clone(), completion_clock.clone(), descriptor.instance_uuid, Some(descriptor.boot_id),
            ServiceIntentJournal::open(&completion_root).unwrap(),
        );
        let completion_budget = || CallBudget { deadline: MonoInstant(5_000), cancellation: Cancellation::default() };
        assert_eq!(completion_client.register(&completion_budget()).await.unwrap().author, registration.author);
        completion_clock.armed.store(true, Ordering::SeqCst);
        let completion_key = OperationId::new("received-response-local-completion-failed");
        let completion_failure = completion_client.submit(ServiceOperation::EnsureThread(EnsureManagedThread {
            thread: ThreadId::new("completion-fault-thread"), topic: "created once".into(),
            goal: "durable response".into(), operation: completion_key.clone(),
        }), &completion_budget()).await.unwrap_err();
        let (reported_key, reported_result) = completion_failure.definitive_completion().expect("daemon response was not received before local completion failure");
        assert_eq!(reported_key, &completion_key);
        assert!(matches!(reported_result, Ok(ServiceResult::ThreadEnsured(state)) if state.thread.as_str() == "completion-fault-thread"));
        assert_eq!(db.query_row("SELECT count(*) FROM operations WHERE operation_key=?1", [completion_key.as_str()], |row| row.get::<_,i64>(0)).unwrap(), 1);
        assert_eq!(db.query_row("SELECT count(*) FROM threads WHERE id='completion-fault-thread'", [], |row| row.get::<_,i64>(0)).unwrap(), 1);
        fs::set_permissions(&completion_root, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(completion_client.journal().pending().unwrap(), vec![completion_key.clone()]);
        let original = completion_client.journal().inspect(&completion_key).unwrap();
        let replayed = completion_client.replay(&completion_key, &completion_budget()).await.unwrap();
        assert!(matches!(replayed, ServiceResult::ThreadEnsured(ref state) if state.thread.as_str() == "completion-fault-thread"));
        assert_eq!(completion_client.journal().inspect_completed(&completion_key).unwrap().saved.request, original.request);
        assert!(completion_client.journal().pending().unwrap().is_empty());
        assert_eq!(db.query_row("SELECT count(*) FROM operations WHERE operation_key=?1", [completion_key.as_str()], |row| row.get::<_,i64>(0)).unwrap(), 1);
        assert_eq!(db.query_row("SELECT count(*) FROM threads WHERE id='completion-fault-thread'", [], |row| row.get::<_,i64>(0)).unwrap(), 1);
        completion_client.disconnect().await;
        let until = Instant::now() + Duration::from_secs(3);
        loop {
            let CommandResult::ServiceInspection(state) = disconnected() else { panic!("missing service inspection") };
            if !state.connected { break; }
            assert!(Instant::now() < until, "completion replay socket did not revoke");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let reconnected = client.register(&budget()).await.unwrap();
        assert_eq!(reconnected.author, registration.author);
        assert!(reconnected.connection_generation > registration.connection_generation);
        let inspected = std::thread::scope(|scope| scope.spawn(|| ordinary.call(Command::ServiceInspect, &budget())).join().unwrap()).unwrap();
        let CommandResult::ServiceInspection(inspected) = inspected else { panic!("missing service inspection") };
        assert!(inspected.connected);
        assert_eq!(inspected.connection_generation, Some(reconnected.connection_generation));
        let stale = std::thread::scope(|scope| scope.spawn(|| ordinary.call(Command::ServiceDisconnect(ServiceDisconnectRequest { expected_boot: registration.daemon_boot.clone(), expected_generation: registration.connection_generation }), &budget())).join().unwrap()).unwrap_err();
        assert_eq!(stale.code, ErrorCode::StaleServiceGeneration);
        let (_, released) = client
            .submit(
                ServiceOperation::ReleaseRequirement(ReleaseRequirement {
                    thread,
                    seat: SeatId::new("pending"),
                    requirement: required.requirement,
                    operation: OperationId::new("release"),
                }),
                &budget(),
            )
            .await
            .unwrap();
        let ServiceResult::RequirementReleased(released) = released else {
            panic!("missing requirement release");
        };
        assert_eq!(released.state, herdr_threads::protocol::service::RequirementState::Released);
        assert_eq!(client.journal().inspect_completed(&OperationId::new("release")).unwrap().response.result, Ok(ServiceResult::RequirementReleased(released.clone())));
        let left = std::thread::scope(|scope| scope.spawn(|| ordinary.call(Command::Leave(Leave {
            thread: ThreadId::new("graph-system"), operation: OperationId::new("native-voluntary-leave"), claim: native,
        }), &budget())).join().unwrap()).unwrap();
        assert!(matches!(left, CommandResult::Left(_)));
    });
    daemon.finish();
}
