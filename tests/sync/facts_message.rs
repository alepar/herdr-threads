//! `message.sent` (fact-kinds design section 8; ht-5fr.4.11): the payload,
//! its keys, the local-adopt materializer branch, idempotence, origin
//! authority and the journaled `publish_send` path.
//!
//! The kind-level cases run on a [`Node`] with a hand-staged sealed
//! preparation; the command-level cases (`publish_send` on a journaled
//! thread, an oversized audience) run on a [`Sender`] store with real
//! cooperative claims.

use std::sync::Arc;

use herdr_threads::node::tag_for;
use herdr_threads::ports::{CooperativePermitRequest, DurableWorkAdmission, StorePort};
use herdr_threads::protocol::authority::{
    CallerClaim, CallerRole, Harness, LivenessFingerprint, MutationPermit, ObligationRef,
};
use herdr_threads::protocol::commands::{DeliveryMode, SendMessage};
use herdr_threads::protocol::ids::{
    ExecutionId, HostTargetId, MessageId, NativeSessionId, OperationId, SeatId, ThreadId,
    generate_public_id, prefix, send_message_id_for_preparation,
};
use herdr_threads::protocol::results::{ApiError, CommandResult, ErrorCode};
use herdr_threads::protocol::time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis};
use herdr_threads::store::connection::StoreContext;
use herdr_threads::store::journal;
use herdr_threads::store::materialize::FactRoute;
use herdr_threads::store::messages;
use herdr_threads::store::{SqliteStore, StoreSettings};
use herdr_threads::sync::EntityKey;
use herdr_threads::sync::facts::message::{MessageSent, MessageSentPayload, NEEDS_LIMIT};
use herdr_threads::sync::facts::payload::{ReceiptObligation, RecipientSpec, ReferencedSeats};
use herdr_threads::sync::facts::{FactKind, FactRegistry};
use herdr_threads::test_support::isolation::TestIsolation;
use herdr_threads::test_support::{
    assert_no_local_delivery_for_remote, fact_exchange, write_guard,
};
use rusqlite::{Connection, params};

use crate::facts_seat::{Node, registry};
use crate::facts_thread::sync_all;

/// The hand-moved clock's UTC reading.
const SENT_AT: i64 = 1_700_000_000_000;
const FIVE_MIN: i64 = 300_000;

// ---- payload helpers --------------------------------------------------------

fn spec(seat: &SeatId, receipt: ReceiptObligation, deadline_at: Option<i64>) -> RecipientSpec {
    RecipientSpec {
        seat: seat.clone(),
        receipt,
        deadline_at,
    }
}

fn sent(
    message: &MessageId,
    thread: &ThreadId,
    author: &SeatId,
    mode: DeliveryMode,
    duration_ms: i64,
    mut recipients: Vec<RecipientSpec>,
) -> MessageSentPayload {
    recipients.sort_by(|a, b| a.seat.cmp(&b.seat));
    MessageSentPayload {
        message: message.clone(),
        thread: thread.clone(),
        author: author.clone(),
        body: "hello".into(),
        mode,
        nudge: false,
        relays_user: false,
        user_intent: None,
        sent_at: SENT_AT,
        duration_ms,
        recipients,
    }
}

fn seat_n(n: usize) -> SeatId {
    SeatId::new(format!("sMany{n:08}"))
}

// ---- Node fixtures -----------------------------------------------------------

impl Node {
    /// A sealed local preparation in `thread` the way `prepare_send_step`
    /// leaves one: `acks` are `(seat, frozen duration, eligible)` rows,
    /// `lazy` the staged lazy recipients. Returns the message ID the
    /// preparation derives.
    fn stage_prep(
        &self,
        thread: &ThreadId,
        mode: &str,
        acks: &[(&SeatId, i64, bool)],
        lazy: &[&SeatId],
    ) -> MessageId {
        let prep = generate_public_id(prefix::SEND_PREPARATION, &tag_for(self.id, 0));
        let message =
            MessageId::new(send_message_id_for_preparation(&prep).expect("a send preparation"));
        let db = self.raw();
        db.execute(
            "INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,recipient_count,status,delivery_mode) \
             VALUES (?1,(SELECT id FROM local_node),'seat:stage',?1,zeroblob(32),?2,0,0,0,0,0,0,0,?3,'building',?4)",
            params![prep, thread.as_str(), acks.len() as i64, mode],
        )
        .expect("stage the preparation");
        for (n, (seat, frozen, eligible)) in acks.iter().enumerate() {
            db.execute(
                "INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot,availability_provenance) \
                 VALUES (?1,?2,?3,?4,?5,?6,'test')",
                params![prep, thread.as_str(), seat.as_str(), n as i64 + 1, frozen, eligible],
            )
            .expect("stage an ack recipient");
        }
        for seat in lazy {
            db.execute(
                "INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id) VALUES (?1,?2,?3,?4)",
                params![prep, message.as_str(), thread.as_str(), seat.as_str()],
            )
            .expect("stage a lazy recipient");
        }
        db.execute(
            "UPDATE send_preparations SET status='sealed' WHERE id=?1",
            [&prep],
        )
        .expect("seal the preparation");
        message
    }

