//! A fake herdr-core (`hc`) server: JSON-RPC 2.0 lines on a Unix socket,
//! served by a thread of the test process ([`FakeHc`]) or by the `fake-hc`
//! binary (`src/bin/fake-hc.rs`), both through [`serve`].
//!
//! Responsibility
//! - Owns: serving `hello`, `server.status`, `terminal.get`,
//!   `events.subscribe` (snapshot, `seq`, `missed`, heartbeat), `doc.get` and
//!   `terminal.run` for a set of scripted terminals, and recording every call
//!   (method and params JSON) so a test can assert what a client never sent.
//!   `terminal.run` records the request, creates a live terminal from its
//!   argv, labels, `keep`, `parent` and `cwd`, and honours herdr-core's
//!   explicit-parent rule; [`RunFailure`] scripts its failures.
//! - Knobs: per-terminal state, status (including `null`), turn, pid,
//!   foreground, labels and `keep`; finishes with herdr-core's reason strings
//!   under its `Registry::finish` rule; record pruning (the `removed` event);
//!   core id rotation (a wipe) and seq reset (a restart); a socket rebind
//!   (the peer witness changes); unknown reply fields and unknown events.
//!   [`FakeHc::exec_runs`] makes `terminal.run` really start its argv (as the child a
//!   herdr-core terminal would run, with `HC_TERMINAL_ID` and `HC_TERMINAL_SOCKET`).
//! - Must not: model `terminal.read`, templates, `wait` or any send; they are
//!   not served and answer `method_not_found` (or are ignored).
//! - Role: test fixture for `host::hc`.

use serde_json::{Map, Value, json};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Write},
    ops::Deref,
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

/// The methods this fake serves, as `hello` lists them.
pub const SERVED_METHODS: &[&str] = &[
    "hello",
    "server.status",
    "terminal.get",
    "events.subscribe",
    "doc.get",
    "terminal.run",
];

/// Reasons for which a kept terminal stays `live` (herdr-core `Registry::finish`).
const PENDING_REASONS: &[&str] = &[
    "holder_lost",
    "stopped_by_restart",
    "server_stop",
    "host_shutdown",
    "signaled",
];

/// One scripted terminal record.
#[derive(Debug, Clone)]
pub struct FakeTerminal {
    pub address: String,
    pub name: Option<String>,
    /// `"live"` or `"finished"`.
    pub state: String,
    /// The 7501 status record, or `None` for JSON `null`.
    pub status: Option<Value>,
    pub turn: u64,
    pub pid: Option<u32>,
    pub foreground: Option<Value>,
    pub labels: BTreeMap<String, String>,
    pub keep: bool,
    pub reason: Option<String>,
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
    pub finished_ms: Option<u64>,
    /// The terminal's `title` (a program-set string, never a label).
    pub title: String,
    pub argv: Vec<String>,
    pub cwd: String,
    pub parent: Option<String>,
}

/// How the next `terminal.run` fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunFailure {
    /// A JSON-RPC error of this kind before any record exists.
    Error(&'static str),
    /// The record is made and then the spawn fails: `spawn_failed`, record kept.
    SpawnFailed,
    /// The record is made, then the connection closes with no reply.
    Disconnect,
    /// The record is made, then no reply is ever sent.
    Hang,
}

impl FakeTerminal {
    /// A live terminal with no status, a pid and a shell foreground.
    pub fn live(address: &str) -> Self {
        Self {
            address: address.to_owned(),
            name: None,
            state: "live".into(),
            status: None,
            turn: 0,
            pid: Some(4242),
            foreground: Some(json!({"pid": 4242, "name": "sh", "argv": ["sh"], "cwd": "/"})),
            labels: BTreeMap::new(),
            keep: false,
            reason: None,
            exit_code: None,
            signal: None,
            finished_ms: None,
            title: String::new(),
            argv: vec!["sh".into()],
            cwd: "/".into(),
            parent: None,
        }
    }

