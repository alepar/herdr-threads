//! ht-4is.8.17: `read THREAD --follow` through the installed executable, its
//! detached daemon and the private Herdr endpoint of `sweep.rs`. The follower
//! prints the recent tail IRC style with pane-name nicks, then only the
//! messages appended after it started, never reprinting; `--json --follow`
//! prints one JSON record per line. Ctrl-C ends it with status 0. Every run
//! is bounded by explicit deadlines.

use super::sweep::{FakeHost, Scratch, pane};
use herdr_threads::test_support::spawn::SpawnOwned;
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader},
    os::unix::fs::DirBuilderExt,
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc::{Receiver, channel},
    time::{Duration, Instant},
};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");

struct Plugin {
    state: PathBuf,
    host: PathBuf,
}
impl Plugin {
    fn command(&self) -> Command {
        let mut command = crate::scrubbed_command(BIN);
        command
            .arg("--state-dir")
            .arg(&self.state)
            .arg("--host-endpoint")
            .arg(&self.host)
            .env_remove("HERDR_PLUGIN_STATE_DIR")
            .env_remove("HERDR_SOCKET_PATH")
            .env_remove("HERDR_PANE_ID")
            .env_remove("HERDR_BIN_PATH")
            .env_remove("HERDR_ENV")
            .env("NO_COLOR", "1");
        command
    }
    /// Run as the stand-in agent `(seat, pane, harness)`, or with no caller.
    fn ok(&self, agent: Option<(&str, &str, &str)>, args: &[&str]) -> Value {
        let mut command = self.command();
        command.arg("--json");
        if let Some((seat, target, harness)) = agent {
            command.args([
                "--cooperative-seat",
                seat,
                "--cooperative-target",
                target,
                "--cooperative-harness",
                harness,
                "--cooperative-role",
                "top-level",
            ]);
        }
        command.args(args);
        let output = command.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        assert!(
            output.status.success(),
            "herdr-threads {args:?} failed: {}{stdout}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_str::<Value>(&stdout).unwrap()["result"].clone()
    }
    fn send(&self, agent: (&str, &str, &str), thread: &str, body: &str) -> String {
        self.ok(Some(agent), &["send", thread, "--body", body])["data"]
            .as_str()
            .unwrap()
            .to_owned()
    }
    fn follow(&self, args: &[&str]) -> Follower {
        self.follow_from(None, args)
    }
    fn follow_from(&self, pane: Option<&str>, args: &[&str]) -> Follower {
        let mut command = self.command();
        if let Some(pane) = pane {
            command.env("HERDR_PANE_ID", pane);
        }
        let mut child = command
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn_owned()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Follower {
            child,
            lines: rx,
            seen: Vec::new(),
        }
    }
}
impl Drop for Plugin {
    fn drop(&mut self) {
        let _ = self.command().args(["daemon", "stop"]).output();
    }
}

struct Follower {
    child: herdr_threads::test_support::spawn::OwnedChild,
    lines: Receiver<String>,
    seen: Vec<String>,
}
impl Follower {
    /// Collect lines until one contains `needle` (bounded).
    fn wait_for(&mut self, needle: &str) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if let Ok(line) = self.lines.recv_timeout(Duration::from_millis(200)) {
                let hit = line.contains(needle);
                self.seen.push(line);
                if hit {
                    return;
                }
            }
        }
        panic!(
            "follower never printed {needle:?}; saw:\n{}",
            self.seen.join("\n")
        );
    }
    /// Ctrl-C: the follower exits 0 (bounded).
    fn interrupt(&mut self) {
        // SAFETY: plain signal delivery to our own child process.
        unsafe { libc::kill(self.child.id() as libc::pid_t, libc::SIGINT) };
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert_eq!(status.code(), Some(0), "Ctrl-C exits 0: {status:?}");
                break;
            }
            assert!(Instant::now() < deadline, "follower ignored Ctrl-C");
            std::thread::sleep(Duration::from_millis(50));
        }
        while let Ok(line) = self.lines.recv_timeout(Duration::from_millis(200)) {
            self.seen.push(line);
        }
    }
    fn count(&self, needle: &str) -> usize {
        self.seen
            .iter()
            .filter(|line| line.contains(needle))
            .count()
    }
}
impl Drop for Follower {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn labeled(id: &str, terminal: &str, label: &str) -> Value {
    let mut value = pane(id, terminal);
    value["label"] = json!(label);
    value
}

#[test]
fn follow_prints_the_recent_tail_then_only_new_messages_irc_style() {
    let root = PathBuf::from(format!(
        "/private/tmp/htfo-{}",
        &uuid::Uuid::new_v4().simple().to_string()[..10]
    ));
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    let _scratch = Scratch(root.clone());
    let socket = root.join("herdr.sock");
    let _host = FakeHost::start(
        &socket,
        vec![
            labeled("w1:p1", "term-a", "alice"),
            labeled("w1:p2", "term-b", "mad-hatter"),
        ],
    );
    let plugin = Plugin {
        state: root.join("state"),
        host: socket,
    };
    plugin.ok(None, &["daemon", "ensure"]);
    let resolve = |pane: &str| {
        plugin.ok(None, &["seat", "resolve", "--pane", pane])["data"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let (alice_seat, hatter_seat) = (resolve("w1:p1"), resolve("w1:p2"));
    let alice = (alice_seat.as_str(), "w1:p1", "claude");
    let hatter = (hatter_seat.as_str(), "w1:p2", "codex");
    for agent in [alice, hatter] {
        plugin.ok(
            Some(agent),
            &["check-in", "--lifecycle-event", "agent-start"],
        );
    }
    let thread = plugin.ok(Some(alice), &["thread", "create", "--topic", "tea party"])["data"]
        .as_str()
        .unwrap()
        .to_owned();
    plugin.ok(Some(alice), &["invite", &thread, "--seat", &hatter_seat]);
    plugin.ok(Some(hatter), &["accept", &thread]);
    plugin.send(alice, &thread, "Why is a raven like a writing-desk?");

    // Outside a pane, parent scopes must remain visible.
    let mut outside = plugin.follow(&["read", &thread, "--follow", "--human", "--recent", "1"]);
    outside.wait_for("raven");
    assert!(
        outside
            .seen
            .iter()
            .any(|line| line.contains("<w1/w1:t1/alice·claude>")),
        "{:?}",
        outside.seen
    );
    outside.interrupt();

    // In the same pane scope, local nicks omit workspace and tab parents.
    let mut human = plugin.follow_from(
        Some("w1:p1"),
        &["read", &thread, "--follow", "--human", "--recent", "5"],
    );
    human.wait_for("raven");
    assert!(
        human
            .seen
            .iter()
            .any(|line| line.contains("Topic for") && line.contains("tea party")),
        "{:?}",
        human.seen
    );
    assert!(
        human.count("-!- mad-hatter·codex joined") == 1,
        "join notice with the pane nick and harness: {:?}",
        human.seen
    );
    let raven = human
        .seen
        .iter()
        .find(|line| line.contains("raven"))
        .unwrap()
        .clone();
    assert!(
        raven.starts_with('[') && raven.contains("] <alice·claude> Why is a raven"),
        "{raven}"
    );

    // Machine follower: one JSON record per line.
    let mut machine = plugin.follow(&["--json", "read", &thread, "--follow", "--recent", "1"]);
    machine.wait_for("raven");

    // Appends after the followers started are printed once each, in order.
    let long = format!(
        "I haven't the slightest idea. {}",
        "Twinkle, twinkle, little bat! ".repeat(20)
    );
    plugin.send(hatter, &thread, &long);
    plugin.send(alice, &thread, "Move down! Clean cups!");
    human.wait_for("Clean cups");
    machine.wait_for("Clean cups");
    human.interrupt();
    machine.interrupt();

    assert_eq!(human.count("raven"), 1, "never reprinted: {:?}", human.seen);
    assert_eq!(human.count("Clean cups"), 1, "{:?}", human.seen);
    let hatter_line = human
        .seen
        .iter()
        .position(|line| line.contains("<mad-hatter·codex> I haven't"))
        .unwrap_or_else(|| panic!("{:?}", human.seen));
    let cups_line = human
        .seen
        .iter()
        .position(|line| line.contains("<alice·claude> Move down!"))
        .unwrap();
    assert!(hatter_line < cups_line, "{:?}", human.seen);
    // The full (beyond-preview) body is shown, wrapped across lines.
    let bats: usize = human
        .seen
        .iter()
        .map(|line| line.matches("little bat!").count())
        .sum();
    assert_eq!(bats, 20, "{:?}", human.seen);
    assert!(
        human.seen.iter().all(|line| line.chars().count() <= 100),
        "wrapped at the default width: {:?}",
        human.seen
    );
    assert!(human.seen.iter().all(|line| !line.contains('\u{1b}')));

    let records: Vec<Value> = machine
        .seen
        .iter()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|_| panic!("not JSON: {line}")))
        .collect();
    let sequences: Vec<u64> = records
        .iter()
        .map(|record| record["sequence"].as_u64().unwrap())
        .collect();
    assert!(
        sequences.windows(2).all(|pair| pair[0] < pair[1]),
        "strictly increasing, never repeated: {sequences:?}"
    );
    assert!(
        records
            .iter()
            .any(|r| r["preview_data"].as_str().unwrap().contains("raven"))
    );
    assert!(
        records.last().unwrap()["preview_data"]
            .as_str()
            .unwrap()
            .contains("Clean cups")
    );

    // A daemon restart is reported once and followed through: the follower
    // reconnects to the newly published endpoint and keeps printing.
    let mut restarted = plugin.follow(&["read", &thread, "--follow", "--human", "--recent", "1"]);
    restarted.wait_for("Clean cups");
    plugin.ok(None, &["daemon", "stop"]);
    restarted.wait_for("lost the daemon");
    plugin.ok(None, &["daemon", "ensure"]);
    restarted.wait_for("reconnected to the daemon");
    plugin.send(hatter, &thread, "Off with their heads!");
    restarted.wait_for("Off with their heads!");
    restarted.interrupt();
    assert_eq!(restarted.count("Clean cups"), 1, "{:?}", restarted.seen);
    assert_eq!(
        restarted.count("lost the daemon"),
        1,
        "{:?}",
        restarted.seen
    );

    // Plain `read` from the same pane is the same IRC transcript, oldest first.
    let output = plugin
        .command()
        .env("HERDR_PANE_ID", "w1:p1")
        .args(["read", &thread, "--human", "--recent", "3"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    let cups = text
        .find("] <alice·claude> Move down! Clean cups!")
        .expect(&text);
    let heads = text
        .find("] <mad-hatter·codex> Off with their heads!")
        .expect(&text);
    assert!(cups < heads, "{text}");
}
