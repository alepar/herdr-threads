//! ht-j16.10: the Claude mod delivery flows end to end, without a model.
//!
//! The REAL `herdr-threads watch` / `watch ack` executable plays against an
//! elected daemon that runs in this process (so the sweep can inject a clock:
//! `ShiftedClock`, the system clock plus an offset the test advances) and a
//! private stand-in Herdr endpoint that records every host call. A scripted
//! driver plays the mod's side (read the stdout lines, run `watch ack`). The
//! seat's Claude binding comes from the installed SessionStart hook run in
//! the fake pane, so it is `cooperative_top_level`, harness `claude`, native
//! session `<S>` exactly as in a live session. `scripts/test-claude-mod`
//! (ht-j16.6) covers the JS mod itself; nothing here needs `claude`.
//!
//! Native wake is observed as `agent.prompt` calls the stand-in receives.
//! Grace (30 s) and stall (10 min) are crossed by advancing the clock, never
//! by waiting; the mod-channel worker notices on its next one second tick.

use super::sweep::{Scratch, agent_pane, host_reply, pane};
use herdr_threads::{
    app::{LaneProbe, SystemClock, run_elected_probed},
    cli::hook::installed_argv,
    daemon::paths::{InstancePaths, RuntimeContext},
    harness::{claude::setup::plan_claude, context::Harness},
    host::native::NativeCli,
    ports::HostPort,
    protocol::time::{Cancellation, Clock, MonoInstant, UtcMillis},
    service::{config::ServiceConfig, kicks::Lane},
    test_support::spawn::{self, OwnedChild, SpawnOwned},
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    fs,
    io::{BufRead, BufReader, Read, Write},
    os::unix::{
        fs::{DirBuilderExt, MetadataExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");
const PANE_A: &str = "w1:p1";
const PANE_B: &str = "w1:p2";
const SESSION_B: &str = "SB";
const PANE_C: &str = "w1:p3";
const SESSION_C: &str = "SC";

/// The harness session running in `pane`: B's or C's registered one, or the stand-in agent's (A).
fn session_of(pane: &str) -> &'static str {
    match pane {
        PANE_B => SESSION_B,
        PANE_C => SESSION_C,
        _ => "SA",
    }
}
/// A step the sweep waits for; generous because the suite shares the machine.
const STEP: Duration = Duration::from_secs(20);
/// How long an absence is observed (a negative check) once its trigger has
/// happened: a native wake follows its trigger within tens of milliseconds, a
/// watch drains right after it connects, and a sweep the tests need runs
/// right after a kicked pass (otherwise on the worker's one second tick).
const QUIET: Duration = Duration::from_millis(1500);
const MINUTE_MS: u64 = 60_000;
/// The line both Claude hooks end their context with when something is
/// pending (the attention digest); every other part of the hook text is static.
const DIGEST_MARK: &str = "attention digest";

// ---------------------------------------------------------------- clock

/// The system clock plus an offset the test advances. Both domains shift
/// together; the daemon's lanes and the mod-channel worker read it.
struct ShiftedClock {
    base: SystemClock,
    shift_ms: AtomicU64,
    frozen_utc: Mutex<Option<UtcMillis>>,
}
impl ShiftedClock {
    fn new() -> Self {
        Self {
            base: SystemClock::new(),
            shift_ms: AtomicU64::new(0),
            frozen_utc: Mutex::new(None),
        }
    }
    fn freeze_utc(&self) {
        let now = self.utc_now();
        *self.frozen_utc.lock().unwrap() = Some(now);
    }
    fn advance(&self, millis: u64) {
        self.shift_ms.fetch_add(millis, Ordering::SeqCst);
    }
}
impl Clock for ShiftedClock {
    fn utc_now(&self) -> UtcMillis {
        if let Some(now) = *self.frozen_utc.lock().unwrap() {
            return now;
        }
        UtcMillis(
            self.base
                .utc_now()
                .0
                .saturating_add(self.shift_ms.load(Ordering::SeqCst) as i64),
        )
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(
            self.base
                .monotonic_now()
                .0
                .saturating_add(self.shift_ms.load(Ordering::SeqCst)),
        )
    }
}

fn wait_until<T>(what: &str, timeout: Duration, mut probe: impl FnMut() -> Option<T>) -> T {
    let until = Instant::now() + timeout;
    loop {
        if let Some(found) = probe() {
            return found;
        }
        assert!(Instant::now() < until, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

// ------------------------------------------------------------- stand-in Herdr

type Calls = Arc<Mutex<Vec<(String, Value)>>>;

/// Serving threads and, while each still serves, a handle on its connection.
/// A thread removes its own handle when it finishes (so the connection closes
/// exactly as before); drop shuts the ones still serving (a pending read
/// returns at once) and joins every thread.
#[derive(Default)]
struct ServingState {
    next: u64,
    open: std::collections::HashMap<u64, std::os::unix::net::UnixStream>,
    threads: Vec<JoinHandle<()>>,
}
type Serving = Arc<Mutex<ServingState>>;

/// The private Herdr endpoint: serves the scripted (mutable) panes like the
/// sweep's, answers `agent.prompt` as delivered, and records every call.
/// Each connection is answered on its own thread as soon as it is accepted
/// (a blocking accept, no polling), like a real Herdr server: concurrent
/// hooks and daemon reads never queue behind one another here.
struct Host {
    stop: Arc<AtomicBool>,
    socket: PathBuf,
    worker: Option<JoinHandle<()>>,
    serving: Serving,
    panes: Arc<Mutex<Vec<Value>>>,
    calls: Calls,
}
impl Host {
    fn start(socket: &std::path::Path, panes: Vec<Value>) -> Self {
        let listener = UnixListener::bind(socket).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let panes = Arc::new(Mutex::new(panes));
        let calls: Calls = Arc::default();
        let serving: Serving = Arc::default();
        let (stopped, scripted, log, live) =
            (stop.clone(), panes.clone(), calls.clone(), serving.clone());
        let worker = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if stopped.load(Ordering::SeqCst) {
                    return;
                }
                let Ok(stream) = stream else {
                    // Do not spin on a persistent accept error.
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                };
                let Ok(handle) = stream.try_clone() else {
                    continue;
                };
                let (scripted, log, mine) = (scripted.clone(), log.clone(), live.clone());
                let mut state = live.lock().unwrap();
                let id = state.next;
                state.next += 1;
                state.open.insert(id, handle);
                state.threads.retain(|thread| !thread.is_finished());
                state.threads.push(std::thread::spawn(move || {
                    Self::serve(stream, &scripted, &log);
                    mine.lock().unwrap().open.remove(&id);
                }));
            }
        });
        Self {
            stop,
            socket: socket.to_owned(),
            worker: Some(worker),
            serving,
            panes,
            calls,
        }
    }
    fn serve(
        mut stream: std::os::unix::net::UnixStream,
        scripted: &Mutex<Vec<Value>>,
        log: &Calls,
    ) {
        // macOS rejects this once the peer has hung up.
        if stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .is_err()
        {
            return;
        }
        let mut line = String::new();
        if BufReader::new(&mut stream).read_line(&mut line).is_err() || line.is_empty() {
            return;
        }
        let Ok(request) = serde_json::from_str::<Value>(&line) else {
            return;
        };
        let method = request["method"].as_str().unwrap_or_default().to_owned();
        let panes = {
            // One lock for the record and the read: a call is recorded
            // against the panes it was answered from.
            let mut calls = log.lock().unwrap();
            calls.push((method.clone(), request["params"].clone()));
            let panes = scripted.lock().unwrap().clone();
            drop(calls);
            panes
        };
        let reply = if method == "agent.prompt" {
            match panes
                .iter()
                .find(|pane| pane["pane_id"] == request["params"]["target"])
            {
                Some(pane) if pane["agent"].is_string() => json!({"id":request["id"],
                    "result":{"type":"agent_prompted","agent":{
                        "agent":pane["agent"],"agent_status":"working",
                        "pane_id":pane["pane_id"],"terminal_id":pane["terminal_id"]}}}),
                _ => json!({"id":request["id"],
                    "error":{"code":"agent_not_found","message":"no agent in pane"}}),
            }
        } else {
            host_reply(&panes, &request)
        };
        let _ = writeln!(stream, "{reply}");
    }
    fn set_panes(&self, panes: Vec<Value>) {
        *self.panes.lock().unwrap() = panes;
    }
    /// Number of calls recorded so far (a mark for `prompts_to_since`).
    fn mark(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
    /// `agent.prompt` calls for `pane` after `mark`: native wake attempts.
    fn prompts_since(&self, mark: usize, pane: &str) -> usize {
        self.calls.lock().unwrap()[mark..]
            .iter()
            .filter(|(method, params)| method == "agent.prompt" && params["target"] == pane)
            .count()
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the blocking accept so the worker sees the stop.
        let _ = std::os::unix::net::UnixStream::connect(&self.socket);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        // No serving thread outlives the host: shut each connection (a read
        // still waiting returns at once) and join its thread.
        let threads = {
            let mut state = self.serving.lock().unwrap();
            for stream in state.open.values() {
                let _ = stream.shutdown(std::net::Shutdown::Both);
            }
            std::mem::take(&mut state.threads)
        };
        for thread in threads {
            let _ = thread.join();
        }
    }
}

// ------------------------------------------------------------------- output

struct Out {
    code: i32,
    value: Value,
    stdout: String,
    stderr: String,
}
impl Out {
    fn data(&self, what: &str) -> Value {
        assert_eq!(self.code, 0, "{what}: {}{}", self.stdout, self.stderr);
        self.value["result"]["data"].clone()
    }
    fn text(&self, what: &str) -> String {
        self.data(what).as_str().unwrap().to_owned()
    }
}

fn collect(output: std::process::Output) -> Out {
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    Out {
        code: output.status.code().unwrap_or(-1),
        value: serde_json::from_str(&stdout).unwrap_or(Value::Null),
        stdout,
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn diagnostic_text(value: &str, limit: usize) -> Value {
    let mut end = value.len().min(limit);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    json!({"bytes": value.len(), "prefix": &value[..end], "truncated": end < value.len()})
}

fn session_start(session: &str, source: &str) -> String {
    format!(
        r#"{{"session_id":"{session}","transcript_path":"/tmp/t.jsonl","cwd":"/tmp","hook_event_name":"SessionStart","source":"{source}"}}"#
    )
}

fn pre_tool_use(session: &str) -> String {
    format!(
        r#"{{"session_id":"{session}","transcript_path":"/tmp/t.jsonl","cwd":"/tmp","permission_mode":"default","hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{{"command":"ls"}},"tool_use_id":"toolu_{}"}}"#,
        uuid::Uuid::new_v4().simple()
    )
}

/// A pane running a Claude agent for `session`. Herdr's foreground process of the pane is that
/// session's harness process, the fingerprint of its binding: the composer wake types only to
/// the binding whose process is the pane's foreground process (spec §3 Wake).
fn claude(id: &str, terminal: &str, session: &str) -> Value {
    let mut pane = agent_pane(id, terminal, "claude", Some(session));
    pane["foreground_pid"] = json!(foreground_of(session));
    pane
}

/// The pid of the harness process of `session` (see `hook_in_process_of`).
fn foreground_of(session: &str) -> u32 {
    herdr_threads::test_support::isolation::stand_in_fingerprint("claude", session).pid
}

// ---------------------------------------------------------------------- rig

struct Daemon {
    stop: Cancellation,
    thread: JoinHandle<std::io::Result<bool>>,
}

/// The world: elected daemon (injected clock), stand-in Herdr, two Claude
/// seats registered through the installed hook, one joined thread.
struct Rig {
    root: PathBuf,
    state: PathBuf,
    socket: PathBuf,
    paths: InstancePaths,
    clock: Arc<ShiftedClock>,
    host: Host,
    claude_hooks: Value,
    setup_hook: Value,
    panics: Arc<Mutex<Vec<String>>>,
    daemon: Option<Daemon>,
    config: ServiceConfig,
    /// The running daemon's lanes (its wake lane's finished passes).
    probe: LaneProbe,
    a: String,
    b: String,
    thread: String,
    _guard: MutexGuard<'static, ()>,
    _scratch: Scratch,
}

impl Rig {
    fn new() -> Self {
        Self::with_settings(None)
    }

    fn activation() -> Self {
        Self::configured(None, ServiceConfig::default(), true)
    }

    /// `settings`: the content of the instance's `settings.json`, written
    /// before the daemon starts.
    fn with_settings(settings: Option<Value>) -> Self {
        Self::with_config(settings, ServiceConfig::default())
    }

    /// As `with_settings`, with the daemon started on `config` (a restart keeps it).
    fn with_config(settings: Option<Value>, config: ServiceConfig) -> Self {
        Self::configured(settings, config, false)
    }

    fn configured(
        settings: Option<Value>,
        config: ServiceConfig,
        require_activation: bool,
    ) -> Self {
        let guard = crate::ONE_DAEMON.lock().unwrap_or_else(|e| e.into_inner());
        let root = PathBuf::from(format!(
            "{}/htmd-{}",
            herdr_threads::test_support::SHORT_TMP,
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        ));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let scratch = Scratch(root.clone());
        let state = root.join("state");
        let socket = root.join("h.sock");
        let bin = root.join("bin");
        fs::create_dir_all(&bin).unwrap();
        for name in ["claude", "codex"] {
            let path = bin.join(name);
            fs::write(&path, "#!/bin/sh\nexit 99\n").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let argv = installed_argv(BIN, Some(state.to_str().unwrap()), None, Harness::Claude);
        let plan = plan_claude(b"{}", &argv).unwrap();
        let installed: Value = serde_json::from_slice(&plan.proposed_bytes).unwrap();
        let context = RuntimeContext::explicit(state.clone(), socket.clone(), None).unwrap();
        let paths = InstancePaths::resolve(&context).unwrap();
        paths.prepare_instance_dir().unwrap();
        if let Some(settings) = settings {
            let file = paths.instance_dir.join("settings.json");
            fs::write(&file, serde_json::to_vec(&settings).unwrap()).unwrap();
            fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
        }
        let host = Host::start(
            &socket,
            vec![pane(PANE_A, "term-a"), claude(PANE_B, "term-b", SESSION_B)],
        );
        let mut rig = Self {
            root,
            state,
            socket,
            paths,
            clock: Arc::new(ShiftedClock::new()),
            host,
            claude_hooks: installed["hooks"].clone(),
            setup_hook: Value::Null,
            panics: Arc::default(),
            daemon: None,
            config,
            probe: LaneProbe::default(),
            a: String::new(),
            b: String::new(),
            thread: String::new(),
            _guard: guard,
            _scratch: scratch,
        };
        rig.start_daemon();
        if require_activation {
            rig.require_advertised_presentation();
        }
        rig.a = rig.seat_resolve(PANE_A);
        rig.b = rig.seat_resolve(PANE_B);
        // A is the sender: a plain pane the native ladder never prompts.
        rig.cli(
            Some((&rig.a, PANE_A)),
            &["check-in", "--lifecycle-event", "a-start"],
        )
        .data("a check-in");
        let startup = rig.session_start_hook(PANE_B, SESSION_B, "startup");
        rig.setup_hook = json!({
            "code": startup.code,
            "stdout": diagnostic_text(&startup.stdout, 2048),
            "stderr": diagnostic_text(&startup.stderr, 2048),
        });
        rig.thread = rig
            .cli(
                Some((&rig.a, PANE_A)),
                &["thread", "create", "--topic", "mod delivery"],
            )
            .text("thread create");
        let mark = rig.host.mark();
        rig.cli(
            Some((&rig.a, PANE_A)),
            &["invite", &rig.thread, "--seat", &rig.b],
        )
        .data("invite");
        // Setup deliberately requires a native wake: ordinary invitations
        // are lazy by default, so only this bootstrap requests timed acceptance.
        // The invitation wakes idle B natively. Waiting for that prompt
        // before the accept makes the setup's wake deterministic (accepted
        // first, the wake may or may not still go out), so no quiet window
        // is needed to know that no setup wake is still on its way.
        rig.wait_prompt(mark, PANE_B);
        rig.cli(Some((&rig.b, PANE_B)), &["accept", &rig.thread])
            .data("accept");
        // The wake lane's pass that sent the prompt has finished (its
        // outcome is committed), so the clock jump below cuts no budget of
        // it short: once the lane is kicked, the next finished pass is that
        // one or a later one.
        let passes = rig.probe.registered_idle_events(Lane::Wakes);
        rig.probe.kick_registered(Lane::Wakes);
        wait_until("the wake lane to finish its pass", STEP, || {
            (rig.probe.registered_idle_events(Lane::Wakes) > passes).then_some(())
        });
        // The native ladder's 30 s spacing passes, so the next attention
        // wakes B at once. No watch exists yet, so no drain is in flight.
        rig.clock.advance(31_000);
        rig
    }

    fn start_daemon(&mut self) {
        assert!(self.daemon.is_none());
        let clock: Arc<dyn Clock> = self.clock.clone();
        let host: Arc<dyn HostPort> = Arc::new(NativeCli::new(self.socket.clone(), clock.clone()));
        let stop = Cancellation::default();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (paths, daemon_stop, config) = (self.paths.clone(), stop.clone(), self.config.clone());
        // The production composition (`run_elected`), except that the store
        // commits without a per-commit fsync: nothing here can observe the
        // difference (it only matters on power loss), and a shared loaded
        // disk otherwise dominates every step.
        let probe = LaneProbe::default();
        probe.relax_commit_durability();
        self.probe = probe.clone();
        let thread = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(run_elected_probed(
                    &paths,
                    clock,
                    daemon_stop,
                    config,
                    host,
                    probe,
                    move |descriptor| {
                        ready_tx
                            .send(descriptor.clone())
                            .map_err(std::io::Error::other)
                    },
                ))
        });
        ready_rx
            .recv_timeout(Duration::from_secs(20))
            .expect("the elected daemon did not publish its endpoint");
        // The elected daemon installs a quiet panic hook; restore visible failures.
        // `HT_MOD_DELIVERY_PANIC_LOG` names a file that also receives failures,
        // because the daemon points this process's stderr at daemon.log.
        let panics = self.panics.clone();
        std::panic::set_hook(Box::new(move |info| {
            let mut saved = panics.lock().unwrap_or_else(|error| error.into_inner());
            if saved.len() < 8 {
                let text = info.to_string();
                let mut end = text.len().min(8192);
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                saved.push(text[..end].to_owned());
            }
            drop(saved);
            eprintln!("{info}");
            if let Some(path) = std::env::var_os("HT_MOD_DELIVERY_PANIC_LOG")
                && let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path)
            {
                let _ = writeln!(file, "{info}");
            }
        }));
        self.daemon = Some(Daemon { stop, thread });
    }

    fn require_advertised_presentation(&self) {
        use herdr_threads::{
            daemon::ownership::{read_descriptor, read_existing_namespace},
            protocol::{
                commands::Command,
                results::CommandResult,
                wire::{PROTOCOL_VERSION, WireRequest, WireResponse},
            },
        };
        let descriptor = read_descriptor(
            &self.paths,
            read_existing_namespace(&self.paths).unwrap().unwrap(),
        )
        .unwrap();
        let request = WireRequest {
            version: PROTOCOL_VERSION,
            request_id: uuid::Uuid::new_v4().to_string(),
            expected_instance: descriptor.instance_uuid.to_string(),
            expected_boot: Some(descriptor.boot_id.to_string()),
            output: None,
            command: Command::Capabilities,
        };
        let mut socket = UnixStream::connect(&descriptor.endpoint).unwrap();
        socket.set_read_timeout(Some(STEP)).unwrap();
        activation_write(&mut socket, &serde_json::to_vec(&request).unwrap()).unwrap();
        let response: WireResponse =
            serde_json::from_slice(&activation_read(&mut socket).unwrap()).unwrap();
        assert!(response.correlates_to(&request, request.expected_boot.as_deref()));
        let Ok(CommandResult::Capabilities(list)) = response.result else {
            panic!("actual elected Capabilities route refused before activation setup");
        };
        assert!(
            list.capabilities.iter().any(|name| {
                name == herdr_threads::protocol::capabilities::DELIVERY_PRESENTATION_V2
            }),
            "actual elected Capabilities must advertise presentation V2 before activation setup"
        );
    }

    fn stop_daemon(&mut self) {
        if let Some(daemon) = self.daemon.take() {
            daemon.stop.cancel();
            let _ = daemon.thread.join();
        }
    }

    fn restart_daemon(&mut self) {
        self.stop_daemon();
        self.start_daemon();
    }

    fn path(&self) -> String {
        format!("{}:/usr/bin:/bin", self.root.join("bin").display())
    }

    /// A scrubbed, tagged `herdr-threads` invocation inside the rig. `pane`
    /// is the invoking shell's `HERDR_PANE_ID`.
    fn command(&self, pane: Option<&str>, args: &[&str]) -> std::process::Command {
        self.command_with(true, pane, args)
    }

    fn command_with(&self, json: bool, pane: Option<&str>, args: &[&str]) -> std::process::Command {
        let mut command = spawn::command(BIN);
        let mut words = Vec::new();
        if json {
            words.push("--json");
        }
        words.extend_from_slice(args);
        let args = &words[..];
        command
            .args(herdr_threads::test_support::isolation::routed_argv(
                &[
                    std::ffi::OsStr::new("--state-dir"),
                    AsRef::<std::ffi::OsStr>::as_ref(&&self.state),
                    std::ffi::OsStr::new("--host-endpoint"),
                    AsRef::<std::ffi::OsStr>::as_ref(&&self.socket),
                ],
                args,
            ))
            .env_remove("CLAUDECODE")
            .env_remove("HERDR_PLUGIN_STATE_DIR")
            .env_remove("HERDR_SOCKET_PATH")
            .env_remove("HERDR_PANE_ID")
            .env_remove("HERDR_BIN_PATH")
            .env_remove("HERDR_ENV")
            .env("PATH", self.path())
            .env("HOME", self.root.join("home"))
            .env("CODEX_HOME", self.root.join("codex-config"))
            .env("CLAUDE_CONFIG_DIR", self.root.join("claude-config"))
            .env("HERDR_THREADS_OFFLINE", "1")
            .env("NO_COLOR", "1");
        if let Some(pane) = pane {
            command
                .env("HERDR_ENV", "1")
                .env("HERDR_PANE_ID", pane)
                .env("HERDR_SOCKET_PATH", &self.socket);
            // A pane-routed command runs as the harness session in that pane (decision 15).
            herdr_threads::test_support::isolation::as_agent(
                &mut command,
                "claude",
                session_of(pane),
            );
        }
        command
    }

    /// `argv` exactly as given, with no herdr-threads flag added and an
    /// environment that names no Herdr instance and has no `herdr-threads`
    /// on PATH: only the invoking pane, as in a Claude session's shell.
    fn raw_command(&self, pane: &str, argv: &[String]) -> std::process::Command {
        let mut command = spawn::command(&argv[0]);
        command
            .args(&argv[1..])
            .env_remove("CLAUDECODE")
            .env_remove("HERDR_PLUGIN_STATE_DIR")
            .env_remove("HERDR_SOCKET_PATH")
            .env_remove("HERDR_SESSION")
            .env_remove("HERDR_BIN_PATH")
            .env_remove("XDG_STATE_HOME")
            .env("HERDR_ENV", "1")
            .env("HERDR_PANE_ID", pane)
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", self.root.join("home"))
            .env("CLAUDE_CONFIG_DIR", self.root.join("claude-config"))
            .env("HERDR_THREADS_OFFLINE", "1")
            .env("NO_COLOR", "1");
        herdr_threads::test_support::isolation::as_agent(&mut command, "claude", session_of(pane));
        command
    }

    /// The argv `setup claude` writes into `LAUNCH`.
    fn launch(&self) -> Vec<String> {
        [
            BIN,
            "--state-dir",
            self.state.to_str().unwrap(),
            "--host-endpoint",
            self.socket.to_str().unwrap(),
        ]
        .map(String::from)
        .to_vec()
    }

    /// The mod's `watch` child as the setup-written launch runs it, in the
    /// session-shell environment (see [`Rig::raw_command`]), for B.
    fn raw_watch(&self, session: &str) -> Watch {
        let mut argv = self.launch();
        argv.extend(["watch", "--harness", "claude", "--session", session].map(String::from));
        for _ in 0..80 {
            let mut watch = Watch::spawn(self.raw_command(PANE_B, &argv));
            let first = watch.peek_first(STEP);
            let retry = first
                .as_ref()
                .is_some_and(|line| line["kind"] == "status" && line["reason"] == "no_binding");
            if !retry {
                return watch;
            }
            drop(watch);
            std::thread::sleep(Duration::from_millis(150));
        }
        panic!("the SessionStart check-in never committed for {PANE_B}");
    }

    /// The truncation marker's steps as printed, each parsed into an argv.
    fn marker_steps(&self, body: &str) -> Vec<Vec<String>> {
        let instruction = body.rsplit("…truncated; run ").next().unwrap();
        instruction
            .split(", then ")
            .map(|step| {
                let argv = shlex::split(step).unwrap_or_else(|| panic!("unparsable step {step}"));
                assert_eq!(argv[0], "herdr-threads", "{step}");
                argv
            })
            .collect()
    }

    /// Runs one marker step verbatim in the session-shell environment; only
    /// the program name is swapped for the test binary (the raw PATH has no
    /// `herdr-threads`).
    fn run_marker_step(&self, argv: &[String]) -> Out {
        let mut run = argv.to_vec();
        run[0] = BIN.to_owned();
        collect(
            self.raw_command(PANE_B, &run)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn_owned()
                .unwrap()
                .wait_with_output()
                .unwrap(),
        )
    }

    /// `caller`: (seat, pane) as the cooperative stand-in flags.
    fn cli(&self, caller: Option<(&str, &str)>, args: &[&str]) -> Out {
        let mut all: Vec<&str> = Vec::new();
        if let Some((seat, pane)) = caller {
            all.extend([
                "--cooperative-seat",
                seat,
                "--cooperative-target",
                pane,
                "--cooperative-harness",
                "claude",
                "--cooperative-role",
                "top-level",
            ]);
        }
        all.extend_from_slice(args);
        let mut command = self.command(None, &all);
        if let Some((_, pane)) = caller {
            herdr_threads::test_support::isolation::as_agent(
                &mut command,
                "claude",
                session_of(pane),
            );
        }
        collect(
            command
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn_owned()
                .unwrap()
                .wait_with_output()
                .unwrap(),
        )
    }

    /// B's text-mode CLI (no `--json`), as the cooperative stand-in caller.
    fn cli_text_b(&self, args: &[&str]) -> Out {
        self.cli_text_b_env(args, &[])
    }

    /// `cli_text_b` with extra environment (test-support overrides).
    fn cli_text_b_env(&self, args: &[&str], env: &[(&str, &str)]) -> Out {
        let mut all = vec![
            "--cooperative-seat",
            self.b.as_str(),
            "--cooperative-target",
            PANE_B,
            "--cooperative-harness",
            "claude",
            "--cooperative-role",
            "top-level",
        ];
        all.extend_from_slice(args);
        let mut command = self.command_with(false, None, &all);
        herdr_threads::test_support::isolation::as_agent(&mut command, "claude", SESSION_B);
        command.envs(env.iter().copied());
        collect(
            command
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn_owned()
                .unwrap()
                .wait_with_output()
                .unwrap(),
        )
    }

    fn seat_resolve(&self, pane: &str) -> String {
        self.cli(None, &["seat", "resolve", "--pane", pane])
            .text("seat resolve")
    }

    /// The installed hook, exactly as setup installs it.
    fn hook(&self, pane: &str, stdin: &str) -> Out {
        self.hook_in_process_of(pane, stdin, None)
    }

    /// `hook`, run by the harness process of `process_of` (default: the payload's own session):
    /// a `/clear` starts a new session inside the same process, so the new session's hook
    /// shares the old one's liveness fingerprint and supersedes its binding.
    fn hook_in_process_of(&self, pane: &str, stdin: &str, process_of: Option<&str>) -> Out {
        let payload: Value = serde_json::from_str(stdin).unwrap();
        let event = payload["hook_event_name"].as_str().unwrap();
        let command = self.claude_hooks[event][0]["hooks"][0]["command"]
            .as_str()
            .unwrap_or_else(|| panic!("no installed Claude registration for {event}"))
            .to_owned();
        let mut shell = spawn::command("/bin/sh");
        // The installed hook runs under its harness (the test process stands in for it).
        herdr_threads::test_support::isolation::as_agent(&mut shell, "claude", "hook-session");
        // Each session is a process of its own, as in a real run: hooks of several sessions
        // sharing one fingerprint would supersede one another's bindings, and a disconnected
        // binding loses its mod channel (spec §3 Wake).
        if let Some(session) = process_of.or(payload["session_id"].as_str()) {
            let stand_in =
                herdr_threads::test_support::isolation::stand_in_fingerprint("claude", session)
                    .pid
                    .to_string();
            shell
                .env("CLAUDE_PID", &stand_in)
                .env("HT_TEST_HARNESS_PID", &stand_in);
        }
        let mut child = shell
            .arg("-c")
            .arg(command)
            .env("HERDR_ENV", "1")
            .env("HERDR_PANE_ID", pane)
            .env("HERDR_SOCKET_PATH", &self.socket)
            .env("PATH", self.path())
            .env("HOME", self.root.join("home"))
            .env("CLAUDE_CONFIG_DIR", self.root.join("claude-config"))
            .env("HERDR_THREADS_OFFLINE", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn_owned()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        collect(child.wait_with_output().unwrap())
    }

    fn session_start_hook(&self, pane: &str, session: &str, source: &str) -> Out {
        let out = self.hook(pane, &session_start(session, source));
        assert_eq!(
            out.code, 0,
            "SessionStart hook: {}{}",
            out.stdout, out.stderr
        );
        out
    }

    /// A SessionStart of a new session started inside the process that ran `old_session`
    /// (`/clear`).
    fn session_start_hook_in_process_of(
        &self,
        pane: &str,
        session: &str,
        source: &str,
        old_session: &str,
    ) -> Out {
        let out = self.hook_in_process_of(pane, &session_start(session, source), Some(old_session));
        assert_eq!(
            out.code, 0,
            "SessionStart hook: {}{}",
            out.stdout, out.stderr
        );
        out
    }

    fn db(&self) -> rusqlite::Connection {
        let db = rusqlite::Connection::open_with_flags(
            &self.paths.database_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        db.busy_timeout(Duration::from_secs(5)).unwrap();
        db
    }

    /// The ordinary require-ACK message A sends B; returns its id.
    fn send_ordinary(&self, body: &str) -> String {
        self.cli(
            Some((&self.a, PANE_A)),
            &[
                "send",
                &self.thread,
                "--body",
                body,
                "--require-ack",
                &self.b,
            ],
        )
        .text("send")
    }

    /// A passive (lazy) message from A to B.
    fn send_lazy(&self, body: &str) -> String {
        self.cli(
            Some((&self.a, PANE_A)),
            &["send", &self.thread, "--body", body],
        )
        .text("send lazy")
    }

    /// `pending` until the logical receipt has a settled `receipt_state` row.
    fn receipt_state(&self, message: &str) -> String {
        use rusqlite::OptionalExtension;
        let db = self.db();
        for table in ["receipt_state", "receipts"] {
            let settled: Option<String> = db
                .query_row(
                    &format!(
                        "SELECT state FROM {table} WHERE message_id=?1 AND seat_id=?2 AND state<>'pending'"
                    ),
                    [message, &self.b],
                    |row| row.get(0),
                )
                .optional()
                .unwrap();
            if let Some(state) = settled {
                return state;
            }
        }
        "pending".to_owned()
    }

    fn ack_observation(&self, message: &str) -> Value {
        let db = self.db();
        for table in ["receipt_state", "receipts"] {
            let text: Option<String> = db
                .query_row(
                    &format!(
                        "SELECT ack_observation FROM {table} WHERE message_id=?1 AND seat_id=?2 AND ack_observation IS NOT NULL"
                    ),
                    [message, &self.b],
                    |row| row.get(0),
                )
                .ok();
            if let Some(text) = text {
                return serde_json::from_str(&text).unwrap();
            }
        }
        panic!("no ack observation for {message}");
    }

    /// Mod ack events recorded (one compact event per settling transaction).
    fn mod_ack_events(&self) -> i64 {
        self.db()
            .query_row(
                "SELECT count(*) FROM messages WHERE kind='info' AND event_key LIKE 'ack_mod:%'",
                [],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn lazy_state(&self, message: &str) -> String {
        self.db()
            .query_row(
                "SELECT state FROM lazy_recipients WHERE message_id=?1 AND seat_id=?2",
                [message, &self.b],
                |row| row.get(0),
            )
            .unwrap()
    }

    /// The `mod_channels` block the daemon reports through `doctor --json`.
    fn channels(&self) -> Value {
        let out = self.cli(None, &["doctor"]);
        out.value["doctor"]["harness_states"]["mod_channels"].clone()
    }

    /// The state of B's channel (`live`, `reconnect_grace`, `rebind_grace`),
    /// or `None` when the registry has no entry for B.
    fn channel_state(&self) -> Option<String> {
        // Channels are per binding: a seat can hold a superseded session's channel in grace
        // beside the new session's live one. The seat's state is the strongest of them.
        let channels = self.channels();
        let states: Vec<&str> = channels["channels"]
            .as_array()?
            .iter()
            .filter(|channel| channel["seat"] == self.b.as_str())
            .filter_map(|channel| channel["state"].as_str())
            .collect();
        states
            .iter()
            .find(|state| **state == "live")
            .or(states.first())
            .map(|state| (*state).to_owned())
    }

    fn wait_channel(&self, state: Option<&str>) {
        let what = format!("B's channel to be {state:?}");
        wait_until(&what, STEP, || {
            (self.channel_state().as_deref() == state).then_some(())
        });
    }

    /// Spawns the mod's `watch` child for B (pane `PANE_B`).
    fn watch(&self, session: &str) -> Watch {
        self.watch_with(PANE_B, session, &[])
    }

    /// The mod's `watch` child. Like the mod (spec D3, exit 2), a refusal
    /// `no_binding` (the SessionStart hook's check-in has not committed yet,
    /// which a loaded machine makes visible) is retried with a short backoff;
    /// every other first answer is returned for the test to judge.
    fn watch_with(&self, pane: &str, session: &str, env: &[(&str, &str)]) -> Watch {
        self.try_watch(pane, session, env, 80)
            .unwrap_or_else(|| panic!("the SessionStart check-in never committed for {pane}"))
    }

    fn try_watch(
        &self,
        pane: &str,
        session: &str,
        env: &[(&str, &str)],
        attempts: usize,
    ) -> Option<Watch> {
        for _ in 0..attempts {
            let mut command = self.command(
                Some(pane),
                &["watch", "--harness", "claude", "--session", session],
            );
            command.envs(env.iter().copied());
            let mut watch = Watch::spawn(command);
            let first = watch.peek_first(STEP);
            let retry = first
                .as_ref()
                .is_some_and(|line| line["kind"] == "status" && line["reason"] == "no_binding");
            if !retry {
                return Some(watch);
            }
            drop(watch);
            std::thread::sleep(Duration::from_millis(150));
        }
        None
    }

    /// `watch ack` as the mod runs it; returns (exit code, one value per line).
    fn ack(&self, session: &str, via: &str, ids: &[&str]) -> (i32, Vec<Value>) {
        self.ack_with(session, via, ids, &[])
    }

    fn ack_with(
        &self,
        session: &str,
        via: &str,
        ids: &[&str],
        env: &[(&str, &str)],
    ) -> (i32, Vec<Value>) {
        let mut args = vec!["watch", "ack", "--session", session, "--via", via];
        args.extend_from_slice(ids);
        let mut command = self.command(Some(PANE_B), &args);
        command.envs(env.iter().copied());
        let out = collect(
            command
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn_owned()
                .unwrap()
                .wait_with_output()
                .unwrap(),
        );
        let lines = out
            .stdout
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        (out.code, lines)
    }

    /// The result of one id from a `watch ack`.
    fn ack_result(&self, session: &str, via: &str, id: &str) -> Value {
        let (code, lines) = self.ack(session, via, &[id]);
        assert_eq!(code, 0, "watch ack {id}: {lines:?}");
        lines
            .into_iter()
            .find(|line| line["id"] == id)
            .unwrap_or_else(|| panic!("no result line for {id}"))
    }

    /// Advances the daemon clock by `millis`. A jump invalidates the read
    /// budgets of requests in flight, so the drains an earlier send triggered
    /// are given a moment to finish first.
    fn advance(&self, millis: u64) {
        std::thread::sleep(Duration::from_millis(400));
        self.clock.advance(millis);
    }

    /// Read-only bounded diagnostics for the shared pre-Watch bootstrap.
    /// Query errors are data, so diagnostics cannot replace the original panic.
    fn setup_wake_diagnostics(&self) -> Value {
        let db = match rusqlite::Connection::open_with_flags(
            &self.paths.database_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ) {
            Ok(db) => db,
            Err(error) => {
                return json!({"read_error": error.to_string(), "SessionStart": self.setup_hook});
            }
        };
        let _ = db.busy_timeout(Duration::from_millis(100));
        let mut result = serde_json::Map::new();
        result.insert("SessionStart".into(), self.setup_hook.clone());
        for (name, query) in [
            (
                "bootstrap_invitation",
                "SELECT id,thread_id,seat_id,state,confirmation,deadline_at FROM invitations WHERE seat_id=?1 ORDER BY ordinal DESC LIMIT 2",
            ),
            (
                "binding",
                "SELECT generation,target_generation,target_id,terminal_id,incarnation,host_boot,host_epoch,harness,native_session,observation_provenance,registered_at,ended_at FROM occupant_bindings WHERE seat_id=?1 ORDER BY ordinal DESC LIMIT 2",
            ),
            (
                "seat",
                "SELECT id,state,generation,target_generation,target_id,structural_terminal_id,structural_incarnation,structural_host_boot,structural_host_epoch FROM seats WHERE id=?1 LIMIT 1",
            ),
            (
                "wake",
                "SELECT reason_bits,binding_generation,retry_step,reservation_id,last_outcome,attention_version,checkpoint_version FROM wake_work WHERE seat_id=?1 LIMIT 1",
            ),
        ] {
            let rows = (|| -> rusqlite::Result<Vec<Value>> {
                let mut statement = db.prepare(query)?;
                let columns: Vec<_> = statement
                    .column_names()
                    .iter()
                    .map(|name| (*name).to_owned())
                    .collect();
                let rows = statement.query_map([&self.b], |row| {
                    let mut fields = serde_json::Map::new();
                    for (index, name) in columns.iter().enumerate() {
                        let value = match row.get_ref(index)? {
                            rusqlite::types::ValueRef::Null => Value::Null,
                            rusqlite::types::ValueRef::Integer(value) => json!(value),
                            rusqlite::types::ValueRef::Real(value) => json!(value),
                            rusqlite::types::ValueRef::Text(value) => {
                                diagnostic_text(&String::from_utf8_lossy(value), 256)
                            }
                            rusqlite::types::ValueRef::Blob(value) => {
                                json!({"blob_bytes": value.len()})
                            }
                        };
                        fields.insert(name.clone(), value);
                    }
                    Ok(Value::Object(fields))
                })?;
                rows.collect()
            })();
            result.insert(
                name.into(),
                match rows {
                    Ok(rows) => json!(rows),
                    Err(error) => json!({"read_error": error.to_string()}),
                },
            );
        }
        Value::Object(result)
    }

    /// A native wake prompt reaches `pane` after `mark`.
    fn wait_prompt(&self, mark: usize, pane: &str) {
        let until = Instant::now() + STEP;
        while self.host.prompts_since(mark, pane) == 0 {
            if Instant::now() >= until {
                let calls: Vec<_> = self.host.calls.lock().unwrap()[mark..]
                    .iter()
                    .rev()
                    .take(8)
                    .map(|(method, params)| {
                        format!("{method} {}", diagnostic_text(&params.to_string(), 512))
                    })
                    .collect();
                panic!(
                    "timed out waiting for a native wake prompt; bounded setup diagnostics: {}; last 8 host calls since the mark: {calls:?}",
                    self.setup_wake_diagnostics(),
                );
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        if let Some(path) = std::env::var_os("HT_MOD_DELIVERY_PANIC_LOG") {
            let calls = self.host.calls.lock().unwrap()[mark..]
                .iter()
                .map(|(method, params)| format!("{method} {}", params["target"]))
                .collect::<Vec<_>>();
            let _ = fs::write(
                format!("{}.ok", path.to_string_lossy()),
                format!("{calls:?}"),
            );
        }
    }

    /// No native wake prompt reaches B within `window` after `mark`.
    fn assert_no_prompt(&self, mark: usize, window: Duration) {
        std::thread::sleep(window);
        assert_eq!(
            self.host.prompts_since(mark, PANE_B),
            0,
            "a native wake reached B while its mod channel was live"
        );
    }

    /// Herdr now reports `session` for B's pane (what `/clear` does).
    fn set_session_b(&self, session: &str) {
        // A `/clear` starts a new session inside the same harness process.
        let mut b = claude(PANE_B, "term-b", session);
        b["foreground_pid"] = json!(foreground_of(SESSION_B));
        self.host.set_panes(vec![pane(PANE_A, "term-a"), b]);
    }

    /// B's agent has run its turn after a native prompt: Herdr shows it idle
    /// again. The wake path reads B's UI state when it wakes (a `working`
    /// pane is an active turn and defers the wake), so the state the next
    /// wake sees is what matters; nothing reads it in between (the stand-in
    /// served no status read of B while a turn was played out in real time).
    fn agent_turn(&self) {
        let mut b = claude(PANE_B, "term-b", SESSION_B);
        b["agent_status"] = json!("idle");
        self.host.set_panes(vec![pane(PANE_A, "term-a"), b]);
    }

    /// B's `watch` child gone (a reload or crash).
    fn kill(&self, watch: &mut Watch) {
        let _ = watch.child.stop();
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        // The elected daemon points this process's stderr at daemon.log, so a
        // failed assertion's message lands there: show its tail.
        if std::thread::panicking()
            && let Ok(log) = fs::read(self.paths.instance_dir.join("daemon.log"))
        {
            let tail = &log[log.len().saturating_sub(8 * 1024)..];
            println!("daemon.log tail:\n{}", String::from_utf8_lossy(tail));
        }
        self.stop_daemon();
        if std::thread::panicking() {
            for panic in self
                .panics
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .iter()
            {
                eprintln!("{panic}");
            }
        }
    }
}

// -------------------------------------------------------------------- watch

/// A `watch` child with its stdout lines forwarded by a reader thread.
struct Watch {
    child: OwnedChild,
    lines: Receiver<Value>,
    seen: Vec<Value>,
    /// Lines read ahead by `peek_first` and not yet handed to a waiter.
    pending: VecDeque<Value>,
    reader: Option<JoinHandle<()>>,
    error_reader: Option<JoinHandle<()>>,
    errors: Arc<Mutex<String>>,
    /// Exact child bytes, including newline boundaries, for activation evidence.
    stdout: Arc<Mutex<Vec<u8>>>,
}
impl Watch {
    fn spawn(mut command: std::process::Command) -> Self {
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn_owned()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let errors: Arc<Mutex<String>> = Arc::default();
        let sink = errors.clone();
        let error_reader = std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                let mut text = sink.lock().unwrap();
                text.push_str(&line);
                text.push('\n');
            }
        });
        let (tx, lines) = mpsc::channel();
        let captured = Arc::new(Mutex::new(Vec::new()));
        let sink = captured.clone();
        let reader = std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut line = Vec::new();
            while reader
                .read_until(b'\n', &mut line)
                .is_ok_and(|len| len != 0)
            {
                {
                    let mut bytes = sink.lock().unwrap();
                    // Only the activation proof reads this bounded evidence.
                    if bytes.len() + line.len() <= 131_072 {
                        bytes.extend_from_slice(&line);
                    }
                }
                if let Ok(value) = serde_json::from_slice::<Value>(&line)
                    && tx.send(value).is_err()
                {
                    return;
                }
                line.clear();
            }
        });
        Self {
            child,
            lines,
            seen: Vec::new(),
            pending: VecDeque::new(),
            reader: Some(reader),
            error_reader: Some(error_reader),
            errors,
            stdout: captured,
        }
    }

    /// What the child has printed on stderr so far.
    fn stderr(&self) -> String {
        self.errors.lock().unwrap().clone()
    }

    /// Waits for the next line satisfying `pred`; every line read is kept in
    /// `seen`.
    fn wait_line(&mut self, what: &str, timeout: Duration, pred: impl Fn(&Value) -> bool) -> Value {
        let until = Instant::now() + timeout;
        loop {
            let remaining = until.saturating_duration_since(Instant::now());
            let next = match self.pending.pop_front() {
                Some(line) => Ok(line),
                None => self.lines.recv_timeout(remaining),
            };
            match next {
                Ok(line) => {
                    self.seen.push(line.clone());
                    if pred(&line) {
                        return line;
                    }
                }
                Err(_) => panic!(
                    "timed out waiting for {what}; saw {:#?}; stderr: {}",
                    self.seen,
                    self.stderr()
                ),
            }
        }
    }

    /// The first line the child prints, left for the next waiter.
    fn peek_first(&mut self, timeout: Duration) -> Option<Value> {
        if self.pending.is_empty()
            && self.seen.is_empty()
            && let Ok(line) = self.lines.recv_timeout(timeout)
        {
            self.pending.push_back(line);
        }
        self.pending.front().cloned()
    }

    fn wait_connected(&mut self) {
        self.wait_line("the connected status", STEP, |line| {
            line["kind"] == "status" && line["state"] == "connected"
        });
    }

    /// The `message` / `lazy` line carrying `id`.
    fn wait_item(&mut self, id: &str) -> Value {
        self.wait_line(&format!("item {id}"), STEP, |line| {
            line["id"] == id && matches!(line["kind"].as_str(), Some("message" | "lazy"))
        })
    }

    fn wait_attention_marker(&mut self) -> Value {
        self.wait_line("an attention line", STEP, |line| {
            line["kind"] == "attention"
        })
    }

    /// No line satisfying `pred` was seen, nor arrives within `window`.
    fn assert_no_line(&mut self, what: &str, window: Duration, pred: impl Fn(&Value) -> bool) {
        self.seen.extend(self.pending.drain(..));
        if let Some(line) = self.seen.iter().find(|line| pred(line)) {
            panic!("unexpected {what}: {line}");
        }
        let until = Instant::now() + window;
        loop {
            let remaining = until.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(remaining) {
                Ok(line) => {
                    self.seen.push(line.clone());
                    assert!(!pred(&line), "unexpected {what}: {line}");
                }
                Err(_) => return,
            }
        }
    }

    /// Waits for the child to exit; returns its exit code.
    fn exit_code(&mut self, timeout: Duration) -> i32 {
        wait_until("the watch child to exit", timeout, || {
            self.child
                .try_wait()
                .unwrap()
                .map(|status| status.code().unwrap_or(-1))
        })
    }

    /// The last status line seen or still to arrive after the stream ended.
    fn final_status(&mut self) -> Value {
        self.seen.extend(self.pending.drain(..));
        // The reader hands over everything before its EOF.
        while let Ok(line) = self.lines.recv_timeout(Duration::from_secs(5)) {
            self.seen.push(line);
        }
        self.seen
            .iter()
            .rev()
            .find(|line| line["kind"] == "status")
            .cloned()
            .unwrap_or_else(|| panic!("no status line in {:#?}", self.seen))
    }

    fn running(&mut self) -> bool {
        self.child.try_wait().unwrap().is_none()
    }
}
impl Drop for Watch {
    fn drop(&mut self) {
        let _ = self.child.stop();
        for reader in [self.reader.take(), self.error_reader.take()]
            .into_iter()
            .flatten()
        {
            let _ = reader.join();
        }
    }
}

// --------------------------------------------------- activation observation

/// This relay forwards real frames unchanged. The only local routing change is
/// the isolated descriptor's socket inode, restored before the rig shuts down.
/// It never supplies capabilities, leases, pages, or confirmation results.
#[derive(Clone)]
struct ObservedExchange {
    request: Value,
    reply: Option<Value>,
}

#[derive(Clone, Copy)]
enum ConfirmFault {
    None,
    BeforeCommit,
    LoseReply,
}

/// Test-only causal ordering: real Pull replies and real channel frames wait
/// for explicit permits. No timer determines whether the consumer is quiet.
#[derive(Default)]
struct CapacityFlowState {
    requests: Vec<(usize, u64)>,
    ready: Vec<usize>,
    released: usize,
    attention_seen: usize,
    attention_credits: usize,
    attention_forwarded: Vec<u64>,
    attention_expected_requests: Option<usize>,
    // Actual downstream reply writes, distinct from release permits. At most
    // one per admitted Pull (the existing admission bound is 65).
    reply_writes: Vec<(usize, Option<String>)>,
    failure: Option<String>,
    order: u64,
    stop: bool,
}

#[derive(Default)]
struct CapacityFlow {
    state: Mutex<CapacityFlowState>,
    changed: std::sync::Condvar,
}

impl CapacityFlow {
    fn lock(&self) -> MutexGuard<'_, CapacityFlowState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn fail(&self, state: &mut CapacityFlowState, message: String) -> std::io::Error {
        state.failure.get_or_insert(message.clone());
        state.stop = true;
        self.changed.notify_all();
        std::io::Error::other(message)
    }

    fn start_pull(&self, index: usize) -> std::io::Result<usize> {
        let mut state = self.lock();
        state.order += 1;
        let order = state.order;
        state.requests.push((index, order));
        if state.requests.len() > 65 {
            return Err(self.fail(&mut state, "capacity fixture Pull observation bound".into()));
        }
        self.changed.notify_all();
        Ok(state.requests.len())
    }

    fn hold_reply(&self, ordinal: usize, index: usize) -> std::io::Result<bool> {
        let mut state = self.lock();
        if state.ready.len() + 1 != ordinal {
            return Err(self.fail(&mut state, "one sequential real Watch consumer".into()));
        }
        state.ready.push(index);
        self.changed.notify_all();
        while !state.stop && state.released < ordinal {
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        Ok(!state.stop)
    }

    fn forward_attention(&self, downstream: &mut impl Write, body: &[u8]) -> std::io::Result<bool> {
        let mut state = self.lock();
        state.attention_seen += 1;
        self.changed.notify_all();
        while !state.stop && state.attention_credits == 0 {
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        if state.stop {
            return Ok(false);
        }
        if state.attention_expected_requests != Some(state.requests.len()) {
            return Err(self.fail(
                &mut state,
                "extra real Pull before Attention forwarding".into(),
            ));
        }
        state.attention_credits -= 1;
        // Serialize actual successful frame output with Pull observation. A
        // permission alone, partial output, or failed write is never a marker.
        // Drop shuts down sockets before acquiring this lock to interrupt I/O.
        if let Err(error) = activation_write(downstream, body) {
            return Err(self.fail(
                &mut state,
                format!("real Attention forwarding failed: {error}"),
            ));
        }
        state.order += 1;
        let order = state.order;
        state.attention_forwarded.push(order);
        self.changed.notify_all();
        Ok(true)
    }

    fn wait_reply(&self, ordinal: usize) -> usize {
        self.wait_reply_for(ordinal, STEP)
    }

    fn wait_reply_for(&self, ordinal: usize, timeout: Duration) -> usize {
        let state = self.lock();
        let (state, _) = self
            .changed
            .wait_timeout_while(state, timeout, |state| {
                !state.stop && state.ready.len() < ordinal
            })
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (index, requests, failure) = (
            state.ready.get(ordinal.wrapping_sub(1)).copied(),
            state.requests.clone(),
            state.failure.clone(),
        );
        drop(state);
        assert!(
            failure.is_none(),
            "capacity relay worker failed: {failure:?}"
        );
        index.unwrap_or_else(|| panic!("expected real Watch Pull {ordinal}; requests={requests:?}"))
    }

    fn release_reply(&self, ordinal: usize) {
        let mut state = self.lock();
        let expected = state.released + 1;
        if expected == ordinal {
            state.released = ordinal;
        }
        self.changed.notify_all();
        drop(state);
        assert_eq!(expected, ordinal);
    }

    fn release_attention_after(&self, ordinal: usize) {
        let state = self.lock();
        let (mut state, _) = self
            .changed
            .wait_timeout_while(state, STEP, |state| {
                !state.stop && state.attention_seen == 0
            })
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (seen, requests, forwarded, failure) = (
            state.attention_seen,
            state.requests.len(),
            state.attention_forwarded.len(),
            state.failure.clone(),
        );
        if failure.is_none() && seen > 0 && requests == ordinal && forwarded == 0 {
            state.attention_expected_requests = Some(ordinal);
            state.attention_credits = 1;
            self.changed.notify_all();
        }
        drop(state);
        assert!(
            failure.is_none(),
            "capacity relay worker failed: {failure:?}"
        );
        assert!(seen > 0, "expected an actual queued daemon Attention");
        assert_eq!(
            requests, ordinal,
            "no new pull before a real channel stimulus"
        );
        assert_eq!(forwarded, 0, "initial drain had no channel input");
    }

    fn assert_resumed_after_attention(&self, ordinal: usize) {
        let state = self.lock();
        let (forwarded, request, failure) = (
            state.attention_forwarded.clone(),
            state.requests.get(ordinal.wrapping_sub(1)).copied(),
            state.failure.clone(),
        );
        drop(state);
        assert!(
            failure.is_none(),
            "capacity relay worker failed: {failure:?}"
        );
        assert_eq!(forwarded.len(), 1);
        assert!(
            request.is_some_and(|(_, order)| order > forwarded[0]),
            "the actual next Pull follows the real forwarded channel frame"
        );
    }

    fn stop(&self) {
        self.lock().stop = true;
        self.changed.notify_all();
    }
}

struct ActivationRelay {
    endpoint: PathBuf,
    upstream: PathBuf,
    descriptor: PathBuf,
    original_descriptor: Vec<u8>,
    capacity: Option<Arc<CapacityFlow>>,
    stop: Arc<AtomicBool>,
    release: Arc<AtomicBool>,
    response_release: Arc<AtomicBool>,
    exchanges: Arc<Mutex<Vec<ObservedExchange>>>,
    open: Arc<Mutex<Vec<UnixStream>>>,
    worker: Option<JoinHandle<()>>,
}

fn activation_read(stream: &mut UnixStream) -> std::io::Result<Vec<u8>> {
    let mut prefix = [0; 4];
    stream.read_exact(&mut prefix)?;
    let len = u32::from_be_bytes(prefix) as usize;
    if len == 0 || len > herdr_threads::protocol::wire::MAX_WIRE_FRAME_BYTES {
        return Err(std::io::Error::other("activation frame bound"));
    }
    let mut body = vec![0; len];
    stream.read_exact(&mut body)?;
    Ok(body)
}

fn activation_write(stream: &mut impl Write, body: &[u8]) -> std::io::Result<()> {
    stream.write_all(&(body.len() as u32).to_be_bytes())?;
    stream.write_all(body)
}

impl ActivationRelay {
    fn start(rig: &Rig, hold: bool, fault: ConfirmFault) -> Self {
        Self::start_with_capacity(rig, hold, fault, None)
    }

    fn start_with_capacity(
        rig: &Rig,
        hold: bool,
        fault: ConfirmFault,
        capacity: Option<Arc<CapacityFlow>>,
    ) -> Self {
        let endpoint = rig.paths.socket_path.clone();
        let upstream = rig.root.join("activation-upstream.sock");
        let descriptor = rig.paths.descriptor_path.clone();
        let original_descriptor = fs::read(&descriptor).unwrap();
        fs::rename(&endpoint, &upstream).unwrap();
        let listener = UnixListener::bind(&endpoint).unwrap();
        fs::set_permissions(&endpoint, fs::Permissions::from_mode(0o600)).unwrap();
        let meta = fs::metadata(&endpoint).unwrap();
        let mut routed: Value = serde_json::from_slice(&original_descriptor).unwrap();
        routed["socket_device"] = json!(meta.dev());
        routed["socket_inode"] = json!(meta.ino());
        fs::write(&descriptor, serde_json::to_vec(&routed).unwrap()).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let release = Arc::new(AtomicBool::new(!hold));
        // The positive test observes canonical retirement before this child
        // can receive its Confirm reply and Pull a successor notice page.
        let response_release = Arc::new(AtomicBool::new(
            !hold || !matches!(fault, ConfirmFault::None),
        ));
        let exchanges: Arc<Mutex<Vec<ObservedExchange>>> = Arc::default();
        let open: Arc<Mutex<Vec<UnixStream>>> = Arc::default();
        let used = Arc::new(AtomicBool::new(false));
        let (stopped, released, log, sockets, target) = (
            stop.clone(),
            release.clone(),
            exchanges.clone(),
            open.clone(),
            upstream.clone(),
        );
        let capacity_worker = capacity.clone();
        let response_released = response_release.clone();
        let worker = std::thread::spawn(move || {
            let mut threads = Vec::new();
            for incoming in listener.incoming() {
                if stopped.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(mut downstream) = incoming else { break };
                if downstream.set_write_timeout(Some(STEP)).is_err() {
                    break;
                }
                let Ok(mut upstream) = UnixStream::connect(&target) else {
                    break;
                };
                {
                    let mut open = sockets.lock().unwrap();
                    open.push(downstream.try_clone().unwrap());
                    open.push(upstream.try_clone().unwrap());
                }
                let (stopped, released, log, used) =
                    (stopped.clone(), released.clone(), log.clone(), used.clone());
                let capacity = capacity_worker.clone();
                let response_released = response_released.clone();
                threads.push(std::thread::spawn(move || {
                    let mut serve = || -> std::io::Result<()> {
                        let request_bytes = activation_read(&mut downstream)?;
                        let request: Value = serde_json::from_slice(&request_bytes)?;
                        let control_confirm = request["command"]["kind"]
                            == "delivery_presentation_confirm_v2"
                            && request["command"]["args"]["emitted_controls"]
                                .as_array()
                                .is_some_and(|stamps| !stamps.is_empty());
                        let first = control_confirm && !used.swap(true, Ordering::SeqCst);
                        let index = {
                            let mut log = log.lock().unwrap();
                            let index = log.len();
                            // Purpose-specific bounded capture, never a traffic dump.
                            if index >= 256 {
                                return Err(std::io::Error::other("activation exchange bound"));
                            }
                            log.push(ObservedExchange {
                                request,
                                reply: None,
                            });
                            index
                        };
                        let is_pull = log.lock().unwrap()[index].request["command"]["kind"]
                            == "delivery_pull_v2";
                        let pull_ordinal = if let Some(flow) = &capacity {
                            if is_pull {
                                Some(flow.start_pull(index)?)
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                        if first && matches!(fault, ConfirmFault::BeforeCommit) {
                            return Ok(());
                        }
                        if first && matches!(fault, ConfirmFault::None) {
                            while !released.load(Ordering::SeqCst) {
                                if stopped.load(Ordering::SeqCst) {
                                    return Ok(());
                                }
                                std::thread::sleep(Duration::from_millis(5));
                            }
                        }
                        activation_write(&mut upstream, &request_bytes)?;
                        let reply_bytes = activation_read(&mut upstream)?;
                        log.lock().unwrap()[index].reply =
                            Some(serde_json::from_slice(&reply_bytes)?);
                        if first && matches!(fault, ConfirmFault::None) {
                            while !response_released.load(Ordering::SeqCst) {
                                if stopped.load(Ordering::SeqCst) {
                                    return Ok(());
                                }
                                std::thread::sleep(Duration::from_millis(5));
                            }
                        }
                        if let (Some(flow), Some(ordinal)) = (&capacity, pull_ordinal)
                            && !flow.hold_reply(ordinal, index)?
                        {
                            return Ok(());
                        }
                        if first && matches!(fault, ConfirmFault::LoseReply) {
                            // Keep the real child/owner live until the test replays
                            // the committed request, then simulate the lost response.
                            while !released.load(Ordering::SeqCst) {
                                if stopped.load(Ordering::SeqCst) {
                                    return Ok(());
                                }
                                std::thread::sleep(Duration::from_millis(5));
                            }
                            return Ok(());
                        }
                        let written = activation_write(&mut downstream, &reply_bytes);
                        if let (Some(flow), Some(ordinal)) = (&capacity, pull_ordinal) {
                            let mut state = flow.lock();
                            if state.reply_writes.len() < 65 {
                                state.reply_writes.push((
                                    ordinal,
                                    written
                                        .as_ref()
                                        .err()
                                        .map(|error| format!("{:?}", error.kind())),
                                ));
                            }
                        }
                        written?;
                        // Only Watch has further frames on this connection.
                        if log.lock().unwrap()[index].request.get("watch").is_some() {
                            // Propagate child EOF too: preserving only server-to-child
                            // frames would incorrectly keep a dead owner live.
                            let mut client = downstream.try_clone()?;
                            let mut server = upstream.try_clone()?;
                            let peer = std::thread::spawn(move || {
                                let _ = std::io::copy(&mut client, &mut server);
                                let _ = server.shutdown(std::net::Shutdown::Both);
                            });
                            let forwarded = loop {
                                let body = match activation_read(&mut upstream) {
                                    Ok(body) => body,
                                    Err(error) => break Err(error),
                                };
                                if let Some(flow) = &capacity {
                                    let frame: Value = match serde_json::from_slice(&body) {
                                        Ok(frame) => frame,
                                        Err(error) => break Err(std::io::Error::other(error)),
                                    };
                                    if frame["kind"] == "attention" {
                                        match flow.forward_attention(&mut downstream, &body) {
                                            Ok(true) => continue,
                                            Ok(false) => break Ok(()),
                                            Err(error) => break Err(error),
                                        }
                                    }
                                }
                                if let Err(error) = activation_write(&mut downstream, &body) {
                                    break Err(error);
                                }
                            };
                            let _ = downstream.shutdown(std::net::Shutdown::Both);
                            let _ = peer.join();
                            return forwarded;
                        }
                        Ok(())
                    };
                    let _ = serve();
                    let _ = downstream.shutdown(std::net::Shutdown::Both);
                    let _ = upstream.shutdown(std::net::Shutdown::Both);
                }));
            }
            for thread in threads {
                let _ = thread.join();
            }
        });
        Self {
            endpoint,
            upstream,
            descriptor,
            original_descriptor,
            capacity,
            stop,
            release,
            response_release,
            exchanges,
            open,
            worker: Some(worker),
        }
    }

    fn snapshot(&self) -> Vec<ObservedExchange> {
        self.exchanges.lock().unwrap().clone()
    }

    fn control_confirm(&self, replied: bool) -> ObservedExchange {
        wait_until("the real child's nonempty control Confirm", STEP, || {
            self.snapshot().into_iter().find(|entry| {
                entry.request["command"]["kind"] == "delivery_presentation_confirm_v2"
                    && entry.request["command"]["args"]["emitted_controls"]
                        .as_array()
                        .is_some_and(|stamps| !stamps.is_empty())
                    && (!replied || entry.reply.is_some())
            })
        })
    }

    fn release(&self) {
        self.release.store(true, Ordering::SeqCst);
    }

    fn release_response(&self) {
        self.response_release.store(true, Ordering::SeqCst);
    }

    /// A separate explicit recovery call, never the selected-consumer positive.
    fn replay(&self, captured: &ObservedExchange, change: impl FnOnce(&mut Value)) -> Value {
        let mut request = captured.request.clone();
        request["request_id"] = json!(uuid::Uuid::new_v4().to_string());
        change(&mut request);
        let mut socket = UnixStream::connect(&self.upstream).unwrap();
        socket.set_read_timeout(Some(STEP)).unwrap();
        socket.set_write_timeout(Some(STEP)).unwrap();
        activation_write(&mut socket, &serde_json::to_vec(&request).unwrap()).unwrap();
        serde_json::from_slice(&activation_read(&mut socket).unwrap()).unwrap()
    }
}

impl Drop for ActivationRelay {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.release();
        self.release_response();
        let _ = UnixStream::connect(&self.endpoint);
        for socket in self
            .open
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
        {
            let _ = socket.shutdown(std::net::Shutdown::Both);
        }
        if let Some(flow) = &self.capacity {
            flow.stop();
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        let _ = fs::remove_file(&self.endpoint);
        let _ = fs::rename(&self.upstream, &self.endpoint);
        let _ = fs::write(&self.descriptor, &self.original_descriptor);
    }
}

// These helper regressions require no elected daemon or advertised capability.
// Their local byte streams are helper tests, never an activation capture.
#[test]
fn activation_capacity_flow_oracle_unwind_restores_relay() {
    // SHORT_TMP (/private/tmp on macOS, /tmp elsewhere) keeps the socket paths
    // under the Unix limit; a hardcoded /private/tmp does not exist on Linux.
    let scratch = Scratch(PathBuf::from(format!(
        "{}/ht-cf-{}",
        herdr_threads::test_support::SHORT_TMP,
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    )));
    fs::create_dir(&scratch.0).unwrap();
    let endpoint = scratch.0.join("relay.sock");
    let upstream = scratch.0.join("upstream.sock");
    let descriptor = scratch.0.join("endpoint.json");
    let _listener = UnixListener::bind(&endpoint).unwrap();
    let _real_listener = UnixListener::bind(&upstream).unwrap();
    fs::write(&descriptor, b"routed descriptor").unwrap();
    let flow = Arc::new(CapacityFlow::default());
    let waiting = flow.clone();
    let (finished, completion) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let result = waiting.hold_reply(1, 7);
        let _ = finished.send(result);
    });
    let (socket, mut peer) = UnixStream::pair().unwrap();
    peer.set_read_timeout(Some(STEP)).unwrap();
    let relay = ActivationRelay {
        endpoint: endpoint.clone(),
        upstream: upstream.clone(),
        descriptor: descriptor.clone(),
        original_descriptor: b"original descriptor".to_vec(),
        capacity: Some(flow.clone()),
        stop: Arc::new(AtomicBool::new(false)),
        release: Arc::new(AtomicBool::new(false)),
        response_release: Arc::new(AtomicBool::new(false)),
        exchanges: Arc::default(),
        open: Arc::new(Mutex::new(vec![socket])),
        worker: Some(worker),
    };
    assert_eq!(flow.wait_reply(1), 7);
    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _relay = relay;
        flow.wait_reply_for(2, Duration::ZERO);
    }))
    .expect_err("the missing second Pull must fail the oracle");
    let diagnostic = failure
        .downcast_ref::<String>()
        .expect("original timeout diagnostic");
    assert!(
        diagnostic.contains("expected real Watch Pull 2"),
        "{diagnostic}"
    );
    assert!(
        !flow.state.is_poisoned(),
        "oracle assertions release the controller lock"
    );
    assert!(
        !completion.recv_timeout(STEP).unwrap().unwrap(),
        "Drop released and joined the waiting worker"
    );
    assert_eq!(
        peer.read(&mut [0; 1]).unwrap(),
        0,
        "Drop shut down the owned socket"
    );
    assert_eq!(fs::read(&descriptor).unwrap(), b"original descriptor");
    assert!(endpoint.exists());
    assert!(
        !upstream.exists(),
        "Drop restored the original socket pathname"
    );
}

#[test]
fn activation_capacity_flow_oracle_assertions_do_not_poison() {
    let flow = CapacityFlow::default();
    let release = std::panic::catch_unwind(|| flow.release_reply(2));
    assert!(release.is_err());
    assert!(!flow.state.is_poisoned());
    flow.lock().attention_seen = 1;
    let stimulus = std::panic::catch_unwind(|| flow.release_attention_after(1));
    assert!(stimulus.is_err());
    assert!(!flow.state.is_poisoned());
    let order = std::panic::catch_unwind(|| flow.assert_resumed_after_attention(1));
    assert!(order.is_err());
    assert!(!flow.state.is_poisoned());
    flow.stop();
}

#[test]
fn activation_capacity_flow_stop_recovers_poison_and_releases_waiter() {
    struct StopOnDrop(Arc<CapacityFlow>);
    impl Drop for StopOnDrop {
        fn drop(&mut self) {
            self.0.stop();
        }
    }
    let flow = Arc::new(CapacityFlow::default());
    std::thread::scope(|scope| {
        let _stop = StopOnDrop(flow.clone());
        let waiting = flow.clone();
        let worker = scope.spawn(move || waiting.hold_reply(1, 7));
        assert_eq!(flow.wait_reply(1), 7);
        let poisoned = std::panic::catch_unwind(|| {
            let _state = flow.state.lock().unwrap();
            panic!("induced controller poison");
        });
        assert!(poisoned.is_err());
        assert!(flow.state.is_poisoned());
        flow.stop();
        assert!(
            !worker.join().unwrap().unwrap(),
            "poison recovery releases a condvar waiter"
        );
    });
}

#[test]
fn activation_capacity_flow_failed_or_unwritten_attention_has_no_marker() {
    struct FailedWrite {
        prefix_written: bool,
    }
    impl Write for FailedWrite {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if !self.prefix_written {
                self.prefix_written = true;
                return Ok(bytes.len());
            }
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "induced body write failure",
            ))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let flow = CapacityFlow::default();
    {
        let mut state = flow.lock();
        state.attention_seen = 1;
    }
    flow.release_attention_after(0);
    let forwarded = flow.lock().attention_forwarded.clone();
    assert!(forwarded.is_empty(), "permission is not forwarding");
    let mut writer = FailedWrite {
        prefix_written: false,
    };
    assert!(
        flow.forward_attention(&mut writer, b"helper bytes")
            .is_err()
    );
    assert!(
        writer.prefix_written,
        "partial output still has no successful frame marker"
    );
    let forwarded = flow.lock().attention_forwarded.clone();
    assert!(forwarded.is_empty(), "failed output is not forwarding");
    let failure = std::panic::catch_unwind(|| flow.wait_reply_for(1, Duration::ZERO))
        .expect_err("worker write failure must remain observable");
    assert!(
        failure
            .downcast_ref::<String>()
            .unwrap()
            .contains("real Attention forwarding failed")
    );
    assert!(!flow.state.is_poisoned());
    flow.stop();

    let flow = CapacityFlow::default();
    flow.lock().attention_seen = 1;
    flow.release_attention_after(0);
    flow.start_pull(7).unwrap();
    let mut bytes = Vec::new();
    let error = flow
        .forward_attention(&mut bytes, b"helper bytes")
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("extra real Pull before Attention forwarding")
    );
    assert!(
        bytes.is_empty(),
        "unexpected pre-stimulus Pull prevents frame output"
    );
    let forwarded = flow.lock().attention_forwarded.clone();
    assert!(forwarded.is_empty());
}

