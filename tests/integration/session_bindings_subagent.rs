//! ht-54r.58 (ht-6v2): a subagent's text `inbox` displays its own binding's stream and advances
//! only its own cursor; the display settles nothing. The real binary under a synthetic Claude
//! parent, against a private Herdr endpoint and a detached daemon.
use super::session_claims::{BIN, World, run, stderr};
use herdr_threads::test_support::harness_shim;
use std::{path::Path, process::Output, time::Duration};

impl World {
    /// Like `agent`, but the default text output (the routing flags minus `--json`).
    fn agent_text(&self, session: &str, pane: &str, args: &[&str]) -> Output {
        let routing: Vec<String> = self
            .routing()
            .into_iter()
            .filter(|a| a != "--json")
            .collect();
        let argv: Vec<&str> = routing
            .iter()
            .map(String::as_str)
            .chain(args.iter().copied())
            .collect();
        let mut env = self.env();
        env.push(("HERDR_PANE_ID".into(), pane.into()));
        env.push(("CLAUDE_CODE_SESSION_ID".into(), session.into()));
        let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        run(harness_shim::under_harness(
            "claude",
            Path::new(BIN),
            &argv,
            &env,
        ))
    }

    fn db(&self) -> rusqlite::Connection {
        let db = rusqlite::Connection::open(&self.paths.database_path).unwrap();
        db.busy_timeout(Duration::from_secs(3)).unwrap();
        db
    }
}

// Kills: a subagent text inbox that is refused or reaches the daemon without its claim, a
// display whose AdvanceCursors moves the top-level binding's cursor (the subagent claim built as
// its parent's), and a subagent display that settles (the receipt must stay pending).
#[test]
fn subagent_text_inbox_advances_only_its_own_cursor() {
    let world = World::new(true);
    let seat = world.seat("w1:p1");
    let peer_seat = world.seat("w1:p2");
    for (session, pane, event) in [
        ("sess-main", "w1:p1", "ev-main"),
        ("sess-peer", "w1:p2", "ev-peer"),
    ] {
        let out = world.agent(
            "claude",
            Some(session),
            pane,
            &["check-in", "--lifecycle-event", event],
        );
        assert!(out.status.success(), "{out:?} {}", stderr(&out));
    }
    let thread = World::data(&world.agent(
        "claude",
        Some("sess-peer"),
        "w1:p2",
        &["thread", "create", "--topic", "subagent display"],
    ))
    .as_str()
    .unwrap()
    .to_owned();
    World::data(&world.agent(
        "claude",
        Some("sess-peer"),
        "w1:p2",
        &["invite", &thread, "--seat", &seat],
    ));
    World::data(&world.agent("claude", Some("sess-main"), "w1:p1", &["accept", &thread]));
    let _ = peer_seat;
    // The subagent's first read enrolls its binding (`--agent-id` is what the PreToolUse
    // rewrite inserts).
    let first = world.agent_text("sess-main", "w1:p1", &["--agent-id=agent-1", "inbox"]);
    assert!(first.status.success(), "{first:?} {}", stderr(&first));
    World::data(&world.agent(
        "claude",
        Some("sess-peer"),
        "w1:p2",
        &[
            "send",
            &thread,
            "--body",
            "hello subagent",
            "--require-ack",
            &seat,
        ],
    ));
    let read = world.agent_text("sess-main", "w1:p1", &["--agent-id=agent-1", "inbox"]);
    assert!(read.status.success(), "{read:?} {}", stderr(&read));
    let text = String::from_utf8_lossy(&read.stdout);
    assert!(
        text.contains("hello subagent"),
        "the subagent sees its mail: {text}"
    );

    let db = world.db();
    let binding = |agent: &str| -> String {
        db.query_row(
            "SELECT binding_id FROM occupant_bindings WHERE seat_id=?1 AND native_session='sess-main' \
             AND agent_id=?2 AND ended_at IS NULL",
            [&seat, agent],
            |r| r.get(0),
        )
        .unwrap_or_else(|e| panic!("binding {agent:?}: {e}"))
    };
    let cursor = |binding: &str| -> Option<i64> {
        db.query_row(
            "SELECT seq FROM binding_cursors WHERE binding_id=?1 AND thread_id=?2",
            [binding, thread.as_str()],
            |r| r.get(0),
        )
        .ok()
    };
    let head: i64 = db
        .query_row(
            "SELECT COALESCE(MAX(sequence),0) FROM messages WHERE thread_id=?1",
            [&thread],
            |r| r.get(0),
        )
        .unwrap();
    assert!(head >= 1);
    let (sub, top) = (binding("agent-1"), binding(""));
    assert_eq!(
        cursor(&sub),
        Some(head),
        "the subagent's cursor reached the head"
    );
    assert!(
        cursor(&top).is_none_or(|seq| seq < head),
        "the top-level binding's cursor did not move: {:?}",
        cursor(&top)
    );
    let settled: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM receipt_state WHERE seat_id=?1 AND state<>'pending'",
            [&seat],
            |r| r.get(0),
        )
        .unwrap();
    let owed: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM receipt_state WHERE seat_id=?1 AND state='pending'",
            [&seat],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(settled, 0, "a subagent display settles nothing");
    assert_eq!(owed, 1, "the ACK the seat owes is still pending");
}

