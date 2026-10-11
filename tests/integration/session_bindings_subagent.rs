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
