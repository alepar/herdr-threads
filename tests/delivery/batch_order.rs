//! The batch sort: one page's items go out in `DeliveryOrderKey` order (priority, then
//! deadline with none last), arrival breaking every remaining tie; pages and separate drains keep arrival.
use super::*;
use crate::delivery::page::{DrainEvent, PageError, PageSource, Stop, drain_pages};
use crate::protocol::{
    ids::{InvitationId, MessageId, SeatId, ThreadId},
    pagination::{Consistency, Page, StopReason},
    priority::DeliveryPriority,
    time::UtcMillis,
};
use std::collections::VecDeque;

fn msg(id: &str, priority: u8, deadline: Option<i64>) -> InboxBatchV2Item {
    InboxBatchV2Item::Message {
        priority: Some(DeliveryPriority::new(priority).unwrap()),
        thread: ThreadId::new("t1"),
        topic_data: "topic".into(),
        message: MessageId::new(id),
        sequence: 1,
        sender: Some(SeatId::new("sender")),
        author_role: None,
        relays_user: false,
        user_intent: None,
        author_role_backfilled: false,
        body: "b".into(),
        body_start: 0,
        body_end: 1,
        body_len: 1,
        ack_candidate: None,
        deadline_at: deadline.map(UtcMillis),
    }
}

fn lazy_msg(id: &str, priority: Option<u8>) -> InboxBatchV2Item {
    InboxBatchV2Item::LazyMessage {
        priority: priority.map(|p| DeliveryPriority::new(p).unwrap()),
        thread: ThreadId::new("t1"),
        topic_data: "topic".into(),
        message: MessageId::new(id),
        sequence: 1,
        sender: None,
        author_role: None,
        relays_user: false,
        user_intent: None,
        author_role_backfilled: false,
        body: "b".into(),
        body_start: 0,
        body_end: 1,
        body_len: 1,
        wakes: false,
    }
}

fn plain(id: &str) -> InboxBatchV2Item {
    let mut item = msg(id, 1, None);
    if let InboxBatchV2Item::Message { priority, .. } = &mut item {
        *priority = None;
    }
    item
}

fn invite(id: &str) -> InboxBatchV2Item {
    InboxBatchV2Item::Invitation {
        thread: ThreadId::new("t1"),
        topic_data: "topic".into(),
        goal_data: Some("goal".into()),
        topic_revision: None,
        invitation: InvitationId::new(id),
        confirmation: None,
        deadline_at: None,
        required_service: None,
    }
}

fn page(items: Vec<InboxBatchV2Item>, next: Option<&str>) -> Page<InboxBatchV2Item> {
    Page {
        items,
        next_cursor: next.map(str::to_owned),
        next_argv: None,
        high_water_ordinal: 0,
        scope_revision: None,
        has_more: next.is_some(),
        stop_reason: StopReason::Complete,
        consistency: Consistency::BoundedLive,
    }
}

struct Pages(VecDeque<Page<InboxBatchV2Item>>);
impl PageSource for Pages {
    fn page(&mut self, _cursor: Option<String>) -> Result<Page<InboxBatchV2Item>, PageError> {
        Ok(self.0.pop_front().expect("page left"))
    }
}

fn drain(assembler: &mut Assembler, pages: Vec<Vec<InboxBatchV2Item>>) -> Vec<String> {
    let total = pages.len();
    let mut source = Pages(
        pages
            .into_iter()
            .enumerate()
            .map(|(n, items)| page(items, (n + 1 < total).then_some("next")))
            .collect(),
    );
    let mut ids = Vec::new();
    drain_pages::<()>(&mut source, assembler, 1, 8, |event| {
        match event {
            DrainEvent::Item(item) => ids.push(item.id.as_str().to_owned()),
            DrainEvent::Invitation(inv) => ids.push(format!("invitation:{}", inv.id.as_str())),
            _ => {}
        }
        Ok(Stop::Continue)
    })
    .unwrap();
    ids
}