// ht-54r F1. Kills: a subagent's full `body <id>` read that moves no cursor, one that moves the
// top-level binding's cursor (the claim built as its parent's), and one that settles the ACK.
#[test]
fn subagent_full_body_read_advances_only_its_own_cursor() {
    let world = World::new(true);
    let seat = world.seat("w1:p1");
    let _peer_seat = world.seat("w1:p2");
    for (session, pane, event) in [
        ("sess-main", "w1:p1", "ev-main"),
        ("sess-peer", "w1:p2", "ev-peer"),
    ] {
        let out = world.agent(
            "claude",
            Some(session),
            pane,
            &["check-in", "--lifecycle-event", event],
        );
        assert!(out.status.success(), "{out:?} {}", stderr(&out));
    }
    let thread = World::data(&world.agent(
        "claude",
        Some("sess-peer"),
        "w1:p2",
        &["thread", "create", "--topic", "subagent body"],
    ))
    .as_str()
    .unwrap()
    .to_owned();
    World::data(&world.agent(
        "claude",
        Some("sess-peer"),
        "w1:p2",
        &["invite", &thread, "--seat", &seat],
    ));
    World::data(&world.agent("claude", Some("sess-main"), "w1:p1", &["accept", &thread]));
    let first = world.agent_text("sess-main", "w1:p1", &["--agent-id=agent-1", "inbox"]);
    assert!(first.status.success(), "{first:?} {}", stderr(&first));
    let sent = World::data(&world.agent(
        "claude",
        Some("sess-peer"),
        "w1:p2",
        &[
            "send",
            &thread,
            "--body",
            "read me by id",
            "--require-ack",
            &seat,
        ],
    ));
    let id = sent.as_str().unwrap().to_owned();
    let read = world.agent_text("sess-main", "w1:p1", &["--agent-id=agent-1", "body", &id]);
    assert!(read.status.success(), "{read:?} {}", stderr(&read));
    assert!(
        String::from_utf8_lossy(&read.stdout).contains("read me by id"),
        "{read:?}"
    );

    let db = world.db();
    let binding = |agent: &str| -> String {
        db.query_row(
            "SELECT binding_id FROM occupant_bindings WHERE seat_id=?1 AND native_session='sess-main' \
             AND agent_id=?2 AND ended_at IS NULL",
            [&seat, agent],
            |r| r.get(0),
        )
        .unwrap_or_else(|e| panic!("binding {agent:?}: {e}"))
    };
    let cursor = |binding: &str| -> Option<i64> {
        db.query_row(
            "SELECT seq FROM binding_cursors WHERE binding_id=?1 AND thread_id=?2",
            [binding, thread.as_str()],
            |r| r.get(0),
        )
        .ok()
    };
    let sequence: i64 = db
        .query_row("SELECT sequence FROM messages WHERE id=?1", [&id], |r| {
            r.get(0)
        })
        .unwrap();
    let (sub, top) = (binding("agent-1"), binding(""));
    assert_eq!(
        cursor(&sub),
        Some(sequence),
        "the subagent's full read moved its own cursor past the message"
    );
    assert!(
        cursor(&top).is_none_or(|seq| seq < sequence),
        "the top-level binding's cursor did not move: {:?}",
        cursor(&top)
    );
    let state: String = db
        .query_row(
            "SELECT state FROM receipt_state WHERE seat_id=?1 AND message_id=?2",
            [&seat, &id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "pending", "a subagent body read settles nothing");
}

// The real binary and daemon over a seat carrying many stopped subagents (a subagent stop
// disconnects its binding and never ends it). Kills an unbounded presence list: `seat inspect`,
// `send --seat` (which inspects the recipient seat) and `seat list` refused with
// `invalid_budget` (`seat history item cannot fit`) at the default 16 KiB page.
#[test]
fn many_stopped_subagents_keep_seat_inspect_and_direct_send_working() {
    let world = World::new(true);
    let seat = world.seat("w1:p1");
    world.seat("w1:p2");
    for (session, pane, event) in [
        ("sess-main", "w1:p1", "ev-main"),
        ("sess-peer", "w1:p2", "ev-peer"),
    ] {
        let out = world.agent(
            "claude",
            Some(session),
            pane,
            &["check-in", "--lifecycle-event", event],
        );
        assert!(out.status.success(), "{out:?} {}", stderr(&out));
    }
    let thread = World::data(&world.agent(
        "claude",
        Some("sess-peer"),
        "w1:p2",
        &["thread", "create", "--topic", "stopped subagents"],
    ))
    .as_str()
    .unwrap()
    .to_owned();
    World::data(&world.agent(
        "claude",
        Some("sess-peer"),
        "w1:p2",
        &["invite", &thread, "--seat", &seat],
    ));
    World::data(&world.agent("claude", Some("sess-main"), "w1:p1", &["accept", &thread]));
    let enroll = world.agent_text("sess-main", "w1:p1", &["--agent-id=agent-0", "inbox"]);
    assert!(enroll.status.success(), "{enroll:?} {}", stderr(&enroll));
    // Stop the enrolled subagent as SubagentStop does, then clone it into 300 stopped siblings
    // (~27 KB of presence before the bound).
    let db = world.db();
    let stopped = db
        .execute(
            "UPDATE occupant_bindings SET liveness='disconnected',disconnected_at=1,\
             disconnect_reason='liveness_end_event' WHERE seat_id=?1 AND agent_id='agent-0' \
             AND ended_at IS NULL",
            [&seat],
        )
        .unwrap();
    assert_eq!(stopped, 1, "the subagent binding was enrolled");
    for n in 1..300 {
        db.execute(
            "INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,\
             terminal_id,incarnation,host_boot,host_epoch,harness,native_session,execution_id,\
             observation_provenance,observed_at,registered_at,role,parent_binding,agent_id,\
             liveness,disconnected_at,disconnect_reason) \
             SELECT seat_id,generation,target_generation,target_id,terminal_id,incarnation,\
             host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,\
             observed_at,registered_at,role,parent_binding,?2,liveness,disconnected_at,\
             disconnect_reason FROM occupant_bindings WHERE seat_id=?1 AND agent_id='agent-0' \
             AND ended_at IS NULL",
            [seat.clone(), format!("agent-{n}")],
        )
        .unwrap();
    }
    drop(db);

    let inspect = World::data(&world.human(&["seat", "inspect", &seat]));
    let agents: Vec<_> = inspect["summary"]["bindings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["agent_id"].as_str().unwrap_or("").to_owned())
        .collect();
    assert_eq!(
        agents,
        ["", "agent-296", "agent-297", "agent-298", "agent-299"],
        "the top-level binding and the most recent stopped subagents"
    );
    let sent = world.agent(
        "claude",
        Some("sess-peer"),
        "w1:p2",
        &["send", "--seat", &seat, "--body", "hi"],
    );
    assert!(sent.status.success(), "{sent:?} {}", stderr(&sent));
    let seats = World::data(&world.human(&["seat", "list"]));
    assert!(
        seats["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["seat"] == seat.as_str()),
        "{seats}"
    );
}
