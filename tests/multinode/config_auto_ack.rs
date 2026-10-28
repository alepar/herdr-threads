//! Automatic ACK on the public display paths across two nodes (ht-5fr.49):
//! the sender's and the recipient's seats live on different nodes and the
//! message requests an ACK. A full text `inbox` display and a mod-channel
//! `watch ack` on the recipient's node each author exactly one
//! `receipt.settled`; it replicates to the sender's node, which shows the
//! receipt acknowledged and its due tracking cleared. A truncated display, a
//! read-only read and a refused completion settle nothing on either node, and
//! a settlement made while partitioned applies once after the heal and after a
//! sender restart.

use crate::WAIT;
use crate::support::{self, P1};
use herdr_threads::protocol::pagination::DEFAULT_PAGE_BYTES;
use herdr_threads::protocol::watch::WATCH_BODY_LIMIT_BYTES;
use herdr_threads::test_support::multinode::{Cluster, Seat};

/// Sends an ACK request from `a` to `b` and returns `(thread, message)`.
fn send_ack_request(
    cluster: &Cluster,
    a: &Seat,
    b: &Seat,
    body: &str,
    deadline: Option<&str>,
) -> (String, String) {
    let mut args = vec!["--seat", &b.id, "--body", body, "--require-ack", &b.id];
    if let Some(deadline) = deadline {
        args.extend(["--deadline", deadline]);
    }
    support::send(cluster, a, &args)
}

/// Both nodes agree that the settlement happened exactly once through the
/// displayed path (`ack:` event) or the mod path (`ack_mod:` event): receipt
/// acked, one `receipt.settled` applied, one derived ack event of that path,
/// and the sender node's due tracking and warning link cleared.
fn assert_settled_once(
    cluster: &Cluster,
    a: &Seat,
    b: &Seat,
    thread: &str,
    message: &str,
    mod_path: bool,
) {
    for node in [0, 1] {
        support::wait_receipt(cluster, node, message, b, "acked");
    }
    cluster.wait_converged(WAIT);
    let expected = if mod_path { (0, 1) } else { (1, 0) };
    for node in [0, 1] {
        assert_eq!(
            cluster.kinds(node).get("receipt.settled"),
            Some(&1),
            "node {node}: one settlement fact"
        );
        assert_eq!(
            cluster.rows(
                node,
                "SELECT apply_state FROM journal WHERE kind = 'receipt.settled'"
            ),
            vec![vec!["applied".to_owned()]],
            "node {node}: the settlement applied"
        );
        assert_eq!(
            support::ack_events(cluster, node, thread),
            expected,
            "node {node}: the ack event names the settlement path (displayed, mod)"
        );
    }
    assert_eq!(
        support::due_rows(cluster, a.node, message, b),
        0,
        "the sender node's due tracking is cleared"
    );
    assert_eq!(
        cluster.scalar(
            0,
            "SELECT warning_message_id IS NULL FROM receipt_state \
             WHERE message_id = ?1 AND seat_id = ?2",
            &[&message, &b.id]
        ),
        Some("1".to_owned()),
        "no overdue warning was raised"
    );
}

fn two_seats(label: &str) -> (Cluster, Seat, Seat) {
    let cluster = support::cluster(label, 2, &[(0, 1)], &[0, 1]);
    let a = cluster.seat(0, P1);
    let b = cluster.seat(1, P1);
    support::wait_seats(&cluster, 0, 2);
    support::wait_seats(&cluster, 1, 2);
    (cluster, a, b)
}

// Case 1: a full text inbox display on the recipient's node settles the
// remote sender's receipt and clears its due tracking.
#[test]
fn text_inbox_display_clears_the_remote_senders_due_tracking() {
    let (cluster, a, b) = two_seats("mn-auto-ack-text");
    let (thread, message) = send_ack_request(&cluster, &a, &b, "due body", Some("3600"));
    support::wait_pending(&cluster, 1, &message, &b);
    support::wait_pending(&cluster, 0, &message, &b);
    // The sender node tracks the pending deadline.
    support::wait_due_rows(&cluster, 0, &message, &b, 1);
    assert_eq!(support::due_rows(&cluster, 1, &message, &b), 0);

    let (code, stdout, stderr) = cluster.text_as(Some(&b), 1, &["inbox"]);
    assert_eq!(code, 0, "text inbox failed: {stderr}\n{stdout}");
    assert!(stdout.contains("due body"), "{stdout}");

    assert_settled_once(&cluster, &a, &b, &thread, &message, false);
}

// Case 2: the mod channel's completion (`watch ack`) settles it as a mod
// delivery; streaming the item completes nothing.
#[test]
fn mod_channel_delivery_settles_the_remote_senders_receipt() {
    let (cluster, a, b) = two_seats("mn-auto-ack-mod");
    let mut watch = support::watch(&cluster, &b);
    watch.wait_connected();
    let (thread, message) = send_ack_request(&cluster, &a, &b, "mod body", Some("3600"));
    let line = watch.wait_item(&message);
    assert_eq!(line["ack_required"], true, "{line}");
    assert_ne!(line["truncated"], true, "{line}");
    assert!(support::is_pending(&cluster, 1, &message, &b));
    support::wait_due_rows(&cluster, 0, &message, &b, 1);

    let (code, lines) = support::watch_ack(&cluster, &b, "append", &[&message]);
    assert_eq!(code, 0, "{lines:?}");
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0]["id"], message.as_str(), "{lines:?}");
    assert_eq!(lines[0]["result"], "settled", "{lines:?}");

    assert_settled_once(&cluster, &a, &b, &thread, &message, true);

    let (code, lines) = support::watch_ack(&cluster, &b, "append", &[&message]);
    assert_eq!(code, 0, "{lines:?}");
    assert_eq!(lines[0]["result"], "already_settled", "{lines:?}");
    cluster.wait_converged(WAIT);
    for node in [0, 1] {
        assert_eq!(
            cluster.kinds(node).get("receipt.settled"),
            Some(&1),
            "node {node}: a repeat completion authors no second fact"
        );
    }
    drop(watch);
}