    /// Authors `message.sent` on the journal route.
    fn author_sent(&self, payload: &MessageSentPayload) -> Result<(), ApiError> {
        self.run(|tx, d| {
            FactRoute::Journal
                .emit::<MessageSent>(tx, self.ctx(), d, payload, None, |_, _, _| Ok(()))
                .map(|_| ())
        })
    }

    /// Authors `message.sent` a second time straight through the journal (a
    /// second entry carrying the same payload).
    fn author_sent_again(&self, payload: &MessageSentPayload) -> Result<(), ApiError> {
        self.run(|tx, d| journal::author::<MessageSent>(tx, self.ctx(), d, payload).map(|_| ()))
    }

    fn count(&self, sql: &str) -> i64 {
        self.query(sql, |r| r.get(0))[0]
    }

    /// The rows one publication touches, as one comparable string.
    fn publication_rows(&self) -> String {
        format!(
            "messages={} manifests={} jobs={} wake_work={} lazy={} prepared={} next_seq={:?} timeline={:?} events={}",
            self.count("SELECT count(*) FROM messages"),
            self.count("SELECT count(*) FROM send_manifests"),
            self.count("SELECT count(*) FROM work_jobs WHERE kind='send_attention'"),
            self.count("SELECT count(*) FROM wake_work"),
            self.count("SELECT count(*) FROM lazy_recipients"),
            self.count("SELECT count(*) FROM prepared_recipients"),
            self.query("SELECT next_sequence FROM threads ORDER BY id", |r| r
                .get::<_, i64>(0)),
            self.query("SELECT timeline_revision FROM threads ORDER BY id", |r| r
                .get::<_, i64>(
                0
            )),
            self.count("SELECT count(*) FROM messages WHERE kind IN ('info','warn')"),
        )
    }

    /// `(actor, origin_node, decision_at, delivery_mode, sender_binding_id)`
    /// of an ordinary message row.
    #[allow(clippy::type_complexity)]
    fn message_row(
        &self,
        message: &MessageId,
    ) -> Option<(Option<String>, String, i64, String, Option<String>)> {
        self.query(
            &format!(
                "SELECT actor_seat_id,origin_node,decision_at,delivery_mode,sender_binding_id FROM messages WHERE id='{}'",
                message.as_str()
            ),
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .into_iter()
        .next()
    }

    /// Whether the staging table holds no leftover attribution.
    fn staging_empty(&self) -> bool {
        self.count("SELECT count(*) FROM fact_attribution_staging") == 0
    }
}

/// A node with a thread and three local seats: the author, an ack recipient
/// and a member that owes nothing.
struct Fixture {
    node: Node,
    thread: ThreadId,
    author: SeatId,
    acker: SeatId,
    quiet: SeatId,
}

impl Fixture {
    fn new(seed: u8) -> Self {
        let node = Node::with(seed, registry());
        let (author, acker, quiet) = (node.local_seat(), node.local_seat(), node.local_seat());
        let thread = node
            .create_thread(&author, "topic", None)
            .expect("create the thread");
        Self {
            node,
            thread,
            author,
            acker,
            quiet,
        }
    }

    /// An ordinary message: `acker` owes a timed receipt, `quiet` nothing.
    fn ordinary(&self) -> MessageSentPayload {
        let message = self.node.stage_prep(
            &self.thread,
            "ordinary",
            &[(&self.acker, FIVE_MIN, true)],
            &[],
        );
        self.ordinary_payload(&message)
    }