#[test]
fn activation_capacity_flow_successful_frame_write_serializes_pull_observation() {
    struct OrderedWrite {
        socket: UnixStream,
        flow: Arc<CapacityFlow>,
        ordered_writes: usize,
        unordered_writes: usize,
    }
    impl Write for OrderedWrite {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            // Probe BEFORE each actual socket write, including the length
            // prefix: Pull observation must be excluded for all frame bytes.
            if matches!(
                self.flow.state.try_lock(),
                Err(std::sync::TryLockError::WouldBlock)
            ) {
                self.ordered_writes += 1;
            } else {
                self.unordered_writes += 1;
            }
            self.socket.write(bytes)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.socket.flush()
        }
    }
    let flow = Arc::new(CapacityFlow::default());
    flow.lock().attention_seen = 1;
    flow.release_attention_after(0);
    let (socket, mut peer) = UnixStream::pair().unwrap();
    let mut writer = OrderedWrite {
        socket,
        flow: flow.clone(),
        ordered_writes: 0,
        unordered_writes: 0,
    };
    assert!(
        flow.forward_attention(&mut writer, b"helper frame bytes")
            .unwrap()
    );
    assert_eq!(activation_read(&mut peer).unwrap(), b"helper frame bytes");
    assert!(
        writer.ordered_writes >= 2,
        "both prefix and body writes hold the ordering lock"
    );
    assert_eq!(writer.unordered_writes, 0);
    assert_eq!(flow.start_pull(7).unwrap(), 1);
    flow.assert_resumed_after_attention(1);
}

