//! Priority/threshold configuration matrix on a real daemon and the real CLI. One isolated
//! daemon (own state dir, scripted stand-in Herdr, children from `test_support::spawn`) per
//! nudge threshold T in {-1, 0, 3}; the recipient R seat sets T, the sender S seat acts. One
//! decision per configuration, each listed here as the command sequence that produces it:
//!
//! - delivery row, per p in {0, 1, 5}: `nudge-threshold T` (R); `send THREAD -p p` (S)
//!   -> wake frontier present iff `T < 0 || p <= T`, the inbox item's `wakes` agrees, no
//!   receipt for the message; `inbox` (R) settles it and the frontier clears.
//! - explicit ACK: `send THREAD --require-ack R --deadline 60` (S) -> a receipt for R only,
//!   pending-receipt wake at every T; `inbox` (R) acks it.
//! - invitation: `thread create` + `invite THREAD2 --seat R` (S) -> recorded `require_ack`
//!   with a deadline, pending-invitation wake at every T; `accept THREAD2` (R) clears it.
//! - 1:1 first send: `send --seat R --body .. -p 1` (S) -> both seats joined, no invitation,
//!   wake frontier iff `T < 0 || 1 <= T`.
//! - mixed order (T=0 daemon): `send -p 5`, `send -p 1`, `send -p 0`, `invite THREAD3 --seat R`,
//!   `send --require-ack R --deadline 60` (S) -> `inbox --json` (R) lists ACK message,
//!   invitation, p0, p1, p5 (priority, deadline with none last, then arrival).
use super::lazy_config_smoke::World;
use herdr_threads::store::attention;
use serde_json::Value;
use std::time::{Duration, Instant};

#[derive(Debug, PartialEq, Eq)]
struct Wake {
    delivery_row: bool,
    invitation: bool,
    receipt: bool,
}

/// The predicate under test, restated: a negative threshold admits every priority.
fn admits(threshold: i64, priority: i64) -> bool {
    threshold < 0 || priority <= threshold
}

fn wake(w: &World) -> Wake {
    let db = w.db();
    let seen = attention::wake_seat_attention(&db, &w.seats[1])
        .unwrap()
        .attention;
    Wake {
        delivery_row: seen.frontier.lazy.is_some(),
        invitation: seen.has_pending_invitation,
        receipt: seen.has_pending_receipt,
    }
}

/// Polls `cond` for up to 10 s; publication of a send is asynchronous to the CLI return.
fn eventually(what: &str, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "timed out: {what}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn count(w: &World, sql: &str, id: &str) -> i64 {
    w.db().query_row(sql, [id], |r| r.get(0)).unwrap()
}

fn receipts_for(w: &World, message: &str) -> i64 {
    count(
        w,
        "SELECT (SELECT count(*) FROM receipts WHERE message_id=?1)+(SELECT count(*) FROM receipt_state WHERE message_id=?1)",
        message,
    )
}

fn invitations_total(w: &World) -> i64 {
    w.db()
        .query_row("SELECT count(*) FROM invitations", [], |r| r.get(0))
        .unwrap()
}