    fn to_json(&self, unknown: bool) -> Value {
        let mut t = json!({
            "address": self.address, "name": self.name, "state": self.state,
            "status": self.status, "status_ms": self.status.as_ref().map(|_| 1),
            "turn": self.turn, "pid": self.pid, "foreground": self.foreground,
            "labels": self.labels, "keep": self.keep, "reason": self.reason,
            "exit_code": self.exit_code, "signal": self.signal,
            "finished_ms": self.finished_ms, "argv": self.argv, "cwd": self.cwd,
            "created_ms": 1, "screen": "exact", "cols": 80, "rows": 24,
            "title": self.title, "parent": self.parent,
        });
        if unknown {
            t["x_unknown"] = json!({"nested": [1, 2]});
        }
        t
    }
}

/// One recorded request.
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub method: String,
    pub params: Value,
    /// The accepted connection that carried it (0 for `respond`), so a test
    /// can see which requests shared one connection.
    pub connection: u64,
}

struct Inner {
    core_id: String,
    core_name: String,
    seq: u64,
    terminals: Vec<FakeTerminal>,
    docs: BTreeMap<String, (Value, u64)>,
    calls: Vec<Call>,
    /// The `event` params of every fact change, in order.
    log: Vec<Value>,
    connections: u64,
    unknown_fields: bool,
    unknown_events: bool,
    stream_before_result: bool,
    run_failure: Option<RunFailure>,
    close_after_reply: bool,
    /// When set, `terminal.run` starts its argv for real.
    exec: Option<ExecRuns>,
    /// The programs `terminal.run` started, stopped when the fake drops.
    children: Vec<crate::test_support::spawn::OwnedChild>,
}

/// How `terminal.run` really starts a terminal's program.
struct ExecRuns {
    socket: PathBuf,
    env: Vec<(String, String)>,
}

/// The scripted world and call record behind a fake hc socket.
pub struct FakeHcState(Mutex<Inner>);

