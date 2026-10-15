//! Shared steps of the configuration smoke suite (ht-5fr.11): stand-in seats
//! on fake Herdr servers, sends through the CLI, and the receipt and wake
//! observations every configuration asserts.

use crate::WAIT;
use herdr_threads::test_support::multinode::{Cluster, ClusterOptions, Seat};
use std::time::Duration;

/// The pane every node's fake Herdr server offers first.
pub const P1: &str = "w1:p1";
pub const P2: &str = "w1:p2";

/// A cluster of `n` nodes where each `(dialer, dest)` pair is an explicit
/// dial target; every node in `hosted` gets a fake Herdr server with
/// [`P1`] and [`P2`].
pub fn cluster(label: &str, n: usize, links: &[(usize, usize)], hosted: &[usize]) -> Cluster {
    let mut cluster = Cluster::start(
        label,
        crate::BIN,
        n,
        ClusterOptions {
            links: links.to_vec(),
            ..Default::default()
        },
    );
    for node in hosted {
        cluster.add_host(*node, &[P1, P2]);
    }
    cluster
}

/// Sends as `from` and returns `(thread, message)`.
pub fn send(cluster: &Cluster, from: &Seat, args: &[&str]) -> (String, String) {
    let mut argv = vec!["send"];
    argv.extend_from_slice(args);
    let (code, value, stderr) = cluster.run_as(Some(from), from.node, &argv);
    assert_eq!(code, 0, "{argv:?} as {}: {stderr}{value}", from.id);
    let result = &value["result"];
    (
        result["thread"].as_str().expect("a thread id").to_owned(),
        result["data"].as_str().expect("a message id").to_owned(),
    )
}

/// The ids in `inbox` as `seat` sees them (invitation and message items).
pub fn inbox_ids(cluster: &Cluster, seat: &Seat) -> Vec<String> {
    let data = cluster.ok_as(Some(seat), seat.node, &["inbox"]);
    data["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    item["message"]
                        .as_str()
                        .or_else(|| item["invitation"].as_str())
                })
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// The receipt state `node` holds for `(message, seat)`, `None` while absent.
pub fn receipt(cluster: &Cluster, node: usize, message: &str, seat: &Seat) -> Option<String> {
    cluster.scalar(
        node,
        "SELECT state FROM receipt_state WHERE message_id = ?1 AND seat_id = ?2",
        &[&message, &seat.id],
    )
}

/// Waits until `node` holds the receipt of `(message, seat)` in `state`.
pub fn wait_receipt(cluster: &Cluster, node: usize, message: &str, seat: &Seat, state: &str) {
    cluster.wait_until(
        &format!("node {node} to hold {message} for {} as {state}", seat.id),
        WAIT,
        || (receipt(cluster, node, message, seat).as_deref() == Some(state)).then_some(()),
    );
}

/// Waits until `node` knows `count` seats through `seat.created`.
pub fn wait_seats(cluster: &Cluster, node: usize, count: i64) {
    cluster.wait_until(&format!("node {node} to know {count} seats"), WAIT, || {
        (cluster.kinds(node).get("seat.created") == Some(&count)).then_some(())
    });
}

/// Waits until `host`'s pane has been typed `count` wake markers, then keeps
/// watching a moment to prove it stays at `count`.
pub fn wait_wakes(cluster: &Cluster, node: usize, pane: &str, count: usize) {
    cluster.wait_until(
        &format!("{count} wake marker(s) in {pane} on node {node}"),
        WAIT,
        || (cluster.host(node).prompts_for(pane) >= count).then_some(()),
    );
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(
        cluster.host(node).prompts_for(pane),
        count,
        "node {node}'s {pane} was woken more than {count} time(s): {:?}",
        cluster.host(node).prompts()
    );
}

/// Acknowledges `message` as `seat`, asserting it is acknowledged now.
pub fn ack(cluster: &Cluster, seat: &Seat, message: &str) {
    let data = cluster.ok_as(Some(seat), seat.node, &["ack", message]);
    assert_eq!(
        data["acknowledged"],
        serde_json::json!([message]),
        "ack of {message} as {}: {data}",
        seat.id
    );
}
