//! ht-4is.8.18: the agent-facing machine text form stays small. A 20-message
//! thread read through the installed executable (non-TTY, so the machine
//! form an agent sees) is bounded per line, prints at most one continuation,
//! carries short `c3:` cursors, and the inbox, pending-receipts and check-in
//! forms carry no JSON page blobs. Set `HT_TOKEN_DIET_OUT=<dir>` to keep the
//! rendered samples for byte measurements.

use super::sweep::{FakeHost, Scratch, pane};
use serde_json::{Value, json};
#[path = "../support/lazy_proxy.rs"]
pub(super) mod lazy_proxy;
use herdr_threads::daemon::paths::{NodePaths, RuntimeContext};
use std::sync::atomic::Ordering;
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
            herdr_threads::test_support::isolation::as_agent(
                &mut command,
                harness,
                &format!("diet-{}", seat),
            );
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

/// D8: a priority-0 delivery-row send (no ACK recipient) carries one `[p0] <id>`
/// annotation row (4 bytes, a space, the 9-byte id, a newline: 15 bytes) in body,
/// read and search output. The conversation in this fixture names an ACK recipient
/// in every send, so its ACK lines mark it and it gets no row (ht-yha.66); the
/// probes sent at the end are the p0 delivery-row sends.
fn p0_row(id: &str) -> String {
    format!("[p0] {id}\n")
}

