//! ht-4is.8.18: the agent-facing machine text form stays small. A 20-message
//! thread read through the installed executable (non-TTY, so the machine
//! form an agent sees) is bounded per line, prints at most one continuation,
//! carries short `c3:` cursors, and the inbox, pending-receipts and check-in
//! forms carry no JSON page blobs. Set `HT_TOKEN_DIET_OUT=<dir>` to keep the
//! rendered samples for byte measurements.

use super::sweep::{FakeHost, Scratch, pane};
use serde_json::{Value, json};
use std::{fs, os::unix::fs::DirBuilderExt, path::PathBuf, process::Command};

const BIN: &str = env!("CARGO_BIN_EXE_herdr-threads");

struct Plugin {
    state: PathBuf,
    host: PathBuf,
}
impl Plugin {
    fn command(&self, agent: Option<(&str, &str, &str)>, json: bool) -> Command {
        // The plugin environment an agent's pane provides, so continuation
        // commands carry no explicit selectors (as in a real pane).
        let mut command = crate::scrubbed_command(BIN);
        command
            .env("HERDR_PLUGIN_STATE_DIR", &self.state)
            .env("HERDR_SOCKET_PATH", &self.host)
            .env_remove("HERDR_PANE_ID")
            .env_remove("HERDR_BIN_PATH")
            .env_remove("HERDR_ENV")
            .env("NO_COLOR", "1");
        if json {
            command.arg("--json");
        }
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
        command
    }
    fn ok(&self, agent: Option<(&str, &str, &str)>, args: &[&str]) -> Value {
        let mut command = self.command(agent, true);
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
    /// The machine text an agent's tool call sees (stdout is a pipe).
    fn text(&self, agent: Option<(&str, &str, &str)>, args: &[&str]) -> String {
        let output = self
            .command(agent, false)
            .arg("--machine")
            .args(args)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        assert!(
            output.status.success(),
            "herdr-threads {args:?} failed: {}{stdout}",
            String::from_utf8_lossy(&output.stderr)
        );
        stdout
    }
}
impl Drop for Plugin {
    fn drop(&mut self) {
        let _ = self.command(None, true).args(["daemon", "stop"]).output();
    }
}

fn labeled(id: &str, terminal: &str, label: &str) -> Value {
    let mut value = pane(id, terminal);
    value["label"] = json!(label);
    value
}

fn keep(name: &str, text: &str) {
    if let Some(dir) = std::env::var_os("HT_TOKEN_DIET_OUT") {
        let dir = PathBuf::from(dir);
        let _ = fs::create_dir_all(&dir);
        fs::write(dir.join(name), text).unwrap();
    }
}

#[test]
fn agent_facing_machine_text_is_compact() {
    let root = PathBuf::from(format!(
        "/private/tmp/httd-{}",
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
    // A second thread with a pending invitation for the hatter's inbox.
    let second = plugin.ok(Some(alice), &["thread", "create", "--topic", "croquet"])["data"]
        .as_str()
        .unwrap()
        .to_owned();
    plugin.ok(Some(alice), &["invite", &second, "--seat", &hatter_seat]);
    // ht-4is.3.12: inviting a seat that already joined is a settled no-op,
    // exit 0 in either form, and leaves no pending intent behind.
    let again = plugin.text(Some(alice), &["invite", &thread, "--seat", &hatter_seat]);
    assert_eq!(
        again,
        format!("already_joined {thread} {hatter_seat}: no invitation sent\n")
    );
    let again = plugin.ok(Some(alice), &["invite", &thread, "--seat", &hatter_seat]);
    assert_eq!(again["kind"], "already_joined", "{again}");
    let ops = plugin.ok(None, &["pending-ops"]);
    assert_eq!(ops["data"]["items"], json!([]), "{ops}");

    let long = format!(
        "Begin at the beginning and go on till you come to the end. {}",
        "Then stop. ".repeat(40)
    );
    let mut acked = Vec::new();
    let mut long_message = String::new();
    for index in 0..20 {
        let (from, body) = match index % 4 {
            0 => (
                alice,
                format!("Why is a raven like a writing-desk? (#{index})"),
            ),
            1 => (hatter, format!("I haven't the slightest idea (#{index})")),
            2 => (alice, long.clone()),
            _ => (hatter, format!("Twinkle, twinkle, little bat! (#{index})")),
        };
        let mut args = vec!["send", thread.as_str(), "--body", body.as_str()];
        if from.0 == alice_seat && index % 8 == 0 {
            args.extend(["--require-ack", hatter_seat.as_str()]);
        }
        let message = plugin.ok(Some(from), &args)["data"]
            .as_str()
            .unwrap()
            .to_owned();
        if index == 2 {
            long_message = message.clone();
        }
        if args.len() > 4 && index < 10 {
            acked.push(message);
        }
    }
    // One ACK so the timeline carries an info event, and one receipt stays pending.
    plugin.ok(Some(hatter), &["ack", &acked[0]]);

    let read = plugin.text(Some(hatter), &["read", &thread, "--recent", "20"]);
    keep("read-recent-20.txt", &read);
    let inbox = plugin.text(Some(hatter), &["inbox"]);
    keep("inbox.txt", &inbox);
    let pending = plugin.text(Some(hatter), &["pending-receipts", "--seat", &hatter_seat]);
    keep("pending-receipts.txt", &pending);
    let check_in = plugin.text(Some(hatter), &["check-in"]);
    keep("check-in.txt", &check_in);
    let show = plugin.text(Some(hatter), &["thread", "show", &thread]);
    keep("thread-show.txt", &show);
    let participants = plugin.text(Some(hatter), &["thread", "participants", &thread]);
    keep("thread-participants.txt", &participants);
    // A one-row page forces a continuation.
    let paged = plugin.text(Some(hatter), &["read", &thread, "--recent", "3"]);
    keep("read-recent-3.txt", &paged);
    let body = plugin.text(Some(hatter), &["body", &long_message]);
    keep("body.txt", &body);
    let body_paged = plugin.text(Some(hatter), &["body", &long_message, "--max-bytes", "256"]);
    keep("body-paged.txt", &body_paged);

    // body: one header row, the full body verbatim and indented, no JSON.
    let body_lines: Vec<&str> = body.lines().collect();
    assert_eq!(body_lines.len(), 2, "{body}");
    assert!(
        body_lines[0].starts_with(&format!("#6 {long_message} {alice_seat} "))
            && body_lines[0].ends_with('Z'),
        "{body}"
    );
    assert_eq!(body_lines[1], format!("  {long}"));
    // A clipped body names its byte range and ends with one `more:` line
    // that runs as printed.
    let paged_lines: Vec<&str> = body_paged.lines().collect();
    assert!(paged_lines[0].contains(" bytes=0-"), "{body_paged}");
    let more = paged_lines
        .last()
        .unwrap()
        .strip_prefix("more: ")
        .expect("more line");
    let args: Vec<&str> = more.split_whitespace().skip(1).collect();
    let rest = plugin.text(Some(hatter), &args);
    assert!(rest.starts_with("#6 "), "{rest}");
    let json = plugin.ok(Some(hatter), &["body", &long_message]);
    assert_eq!(json["data"]["content"]["body_data"], json!(long));

    // read --recent 20: kind line, one row per entry, one `next:` line.
    let lines: Vec<&str> = read.lines().collect();
    assert_eq!(lines[0], "history", "{read}");
    assert_eq!(lines.len(), 22, "{read}");
    assert_eq!(
        lines.iter().filter(|l| l.starts_with("next: ")).count(),
        1,
        "{read}"
    );
    assert!(
        !read.contains("page: ") && !read.contains("\"created_at\""),
        "{read}"
    );
    assert!(
        lines[1].starts_with("#24 ack ") && lines[1].contains(&hatter_seat),
        "{read}"
    );
    let message_id = lines[2].split_whitespace().nth(1).unwrap();
    assert!(
        lines[2].starts_with("#23 ")
            && herdr_threads::protocol::ids::is_short_public_id(
                herdr_threads::protocol::ids::prefix::MESSAGE,
                message_id,
            )
            && lines[2].ends_with(": Twinkle, twinkle, little bat! (#19)"),
        "{read}"
    );
    let clipped = lines
        .iter()
        .find(|l| l.contains("[more: "))
        .expect("clipped row");
    let clipped_id = clipped
        .split_once("[more: herdr-threads body ")
        .and_then(|(_, rest)| rest.split_once(']'))
        .map(|(id, _)| id)
        .unwrap();
    assert!(
        herdr_threads::protocol::ids::is_short_public_id(
            herdr_threads::protocol::ids::prefix::MESSAGE,
            clipped_id,
        ),
        "{clipped}"
    );
    for row in &lines[2..21] {
        assert!(
            row.starts_with('#') && row.contains("Z") && row.contains(": "),
            "{row}"
        );
        assert!(row.len() < 400, "{row}");
    }
    let next = lines[21].strip_prefix("next: ").unwrap();
    assert!(
        !next.contains("--json"),
        "a text read continues in text: {next}"
    );
    let cursor = next
        .split_whitespace()
        .skip_while(|w| *w != "--cursor")
        .nth(1)
        .unwrap();
    assert!(cursor.starts_with("c3:") && cursor.len() <= 24, "{cursor}");
    assert!(read.len() < 3_500, "{} bytes", read.len());

    // The continuation runs exactly as printed and reaches the oldest rows.
    let args: Vec<&str> = next.split_whitespace().skip(1).collect();
    let rest = plugin.text(Some(hatter), &args);
    assert!(rest.starts_with("history\n#4 "), "{rest}");
    assert!(!rest.contains("next: "), "{rest}");
    // The same cursor on another thread is refused as an invalid cursor.
    let wrong = plugin
        .command(Some(hatter), false)
        .args(["read", &second, "--cursor", cursor])
        .output()
        .unwrap();
    assert!(!wrong.status.success());
    assert!(
        String::from_utf8_lossy(&wrong.stderr).contains("cursor"),
        "{}",
        String::from_utf8_lossy(&wrong.stderr)
    );
    // --json keeps the full structured page.
    let json = plugin.ok(Some(hatter), &["read", &thread, "--recent", "3"]);
    assert_eq!(json["kind"], "history");
    assert!(
        json["data"]["next_cursor"]
            .as_str()
            .unwrap()
            .starts_with("c3:")
    );
    assert!(json["data"]["next_argv"].is_array());
    assert!(json["data"]["items"][0]["created_at"].is_number());

    assert_eq!(
        inbox,
        format!("inbox\n{thread} receipts=9\n{second} invitations=1\n"),
    );
    assert!(
        pending.starts_with(&format!("pending_receipts {hatter_seat}\n")),
        "{pending}"
    );
    assert_eq!(pending.lines().count(), 10, "{pending}");
    assert!(
        pending.lines().skip(1).all(|l| {
            l.split_whitespace().next().is_some_and(|id| {
                herdr_threads::protocol::ids::is_short_public_id(
                    herdr_threads::protocol::ids::prefix::MESSAGE,
                    id,
                )
            }) && l.contains(&format!(" from {alice_seat} due "))
        }),
        "{pending}"
    );
    assert!(
        check_in.starts_with(&format!("checked_in {hatter_seat} codex top_level ")),
        "{check_in}"
    );
    assert!(
        check_in.contains(&format!("inbox:\n{thread} receipts=9\n")),
        "{check_in}"
    );
    assert!(!check_in.contains('{'), "{check_in}");
    assert!(
        show.starts_with(&format!("thread {thread} messages=24 ")),
        "{show}"
    );
    assert!(
        show.contains(&format!("{hatter_seat} joined self\n")),
        "{show}"
    );
    assert!(!show.contains("native_observation"), "{show}");
    let canonical = format!("participants\n{alice_seat} joined\n{hatter_seat} joined self\n");
    assert!(participants.starts_with(&canonical), "{participants}");
    let hints: Vec<_> = participants[canonical.len()..].lines().collect();
    assert_eq!(hints.len(), 2, "{participants}");
    for (hint, seat, target) in [
        (hints[0], &alice_seat, "w1:p1"),
        (hints[1], &hatter_seat, "w1:p2"),
    ] {
        assert!(
            hint.starts_with(&format!("location {seat}: "))
                && hint.contains(target)
                && hint.ends_with("[advisory]"),
            "{hint}"
        );
    }
    // Advisory rows fit the compact budget and never crowd out canonical rows.
    assert!(participants.len() <= 256, "{} bytes", participants.len());
    let bounded = plugin.text(
        Some(hatter),
        &["thread", "participants", &thread, "--max-bytes", "256"],
    );
    assert!(bounded.starts_with(&canonical), "{bounded}");
    assert!(bounded.len() <= 256, "{} bytes", bounded.len());
    assert!(show.len() < 1_000, "{} bytes", show.len());
    assert!(paged.ends_with(" --limit 3\n"), "{paged}");
}