/// Check the actual discovery response and SAME child's registration, never a
/// separately opened raw Watch which would replace the child's owner.
fn activation_owner(relay: &ActivationRelay) -> String {
    activation_owner_from(relay, 0)
}

fn activation_owner_from(relay: &ActivationRelay, start: usize) -> String {
    let log: Vec<_> = relay.snapshot().into_iter().skip(start).collect();
    let capabilities = log
        .iter()
        .find(|entry| entry.request["command"]["kind"] == "capabilities" && entry.reply.is_some())
        .expect("the watch child must discover capabilities");
    let response: herdr_threads::protocol::wire::WireResponse =
        serde_json::from_value(capabilities.reply.clone().unwrap()).unwrap();
    let Ok(herdr_threads::protocol::results::CommandResult::Capabilities(list)) = response.result
    else {
        panic!("real capability route refused")
    };
    assert!(
        list.capabilities.iter().any(|name| {
            name == herdr_threads::protocol::capabilities::DELIVERY_PRESENTATION_V2
        }),
        "elected daemon must advertise presentation V2 before activation proof"
    );
    let accepted = log
        .iter()
        .find(|entry| entry.request.get("watch").is_some())
        .expect("same child Watch registration");
    let lease = accepted.reply.as_ref().unwrap()["outcome"]["args"]["presentation_lease"]
        .as_str()
        .expect("advertised same-child Accepted lease")
        .to_owned();
    assert!(herdr_threads::protocol::delivery::valid_presentation_token(
        &lease
    ));
    let log = wait_until("the advertised child's selected V2 Pull", STEP, || {
        let log: Vec<_> = relay.snapshot().into_iter().skip(start).collect();
        log.iter()
            .any(|entry| entry.request["command"]["kind"] == "delivery_pull_v2")
            .then_some(log)
    });
    for entry in log.iter().filter(|entry| {
        matches!(
            entry.request["command"]["kind"].as_str(),
            Some("delivery_pull_v2" | "delivery_presentation_confirm_v2")
        )
    }) {
        assert_eq!(
            entry.request["command"]["args"]["claim"],
            accepted.request["watch"]["claim"]
        );
        assert_eq!(
            entry.request["command"]["args"]["presentation_lease"],
            lease
        );
    }
    assert!(
        log.iter()
            .any(|entry| entry.request["command"]["kind"] == "delivery_pull_v2")
    );
    assert!(
        !log.iter()
            .any(|entry| entry.request["command"]["kind"] == "inbox_batch_v2")
    );
    lease
}

