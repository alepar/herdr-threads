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
    harness::{context::Harness, setup::plan_claude},
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
    io::{BufRead, BufReader, Write},
    os::unix::{
        fs::{DirBuilderExt, PermissionsExt},
        net::UnixListener,
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
}
impl ShiftedClock {
    fn new() -> Self {
        Self {
            base: SystemClock::new(),
            shift_ms: AtomicU64::new(0),
        }
    }
    fn advance(&self, millis: u64) {
        self.shift_ms.fetch_add(millis, Ordering::SeqCst);
    }
}
impl Clock for ShiftedClock {
    fn utc_now(&self) -> UtcMillis {
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

/// The private Herdr endpoint: serves the scripted (mutable) panes like the
/// sweep's, answers `agent.prompt` as delivered, and records every call.
struct Host {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    panes: Arc<Mutex<Vec<Value>>>,
    calls: Calls,
}
impl Host {
    fn start(socket: &std::path::Path, panes: Vec<Value>) -> Self {
        let listener = UnixListener::bind(socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let panes = Arc::new(Mutex::new(panes));
        let calls: Calls = Arc::default();
        let (stopped, scripted, log) = (stop.clone(), panes.clone(), calls.clone());
        let worker = std::thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                let mut stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("stand-in Herdr accept: {error}"),
                };
                // macOS rejects these once the peer has hung up.
                if stream.set_nonblocking(false).is_err()
                    || stream
                        .set_read_timeout(Some(Duration::from_secs(10)))
                        .is_err()
                {
                    continue;
                }
                let mut line = String::new();
                if BufReader::new(&mut stream).read_line(&mut line).is_err() || line.is_empty() {
                    continue;
                }
                let Ok(request) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                let method = request["method"].as_str().unwrap_or_default().to_owned();
                log.lock()
                    .unwrap()
                    .push((method.clone(), request["params"].clone()));
                let panes = scripted.lock().unwrap().clone();
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
        });
        Self {
            stop,
            worker: Some(worker),
            panes,
            calls,
        }
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
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
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

fn claude(id: &str, terminal: &str, session: &str) -> Value {
    agent_pane(id, terminal, "claude", Some(session))
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
    daemon: Option<Daemon>,
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

    /// `settings`: the content of the instance's `settings.json`, written
    /// before the daemon starts.
    fn with_settings(settings: Option<Value>) -> Self {
        let guard = crate::ONE_DAEMON.lock().unwrap_or_else(|e| e.into_inner());
        let root = PathBuf::from(format!(
            "/private/tmp/htmd-{}",
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
            daemon: None,
            probe: LaneProbe::default(),
            a: String::new(),
            b: String::new(),
            thread: String::new(),
            _guard: guard,
            _scratch: scratch,
        };
        rig.start_daemon();
        rig.a = rig.seat_resolve(PANE_A);
        rig.b = rig.seat_resolve(PANE_B);
        // A is the sender: a plain pane the native ladder never prompts.
        rig.cli(
            Some((&rig.a, PANE_A)),
            &["check-in", "--lifecycle-event", "a-start"],
        )
        .data("a check-in");
        rig.session_start_hook(PANE_B, SESSION_B, "startup");
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
        let (paths, daemon_stop) = (self.paths.clone(), stop.clone());
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
                    ServiceConfig::default(),
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
        std::panic::set_hook(Box::new(|info| {
            eprintln!("{info}");
            if let Some(path) = std::env::var_os("HT_MOD_DELIVERY_PANIC_LOG")
                && let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path)
            {
                let _ = writeln!(file, "{info}");
            }
        }));
        self.daemon = Some(Daemon { stop, thread });
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
        if json {
            command.arg("--json");
        }
        command
            .arg("--state-dir")
            .arg(&self.state)
            .arg("--host-endpoint")
            .arg(&self.socket)
            .args(args)
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
        let payload: Value = serde_json::from_str(stdin).unwrap();
        let event = payload["hook_event_name"].as_str().unwrap();
        let command = self.claude_hooks[event][0]["hooks"][0]["command"]
            .as_str()
            .unwrap_or_else(|| panic!("no installed Claude registration for {event}"))
            .to_owned();
        let mut child = spawn::command("/bin/sh")
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
        self.channels()["channels"]
            .as_array()?
            .iter()
            .find(|channel| channel["seat"] == self.b.as_str())
            .and_then(|channel| channel["state"].as_str().map(str::to_owned))
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
        let mut args = vec!["watch", "ack", "--session", session, "--via", via];
        args.extend_from_slice(ids);
        let mut command = self.command(Some(PANE_B), &args);
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

    /// A native wake prompt reaches `pane` after `mark`.
    fn wait_prompt(&self, mark: usize, pane: &str) {
        let until = Instant::now() + STEP;
        while self.host.prompts_since(mark, pane) == 0 {
            assert!(
                Instant::now() < until,
                "timed out waiting for a native wake prompt; host calls since the mark: {:?}",
                self.host.calls.lock().unwrap()[mark..]
                    .iter()
                    .map(|(method, params)| format!("{method} {}", params["target"]))
                    .collect::<Vec<_>>()
            );
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
        self.host.set_panes(vec![
            pane(PANE_A, "term-a"),
            claude(PANE_B, "term-b", session),
        ]);
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
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Ok(value) = serde_json::from_str::<Value>(&line)
                    && tx.send(value).is_err()
                {
                    return;
                }
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
    rig.advance(31_000);
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
/// `attention` line carrying the fixed marker, never as an acked item.
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
        line["text"], "herdr-threads: attention pending; run herdr-threads inbox",
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

/// ht-j16.33: an attention line whose invitation was accepted, or whose
/// notices a check-in offered, is retracted with `attention_cleared`.
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
    // (a) an invitation, then B accepts it.
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
    watch.wait_attention_marker();
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

    // (b) a notice published with no live channel, a connect that shows it,
    // then a check-in that offers (settles) it.
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
    let mut watch = rig.watch(SESSION_B);
    watch.wait_connected();
    watch.wait_attention_marker();
    // The check-in offers the pending notice and settles it.
    let shown = wait_until("the notice in the PreToolUse hook", STEP, || {
        let out = rig.hook(PANE_B, &pre_tool_use(SESSION_B));
        out.stdout.contains("offered notices").then_some(out)
    });
    assert!(
        shown.stdout.contains("thread-notice-stale"),
        "{:?}",
        shown.stdout
    );
    watch.wait_line("the retraction", STEP, |l| l["kind"] == "attention_cleared");
    assert_eq!(rig.channel_state().as_deref(), Some("live"));
    assert!(watch.running(), "the watch child died");
}

/// Snapshot of every file below `dir` (the seat's local context hints).
fn snapshot(dir: &std::path::Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn walk(dir: &std::path::Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if let Ok(bytes) = fs::read(&path) {
                out.push((path, bytes));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, &mut out);
    out.sort();
    out
}

/// Spec D2/D6 `/clear`: the lifecycle check-in rotates the binding
/// generation and closes the channel (`binding_changed`); an ack naming the
/// cleared session is `stale_generation` and settles nothing; the new
/// session's watch registers and gets the still-pending item re-streamed.
#[test]
fn clear_rebind_stale_generation_ack_refused_and_restreamed() {
    let rig = Rig::new();
    let mut old = rig.watch(SESSION_B);
    old.wait_connected();
    let id = rig.send_ordinary("pending across /clear");
    old.wait_item(&id);
    rig.set_session_b("SB2");
    rig.session_start_hook(PANE_B, "SB2", "clear");
    let closing = old.wait_line("the binding_changed close", STEP, |line| {
        line["kind"] == "status" && line["state"] == "closing"
    });
    assert_eq!(closing["reason"], "binding_changed", "{closing}");
    assert_eq!(old.exit_code(STEP), 0);
    rig.wait_channel(Some("rebind_grace"));
    let stale = rig.ack_result(SESSION_B, "context", &id);
    assert_eq!(stale["result"], "stale_generation", "{stale}");
    assert_eq!(rig.receipt_state(&id), "pending");
    let mut new = rig.watch("SB2");
    new.wait_connected();
    let line = new.wait_item(&id);
    assert_eq!(
        line["body"], "pending across /clear",
        "re-streamed to the new session"
    );
    rig.wait_channel(Some("live"));
    let settled = rig.ack_result("SB2", "context", &id);
    assert_eq!(settled["result"], "settled", "{settled}");
}

/// Spec D7 rebind grace: the SessionStart check-in that rotated the
/// generation omits its digest (the seat still counts as live), no native
/// prompt follows, and only when the grace lapses without a registration is
/// the wake lane kicked.
#[test]
fn clear_within_rebind_grace_emits_no_session_start_digest_and_no_native_kick() {
    let rig = Rig::new();
    let mut watch = rig.watch(SESSION_B);
    watch.wait_connected();
    let id = rig.send_ordinary("pending across /clear");
    watch.wait_item(&id);
    let mark = rig.host.mark();
    rig.set_session_b("SB2");
    let started = rig.session_start_hook(PANE_B, "SB2", "clear");
    assert!(
        !started.stdout.contains(DIGEST_MARK),
        "digest during rebind grace: {}",
        started.stdout
    );
    rig.wait_channel(Some("rebind_grace"));
    rig.assert_no_prompt(mark, QUIET);
    rig.advance(31_000);
    rig.wait_channel(None);
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

/// Spec D6 resume: `/resume` keeps the native session id but rotates the
/// generation. An ack that still carries the previous generation (the local
/// hint is stale) is accepted for the same native session and settles; the
/// daemon decides against its canonical binding, not the client's claim.
#[test]
fn resume_reack_settles() {
    let rig = Rig::new();
    let mut old = rig.watch(SESSION_B);
    old.wait_connected();
    let id = rig.send_ordinary("delivered before the resume");
    old.wait_item(&id);
    let contexts = rig.paths.instance_dir.join("contexts");
    let before = snapshot(&contexts);
    let generation = rig.channels()["channels"][0]["binding_generation"]
        .as_u64()
        .unwrap();
    rig.session_start_hook(PANE_B, SESSION_B, "resume");
    let closing = old.wait_line("the binding_changed close", STEP, |line| {
        line["kind"] == "status" && line["state"] == "closing"
    });
    assert_eq!(closing["reason"], "binding_changed", "{closing}");
    let mut new = rig.watch(SESSION_B);
    new.wait_connected();
    new.wait_item(&id);
    rig.wait_channel(Some("live"));
    let rotated = rig.channels()["channels"][0]["binding_generation"]
        .as_u64()
        .unwrap();
    assert!(rotated > generation, "resume rotates the generation");
    // The ack process still sees the pre-resume context (previous generation).
    let after = snapshot(&contexts);
    assert_ne!(before, after, "the resume hook updated the local context");
    for (path, _) in &after {
        if !before.iter().any(|(old, _)| old == path) {
            fs::remove_file(path).unwrap();
        }
    }
    for (path, bytes) in &before {
        fs::write(path, bytes).unwrap();
    }
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

/// ht-j16.21: a notice published while the channel is live is offered by the
/// next PreToolUse hook, once; the digest stays suppressed and the channel
/// stays live (spec D7: the live channel carries attention, not notices).
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
        });
    // The notice projection runs on a worker: poll the hook until it shows.
    let shown = wait_until("the notice in the PreToolUse hook", STEP, || {
        let out = rig.hook(PANE_B, &pre_tool_use(SESSION_B));
        out.stdout.contains("offered notices: 1").then_some(out)
    });
    assert!(
        !shown.stdout.contains(DIGEST_MARK),
        "digest while live: {:?}",
        shown.stdout
    );
    assert!(
        !shown.stdout.contains("pending mail"),
        "ready commands while live: {:?}",
        shown.stdout
    );
    assert!(
        shown.stdout.contains("thread-notice-live"),
        "the offer names the notice's thread: {:?}",
        shown.stdout
    );
    let again = rig.hook(PANE_B, &pre_tool_use(SESSION_B));
    assert!(
        !again.stdout.contains("offered notices"),
        "the notice was offered twice: {:?}",
        again.stdout
    );
    assert_eq!(rig.channel_state().as_deref(), Some("live"));
    assert!(watch.running(), "the watch child died");
}

/// Spec D2: watch connections have their own budget of 64 (outside the 32
/// ordinary slots). With 64 seats holding a channel, a 65th registration is
/// refused `busy` (retryable, exit 2) while ordinary requests are still served.
#[test]
fn watch_admission_over_cap_refuses_busy() {
    const CAP: usize = herdr_threads::protocol::watch::MAX_WATCH_CONNECTIONS;
    let rig = Rig::new();
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
    // Register the extra seats through their SessionStart hooks, four at a
    // time: the stand-in Herdr answers one call at a time, so wider batches
    // only queue the hooks' host calls past their budgets (an uncommitted
    // check-in, retried below) and make the registration slower overall.
    for chunk in extra.chunks(4) {
        std::thread::scope(|scope| {
            for (id, session) in chunk {
                let rig = &rig;
                scope.spawn(move || rig.session_start_hook(id, session, "startup"));
            }
        });
    }
    // The holders' `watch` children start sixteen at a time (inside the 32
    // ordinary request slots their registration also uses: all 64 at once
    // drew `daemon_unavailable` refusals); each first answer is judged once
    // its chunk was spawned, and a refusal the mod would retry is retried.
    let spawn_holder = |id: &str, session: &str| {
        Watch::spawn(rig.command(
            Some(id),
            &["watch", "--harness", "claude", "--session", session],
        ))
    };
    let mut holders: Vec<Watch> = Vec::with_capacity(CAP);
    for chunk in extra.chunks(16) {
        let mut spawned: Vec<Watch> = chunk
            .iter()
            .map(|(id, session)| spawn_holder(id, session))
            .collect();
        for (holder, (id, session)) in spawned.iter_mut().zip(chunk) {
            for _ in 0..10 {
                let first = holder.peek_first(STEP);
                let reason = first
                    .as_ref()
                    .filter(|line| line["kind"] == "status" && line["state"] == "refused")
                    .and_then(|line| line["reason"].as_str().map(str::to_owned));
                match reason.as_deref() {
                    // A hook that missed its budget under load left its
                    // check-in uncommitted; run it again, as the next hook
                    // would.
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
        holders.append(&mut spawned);
    }
    assert_eq!(rig.channels()["live_channels"], CAP, "{}", rig.channels());
    let mut over = rig.watch(SESSION_B);
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
        let mut retry = rig.watch(SESSION_B);
        let line = retry.wait_line("a registration answer", STEP, |line| {
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
    const PANE_C: &str = "w1:p3";
    const SESSION_C: &str = "SC";
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
    bystander.assert_no_line("attention before the deadline", QUIET, |l| {
        l["kind"] == "attention"
    });

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
    // Only C misses the deadline: one open condition about C.
    rig.advance(2 * MINUTE_MS);
    wait_until("C's overdue condition to open", STEP, || {
        (conditions() == [None]).then_some(())
    });
    affected.wait_attention_marker();
    bystander.assert_no_line("attention for C's overdue open", QUIET, |l| {
        l["kind"] == "attention"
    });

    // C's late ACK clears it: a clear wakes nobody.
    rig.cli(Some((&c, PANE_C)), &["ack", &id]).data("late ack");
    wait_until("C's overdue condition to clear", STEP, || {
        let now = conditions();
        (now.len() == 1 && now[0].is_some()).then_some(())
    });
    bystander.assert_no_line("attention for C's overdue clear", QUIET, |l| {
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