    fn ordinary_payload(&self, message: &MessageId) -> MessageSentPayload {
        sent(
            message,
            &self.thread,
            &self.author,
            DeliveryMode::Ordinary,
            FIVE_MIN,
            vec![
                spec(
                    &self.acker,
                    ReceiptObligation::Ack,
                    Some(SENT_AT + FIVE_MIN),
                ),
                spec(&self.quiet, ReceiptObligation::None, None),
            ],
        )
    }
}

// ---- payload, keys, registration --------------------------------------------

#[test]
fn round_trip() {
    let payload = MessageSentPayload {
        message: MessageId::new("mAbc12345678"),
        thread: ThreadId::new("tAbc12345678"),
        author: SeatId::new("sAAAA12345678"),
        body: "hi".into(),
        mode: DeliveryMode::Ordinary,
        nudge: false,
        relays_user: true,
        user_intent: Some(herdr_threads::protocol::summary::UserIntent::Request),
        sent_at: 1_700_000_000_000,
        duration_ms: 300_000,
        recipients: vec![
            spec(
                &SeatId::new("sBBBB12345678"),
                ReceiptObligation::Ack,
                Some(1_700_000_300_000),
            ),
            spec(&SeatId::new("sCCCC12345678"), ReceiptObligation::None, None),
        ],
    };
    let json = serde_json::to_string(&payload).unwrap();
    assert_eq!(
        json,
        r#"{"message":"mAbc12345678","thread":"tAbc12345678","author":"sAAAA12345678","body":"hi","mode":"ordinary","nudge":false,"relays_user":true,"user_intent":"request","sent_at":1700000000000,"duration_ms":300000,"recipients":[{"seat":"sBBBB12345678","receipt":"ack","deadline_at":1700000300000},{"seat":"sCCCC12345678","receipt":"none","deadline_at":null}]}"#
    );
    assert_eq!(
        serde_json::from_str::<MessageSentPayload>(&json).unwrap(),
        payload
    );
    // An additive field is ignored; the optional fields default.
    let newer: MessageSentPayload = serde_json::from_str(
        r#"{"message":"mAbc12345678","thread":"tAbc12345678","author":"sAAAA12345678","body":"hi","sent_at":5,"future":{"x":1}}"#,
    )
    .unwrap();
    assert_eq!(newer.mode, DeliveryMode::Ordinary);
    assert_eq!(
        (newer.nudge, newer.relays_user, newer.user_intent),
        (false, false, None)
    );
    assert_eq!((newer.duration_ms, newer.recipients.len()), (0, 0));
    assert_eq!(MessageSent::NAME, "message.sent");
    assert_eq!(MessageSent::MAJOR, 1);
    assert!(
        FactRegistry::builtins().lookup("message.sent", 1).is_some(),
        "registered"
    );
    // The author comes first, then the recipients in payload order.
    assert_eq!(
        payload.referenced_seats(),
        vec![
            SeatId::new("sAAAA12345678"),
            SeatId::new("sBBBB12345678"),
            SeatId::new("sCCCC12345678")
        ]
    );
}

#[test]
fn creates_needs_with_256_cutoff() {
    let message = MessageId::new("mAbc12345678");
    let thread = ThreadId::new("tAbc12345678");
    let author = SeatId::new("sAuthor123456");
    let payload_of = |n: usize| {
        sent(
            &message,
            &thread,
            &author,
            DeliveryMode::Lazy,
            0,
            (0..n)
                .map(|i| spec(&seat_n(i), ReceiptObligation::None, None))
                .collect(),
        )
    };
    let key = |s: &str| EntityKey::new(s).unwrap();
    assert_eq!(
        MessageSent::creates(&payload_of(0)),
        Some(key("mAbc12345678"))
    );
    // Few recipients: the thread, the author and every recipient.
    let few = MessageSent::needs(&payload_of(3));
    assert_eq!(few.len(), 5);
    assert_eq!(&few[..2], &[key("tAbc12345678"), key("sAuthor123456")]);
    assert_eq!(few[2], key(seat_n(0).as_str()));
    // Exactly at the limit: thread + author + 254 recipients are all named.
    let at_limit = MessageSent::needs(&payload_of(NEEDS_LIMIT - 2));
    assert_eq!(at_limit.len(), NEEDS_LIMIT);
    assert_eq!(
        at_limit.last(),
        Some(&key(seat_n(NEEDS_LIMIT - 3).as_str()))
    );
    // Past it: the thread, the author and the first 254 recipients only.
    let over = MessageSent::needs(&payload_of(300));
    assert_eq!(over.len(), NEEDS_LIMIT);
    assert_eq!(over[0], key("tAbc12345678"));
    assert_eq!(over[1], key("sAuthor123456"));
    assert_eq!(over.last(), Some(&key(seat_n(NEEDS_LIMIT - 3).as_str())));
    assert!(
        !over.contains(&key(seat_n(NEEDS_LIMIT - 2).as_str())),
        "the 255th recipient is dropped from needs"
    );
    // referenced_seats still names everyone, so each local one is published.
    assert_eq!(payload_of(300).referenced_seats().len(), 301);
}

#[test]
fn oversized_audience_refused_before_authoring() {
    let message = MessageId::new("mAbc12345678");
    let thread = ThreadId::new("tAbc12345678");
    let author = SeatId::new("sAuthor123456");
    let build = |n: usize| {
        sent(
            &message,
            &thread,
            &author,
            DeliveryMode::Lazy,
            0,
            (0..n)
                .map(|i| spec(&seat_n(i), ReceiptObligation::None, None))
                .collect(),
        )
    };
    build(100).validate_for_author().expect("a small audience");
    let big = build(8_000);
    assert!(serde_json::to_vec(&big).unwrap().len() > 448 * 1024);
    let error = big.validate_for_author().unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert_eq!(error.detail, "audience too large to replicate");
    // The 64 KiB body plus 4,000 recipients still fits.
    let mut mid = build(4_000);
    mid.body = "x".repeat(65_536);
    mid.validate_for_author().expect("under the cap");
}

// ---- the local-adopt branch --------------------------------------------------

#[test]
fn local_branch_adopts_the_staged_preparation() {
    let f = Fixture::new(81);
    let payload = f.ordinary();
    let before = f.node.count("SELECT count(*) FROM messages");
    f.node.author_sent(&payload).unwrap();
    assert_eq!(f.node.count_kind("message.sent"), 1);
    let row = f
        .node
        .message_row(&payload.message)
        .expect("the message row");
    assert_eq!(row.0.as_deref(), Some(f.author.as_str()));
    assert_eq!(row.1, f.node.id.to_string());
    assert_eq!((row.2, row.3.as_str()), (SENT_AT, "ordinary"));
    assert_eq!(row.4, None, "no binding made this fixture message");
    assert_eq!(
        f.node.count("SELECT count(*) FROM messages"),
        before + 1,
        "one ordinary row"
    );
    assert_eq!(
        f.node.count(&format!(
            "SELECT recipient_count FROM send_manifests WHERE message_id='{}'",
            payload.message.as_str()
        )),
        1
    );
    assert_eq!(
        f.node.count(&format!(
            "SELECT count(*) FROM work_jobs WHERE id='work:send:{}'",
            payload.message.as_str()
        )),
        1,
        "the send_attention job"
    );
    assert!(f.node.staging_empty(), "attribution is consumed");
    assert_no_local_delivery_for_remote(&f.node.raw());
}

#[test]
fn lazy_message_adopts_lazy_recipients() {
    let f = Fixture::new(82);
    let message = f
        .node
        .stage_prep(&f.thread, "lazy", &[], &[&f.acker, &f.quiet]);
    let payload = sent(
        &message,
        &f.thread,
        &f.author,
        DeliveryMode::Lazy,
        0,
        vec![
            spec(&f.acker, ReceiptObligation::None, None),
            spec(&f.quiet, ReceiptObligation::None, None),
        ],
    );
    f.node.author_sent(&payload).unwrap();
    let row = f.node.message_row(&message).unwrap();
    assert_eq!(row.3, "lazy");
    assert_eq!(
        f.node
            .count("SELECT count(*) FROM work_jobs WHERE kind='send_attention'"),
        0,
        "a lazy message has no attention job"
    );
    // A lazy payload that drops a staged lazy recipient is refused.
    let second = f
        .node
        .stage_prep(&f.thread, "lazy", &[], &[&f.acker, &f.quiet]);
    let short = sent(
        &second,
        &f.thread,
        &f.author,
        DeliveryMode::Lazy,
        0,
        vec![spec(&f.acker, ReceiptObligation::None, None)],
    );
    let error = f.node.author_sent(&short).unwrap_err();
    assert_eq!(error.code, ErrorCode::StoreCorrupt);
}

#[test]
fn apply_twice_noop() {
    let f = Fixture::new(83);
    let payload = f.ordinary();
    f.node.author_sent(&payload).unwrap();
    let rows = f.node.publication_rows();
    // A second entry with the same payload: the message exists, so nothing
    // is published, evented or woken again.
    f.node.author_sent_again(&payload).unwrap();
    assert_eq!(f.node.publication_rows(), rows);
    assert_eq!(f.node.count_kind("message.sent"), 2);
    assert!(
        f.node
            .journal()
            .iter()
            .all(|(_, state, _)| state == "applied"),
        "both entries applied"
    );
    assert!(f.node.staging_empty());
}

#[test]
fn reauthored_second_incarnation_noop() {
    let f = Fixture::new(84);
    let payload = f.ordinary();
    f.node.author_sent(&payload).unwrap();
    let rows = f.node.publication_rows();
    let wake_version = f.node.query(
        "SELECT seat_id,attention_version FROM wake_work ORDER BY seat_id",
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
    );
    f.node
        .run(|tx, d| journal::begin_incarnation(tx, f.node.ctx(), d, 77, None, "host").map(|_| ()))
        .unwrap();
    // The new incarnation re-issues the same payload as its own copy.
    f.node.author_sent_again(&payload).unwrap();
    assert_eq!(
        f.node.publication_rows(),
        rows,
        "no second publication or event"
    );
    assert_eq!(
        f.node.query(
            "SELECT seat_id,attention_version FROM wake_work ORDER BY seat_id",
            |r| { Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)) }
        ),
        wake_version,
        "no second attention write"
    );
    assert_eq!(f.node.count_kind("message.sent"), 2);
    assert!(
        f.node
            .journal()
            .iter()
            .all(|(_, state, _)| state == "applied"),
        "the copy applied"
    );
}

