//! Configurations 9 and 10 (spec §13): delivery across daemons. An idle seat on
//! B is woken, or not, by A's sends exactly as the delivery contract says, and
//! a seat that no fact references yet is opened as a direct pair by id alone.

use crate::support::{self, P1, P2, P3};
use std::time::Duration;

#[test]
fn wakes_follow_the_delivery_contract_across_daemons() {
    // One recipient seat per case: a seat is woken at most once per 30 s.
    let panes = [P1, P2, P3, "w1:p4", "w1:p5", "w1:p6", "w1:p7"];
    let cluster = support::cluster_with_panes("mn-delivery", 2, &[(0, 1)], &[0, 1], &panes);
    let a = cluster.seat(0, P1);
    let nudged_seat = cluster.seat(1, P1);
    let p0_seat = cluster.seat(1, P2);
    let p1_seat = cluster.seat(1, P3);
    let p3_wakes = cluster.seat(1, "w1:p4");
    let p3_quiet = cluster.seat(1, "w1:p5");
    let all_seat = cluster.seat(1, "w1:p6");
    let ack_seat = cluster.seat(1, "w1:p7");
    for node in [0, 1] {
        support::wait_seats(&cluster, node, 8);
    }
    // The threshold is node-local: each seat sets its own on B.
    support::set_threshold(&cluster, &p3_wakes, 3);
    support::set_threshold(&cluster, &all_seat, -1);

    // `--nudge` is `-p 0`: one wake marker through B's host, in the seat's pane.
    let (_, nudged) = support::send(
        &cluster,
        &a,
        &["--seat", &nudged_seat.id, "--body", "nudge", "--nudge"],
    );
    support::wait_wakes(&cluster, 1, P1, 1);
    assert!(support::inbox_ids(&cluster, &nudged_seat).contains(&nudged));

    // `-p 0` wakes a T = 0 seat once.
    let (_, p0) = support::send(
        &cluster,
        &a,
        &["--seat", &p0_seat.id, "--body", "urgent", "-p", "0"],
    );
    support::wait_wakes(&cluster, 1, P2, 1);
    assert!(support::inbox_ids(&cluster, &p0_seat).contains(&p0));

    // `-p 3` wakes a T = 3 seat, and a T = -1 seat wakes on a default p1.
    let (_, p3) = support::send(
        &cluster,
        &a,
        &["--seat", &p3_wakes.id, "--body", "p3", "-p", "3"],
    );
    support::wait_wakes(&cluster, 1, "w1:p4", 1);
    assert!(support::inbox_ids(&cluster, &p3_wakes).contains(&p3));
    let (_, p1_all) = support::send(
        &cluster,
        &a,
        &["--seat", &all_seat.id, "--body", "p1 for T -1"],
    );
    support::wait_wakes(&cluster, 1, "w1:p6", 1);
    assert!(support::inbox_ids(&cluster, &all_seat).contains(&p1_all));

    // Silent: a default p1 to a T = 0 seat, and a p3 to a T = 0 seat, wake
    // nothing; both wait for the seat's next turn.
    let (_, p1) = support::send(&cluster, &a, &["--seat", &p1_seat.id, "--body", "p1"]);
    let (_, quiet3) = support::send(
        &cluster,
        &a,
        &["--seat", &p3_quiet.id, "--body", "p3 quiet", "-p", "3"],
    );
    for message in [&p1, &quiet3] {
        cluster.wait_until("the silent message to reach B", crate::WAIT, || {
            cluster.scalar(1, "SELECT id FROM messages WHERE id = ?1", &[message])
        });
    }
    std::thread::sleep(Duration::from_millis(1500));
    for pane in [P3, "w1:p5"] {
        assert_eq!(
            cluster.host(1).prompts_for(pane),
            0,
            "{pane}: a send above the threshold wakes nothing: {:?}",
            cluster.host(1).prompts()
        );
    }
    assert!(support::inbox_ids(&cluster, &p1_seat).contains(&p1));
    assert!(support::inbox_ids(&cluster, &p3_quiet).contains(&quiet3));

    // `--require-ack` is p0: it wakes, the ACK settles, and A shows `acked`.
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
    support::wait_wakes(&cluster, 1, "w1:p7", 1);
    support::ack(&cluster, &ack_seat, &acked);
    support::wait_receipt(&cluster, 0, &acked, &ack_seat, "acked");
    cluster.wait_converged(crate::WAIT);

    // The priority is a replicated fact: both nodes hold the same one.
    for (message, priority) in [
        (&nudged, "0"),
        (&p0, "0"),
        (&p3, "3"),
        (&p1_all, "1"),
        (&p1, "1"),
        (&quiet3, "3"),
        (&acked, "0"),
    ] {
        for node in [0, 1] {
            assert_eq!(
                cluster
                    .scalar(
                        node,
                        "SELECT priority FROM messages WHERE id = ?1",
                        &[message]
                    )
                    .as_deref(),
                Some(priority),
                "node {node}: priority of {message}"
            );
        }
    }
    assert_eq!(
        cluster
            .scalar(
                1,
                "SELECT priority FROM lazy_recipients WHERE message_id = ?1 AND seat_id = ?2",
                &[&quiet3, &p3_quiet.id],
            )
            .as_deref(),
        Some("3"),
        "B stages the delivery row at the message priority"
    );
    assert_eq!(
        cluster.host(0).prompts_for(P1),
        0,
        "the sender is never woken"
    );
    for (pane, wakes) in [(P1, 1), (P2, 1), ("w1:p4", 1), ("w1:p6", 1), ("w1:p7", 1)] {
        assert_eq!(cluster.host(1).prompts_for(pane), wakes, "{pane}");
    }
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
fn a_seat_no_fact_references_is_opened_as_a_pair_by_id() {
    let cluster = support::cluster("mn-by-id", 2, &[(0, 1)], &[0, 1]);
    let a = cluster.seat(0, P1);
    let b = cluster.seat(1, P1);
    let b2 = cluster.seat(1, P2);
    for node in [0, 1] {
        support::wait_seats(&cluster, node, 3);
        assert_eq!(
            cluster.scalar(node, "SELECT count(*) FROM threads", &[]),
            Some("0".into()),
            "nothing references any seat yet"
        );
    }

    // A p1 opener opens the pair: both seats joined, no invitation, no wake.
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
        assert_eq!(state(&b.id).as_deref(), Some("joined"), "node {node}");
        assert_eq!(
            cluster.scalar(node, "SELECT count(*) FROM invitations", &[]),
            Some("0".into()),
            "node {node}: a new pair creates no invitation"
        );
    }
    // B is offered the opening message and no invitation; nobody is woken.
    let ids = support::inbox_ids(&cluster, &b);
    assert!(ids.contains(&message), "{ids:?}");
    assert!(!ids.iter().any(|id| id.starts_with('i')), "{ids:?}");
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(
        cluster.host(1).prompts_for(P1),
        0,
        "a p1 opener wakes nobody: {:?}",
        cluster.host(1).prompts()
    );

    // A `-p 0` opener of a second pair wakes its recipient once.
    let (thread2, urgent) = support::send(
        &cluster,
        &a,
        &["--seat", &b2.id, "--body", "urgent hello", "-p", "0"],
    );
    assert_ne!(thread, thread2);
    support::wait_wakes(&cluster, 1, P2, 1);
    assert!(support::inbox_ids(&cluster, &b2).contains(&urgent));
    cluster.wait_converged(crate::WAIT);
    for node in [0, 1] {
        assert_eq!(
            cluster.scalar(
                node,
                "SELECT count(*) FROM memberships WHERE thread_id = ?1 AND state = 'joined'",
                &[&thread2],
            ),
            Some("2".into()),
            "node {node}: the second pair is joined too"
        );
        assert_eq!(
            cluster.scalar(node, "SELECT count(*) FROM invitations", &[]),
            Some("0".into()),
            "node {node}"
        );
    }
    assert_eq!(cluster.host(1).prompts_for(P1), 0);
}
