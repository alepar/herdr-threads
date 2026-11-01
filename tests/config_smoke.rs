//! ht-kqrt.59 (Config-smoke): every caller site once per builtin harness stand-in, then messaging
//! across sites inside one daemon.
//!
//! The 4 x 4 matrix: a Herdr pane (isolated, named Herdr server), a plain `HERDR_THREADS_SEAT_NAME`
//! process, a named seat inside a private `tmux -L ht-test-<uuid>` server, and an hc terminal
//! against `fake_hc`; each with the claude, codex, hermes and agy hook stand-ins. A cell runs the
//! recipient's lifecycle hook until it enrolls, one turn hook, then a send / hook delivery / inbox /
//! ACK round trip from a claude sender of the same site, and checks the recipient's wake
//! classification as the sender sees it: a `warning: <seat> wake=no_wake` on the first ACK-requesting
//! send iff the seat has no idle wake (`IdleWakeOn::None`). Every child is a `test_support` owned
//! child; every daemon, Herdr server, tmux server and fake core is stopped on drop.

#[allow(dead_code)]
#[path = "named_seats/rig.rs"]
mod rig;

use herdr_threads::test_support::{
    fake_hc::{FakeHc, FakeTerminal},
    isolation::as_agent,
    tmux::{Pane, PrivateTmux},
};
use rig::{Ran, Rig, run, sent_message, wait_for};
use serde_json::{Value, json};
use std::{os::unix::fs::PermissionsExt, path::PathBuf, time::Duration};

const AGY_ID: &str = "534e47d6-64b3-4fd6-bd7f-446ceec4d86c";
const HARNESSES: [&str; 4] = ["claude", "codex", "hermes", "agy"];
/// The fake core's name: the `<session>` of an hc terminal's `<session>:tN` label.
const CORE: &str = "alpha";

/// Where a caller runs.
#[derive(Clone)]
enum Loc {
    Name(String),
    Tmux(String, Pane),
    Pane(String),
    Terminal(String),
}

/// One agent stand-in: a harness, its session and its site.
#[derive(Clone)]
struct Caller {
    harness: &'static str,
    session: String,
    loc: Loc,
}

/// What the sender is told about a seat's idle wake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wake {
    /// A host wake path exists (Herdr composer, verified tmux): no `no_wake` warning.
    Composer,
    /// No idle wake: one `wake=no_wake` warning.
    None,
}

struct World {
    // Drop order: the daemon (and Herdr under it) stops before the servers beside it.
    rig: Rig,
    tmux: Option<PrivateTmux>,
    fake: Option<FakeHc>,
    /// The `TMUX` variable a process in the private server's panes has.
    tmux_var: String,
    /// The runner directory of each tmux pane that executes commands (see [`World::tmux_pane`]).
    runners: std::cell::RefCell<std::collections::HashMap<String, PathBuf>>,
    calls: std::cell::Cell<u64>,
}

