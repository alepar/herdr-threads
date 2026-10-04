use herdr_threads::{
    app::{SystemClock, run_elected},
    client::local::LocalSocketClient,
    daemon::{
        ownership::{EndpointDescriptor, OwnerLock},
        paths::{InstancePaths, RuntimeContext},
    },
    ports::*,
    protocol::{
        commands::*,
        ids::*,
        results::{ApiError, CommandResult, ErrorCode},
        time::{CallBudget, Cancellation, Clock, MonoInstant},
    },
    service::config::ServiceConfig,
    store::{SqliteStore, StoreSettings, connection::StoreContext},
};
use std::{
    fs,
    os::unix::fs::DirBuilderExt,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::Duration,
};
struct Host {
    path: PathBuf,
    clock: Arc<dyn Clock>,
    sequence: AtomicU64,
    calls: AtomicU64,
    fail: AtomicBool,
    invalidate: AtomicBool,
    unqualified: AtomicBool,
    store: Arc<SqliteStore>,
    instance: String,
}
impl Host {
    fn observation(&self, target: &str, sequence: u64) -> HostObservation {
        let at = self.clock.monotonic_now();
        HostObservation {
            focused: false,
            target: HostTargetId::new(target),
            host_boot: HostBootId::new("host"),
            epoch: 1,
            generation: 1,
            observed_at_utc: self.clock.utc_now(),
            observed_at_mono: at,
            provenance: ObservationProvenance::FreshCurrentTarget,
            occupant: None,
            ui: HostUiState::Idle,
            terminal: Some(TerminalId::new(format!("terminal-{target}"))),
            occupancy: StructuralOccupancy::EmptyShell,
            incarnation: IncarnationEvidence::Verified {
                identity: "incarnation".into(),
                evidence_kind: EvidenceKind::CoherentEnumeration,
            },
            execution: ExecutionEvidence::Unknown,
            call_id: HostCallId::new(format!("call-{sequence}-{target}")),
            connection_epoch: 1,
            observation_sequence: sequence,
            started_at_mono: at,
            completed_at_mono: at,
        }
    }
}
impl HostPort for Host {
    fn native_launch_capability(&self) -> NativeLaunchCapability {
        NativeLaunchCapability::Unsupported
    }
    fn observe_current_target(
        &self,
        target: &HostTargetId,
        context: &HostCallContext,
    ) -> Result<HostObservation, ApiError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let db = rusqlite::Connection::open(&self.path).unwrap();
        db.busy_timeout(super::HOST_IO_WRITER_PROBE_WAIT).unwrap();
        db.execute_batch("BEGIN IMMEDIATE; ROLLBACK")
            .expect("operator host I/O held SQLite writer");
        assert!(
            db.query_row(
                "SELECT observation_admission_sequence FROM host_instances WHERE id=?1",
                [&self.instance],
                |r| r.get::<_, i64>(0)
            )
            .unwrap()
                > 0
        );
        if self.fail.load(Ordering::SeqCst) {
            return Err(ApiError::host_unavailable("fixture unavailable"));
        }
        let mut observation = self.observation(
            target.as_str(),
            self.sequence.fetch_add(1, Ordering::SeqCst),
        );
        if self.unqualified.load(Ordering::SeqCst) {
            observation.incarnation = IncarnationEvidence::Unknown;
        }
        if self.invalidate.swap(false, Ordering::SeqCst) {
            let newer = self
                .store
                .begin_host_observation(&self.instance, &context.budget)
                .unwrap();
            self.store
                .invalidate_host_observation(
                    &newer,
                    HostInvalidationReason::HostUnavailable,
                    &context.budget,
                )
                .unwrap();
        }
        Ok(observation)
    }
    fn enumerate_targets(&self, _: &HostCallContext) -> Result<HostSnapshot, ApiError> {
        let sequence = self.sequence.fetch_add(1, Ordering::SeqCst);
        Ok(HostSnapshot {
            boot: HostBootId::new("host"),
            epoch: 1,
            observation_sequence: sequence,
            complete: true,
            enumeration: EnumerationEvidence::CoherentVerified,
            incarnation: IncarnationEvidence::Verified {
                identity: "incarnation".into(),
                evidence_kind: EvidenceKind::CoherentEnumeration,
            },
            targets: ["w1:p1", "w1:p2", "w1:p3"]
                .into_iter()
                .map(|p| self.observation(p, sequence))
                .collect(),
        })
    }
    fn safe_wake_target(&self, _: &SeatId, _: &HostObservation) -> Option<SafeWakeTarget> {
        None
    }
    fn submit_prompt(
        &self,
        _: &SafeWakeTarget,
        _: &str,
        _: &HostCallContext,
    ) -> Result<PromptOutcome, ApiError> {
        unreachable!()
    }
    fn pane_agent_state(
        &self,
        _target: &SafeWakeTarget,
        _context: &HostCallContext,
    ) -> Result<herdr_threads::ports::AgentComposerState, ApiError> {
        Ok(herdr_threads::ports::AgentComposerState::Submitted)
    }

    fn launch_native(
        &self,
        _: NativeLaunchRequest,
        _: &HostCallContext,
    ) -> Result<NativeLaunchOutcome, ApiError> {
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
struct Fixture {
    root: PathBuf,
    paths: InstancePaths,
    host: Arc<Host>,
    clock: Arc<dyn Clock>,
    descriptor: EndpointDescriptor,
    stop: Cancellation,
    daemon: Option<std::thread::JoinHandle<std::io::Result<bool>>>,
    _guard: std::sync::MutexGuard<'static, ()>,
}
impl Fixture {
    fn new() -> Self {
        let guard = super::IN_PROCESS_DAEMON
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let root = std::env::temp_dir().join(format!("operator-ipc-{}", uuid::Uuid::new_v4()));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let runtime =
            RuntimeContext::explicit(root.join("state"), root.join("host.sock"), None).unwrap();
        let paths = InstancePaths::resolve(&runtime).unwrap();
        let owner = OwnerLock::acquire(&paths).unwrap();
        let instance = owner.instance_uuid().to_string();
        drop(owner);
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let store = Arc::new(
            SqliteStore::new(
                StoreContext::new(paths.database_path.clone(), clock.clone()),
                &instance,
                StoreSettings::default(),
            )
            .unwrap(),
        );
        let db = rusqlite::Connection::open(&paths.database_path).unwrap();
        db.execute(
            "INSERT INTO host_instances(id,created_at) VALUES (?1,0)",
            [&instance],
        )
        .unwrap();
        db.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('saved',?1,'unresolved','native',1,0)",[&instance]).unwrap();
        drop(db);
        let host = Arc::new(Host {
            path: paths.database_path.clone(),
            clock: clock.clone(),
            sequence: AtomicU64::new(1),
            calls: AtomicU64::new(0),
            fail: AtomicBool::new(false),
            invalidate: AtomicBool::new(false),
            unqualified: AtomicBool::new(false),
            store,
            instance,
        });
        let budget = CallBudget {
            deadline: MonoInstant(clock.monotonic_now().0 + 5000),
            cancellation: Cancellation::default(),
        };
        let captured = host
            .enumerate_targets(&HostCallContext {
                budget: budget.clone(),
                expected_boot: None,
                expected_epoch: None,
            })
            .unwrap();
        let admission = host
            .store
            .begin_host_observation(&host.instance, &budget)
            .unwrap();
        let stage = host
            .store
            .begin_snapshot_stage(
                SnapshotHeader::from_captured(admission, &captured).unwrap(),
                &budget,
            )
            .unwrap();
        host.store
            .stage_snapshot_targets(
                &stage.id,
                0,
                &captured.targets,
                DurableWorkAdmission::new(16).unwrap(),
                &budget,
            )
            .unwrap();
        host.store.seal_snapshot_stage(&stage.id, &budget).unwrap();
        host.store
            .publish_snapshot_stage(&stage.id, &budget)
            .unwrap();
        let stop = Cancellation::default();
        let (tx, rx) = mpsc::sync_channel(1);
        let thread_paths = paths.clone();
        let thread_clock = clock.clone();
        let thread_host = host.clone();
        let thread_stop = stop.clone();
        let daemon = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(run_elected(
                    &thread_paths,
                    thread_clock,
                    thread_stop,
                    ServiceConfig::default(),
                    thread_host,
                    move |d| {
                        tx.send(d.clone()).unwrap();
                        Ok(())
                    },
                ))
        });
        let descriptor = match rx.recv_timeout(Duration::from_secs(30)) {
            Ok(d) => d,
            Err(error) => {
                stop.cancel();
                let outcome = daemon.join();
                let log =
                    fs::read_to_string(paths.instance_dir.join("daemon.log")).unwrap_or_default();
                let _ = fs::remove_dir_all(&root);
                panic!("private operator daemon not ready: {error}; {outcome:?}; {log}");
            }
        };
        Self {
            root,
            paths,
            host,
            clock,
            descriptor,
            stop,
            daemon: Some(daemon),
            _guard: guard,
        }
    }
    fn call(&self, command: Command) -> Result<CommandResult, ApiError> {
        let client = LocalSocketClient::new(
            self.descriptor.endpoint.clone(),
            self.clock.clone(),
            self.descriptor.instance_uuid,
            Some(self.descriptor.boot_id),
        );
        client.call(
            command,
            &CallBudget {
                deadline: MonoInstant(self.clock.monotonic_now().0 + 5000),
                cancellation: Cancellation::default(),
            },
        )
    }
    fn db(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(&self.paths.database_path).unwrap()
    }
    fn fresh(&self, target: &str, operation: &str) -> Result<CommandResult, ApiError> {
        self.call(Command::OperatorFreshSeat(OperatorFreshSeat {
            target: HostTargetId::new(target),
            operation: OperationId::new(operation),
        }))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(t) = self.daemon.take() {
            let _ = t.join();
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}
#[test]
fn elected_operator_rebind_and_fresh_preserve_history_and_replay_without_host() {
    let f = Fixture::new();
    let history = f.db();
    history.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence) VALUES ('history',?1,'original topic','original goal',0,0,2)",[f.descriptor.instance_uuid.to_string()]).unwrap();
    history.execute_batch("INSERT INTO memberships(thread_id,seat_id,state,episode) VALUES ('history','saved','invited',1);").unwrap();
    history.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_seq,decision_at) VALUES ('old-message',?1,'history',1,'ordinary','unresolved history',1,0)",[f.descriptor.instance_uuid.to_string()]).unwrap();
    history.execute_batch("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES ('old-message','history','saved','pending',90000);").unwrap();
    let rebind = Command::OperatorRebind(OperatorRebind {
        seat: SeatId::new("saved"),
        target: HostTargetId::new("w1:p1"),
        operation: OperationId::new("rebind"),
    });
    let result = f.call(rebind.clone()).unwrap();
    assert_eq!(result, CommandResult::OperatorRebound(SeatId::new("saved")));
    let db = f.db();
    let label: String = db
        .query_row(
            "SELECT operator_label FROM allocation_decisions WHERE kind='operator_rebind'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    use std::os::unix::fs::MetadataExt;
    let uid = fs::metadata(&f.paths.instance_dir).unwrap().uid();
    assert_eq!(label, format!("operator:local-user:{uid}"));
    assert_eq!(
        db.query_row("SELECT count(*) FROM occupant_bindings", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM receipts WHERE state!='pending' OR available_at IS NOT NULL OR deadline_at IS NOT NULL", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    let CommandResult::OperatorFreshSeat(new) = f.fresh("w1:p2", "fresh").unwrap() else {
        panic!()
    };
    assert_ne!(new, SeatId::new("saved"));
    assert_eq!(
        f.fresh("w1:p1", "collision").unwrap_err().code,
        ErrorCode::TargetAlreadyOwned
    );
    db.execute(
        "UPDATE seats SET state='retired',retired_at=1 WHERE id='saved'",
        [],
    )
    .unwrap();
    let retired = Command::OperatorRebind(OperatorRebind {
        seat: SeatId::new("saved"),
        target: HostTargetId::new("w1:p3"),
        operation: OperationId::new("retired"),
    });
    assert_eq!(
        f.call(retired).unwrap_err().code,
        ErrorCode::TargetUnresolved
    );
    let calls = f.host.calls.load(Ordering::SeqCst);
    f.host.fail.store(true, Ordering::SeqCst);
    assert_eq!(f.call(rebind).unwrap(), result);
    assert_eq!(
        f.fresh("w1:p2", "fresh").unwrap(),
        CommandResult::OperatorFreshSeat(new)
    );
    assert_eq!(f.host.calls.load(Ordering::SeqCst), calls);
    assert_eq!(
        db.query_row("SELECT count(*) FROM allocation_decisions", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        db.query_row("SELECT role FROM seats WHERE id='saved'", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "native"
    );
}
#[test]
fn elected_operator_orphan_invites_atomically_without_join_or_target_capture() {
    let f = Fixture::new();
    let db = f.db();
    db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('thread',?1,'topic','goal',0,0)",[f.descriptor.instance_uuid.to_string()]).unwrap();
    let invite = OperatorOrphanInvite {
        thread: ThreadId::new("thread"),
        seat: SeatId::new("saved"),
        deadline_millis: Some(9000),
        operation: OperationId::new("invite"),
    };
    f.host.fail.store(true, Ordering::SeqCst);
    let result = f
        .call(Command::OperatorOrphanInvite(invite.clone()))
        .unwrap();
    assert!(matches!(&result, CommandResult::OperatorInvited(_)));
    assert_eq!(
        db.query_row(
            "SELECT state FROM memberships WHERE thread_id='thread' AND seat_id='saved'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "invited"
    );
    assert_eq!(f.host.calls.load(Ordering::SeqCst), 0);
    db.execute(
        "UPDATE memberships SET state='joined',joined_at=1 WHERE seat_id='saved'",
        [],
    )
    .unwrap();
    assert_eq!(
        f.call(Command::OperatorOrphanInvite(invite.clone()))
            .unwrap(),
        result
    );
    assert_eq!(
        f.call(Command::OperatorOrphanInvite(OperatorOrphanInvite {
            operation: OperationId::new("joined"),
            ..invite.clone()
        }))
        .unwrap_err()
        .code,
        ErrorCode::ThreadNotOrphaned
    );
    assert_eq!(
        f.call(Command::OperatorOrphanInvite(OperatorOrphanInvite {
            deadline_millis: None,
            ..invite
        }))
        .unwrap_err()
        .code,
        ErrorCode::OperationPayloadMismatch
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM invitations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
#[test]
fn elected_operator_rejects_newer_invalidation_without_allocating() {
    let f = Fixture::new();
    f.host.invalidate.store(true, Ordering::SeqCst);
    assert_eq!(
        f.fresh("w1:p1", "invalidated").unwrap_err().code,
        ErrorCode::StaleHostObservation
    );
    assert_eq!(
        f.db()
            .query_row("SELECT count(*) FROM allocation_decisions", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        f.db()
            .query_row("SELECT count(*) FROM seats", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn elected_operator_unqualified_target_has_no_allocation_or_native_authority() {
    let f = Fixture::new();
    f.host.unqualified.store(true, Ordering::SeqCst);
    assert_eq!(
        f.fresh("w1:p1", "unqualified").unwrap_err().code,
        ErrorCode::StaleHostObservation
    );
    let db = f.db();
    assert_eq!(
        db.query_row("SELECT count(*) FROM allocation_decisions", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM occupant_bindings", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM operations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

struct LostOutput;
impl std::io::Write for LostOutput {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "lost operator output",
        ))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// End-to-end CLI mutation surface over the production elected service.
/// Kills: the one-shot catch-all `Mutation(_) => Unsupported` (operator forms
/// fail), an operator intent that is not replayable by `retry`, and a caller
/// derivation that ignores the invoking pane (the recipient's accept/ACK would
/// be attributed to the wrong seat and be rejected).
#[test]
fn cli_operator_and_pane_derived_agent_forms_drive_the_elected_service() {
    use herdr_threads::cli::{RunError, run_in_pane};
    let f = Fixture::new();
    let state = f.root.join("state");
    let host = f.root.join("host.sock");
    let base = |args: &[&str]| -> Vec<String> {
        let mut argv = vec![
            "herdr-threads".to_owned(),
            "--json".to_owned(),
            "--state-dir".to_owned(),
            state.to_str().unwrap().to_owned(),
            "--host-endpoint".to_owned(),
            host.to_str().unwrap().to_owned(),
        ];
        argv.extend(args.iter().map(|a| (*a).to_owned()));
        argv
    };
    let run = |pane: Option<&str>, args: &[&str]| -> Result<serde_json::Value, RunError> {
        let mut out = Vec::new();
        run_in_pane(base(args), pane, &mut out)?;
        Ok(serde_json::from_slice(&out).unwrap())
    };
    let ok = |pane: Option<&str>, args: &[&str]| -> serde_json::Value {
        run(pane, args).unwrap_or_else(|error| panic!("{args:?}: {error:?}"))
    };
    let api_error = |result: Result<serde_json::Value, RunError>| match result {
        Err(RunError::Api(error)) => error,
        other => panic!("expected API error, got {other:?}"),
    };

    // Operator forms reach the daemon and are attributed to the local user.
    let fresh = ok(
        None,
        &[
            "seat",
            "resolve",
            "--pane",
            "w1:p2",
            "--new-seat",
            "--operator",
        ],
    );
    assert_eq!(fresh["result"]["kind"], "operator_fresh_seat");
    let recipient = fresh["result"]["data"].as_str().unwrap().to_owned();
    let rebound = ok(
        None,
        &["seat", "rebind", "saved", "--pane", "w1:p1", "--operator"],
    );
    assert_eq!(rebound["result"]["kind"], "operator_rebound");
    assert_eq!(rebound["result"]["data"], "saved");
    use std::os::unix::fs::MetadataExt;
    let uid = fs::metadata(&f.paths.instance_dir).unwrap().uid();
    let labels: Vec<String> = f
        .db()
        .prepare("SELECT operator_label FROM allocation_decisions ORDER BY kind")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(labels, vec![format!("operator:local-user:{uid}"); 2]);
    // Operator authority is never mixed with a caller seat selection.
    let mixed = api_error(run(
        None,
        &[
            "--cooperative-seat",
            "saved",
            "--cooperative-target",
            "w1:p1",
            "--cooperative-harness",
            "claude",
            "--cooperative-role",
            "top-level",
            "seat",
            "rebind",
            "saved",
            "--pane",
            "w1:p1",
            "--operator",
        ],
    ));
    assert_eq!(mixed.code, ErrorCode::InvalidRequest);
    assert!(mixed.detail.contains("--operator"), "{}", mixed.detail);

    // Launch-driver lifecycle check-ins record each seat's private context.
    for (seat, pane, event) in [
        ("saved", "w1:p1", "launch-a"),
        (recipient.as_str(), "w1:p2", "launch-b"),
    ] {
        let checked = ok(
            None,
            &[
                "--cooperative-seat",
                seat,
                "--cooperative-target",
                pane,
                "--cooperative-harness",
                "claude",
                "--cooperative-role",
                "top-level",
                "check-in",
                "--lifecycle-event",
                event,
            ],
        );
        assert_eq!(checked["result"]["kind"], "checked_in");
    }

    // Documented agent forms without selection flags derive the caller from
    // the invoking pane and act as that seat.
    let created = ok(
        Some("w1:p1"),
        &["thread", "create", "--topic", "cli surface"],
    );
    assert_eq!(created["result"]["kind"], "thread_created");
    let thread = created["result"]["data"].as_str().unwrap().to_owned();
    assert_eq!(
        ok(
            Some("w1:p1"),
            &["thread", "topic", &thread, "--set", "renamed"]
        )["result"]["kind"],
        "topic_changed"
    );
    assert_eq!(
        ok(Some("w1:p1"), &["invite", &thread, "--seat", &recipient])["result"]["kind"],
        "invitation"
    );
    let body = f.root.join("body.txt");
    fs::write(&body, "line one\nline two ünïcode\n").unwrap();
    let sent = ok(
        Some("w1:p1"),
        &[
            "send",
            &thread,
            "--file",
            body.to_str().unwrap(),
            "--require-ack",
            &recipient,
        ],
    );
    assert_eq!(sent["result"]["kind"], "message_sent");
    let message = sent["result"]["data"].as_str().unwrap().to_owned();
    assert_eq!(
        ok(Some("w1:p2"), &["accept", &thread])["result"]["kind"],
        "accepted"
    );
    let pending = ok(None, &["pending-receipts", "--seat", &recipient]);
    assert_eq!(
        pending["result"]["data"]["items"][0]["message"],
        message.as_str()
    );
    // Reading never ACKs.
    let history = ok(Some("w1:p2"), &["read", &thread, "--recent", "5"]);
    assert_eq!(history["result"]["kind"], "history");
    assert_eq!(
        ok(None, &["pending-receipts", "--seat", &recipient])["result"]["data"]["items"][0]["message"],
        message.as_str()
    );
    let acked = ok(Some("w1:p2"), &["ack", &message]);
    assert_eq!(acked["result"]["kind"], "acknowledged");
    let acked_seat: String = f
        .db()
        .query_row(
            "SELECT seat_id FROM receipt_state WHERE message_id=?1 AND state='acked'",
            [&message],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(acked_seat, recipient);
    let stored: String = f
        .db()
        .query_row("SELECT body FROM messages WHERE id=?1", [&message], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(stored, "line one\nline two ünïcode\n");
    assert_eq!(
        ok(Some("w1:p1"), &["archive", &thread])["result"]["kind"],
        "archived"
    );
    assert_eq!(
        ok(Some("w1:p1"), &["reopen", &thread])["result"]["kind"],
        "reopened"
    );

    // Without a pane or selection flags the CLI explains what is required;
    // a pane without a mapped seat cannot act. Both are the one "no caller
    // located" classification: invalid_request (exit 2), never unsupported
    // (4) or target_unresolved (1).
    let no_caller = api_error(run(None, &["ack", &message]));
    assert_eq!(no_caller.code, ErrorCode::InvalidRequest);
    assert!(
        no_caller.detail.contains("HERDR_PANE_ID")
            && no_caller.detail.contains("--cooperative-seat"),
        "{}",
        no_caller.detail
    );
    assert_eq!(
        api_error(run(Some("w1:p3"), &["ack", &message])).code,
        ErrorCode::InvalidRequest
    );

    // Zero-joined recovery through the documented operator invite.
    assert_eq!(
        ok(Some("w1:p1"), &["leave", &thread])["result"]["kind"],
        "left"
    );
    assert_eq!(
        ok(Some("w1:p2"), &["leave", &thread])["result"]["kind"],
        "left"
    );
    let invited = ok(None, &["invite", &thread, "--seat", "saved", "--operator"]);
    assert_eq!(invited["result"]["kind"], "operator_invited");

    // An operator mutation whose output was lost stays in the private journal
    // under operator scope and is completed by `retry` with the same key.
    let mut lost = LostOutput;
    assert!(
        run_in_pane(
            base(&[
                "seat",
                "resolve",
                "--pane",
                "w1:p3",
                "--new-seat",
                "--operator"
            ]),
            None,
            &mut lost
        )
        .is_err()
    );
    let pending_ops = ok(None, &["pending-ops"]);
    let items = pending_ops["result"]["data"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{pending_ops}");
    assert_eq!(items[0]["kind"], "operator_fresh_seat");
    let reference = items[0]["recovery_ref"].as_str().unwrap().to_owned();
    let retried = ok(None, &["retry", &reference]);
    assert_eq!(retried["result"]["kind"], "operator_fresh_seat");
    let third_seat: String = f
        .db()
        .query_row("SELECT id FROM seats WHERE target_id='w1:p3'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(retried["result"]["data"], third_seat.as_str());
    assert_eq!(
        ok(None, &["pending-ops"])["result"]["data"]["items"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

/// Every seat-defaulting CLI action uses the same pane-to-caller derivation:
/// a pane-derived send whose output was lost is retried from the same pane with
/// its persisted operation key, and `pending-receipts`/`inbox` inside a pane
/// default to that pane's seat without any accountable effect.
/// Kills: (R) `retry` without Retry derivation (the cooperative intent is
/// rejected as needing --cooperative-* flags); (S) read defaults without the
/// pane seat (`pending-receipts`/`inbox` fail with "require seat");
/// (F) a thread-scoped `pending-receipts` that fails instead of staying
/// thread-scoped in a pane that maps to no seat; (W) a retry whose derived seat
/// is not checked against the intent's seat (another pane's seat would replay it).
#[test]
fn cli_pane_derivation_covers_retry_and_seat_default_reads() {
    use herdr_threads::cli::{RunError, run_in_pane};
    let f = Fixture::new();
    let state = f.root.join("state");
    let host = f.root.join("host.sock");
    let base = |args: &[&str]| -> Vec<String> {
        let mut argv = vec![
            "herdr-threads".to_owned(),
            "--json".to_owned(),
            "--state-dir".to_owned(),
            state.to_str().unwrap().to_owned(),
            "--host-endpoint".to_owned(),
            host.to_str().unwrap().to_owned(),
        ];
        argv.extend(args.iter().map(|a| (*a).to_owned()));
        argv
    };
    let run = |pane: Option<&str>, args: &[&str]| -> Result<serde_json::Value, RunError> {
        let mut out = Vec::new();
        run_in_pane(base(args), pane, &mut out)?;
        Ok(serde_json::from_slice(&out).unwrap())
    };
    let ok = |pane: Option<&str>, args: &[&str]| -> serde_json::Value {
        run(pane, args).unwrap_or_else(|error| panic!("{args:?}: {error:?}"))
    };
    let api_error = |result: Result<serde_json::Value, RunError>| match result {
        Err(RunError::Api(error)) => error,
        other => panic!("expected API error, got {other:?}"),
    };
    let fresh = ok(
        None,
        &[
            "seat",
            "resolve",
            "--pane",
            "w1:p2",
            "--new-seat",
            "--operator",
        ],
    );
    let recipient = fresh["result"]["data"].as_str().unwrap().to_owned();
    ok(
        None,
        &["seat", "rebind", "saved", "--pane", "w1:p1", "--operator"],
    );
    for (seat, pane, event) in [
        ("saved", "w1:p1", "launch-a"),
        (recipient.as_str(), "w1:p2", "launch-b"),
    ] {
        ok(
            None,
            &[
                "--cooperative-seat",
                seat,
                "--cooperative-target",
                pane,
                "--cooperative-harness",
                "claude",
                "--cooperative-role",
                "top-level",
                "check-in",
                "--lifecycle-event",
                event,
            ],
        );
    }
    let created = ok(Some("w1:p1"), &["thread", "create", "--topic", "retry"]);
    let thread = created["result"]["data"].as_str().unwrap().to_owned();
    ok(Some("w1:p1"), &["invite", &thread, "--seat", &recipient]);
    ok(Some("w1:p2"), &["accept", &thread]);

    // A pane-derived send whose output is lost stays journaled.
    let mut lost = LostOutput;
    assert!(
        run_in_pane(
            base(&[
                "send",
                &thread,
                "--body",
                "lost reply",
                "--require-ack",
                &recipient
            ]),
            Some("w1:p1"),
            &mut lost
        )
        .is_err()
    );
    let pending_ops = ok(None, &["pending-ops"]);
    let items = pending_ops["result"]["data"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{pending_ops}");
    assert_eq!(items[0]["kind"], "send_message");
    let reference = items[0]["recovery_ref"].as_str().unwrap().to_owned();
    let committed: String = f
        .db()
        .query_row("SELECT id FROM messages WHERE body='lost reply'", [], |r| {
            r.get(0)
        })
        .unwrap();

    // (W) Another pane's seat cannot replay this seat's intent.
    let foreign = api_error(run(Some("w1:p2"), &["retry", &reference]));
    assert!(
        foreign.detail.contains("different instance or seat"),
        "{}",
        foreign.detail
    );
    assert_eq!(
        ok(None, &["pending-ops"])["result"]["data"]["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    // (R) The documented recovery runs from the originating pane with the
    // persisted key: it renders the already committed message, sends nothing new.
    let retried = ok(Some("w1:p1"), &["retry", &reference]);
    assert_eq!(retried["result"]["kind"], "message_sent");
    assert_eq!(retried["result"]["data"], committed.as_str());
    let copies: i64 = f
        .db()
        .query_row(
            "SELECT count(*) FROM messages WHERE body='lost reply'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(copies, 1);
    assert_eq!(
        ok(None, &["pending-ops"])["result"]["data"]["items"]
            .as_array()
            .unwrap()
            .len(),
        0
    );

    // (S) Reads in the recipient's pane default to its seat and never ACK.
    let receipts = ok(Some("w1:p2"), &["pending-receipts"]);
    let listed = receipts["result"]["data"]["items"].as_array().unwrap();
    assert_eq!(listed.len(), 1, "{receipts}");
    assert_eq!(listed[0]["message"], committed.as_str());
    let inbox = ok(Some("w1:p2"), &["inbox"]);
    assert_eq!(inbox["result"]["kind"], "inbox");
    assert!(inbox.to_string().contains(&thread), "{inbox}");
    // Explicit read selectors choose the requested seat, independently of the caller.
    let selected = ok(Some("w1:p1"), &["pending-receipts", "--pane", "w1:p2"]);
    assert_eq!(selected["result"]["data"], receipts["result"]["data"]);
    let selected_inbox = ok(Some("w1:p1"), &["inbox", "--pane", "w1:p2"]);
    assert_eq!(selected_inbox["result"]["data"], inbox["result"]["data"]);

    // The sender's pane selects the sender's seat, which owes nothing.
    assert_eq!(
        ok(Some("w1:p1"), &["pending-receipts"])["result"]["data"]["items"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    let acked: i64 = f
        .db()
        .query_row(
            "SELECT count(*) FROM receipt_state WHERE message_id=?1 AND state='acked'",
            [&committed],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(acked, 0);
    // (F) An explicit thread scope still works where the pane has no seat.
    let scoped = ok(Some("w1:p3"), &["pending-receipts", "--thread", &thread]);
    assert_eq!(
        scoped["result"]["data"]["items"].as_array().unwrap().len(),
        1,
        "{scoped}"
    );
    // Without a pane or --seat, the seat-defaulting reads name --seat.
    for args in [&["inbox"][..], &["pending-receipts"][..]] {
        let missing = api_error(run(None, args));
        assert_eq!(missing.code, ErrorCode::InvalidRequest);
        assert!(missing.detail.contains("--seat"), "{}", missing.detail);
    }
}

fn operator_label_for_daemon(f: &Fixture) -> String {
    use std::os::unix::fs::MetadataExt;
    format!(
        "operator:local-user:{}",
        fs::metadata(&f.paths.instance_dir).unwrap().uid()
    )
}

// B5 (ht-rzi.1): `seat retire --operator` crosses the elected same-user
// operator arm without a host observation, is audited by operator label and
// replays. Kills: a dispatch arm that omits OperatorRetire (Unsupported), an
// audit row without the label, a retire that takes a host call.
#[test]
fn elected_operator_retire_over_ipc_is_audited_and_peer_checked() {
    let f = Fixture::new();
    let retire = Command::OperatorRetire(OperatorRetire {
        seat: SeatId::new("saved"),
        operation: OperationId::new("retire-saved"),
    });
    let calls = f.host.calls.load(Ordering::SeqCst);
    assert_eq!(
        f.call(retire.clone()).unwrap(),
        CommandResult::OperatorRetired(SeatId::new("saved"))
    );
    assert_eq!(
        f.host.calls.load(Ordering::SeqCst),
        calls,
        "retire claims no target and observes nothing"
    );
    let db = f.db();
    assert_eq!(
        db.query_row("SELECT state FROM seats WHERE id='saved'", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "retired"
    );
    let audit: (String, Option<String>) = db
        .query_row(
            "SELECT kind,operator_label FROM allocation_decisions",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        audit,
        (
            "operator_retire".into(),
            Some(operator_label_for_daemon(&f))
        )
    );
    assert_eq!(
        f.call(retire).unwrap(),
        CommandResult::OperatorRetired(SeatId::new("saved"))
    );
    assert_eq!(
        f.call(Command::OperatorRetire(OperatorRetire {
            seat: SeatId::new("saved"),
            operation: OperationId::new("retire-saved-again"),
        }))
        .unwrap_err()
        .code,
        ErrorCode::NotFound
    );
}

#[test]
fn elected_operator_replace_over_ipc() {
    let f = Fixture::new();
    let CommandResult::OperatorFreshSeat(new) = f.fresh("w1:p2", "fresh-other").unwrap() else {
        panic!()
    };
    let replace = |operation: &str, replace: &SeatId| {
        Command::OperatorReplace(OperatorReplace {
            seat: SeatId::new("saved"),
            target: HostTargetId::new("w1:p2"),
            replace: replace.clone(),
            operation: OperationId::new(operation),
        })
    };
    // A seat that does not own the pane is refused and nothing changes.
    assert_eq!(
        f.call(replace("replace-wrong", &SeatId::new("saved")))
            .unwrap_err()
            .code,
        ErrorCode::TargetAlreadyOwned
    );
    assert_eq!(
        f.call(replace("replace-ok", &new)).unwrap(),
        CommandResult::OperatorRebound(SeatId::new("saved"))
    );
    let db = f.db();
    let states: (String, String, Option<String>) = db
        .query_row(
            "SELECT (SELECT state FROM seats WHERE id=?1),(SELECT state FROM seats WHERE id='saved'),(SELECT target_id FROM seats WHERE id='saved')",
            [new.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        states,
        ("retired".into(), "resolved".into(), Some("w1:p2".into()))
    );
    let labels: Vec<(String, Option<String>)> = db
        .prepare("SELECT kind,operator_label FROM allocation_decisions WHERE kind IN ('operator_retire','operator_rebind') ORDER BY ordinal")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let label = Some(operator_label_for_daemon(&f));
    assert_eq!(
        labels,
        vec![
            ("operator_retire".to_owned(), label.clone()),
            ("operator_rebind".to_owned(), label)
        ]
    );
}
