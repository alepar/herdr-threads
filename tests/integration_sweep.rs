//! ht-kqrt.66 (Integration sweep): the flows of the three-environment goal that no per-seam test
//! drives through a real daemon.
//!
//! The pane, plain named, named-in-tmux, hc and cross-environment flows are covered by
//! `config_smoke` (every site, four harnesses, `cross_environment_messaging_one_daemon`), the pane
//! and hc handoffs by `integration::named_handoff` and `hc_handoff`. What none of them observes is
//! the daemon-level tmux composer wake: an idle named recipient in a private tmux server is typed
//! into for a p0 item at T = 0 and left alone for a p1 item.

#[allow(dead_code)]
#[path = "named_seats/rig.rs"]
mod rig;

use herdr_threads::test_support::{isolation::as_agent, scratch, tmux::PrivateTmux};
use rig::{Ran, Rig, executable, run, sent_message, wait_for};
use std::{io::Write, path::PathBuf, time::Duration};

const IDLE: &str = include_str!("fixtures/screen_blocked/claude/idle.txt");
const SESSION: &str = "sweep-tmux-rcpt";
const NAME: &str = "rcpt";

fn payload(event: &str, extra: &str) -> Vec<u8> {
    format!(
        r#"{{"session_id":"{SESSION}","transcript_path":"/tmp/t.jsonl","cwd":"/tmp","hook_event_name":"{event}"{extra}}}"#
    )
    .into_bytes()
}

/// A pane that shows an idle Claude composer, then executes the calls handed to [`Pane::call`]
/// as its descendants (the daemon verifies a tmux hint against the caller's ancestry).
struct Recipient<'a> {
    rig: &'a Rig,
    tmux: &'a PrivateTmux,
    tmux_var: String,
    pane: herdr_threads::test_support::tmux::Pane,
    dir: PathBuf,
    calls: u64,
}

