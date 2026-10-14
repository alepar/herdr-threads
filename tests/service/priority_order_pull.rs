//! ht-yha.15: the hook page and the hook presentation page in the delivery order key, for the mixed
//! set of `tests/fixtures/priority_order_mixed.json` published through the real send/invite commands
//! (arrival: p5, p1, invitation, p0, ACK message). The daemon-level surfaces (CLI inbox, watch, accept)
//! are in `tests/integration/priority_order_surfaces.rs`.
use super::*;
use crate::protocol::{
    commands::{CreateThread, DeliveryMode, InvitationConfirmation, Invite, SendMessage},
    delivery::PresentationItemV2,
    priority::{DeliveryOrderKey, DeliveryPriority, SourceRank},
};

const ACK_DEADLINE_MS: i64 = 120_000;
const INVITE_DEADLINE_MS: i64 = 600_000;

struct Mixed {
    /// (label, id, key) in arrival order.
    items: Vec<(&'static str, String, DeliveryOrderKey)>,
}
impl Mixed {
    fn expected(&self) -> Vec<String> {
        let mut sorted: Vec<_> = self.items.iter().collect();
        sorted.sort_by_key(|(_, _, key)| *key);
        sorted.into_iter().map(|(_, id, _)| id.clone()).collect()
    }
    fn id(&self, label: &str) -> &str {
        &self.items.iter().find(|(l, _, _)| *l == label).unwrap().1
    }
}

fn publish_mixed(f: &Fixture, recipient: &CallerClaim) -> Mixed {
    let sender = check_in(f, claim("x", "px", Harness::Codex, "sx", 1), "ci-x");
    let now = now_utc(f).0;
    let key = |priority: u8, deadline: Option<i64>, seq: u64| DeliveryOrderKey {
        priority: DeliveryPriority::new(priority).unwrap(),
        deadline: deadline.map(crate::protocol::time::UtcMillis),
        decision_seq: seq,
        rank: SourceRank::DeliveryRow,
        ordinal: 0,
    };
    let send = |op: &str, priority: u8, acks: Vec<SeatId>, deadline: Option<u64>| -> String {
        let CommandResult::MessageSent(id) = f
            .service
            .handle(
                Command::SendMessage(SendMessage {
                    priority: Some(DeliveryPriority::new(priority).unwrap()),
                    delivery_mode: DeliveryMode::Ordinary,
                    user_intent: None,
                    thread: ThreadId::new("t"),
                    body: format!("body {op}"),
                    direct_recipient: None,
                    invited_recipients: acks,
                    deadline_millis: deadline,
                    operation: OperationId::new(op),
                    claim: sender.clone(),
                    relays_user: false,
                }),
                peer(),
                &budget(),
            )
            .unwrap()
        else {
            panic!("send {op}")
        };
        id.as_str().to_owned()
    };
    let mut items = Vec::new();
    items.push(("p5", send("p5", 5, vec![], None), key(5, None, 1)));
    items.push(("p1", send("p1", 1, vec![], None), key(1, None, 2)));
    let CommandResult::ThreadCreated(thread) = f
        .service
        .handle(
            Command::CreateThread(CreateThread {
                name: None,
                topic: "invited".into(),
                goal: "review".into(),
                operation: OperationId::new("create-invited"),
                claim: sender.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("thread")
    };
    let invitation = f
        .service
        .handle(
            Command::Invite(Invite {
                confirmation: Some(InvitationConfirmation::RequireAck),
                thread,
                seat: recipient.seat.clone(),
                deadline_millis: Some(INVITE_DEADLINE_MS as u64),
                operation: OperationId::new("invite"),
                claim: sender.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    let CommandResult::Invitation(invitation) = invitation else {
        panic!("invite: {invitation:?}")
    };
    items.push((
        "invitation",
        invitation.as_str().to_owned(),
        key(0, Some(now + INVITE_DEADLINE_MS), 3),
    ));
    items.push(("p0", send("p0", 0, vec![], None), key(0, None, 4)));
    items.push((
        "ack",
        send(
            "ack",
            0,
            vec![recipient.seat.clone()],
            Some(ACK_DEADLINE_MS as u64),
        ),
        key(0, Some(now + ACK_DEADLINE_MS), 5),
    ));
    Mixed { items }
}

fn text_ids(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("[herdr-threads] ")?;
            let mut words = rest.split(' ');
            matches!(words.next(), Some("message" | "invitation"))
                .then(|| words.next().unwrap().to_owned())
        })
        .collect()
}

/// The messages of `ids` (everything but the invitation), keeping their order.
fn without_invitation(mixed: &Mixed, ids: Vec<String>) -> Vec<String> {
    ids.into_iter()
        .filter(|id| id != mixed.id("invitation"))
        .collect()
}

fn presentation_ids(page: &crate::protocol::delivery::DeliveryPageV2) -> Vec<String> {
    page.items
        .iter()
        .filter_map(|item| match item {
            PresentationItemV2::Message { id, .. } => Some(id.as_str().to_owned()),
            PresentationItemV2::Invitation { id, .. } => Some(id.as_str().to_owned()),
            _ => None,
        })
        .collect()
}

fn mixed_fixture() -> (Fixture, CallerClaim, Mixed) {
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let mixed = publish_mixed(&f, &recipient);
    let arrival: Vec<String> = mixed.items.iter().map(|(_, id, _)| id.clone()).collect();
    let expected = mixed.expected();
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
    (f, recipient, mixed)
}

/// The hook page frames the messages in key order (deadlined ACK first, then p0, p1, p5, whatever
/// the arrival) and carries the invitation too.
#[test]
fn the_hook_page_frames_the_mixed_messages_in_key_order() {
    let (f, recipient, mixed) = mixed_fixture();
    let page = page(pull(&f, &recipient, None));
    let ids = text_ids(&page.text);
    assert!(
        ids.iter().any(|id| id == mixed.id("invitation")),
        "{}",
        page.text
    );
    assert_eq!(
        without_invitation(&mixed, ids),
        without_invitation(&mixed, mixed.expected()),
        "{}",
        page.text
    );
}

/// The hook presentation page lists the messages in the same order.
#[test]
fn the_hook_presentation_page_lists_the_mixed_messages_in_key_order() {
    let (f, recipient, mixed) = mixed_fixture();
    let crate::protocol::delivery::DeliveryPullReplyV2::Page(page) =
        presentation_test_pull(&f, &recipient)
    else {
        panic!("expected a presentation page")
    };
    let ids = presentation_ids(&page);
    assert!(
        ids.iter().any(|id| id == mixed.id("invitation")),
        "{}",
        page.text
    );
    assert_eq!(
        without_invitation(&mixed, ids),
        without_invitation(&mixed, mixed.expected()),
        "{}",
        page.text
    );
}

/// The invitation (p0, deadline) takes its place between the ACK message (earlier deadline) and
/// the p0 row without one.
#[test]
fn the_hook_page_places_the_invitation_by_its_deadline() {
    let (f, recipient, mixed) = mixed_fixture();
    let page = page(pull(&f, &recipient, None));
    assert_eq!(text_ids(&page.text), mixed.expected(), "{}", page.text);
}

/// The hook presentation page orders the invitation with the messages.
#[test]
fn the_hook_presentation_page_places_the_invitation_by_its_deadline() {
    let (f, recipient, mixed) = mixed_fixture();
    let crate::protocol::delivery::DeliveryPullReplyV2::Page(page) =
        presentation_test_pull(&f, &recipient)
    else {
        panic!("expected a presentation page")
    };
    assert_eq!(presentation_ids(&page), mixed.expected(), "{}", page.text);
}
