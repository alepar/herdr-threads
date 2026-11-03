//! H1-remote (ht-kqrt.60): a herdr-core terminal seat crosses nodes by the labels its home node
//! publishes (`SeatStatusEntry.pane`, the path N7 uses for named seats). The other node displays
//! it as `<session>:tN` and addresses it as `<session>:tN@node` or by its seat id.
//!
//! Node 1 ("A") has only a fake herdr-core as its host; node 0 ("B") has a fake Herdr server and
//! the sending seat.

use crate::support::{self, P1};
use herdr_threads::test_support::{
    fake_hc::{FakeHc, FakeTerminal},
    isolation::as_agent,
    multinode::{Cluster, ClusterOptions},
    spawn::SpawnOwned,
};
use std::io::Write;
use std::process::Stdio;

/// The session name `FakeHc` gives its core.
const SESSION: &str = "fakehost";

/// `A`'s hook-enrolled seat in terminal `t2` of `hc`: the SessionStart hook of a synthetic
/// `claude` agent with the core environment and nothing of Herdr (as `tests/hc_messaging.rs`).
fn enroll_terminal_seat(cluster: &Cluster, hc: &FakeHc, node: usize) -> String {
    let payload = br#"{"session_id":"s-t2","transcript_path":"/tmp/t.jsonl","cwd":"/tmp","hook_event_name":"SessionStart","source":"startup"}"#;
    let mut command = cluster.isolation().command(crate::BIN);
    command
        .arg("--state-dir")
        .arg(cluster.state_dir(node))
        .args(["hook", "claude"]);
    as_agent(&mut command, "claude", "s-t2");
    command
        .env("HC_TERMINAL_ID", "t2")
        .env("HC_TERMINAL_SOCKET", hc.socket())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn_owned().expect("spawn the hook");
    child.stdin.take().unwrap().write_all(payload).unwrap();
    let output = child.wait_with_output().expect("hook output");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    cluster
        .scalar(
            node,
            "SELECT id FROM seats WHERE target_id='t2' AND state<>'retired'",
            &[],
        )
        .expect("a seat on terminal t2")
}

/// Node 0 with a Herdr pane and a sending seat, node 1 on a fake herdr-core with the live
/// terminal `t2` and the seat the agent in it enrolled; converged both ways.
struct Rig {
    cluster: Cluster,
    hc: FakeHc,
    sender: herdr_threads::test_support::multinode::Seat,
    /// Node 1's seat in `t2`.
    hc_seat: String,
    /// Node 1's tag as node 0 knows it.
    tag_a: String,
}

fn rig(label: &str) -> Rig {
    let hc = FakeHc::start();
    hc.state().add_terminal(FakeTerminal::live("t2"));
    let mut cluster = Cluster::start(
        label,
        crate::BIN,
        2,
        ClusterOptions {
            links: vec![(0, 1)],
            // The seat publisher's production periods (5 s check, labels re-read at most every
            // 60 s) would dominate every wait here, the label change in particular.
            env: [0, 1]
                .into_iter()
                .flat_map(|node| {
                    [
                        (node, "HT_TEST_SEAT_CHECK_MS".to_owned(), "200".to_owned()),
                        (
                            node,
                            "HT_TEST_SEAT_REFRESH_MS".to_owned(),
                            "1000".to_owned(),
                        ),
                    ]
                })
                .collect(),
            ..Default::default()
        },
    );
    cluster.add_host(0, &[P1]);
    let sender = cluster.seat(0, P1);
    cluster.restart_with_host(1, hc.socket());
    let hc_seat = enroll_terminal_seat(&cluster, &hc, 1);
    cluster.wait_converged(crate::WAIT);
    support::wait_seats(&cluster, 0, 2);
    let a_uuid = cluster.node_uuid(1).to_string();
    let tag_a: String = cluster
        .scalar(
            0,
            "SELECT tag FROM node_directory WHERE node_uuid=?1",
            &[&a_uuid],
        )
        .expect("node A's tag");
    Rig {
        cluster,
        hc,
        sender,
        hc_seat,
        tag_a,
    }
}

/// The pane label node 0 holds for `seat` once node 1's status snapshot has arrived.
fn published_pane(cluster: &Cluster, seat: &str) -> String {
    cluster.wait_until("the hc seat's published pane", crate::WAIT, || {
        cluster
            .scalar(
                0,
                "SELECT pane FROM remote_seat_status WHERE seat_id=?1",
                &[&seat],
            )
            .filter(|p| !p.is_empty())
    })
}