fn activation_page(relay: &ActivationRelay, confirm: &ObservedExchange) -> Value {
    let token = &confirm.request["command"]["args"]["token"];
    relay
        .snapshot()
        .into_iter()
        .filter_map(|entry| entry.reply)
        .find_map(|reply| {
            let result = &reply["result"]["Ok"];
            (result["kind"] == "delivery_pulled_v2"
                && result["data"]["kind"] == "page"
                && result["data"]["args"]["presentation_token"] == *token)
                .then(|| result["data"]["args"].clone())
        })
        .expect("actual Pull page matching child's Confirm token")
}

fn activation_pending(rig: &Rig) -> (Option<String>, Option<String>, i64) {
    rig.db().query_row(
        "SELECT outbox_token,last_token,last_confirmed_mask FROM invitation_presentation_consumers WHERE seat_id=?1 AND consumer='watch'",
        [&rig.b], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap()
}

fn activation_retirement_diagnostics(rig: &Rig, relay: &ActivationRelay) -> Value {
    let (current, last, mask) = activation_pending(rig);
    let recent: Vec<_> = relay
        .snapshot()
        .into_iter()
        .rev()
        .filter(|entry| {
            matches!(
                entry.request["command"]["kind"].as_str(),
                Some("delivery_pull_v2" | "delivery_presentation_confirm_v2")
            )
        })
        .take(6)
        .map(|entry| {
            let reply = entry.reply.as_ref().map(|reply| &reply["result"]["Ok"]);
            let items: Vec<_> = reply
                .and_then(|reply| reply["data"]["args"]["items"].as_array())
                .into_iter()
                .flatten()
                .map(|item| json!({"kind": item["kind"], "id": item["id"]}))
                .collect();
            json!({
                "request": entry.request["command"]["kind"],
                "submitted_token": entry.request["command"]["args"]["token"],
                "submitted_controls": entry.request["command"]["args"]["emitted_controls"],
                "reply": reply.map(|reply| json!({
                    "kind": reply["kind"], "pull_kind": reply["data"]["kind"],
                    "page_token": reply["data"]["args"]["presentation_token"],
                    "more": reply["data"]["args"]["more"], "items": items,
                    "controls": reply["data"]["args"]["emitted_controls"],
                    "status": reply["data"]["status"],
                    "acknowledged_controls": reply["data"]["acknowledged_controls"],
                })),
            })
        })
        .collect();
    json!({"canonical": {"current": current, "last": last, "last_confirmed_mask": mask},
        "recent_actual_exchanges_newest_first": recent})
}

fn activation_confirm_result(reply: &Value, status: &str, controls: &Value) {
    let result = &reply["result"]["Ok"];
    assert_eq!(
        result["kind"], "delivery_presentation_confirmed_v2",
        "{reply}"
    );
    assert_eq!(result["data"]["status"], status, "{reply}");
    assert_eq!(
        &result["data"]["acknowledged_controls"], controls,
        "{reply}"
    );
}

fn activation_invitation(rig: &Rig) -> String {
    let thread = rig
        .cli(
            Some((&rig.a, PANE_A)),
            &["thread", "create", "--topic", "activation invitation"],
        )
        .text("activation thread");
    rig.cli(
        Some((&rig.a, PANE_A)),
        &["invite", &thread, "--seat", &rig.b],
    )
    .data("activation invitation");
    thread
}

fn activation_warn(rig: &Rig) {
    use herdr_threads::{
        client::service::{PersistentServiceClient, ServiceIntentJournal},
        daemon::ownership::{read_descriptor, read_existing_namespace},
        protocol::{
            ids::{OperationId, SeatId, ThreadId},
            service::{
                EnsureManagedThread, InvitationConstraint, NotificationSeverity, ServiceInvite,
                ServiceNotify, ServiceOperation,
            },
            time::CallBudget,
        },
    };
    let descriptor = read_descriptor(
        &rig.paths,
        read_existing_namespace(&rig.paths).unwrap().unwrap(),
    )
    .unwrap();
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let client = PersistentServiceClient::new(
        descriptor.endpoint,
        clock.clone(),
        descriptor.instance_uuid,
        Some(descriptor.boot_id),
        ServiceIntentJournal::open(rig.root.join("activation-intents")).unwrap(),
    );
    let budget = || CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0 + 10_000),
        cancellation: Cancellation::default(),
    };
    let thread = ThreadId::new("activation-warn");
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            client.register(&budget()).await.unwrap();
            client
                .submit(
                    ServiceOperation::EnsureThread(EnsureManagedThread {
                        thread: thread.clone(),
                        topic: "activation Warn".into(),
                        goal: "observe canonical notice".into(),
                        operation: OperationId::new("ensure"),
                    }),
                    &budget(),
                )
                .await
                .unwrap();
            client
                .submit(
                    ServiceOperation::Invite(ServiceInvite {
                        thread: thread.clone(),
                        seat: SeatId::new(rig.b.clone()),
                        constraint: InvitationConstraint::Ordinary,
                        deadline_millis: Some(300_000),
                        operation: OperationId::new("invite"),
                    }),
                    &budget(),
                )
                .await
                .unwrap();
            rig.cli(Some((&rig.b, PANE_B)), &["accept", thread.as_str()])
                .data("accept Warn thread");
            client
                .submit(
                    ServiceOperation::Notify(ServiceNotify {
                        thread: thread.clone(),
                        severity: NotificationSeverity::Warn,
                        event_json: json!({"kind":"activation", "detail":"actual Warn notice"}),
                        operation: OperationId::new("notify"),
                    }),
                    &budget(),
                )
                .await
                .unwrap();
        });
    wait_until("the actual Warn notice projection", STEP, || {
        let count: i64 = rig.db().query_row(
            "SELECT count(*) FROM digest_programmatic_warnings WHERE seat_id=?1 AND thread_id=?2",
            [&rig.b, thread.as_str()], |row| row.get(0),
        ).unwrap();
        (count > 0).then_some(())
    });
}

/// Every selected page is observed on the real child's stdout before its exact
/// Confirm reaches the daemon. Removing advertisement must fail at discovery.
#[test]
fn activation_v2_elected_watch_capabilities_lease_output_confirm() {
    let rig = Rig::activation();
    activation_warn(&rig);
    let invitation = activation_invitation(&rig);
    let lazy = rig.send_lazy("activation passive lazy");
    let ordinary = rig.send_ordinary("activation ordinary ACK candidate");
    let relay = ActivationRelay::start(&rig, true, ConfirmFault::None);
    let mut watch = rig.watch(SESSION_B);
    watch.wait_connected();
    let lease = activation_owner(&relay);
    let confirm = relay.control_confirm(false);
    let page = activation_page(&relay, &confirm);
    let token = confirm.request["command"]["args"]["token"]
        .as_str()
        .unwrap();
    assert_eq!(
        confirm.request["command"]["args"]["emitted_controls"],
        page["emitted_controls"]
    );
    assert!(!page["emitted_controls"].as_array().unwrap().is_empty());
    assert_eq!(page["render"]["kind"], "watch_json_lines_v1");
    let items = page["items"].as_array().unwrap();
    assert!(!items.is_empty() && items.len() <= 32);
    for kind in ["message", "invitation", "notice", "attention"] {
        assert!(
            items.iter().any(|item| item["kind"] == kind),
            "missing {kind}: {page}"
        );
    }
    assert!(
        items
            .iter()
            .any(|item| item["id"] == lazy && item["lazy"] == true)
    );
    assert!(
        items
            .iter()
            .any(|item| item["id"] == ordinary && item["lazy"] == false)
    );
    watch.wait_attention_marker();
    let bytes = watch.stdout.lock().unwrap().clone();
    let emitted: Vec<&[u8]> = bytes
        .split_inclusive(|byte| *byte == b'\n')
        .skip(1)
        .collect();
    assert_eq!(
        emitted.len(),
        items.len(),
        "one ordered line per captured item"
    );
    assert_eq!(
        emitted.iter().map(|line| line.len()).sum::<usize>(),
        page["output_bytes"].as_u64().unwrap() as usize
    );
    assert!(page["output_bytes"].as_u64().unwrap() <= 65_536);
    for (item, bytes) in items.iter().zip(emitted) {
        let line: Value = serde_json::from_slice(bytes).unwrap();
        match item["kind"].as_str().unwrap() {
            "message" => {
                assert_eq!(line["id"], item["id"]);
                assert_eq!(
                    line["kind"],
                    if item["lazy"] == true {
                        "lazy"
                    } else {
                        "message"
                    }
                );
                assert_eq!(line["body"], item["content"]["body"]);
                assert_eq!(line["ack_required"], item["content"]["ack_required"]);
            }
            "invitation" => {
                assert_eq!(line["kind"], "invitation");
                assert_eq!(
                    line["id"],
                    format!("invitation:{}", item["id"].as_str().unwrap())
                );
                assert_eq!(line["body"], item["content"]["body"]);
                // Every agent invitation now requires acceptance, so it wakes.
                assert_eq!(line["wakes"], true);
            }
            "notice" => {
                assert_eq!(line["kind"], "notice");
                assert_eq!(line["warning"], item["warning"]);
                assert_eq!(line["body"], item["body"]);
                assert_eq!(line["thread"], "activation-warn");
            }
            "attention" => {
                assert_eq!(line["kind"], "attention");
                assert_eq!(line["presentation_epoch"], item["epoch"]);
                assert_eq!(line["attention_version"], item["attention_version"]);
            }
            other => panic!("unexpected initial activation item {other}"),
        }
    }
    // The real Pull settles its carried notice for this exact occupant before
    // replying, independently of the still-held presentation Confirm. Session
    // identity is deliberately different from execution identity in this rig.
    let notice = items.iter().find(|item| item["kind"] == "notice").unwrap();
    let claim = &confirm.request["command"]["args"]["claim"];
    assert_ne!(claim["execution"], claim["native_session"]);
    let (offered_generation, offered_execution, frontier, notice_ordinal, binding_generation, binding_execution):
        (i64, String, i64, i64, i64, String) = rig.db().query_row(
            "SELECT offered.generation,offered.execution_id,o.offered_ordinal,d.ordinal,b.generation,b.execution_id FROM digest_notice_offer o JOIN occupant_bindings offered ON offered.binding_id=o.binding_id AND offered.seat_id=o.seat_id JOIN digest_programmatic_warnings d ON d.seat_id=o.seat_id JOIN occupant_bindings b ON b.seat_id=o.seat_id AND b.harness='claude' AND b.native_session=?3 AND b.agent_id='' AND b.role='top_level' AND b.ended_at IS NULL AND b.liveness='connected' AND b.binding_id=o.binding_id WHERE o.seat_id=?1 AND d.warning_id=?2",
            [rig.b.as_str(), notice["warning"].as_str().unwrap(), claim["native_session"].as_str().unwrap()],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?)),
        ).unwrap();
    assert_eq!(offered_generation, binding_generation);
    assert_eq!(
        offered_generation as u64,
        claim["binding_generation"].as_u64().unwrap()
    );
    assert_eq!(offered_execution, binding_execution);
    assert_eq!(offered_execution, claim["execution"].as_str().unwrap());
    assert!(
        frontier >= notice_ordinal,
        "the actual carried notice is covered by the current occupant's frontier"
    );
    assert_eq!(
        activation_pending(&rig).0.as_deref(),
        Some(token),
        "stdout alone does not retire outbox"
    );
    assert_eq!(rig.receipt_state(&ordinary), "pending");
    assert_eq!(rig.lazy_state(&lazy), "pending");
    let before = activation_pending(&rig);
    let unseen = relay.replay(&confirm, |request| {
        request["command"]["args"]["emitted_controls"][0]["epoch"] = json!("18446744073709551615");
    });
    assert_eq!(
        unseen["result"]["Err"]["code"], "invalid_request",
        "{unseen}"
    );
    assert_eq!(
        activation_pending(&rig),
        before,
        "invalid subset cannot retire unseen output"
    );
    let mark = rig.host.mark();
    relay.release();
    let committed = relay.control_confirm(true);
    assert_eq!(committed.request, confirm.request);
    activation_confirm_result(
        committed.reply.as_ref().unwrap(),
        "confirmed",
        &page["emitted_controls"],
    );
    // The upstream response proves the transaction finished. Its unchanged
    // bytes are still held downstream, so this child cannot Pull a successor
    // that would replace last_token before this exact canonical observation.
    let (current, last, mask) = activation_pending(&rig);
    assert_eq!(
        (current, last.as_deref(), mask),
        (
            None,
            Some(token),
            (1i64 << page["emitted_controls"].as_array().unwrap().len()) - 1,
        ),
        "canonical retirement of the matched token/control subset; {}",
        activation_retirement_diagnostics(&rig, &relay)
    );
    assert_eq!(
        rig.receipt_state(&ordinary),
        "pending",
        "presentation is not ACK"
    );
    assert_eq!(
        rig.lazy_state(&lazy),
        "pending",
        "presentation is not lazy consumption"
    );
    let pending: i64 = rig
        .db()
        .query_row(
            "SELECT count(*) FROM digest_pending_invitations WHERE seat_id=?1 AND thread_id=?2",
            [&rig.b, invitation.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pending, 1, "presentation is not acceptance");
    rig.assert_no_prompt(mark, QUIET);
    relay.release_response();
    // ACK and acceptance are later explicit actions; the real Watch then emits
    // a scoped clear. Keep these separate from presentation's passive effects.
    assert_eq!(
        rig.ack_result(SESSION_B, "context", &ordinary)["result"],
        "settled"
    );
    rig.cli(Some((&rig.b, PANE_B)), &["accept", &invitation])
        .data("accept activation invitation");
    let clear = watch.wait_line("the active scoped clear", STEP, |line| {
        line["kind"] == "attention_cleared"
    });
    let attention = items
        .iter()
        .find(|item| item["kind"] == "attention")
        .unwrap();
    assert_eq!(
        clear["presentation_epoch"], attention["epoch"],
        "the real clear closes the captured attention epoch"
    );
    assert_eq!(
        watch
            .seen
            .iter()
            .filter(|line| line["kind"] == "notice" && line["warning"] == notice["warning"])
            .count(),
        1,
        "subsequent real Watch Pulls do not re-emit the settled notice"
    );
    assert_eq!(rig.channel_state().as_deref(), Some("live"));
    assert_eq!(lease.len(), 32);
    if let Some(path) = std::env::var_os("HT_ACTIVATION_CAPTURE_PATH") {
        // Optional downstream replay evidence. Correctness never reads this file.
        let bytes = watch.stdout.lock().unwrap().clone();
        assert!(bytes.len() <= 65_536 * 2);
        fs::write(path, bytes).unwrap();
    }
}

fn capacity_normal_snapshot(rig: &Rig) -> Value {
    rig.db().query_row(
        "SELECT ordinary_pending,ordinary_partial,ordinary_cursor,ordinary_skip_body,ordinary_done,ordinary_version,ordinary_scan_passive_generation FROM invitation_presentation_consumers WHERE seat_id=?1 AND consumer='watch'",
        [&rig.b], |row| Ok(json!({
            "carry": row.get::<_, String>(0)?,
            "partial": row.get::<_, String>(1)?,
            "cursor": row.get::<_, Option<String>>(2)?,
            "skip": row.get::<_, Option<String>>(3)?,
            "done": row.get::<_, i64>(4)?,
            "version": row.get::<_, String>(5)?,
            "passive_generation": row.get::<_, i64>(6)?,
        })),
    ).unwrap()
}

/// Kills: a retained unknown ordinary carry makes a full real Watch issue
/// empty Page.more Pulls forever instead of entering its channel stream wait.
#[test]
fn activation_v2_watch_full_pending_capacity_parks_until_real_channel_input() {
    let rig = Rig::activation();
    // UTC governs offers; monotonic request budgets still advance normally.
    rig.clock.freeze_utc();
    let flow = Arc::new(CapacityFlow::default());
    let relay =
        ActivationRelay::start_with_capacity(&rig, false, ConfirmFault::None, Some(flow.clone()));
    // This body admits roughly ten items per 64KiB page, so the final normal
    // source slice crosses the 256-slot boundary with ID257 still carried.
    // Its 6000 bytes fit the ordinary 8192-byte body limit without truncation.
    let body = "c".repeat(6000);
    let first = rig.send_ordinary(&body);
    let seed = relay
        .snapshot()
        .into_iter()
        .find(|entry| entry.request["command"]["kind"] == "send_message")
        .expect("the real sender CLI's canonical SendMessage request");
    let mut sent = vec![first];
    // Canonical RPC seeding reuses the real sender's recorded cooperative
    // claim, with fresh operation/request IDs. It does not seed SQL/ledger
    // state or participate in the selected Watch discovery/output positive.
    for _ in 1..257 {
        let response = relay.replay(&seed, |request| {
            request["command"]["args"]["operation"] = json!(uuid::Uuid::new_v4().to_string());
        });
        assert_eq!(
            response["result"]["Ok"]["kind"], "message_sent",
            "{response}"
        );
        sent.push(
            response["result"]["Ok"]["data"]
                .as_str()
                .unwrap()
                .to_owned(),
        );
    }
    assert_eq!(
        sent.iter().collect::<std::collections::BTreeSet<_>>().len(),
        257
    );
    let watch_start = relay.snapshot().len();
    let mut watch = rig.watch(SESSION_B);
    watch.wait_connected();
    activation_owner_from(&relay, watch_start);
    let mut offered = std::collections::BTreeSet::new();
    let mut terminal = None;
    // Observation budget for this fixed 257-row fixture, not a client drain
    // limit: ~26 output pages plus bounded publication/maintenance quanta.
    // Every next observation is a real request, not a wall-time RPC-rate test.
    for ordinal in 1..=64 {
        let index = flow.wait_reply(ordinal);
        let exchange = relay.snapshot()[index].clone();
        let reply = &exchange.reply.as_ref().unwrap()["result"]["Ok"];
        assert_eq!(reply["kind"], "delivery_pulled_v2");
        let result = &reply["data"];
        let more = match result["kind"].as_str().unwrap() {
            "page" => {
                let page = &result["args"];
                assert!(page["items"].as_array().unwrap().len() <= 32);
                assert!(page["output_bytes"].as_u64().unwrap() <= 65_536);
                assert!(page["truncated"].as_array().unwrap().is_empty());
                for id in page["offered"].as_array().unwrap() {
                    let id = id.as_str().unwrap().to_owned();
                    assert_ne!(
                        id, sent[256],
                        "full capacity never offers the unknown identity"
                    );
                    assert!(
                        offered.insert(id),
                        "frozen UTC forbids an early reoffer/history reset"
                    );
                }
                page["more"].as_bool().unwrap()
            }
            "unchanged" => false,
            other => panic!("the real current owner refused its capacity drain: {other}"),
        };
        if !more {
            terminal = Some((ordinal, index, Instant::now()));
            // The terminal response is still held: all source effects above
            // are actual daemon decisions, before any channel frame is released.
            break;
        }
        flow.release_reply(ordinal);
    }
    let (terminal, terminal_index, terminal_held_at) = terminal.expect(
        "full pending capacity must reach a terminal reply; empty Page.more is not runnable work",
    );
    assert_eq!(offered, sent[..256].iter().cloned().collect());
    let terminal_exchange = relay.snapshot()[terminal_index].clone();
    let terminal_result = &terminal_exchange.reply.as_ref().unwrap()["result"]["Ok"]["data"];
    let terminal_page = &terminal_result["args"];
    let terminal_shape = json!({
        "ordinal": terminal,
        "kind": terminal_result["kind"],
        "more": terminal_page["more"],
        "token": terminal_page["presentation_token"],
        "item_count": terminal_page["items"].as_array().map(Vec::len),
        "target_offered": terminal_page["offered"].as_array().is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(sent[255].as_str()))),
        "target_item": terminal_page["items"].as_array().is_some_and(|items| items.iter().any(|item| item["id"] == sent[255])),
        "first_ids": terminal_page["items"].as_array().map(|items| items.iter().take(8).map(|item| item["id"].as_str().map(|id| id.chars().take(32).collect::<String>())).collect::<Vec<_>>()),
    });
    let held_before_release = terminal_held_at.elapsed();
    flow.release_reply(terminal);
    let output_diagnostic = |watch: &mut Watch| {
        let child = format!("{:?}", watch.child.try_wait());
        let state = flow.lock();
        let flow_state = json!({
            "requests": state.requests.len(), "ready": state.ready.len(), "released": state.released,
            "attention_seen": state.attention_seen, "attention_forwarded": state.attention_forwarded.len(),
            "terminal_write": state.reply_writes.iter().find(|(ordinal, _)| *ordinal == terminal).map(|(_, error)| json!({"ok": error.is_none(), "error_kind": error})),
            "writes": state.reply_writes.len(),
            "failure": state.failure.as_deref().map(|text| text.chars().take(128).collect::<String>()),
        });
        drop(state);
        let compact = |line: &Value| {
            json!({
                "kind": line["kind"].as_str().map(|s| s.chars().take(24).collect::<String>()),
                "id": line["id"].as_str().map(|s| s.chars().take(32).collect::<String>()),
                "state": line["state"].as_str().map(|s| s.chars().take(24).collect::<String>()),
                "reason": line["reason"].as_str().map(|s| s.chars().take(64).collect::<String>()),
                "epoch": line["presentation_epoch"], "version": line["attention_version"],
            })
        };
        json!({
            "child_try_wait": child,
            "flow": flow_state,
            "seen_count": watch.seen.len(),
            "target_seen": watch.seen.iter().any(|line| line["id"] == sent[255]),
            "seen_tail": watch.seen.iter().rev().take(4).map(compact).collect::<Vec<_>>(),
            "status_tail": watch.seen.iter().rev().filter(|line| matches!(line["kind"].as_str(), Some("status" | "attention" | "attention_cleared"))).take(4).map(compact).collect::<Vec<_>>(),
            "stderr": diagnostic_text(&watch.stderr(), 256),
        })
    };
    let before_output_wait = output_diagnostic(&mut watch);
    let output_wait =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| watch.wait_item(&sent[255])));
    if let Err(failure) = output_wait {
        let mut diagnostic = format!(
            "capacity output diagnostic: terminal={} hold_ms={} phase=terminal_output_before_barrier before={} after={}",
            terminal_shape,
            held_before_release.as_millis(),
            before_output_wait,
            output_diagnostic(&mut watch),
        );
        // A separate retained record survives daemon stderr ownership and the
        // generic panic's body-heavy prefix. Cap this record at four KiB.
        if diagnostic.len() > 4096 {
            diagnostic.truncate(diagnostic.floor_char_boundary(4084));
            diagnostic.push_str("[truncated]");
        }
        rig.panics
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(diagnostic);
        std::panic::resume_unwind(failure);
    }
    // Deliver the terminal reply before unrelated setup work can exhaust its
    // RPC budget. Keep Attention credits at zero while the registered channel
    // observes real background commits. Finish every send_attention job before
    // forwarding the real frame, so it cannot restart a nonempty warning-source
    // sweep. Include bootstrap jobs; no send/notify follows, and UTC stays frozen.
    let attention_work_diagnostic = || {
        let db = rig.db();
        let metadata = |sql: &str| -> Value {
            match db.query_row(sql, [], |row| row.get::<_, String>(0)) {
                Ok(raw) => serde_json::from_str(&raw)
                    .unwrap_or_else(|error| json!({"decode_error": error.to_string()})),
                Err(error) => json!({"query_error": error.to_string()}),
            }
        };
        json!({
            "utc": rig.clock.utc_now().0,
            "monotonic": rig.clock.monotonic_now().0,
            "deadline_idle_events": rig.probe.registered_idle_events(Lane::Deadlines),
            "deadline_health": rig.probe.lane_health(Lane::Deadlines).map(|detail| diagnostic_text(&detail, 512)),
            "by_status": metadata("SELECT json_group_array(json_object('kind',kind,'status',status,'count',n,'min_ordinal',lo,'max_ordinal',hi,'min_position',pmin,'max_position',pmax,'min_high_water',hmin,'max_high_water',hmax,'min_completed_units',umin,'max_completed_units',umax,'last_error',error)) FROM (SELECT kind,status,count(*) n,min(ordinal) lo,max(ordinal) hi,min(position) pmin,max(position) pmax,min(high_water) hmin,max(high_water) hmax,min(completed_units) umin,max(completed_units) umax,substr(max(last_error),1,128) error FROM work_jobs GROUP BY kind,status ORDER BY kind,status LIMIT 18)"),
            "incomplete": metadata("SELECT json_group_array(json_object('id',substr(id,1,128),'ordinal',ordinal,'kind',kind,'status',status,'position',position,'high_water',high_water,'completed_units',completed_units,'last_error',substr(last_error,1,128))) FROM (SELECT id,ordinal,kind,status,position,high_water,completed_units,last_error FROM work_jobs WHERE kind='send_attention' AND status<>'complete' ORDER BY ordinal LIMIT 20)"),
            // Per-job retry deadlines are private monotonic driver state,
            // not work_jobs columns. Lane health is the exposed retry view.
        })
    };
    let attention_work_before = attention_work_diagnostic();
    let mut last_kicked_idle = None;
    let attention_work_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        wait_until(
            "all fixture send_attention jobs to complete before forwarding Attention",
            STEP,
            || {
                let (total, complete, outstanding): (i64, i64, i64) = rig.db().query_row(
                "SELECT count(*),COALESCE(sum(status='complete'),0),COALESCE(sum(status<>'complete'),0) FROM work_jobs WHERE kind='send_attention'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            ).unwrap();
                if outstanding == 0 {
                    return Some((total, complete));
                }
                // Setup-only stepping: completed small jobs otherwise wait
                // for the next safety tick. One ordinary kick per observed
                // finished pass keeps real worker quanta and backoff intact.
                let idle = rig.probe.registered_idle_events(Lane::Deadlines);
                if last_kicked_idle != Some(idle) {
                    last_kicked_idle = Some(idle);
                    rig.probe.kick_registered(Lane::Deadlines);
                }
                None
            },
        )
    }));
    let (attention_jobs, completed_jobs) = attention_work_result.unwrap_or_else(|failure| {
        let diagnostic = format!(
            "capacity attention-work timeout diagnostic={}",
            json!({
                "before": attention_work_before,
                "after": attention_work_diagnostic(),
            })
        );
        // The daemon owns stderr until Rig::drop restores it. Preserve this
        // metadata alongside the fixture's captured original panic instead.
        rig.panics
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(diagnostic);
        std::panic::resume_unwind(failure)
    });
    assert_eq!(attention_jobs, completed_jobs);
    assert!(
        attention_jobs >= sent.len() as i64,
        "the terminal source barrier includes every seeded send, not an empty projection"
    );
    // Diagnostic-only snapshots before forwarding real Attention. Fixed
    // metadata and bounded descriptor identities omit bodies.
    let diagnostic_state = || {
        let db = rig.db();
        let metadata = |sql: &str| -> Value {
            match db.query_row(sql, [&rig.b], |row| row.get::<_, String>(0)) {
                Ok(raw) => serde_json::from_str(&raw)
                    .unwrap_or_else(|error| json!({"decode_error": error.to_string()})),
                Err(error) => json!({"query_error": error.to_string()}),
            }
        };
        json!({
            "consumer": metadata("SELECT json_object('control',substr(control_state,1,2048),'outbox_more',outbox_more,'token',outbox_token,'retired',outbox_retired_mask,'confirmed',outbox_confirmed_mask,'descriptor_count',json_array_length(outbox_descriptors),'descriptors',json((SELECT json_group_array(json_object('kind',json_extract(value,'$.item.kind'),'id',json_extract(value,'$.item.id'),'warning',json_extract(value,'$.item.warning'),'epoch',json_extract(value,'$.item.epoch'),'version',json_extract(value,'$.item.attention_version'))) FROM (SELECT value FROM json_each(outbox_descriptors) LIMIT 32))),'publication_after',ordinary_publication_after,'publication_high',ordinary_publication_high,'passive_generation',ordinary_passive_generation,'initial_after',initial_after,'initial_high',initial_high,'context_after',context_after_seq,'context_high',context_high,'context_thread',substr(context_thread,1,128),'context_invitation_after',context_invitation_after,'context_invitation_high',context_invitation_high,'dirty_after',dirty_after,'dirty_high',dirty_high,'revisit_after_bytes',revisit_after_bytes,'revisit_after_ordinal',revisit_after_ordinal,'revisit_high_bytes',revisit_high_bytes,'revisit_high_ordinal',revisit_high_ordinal,'ordinary_done',ordinary_done,'ordinary_version',ordinary_version,'carry_count',json_array_length(ordinary_pending,'$.items'),'partial_bytes',length(ordinary_partial),'rotation',rotation) FROM invitation_presentation_consumers WHERE seat_id=?1 AND consumer='watch'"),
            "relevance": metadata("SELECT COALESCE((SELECT json_object('fence',json_extract(proof_json,'$.fence'),'frontier',json_extract(proof_json,'$.frontier'),'native_version',json_extract(proof_json,'$.native_version'),'absent',json_extract(proof_json,'$.absent'),'witness',substr(COALESCE(json_extract(proof_json,'$.witness'),'null'),1,512),'slots',substr(COALESCE(json_extract(proof_json,'$.slots'),'null'),1,8192),'source_rotation',json_extract(proof_json,'$.source_rotation'),'fallback_rotation',json_extract(proof_json,'$.fallback_rotation')) FROM presentation_warning_proofs WHERE seat_id=?1 AND consumer='watch'),'{}')"),
            "canonical": metadata("SELECT json_object('fence',(SELECT version FROM presentation_warning_fence WHERE singleton=1),'notice_frontier',(SELECT offered_ordinal FROM digest_notice_offer WHERE seat_id=?1),'confirming_invitations',(SELECT count(*) FROM presentation_confirming_invitations WHERE seat_id=?1),'pending_invitations',(SELECT count(*) FROM digest_pending_invitations WHERE seat_id=?1),'physical_pending_receipts',(SELECT count(*) FROM receipts WHERE seat_id=?1 AND state='pending'),'host_decision_max',(SELECT max(decision_seq) FROM host_instances))")
        })
    };
    let diagnostic_before = diagnostic_state();
    let before = capacity_normal_snapshot(&rig);
    let carry: Value = serde_json::from_str(before["carry"].as_str().unwrap()).unwrap();
    let retained = carry["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["content"]["id"] == sent[256])
        .expect("ID257 is retained in the actual canonical normal carry");
    assert_eq!(retained["content"]["content"]["body"], body);
    for id in &sent {
        assert_eq!(rig.receipt_state(id), "pending");
    }
    assert_eq!(rig.mod_ack_events(), 0);
    // No real Attention was forwarded during the initial finite drain. The
    // next Pull is attributable to this explicit release, not continuation.
    flow.release_attention_after(terminal);
    let resumed = terminal + 1;
    let index = flow.wait_reply(resumed);
    flow.assert_resumed_after_attention(resumed);
    let next = relay.snapshot()[index].reply.clone().unwrap();
    let result = &next["result"]["Ok"]["data"];
    match result["kind"].as_str().unwrap() {
        "unchanged" => {}
        "page" => {
            let page = &result["args"];
            assert_eq!(
                page["more"],
                false,
                "stable full-capacity response must return to stream waiting; diagnostic={}",
                json!({
                    "terminal_ordinal": terminal,
                    "resumed_ordinal": resumed,
                    "reply": {
                        "kind": result["kind"], "more": page["more"],
                        "offered_count": page["offered"].as_array().map(Vec::len),
                        "truncated_count": page["truncated"].as_array().map(Vec::len),
                        "token": page["presentation_token"],
                        "attention_version": page["attention_version"],
                        "output_bytes": page["output_bytes"],
                        "items": page["items"].as_array().map(|items| items.iter().take(32).map(|item| json!({
                            "kind": item["kind"], "id": item["id"], "warning": item["warning"],
                            "epoch": item["epoch"], "version": item["attention_version"]
                        })).collect::<Vec<_>>()),
                        "controls": page["emitted_controls"].as_array().map(|controls| controls.iter().take(32).cloned().collect::<Vec<_>>()),
                    },
                    "before": diagnostic_before,
                    "after": diagnostic_state(),
                })
            );
            assert_eq!(page["offered"], json!([]));
            assert_eq!(page["truncated"], json!([]));
            assert_eq!(page["items"], json!([]));
        }
        other => panic!("real channel-triggered Pull refused: {other}"),
    }
    assert_eq!(
        capacity_normal_snapshot(&rig),
        before,
        "parking preserves exact normal cursor/carry/partial tuple"
    );
    flow.release_reply(resumed);
    for id in &sent {
        assert_eq!(rig.receipt_state(id), "pending");
    }
    assert!(watch.running());
    // Owned-child EOF still reaches the canonical registry through the relay.
    rig.kill(&mut watch);
    rig.wait_channel(Some("reconnect_grace"));
}