// Case 3: a truncated display, a read-only read and a refused completion
// leave the receipt pending on both nodes.
#[test]
fn incomplete_read_only_and_refused_displays_leave_the_receipt_pending() {
    let (cluster, a, b) = two_seats("mn-auto-ack-negative");

    // (a) A text display that cannot show the whole body.
    let huge = "x".repeat(DEFAULT_PAGE_BYTES as usize + 4096);
    let (_, truncated) = send_ack_request(&cluster, &a, &b, &huge, None);
    support::wait_pending(&cluster, 1, &truncated, &b);
    let (code, stdout, stderr) = cluster.text_as(Some(&b), 1, &["inbox"]);
    assert_eq!(code, 0, "truncated text inbox failed: {stderr}\n{stdout}");
    assert!(stdout.contains(&truncated), "{stdout}");
    assert!(
        stdout.matches('x').count() < huge.len(),
        "the display shows the first chunk only"
    );
    assert!(stdout.contains("next:"), "a continuation line: {stdout}");

    // (b) Read-only reads: `--json` and `--machine`.
    let (_, passive) = send_ack_request(&cluster, &a, &b, "read only body", Some("3600"));
    support::wait_pending(&cluster, 1, &passive, &b);
    // The truncated message still heads the inbox, so the first JSON page holds
    // only it; the read-only reads continue from its cursor to this message.
    let (code, value, stderr) = cluster.run_as(Some(&b), 1, &["inbox"]);
    assert_eq!(code, 0, "json inbox failed: {stderr}{value}");
    let cursor = value["result"]["data"]["next_cursor"]
        .as_str()
        .unwrap_or_else(|| panic!("a continuation cursor: {value}"))
        .to_owned();
    let (code, value, stderr) = cluster.run_as(Some(&b), 1, &["inbox", "--cursor", &cursor]);
    assert_eq!(code, 0, "json inbox page 2 failed: {stderr}{value}");
    assert!(value.to_string().contains(&passive), "{value}");
    let (code, stdout, stderr) =
        cluster.text_as(Some(&b), 1, &["inbox", "--machine", "--cursor", &cursor]);
    assert_eq!(code, 0, "machine inbox failed: {stderr}\n{stdout}");
    assert!(stdout.contains("read only body"), "{stdout}");

    // (c) A completion the daemon refuses: the streamed body was cut.
    let mut watch = support::watch(&cluster, &b);
    watch.wait_connected();
    let big = "t".repeat(WATCH_BODY_LIMIT_BYTES + 1000);
    let (_, refused) = send_ack_request(&cluster, &a, &b, &big, None);
    let line = watch.wait_item(&refused);
    assert_eq!(line["truncated"], true, "{line}");
    let (code, lines) = support::watch_ack(&cluster, &b, "append", &[&refused]);
    assert_eq!(code, 0, "{lines:?}");
    assert_eq!(lines[0]["result"], "refused_terminal", "{lines:?}");
    assert_eq!(lines[0]["reason"], "truncated", "{lines:?}");

    cluster.wait_converged(WAIT);
    for node in [0, 1] {
        for message in [&truncated, &passive, &refused] {
            assert!(
                support::is_pending(&cluster, node, message, &b),
                "node {node}: {message} stays pending"
            );
        }
        assert_eq!(
            cluster.kinds(node).get("receipt.settled"),
            None,
            "node {node}: nothing was settled"
        );
    }
    // The sender node still tracks the read-only message's deadline.
    support::wait_due_rows(&cluster, 0, &passive, &b, 1);
    drop(watch);
}

// Case 4: the recipient node settles while the link is down; after the heal
// and after the sender restarts, exactly one settlement applies there.
#[test]
fn offline_settlement_applies_once_after_heal_and_resync() {
    let (mut cluster, a, b) = two_seats("mn-auto-ack-offline");
    let (thread, message) = send_ack_request(&cluster, &a, &b, "offline body", Some("3600"));
    support::wait_pending(&cluster, 1, &message, &b);
    support::wait_pending(&cluster, 0, &message, &b);
    cluster.wait_converged(WAIT);
    cluster.partition(0, 1);

    let (code, stdout, stderr) = cluster.text_as(Some(&b), 1, &["inbox"]);
    assert_eq!(code, 0, "offline text inbox failed: {stderr}\n{stdout}");
    support::wait_receipt(&cluster, 1, &message, &b, "acked");
    assert_eq!(cluster.kinds(1).get("receipt.settled"), Some(&1));
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert!(
        support::is_pending(&cluster, 0, &message, &b),
        "the cut sender node still sees it pending"
    );
    support::wait_due_rows(&cluster, 0, &message, &b, 1);
    assert_eq!(cluster.kinds(0).get("receipt.settled"), None);
    let (code, _, stderr) = cluster.text_as(Some(&b), 1, &["inbox"]);
    assert_eq!(code, 0, "repeat text inbox failed: {stderr}");
    assert_eq!(
        cluster.kinds(1).get("receipt.settled"),
        Some(&1),
        "a repeat display authors no second fact"
    );

    cluster.heal(0, 1);
    assert_settled_once(&cluster, &a, &b, &thread, &message, false);

    cluster.stop_node(0);
    cluster.start_node(0);
    cluster.wait_converged(WAIT);
    assert_settled_once(&cluster, &a, &b, &thread, &message, false);
}