impl Default for FakeHcState {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeHcState {
    pub fn new() -> Self {
        Self(Mutex::new(Inner {
            core_id: "core-0001".into(),
            core_name: "fakehost".into(),
            seq: 0,
            terminals: Vec::new(),
            docs: BTreeMap::new(),
            calls: Vec::new(),
            log: Vec::new(),
            connections: 0,
            unknown_fields: false,
            unknown_events: false,
            stream_before_result: false,
            run_failure: None,
            close_after_reply: false,
            exec: None,
            children: Vec::new(),
        }))
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Adds a terminal (announced with a `created` event).
    pub fn add_terminal(&self, terminal: FakeTerminal) {
        let mut s = self.lock();
        let address = terminal.address.clone();
        s.terminals.push(terminal);
        s.emit("created", &address, json!({}));
    }

    /// The record of terminal `address`, if any.
    pub fn terminal(&self, address: &str) -> Option<FakeTerminal> {
        self.lock()
            .terminals
            .iter()
            .find(|t| t.address == address)
            .cloned()
    }

    /// Edits a terminal in place; the caller names the event it implies.
    fn edit(&self, address: &str, kind: &str, f: impl FnOnce(&mut FakeTerminal)) {
        let mut s = self.lock();
        let t = s
            .terminals
            .iter_mut()
            .find(|t| t.address == address)
            .unwrap_or_else(|| panic!("fake hc: no terminal {address}"));
        f(t);
        s.emit(kind, address, json!({}));
    }

    pub fn set_status(&self, address: &str, status: Option<Value>) {
        self.edit(address, "status", |t| t.status = status);
    }

    pub fn set_turn(&self, address: &str, turn: u64) {
        self.edit(address, "turn", |t| t.turn = turn);
    }

    /// The core restarts a kept terminal waiting in `live` with no holder:
    /// it has a pid and a shell foreground again and no finish reason
    /// (`restarted` event). The same `tN`: ids are never reused.
    pub fn restart(&self, address: &str) {
        self.edit(address, "restarted", |t| {
            assert!(t.state == "live", "fake hc: {} is finished", t.address);
            t.pid = Some(4243);
            t.foreground = Some(json!({"pid": 4243, "name": "sh", "argv": ["sh"], "cwd": "/"}));
            t.reason = None;
            t.exit_code = None;
            t.signal = None;
        });
    }

    pub fn set_pid(&self, address: &str, pid: Option<u32>) {
        self.edit(address, "restarted", |t| t.pid = pid);
    }

    pub fn set_foreground(&self, address: &str, foreground: Option<Value>) {
        self.edit(address, "restarted", |t| t.foreground = foreground);
    }

    pub fn set_labels(&self, address: &str, labels: &[(&str, &str)]) {
        self.edit(address, "labels", |t| {
            t.labels = labels
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect();
        });
    }

    pub fn set_keep(&self, address: &str, keep: bool) {
        self.edit(address, "keep", |t| t.keep = keep);
    }

    /// Ends the terminal's program with herdr-core's `Registry::finish` rule:
    /// a kept terminal finishing for a pending reason (`holder_lost`,
    /// `stopped_by_restart`, `server_stop`, `host_shutdown`, `signaled`) stays
    /// `live` with no pid or foreground (`lost` event); any other finish
    /// (including an unknown reason string) turns it `finished` (`finished`).
    /// A terminal already `finished` is untouched.
    pub fn finish(&self, address: &str, reason: &str, exit_code: Option<i32>, signal: Option<i32>) {
        let mut s = self.lock();
        let t = s
            .terminals
            .iter_mut()
            .find(|t| t.address == address)
            .unwrap_or_else(|| panic!("fake hc: no terminal {address}"));
        if t.state != "live" {
            return;
        }
        let pending = t.keep && PENDING_REASONS.contains(&reason);
        t.state = if pending { "live" } else { "finished" }.into();
        if pending {
            t.pid = None;
        }
        t.exit_code = exit_code;
        t.signal = signal;
        t.reason = Some(reason.to_owned());
        t.finished_ms = (!pending).then_some(2);
        t.foreground = None;
        t.status = None;
        let kind = if pending { "lost" } else { "finished" };
        s.emit(kind, address, json!({"reason": reason}));
    }

    /// Forgets a record (pruning or `rm`) and emits `removed`.
    pub fn prune(&self, address: &str) {
        let mut s = self.lock();
        s.terminals.retain(|t| t.address != address);
        s.emit("removed", address, json!({}));
    }

    pub fn set_doc(&self, name: &str, value: Value, version: u64) {
        self.lock().docs.insert(name.to_owned(), (value, version));
    }

    /// Renames the core (its session), as a core started under another `HC_SESSION`.
    pub fn set_core_name(&self, name: &str) {
        self.lock().core_name = name.to_owned();
    }

    /// A wipe: a new permanent core id, no terminals, the sequence back at 0.
    pub fn rotate_core_id(&self, id: &str) {
        let mut s = self.lock();
        s.core_id = id.to_owned();
        s.terminals.clear();
        s.log.clear();
        s.seq = 0;
    }

    /// A restart: the event sequence begins again at 0.
    pub fn reset_seq(&self) {
        self.lock().seq = 0;
    }

    /// Every reply object carries fields this client does not know.
    pub fn set_unknown_fields(&self, on: bool) {
        self.lock().unknown_fields = on;
    }

    /// `events.subscribe` replies are preceded by a heartbeat, a known event,
    /// a `missed` snapshot, an event of an unknown type and a notification of
    /// an unknown method (`set_unknown_events` governs the last two).
    pub fn set_stream_before_result(&self, on: bool) {
        self.lock().stream_before_result = on;
    }

    pub fn set_unknown_events(&self, on: bool) {
        self.lock().unknown_events = on;
    }

    /// The next `terminal.run` fails this way (once).
    pub fn fail_next_run(&self, failure: RunFailure) {
        self.lock().run_failure = Some(failure);
    }

    /// Whether the last request asked the connection to close (read once).
    fn take_close(&self) -> bool {
        std::mem::take(&mut self.lock().close_after_reply)
    }

    /// The `event` params of every fact change so far (`seq`, `type`,
    /// `terminal`, `data`), e.g. to see that a prune emitted `removed`.
    pub fn events(&self) -> Vec<Value> {
        self.lock().log.clone()
    }

    /// Every request received, in order.
    pub fn calls(&self) -> Vec<Call> {
        self.lock().calls.clone()
    }

    pub fn calls_to(&self, method: &str) -> Vec<Call> {
        self.calls()
            .into_iter()
            .filter(|c| c.method == method)
            .collect()
    }

    fn next_connection(&self) -> u64 {
        let mut s = self.lock();
        s.connections += 1;
        s.connections
    }

    /// The reply lines for one request line (the response, plus notification
    /// lines around an `events.subscribe` result).
    pub fn respond(&self, line: &str) -> Vec<String> {
        self.respond_on(0, line)
    }

    /// [`Self::respond`] for a request that arrived on connection `connection`.
    fn respond_on(&self, connection: u64, line: &str) -> Vec<String> {
        let mut s = self.lock();
        let request: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                return vec![error_line(
                    &Value::Null,
                    -32700,
                    "parse_error",
                    &e.to_string(),
                )];
            }
        };
        let id = request["id"].clone();
        let method = request["method"].as_str().unwrap_or_default().to_owned();
        let params = request.get("params").cloned().unwrap_or(Value::Null);
        s.calls.push(Call {
            method: method.clone(),
            params: params.clone(),
            connection,
        });
        match method.as_str() {
            "hello" => vec![s.result_line(&id, s.hello())],
            "server.status" => vec![s.result_line(&id, s.status())],
            "terminal.get" => {
                let wanted = params["terminal"].as_str().unwrap_or_default();
                match s.find(wanted) {
                    Some(t) => vec![s.result_line(&id, t.to_json(s.unknown_fields))],
                    None => vec![error_line(&id, -32000, "not_found", wanted)],
                }
            }
            "doc.get" => {
                let name = params["name"].as_str().unwrap_or_default();
                let (value, version) = s.docs.get(name).cloned().unwrap_or((Value::Null, 0));
                vec![s.result_line(&id, json!({"value": value, "version": version}))]
            }
            "events.subscribe" => s.subscribe(&id, &params),
            "terminal.run" => s.run(&id, &params),
            other => vec![error_line(&id, -32601, "method_not_found", other)],
        }
    }
}