#[test]
fn staged_audience_mismatch_aborts_nothing_committed() {
    let f = Fixture::new(85);
    let payload = f.ordinary();
    let rows = f.node.publication_rows();
    let journal_before = f.node.journal_kinds();
    let (acker, quiet) = (f.acker.clone(), f.quiet.clone());
    type Mutation = Box<dyn Fn(&mut MessageSentPayload)>;
    let mutations: Vec<(&str, Mutation)> = vec![
        (
            "an ack recipient the preparation lacks",
            Box::new(move |p| {
                for r in &mut p.recipients {
                    if r.seat == quiet {
                        r.receipt = ReceiptObligation::Ack;
                        r.deadline_at = Some(SENT_AT + FIVE_MIN);
                    }
                }
            }),
        ),
        (
            "an ack recipient turned receipt-less",
            Box::new(move |p| {
                for r in &mut p.recipients {
                    if r.seat == acker {
                        r.receipt = ReceiptObligation::None;
                        r.deadline_at = None;
                    }
                }
            }),
        ),
        (
            "a deadline off the frozen duration",
            Box::new(|p| {
                for r in &mut p.recipients {
                    if r.deadline_at.is_some() {
                        r.deadline_at = Some(SENT_AT + FIVE_MIN + 1);
                    }
                }
            }),
        ),
        (
            "an untimed receipt the preparation timed",
            Box::new(|p| {
                for r in &mut p.recipients {
                    r.deadline_at = None;
                }
            }),
        ),
        ("another duration", Box::new(|p| p.duration_ms += 1)),
        ("another mode", Box::new(|p| p.mode = DeliveryMode::Lazy)),
        (
            "another thread",
            Box::new(|p| p.thread = ThreadId::new("tOther000001")),
        ),
        (
            "the author in the audience",
            Box::new(|p| {
                let author = p.author.clone();
                p.recipients
                    .push(spec(&author, ReceiptObligation::None, None));
                p.recipients.sort_by(|a, b| a.seat.cmp(&b.seat));
            }),
        ),
    ];
    for (what, mutate) in mutations {
        let mut forged = payload.clone();
        mutate(&mut forged);
        let error = f.node.author_sent(&forged).unwrap_err();
        // Another thread fails earlier (its key is unknown to the journal);
        // every other mismatch is the corrupt-audience refusal.
        if what == "another thread" {
            assert_ne!(error.code, ErrorCode::StoreCorrupt, "{what}");
        } else {
            assert_eq!(
                error.code,
                ErrorCode::StoreCorrupt,
                "{what}: {}",
                error.detail
            );
            assert!(
                error
                    .detail
                    .contains("staged audience differs from authored audience"),
                "{what}: {}",
                error.detail
            );
        }
        assert_eq!(f.node.publication_rows(), rows, "{what}: nothing committed");
        assert_eq!(
            f.node.journal_kinds(),
            journal_before,
            "{what}: nothing journaled"
        );
        assert!(f.node.staging_empty(), "{what}: no staged attribution");
    }
    // The preparation is untouched: the right payload still publishes.
    f.node.author_sent(&payload).unwrap();
    assert_eq!(f.node.count_kind("message.sent"), 1);
}

