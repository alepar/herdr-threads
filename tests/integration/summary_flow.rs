//! Thread summaries end to end (ht-1ip.16): the installed executable and its
//! detached daemon against a private Herdr endpoint, with a scripted worker
//! that drives the CLI exactly as a seat's summary worker would (spec §4-§9).
//! No model runs: the worker's submissions are built from the job bundles by
//! code, and the stand-in hook payloads are the native JSON a harness writes.
//!
//! One thread, five info events and twelve ordinary messages from A, with a
//! chunk size small enough that the sequences fall into six full level-0
//! chunks and a one-message raw tail (see `Fixture::build`).

use super::sweep::{FakeHost, Scratch, agent_pane, pane};
use herdr_threads::{
    cli::hook::installed_argv,
    daemon::paths::{InstancePaths, RuntimeContext},
    harness::{context::Harness, setup::plan_claude},
    test_support::spawn::SpawnOwned,
};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::PathBuf,
    process::Stdio,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");

/// Settings written before the daemon starts (spec §13). Small chunks give
/// two messages per chunk; `p99_cold_ms` is the extension a catch-up entry
/// and every stored block grants, `exit_grace_ms` the one after Ready.
const SETTINGS: &str = r#"{"summary":{"chunk_bytes":1024,"display_bytes":4096,"narrative_bytes":512,"p99_cold_ms":8000,"exit_grace_ms":1000}}"#;
const P99_COLD_MS: u64 = 8_000;
const EXIT_GRACE_MS: u64 = 1_000;
/// The frozen deadline, in seconds, of the one explicit receipt A sends B.
const ACK_DEADLINE_SECONDS: u64 = 3;
/// Sequence of the `--relays-user` message (first message of chunk 2).
const PRIORITY_SEQ: u64 = 9;
/// Head of the fixture thread: the require-ACK message, the raw tail.
const HEAD: u64 = 17;
const FILL: &str = "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima mike november oscar papa quebec romeo sierra tango uniform victor whiskey xray yankee zulu alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima mike november oscar papa quebec romeo sierra tango uniform victor whiskey xray yankee zulu";

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

/// A stand-in occupant: the cooperative caller flags for one seat and pane.
#[derive(Clone, Copy)]
struct Caller<'a> {
    seat: &'a str,
    pane: &'a str,
}

fn utc_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn wait_until(what: &str, timeout: Duration, mut done: impl FnMut() -> bool) {
    let until = Instant::now() + timeout;
    while !done() {
        assert!(Instant::now() < until, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(200));
    }
}