impl Inner {
    fn emit(&mut self, kind: &str, address: &str, data: Value) {
        self.seq += 1;
        self.log
            .push(json!({"seq": self.seq, "type": kind, "terminal": address, "data": data}));
    }

    fn find(&self, wanted: &str) -> Option<&FakeTerminal> {
        self.terminals
            .iter()
            .find(|t| t.address == wanted || t.name.as_deref() == Some(wanted))
    }

    fn core(&self) -> Value {
        json!({"id": self.core_id, "name": self.core_name})
    }

    fn hello(&self) -> Value {
        json!({"generation": 1, "version": "0.1.15", "core": self.core(), "framed": false,
               "methods": SERVED_METHODS, "features": ["events"]})
    }

    fn status(&self) -> Value {
        let live = self.terminals.iter().filter(|t| t.state == "live").count();
        json!({"core": self.core(), "version": "0.1.15", "pid": std::process::id(),
               "uptime_ms": 1, "holders": [], "holder": null, "live": live,
               "finished": self.terminals.len() - live})
    }

    fn result_line(&self, id: &Value, mut result: Value) -> String {
        if self.unknown_fields
            && let Some(map) = result.as_object_mut()
        {
            map.insert("x_unknown".into(), json!("ignored"));
        }
        json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string()
    }