/// An actual advertised consumer still treats a lazy-only page as passive.
#[test]
fn activation_v2_watch_lazy_only_page_is_passive() {
    let rig = Rig::activation();
    let relay = ActivationRelay::start(&rig, false, ConfirmFault::None);
    let mut watch = rig.watch(SESSION_B);
    watch.wait_connected();
    let lease = activation_owner(&relay);
    let start = relay.snapshot().len();
    let mark = rig.host.mark();
    let lazy = rig.send_lazy("activation lazy-only page");
    let line = watch.wait_item(&lazy);
    assert_eq!(line["kind"], "lazy");
    assert_eq!(line["ack_required"], false);
    let (pull, page) = wait_until("the actual selected lazy-only V2 Pull page", STEP, || {
        // An already in-flight initial Pull may select the newly sent lazy
        // item, so identify its actual reply by ID rather than request index.
        relay.snapshot().into_iter().find_map(|entry| {
            let reply = entry.reply.as_ref()?;
            let result = &reply["result"]["Ok"];
            let page = &result["data"]["args"];
            (entry.request["command"]["kind"] == "delivery_pull_v2"
                && result["kind"] == "delivery_pulled_v2"
                && result["data"]["kind"] == "page"
                && page["items"]
                    .as_array()
                    .is_some_and(|items| items.iter().any(|item| item["id"] == lazy)))
            .then(|| (entry.request.clone(), page.clone()))
        })
    });
    assert_eq!(pull["command"]["args"]["presentation_lease"], lease);
    assert_eq!(page["items"].as_array().unwrap().len(), 1);
    // Ordinary/lazy output has no presentation descriptor or confirmation
    // token. Its successful stdout write does not request a Confirm RPC.
    assert!(page.get("presentation_token").is_none(), "{page}");
    assert_eq!(page["emitted_controls"], json!([]));
    assert!(activation_pending(&rig).0.is_none());
    assert_eq!(rig.lazy_state(&lazy), "pending");
    assert_eq!(rig.mod_ack_events(), 0);
    watch.assert_no_line("passive page attention", QUIET, |line| {
        line["kind"] == "attention"
    });
    assert_eq!(rig.host.prompts_since(mark, PANE_B), 0);
    // Crossing the health timeout cannot close a purely passive channel.
    rig.advance(11 * MINUTE_MS);
    assert_eq!(rig.channel_state().as_deref(), Some("live"));
    assert!(watch.running());
    rig.kill(&mut watch);
    rig.wait_channel(Some("reconnect_grace"));
    let log: Vec<_> = relay.snapshot().into_iter().skip(start).collect();
    assert!(
        !log.iter().any(|entry| matches!(
            entry.request["command"]["kind"].as_str(),
            Some(
                "delivery_presentation_confirm_v2"
                    | "ack"
                    | "ack_displayed"
                    | "ack_mod_delivered"
                    | "accept"
                    | "accept_required"
                    | "complete_inbox_delivery"
            )
        )),
        "lazy-only output requests no Confirm, ACK, acceptance or lazy completion"
    );
    assert!(activation_pending(&rig).0.is_none());
    assert_eq!(rig.lazy_state(&lazy), "pending");
    assert_eq!(rig.mod_ack_events(), 0);
    assert_eq!(rig.host.prompts_since(mark, PANE_B), 0);
}

