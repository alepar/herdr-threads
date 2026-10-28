//! Remote addresses (`SEAT@MACHINE`): a seat argument with a machine suffix
//! is resolved by that machine's own daemon against its live topology and
//! seat table; the asking node only learns the seat ID. A machine name that
//! matches several nodes is refused listing them; a peer without
//! `addr.resolve.v1` and an unreachable peer are refused, never resolved
//! locally.

use crate::support::{self, P1, P2};
use herdr_threads::test_support::multinode::{Cluster, ClusterOptions};

const P3: &str = "w1:p3";

/// `node`'s tag as `observer` knows it.
fn tag_of(cluster: &Cluster, observer: usize, node: usize) -> String {
    let uuid = cluster.node_uuid(node).to_string();
    cluster
        .scalar(
            observer,
            "SELECT tag FROM node_directory WHERE node_uuid=?1",
            &[&uuid],
        )
        .expect("a known node")
}

/// The seat `seat inspect ADDRESS` names, as `as_seat` on node 0.
fn inspect(
    cluster: &Cluster,
    as_seat: &herdr_threads::test_support::multinode::Seat,
    address: &str,
) -> Result<String, String> {
    let (code, value, stderr) = cluster.run_as(Some(as_seat), 0, &["seat", "inspect", address]);
    if code == 0 {
        Ok(value["result"]["data"]["summary"]["seat"]
            .as_str()
            .unwrap_or_default()
            .to_owned())
    } else {
        Err(format!("{stderr}{value}"))
    }
}

// Kills: a suffix resolved against the local topology, a remote seat not
// found by its path, the machine matched by only one of uuid/tag/name, a
// shared host name accepted, an unseated remote pane allocated or silently
// mapped, a capless peer answered by a local fallback, and an unreachable
// peer not reported.
#[test]
fn remote_addresses_resolve_on_their_home_node() {
    let mut env = Vec::new();
    for (node, label) in [(0, "alpha"), (1, "beta"), (2, "beta.lab")] {
        env.push((node, "HT_TEST_HOST_LABEL".to_owned(), label.to_owned()));
    }
    env.push((
        2,
        "HT_TEST_WITHHOLD_CAPS".to_owned(),
        "addr.resolve.v1".to_owned(),
    ));
    let cluster = {
        let mut cluster = Cluster::start(
            "mn-remote-addr",
            crate::BIN,
            3,
            ClusterOptions {
                links: vec![(0, 1), (0, 2)],
                env,
                ..Default::default()
            },
        );
        // Local labels that carry an `@`: one per collision case.
        let b_prefix = cluster.node_uuid(1).to_string()[..8].to_owned();
        let collide_remote = format!("w1:p6=w1:p1@{b_prefix}");
        cluster.add_host(
            0,
            &[
                P1,
                "w1:p4=w1:p1@beta.lab",
                "w1:p5=user@nowhere",
                &collide_remote,
                "w1:p7=w1:p1@alpha",
            ],
        );
        cluster.add_host(1, &[P1, P2]);
        cluster.add_host(2, &[P1, P3]);
        cluster
    };
    let a = cluster.seat(0, P1);
    let b = cluster.seat(1, P1);
    let c = cluster.seat(2, P1);
    support::wait_seats(&cluster, 0, 3);
    support::wait_seats(&cluster, 1, 3);
    cluster.wait_converged(crate::WAIT);
    let b_tag = tag_of(&cluster, 0, 1);
    let b_uuid = cluster.node_uuid(1).to_string();

    // A path on beta by tag, uuid prefix and full uuid: beta's own seat.
    // A 9-hex prefix: the 8-hex one is a colliding local label below.
    for machine in [b_tag.as_str(), &b_uuid[..9], &b_uuid] {
        let address = format!("w1/w1:t1/{P1}@{machine}");
        assert_eq!(
            inspect(&cluster, &a, &address),
            Ok(b.id.clone()),
            "{address}"
        );
    }
    // `beta` names both beta and beta.lab (its first DNS label): refused,
    // listing both uuids.
    let err = inspect(&cluster, &a, &format!("{P1}@beta")).unwrap_err();
    assert!(err.contains("ambiguous machine beta: 2 matches"), "{err}");
    assert!(err.contains(&b_uuid), "{err}");
    assert!(err.contains("pass @<uuid>"), "{err}");
    // A send by remote path reaches beta's seat.
    let (_, message) = support::send(
        &cluster,
        &a,
        &[
            "--seat",
            &format!("w1/w1:t1/{P1}@{b_tag}"),
            "--body",
            "over there",
        ],
    );
    cluster.wait_until("b to hold the message", crate::WAIT, || {
        support::inbox_ids(&cluster, &b)
            .contains(&message)
            .then_some(())
    });
    // An exact seat ID qualified by its own machine, and by another one.
    assert_eq!(
        inspect(&cluster, &a, &format!("{}@{b_tag}", b.id)),
        Ok(b.id.clone())
    );
    let err = inspect(&cluster, &a, &format!("{}@alpha", b.id)).unwrap_err();
    assert!(err.contains("lives on another machine"), "{err}");
    // An unseated pane on beta: refused, nothing allocated there.
    let err = inspect(&cluster, &a, &format!("{P2}@{b_tag}")).unwrap_err();
    assert!(err.contains("no seat at"), "{err}");
    // An unknown machine.
    let err = inspect(&cluster, &a, &format!("{P1}@nowhere")).unwrap_err();
    assert!(err.contains("no machine nowhere; known:"), "{err}");
    // Our own machine resolves locally (by tag: `w1:p1@alpha` is also a
    // local label, below).
    let a_tag = tag_of(&cluster, 0, 0);
    assert_eq!(
        inspect(&cluster, &a, &format!("{P1}@{a_tag}")),
        Ok(a.id.clone())
    );
    // beta.lab has no addr.resolve.v1: refused naming the cap, even though
    // the same path names a local pane.
    let err = inspect(&cluster, &a, &format!("{P1}@beta.lab")).unwrap_err();
    assert!(err.contains("needs addr.resolve.v1"), "{err}");
    assert!(err.contains(r"\@"), "the local escape hint: {err}");
    // A local `@` label whose suffix names no machine keeps its local
    // meaning (that pane holds no seat).
    let err = inspect(&cluster, &a, "user@nowhere").unwrap_err();
    assert!(err.contains("w1:p5"), "{err}");
    // A local label that is also a live remote address: both readings are
    // listed and nothing is picked.
    let err = inspect(&cluster, &a, &format!("w1:p1@{}", &b_uuid[..8])).unwrap_err();
    assert!(err.contains("2 readings"), "{err}");
    assert!(err.contains(r"\@"), "{err}");
    // The same with our own machine: the label and the plain location name
    // different local panes.
    let err = inspect(&cluster, &a, "w1:p1@alpha").unwrap_err();
    assert!(err.contains("2 readings"), "{err}");
    // Its exact seat ID still works.
    assert_eq!(inspect(&cluster, &a, &c.id), Ok(c.id.clone()));
    // beta unreachable: refused, the exact seat ID still works.
    cluster.partition(0, 1);
    cluster.wait_until("node 0's link to beta to drop", crate::WAIT, || {
        inspect(&cluster, &a, &format!("w1/w1:t1/{P1}@{b_tag}"))
            .err()
            .filter(|e| e.contains("unreachable"))
    });
    assert_eq!(inspect(&cluster, &a, &b.id), Ok(b.id.clone()));
    cluster.heal(0, 1);
}