    /// `terminal.run`: herdr-core's `start` for the params this system sends.
    fn run(&mut self, id: &Value, params: &Value) -> Vec<String> {
        let failure = self.run_failure.take();
        if let Some(RunFailure::Error(kind)) = failure {
            return vec![error_line(id, -32000, kind, "scripted run failure")];
        }
        let text = |key: &str| params[key].as_str().map(str::to_owned);
        let argv: Vec<String> = params["argv"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        let parent = match text("parent") {
            Some(p) if self.find(&p).is_none() => {
                return vec![error_line(id, -32000, "not_found", &p)];
            }
            explicit => explicit,
        };
        let next = self
            .terminals
            .iter()
            .filter_map(|t| t.address.strip_prefix('t')?.parse::<u64>().ok())
            .max()
            .unwrap_or(0)
            + 1;
        let address = format!("t{next}");
        let mut t = FakeTerminal::live(&address);
        t.pid = Some(5000 + next as u32);
        if let Some(exec) = &self.exec {
            use crate::test_support::spawn::SpawnOwned;
            let Some((program, args)) = argv.split_first() else {
                return vec![error_line(id, -32000, "spawn_failed", "empty argv")];
            };
            let mut command = crate::test_support::spawn::command(program);
            command
                .args(args)
                .env("HC_TERMINAL_ID", &address)
                .env("HC_TERMINAL_SOCKET", &exec.socket)
                .envs(exec.env.iter().map(|(k, v)| (k, v)))
                .current_dir(text("cwd").unwrap_or_else(|| "/".into()))
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            match command.spawn_owned() {
                Ok(child) => {
                    t.pid = Some(child.id());
                    self.children.push(child);
                }
                Err(e) => {
                    return vec![error_line(id, -32000, "spawn_failed", &e.to_string())];
                }
            }
        }
        t.argv = argv;
        t.cwd = text("cwd").unwrap_or_else(|| "/".into());
        t.keep = params["keep"].as_bool().unwrap_or(false);
        t.parent = parent;
        t.labels = params["labels"]
            .as_object()
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
                    .collect()
            })
            .unwrap_or_default();
        let notify = !params["no_notify"].as_bool().unwrap_or(false);
        let mut reply = t.to_json(self.unknown_fields);
        reply["notify"] = json!(notify);
        reply["first_message"] = Value::Null;
        match failure {
            Some(RunFailure::SpawnFailed) => {
                self.terminals.push(t);
                self.emit("created", &address, json!({}));
                vec![error_line(
                    id,
                    -32000,
                    "spawn_failed",
                    "scripted spawn failure",
                )]
            }
            Some(RunFailure::Disconnect | RunFailure::Hang) => {
                self.terminals.push(t);
                self.emit("created", &address, json!({}));
                self.close_after_reply = failure == Some(RunFailure::Disconnect);
                Vec::new()
            }
            _ => {
                self.terminals.push(t);
                self.emit("created", &address, json!({}));
                vec![self.result_line(id, reply)]
            }
        }
    }

    fn subscribe(&self, id: &Value, params: &Value) -> Vec<String> {
        let only = params["terminal"].as_str();
        let want: Map<String, Value> = params["labels"].as_object().cloned().unwrap_or_default();
        let terminals: Vec<Value> = self
            .terminals
            .iter()
            .filter(|t| only.is_none_or(|a| t.address == a))
            .filter(|t| {
                want.iter()
                    .all(|(k, v)| t.labels.get(k) == v.as_str().map(str::to_owned).as_ref())
            })
            .map(|t| t.to_json(self.unknown_fields))
            .collect();
        let snapshot = json!({"seq": self.seq, "terminals": terminals});
        let mut lines = Vec::new();
        if self.stream_before_result {
            lines.push(notification("heartbeat", json!({"seq": self.seq})));
            lines.push(notification(
                "event",
                json!({"seq": self.seq + 1, "type": "turn", "terminal": "t1", "data": {}}),
            ));
            lines.push(notification("missed", snapshot.clone()));
            if self.unknown_events {
                lines.push(notification(
                    "event",
                    json!({"seq": self.seq + 2, "type": "x_future_event", "terminal": "t1",
                           "data": {"new": true}}),
                ));
                lines.push(notification(
                    "x_future_method",
                    json!({"seq": self.seq + 3}),
                ));
            }
        }
        lines.push(self.result_line(id, snapshot));
        lines
    }
}

