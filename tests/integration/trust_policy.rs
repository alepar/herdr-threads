//! B5 trust-policy guards end to end (ht-rzi.8): the installed executable, its
//! detached daemon, the installed hook and a stand-in Herdr that runs as its
//! own process. A new Herdr process is a new server incarnation to the
//! daemon's kernel peer witness, so a restart of the stand-in is exactly the
//! "Herdr incarnation change" of TRUST-POLICY C2 (macOS and Linux: elsewhere the
//! adapter never produces a verified incarnation). No model runs; the agent's
//! hook payloads are the native JSON a harness would write on stdin.
//!
//! The stand-in re-reads its scripted pane file on every request and records
//! every `agent.prompt` it receives, so a test changes what Herdr "sees" (an
//! agent appearing in a pane, a failing read) without restarting it.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use super::sweep::{Scratch, agent_pane, host_reply, pane};
use herdr_threads::test_support::spawn::SpawnOwned;
use herdr_threads::{
    cli::hook::installed_argv,
    harness::{claude::setup::plan_claude, context::Harness},
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::{fs::DirBuilderExt, net::UnixListener},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");
const CHILD_SOCKET: &str = "HT_TRUST_POLICY_HERDR_SOCKET";
const CHILD_PANES: &str = "HT_TRUST_POLICY_HERDR_PANES";
const CHILD_PROMPTS: &str = "HT_TRUST_POLICY_HERDR_PROMPTS";

/// Not a test: the entry point of the stand-in Herdr process. The parent
/// re-executes this test binary with the environment below; run normally (no
/// environment) it does nothing.
#[test]
fn stand_in_herdr_process_entry() {
    let Ok(socket) = std::env::var(CHILD_SOCKET) else {
        return;
    };
    let panes_file = PathBuf::from(std::env::var(CHILD_PANES).unwrap());
    let prompts_file = PathBuf::from(std::env::var(CHILD_PROMPTS).unwrap());
    let _ = fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let parent = unsafe { libc::getppid() };
    loop {
        // An orphaned stand-in (the test crashed) exits instead of lingering.
        if unsafe { libc::getppid() } != parent {
            std::process::exit(0);
        }
        let mut stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }
            Err(error) => panic!("stand-in Herdr accept: {error}"),
        };
        // macOS rejects these (EINVAL) once the peer has already closed: a
        // readiness probe that connects and hangs up is not a request.
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
        let panes: Vec<Value> = fs::read(&panes_file)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let reply = if request["method"] == "agent.prompt" {
            let target = request["params"]["target"].clone();
            let mut record = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&prompts_file)
                .unwrap();
            writeln!(
                record,
                "{}",
                json!({"target":target,"text":request["params"]["text"]})
            )
            .unwrap();
            match panes.iter().find(|pane| pane["pane_id"] == target) {
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
}

/// The stand-in Herdr as a child process. `restart` is a new server process,
/// which the daemon sees as a new incarnation.
struct Herdr {
    socket: PathBuf,
    panes: PathBuf,
    prompts: PathBuf,
    child: Option<herdr_threads::test_support::spawn::OwnedChild>,
}
impl Herdr {
    fn start(root: &Path, panes: Vec<Value>) -> Self {
        let mut herdr = Self {
            socket: root.join("herdr.sock"),
            panes: root.join("panes.json"),
            prompts: root.join("prompts.jsonl"),
            child: None,
        };
        herdr.set_panes(panes);
        herdr.spawn(); // leak-guard: fixture method, spawns via spawn_owned
        herdr
    }
    fn spawn(&mut self) {
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "trust_policy::stand_in_herdr_process_entry",
                "--exact",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD_SOCKET, &self.socket)
            .env(CHILD_PANES, &self.panes)
            .env(CHILD_PROMPTS, &self.prompts)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn_owned()
            .unwrap();
        self.child = Some(child);
        let until = Instant::now() + Duration::from_secs(20);
        while std::os::unix::net::UnixStream::connect(&self.socket).is_err() {
            assert!(Instant::now() < until, "stand-in Herdr never listened");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    fn kill(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
    /// A new Herdr server process serving `panes` (a new incarnation).
    fn restart(&mut self, panes: Vec<Value>) {
        self.kill();
        self.set_panes(panes);
        self.spawn(); // leak-guard: fixture method, spawns via spawn_owned
    }
    fn set_panes(&self, panes: Vec<Value>) {
        let temporary = self.panes.with_extension("tmp");
        fs::write(&temporary, serde_json::to_vec(&panes).unwrap()).unwrap();
        fs::rename(temporary, &self.panes).unwrap();
    }
    fn prompts(&self) -> Vec<Value> {
        fs::read_to_string(&self.prompts)
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}
impl Drop for Herdr {
    fn drop(&mut self) {
        self.kill();
    }
}

/// An agent record for a Claude pane whose Herdr integration reports `session`.
fn claude(id: &str, terminal: &str, session: &str) -> Value {
    agent_pane(id, terminal, "claude", Some(session))
}

/// A stand-in occupant: the cooperative caller flags for one seat and pane.
#[derive(Clone, Copy)]
struct Caller<'a> {
    seat: &'a str,
    pane: &'a str,
    harness: &'a str,
}

struct Output {
    code: i32,
    value: Value,
    stderr: String,
}
impl Output {
    /// `result.data` of a successful call.
    fn data(&self, what: &str) -> Value {
        assert_eq!(self.code, 0, "{what} failed: {}{}", self.stderr, self.value);
        self.value["result"]["data"].clone()
    }
    /// `result.kind` of a successful call (`me init` and `check-in` answer with it).
    fn kind(&self, what: &str) -> String {
        assert_eq!(self.code, 0, "{what} failed: {}{}", self.stderr, self.value);
        self.value["result"]["kind"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }
    fn text(&self, what: &str) -> String {
        self.data(what).as_str().unwrap().to_owned()
    }
    fn refused(&self, what: &str) -> &str {
        assert_ne!(self.code, 0, "{what} must be refused: {}", self.value);
        &self.stderr
    }
}

/// The harness session running in `pane`: pane 1 is the real Claude session `SA`, whose hook
/// registered it; the stand-in agents of the other panes each have a session of their own.
fn session_of(pane: &str, seat: &str) -> String {
    if pane == "w1:p1" {
        "SA".to_owned()
    } else {
        format!("trust-{seat}")
    }
}

struct World {
    root: PathBuf,
    state: PathBuf,
    herdr: Herdr,
    claude_hooks: Value,
    codex_hook: String,
    /// The reconciliation marker recorded before the latest restart, so a
    /// wait cannot be satisfied by the previous daemon's stale marker.
    stale_marker: std::cell::RefCell<Option<(String, i64)>>,
    /// Seats whose stand-in caller runs the session their hook registered, so the caller's
    /// commands reconnect that binding instead of enrolling a second one.
    sessions: std::cell::RefCell<std::collections::HashMap<String, String>>,
    /// Extra environment for every command, so the daemon `ensure` starts inherits it.
    env: Vec<(String, String)>,
    _scratch: Scratch,
}
impl World {
    fn start(prefix: &str, panes: Vec<Value>) -> Self {
        Self::start_with_wake_batch_delay(prefix, panes, None)
    }

    fn start_with_wake_batch_delay(
        prefix: &str,
        panes: Vec<Value>,
        wake_batch_delay_ms: Option<u64>,
    ) -> Self {
        Self::start_full(prefix, panes, wake_batch_delay_ms, Vec::new())
    }

    /// `env` reaches every command and the daemon it starts.
    fn start_with_env(prefix: &str, panes: Vec<Value>, env: &[(&str, &str)]) -> Self {
        let env = env
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        Self::start_full(prefix, panes, None, env)
    }

    fn start_full(
        prefix: &str,
        panes: Vec<Value>,
        wake_batch_delay_ms: Option<u64>,
        env: Vec<(String, String)>,
    ) -> Self {
        let root = herdr_threads::test_support::scratch::temp_path(format!(
            "{prefix}-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        ));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let scratch = Scratch(root.clone());
        let state = root.join("state");
        // Executable availability is metadata only; these wrappers must not
        // be invoked to admit installed lifecycle/tool registrations.
        let bin = root.join("bin");
        fs::create_dir_all(&bin).unwrap();
        for name in ["claude", "codex"] {
            use std::os::unix::fs::PermissionsExt;
            let path = bin.join(name);
            fs::write(&path, "#!/bin/sh\nexit 99\n").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let argv = installed_argv(BIN, Some(state.to_str().unwrap()), None, Harness::Claude);
        let plan = plan_claude(b"{}", &argv).unwrap();
        let settings: Value = serde_json::from_slice(&plan.proposed_bytes).unwrap();
        let claude_hooks = settings["hooks"].clone();
        let codex_argv = installed_argv(BIN, Some(state.to_str().unwrap()), None, Harness::Codex);
        let codex_hook = herdr_threads::harness::setup::shell_command(&codex_argv).unwrap();
        let herdr = Herdr::start(&root, panes);
        let world = Self {
            root,
            state,
            herdr,
            claude_hooks,
            codex_hook,
            stale_marker: std::cell::RefCell::new(None),
            sessions: std::cell::RefCell::new(Default::default()),
            env,
            _scratch: scratch,
        };
        if let Some(delay) = wake_batch_delay_ms {
            use herdr_threads::daemon::paths::{NodePaths, RuntimeContext};
            use std::os::unix::fs::PermissionsExt;
            let context =
                RuntimeContext::explicit(world.state.clone(), world.herdr.socket.clone(), None)
                    .unwrap();
            let paths = NodePaths::resolve(&context.state_dir).unwrap();
            paths.prepare_node_dir().unwrap();
            let settings_path = paths.node_dir.join("settings.json");
            fs::write(
                &settings_path,
                serde_json::to_vec(&json!({"wake_batch_delay_ms": delay})).unwrap(),
            )
            .unwrap();
            fs::set_permissions(settings_path, fs::Permissions::from_mode(0o600)).unwrap();
        }
        world.ensure();
        world
    }
    fn path(&self) -> String {
        format!("{}:/usr/bin:/bin", self.root.join("bin").display())
    }
    /// `pane`: the invoking shell's `HERDR_PANE_ID`.
    fn run(&self, pane: Option<&str>, caller: Option<Caller>, args: &[&str]) -> Output {
        self.run_with_env(pane, caller, args, &[])
    }
    fn run_with_env(
        &self,
        pane: Option<&str>,
        caller: Option<Caller>,
        args: &[&str],
        env: &[(&str, &str)],
    ) -> Output {
        let mut command = Command::new(BIN); // leak-guard: tagged on the next line via spawn::tag
        herdr_threads::test_support::spawn::tag(&mut command);
        let args = if args.first() == Some(&"human") {
            command.arg("human");
            &args[1..]
        } else {
            args
        };
        command
            .arg("--json")
            .arg("--state-dir")
            .arg(&self.state)
            .arg("--host-endpoint")
            .arg(&self.herdr.socket);
        if let Some(caller) = caller {
            command.args([
                "--cooperative-seat",
                caller.seat,
                "--cooperative-target",
                caller.pane,
                "--cooperative-harness",
                caller.harness,
                "--cooperative-role",
                "top-level",
            ]);
        }
        // The runner's own agent environment must not reach the A4 guard.
        for (key, _) in std::env::vars() {
            if key.starts_with("CODEX_") {
                command.env_remove(key);
            }
        }
        // A stand-in seat is an agent: it acts as a harness session (decision 15).
        if let Some(caller) = &caller {
            herdr_threads::test_support::isolation::as_agent(
                &mut command,
                caller.harness,
                &self
                    .sessions
                    .borrow()
                    .get(caller.seat)
                    .cloned()
                    .unwrap_or_else(|| session_of(caller.pane, caller.seat)),
            );
        }
        command
            .args(args)
            .env_remove("CLAUDECODE")
            .env_remove("HERDR_PLUGIN_STATE_DIR")
            .env_remove("HERDR_SOCKET_PATH")
            .env_remove("HERDR_PANE_ID")
            .env_remove("HERDR_BIN_PATH")
            .env_remove("HERDR_ENV")
            .env("PATH", self.path())
            .env("CLAUDE_CONFIG_DIR", self.root.join("claude-config"));
        if let Some(pane) = pane {
            command.env("HERDR_PANE_ID", pane);
        }
        command.envs(self.env.iter().map(|(key, value)| (key, value)));
        command.envs(env.iter().copied());
        let output = command.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        Output {
            code: output.status.code().unwrap_or(-1),
            value: serde_json::from_str(&stdout).unwrap_or(Value::Null),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }
    fn ensure(&self) -> Value {
        self.run(None, None, &["daemon", "ensure"])
            .data("daemon ensure")
    }
    fn stop(&self) {
        self.run(None, None, &["daemon", "stop"])
            .data("daemon stop");
    }
    /// A daemon restart on the same Herdr (a new daemon boot only).
    fn restart_daemon(&self) -> Value {
        *self.stale_marker.borrow_mut() = self.marker();
        self.stop();
        self.ensure()
    }
    /// The Herdr server restarts with `panes` (a new incarnation) and the
    /// daemon comes up against it.
    fn restart_herdr(&mut self, panes: Vec<Value>) -> Value {
        *self.stale_marker.borrow_mut() = self.marker();
        self.stop();
        self.herdr.restart(panes);
        self.ensure()
    }
    /// The Herdr server restarts with `panes` (a new incarnation) while the
    /// daemon keeps running: nothing here waits for its next capture or
    /// reconciliation pass, so a hook fired now races both.
    fn restart_herdr_keeping_daemon(&mut self, panes: Vec<Value>) {
        *self.stale_marker.borrow_mut() = self.marker();
        self.herdr.restart(panes);
    }
    /// Pending `*.intent` files in the hook's intent journal.
    fn intents(&self) -> usize {
        self.find("intents").map_or(0, |dir| {
            fs::read_dir(dir).map_or(0, |entries| {
                entries
                    .flatten()
                    .filter(|e| e.path().extension().is_some_and(|x| x == "intent"))
                    .count()
            })
        })
    }
    /// The installed hook, exactly as setup installs it: `sh -c` with the
    /// native JSON on stdin and the pane identity from `HERDR_*`.
    fn hook(&self, harness: &str, pane: &str, stdin: &[u8]) -> (i32, String, String) {
        self.hook_with_env(harness, pane, stdin, &[])
    }
    /// A hook whose continuity claim cannot be decided (Herdr is down): it runs with the
    /// production bounds, so its retry window is the real 5 s, not the loaded-suite tenfold.
    fn hook_unanswered(&self, harness: &str, pane: &str, stdin: &[u8]) {
        let scale = herdr_threads::protocol::time::TEST_TIMEOUT_SCALE_ENV;
        let _ = self.hook_with_env(harness, pane, stdin, &[(scale, "1")]);
    }
    fn hook_with_env(
        &self,
        harness: &str,
        pane: &str,
        stdin: &[u8],
        env: &[(&str, &str)],
    ) -> (i32, String, String) {
        let command = if harness == "codex" {
            self.codex_hook.as_str()
        } else {
            let payload = serde_json::from_slice::<Value>(stdin).unwrap();
            let event = payload["hook_event_name"].as_str().unwrap();
            self.claude_hooks[event][0]["hooks"][0]["command"]
                .as_str()
                .unwrap_or_else(|| panic!("no installed Claude registration for {event}"))
        };
        let mut shell = Command::new("/bin/sh");
        // The installed hook runs under its harness (the test process stands in for it).
        // The hook's process is its harness session's: the session named in the payload.
        let session = serde_json::from_slice::<Value>(stdin)
            .ok()
            .and_then(|payload| payload["session_id"].as_str().map(str::to_owned))
            .unwrap_or_else(|| "hook-session".to_owned());
        herdr_threads::test_support::isolation::as_agent(&mut shell, harness, &session);
        let mut child = shell
            .envs([herdr_threads::daemon::lifecycle::test_owner_env()])
            .arg("-c")
            .arg(command)
            .env("HERDR_ENV", "1")
            .env("HERDR_PANE_ID", pane)
            .env("HERDR_SOCKET_PATH", &self.herdr.socket)
            .env("PATH", self.path())
            .env_remove("HERDR_PLUGIN_STATE_DIR")
            .env_remove("HERDR_BIN_PATH")
            .envs(self.env.iter().map(|(key, value)| (key, value)))
            .envs(env.iter().copied())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn_owned()
            .unwrap();
        child.stdin.take().unwrap().write_all(stdin).unwrap();
        let output = child.wait_with_output().unwrap();
        (
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }
    /// The first file or directory called `name` below the state directory.
    fn find(&self, name: &str) -> Option<PathBuf> {
        fn walk(dir: &Path, name: &str) -> Option<PathBuf> {
            for entry in fs::read_dir(dir).ok()?.flatten() {
                let path = entry.path();
                if path.file_name().is_some_and(|candidate| candidate == name) {
                    return Some(path);
                }
                if path.is_dir()
                    && let Some(found) = walk(&path, name)
                {
                    return Some(found);
                }
            }
            None
        }
        walk(&self.state, name)
    }
    fn database(&self) -> rusqlite::Connection {
        let path = self.find("herdr-threads.db").expect("daemon database");
        let db = rusqlite::Connection::open_with_flags(
            &path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        db.busy_timeout(Duration::from_secs(5)).unwrap();
        db
    }
    fn count(&self, sql: &str) -> i64 {
        self.database().query_row(sql, [], |r| r.get(0)).unwrap()
    }
    fn wait_for(&self, what: &str, done: impl FnMut(&Self) -> bool) {
        self.wait_within(what, Duration::from_secs(30), done);
    }
    /// After Herdr comes back under a daemon that kept running, the observation lane's
    /// capped backoff (30 s) comes first: kept claims are decided after its next capture.
    fn wait_for_recovery(&self, what: &str, done: impl FnMut(&Self) -> bool) {
        self.wait_within(what, Duration::from_secs(90), done);
    }
    fn wait_within(&self, what: &str, within: Duration, mut done: impl FnMut(&Self) -> bool) {
        let until = Instant::now() + within;
        while !done(self) {
            assert!(Instant::now() < until, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    /// `(reconciled_boot, reconciled_epoch)` once a pass has recorded one.
    fn marker(&self) -> Option<(String, i64)> {
        self.database()
            .query_row(
                "SELECT reconciled_boot,reconciled_epoch FROM herdr_hosts WHERE reconciled_boot IS NOT NULL",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok()
    }
    /// The daemon's reconciliation pass for the current recovery boot/epoch has
    /// recorded its marker (the precondition of every hold lift), and the
    /// marker is not the one a previous daemon left behind.
    fn wait_reconciled(&self) {
        self.wait_for("the reconciliation marker", |world| {
            world.count(
                "SELECT count(*) FROM herdr_hosts WHERE active_snapshot_id IS NOT NULL \
                 AND reconciled_boot IS NOT NULL AND reconciled_boot=recovery_boot \
                 AND reconciled_epoch=recovery_epoch",
            ) == 1
                && *world.stale_marker.borrow() != world.marker()
        });
    }
    fn hold_unclaimed(&self) -> i64 {
        self.count("SELECT baseline_hold_unclaimed FROM herdr_hosts")
    }
    fn seat_state(&self, seat: &str) -> String {
        self.database()
            .query_row("SELECT state FROM seats WHERE id=?1", [seat], |r| r.get(0))
            .unwrap()
    }
    /// `(harness, provenance, native_session)` of the seat's open binding.
    fn open_binding(&self, seat: &str) -> Vec<(String, String, String)> {
        self.database()
            .prepare("SELECT harness,observation_provenance,native_session FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL")
            .unwrap()
            .query_map([seat], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }
    /// Resolve `pane` to its seat through ordinary resolution.
    fn resolve(&self, pane: &str) -> String {
        self.run(None, None, &["seat", "resolve", "--pane", pane])
            .text("seat resolve")
    }
    fn pending(&self, seat: &str) -> Vec<Value> {
        self.run(
            None,
            None,
            &["pending-receipts", "--seat", seat, "--limit", "100"],
        )
        .data("pending-receipts")["items"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }
    fn warnings(&self, seat: &str) -> Vec<Value> {
        self.run(None, None, &["warnings", "--seat", seat, "--limit", "100"])
            .data("warnings")["items"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }
    /// The seat's history rows from `seat inspect`.
    fn inspect(&self, seat: &str) -> Value {
        self.run(None, None, &["seat", "inspect", seat])
            .data("seat inspect")
    }
}

/// The native SessionStart JSON for `harness` on stdin.
fn session_start(harness: &str, session: &str, source: &str) -> Vec<u8> {
    match harness {
        "codex" => format!(
            r#"{{"session_id":"{session}","turn_id":"t1","hook_event_name":"SessionStart","source":"{source}"}}"#
        ),
        _ => format!(
            r#"{{"session_id":"{session}","transcript_path":"/tmp/t.jsonl","cwd":"/tmp","hook_event_name":"SessionStart","source":"{source}"}}"#
        ),
    }
    .into_bytes()
}

/// `(decision_kind, operator_label, continuity_diagnostic)` of every repair
/// row in the seat's inspect history, oldest first.
fn repairs(world: &World, seat: &str) -> Vec<(String, Option<String>, Option<String>)> {
    world.inspect(seat)["history"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["kind"] == "repair")
        .map(|item| {
            let data = &item["data"];
            (
                data["decision_kind"].as_str().unwrap().to_owned(),
                data["operator_label"].as_str().map(str::to_owned),
                data["continuity_diagnostic"].as_str().map(str::to_owned),
            )
        })
        .collect()
}

/// Seats named by the thread's `recipient_unavailable` warnings.
fn unavailable_seats(world: &World, thread: &str) -> Vec<String> {
    world
        .run(None, None, &["read", thread, "--limit", "100"])
        .data("read")["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["kind"] == "warn")
        .filter_map(|item| {
            let preview: Value = serde_json::from_str(item["preview_data"].as_str()?).ok()?;
            (preview["event"] == "recipient_unavailable")
                .then(|| preview["seat"].as_str().unwrap().to_owned())
        })
        .collect()
}

/// F6 walking skeleton (TRUST-POLICY C1-C4): a Herdr incarnation change leaves
/// three joined seats unresolved and every unowned target held; a resumed
/// Claude session reattaches its own seat; one collision is abandoned by
/// retiring the old seat, the other by `--replace`; the last unresolved seat
/// resolving lifts the hold, ordinary resolution works again, and a message
/// to the reattached seat is sent with no unavailability warning.
#[test]
fn f6_walking_skeleton() {
    let mut world = World::start(
        "htf6",
        vec![
            claude("w1:p1", "term-a", "SA"),
            pane("w1:p2", "term-b"),
            pane("w1:p3", "term-c"),
            pane("w1:p5", "term-e"),
        ],
    );
    let (a, b, c) = (
        world.resolve("w1:p1"),
        world.resolve("w1:p2"),
        world.resolve("w1:p3"),
    );
    let (a_pane, b_pane, c_pane) = (
        Caller {
            seat: &a,
            pane: "w1:p1",
            harness: "claude",
        },
        Caller {
            seat: &b,
            pane: "w1:p2",
            harness: "claude",
        },
        Caller {
            seat: &c,
            pane: "w1:p3",
            harness: "claude",
        },
    );
    // A is a real Claude session: its SessionStart hook registers it.
    let (code, _, stderr) =
        world.hook("claude", "w1:p1", &session_start("claude", "SA", "startup"));
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(
        world.open_binding(&a),
        vec![("claude".into(), "cooperative_top_level".into(), "SA".into())]
    );
    for (caller, event) in [(b_pane, "b-start"), (c_pane, "c-start")] {
        world
            .run(
                None,
                Some(caller),
                &["check-in", "--lifecycle-event", event],
            )
            .data("check-in");
    }
    let thread = world
        .run(None, Some(a_pane), &["thread", "create", "--topic", "f6"])
        .text("thread create");
    for (caller, seat) in [(b_pane, &b), (c_pane, &c)] {
        world
            .run(None, Some(a_pane), &["invite", &thread, "--seat", seat])
            .data("invite");
        world
            .run(None, Some(caller), &["accept", &thread])
            .data("accept");
    }
    world
        .run(
            None,
            Some(a_pane),
            &["send", &thread, "--body", "for b", "--require-ack", &b],
        )
        .data("send to b");
    world
        .run(
            None,
            Some(a_pane),
            &["send", &thread, "--body", "for c", "--require-ack", &c],
        )
        .data("send to c");
    assert_eq!(world.hold_unclaimed(), 0, "no restore yet, nothing is held");
    for seat in [&b, &c] {
        assert!(!world.pending(seat).is_empty(), "{seat} owes receipts");
    }

    // Herdr restarts: a new server process, new terminals, the same pane names.
    world.restart_herdr(vec![
        claude("w1:p1", "term-a2", "SA"),
        pane("w1:p2", "term-b2"),
        pane("w1:p3", "term-c2"),
        pane("w1:p5", "term-e2"),
    ]);
    world.wait_reconciled();
    for seat in [&a, &b, &c] {
        assert_eq!(world.seat_state(seat), "unresolved", "{seat}");
    }
    assert_eq!(
        world.hold_unclaimed(),
        1,
        "the baseline holds unowned targets"
    );
    let held = world.run(None, None, &["seat", "resolve", "--pane", "w1:p5"]);
    held.refused("ordinary resolution of a held target");
    assert_eq!(
        world.count("SELECT count(*) FROM seats WHERE state='resolved'"),
        0
    );

    // A's resumed session reattaches its own seat (C1), nothing else moves.
    let (code, _, stderr) = world.hook("claude", "w1:p1", &session_start("claude", "SA", "resume"));
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(world.seat_state(&a), "resolved");
    assert_eq!(
        world.open_binding(&a),
        vec![("claude".into(), "cooperative_top_level".into(), "SA".into())]
    );
    assert_eq!(
        repairs(&world, &a).last(),
        Some(&("cooperative_continuity".into(), None, Some("match".into())))
    );
    assert_eq!(world.seat_state(&b), "unresolved");
    assert_eq!(world.seat_state(&c), "unresolved");
    assert_eq!(world.hold_unclaimed(), 1, "B and C still need an operator");

    // B's restored pane got a new role (N1): rebind is refused with both
    // abandonment argv, and retiring B settles its obligation as retired.
    let n1 = world
        .run(
            None,
            None,
            &[
                "human",
                "seat",
                "resolve",
                "--pane",
                "w1:p2",
                "--new-seat",
                "--operator",
            ],
        )
        .text("fresh seat for B's pane");
    let refused = world.run(
        None,
        None,
        &[
            "human",
            "seat",
            "rebind",
            &b,
            "--pane",
            "w1:p2",
            "--operator",
        ],
    );
    let text = refused.refused("rebind onto an owned target").to_owned();
    assert!(text.contains(&n1), "{text}");
    assert!(
        text.contains(&format!("herdr-threads human seat retire {b} --operator")),
        "{text}"
    );
    assert!(
        text.contains(&format!(
            "herdr-threads human seat rebind {b} --pane w1:p2 --replace {n1} --operator"
        )),
        "{text}"
    );
    assert_eq!(
        world.seat_state(&b),
        "unresolved",
        "a refusal changes nothing"
    );
    world
        .run(None, None, &["human", "seat", "retire", &b, "--operator"])
        .data("seat retire");
    assert_eq!(world.seat_state(&b), "retired");
    assert_eq!(world.seat_state(&n1), "resolved");

    // C's pane is owned by N2: abandon the new role instead.
    let n2 = world
        .run(
            None,
            None,
            &[
                "human",
                "seat",
                "resolve",
                "--pane",
                "w1:p3",
                "--new-seat",
                "--operator",
            ],
        )
        .text("fresh seat for C's pane");
    assert_eq!(world.hold_unclaimed(), 1, "C is still unresolved");
    world
        .run(
            None,
            None,
            &[
                "human",
                "seat",
                "rebind",
                &c,
                "--pane",
                "w1:p3",
                "--replace",
                &n2,
                "--operator",
            ],
        )
        .data("rebind --replace");
    assert_eq!(world.seat_state(&c), "resolved");
    assert_eq!(world.seat_state(&n2), "retired");

    // Nothing unresolved is left: the daemon's reconciliation pass lifts the hold.
    assert_eq!(
        world.count("SELECT count(*) FROM seats WHERE state='unresolved'"),
        0
    );
    world.wait_for("the baseline hold to lift", |world| {
        world.hold_unclaimed() == 0
    });
    assert_eq!(
        world.count("SELECT count(*) FROM recovery_holds WHERE released_at IS NULL"),
        0
    );
    let label = format!("operator:local-user:{}", unsafe { libc::geteuid() });
    assert_eq!(
        repairs(&world, &b).last(),
        Some(&("operator_retire".into(), Some(label.clone()), None))
    );
    assert_eq!(
        repairs(&world, &c).last().map(|row| row.0.as_str()),
        Some("operator_rebind")
    );
    assert!(
        world.pending(&b).is_empty(),
        "B's obligations settled as recipient-retired and moved to nobody"
    );
    assert_eq!(
        world.pending(&c).len(),
        1,
        // Only the explicit `--require-ack` recipient of "for c" owes a receipt (D9).
        "C was rebound, not replaced: it still owes what it owed"
    );
    assert!(world.pending(&a).is_empty());

    // Ordinary resolution and `me init` work in the previously held pane.
    let me = world.run(Some("w1:p5"), None, &["human", "me", "init"]);
    assert_eq!(me.kind("me init"), "checked_in");
    let d = world.resolve("w1:p5");
    let human = world.open_binding(&d);
    assert_eq!(human.len(), 1);
    assert_eq!(
        (human[0].0.as_str(), human[0].1.as_str()),
        ("human", "operator_human")
    );
    world
        .run(None, Some(a_pane), &["invite", &thread, "--seat", &d])
        .data("invite D");
    world
        .run(Some("w1:p5"), None, &["human", "accept", &thread])
        .data("D accepts");

    // D writes to the reattached, joined A: A is available, so the send
    // starts its receipt timer and prepares no unavailability warning.
    let sent = world
        .run(
            Some("w1:p5"),
            None,
            &[
                "human",
                "send",
                &thread,
                "--body",
                "welcome back",
                "--require-ack",
                &a,
            ],
        )
        .text("send to the reattached seat");
    let owed = world
        .pending(&a)
        .into_iter()
        .find(|item| item["message"] == sent.as_str())
        .expect("A owes the receipt");
    assert!(
        owed["available_at"].as_u64().is_some() && owed["deadline"].as_u64().is_some(),
        "A's receipt timer started at the send: {owed}"
    );
    // The only seat the send found unavailable is C (rebound, not yet
    // checked in); A, reattached by its resumed session, is not named.
    assert_eq!(unavailable_seats(&world, &thread), vec![c.clone()]);
}

/// Seat A on `w1:p1` registered by a real hook for `harness`/`SA`, then a Herdr
/// incarnation change leaves it unresolved, and its resumed session reattaches
/// it. `restored` is what the restored Herdr shows for `w1:p1`.
fn reattached(harness: &str, before: Value, restored: Value) -> (World, String) {
    let mut world = World::start("htre", vec![before, pane("w1:p2", "term-b")]);
    let a = world.resolve("w1:p1");
    let (code, _, stderr) = world.hook(harness, "w1:p1", &session_start(harness, "SA", "startup"));
    assert_eq!(code, 0, "{stderr}");
    world.restart_herdr(vec![restored, pane("w1:p2", "term-b2")]);
    world.wait_reconciled();
    assert_eq!(world.seat_state(&a), "unresolved");
    let (code, _, stderr) = world.hook(harness, "w1:p1", &session_start(harness, "SA", "resume"));
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(world.seat_state(&a), "resolved");
    assert_eq!(
        world.open_binding(&a),
        vec![(harness.into(), "cooperative_top_level".into(), "SA".into())]
    );
    (world, a)
}

/// TRUST-POLICY C1 (ht-rzi.18): the daemon is kept running across a Herdr
/// restart that renumbers the panes, and the resumed session's hook fires in
/// its new pane before the daemon has finished reconciling the new
/// incarnation: the saved seat is not yet unresolved, so "no unresolved seat"
/// is pending, not final. The hook retries under one operation key until the
/// daemon has reconciled, and the seat reattaches in one transaction that also
/// opens its successor binding on the restored pane.
#[test]
fn resume_before_reconciliation_with_daemon_kept_running_is_retried_then_reattaches() {
    let mut world = World::start(
        "htrk",
        vec![claude("w1:p1", "term-a", "SA"), pane("w1:p2", "term-b")],
    );
    let a = world.resolve("w1:p1");
    let (code, _, stderr) =
        world.hook("claude", "w1:p1", &session_start("claude", "SA", "startup"));
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(world.seat_state(&a), "resolved");
    // The restored Herdr numbers the session's pane w1:p3; the daemon is not
    // stopped, and nothing waits for its next capture or reconciliation.
    world.restart_herdr_keeping_daemon(vec![
        claude("w1:p3", "term-c", "SA"),
        pane("w1:p2", "term-b2"),
    ]);
    let (code, _, stderr) = world.hook("claude", "w1:p3", &session_start("claude", "SA", "resume"));
    assert_eq!(code, 0, "{stderr}");
    world.wait_reconciled();
    assert_eq!(world.seat_state(&a), "resolved", "{stderr}");
    assert_eq!(
        world
            .count("SELECT count(*) FROM allocation_decisions WHERE kind='cooperative_continuity'"),
        1,
        "{stderr}"
    );
    assert_eq!(
        world.open_binding(&a),
        vec![("claude".into(), "cooperative_top_level".into(), "SA".into())],
        "{stderr}"
    );
    assert_eq!(
        world.count(&format!(
            "SELECT count(*) FROM seats WHERE id='{a}' AND target_id='w1:p3'"
        )),
        1
    );
    assert_eq!(world.intents(), 0, "the continuity intent is finished");
}

/// TRUST-POLICY C1 x C2 (ht-p63): as above, but the restored Herdr gives the
/// resumed session's pane the SAME id it had before (w1:p1), and the daemon
/// keeps running. The resume hook fires before the daemon has noticed the new
/// incarnation, so the pane still looks resolved to the stale mapping. The
/// seat must not end unresolved with its own pane held: the resumed session
/// reattaches by cooperative continuity on the restored pane.
#[test]
fn resume_on_same_pane_id_before_reconciliation_with_daemon_kept_running_reattaches() {
    let mut world = World::start(
        "htsp",
        vec![claude("w1:p1", "term-a", "SA"), pane("w1:p2", "term-b")],
    );
    let a = world.resolve("w1:p1");
    let (code, _, stderr) =
        world.hook("claude", "w1:p1", &session_start("claude", "SA", "startup"));
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(world.seat_state(&a), "resolved");
    // The restored Herdr reuses the pane id w1:p1 on a new terminal; the
    // daemon is not stopped, and nothing waits for its next capture.
    world.restart_herdr_keeping_daemon(vec![
        claude("w1:p1", "term-a2", "SA"),
        pane("w1:p2", "term-b2"),
    ]);
    let (code, _, stderr) = world.hook("claude", "w1:p1", &session_start("claude", "SA", "resume"));
    assert_eq!(code, 0, "{stderr}");
    world.wait_reconciled();
    assert_eq!(world.seat_state(&a), "resolved", "{stderr}");
    assert_eq!(
        world
            .count("SELECT count(*) FROM allocation_decisions WHERE kind='cooperative_continuity'"),
        1,
        "{stderr}"
    );
    assert_eq!(
        world.open_binding(&a),
        vec![("claude".into(), "cooperative_top_level".into(), "SA".into())],
        "{stderr}"
    );
    assert_eq!(
        world.count(&format!(
            "SELECT count(*) FROM seats WHERE id='{a}' AND target_id='w1:p1'"
        )),
        1
    );
    assert_eq!(world.hold_unclaimed(), 0, "the restored pane is not held");
    assert_eq!(world.intents(), 0, "the continuity intent is finished");
}

/// TRUST-POLICY A4 (agent to human) x C1 (ht-rzi.2 x ht-rzi.3): the seat a
/// resumed session reattached holds a `cooperative_top_level` binding, so a
/// person's `me init` over it is refused. Herdr reports no agent in the pane,
/// so the refusal comes from the reattached seat itself: first from the
/// daemon's own check of the binding (A2: client-local hints never authorize; an
/// agent saves none). The refusal does not touch the binding; `--operator`
/// overrides.
#[test]
fn me_init_over_reattached_seat_is_refused() {
    let (world, a) = reattached(
        "claude",
        claude("w1:p1", "term-a", "SA"),
        pane("w1:p1", "term-a2"),
    );
    let before = world.open_binding(&a);
    // An agent saves no client-local context, so only the daemon can refuse.
    let by_daemon = world.run(Some("w1:p1"), None, &["human", "me", "init"]);
    let text = by_daemon.refused("me init over a reattached agent seat");
    assert!(!text.contains("belongs to a Claude agent"), "{text}");
    assert!(text.contains("never replaces an agent's binding"), "{text}");
    assert_eq!(world.open_binding(&a), before);

    // The documented override is the local account's explicit decision.
    let overridden = world.run(Some("w1:p1"), None, &["human", "me", "init", "--operator"]);
    assert_eq!(overridden.kind("me init --operator"), "checked_in");
    assert_eq!(world.open_binding(&a)[0].0, "human");
}

/// TRUST-POLICY A4 (second agent) x C1 (ht-rzi.2 x ht-rzi.4): `launch` onto the
/// pane of a seat that a resumed session reattached is refused while Herdr
/// reports the bound agent live there. The reachable shape is `launch --pane`
/// naming the reattached seat's own pane: ordinary resolution maps it to the
/// seat, whose open binding is `cooperative_top_level` on that pane.
#[test]
fn launch_onto_reattached_seat_with_live_agent_is_refused() {
    let (world, a) = reattached(
        "claude",
        claude("w1:p1", "term-a", "SA"),
        claude("w1:p1", "term-a2", "SA"),
    );
    let refused = world.run(
        None,
        None,
        &["launch", "--pane", "w1:p1", "--kind", "claude"],
    );
    let text = refused
        .refused("launch onto a live agent's seat")
        .to_owned();
    assert!(text.contains(&a), "{text}");
    assert!(text.contains("second agent"), "{text}");
    // Control: the same launch where Herdr reports no agent is not stopped by
    // that guard (it proceeds to the hook-configuration preflight).
    world
        .herdr
        .set_panes(vec![pane("w1:p1", "term-a2"), pane("w1:p2", "term-b2")]);
    let proceeds = world.run(
        None,
        None,
        &["launch", "--pane", "w1:p1", "--kind", "claude"],
    );
    let text = proceeds
        .refused("launch without hooks configured")
        .to_owned();
    assert!(!text.contains("second agent"), "{text}");
}

/// `(generation, host_epoch, registered, ended)` of every binding row of `seat`.
fn binding_rows(world: &World, seat: &str) -> Vec<(i64, i64, bool, bool)> {
    world
        .database()
        .prepare("SELECT generation,host_epoch,registered_at IS NOT NULL,ended_at IS NOT NULL FROM occupant_bindings WHERE seat_id=?1 ORDER BY ordinal")
        .unwrap()
        .query_map([seat], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// O1 carry-forward x C1 (ht-rzi.5 x ht-rzi.2): a daemon restart on the same
/// Herdr carries the open binding forward with its harness session id, so a
/// later Herdr incarnation change can still match the resumed session to the
/// seat. A carry-forward that dropped or rewrote the session would leave the
/// resume with no unique match and the seat unresolved.
#[test]
fn carried_forward_binding_still_matches_session_after_later_incarnation_change() {
    let mut world = World::start(
        "htcf",
        vec![claude("w1:p1", "term-a", "SA"), pane("w1:p2", "term-b")],
    );
    let a = world.resolve("w1:p1");
    let (code, _, stderr) =
        world.hook("claude", "w1:p1", &session_start("claude", "SA", "startup"));
    assert_eq!(code, 0, "{stderr}");
    let registered = binding_rows(&world, &a);
    assert_eq!(registered.len(), 1);

    // A daemon restart: same Herdr, same terminals, a new daemon boot and epoch.
    world.restart_daemon();
    world.wait_reconciled();
    assert_eq!(world.seat_state(&a), "resolved");
    let carried = binding_rows(&world, &a);
    assert_eq!(carried.len(), 1, "no new binding row: {carried:?}");
    assert_eq!(carried[0].0, registered[0].0, "the same binding generation");
    assert!(
        carried[0].1 > registered[0].1,
        "moved to the new host epoch: {registered:?} -> {carried:?}"
    );
    assert!(carried[0].2 && !carried[0].3, "still registered and open");
    assert_eq!(
        world.open_binding(&a),
        vec![("claude".into(), "cooperative_top_level".into(), "SA".into())],
        "the carried binding is the same agent session"
    );
    assert_eq!(world.hold_unclaimed(), 0);

    // Later the Herdr incarnation changes; the resumed session finds its seat.
    world.restart_herdr(vec![
        claude("w1:p1", "term-a2", "SA"),
        pane("w1:p2", "term-b2"),
    ]);
    world.wait_reconciled();
    assert_eq!(world.seat_state(&a), "unresolved");
    assert_eq!(world.hold_unclaimed(), 1);
    let (code, _, stderr) = world.hook("claude", "w1:p1", &session_start("claude", "SA", "resume"));
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(world.seat_state(&a), "resolved");
    assert_eq!(
        repairs(&world, &a).last(),
        Some(&("cooperative_continuity".into(), None, Some("match".into())))
    );
    assert_eq!(world.hold_unclaimed(), 0);
}

/// P35 expected boot x O1 carry-forward (ht-rzi.5): after `daemon stop` and
/// `daemon ensure` through the binary, the CLI addresses the new daemon boot
/// and the joined recipient's availability carried forward, so a send exits 0
/// with no `unknown_outcome` and no unavailability warning.
#[test]
fn expected_boot_and_carry_forward_give_no_unknown_outcome_across_stop_ensure() {
    let world = World::start(
        "htbt",
        vec![pane("w1:p1", "term-a"), pane("w1:p2", "term-b")],
    );
    let (a, b) = (world.resolve("w1:p1"), world.resolve("w1:p2"));
    let (a_pane, b_pane) = (
        Caller {
            seat: &a,
            pane: "w1:p1",
            harness: "claude",
        },
        Caller {
            seat: &b,
            pane: "w1:p2",
            harness: "claude",
        },
    );
    for (caller, event) in [(a_pane, "a-start"), (b_pane, "b-start")] {
        world
            .run(
                None,
                Some(caller),
                &["check-in", "--lifecycle-event", event],
            )
            .data("check-in");
    }
    let thread = world
        .run(None, Some(a_pane), &["thread", "create", "--topic", "boot"])
        .text("thread create");
    world
        .run(None, Some(a_pane), &["invite", &thread, "--seat", &b])
        .data("invite");
    world
        .run(None, Some(b_pane), &["accept", &thread])
        .data("accept");

    let registered = binding_rows(&world, &b);
    let first = world.ensure()["boot_id"].clone();
    // The old daemon's marker must not satisfy `wait_reconciled` before the
    // new daemon's own first pass (merge note: the combined tree's startup
    // order exposed this race; `restart_daemon` records it the same way).
    *world.stale_marker.borrow_mut() = world.marker();
    world.stop();
    let second = world.ensure()["boot_id"].clone();
    assert_ne!(first, second, "a genuinely new daemon boot");

    // Carry-forward is the daemon's first reconciliation pass, not `ensure`'s.
    world.wait_reconciled();
    let carried = binding_rows(&world, &b);
    assert_eq!(carried.len(), 1, "{carried:?}");
    assert_eq!(carried[0].0, registered[0].0);
    assert!(carried[0].1 > registered[0].1 && carried[0].2 && !carried[0].3);
    let sent = world.run(
        None,
        Some(a_pane),
        &[
            "send",
            &thread,
            "--body",
            "after the restart",
            "--require-ack",
            &b,
        ],
    );
    assert_eq!(sent.code, 0, "{}{}", sent.stderr, sent.value);
    for text in [sent.stderr.clone(), sent.value.to_string()] {
        assert!(
            !text.contains("unknown_outcome") && !text.contains("UnknownOutcome"),
            "{text}"
        );
    }
    let message = sent.text("send");
    let owed = world
        .pending(&b)
        .into_iter()
        .find(|item| item["message"] == message.as_str())
        .expect("B owes the receipt");
    assert!(
        owed["available_at"].as_u64().is_some(),
        "B stayed available across the restart: {owed}"
    );
    assert!(unavailable_seats(&world, &thread).is_empty());
    assert!(world.warnings(&a).is_empty() && world.warnings(&b).is_empty());
}

/// C4 pre-reconciliation window (ht-rzi.19): a send immediately after
/// `daemon ensure`, without waiting for the first reconciliation pass, to a
/// joined seat whose binding the pass will carry. The send exits 0 and writes
/// no recipient_unavailable warning; once the pass has run the receipt's timer
/// exists. (Whether the send lands before or after the pass is a race; the
/// deterministic pre-pass case is
/// `send_before_first_pass_to_structurally_continuous_seat_has_no_warning`.)
#[test]
fn send_right_after_ensure_before_reconciliation_pass_gives_no_warning_and_a_timer() {
    let world = World::start(
        "htpp",
        vec![pane("w1:p1", "term-a"), pane("w1:p2", "term-b")],
    );
    let (a, b) = (world.resolve("w1:p1"), world.resolve("w1:p2"));
    let (a_pane, b_pane) = (
        Caller {
            seat: &a,
            pane: "w1:p1",
            harness: "claude",
        },
        Caller {
            seat: &b,
            pane: "w1:p2",
            harness: "claude",
        },
    );
    for (caller, event) in [(a_pane, "a-start"), (b_pane, "b-start")] {
        world
            .run(
                None,
                Some(caller),
                &["check-in", "--lifecycle-event", event],
            )
            .data("check-in");
    }
    let thread = world
        .run(
            None,
            Some(a_pane),
            &["thread", "create", "--topic", "prepass"],
        )
        .text("thread create");
    let invited = world.run(None, Some(a_pane), &["invite", &thread, "--seat", &b]);
    invited.data("invite");
    // A wakeable (Claude) invitee is never reported as `wake=no_wake` (ht-wake-outcome).
    assert!(!invited.stderr.contains("wake="), "{}", invited.stderr);
    world
        .run(None, Some(b_pane), &["accept", &thread])
        .data("accept");

    world.stop();
    world.ensure();
    // No wait_reconciled before the send.
    let sent = world.run(
        None,
        Some(a_pane),
        &[
            "send",
            &thread,
            "--body",
            "right after ensure",
            "--require-ack",
            &b,
        ],
    );
    assert_eq!(sent.code, 0, "{}{}", sent.stderr, sent.value);
    assert!(!sent.stderr.contains("wake="), "{}", sent.stderr);
    assert!(
        !sent.stderr.contains("recipient_unavailable")
            && !sent.value.to_string().contains("recipient_unavailable"),
        "{}{}",
        sent.stderr,
        sent.value
    );
    let message = sent.text("send");
    world.wait_reconciled();
    let owed = world
        .pending(&b)
        .into_iter()
        .find(|item| item["message"] == message.as_str())
        .expect("B owes the receipt");
    assert!(
        owed["available_at"].as_u64().is_some(),
        "the carry-forward started the receipt timer: {owed}"
    );
    assert!(unavailable_seats(&world, &thread).is_empty());
    assert!(world.warnings(&a).is_empty() && world.warnings(&b).is_empty());
}

/// C2 (nothing left to protect) x C1 structural reconfirm: a daemon restart on
/// the same Herdr incarnation reconfirms every saved seat from its terminal,
/// so no seat is unresolved and no hold is taken for any target, including an
/// unowned pane that the baseline covers.
#[test]
fn restore_with_every_seat_structurally_reconfirmed_leaves_no_hold() {
    let world = World::start(
        "htrc",
        vec![
            pane("w1:p1", "term-a"),
            pane("w1:p2", "term-b"),
            pane("w1:p3", "term-c"),
        ],
    );
    let (a, b) = (world.resolve("w1:p1"), world.resolve("w1:p2"));
    world.restart_daemon();
    world.wait_reconciled();
    assert_eq!(world.seat_state(&a), "resolved");
    assert_eq!(world.seat_state(&b), "resolved");
    assert_eq!(world.hold_unclaimed(), 0);
    assert_eq!(
        world.count("SELECT count(*) FROM recovery_holds WHERE released_at IS NULL"),
        0
    );
    // An unowned pane resolves at once: nothing is held.
    let c = world.resolve("w1:p3");
    assert_eq!(world.seat_state(&c), "resolved");
}

/// C1 with no Herdr hint, then A4 wake (ht-rzi.2 x ht-rzi.4): two Codex
/// sessions are reattached through the installed hook after a Herdr
/// incarnation change, neither with an `agent_session` from Herdr (one pane
/// shows no agent record, the other a `codex` agent with no session), so the
/// diagnostic is `absent` for both and the reattachment is unaffected. A
/// message to each then wakes it. Herdr shows no agent in the first pane, so
/// that wake is refused and nothing is typed there; in the second a `codex`
/// agent is idle, so the wake prompt is submitted to it alone.
#[test]
fn codex_reattachment_without_herdr_hint_then_wake() {
    // Continuity and wake safety need an immediate first attempt here; initial
    // batching is tested independently and retry spacing stays unchanged.
    let mut world = World::start_with_wake_batch_delay(
        "htcx",
        vec![
            pane("w1:p1", "term-a"),
            pane("w1:p2", "term-b"),
            pane("w1:p3", "term-c"),
        ],
        Some(0),
    );
    let (x, y) = (world.resolve("w1:p1"), world.resolve("w1:p3"));
    for (pane, session) in [("w1:p1", "SX"), ("w1:p3", "SY")] {
        let (code, _, stderr) =
            world.hook("codex", pane, &session_start("codex", session, "startup"));
        assert_eq!(code, 0, "{stderr}");
    }
    world.restart_herdr(vec![pane("w1:p1", "term-a2"), pane("w1:p2", "term-b2"), {
        // Herdr's foreground process of the pane is the session's own process: the
        // composer wake types only to the binding with that fingerprint pid.
        let mut agent = agent_pane("w1:p3", "term-c2", "codex", None);
        agent["foreground_pid"] =
            json!(herdr_threads::test_support::isolation::stand_in_fingerprint("codex", "SY").pid);
        agent
    }]);
    world.wait_reconciled();
    for seat in [&x, &y] {
        assert_eq!(world.seat_state(seat), "unresolved");
    }
    for (pane, session) in [("w1:p1", "SX"), ("w1:p3", "SY")] {
        let (code, _, stderr) =
            world.hook("codex", pane, &session_start("codex", session, "resume"));
        assert_eq!(code, 0, "{stderr}");
    }
    for (seat, session) in [(&x, "SX"), (&y, "SY")] {
        assert_eq!(world.seat_state(seat), "resolved");
        assert_eq!(
            world.open_binding(seat),
            vec![(
                "codex".into(),
                "cooperative_top_level".into(),
                session.into()
            )]
        );
        assert_eq!(
            repairs(&world, seat).last(),
            Some(&("cooperative_continuity".into(), None, Some("absent".into())))
        );
    }
    // The seats' commands run the sessions their hooks registered: one binding per seat, so the
    // composer wake has the one target to match against Herdr's foreground process.
    for (seat, session) in [(&x, "SX"), (&y, "SY")] {
        world
            .sessions
            .borrow_mut()
            .insert(seat.clone(), session.to_owned());
    }
    // Both unresolved seats are back: the hold is lifted and a new pane resolves.
    world.wait_for("the baseline hold to lift", |world| {
        world.hold_unclaimed() == 0
    });
    let b = world.resolve("w1:p2");
    let b_pane = Caller {
        seat: &b,
        pane: "w1:p2",
        harness: "claude",
    };
    world
        .run(
            None,
            Some(b_pane),
            &["check-in", "--lifecycle-event", "b-start"],
        )
        .data("check-in");
    let thread = world
        .run(None, Some(b_pane), &["thread", "create", "--topic", "wake"])
        .text("thread create");
    for (seat, pane) in [(&x, "w1:p1"), (&y, "w1:p3")] {
        world
            .run(None, Some(b_pane), &["invite", &thread, "--seat", seat])
            .data("invite");
        let codex = Caller {
            seat,
            pane,
            harness: "codex",
        };
        world
            .run(None, Some(codex), &["accept", &thread])
            .data("accept");
    }
    for seat in [&x, &y] {
        world
            .run(
                None,
                Some(b_pane),
                &["send", &thread, "--body", "wake up", "--require-ack", seat],
            )
            .data("send");
    }

    // The first wake attempt for each seat is made right away.
    let outcome = |seat: &str| -> Option<String> {
        world
            .database()
            .query_row(
                "SELECT last_outcome FROM wake_work WHERE seat_id=?1 AND completed_at_utc IS NOT NULL",
                [seat],
                |r| r.get(0),
            )
            .ok()
    };
    // Wait for the codex seat's row to record the delivered prompt: the stand-in
    // Herdr records a prompt before the daemon commits that attempt's outcome,
    // and an earlier attempt made before Herdr detected the codex agent may have
    // left `unsafe` (ht-ddp, under load).
    world.wait_for("both wake attempts", |world| {
        outcome(&x).is_some()
            && outcome(&y).as_deref() == Some("submitted")
            && !world.herdr.prompts().is_empty()
    });
    let prompts = world.herdr.prompts();
    assert_eq!(prompts.len(), 1, "{prompts:?}");
    assert_eq!(
        prompts[0]["target"], "w1:p3",
        "only the pane with a codex agent"
    );
    assert_ne!(
        outcome(&x),
        outcome(&y),
        "refused where no agent is detected"
    );
    eprintln!(
        "DBG outcomes {:?} {:?} {:?}",
        outcome(&x),
        outcome(&y),
        prompts
    );
}

/// The wake receipt (ht-wake-loop) through the installed hook and the detached daemon: two
/// hook-registered Codex seats are woken for an ACK request each. One session makes a daemon
/// call after the wake; the other never does, as when Codex no longer runs our hooks. When the
/// first ladder retry comes due (30 s) the silent seat's wake is recorded `unreceived` and not
/// typed again, while the seat that was heard from keeps its ladder and is woken again.
#[test]
fn a_wake_with_no_session_contact_before_its_retry_is_unreceived_and_not_retyped() {
    let codex = |id: &str, terminal: &str, session: &str| {
        let mut agent = agent_pane(id, terminal, "codex", None);
        agent["foreground_pid"] = json!(
            herdr_threads::test_support::isolation::stand_in_fingerprint("codex", session).pid
        );
        agent
    };
    let world = World::start_with_wake_batch_delay(
        "htwr",
        vec![
            codex("w1:p1", "term-a", "SX"),
            codex("w1:p2", "term-b", "SY"),
            pane("w1:p3", "term-c"),
        ],
        Some(0),
    );
    let (x, y) = (world.resolve("w1:p1"), world.resolve("w1:p2"));
    for (pane, session, seat) in [("w1:p1", "SX", &x), ("w1:p2", "SY", &y)] {
        let (code, _, stderr) =
            world.hook("codex", pane, &session_start("codex", session, "startup"));
        assert_eq!(code, 0, "{stderr}");
        world
            .sessions
            .borrow_mut()
            .insert(seat.clone(), session.to_owned());
    }
    let b = world.resolve("w1:p3");
    let b_pane = Caller {
        seat: &b,
        pane: "w1:p3",
        harness: "claude",
    };
    world
        .run(
            None,
            Some(b_pane),
            &["check-in", "--lifecycle-event", "b-start"],
        )
        .data("check-in");
    // Each woken seat creates its own thread and invites the sender: no invitation (a p0 that
    // wakes) ever reaches the seats under test, so the only wakes are the ACK requests'.
    let mut threads = Vec::new();
    for (seat, pane) in [(&x, "w1:p1"), (&y, "w1:p2")] {
        let codex = Caller {
            seat,
            pane,
            harness: "codex",
        };
        let thread = world
            .run(None, Some(codex), &["thread", "create", "--topic", "wake"])
            .text("thread create");
        world
            .run(None, Some(codex), &["invite", &thread, "--seat", &b])
            .data("invite");
        world
            .run(None, Some(b_pane), &["accept", &thread])
            .data("accept");
        threads.push(thread);
    }
    let thread = threads[0].clone();
    for (seat, thread) in [(&x, &threads[0]), (&y, &threads[1])] {
        world
            .run(
                None,
                Some(b_pane),
                &["send", thread, "--body", "ping", "--require-ack", seat],
            )
            .data("send");
    }
    let outcome = |seat: &str| -> Option<String> {
        world
            .database()
            .query_row(
                "SELECT last_outcome FROM wake_work WHERE seat_id=?1 AND completed_at_utc IS NOT NULL",
                [seat],
                |r| r.get(0),
            )
            .ok()
    };
    world.wait_for("both wakes submitted", |world| {
        outcome(&x).as_deref() == Some("submitted")
            && outcome(&y).as_deref() == Some("submitted")
            && world.herdr.prompts().len() == 2
    });
    // Seat X's session ran after the wake: a claim-bearing read reaches the daemon and leaves
    // the ACK request owed (a turn-start hook would also serve it in-turn).
    let x_codex = Caller {
        seat: &x,
        pane: "w1:p1",
        harness: "codex",
    };
    world
        .run(None, Some(x_codex), &["summary", &thread])
        .data("summary");

    let prompts_to = |pane: &str| {
        world
            .herdr
            .prompts()
            .iter()
            .filter(|prompt| prompt["target"] == pane)
            .count()
    };
    let until = Instant::now() + Duration::from_secs(60);
    while outcome(&y).as_deref() != Some("unreceived") || prompts_to("w1:p1") < 2 {
        assert!(
            Instant::now() < until,
            "retry verdicts: X {:?} ({} prompts), Y {:?}",
            outcome(&x),
            prompts_to("w1:p1"),
            outcome(&y)
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    // A few more lane passes: the silent seat is not typed to again.
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(prompts_to("w1:p2"), 1, "{:?}", world.herdr.prompts());
    assert_eq!(outcome(&y).as_deref(), Some("unreceived"));
    assert_eq!(outcome(&x).as_deref(), Some("submitted"));
}

/// Seam of the pane-agent observation port (ht-rzi.9) across `me init`
/// (ht-rzi.3), `launch` (ht-rzi.4) and the continuity diagnostic (ht-rzi.2):
/// one scripted Herdr answer per pane -- an agent with a session, no agent
/// record, a failing read -- reaches all three consumers as the same value.
/// Present: `me init` and `launch` refuse naming the agent and the diagnostic
/// is `match`. Absent: neither sees an agent and the diagnostic is `absent`.
/// Error: neither guesses. `launch` is the required guard and fails on the
/// read; `me init` is advisory (A4's daemon refusal is the guard), so a failed
/// read is not evidence and the seat's own agent context stops it instead. The
/// diagnostic is `read_error`, which never stops the reattachment.
#[test]
fn pane_agent_observation_reads_the_same_in_me_init_launch_and_diagnostics() {
    let failing = |id: &str, terminal: &str| {
        let mut value = pane(id, terminal);
        value["agent_get_error"] = json!("permission_denied");
        value
    };
    let mut world = World::start(
        "htsm",
        vec![
            claude("w1:p1", "term-a", "S1"),
            pane("w1:p2", "term-b"),
            failing("w1:p3", "term-c"),
        ],
    );
    let seats: Vec<String> = ["w1:p1", "w1:p2", "w1:p3"]
        .iter()
        .map(|pane| world.resolve(pane))
        .collect();
    for (pane, session) in [("w1:p1", "S1"), ("w1:p2", "S2"), ("w1:p3", "S3")] {
        let (code, _, stderr) =
            world.hook("claude", pane, &session_start("claude", session, "startup"));
        assert_eq!(code, 0, "{stderr}");
    }
    let launch = |pane: &str| {
        let out = world.run(None, None, &["launch", "--pane", pane, "--kind", "claude"]);
        (out.refused("launch").to_owned(), out.value.clone())
    };
    let me_init = |pane: &str| {
        let out = world.run(Some(pane), None, &["human", "me", "init"]);
        out.refused("me init").to_owned()
    };
    let (present_me, absent_me, error_me) = (me_init("w1:p1"), me_init("w1:p2"), me_init("w1:p3"));
    let ((present_launch, _), (absent_launch, absent_report), (error_launch, _)) =
        (launch("w1:p1"), launch("w1:p2"), launch("w1:p3"));
    // Present: both guards name the agent Herdr reported.
    assert!(
        present_me.contains("Herdr reports a `claude` agent"),
        "{present_me}"
    );
    assert!(
        present_launch.contains("live claude agent"),
        "{present_launch}"
    );
    // Absent: neither saw an agent (`me init` was stopped later, by the seat's
    // own agent context; `launch` went past its agent guard to the native start, which
    // advisory hook readiness no longer vetoes (TRUST-POLICY, 673a27ad), and the
    // stand-in Herdr leaves unconfirmed).
    assert!(!absent_me.contains("Herdr reports"), "{absent_me}");
    assert!(!absent_launch.contains("agent"), "{absent_launch}");
    assert_eq!(absent_report["launch"]["pane"], "w1:p2", "{absent_report}");
    assert_eq!(
        absent_report["launch"]["outcome"], "outcome_unknown",
        "{absent_report}"
    );
    // Error: neither guesses. `launch` fails on the read itself; `me init`
    // treats the failed read as no evidence and is stopped by the daemon's own
    // check of the agent seat's binding.
    assert!(!error_me.contains("Herdr reports"), "{error_me}");
    assert!(!error_me.contains("scripted read error"), "{error_me}");
    assert!(
        error_me.contains("never replaces an agent's binding"),
        "{error_me}"
    );
    assert!(
        error_launch.contains("scripted read error"),
        "{error_launch}"
    );

    world.restart_herdr(vec![
        claude("w1:p1", "term-a2", "S1"),
        pane("w1:p2", "term-b2"),
        failing("w1:p3", "term-c2"),
    ]);
    world.wait_reconciled();
    for (pane, session) in [("w1:p1", "S1"), ("w1:p2", "S2"), ("w1:p3", "S3")] {
        let (code, _, stderr) =
            world.hook("claude", pane, &session_start("claude", session, "resume"));
        assert_eq!(code, 0, "{stderr}");
    }
    let diagnostics: Vec<_> = seats
        .iter()
        .map(|seat| {
            assert_eq!(
                world.seat_state(seat),
                "resolved",
                "{seat}: a read never blocks"
            );
            repairs(&world, seat).last().unwrap().2.clone().unwrap()
        })
        .collect();
    assert_eq!(diagnostics, ["match", "absent", "read_error"]);
}

/// The mod's `watch` child for `session` in `pane`, as the Claude mod runs it inside that
/// session's process: its stdout lines arrive on the returned channel.
struct WatchChild {
    child: herdr_threads::test_support::spawn::OwnedChild,
    lines: std::sync::mpsc::Receiver<Value>,
}
impl WatchChild {
    /// The next JSON line within `within`.
    fn next(&self, within: Duration) -> Option<Value> {
        self.lines.recv_timeout(within).ok()
    }
}
impl Drop for WatchChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl World {
    fn watch(&self, pane: &str, session: &str) -> WatchChild {
        let mut command = Command::new(BIN); // leak-guard: spawned via spawn_owned below
        herdr_threads::test_support::isolation::as_agent(&mut command, "claude", session);
        let mut child = command
            .envs([herdr_threads::daemon::lifecycle::test_owner_env()])
            .args(["--state-dir"])
            .arg(&self.state)
            .arg("--host-endpoint")
            .arg(&self.herdr.socket)
            .args(["watch", "--harness", "claude", "--session", session])
            .env("HERDR_ENV", "1")
            .env("HERDR_PANE_ID", pane)
            .env("HERDR_SOCKET_PATH", &self.herdr.socket)
            .env("PATH", self.path())
            .env("CLAUDE_CONFIG_DIR", self.root.join("claude-config"))
            .env_remove("HERDR_PLUGIN_STATE_DIR")
            .env_remove("HERDR_BIN_PATH")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn_owned()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, lines) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Ok(value) = serde_json::from_str::<Value>(&line)
                    && sender.send(value).is_err()
                {
                    break;
                }
            }
        });
        WatchChild { child, lines }
    }
    /// Run `watch` the way the mod does until it connects: a refusal is retried after a
    /// short pause (the mod's backoff), at most `attempts` times.
    fn watch_until_connected(&self, pane: &str, session: &str, attempts: usize) -> WatchChild {
        for _ in 0..attempts {
            let watch = self.watch(pane, session);
            if let Some(first) = watch.next(Duration::from_secs(20))
                && first["kind"] == "status"
                && first["state"] == "connected"
            {
                return watch;
            }
            drop(watch);
            std::thread::sleep(Duration::from_millis(300));
        }
        panic!("watch for {session} in {pane} never connected");
    }
    /// One target read of the other pane, so the daemon's observation lane captures the
    /// restarted Herdr now instead of after its capped backoff. Its own outcome (refused as
    /// held, or a seat for that pane) is not what the caller checks.
    fn kick_observation(&self) {
        let _ = self.run(None, None, &["seat", "resolve", "--pane", "w1:p2"]);
    }
    fn kept_claims(&self) -> i64 {
        self.count("SELECT count(*) FROM kept_continuity_claims")
    }
    fn continuity_decisions(&self) -> i64 {
        self.count("SELECT count(*) FROM allocation_decisions WHERE kind='cooperative_continuity'")
    }
}

/// ht-dtx0 acceptance 1 and D1b: a Herdr restart leaves the seat of an idle Claude mod
/// session unresolved and the daemon gone; the session makes no hook call. The mod's own
/// `watch` ensures a daemon, claims the session's seat (C1, `same_session`) and registers its
/// channel; a p0 send then reaches that channel.
#[test]
fn idle_mod_session_reattaches_through_its_watch_and_is_woken() {
    let mut world = World::start(
        "htdw",
        vec![claude("w1:p1", "term-a", "SA"), pane("w1:p2", "term-b")],
    );
    let (a, b) = (world.resolve("w1:p1"), world.resolve("w1:p2"));
    let (a_pane, b_pane) = (
        Caller {
            seat: &a,
            pane: "w1:p1",
            harness: "claude",
        },
        Caller {
            seat: &b,
            pane: "w1:p2",
            harness: "claude",
        },
    );
    let (code, _, stderr) =
        world.hook("claude", "w1:p1", &session_start("claude", "SA", "startup"));
    assert_eq!(code, 0, "{stderr}");
    world
        .run(
            None,
            Some(b_pane),
            &["check-in", "--lifecycle-event", "b-start"],
        )
        .data("check-in");
    let thread = world
        .run(None, Some(b_pane), &["thread", "create", "--topic", "dtx0"])
        .text("thread create");
    world
        .run(None, Some(b_pane), &["invite", &thread, "--seat", &a])
        .data("invite");
    world
        .run(None, Some(a_pane), &["accept", &thread])
        .data("accept");

    world.restart_herdr(vec![
        claude("w1:p1", "term-a2", "SA"),
        pane("w1:p2", "term-b2"),
    ]);
    world.wait_reconciled();
    assert_eq!(world.seat_state(&a), "unresolved");
    // The daemon is gone (as it was when pane teardown killed it): nothing else runs.
    world.stop();

    // Another session in the pane claims nothing: its watch stays refused and the seat
    // unresolved (it starts the daemon, D1b).
    let other = world.watch("w1:p1", "S2");
    let first = other
        .next(Duration::from_secs(20))
        .expect("the S2 watch reports");
    assert_eq!(
        (first["state"].as_str(), first["reason"].as_str()),
        (Some("refused"), Some("no_binding")),
        "{first}"
    );
    drop(other);
    assert_eq!(world.seat_state(&a), "unresolved");
    let watch = world.watch_until_connected("w1:p1", "SA", 20);
    assert_eq!(world.seat_state(&a), "resolved");
    assert_eq!(
        world.open_binding(&a),
        vec![("claude".into(), "cooperative_top_level".into(), "SA".into())]
    );
    assert_eq!(world.continuity_decisions(), 1);
    assert_eq!(
        world.count(&format!(
            "SELECT count(*) FROM seats WHERE id='{a}' AND target_id='w1:p1'"
        )),
        1
    );

    // B gets its seat back from the operator and sends A an ACK request (p0).
    world
        .run(
            None,
            None,
            &[
                "human",
                "seat",
                "rebind",
                &b,
                "--pane",
                "w1:p2",
                "--operator",
            ],
        )
        .data("rebind B");
    world
        .run(
            None,
            Some(b_pane),
            &["check-in", "--lifecycle-event", "b-again"],
        )
        .data("check-in");
    world
        .run(
            None,
            Some(b_pane),
            &["send", &thread, "--body", "wake", "--require-ack", &a],
        )
        .data("send");
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let line = watch
            .next(deadline.saturating_duration_since(Instant::now()))
            .expect("the ACK request reached the reattached session's channel");
        if line["kind"] == "message" && line["body"] == "wake" {
            assert_eq!(line["ack_required"], true, "{line}");
            break;
        }
    }
}

/// ht-mw10: a full reboot. The daemon is down across the Herdr restart, so it never observes
/// the new incarnation live: when it next starts, the store still maps the seat, resolved, to
/// the restored pane id. The resumed session's SessionStart reached no daemon (none could
/// start), so nothing is kept; the session sits idle and only the mod's `watch` runs, restarted
/// after every exit as the mod does. Its runs ensure the daemon (D1b) and, once the first pass
/// has unresolved the seat, claim it (D1). No run may end with a status the mod treats as
/// terminal for the session.
#[test]
fn reboot_with_daemon_down_idle_mod_session_reattaches_through_its_watch() {
    let mut world = World::start(
        "htrb",
        vec![claude("w1:p1", "term-a", "SA"), pane("w1:p2", "term-b")],
    );
    let a = world.resolve("w1:p1");
    let (code, _, stderr) =
        world.hook("claude", "w1:p1", &session_start("claude", "SA", "startup"));
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(world.seat_state(&a), "resolved");
    // The reboot: the daemon goes first, Herdr comes back as a new incarnation with the
    // same pane ids, and no daemon runs until a watch ensures one.
    *world.stale_marker.borrow_mut() = world.marker();
    world.stop();
    world.herdr.restart(vec![
        claude("w1:p1", "term-a2", "SA"),
        pane("w1:p2", "term-b2"),
    ]);
    assert_eq!(
        world.seat_state(&a),
        "resolved",
        "nothing observed the restart"
    );
    assert_eq!(world.kept_claims(), 0);

    // `register.js` stops for the session on exit 3 with one of these reasons.
    const TERMINAL: [&str; 6] = [
        "not_claude",
        "disabled",
        "replaced",
        "no_pane",
        "env_disabled",
        "unsupported",
    ];
    let reattached =
        |world: &World| world.seat_state(&a) == "resolved" && world.continuity_decisions() >= 1;
    let mut seen = Vec::new();
    let mut live = None;
    for _ in 0..30 {
        let watch = world.watch("w1:p1", "SA");
        let mut connected = false;
        loop {
            // A connected run is never restarted by the mod: it must end by itself (closed by
            // the pass that unresolves its seat) or be the reattached one.
            let within = Duration::from_secs(if connected { 60 } else { 20 });
            let Some(line) = watch.next(within) else {
                assert!(
                    !connected || reattached(&world),
                    "a connected watch stayed open on an unresolved seat: {seen:?}"
                );
                break;
            };
            if line["kind"] != "status" {
                continue;
            }
            seen.push(format!("{}:{}", line["state"], line["reason"]));
            assert!(
                !(line["exit"] == 3 && TERMINAL.contains(&line["reason"].as_str().unwrap_or(""))),
                "the mod would stop for the session: {seen:?}"
            );
            if line["state"] != "connected" {
                break;
            }
            connected = true;
            if reattached(&world) {
                break;
            }
        }
        if reattached(&world) {
            live = Some(watch);
            break;
        }
        drop(watch);
        std::thread::sleep(Duration::from_millis(300));
    }
    assert!(live.is_some(), "the seat was never reattached: {seen:?}");
    assert_eq!(
        world.open_binding(&a),
        vec![("claude".into(), "cooperative_top_level".into(), "SA".into())],
        "{seen:?}"
    );
}

/// ht-dtx0 acceptance 2: Herdr is down while a resumed session's SessionStart reaches the
/// daemon, so the claim is refused as not ready and the hook gives up. The daemon keeps the
/// claim and, once Herdr is back and the new incarnation reconciled, reattaches the seat with
/// no further call from the session.
fn resume_claim_while_herdr_is_down(harness: &str, prefix: &str) {
    let mut world = World::start(
        prefix,
        vec![
            agent_pane("w1:p1", "term-a", harness, Some("SA")),
            pane("w1:p2", "term-b"),
        ],
    );
    let a = world.resolve("w1:p1");
    let (code, _, stderr) = world.hook(harness, "w1:p1", &session_start(harness, "SA", "startup"));
    assert_eq!(code, 0, "{stderr}");
    world.herdr.kill();
    world.hook_unanswered(harness, "w1:p1", &session_start(harness, "SA", "resume"));
    assert_eq!(world.kept_claims(), 1, "the claim is kept");
    assert_eq!(world.continuity_decisions(), 0);
    world.herdr.set_panes(vec![
        agent_pane("w1:p1", "term-a2", harness, Some("SA")),
        pane("w1:p2", "term-b2"),
    ]);
    world.herdr.spawn(); // leak-guard: fixture method, spawns via spawn_owned
    world.kick_observation();
    world.wait_for_recovery("the kept claim to reattach the seat", |world| {
        world.seat_state(&a) == "resolved" && world.kept_claims() == 0
    });
    assert_eq!(world.continuity_decisions(), 1);
    assert_eq!(
        world.open_binding(&a),
        vec![(harness.into(), "cooperative_top_level".into(), "SA".into())]
    );
    // The session's own later resubmission replays the decision.
    let (code, _, stderr) = world.hook(harness, "w1:p1", &session_start(harness, "SA", "resume"));
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(world.continuity_decisions(), 1);
}

#[test]
fn claude_resume_claim_while_herdr_is_down_is_kept_and_decided() {
    resume_claim_while_herdr_is_down("claude", "htkc");
}

#[test]
fn codex_resume_claim_while_herdr_is_down_is_kept_and_decided() {
    resume_claim_while_herdr_is_down("codex", "htkx");
}

/// ht-dtx0 review: a process that resumes S in pane P (seat A) and then `/resume`s into T,
/// seat B's session, before the daemon has reconciled, leaves only T's claim kept. S's older
/// row never reattaches A to the pane now running T; B reattaches there instead.
#[test]
fn a_resume_into_another_seats_session_supersedes_the_kept_claim() {
    let mut world = World::start(
        "htks",
        vec![
            claude("w1:p1", "term-a", "SA"),
            claude("w1:p2", "term-b", "TB"),
        ],
    );
    let a = world.resolve("w1:p1");
    let b = world.resolve("w1:p2");
    for (pane, session) in [("w1:p1", "SA"), ("w1:p2", "TB")] {
        let (code, _, stderr) =
            world.hook("claude", pane, &session_start("claude", session, "startup"));
        assert_eq!(code, 0, "{stderr}");
    }
    world.herdr.kill();
    world.hook_unanswered("claude", "w1:p1", &session_start("claude", "SA", "resume"));
    assert_eq!(world.kept_claims(), 1);
    // The same process (SA's stand-in) now runs TB.
    let process = herdr_threads::test_support::isolation::stand_in_fingerprint("claude", "SA")
        .pid
        .to_string();
    let scale = herdr_threads::protocol::time::TEST_TIMEOUT_SCALE_ENV;
    let _ = world.hook_with_env(
        "claude",
        "w1:p1",
        &session_start("claude", "TB", "resume"),
        &[
            (scale, "1"),
            ("CLAUDE_PID", &process),
            ("HT_TEST_HARNESS_PID", &process),
        ],
    );
    assert_eq!(world.kept_claims(), 1, "T's claim supersedes S's");
    world.herdr.set_panes(vec![
        claude("w1:p1", "term-a2", "TB"),
        pane("w1:p2", "term-b2"),
    ]);
    world.herdr.spawn(); // leak-guard: fixture method, spawns via spawn_owned
    world.kick_observation();
    world.wait_for_recovery("B to reattach to the pane running TB", |world| {
        world.seat_state(&b) == "resolved" && world.kept_claims() == 0
    });
    assert_eq!(world.seat_state(&a), "unresolved");
    assert_eq!(
        world.open_binding(&b),
        vec![("claude".into(), "cooperative_top_level".into(), "TB".into())]
    );
    assert_eq!(world.continuity_decisions(), 1);
}

/// ht-dtx0 acceptance 3: the claim kept by a daemon that then stops is decided by its
/// replacement.
#[test]
fn kept_claim_survives_a_daemon_restart() {
    let mut world = World::start(
        "htkr",
        vec![claude("w1:p1", "term-a", "SA"), pane("w1:p2", "term-b")],
    );
    let a = world.resolve("w1:p1");
    let (code, _, stderr) =
        world.hook("claude", "w1:p1", &session_start("claude", "SA", "startup"));
    assert_eq!(code, 0, "{stderr}");
    world.herdr.kill();
    world.hook_unanswered("claude", "w1:p1", &session_start("claude", "SA", "resume"));
    assert_eq!(world.kept_claims(), 1);
    world.restart_herdr(vec![
        claude("w1:p1", "term-a2", "SA"),
        pane("w1:p2", "term-b2"),
    ]);
    world.wait_for_recovery("the replacement to decide the kept claim", |world| {
        world.seat_state(&a) == "resolved" && world.kept_claims() == 0
    });
    assert_eq!(world.continuity_decisions(), 1);
}

/// ht-dtx0 acceptance 4: a different session's kept claim never takes the seat, a kept claim
/// of the seat's own session whose process has exited is dropped undecided, and Herdr's agent
/// field naming the seat's own session moves nothing without a live claim.
#[test]
fn kept_claim_of_another_session_takes_nothing() {
    let mut world = World::start(
        "htkn",
        vec![claude("w1:p1", "term-a", "SA"), pane("w1:p2", "term-b")],
    );
    let a = world.resolve("w1:p1");
    let (code, _, stderr) =
        world.hook("claude", "w1:p1", &session_start("claude", "SA", "startup"));
    assert_eq!(code, 0, "{stderr}");
    world.herdr.kill();
    world.hook_unanswered("claude", "w1:p1", &session_start("claude", "S2", "resume"));
    assert_eq!(world.kept_claims(), 1);
    // A kept claim of the seat's own session SA whose process has exited: it is dropped
    // undecided (a gone process cannot be the session that claimed).
    {
        let path = world.find("herdr-threads.db").expect("daemon database");
        let db = rusqlite::Connection::open(path).unwrap();
        db.busy_timeout(Duration::from_secs(5)).unwrap();
        db.execute(
            "INSERT INTO kept_continuity_claims(host_id,target_id,harness,native_session,command_json,received_at) \
             SELECT host_id,target_id,harness,'SA',json_set(command_json,'$.native_session','SA','$.operation','op-dead','$.liveness',json('{\"pid\":1,\"start\":1}')),received_at \
             FROM kept_continuity_claims",
            [],
        )
        .unwrap();
    }
    assert_eq!(world.kept_claims(), 2);
    // Herdr's agent field shows SA, the seat's own session: a hint only.
    world.herdr.set_panes(vec![
        claude("w1:p1", "term-a2", "SA"),
        pane("w1:p2", "term-b2"),
    ]);
    world.herdr.spawn(); // leak-guard: fixture method, spawns via spawn_owned
    world.kick_observation();
    world.wait_for_recovery("the kept claim to be refused", |world| {
        world.kept_claims() == 0 && world.seat_state(&a) == "unresolved"
    });
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(world.seat_state(&a), "unresolved");
    assert_eq!(world.continuity_decisions(), 0);
    // With the epoch reconciled, a session no seat knows is refused definitively at intake
    // and nothing is kept.
    world.wait_reconciled();
    world.hook_unanswered("claude", "w1:p1", &session_start("claude", "S3", "resume"));
    assert_eq!(world.kept_claims(), 0);
    assert_eq!(world.seat_state(&a), "unresolved");
}

/// ht-wpq4: captures every 300 ms and a 2 s restored-pane grace.
const RESTORED_PANE_ENV: &[(&str, &str)] = &[
    ("HT_TEST_LANE_TICK_MS", "300"),
    ("HT_TEST_RESTORED_PANE_GRACE_MS", "2000"),
];

impl World {
    fn restored_watches(&self) -> i64 {
        self.count("SELECT count(*) FROM restored_pane_watch")
    }
    fn closure_evidence(&self, seat: &str) -> Option<String> {
        self.database()
            .query_row(
                "SELECT closure_evidence FROM retirements WHERE seat_id=?1",
                [seat],
                |r| r.get(0),
            )
            .unwrap()
    }
    /// Seats A (`SA` on w1:p1) and B (`SB` on w1:p2) registered by their hooks, then a
    /// Herdr restart that restores both panes leaves both unresolved and watched.
    fn restored_pair(prefix: &str, env: &[(&str, &str)]) -> (Self, String, String) {
        let mut world = World::start_with_env(
            prefix,
            vec![
                claude("w1:p1", "term-a", "SA"),
                claude("w1:p2", "term-b", "SB"),
            ],
            env,
        );
        let a = world.resolve("w1:p1");
        let b = world.resolve("w1:p2");
        for (pane, session) in [("w1:p1", "SA"), ("w1:p2", "SB")] {
            let (code, _, stderr) =
                world.hook("claude", pane, &session_start("claude", session, "startup"));
            assert_eq!(code, 0, "{stderr}");
        }
        world.restart_herdr(vec![
            claude("w1:p1", "term-a2", "SA"),
            claude("w1:p2", "term-b2", "SB"),
        ]);
        world.wait_reconciled();
        assert_eq!(world.seat_state(&a), "unresolved");
        assert_eq!(world.seat_state(&b), "unresolved");
        world.wait_for("both restored panes to be watched", |world| {
            world.restored_watches() == 2
        });
        (world, a, b)
    }
}

/// ht-wpq4 acceptance 1, 5 and 7: a seat a new incarnation left unresolved retires once its
/// restored pane closes and the grace is observed, also across a daemon restart within the
/// incarnation; the holds lift when the last unresolved seat goes, and a resolved seat's pane
/// close still retires it without restored-pane evidence.
#[test]
fn unresolved_seat_retires_when_its_restored_pane_closes_and_the_hold_lifts() {
    let (world, a, b) = World::restored_pair("htrp", RESTORED_PANE_ENV);
    assert_eq!(world.hold_unclaimed(), 1);
    world.restart_daemon();
    world.wait_reconciled();
    assert_eq!(
        world.restored_watches(),
        2,
        "the watches survive the restart"
    );
    world.herdr.set_panes(vec![
        claude("w1:p2", "term-b2", "SB"),
        pane("w1:p3", "term-c"),
    ]);
    world.wait_for("A to retire", |world| world.seat_state(&a) == "retired");
    assert_eq!(
        world.closure_evidence(&a).as_deref(),
        Some("restored_pane_closed")
    );
    assert_eq!(
        world.seat_state(&b),
        "unresolved",
        "B's pane is still there"
    );
    assert_eq!(world.hold_unclaimed(), 1, "B still holds the restore");
    // A pane created after the restore resolves ordinarily.
    let c = world.resolve("w1:p3");
    assert_eq!(world.seat_state(&c), "resolved");
    world.herdr.set_panes(vec![pane("w1:p4", "term-d")]);
    world.wait_for("B and C to retire", |world| {
        world.seat_state(&b) == "retired" && world.seat_state(&c) == "retired"
    });
    assert_eq!(
        world.closure_evidence(&b).as_deref(),
        Some("restored_pane_closed")
    );
    assert_eq!(world.closure_evidence(&c), None, "a pane close, as before");
    world.wait_for("the hold to lift", |world| world.hold_unclaimed() == 0);
    assert_eq!(world.restored_watches(), 0);
}

/// ht-wpq4 acceptance 2: a seat whose restored pane stays is never retired, however long,
/// and its resumed session reattaches it.
#[test]
fn unresolved_seat_whose_restored_pane_stays_is_kept_and_reattaches() {
    let (world, a, _b) = World::restored_pair("htrs", RESTORED_PANE_ENV);
    std::thread::sleep(Duration::from_secs(4));
    assert_eq!(world.seat_state(&a), "unresolved");
    let (code, _, stderr) = world.hook("claude", "w1:p1", &session_start("claude", "SA", "resume"));
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(world.seat_state(&a), "resolved");
    assert_eq!(world.count("SELECT count(*) FROM retirements"), 0);
}

/// ht-wpq4 acceptance 3: the restored pane closes and the session, resumed in another pane,
/// claims its seat while the closure grace runs; the seat is never retired. The 30 s grace
/// dwarfs one hook round trip, so suite load cannot let the retirement win the race.
#[test]
fn a_seat_whose_session_claims_during_the_grace_is_never_retired() {
    let env = &[
        ("HT_TEST_LANE_TICK_MS", "300"),
        ("HT_TEST_RESTORED_PANE_GRACE_MS", "30000"),
    ];
    let (world, a, _b) = World::restored_pair("htrg", env);
    world.herdr.set_panes(vec![
        claude("w1:p2", "term-b2", "SB"),
        claude("w1:p3", "term-c", "SA"),
    ]);
    world.wait_for("the closure to be observed", |world| {
        world.count(&format!(
            "SELECT count(*) FROM restored_pane_watch WHERE seat_id='{a}' AND absent_seen_at IS NOT NULL"
        )) == 1
    });
    let (code, _, stderr) = world.hook("claude", "w1:p3", &session_start("claude", "SA", "resume"));
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(world.seat_state(&a), "resolved");
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(world.seat_state(&a), "resolved");
    assert_eq!(
        world.count(&format!(
            "SELECT count(*) FROM seats WHERE id='{a}' AND target_id='w1:p3'"
        )),
        1
    );
    assert_eq!(world.count("SELECT count(*) FROM retirements"), 0);
}
