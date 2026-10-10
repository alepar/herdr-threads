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
            .env("HOME", self.state.parent().unwrap())
            .env(
                "CLAUDE_CONFIG_DIR",
                self.state.parent().unwrap().join("claude"),
            )
            .env("CODEX_HOME", self.state.parent().unwrap().join("codex"))
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
        "{}/htfo-{}",
        herdr_threads::test_support::SHORT_TMP,
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
            .any(|line| line.contains(&format!("<w1/alice/{alice_seat}>"))),
        "{:?}",
        outside.seen
    );
    outside.interrupt();

    // Agent nicks retain the space and seat ID even for a caller in that space.
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
        human.count(&format!("-!- w1/mad-hatter/{hatter_seat} joined")) == 1,
        "join notice with the space, pane name and seat ID: {:?}",
        human.seen
    );
    let raven = human
        .seen
        .iter()
        .find(|line| line.contains("raven"))
        .unwrap()
        .clone();
    assert!(
        raven.starts_with('[')
            && raven.contains(&format!("] <w1/alice/{alice_seat}> Why is a raven")),
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
        .position(|line| line.contains(&format!("<w1/mad-hatter/{hatter_seat}> I haven't")))
        .unwrap_or_else(|| panic!("{:?}", human.seen));
    let cups_line = human
        .seen
        .iter()
        .position(|line| line.contains(&format!("<w1/alice/{alice_seat}> Move down!")))
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
        .find(&format!("] <w1/alice/{alice_seat}> Move down! Clean cups!"))
        .expect(&text);
    let heads = text
        .find(&format!(
            "] <w1/mad-hatter/{hatter_seat}> Off with their heads!"
        ))
        .expect(&text);
    assert!(cups < heads, "{text}");
}

// Kills: alias skips canonical name resolution, accepts invitations, ACKs reads,
// ignores compatible options, or treats no-thread machine input as a picker.
#[test]
fn public_follow_alias_resolves_names_streams_and_leaves_obligations_untouched() {
    let root = PathBuf::from(format!(
        "{}/htfo-{}",
        herdr_threads::test_support::SHORT_TMP,
        &uuid::Uuid::new_v4().simple().to_string()[..10]
    ));
    fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
    let _scratch = Scratch(root.clone());
    let socket = root.join("herdr.sock");
    let _host = FakeHost::start(
        &socket,
        vec![
            labeled("w1:p1", "term-a", "alice"),
            labeled("w1:p2", "term-b", "bob"),
        ],
    );
    let plugin = Plugin {
        state: root.join("state"),
        host: socket,
    };
    plugin.ok(None, &["daemon", "ensure"]);
    let alice_seat = plugin.ok(None, &["seat", "resolve", "--pane", "w1:p1"])["data"]
        .as_str()
        .unwrap()
        .to_owned();
    let bob_seat = plugin.ok(None, &["seat", "resolve", "--pane", "w1:p2"])["data"]
        .as_str()
        .unwrap()
        .to_owned();
    let alice = (alice_seat.as_str(), "w1:p1", "claude");
    let bob = (bob_seat.as_str(), "w1:p2", "codex");
    for agent in [alice, bob] {
        plugin.ok(
            Some(agent),
            &["check-in", "--lifecycle-event", "agent-start"],
        );
    }
    let thread = plugin.ok(
        Some(alice),
        &[
            "thread",
            "create",
            "--name",
            "alias review",
            "--topic",
            "Follow alias",
        ],
    )["data"]
        .as_str()
        .unwrap()
        .to_owned();
    plugin.ok(Some(alice), &["invite", &thread, "--seat", &bob_seat]);
    let message = plugin.ok(
        Some(alice),
        &[
            "send",
            &thread,
            "--body",
            "alias initial",
            "--require-ack",
            &bob_seat,
        ],
    )["data"]
        .as_str()
        .unwrap()
        .to_owned();
    let participants_before = plugin.ok(None, &["participants", &thread]);
    let receipts_before = plugin.ok(None, &["delivery", "inspect", &message]);

    let mut exact = plugin.follow_from(
        Some("w1:p2"),
        &[
            "--machine",
            "follow",
            &thread,
            "--recent",
            "1",
            "--no-system",
            "--max-bytes",
            "512",
        ],
    );
    exact.wait_for("alias initial");
    let mut named = plugin.follow_from(
        Some("w1:p2"),
        &[
            "--json",
            "follow",
            "alias review",
            "--recent",
            "1",
            "--no-system",
        ],
    );
    named.wait_for("alias initial");
    let mut original = plugin.follow_from(
        Some("w1:p2"),
        &[
            "--json",
            "read",
            &thread,
            "--follow",
            "--recent",
            "1",
            "--no-system",
        ],
    );
    original.wait_for("alias initial");
    // A real topic event must be skipped by every --no-system follower.
    plugin.ok(
        Some(alice),
        &["thread", "topic", &thread, "--set", "Updated alias topic"],
    );
    plugin.send(alice, &thread, "alias appended");
    for follower in [&mut exact, &mut named, &mut original] {
        follower.wait_for("alias appended");
        follower.interrupt();
        assert_eq!(follower.count("alias initial"), 1);
        assert_eq!(follower.count("alias appended"), 1);
    }
    let parse_records = |lines: &[String]| {
        lines
            .iter()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>()
    };
    let named_records = parse_records(&named.seen);
    assert_eq!(named_records, parse_records(&original.seen));
    let machine_records = parse_records(&exact.seen);
    assert_eq!(machine_records.len(), 2);
    for (machine, json) in machine_records.iter().zip(&named_records) {
        // Continuation argv intentionally keeps each requested output format.
        for field in [
            "thread",
            "message",
            "sequence",
            "kind",
            "author",
            "preview_data",
        ] {
            assert_eq!(machine[field], json[field], "{field}");
        }
    }
    assert_eq!(named_records.len(), 2);
    assert!(
        named_records
            .iter()
            .all(|record| record["thread"].as_str() == Some(thread.as_str()))
    );
    let sequence = named_records[0]["sequence"].as_u64().unwrap().to_string();
    let mut after = plugin.follow(&[
        "--json",
        "follow",
        "alias review",
        "--after",
        &sequence,
        "--no-system",
    ]);
    after.wait_for("alias appended");
    after.interrupt();
    assert_eq!(after.count("alias initial"), 0);
    assert_eq!(after.count("alias appended"), 1);

    for args in [
        &["--machine", "follow"][..],
        &["--json", "follow"][..],
        &["follow"][..],
    ] {
        let output = plugin.command().args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{:?}", output);
        assert!(output.stdout.is_empty(), "{:?}", output);
    }
    assert_eq!(
        participants_before,
        plugin.ok(None, &["participants", &thread]),
        "read/follow must not accept the invitation"
    );
    assert_eq!(
        receipts_before,
        plugin.ok(None, &["delivery", "inspect", &message]),
        "read/follow must not ACK the requested receipt"
    );
}
