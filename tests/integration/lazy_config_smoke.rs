//! Actual executable/current-daemon configuration matrix. Legacy means inbox-v1
//! capabilities on wire protocol 6, never a historical executable or protocol-1 daemon.
use herdr_threads::{
    daemon::{
        ownership::{read_descriptor, read_existing_namespace},
        paths::{InstancePaths, RuntimeContext},
    },
    test_support::spawn,
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::{
        fs::DirBuilderExt,
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    process::Output,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};
const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");
pub(crate) struct Scratch(pub(crate) PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(crate) fn pane(id: &str, terminal: &str) -> Value {
    // A plain shell pane: no recognized harness agent, so a safe wake is
    // always refused before submission.
    json!({"pane_id":id,"terminal_id":terminal,"workspace_id":"w1","tab_id":"w1:t1",
           "focused":false,"agent_status":"idle","revision":3})
}

/// The stand-in Herdr's answer to one request against the scripted `panes`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn host_reply(panes: &[Value], request: &Value) -> Value {
    let id = request["id"].clone();
    match request["method"].as_str().unwrap_or_default() {
        "ping" => {
            json!({"id":id,"result":{"type":"pong","version":"0.9.1","protocol":22}})
        }
        "session.snapshot" => json!({"id":id,"result":{"type":"session_snapshot",
                "snapshot":{"version":"0.9.1","protocol":22,"panes":panes,
                            "agents":[],"tabs":[{"tab_id":"w1:t1","workspace_id":"w1"}],"workspaces":[{"workspace_id":"w1"}],"layouts":[]}}}),
        "pane.current" => match panes
            .iter()
            .find(|pane| pane["pane_id"] == request["params"]["caller_pane_id"])
        {
            Some(pane) => json!({"id":id,"result":{"type":"pane_current","pane":pane}}),
            None => json!({"id":id,"error":{"code":"pane_not_found","message":"pane not found"}}),
        },
        "pane.get" => match panes
            .iter()
            .find(|pane| pane["pane_id"] == request["params"]["pane_id"])
        {
            Some(pane) => json!({"id":id,"result":{"type":"pane_info","pane":pane}}),
            None => {
                json!({"id":id,"error":{"code":"pane_not_found","message":"pane not found"}})
            }
        },
        "agent.get" => {
            let pane = panes
                .iter()
                .find(|pane| pane["pane_id"] == request["params"]["target"]);
            match pane {
                Some(pane) if pane.get("agent_get_error").is_some() => {
                    json!({"id":id,"error":{"code":pane["agent_get_error"],
                            "message":"scripted read error"}})
                }
                Some(pane) if pane["agent"].is_string() => {
                    let mut agent = json!({"agent":pane["agent"],
                            "agent_status":pane["agent_status"],
                            "pane_id":pane["pane_id"],
                            "terminal_id":pane["terminal_id"]});
                    if let Some(session) = pane.get("agent_session") {
                        agent["agent_session"] = session.clone();
                    }
                    json!({"id":id,"result":{"type":"agent_info","agent":agent}})
                }
                _ => {
                    json!({"id":id,"error":{"code":"agent_not_found","message":"no agent in pane"}})
                }
            }
        }
        _ => {
            json!({"id":id,"error":{"code":"agent_not_found","message":"no agent in pane"}})
        }
    }
}

/// The private Herdr endpoint. One request per accepted connection. Accept
/// blocks (no polling latency on every host call); Drop wakes it with a
/// connection of its own after raising `stop`.
pub(crate) struct FakeHost {
    stop: Arc<AtomicBool>,
    socket: PathBuf,
    worker: Option<JoinHandle<()>>,
}
impl FakeHost {
    pub(crate) fn start(socket: &Path, panes: Vec<Value>) -> Self {
        let listener = UnixListener::bind(socket).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let worker = std::thread::spawn(move || {
            loop {
                let accepted = listener.accept();
                if stopped.load(Ordering::SeqCst) {
                    return;
                }
                let mut stream = match accepted {
                    Ok((stream, _)) => stream,
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) => panic!("fake host accept: {error}"),
                };
                // A client that already hung up makes this fail (EINVAL on
                // macOS); drop that connection instead of killing the fake host.
                if stream
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
                let reply = host_reply(&panes, &request);
                let _ = writeln!(stream, "{reply}");
            }
        });
        Self {
            stop,
            socket: socket.to_owned(),
            worker: Some(worker),
        }
    }
}
impl Drop for FakeHost {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the blocked accept. If that is impossible, leave the parked
        // worker to process exit rather than hang the test in join.
        if UnixStream::connect(&self.socket).is_ok()
            && let Some(worker) = self.worker.take()
        {
            let _ = worker.join();
        }
    }
}

