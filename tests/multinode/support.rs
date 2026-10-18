//! Shared steps of the configuration smoke suite (ht-5fr.11): stand-in seats
//! on fake Herdr servers, sends through the CLI, and the receipt and wake
//! observations every configuration asserts.

use crate::WAIT;
use herdr_threads::store::effective::{EffectiveReceiptState, effective_receipt};
use herdr_threads::test_support::multinode::{Cluster, ClusterOptions, Seat};
use herdr_threads::test_support::spawn::{OwnedChild, SpawnOwned};
use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::process::Stdio;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

/// The pane every node's fake Herdr server offers first.
pub const P1: &str = "w1:p1";
pub const P2: &str = "w1:p2";
pub const P3: &str = "w1:p3";

/// A cluster of `n` nodes where each `(dialer, dest)` pair is an explicit
/// dial target; every node in `hosted` gets a fake Herdr server with
/// [`P1`], [`P2`] and [`P3`].
pub fn cluster(label: &str, n: usize, links: &[(usize, usize)], hosted: &[usize]) -> Cluster {
    cluster_with_panes(label, n, links, hosted, &[P1, P2, P3])
}

/// [`cluster`] with `panes` on every hosted node's fake server, for a test
/// that needs one recipient seat per wake case (a seat is woken at most once
/// per 30 s).
pub fn cluster_with_panes(
    label: &str,
    n: usize,
    links: &[(usize, usize)],
    hosted: &[usize],
    panes: &[&str],
) -> Cluster {
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
        cluster.add_host(*node, panes);
    }
    cluster
}

/// Sets `seat`'s nudge threshold to `t` on its own node (the threshold is
/// node-local; a remote seat's is refused).
pub fn set_threshold(cluster: &Cluster, seat: &Seat, t: i32) {
    let value = t.to_string();
    cluster.ok_as(Some(seat), seat.node, &["nudge-threshold", &value]);
    assert_eq!(
        cluster
            .scalar(
                seat.node,
                "SELECT nudge_threshold FROM seats WHERE id = ?1",
                &[&seat.id]
            )
            .as_deref(),
        Some(value.as_str()),
        "{}'s threshold",
        seat.id
    );
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

fn seats_ready(cluster: &Cluster, node: usize, count: i64) -> bool {
    // Received journal facts can still be pending. Sends need their projection,
    // including publication of locally allocated seats, before resolving ids.
    cluster.scalar(node, "SELECT count(*) FROM seats WHERE published=1", &[])
        == Some(count.to_string())
}

/// Waits until `node` has materialized `count` published seats.
pub fn wait_seats(cluster: &Cluster, node: usize, count: i64) {
    cluster.wait_until(&format!("node {node} to know {count} seats"), WAIT, || {
        seats_ready(cluster, node, count).then_some(())
    });
}

#[test]
fn received_seat_facts_are_not_ready_until_materialized() {
    use herdr_threads::{
        app::SystemClock,
        node::NodeIdentity,
        protocol::time::{CallBudget, Cancellation, MonoInstant},
        store::{apply, connection::StoreContext, journal},
        sync::{BatchLimits, LogKey, facts::FactRegistry, journal_port::IngestOutcome},
        test_support::multinode::default_seed,
    };
    use std::sync::Arc;

    // No link: stage the actual remote envelopes without an Apply worker,
    // deterministically exposing the normal ingest-before-apply boundary.
    let mut cluster = cluster("mn-seat-ready", 2, &[], &[0, 1]);
    cluster.seat(0, P1);
    let remote = cluster.seat(1, P1);
    cluster.stop();
    let source = cluster.open_store(1).unwrap();
    let vector = cluster.vector(1);
    let log = LogKey {
        node: cluster.node_uuid(1),
        incarnation: vector.iter().map(|row| row.1 as u32).max().unwrap(),
    };
    let entries = journal::serve(&source, &log, 1, BatchLimits::default()).unwrap();
    let path = cluster.open_store(0).unwrap().path().unwrap().to_owned();
    let seed = default_seed(0);
    let ctx = StoreContext::new(path.into(), Arc::new(SystemClock::new())).with_journal_seed(seed);
    let mut writer = ctx.open_writer().unwrap();
    journal::open(
        &ctx,
        &mut writer,
        Arc::new(NodeIdentity::from_seed(seed)),
        "multinode",
        FactRegistry::builtins(),
    )
    .unwrap();
    let budget = CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Cancellation::default(),
    };
    let outcome = journal::ingest(&ctx, &mut writer, &log, entries, &budget).unwrap();
    assert!(
        matches!(outcome, IngestOutcome::Stored { .. }),
        "{outcome:?}"
    );
    assert_eq!(cluster.kinds(0).get("seat.created"), Some(&2));
    assert_eq!(
        cluster.scalar(0, "SELECT id FROM seats WHERE id=?1", &[&remote.id]),
        None,
        "the ingested remote seat is not visible to a send yet"
    );
    assert!(
        !seats_ready(&cluster, 0, 2),
        "receiving both seat facts must not release the readiness barrier"
    );
    ctx.execute_decision(
        &mut writer,
        |_| Ok(()),
        |tx, decision, ()| apply::apply_pass(tx, &ctx, decision, apply::ApplyLimits::default()),
    )
    .unwrap();
    assert!(seats_ready(&cluster, 0, 2));
    assert_eq!(
        cluster.scalar(0, "SELECT id FROM seats WHERE id=?1", &[&remote.id]),
        Some(remote.id)
    );
}

