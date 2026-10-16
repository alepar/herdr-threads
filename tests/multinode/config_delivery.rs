//! Configurations 9 and 10 (spec §13): delivery across daemons. An idle seat on
//! B is woken, or not, by A's sends exactly as the delivery contract says, and
//! a seat that no fact references yet is opened as a direct pair by id alone.

use crate::support::{self, P1, P2, P3};
use std::time::Duration;

#[test]
#[ignore = "pending ht-5fr reconcile item 22"]
fn wakes_follow_the_delivery_contract_across_daemons() {
    // One recipient seat per case: a seat is woken at most once per 30 s.
    let cluster = support::cluster("mn-delivery", 2, &[(0, 1)], &[0, 1]);
    let a = cluster.seat(0, P1);
    let nudged_seat = cluster.seat(1, P1);
    let lazy_seat = cluster.seat(1, P2);
    let ack_seat = cluster.seat(1, P3);
    for node in [0, 1] {
        support::wait_seats(&cluster, node, 4);
    }

    // `--nudge`: one wake marker through B's host, in the seat's own pane.
    let (_, nudged) = support::send(
        &cluster,
        &a,
        &["--seat", &nudged_seat.id, "--body", "nudge", "--nudge"],
    );
    support::wait_wakes(&cluster, 1, P1, 1);
    assert!(support::inbox_ids(&cluster, &nudged_seat).contains(&nudged));

    // Lazy: the idle seat is not woken; the item waits for its next turn.
    let (_, lazy) = support::send(&cluster, &a, &["--seat", &lazy_seat.id, "--body", "lazy"]);
    cluster.wait_until("the lazy message to reach B", crate::WAIT, || {
        cluster.scalar(1, "SELECT id FROM messages WHERE id = ?1", &[&lazy])
    });
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(
        cluster.host(1).prompts_for(P2),
        0,
        "a lazy send wakes nothing: {:?}",
        cluster.host(1).prompts()
    );
    assert!(
        support::inbox_ids(&cluster, &lazy_seat).contains(&lazy),
        "the lazy item is pending for the seat's next turn"
    );

    // `--require-ack` is nudge-class: it wakes.
    let (_, acked) = support::send(
        &cluster,
        &a,
        &[
            "--seat",
            &ack_seat.id,
            "--body",
            "ack me",
            "--require-ack",
            &ack_seat.id,
        ],
    );
    support::wait_wakes(&cluster, 1, P3, 1);
    support::ack(&cluster, &ack_seat, &acked);
    support::wait_receipt(&cluster, 0, &acked, &ack_seat, "acked");
    cluster.wait_converged(crate::WAIT);
    assert_eq!(
        cluster.host(0).prompts_for(P1),
        0,
        "the sender is never woken"
    );
    assert_eq!(
        cluster.host(1).prompts_for(P1),
        1,
        "one marker for the nudge"
    );
}

#[test]
fn settled_history_pulled_at_first_sync_wakes_nobody() {
    // A's two seats settle a nudged, ACKed exchange while B is not linked; B,
    // with an idle seat of its own, then syncs for the first time.
    let mut cluster = support::cluster("mn-catchup", 2, &[], &[0, 1]);
    let a1 = cluster.seat(0, P1);
    let a2 = cluster.seat(0, P2);
    let b = cluster.seat(1, P1);
    let (thread, message) = support::send(
        &cluster,
        &a1,
        &[
            "--seat",
            &a2.id,
            "--body",
            "settled",
            "--require-ack",
            &a2.id,
        ],
    );
    support::wait_wakes(&cluster, 0, P2, 1);
    support::ack(&cluster, &a2, &message);
    support::wait_receipt(&cluster, 0, &message, &a2, "acked");

    cluster.link(1, 0);
    cluster.wait_converged(crate::WAIT);
    assert_eq!(
        support::receipt(&cluster, 1, &message, &a2).as_deref(),
        Some("acked"),
        "B pulled the settled history"
    );
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(
        cluster.host(1).prompts_for(P1),
        0,
        "{:?}",
        cluster.host(1).prompts()
    );
    assert!(
        support::inbox_ids(&cluster, &b).is_empty(),
        "thread {thread}"
    );
}

#[test]
#[ignore = "pending ht-5fr reconcile item 22"]
fn a_seat_no_fact_references_is_opened_as_a_pair_by_id() {
    let cluster = support::cluster("mn-by-id", 2, &[(0, 1)], &[0, 1]);
    let a = cluster.seat(0, P1);
    let b = cluster.seat(1, P1);
    support::wait_seats(&cluster, 0, 2);
    support::wait_seats(&cluster, 1, 2);
    for node in [0, 1] {
        assert_eq!(
            cluster.scalar(node, "SELECT count(*) FROM threads", &[]),
            Some("0".into()),
            "nothing references either seat yet"
        );
    }

    let (thread, message) = support::send(&cluster, &a, &["--seat", &b.id, "--body", "hello"]);
    assert!(thread.starts_with('d'), "a direct pair thread: {thread}");
    cluster.wait_converged(crate::WAIT);
    for node in [0, 1] {
        let state = |seat: &str| {
            cluster.scalar(
                node,
                "SELECT state FROM memberships WHERE thread_id = ?1 AND seat_id = ?2",
                &[&thread, &seat],
            )
        };
        assert_eq!(state(&a.id).as_deref(), Some("joined"), "node {node}");
        assert_eq!(state(&b.id).as_deref(), Some("invited"), "node {node}");
        assert_eq!(
            cluster.scalar(
                node,
                "SELECT state FROM invitations WHERE thread_id = ?1 AND seat_id = ?2",
                &[&thread, &b.id],
            ),
            Some("pending".into()),
            "node {node} holds B's pending invitation"
        );
    }
    // B is offered the invitation and the opening message.
    let ids = support::inbox_ids(&cluster, &b);
    assert!(ids.contains(&message), "{ids:?}");
    assert!(ids.iter().any(|id| id.starts_with('i')), "{ids:?}");
}
