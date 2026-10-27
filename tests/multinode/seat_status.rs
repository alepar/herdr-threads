//! Seat status (ht-xg88 phase 2): a node publishes its own seats' advisory
//! status (presence, harness, labels) on `seat_status`; its peer stores the
//! snapshot and shows it only as fresh as the publishing node's status
//! allows: once that node is unreachable the seat reads `Unreachable`, never
//! a stale presence, and the last-known labels survive a restart.

use crate::support::{self, P1, P2};
use herdr_threads::store::remote_seat_status::{RemotePresence, get, presence_of};
use herdr_threads::store::sync_contacts::{NodeReach, reach};
use herdr_threads::test_support::multinode::{Cluster, ClusterOptions};
use std::time::Duration;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// `node`'s view of remote `seat`: its stored row and the freshness rule's
/// answer, with the origin taken as live while `live`.
fn view(
    cluster: &Cluster,
    node: usize,
    seat: &str,
    live: bool,
) -> Option<(RemotePresence, String)> {
    let conn = cluster.open_store(node)?;
    let row = get(&conn, seat).ok()??;
    let origin_reach = reach(&conn, &row.origin, now_ms()).ok()?;
    let labels = format!(
        "{} / {} / {} {}",
        row.seat.workspace.clone().unwrap_or_default(),
        row.seat.tab.clone().unwrap_or_default(),
        row.seat.pane.clone().unwrap_or_default(),
        row.seat.harness.clone().unwrap_or_default()
    );
    Some((
        presence_of(Some(&row), origin_reach, live, now_ms()),
        labels,
    ))
}

// Kills: no seat status published or relayed, presence or labels lost on
// the way, a seat on an unreachable node still shown with its presence, and
// last-known labels gone after the observer restarts.
#[test]
fn remote_seat_status_follows_its_node() {
    let mut env = Vec::new();
    for node in [0, 1] {
        env.push((node, "HT_TEST_SEAT_CHECK_MS".to_owned(), "200".to_owned()));
        env.push((
            node,
            "HT_TEST_SEAT_REFRESH_MS".to_owned(),
            "60000".to_owned(),
        ));
    }
    let mut cluster = Cluster::start(
        "mn-seat-status",
        crate::BIN,
        2,
        ClusterOptions {
            links: vec![(0, 1)],
            env,
            ..Default::default()
        },
    );
    cluster.add_host(0, &[P1]);
    cluster.add_host(1, &[P1, P2]);
    let a = cluster.seat(0, P1);
    let b = cluster.seat(1, P1);
    support::wait_seats(&cluster, 0, 2);
    support::wait_seats(&cluster, 1, 2);
    cluster.wait_converged(crate::WAIT);
    support::send(&cluster, &a, &["--seat", &b.id, "--body", "hello"]);

    // Node 1's seat reaches node 0 with its presence, harness and labels
    // (the fake Herdr names nothing, so the labels are its ids).
    let (presence, labels) =
        cluster.wait_until("node 0 to hold b's seat status", crate::WAIT, || {
            view(&cluster, 0, &b.id, false)
                .filter(|(p, _)| matches!(p, RemotePresence::Fresh { .. }))
        });
    let RemotePresence::Fresh { presence, .. } = presence else {
        unreachable!()
    };
    assert!(["idle", "busy"].contains(&presence.as_str()), "{presence}");
    assert_eq!(labels, "w1 / w1:t1 / w1:p1 claude");
    // Node 0 does not describe its own seat from a snapshot.
    assert!(view(&cluster, 0, &a.id, false).is_none());

    // A change reaches node 0 within the check period, not the refresh: a
    // second seat appears on node 1.
    let c = cluster.seat(1, P2);
    support::wait_seats(&cluster, 0, 3);
    cluster.wait_until("node 0 to hold c's seat status", crate::WAIT, || {
        view(&cluster, 0, &c.id, false).filter(|(p, _)| matches!(p, RemotePresence::Fresh { .. }))
    });

    // Node 1 goes away. Past the reach threshold (aged; age_contact restarts
    // node 0) the seat is unreachable, never its stale presence, and its
    // last-known labels are still there after the restart.
    cluster.partition(0, 1);
    cluster.wait_until("node 0's dial to be refused", crate::WAIT, || {
        (cluster.target_state(0).as_deref() == Some("backoff")).then_some(())
    });
    cluster.age_contact(0, 1, Duration::from_secs(200));
    let (presence, labels) =
        cluster.wait_until("node 0 to read b unreachable", crate::WAIT, || {
            view(&cluster, 0, &b.id, false)
        });
    assert!(
        matches!(presence, RemotePresence::Unreachable { since: Some(_) }),
        "{presence:?}"
    );
    assert_eq!(
        labels, "w1 / w1:t1 / w1:p1 claude",
        "last-known labels kept"
    );
    let conn = cluster.open_store(0).unwrap();
    let origin = cluster.node_uuid(1);
    assert!(matches!(
        reach(&conn, &origin, now_ms()).unwrap(),
        NodeReach::Unreachable { .. }
    ));
    cluster.heal(0, 1);
}