/// A lost pre-dispatch Confirm cannot consume bytes; replacement must physically
/// emit the page and may only confirm using its own freshly accepted lease.
#[test]
fn activation_v2_watch_replacement_replays_uncommitted_output() {
    let rig = Rig::activation();
    // An actual programmatic Warn opens V2 relevance. Its independent setup
    // thread is accepted before the tested passive invitation even exists.
    activation_warn(&rig);
    let invitation = activation_invitation(&rig);
    // Unlike the informational Warn, a pending timed invitation stays relevant
    // after its contents are carried. Keep this independent from the passive
    // invitation whose exact uncommitted output must replay below.
    let attention_thread = rig
        .cli(
            Some((&rig.a, PANE_A)),
            &[
                "thread",
                "create",
                "--topic",
                "activation replacement attention",
            ],
        )
        .text("replacement attention thread");
    rig.cli(
        Some((&rig.a, PANE_A)),
        &["invite", &attention_thread, "--seat", &rig.b],
    )
    .data("replacement timed invitation");
    let confirming = || {
        rig.db().query_row(
        "SELECT count(*) FROM presentation_confirming_invitations p JOIN invitations i ON i.id=p.invitation_id AND i.seat_id=p.seat_id WHERE p.seat_id=?1 AND i.thread_id=?2 AND i.state='pending' AND i.confirmation='require_ack'",
        [&rig.b, attention_thread.as_str()], |row| row.get::<_, i64>(0),
    ).unwrap()
    };
    assert_eq!(
        confirming(),
        1,
        "real timed invitation opens canonical relevance"
    );
    // Ordinary mail is the no-receipt-ACK witness; it does not open the epoch.
    let ordinary = rig.send_ordinary("activation replacement ACK candidate");
    let relay = ActivationRelay::start(&rig, false, ConfirmFault::BeforeCommit);
    let mut old = rig.watch(SESSION_B);
    old.wait_connected();
    let old_lease = activation_owner(&relay);
    let old_confirm = relay.control_confirm(false);
    let page = activation_page(&relay, &old_confirm);
    old.wait_attention_marker();
    assert_eq!(old.exit_code(STEP), 1);
    let notice = page["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["kind"] == "notice")
        .unwrap();
    let covered: bool = rig.db().query_row(
        "SELECT d.ordinal<=COALESCE(o.offered_ordinal,0) FROM digest_programmatic_warnings d JOIN occupant_bindings b ON b.seat_id=d.seat_id AND b.harness='claude' AND b.native_session=?3 AND b.agent_id='' AND b.role='top_level' AND b.ended_at IS NULL AND b.liveness='connected' LEFT JOIN digest_notice_offer o ON o.seat_id=b.seat_id AND o.binding_id=b.binding_id WHERE d.seat_id=?1 AND d.warning_id=?2",
        [rig.b.as_str(), notice["warning"].as_str().unwrap(), old_confirm.request["command"]["args"]["claim"]["native_session"].as_str().unwrap()], |row| row.get(0),
    ).unwrap();
    assert!(
        covered,
        "original real Pull already settled the Warn for the canonical execution"
    );
    assert_eq!(
        confirming(),
        1,
        "replacement still has an independent open condition"
    );
    assert_eq!(
        activation_pending(&rig).0,
        Some(
            old_confirm.request["command"]["args"]["token"]
                .as_str()
                .unwrap()
                .to_owned()
        )
    );
    let mark = relay.snapshot().len();
    let mut replacement = rig.watch(SESSION_B);
    replacement.wait_connected();
    replacement.wait_attention_marker();
    let new_accepted = relay
        .snapshot()
        .into_iter()
        .skip(mark)
        .find(|entry| entry.request.get("watch").is_some())
        .unwrap();
    let new_lease = &new_accepted.reply.unwrap()["outcome"]["args"]["presentation_lease"];
    assert_ne!(new_lease, &json!(old_lease));
    let new_confirm = wait_until("replacement's own exact Confirm", STEP, || {
        relay.snapshot().into_iter().skip(mark).find(|entry| {
            entry.request["command"]["kind"] == "delivery_presentation_confirm_v2"
                && entry.reply.is_some()
        })
    });
    assert_eq!(
        &new_confirm.request["command"]["args"]["presentation_lease"],
        new_lease
    );
    assert_eq!(
        new_confirm.request["command"]["args"]["token"],
        old_confirm.request["command"]["args"]["token"],
        "replacement replays the original canonical page token"
    );
    assert_eq!(
        new_confirm.request["command"]["args"]["emitted_controls"],
        page["emitted_controls"]
    );
    activation_confirm_result(
        new_confirm.reply.as_ref().unwrap(),
        "confirmed",
        &page["emitted_controls"],
    );
    let stale = relay.replay(&old_confirm, |_| {});
    activation_confirm_result(&stale, "stale_binding", &json!([]));
    let invitation_bytes = |watch: &Watch| {
        watch
            .stdout
            .lock()
            .unwrap()
            .split_inclusive(|byte| *byte == b'\n')
            .filter(|bytes| {
                let line: Value = serde_json::from_slice(bytes).unwrap();
                line["kind"] == "invitation" && line["thread"] == invitation
            })
            .map(<[u8]>::to_vec)
            .collect::<Vec<_>>()
    };
    let original_bytes = invitation_bytes(&old);
    assert_eq!(
        original_bytes.len(),
        1,
        "original child physically emitted the passive invitation"
    );
    assert_eq!(
        invitation_bytes(&replacement),
        original_bytes,
        "replacement physically re-emits the exact raw invitation line"
    );
    assert!(
        replacement
            .seen
            .iter()
            .any(|line| line["kind"] == "invitation"
                && line["thread"] == invitation
                && line["wakes"] == true)
    );
    assert_eq!(
        rig.receipt_state(&ordinary),
        "pending",
        "owner replay is not receipt ACK"
    );
    assert_eq!(rig.mod_ack_events(), 0);
    let pending: i64 = rig
        .db()
        .query_row(
            "SELECT count(*) FROM digest_pending_invitations WHERE seat_id=?1 AND thread_id=?2",
            [&rig.b, invitation.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pending, 1, "owner replay is not invitation acceptance");
    assert_eq!(
        confirming(),
        1,
        "owner replay does not accept the timed source invitation"
    );
}

/// A committed response lost in transit permits exact idempotent recovery on
/// the still-current owner. Foreign controls and empty subsets invent no ACK.
#[test]
fn activation_v2_watch_lost_confirm_response_replays_exact_subset() {
    let rig = Rig::activation();
    // Reuse the actual Warn producer; its separate setup acceptance precedes
    // the passive invitation, which presentation/recovery never accepts.
    activation_warn(&rig);
    let invitation = activation_invitation(&rig);
    // Ordinary mail remains an independent no-receipt-ACK witness.
    let ordinary = rig.send_ordinary("activation lost-response ACK candidate");
    let relay = ActivationRelay::start(&rig, true, ConfirmFault::LoseReply);
    let mut watch = rig.watch(SESSION_B);
    watch.wait_connected();
    activation_owner(&relay);
    watch.wait_attention_marker();
    let confirm = relay.control_confirm(true);
    let controls = &confirm.request["command"]["args"]["emitted_controls"];
    activation_confirm_result(confirm.reply.as_ref().unwrap(), "confirmed", controls);
    assert!(
        watch.running(),
        "response held while the real child owns the lease"
    );
    let before = activation_pending(&rig);
    let exact = relay.replay(&confirm, |_| {});
    activation_confirm_result(&exact, "already_confirmed", controls);
    let subset = relay.replay(&confirm, |request| {
        request["command"]["args"]["emitted_controls"] = json!([]);
    });
    activation_confirm_result(&subset, "already_confirmed", &json!([]));
    let foreign = relay.replay(&confirm, |request| {
        request["command"]["args"]["emitted_controls"][0]["epoch"] = json!("18446744073709551615");
    });
    assert_eq!(
        foreign["result"]["Err"]["code"], "invalid_request",
        "{foreign}"
    );
    assert_eq!(
        activation_pending(&rig),
        before,
        "replay/subset/refusal cannot consume another page"
    );
    relay.release();
    assert_eq!(
        watch.exit_code(STEP),
        1,
        "lost completion remains truthful at consumer"
    );
    assert!(watch.seen.iter().any(|line| line["kind"] == "invitation"
        && line["thread"] == invitation
        && line["wakes"] == true));
    assert_eq!(
        rig.receipt_state(&ordinary),
        "pending",
        "presentation recovery is not receipt ACK"
    );
    assert_eq!(rig.mod_ack_events(), 0);
    let pending: i64 = rig
        .db()
        .query_row(
            "SELECT count(*) FROM digest_pending_invitations WHERE seat_id=?1 AND thread_id=?2",
            [&rig.b, invitation.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        pending, 1,
        "presentation recovery is not invitation acceptance"
    );
}

// -------------------------------------------------------------------- tests

/// Spec D4/D6 main flow: a message sent to a seat with a live channel is
/// pushed (Attention), `watch` drains and prints it, `watch ack --via context`
/// settles it, and the receipt records `cooperative_mod_delivery`.
#[test]
fn send_attention_drain_ack_settles_with_mod_delivery_provenance() {
    let rig = Rig::new();
    let mut watch = rig.watch(SESSION_B);
    watch.wait_connected();
    let id = rig.send_ordinary("hello through the mod");
    let line = watch.wait_item(&id);
    assert_eq!(line["kind"], "message");
    assert_eq!(line["body"], "hello through the mod");
    assert_eq!(line["ack_required"], true);
    assert_eq!(
        rig.receipt_state(&id),
        "pending",
        "streaming settles nothing"
    );
    let result = rig.ack_result(SESSION_B, "context", &id);
    assert_eq!(result["result"], "settled", "{result}");
    assert_eq!(rig.receipt_state(&id), "acked");
    let observation = rig.ack_observation(&id);
    assert_eq!(observation["action_provenance"], "cooperative_mod_delivery");
    assert_eq!(observation["provenance"], "cooperative_top_level");
    assert_eq!(observation["via"], "context");
    // The registration's audit line names the channel claim's provenance.
    let log = fs::read_to_string(rig.paths.instance_dir.join("daemon.log")).unwrap_or_default();
    assert!(
        log.contains("mod channel registered")
            && log.contains("provenance=cooperative_mod_channel"),
        "{log}"
    );
}

/// ht-j16.20: `setup claude` hands the mod the hooks' invocation. The argv the
/// installed `register.js` carries, with nothing added, reaches the rig's
/// daemon (non-default state dir and host endpoint, no `herdr-threads` on
/// PATH) for both `watch` and `watch ack`.
#[test]
fn setup_written_launch_runs_watch_with_no_extra_flags() {
    let rig = Rig::new();
    let config = rig.root.join("claude-config");
    fs::create_dir_all(&config).unwrap();
    fs::write(config.join("settings.json"), b"{}").unwrap();
    let mut setup = rig.command(None, &["setup", "claude"]);
    setup.env(
        herdr_threads::harness::claude_mod::TEST_MANAGED_SETTINGS_ENV,
        rig.root.join("no-managed-settings.json"),
    );
    let out = collect(
        setup
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn_owned()
            .unwrap()
            .wait_with_output()
            .unwrap(),
    );
    assert_eq!(out.code, 0, "setup claude: {}{}", out.stdout, out.stderr);
    let register =
        fs::read_to_string(rig.state.join("claude-mod/herdr-threads/hooks/register.js")).unwrap();
    let line = register
        .lines()
        .find(|line| line.starts_with("const LAUNCH = {"))
        .unwrap_or_else(|| panic!("no rendered launch in the installed register.js"));
    let json = line
        .strip_prefix("const LAUNCH = ")
        .and_then(|rest| rest.split_once(" // herdr-threads:launch"))
        .map(|(json, _)| json)
        .unwrap();
    let launch: Vec<String> = serde_json::from_str::<Value>(json).unwrap()["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(launch[0], BIN);
    assert_eq!(
        launch[1..],
        [
            "--state-dir",
            rig.state.to_str().unwrap(),
            "--host-endpoint",
            rig.socket.to_str().unwrap()
        ]
    );
    let with = |rest: &[&str]| -> Vec<String> {
        launch
            .iter()
            .cloned()
            .chain(rest.iter().map(|item| (*item).to_owned()))
            .collect()
    };

    // `watch`, as the mod spawns it (the SessionStart check-in may need a retry).
    let mut watch = (0..80)
        .find_map(|_| {
            let mut watch = Watch::spawn(rig.raw_command(
                PANE_B,
                &with(&["watch", "--harness", "claude", "--session", SESSION_B]),
            ));
            let first = watch.peek_first(STEP);
            let retry = first
                .as_ref()
                .is_some_and(|line| line["kind"] == "status" && line["reason"] == "no_binding");
            if retry {
                std::thread::sleep(Duration::from_millis(150));
                return None;
            }
            Some(watch)
        })
        .expect("the SessionStart check-in never committed for B");
    watch.wait_connected();
    rig.wait_channel(Some("live"));

    // `watch ack`, as the mod runs it.
    let id = rig.send_ordinary("through the setup-written launch");
    assert_eq!(
        watch.wait_item(&id)["body"],
        "through the setup-written launch"
    );
    let mut ack = rig.raw_command(
        PANE_B,
        &with(&[
            "watch",
            "ack",
            "--session",
            SESSION_B,
            "--via",
            "context",
            &id,
        ]),
    );
    let acked = collect(
        ack.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn_owned()
            .unwrap()
            .wait_with_output()
            .unwrap(),
    );
    assert_eq!(acked.code, 0, "{}{}", acked.stdout, acked.stderr);
    let result: Value = acked
        .stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|line| line["id"] == id.as_str())
        .unwrap_or_else(|| panic!("no result line for {id}: {}", acked.stdout));
    assert_eq!(result["result"], "settled", "{result}");
    assert_eq!(rig.receipt_state(&id), "acked");
}

/// Spec D4/D6 lazy rows: streamed as `lazy` (no ACK required), and the
/// `append` ack completes them as displayed; a repeat is idempotent.
#[test]
fn lazy_row_appended_then_completed() {
    let rig = Rig::new();
    let mut watch = rig.watch(SESSION_B);
    watch.wait_connected();
    let id = rig.send_lazy("passive hello");
    let line = watch.wait_item(&id);
    assert_eq!(line["kind"], "lazy");
    assert_eq!(line["ack_required"], false);
    assert_eq!(line["body"], "passive hello");
    assert_eq!(
        rig.lazy_state(&id),
        "pending",
        "streaming completes nothing"
    );
    let result = rig.ack_result(SESSION_B, "append", &id);
    assert_eq!(result["result"], "settled", "{result}");
    assert_eq!(rig.lazy_state(&id), "displayed");
    let again = rig.ack_result(SESSION_B, "append", &id);
    assert_eq!(again["result"], "already_settled", "{again}");
}

/// The accept CLI's live-channel grace (`HT_TEST_ACCEPT_CHANNEL_GRACE_MS`, test builds only).
const ACCEPT_GRACE_ENV: &str = "HT_TEST_ACCEPT_CHANNEL_GRACE_MS";

/// A pair opener from A to B, streamed to B's live watch before B accepts: (pair, id).
fn pair_opener_streamed(rig: &Rig, watch: &mut Watch, body: &str) -> (String, String) {
    rig.cli(
        Some((&rig.a, PANE_A)),
        &["send", "--seat", &rig.b, "--body", body],
    )
    .data("direct send");
    let (pair, id): (String, String) = rig
        .db()
        .query_row(
            "SELECT m.thread_id,m.id FROM messages m JOIN direct_threads d ON d.thread_id=m.thread_id WHERE m.body=?1",
            [body],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let line = watch.wait_item(&id);
    assert_eq!(line["kind"], "lazy", "{line}");
    assert_eq!(line["body"], body);
    (pair, id)
}

/// ht-br0 handshake, channel in time: B's live mod channel completes the opener (its `append`
/// ack) while B's text `accept` is still running. The accept waits for that completion within
/// its bounded grace (a generous bound here: the test ends on the completion, not the bound),
/// prints no opening body, and the watch streams the opener exactly once.
#[test]
fn text_accept_with_a_live_mod_channel_prints_nothing_when_the_channel_completes_in_time() {
    let rig = Rig::new();
    let mut watch = rig.watch(SESSION_B);
    watch.wait_connected();
    let (pair, id) = pair_opener_streamed(&rig, &mut watch, "pair opener via mod");
    let accept = std::thread::scope(|scope| {
        let accept =
            scope.spawn(|| rig.cli_text_b_env(&["accept", &pair], &[(ACCEPT_GRACE_ENV, "20000")]));
        // The channel appends and acks only once the accept committed, so the accept is inside
        // its grace (or has not peeked yet) when the completion lands.
        wait_until("B joined the pair", STEP, || {
            rig.db()
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM memberships WHERE thread_id=?1 AND seat_id=?2 AND state='joined')",
                    [pair.as_str(), rig.b.as_str()],
                    |row| row.get::<_, bool>(0),
                )
                .unwrap()
                .then_some(())
        });
        let result = rig.ack_result(SESSION_B, "append", &id);
        assert_eq!(result["result"], "settled", "{result}");
        accept.join().unwrap()
    });
    assert_eq!(accept.code, 0, "{}{}", accept.stdout, accept.stderr);
    assert!(accept.stdout.starts_with("accepted"), "{}", accept.stdout);
    assert!(
        !accept.stdout.contains("pair opener via mod"),
        "{}",
        accept.stdout
    );
    assert!(accept.stderr.is_empty(), "{}", accept.stderr);
    assert_eq!(rig.lazy_state(&id), "displayed");
    // Only lines after the first copy count (`seen` keeps the one `wait_item` returned).
    assert_eq!(
        watch.seen.iter().filter(|line| line["id"] == id).count(),
        1,
        "{:#?}",
        watch.seen
    );
    watch.seen.clear();
    watch.assert_no_line("a second copy of the opener", QUIET, |line| {
        line["id"] == id
    });
}

/// ht-br0 handshake, channel late: B's live watch streamed the opener but the mod never completes
/// it (not consuming, or slower than the grace). After its bounded grace (shortened here) B's
/// text `accept` shows the opener itself and settles it as its own text-inbox display; the
/// channel's late `append` ack then finds it already settled (at least once).
#[test]
fn text_accept_with_a_live_mod_channel_shows_the_opener_after_the_grace() {
    let rig = Rig::new();
    let mut watch = rig.watch(SESSION_B);
    watch.wait_connected();
    let (pair, id) = pair_opener_streamed(&rig, &mut watch, "pair opener the mod missed");
    let accept = rig.cli_text_b_env(&["accept", &pair], &[(ACCEPT_GRACE_ENV, "200")]);
    assert_eq!(accept.code, 0, "{}{}", accept.stdout, accept.stderr);
    assert!(accept.stdout.starts_with("accepted"), "{}", accept.stdout);
    assert_eq!(
        accept.stdout.matches("pair opener the mod missed").count(),
        1,
        "{}",
        accept.stdout
    );
    assert!(accept.stderr.is_empty(), "{}", accept.stderr);
    assert_eq!(rig.lazy_state(&id), "displayed");
    let late = rig.ack_result(SESSION_B, "append", &id);
    assert_eq!(late["result"], "already_settled", "{late}");
}

/// Spec D4: a body over 8 KiB streams as its first 8 KiB plus the marker with
/// `truncated: true`; the mod must not ack it, and when it does the ack is a
/// terminal refusal decided by the daemon from the stored length (the local
/// hint file is not needed); the receipt settles when the agent follows the
/// marker's own instruction (`body`, which is read-only, then `ack`). The
/// watch runs as the setup-written launch in the session-shell environment
/// and the agent runs the marker exactly as printed.
#[test]
fn truncated_body_streamed_with_marker_and_ack_refused_terminal() {
    let rig = Rig::new();
    let mut watch = rig.raw_watch(SESSION_B);
    watch.wait_connected();
    let id = rig.send_ordinary(&"t".repeat(9000));
    let line = watch.wait_item(&id);
    assert_eq!(line["truncated"], true, "{line}");
    assert_eq!(line["body_len"], 9000);
    let body = line["body"].as_str().unwrap();
    assert!(
        body[8 * 1024..].starts_with("…truncated; run "),
        "cut at the 8 KiB limit: {body}"
    );
    assert!(
        body.contains(&format!("--state-dir {}", rig.state.display())),
        "{body}"
    );
    assert!(body.contains(&format!(" body {id}")), "{body}");
    assert!(body.contains(&format!(" ack {id}")), "{body}");
    let result = rig.ack_result(SESSION_B, "context", &id);
    assert_eq!(result["result"], "refused_terminal", "{result}");
    assert_eq!(result["reason"], "truncated");
    // Without the local hint the daemon still refuses (stored body length).
    let contexts = rig.paths.instance_dir.join("contexts");
    for entry in fs::read_dir(&contexts).unwrap().flatten() {
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with("watch-truncated-")
        {
            fs::remove_file(entry.path()).unwrap();
        }
    }
    let result = rig.ack_result(SESSION_B, "context", &id);
    assert_eq!(result["result"], "refused_terminal", "{result}");
    assert_eq!(result["reason"], "truncated");
    assert_eq!(rig.receipt_state(&id), "pending");
    // Negative control: the bare command does not reach this instance.
    let bare = collect(
        rig.raw_command(PANE_B, &[BIN.to_owned(), "body".to_owned(), id.clone()])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn_owned()
            .unwrap()
            .wait_with_output()
            .unwrap(),
    );
    assert_ne!(bare.code, 0, "bare body: {}{}", bare.stdout, bare.stderr);
    // Follow the marker as streamed: `body` (read-only), then `ack`.
    let steps = rig.marker_steps(body);
    assert_eq!(steps.len(), 2, "{body}");
    let argv = &steps[0];
    let out = rig.run_marker_step(argv);
    assert_eq!(out.code, 0, "{argv:?}: {}{}", out.stdout, out.stderr);
    assert!(
        out.stdout.matches('t').count() >= 9000,
        "body returns the rest of the text: {argv:?}: {}{}",
        out.stdout,
        out.stderr
    );
    assert_eq!(rig.receipt_state(&id), "pending", "body is read-only");
    let argv = &steps[1];
    let out = rig.run_marker_step(argv);
    assert_eq!(out.code, 0, "{argv:?}: {}{}", out.stdout, out.stderr);
    assert_eq!(rig.receipt_state(&id), "acked");
}

/// Spec D4: a lazy row has no receipt, so a cut lazy row's marker names only
/// the read-only `body`.
#[test]
fn truncated_lazy_row_marker_names_only_body() {
    let rig = Rig::new();
    let mut watch = rig.raw_watch(SESSION_B);
    watch.wait_connected();
    let id = rig.send_lazy(&"l".repeat(9000));
    let line = watch.wait_item(&id);
    assert_eq!(line["kind"], "lazy", "{line}");
    assert_eq!(line["truncated"], true, "{line}");
    assert_eq!(line["body_len"], 9000);
    let body = line["body"].as_str().unwrap();
    assert!(
        body.contains(&format!("--state-dir {}", rig.state.display())),
        "{body}"
    );
    assert!(body.contains(&format!(" body {id}")), "{body}");
    assert!(!body.contains(" ack "), "{body}");
    let steps = rig.marker_steps(body);
    assert_eq!(steps.len(), 1, "{body}");
    let argv = &steps[0];
    let out = rig.run_marker_step(argv);
    assert_eq!(out.code, 0, "{argv:?}: {}{}", out.stdout, out.stderr);
    assert!(
        out.stdout.matches('l').count() >= 9000,
        "{argv:?}: {}{}",
        out.stdout,
        out.stderr
    );
    assert_eq!(rig.lazy_state(&id), "pending", "reading completes nothing");
}

/// ht-54r F1: a truncated ordinary message is refused by the mod's settlement, so the binding's
/// cursor stays behind it. The agent's full `body <id>` read is a whole display of it: the
/// calling binding's cursor passes it, so a later message the mod displays is no longer unread.
/// The read settles nothing; the explicit `ack` that follows does.
#[test]
fn full_body_read_of_truncated_message_unblocks_binding_cursor() {
    let rig = Rig::new();
    let mut watch = rig.raw_watch(SESSION_B);
    watch.wait_connected();
    let long = rig.send_ordinary(&"t".repeat(9000));
    let line = watch.wait_item(&long);
    assert_eq!(line["truncated"], true, "{line}");
    let refused = rig.ack_result(SESSION_B, "context", &long);
    assert_eq!(refused["result"], "refused_terminal", "{refused}");
    let (binding, _, _) = open_binding_of(&rig, SESSION_B).expect("SB binding");
    let seq_long = sequence_of(&rig, &long);
    assert!(
        cursor_of(&rig, &binding).is_none_or(|seq| seq < seq_long),
        "the refused display leaves the cursor behind it"
    );
    let steps = rig.marker_steps(line["body"].as_str().unwrap());
    let out = rig.run_marker_step(&steps[0]);
    assert_eq!(out.code, 0, "{:?}: {}{}", steps[0], out.stdout, out.stderr);
    assert!(
        !out.stdout.contains("more: "),
        "one full read: {}",
        out.stdout
    );
    assert_eq!(
        cursor_of(&rig, &binding),
        Some(seq_long),
        "a full body read moves the reading binding's cursor past the message"
    );
    assert_eq!(rig.receipt_state(&long), "pending", "body settles nothing");
    let later = rig.send_ordinary("after the long one");
    watch.wait_item(&later);
    let settled = rig.ack_result(SESSION_B, "context", &later);
    assert_eq!(settled["result"], "settled", "{settled}");
    let read_to = cursor_of(&rig, &binding);
    assert!(
        read_to.is_some_and(|seq| seq >= sequence_of(&rig, &later)),
        "the later message is read: the cursor is no longer stuck: {read_to:?}"
    );
    let out = rig.run_marker_step(&steps[1]);
    assert_eq!(out.code, 0, "{:?}: {}{}", steps[1], out.stdout, out.stderr);
    assert_eq!(rig.receipt_state(&long), "acked");
    assert!(
        cursor_of(&rig, &binding) >= read_to,
        "the explicit ACK of the fully read message never moves the cursor back"
    );
}

/// ht-54r F1: the mod displayed a later message whole (and settled it) while the cursor was held
/// behind the truncated one. That display ACK is this binding's own evidence, so the full
/// `body` read of the truncated message moves the cursor past both.
#[test]
fn full_body_read_passes_later_message_the_binding_already_displayed() {
    let rig = Rig::new();
    let mut watch = rig.raw_watch(SESSION_B);
    watch.wait_connected();
    let long = rig.send_ordinary(&"t".repeat(9000));
    let line = watch.wait_item(&long);
    let refused = rig.ack_result(SESSION_B, "context", &long);
    assert_eq!(refused["result"], "refused_terminal", "{refused}");
    let later = rig.send_ordinary("displayed while held");
    watch.wait_item(&later);
    let settled = rig.ack_result(SESSION_B, "context", &later);
    assert_eq!(settled["result"], "settled", "{settled}");
    let (binding, _, _) = open_binding_of(&rig, SESSION_B).expect("SB binding");
    assert!(
        cursor_of(&rig, &binding).is_none_or(|seq| seq < sequence_of(&rig, &long)),
        "held behind the truncated message"
    );
    let steps = rig.marker_steps(line["body"].as_str().unwrap());
    let out = rig.run_marker_step(&steps[0]);
    assert_eq!(out.code, 0, "{:?}: {}{}", steps[0], out.stdout, out.stderr);
    let read_to = cursor_of(&rig, &binding);
    assert!(
        read_to.is_some_and(|seq| seq >= sequence_of(&rig, &later)),
        "the full read passes the truncated message and the one already displayed: {read_to:?}"
    );
    assert_eq!(rig.receipt_state(&long), "pending", "body settles nothing");
}

/// ht-54r F1: like `inbox --json` and `inbox --machine`, a `--json` or `--machine` body read is
/// read-only, even when whole; only the text display moves the reading binding's cursor.
#[test]
fn machine_and_json_body_reads_do_not_advance_binding_cursor() {
    let rig = Rig::new();
    let mut watch = rig.raw_watch(SESSION_B);
    watch.wait_connected();
    let long = rig.send_ordinary(&"j".repeat(9000));
    let line = watch.wait_item(&long);
    let refused = rig.ack_result(SESSION_B, "context", &long);
    assert_eq!(refused["result"], "refused_terminal", "{refused}");
    let (binding, _, _) = open_binding_of(&rig, SESSION_B).expect("SB binding");
    let before = cursor_of(&rig, &binding);
    let body_step = rig.marker_steps(line["body"].as_str().unwrap()).remove(0);
    for flag in ["--json", "--machine"] {
        let mut argv = body_step.clone();
        argv.extend([
            flag.to_owned(),
            "--max-bytes".to_owned(),
            "65536".to_owned(),
        ]);
        let out = rig.run_marker_step(&argv);
        assert_eq!(out.code, 0, "{argv:?}: {}{}", out.stdout, out.stderr);
        assert!(
            out.stdout.matches('j').count() >= 9000,
            "whole: {}",
            out.stdout
        );
        assert_eq!(
            cursor_of(&rig, &binding),
            before,
            "{flag} body read is read-only"
        );
    }
    let out = rig.run_marker_step(&body_step);
    assert_eq!(out.code, 0, "{body_step:?}: {}{}", out.stdout, out.stderr);
    assert_eq!(cursor_of(&rig, &binding), Some(sequence_of(&rig, &long)));
}

/// ht-54r F1: only a full read advances. A first chunk, or a read that starts at an explicit
/// offset and reaches the end, leaves the cursor where it was; following the body's own
/// continuation to the end is a full read.
#[test]
fn partial_body_read_does_not_advance_binding_cursor() {
    let rig = Rig::new();
    let mut watch = rig.raw_watch(SESSION_B);
    watch.wait_connected();
    let long = rig.send_ordinary(&"p".repeat(9000));
    let line = watch.wait_item(&long);
    let refused = rig.ack_result(SESSION_B, "context", &long);
    assert_eq!(refused["result"], "refused_terminal", "{refused}");
    let (binding, _, _) = open_binding_of(&rig, SESSION_B).expect("SB binding");
    let before = cursor_of(&rig, &binding);
    let seq_long = sequence_of(&rig, &long);
    let body_step = rig.marker_steps(line["body"].as_str().unwrap()).remove(0);
    let mut first = body_step.clone();
    first.extend(["--max-bytes".to_owned(), "4096".to_owned()]);
    let out = rig.run_marker_step(&first);
    assert_eq!(out.code, 0, "{first:?}: {}{}", out.stdout, out.stderr);
    let more = out
        .stdout
        .lines()
        .find_map(|l| l.strip_prefix("more: "))
        .unwrap_or_else(|| panic!("a partial read names its continuation: {}", out.stdout))
        .to_owned();
    assert_eq!(
        cursor_of(&rig, &binding),
        before,
        "a first chunk is partial"
    );
    let mut tail = body_step.clone();
    tail.extend(["--offset".to_owned(), "8000".to_owned()]);
    let out = rig.run_marker_step(&tail);
    assert_eq!(out.code, 0, "{tail:?}: {}{}", out.stdout, out.stderr);
    assert!(!out.stdout.contains("more: "), "{}", out.stdout);
    assert_eq!(
        cursor_of(&rig, &binding),
        before,
        "a read from an explicit offset skipped the start"
    );
    let mut argv = shlex::split(&more).unwrap();
    for _ in 0..8 {
        let out = rig.run_marker_step(&argv);
        assert_eq!(out.code, 0, "{argv:?}: {}{}", out.stdout, out.stderr);
        match out.stdout.lines().find_map(|l| l.strip_prefix("more: ")) {
            Some(next) => {
                assert_eq!(cursor_of(&rig, &binding), before, "still partial");
                argv = shlex::split(next).unwrap();
            }
            None => break,
        }
    }
    assert_eq!(
        cursor_of(&rig, &binding),
        Some(seq_long),
        "following the continuation to the end is a full read"
    );
    assert_eq!(rig.receipt_state(&long), "pending", "body settles nothing");
}

/// Main review I1: on a `wake_on: all` seat the real watch child streams a lazy message as a
/// `message` line with no ACK and `wakes: true`, which the mod treats as idle-wake work; a plain
/// lazy row on the same seat before the opt-in streams as `lazy` with no `wakes`.
#[test]
fn wake_on_all_promoted_lazy_line_carries_wakes() {
    let rig = Rig::new();
    let mut watch = rig.raw_watch(SESSION_B);
    watch.wait_connected();
    let quiet = rig.send_lazy("before the opt-in");
    let line = watch.wait_item(&quiet);
    assert_eq!(line["kind"], "lazy", "{line}");
    assert!(line.get("wakes").is_none(), "{line}");
    rig.cli(Some((&rig.b, PANE_B)), &["wake-on", "all"])
        .data("wake-on");
    let loud = rig.send_lazy("after the opt-in");
    let line = watch.wait_item(&loud);
    assert_eq!(line["kind"], "message", "{line}");
    assert_eq!(line["ack_required"], false, "{line}");
    assert_eq!(line["wakes"], true, "{line}");
}

/// Spec D7: while a channel is live no native wake prompt is sent for the
/// seat, and the pending item stays pending until acked.
#[test]
fn live_channel_suppresses_native_wake() {
    let rig = Rig::new();
    let mut watch = rig.watch(SESSION_B);
    watch.wait_connected();
    let mark = rig.host.mark();
    let id = rig.send_ordinary("delivered through the channel");
    watch.wait_item(&id);
    rig.assert_no_prompt(mark, QUIET);
    assert_eq!(rig.receipt_state(&id), "pending");
    assert_eq!(rig.channel_state().as_deref(), Some("live"));
}

/// Spec D7 stall handover: with no mod ack for 10 minutes while an ordinary
/// receipt older than that is pending, the daemon closes the channel
/// (`stalled`), the native ladder resumes, and re-registration of that
/// binding generation is refused (`cooldown`, exit 2) for 10 minutes.
#[test]
fn stall_after_ten_minutes_hands_over_to_native_ladder_and_cooldown_refuses() {
    let rig = Rig::new();
    let mut watch = rig.watch(SESSION_B);
    watch.wait_connected();
    let id = rig.send_ordinary("never acked by the mod");
    watch.wait_item(&id);
    let mark = rig.host.mark();
    rig.advance(11 * MINUTE_MS);
    let closing = watch.wait_line("the stalled close", STEP, |line| {
        line["kind"] == "status" && line["state"] == "closing"
    });
    assert_eq!(closing["reason"], "stalled", "{closing}");
    assert_eq!(watch.exit_code(STEP), 0);
    rig.wait_channel(None);
    rig.wait_prompt(mark, PANE_B);
    // Cooldown: the same binding generation is refused with exit 2.
    let mut again = rig.watch(SESSION_B);
    let refusal = again.wait_line("the cooldown refusal", STEP, |line| {
        line["kind"] == "status" && line["state"] == "refused"
    });
    assert_eq!(refusal["reason"], "cooldown", "{refusal}");
    assert_eq!(refusal["exit"], 2);
    assert_eq!(again.exit_code(STEP), 2);
    // Once the cooldown has passed the channel can be opened again.
    rig.advance(10 * MINUTE_MS + 1_000);
    let mut third = rig.watch(SESSION_B);
    third.wait_connected();
    rig.wait_channel(Some("live"));
}

/// Spec D7: a truncated pending item is never counted toward the stall
/// predicate (the mod cannot ack it), so an old truncated receipt alone never
/// hands the seat back to the native ladder.
#[test]
fn truncated_item_never_counts_toward_stall() {
    let rig = Rig::new();
    let mut watch = rig.watch(SESSION_B);
    watch.wait_connected();
    let id = rig.send_ordinary(&"t".repeat(9000));
    watch.wait_item(&id);
    let mark = rig.host.mark();
    rig.advance(11 * MINUTE_MS);
    // A lazy row commits to an attention source, which kicks the worker at
    // once: the pass that streams it, and the sweep right after that pass,
    // run on the advanced clock. A stall would close the channel in that
    // sweep (or the next tick's), so no `closing` line before the row nor
    // within the window after it; a lazy row has no receipt, so it cannot
    // feed the stall predicate itself.
    let kick = rig.send_lazy("kicks the worker");
    let line = watch.wait_line("the kicking row or a close", STEP, |line| {
        line["id"] == kick.as_str() || line["state"] == "closing"
    });
    assert_ne!(line["state"], "closing", "the stall closed the channel");
    watch.assert_no_line("closing", QUIET, |line| line["state"] == "closing");
    assert!(
        watch.running(),
        "the watch was closed: {:#?} stderr: {}",
        watch.final_status(),
        watch.stderr()
    );
    assert_eq!(rig.channel_state().as_deref(), Some("live"));
    assert_eq!(rig.host.prompts_since(mark, PANE_B), 0);
}

/// Spec D7 reconnect grace: a dropped watch keeps the seat live for 30 s (no
/// native wake); when the grace expires without a registration the entry is
/// removed and the wake lane is kicked, so the native wake follows.
#[test]
fn drop_then_grace_expiry_kicks_native_wake() {
    let rig = Rig::new();
    let mut watch = rig.watch(SESSION_B);
    watch.wait_connected();
    rig.kill(&mut watch);
    rig.wait_channel(Some("reconnect_grace"));
    let mark = rig.host.mark();
    let id = rig.send_ordinary("sent while the watch is down");
    rig.assert_no_prompt(mark, QUIET);
    // The quiet window just observed already gave what was in flight time
    // to finish (it is longer than `Rig::advance`'s pause), so jump at once.
    rig.clock.advance(31_000);
    rig.wait_channel(None);
    rig.wait_prompt(mark, PANE_B);
    assert_eq!(rig.receipt_state(&id), "pending");
}

/// Spec D6/D7: a watch that re-registers within the grace keeps the native
/// wake suppressed, re-streams what is still pending, and its late ack
/// settles once; a repeat ack is `already_settled` and writes nothing.
#[test]
fn reconnect_within_grace_keeps_wake_suppressed_restreams_and_late_ack_settles_once() {
    let rig = Rig::new();
    let mut first = rig.watch(SESSION_B);
    first.wait_connected();
    let id = rig.send_ordinary("streamed, then the watch dies");
    first.wait_item(&id);
    rig.kill(&mut first);
    rig.wait_channel(Some("reconnect_grace"));
    assert_eq!(rig.receipt_state(&id), "pending", "a drop settles nothing");
    let mark = rig.host.mark();
    let mut second = rig.watch(SESSION_B);
    second.wait_connected();
    let line = second.wait_item(&id);
    assert_eq!(line["kind"], "message", "re-streamed after reconnect");
    rig.wait_channel(Some("live"));
    // The old grace window passes without a kick: the registration ended it.
    // A lazy row kicks the worker, so the sweep that would expire the old
    // grace (and kick the wake lane) runs on the advanced clock right after
    // the pass that streams the row.
    rig.advance(31_000);
    let kick = rig.send_lazy("kicks the worker");
    second.wait_item(&kick);
    rig.assert_no_prompt(mark, QUIET);
    assert_eq!(rig.channel_state().as_deref(), Some("live"));
    let late = rig.ack_result(SESSION_B, "context", &id);
    assert_eq!(late["result"], "settled", "{late}");
    let repeat = rig.ack_result(SESSION_B, "context", &id);
    assert_eq!(repeat["result"], "already_settled", "{repeat}");
    assert_eq!(rig.receipt_state(&id), "acked");
    assert_eq!(rig.mod_ack_events(), 1, "settled exactly once");
}

/// Spec D6: `retryable` (no live channel) keeps the item pending; once a
/// channel is registered and the next Attention arrives, the same ack settles.
#[test]
fn retryable_ack_retried_on_next_attention() {
    let rig = Rig::new();
    let id = rig.send_ordinary("acked before any channel exists");
    let early = rig.ack_result(SESSION_B, "context", &id);
    assert_eq!(early["result"], "retryable", "{early}");
    assert_eq!(early["reason"], "no_live_channel");
    assert_eq!(rig.receipt_state(&id), "pending");
    let mut watch = rig.watch(SESSION_B);
    watch.wait_connected();
    watch.wait_item(&id);
    let retried = rig.ack_result(SESSION_B, "context", &id);
    assert_eq!(retried["result"], "settled", "{retried}");
    assert_eq!(rig.receipt_state(&id), "acked");
}

/// Spec D3/D7: a session without the mod (old Claude, or the mod never
/// loaded) has no channel, so the native wake fires; `watch` with
/// `HERDR_THREADS_MOD_DELIVERY=off` exits 3 before connecting and registers
/// nothing, so the native wake keeps working for that session too.
#[test]
fn no_mod_or_env_off_keeps_native_wake_and_exits_3() {
    let rig = Rig::new();
    // No mod at all: the native ladder wakes the seat.
    let mark = rig.host.mark();
    let first = rig.send_ordinary("no mod, native wake");
    rig.wait_prompt(mark, PANE_B);
    // Env override: exit 3, one refusal line, no registration.
    let mut off = rig.watch_with(
        PANE_B,
        SESSION_B,
        &[(herdr_threads::protocol::watch::MOD_DELIVERY_ENV, "off")],
    );
    let refusal = off.wait_line("the env refusal", STEP, |line| line["kind"] == "status");
    assert_eq!(refusal["state"], "refused", "{refusal}");
    assert_eq!(refusal["reason"], "env_disabled");
    assert_eq!(refusal["exit"], 3);
    assert_eq!(off.exit_code(STEP), 3);
    assert_eq!(rig.channel_state(), None);
    // `watch ack` honours the same override.
    let mut ack = rig.command(
        Some(PANE_B),
        &[
            "watch",
            "ack",
            "--session",
            SESSION_B,
            "--via",
            "context",
            "m",
        ],
    );
    ack.env(herdr_threads::protocol::watch::MOD_DELIVERY_ENV, "off");
    let out = collect(
        ack.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn_owned()
            .unwrap()
            .wait_with_output()
            .unwrap(),
    );
    assert_eq!(out.code, 3, "{}{}", out.stdout, out.stderr);
    // Still native: once the first message is handled, the next one wakes again.
    rig.agent_turn();
    rig.cli(Some((&rig.b, PANE_B)), &["ack", &first])
        .data("ack");
    rig.advance(31_000);
    let mark = rig.host.mark();
    rig.send_ordinary("still native");
    rig.wait_prompt(mark, PANE_B);
}

/// Spec D2/D3: the daemon setting `mod_delivery: off` refuses registration
/// (`disabled`, exit 3, nothing registered), reports itself through
/// `doctor`, and leaves hooks plus native wake as the delivery path.
#[test]
fn daemon_setting_off_refuses_registration_disabled() {
    let rig = Rig::with_settings(Some(json!({"mod_delivery": "off"})));
    let mut watch = rig.watch(SESSION_B);
    let refusal = watch.wait_line("the disabled refusal", STEP, |line| {
        line["kind"] == "status"
    });
    assert_eq!(refusal["state"], "refused", "{refusal}");
    assert_eq!(refusal["reason"], "disabled");
    assert_eq!(refusal["exit"], 3);
    assert_eq!(watch.exit_code(STEP), 3);
    let channels = rig.channels();
    assert_eq!(channels["mod_delivery"], "off", "{channels}");
    assert_eq!(channels["live_channels"], 0);
    let mark = rig.host.mark();
    rig.send_ordinary("setting off, native wake");
    rig.wait_prompt(mark, PANE_B);
}

/// Spec D2 operator switch: the setting is read at daemon start, so editing
/// it to `off` and restarting the daemon ends the live watch (stream ended,
/// the mod restarts it) and the restarted watch is refused with exit 3.
#[test]
fn operator_switch_off_takes_effect_on_daemon_restart() {
    let mut rig = Rig::new();
    let mut watch = rig.watch(SESSION_B);
    watch.wait_connected();
    // Idle in the stream loop (the first drain is over) when the daemon stops.
    let id = rig.send_ordinary("before the operator switch");
    watch.wait_item(&id);
    let settings = rig.paths.instance_dir.join("settings.json");
    fs::write(&settings, br#"{"mod_delivery":"off"}"#).unwrap();
    fs::set_permissions(&settings, fs::Permissions::from_mode(0o600)).unwrap();
    rig.restart_daemon();
    // Stream ended (0) or a drain cut off by the stop (1): both restart.
    let code = watch.exit_code(STEP);
    assert!(
        matches!(code, 0 | 1),
        "the daemon stop ends the watch: {:#?}",
        watch.final_status()
    );
    let mut again = rig.watch(SESSION_B);
    let refusal = again.wait_line("the disabled refusal", STEP, |line| {
        line["kind"] == "status"
    });
    assert_eq!(refusal["reason"], "disabled", "{refusal}");
    assert_eq!(again.exit_code(STEP), 3);
    assert_eq!(rig.channels()["mod_delivery"], "off");
}

/// Spec D3: no `HERDR_PANE_ID` (desktop, VS Code, `claude -p`) is permanent.
#[test]
fn missing_pane_exits_3() {
    let rig = Rig::new();
    let command = rig.command(
        None,
        &["watch", "--harness", "claude", "--session", SESSION_B],
    );
    let mut watch = Watch::spawn(command);
    let refusal = watch.wait_line("the no-pane refusal", STEP, |line| line["kind"] == "status");
    assert_eq!(refusal["state"], "refused", "{refusal}");
    assert_eq!(refusal["reason"], "no_pane");
    assert_eq!(refusal["exit"], 3);
    assert_eq!(watch.exit_code(STEP), 3);
    assert_eq!(rig.channel_state(), None);
}

/// Spec D6: a mod ack racing the agent's own ACK on the same receipt settles
/// it exactly once; the loser is `already_settled` (or a plain idempotent ACK).
#[test]
fn concurrent_mod_ack_and_inbox_ack_settle_exactly_once() {
    let rig = Rig::new();
    let mut watch = rig.watch(SESSION_B);
    watch.wait_connected();
    for round in 0..2 {
        let id = rig.send_ordinary(&format!("race {round}"));
        watch.wait_item(&id);
        let before: i64 = rig
            .db()
            .query_row(
                "SELECT count(*) FROM messages WHERE kind='info' AND event_key LIKE 'ack%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let barrier = std::sync::Barrier::new(2);
        let (mod_side, inbox_side) = std::thread::scope(|scope| {
            let mod_ack = scope.spawn(|| {
                barrier.wait();
                rig.ack(SESSION_B, "context", &[&id])
            });
            let inbox_ack = scope.spawn(|| {
                barrier.wait();
                rig.cli(Some((&rig.b, PANE_B)), &["ack", &id])
            });
            (mod_ack.join().unwrap(), inbox_ack.join().unwrap())
        });
        assert_eq!(
            inbox_side.code, 0,
            "{}{}",
            inbox_side.stdout, inbox_side.stderr
        );
        let (code, lines) = mod_side;
        assert_eq!(code, 0, "{lines:?}");
        let result = lines[0]["result"].as_str().unwrap();
        assert!(
            matches!(result, "settled" | "already_settled"),
            "round {round}: {lines:?}"
        );
        assert_eq!(rig.receipt_state(&id), "acked");
        let after: i64 = rig
            .db()
            .query_row(
                "SELECT count(*) FROM messages WHERE kind='info' AND event_key LIKE 'ack%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(after - before, 1, "round {round}: one settlement event");
    }
}

/// Spec D6: after the agent's own display ACK (`inbox`), the mod's ack of the
/// same id is `already_settled`, and the receipt keeps the inbox provenance.
#[test]
fn watch_ack_already_settled_after_inbox_ack() {
    let rig = Rig::new();
    let mut watch = rig.watch(SESSION_B);
    watch.wait_connected();
    let id = rig.send_ordinary("read through inbox first");
    watch.wait_item(&id);
    let inbox = rig.cli_text_b(&["inbox"]);
    assert_eq!(inbox.code, 0, "{}{}", inbox.stdout, inbox.stderr);
    assert!(
        inbox.stdout.contains("read through inbox first"),
        "{}",
        inbox.stdout
    );
    assert_eq!(rig.receipt_state(&id), "acked");
    let result = rig.ack_result(SESSION_B, "context", &id);
    assert_eq!(result["result"], "already_settled", "{result}");
    let observation = rig.ack_observation(&id);
    assert_ne!(observation["action_provenance"], "cooperative_mod_delivery");
    assert_eq!(rig.mod_ack_events(), 0);
}

/// Spec D4: invitations (anything without a body) reach the mod as one
/// `attention` line carrying the fixed marker, never as an acked item. The
/// invitation requests confirmation (every agent invitation does since the
/// message-priority change; there is no lazy CLI invitation any more).
#[test]
fn attention_marker_line_for_invitation() {
    let rig = Rig::new();
    let mut watch = rig.watch(SESSION_B);
    watch.wait_connected();
    let other = rig
        .cli(
            Some((&rig.a, PANE_A)),
            &["thread", "create", "--topic", "second thread"],
        )
        .text("thread create");
    rig.cli(
        Some((&rig.a, PANE_A)),
        &["invite", &other, "--seat", &rig.b],
    )
    .data("invite");
    let line = watch.wait_attention_marker();
    assert_eq!(
        line["text"],
        format!(
            "herdr-threads: attention pending; run herdr-threads --state-dir {} inbox",
            rig.state.display()
        ),
        "{line}"
    );
    assert!(line["attention_version"].as_u64().is_some());
    assert!(line["id"].as_str().unwrap().starts_with("attention:"));
}

/// ht-j16.33 step 1: a session that starts after B accepted the invitation
/// sees no attention at connect.
#[test]
fn fresh_watch_after_accept_prints_no_attention() {
    let rig = Rig::new();
    rig.session_start_hook(PANE_B, SESSION_B, "startup");
    let mut watch = rig.watch(SESSION_B);
    watch.wait_connected();
    watch.assert_no_line("attention", QUIET, |l| l["kind"] == "attention");
}

/// ht-j16.33: accepting a confirming invitation or carrying a notice on V2
/// Watch clears the matching attention epoch. Keep the historical selector.
#[test]
fn stale_attention_is_retracted_after_accept_and_after_a_notice_offer() {
    use herdr_threads::{
        client::service::{PersistentServiceClient, ServiceIntentJournal},
        daemon::ownership::{read_descriptor, read_existing_namespace},
        protocol::{
            ids::{OperationId, SeatId, ThreadId},
            service::{
                EnsureManagedThread, InvitationConstraint, NotificationSeverity, ServiceInvite,
                ServiceNotify, ServiceOperation,
            },
            time::CallBudget,
        },
    };

    let rig = Rig::new();
    // (a) a confirming invitation, then B accepts it.
    let mut watch = rig.watch(SESSION_B);
    watch.wait_connected();
    let other = rig
        .cli(
            Some((&rig.a, PANE_A)),
            &["thread", "create", "--topic", "second thread"],
        )
        .text("thread create");
    rig.cli(
        Some((&rig.a, PANE_A)),
        &["invite", &other, "--seat", &rig.b],
    )
    .data("invite");
    let attention = watch.wait_attention_marker();
    assert!(
        attention["presentation_epoch"].as_str().is_some(),
        "{attention}"
    );
    rig.cli(Some((&rig.b, PANE_B)), &["accept", &other])
        .data("accept");
    let cleared = watch.wait_line("the retraction", STEP, |l| l["kind"] == "attention_cleared");
    assert!(
        cleared["id"]
            .as_str()
            .unwrap()
            .starts_with("attention_cleared:"),
        "{cleared}"
    );
    assert!(cleared["attention_version"].as_u64().is_some(), "{cleared}");
    assert_eq!(
        cleared["presentation_epoch"],
        attention["presentation_epoch"]
    );

    // (b) a notice published with no live channel, then carried and settled
    // by the replacement Watch before any hook can offer it again.
    // The killed child leaves the channel in reconnect grace; the next watch
    // replaces it.
    rig.kill(&mut watch);
    let instance = read_existing_namespace(&rig.paths)
        .unwrap()
        .expect("instance");
    let descriptor = read_descriptor(&rig.paths, instance).unwrap();
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let client = PersistentServiceClient::new(
        descriptor.endpoint.clone(),
        Arc::clone(&clock),
        descriptor.instance_uuid,
        Some(descriptor.boot_id),
        ServiceIntentJournal::open(rig.root.join("intents")).unwrap(),
    );
    let budget = || CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0 + 10_000),
        cancellation: Cancellation::default(),
    };
    let managed = ThreadId::new("thread-notice-stale");
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            client.register(&budget()).await.unwrap();
            client
                .submit(
                    ServiceOperation::EnsureThread(EnsureManagedThread {
                        thread: managed.clone(),
                        topic: "notice stale".into(),
                        goal: "coordination".into(),
                        operation: OperationId::new("ensure"),
                    }),
                    &budget(),
                )
                .await
                .unwrap();
            client
                .submit(
                    ServiceOperation::Invite(ServiceInvite {
                        thread: managed.clone(),
                        seat: SeatId::new(rig.b.clone()),
                        constraint: InvitationConstraint::Ordinary,
                        deadline_millis: Some(300_000),
                        operation: OperationId::new("invite"),
                    }),
                    &budget(),
                )
                .await
                .unwrap();
            rig.cli(Some((&rig.b, PANE_B)), &["accept", managed.as_str()])
                .data("accept");
            client
                .submit(
                    ServiceOperation::Notify(ServiceNotify {
                        thread: managed.clone(),
                        severity: NotificationSeverity::Warn,
                        event_json: json!({"kind": "integration", "detail": "stale notice"}),
                        operation: OperationId::new("notify"),
                    }),
                    &budget(),
                )
                .await
                .unwrap();
        });
    // Start the replacement only after the real worker has attributed Warn
    // to B; its initial Pull then observes that open relevance condition.
    wait_until("the stale notice projection", STEP, || {
        let count: i64 = rig.db().query_row(
            "SELECT count(*) FROM digest_programmatic_warnings WHERE seat_id=?1 AND thread_id=?2",
            [&rig.b, managed.as_str()], |row| row.get(0),
        ).unwrap();
        (count > 0).then_some(())
    });
    let mut watch = rig.watch(SESSION_B);
    watch.wait_connected();
    let attention = watch.wait_attention_marker();
    assert!(
        attention["presentation_epoch"].as_str().is_some(),
        "{attention}"
    );
    // Notice can precede Attention in the same flushed page. Retain that
    // actual stdout witness instead of waiting for a second offer.
    let notice = watch
        .seen
        .iter()
        .find(|line| line["kind"] == "notice" && line["thread"] == managed.as_str())
        .cloned()
        .unwrap_or_else(|| {
            watch.wait_line("the stale Watch notice", STEP, |line| {
                line["kind"] == "notice" && line["thread"] == managed.as_str()
            })
        });
    let warning = notice["warning"]
        .as_str()
        .filter(|id| !id.is_empty())
        .unwrap();
    assert_eq!(notice["schema"], 1);
    assert_eq!(notice["id"], format!("notice:{warning}"));
    assert_eq!(
        notice["body"],
        format!(
            "informational, no ACK; read: herdr-threads --state-dir {} body {warning}",
            rig.state.display()
        )
    );
    assert!(notice.get("ack_required").is_none(), "{notice}");
    let (expected_binding, expected_generation, expected_execution) =
        expected_notice_binding(&rig, descriptor.instance_uuid);
    let (offered_generation, offered_execution, frontier, ordinal, generation, execution, thread, offered_binding):
        (i64, String, i64, i64, i64, String, String, String) = rig.db().query_row(
            "SELECT offered.generation,offered.execution_id,o.offered_ordinal,d.ordinal,b.generation,b.execution_id,d.thread_id,o.binding_id FROM digest_notice_offer o JOIN occupant_bindings offered ON offered.binding_id=o.binding_id AND offered.seat_id=o.seat_id JOIN digest_programmatic_warnings d ON d.seat_id=o.seat_id JOIN occupant_bindings b ON b.seat_id=o.seat_id AND b.harness='claude' AND b.native_session=?3 AND b.agent_id='' AND b.role='top_level' AND b.ended_at IS NULL AND b.liveness='connected' AND b.binding_id=o.binding_id WHERE o.seat_id=?1 AND d.warning_id=?2 AND o.binding_id=?4",
            [rig.b.as_str(), warning, SESSION_B, expected_binding.as_str()],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?)),
        ).unwrap();
    assert_eq!(thread, managed.as_str());
    assert_eq!(offered_binding, expected_binding);
    assert_eq!(offered_generation, generation);
    assert_eq!(offered_generation, expected_generation);
    assert_eq!(offered_execution, execution);
    assert_eq!(offered_execution, expected_execution);
    assert_ne!(execution, SESSION_B);
    assert!(ordinal > 0 && frontier >= ordinal);
    // The replacement may replay the earlier invitation's clear as well.
    // Retraction of this notice must name its own emitted Attention epoch.
    let is_notice_clear = |line: &Value| {
        line["kind"] == "attention_cleared"
            && line["presentation_epoch"] == attention["presentation_epoch"]
    };
    let cleared = watch
        .seen
        .iter()
        .find(|line| is_notice_clear(line))
        .cloned()
        .unwrap_or_else(|| watch.wait_line("the scoped notice retraction", STEP, is_notice_clear));
    assert_eq!(
        cleared["presentation_epoch"],
        attention["presentation_epoch"]
    );
    let hook = rig.hook(PANE_B, &pre_tool_use(SESSION_B));
    assert_eq!(hook.code, 0, "{}{}", hook.stdout, hook.stderr);
    assert!(
        !hook.stdout.contains("offered notices"),
        "{:?}",
        hook.stdout
    );
    assert_eq!(rig.mod_ack_events(), 0);
    assert_eq!(rig.channel_state().as_deref(), Some("live"));
    assert!(watch.running(), "the watch child died");
}

