use herdr_threads::service::config::ServiceConfig;
use herdr_threads::test_support::spawn::{OwnedChild, SpawnOwned};
use herdr_threads::{
    app::SystemClock,
    client::local::LocalSocketClient,
    daemon::{
        ownership::{OwnerLock, read_descriptor, read_existing_namespace},
        paths::{InstancePaths, RuntimeContext},
    },
    protocol::{
        commands::{Command, DirectoryMembership, DirectoryQuery},
        ids::HostTargetId,
        output::{ContinuationContext, OutputFormat, OutputSpec, encode_selected},
        pagination::PageRequest,
        results::{CommandResult, ErrorCode, HealthState},
        time::{CallBudget, Cancellation, Clock, MonoInstant},
        wire::{PROTOCOL_VERSION, WireRequest},
    },
    store::connection::StoreContext,
};
use std::{
    fs,
    io::Read,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};
use uuid::Uuid;

struct TestChild(OwnedChild);
impl Drop for TestChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn configured_timing_reaches_store_and_rejects_unsafe_wake_spacing() {
    let boot = Uuid::new_v4();
    let default = ServiceConfig::default();
    let settings = default.store_settings(boot);
    assert_eq!(settings.invitation_default_ms, Some(300_000));
    assert_eq!(settings.message_limits.receipt_duration_ms, 300_000);
    assert_eq!(settings.minimum_wake_delay_ms, 30_000);
    assert_eq!(settings.daemon_boot, Some(boot));

    let custom = ServiceConfig::new(120_000, 240_000, 45_000).unwrap();
    let settings = custom.store_settings(boot);
    assert_eq!(settings.invitation_default_ms, Some(120_000));
    assert_eq!(settings.message_limits.receipt_duration_ms, 240_000);
    assert_eq!(settings.minimum_wake_delay_ms, 45_000);
    assert!(ServiceConfig::new(120_000, 240_000, 29_999).is_err());
    assert!(ServiceConfig::new(120_000, 240_000, u64::MAX).is_err());
}

#[test]
fn selected_wire_output_is_optional_bounded_and_never_an_authority_field() {
    let instance = Uuid::new_v4().to_string();
    let default = WireRequest {
        version: PROTOCOL_VERSION,
        request_id: "r".into(),
        expected_instance: instance.clone(),
        expected_boot: None,
        output: None,
        command: Command::Health,
    };
    let encoded = serde_json::to_vec(&default).unwrap();
    assert_eq!(WireRequest::decode(&encoded).unwrap(), default);
    assert!(!String::from_utf8(encoded).unwrap().contains("\"output\""));
    let selected = WireRequest {
        output: Some(OutputSpec {
            format: OutputFormat::Text,
            context: ContinuationContext {
                state_dir: Some("/tmp/state".into()),
                host: Some("/tmp/host.sock".into()),
            },
        }),
        ..default
    };
    let mut raw = serde_json::to_value(&selected).unwrap();
    assert_eq!(
        WireRequest::decode(raw.to_string().as_bytes()).unwrap(),
        selected
    );
    raw["output"]["context"]["state_dir"] = serde_json::json!("x".repeat(1025));
    assert!(WireRequest::decode(raw.to_string().as_bytes()).is_err());
    raw["output"]["context"]["state_dir"] = serde_json::json!("/tmp/state");
    raw["output"]["verified_actor"] = serde_json::json!({"seat":"forged"});
    assert!(WireRequest::decode(raw.to_string().as_bytes()).is_err());
}