impl World {
    fn new(rig: Rig, tmux: Option<PrivateTmux>, fake: Option<FakeHc>) -> Self {
        let tmux_var = tmux.as_ref().map_or_else(String::new, |tmux| {
            format!("{},{},0", tmux.socket_path().display(), tmux.server_pid())
        });
        if tmux.is_some() {
            // The daemon's whole PATH is the rig's bin directory; hint verification runs `tmux`.
            let tmux_program = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                .map(|dir| dir.join("tmux"))
                .find(|path| path.is_file())
                .expect("tmux on PATH");
            rig::executable(
                &rig.bin.join("tmux"),
                &format!("#!/bin/sh\nexec {} \"$@\"\n", tmux_program.display()),
            );
        }
        // Bare Hermes callback admission must not depend on the embedded manifest.
        let paths = herdr_threads::daemon::paths::NodePaths::resolve(&rig.state).unwrap();
        paths.prepare_node_dir().unwrap();
        let settings = paths
            .node_dir
            .join(herdr_threads::daemon::settings::SETTINGS_FILE);
        std::fs::write(&settings, br#"{"harness_manifest":"off"}"#).unwrap();
        std::fs::set_permissions(&settings, std::fs::Permissions::from_mode(0o600)).unwrap();
        Self {
            rig,
            tmux,
            fake,
            tmux_var,
            runners: Default::default(),
            calls: Default::default(),
        }
    }

    fn fake_with(terminals: &[&str]) -> FakeHc {
        let fake = FakeHc::start();
        fake.state().set_core_name(CORE);
        for terminal in terminals {
            fake.add_terminal(FakeTerminal::live(terminal));
        }
        fake
    }

    fn agy_home(&self) -> PathBuf {
        let data = self.rig.isolation.path("agy-home/.gemini/antigravity-cli");
        for file in [
            format!("presence/{AGY_ID}.lock"),
            format!("brain/{AGY_ID}/.system_generated/logs/transcript_full.jsonl"),
        ] {
            let path = data.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            if !path.exists() {
                std::fs::File::create(&path).unwrap();
            }
        }
        self.rig.isolation.path("agy-home")
    }

    /// `args` as `caller`: the real binary, the harness stand-in and the site's environment.
    fn run(&self, caller: &Caller, args: &[&str], stdin: Option<&[u8]>) -> Ran {
        let command = self.prepare(caller, args);
        match &caller.loc {
            Loc::Tmux(_, pane) => self.run_in_pane(pane, &command, stdin),
            _ => run(command, stdin),
        }
    }

    fn prepare(&self, caller: &Caller, args: &[&str]) -> std::process::Command {
        let mut command = self.rig.command();
        command.args(args);
        as_agent(&mut command, caller.harness, &caller.session);
        match &caller.loc {
            Loc::Name(name) => {
                command.env("HERDR_THREADS_SEAT_NAME", name);
            }
            Loc::Tmux(name, pane) => {
                // The pane's own process stands in for the harness process, so the daemon's
                // ancestry check finds the binding's process inside the pane.
                let pid = pane.pid.to_string();
                command
                    .env("HERDR_THREADS_SEAT_NAME", name)
                    .env("TMUX", &self.tmux_var)
                    .env("TMUX_PANE", &pane.id)
                    .env("HT_TEST_HARNESS_PID", &pid)
                    .env("CLAUDE_PID", &pid);
            }
            Loc::Pane(pane) => {
                command.env("HERDR_ENV", "1").env("HERDR_PANE_ID", pane);
            }
            Loc::Terminal(terminal) => {
                command
                    .env("HC_TERMINAL_ID", terminal)
                    .env("HC_TERMINAL_SOCKET", self.fake.as_ref().unwrap().socket());
            }
        }
        if caller.harness == "agy" {
            command.env("HOME", self.agy_home());
        }
        command
    }

    /// A new tmux pane that executes the commands handed to [`World::run_in_pane`]. The daemon
    /// verifies a tmux hint against the ancestry of the calling `herdr-threads` process, so the
    /// call must be a descendant of the pane's process: the pane runs a loop that reads call numbers
    /// from a fifo and runs `<n>.sh` with `<n>.in` as stdin.
    fn tmux_pane(&self) -> Pane {
        let dir = self
            .rig
            .isolation
            .path(format!("tmux-runner-{}", self.runners.borrow().len()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut mkfifo = std::process::Command::new("mkfifo");
        mkfifo.arg(dir.join("fifo"));
        assert_eq!(run(mkfifo, None).code, Some(0), "mkfifo");
        rig::executable(
            &dir.join("runner.sh"),
            r#"#!/bin/bash
d=$1
exec 3<>"$d/fifo"
while read -r n <&3; do
  /bin/bash "$d/$n.sh" <"$d/$n.in" >"$d/$n.out" 2>"$d/$n.err"
  echo $? >"$d/$n.code"
done
"#,
        );
        let pane = self
            .tmux
            .as_ref()
            .unwrap()
            // The shell stays the pane's process and the runner is its child: the live process a
            // verified binding is anchored on, as a harness started by a shell is.
            .new_pane(&format!("/bin/bash {0}/runner.sh {0}; true", dir.display()));
        self.runners.borrow_mut().insert(pane.id.clone(), dir);
        pane
    }

    /// `command` (program, arguments and its resolved environment) run by the runner of `pane`.
    fn run_in_pane(
        &self,
        pane: &Pane,
        command: &std::process::Command,
        stdin: Option<&[u8]>,
    ) -> Ran {
        use std::io::Write;
        let dir = self.runners.borrow()[&pane.id].clone();
        let n = self.calls.get() + 1;
        self.calls.set(n);
        let mut env: std::collections::BTreeMap<std::ffi::OsString, std::ffi::OsString> =
            std::env::vars_os().collect();
        for (key, value) in command.get_envs() {
            match value {
                Some(value) => env.insert(key.to_owned(), value.to_owned()),
                None => env.remove(key),
            };
        }
        let quote = |text: &str| shlex::try_quote(text).unwrap().into_owned();
        let mut script = String::from("#!/bin/bash\nexec /usr/bin/env -i");
        for (key, value) in &env {
            script.push_str(&format!(
                " {}",
                quote(&format!(
                    "{}={}",
                    key.to_string_lossy(),
                    value.to_string_lossy()
                ))
            ));
        }
        script.push_str(&format!(
            " {}",
            quote(&command.get_program().to_string_lossy())
        ));
        for arg in command.get_args() {
            script.push_str(&format!(" {}", quote(&arg.to_string_lossy())));
        }
        script.push('\n');
        std::fs::write(dir.join(format!("{n}.sh")), script).unwrap();
        std::fs::write(dir.join(format!("{n}.in")), stdin.unwrap_or_default()).unwrap();
        let mut fifo = std::fs::OpenOptions::new()
            .write(true)
            .open(dir.join("fifo"))
            .unwrap();
        writeln!(fifo, "{n}").unwrap();
        drop(fifo);
        let code = wait_for(
            &format!("tmux pane {} running call {n}", pane.id),
            Duration::from_secs(120),
            || std::fs::read_to_string(dir.join(format!("{n}.code"))).map_err(|e| e.to_string()),
        );
        let read = |ext: &str| std::fs::read_to_string(dir.join(format!("{n}.{ext}"))).unwrap();
        Ran {
            code: code.trim().parse().ok(),
            stdout: read("out"),
            stderr: read("err"),
        }
    }

    /// One hook of `caller`'s harness: its lifecycle start (`turn: false`) or a turn start.
    fn hook(&self, caller: &Caller, turn: bool) -> Ran {
        let session = caller.session.as_str();
        let (args, payload): (Vec<&str>, Vec<u8>) = match caller.harness {
            "claude" => (
                vec!["hook", "claude"],
                json!({"session_id": session, "transcript_path": "/tmp/t.jsonl", "cwd": "/tmp",
                    "hook_event_name": if turn { "UserPromptSubmit" } else { "SessionStart" },
                    "source": "startup", "prompt": "go"})
                .to_string()
                .into_bytes(),
            ),
            "codex" => (
                vec!["hook", "codex"],
                if turn {
                    json!({"session_id": session, "cwd": "/tmp", "model": "gpt-5",
                        "permission_mode": "default", "transcript_path": null,
                        "hook_event_name": "UserPromptSubmit", "prompt": "go", "turn_id": "t1"})
                } else {
                    json!({"session_id": session, "hook_event_name": "SessionStart",
                        "source": "startup", "turn_id": "t0", "model": "gpt-5"})
                }
                .to_string()
                .into_bytes(),
            ),
            "hermes" => (vec!["hook", "hermes"], hermes_envelope(session, turn)),
            "agy" => {
                let home = self.agy_home();
                let name = if turn {
                    "pre-invocation-inv0-fresh"
                } else {
                    "session-start-top"
                };
                let text = std::fs::read_to_string(
                    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                        .join("tests/fixtures/agy/payloads")
                        .join(format!("{name}.json")),
                )
                .unwrap()
                .replace("<root>", home.to_str().unwrap());
                (
                    vec![
                        "hook",
                        "agy",
                        "--event",
                        if turn {
                            "PreInvocation"
                        } else {
                            "SessionStart"
                        },
                    ],
                    text.into_bytes(),
                )
            }
            other => panic!("no hook payload for {other}"),
        };
        self.run(caller, &args, Some(&payload))
    }

    fn db(&self) -> rusqlite::Connection {
        self.rig.db()
    }

    /// The caller's seat, once its hook has created it.
    fn seat(&self, caller: &Caller) -> Option<String> {
        let (column, value) = match &caller.loc {
            Loc::Name(name) | Loc::Tmux(name, _) => ("name", name.as_str()),
            Loc::Pane(target) | Loc::Terminal(target) => ("target_id", target.as_str()),
        };
        self.db()
            .query_row(
                &format!("SELECT id FROM seats WHERE {column}=?1 AND state<>'retired'"),
                [value],
                |row| row.get(0),
            )
            .ok()
    }

    /// The lifecycle hook until the caller's seat has an open binding of its harness (a named or
    /// hc host first observes a new seat on the daemon's cadence). Returns the seat.
    fn enroll(&self, caller: &Caller) -> String {
        wait_for(
            &format!("{} {} enrolled", caller.harness, caller.session),
            Duration::from_secs(90),
            || {
                // A harness may bind at its first turn instead of its session start.
                let ran = self.hook(caller, false);
                let _ = self.hook(caller, true);
                let Some(seat) = self.seat(caller) else {
                    return Err(format!("no seat: {ran:?}"));
                };
                let bound: i64 = self
                    .db()
                    .query_row(
                        "SELECT COUNT(*) FROM occupant_bindings WHERE seat_id=?1 \
                         AND harness=?2 AND ended_at IS NULL",
                        [&seat, caller.harness],
                        |row| row.get(0),
                    )
                    .unwrap();
                if ran.code == Some(0) && bound > 0 {
                    Ok(seat)
                } else {
                    Err(format!("{seat} unbound: {ran:?}"))
                }
            },
        )
    }

    /// A turn hook until its output carries `want`; the hook's output.
    fn delivery(&self, caller: &Caller, want: &str) -> String {
        wait_for(
            &format!("{} turn hook delivering {want:?}", caller.harness),
            Duration::from_secs(45),
            || {
                let ran = self.hook(caller, true);
                if ran.code == Some(0) && ran.stdout.contains(want) {
                    Ok(ran.stdout)
                } else {
                    Err(format!("{ran:?}"))
                }
            },
        )
    }
}

/// A Hermes callback envelope for `session`: the fixture a legacy bridge sends, with fresh times.
fn hermes_envelope(session: &str, turn: bool) -> Vec<u8> {
    let mut payload: Value =
        serde_json::from_slice(include_bytes!("fixtures/hermes/envelopes.json")).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let event = format!("e-{now}");
    payload["callback"] = if turn {
        "pre_llm_call"
    } else {
        "on_session_start"
    }
    .into();
    payload["session_id"] = session.into();
    payload["event_id"] = event.as_str().into();
    payload["started_at"] = now.into();
    payload["deadline_at"] = (now + if turn { 1200 } else { 4500 }).into();
    payload["observation_order"]["observed_at_millis"] = now.into();
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    payload["observation_order"]["sequence"] = SEQUENCE
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        .into();
    if turn {
        payload["turn_id"] = event.as_str().into();
        payload["role_association"]["session_id"] = session.into();
        payload["role_association"]["turn_id"] = event.as_str().into();
        payload["role_association"]["enrollment_witness"] = "cli_platform".into();
    } else {
        // A CLI session start: the top-level association with its enrollment witness, no turn
        // and no parent.
        let missing = json!({"presence": "missing", "type": "absent"});
        payload["turn_id"] = Value::Null;
        payload["shape"]["turn_id"] = missing.clone();
        payload["parent_session_id"] = Value::Null;
        payload["shape"]["parent_session_id"] = missing;
        payload["role_association"] = json!({"role": "top", "provenance": "cli_session_start",
            "session_id": session, "turn_id": null, "enrollment_witness": "cli_platform"});
        payload["observation_order"]["callback_budget_millis"] = 4500.into();
    }
    serde_json::to_vec(&payload).unwrap()
}

/// How many `wake=no_wake` warnings a send printed.
fn no_wake_warnings(ran: &Ran) -> usize {
    ran.stderr.matches("wake=no_wake").count() + ran.stdout.matches("wake=no_wake").count()
}

/// What the registry says the seat's wake is on a host with `composer` (a host wake path).
fn registry_wake(harness: &str, composer: bool) -> Wake {
    use herdr_threads::{
        harness::registry::{IdleWakeOn, builtins},
        ports::{ComposerTraits, WakeCapability},
    };
    let capability = if composer {
        WakeCapability::Composer(ComposerTraits::TMUX)
    } else {
        WakeCapability::None
    };
    match builtins().idle_wake_on(harness, capability, false) {
        Some(IdleWakeOn::Composer) => Wake::Composer,
        Some(IdleWakeOn::None) => Wake::None,
        other => panic!("{harness} on a {capability:?} host: {other:?}"),
    }
}

/// One cell: `recipient` of `harness` at the site, messaged by `sender` (an enrolled claude of the
/// same site). Returns the printed cell line.
fn cell(world: &World, site: &str, sender: &Caller, recipient: &Caller, expect: Wake) -> String {
    let seat = world.enroll(recipient);
    let turn = world.hook(recipient, true);
    assert_eq!(
        turn.code,
        Some(0),
        "{site}/{}: quiet turn hook: {turn:?}",
        recipient.harness
    );

    // A CLI call resolves the seat by name and carries the tmux hint the daemon verifies; the
    // hooks do not.
    let first_look = world.run(recipient, &["inbox"], None);
    assert_eq!(
        first_look.code,
        Some(0),
        "{site}/{}: {first_look:?}",
        recipient.harness
    );

    let body = format!("smoke {site} {}", recipient.harness);
    let sent = world.run(
        sender,
        &[
            "send",
            "--seat",
            &seat,
            "--body",
            &body,
            "--require-ack",
            &seat,
        ],
        None,
    );
    assert_eq!(sent.code, Some(0), "{site}/{}: {sent:?}", recipient.harness);
    let warned = no_wake_warnings(&sent);
    let observed = if warned == 0 {
        Wake::Composer
    } else {
        Wake::None
    };
    assert_eq!(
        observed, expect,
        "{site}/{}: the wake classification the sender saw: {sent:?}",
        recipient.harness
    );
    assert!(
        warned <= 1,
        "{site}/{}: warned once at most: {sent:?}",
        recipient.harness
    );
    let first = sent_message(&sent);

    // Hook delivery: the recipient's turn hook carries the body.
    let delivered = world.delivery(recipient, &body);
    assert!(delivered.contains(&body), "{delivered}");

    // Round trip: inbox shows the sender's message, an explicit ACK settles the obligation.
    let again = format!("{body} again");
    let second = world.run(
        sender,
        &[
            "send",
            "--seat",
            &seat,
            "--body",
            &again,
            "--require-ack",
            &seat,
        ],
        None,
    );
    assert_eq!(second.code, Some(0), "{second:?}");
    let second_id = sent_message(&second);
    let inbox = world.run(recipient, &["inbox"], None);
    assert!(
        inbox.stdout.contains(&again),
        "{site}/{}: {inbox:?}",
        recipient.harness
    );
    assert!(
        inbox.stdout.contains(&world.seat(sender).unwrap()),
        "{inbox:?}"
    );
    let ack = world.run(recipient, &["ack", &second_id], None);
    assert_eq!(ack.code, Some(0), "{ack:?}");
    for id in [&first, &second_id] {
        let owed = world.run(recipient, &["pending-receipts", "--machine"], None);
        assert!(
            !owed.stdout.contains(id.as_str()),
            "{site}/{}: {owed:?}",
            recipient.harness
        );
    }
    format!(
        "cell site={site} harness={} wake={observed:?} hook=ok send=ok inbox=ok ack=ok",
        recipient.harness
    )
}

/// Prints the cell list and checks it is the four harnesses in order.
fn print_cells(site: &str, cells: &[String]) {
    println!("config-smoke cells for {site}:");
    for cell in cells {
        println!("  {cell}");
    }
    assert_eq!(cells.len(), HARNESSES.len(), "{site}: {cells:?}");
}

fn claude(session: &str, loc: Loc) -> Caller {
    Caller {
        harness: "claude",
        session: session.to_owned(),
        loc,
    }
}

fn recipient(harness: &'static str, loc: Loc) -> Caller {
    Caller {
        harness,
        session: if harness == "agy" {
            AGY_ID.to_owned()
        } else {
            format!("s-{harness}")
        },
        loc,
    }
}

// Kills: a Herdr pane seat of any harness whose hook does not enroll, whose send/inbox/ACK trip
// fails, or that the sender is told has no wake (a pane is a Herdr composer host).
#[test]
fn herdr_pane_site_four_harnesses() {
    let Some(rig) = Rig::beside_herdr("cfgs-pane") else {
        return;
    };
    let world = World::new(rig, None, None);
    let pane = world.rig.pane("cfgs-sender");
    let sender = claude("s-sender", Loc::Pane(pane));
    world.enroll(&sender);
    let mut cells = Vec::new();
    for harness in HARNESSES {
        let pane = world.rig.pane(&format!("cfgs-{harness}"));
        let who = recipient(harness, Loc::Pane(pane));
        cells.push(cell(
            &world,
            "pane",
            &sender,
            &who,
            registry_wake(harness, true),
        ));
    }
    print_cells("pane", &cells);
}

// Kills: a plain named seat of any harness that cannot enroll or message, and one that is told it
// has an idle wake with no mod channel (a named host without tmux has no wake path).
#[test]
fn named_site_four_harnesses() {
    let world = World::new(Rig::standalone("cfgs-named"), None, None);
    let sender = claude("s-sender", Loc::Name("sender".into()));
    world.enroll(&sender);
    let mut cells = Vec::new();
    for harness in HARNESSES {
        let who = recipient(harness, Loc::Name(format!("rcpt-{harness}")));
        cells.push(cell(
            &world,
            "named",
            &sender,
            &who,
            registry_wake(harness, false),
        ));
    }
    print_cells("named", &cells);
}

// Kills: a named seat in a verified tmux pane that still reads as having no wake, a hint that is
// not verified for a harness other than claude, and a wake classification that depends on the
// harness's turn-state edge instead of the registry's composer policy.
#[test]
fn named_tmux_site_four_harnesses() {
    let Some(tmux) = PrivateTmux::start("cfgs-named-tmux") else {
        return;
    };
    let world = World::new(Rig::standalone("cfgs-tmux"), Some(tmux), None);
    let sender = claude("s-sender", Loc::Name("sender".into()));
    world.enroll(&sender);
    let mut cells = Vec::new();
    for harness in HARNESSES {
        let pane = world.tmux_pane();
        let who = recipient(harness, Loc::Tmux(format!("rcpt-{harness}"), pane));
        // Composer wherever the harness has a composer policy; claude and codex also report turn
        // edges, which make the wake gate's idle check answer (the other two retry on the ladder).
        cells.push(cell(
            &world,
            "named+tmux",
            &sender,
            &who,
            registry_wake(harness, true),
        ));
    }
    print_cells("named+tmux", &cells);
}

// Kills: an hc terminal of any harness that cannot enroll or message, and one told it has an idle
// wake with no mod channel (hc has no composer wake).
#[test]
fn hc_terminal_site_four_harnesses() {
    let fake = World::fake_with(&["t1", "t2", "t3", "t4", "t5"]);
    let world = World::new(Rig::standalone("cfgs-hc"), None, Some(fake));
    let sender = claude("s-sender", Loc::Terminal("t1".into()));
    world.enroll(&sender);
    let mut cells = Vec::new();
    for (n, harness) in HARNESSES.into_iter().enumerate() {
        let who = recipient(harness, Loc::Terminal(format!("t{}", n + 2)));
        cells.push(cell(
            &world,
            "hc",
            &sender,
            &who,
            registry_wake(harness, false),
        ));
    }
    print_cells("hc", &cells);
}

/// The additional context a Claude turn hook printed.
fn context_of(hook: &str) -> String {
    let value: Value =
        serde_json::from_str(hook).unwrap_or_else(|e| panic!("hook stdout: {e}: {hook}"));
    value["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap_or_else(|| panic!("hook context in {hook}"))
        .to_owned()
}

/// The delivery header line of message `id` in a turn hook's context.
fn header_of(context: &str, id: &str) -> String {
    context
        .lines()
        .find(|line| line.contains(&format!("message {id} ")))
        .unwrap_or_else(|| panic!("a header for {id} in {context}"))
        .to_owned()
}

/// One direction through the shared daemon: send with an ACK request, hook delivery whose header
/// names the sender's seat and label, then a second message read by `inbox` and ACKed by id.
fn direction(world: &World, from: &Caller, from_label: &str, to: &Caller) {
    let (sender, seat) = (world.seat(from).unwrap(), world.seat(to).unwrap());
    let body = format!("cross {} to {}", from.session, to.session);
    let sent = world.run(
        from,
        &[
            "send",
            "--seat",
            &seat,
            "--body",
            &body,
            "--require-ack",
            &seat,
        ],
        None,
    );
    assert_eq!(sent.code, Some(0), "{sent:?}");
    let id = sent_message(&sent);
    let context = context_of(&world.delivery(to, &body));
    let header = header_of(&context, &id);
    assert!(
        header.contains(&format!("from {sender} ")),
        "header for {} -> {}: {header}",
        from.session,
        to.session
    );
    // The sender's site label is the last column of its `seat list` row.
    let seats = world.run(to, &["seat", "list"], None);
    let row = seats
        .stdout
        .lines()
        .find(|line| line.starts_with(&sender))
        .unwrap_or_else(|| panic!("a seat row for {sender}: {seats:?}"));
    assert!(
        row.ends_with(&format!("/ {from_label}")),
        "label of {}: {row}",
        from.session
    );
    let owed = world.run(to, &["pending-receipts", "--machine"], None);
    assert!(!owed.stdout.contains(&id), "the display ACKed it: {owed:?}");

    let again = format!("{body} again");
    let second = world.run(
        from,
        &[
            "send",
            "--seat",
            &seat,
            "--body",
            &again,
            "--require-ack",
            &seat,
        ],
        None,
    );
    let second_id = sent_message(&second);
    let inbox = world.run(to, &["inbox"], None);
    assert!(
        inbox.stdout.contains(&again) && inbox.stdout.contains(&sender),
        "{inbox:?}"
    );
    let ack = world.run(to, &["ack", &second_id], None);
    assert_eq!(ack.code, Some(0), "{ack:?}");
    let owed = world.run(to, &["pending-receipts", "--machine"], None);
    assert!(!owed.stdout.contains(&second_id), "{owed:?}");
}

// Kills: any of hc->named, named->hc, hc->pane, pane->hc that fails to send, deliver, display or ACK
// through one daemon serving the Herdr host, the named host and the hc core, and a delivery header
// that names the sender by anything but its own seat and site label.
#[test]
fn cross_environment_messaging_one_daemon() {
    let Some(rig) = Rig::beside_herdr("cfgs-cross") else {
        return;
    };
    let world = World::new(rig, None, Some(World::fake_with(&["t1"])));
    let pane = world.rig.pane("cfgs-cross-pane");
    let hc = claude("s-hc", Loc::Terminal("t1".into()));
    let named = claude("s-named", Loc::Name("alice".into()));
    let paned = claude("s-pane", Loc::Pane(pane.clone()));
    for caller in [&hc, &named, &paned] {
        world.enroll(caller);
    }
    let hc_label = format!("{CORE}:t1");
    let pane_label = pane.clone();
    let labels = [
        (&hc, hc_label.as_str()),
        (&named, "alice"),
        (&paned, pane_label.as_str()),
    ];
    let label = |caller: &Caller| {
        labels
            .iter()
            .find(|(c, _)| c.session == caller.session)
            .unwrap()
            .1
    };
    for (from, to) in [(&hc, &named), (&named, &hc), (&hc, &paned), (&paned, &hc)] {
        direction(&world, from, label(from), to);
        println!(
            "direction {} -> {}: send, hook delivery, inbox, ACK ok",
            label(from),
            label(to)
        );
    }
}

// Kills: a herdr-app terminal (doc names, `herdr.space` / `herdr.tab` labels) that cannot be seated,
// message, ACK or take hook delivery like any hc terminal, and one displayed without its
// `<session>:tN` label or with the doc's names missing.
#[test]
fn herdr_app_terminal_full_messaging_and_label() {
    let fake = World::fake_with(&["t2"]);
    let mut app = FakeTerminal::live("t1");
    app.labels.insert("herdr.space".into(), "s1".into());
    app.labels.insert("herdr.tab".into(), "a".into());
    fake.add_terminal(app);
    fake.state().set_doc(
        "herdr-app",
        json!({"spaces": [{"id": "s1", "name": "Work", "folder": "/x",
            "tabs": [{"id": "a", "name": "first", "root": {}}]}]}),
        1,
    );
    let world = World::new(Rig::standalone("cfgs-app"), None, Some(fake));
    let app = claude("s-app", Loc::Terminal("t1".into()));
    let peer = claude("s-peer", Loc::Terminal("t2".into()));
    world.enroll(&app);
    world.enroll(&peer);
    let seats = world.run(&peer, &["seat", "list"], None);
    let row = seats
        .stdout
        .lines()
        .find(|line| line.starts_with(&world.seat(&app).unwrap()))
        .unwrap_or_else(|| panic!("the app terminal's row: {seats:?}"));
    assert!(
        row.ends_with("Work / first / alpha:t1"),
        "doc names and the session label: {row}"
    );
    let label = format!("{CORE}:t1");
    direction(&world, &app, &label, &peer);
    direction(&world, &peer, &format!("{CORE}:t2"), &app);
}

/// A running `watch` child of a claude caller (the mod's channel); stopped when dropped.
struct Channel {
    lines: std::sync::mpsc::Receiver<String>,
    _child: herdr_threads::test_support::spawn::OwnedChild,
}

impl Channel {
    fn start(world: &World, caller: &Caller) -> Self {
        use herdr_threads::test_support::spawn::SpawnOwned;
        use std::io::BufRead;
        let mut command = world.prepare(
            caller,
            &["watch", "--harness", "claude", "--session", &caller.session],
        );
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        let mut child = command.spawn_owned().expect("spawn the watch child");
        let stdout = child.stdout.take().unwrap();
        let (sender, lines) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout)
                .lines()
                .map_while(Result::ok)
            {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            lines,
            _child: child,
        }
    }

    fn until(&self, what: &str, want: impl Fn(&Value) -> bool) {
        let until = std::time::Instant::now() + Duration::from_secs(30);
        let mut seen = Vec::new();
        while let Some(left) = until.checked_duration_since(std::time::Instant::now()) {
            let Ok(line) = self.lines.recv_timeout(left) else {
                break;
            };
            if serde_json::from_str::<Value>(&line).is_ok_and(|value| want(&value)) {
                return;
            }
            seen.push(line);
        }
        panic!("no watch line for {what}; saw {seen:?}");
    }
}

// Kills: a live mod channel on a named or hc seat that still reads as having no wake (Channel is
// the answer there, so the sender gets no warning), and a channel that does not deliver.
#[test]
fn live_mod_channel_is_a_wake_on_named_and_hc_sites() {
    for (site, world, sender, rcpt) in [
        (
            "named",
            World::new(Rig::standalone("cfgs-chan-named"), None, None),
            claude("s-sender", Loc::Name("sender".into())),
            claude("s-rcpt", Loc::Name("rcpt".into())),
        ),
        (
            "hc",
            World::new(
                Rig::standalone("cfgs-chan-hc"),
                None,
                Some(World::fake_with(&["t1", "t2"])),
            ),
            claude("s-sender", Loc::Terminal("t1".into())),
            claude("s-rcpt", Loc::Terminal("t2".into())),
        ),
    ] {
        world.enroll(&sender);
        let seat = world.enroll(&rcpt);
        let channel = Channel::start(&world, &rcpt);
        channel.until("connected", |v| {
            v["kind"] == "status" && v["state"] == "connected"
        });
        let body = format!("via channel {site}");
        let sent = world.run(
            &sender,
            &[
                "send",
                "--seat",
                &seat,
                "--body",
                &body,
                "--require-ack",
                &seat,
            ],
            None,
        );
        assert_eq!(sent.code, Some(0), "{site}: {sent:?}");
        assert_eq!(
            no_wake_warnings(&sent),
            0,
            "{site}: a live channel wakes it: {sent:?}"
        );
        channel.until("the message", |v| v["body"] == body.as_str());
        println!("channel site={site} harness=claude wake=Channel send=ok delivery=ok");
    }
}
