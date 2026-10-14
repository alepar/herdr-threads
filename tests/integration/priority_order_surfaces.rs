//! ht-yha.15: one delivery order across the presentation surfaces of a real daemon.
//!
//! One mixed pending set for seat B is published in an arrival order that differs from the
//! `DeliveryOrderKey` order: a p5 and a p1 message, a confirming invitation (p0, deadline), a p0
//! message without ACK and an explicit-ACK message (p0, shorter deadline). The expected order is
//! computed from the published ids and their keys, then asserted through the CLI `inbox` text and
//! `--json` (whole, and split by a continuation), the watch stream (the presentation page: delivery
//! rows plus the deadline class, invitations as their own descriptors) and `accept`'s thread-scoped
//! display (delivery rows only). The hook page and the hook presentation page are in
//! `tests/service/priority_order_pull.rs`; the mod's JS mirror runs the same mixed set from
//! `tests/fixtures/priority_order_mixed.json` (`integrations/claude/mod/tests/delivery.test.ts`).
//! The invitation is presented outside the message order on the watch and hook surfaces (ht-cbb):
//! the order assertions there cover the messages, and the full-order assertions are `#[ignore]`d
//! until that is fixed.
use super::*;
use herdr_threads::protocol::{
    priority::{DeliveryOrderKey, DeliveryPriority, SourceRank},
    time::UtcMillis,
};

/// The ACK message's and the invitation's deadlines, in seconds after the (frozen) publish time.
const ACK_DEADLINE_S: u64 = 120;
const INVITE_DEADLINE_S: u64 = 600;

/// What one published item is: the id every surface names it by, and its order key.
struct Published {
    label: &'static str,
    id: String,
    key: DeliveryOrderKey,
}

struct Mixed {
    /// Arrival order.
    items: Vec<Published>,
}
impl Mixed {
    /// Ids in `DeliveryOrderKey` order, computed from the keys (never from a hard-coded list).
    fn expected(&self) -> Vec<String> {
        let mut sorted: Vec<&Published> = self.items.iter().collect();
        sorted.sort_by_key(|item| item.key);
        sorted.into_iter().map(|item| item.id.clone()).collect()
    }
    fn id(&self, label: &str) -> &str {
        &self
            .items
            .iter()
            .find(|item| item.label == label)
            .unwrap()
            .id
    }
}

/// Publishes the mixed set from A to B, in the arrival order p5, p1, invitation, p0, ACK.
fn publish_mixed(rig: &Rig) -> Mixed {
    // The deadlines are explicit: with the clock frozen they are exactly `now + seconds`.
    rig.clock.freeze_utc();
    let now = rig.clock.utc_now().0;
    let a = Some((rig.a.as_str(), PANE_A));
    let key = |priority: u8, deadline: Option<i64>, seq: u64| DeliveryOrderKey {
        priority: DeliveryPriority::new(priority).unwrap(),
        deadline: deadline.map(UtcMillis),
        decision_seq: seq,
        rank: SourceRank::DeliveryRow,
        ordinal: 0,
    };
    let send = |body: &str, extra: &[&str]| {
        let mut args = vec!["send", rig.thread.as_str(), "--body", body];
        args.extend_from_slice(extra);
        rig.cli(a, &args).text(body)
    };
    let mut items = Vec::new();
    items.push(Published {
        label: "p5",
        id: send("p5", &["--priority", "5"]),
        key: key(5, None, 1),
    });
    items.push(Published {
        label: "p1",
        id: send("p1", &["--priority", "1"]),
        key: key(1, None, 2),
    });
    let invited_thread = rig
        .cli(a, &["thread", "create", "--topic", "invited"])
        .text("thread create");
    let invitation = rig
        .cli(
            a,
            &[
                "invite",
                &invited_thread,
                "--seat",
                &rig.b,
                "--deadline",
                &INVITE_DEADLINE_S.to_string(),
            ],
        )
        .data("invite");
    items.push(Published {
        label: "invitation",
        id: invitation
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| invitation.to_string().trim_matches('"').to_owned()),
        key: key(0, Some(now + INVITE_DEADLINE_S as i64 * 1000), 3),
    });
    items.push(Published {
        label: "p0",
        id: send("p0", &["--priority", "0"]),
        key: key(0, None, 4),
    });
    items.push(Published {
        label: "ack",
        id: send(
            "ack",
            &[
                "--require-ack",
                &rig.b,
                "--deadline",
                &ACK_DEADLINE_S.to_string(),
            ],
        ),
        key: key(0, Some(now + ACK_DEADLINE_S as i64 * 1000), 5),
    });
    Mixed { items }
}