#[test]
fn message_without_a_preparation_is_not_yet_supported() {
    // Until the synthesized branch lands (ht-5fr.4.16) a fact with no local
    // preparation is refused with a clear, non-corrupt rejection.
    let f = Fixture::new(86);
    let message = MessageId::new(
        send_message_id_for_preparation(&generate_public_id(
            prefix::SEND_PREPARATION,
            &tag_for(f.node.id, 0),
        ))
        .unwrap(),
    );
    let payload = f.ordinary_payload(&message);
    let error = f.node.author_sent(&payload).unwrap_err();
    assert_eq!(error.code, ErrorCode::Unsupported);
    assert!(
        error.detail.contains("without a local preparation"),
        "{}",
        error.detail
    );
    assert_eq!(f.node.count_kind("message.sent"), 0);
}

// ---- origin authority --------------------------------------------------------

#[test]
fn foreign_author_message_sent_failed_no_rows() {
    let reg = registry();
    let (a, b, c) = (
        Node::with(87, reg),
        Node::with(88, reg),
        Node::with(89, reg),
    );
    let b_seat = b.local_seat();
    b.create_thread(&b_seat, "b's", None).unwrap();
    let c_seat = c.local_seat();
    let thread = c.create_thread(&c_seat, "c's", None).unwrap();
    sync_all(&[&a, &b, &c]);
    assert!(c.seat(&b_seat).is_some(), "C holds B's seat");
    // C authors a message whose author is B's seat. C's own apply passes (a
    // local origin); every other node rejects it.
    let message = c.stage_prep(&thread, "ordinary", &[(&c_seat, FIVE_MIN, true)], &[]);
    let payload = sent(
        &message,
        &thread,
        &b_seat,
        DeliveryMode::Ordinary,
        FIVE_MIN,
        vec![spec(
            &c_seat,
            ReceiptObligation::Ack,
            Some(SENT_AT + FIVE_MIN),
        )],
    );
    c.author_sent(&payload).unwrap();
    for node in [&a, &b] {
        fact_exchange::sync(&node.store, &c.store);
        let failed: Vec<_> = node
            .journal()
            .into_iter()
            .filter(|(k, state, _)| k == "message.sent" && state == "failed")
            .collect();
        assert_eq!(failed.len(), 1);
        let error = failed[0].2.as_deref().unwrap();
        assert!(
            error.contains("is not the authorized author of message.sent for")
                && error.contains(b_seat.as_str()),
            "{error}"
        );
        for table in [
            "messages WHERE kind='ordinary'",
            "send_preparations",
            "send_manifests",
            "prepared_recipients",
            "lazy_recipients",
            "wake_work",
            "work_jobs WHERE kind='send_attention'",
        ] {
            assert_eq!(
                node.count(&format!("SELECT count(*) FROM {table}")),
                0,
                "{table}"
            );
        }
        assert!(node.message_row(&message).is_none());
    }
}

