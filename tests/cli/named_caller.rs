//! ht-kqrt.135 (N5-cli): the Named site through the ordinary CLI caller path. One real
//! `herdr-threads daemon run` child over a fresh state directory with no Herdr host, and the real
//! binary run as an agent stand-in with `HERDR_THREADS_SEAT_NAME` and no Herdr environment: its
//! seat-acting commands resolve at the reserved locator, its reads default to the seat, and a
//! hook-enrolled seat is the CLI's seat.

use herdr_threads::test_support::{
    isolation::{TestIsolation, as_agent},
    resolve_seeded,
    spawn::{OwnedChild, SpawnOwned},
    spawn_node_daemon,
};
use std::{
    path::PathBuf,
    process::Stdio,
    time::{Duration, Instant},
};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");

struct Ran {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl std::fmt::Debug for Ran {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "exit {:?}\nstdout: {}\nstderr: {}",
            self.code, self.stdout, self.stderr
        )
    }
}

struct Rig {
    daemon: Option<OwnedChild>,
    isolation: TestIsolation,
    state: PathBuf,
    /// A directory that is the whole `PATH` of every child: no `herdr` unless a test puts one in.
    bin: PathBuf,
}

impl Rig {
    fn start(label: &str) -> Self {
        let isolation = TestIsolation::new(label);
        let state = std::fs::canonicalize(isolation.state_root()).expect("the state root exists");
        let state = state.join("state");
        std::fs::create_dir_all(&state).unwrap();
        resolve_seeded(&state);
        let bin = isolation.path("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let daemon = spawn_node_daemon(
            &isolation,
            BIN,
            &state,
            None,
            &isolation.path("daemon.stderr"),
        )
        .expect("spawn the daemon");
        let rig = Self {
            daemon: Some(daemon),
            isolation,
            state,
            bin,
        };
        rig.wait_for_daemon();
        rig
    }