const B: fn(&Rig) -> Option<(&str, &str)> = |rig| Some((rig.b.as_str(), PANE_B));

/// The ids of a JSON inbox page, in page order (a message by its id, an invitation by its own).
fn json_ids(page: &Value) -> Vec<String> {
    page["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| match item["kind"].as_str().unwrap() {
            "invitation" => item["invitation"].as_str().unwrap().to_owned(),
            "message" | "lazy_message" => item["message"].as_str().unwrap().to_owned(),
            other => panic!("unexpected inbox item {other}: {item}"),
        })
        .collect()
}

/// The ids of a text inbox or accept output, in print order (header lines start at column 0).
fn text_ids(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let mut words = line.split(' ');
            matches!(words.next(), Some("message" | "invitation"))
                .then(|| words.next().unwrap().to_owned())
        })
        .collect()
}

/// Every surface lists the set in the key order; the read-only JSON surfaces first, because the
/// text inbox completes what it displays.
#[test]
fn inbox_json_text_continuation_and_watch_follow_the_delivery_order_key() {
    let rig = Rig::new();
    let mixed = publish_mixed(&rig);
    let expected = mixed.expected();
    // Self-check: the arrival order is not the expected order, and the ACK message leads.
    let arrival: Vec<String> = mixed.items.iter().map(|item| item.id.clone()).collect();
    assert_ne!(arrival, expected);
    assert_eq!(
        expected,
        [
            mixed.id("ack"),
            mixed.id("invitation"),
            mixed.id("p0"),
            mixed.id("p1"),
            mixed.id("p5")
        ]
    );

    // CLI `inbox --json`: the whole set.
    let whole = rig.cli(B(&rig), &["inbox"]);
    let whole_page = whole.data("inbox --json");
    assert_eq!(json_ids(&whole_page), expected, "inbox --json");
    assert_eq!(whole_page["has_more"], false);

    // A continuation that splits the set: two items per page, concatenated, in the same order.
    let mut pages = 0;
    let mut joined = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut args = vec!["inbox", "--limit", "2"];
        if let Some(cursor) = cursor.as_deref() {
            args.extend(["--cursor", cursor]);
        }
        let page = rig.cli(B(&rig), &args).data("inbox --limit 2");
        joined.extend(json_ids(&page));
        pages += 1;
        assert!(pages <= 5, "the continuation does not end");
        match page["next_cursor"].as_str() {
            Some(next) => cursor = Some(next.to_owned()),
            None => break,
        }
    }
    assert!(pages >= 2, "the split must actually split ({pages} pages)");
    assert_eq!(joined, expected, "inbox --json continuation");

    // The watch stream: the presentation page (delivery rows plus the deadline class).
    let mut watch = rig.watch(SESSION_B);
    watch.wait_connected();
    watch.wait_attention_marker();
    let streamed: Vec<String> = watch
        .seen
        .iter()
        .filter(|line| matches!(line["kind"].as_str(), Some("message" | "lazy")))
        .map(|line| line["id"].as_str().unwrap().to_owned())
        .collect();
    let messages: Vec<String> = expected
        .iter()
        .filter(|id| id.as_str() != mixed.id("invitation"))
        .cloned()
        .collect();
    assert_eq!(streamed, messages, "watch stream messages");
    assert!(
        watch.seen.iter().any(|line| {
            line["kind"] == "invitation"
                && line["id"] == format!("invitation:{}", mixed.id("invitation"))
        }),
        "the invitation is streamed: {:#?}",
        watch.seen
    );
    drop(watch);

    // CLI `inbox` text (it completes what it displays, so it runs last).
    let text = rig.cli_text_b(&["inbox"]);
    assert_eq!(text.code, 0, "{}{}", text.stdout, text.stderr);
    assert_eq!(
        text_ids(&text.stdout),
        expected,
        "inbox text:\n{}",
        text.stdout
    );
}