// Kills: an hc seat whose label is not published through `SeatStatusEntry.pane`, a selector that
// reads only host pane paths and names (so `<session>:tN@node` is "neither a seat ID nor a pane
// name"), a `from` header or `thread show` row that falls back to the bare seat id, and a seat
// list that shows a remote terminal seat nameless.
#[test]
fn node_b_addresses_hc_seat_of_a() {
    let Rig {
        cluster,
        hc: _hc,
        sender,
        hc_seat,
        tag_a,
    } = rig("mn-hc-seats");
    let label = format!("{SESSION}:t2");
    assert_eq!(published_pane(&cluster, &hc_seat), label);

    // `<session>:tN@A` addresses the seat.
    let (code, _, stderr) = cluster.text_as(
        Some(&sender),
        0,
        &[
            "send",
            "--seat",
            &format!("{label}@{tag_a}"),
            "--body",
            "by label",
        ],
    );
    assert_eq!(code, 0, "{stderr}");
    assert!(
        stderr.contains(&format!("{label}@{tag_a}")) && stderr.contains(&hc_seat),
        "{stderr}"
    );
    // So does the seat id.
    let (code, _, stderr) = cluster.text_as(
        Some(&sender),
        0,
        &["send", "--seat", &hc_seat, "--body", "by id"],
    );
    assert_eq!(code, 0, "{stderr}");
    let delivered = cluster.wait_until("both messages to reach the hc seat", crate::WAIT, || {
        cluster
            .scalar(
                1,
                "SELECT count(*) FROM messages WHERE body IN ('by label','by id')",
                &[],
            )
            .filter(|n| n == "2")
    });
    assert_eq!(delivered, "2");

    // An unknown terminal of that core on A is a refusal that sends nothing.
    let (code, _, stderr) = cluster.text_as(
        Some(&sender),
        0,
        &[
            "send",
            "--seat",
            &format!("{SESSION}:t9@{tag_a}"),
            "--body",
            "nowhere",
        ],
    );
    assert_ne!(code, 0, "{stderr}");
    assert_eq!(
        cluster
            .scalar(1, "SELECT count(*) FROM messages WHERE body='nowhere'", &[])
            .as_deref(),
        Some("0")
    );

    // The label renders in `seat list`, once, as the pane part.
    let (code, text, stderr) = cluster.text_as(None, 0, &["seat", "list"]);
    assert_eq!(code, 0, "{stderr}");
    let row = text
        .lines()
        .find(|l| l.starts_with(&format!("{hc_seat} ")))
        .unwrap_or_else(|| panic!("{text}"));
    assert_eq!(row.matches(&label).count(), 1, "{row}");

    // The seat answers on A; the reply shows its label in the delivery `from` header on B and in
    // `thread show`.
    let thread = cluster
        .scalar(1, "SELECT thread_id FROM messages WHERE body='by id'", &[])
        .expect("the thread the sends opened");
    cluster.wait_until("the reply from A", crate::WAIT, || {
        let mut command = cluster.isolation().command(crate::BIN);
        command
            .arg("--state-dir")
            .arg(cluster.state_dir(1))
            .args(["send", &thread, "--body", "back"]);
        as_agent(&mut command, "claude", "s-t2");
        command
            .env("HC_TERMINAL_ID", "t2")
            .env("HC_TERMINAL_SOCKET", _hc.socket());
        let output = command.output().ok()?;
        output.status.success().then_some(())
    });
    cluster.wait_converged(crate::WAIT);
    let (code, shown, stderr) = cluster.text_as(Some(&sender), 0, &["thread", "show", &thread]);
    assert_eq!(code, 0, "{stderr}");
    assert!(shown.contains(&label), "thread show: {shown}");
    let (code, inbox, stderr) = cluster.text_as(Some(&sender), 0, &["inbox"]);
    assert_eq!(code, 0, "{stderr}");
    // Delivery headers name every sender by seat id (Herdr-pane and named seats alike): the
    // id is what the label resolves to.
    assert!(
        inbox.contains(&format!("from {hc_seat}")),
        "from header: {inbox}"
    );
}

// Kills: a status entry stored for a seat this node has not materialized from a `seat.created`
// (or whose home node is another), here with an hc pane label.
#[test]
fn remote_hc_entries_only_for_materialized_seats() {
    let Rig {
        cluster,
        hc: _hc,
        hc_seat,
        ..
    } = rig("mn-hc-guard");
    let label = format!("{SESSION}:t2");
    assert_eq!(published_pane(&cluster, &hc_seat), label);

    // B no longer holds the seat as a remote seat of A: A's next snapshot must not keep its entry.
    let home = cluster.node_uuid(1).to_string();
    let changed = cluster.write_sql(
        0,
        &format!(
            "UPDATE seats SET home_node='{}' WHERE id='{hc_seat}' AND home_node='{home}'",
            "00000000000000000000000000000001"
        ),
    );
    assert_eq!(changed, 1);
    // A republishes when its labels change.
    _hc.state()
        .set_labels("t2", &[("herdr.space", "w9"), ("herdr.tab", "x9")]);
    cluster.wait_until("the seat's entry to be dropped", crate::WAIT, || {
        cluster
            .scalar(
                0,
                "SELECT count(*) FROM remote_seat_status WHERE seat_id=?1",
                &[&hc_seat],
            )
            .filter(|n| n == "0")
    });
}