impl<'a> Recipient<'a> {
    fn new(rig: &'a Rig, tmux: &'a PrivateTmux) -> Self {
        let dir = rig.isolation.path("tmux-runner");
        std::fs::create_dir_all(&dir).unwrap();
        let mut mkfifo = std::process::Command::new("mkfifo");
        mkfifo.arg(dir.join("fifo"));
        assert_eq!(run(mkfifo, None).code, Some(0), "mkfifo");
        executable(
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
        let screen = scratch::temp_path("sweep-idle-screen");
        std::fs::write(&screen, IDLE).unwrap();
        let width = IDLE
            .lines()
            .map(|l| l.chars().count())
            .max()
            .unwrap()
            .max(40);
        let rows = (IDLE.lines().count() + 4).max(24);
        let pane = tmux.new_pane(&format!(
            "until [ \"$(stty size | cut -d' ' -f2)\" -ge {width} ]; do sleep 0.05; done; \
             cat {}; /bin/bash {1}/runner.sh {1}; true",
            screen.display(),
            dir.display()
        ));
        let resized = tmux.run(&[
            "resize-window",
            "-t",
            &pane.id,
            "-x",
            &width.to_string(),
            "-y",
            &rows.to_string(),
        ]);
        assert!(resized.status.success(), "resize-window");
        let this = Self {
            rig,
            tmux,
            tmux_var: format!("{},{},0", tmux.socket_path().display(), tmux.server_pid()),
            pane,
            dir,
            calls: 0,
        };
        wait_for("idle screen shown", Duration::from_secs(30), || {
            this.capture()
                .contains("manual mode on")
                .then_some(())
                .ok_or_else(|| this.capture())
        });
        this
    }

    fn capture(&self) -> String {
        let out = self.tmux.run(&["capture-pane", "-p", "-t", &self.pane.id]);
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// `args` as the recipient: a named seat in the tmux pane, run by the pane's own runner.
    fn call(&mut self, args: &[&str], stdin: &[u8]) -> Ran {
        let mut command = self.rig.command();
        command.args(args).env("HERDR_THREADS_SEAT_NAME", NAME);
        as_agent(&mut command, "claude", SESSION);
        let pid = self.pane.pid.to_string();
        command
            .env("TMUX", &self.tmux_var)
            .env("TMUX_PANE", &self.pane.id)
            .env("HT_TEST_HARNESS_PID", &pid)
            .env("CLAUDE_PID", &pid);
        let mut env: std::collections::BTreeMap<_, _> = std::env::vars_os().collect();
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
        self.calls += 1;
        let n = self.calls;
        std::fs::write(self.dir.join(format!("{n}.sh")), script).unwrap();
        std::fs::write(self.dir.join(format!("{n}.in")), stdin).unwrap();
        let mut fifo = std::fs::OpenOptions::new()
            .write(true)
            .open(self.dir.join("fifo"))
            .unwrap();
        writeln!(fifo, "{n}").unwrap();
        drop(fifo);
        let code = wait_for("pane call", Duration::from_secs(120), || {
            std::fs::read_to_string(self.dir.join(format!("{n}.code"))).map_err(|e| e.to_string())
        });
        let read =
            |ext: &str| std::fs::read_to_string(self.dir.join(format!("{n}.{ext}"))).unwrap();
        Ran {
            code: code.trim().parse().ok(),
            stdout: read("out"),
            stderr: read("err"),
        }
    }
}

// Kills: a named seat in a verified tmux pane whose idle composer is never typed into for a p0
// item (the wake dispatcher not reaching the named host), one typed into for a p1 item at T = 0
// (the threshold ignored on the named path), and a wake that does not carry the content-free marker.
#[test]
fn named_tmux_idle_recipient_wakes_for_p0_not_p1() {
    let Some(tmux) = PrivateTmux::start("sweep-tmux-wake") else {
        return;
    };
    let rig = Rig::standalone("sweep-tmux-wake");
    // The daemon's whole PATH is the rig's bin directory; hint verification runs `tmux`.
    let tmux_program = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|dir| dir.join("tmux"))
        .find(|path| path.is_file())
        .expect("tmux on PATH");
    executable(
        &rig.bin.join("tmux"),
        &format!("#!/bin/sh\nexec {} \"$@\"\n", tmux_program.display()),
    );
    let mut rcpt = Recipient::new(&rig, &tmux);

    // Enrollment: the lifecycle hook until the seat has an open claude binding.
    let seat = wait_for("recipient enrolled", Duration::from_secs(90), || {
        let started = rcpt.call(
            &["hook", "claude"],
            &payload("SessionStart", r#","source":"startup""#),
        );
        rcpt.call(
            &["hook", "claude"],
            &payload("UserPromptSubmit", r#","prompt":"go""#),
        );
        let seat: Result<String, _> = rig.db().query_row(
            "SELECT s.id FROM seats s JOIN occupant_bindings b ON b.seat_id=s.id \
             WHERE s.name=?1 AND b.harness='claude' AND b.ended_at IS NULL AND s.state<>'retired'",
            [NAME],
            |row| row.get(0),
        );
        seat.map_err(|e| format!("{e}: {started:?}"))
    });
    // The turn ends: the binding is idle, the composer is empty.
    let stop = rcpt.call(
        &["hook", "claude"],
        &payload("Stop", r#","stop_hook_active":false"#),
    );
    assert_eq!(stop.code, Some(0), "{stop:?}");
    let first_look = rcpt.call(&["inbox"], b"");
    assert_eq!(first_look.code, Some(0), "{first_look:?}");

    rig.enroll("sender", "sweep-tmux-sender");
    let sender = |args: &[&str]| rig.agent("sender", "sweep-tmux-sender", args);

    let before = rcpt.capture();
    assert!(!before.contains("herdr-threads:"), "{before}");

    // p1 at T = 0: no wake. Give the dispatcher a generous window to (wrongly) type.
    let quiet = sender(&["send", "--seat", &seat, "--body", "sweep p1 quiet"]);
    assert_eq!(quiet.code, Some(0), "{quiet:?}");
    sent_message(&quiet);
    std::thread::sleep(Duration::from_secs(6));
    let after_p1 = rcpt.capture();
    assert!(
        !after_p1.contains("herdr-threads:"),
        "a p1 item woke the recipient: {after_p1}"
    );

    // p0 (--nudge): the idle composer is typed into.
    let loud = sender(&[
        "send",
        "--seat",
        &seat,
        "--nudge",
        "--body",
        "sweep p0 wake",
    ]);
    assert_eq!(loud.code, Some(0), "{loud:?}");
    sent_message(&loud);
    assert!(
        !loud.stderr.contains("wake=no_wake"),
        "a verified tmux seat has a wake: {loud:?}"
    );
    let woken = wait_for(
        "p0 wake typed into the tmux composer",
        Duration::from_secs(90),
        || {
            let screen = rcpt.capture();
            screen
                .contains("herdr-threads:")
                .then_some(screen.clone())
                .ok_or(screen)
        },
    );
    // A hook-registered binding gets the content-free wake (R18), never the `inbox` marker.
    assert!(
        woken.contains(herdr_threads::notification::policy::WAKE_UP),
        "{woken}"
    );
    assert!(
        !woken.contains(herdr_threads::notification::policy::MARKER),
        "{woken}"
    );
}
