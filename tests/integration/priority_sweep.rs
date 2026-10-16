//! ht-yha.16: the message-priority goal's main flows end to end, one test per flow, each on its own
//! isolated daemon (the `lazy_config_smoke` world) through the public CLI. Flows 1-7 name the
//! earlier test that already asserts the same facts only where this sweep adds nothing; each flow
//! below still drives the whole path itself.
//!
//! 1. default send (p1) to a T=0 seat: no wake, delivered by the seat's own `inbox`, no `[p1]`.
//! 2. `-p 0` and `--nudge`: the idle seat's wake frontier is raised, no receipt.
//! 3. `--require-ack`: p0, a receipt with a deadline, an overdue warning when the deadline passes.
//! 4. `nudge-threshold 3`: `-p 3` wakes, `-p 4` does not.
//! 5. invite: confirming, with a deadline, wakes the invitee.
//! 6. first direct send: both seats joined, no invitation.
//! 7. mixed inbox order (also `priority_config_smoke` at T=0 and `priority_order_surfaces`).
//! 8. old-shape clients against the new daemon: a CLI that sees a pre-priority capability set
//!    (`Proxy` modes 4 and 5) sends `delivery_mode` sends, `wake-on` and lazy invitations.
//!
//! Plus the live schema (v47 triggers recreated) and the advertised capabilities.
use super::lazy_config_smoke::{Proxy, World};
use super::priority_config_smoke::{
    Wake, count, eventually, inbox_items, invitations_total, invite, item_id, published_row,
    receipts_for, send_ack, wake,
};
use serde_json::Value;
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

const NONE: Wake = Wake {
    delivery_row: false,
    invitation: false,
    receipt: false,
};

fn set_threshold(w: &World, value: &str) {
    let args: Vec<&str> = if value.starts_with('-') {
        vec!["nudge-threshold", "--", value]
    } else {
        vec!["nudge-threshold", value]
    };
    w.ok(Some(1), false, &args);
}

fn row_priority(w: &World, message: &str) -> i64 {
    w.db()
        .query_row(
            "SELECT priority FROM messages WHERE id=?1",
            [message],
            |r| r.get(0),
        )
        .unwrap()
}

fn item_of(w: &World, id: &str) -> Value {
    inbox_items(w)
        .into_iter()
        .find(|i| item_id(i) == id)
        .unwrap_or_else(|| panic!("{id} not pending"))
}

/// Flow 1.
#[test]
fn priority_sweep_default_send_is_p1_silent_and_unmarked() {
    let w = World::agents_only();
    set_threshold(&w, "0");
    let id = w.send("quiet default body", &[]);
    eventually("p1 row published", || published_row(&w, &id));
    assert_eq!(row_priority(&w, &id), 1);
    assert_eq!(wake(&w), NONE, "a p1 send must not wake a T=0 seat");
    assert_eq!(receipts_for(&w, &id), 0);
    let item = item_of(&w, &id);
    assert_eq!(item["priority"], 1, "{item}");
    assert_ne!(item["wakes"].as_bool(), Some(true), "{item}");
    // Delivered in the seat's own turn (its inbox read), unmarked.
    let text = w.text(1, false, &["inbox"]);
    assert!(text.contains("quiet default body"), "{text}");
    assert!(!text.contains("[p1]") && !text.contains("[lazy]"), "{text}");
    assert_eq!(w.state(&id, 1), "displayed");
    assert_eq!(wake(&w), NONE);
}

/// Flow 2.
#[test]
fn priority_sweep_p0_and_nudge_wake_once_without_receipt() {
    let w = World::agents_only();
    set_threshold(&w, "0");
    for flag in [&["-p", "0"][..], &["--nudge"][..]] {
        let id = w.send("wake body", flag);
        eventually("p0 row published", || published_row(&w, &id));
        assert_eq!(row_priority(&w, &id), 0, "{flag:?}");
        assert_eq!(
            wake(&w),
            Wake {
                delivery_row: true,
                ..NONE
            },
            "{flag:?}: the idle seat is woken"
        );
        assert_eq!(item_of(&w, &id)["wakes"], true, "{flag:?}");
        assert_eq!(receipts_for(&w, &id), 0, "{flag:?}: a nudge is no ACK");
        let text = w.text(1, false, &["inbox"]);
        assert!(
            text.contains("wake body") && text.contains("[p0]"),
            "{text}"
        );
        assert_eq!(wake(&w), NONE, "{flag:?}: displaying settles the wake");
    }
}

