//! Invitations across nodes: an invitation is always
//! confirming and priority 0, so it wakes an idle invitee at every nudge
//! threshold, its deadline is replicated, and the overdue warning for an
//! unanswered one is emitted by the inviter's node only.

use crate::support::{self, P1, P2};
use herdr_threads::test_support::multinode::Cluster;

/// The invitation of `seat` in `thread` as `node` holds it: `(state,
/// confirmation, deadline_at)`.
fn invitation(
    cluster: &Cluster,
    node: usize,
    thread: &str,
    seat: &str,
) -> Option<(String, String, String)> {
    let rows = cluster.rows(
        node,
        &format!(
            "SELECT state, coalesce(confirmation,''), deadline_at FROM invitations \
             WHERE thread_id = '{thread}' AND seat_id = '{seat}'"
        ),
    );
    rows.into_iter().next().map(|row| {
        let mut row = row.into_iter();
        (
            row.next().unwrap(),
            row.next().unwrap(),
            row.next().unwrap(),
        )
    })
}

/// Waits until `node` holds a pending invitation of `seat` in `thread`.
fn wait_pending(cluster: &Cluster, node: usize, thread: &str, seat: &str) {
    cluster.wait_until(
        &format!("node {node} to hold the invitation"),
        crate::WAIT,
        || {
            invitation(cluster, node, thread, seat)
                .filter(|row| row.0 == "pending")
                .map(|_| ())
        },
    );
}

/// Whether `node` recorded the overdue warning of `seat`'s invitation.
fn warned(cluster: &Cluster, node: usize, thread: &str, seat: &str) -> bool {
    cluster
        .scalar(
            node,
            "SELECT warning_message_id IS NOT NULL FROM invitations \
             WHERE thread_id = ?1 AND seat_id = ?2",
            &[&thread, &seat],
        )
        .as_deref()
        == Some("1")
}

#[test]
fn invitation_across_nodes_wakes_and_overdue_stays_on_the_inviter() {
    let cluster = support::cluster("mn-invite", 2, &[(0, 1)], &[0, 1]);
    let a = cluster.seat(0, P1);
    let accepts = cluster.seat(1, P1);
    let ignores = cluster.seat(1, P2);
    for node in [0, 1] {
        support::wait_seats(&cluster, node, 3);
    }

    // `invite --lazy` no longer exists: the CLI refuses it and nothing changes.
    let thread = cluster.ok_as(Some(&a), 0, &["thread", "create", "--topic", "invites"]);
    let thread = thread.as_str().expect("a thread id").to_owned();
    let (code, _, stderr) = cluster.run_as(
        Some(&a),
        0,
        &["invite", &thread, "--seat", &accepts.id, "--lazy"],
    );
    assert_ne!(code, 0, "invite --lazy must be refused");
    assert!(stderr.contains("--lazy"), "an argument error: {stderr}");
    assert_eq!(
        cluster.scalar(0, "SELECT count(*) FROM invitations", &[]),
        Some("0".into()),
        "the refused invite changed nothing"
    );

    // An invitation wakes its idle T = 0 invitee once, though it is a default
    // send everywhere else; deadline and confirmation are the same on both
    // nodes.
    cluster.ok_as(
        Some(&a),
        0,
        &[
            "invite",
            &thread,
            "--seat",
            &accepts.id,
            "--deadline",
            "600",
        ],
    );
    wait_pending(&cluster, 1, &thread, &accepts.id);
    support::wait_wakes(&cluster, 1, P1, 1);
    cluster.wait_converged(crate::WAIT);
    let on_a = invitation(&cluster, 0, &thread, &accepts.id).expect("A holds it");
    let on_b = invitation(&cluster, 1, &thread, &accepts.id).expect("B holds it");
    assert_eq!(on_a, on_b, "the replicated invitation facts agree");
    assert_eq!(on_a.1, "require_ack", "an invitation always confirms");
    assert_eq!(
        cluster
            .scalar(
                0,
                "SELECT deadline_at - created_at FROM invitations WHERE thread_id = ?1",
                &[&thread]
            )
            .as_deref(),
        Some("600000"),
        "the deadline is created_at plus the frozen duration"
    );
    assert!(
        support::inbox_ids(&cluster, &accepts)
            .iter()
            .any(|id| id.starts_with('i')),
        "the invitee is offered the invitation"
    );

    // The accepted one round-trips: B accepts, both nodes show it accepted.
    cluster.ok_as(Some(&accepts), 1, &["accept", &thread]);
    for node in [0, 1] {
        cluster.wait_until("the invitation to be accepted", crate::WAIT, || {
            invitation(&cluster, node, &thread, &accepts.id)
                .filter(|row| row.0 == "accepted")
                .map(|_| ())
        });
    }
    assert!(
        !warned(&cluster, 0, &thread, &accepts.id),
        "an accepted invitation is not overdue"
    );

    // An unanswered one with a short deadline: it wakes its invitee once, and
    // the overdue warning is recorded on the inviter's node only.
    let missed = cluster.ok_as(Some(&a), 0, &["thread", "create", "--topic", "ignored"]);
    let missed = missed.as_str().expect("a thread id").to_owned();
    cluster.ok_as(
        Some(&a),
        0,
        &["invite", &missed, "--seat", &ignores.id, "--deadline", "1"],
    );
    wait_pending(&cluster, 1, &missed, &ignores.id);
    support::wait_wakes(&cluster, 1, P2, 1);
    cluster.wait_until("the overdue warning on node 0", crate::WAIT, || {
        warned(&cluster, 0, &missed, &ignores.id).then_some(())
    });
    cluster.wait_converged(crate::WAIT);
    assert!(
        !warned(&cluster, 1, &missed, &ignores.id),
        "the warning stays on the inviter's node"
    );
    assert_eq!(
        cluster
            .scalar(
                1,
                "SELECT count(*) FROM invitations WHERE state = 'pending'",
                &[]
            )
            .as_deref(),
        Some("1"),
        "the unanswered invitation is still pending on B"
    );
    let warnings = cluster.ok_as(None, 0, &["warnings", "--seat", &a.id, "--limit", "100"]);
    assert!(
        warnings["items"]
            .as_array()
            .is_some_and(|items| !items.is_empty()),
        "node 0 lists the overdue warning for the inviter: {warnings}"
    );
    support::wait_wakes(&cluster, 1, P2, 1);
}
