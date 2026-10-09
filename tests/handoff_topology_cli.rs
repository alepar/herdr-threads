//! Actual public CLI composition, with owned child daemons and model-free UDS peers.
//! These tests catch wrong routing/options, focus-based selection, duplicate durable
//! effects after reply loss and invented delivery/launch completion. No global state.
use herdr_threads::{
    app::SystemClock,
    daemon::{
        ownership::OwnerLock,
        paths::{InstancePaths, RuntimeContext},
    },
    harness::context::{ContextJournal, Harness, OccupantContext, Role, SessionReference},
    host::native::NativeCli,
    ports::{BootstrapObserver, DurableWorkAdmission, HostCallContext, SnapshotHeader, StorePort},
    protocol::time::{CallBudget, Cancellation, Clock, MonoInstant},
    store::{SqliteStore, StoreSettings, connection::StoreContext},
    test_support::{
        isolation::TestIsolation,
        spawn::{self, OwnedChild, SpawnOwned},
    },
};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, BufRead, BufReader, Read, Write},
    os::unix::{
        fs::{DirBuilderExt, MetadataExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use uuid::Uuid;

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");
const WAIT: Duration = Duration::from_secs(15);
const BODY: &str = "literal '$HOME' \"quoted\" work\n--agent-arg=body stays here";
const OPTIONS: &str = "--model 'env model' --config 'note=\"literal $HOME\"' --config repeat=true";
const FROZEN: &[&str] = &[
    "--model",
    "env model",
    "--config",
    "note=\"literal $HOME\"",
    "--config",
    "repeat=true",
    "--config",
    "caller=\"quoted value\"",
    "--config",
    "repeat=true",
];

fn wait(what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !ready() {
        assert!(Instant::now() < deadline, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}
fn finish(mut child: OwnedChild) -> Output {
    wait("owned CLI/helper exit", || {
        child.try_wait().unwrap().is_some()
    });
    child.wait_with_output().unwrap()
}
fn successful(out: Output) -> Value {
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
fn pane(id: &str, agent: Option<&Value>) -> Value {
    let (space, tab, terminal) = match id {
        "w4:p1" => ("w4", "w4:t1", "term_1"),
        "w4:p2" => ("w4", "w4:t1", "term_2"),
        "w4:p3" => ("w4", "w4:t2", "term_3"),
        "w5:p1" => ("w5", "w5:t1", "term_focus"),
        _ => panic!("unexpected private pane {id}"),
    };
    let mut value = json!({"pane_id":id,"terminal_id":terminal,"workspace_id":space,"tab_id":tab,"focused":id=="w5:p1","revision":1,"agent_status":"idle"});
    if let Some(agent) = agent {
        value["agent"] = agent["agent"].clone();
        value["agent_status"] = json!("working");
    }
    value
}
#[derive(Default)]
struct HostState {
    requests: Vec<Value>,
    created: bool,
    agent: Option<Value>,
    lose: Option<&'static str>,
    // Lose one reply only after the original journal publication is visible,
    // so the injected failure is a witnessed post-publication read.
    lose_after_publication: Option<(&'static str, PathBuf)>,
    lost_after_publication: Vec<Value>,
    helpers: Vec<Value>,
}
struct Host {
    state: Arc<Mutex<HostState>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    socket: PathBuf,
}
impl Host {
    fn start(iso: &TestIsolation, socket: &Path) -> Self {
        let listener = UnixListener::bind(socket).unwrap();
        let state = Arc::new(Mutex::new(HostState::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (shared, stopping) = (state.clone(), stop.clone());
        let root = iso.state_root().to_path_buf();
        let worker = std::thread::spawn(move || {
            while !stopping.load(Ordering::Acquire) {
                let (mut stream, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(e) => panic!("owned host accept: {e}"),
                };
                if stopping.load(Ordering::Acquire) {
                    break;
                }
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut line = String::new();
                if BufReader::new(&mut stream).read_line(&mut line).is_err() {
                    continue;
                }
                let Ok(request) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                let mut state = shared.lock().unwrap();
                state.requests.push(request.clone());
                let method = request["method"].as_str().unwrap();
                let result = match method {
                    "ping" => json!({"type":"pong","version":"0.9.1","protocol":22}),
                    "session.snapshot" => {
                        let mut tabs = vec![
                            json!({"tab_id":"w4:t1","workspace_id":"w4","label":"Original"}),
                            json!({"tab_id":"w5:t1","workspace_id":"w5","label":"Focused"}),
                        ];
                        let mut panes = vec![
                            pane("w4:p1", None),
                            pane(
                                "w4:p2",
                                state.agent.as_ref().filter(|a| a["pane_id"] == "w4:p2"),
                            ),
                            pane("w5:p1", None),
                        ];
                        if state.created {
                            tabs.push(json!({"tab_id":"w4:t2","workspace_id":"w4","label":"Peer"}));
                            panes.push(pane(
                                "w4:p3",
                                state.agent.as_ref().filter(|a| a["pane_id"] == "w4:p3"),
                            ));
                        }
                        json!({"type":"session_snapshot","snapshot":{"version":"0.9.1","protocol":22,"agents":state.agent.iter().collect::<Vec<_>>(),"layouts":[],"workspaces":[{"workspace_id":"w4","label":"Caller"},{"workspace_id":"w5","label":"Other","focused":true}],"tabs":tabs,"panes":panes}})
                    }
                    "pane.get" => {
                        let id = request["params"]["pane_id"].as_str().unwrap();
                        json!({"type":"pane_info","pane":pane(id,state.agent.as_ref().filter(|a|a["pane_id"]==id))})
                    }
                    "tab.create" => {
                        state.created = true;
                        json!({"type":"tab_created","tab":{"tab_id":"w4:t2","workspace_id":"w4"},"root_pane":pane("w4:p3",None)})
                    }
                    "agent.start" => {
                        let kind = request["params"]["kind"].as_str().unwrap();
                        assert!(matches!(kind, "claude" | "codex"));
                        let argv: Vec<String> =
                            serde_json::from_value(request["params"]["args"].clone()).unwrap();
                        // Native start carries kind+args, not the admitted binary path.
                        // This owned fake host maps that kind to its stand-in executable.
                        let mut helper = spawn::command(root.join(kind));
                        helper
                            .args(&argv)
                            .env("HOME", root.join("home"))
                            .env("CLAUDE_CONFIG_DIR", root.join("claude-config"))
                            .env("CODEX_HOME", root.join("codex-home"))
                            .stdin(Stdio::null())
                            .stdout(Stdio::piped())
                            .stderr(Stdio::piped());
                        let child = helper.spawn_owned().unwrap();
                        let pid = child.id();
                        let out = finish(child);
                        assert!(out.status.success());
                        let log: Value = serde_json::from_slice(&out.stdout).unwrap();
                        assert_eq!(log["argv"], request["params"]["args"]);
                        state
                            .helpers
                            .push(json!({"pid":pid,"reaped":true,"capture":log}));
                        let mut agent = pane(request["params"]["pane_id"].as_str().unwrap(), None);
                        agent["agent"] = json!(kind);
                        agent["agent_status"] = json!("working");
                        agent["name"] = request["params"]["name"].clone();
                        agent["interactive_ready"] = json!(true);
                        state.agent = Some(agent.clone());
                        json!({"type":"agent_started","argv":request["params"]["args"],"agent":agent})
                    }
                    "agent.get" => json!({"type":"agent_info","agent":state.agent}),
                    other => panic!("unexpected private native request {other}: {request}"),
                };
                if state.lose == Some(method) {
                    state.lose = None;
                    continue;
                }
                if let Some((selected, journal)) = &state.lose_after_publication
                    && *selected == method
                    && fs::read_dir(journal).is_ok_and(|entries| {
                        entries
                            .flatten()
                            .any(|e| e.path().extension().is_some_and(|x| x == "intent"))
                    })
                {
                    state.lose_after_publication = None;
                    state.lost_after_publication.push(request.clone());
                    continue;
                }
                let _ = writeln!(stream, "{}", json!({"id":request["id"],"result":result}));
            }
        });
        Self {
            state,
            stop,
            worker: Some(worker),
            socket: socket.to_path_buf(),
        }
    }
    fn stop_and_join(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = UnixStream::connect(&self.socket);
        if let Some(worker) = self.worker.take()
            && worker.join().is_err()
        {
            let _ = writeln!(std::io::stderr(), "owned host panicked during cleanup");
        }
    }
    fn requests(&self, method: &str) -> Vec<Value> {
        self.state
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|r| r["method"] == method)
            .cloned()
            .collect()
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        self.stop_and_join();
    }
}

type IdentityRow = Vec<(String, rusqlite::types::Value)>;
#[derive(Debug, PartialEq)]
struct RecipientIdentity {
    seat: IdentityRow,
    bindings: Vec<IdentityRow>,
}

struct Fixture {
    daemon: OwnedChild,
    host: Host,
    iso: TestIsolation,
    context: RuntimeContext,
    paths: InstancePaths,
    instance: Uuid,
    cli_log: Mutex<Vec<Value>>,
}
impl Fixture {
    fn new() -> Self {
        Self::with_instance(None)
    }
    fn with_instance(copied: Option<Uuid>) -> Self {
        Self::with_layout(copied, false)
    }
    fn with_layout(copied: Option<Uuid>, long_state: bool) -> Self {
        let iso = TestIsolation::new("handoff-topology-cli");
        for dir in ["claude-config", "codex-home", "cwd"] {
            fs::create_dir(iso.path(dir)).unwrap();
        }
        let python = spawn::command("/usr/bin/which")
            .arg("python3")
            .output()
            .unwrap();
        assert!(python.status.success());
        let python = String::from_utf8(python.stdout).unwrap();
        // A tiny owned helper captures literal argv/environment; no harness/model is invoked.
        let script = format!(
            "#!{}\nimport json,os,sys\nprint(json.dumps({{'argv':sys.argv[1:],'home':os.environ['HOME'],'claude':os.environ['CLAUDE_CONFIG_DIR'],'codex':os.environ['CODEX_HOME']}}))\n",
            python.trim()
        );
        executable(&iso.path("claude"), &script);
        executable(&iso.path("codex"), &script);
        executable(
            &iso.path("shell"),
            "#!/bin/sh\nprintf '%s\\n' probe >> \"$HT_SMOKE_SHELL_LOG\"\nprintf '%s' HT_PANE_ENV_BEGINHT_PANE_ENV_END\n",
        );
        let socket = iso.socket_path("host.sock");
        let host = Host::start(&iso, &socket);
        let mut state = if long_state {
            iso.socket_path("state")
        } else {
            iso.path("state")
        };
        if long_state {
            for _ in 0..2 {
                state = state.join("\u{1}".repeat(170));
            }
        }
        fs::create_dir_all(&state).unwrap();
        let context = RuntimeContext::explicit(state, socket, None).unwrap();
        let paths = InstancePaths::resolve(&context).unwrap();
        let owner = OwnerLock::acquire(&paths).unwrap();
        let mut instance = owner.instance_uuid();
        drop(owner);
        if let Some(copied) = copied {
            fs::write(&paths.namespace_path, copied.to_string()).unwrap();
            let owner = OwnerLock::acquire(&paths).unwrap();
            instance = owner.instance_uuid();
            assert_eq!(instance, copied);
            drop(owner);
        }
        Self::seed(&paths, &context, instance);
        let mut cmd = iso.command(BIN);
        cmd.args(["daemon", "run", "--state-dir"])
            .arg(&context.state_dir)
            .arg("--host-endpoint")
            .arg(&context.host_endpoint)
            .env("CLAUDE_CONFIG_DIR", iso.path("claude-config"))
            .env("CODEX_HOME", iso.path("codex-home"))
            .stdout(fs::File::create(iso.path("daemon.stdout")).unwrap())
            .stderr(fs::File::create(iso.path("daemon.stderr")).unwrap());
        let mut daemon = cmd.spawn_owned().unwrap();
        wait("owned elected daemon descriptor/reconciliation", || {
            assert!(daemon.try_wait().unwrap().is_none(), "owned daemon exited");
            if !paths.descriptor_path.exists() {
                return false;
            }
            let db = Connection::open(&paths.database_path).unwrap();
            db.query_row("SELECT count(*) FROM host_instances WHERE reconciled_boot IS NOT NULL AND reconciled_boot=recovery_boot AND reconciled_epoch=recovery_epoch",[],|r|r.get::<_,i64>(0)).unwrap_or(0)>0
        });
        println!(
            "fixture instance={instance} root={} host={} daemon={}",
            iso.state_root().display(),
            context.host_endpoint.display(),
            daemon.id()
        );
        Self {
            daemon,
            host,
            iso,
            context,
            paths,
            instance,
            cli_log: Mutex::new(Vec::new()),
        }
    }
    fn seed(paths: &InstancePaths, context: &RuntimeContext, instance: Uuid) {
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let native = NativeCli::new(context.host_endpoint.clone(), clock.clone());
        let budget = CallBudget {
            deadline: MonoInstant(clock.monotonic_now().0 + 5000),
            cancellation: Cancellation::default(),
        };
        let ctx = HostCallContext {
            budget: budget.clone(),
            expected_boot: None,
            expected_epoch: None,
        };
        let observed = native
            .observe_bootstrap_target(
                &herdr_threads::protocol::ids::HostTargetId::new("w4:p1"),
                &ctx,
            )
            .unwrap();
        let observation = observed.observation();
        let store_context = StoreContext::new(paths.database_path.clone(), clock.clone());
        let db = store_context.open_writer().unwrap();
        db.execute(
            "INSERT INTO host_instances(id,created_at) VALUES(?1,0)",
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
        let snapshot = herdr_threads::ports::HostPort::enumerate_targets(&native, &ctx).unwrap();
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
            db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,structural_terminal_id,structural_incarnation,structural_host_boot,structural_host_epoch,structural_incarnation_kind,structural_connection_epoch,structural_observation_sequence,created_at) VALUES(?1,?2,'resolved','native',?3,1,1,?4,?5,?5,1,'native_current_target',1,1,0)",params![seat,instance.to_string(),target,terminal,observation.host_boot.as_str()]).unwrap();
        }
        db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES('sender',1,'w4:p1',?1,1,1,'codex','session',?2,'cooperative_top_level',0,0,'term_1',?1)",params![observation.host_boot.as_str(),execution.to_string()]).unwrap();
        let contexts = paths
            .instance_dir
            .join("contexts")
            .join(format!("{:x}", Sha256::digest(b"sender")));
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&contexts)
            .unwrap();
        fs::set_permissions(&contexts, fs::Permissions::from_mode(0o700)).unwrap();
        ContextJournal::open(&contexts, instance, "sender", Duration::from_secs(1))
            .unwrap()
            .install_reattached(OccupantContext {
                format_version: 1,
                instance,
                seat: "sender".into(),
                target: "w4:p1".into(),
                harness: Harness::Codex,
                binding_generation: 1,
                execution,
                session: SessionReference::Native("session".into()),
                role: Role::TopLevel,
            })
            .unwrap();
    }
    fn owned_path(&self) -> std::ffi::OsString {
        let mut paths = vec![self.iso.path("")];
        if let Some(path) = std::env::var_os("PATH") {
            paths.extend(std::env::split_paths(&path));
        }
        std::env::join_paths(paths).unwrap()
    }
    fn command(&self) -> Command {
        self.command_in(&self.context.state_dir, &self.context.host_endpoint)
    }
    fn command_in(&self, state: &Path, host: &Path) -> Command {
        self.command_in_format(state, host, true)
    }
    fn command_in_format(&self, state: &Path, host: &Path, json: bool) -> Command {
        let mut cmd = self.iso.command(BIN);
        if json {
            cmd.arg("--json");
        }
        cmd.arg("--state-dir")
            .arg(state)
            .arg("--host-endpoint")
            .arg(host)
            .env("CLAUDE_CONFIG_DIR", self.iso.path("claude-config"))
            .env("CODEX_HOME", self.iso.path("codex-home"))
            .env("PATH", self.owned_path())
            .env("SHELL", self.iso.path("shell"))
            .env("HT_SMOKE_SHELL_LOG", self.iso.path("shell.log"))
            .env("HERDR_PLUGIN_STATE_DIR", self.iso.path("wrong-state"))
            .env("HERDR_SOCKET_PATH", self.iso.socket_path("wrong.sock"))
            .current_dir(self.iso.path("cwd"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd
    }
    fn run(&self, args: &[&str], options: &str) -> Output {
        self.run_format(args, options, true)
    }
    fn run_format(&self, args: &[&str], options: &str, json: bool) -> Output {
        let mut cmd = if json {
            self.command()
        } else {
            self.command_in_format(&self.context.state_dir, &self.context.host_endpoint, false)
        };
        cmd.args([
            "--cooperative-seat",
            "sender",
            "--cooperative-target",
            "w4:p1",
            "--cooperative-harness",
            "codex",
            "--cooperative-role",
            "top-level",
        ])
        .args(args)
        .env("HERDR_THREADS_CODEX_OPTS", options)
        .env("HERDR_THREADS_CLAUDE_OPTS", options);
        self.capture(cmd)
    }
    fn capture(&self, mut cmd: Command) -> Output {
        let argv: Vec<_> = cmd
            .get_args()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        let child = cmd.spawn_owned().unwrap();
        let pid = child.id();
        let out = finish(child);
        self.cli_log.lock().unwrap().push(json!({"argv":argv,"pid":pid,"code":out.status.code(),"out":String::from_utf8_lossy(&out.stdout),"err":String::from_utf8_lossy(&out.stderr)}));
        out
    }
    fn setup(&self, kind: &str) {
        let mut cmd = self.iso.command(BIN);
        cmd.arg("human")
            .args(["--json", "--state-dir"])
            .arg(&self.context.state_dir)
            .arg("--host-endpoint")
            .arg(&self.context.host_endpoint)
            .args(["setup", kind, "--harness-binary"])
            .arg(self.iso.path(kind))
            .env("PATH", self.owned_path())
            .env("CLAUDE_CONFIG_DIR", self.iso.path("claude-config"))
            .env("CODEX_HOME", self.iso.path("codex-home"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        successful(self.capture(cmd));
    }
    fn db(&self) -> Connection {
        let db = Connection::open(&self.paths.database_path).unwrap();
        db.busy_timeout(Duration::from_secs(5)).unwrap();
        db
    }
    fn count(&self, sql: &str) -> i64 {
        self.db().query_row(sql, [], |r| r.get(0)).unwrap()
    }
    fn recipient_identity(&self) -> RecipientIdentity {
        let db = self.db();
        let tx = db.unchecked_transaction().unwrap();
        // Compare complete schema-27 rows, including observation/registration
        // timestamps. This fixed topology has no legitimate recipient transition.
        let rows = |sql: &str| {
            let mut stmt = tx.prepare(sql).unwrap();
            let columns: Vec<_> = stmt.column_names().into_iter().map(str::to_owned).collect();
            stmt.query_map([], |row| {
                columns
                    .iter()
                    .enumerate()
                    .map(|(i, name)| Ok((name.clone(), row.get(i)?)))
                    .collect::<rusqlite::Result<IdentityRow>>()
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
        };
        let mut seats = rows("SELECT * FROM seats WHERE id='recipient'");
        assert_eq!(seats.len(), 1);
        RecipientIdentity {
            seat: seats.pop().unwrap(),
            bindings: rows(
                "SELECT * FROM occupant_bindings WHERE seat_id='recipient' ORDER BY ordinal",
            ),
        }
    }
    fn journal(&self) -> PathBuf {
        self.paths.instance_dir.join("intents")
    }
    fn reference(&self) -> String {
        let files: Vec<_> = fs::read_dir(self.journal())
            .unwrap()
            .filter_map(|e| {
                let p = e.unwrap().path();
                (p.extension().is_some_and(|s| s == "intent")).then_some(p)
            })
            .collect();
        assert_eq!(files.len(), 1, "one original compound: {files:?}");
        let text = fs::read_to_string(&files[0]).unwrap();
        let header: Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        format!("local:{}", header["reference"]["ordinal"].as_u64().unwrap())
    }
    fn effect_counts(&self) -> (usize, usize, i64, i64) {
        (
            self.host.requests("tab.create").len(),
            self.host.requests("agent.start").len(),
            self.count("SELECT count(*) FROM invitations"),
            self.count("SELECT count(*) FROM messages WHERE kind='ordinary'"),
        )
    }
    fn assert_work(&self, recipient: &str, thread: &str) {
        let db = self.db();
        let body: String = db
            .query_row(
                "SELECT body FROM messages WHERE kind='ordinary' AND thread_id=?1",
                [thread],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(body, BODY);
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM prepared_recipients p JOIN send_manifests m ON m.preparation_id=p.preparation_id WHERE p.seat_id=?1 AND p.thread_id=?2 AND p.ack_required=1",
                params![recipient, thread],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            db.query_row(
                "SELECT (SELECT count(*) FROM receipts WHERE state='acked')+(SELECT count(*) FROM receipt_state WHERE state='acked')",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM channel_handoff_fences WHERE state='completed'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }
    fn assert_launch(&self, kind: &str, target: &str) {
        self.assert_launch_argv(kind, target, FROZEN);
    }
    fn assert_launch_argv(&self, kind: &str, target: &str, frozen: &[&str]) {
        let starts = self.host.requests("agent.start");
        assert_eq!(starts.len(), 1);
        assert_eq!(starts[0]["params"]["kind"], kind);
        assert_eq!(starts[0]["params"]["pane_id"], target);
        let argv: Vec<String> =
            serde_json::from_value(starts[0]["params"]["args"].clone()).unwrap();
        assert_eq!(&argv[..frozen.len()], frozen);
        assert_eq!(argv.len(), frozen.len() + 1);
        let bootstrap = argv.last().unwrap();
        assert!(bootstrap.contains(&self.instance.to_string()));
        assert!(bootstrap.contains(self.context.state_dir.to_str().unwrap()));
        assert!(bootstrap.contains(self.context.host_endpoint.to_str().unwrap()));
        assert!(!bootstrap.contains(BODY));
        let host = self.host.state.lock().unwrap();
        assert_eq!(host.helpers.len(), 1);
        let log = &host.helpers[0]["capture"];
        assert_eq!(log["home"], self.iso.home().to_str().unwrap());
        assert_eq!(
            log["claude"],
            self.iso.path("claude-config").to_str().unwrap()
        );
        assert_eq!(log["codex"], self.iso.path("codex-home").to_str().unwrap());
    }
    fn named_thread(&self) -> String {
        successful(self.run(
            &[
                "thread",
                "create",
                "--name",
                "chosen",
                "--topic",
                "Private existing channel",
                "--goal",
                "Exact recipient",
            ],
            "",
        ));
        self.db()
            .query_row("SELECT id FROM threads WHERE name='chosen'", [], |r| {
                r.get(0)
            })
            .unwrap()
    }
    fn peer_binding(&self) {
        self.db().execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) SELECT 'recipient',1,'w4:p2',host_boot,host_epoch,1,'codex','peer-session',?1,'cooperative_top_level',observed_at,registered_at,'term_2',incarnation FROM occupant_bindings WHERE seat_id='sender'",[Uuid::new_v4().to_string()]).unwrap();
    }
    fn evidence_dir(&self, out: impl AsRef<Path>) -> PathBuf {
        out.as_ref()
            .join(self.iso.state_root().file_name().unwrap())
    }
    fn original_bytes(&self) -> Vec<(PathBuf, Vec<u8>)> {
        let mut files: Vec<_> = fs::read_dir(self.journal())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                p.extension()
                    .is_some_and(|s| matches!(s.to_str(), Some("intent" | "terminal" | "progress")))
            })
            .map(|p| {
                let b = fs::read(&p).unwrap();
                (p, b)
            })
            .collect();
        files.sort_by(|a, b| a.0.cmp(&b.0));
        files
    }
    fn new_thread_args<'a>(&'a self, kind: &'a str, binary: &'a str) -> Vec<&'a str> {
        vec![
            "handoff",
            "--new-tab",
            "Peer",
            "--new-thread",
            "--topic",
            "Private smoke",
            "--goal",
            "Exact durable work",
            "--kind",
            kind,
            "--harness-binary",
            binary,
            "--agent-arg=--config",
            "--agent-arg=caller=\"quoted value\"",
            "--agent-arg=--config",
            "--agent-arg=repeat=true",
            "--",
            BODY,
        ]
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.daemon.stop();
        self.host.stop_and_join();
        if let Some(out) = std::env::var_os("HT_HANDOFF_SMOKE_OUT") {
            // Evidence failures must not interrupt resource cleanup during unwinding.
            let captured = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let out = self.evidence_dir(PathBuf::from(out));
                fs::create_dir_all(&out).unwrap();
                for file in ["daemon.stdout", "daemon.stderr"] {
                    fs::copy(self.iso.path(file), out.join(file)).unwrap();
                }
                fs::write(
                    out.join("cli.json"),
                    serde_json::to_vec_pretty(&*self.cli_log.lock().unwrap()).unwrap(),
                )
                .unwrap();
                let host = self.host.state.lock().unwrap();
                fs::write(out.join("host.json"),serde_json::to_vec_pretty(&json!({"requests":host.requests,"helpers":host.helpers,"created":host.created,"agent":host.agent})).unwrap()).unwrap();
                // Store snapshot for independent real-row inspection, outside Cargo target.
                self.db()
                    .execute(
                        "VACUUM INTO ?1",
                        [out.join("store.sqlite3").to_str().unwrap()],
                    )
                    .unwrap();
                let originals:Vec<_>=fs::read_dir(self.journal()).into_iter().flatten().map(|e| {let p=e.unwrap().path(); json!({"name":p.file_name().unwrap().to_string_lossy(),"bytes":fs::read(&p).unwrap()})}).collect();
                fs::write(
                    out.join("journal.json"),
                    serde_json::to_vec_pretty(&originals).unwrap(),
                )
                .unwrap();
                fs::write(out.join("inventory.json"),serde_json::to_vec_pretty(&json!({"instance":self.instance,"root":self.iso.state_root(),"host":self.context.host_endpoint,"daemon":self.daemon.id(),"daemon_reaped":self.daemon.try_wait().unwrap().is_some(),"host_joined":self.host.worker.is_none(),"topology":if host.created {vec!["w4:t2","w4:p3"]} else {vec![]}})).unwrap()).unwrap();
            }));
            if captured.is_err() {
                let _ = writeln!(std::io::stderr(), "owned evidence capture failed");
            }
        }
    }
}

// Losing configured-prefix ordering, repeated args, body separation or selected
// pane routing breaks this actual executable test, not a coordinator mock.
#[test]
fn explicit_pane_preserves_both_harness_configs() {
    for kind in ["claude", "codex"] {
        let f = Fixture::new();
        f.setup(kind);
        let binary = f.iso.path(kind);
        let binary = binary.to_str().unwrap();
        let (flag, options, frozen) = if kind == "claude" {
            (
                "--agent-arg=--model",
                "--model 'env model' --model 'note=\"literal $HOME\"' --model repeat=true",
                &[
                    "--model",
                    "env model",
                    "--model",
                    "note=\"literal $HOME\"",
                    "--model",
                    "repeat=true",
                    "--model",
                    "caller=\"quoted value\"",
                    "--model",
                    "repeat=true",
                ][..],
            )
        } else {
            ("--agent-arg=--config", OPTIONS, FROZEN)
        };
        let out = successful(f.run(
            &[
                "handoff",
                "--pane",
                "w4:p2",
                "--new-thread",
                "--topic",
                "Private smoke",
                "--goal",
                "Exact durable work",
                "--kind",
                kind,
                "--harness-binary",
                binary,
                flag,
                "--agent-arg=caller=\"quoted value\"",
                flag,
                "--agent-arg=repeat=true",
                "--",
                BODY,
            ],
            options,
        ));
        assert_eq!(out["handoff"]["outcome"], "started");
        f.assert_launch_argv(kind, "w4:p2", frozen);
        f.assert_work("recipient", out["handoff"]["thread"].as_str().unwrap());
        assert_eq!(f.effect_counts(), (0, 1, 1, 1));
        let replay = f.run(
            &["retry", out["handoff"]["recovery_ref"].as_str().unwrap()],
            "'malformed",
        );
        assert!(!replay.status.success());
        assert!(String::from_utf8_lossy(&replay.stderr).contains("intent reference not found"));
        assert_eq!(f.effect_counts(), (0, 1, 1, 1));
    }
}

// Selecting focus instead of caller workspace, copying caller env, or losing
// canonical cwd/routing freezes breaks the complete typed native request.
#[test]
fn new_tab_preserves_both_harness_configs_and_caller_workspace() {
    for kind in ["claude", "codex"] {
        let f = Fixture::new();
        f.setup(kind);
        let binary = f.iso.path(kind);
        let binary = binary.to_str().unwrap();
        let mut args = f.new_thread_args(kind, binary);
        let cwd = f.iso.path("custom");
        fs::create_dir(&cwd).unwrap();
        if kind == "claude" {
            args.splice(3..3, ["--space", "Caller", "--cwd", cwd.to_str().unwrap()]);
        }
        let out = successful(f.run(&args, OPTIONS));
        assert_eq!(out["handoff"]["outcome"], "started");
        f.assert_launch(kind, "w4:p3");
        let creates = f.host.requests("tab.create");
        assert_eq!(creates.len(), 1);
        assert_eq!(
            creates[0]["params"],
            json!({"workspace_id":"w4","cwd":if kind=="claude" {cwd.canonicalize().unwrap()} else {f.iso.path("cwd").canonicalize().unwrap()},"label":"Peer","focus":false,"env":{}})
        );
        assert_eq!(
            f.count("SELECT count(*) FROM bootstrap_handoffs WHERE state='completed'"),
            1
        );
        assert_eq!(f.count("SELECT count(*) FROM bootstrap_reports"), 1);
        let thread = out["handoff"]["thread"].as_str().unwrap();
        let recipient = out["handoff"]["seat"].as_str().unwrap();
        f.assert_work(recipient, thread);
        let bytes: Vec<u8> = f
            .db()
            .query_row("SELECT identity_json FROM bootstrap_handoffs", [], |r| {
                r.get(0)
            })
            .unwrap();
        let id: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            id["payload"]["handoff"]["namespace"],
            json!({"instance":f.instance,"state_dir":f.context.state_dir,"host_endpoint":f.context.host_endpoint})
        );
        assert_eq!(id["payload"]["handoff"]["body"], BODY);
        assert_eq!(id["payload"]["launch"]["argv"], json!(FROZEN));
        let alias = f.iso.path("state-alias");
        std::os::unix::fs::symlink(&f.context.state_dir, &alias).unwrap();
        let host_parent = f.iso.path("host-alias");
        std::os::unix::fs::symlink(f.context.host_endpoint.parent().unwrap(), &host_parent)
            .unwrap();
        let mut cmd = f.command_in(&alias, &host_parent.join("host.sock"));
        cmd.args(["retry", out["handoff"]["recovery_ref"].as_str().unwrap()]);
        let replay = successful(f.capture(cmd));
        assert_eq!(replay, out);
        assert_eq!(f.effect_counts(), (1, 1, 1, 1));
    }
}

fn read_frame(stream: &mut UnixStream) -> io::Result<Vec<u8>> {
    let mut prefix = [0; 4];
    stream.read_exact(&mut prefix)?;
    let len = u32::from_be_bytes(prefix) as usize;
    if len > 1_048_576 {
        return Err(io::Error::other("oversized private proxy frame"));
    }
    let mut body = vec![0; len];
    stream.read_exact(&mut body)?;
    Ok(body)
}
fn write_frame(stream: &mut UnixStream, body: &[u8]) -> io::Result<()> {
    stream.write_all(&(body.len() as u32).to_be_bytes())?;
    stream.write_all(body)
}
struct ReplyLoss {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<io::Result<()>>>,
    dropped: Arc<Mutex<Option<Value>>>,
    socket: PathBuf,
    upstream: PathBuf,
    descriptor: PathBuf,
    original: Vec<u8>,
}
impl ReplyLoss {
    fn new(f: &Fixture, kind: &'static str, phase: Option<&'static str>) -> Self {
        let original = fs::read(&f.paths.descriptor_path).unwrap();
        let socket = f.paths.socket_path.clone();
        let upstream = socket.with_extension("upstream");
        fs::rename(&socket, &upstream).unwrap();
        // Own restoration before any further fallible constructor operation.
        let mut proxy = Self {
            stop: Arc::new(AtomicBool::new(false)),
            worker: None,
            dropped: Arc::new(Mutex::new(None)),
            socket,
            upstream,
            descriptor: f.paths.descriptor_path.clone(),
            original,
        };
        let listener = UnixListener::bind(&proxy.socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        fs::set_permissions(&proxy.socket, fs::Permissions::from_mode(0o600)).unwrap();
        let meta = fs::metadata(&proxy.socket).unwrap();
        let mut descriptor: Value = serde_json::from_slice(&proxy.original).unwrap();
        descriptor["socket_device"] = json!(meta.dev());
        descriptor["socket_inode"] = json!(meta.ino());
        fs::write(&proxy.descriptor, serde_json::to_vec(&descriptor).unwrap()).unwrap();
        let (stop, dropped, target) = (
            proxy.stop.clone(),
            proxy.dropped.clone(),
            proxy.upstream.clone(),
        );
        proxy.worker = Some(std::thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                let (mut client, _) = match listener.accept() {
                    Ok(v) => v,
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(e) => return Err(e),
                };
                // macOS accepted sockets inherit O_NONBLOCK from the listener.
                // Ordinary public requests write prefix and body separately.
                client.set_nonblocking(false)?;
                client.set_read_timeout(Some(Duration::from_secs(5)))?;
                client.set_write_timeout(Some(Duration::from_secs(5)))?;
                let Ok(request) = read_frame(&mut client) else {
                    continue;
                };
                let parsed: Value = serde_json::from_slice(&request)?;
                let mut daemon = UnixStream::connect(&target)?;
                daemon.set_read_timeout(Some(Duration::from_secs(5)))?;
                daemon.set_write_timeout(Some(Duration::from_secs(5)))?;
                write_frame(&mut daemon, &request)?;
                let response = read_frame(&mut daemon)?;
                let selected = parsed["command"]["kind"] == kind
                    && phase.is_none_or(|p| parsed["command"]["args"]["action"]["phase"] == p);
                let mut log = dropped.lock().unwrap();
                if selected && log.is_none() {
                    let reply: Value = serde_json::from_slice(&response)?;
                    if reply["result"]["Ok"].is_null() {
                        return Err(io::Error::other(format!(
                            "selected phase did not commit: {reply}"
                        )));
                    }
                    *log = Some(json!({"request":parsed,"response":reply}));
                    continue; // Successful daemon decision; lose only its reply.
                }
                let _ = write_frame(&mut client, &response);
            }
            Ok(())
        }));
        proxy
    }
    fn evidence(&self) -> Value {
        self.dropped
            .lock()
            .unwrap()
            .clone()
            .expect("selected actual commit reply was dropped")
    }
    fn finish(&mut self) -> io::Result<()> {
        self.stop.store(true, Ordering::Release);
        let result = if let Some(worker) = self.worker.take() {
            worker
                .join()
                .unwrap_or_else(|_| Err(io::Error::other("private reply proxy panicked")))
        } else {
            Ok(())
        };
        // Restore even if the forwarding worker failed.
        let _ = fs::remove_file(&self.socket);
        fs::rename(&self.upstream, &self.socket)?;
        fs::write(&self.descriptor, &self.original)?;
        result
    }
}
impl Drop for ReplyLoss {
    fn drop(&mut self) {
        if self.upstream.exists()
            && let Err(e) = self.finish()
        {
            let _ = writeln!(std::io::stderr(), "owned proxy cleanup: {e}");
        }
    }
}
fn unknown(out: &Output) {
    assert!(!out.status.success());
    assert!(
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
        .contains("unknown_outcome"),
        "{} {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

// Delivery must neither allocate a seat nor preflight/start a native harness;
// pending invitation episodes, joined peers and unbound staging stay distinct.
#[test]
fn existing_peer_pane_and_seat_do_not_launch() {
    for (selector, state, want_invites, want_participation) in [
        ("--pane", "joined", 0, "joined"),
        ("--seat", "pending", 1, "invited_pending"),
        ("--pane", "unbound", 1, "staged_unbound"),
    ] {
        let f = Fixture::new();
        let thread = f.named_thread();
        if state != "unbound" {
            f.peer_binding();
        }
        if state == "joined" {
            f.db().execute("INSERT INTO memberships(thread_id,seat_id,state,joined_at) VALUES(?1,'recipient','joined',0)",[&thread]).unwrap();
            f.db().execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES(?1,'recipient',1,1)",[&thread]).unwrap();
        } else if state == "pending" {
            successful(f.run(&["invite", &thread, "--seat", "recipient"], ""));
        }
        let before = f.count("SELECT count(*) FROM seats");
        let bindings = f.count("SELECT count(*) FROM occupant_bindings");
        let identity = f.recipient_identity();
        println!("recipient {state} before delivery: {identity:?}");
        let target = if selector == "--pane" {
            "w4:p2"
        } else {
            "recipient"
        };
        let frame = successful(f.run(
            &[
                "handoff",
                "--existing",
                selector,
                target,
                "--thread",
                "chosen",
                "--",
                BODY,
            ],
            "'malformed launch option",
        ));
        assert_eq!(frame["delivery"]["outcome"], "staged");
        assert_eq!(frame["delivery"]["participation"], want_participation);
        assert_eq!(frame["delivery"]["recipient"], "recipient");
        assert_eq!(frame["delivery"]["thread"], thread);
        f.assert_work("recipient", &thread);
        assert_eq!(f.effect_counts(), (0, 0, want_invites, 1));
        assert_eq!(f.count("SELECT count(*) FROM seats"), before);
        assert_eq!(f.count("SELECT count(*) FROM occupant_bindings"), bindings);
        let delivered_identity = f.recipient_identity();
        println!("recipient {state} after delivery: {delivered_identity:?}");
        assert_eq!(delivered_identity, identity);
        assert!(f.host.state.lock().unwrap().helpers.is_empty());
        assert!(!f.iso.path("shell.log").exists());
        assert_eq!(
            f.count("SELECT count(*) FROM invitations WHERE state='accepted'"),
            0
        );
        let exact = f.effect_counts();
        let replay = successful(f.run(
            &["retry", frame["delivery"]["recovery_ref"].as_str().unwrap()],
            "'malformed",
        ));
        assert_eq!(replay, frame);
        assert_eq!(f.effect_counts(), exact);
        assert_eq!(f.count("SELECT count(*) FROM seats"), before);
        assert_eq!(f.count("SELECT count(*) FROM occupant_bindings"), bindings);
        let replayed_identity = f.recipient_identity();
        println!("recipient {state} after retry: {replayed_identity:?}");
        assert_eq!(replayed_identity, identity);
        assert!(!f.iso.path("shell.log").exists());
    }
}

// Repeat child requests after actual canonical commitment, not simulated fake
// client results. Frozen keys conserve episodes/messages and possible start.
#[test]
fn public_durable_reply_loss_reuses_exact_children() {
    for (kind, phase, mode) in [
        ("create_thread", None, "bootstrap"),
        ("invite", None, "legacy"),
        ("handoff_delivery", Some("send"), "delivery"),
        ("complete_handoff", None, "legacy"),
        ("complete_linked_bootstrap", None, "bootstrap"),
        ("handoff_delivery", Some("complete"), "delivery"),
    ] {
        let f = Fixture::new();
        if mode != "delivery" {
            f.setup("codex");
        }
        let existing = if kind == "invite" {
            Some(f.named_thread())
        } else {
            None
        };
        let binary = f.iso.path("codex");
        let binary = binary.to_str().unwrap();
        let mut proxy = ReplyLoss::new(&f, kind, phase);
        let mut args = if mode == "delivery" {
            vec![
                "handoff",
                "--existing",
                "--seat",
                "recipient",
                "--new-thread",
                "--",
                BODY,
            ]
        } else {
            f.new_thread_args("codex", binary)
        };
        if mode == "legacy" {
            args.splice(1..3, ["--pane", "w4:p2"]);
        }
        if existing.is_some() {
            let index = args.iter().position(|s| *s == "--new-thread").unwrap();
            args.splice(index..index + 5, ["--thread", "chosen"]);
        }
        let out = f.run(&args, OPTIONS);
        let initial = out
            .status
            .success()
            .then(|| serde_json::from_slice::<Value>(&out.stdout).unwrap());
        let loss = proxy.evidence(); // Loss can be resolved immediately by canonical status.
        assert!(initial.is_none() || mode != "legacy");
        println!("committed reply loss mode={mode} kind={kind} phase={phase:?}: {loss}");
        let reference = initial.as_ref().map_or_else(
            || f.reference(),
            |frame| {
                frame[if mode == "delivery" {
                    "delivery"
                } else {
                    "handoff"
                }]["recovery_ref"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            },
        );
        let original = f.original_bytes();
        assert!(!original.is_empty());
        let before = f.effect_counts();
        if let Some(thread) = &existing {
            assert_eq!(
                f.count("SELECT count(*) FROM channel_handoff_fences WHERE state='live'"),
                1
            );
            f.db()
                .execute(
                    "UPDATE threads SET name='renamed-after-publication' WHERE id=?1",
                    [thread],
                )
                .unwrap();
        }
        let frame = successful(f.run(&["retry", &reference], "'malformed current options"));
        if let Some(initial) = initial {
            assert_eq!(frame, initial);
        }
        if mode == "delivery" {
            assert_eq!(frame["delivery"]["outcome"], "staged");
            assert_eq!(f.effect_counts(), (0, 0, 1, 1));
        } else {
            assert_eq!(frame["handoff"]["outcome"], "started");
            f.assert_launch(
                "codex",
                if mode == "bootstrap" {
                    "w4:p3"
                } else {
                    "w4:p2"
                },
            );
            assert_eq!(
                f.effect_counts(),
                (usize::from(mode == "bootstrap"), 1, 1, 1)
            );
        }
        if let Some(thread) = existing {
            assert_eq!(frame["handoff"]["thread"], thread);
        }
        let after = f.effect_counts();
        assert!(after.0 >= before.0 && after.1 >= before.1);
        let report_key = if mode == "delivery" {
            "delivery"
        } else {
            "handoff"
        };
        f.assert_work(
            frame[report_key][if mode == "delivery" {
                "recipient"
            } else {
                "seat"
            }]
            .as_str()
            .unwrap(),
            frame[report_key]["thread"].as_str().unwrap(),
        );
        let terminal = f.run(&["retry", &reference], "different options");
        if mode == "legacy" {
            assert!(!terminal.status.success());
            assert!(
                String::from_utf8_lossy(&terminal.stderr).contains("intent reference not found")
            );
        } else {
            assert_eq!(successful(terminal), frame);
        }
        assert_eq!(f.effect_counts(), after);
        proxy.finish().unwrap();
        if let Some(out) = std::env::var_os("HT_HANDOFF_SMOKE_OUT") {
            let dir = f.evidence_dir(PathBuf::from(out));
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join("reply-loss.json"),
                serde_json::to_vec_pretty(
                    &json!({"loss":loss,"original":original,"before":before,"after":after}),
                )
                .unwrap(),
            )
            .unwrap();
        }
    }
}

// Lost reservation permission is not permission to create; lost native result
// is not proof of noncreation. Neither boundary may allocate another attempt.
#[test]
fn public_creation_reservation_and_host_reply_loss_never_resubmit() {
    for reservation in [true, false] {
        let f = Fixture::new();
        f.setup("codex");
        let mut proxy = reservation.then(|| ReplyLoss::new(&f, "reserve_bootstrap_attempt", None));
        if !reservation {
            f.host.state.lock().unwrap().lose = Some("tab.create");
        }
        let binary = f.iso.path("codex");
        let args = f.new_thread_args("codex", binary.to_str().unwrap());
        let out = f.run(&args, OPTIONS);
        assert!(!out.status.success());
        if let Some(proxy) = &proxy {
            println!("reservation reply loss: {}", proxy.evidence());
        } else {
            unknown(&out);
        }
        let reference = f.reference();
        let original = f.original_bytes();
        let effects = f.effect_counts();
        assert_eq!(effects, (usize::from(!reservation), 0, 0, 0));
        assert_eq!(f.count("SELECT count(*) FROM bootstrap_attempts"), 1);
        assert_eq!(f.count("SELECT count(*) FROM bootstrap_attempts WHERE state IN ('possible_creation','outcome_unknown')"),1);
        assert_eq!(
            f.count("SELECT count(*) FROM bootstrap_handoffs WHERE state='possible_creation'"),
            1
        );
        assert_eq!(f.count("SELECT count(*) FROM bootstrap_reports"), 0);
        let retry = f.run(&["retry", &reference], "'malformed");
        unknown(&retry);
        assert_eq!(f.effect_counts(), effects);
        assert_eq!(f.count("SELECT count(*) FROM bootstrap_attempts"), 1);
        for (path, bytes) in original
            .iter()
            .filter(|(p, _)| p.extension().is_some_and(|s| s == "intent"))
        {
            assert_eq!(fs::read(path).unwrap(), *bytes);
        }
        if let Some(proxy) = &mut proxy {
            proxy.finish().unwrap();
        }
    }
}

// A native start may have occurred even though its response vanished. Retry
// cannot reset the persisted possible-start gate, start again, or invent success.
#[test]
fn public_native_start_reply_loss_never_starts_again() {
    let f = Fixture::new();
    f.setup("claude");
    f.host.state.lock().unwrap().lose = Some("agent.start");
    let binary = f.iso.path("claude");
    let args = f.new_thread_args("claude", binary.to_str().unwrap());
    let out = f.run(&args, OPTIONS);
    unknown(&out);
    let reference = f.reference();
    assert_eq!(f.effect_counts(), (1, 1, 1, 1));
    assert_eq!(
        f.count("SELECT count(*) FROM bootstrap_handoffs WHERE state='attached'"),
        1
    );
    assert_eq!(
        f.count("SELECT count(*) FROM channel_handoff_fences WHERE state='live'"),
        1
    );
    assert_eq!(f.count("SELECT count(*) FROM bootstrap_reports"), 0);
    let retry = f.run(&["retry", &reference], "'malformed");
    unknown(&retry);
    f.assert_launch("claude", "w4:p3");
    assert_eq!(f.effect_counts(), (1, 1, 1, 1));
    assert_eq!(f.count("SELECT count(*) FROM bootstrap_reports"), 0);
    let progress: Vec<_> = f
        .original_bytes()
        .into_iter()
        .filter(|(p, _)| p.extension().is_some_and(|s| s == "progress"))
        .collect();
    assert!(!progress.is_empty());
}

// A copied UUID in a foreign selected state/socket root cannot authorize the
// original compound or historical presentation/cleanup, even after completion.
#[test]
fn public_copied_uuid_namespace_refuses_before_effects() {
    let original = Fixture::new();
    let frame = successful(original.run(
        &[
            "handoff",
            "--existing",
            "--seat",
            "recipient",
            "--new-thread",
            "--",
            BODY,
        ],
        "'ignored",
    ));
    let saved = original.original_bytes();
    assert!(!saved.is_empty());
    let foreign = Fixture::with_instance(Some(original.instance));
    assert_eq!(foreign.instance, original.instance);
    fs::create_dir_all(foreign.journal()).unwrap();
    fs::set_permissions(foreign.journal(), fs::Permissions::from_mode(0o700)).unwrap();
    for (path, bytes) in &saved {
        fs::write(foreign.journal().join(path.file_name().unwrap()), bytes).unwrap();
        fs::set_permissions(
            foreign.journal().join(path.file_name().unwrap()),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
    }
    for metadata in ["journal-format", "next-ordinal"] {
        fs::copy(
            original.journal().join(metadata),
            foreign.journal().join(metadata),
        )
        .unwrap();
    }
    let copied = foreign.original_bytes();
    let out = foreign.run(
        &["retry", frame["delivery"]["recovery_ref"].as_str().unwrap()],
        "'malformed",
    );
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("namespace"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(foreign.effect_counts(), (0, 0, 0, 0));
    assert_eq!(
        foreign.count("SELECT count(*) FROM channel_handoff_fences"),
        0
    );
    assert_eq!(foreign.original_bytes(), copied);
    assert_eq!(original.original_bytes(), saved);
    assert_eq!(original.effect_counts(), (0, 0, 1, 1));
}

// Removing fresh composed-argv validation must expose topology/staging before refusal.
fn final_sdd_args(f: &Fixture, kind: &str, argv: &[String]) -> Vec<String> {
    let mut args = vec![
        "handoff".into(),
        "--new-tab".into(),
        "Peer".into(),
        "--new-thread".into(),
        "--kind".into(),
        kind.into(),
        "--harness-binary".into(),
        f.iso.path(kind).to_str().unwrap().into(),
    ];
    args.extend(argv.iter().map(|arg| format!("--agent-arg={arg}")));
    args.extend(["--".into(), BODY.into()]);
    args
}
#[test]
fn final_sdd_actual_native_count_refuses_before_publication_or_effects() {
    for kind in ["claude", "codex"] {
        let f = Fixture::new();
        f.setup(kind);
        let args = final_sdd_args(&f, kind, &vec!["--config=repeat=true".into(); 64]);
        let out = f.run(&args.iter().map(String::as_str).collect::<Vec<_>>(), "");
        assert!(!out.status.success());
        assert!(
            out.stdout.is_empty(),
            "prepublication refusal output: {}",
            String::from_utf8_lossy(&out.stdout)
        );
        assert_eq!(
            f.effect_counts(),
            (0, 0, 0, 0),
            "actual composed65 native refusal came after effects: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(f.count("SELECT count(*) FROM bootstrap_handoffs"), 0);
        assert!(f.original_bytes().is_empty());
    }
}
#[test]
fn final_sdd_actual_native_maximum_completes_and_historical_retry_never_relaunches() {
    for kind in ["claude", "codex"] {
        let f = Fixture::new();
        f.setup(kind);
        let frozen = vec!["--config=repeat=true".into(); 63];
        let args = final_sdd_args(&f, kind, &frozen);
        let first = successful(f.run(&args.iter().map(String::as_str).collect::<Vec<_>>(), ""));
        let starts = f.host.requests("agent.start");
        let actual: Vec<String> =
            serde_json::from_value(starts[0]["params"]["args"].clone()).unwrap();
        assert_eq!(actual.len(), 64);
        assert_eq!(&actual[..63], frozen);
        assert!(actual[63].contains(f.context.state_dir.to_str().unwrap()));
        assert!(actual[63].contains(f.context.host_endpoint.to_str().unwrap()));
        assert!(actual[63].len() <= 4096);
        let reference = first["handoff"]["recovery_ref"].as_str().unwrap();
        let retry = successful(f.run(&["retry", reference], "'malformed"));
        assert_eq!(retry, first);
        assert_eq!(f.effect_counts(), (1, 1, 1, 1));
        assert_eq!(f.count("SELECT count(*) FROM bootstrap_reports"), 1);
    }
}
fn final_sdd_pending(f: &Fixture, out: &Output, phase: &str) -> Value {
    assert!(!out.status.success());
    let frame: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "missing admitted pinned report: {e}; stdout={} stderr={}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    });
    let report = &frame["bootstrap"];
    assert_eq!(report["phase"], phase);
    assert_eq!(
        report["namespace"]["state_dir"],
        f.context.state_dir.to_str().unwrap()
    );
    assert_eq!(
        report["namespace"]["host_endpoint"],
        f.context.host_endpoint.to_str().unwrap()
    );
    assert_eq!(report["recovery_ref"], f.reference());
    assert_eq!(report["attempt"], 1);
    for key in ["retry_argv", "inspect_argv"] {
        let argv: Vec<String> = serde_json::from_value(report[key].clone()).unwrap();
        assert!(argv.contains(&f.context.state_dir.to_string_lossy().into_owned()));
        assert!(argv.contains(&f.context.host_endpoint.to_string_lossy().into_owned()));
    }
    report.clone()
}
#[test]
fn final_sdd_public_creation_unknown_reports_pinned_attempt_and_conditional_human_recovery() {
    let f = Fixture::new();
    f.setup("codex");
    f.host.state.lock().unwrap().lose = Some("tab.create");
    let args = final_sdd_args(&f, "codex", &["--config=frozen=true".into()]);
    let out = f.run(&args.iter().map(String::as_str).collect::<Vec<_>>(), "");
    let report = final_sdd_pending(&f, &out, "creation");
    assert_eq!(report["outcome"], "creation_unknown");
    assert!(report["pane"].is_null());
    assert!(report["manual_launch_after_confirming_no_start_argv"].is_null());
    let recovery = &report["conditional_human_recovery"];
    for key in ["created_pane_argv", "not_created_argv", "cancel_argv"] {
        let argv: Vec<String> = serde_json::from_value(recovery[key].clone()).unwrap();
        assert_eq!(argv[1], "human");
        assert!(argv.windows(2).any(|w| w == ["--attempt", "1"]));
        assert!(argv.contains(&f.reference()));
        let mut argv = argv;
        for word in &mut argv {
            if word == "REPLACE_WITH_EXACT_INSPECTED_PANE" {
                *word = "w4:p3".into();
            }
        }
        let parsed = herdr_threads::cli::commands::parse_argv(argv).unwrap();
        assert!(matches!(
            parsed.action,
            herdr_threads::cli::commands::CliAction::TopologyRecover(_)
        ));
    }
    let text = f.run_format(&["retry", &f.reference()], "'malformed", false);
    unknown(&text);
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(text.contains("phase: creation"));
    assert!(text.contains("outcome: creation_unknown"));
    assert!(text.contains("created_pane_argv: herdr-threads human --state-dir"));
    assert!(text.contains(f.context.state_dir.to_str().unwrap()));
    assert!(text.contains(f.context.host_endpoint.to_str().unwrap()));
    let retry = f.run(&["retry", &f.reference()], "'malformed");
    let second = final_sdd_pending(&f, &retry, "creation");
    assert_eq!(report, second);
    assert_eq!(f.effect_counts(), (1, 0, 0, 0));
}
#[test]
fn final_sdd_public_possible_start_reports_known_committed_work_without_relaunch() {
    let f = Fixture::new();
    f.setup("claude");
    f.host.state.lock().unwrap().lose = Some("agent.start");
    let args = final_sdd_args(&f, "claude", &["--config=frozen=true".into()]);
    let out = f.run(&args.iter().map(String::as_str).collect::<Vec<_>>(), "");
    let report = final_sdd_pending(&f, &out, "launch");
    assert_eq!(report["outcome"], "possible_start");
    assert_eq!(report["pane"], "w4:p3");
    assert!(report["thread"].is_string());
    assert!(!report["invitation"].is_null());
    assert!(!report["message"].is_null());
    assert!(report["conditional_human_recovery"].is_null());
    let manual: Vec<String> =
        serde_json::from_value(report["manual_launch_after_confirming_no_start_argv"].clone())
            .unwrap();
    assert_eq!(
        &manual[..3],
        [
            "env",
            "HERDR_THREADS_CODEX_OPTS=",
            "HERDR_THREADS_CLAUDE_OPTS="
        ]
    );
    assert!(manual.contains(&"--config=frozen=true".into()));
    let retry = f.run(&["retry", &f.reference()], "'malformed");
    final_sdd_pending(&f, &retry, "launch");
    let text = f.run_format(&["retry", &f.reference()], "'malformed", false);
    unknown(&text);
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(text.contains("phase: launch"));
    assert!(text.contains("outcome: possible_start"));
    assert!(text.contains("manual_launch_after_confirming_no_start_argv: env HERDR_THREADS_CODEX_OPTS= HERDR_THREADS_CLAUDE_OPTS="));
    assert!(text.contains("--config=frozen=true"));
    assert!(text.contains("seat_inspect_argv:"));
    assert!(text.contains("inspect_argv:"));
    assert_eq!(f.effect_counts(), (1, 1, 1, 1));
}

#[test]
fn final_sdd_actual_native_byte_and_line_limits_refuse_before_effects() {
    for argv in [
        vec![format!("--config={}", "x".repeat(3900)); 9],
        vec!["--config=line\nbreak".into()],
        vec!["--config=line\rbreak".into()],
        vec!["--config=single".to_owned() + &"x".repeat(4096)],
    ] {
        let f = Fixture::new();
        f.setup("claude");
        let args = final_sdd_args(&f, "claude", &argv);
        let out = f.run(&args.iter().map(String::as_str).collect::<Vec<_>>(), "");
        assert!(!out.status.success());
        assert!(out.stdout.is_empty());
        assert_eq!(f.effect_counts(), (0, 0, 0, 0));
        assert!(f.original_bytes().is_empty());
    }
    // TAB is allowed by the actual native contract and must remain byte-exact.
    let f = Fixture::new();
    f.setup("claude");
    let frozen = vec!["--config=allowed\tbyte".into()];
    let args = final_sdd_args(&f, "claude", &frozen);
    successful(f.run(&args.iter().map(String::as_str).collect::<Vec<_>>(), ""));
    assert_eq!(
        f.host.requests("agent.start")[0]["params"]["args"][0],
        frozen[0]
    );
    // A captured empty tools value is legal and reaches the actual native boundary.
    let f = Fixture::new();
    f.setup("claude");
    let frozen = vec!["--tools".into(), "".into()];
    let args = final_sdd_args(&f, "claude", &frozen);
    successful(f.run(&args.iter().map(String::as_str).collect::<Vec<_>>(), ""));
    let actual: Vec<String> =
        serde_json::from_value(f.host.requests("agent.start")[0]["params"]["args"].clone())
            .unwrap();
    assert_eq!(&actual[..2], frozen);
    assert_eq!(f.effect_counts(), (1, 1, 1, 1));
}

#[test]
fn final_sdd_public_staging_loss_reports_actual_create_phase_and_retries_only_saved_work() {
    let f = Fixture::new();
    f.setup("codex");
    let mut proxy = ReplyLoss::new(&f, "create_thread", None);
    let args = final_sdd_args(&f, "codex", &["--config=frozen=true".into()]);
    let out = f.run(&args.iter().map(String::as_str).collect::<Vec<_>>(), "");
    let report = final_sdd_pending(&f, &out, "create");
    assert_eq!(report["outcome"], "pending");
    assert_eq!(report["pane"], "w4:p3");
    assert!(report["thread"].is_null());
    assert!(report["manual_launch_after_confirming_no_start_argv"].is_null());
    assert_eq!(f.effect_counts(), (1, 0, 0, 0));
    let reference = report["recovery_ref"].as_str().unwrap();
    let original = f.original_bytes();
    proxy.finish().unwrap();
    let success = successful(f.run(&["retry", reference], "'malformed"));
    assert_eq!(success["handoff"]["outcome"], "started");
    successful(f.run(&["retry", reference], "'malformed"));
    assert_eq!(f.effect_counts(), (1, 1, 1, 1));
    assert_eq!(f.count("SELECT count(*) FROM threads"), 1);
    assert!(
        original
            .iter()
            .any(|(p, _)| p.extension().is_some_and(|v| v == "intent"))
    );
}

#[test]
fn final_sdd_generated_pinned_prompt_limit_refuses_before_publication_or_effects() {
    let f = Fixture::with_layout(None, true);
    // No installed-hook claim: actual preparation rejects this argument geometry
    // before it could inspect configuration or publish a bootstrap.
    let args = final_sdd_args(&f, "claude", &[]);
    let out = f.run(&args.iter().map(String::as_str).collect::<Vec<_>>(), "");
    assert!(
        !out.status.success(),
        "expanded pinned prompt must exceed retained report cap"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("retained report limit"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.stdout.is_empty());
    assert_eq!(f.effect_counts(), (0, 0, 0, 0));
    assert_eq!(f.count("SELECT count(*) FROM bootstrap_handoffs"), 0);
    assert!(f.original_bytes().is_empty());
}

// The actual ordinary CLI publishes the original, then a witnessed host read
// fails before the inner writer. The process must still present the durable
// reference with honest unknown canonical status, and the same route completes.
#[test]
fn pending_public_postpublication_witnessed_read_failure_reports() {
    for json in [true, false] {
        let f = Fixture::new();
        f.setup("codex");
        f.host.state.lock().unwrap().lose_after_publication = Some(("pane.get", f.journal()));
        let binary = f.iso.path("codex");
        let args = f.new_thread_args("codex", binary.to_str().unwrap());
        let out = f.run_format(&args, OPTIONS, json);
        assert!(!out.status.success());
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(!stderr.is_empty(), "generic phase error retained");
        let lost = f.host.state.lock().unwrap().lost_after_publication.clone();
        assert_eq!(lost.len(), 1, "witnessed post-publication read was lost");
        let reference = f.reference();
        let original = f.original_bytes();
        assert_eq!(f.effect_counts(), (0, 0, 0, 0));
        assert_eq!(f.count("SELECT count(*) FROM bootstrap_handoffs"), 0);
        if json {
            let frame: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
                panic!(
                    "missing post-publication report: {e}; stdout={} stderr={stderr}",
                    String::from_utf8_lossy(&out.stdout)
                )
            });
            let report = &frame["bootstrap"];
            assert_eq!(report["failed"], true);
            assert_eq!(report["phase"], "creation");
            assert_eq!(report["outcome"], "pending");
            assert_eq!(report["recovery_ref"], reference);
            assert_eq!(
                report["namespace"]["state_dir"],
                f.context.state_dir.to_str().unwrap()
            );
            assert_eq!(
                report["namespace"]["host_endpoint"],
                f.context.host_endpoint.to_str().unwrap()
            );
            // No canonical status was observed: unknown, never invented.
            assert!(report["attempt"].is_null());
            assert!(report["state"].is_null());
            assert_eq!(report["status_unknown"], true);
            assert_eq!(report["status_is_last_observed"], false);
            for absent in [
                "creation",
                "pane",
                "seat",
                "thread",
                "conditional_human_recovery",
            ] {
                assert!(report[absent].is_null(), "fabricated {absent}");
            }
            let retry: Vec<String> = serde_json::from_value(report["retry_argv"].clone()).unwrap();
            assert_eq!(retry[1], "HERDR_PANE_ID=w4:p1");
            assert!(retry.contains(&f.context.state_dir.to_string_lossy().into_owned()));
            assert!(retry.contains(&f.context.host_endpoint.to_string_lossy().into_owned()));
            assert_eq!(
                retry[retry.len() - 2..],
                ["retry".to_owned(), reference.clone()]
            );
        } else {
            let text = String::from_utf8(out.stdout.clone()).unwrap();
            assert!(text.contains("phase: creation"), "{text}");
            assert!(text.contains("status_unknown: true"), "{text}");
            assert!(text.contains(&format!("retry {reference}")), "{text}");
            assert!(text.contains(f.context.state_dir.to_str().unwrap()));
        }
        assert!(!String::from_utf8_lossy(&out.stdout).contains("work\n--agent-arg"));
        // Same public route, no fault: completes exactly once from the original.
        let done = successful(f.run_format(&["retry", &reference], "'malformed", true));
        assert_eq!(done["handoff"]["outcome"], "started");
        assert_eq!(f.effect_counts(), (1, 1, 1, 1));
        assert!(
            original
                .iter()
                .any(|(p, _)| p.extension().is_some_and(|v| v == "intent"))
        );
    }
}

#[test]
fn pending_public_prepublication_and_authority_failures_are_silent() {
    // Prepublication: the same witnessed read fails before any original exists.
    let f = Fixture::new();
    f.setup("codex");
    f.host.state.lock().unwrap().lose = Some("pane.get");
    let binary = f.iso.path("codex");
    let args = f.new_thread_args("codex", binary.to_str().unwrap());
    let out = f.run(&args, OPTIONS);
    assert!(!out.status.success());
    assert!(
        out.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(f.original_bytes().is_empty());
    assert_eq!(f.effect_counts(), (0, 0, 0, 0));
    // Authority: a different live caller cannot obtain the original's report.
    let f = Fixture::new();
    f.setup("codex");
    f.host.state.lock().unwrap().lose_after_publication = Some(("pane.get", f.journal()));
    let args = args_for(&f);
    let out = f.run(
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
        OPTIONS,
    );
    assert!(!out.status.success());
    let reference = f.reference();
    f.host.state.lock().unwrap().lose_after_publication = Some(("pane.get", f.journal()));
    let mut cmd = f.command();
    cmd.args([
        "--cooperative-seat",
        "recipient",
        "--cooperative-target",
        "w4:p2",
        "--cooperative-harness",
        "codex",
        "--cooperative-role",
        "top-level",
        "retry",
        &reference,
    ]);
    let out = f.capture(cmd);
    assert!(!out.status.success());
    assert!(
        out.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert_eq!(f.effect_counts(), (0, 0, 0, 0));
}
fn args_for(f: &Fixture) -> Vec<String> {
    let binary = f.iso.path("codex");
    f.new_thread_args("codex", binary.to_str().unwrap())
        .into_iter()
        .map(str::to_owned)
        .collect()
}
