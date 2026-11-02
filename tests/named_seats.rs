//! ht-kqrt.34 (N-e2e): named seats end to end through one daemon. A real `daemon run` child over
//! a scratch state directory and the real binary as the agent stand-in; the Herdr cases add one
//! isolated, named Herdr server whose socket the same daemon is given. No `herdr` is on the
//! `PATH` of the standalone cases. Every child is a `test_support` owned child and every test
//! stops its daemon and Herdr session on drop.

#[allow(dead_code)]
#[path = "named_seats/rig.rs"]
mod rig;

use herdr_threads::test_support::harness_shim;
use rig::{BIN, Ran, Rig, run, sent_message, wait_for};
use std::{path::Path, time::Duration};

const M8: &str = include_str!("cli/golden/m8.txt");

fn two_named(rig: &Rig) -> (String, String) {
    rig.enroll("alice", "s-alice");
    rig.enroll("bob", "s-bob");
    (rig.seat_of("alice"), rig.seat_of("bob"))
}

const START: &str = r#"{"session_id":"SESSION","transcript_path":"/tmp/t.jsonl","cwd":"/tmp","hook_event_name":"SessionStart","source":"startup"}"#;

/// A scratch shell function file: `enroll SESSION` runs the SessionStart hook until it enrolls.
fn enroll_script(rig: &Rig) -> std::path::PathBuf {
    let path = rig.isolation.path("enroll.sh");
    rig::executable(
        &path,
        r#"#!/bin/bash
payload=${HT_START//SESSION/$CLAUDE_CODE_SESSION_ID}
for ((i=0;i<120;i++)); do
  out=$(printf '%s' "$payload" | "$HT_BIN" --state-dir "$HT_STATE" hook claude 2>&1)
  case "$out" in *"Your seat"*) echo enrolled; exit 0;; esac
  /bin/sleep 0.5
done
echo "$out"; exit 1
"#,
    );
    path
}

/// `program args` as a child of a claude-shaped process (real ancestry, a harness process that
/// exits with the call), as `name` in `session`.
fn shimmed(rig: &Rig, name: &str, session: &str, program: &str, args: &[&str]) -> Ran {
    let mut command = harness_shim::under_harness("claude", Path::new(program), args, &[]);
    herdr_threads::test_support::isolation::scrub_env(&mut command);
    command
        .env("PATH", &rig.bin)
        .env("HOME", rig.isolation.home())
        .env("HT_BIN", BIN)
        .env("HT_STATE", &rig.state)
        .env("HT_START", START)
        .env("HERDR_THREADS_SEAT_NAME", name)
        .env("CLAUDE_CODE_SESSION_ID", session);
    run(command, None)
}

