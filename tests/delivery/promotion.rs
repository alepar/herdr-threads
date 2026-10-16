//! Push-channel promotion of `wake_on: all` lazy rows (delivery-policy DP8):
//! the wire omits `wakes` when false, and the drain emits a waking lazy row as
//! a `message` item with no ACK to run.
use super::*;
use crate::delivery::assemble::{Assembler, DeliveryKind};
use crate::protocol::{
    ids::{MessageId, SeatId, ThreadId},
    pagination::{Consistency, Page, StopReason},
    results::InboxBatchV2Item,
    watch::{WatchItem, truncation_marker_for},
};

fn lazy(id: &str, body: &str, wakes: bool) -> InboxBatchV2Item {
    InboxBatchV2Item::LazyMessage {
        priority: None,
        thread: ThreadId::new("t1"),
        topic_data: "topic".into(),
        message: MessageId::new(id),
        sequence: 1,
        sender: Some(SeatId::new("sender")),
        author_role: None,
        relays_user: false,
        user_intent: None,
        author_role_backfilled: false,
        body: body.into(),
        body_start: 0,
        body_end: body.len() as u64,
        body_len: body.len() as u64,
        wakes,
    }
}

struct One(Option<Page<InboxBatchV2Item>>);
impl PageSource for One {
    fn page(&mut self, _cursor: Option<String>) -> Result<Page<InboxBatchV2Item>, PageError> {
        Ok(self.0.take().expect("one page"))
    }
}

fn drain(items: Vec<InboxBatchV2Item>) -> Vec<DeliveryItem> {
    let mut source = One(Some(Page {
        items,
        next_cursor: None,
        next_argv: None,
        high_water_ordinal: 0,
        scope_revision: None,
        has_more: false,
        stop_reason: StopReason::Complete,
        consistency: Consistency::BoundedLive,
    }));
    let mut out = Vec::new();
    drain_pages::<()>(&mut source, &mut Assembler::new(None), 1, 4, |event| {
        if let DrainEvent::Item(item) = event {
            out.push(item);
        }
        Ok(Stop::Continue)
    })
    .unwrap();
    out
}

#[test]
fn wire_omits_wakes_when_false_and_carries_it_when_true() {
    let quiet = serde_json::to_value(lazy("m", "b", false)).unwrap();
    assert!(quiet.as_object().unwrap().get("wakes").is_none(), "{quiet}");
    assert_eq!(quiet["kind"], "lazy_message");
    let waking = serde_json::to_value(lazy("m", "b", true)).unwrap();
    assert_eq!(waking["wakes"], serde_json::json!(true));
    // An older daemon's line (no `wakes` key) reads as not waking.
    let back: InboxBatchV2Item = serde_json::from_value(quiet).unwrap();
    assert_eq!(back, lazy("m", "b", false));
    let back: InboxBatchV2Item = serde_json::from_value(waking).unwrap();
    assert_eq!(back, lazy("m", "b", true));
}

#[test]
fn non_waking_lazy_row_keeps_its_lazy_line() {
    let items = drain(vec![lazy("quiet", "hello", false)]);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].kind, DeliveryKind::Lazy);
    assert!(!items[0].message.ack_required);
}

#[test]
fn waking_lazy_row_is_a_message_without_ack() {
    let items = drain(vec![lazy("loud", "hello", true)]);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].kind, DeliveryKind::Message);
    assert_eq!(items[0].id, MessageId::new("loud"));
    assert!(!items[0].message.ack_required);
    assert_eq!(items[0].message.body, "hello");
    // The cli maps the kind to the frozen watch item.
    assert!(matches!(
        match items[0].kind {
            DeliveryKind::Lazy => WatchItem::Lazy(items[0].message.clone()),
            DeliveryKind::Message => WatchItem::Message(items[0].message.clone()),
        },
        WatchItem::Message(_)
    ));
}

#[test]
fn promoted_row_truncation_marker_names_body_only() {
    let body = "x".repeat(BODY_LIMIT_BYTES + 10);
    let items = drain(vec![lazy("long", &body, true)]);
    assert!(items[0].message.truncated);
    assert!(!items[0].message.ack_required);
    assert!(items[0].message.body.ends_with(&truncation_marker_for(
        &[],
        &MessageId::new("long"),
        true
    )));
}

/// Main review I1: a promoted lazy row and an ordinary message another binding already settled
/// both reach the mod as `message` lines with `ack_required: false`. The promoted one carries
/// `wakes: true` on the watch line, so the mod still wakes an idle session for it; the settled
/// ordinary line omits the key (an older watch child's line reads the same).
#[test]
fn watch_line_marks_a_promoted_row_waking_and_omits_it_otherwise() {
    let line = |items: Vec<DeliveryItem>| {
        let item = &items[0];
        serde_json::to_value(crate::protocol::watch::WatchLine::new(
            item.id.as_str(),
            match item.kind {
                DeliveryKind::Lazy => WatchItem::Lazy(item.message.clone()),
                DeliveryKind::Message => WatchItem::Message(item.message.clone()),
            },
        ))
        .unwrap()
    };
    let promoted = line(drain(vec![lazy("loud", "hello", true)]));
    assert_eq!(promoted["kind"], "message", "{promoted}");
    assert_eq!(promoted["ack_required"], false, "{promoted}");
    assert_eq!(promoted["wakes"], true, "{promoted}");
    let quiet = line(drain(vec![lazy("quiet", "hello", false)]));
    assert!(quiet.get("wakes").is_none(), "{quiet}");
    let settled = line(drain(vec![InboxBatchV2Item::Message {
        priority: None,
        thread: ThreadId::new("t1"),
        topic_data: "topic".into(),
        message: MessageId::new("settled"),
        sequence: 2,
        sender: Some(SeatId::new("sender")),
        author_role: None,
        relays_user: false,
        user_intent: None,
        author_role_backfilled: false,
        body: "hi".into(),
        body_start: 0,
        body_end: 2,
        body_len: 2,
        ack_candidate: None,
        deadline_at: None,
    }]));
    assert_eq!(settled["kind"], "message", "{settled}");
    assert_eq!(settled["ack_required"], false, "{settled}");
    assert!(settled.get("wakes").is_none(), "{settled}");
}
