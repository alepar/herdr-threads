//! The batch sort: one drain's combined items go out in `DeliveryOrderKey` order (priority, then
//! deadline with none last), arrival breaking every remaining tie; separate drains keep arrival.
use super::*;
use crate::delivery::page::{DrainEvent, PageError, PageSource, Stop, drain_pages};
use crate::protocol::{
    ids::{MessageId, SeatId, ThreadId},
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
        if let DrainEvent::Item(item) = event {
            ids.push(item.id.as_str().to_owned());
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
fn a_later_page_p0_precedes_an_earlier_pages_p1() {
    let pages = vec![
        vec![msg("first-p1", 1, None), msg("second-p1", 1, None)],
        vec![msg("late-p0", 0, None)],
    ];
    assert_eq!(
        drain(&mut Assembler::new(None), pages),
        ["late-p0", "first-p1", "second-p1"]
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
fn absent_priority_sorts_as_default() {
    let mut none = msg("none", 1, None);
    if let InboxBatchV2Item::Message { priority, .. } = &mut none {
        *priority = None;
    }
    let items = vec![msg("p2", 2, None), none, msg("p0", 0, None)];
    assert_eq!(
        drain(&mut Assembler::new(None), vec![items]),
        ["p0", "none", "p2"]
    );
}
