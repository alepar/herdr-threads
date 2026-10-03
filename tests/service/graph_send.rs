//! One real-daemon flow for the service-authored, ACK-required request queue
//! (ht-5nb.4): register v2, managed thread, required invite and accept, a
//! deadline send that wakes an idle recipient, an exact-id native ACK read back
//! through Receipts, a native reply read back through History, retirement
//! settlement, response-loss replay without a duplicate, and a v1 session that
//! cannot send.

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
    ports::{
        AgentComposerState, HostCallContext, HostObservation, HostPort, HostSnapshot, HostUiState,
        IncarnationEvidence, NativeLaunchCapability, NativeLaunchOutcome, NativeLaunchRequest,
        ObservationProvenance, PromptOutcome, SafeWakeTarget, StructuralOccupancy, WakeTargetBasis,
    },
    protocol::{
        authority::{CallerClaim, CallerRole, Harness},
        commands::{
            AcceptRequired, Ack, CheckIn, CheckInMode, Command, HistoryRange, OperatorRetire,
            SendMessage,
        },
        ids::{
            ExecutionId, HostBootId, HostCallId, HostTargetId, NativeSessionId, OperationId,
            SeatId, TerminalId, ThreadId,
        },
        pagination::PageRequest,
        results::{ApiError, CommandResult, ErrorCode, ReceiptStatus},
        service::{
            EnsureManagedThread, InvitationConstraint, RequirementState,
            SERVICE_SESSION_CAPABILITY, SERVICE_SESSION_CAPABILITY_V2, ServiceHistoryQuery,
            ServiceInvite, ServiceMembershipQuery, ServiceOperation, ServiceReceiptsQuery,
            ServiceRegister, ServiceRequest, ServiceResult, ServiceSend, ServiceWireRequest,
            ServiceWireResponse,
        },
        time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis},
        wire::PROTOCOL_VERSION,
    },
    service::config::ServiceConfig,
    store::connection::StoreContext,
};
use std::{
    fs,
    os::unix::fs::DirBuilderExt,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use tokio::net::UnixStream;
use uuid::Uuid;

struct TempRoot(PathBuf);
impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct JournalDeadlineClock {
    root: PathBuf,
    expire_after_record: AtomicBool,
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

/// A host that answers the way an idle, coherent agent pane does and records
/// every wake prompt it is asked to submit.
struct WakeHost {
    prompts: Arc<Mutex<Vec<(HostTargetId, String)>>>,
    /// Until armed the pane is not a safe wake target, so the daemon refuses
    /// (sends nothing) and retries; every prompt recorded after arming
    /// therefore answers attention that was pending when the test armed it.
    armed: Arc<AtomicBool>,
    sequence: AtomicU64,
}
impl WakeHost {
    fn observation(&self, target: &HostTargetId) -> HostObservation {
        let sequence = self.sequence.fetch_add(1, Ordering::SeqCst) + 1;
        let at = MonoInstant(sequence);
        HostObservation {
            focused: false,
            target: target.clone(),
            host_boot: HostBootId::new("fixture-host"),
            epoch: 1,
            generation: 0,
            observed_at_utc: UtcMillis(0),
            observed_at_mono: at,
            provenance: ObservationProvenance::FreshCurrentTarget,
            occupant: None,
            ui: HostUiState::Idle,
            terminal: Some(TerminalId::new(format!("term-{}", target.as_str()))),
            occupancy: StructuralOccupancy::Occupied,
            incarnation: IncarnationEvidence::Verified {
                identity: "inc".into(),
                evidence_kind: herdr_threads::ports::EvidenceKind::NativeCurrentTarget,
            },
            execution: herdr_threads::ports::ExecutionEvidence::Unknown,
            call_id: HostCallId::new(format!("wake-{sequence}")),
            connection_epoch: 1,
            observation_sequence: sequence,
            started_at_mono: at,
            completed_at_mono: at,
        }
    }
}
impl HostPort for WakeHost {
    fn native_launch_capability(&self) -> NativeLaunchCapability {
        NativeLaunchCapability::Unsupported
    }
    fn observe_current_target(
        &self,
        target: &HostTargetId,
        _: &HostCallContext,
    ) -> Result<HostObservation, ApiError> {
        Ok(self.observation(target))
    }
    fn enumerate_targets(&self, _: &HostCallContext) -> Result<HostSnapshot, ApiError> {
        Err(ApiError::unsupported(
            "fixture has no enumeration authority",
        ))
    }
    fn safe_wake_target(
        &self,
        seat: &SeatId,
        observation: &HostObservation,
    ) -> Option<SafeWakeTarget> {
        if !self.armed.load(Ordering::SeqCst) {
            return None;
        }
        let IncarnationEvidence::Verified { identity, .. } = &observation.incarnation else {
            return None;
        };
        Some(SafeWakeTarget {
            seat: seat.clone(),
            target: observation.target.clone(),
            host_boot: observation.host_boot.clone(),
            generation: observation.generation,
            terminal: observation.terminal.clone()?,
            incarnation: identity.clone(),
            basis: WakeTargetBasis::CooperativeAgent,
            epoch: observation.epoch,
            observation_sequence: observation.observation_sequence,
            bound_harness: None,
        })
    }
    fn submit_prompt(
        &self,
        target: &SafeWakeTarget,
        text: &str,
        _: &HostCallContext,
    ) -> Result<PromptOutcome, ApiError> {
        self.prompts
            .lock()
            .unwrap()
            .push((target.target.clone(), text.to_owned()));
        Ok(PromptOutcome::Submitted)
    }
    fn pane_agent_state(
        &self,
        _: &SafeWakeTarget,
        _: &HostCallContext,
    ) -> Result<AgentComposerState, ApiError> {
        Ok(AgentComposerState::Submitted)
    }
    fn launch_native(
        &self,
        _: NativeLaunchRequest,
        _: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, ApiError> {
        unreachable!()
    }
    fn send_submit_key(&self, _: &SafeWakeTarget, _: &HostCallContext) -> Result<(), ApiError> {
        Ok(())
    }
}

struct Daemon {
    stop: Cancellation,
    worker: Option<std::thread::JoinHandle<std::io::Result<bool>>>,
}
impl Drop for Daemon {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Daemon {
    fn finish(mut self) {
        self.stop.cancel();
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

fn wire(instance: &str, id: &str, service: ServiceRequest) -> ServiceWireRequest {
    ServiceWireRequest {
        version: PROTOCOL_VERSION,
        request_id: id.into(),
        expected_instance: instance.into(),
        service,
    }
}

fn wait_until(what: &str, mut ready: impl FnMut() -> bool) {
    // Refused wakes retry on a doubling backoff, so allow several of its steps.
    let until = Instant::now() + Duration::from_secs(10);
    while !ready() {
        assert!(Instant::now() < until, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn service_ack_required_request_flow() {
    let _daemon_lock = super::IN_PROCESS_DAEMON
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let root = std::env::temp_dir().join(format!("herdr-graph-send-{}", Uuid::new_v4()));
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
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let armed = Arc::new(AtomicBool::new(false));
    let host: Arc<dyn HostPort> = Arc::new(WakeHost {
        prompts: Arc::clone(&prompts),
        armed: Arc::clone(&armed),
        sequence: AtomicU64::new(0),
    });
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
    let native = |command: Command| -> Result<CommandResult, ApiError> {
        use herdr_threads::ports::LocalClient;
        std::thread::scope(|scope| {
            scope
                .spawn(|| ordinary.call(command, &budget()))
                .join()
                .unwrap()
        })
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let db = rusqlite::Connection::open(&paths.database_path).unwrap();
        let count = |sql: &str, key: &str| -> i64 {
            db.query_row(sql, [key], |row| row.get(0)).unwrap()
        };
        let seed_seat = |seat: &str, target: Option<&str>| {
            match target {
                Some(target) => {
                    db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES (?1,?2,'resolved','native',?3,0,0,0)", rusqlite::params![seat, instance.to_string(), target]).unwrap();
                    db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,occupancy,ui_state,top_level_occupant,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES (?1,?2,'fixture-host',1,0,1,'fresh','unknown','unknown',0,0,'term-'||?2,'inc','coherent_enumeration',1)", rusqlite::params![instance.to_string(), target]).unwrap();
                }
                None => {
                    db.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES (?1,?2,'resolved','native',1,0)", rusqlite::params![seat, instance.to_string()]).unwrap();
                }
            }
        };

        // 1. Register v2, ensure a managed thread, and send a required invite.
        let registration = client.register(&budget()).await.unwrap();
        let thread = ThreadId::new("graph-requests");
        let (_, ensured) = client
            .submit(
                ServiceOperation::EnsureThread(EnsureManagedThread {
                    thread: thread.clone(),
                    topic: "requests".into(),
                    goal: "ack-required requests".into(),
                    operation: OperationId::new("ensure"),
                }),
                &budget(),
            )
            .await
            .unwrap();
        assert!(matches!(ensured, ServiceResult::ThreadEnsured(_)));
        seed_seat("worker", Some("pane-worker"));
        let (_, invited) = client
            .submit(
                ServiceOperation::Invite(ServiceInvite {
                    thread: thread.clone(),
                    seat: SeatId::new("worker"),
                    constraint: InvitationConstraint::Required,
                    deadline_millis: Some(300_000),
                    operation: OperationId::new("invite"),
                }),
                &budget(),
            )
            .await
            .unwrap();
        let ServiceResult::Invitation(invitation) = invited else {
            panic!("missing required invitation");
        };
        let required = invitation.requirement.clone().expect("required episode");
        assert_eq!(required.state, RequirementState::Pending);

        // 2. The native seat checks in and accepts explicitly.
        let claim = CallerClaim {
            instance: instance.to_string(),
            seat: SeatId::new("worker"),
            binding_generation: 0,
            role: CallerRole::TopLevel,
            harness: Harness::Codex,
            native_session: NativeSessionId::new("plugin_context:worker"),
            execution: ExecutionId::new("00000000-0000-4000-8000-000000000211"),
            target: HostTargetId::new("pane-worker"),
        };
        let CommandResult::CheckedIn(checked) = native(Command::CheckIn(CheckIn {
            mode: CheckInMode::Lifecycle {
                expected_binding_generation: 0,
            },
            claim,
            operation: OperationId::new("worker-check-in"),
        }))
        .unwrap() else {
            panic!("missing check-in");
        };
        let worker = checked.context;
        let accepted = native(Command::AcceptRequired(AcceptRequired {
            thread: thread.clone(),
            invitation: invitation.invitation.clone(),
            requirement: required.requirement.clone(),
            expected_revision: required.revision,
            operation: OperationId::new("worker-accept"),
            claim: worker.clone(),
        }))
        .unwrap();
        assert!(
            matches!(accepted, CommandResult::RequiredAccepted(ref state) if state.state == RequirementState::Accepted),
            "{accepted:?}"
        );

        // 3. The service sends with a deadline; the idle recipient is woken.
        // The pane refuses wakes until armed below, after the send committed, so
        // the prompt recorded afterwards answers the send's attention.
        assert!(prompts.lock().unwrap().is_empty());
        let (_, sent) = client
            .send(
                ServiceSend {
                    thread: thread.clone(),
                    body: "do X".into(),
                    recipients: vec![],
                    deadline_millis: Some(600_000),
                    operation: OperationId::new("req-1"),
                },
                &budget(),
            )
            .await
            .unwrap();
        assert_eq!(sent.recipient_count, 1);
        assert_eq!(sent.author, registration.author);
        let req_1 = sent.summary.message.clone();
        armed.store(true, Ordering::SeqCst);
        wait_until("the native wake prompt", || {
            prompts
                .lock()
                .unwrap()
                .iter()
                .any(|(target, _)| target.as_str() == "pane-worker")
        });
        wait_until("a completed wake attempt", || {
            db.query_row(
                "SELECT count(*) FROM wake_work WHERE seat_id='worker' AND completed_at_utc IS NOT NULL AND last_outcome IS NOT NULL",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
                == 1
        });
        assert!(
            prompts
                .lock()
                .unwrap()
                .iter()
                .all(|(target, _)| target.as_str() == "pane-worker"),
            "only the idle recipient is woken"
        );

        // 4. The native seat ACKs by exact id; Receipts shows the ACK time.
        let before_ack = client
            .receipts(
                ServiceReceiptsQuery {
                    message: req_1.clone(),
                    page: PageRequest::default(),
                },
                &budget(),
            )
            .await
            .unwrap();
        assert_eq!(before_ack.delivery.acknowledged, 0);
        assert_eq!(before_ack.recipients.items[0].status, ReceiptStatus::Pending);
        let acked = native(Command::Ack(Ack {
            messages: vec![req_1.clone()],
            operation: OperationId::new("worker-ack"),
            claim: worker.clone(),
        }))
        .unwrap();
        assert!(matches!(acked, CommandResult::Acknowledged(_)), "{acked:?}");
        let receipts = client
            .receipts(
                ServiceReceiptsQuery {
                    message: req_1.clone(),
                    page: PageRequest::default(),
                },
                &budget(),
            )
            .await
            .unwrap();
        assert_eq!(receipts.delivery.acknowledged, 1);
        let [recipient] = receipts.recipients.items.as_slice() else {
            panic!("one recipient expected: {receipts:?}");
        };
        assert_eq!(recipient.seat.as_str(), "worker");
        assert_eq!(recipient.status, ReceiptStatus::Acknowledged);
        let provenance = recipient.ack_provenance.as_ref().expect("ack provenance");
        assert_eq!(provenance.actor.as_str(), "worker");
        assert!(provenance.decided_at.0 > 0);

        // 5. The native seat replies; History After{seq} sees it.
        let CommandResult::MessageSent(reply) = native(Command::SendMessage(SendMessage {
            thread: thread.clone(),
            body: "DONE".into(),
            invited_recipients: vec![],
            deadline_millis: None,
            operation: OperationId::new("worker-reply"),
            claim: worker.clone(),
            relays_user: false,
        }))
        .unwrap() else {
            panic!("missing reply");
        };
        let history = client
            .history(
                ServiceHistoryQuery {
                    thread: thread.clone(),
                    page: PageRequest::default(),
                    initial: Some(HistoryRange::After {
                        sequence: sent.summary.sequence,
                    }),
                },
                &budget(),
            )
            .await
            .unwrap();
        let seen = history
            .items
            .iter()
            .find(|item| item.message == reply)
            .expect("reply absent from History After");
        assert_eq!(seen.author.as_ref().map(SeatId::as_str), Some("worker"));
        assert!(
            history.items.iter().all(|item| item.sequence > sent.summary.sequence),
            "After{{seq}} excludes the request itself"
        );

        // 6. A second send reaches `worker` and `retiree`; retiring `retiree`
        // settles its obligation as recipient_retired.
        seed_seat("retiree", None);
        db.execute("INSERT INTO memberships(thread_id,seat_id,state,voluntary_state) VALUES (?1,'retiree','joined','joined')", [thread.as_str()]).unwrap();
        db.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES (?1,'retiree',1,1)", [thread.as_str()]).unwrap();
        let (_, second) = client
            .send(
                ServiceSend {
                    thread: thread.clone(),
                    body: "do Y".into(),
                    recipients: vec![],
                    deadline_millis: Some(600_000),
                    operation: OperationId::new("req-2"),
                },
                &budget(),
            )
            .await
            .unwrap();
        assert_eq!(second.recipient_count, 2);
        let req_2 = second.summary.message.clone();
        assert!(matches!(
            native(Command::OperatorRetire(OperatorRetire {
                seat: SeatId::new("retiree"),
                operation: OperationId::new("retire-retiree"),
            }))
            .unwrap(),
            CommandResult::OperatorRetired(_)
        ));
        let retirement_state = || -> String {
            format!(
                "{:?}",
                db.query_row(
                    "SELECT status,phase,last_error FROM retirements WHERE seat_id='retiree'",
                    [],
                    |row| Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?
                    ))
                )
                .ok()
            )
        };
        // The retirement job is advanced by the deadline lane, which falls back
        // to its 5 s safety tick between slices: allow more than one tick.
        let until = Instant::now() + Duration::from_secs(20);
        while !retirement_state().contains("\"complete\", \"complete\"") {
            assert!(
                Instant::now() < until,
                "timed out waiting for the retirement job: {}",
                retirement_state()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let settled = client
            .receipts(
                ServiceReceiptsQuery {
                    message: req_2.clone(),
                    page: PageRequest::default(),
                },
                &budget(),
            )
            .await
            .unwrap();
        let state_of = |seat: &str| {
            settled
                .recipients
                .items
                .iter()
                .find(|recipient| recipient.seat.as_str() == seat)
                .unwrap_or_else(|| panic!("{seat} missing from {settled:?}"))
        };
        assert_eq!(state_of("retiree").status, ReceiptStatus::Retired);
        assert!(state_of("retiree").retirement_cutover.is_some());
        assert_eq!(state_of("worker").status, ReceiptStatus::Pending);

        // 7. Response loss between commit and response, then replay.
        client.disconnect().await;
        let wait_revoked = || {
            let until = Instant::now() + Duration::from_secs(3);
            loop {
                let CommandResult::ServiceInspection(state) =
                    native(Command::ServiceInspect).unwrap()
                else {
                    panic!("missing service inspection");
                };
                if !state.connected {
                    break;
                }
                assert!(Instant::now() < until, "service connection did not revoke");
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        wait_revoked();
        let recovery_root = root.join("recovery-intents");
        let deadline_clock = Arc::new(JournalDeadlineClock {
            root: recovery_root.clone(),
            expire_after_record: AtomicBool::new(true),
        });
        let recovery_clock: Arc<dyn Clock> = deadline_clock.clone();
        let recovery_client = PersistentServiceClient::new(
            descriptor.endpoint.clone(),
            recovery_clock,
            descriptor.instance_uuid,
            Some(descriptor.boot_id),
            ServiceIntentJournal::open(&recovery_root).unwrap(),
        );
        let recovery_budget = || CallBudget {
            deadline: MonoInstant(5_000),
            cancellation: Cancellation::default(),
        };
        assert_eq!(
            recovery_client.register(&recovery_budget()).await.unwrap().author,
            registration.author
        );
        let lost = OperationId::new("req-lost");
        let failed = recovery_client
            .submit(
                ServiceOperation::Send(ServiceSend {
                    thread: thread.clone(),
                    body: "do Z once".into(),
                    recipients: vec![],
                    deadline_millis: Some(600_000),
                    operation: lost.clone(),
                }),
                &recovery_budget(),
            )
            .await
            .unwrap_err();
        assert_eq!(failed.pending_operation(), Some(&lost));
        let saved = recovery_client.journal().inspect(&lost).unwrap();
        assert_eq!(saved.author, registration.author);
        recovery_client.disconnect().await;
        wait_revoked();
        let mut raw = UnixStream::connect(&descriptor.endpoint).await.unwrap();
        let registered = raw_service_call(
            &mut raw,
            &wire(
                &instance.to_string(),
                "raw-register-v2",
                ServiceRequest::Register(ServiceRegister {
                    capability: SERVICE_SESSION_CAPABILITY_V2.into(),
                }),
            ),
        )
        .await;
        assert!(
            matches!(registered.result, Ok(ServiceResult::Registered(ref state)) if state.author == registration.author),
            "{registered:?}"
        );
        write_frame(&mut raw, &serde_json::to_vec(&saved.request).unwrap())
            .await
            .unwrap();
        wait_until("the response-loss send to commit", || {
            count("SELECT count(*) FROM operations WHERE operation_key=?1", lost.as_str()) == 1
        });
        drop(raw); // the caller never observes the committed response
        wait_revoked();
        deadline_clock.expire_after_record.store(false, Ordering::SeqCst);
        assert_eq!(
            recovery_client.register(&recovery_budget()).await.unwrap().author,
            registration.author
        );
        let replayed = recovery_client.replay(&lost, &recovery_budget()).await.unwrap();
        let ServiceResult::MessageSent(replayed) = replayed else {
            panic!("replay did not return MessageSent");
        };
        assert_eq!(replayed.recipient_count, 1, "worker only; retiree is retired");
        assert_eq!(
            count("SELECT count(*) FROM messages WHERE body=?1", "do Z once"),
            1,
            "replay must not publish a duplicate"
        );
        assert_eq!(
            count("SELECT count(*) FROM operations WHERE operation_key=?1", lost.as_str()),
            1
        );
        assert!(recovery_client.journal().pending().unwrap().is_empty());
        recovery_client.disconnect().await;
        wait_revoked();

        // 8. A v1 session cannot send, but its v1 operations still work.
        let mut v1 = UnixStream::connect(&descriptor.endpoint).await.unwrap();
        let registered = raw_service_call(
            &mut v1,
            &wire(
                &instance.to_string(),
                "raw-register-v1",
                ServiceRequest::Register(ServiceRegister {
                    capability: SERVICE_SESSION_CAPABILITY.into(),
                }),
            ),
        )
        .await;
        assert!(matches!(registered.result, Ok(ServiceResult::Registered(_))), "{registered:?}");
        let refused = raw_service_call(
            &mut v1,
            &wire(
                &instance.to_string(),
                "raw-v1-send",
                ServiceRequest::Operation(ServiceOperation::Send(ServiceSend {
                    thread: thread.clone(),
                    body: "v1 must not publish".into(),
                    recipients: vec![],
                    deadline_millis: None,
                    operation: OperationId::new("req-v1"),
                })),
            ),
        )
        .await;
        assert_eq!(refused.result.unwrap_err().code, ErrorCode::Unsupported);
        let still_v1 = raw_service_call(
            &mut v1,
            &wire(
                &instance.to_string(),
                "raw-v1-membership",
                ServiceRequest::Operation(ServiceOperation::Membership(ServiceMembershipQuery {
                    thread: thread.clone(),
                    seat: None,
                    page: PageRequest::default(),
                })),
            ),
        )
        .await;
        assert!(still_v1.result.is_ok(), "{still_v1:?}");
        assert_eq!(
            count("SELECT count(*) FROM messages WHERE body=?1", "v1 must not publish"),
            0
        );
        drop(v1);
        wait_revoked();

        // 9. No receipt row ever names the service author as a seat.
        for table in ["receipt_state", "receipts", "prepared_recipients"] {
            assert_eq!(
                count(
                    &format!("SELECT count(*) FROM {table} WHERE seat_id=?1"),
                    registration.author.as_str()
                ),
                0,
                "{table} holds a row for the service author"
            );
        }
    });
    daemon.finish();
}
