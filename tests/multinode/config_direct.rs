//! Configuration 1 (spec §13): two nodes linked directly; a seat on each; a
//! direct send with `--require-ack` from A to B's seat, delivered and woken on
//! B, ACKed on B, settled on both.

use crate::support::{self, P1, P2, P3};

#[test]
fn two_nodes_direct_send_and_ack() {
    let cluster = support::cluster("mn-direct", 2, &[(0, 1)], &[0, 1]);
    let a = cluster.seat(0, P1);
    let b = cluster.seat(1, P1);
    let slow = cluster.seat(1, P3);
    let person = cluster.human_seat(1, P2);
    support::wait_seats(&cluster, 0, 4);
    support::wait_seats(&cluster, 1, 4);

    let (thread, message) = support::send(
        &cluster,
        &a,
        &["--seat", &b.id, "--body", "ping", "--require-ack", &b.id],
    );
    // B holds the message and its pending receipt; its idle seat was woken once.
    support::wait_receipt(&cluster, 1, &message, &b, "pending");
    support::wait_wakes(&cluster, 1, P1, 1);
    assert!(
        support::inbox_ids(&cluster, &b).contains(&message),
        "B's seat is offered the message"
    );
    assert_eq!(
        cluster.host(0).prompts_for(P1),
        0,
        "the sender is not woken"
    );

    support::ack(&cluster, &b, &message);
    support::wait_receipt(&cluster, 0, &message, &b, "acked");
    support::wait_receipt(&cluster, 1, &message, &b, "acked");
    cluster.wait_converged(crate::WAIT);
    for node in [0, 1] {
        let kinds = cluster.kinds(node);
        assert_eq!(
            kinds.get("message.sent"),
            Some(&1),
            "node {node}: {kinds:?}"
        );
        assert!(
            kinds.contains_key("receipt.settled"),
            "node {node}: {kinds:?}"
        );
        assert_eq!(
            cluster.scalar(
                node,
                "SELECT count(*) FROM threads WHERE id = ?1",
                &[&thread]
            ),
            Some("1".into()),
            "node {node} holds the pair thread"
        );
    }

    // D9: a person named with `--require-ack` gets a p0 delivery row on their
    // own node and no receipt on either node.
    let (_, to_person) = support::send(
        &cluster,
        &a,
        &[
            "--seat",
            &person.id,
            "--body",
            "for a person",
            "--require-ack",
            &person.id,
        ],
    );
    cluster.wait_converged(crate::WAIT);
    cluster.wait_until("the person's delivery row on node 1", crate::WAIT, || {
        cluster.scalar(
            1,
            "SELECT priority FROM lazy_recipients WHERE message_id = ?1 AND seat_id = ?2",
            &[&to_person, &person.id],
        )
    });
    assert_eq!(
        cluster
            .scalar(
                1,
                "SELECT priority FROM lazy_recipients WHERE message_id = ?1 AND seat_id = ?2",
                &[&to_person, &person.id],
            )
            .as_deref(),
        Some("0"),
        "an ACK request is p0, so is the person's row"
    );
    for node in [0, 1] {
        assert_eq!(
            cluster.scalar(
                node,
                "SELECT count(*) FROM receipt_state WHERE message_id = ?1",
                &[&to_person],
            ),
            Some("0".into()),
            "node {node}: no receipt for a person"
        );
    }
    assert_eq!(
        cluster.scalar(
            0,
            "SELECT count(*) FROM lazy_recipients WHERE message_id = ?1 AND seat_id = ?2",
            &[&to_person, &person.id],
        ),
        Some("0".into()),
        "the sender's node stages no row for a remote seat"
    );
    assert_eq!(
        cluster.scalar(
            0,
            "SELECT count(*) FROM lazy_recipients WHERE message_id = ?1 AND seat_id = ?2",
            &[&to_person, &a.id]
        ),
        Some("1".into()),
        "the sender's node retains its local author sibling row"
    );

    // Overdue is recorded on the sender's node only.
    let (_, unanswered) = support::send(
        &cluster,
        &a,
        &[
            "--seat",
            &slow.id,
            "--body",
            "answer soon",
            "--require-ack",
            &slow.id,
            "--deadline",
            "1",
        ],
    );
    let overdue = |node: usize| {
        cluster
            .scalar(
                node,
                "SELECT warning_message_id IS NOT NULL FROM receipt_state \
                 WHERE message_id = ?1 AND seat_id = ?2",
                &[&unanswered, &slow.id],
            )
            .as_deref()
            == Some("1")
    };
    cluster.wait_until("node 0 to record the overdue warning", crate::WAIT, || {
        overdue(0).then_some(())
    });
    cluster.wait_converged(crate::WAIT);
    assert!(
        !overdue(1),
        "the recipient's node records no overdue warning"
    );
    assert_eq!(
        support::receipt(&cluster, 1, &unanswered, &slow).as_deref(),
        Some("pending")
    );
    drop(cluster);
}

// A JSON read must not settle a receipt; a fully flushed text inbox must
// settle through the real CLI and replicate to the remote sender.
#[test]
fn text_inbox_auto_ack_reaches_remote_sender_json_read_is_passive() {
    let cluster = support::cluster("mn-inbox-ack", 2, &[(0, 1)], &[0, 1]);
    let a = cluster.seat(0, P1);
    let b = cluster.seat(1, P1);
    support::wait_seats(&cluster, 0, 2);
    support::wait_seats(&cluster, 1, 2);
    let (_, message) = support::send(
        &cluster,
        &a,
        &[
            "--seat",
            &b.id,
            "--body",
            "remote receipt body",
            "--require-ack",
            &b.id,
        ],
    );
    support::wait_receipt(&cluster, 1, &message, &b, "pending");
    assert!(support::inbox_ids(&cluster, &b).contains(&message));
    for node in [0, 1] {
        // Each node projects its physical receipt from its own work job.
        support::wait_receipt(&cluster, node, &message, &b, "pending");
        assert!(!cluster.kinds(node).contains_key("receipt.settled"));
    }
    let (code, stdout, stderr) = cluster.text_as(Some(&b), 1, &["inbox"]);
    assert_eq!(code, 0, "text inbox failed: {stderr}\n{stdout}");
    assert!(stdout.contains(&message), "{stdout}");
    assert!(stdout.contains("remote receipt body"), "{stdout}");
    for node in [0, 1] {
        support::wait_receipt(&cluster, node, &message, &b, "acked");
    }
    let (code, _, stderr) = cluster.text_as(Some(&b), 1, &["inbox"]);
    assert_eq!(code, 0, "second text inbox failed: {stderr}");
    cluster.wait_converged(crate::WAIT);
    for node in [0, 1] {
        assert_eq!(cluster.kinds(node).get("receipt.settled"), Some(&1));
    }
}