// ---- the command: publish_send on a journaled thread ------------------------

struct FixedClock;
impl Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(SENT_AT)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(100)
    }
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Cancellation::default(),
    }
}

/// A store with the journal open (the test store opens it) and cooperative
/// seats on a journaled thread: `author` and `acker` (agents with registered
/// bindings) and `human` (a human member, who owes no receipt).
struct Sender {
    store: SqliteStore,
    _iso: TestIsolation,
    thread: ThreadId,
    author: SeatId,
    acker: SeatId,
    human: SeatId,
}

impl Sender {
    const INSTANCE: &'static str = "34750f98bd59fcfc946da45aaabe933b";

    fn open() -> (SqliteStore, TestIsolation) {
        let iso = TestIsolation::new("facts-message-sender");
        let context =
            StoreContext::new(iso.path("store.db"), Arc::new(FixedClock)).with_test_host();
        let store = SqliteStore::new(context, Self::INSTANCE, StoreSettings::default())
            .expect("open the store");
        (store, iso)
    }

    fn new() -> Self {
        let (store, iso) = Self::open();
        let node = herdr_threads::test_support::test_node_seed().uuid;
        let seat = |k: u32| SeatId::new(generate_public_id(prefix::SEAT, &tag_for(node, k)));
        let (author, acker, human) = (seat(0), seat(0), seat(0));
        let ctx = store.context();
        let conn = ctx.open_writer().unwrap();
        conn.execute("UPDATE herdr_hosts SET host_boot='b',host_epoch=1", [])
            .unwrap();
        for (n, id) in [&author, &acker, &human].into_iter().enumerate() {
            conn.execute(
                "INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) \
                 VALUES (?1,?2,'resolved','native',?1,1,1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))",
                params![id.as_str(), Self::INSTANCE],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) \
                 VALUES ((SELECT min(host_id) FROM herdr_hosts),?1,'b',1,1,0,'fresh','term-'||?1,'inc','coherent_enumeration',1)",
                [id.as_str()],
            )
            .unwrap();
            let harness = if id == &human { "human" } else { "codex" };
            conn.execute(
                "INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation,registered_at) \
                 VALUES (?1,1,1,?1,'b',1,?4,?2,?3,'cooperative_top_level',0,'term-'||?1,'inc',0)",
                params![id.as_str(), session(n), execution(n), harness],
            )
            .unwrap();
        }
        // The thread is created by its fact; membership is still a local write.
        let thread = ThreadId::new(generate_public_id(prefix::THREAD, &tag_for(node, 0)));
        let created = herdr_threads::sync::facts::thread::ThreadCreatedPayload {
            thread: thread.clone(),
            topic: "topic".into(),
            goal: "goal".into(),
            name: None,
            creator: author.clone(),
        };
        let mut conn = conn;
        ctx.execute_decision(
            &mut conn,
            |_| Ok(()),
            |tx, d, ()| {
                FactRoute::Journal
                    .emit::<herdr_threads::sync::facts::thread::ThreadCreated>(
                        tx,
                        ctx,
                        d,
                        &created,
                        None,
                        |_, _, _| Ok(()),
                    )
                    .map(|_| ())
            },
        )
        .unwrap();
        for id in [&author, &acker, &human] {
            conn.execute(
                "INSERT INTO memberships(thread_id,seat_id,state,voluntary_state,joined_at) VALUES (?1,?2,'joined','joined',1)",
                params![thread.as_str(), id.as_str()],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES (?1,?2,1,1)",
                params![thread.as_str(), id.as_str()],
            )
            .unwrap();
        }
        conn.execute(
            "UPDATE threads SET membership_revision=membership_revision+3 WHERE id=?1",
            [thread.as_str()],
        )
        .unwrap();
        drop(conn);
        Self {
            store,
            _iso: iso,
            thread,
            author,
            acker,
            human,
        }
    }

    fn db(&self) -> Connection {
        self.store.context().open_writer().unwrap()
    }

    fn request(&self, operation: &str, mode: DeliveryMode, deadline: Option<u64>) -> SendMessage {
        SendMessage {
            delivery_mode: mode,
            thread: self.thread.clone(),
            body: "hello".into(),
            direct_recipient: None,
            invited_recipients: vec![],
            deadline_millis: deadline,
            operation: OperationId::new(operation),
            claim: CallerClaim {
                host: herdr_threads::test_support::test_host_locator(),
                instance: Self::INSTANCE.into(),
                seat: self.author.clone(),
                binding_generation: 1,
                role: CallerRole::TopLevel,
                harness: Harness::Codex,
                native_session: NativeSessionId::new(session(0)),
                execution: ExecutionId::new(execution(0)),
                target: HostTargetId::new(self.author.as_str()),
                agent_id: None,
                liveness: Some(LivenessFingerprint::TEST),
            },
            relays_user: false,
            user_intent: None,
        }
    }

    fn permit(&self, request: &SendMessage) -> MutationPermit {
        self.store
            .issue_cooperative_permit(
                CooperativePermitRequest {
                    claim: request.claim.clone(),
                    operation: request.operation.clone(),
                    obligation: ObligationRef::Control(request.thread.clone()),
                    payload_hash: herdr_threads::store::schema::canonical_digest(
                        &messages::send_payload(request),
                    )
                    .unwrap(),
                    check_in_mode: None,
                },
                &budget(),
            )
            .unwrap()
    }

    /// Prepares to `Ready` (unguarded: the preparation phase stages rows the
    /// guard would flag) and then publishes with the write guard armed.
    fn send(&self, request: &SendMessage) -> Result<CommandResult, ApiError> {
        let mut conn = self.db();
        for _ in 0..10_000 {
            let step = messages::prepare_send_step(
                self.store.context(),
                &mut conn,
                request,
                messages::MessageLimits::default(),
                &budget(),
                DurableWorkAdmission { max_units: 16 },
            )?;
            if !matches!(
                step,
                herdr_threads::ports::SendPreparationProgress::More { .. }
            ) {
                break;
            }
        }
        let _guard = write_guard::arm(&conn);
        let mut permit = self.permit(request);
        messages::publish_send(
            self.store.context(),
            &mut conn,
            request,
            &mut permit,
            &budget(),
            || 65_536,
        )
    }

    fn count(&self, sql: &str) -> i64 {
        self.db().query_row(sql, [], |r| r.get(0)).unwrap()
    }
}