#[test]
fn batch_sorts_by_priority_then_deadline_none_last_then_arrival() {
    let items = vec![
        msg("a-p1", 1, None),
        msg("b-p0-none", 0, None),
        msg("c-p0-d200", 0, Some(200)),
        msg("d-p5", 5, None),
        msg("e-p0-d100", 0, Some(100)),
        msg("f-p1-second", 1, None),
    ];
    assert_eq!(
        drain(&mut Assembler::new(None), vec![items]),
        [
            "e-p0-d100",
            "c-p0-d200",
            "b-p0-none",
            "a-p1",
            "f-p1-second",
            "d-p5"
        ]
    );
}

#[test]
fn equal_keys_keep_arrival_order() {
    let items = vec![msg("z", 1, None), msg("a", 1, None), msg("m", 1, None)];
    assert_eq!(
        drain(&mut Assembler::new(None), vec![items]),
        ["z", "a", "m"]
    );
}

#[test]
fn each_page_is_emitted_in_arrival_order_before_the_next() {
    let pages = vec![
        vec![msg("first-p1", 1, None), msg("second-p1", 1, None)],
        vec![msg("late-p0", 0, None)],
    ];
    assert_eq!(
        drain(&mut Assembler::new(None), pages),
        ["first-p1", "second-p1", "late-p0"]
    );
}

#[test]
fn separate_drains_stay_in_arrival_order() {
    let mut assembler = Assembler::new(None);
    let first = drain(&mut assembler, vec![vec![msg("one-p5", 5, None)]]);
    let second = drain(&mut assembler, vec![vec![msg("two-p0", 0, None)]]);
    assert_eq!([first, second].concat(), ["one-p5", "two-p0"]);
}

#[test]
fn absent_priority_follows_the_legacy_kind() {
    let items = vec![
        msg("p2", 2, None),
        lazy_msg("lazy", None),
        plain("plain"),
        msg("p0", 0, None),
    ];
    assert_eq!(
        drain(&mut Assembler::new(None), vec![items]),
        ["plain", "p0", "lazy", "p2"]
    );
}

#[test]
fn an_invitation_ties_with_a_message_by_arrival() {
    let forward = vec![msg("m", 0, None), invite("i1")];
    assert_eq!(
        drain(&mut Assembler::new(None), vec![forward]),
        ["m", "invitation:i1"]
    );
    let reverse = vec![invite("i2"), msg("m2", 0, None)];
    assert_eq!(
        drain(&mut Assembler::new(None), vec![reverse]),
        ["invitation:i2", "m2"]
    );
}

#[test]
fn order_deliveries_uses_the_same_key() {
    use crate::protocol::{
        delivery::PresentationItemV2 as Item,
        watch::{WatchInvitation, WatchMessage},
    };
    let content = |priority: Option<u8>| WatchMessage {
        thread: ThreadId::new("t1"),
        thread_name: None,
        sender: None,
        sender_name: None,
        author_role: None,
        relays_user: false,
        user_intent: None,
        body: "b".into(),
        body_len: 1,
        truncated: false,
        ack_required: false,
        wakes: false,
        priority: priority.map(|p| DeliveryPriority::new(p).unwrap()),
        deadline_at: None,
    };
    let message = |id: &str, lazy: bool, priority: Option<u8>| Item::Message {
        id: MessageId::new(id),
        lazy,
        content: content(priority),
    };
    let mut items = vec![
        message("lazy", true, None),
        Item::Notice {
            warning: MessageId::new("w"),
            thread: ThreadId::new("t1"),
            body: "n".into(),
        },
        message("plain", false, None),
        Item::Invitation {
            id: InvitationId::new("inv"),
            content: WatchInvitation {
                command_context_epoch: None,
                topic_revision: None,
                thread: ThreadId::new("t1"),
                thread_name: None,
                body: "join".into(),
                truncated: false,
                wakes: true,
                deadline_at: None,
            },
        },
        message("p2", false, Some(2)),
    ];
    crate::delivery::presentation::order_deliveries(&mut items);
    let names: Vec<String> = items
        .iter()
        .map(|item| match item {
            Item::Message { id, .. } => id.as_str().to_owned(),
            Item::Invitation { id, .. } => format!("invitation:{}", id.as_str()),
            Item::Notice { .. } => "notice".to_owned(),
            _ => "other".to_owned(),
        })
        .collect();
    // plain (p0) and the invitation (p0) tie by arrival; the notice keeps slot 1.
    assert_eq!(names, ["plain", "notice", "invitation:inv", "lazy", "p2"]);
}