/// The watch stream carries the invitation at its key position (p0 with its deadline). Today its
/// descriptor carries neither a priority nor a deadline, so the mod places it by arrival.
#[test]
#[ignore = "blocked by ht-cbb: the watch invitation line has no deadline_at"]
fn watch_stream_places_the_invitation_by_its_deadline() {
    let rig = Rig::new();
    let mixed = publish_mixed(&rig);
    let mut watch = rig.watch(SESSION_B);
    watch.wait_connected();
    watch.wait_attention_marker();
    let streamed: Vec<String> = watch
        .seen
        .iter()
        .filter(|line| {
            matches!(
                line["kind"].as_str(),
                Some("message" | "lazy" | "invitation")
            )
        })
        .map(|line| {
            let id = line["id"].as_str().unwrap();
            id.strip_prefix("invitation:").unwrap_or(id).to_owned()
        })
        .collect();
    assert_eq!(streamed, mixed.expected());
}

/// Accept's thread-scoped display shows the accepted thread's pending delivery rows of every
/// priority, in key order, however they arrived. A thread's opening mail before its recipient joins
/// exists only for a pre-upgrade pending pair invitation (a new pair auto-joins), so the pair is
/// created the pre-upgrade way.
#[test]
fn accept_display_lists_the_threads_delivery_rows_in_key_order() {
    let rig = Rig::new();
    let writer = rusqlite::Connection::open(&rig.paths.database_path).unwrap();
    writer.busy_timeout(Duration::from_secs(5)).unwrap();
    writer
        .pragma_update(
            None,
            "application_id",
            herdr_threads::store::control::LEGACY_PAIR_MARKER,
        )
        .unwrap();
    drop(writer);
    let a = Some((rig.a.as_str(), PANE_A));
    for (body, priority) in [("p5", "5"), ("p1", "1"), ("p0", "0")] {
        rig.cli(
            a,
            &[
                "send",
                "--seat",
                &rig.b,
                "--body",
                body,
                "--priority",
                priority,
            ],
        )
        .data(body);
    }
    let db = rig.db();
    let pair: String = db
        .query_row("SELECT thread_id FROM direct_threads LIMIT 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    let id = |body: &str| -> String {
        db.query_row(
            "SELECT id FROM messages WHERE thread_id=?1 AND body=?2",
            [pair.as_str(), body],
            |row| row.get(0),
        )
        .unwrap()
    };
    let accept = rig.cli_text_b(&["accept", &pair]);
    assert_eq!(accept.code, 0, "{}{}", accept.stdout, accept.stderr);
    assert_eq!(
        text_ids(&accept.stdout),
        [id("p0"), id("p1"), id("p5")],
        "accept display:\n{}",
        accept.stdout
    );
}

const FIXTURE: &str = include_str!("../fixtures/priority_order_mixed.json");
/// The mod's own test imports one module from inside its folder (the plugin sandbox reads nothing
/// else), so it carries a copy of the fixture.
const MOD_COPY: &str =
    include_str!("../../integrations/claude/mod/tests/fixtures/priority-order-mixed.ts");

/// The shared fixture: the watch-shaped mixed set in arrival order, sorted by the batch order the
/// delivery layer uses (`batch_order`: priority, deadline with none last, arrival), equals the
/// expected list the mod's sort and frame are held to.
#[test]
fn the_shared_fixture_batch_orders_to_its_expected_list() {
    let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
    let mut items: Vec<Value> = fixture["items"].as_array().unwrap().clone();
    let arrival: Vec<&str> = items.iter().map(|i| i["id"].as_str().unwrap()).collect();
    let expected: Vec<&str> = fixture["expected"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap())
        .collect();
    assert_ne!(arrival, expected, "the fixture's arrival order must differ");
    herdr_threads::delivery::assemble::batch_order(&mut items, |item| DeliveryOrderKey {
        priority: DeliveryPriority::new(item["priority"].as_u64().unwrap() as u8).unwrap(),
        deadline: item["deadline_at"].as_i64().map(UtcMillis),
        decision_seq: 0,
        rank: SourceRank::DeliveryRow,
        ordinal: 0,
    });
    let sorted: Vec<&str> = items.iter().map(|i| i["id"].as_str().unwrap()).collect();
    assert_eq!(sorted, expected);
}

#[test]
fn the_mod_test_copy_equals_the_shared_fixture() {
    let json = MOD_COPY.strip_prefix("export default ").unwrap();
    let copy: Value = serde_json::from_str(json).unwrap();
    let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
    assert_eq!(copy, fixture);
}