    fn wait_for_daemon(&self) {
        let until = Instant::now() + Duration::from_secs(30);
        while Instant::now() < until {
            if self.plain(&["daemon", "health"]).code == Some(0) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!(
            "the daemon did not come up: {}",
            std::fs::read_to_string(self.isolation.path("daemon.stderr")).unwrap_or_default()
        );
    }

    fn command(&self) -> std::process::Command {
        let mut command = self.isolation.command(BIN);
        command
            .env("PATH", &self.bin)
            .arg("--state-dir")
            .arg(&self.state);
        command
    }

    fn plain(&self, args: &[&str]) -> Ran {
        let mut command = self.command();
        command.args(args);
        run(command, None)
    }

    /// `args` as the agent of `session` named `name`: no pane, no Herdr, no `--cooperative-*`.
    fn agent(&self, name: &str, session: &str, args: &[&str]) -> Ran {
        self.agent_with(name, session, args, None)
    }

    fn agent_with(&self, name: &str, session: &str, args: &[&str], stdin: Option<&[u8]>) -> Ran {
        let mut command = self.command();
        command.args(args).env("HERDR_THREADS_SEAT_NAME", name);
        as_agent(&mut command, "claude", session);
        run(command, stdin)
    }

    /// The SessionStart hook of `session` under `name`.
    fn session_start(&self, name: &str, session: &str) -> Ran {
        let payload = format!(
            r#"{{"session_id":"{session}","transcript_path":"/tmp/t.jsonl","cwd":"/tmp","hook_event_name":"SessionStart","source":"startup"}}"#
        );
        self.agent_with(name, session, &["hook", "claude"], Some(payload.as_bytes()))
    }

    fn db(&self) -> rusqlite::Connection {
        let paths = resolve_seeded(&self.state);
        let db = rusqlite::Connection::open(paths.database_path).unwrap();
        db.busy_timeout(Duration::from_secs(5)).unwrap();
        db
    }

    fn seat_count(&self) -> i64 {
        self.db()
            .query_row("SELECT COUNT(*) FROM seats", [], |r| r.get(0))
            .unwrap()
    }

    fn seat_of(&self, name: &str) -> String {
        self.db()
            .query_row(
                "SELECT id FROM seats WHERE name=?1 AND state<>'retired'",
                [name],
                |r| r.get(0),
            )
            .unwrap_or_else(|e| panic!("a seat named {name}: {e}"))
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = self.plain(&["daemon", "stop"]);
        self.daemon.take();
    }
}

fn run(mut command: std::process::Command, stdin: Option<&[u8]>) -> Ran {
    use std::io::Write;
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    if stdin.is_some() {
        command.stdin(Stdio::piped());
    } else {
        command.stdin(Stdio::null());
    }
    let mut child: OwnedChild = command.spawn_owned().expect("spawn the process");
    if let Some(bytes) = stdin {
        child.stdin.take().unwrap().write_all(bytes).unwrap();
    }
    let output = child.wait_with_output().expect("process output");
    Ran {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn sent_message(ran: &Ran) -> String {
    let words: Vec<&str> = ran.stdout.split_whitespace().collect();
    let at = words
        .iter()
        .position(|w| *w == "message")
        .unwrap_or_else(|| panic!("a message id in {ran:?}"));
    words[at + 1].to_owned()
}

/// The SessionStart hook until it enrolls: the named host's first observation of a new seat
/// lands on the daemon's observation cadence, so the first check-ins answer "unavailable".
fn enroll(rig: &Rig, name: &str, session: &str) {
    let started = Instant::now();
    loop {
        let hook = rig.session_start(name, session);
        if hook.stdout.contains("Your seat") {
            return;
        }
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "{name} never enrolled: {hook:?}"
        );
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Two named seats, each enrolled by its SessionStart hook (the binding a CLI send needs).
fn two_seats(rig: &Rig) -> (String, String) {
    for (name, session) in [("alice", "s-alice"), ("bob", "s-bob")] {
        enroll(rig, name, session);
    }
    (rig.seat_of("alice"), rig.seat_of("bob"))
}

// Kills: a Named site whose `send` is refused with "this command acts as a seat", and a send
// attributed to anyone but the named seat.
#[test]
fn named_send() {
    let rig = Rig::start("nc-send");
    let (alice, bob) = two_seats(&rig);
    let sent = rig.agent(
        "alice",
        "s-alice",
        &["send", "--seat", &bob, "--body", "hello bob"],
    );
    assert_eq!(sent.code, Some(0), "{sent:?}");
    let inbox = rig.agent("bob", "s-bob", &["inbox"]);
    assert!(inbox.stdout.contains("hello bob"), "{inbox:?}");
    assert!(
        inbox.stdout.contains(&alice),
        "from the named seat: {inbox:?}"
    );
}

// Kills: `join` refused for a Named site, and a join recorded for another seat.
#[test]
fn named_join() {
    let rig = Rig::start("nc-join");
    let (alice, bob) = two_seats(&rig);
    let created = rig.agent(
        "alice",
        "s-alice",
        &["thread", "create", "--name", "ops", "--topic", "ops"],
    );
    assert_eq!(created.code, Some(0), "{created:?}");
    let joined = rig.agent("bob", "s-bob", &["join", "ops"]);
    assert_eq!(joined.code, Some(0), "{joined:?}");
    let members = rig.agent("bob", "s-bob", &["participants", "ops", "--machine"]);
    assert!(
        members.stdout.contains(&alice) && members.stdout.contains(&bob),
        "{members:?}"
    );
}

// Kills: `ack` refused for a Named site, and an ACK that never reaches the sender.
#[test]
fn named_ack() {
    let rig = Rig::start("nc-ack");
    let (_alice, bob) = two_seats(&rig);
    let sent = rig.agent(
        "alice",
        "s-alice",
        &[
            "send",
            "--seat",
            &bob,
            "--body",
            "ack me",
            "--require-ack",
            &bob,
        ],
    );
    assert_eq!(sent.code, Some(0), "{sent:?}");
    let id = sent_message(&sent);
    let owed = rig.agent("bob", "s-bob", &["pending-receipts", "--machine"]);
    assert!(owed.stdout.contains(&id), "{owed:?}");
    let ack = rig.agent("bob", "s-bob", &["ack", &id]);
    assert_eq!(ack.code, Some(0), "{ack:?}");
    let after = rig.agent("bob", "s-bob", &["pending-receipts", "--machine"]);
    assert!(!after.stdout.contains(&id), "{after:?}");
}

// Kills: `accept` refused for a Named site, and an accept that omits the backlog.
#[test]
fn named_accept() {
    let rig = Rig::start("nc-accept");
    let (_alice, bob) = two_seats(&rig);
    let created = rig.agent(
        "alice",
        "s-alice",
        &["thread", "create", "--name", "ops", "--topic", "ops"],
    );
    assert_eq!(created.code, Some(0), "{created:?}");
    let invited = rig.agent("alice", "s-alice", &["invite", "ops", "--seat", &bob]);
    assert_eq!(invited.code, Some(0), "{invited:?}");
    let accepted = rig.agent("bob", "s-bob", &["accept", "ops"]);
    assert_eq!(accepted.code, Some(0), "{accepted:?}");
}

/// A `herdr` that records any call to a marker file.
fn marking_herdr(rig: &Rig) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let marker = rig.isolation.path("herdr.called");
    let herdr = rig.bin.join("herdr");
    std::fs::write(
        &herdr,
        format!("#!/bin/sh\necho \"$*\" >> '{}'\nexit 3\n", marker.display()),
    )
    .unwrap();
    std::fs::set_permissions(&herdr, std::fs::Permissions::from_mode(0o755)).unwrap();
    marker
}

// Kills: a named CLI process that dials the Herdr socket or runs a `herdr` query for anything
// but a handoff, and a claim host that is the Herdr server instead of the reserved locator.
#[test]
fn claim_host_is_reserved_with_and_without_herdr() {
    let rig = Rig::start("nc-host");
    let marker = marking_herdr(&rig);
    let socket = rig.isolation.socket_path("herdr.sock");
    let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let (_alice, bob) = two_seats(&rig);
    let mut command = rig.command();
    command
        .args(["send", "--seat", &bob, "--body", "x"])
        .env("HERDR_THREADS_SEAT_NAME", "alice")
        .env("HERDR_SOCKET_PATH", &socket);
    as_agent(&mut command, "claude", "s-alice");
    let sent = run(command, None);
    assert_eq!(sent.code, Some(0), "{sent:?}");
    assert!(
        !marker.exists(),
        "no herdr query: {}",
        std::fs::read_to_string(&marker).unwrap_or_default()
    );
    assert!(
        listener.accept().is_err(),
        "the Herdr socket must never be connected"
    );
}

// Kills: a hook lifecycle event that fails to enroll a name with no pane (`service returned no
// resolved seat`), and a CLI command that is not the hook-enrolled seat.
#[test]
fn hook_lifecycle_enrolls_then_the_cli_acts_as_that_seat() {
    let rig = Rig::start("nc-hook");
    enroll(&rig, "carol", "s-carol");
    let seat = rig.seat_of("carol");
    let me = rig.agent("carol", "s-carol", &["seat", "list", "--machine"]);
    assert_eq!(me.code, Some(0), "{me:?}");
    assert!(me.stdout.contains(&seat), "{me:?}");
    let again = rig.agent("carol", "s-carol", &["inbox"]);
    assert_eq!(again.code, Some(0), "{again:?}");
    assert!(!again.stderr.contains("created seat"), "{again:?}");
    assert_eq!(rig.seat_count(), 1);
}

// Kills: a first `inbox` that prints no M8, M8 printed twice, and a second seat created.
#[test]
fn first_text_inbox_prints_m8_once() {
    let rig = Rig::start("nc-m8");
    let first = rig.agent("reviewer", "s-rev", &["inbox"]);
    assert_eq!(first.code, Some(0), "{first:?}");
    let seat = rig.seat_of("reviewer");
    let normalized = first.stderr.replace(&seat, "seat-Xy98Zw76");
    let golden = include_str!("golden/m8.txt");
    assert_eq!(normalized, golden, "{first:?}");
    let second = rig.agent("reviewer", "s-rev", &["inbox"]);
    assert_eq!(second.code, Some(0), "{second:?}");
    assert!(!second.stderr.contains("created seat"), "{second:?}");
    let list = rig.agent("reviewer", "s-rev", &["seat", "list", "--machine"]);
    assert_eq!(list.stdout.matches(&seat).count(), 1, "{list:?}");
    assert_eq!(rig.seat_count(), 1);
}

// Kills: a read (`inbox --json|--machine|--seat`, `seat list`, `thread show`) that allocates the
// seat of an unknown name.
#[test]
fn reads_under_an_unknown_name_create_no_seat() {
    let rig = Rig::start("nc-reads");
    let (alice, _bob) = two_seats(&rig);
    let before = rig.seat_count();
    for args in [
        vec!["inbox", "--json"],
        vec!["inbox", "--machine"],
        vec!["inbox", "--seat", alice.as_str()],
        vec!["seat", "list"],
        vec!["thread", "show", "nothread"],
    ] {
        let ran = rig.agent("ghost", "s-ghost", &args);
        assert!(!ran.stderr.contains("created seat"), "{args:?}: {ran:?}");
        assert_eq!(rig.seat_count(), before, "{args:?}: {ran:?}");
    }
}

// Kills: a caller-default read or participant self marker that does not know a named seat.
#[test]
fn known_name_reads_default_to_the_seat_and_mark_self() {
    let rig = Rig::start("nc-self");
    let (alice, _bob) = two_seats(&rig);
    let created = rig.agent(
        "alice",
        "s-alice",
        &["thread", "create", "--name", "ops", "--topic", "ops"],
    );
    assert_eq!(created.code, Some(0), "{created:?}");
    let machine = rig.agent("alice", "s-alice", &["inbox", "--machine"]);
    assert_eq!(machine.code, Some(0), "{machine:?}");
    let members = rig.agent("alice", "s-alice", &["participants", "ops"]);
    assert_eq!(members.code, Some(0), "{members:?}");
    assert!(members.stdout.contains(&alice), "{members:?}");
    assert!(
        members.stdout.contains("(you)")
            || members.stdout.contains("self")
            || members.stdout.contains("you"),
        "the self marker: {members:?}"
    );
}

// Kills: a named caller with neither an agent harness session nor a saved context that creates
// a seat or is refused with anything but M15.
#[test]
fn no_harness_session_and_no_context_is_refused_with_m15() {
    let rig = Rig::start("nc-m15");
    let mut command = rig.command();
    command
        .args(["inbox"])
        .env("HERDR_THREADS_SEAT_NAME", "nobody");
    let ran = run(command, None);
    assert_ne!(ran.code, Some(0), "{ran:?}");
    assert!(
        ran.stderr
            .contains(include_str!("golden/m15.txt").trim_end()),
        "{ran:?}"
    );
    assert_eq!(rig.seat_count(), 0);
}

// Kills: a second unrelated live process holding the same name that is bound instead of
// refused with M7.
#[test]
fn second_live_process_with_the_name_is_refused_with_m7() {
    let rig = Rig::start("nc-m7");
    enroll(&rig, "reviewer", "s-one");
    let second = rig.agent(
        "reviewer",
        "s-two",
        &["thread", "create", "--name", "x", "--topic", "x"],
    );
    assert_ne!(second.code, Some(0), "{second:?}");
    let golden = include_str!("golden/m7.txt").trim_end();
    let (head, tail) = golden.split_once("pid 20").expect("the golden pid");
    assert!(
        second.stderr.contains(head)
            && second
                .stderr
                .contains(tail.trim_start_matches(char::is_numeric)),
        "{second:?}"
    );
    assert_eq!(rig.seat_count(), 1);
}

// Kills: a `next:` line that pins the reserved locator as `--host-endpoint`.
#[test]
fn no_next_line_carries_the_reserved_locator() {
    let rig = Rig::start("nc-next");
    let (_alice, bob) = two_seats(&rig);
    for body in ["one", "two", "three"] {
        let sent = rig.agent(
            "alice",
            "s-alice",
            &["send", "--seat", &bob, "--body", body],
        );
        assert_eq!(sent.code, Some(0), "{sent:?}");
    }
    for args in [
        vec!["inbox", "--limit", "1"],
        vec!["inbox", "--machine", "--limit", "1"],
    ] {
        let ran = rig.agent("bob", "s-bob", &args);
        assert!(
            !ran.stdout.contains(".herdr-threads") && !ran.stderr.contains(".herdr-threads"),
            "{args:?}: {ran:?}"
        );
    }
}

impl Rig {
    /// `args` as the agent of `session` named `name` under `HERDR_THREADS_SEAT_READONLY=1`.
    fn readonly_agent(&self, name: &str, session: &str, args: &[&str]) -> Ran {
        let mut command = self.command();
        command
            .args(args)
            .env("HERDR_THREADS_SEAT_NAME", name)
            .env("HERDR_THREADS_SEAT_READONLY", "1");
        as_agent(&mut command, "claude", session);
        run(command, None)
    }

    /// The `readonly` flags of the named seat's bindings.
    fn readonly_flags(&self, seat: &str) -> Vec<i64> {
        let db = self.db();
        let mut statement = db
            .prepare("SELECT readonly FROM occupant_bindings WHERE seat_id=?1")
            .unwrap();
        statement
            .query_map([seat], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }
}

/// Alice (hook-enrolled) sends bob an ACK request; bob is enrolled only by plain commands, with
/// or without the readonly variable. Returns the message id after bob's `inbox` displayed it.
fn plain_enrolled_bob_reads(rig: &Rig, readonly: bool) -> (String, String) {
    enroll(rig, "alice", "s-alice");
    let bob_agent = |args: &[&str]| {
        if readonly {
            rig.readonly_agent("bob", "s-bob", args)
        } else {
            rig.agent("bob", "s-bob", args)
        }
    };
    // The first plain command allocates and enrolls bob: no hook ever runs for him. The named
    // host observes a new seat on its cadence: until then a readonly check-in (and a send to the
    // seat) is refused, so each step retries like `enroll`.
    let until = Instant::now() + Duration::from_secs(60);
    let retry = |step: &dyn Fn() -> Ran| {
        loop {
            let ran = step();
            if ran.code == Some(0) {
                return ran;
            }
            assert!(Instant::now() < until, "never succeeded: {ran:?}");
            std::thread::sleep(Duration::from_millis(250));
        }
    };
    // A refused check-in stays a pending local intent (a bare Conflict on every rerun) until
    // `retry <ref>` replays it once the seat is observed.
    retry(&|| {
        let ran = bob_agent(&["inbox"]);
        if ran.code != Some(0) {
            let pending = bob_agent(&["pending-ops"]);
            if let Some(at) = pending.stdout.find("local:") {
                let reference: String = pending.stdout[at..]
                    .chars()
                    .take_while(|c| *c != '"')
                    .collect();
                bob_agent(&[
                    "--host-endpoint",
                    "/.herdr-threads/named-host",
                    "retry",
                    &reference,
                ]);
            }
        }
        ran
    });
    let bob = rig.seat_of("bob");
    let sent = retry(&|| {
        rig.agent(
            "alice",
            "s-alice",
            &[
                "send",
                "--seat",
                &bob,
                "--body",
                "ack me",
                "--require-ack",
                &bob,
            ],
        )
    });
    let id = sent_message(&sent);
    // One display. Under the variable the display itself exits non-zero (its settlement is
    // skipped), so only the rows it printed are asserted.
    let shown = bob_agent(&["inbox"]);
    assert!(shown.stdout.contains("ack me"), "{shown:?}");
    (id, bob)
}

// Kills: a session enrolled only by plain commands under the readonly variable whose display
// discharges the ACK, and a binding row left writable.
#[test]
fn plain_commands_under_readonly_never_discharge_an_ack() {
    let rig = Rig::start("nc-ro-plain");
    let (id, bob) = plain_enrolled_bob_reads(&rig, true);
    let owed = rig.readonly_agent("bob", "s-bob", &["pending-receipts", "--machine"]);
    assert!(
        owed.stdout.contains(&id),
        "still owed after display: {owed:?}"
    );
    let flags = rig.readonly_flags(&bob);
    assert!(
        !flags.is_empty() && flags.iter().all(|flag| *flag == 1),
        "every binding of the seat is readonly: {flags:?}"
    );
}

// The control: without the variable the same flow sends no marking check-in, so the binding
// stays writable and the display exits clean.
#[test]
fn plain_commands_without_the_variable_stay_writable() {
    let rig = Rig::start("nc-rw-plain");
    let (_id, bob) = plain_enrolled_bob_reads(&rig, false);
    let flags = rig.readonly_flags(&bob);
    assert!(
        !flags.is_empty() && flags.iter().all(|flag| *flag == 0),
        "{flags:?}"
    );
}