#[test]
fn private_instance_settings_load_overrides_and_reject_unsafe_files() {
    let root = std::env::temp_dir().join(format!("herdr-settings-{}", Uuid::new_v4()));
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    let context =
        RuntimeContext::explicit(root.join("state"), root.join("host.sock"), None).unwrap();
    let paths = InstancePaths::resolve(&context).unwrap();
    let defaults = ServiceConfig::load(&paths)
        .unwrap()
        .store_settings(Uuid::new_v4());
    assert_eq!(defaults.invitation_default_ms, Some(300_000));
    paths.prepare_instance_dir().unwrap();
    let file = paths.instance_dir.join("settings.json");
    fs::write(&file, br#"{"invitation_default_ms":120000,"receipt_default_ms":240000,"minimum_wake_delay_ms":45000}"#).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
    let settings = ServiceConfig::load(&paths)
        .unwrap()
        .store_settings(Uuid::new_v4());
    assert_eq!(settings.invitation_default_ms, Some(120_000));
    assert_eq!(settings.message_limits.receipt_duration_ms, 240_000);
    assert_eq!(settings.minimum_wake_delay_ms, 45_000);
    fs::write(&file, br#"{"minimum_wake_delay_ms":29999}"#).unwrap();
    assert!(ServiceConfig::load(&paths).is_err());
    fs::write(&file, br#"{"unknown":1}"#).unwrap();
    assert!(ServiceConfig::load(&paths).is_err());
    fs::write(&file, br#"{}"#).unwrap();
    fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(ServiceConfig::load(&paths).is_err());
    fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(&file, vec![b' '; 4097]).unwrap();
    assert!(ServiceConfig::load(&paths).is_err());
    fs::remove_file(&file).unwrap();
    std::os::unix::fs::symlink(root.join("missing"), &file).unwrap();
    assert!(ServiceConfig::load(&paths).is_err());
    let real_parent = root.join("real");
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&real_parent)
        .unwrap();
    let real_instance = real_parent.join("instance");
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&real_instance)
        .unwrap();
    let nested_file = real_instance.join("settings.json");
    fs::write(&nested_file, br#"{}"#).unwrap();
    fs::set_permissions(&nested_file, fs::Permissions::from_mode(0o600)).unwrap();
    std::os::unix::fs::symlink(&real_parent, root.join("alias")).unwrap();
    let mut aliased = paths.clone();
    aliased.instance_dir = root.join("alias/instance");
    assert!(ServiceConfig::load(&aliased).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn daemon_rejects_invalid_instance_settings_before_endpoint_publication() {
    let root = std::env::temp_dir().join(format!("herdr-config-child-{}", Uuid::new_v4()));
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    let context =
        RuntimeContext::explicit(root.join("state"), root.join("host.sock"), None).unwrap();
    let paths = InstancePaths::resolve(&context).unwrap();
    fs::create_dir_all(&paths.instance_dir).unwrap();
    for directory in [
        &context.state_dir,
        &context.state_dir.join("instances"),
        &paths.instance_dir,
    ] {
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let settings = paths.instance_dir.join("settings.json");
    fs::write(&settings, br#"{"minimum_wake_delay_ms":1}"#).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&settings, fs::Permissions::from_mode(0o600)).unwrap();
    let mut child = TestChild(
        super::scrubbed_command(env!("CARGO_BIN_EXE_herdr-threads"))
            .args([
                "daemon",
                "run",
                "--state-dir",
                root.join("state").to_str().unwrap(),
                "--host-endpoint",
                root.join("host.sock").to_str().unwrap(),
            ])
            .stdin(Stdio::null())
            .stderr(Stdio::piped())
            .spawn_owned()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "daemon accepted invalid settings"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    let mut stderr = String::new();
    child
        .0
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert_eq!(status.code(), Some(2));
    assert!(stderr.contains("invalid minimum wake spacing"), "{stderr}");
    assert!(!paths.descriptor_path.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn pending_ops_cli_keeps_explicit_context_in_its_local_journal_route() {
    let root = std::env::temp_dir().join(format!("herdr-intents-{}", Uuid::new_v4()));
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    let mut rendered = Vec::new();
    herdr_threads::cli::run(
        [
            "herdr-threads",
            "--json",
            "--state-dir",
            root.join("state").to_str().unwrap(),
            "--host-endpoint",
            root.join("host.sock").to_str().unwrap(),
            "pending-ops",
        ],
        &mut rendered,
    )
    .unwrap();
    let output: serde_json::Value = serde_json::from_slice(&rendered).unwrap();
    assert_eq!(output["result"]["kind"], "local_intents");
    assert_eq!(
        output["result"]["data"]["items"].as_array().unwrap().len(),
        0
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn explicit_cooperative_cli_check_in_reaches_elected_service_over_socket() {
    // run_elected_with_diagnostics dup2()s the process-wide stdout/stderr: hold
    // the in-process-daemon lock so a parallel run keeps the binary's output.
    let _stdio_guard = super::IN_PROCESS_DAEMON
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    use sha2::Digest;
    use std::sync::atomic::{AtomicU64, Ordering};
    struct CountCheckIns {
        inner: Arc<dyn herdr_threads::ports::LocalService>,
        calls: Arc<AtomicU64>,
    }
    impl herdr_threads::ports::LocalService for CountCheckIns {
        fn service_control(
            &self,
            command: Command,
            peer: herdr_threads::protocol::authority::PeerIdentity,
            instance: &str,
            boot: &str,
            gate: &herdr_threads::service::live_gate::LiveServiceGate,
            budget: &CallBudget,
        ) -> Result<CommandResult, herdr_threads::protocol::results::ApiError> {
            self.inner
                .service_control(command, peer, instance, boot, gate, budget)
        }
        fn audit_service_disconnect(
            &self,
            boot: &str,
            generation: u64,
            peer: herdr_threads::protocol::authority::PeerIdentity,
            budget: &CallBudget,
        ) -> Result<(), herdr_threads::protocol::results::ApiError> {
            self.inner
                .audit_service_disconnect(boot, generation, peer, budget)
        }
        fn service_operation(
            &self,
            operation: herdr_threads::protocol::service::ServiceOperation,
            connection: &herdr_threads::ports::ServiceConnectionAuthority,
            gate: &dyn herdr_threads::ports::ServiceAuthorityGate,
            budget: &CallBudget,
        ) -> Result<
            herdr_threads::protocol::service::ServiceResult,
            herdr_threads::protocol::results::ApiError,
        > {
            self.inner
                .service_operation(operation, connection, gate, budget)
        }
        fn handle(
            &self,
            command: Command,
            peer: herdr_threads::protocol::authority::PeerIdentity,
            budget: &CallBudget,
        ) -> Result<CommandResult, herdr_threads::protocol::results::ApiError> {
            if matches!(command, Command::CheckIn(_)) {
                self.calls.fetch_add(1, Ordering::SeqCst);
            }
            self.inner.handle(command, peer, budget)
        }
        fn handle_with_output(
            &self,
            command: Command,
            peer: herdr_threads::protocol::authority::PeerIdentity,
            budget: &CallBudget,
            output: &OutputSpec,
        ) -> Result<CommandResult, herdr_threads::protocol::results::ApiError> {
            if matches!(command, Command::CheckIn(_)) {
                self.calls.fetch_add(1, Ordering::SeqCst);
            }
            self.inner.handle_with_output(command, peer, budget, output)
        }
    }
    let root = std::env::temp_dir().join(format!("herdr-cli-ipc-{}", Uuid::new_v4()));
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    let state = root.join("state");
    let host = root.join("host.sock");
    let context = RuntimeContext::explicit(state.clone(), host.clone(), None).unwrap();
    let paths = InstancePaths::resolve(&context).unwrap();
    let owner = OwnerLock::acquire(&paths).unwrap();
    let instance = owner.instance_uuid();
    drop(owner);
    let setup = StoreContext::new(paths.database_path.clone(), Arc::new(SystemClock::new()));
    let db = setup.open_writer().unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at,host_boot,host_epoch) VALUES (?1,0,'b',1)",
        [instance.to_string()],
    )
    .unwrap();
    db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('seat',?1,'resolved','native','pane',1,0,0)", [instance.to_string()]).unwrap();
    db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,occupancy,ui_state,top_level_occupant,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES (?1,'pane','b',1,0,1,'fresh','unknown','unknown',0,0,'term-'||'pane','inc','coherent_enumeration',1)", [instance.to_string()]).unwrap();
    db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('recipient',?1,'resolved','native','other',1,0,0)", [instance.to_string()]).unwrap();
    db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,occupancy,ui_state,top_level_occupant,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES (?1,'other','b',1,0,2,'fresh','unknown','unknown',0,0,'term-'||'other','inc','coherent_enumeration',1)", [instance.to_string()]).unwrap();
    drop(db);
    let stop = Cancellation::default();
    let worker_stop = stop.clone();
    let worker_paths = paths.clone();
    let check_in_dispatches = Arc::new(AtomicU64::new(0));
    let counted = Arc::clone(&check_in_dispatches);
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        use herdr_threads::{
            daemon::{
                control::{ControlService, StopController},
                health::HealthInputs,
                run_elected_with_diagnostics,
            },
            ports::{LocalService, StorePort},
            service::{dispatch::DomainService, fair_writer::FairWriter},
            store::{SqliteStore, StoreSettings},
        };
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let service_clock = Arc::clone(&clock);
        let database = worker_paths.database_path.clone();
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(run_elected_with_diagnostics(
                &worker_paths,
                clock,
                worker_stop,
                move |instance, boot, cancellation| {
                    let store: Arc<dyn StorePort> = Arc::new(
                        SqliteStore::new(
                            StoreContext::new(database, Arc::clone(&service_clock)),
                            instance.to_string(),
                            StoreSettings::default(),
                        )
                        .map_err(|error| std::io::Error::other(error.detail))?,
                    );
                    let domain = DomainService::new(instance.to_string(), store, service_clock)
                        .with_cooperative_owner(
                            unsafe { libc::geteuid() },
                            Arc::new(FairWriter::new(32)),
                        );
                    let inner = Arc::new(ControlService::new(
                        StopController::new(instance, boot, cancellation),
                        move |_: &CallBudget| HealthInputs::unknown(instance, boot),
                        domain,
                    )) as Arc<dyn LocalService>;
                    Ok(Arc::new(CountCheckIns {
                        inner,
                        calls: Arc::clone(&counted),
                    }) as Arc<dyn LocalService>)
                },
                move |descriptor| {
                    ready_tx
                        .send(descriptor.clone())
                        .map_err(std::io::Error::other)
                },
                || async { Ok(()) },
            ))
            .unwrap();
    });
    let descriptor = ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(descriptor.instance_uuid, instance);
    let args = [
        "herdr-threads",
        "--state-dir",
        state.to_str().unwrap(),
        "--host-endpoint",
        host.to_str().unwrap(),
        "--cooperative-seat",
        "seat",
        "--cooperative-target",
        "pane",
        "--cooperative-harness",
        "codex",
        "--cooperative-role",
        "top-level",
        "check-in",
        "--lifecycle-event",
        "launch-one",
    ];
    let mut wrong_target = args;
    wrong_target[8] = "other";
    let rejected = herdr_threads::cli::run(wrong_target, &mut Vec::new()).unwrap_err();
    assert!(
        matches!(rejected, herdr_threads::cli::RunError::Api(ref error) if error.code == ErrorCode::TargetUnresolved)
    );
    let operations_before: i64 = rusqlite::Connection::open(&paths.database_path)
        .unwrap()
        .query_row("SELECT count(*) FROM operations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        operations_before, 0,
        "wrong target must fail before CheckIn dispatch"
    );
    let mut out = Vec::new();
    herdr_threads::cli::run(args, &mut out).unwrap();
    let output = String::from_utf8(out).unwrap();
    assert!(output.contains("seat"), "{output}");
    let db = rusqlite::Connection::open(&paths.database_path).unwrap();
    let generation: i64 = db
        .query_row("SELECT generation FROM seats WHERE id='seat'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(generation, 2);
    let count: i64 = db.query_row("SELECT count(*) FROM occupant_bindings WHERE seat_id='seat' AND ended_at IS NULL AND observation_provenance='cooperative_top_level'", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 1);
    drop(db);
    let invoke = |seat: &str, target: &str, command: &[&str]| -> serde_json::Value {
        let mut args = vec![
            "herdr-threads",
            "--json",
            "--state-dir",
            state.to_str().unwrap(),
            "--host-endpoint",
            host.to_str().unwrap(),
            "--cooperative-seat",
            seat,
            "--cooperative-target",
            target,
            "--cooperative-harness",
            "codex",
            "--cooperative-role",
            "top-level",
        ];
        args.extend_from_slice(command);
        let mut out = Vec::new();
        herdr_threads::cli::run(args, &mut out).unwrap();
        serde_json::from_slice(&out).unwrap()
    };
    let created = invoke("seat", "pane", &["thread", "create", "--topic", "shared"]);
    assert_eq!(created["result"]["kind"], "thread_created");
    let thread = created["result"]["data"].as_str().unwrap();
    let invited = invoke("seat", "pane", &["invite", thread, "--seat", "recipient"]);
    assert_eq!(invited["result"]["kind"], "invitation");
    let sent = invoke(
        "seat",
        "pane",
        &[
            "send",
            thread,
            "--body",
            "pending mail",
            "--require-ack",
            "recipient",
        ],
    );
    assert_eq!(sent["result"]["kind"], "message_sent");
    let message = sent["result"]["data"].as_str().unwrap();
    let receipt_before: i64 = rusqlite::Connection::open(&paths.database_path).unwrap()
        .query_row("SELECT count(*) FROM prepared_recipients pr JOIN send_manifests sm ON sm.preparation_id=pr.preparation_id WHERE sm.message_id=?1 AND pr.seat_id='recipient'", [message], |r| r.get(0)).unwrap();
    assert_eq!(receipt_before, 1);
    let recipient_checkin = invoke(
        "recipient",
        "other",
        &["check-in", "--lifecycle-event", "recipient-launch"],
    );
    assert_eq!(recipient_checkin["result"]["kind"], "checked_in");
    assert_eq!(
        invoke(
            "recipient",
            "other",
            &["check-in", "--lifecycle-event", "recipient-launch"]
        ),
        recipient_checkin,
        "a repeated lifecycle event must replay its original context",
    );
    let accepted = invoke("recipient", "other", &["accept", thread]);
    assert_eq!(accepted["result"]["kind"], "accepted");
    let receipt_after_accept: i64 = rusqlite::Connection::open(&paths.database_path).unwrap()
        .query_row("SELECT count(*) FROM prepared_recipients pr JOIN send_manifests sm ON sm.preparation_id=pr.preparation_id WHERE sm.message_id=?1 AND pr.seat_id='recipient'", [message], |r| r.get(0)).unwrap();
    assert_eq!(receipt_after_accept, 1);
    let acknowledged = invoke("recipient", "other", &["ack", message]);
    assert_eq!(acknowledged["result"]["kind"], "acknowledged");
    let receipt_after_ack: i64 = rusqlite::Connection::open(&paths.database_path).unwrap()
        .query_row("SELECT count(*) FROM receipt_state WHERE message_id=?1 AND seat_id='recipient' AND state='acked' AND acked_at IS NOT NULL AND ack_generation=2", [message], |r| r.get(0)).unwrap();
    assert_eq!(receipt_after_ack, 1);
    let prior: herdr_threads::protocol::authority::CallerClaim =
        serde_json::from_value(recipient_checkin["result"]["data"]["context"].clone()).unwrap();
    let successor = invoke(
        "recipient",
        "other",
        &["check-in", "--lifecycle-event", "recipient-restart"],
    );
    assert_eq!(
        successor["result"]["data"]["context"]["binding_generation"],
        3
    );
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let client = LocalSocketClient::new(
        descriptor.endpoint,
        Arc::clone(&clock),
        instance,
        Some(descriptor.boot_id),
    );
    let stale = herdr_threads::ports::LocalClient::call(
        &client,
        Command::Ack(herdr_threads::protocol::commands::Ack {
            messages: vec![herdr_threads::protocol::ids::MessageId::new(message)],
            operation: herdr_threads::protocol::ids::OperationId::new(Uuid::new_v4().to_string()),
            claim: prior,
        }),
        &CallBudget {
            deadline: MonoInstant(clock.monotonic_now().0 + 5_000),
            cancellation: Cancellation::default(),
        },
    )
    .unwrap_err();
    assert_eq!(stale.code, ErrorCode::CallerUnverified);
    // A committed CheckIn whose presentation failed remains recoverable after
    // another caller advances the service. The original CLI context stays behind.
    struct LostOutput;
    impl std::io::Write for LostOutput {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "lost CheckIn output",
            ))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let lost_args = [
        "herdr-threads",
        "--json",
        "--state-dir",
        state.to_str().unwrap(),
        "--host-endpoint",
        host.to_str().unwrap(),
        "--cooperative-seat",
        "recipient",
        "--cooperative-target",
        "other",
        "--cooperative-harness",
        "codex",
        "--cooperative-role",
        "top-level",
        "check-in",
        "--lifecycle-event",
        "lost-offer",
        "--native-session",
        "native-original",
    ];
    assert!(herdr_threads::cli::run(lost_args, &mut LostOutput).is_err());
    let context_dir = paths
        .instance_dir
        .canonicalize()
        .unwrap()
        .join("contexts")
        .join(format!("{:x}", sha2::Sha256::digest(b"recipient")));
    let contexts = herdr_threads::harness::context::ContextJournal::open(
        &context_dir,
        instance,
        "recipient",
        Duration::from_secs(1),
    )
    .unwrap();
    let predecessor = contexts.current().unwrap().unwrap();
    assert_eq!(predecessor.binding_generation, 4);
    let next = predecessor
        .for_event(
            herdr_threads::harness::context::EventKind::Restart,
            Uuid::new_v4(),
            None,
        )
        .unwrap();
    let next_claim = herdr_threads::harness::bridge::caller_claim(&next).unwrap();
    let next_result = herdr_threads::ports::LocalClient::call(
        &client,
        Command::CheckIn(herdr_threads::protocol::commands::CheckIn {
            mode: herdr_threads::protocol::commands::CheckInMode::Lifecycle {
                expected_binding_generation: 4,
            },
            claim: next_claim,
            operation: herdr_threads::protocol::ids::OperationId::new(Uuid::new_v4().to_string()),
        }),
        &CallBudget {
            deadline: MonoInstant(clock.monotonic_now().0 + 5_000),
            cancellation: Cancellation::default(),
        },
    );
    let next_result = next_result.unwrap();
    let CommandResult::CheckedIn(next_check) = next_result else {
        panic!("missing successor CheckIn")
    };
    assert_eq!(next_check.context.binding_generation, 5);
    let before: (i64, i64, i64) = rusqlite::Connection::open(&paths.database_path).unwrap()
        .query_row("SELECT (SELECT generation FROM seats WHERE id='recipient'),
            (SELECT count(*) FROM operations),
            (SELECT count(*) FROM occupant_bindings WHERE seat_id='recipient' AND ended_at IS NULL AND generation=5)",
            [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
    assert_eq!(before.0, 5);
    assert_eq!(before.2, 1);
    let omitted_native = lost_args[..17].to_vec();
    let mut changed_native = lost_args;
    changed_native[18] = "native-edited";
    assert!(herdr_threads::cli::run(omitted_native.clone(), &mut Vec::new()).is_err());
    assert!(herdr_threads::cli::run(changed_native, &mut Vec::new()).is_err());
    let mut recovered = Vec::new();
    herdr_threads::cli::run(lost_args, &mut recovered).unwrap();
    let recovered: serde_json::Value = serde_json::from_slice(&recovered).unwrap();
    assert_eq!(
        recovered["result"]["data"]["context"]["binding_generation"],
        4
    );
    assert_eq!(contexts.current().unwrap().unwrap(), predecessor);
    let journal =
        herdr_threads::cli::journal::Journal::open(paths.instance_dir.join("intents")).unwrap();
    assert!(
        journal
            .page(&herdr_threads::protocol::pagination::PageRequest {
                cursor: None,
                limit: 10,
                max_bytes: herdr_threads::protocol::pagination::MAX_PAGE_BYTES,
            })
            .unwrap()
            .items
            .is_empty(),
        "successful output flush removes the CLI intent"
    );
    assert!(contexts.pending().unwrap().is_none());
    assert!(
        contexts.request_for_event("lost-offer").unwrap().is_some(),
        "completed context remains cached"
    );
    assert!(herdr_threads::cli::run(omitted_native, &mut Vec::new()).is_err());
    assert!(herdr_threads::cli::run(changed_native, &mut Vec::new()).is_err());
    let dispatched_before = check_in_dispatches.load(Ordering::SeqCst);
    let mut completed_replay = Vec::new();
    herdr_threads::cli::run(lost_args, &mut completed_replay).unwrap();
    let completed_replay: serde_json::Value = serde_json::from_slice(&completed_replay).unwrap();
    assert_eq!(completed_replay, recovered);
    assert_eq!(
        check_in_dispatches.load(Ordering::SeqCst),
        dispatched_before,
        "completed replay must not dispatch CheckIn to elected service"
    );
    assert_eq!(contexts.current().unwrap().unwrap(), predecessor);
    // A mutation through the stale local context (generation 4 against the
    // service's 5) stays refused before dispatch.
    assert!(matches!(herdr_threads::cli::run([
        "herdr-threads", "--json", "--state-dir", state.to_str().unwrap(),
        "--host-endpoint", host.to_str().unwrap(), "--cooperative-seat", "recipient",
        "--cooperative-target", "other", "--cooperative-harness", "codex",
        "--cooperative-role", "top-level", "ack", message,
    ], &mut Vec::new()), Err(herdr_threads::cli::RunError::Api(error)) if error.code == ErrorCode::TargetUnresolved));
    // Root decision (wave-2 fix1 (b)): a fresh lifecycle event is a
    // deliberate new registration at the service's current generation (5),
    // not a replay; it passes the context gate and replaces the local
    // context. Kills: the gate refusing it (fix1 S2) and seeding it from the
    // stale local generation 4 (the service would refuse that CAS).
    let mut changed = lost_args;
    changed[16] = "changed-key";
    let mut fresh = Vec::new();
    herdr_threads::cli::run(changed, &mut fresh).unwrap();
    let fresh: serde_json::Value = serde_json::from_slice(&fresh).unwrap();
    assert_eq!(fresh["result"]["data"]["context"]["binding_generation"], 6);
    assert_eq!(contexts.current().unwrap().unwrap().binding_generation, 6);
    // A completed socket inspection fences the no-late-effect assertion.
    let _ = herdr_threads::ports::LocalClient::call(
        &client,
        Command::SeatInspect(herdr_threads::protocol::commands::SeatInspectQuery {
            seat: herdr_threads::protocol::ids::SeatId::new("recipient"),
            page: herdr_threads::protocol::pagination::PageRequest {
                cursor: None,
                limit: 1,
                max_bytes: herdr_threads::protocol::pagination::MAX_PAGE_BYTES,
            },
        }),
        &CallBudget {
            deadline: MonoInstant(clock.monotonic_now().0 + 5_000),
            cancellation: Cancellation::default(),
        },
    )
    .unwrap();
    stop.cancel();
    worker.join().unwrap();
    let after: (i64, i64, i64) = rusqlite::Connection::open(&paths.database_path).unwrap()
        .query_row("SELECT (SELECT generation FROM seats WHERE id='recipient'),
            (SELECT count(*) FROM operations),
            (SELECT count(*) FROM occupant_bindings WHERE seat_id='recipient' AND ended_at IS NULL AND generation=5)",
            [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
    // Exactly the fresh registration took effect: one new operation, the
    // generation-5 binding replaced by the generation-6 one.
    assert_eq!(after, (6, before.1 + 1, 0));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn health_unknown_settings_stay_absent_and_resolved_settings_are_forwarded() {
    use herdr_threads::{daemon::health::HealthInputs, protocol::results::HealthSettings};
    let mut inputs = HealthInputs::unknown(Uuid::new_v4(), Uuid::new_v4());
    assert!(inputs.assemble().settings.is_none());
    inputs = HealthInputs::unknown(Uuid::new_v4(), Uuid::new_v4());
    inputs.settings = Some(HealthSettings {
        invitation_default_ms: 120_000,
        receipt_default_ms: 240_000,
        minimum_wake_delay_ms: 45_000,
    });
    let actual = inputs.assemble().settings.unwrap();
    assert_eq!(actual.invitation_default_ms, 120_000);
    assert_eq!(actual.receipt_default_ms, 240_000);
    assert_eq!(actual.minimum_wake_delay_ms, 45_000);
}

#[test]
fn elected_service_opens_real_sqlite_and_routes_health_and_public_query_over_ipc() {
    elected_service_fixture(false);
}

#[test]
fn elected_service_reports_actual_private_settings_over_ipc() {
    elected_service_fixture(true);
}

fn elected_service_fixture(custom_settings: bool) {
    let root = std::env::temp_dir().join(format!("herdr-service-{}", Uuid::new_v4()));
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    let context =
        RuntimeContext::explicit(root.join("state"), root.join("host.sock"), None).unwrap();
    let paths = InstancePaths::resolve(&context).unwrap();
    let owner = OwnerLock::acquire(&paths).unwrap();
    let instance = owner.instance_uuid();
    drop(owner);
    let setup = StoreContext::new(paths.database_path.clone(), Arc::new(SystemClock::new()));
    let db = setup.open_writer().unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at) VALUES (?1,0)",
        [instance.to_string()],
    )
    .unwrap();
    db.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('s2',?1,'resolved','native',1,0)", [instance.to_string()]).unwrap();
    db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t',?1,'topic','goal',0,0)", [instance.to_string()]).unwrap();
    db.execute_batch("INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t','s2','invited');
        INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms)
        VALUES ('inv','t','s2',1,'pending',0,1,100,100);").unwrap();
    if custom_settings {
        db.execute_batch("CREATE TRIGGER health_due_failure BEFORE INSERT ON messages WHEN NEW.kind='warn' BEGIN SELECT RAISE(ABORT,'health fixture due failure'); END;").unwrap();
    }
    drop(db);
    if custom_settings {
        let file = paths.instance_dir.join("settings.json");
        fs::write(&file, br#"{"invitation_default_ms":120000,"receipt_default_ms":240000,"minimum_wake_delay_ms":45000}"#).unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
    }
    let mut child = TestChild(
        super::scrubbed_command(env!("CARGO_BIN_EXE_herdr-threads"))
            .args([
                "daemon",
                "run",
                "--state-dir",
                root.join("state").to_str().unwrap(),
                "--host-endpoint",
                root.join("host.sock").to_str().unwrap(),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn_owned()
            .unwrap(),
    );
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let deadline = Instant::now() + Duration::from_secs(5);
    let descriptor = loop {
        if let Some(instance) = read_existing_namespace(&paths).unwrap()
            && let Ok(descriptor) = read_descriptor(&paths, instance)
        {
            break descriptor;
        }
        if let Some(status) = child.0.try_wait().unwrap() {
            let mut stderr = String::new();
            child
                .0
                .stderr
                .take()
                .unwrap()
                .read_to_string(&mut stderr)
                .unwrap();
            panic!("child exited before publication ({status}): {stderr}");
        }
        assert!(Instant::now() < deadline, "child did not publish endpoint");
        std::thread::sleep(Duration::from_millis(10));
    };
    let client = LocalSocketClient::new(
        descriptor.endpoint.clone(),
        Arc::clone(&clock),
        descriptor.instance_uuid,
        Some(descriptor.boot_id),
    );
    let budget = || CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0 + 3_000),
        cancellation: Cancellation::default(),
    };
    if custom_settings {
        let until = Instant::now() + Duration::from_secs(3);
        loop {
            let CommandResult::Health(health) =
                herdr_threads::ports::LocalClient::call(&client, Command::Health, &budget())
                    .unwrap()
            else {
                panic!("missing health");
            };
            // The actual SQLite due failure reaches app Health only as its
            // typed redacted class and code, never the private trigger text.
            assert!(
                !health
                    .limitations
                    .iter()
                    .any(|detail| detail.contains("health fixture due failure")),
                "private due failure text reached app Health: {:?}",
                health.limitations
            );
            if health
                .limitations
                .iter()
                .any(|detail| detail == "scheduler degraded: invitation due scan failed: Conflict")
                // ...and the daemon-log pointer (`degraded: <remedy naming daemon.log>`)
                // sits on its own line.
                && health
                    .limitations
                    .iter()
                    .any(|detail| detail.starts_with("degraded: "))
            {
                break;
            }
            assert!(
                Instant::now() < until,
                "actual due failure did not reach app Health: {:?}",
                health.limitations
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let db = rusqlite::Connection::open(&paths.database_path).unwrap();
        db.execute_batch("DROP TRIGGER health_due_failure").unwrap();
    }
    let warning_deadline =
        Instant::now() + Duration::from_secs(if custom_settings { 8 } else { 3 });
    loop {
        let db = rusqlite::Connection::open(&paths.database_path).unwrap();
        let warnings: i64 = db
            .query_row(
                "SELECT count(*) FROM messages WHERE thread_id='t' AND kind='warn'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        if warnings == 1 {
            break;
        }
        assert!(
            Instant::now() < warning_deadline,
            "boot worker did not commit overdue warning"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    // Elected composition of the observation worker's WorkerStatus: the
    // host socket never exists, so the real observation worker started by
    // `app.rs` records `Frozen(HostUnavailable)`: Herdr is unavailable, so
    // nothing is invalidated (TRUST-POLICY C4, ht-yms). It must reach the
    // Health builder through the *same* shared WorkerStatus, rendered only
    // as its redacted class, with no private path text anywhere in Health.
    //
    // Kills: passing a fresh `WorkerStatus` (instead of
    // `factory_observation_status`) to `start_observation_worker` in
    // `src/app.rs`: the observation lane's failure is then absent from
    // elected Health and this wait times out.
    let observation_deadline = Instant::now() + Duration::from_secs(8);
    let root_text = root.to_str().unwrap().to_owned();
    loop {
        let CommandResult::Health(health) =
            herdr_threads::ports::LocalClient::call(&client, Command::Health, &budget()).unwrap()
        else {
            panic!("missing health");
        };
        assert!(
            !health
                .limitations
                .iter()
                // The one deliberate path: the daemon-log pointer (ht-p03.27).
                .filter(|detail| !detail.starts_with("degraded: "))
                .any(|detail| detail.contains(&root_text) || detail.contains("host.sock")),
            "private host path reached elected Health: {:?}",
            health.limitations
        );
        // The lane Pacer's backoff suffix (`; retrying (attempt N, ...)`)
        // follows the redacted failure while the host stays down.
        if health.limitations.iter().any(|detail| {
            detail.starts_with(
                "scheduler degraded: host unavailable (HostUnavailable): \
                 seats and bindings frozen until Herdr answers",
            )
        }) {
            break;
        }
        assert!(
            Instant::now() < observation_deadline,
            "elected observation worker failure never reached app Health: {:?}",
            health.limitations
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let CommandResult::Health(health) =
        herdr_threads::ports::LocalClient::call(&client, Command::Health, &budget()).unwrap()
    else {
        panic!("missing health");
    };
    assert_eq!(health.state, HealthState::Degraded);
    assert_eq!(health.instance_id, descriptor.instance_uuid.to_string());
    if custom_settings {
        assert!(
            !health
                .limitations
                .iter()
                .any(|detail| detail.contains("health fixture due failure")),
            "successful affected due retry remained in app Health: {:?}",
            health.limitations
        );
        assert_eq!(
            health.host.current_execution,
            herdr_threads::protocol::results::CapabilityState::Unsupported
        );
    }
    if custom_settings {
        assert_eq!(
            health.settings.as_ref().unwrap().invitation_default_ms,
            120_000
        );
        assert_eq!(
            health.settings.as_ref().unwrap().receipt_default_ms,
            240_000
        );
        assert_eq!(
            health.settings.as_ref().unwrap().minimum_wake_delay_ms,
            45_000
        );
    } else {
        assert_eq!(
            health.settings.as_ref().unwrap().invitation_default_ms,
            300_000
        );
        assert_eq!(
            health.settings.as_ref().unwrap().receipt_default_ms,
            300_000
        );
        assert_eq!(
            health.settings.as_ref().unwrap().minimum_wake_delay_ms,
            30_000
        );
    }
    let result = herdr_threads::ports::LocalClient::call(
        &client,
        Command::Directory(DirectoryQuery {
            membership: None,
            membership_filter: DirectoryMembership::All,
            topic_contains: None,
            page: PageRequest::default(),
        }),
        &budget(),
    )
    .unwrap();
    let CommandResult::Directory(page) = result else {
        panic!("missing directory");
    };
    assert_eq!(page.items.len(), 1);
    let db = rusqlite::Connection::open(&paths.database_path).unwrap();
    db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t2',?1,?2,'goal',1,1)", rusqlite::params![instance.to_string(), "second \"topic\"\n\u{2028}"]).unwrap();
    drop(db);
    let selected = OutputSpec {
        format: OutputFormat::Text,
        context: ContinuationContext {
            state_dir: Some(root.join("state").to_str().unwrap().to_owned()),
            host: Some(root.join("host.sock").to_str().unwrap().into()),
        },
    };
    let listing = Command::Directory(DirectoryQuery {
        membership: None,
        membership_filter: DirectoryMembership::All,
        topic_contains: None,
        page: PageRequest {
            cursor: None,
            // ht-4is.8.18: the compact selected text (no page blob, one
            // short `c3:` continuation) fits 2 KiB; the smallest legal
            // budget still cannot hold the long selector paths.
            limit: 1,
            max_bytes: 256,
        },
    });
    let selected_error = client
        .call_with_output(listing.clone(), &selected, &budget())
        .unwrap_err();
    assert_eq!(selected_error.code, ErrorCode::InvalidBudget);
    assert!(selected_error.required_minimum_bytes.unwrap() > 256);
    let mut listing = listing;
    if let Command::Directory(query) = &mut listing {
        query.page.max_bytes = 2_048;
    }
    let mut selected_listing = listing.clone();
    if let Command::Directory(query) = &mut selected_listing {
        query.page.max_bytes = 4_096;
    }
    let selected_result = herdr_threads::ports::LocalClient::call_with_output(
        &client,
        selected_listing,
        &selected,
        &budget(),
    )
    .unwrap();
    let bound = herdr_threads::cli::SelectedSocketClient {
        client: &client,
        output: &selected,
    };
    let mismatch = herdr_threads::ports::LocalClient::call_with_output(
        &bound,
        Command::Health,
        &OutputSpec::default(),
        &budget(),
    )
    .unwrap_err();
    assert_eq!(mismatch.code, ErrorCode::InvalidRequest);
    let CommandResult::Directory(selected_page) = &selected_result else {
        panic!("missing selected directory");
    };
    assert!(selected_page.has_more);
    let next = selected_page.next_argv.as_ref().unwrap();
    assert!(
        next.windows(2)
            .any(|args| args == ["--state-dir", root.join("state").to_str().unwrap()])
    );
    assert!(
        next.windows(2)
            .any(|args| args == ["--host-endpoint", root.join("host.sock").to_str().unwrap()])
    );
    assert!(encode_selected(&selected_result, &selected).unwrap().len() <= 4_096);
    let next_result = client
        .call_with_output(
            Command::Directory(DirectoryQuery {
                membership: None,
                membership_filter: DirectoryMembership::All,
                topic_contains: None,
                page: PageRequest {
                    cursor: selected_page.next_cursor.clone(),
                    limit: 1,
                    max_bytes: 4_096,
                },
            }),
            &selected,
            &budget(),
        )
        .unwrap();
    let escaped = String::from_utf8(encode_selected(&next_result, &selected).unwrap()).unwrap();
    assert!(escaped.contains("\\u2028"), "{escaped}");
    assert!(!escaped.contains('\u{2028}'));
    let mut selected_cli = Vec::new();
    herdr_threads::cli::run(
        [
            "herdr-threads",
            "--state-dir",
            root.join("state").to_str().unwrap(),
            "--host-endpoint",
            root.join("host.sock").to_str().unwrap(),
            "thread",
            "list",
            "--all",
            "--limit",
            "1",
            "--max-bytes",
            "4096",
        ],
        &mut selected_cli,
    )
    .unwrap();
    assert_eq!(
        selected_cli,
        encode_selected(&selected_result, &selected).unwrap()
    );
    assert!(String::from_utf8_lossy(&selected_cli).starts_with("directory\n"));
    let default_result =
        herdr_threads::ports::LocalClient::call(&client, listing, &budget()).unwrap();
    let CommandResult::Directory(default_page) = default_result else {
        panic!("missing default directory");
    };
    assert!(
        !default_page
            .next_argv
            .unwrap()
            .contains(&"--state-dir".to_owned())
    );
    let mut rendered = Vec::new();
    herdr_threads::cli::run(
        [
            "herdr-threads",
            "--json",
            "--state-dir",
            root.join("state").to_str().unwrap(),
            "--host-endpoint",
            root.join("host.sock").to_str().unwrap(),
            "daemon",
            "health",
        ],
        &mut rendered,
    )
    .unwrap();
    let output: serde_json::Value = serde_json::from_slice(&rendered).unwrap();
    assert_eq!(
        output["result"]["data"]["instance_id"],
        descriptor.instance_uuid.to_string()
    );
    assert_eq!(output["result"]["data"]["state"], "degraded");
    let binary = super::scrubbed_command(env!("CARGO_BIN_EXE_herdr-threads"))
        .args([
            "--json",
            "--state-dir",
            root.join("state").to_str().unwrap(),
            "--host-endpoint",
            root.join("host.sock").to_str().unwrap(),
            "daemon",
            "health",
        ])
        .output()
        .unwrap();
    assert!(
        binary.status.success(),
        "binary failed: {}",
        String::from_utf8_lossy(&binary.stderr)
    );
    let output: serde_json::Value = serde_json::from_slice(&binary.stdout).unwrap();
    assert_eq!(
        output["result"]["data"]["boot_id"],
        descriptor.boot_id.to_string()
    );
    let mut view = Vec::new();
    herdr_threads::cli::run(
        [
            "herdr-threads",
            "--state-dir",
            root.join("state").to_str().unwrap(),
            "--host-endpoint",
            root.join("host.sock").to_str().unwrap(),
            "view",
            "--once",
        ],
        &mut view,
    )
    .unwrap();
    let view = String::from_utf8(view).unwrap();
    assert!(view.starts_with("herdr-threads view\n"));
    assert!(view.contains("health"));
    let error = herdr_threads::ports::LocalClient::call(&client, Command::CheckIn(
        serde_json::from_value(serde_json::json!({"claim":{"instance":"unconfigured-fixture","seat":"s","binding_generation":0,"role":"top_level","harness":"codex","native_session":"s","execution":"e","target":"p"},"mode":{"kind":"current"},"operation":"op"})).unwrap()
    ), &budget()).unwrap_err();
    assert_eq!(error.code, ErrorCode::CallerUnverified);
    herdr_threads::daemon::control::stop_and_wait(
        &client,
        &paths,
        &descriptor,
        clock.as_ref(),
        &budget(),
    )
    .unwrap();
    assert!(child.0.wait().unwrap().success());
    assert!(paths.database_path.exists());
    fs::remove_dir_all(root).unwrap();
}

struct HungWakeHost {
    entered: std::sync::mpsc::SyncSender<()>,
    release: Arc<std::sync::atomic::AtomicBool>,
}
impl herdr_threads::ports::HostPort for HungWakeHost {
    fn native_launch_capability(&self) -> herdr_threads::ports::NativeLaunchCapability {
        herdr_threads::ports::NativeLaunchCapability::Unsupported
    }
    fn observe_current_target(
        &self,
        _: &HostTargetId,
        context: &herdr_threads::ports::HostCallContext,
    ) -> Result<herdr_threads::ports::HostObservation, herdr_threads::protocol::results::ApiError>
    {
        let _ = self.entered.try_send(());
        // Deliberately retain this physical call after cancellation until the
        // test releases it. The daemon must retain elected ownership meanwhile.
        while !self.release.load(std::sync::atomic::Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(context.budget.cancellation.is_cancelled());
        Err(herdr_threads::protocol::results::ApiError::cancelled(
            "fixture released after cancellation",
        ))
    }
    fn enumerate_targets(
        &self,
        _: &herdr_threads::ports::HostCallContext,
    ) -> Result<herdr_threads::ports::HostSnapshot, herdr_threads::protocol::results::ApiError>
    {
        while !self.release.load(std::sync::atomic::Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(5));
        }
        Err(herdr_threads::protocol::results::ApiError::unsupported(
            "fixture has no enumeration authority",
        ))
    }
    fn safe_wake_target(
        &self,
        _: &herdr_threads::protocol::ids::SeatId,
        _: &herdr_threads::ports::HostObservation,
    ) -> Option<herdr_threads::ports::SafeWakeTarget> {
        unreachable!()
    }
    fn submit_prompt(
        &self,
        _: &herdr_threads::ports::SafeWakeTarget,
        _: &str,
        _: &herdr_threads::ports::HostCallContext,
    ) -> Result<herdr_threads::ports::PromptOutcome, herdr_threads::protocol::results::ApiError>
    {
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
    ) -> Result<herdr_threads::ports::NativeLaunchOutcome, herdr_threads::protocol::results::ApiError>
    {
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

#[test]
fn elected_wake_hang_allows_health_deadlines_and_retains_owner_until_joined() {
    let _stdio_guard = super::IN_PROCESS_DAEMON
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    use herdr_threads::protocol::commands::StopRequest;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    };
    let root = std::env::temp_dir().join(format!("herdr-wake-service-{}", Uuid::new_v4()));
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    let context =
        RuntimeContext::explicit(root.join("state"), root.join("host.sock"), None).unwrap();
    let paths = InstancePaths::resolve(&context).unwrap();
    let owner = OwnerLock::acquire(&paths).unwrap();
    let instance = owner.instance_uuid();
    drop(owner);
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let db = StoreContext::new(paths.database_path.clone(), Arc::clone(&clock))
        .open_writer()
        .unwrap();
    db.execute("INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES (?1,0,'host',1,1)", [instance.to_string()]).unwrap();
    db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('wake',?1,'resolved','native','pane',1,1,0)", [instance.to_string()]).unwrap();
    db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,occupancy,ui_state,verified_execution,top_level_occupant,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES (?1,'pane','host',1,1,0,'fresh','occupied','idle','exec',1,'term-'||'pane','inc','coherent_enumeration',1)", [instance.to_string()]).unwrap();
    db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t',?1,'topic','goal',0,0)", [instance.to_string()]).unwrap();
    db.execute_batch("INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t','wake','invited'); INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES ('inv','t','wake',1,'pending',0,1,9223372036854775807,300000);").unwrap();
    drop(db);
    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let release = Arc::new(AtomicBool::new(false));
    let host = Arc::new(HungWakeHost {
        entered: entered_tx,
        release: Arc::clone(&release),
    });
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let daemon_paths = paths.clone();
    let daemon_clock = Arc::clone(&clock);
    let shutdown = Cancellation::default();
    let fallback_stop = shutdown.clone();
    let daemon = std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(herdr_threads::app::run_elected(
                &daemon_paths,
                daemon_clock,
                shutdown,
                ServiceConfig::default(),
                host,
                move |descriptor: &herdr_threads::daemon::ownership::EndpointDescriptor| {
                    ready_tx.send(descriptor.clone()).unwrap();
                    Ok(())
                },
            ))
    });
    // Release and join even if an assertion fails, so no fixture worker escapes.
    struct Cleanup {
        release: Arc<AtomicBool>,
        stop: Cancellation,
        daemon: Option<std::thread::JoinHandle<std::io::Result<bool>>>,
    }
    impl Drop for Cleanup {
        fn drop(&mut self) {
            self.stop.cancel();
            self.release.store(true, Ordering::SeqCst);
            if let Some(worker) = self.daemon.take() {
                let _ = worker.join();
            }
        }
    }
    let mut cleanup = Cleanup {
        release,
        stop: fallback_stop,
        daemon: Some(daemon),
    };
    let descriptor = ready_rx.recv_timeout(Duration::from_secs(30)).unwrap();
    entered_rx
        .recv_timeout(Duration::from_secs(30))
        .expect("real committed reservation did not reach native dispatcher");
    let db = rusqlite::Connection::open(&paths.database_path).unwrap();
    let reserved: String = db
        .query_row(
            "SELECT reservation_id FROM wake_work WHERE seat_id='wake'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!reserved.is_empty());
    // Make a different invitation overdue only after the host call is held.
    db.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('due',?1,'unresolved','native',1,0)", [instance.to_string()]).unwrap();
    db.execute_batch("INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t','due','invited'); INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES ('due-inv','t','due',1,'pending',0,2,100,100);").unwrap();
    let client = LocalSocketClient::new(
        descriptor.endpoint.clone(),
        Arc::clone(&clock),
        descriptor.instance_uuid,
        Some(descriptor.boot_id),
    );
    let budget = || CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0 + 1_000),
        cancellation: Cancellation::default(),
    };
    assert!(matches!(
        herdr_threads::ports::LocalClient::call(&client, Command::Health, &budget()).unwrap(),
        CommandResult::Health(_)
    ));
    // The row above is inserted on a separate connection, so it fires no
    // commit hook and no kick: only the deadline lane's 5 s safety tick finds
    // it (ht-p03.9.4; it used to be a 1 s gate).
    let until = Instant::now() + Duration::from_secs(8);
    loop {
        let warnings: i64 = db
            .query_row("SELECT count(*) FROM messages WHERE kind='warn'", [], |r| {
                r.get(0)
            })
            .unwrap();
        if warnings > 0 {
            break;
        }
        assert!(
            Instant::now() < until,
            "held host prevented local deadline progress"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(matches!(
        herdr_threads::ports::LocalClient::call(
            &client,
            Command::Stop(StopRequest {
                expected_boot: descriptor.boot_id.to_string()
            }),
            &budget()
        )
        .unwrap(),
        CommandResult::StopAccepted(_)
    ));
    std::thread::sleep(Duration::from_millis(50));
    assert!(
        !cleanup.daemon.as_ref().unwrap().is_finished(),
        "daemon released owned hung attempt"
    );
    assert!(
        OwnerLock::acquire(&paths).is_err(),
        "elected lease released before wake join"
    );
    cleanup.release.store(true, Ordering::SeqCst);
    assert!(cleanup.daemon.take().unwrap().join().unwrap().unwrap());
    let settled: (Option<String>, String) = db
        .query_row(
            "SELECT reservation_id,last_outcome FROM wake_work WHERE seat_id='wake'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(settled, (None, "outcome_unknown".into()));
    drop(db);
    drop(OwnerLock::acquire(&paths).unwrap());
    fs::remove_dir_all(root).unwrap();
}

// This fixture serves only a private protocol endpoint. Every host operation
// goes through the production NativeCli; no HostPort capability is replaced.
struct ActualNativeRoot(std::path::PathBuf);
impl Drop for ActualNativeRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct ActualNativeFixture {
    _root: ActualNativeRoot,
    paths: InstancePaths,
    clock: Arc<dyn Clock>,
    descriptor: herdr_threads::daemon::ownership::EndpointDescriptor,
    stop: Cancellation,
    daemon: Option<std::thread::JoinHandle<std::io::Result<bool>>>,
    server_stop: Arc<std::sync::atomic::AtomicBool>,
    snapshot_entered: Arc<std::sync::atomic::AtomicBool>,
    peer_eof: Arc<std::sync::atomic::AtomicBool>,
    server: Option<std::thread::JoinHandle<()>>,
    native: std::sync::Weak<herdr_threads::host::native::NativeCli>,
    /// Kicks the daemon's lanes for rows written on a separate connection.
    lanes: herdr_threads::app::LaneProbe,
    endpoint: std::path::PathBuf,
    /// Live panes served by the private endpoint's snapshot and pane reads.
    panes: Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
    _stdio: std::sync::MutexGuard<'static, ()>,
}
fn actual_pane(pane: &str, terminal: &str) -> serde_json::Value {
    serde_json::json!({"pane_id":pane,"terminal_id":terminal,"workspace_id":"w4","tab_id":"w4:t1","focused":false,"agent_status":"idle","revision":7})
}
impl ActualNativeFixture {
    fn start(held_snapshot: bool, saved: usize) -> Self {
        Self::start_with(held_snapshot, saved, Vec::new())
    }
    fn start_with(held_snapshot: bool, saved: usize, panes: Vec<serde_json::Value>) -> Self {
        use std::io::{BufRead, BufReader, Write};
        use std::sync::atomic::{AtomicBool, Ordering};
        let stdio = super::IN_PROCESS_DAEMON
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let root = std::path::PathBuf::from("/private/tmp")
            .join(format!("herdr-actual-native-{}", Uuid::new_v4()));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let root_cleanup = ActualNativeRoot(root.clone());
        let endpoint = root.join("host.sock");
        let paths = InstancePaths::resolve(
            &RuntimeContext::explicit(root.join("state"), endpoint.clone(), None).unwrap(),
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
            "INSERT INTO host_instances(id,created_at) VALUES (?1,0)",
            [instance.to_string()],
        )
        .unwrap();
        for n in 0..saved {
            db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES (?1,?2,'resolved','native',?3,1,1,0)", rusqlite::params![format!("saved-{n}"),instance.to_string(),format!("absent-{n}")]).unwrap();
        }
        drop(db);
        let listener = std::os::unix::net::UnixListener::bind(&endpoint).unwrap();
        listener.set_nonblocking(true).unwrap();
        let server_stop = Arc::new(AtomicBool::new(false));
        let snapshot_entered = Arc::new(AtomicBool::new(false));
        let peer_eof = Arc::new(AtomicBool::new(false));
        let thread_stop = server_stop.clone();
        let entered = snapshot_entered.clone();
        let eof = peer_eof.clone();
        let panes = Arc::new(std::sync::Mutex::new(panes));
        let served = panes.clone();
        let server = std::thread::spawn(move || {
            while !thread_stop.load(Ordering::SeqCst) {
                let (mut stream, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("private endpoint accept: {error}"),
                };
                // macOS accept(2) inherits O_NONBLOCK from the listener: a
                // request read racing ahead of the client's write failed
                // WouldBlock under CPU load. The request read blocks; its
                // timeout is a hang guard, not a scheduling bound.
                // A client that already hung up can make these fail (EINVAL
                // on macOS). Blocking mode is required: without it, drop that
                // connection instead of killing the fake host. The timeout is
                // best effort: the buffered request is still readable.
                if stream.set_nonblocking(false).is_err() {
                    continue;
                }
                let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
                let mut request = String::new();
                BufReader::new(&mut stream).read_line(&mut request).unwrap();
                // The held snapshot polls for stop between short reads. macOS
                // rejects this (EINVAL) only once the peer closed, and the held
                // read then observes that EOF immediately.
                let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
                let request: serde_json::Value = serde_json::from_str(&request).unwrap();
                let result = match request["method"].as_str().unwrap() {
                    "ping" => serde_json::json!({"type":"pong","version":"0.9.1","protocol":22}),
                    "session.snapshot" => {
                        entered.store(true, Ordering::SeqCst);
                        if held_snapshot {
                            let mut byte = [0];
                            loop {
                                match stream.read(&mut byte) {
                                    Ok(0) => {
                                        eof.store(true, Ordering::SeqCst);
                                        break;
                                    }
                                    Ok(_) => panic!("unexpected bytes after snapshot request"),
                                    Err(error)
                                        if matches!(
                                            error.kind(),
                                            std::io::ErrorKind::WouldBlock
                                                | std::io::ErrorKind::TimedOut
                                        ) =>
                                    {
                                        if thread_stop.load(Ordering::SeqCst) {
                                            break;
                                        }
                                    }
                                    Err(error) => panic!("held snapshot peer: {error}"),
                                }
                            }
                            continue;
                        }
                        let panes = served.lock().unwrap().clone();
                        serde_json::json!({"type":"session_snapshot","snapshot":{"version":"0.9.1","protocol":22,"panes":panes,"agents":[],"tabs":[],"workspaces":[],"layouts":[]}})
                    }
                    "pane.get" => {
                        let wanted = request["params"]["pane_id"].clone();
                        let found = served
                            .lock()
                            .unwrap()
                            .iter()
                            .find(|pane| pane["pane_id"] == wanted)
                            .cloned();
                        match found {
                            Some(pane) => serde_json::json!({"type":"pane_info","pane":pane}),
                            // No configured panes: the legacy single-pane answer.
                            None if served.lock().unwrap().is_empty() => {
                                serde_json::json!({"type":"pane_info","pane":actual_pane("w4:p1","terminal")})
                            }
                            None => {
                                writeln!(
                                    stream,
                                    "{}",
                                    serde_json::json!({"id":request["id"],"error":{"code":"pane_not_found","message":"pane not found"}})
                                )
                                .unwrap();
                                continue;
                            }
                        }
                    }
                    method => panic!("unexpected native operation: {method}"),
                };
                // The observation lane starts a capture at once, so a daemon
                // shutting down (restart) can hang up with a request in
                // flight; a reply to a gone peer must not kill the fake host.
                if writeln!(
                    stream,
                    "{}",
                    serde_json::json!({"id":request["id"],"result":result})
                )
                .is_err()
                {
                    continue;
                }
                // Each accepted connection contains exactly one request. A
                // transport that reuses ping for the operation hits peer EOF.
            }
        });
        let host = Arc::new(herdr_threads::host::native::NativeCli::new(
            endpoint.clone(),
            Arc::clone(&clock),
        ));
        let native = Arc::downgrade(&host);
        let probe = if !held_snapshot {
            Some(herdr_threads::ports::HostPort::enumerate_targets(
                host.as_ref(),
                &herdr_threads::ports::HostCallContext {
                    budget: CallBudget {
                        deadline: MonoInstant(clock.monotonic_now().0 + 1_000),
                        cancellation: Cancellation::default(),
                    },
                    expected_boot: None,
                    expected_epoch: None,
                },
            ))
        } else {
            None
        };
        let stop = Cancellation::default();
        let thread_stop = stop.clone();
        let thread_paths = paths.clone();
        let thread_clock = clock.clone();
        let (ready, received) = std::sync::mpsc::sync_channel(1);
        let lanes = herdr_threads::app::LaneProbe::default();
        let thread_lanes = lanes.clone();
        let daemon = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(herdr_threads::app::run_elected_probed(
                    &thread_paths,
                    thread_clock,
                    thread_stop,
                    ServiceConfig::default(),
                    host,
                    thread_lanes,
                    move |descriptor| {
                        ready.send(descriptor.clone()).unwrap();
                        Ok(())
                    },
                ))
        });
        let descriptor = match received.recv_timeout(Duration::from_secs(30)) {
            Ok(descriptor) => descriptor,
            Err(error) => {
                stop.cancel();
                let outcome = daemon.join();
                server_stop.store(true, Ordering::SeqCst);
                let server_outcome = server.join();
                panic!("native fixture startup failed: {error}; {outcome:?}; {server_outcome:?}");
            }
        };
        let fixture = Self {
            _root: root_cleanup,
            paths,
            clock,
            descriptor,
            stop,
            daemon: Some(daemon),
            server_stop,
            snapshot_entered,
            peer_eof,
            server: Some(server),
            native,
            lanes,
            endpoint,
            panes,
            _stdio: stdio,
        };
        if let Some(snapshot) = probe {
            // The private endpoint is served by this process, so its response
            // connections carry a kernel-witnessed server incarnation (macOS).
            let snapshot = snapshot.unwrap();
            let verified = cfg!(target_os = "macos");
            assert_eq!(
                snapshot.enumeration,
                if verified {
                    herdr_threads::ports::EnumerationEvidence::CoherentVerified
                } else {
                    herdr_threads::ports::EnumerationEvidence::CompleteUnverified
                }
            );
            assert_eq!(
                matches!(
                    snapshot.incarnation,
                    herdr_threads::ports::IncarnationEvidence::Verified { .. }
                ),
                verified
            );
            assert_eq!(snapshot.authorizes_absence_closure(), verified);
        }
        fixture
    }
    /// Stop the elected daemon and start a new boot with a fresh production
    /// NativeCli (connection epoch 1) against the same private endpoint.
    fn restart(&mut self) {
        self.stop.cancel();
        assert!(self.daemon.take().unwrap().join().unwrap().unwrap());
        assert!(self.native.upgrade().is_none());
        self.stop = Cancellation::default();
        let host = Arc::new(herdr_threads::host::native::NativeCli::new(
            self.endpoint.clone(),
            Arc::clone(&self.clock),
        ));
        self.native = Arc::downgrade(&host);
        let paths = self.paths.clone();
        let clock = self.clock.clone();
        let stop = self.stop.clone();
        let (ready, received) = std::sync::mpsc::sync_channel(1);
        self.lanes = herdr_threads::app::LaneProbe::default();
        let lanes = self.lanes.clone();
        self.daemon = Some(std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(herdr_threads::app::run_elected_probed(
                    &paths,
                    clock,
                    stop,
                    ServiceConfig::default(),
                    host,
                    lanes,
                    move |descriptor| {
                        ready.send(descriptor.clone()).unwrap();
                        Ok(())
                    },
                ))
        }));
        self.descriptor = received.recv_timeout(Duration::from_secs(30)).unwrap();
    }
    fn resolve(
        &self,
        pane: &str,
        operation: &str,
    ) -> Result<CommandResult, herdr_threads::protocol::results::ApiError> {
        herdr_threads::ports::LocalClient::call(
            &self.client(),
            Command::ResolveSeat(herdr_threads::protocol::commands::ResolveSeat {
                target: HostTargetId::new(pane),
                operation: herdr_threads::protocol::ids::OperationId::new(operation),
            }),
            &self.budget(),
        )
    }
    fn wait_for(&self, what: &str, mut done: impl FnMut(&rusqlite::Connection) -> bool) {
        let db = self.db();
        let until = Instant::now() + Duration::from_secs(30);
        while !done(&db) {
            assert!(Instant::now() < until, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    fn budget(&self) -> CallBudget {
        CallBudget {
            deadline: MonoInstant(self.clock.monotonic_now().0 + 1_000),
            cancellation: Cancellation::default(),
        }
    }
    fn client(&self) -> LocalSocketClient {
        LocalSocketClient::new(
            self.descriptor.endpoint.clone(),
            self.clock.clone(),
            self.descriptor.instance_uuid,
            Some(self.descriptor.boot_id),
        )
    }
    fn db(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(&self.paths.database_path).unwrap()
    }
}
impl Drop for ActualNativeFixture {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(worker) = self.daemon.take() {
            let _ = worker.join();
        }
        self.server_stop
            .store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(worker) = self.server.take() {
            let _ = worker.join();
        }
        if std::thread::panicking()
            && let Ok(log) = fs::read_to_string(self.paths.instance_dir.join("daemon.log"))
        {
            eprintln!("{log}");
        }
    }
}

#[test]
fn actual_native_snapshot_cancellation_closes_peer_before_elected_worker_join_and_owner_release() {
    use std::sync::atomic::Ordering;
    let mut fixture = ActualNativeFixture::start(true, 0);
    let until = Instant::now() + Duration::from_secs(2);
    while !fixture.snapshot_entered.load(Ordering::SeqCst) {
        assert!(
            Instant::now() < until,
            "actual NativeCli did not open snapshot connection"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let db = fixture.db();
    let instance = fixture.descriptor.instance_uuid.to_string();
    db.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('due',?1,'unresolved','native',1,0)", [&instance]).unwrap();
    db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t',?1,'topic','goal',0,0)", [&instance]).unwrap();
    db.execute_batch("INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t','due','invited'); INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES ('due-inv','t','due',1,'pending',0,1,100,100);").unwrap();
    assert!(matches!(
        herdr_threads::ports::LocalClient::call(
            &fixture.client(),
            Command::Health,
            &fixture.budget()
        )
        .unwrap(),
        CommandResult::Health(_)
    ));
    // The row above is inserted on a separate connection, so it fires no
    // commit hook. Kick the deadline lane as that commit would: waiting for
    // its 5 s safety tick would outlast the held native read's own 5 s budget.
    fixture
        .lanes
        .kick(herdr_threads::service::kicks::Lane::Deadlines);
    loop {
        let warnings: i64 = db
            .query_row("SELECT count(*) FROM messages WHERE kind='warn'", [], |r| {
                r.get(0)
            })
            .unwrap();
        if warnings > 0 {
            break;
        }
        assert!(
            Instant::now() < until,
            "actual native read blocked local deadline work"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!fixture.peer_eof.load(Ordering::SeqCst));
    assert!(OwnerLock::acquire(&fixture.paths).is_err());
    // Hold failure maintenance briefly so peer closure and the still-owned
    // worker can be observed separately, before releasing its final decision.
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert!(matches!(
        herdr_threads::ports::LocalClient::call(
            &fixture.client(),
            Command::Stop(herdr_threads::protocol::commands::StopRequest {
                expected_boot: fixture.descriptor.boot_id.to_string()
            }),
            &fixture.budget()
        )
        .unwrap(),
        CommandResult::StopAccepted(_)
    ));
    let until = Instant::now() + Duration::from_secs(1);
    while !fixture.peer_eof.load(Ordering::SeqCst) {
        assert!(
            Instant::now() < until,
            "cancelled production socket remained open"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!fixture.daemon.as_ref().unwrap().is_finished());
    assert!(
        OwnerLock::acquire(&fixture.paths).is_err(),
        "owner released before joined failure maintenance"
    );
    db.execute_batch("ROLLBACK").unwrap();
    assert!(fixture.daemon.take().unwrap().join().unwrap().unwrap());
    assert!(
        fixture.native.upgrade().is_none(),
        "elected runtime retained NativeCli after join"
    );
    assert!(OwnerLock::acquire(&fixture.paths).is_ok());
    let active: Option<String> = db
        .query_row("SELECT active_snapshot_id FROM host_instances", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(active.is_none());
    eprintln!(
        "actual NativeCli: peer EOF observed after StopAccepted; worker retained elected lease through failure maintenance; daemon joined; adapter dropped; owner reacquired"
    );
}

/// Seventeen saved seats without structural proof and a live pane in the
/// first verified baseline. The actual adapter now publishes, but that
/// capture cannot retire proofless seats, and the unclaimed baseline target
/// stays held for repair instead of being allocated. Kills: publishing a
/// verified snapshot that retires saved seats or releases the recovery hold.
#[cfg(target_os = "macos")]
#[test]
fn actual_native_verified_baseline_holds_targets_and_cannot_retire_proofless_seats() {
    let mut fixture =
        ActualNativeFixture::start_with(false, 17, vec![actual_pane("w4:p1", "terminal")]);
    fixture.wait_for("17 unresolved saved seats", |db| {
        db.query_row(
            "SELECT count(*) FROM seats WHERE state='unresolved'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
            == 17
    });
    let result = fixture.resolve("w4:p1", "actual-held").unwrap_err();
    assert_eq!(result.code, ErrorCode::TargetUnresolved);
    let db = fixture.db();
    let counts: (i64,i64) = db.query_row("SELECT (SELECT count(*) FROM allocation_decisions),(SELECT count(*) FROM seats WHERE state IN ('retiring','retired'))", [], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(counts, (0, 0));
    let baselines: (Option<String>, Option<String>, i64) = db
        .query_row(
            "SELECT active_snapshot_id,recovery_baseline_generation_id,baseline_hold_unclaimed FROM host_instances",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert!(baselines.0.is_some() && baselines.1.is_some());
    assert_eq!(baselines.2, 1);
    fixture.stop.cancel();
    assert!(fixture.daemon.take().unwrap().join().unwrap().unwrap());
    assert!(fixture.native.upgrade().is_none());
    eprintln!(
        "actual NativeCli: verified baseline projected 17 unresolved proofless seats; w4:p1 held (TargetUnresolved); allocation=0 retirement=0; adapter dropped after join"
    );
}

fn host_state(db: &rusqlite::Connection) -> (Option<String>, i64, i64) {
    db.query_row(
        "SELECT active_snapshot_id,host_epoch,invalidation_revision FROM host_instances",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .unwrap()
}

/// Production NativeCli + elected SQLite service against a private
/// protocol-22 endpoint. Kills: (1) adapter reporting Unknown incarnation
/// (no snapshot publishes, resolve is StaleHostObservation), (2) adapter
/// rejecting the store's published boot/epoch fence (resolve always
/// StaleHostObservation), (3) invalidating the lane when Herdr says a
/// mistyped pane does not exist, and (4) retirement never following a
/// coherent same-incarnation absence.
#[cfg(target_os = "macos")]
#[test]
fn actual_native_verified_snapshot_resolves_live_pane_and_retires_it_after_close() {
    let fixture = ActualNativeFixture::start_with(false, 0, vec![actual_pane("w4:p1", "term_a")]);
    fixture.wait_for("first verified publication", |db| {
        host_state(db).0.is_some()
    });
    let CommandResult::SeatResolved(seat) = fixture.resolve("w4:p1", "resolve-live").unwrap()
    else {
        panic!("resolve did not return a seat");
    };
    assert!(
        herdr_threads::protocol::ids::is_short_public_id("seat", seat.as_str()),
        "new seat IDs are short: {}",
        seat.as_str()
    );
    let CommandResult::SeatResolved(again) = fixture.resolve("w4:p1", "resolve-again").unwrap()
    else {
        panic!("repeat resolve did not return a seat");
    };
    assert_eq!(again, seat);
    let db = fixture.db();
    let (_, epoch, invalidations) = host_state(&db);
    let missing = fixture.resolve("w4:p404", "resolve-missing").unwrap_err();
    assert_eq!(missing.code, ErrorCode::NotFound);
    let (_, epoch_after, invalidations_after) = host_state(&db);
    assert_eq!((epoch_after, invalidations_after), (epoch, invalidations));
    let row: (String, String, String) = db
        .query_row(
            "SELECT state,target_id,structural_terminal_id FROM seats WHERE id=?1",
            [seat.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(row, ("resolved".into(), "w4:p1".into(), "term_a".into()));
    fixture.panes.lock().unwrap().clear();
    fixture
        .panes
        .lock()
        .unwrap()
        .push(actual_pane("w4:p2", "term_b"));
    fixture.wait_for("retirement after coherent absence", |db| {
        db.query_row(
            "SELECT state FROM seats WHERE id=?1",
            [seat.as_str()],
            |r| r.get::<_, String>(0),
        )
        .unwrap()
            == "retired"
    });
    eprintln!(
        "actual NativeCli: verified publication -> resolve {seat:?} (repeat same) -> missing pane NotFound without invalidation -> absence retired the seat"
    );
}

/// Kills the review-S3 mutation: restoring the hard-coded Health host fields
/// in the shared elected Health builder `app::elected_health_provider` (the
/// one `app::run_elected` uses) (`host = Unsupported("native current-target authority
/// has not been verified")`, `coherent_enumeration = Unsupported`,
/// `last_reconciliation_at = None`). Health must report the evidence the
/// actual NativeCli observed: a verified coherent publication in the current
/// server incarnation, and the time its reconciliation pass completed.
#[cfg(target_os = "macos")]
#[test]
fn actual_native_health_reports_observed_verified_host_evidence() {
    use herdr_threads::protocol::results::{CapabilityState, ComponentState};
    let started = SystemClock::new().utc_now();
    let fixture = ActualNativeFixture::start_with(false, 0, vec![actual_pane("w4:p1", "term_a")]);
    fixture.wait_for("first verified publication", |db| {
        host_state(db).0.is_some()
    });
    let until = Instant::now() + Duration::from_secs(8);
    let health = loop {
        let CommandResult::Health(health) = herdr_threads::ports::LocalClient::call(
            &fixture.client(),
            Command::Health,
            &fixture.budget(),
        )
        .unwrap() else {
            panic!("missing health");
        };
        if health.last_reconciliation_at.is_some() || Instant::now() >= until {
            break health;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(
        health.host.reachability,
        ComponentState::Ready,
        "{health:?}"
    );
    assert_eq!(
        health.host.coherent_enumeration,
        CapabilityState::Supported,
        "{health:?}"
    );
    let reconciled = health
        .last_reconciliation_at
        .expect("verified publication reconciled without a timestamp");
    assert!(reconciled >= started, "{reconciled:?} < {started:?}");
    assert!(
        !health
            .limitations
            .iter()
            .any(|line| line.starts_with("host ")),
        "{:?}",
        health.limitations
    );
    // Capabilities this lane did not prove stay explicitly unsupported.
    assert_eq!(health.host.current_execution, CapabilityState::Unsupported);
    // The cooperative wake prompt is the adapter's static capability (its
    // own same-incarnation recheck), stated with its cooperative basis as a
    // note: the designed mode, never a limitation.
    assert_eq!(health.host.safe_prompt, CapabilityState::Supported);
    assert!(
        health
            .notes
            .iter()
            .any(|line| line == herdr_threads::daemon::health::COOPERATIVE_WAKE_LINE),
        "{:?}",
        health.notes
    );
    assert!(
        !health
            .limitations
            .iter()
            .any(|line| line.starts_with("wake ")),
        "{:?}",
        health.limitations
    );
    assert_eq!(
        health.host.receipt_registration,
        CapabilityState::Unsupported
    );
    // The harness verdict depends on the claude/codex on this test's PATH;
    // the host side alone never degrades the cooperative mode.
    assert_ne!(health.state, HealthState::Unavailable);
    assert!(health.validate().is_ok(), "{health:?}");
}

/// Kills: (1) no epoch resume at elected startup (a fresh adapter's epoch 1
/// and restarted sequence are "snapshot order is stale" against the
/// previous boot's publication, so nothing publishes after restart), and
/// (2) strict structural-epoch equality for an existing owner (resolve of the
/// same live terminal after restart is TargetUnresolved).
#[cfg(target_os = "macos")]
#[test]
fn actual_native_daemon_restart_keeps_same_terminal_seat_resolvable() {
    let mut fixture =
        ActualNativeFixture::start_with(false, 0, vec![actual_pane("w4:p1", "term_a")]);
    fixture.wait_for("first verified publication", |db| {
        host_state(db).0.is_some()
    });
    let CommandResult::SeatResolved(seat) = fixture.resolve("w4:p1", "before-restart").unwrap()
    else {
        panic!("resolve did not return a seat");
    };
    let (active, epoch, _) = host_state(&fixture.db());
    fixture.restart();
    fixture.wait_for("publication by the restarted boot", |db| {
        let (now_active, now_epoch, _) = host_state(db);
        now_active.is_some() && now_active != active && now_epoch > epoch
    });
    let CommandResult::SeatResolved(after) = fixture.resolve("w4:p1", "after-restart").unwrap()
    else {
        panic!("resolve after restart did not return a seat");
    };
    assert_eq!(after, seat);
    let state: String = fixture
        .db()
        .query_row(
            "SELECT state FROM seats WHERE id=?1",
            [seat.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "resolved");
    eprintln!(
        "actual NativeCli: restart moved host epoch past {epoch}; same live terminal kept seat {seat:?}"
    );
}

/// P35: a client holding the previous boot's descriptor reaches the restarted
/// daemon. The daemon refuses before dispatch; the client reports a definite
/// rejection, never UnknownOutcome, and nothing is recorded.
/// Kills: no `expected_boot` on the wire (the resolve would create a seat) and
/// the client treating the refusal as an identity mismatch (UnknownOutcome).
#[cfg(target_os = "macos")]
#[test]
fn request_with_previous_boot_after_restart_is_daemon_boot_changed() {
    use herdr_threads::protocol::results::ErrorCode;
    let mut fixture =
        ActualNativeFixture::start_with(false, 0, vec![actual_pane("w4:p1", "term_a")]);
    fixture.wait_for("first verified publication", |db| {
        host_state(db).0.is_some()
    });
    let stale = fixture.client();
    let old_boot = fixture.descriptor.boot_id;
    fixture.restart();
    assert_ne!(fixture.descriptor.boot_id, old_boot);
    let resolve = || {
        Command::ResolveSeat(herdr_threads::protocol::commands::ResolveSeat {
            target: HostTargetId::new("w4:p1"),
            operation: herdr_threads::protocol::ids::OperationId::new("stale-boot"),
        })
    };
    let definite = stale
        .call_definitive(resolve(), &fixture.budget())
        .expect("a boot refusal is a correlated answer, not an unknown outcome");
    assert_eq!(definite.unwrap_err().code, ErrorCode::DaemonBootChanged);
    let error =
        herdr_threads::ports::LocalClient::call(&stale, resolve(), &fixture.budget()).unwrap_err();
    assert_eq!(error.code, ErrorCode::DaemonBootChanged);
    let seats: i64 = fixture
        .db()
        .query_row("SELECT count(*) FROM seats", [], |r| r.get(0))
        .unwrap();
    assert_eq!(seats, 0, "the refused request applied nothing");
    // The current descriptor still works.
    assert!(matches!(
        fixture.resolve("w4:p1", "fresh-boot").unwrap(),
        CommandResult::SeatResolved(_)
    ));
}

/// O1/C4: a joined, registered seat stays effectively available across a
/// daemon restart. The restarted boot resumes a higher host epoch of the same
/// Herdr incarnation; the structural carry-forward moves the open binding to
/// it, so the send-time availability projection (the one `stage_recipient`
/// uses) still confirms the seat and the first send does not warn.
/// Kills: no carry-forward on a resolved seat whose binding lags the epoch.
#[cfg(target_os = "macos")]
fn restart_keeps_registered_seat_available(provenance: &str) {
    let mut fixture =
        ActualNativeFixture::start_with(false, 0, vec![actual_pane("w4:p1", "term_a")]);
    fixture.wait_for("first verified publication", |db| {
        host_state(db).0.is_some()
    });
    let CommandResult::SeatResolved(seat) = fixture.resolve("w4:p1", "before-restart").unwrap()
    else {
        panic!("resolve did not return a seat");
    };
    let available = |db: &rusqlite::Connection| {
        herdr_threads::store::schema::effective_registered_availability(db, seat.as_str(), None)
            .unwrap()
    };
    {
        // The seat's agent has checked in at the current epoch.
        let db = fixture.db();
        db.execute(
            "INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) SELECT s.id,s.generation,s.target_generation,s.target_id,h.host_boot,h.host_epoch,'claude','plugin_context:x','exec-x',?2,1,1,s.structural_terminal_id,s.structural_incarnation FROM seats s JOIN host_instances h ON h.id=s.instance_id WHERE s.id=?1",
            rusqlite::params![seat.as_str(), provenance],
        )
        .unwrap();
        assert_eq!(available(&db).as_deref(), Some(provenance));
    }
    let (active, epoch, _) = host_state(&fixture.db());
    fixture.restart();
    fixture.wait_for("publication by the restarted boot", |db| {
        let (now_active, now_epoch, _) = host_state(db);
        now_active.is_some() && now_active != active && now_epoch > epoch
    });
    fixture.wait_for("binding carried to the restarted boot's epoch", |db| {
        available(db).is_some()
    });
    let db = fixture.db();
    assert_eq!(available(&db).as_deref(), Some(provenance));
    let (binding_epoch, host_epoch, ended): (i64, i64, Option<i64>) = db
        .query_row(
            "SELECT b.host_epoch,h.host_epoch,b.ended_at FROM occupant_bindings b JOIN seats s ON s.id=b.seat_id JOIN host_instances h ON h.id=s.instance_id WHERE b.seat_id=?1",
            [seat.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert!(host_epoch > epoch);
    assert_eq!(binding_epoch, host_epoch);
    assert_eq!(ended, None);
}

#[cfg(target_os = "macos")]
#[test]
fn daemon_restart_keeps_joined_cooperative_seat_available_without_warning() {
    restart_keeps_registered_seat_available("cooperative_top_level");
}

#[cfg(target_os = "macos")]
#[test]
fn daemon_restart_keeps_joined_human_seat_available() {
    restart_keeps_registered_seat_available("operator_human");
}

/// S1 (wave-2 fix1): the writer's startup binding-evidence verification is
/// surfaced, not discarded. A legacy store (bindings written before the
/// evidence invariant) holds one binding the seat's own structural proof can
/// backfill and one it cannot. The production elected daemon must report
/// both counts in Health (which `doctor` renders) and write them to its
/// daemon log, and Health must be Degraded while a binding still lacks
/// evidence.
/// Kills: `ElectedHealth::inputs` not reading the binding evidence
/// (no limitation line), `run_elected` not logging the result (no log line),
/// and `HealthInputs::assemble` ignoring `still_lacking` for the state.
/// Also kills (wave-2 fix2 (b)): Health reading the boot-frozen startup
/// count instead of `binding_evidence_current` (the healed seat would keep
/// "still lacking 1"), the store not pruning a healed seat, and
/// `ElectedHealth::inputs` not reading the unresolved-seat summary (no
/// count, no named seat).
#[test]
fn elected_daemon_reports_startup_binding_evidence_counts_in_health_and_log() {
    let _stdio_guard = super::IN_PROCESS_DAEMON
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let root = std::env::temp_dir().join(format!("herdr-evidence-{}", Uuid::new_v4()));
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    struct Remove(std::path::PathBuf);
    impl Drop for Remove {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _remove = Remove(root.clone());
    let context =
        RuntimeContext::explicit(root.join("state"), root.join("host.sock"), None).unwrap();
    let paths = InstancePaths::resolve(&context).unwrap();
    let owner = OwnerLock::acquire(&paths).unwrap();
    let instance = owner.instance_uuid().to_string();
    drop(owner);
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    drop(
        StoreContext::new(paths.database_path.clone(), Arc::clone(&clock))
            .open_writer()
            .unwrap(),
    );
    {
        // A pre-invariant store: raw connection, so no evidence guard.
        let db = rusqlite::Connection::open(&paths.database_path).unwrap();
        db.execute(
            "INSERT INTO host_instances(id,created_at,host_boot,host_epoch) VALUES (?1,0,'b',1)",
            [&instance],
        )
        .unwrap();
        for seat in ["healable", "noproof"] {
            db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES (?1,?2,'resolved','native','p-'||?1,1,1,0)", [seat, instance.as_str()]).unwrap();
            db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at) VALUES (?1,1,1,'p-'||?1,'b',1,'codex','n','e-'||?1,'cooperative_top_level',1,1)", [seat]).unwrap();
        }
        db.execute("UPDATE seats SET structural_terminal_id='term',structural_incarnation='inc',structural_incarnation_kind='coherent_enumeration',structural_host_boot='b',structural_host_epoch=1,structural_connection_epoch=1,structural_observation_sequence=1 WHERE id='healable'", []).unwrap();
        // One unresolved seat for the unresolved-seat channel.
        db.execute("INSERT INTO seats(id,instance_id,state,unresolved_reason,role,target_id,generation,target_generation,created_at) VALUES ('stuck',?1,'unresolved','other','native','p-stuck',2,1,0)", [instance.as_str()]).unwrap();
    }
    let host = Arc::new(herdr_threads::host::native::NativeCli::new(
        root.join("absent-herdr.sock"),
        Arc::clone(&clock),
    ));
    let stop = Cancellation::default();
    let thread_stop = stop.clone();
    let thread_paths = paths.clone();
    let thread_clock = Arc::clone(&clock);
    let (ready, received) = std::sync::mpsc::sync_channel(1);
    let daemon = std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(herdr_threads::app::run_elected(
                &thread_paths,
                thread_clock,
                thread_stop,
                ServiceConfig::default(),
                host,
                move |descriptor| {
                    ready.send(descriptor.clone()).unwrap();
                    Ok(())
                },
            ))
    });
    struct Join {
        stop: Cancellation,
        daemon: Option<std::thread::JoinHandle<std::io::Result<bool>>>,
    }
    impl Drop for Join {
        fn drop(&mut self) {
            self.stop.cancel();
            if let Some(daemon) = self.daemon.take() {
                let _ = daemon.join();
            }
        }
    }
    let mut join = Join {
        stop: stop.clone(),
        daemon: Some(daemon),
    };
    let descriptor = received.recv_timeout(Duration::from_secs(5)).unwrap();
    let client = LocalSocketClient::new(
        descriptor.endpoint.clone(),
        Arc::clone(&clock),
        descriptor.instance_uuid,
        Some(descriptor.boot_id),
    );
    // The observation lane captures at once and, host being absent, freezes:
    // Herdr unavailability writes no host invalidation (TRUST-POLICY C4,
    // ht-yms), so the saved seats stay resolved. Wait for the lane's frozen
    // outcome to reach Health so Health is read after that capture.
    let read_health = || {
        let CommandResult::Health(health) = herdr_threads::ports::LocalClient::call(
            &client,
            Command::Health,
            &CallBudget {
                deadline: MonoInstant(clock.monotonic_now().0 + 2_000),
                cancellation: Cancellation::default(),
            },
        )
        .unwrap() else {
            panic!("health")
        };
        health
    };
    let settle_until = std::time::Instant::now() + Duration::from_secs(10);
    let health = loop {
        let health = read_health();
        if health.limitations.iter().any(|line| {
            line.contains("host unavailable (HostUnavailable): seats and bindings frozen")
        }) {
            break health;
        }
        assert!(
            std::time::Instant::now() < settle_until,
            "the absent host never froze the observation lane: {:?}",
            health.limitations
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    let expected = "store startup binding evidence: backfilled 1, still lacking 1; those seats \
                    cannot be reconfirmed automatically after a host invalidation: repair each \
                    with `seat rebind SEAT --pane PANE --operator`";
    let lacking_now = "binding evidence: backfilled 1 at store startup, still lacking 1 now; \
                       after a host invalidation those seats stay unresolved until their agent \
                       registers again or `seat rebind SEAT --pane PANE --operator`";
    assert!(
        health.limitations.iter().any(|line| line == lacking_now),
        "{:?}",
        health.limitations
    );
    assert_eq!(health.state, HealthState::Degraded);
    // Only `stuck`: the frozen capture marked neither bound seat unresolved.
    assert_eq!(health.unresolved_seats, Some(1));
    {
        let db = rusqlite::Connection::open(&paths.database_path).unwrap();
        db.busy_timeout(Duration::from_secs(2)).unwrap();
        let resolved: i64 = db
            .query_row(
                "SELECT count(*) FROM seats WHERE id IN ('healable','noproof') AND state='resolved'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(resolved, 2, "host unavailability unresolved a bound seat");
    }
    assert!(
        health
            .limitations
            .iter()
            .any(|line| line == "unresolved seat stuck on p-stuck (other)"),
        "{:?}",
        health.limitations
    );
    // Wave-2 fix2 (b) / fix2 review N1: Health is recomputed when the
    // evidence changes, not frozen at boot. The lacking seat's agent
    // re-registers with evidence (its latest binding now carries terminal
    // and incarnation) and `stuck` is resolved again. The lane's repeated
    // unavailability failures write no invalidation, so nothing marks them.
    {
        let db = rusqlite::Connection::open(&paths.database_path).unwrap();
        db.busy_timeout(Duration::from_secs(2)).unwrap();
        db.execute(
            "UPDATE occupant_bindings SET ended_at=2 WHERE seat_id='noproof'",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES ('noproof',2,1,'p-noproof','b',1,'codex','n','e-noproof-2','cooperative_top_level',3,3,'term-n','inc')", []).unwrap();
        db.execute("UPDATE seats SET generation=2 WHERE id='noproof'", [])
            .unwrap();
        db.execute(
            "UPDATE seats SET state='resolved',unresolved_reason=NULL WHERE id='stuck'",
            [],
        )
        .unwrap();
    }
    let CommandResult::Health(health) = herdr_threads::ports::LocalClient::call(
        &client,
        Command::Health,
        &CallBudget {
            deadline: MonoInstant(clock.monotonic_now().0 + 2_000),
            cancellation: Cancellation::default(),
        },
    )
    .unwrap() else {
        panic!("health")
    };
    assert!(
        health
            .limitations
            .iter()
            .any(|line| line
                == "binding evidence: backfilled 1 at store startup, still lacking 0 now"),
        "{:?}",
        health.limitations
    );
    assert!(
        !health
            .limitations
            .iter()
            .any(|line| line.contains("still lacking 1") || line.starts_with("unresolved seats")),
        "{:?}",
        health.limitations
    );
    assert_eq!(health.unresolved_seats, Some(0));
    // Clean stop, then the daemon log (the child-owned diagnostics) holds
    // the same line.
    stop.cancel();
    assert!(join.daemon.take().unwrap().join().unwrap().unwrap());
    let log = fs::read_to_string(paths.instance_dir.join("daemon.log")).unwrap();
    assert!(log.contains(expected), "{log}");
}