struct World {
    root: PathBuf,
    state: PathBuf,
    host: PathBuf,
    instance_dir: PathBuf,
    claude_hooks: Value,
    codex_hook: String,
    _host: FakeHost,
    _scratch: Scratch,
}
impl World {
    fn start(panes: Vec<Value>) -> Self {
        let root = PathBuf::from(format!(
            "/private/tmp/htsf-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        ));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let scratch = Scratch(root.clone());
        let state = root.join("state");
        let socket = root.join("herdr.sock");
        let host = FakeHost::start(&socket, panes);
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
        let context = RuntimeContext::explicit(state.clone(), socket.clone(), None).unwrap();
        let paths = InstancePaths::resolve(&context).unwrap();
        paths.prepare_instance_dir().unwrap();
        let file = paths.instance_dir.join("settings.json");
        fs::write(&file, SETTINGS).unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
        let argv = installed_argv(BIN, Some(state.to_str().unwrap()), None, Harness::Claude);
        let plan = plan_claude(b"{}", &argv).unwrap();
        let installed: Value = serde_json::from_slice(&plan.proposed_bytes).unwrap();
        let claude_hooks = installed["hooks"].clone();
        let codex_argv = installed_argv(BIN, Some(state.to_str().unwrap()), None, Harness::Codex);
        let codex_hook = herdr_threads::harness::setup::shell_command(&codex_argv).unwrap();
        let world = Self {
            root,
            state,
            host: socket,
            instance_dir: paths.instance_dir,
            claude_hooks,
            codex_hook,
            _host: host,
            _scratch: scratch,
        };
        let ensured = world.cli(None, None, &["daemon", "ensure"]);
        ensured.data("daemon ensure");
        world
    }
    fn path(&self) -> String {
        format!("{}:/usr/bin:/bin", self.root.join("bin").display())
    }
    /// The CLI with `--json`, as `caller` when given.
    fn cli(&self, caller: Option<Caller>, stdin: Option<&str>, args: &[&str]) -> Out {
        self.exec(caller, stdin, args, true)
    }
    /// The CLI in its human format (no `--json`).
    fn human(&self, caller: Option<Caller>, args: &[&str]) -> Out {
        self.exec(caller, None, args, false)
    }
    fn exec(&self, caller: Option<Caller>, stdin: Option<&str>, args: &[&str], json: bool) -> Out {
        self.exec_in_pane(caller, None, stdin, args, json)
    }
    /// A person's actual pane context, initialized by `me init`; no agent
    /// caller claim is relabeled as human.
    fn person(&self, pane: &str, args: &[&str]) -> Out {
        self.exec_in_pane(None, Some(pane), None, args, true)
    }
    fn exec_in_pane(
        &self,
        caller: Option<Caller>,
        pane: Option<&str>,
        stdin: Option<&str>,
        args: &[&str],
        json: bool,
    ) -> Out {
        let mut command = crate::scrubbed_command(BIN);
        if json {
            command.arg("--json");
        }
        command
            .arg("--state-dir")
            .arg(&self.state)
            .arg("--host-endpoint")
            .arg(&self.host);
        if let Some(caller) = caller {
            command.args([
                "--cooperative-seat",
                caller.seat,
                "--cooperative-target",
                caller.pane,
                "--cooperative-harness",
                "claude",
                "--cooperative-role",
                "top-level",
            ]);
        }
        // `scrubbed_command` already removed the test runner's own agent
        // environment (TRUST-POLICY A4 agent-marker guard).
        command
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
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(pane) = pane {
            command.env("HERDR_PANE_ID", pane).env("HERDR_ENV", "1");
        }
        let mut child = command.spawn_owned().unwrap();
        if let Some(text) = stdin {
            child
                .stdin
                .take()
                .unwrap()
                .write_all(text.as_bytes())
                .unwrap();
        }
        collect(child.wait_with_output().unwrap())
    }
    /// The installed hook, exactly as setup installs it: `sh -c` with the
    /// native JSON on stdin and the pane identity from `HERDR_*`.
    fn hook(&self, harness: &str, pane: &str, stdin: &str) -> Out {
        let command = if harness == "codex" {
            self.codex_hook.as_str()
        } else {
            let payload = serde_json::from_str::<Value>(stdin).unwrap();
            let event = payload["hook_event_name"].as_str().unwrap();
            self.claude_hooks[event][0]["hooks"][0]["command"]
                .as_str()
                .unwrap_or_else(|| panic!("no installed Claude registration for {event}"))
        };
        let mut child = crate::scrubbed_command("/bin/sh")
            .arg("-c")
            .arg(command)
            .env("HERDR_ENV", "1")
            .env("HERDR_PANE_ID", pane)
            .env("HERDR_SOCKET_PATH", &self.host)
            .env("PATH", self.path())
            .env_remove("HERDR_PLUGIN_STATE_DIR")
            .env_remove("HERDR_BIN_PATH")
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
    fn db(&self) -> rusqlite::Connection {
        let db = rusqlite::Connection::open_with_flags(
            self.instance_dir.join("threads.sqlite3"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        db.busy_timeout(Duration::from_secs(5)).unwrap();
        db
    }
    /// The real compact message API as JSON. Public `--json inbox` is a
    /// thread-count aggregate; own text inbox uses this API and may then ACK.
    /// Calling its read boundary directly keeps this test free of display ACKs.
    fn inbox_batch_json(&self, seat: &str) -> Value {
        use herdr_threads::{
            app::SystemClock,
            client::local::LocalSocketClient,
            daemon::ownership::{read_descriptor, read_existing_namespace},
            protocol::{
                commands::{Command, InboxQuery},
                ids::SeatId,
                output::{OutputFormat, OutputSpec, encode_selected},
                pagination::PageRequest,
                time::{CallBudget, Cancellation, Clock, MonoInstant},
            },
        };
        use std::sync::Arc;

        let paths = InstancePaths::resolve(
            &RuntimeContext::explicit(self.state.clone(), self.host.clone(), None).unwrap(),
        )
        .unwrap();
        let instance = read_existing_namespace(&paths).unwrap().unwrap();
        let descriptor = read_descriptor(&paths, instance).unwrap();
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let client = LocalSocketClient::new(
            descriptor.endpoint,
            Arc::clone(&clock),
            instance,
            Some(descriptor.boot_id),
        );
        let spec = OutputSpec {
            format: OutputFormat::Json,
            ..Default::default()
        };
        let result = client
            .call_with_output(
                Command::InboxBatch(InboxQuery {
                    seat: Some(SeatId::new(seat)),
                    page: PageRequest {
                        limit: 100,
                        ..Default::default()
                    },
                }),
                &spec,
                &CallBudget {
                    deadline: MonoInstant(clock.monotonic_now().0 + 5_000),
                    cancellation: Cancellation::default(),
                },
            )
            .unwrap();
        let encoded = encode_selected(&result, &spec).unwrap();
        let envelope: Value = serde_json::from_slice(&encoded).unwrap();
        envelope["result"]["data"].clone()
    }
    /// `(state, end_reason, frontier, extension_until, ended_at)` of the
    /// seat's catch-up row on the thread.
    fn catch_up(&self, seat: &str, thread: &str) -> Option<CatchUp> {
        self.db()
            .query_row(
                "SELECT state,end_reason,frontier_seq,extension_until,ended_at FROM catch_up \
                 WHERE seat_id=?1 AND thread_id=?2",
                [seat, thread],
                |r| {
                    Ok(CatchUp {
                        state: r.get(0)?,
                        end_reason: r.get(1)?,
                        frontier: r.get(2)?,
                        extension_until: r.get(3)?,
                        ended_at: r.get(4)?,
                    })
                },
            )
            .ok()
    }
    fn catch_up_rows(&self, seat: &str) -> i64 {
        self.db()
            .query_row(
                "SELECT count(*) FROM catch_up WHERE seat_id=?1",
                [seat],
                |r| r.get(0),
            )
            .unwrap()
    }
    /// The seat's wake-work attention version (bumped by every committed
    /// attention change that re-derives its wake reasons).
    fn wake_attention(&self, seat: &str) -> i64 {
        self.db()
            .query_row(
                "SELECT attention_version FROM wake_work WHERE seat_id=?1",
                [seat],
                |r| r.get(0),
            )
            .unwrap_or(0)
    }
    fn pending(&self, seat: &str) -> Vec<Value> {
        self.cli(
            None,
            None,
            &["pending-receipts", "--seat", seat, "--limit", "100"],
        )
        .data("pending-receipts")["items"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }
    fn pending_for(&self, seat: &str, message: &str) -> Value {
        self.pending(seat)
            .into_iter()
            .find(|item| item["message"] == message)
            .unwrap_or_else(|| panic!("no pending receipt for {message} at {seat}"))
    }
    fn overdue_subjects(&self) -> Vec<String> {
        self.cli(None, None, &["overdue", "--limit", "100"])
            .data("overdue")["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["subject"].as_str().unwrap().to_owned())
            .collect()
    }
    fn warnings(&self, seat: &str) -> Vec<Value> {
        self.cli(None, None, &["warnings", "--seat", seat, "--limit", "100"])
            .data("warnings")["items"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }
}
impl Drop for World {
    fn drop(&mut self) {
        let _ = self.cli(None, None, &["daemon", "stop"]);
        if std::thread::panicking()
            && let Ok(log) = fs::read(self.instance_dir.join("daemon.log"))
        {
            let tail = &log[log.len().saturating_sub(16 * 1024)..];
            eprintln!(
                "summary fixture daemon log:\n{}",
                String::from_utf8_lossy(tail)
            );
        }
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

#[derive(Debug)]
struct CatchUp {
    state: String,
    end_reason: Option<String>,
    frontier: i64,
    extension_until: Option<i64>,
    ended_at: Option<i64>,
}

fn claude(id: &str, terminal: &str, session: &str) -> Value {
    agent_pane(id, terminal, "claude", Some(session))
}
fn session_start(harness: &str, session: &str, source: &str) -> String {
    match harness {
        "codex" => format!(
            r#"{{"session_id":"{session}","turn_id":"t1","hook_event_name":"SessionStart","source":"{source}"}}"#
        ),
        _ => format!(
            r#"{{"session_id":"{session}","transcript_path":"/tmp/t.jsonl","cwd":"/tmp","hook_event_name":"SessionStart","source":"{source}"}}"#
        ),
    }
}
/// A PreToolUse boundary: the hook's attention digest is compared with the
/// seat's stored mark and printed only when something new arrived.
fn tool_boundary(session: &str) -> String {
    format!(
        r#"{{"session_id":"{session}","transcript_path":"/tmp/t.jsonl","cwd":"/tmp","permission_mode":"default","hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{{"command":"ls"}},"tool_use_id":"toolu_{}"}}"#,
        uuid::Uuid::new_v4().simple()
    )
}

/// One ordinary message body of about 250 bytes: with the header line two of
/// them close a 1024-byte chunk and one alone never does (the five info events before them stay well under it).
fn body(index: usize) -> String {
    format!("m{index} {}", &FILL.repeat(2)[..600])
}

struct Fixture {
    world: World,
    a: String,
    b: String,
    c: String,
    thread: String,
    /// The one explicit require-ACK message A sent B (sequence `HEAD`).
    ack: String,
}
const PANE_A: &str = "w1:p1";
const PANE_B: &str = "w1:p2";
const PANE_C: &str = "w1:p3";
const PANE_D: &str = "w1:p4";
const SESSION_B: &str = "SB";

impl Fixture {
    /// Daemon + stand-in Herdr + three cooperative seats: A (sender), B (the
    /// summarizer, registered through its SessionStart hook so the tool
    /// boundary hook can read its attention digest) and C (second reader).
    /// Thread T has A, B and C joined. A sends twelve ordinary messages:
    ///
    /// | chunk | sequences | content                                         |
    /// |-------|-----------|-------------------------------------------------|
    /// | 0     | 1-6       | five info events, then m0                       |
    /// | 1     | 7-8       | m1, m2                                          |
    /// | 2     | 9-10      | m3 (`--relays-user`, priority), m4              |
    /// | 3     | 11-12     | m5, m6                                          |
    /// | 4     | 13-14     | m7, m8                                          |
    /// | 5     | 15-16     | m9, m10                                         |
    /// | tail  | 17        | m11, require-ACK to B with a short deadline     |
    fn build() -> Self {
        let world = World::start(vec![
            claude(PANE_A, "term-a", "SA"),
            claude(PANE_B, "term-b", SESSION_B),
            pane(PANE_C, "term-c"),
            agent_pane(PANE_D, "term-d", "codex", Some("cx-sess")),
        ]);
        let resolve = |pane: &str| {
            world
                .cli(None, None, &["seat", "resolve", "--pane", pane])
                .text("seat resolve")
        };
        let (a, b, c) = (resolve(PANE_A), resolve(PANE_B), resolve(PANE_C));
        for (pane, session) in [(PANE_A, "SA"), (PANE_B, SESSION_B)] {
            let started = world.hook("claude", pane, &session_start("claude", session, "startup"));
            assert_eq!(started.code, 0, "{}", started.stderr);
        }
        let caller_c = Caller {
            seat: &c,
            pane: PANE_C,
        };
        world
            .cli(
                Some(caller_c),
                None,
                &["check-in", "--lifecycle-event", "c-start"],
            )
            .data("c check-in");
        let author = Caller {
            seat: &a,
            pane: PANE_A,
        };
        let thread = world
            .cli(
                Some(author),
                None,
                &["thread", "create", "--topic", "summary flow"],
            )
            .text("thread create");
        for (seat, pane) in [(&b, PANE_B), (&c, PANE_C)] {
            world
                .cli(Some(author), None, &["invite", &thread, "--seat", seat])
                .data("invite");
            world
                .cli(Some(Caller { seat, pane }), None, &["accept", &thread])
                .data("accept");
        }
        let mut ack = String::new();
        for index in 0..12 {
            let text = body(index);
            let mut args = vec!["send", thread.as_str(), "--body", text.as_str()];
            let deadline = ACK_DEADLINE_SECONDS.to_string();
            if index == 3 {
                args.push("--relays-user");
            }
            if index == 11 {
                args.extend(["--require-ack", b.as_str(), "--deadline", deadline.as_str()]);
            }
            let sent = world.cli(Some(author), None, &args).text("send");
            if index == 11 {
                ack = sent;
            }
        }
        Self {
            world,
            a,
            b,
            c,
            thread,
            ack,
        }
    }
    fn caller_a(&self) -> Caller<'_> {
        Caller {
            seat: &self.a,
            pane: PANE_A,
        }
    }
    fn caller_b(&self) -> Caller<'_> {
        Caller {
            seat: &self.b,
            pane: PANE_B,
        }
    }
    fn caller_c(&self) -> Caller<'_> {
        Caller {
            seat: &self.c,
            pane: PANE_C,
        }
    }
    /// `summary T` as `caller`, parsed: `{"status": ..., "data": ...}`.
    fn summary(&self, caller: Caller) -> Value {
        self.world
            .cli(Some(caller), None, &["summary", &self.thread])
            .data("summary")
    }
    fn send_as_a(&self, text: &str, flags: &[&str]) -> String {
        let mut args = vec!["send", self.thread.as_str(), "--body", text];
        args.extend_from_slice(flags);
        self.world
            .cli(Some(self.caller_a()), None, &args)
            .text("send")
    }
    /// B's boundary hook: the digest text, empty when nothing new arrived.
    fn boundary_b(&self) -> String {
        let hook = self.world.hook("claude", PANE_B, &tool_boundary(SESSION_B));
        assert_eq!(hook.code, 0, "{}", hook.stderr);
        hook.stdout
    }
}

// ---- the scripted worker ----

/// The level-0 chunks of the fixture thread.
const CHUNKS: [(u64, u64); 6] = [(1, 6), (7, 8), (9, 10), (11, 12), (13, 14), (15, 16)];
/// The chunk whose first two submissions are invalid (fallback).
const FALLBACK_CHUNK: usize = 4;

#[derive(Clone, Debug)]
struct Ticket {
    job: String,
    lease: String,
    index: usize,
    first: u64,
    last: u64,
}
fn tickets(work: &Value) -> Vec<Ticket> {
    assert_eq!(work["status"], "work", "{work}");
    work["data"]["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|job| Ticket {
            job: job["job_id"].as_str().unwrap().to_owned(),
            lease: job["lease_token"].as_str().unwrap().to_owned(),
            index: job["index"].as_u64().unwrap() as usize,
            first: job["range"]["first_seq"].as_u64().unwrap(),
            last: job["range"]["last_seq"].as_u64().unwrap(),
        })
        .collect()
}

impl Fixture {
    /// `summary job`: the one-line tagged outcome, `{"status":..,"data":..}`.
    fn fetch(&self, caller: Caller, ticket: &Ticket) -> Value {
        let out = self.world.cli(
            Some(caller),
            None,
            &["summary", "job", &ticket.job, "--lease", &ticket.lease],
        );
        assert_eq!(out.code, 0, "{}{}", out.stdout, out.stderr);
        out.value
    }
    /// `summary submit` with the submission on stdin; the outcome is
    /// `result.data` (`stored` exits 0, `rejected` exits 1).
    fn submit(&self, caller: Caller, ticket: &Ticket, submission: &Value) -> (i32, Value) {
        let out = self.world.cli(
            Some(caller),
            Some(&submission.to_string()),
            &["summary", "submit", &ticket.job, "--lease", &ticket.lease],
        );
        (out.code, out.value["result"]["data"].clone())
    }
    /// What a model worker would write for chunk `ticket.index`, built from
    /// the bundle it fetched (spec §6). A and B are the thread's two seats.
    fn good_submission(&self, ticket: &Ticket, bundle: &Value) -> Value {
        let (first, last) = (ticket.first, ticket.last);
        let mut submission = json!({
            "submission_schema": herdr_threads::protocol::summary::SUBMISSION_SCHEMA,
            "narrative": format!("{} sent messages #{first}-#{last}.", self.a),
            "prompt_version": "integration-1",
            "model": "scripted",
        });
        let entries = bundle["data"]["fold"]["entries"].as_array().unwrap();
        match ticket.index {
            // An item introduced here, closed by the next chunk's job.
            0 => {
                submission["new_open_items"] = json!([{
                    "ref": "ask0", "seq": 6, "kind": "ask",
                    "from_seat": self.a, "to_seat": self.b,
                    "text": "A asks B to review the plan"
                }]);
            }
            // Closes the item chunk 0 stored: the bundle's fold lists it open.
            1 => {
                let open: Vec<&Value> = entries
                    .iter()
                    .filter(|entry| {
                        entry["item"]["body"]["type"] == "open_item" && entry["status"] == "open"
                    })
                    .collect();
                assert_eq!(
                    open.len(),
                    1,
                    "chunk 1 must see chunk 0's open item: {bundle}"
                );
                submission["transitions"] = json!([{
                    "target": open[0]["item"]["id"], "new_status": "resolved", "cite_seq": 8
                }]);
            }
            // The priority message's prefill instruction `i.9` is introduced
            // and completed within this chunk.
            2 => {
                let own = format!("i.{PRIORITY_SEQ}");
                assert!(
                    entries
                        .iter()
                        .any(|entry| entry["item"]["id"] == own.as_str()
                            && entry["status"] == "open"),
                    "chunk 2's own prefill instruction must be open in its bundle: {bundle}"
                );
                submission["transitions"] =
                    json!([{ "target": own, "new_status": "done", "cite_seq": 10 }]);
            }
            // An item introduced and resolved by its own ref in one chunk.
            3 => {
                submission["new_open_items"] = json!([{
                    "ref": "q3", "seq": 11, "kind": "question", "from_seat": self.a,
                    "text": "A asks whether the migration is reversible"
                }]);
                submission["transitions"] =
                    json!([{ "target": "q3", "new_status": "resolved", "cite_seq": 12 }]);
            }
            _ => {}
        }
        submission
    }
    /// Run `tickets` in ascending chunk order as `caller`; the fallback chunk
    /// takes two invalid submissions. Returns each chunk's block id.
    fn run_jobs(&self, caller: Caller, tickets: &[Ticket]) -> Vec<String> {
        let mut ordered = tickets.to_vec();
        ordered.sort_by_key(|ticket| ticket.index);
        let mut blocks = Vec::new();
        for ticket in &ordered {
            let bundle = self.fetch(caller, ticket);
            assert_eq!(bundle["status"], "bundle", "{bundle}");
            let good = self.good_submission(ticket, &bundle);
            if ticket.index == FALLBACK_CHUNK {
                let invalid = [
                    json!({"submission_schema": 9}),
                    json!({"submission_schema": herdr_threads::protocol::summary::SUBMISSION_SCHEMA, "narrative": "n",
                           "prompt_version": "integration-1", "model": "scripted",
                           "transitions": [{"target": "nope", "new_status": "done", "cite_seq": 999}]}),
                ];
                let (code, first) = self.submit(caller, ticket, &invalid[0]);
                assert_eq!(
                    (code, first["status"].as_str()),
                    (1, Some("rejected")),
                    "{first}"
                );
                assert!(!first["data"]["reasons"].as_array().unwrap().is_empty());
                let (code, second) = self.submit(caller, ticket, &invalid[1]);
                assert_eq!(code, 0, "{second}");
                assert_eq!(second["status"], "stored", "{second}");
                assert_eq!(
                    second["data"]["fallback"], true,
                    "second rejection falls back"
                );
                // Final: a later valid submission returns the stored fallback.
                let (code, later) = self.submit(caller, ticket, &good);
                assert_eq!(code, 0, "{later}");
                assert_eq!(
                    later["data"]["block_id"], second["data"]["block_id"],
                    "{later}"
                );
                assert_eq!(later["data"]["fallback"], true, "{later}");
                blocks.push(second["data"]["block_id"].as_str().unwrap().to_owned());
            } else {
                let (code, stored) = self.submit(caller, ticket, &good);
                assert_eq!(code, 0, "{stored}");
                assert_eq!(stored["status"], "stored", "{stored}");
                assert_eq!(stored["data"]["fallback"], false, "{stored}");
                blocks.push(stored["data"]["block_id"].as_str().unwrap().to_owned());
            }
        }
        blocks
    }
}

fn shapes(tickets: &[Ticket]) -> Vec<(u64, u64)> {
    let mut ranges: Vec<(usize, u64, u64)> =
        tickets.iter().map(|t| (t.index, t.first, t.last)).collect();
    ranges.sort();
    ranges
        .into_iter()
        .map(|(_, first, last)| (first, last))
        .collect()
}

// ---- cases ----

/// Case 1: Work enters catch-up at the frontier and extends the effective
/// deadline of B's receipts; the sender sees the deferral.
#[test]
fn work_enters_catch_up_and_extends_the_deadline() {
    let fx = Fixture::build();
    let frozen = fx.world.pending_for(&fx.b, &fx.ack);
    assert!(
        frozen["effective_deadline"].is_null() && frozen["deferred_until"].is_null(),
        "no catch-up row yet: {frozen}"
    );
    let work = fx.summary(fx.caller_b());
    let jobs = tickets(&work);
    assert_eq!(work["data"]["frontier"], HEAD, "{work}");
    assert_eq!(
        shapes(&jobs),
        CHUNKS,
        "six full chunks, the tail is never a job"
    );
    let row = fx.world.catch_up(&fx.b, &fx.thread).expect("catch-up row");
    assert_eq!(
        (row.state.as_str(), row.frontier),
        ("active", HEAD as i64),
        "{row:?}"
    );
    let entered_extension = row.extension_until.expect("entry extends");

    let extended = fx.world.pending_for(&fx.b, &fx.ack);
    let deadline = extended["deadline"].as_u64().unwrap();
    let effective = extended["effective_deadline"].as_u64().unwrap();
    assert!(
        effective > deadline,
        "effective runs past the frozen deadline: {extended}"
    );
    assert_eq!(effective as i64, entered_extension, "{extended}");
    assert_eq!(
        extended["deferred_until"], extended["effective_deadline"],
        "{extended}"
    );
    assert_eq!(extended["overdue"], false, "{extended}");
    // The frozen deadline stays what it was at send.
    assert_eq!(deadline, frozen["deadline"].as_u64().unwrap());
    assert!(
        effective >= deadline - ACK_DEADLINE_SECONDS * 1000 + P99_COLD_MS,
        "entry grants entry + p99: {extended}"
    );

    // The sender's view carries the deferral line.
    let sender = fx
        .world
        .human(Some(fx.caller_a()), &["pending-receipts", "--seat", &fx.b]);
    assert_eq!(sender.code, 0, "{}", sender.stderr);
    assert!(
        sender
            .stdout
            .contains("deferred: recipient catching up (until"),
        "{}",
        sender.stdout
    );
    let recipients = fx
        .world
        .cli(
            Some(fx.caller_a()),
            None,
            &["delivery", "recipients", &fx.ack],
        )
        .data("delivery recipients");
    let mine = recipients["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["seat"] == fx.b.as_str())
        .unwrap_or_else(|| panic!("{recipients}"));
    assert_eq!(
        mine["effective_deadline"].as_u64(),
        Some(effective),
        "{mine}"
    );
    assert_eq!(mine["deadline"].as_u64(), Some(deadline), "{mine}");
}

/// Case 2: re-polling returns the same tickets, and the frontier does not
/// follow the head while the row is active.
#[test]
fn repoll_returns_the_same_tickets() {
    let fx = Fixture::build();
    let first = fx.summary(fx.caller_b());
    fx.send_as_a("a message above the frontier", &[]);
    let again = fx.summary(fx.caller_b());
    assert_eq!(again, first, "same job ids, lease tokens, frontier");
    assert_eq!(again["data"]["frontier"], HEAD);
    let third = fx.summary(fx.caller_b());
    assert_eq!(third["data"]["jobs"], first["data"]["jobs"]);
}

/// Case 3: during catch-up an ordinary message above F is held (B's digest
/// does not offer it, history shows it, its receipt stays pending) while a
/// `--relays-user` message bypasses the hold.
#[test]
fn hold_and_bypass() {
    let fx = Fixture::build();
    // B's first boundary presents what is pending and records the mark.
    let first = fx.boundary_b();
    assert!(
        first.contains(&fx.ack),
        "the mark starts at everything pending: {first}"
    );
    assert_eq!(fx.summary(fx.caller_b())["status"], "work");
    let held = fx.send_as_a("an ordinary request", &["--require-ack", &fx.b]);
    let quiet = fx.boundary_b();
    assert!(
        !quiet.contains(&held),
        "an ordinary message above F is held from the digest: {quiet}"
    );
    let history = fx
        .world
        .cli(Some(fx.caller_b()), None, &["read", &fx.thread])
        .data("read");
    assert!(
        history["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["message"] == held.as_str()),
        "history is never filtered"
    );
    assert_eq!(
        fx.world.pending_for(&fx.b, &held)["message"],
        held.as_str(),
        "held keeps its receipt"
    );

    let priority = fx.send_as_a(
        "the user says stop",
        &["--relays-user", "--require-ack", &fx.b],
    );
    let offered = fx.boundary_b();
    assert!(
        offered.contains(&priority),
        "a priority message bypasses the hold: {offered}"
    );
    assert!(
        !offered.contains(&held),
        "the ordinary one is still held: {offered}"
    );
}

/// What a completed catch-up leaves behind, for the cases that look at it.
struct Completed {
    blocks: Vec<String>,
    /// The human-format Ready B received (the first Ready ends the row).
    ready_text: String,
    /// Wall-clock bounds of the Ready call.
    ready_between: (u64, u64),
}

impl Fixture {
    /// B's whole catch-up: Work, every job through the scripted worker
    /// (one chunk rejected into a fallback), then Ready in the human format.
    fn complete_catch_up(&self) -> Completed {
        let work = self.summary(self.caller_b());
        let jobs = tickets(&work);
        assert_eq!(shapes(&jobs), CHUNKS);
        let blocks = self.run_jobs(self.caller_b(), &jobs);
        let before = utc_ms();
        let ready = self
            .world
            .human(Some(self.caller_b()), &["summary", &self.thread]);
        let after = utc_ms();
        assert_eq!(ready.code, 0, "{}{}", ready.stdout, ready.stderr);
        Completed {
            blocks,
            ready_text: ready.stdout,
            ready_between: (before, after),
        }
    }
}

/// Case 4: every job through the worker (one rejected into the fallback),
/// then Ready: cover plus a raw tail ending exactly at F, the fold's ledger
/// closed three ways, the fallback block final and marked, and B's row ended
/// `ready` with the exit grace.
#[test]
fn workers_to_ready_with_fallback_and_fold() {
    let fx = Fixture::build();
    let done = fx.complete_catch_up();
    let text = &done.ready_text;
    assert!(
        text.starts_with(&format!("summary {} frontier #{HEAD} ", fx.thread)),
        "{text}"
    );
    // The cover: six level-0 blocks over the full chunks, in order, each
    // with the id the worker's submit returned.
    let block_lines: Vec<&str> = text.lines().filter(|l| l.starts_with("block L")).collect();
    assert_eq!(block_lines.len(), 6, "{text}");
    for (index, ((first, last), line)) in CHUNKS.iter().zip(&block_lines).enumerate() {
        assert!(
            line.starts_with(&format!("block L0 #{first}-#{last} {}", done.blocks[index])),
            "{line}"
        );
        assert_eq!(
            line.ends_with(" fallback"),
            index == FALLBACK_CHUNK,
            "{line}"
        );
    }
    // The fallback block renders its marker, not a narrative.
    assert!(
        text.contains(&format!(
            "{}{}",
            block_lines[FALLBACK_CHUNK], "\n  (fallback: no narrative)\n"
        )),
        "{text}"
    );
    assert_eq!(
        text.matches("(fallback: no narrative)").count(),
        1,
        "{text}"
    );
    // The fold closes: chunk 0's item by chunk 1's transition (item ids are
    // `<chunking_version>.<chunk>.<n>`), chunk 2's unclassified human input through
    // the fold, chunk 3's item through its same-chunk ref.
    let ledger = |kind: &str, status: &str, at: u64| {
        text.lines().any(|line| {
            let line = line.trim_start();
            line.starts_with(&format!("{kind} ")) && line.ends_with(&format!(" {status} at #{at}"))
        })
    };
    assert!(
        ledger("open_item", "resolved", 8),
        "cross-chunk close: {text}"
    );
    assert!(
        text.lines()
            .any(|line| line.trim_start().starts_with(&format!(
                "unclassified human input i.{PRIORITY_SEQ} done at #10 [agent relays-user]"
            ))),
        "same-chunk unclassified human input: {text}"
    );
    assert!(
        ledger("open_item", "resolved", 12),
        "same-chunk ref: {text}"
    );
    assert!(
        !text
            .lines()
            .any(|line| line.trim_start().starts_with("open_item") && line.contains(" open:")),
        "nothing is left open: {text}"
    );
    // The raw tail starts after the last full chunk and ends exactly at F.
    assert!(text.contains("tail #17-#17 (complete):"), "{text}");
    assert!(
        text.lines().any(|line| line.starts_with("#17 MSG ")),
        "the require-ACK message is the tail: {text}"
    );
    assert!(
        !text.contains("#16 MSG "),
        "covered messages are not repeated raw: {text}"
    );

    // The row ended `ready` and the exit grace was granted from that moment.
    let row = fx.world.catch_up(&fx.b, &fx.thread).expect("row");
    assert_eq!(
        (row.state.as_str(), row.end_reason.as_deref(), row.frontier),
        ("ended", Some("ready"), HEAD as i64),
        "{row:?}"
    );
    let (before, after) = done.ready_between;
    let ended = row.ended_at.unwrap() as u64;
    assert!(
        (before..=after).contains(&ended),
        "{row:?} not in {before}..={after}"
    );
    let extension = row.extension_until.unwrap() as u64;
    assert!(
        extension >= before + EXIT_GRACE_MS && extension <= after + EXIT_GRACE_MS,
        "extension_until is now + exit_grace: {row:?} (ready call {before}..={after})"
    );
}

/// Case 5: the held message is pushed once the row ends: B's tool-boundary
/// digest advances beyond the mark recorded before exit and offers it, and
/// the wake work re-derives, with no explicit `inbox` call.
#[test]
fn held_items_are_pushed_after_ready() {
    let fx = Fixture::build();
    let first = fx.boundary_b();
    assert!(first.contains(&fx.ack), "{first}");
    let work = fx.summary(fx.caller_b());
    let jobs = tickets(&work);
    let held = fx.send_as_a("an ordinary request", &["--require-ack", &fx.b]);
    assert!(!fx.boundary_b().contains(&held), "held while catching up");
    let wake_before = fx.world.wake_attention(&fx.b);
    fx.run_jobs(fx.caller_b(), &jobs);
    // Still held after every block is stored: only the exit releases it.
    assert!(!fx.boundary_b().contains(&held), "held until Ready");
    let ready = fx.summary(fx.caller_b());
    assert_eq!(ready["status"], "ready", "{ready}");
    assert_eq!(
        ready["data"]["frontier"], HEAD,
        "Ready stops at F, not at the head"
    );
    assert_eq!(
        ready["data"]["tail"].as_array().unwrap().len(),
        1,
        "the held message is not in Ready: {ready}"
    );
    let row = fx.world.catch_up(&fx.b, &fx.thread).unwrap();
    assert_eq!(row.end_reason.as_deref(), Some("ready"), "{row:?}");

    let pushed = fx.boundary_b();
    assert!(
        pushed.contains(&held),
        "the release is a pushed attention change: {pushed}"
    );
    assert!(
        fx.world.wake_attention(&fx.b) > wake_before,
        "wake reasons were re-derived at the row end"
    );
}

/// Case 6: a second seat gets Ready immediately, from the stored blocks.
#[test]
fn second_seat_reuses_blocks() {
    let fx = Fixture::build();
    let done = fx.complete_catch_up();
    let ready = fx.summary(fx.caller_c());
    assert_eq!(ready["status"], "ready", "{ready}");
    let data = &ready["data"];
    let cover: Vec<(&str, u64, u64)> = data["cover"]
        .as_array()
        .unwrap()
        .iter()
        .map(|block| {
            (
                block["block_id"].as_str().unwrap(),
                block["header"]["range"]["first_seq"].as_u64().unwrap(),
                block["header"]["range"]["last_seq"].as_u64().unwrap(),
            )
        })
        .collect();
    let expected: Vec<(&str, u64, u64)> = done
        .blocks
        .iter()
        .zip(CHUNKS)
        .map(|(id, (first, last))| (id.as_str(), first, last))
        .collect();
    assert_eq!(cover, expected, "the same block ids, no new jobs");
    assert_eq!(
        fx.world.catch_up_rows(&fx.c),
        0,
        "Ready without Work enters no row"
    );
    // The fallback block is final and says so in its header.
    let blocks = data["cover"].as_array().unwrap();
    assert_eq!(blocks[FALLBACK_CHUNK]["header"]["fallback"], true);
    assert_eq!(blocks[FALLBACK_CHUNK]["narrative"], "");
    assert!(
        blocks
            .iter()
            .enumerate()
            .all(|(i, b)| (i == FALLBACK_CHUNK) == b["header"]["fallback"].as_bool().unwrap())
    );
    // The fold shows the three closures with the sequence that closed them.
    let closed: Vec<(String, String, u64)> = data["fold"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry["item"]["id"].as_str().unwrap().to_owned(),
                entry["status"].as_str().unwrap().to_owned(),
                entry["closed_at_seq"].as_u64().unwrap_or(0),
            )
        })
        .collect();
    assert_eq!(closed.len(), 3, "{closed:?}");
    assert!(
        closed.contains(&(format!("i.{PRIORITY_SEQ}"), "done".into(), 10)),
        "{closed:?}"
    );
    assert_eq!(
        closed
            .iter()
            .filter(|(_, status, _)| status == "resolved")
            .map(|(_, _, at)| *at)
            .collect::<std::collections::BTreeSet<_>>(),
        [8, 12].into(),
        "{closed:?}"
    );
    // C's tail is the raw message after the last full chunk.
    let tail: Vec<u64> = data["tail"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["sequence"].as_u64().unwrap())
        .collect();
    assert_eq!(tail, [HEAD]);
    assert_eq!(data["tail_complete"], true);
}

/// Case 7: an unfetched reservation lapses after 60 s, and a fetch of the
/// lapsed job is honoured while the job is still free; once another seat has
/// leased it the fetch reports `reservation_lapsed`.
#[test]
fn lapsed_reservation_fetch_is_honoured_while_free() {
    let fx = Fixture::build();
    let work = fx.summary(fx.caller_b());
    let jobs = tickets(&work);
    let by_index = |index: usize| jobs.iter().find(|t| t.index == index).unwrap().clone();
    // Only the expired reservation state matters to this CLI/daemon case.
    // The real-store clock tests cover the 60 s transition. Backdate both
    // timestamps in this private fixture, retaining the reservation duration,
    // owner and token; every fetch and competing lease still uses the daemon.
    let mut db = rusqlite::Connection::open(fx.world.instance_dir.join("threads.sqlite3")).unwrap();
    db.busy_timeout(Duration::from_secs(5)).unwrap();
    // Take the writer lock before reading: a deferred read transaction cannot
    // upgrade while a daemon writer is active, even with the busy timeout.
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let expired_at = utc_ms() as i64 - 1;
    for ticket in &jobs {
        let (reserved_at, lease_until, fetched_at): (i64, i64, Option<i64>) = tx
            .query_row(
                "SELECT reserved_at,lease_until,fetched_at FROM summary_jobs WHERE id=?1",
                [&ticket.job],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(fetched_at, None, "the fixture must be unfetched");
        assert_eq!(lease_until - reserved_at, 60_000);
        let shift = lease_until - expired_at;
        assert!(shift > 0, "the initial reservation must still be live");
        assert_eq!(
            tx.execute(
                "UPDATE summary_jobs SET reserved_at=reserved_at-?1,lease_until=lease_until-?1 \
                 WHERE id=?2 AND fetched_at IS NULL",
                rusqlite::params![shift, ticket.job],
            )
            .unwrap(),
            1
        );
    }
    tx.commit().unwrap();
    drop(db);
    // Free and lapsed: honoured, and the lease starts now.
    let honoured = fx.fetch(fx.caller_b(), &by_index(0));
    assert_eq!(honoured["status"], "bundle", "{honoured}");
    assert_eq!(honoured["data"]["range"]["first_seq"], 1);
    // C polls: job 0 is B's live lease, the rest lapsed and are free, so C
    // leases them.
    let theirs = fx.summary(fx.caller_c());
    assert_eq!(theirs["status"], "work", "{theirs}");
    let c_jobs = tickets(&theirs);
    // A warning appended by the running daemon can fill another chunk.
    let indexes: Vec<usize> = c_jobs.iter().map(|t| t.index).collect();
    assert!(
        indexes.starts_with(&[1, 2, 3, 4, 5]) && !indexes.contains(&0),
        "C leases the free jobs, never B's live one: {indexes:?}"
    );
    let elsewhere: Vec<&Value> = theirs["data"]["leased_elsewhere"]
        .as_array()
        .unwrap()
        .iter()
        .collect();
    assert_eq!(elsewhere.len(), 1, "{theirs}");
    assert_eq!(elsewhere[0]["job_id"], by_index(0).job.as_str(), "{theirs}");
    // B's lapsed reservation for job 1 now belongs to C.
    let lost = fx.fetch(fx.caller_b(), &by_index(1));
    assert_eq!(lost["status"], "reservation_lapsed", "{lost}");
    assert_eq!(
        lost["data"]["leased_elsewhere"]["job_id"],
        by_index(1).job.as_str(),
        "{lost}"
    );
}

/// Case 8: a stalled run lapses. B gets Work and submits nothing; the
/// overdue warning for its receipt does not fire at the frozen deadline but
/// only once the extension has lapsed (the effective deadline), the row ends
/// `stalled`, and the held message is pushed.
#[test]
fn stall_lapses_and_the_warning_fires_on_the_effective_deadline() {
    let fx = Fixture::build();
    assert!(fx.boundary_b().contains(&fx.ack));
    assert_eq!(fx.summary(fx.caller_b())["status"], "work");
    let held = fx.send_as_a("an ordinary request", &["--require-ack", &fx.b]);
    assert!(!fx.boundary_b().contains(&held));
    let pending = fx.world.pending_for(&fx.b, &fx.ack);
    let frozen = pending["deadline"].as_u64().unwrap();
    let effective = pending["effective_deadline"].as_u64().unwrap();
    assert!(
        effective > frozen + 2_000,
        "the extension is the point: {pending}"
    );
    // The operator's overdue listing names the receipt (`overdue_receipt:
    // MESSAGE:SEAT`), the seat's warning page holds its warning.
    let warned = |fx: &Fixture| {
        fx.world
            .overdue_subjects()
            .contains(&format!("overdue_receipt:{}:{}", fx.ack, fx.b))
            && !fx.world.warnings(&fx.b).is_empty()
    };

    // Past the frozen deadline but inside the extension: no warning, not
    // overdue, the row still active.
    wait_until("the frozen deadline", Duration::from_secs(30), || {
        utc_ms() > frozen + 1_500
    });
    assert!(
        utc_ms() < effective - 1_000,
        "test timing: stay inside the extension"
    );
    assert!(!warned(&fx), "no warning at the frozen deadline");
    let inside = fx.world.pending_for(&fx.b, &fx.ack);
    assert_eq!(inside["overdue"], false, "{inside}");
    assert_eq!(
        fx.world.catch_up(&fx.b, &fx.thread).unwrap().state,
        "active",
        "still within the extension"
    );

    // No progress: the row stalls once the extension has passed.
    wait_until("the row to stall", Duration::from_secs(60), || {
        fx.world
            .catch_up(&fx.b, &fx.thread)
            .is_some_and(|row| row.state == "ended")
    });
    let row = fx.world.catch_up(&fx.b, &fx.thread).unwrap();
    assert_eq!(row.end_reason.as_deref(), Some("stalled"), "{row:?}");
    assert!(
        row.ended_at.unwrap() as u64 >= effective,
        "it lapses only after the extension: {row:?} vs {effective}"
    );
    assert_eq!(
        row.extension_until.unwrap() as u64,
        effective,
        "no progress, no extension"
    );
    // The release is a push.
    assert!(
        fx.boundary_b().contains(&held),
        "held items are pushed when the row stalls"
    );
    // The warning follows the effective deadline.
    wait_until("the overdue warning", Duration::from_secs(60), || {
        warned(&fx)
    });
    assert!(
        utc_ms() > effective,
        "the warning fired after the effective deadline"
    );
    assert_eq!(fx.world.pending_for(&fx.b, &fx.ack)["overdue"], true);
}

/// Case 9: a new binding ends the row `superseded`; the successor does not
/// inherit the hold, so what was held is offered.
#[test]
fn binding_change_supersedes_and_releases() {
    let fx = Fixture::build();
    assert!(fx.boundary_b().contains(&fx.ack));
    assert_eq!(fx.summary(fx.caller_b())["status"], "work");
    let held = fx.send_as_a("an ordinary request", &["--require-ack", &fx.b]);
    assert!(!fx.boundary_b().contains(&held));
    let before = fx.world.wake_attention(&fx.b);
    // A `clear` in B's pane is a new execution of the seat: the hook's
    // lifecycle check-in registers it and ends B's row.
    let restarted = fx.world.hook(
        "claude",
        PANE_B,
        &session_start("claude", SESSION_B, "clear"),
    );
    assert_eq!(restarted.code, 0, "{}", restarted.stderr);
    let row = fx.world.catch_up(&fx.b, &fx.thread).unwrap();
    assert_eq!(
        (row.state.as_str(), row.end_reason.as_deref()),
        ("ended", Some("superseded")),
        "{row:?}"
    );
    // The hook reads its digest before the check-in that ends the row, so the
    // successor's next boundary is where the release arrives.
    let successor = fx.boundary_b();
    assert!(
        successor.contains(&held),
        "the successor is offered what the predecessor had held: {successor}"
    );
    assert!(
        fx.world.wake_attention(&fx.b) > before,
        "the row end re-derived the seat's wake reasons"
    );
    // The successor inherits no hold: a new ordinary message above F is
    // offered at its next boundary.
    let fresh = fx.send_as_a("a message after the change", &[]);
    assert!(
        fx.boundary_b().contains(&fresh),
        "no hold survives the binding change"
    );
}

/// Case 10: a Codex SessionStart `compact` in the seat's pane names its hot
/// thread in the recovery text, with the fixed instruction outside the
/// escaped peer data. The seat is D, a Codex seat invited to T (a pending
/// invitation makes T hot); B is a Claude seat, whose compaction is not yet
/// an evidenced recovery event.
#[test]
fn codex_compact_recovery_names_the_hot_thread() {
    let fx = Fixture::build();
    let d = fx
        .world
        .cli(None, None, &["seat", "resolve", "--pane", PANE_D])
        .text("seat resolve");
    let started = fx.world.hook(
        "codex",
        PANE_D,
        &session_start("codex", "cx-sess", "startup"),
    );
    assert_eq!(started.code, 0, "{}", started.stderr);
    fx.world
        .cli(
            Some(fx.caller_a()),
            None,
            &["invite", &fx.thread, "--seat", &d],
        )
        .data("invite");
    let compact = fx.world.hook(
        "codex",
        PANE_D,
        r#"{"session_id":"cx-sess","turn_id":"t2","hook_event_name":"SessionStart","source":"compact"}"#,
    );
    assert_eq!(compact.code, 0, "{}", compact.stderr);
    let output: Value = serde_json::from_str(&compact.stdout).unwrap();
    assert_eq!(
        output["hookSpecificOutput"]["hookEventName"],
        "SessionStart"
    );
    let context = output["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    let (fixed, data) = context.split_once("\nuntrusted_peer_data: ").unwrap();
    assert!(
        fixed
            .lines()
            .any(|line| line == herdr_threads::harness::recovery_instruction()),
        "{context}"
    );
    let data: String = serde_json::from_str(data.lines().next().unwrap()).unwrap();
    assert!(data.contains(&fx.thread), "the hot thread is named: {data}");
    assert!(data.contains("summary flow"), "with its topic: {data}");
    assert!(
        !fixed.contains("summary flow"),
        "the peer-chosen topic stays out of the fixed text: {fixed}"
    );
    // Not a reset: the seat's ordinary tool boundary carries no recovery text.
    let tool = fx.world.hook(
        "codex",
        PANE_D,
        r#"{"session_id":"cx-sess","turn_id":"t3","hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"ls"},"tool_use_id":"call_1"}"#,
    );
    assert_eq!(tool.code, 0, "{}", tool.stderr);
    assert!(
        !tool.stdout.contains("Context was reset"),
        "{}",
        tool.stdout
    );
}

/// Case 11: programmatic rows map to `service`. A registered service client
/// posts a notification on a thread it manages, and the history through the
/// installed executable shows it as authored by role `service`, with none of
/// the `[human]` / `[relays user]` markers. (The migration's own backfill of
/// pre-existing programmatic rows is covered at the store level by
/// `v10_upgrade_backfills_author_role_from_the_covering_binding`; the
/// integration harness cannot hand a running daemon a pre-migration store.)
#[test]
fn programmatic_rows_map_to_service() {
    use herdr_threads::{
        app::SystemClock,
        client::service::{PersistentServiceClient, ServiceIntentJournal},
        daemon::ownership::{read_descriptor, read_existing_namespace},
        protocol::{
            ids::{OperationId, ThreadId},
            service::{
                EnsureManagedThread, InvitationConstraint, NotificationSeverity, ServiceInvite,
                ServiceNotify, ServiceOperation, ServiceResult,
            },
            time::{CallBudget, Cancellation, Clock, MonoInstant},
        },
    };
    use std::sync::Arc;

    let fx = Fixture::build();
    let context =
        RuntimeContext::explicit(fx.world.state.clone(), fx.world.host.clone(), None).unwrap();
    let paths = InstancePaths::resolve(&context).unwrap();
    let instance = read_existing_namespace(&paths).unwrap().expect("instance");
    let descriptor = read_descriptor(&paths, instance).unwrap();
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let client = PersistentServiceClient::new(
        descriptor.endpoint.clone(),
        Arc::clone(&clock),
        descriptor.instance_uuid,
        Some(descriptor.boot_id),
        ServiceIntentJournal::open(fx.world.root.join("intents")).unwrap(),
    );
    let budget = || CallBudget {
        deadline: MonoInstant(clock.monotonic_now().0 + 10_000),
        cancellation: Cancellation::default(),
    };
    let managed = ThreadId::new("thread-service-flow");
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            client.register(&budget()).await.unwrap();
            let (_, ensured) = client
                .submit(
                    ServiceOperation::EnsureThread(EnsureManagedThread {
                        thread: managed.clone(),
                        topic: "service flow".into(),
                        goal: "coordination".into(),
                        operation: OperationId::new("ensure"),
                    }),
                    &budget(),
                )
                .await
                .unwrap();
            assert!(
                matches!(ensured, ServiceResult::ThreadEnsured(_)),
                "{ensured:?}"
            );
            client
                .submit(
                    ServiceOperation::Invite(ServiceInvite {
                        thread: managed.clone(),
                        seat: herdr_threads::protocol::ids::SeatId::new(fx.a.clone()),
                        constraint: InvitationConstraint::Ordinary,
                        deadline_millis: Some(300_000),
                        operation: OperationId::new("invite"),
                    }),
                    &budget(),
                )
                .await
                .unwrap();
            client
                .submit(
                    ServiceOperation::Notify(ServiceNotify {
                        thread: managed.clone(),
                        severity: NotificationSeverity::Info,
                        event_json: json!({"kind": "integration", "detail": "service says hello"}),
                        operation: OperationId::new("notify"),
                    }),
                    &budget(),
                )
                .await
                .unwrap();
        });
    fx.world
        .cli(Some(fx.caller_a()), None, &["accept", managed.as_str()])
        .data("accept");
    let history = fx
        .world
        .cli(Some(fx.caller_a()), None, &["read", managed.as_str()])
        .data("read");
    let notices: Vec<&Value> = history["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["event_author"]["kind"] == "programmatic")
        .collect();
    // The service's invitation event and its notification: both programmatic.
    assert_eq!(notices.len(), 2, "{history}");
    for notice in &notices {
        assert_eq!(notice["author_role"], "service", "{notice}");
        assert!(
            notice["relays_user"].is_null(),
            "a service never relays a user: {notice}"
        );
    }
    assert!(
        notices.iter().any(|notice| {
            herdr_threads::protocol::ids::is_short_public_id(
                herdr_threads::protocol::ids::prefix::NOTIFY,
                notice["message"].as_str().unwrap(),
            )
        }),
        "{history}"
    );
    // Native sends on the same thread keep their own role.
    let sent = fx
        .world
        .cli(
            Some(fx.caller_a()),
            None,
            &["send", managed.as_str(), "--body", "from an agent"],
        )
        .text("send");
    let after = fx
        .world
        .cli(Some(fx.caller_a()), None, &["read", managed.as_str()])
        .data("read");
    let sent_row = after["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["message"] == sent.as_str())
        .unwrap();
    assert_eq!(sent_row["author_role"], "agent", "{sent_row}");
    // The human history marks neither.
    let human = fx
        .world
        .human(Some(fx.caller_a()), &["read", managed.as_str()]);
    assert_eq!(human.code, 0, "{}", human.stderr);
    assert!(
        !human.stdout.contains("[human]") && !human.stdout.contains("[relays user]"),
        "{}",
        human.stdout
    );
}

#[path = "summary_sweep.rs"]
mod summary_sweep;

impl Fixture {
    /// One private daemon for the whole configuration matrix. C declares its
    /// human binding through the real pane-context path; D stays unbound.
    fn intent_configuration() -> Self {
        let world = World::start(vec![
            claude(PANE_A, "term-a", "SA"),
            claude(PANE_B, "term-b", SESSION_B),
            pane(PANE_C, "term-c"),
            pane(PANE_D, "term-d"),
        ]);
        let resolve = |pane: &str| {
            world
                .cli(None, None, &["seat", "resolve", "--pane", pane])
                .text("seat resolve")
        };
        let (a, b, c) = (resolve(PANE_A), resolve(PANE_B), resolve(PANE_C));
        for (pane, session) in [(PANE_A, "SA"), (PANE_B, SESSION_B)] {
            let started = world.hook("claude", pane, &session_start("claude", session, "startup"));
            assert_eq!(started.code, 0, "{}", started.stderr);
        }
        world.person(PANE_C, &["me", "init"]).data("human me init");
        let author = Caller {
            seat: &a,
            pane: PANE_A,
        };
        let thread = world
            .cli(
                Some(author),
                None,
                &["thread", "create", "--topic", "intent configuration"],
            )
            .text("thread create");
        for seat in [&b, &c] {
            world
                .cli(Some(author), None, &["invite", &thread, "--seat", seat])
                .data("invite");
        }
        world
            .cli(
                Some(Caller {
                    seat: &b,
                    pane: PANE_B,
                }),
                None,
                &["accept", &thread],
            )
            .data("agent accept");
        world
            .person(PANE_C, &["accept", &thread])
            .data("human accept");
        Self {
            world,
            a,
            b,
            c,
            thread,
            ack: String::new(),
        }
    }
    fn intent_send(&self, human: bool, relay: bool, intent: Option<&str>, text: &str) -> String {
        let mut args = vec![
            "send",
            &self.thread,
            "--body",
            text,
            "--require-ack",
            &self.b,
        ];
        if relay {
            args.push("--relays-user");
        }
        if let Some(intent) = intent {
            args.extend(["--user-intent", intent]);
        }
        if human {
            self.world.person(PANE_C, &args)
        } else {
            self.world.cli(Some(self.caller_a()), None, &args)
        }
        .text("configured send")
    }
    fn sequence(&self, message: &str) -> u64 {
        self.world
            .db()
            .query_row(
                "SELECT sequence FROM messages WHERE id=?1",
                [message],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
            .try_into()
            .unwrap()
    }
    fn assert_pending_intent_receipt(&self, message: &str) {
        self.world.pending_for(&self.b, message);
        assert_pending_intent_receipt_in(&self.world.db(), &self.b, message);
    }
}

fn assert_pending_intent_receipt_in(db: &rusqlite::Connection, seat: &str, message: &str) {
    use herdr_threads::store::effective::{EffectiveReceiptState, effective_receipt};
    // Canonical pending receipts include published manifests whose physical
    // receipt_state projection has not run yet. One read transaction keeps all
    // component lookups on the same view of the receipt and its provenance.
    let tx = db.unchecked_transaction().unwrap();
    let receipt = effective_receipt(&tx, message, seat)
        .unwrap()
        .expect("addressed receipt");
    assert_eq!(receipt.state, EffectiveReceiptState::Pending);
    assert_eq!(receipt.ack_actor_seat_id, None);
    assert_eq!(receipt.ack_observation, None);
    assert_eq!(receipt.acked_at, None);
    tx.commit().unwrap();
}

#[test]
fn user_intent_pending_receipt_does_not_require_materialization() {
    let fx = Fixture::intent_configuration();
    let message = fx.intent_send(false, true, Some("request"), "unmaterialized request");
    fx.world.pending_for(&fx.b, &message);
    // An offline snapshot freezes the lifecycle; removing only its optional
    // projection makes the pre-materialization boundary deterministic.
    let snapshot = fx.world.root.join("receipt-snapshot.sqlite3");
    fx.world
        .db()
        .execute("VACUUM INTO ?1", [snapshot.to_str().unwrap()])
        .unwrap();
    let db = rusqlite::Connection::open(&snapshot).unwrap();
    db.execute("DELETE FROM receipt_state WHERE message_id=?1", [&message])
        .unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM receipt_state WHERE message_id=?1",
            [&message],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_pending_intent_receipt_in(&db, &fx.b, &message);
}

fn assert_intent_claim(row: &Value, human: bool, relay: bool, intent: Option<&str>) {
    assert_eq!(
        row["author_role"],
        if human { "human" } else { "agent" },
        "{row}"
    );
    assert_eq!(
        row.get("relays_user")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        relay,
        "{row}"
    );
    assert_eq!(
        row.get("user_intent"),
        intent.map(|value| json!(value)).as_ref(),
        "{row}"
    );
}

#[test]
fn user_intent_configuration_smoke() {
    let fx = Fixture::intent_configuration();
    let mut sent = Vec::new();
    for (human, relay) in [(true, false), (true, true), (false, true)] {
        for intent in [Some("query"), Some("request"), Some("rule"), None] {
            let text = format!(
                "configuration-{human}-{relay}-{}",
                intent.unwrap_or("legacy")
            );
            let message = fx.intent_send(human, relay, intent, &text);
            sent.push((message, text, human, relay, intent));
        }
    }
    let history = fx
        .world
        .cli(
            Some(fx.caller_b()),
            None,
            &["read", &fx.thread, "--limit", "100"],
        )
        .data("read");
    let aggregate = fx
        .world
        .cli(Some(fx.caller_b()), None, &["inbox", "--limit", "100"])
        .data("inbox");
    assert_eq!(
        aggregate["items"].as_array().unwrap().len(),
        1,
        "{aggregate}"
    );
    assert_eq!(aggregate["items"][0]["thread"], fx.thread);
    assert_eq!(aggregate["items"][0]["pending_receipts"], sent.len());
    let inbox = fx.world.inbox_batch_json(&fx.b);
    for (message, text, human, relay, intent) in &sent {
        let find = |page: &Value| {
            page["items"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["message"] == *message)
                .unwrap_or_else(|| panic!("missing {message} in {page}"))
                .clone()
        };
        let row = find(&history);
        assert_intent_claim(&row, *human, *relay, *intent);
        assert_eq!(row["preview_data"], *text);
        let row = find(&inbox);
        assert_intent_claim(&row, *human, *relay, *intent);
        assert_eq!(row["body"], *text);
        let body = fx.world.cli(None, None, &["body", message]).data("body");
        assert_intent_claim(&body["summary"], *human, *relay, *intent);
        assert_eq!(body["content"]["body_data"], *text, "{body}");
        // This checks stored projections, not search latency. The bounded
        // read may honestly exhaust its budget under parallel CI load; retry
        // only that read-only outcome, with a finite fixture deadline.
        let deadline = Instant::now() + Duration::from_secs(3);
        let search = loop {
            let out = fx
                .world
                .cli(None, None, &["search", text, "--thread", &fx.thread]);
            if out.code != 0
                && out.stderr.contains("(read_budget_exhausted)")
                && Instant::now() < deadline
            {
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
            break out.data("search");
        };
        let matches = search["matches"]["items"].as_array().unwrap();
        assert_eq!(matches.len(), 1, "{search}");
        assert_eq!(matches[0]["data"]["message"], *message);
        assert_intent_claim(&matches[0]["data"], *human, *relay, *intent);
    }
    // JSON inbox is read-only: every real receipt remains pending and no ACK
    // actor or provenance is manufactured by displaying any classification.
    for (message, ..) in &sent {
        fx.assert_pending_intent_receipt(message);
    }
    let unbound = fx
        .world
        .cli(None, None, &["seat", "resolve", "--pane", PANE_D])
        .text("unbound seat");
    let count = |sql: &str| -> i64 { fx.world.db().query_row(sql, [], |r| r.get(0)).unwrap() };
    let published = count("SELECT count(*) FROM messages WHERE kind='ordinary'");
    let operations = count("SELECT count(*) FROM operations");
    let manifests = count("SELECT count(*) FROM send_manifests");
    for (name, caller, relay) in [
        ("unrelayed-agent", fx.caller_a(), false),
        (
            "unbound",
            Caller {
                seat: &unbound,
                pane: PANE_D,
            },
            true,
        ),
        (
            "mismatched",
            Caller {
                seat: &fx.a,
                pane: PANE_B,
            },
            true,
        ),
    ] {
        let mut args = vec![
            "send",
            &fx.thread,
            "--body",
            name,
            "--user-intent",
            "query",
            "--require-ack",
            &fx.b,
        ];
        if relay {
            args.push("--relays-user");
        }
        let refused = fx.world.cli(Some(caller), None, &args);
        assert_ne!(refused.code, 0, "{name}: {}", refused.stdout);
        assert_eq!(
            count("SELECT count(*) FROM messages WHERE kind='ordinary'"),
            published
        );
        assert_eq!(count("SELECT count(*) FROM operations"), operations);
        assert_eq!(count("SELECT count(*) FROM send_manifests"), manifests);
        assert_eq!(count("SELECT count(*) FROM summary_items"), 0);
    }
    assert_eq!(
        count(
            "SELECT count(*) FROM (SELECT acked_at,ack_actor_seat_id,ack_observation FROM receipts UNION ALL SELECT acked_at,ack_actor_seat_id,ack_observation FROM receipt_state) WHERE acked_at IS NOT NULL OR ack_actor_seat_id IS NOT NULL OR ack_observation IS NOT NULL"
        ),
        0
    );
    assert_eq!(
        count("SELECT count(*) FROM occupant_bindings WHERE harness='human'"),
        1
    );
    let after = fx
        .world
        .cli(
            Some(fx.caller_b()),
            None,
            &["read", &fx.thread, "--limit", "100"],
        )
        .data("refused history");
    assert_eq!(
        after["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["kind"] == "ordinary")
            .count(),
        sent.len()
    );
    // Unknown canonical roles and service/event attempts are exercised by
    // user_intent_schema22_checks_and_eligibility and the service-send test;
    // the public service protocol deliberately has no intent send option.
}

fn intent_submission(transitions: Value) -> Value {
    json!({"submission_schema": herdr_threads::protocol::summary::SUBMISSION_SCHEMA,
        "narrative": "Scripted configuration lifetime evidence.", "prompt_version": "configuration-v2",
        "model": "scripted", "transitions": transitions})
}

fn intent_fold_entry(fold: &Value, sequence: u64) -> &Value {
    let id = format!("i.{sequence}");
    let entries = fold["entries"].as_array().unwrap();
    let matches: Vec<_> = entries
        .iter()
        .filter(|entry| entry["item"]["id"] == id)
        .collect();
    assert_eq!(matches.len(), 1, "one stable source {id}: {fold}");
    matches[0]
}

#[test]
fn user_intent_configuration_summary_lifetimes() {
    let fx = Fixture::intent_configuration();
    let mut sources = Vec::new();
    for (human, relay, intent, text) in [
        (true, false, Some("query"), "What is our progress?"),
        (false, true, Some("request"), "Cut a build."),
        (false, true, Some("rule"), "Always test before releasing."),
        (true, false, None, "Legacy human task."),
        (false, true, None, "Legacy forwarded task."),
    ] {
        let message = fx.intent_send(
            human,
            relay,
            intent,
            &format!("{text} {}", body(sources.len())),
        );
        sources.push((fx.sequence(&message), message, human, relay, intent));
    }
    let completion = fx.intent_send(
        false,
        false,
        None,
        &format!(
            "Progress answered, build cut, tests passed before release. {}",
            body(6)
        ),
    );
    let completion_seq = fx.sequence(&completion);
    let incidental_quote = fx.intent_send(
        false,
        false,
        None,
        &format!(
            "Discussing the quote: ‘Always test before releasing.’ {}",
            body(7)
        ),
    );
    let quote_seq = fx.sequence(&incidental_quote);
    fx.intent_send(false, false, None, "raw tail");
    let history_before = query_history(&fx);
    for (_, message, human, relay, intent) in &sources {
        let row = history_before
            .iter()
            .find(|row| row["message"] == message.as_str())
            .unwrap();
        assert_eq!(
            row["author"],
            if *human { fx.c.as_str() } else { fx.a.as_str() }
        );
        assert_intent_claim(row, *human, *relay, *intent);
        fx.assert_pending_intent_receipt(message);
    }
    let quoted_row = history_before
        .iter()
        .find(|row| row["message"] == incidental_quote.as_str())
        .unwrap();
    assert_intent_claim(quoted_row, false, false, None);
    assert_eq!(quoted_row["author"], fx.a);
    let work = fx.summary(fx.caller_b());
    let jobs = tickets(&work);
    let b = jobs
        .iter()
        .find(|ticket| ticket.first <= completion_seq && completion_seq <= ticket.last)
        .expect("completion is in a full chunk");
    let a = jobs
        .iter()
        .find(|ticket| ticket.first <= sources[0].0 && sources[0].0 <= ticket.last)
        .unwrap();
    assert!(
        a.last < b.first,
        "sources and completion must cross chunks: {jobs:?}"
    );
    let bundle_b = fx.fetch(fx.caller_b(), b);
    assert_eq!(bundle_b["status"], "bundle", "{bundle_b}");
    for (sequence, message, human, relay, intent) in &sources {
        let entry = intent_fold_entry(&bundle_b["data"]["fold"], *sequence);
        assert_eq!(
            entry["status"],
            if *intent == Some("rule") {
                "active"
            } else {
                "open"
            }
        );
        assert_eq!(entry["item"]["body"]["message_id"], *message);
        assert_intent_claim(&entry["item"]["body"], *human, *relay, *intent);
    }
    assert_eq!(
        fx.world
            .db()
            .query_row("SELECT count(*) FROM summary_blocks", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0,
        "B sees sources before A stores"
    );
    // The same actual CLI fetch is frozen in the canonical job row, including
    // the earlier unstored sources, rather than reconstructed from timestamps.
    let frozen: String = fx
        .world
        .db()
        .query_row(
            "SELECT fetched_bundle_json FROM summary_jobs WHERE id=?1",
            [&b.job],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&frozen).unwrap(),
        bundle_b["data"]
    );
    assert_eq!(fx.fetch(fx.caller_b(), b), bundle_b);
    let closing = json!([
        {"target":format!("i.{}", sources[0].0), "new_status":"resolved", "cite_seq":completion_seq},
        {"target":format!("i.{}", sources[1].0), "new_status":"resolved", "cite_seq":completion_seq},
        {"target":format!("i.{}", sources[3].0), "new_status":"done", "cite_seq":completion_seq}
    ]);
    let (code, stored) = fx.submit(fx.caller_b(), b, &intent_submission(closing));
    assert_eq!((code, &stored["status"]), (0, &json!("stored")), "{stored}");
    assert_eq!(stored["data"]["fallback"], false);
    // A fetches only after B stores, yet its earlier frontier keeps work open.
    let bundle_a = fx.fetch(fx.caller_b(), a);
    assert_eq!(
        intent_fold_entry(&bundle_a["data"]["fold"], sources[0].0)["status"],
        "open"
    );
    for ticket in jobs.iter().filter(|ticket| ticket.job != b.job) {
        fx.fetch(fx.caller_b(), ticket);
        if ticket.job == a.job {
            // A's deterministic Query survives fallback while B's earlier
            // stored resolution still wins in the final sequence-ordered fold.
            let mut invalid = intent_submission(json!([]));
            invalid["submission_schema"] = json!(1);
            let (code, rejected) = fx.submit(fx.caller_b(), ticket, &invalid);
            assert_eq!(
                (code, &rejected["status"]),
                (1, &json!("rejected")),
                "{rejected}"
            );
            invalid["narrative"] = json!("Second distinct schema1 rejection.");
            let (code, stored) = fx.submit(fx.caller_b(), ticket, &invalid);
            assert_eq!((code, &stored["status"]), (0, &json!("stored")), "{stored}");
            assert_eq!(stored["data"]["fallback"], true);
            let prompt: String = fx
                .world
                .db()
                .query_row(
                    "SELECT prompt_version FROM summary_blocks WHERE id=?1",
                    [stored["data"]["block_id"].as_str().unwrap()],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(prompt, "daemon-fallback-v2");
        } else {
            let (code, stored) = fx.submit(fx.caller_b(), ticket, &intent_submission(json!([])));
            assert_eq!((code, &stored["status"]), (0, &json!("stored")), "{stored}");
            assert_eq!(stored["data"]["fallback"], false);
        }
    }
    let ready = fx.summary(fx.caller_b());
    assert_eq!(ready["status"], "ready", "{ready}");
    assert!(
        !ready["data"]["fold"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["item"]["seq"] == quote_seq),
        "ordinary quotations must not invent human work: {ready}"
    );
    for (sequence, message, human, relay, intent) in &sources {
        let entry = intent_fold_entry(&ready["data"]["fold"], *sequence);
        assert_eq!(
            entry["item"]["body"]["author_seat"],
            if *human { fx.c.as_str() } else { fx.a.as_str() }
        );
        assert_eq!(entry["item"]["body"]["message_id"], message.as_str());
        assert_intent_claim(&entry["item"]["body"], *human, *relay, *intent);
    }
    for index in [0, 1, 3] {
        let entry = intent_fold_entry(&ready["data"]["fold"], sources[index].0);
        assert_eq!(
            entry["status"],
            if index == 3 { "done" } else { "resolved" }
        );
        assert_eq!(entry["closed_at_seq"], completion_seq);
        assert_eq!(entry["item"]["body"]["message_id"], sources[index].1);
    }
    let rule = intent_fold_entry(&ready["data"]["fold"], sources[2].0);
    assert_eq!(
        rule["status"], "active",
        "compliance must leave the rule active"
    );
    assert!(rule["closed_at_seq"].is_null());
    assert_eq!(
        intent_fold_entry(&ready["data"]["fold"], sources[4].0)["status"],
        "open"
    );
    let displayed = fx
        .world
        .human(Some(fx.caller_b()), &["summary", &fx.thread]);
    assert_eq!(displayed.code, 0, "{}", displayed.stderr);
    for marker in [
        "[query]",
        "[request]",
        "[agent relays-user] [rule]",
        "unclassified human input",
    ] {
        assert!(
            displayed.stdout.contains(marker),
            "{marker}: {}",
            displayed.stdout
        );
    }
    for (index, kind, status) in [
        (0, "question", "resolved"),
        (1, "ask", "resolved"),
        (2, "rule", "active"),
        (3, "unclassified human input", "done"),
        (4, "unclassified human input", "open"),
    ] {
        let line = format!("{kind} i.{} {status}", sources[index].0);
        assert!(
            displayed.stdout.contains(&line),
            "{line}: {}",
            displayed.stdout
        );
    }
    let active_rule_line = format!(
        "rule i.{} active [agent relays-user] [rule] {}: Always test before releasing.",
        sources[2].0, fx.a
    );
    assert!(
        displayed.stdout.contains(&active_rule_line),
        "{active_rule_line}: {}",
        displayed.stdout
    );
    let quote = "Withdraw the testing rule and replace the legacy forwarded task.";
    let withdrawal = fx.intent_send(false, true, None, &format!("{quote} {}", body(8)));
    let withdrawal_seq = fx.sequence(&withdrawal);
    fx.intent_send(false, false, None, &body(9));
    fx.intent_send(false, false, None, "next raw tail");
    let work = fx.summary(fx.caller_b());
    let jobs = tickets(&work);
    for ticket in &jobs {
        fx.fetch(fx.caller_b(), ticket);
        let transitions = if ticket.first <= withdrawal_seq && withdrawal_seq <= ticket.last {
            json!([
                {"target":format!("i.{}", sources[2].0), "new_status":"superseded", "cite_seq":withdrawal_seq, "rule_change":"withdrawn", "quote":quote},
                {"target":format!("i.{}", sources[4].0), "new_status":"superseded", "cite_seq":withdrawal_seq}
            ])
        } else {
            json!([])
        };
        let (code, stored) = fx.submit(fx.caller_b(), ticket, &intent_submission(transitions));
        assert_eq!((code, &stored["status"]), (0, &json!("stored")), "{stored}");
        assert_eq!(stored["data"]["fallback"], false);
    }
    let ready = fx.summary(fx.caller_b());
    assert_eq!(ready["status"], "ready", "{ready}");
    for index in [2, 4] {
        let entry = intent_fold_entry(&ready["data"]["fold"], sources[index].0);
        assert_eq!(entry["status"], "superseded");
        assert_eq!(entry["closed_at_seq"], withdrawal_seq);
        assert_eq!(entry["item"]["body"]["message_id"], sources[index].1);
    }
    // Summary closure is independent of receipt settlement, including Query
    // and Request resolution and explicit Rule withdrawal.
    for (_, message, ..) in &sources {
        fx.assert_pending_intent_receipt(message);
    }
    let history_after = query_history(&fx);
    for original in &history_before {
        let after = history_after
            .iter()
            .find(|row| row["message"] == original["message"]);
        assert_eq!(
            after,
            Some(original),
            "original source must remain unchanged"
        );
    }
    let displayed = fx
        .world
        .human(Some(fx.caller_b()), &["summary", &fx.thread]);
    assert_eq!(displayed.code, 0, "{}", displayed.stderr);
    let rule_line = format!(
        "rule i.{} superseded at #{withdrawal_seq} [agent relays-user] [rule] {}",
        sources[2].0, fx.a
    );
    assert!(
        displayed.stdout.contains(&rule_line),
        "{rule_line}: {}",
        displayed.stdout
    );
    assert_eq!(fx.world.db().query_row("SELECT count(*) FROM (SELECT acked_at,ack_actor_seat_id,ack_observation FROM receipts UNION ALL SELECT acked_at,ack_actor_seat_id,ack_observation FROM receipt_state) WHERE acked_at IS NOT NULL OR ack_actor_seat_id IS NOT NULL OR ack_observation IS NOT NULL", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
}

// ---- query withdrawal/replacement: scripted semantic judgments ----

fn query_history(fx: &Fixture) -> Vec<Value> {
    fx.world
        .cli(
            Some(fx.caller_b()),
            None,
            &["read", &fx.thread, "--limit", "100"],
        )
        .data("query source history")["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["kind"] == "ordinary")
        .cloned()
        .collect()
}

fn query_later_bundle(fx: &Fixture, jobs: &[Ticket], source: u64, later: u64) -> Value {
    let containing = |sequence| {
        jobs.iter()
            .find(|job| job.first <= sequence && sequence <= job.last)
            .expect("query and later evidence belong to full chunks")
    };
    let (a, b) = (containing(source), containing(later));
    assert!(a.last < b.first, "A must precede B: {jobs:?}");
    assert_eq!(
        fx.world
            .db()
            .query_row("SELECT count(*) FROM summary_blocks", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0,
        "fetch B before A or any other chunk stores"
    );
    let bundle = fx.fetch(fx.caller_b(), b);
    assert_eq!(bundle["status"], "bundle", "{bundle}");
    assert_eq!(
        intent_fold_entry(&bundle["data"]["fold"], source)["status"],
        "open"
    );
    assert!(
        bundle["data"]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["sequence"] == later),
        "later evidence must be in B's current chunk: {bundle}"
    );
    bundle
}

fn query_finish_jobs(fx: &Fixture, jobs: &[Ticket], resolutions: &[(u64, u64)]) -> Value {
    let mut ordered = jobs.to_vec();
    ordered.sort_by_key(|job| std::cmp::Reverse(job.first));
    for job in &ordered {
        fx.fetch(fx.caller_b(), job);
        let transitions: Vec<_> = resolutions.iter()
            .filter(|(_, cite)| job.first <= *cite && *cite <= job.last)
            .map(|(source, cite)| json!({"target":format!("i.{source}"), "new_status":"resolved", "cite_seq":cite}))
            .collect();
        // B submits first, then A; the worker's explicit script decides closure.
        let (code, stored) = fx.submit(fx.caller_b(), job, &intent_submission(json!(transitions)));
        assert_eq!((code, &stored["status"]), (0, &json!("stored")), "{stored}");
        assert_eq!(stored["data"]["fallback"], false);
    }
    let ready = fx.summary(fx.caller_b());
    assert_eq!(ready["status"], "ready", "{ready}");
    ready
}

#[test]
fn user_intent_query_withdrawal_resolves_original() {
    let fx = Fixture::intent_configuration();
    let mut originals = Vec::new();
    // The same withdrawal script runs for actual Human and relaying Agent input.
    for (human, relay) in [(true, false), (false, true)] {
        let text = format!(
            "What is progress for {human}-{relay}? {}",
            body(originals.len())
        );
        let message = fx.intent_send(human, relay, Some("query"), &text);
        originals.push((message, text, human, relay));
    }
    for index in 2..5 {
        fx.intent_send(false, false, None, &body(index));
    }
    let withdrawals: Vec<_> = originals
        .iter()
        .map(|(_, _, human, relay)| {
            fx.intent_send(
                *human,
                *relay,
                None,
                &format!("Never mind that question for {human}-{relay}. {}", body(5)),
            )
        })
        .collect();
    fx.intent_send(false, false, None, &body(6));
    fx.intent_send(false, false, None, &body(7));
    fx.intent_send(false, false, None, "raw tail");
    let before = query_history(&fx);
    let jobs = tickets(&fx.summary(fx.caller_b()));
    let mut resolutions = Vec::new();
    for ((message, text, human, relay), withdrawal) in originals.iter().zip(&withdrawals) {
        let (source, cite) = (fx.sequence(message), fx.sequence(withdrawal));
        let bundle = query_later_bundle(&fx, &jobs, source, cite);
        let citing = bundle["data"]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|message| message["sequence"] == cite)
            .unwrap();
        assert_intent_claim(citing, *human, *relay, None);
        let entry = intent_fold_entry(&bundle["data"]["fold"], source);
        assert_eq!(entry["item"]["body"]["message_id"], *message);
        assert_eq!(entry["item"]["body"]["text"], *text);
        assert_intent_claim(&entry["item"]["body"], *human, *relay, Some("query"));
        resolutions.push((source, cite));
    }
    let ready = query_finish_jobs(&fx, &jobs, &resolutions);
    for ((message, _, human, relay), (source, cite)) in originals.iter().zip(&resolutions) {
        let entry = intent_fold_entry(&ready["data"]["fold"], *source);
        assert_eq!(entry["status"], "resolved");
        assert_eq!(entry["closed_at_seq"], *cite);
        assert_eq!(entry["item"]["body"]["message_id"], *message);
        assert_intent_claim(&entry["item"]["body"], *human, *relay, Some("query"));
    }
    assert_eq!(
        query_history(&fx),
        before,
        "summary closure preserves every source claim"
    );
    for (message, ..) in &originals {
        fx.assert_pending_intent_receipt(message);
    }
    for message in &withdrawals {
        fx.assert_pending_intent_receipt(message);
    }
}

#[test]
fn user_intent_query_replacement_has_separate_source() {
    let fx = Fixture::intent_configuration();
    let old_text = format!("What is our progress? {}", body(0));
    let old = fx.intent_send(true, false, Some("query"), &old_text);
    for index in 1..4 {
        fx.intent_send(false, false, None, &body(index));
    }
    let replacement_text = format!(
        "Instead of that progress question, which release should we ship? {}",
        body(4)
    );
    let replacement = fx.intent_send(false, true, Some("query"), &replacement_text);
    fx.intent_send(false, false, None, &body(5));
    fx.intent_send(false, false, None, &body(6));
    fx.intent_send(false, false, None, "raw tail");
    let before = query_history(&fx);
    let (old_seq, replacement_seq) = (fx.sequence(&old), fx.sequence(&replacement));
    let jobs = tickets(&fx.summary(fx.caller_b()));
    let bundle = query_later_bundle(&fx, &jobs, old_seq, replacement_seq);
    for sequence in [old_seq, replacement_seq] {
        assert_eq!(
            intent_fold_entry(&bundle["data"]["fold"], sequence)["status"],
            "open"
        );
    }
    let ready = query_finish_jobs(&fx, &jobs, &[(old_seq, replacement_seq)]);
    let fold = &ready["data"]["fold"];
    for (sequence, message, text, human, relay, status) in [
        (old_seq, &old, &old_text, true, false, "resolved"),
        (
            replacement_seq,
            &replacement,
            &replacement_text,
            false,
            true,
            "open",
        ),
    ] {
        let entry = intent_fold_entry(fold, sequence);
        assert_eq!(entry["status"], status);
        assert_eq!(entry["item"]["seq"], sequence);
        assert_eq!(entry["item"]["body"]["message_id"], *message);
        assert_eq!(entry["item"]["body"]["text"], *text);
        assert_eq!(
            entry["item"]["body"]["author_seat"],
            if human { fx.c.as_str() } else { fx.a.as_str() }
        );
        assert_intent_claim(&entry["item"]["body"], human, relay, Some("query"));
    }
    assert_eq!(
        intent_fold_entry(fold, old_seq)["closed_at_seq"],
        replacement_seq
    );
    assert!(intent_fold_entry(fold, replacement_seq)["closed_at_seq"].is_null());
    assert_eq!(
        fold["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["item"]["body"]["user_intent"] == "query")
            .count(),
        2
    );
    assert_eq!(
        query_history(&fx),
        before,
        "replacement preserves independent sources"
    );
    fx.assert_pending_intent_receipt(&old);
    fx.assert_pending_intent_receipt(&replacement);
}

#[test]
fn user_intent_query_uncertain_or_agent_cancellation_stays_open() {
    let fx = Fixture::intent_configuration();
    let mut originals = Vec::new();
    for (index, question) in [
        "Unanswered: what is our progress?",
        "Partial: what passed and what failed?",
        "Uncertain: is the release safe?",
        "Cancellation: which release should we ship?",
    ]
    .iter()
    .enumerate()
    {
        let text = format!("{question} {}", body(index));
        let message = fx.intent_send(true, false, Some("query"), &text);
        originals.push((message, text));
    }
    fx.intent_send(false, false, None, &body(4));
    let mut evidence = Vec::new();
    for text in [
        "Partial answer: tests passed; I have not checked failures.",
        "Uncertain answer: perhaps the release is safe, but I am unsure.",
        "Ignore the question about which release we should ship.",
    ] {
        evidence.push(fx.intent_send(false, false, None, &format!("{text} {}", body(5))));
    }
    fx.intent_send(false, false, None, &body(6));
    fx.intent_send(false, false, None, &body(7));
    fx.intent_send(false, false, None, "raw tail");
    let before = query_history(&fx);
    let jobs = tickets(&fx.summary(fx.caller_b()));
    for ((message, _), later) in originals[1..].iter().zip(&evidence) {
        let later_seq = fx.sequence(later);
        let bundle = query_later_bundle(&fx, &jobs, fx.sequence(message), later_seq);
        let citing = bundle["data"]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|message| message["sequence"] == later_seq)
            .unwrap();
        assert_intent_claim(citing, false, false, None);
    }
    // The cooperative worker omits closures: the daemon does not classify prose
    // as an answer versus an agent's unilateral cancellation. Ordinary agent
    // answers remain valid, covered by user_intent_crosschunk_query_request_b_before_a.
    let ready = query_finish_jobs(&fx, &jobs, &[]);
    for (message, text) in &originals {
        let entry = intent_fold_entry(&ready["data"]["fold"], fx.sequence(message));
        assert_eq!(entry["status"], "open");
        assert!(entry["closed_at_seq"].is_null());
        assert_eq!(entry["item"]["body"]["message_id"], *message);
        assert_eq!(entry["item"]["body"]["text"], *text);
        assert_intent_claim(&entry["item"]["body"], true, false, Some("query"));
        fx.assert_pending_intent_receipt(message);
    }
    assert_eq!(
        query_history(&fx),
        before,
        "noncompletion preserves source claims"
    );
}

#[test]
fn user_intent_snapshot_worker_retries_exact_bundle() {
    let fx = Fixture::build();
    let jobs = tickets(&fx.summary(fx.caller_b()));
    // Separate worker invocations under the seat's lease, as real summary workers run.
    let first = fx.fetch(fx.caller_b(), &jobs[1]);
    let first_bytes = serde_json::to_string(&first["data"]).unwrap();
    let earlier = fx.fetch(fx.caller_b(), &jobs[0]);
    let submission = fx.good_submission(&jobs[0], &earlier);
    let (code, stored) = fx.submit(fx.caller_b(), &jobs[0], &submission);
    assert_eq!(code, 0, "{stored}");
    let repeat = fx.fetch(fx.caller_b(), &jobs[1]);
    assert_eq!(serde_json::to_string(&repeat["data"]).unwrap(), first_bytes);
    // This worker never saw the earlier worker's newly invented open item.
    assert!(
        first["data"]["fold"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["item"]["body"]["type"] != "open_item")
    );
    let submission = json!({"submission_schema":herdr_threads::protocol::summary::SUBMISSION_SCHEMA,"narrative":format!("Messages {}-{}",jobs[1].first,jobs[1].last),"model":"scripted","prompt_version":"snapshot-test"});
    let (code, stored) = fx.submit(fx.caller_b(), &jobs[1], &submission);
    assert_eq!(code, 0, "{stored}");
    assert_eq!(stored["data"]["fallback"], false);
    fx.run_jobs(fx.caller_b(), &jobs[2..]);
    let ready = fx.summary(fx.caller_b());
    assert_eq!(ready["status"], "ready", "{ready}");
    assert_eq!(ready["data"]["cover"].as_array().unwrap().len(), 6);
    assert!(
        ready["data"]["fold"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["item"]["body"]["text"] == "A asks B to review the plan")
    );
}