/// Splits the leading `[p0] <message id>` annotation rows off a history read
/// and returns the annotated ids with the text from the `history` kind line on.
fn split_p0_rows(text: &str) -> (Vec<&str>, &str) {
    let mut ids = Vec::new();
    let mut rest = text;
    while let Some(row) = rest.strip_prefix("[p0] ") {
        let (id, tail) = row.split_once('\n').expect("annotation row ends");
        assert!(
            herdr_threads::protocol::ids::is_public_id(
                herdr_threads::protocol::ids::prefix::MESSAGE,
                id,
            ),
            "{text}"
        );
        ids.push(id);
        rest = tail;
    }
    (ids, rest)
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
        "{}/httd-{}",
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
        let tracked = from.0 == alice_seat && index % 8 == 0;
        // Only an explicit ACK recipient owes a receipt (D9): the other participant is named.
        let other = if from.0 == alice_seat {
            hatter_seat.as_str()
        } else {
            alice_seat.as_str()
        };
        if !tracked {
            args.push("--nudge");
        }
        args.extend(["--require-ack", other]);
        let message = plugin.ok(Some(from), &args)["data"]
            .as_str()
            .unwrap()
            .to_owned();
        if index == 2 {
            long_message = message.clone();
        }
        if tracked && index < 10 {
            acked.push(message);
        }
    }
    // One ACK so the timeline carries an info event, and one receipt stays pending.
    plugin.ok(Some(hatter), &["ack", &acked[0]]);

    let read = plugin.text(Some(hatter), &["read", &thread, "--recent", "20"]);
    keep("read-recent-20.txt", &read);
    // Retain the legacy compact byte contract on supported wire6 with
    // pre-lazy capabilities. Current v2 machine bodies are covered separately.
    let inbox = {
        let paths = NodePaths::resolve(
            &RuntimeContext::explicit(plugin.state.clone(), plugin.host.clone(), None)
                .unwrap()
                .state_dir,
        )
        .unwrap();
        let proxy = lazy_proxy::Proxy::new(paths, &root);
        proxy.mode.store(1, Ordering::SeqCst);
        let text = plugin.text(Some(hatter), &["inbox"]);
        let requests = proxy.requests();
        assert!(requests.iter().any(|q| q["command"]["kind"] == "inbox"));
        assert!(
            !requests
                .iter()
                .any(|q| q["command"]["kind"] == "inbox_batch_v2")
        );
        drop(proxy); // join and restore the daemon before any subsequent read
        text
    };
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
    // An ACK send carries no `[p0]` row (ht-yha.66); the 256 floor still fits the clipped body.
    let body_budget = "256";
    let body_paged = plugin.text(
        Some(hatter),
        &["body", &long_message, "--max-bytes", body_budget],
    );
    keep("body-paged.txt", &body_paged);

    // body: one header row, the full body verbatim and indented, no JSON, and no
    // `[p0]` row for an ACK send (its ACK line marks it).
    assert!(!body.starts_with("[p"), "{body}");
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
    assert!(!body_paged.starts_with("[p"), "{body_paged}");
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

    // read --recent 20: no `[p0] <id>` row (every fixture message names an ACK
    // recipient, so its ACK line marks it: ht-yha.66), then the kind line, one row
    // per entry, one `next:` line.
    let (annotated, history) = split_p0_rows(&read);
    assert!(annotated.is_empty(), "{read}");
    let lines: Vec<&str> = history.lines().collect();
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
            && herdr_threads::protocol::ids::is_public_id(
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
        herdr_threads::protocol::ids::is_public_id(
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
    let (rest_annotated, rest_history) = split_p0_rows(&rest);
    assert!(rest_history.starts_with("history\n#4 "), "{rest}");
    assert!(rest_annotated.is_empty(), "{rest}");
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
                herdr_threads::protocol::ids::is_public_id(
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
        show.contains(&format!("{hatter_seat} joined active self\n")),
        "{show}"
    );
    assert!(!show.contains("native_observation"), "{show}");
    let canonical =
        format!("participants\n{alice_seat} joined active\n{hatter_seat} joined active self\n");
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

    // A small --max-bytes bound still returns a p0 message (ht-yha.50): the refit
    // reserves only the `[p0] <id>` row, so body/read/search work from the floor up.
    // The probes are p0 delivery-row sends (no ACK recipient), the marked kind.
    let long_probe = plugin.ok(
        Some(alice),
        &["send", &thread, "--body", long.as_str(), "--nudge"],
    )["data"]
        .as_str()
        .unwrap()
        .to_owned();
    let probe = plugin.ok(
        Some(alice),
        &["send", &thread, "--body", "tiny-p0-probe", "--nudge"],
    )["data"]
        .as_str()
        .unwrap()
        .to_owned();
    for bound in [256, 257, 270, 271, 272, 300, 341, 342] {
        let bound_arg = bound.to_string();
        let body = plugin.text(
            Some(hatter),
            &["body", &long_probe, "--max-bytes", &bound_arg],
        );
        assert!(body.len() <= bound, "body {bound}: {body}");
        assert!(
            body.starts_with(&p0_row(&long_probe)),
            "body {bound}: {body}"
        );
        let header = body
            .lines()
            .find(|l| l.starts_with('#') && l.contains(&format!(" {long_probe} ")))
            .unwrap_or_else(|| panic!("body {bound} header: {body}"));
        let range = header.split("bytes=").nth(1);
        match range {
            Some(range) => {
                let end: usize = range
                    .split('/')
                    .next()
                    .unwrap()
                    .strip_prefix("0-")
                    .unwrap_or_else(|| panic!("body {bound} range: {body}"))
                    .parse()
                    .unwrap();
                assert!(end > 0, "body {bound}: {body}");
            }
            None => assert!(body.lines().count() >= 3, "body {bound} full: {body}"),
        }
        let read = plugin.text(
            Some(hatter),
            &["read", &thread, "--recent", "1", "--max-bytes", &bound_arg],
        );
        assert!(read.len() <= bound, "read {bound}: {read}");
        assert!(read.starts_with("[p0] "), "read {bound}: {read}");
        assert!(read.contains(&probe), "read {bound}: {read}");
    }
    // A one-hit search page needs 592 bytes from the daemon even unannotated
    // (`search match cannot fit`, required minimum 592; 574 on 8-character ids, and the
    // 3-character node tag adds 18 bytes over its 6 id occurrences: message, the
    // `preview_detail` body argv, author, event_author, thread and the continuation's
    // `--thread`), so search is measured from there: minimum, minimum + 1, minimum + the
    // 18-byte `[p0] <id>\n` row, and a roomier bound. The `[p0]` row never pushes a
    // fitting page into a refusal.
    for bound in [592, 593, 610, 618] {
        let bound_arg = bound.to_string();
        let search = plugin.text(
            Some(hatter),
            &[
                "search",
                "tiny-p0-probe",
                "--thread",
                &thread,
                "--max-bytes",
                &bound_arg,
            ],
        );
        assert!(search.len() <= bound, "search {bound}: {search}");
        assert!(search.starts_with("[p0] "), "search {bound}: {search}");
        assert!(search.contains(&probe), "search {bound}: {search}");
    }
}