// Kills: a first named `inbox` that prints no M8 or prints it twice, a second process of the
// same name that creates a second seat or is refused (M7) when the first has exited, and a
// descendant of the holder that is refused or minted another seat. No `herdr` is on PATH.
#[test]
fn first_inbox_prints_m8_once_then_same_seat() {
    let rig = Rig::standalone("ns-m8");
    assert!(
        !rig.bin.join("herdr").exists(),
        "no herdr on the children's PATH"
    );
    let enroll = enroll_script(&rig);
    let enroll = enroll.to_str().unwrap();
    let first = shimmed(
        &rig,
        "reviewer",
        "s-one",
        BIN,
        &["--state-dir", rig.state.to_str().unwrap(), "inbox"],
    );
    assert_eq!(first.code, Some(0), "{first:?}");
    let seat = rig.seat_of("reviewer");
    assert_eq!(
        first.stderr.replace(&seat, "seat-Xy98Zw76"),
        M8,
        "M8 once: {first:?}"
    );

    // Started after the first process exited (its harness process is gone): it enrolls and binds
    // the same seat.
    let after = shimmed(&rig, "reviewer", "s-two", enroll, &[]);
    assert_eq!(after.code, Some(0), "{after:?}");
    assert_eq!(rig.seat_count(), 1);
    assert_eq!(rig.seat_of("reviewer"), seat);

    // A descendant of a live holder: a second claude-shaped process under the live claude process
    // of session s-three. The holder stays alive across both enrollments.
    let script = r#"
/bin/bash "$HT_ENROLL" >"$HT_OUT/holder.out" 2>&1; echo $? >"$HT_OUT/holder.code"
( export CLAUDE_CODE_SESSION_ID=s-child
  exec -a claude "$BASH" -c 'export CLAUDE_PID=$$; /bin/bash "$HT_ENROLL" >"$HT_OUT/child.out" 2>&1; echo $? >"$HT_OUT/child.code"' ht-shim )
"#;
    let out = rig.isolation.path("desc");
    std::fs::create_dir_all(&out).unwrap();
    let mut command =
        harness_shim::under_harness("claude", Path::new("/bin/bash"), &["-c", script], &[]);
    herdr_threads::test_support::isolation::scrub_env(&mut command);
    command
        .env("PATH", &rig.bin)
        .env("HOME", rig.isolation.home())
        .env("HT_BIN", BIN)
        .env("HT_STATE", &rig.state)
        .env("HT_START", START)
        .env("HT_ENROLL", enroll)
        .env("HT_OUT", &out)
        .env("HERDR_THREADS_SEAT_NAME", "reviewer")
        .env("CLAUDE_CODE_SESSION_ID", "s-three");
    let ran = run(command, None);
    let read = |name: &str| std::fs::read_to_string(out.join(name)).unwrap_or_default();
    for who in ["holder", "child"] {
        assert_eq!(
            read(&format!("{who}.code")).trim(),
            "0",
            "{who}: {ran:?} {}",
            read(&format!("{who}.out"))
        );
    }
    assert_eq!(rig.seat_count(), 1, "one seat for all four processes");
    let rows: Vec<(String, bool)> = {
        let db = rig.db();
        let mut statement = db
            .prepare("SELECT native_session,liveness='connected' FROM occupant_bindings WHERE seat_id=?1 ORDER BY ordinal")
            .unwrap();
        statement
            .query_map([&seat], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    for session in ["s-two", "s-three", "s-child"] {
        assert!(
            rows.iter().any(|(s, _)| s == session),
            "{session} bound the seat: {rows:?}"
        );
    }
}

// Kills: a named send, inbox display, ACK or turn-hook delivery that fails between two named
// seats, and an ACK that never settles the sender's obligation.
#[test]
fn named_to_named_send_inbox_ack_hook() {
    let rig = Rig::standalone("ns-n2n");
    let (alice, bob) = two_named(&rig);
    let sent = rig.agent(
        "alice",
        "s-alice",
        &[
            "send",
            "--seat",
            &bob,
            "--body",
            "hello bob",
            "--require-ack",
            &bob,
        ],
    );
    assert_eq!(sent.code, Some(0), "{sent:?}");
    let id = sent_message(&sent);

    // Hook delivery: bob's turn-start hook carries the item into his context.
    let context = rig.hook_context("hello bob", || rig.named_prompt_hook("bob", "s-bob"));
    assert!(context.contains(&id), "{context}");

    // The display ACKed it: nothing is owed.
    let owed = rig.agent("bob", "s-bob", &["pending-receipts", "--machine"]);
    assert!(!owed.stdout.contains(&id), "{owed:?}");

    // A second item: inbox shows it from alice; an explicit ACK settles the request.
    let second = rig.agent(
        "alice",
        "s-alice",
        &[
            "send",
            "--seat",
            &bob,
            "--body",
            "second",
            "--require-ack",
            &bob,
        ],
    );
    assert_eq!(second.code, Some(0), "{second:?}");
    let second_id = sent_message(&second);
    let owed = rig.agent("bob", "s-bob", &["pending-receipts", "--machine"]);
    assert!(owed.stdout.contains(&second_id), "{owed:?}");
    let inbox = rig.agent("bob", "s-bob", &["inbox"]);
    assert!(inbox.stdout.contains("second"), "{inbox:?}");
    assert!(inbox.stdout.contains(&alice), "{inbox:?}");
    let ack = rig.agent("bob", "s-bob", &["ack", &second_id]);
    assert_eq!(ack.code, Some(0), "{ack:?}");
    let owed = rig.agent("bob", "s-bob", &["pending-receipts", "--machine"]);
    assert!(!owed.stdout.contains(&second_id), "{owed:?}");

    // And back: bob replies, alice reads it.
    let reply = rig.agent(
        "bob",
        "s-bob",
        &["send", "--seat", &alice, "--body", "hello alice"],
    );
    assert_eq!(reply.code, Some(0), "{reply:?}");
    let back = rig.agent("alice", "s-alice", &["inbox"]);
    assert!(back.stdout.contains("hello alice"), "{back:?}");
}

// Kills: a named seat and a pane seat that cannot message each other through one daemon (either
// direction), an ACK across the two kinds that never settles, a pane hook that does not deliver
// the named sender's item, and a second daemon.
#[test]
fn named_to_pane_in_isolated_herdr() {
    let Some(rig) = Rig::beside_herdr("ns-n2p") else {
        return;
    };
    let pane = rig.pane("ns-n2p");
    rig.enroll("alice", "s-alice");
    rig.enroll_pane(&pane, "s-pane");
    let (alice, pane_seat) = (rig.seat_of("alice"), rig.seat_of_pane(&pane));
    assert_ne!(alice, pane_seat);

    let kinds: Vec<String> = {
        let db = rig.db();
        let mut statement = db
            .prepare("SELECT kind FROM herdr_hosts ORDER BY kind")
            .unwrap();
        statement
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert!(
        kinds.iter().any(|k| k == "named") && kinds.iter().any(|k| k != "named"),
        "one daemon serves the named host and the Herdr host: {kinds:?}"
    );

    // named -> pane, with an ACK request.
    let sent = rig.agent(
        "alice",
        "s-alice",
        &[
            "send",
            "--seat",
            &pane_seat,
            "--body",
            "to the pane",
            "--require-ack",
            &pane_seat,
        ],
    );
    assert_eq!(sent.code, Some(0), "{sent:?}");
    let id = sent_message(&sent);
    rig.hook_context("to the pane", || rig.pane_prompt_hook(&pane, "s-pane"));
    let owed = rig.pane_agent(&pane, "s-pane", &["pending-receipts", "--machine"]);
    assert!(!owed.stdout.contains(&id), "the display ACKed it: {owed:?}");
    let again = rig.agent(
        "alice",
        "s-alice",
        &[
            "send",
            "--seat",
            &pane_seat,
            "--body",
            "second",
            "--require-ack",
            &pane_seat,
        ],
    );
    let again_id = sent_message(&again);
    let inbox = rig.pane_agent(&pane, "s-pane", &["inbox"]);
    assert!(inbox.stdout.contains("second"), "{inbox:?}");
    let ack = rig.pane_agent(&pane, "s-pane", &["ack", &again_id]);
    assert_eq!(ack.code, Some(0), "{ack:?}");
    let owed = rig.pane_agent(&pane, "s-pane", &["pending-receipts", "--machine"]);
    assert!(!owed.stdout.contains(&again_id), "{owed:?}");

    // pane -> named, delivered by the named seat's hook and read by inbox.
    let reply = rig.pane_agent(
        &pane,
        "s-pane",
        &["send", "--seat", &alice, "--body", "to alice"],
    );
    assert_eq!(reply.code, Some(0), "{reply:?}");
    rig.hook_context("to alice", || rig.named_prompt_hook("alice", "s-alice"));
}

// Kills: a named seat that cannot join an ordinary thread or accept an invitation from the Named
// site, and a text `accept` that omits the joined thread's backlog (needing an extra call).
#[test]
fn named_join_accept_backlog() {
    let rig = Rig::standalone("ns-accept");
    let (_alice, bob) = two_named(&rig);
    rig.enroll("carol", "s-carol");
    let created = rig.agent(
        "alice",
        "s-alice",
        &["thread", "create", "--name", "ops", "--topic", "ops"],
    );
    assert_eq!(created.code, Some(0), "{created:?}");
    // carol joins an ordinary thread by name.
    let joined = rig.agent("carol", "s-carol", &["join", "ops"]);
    assert_eq!(joined.code, Some(0), "{joined:?}");
    // bob joins, is sent a row he never reads, leaves and is invited back: that unread row is the
    // backlog his accept shows.
    let first = rig.agent("bob", "s-bob", &["join", "ops"]);
    assert_eq!(first.code, Some(0), "{first:?}");
    let sent = rig.agent(
        "alice",
        "s-alice",
        &["send", "ops", "--body", "backlog one"],
    );
    assert_eq!(sent.code, Some(0), "{sent:?}");
    let left = rig.agent("bob", "s-bob", &["leave", "ops"]);
    assert_eq!(left.code, Some(0), "{left:?}");
    let invited = rig.agent("alice", "s-alice", &["invite", "ops", "--seat", &bob]);
    assert_eq!(invited.code, Some(0), "{invited:?}");
    // Not read first: the accept itself must show the row, with no extra call.
    let accepted = rig.agent("bob", "s-bob", &["--human", "accept", "ops"]);
    assert_eq!(accepted.code, Some(0), "{accepted:?}");
    assert!(
        accepted.stdout.contains("backlog one"),
        "accept prints the backlog itself: {accepted:?}"
    );
    let members = rig.agent("bob", "s-bob", &["participants", "ops", "--machine"]);
    assert!(members.stdout.contains(&bob), "{members:?}");
}

// Kills: a new 1:1 pair of a named seat and a pane seat whose participants are not auto-joined,
// and an opener that needs more than the recipient's normal turn-hook delivery.
#[test]
fn named_pair_with_pane_auto_join_opener() {
    let Some(rig) = Rig::beside_herdr("ns-pair") else {
        return;
    };
    let pane = rig.pane("ns-pair");
    rig.enroll("alice", "s-alice");
    rig.enroll_pane(&pane, "s-pane");
    let (alice, pane_seat) = (rig.seat_of("alice"), rig.seat_of_pane(&pane));
    let opener = rig.agent(
        "alice",
        "s-alice",
        &["send", "--seat", &pane_seat, "--body", "pair opener"],
    );
    assert_eq!(opener.code, Some(0), "{opener:?}");
    // Normal delivery: the pane's hook, with no accept and no inbox call.
    rig.hook_context("pair opener", || rig.pane_prompt_hook(&pane, "s-pane"));
    let thread: String = rig
        .db()
        .query_row(
            "SELECT thread_id FROM messages WHERE body LIKE '%pair opener%' LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap_or_else(|_| panic!("the opener's thread: {opener:?}"));
    let members = rig.agent("alice", "s-alice", &["participants", &thread, "--machine"]);
    assert!(
        members.stdout.contains(&alice) && members.stdout.contains(&pane_seat),
        "both seats auto-joined: {members:?}"
    );
}

fn find_key<'a>(value: &'a serde_json::Value, key: &str) -> &'a serde_json::Value {
    fn walk<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
        match value {
            serde_json::Value::Object(map) => map
                .get(key)
                .or_else(|| map.values().find_map(|v| walk(v, key))),
            serde_json::Value::Array(items) => items.iter().find_map(|v| walk(v, key)),
            _ => None,
        }
    }
    walk(value, key).unwrap_or_else(|| panic!("no {key} in {value}"))
}

// Kills: a handoff recovery hint of a named caller that does not re-establish the name, a hint
// that resolves to a different (new) seat, and a hint whose command the CLI cannot replay.
#[test]
fn recovery_hint_round_trip() {
    use herdr_threads::{
        cli::{caller_location::claim_recovery_env, hook::cli_prefix},
        protocol::{
            ids::{HostLocator, NAMED_HOST_LOCATOR},
            output::ContinuationContext,
        },
    };
    let Some(rig) = Rig::beside_herdr("ns-hint") else {
        return;
    };
    let target = rig.pane("ns-hint-target");
    rig.enroll("alice", "s-alice");
    let alice = rig.seat_of("alice");
    let socket = rig.herdr.as_ref().unwrap().socket_path();
    let (state, endpoint) = (rig.state.to_str().unwrap(), socket.to_str().unwrap());
    let handoff = rig.agent(
        "alice",
        "s-alice",
        &[
            "--json",
            "--host-endpoint",
            endpoint,
            "handoff",
            "--pane",
            &target,
            "--kind",
            "codex",
            "--",
            "named-task",
        ],
    );
    assert_eq!(handoff.code, Some(0), "{handoff:?}");
    let out: serde_json::Value = serde_json::from_str(&handoff.stdout).unwrap();
    let reference = find_key(&out, "recovery_ref").as_str().unwrap().to_owned();

    // The hint the handoff's report builds for its (named) frozen claim.
    let claim_host = HostLocator::parse(NAMED_HOST_LOCATOR).unwrap();
    let mut hint = vec!["env".to_owned()];
    hint.extend(claim_recovery_env(&claim_host, "alice"));
    hint.extend(cli_prefix(&ContinuationContext {
        state_dir: Some(state.to_owned()),
        host: Some(endpoint.to_owned()),
    }));
    hint.extend(["retry".to_owned(), reference]);
    assert_eq!(
        hint[1], "HERDR_THREADS_SEAT_NAME=alice",
        "the hint names the seat: {hint:?}"
    );

    // Run the hint as printed (its `herdr-threads` word is this build), in a process that has the
    // session but not the name: it resolves back to the same seat.
    let program = hint.iter().position(|w| w == "herdr-threads").unwrap();
    hint[program] = BIN.to_owned();
    let mut command = rig.base("env");
    command.args(&hint[1..]);
    herdr_threads::test_support::isolation::as_agent(&mut command, "claude", "s-alice");
    let before = rig.seat_count();
    let replay = run(command, None);
    assert_eq!(replay.code, Some(0), "{replay:?}");
    assert!(!replay.stderr.contains("created seat"), "{replay:?}");
    assert_eq!(rig.seat_count(), before, "no new seat");
    assert_eq!(rig.seat_of("alice"), alice, "the same seat");
}

// Kills: a daemon that never idle-exits because the named host is attached (only a dialled host
// or a wake-armed binding keeps it up), and an exit while a client is still connected.
#[test]
fn idle_exit_with_only_named_host() {
    let mut rig = Rig::standalone_with("ns-idle", &[("HT_TEST_IDLE_EXIT_MS", "2000")]);
    rig.enroll("alice", "s-alice");
    let named_hosts: i64 = rig
        .db()
        .query_row(
            "SELECT COUNT(*) FROM herdr_hosts WHERE kind='named'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        named_hosts, 1,
        "the named host is attached and enrolled a seat"
    );
    wait_for("the idle daemon to exit", Duration::from_secs(30), || {
        if rig.daemon_exited() {
            Ok(())
        } else {
            Err("still running".to_owned())
        }
    });
}
