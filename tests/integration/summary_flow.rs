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
    process::{Command, Stdio},
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
    claude_hook: String,
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
        // The installed hook parses only under a harness version it can
        // observe on PATH: pinned reporters stand in for `claude`/`codex`.
        let bin = root.join("bin");
        fs::create_dir_all(&bin).unwrap();
        for (name, line) in [
            ("claude", "2.1.283 (Claude Code)"),
            ("codex", "codex-cli 0.157.1"),
        ] {
            let path = bin.join(name);
            fs::write(&path, format!("#!/bin/sh\necho '{line}'\n")).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
            // A freshly written script's first exec can outlast the hook's
            // version-observation deadline on macOS: warm it here.
            assert!(
                Command::new(&path)
                    .arg("--version")
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
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
        let claude_hook = installed["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .to_owned();
        let codex_argv = installed_argv(BIN, Some(state.to_str().unwrap()), None, Harness::Codex);
        let codex_hook = herdr_threads::harness::setup::shell_command(&codex_argv).unwrap();
        let world = Self {
            root,
            state,
            host: socket,
            instance_dir: paths.instance_dir,
            claude_hook,
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
            .env("CLAUDE_CONFIG_DIR", self.root.join("claude-config"))
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
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
            &self.codex_hook
        } else {
            &self.claude_hook
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
            "submission_schema": 1,
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
                    json!({"submission_schema": 1, "narrative": "n",
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
    // `<chunking_version>.<chunk>.<n>`), chunk 2's own instruction through
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
        ledger("instruction", "done", 10)
            && text.contains(&format!("instruction i.{PRIORITY_SEQ} done at #10")),
        "same-chunk instruction: {text}"
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
    let lapse = work["data"]["jobs"][0]["lease_until"].as_u64().unwrap();
    wait_until("the reservation to lapse", Duration::from_secs(90), || {
        utc_ms() > lapse + 500
    });
    // Free and lapsed: honoured, and the lease starts now.
    let honoured = fx.fetch(fx.caller_b(), &by_index(0));
    assert_eq!(honoured["status"], "bundle", "{honoured}");
    assert_eq!(honoured["data"]["range"]["first_seq"], 1);
    // C polls: job 0 is B's live lease, the rest lapsed and are free, so C
    // leases them.
    let theirs = fx.summary(fx.caller_c());
    assert_eq!(theirs["status"], "work", "{theirs}");
    let c_jobs = tickets(&theirs);
    // The overdue warning the stalled receipt earned during the wait is a
    // thread message too, so a seventh chunk may have filled meanwhile.
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
        notices
            .iter()
            .any(|notice| notice["message"].as_str().unwrap().starts_with("notify-")),
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