pub(super) struct World {
    pub(super) root: PathBuf,
    state: PathBuf,
    host: PathBuf,
    pub(super) seats: Vec<String>,
    pub(super) thread: String,
    _host: FakeHost,
    _scratch: Scratch,
}
impl World {
    pub(super) fn new() -> Self {
        Self::with_routing_names("state", "h.sock")
    }
    pub(super) fn with_routing_names(state_name: &str, host_name: &str) -> Self {
        Self::build(state_name, host_name, true)
    }
    /// The same world without the Human seat 2: for cases whose callers and
    /// assertions involve only the agent seats 0 and 1 (`seats` has two entries).
    pub(super) fn agents_only() -> Self {
        Self::agents_only_with_routing_names("state", "h.sock")
    }
    pub(super) fn agents_only_with_routing_names(state_name: &str, host_name: &str) -> Self {
        Self::build(state_name, host_name, false)
    }
    fn build(state_name: &str, host_name: &str, with_human: bool) -> Self {
        let root = PathBuf::from(format!(
            "/private/tmp/htlc-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..10]
        ));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let scratch = Scratch(root.clone());
        let host = root.join(host_name);
        let fake = FakeHost::start(
            &host,
            (1..=4)
                .map(|n| pane(&format!("w1:p{n}"), &format!("term-{n}")))
                .collect(),
        );
        let mut w = Self {
            root: root.clone(),
            state: root.join(state_name),
            host,
            seats: vec![],
            thread: String::new(),
            _host: fake,
            _scratch: scratch,
        };
        w.ok(None, false, &["daemon", "ensure"]);
        // Seats 0 (sender), 1 (agent recipient) and, unless agents-only,
        // 2 (Human). Only the post-join case needs a fourth seat; it adds it
        // with `add_fourth_seat`.
        let members: &[usize] = if with_human { &[1, 2] } else { &[1] };
        for n in 0..=members.len() {
            w.resolve_seat(n);
        }
        for n in [0, 1] {
            w.ok(Some(n), false, &["check-in", "--lifecycle-event", "start"]);
        }
        if with_human {
            // Real immediate argv[1] Human namespace, lawful shell identity.
            w.ok(Some(2), true, &["me", "init"]);
        }
        w.thread = w.ok(
            Some(0),
            false,
            &["thread", "create", "--topic", "lazy configuration"],
        )["data"]
            .as_str()
            .unwrap()
            .into();
        for &n in members {
            w.ok(
                Some(0),
                false,
                &["invite", &w.thread, "--seat", &w.seats[n]],
            );
            w.ok(Some(n), n == 2, &["accept", &w.thread]);
        }
        w
    }
    fn resolve_seat(&mut self, n: usize) {
        let p = format!("w1:p{}", n + 1);
        let v = self.ok(None, false, &["seat", "resolve", "--pane", &p]);
        assert_eq!(self.seats.len(), n);
        self.seats.push(v["data"].as_str().unwrap().into());
    }
    /// A checked-in agent seat 3 on pane w1:p4 that has not joined the thread.
    pub(super) fn add_fourth_seat(&mut self) {
        self.resolve_seat(3);
        self.ok(Some(3), false, &["check-in", "--lifecycle-event", "start"]);
    }
    fn command(&self, who: Option<usize>, human: bool, args: &[&str]) -> std::process::Command {
        let mut c = spawn::command(BIN);
        if human {
            c.arg("human");
        }
        c.args(["--state-dir"])
            .arg(&self.state)
            .arg("--host-endpoint")
            .arg(&self.host)
            .env("HOME", self.root.join("home"))
            .env("CLAUDE_CONFIG_DIR", self.root.join("claude"))
            .env("CODEX_HOME", self.root.join("codex"))
            .env("HERDR_THREADS_OFFLINE", "1")
            .env("NO_COLOR", "1");
        if let Some(n) = who {
            let target = format!("w1:p{}", n + 1);
            if human {
                c.env("HERDR_PANE_ID", target);
            } else {
                c.args([
                    "--cooperative-seat",
                    &self.seats[n],
                    "--cooperative-target",
                    &target,
                    "--cooperative-harness",
                    "codex",
                    "--cooperative-role",
                    "top-level",
                ]);
            }
        }
        c.args(args);
        c
    }
    pub(super) fn raw(&self, who: Option<usize>, human: bool, args: &[&str]) -> Output {
        let mut command = self.command(who, human, args);
        let output = command.output().unwrap();
        // Optional task evidence retains complete successful output as well as
        // failure diagnostics; each isolated World writes a separate transcript.
        if let Some(dir) = std::env::var_os("HT_LAZY_SMOKE_CAPTURE") {
            let path = PathBuf::from(dir).join(format!(
                "{}.jsonl",
                self.root.file_name().unwrap().to_string_lossy()
            ));
            let mut file = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .unwrap();
            let argv: Vec<_> = command
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            writeln!(file,"{}",json!({"argv":argv,"status":output.status.code(),"stdout":String::from_utf8_lossy(&output.stdout),"stderr":String::from_utf8_lossy(&output.stderr)})).unwrap();
        }
        output
    }
    pub(super) fn text(&self, who: usize, human: bool, args: &[&str]) -> String {
        let o = self.raw(Some(who), human, args);
        assert!(
            o.status.success(),
            "argv={args:?} human={human} stdout={} stderr={}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8(o.stdout).unwrap()
    }
    pub(super) fn ok(&self, who: Option<usize>, human: bool, args: &[&str]) -> Value {
        let mut a = vec!["--json"];
        a.extend_from_slice(args);
        let o = self.raw(who, human, &a);
        assert!(
            o.status.success(),
            "argv={a:?} human={human} stdout={} stderr={}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        );
        serde_json::from_slice::<Value>(&o.stdout).unwrap()["result"].clone()
    }
    pub(super) fn paths(&self) -> InstancePaths {
        InstancePaths::resolve(
            &RuntimeContext::explicit(self.state.clone(), self.host.clone(), None).unwrap(),
        )
        .unwrap()
    }
    pub(super) fn db(&self) -> rusqlite::Connection {
        rusqlite::Connection::open_with_flags(
            self.paths().database_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap()
    }
    pub(super) fn state(&self, message: &str, seat: usize) -> String {
        self.db()
            .query_row(
                "SELECT state FROM lazy_recipients WHERE message_id=?1 AND seat_id=?2",
                [message, &self.seats[seat]],
                |r| r.get(0),
            )
            .unwrap()
    }
    pub(super) fn send(&self, body: &str, extra: &[&str]) -> String {
        let mut a = vec!["send", &self.thread, "--body", body];
        a.extend_from_slice(extra);
        self.ok(Some(0), false, &a)["data"].as_str().unwrap().into()
    }
    pub(super) fn projection_snapshot(&self) -> Vec<(String, Vec<Vec<String>>)> {
        let db = self.db();
        [
            "messages",
            "lazy_recipients",
            "receipts",
            "receipt_state",
            "work_jobs",
            "warning_offer",
        ]
        .into_iter()
        .map(|table| {
            let suffix = if table == "work_jobs" {
                " WHERE kind='send_attention'"
            } else {
                ""
            };
            let mut stmt = db
                .prepare(&format!("SELECT * FROM {table}{suffix}"))
                .unwrap();
            let count = stmt.column_count();
            let mut rows: Vec<Vec<String>> = stmt
                .query_map([], |r| {
                    Ok((0..count)
                        .map(|n| format!("{:?}", r.get_ref(n).unwrap()))
                        .collect())
                })
                .unwrap()
                .map(Result::unwrap)
                .collect();
            rows.sort();
            (table.into(), rows)
        })
        .collect()
    }
    pub(super) fn intents(&self) -> Vec<(PathBuf, Vec<u8>)> {
        fn walk(p: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
            if let Ok(es) = fs::read_dir(p) {
                for e in es.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        walk(&p, out)
                    } else if p.components().any(|s| s.as_os_str() == "intents") {
                        out.push((p.clone(), fs::read(p).unwrap()));
                    }
                }
            }
        }
        let mut out = vec![];
        walk(&self.state, &mut out);
        out.sort();
        out
    }
}
impl Drop for World {
    fn drop(&mut self) {
        let _ = self.raw(None, false, &["daemon", "stop"]);
    }
}

#[path = "../support/lazy_proxy.rs"]
mod lazy_proxy;
pub(super) use lazy_proxy::Proxy;
use lazy_proxy::{frame, write_frame};

#[test]
fn lazy_config_default_text_settles() {
    let w = World::agents_only();
    let lazy = w.send("complete passive body", &[]);
    let ordinary = w.send("ordinary independent body", &["--require-ack", &w.seats[1]]);
    let out = w.text(1, false, &["inbox"]);
    assert!(
        out.contains("complete passive body") && out.contains(&lazy) && out.contains("[lazy]"),
        "{out}"
    );
    assert!(out.contains("ordinary independent body"), "{out}");
    assert_eq!(w.state(&lazy, 1), "displayed");
    let ack: i64 = w
        .db()
        .query_row(
            "SELECT count(*) FROM (SELECT message_id,seat_id FROM receipts WHERE message_id=?1 AND seat_id=?2 AND state='acked' UNION SELECT message_id,seat_id FROM receipt_state WHERE message_id=?1 AND seat_id=?2 AND state='acked')",
            [&ordinary, &w.seats[1]],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(ack, 1);
    assert_eq!(
        w.db()
            .query_row(
                "SELECT count(*) FROM receipts WHERE message_id=?1",
                [&lazy],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}
#[test]
fn lazy_config_json_machine_explicit_seat_are_readonly() {
    let w = World::agents_only();
    let m = w.send("body preserved in each supported mode", &[]);
    let before = w.intents();
    let projections = w.projection_snapshot();
    for args in [
        vec!["inbox", "--json"],
        vec!["inbox", "--machine"],
        vec!["inbox", "--seat", &w.seats[1]],
    ] {
        let out = w.text(1, false, &args);
        assert!(
            out.contains("body preserved in each supported mode"),
            "argv={args:?}: {out}"
        );
        assert_eq!(w.state(&m, 1), "pending", "argv={args:?}");
        assert_eq!(w.intents(), before, "argv={args:?}");
        assert_eq!(
            w.projection_snapshot(),
            projections,
            "readonly database changed argv={args:?}"
        );
    }
    w.text(1, false, &["inbox"]);
    assert_eq!(w.state(&m, 1), "displayed");
}
#[test]
fn lazy_config_current_cli_inbox_v1_daemon_refuses_lazy_before_intent() {
    let w = World::agents_only();
    let ordinary = w.send("ordinary legacy fallback", &["--require-ack", &w.seats[1]]);
    let p = Proxy::new(w.paths(), &w.root);
    p.mode.store(1, Ordering::SeqCst);
    let before = w.intents();
    let count: i64 = w
        .db()
        .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))
        .unwrap();
    for extra in [vec![], vec!["--lazy"]] {
        let mut a = vec!["send", &w.thread, "--body", "must never be sent"];
        a.extend(extra);
        let o = w.raw(Some(0), false, &a);
        assert!(!o.status.success(), "argv={a:?}");
        assert!(String::from_utf8_lossy(&o.stderr).contains("send.lazy_v1"));
        assert_eq!(w.intents(), before);
        assert_eq!(
            w.db()
                .query_row("SELECT count(*) FROM messages", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            count
        );
    }
    let out = w.text(1, false, &["inbox", "--machine"]);
    assert!(
        out.contains(&w.thread) && out.contains("receipts=1"),
        "{out}"
    );
    let body = w.text(1, false, &["body", &ordinary]);
    assert!(body.contains("ordinary legacy fallback"), "{body}");
    assert_eq!(
        w.db()
            .query_row(
                "SELECT state FROM receipt_state WHERE message_id=?1 AND seat_id=?2 UNION SELECT state FROM receipts WHERE message_id=?1 AND seat_id=?2",
                [&ordinary, &w.seats[1]],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "pending"
    );
    let kinds: Vec<_> = p
        .requests()
        .iter()
        .map(|q| q["command"]["kind"].as_str().unwrap().to_owned())
        .collect();
    assert!(kinds.contains(&"inbox".into()), "{kinds:?}");
    assert!(
        !kinds.iter().any(|k| matches!(
            k.as_str(),
            "send_message" | "inbox_batch_v2" | "complete_inbox_delivery" | "ack_displayed"
        )),
        "{kinds:?}"
    );
    let compatible = w.send("explicit nudge remains compatible", &["--nudge"]);
    assert_eq!(
        w.db()
            .query_row(
                "SELECT delivery_mode FROM messages WHERE id=?1",
                [&compatible],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "ordinary"
    );
    let wire = p
        .requests()
        .into_iter()
        .find(|q| q["command"]["kind"] == "send_message")
        .expect("ordinary send submitted");
    assert!(
        wire["command"]["args"].get("delivery_mode").is_none(),
        "legacy ordinary wire must omit mode: {wire}"
    );
}
#[test]
fn lazy_config_inbox_v1_client_current_daemon_ordinary_shapes() {
    let w = World::agents_only();
    let lazy = w.send("legacy history gets full passive content", &[]);
    let ordinary = w.send("legacy ordinary item", &["--require-ack", &w.seats[1]]);
    let p = w.paths();
    let instance = read_existing_namespace(&p).unwrap().unwrap();
    let d = read_descriptor(&p, instance).unwrap();
    // Frozen pre-lazy envelope/command: no delivery_mode, v2 item or new selectors.
    let q = json!({"version":6,"request_id":"legacy-inbox-smoke","expected_instance":instance.to_string(),"expected_boot":d.boot_id.to_string(),"command":{"kind":"inbox_batch","args":{"seat":w.seats[1],"page":{"cursor":null,"limit":20,"max_bytes":16384}}}});
    let mut s = UnixStream::connect(&p.socket_path).unwrap();
    write_frame(&mut s, &serde_json::to_vec(&q).unwrap());
    let v: Value = serde_json::from_slice(&frame(&mut s).unwrap()).unwrap();
    let items = v["result"]["Ok"]["data"]["items"]
        .as_array()
        .expect("legacy items");
    assert!(items.iter().any(|i| i["message"] == ordinary), "{v}");
    assert!(!items.iter().any(|i| i["message"] == lazy), "{v}");
    let mut keys: Vec<_> = v["result"]["Ok"]["data"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "consistency",
            "has_more",
            "high_water_ordinal",
            "items",
            "next_argv",
            "next_cursor",
            "scope_revision",
            "stop_reason"
        ]
    );
    for i in items {
        assert!(
            i.as_object().unwrap().keys().all(|k| [
                "kind",
                "thread",
                "topic_data",
                "message",
                "sequence",
                "sender",
                "author_role",
                "relays_user",
                "user_intent",
                "author_role_backfilled",
                "body",
                "body_start",
                "body_end",
                "body_len",
                "ack_candidate"
            ]
            .contains(&k.as_str())),
            "new legacy item field {i}"
        );
        assert_ne!(i["kind"], "lazy_message");
        assert!(i.get("delivery_mode").is_none());
    }
    // Frozen legacy body command, so current CLI capability choices cannot mask
    // a backwards-incompatible body result.
    let q = json!({"version":6,"request_id":"legacy-body-smoke","expected_instance":instance.to_string(),"expected_boot":d.boot_id.to_string(),"command":{"kind":"message","args":{"message":lazy,"body":{"cursor":null,"offset":null,"max_bytes":16384}}}});
    let mut socket = UnixStream::connect(&p.socket_path).unwrap();
    write_frame(&mut socket, &serde_json::to_vec(&q).unwrap());
    let reply: Value = serde_json::from_slice(&frame(&mut socket).unwrap()).unwrap();
    let body = &reply["result"]["Ok"];
    assert_eq!(body["kind"], "message");
    assert_eq!(body["data"]["content"]["kind"], "ordinary");
    assert_eq!(
        body["data"]["content"]["body_data"],
        "legacy history gets full passive content"
    );
    assert_eq!(body["data"]["content"]["body_complete"], true);
    assert!(body["data"]["summary"].get("delivery_mode").is_none());
    assert_eq!(w.state(&lazy, 1), "pending");
}
#[test]
fn lazy_config_postjoin_human_unavailable_restart() {
    let mut w = World::new();
    // The later joiner is a checked-in seat before the message is sent.
    w.add_fourth_seat();
    let writer = rusqlite::Connection::open(w.paths().database_path).unwrap();
    writer
        .execute(
            "UPDATE occupant_bindings SET registered_at=NULL WHERE seat_id=?1 AND ended_at IS NULL",
            [&w.seats[1]],
        )
        .unwrap();
    drop(writer);
    let m = w.send("human and temporarily unavailable", &[]);
    assert_eq!(w.state(&m, 1), "pending");
    assert_eq!(w.state(&m, 2), "pending");
    w.ok(
        Some(0),
        false,
        &["invite", &w.thread, "--seat", &w.seats[3]],
    );
    w.ok(Some(3), false, &["accept", &w.thread]);
    let n: i64 = w
        .db()
        .query_row(
            "SELECT count(*) FROM lazy_recipients WHERE message_id=?1 AND seat_id=?2",
            [&m, &w.seats[3]],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 0);
    let out = w.text(2, true, &["inbox"]);
    assert!(out.contains("human and temporarily unavailable"), "{out}");
    assert_eq!(w.state(&m, 2), "displayed");
    w.ok(None, false, &["daemon", "stop"]);
    w.ok(None, false, &["daemon", "ensure"]);
    assert_eq!(w.state(&m, 1), "pending");
    w.ok(
        Some(1),
        false,
        &["check-in", "--lifecycle-event", "recipient-return"],
    );
    let out = w.text(1, false, &["inbox"]);
    assert!(out.contains("human and temporarily unavailable"), "{out}");
    assert_eq!(w.state(&m, 1), "displayed");
}

/// The printed continuation is executed verbatim (including bound selectors),
/// with the same cooperative caller, across a real private daemon restart.
#[test]
fn lazy_config_multichunk_restart_printed_continuations() {
    let w = World::agents_only();
    // 11.7 KB: more than two 4096-byte pages, so at least two continuations.
    let body = "chunk-é-界-".repeat(900);
    let m = w.send(&body, &[]);
    let mut out = w.text(1, false, &["inbox", "--max-bytes", "4096"]);
    assert_eq!(w.state(&m, 1), "pending");
    w.ok(None, false, &["daemon", "stop"]);
    w.ok(None, false, &["daemon", "ensure"]);
    let mut full = String::new();
    let mut pages = 0;
    loop {
        assert!(
            out.len() <= 4096,
            "encoded text page exceeds requested bound"
        );
        // Body lines are indented by the actual text encoder; this body has no
        // line breaks or escaping, so concatenation must recover its exact bytes.
        for line in out
            .lines()
            .filter_map(|l| l.strip_prefix("  "))
            .filter(|l| !l.starts_with("read: "))
        {
            full.push_str(line);
        }
        let next = out
            .lines()
            .find_map(|l| l.strip_prefix("next: "))
            .map(str::to_owned);
        let Some(next) = next else { break };
        let argv = shlex::split(&next).expect("printed executable argv");
        assert_eq!(argv[0], "herdr-threads");
        assert!(
            !argv.iter().any(|a| a == "--seat"),
            "own continuation must remain own text: {argv:?}"
        );
        let refs: Vec<_> = argv[1..].iter().map(String::as_str).collect();
        out = w.text(1, false, &refs);
        pages += 1;
        assert!(pages < 40, "nonprogressing continuation {next}");
    }
    assert!(pages > 1, "did not exercise chunk continuation");
    assert_eq!(full, body);
    assert_eq!(w.state(&m, 1), "displayed");
}
#[test]
fn lazy_config_lost_completion_reply_frozen_retry() {
    let w = World::agents_only();
    let m = w.send("lost reply body", &[]);
    let p = Proxy::new(w.paths(), &w.root);
    p.mode.store(2, Ordering::SeqCst);
    let o = w.raw(Some(1), false, &["inbox"]);
    assert!(
        !o.status.success(),
        "completion reply was intentionally lost"
    );
    assert!(String::from_utf8_lossy(&o.stdout).contains("lost reply body"));
    assert_eq!(
        w.state(&m, 1),
        "displayed",
        "canonical mutation committed before reply loss"
    );
    let stderr = String::from_utf8_lossy(&o.stderr);
    let reference = stderr
        .split_whitespace()
        .find(|s| s.starts_with("local:"))
        .expect("exact pending retry reference")
        .trim_end_matches(['.', ',', ';']);
    let original = p
        .requests()
        .into_iter()
        .find(|q| q["command"]["kind"] == "complete_inbox_delivery")
        .expect("real completion submitted")["command"]
        .clone();
    p.mode.store(0, Ordering::SeqCst);
    w.text(1, false, &["retry", reference]);
    let commands: Vec<_> = p
        .requests()
        .iter()
        .filter(|q| q["command"]["kind"] == "complete_inbox_delivery")
        .map(|q| q["command"].clone())
        .collect();
    assert_eq!(commands, vec![original.clone(), original]);
    // Successful cleanup removes the short ref; repeating it refuses before a
    // further completion submission rather than recreating an intent.
    let repeated = w.raw(Some(1), false, &["retry", reference]);
    assert!(!repeated.status.success());
    assert!(String::from_utf8_lossy(&repeated.stderr).contains("intent reference not found"));
    assert_eq!(
        p.requests()
            .iter()
            .filter(|q| q["command"]["kind"] == "complete_inbox_delivery")
            .count(),
        2
    );
    assert_eq!(w.state(&m, 1), "displayed");
}