fn published_row(w: &World, message: &str) -> bool {
    w.db()
        .query_row(
            "SELECT count(*) FROM lazy_recipients WHERE message_id=?1 AND seat_id=?2 AND decision_seq IS NOT NULL",
            [message, &w.seats[1]],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
        == 1
}

fn inbox_items(w: &World) -> Vec<Value> {
    w.ok(Some(1), false, &["inbox"])["data"]["items"]
        .as_array()
        .unwrap()
        .clone()
}

fn item_id(item: &Value) -> String {
    let key = if item["kind"] == "invitation" {
        "invitation"
    } else {
        "message"
    };
    item[key].as_str().unwrap().to_string()
}

fn new_thread(w: &World, topic: &str) -> String {
    w.ok(Some(0), false, &["thread", "create", "--topic", topic])["data"]
        .as_str()
        .unwrap()
        .to_string()
}

fn send_ack(w: &World) -> String {
    w.send(
        "ack body",
        &["--require-ack", &w.seats[1], "--deadline", "60"],
    )
}

fn invite(w: &World, topic: &str) -> (String, String) {
    let thread = new_thread(w, topic);
    let invitation = w.ok(Some(0), false, &["invite", &thread, "--seat", &w.seats[1]])["data"]
        .as_str()
        .unwrap()
        .to_string();
    (thread, invitation)
}

fn configuration(threshold: i64) {
    let w = World::agents_only();
    let t = threshold.to_string();
    let args: Vec<&str> = if threshold < 0 {
        vec!["nudge-threshold", "--", &t]
    } else {
        vec!["nudge-threshold", &t]
    };
    w.ok(Some(1), false, &args);
    let shown = w.ok(Some(1), false, &["nudge-threshold"]);
    assert_eq!(shown["data"]["threshold"], threshold.max(-1), "{shown}");

    // Delivery rows: wake iff admitted by T, never a receipt.
    for p in [0_i64, 1, 5] {
        let ps = p.to_string();
        let admitted = admits(threshold, p);
        let id = w.send(&format!("p{p} body"), &["-p", &ps]);
        eventually("delivery row published", || published_row(&w, &id));
        assert_eq!(
            wake(&w),
            Wake {
                delivery_row: admitted,
                invitation: false,
                receipt: false
            },
            "T={threshold} p={p}"
        );
        assert_eq!(
            receipts_for(&w, &id),
            0,
            "T={threshold} p={p}: no receipt without --require-ack"
        );
        let items = inbox_items(&w);
        let item = items
            .iter()
            .find(|i| item_id(i) == id)
            .expect("pending row listed");
        assert_eq!(item["priority"], p, "{item}");
        assert_eq!(
            item["wakes"].as_bool().unwrap_or(false),
            admitted && item["kind"] == "lazy_message",
            "T={threshold} p={p}: per-item wakes: {item}"
        );
        w.text(1, false, &["inbox"]);
        assert_eq!(w.state(&id, 1), "displayed");
        assert_eq!(
            wake(&w),
            Wake {
                delivery_row: false,
                invitation: false,
                receipt: false
            }
        );
    }

    // Explicit ACK: receipt for R only, wake at every T.
    let ack = send_ack(&w);
    eventually("ack receipt pending", || wake(&w).receipt);
    assert_eq!(
        wake(&w),
        Wake {
            delivery_row: false,
            invitation: false,
            receipt: true
        },
        "T={threshold}"
    );
    assert_eq!(
        receipts_for(&w, &ack),
        1,
        "T={threshold}: exactly R's receipt"
    );
    let db = w.db();
    let mut stmt = db
        .prepare("SELECT seat_id FROM receipts WHERE message_id=?1 UNION SELECT seat_id FROM receipt_state WHERE message_id=?1")
        .unwrap();
    let owners: Vec<String> = stmt
        .query_map([&ack], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        owners,
        [w.seats[1].clone()],
        "T={threshold}: receipts only for the explicit recipient"
    );
    drop(stmt);
    drop(db);
    w.text(1, false, &["inbox"]);
    assert_eq!(
        wake(&w),
        Wake {
            delivery_row: false,
            invitation: false,
            receipt: false
        }
    );

    // Invitation: confirming with a deadline, wake at every T.
    let (thread, invitation) = invite(&w, "invitation configuration");
    assert_eq!(
        wake(&w),
        Wake {
            delivery_row: false,
            invitation: true,
            receipt: false
        },
        "T={threshold}"
    );
    let (confirmation, deadline): (String, Option<i64>) = w
        .db()
        .query_row(
            "SELECT confirmation,deadline_at FROM invitations WHERE id=?1",
            [&invitation],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(confirmation, "require_ack");
    assert!(deadline.is_some(), "invitation carries a deadline");
    w.ok(Some(1), false, &["accept", &thread]);
    assert_eq!(
        wake(&w),
        Wake {
            delivery_row: false,
            invitation: false,
            receipt: false
        }
    );

    // 1:1 first send auto-joins both seats; the opener is a p1 delivery row.
    let before = invitations_total(&w);
    let sent = w.ok(
        Some(0),
        false,
        &[
            "send",
            "--seat",
            &w.seats[1],
            "--body",
            "pair opener",
            "-p",
            "1",
        ],
    );
    assert_eq!(sent["selection"]["invitation"], Value::Null, "{sent}");
    assert_eq!(sent["selection"]["outcome"], "already_joined", "{sent}");
    let pair = sent["thread"].as_str().unwrap().to_string();
    let opener = sent["data"].as_str().unwrap().to_string();
    assert_eq!(
        count(
            &w,
            "SELECT count(*) FROM memberships WHERE thread_id=?1 AND state='joined'",
            &pair
        ),
        2,
        "both pair seats joined"
    );
    assert_eq!(
        count(
            &w,
            "SELECT count(*) FROM invitations WHERE thread_id=?1",
            &pair
        ),
        0
    );
    assert_eq!(invitations_total(&w), before, "no invitation row created");
    eventually("pair opener published", || published_row(&w, &opener));
    assert_eq!(
        wake(&w),
        Wake {
            delivery_row: admits(threshold, 1),
            invitation: false,
            receipt: false
        },
        "T={threshold}: pair opener"
    );
    assert_eq!(receipts_for(&w, &opener), 0);
    w.text(1, false, &["inbox"]);
    assert_eq!(
        wake(&w),
        Wake {
            delivery_row: false,
            invitation: false,
            receipt: false
        }
    );

    if threshold == 0 {
        mixed_order(&w);
    }
}

/// p5, p1, p0 (no ACK), invitation, p0 ACK with a 60 s deadline arrive in that order; the inbox
/// lists them by (priority, deadline with none last, arrival).
fn mixed_order(w: &World) {
    let p5 = w.send("mixed p5", &["-p", "5"]);
    let p1 = w.send("mixed p1", &["-p", "1"]);
    let p0 = w.send("mixed p0", &["-p", "0"]);
    let (_thread, invitation) = invite(w, "mixed invitation");
    let ack = send_ack(w);
    for id in [&p5, &p1, &p0] {
        eventually("mixed rows published", || published_row(w, id));
    }
    eventually("mixed ack receipt", || wake(w).receipt);
    // The ACK deadline (60 s) precedes the invitation's default deadline, both p0; the
    // deadline-less p0 follows them, then p1 and p5.
    let expected = [ack, invitation, p0, p1, p5];
    let got: Vec<String> = inbox_items(w).iter().map(item_id).collect();
    assert_eq!(got, expected, "mixed inbox order");
}

#[test]
fn priority_config_smoke_threshold_minus_one() {
    configuration(-1);
}

#[test]
fn priority_config_smoke_threshold_zero() {
    configuration(0);
}

#[test]
fn priority_config_smoke_threshold_three() {
    configuration(3);
}
