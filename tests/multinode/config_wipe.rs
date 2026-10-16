//! Configurations 11 and 11a (spec §13): a wiped store with the key intact, and
//! a daemon cold-started after a deadline passed while a peer holds the
//! recipient's in-time settlement.

use crate::support::{self, P1};
use herdr_threads::test_support::multinode::{Cluster, ClusterOptions, Seat};
use std::time::{Duration, Instant};

/// The open overdue warnings `node` lists for `seat`.
fn warnings(cluster: &Cluster, node: usize, seat: &Seat) -> usize {
    let data = cluster.ok_as(
        None,
        node,
        &["warnings", "--seat", &seat.id, "--limit", "100"],
    );
    data["items"].as_array().map_or(0, Vec::len)
}

/// 11: B's seat ACKs A's message; B's store is moved aside (key kept). B restarts
/// store-only and pulls A's log and its own old log back: its own seat waits for
/// a host (`parked_dep` on `wait:host`) and is restored when one attaches, with
/// no `sync unapplied replay`. The settled message wakes nothing and is not
/// offered, and an unsettled `-p 0` sent while B was down wakes the repaired
/// seat once.
#[test]
fn wipe_with_the_key_intact_suppresses_settled_history() {
    let mut cluster = support::cluster("mn-wipe", 2, &[(0, 1)], &[0, 1]);
    let a = cluster.seat(0, P1);
    let b = cluster.seat(1, P1);
    support::wait_seats(&cluster, 0, 2);
    support::wait_seats(&cluster, 1, 2);

    // A settled exchange: a `--require-ack` send (nudge-class), woken once and ACKed.
    let (_, settled) = support::send(
        &cluster,
        &a,
        &["--seat", &b.id, "--body", "settled", "--require-ack", &b.id],
    );
    support::wait_wakes(&cluster, 1, P1, 1);
    support::ack(&cluster, &b, &settled);
    support::wait_receipt(&cluster, 0, &settled, &b, "acked");
    cluster.wait_converged(crate::WAIT);

    // B is down; an unsettled `-p 0` is sent meanwhile; B's store is wiped.
    cluster.stop_node(1);
    let (_, unsettled) = support::send(
        &cluster,
        &a,
        &["--seat", &b.id, "--body", "while down", "-p", "0"],
    );
    cluster.wipe_store(1);
    cluster.wait_until("B's own seat to wait for a host", crate::WAIT, || {
        (cluster
            .scalar(
                1,
                "SELECT apply_state||' '||coalesce(parked_on,'') FROM journal \
                 WHERE kind='seat.created' AND creates=?1",
                &[&b.id],
            )
            .as_deref()
            == Some("parked_dep wait:host"))
        .then_some(())
    });
    assert_eq!(
        cluster
            .scalar(1, "SELECT count(*) FROM herdr_hosts", &[])
            .as_deref(),
        Some("0"),
        "B restarted store-only"
    );
    assert_eq!(
        cluster
            .scalar(1, "SELECT count(*) FROM seats WHERE id = ?1", &[&b.id])
            .as_deref(),
        Some("0"),
        "B's seat is absent until a host attaches"
    );
    cluster.attach_host(&b);
    cluster.wait_until("B to pull A's log and its own old log", crate::WAIT, || {
        (support::receipt(&cluster, 1, &settled, &b).as_deref() == Some("acked")
            && cluster
                .scalar(1, "SELECT id FROM messages WHERE id = ?1", &[&unsettled])
                .is_some())
        .then_some(())
    });
    cluster.wait_converged(crate::WAIT);
    // The restored seat is B's own again, unresolved until its operator rebinds it.
    assert_eq!(
        cluster
            .scalar(1, "SELECT state FROM seats WHERE id = ?1", &[&b.id])
            .as_deref(),
        Some("unresolved")
    );
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(
        cluster.host(1).prompts().len(),
        1,
        "settled history wakes nothing: {:?}",
        cluster.host(1).prompts()
    );

    // The operator rebinds the seat to its pane and the agent checks in again (a
    // fresh lifecycle event: the old one is in the CLI's intent journal).
    let out = cluster.human(1, &["seat", "rebind", &b.id, "--pane", P1, "--operator"]);
    assert!(out.status.success(), "{out:?}");
    let begun = Instant::now();
    loop {
        let (code, _, stderr) = cluster.run_as(
            Some(&b),
            1,
            &["check-in", "--lifecycle-event", "agent-restart"],
        );
        assert_eq!(code, 0, "{stderr}");
        if cluster
            .scalar(1, "SELECT count(*) FROM occupant_bindings", &[])
            .as_deref()
            != Some("0")
        {
            break;
        }
        assert!(
            begun.elapsed() < crate::WAIT,
            "the check-in never registered"
        );
        std::thread::sleep(Duration::from_millis(500));
    }
    // The unsettled message wakes the seat once; the settled one is not offered.
    support::wait_wakes(&cluster, 1, P1, 2);
    // The unsettled message is a pending delivery. The settled one has none:
    // the pair's read floor is its first local position (an auto-joined pair
    // seat sees the opener), so a wiped seat's inbox walk still lists it as
    // history, but it is already acknowledged and never a delivery to wake for.
    let inbox = cluster.ok_as(Some(&b), 1, &["inbox"]);
    let item = |id: &str| {
        inbox["items"]
            .as_array()
            .and_then(|items| items.iter().find(|item| item["message"] == id))
            .cloned()
    };
    let pending = item(&unsettled).unwrap_or_else(|| panic!("{unsettled} offered: {inbox}"));
    assert_eq!(pending["wakes"], true, "{pending}");
    if let Some(history) = item(&settled) {
        assert_eq!(history["kind"], "message", "no delivery row: {history}");
        assert!(history["ack_candidate"].is_null(), "{history}");
    }
    assert_eq!(
        cluster.rows(
            1,
            "SELECT message_id FROM lazy_recipients WHERE state = 'pending'"
        ),
        vec![vec![unsettled.clone()]],
        "only the unsettled message is a pending delivery"
    );
    assert_eq!(
        support::receipt(&cluster, 1, &settled, &b).as_deref(),
        Some("acked")
    );
    cluster.wait_converged(crate::WAIT);
}

