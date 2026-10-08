//! The README conversation through the public CLI and a real private daemon.
//! Only the external Herdr socket and harness version reporters are scripted.
//! Cooperative actors are model-free claims; this is not native/model evidence.
use super::sweep::{Scratch, host_reply, pane};
use herdr_threads::{
    daemon::paths::{InstancePaths, RuntimeContext},
    store::effective::{EffectiveReceiptState, effective_receipt},
    test_support::spawn::{OwnedChild, SpawnOwned},
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::{
        fs::{DirBuilderExt, PermissionsExt},
        net::UnixListener,
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, channel},
    },
    thread::JoinHandle,
    time::Duration,
};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");
const ALICE: &str = "You are Alice. Read this thread, make the case for spaces, and discuss it with Bob when he joins.";
const BOB: &str = "You are Bob. Read this thread, make the case for tabs, and agree on a recommendation with Alice.";
const QUESTION: &str = "Please settle on one recommendation and explain the tradeoff.";

/// Model the actual agent.start shell argument restriction, at the socket
/// boundary rather than by inspecting the bootstrap builder's source.
struct GuardedHost {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    starts: Arc<Mutex<Vec<Value>>>,
}
impl GuardedHost {
    fn start(socket: &Path) -> Self {
        let listener = UnixListener::bind(socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let starts = Arc::new(Mutex::new(Vec::new()));
        let recorded = starts.clone();
        let worker = std::thread::spawn(move || {
            let mut panes = vec![
                pane("w1:p1", "human-terminal"),
                pane("w1:p2", "alice-terminal"),
                pane("w1:p3", "bob-terminal"),
            ];
            panes[0]["label"] = json!("human");
            panes[1]["label"] = json!("alice");
            panes[2]["label"] = json!("bob");
            while !stopped.load(Ordering::SeqCst) {
                let mut stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(e) => panic!("private host accept: {e}"),
                };
                if stream.set_nonblocking(false).is_err()
                    || stream
                        .set_read_timeout(Some(Duration::from_millis(250)))
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
                let reply = if request["method"] == "agent.start" {
                    let params = &request["params"];
                    let args = params["args"].as_array().expect("argument array");
                    if args
                        .iter()
                        .any(|arg| arg.as_str().is_none_or(|s| s.contains(['\n', '\r'])))
                    {
                        json!({"id":request["id"],"error":{"code":"invalid_argument","message":"agent.start argument contains line breaks"}})
                    } else {
                        let target = panes
                            .iter_mut()
                            .find(|p| p["pane_id"] == params["pane_id"])
                            .expect("known target");
                        target["agent"] = params["kind"].clone();
                        target["agent_status"] = json!("working");
                        target["name"] = params["name"].clone();
                        target["interactive_ready"] = json!(true);
                        recorded.lock().unwrap().push(params.clone());
                        let mut argv = vec![params["kind"].clone()];
                        argv.extend(args.iter().cloned());
                        json!({"id":request["id"],"result":{"type":"agent_started","argv":argv,"agent":target}})
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
            starts,
        }
    }
}
impl Drop for GuardedHost {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}

struct World {
    root: PathBuf,
    state: PathBuf,
    socket: PathBuf,
    paths: InstancePaths,
    host: GuardedHost,
    _scratch: Scratch,
}
impl World {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "htr-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        ));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let scratch = Scratch(root.clone());
        fs::create_dir(root.join("bin")).unwrap();
        for (kind, version) in [
            ("claude", "2.1.285 (Claude Code)"),
            ("codex", "codex-cli 0.158.0"),
        ] {
            let path = root.join("bin").join(kind);
            // A reporter cannot accidentally become an agent/model process.
            fs::write(&path, format!("#!/bin/sh\nif [ \"$#\" = 1 ] && [ \"$1\" = --version ]; then\n  echo '{version}'\nelse\n  echo 'model execution forbidden in README regression' >&2\n  exit 99\nfi\n")).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let shell = root.join("shell");
        fs::write(&shell, "#!/bin/sh\n# Private shell-probe stand-in: no startup files or config overrides.\nexit 0\n").unwrap();
        fs::set_permissions(shell, fs::Permissions::from_mode(0o700)).unwrap();
        let socket = root.join("host.sock");
        let host = GuardedHost::start(&socket);
        let state = root.join("state");
        let paths = InstancePaths::resolve(
            &RuntimeContext::explicit(state.clone(), socket.clone(), None).unwrap(),
        )
        .unwrap();
        let world = Self {
            root,
            state,
            socket,
            paths,
            host,
            _scratch: scratch,
        };
        for kind in ["claude", "codex"] {
            world.json(None, &["setup", kind]);
        }
        world.json(None, &["daemon", "ensure"]);
        world
    }
    fn command(&self, actor: Option<(&str, &str, &str)>, human_route: bool) -> Command {
        let mut cmd = crate::scrubbed_command(BIN);
        if human_route {
            cmd.arg("human");
        }
        cmd.arg("--state-dir")
            .arg(&self.state)
            .arg("--host-endpoint")
            .arg(&self.socket)
            .env("HOME", self.root.join("home"))
            .env("CLAUDE_CONFIG_DIR", self.root.join("claude"))
            .env("CODEX_HOME", self.root.join("codex"))
            .env("SHELL", self.root.join("shell"))
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", self.root.join("bin").display()),
            )
            .env("HERDR_PANE_ID", actor.map_or("w1:p1", |(_, pane, _)| pane))
            .env("NO_COLOR", "1");
        if let Some((seat, pane, kind)) = actor {
            cmd.args([
                "--cooperative-seat",
                seat,
                "--cooperative-target",
                pane,
                "--cooperative-harness",
                kind,
                "--cooperative-role",
                "top-level",
            ]);
        }
        cmd
    }
    fn run(&self, actor: Option<(&str, &str, &str)>, args: &[&str], json: bool) -> String {
        let human_route = args.first() == Some(&"human");
        let args = if human_route { &args[1..] } else { args };
        let mut cmd = self.command(actor, human_route);
        if json {
            cmd.arg("--json");
        }
        let output = cmd.args(args).output().unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
        String::from_utf8(output.stdout).unwrap()
    }
    fn json(&self, actor: Option<(&str, &str, &str)>, args: &[&str]) -> Value {
        let value: Value = serde_json::from_str(&self.run(actor, args, true)).unwrap();
        if let Some(handoff) = value.get("handoff") {
            handoff.clone()
        } else {
            value["result"]["data"].clone()
        }
    }
    fn count(&self, sql: &str) -> i64 {
        self.db().query_row(sql, [], |row| row.get(0)).unwrap()
    }
    fn db(&self) -> rusqlite::Connection {
        let db = rusqlite::Connection::open_with_flags(
            &self.paths.database_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        db.busy_timeout(Duration::from_secs(2)).unwrap();
        db
    }
    fn pending_pairs(&self, expected: &[(&str, &str)]) {
        let pending = self.json(None, &["pending-receipts", "--thread", "review"]);
        assert_eq!(pending["has_more"], false);
        let mut actual = pending["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| {
                (
                    item["message"].as_str().unwrap().to_owned(),
                    item["seat"].as_str().unwrap().to_owned(),
                )
            })
            .collect::<Vec<_>>();
        let mut expected = expected
            .iter()
            .map(|(message, seat)| (message.to_string(), seat.to_string()))
            .collect::<Vec<_>>();
        actual.sort();
        expected.sort();
        assert_eq!(actual, expected);
    }
    fn receipt(&self, message: &str, seat: &str, state: EffectiveReceiptState) {
        let receipt = effective_receipt(&self.db(), message, seat)
            .unwrap()
            .expect("canonical receipt");
        assert_eq!(receipt.state, state, "{message} -> {seat}");
        if state == EffectiveReceiptState::Acknowledged {
            assert_eq!(receipt.ack_actor_seat_id.as_deref(), Some(seat));
            let observation: Value =
                serde_json::from_str(receipt.ack_observation.as_deref().unwrap()).unwrap();
            assert_eq!(observation["provenance"], "cooperative_top_level");
            assert_eq!(
                observation["action_provenance"],
                "cooperative_inbox_display"
            );
        }
    }
}
impl Drop for World {
    fn drop(&mut self) {
        let _ = self.command(None, false).args(["daemon", "stop"]).output();
    }
}

struct Follower {
    child: OwnedChild,
    reader: Option<JoinHandle<()>>,
    lines: Receiver<String>,
    seen: Vec<String>,
}
impl Follower {
    fn start(world: &World) -> Self {
        let mut child = world
            .command(None, false)
            .args(["read", "review", "--follow"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn_owned()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = channel();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            reader: Some(reader),
            lines,
            seen: Vec::new(),
        }
    }
    fn wait_for(&mut self, needle: &str) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if let Ok(line) = self.lines.recv_timeout(Duration::from_millis(50)) {
                let found = line.contains(needle);
                self.seen.push(line);
                if found {
                    return;
                }
            }
        }
        panic!("follow missed {needle:?}: {:?}", self.seen);
    }
}
impl Drop for Follower {
    fn drop(&mut self) {
        self.child.stop();
        if let Some(reader) = self.reader.take() {
            reader.join().unwrap();
        }
    }
}

/// Kills the multiline bootstrap regression, implicit launch acceptance/ACK,
/// duplicate channel creation, missing pane ACK routing and follow omissions.
#[test]
fn readme_tryout_named_handoffs_and_conversation_obey_launch_and_receipt_boundaries() {
    let world = World::new();
    let human = world.json(None, &["human", "me", "init"])["context"]["seat"]
        .as_str()
        .unwrap()
        .to_owned();
    let alice = world.json(
        None,
        &[
            "human",
            "handoff",
            "--new-thread",
            "--thread-name",
            "review",
            "--topic",
            "Tabs or spaces?",
            "--pane",
            "alice",
            "--kind",
            "claude",
            "--",
            ALICE,
        ],
    );
    let bob = world.json(
        None,
        &[
            "human", "handoff", "--thread", "review", "--pane", "bob", "--kind", "codex", "--", BOB,
        ],
    );
    for report in [&alice, &bob] {
        assert_eq!(report["outcome"], "started", "{report}");
        assert_eq!(report["failed"], false, "{report}");
    }
    assert_eq!(alice["thread"], bob["thread"]);
    assert_eq!(world.count("SELECT count(*) FROM threads"), 1);
    assert_eq!(
        world.count("SELECT count(*) FROM memberships WHERE state='joined'"),
        1
    );
    assert_eq!(
        world.count("SELECT count(*) FROM invitations WHERE state='pending'"),
        2
    );
    assert_eq!(world.count("SELECT count(*) FROM occupant_bindings WHERE observation_provenance='managed_launch' AND registered_at IS NULL AND ended_at IS NULL"), 2);
    assert_eq!(
        world.count("SELECT count(*) FROM channel_handoff_fences WHERE state='completed'"),
        2
    );
    assert_eq!(
        world.count("SELECT count(*) FROM channel_handoff_fences WHERE state='live'"),
        0
    );
    assert_eq!(world.json(None, &["pending-ops"])["items"], json!([]));
    let starts = world.host.starts.lock().unwrap().clone();
    assert_eq!(starts.len(), 2);
    for (start, kind, target) in [
        (&starts[0], "claude", "w1:p2"),
        (&starts[1], "codex", "w1:p3"),
    ] {
        assert_eq!(start["kind"], kind);
        assert_eq!(start["pane_id"], target);
        let args = start["args"].as_array().unwrap();
        assert!(
            args.iter()
                .all(|arg| !arg.as_str().unwrap().contains(['\n', '\r']))
        );
        assert!(!args.iter().any(|arg| matches!(
            arg.as_str().unwrap(),
            "--dangerously-skip-permissions"
                | "--dangerously-bypass-approvals-and-sandbox"
                | "--full-auto"
                | "--permission-mode"
                | "--approval-policy"
        )));
    }
    let a_seat = alice["seat"].as_str().unwrap();
    let b_seat = bob["seat"].as_str().unwrap();
    let a = (a_seat, "w1:p2", "claude");
    let b = (b_seat, "w1:p3", "codex");
    world.pending_pairs(&[
        (alice["message"]["data"].as_str().unwrap(), a_seat),
        (bob["message"]["data"].as_str().unwrap(), b_seat),
    ]);
    for (report, actor, body) in [(&alice, a, ALICE), (&bob, b, BOB)] {
        let message = report["message"]["data"].as_str().unwrap();
        world.receipt(message, actor.0, EffectiveReceiptState::Pending);
        let recorded: (String, String, String) = world
            .db()
            .query_row(
                "SELECT actor_seat_id,body,author_role FROM messages WHERE id=?1",
                [message],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(recorded, (human.clone(), body.to_owned(), "human".into()));
        world.json(Some(actor), &["check-in", "--lifecycle-event", "initial"]);
        world.receipt(message, actor.0, EffectiveReceiptState::Pending);
        world.json(Some(actor), &["accept", "review"]);
        world.receipt(message, actor.0, EffectiveReceiptState::Pending);
        let inbox = world.json(Some(actor), &["inbox"]);
        // The advertised v2 inbox carries addressed messages, not the legacy
        // per-thread pending count. JSON offers an ACK candidate but cannot ACK.
        assert_eq!(inbox["has_more"], false, "{inbox}");
        let items = inbox["items"].as_array().unwrap();
        assert_eq!(items.len(), 1, "{inbox}");
        let item = &items[0];
        assert_eq!(item["kind"], "message");
        assert_eq!(item["thread"], report["thread"]);
        assert_eq!(item["message"], message);
        assert_eq!(item["sender"], human);
        assert_eq!(item["author_role"], "human");
        assert_eq!(item["body"], body);
        assert_eq!(item["body_start"], 0);
        assert_eq!(item["body_end"], body.len());
        assert_eq!(item["body_len"], body.len());
        assert_eq!(item["ack_candidate"], message);
        world.receipt(message, actor.0, EffectiveReceiptState::Pending);
        assert!(world.run(Some(actor), &["inbox"], false).contains(body));
        world.receipt(message, actor.0, EffectiveReceiptState::Acknowledged);
    }
    assert_eq!(
        world.count("SELECT count(*) FROM memberships WHERE state='joined'"),
        3
    );
    assert_eq!(
        world.count("SELECT count(*) FROM invitations WHERE state='accepted'"),
        2
    );
    let db = world.db();
    let mut joined = db
        .prepare("SELECT seat_id FROM memberships WHERE state='joined' ORDER BY seat_id")
        .unwrap();
    let actual = joined
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let mut expected = vec![human.clone(), a_seat.to_owned(), b_seat.to_owned()];
    expected.sort();
    assert_eq!(actual, expected);
    assert_eq!(
        db.query_row("SELECT name,topic FROM threads", [], |row| Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?
        )))
        .unwrap(),
        ("review".to_owned(), "Tabs or spaces?".to_owned())
    );
    let question = world
        .json(
            None,
            &[
                "human",
                "send",
                "review",
                "--require-ack-pane",
                "alice",
                "--require-ack-pane",
                "bob",
                "--body",
                QUESTION,
            ],
        )
        .as_str()
        .unwrap()
        .to_owned();
    world.pending_pairs(&[(&question, a_seat), (&question, b_seat)]);
    assert!(
        world
            .run(None, &["read", "review"], false)
            .contains(QUESTION)
    );
    let mut follower = Follower::start(&world);
    follower.wait_for(QUESTION);
    for actor in [a, b] {
        world.receipt(&question, actor.0, EffectiveReceiptState::Pending);
        assert!(
            world
                .run(Some(actor), &["inbox", "--machine"], false)
                .contains(alice["thread"].as_str().unwrap())
        );
        world.receipt(&question, actor.0, EffectiveReceiptState::Pending);
        assert!(world.run(Some(actor), &["inbox"], false).contains(QUESTION));
        world.receipt(&question, actor.0, EffectiveReceiptState::Acknowledged);
    }
    for (actor, body) in [
        (a, "Spaces keep indentation consistent across editors."),
        (b, "Tabs allow each reader to choose indentation width."),
        (
            a,
            "Recommendation: use spaces for predictable formatting; tabs preserve reader preferences.",
        ),
    ] {
        world.json(Some(actor), &["send", "review", "--body", body]);
        follower.wait_for(body);
        assert_eq!(
            follower
                .seen
                .iter()
                .filter(|line| line.contains(body))
                .count(),
            1
        );
        assert!(world.run(None, &["read", "review"], false).contains(body));
    }
    assert_eq!(
        world.count("SELECT count(*) FROM messages WHERE kind='ordinary'"),
        6
    );
    assert_eq!(
        world.count("SELECT count(*) FROM messages WHERE kind='ordinary' AND author_role='human'"),
        3
    );
    assert_eq!(
        world.count("SELECT count(*) FROM messages WHERE kind='ordinary' AND author_role='agent'"),
        3
    );
    assert_eq!(world.count("SELECT count(*) FROM threads"), 1);
    assert_eq!(world.count("SELECT count(*) FROM seats"), 3);
    assert_eq!(world.host.starts.lock().unwrap().len(), 2);
    world.pending_pairs(&[]);
}