fn session(n: usize) -> String {
    format!("session-{n}")
}

fn execution(n: usize) -> String {
    format!("00000000-0000-4000-8000-0000000000{n:02x}")
}

#[test]
fn local_send_authors_exactly_one_message_sent() {
    let s = Sender::new();
    let before_events = s.count("SELECT count(*) FROM messages");
    let request = s.request("op-1", DeliveryMode::Ordinary, None);
    let CommandResult::MessageSent(message) = s.send(&request).expect("the send") else {
        panic!("not a MessageSent result")
    };
    let db = s.db();
    let payloads: Vec<Vec<u8>> = db
        .prepare("SELECT payload FROM journal WHERE kind='message.sent'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(payloads.len(), 1, "exactly one message.sent");
    let payload: MessageSentPayload = serde_json::from_slice(&payloads[0]).unwrap();
    assert_eq!(payload.message, message);
    assert_eq!(
        (payload.author.clone(), payload.sent_at),
        (s.author.clone(), SENT_AT)
    );
    assert_eq!(payload.mode, DeliveryMode::Ordinary);
    // The audience is complete: the registered agent owes a timed receipt, the
    // binding-less member owes nothing, the author is excluded.
    let mut expected = vec![
        spec(&s.acker, ReceiptObligation::Ack, Some(SENT_AT + FIVE_MIN)),
        spec(&s.human, ReceiptObligation::None, None),
    ];
    expected.sort_by(|a, b| a.seat.cmp(&b.seat));
    assert_eq!(payload.recipients, expected);
    assert_eq!(payload.duration_ms, FIVE_MIN);
    // The rows a local send always left.
    let row: (String, String, i64, Option<String>, String) = db
        .query_row(
            "SELECT actor_seat_id,delivery_mode,decision_at,sender_binding_id,origin_node FROM messages WHERE id=?1",
            [message.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    assert_eq!(row.0, s.author.as_str());
    assert_eq!((row.1.as_str(), row.2), ("ordinary", SENT_AT));
    assert!(row.3.is_some(), "the acting binding is attributed");
    assert_eq!(row.4, Sender::INSTANCE);
    assert_eq!(
        s.count("SELECT count(*) FROM messages") - before_events,
        1,
        "one ordinary row"
    );
    assert_eq!(s.count("SELECT count(*) FROM send_manifests"), 1);
    assert_eq!(
        s.count("SELECT count(*) FROM work_jobs WHERE kind='send_attention'"),
        1
    );
    assert_eq!(s.count("SELECT count(*) FROM fact_attribution_staging"), 0);
    assert_eq!(
        s.count(
            "SELECT count(*) FROM journal WHERE kind='message.sent' AND apply_state<>'applied'"
        ),
        0
    );
    assert_no_local_delivery_for_remote(&db);
    // A retry of the same operation replays the stored result and authors
    // nothing more.
    let replay = s.send(&request).expect("the replay");
    assert_eq!(replay, CommandResult::MessageSent(message));
    assert_eq!(
        s.count("SELECT count(*) FROM journal WHERE kind='message.sent'"),
        1
    );
}

#[test]
fn lazy_local_send_authors_none_receipts() {
    let s = Sender::new();
    let request = s.request("op-lazy", DeliveryMode::Lazy, None);
    let CommandResult::MessageSent(message) = s.send(&request).expect("the send") else {
        panic!("not a MessageSent result")
    };
    let payload: MessageSentPayload = serde_json::from_slice(
        &s.db()
            .query_row::<Vec<u8>, _, _>(
                "SELECT payload FROM journal WHERE kind='message.sent'",
                [],
                |r| r.get(0),
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(payload.message, message);
    assert_eq!(payload.mode, DeliveryMode::Lazy);
    assert!(
        payload
            .recipients
            .iter()
            .all(|r| r.receipt == ReceiptObligation::None && r.deadline_at.is_none()),
        "{:?}",
        payload.recipients
    );
    let mut seats: Vec<&SeatId> = payload.recipients.iter().map(|r| &r.seat).collect();
    seats.sort();
    let mut want = vec![&s.acker, &s.human];
    want.sort();
    assert_eq!(seats, want);
    assert_eq!(s.count("SELECT count(*) FROM lazy_recipients"), 2);
    assert_eq!(
        s.count("SELECT count(*) FROM work_jobs WHERE kind='send_attention'"),
        0
    );
}

#[test]
fn oversized_send_is_refused_before_anything_is_authored() {
    let s = Sender::new();
    // 6,000 more lazy members, with long IDs so the audience passes 448 KiB.
    s.db()
        .execute_batch(
            "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<6000) \
             INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) \
             SELECT 'sBulk'||printf('%070d',x),(SELECT id FROM local_node),'resolved','native','t'||x,1,1,0,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts) FROM n;",
        )
        .unwrap();
    let thread = s.thread.as_str().to_owned();
    s.db()
        .execute(
            "INSERT INTO memberships(thread_id,seat_id,state,voluntary_state,joined_at) SELECT ?1,id,'joined','joined',1 FROM seats WHERE id LIKE 'sBulk%'",
            [&thread],
        )
        .unwrap();
    s.db()
        .execute(
            "INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) SELECT ?1,id,1,1 FROM seats WHERE id LIKE 'sBulk%'",
            [&thread],
        )
        .unwrap();
    s.db()
        .execute(
            "UPDATE threads SET membership_revision=membership_revision+6000 WHERE id=?1",
            [&thread],
        )
        .unwrap();
    let request = s.request("op-big", DeliveryMode::Lazy, None);
    let error = s.send(&request).unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest, "{}", error.detail);
    assert_eq!(error.detail, "audience too large to replicate");
    assert_eq!(
        s.count("SELECT count(*) FROM journal WHERE kind='message.sent'"),
        0
    );
    assert_eq!(
        s.count("SELECT count(*) FROM messages WHERE kind='ordinary'"),
        0
    );
    assert_eq!(s.count("SELECT count(*) FROM send_manifests"), 0);
    assert_eq!(s.count("SELECT count(*) FROM fact_attribution_staging"), 0);
    assert_eq!(
        s.count("SELECT count(*) FROM seats WHERE id LIKE 'sBulk%' AND published=1"),
        0,
        "no audience seat was published"
    );
}

#[test]
fn local_only_route_for_a_journal_less_thread_journals_nothing() {
    // A pair thread `ensure_direct_thread` still inserts directly (no creating
    // fact) keeps the node-local write: nothing is journaled for its send.
    let s = Sender::new();
    s.db()
        .execute(
            "INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,journaled) VALUES ('tNoFact000001',?1,'t','g',0,0,1)",
            [Sender::INSTANCE],
        )
        .unwrap();
    for id in [&s.author, &s.acker] {
        s.db()
            .execute(
                "INSERT INTO memberships(thread_id,seat_id,state,voluntary_state,joined_at) VALUES ('tNoFact000001',?1,'joined','joined',1)",
                [id.as_str()],
            )
            .unwrap();
        s.db()
            .execute(
                "INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('tNoFact000001',?1,1,1)",
                [id.as_str()],
            )
            .unwrap();
    }
    let mut request = s.request("op-nofact", DeliveryMode::Ordinary, None);
    request.thread = ThreadId::new("tNoFact000001");
    s.send(&request).expect("the send");
    assert_eq!(
        s.count("SELECT count(*) FROM journal WHERE kind='message.sent'"),
        0
    );
    assert_eq!(
        s.count(
            "SELECT count(*) FROM messages WHERE thread_id='tNoFact000001' AND kind='ordinary'"
        ),
        1
    );
}