/// Spec D2/D6 `/clear`: the new session supersedes the cleared session's binding
/// (session-keyed bindings: the generation no longer rotates) and closes its channel
/// (`binding_changed`); the new session's watch registers and gets the still-pending item
/// re-streamed, and its ack settles.
#[test]
fn clear_rebind_pending_item_restreamed_to_new_session() {
    let rig = Rig::new();
    let mut old = rig.watch(SESSION_B);
    old.wait_connected();
    let id = rig.send_ordinary("pending across /clear");
    old.wait_item(&id);
    rig.set_session_b("SB2");
    rig.session_start_hook_in_process_of(PANE_B, "SB2", "clear", SESSION_B);
    let closing = old.wait_line("the binding_changed close", STEP, |line| {
        line["kind"] == "status" && line["state"] == "closing"
    });
    assert_eq!(closing["reason"], "binding_changed", "{closing}");
    assert_eq!(old.exit_code(STEP), 0);
    rig.wait_channel(Some("rebind_grace"));
    assert_eq!(rig.receipt_state(&id), "pending");
    // `/clear` keeps the harness process, but its native session (and therefore
    // execution claim) is SB2. Retain the old process fingerprint from command().
    let mut new = rig.watch_with(PANE_B, "SB2", &[("CLAUDE_CODE_SESSION_ID", "SB2")]);
    new.wait_connected();
    let line = new.wait_item(&id);
    assert_eq!(
        line["body"], "pending across /clear",
        "re-streamed to the new session"
    );
    rig.wait_channel(Some("live"));
    let (code, lines) = rig.ack_with(
        "SB2",
        "context",
        &[&id],
        &[("CLAUDE_CODE_SESSION_ID", "SB2")],
    );
    assert_eq!(code, 0, "watch ack {id}: {lines:?}");
    let settled = lines
        .into_iter()
        .find(|line| line["id"] == id)
        .unwrap_or_else(|| panic!("no result line for {id}"));
    assert_eq!(settled["result"], "settled", "{settled}");
}

