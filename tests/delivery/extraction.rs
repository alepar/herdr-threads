//! The assembler and the drain loop over scripted pages: the behaviour that
//! moved out of the `watch` CLI, asserted without it.
use super::*;
use crate::delivery::assemble::{Assembled, Assembler, DeliveryKind, label};
use crate::protocol::{
    ids::{InvitationId, MessageId, SeatId, ThreadId},
    pagination::{Consistency, Page, StopReason},
    results::InboxBatchV2Item,
    watch::truncation_marker_for,
};
use std::collections::VecDeque;

fn page_of(items: Vec<InboxBatchV2Item>, next: Option<&str>) -> Page<InboxBatchV2Item> {
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

/// Serves its pages in order and records the cursor of every request.
struct Scripted {
    pages: VecDeque<Result<Page<InboxBatchV2Item>, PageError>>,
    cursors: Vec<Option<String>>,
}

impl Scripted {
    /// Page `n` links to cursor `p<n+1>` except the last.
    fn of(pages: Vec<Vec<InboxBatchV2Item>>) -> Self {
        let total = pages.len();
        Self {
            pages: pages
                .into_iter()
                .enumerate()
                .map(|(n, items)| {
                    let next = format!("p{}", n + 1);
                    Ok(page_of(items, (n + 1 < total).then_some(next.as_str())))
                })
                .collect(),
            cursors: Vec::new(),
        }
    }
}

impl PageSource for Scripted {
    fn page(&mut self, cursor: Option<String>) -> Result<Page<InboxBatchV2Item>, PageError> {
        self.cursors.push(cursor);
        self.pages.pop_front().expect("no page left")
    }
}

fn message(id: &str, body: &str, start: u64, end: u64, len: u64, ack: bool) -> InboxBatchV2Item {
    InboxBatchV2Item::Message {
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
        body_start: start,
        body_end: end,
        body_len: len,
        ack_candidate: ack.then(|| MessageId::new(id)),
        deadline_at: None,
    }
}

fn whole(id: &str, body: &str) -> InboxBatchV2Item {
    message(id, body, 0, body.len() as u64, body.len() as u64, true)
}

fn lazy(id: &str, body: &str) -> InboxBatchV2Item {
    InboxBatchV2Item::LazyMessage {
        priority: None,
        thread: ThreadId::new("t1"),
        topic_data: "topic".into(),
        message: MessageId::new(id),
        sequence: 1,
        sender: None,
        author_role: None,
        relays_user: false,
        user_intent: None,
        author_role_backfilled: false,
        body: body.into(),
        body_start: 0,
        body_end: body.len() as u64,
        body_len: body.len() as u64,
        wakes: false,
    }
}

fn invitation() -> InboxBatchV2Item {
    InboxBatchV2Item::Invitation {
        thread: ThreadId::new("t1"),
        topic_data: "topic".into(),
        goal_data: Some("goal".into()),
        topic_revision: None,
        invitation: InvitationId::new("invitation-1"),
        confirmation: None,
        deadline_at: None,
        required_service: None,
    }
}

fn warning(informational: bool) -> InboxBatchV2Item {
    InboxBatchV2Item::Warning {
        thread: ThreadId::new("t1"),
        topic_data: "topic".into(),
        warning: MessageId::new("w1"),
        sequence: 2,
        informational,
    }
}

type Drained = (Result<(), DrainPagesError<&'static str>>, Vec<DrainEvent>);

fn drain(source: &mut Scripted, assembler: &mut Assembler, version: u64) -> Drained {
    let mut events = Vec::new();
    let result = drain_pages(source, assembler, version, MAX_DRAIN_PAGES, |event| {
        events.push(event);
        Ok(Stop::Continue)
    });
    (result, events)
}

fn item_of(event: &DrainEvent) -> &DeliveryItem {
    match event {
        DrainEvent::Item(item) => item,
        other => panic!("not an item: {other:?}"),
    }
}

fn ids(events: &[DrainEvent]) -> Vec<String> {
    events
        .iter()
        .map(|event| match event {
            DrainEvent::Item(item) => item.id.as_str().to_owned(),
            DrainEvent::Invitation(item) => format!("invitation:{}", item.id.as_str()),
            DrainEvent::Attention { version } => format!("attention:{version}"),
            DrainEvent::AttentionCleared { version } => format!("cleared:{version}"),
            DrainEvent::Exhausted => "exhausted".to_owned(),
        })
        .collect()
}

#[test]
fn page_constants_are_the_protocol_values() {
    assert_eq!(BODY_LIMIT_BYTES, 8 * 1024);
    assert_eq!(PAGE_MAX_ITEMS, 32);
    assert_eq!(PAGE_MAX_BYTES, 64 * 1024);
    assert_eq!(
        BODY_LIMIT_BYTES,
        crate::protocol::watch::WATCH_BODY_LIMIT_BYTES
    );
    assert_eq!(PAGE_MAX_ITEMS, crate::protocol::watch::WATCH_PAGE_MAX_ITEMS);
    assert_eq!(PAGE_MAX_BYTES, crate::protocol::watch::WATCH_PAGE_MAX_BYTES);
}

#[test]
fn pages_are_followed_by_cursor_and_items_keep_page_order() {
    let batch = |from: usize, count: usize| -> Vec<InboxBatchV2Item> {
        (from..from + count)
            .map(|n| whole(&format!("m{n:03}"), &format!("body {n}")))
            .collect()
    };
    let mut source = Scripted::of(vec![batch(0, PAGE_MAX_ITEMS), batch(32, 32), batch(64, 5)]);
    let mut assembler = Assembler::default();
    let (result, events) = drain(&mut source, &mut assembler, 4);
    assert!(result.is_ok());
    assert_eq!(events.len(), 70);
    assert_eq!(events[69], DrainEvent::Exhausted);
    for (n, event) in events[..69].iter().enumerate() {
        let item = item_of(event);
        assert_eq!(item.id.as_str(), format!("m{n:03}"));
        assert_eq!(item.kind, DeliveryKind::Message);
        assert_eq!(item.message.body, format!("body {n}"));
        assert!(item.message.ack_required);
    }
    assert_eq!(
        source.cursors,
        [None, Some("p1".to_owned()), Some("p2".to_owned())]
    );
}

#[test]
fn chunks_join_into_one_item() {
    let mut source = Scripted::of(vec![
        vec![message("c1", "ab", 0, 2, 6, true)],
        vec![message("c1", "cd", 2, 4, 6, true)],
        vec![message("c1", "ef", 4, 6, 6, true)],
    ]);
    let (result, events) = drain(&mut source, &mut Assembler::default(), 1);
    assert!(result.is_ok());
    assert_eq!(ids(&events), ["c1", "exhausted"]);
    let message = &item_of(&events[0]).message;
    assert_eq!(message.body, "abcdef");
    assert_eq!(message.body_len, 6);
    assert!(!message.truncated);
}

#[test]
fn a_long_chunked_body_is_produced_once_its_first_8_kib_are_assembled() {
    let mut source = Scripted::of(vec![
        vec![message("c2", &"x".repeat(5000), 0, 5000, 10_000, true)],
        vec![message("c2", &"y".repeat(4000), 5000, 9000, 10_000, true)],
        vec![message("c2", &"z".repeat(1000), 9000, 10_000, 10_000, true)],
    ]);
    let mut assembler = Assembler::default();
    let (result, events) = drain(&mut source, &mut assembler, 1);
    assert!(result.is_ok());
    // The last chunk finds the id already produced.
    assert_eq!(ids(&events), ["c2", "exhausted"]);
    let message = &item_of(&events[0]).message;
    assert!(message.truncated);
    assert_eq!(message.body_len, 10_000);
    assert_eq!(
        message.body,
        format!(
            "{}{}{}",
            "x".repeat(5000),
            "y".repeat(3192),
            truncation_marker_for(&[], &MessageId::new("c2"), false)
        )
    );
}

#[test]
fn truncation_cuts_at_8_kib_on_a_char_boundary_and_records_the_hint() {
    let dir = std::env::temp_dir().join(format!(
        "htd-trunc-{:08x}",
        uuid::Uuid::new_v4().as_fields().0
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let hint = dir.join("hint.ids");
    let exact = "a".repeat(8192);
    let over = "b".repeat(8193);
    // 8191 ASCII bytes then a 2-byte char straddling the 8192 cut.
    let straddle = format!("{}é{}", "c".repeat(8191), "d");
    let mut source = Scripted::of(vec![vec![
        whole("exact", &exact),
        whole("over", &over),
        whole("straddle", &straddle),
        lazy("lazy-over", &over),
    ]]);
    let mut assembler = Assembler::new(Some(hint.clone())).with_command_prefix(vec!["ht".into()]);
    let (result, events) = drain(&mut source, &mut assembler, 1);
    assert!(result.is_ok());
    let marker =
        |id: &str, lazy| truncation_marker_for(&["ht".to_owned()], &MessageId::new(id), lazy);
    let exact_item = item_of(&events[0]);
    assert!(!exact_item.message.truncated);
    assert_eq!(exact_item.message.body, exact);
    assert_eq!(
        item_of(&events[1]).message.body,
        format!("{}{}", "b".repeat(8192), marker("over", false))
    );
    assert_eq!(
        item_of(&events[2]).message.body,
        format!("{}{}", "c".repeat(8191), marker("straddle", false))
    );
    let lazy_item = item_of(&events[3]);
    assert_eq!(lazy_item.kind, DeliveryKind::Lazy);
    assert!(lazy_item.message.truncated);
    assert_eq!(
        lazy_item.message.body,
        format!("{}{}", "b".repeat(8192), marker("lazy-over", true))
    );
    assert_ne!(marker("over", false), marker("over", true));
    assert_eq!(
        std::fs::read_to_string(&hint)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        ["over", "straddle", "lazy-over"],
        "only truncated ids are hinted"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_produced_id_is_skipped_by_the_same_assembler() {
    let mut assembler = Assembler::default();
    let (_, first) = drain(
        &mut Scripted::of(vec![vec![whole("m1", "hi")]]),
        &mut assembler,
        1,
    );
    assert_eq!(ids(&first), ["m1", "exhausted"]);
    let (_, again) = drain(
        &mut Scripted::of(vec![vec![whole("m1", "hi")]]),
        &mut assembler,
        1,
    );
    assert_eq!(ids(&again), ["exhausted"]);
    assert_eq!(assembler.push(whole("m1", "hi")), Ok(Assembled::Duplicate),);
    // A fresh assembler produces it again.
    let (_, fresh) = drain(
        &mut Scripted::of(vec![vec![whole("m1", "hi")]]),
        &mut Assembler::default(),
        1,
    );
    assert_eq!(ids(&fresh), ["m1", "exhausted"]);
}

#[test]
fn push_classifies_every_item_kind() {
    let mut assembler = Assembler::default();
    assert!(
        matches!(assembler.push(invitation()), Ok(Assembled::Invitation(item)) if item.content.wakes)
    );
    assert_eq!(assembler.push(warning(false)), Ok(Assembled::Attention));
    assert_eq!(
        assembler.push(warning(true)),
        Ok(Assembled::InformationalNotice)
    );
    assert_eq!(
        assembler.push(message("c", "ab", 0, 2, 6, true)),
        Ok(Assembled::Pending)
    );
    assert!(matches!(
        assembler.push(message("c", "cdef", 2, 6, 6, true)),
        Ok(Assembled::Item(_))
    ));
}

#[test]
fn informational_warnings_raise_no_attention() {
    let mut assembler = Assembler::default();
    let (_, events) = drain(
        &mut Scripted::of(vec![vec![warning(true), whole("m1", "hi")]]),
        &mut assembler,
        3,
    );
    assert_eq!(ids(&events), ["m1", "exhausted"]);
    assert!(!assembler.attention_shown);
}

#[test]
fn invitations_and_warnings_collapse_into_one_attention_per_version() {
    let items = || {
        vec![
            invitation(),
            warning(false),
            invitation(),
            whole("m1", "hi"),
        ]
    };
    let mut assembler = Assembler::default();
    let (_, first) = drain(&mut Scripted::of(vec![items()]), &mut assembler, 5);
    assert_eq!(
        ids(&first),
        ["invitation:invitation-1", "m1", "attention:5", "exhausted"]
    );
    let (_, same) = drain(&mut Scripted::of(vec![items()]), &mut assembler, 5);
    assert_eq!(ids(&same), ["exhausted"], "same version: nothing new");
    let (_, newer) = drain(&mut Scripted::of(vec![items()]), &mut assembler, 6);
    assert_eq!(ids(&newer), ["attention:6", "exhausted"]);
    // Messages alone never raise one.
    let (_, plain) = drain(
        &mut Scripted::of(vec![vec![whole("m9", "x")]]),
        &mut Assembler::default(),
        7,
    );
    assert_eq!(ids(&plain), ["m9", "exhausted"]);
}

#[test]
fn a_drain_that_finds_nothing_pending_retracts_the_last_attention() {
    let mut assembler = Assembler::default();
    let (_, got) = drain(
        &mut Scripted::of(vec![vec![invitation()]]),
        &mut assembler,
        5,
    );
    assert_eq!(
        ids(&got),
        ["invitation:invitation-1", "attention:5", "exhausted"]
    );
    let (_, got) = drain(
        &mut Scripted::of(vec![vec![whole("m1", "hi")]]),
        &mut assembler,
        6,
    );
    assert_eq!(ids(&got), ["m1", "cleared:6", "exhausted"]);
    // Already retracted: nothing more.
    let (_, got) = drain(&mut Scripted::of(vec![vec![]]), &mut assembler, 6);
    assert_eq!(ids(&got), ["exhausted"]);
    // Attention coming back at an already seen version is raised again.
    let (_, got) = drain(
        &mut Scripted::of(vec![vec![invitation()]]),
        &mut assembler,
        5,
    );
    assert_eq!(ids(&got), ["attention:5", "exhausted"]);
}

#[test]
fn a_chunk_that_does_not_continue_the_text_is_a_contract_break() {
    let mut source = Scripted::of(vec![vec![
        message("c3", "ab", 0, 2, 6, true),
        message("c3", "ef", 4, 6, 6, true),
    ]]);
    let (result, events) = drain(&mut source, &mut Assembler::default(), 1);
    assert!(matches!(result, Err(DrainPagesError::Assemble(_))));
    assert!(events.is_empty());
}

#[test]
fn chunks_left_by_a_failed_drain_do_not_carry_over() {
    let mut assembler = Assembler::default();
    let mut source = Scripted::of(vec![vec![message("c4", "ab", 0, 2, 6, true)]]);
    source.pages.push_back(Err(PageError::Api));
    // The first page links on, the second fails: "ab" stays buffered.
    source.pages[0] = Ok(page_of(
        vec![message("c4", "ab", 0, 2, 6, true)],
        Some("p1"),
    ));
    let (result, _) = drain(&mut source, &mut assembler, 1);
    assert!(matches!(result, Err(DrainPagesError::Page(PageError::Api))));
    // A new drain starts with a clean buffer, so a first chunk at 0 is fine.
    let (result, events) = drain(
        &mut Scripted::of(vec![vec![whole("c4", "abcdef")]]),
        &mut assembler,
        1,
    );
    assert!(result.is_ok());
    assert_eq!(item_of(&events[0]).message.body, "abcdef");
}

#[test]
fn page_errors_and_endless_cursors_are_reported() {
    let mut source = Scripted::of(vec![]);
    source.pages.push_back(Err(PageError::Protocol));
    let (result, _) = drain(&mut source, &mut Assembler::default(), 1);
    assert!(matches!(
        result,
        Err(DrainPagesError::Page(PageError::Protocol))
    ));

    // A source that always links on is cut off at max_pages.
    struct Endless(usize);
    impl PageSource for Endless {
        fn page(&mut self, _: Option<String>) -> Result<Page<InboxBatchV2Item>, PageError> {
            self.0 += 1;
            Ok(page_of(vec![], Some("more")))
        }
    }
    let mut endless = Endless(0);
    let result = drain_pages::<()>(&mut endless, &mut Assembler::default(), 1, 7, |_| {
        Ok(Stop::Continue)
    });
    assert!(matches!(result, Err(DrainPagesError::TooManyPages)));
    assert_eq!(endless.0, 7);
}

#[test]
fn the_sink_can_halt_or_fail_the_drain() {
    let two = || Scripted::of(vec![vec![whole("m1", "a"), whole("m2", "b")]]);
    let mut seen = Vec::new();
    let result = drain_pages::<()>(&mut two(), &mut Assembler::default(), 1, 10, |event| {
        seen.push(event);
        Ok(Stop::Halt)
    });
    assert!(result.is_ok());
    assert_eq!(ids(&seen), ["m1"], "halt ends the drain at once");

    let result = drain_pages(&mut two(), &mut Assembler::default(), 1, 10, |_| {
        Err("gone")
    });
    assert!(matches!(result, Err(DrainPagesError::Sink("gone"))));
}

#[test]
fn a_sink_halt_never_fetches_the_next_page() {
    let mut source = Scripted::of(vec![
        vec![whole("m1", "a"), whole("m2", "b")],
        vec![whole("m3", "c")],
    ]);
    let mut seen = Vec::new();
    let result = drain_pages::<()>(&mut source, &mut Assembler::default(), 1, 10, |event| {
        seen.push(event);
        Ok(Stop::Halt)
    });
    assert!(result.is_ok());
    assert_eq!(ids(&seen), ["m1"]);
    assert_eq!(source.cursors, vec![None], "page 2 was never requested");
}

#[test]
fn pages_are_emitted_as_they_arrive() {
    use std::{cell::Cell, rc::Rc};
    struct Counting {
        inner: Scripted,
        served: Rc<Cell<usize>>,
    }
    impl PageSource for Counting {
        fn page(&mut self, cursor: Option<String>) -> Result<Page<InboxBatchV2Item>, PageError> {
            self.served.set(self.served.get() + 1);
            self.inner.page(cursor)
        }
    }
    let served = Rc::new(Cell::new(0));
    let mut source = Counting {
        inner: Scripted::of(vec![vec![whole("m1", "a")], vec![whole("m2", "b")]]),
        served: Rc::clone(&served),
    };
    let mut at = Vec::new();
    drain_pages::<()>(&mut source, &mut Assembler::default(), 1, 10, |event| {
        if let DrainEvent::Item(item) = &event {
            at.push((item.id.as_str().to_owned(), served.get()));
        }
        Ok(Stop::Continue)
    })
    .unwrap();
    assert_eq!(
        at,
        [("m1".to_owned(), 1), ("m2".to_owned(), 2)],
        "each item is emitted before the next page is served"
    );
}

#[test]
fn label_flattens_controls_trims_and_cuts_at_120_chars() {
    assert_eq!(
        label("release plan\nsecond\u{2028}line").as_deref(),
        Some("release plan second line")
    );
    assert_eq!(label("\n\t"), None);
    assert_eq!(label("  padded  ").as_deref(), Some("padded"));
    assert_eq!(label(&"x".repeat(120)).unwrap().chars().count(), 120);
    assert_eq!(
        label(&"x".repeat(121)).as_deref(),
        Some(format!("{}…", "x".repeat(120)).as_str())
    );
}

#[test]
fn lazy_and_service_items_carry_their_own_names() {
    let (_, events) = drain(
        &mut Scripted::of(vec![vec![
            lazy("l1", "lazy body"),
            message("m2", "plain", 0, 5, 5, false),
        ]]),
        &mut Assembler::default(),
        1,
    );
    let lazy_item = item_of(&events[0]);
    assert_eq!(lazy_item.kind, DeliveryKind::Lazy);
    assert!(!lazy_item.message.ack_required);
    assert_eq!(lazy_item.message.sender_name.as_deref(), Some("service"));
    assert_eq!(lazy_item.message.thread_name.as_deref(), Some("topic"));
    let plain = item_of(&events[1]);
    assert!(!plain.message.ack_required, "no ack_candidate");
    assert_eq!(plain.message.sender_name.as_deref(), Some("sender"));
}

#[test]
fn invitation_context_carries_goal_or_requires_inspection_before_acceptance() {
    for goal in [
        Some("Decide whether this review belongs to your remit"),
        None,
    ] {
        let mut encoded = serde_json::to_value(invitation()).unwrap();
        if let Some(goal) = goal {
            encoded["goal_data"] = serde_json::Value::String(goal.into());
        } else {
            encoded.as_object_mut().unwrap().remove("goal_data");
        }
        let item: InboxBatchV2Item = serde_json::from_value(encoded).unwrap();
        let Assembled::Invitation(item) = Assembler::default().push(item).unwrap() else {
            panic!("invitation content is required")
        };
        if let Some(goal) = goal {
            assert!(item.content.body.contains(goal), "{}", item.content.body);
        } else {
            let inspect = item
                .content
                .body
                .find("thread show")
                .expect("old or byte-constrained goal omission requires inspection");
            let accept = item.content.body.find("accept").unwrap();
            assert!(
                inspect < accept,
                "inspection precedes acceptance: {}",
                item.content.body
            );
        }
    }
}

#[test]
fn an_omitted_invitation_goal_is_incomplete_until_full_context_at_the_same_revision() {
    let mut assembler = Assembler::default();
    let mut encoded = serde_json::to_value(invitation()).unwrap();
    encoded.as_object_mut().unwrap().remove("goal_data");
    encoded["topic_revision"] = serde_json::json!(7);
    encoded["confirmation"] = serde_json::json!("lazy");
    let omitted = serde_json::from_value(encoded.clone()).unwrap();
    let Assembled::Invitation(item) = assembler.push(omitted).unwrap() else {
        panic!("the inspection continuation must be offered")
    };
    assert!(
        item.content.truncated,
        "an omitted goal is not whole decision context"
    );
    assert!(assembler.invitation_displays.is_empty());
    encoded["goal_data"] = serde_json::json!("review recipient remit");
    let full: InboxBatchV2Item = serde_json::from_value(encoded).unwrap();
    let Assembled::Invitation(item) = assembler.push(full.clone()).unwrap() else {
        panic!("the same revision must permit its newly available full goal")
    };
    assert!(!item.content.truncated);
    assert!(item.content.body.contains("goal: review recipient remit"));
    assert!(matches!(assembler.push(full), Ok(Assembled::Duplicate)));
}