fn notification(method: &str, params: Value) -> String {
    json!({"jsonrpc": "2.0", "method": method, "params": params}).to_string()
}

fn error_line(id: &Value, code: i64, kind: &str, message: &str) -> String {
    json!({"jsonrpc": "2.0", "id": id,
           "error": {"code": code, "message": message, "data": {"kind": kind}}})
    .to_string()
}

fn handle(stream: UnixStream, connection: u64, state: &FakeHcState) {
    let Ok(mut out) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        for reply in state.respond_on(connection, line.trim_end()) {
            if out.write_all(reply.as_bytes()).is_err() || out.write_all(b"\n").is_err() {
                return;
            }
        }
        let _ = out.flush();
        if state.take_close() {
            return;
        }
    }
}

/// Accepts connections on `listener` until `stop` is set, one thread per
/// connection. The `fake-hc` binary and [`FakeHc`] both run this.
pub fn serve(listener: UnixListener, state: Arc<FakeHcState>, stop: Arc<AtomicBool>) {
    listener
        .set_nonblocking(true)
        .expect("fake hc: nonblocking listener");
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => {
                let _ = stream.set_nonblocking(false);
                let state = state.clone();
                let connection = state.next_connection();
                std::thread::spawn(move || handle(stream, connection, &state));
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(5)),
        }
    }
}

/// An in-process fake hc on a scratch socket; stops and removes the socket on Drop.
pub struct FakeHc {
    socket: PathBuf,
    state: Arc<FakeHcState>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Deref for FakeHc {
    type Target = FakeHcState;
    fn deref(&self) -> &FakeHcState {
        &self.state
    }
}

impl Default for FakeHc {
    fn default() -> Self {
        Self::start()
    }
}

impl FakeHc {
    pub fn start() -> Self {
        let socket = crate::test_support::scratch::temp_path(format!(
            "ht-fake-hc-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let state = Arc::new(FakeHcState::new());
        let (stop, worker) = Self::listen(&socket, &state);
        Self {
            socket,
            state,
            stop,
            worker: Some(worker),
        }
    }

    fn listen(socket: &Path, state: &Arc<FakeHcState>) -> (Arc<AtomicBool>, JoinHandle<()>) {
        let listener = UnixListener::bind(socket).expect("fake hc: bind");
        let stop = Arc::new(AtomicBool::new(false));
        let worker = {
            let (state, stop) = (state.clone(), stop.clone());
            std::thread::spawn(move || serve(listener, state, stop))
        };
        (stop, worker)
    }

    fn halt(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        let _ = std::fs::remove_file(&self.socket);
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// From now on `terminal.run` really starts its argv (through `test_support::spawn`, in
    /// its own group, stopped with this fake), with `HC_TERMINAL_ID`, `HC_TERMINAL_SOCKET`
    /// and `env`; the terminal's `pid` is the child's. The mutex is held across the spawn,
    /// so the shim's own `hello` waits for the reply.
    pub fn exec_runs(&self, env: &[(&str, &str)]) {
        self.state.lock().exec = Some(ExecRuns {
            socket: self.socket.clone(),
            env: env
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        });
    }

    pub fn state(&self) -> Arc<FakeHcState> {
        self.state.clone()
    }

    /// Serves the same state on a fresh socket at the same path: the peer
    /// witness's socket identity changes, as when a core restarts.
    pub fn rebind(&mut self) {
        self.halt();
        let (stop, worker) = Self::listen(&self.socket.clone(), &self.state);
        self.stop = stop;
        self.worker = Some(worker);
    }
}

impl Drop for FakeHc {
    fn drop(&mut self) {
        self.halt();
    }
}
