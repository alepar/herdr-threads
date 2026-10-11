//! ht-54r.25: end-to-end flows of session-keyed seat bindings that no per-seam test and no
//! configuration of `session_bindings_smoke` asserts. They reuse the smoke fixture (an isolated
//! named Herdr session, a real detached daemon, the real executable under isolated HOMEs).
use super::session_bindings_smoke::{
    Agent, REFUSED, Smoke, claude_start, claude_subagent, joined_thread, wait_until,
};
use std::time::Duration;

impl Smoke {
    /// `(state, ack_binding_id)` of every receipt row of `message` on `seat`.
    fn receipts(&self, message: &str, seat: &str) -> Vec<(String, Option<String>)> {
        let db = self.db();
        let mut statement = db
            .prepare(
                "SELECT state,ack_binding_id FROM receipt_state WHERE message_id=?1 AND seat_id=?2",
            )
            .unwrap();
        statement
            .query_map([message, seat], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }
}

// Kills: a receipt that settles per binding (two settlements, or the later ACK overwriting
// `ack_binding_id`), a later ACK of a settled obligation that errors, a subagent ACK that is not
// refused or that settles, a display by one top-level binding that moves another's cursor, and a
// subagent that cannot read the stream its parent's seat receives.
#[test]
fn two_agents_in_one_pane_settle_one_obligation_once_and_a_subagent_reads_only() {
    let Some(mut smoke) = Smoke::new("e2e_two_agents_settle_once") else {
        return;
    };
    let (pane, peer_pane) = (smoke.pane(), smoke.pane());
    let (seat, _) = (smoke.seat(&pane), smoke.seat(&peer_pane));
    let first = Agent::claude("sess-a", &pane).in_process(smoke.stand_in());
    let second = Agent::claude("sess-b", &pane).in_process(smoke.stand_in());
    let peer = Agent::claude("sess-peer", &peer_pane);
    for who in [&first, &second, &peer] {
        let out = smoke.hook(who, &claude_start(&who.session, "startup"));
        assert_eq!(out.code, 0, "{}", out.text());
    }
    let out = smoke.hook(
        &first,
        &claude_subagent("SubagentStart", "sess-a", "agent-1"),
    );
    assert_eq!(out.code, 0, "{}", out.text());
    let (b_first, b_second, b_sub) = (
        smoke.binding(&seat, "claude", "sess-a", ""),
        smoke.binding(&seat, "claude", "sess-b", ""),
        smoke.binding(&seat, "claude", "sess-a", "agent-1"),
    );
    assert_eq!(b_first.liveness, "connected", "{b_first:?}");
    assert_eq!(b_second.liveness, "connected", "{b_second:?}");
    assert_eq!(b_sub.role, "subagent");

    let thread = joined_thread(&smoke, &peer, &first, &seat);
    let message = smoke
        .cli(
            &peer,
            &["send", &thread, "--body", "owed", "--require-ack", &seat],
        )
        .data()
        .as_str()
        .unwrap()
        .to_owned();

    // The subagent reads the seat's stream (read-only) and cannot settle or send.
    let items = smoke.cli(&first, &["--agent-id=agent-1", "inbox"]).data()["items"].clone();
    assert!(
        items
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["message"] == message.as_str()),
        "the subagent sees the obligation: {items}"
    );
    let refused = smoke.cli(&first, &["--agent-id=agent-1", "ack", &message]);
    assert_eq!(refused.code, REFUSED, "{}", refused.text());
    assert!(refused.text().contains("subagent"), "{}", refused.text());
    assert_eq!(
        smoke.receipts(&message, &seat),
        vec![("pending".to_owned(), None)],
        "a subagent settles nothing"
    );

    // The second top-level binding ACKs first: it settles the obligation, recorded as its own.
    let acked = smoke.cli(&second, &["ack", &message]).data();
    assert_eq!(acked["acknowledged"][0], message.as_str(), "{acked}");
    assert_eq!(
        smoke.receipts(&message, &seat),
        vec![("acked".to_owned(), Some(b_second.id.clone()))]
    );
    // The first binding's later ACK of the settled obligation succeeds and changes nothing.
    let again = smoke.cli(&first, &["ack", &message]);
    assert_eq!(again.code, 0, "{}", again.text());
    assert_eq!(
        smoke.receipts(&message, &seat),
        vec![("acked".to_owned(), Some(b_second.id.clone()))],
        "the first settlement stays, once"
    );

