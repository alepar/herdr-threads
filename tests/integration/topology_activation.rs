//! Model-free elected/public composition against an owned real native UDS peer.
use herdr_threads::{
    app::{SystemClock, run_elected, run_elected_guarded},
    client::local::LocalSocketClient,
    daemon::{
        ownership::OwnerLock,
        paths::{InstancePaths, RuntimeContext},
    },
    harness::context::{ContextJournal, Harness, OccupantContext, Role, SessionReference},
    host::native::NativeCli,
    ports::{
        BootstrapObserver, DurableWorkAdmission, HostCallContext, LocalClient, SnapshotHeader,
        StorePort,
    },
    protocol::{
        commands::Command,
        results::CommandResult,
        time::{CallBudget, Cancellation, Clock, MonoInstant},
    },
    service::config::ServiceConfig,
    store::{SqliteStore, StoreSettings, connection::StoreContext},
    test_support::spawn,
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::{fs::DirBuilderExt, net::UnixListener},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use uuid::Uuid;

fn f_copy_database(source: &rusqlite::Connection, target: &std::path::Path) {
    source
        .execute("VACUUM INTO ?1", [target.to_str().unwrap()])
        .unwrap();
}
fn scripted_pane(target: &str, launched: bool) -> Value {
    let terminal = match target {
        "w4:p1" => "term_1",
        "w4:p2" => "term_2",
        "w4:p3" => "term_3",
        _ => panic!("unexpected scripted target {target}"),
    };
    let mut pane = json!({"pane_id":target,"terminal_id":terminal,"workspace_id":"w4","tab_id":if target == "w4:p3" {"w4:t2"} else {"w4:t1"},"focused":false,"agent_status":if launched {"working"} else {"idle"},"revision":1});
    if launched {
        pane["agent"] = json!("codex");
    }
    pane
}

struct PendingHost {
    root: PathBuf,
    stop: Arc<AtomicBool>,
    host: Option<JoinHandle<()>>,
}
impl Drop for PendingHost {
    fn drop(&mut self) {
        if let Some(host) = self.host.take() {
            self.stop.store(true, Ordering::Release);
            let _ = host.join();
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}
struct PendingDaemon {
    stop: Cancellation,
    daemon: Option<JoinHandle<std::io::Result<bool>>>,
}
impl Drop for PendingDaemon {
    fn drop(&mut self) {
        if let Some(daemon) = self.daemon.take() {
            self.stop.cancel();
            let _ = daemon.join();
        }
    }
}
enum HostControl {
    Endpoint(UnixListener, std::sync::mpsc::SyncSender<()>),
    AllowStart(std::sync::mpsc::SyncSender<()>),
    RefusePaneReads(std::sync::mpsc::SyncSender<()>),
}
type HostTrace = Arc<std::sync::Mutex<std::collections::VecDeque<String>>>;
fn trace_host(trace: &HostTrace, event: String) {
    let mut trace = trace.lock().unwrap();
    if trace.len() == 128 {
        trace.pop_front();
    }
    trace.push_back(event);
}

struct Fixture {
    root: PathBuf,
    context: RuntimeContext,
    paths: InstancePaths,
    instance: Uuid,
    client: LocalSocketClient,
    clock: Arc<dyn Clock>,
    pane_reads: Arc<AtomicUsize>,
    effects: Arc<AtomicUsize>,
    creates: Arc<AtomicUsize>,
    starts: Arc<AtomicUsize>,
    host_control: std::sync::mpsc::Sender<HostControl>,
    host_trace: HostTrace,
    host_stop: Arc<AtomicBool>,
    host: Option<JoinHandle<()>>,
    stop: Cancellation,
    daemon: Option<JoinHandle<std::io::Result<bool>>>,
}
impl Fixture {
    fn new() -> Self {
        Self::new_runtime(true)
    }
    fn new_runtime(guarded: bool) -> Self {
        Self::with_refusal(guarded, false)
    }
    fn with_refusal(guarded: bool, refuse_start: bool) -> Self {
        Self::with_creation_reply_loss(guarded, refuse_start, false)
    }
    fn with_creation_reply_loss(
        guarded: bool,
        refuse_start: bool,
        lose_create_reply: bool,
    ) -> Self {
        Self::with_original_harness(guarded, refuse_start, lose_create_reply, Harness::Codex)
    }
    fn with_original_harness(
        guarded: bool,
        refuse_start: bool,
        lose_create_reply: bool,
        original_harness: Harness,
    ) -> Self {
        let root = PathBuf::from("/private/tmp").join(format!("ht-a-{}", Uuid::new_v4()));
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&root)
            .unwrap();
        let bin = root.join("bin");
        fs::create_dir(&bin).unwrap();
        for command in ["codex", "claude"] {
            std::os::unix::fs::symlink("/usr/bin/true", bin.join(command)).unwrap();
        }
        let endpoint = root.join("h.sock");
        let listener = UnixListener::bind(&endpoint).unwrap();
        listener.set_nonblocking(true).unwrap();
        let host_stop = Arc::new(AtomicBool::new(false));
        let pane_reads = Arc::new(AtomicUsize::new(0));
        let effects = Arc::new(AtomicUsize::new(0));
        let creates = Arc::new(AtomicUsize::new(0));
        let starts = Arc::new(AtomicUsize::new(0));
        let (stop_host, reads, native_effects, create_count, start_count) = (
            host_stop.clone(),
            pane_reads.clone(),
            effects.clone(),
            creates.clone(),
            starts.clone(),
        );
        let (host_control, controls) = std::sync::mpsc::channel::<HostControl>();
        let host_trace = HostTrace::default();
        let trace = host_trace.clone();
        let host = std::thread::spawn(move || {
            let mut listener = listener;
            let mut refuse_start = refuse_start;
            let mut refuse_pane_reads = false;
            let mut created = false;
            let mut agent = None;
            while !stop_host.load(Ordering::Acquire) {
                if let Ok(control) = controls.try_recv() {
                    match control {
                        HostControl::Endpoint(replacement, ready) => {
                            listener = replacement;
                            let _ = ready.send(());
                        }
                        HostControl::AllowStart(ready) => {
                            refuse_start = false;
                            let _ = ready.send(());
                        }
                        HostControl::RefusePaneReads(ready) => {
                            refuse_pane_reads = true;
                            let _ = ready.send(());
                        }
                    }
                }
                let (mut stream, _) = match listener.accept() {
                    Ok(v) => v,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(e) => panic!("owned host accept: {e}"),
                };
                // macOS accepted sockets inherit the listener's O_NONBLOCK.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut line = String::new();
                let read = BufReader::new(&mut stream).read_line(&mut line);
                trace_host(&trace, format!("request read: {read:?}"));
                if read.is_err() {
                    continue;
                }
                let request: Value = serde_json::from_str(&line).unwrap();
                trace_host(&trace, format!("method: {}", request["method"]));
                let result = match request["method"].as_str().unwrap() {
                    "ping" => json!({"type":"pong","version":"0.9.1","protocol":22}),
                    "session.snapshot" => {
                        let mut tabs =
                            vec![json!({"tab_id":"w4:t1","workspace_id":"w4","label":"Tab"})];
                        let mut panes =
                            vec![scripted_pane("w4:p1", false), scripted_pane("w4:p2", false)];
                        if created {
                            tabs.push(json!({"tab_id":"w4:t2","workspace_id":"w4","label":"New"}));
                            panes.push(scripted_pane("w4:p3", agent.is_some()));
                        }
                        json!({"type":"session_snapshot","snapshot":{"version":"0.9.1","protocol":22,"agents":agent.iter().collect::<Vec<_>>(),"layouts":[],"workspaces":[{"workspace_id":"w4","label":"Space"}],"tabs":tabs,"panes":panes}})
                    }
                    "pane.get" => {
                        reads.fetch_add(1, Ordering::Relaxed);
                        if refuse_pane_reads {
                            continue;
                        }
                        let target = request["params"]["pane_id"].as_str().unwrap();
                        json!({"type":"pane_info","pane":scripted_pane(target, target == "w4:p3" && agent.is_some())})
                    }
                    "tab.create" => {
                        native_effects.fetch_add(1, Ordering::Relaxed);
                        create_count.fetch_add(1, Ordering::Relaxed);
                        created = true;
                        if lose_create_reply {
                            // The real typed create boundary has submitted once.
                            // EOF loses its correlated answer without undoing the pane.
                            continue;
                        }
                        json!({"type":"tab_created","tab":{"tab_id":"w4:t2","workspace_id":"w4"},"root_pane":scripted_pane("w4:p3", false)})
                    }
                    "agent.start" => {
                        native_effects.fetch_add(1, Ordering::Relaxed);
                        start_count.fetch_add(1, Ordering::Relaxed);
                        if refuse_start {
                            let _ = writeln!(
                                stream,
                                "{}",
                                json!({"id":request["id"],"error":{"code":"agent_pane_busy","message":"owned scripted pre-start refusal"}})
                            );
                            continue;
                        }
                        let mut value = scripted_pane("w4:p3", true);
                        value["name"] = request["params"]["name"].clone();
                        value["interactive_ready"] = json!(true);
                        agent = Some(value.clone());
                        json!({"type":"agent_started","argv":request["params"]["args"],"agent":value})
                    }
                    _ => json!({"type":"agent_info","agent":agent}),
                };
                let wrote = writeln!(stream, "{}", json!({"id":request["id"],"result":result}));
                trace_host(&trace, format!("response write: {wrote:?}"));
            }
        });
        let mut pending_host = PendingHost {
            root: root.clone(),
            stop: host_stop.clone(),
            host: Some(host),
        };
        let context = RuntimeContext::explicit(root.join("state"), endpoint, None).unwrap();
        let paths = InstancePaths::resolve(&context).unwrap();
        let owner = OwnerLock::acquire(&paths).unwrap();
        let instance = owner.instance_uuid();
        drop(owner);
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let native = Arc::new(NativeCli::new(context.host_endpoint.clone(), clock.clone()));
        let budget = CallBudget {
            deadline: MonoInstant(clock.monotonic_now().0 + 5000),
            cancellation: Cancellation::default(),
        };
        let pane = native
            .observe_bootstrap_target(
                &herdr_threads::protocol::ids::HostTargetId::new("w4:p1"),
                &HostCallContext {
                    budget: budget.clone(),
                    expected_boot: None,
                    expected_epoch: None,
                },
            )
            .unwrap();
        let observation = pane.observation();
        let store_context = StoreContext::new(paths.database_path.clone(), clock.clone());
        let db = store_context.open_writer().unwrap();
        db.execute(
            "INSERT INTO host_instances(id,created_at) VALUES (?1,0)",
            [instance.to_string()],
        )
        .unwrap();
        let store = SqliteStore::new(
            store_context,
            instance.to_string(),
            StoreSettings::default(),
        )
        .unwrap();
        let admission = store
            .begin_host_observation(&instance.to_string(), &budget)
            .unwrap();
        let snapshot = herdr_threads::ports::HostPort::enumerate_targets(
            native.as_ref(),
            &HostCallContext {
                budget: budget.clone(),
                expected_boot: None,
                expected_epoch: None,
            },
        )
        .unwrap();
        let stage = store
            .begin_snapshot_stage(
                SnapshotHeader::from_captured(admission, &snapshot).unwrap(),
                &budget,
            )
            .unwrap();
        store
            .stage_snapshot_targets(
                &stage.id,
                0,
                &snapshot.targets,
                DurableWorkAdmission::new(16).unwrap(),
                &budget,
            )
            .unwrap();
        store.seal_snapshot_stage(&stage.id, &budget).unwrap();
        store.publish_snapshot_stage(&stage.id, &budget).unwrap();
        let execution = Uuid::new_v4();
        for (seat, target, terminal) in [
            ("sender", "w4:p1", "term_1"),
            ("recipient", "w4:p2", "term_2"),
        ] {
            db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,structural_terminal_id,structural_incarnation,structural_host_boot,structural_host_epoch,structural_incarnation_kind,structural_connection_epoch,structural_observation_sequence,created_at) VALUES(?1,?2,'resolved','native',?3,1,1,?4,?5,?5,1,'native_current_target',1,1,0)",rusqlite::params![seat,instance.to_string(),target,terminal,observation.host_boot.as_str()]).unwrap();
        }
        db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES('sender',1,'w4:p1',?1,1,1,?3,'session',?2,'cooperative_top_level',0,0,'term_1',?1)",rusqlite::params![observation.host_boot.as_str(),execution.to_string(), original_harness.as_str()]).unwrap();
        drop(db);
        drop(store);
        use sha2::{Digest, Sha256};
        let contexts = paths
            .instance_dir
            .join("contexts")
            .join(format!("{:x}", Sha256::digest(b"sender")));
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&contexts)
            .unwrap();
        ContextJournal::open(&contexts, instance, "sender", Duration::from_secs(1))
            .unwrap()
            .install_reattached(OccupantContext {
                format_version: 1,
                instance,
                seat: "sender".into(),
                target: "w4:p1".into(),
                harness: original_harness,
                binding_generation: 1,
                execution,
                session: SessionReference::Native("session".into()),
                role: Role::TopLevel,
            })
            .unwrap();
        let stop = Cancellation::default();
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let (daemon_paths, daemon_context, daemon_clock, daemon_stop) =
            (paths.clone(), context.clone(), clock.clone(), stop.clone());
        let daemon = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async move {
                    let ready =
                        move |descriptor: &herdr_threads::daemon::ownership::EndpointDescriptor| {
                            ready_tx.send(descriptor.clone()).unwrap();
                            Ok(())
                        };
                    if guarded {
                        run_elected_guarded(
                            &daemon_paths,
                            &daemon_context,
                            daemon_clock,
                            daemon_stop,
                            ServiceConfig::default(),
                            native.clone(),
                            native,
                            ready,
                        )
                        .await
                    } else {
                        run_elected(
                            &daemon_paths,
                            daemon_clock,
                            daemon_stop,
                            ServiceConfig::default(),
                            native,
                            ready,
                        )
                        .await
                    }
                })
        });
        let mut pending_daemon = PendingDaemon {
            stop: stop.clone(),
            daemon: Some(daemon),
        };
        let descriptor = ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let client = LocalSocketClient::new(
            descriptor.endpoint.clone(),
            clock.clone(),
            instance,
            Some(descriptor.boot_id),
        );
        let fixture = Self {
            root,
            context,
            paths,
            instance,
            client,
            clock,
            pane_reads,
            effects,
            creates,
            starts,
            host_control,
            host_trace,
            host_stop,
            host: pending_host.host.take(),
            stop,
            daemon: pending_daemon.daemon.take(),
        };
        let until = Instant::now() + Duration::from_secs(5);
        while fixture.db().query_row("SELECT count(*) FROM host_instances WHERE reconciled_boot IS NOT NULL AND reconciled_boot=recovery_boot AND reconciled_epoch=recovery_epoch", [], |r|r.get::<_,i64>(0)).unwrap() == 0 {
            assert!(Instant::now() < until, "canonical reconciliation");
            std::thread::sleep(Duration::from_millis(10));
        }
        fixture
    }
    fn call(
        &self,
        command: Command,
    ) -> Result<CommandResult, herdr_threads::protocol::results::ApiError> {
        self.client.call(
            command,
            &CallBudget {
                deadline: MonoInstant(self.clock.monotonic_now().0 + 5000),
                cancellation: Cancellation::default(),
            },
        )
    }
    fn cli(&self, args: &[&str], actor: bool) -> std::process::Output {
        self.cli_with_pane(args, actor, None)
    }
    fn cli_with_pane(
        &self,
        args: &[&str],
        actor: bool,
        pane: Option<&str>,
    ) -> std::process::Output {
        let mut cmd = spawn::command(env!("CARGO_BIN_EXE_herdr-threads"));
        if actor {
            cmd.arg("human");
        }
        cmd.arg("--json")
            .arg("--state-dir")
            .arg(&self.context.state_dir)
            .arg("--host-endpoint")
            .arg(&self.context.host_endpoint)
            .args(args)
            .env("HOME", self.root.join("home"))
            .env("CLAUDE_CONFIG_DIR", self.root.join("claude"))
            .env("CODEX_HOME", self.root.join("codex"))
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    self.root.join("bin").display(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            );
        if let Some(pane) = pane {
            cmd.env("HERDR_PANE_ID", pane);
        }
        spawn::tag(&mut cmd).output().unwrap()
    }
    fn rotate_owned_endpoint(&self) {
        use std::os::unix::fs::MetadataExt;
        let old = fs::metadata(&self.context.host_endpoint).unwrap().ino();
        fs::rename(&self.context.host_endpoint, self.root.join("previous.sock")).unwrap();
        let listener = UnixListener::bind(&self.context.host_endpoint).unwrap();
        listener.set_nonblocking(true).unwrap();
        assert_ne!(
            fs::metadata(&self.context.host_endpoint).unwrap().ino(),
            old
        );
        let (ready, acknowledged) = std::sync::mpsc::sync_channel(0);
        self.host_control
            .send(HostControl::Endpoint(listener, ready))
            .unwrap();
        acknowledged.recv_timeout(Duration::from_secs(3)).unwrap();
    }
    fn allow_owned_start(&self) {
        let (ready, acknowledged) = std::sync::mpsc::sync_channel(0);
        self.host_control
            .send(HostControl::AllowStart(ready))
            .unwrap();
        acknowledged.recv_timeout(Duration::from_secs(3)).unwrap();
    }
    fn refuse_owned_pane_reads(&self) {
        let (ready, acknowledged) = std::sync::mpsc::sync_channel(0);
        self.host_control
            .send(HostControl::RefusePaneReads(ready))
            .unwrap();
        acknowledged.recv_timeout(Duration::from_secs(3)).unwrap();
    }
    fn unconfigured_domain(&self) -> herdr_threads::service::dispatch::DomainService {
        let store = Arc::new(
            SqliteStore::new(
                StoreContext::new(self.paths.database_path.clone(), self.clock.clone()),
                self.instance.to_string(),
                StoreSettings::default(),
            )
            .unwrap(),
        );
        let host = Arc::new(NativeCli::new(
            self.context.host_endpoint.clone(),
            self.clock.clone(),
        ));
        let writer = Arc::new(herdr_threads::service::fair_writer::FairWriter::new(32));
        let identity = Arc::new(herdr_threads::identity::repair::OrdinaryIdentity::new(
            self.instance.to_string(),
            store.clone(),
            host,
            self.clock.clone(),
            writer.clone(),
        ));
        herdr_threads::service::dispatch::DomainService::with_identity(
            self.instance.to_string(),
            store,
            self.clock.clone(),
            identity,
        )
        .with_operator_owner(unsafe { libc::geteuid() })
        .with_cooperative_owner(unsafe { libc::geteuid() }, writer)
    }
    fn domain_in_namespace(
        &self,
        namespace: herdr_threads::protocol::handoff::HandoffNamespace,
    ) -> herdr_threads::service::dispatch::DomainService {
        let copy_context = RuntimeContext::explicit(
            namespace.state_dir.clone(),
            namespace.host_endpoint.clone(),
            None,
        )
        .unwrap();
        let copy_paths = InstancePaths::resolve(&copy_context).unwrap();
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(copy_paths.database_path.parent().unwrap())
            .unwrap();
        let copy = copy_paths
            .database_path
            .with_file_name(format!("copy-{}.db", Uuid::new_v4()));
        f_copy_database(&self.db(), &copy);
        let store = Arc::new(
            SqliteStore::new(
                StoreContext::new(copy, self.clock.clone()),
                self.instance.to_string(),
                StoreSettings::default(),
            )
            .unwrap(),
        );
        let host = Arc::new(NativeCli::new(
            namespace.host_endpoint.clone(),
            self.clock.clone(),
        ));
        let writer = Arc::new(herdr_threads::service::fair_writer::FairWriter::new(32));
        let identity = Arc::new(herdr_threads::identity::repair::OrdinaryIdentity::new(
            self.instance.to_string(),
            store.clone(),
            host.clone(),
            self.clock.clone(),
            writer.clone(),
        ));
        herdr_threads::service::dispatch::DomainService::with_identity(
            self.instance.to_string(),
            store.clone(),
            self.clock.clone(),
            identity,
        )
        .with_operator_owner(unsafe { libc::geteuid() })
        .with_cooperative_owner(unsafe { libc::geteuid() }, writer)
        .with_bootstrap_runtime(namespace, host, store)
        .unwrap()
    }
    fn db(&self) -> rusqlite::Connection {
        let db = rusqlite::Connection::open(&self.paths.database_path).unwrap();
        db.busy_timeout(Duration::from_secs(5)).unwrap();
        db
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(daemon) = self.daemon.take() {
            let _ = daemon.join();
        }
        self.host_stop.store(true, Ordering::Release);
        if let Some(host) = self.host.take() {
            let _ = host.join();
        }
        if std::thread::panicking()
            && let Ok(bytes) = fs::read(herdr_threads::daemon::logs::daemon_log_path(&self.paths))
        {
            let tail = &bytes[bytes.len().saturating_sub(16 * 1024)..];
            eprintln!(
                "owned public-chain failure: {}",
                String::from_utf8_lossy(tail)
            );
        }
        if std::thread::panicking() {
            eprintln!("owned host trace: {:?}", self.host_trace.lock().unwrap());
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}
// Exercise retained registered histories through actual equipped, copied and
// unconfigured consumers; no local receipt or authority is synthesized.
fn registered_begin_create_replay_matrix(
    f: &Fixture,
    identity: &herdr_threads::protocol::handoff::BootstrapIdentity,
    attachment: &herdr_threads::protocol::handoff::BootstrapAttachment,
    completed: bool,
) {
    use herdr_threads::{
        ports::{CooperativePermitRequest, LocalService},
        protocol::{
            authority::ObligationRef,
            commands::{CreateThread, PermitMutation},
            handoff::{HandoffChannel, HandoffMutation},
        },
    };
    let HandoffChannel::New { name, topic, goal } = &identity.payload.handoff.channel else {
        panic!("owned public fixture must use frozen new channel");
    };
    let create = CreateThread {
        name: name.clone(),
        topic: topic.clone(),
        goal: goal.clone(),
        operation: identity.payload.handoff.keys.create.clone(),
        claim: identity.claim.clone(),
    };
    let begin = HandoffMutation {
        identity: attachment.handoff.clone(),
        operation: identity.payload.handoff.keys.begin.clone(),
    };
    let commands = [
        Command::CreateThread(create.clone()),
        Command::BeginHandoff(begin.clone()),
    ];
    let positive: Vec<_> = commands
        .iter()
        .map(|c| f.call(c.clone()).unwrap())
        .collect();
    let raw = || {
        f.db()
            .prepare(
                "SELECT operation_key,digest,result_json FROM operations ORDER BY operation_key",
            )
            .unwrap()
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Vec<u8>>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    let canonical = || {
        [
            "SELECT identity_json FROM bootstrap_handoffs",
            "SELECT attachment_json FROM bootstrap_attachments",
            "SELECT completed_json FROM bootstrap_reports",
        ]
        .into_iter()
        .map(|sql| {
            f.db()
                .prepare(sql)
                .unwrap()
                .query_map([], |r| r.get::<_, Vec<u8>>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        })
        .collect::<Vec<_>>()
    };
    let counts = || {
        ["threads", "messages", "invitations"].map(|table| {
            f.db()
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| {
                    r.get::<_, i64>(0)
                })
                .unwrap()
        })
    };
    let saved = raw();
    let saved_canonical = canonical();
    let saved_counts = counts();
    let native = (
        f.creates.load(Ordering::Relaxed),
        f.starts.load(Ordering::Relaxed),
        f.effects.load(Ordering::Relaxed),
    );
    let mut accepted = Vec::new();
    let mut wrong_root = identity.payload.handoff.namespace.clone();
    wrong_root.state_dir = f.root.join("r1-copy-root");
    let mut wrong_endpoint = identity.payload.handoff.namespace.clone();
    wrong_endpoint.host_endpoint = f.root.join("r1-copy.sock");
    let mut spelling = identity.payload.handoff.namespace.clone();
    spelling.state_dir = PathBuf::from(format!("{}//state", f.root.display()));
    let mut endpoint_spelling = identity.payload.handoff.namespace.clone();
    endpoint_spelling.host_endpoint = PathBuf::from(format!("{}//h.sock", f.root.display()));
    for (label, namespace) in [
        ("root", wrong_root),
        ("endpoint", wrong_endpoint),
        ("root-spelling", spelling),
        ("endpoint-spelling", endpoint_spelling),
    ] {
        let foreign = f.domain_in_namespace(namespace);
        for (index, command) in commands.iter().enumerate() {
            let result = foreign.handle(
                command.clone(),
                herdr_threads::test_support::peer_identity(unsafe { libc::geteuid() }),
                &CallBudget {
                    deadline: MonoInstant(f.clock.monotonic_now().0 + 5000),
                    cancellation: Cancellation::default(),
                },
            );
            if result.is_ok() {
                accepted.push(format!(
                    "{label}/{}={result:?}",
                    if index == 0 { "Create" } else { "Begin" }
                ));
            }
        }
    }
    let generic = f.unconfigured_domain();
    let store = SqliteStore::new(
        StoreContext::new(f.paths.database_path.clone(), f.clock.clone()),
        f.instance.to_string(),
        StoreSettings::default(),
    )
    .unwrap();
    let budget = CallBudget {
        deadline: MonoInstant(f.clock.monotonic_now().0 + 5000),
        cancellation: Cancellation::default(),
    };
    for (index, command) in commands.iter().enumerate() {
        let result = generic.handle(
            command.clone(),
            herdr_threads::test_support::peer_identity(unsafe { libc::geteuid() }),
            &budget,
        );
        if result.is_ok() {
            accepted.push(format!(
                "generic/{}={result:?}",
                if index == 0 { "Create" } else { "Begin" }
            ));
        }
        let (mutation, digest) = if index == 0 {
            (
                PermitMutation::CreateThread(create.clone()),
                herdr_threads::store::control::cooperative_payload_hash("create_thread", &create)
                    .unwrap(),
            )
        } else {
            (
                PermitMutation::BeginHandoff(begin.clone()),
                herdr_threads::store::control::cooperative_payload_hash("begin_handoff", &begin)
                    .unwrap(),
            )
        };
        let permit = store
            .issue_cooperative_permit(
                CooperativePermitRequest {
                    claim: identity.claim.clone(),
                    operation: if index == 0 {
                        create.operation.clone()
                    } else {
                        begin.operation.clone()
                    },
                    obligation: ObligationRef::CheckIn(identity.claim.seat.clone()),
                    payload_hash: digest,
                    check_in_mode: None,
                },
                &budget,
            )
            .unwrap();
        let result = store.mutate(mutation, permit, &budget);
        if result.is_ok() {
            accepted.push(format!(
                "direct/{}={result:?}",
                if index == 0 { "Create" } else { "Begin" }
            ));
        }
    }
    // Pure cached reads must leave every original operation/result and frozen
    // canonical record intact even when an incorrect consumer returned it.
    assert_eq!(raw(), saved);
    assert_eq!(canonical(), saved_canonical);
    assert_eq!(counts(), saved_counts);
    assert_eq!(
        (
            f.creates.load(Ordering::Relaxed),
            f.starts.load(Ordering::Relaxed),
            f.effects.load(Ordering::Relaxed)
        ),
        native
    );
    assert!(
        accepted.is_empty(),
        "R1 registered history needs exact selected context (completed={completed}): {accepted:#?}"
    );

    let mut substitution = create.clone();
    substitution.topic.push_str(" substituted");
    assert!(f.call(Command::CreateThread(substitution)).is_err());
    let mut substitution = begin.clone();
    substitution.identity.recipient = herdr_threads::protocol::ids::SeatId::new("sender");
    assert!(f.call(Command::BeginHandoff(substitution)).is_err());
    let mut phase = create.clone();
    phase.operation = identity.payload.handoff.keys.invite.clone();
    assert!(f.call(Command::CreateThread(phase)).is_err());
    let mut phase = begin.clone();
    phase.operation = herdr_threads::protocol::ids::OperationId::new("owned-r1-unregistered-begin");
    assert!(
        f.call(Command::BeginHandoff(phase)).is_err(),
        "canonical attached child requires its exact frozen Begin key"
    );
    for seat in [&attachment.resolved_seat, &identity.claim.seat] {
        f.db().execute("INSERT INTO recovery_holds(instance_id,target_id,baseline_boot,baseline_epoch,reason) SELECT instance_id,target_id,structural_host_boot,structural_host_epoch,'owned R1 guard' FROM seats WHERE id=?1",[seat.as_str()]).unwrap();
        for (command, expected) in commands.iter().zip(&positive) {
            let result = f.call(command.clone());
            if completed {
                assert_eq!(result.unwrap(), *expected);
            } else {
                assert!(
                    result.is_err(),
                    "live registered cached phase must check held seat: {result:?}"
                );
            }
        }
        f.db()
            .execute(
                "DELETE FROM recovery_holds WHERE reason='owned R1 guard'",
                [],
            )
            .unwrap();
    }
    let thread = positive[1].clone();
    let CommandResult::Handoff(child) = thread else {
        panic!("actual Begin result");
    };
    let thread = child.thread.unwrap();
    f.db()
        .execute(
            "UPDATE threads SET archived=1 WHERE id=?1",
            [thread.as_str()],
        )
        .unwrap();
    for (command, expected) in commands.iter().zip(&positive) {
        let result = f.call(command.clone());
        if completed {
            assert_eq!(result.unwrap(), *expected);
        } else {
            assert!(
                result.is_err(),
                "live cached phase must check archived channel: {result:?}"
            );
        }
    }
    f.db()
        .execute(
            "UPDATE threads SET archived=0 WHERE id=?1",
            [thread.as_str()],
        )
        .unwrap();
    for (command, expected) in commands.iter().zip(&positive) {
        assert_eq!(f.call(command.clone()).unwrap(), *expected);
    }
    if completed {
        f.db()
            .execute(
                "UPDATE occupant_bindings SET ended_at=1 WHERE seat_id='sender'",
                [],
            )
            .unwrap();
        for (command, expected) in commands.iter().zip(&positive) {
            assert_eq!(
                f.call(command.clone()).unwrap(),
                *expected,
                "exact completed registered history bypasses current binding only"
            );
        }
    }
    assert_eq!(raw(), saved);
    assert_eq!(canonical(), saved_canonical);
    assert_eq!(counts(), saved_counts);
}

#[test]
fn elected_public_delivery_is_staged_without_native_effect_and_completed_retry_is_historical() {
    let _serial = crate::ONE_DAEMON.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let CommandResult::Capabilities(caps) = f.call(Command::Capabilities).unwrap() else {
        panic!("capabilities")
    };
    assert!(
        caps.capabilities
            .iter()
            .any(|v| v == herdr_threads::protocol::capabilities::BOOTSTRAP_GUARDED_RESOLUTION_V1)
    );
    let until = Instant::now() + Duration::from_secs(5);
    while f.db().query_row("SELECT count(*) FROM host_instances WHERE reconciled_boot IS NOT NULL AND reconciled_boot=recovery_boot AND reconciled_epoch=recovery_epoch", [], |r|r.get::<_,i64>(0)).unwrap()==0 {assert!(Instant::now()<until,"reconciliation"); std::thread::sleep(Duration::from_millis(10));}
    let out = f.cli(
        &[
            "--cooperative-seat",
            "sender",
            "--cooperative-target",
            "w4:p1",
            "--cooperative-harness",
            "codex",
            "--cooperative-role",
            "top-level",
            "handoff",
            "--existing",
            "--seat",
            "recipient",
            "--new-thread",
            "--",
            "literal work",
        ],
        false,
    );
    assert!(
        out.status.success(),
        "delivery: {} {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let frame: Value = serde_json::from_slice(&out.stdout).unwrap();
    let report = &frame["delivery"];
    assert_eq!(report["outcome"], "staged");
    assert_eq!(report["participation"], "staged_unbound");
    let reference = report["recovery_ref"].as_str().unwrap();
    assert_eq!(f.effects.load(Ordering::Relaxed), 0);
    f.refuse_owned_pane_reads();
    let db = f.db();
    db.execute_batch("UPDATE threads SET archived=1; UPDATE occupant_bindings SET ended_at=1; UPDATE seats SET generation=generation+1").unwrap();
    drop(db);
    let retry = f.cli(&["retry", reference], false);
    assert!(
        retry.status.success(),
        "historical retry: {}",
        String::from_utf8_lossy(&retry.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&retry.stdout).unwrap(),
        frame
    );

    // Human argv does not rewrite the immutable Agent origin; historical output remains honest.
    let human = f.cli(&["retry", reference], true);
    assert!(
        human.status.success(),
        "{}",
        String::from_utf8_lossy(&human.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&human.stdout).unwrap(),
        frame
    );
    let refusal = f.cli(
        &[
            "--cooperative-seat",
            "sender",
            "--cooperative-target",
            "w4:p1",
            "--cooperative-harness",
            "codex",
            "--cooperative-role",
            "top-level",
            "handoff",
            "--existing",
            "--seat",
            "recipient",
            "--new-thread",
            "--",
            "forbidden origin",
        ],
        true,
    );
    assert!(!refusal.status.success());
    assert!(refusal.stdout.is_empty());
    assert_eq!(f.effects.load(Ordering::Relaxed), 0);
    assert!(f.instance != Uuid::nil());
}

#[test]
fn elected_public_new_tab_completes_one_native_create_and_start_then_replays_history() {
    let _serial = crate::ONE_DAEMON.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let setup = f.cli(
        &["setup", "codex", "--harness-binary", "/usr/bin/true"],
        true,
    );
    assert!(
        setup.status.success(),
        "owned setup: {}",
        String::from_utf8_lossy(&setup.stderr)
    );
    let out = f.cli(
        &[
            "--cooperative-seat",
            "sender",
            "--cooperative-target",
            "w4:p1",
            "--cooperative-harness",
            "codex",
            "--cooperative-role",
            "top-level",
            "handoff",
            "--new-tab",
            "Peer",
            "--new-thread",
            "--kind",
            "codex",
            "--harness-binary",
            "/usr/bin/true",
            "--agent-arg=--model",
            "--agent-arg=test model",
            "--",
            "one frozen native work item",
        ],
        false,
    );
    if !out.status.success() {
        let db = f.db();
        let identity: Result<Vec<u8>, _> =
            db.query_row("SELECT identity_json FROM bootstrap_handoffs", [], |r| {
                r.get(0)
            });
        eprintln!(
            "actual canonical identity: {:?}; public stderr: {}",
            identity.map(|v| String::from_utf8_lossy(&v).into_owned()),
            String::from_utf8_lossy(&out.stderr)
        );
        let mut statement = db.prepare("SELECT actor_scope,operation_key,hex(digest),result_json FROM operations ORDER BY rowid").unwrap();
        for row in statement
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .unwrap()
        {
            eprintln!("actual retained operation: {:?}", row.unwrap());
        }
        eprintln!(
            "native creates={} starts={}",
            f.creates.load(Ordering::Relaxed),
            f.starts.load(Ordering::Relaxed)
        );
    }
    assert!(
        out.status.success(),
        "new-tab: {} {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let frame: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(f.creates.load(Ordering::Relaxed), 1);
    assert_eq!(f.starts.load(Ordering::Relaxed), 1);
    let terminal = f
        .db()
        .query_row("SELECT state FROM bootstrap_handoffs", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap();
    assert_eq!(terminal, "completed");
    let bytes = f
        .db()
        .query_row("SELECT identity_json FROM bootstrap_handoffs", [], |r| {
            r.get::<_, Vec<u8>>(0)
        })
        .unwrap();
    let identity: herdr_threads::protocol::handoff::BootstrapIdentity =
        serde_json::from_slice(&bytes).unwrap();
    assert_eq!(identity.payload.handoff.body, "one frozen native work item");
    assert_eq!(identity.payload.launch.argv, ["--model", "test model"]);
    assert_eq!(frame["handoff"]["outcome"], "started");
    let completed: herdr_threads::protocol::handoff::CompletedBootstrapResult =
        serde_json::from_slice(
            &f.db()
                .query_row("SELECT completed_json FROM bootstrap_reports", [], |r| {
                    r.get::<_, Vec<u8>>(0)
                })
                .unwrap(),
        )
        .unwrap();
    registered_begin_create_replay_matrix(&f, &identity, &completed.attachment, true);
    let legacy_begin = herdr_threads::protocol::handoff::HandoffMutation {
        identity: completed.attachment.handoff.clone(),
        operation: identity.payload.handoff.keys.begin.clone(),
    };
    let old_begin: (Vec<u8>, String) = f.db().query_row("SELECT digest,result_json FROM operations WHERE actor_scope='seat:sender' AND operation_key=?1", [identity.payload.handoff.keys.begin.as_str()], |r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(
        old_begin.0,
        herdr_threads::store::control::cooperative_payload_hash("begin_handoff", &legacy_begin)
            .unwrap()
    );
    let invite = Command::Invite(herdr_threads::protocol::commands::Invite {
        thread: completed.retained.thread.clone(),
        seat: completed.retained.recipient.clone(),
        deadline_millis: None,
        operation: identity.payload.handoff.keys.invite.clone(),
        claim: identity.claim.clone(),
    });
    let send = Command::SendMessage(herdr_threads::protocol::commands::SendMessage {
        delivery_mode: herdr_threads::protocol::commands::DeliveryMode::Ordinary,
        thread: completed.retained.thread.clone(),
        body: identity.payload.handoff.body.clone(),
        invited_recipients: vec![completed.retained.recipient.clone()],
        deadline_millis: None,
        operation: identity.payload.handoff.keys.send.clone(),
        claim: identity.claim.clone(),
        relays_user: false,
        user_intent: None,
    });
    let current_invite = f.call(invite.clone()).unwrap();
    let current_send = f.call(send.clone()).unwrap();
    use herdr_threads::ports::LocalService;
    let mut wrong_root = identity.payload.handoff.namespace.clone();
    wrong_root.state_dir = f.root.join("copied-selected-root");
    let mut wrong_endpoint = identity.payload.handoff.namespace.clone();
    wrong_endpoint.host_endpoint = f.root.join("copied.sock");
    let mut spelling = identity.payload.handoff.namespace.clone();
    spelling.state_dir = PathBuf::from(format!("{}//state", f.root.display()));
    let mut endpoint_spelling = identity.payload.handoff.namespace.clone();
    endpoint_spelling.host_endpoint = PathBuf::from(format!("{}//h.sock", f.root.display()));
    for wrong in [wrong_root, wrong_endpoint, spelling, endpoint_spelling] {
        let foreign = f.domain_in_namespace(wrong);
        let budget = CallBudget {
            deadline: MonoInstant(f.clock.monotonic_now().0 + 5000),
            cancellation: Cancellation::default(),
        };
        let foreign_invite = foreign.handle(
            invite.clone(),
            herdr_threads::test_support::peer_identity(unsafe { libc::geteuid() }),
            &budget,
        );
        let foreign_send = foreign.handle(
            send.clone(),
            herdr_threads::test_support::peer_identity(unsafe { libc::geteuid() }),
            &budget,
        );
        assert!(
            foreign_invite.is_err() && foreign_send.is_err(),
            "copied selected namespace must refuse both cached phases: invite={foreign_invite:?} send={foreign_send:?}; canonical={current_invite:?} {current_send:?}"
        );
    }
    let generic = f.unconfigured_domain();
    let budget = CallBudget {
        deadline: MonoInstant(f.clock.monotonic_now().0 + 5000),
        cancellation: Cancellation::default(),
    };
    let generic_invite = generic.handle(
        invite.clone(),
        herdr_threads::test_support::peer_identity(unsafe { libc::geteuid() }),
        &budget,
    );
    let generic_send = generic.handle(
        send.clone(),
        herdr_threads::test_support::peer_identity(unsafe { libc::geteuid() }),
        &budget,
    );
    let store = SqliteStore::new(
        StoreContext::new(f.paths.database_path.clone(), f.clock.clone()),
        f.instance.to_string(),
        StoreSettings::default(),
    )
    .unwrap();
    let Command::Invite(direct_invite) = invite.clone() else {
        unreachable!()
    };
    let old_hash =
        herdr_threads::store::control::cooperative_payload_hash("invite", &direct_invite).unwrap();
    let permit = store
        .issue_cooperative_permit(
            herdr_threads::ports::CooperativePermitRequest {
                claim: identity.claim.clone(),
                operation: direct_invite.operation.clone(),
                obligation: herdr_threads::protocol::authority::ObligationRef::Control(
                    direct_invite.thread.clone(),
                ),
                payload_hash: old_hash,
                check_in_mode: None,
            },
            &budget,
        )
        .unwrap();
    let direct = store.mutate(
        herdr_threads::protocol::commands::PermitMutation::Invite(direct_invite),
        permit,
        &budget,
    );
    let Command::SendMessage(direct_send) = send.clone() else {
        unreachable!()
    };
    let prepare = store.prepare_send_step(
        &direct_send,
        DurableWorkAdmission::new(16).unwrap(),
        &budget,
    );
    assert!(
        generic_invite.is_err() && generic_send.is_err() && direct.is_err() && prepare.is_err(),
        "registered phases require selected namespace even without a configured adapter: generic={generic_invite:?} {generic_send:?}; direct={direct:?}; prepare={prepare:?}"
    );
    let parent_begin =
        Command::BeginBootstrap(Box::new(herdr_threads::protocol::handoff::BeginBootstrap {
            identity: identity.clone(),
            operation: identity.payload.handoff.keys.begin.clone(),
        }));
    f.db()
        .execute(
            "UPDATE occupant_bindings SET ended_at=1 WHERE seat_id='sender'",
            [],
        )
        .unwrap();
    assert!(
        matches!(f.call(parent_begin).unwrap(), CommandResult::Bootstrap(ref v) if v.state == herdr_threads::protocol::handoff::BootstrapState::Completed)
    );

    let recovery = frame["handoff"]["recovery_ref"].as_str().unwrap();
    let reads = f.pane_reads.load(Ordering::Relaxed);
    let retry = f.cli(&["retry", recovery], false);
    assert!(
        retry.status.success(),
        "historical bootstrap: {}",
        String::from_utf8_lossy(&retry.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&retry.stdout).unwrap(),
        frame
    );
    assert_eq!(f.pane_reads.load(Ordering::Relaxed), reads);
    assert_eq!(f.creates.load(Ordering::Relaxed), 1);
    assert_eq!(f.starts.load(Ordering::Relaxed), 1);
    let preserved: (Vec<u8>, String) = f.db().query_row("SELECT digest,result_json FROM operations WHERE actor_scope='seat:sender' AND operation_key=?1", [identity.payload.handoff.keys.begin.as_str()], |r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(preserved, old_begin);
}

#[test]
fn elected_unconfigured_daemon_refuses_new_mode_before_semantic_publication_or_effects() {
    let _serial = crate::ONE_DAEMON.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new_runtime(false);
    let CommandResult::Capabilities(caps) = f.call(Command::Capabilities).unwrap() else {
        panic!("capabilities")
    };
    assert!(
        !caps
            .capabilities
            .iter()
            .any(|v| v == herdr_threads::protocol::capabilities::BOOTSTRAP_GUARDED_RESOLUTION_V1)
    );
    let reads = f.pane_reads.load(Ordering::Relaxed);
    let refusal = f.cli(
        &[
            "--cooperative-seat",
            "sender",
            "--cooperative-target",
            "w4:p1",
            "--cooperative-harness",
            "codex",
            "--cooperative-role",
            "top-level",
            "handoff",
            "--existing",
            "--seat",
            "recipient",
            "--new-thread",
            "--",
            "refused before publication",
        ],
        false,
    );
    assert!(!refusal.status.success());
    assert!(
        String::from_utf8_lossy(&refusal.stderr)
            .contains("daemon lacks guarded handoff capability"),
        "{}",
        String::from_utf8_lossy(&refusal.stderr)
    );
    assert!(refusal.stdout.is_empty());
    assert_eq!(f.pane_reads.load(Ordering::Relaxed), reads);
    assert_eq!(f.effects.load(Ordering::Relaxed), 0);
    assert_eq!(
        f.db()
            .query_row("SELECT count(*) FROM operations", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        f.db()
            .query_row("SELECT count(*) FROM bootstrap_handoffs", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        f.db()
            .query_row("SELECT count(*) FROM channel_handoff_fences", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let directory = f.paths.instance_dir.join("intents");
    assert!(fs::read_dir(directory).unwrap().all(|e| {
        !e.unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".intent")
    }));
}

#[test]
fn elected_live_registered_phase_replays_recheck_current_caller_recipient_and_payload() {
    let _serial = crate::ONE_DAEMON.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::with_refusal(true, true);
    let setup = f.cli(
        &["setup", "codex", "--harness-binary", "/usr/bin/true"],
        true,
    );
    assert!(
        setup.status.success(),
        "{}",
        String::from_utf8_lossy(&setup.stderr)
    );
    let out = f.cli(
        &[
            "--cooperative-seat",
            "sender",
            "--cooperative-target",
            "w4:p1",
            "--cooperative-harness",
            "codex",
            "--cooperative-role",
            "top-level",
            "handoff",
            "--new-tab",
            "Peer",
            "--new-thread",
            "--kind",
            "codex",
            "--harness-binary",
            "/usr/bin/true",
            "--",
            "live retained work",
        ],
        false,
    );
    assert!(!out.status.success());
    assert_eq!(
        f.db()
            .query_row("SELECT state FROM bootstrap_handoffs", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "attached",
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let identity: herdr_threads::protocol::handoff::BootstrapIdentity = serde_json::from_slice(
        &f.db()
            .query_row("SELECT identity_json FROM bootstrap_handoffs", [], |r| {
                r.get::<_, Vec<u8>>(0)
            })
            .unwrap(),
    )
    .unwrap();
    let attachment: herdr_threads::protocol::handoff::BootstrapAttachment = serde_json::from_slice(
        &f.db()
            .query_row(
                "SELECT attachment_json FROM bootstrap_attachments",
                [],
                |r| r.get::<_, Vec<u8>>(0),
            )
            .unwrap(),
    )
    .unwrap();
    let thread: herdr_threads::protocol::ids::ThreadId =
        herdr_threads::protocol::ids::ThreadId::new(
            f.db()
                .query_row("SELECT thread_id FROM bootstrap_handoffs", [], |r| {
                    r.get::<_, String>(0)
                })
                .unwrap(),
        );
    let invite = Command::Invite(herdr_threads::protocol::commands::Invite {
        thread: thread.clone(),
        seat: attachment.resolved_seat.clone(),
        deadline_millis: None,
        operation: identity.payload.handoff.keys.invite.clone(),
        claim: identity.claim.clone(),
    });
    let send = Command::SendMessage(herdr_threads::protocol::commands::SendMessage {
        delivery_mode: herdr_threads::protocol::commands::DeliveryMode::Ordinary,
        thread,
        body: identity.payload.handoff.body.clone(),
        invited_recipients: vec![attachment.resolved_seat.clone()],
        deadline_millis: None,
        operation: identity.payload.handoff.keys.send.clone(),
        claim: identity.claim.clone(),
        relays_user: false,
        user_intent: None,
    });
    registered_begin_create_replay_matrix(&f, &identity, &attachment, false);
    let original_invite = f.call(invite.clone()).unwrap();
    let original_send = f.call(send.clone()).unwrap();
    let raw = |key: &str| {
        f.db().query_row("SELECT digest,result_json FROM operations WHERE actor_scope='seat:sender' AND operation_key=?1", [key], |r|Ok((r.get::<_,Vec<u8>>(0)?,r.get::<_,String>(1)?))).unwrap()
    };
    let before_invite = raw(identity.payload.handoff.keys.invite.as_str());
    let before_send = raw(identity.payload.handoff.keys.send.as_str());
    let mut payload = send.clone();
    let Command::SendMessage(ref mut changed) = payload else {
        unreachable!()
    };
    changed.body = "substituted frozen work".into();
    assert!(f.call(payload).is_err());
    let mut phase = send.clone();
    let Command::SendMessage(ref mut changed) = phase else {
        unreachable!()
    };
    changed.operation = identity.payload.handoff.keys.invite.clone();
    assert!(f.call(phase).is_err());
    f.db().execute("INSERT INTO recovery_holds(instance_id,target_id,baseline_boot,baseline_epoch,reason) SELECT instance_id,target_id,structural_host_boot,structural_host_epoch,'owned live guard' FROM seats WHERE id=?1", [attachment.resolved_seat.as_str()]).unwrap();
    assert!(f.call(invite.clone()).is_err());
    assert!(f.call(send.clone()).is_err());
    f.db()
        .execute(
            "DELETE FROM recovery_holds WHERE reason='owned live guard'",
            [],
        )
        .unwrap();
    assert_eq!(f.call(invite.clone()).unwrap(), original_invite);
    assert_eq!(f.call(send.clone()).unwrap(), original_send);
    f.db()
        .execute(
            "UPDATE occupant_bindings SET ended_at=1 WHERE seat_id='sender'",
            [],
        )
        .unwrap();
    assert!(f.call(invite).is_err());
    assert!(f.call(send).is_err());
    let herdr_threads::protocol::handoff::HandoffChannel::New { name, topic, goal } =
        &identity.payload.handoff.channel
    else {
        unreachable!()
    };
    assert!(
        f.call(Command::CreateThread(
            herdr_threads::protocol::commands::CreateThread {
                name: name.clone(),
                topic: topic.clone(),
                goal: goal.clone(),
                operation: identity.payload.handoff.keys.create.clone(),
                claim: identity.claim.clone()
            }
        ))
        .is_err()
    );
    assert!(
        f.call(Command::BeginHandoff(
            herdr_threads::protocol::handoff::HandoffMutation {
                identity: attachment.handoff.clone(),
                operation: identity.payload.handoff.keys.begin.clone(),
            }
        ))
        .is_err()
    );
    assert_eq!(
        raw(identity.payload.handoff.keys.invite.as_str()),
        before_invite
    );
    assert_eq!(
        raw(identity.payload.handoff.keys.send.as_str()),
        before_send
    );
    assert_eq!(f.creates.load(Ordering::Relaxed), 1);
    assert_eq!(f.starts.load(Ordering::Relaxed), 1);
}

fn public_unknown_created_pane_agent_retry_chain(retain_request: bool) {
    use herdr_threads::{
        cli::journal::{Journal, OriginalActor, classify_original_actor},
        protocol::{handoff::BootstrapIdentity, results::IntentKind},
    };
    let f = Fixture::with_creation_reply_loss(true, true, true);
    let setup = f.cli(
        &["setup", "codex", "--harness-binary", "/usr/bin/true"],
        true,
    );
    assert!(
        setup.status.success(),
        "{}",
        String::from_utf8_lossy(&setup.stderr)
    );
    let unknown = f.cli(
        &[
            "--cooperative-seat",
            "sender",
            "--cooperative-target",
            "w4:p1",
            "--cooperative-harness",
            "codex",
            "--cooperative-role",
            "top-level",
            "handoff",
            "--new-tab",
            "Peer",
            "--new-thread",
            "--kind",
            "codex",
            "--harness-binary",
            "/usr/bin/true",
            "--agent-arg=--model",
            "--agent-arg=frozen recovery model",
            "--",
            "frozen public recovery work",
        ],
        false,
    );
    assert!(!unknown.status.success());
    let pending: serde_json::Value = serde_json::from_slice(&unknown.stdout).unwrap();
    assert_eq!(pending["bootstrap"]["phase"], "creation");
    assert_eq!(pending["bootstrap"]["outcome"], "creation_unknown");
    assert_eq!(pending["bootstrap"]["attempt"], 1);
    assert!(
        String::from_utf8_lossy(&unknown.stderr).contains("outcome_unknown"),
        "{}",
        String::from_utf8_lossy(&unknown.stderr)
    );
    assert_eq!(f.creates.load(Ordering::Relaxed), 1);
    assert_eq!(f.starts.load(Ordering::Relaxed), 0);
    assert_eq!(
        f.db()
            .query_row("SELECT state FROM bootstrap_handoffs", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "possible_creation"
    );
    assert_eq!(
        f.db()
            .query_row("SELECT current_attempt FROM bootstrap_handoffs", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap(),
        1
    );
    let directory = f.paths.instance_dir.join("intents");
    let journal = Journal::open(&directory).unwrap();
    let pending = journal.page(&Default::default()).unwrap();
    let local = pending
        .items
        .iter()
        .find(|v| v.kind == IntentKind::HandoffBootstrap)
        .unwrap();
    let recovery = local.recovery_ref.as_str();
    let reference = journal.resolve_recovery_ref(recovery).unwrap();
    let original_path = directory.join(format!(
        "{:020}-{}.intent",
        reference.ordinal,
        reference.operation.as_str()
    ));
    let original = fs::read(&original_path).unwrap();
    let frozen = journal.load(&reference).unwrap();
    assert_eq!(
        classify_original_actor(&frozen.header.scope, &frozen.semantic).unwrap(),
        OriginalActor::Agent
    );
    let identity_bytes: Vec<u8> = f
        .db()
        .query_row("SELECT identity_json FROM bootstrap_handoffs", [], |r| {
            r.get(0)
        })
        .unwrap();
    let identity: BootstrapIdentity = serde_json::from_slice(&identity_bytes).unwrap();
    assert_eq!(
        identity.claim.harness,
        herdr_threads::protocol::authority::Harness::Codex
    );
    assert_eq!(identity.payload.handoff.body, "frozen public recovery work");
    assert_eq!(
        identity.payload.launch.argv,
        ["--model", "frozen recovery model"]
    );
    let progress_path =
        directory.join(format!("handoff-{}.progress", reference.operation.as_str()));
    let mut progress: Value = serde_json::from_slice(&fs::read(&progress_path).unwrap()).unwrap();
    assert_eq!(progress["possible_creation"], true);
    assert!(progress["creation"].is_null());
    let original_request = progress["request"].clone();
    assert!(
        original_request.is_object(),
        "actual producer must retain the submitted request"
    );
    if !retain_request {
        // A request-less valid progress record carries no invented local receipt.
        // Keep all fields from the real producer and change only its Option value.
        use std::os::unix::fs::OpenOptionsExt;
        progress["request"] = Value::Null;
        let temp = directory.join(".owned-request-absent");
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)
            .unwrap();
        file.write_all(&serde_json::to_vec(&progress).unwrap())
            .unwrap();
        file.sync_all().unwrap();
        fs::rename(temp, &progress_path).unwrap();
        fs::File::open(&directory).unwrap().sync_all().unwrap();
    }
    let saved_progress = fs::read(&progress_path).unwrap();
    let before_recovery_operations: Vec<(String, String, Vec<u8>, String)> = f.db()
        .prepare("SELECT actor_scope,operation_key,digest,result_json FROM operations ORDER BY actor_scope,operation_key")
        .unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap().map(Result::unwrap).collect();
    let journal_snapshot = || {
        let mut files: Vec<_> = fs::read_dir(&directory)
            .unwrap()
            .map(|e| {
                let path = e.unwrap().path();
                (
                    path.file_name().unwrap().to_owned(),
                    fs::read(path).unwrap(),
                )
            })
            .collect();
        files.sort();
        files
    };
    let before_wrong_actor = journal_snapshot();
    let wrong_human_retry = f.cli(&["retry", recovery], true);
    assert!(!wrong_human_retry.status.success());
    assert!(wrong_human_retry.stdout.is_empty());
    assert_eq!(journal_snapshot(), before_wrong_actor);
    let wrong_root_assertion = f.cli(
        &[
            "handoff",
            "recover",
            recovery,
            "--attempt",
            "1",
            "--created-pane",
            "w4:p3",
        ],
        false,
    );
    assert!(!wrong_root_assertion.status.success());
    assert!(wrong_root_assertion.stdout.is_empty());
    assert_eq!(journal_snapshot(), before_wrong_actor);
    assert_eq!(f.creates.load(Ordering::Relaxed), 1);
    assert_eq!(f.starts.load(Ordering::Relaxed), 0);
    f.rotate_owned_endpoint();
    let recovered = f.cli(
        &[
            "handoff",
            "recover",
            recovery,
            "--attempt",
            "1",
            "--created-pane",
            "w4:p3",
        ],
        true,
    );
    assert!(
        recovered.status.success(),
        "human inspection: {} {}",
        String::from_utf8_lossy(&recovered.stdout),
        String::from_utf8_lossy(&recovered.stderr)
    );
    assert_eq!(fs::read(&original_path).unwrap(), original);
    assert_eq!(fs::read(&progress_path).unwrap(), saved_progress);
    assert_eq!(
        f.db()
            .query_row("SELECT identity_json FROM bootstrap_handoffs", [], |r| r
                .get::<_, Vec<u8>>(0))
            .unwrap(),
        identity_bytes
    );
    assert_eq!(
        f.db()
            .query_row("SELECT state FROM bootstrap_handoffs", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "created"
    );
    assert_eq!(
        f.db()
            .query_row("SELECT count(*) FROM threads", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    let after_recovery_operations: Vec<(String, String, Vec<u8>, String)> = f.db()
        .prepare("SELECT actor_scope,operation_key,digest,result_json FROM operations ORDER BY actor_scope,operation_key")
        .unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap().map(Result::unwrap).collect();
    assert_eq!(
        after_recovery_operations, before_recovery_operations,
        "Human recovery preserves all preexisting decisions and stages no child work"
    );
    let created: herdr_threads::ports::CreatedTab = serde_json::from_slice(
        &f.db()
            .query_row("SELECT creation_json FROM bootstrap_attempts", [], |r| {
                r.get::<_, Vec<u8>>(0)
            })
            .unwrap(),
    )
    .unwrap();
    assert_eq!(created.root_pane.as_str(), "w4:p3");
    assert_ne!(
        serde_json::to_value(&created.witness).unwrap()["socket"],
        original_request["expected_witness"]["socket"]
    );
    if retain_request {
        assert_eq!(
            serde_json::to_value(&created.correlation).unwrap(),
            original_request["correlation"]
        );
    }
    assert_eq!(f.creates.load(Ordering::Relaxed), 1);
    assert_eq!(
        f.starts.load(Ordering::Relaxed),
        0,
        "Human recovery cannot launch"
    );
    let first = f.cli_with_pane(&["retry", recovery], false, Some("w4:p1"));
    assert!(
        !first.status.success(),
        "the owned native peer confirms a pre-start refusal"
    );
    assert_eq!(
        f.starts.load(Ordering::Relaxed),
        1,
        "first public retry stderr: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(f.creates.load(Ordering::Relaxed), 1);
    assert_eq!(
        f.db()
            .query_row("SELECT state FROM bootstrap_handoffs", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "attached"
    );
    assert_eq!(fs::read(&original_path).unwrap(), original);
    let retained: Value = serde_json::from_slice(&fs::read(&progress_path).unwrap()).unwrap();
    assert_eq!(
        retained["request"],
        if retain_request {
            original_request
        } else {
            Value::Null
        }
    );
    assert!(
        retained["creation"].is_null(),
        "fresh operator evidence cannot fabricate a local native submission receipt"
    );
    let operations = || {
        f.db().prepare("SELECT operation_key,digest,result_json FROM operations WHERE actor_scope='seat:sender' ORDER BY operation_key").unwrap().query_map([], |r|Ok((r.get::<_,String>(0)?,r.get::<_,Vec<u8>>(1)?,r.get::<_,String>(2)?))).unwrap().map(Result::unwrap).collect::<Vec<_>>()
    };
    let staged = operations();
    for key in [
        &identity.payload.handoff.keys.create,
        &identity.payload.handoff.keys.invite,
        &identity.payload.handoff.keys.send,
    ] {
        assert_eq!(staged.iter().filter(|v| v.0 == key.as_str()).count(), 1);
    }
    let staged_thread_count: i64 = f
        .db()
        .query_row("SELECT count(*) FROM threads", [], |r| r.get(0))
        .unwrap();
    assert_eq!(staged_thread_count, 1);
    f.allow_owned_start();
    let second = f.cli_with_pane(&["retry", recovery], false, Some("w4:p1"));
    assert!(
        second.status.success(),
        "second OriginalAgent public launcher retry: {} {}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );
    let frame: Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(frame["handoff"]["outcome"], "started");
    assert_eq!(frame["handoff"]["attempt"], 1);
    assert_eq!(f.creates.load(Ordering::Relaxed), 1);
    assert_eq!(
        f.starts.load(Ordering::Relaxed),
        2,
        "one refused submission and one successful start"
    );
    assert!(journal.load(&reference).is_err());
    assert!(!progress_path.exists());
    let completed_bytes: Vec<u8> = f
        .db()
        .query_row("SELECT completed_json FROM bootstrap_reports", [], |r| {
            r.get(0)
        })
        .unwrap();
    let completed: herdr_threads::protocol::handoff::CompletedBootstrapResult =
        serde_json::from_slice(&completed_bytes).unwrap();
    assert_eq!(completed.identity, identity);
    assert_eq!(completed.attachment.created, created);
    assert_eq!(completed.attachment.attempt.get(), 1);
    assert_eq!(completed.attachment.handoff.claim, identity.claim);
    let terminal = directory.join(format!(
        "bootstrap-{:020}-{}.terminal",
        reference.ordinal,
        reference.operation.as_str()
    ));
    let terminal_value: Value = serde_json::from_slice(&fs::read(terminal).unwrap()).unwrap();
    assert_eq!(
        terminal_value["original"].as_str().unwrap().as_bytes(),
        original
    );
    let completed_operations = operations();
    for row in staged {
        assert!(
            completed_operations.contains(&row),
            "exact child decision retained"
        );
    }
    f.db()
        .execute(
            "UPDATE occupant_bindings SET ended_at=1 WHERE seat_id='sender' AND ended_at IS NULL",
            [],
        )
        .unwrap();
    f.refuse_owned_pane_reads();
    let historical = f.cli(&["retry", recovery], false);
    assert!(
        historical.status.success(),
        "{}",
        String::from_utf8_lossy(&historical.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&historical.stdout).unwrap(),
        frame
    );
    assert_eq!(f.creates.load(Ordering::Relaxed), 1);
    assert_eq!(f.starts.load(Ordering::Relaxed), 2);
    assert_eq!(operations(), completed_operations);
    assert_eq!(
        f.db()
            .query_row("SELECT completed_json FROM bootstrap_reports", [], |r| r
                .get::<_, Vec<u8>>(0))
            .unwrap(),
        completed_bytes
    );
    assert_eq!(
        f.db()
            .query_row("SELECT count(*) FROM threads", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        staged_thread_count
    );
}

#[test]
fn elected_public_unknown_created_pane_retains_request_through_second_agent_retry() {
    let _serial = crate::ONE_DAEMON.lock().unwrap_or_else(|e| e.into_inner());
    public_unknown_created_pane_agent_retry_chain(true);
}

#[test]
fn elected_public_unknown_created_pane_without_request_uses_fresh_inspection() {
    let _serial = crate::ONE_DAEMON.lock().unwrap_or_else(|e| e.into_inner());
    public_unknown_created_pane_agent_retry_chain(false);
}

#[test]
fn current_main_registered_original_caller_preserves_bootstrap_and_delivery_claim() {
    use herdr_threads::protocol::{
        authority::{CallerClaim, CallerRole, Harness as AuthorityHarness},
        handoff::BootstrapIdentity,
        ids::*,
    };
    let _serial = crate::ONE_DAEMON.lock().unwrap_or_else(|e| e.into_inner());
    let hermes = AuthorityHarness::Agent(
        herdr_threads::harness::registry::builtins()
            .agent("hermes")
            .unwrap(),
    );
    let f = Fixture::with_original_harness(true, false, false, Harness::from(hermes));
    let execution: String = f
        .db()
        .query_row(
            "SELECT execution_id FROM occupant_bindings WHERE seat_id='sender'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let expected = CallerClaim {
        instance: f.instance.to_string(),
        seat: SeatId::new("sender"),
        target: HostTargetId::new("w4:p1"),
        binding_generation: 1,
        role: CallerRole::TopLevel,
        harness: hermes,
        native_session: NativeSessionId::new("session"),
        execution: ExecutionId::new(execution),
    };
    let setup = f.cli(
        &["setup", "codex", "--harness-binary", "/usr/bin/true"],
        true,
    );
    assert!(
        setup.status.success(),
        "{}",
        String::from_utf8_lossy(&setup.stderr)
    );
    let selection = [
        "--cooperative-seat",
        "sender",
        "--cooperative-target",
        "w4:p1",
        "--cooperative-harness",
        "hermes",
        "--cooperative-role",
        "top-level",
    ];
    let mut unsupported = selection.to_vec();
    unsupported.extend([
        "handoff",
        "--new-tab",
        "Unsupported",
        "--new-thread",
        "--kind",
        "hermes",
        "--",
        "opaque",
    ]);
    let refusal = f.cli(&unsupported, false);
    assert!(!refusal.status.success());
    assert!(refusal.stdout.is_empty());
    assert_eq!(
        f.db()
            .query_row("SELECT count(*) FROM bootstrap_handoffs", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(f.creates.load(Ordering::Relaxed), 0);
    assert_eq!(f.starts.load(Ordering::Relaxed), 0);
    for native in [false, true] {
        let mut args = selection.to_vec();
        if native {
            args.extend([
                "handoff",
                "--new-tab",
                "Hermes caller",
                "--new-thread",
                "--kind",
                "codex",
                "--harness-binary",
                "/usr/bin/true",
                "--",
                "Hermes original '$HOME' body",
            ]);
        } else {
            args.extend([
                "handoff",
                "--existing",
                "--seat",
                "recipient",
                "--new-thread",
                "--",
                "Hermes original '$HOME' body",
            ]);
        }
        let out = f.cli(&args, false);
        assert!(
            out.status.success(),
            "{} {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let frame: Value = serde_json::from_slice(&out.stdout).unwrap();
        let report = &frame[if native { "handoff" } else { "delivery" }];
        let reference = report["recovery_ref"].as_str().unwrap();
        if native {
            let bytes: Vec<u8> = f
                .db()
                .query_row("SELECT identity_json FROM bootstrap_handoffs", [], |r| {
                    r.get(0)
                })
                .unwrap();
            let identity: BootstrapIdentity = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(identity.claim, expected);
            assert_eq!(identity.payload.launch.harness, AuthorityHarness::Codex);
            assert_eq!(identity.digest, identity.semantic_digest().unwrap());
        }
        let db = f.db();
        let mut stmt = db
            .prepare("SELECT claim_json FROM channel_handoff_fences")
            .unwrap();
        for claim in stmt.query_map([], |r| r.get::<_, String>(0)).unwrap() {
            assert_eq!(
                serde_json::from_str::<CallerClaim>(&claim.unwrap()).unwrap(),
                expected
            );
        }
        assert_eq!(
            db.query_row(
                "SELECT observation_provenance FROM occupant_bindings WHERE seat_id='sender'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "cooperative_top_level"
        );
        drop(stmt);
        drop(db);
        for _ in 0..2 {
            let retry = f.cli(&["retry", reference], false);
            assert!(
                retry.status.success(),
                "{}",
                String::from_utf8_lossy(&retry.stderr)
            );
            assert_eq!(
                serde_json::from_slice::<Value>(&retry.stdout).unwrap(),
                frame
            );
        }
        assert_eq!(f.creates.load(Ordering::Relaxed), usize::from(native));
        assert_eq!(f.starts.load(Ordering::Relaxed), usize::from(native));
    }
}

#[test]
fn current_main_topology_native_preparation_refuses_before_publication() {
    let _serial = crate::ONE_DAEMON.lock().unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let setup = f.cli(
        &["setup", "codex", "--harness-binary", "/usr/bin/true"],
        true,
    );
    assert!(
        setup.status.success(),
        "{}",
        String::from_utf8_lossy(&setup.stderr)
    );
    let out = f.cli(
        &[
            "--cooperative-seat",
            "sender",
            "--cooperative-target",
            "w4:p1",
            "--cooperative-harness",
            "codex",
            "--cooperative-role",
            "top-level",
            "handoff",
            "--new-tab",
            "Refused",
            "--new-thread",
            "--kind",
            "codex",
            "--harness-binary",
            "/usr/bin/false",
            "--",
            "opaque",
        ],
        false,
    );
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("admitted executable must match"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        f.creates.load(Ordering::Relaxed),
        0,
        "statically refused native preparation must precede topology effects"
    );
    assert_eq!(f.starts.load(Ordering::Relaxed), 0);
    assert_eq!(
        f.db()
            .query_row("SELECT count(*) FROM bootstrap_handoffs", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(out.stdout.is_empty());
    let journal =
        herdr_threads::cli::journal::Journal::open(f.paths.instance_dir.join("intents")).unwrap();
    assert!(journal.page(&Default::default()).unwrap().items.is_empty());
}