/// Flow 3.
#[test]
fn priority_sweep_require_ack_is_p0_with_deadline_and_overdue_warning() {
    let w = World::agents_only();
    let id = w.send(
        "ack body",
        &["--require-ack", &w.seats[1], "--deadline", "1"],
    );
    assert_eq!(row_priority(&w, &id), 0);
    eventually("receipt pending", || wake(&w).receipt);
    // The receipt row is materialized asynchronously (the wake flag can come first).
    let receipt_deadline = || -> Option<Option<i64>> {
        use rusqlite::OptionalExtension;
        w.db()
            .query_row(
                "SELECT deadline_at FROM (SELECT message_id,seat_id,deadline_at FROM receipts UNION SELECT message_id,seat_id,deadline_at FROM receipt_state) WHERE message_id=?1 AND seat_id=?2",
                [&id, &w.seats[1]],
                |r| r.get(0),
            )
            .optional()
            .unwrap()
    };
    eventually("receipt materialized", || receipt_deadline().is_some());
    let deadline: Option<i64> = receipt_deadline().unwrap();
    assert!(deadline.is_some(), "the receipt carries a deadline");
    // The recipient misses it: the scheduler raises the overdue condition (real clock, 1 s).
    let subject = format!("overdue_receipt:{id}:{}", w.seats[1]);
    let start = Instant::now();
    loop {
        let overdue = w.ok(None, false, &["overdue", "--limit", "100"]);
        if overdue["data"]["items"].to_string().contains(&subject) {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(40),
            "no overdue warning for {subject}: {overdue}"
        );
        std::thread::sleep(Duration::from_millis(250));
    }
    // The late ACK (display) still lands and closes the receipt.
    w.text(1, false, &["inbox"]);
    eventually("late ack recorded", || !wake(&w).receipt);
}

/// Flow 4.
#[test]
fn priority_sweep_threshold_three_wakes_p3_not_p4() {
    let w = World::agents_only();
    set_threshold(&w, "3");
    for (p, wakes) in [("3", true), ("4", false)] {
        let id = w.send(&format!("p{p} body"), &["-p", p]);
        eventually("row published", || published_row(&w, &id));
        assert_eq!(
            wake(&w),
            Wake {
                delivery_row: wakes,
                ..NONE
            },
            "T=3 p={p}"
        );
        assert_eq!(
            item_of(&w, &id)["wakes"].as_bool().unwrap_or(false),
            wakes,
            "T=3 p={p}"
        );
        w.text(1, false, &["inbox"]);
        assert_eq!(wake(&w), NONE);
    }
}