/// Waits until `node`'s first host has typed `count` wake markers into
/// `pane`, then watches a moment to prove it stays at `count`.
pub fn wait_wakes(cluster: &Cluster, node: usize, pane: &str, count: usize) {
    wait_wakes_at(cluster, node, 0, pane, count);
}

/// [`wait_wakes`] on the node's `host`th server.
pub fn wait_wakes_at(cluster: &Cluster, node: usize, host: usize, pane: &str, count: usize) {
    let fake = cluster.host_at(node, host);
    cluster.wait_until(
        &format!("{count} wake marker(s) in {pane} on node {node} host {host}"),
        WAIT,
        || (fake.prompts_for(pane) >= count).then_some(()),
    );
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(
        fake.prompts_for(pane),
        count,
        "node {node} host {host}'s {pane} was woken more than {count} time(s): {:?}",
        fake.prompts()
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

/// The logical receipt `node` holds for `(message, seat)`. A pending
/// obligation need not have a physical `receipt_state` row, so this reads the
/// canonical effective view.
pub fn effective_receipt_of(
    cluster: &Cluster,
    node: usize,
    message: &str,
    seat: &Seat,
) -> Option<EffectiveReceiptState> {
    let conn = cluster.open_store(node)?;
    effective_receipt(&conn, message, &seat.id)
        .ok()
        .flatten()
        .map(|receipt| receipt.state)
}

/// Whether `node`'s effective view of `(message, seat)` is pending.
pub fn is_pending(cluster: &Cluster, node: usize, message: &str, seat: &Seat) -> bool {
    effective_receipt_of(cluster, node, message, seat) == Some(EffectiveReceiptState::Pending)
}

/// Waits until `node`'s effective view of `(message, seat)` is pending.
pub fn wait_pending(cluster: &Cluster, node: usize, message: &str, seat: &Seat) {
    cluster.wait_until(
        &format!("node {node} to hold {message} pending for {}", seat.id),
        crate::WAIT,
        || is_pending(cluster, node, message, seat).then_some(()),
    );
}

/// The sender-node due-tracking rows (`receipt_due_sparse`) of `(message, seat)`.
pub fn due_rows(cluster: &Cluster, node: usize, message: &str, seat: &Seat) -> i64 {
    cluster
        .scalar(
            node,
            "SELECT count(*) FROM receipt_due_sparse WHERE message_id = ?1 AND seat_id = ?2",
            &[&message, &seat.id],
        )
        .and_then(|n| n.parse().ok())
        .expect("a due-row count")
}

/// How many derived `ack` info events `node` holds in `thread`, split by the
/// settlement path that authored them: `ack:` for a displayed/explicit
/// settlement and `ack_mod:` for a mod or policy completion.
pub fn ack_events(cluster: &Cluster, node: usize, thread: &str) -> (i64, i64) {
    let count = |prefix: &str| {
        cluster
            .scalar(
                node,
                "SELECT count(*) FROM messages WHERE kind = 'info' AND thread_id = ?1 \
                 AND substr(event_key, 1, length(?2)) = ?2",
                &[&thread, &prefix],
            )
            .and_then(|n| n.parse().ok())
            .expect("an event count")
    };
    (count("ack:"), count("ack_mod:"))
}

/// A `watch` child (the mod's channel) with its stdout lines forwarded by a
/// reader thread; dropping it kills the child.
pub struct Watch {
    _child: OwnedChild,
    lines: Receiver<Value>,
    seen: Vec<Value>,
    first: Option<Value>,
}

impl Watch {
    fn spawn(cluster: &Cluster, seat: &Seat) -> Self {
        let mut command = cluster.pane_command(seat);
        command
            .args(["watch", "--harness", "claude", "--session"])
            .arg(cluster.session_of(seat))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command.spawn_owned().expect("spawn the watch child");
        let stdout = child.stdout.take().expect("a piped stdout");
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Ok(value) = serde_json::from_str::<Value>(&line)
                    && tx.send(value).is_err()
                {
                    return;
                }
            }
        });
        Self {
            _child: child,
            lines,
            seen: Vec::new(),
            first: None,
        }
    }

    /// Waits for the next line satisfying `pred`; every line read is kept.
    pub fn wait_line(
        &mut self,
        what: &str,
        timeout: Duration,
        pred: impl Fn(&Value) -> bool,
    ) -> Value {
        let until = Instant::now() + timeout;
        loop {
            let next = match self.first.take() {
                Some(line) => Ok(line),
                None => self
                    .lines
                    .recv_timeout(until.saturating_duration_since(Instant::now())),
            };
            match next {
                Ok(line) => {
                    self.seen.push(line.clone());
                    if pred(&line) {
                        return line;
                    }
                }
                Err(_) => panic!("timed out waiting for {what}; saw {:#?}", self.seen),
            }
        }
    }

    pub fn wait_connected(&mut self) {
        self.wait_line("the connected status", crate::WAIT, |line| {
            line["kind"] == "status" && line["state"] == "connected"
        });
    }

    /// The `message` / `lazy` line carrying `id`.
    pub fn wait_item(&mut self, id: &str) -> Value {
        self.wait_line(&format!("item {id}"), crate::WAIT, |line| {
            line["id"] == id && matches!(line["kind"].as_str(), Some("message" | "lazy"))
        })
    }
}