/// 11a: A's seat sends a `--require-ack` with a short deadline; B's seat ACKs in
/// time while A's daemon is stopped; A restarts after the deadline. A holds
/// overdue emission until its first sync round completes, so it emits no warning
/// and the settlement applies. With B unreachable it warns only once the hold's
/// cap passes (shortened by `HT_TEST_SYNC_HOLD_MS`; the constant is 60 s).
#[test]
fn cold_start_after_the_deadline_holds_overdue_until_a_round_completes() {
    const HOLD_MS: u64 = 12_000;
    let mut cluster = Cluster::start(
        "mn-cold-start",
        crate::BIN,
        2,
        ClusterOptions {
            links: vec![(0, 1)],
            env: vec![(0, "HT_TEST_SYNC_HOLD_MS".into(), HOLD_MS.to_string())],
            ..Default::default()
        },
    );
    cluster.add_host(0, &[P1]);
    cluster.add_host(1, &[P1]);
    let a = cluster.seat(0, P1);
    let b = cluster.seat(1, P1);
    support::wait_seats(&cluster, 0, 2);
    support::wait_seats(&cluster, 1, 2);

    // B ACKs in time while A is down; A comes back after the deadline.
    let (_, timely) = support::send(
        &cluster,
        &a,
        &[
            "--seat",
            &b.id,
            "--body",
            "in time",
            "--require-ack",
            &b.id,
            "--deadline",
            "3",
        ],
    );
    let sent = Instant::now();
    support::wait_receipt(&cluster, 1, &timely, &b, "pending");
    cluster.wait_converged(crate::WAIT);
    cluster.stop_node(0);
    support::ack(&cluster, &b, &timely);
    while sent.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(100));
    }
    cluster.start_node(0);
    // The settlement applies after A's first sync round, and no warning was emitted.
    support::wait_receipt(&cluster, 0, &timely, &b, "acked");
    std::thread::sleep(Duration::from_secs(7));
    assert_eq!(
        warnings(&cluster, 0, &a),
        0,
        "no overdue warning for a deadline B met"
    );

    // With B unreachable A cannot learn that, and warns only after the cap.
    let (_, missed) = support::send(
        &cluster,
        &a,
        &[
            "--seat",
            &b.id,
            "--body",
            "never ACKed",
            "--require-ack",
            &b.id,
            "--deadline",
            "3",
        ],
    );
    let sent = Instant::now();
    support::wait_receipt(&cluster, 1, &missed, &b, "pending");
    cluster.wait_converged(crate::WAIT);
    // Unreachable: every dial of A is refused (a dial would start B's daemon).
    cluster.stop_node(0);
    cluster.partition(0, 1);
    while sent.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(100));
    }
    cluster.start_node(0);
    let started = Instant::now();
    while started.elapsed() < Duration::from_millis(HOLD_MS - 5_000) {
        assert_eq!(
            warnings(&cluster, 0, &a),
            0,
            "the hold keeps A quiet ({:?} after start)",
            started.elapsed()
        );
        std::thread::sleep(Duration::from_millis(500));
    }
    cluster.wait_until(
        "A to warn once the hold's cap passed",
        Duration::from_secs(40),
        || (warnings(&cluster, 0, &a) > 0).then_some(()),
    );
    assert!(
        started.elapsed() >= Duration::from_millis(HOLD_MS),
        "the warning came after the cap, not before ({:?})",
        started.elapsed()
    );
}