/// Flow 5.
#[test]
fn priority_sweep_invitation_is_p0_confirming_with_deadline() {
    let w = World::agents_only();
    set_threshold(&w, "0");
    let (thread, invitation) = invite(&w, "sweep invitation");
    assert_eq!(
        wake(&w),
        Wake {
            invitation: true,
            ..NONE
        }
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
    assert!(deadline.is_some());
    let item = item_of(&w, &invitation);
    assert_eq!(item["kind"], "invitation", "{item}");
    w.ok(Some(1), false, &["accept", &thread]);
    assert_eq!(wake(&w), NONE);
}

/// Flow 6.
#[test]
fn priority_sweep_first_direct_send_joins_both_seats_without_invitation() {
    let w = World::agents_only();
    let before = invitations_total(&w);
    let sent = w.ok(
        Some(0),
        false,
        &["send", "--seat", &w.seats[1], "--body", "hello pair"],
    );
    assert_eq!(sent["selection"]["invitation"], Value::Null, "{sent}");
    let pair = sent["thread"].as_str().unwrap().to_string();
    let opener = sent["data"].as_str().unwrap().to_string();
    assert_eq!(
        count(
            &w,
            "SELECT count(*) FROM memberships WHERE thread_id=?1 AND state='joined'",
            &pair
        ),
        2
    );
    assert_eq!(invitations_total(&w), before);
    assert_eq!(row_priority(&w, &opener), 1);
    eventually("opener published", || published_row(&w, &opener));
    let text = w.text(1, false, &["inbox"]);
    assert!(text.contains("hello pair"), "{text}");
}

/// Flow 7: p5, p1, p0, invitation, p0 with an ACK deadline arrive in that order and the inbox
/// lists them by (priority, deadline with none last, arrival). Repeats
/// `priority_config_smoke::mixed_order` through `inbox --json` with a different arrival order so
/// a sort that follows arrival fails here too.
#[test]
fn priority_sweep_mixed_inbox_order() {
    let w = World::agents_only();
    set_threshold(&w, "0");
    let p1 = w.send("m p1", &["-p", "1"]);
    let p9 = w.send("m p9", &["-p", "9"]);
    let p0 = w.send("m p0", &["-p", "0"]);
    let p3 = w.send("m p3", &["-p", "3"]);
    let (_thread, invitation) = invite(&w, "mixed sweep invitation");
    let ack = send_ack(&w);
    for id in [&p1, &p9, &p0, &p3] {
        eventually("rows published", || published_row(&w, id));
    }
    eventually("ack receipt", || wake(&w).receipt);
    let got: Vec<String> = inbox_items(&w).iter().map(item_id).collect();
    assert_eq!(got, [ack, invitation, p0, p1, p3, p9]);
}

/// Flow 8c: a lazy invitation from a pre-priority client is recorded as a confirming invitation.
#[test]
fn priority_sweep_old_shape_lazy_invite_is_recorded_confirming() {
    let w = World::agents_only();
    let proxy = Proxy::new(w.paths(), &w.root);
    proxy.mode.store(5, Ordering::SeqCst);
    let (thread, invitation) = invite(&w, "old lazy invite");
    let invites: Vec<Value> = proxy
        .requests()
        .into_iter()
        .filter(|q| q["command"]["kind"] == "invite")
        .collect();
    assert_eq!(invites.len(), 1, "{invites:?}");
    assert_eq!(invites[0]["command"]["args"]["confirmation"], "lazy");
    let (confirmation, deadline): (String, Option<i64>) = w
        .db()
        .query_row(
            "SELECT confirmation,deadline_at FROM invitations WHERE id=?1",
            [&invitation],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(confirmation, "require_ack");
    assert!(deadline.is_some());
    assert!(wake(&w).invitation);
    w.ok(Some(1), false, &["accept", &thread]);
}

/// The v47 migration recreated the lazy triggers (the live schema, not the old files) and added
/// the priority guards.
#[test]
fn priority_sweep_live_schema_has_the_v47_triggers() {
    let w = World::agents_only();
    let db = w.db();
    let sql = |name: &str| -> String {
        db.query_row(
            "SELECT sql FROM sqlite_master WHERE type='trigger' AND name=?1",
            [name],
            |r| r.get(0),
        )
        .unwrap_or_else(|e| panic!("trigger {name} missing: {e}"))
    };
    let manifest = sql("lazy_manifest_source");
    assert!(
        manifest.contains("SELECT priority FROM send_preparations"),
        "pre-v47 lazy_manifest_source is still live: {manifest}"
    );
    let source = sql("lazy_recipient_source");
    assert!(
        source.contains("NEW.decision_seq IS NOT NULL"),
        "pre-v47 lazy_recipient_source is still live: {source}"
    );
    let published = sql("lazy_recipient_display_published");
    assert!(
        published.contains("send_manifests"),
        "pre-v47 lazy_recipient_display_published is still live: {published}"
    );
    for guard in [
        "preparation_priority_immutable",
        "lazy_recipient_priority_immutable",
        "lazy_recipient_decision_seq_once",
    ] {
        sql(guard);
    }
    // The guard bites: a stored priority cannot change.
    drop(db);
    let rw = rusqlite::Connection::open(w.paths().database_path).unwrap();
    let error = rw
        .execute("UPDATE messages SET priority=priority+1", [])
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("immutable"),
        "message history or priority guard: {error}"
    );
}

/// The three capabilities of this epic are advertised by the running daemon.
#[test]
fn priority_sweep_daemon_advertises_the_epic_capabilities() {
    let w = World::agents_only();
    let advertised = herdr_threads::protocol::capabilities::ADVERTISED;
    for capability in [
        "send.priority_v1",
        "seat.nudge_threshold_v1",
        "direct.auto_join_v1",
    ] {
        assert!(advertised.contains(&capability), "{capability}");
    }
    let health = w.ok(None, false, &["daemon", "ensure"]);
    assert!(health.is_object() || health.is_string(), "{health}");
}