    // Both receive and display the same message independently: each display moves only its own
    // cursor, the subagent's included.
    // (An ACK adds its own marker after the message, so the head is read again.)
    let head = smoke.head(&thread);
    let sub_before = smoke.cursor(&b_sub.id, &thread).unwrap();
    assert!(sub_before < head);
    for (who, binding) in [(&first, &b_first), (&second, &b_second)] {
        assert!(smoke.cursor(&binding.id, &thread).unwrap() < head);
        let read = smoke.text(who, &["inbox"]);
        assert_eq!(read.code, 0, "{}", read.text());
        assert!(read.stdout.contains("owed"), "{}", read.text());
        assert_eq!(
            smoke.cursor(&binding.id, &thread),
            Some(head),
            "{binding:?}"
        );
        assert_eq!(
            smoke.cursor(&b_sub.id, &thread),
            Some(sub_before),
            "{binding:?}'s display leaves the subagent's cursor alone"
        );
    }
    assert_eq!(smoke.receipts(&message, &seat).len(), 1, "one obligation");
}

// Kills: a one-shot that is disconnected with another reason or provenance, a disconnect that ends
// the binding, a later call of the finished one-shot's session that does not reconnect it (or one
// that leaves it connected after its own process is gone), a send whose binding is lost with the
// process, and a one-shot exit that disturbs the pane's live session.
#[test]
fn a_one_shot_adds_sends_exits_and_a_later_call_reconnects_until_its_process_exits() {
    let Some(mut smoke) = Smoke::new("e2e_one_shot_exits") else {
        return;
    };
    let pane = smoke.pane();
    let seat = smoke.seat(&pane);
    let live = Agent::claude("sess-live", &pane).in_process(smoke.stand_in());
    assert_eq!(
        smoke
            .hook(&live, &claude_start("sess-live", "startup"))
            .code,
        0
    );
    let one_shot = Agent::claude("sess-p", &pane).shimmed();
    let thread = smoke
        .cli(&one_shot, &["thread", "create", "--topic", "one-shot"])
        .data()
        .as_str()
        .unwrap()
        .to_owned();
    let sent = smoke
        .cli(&one_shot, &["send", &thread, "--body", "from the one-shot"])
        .data()
        .as_str()
        .unwrap()
        .to_owned();
    let binding = smoke.binding(&seat, "claude", "sess-p", "");
    assert_eq!(binding.role, "top_level");

    // The one-shot's process is gone; the liveness worker disconnects it within a cadence.
    wait_until(
        "the one-shot to disconnect",
        Duration::from_secs(30),
        || smoke.binding(&seat, "claude", "sess-p", "").liveness == "disconnected",
    );
    let gone = smoke.binding(&seat, "claude", "sess-p", "");
    let disconnected_at = |smoke: &Smoke| -> Option<i64> {
        smoke
            .db()
            .query_row(
                "SELECT disconnected_at FROM occupant_bindings WHERE binding_id=?1",
                [&binding.id],
                |r| r.get(0),
            )
            .unwrap()
    };
    let first_disconnect = disconnected_at(&smoke).expect("a disconnected row has a time");
    assert_eq!(gone.reason.as_deref(), Some("liveness_process_exit"));
    assert!(!gone.ended, "disconnection never ends a binding: {gone:?}");
    assert_eq!(
        smoke.binding(&seat, "claude", "sess-live", "").liveness,
        "connected"
    );
    let sender: Option<String> = smoke
        .db()
        .query_row(
            "SELECT sender_binding_id FROM messages WHERE id=?1",
            [&sent],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(sender.as_deref(), Some(binding.id.as_str()));

    // A call (a write, which resolves the caller's claim) from the same session id in a new
    // process carries a fingerprint and reconnects the binding (spec 3: any call the binding
    // makes). That process exits too, so the worker disconnects it again; a later
    // `disconnected_at` proves it reconnected between.
    let touch = smoke.cli(&one_shot, &["send", &thread, "--body", "a later call"]);
    assert_eq!(touch.code, 0, "{}", touch.text());
    wait_until(
        "the reconnected one-shot to disconnect again",
        Duration::from_secs(30),
        || {
            smoke.binding(&seat, "claude", "sess-p", "").liveness == "disconnected"
                && disconnected_at(&smoke).is_some_and(|at| at > first_disconnect)
        },
    );
    let again = smoke.binding(&seat, "claude", "sess-p", "");
    assert_eq!(again.reason.as_deref(), Some("liveness_process_exit"));
    assert_eq!(again.id, binding.id, "the same binding, not a new one");
}