/// The mod's `watch` child for `seat`. Like the mod, a first `no_binding`
/// status (the seat's check-in has not committed yet) is retried with a short
/// backoff; every other first answer is returned for the test to judge.
pub fn watch(cluster: &Cluster, seat: &Seat) -> Watch {
    for _ in 0..80 {
        let mut watch = Watch::spawn(cluster, seat);
        watch.first = watch.lines.recv_timeout(Duration::from_secs(10)).ok();
        let retry = watch
            .first
            .as_ref()
            .is_some_and(|line| line["kind"] == "status" && line["reason"] == "no_binding");
        if !retry {
            return watch;
        }
        drop(watch);
        std::thread::sleep(Duration::from_millis(150));
    }
    panic!("the check-in of {} never committed", seat.id);
}

/// `watch ack` as the mod runs it; returns the exit code and one value per
/// output line.
pub fn watch_ack(cluster: &Cluster, seat: &Seat, via: &str, ids: &[&str]) -> (i32, Vec<Value>) {
    let mut command = cluster.pane_command(seat);
    command
        .args(["watch", "ack", "--session"])
        .arg(cluster.session_of(seat))
        .args(["--via", via])
        .args(ids)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = command
        .spawn_owned()
        .expect("spawn watch ack")
        .wait_with_output()
        .expect("watch ack output");
    let lines = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    (output.status.code().unwrap_or(-1), lines)
}