/// The exact Watch binding expected by the notice tests, independently of its offer row.
fn expected_notice_binding(rig: &Rig, instance: uuid::Uuid) -> (String, i64, String) {
    let (binding, generation): (String, i64) = rig.db().query_row(
        "SELECT b.binding_id,s.generation FROM seats s JOIN occupant_bindings b ON b.seat_id=s.id WHERE s.id=?1 AND s.instance_id=?2 AND s.state='resolved' AND s.target_id=?3 AND b.harness='claude' AND b.native_session=?4 AND b.agent_id='' AND b.role='top_level' AND b.ended_at IS NULL AND b.liveness='connected'",
        rusqlite::params![rig.b.as_str(), instance.to_string(), PANE_B, SESSION_B],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    let execution = herdr_threads::harness::context::session_execution(
        instance,
        Harness::Claude,
        SESSION_B,
        None,
    )
    .to_string();
    (binding, generation, execution)
}

/// The open binding of native session `session` on B's seat: `(binding_id, liveness,
/// disconnect_reason)`. `None` when the seat has no such open binding.
fn open_binding_of(rig: &Rig, session: &str) -> Option<(String, String, Option<String>)> {
    use rusqlite::OptionalExtension;
    rig.db()
        .query_row(
            "SELECT binding_id,liveness,disconnect_reason FROM occupant_bindings \
             WHERE seat_id=?1 AND native_session=?2 AND ended_at IS NULL",
            [rig.b.as_str(), session],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .unwrap()
}

/// The binding's read cursor in the rig's thread.
fn cursor_of(rig: &Rig, binding: &str) -> Option<i64> {
    use rusqlite::OptionalExtension;
    rig.db()
        .query_row(
            "SELECT seq FROM binding_cursors WHERE binding_id=?1 AND thread_id=?2",
            [binding, rig.thread.as_str()],
            |row| row.get(0),
        )
        .optional()
        .unwrap()
}

fn sequence_of(rig: &Rig, message: &str) -> i64 {
    rig.db()
        .query_row(
            "SELECT sequence FROM messages WHERE id=?1",
            [message],
            |row| row.get(0),
        )
        .unwrap()
}

/// `/clear` is a new binding (run.md parked escalation, codeBuckets.review; spec 1.3 and 1.5:
/// the cleared session's binding is superseded by the new session's), and its cursors start at
/// the seat maximum (spec 2 Cursors). Mail the old binding already displayed is therefore
/// unread for nobody: the new binding's SessionStart carries no digest, no composer wake fires
/// (spec 3 Wake: only mail unread by that binding wakes it), and the old binding's rebind grace
/// expiring changes nothing.
#[test]
fn clear_new_binding_gets_no_digest_or_wake_for_mail_already_read() {
    let rig = Rig::new();
    let mut watch = rig.watch(SESSION_B);
    watch.wait_connected();
    let id = rig.send_ordinary("read before /clear");
    watch.wait_item(&id);
    let inbox = rig.cli_text_b(&["inbox"]);
    assert_eq!(inbox.code, 0, "{}{}", inbox.stdout, inbox.stderr);
    assert!(
        inbox.stdout.contains("read before /clear"),
        "{}",
        inbox.stdout
    );
    let (old_binding, _, _) = open_binding_of(&rig, SESSION_B).expect("SB binding");
    assert_eq!(
        cursor_of(&rig, &old_binding),
        Some(sequence_of(&rig, &id)),
        "SB displayed the message, so its cursor is at that sequence"
    );
    let mark = rig.host.mark();
    rig.set_session_b("SB2");
    let started = rig.session_start_hook_in_process_of(PANE_B, "SB2", "clear", SESSION_B);
    // Without a live channel for the new binding the hook still prints the digest line, but it is
    // empty: nothing pending (the text inbox ACKed the displayed message).
    assert!(
        started
            .stdout
            .contains("attention digest: invitations=0; receipts=0; warnings=0"),
        "expected an empty digest line: {}",
        started.stdout
    );
    let (_, old_liveness, old_reason) = open_binding_of(&rig, SESSION_B).expect("SB binding");
    assert_eq!(old_liveness, "disconnected");
    assert_eq!(old_reason.as_deref(), Some("superseded"));
    let (new_binding, new_liveness, _) = open_binding_of(&rig, "SB2").expect("SB2 binding");
    assert_eq!(new_liveness, "connected");
    assert_ne!(new_binding, old_binding, "/clear opens a new binding");
    rig.assert_no_prompt(mark, QUIET);
    // The quiet window gave what was in flight time to finish, so jump at once.
    rig.clock.advance(31_000);
    rig.wait_channel(None);
    rig.assert_no_prompt(mark, QUIET);
}

/// The other half of the `/clear` model (same sources as above): mail no binding displayed is
/// unread for the new binding too, since its seat-maximum cursor is short of it. The new
/// binding's SessionStart digests it and the composer wake fires at once, with no rebind-grace
/// suppression (the clock never passes `MOD_REBIND_GRACE_MS`).
#[test]
fn clear_new_binding_digests_and_wakes_for_its_unread_mail_without_grace() {
    let rig = Rig::new();
    let mut watch = rig.watch(SESSION_B);
    watch.wait_connected();
    let id = rig.send_ordinary("only the watch saw this");
    watch.wait_item(&id);
    let mark = rig.host.mark();
    rig.set_session_b("SB2");
    let started = rig.session_start_hook_in_process_of(PANE_B, "SB2", "clear", SESSION_B);
    assert!(
        started.stdout.contains(DIGEST_MARK),
        "no digest for mail unread by the new binding: {}",
        started.stdout
    );
    assert!(
        started.stdout.contains("receipts=1 ["),
        "the digest does not list the unread mail's pending receipt: {}",
        started.stdout
    );
    rig.wait_prompt(mark, PANE_B);
    assert_eq!(rig.receipt_state(&id), "pending");
}

/// Spec D5/D6 reload: the plugin reloads without rotating the generation; the
/// restarted `watch` replaces the old stream (`replaced`, exit 3, so two
/// watchers never ping-pong), gets the pending item again, and its ack under
/// the same generation settles.
#[test]
fn reload_reack_same_generation_settles() {
    let rig = Rig::new();
    let mut old = rig.watch(SESSION_B);
    old.wait_connected();
    let id = rig.send_ordinary("delivered before the reload");
    old.wait_item(&id);
    let generation = rig.channels()["channels"][0]["binding_generation"].clone();
    let mut new = rig.watch(SESSION_B);
    new.wait_connected();
    let closing = old.wait_line("the replaced close", STEP, |line| {
        line["kind"] == "status" && line["state"] == "closing"
    });
    assert_eq!(closing["reason"], "replaced", "{closing}");
    assert_eq!(old.exit_code(STEP), 3);
    new.wait_item(&id);
    assert_eq!(
        rig.channels()["channels"][0]["binding_generation"],
        generation,
        "a reload does not rotate the generation"
    );
    let result = rig.ack_result(SESSION_B, "context", &id);
    assert_eq!(result["result"], "settled", "{result}");
}

/// `/resume` keeps the native session, so it is a lifecycle check-in of the same binding
/// (spec 1.3, `Resolution::Existing`): no `binding_changed` close, the channel and its binding
/// generation stay as they were, and the re-ACK settles on that binding.
#[test]
fn resume_keeps_binding_and_reack_settles() {
    let rig = Rig::new();
    let mut old = rig.watch(SESSION_B);
    old.wait_connected();
    let id = rig.send_ordinary("delivered before the resume");
    old.wait_item(&id);
    let (binding, _, _) = open_binding_of(&rig, SESSION_B).expect("SB binding");
    let channel = rig.channels()["channels"][0].clone();
    assert_eq!(channel["state"], "live", "{channel}");
    rig.session_start_hook(PANE_B, SESSION_B, "resume");
    old.assert_no_line("a closing status", QUIET, |line| {
        line["kind"] == "status" && line["state"] == "closing"
    });
    let after = rig.channels()["channels"][0].clone();
    assert_eq!(after["state"], "live", "{after}");
    assert_eq!(
        after["binding_generation"], channel["binding_generation"],
        "resume does not change the binding generation"
    );
    let open: i64 = rig
        .db()
        .query_row(
            "SELECT count(*) FROM occupant_bindings WHERE seat_id=?1 AND native_session=?2 AND ended_at IS NULL",
            [rig.b.as_str(), SESSION_B],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(open, 1, "exactly one open SB binding");
    let (same, liveness, _) = open_binding_of(&rig, SESSION_B).expect("SB binding");
    assert_eq!(same, binding, "resume keeps the same binding");
    assert_eq!(liveness, "connected");
    let result = rig.ack_result(SESSION_B, "context", &id);
    assert_eq!(result["result"], "settled", "{result}");
    assert_eq!(rig.receipt_state(&id), "acked");
}

/// Spec D3: `watch` exits when its parent dies (`$.process.spawn` closes
/// stdin, so it polls the parent pid): the channel leaves `live` within
/// seconds and the stream is closed.
#[test]
fn parent_death_unregisters_within_seconds() {
    let rig = Rig::new();
    let out = rig.root.join("orphan.out");
    let pid_file = rig.root.join("orphan.pid");
    // The parent shell starts `watch`, waits until it is connected, then exits.
    let script = format!(
        r#""$@" > "{out}" 2>/dev/null & echo $! > "{pid}"; n=0; while ! grep -q connected "{out}" 2>/dev/null; do n=$((n+1)); [ $n -gt 400 ] && exit 9; sleep 0.05; done; exit 0"#,
        out = out.display(),
        pid = pid_file.display()
    );
    let mut command = spawn::command("/bin/sh");
    let args = ["watch", "--harness", "claude", "--session", SESSION_B];
    let inner = rig.command(Some(PANE_B), &args);
    command
        .arg("-c")
        .arg(script)
        .arg("sh")
        .arg(inner.get_program());
    command.args(inner.get_args());
    for (key, value) in inner.get_envs() {
        match value {
            Some(value) => command.env(key, value),
            None => command.env_remove(key),
        };
    }
    let status = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn_owned()
        .unwrap()
        .wait_with_output()
        .unwrap()
        .status;
    assert_eq!(
        status.code(),
        Some(0),
        "the watch connected before its parent exited"
    );
    let pid: i32 = fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    // SAFETY: signal 0 only probes for the process.
    let alive = |pid: i32| unsafe { libc::kill(pid, 0) } == 0;
    wait_until(
        "the orphaned watch to exit",
        Duration::from_secs(10),
        || (!alive(pid)).then_some(()),
    );
    wait_until("the channel to leave live", Duration::from_secs(10), || {
        (rig.channel_state().as_deref() != Some("live")).then_some(())
    });
}

/// Spec D2/D3: a daemon restart drops every registration. The running watch
/// ends (stream ended or a cut-off drain: both restart), and the mod-style
/// restart registers again and gets the pending item streamed.
#[test]
fn daemon_restart_ends_watch_and_restart_reregisters() {
    let mut rig = Rig::new();
    let mut first = rig.watch(SESSION_B);
    first.wait_connected();
    let id = rig.send_ordinary("pending across the daemon restart");
    first.wait_item(&id);
    rig.restart_daemon();
    let code = first.exit_code(STEP);
    assert!(
        matches!(code, 0 | 1),
        "exit {code}: {:#?}",
        first.final_status()
    );
    assert_eq!(
        rig.channel_state(),
        None,
        "a restart drops every registration"
    );
    let mut second = rig.watch(SESSION_B);
    second.wait_connected();
    second.wait_item(&id);
    rig.wait_channel(Some("live"));
}

/// Spec D7: while a channel is live, neither the `PreToolUse(Bash)` nor the
/// `SessionStart` hook prints an attention digest; without a channel the same
/// hooks do (the control).
#[test]
fn session_start_and_pre_tool_use_emit_no_digest_while_live() {
    let rig = Rig::new();
    // Control: no channel, so both hooks print the attention digest.
    rig.send_ordinary("digest control");
    let control = rig.hook(PANE_B, &pre_tool_use(SESSION_B));
    assert_eq!(control.code, 0, "{}{}", control.stdout, control.stderr);
    assert!(
        control.stdout.contains(DIGEST_MARK),
        "the control PreToolUse hook prints a digest: {:?}",
        control.stdout
    );
    let control = rig.session_start_hook(PANE_B, SESSION_B, "startup");
    assert!(
        control.stdout.contains(DIGEST_MARK),
        "the control SessionStart hook prints a digest: {:?}",
        control.stdout
    );
    let mut watch = rig.watch(SESSION_B);
    watch.wait_connected();
    let id = rig.send_ordinary("digest suppressed");
    watch.wait_item(&id);
    let boundary = rig.hook(PANE_B, &pre_tool_use(SESSION_B));
    assert_eq!(boundary.code, 0, "{}{}", boundary.stdout, boundary.stderr);
    assert!(
        !boundary.stdout.contains(DIGEST_MARK),
        "PreToolUse digest while live: {:?}",
        boundary.stdout
    );
    let started = rig.session_start_hook(PANE_B, SESSION_B, "startup");
    assert!(
        !started.stdout.contains(DIGEST_MARK),
        "SessionStart digest while live: {:?}",
        started.stdout
    );
}

/// ht-j16.21: live V2 Watch carries a notice once; subsequent PreToolUse
/// hooks stay quiet. Keep the historical selector for the test inventory.
#[test]
fn notice_published_while_live_reaches_the_next_pre_tool_use_once() {
    use herdr_threads::{
        client::service::{PersistentServiceClient, ServiceIntentJournal},
        daemon::ownership::{read_descriptor, read_existing_namespace},
        protocol::{
            ids::{OperationId, SeatId, ThreadId},
            service::{
                EnsureManagedThread, InvitationConstraint, NotificationSeverity, ServiceInvite,
                ServiceNotify, ServiceOperation,
            },
            time::CallBudget,
        },
    };

    let rig = Rig::new();
    let mut watch = rig.watch(SESSION_B);
    watch.wait_connected();
    let instance = read_existing_namespace(&rig.paths)
        .unwrap()
        .expect("instance");
    let descriptor = read_descriptor(&rig.paths, instance).unwrap();
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let client = PersistentServiceClient::new(
        descriptor.endpoint.clone(),
        Arc::clone(&clock),
        descriptor.instance_uuid,
        Some(descriptor.boot_id),
        ServiceIntentJournal::open(rig.root.join("intents")).unwrap(),
    );
    let budget = || CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0 + 10_000),
        cancellation: Cancellation::default(),
    };
    let managed = ThreadId::new("thread-notice-live");
    let mark = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            client.register(&budget()).await.unwrap();
            client
                .submit(
                    ServiceOperation::EnsureThread(EnsureManagedThread {
                        thread: managed.clone(),
                        topic: "notice while live".into(),
                        goal: "coordination".into(),
                        operation: OperationId::new("ensure"),
                    }),
                    &budget(),
                )
                .await
                .unwrap();
            client
                .submit(
                    ServiceOperation::Invite(ServiceInvite {
                        thread: managed.clone(),
                        seat: SeatId::new(rig.b.clone()),
                        constraint: InvitationConstraint::Ordinary,
                        deadline_millis: Some(300_000),
                        operation: OperationId::new("invite"),
                    }),
                    &budget(),
                )
                .await
                .unwrap();
            // B accepts through its own CLI, then the service notifies on the
            // same connection (one runtime: the client is bound to it).
            rig.cli(Some((&rig.b, PANE_B)), &["accept", managed.as_str()])
                .data("accept");
            let mark = rig.host.mark();
            client
                .submit(
                    ServiceOperation::Notify(ServiceNotify {
                        thread: managed.clone(),
                        severity: NotificationSeverity::Warn,
                        event_json: json!({"kind": "integration", "detail": "notice while live"}),
                        operation: OperationId::new("notify"),
                    }),
                    &budget(),
                )
                .await
                .unwrap();
            mark
        });
    // Establish the actual Watch delivery before invoking either hook.
    let notice = watch.wait_line("the live Watch notice", STEP, |line| {
        line["kind"] == "notice" && line["thread"] == managed.as_str()
    });
    let warning = notice["warning"]
        .as_str()
        .filter(|id| !id.is_empty())
        .unwrap();
    assert_eq!(notice["schema"], 1);
    assert_eq!(notice["id"], format!("notice:{warning}"));
    assert_eq!(
        notice["body"],
        format!(
            "informational, no ACK; read: herdr-threads --state-dir {} body {warning}",
            rig.state.display()
        )
    );
    assert!(notice.get("ack_required").is_none(), "{notice}");
    let (expected_binding, expected_generation, expected_execution) =
        expected_notice_binding(&rig, descriptor.instance_uuid);
    let (offered_generation, offered_execution, frontier, ordinal, generation, execution, thread, offered_binding):
        (i64, String, i64, i64, i64, String, String, String) = rig.db().query_row(
            "SELECT offered.generation,offered.execution_id,o.offered_ordinal,d.ordinal,b.generation,b.execution_id,d.thread_id,o.binding_id FROM digest_notice_offer o JOIN occupant_bindings offered ON offered.binding_id=o.binding_id AND offered.seat_id=o.seat_id JOIN digest_programmatic_warnings d ON d.seat_id=o.seat_id JOIN occupant_bindings b ON b.seat_id=o.seat_id AND b.harness='claude' AND b.native_session=?3 AND b.agent_id='' AND b.role='top_level' AND b.ended_at IS NULL AND b.liveness='connected' AND b.binding_id=o.binding_id WHERE o.seat_id=?1 AND d.warning_id=?2 AND o.binding_id=?4",
            [rig.b.as_str(), warning, SESSION_B, expected_binding.as_str()],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?)),
        ).unwrap();
    assert_eq!(thread, managed.as_str());
    assert_eq!(offered_binding, expected_binding);
    assert_eq!(offered_generation, generation);
    assert_eq!(offered_generation, expected_generation);
    assert_eq!(offered_execution, execution);
    assert_eq!(offered_execution, expected_execution);
    assert_ne!(execution, SESSION_B);
    assert!(ordinal > 0 && frontier >= ordinal);
    for _ in 0..2 {
        let hook = rig.hook(PANE_B, &pre_tool_use(SESSION_B));
        assert_eq!(hook.code, 0, "{}{}", hook.stdout, hook.stderr);
        assert!(
            !hook.stdout.contains(DIGEST_MARK),
            "digest while live: {:?}",
            hook.stdout
        );
        assert!(
            !hook.stdout.contains("pending mail"),
            "ready commands while live: {:?}",
            hook.stdout
        );
        assert!(
            !hook.stdout.contains("offered notices"),
            "repeat notice offer: {:?}",
            hook.stdout
        );
    }
    // Scan prior and future real stdout, accepting exactly the one observed
    // Notice while rejecting any repeat for this warning.
    let occurrences = std::cell::Cell::new(0);
    watch.assert_no_line("duplicate live Notice", QUIET, |line| {
        if line["kind"] == "notice" && line["warning"] == warning {
            occurrences.set(occurrences.get() + 1);
            occurrences.get() > 1
        } else {
            false
        }
    });
    assert_eq!(occurrences.get(), 1);
    rig.assert_no_prompt(mark, Duration::ZERO);
    assert_eq!(rig.mod_ack_events(), 0);
    assert_eq!(rig.channel_state().as_deref(), Some("live"));
    assert!(watch.running(), "the watch child died");
}

/// Spec D2 end to end: once the registry's watch budget is full (64 in
/// production; 4 here, the 64 bound is pinned by the transport and registry
/// unit tests), a further registration is refused `busy` (retryable, exit 2)
/// while ordinary requests are still served.
#[test]
fn watch_admission_over_cap_refuses_busy() {
    const CAP: usize = 4;
    let deadline = Instant::now() + Duration::from_secs(25);
    let rig = Rig::with_config(
        None,
        ServiceConfig::default()
            .with_watch_connection_cap(CAP)
            .unwrap(),
    );
    let extra: Vec<(String, String)> = (0..CAP)
        .map(|n| (format!("w1:p{}", n + 3), format!("X{n}")))
        .collect();
    let mut panes = vec![pane(PANE_A, "term-a"), claude(PANE_B, "term-b", SESSION_B)];
    panes.extend(
        extra
            .iter()
            .enumerate()
            .map(|(n, (id, session))| claude(id, &format!("term-x{n}"), session)),
    );
    rig.host.set_panes(panes);
    // Register the extra seats through their SessionStart hooks.
    for (id, session) in &extra {
        rig.session_start_hook(id, session, "startup");
    }
    assert!(
        Instant::now() < deadline,
        "hook registration ran past the test deadline"
    );
    // Each holder is its session's `watch` child, so it runs as the same
    // harness process (execution identity) its SessionStart hook bound.
    let spawn_holder = |id: &str, session: &str| {
        let mut command = rig.command(
            Some(id),
            &["watch", "--harness", "claude", "--session", session],
        );
        herdr_threads::test_support::isolation::as_agent(&mut command, "claude", session);
        Watch::spawn(command)
    };
    let mut spawned: Vec<Watch> = extra
        .iter()
        .map(|(id, session)| spawn_holder(id, session))
        .collect();
    for (holder, (id, session)) in spawned.iter_mut().zip(&extra) {
        for _ in 0..5 {
            let first = holder.peek_first(STEP);
            let reason = first
                .as_ref()
                .filter(|line| line["kind"] == "status" && line["state"] == "refused")
                .and_then(|line| line["reason"].as_str().map(str::to_owned));
            match reason.as_deref() {
                // A hook that missed its budget under load left its check-in
                // uncommitted; run it again, as the next hook would.
                Some("no_binding") => {
                    rig.session_start_hook(id, session, "startup");
                }
                // Exit 1: the mod restarts the watch.
                Some("daemon_unavailable" | "error") => {}
                _ => break,
            }
            std::thread::sleep(Duration::from_millis(150));
            *holder = spawn_holder(id, session);
        }
        holder.wait_connected();
    }
    let mut holders = spawned;
    assert!(
        Instant::now() < deadline,
        "the holders' connections ran past the test deadline"
    );
    assert_eq!(rig.channels()["live_channels"], CAP, "{}", rig.channels());
    let mut over = rig
        .try_watch(PANE_B, SESSION_B, &[], 3)
        .expect("the SessionStart check-in never committed for the over-cap seat");
    let refusal = over.wait_line("the busy refusal", STEP, |line| {
        line["kind"] == "status" && line["state"] == "refused"
    });
    assert_eq!(refusal["reason"], "busy", "{refusal}");
    assert_eq!(refusal["exit"], 2);
    assert_eq!(over.exit_code(STEP), 2);
    // Ordinary requests have their own slots: a send still goes through.
    rig.send_ordinary("sent while the watch budget is full");
    // Releasing one holder frees a slot (after its reconnect grace expires).
    let freed = holders.remove(0);
    drop(freed);
    rig.advance(31_000);
    wait_until("the freed slot to be usable", STEP, || {
        assert!(
            Instant::now() < deadline,
            "the freed slot probe ran past the test deadline"
        );
        let mut retry = rig.try_watch(PANE_B, SESSION_B, &[], 1)?;
        let line = retry.wait_line("a registration answer", Duration::from_secs(5), |line| {
            line["kind"] == "status"
        });
        (line["state"] == "connected").then(|| drop(retry))
    });
    // Signal every holder first so they exit together; each Drop then only
    // reaps its child.
    for holder in &holders {
        // SAFETY: signals only the process group this owned child leads.
        unsafe { libc::killpg(holder.child.id() as libc::pid_t, libc::SIGTERM) };
    }
}

/// ht-j16.34: another seat's overdue open and clear transitions are
/// informational notices for a bystander (TRUST-POLICY A7). They wake only
/// the affected seat natively, so they must not raise mod attention either:
/// the bystander's live watch prints no `attention` line (the mod would
/// submit it into an idle session), while the affected seat's watch does.
#[test]
fn other_seat_overdue_transitions_raise_no_mod_attention_for_a_bystander() {
    let rig = Rig::new();
    rig.host.set_panes(vec![
        pane(PANE_A, "term-a"),
        claude(PANE_B, "term-b", SESSION_B),
        claude(PANE_C, "term-c", SESSION_C),
    ]);
    let c = rig.seat_resolve(PANE_C);
    rig.session_start_hook(PANE_C, SESSION_C, "startup");
    rig.cli(
        Some((&rig.a, PANE_A)),
        &["invite", &rig.thread, "--seat", &c],
    )
    .data("invite c");
    rig.cli(Some((&c, PANE_C)), &["accept", &rig.thread])
        .data("accept c");
    let mut bystander = rig.watch(SESSION_B);
    bystander.wait_connected();
    let mut affected = rig.watch_with(PANE_C, SESSION_C, &[]);
    affected.wait_connected();

    let id = rig
        .cli(
            Some((&rig.a, PANE_A)),
            &[
                "send",
                &rig.thread,
                "--body",
                "critical",
                "--require-ack",
                &rig.b,
                "--require-ack",
                &c,
                "--deadline",
                "60",
            ],
        )
        .text("send with deadline");
    bystander.wait_item(&id);
    affected.wait_item(&id);
    let result = rig.ack_result(SESSION_B, "context", &id);
    assert_eq!(result["result"], "settled", "{result}");
    // The bystander's absence of `attention` is checked once, at the end:
    // `assert_no_line` also judges every line already read, so that one
    // window covers everything the bystander printed since it connected
    // (before the deadline, its drain while C is overdue, after C's clear),
    // and it starts after the last transition.

    let conditions = || -> Vec<Option<String>> {
        let db = rig.db();
        let mut stmt = db
            .prepare("SELECT clear_warning_id FROM warning_conditions ORDER BY ordinal")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    // Only C misses the deadline: one open condition about C. The deadline
    // lane notices the advanced clock on its next pass; kick it rather than
    // wait out its safety tick (detection latency is not under test).
    rig.advance(2 * MINUTE_MS);
    rig.probe.kick_registered(Lane::Deadlines);
    wait_until("C's overdue condition to open", STEP, || {
        (conditions() == [None]).then_some(())
    });
    affected.wait_attention_marker();
    // Make the bystander drain while C's condition is still open: a lazy row
    // changes its fingerprint, so the next worker pass (kicked by the row's
    // commit, which follows the open's) pushes it Attention and its drain
    // reads every pending item, the open notice among them. An `attention`
    // line from that drain follows the row's line and is caught below.
    let synced = rig.send_lazy("drains the bystander while C is overdue");
    bystander.wait_item(&synced);

    // C's late ACK clears it: a clear wakes nobody.
    rig.cli(Some((&c, PANE_C)), &["ack", &id]).data("late ack");
    wait_until("C's overdue condition to clear", STEP, || {
        let now = conditions();
        (now.len() == 1 && now[0].is_some()).then_some(())
    });
    bystander.assert_no_line("attention for C's overdue open or clear", QUIET, |l| {
        l["kind"] == "attention"
    });
    // C's own ACK may drain while its open condition is still uncleared (one
    // more attention line); once the clear commits, its drain finds nothing
    // that wakes it and retracts.
    affected.wait_line("C's retraction after the clear", STEP, |l| {
        l["kind"] == "attention_cleared"
    });
    assert!(bystander.running() && affected.running());
}

/// The stand-in host joins every serving thread on drop: a connection whose
/// request never arrives (its thread blocked in the read, with a 10 s read
/// timeout) is shut and joined at once, and the client sees the close.
#[test]
fn stand_in_host_drop_joins_a_blocked_serving_thread() {
    let root = Scratch(std::env::temp_dir().join(format!(
        "md-host-{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    )));
    fs::create_dir_all(&root.0).unwrap();
    let socket = root.0.join("h.sock");
    let host = Host::start(&socket, vec![]);
    let mut silent = std::os::unix::net::UnixStream::connect(&socket).unwrap();
    // Before the host closes it: macOS refuses the option once the peer hung up.
    silent
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    wait_until(
        "the silent connection to be served",
        Duration::from_secs(5),
        || (host.serving.lock().unwrap().open.len() == 1).then_some(()),
    );
    let serving = host.serving.clone();
    let started = Instant::now();
    drop(host);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "drop waited {:?} for a blocked serving thread",
        started.elapsed()
    );
    // Only this handle remains: the accept worker and every serving thread
    // (each holds a clone) have been joined.
    assert_eq!(
        Arc::strong_count(&serving),
        1,
        "a serving thread was not joined"
    );
    let mut rest = String::new();
    assert_eq!(
        std::io::Read::read_to_string(&mut silent, &mut rest).unwrap(),
        0,
        "the client sees the shut connection"
    );
}

/// ht-poh.11: the agy adapter's cross-leaf paths on this rig (a child module, so it reuses the
/// rig's private items).
#[path = "agy_sweep.rs"]
mod agy_sweep;
