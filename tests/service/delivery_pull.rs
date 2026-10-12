//! Daemon-side hook-pull delivery (delivery-policy DP4-DP6, ht-poh.3.4): the
//! `DeliveryPull` page, the offered ledger and `DeliveryConfirm`, driven through
//! `DomainService` over a temp `SqliteStore` with the test-support
//! `synthetic_fourth` adapter (`HookPull`, `cooperative_hook_delivery`) as the
//! pull seat.
use super::*;
use crate::{
    delivery::{
        HEADER,
        ledger::{MAX_OFFERS_PER_GENERATION, REOFFER_AFTER_MS},
    },
    ports::{HookPullOutlook, LocalService, NextPull, StorePort},
    protocol::{
        authority::{CallerClaim, CallerRole, Harness, PeerIdentity},
        commands::{CheckIn, CheckInMode, Command},
        delivery::{
            DeliveryConfirm, DeliveryConfirmOutcome as Outcome, DeliveryConfirmReason as Reason,
            DeliveryPull, DeliveryPullReply, DeliveryRefusalReason as Refusal,
        },
        ids::*,
        results::CommandResult,
        time::{CallBudget, Clock, MonoInstant, UtcMillis},
        watch::WATCH_BODY_LIMIT_BYTES,
    },
    service::{delivery_pull::DeliveryPullState, dispatch::DomainService, fair_writer::FairWriter},
    store::{SqliteStore, StoreSettings, connection::StoreContext},
};
use rusqlite::{Connection, params};
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

#[path = "q4_accept_controls.rs"]
mod q4_accept_controls;

// Q3 controls keep publication, canonical reader changes and channel input in
// separate steps. A parked client never obtains success by unconditional Pull.
struct GuardModStoreReads(Arc<dyn StorePort>);
impl crate::ports::ModStoreReads for GuardModStoreReads {
    fn mod_binding_view(
        &self,
        _: &SeatId,
        _: crate::ports::ModBindingRef<'_>,
        _: &CallBudget,
    ) -> Result<Option<crate::ports::ModSeatView>, crate::protocol::results::ApiError> {
        panic!("capable channel invoked the legacy aggregate view")
    }
    fn presentation_seat_view(
        &self,
        seat: &SeatId,
        binding: crate::ports::ModBindingRef<'_>,
        budget: &CallBudget,
    ) -> Result<Option<crate::ports::ModSeatView>, crate::protocol::results::ApiError> {
        self.0.presentation_seat_view(seat, binding, budget)
    }
    fn presentation_refresh(
        &self,
        seat: &SeatId,
        binding: &BindingId,
        generation: u64,
        native: &str,
        budget: &CallBudget,
    ) -> Result<Option<crate::ports::ModPresentationRefresh>, crate::protocol::results::ApiError>
    {
        self.0
            .presentation_refresh(seat, binding, generation, native, budget)
    }
    fn mod_stall_oldest(
        &self,
        seat: &SeatId,
        at: UtcMillis,
        limit: usize,
        budget: &CallBudget,
    ) -> Result<Option<UtcMillis>, crate::protocol::results::ApiError> {
        self.0.mod_stall_oldest(seat, at, limit, budget)
    }
}

#[derive(Default)]
struct Q3Sink(std::sync::Mutex<Vec<crate::protocol::watch::WatchFrame>>);
impl crate::ports::ModChannelSink for Q3Sink {
    fn push(&self, frame: crate::protocol::watch::WatchFrame) -> bool {
        self.0.lock().unwrap().push(frame);
        true
    }
}

struct Q3Channel {
    window: OfferWindowFixture,
    sink: Arc<Q3Sink>,
}
impl Q3Channel {
    fn new() -> Self {
        use crate::{
            ports::ModChannels,
            protocol::watch::{ModDeliverySetting, WatchRequest},
            service::mod_channels::ModChannelRegistry,
        };
        let mut f = fixture(0);
        let recipient = check_in(
            &f,
            fixture_claim(&f, "c", "pc", Harness::Claude, "q3-reader-b"),
            "q3-reader-b-ci",
        );
        let sender = check_in(
            &f,
            fixture_claim(&f, "x", "px", fourth(), "q3-sender"),
            "q3-sender-ci",
        );
        let registry = Arc::new(ModChannelRegistry::new(
            f.clock.clone(),
            Arc::new(GuardModStoreReads(f.store.clone())),
            "i".into(),
            Arc::new(|| {}),
            ModDeliverySetting::On,
        ));
        f.service = DomainService::new("i".into(), f.store.clone(), f.clock.clone())
            .with_cooperative_owner(501, Arc::new(FairWriter::new(32)))
            .with_delivery_pull(f.state.clone())
            .with_mod_channels(registry.clone());
        let sink = Arc::new(Q3Sink::default());
        let (channel, _) = f
            .service
            .watch_register(
                &WatchRequest {
                    presentation_v2: true,
                    claim: recipient.clone(),
                },
                sink.clone(),
                &budget(),
            )
            .unwrap();
        let lease = registry.presentation_lease(channel).unwrap();
        let this = Self {
            window: OfferWindowFixture {
                f,
                recipient,
                sender,
                watch: Some((registry, channel, lease)),
            },
            sink,
        };
        // Actual Accepted drain and bounded continuation precede quietness.
        this.drain_until_quiet();
        this.registry().pass(now_utc(&this.window.f)).unwrap();
        this.take_frames();
        this.registry().pass(now_utc(&this.window.f)).unwrap();
        assert!(this.take_frames().is_empty());
        this
    }
    fn registry(&self) -> &crate::service::mod_channels::ModChannelRegistry {
        &self.window.watch.as_ref().unwrap().0
    }
    fn take_frames(&self) -> Vec<crate::protocol::watch::WatchFrame> {
        std::mem::take(&mut *self.sink.0.lock().unwrap())
    }
    fn drain_until_quiet(&self) {
        self.drain_continuation(self.window.pull());
    }
    fn drain_continuation(
        &self,
        mut reply: crate::protocol::delivery::DeliveryPullReplyV2,
    ) -> std::collections::BTreeSet<MessageId> {
        use crate::protocol::delivery::DeliveryPullReplyV2;
        let mut shown = std::collections::BTreeSet::new();
        for _ in 0..256 {
            match reply {
                DeliveryPullReplyV2::Unchanged { .. } => return shown,
                DeliveryPullReplyV2::Page(page) => {
                    for id in page.offered.iter().chain(&page.truncated) {
                        assert!(
                            shown.insert(id.clone()),
                            "fixed-clock continuation cannot reoffer {id:?}"
                        );
                    }
                    if !page.more {
                        return shown;
                    }
                    reply = self.window.pull();
                }
                other => panic!("admitted continuation refused: {other:?}"),
            }
        }
        panic!("existing bounded continuation did not quiet")
    }
    fn sibling(&self) -> CallerClaim {
        let mut claim = fixture_claim(&self.window.f, "c", "pc", Harness::Claude, "q3-reader-a");
        claim.liveness.as_mut().unwrap().pid = 4242;
        let claim = check_in(&self.window.f, fresh_execution(claim, 9), "q3-reader-a-ci");
        assert_ne!(
            binding_for_claim(&self.window.f, &claim),
            binding_for_claim(&self.window.f, &self.window.recipient)
        );
        assert_eq!(self.window.f.db.query_row(
            "SELECT count(*) FROM occupant_bindings WHERE seat_id='c' AND ended_at IS NULL AND liveness='connected'",
            [], |r| r.get::<_, i64>(0),
        ).unwrap(), 2);
        claim
    }
    fn native_version(&self) -> i64 {
        self.window
            .f
            .db
            .query_row(
                "SELECT COALESCE((SELECT attention_version FROM wake_work WHERE seat_id='c'),0)",
                [],
                |row| row.get(0),
            )
            .unwrap()
    }
    fn finish_attention(&self, message: &MessageId) {
        let job: String = self.window.f.db.query_row(
            "SELECT id FROM work_jobs WHERE kind='send_attention' AND subject_id=(SELECT preparation_id FROM send_manifests WHERE message_id=?1)",
            [message.as_str()], |row| row.get(0),
        ).unwrap();
        finish_presentation_attention_job(&self.window.f, &job);
    }
    fn ack_as(&self, claim: &CallerClaim, message: &MessageId, operation: &str) {
        let result = self
            .window
            .f
            .service
            .handle(
                Command::Ack(crate::protocol::commands::Ack {
                    claim: claim.clone(),
                    messages: vec![message.clone()],
                    operation: OperationId::new(operation),
                }),
                peer(),
                &budget(),
            )
            .unwrap();
        assert!(
            matches!(result, CommandResult::Acknowledged(_)),
            "{result:?}"
        );
    }
    fn frame_then_pull(&self) -> crate::protocol::delivery::DeliveryPullReplyV2 {
        use crate::protocol::watch::WatchFrame;
        // These are normal safety/worker passes at the unchanged clock. No
        // notify, mark_dirty, reconnect or synthetic version drives the client.
        let manifests: i64 = self
            .window
            .f
            .db
            .query_row(
                "SELECT count(*) FROM send_manifests WHERE instance_id='i'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        // The stable-prefix publication contract is ceil(M/100)+2 successful
        // quanta. There are no concurrent arrivals, new threads or deadlines.
        let manifests = usize::try_from(manifests).expect("nonnegative manifest count fits usize");
        for _ in 0..manifests.div_ceil(100) + 2 {
            self.registry().pass(now_utc(&self.window.f)).unwrap();
            let frames = self.take_frames();
            if !frames.is_empty() {
                assert!(
                    frames
                        .iter()
                        .all(|v| matches!(v, WatchFrame::Attention { .. }))
                );
                return self.window.pull();
            }
        }
        panic!("canonical change never requested the parked channel's next Pull")
    }
    fn receipt(&self, message: &MessageId) -> String {
        use crate::store::effective::{EffectiveReceiptState, effective_receipt};
        let receipt = effective_receipt(&self.window.f.db, message.as_str(), "c")
            .unwrap()
            .expect("canonical addressed receipt exists");
        assert_eq!(receipt.message_id, message.as_str());
        assert_eq!(receipt.seat_id, "c");
        assert_eq!(receipt.thread_id, "t");
        match receipt.state {
            EffectiveReceiptState::Pending => "pending",
            EffectiveReceiptState::Acknowledged => "acked",
            EffectiveReceiptState::RecipientRetired => "recipient_retired",
            EffectiveReceiptState::NotRequired => "not_required",
        }
        .into()
    }
}

// Kills Lazy-only publication observation when an ordinary recipient's shared
// receipt settles before the materializer can create its native signal.
#[test]
fn presentation_q3_quiet_ordinary_sibling_ack_before_materialization_drains_passively() {
    use crate::protocol::{
        commands::DeliveryMode,
        delivery::{DeliveryPullReplyV2, PresentationItemV2},
    };
    let q = Q3Channel::new();
    let a = q.sibling();
    let native = q.native_version();
    let body = "ordinary published before sibling settlement";
    let message = q.window.send(0, DeliveryMode::Ordinary, body);
    assert_eq!(q.native_version(), native);
    assert_eq!(
        q.window
            .f
            .db
            .query_row(
                "SELECT count(*) FROM receipts WHERE message_id=?1 AND seat_id='c'",
                [message.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0,
        "the ACK race precedes physical materialization"
    );
    assert_eq!(reclamation_read_full_body(&q.window.f, &message), body);
    reclamation_advance_displayed(&q.window.f, &a, std::slice::from_ref(&message), "q3-a-read");
    let b_cursor = reclamation_cursor(&q.window.f, &q.window.recipient);
    q.ack_as(&a, &message, "q3-a-early-ack");
    assert_eq!(q.receipt(&message), "acked");
    assert_eq!(
        reclamation_cursor(&q.window.f, &q.window.recipient),
        b_cursor
    );
    q.finish_attention(&message);
    assert_eq!(
        q.native_version(),
        native,
        "terminal materialization creates no pending signal"
    );
    let DeliveryPullReplyV2::Page(page) = q.frame_then_pull() else {
        panic!("passive body page")
    };
    assert!(page.offered.contains(&message));
    assert!(page.items.iter().any(
        |item| matches!(item, PresentationItemV2::Message { id, lazy: false, content }
        if id == &message && content.body == body && !content.ack_required && !content.wakes)
    ));
    assert!(page.emitted_controls.is_empty());
    assert_eq!(
        q.receipt(&message),
        "acked",
        "presentation cannot recreate an obligation"
    );
    assert_eq!(q.native_version(), native);
    assert_eq!(
        reclamation_cursor(&q.window.f, &q.window.recipient),
        b_cursor
    );
    use crate::ports::ModChannels;
    let (a_channel, _) = q
        .window
        .f
        .service
        .watch_register(
            &crate::protocol::watch::WatchRequest {
                presentation_v2: true,
                claim: a.clone(),
            },
            Arc::new(Q3Sink::default()),
            &budget(),
        )
        .unwrap();
    let CommandResult::DeliveryPulledV2(a_page) = q
        .window
        .f
        .service
        .handle(
            Command::DeliveryPullV2(crate::protocol::delivery::DeliveryPullV2 {
                claim: a,
                consumer: crate::protocol::delivery::PresentationConsumer::Watch,
                render: crate::protocol::delivery::PresentationRenderRequest::WatchJsonLinesV1,
                presentation_lease: q.registry().presentation_lease(a_channel),
                marker_prefix: vec![],
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("A's real Watch drain")
    };
    assert!(!matches!(a_page, DeliveryPullReplyV2::Refused { .. }));
    assert!(
        !matches!(a_page, DeliveryPullReplyV2::Page(ref page) if page.offered.contains(&message)),
        "A's own completed cursor suppresses its settled body"
    );
    q.registry().unregister(a_channel, now_utc(&q.window.f));
    q.drain_continuation(DeliveryPullReplyV2::Page(page));
    q.registry().pass(now_utc(&q.window.f)).unwrap();
    q.take_frames();
    q.registry().pass(now_utc(&q.window.f)).unwrap();
    assert!(
        q.take_frames().is_empty(),
        "fixed eligibility does not repeat a passive drain"
    );
}

// Kills treating transport drain requests as message ACK/wake authority. Both
// paths use actual producers and materializers; their resulting items differ.
#[test]
fn presentation_q3_quiet_pending_and_lazy_publications_have_distinct_authority() {
    use crate::protocol::{
        commands::DeliveryMode,
        delivery::{DeliveryPullReplyV2, PresentationItemV2},
    };
    for lazy in [false, true] {
        let q = Q3Channel::new();
        let native = q.native_version();
        let message = q.window.send(
            0,
            if lazy {
                DeliveryMode::Lazy
            } else {
                DeliveryMode::Ordinary
            },
            "canonical quiet publication",
        );
        if !lazy {
            q.finish_attention(&message);
        }
        let after_publication = q.native_version();
        assert_eq!(after_publication > native, !lazy);
        let DeliveryPullReplyV2::Page(page) = q.frame_then_pull() else {
            panic!("published content page")
        };
        assert!(page.items.iter().any(|item| matches!(item, PresentationItemV2::Message { id, lazy: actual_lazy, content }
            if id == &message && *actual_lazy == lazy && content.body == "canonical quiet publication"
                && content.ack_required != lazy && !content.wakes)));
        assert!(page.emitted_controls.is_empty());
        assert_eq!(q.native_version(), after_publication);
        if lazy {
            assert_eq!(q.window.f.db.query_row(
                "SELECT (SELECT count(*) FROM receipts WHERE message_id=?1)+(SELECT count(*) FROM receipt_state WHERE message_id=?1)",
                [message.as_str()], |row| row.get::<_, i64>(0),
            ).unwrap(), 0);
            assert_eq!(
                q.window
                    .f
                    .db
                    .query_row(
                        "SELECT state FROM lazy_recipients WHERE message_id=?1 AND seat_id='c'",
                        [message.as_str()],
                        |row| row.get::<_, String>(0),
                    )
                    .unwrap(),
                "pending",
                "the drain hint cannot complete display"
            );
        } else {
            assert_eq!(q.receipt(&message), "pending");
        }
    }
}

fn assert_q3_parked_reader_progress(read_first: bool) {
    use crate::protocol::{commands::DeliveryMode, delivery::DeliveryPullReplyV2};
    let q = Q3Channel::new();
    let sent = (0..257)
        .map(|n| q.window.send(n, DeliveryMode::Ordinary, "real parked body"))
        .collect::<Vec<_>>();
    q.finish_attention(&sent[0]);
    let initial = q.frame_then_pull();
    assert_eq!(
        q.drain_continuation(initial),
        sent[..256].iter().cloned().collect()
    );
    let stamps = q.window.offers();
    assert_eq!(stamps.len(), 256);
    assert!(!stamps.contains_key(&sent[256]));
    q.registry().pass(now_utc(&q.window.f)).unwrap();
    q.take_frames();
    q.registry().pass(now_utc(&q.window.f)).unwrap();
    assert!(q.take_frames().is_empty());
    let parked = capacity_normal_state(&q.window);
    let cursor = reclamation_cursor(&q.window.f, &q.window.recipient);
    let native = q.native_version();
    let read = || {
        assert_eq!(
            reclamation_read_full_body(&q.window.f, &sent[0]),
            "real parked body"
        );
        reclamation_advance_displayed(
            &q.window.f,
            &q.window.recipient,
            &sent[..1],
            "q3-parked-read",
        );
    };
    if read_first {
        read();
        assert!(reclamation_cursor(&q.window.f, &q.window.recipient) > cursor);
    } else {
        q.ack_as(&q.window.recipient, &sent[0], "q3-parked-ack");
    }
    // The first authority alone cannot evict unread or pending offers. Consume
    // any legitimate first-event frame, but never poll a parked client.
    q.registry().pass(now_utc(&q.window.f)).unwrap();
    if !q.take_frames().is_empty() {
        q.drain_until_quiet();
    }
    assert_eq!(q.window.offers(), stamps);
    assert_eq!(capacity_normal_state(&q.window), parked);
    if read_first {
        q.ack_as(&q.window.recipient, &sent[0], "q3-parked-ack");
    } else {
        read();
    }
    assert_eq!(q.native_version(), native);
    assert_eq!(q.receipt(&sent[0]), "acked");
    let DeliveryPullReplyV2::Page(page) = q.frame_then_pull() else {
        panic!("released capacity resumes backlog")
    };
    assert!(
        page.offered.contains(&sent[256]),
        "ID257 must arrive without new send or reconnect: {page:?}"
    );
    assert!(!page.offered.contains(&sent[0]));
    let offers = q.window.offers();
    assert!(!offers.contains_key(&sent[0]));
    assert!(offers.contains_key(&sent[256]));
    for id in &sent[1..256] {
        assert_eq!(offers.get(id), stamps.get(id));
    }
    assert_eq!(q.receipt(&sent[256]), "pending");
    assert_eq!(q.native_version(), native);
}

#[test]
fn presentation_q3_parked_reader_then_ack_resumes_existing_id257() {
    assert_q3_parked_reader_progress(true);
}

#[test]
fn presentation_q3_parked_ack_then_reader_resumes_existing_id257() {
    assert_q3_parked_reader_progress(false);
}

#[test]
fn presentation_q3_capable_registry_uses_only_bounded_real_store_reads() {
    let q = Q3Channel::new();
    for _ in 0..3 {
        q.registry().pass(now_utc(&q.window.f)).unwrap();
    }
    assert!(q.take_frames().is_empty());
    assert_eq!(q.native_version(), 0);
}

// Kills omission of the receipt recheck independently of B's own reader
// invalidation: the latter is fully consumed before sibling A settles ID256.
#[test]
fn presentation_q3_shared_ack_rechecks_read_captured_body_but_keeps_unread_sibling() {
    use crate::protocol::{commands::DeliveryMode, delivery::DeliveryPullReplyV2};
    for read_b in [true, false] {
        let q = Q3Channel::new();
        let a = q.sibling();
        let body = "\0".repeat(58_000);
        let mut sent = (0..255)
            .map(|n| {
                q.window.send(
                    n,
                    DeliveryMode::Ordinary,
                    "complete prefix before captured body",
                )
            })
            .collect::<Vec<_>>();
        let captured = q.window.send(255, DeliveryMode::Ordinary, &body);
        sent.push(captured.clone());
        let successor = q
            .window
            .send(256, DeliveryMode::Ordinary, "successor after captured body");
        q.finish_attention(&sent[0]);
        let shown = q.drain_continuation(q.frame_then_pull());
        assert_eq!(
            shown,
            sent.iter().cloned().collect(),
            "SETUP: all 256 real offers precede parking"
        );
        let stamps = q.window.offers();
        assert_eq!(stamps.len(), 256, "SETUP: full real logical ledger");
        assert!(
            stamps[&captured].truncated,
            "SETUP: captured identity was actually offered"
        );
        assert!(!stamps.contains_key(&successor));
        let captured_cursor = || {
            q.window.f.db.query_row(
            "SELECT ordinary_cursor,ordinary_skip_body,ordinary_partial FROM invitation_presentation_consumers WHERE binding_id=?1 AND consumer='watch'",
            [binding_for_claim(&q.window.f, &q.window.recipient).as_str()],
            |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, String>(2)?)),
        ).unwrap()
        };
        let initial = captured_cursor();
        assert_eq!(
            initial.1.as_deref(),
            Some(captured.as_str()),
            "SETUP: actual offered skip-body remains captured at more:false"
        );
        assert_eq!(initial.2, "{}");
        let cursor = crate::protocol::pagination::InboxBatchV2CursorState::decode_for(
            initial.0.as_deref().expect("SETUP: continued cursor"),
            "i",
            &q.window.recipient.seat,
        )
        .unwrap();
        let position = cursor.body.expect("SETUP: actual captured continuation");
        assert_eq!(position.message, captured);
        assert!(
            position.offset >= WATCH_BODY_LIMIT_BYTES as u64 && position.offset < body.len() as u64
        );
        // Consume publication and any existing frame with its real continuation.
        q3_consume_frames_to_quiet(&q);
        let parked = capacity_normal_state(&q.window);
        let native = q.native_version();
        let before_reader = reclamation_cursor(&q.window.f, &q.window.recipient);
        if read_b {
            assert_eq!(
                reclamation_read_full_body_with_page(
                    &q.window.f,
                    &captured,
                    crate::protocol::pagination::MAX_PAGE_BYTES
                ),
                body
            );
            // The 255 small bodies were completely emitted by the real pages;
            // only the truncated 256th needed a public body continuation.
            reclamation_advance_displayed(
                &q.window.f,
                &q.window.recipient,
                &sent,
                "q3-captured-b-read",
            );
            assert!(reclamation_cursor(&q.window.f, &q.window.recipient) > before_reader);
        }
        // This is the causal fence: a reader-only production implementation can
        // produce all of its frames now, and none survive to the A-only event.
        q3_consume_frames_to_quiet(&q);
        assert_eq!(
            q.window.offers(),
            stamps,
            "Pending keeps every full offer stamp even after B read"
        );
        assert_eq!(
            capacity_normal_state(&q.window),
            parked,
            "Pending/full capacity keeps the captured body parked"
        );
        assert_eq!(captured_cursor(), initial);
        assert!(q.take_frames().is_empty());
        assert_eq!(q.window.f.db.query_row(
            "SELECT COALESCE((SELECT state FROM receipt_state WHERE message_id=?1 AND seat_id='c'),'pending')",
            [captured.as_str()], |row| row.get::<_, String>(0),
        ).unwrap(), "pending");
        // No send, read, display, notification or retained reader hint intervenes.
        q.ack_as(&a, &captured, "q3-captured-a-only-ack");
        assert_eq!(q.native_version(), native);
        let reply = q.frame_then_pull();
        assert!(!matches!(reply, DeliveryPullReplyV2::Refused { .. }));
        if read_b {
            assert!(
                matches!(reply, DeliveryPullReplyV2::Page(ref page) if page.offered.contains(&successor)),
                "A-only event releases the read offered ID256 and emits ID257: {reply:?}"
            );
            assert!(!q.window.offers().contains_key(&captured));
            assert_ne!(captured_cursor().1.as_deref(), Some(captured.as_str()));
            let work = q
                .window
                .f
                .service
                .presentation
                .last_work
                .lock()
                .unwrap()
                .unwrap();
            assert!(
                work.ordinary_continuation_key_probes > 0
                    && work.ordinary_continuation_key_probes <= 15,
                "same frame-driven Pull must actually retire the Q1 continued body: {work:?}"
            );
        } else {
            assert_eq!(
                q.window.offers(),
                stamps,
                "A's settlement cannot reclaim unread B's offered body"
            );
            assert_eq!(capacity_normal_state(&q.window), parked);
            assert_eq!(captured_cursor(), initial);
            assert_eq!(
                reclamation_cursor(&q.window.f, &q.window.recipient),
                before_reader
            );
        }
        assert_eq!(q.receipt(&captured), "acked");
        assert_eq!(q.native_version(), native);
    }
}

fn q3_consume_frames_to_quiet(q: &Q3Channel) {
    let manifests: i64 = q
        .window
        .f
        .db
        .query_row(
            "SELECT count(*) FROM send_manifests WHERE instance_id='i'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let passes = usize::try_from(manifests).unwrap().div_ceil(100) + 2;
    for _ in 0..passes {
        q.registry().pass(now_utc(&q.window.f)).unwrap();
        let frames = q.take_frames();
        if !frames.is_empty() {
            assert!(frames.iter().all(|frame| matches!(
                frame,
                crate::protocol::watch::WatchFrame::Attention { .. }
            )));
            assert!(
                q.drain_continuation(q.window.pull()).is_empty(),
                "maintenance cannot allocate at fixed full capacity"
            );
        }
    }
    q.registry().pass(now_utc(&q.window.f)).unwrap();
    assert!(
        q.take_frames().is_empty(),
        "SETUP: all prior reader/publication notifications must be consumed before A ACK"
    );
}

fn q3_consumer_snapshot(
    f: &Fixture,
    claim: &CallerClaim,
) -> Vec<(String, Vec<rusqlite::types::Value>)> {
    let mut query =
        f.db.prepare(
            "SELECT * FROM invitation_presentation_consumers WHERE binding_id=?1 ORDER BY consumer",
        )
        .unwrap();
    query
        .query_map([binding_for_claim(f, claim).as_str()], |row| {
            Ok((
                row.get::<_, String>("consumer")?,
                (0..row.as_ref().column_count())
                    .map(|column| row.get::<_, rusqlite::types::Value>(column))
                    .collect::<rusqlite::Result<Vec<_>>>()?,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

fn q3_preserved_reader_state(f: &Fixture, claim: &CallerClaim) -> Vec<Vec<rusqlite::types::Value>> {
    let mut query =
        f.db.prepare(
            "SELECT * FROM invitation_presentation_consumers WHERE binding_id=?1 ORDER BY consumer",
        )
        .unwrap();
    let preserved = query
        .column_names()
        .iter()
        .enumerate()
        .filter_map(|(index, name)| {
            (!matches!(*name, "ordinary_passive_generation" | "mod_version")).then_some(index)
        })
        .collect::<Vec<_>>();
    query
        .query_map([binding_for_claim(f, claim).as_str()], |row| {
            preserved
                .iter()
                .map(|column| row.get::<_, rusqlite::types::Value>(*column))
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

fn q3_reader_fixture(consumers: usize) -> (Fixture, CallerClaim, CallerClaim, MessageId) {
    let f = fixture(0);
    let a = pull_seat(&f);
    let b = second_binding(&f);
    let sender = check_in(
        &f,
        fixture_claim(&f, "x", "px", fourth(), "q3-reader-sender"),
        "q3-reader-sender-ci",
    );
    for claim in [&a, &b] {
        if consumers > 0 {
            let reply = presentation_test_pull(&f, claim);
            assert!(!matches!(
                reply,
                crate::protocol::delivery::DeliveryPullReplyV2::Refused { .. }
            ));
        }
        if consumers > 1 {
            f.store
                .presentation_refresh(
                    &claim.seat,
                    &binding_for_claim(&f, claim),
                    claim.binding_generation,
                    claim.native_session.as_str(),
                    &budget(),
                )
                .unwrap()
                .unwrap();
        }
    }
    let CommandResult::MessageSent(message) = f
        .service
        .handle(
            Command::SendMessage(crate::protocol::commands::SendMessage {
                delivery_mode: crate::protocol::commands::DeliveryMode::Ordinary,
                user_intent: None,
                thread: ThreadId::new("t"),
                body: "reader invalidation canonical body".into(),
                direct_recipient: None,
                invited_recipients: vec![],
                deadline_millis: None,
                operation: OperationId::new("q3-reader-send"),
                claim: sender,
                relays_user: false,
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("canonical reader fixture send")
    };
    let job: String = f.db.query_row("SELECT id FROM work_jobs WHERE kind='send_attention' AND subject_id=(SELECT preparation_id FROM send_manifests WHERE message_id=?1)", [message.as_str()], |row| row.get(0)).unwrap();
    finish_presentation_attention_job(&f, &job);
    assert_eq!(
        reclamation_read_full_body(&f, &message),
        "reader invalidation canonical body"
    );
    (f, a, b, message)
}

// Kills ordinary publication observation borrowing DomainService's ACK hint.
#[test]
fn presentation_q3_ordinary_publication_generation_observes_early_store_ack_without_service_hint() {
    use crate::protocol::commands::PermitMutation;
    use crate::protocol::commands::{Ack, DeliveryMode};
    let q = Q3Channel::new();
    let a = q.sibling();
    let binding = binding_for_claim(&q.window.f, &q.window.recipient);
    let counters = || {
        q.window.f.db.query_row(
        "SELECT ordinary_passive_generation,mod_version FROM invitation_presentation_consumers WHERE binding_id=?1 AND consumer='watch'",
        [binding.as_str()], |row| Ok((row.get::<_, i64>(0)?,row.get::<_, i64>(1)?)),
    ).unwrap()
    };
    let before = counters();
    let native = q.native_version();
    let message = q.window.send(
        0,
        DeliveryMode::Ordinary,
        "publication independent of receipt hint",
    );
    assert_eq!(
        reclamation_read_full_body(&q.window.f, &message),
        "publication independent of receipt hint"
    );
    reclamation_advance_displayed(
        &q.window.f,
        &a,
        std::slice::from_ref(&message),
        "q3-direct-a-read",
    );
    let mutation = PermitMutation::Ack(Ack {
        claim: a,
        messages: vec![message.clone()],
        operation: OperationId::new("q3-direct-store-ack"),
    });
    let request = crate::store::cooperative_permit_request(&mutation).unwrap();
    let permit = q
        .window
        .f
        .store
        .issue_cooperative_permit(request, &budget())
        .unwrap();
    assert!(matches!(
        q.window
            .f
            .store
            .mutate(mutation, permit, &budget())
            .unwrap(),
        CommandResult::Acknowledged(_)
    ));
    q.finish_attention(&message);
    assert_eq!(q.receipt(&message), "acked");
    assert_eq!(q.native_version(), native);
    let progress = q
        .window
        .f
        .store
        .presentation_refresh(
            &q.window.recipient.seat,
            &binding,
            q.window.recipient.binding_generation,
            q.window.recipient.native_session.as_str(),
            &budget(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(progress.ordinary_publication_visits, 1);
    assert!(progress.ordinary_publication_probes <= 2 && progress.ordinary_publication_seeks <= 3);
    assert_eq!(
        counters(),
        (before.0 + 1, before.1 + 1),
        "an addressed ordinary publication must change the private source even after early terminal ACK"
    );
    assert_eq!(q.native_version(), native);
    assert_eq!(q.receipt(&message), "acked");
}

type Q3ReceiptSnapshot = (
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
);

fn q3_receipt_snapshot(f: &Fixture) -> Vec<Q3ReceiptSnapshot> {
    let mut query = f.db.prepare("SELECT 'physical',message_id,seat_id,state,ack_binding_id,ack_observation FROM receipts UNION ALL SELECT 'sparse',message_id,seat_id,state,ack_binding_id,ack_observation FROM receipt_state ORDER BY 1,2,3").unwrap();
    query
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

// All additional membership, publication and gaps use public canonical writers.
fn q3_many_thread_messages(
    f: &Fixture,
    a: &CallerClaim,
    mode: crate::protocol::commands::DeliveryMode,
) -> (Vec<ThreadId>, Vec<MessageId>) {
    use crate::protocol::commands::{CreateThread, Join, SendMessage};
    let sender = check_in(
        f,
        fresh_execution(
            fixture_claim(f, "x", "px", fourth(), "q3-many-thread-sender"),
            17,
        ),
        "q3-many-thread-sender-ci",
    );
    let mut threads = Vec::new();
    let mut messages = Vec::new();
    // Twelve new gaps plus the fixture's original thread exceeds the eight
    // descriptions returned by fast-forward, without growing the input batch.
    for n in 0..12 {
        let CommandResult::ThreadCreated(thread) = f
            .service
            .handle(
                Command::CreateThread(CreateThread {
                    name: None,
                    topic: format!("q3 real gap {n}"),
                    goal: "coalesce reader changes".into(),
                    operation: OperationId::new(format!("q3-many-create-{n}")),
                    claim: a.clone(),
                }),
                peer(),
                &budget(),
            )
            .unwrap()
        else {
            panic!("SETUP: canonical joined thread")
        };
        assert!(matches!(
            f.service
                .handle(
                    Command::Join(Join {
                        thread: thread.clone(),
                        operation: OperationId::new(format!("q3-many-join-{n}")),
                        claim: sender.clone(),
                    }),
                    peer(),
                    &budget()
                )
                .unwrap(),
            CommandResult::Joined(_)
        ));
        for ordinal in 0..2 {
            let body = "m".repeat(2048);
            let CommandResult::MessageSent(message) = f
                .service
                .handle(
                    Command::SendMessage(SendMessage {
                        delivery_mode: mode,
                        user_intent: None,
                        thread: thread.clone(),
                        body: body.clone(),
                        direct_recipient: None,
                        invited_recipients: vec![],
                        deadline_millis: None,
                        operation: OperationId::new(format!("q3-many-send-{n}-{ordinal}")),
                        claim: sender.clone(),
                        relays_user: false,
                    }),
                    peer(),
                    &budget(),
                )
                .unwrap()
            else {
                panic!("SETUP: canonical gap send")
            };
            assert_eq!(reclamation_read_full_body(f, &message), body);
            messages.push(message);
        }
        threads.push(thread);
    }
    (threads, messages)
}

fn q3_reader_counters(f: &Fixture, claim: &CallerClaim) -> Vec<(String, i64, i64)> {
    let mut query = f.db.prepare("SELECT consumer,ordinary_passive_generation,mod_version FROM invitation_presentation_consumers WHERE binding_id=?1 ORDER BY consumer").unwrap();
    query
        .query_map([binding_for_claim(f, claim).as_str()], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

fn q3_assert_one_reader_signal(before: &[(String, i64, i64)], after: &[(String, i64, i64)]) {
    assert_eq!(before.len(), after.len());
    for (prior, next) in before.iter().zip(after) {
        assert_eq!(
            next,
            &(prior.0.clone(), prior.1 + 1, prior.2 + 1),
            "one signal per operation/binding, independent of moved gap count"
        );
    }
}

fn q3_cursors(f: &Fixture, claim: &CallerClaim) -> Vec<(String, i64)> {
    let mut query = f
        .db
        .prepare("SELECT thread_id,seq FROM binding_cursors WHERE binding_id=?1 ORDER BY thread_id")
        .unwrap();
    query
        .query_map([binding_for_claim(f, claim).as_str()], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

#[test]
fn presentation_q3_advance_cursors_coalesces_many_real_thread_gaps() {
    let (f, a, b, first) = q3_reader_fixture(2);
    let (threads, mut messages) =
        q3_many_thread_messages(&f, &a, crate::protocol::commands::DeliveryMode::Ordinary);
    messages.insert(0, first);
    let before = q3_reader_counters(&f, &a);
    let retained = q3_preserved_reader_state(&f, &a);
    let sibling = q3_consumer_snapshot(&f, &b);
    let sibling_cursors = q3_cursors(&f, &b);
    let receipts = q3_receipt_snapshot(&f);
    reclamation_advance_displayed(&f, &a, &messages, "q3-many-display");
    q3_assert_one_reader_signal(&before, &q3_reader_counters(&f, &a));
    assert_eq!(q3_preserved_reader_state(&f, &a), retained);
    assert_eq!(q3_consumer_snapshot(&f, &b), sibling);
    assert_eq!(q3_cursors(&f, &b), sibling_cursors);
    assert_eq!(q3_receipt_snapshot(&f), receipts);
    for thread in threads {
        let head: i64 =
            f.db.query_row(
                "SELECT next_sequence-1 FROM threads WHERE id=?1",
                [thread.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            f.db.query_row::<i64, _, _>(
                "SELECT seq FROM binding_cursors WHERE binding_id=?1 AND thread_id=?2",
                params![binding_for_claim(&f, &a).as_str(), thread.as_str()],
                |row| row.get(0)
            )
            .unwrap(),
            head
        );
    }
    let after = q3_consumer_snapshot(&f, &a);
    reclamation_advance_displayed(&f, &a, &messages, "q3-many-display");
    assert_eq!(q3_consumer_snapshot(&f, &a), after);
    reclamation_advance_displayed(&f, &a, &messages, "q3-many-fresh-noop");
    assert_eq!(q3_consumer_snapshot(&f, &a), after);
}

#[test]
fn presentation_q3_complete_inbox_delivery_coalesces_many_real_thread_gaps() {
    use crate::protocol::commands::{CompleteInboxDelivery, DeliveryMode};
    let (f, a, b, _) = q3_reader_fixture(2);
    let (threads, messages) = q3_many_thread_messages(&f, &a, DeliveryMode::Lazy);
    let before = q3_reader_counters(&f, &a);
    let retained = q3_preserved_reader_state(&f, &a);
    let sibling = q3_consumer_snapshot(&f, &b);
    let sibling_cursors = q3_cursors(&f, &b);
    let receipts = q3_receipt_snapshot(&f);
    let request = CompleteInboxDelivery {
        messages: messages.clone(),
        operation: OperationId::new("q3-many-complete"),
        claim: a.clone(),
        via: None,
    };
    assert!(matches!(
        f.service
            .handle(
                Command::CompleteInboxDelivery(request.clone()),
                peer(),
                &budget()
            )
            .unwrap(),
        CommandResult::InboxDeliveryCompleted(_)
    ));
    q3_assert_one_reader_signal(&before, &q3_reader_counters(&f, &a));
    assert_eq!(q3_preserved_reader_state(&f, &a), retained);
    assert_eq!(q3_consumer_snapshot(&f, &b), sibling);
    assert_eq!(q3_cursors(&f, &b), sibling_cursors);
    assert_eq!(q3_receipt_snapshot(&f), receipts);
    for message in &messages {
        assert_eq!(
            f.db.query_row::<String, _, _>(
                "SELECT state FROM lazy_recipients WHERE message_id=?1 AND seat_id='s'",
                [message.as_str()],
                |row| row.get(0)
            )
            .unwrap(),
            "displayed"
        );
    }
    for thread in threads {
        let head: i64 =
            f.db.query_row(
                "SELECT next_sequence-1 FROM threads WHERE id=?1",
                [thread.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            f.db.query_row::<i64, _, _>(
                "SELECT seq FROM binding_cursors WHERE binding_id=?1 AND thread_id=?2",
                params![binding_for_claim(&f, &a).as_str(), thread.as_str()],
                |row| row.get(0)
            )
            .unwrap(),
            head
        );
    }
    let after = q3_consumer_snapshot(&f, &a);
    f.service
        .handle(
            Command::CompleteInboxDelivery(request.clone()),
            peer(),
            &budget(),
        )
        .unwrap();
    assert_eq!(q3_consumer_snapshot(&f, &a), after);
    f.service
        .handle(
            Command::CompleteInboxDelivery(CompleteInboxDelivery {
                operation: OperationId::new("q3-many-complete-noop"),
                ..request
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    assert_eq!(q3_consumer_snapshot(&f, &a), after);
}

#[test]
fn presentation_q3_fast_forward_coalesces_every_gap_beyond_description_cap() {
    use crate::protocol::commands::{BindingEvent, BindingEventKind, DeliveryMode};
    for consumers in [0, 2] {
        let (f, a, b, _) = q3_reader_fixture(consumers);
        let (threads, _) = q3_many_thread_messages(&f, &a, DeliveryMode::Ordinary);
        let binding = binding_for_claim(&f, &a);
        let canonical_scope = || {
            f.db.query_row(
                "SELECT generation,execution_id FROM occupant_bindings WHERE binding_id=?1",
                [binding.as_str()],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .unwrap()
        };
        let scope_before = canonical_scope();
        let current_consumers: i64 = f.db.query_row(
            "SELECT COUNT(*) FROM invitation_presentation_consumers c JOIN occupant_bindings b ON b.binding_id=c.binding_id JOIN seats s ON s.id=b.seat_id WHERE c.binding_id=?1 AND c.instance_id=s.instance_id AND ((c.generation=b.generation AND c.execution=b.execution_id) OR (c.mod_generation=b.generation AND c.mod_execution=b.execution_id))",
            [binding.as_str()], |row| row.get(0),
        ).unwrap();
        assert_eq!(
            current_consumers,
            i64::try_from(consumers).unwrap(),
            "SETUP: exact current consumer scope"
        );
        // The send allocator also reserves positions for deferred warnings.
        // Capture published heads before reconnect; allocated next_sequence-1
        // is not a message and cannot be the expected fast-forward cursor.
        let heads = threads
            .iter()
            .cloned()
            .chain(std::iter::once(ThreadId::new("t")))
            .map(|thread| {
                let head: i64 =
                    f.db.query_row(
                        "SELECT COALESCE(MAX(sequence),0) FROM messages WHERE thread_id=?1",
                        [thread.as_str()],
                        |row| row.get(0),
                    )
                    .unwrap();
                (thread, head)
            })
            .collect::<Vec<_>>();
        assert_eq!(heads.len(), 13);
        let before = q3_reader_counters(&f, &a);
        let retained = q3_preserved_reader_state(&f, &a);
        let sibling = q3_consumer_snapshot(&f, &b);
        let sibling_cursors = q3_cursors(&f, &b);
        let receipts = q3_receipt_snapshot(&f);
        f.service
            .handle(
                Command::BindingEvent(BindingEvent {
                    claim: a.clone(),
                    kind: BindingEventKind::SessionEnd,
                    child_session: None,
                    operation: OperationId::new("q3-many-end"),
                }),
                peer(),
                &budget(),
            )
            .unwrap();
        let CommandResult::CheckedIn(result) = f
            .service
            .handle(
                Command::CheckIn(CheckIn {
                    mode: CheckInMode::Lifecycle {
                        expected_binding_generation: a.binding_generation,
                    },
                    claim: a.clone(),
                    operation: OperationId::new("q3-many-resume"),
                    lineage_parent: None,
                }),
                peer(),
                &budget(),
            )
            .unwrap()
        else {
            panic!("SETUP: canonical lifecycle resume")
        };
        let fast_forward = result
            .fast_forward
            .expect("SETUP: actual backlog exceeds one ordinary page");
        assert_eq!(fast_forward.threads.len(), 8);
        assert_eq!(
            fast_forward.more, 5,
            "all thirteen joined gaps move; only eight descriptions fit"
        );
        assert_eq!(
            canonical_scope(),
            scope_before,
            "lifecycle reconnect keeps the canonical generation/execution scope"
        );
        assert_eq!(result.context.binding_generation, a.binding_generation);
        assert_eq!(result.context.execution, a.execution);
        assert_eq!(binding_for_claim(&f, &result.context), binding);
        q3_assert_one_reader_signal(&before, &q3_reader_counters(&f, &a));
        assert_eq!(q3_preserved_reader_state(&f, &a), retained);
        assert_eq!(q3_consumer_snapshot(&f, &b), sibling);
        assert_eq!(q3_cursors(&f, &b), sibling_cursors);
        assert_eq!(q3_receipt_snapshot(&f), receipts);
        for (thread, head) in heads {
            assert_eq!(
                f.db.query_row::<i64, _, _>(
                    "SELECT seq FROM binding_cursors WHERE binding_id=?1 AND thread_id=?2",
                    params![binding_for_claim(&f, &a).as_str(), thread.as_str()],
                    |row| row.get(0)
                )
                .unwrap(),
                head
            );
        }
    }
}

// Kills invalidation that clears real retained body/carry/outbox bytes or full
// logical stamps while empty/default consumer rows still appear unchanged.
#[test]
fn presentation_q3_reader_change_preserves_nonempty_progress_outbox_and_offers() {
    use crate::protocol::{
        commands::{DeliveryMode, SendMessage},
        delivery::DeliveryPullReplyV2,
    };
    let (f, a, b, original) = q3_reader_fixture(2);
    let sender = check_in(
        &f,
        fresh_execution(
            fixture_claim(&f, "x", "px", fourth(), "q3-retained-sender"),
            18,
        ),
        "q3-retained-sender-ci",
    );
    let send = |n, body: String| {
        let CommandResult::MessageSent(id) = f
            .service
            .handle(
                Command::SendMessage(SendMessage {
                    delivery_mode: DeliveryMode::Ordinary,
                    user_intent: None,
                    thread: ThreadId::new("t"),
                    body,
                    direct_recipient: None,
                    invited_recipients: vec![],
                    deadline_millis: None,
                    operation: OperationId::new(format!("q3-retained-send-{n}")),
                    claim: sender.clone(),
                    relays_user: false,
                }),
                peer(),
                &budget(),
            )
            .unwrap()
        else {
            panic!("SETUP: retained body producer")
        };
        id
    };
    let first = send(0, "a".repeat(58_000));
    let second = send(1, "\0".repeat(58_000));
    let DeliveryPullReplyV2::Page(first_page) = presentation_test_pull(&f, &a) else {
        panic!("SETUP: real ordinary page")
    };
    assert!(first_page.offered.contains(&original));
    assert!(first_page.truncated.contains(&first));
    canonical_lazy_invitation_backlog(&f, &a, &sender, 1, "q3-retained-outbox");
    let DeliveryPullReplyV2::Page(outbox_page) = presentation_test_pull(&f, &a) else {
        panic!("SETUP: real invitation outbox")
    };
    assert!(
        outbox_page.presentation_token.is_some(),
        "SETUP: genuine outstanding presentation token"
    );
    assert!(!presentation_invitations(&outbox_page).is_empty());
    let binding = binding_for_claim(&f, &a);
    let nonempty: (Option<String>,String,Option<String>,String) = f.db.query_row(
        "SELECT ordinary_cursor,ordinary_partial,outbox_token,outbox_descriptors FROM invitation_presentation_consumers WHERE binding_id=?1 AND consumer='hook'",
        [binding.as_str()], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
    ).unwrap();
    assert!(
        nonempty.0.is_some(),
        "SETUP: source-created body continuation remains live"
    );
    assert!(
        nonempty.2.is_some() && nonempty.3 != "[]",
        "SETUP: nonempty outbox/token"
    );
    let offers = f.state.lock().get(&binding).unwrap().offers.clone();
    assert!(offers.contains_key(&original) && offers.contains_key(&first));
    assert!(
        offers
            .values()
            .all(|stamp| stamp.count > 0 && stamp.first_at.0 > 0),
        "SETUP: actual nontrivial full offer stamps"
    );
    // Reading first never claims that the second captured body was displayed.
    assert_eq!(reclamation_read_full_body(&f, &first), "a".repeat(58_000));
    assert_ne!(first, second);
    let retained = q3_preserved_reader_state(&f, &a);
    let sibling = q3_consumer_snapshot(&f, &b);
    let receipts = q3_receipt_snapshot(&f);
    let counters = q3_reader_counters(&f, &a);
    reclamation_advance_displayed(
        &f,
        &a,
        &[original.clone(), first.clone()],
        "q3-retained-read",
    );
    q3_assert_one_reader_signal(&counters, &q3_reader_counters(&f, &a));
    assert_eq!(q3_preserved_reader_state(&f, &a), retained);
    assert_eq!(q3_consumer_snapshot(&f, &b), sibling);
    assert_eq!(q3_receipt_snapshot(&f), receipts);
    assert_eq!(f.state.lock().get(&binding).unwrap().offers, offers);
    let after = q3_consumer_snapshot(&f, &a);
    reclamation_advance_displayed(
        &f,
        &a,
        &[original.clone(), first.clone()],
        "q3-retained-read",
    );
    assert_eq!(q3_consumer_snapshot(&f, &a), after);
    reclamation_advance_displayed(&f, &a, &[original, first], "q3-retained-noop");
    assert_eq!(q3_consumer_snapshot(&f, &a), after);
    assert_eq!(f.state.lock().get(&binding).unwrap().offers, offers);
}

// Kills seat-wide invalidation, per-message bumps, skipped refresh-only rows,
// stale-header promotion, and repeat operations inventing a new private signal.
#[test]
fn presentation_q3_reader_change_is_binding_local_coalesced_and_noop_stable() {
    for consumers in 0..=2 {
        let (f, a, b, message) = q3_reader_fixture(consumers);
        let a_binding = binding_for_claim(&f, &a);
        assert_eq!(q3_consumer_snapshot(&f, &a).len(), consumers);
        assert_eq!(q3_consumer_snapshot(&f, &b).len(), consumers);
        let receipts = q3_receipt_snapshot(&f);
        let b_before = q3_consumer_snapshot(&f, &b);
        let cursor_b = reclamation_cursor(&f, &b);
        let preserved = q3_preserved_reader_state(&f, &a);
        let offers = f
            .state
            .lock()
            .get(&a_binding)
            .map(|entry| entry.offers.clone());
        let mut counters = f.db.prepare("SELECT consumer,ordinary_passive_generation,mod_version FROM invitation_presentation_consumers WHERE binding_id=?1 ORDER BY consumer").unwrap();
        let before = counters
            .query_map([a_binding.as_str()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        drop(counters);
        let native = wake_attention(&f);
        let authority = || {
            f.db.query_row(
            "SELECT json_array((SELECT count(*) FROM receipt_state),(SELECT count(*) FROM receipts),(SELECT count(*) FROM invitations WHERE state='accepted'),(SELECT count(*) FROM lazy_recipients WHERE state='displayed'))",
            [], |row| row.get::<_, String>(0),
        ).unwrap()
        };
        let authority_before = authority();
        reclamation_advance_displayed(&f, &a, std::slice::from_ref(&message), "q3-reader-change");
        assert!(reclamation_cursor(&f, &a) > cursor_b);
        assert_eq!(reclamation_cursor(&f, &b), cursor_b);
        assert_eq!(q3_consumer_snapshot(&f, &b), b_before);
        assert_eq!(
            q3_preserved_reader_state(&f, &a),
            preserved,
            "reader invalidation preserves every token, captured tuple, carry and partial byte"
        );
        assert_eq!(
            f.state
                .lock()
                .get(&a_binding)
                .map(|entry| entry.offers.clone()),
            offers
        );
        for (consumer, passive, version) in before {
            assert_eq!(f.db.query_row(
                "SELECT ordinary_passive_generation,mod_version FROM invitation_presentation_consumers WHERE binding_id=?1 AND consumer=?2",
                params![a_binding.as_str(), consumer], |row| Ok((row.get::<_, i64>(0)?,row.get::<_, i64>(1)?)),
            ).unwrap(), (passive+1, version+1), "one coalesced change per existing current consumer");
        }
        let after = q3_consumer_snapshot(&f, &a);
        reclamation_advance_displayed(&f, &a, std::slice::from_ref(&message), "q3-reader-change");
        assert_eq!(
            q3_consumer_snapshot(&f, &a),
            after,
            "retained replay changes nothing"
        );
        reclamation_advance_displayed(&f, &a, std::slice::from_ref(&message), "q3-reader-noop");
        assert_eq!(
            q3_consumer_snapshot(&f, &a),
            after,
            "fresh no-op creates no signal"
        );
        assert_eq!(wake_attention(&f), native);
        assert_eq!(q3_receipt_snapshot(&f), receipts);
        assert_eq!(
            authority(),
            authority_before,
            "reader signals grant no ACK, acceptance or Lazy completion authority"
        );
    }
}

// Kills invalidation that repairs old scope headers, ignores a current
// refresh-only owner, or treats a foreign instance as this binding's state.
fn q3_foreign_consumer(f: &Fixture) -> CallerClaim {
    // A foreign consumer must satisfy its composite canonical binding FK;
    // changing an own row's instance would manufacture impossible state.
    f.db.execute("INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('foreign-instance',0,'b',1,3)", []).unwrap();
    f.db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('foreign-seat','foreign-instance','resolved','native','foreign-p',1,0,0)", []).unwrap();
    f.db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,occupancy,ui_state,top_level_occupant,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('foreign-instance','foreign-p','b',1,0,1,'fresh','unknown','unknown',0,0,'foreign-term','inc','coherent_enumeration',1)", []).unwrap();
    let store: Arc<dyn StorePort> = Arc::new(
        SqliteStore::new(
            StoreContext::new(f.directory.join("store.db"), f.clock.clone()),
            "foreign-instance",
            StoreSettings::default(),
        )
        .unwrap(),
    );
    let service = DomainService::new("foreign-instance".into(), store.clone(), f.clock.clone())
        .with_cooperative_owner(501, Arc::new(FairWriter::new(32)));
    let mut foreign = fresh_execution(
        claim("foreign-seat", "foreign-p", fourth(), "foreign-session", 1),
        77,
    );
    foreign.instance = "foreign-instance".into();
    foreign.liveness.as_mut().unwrap().pid = 31337;
    let CommandResult::CheckedIn(result) = service
        .handle(
            Command::CheckIn(CheckIn {
                mode: CheckInMode::Lifecycle {
                    expected_binding_generation: foreign.binding_generation,
                },
                claim: foreign,
                operation: OperationId::new("q3-foreign-ci"),
                lineage_parent: None,
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("SETUP: foreign canonical binding")
    };
    let foreign = result.context;
    store
        .presentation_refresh(
            &foreign.seat,
            &binding_for_claim(f, &foreign),
            foreign.binding_generation,
            foreign.native_session.as_str(),
            &budget(),
        )
        .unwrap()
        .expect("SETUP: foreign canonical consumer");
    assert_eq!(q3_consumer_snapshot(f, &foreign).len(), 1);
    foreign
}

#[test]
fn presentation_q3_reader_change_respects_current_primary_or_refresh_scope() {
    for scope in ["primary", "refresh_only", "both_stale", "foreign_instance"] {
        let (f, a, b, message) = q3_reader_fixture(2);
        let binding = binding_for_claim(&f, &a);
        let foreign = (scope == "foreign_instance").then(|| q3_foreign_consumer(&f));
        let foreign_before = foreign
            .as_ref()
            .map(|claim| q3_consumer_snapshot(&f, claim));
        match scope {
            "primary" => {
                f.db.execute(
                "UPDATE invitation_presentation_consumers SET mod_generation=mod_generation+1 WHERE binding_id=?1 AND consumer='watch'",
                [binding.as_str()],
            ).unwrap();
            }
            "refresh_only" => {
                f.db.execute(
                "UPDATE invitation_presentation_consumers SET generation=generation+1 WHERE binding_id=?1 AND consumer='watch'",
                [binding.as_str()],
            ).unwrap();
            }
            "both_stale" => {
                f.db.execute(
                "UPDATE invitation_presentation_consumers SET generation=generation+1,mod_generation=mod_generation+1 WHERE binding_id=?1 AND consumer='watch'",
                [binding.as_str()],
            ).unwrap();
            }
            "foreign_instance" => {}
            _ => unreachable!(),
        }
        let counter = || {
            f.db.query_row(
            "SELECT ordinary_passive_generation,mod_version FROM invitation_presentation_consumers WHERE binding_id=?1 AND consumer='watch'",
            [binding.as_str()], |row| Ok((row.get::<_, i64>(0)?,row.get::<_, i64>(1)?)),
        ).unwrap()
        };
        let before = counter();
        let preserved = q3_preserved_reader_state(&f, &a);
        let sibling = q3_consumer_snapshot(&f, &b);
        reclamation_advance_displayed(&f, &a, std::slice::from_ref(&message), "q3-scoped-reader");
        let delta = i64::from(matches!(
            scope,
            "primary" | "refresh_only" | "foreign_instance"
        ));
        assert_eq!(counter(), (before.0 + delta, before.1 + delta), "{scope}");
        assert_eq!(q3_preserved_reader_state(&f, &a), preserved);
        assert_eq!(q3_consumer_snapshot(&f, &b), sibling);
        assert_eq!(
            foreign
                .as_ref()
                .map(|claim| q3_consumer_snapshot(&f, claim)),
            foreign_before,
            "own current rows change once; a genuine foreign-instance consumer stays untouched"
        );
    }
}

// Kills private checked failures committing the canonical cursor first, and a
// later SQL failure retaining an earlier consumer's tentative update.
#[test]
fn presentation_q3_reader_change_failure_rolls_back_cursor_and_private_state() {
    for failure in ["ordinary_passive_generation", "mod_version", "later_write"] {
        let (f, a, b, message) = q3_reader_fixture(2);
        let binding = binding_for_claim(&f, &a);
        if failure == "later_write" {
            f.db.execute_batch("CREATE TRIGGER q3_late_reader_failure BEFORE UPDATE OF mod_version ON invitation_presentation_consumers WHEN NEW.consumer='watch' BEGIN SELECT RAISE(ABORT,'q3 late reader failure'); END;").unwrap();
        } else {
            f.db.execute(&format!("UPDATE invitation_presentation_consumers SET {failure}=?1 WHERE binding_id=?2 AND consumer='watch'"),
                params![i64::MAX, binding.as_str()]).unwrap();
        }
        let before = q3_consumer_snapshot(&f, &a);
        let sibling = q3_consumer_snapshot(&f, &b);
        let cursor = reclamation_cursor(&f, &a);
        let native = wake_attention(&f);
        let result = f.service.handle(
            Command::AdvanceCursors(crate::protocol::commands::AdvanceCursors {
                claim: a.clone(),
                operation: OperationId::new("q3-reader-rollback"),
                displayed: vec![message],
            }),
            peer(),
            &budget(),
        );
        let error = result.expect_err("applicable checked failure must abort");
        if failure == "later_write" {
            assert!(
                error.detail.contains("q3 late reader failure"),
                "SQL abort, not admission failure: {error:?}"
            );
        } else {
            assert_eq!(
                error.code,
                crate::protocol::results::ErrorCode::SequenceExhausted,
                "checked exhaustion, not admission failure"
            );
        }
        assert_eq!(reclamation_cursor(&f, &a), cursor);
        assert_eq!(q3_consumer_snapshot(&f, &a), before);
        assert_eq!(q3_consumer_snapshot(&f, &b), sibling);
        assert_eq!(wake_attention(&f), native);
    }
}

struct TestClock(AtomicI64);
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(self.0.load(Ordering::SeqCst))
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(100)
    }
}

struct Fixture {
    service: DomainService,
    store: Arc<dyn StorePort>,
    /// The offered ledger the service and (in production) the wake lane share.
    state: Arc<DeliveryPullState>,
    clock: Arc<TestClock>,
    db: Connection,
    directory: std::path::PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    }
}
fn peer() -> PeerIdentity {
    PeerIdentity::from_kernel(501)
}
fn fourth() -> Harness {
    Harness::Agent(
        crate::harness::registry::builtins()
            .agent("synthetic_fourth")
            .unwrap(),
    )
}

/// Seats `s` (synthetic_fourth), `x` (Codex), `c` (Claude); thread `t` with all
/// three joined. `s` has pending ordinary messages `m1`..`m<ordinary>`.
fn fixture(ordinary: usize) -> Fixture {
    use std::os::unix::fs::DirBuilderExt;
    let directory = std::env::temp_dir().join(format!("delivery-pull-{}", uuid::Uuid::new_v4()));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory)
        .unwrap();
    let clock = Arc::new(TestClock(AtomicI64::new(1_000_000)));
    let shared: Arc<dyn Clock> = clock.clone();
    let context = StoreContext::new(directory.join("store.db"), shared.clone());
    let db = context.open_writer().unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'b',1,3)",
        [],
    )
    .unwrap();
    for (seat, target) in [("s", "p"), ("x", "px"), ("c", "pc")] {
        db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES (?1,'i','resolved','native',?2,1,0,0)", [seat, target]).unwrap();
        db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,occupancy,ui_state,top_level_occupant,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i',?1,'b',1,0,1,'fresh','unknown','unknown',0,0,'term-'||?1,'inc','coherent_enumeration',1)", [target]).unwrap();
    }
    db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence) VALUES ('t','i','topic','goal',0,0,1)", []).unwrap();
    for seat in ["s", "x", "c"] {
        db.execute(
            "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t',?1,'joined')",
            [seat],
        )
        .unwrap();
        db.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t',?1,1,1)", [seat]).unwrap();
    }
    let store: Arc<dyn StorePort> =
        Arc::new(SqliteStore::new(context, "i", StoreSettings::default()).unwrap());
    let state = Arc::new(DeliveryPullState::default());
    let service = DomainService::new("i".into(), Arc::clone(&store), shared)
        .with_cooperative_owner(501, Arc::new(FairWriter::new(32)))
        .with_delivery_pull(Arc::clone(&state));
    let f = Fixture {
        service,
        store,
        state,
        clock,
        db,
        directory,
    };
    for n in 1..=ordinary {
        ordinary_message(&f, &format!("m{n}"), "hello");
    }
    f
}

/// A daemon restart: a fresh service (and so a fresh process-local ledger) over the same store.
fn restart(f: &mut Fixture) {
    let shared: Arc<dyn Clock> = f.clock.clone();
    f.state = Arc::new(DeliveryPullState::default());
    f.service = DomainService::new("i".into(), Arc::clone(&f.store), shared)
        .with_cooperative_owner(501, Arc::new(FairWriter::new(32)))
        .with_delivery_pull(Arc::clone(&f.state));
}
/// The seat's wake attention counter (the store bumps it for every new pending source; 0 without a row).
fn wake_attention(f: &Fixture) -> u64 {
    use rusqlite::OptionalExtension;
    f.db.query_row(
        "SELECT attention_version FROM wake_work WHERE seat_id='s'",
        [],
        |r| r.get::<_, i64>(0),
    )
    .optional()
    .unwrap()
    .unwrap_or(0) as u64
}
/// What the store does for a new pending source: move the seat's wake attention counter (the fixture inserts
/// messages directly, so it must be moved by hand).
fn bump_wake_attention(f: &Fixture) {
    f.db.execute(
        "INSERT INTO wake_work(seat_id,attention_version) VALUES ('s',1) ON CONFLICT(seat_id) DO UPDATE SET attention_version=attention_version+1",
        [],
    )
    .unwrap();
}

/// Next free message sequence (also used for the decision sequence).
fn next_sequence(db: &Connection) -> i64 {
    db.query_row("SELECT next_sequence FROM threads WHERE id='t'", [], |r| {
        r.get(0)
    })
    .unwrap()
}
fn bump(db: &Connection, sequence: i64) {
    db.execute(
        "UPDATE threads SET next_sequence=?1 WHERE id='t'",
        [sequence + 1],
    )
    .unwrap();
    db.execute(
        "UPDATE host_instances SET decision_seq=?1 WHERE id='i'",
        [sequence + 3],
    )
    .unwrap();
}
fn ordinary_message(f: &Fixture, id: &str, body: &str) {
    let sequence = next_sequence(&f.db);
    f.db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES (?1,'i','t',?2,'ordinary',?3,0,?4)", params![id, sequence, body, sequence + 2]).unwrap();
    f.db.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES (?1,'t','s','pending',300000)", [id]).unwrap();
    bump(&f.db, sequence);
}
/// A published lazy message addressed to `s` (state `pending`).
fn lazy_message(f: &Fixture, id: &str, body: &str) {
    let sequence = next_sequence(&f.db);
    let prep = format!("prep-{}", id.trim_start_matches("msg-"));
    f.db.execute(
        "INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status,delivery_mode) VALUES (?1,'i','actor',?1,zeroblob(32),'t',0,0,0,0,0,0,1,'building','lazy')",
        [&prep],
    )
    .unwrap();
    f.db.execute("INSERT INTO lazy_recipients(preparation_id,message_id,thread_id,seat_id) VALUES (?1,?2,'t','s')", params![prep, id]).unwrap();
    f.db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq,delivery_mode) VALUES (?1,'i','t',?2,'ordinary',?3,0,?4,'lazy')", params![id, sequence, body, sequence + 2]).unwrap();
    f.db.execute("INSERT INTO send_manifests(instance_id,preparation_id,message_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('i',?1,?2,'t',?3,0,?4,0,1,0)", params![prep, id, sequence + 2, sequence]).unwrap();
    bump(&f.db, sequence);
}

fn claim(
    seat: &str,
    target: &str,
    harness: Harness,
    session: &str,
    generation: u64,
) -> CallerClaim {
    CallerClaim {
        instance: "i".into(),
        seat: SeatId::new(seat),
        binding_generation: generation,
        role: CallerRole::TopLevel,
        harness,
        native_session: NativeSessionId::new(session),
        execution: ExecutionId::new("00000000-0000-4000-8000-000000000001"),
        target: HostTargetId::new(target),
        agent_id: None,
        liveness: Some(crate::protocol::authority::LivenessFingerprint {
            // One harness process per pane: a seat's sessions share it.
            pid: 1000 + seat.bytes().map(u32::from).sum::<u32>(),
            start: 1,
        }),
    }
}
/// Initial positive fixture claim uses the seat's actual mapping generation.
/// Raw `claim` and `check_in` keep caller-supplied generations for negative controls.
fn fixture_claim(
    f: &Fixture,
    seat: &str,
    target: &str,
    harness: Harness,
    session: &str,
) -> CallerClaim {
    let generation: i64 = f.db.query_row(
        "SELECT generation FROM seats WHERE id=?1 AND instance_id='i' AND target_id=?2 AND state='resolved'",
        params![seat, target],
        |row| row.get(0),
    ).unwrap();
    claim(
        seat,
        target,
        harness,
        session,
        u64::try_from(generation).unwrap(),
    )
}
fn fresh_execution(mut claim: CallerClaim, n: u8) -> CallerClaim {
    claim.execution = ExecutionId::new(format!("00000000-0000-4000-8000-0000000000{n:02}"));
    claim
}
/// Lifecycle check-in; returns the claim the daemon hands back.
fn check_in(f: &Fixture, claim: CallerClaim, operation: &str) -> CallerClaim {
    let command = Command::CheckIn(CheckIn {
        mode: CheckInMode::Lifecycle {
            expected_binding_generation: claim.binding_generation,
        },
        claim,
        operation: OperationId::new(operation),
        lineage_parent: None,
    });
    match f.service.handle(command, peer(), &budget()).unwrap() {
        CommandResult::CheckedIn(result) => result.context,
        other => panic!("unexpected {other:?}"),
    }
}
/// The pull seat `s`, checked in as `synthetic_fourth` in native session `n1`.
fn pull_seat(f: &Fixture) -> CallerClaim {
    check_in(f, claim("s", "p", fourth(), "n1", 1), "ci-1")
}
/// Rotate `s` to a new native session (a `/clear`) or the same one (a resume).
fn rotate(f: &Fixture, previous: &CallerClaim, session: &str, n: u8) -> CallerClaim {
    check_in(
        f,
        fresh_execution(
            claim("s", "p", fourth(), session, previous.binding_generation),
            n,
        ),
        &format!("ci-{n}"),
    )
}

fn pull(f: &Fixture, claim: &CallerClaim, seen: Option<u64>) -> DeliveryPullReply {
    let command = Command::DeliveryPull(DeliveryPull {
        claim: claim.clone(),
        seen_attention_version: seen,
        marker_prefix: vec![],
    });
    match f.service.handle(command, peer(), &budget()).unwrap() {
        CommandResult::DeliveryPulled(reply) => reply,
        other => panic!("unexpected {other:?}"),
    }
}
fn page(reply: DeliveryPullReply) -> crate::protocol::delivery::DeliveryPage {
    match reply {
        DeliveryPullReply::Page(page) => page,
        other => panic!("expected a page, got {other:?}"),
    }
}
fn refusal(reply: DeliveryPullReply) -> Refusal {
    match reply {
        DeliveryPullReply::Refused { reason } => reason,
        other => panic!("expected a refusal, got {other:?}"),
    }
}
fn ids(list: &[&str]) -> Vec<MessageId> {
    list.iter().map(|id| MessageId::new(*id)).collect()
}
fn confirm(
    f: &Fixture,
    claim: &CallerClaim,
    list: &[&str],
    operation: &str,
) -> Vec<(String, Outcome, Option<Reason>)> {
    let command = Command::DeliveryConfirm(DeliveryConfirm {
        claim: claim.clone(),
        ids: ids(list),
        operation: OperationId::new(operation),
    });
    match f.service.handle(command, peer(), &budget()).unwrap() {
        CommandResult::DeliveryConfirmed(report) => report
            .results
            .into_iter()
            .map(|item| (item.id.as_str().to_owned(), item.result, item.reason))
            .collect(),
        other => panic!("unexpected {other:?}"),
    }
}
fn receipt_state(f: &Fixture, id: &str) -> String {
    f.db.query_row(
        "SELECT state FROM receipts WHERE message_id=?1 AND seat_id='s'",
        [id],
        |r| r.get(0),
    )
    .unwrap()
}
fn lazy_state(f: &Fixture, id: &str) -> String {
    f.db.query_row(
        "SELECT state FROM lazy_recipients WHERE message_id=?1 AND seat_id='s'",
        [id],
        |r| r.get(0),
    )
    .unwrap()
}
fn now_utc(f: &Fixture) -> UtcMillis {
    UtcMillis(f.clock.0.load(Ordering::SeqCst))
}
fn advance(f: &Fixture, ms: i64) {
    f.clock.0.fetch_add(ms, Ordering::SeqCst);
}

#[test]
fn pull_returns_a_framed_page_whose_ordinary_ids_settle_on_confirm() {
    let f = fixture(2);
    let context = pull_seat(&f);
    let page = page(pull(&f, &context, None));
    assert!(page.text.starts_with(HEADER), "{}", page.text);
    assert!(!page.text.starts_with('/'));
    assert!(page.text.contains("[herdr-threads] message m1 in "));
    assert_eq!(page.offered, ids(&["m1", "m2"]));
    assert!(page.truncated.is_empty());
    assert!(!page.more);
    assert_eq!(receipt_state(&f, "m1"), "pending", "a pull settles nothing");

    let report = confirm(&f, &context, &["m1", "m2"], "op-1");
    assert_eq!(
        report,
        vec![
            ("m1".to_owned(), Outcome::Settled, None),
            ("m2".to_owned(), Outcome::Settled, None)
        ]
    );
    assert_eq!(receipt_state(&f, "m1"), "acked");
    let observation: String =
        f.db.query_row(
            "SELECT ack_observation FROM receipts WHERE message_id='m1' AND seat_id='s'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let observation: serde_json::Value = serde_json::from_str(&observation).unwrap();
    assert_eq!(
        observation["action_provenance"],
        "cooperative_hook_delivery"
    );
    assert_eq!(observation["via"], "transcript");
    assert_eq!(observation["provenance"], "cooperative_top_level");
}

#[test]
fn a_lazy_row_is_offered_framed_and_completes_on_confirm() {
    let f = fixture(0);
    // Session-keyed bindings: a fresh binding's lazy cursor starts at the thread head (first
    // contact), so the lazy row is published after the session checks in.
    let context = pull_seat(&f);
    lazy_message(&f, "msg-lz1", "passive text");
    let page = page(pull(&f, &context, None));
    assert!(
        page.text.contains("[herdr-threads] lazy msg-lz1 in "),
        "{}",
        page.text
    );
    assert_eq!(page.offered, ids(&["msg-lz1"]));
    assert_eq!(lazy_state(&f, "msg-lz1"), "pending");
    let report = confirm(&f, &context, &["msg-lz1"], "op-1");
    assert_eq!(report, vec![("msg-lz1".to_owned(), Outcome::Settled, None)]);
    assert_eq!(lazy_state(&f, "msg-lz1"), "displayed");
    // A repeat confirm is answered from the store: already settled.
    let again = confirm(&f, &context, &["msg-lz1"], "op-2");
    assert_eq!(
        again[0].1,
        Outcome::NotOffered,
        "settled ids left the ledger"
    );
}

#[test]
fn a_second_pull_with_the_returned_version_is_unchanged() {
    let f = fixture(2);
    let context = pull_seat(&f);
    let first = page(pull(&f, &context, None));
    let second = pull(&f, &context, Some(first.attention_version));
    assert_eq!(
        second,
        DeliveryPullReply::Unchanged {
            attention_version: first.attention_version
        }
    );
    // Without the version the unconfirmed items are not shown again either
    // (the 60 s re-offer bound), but the version is not trusted blindly: a
    // stale version gets the same answer only once nothing is due.
    let stale = pull(&f, &context, Some(first.attention_version ^ 1));
    assert_eq!(
        stale,
        DeliveryPullReply::Unchanged {
            attention_version: first.attention_version
        },
        "offered items are skipped until due"
    );
    // New pending work moves the version.
    ordinary_message(&f, "m3", "later");
    let third = page(pull(&f, &context, Some(first.attention_version)));
    assert_ne!(third.attention_version, first.attention_version);
    assert_eq!(third.offered, ids(&["m3"]));
}

#[test]
fn more_than_a_page_of_blocks_sets_more_and_the_next_pull_continues() {
    let f = fixture(40);
    let context = pull_seat(&f);
    let first = page(pull(&f, &context, None));
    assert!(first.more);
    assert_eq!(first.offered.len(), 32);
    assert_eq!(first.offered[0], MessageId::new("m1"));
    // Same version, but `more_pending` forbids the Unchanged short-circuit.
    let second = page(pull(&f, &context, Some(first.attention_version)));
    assert!(!second.more);
    assert_eq!(
        second.offered.len(),
        8,
        "the remaining blocks, oldest first"
    );
    assert_eq!(second.offered[0], MessageId::new("m33"));
    let done = pull(&f, &context, Some(second.attention_version));
    assert!(matches!(done, DeliveryPullReply::Unchanged { .. }));
}

#[test]
fn an_unconfirmed_id_is_reoffered_after_sixty_seconds_at_most_three_times() {
    let f = fixture(1);
    let context = pull_seat(&f);
    let first = page(pull(&f, &context, None));
    assert_eq!(first.offered, ids(&["m1"]));
    let seen = Some(first.attention_version);

    advance(&f, REOFFER_AFTER_MS - 1);
    assert!(matches!(
        pull(&f, &context, seen),
        DeliveryPullReply::Unchanged { .. }
    ));
    advance(&f, 1);
    let second = page(pull(&f, &context, seen));
    assert_eq!(second.offered, ids(&["m1"]), "due after 60 s");
    // Not again straight away: the 60 s runs from the last showing.
    assert!(matches!(
        pull(&f, &context, seen),
        DeliveryPullReply::Unchanged { .. }
    ));
    advance(&f, REOFFER_AFTER_MS);
    let third = page(pull(&f, &context, seen));
    assert_eq!(third.offered, ids(&["m1"]));
    // Parked after the third showing.
    advance(&f, 10 * REOFFER_AFTER_MS);
    assert!(matches!(
        pull(&f, &context, seen),
        DeliveryPullReply::Unchanged { .. }
    ));
    assert_eq!(receipt_state(&f, "m1"), "pending", "it waits for inbox");
}

#[test]
fn a_rotated_generation_discards_offers() {
    let f = fixture(1);
    let first = pull_seat(&f);
    page(pull(&f, &first, None));
    // /clear: a new native session is a new binding of the seat (session-keyed bindings; the
    // binding generation is the seat's and does not move).
    let second = rotate(&f, &first, "n2", 2);
    assert_ne!(second.native_session, first.native_session);
    // The new generation never saw the offer.
    assert_eq!(
        confirm(&f, &second, &["m1"], "op-1"),
        vec![("m1".to_owned(), Outcome::NotOffered, None)]
    );
    assert_eq!(receipt_state(&f, "m1"), "pending");
    // The old generation's claim is stale (a different native session).
    assert_eq!(
        confirm(&f, &first, &["m1"], "op-2"),
        vec![("m1".to_owned(), Outcome::StaleGeneration, None)]
    );
    // The pending item is offered afresh to the new generation.
    assert_eq!(page(pull(&f, &second, None)).offered, ids(&["m1"]));
}

// Session-keyed bindings: a resume re-registers the same session's binding in place (same binding
// generation), so its offered set survives and the resumed session confirms what it was shown.
#[test]
fn a_resume_keeps_the_binding_and_its_offers() {
    let f = fixture(1);
    let first = pull_seat(&f);
    page(pull(&f, &first, None));
    let resumed = rotate(&f, &first, "n1", 2);
    assert_eq!(resumed.binding_generation, first.binding_generation);
    assert_eq!(
        confirm(&f, &resumed, &["m1"], "op-1"),
        vec![("m1".to_owned(), Outcome::Settled, None)]
    );
}

#[test]
fn a_truncated_id_is_listed_truncated_and_refused_on_confirm() {
    let f = fixture(0);
    ordinary_message(&f, "big", &"x".repeat(WATCH_BODY_LIMIT_BYTES + 100));
    ordinary_message(&f, "small", "short");
    let context = pull_seat(&f);
    let page = page(pull(&f, &context, None));
    assert_eq!(page.truncated, ids(&["big"]));
    assert_eq!(page.offered, ids(&["small"]));
    assert!(page.text.contains("herdr-threads"), "{}", page.text);
    let report = confirm(&f, &context, &["big", "small"], "op-1");
    assert_eq!(
        report,
        vec![
            (
                "big".to_owned(),
                Outcome::RefusedTerminal,
                Some(Reason::Truncated)
            ),
            ("small".to_owned(), Outcome::Settled, None),
        ]
    );
    assert_eq!(receipt_state(&f, "big"), "pending");
}

#[test]
fn a_plain_call_enrolled_hook_pull_binding_is_served() {
    let f = fixture(1);
    // A plain (non-lifecycle) check-in enrolls the session: its binding keeps the honest
    // `cooperative_env_session` provenance and is still served (role + liveness + harness).
    let command = Command::CheckIn(CheckIn {
        mode: CheckInMode::Current,
        claim: claim("s", "p", fourth(), "n1", 1),
        operation: OperationId::new("plain-1"),
        lineage_parent: None,
    });
    let context = match f.service.handle(command, peer(), &budget()).unwrap() {
        CommandResult::CheckedIn(result) => result.context,
        other => panic!("unexpected {other:?}"),
    };
    let provenance: String = f
        .db
        .query_row(
            "SELECT observation_provenance FROM occupant_bindings WHERE seat_id='s' AND native_session='n1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(provenance, "cooperative_env_session");
    let reply = pull(&f, &context, None);
    assert!(
        matches!(reply, DeliveryPullReply::Page(_)),
        "expected a page, got {reply:?}"
    );
}

#[test]
fn claude_and_codex_bindings_are_refused_no_hook_pull() {
    let f = fixture(1);
    let codex = check_in(&f, claim("x", "px", Harness::Codex, "nx", 1), "ci-x");
    assert_eq!(refusal(pull(&f, &codex, None)), Refusal::NoHookPull);
    let claude = check_in(&f, claim("c", "pc", Harness::Claude, "nc", 1), "ci-c");
    assert_eq!(refusal(pull(&f, &claude, None)), Refusal::NoHookPull);
    assert_eq!(
        confirm(&f, &codex, &["m1"], "op-1"),
        vec![(
            "m1".to_owned(),
            Outcome::RefusedTerminal,
            Some(Reason::NoHookPull)
        )]
    );
}

#[test]
fn session_mismatch_subagent_held_and_unresolved_seats_are_refused() {
    let f = fixture(1);
    let context = pull_seat(&f);

    let mut other_session = context.clone();
    other_session.native_session = NativeSessionId::new("someone-else");
    // Session-keyed bindings: the claim's key names no binding of the seat.
    assert_eq!(refusal(pull(&f, &other_session, None)), Refusal::NoBinding);

    let mut subagent = context.clone();
    subagent.role = CallerRole::Subagent;
    assert_eq!(refusal(pull(&f, &subagent, None)), Refusal::SessionMismatch);

    let mut other_instance = context.clone();
    other_instance.instance = "elsewhere".into();
    assert_eq!(
        refusal(pull(&f, &other_instance, None)),
        Refusal::SessionMismatch
    );

    // A seat that never checked in has no binding.
    let none = claim("x", "px", fourth(), "nx", 1);
    assert_eq!(refusal(pull(&f, &none, None)), Refusal::NoBinding);

    // An unknown seat is unresolved.
    let ghost = claim("ghost", "pg", fourth(), "ng", 1);
    assert_eq!(refusal(pull(&f, &ghost, None)), Refusal::Unresolved);
    f.db.execute("UPDATE seats SET state='unresolved' WHERE id='s'", [])
        .unwrap();
    assert_eq!(refusal(pull(&f, &context, None)), Refusal::Unresolved);
    f.db.execute("UPDATE seats SET state='resolved' WHERE id='s'", [])
        .unwrap();

    f.db.execute("INSERT INTO recovery_holds(instance_id,target_id,baseline_boot,baseline_epoch,reason) VALUES('i','p','b',1,'repair')", []).unwrap();
    assert_eq!(refusal(pull(&f, &context, None)), Refusal::Held);
}

const MARK_MIGRATED: &str = "UPDATE occupant_bindings SET migrated=1, migrated_session=native_session, liveness='disconnected', disconnected_at=1, disconnect_reason='migrated', liveness_pid=NULL, liveness_start=NULL WHERE seat_id='s' AND ended_at IS NULL";

fn open_binding_row(f: &Fixture) -> (String, String, Option<i64>, String) {
    f.db.query_row(
        "SELECT binding_id, liveness, liveness_pid, observation_provenance FROM occupant_bindings WHERE seat_id='s' AND ended_at IS NULL",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )
    .unwrap()
}

#[test]
fn a_pull_re_adopts_a_migrated_binding_and_delivers() {
    let f = fixture(1);
    let context = pull_seat(&f);
    let (binding_id, ..) = open_binding_row(&f);
    f.db.execute(MARK_MIGRATED, []).unwrap();
    assert_eq!(open_binding_row(&f).1, "disconnected");

    let offered = page(pull(&f, &context, None));
    assert!(offered.text.contains("m1"), "{}", offered.text);
    let (after_id, liveness, pid, provenance) = open_binding_row(&f);
    assert_eq!(after_id, binding_id, "the same binding is re-adopted");
    assert_eq!(liveness, "connected");
    assert_eq!(
        pid,
        context.liveness.map(|l| i64::from(l.pid)),
        "the claim's fingerprint is recorded"
    );
    assert_eq!(provenance, "cooperative_env_session");
    assert_eq!(
        confirm(&f, &context, &["m1"], "op-1"),
        vec![("m1".to_owned(), Outcome::Settled, None)]
    );
}

#[test]
fn a_pull_without_a_fingerprint_does_not_re_adopt_and_is_refused_no_binding() {
    let f = fixture(1);
    let mut context = pull_seat(&f);
    f.db.execute(MARK_MIGRATED, []).unwrap();
    context.liveness = None;
    assert_eq!(refusal(pull(&f, &context, None)), Refusal::NoBinding);
    assert_eq!(open_binding_row(&f).1, "disconnected");
}

#[test]
fn a_pull_on_a_binding_the_store_never_reconnects_writes_nothing() {
    let f = fixture(1);
    let context = pull_seat(&f);
    f.db.execute(
        "UPDATE occupant_bindings SET liveness='disconnected', disconnected_at=1, disconnect_reason='liveness_end_event' WHERE seat_id='s' AND ended_at IS NULL",
        [],
    )
    .unwrap();
    let operations = |f: &Fixture| -> i64 {
        f.db.query_row("SELECT count(*) FROM operations", [], |r| r.get(0))
            .unwrap()
    };
    let before = operations(&f);
    assert_eq!(refusal(pull(&f, &context, None)), Refusal::NoBinding);
    let (_, liveness, ..) = open_binding_row(&f);
    assert_eq!(liveness, "disconnected");
    assert_eq!(operations(&f), before, "no synthetic operation is recorded");
}

#[test]
fn a_pull_reconnects_a_disconnected_non_migrated_binding_and_delivers() {
    let f = fixture(1);
    let context = pull_seat(&f);
    f.db.execute(
        "UPDATE occupant_bindings SET liveness='disconnected', disconnected_at=1, disconnect_reason='liveness_process_exit' WHERE seat_id='s' AND ended_at IS NULL",
        [],
    )
    .unwrap();
    let (binding_id, _, _, provenance_before) = open_binding_row(&f);
    let observed = |f: &Fixture| -> i64 {
        f.db.query_row(
            "SELECT observed_at FROM occupant_bindings WHERE seat_id='s' AND ended_at IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };
    let observed_before = observed(&f);
    assert_eq!(open_binding_row(&f).1, "disconnected");

    let offered = page(pull(&f, &context, None));
    assert!(offered.text.contains("m1"), "{}", offered.text);
    let (after_id, liveness, pid, provenance) = open_binding_row(&f);
    assert_eq!(after_id, binding_id);
    assert_eq!(liveness, "connected");
    assert_eq!(pid, context.liveness.map(|l| i64::from(l.pid)));
    assert_eq!(
        provenance, provenance_before,
        "provenance stays as recorded"
    );
    assert_eq!(observed(&f), observed_before, "claim time stays");
    assert_eq!(
        confirm(&f, &context, &["m1"], "op-1"),
        vec![("m1".to_owned(), Outcome::Settled, None)]
    );
}

#[test]
fn confirm_of_an_id_never_offered_is_not_offered_and_settles_nothing() {
    let f = fixture(1);
    let context = pull_seat(&f);
    assert_eq!(
        confirm(&f, &context, &["m1"], "op-1"),
        vec![("m1".to_owned(), Outcome::NotOffered, None)]
    );
    assert_eq!(receipt_state(&f, "m1"), "pending");
    // Only what a pull showed is confirmable: m9 arrived after the page.
    page(pull(&f, &context, None));
    ordinary_message(&f, "m9", "unseen");
    assert_eq!(
        confirm(&f, &context, &["m1", "m9", "ghost"], "op-2"),
        vec![
            ("m1".to_owned(), Outcome::Settled, None),
            ("m9".to_owned(), Outcome::NotOffered, None),
            ("ghost".to_owned(), Outcome::NotOffered, None),
        ]
    );
    assert_eq!(receipt_state(&f, "m9"), "pending");
}

#[test]
fn settled_elsewhere_ids_leave_the_ledger_and_an_empty_page_is_unchanged() {
    let f = fixture(1);
    let context = pull_seat(&f);
    let first = page(pull(&f, &context, None));
    // The seat reads the item with `ack` outside the hook.
    f.db.execute(
        "UPDATE receipts SET state='acked' WHERE message_id='m1' AND seat_id='s'",
        [],
    )
    .unwrap();
    advance(&f, REOFFER_AFTER_MS);
    // Nothing is pending any more: the version moved or the page is empty, and
    // the empty page answers Unchanged.
    let reply = pull(&f, &context, Some(first.attention_version));
    assert!(
        matches!(reply, DeliveryPullReply::Unchanged { .. }),
        "{reply:?}"
    );
    assert_eq!(
        confirm(&f, &context, &["m1"], "op-1"),
        vec![("m1".to_owned(), Outcome::NotOffered, None)],
        "the dropped offer is gone"
    );
}

#[test]
fn already_settled_ids_report_already_settled_and_leave_the_ledger() {
    let f = fixture(1);
    let context = pull_seat(&f);
    page(pull(&f, &context, None));
    f.db.execute(
        "UPDATE receipts SET state='acked' WHERE message_id='m1' AND seat_id='s'",
        [],
    )
    .unwrap();
    assert_eq!(
        confirm(&f, &context, &["m1"], "op-1"),
        vec![("m1".to_owned(), Outcome::AlreadySettled, None)]
    );
    assert_eq!(
        confirm(&f, &context, &["m1"], "op-2"),
        vec![("m1".to_owned(), Outcome::NotOffered, None)]
    );
}

#[test]
fn the_marker_prefix_reaches_the_truncation_marker() {
    let f = fixture(0);
    ordinary_message(&f, "big", &"y".repeat(WATCH_BODY_LIMIT_BYTES + 10));
    let context = pull_seat(&f);
    let command = Command::DeliveryPull(DeliveryPull {
        claim: context,
        seen_attention_version: None,
        marker_prefix: vec!["/opt/ht".into(), "--state-dir".into(), "/s".into()],
    });
    let CommandResult::DeliveryPulled(reply) =
        f.service.handle(command, peer(), &budget()).unwrap()
    else {
        panic!("not a pull reply");
    };
    assert!(page(reply).text.contains("/opt/ht --state-dir /s body big"));
}

/// One pending invitation for seat `s` (what wakes a seat without a message).
fn pending_invitation(f: &Fixture) {
    let decision: i64 =
        f.db.query_row(
            "SELECT decision_seq+1 FROM host_instances WHERE id='i'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    f.db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence) VALUES ('t2','i','topic','goal',0,0,1)", []).unwrap();
    f.db.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,frozen_duration_ms,deadline_at) VALUES('inv','t2','s',1,'pending',0,?1,300,300)", [decision]).unwrap();
    bump_wake_attention(f);
}
fn pull_with_prefix(
    f: &Fixture,
    claim: &CallerClaim,
    seen: Option<u64>,
    prefix: &[&str],
) -> DeliveryPullReply {
    let command = Command::DeliveryPull(DeliveryPull {
        claim: claim.clone(),
        seen_attention_version: seen,
        marker_prefix: prefix.iter().map(|part| (*part).to_owned()).collect(),
    });
    match f.service.handle(command, peer(), &budget()).unwrap() {
        CommandResult::DeliveryPulled(reply) => reply,
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn attention_block_is_present_with_a_pending_invitation_and_follows_the_marker_prefix() {
    let f = fixture(0);
    pending_invitation(&f);
    let context = pull_seat(&f);
    let first = page(pull_with_prefix(
        &f,
        &context,
        None,
        &["/opt/ht", "--state-dir", "/s"],
    ));
    assert!(
        first.text.contains(&format!(
            "[herdr-threads] attention {}:",
            first.attention_version
        )),
        "{}",
        first.text
    );
    assert!(
        first.text.contains("run /opt/ht --state-dir /s inbox"),
        "{}",
        first.text
    );
    // The default prefix keeps the wake marker byte for byte.
    let f = fixture(0);
    pending_invitation(&f);
    let context = pull_seat(&f);
    let default = page(pull(&f, &context, None));
    assert!(
        default.text.contains(crate::notification::policy::MARKER),
        "{}",
        default.text
    );
}

#[test]
fn attention_block_is_absent_when_the_version_was_seen() {
    let f = fixture(0);
    pending_invitation(&f);
    let context = pull_seat(&f);
    let first = page(pull(&f, &context, None));
    assert!(first.text.contains("attention"), "{}", first.text);
    assert_eq!(
        pull(&f, &context, Some(first.attention_version)),
        DeliveryPullReply::Unchanged {
            attention_version: first.attention_version
        }
    );
}

#[test]
fn an_attention_only_page_answers_page() {
    let f = fixture(0);
    pending_invitation(&f);
    let context = pull_seat(&f);
    let only = page(pull(&f, &context, None));
    assert!(only.offered.is_empty(), "{:?}", only.offered);
    assert!(only.truncated.is_empty(), "{:?}", only.truncated);
    assert!(only.text.contains("attention"), "{}", only.text);
    assert!(
        !only.text.contains("[herdr-threads] message"),
        "{}",
        only.text
    );
}

#[test]
fn attention_version_moves_with_every_fingerprint_field() {
    use crate::ports::ModFingerprint;
    let base = ModFingerprint::default();
    let version = |fingerprint: &ModFingerprint| attention_version(fingerprint, None);
    let moved = [
        ModFingerprint {
            attention_version: 1,
            ..base.clone()
        },
        ModFingerprint {
            pending_receipts: 1,
            ..base.clone()
        },
        ModFingerprint {
            max_pending_ordinal: 1,
            ..base.clone()
        },
        ModFingerprint {
            lazy_pending: 1,
            ..base.clone()
        },
        ModFingerprint {
            max_lazy_rowid: 1,
            ..base.clone()
        },
        ModFingerprint {
            unread: 1,
            ..base.clone()
        },
        ModFingerprint {
            max_unread_rowid: 1,
            ..base.clone()
        },
        ModFingerprint {
            other_pending: 1,
            ..base.clone()
        },
        ModFingerprint {
            binding_generation: Some(0),
            ..base.clone()
        },
    ];
    let mut seen = std::collections::BTreeSet::from([version(&base)]);
    for fingerprint in &moved {
        assert!(seen.insert(version(fingerprint)), "{fingerprint:?}");
    }
    assert_eq!(version(&base), version(&base.clone()), "deterministic");
    // The notice term: absent, ordinal 1 and ordinal 2 are three versions.
    let with_notice = |notice| attention_version(&base, notice);
    let mut seen = std::collections::BTreeSet::new();
    for notice in [None, Some(1), Some(2)] {
        assert!(seen.insert(with_notice(notice)), "{notice:?}");
    }
}

#[test]
fn delivery_pull_is_advertised() {
    assert!(crate::protocol::capabilities::ADVERTISED.contains(&"delivery.pull_v1"));
}

// Kills: a pull answered `Unchanged` before this process read the seat's whole
// pending set, which strands offered-unconfirmed items after a daemon restart.
#[test]
fn a_restarted_daemon_offers_pending_items_again_to_a_hook_holding_its_last_version() {
    let mut f = fixture(1);
    let context = pull_seat(&f);
    let first = page(pull(&f, &context, None));
    assert_eq!(first.offered, ids(&["m1"]));
    restart(&mut f);
    assert_eq!(
        confirm(&f, &context, &["m1"], "op-1"),
        vec![("m1".to_owned(), Outcome::NotOffered, None)]
    );
    assert_eq!(receipt_state(&f, "m1"), "pending");
    let again = page(pull(&f, &context, Some(first.attention_version)));
    assert_eq!(again.offered, ids(&["m1"]));
    assert_eq!(
        pull(&f, &context, Some(again.attention_version)),
        DeliveryPullReply::Unchanged {
            attention_version: again.attention_version
        },
        "the short-circuit works again after a whole read"
    );
}

// Kills: an outlook that ignores offers, due re-offers, parking, new attention
// or a rotated generation.
#[test]
fn the_hook_pull_outlook_follows_offers_reoffers_and_parking() {
    let f = fixture(1);
    let context = pull_seat(&f);
    let seat = SeatId::new("s");
    let generation = context.binding_generation;
    let targets = [binding_of(&f, "n1")];
    let now = |f: &Fixture| UtcMillis(f.clock.0.load(Ordering::SeqCst));
    let outlook = |f: &Fixture| {
        f.state
            .next_pull(&seat, &targets, generation, wake_attention(f), now(f))
    };
    assert_eq!(outlook(&f), NextPull::Unknown, "before any pull");
    page(pull(&f, &context, None));
    assert_eq!(
        outlook(&f),
        NextPull::ReofferAt(UtcMillis(now(&f).0 + REOFFER_AFTER_MS))
    );
    assert!(!f.state.holds_parked(&seat, &targets, generation));
    for _ in 1..MAX_OFFERS_PER_GENERATION {
        advance(&f, REOFFER_AFTER_MS);
        assert_eq!(outlook(&f), NextPull::Delivers, "the re-offer is due");
        page(pull(&f, &context, None));
    }
    assert_eq!(outlook(&f), NextPull::Nothing);
    assert!(f.state.holds_parked(&seat, &targets, generation));
    ordinary_message(&f, "m2", "later");
    bump_wake_attention(&f);
    assert_eq!(outlook(&f), NextPull::Delivers, "new attention");
    let rotated = rotate(&f, &context, "n2", 2);
    // Session-keyed bindings: a new session keeps the seat's binding generation; its first pull
    // discards the previous session's entry and starts over.
    page(pull(&f, &rotated, None));
    let rotated_targets = [binding_of(&f, "n2")];
    assert_eq!(
        f.state.next_pull(
            &seat,
            &rotated_targets,
            rotated.binding_generation,
            wake_attention(&f),
            now(&f)
        ),
        NextPull::ReofferAt(UtcMillis(now(&f).0 + REOFFER_AFTER_MS)),
        "a new session starts over"
    );
    assert!(
        !f.state
            .holds_parked(&seat, &rotated_targets, rotated.binding_generation)
    );
}

// ---- the offered ledger is per binding (ht-54r.94) ----

/// A second live top-level binding of seat `s` in native session `n2`: a second process, so it does not
/// supersede the first.
fn second_binding(f: &Fixture) -> CallerClaim {
    let mut second = claim("s", "p", fourth(), "n2", 1);
    second.liveness = Some(crate::protocol::authority::LivenessFingerprint {
        pid: 4242,
        start: 1,
    });
    let second = check_in(f, fresh_execution(second, 9), "ci-b");
    let connected: i64 = f
        .db
        .query_row(
            "SELECT count(*) FROM occupant_bindings WHERE seat_id='s' AND ended_at IS NULL AND liveness='connected'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(connected, 2, "both bindings of the seat are connected");
    second
}

/// The open binding of seat `s` in native session `session`.
fn binding_of(f: &Fixture, session: &str) -> BindingId {
    BindingId::new(
        f.db.query_row::<String, _, _>(
            "SELECT binding_id FROM occupant_bindings WHERE seat_id='s' AND native_session=?1 AND ended_at IS NULL",
            [session],
            |r| r.get(0),
        )
        .unwrap(),
    )
}

/// Resolve the exact canonical open binding named by a claim; no latest-seat fallback.
fn binding_for_claim(f: &Fixture, claim: &CallerClaim) -> BindingId {
    BindingId::new(f.db.query_row::<String, _, _>(
        "SELECT binding_id FROM occupant_bindings WHERE seat_id=?1 AND harness=?2 AND native_session=?3 AND agent_id=?4 AND ended_at IS NULL",
        params![claim.seat.as_str(), claim.harness.as_str(), claim.native_session.as_str(), claim.agent_id.as_ref().map_or("", |agent| agent.as_str())],
        |row| row.get(0),
    ).unwrap())
}

// Kills: one ledger entry per seat, so B's pull wiped A's offers and A's confirm answered NotOffered
// (main review I2).
#[test]
fn interleaved_pulls_of_two_bindings_confirm_their_own_offers() {
    let f = fixture(1);
    let a = pull_seat(&f);
    let b = second_binding(&f);
    assert_eq!(page(pull(&f, &a, None)).offered, ids(&["m1"]));
    assert_eq!(page(pull(&f, &b, None)).offered, ids(&["m1"]));
    assert_eq!(
        confirm(&f, &a, &["m1"], "op-a"),
        vec![("m1".to_owned(), Outcome::Settled, None)]
    );
    assert_eq!(receipt_state(&f, "m1"), "acked");
    assert_eq!(
        confirm(&f, &b, &["m1"], "op-b"),
        vec![("m1".to_owned(), Outcome::AlreadySettled, None)]
    );
}

// Kills: alternating pulls recreating count 1 and last_at, so neither the 60 s spacing nor the 3-offer bound
// held per binding.
#[test]
fn alternating_pulls_keep_each_bindings_reoffer_spacing_and_count() {
    let f = fixture(1);
    let a = pull_seat(&f);
    let b = second_binding(&f);
    let first = page(pull(&f, &a, None));
    assert_eq!(first.offered, ids(&["m1"]));
    assert_eq!(page(pull(&f, &b, None)).offered, ids(&["m1"]));
    assert_eq!(
        pull(&f, &a, Some(first.attention_version)),
        DeliveryPullReply::Unchanged {
            attention_version: first.attention_version
        },
        "A's own spacing holds after B pulled"
    );
    for _ in 1..MAX_OFFERS_PER_GENERATION {
        advance(&f, REOFFER_AFTER_MS);
        assert_eq!(page(pull(&f, &a, None)).offered, ids(&["m1"]));
        assert_eq!(page(pull(&f, &b, None)).offered, ids(&["m1"]));
    }
    advance(&f, 10 * REOFFER_AFTER_MS);
    for binding in [&a, &b] {
        let reply = pull(&f, binding, Some(first.attention_version));
        assert!(
            matches!(reply, DeliveryPullReply::Unchanged { .. }),
            "three showings per binding, then parked: {reply:?}"
        );
    }
    assert_eq!(receipt_state(&f, "m1"), "pending");
}

// Kills: a new session of one binding replacing the seat's entry and wiping its sibling's offers.
#[test]
fn a_new_session_of_one_binding_keeps_its_siblings_offers() {
    let f = fixture(1);
    let a = pull_seat(&f);
    let b = second_binding(&f);
    page(pull(&f, &a, None));
    page(pull(&f, &b, None));
    let a2 = rotate(&f, &a, "n3", 3);
    page(pull(&f, &a2, None));
    assert_eq!(
        confirm(&f, &b, &["m1"], "op-b"),
        vec![("m1".to_owned(), Outcome::Settled, None)]
    );
}

// Kills: an outlook that answers from one binding's entry: a wake that reaches both bindings must wait for both.
#[test]
fn the_hook_pull_outlook_waits_for_every_target_binding() {
    let f = fixture(1);
    let a = pull_seat(&f);
    let b = second_binding(&f);
    let seat = SeatId::new("s");
    let generation = a.binding_generation;
    let (bind_a, bind_b) = (binding_of(&f, "n1"), binding_of(&f, "n2"));
    let outlook = |f: &Fixture, targets: &[BindingId]| {
        f.state
            .next_pull(&seat, targets, generation, wake_attention(f), now_utc(f))
    };
    let shown_at = now_utc(&f);
    page(pull(&f, &a, None));
    assert_eq!(
        outlook(&f, std::slice::from_ref(&bind_a)),
        NextPull::ReofferAt(UtcMillis(shown_at.0 + REOFFER_AFTER_MS))
    );
    assert_eq!(
        outlook(&f, &[bind_a.clone(), bind_b.clone()]),
        NextPull::Unknown,
        "B has not pulled in this process: its next pull may deliver"
    );
    advance(&f, 5_000);
    page(pull(&f, &b, None));
    assert_eq!(
        outlook(&f, &[bind_a.clone(), bind_b.clone()]),
        NextPull::ReofferAt(UtcMillis(shown_at.0 + REOFFER_AFTER_MS)),
        "the earlier of the two re-offers"
    );
    assert_eq!(
        outlook(&f, std::slice::from_ref(&bind_b)),
        NextPull::ReofferAt(UtcMillis(shown_at.0 + 5_000 + REOFFER_AFTER_MS))
    );
    assert_eq!(outlook(&f, &[]), outlook(&f, &[bind_a, bind_b]));
}

// Kills: same-seat, same-generation D state aliasing across two connected bindings.
// Real pulls populate both ledgers and withheld carry; foreign hints/tokens cannot advance either.
#[test]
fn presentation_two_live_bindings_keep_offer_stamps_replay_and_carry_independent() {
    use crate::protocol::delivery::{
        DeliveryPresentationConfirmV2, DeliveryPresentationConfirmationV2, DeliveryPullReplyV2,
        DeliveryPullV2, PresentationConsumer, PresentationRenderRequest,
    };
    let f = fixture(1);
    let a = pull_seat(&f);
    let b = second_binding(&f);
    let (binding_a, binding_b) = (binding_for_claim(&f, &a), binding_for_claim(&f, &b));
    assert_eq!(a.seat, b.seat);
    assert_eq!(a.binding_generation, b.binding_generation);
    assert_ne!(binding_a, binding_b);
    f.db.execute("UPDATE seats SET wake_on='all' WHERE id='s'", [])
        .unwrap();
    let inviter = check_in(
        &f,
        fixture_claim(&f, "x", "px", fourth(), "binding-isolation-inviter"),
        "binding-isolation-ci",
    );
    canonical_lazy_invitation_backlog(&f, &a, &inviter, 1, "binding-isolation-baseline");
    for claim in [&a, &b] {
        let DeliveryPullReplyV2::Page(page) = presentation_test_pull(&f, claim) else {
            panic!("each binding has its own initial page")
        };
        assert_eq!(page.offered, ids(&["m1"]));
        presentation_test_confirm(&f, claim, &page);
    }
    let stamps = |binding: &BindingId| f.state.lock().get(binding).unwrap().offers.clone();
    let a_stamps = stamps(&binding_a);
    let b_stamps = stamps(&binding_b);
    assert_eq!(a_stamps, b_stamps);
    assert_eq!(a_stamps[&MessageId::new("m1")].count, 1);
    canonical_lazy_invitation_backlog(&f, &a, &inviter, 24, "binding-isolation-priority");
    lazy_message(&f, "msg-binding-isolation-carry", &"\0".repeat(9000));
    let pull = |claim: &CallerClaim| {
        let CommandResult::DeliveryPulledV2(DeliveryPullReplyV2::Page(page)) = f
            .service
            .handle(
                Command::DeliveryPullV2(DeliveryPullV2 {
                    claim: claim.clone(),
                    consumer: PresentationConsumer::Hook,
                    render: PresentationRenderRequest::Hook {
                        confirmable_selector: 0,
                        passive_selector: 0,
                    },
                    presentation_lease: None,
                    marker_prefix: vec!["p".repeat(4096)],
                }),
                peer(),
                &budget(),
            )
            .unwrap()
        else {
            panic!("independent priority page")
        };
        page
    };
    let state = |binding: &BindingId| {
        let mut statement = f.db.prepare("SELECT * FROM invitation_presentation_consumers WHERE binding_id=?1 AND consumer='hook'").unwrap();
        statement
            .query_row([binding.as_str()], |row| {
                (0..row.as_ref().column_count())
                    .map(|column| row.get::<_, rusqlite::types::Value>(column))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
    };
    let carry = |binding: &BindingId| {
        let raw: String = f.db.query_row("SELECT ordinary_pending FROM invitation_presentation_consumers WHERE binding_id=?1 AND consumer='hook'", [binding.as_str()], |row| row.get(0)).unwrap();
        let carry: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(carry["prefix"], serde_json::json!(["p".repeat(4096)]));
        assert!(
            carry["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["content"]["id"] == "msg-binding-isolation-carry"),
            "real withheld carry: {carry}"
        );
        raw
    };
    let a_page = pull(&a);
    let a_carry = carry(&binding_a);
    let a_state = state(&binding_a);
    let b_page = pull(&b);
    assert_ne!(a_page.presentation_token, b_page.presentation_token);
    let b_carry = carry(&binding_b);
    let b_state = state(&binding_b);
    assert_eq!(
        state(&binding_a),
        a_state,
        "B cannot replace A's outbox/carry/frontiers"
    );
    assert_eq!(stamps(&binding_a), a_stamps);
    assert_eq!(stamps(&binding_b), b_stamps);
    let CommandResult::DeliveryPresentationConfirmedV2(wrong) = f
        .service
        .handle(
            Command::DeliveryPresentationConfirmV2(DeliveryPresentationConfirmV2 {
                claim: b.clone(),
                consumer: PresentationConsumer::Hook,
                token: a_page.presentation_token.clone().unwrap(),
                presentation_lease: None,
                emitted_controls: a_page.emitted_controls.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("physical confirmation reply")
    };
    assert_eq!(
        wrong.status,
        DeliveryPresentationConfirmationV2::NotCurrentPage
    );
    assert_eq!(state(&binding_a), a_state);
    assert_eq!(state(&binding_b), b_state);
    let replayed = pull(&a);
    assert_eq!(replayed, a_page);
    assert_eq!(carry(&binding_a), a_carry);
    assert_eq!(carry(&binding_b), b_carry);
    assert_eq!(state(&binding_b), b_state);
    let due_a = f
        .state
        .lock()
        .binding_mut(&a.seat, &binding_a, a.binding_generation)
        .unwrap()
        .due_snapshot(PresentationConsumer::Watch);
    let due_b = f
        .state
        .lock()
        .binding_mut(&b.seat, &binding_b, b.binding_generation)
        .unwrap()
        .due_snapshot(PresentationConsumer::Watch);
    let mut foreign = offer_reconciliation_input(&f, &b, due_a.stamps.keys().cloned().collect());
    foreign.ordinary_due_snapshot = Some(due_a);
    assert_eq!(
        f.store
            .presentation_pull(foreign.clone(), &budget())
            .unwrap_err()
            .code,
        crate::protocol::results::ErrorCode::InvalidRequest
    );
    foreign.ordinary_due_snapshot = Some(due_b);
    install_owner_hint(&mut foreign);
    foreign.owner_replay.as_mut().unwrap().scope.binding_id = binding_a.clone();
    assert_eq!(
        f.store
            .presentation_pull(foreign, &budget())
            .unwrap_err()
            .code,
        crate::protocol::results::ErrorCode::InvalidRequest
    );
    assert_eq!(state(&binding_a), a_state);
    assert_eq!(state(&binding_b), b_state);
    assert_eq!(stamps(&binding_a), a_stamps);
    assert_eq!(stamps(&binding_b), b_stamps);
    presentation_test_confirm(&f, &a, &a_page);
    assert_eq!(
        state(&binding_b),
        b_state,
        "A's current physical confirmation cannot consume B"
    );
    presentation_test_confirm(&f, &b, &b_page);
    assert_eq!(receipt_state(&f, "m1"), "pending");
    assert_eq!(lazy_state(&f, "msg-binding-isolation-carry"), "pending");
    assert_eq!(
        f.db.query_row::<i64, _, _>(
            "SELECT count(*) FROM invitations WHERE seat_id='s' AND state='pending'",
            [],
            |row| row.get(0)
        )
        .unwrap(),
        25
    );
}

// ---- informational notices on the pull (ht-poh.53) ----

// Kills: leftover notices counted as wake work (final review 9 N1). Notices never wake: an offered id
// shown 10 s ago holds the wake until its re-offer is due, even with notices left over; the next pull
// still carries those notices and does not re-show the id.
#[test]
fn leftover_notices_do_not_bring_a_held_wake_forward() {
    let f = fixture(1);
    let context = pull_seat(&f);
    let seat = SeatId::new("s");
    let generation = context.binding_generation;
    let total = crate::protocol::results::MAX_NOTICE_PAGE_ITEMS + 2;
    for n in 0..total {
        service_notice(&f, &format!("w-{n:02}"));
    }
    let offered_at = UtcMillis(f.clock.0.load(Ordering::SeqCst));
    let first = page(pull(&f, &context, None));
    assert_eq!(first.offered, ids(&["m1"]));
    assert_eq!(
        first.text.matches("] notice w-").count(),
        16,
        "{}",
        first.text
    );
    advance(&f, 10_000);
    let targets = [binding_of(&f, "n1")];
    let outlook = |f: &Fixture| {
        f.state.next_pull(
            &seat,
            &targets,
            generation,
            wake_attention(f),
            UtcMillis(f.clock.0.load(Ordering::SeqCst)),
        )
    };
    assert_eq!(
        outlook(&f),
        NextPull::ReofferAt(UtcMillis(offered_at.0 + REOFFER_AFTER_MS)),
        "leftover notices are not wake work"
    );
    let second = page(pull(&f, &context, Some(first.attention_version)));
    assert!(
        second.text.contains("notice w-16") && second.text.contains("notice w-17"),
        "{}",
        second.text
    );
    assert!(second.offered.is_empty(), "m1's re-offer is not due yet");
    assert_eq!(
        outlook(&f),
        NextPull::ReofferAt(UtcMillis(offered_at.0 + REOFFER_AFTER_MS))
    );
    assert!(matches!(
        pull(&f, &context, Some(second.attention_version)),
        DeliveryPullReply::Unchanged { .. }
    ));
    advance(&f, REOFFER_AFTER_MS - 10_000);
    assert_eq!(outlook(&f), NextPull::Delivers, "the re-offer is due");
}

/// A projected informational notice for `s`: an overdue transition about
/// another seat (`x`), which never wakes `s` (TRUST-POLICY A7).
fn service_notice(f: &Fixture, id: &str) {
    let sequence = next_sequence(&f.db);
    f.db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,event_json,decision_at,decision_seq,event_offset) VALUES (?1,'i','t',?2,'warn','{}',0,?3,1)", params![id, sequence, sequence + 2]).unwrap();
    f.db.execute("INSERT INTO warning_conditions(condition_kind,thread_id,condition_id,affected_seat_id,open_warning_id,opened_seq) VALUES ('receipt','t',?1,'x',?1,?2)", params![id, sequence + 2]).unwrap();
    f.db.execute("INSERT INTO digest_programmatic_warnings(seat_id,warning_id,thread_id,event_seq,event_offset) VALUES ('s',?1,'t',?2,1)", params![id, sequence + 2]).unwrap();
    bump(&f.db, sequence);
}
fn overdue_notice_about_x(f: &Fixture, id: &str) {
    service_notice(f, id);
}
/// A projected invitation-rejection notice for `s`, authored by `x`.
fn rejection_notice(f: &Fixture, id: &str) {
    let sequence = next_sequence(&f.db);
    let inv = format!("inv-{id}");
    f.db.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,frozen_duration_ms,deadline_at) VALUES(?1,'t','x',9,'pending',0,1,300,300)", [&inv]).unwrap();
    f.db.execute("INSERT INTO invitation_rejections(invitation_id,reason,rejected_at,actor_seat_id,generation,observation) VALUES (?1,'outside role',100,'x',1,'cooperative_top_level:test')", [&inv]).unwrap();
    let json =
        format!(r#"{{"action":"reject","seat":"x","invitation":"{inv}","reason":"outside role"}}"#);
    f.db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,author_kind,actor_seat_id,source_invitation_id,event_key,event_json,decision_at,decision_seq,event_offset) VALUES (?1,'i','t',?2,'warn','native','x',?3,'reject:'||?3,?4,100,?5,1)", params![id, sequence, inv, json, sequence + 2]).unwrap();
    f.db.execute("INSERT INTO digest_programmatic_warnings(seat_id,warning_id,thread_id,event_seq,event_offset) VALUES ('s',?1,'t',?2,1)", params![id, sequence + 2]).unwrap();
    bump(&f.db, sequence);
}
fn notice_frontier(f: &Fixture) -> i64 {
    f.db.query_row(
        "SELECT COALESCE(MAX(offered_ordinal),0) FROM digest_notice_offer WHERE seat_id='s'",
        [],
        |r| r.get(0),
    )
    .unwrap()
}
fn current_check_in(
    f: &Fixture,
    claim: CallerClaim,
    operation: &str,
) -> crate::protocol::results::CheckInResult {
    let command = Command::CheckIn(CheckIn {
        mode: CheckInMode::Current,
        claim,
        operation: OperationId::new(operation),
        lineage_parent: None,
    });
    match f.service.handle(command, peer(), &budget()).unwrap() {
        CommandResult::CheckedIn(result) => result,
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn a_service_notice_reaches_the_next_pull_once_as_a_notice_block() {
    let f = fixture(0);
    let context = pull_seat(&f);
    service_notice(&f, "w-rej");
    let first = page(pull_with_prefix(&f, &context, None, &["ht"]));
    assert!(
        first.text.contains("[herdr-threads] notice w-rej in "),
        "{}",
        first.text
    );
    assert!(first.text.contains("ht body w-rej"), "{}", first.text);
    assert!(
        first.text.contains("informational, no ACK"),
        "{}",
        first.text
    );
    assert!(first.offered.is_empty() && first.truncated.is_empty());
    assert_eq!(
        pull(&f, &context, Some(first.attention_version)),
        DeliveryPullReply::Unchanged {
            attention_version: first.attention_version
        }
    );
    // Shown once: even without the version it is not shown again.
    assert_eq!(
        pull(&f, &context, None),
        DeliveryPullReply::Unchanged {
            attention_version: first.attention_version
        }
    );
}

#[test]
fn a_rejection_notice_reaches_the_next_pull_once_as_a_notice_block() {
    let f = fixture(0);
    let context = pull_seat(&f);
    rejection_notice(&f, "w-rej");
    assert!(
        !crate::store::attention::warning_wakes_seat(&f.db, "s", "w-rej").unwrap(),
        "a rejection notice is informational"
    );
    let first = page(pull(&f, &context, None));
    assert!(
        first.text.contains("[herdr-threads] notice w-rej in "),
        "{}",
        first.text
    );
    assert!(first.text.contains("from x"), "{}", first.text);
    assert!(first.text.contains("body w-rej"), "{}", first.text);
    assert!(first.offered.is_empty() && first.truncated.is_empty());
    assert!(matches!(
        pull(&f, &context, Some(first.attention_version)),
        DeliveryPullReply::Unchanged { .. }
    ));
}

#[test]
fn an_overdue_transition_about_another_seat_reaches_the_next_pull() {
    let f = fixture(0);
    let context = pull_seat(&f);
    overdue_notice_about_x(&f, "w-over");
    let first = page(pull(&f, &context, None));
    assert!(
        first.text.contains("[herdr-threads] notice w-over in "),
        "{}",
        first.text
    );
    assert!(first.offered.is_empty());
    assert!(matches!(
        pull(&f, &context, Some(first.attention_version)),
        DeliveryPullReply::Unchanged { .. }
    ));
}

#[test]
fn a_pending_notice_reserves_no_wake() {
    let f = fixture(0);
    let context = pull_seat(&f);
    let before = wake_attention(&f);
    overdue_notice_about_x(&f, "w-over");
    let view = f
        .store
        .mod_binding_view(
            &SeatId::new("s"),
            crate::ports::ModBindingRef::Claim(&context),
            &budget(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        view.fingerprint.other_pending, 0,
        "a notice is not mod attention"
    );
    assert_eq!(wake_attention(&f), before);
    let candidates: i64 =
        f.db.query_row(
            "SELECT count(*) FROM wake_batches WHERE seat_id='s'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(candidates, 0);
    let _ = page(pull(&f, &context, None));
    assert_eq!(
        wake_attention(&f),
        before,
        "carrying it moves no wake counter either"
    );
}

#[test]
fn a_notice_carried_by_a_pull_is_not_carried_by_a_later_lifecycle_check_in() {
    let f = fixture(0);
    let context = pull_seat(&f);
    service_notice(&f, "w-1");
    let _ = page(pull(&f, &context, None));
    let result = current_check_in(&f, context.clone(), "ci-2");
    assert!(result.notices.items.is_empty(), "{:?}", result.notices);
}

#[test]
fn a_notice_carried_by_a_check_in_is_not_pulled() {
    let f = fixture(0);
    let context = pull_seat(&f);
    service_notice(&f, "w-1");
    let result = current_check_in(&f, context.clone(), "ci-2");
    assert_eq!(result.notices.items.len(), 1);
    let context = result.context;
    assert_eq!(
        pull(&f, &context, None),
        DeliveryPullReply::Unchanged {
            attention_version: attention_version(
                &f.store
                    .mod_binding_view(
                        &SeatId::new("s"),
                        crate::ports::ModBindingRef::Claim(&context),
                        &budget(),
                    )
                    .unwrap()
                    .unwrap()
                    .fingerprint,
                None
            )
        }
    );
}

#[test]
fn a_new_notice_moves_the_version_past_unchanged() {
    let f = fixture(1);
    let context = pull_seat(&f);
    let first = page(pull(&f, &context, None));
    assert!(matches!(
        pull(&f, &context, Some(first.attention_version)),
        DeliveryPullReply::Unchanged { .. }
    ));
    service_notice(&f, "w-late");
    let second = page(pull(&f, &context, Some(first.attention_version)));
    assert!(second.text.contains("notice w-late"), "{}", second.text);
    assert!(second.offered.is_empty(), "the message is not re-offered");
}

#[test]
fn notices_beyond_one_page_continue_on_the_next_pull() {
    let f = fixture(0);
    let context = pull_seat(&f);
    let total = crate::protocol::results::MAX_NOTICE_PAGE_ITEMS + 2;
    for n in 0..total {
        service_notice(&f, &format!("w-{n:02}"));
    }
    let first = page(pull(&f, &context, None));
    assert_eq!(
        first.text.matches("] notice w-").count(),
        16,
        "{}",
        first.text
    );
    let second = page(pull(&f, &context, Some(first.attention_version)));
    assert_eq!(
        second.text.matches("] notice w-").count(),
        2,
        "{}",
        second.text
    );
    assert!(second.text.contains("notice w-16") && second.text.contains("notice w-17"));
    assert!(matches!(
        pull(&f, &context, Some(second.attention_version)),
        DeliveryPullReply::Unchanged { .. }
    ));
}

// Kills: notices waiting behind a message backlog (main review Important 2 knock-on).
#[test]
fn notices_ride_a_full_message_page() {
    let f = fixture(40);
    let context = pull_seat(&f);
    service_notice(&f, "w-wait");
    let first = page(pull(&f, &context, None));
    assert!(first.more);
    assert_eq!(
        first.offered.len(),
        PAGE_MAX_BLOCKS - 1,
        "one block slot is held for the notice"
    );
    assert!(first.text.contains("notice w-wait"), "{}", first.text);
    assert!(notice_frontier(&f) > 0, "settled by the frontier rule");
    let second = page(pull(&f, &context, Some(first.attention_version)));
    assert!(
        !second.text.contains("notice w-wait"),
        "shown once: {}",
        second.text
    );
    assert_eq!(second.offered.len(), 40 - (PAGE_MAX_BLOCKS - 1));
    assert!(!second.more);
}

#[test]
fn an_invitation_attention_block_is_not_repeated_after_notices_settle() {
    let f = fixture(0);
    pending_invitation(&f);
    let context = pull_seat(&f);
    service_notice(&f, "w-1");
    let first = page(pull(&f, &context, None));
    assert!(first.text.contains("attention pending"), "{}", first.text);
    assert!(first.text.contains("notice w-1"), "{}", first.text);
    assert_eq!(
        pull(&f, &context, Some(first.attention_version)),
        DeliveryPullReply::Unchanged {
            attention_version: first.attention_version
        }
    );
}

#[test]
fn confirm_of_a_notice_id_is_not_offered() {
    let f = fixture(0);
    let context = pull_seat(&f);
    service_notice(&f, "w-1");
    let _ = page(pull(&f, &context, None));
    assert_eq!(
        confirm(&f, &context, &["w-1"], "op-n"),
        vec![("w-1".to_owned(), Outcome::NotOffered, None)]
    );
}

// ---- a backlog over the offer table (ht-poh.59, main review Important 2) ----

/// One pull as a hook makes it: it sends the last version it saw and keeps the reply's. Records when each id was
/// shown and returns the page text (None for `Unchanged`).
fn hook_pull_once(
    f: &Fixture,
    context: &CallerClaim,
    seen: &mut Option<u64>,
    shown: &mut std::collections::BTreeMap<MessageId, Vec<i64>>,
) -> Option<String> {
    match pull(f, context, *seen) {
        DeliveryPullReply::Page(page) => {
            *seen = Some(page.attention_version);
            let at = f.clock.0.load(Ordering::SeqCst);
            for id in page.offered.iter().chain(&page.truncated) {
                shown.entry(id.clone()).or_default().push(at);
            }
            Some(page.text)
        }
        DeliveryPullReply::Unchanged { attention_version } => {
            *seen = Some(attention_version);
            None
        }
        other => panic!("unexpected {other:?}"),
    }
}

/// Pending items the seat's pulls have read so far (the drain's store walk).
fn items_read(f: &Fixture) -> usize {
    f.state.probe.items_read.load(Ordering::SeqCst)
}

// Kills: a full offer table evicting still-pending ids (re-offered afresh: shown again < 60 s later, more than
// three times), a full table answering Delivers forever, notices starving behind the endless `more` backlog
// (main review Important 2), and a full-table pull walking the whole pending set when the last whole read
// already saw it (final review 11 S2: 70 s+ here; a large backlog overran the hook's budget).
#[test]
fn a_backlog_over_the_offer_table_keeps_the_reoffer_bounds_and_parks_for_inbox() {
    use crate::delivery::ledger::MAX_OFFERS_PER_BINDING;
    // The smallest backlog that overfills the table, plus a few. The first four bodies are over the limit
    // (truncated, never confirmable); the rest are short, so a store page holds 32 of them.
    let backlog = MAX_OFFERS_PER_BINDING + 4;
    let f = fixture(0);
    let big = "x".repeat(WATCH_BODY_LIMIT_BYTES + 100);
    for n in 0..backlog {
        let body = if n < 4 { big.as_str() } else { "hello" };
        ordinary_message(&f, &format!("b{n:03}"), body);
    }
    let context = pull_seat(&f);
    // After the check-in, which would carry a notice already pending.
    service_notice(&f, "w-1");
    let seat = SeatId::new("s");
    let generation = context.binding_generation;
    let targets = [binding_of(&f, "n1")];
    let now = |f: &Fixture| UtcMillis(f.clock.0.load(Ordering::SeqCst));
    let outlook = |f: &Fixture| {
        f.state
            .next_pull(&seat, &targets, generation, wake_attention(f), now(f))
    };
    let mut shown = std::collections::BTreeMap::new();
    let mut seen = None;
    let bounded = |shown: &std::collections::BTreeMap<MessageId, Vec<i64>>| {
        for (id, times) in shown {
            assert!(
                times.len() <= usize::from(MAX_OFFERS_PER_GENERATION),
                "{id:?} shown {} times: {times:?}",
                times.len()
            );
            assert!(
                times
                    .windows(2)
                    .all(|pair| pair[1] - pair[0] >= REOFFER_AFTER_MS),
                "{id:?} shown again under 60 s: {times:?}"
            );
        }
    };

    // A hook pull every 100 ms for 1.6 s: the table fills in the first nine.
    let mut texts = Vec::new();
    for _ in 0..16 {
        texts.push(hook_pull_once(&f, &context, &mut seen, &mut shown));
        advance(&f, 100);
    }
    bounded(&shown);
    let first = texts[0].as_deref().expect("the first pull shows a page");
    assert!(
        first.contains("notice w-1"),
        "the notice rides the backlog page: {first}"
    );
    assert!(notice_frontier(&f) > 0, "the carried notice settled");
    assert_eq!(
        texts
            .iter()
            .flatten()
            .filter(|text| text.contains("notice w-1"))
            .count(),
        1,
        "the notice is shown once"
    );
    assert_eq!(
        shown.len(),
        MAX_OFFERS_PER_BINDING,
        "the table filled, then took no new id"
    );
    for n in MAX_OFFERS_PER_BINDING..backlog {
        let id = format!("b{n:03}");
        assert!(
            !shown.contains_key(&MessageId::new(&id)),
            "{id} waits for inbox"
        );
        assert_eq!(receipt_state(&f, &id), "pending");
    }
    let first_shown = shown.values().map(|times| times[0]).min().unwrap();
    assert_eq!(
        outlook(&f),
        NextPull::ReofferAt(UtcMillis(first_shown + REOFFER_AFTER_MS)),
        "a full table is not Delivers-forever"
    );
    assert!(
        f.state.holds_parked(&seat, &targets, generation),
        "a full table parks new ids for inbox"
    );

    // Past 60 s, 120 s and 180 s: 1.2 s of pulls every 100 ms each time. The last whole read saw this same
    // pending set, so a re-offer pull reads only as far as its due ids.
    for round in 1..=3 {
        f.clock
            .0
            .store(first_shown + round * REOFFER_AFTER_MS, Ordering::SeqCst);
        for index in 0..12 {
            let before = items_read(&f);
            hook_pull_once(&f, &context, &mut seen, &mut shown);
            if round == 1 && index == 0 {
                let read = items_read(&f) - before;
                assert!(
                    read <= PAGE_MAX_BLOCKS,
                    "the first re-offer pull stops after its due ids; it read {read} of {backlog}"
                );
            }
            advance(&f, 100);
        }
        bounded(&shown);
    }
    assert_eq!(shown.len(), MAX_OFFERS_PER_BINDING);
    assert!(
        shown
            .values()
            .all(|times| times.len() == usize::from(MAX_OFFERS_PER_GENERATION)),
        "every offered id was re-offered up to the bound, then parked"
    );
    assert_eq!(outlook(&f), NextPull::Nothing);
    assert!(f.state.holds_parked(&seat, &targets, generation));

    // A new notice on the parked full table rides a page without reading the pending set: nothing in it could be
    // shown or freed.
    service_notice(&f, "w-2");
    let before = items_read(&f);
    let text = hook_pull_once(&f, &context, &mut seen, &mut shown).expect("the notice is shown");
    assert!(text.contains("notice w-2"), "{text}");
    assert_eq!(items_read(&f), before, "the drain was skipped");
    assert!(
        shown
            .values()
            .all(|times| times.len() == usize::from(MAX_OFFERS_PER_GENERATION)),
        "no message rides along"
    );
    // New attention still wakes; the parked table makes that wake name `inbox`.
    bump_wake_attention(&f);
    assert_eq!(outlook(&f), NextPull::Delivers);
}

// Kills: an id withheld at capacity stuck behind `Unchanged` after the same whole read freed a slot.
#[test]
fn an_id_withheld_at_capacity_is_shown_once_a_whole_read_frees_a_slot() {
    use crate::delivery::ledger::MAX_OFFERS_PER_BINDING;
    let f = fixture(MAX_OFFERS_PER_BINDING + 1);
    let context = pull_seat(&f);
    let mut seen = None;
    for _ in 0..MAX_OFFERS_PER_BINDING / PAGE_MAX_BLOCKS {
        seen = Some(page(pull(&f, &context, seen)).attention_version);
    }
    let last = format!("m{}", MAX_OFFERS_PER_BINDING + 1);
    assert!(
        matches!(
            pull(&f, &context, seen),
            DeliveryPullReply::Unchanged { .. }
        ),
        "the table is full: {last} waits"
    );
    assert!(f.state.holds_parked(
        &SeatId::new("s"),
        &[binding_of(&f, "n1")],
        context.binding_generation
    ));
    // `inbox` settles m1 outside the hook: the version moves and the whole read drops m1 from the table.
    f.db.execute(
        "UPDATE receipts SET state='acked' WHERE message_id='m1' AND seat_id='s'",
        [],
    )
    .unwrap();
    let version = match pull(&f, &context, seen) {
        DeliveryPullReply::Unchanged { attention_version } => attention_version,
        other => panic!("nothing fits before the whole read frees the slot: {other:?}"),
    };
    let freed = page(pull(&f, &context, Some(version)));
    assert_eq!(
        freed.offered,
        ids(&[last.as_str()]),
        "no time passed: the freed slot shows it"
    );
}

// Kills: a pull that settled its notices and then lost a rotation race before recording, answering Busy so the
// settled notices are never shown (final review 11 S3).
#[test]
fn notices_settled_by_a_pull_that_lost_a_rotation_race_are_still_returned() {
    let f = fixture(2);
    let context = pull_seat(&f);
    service_notice(&f, "w-race");
    let newer = context.binding_generation + 1;
    let racing = binding_of(&f, "n1");
    let racing_entry = racing.clone();
    *f.state.probe.before_record.lock().unwrap() =
        Some(Box::new(move |ledger: &mut OfferedLedger| {
            ledger
                .binding_mut(&SeatId::new("s"), &racing, newer)
                .expect("a newer generation replaces the entry");
        }));
    let page = page(pull(&f, &context, None));
    assert!(
        page.text.contains("[herdr-threads] notice w-race in "),
        "{}",
        page.text
    );
    assert!(
        notice_frontier(&f) > 0,
        "the notice settled before the race"
    );
    assert!(
        page.offered.is_empty() && page.truncated.is_empty() && !page.more,
        "no message is shown to a superseded generation: {page:?}"
    );
    assert!(!page.text.contains("message m1"), "{}", page.text);
    let ledger = f.state.lock();
    let entry = ledger.get(&racing_entry).unwrap();
    assert_eq!(entry.generation, newer);
    assert!(
        entry.offers.is_empty(),
        "nothing recorded into the newer entry"
    );
}

// Kills: dropping informational invitations from the actual service page, or
// treating a displayed invitation as a message receipt / accepted membership.
#[test]
fn a_lazy_invitation_reaches_hook_pull_without_attention_or_acceptance() {
    use crate::protocol::commands::{CreateThread, InvitationConfirmation, Invite};
    let f = fixture(0);
    let context = pull_seat(&f);
    let DeliveryPullReply::Unchanged {
        attention_version: quiet_version,
    } = pull(&f, &context, None)
    else {
        panic!("quiet pull must be unchanged")
    };
    let inviter = check_in(
        &f,
        fixture_claim(&f, "x", "px", fourth(), "inviter"),
        "ci-inviter",
    );
    let CommandResult::ThreadCreated(thread) = f
        .service
        .handle(
            Command::CreateThread(CreateThread {
                name: None,
                topic: "lazy invitation topic".into(),
                goal: "review the handoff".into(),
                operation: OperationId::new("create-lazy-invite"),
                claim: inviter.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("thread not created")
    };
    f.service
        .handle(
            Command::Invite(Invite {
                confirmation: Some(InvitationConfirmation::Lazy),
                thread: thread.clone(),
                seat: SeatId::new("s"),
                deadline_millis: None,
                operation: OperationId::new("lazy-invite"),
                claim: inviter,
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    assert_eq!(
        wake_attention(&f),
        0,
        "Lazy invitation never schedules ordinary wake work"
    );
    let first = page(pull(&f, &context, Some(quiet_version)));
    assert!(
        first.text.contains("lazy invitation topic"),
        "{}",
        first.text
    );
    assert!(first.text.contains(thread.as_str()), "{}", first.text);
    assert!(first.text.contains("accept"), "{}", first.text);
    assert!(!first.text.contains("] attention "), "{}", first.text);
    assert!(
        first.offered.is_empty(),
        "invitations are not confirmable message ids"
    );
    assert!(first.truncated.is_empty());
    assert!(
        first.text.contains("review the handoff"),
        "invitation goal reaches context: {}",
        first.text
    );
    assert!(
        matches!(
            pull(&f, &context, Some(first.attention_version)),
            DeliveryPullReply::Unchanged { .. }
        ),
        "whole passive display is not repeatedly injected"
    );
    let (state, mode): (String, String) =
        f.db.query_row(
            "SELECT state,confirmation FROM invitations WHERE thread_id=?1 AND seat_id='s'",
            [thread.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!((state.as_str(), mode.as_str()), ("pending", "lazy"));
    let attention = crate::store::attention::wake_seat_attention(&f.db, "s").unwrap();
    assert!(!attention.attention.has_pending_invitation);
    assert!(!attention.attention.has_pending_receipt);
    assert!(attention.attention.latest_warning_seq.is_none());
}

// Kills: caching an offered informational frontier as if it settled the
// original sender's independent open-condition attention.
#[test]
fn departed_receipt_origins_survive_pull_offers_late_association_and_partial_ack() {
    use crate::protocol::commands::{Ack, Leave};
    let mut f = fixture(0);
    let a = pull_seat(&f);
    let b = check_in(
        &f,
        fixture_claim(&f, "x", "px", fourth(), "origin-b"),
        "ci-origin-b",
    );
    let recipient = check_in(
        &f,
        fixture_claim(&f, "c", "pc", fourth(), "recipient"),
        "ci-recipient",
    );
    for (n, author) in [(1, "s"), (2, "s"), (3, "x")] {
        let id = format!("origin-{n}");
        f.db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,author_kind,actor_seat_id,body,decision_at,decision_seq) VALUES (?1,'i','t',?2,'ordinary','native',?3,'origin body',0,?2)", params![id,n,author]).unwrap();
        if n < 3 {
            f.db.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) VALUES (?1,'t','c','pending',1,0,500)", [&id]).unwrap();
        } else {
            f.db.execute("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES ('origin-p','i','actor','origin-p',zeroblob(32),'t',0,0,0,0,0,0,1,'sealed')", []).unwrap();
            f.db.execute("INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES ('origin-p','t','c',1,1001000,0)", []).unwrap();
            f.db.execute("INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('origin-p',?1,'i','t',3,0,3,0,1,0)", [&id]).unwrap();
            f.db.execute("INSERT INTO receipt_state(message_id,seat_id,state,deadline_at,ack_required) VALUES (?1,'c','pending',1001000,1)", [&id]).unwrap();
        }
    }
    f.db.execute("UPDATE threads SET next_sequence=1000 WHERE id='t'", [])
        .unwrap();
    f.db.execute(
        "UPDATE host_instances SET decision_seq=1000 WHERE id='i'",
        [],
    )
    .unwrap();
    for (who, operation) in [(&a, "leave-a"), (&b, "leave-b")] {
        f.service
            .handle(
                Command::Leave(Leave {
                    thread: ThreadId::new("t"),
                    operation: OperationId::new(operation),
                    claim: who.clone(),
                }),
                peer(),
                &budget(),
            )
            .unwrap();
    }
    let shared: Arc<dyn Clock> = f.clock.clone();
    let context = StoreContext::new(f.directory.join("store.db"), shared);
    crate::store::receipts::scan_due(
        &context,
        &mut f.db,
        100,
        &mut crate::store::receipts::ReceiptDueCursor::default(),
    )
    .unwrap();
    let warning: String =
        f.db.query_row(
            "SELECT open_warning_id FROM warning_conditions WHERE condition_kind='receipt'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    while crate::store::materialization::advance_work(
        &mut f.db,
        &format!("work:{warning}"),
        crate::ports::DurableWorkAdmission::new(16).unwrap(),
        &budget(),
        context.clock(),
    )
    .unwrap()
    .has_more
    {}
    let first_a = page(pull(&f, &a, None));
    assert!(first_a.text.contains("] attention "), "{}", first_a.text);
    assert!(
        matches!(
            pull(&f, &a, Some(first_a.attention_version)),
            DeliveryPullReply::Unchanged { .. }
        ),
        "whole warning offer does not force repeated context"
    );
    struct OriginNotifier(std::sync::Mutex<Vec<SeatId>>);
    impl crate::ports::NotificationPort for OriginNotifier {
        fn attempt_wake(
            &self,
            reservation: crate::ports::WakeReservation,
            _: &crate::ports::HostCallContext,
        ) -> Result<crate::ports::WakeOutcome, ApiError> {
            self.0.lock().unwrap().push(reservation.seat);
            Ok(crate::ports::WakeOutcome::Unavailable)
        }
    }
    f.db.execute(
        "UPDATE observed_targets SET ui_state='active_turn',observed_at=?1",
        [f.clock.utc_now().0],
    )
    .unwrap();
    f.db.execute(
        "UPDATE observed_targets SET ui_state='idle' WHERE target_id='p'",
        [],
    )
    .unwrap();
    let elected_boot = uuid::Uuid::new_v4();
    let wake_store: Arc<dyn StorePort> = Arc::new(
        SqliteStore::new(
            StoreContext::new(f.directory.join("store.db"), f.clock.clone()),
            "i",
            StoreSettings {
                daemon_boot: Some(elected_boot),
                ..StoreSettings::default()
            },
        )
        .unwrap(),
    );
    let scheduled =
        crate::service::workers::ScheduledStore::new(wake_store, Arc::new(FairWriter::new(32)));
    let notifier = OriginNotifier(std::sync::Mutex::new(Vec::new()));
    let scheduler = crate::scheduler::Scheduler::new(
        "i".into(),
        &scheduled,
        &scheduled,
        &notifier,
        crate::notification::policy::RetryConfig::default(),
        elected_boot,
    )
    .with_hook_pull(f.state.as_ref());
    scheduler.drive_wakes(&budget()).unwrap();
    assert!(
        notifier.0.lock().unwrap().contains(&a.seat),
        "actual canonical scheduler must retain an open origin warning after whole offer despite NextPull::Nothing"
    );

    let first_b = pull(&f, &b, None);
    let seen_b = match first_b {
        DeliveryPullReply::Page(p) => p.attention_version,
        DeliveryPullReply::Unchanged { attention_version } => attention_version,
        other => panic!("unexpected {other:?}"),
    };
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM receipt_warning_origins WHERE seat_id='x'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    f.clock.0.store(1_002_000, Ordering::SeqCst);
    crate::store::receipts::scan_due(
        &context,
        &mut f.db,
        100,
        &mut crate::store::receipts::ReceiptDueCursor::default(),
    )
    .unwrap();
    let later_b = page(pull(&f, &b, Some(seen_b)));
    assert!(later_b.text.contains("] attention "), "{}", later_b.text);
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM warning_conditions WHERE condition_kind='receipt'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    for (n, remaining) in [(1, 2), (2, 1), (3, 0)] {
        f.service
            .handle(
                Command::Ack(Ack {
                    messages: vec![MessageId::new(format!("origin-{n}"))],
                    operation: OperationId::new(format!("ack-origin-{n}")),
                    claim: recipient.clone(),
                }),
                peer(),
                &budget(),
            )
            .unwrap();
        let a_view = f
            .store
            .mod_binding_view(&a.seat, crate::ports::ModBindingRef::Claim(&a), &budget())
            .unwrap()
            .unwrap();
        let b_view = f
            .store
            .mod_binding_view(&b.seat, crate::ports::ModBindingRef::Claim(&b), &budget())
            .unwrap()
            .unwrap();
        assert_eq!(a_view.fingerprint.other_pending, u64::from(remaining > 0));
        assert_eq!(b_view.fingerprint.other_pending, u64::from(remaining > 0));
        assert_eq!(
            f.db.query_row(
                "SELECT count(*) FROM receipt_warning_origins WHERE warning_id=?1",
                [&warning],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            if remaining > 0 { 2 } else { 0 }
        );
    }
    let before_final_pass = notifier.0.lock().unwrap().len();
    scheduler.drive_wakes(&budget()).unwrap();
    assert_eq!(
        notifier.0.lock().unwrap().len(),
        before_final_pass,
        "passive final clear creates no warning-only wake"
    );
    assert_eq!(f.db.query_row("SELECT count(*) FROM warning_conditions WHERE condition_kind='receipt' AND clear_warning_id IS NOT NULL", [], |r| r.get::<_,i64>(0)).unwrap(), 1);
}

// Kills: a real canonical lazy arrival never triggering the live watch consumer.
#[test]
fn canonical_lazy_invitation_signals_a_passive_channel_drain_without_attention_history() {
    use crate::{
        ports::ModChannelSink,
        protocol::{
            commands::{CreateThread, InvitationConfirmation, Invite},
            watch::{ModDeliverySetting, WatchFrame, WatchRequest},
        },
        service::mod_channels::ModChannelRegistry,
    };
    struct Sink(std::sync::Mutex<Vec<WatchFrame>>);
    impl ModChannelSink for Sink {
        fn push(&self, frame: WatchFrame) -> bool {
            self.0.lock().unwrap().push(frame);
            true
        }
    }
    let f = fixture(0);
    let recipient = check_in(
        &f,
        fixture_claim(&f, "s", "p", Harness::Claude, "channel"),
        "channel-ci",
    );
    let inviter = check_in(
        &f,
        fixture_claim(&f, "x", "px", fourth(), "inviter"),
        "channel-inviter-ci",
    );
    let reads = Arc::new(
        SqliteStore::new(
            StoreContext::new(f.directory.join("store.db"), f.clock.clone()),
            "i",
            StoreSettings::default(),
        )
        .unwrap(),
    );
    let registry = Arc::new(ModChannelRegistry::new(
        f.clock.clone(),
        reads,
        "i".into(),
        Arc::new(|| {}),
        ModDeliverySetting::On,
    ));
    let service = DomainService::new("i".into(), f.store.clone(), f.clock.clone())
        .with_cooperative_owner(501, Arc::new(FairWriter::new(32)))
        .with_mod_channels(registry.clone());
    let sink = Arc::new(Sink(std::sync::Mutex::new(Vec::new())));
    service
        .watch_register(
            &WatchRequest {
                presentation_v2: true,
                claim: recipient.clone(),
            },
            sink.clone(),
            &budget(),
        )
        .unwrap();
    registry.pass(f.clock.utc_now()).unwrap();
    assert!(sink.0.lock().unwrap().is_empty());
    let before = f
        .store
        .mod_binding_view(
            &recipient.seat,
            crate::ports::ModBindingRef::Claim(&recipient),
            &budget(),
        )
        .unwrap()
        .unwrap();
    let CommandResult::ThreadCreated(thread) = service
        .handle(
            Command::CreateThread(CreateThread {
                name: None,
                topic: "channel invitation".into(),
                goal: "review channel".into(),
                operation: OperationId::new("channel-create"),
                claim: inviter.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("thread not created")
    };
    service
        .handle(
            Command::Invite(Invite {
                confirmation: Some(InvitationConfirmation::Lazy),
                thread,
                seat: recipient.seat.clone(),
                deadline_millis: None,
                operation: OperationId::new("channel-invite"),
                claim: inviter,
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    let after = f
        .store
        .mod_binding_view(
            &recipient.seat,
            crate::ports::ModBindingRef::Claim(&recipient),
            &budget(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        before.fingerprint, after.fingerprint,
        "passive content never changes wake eligibility"
    );
    registry.pass(f.clock.utc_now()).unwrap();
    assert_eq!(
        sink.0.lock().unwrap().len(),
        1,
        "actual channel receives a drain signal"
    );
    registry.pass(f.clock.utc_now()).unwrap();
    assert_eq!(
        sink.0.lock().unwrap().len(),
        1,
        "unchanged canonical content does not flood the channel"
    );
}

#[test]
fn canonical_lazy_invitation_content_changes_on_accept_reject_and_retirement_only() {
    use crate::protocol::commands::{
        Accept, CreateThread, InvitationConfirmation, Invite, OperatorRetire, Reject,
    };
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let inviter = check_in(
        &f,
        fixture_claim(&f, "x", "px", fourth(), "version-inviter"),
        "version-inviter-ci",
    );
    let create = |suffix: &str, invite: bool| {
        let CommandResult::ThreadCreated(thread) = f
            .service
            .handle(
                Command::CreateThread(CreateThread {
                    name: None,
                    topic: suffix.into(),
                    goal: "review".into(),
                    operation: OperationId::new(format!("version-create-{suffix}")),
                    claim: inviter.clone(),
                }),
                peer(),
                &budget(),
            )
            .unwrap()
        else {
            panic!("not created")
        };
        let invitation = if invite {
            let CommandResult::Invitation(id) = f
                .service
                .handle(
                    Command::Invite(Invite {
                        confirmation: Some(InvitationConfirmation::Lazy),
                        thread: thread.clone(),
                        seat: recipient.seat.clone(),
                        deadline_millis: None,
                        operation: OperationId::new(format!("version-invite-{suffix}")),
                        claim: inviter.clone(),
                    }),
                    peer(),
                    &budget(),
                )
                .unwrap()
            else {
                panic!("not invited")
            };
            Some(id)
        } else {
            None
        };
        (thread, invitation)
    };
    let version = || {
        f.store
            .mod_binding_view(
                &recipient.seat,
                crate::ports::ModBindingRef::Claim(&recipient),
                &budget(),
            )
            .unwrap()
            .unwrap()
            .pending_invitation_version
    };
    assert_eq!(version(), 0);
    let (accepted, _) = create("accepted", true);
    let (rejected, rejection) = create("rejected", true);
    let (newest, _) = create("newest", true);
    let initial = version();
    let shown = page(pull(&f, &recipient, None));
    create("unrelated", false);
    assert_eq!(
        version(),
        initial,
        "unrelated real decisions do not change recipient content"
    );
    assert!(matches!(
        pull(&f, &recipient, Some(shown.attention_version)),
        DeliveryPullReply::Unchanged { .. }
    ));
    f.service
        .handle(
            Command::Accept(Accept {
                thread: accepted,
                operation: OperationId::new("version-accept"),
                claim: recipient.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    let after_accept = version();
    assert_ne!(
        after_accept, initial,
        "older acceptance changes content despite unchanged newest pending invitation"
    );
    f.service
        .handle(
            Command::Reject(Reject {
                thread: rejected,
                invitation: rejection.unwrap(),
                reason: "outside remit".into(),
                operation: OperationId::new("version-reject"),
                claim: recipient.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    assert_ne!(
        version(),
        after_accept,
        "older rejection removes canonical content"
    );
    let before_topic = version();
    f.service
        .handle(
            Command::SetTopic(crate::protocol::commands::SetTopic {
                thread: newest,
                topic: "updated invitation decision context".into(),
                operation: OperationId::new("version-topic"),
                claim: inviter.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    assert_ne!(
        version(),
        before_topic,
        "canonical topic revision invalidates a previously whole-offered invitation"
    );
    let changed = page(pull(&f, &recipient, Some(shown.attention_version)));
    assert!(changed.text.contains("updated invitation decision context"));
    assert!(
        !changed.text.contains("accepted"),
        "settled/unchanged invitation contexts do not flood the next page"
    );
    f.store
        .mutate_operator(
            crate::ports::OperatorRequest::Retire(OperatorRetire {
                seat: recipient.seat.clone(),
                operation: OperationId::new("version-retire"),
            }),
            crate::protocol::authority::OperatorActor::from_peer(peer(), 501).unwrap(),
            &budget(),
        )
        .unwrap();
    assert_eq!(
        version(),
        0,
        "retirement removes recipient-visible pending content"
    );
}

fn canonical_lazy_invitation_backlog(
    f: &Fixture,
    recipient: &CallerClaim,
    inviter: &CallerClaim,
    count: usize,
    prefix: &str,
) -> ThreadId {
    use crate::protocol::commands::{CreateThread, InvitationConfirmation, Invite};
    let mut oldest = None;
    for n in 0..count {
        let CommandResult::ThreadCreated(thread) = f
            .service
            .handle(
                Command::CreateThread(CreateThread {
                    name: None,
                    topic: format!("{prefix}-topic-{n}"),
                    goal: "review".into(),
                    operation: OperationId::new(format!("{prefix}-create-{n}")),
                    claim: inviter.clone(),
                }),
                peer(),
                &budget(),
            )
            .unwrap()
        else {
            panic!("thread not created")
        };
        oldest.get_or_insert_with(|| thread.clone());
        f.service
            .handle(
                Command::Invite(Invite {
                    confirmation: Some(InvitationConfirmation::Lazy),
                    thread,
                    seat: recipient.seat.clone(),
                    deadline_millis: None,
                    operation: OperationId::new(format!("{prefix}-invite-{n}")),
                    claim: inviter.clone(),
                }),
                peer(),
                &budget(),
            )
            .unwrap();
    }
    oldest.expect("nonempty canonical backlog")
}

// These saturation controls intentionally exercise the capable route. The
// historical V1 REDs remain evidence of the unsupported old cache limit.
fn presentation_test_pull(
    f: &Fixture,
    claim: &CallerClaim,
) -> crate::protocol::delivery::DeliveryPullReplyV2 {
    let result = f
        .service
        .handle(
            Command::DeliveryPullV2(crate::protocol::delivery::DeliveryPullV2 {
                claim: claim.clone(),
                consumer: crate::protocol::delivery::PresentationConsumer::Hook,
                render: crate::protocol::delivery::PresentationRenderRequest::Hook {
                    confirmable_selector: 0,
                    passive_selector: 0,
                },
                presentation_lease: None,
                marker_prefix: Vec::new(),
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    let CommandResult::DeliveryPulledV2(reply) = result else {
        panic!("wrong presentation result {result:?}")
    };
    reply
}
fn presentation_invitations(
    page: &crate::protocol::delivery::DeliveryPageV2,
) -> Vec<crate::protocol::delivery::PresentationInvitation> {
    page.items
        .iter()
        .filter_map(|v| match v {
            crate::protocol::delivery::PresentationItemV2::Invitation { id, content } => {
                Some(crate::protocol::delivery::PresentationInvitation {
                    id: id.clone(),
                    content: content.clone(),
                })
            }
            _ => None,
        })
        .collect()
}
fn presentation_watch_frozen(
    f: &Fixture,
    claim: &CallerClaim,
) -> Option<crate::delivery::presentation::FrozenPresentationRender> {
    f.store
        .presentation_render_selection(
            claim,
            crate::protocol::delivery::PresentationConsumer::Watch,
            &budget(),
        )
        .unwrap()
        .map(
            |identity| crate::delivery::presentation::FrozenPresentationRender {
                identity,
                codec: None,
            },
        )
}
fn presentation_test_confirm(
    f: &Fixture,
    claim: &CallerClaim,
    page: &crate::protocol::delivery::DeliveryPageV2,
) {
    if let Some(token) = &page.presentation_token {
        let result = f
            .service
            .handle(
                Command::DeliveryPresentationConfirmV2(
                    crate::protocol::delivery::DeliveryPresentationConfirmV2 {
                        claim: claim.clone(),
                        consumer: crate::protocol::delivery::PresentationConsumer::Hook,
                        token: token.clone(),
                        emitted_controls: page.emitted_controls.clone(),
                        presentation_lease: None,
                    },
                ),
                peer(),
                &budget(),
            )
            .unwrap();
        assert!(matches!(
            result,
            CommandResult::DeliveryPresentationConfirmedV2(
                crate::protocol::delivery::DeliveryPresentationConfirmationReplyV2 {
                    status:
                        crate::protocol::delivery::DeliveryPresentationConfirmationV2::Confirmed,
                    ..
                }
            )
        ));
    }
}

// Kills stale command hydration after real attach/release, historical-link reuse,
// and borrowing another invitation's current requirement when both remain pending.
// This is the canonical service/render boundary, not a registered native hook.
#[test]
fn presentation_v2_service_requirement_attach_release_fresh_replacement_keeps_exact_commands() {
    use crate::{
        ports::{
            ServiceAuthorityGate, ServiceConnectionAuthority, ServiceDecisionGuard,
            ServiceWriteTransactionProof,
        },
        protocol::{
            delivery::{
                DeliveryPresentationConfirmV2, DeliveryPresentationConfirmationV2,
                DeliveryPullReplyV2, PresentationConsumer, PresentationItemV2,
            },
            results::ApiError,
            service::{
                EnsureManagedThread, InvitationConstraint, ReleaseRequirement, RequiredMembership,
                RequirementState, ServiceInvite, ServiceOperation, ServiceResult,
            },
        },
    };
    struct Gate(ServiceAuthorId);
    struct Guard(ServiceAuthorId);
    impl ServiceDecisionGuard for Guard {
        fn author(&self) -> &ServiceAuthorId {
            &self.0
        }
    }
    impl ServiceAuthorityGate for Gate {
        fn register(
            &self,
            instance: &str,
            boot: &str,
            author: ServiceAuthorId,
        ) -> Result<ServiceConnectionAuthority, ApiError> {
            Ok(ServiceConnectionAuthority::new(
                instance.into(),
                boot.into(),
                1,
                author,
            ))
        }
        fn decision_guard<'a>(
            &'a self,
            _: &ServiceWriteTransactionProof,
            connection: &ServiceConnectionAuthority,
        ) -> Result<Box<dyn ServiceDecisionGuard + 'a>, ApiError> {
            assert_eq!(connection.author(), &self.0);
            Ok(Box::new(Guard(self.0.clone())))
        }
        fn revoke_exact(&self, _: &ServiceConnectionAuthority) -> bool {
            true
        }
    }

    let f = fixture(0);
    let recipient = pull_seat(&f);
    let tx = f.db.unchecked_transaction().unwrap();
    let author =
        crate::store::service_substrate::ensure_reserved_author(&tx, "i", UtcMillis(0)).unwrap();
    tx.commit().unwrap();
    let gate = Gate(author.clone());
    let connection = gate.register("i", "boot", author).unwrap();
    let operate = |operation| {
        f.service
            .service_operation(operation, &connection, &gate, &budget())
            .unwrap()
    };
    let thread = ThreadId::new("required-command-context");
    const TOPIC: &str = "Review every retained invitation command";
    const GOAL: &str =
        "Preserve ordinary timing and require the exact current participation revision";
    assert!(matches!(
        operate(ServiceOperation::EnsureThread(EnsureManagedThread {
            thread: thread.clone(),
            topic: TOPIC.into(),
            goal: GOAL.into(),
            operation: OperationId::new("required-command-thread"),
        })),
        ServiceResult::ThreadEnsured(_)
    ));
    let invite = |constraint, duration, operation: &str| {
        let ServiceResult::Invitation(invitation) =
            operate(ServiceOperation::Invite(ServiceInvite {
                thread: thread.clone(),
                seat: recipient.seat.clone(),
                constraint,
                deadline_millis: Some(duration),
                operation: OperationId::new(operation),
            }))
        else {
            panic!("real service invitation")
        };
        invitation
    };
    // Full rows are independent witnesses; Pull/Confirm may change presentation
    // bookkeeping, but may not accept, ACK, cancel, or complete an obligation.
    let rows = |table: &str| {
        let mut statement =
            f.db.prepare(&format!(
                "SELECT * FROM {table} WHERE thread_id=?1 ORDER BY rowid"
            ))
            .unwrap();
        statement
            .query_map([thread.as_str()], |row| {
                (0..row.as_ref().column_count())
                    .map(|column| row.get::<_, rusqlite::types::Value>(column))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    let obligations = || {
        [
            "invitations",
            "requirement_episodes",
            "memberships",
            "membership_intervals",
            "receipts",
            "lazy_recipients",
        ]
        .into_iter()
        .map(&rows)
        .collect::<Vec<_>>()
    };
    let epoch = |id: &InvitationId| {
        f.db.query_row("SELECT i.command_context_epoch,coalesce(c.context_revision,0),coalesce(c.last_change_seq,0) FROM invitations i LEFT JOIN invitation_context_latest c ON c.thread_id=i.thread_id WHERE i.id=?1",
            [id.as_str()], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?))).unwrap()
    };
    let timing = |id: &InvitationId| {
        f.db.query_row(
            "SELECT episode,created_at,frozen_duration_ms,deadline_at FROM invitations WHERE id=?1",
            [id.as_str()],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            },
        )
        .unwrap()
    };
    let assert_pending = |invitations: i64, requirements: i64| {
        let counts = f.db.query_row("SELECT (SELECT count(*) FROM invitations WHERE thread_id=?1 AND state='pending'),(SELECT count(*) FROM requirement_episodes WHERE thread_id=?1 AND state='pending'),(SELECT count(*) FROM membership_intervals WHERE thread_id=?1),(SELECT count(*) FROM receipts WHERE thread_id=?1),(SELECT count(*) FROM lazy_recipients WHERE thread_id=?1),(SELECT count(*) FROM invitation_cancellations c JOIN invitations i ON i.id=c.invitation_id WHERE i.thread_id=?1)",
            [thread.as_str()], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?,r.get::<_,i64>(4)?,r.get::<_,i64>(5)?))).unwrap();
        assert_eq!(counts, (invitations, requirements, 0, 0, 0, 0));
        let membership =
            f.db.query_row(
                "SELECT state,voluntary_state FROM memberships WHERE thread_id=?1 AND seat_id=?2",
                params![thread.as_str(), recipient.seat.as_str()],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .unwrap();
        assert_eq!(membership, ("invited".into(), "invited".into()));
    };
    let present =
        |claim: &CallerClaim, expected: &[(&InvitationId, Option<&RequiredMembership>, i64)]| {
            let before = obligations();
            let wake_before = wake_attention(&f);
            assert!(
                wake_before > 0,
                "timed/required invitations create real wake work"
            );
            let mut seen = std::collections::HashSet::new();
            // At most two invitations, no ordinary backlog, no elapsed-time retry.
            for _ in 0..8 {
                let DeliveryPullReplyV2::Page(page) = presentation_test_pull(&f, claim) else {
                    panic!("current command context must have a canonical V2 page")
                };
                assert!(page.items.len() <= 32 && page.output_bytes <= 65_536);
                for item in &page.items {
                    let PresentationItemV2::Invitation { id, content } = item else {
                        continue;
                    };
                    let (_, required, deadline) = expected
                        .iter()
                        .find(|(wanted, _, _)| *wanted == id)
                        .expect("only this phase's pending invitations are offered");
                    assert_eq!(content.thread, thread);
                    assert!(!content.truncated);
                    assert!(content.wakes);
                    let (topic, goal, revision): (String, String, i64) =
                        f.db.query_row(
                            "SELECT topic,goal,topic_revision FROM threads WHERE id=?1",
                            [thread.as_str()],
                            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                        )
                        .unwrap();
                    assert_eq!((topic.as_str(), goal.as_str()), (TOPIC, GOAL));
                    assert_eq!(content.topic_revision, Some(revision.to_string()));
                    assert_eq!(content.command_context_epoch, Some(epoch(id).0.to_string()));
                    let command = if let Some(required) = required {
                        let canonical: (String,i64,String) = f.db.query_row("SELECT invitation_id,revision,state FROM requirement_episodes WHERE id=?1 AND thread_id=?2 AND seat_id=?3",
                        params![required.requirement.as_str(),thread.as_str(),claim.seat.as_str()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
                        assert_eq!(
                            canonical,
                            (
                                id.as_str().into(),
                                required.revision as i64,
                                "pending".into()
                            )
                        );
                        assert_eq!(&required.invitation, id);
                        assert_eq!(required.state, RequirementState::Pending);
                        assert_eq!(required.accepted_by, None);
                        assert_eq!(required.accepted_at, None);
                        format!(
                            "herdr-threads accept-required {} --invitation {} --requirement {} --revision {}",
                            thread.as_str(),
                            id.as_str(),
                            required.requirement.as_str(),
                            required.revision
                        )
                    } else {
                        format!("herdr-threads accept {}", thread.as_str())
                    };
                    let expected_body =
                        format!("{TOPIC}\ngoal: {GOAL}\ndeadline: {deadline}\nnext: {command}");
                    assert_eq!(content.body, expected_body);
                    let actual_command = content
                        .body
                        .lines()
                        .find_map(|line| line.strip_prefix("next: "))
                        .unwrap();
                    let argv =
                        shlex::split(actual_command).expect("rendered command is shell argv");
                    assert_eq!(
                        argv,
                        command
                            .split_whitespace()
                            .map(str::to_owned)
                            .collect::<Vec<_>>()
                    );
                    crate::cli::commands::parse_argv(argv)
                        .expect("rendered argv parses without executing acceptance");
                    assert!(page.text.contains(&format!(
                        "[herdr-threads] invitation {} in {}",
                        id.as_str(),
                        thread.as_str()
                    )));
                    assert!(
                        page.text
                            .contains(&format!("  {}", expected_body.replace('\n', "\n  ")))
                    );
                    assert!(
                        seen.insert(id.clone()),
                        "one complete current command offer per invitation"
                    );
                }
                if let Some(token) = &page.presentation_token {
                    // Explicitly confirm this rendered page and its current controls;
                    // no acceptance, receipt ACK, or transcript completion is issued.
                    let CommandResult::DeliveryPresentationConfirmedV2(confirmed) = f
                        .service
                        .handle(
                            Command::DeliveryPresentationConfirmV2(DeliveryPresentationConfirmV2 {
                                claim: claim.clone(),
                                consumer: PresentationConsumer::Hook,
                                token: token.clone(),
                                emitted_controls: page.emitted_controls.clone(),
                                presentation_lease: None,
                            }),
                            peer(),
                            &budget(),
                        )
                        .unwrap()
                    else {
                        panic!("structured V2 confirmation")
                    };
                    assert_eq!(
                        confirmed.status,
                        DeliveryPresentationConfirmationV2::Confirmed
                    );
                    assert_eq!(confirmed.acknowledged_controls, page.emitted_controls);
                    let consumer: (Option<String>,String,i64,String) = f.db.query_row("SELECT outbox_token,last_token,generation,execution FROM invitation_presentation_consumers WHERE binding_id=?1 AND consumer='hook'",
                    [binding_for_claim(&f, claim).as_str()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
                    assert_eq!(
                        consumer,
                        (
                            None,
                            token.clone(),
                            claim.binding_generation as i64,
                            claim.execution.as_str().into()
                        )
                    );
                } else {
                    assert!(
                        page.items.is_empty(),
                        "rendered invitations require an explicit token"
                    );
                }
                assert_eq!(
                    obligations(),
                    before,
                    "presentation never settles participation or messages"
                );
                assert_eq!(
                    wake_attention(&f),
                    wake_before,
                    "presentation does not erase pending wake work"
                );
                if seen.len() == expected.len() {
                    for (id, _, _) in expected {
                        let complete: i64 = f.db.query_row("SELECT complete_count FROM invitation_presentation_items WHERE binding_id=?1 AND consumer='hook' AND invitation_id=?2 AND generation=?3 AND execution=?4",
                        params![binding_for_claim(&f, claim).as_str(),id.as_str(),claim.binding_generation as i64,claim.execution.as_str()], |r| r.get(0)).unwrap();
                        assert_eq!(complete, 1, "current execution records a complete offer");
                    }
                    return;
                }
            }
            panic!("finite canonical command context did not drain: {seen:?}");
        };

    let ordinary = invite(
        InvitationConstraint::Ordinary,
        60_000,
        "required-command-ordinary",
    );
    assert_eq!(ordinary.requirement, None);
    let original_timing = timing(&ordinary.invitation);
    assert_eq!(original_timing, (1, 1_000_000, 60_000, 1_060_000));
    let initial = epoch(&ordinary.invitation);
    assert_eq!(initial.0, 0);
    assert_pending(1, 0);
    present(&recipient, &[(&ordinary.invitation, None, 1_060_000)]);

    let attached = invite(
        InvitationConstraint::Required,
        90_000,
        "required-command-attach",
    );
    assert_eq!(
        attached.invitation, ordinary.invitation,
        "first attachment reuses the unreferenced timed invitation"
    );
    let requirement = attached.requirement.as_ref().unwrap();
    assert_eq!(requirement.revision, 1);
    assert_eq!(timing(&ordinary.invitation), original_timing);
    let attached_epoch = epoch(&ordinary.invitation);
    assert_eq!(attached_epoch.0, initial.0 + 1);
    assert_eq!(attached_epoch.1, initial.1 + 1);
    assert!(attached_epoch.2 > initial.2);
    assert_pending(1, 1);
    present(
        &recipient,
        &[(&ordinary.invitation, Some(requirement), 1_060_000)],
    );

    let ServiceResult::RequirementReleased(released) =
        operate(ServiceOperation::ReleaseRequirement(ReleaseRequirement {
            thread: thread.clone(),
            seat: recipient.seat.clone(),
            requirement: requirement.requirement.clone(),
            operation: OperationId::new("required-command-release"),
        }))
    else {
        panic!("real release result")
    };
    assert_eq!(released.state, RequirementState::Released);
    assert_eq!(released.revision, 2);
    assert_eq!(released.invitation, ordinary.invitation);
    let released_row: (String, i64, String) =
        f.db.query_row(
            "SELECT state,revision,invitation_id FROM requirement_episodes WHERE id=?1",
            [requirement.requirement.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        released_row,
        ("released".into(), 2, ordinary.invitation.as_str().into())
    );
    assert_eq!(timing(&ordinary.invitation), original_timing);
    let released_epoch = epoch(&ordinary.invitation);
    assert_eq!(released_epoch.0, attached_epoch.0 + 1);
    assert_eq!(released_epoch.1, attached_epoch.1 + 1);
    assert!(released_epoch.2 > attached_epoch.2);
    assert_pending(1, 0);
    present(&recipient, &[(&ordinary.invitation, None, 1_060_000)]);
    let old_invitation = rows("invitations");
    let old_requirement = rows("requirement_episodes");

    let replacement = invite(
        InvitationConstraint::Required,
        90_000,
        "required-command-replacement",
    );
    let replacement_requirement = replacement.requirement.as_ref().unwrap();
    assert_ne!(replacement.invitation, ordinary.invitation);
    assert_ne!(replacement_requirement.requirement, requirement.requirement);
    assert_eq!(replacement_requirement.revision, 1);
    assert_eq!(
        timing(&replacement.invitation),
        (2, 1_000_000, 90_000, 1_090_000)
    );
    assert_eq!(epoch(&replacement.invitation).0, 1);
    let replaced_epoch = epoch(&ordinary.invitation);
    assert_eq!(
        replaced_epoch.0, released_epoch.0,
        "old command stays ordinary"
    );
    assert_eq!(replaced_epoch.1, released_epoch.1 + 1);
    assert!(replaced_epoch.2 > released_epoch.2);
    assert_eq!(&rows("invitations")[..1], old_invitation.as_slice());
    assert_eq!(
        &rows("requirement_episodes")[..1],
        old_requirement.as_slice()
    );
    assert_pending(2, 1);
    present(
        &recipient,
        &[(
            &replacement.invitation,
            Some(replacement_requirement),
            1_090_000,
        )],
    );

    // A confirmed unchanged old command is not duplicated in that execution.
    // Real CheckIn rotation makes both pending items newly visible together.
    let before_rotation = obligations();
    let original_binding = binding_for_claim(&f, &recipient);
    let original_consumer = || {
        f.db.query_row(
            "SELECT * FROM invitation_presentation_consumers WHERE binding_id=?1 AND consumer='hook'",
            [original_binding.as_str()],
            |row| (0..row.as_ref().column_count())
                .map(|column| row.get::<_, rusqlite::types::Value>(column))
                .collect::<rusqlite::Result<Vec<_>>>(),
        ).unwrap()
    };
    let before_consumer = original_consumer();
    let rotated = rotate(&f, &recipient, "required-command-fresh-session", 2);
    assert_ne!(binding_for_claim(&f, &rotated), original_binding);
    assert_ne!(rotated.execution, recipient.execution);
    assert_eq!(rotated.seat, recipient.seat);
    assert_eq!(obligations(), before_rotation);
    present(
        &rotated,
        &[
            (&ordinary.invitation, None, 1_060_000),
            (
                &replacement.invitation,
                Some(replacement_requirement),
                1_090_000,
            ),
        ],
    );
    assert_eq!(
        original_consumer(),
        before_consumer,
        "fresh binding presentation preserves the old consumer row and token"
    );
    // Parsing the ordinary command is not a claim that native acceptance may
    // bypass the still-pending required invitation; neither command is run.
    assert_eq!(&rows("invitations")[..1], old_invitation.as_slice());
    assert_eq!(
        &rows("requirement_episodes")[..1],
        old_requirement.as_slice()
    );
    assert_eq!(epoch(&ordinary.invitation), replaced_epoch);
    assert_pending(2, 1);
}

#[test]
fn an_old_visible_invitation_topic_change_survives_a_saturated_newest_window() {
    use crate::protocol::commands::SetTopic;
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let inviter = check_in(
        &f,
        fixture_claim(&f, "x", "px", fourth(), "saturated-inviter"),
        "saturated-inviter-ci",
    );
    // The smallest backlog with a visible invitation outside WINDOW1001.
    let oldest = canonical_lazy_invitation_backlog(
        &f,
        &recipient,
        &inviter,
        crate::store::attention::WINDOW + 1,
        "window",
    );

    let mut exhausted = false;
    let mut saw_oldest = false;
    for _ in 0..40 {
        let crate::protocol::delivery::DeliveryPullReplyV2::Page(shown) =
            presentation_test_pull(&f, &recipient)
        else {
            panic!("expected finite presentation page")
        };
        saw_oldest |= shown.text.contains("window-topic-0");
        presentation_test_confirm(&f, &recipient, &shown);
        if !shown.more {
            exhausted = true;
            break;
        }
    }
    assert!(
        exhausted,
        "finite1002 invitation display must make progress"
    );
    assert!(saw_oldest, "the old context was actually displayed");
    assert_eq!(f.db.query_row("SELECT count(*) FROM invitation_presentation_items WHERE seat_id='s' AND consumer='hook' AND complete_count=1",[],|r|r.get::<_,i64>(0)).unwrap(),(crate::store::attention::WINDOW+1) as i64,"every canonical context completed before quiet");
    assert!(matches!(
        presentation_test_pull(&f, &recipient),
        crate::protocol::delivery::DeliveryPullReplyV2::Unchanged { .. }
    ));
    f.service
        .handle(
            Command::SetTopic(SetTopic {
                thread: oldest,
                topic: "changed oldest visible invitation context".into(),
                operation: OperationId::new("window-change-oldest"),
                claim: inviter,
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    let crate::protocol::delivery::DeliveryPullReplyV2::Page(changed) =
        presentation_test_pull(&f, &recipient)
    else {
        panic!("expected changed canonical context")
    };
    assert!(
        changed
            .text
            .contains("changed oldest visible invitation context"),
        "{}",
        changed.text
    );
    assert!(changed.offered.is_empty());
    assert!(changed.truncated.is_empty());
    presentation_test_confirm(&f, &recipient, &changed);
    assert!(matches!(
        presentation_test_pull(&f, &recipient),
        crate::protocol::delivery::DeliveryPullReplyV2::Unchanged { .. }
    ));
}

#[test]
fn a_finite_invitation_backlog_progresses_beyond_the_whole_display_cache() {
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let inviter = check_in(
        &f,
        fixture_claim(&f, "x", "px", fourth(), "capacity-inviter"),
        "capacity-inviter-ci",
    );
    let count = crate::delivery::ledger::MAX_INVITATION_DISPLAYS_PER_BINDING + 1;
    canonical_lazy_invitation_backlog(&f, &recipient, &inviter, count, "capacity");

    let mut saw_final = false;
    let mut reached_quiet = false;
    for _ in 0..40 {
        match presentation_test_pull(&f, &recipient) {
            crate::protocol::delivery::DeliveryPullReplyV2::Page(shown) => {
                saw_final |= shown
                    .text
                    .contains(&format!("capacity-topic-{}", count - 1));
                presentation_test_confirm(&f, &recipient, &shown);
                assert!(shown.offered.is_empty());
                assert!(shown.truncated.is_empty());
            }
            crate::protocol::delivery::DeliveryPullReplyV2::Unchanged { .. } => {
                reached_quiet = true;
                break;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!(
        saw_final,
        "finite1025 backlog must display its last invitation before quiet"
    );
    assert!(reached_quiet, "whole context is not repeatedly flooded");
    assert_eq!(f.db.query_row("SELECT count(*) FROM invitation_presentation_items WHERE seat_id='s' AND consumer='hook' AND complete_count=1",[],|r|r.get::<_,i64>(0)).unwrap(),count as i64,"every canonical context completed before quiet");
    assert_eq!(f.db.query_row(
        "SELECT count(*) FROM invitations WHERE seat_id='s' AND state='pending' AND confirmation='lazy'",
        [], |r| r.get::<_, i64>(0),
    ).unwrap(), count as i64);
    assert_eq!(wake_attention(&f), 0);
}

#[test]
fn a_departed_inviter_remains_actionable_after_whole_deadline_warning_until_acceptance() {
    use crate::protocol::commands::{Accept, CreateThread, InvitationConfirmation, Invite, Leave};
    let mut f = fixture(0);
    let inviter = pull_seat(&f);
    let recipient = check_in(
        &f,
        fixture_claim(&f, "x", "px", fourth(), "invited-recipient"),
        "invited-recipient-ci",
    );
    let CommandResult::ThreadCreated(thread) = f
        .service
        .handle(
            Command::CreateThread(CreateThread {
                name: None,
                topic: "inviter deadline origin".into(),
                goal: "review".into(),
                operation: OperationId::new("inviter-origin-create"),
                claim: inviter.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("not created")
    };
    let CommandResult::Invitation(invitation) = f
        .service
        .handle(
            Command::Invite(Invite {
                confirmation: Some(InvitationConfirmation::RequireAck),
                thread: thread.clone(),
                seat: recipient.seat.clone(),
                deadline_millis: Some(1000),
                operation: OperationId::new("inviter-origin-invite"),
                claim: inviter.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("not invited")
    };
    f.service
        .handle(
            Command::Leave(Leave {
                thread: thread.clone(),
                operation: OperationId::new("inviter-origin-leave"),
                claim: inviter.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    let _ = pull(&f, &inviter, None);
    f.clock.0.store(1_001_001, Ordering::SeqCst);
    let context = StoreContext::new(f.directory.join("store.db"), f.clock.clone());
    let expired =
        crate::store::invitation_due::scan_invitation_due_batch(&context, &mut f.db, None, 100)
            .unwrap();
    assert_eq!(expired.warnings_added, 1);
    let warning: String = f.db.query_row(
        "SELECT open_warning_id FROM warning_conditions WHERE condition_kind='invitation' AND condition_id=?1",
        [invitation.as_str()], |r| r.get(0),
    ).unwrap();
    while crate::store::materialization::advance_work(
        &mut f.db,
        &format!("work:{warning}"),
        crate::ports::DurableWorkAdmission::new(16).unwrap(),
        &budget(),
        context.clock(),
    )
    .unwrap()
    .has_more
    {}
    let first = page(pull(&f, &inviter, None));
    assert!(first.text.contains("] attention "), "{}", first.text);
    assert!(matches!(
        pull(&f, &inviter, Some(first.attention_version)),
        DeliveryPullReply::Unchanged { .. }
    ));
    assert_eq!(
        f.store
            .mod_binding_view(
                &inviter.seat,
                crate::ports::ModBindingRef::Claim(&inviter),
                &budget()
            )
            .unwrap()
            .unwrap()
            .fingerprint
            .other_pending,
        1,
        "whole display does not consume the original departed inviter's condition"
    );
    f.service
        .handle(
            Command::Accept(Accept {
                thread,
                operation: OperationId::new("inviter-origin-accept"),
                claim: recipient,
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    assert_eq!(
        f.store
            .mod_binding_view(
                &inviter.seat,
                crate::ports::ModBindingRef::Claim(&inviter),
                &budget()
            )
            .unwrap()
            .unwrap()
            .fingerprint
            .other_pending,
        0
    );
    assert_eq!(f.db.query_row(
        "SELECT count(*) FROM warning_conditions WHERE condition_kind='invitation' AND condition_id=?1 AND open_warning_id=?2 AND clear_warning_id IS NOT NULL",
        params![invitation.as_str(), warning], |r| r.get::<_, i64>(0),
    ).unwrap(), 1, "one immutable warning lifetime clears on actual acceptance");
}

#[test]
fn presentation_v2_replays_one_canonical_invitation_until_emit_confirmation() {
    use serde_json::json;
    let mut f = fixture(0);
    let recipient = pull_seat(&f);
    let inviter = check_in(
        &f,
        fixture_claim(&f, "x", "px", fourth(), "v2-inviter"),
        "v2-inviter-ci",
    );
    canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 1, "durable-v2");
    let wire = json!({"kind":"delivery_pull_v2","args":{
        "claim":recipient,"consumer":"hook",
        "render":{"kind":"hook","confirmable_selector":0,"passive_selector":0}
    }});
    let command = serde_json::from_value::<Command>(wire.clone());
    assert!(
        command.is_ok(),
        "actual canonical service needs the V2 route: {command:?}"
    );
    let result = f
        .service
        .handle(command.unwrap(), peer(), &budget())
        .unwrap();
    let first = serde_json::to_value(result).unwrap();
    assert_eq!(first["kind"], "delivery_pulled_v2");
    assert_eq!(first["data"]["kind"], "page");
    let page = &first["data"]["args"];
    assert!(
        page["text"]
            .as_str()
            .unwrap()
            .contains("durable-v2-topic-0")
    );
    let items = page["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["kind"], "invitation");
    // This ordered invitation-only page emits no Attention or Cleared controls.
    let expected_controls = json!([]);
    assert_eq!(page["emitted_controls"], expected_controls);
    let token = page["presentation_token"].as_str().unwrap().to_owned();
    assert_eq!(token.len(), 32);
    assert!(crate::protocol::delivery::valid_presentation_token(&token));
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM invitations WHERE state='pending'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    // A lost reply and service restart cannot consume the invitation offer.
    restart(&mut f);
    let replay = f
        .service
        .handle(
            serde_json::from_value(wire.clone()).unwrap(),
            peer(),
            &budget(),
        )
        .unwrap();
    assert_eq!(serde_json::to_value(replay).unwrap(), first);
    let wrong = json!({"kind":"delivery_presentation_confirm_v2","args":{
        "claim":recipient,"consumer":"hook","token":"ffffffffffffffffffffffffffffffff",
        "emitted_controls":expected_controls
    }});
    let wrong = f
        .service
        .handle(serde_json::from_value(wrong).unwrap(), peer(), &budget())
        .unwrap();
    let wrong = serde_json::to_value(wrong).unwrap();
    assert_eq!(wrong["data"]["status"], "not_current_page");
    assert_eq!(wrong["data"]["acknowledged_controls"], expected_controls);
    let confirmation = json!({"kind":"delivery_presentation_confirm_v2","args":{
        "claim":recipient,"consumer":"hook","token":token,
        "emitted_controls":expected_controls
    }});
    let settled = f
        .service
        .handle(
            serde_json::from_value(confirmation.clone()).unwrap(),
            peer(),
            &budget(),
        )
        .unwrap();
    let settled = serde_json::to_value(settled).unwrap();
    assert_eq!(settled["data"]["status"], "confirmed");
    assert_eq!(settled["data"]["acknowledged_controls"], expected_controls);
    let duplicate = f
        .service
        .handle(
            serde_json::from_value(confirmation).unwrap(),
            peer(),
            &budget(),
        )
        .unwrap();
    let duplicate = serde_json::to_value(duplicate).unwrap();
    assert_eq!(duplicate["data"]["status"], "already_confirmed");
    assert_eq!(
        duplicate["data"]["acknowledged_controls"],
        expected_controls
    );
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM invitations WHERE state='pending'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1,
        "presentation never accepts or ACKs"
    );
    assert_eq!(
        f.db.query_row("SELECT count(*) FROM receipts", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0,
        "presentation confirmation creates no receipt ACK"
    );
}

#[test]
fn canonical_invitation_context_coalesces_topic_changes_without_recipient_fanout() {
    use crate::protocol::commands::SetTopic;
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let inviter = check_in(
        &f,
        fixture_claim(&f, "x", "px", fourth(), "context-inviter"),
        "context-inviter-ci",
    );
    let thread = canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 1, "context-source");
    let original_wake = wake_attention(&f);
    for (n, topic) in [(1, "first latest topic"), (2, "second latest topic")] {
        f.service
            .handle(
                Command::SetTopic(SetTopic {
                    thread: thread.clone(),
                    topic: topic.into(),
                    operation: OperationId::new(format!("context-topic-{n}")),
                    claim: inviter.clone(),
                }),
                peer(),
                &budget(),
            )
            .unwrap();
    }
    let projection = f.db.prepare("SELECT c.context_revision,c.last_change_seq,t.topic,t.topic_revision FROM invitation_context_latest c JOIN threads t ON t.id=c.thread_id WHERE c.instance_id='i' AND c.thread_id=?1");
    assert!(
        projection.is_ok(),
        "canonical mutations need a durable latest-context projection: {:?}",
        projection.as_ref().err()
    );
    let (revision, seq, topic, topic_revision): (i64, i64, String, i64) = projection
        .unwrap()
        .query_row([thread.as_str()], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .unwrap();
    assert_eq!(topic, "second latest topic");
    assert_eq!(topic_revision, 2);
    assert!(revision >= 2);
    assert!(seq > 0);
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM invitation_context_latest WHERE thread_id=?1",
            [thread.as_str()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1,
        "coalesced state stays one row per thread"
    );
    assert_eq!(
        wake_attention(&f),
        original_wake,
        "passive topic changes do not wake"
    );
    let before: (i64, i64) = (revision, seq);
    // Exact operation replay must not invent a new canonical change sequence.
    f.service
        .handle(
            Command::SetTopic(SetTopic {
                thread: thread.clone(),
                topic: "second latest topic".into(),
                operation: OperationId::new("context-topic-2"),
                claim: inviter,
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    assert_eq!(f.db.query_row("SELECT context_revision,last_change_seq FROM invitation_context_latest WHERE thread_id=?1", [thread.as_str()], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?))).unwrap(), before);
}

#[test]
fn presentation_v2_resumes_control_heavy_ordinary_partial_without_changing_body_limit() {
    let f = fixture(0);
    let recipient = pull_seat(&f);
    ordinary_message(&f, "partial-prefix", &"a".repeat(20000));
    ordinary_message(&f, "partial-controls", &"\0".repeat(9000));
    let first = presentation_test_pull(&f, &recipient);
    assert!(matches!(
        first,
        crate::protocol::delivery::DeliveryPullReplyV2::Page(_)
    ));
    let partial:String=f.db.query_row("SELECT ordinary_partial FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'",[],|r|r.get(0)).unwrap();
    let partial: std::collections::HashMap<String, String> =
        serde_json::from_str(&partial).unwrap();
    let bytes = partial
        .get("partial-controls")
        .expect("real page split retained control bytes");
    assert!(bytes.len() > 16384 / 6 && bytes.len() < WATCH_BODY_LIMIT_BYTES);
    assert!(bytes.bytes().all(|b| b == 0));
    let mut completed = false;
    for _ in 0..4 {
        if let crate::protocol::delivery::DeliveryPullReplyV2::Page(page) =
            presentation_test_pull(&f, &recipient)
            && page.truncated.contains(&MessageId::new("partial-controls"))
        {
            assert!(page.text.contains(&"\0".repeat(WATCH_BODY_LIMIT_BYTES)));
            completed = true;
            break;
        }
    }
    assert!(completed, "partial resumes to inherited8KiB truncation");
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM receipts WHERE state='pending'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2,
        "no synthetic ordinary ACK"
    );
}

#[test]
fn presentation_v2_keeps_truncated_ordinary_cursor_progress_across_quanta() {
    let f = fixture(0);
    let recipient = pull_seat(&f);
    ordinary_message(&f, "multipage-ordinary", &"z".repeat(65536));
    ordinary_message(&f, "after-multipage", "following ordinary body");
    let crate::protocol::delivery::DeliveryPullReplyV2::Page(first) =
        presentation_test_pull(&f, &recipient)
    else {
        panic!("first ordinary body missing")
    };
    assert_eq!(first.truncated, vec![MessageId::new("multipage-ordinary")]);
    assert!(first.text.contains(&"z".repeat(WATCH_BODY_LIMIT_BYTES)));
    let mut quiet = false;
    let mut following = false;
    for _ in 0..5 {
        let next = presentation_test_pull(&f, &recipient);
        if let crate::protocol::delivery::DeliveryPullReplyV2::Page(page) = &next {
            assert!(
                !page
                    .truncated
                    .contains(&MessageId::new("multipage-ordinary")),
                "no repeated truncated body"
            );
            if page.offered.contains(&MessageId::new("after-multipage")) {
                assert!(page.text.contains("following ordinary body"));
                following = true;
            }
        }
        if matches!(
            next,
            crate::protocol::delivery::DeliveryPullReplyV2::Unchanged { .. }
        ) {
            quiet = true;
            break;
        }
    }
    assert!(
        following || first.offered.contains(&MessageId::new("after-multipage")),
        "following fetched item retained"
    );
    assert!(
        quiet,
        "already displayed truncated body remainder advances without reassembly failure or repeated display"
    );
    assert_eq!(
        f.db.query_row(
            "SELECT state FROM receipts WHERE message_id='multipage-ordinary'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "pending"
    );
}

#[test]
fn presentation_refresh_observes_lazy_context_and_removal_without_consuming_output() {
    use crate::protocol::commands::{Accept, SetTopic};
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let inviter = check_in(
        &f,
        fixture_claim(&f, "x", "px", fourth(), "refresh-inviter"),
        "refresh-inviter-ci",
    );
    let thread = canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 3, "refresh");
    let wake = wake_attention(&f);
    let refresh = || {
        f.store
            .presentation_refresh(
                &recipient.seat,
                &binding_for_claim(&f, &recipient),
                recipient.binding_generation,
                recipient.native_session.as_str(),
                &budget(),
            )
            .unwrap()
            .expect("canonical bounded passive refresh available")
    };
    let mut initial = refresh();
    for _ in 0..8 {
        assert!(initial.context_visits <= 40 && initial.lifecycle_visits <= 20);
        if !initial.more {
            break;
        }
        initial = refresh();
    }
    assert!(initial.version > 0);
    assert!(!initial.more);
    let before = initial.version;
    assert_eq!(refresh().version, before, "quiet content does not churn");
    assert_eq!(f.db.query_row("SELECT initial_after FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='watch'",[],|r|r.get::<_,i64>(0)).unwrap(),0,"maintenance never consumes presentation discovery");
    assert_eq!(f.db.query_row("SELECT count(*) FROM invitation_presentation_items WHERE seat_id='s' AND (complete_count!=0 OR partial_count!=0)",[],|r|r.get::<_,i64>(0)).unwrap(),0,"maintenance creates no display offers");
    f.service
        .handle(
            Command::SetTopic(SetTopic {
                thread: thread.clone(),
                topic: "latest passive refresh topic".into(),
                operation: OperationId::new("refresh-topic"),
                claim: inviter,
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    let changed = refresh();
    assert!(changed.version > before);
    assert_eq!(refresh().version, changed.version);
    f.service
        .handle(
            Command::Accept(Accept {
                thread,
                operation: OperationId::new("refresh-accept"),
                claim: recipient.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    let removed = refresh();
    assert!(
        removed.version > changed.version,
        "removal invalidates passive content"
    );
    assert_eq!(refresh().version, removed.version);
    assert_eq!(
        wake_attention(&f),
        wake,
        "passive maintenance and lazy acceptance do not manufacture wake attention"
    );
}

#[test]
fn presentation_refresh_preserves_live_presentation_header_and_outbox_across_rebind() {
    use crate::{
        ports::PresentationInput,
        protocol::delivery::{DeliveryPullReplyV2, DeliveryPullV2, PresentationConsumer},
    };
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let inviter = check_in(
        &f,
        fixture_claim(&f, "x", "px", fourth(), "refresh-preserve-inviter"),
        "refresh-preserve-ci",
    );
    canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 1, "refresh-preserve");
    let pull =
        |claim: CallerClaim| {
            f.store
            .presentation_pull(
                PresentationInput { binding_id: binding_for_claim(&f, &claim), owner_replay: None,
                    ordinary_due_snapshot: None,
                    frozen_render: presentation_watch_frozen(&f, &claim),
                    render_profile:
                        crate::delivery::presentation::PresentationRenderProfile::WatchJsonLinesV1,
                    pull: DeliveryPullV2 {
                        claim,
                        consumer: PresentationConsumer::Watch,
                        render:
                            crate::protocol::delivery::PresentationRenderRequest::WatchJsonLinesV1,
                        presentation_lease: Some("0123456789abcdef0123456789abcdef".into()),
                        marker_prefix: vec![],
                    },
                    attention_version: 0,
                    ordinary_version: 0,
                    with_attention: false,
                    retry_messages: false,
                    blocked_messages: vec![],
                    known_messages: vec![],
                    new_message_slots: 256,
                },
                &budget(),
            )
            .unwrap()
            .reply
        };
    let DeliveryPullReplyV2::Page(first) = pull(recipient.clone()) else {
        panic!("initial full context")
    };
    assert_eq!(presentation_invitations(&first).len(), 1);
    assert!(!presentation_invitations(&first)[0].content.truncated);
    let snapshot = || {
        f.db.query_row("SELECT json_object('generation',generation,'execution',execution,'initial_after',initial_after,'initial_high',initial_high,'context_after_seq',context_after_seq,'revisit_after_bytes',revisit_after_bytes,'rotation',rotation,'outbox_token',outbox_token,'outbox_text',outbox_text,'outbox_descriptors',outbox_descriptors,'counters',(SELECT json_group_array(json_array(generation,execution,context_key,partial_count,complete_count,output_class)) FROM invitation_presentation_items WHERE binding_id=?1 AND consumer='watch')) FROM invitation_presentation_consumers WHERE binding_id=?1 AND consumer='watch'",[binding_for_claim(&f, &recipient).as_str()],|r|r.get::<_,String>(0)).unwrap()
    };
    let before = snapshot();
    f.store
        .presentation_refresh(
            &recipient.seat,
            &binding_for_claim(&f, &recipient),
            recipient.binding_generation,
            recipient.native_session.as_str(),
            &budget(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        snapshot(),
        before,
        "same-binding maintenance preserves allocation and replay"
    );
    let next = rotate(&f, &recipient, "refresh-new-native", 2);
    assert_eq!(
        snapshot(),
        before,
        "canonical rebind itself does not rewrite presentation rows"
    );
    f.store
        .presentation_refresh(
            &next.seat,
            &binding_for_claim(&f, &next),
            next.binding_generation,
            next.native_session.as_str(),
            &budget(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        snapshot(),
        before,
        "maintenance must not perform presentation binding reset"
    );
    let DeliveryPullReplyV2::Page(fresh) = pull(next) else {
        panic!("actual presentation owns reset and redisplays full context")
    };
    assert_eq!(presentation_invitations(&fresh).len(), 1);
    assert!(!presentation_invitations(&fresh)[0].content.truncated);
    assert_ne!(fresh.presentation_token, first.presentation_token);
}

#[test]
fn presentation_refresh_stub_allows_actual_watch_full_offer_and_replaced_owner_replay() {
    use crate::{
        ports::{ModChannelSink, ModChannels, ModStoreReads},
        protocol::{
            delivery::{
                DeliveryPresentationConfirmV2, DeliveryPresentationConfirmationV2,
                DeliveryPullReplyV2, DeliveryPullV2, PresentationConsumer,
            },
            watch::{ModDeliverySetting, WatchFrame, WatchRequest},
        },
        service::mod_channels::ModChannelRegistry,
    };
    struct Sink;
    impl ModChannelSink for Sink {
        fn push(&self, _: WatchFrame) -> bool {
            true
        }
    }
    let mut f = fixture(0);
    let recipient = check_in(
        &f,
        fixture_claim(&f, "c", "pc", Harness::Claude, "watch-full-context"),
        "watch-full-ci",
    );
    let inviter = check_in(
        &f,
        fixture_claim(&f, "x", "px", fourth(), "watch-full-inviter"),
        "watch-full-inviter-ci",
    );
    canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 1, "watch-full");
    let reads: Arc<dyn ModStoreReads> = f.store.clone();
    let registry = Arc::new(ModChannelRegistry::new(
        f.clock.clone(),
        reads,
        "i".into(),
        Arc::new(|| {}),
        ModDeliverySetting::On,
    ));
    f.service = DomainService::new("i".into(), f.store.clone(), f.clock.clone())
        .with_cooperative_owner(501, Arc::new(FairWriter::new(32)))
        .with_delivery_pull(f.state.clone())
        .with_mod_channels(registry.clone());
    let refresh = f
        .store
        .presentation_refresh(
            &recipient.seat,
            &binding_for_claim(&f, &recipient),
            recipient.binding_generation,
            recipient.native_session.as_str(),
            &budget(),
        )
        .unwrap()
        .unwrap();
    assert!(refresh.version > 0);
    let (first, _) = f
        .service
        .watch_register(
            &WatchRequest {
                presentation_v2: true,
                claim: recipient.clone(),
            },
            Arc::new(Sink),
            &budget(),
        )
        .unwrap();
    let old = registry.presentation_lease(first).unwrap();
    let pull = |lease: String| match f
        .service
        .handle(
            Command::DeliveryPullV2(DeliveryPullV2 {
                claim: recipient.clone(),
                consumer: PresentationConsumer::Watch,
                render: crate::protocol::delivery::PresentationRenderRequest::WatchJsonLinesV1,
                presentation_lease: Some(lease),
                marker_prefix: vec![],
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    {
        CommandResult::DeliveryPulledV2(reply) => reply,
        other => panic!("{other:?}"),
    };
    let DeliveryPullReplyV2::Page(full) = pull(old.clone()) else {
        panic!("mod-only stub must not hide first full output")
    };
    assert_eq!(presentation_invitations(&full).len(), 1);
    assert!(!presentation_invitations(&full)[0].content.truncated);
    assert!(
        presentation_invitations(&full)[0]
            .content
            .body
            .contains("review")
    );
    let (second, _) = f
        .service
        .watch_register(
            &WatchRequest {
                presentation_v2: true,
                claim: recipient.clone(),
            },
            Arc::new(Sink),
            &budget(),
        )
        .unwrap();
    let new = registry.presentation_lease(second).unwrap();
    let DeliveryPullReplyV2::Page(replay) = pull(new.clone()) else {
        panic!("replacement reuses current outbox")
    };
    assert_eq!(replay, full);
    let confirm = |lease: String| {
        f.service
            .handle(
                Command::DeliveryPresentationConfirmV2(DeliveryPresentationConfirmV2 {
                    claim: recipient.clone(),
                    consumer: PresentationConsumer::Watch,
                    token: full.presentation_token.clone().unwrap(),
                    emitted_controls: full.emitted_controls.clone(),
                    presentation_lease: Some(lease),
                }),
                peer(),
                &budget(),
            )
            .unwrap()
    };
    assert!(matches!(
        confirm(old),
        CommandResult::DeliveryPresentationConfirmedV2(
            crate::protocol::delivery::DeliveryPresentationConfirmationReplyV2 {
                status: DeliveryPresentationConfirmationV2::StaleBinding,
                ..
            }
        )
    ));
    assert!(matches!(
        confirm(new),
        CommandResult::DeliveryPresentationConfirmedV2(
            crate::protocol::delivery::DeliveryPresentationConfirmationReplyV2 {
                status: DeliveryPresentationConfirmationV2::Confirmed,
                ..
            }
        )
    ));
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM invitations WHERE seat_id='c' AND state='pending'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1,
        "no invitation acceptance or fake ACK"
    );
}

#[test]
fn presentation_final_watch_codec_counts_escaped_canonical_invitation_batch() {
    use crate::{
        ports::PresentationInput,
        protocol::{
            commands::{CreateThread, InvitationConfirmation, Invite},
            delivery::{DeliveryPullReplyV2, DeliveryPullV2, PresentationConsumer},
            watch::{WatchItem, WatchLine},
        },
    };
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let inviter = check_in(
        &f,
        fixture_claim(&f, "x", "px", fourth(), "watch-frame-inviter"),
        "watch-frame-ci",
    );
    let topic = "\0".repeat(300);
    let goal = "\0".repeat(300);
    for n in 0..24 {
        let CommandResult::ThreadCreated(thread) = f
            .service
            .handle(
                Command::CreateThread(CreateThread {
                    name: None,
                    topic: topic.clone(),
                    goal: goal.clone(),
                    operation: OperationId::new(format!("watch-frame-create-{n}")),
                    claim: inviter.clone(),
                }),
                peer(),
                &budget(),
            )
            .unwrap()
        else {
            panic!("canonical thread")
        };
        f.service
            .handle(
                Command::Invite(Invite {
                    confirmation: Some(InvitationConfirmation::Lazy),
                    thread,
                    seat: recipient.seat.clone(),
                    deadline_millis: None,
                    operation: OperationId::new(format!("watch-frame-invite-{n}")),
                    claim: inviter.clone(),
                }),
                peer(),
                &budget(),
            )
            .unwrap();
    }
    let wake = wake_attention(&f);
    let mut displayed = std::collections::HashSet::new();
    for _ in 0..24 {
        let result = f
            .store
            .presentation_pull(
                PresentationInput {
                    binding_id: binding_for_claim(&f, &recipient),
                    owner_replay: None,
                    ordinary_due_snapshot: None,
                    frozen_render: presentation_watch_frozen(&f, &recipient),
                    render_profile:
                        crate::delivery::presentation::PresentationRenderProfile::WatchJsonLinesV1,
                    pull: DeliveryPullV2 {
                        claim: recipient.clone(),
                        consumer: PresentationConsumer::Watch,
                        render:
                            crate::protocol::delivery::PresentationRenderRequest::WatchJsonLinesV1,
                        presentation_lease: Some("0123456789abcdef0123456789abcdef".into()),
                        marker_prefix: vec![],
                    },
                    attention_version: 0,
                    ordinary_version: 0,
                    with_attention: false,
                    retry_messages: false,
                    blocked_messages: vec![],
                    known_messages: vec![],
                    new_message_slots: 256,
                },
                &budget(),
            )
            .unwrap();
        let DeliveryPullReplyV2::Page(page) = result.reply else {
            break;
        };
        let mut final_bytes = 0;
        for item in &presentation_invitations(&page) {
            assert!(!item.content.truncated, "standalone full context fits");
            assert_eq!(
                item.content.body.matches(&topic).count(),
                2,
                "exact canonical topic and goal preserved"
            );
            final_bytes += serde_json::to_vec(&WatchLine::new(
                format!("invitation:{}", item.id.as_str()),
                WatchItem::Invitation(item.content.clone()),
            ))
            .unwrap()
            .len()
                + 1;
            displayed.insert(item.id.clone());
        }
        assert!(
            final_bytes <= 65536,
            "final Watch JSON-lines batch is {final_bytes} bytes"
        );
        if let Some(token) = page.presentation_token {
            let result = f
                .store
                .presentation_confirm(
                    &crate::protocol::delivery::DeliveryPresentationConfirmV2 {
                        claim: recipient.clone(),
                        consumer: PresentationConsumer::Watch,
                        token,
                        presentation_lease: Some("0123456789abcdef0123456789abcdef".into()),
                        emitted_controls: page.emitted_controls,
                    },
                    &budget(),
                )
                .unwrap();
            assert!(matches!(result.status,crate::protocol::delivery::DeliveryPresentationConfirmationV2::Confirmed|crate::protocol::delivery::DeliveryPresentationConfirmationV2::AlreadyConfirmed));
        }
    }
    assert_eq!(
        displayed.len(),
        24,
        "every canonical invitation eventually full"
    );
    assert_eq!(
        wake_attention(&f),
        wake,
        "lazy output never creates wake work"
    );
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM invitations WHERE seat_id='s' AND state='pending'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        24
    );
}

#[test]
fn presentation_final_cleanup_preserves_passive_removal_observation() {
    use crate::{
        ports::PresentationInput,
        protocol::{
            commands::Accept,
            delivery::{DeliveryPullV2, PresentationConsumer},
        },
    };
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let inviter = check_in(
        &f,
        fixture_claim(&f, "x", "px", fourth(), "cleanup-inviter"),
        "cleanup-ci",
    );
    let thread = canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 1, "cleanup-order");
    let refresh = || {
        f.store
            .presentation_refresh(
                &recipient.seat,
                &binding_for_claim(&f, &recipient),
                recipient.binding_generation,
                recipient.native_session.as_str(),
                &budget(),
            )
            .unwrap()
            .unwrap()
    };
    let initial = refresh();
    assert!(initial.version > 0);
    f.service
        .handle(
            Command::Accept(Accept {
                thread,
                operation: OperationId::new("cleanup-accept"),
                claim: recipient.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    // Presentation cleanup wins the race with passive maintenance.
    f.store
        .presentation_pull(
            PresentationInput {
                binding_id: binding_for_claim(&f, &recipient),
                owner_replay: None,
                ordinary_due_snapshot: None,
                frozen_render: presentation_watch_frozen(&f, &recipient),
                render_profile:
                    crate::delivery::presentation::PresentationRenderProfile::WatchJsonLinesV1,
                pull: DeliveryPullV2 {
                    claim: recipient.clone(),
                    consumer: PresentationConsumer::Watch,
                    render: crate::protocol::delivery::PresentationRenderRequest::WatchJsonLinesV1,
                    presentation_lease: Some("0123456789abcdef0123456789abcdef".into()),
                    marker_prefix: vec![],
                },
                attention_version: 0,
                ordinary_version: 0,
                with_attention: false,
                retry_messages: false,
                blocked_messages: vec![],
                known_messages: vec![],
                new_message_slots: 256,
            },
            &budget(),
        )
        .unwrap();
    let removed = refresh();
    assert!(
        removed.version > initial.version,
        "removal must invalidate passive content even after presentation cleanup"
    );
    assert_eq!(refresh().version, removed.version, "removal observed once");
    assert_eq!(f.db.query_row("SELECT count(*) FROM invitation_presentation_items WHERE seat_id='s' AND consumer='watch'",[],|r|r.get::<_,i64>(0)).unwrap(),0,"settled current row is reclaimed");
}

#[test]
fn presentation_registry_pass_drives_bounded_passive_refresh_before_full_pull() {
    use crate::{
        ports::{ModChannelSink, ModChannels, ModStoreReads},
        protocol::{
            delivery::{DeliveryPullReplyV2, DeliveryPullV2, PresentationConsumer},
            watch::{ModDeliverySetting, WatchFrame, WatchRequest},
        },
        service::mod_channels::ModChannelRegistry,
    };
    #[derive(Default)]
    struct Sink(std::sync::Mutex<Vec<WatchFrame>>);
    impl ModChannelSink for Sink {
        fn push(&self, frame: WatchFrame) -> bool {
            self.0.lock().unwrap().push(frame);
            true
        }
    }
    let mut f = fixture(0);
    let recipient = check_in(
        &f,
        fixture_claim(&f, "c", "pc", Harness::Claude, "registry-refresh"),
        "registry-refresh-ci",
    );
    let inviter = check_in(
        &f,
        fixture_claim(&f, "x", "px", fourth(), "registry-refresh-inviter"),
        "registry-refresh-inviter-ci",
    );
    let reads: Arc<dyn ModStoreReads> = f.store.clone();
    let registry = Arc::new(ModChannelRegistry::new(
        f.clock.clone(),
        reads,
        "i".into(),
        Arc::new(|| {}),
        ModDeliverySetting::On,
    ));
    f.service = DomainService::new("i".into(), f.store.clone(), f.clock.clone())
        .with_cooperative_owner(501, Arc::new(FairWriter::new(32)))
        .with_delivery_pull(f.state.clone())
        .with_mod_channels(registry.clone());
    let sink = Arc::new(Sink::default());
    let (channel, _) = f
        .service
        .watch_register(
            &WatchRequest {
                presentation_v2: true,
                claim: recipient.clone(),
            },
            sink.clone(),
            &budget(),
        )
        .unwrap();
    registry.pass(UtcMillis(0)).unwrap();
    sink.0.lock().unwrap().clear();
    canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 1, "registry-refresh");
    registry.pass(UtcMillis(1)).unwrap();
    let version:i64=f.db.query_row("SELECT COALESCE((SELECT mod_version FROM invitation_presentation_consumers WHERE seat_id='c' AND consumer='watch'),0)",[],|r|r.get(0)).unwrap();
    assert!(
        version > 0,
        "actual registry worker must drive independent canonical maintenance"
    );
    let frames = sink.0.lock().unwrap().clone();
    assert!(
        matches!(frames.as_slice(), [WatchFrame::Attention { .. }]),
        "passive arrival requests exactly one in-turn drain: {frames:?}"
    );
    registry.pass(UtcMillis(2)).unwrap();
    assert_eq!(
        *sink.0.lock().unwrap(),
        frames,
        "quiet maintenance does not flood context"
    );
    assert_eq!(f.db.query_row("SELECT initial_after FROM invitation_presentation_consumers WHERE seat_id='c' AND consumer='watch'",[],|r|r.get::<_,i64>(0)).unwrap(),0,"worker has not consumed presentation discovery");
    let lease = registry.presentation_lease(channel).unwrap();
    let result = f
        .service
        .handle(
            Command::DeliveryPullV2(DeliveryPullV2 {
                claim: recipient,
                consumer: PresentationConsumer::Watch,
                render: crate::protocol::delivery::PresentationRenderRequest::WatchJsonLinesV1,
                presentation_lease: Some(lease),
                marker_prefix: vec![],
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    let CommandResult::DeliveryPulledV2(DeliveryPullReplyV2::Page(page)) = result else {
        panic!("actual full pull")
    };
    assert_eq!(presentation_invitations(&page).len(), 1);
    assert!(!presentation_invitations(&page)[0].content.truncated);
    assert!(
        presentation_invitations(&page)[0]
            .content
            .body
            .contains("goal: review")
    );
}

#[test]
fn presentation_private_partial_guards_reject_corrupt_supported_shapes() {
    use crate::protocol::results::ErrorCode;
    let cases = [
        (
            "cardinality",
            serde_json::json!({"one":"a","two":"b"}).to_string(),
        ),
        (
            "raw-limit",
            serde_json::json!({"raw":"\0".repeat(8192)}).to_string(),
        ),
        ("invalid-id", serde_json::json!({"é":"x"}).to_string()),
        (
            "serialized-limit",
            serde_json::json!({"raw":"\0".repeat(9000)}).to_string(),
        ),
        ("invalid-json", "not-json".into()),
    ];
    for (label, partial) in cases {
        let f = fixture(0);
        let recipient = pull_seat(&f);
        presentation_test_pull(&f, &recipient);
        // Deliberate local corruption exercises canonical checked reads even
        // when an operator/older writer bypassed the schema's storage guard.
        f.db.execute_batch("PRAGMA ignore_check_constraints=ON")
            .unwrap();
        f.db.execute("UPDATE invitation_presentation_consumers SET ordinary_partial=?1,ordinary_done=0,ordinary_cursor=NULL WHERE seat_id='s' AND consumer='hook'",[partial]).unwrap();
        f.db.execute_batch("PRAGMA ignore_check_constraints=OFF")
            .unwrap();
        let error = f
            .service
            .handle(
                Command::DeliveryPullV2(crate::protocol::delivery::DeliveryPullV2 {
                    claim: recipient,
                    consumer: crate::protocol::delivery::PresentationConsumer::Hook,
                    render: crate::protocol::delivery::PresentationRenderRequest::Hook {
                        confirmable_selector: 0,
                        passive_selector: 0,
                    },
                    presentation_lease: None,
                    marker_prefix: vec![],
                }),
                peer(),
                &budget(),
            )
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::StoreCorrupt, "{label}: {error:?}");
        assert!(f.db.query_row("SELECT outbox_token IS NULL FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'",[],|r|r.get::<_,bool>(0)).unwrap(),"failed allocation has no token");
    }
}

#[test]
fn presentation_private_skipped_body_requires_exact_retained_cursor() {
    use crate::protocol::results::ErrorCode;
    let f = fixture(0);
    let recipient = pull_seat(&f);
    presentation_test_pull(&f, &recipient);
    f.db.execute("UPDATE invitation_presentation_consumers SET ordinary_skip_body='missing-body',ordinary_done=0,ordinary_cursor=NULL WHERE seat_id='s' AND consumer='hook'",[]).unwrap();
    let error = f
        .service
        .handle(
            Command::DeliveryPullV2(crate::protocol::delivery::DeliveryPullV2 {
                claim: recipient,
                consumer: crate::protocol::delivery::PresentationConsumer::Hook,
                render: crate::protocol::delivery::PresentationRenderRequest::Hook {
                    confirmable_selector: 0,
                    passive_selector: 0,
                },
                presentation_lease: None,
                marker_prefix: vec![],
            }),
            peer(),
            &budget(),
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::StoreCorrupt);
    assert_eq!(
        error.detail,
        "ordinary accumulator lacks matching body cursor"
    );
}

#[test]
fn presentation_ordinary_carry_survives_completed_control_partial_and_invitation_priority() {
    use crate::protocol::delivery::{DeliveryPullReplyV2, DeliveryPullV2, PresentationConsumer};
    let f = fixture(0);
    let recipient = pull_seat(&f);
    ordinary_message(&f, "carry-prefix", &"a".repeat(20000));
    ordinary_message(&f, "carry-controls", &"\0".repeat(9000));
    for n in 0..6 {
        ordinary_message(&f, &format!("carry-after-{n}"), &"b".repeat(8192));
    }
    let first = presentation_test_pull(&f, &recipient);
    assert!(matches!(first, DeliveryPullReplyV2::Page(_)));
    let partial:String=f.db.query_row("SELECT ordinary_partial FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'",[],|r|r.get(0)).unwrap();
    let partial: std::collections::HashMap<String, String> =
        serde_json::from_str(&partial).unwrap();
    assert!(
        partial.contains_key("carry-controls"),
        "actual query split retains control-heavy partial"
    );
    let inviter = check_in(
        &f,
        fixture_claim(&f, "x", "px", fourth(), "carry-inviter"),
        "carry-inviter-ci",
    );
    canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 24, "carry-priority");
    let prefix = vec!["p".repeat(4096)];
    let mut seen = std::collections::HashSet::new();
    for _ in 0..24 {
        let result=f.service.handle(Command::DeliveryPullV2(DeliveryPullV2{claim:recipient.clone(),consumer:PresentationConsumer::Hook,render:crate::protocol::delivery::PresentationRenderRequest::Hook{confirmable_selector:0,passive_selector:0},presentation_lease:None,marker_prefix:prefix.clone()}),peer(),&budget()).expect("valid bounded ordinary query plus prior partial must persist withheld ordinary items");
        let CommandResult::DeliveryPulledV2(reply) = result else {
            panic!("pull result")
        };
        let DeliveryPullReplyV2::Page(page) = reply else {
            break;
        };
        for id in page.offered.iter().chain(&page.truncated) {
            seen.insert(id.as_str().to_owned());
        }
        presentation_test_confirm(&f, &recipient, &page);
    }
    assert!(
        seen.contains("carry-controls"),
        "completed partial eventually presents inherited truncated body"
    );
    for n in 0..6 {
        assert!(
            seen.contains(&format!("carry-after-{n}")),
            "following fetched ordinary item retained"
        );
    }
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM receipts WHERE state='pending'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        8,
        "presentation is no ACK"
    );
}

#[test]
fn presentation_raw_carry_serializer_accounting_witness() {
    use crate::{
        delivery::{Assembled, Assembler},
        protocol::{
            ids::{MessageId, SeatId, ThreadId},
            results::InboxBatchV2Item,
        },
    };
    let prefix = vec!["\0".repeat(4096)];
    assert_eq!(serde_json::to_vec(&prefix).unwrap().len(), 24580);
    // Eight arguments maximize independent quote/comma overhead at fixed raw bytes.
    let max_prefix = vec!["\0".repeat(512); 8];
    assert_eq!(serde_json::to_vec(&max_prefix).unwrap().len(), 24601);
    let mut max_delta = 0isize;
    for topic in [
        "".into(),
        "x".repeat(121),
        "é".repeat(121),
        "\0".repeat(121),
        "\"".repeat(121),
        "\u{2028}".repeat(121),
    ] {
        for lazy in [false, true] {
            let message = MessageId::parse("\"".repeat(128)).unwrap();
            let sender = Some(SeatId::parse("\"".repeat(128)).unwrap());
            let item = if lazy {
                InboxBatchV2Item::LazyMessage {
                    thread: ThreadId::parse("t").unwrap(),
                    topic_data: topic.clone(),
                    message: message.clone(),
                    sequence: 1,
                    sender: sender.clone(),
                    author_role: None,
                    relays_user: false,
                    user_intent: None,
                    author_role_backfilled: false,
                    body: "\0".repeat(8192),
                    body_start: 0,
                    body_end: 8192,
                    body_len: 8193,
                    wakes: false,
                }
            } else {
                InboxBatchV2Item::Message {
                    thread: ThreadId::parse("t").unwrap(),
                    topic_data: topic.clone(),
                    message: message.clone(),
                    sequence: 1,
                    sender: sender.clone(),
                    author_role: None,
                    relays_user: false,
                    user_intent: None,
                    author_role_backfilled: false,
                    body: "\0".repeat(8192),
                    body_start: 0,
                    body_end: 8192,
                    body_len: 8193,
                    ack_candidate: None,
                }
            };
            let source_len = serde_json::to_vec(&item).unwrap().len();
            let mut assembler = Assembler::new(None).with_command_prefix(prefix.clone());
            let Assembled::Item(mut assembled) = assembler.push(item).unwrap() else {
                panic!("completed supported chunk")
            };
            let marker = crate::protocol::watch::truncation_marker_for(&prefix, &message, lazy);
            assert!(assembled.message.truncated);
            assembled.message.body = assembled
                .message
                .body
                .strip_suffix(&marker)
                .expect("producer-owned exact generated marker")
                .to_owned();
            assert_eq!(assembled.message.body, "\0".repeat(8192));
            let raw = serde_json::json!({"marker_lazy":lazy,"content":{"id":message,"lazy":lazy,"content":assembled.message}});
            let delta = serde_json::to_vec(&raw).unwrap().len() as isize - source_len as isize;
            max_delta = max_delta.max(delta);
            assert!(delta <= 512, "metadata delta {delta}");
        }
    }
    // Page bytes + one prior raw partial + per-item metadata + one prefix + shell.
    let derived = 65536 + 6 * (8192 - 1) + 32 * 512 + 24601 + 64;
    assert_eq!(derived, 155731);
    eprintln!("raw carry metadata witness max_delta={max_delta}, derived_cap={derived}");
}

#[test]
fn presentation_current_handler_derives_timed_invitation_attention() {
    use crate::protocol::commands::{CreateThread, InvitationConfirmation, Invite};
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let inviter = check_in(
        &f,
        fixture_claim(&f, "x", "px", fourth(), "relevance-inviter"),
        "relevance-inviter-ci",
    );
    let CommandResult::ThreadCreated(thread) = f
        .service
        .handle(
            Command::CreateThread(CreateThread {
                name: None,
                topic: "timed invitation".into(),
                goal: "review".into(),
                operation: OperationId::new("relevance-thread"),
                claim: inviter.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("thread")
    };
    f.service
        .handle(
            Command::Invite(Invite {
                confirmation: Some(InvitationConfirmation::RequireAck),
                thread,
                seat: recipient.seat.clone(),
                deadline_millis: Some(1000),
                operation: OperationId::new("relevance-invite"),
                claim: inviter,
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    let crate::protocol::delivery::DeliveryPullReplyV2::Page(page) =
        presentation_test_pull(&f, &recipient)
    else {
        panic!("actual pending invitation page")
    };
    assert!(
        page.text.contains("[herdr-threads] attention "),
        "actual handler must derive canonical confirming invitation relevance, not digest-free placeholder metadata: {}",
        page.text
    );
}

#[test]
fn presentation_promoted_lazy_raw_carry_survives_restart_with_frozen_prefix() {
    use crate::protocol::delivery::{
        DeliveryPullReplyV2, DeliveryPullV2, PresentationConsumer, PresentationItemV2,
        PresentationRenderRequest,
    };
    let mut f = fixture(0);
    let recipient = pull_seat(&f);
    f.db.execute("UPDATE seats SET wake_on='all' WHERE id='s'", [])
        .unwrap();
    let inviter = check_in(
        &f,
        fixture_claim(&f, "x", "px", fourth(), "promoted-carry-inviter"),
        "promoted-carry-ci",
    );
    canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 1, "promoted-baseline");
    let DeliveryPullReplyV2::Page(baseline) = presentation_test_pull(&f, &recipient) else {
        panic!("baseline invitation")
    };
    presentation_test_confirm(&f, &recipient, &baseline);
    canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 24, "promoted-priority");
    lazy_message(&f, "msg-promoted-carry", &"\0".repeat(9000));
    let old_prefix = vec!["p".repeat(4096)];
    let pull = |f: &Fixture, prefix: Vec<String>| match f
        .service
        .handle(
            Command::DeliveryPullV2(DeliveryPullV2 {
                claim: recipient.clone(),
                consumer: PresentationConsumer::Hook,
                render: PresentationRenderRequest::Hook {
                    confirmable_selector: 0,
                    passive_selector: 0,
                },
                presentation_lease: None,
                marker_prefix: prefix,
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    {
        CommandResult::DeliveryPulledV2(reply) => reply,
        other => panic!("{other:?}"),
    };
    let DeliveryPullReplyV2::Page(first) = pull(&f, old_prefix.clone()) else {
        panic!("priority page")
    };
    let json:String=f.db.query_row("SELECT ordinary_pending FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'",[],|r|r.get(0)).unwrap();
    let carry: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(carry["prefix"], serde_json::json!(old_prefix));
    let row = carry["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["content"]["id"] == "msg-promoted-carry")
        .expect("actual promoted lazy withheld carry");
    assert_eq!(row["marker_lazy"], true);
    assert_eq!(
        row["content"]["lazy"], false,
        "promotion changes output type only"
    );
    assert_eq!(row["content"]["content"]["body"], "\0".repeat(8192));
    presentation_test_confirm(&f, &recipient, &first);
    restart(&mut f);
    let mut emitted = None;
    for _ in 0..24 {
        let DeliveryPullReplyV2::Page(page) = pull(&f, vec!["new-prefix".into()]) else {
            break;
        };
        for item in &page.items {
            if let PresentationItemV2::Message { id, lazy, content } = item
                && id.as_str() == "msg-promoted-carry"
            {
                assert!(!lazy);
                emitted = Some(content.body.clone());
            }
        }
        presentation_test_confirm(&f, &recipient, &page);
        if emitted.is_some() {
            break;
        }
    }
    let expected = format!(
        "{}{}",
        "\0".repeat(8192),
        crate::protocol::watch::truncation_marker_for(
            &old_prefix,
            &MessageId::new("msg-promoted-carry"),
            true
        )
    );
    assert_eq!(
        emitted,
        Some(expected),
        "restart and new request prefix preserve original presentation bytes"
    );
    assert_eq!(
        lazy_state(&f, "msg-promoted-carry"),
        "pending",
        "presentation is no lazy completion"
    );
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM receipts WHERE message_id='msg-promoted-carry'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

// Private fixed-state transition coverage; actual canonical Attention allocation
// is separately exercised by the current-handler relevance regression.
#[test]
fn presentation_confirmation_preserves_original_slots_and_exact_subsets_on_restart() {
    use crate::protocol::delivery::{
        DeliveryPresentationConfirmV2, DeliveryPresentationConfirmationV2 as Status,
        PresentationConsumer, PresentationControlStampV2 as Stamp, PresentationItemV2 as Item,
    };
    let mut f = fixture(0);
    let recipient = pull_seat(&f);
    let inviter = check_in(
        &f,
        fixture_claim(&f, "x", "px", fourth(), "confirm-slot-inviter"),
        "confirm-slot-ci",
    );
    canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 1, "confirm-slot");
    let crate::protocol::delivery::DeliveryPullReplyV2::Page(page) =
        presentation_test_pull(&f, &recipient)
    else {
        panic!("canonical invitation page")
    };
    let token = page.presentation_token.clone().unwrap();
    let mut descriptors:serde_json::Value=serde_json::from_str(&f.db.query_row("SELECT outbox_descriptors FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'",[],|r|r.get::<_,String>(0)).unwrap()).unwrap();
    let attention = Stamp::Attention {
        epoch: "1".into(),
        version: 9,
    };
    descriptors.as_array_mut().unwrap().push(serde_json::json!({"item":Item::Attention{epoch:"1".into(),attention_version:9,text:"ht inbox".into()},"context_key":null}));
    let controls = serde_json::json!({"counter":"1","last_exact":true,"active_epoch":"1","allocated_attention":attention});
    f.db.execute("UPDATE invitation_presentation_consumers SET outbox_descriptors=?1,control_state=?2 WHERE seat_id='s' AND consumer='hook'",rusqlite::params![descriptors.to_string(),controls.to_string()]).unwrap();
    let confirm = |f: &Fixture, stamps: Vec<Stamp>| {
        f.store.presentation_confirm(
            &DeliveryPresentationConfirmV2 {
                claim: recipient.clone(),
                consumer: PresentationConsumer::Hook,
                token: token.clone(),
                presentation_lease: None,
                emitted_controls: stamps,
            },
            &budget(),
        )
    };
    let before=f.db.query_row("SELECT outbox_descriptors||control_state||outbox_retired_mask FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'",[],|r|r.get::<_,String>(0)).unwrap();
    for stamps in [
        vec![Stamp::Attention {
            epoch: "1".into(),
            version: 10,
        }],
        vec![attention.clone(), attention.clone()],
    ] {
        assert!(confirm(&f, stamps).is_err());
        assert_eq!(f.db.query_row("SELECT outbox_descriptors||control_state||outbox_retired_mask FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'",[],|r|r.get::<_,String>(0)).unwrap(),before,"invalid subset has no effects");
    }
    let first = confirm(&f, vec![]).unwrap();
    assert_eq!(first.status, Status::Confirmed);
    assert!(first.acknowledged_controls.is_empty());
    let retained=f.db.query_row("SELECT outbox_token,outbox_descriptors,outbox_retired_mask,outbox_confirmed_mask FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?))).unwrap();
    assert_eq!(
        retained,
        (token.clone(), descriptors.to_string(), 1, 1),
        "slot1 survives original slot0 retirement without compaction"
    );
    restart(&mut f);
    assert_eq!(
        confirm(&f, vec![]).unwrap().status,
        Status::AlreadyConfirmed,
        "old empty completion cannot consume newly visible control"
    );
    let exact = confirm(&f, vec![attention.clone()]).unwrap();
    assert_eq!(exact.status, Status::Confirmed);
    assert_eq!(exact.acknowledged_controls, vec![attention.clone()]);
    let retry = confirm(&f, vec![attention.clone()]).unwrap();
    assert_eq!(retry.status, Status::AlreadyConfirmed);
    assert_eq!(retry.acknowledged_controls, vec![attention.clone()]);
    f.db.execute("UPDATE invitation_presentation_consumers SET control_state=?1 WHERE seat_id='s' AND consumer='hook'",[serde_json::json!({"counter":"2","last_exact":true,"active_epoch":"2"}).to_string()]).unwrap();
    assert!(
        confirm(&f, vec![attention])
            .unwrap()
            .acknowledged_controls
            .is_empty(),
        "last-token retry cannot resurrect obsolete epoch"
    );
}

#[test]
fn presentation_actual_candidate_sizing_and_rpc_wrapper_work_remain_bounded() {
    use crate::{
        delivery::presentation::{FrozenPresentationRender, PresentationRenderProfile},
        ports::PresentationInput,
        protocol::delivery::{
            DeliveryPullReplyV2, DeliveryPullV2, PresentationConsumer, PresentationRenderRequest,
        },
    };
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let inviter = check_in(
        &f,
        fixture_claim(&f, "x", "px", fourth(), "sizing-bound-inviter"),
        "sizing-bound-ci",
    );
    canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 100, "sizing-bound");
    let registry = crate::harness::registry::builtins();
    let adapter = registry
        .by_id(registry.agent(recipient.harness.as_str()).unwrap())
        .unwrap();
    let codec = adapter.presentation_codec(0).unwrap();
    let profile = PresentationRenderProfile::Hook {
        confirmable_selector: 0,
        passive_selector: 0,
        confirmable: codec,
        passive: codec,
    };
    let mut all = std::collections::HashSet::new();
    let mut max_passes = 0;
    for _ in 0..20 {
        let frozen_render = f
            .store
            .presentation_render_selection(&recipient, PresentationConsumer::Hook, &budget())
            .unwrap()
            .map(|identity| FrozenPresentationRender {
                identity,
                codec: Some(codec),
            });
        let output = f
            .store
            .presentation_pull(
                PresentationInput {
                    binding_id: binding_for_claim(&f, &recipient),
                    owner_replay: None,
                    ordinary_due_snapshot: None,
                    frozen_render,
                    render_profile: profile.clone(),
                    pull: DeliveryPullV2 {
                        claim: recipient.clone(),
                        consumer: PresentationConsumer::Hook,
                        render: PresentationRenderRequest::Hook {
                            confirmable_selector: 0,
                            passive_selector: 0,
                        },
                        presentation_lease: None,
                        marker_prefix: vec![],
                    },
                    attention_version: u64::MAX,
                    ordinary_version: 0,
                    with_attention: false,
                    retry_messages: false,
                    blocked_messages: vec![],
                    known_messages: vec![],
                    new_message_slots: 256,
                },
                &budget(),
            )
            .unwrap();
        let work = &output.work;
        assert!(
            work.initial <= 40
                && work.context <= 80 - work.initial
                && work.revisit <= 100 - work.initial - work.context
                && work.initial + work.context + work.revisit <= 100
                && work.ordinary <= 100
                && work.notices <= 17
                && work.notice_validations <= 16
                && work.replay <= 32,
            "{work:?}"
        );
        assert!(
            work.output_passes <= 352
                && work.output_bytes <= 352 * 65537
                && work.rpc_passes == 1
                && work.rpc_bytes <= crate::delivery::presentation::RPC_RESULT_LIMIT + 1
        );
        max_passes = max_passes.max(work.output_passes);
        let result = CommandResult::DeliveryPulledV2(output.reply.clone());
        let encoded_result = serde_json::to_vec(&result).unwrap();
        assert_eq!(work.rpc_bytes, encoded_result.len());
        let response = crate::protocol::wire::WireResponse {
            version: crate::protocol::wire::PROTOCOL_VERSION,
            request_id: "\"".repeat(128),
            instance: uuid::Uuid::from_u128(1).to_string(),
            daemon_boot: uuid::Uuid::from_u128(2).to_string(),
            result: Ok(result),
        };
        let encoded = crate::protocol::wire::encode_wire_response(&response).unwrap();
        assert!(
            serde_json::from_slice::<crate::protocol::wire::WireResponse>(&encoded[4..]).is_ok()
        );
        assert_eq!(
            u32::from_be_bytes(encoded[..4].try_into().unwrap()) as usize,
            encoded.len() - 4
        );
        assert!(
            encoded.len() - encoded_result.len() <= 512
                && encoded.len() <= crate::protocol::wire::MAX_WIRE_FRAME_BYTES
        );
        let DeliveryPullReplyV2::Page(page) = output.reply else {
            break;
        };
        let (_, actual) = crate::delivery::presentation::measure(&profile, &page.items).unwrap();
        assert_eq!(actual, page.output_bytes as usize);
        for invitation in presentation_invitations(&page) {
            if !invitation.content.truncated {
                all.insert(invitation.id);
            }
        }
        presentation_test_confirm(&f, &recipient, &page);
        if all.len() == 100 {
            break;
        }
    }
    assert_eq!(all.len(), 100);
    assert!(
        max_passes > 32,
        "actual candidate sizing, not only one final serializer"
    );
    eprintln!(
        "actual_candidate_max_output_passes={max_passes}; declared_bound=352; wrapper_reserve=512"
    );
}

#[test]
fn presentation_handler_clears_unconfirmed_epoch_then_reopens_new_episode() {
    use crate::protocol::{
        commands::{Accept, CreateThread, InvitationConfirmation, Invite},
        delivery::{DeliveryPullReplyV2, PresentationControlStampV2 as Stamp},
    };
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let inviter = check_in(
        &f,
        fixture_claim(&f, "x", "px", fourth(), "episode-inviter"),
        "episode-ci",
    );
    let invite = |suffix: &str| {
        let CommandResult::ThreadCreated(thread) = f
            .service
            .handle(
                Command::CreateThread(CreateThread {
                    name: None,
                    topic: "episode".into(),
                    goal: "review".into(),
                    operation: OperationId::new(format!("episode-thread-{suffix}")),
                    claim: inviter.clone(),
                }),
                peer(),
                &budget(),
            )
            .unwrap()
        else {
            panic!("thread")
        };
        f.service
            .handle(
                Command::Invite(Invite {
                    confirmation: Some(InvitationConfirmation::RequireAck),
                    thread: thread.clone(),
                    seat: recipient.seat.clone(),
                    deadline_millis: Some(1000),
                    operation: OperationId::new(format!("episode-invite-{suffix}")),
                    claim: inviter.clone(),
                }),
                peer(),
                &budget(),
            )
            .unwrap();
        thread
    };
    let first_thread = invite("first");
    let DeliveryPullReplyV2::Page(first) = presentation_test_pull(&f, &recipient) else {
        panic!("first")
    };
    let first_stamp = first
        .emitted_controls
        .iter()
        .find(|v| matches!(v, Stamp::Attention { .. }))
        .unwrap()
        .clone();
    let Stamp::Attention {
        epoch: first_epoch, ..
    } = &first_stamp
    else {
        unreachable!()
    };
    // Acceptance closes the condition before any emission confirmation. Clear
    // must still retire the possibly emitted epoch, retaining original slots.
    f.service
        .handle(
            Command::Accept(Accept {
                thread: first_thread.clone(),
                operation: OperationId::new("episode-accept"),
                claim: recipient.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    // Retained canonical warning-job rows whose publications have not arrived
    // force bounded proof progress before exact absence. They are fixture
    // source rows, not a claim that public commands created 301 warnings.
    f.db.execute("WITH RECURSIVE n(x) AS(SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<301) INSERT INTO warning_jobs(warning_id,event_seq,thread_id,interval_high_water,affected_seat_id,condition_kind,condition_id) SELECT 'episode-unpublished-'||x,20,?1,1000000,'s','invitation',?2 FROM n",rusqlite::params![first_thread.as_str(),presentation_invitations(&first)[0].id.as_str()]).unwrap();
    let DeliveryPullReplyV2::Page(unknown) = presentation_test_pull(&f, &recipient) else {
        panic!("bounded Unknown replay")
    };
    assert_eq!(
        unknown.presentation_token, first.presentation_token,
        "Unknown must preserve original outbox identity"
    );
    assert!(
        unknown.emitted_controls.is_empty() && unknown.more,
        "Unknown withholds Attention while progressing proof"
    );
    let mut clear = None;
    let mut diagnostics = Vec::new();
    // Two indexed sources each retain301 rows. Allow one source and one
    // fallback microstep per row, each scheduled within13 eligible calls,
    // plus13 startup slots. This is a finite test guard, not a delivery cap.
    let convergence_guard = 13 * (2 * 2 * 301 + 13);
    for quantum in 0..convergence_guard {
        let reply = presentation_test_pull(&f, &recipient);
        let work = f.service.presentation.last_work.lock().unwrap().unwrap();
        assert_eq!(
            work.relevance_calls, 1,
            "each successful handler call advances exactly once"
        );
        assert!(
            work.relevance_sources + work.relevance_fallback <= 100 && work.relevance_seeks <= 26
        );
        let fence: i64 =
            f.db.query_row(
                "SELECT version FROM presentation_warning_fence WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let proof:String=f.db.query_row("SELECT proof_json FROM presentation_warning_proofs WHERE seat_id='s' AND consumer='hook'",[],|r|r.get(0)).unwrap();
        let diagnostic = serde_json::json!({"quantum":quantum,"work":format!("{work:?}"),"fence":fence,"proof":serde_json::from_str::<serde_json::Value>(&proof).unwrap()});
        if diagnostics.len() < 32 {
            diagnostics.push(diagnostic);
        } else {
            diagnostics[31] = diagnostic;
        }
        match reply {
            DeliveryPullReplyV2::Page(page) => {
                if page
                    .emitted_controls
                    .iter()
                    .any(|v| matches!(v,Stamp::Cleared{epoch,..} if epoch==first_epoch))
                {
                    clear = Some(page);
                    break;
                }
                assert!(
                    !page.emitted_controls.contains(&first_stamp),
                    "closed Attention cannot replay"
                );
                if !page.items.is_empty() {
                    presentation_test_confirm(&f, &recipient, &page);
                }
            }
            DeliveryPullReplyV2::Unchanged { .. } => {}
            other => panic!("{other:?}"),
        }
    }
    let clear = clear.unwrap_or_else(|| {
        panic!(
            "unconfirmed allocated Attention receives exact scoped clear; diagnostics={}",
            serde_json::to_string(&diagnostics).unwrap()
        )
    });
    assert!(!clear.emitted_controls.contains(&first_stamp));
    presentation_test_confirm(&f, &recipient, &clear);
    invite("second");
    let mut reopened = None;
    for _ in 0..32 {
        if let DeliveryPullReplyV2::Page(page) = presentation_test_pull(&f, &recipient) {
            reopened = page
                .emitted_controls
                .iter()
                .find(|v| matches!(v, Stamp::Attention { .. }))
                .cloned();
            presentation_test_confirm(&f, &recipient, &page);
            if reopened.is_some() {
                break;
            }
        }
    }
    let Stamp::Attention { epoch, .. } = reopened.expect("new canonical open receives new episode")
    else {
        unreachable!()
    };
    assert_ne!(
        &epoch, first_epoch,
        "presentation epoch never reuses a retired epoch"
    );
}

#[test]
fn presentation_handler_published_warn_reopens_at_unchanged_native_version() {
    use crate::{
        ports::{
            ServiceAuthorityGate, ServiceConnectionAuthority, ServiceDecisionGuard,
            ServiceWriteTransactionProof,
        },
        protocol::{
            commands::{Accept, CreateThread, InvitationConfirmation, Invite},
            delivery::{DeliveryPullReplyV2, PresentationControlStampV2 as Stamp},
            results::ApiError,
            service::{NotificationSeverity, ServiceNotify, ServiceOperation, ServiceResult},
        },
    };
    struct Gate(ServiceAuthorId);
    struct Guard(ServiceAuthorId);
    impl ServiceDecisionGuard for Guard {
        fn author(&self) -> &ServiceAuthorId {
            &self.0
        }
    }
    impl ServiceAuthorityGate for Gate {
        fn register(
            &self,
            instance: &str,
            boot: &str,
            author: ServiceAuthorId,
        ) -> Result<ServiceConnectionAuthority, ApiError> {
            Ok(ServiceConnectionAuthority::new(
                instance.into(),
                boot.into(),
                1,
                author,
            ))
        }
        fn decision_guard<'a>(
            &'a self,
            _: &ServiceWriteTransactionProof,
            connection: &ServiceConnectionAuthority,
        ) -> Result<Box<dyn ServiceDecisionGuard + 'a>, ApiError> {
            assert_eq!(connection.author(), &self.0);
            Ok(Box::new(Guard(self.0.clone())))
        }
        fn revoke_exact(&self, _: &ServiceConnectionAuthority) -> bool {
            true
        }
    }
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let inviter = check_in(
        &f,
        fixture_claim(&f, "x", "px", fourth(), "warn-epoch-inviter"),
        "warn-epoch-ci",
    );
    let CommandResult::ThreadCreated(thread) = f
        .service
        .handle(
            Command::CreateThread(CreateThread {
                name: None,
                topic: "first epoch".into(),
                goal: "review".into(),
                operation: OperationId::new("warn-epoch-thread"),
                claim: inviter.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("thread")
    };
    f.service
        .handle(
            Command::Invite(Invite {
                confirmation: Some(InvitationConfirmation::RequireAck),
                thread: thread.clone(),
                seat: recipient.seat.clone(),
                deadline_millis: Some(1000),
                operation: OperationId::new("warn-epoch-invite"),
                claim: inviter,
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    let DeliveryPullReplyV2::Page(first) = presentation_test_pull(&f, &recipient) else {
        panic!("first Attention")
    };
    let first_epoch = first
        .emitted_controls
        .iter()
        .find_map(|v| match v {
            Stamp::Attention { epoch, .. } => Some(epoch.clone()),
            _ => None,
        })
        .unwrap();
    presentation_test_confirm(&f, &recipient, &first);
    f.service
        .handle(
            Command::Accept(Accept {
                thread,
                operation: OperationId::new("warn-epoch-accept"),
                claim: recipient.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    let mut clear_version = None;
    for _ in 0..32 {
        if let DeliveryPullReplyV2::Page(page) = presentation_test_pull(&f, &recipient) {
            if page
                .emitted_controls
                .iter()
                .any(|v| matches!(v,Stamp::Cleared{epoch,..} if epoch==&first_epoch))
            {
                clear_version = Some(page.attention_version);
            }
            if !page.items.is_empty() {
                presentation_test_confirm(&f, &recipient, &page);
            }
            if clear_version.is_some() {
                break;
            }
        }
    }
    let prior_version = clear_version.expect("first exact epoch cleared");
    let native_version = || {
        f.db.query_row(
            "SELECT attention_version FROM wake_work WHERE seat_id='s'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
    };
    let before = native_version();
    let tx = f.db.unchecked_transaction().unwrap();
    let author =
        crate::store::service_substrate::ensure_reserved_author(&tx, "i", UtcMillis(0)).unwrap();
    tx.commit().unwrap();
    let gate = Gate(author.clone());
    let connection = ServiceConnectionAuthority::new("i".into(), "boot".into(), 1, author);
    let ServiceResult::Notification(published) = f
        .store
        .service_operation(
            ServiceOperation::Notify(ServiceNotify {
                thread: ThreadId::new("t"),
                severity: NotificationSeverity::Warn,
                event_json: serde_json::json!({"text":"new warning before materialization"}),
                operation: OperationId::new("warn-epoch-notify"),
            }),
            &connection,
            &gate,
            &budget(),
            None,
        )
        .unwrap()
    else {
        panic!("published Warn")
    };
    assert_eq!(
        native_version(),
        before,
        "canonical publication precedes its native materializer bump"
    );
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM messages WHERE id=?1 AND kind='warn'",
            [published.summary.message.as_str()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(f.db.query_row("SELECT count(*) FROM work_jobs WHERE kind='send_attention' AND subject_id LIKE 'service-notify:%' AND status='pending'",[],|r|r.get::<_,i64>(0)).unwrap(),1,"actual materialization remains pending");
    let mut reopened = None;
    for _ in 0..32 {
        if let DeliveryPullReplyV2::Page(page) = presentation_test_pull(&f, &recipient) {
            assert_eq!(
                page.attention_version, prior_version,
                "actual handler native metadata remains unchanged"
            );
            reopened = page.emitted_controls.iter().find_map(|v| match v {
                Stamp::Attention { epoch, .. } => Some(epoch.clone()),
                _ => None,
            });
            if !page.items.is_empty() {
                presentation_test_confirm(&f, &recipient, &page);
            }
            if reopened.is_some() {
                break;
            }
        }
    }
    assert_ne!(
        reopened.expect("published Warn reopens presentation relevance before materialization"),
        first_epoch
    );
    assert_eq!(
        native_version(),
        before,
        "V2 pull/Confirm never runs native materialization"
    );
}

#[test]
fn presentation_quiet_channel_observes_new_lazy_ordinary_without_wake() {
    use crate::{
        ports::{ModChannelSink, ModStoreReads},
        protocol::{
            commands::{DeliveryMode, SendMessage},
            watch::{ModDeliverySetting, WatchFrame, WatchRequest},
        },
        service::mod_channels::ModChannelRegistry,
    };
    #[derive(Default)]
    struct Sink(std::sync::Mutex<Vec<WatchFrame>>);
    impl ModChannelSink for Sink {
        fn push(&self, frame: WatchFrame) -> bool {
            self.0.lock().unwrap().push(frame);
            true
        }
    }
    let mut f = fixture(0);
    let recipient = check_in(
        &f,
        fixture_claim(&f, "c", "pc", Harness::Claude, "quiet-lazy-recipient"),
        "quiet-lazy-ci",
    );
    let sender = check_in(
        &f,
        fixture_claim(&f, "x", "px", fourth(), "quiet-lazy-sender"),
        "quiet-lazy-sender-ci",
    );
    let reads: Arc<dyn ModStoreReads> = f.store.clone();
    let registry = Arc::new(ModChannelRegistry::new(
        f.clock.clone(),
        reads,
        "i".into(),
        Arc::new(|| {}),
        ModDeliverySetting::On,
    ));
    f.service = DomainService::new("i".into(), f.store.clone(), f.clock.clone())
        .with_cooperative_owner(501, Arc::new(FairWriter::new(32)))
        .with_delivery_pull(f.state.clone())
        .with_mod_channels(registry.clone());
    let sink = Arc::new(Sink::default());
    let (_channel, _) = f
        .service
        .watch_register(
            &WatchRequest {
                presentation_v2: true,
                claim: recipient.clone(),
            },
            sink.clone(),
            &budget(),
        )
        .unwrap();
    registry.pass(UtcMillis(0)).unwrap();
    sink.0.lock().unwrap().clear();
    let native = || {
        f.db.query_row(
            "SELECT COALESCE((SELECT attention_version FROM wake_work WHERE seat_id='c'),0)",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
    };
    let before = native();
    f.service
        .handle(
            Command::SendMessage(SendMessage {
                delivery_mode: DeliveryMode::Lazy,
                user_intent: None,
                thread: ThreadId::new("t"),
                body: "new lazy content after quiet registration".into(),
                direct_recipient: None,
                invited_recipients: vec![],
                deadline_millis: None,
                operation: OperationId::new("quiet-lazy-send"),
                claim: sender,
                relays_user: false,
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    assert_eq!(f.db.query_row("SELECT count(*) FROM lazy_recipients r JOIN send_manifests sm ON sm.preparation_id=r.preparation_id AND sm.message_id=r.message_id WHERE r.seat_id='c' AND r.state='pending'",[],|r|r.get::<_,i64>(0)).unwrap(),1,"actual canonical lazy publication");
    assert_eq!(
        native(),
        before,
        "ordinary Lazy does not create native attention"
    );
    registry.pass(UtcMillis(1)).unwrap();
    let frames = sink.0.lock().unwrap().clone();
    assert!(
        matches!(frames.as_slice(), [WatchFrame::Attention { .. }]),
        "new recipient-visible Lazy content needs one passive in-turn drain after quiet registration: {frames:?}"
    );
    registry.pass(UtcMillis(2)).unwrap();
    assert_eq!(
        *sink.0.lock().unwrap(),
        frames,
        "unchanged lazy content does not repeat passive drains"
    );
    assert_eq!(
        native(),
        before,
        "passive observation preserves native eligibility"
    );
}

#[test]
fn presentation_confirm_rejects_oversized_private_json_before_decoding() {
    use crate::protocol::delivery::{
        DeliveryPresentationConfirmV2, DeliveryPullReplyV2, PresentationConsumer,
    };
    for (field, limit) in [
        ("last_descriptors", 4096),
        ("control_state", 4096),
        ("outbox_descriptors", 262144),
        ("outbox_prefix", 24601),
        ("outbox_selection", 128),
        ("outbox_text", 65536),
        ("ordinary_pending", 155731),
        ("ordinary_partial", 49415),
        ("ordinary_version", 20),
        ("outbox_version", 20),
        ("ordinary_cursor", 8192),
        ("ordinary_skip_body", 128),
        ("execution", 128),
        ("mod_execution", 128),
        ("render_key", 64),
        ("outbox_render_key", 64),
        ("outbox_codec_key", 64),
    ] {
        let f = fixture(0);
        let recipient = pull_seat(&f);
        let inviter = check_in(
            &f,
            fixture_claim(&f, "x", "px", fourth(), "private-bound-inviter"),
            "private-bound-ci",
        );
        canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 1, "private-bound");
        let DeliveryPullReplyV2::Page(page) = presentation_test_pull(&f, &recipient) else {
            panic!("canonical page")
        };
        presentation_test_confirm(&f, &recipient, &page);
        let original=f.db.query_row(&format!("SELECT {field} FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'"),[],|r|r.get::<_,Option<String>>(0)).unwrap().unwrap_or_default();
        let corrupted = original + &" ".repeat(limit + 1);
        f.db.execute_batch("PRAGMA ignore_check_constraints=ON")
            .unwrap();
        f.db.execute(&format!("UPDATE invitation_presentation_consumers SET {field}=?1 WHERE seat_id='s' AND consumer='hook'"),[corrupted]).unwrap();
        f.db.execute_batch("PRAGMA ignore_check_constraints=OFF")
            .unwrap();
        let result = f.store.presentation_confirm(
            &DeliveryPresentationConfirmV2 {
                claim: recipient,
                consumer: PresentationConsumer::Hook,
                token: page.presentation_token.unwrap(),
                emitted_controls: vec![],
                presentation_lease: None,
            },
            &budget(),
        );
        assert!(
            matches!(
                result,
                Err(crate::protocol::results::ApiError {
                    code: crate::protocol::results::ErrorCode::StoreCorrupt,
                    ..
                })
            ),
            "{field} must enforce declared{limit}byte bound before allocation/parsing/effects: {result:?}"
        );
    }
}

#[test]
fn presentation_last_token_control_serializer_has_exact_bounded_witness() {
    use crate::protocol::delivery::PresentationControlStampV2 as Stamp;
    let stamps = (0..32)
        .map(|n| Stamp::Attention {
            epoch: (u64::MAX - n).to_string(),
            version: u64::MAX,
        })
        .collect::<Vec<_>>();
    crate::protocol::delivery::validate_control_stamps(&stamps).unwrap();
    for stamp in &stamps {
        assert_eq!(serde_json::to_vec(stamp).unwrap().len(), 82);
    }
    let serialized = serde_json::to_vec(&stamps).unwrap();
    assert_eq!(serialized.len(), 32 * 82 + 31 + 2);
    assert_eq!(serialized.len(), 2657);
    assert!(serialized.len() <= 4096);
    assert_eq!(
        serde_json::from_slice::<Vec<Stamp>>(&serialized).unwrap(),
        stamps
    );
}

#[test]
fn presentation_handler_partial_confirmation_retains_canonical_slots_across_restart() {
    use crate::protocol::{
        commands::{CreateThread, InvitationConfirmation, Invite},
        delivery::{
            DeliveryPresentationConfirmV2, DeliveryPresentationConfirmationV2 as Status,
            DeliveryPullReplyV2, PresentationConsumer, PresentationControlStampV2 as Stamp,
            PresentationItemV2 as Item,
        },
    };
    let mut f = fixture(0);
    let recipient = pull_seat(&f);
    let inviter = check_in(
        &f,
        fixture_claim(&f, "x", "px", fourth(), "public-subset-inviter"),
        "public-subset-ci",
    );
    let CommandResult::ThreadCreated(thread) = f
        .service
        .handle(
            Command::CreateThread(CreateThread {
                name: None,
                topic: "canonical partial confirmation".into(),
                goal: "review".into(),
                operation: OperationId::new("public-subset-thread"),
                claim: inviter.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("thread")
    };
    f.service
        .handle(
            Command::Invite(Invite {
                confirmation: Some(InvitationConfirmation::RequireAck),
                thread,
                seat: recipient.seat.clone(),
                deadline_millis: Some(1000),
                operation: OperationId::new("public-subset-invite"),
                claim: inviter,
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    let DeliveryPullReplyV2::Page(page) = presentation_test_pull(&f, &recipient) else {
        panic!("canonical page")
    };
    assert_eq!(page.emitted_controls.len(), 1);
    assert!(
        page.items
            .iter()
            .any(|v| matches!(v, Item::Invitation { .. }))
    );
    let token = page.presentation_token.clone().unwrap();
    let stamp = page.emitted_controls[0].clone();
    let row = |f: &Fixture| {
        f.db.query_row("SELECT outbox_descriptors,outbox_retired_mask,outbox_confirmed_mask,control_state,last_descriptors,last_confirmed_mask FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,i64>(5)?))).unwrap()
    };
    let original = row(&f);
    let descriptors: Vec<serde_json::Value> = serde_json::from_str(&original.0).unwrap();
    let control_slot = descriptors
        .iter()
        .position(|v| v["item"]["kind"] == "attention")
        .unwrap();
    let allowed = (1i64 << descriptors.len()) - 1;
    let noncontrols = allowed & !(1i64 << control_slot);
    let confirm = |f: &Fixture, emitted_controls: Vec<Stamp>| {
        f.service.handle(
            Command::DeliveryPresentationConfirmV2(DeliveryPresentationConfirmV2 {
                claim: recipient.clone(),
                consumer: PresentationConsumer::Hook,
                token: token.clone(),
                presentation_lease: None,
                emitted_controls,
            }),
            peer(),
            &budget(),
        )
    };
    let CommandResult::DeliveryPresentationConfirmedV2(empty) = confirm(&f, vec![]).unwrap() else {
        panic!("confirm")
    };
    assert_eq!(empty.status, Status::Confirmed);
    assert!(empty.acknowledged_controls.is_empty());
    let partial = row(&f);
    assert_eq!(partial.0, original.0);
    assert_eq!((partial.1, partial.2), (noncontrols, noncontrols));
    restart(&mut f);
    let DeliveryPullReplyV2::Page(replay) = presentation_test_pull(&f, &recipient) else {
        panic!("canonical replay")
    };
    assert_eq!(replay.presentation_token, Some(token.clone()));
    assert_eq!(replay.emitted_controls, vec![stamp.clone()]);
    assert!(
        replay
            .items
            .iter()
            .all(|v| matches!(v, Item::Attention { .. }))
    );
    assert_eq!(
        row(&f),
        partial,
        "replay preserves original slots and masks"
    );
    let foreign = match &stamp {
        Stamp::Attention { epoch, version } => Stamp::Attention {
            epoch: epoch.clone(),
            version: version + 1,
        },
        _ => panic!("attention"),
    };
    for invalid in [vec![foreign], vec![stamp.clone(), stamp.clone()]] {
        assert!(confirm(&f, invalid).is_err());
        assert_eq!(
            row(&f),
            partial,
            "invalid capture cannot mutate slots or controls"
        );
    }
    let CommandResult::DeliveryPresentationConfirmedV2(reply) =
        confirm(&f, vec![stamp.clone()]).unwrap()
    else {
        panic!("confirm")
    };
    assert_eq!(reply.status, Status::Confirmed);
    assert_eq!(reply.acknowledged_controls, vec![stamp.clone()]);
    restart(&mut f);
    let CommandResult::DeliveryPresentationConfirmedV2(retry) =
        confirm(&f, vec![stamp.clone()]).unwrap()
    else {
        panic!("retry")
    };
    assert_eq!(retry.status, Status::AlreadyConfirmed);
    assert_eq!(
        retry.acknowledged_controls,
        vec![stamp],
        "lost reply retry acknowledges the original still-applicable capture"
    );
    assert_eq!(
        f.db.query_row("SELECT count(*) FROM receipts", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0,
        "presentation confirmation is not message ACK"
    );
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM invitations WHERE accepted_at IS NOT NULL",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0,
        "presentation confirmation is not invitation acceptance"
    );
}

#[test]
fn presentation_private_state_rejects_typed_corruption_without_confirmation_effects() {
    use crate::protocol::delivery::{
        DeliveryPresentationConfirmV2, DeliveryPullReplyV2, PresentationConsumer,
    };
    for (field, value) in [
        ("last_descriptors", "null"),
        ("last_descriptors", "7"),
        ("last_descriptors", "["),
        ("control_state", "null"),
        ("control_state", "{\"unknown\":true}"),
        ("control_state", "{\"counter\":\"-1\"}"),
        ("last_confirmed_mask", "-1"),
        ("last_confirmed_mask", "1"),
    ] {
        let f = fixture(0);
        let recipient = pull_seat(&f);
        let inviter = check_in(
            &f,
            fixture_claim(&f, "x", "px", fourth(), "typed-corrupt-inviter"),
            "typed-corrupt-ci",
        );
        canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 1, "typed-corrupt");
        let DeliveryPullReplyV2::Page(page) = presentation_test_pull(&f, &recipient) else {
            panic!("canonical page")
        };
        presentation_test_confirm(&f, &recipient, &page);
        f.db.execute_batch("PRAGMA ignore_check_constraints=ON")
            .unwrap();
        f.db.execute(&format!("UPDATE invitation_presentation_consumers SET {field}=?1 WHERE seat_id='s' AND consumer='hook'"),[value]).unwrap();
        f.db.execute_batch("PRAGMA ignore_check_constraints=OFF")
            .unwrap();
        let snapshot = || {
            f.db.query_row("SELECT last_token,last_descriptors,last_confirmed_mask,control_state FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,String>(3)?))).unwrap()
        };
        let before = snapshot();
        let result = f.store.presentation_confirm(
            &DeliveryPresentationConfirmV2 {
                claim: recipient,
                consumer: PresentationConsumer::Hook,
                token: page.presentation_token.unwrap(),
                emitted_controls: vec![],
                presentation_lease: None,
            },
            &budget(),
        );
        assert!(
            matches!(
                result,
                Err(crate::protocol::results::ApiError {
                    code: crate::protocol::results::ErrorCode::StoreCorrupt,
                    ..
                })
            ),
            "typed {field}={value} must refuse before effects: {result:?}"
        );
        assert_eq!(snapshot(), before);
        assert!(f.db.execute("UPDATE invitation_presentation_consumers SET last_descriptors=NULL WHERE seat_id='s' AND consumer='hook'",[]).is_err(),"STRICT NOT NULL rejects SQL NULL at storage boundary");
    }
}

#[test]
fn presentation_watch_fresh_confirmation_records_exact_owner_publication_cutoff_once() {
    use crate::{
        ports::{ModChannelSink, ModChannels, ModStoreReads},
        protocol::{
            commands::{
                Ack, CreateThread, DeliveryMode, InvitationConfirmation, Invite, SendMessage,
            },
            delivery::{
                DeliveryPresentationConfirmV2, DeliveryPresentationConfirmationV2 as Status,
                DeliveryPullReplyV2, DeliveryPullV2, PresentationConsumer,
                PresentationControlStampV2 as Stamp, PresentationRenderRequest,
            },
            watch::{MOD_STALL_AFTER_MS, ModDeliverySetting, WatchFrame, WatchRequest},
        },
        service::mod_channels::ModChannelRegistry,
    };
    struct Sink;
    impl ModChannelSink for Sink {
        fn push(&self, _: WatchFrame) -> bool {
            true
        }
    }
    let mut f = fixture(0);
    let recipient = check_in(
        &f,
        fixture_claim(&f, "c", "pc", Harness::Claude, "stall-v2-recipient"),
        "stall-v2-ci",
    );
    let sender = check_in(
        &f,
        fixture_claim(&f, "x", "px", fourth(), "stall-v2-sender"),
        "stall-v2-sender-ci",
    );
    let reads: Arc<dyn ModStoreReads> = f.store.clone();
    let registry = Arc::new(ModChannelRegistry::new(
        f.clock.clone(),
        reads,
        "i".into(),
        Arc::new(|| {}),
        ModDeliverySetting::On,
    ));
    f.service = DomainService::new("i".into(), f.store.clone(), f.clock.clone())
        .with_cooperative_owner(501, Arc::new(FairWriter::new(32)))
        .with_delivery_pull(f.state.clone())
        .with_mod_channels(registry.clone());
    let (channel, _) = f
        .service
        .watch_register(
            &WatchRequest {
                presentation_v2: true,
                claim: recipient.clone(),
            },
            Arc::new(Sink),
            &budget(),
        )
        .unwrap();
    let lease = registry.presentation_lease(channel).unwrap();
    let send = |f: &Fixture, key: &str| {
        let CommandResult::MessageSent(id) = f
            .service
            .handle(
                Command::SendMessage(SendMessage {
                    delivery_mode: DeliveryMode::Ordinary,
                    user_intent: None,
                    thread: ThreadId::new("t"),
                    body: key.into(),
                    direct_recipient: None,
                    invited_recipients: vec![],
                    deadline_millis: None,
                    operation: OperationId::new(key),
                    claim: sender.clone(),
                    relays_user: false,
                }),
                peer(),
                &budget(),
            )
            .unwrap()
        else {
            panic!("message")
        };
        id
    };
    let first_receipt = send(&f, "stall-v2-first");
    // Timed invitation supplies an independently canonical NUDGE condition;
    // this fixture proves mod health bookkeeping, not initial receipt relevance.
    let CommandResult::ThreadCreated(thread) = f
        .service
        .handle(
            Command::CreateThread(CreateThread {
                name: None,
                topic: "stall proof".into(),
                goal: "review".into(),
                operation: OperationId::new("stall-v2-thread"),
                claim: sender.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("thread")
    };
    f.service
        .handle(
            Command::Invite(Invite {
                confirmation: Some(InvitationConfirmation::RequireAck),
                thread,
                seat: recipient.seat.clone(),
                deadline_millis: Some(1000),
                operation: OperationId::new("stall-v2-invite"),
                claim: sender.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    f.clock
        .0
        .store(1_000_000 + MOD_STALL_AFTER_MS as i64 + 1, Ordering::Relaxed);
    let CommandResult::DeliveryPulledV2(DeliveryPullReplyV2::Page(page)) = f
        .service
        .handle(
            Command::DeliveryPullV2(DeliveryPullV2 {
                claim: recipient.clone(),
                consumer: PresentationConsumer::Watch,
                render: PresentationRenderRequest::WatchJsonLinesV1,
                presentation_lease: Some(lease.clone()),
                marker_prefix: vec![],
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("Watch page")
    };
    assert_eq!(
        page.emitted_controls
            .iter()
            .filter(|v| matches!(v, Stamp::Attention { .. }))
            .count(),
        1,
        "single-control producer dependency of fresh status"
    );
    assert_eq!(page.emitted_controls.len(), 1);
    assert!(
        !registry.stalled(&recipient.seat, f.clock.utc_now()),
        "allocation/replay is not physical emission evidence; no legacy observer pass in this fixture"
    );
    let request = DeliveryPresentationConfirmV2 {
        claim: recipient.clone(),
        consumer: PresentationConsumer::Watch,
        token: page.presentation_token.unwrap(),
        presentation_lease: Some(lease),
        emitted_controls: page.emitted_controls,
    };
    let CommandResult::DeliveryPresentationConfirmedV2(first) = f
        .service
        .handle(
            Command::DeliveryPresentationConfirmV2(request.clone()),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("confirm")
    };
    assert_eq!(first.status, Status::Confirmed);
    assert_eq!(first.acknowledged_controls, request.emitted_controls);
    assert!(
        registry.stalled(&recipient.seat, f.clock.utc_now()),
        "fresh exact Watch emission confirmation records receipt-publication cutoff, not a new countdown"
    );
    // Drop the first response as a lost-reply simulation. Isolate the original
    // qualifying receipt via a REAL ACK before testing the later cutoff.
    f.service
        .handle(
            Command::Ack(Ack {
                messages: vec![first_receipt],
                operation: OperationId::new("stall-v2-real-ack"),
                claim: recipient.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    f.clock.0.fetch_add(1000, Ordering::Relaxed);
    let _later_receipt = send(&f, "stall-v2-later");
    f.clock
        .0
        .fetch_add(MOD_STALL_AFTER_MS as i64 + 1, Ordering::Relaxed);
    assert!(
        !registry.stalled(&recipient.seat, f.clock.utc_now()),
        "later receipt is after original push cutoff; prior receipt settled and real ACK is now old"
    );
    let CommandResult::DeliveryPresentationConfirmedV2(retry) = f
        .service
        .handle(
            Command::DeliveryPresentationConfirmV2(request),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("retry")
    };
    assert_eq!(retry.status, Status::AlreadyConfirmed);
    assert!(!retry.acknowledged_controls.is_empty());
    assert!(
        !registry.stalled(&recipient.seat, f.clock.utc_now()),
        "AlreadyConfirmed lost-reply retry must not move publication cutoff"
    );
}

#[test]
fn presentation_publication_scalar_maximum_roundtrips_without_changing_carry() {
    use crate::protocol::delivery::DeliveryPullReplyV2;
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let inviter = check_in(
        &f,
        fixture_claim(&f, "x", "px", fourth(), "scalar-inviter"),
        "scalar-ci",
    );
    canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 1, "scalar");
    let DeliveryPullReplyV2::Page(page) = presentation_test_pull(&f, &recipient) else {
        panic!("page")
    };
    let carry:String=f.db.query_row("SELECT ordinary_pending FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'",[],|r|r.get(0)).unwrap();
    let fields = [
        "ordinary_publication_after",
        "ordinary_publication_high",
        "ordinary_passive_generation",
        "ordinary_scan_passive_generation",
    ];
    for field in fields {
        assert_eq!(f.db.query_row(&format!("SELECT {field} FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'"),[],|r|r.get::<_,i64>(0)).unwrap(),0);
    }
    f.db.execute("UPDATE invitation_presentation_consumers SET ordinary_publication_after=?1,ordinary_publication_high=?1,ordinary_passive_generation=?1,ordinary_scan_passive_generation=?1 WHERE seat_id='s' AND consumer='hook'",[i64::MAX]).unwrap();
    presentation_test_confirm(&f, &recipient, &page);
    let mut map = serde_json::Map::new();
    for field in fields {
        let(value,kind)=f.db.query_row(&format!("SELECT {field},typeof({field}) FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'"),[],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?))).unwrap();
        assert_eq!((value, kind), (i64::MAX, "integer".into()));
        map.insert(field.into(), serde_json::json!(value));
    }
    assert_eq!(serde_json::to_vec(&map).unwrap().len(), 203);
    let with_other = serde_json::json!({"existing":0});
    let old = serde_json::to_vec(&with_other).unwrap().len();
    let mut with_other = with_other.as_object().unwrap().clone();
    with_other.extend(map);
    assert_eq!(serde_json::to_vec(&with_other).unwrap().len() - old, 202);
    assert_eq!(f.db.query_row("SELECT ordinary_pending FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'",[],|r|r.get::<_,String>(0)).unwrap(),carry,"four scalars add zero carry bytes");
}

#[test]
fn presentation_publication_walk_counts_foreign_prefix_before_filter_and_progresses() {
    use crate::protocol::commands::{CreateThread, DeliveryMode, SendMessage};
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let sender = check_in(
        &f,
        fixture_claim(&f, "x", "px", fourth(), "prefix-sender"),
        "prefix-sender-ci",
    );
    let CommandResult::ThreadCreated(foreign) = f
        .service
        .handle(
            Command::CreateThread(CreateThread {
                name: None,
                topic: "foreign prefix".into(),
                goal: "bounded walk".into(),
                operation: OperationId::new("prefix-thread"),
                claim: sender.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("thread")
    };
    let send = |thread: ThreadId, n: usize| {
        f.service
            .handle(
                Command::SendMessage(SendMessage {
                    delivery_mode: DeliveryMode::Lazy,
                    user_intent: None,
                    thread,
                    body: format!("prefix-body-{n}"),
                    direct_recipient: None,
                    invited_recipients: vec![],
                    deadline_millis: None,
                    operation: OperationId::new(format!("prefix-send-{n}")),
                    claim: sender.clone(),
                    relays_user: false,
                }),
                peer(),
                &budget(),
            )
            .unwrap()
    };
    for n in 0..101 {
        send(foreign.clone(), n);
    }
    send(ThreadId::new("t"), 101);
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM send_manifests WHERE instance_id='i'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        102
    );
    let native = wake_attention(&f);
    let refresh = || {
        f.store
            .presentation_refresh(
                &recipient.seat,
                &binding_for_claim(&f, &recipient),
                recipient.binding_generation,
                recipient.native_session.as_str(),
                &budget(),
            )
            .unwrap()
            .unwrap()
    };
    let first = refresh();
    // Each valid physical manifest costs one exact message point plus its
    // mode-selected recipient point, even when that recipient is unrelated.
    assert_eq!(
        (
            first.ordinary_publication_visits,
            first.ordinary_publication_probes,
            first.ordinary_publication_seeks
        ),
        (100, 200, 2)
    );
    assert!(
        first.more,
        "foreign rows still require bounded maintenance continuation"
    );
    assert_eq!(
        first.version, 0,
        "no addressed publication in first quantum"
    );
    let second = refresh();
    assert_eq!(
        (
            second.ordinary_publication_visits,
            second.ordinary_publication_probes,
            second.ordinary_publication_seeks
        ),
        (2, 4, 2)
    );
    assert!(
        second.version > first.version,
        "addressed final row advances passive content"
    );
    let quiet = refresh();
    assert_eq!(
        (
            quiet.ordinary_publication_visits,
            quiet.ordinary_publication_probes,
            quiet.ordinary_publication_seeks
        ),
        (0, 0, 1)
    );
    assert_eq!(quiet.version, second.version);
    assert!(!quiet.more);
    assert_eq!(
        wake_attention(&f),
        native,
        "maintenance never adds native wake eligibility"
    );
    let mut bodies = Vec::new();
    for _ in 0..32 {
        match presentation_test_pull(&f, &recipient) {
            crate::protocol::delivery::DeliveryPullReplyV2::Page(page) => {
                bodies.push(page.text.clone());
                presentation_test_confirm(&f, &recipient, &page);
                if !page.more {
                    break;
                }
            }
            _ => break,
        }
    }
    assert!(bodies.join("\n").contains("prefix-body-101"));
    assert!(!bodies.join("\n").contains("prefix-body-100"));
}

#[test]
fn presentation_publication_invalid_scalars_refuse_before_confirmation_effects() {
    use crate::protocol::delivery::{
        DeliveryPresentationConfirmV2, DeliveryPullReplyV2, PresentationConsumer,
    };
    for (after, high, passive, scan) in [(2, 1, 0, 0), (0, 0, 1, 2)] {
        let f = fixture(0);
        let recipient = pull_seat(&f);
        let inviter = check_in(
            &f,
            fixture_claim(&f, "x", "px", fourth(), "invalid-scalar-inviter"),
            "invalid-scalar-ci",
        );
        canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 1, "invalid-scalar");
        let DeliveryPullReplyV2::Page(page) = presentation_test_pull(&f, &recipient) else {
            panic!("page")
        };
        f.db.execute("UPDATE invitation_presentation_consumers SET ordinary_publication_after=?1,ordinary_publication_high=?2,ordinary_passive_generation=?3,ordinary_scan_passive_generation=?4 WHERE seat_id='s' AND consumer='hook'",rusqlite::params![after,high,passive,scan]).unwrap();
        let snapshot = || {
            f.db.query_row("SELECT outbox_token,outbox_retired_mask,outbox_confirmed_mask FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?))).unwrap()
        };
        let before = snapshot();
        let result = f.store.presentation_confirm(
            &DeliveryPresentationConfirmV2 {
                claim: recipient,
                consumer: PresentationConsumer::Hook,
                token: page.presentation_token.unwrap(),
                emitted_controls: page.emitted_controls,
                presentation_lease: None,
            },
            &budget(),
        );
        assert!(matches!(
            result,
            Err(crate::protocol::results::ApiError {
                code: crate::protocol::results::ErrorCode::StoreCorrupt,
                ..
            })
        ));
        assert_eq!(snapshot(), before);
    }
}

#[test]
fn presentation_publication_refresh_and_pull_rebind_preserve_each_others_progress() {
    use crate::{
        ports::PresentationInput,
        protocol::{
            commands::{DeliveryMode, SendMessage},
            delivery::{DeliveryPullV2, PresentationConsumer},
        },
    };
    for (refresh_first, same_generation_repair) in
        [(false, false), (true, false), (false, true), (true, true)]
    {
        let f = fixture(0);
        let recipient = pull_seat(&f);
        let sender = check_in(
            &f,
            fixture_claim(&f, "x", "px", fourth(), "reset-publication-sender"),
            "reset-publication-ci",
        );
        for n in 0..201 {
            f.service
                .handle(
                    Command::SendMessage(SendMessage {
                        delivery_mode: DeliveryMode::Lazy,
                        user_intent: None,
                        thread: ThreadId::new("t"),
                        body: format!("reset body {n}"),
                        direct_recipient: None,
                        invited_recipients: vec![],
                        deadline_millis: None,
                        operation: OperationId::new(format!("reset-publication-{n}")),
                        claim: sender.clone(),
                        relays_user: false,
                    }),
                    peer(),
                    &budget(),
                )
                .unwrap();
        }
        let pull = |claim: &CallerClaim| {
            f.store.presentation_pull(PresentationInput { binding_id: binding_for_claim(&f, claim), owner_replay:None,ordinary_due_snapshot:None,frozen_render:presentation_watch_frozen(&f,claim),render_profile:crate::delivery::presentation::PresentationRenderProfile::WatchJsonLinesV1,pull:DeliveryPullV2{claim:claim.clone(),consumer:PresentationConsumer::Watch,render:crate::protocol::delivery::PresentationRenderRequest::WatchJsonLinesV1,presentation_lease:Some("0123456789abcdef0123456789abcdef".into()),marker_prefix:vec![]},attention_version:0,ordinary_version:0,with_attention:false,retry_messages:false,blocked_messages:vec![],known_messages:vec![],new_message_slots:256},&budget()).unwrap()
        };
        let refresh = |claim: &CallerClaim| {
            f.store
                .presentation_refresh(
                    &claim.seat,
                    &binding_for_claim(&f, claim),
                    claim.binding_generation,
                    claim.native_session.as_str(),
                    &budget(),
                )
                .unwrap()
                .unwrap()
        };
        let initial = refresh(&recipient);
        assert_eq!(
            (
                initial.ordinary_publication_visits,
                initial.ordinary_publication_probes,
                initial.ordinary_publication_seeks
            ),
            (100, 200, 2),
            "all addressed candidates reach both indexed probes"
        );
        let scalar = |claim: &CallerClaim| {
            f.db.query_row("SELECT ordinary_publication_after,ordinary_publication_high,ordinary_passive_generation FROM invitation_presentation_consumers WHERE binding_id=?1 AND consumer='watch'",[binding_for_claim(&f, claim).as_str()],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?))).unwrap()
        };
        assert_eq!(scalar(&recipient).2, 1);
        assert!(scalar(&recipient).0 > 0 && scalar(&recipient).0 < scalar(&recipient).1);
        let next = if same_generation_repair {
            // Explicit canonical repair fixture: preserve generation/session,
            // replace execution, and prove private ownership compares BOTH.
            let next = fresh_execution(recipient.clone(), 2);
            f.db.execute("UPDATE occupant_bindings SET execution_id=?1 WHERE seat_id='s' AND generation=?2 AND ended_at IS NULL",rusqlite::params![next.execution.as_str(),next.binding_generation as i64]).unwrap();
            assert_eq!(next.binding_generation, recipient.binding_generation);
            assert_ne!(next.execution, recipient.execution);
            next
        } else {
            let next = rotate(&f, &recipient, "publication-reset-native", 2);
            assert_eq!(next.binding_generation, recipient.binding_generation);
            assert_ne!(
                binding_for_claim(&f, &next),
                binding_for_claim(&f, &recipient)
            );
            next
        };
        if refresh_first {
            refresh(&next);
        } else {
            pull(&next);
        }
        let first = scalar(&next);
        assert!(first.0 > 0 && first.0 < first.1);
        assert_eq!(first.2, 1, "first new owner initializes exactly once");
        if refresh_first {
            pull(&next);
        } else {
            refresh(&next);
        }
        let second = scalar(&next);
        assert!(
            second.0 > first.0,
            "second owner keeps positive cursor, continues instead of resetting"
        );
        assert_eq!(second.1, first.1);
        assert_eq!(second.2, 2);
        // Reopen the canonical store and continue the same finite frontier.
        let context = StoreContext::new(f.directory.join("store.db"), f.clock.clone());
        let _reopened = context.open_writer().unwrap();
        refresh(&next);
        let done = scalar(&next);
        assert_eq!(done.0, done.1);
        assert_eq!(done.2, 3);
        let before = scalar(&next);
        refresh(&next);
        assert_eq!(
            scalar(&next),
            before,
            "quiet refresh preserves completed progress"
        );
    }
}

#[test]
fn presentation_private_context_cursors_refuse_invalid_thread_ids_before_effects() {
    use crate::protocol::delivery::{
        DeliveryPresentationConfirmV2, DeliveryPullReplyV2, PresentationConsumer,
    };
    for field in [
        "context_after_thread",
        "context_thread",
        "mod_context_after_thread",
        "mod_context_thread",
    ] {
        for bad in ["x".repeat(129), "bad thread".into(), "λ".into()] {
            let f = fixture(0);
            let recipient = pull_seat(&f);
            let inviter = check_in(
                &f,
                fixture_claim(&f, "x", "px", fourth(), "cursor-bound-inviter"),
                "cursor-bound-ci",
            );
            canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 1, "cursor-bound");
            let DeliveryPullReplyV2::Page(page) = presentation_test_pull(&f, &recipient) else {
                panic!("page")
            };
            f.db.execute(&format!("UPDATE invitation_presentation_consumers SET {field}=?1 WHERE seat_id='s' AND consumer='hook'"),[bad]).unwrap();
            let snapshot = || {
                f.db.query_row("SELECT outbox_token,outbox_retired_mask,outbox_confirmed_mask,context_after_thread,context_thread,mod_context_after_thread,mod_context_thread FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?,r.get::<_,String>(6)?))).unwrap()
            };
            let before = snapshot();
            let result = f.store.presentation_confirm(
                &DeliveryPresentationConfirmV2 {
                    claim: recipient,
                    consumer: PresentationConsumer::Hook,
                    token: page.presentation_token.unwrap(),
                    emitted_controls: page.emitted_controls,
                    presentation_lease: None,
                },
                &budget(),
            );
            assert!(
                matches!(
                    result,
                    Err(crate::protocol::results::ApiError {
                        code: crate::protocol::results::ErrorCode::StoreCorrupt,
                        ..
                    })
                ),
                "{field} invalid cursor must refuse before effects: {result:?}"
            );
            assert_eq!(snapshot(), before);
        }
    }
}

#[test]
fn presentation_private_item_refresh_text_bounds_refuse_before_effects() {
    for (field, bad) in [
        ("mod_key", "x".repeat(4097)),
        ("mod_execution", "x".repeat(129)),
    ] {
        let f = fixture(0);
        let recipient = pull_seat(&f);
        let inviter = check_in(
            &f,
            fixture_claim(&f, "x", "px", fourth(), "item-bound-inviter"),
            "item-bound-ci",
        );
        canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 1, "item-bound");
        f.store
            .presentation_refresh(
                &recipient.seat,
                &binding_for_claim(&f, &recipient),
                recipient.binding_generation,
                recipient.native_session.as_str(),
                &budget(),
            )
            .unwrap()
            .unwrap();
        f.db.execute_batch("PRAGMA ignore_check_constraints=ON")
            .unwrap();
        f.db.execute(&format!("UPDATE invitation_presentation_items SET {field}=?1 WHERE seat_id='s' AND consumer='watch'"),[bad]).unwrap();
        f.db.execute_batch("PRAGMA ignore_check_constraints=OFF")
            .unwrap();
        let snapshot = || {
            f.db.query_row("SELECT mod_version,mod_cleanup_after,mod_cleanup_high,mod_initial_after FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='watch'",[],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?))).unwrap()
        };
        let before = snapshot();
        let result = f.store.presentation_refresh(
            &recipient.seat,
            &binding_for_claim(&f, &recipient),
            recipient.binding_generation,
            recipient.native_session.as_str(),
            &budget(),
        );
        assert!(
            matches!(
                result,
                Err(crate::protocol::results::ApiError {
                    code: crate::protocol::results::ErrorCode::StoreCorrupt,
                    ..
                })
            ),
            "private {field} must refuse before effects: {result:?}"
        );
        assert_eq!(snapshot(), before);
    }
}

#[test]
fn presentation_publication_signal_preserves_active_capture_and_replays_across_restart() {
    use crate::protocol::{
        commands::{DeliveryMode, SendMessage},
        delivery::DeliveryPullReplyV2,
    };
    let mut f = fixture(0);
    let recipient = pull_seat(&f);
    let sender = check_in(
        &f,
        fixture_claim(&f, "x", "px", fourth(), "capture-lazy-sender"),
        "capture-lazy-ci",
    );
    let send = |f: &Fixture, key: &str, body: String| {
        f.service
            .handle(
                Command::SendMessage(SendMessage {
                    delivery_mode: DeliveryMode::Lazy,
                    user_intent: None,
                    thread: ThreadId::new("t"),
                    body,
                    direct_recipient: None,
                    invited_recipients: vec![],
                    deadline_millis: None,
                    operation: OperationId::new(key),
                    claim: sender.clone(),
                    relays_user: false,
                }),
                peer(),
                &budget(),
            )
            .unwrap()
    };
    canonical_lazy_invitation_backlog(&f, &recipient, &sender, 1, "capture-outbox");
    send(&f, "capture-large", "z".repeat(65536));
    let DeliveryPullReplyV2::Page(first) = presentation_test_pull(&f, &recipient) else {
        panic!("large body page")
    };
    let scalar = |f: &Fixture| {
        f.db.query_row("SELECT ordinary_scan_passive_generation,ordinary_passive_generation,ordinary_version,ordinary_cursor,ordinary_pending,ordinary_partial FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'",[],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?))).unwrap()
    };
    let frozen_text:String=f.db.query_row("SELECT outbox_text FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'",[],|r|r.get(0)).unwrap();
    assert!(
        first.presentation_token.is_some(),
        "canonical nonordinary invitation owns replay token"
    );
    let captured = scalar(&f);
    assert_eq!((captured.0, captured.1), (1, 1));
    assert!(
        captured.3.is_some(),
        "actual bounded large body capture remains active"
    );
    send(&f, "capture-following", "later-publication-body".into());
    let DeliveryPullReplyV2::Page(replay) = presentation_test_pull(&f, &recipient) else {
        panic!("immutable pending output replay")
    };
    assert_eq!(replay.presentation_token, first.presentation_token);
    assert_eq!(
        replay.text, frozen_text,
        "only frozen nonordinary projection is replayed; ordinary uses its existing display ledger"
    );
    let newer = scalar(&f);
    assert_eq!((newer.0, newer.1), (1, 2));
    assert_eq!(
        (&newer.2, &newer.3, &newer.4, &newer.5),
        (&captured.2, &captured.3, &captured.4, &captured.5),
        "new signal must not reset active START tuple/body/carry"
    );
    restart(&mut f);
    let DeliveryPullReplyV2::Page(restarted) = presentation_test_pull(&f, &recipient) else {
        panic!("restart replay")
    };
    assert_eq!(restarted.text, frozen_text);
    presentation_test_confirm(&f, &recipient, &restarted);
    let mut later = false;
    for _ in 0..32 {
        match presentation_test_pull(&f, &recipient) {
            DeliveryPullReplyV2::Page(page) => {
                later |= page.text.contains("later-publication-body");
                presentation_test_confirm(&f, &recipient, &page);
                if later && !page.more {
                    break;
                }
            }
            DeliveryPullReplyV2::Unchanged { .. } => break,
            other => panic!("unexpected active capture reply: {other:?}"),
        }
    }
    assert!(
        later,
        "after frozen capture finishes, changed passive tuple starts next capture without losing following publication"
    );
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM lazy_recipients WHERE seat_id='s' AND state='pending'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2,
        "no synthetic lazy completion"
    );
}

fn finish_presentation_attention_job(f: &Fixture, job: &str) {
    for _ in 0..16 {
        let progress = f
            .store
            .advance_work(
                job,
                crate::ports::DurableWorkAdmission::new(1).unwrap(),
                &budget(),
            )
            .unwrap();
        assert!(progress.processed_this_turn <= 1);
        assert_eq!(progress.last_error, None);
        if !progress.has_more {
            return;
        }
    }
    panic!("fixture attention job did not finish within its finite recipient bound");
}

#[test]
fn presentation_registered_channel_materializes_each_published_warn_before_drain() {
    use crate::{
        ports::{
            ModChannelSink, ModChannels, ModStoreReads, ServiceAuthorityGate,
            ServiceConnectionAuthority, ServiceDecisionGuard, ServiceWriteTransactionProof,
        },
        protocol::{
            delivery::{
                DeliveryPullReplyV2, DeliveryPullV2, PresentationConsumer,
                PresentationControlStampV2, PresentationRenderRequest,
            },
            results::ApiError,
            service::{NotificationSeverity, ServiceNotify, ServiceOperation, ServiceResult},
            watch::{ModDeliverySetting, WatchFrame, WatchRequest},
        },
        service::mod_channels::ModChannelRegistry,
    };
    struct Gate(ServiceAuthorId);
    struct Guard(ServiceAuthorId);
    impl ServiceDecisionGuard for Guard {
        fn author(&self) -> &ServiceAuthorId {
            &self.0
        }
    }
    impl ServiceAuthorityGate for Gate {
        fn register(
            &self,
            instance: &str,
            boot: &str,
            author: ServiceAuthorId,
        ) -> Result<ServiceConnectionAuthority, ApiError> {
            Ok(ServiceConnectionAuthority::new(
                instance.into(),
                boot.into(),
                1,
                author,
            ))
        }
        fn decision_guard<'a>(
            &'a self,
            _: &ServiceWriteTransactionProof,
            connection: &ServiceConnectionAuthority,
        ) -> Result<Box<dyn ServiceDecisionGuard + 'a>, ApiError> {
            assert_eq!(connection.author(), &self.0);
            Ok(Box::new(Guard(self.0.clone())))
        }
        fn revoke_exact(&self, _: &ServiceConnectionAuthority) -> bool {
            true
        }
    }

    #[derive(Default)]
    struct Sink(std::sync::Mutex<Vec<WatchFrame>>);
    impl ModChannelSink for Sink {
        fn push(&self, frame: WatchFrame) -> bool {
            self.0.lock().unwrap().push(frame);
            true
        }
    }
    let mut f = fixture(0);
    let recipient = check_in(
        &f,
        fixture_claim(&f, "c", "pc", Harness::Claude, "warn-channel-recipient"),
        "warn-channel-ci",
    );
    let reads: Arc<dyn ModStoreReads> = f.store.clone();
    let registry = Arc::new(ModChannelRegistry::new(
        f.clock.clone(),
        reads,
        "i".into(),
        Arc::new(|| {}),
        ModDeliverySetting::On,
    ));
    f.service = DomainService::new("i".into(), f.store.clone(), f.clock.clone())
        .with_cooperative_owner(501, Arc::new(FairWriter::new(32)))
        .with_delivery_pull(f.state.clone())
        .with_mod_channels(registry.clone());
    let sink = Arc::new(Sink::default());
    let (channel, _) = f
        .service
        .watch_register(
            &WatchRequest {
                presentation_v2: true,
                claim: recipient.clone(),
            },
            sink.clone(),
            &budget(),
        )
        .unwrap();
    let lease = registry.presentation_lease(channel).unwrap();
    registry.pass(f.clock.utc_now()).unwrap();
    sink.0.lock().unwrap().clear();
    let native = || {
        f.db.query_row(
            "SELECT COALESCE((SELECT attention_version FROM wake_work WHERE seat_id='c'),0)",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
    };
    let tx = f.db.unchecked_transaction().unwrap();
    let author =
        crate::store::service_substrate::ensure_reserved_author(&tx, "i", UtcMillis(0)).unwrap();
    tx.commit().unwrap();
    let gate = Gate(author.clone());
    let connection = ServiceConnectionAuthority::new("i".into(), "boot".into(), 1, author);
    for n in 0..2 {
        sink.0.lock().unwrap().clear();
        let before = native();
        let text = format!("distinct channel warning {n}");
        let ServiceResult::Notification(notification) = f
            .store
            .service_operation(
                ServiceOperation::Notify(ServiceNotify {
                    thread: ThreadId::new("t"),
                    severity: NotificationSeverity::Warn,
                    event_json: serde_json::json!({"text":text}),
                    operation: OperationId::new(format!("warn-channel-notify-{n}")),
                }),
                &connection,
                &gate,
                &budget(),
                None,
            )
            .unwrap()
        else {
            panic!("Warn")
        };
        assert_eq!(
            native(),
            before,
            "publication precedes bounded native projection"
        );
        let job:String=f.db.query_row("SELECT id FROM work_jobs WHERE kind='send_attention' AND subject_id=(SELECT 'service-notify:'||preparation_id FROM service_notification_publications WHERE message_id=?1)",[notification.summary.message.as_str()],|r|r.get(0)).unwrap();
        finish_presentation_attention_job(&f, &job);
        assert!(
            native() > before,
            "each distinct Warn projects a native change even while generic relevance remains Open"
        );
        registry.mark_dirty();
        registry.pass(f.clock.utc_now()).unwrap();
        assert!(
            sink.0
                .lock()
                .unwrap()
                .iter()
                .any(|frame| matches!(frame, WatchFrame::Attention { .. })),
            "healthy bounded materialization causes channel drain"
        );
        let CommandResult::DeliveryPulledV2(DeliveryPullReplyV2::Page(page)) = f
            .service
            .handle(
                Command::DeliveryPullV2(DeliveryPullV2 {
                    claim: recipient.clone(),
                    consumer: PresentationConsumer::Watch,
                    render: PresentationRenderRequest::WatchJsonLinesV1,
                    presentation_lease: Some(lease.clone()),
                    marker_prefix: vec![],
                }),
                peer(),
                &budget(),
            )
            .unwrap()
        else {
            panic!("drained page")
        };
        assert!(page.items.iter().any(|item|matches!(item,crate::protocol::delivery::PresentationItemV2::Notice{warning,..} if warning==&notification.summary.message)),"each published Warn delivers its distinct Notice");
        if n == 0 {
            assert!(
                page.emitted_controls
                    .iter()
                    .any(|v| matches!(v, PresentationControlStampV2::Attention { .. }))
            );
        }
        if let Some(token) = page.presentation_token {
            f.service
                .handle(
                    Command::DeliveryPresentationConfirmV2(
                        crate::protocol::delivery::DeliveryPresentationConfirmV2 {
                            claim: recipient.clone(),
                            consumer: PresentationConsumer::Watch,
                            token,
                            emitted_controls: page.emitted_controls,
                            presentation_lease: Some(lease.clone()),
                        },
                    ),
                    peer(),
                    &budget(),
                )
                .unwrap();
        }
    }
    registry.unregister(channel, f.clock.utc_now());
}

// Kills: storing native_session as execution when a real V2 page carries a
// notice, overwriting a valid shared frontier and reoffering settled notices.
#[test]
fn presentation_hook_notice_settles_current_execution_without_reoffer() {
    presentation_notice_settles_current_execution(false, false);
}

#[test]
fn presentation_watch_notice_settles_current_execution_without_reoffer() {
    presentation_notice_settles_current_execution(true, false);
}

#[test]
fn presentation_watch_notice_continuation_preserves_remaining_confirming_condition() {
    presentation_notice_settles_current_execution(true, true);
}

fn presentation_notice_settles_current_execution(watch: bool, remaining_condition: bool) {
    use crate::{
        ports::{
            ServiceAuthorityGate, ServiceConnectionAuthority, ServiceDecisionGuard,
            ServiceWriteTransactionProof,
        },
        protocol::{
            delivery::{DeliveryPullReplyV2, PresentationItemV2},
            results::ApiError,
            service::{NotificationSeverity, ServiceNotify, ServiceOperation, ServiceResult},
        },
    };
    struct Gate(ServiceAuthorId);
    struct Guard(ServiceAuthorId);
    impl ServiceDecisionGuard for Guard {
        fn author(&self) -> &ServiceAuthorId {
            &self.0
        }
    }
    impl ServiceAuthorityGate for Gate {
        fn register(
            &self,
            instance: &str,
            boot: &str,
            author: ServiceAuthorId,
        ) -> Result<ServiceConnectionAuthority, ApiError> {
            Ok(ServiceConnectionAuthority::new(
                instance.into(),
                boot.into(),
                1,
                author,
            ))
        }
        fn decision_guard<'a>(
            &'a self,
            _: &ServiceWriteTransactionProof,
            connection: &ServiceConnectionAuthority,
        ) -> Result<Box<dyn ServiceDecisionGuard + 'a>, ApiError> {
            assert_eq!(connection.author(), &self.0);
            Ok(Box::new(Guard(self.0.clone())))
        }
        fn revoke_exact(&self, _: &ServiceConnectionAuthority) -> bool {
            true
        }
    }
    let window = OfferWindowFixture::new(watch);
    let f = &window.f;
    let recipient = &window.recipient;
    assert_ne!(
        recipient.execution.as_str(),
        recipient.native_session.as_str()
    );
    let tx = f.db.unchecked_transaction().unwrap();
    let author =
        crate::store::service_substrate::ensure_reserved_author(&tx, "i", UtcMillis(0)).unwrap();
    tx.commit().unwrap();
    let gate = Gate(author.clone());
    let connection = ServiceConnectionAuthority::new("i".into(), "boot".into(), 1, author);
    let notify = |operation: &str| {
        let ServiceResult::Notification(notification) = f
            .store
            .service_operation(
                ServiceOperation::Notify(ServiceNotify {
                    thread: ThreadId::new("t"),
                    severity: NotificationSeverity::Warn,
                    event_json: serde_json::json!({"detail": operation}),
                    operation: OperationId::new(operation),
                }),
                &connection,
                &gate,
                &budget(),
                None,
            )
            .unwrap()
        else {
            panic!("real Notify result")
        };
        let warning = notification.summary.message;
        let job: String = f.db.query_row(
            "SELECT id FROM work_jobs WHERE kind='send_attention' AND subject_id=(SELECT 'service-notify:'||preparation_id FROM service_notification_publications WHERE message_id=?1)",
            [warning.as_str()], |row| row.get(0),
        ).unwrap();
        finish_presentation_attention_job(f, &job);
        warning
    };
    // A real Current CheckIn establishes an earlier valid shared frontier.
    // The second notice must be settled by V2 itself, without a repair hook.
    let first = notify("frontier-first-notify");
    let initial = current_check_in(f, recipient.clone(), "frontier-initial-offer");
    assert!(
        initial
            .notices
            .items
            .iter()
            .any(|notice| notice.warning == first)
    );
    let frontier = || {
        f.db.query_row(
        "SELECT b.generation,b.execution_id,o.offered_ordinal FROM digest_notice_offer o JOIN occupant_bindings b ON b.binding_id=o.binding_id WHERE o.binding_id=?1 AND o.seat_id=b.seat_id",
        [binding_for_claim(f, recipient).as_str()], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?)),
    ).unwrap()
    };
    let before = frontier();
    assert_eq!(before.0 as u64, recipient.binding_generation);
    assert_eq!(before.1, recipient.execution.as_str());
    let remaining_thread = if remaining_condition {
        use crate::protocol::commands::{CreateThread, InvitationConfirmation, Invite};
        let CommandResult::ThreadCreated(thread) = f
            .service
            .handle(
                Command::CreateThread(CreateThread {
                    name: None,
                    topic: "remaining condition".into(),
                    goal: "review".into(),
                    operation: OperationId::new("notice-condition-thread"),
                    claim: window.sender.clone(),
                }),
                peer(),
                &budget(),
            )
            .unwrap()
        else {
            panic!("real thread")
        };
        f.service
            .handle(
                Command::Invite(Invite {
                    confirmation: Some(InvitationConfirmation::RequireAck),
                    thread: thread.clone(),
                    seat: recipient.seat.clone(),
                    deadline_millis: Some(60_000),
                    operation: OperationId::new("notice-condition-invite"),
                    claim: window.sender.clone(),
                }),
                peer(),
                &budget(),
            )
            .unwrap();
        Some(thread)
    } else {
        None
    };
    let remaining_witness = || {
        remaining_thread.as_ref().map(|thread| {
        f.db.query_row("SELECT id,state,confirmation,deadline_at FROM invitations WHERE thread_id=?1 AND seat_id=?2",
            params![thread.as_str(),recipient.seat.as_str()], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<i64>>(3)?))).unwrap()
    })
    };
    let pending_witness = remaining_witness();
    if let Some((id, state, confirmation, deadline)) = &pending_witness {
        assert!(!id.is_empty());
        assert_eq!(state, "pending");
        assert_eq!(confirmation, "require_ack");
        assert!(deadline.is_some());
    }
    let second = notify("frontier-second-notify");
    let initial_reply = if watch {
        window.raw_watch_pull()
    } else {
        presentation_test_pull(f, recipient)
    };
    let DeliveryPullReplyV2::Page(page) = &initial_reply else {
        panic!("actual V2 notice page")
    };
    assert_eq!(
        page.items
            .iter()
            .filter(|item| matches!(item,
        PresentationItemV2::Notice { warning, .. } if warning == &second))
            .count(),
        1
    );
    assert!(!page.items.iter().any(|item| matches!(item,
        PresentationItemV2::Notice { warning, .. } if warning == &first)));
    let after = frontier();
    assert_eq!(after.0 as u64, recipient.binding_generation);
    assert_eq!(
        after.1,
        recipient.execution.as_str(),
        "V2 preserves canonical execution, not native session"
    );
    let ordinal: i64 =
        f.db.query_row(
            "SELECT ordinal FROM digest_programmatic_warnings WHERE seat_id=?1 AND warning_id=?2",
            [recipient.seat.as_str(), second.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(after.2 >= ordinal && after.2 > before.2);
    assert_eq!(
        remaining_witness(),
        pending_witness,
        "notice settlement preserves the distinct pending invitation"
    );
    use crate::protocol::delivery::PresentationControlStampV2;
    let attention = page
        .emitted_controls
        .iter()
        .find_map(|stamp| match stamp {
            PresentationControlStampV2::Attention { epoch, version } => {
                Some((epoch.clone(), *version))
            }
            _ => None,
        })
        .expect("notice's actual Attention episode");
    assert!(
        page.more,
        "settling the carried witness must advertise continuation"
    );
    if watch {
        let outbox = || {
            f.db.query_row("SELECT json_array(outbox_token,outbox_descriptors,outbox_selection,outbox_render_key,outbox_codec_key,outbox_prefix,outbox_retired_mask,outbox_confirmed_mask) FROM invitation_presentation_consumers WHERE seat_id=?1 AND consumer='watch'", [recipient.seat.as_str()], |r|r.get::<_,String>(0)).unwrap()
        };
        let retained: serde_json::Value = serde_json::from_str(&outbox()).unwrap();
        let replay = window.raw_watch_pull();
        let DeliveryPullReplyV2::Page(replayed) = &replay else {
            panic!("lost-output notice replay")
        };
        assert_eq!(replayed.presentation_token, page.presentation_token);
        let notices = |items: &[PresentationItemV2]| {
            items
                .iter()
                .filter(|item| matches!(item, PresentationItemV2::Notice { .. }))
                .cloned()
                .collect::<Vec<_>>()
        };
        assert_eq!(
            notices(&replayed.items),
            notices(&page.items),
            "lost output replays the identical Notice"
        );
        if remaining_condition {
            assert_eq!(replayed.emitted_controls, page.emitted_controls);
        } else {
            assert!(
                replayed
                    .emitted_controls
                    .iter()
                    .all(|control| page.emitted_controls.contains(control)),
                "replay cannot invent a control stamp"
            );
        }
        let replay_state: serde_json::Value = serde_json::from_str(&outbox()).unwrap();
        for index in 0..6 {
            assert_eq!(
                replay_state[index], retained[index],
                "immutable outbox field {index}"
            );
        }
        assert_eq!(replay_state[7], retained[7], "replay is not confirmation");
        let descriptors: Vec<serde_json::Value> =
            serde_json::from_str(retained[1].as_str().unwrap()).unwrap();
        let allowed_retirement = descriptors
            .iter()
            .enumerate()
            .filter(|(_, v)| v["item"]["kind"] == "attention")
            .fold(0u64, |bits, (slot, _)| bits | (1 << slot));
        let before_mask = retained[6].as_u64().unwrap();
        let after_mask = replay_state[6].as_u64().unwrap();
        assert_eq!(
            before_mask & after_mask,
            before_mask,
            "retirement is monotone"
        );
        assert_eq!(
            after_mask & !allowed_retirement,
            before_mask & !allowed_retirement,
            "only canonically obsolete Attention may retire; Notice slots remain"
        );
        if remaining_condition {
            assert_eq!(
                after_mask, before_mask,
                "independent pending witness keeps Attention applicable"
            );
        }
        assert!(
            replayed.more,
            "durable notice continuation survives lost output"
        );
        assert_eq!(
            window
                .f
                .service
                .presentation
                .last_work
                .lock()
                .unwrap()
                .unwrap()
                .ordinary,
            0
        );
        window.confirm_nonmessage_presentation(&replay);
    } else {
        window.confirm_nonmessage_presentation(&initial_reply);
    }
    let mut more = page.more;
    let mut cleared = false;
    for _ in 0..32 {
        if !more {
            break;
        }
        let reply = window.pull();
        let work = window
            .f
            .service
            .presentation
            .last_work
            .lock()
            .unwrap()
            .unwrap();
        assert_eq!(work.relevance_calls, 1);
        assert!(work.output_passes <= 352 && work.output_bytes <= 65536);
        match reply {
            DeliveryPullReplyV2::Page(page) => {
                assert!(!page.items.iter().any(|item| matches!(item,
                    PresentationItemV2::Notice { warning, .. } if warning == &first || warning == &second)));
                for stamp in &page.emitted_controls {
                    if let PresentationControlStampV2::Cleared { epoch, version } = stamp {
                        assert_eq!((epoch, version), (&attention.0, &attention.1));
                        cleared = true;
                    }
                }
                more = page.more;
            }
            DeliveryPullReplyV2::Unchanged { .. } => {
                more = false;
            }
            other => panic!("admitted continuation refused: {other:?}"),
        }
    }
    assert!(!more, "advertised continuation terminates");
    assert_eq!(
        cleared, !remaining_condition,
        "only canonical absence clears the actual episode"
    );
    assert!(
        window.offers().is_empty(),
        "no receipt ACK used to drive continuation"
    );
    assert_eq!(
        remaining_witness(),
        pending_witness,
        "exact distinct confirming invitation stays pending throughout notice continuation"
    );
    assert_eq!(
        frontier(),
        after,
        "subsequent V2 pulls preserve settled frontier"
    );
    let next = current_check_in(f, recipient.clone(), "frontier-after-v2");
    assert!(
        next.notices.items.is_empty(),
        "a later CheckIn does not carry V2-settled notices"
    );
}

// These fixtures issue through the real V2 service and settle through the real
// canonical ACK writers. No offers or terminal receipt states are preseeded.
struct OfferWindowFixture {
    f: Fixture,
    recipient: CallerClaim,
    sender: CallerClaim,
    watch: Option<(
        Arc<crate::service::mod_channels::ModChannelRegistry>,
        crate::ports::ModChannelId,
        String,
    )>,
}

impl OfferWindowFixture {
    fn new(watch: bool) -> Self {
        use crate::{
            ports::{ModChannelSink, ModChannels, ModStoreReads},
            protocol::watch::{ModDeliverySetting, WatchFrame, WatchRequest},
            service::mod_channels::ModChannelRegistry,
        };
        struct Sink;
        impl ModChannelSink for Sink {
            fn push(&self, _: WatchFrame) -> bool {
                true
            }
        }
        let mut f = fixture(0);
        let recipient = if watch {
            check_in(
                &f,
                claim("c", "pc", Harness::Claude, "offer-window-recipient", 1),
                "offer-window-recipient-ci",
            )
        } else {
            pull_seat(&f)
        };
        let sender = check_in(
            &f,
            claim("x", "px", fourth(), "offer-window-sender", 1),
            "offer-window-sender-ci",
        );
        let watch = watch.then(|| {
            let reads: Arc<dyn ModStoreReads> = f.store.clone();
            let registry = Arc::new(ModChannelRegistry::new(
                f.clock.clone(),
                reads,
                "i".into(),
                Arc::new(|| {}),
                ModDeliverySetting::On,
            ));
            f.service = DomainService::new("i".into(), f.store.clone(), f.clock.clone())
                .with_cooperative_owner(501, Arc::new(FairWriter::new(32)))
                .with_delivery_pull(f.state.clone())
                .with_mod_channels(registry.clone());
            let (channel, _) = f
                .service
                .watch_register(
                    &WatchRequest {
                        presentation_v2: true,
                        claim: recipient.clone(),
                    },
                    Arc::new(Sink),
                    &budget(),
                )
                .unwrap();
            let lease = registry.presentation_lease(channel).unwrap();
            (registry, channel, lease)
        });
        Self {
            f,
            recipient,
            sender,
            watch,
        }
    }

    fn send(
        &self,
        n: usize,
        mode: crate::protocol::commands::DeliveryMode,
        body: &str,
    ) -> MessageId {
        use crate::protocol::commands::SendMessage;
        let CommandResult::MessageSent(id) = self
            .f
            .service
            .handle(
                Command::SendMessage(SendMessage {
                    delivery_mode: mode,
                    user_intent: None,
                    thread: ThreadId::new("t"),
                    body: body.into(),
                    // This fixture's existing thread has joined memberships;
                    // it is not an automatic direct-pair channel.
                    direct_recipient: None,
                    invited_recipients: vec![],
                    deadline_millis: None,
                    operation: OperationId::new(format!("offer-window-send-{n}")),
                    claim: self.sender.clone(),
                    relays_user: false,
                }),
                peer(),
                &budget(),
            )
            .unwrap()
        else {
            panic!("real send result")
        };
        id
    }

    fn pull(&self) -> crate::protocol::delivery::DeliveryPullReplyV2 {
        use crate::protocol::delivery::{
            DeliveryPullV2, PresentationConsumer, PresentationRenderRequest,
        };
        let Some((_, _, lease)) = &self.watch else {
            let reply = presentation_test_pull(&self.f, &self.recipient);
            self.confirm_nonmessage_presentation(&reply);
            return reply;
        };
        let CommandResult::DeliveryPulledV2(reply) = self
            .f
            .service
            .handle(
                Command::DeliveryPullV2(DeliveryPullV2 {
                    claim: self.recipient.clone(),
                    consumer: PresentationConsumer::Watch,
                    render: PresentationRenderRequest::WatchJsonLinesV1,
                    presentation_lease: Some(lease.clone()),
                    marker_prefix: vec![],
                }),
                peer(),
                &budget(),
            )
            .unwrap()
        else {
            panic!("real V2 pull result")
        };
        self.confirm_nonmessage_presentation(&reply);
        reply
    }

    fn replace_watch(&mut self) -> String {
        use crate::{
            ports::{ModChannelSink, ModChannels},
            protocol::watch::{WatchFrame, WatchRequest},
        };
        struct Sink;
        impl ModChannelSink for Sink {
            fn push(&self, _: WatchFrame) -> bool {
                true
            }
        }
        let (registry, _, old_lease) = self.watch.as_ref().unwrap();
        let old_lease = old_lease.clone();
        let (channel, _) = self
            .f
            .service
            .watch_register(
                &WatchRequest {
                    presentation_v2: true,
                    claim: self.recipient.clone(),
                },
                Arc::new(Sink),
                &budget(),
            )
            .unwrap();
        let lease = registry.presentation_lease(channel).unwrap();
        self.watch = Some((registry.clone(), channel, lease));
        old_lease
    }

    fn watch_request(&self) -> crate::protocol::delivery::DeliveryPullV2 {
        use crate::protocol::delivery::{
            DeliveryPullV2, PresentationConsumer, PresentationRenderRequest,
        };
        DeliveryPullV2 {
            claim: self.recipient.clone(),
            consumer: PresentationConsumer::Watch,
            render: PresentationRenderRequest::WatchJsonLinesV1,
            presentation_lease: Some(self.watch.as_ref().unwrap().2.clone()),
            marker_prefix: vec![],
        }
    }

    fn raw_watch_pull(&self) -> crate::protocol::delivery::DeliveryPullReplyV2 {
        let CommandResult::DeliveryPulledV2(reply) = self
            .f
            .service
            .handle(
                Command::DeliveryPullV2(self.watch_request()),
                peer(),
                &budget(),
            )
            .unwrap()
        else {
            panic!("actual Watch response")
        };
        reply
    }

    fn confirm_nonmessage_presentation(
        &self,
        reply: &crate::protocol::delivery::DeliveryPullReplyV2,
    ) {
        use crate::protocol::delivery::{
            DeliveryPresentationConfirmV2, DeliveryPullReplyV2, PresentationConsumer,
        };
        if let DeliveryPullReplyV2::Page(page) = reply
            && let Some(token) = &page.presentation_token
        {
            self.f
                .service
                .handle(
                    Command::DeliveryPresentationConfirmV2(DeliveryPresentationConfirmV2 {
                        claim: self.recipient.clone(),
                        consumer: if self.watch.is_some() {
                            PresentationConsumer::Watch
                        } else {
                            PresentationConsumer::Hook
                        },
                        token: token.clone(),
                        presentation_lease: self.watch.as_ref().map(|(_, _, lease)| lease.clone()),
                        emitted_controls: page.emitted_controls.clone(),
                    }),
                    peer(),
                    &budget(),
                )
                .unwrap();
        }
    }

    fn offers(&self) -> std::collections::BTreeMap<MessageId, crate::delivery::ledger::Offer> {
        self.f
            .state
            .lock()
            .get(&binding_for_claim(&self.f, &self.recipient))
            .unwrap()
            .offers
            .clone()
    }

    fn drain(&self, wanted: usize) -> std::collections::BTreeSet<MessageId> {
        use crate::protocol::delivery::DeliveryPullReplyV2;
        let mut shown = std::collections::BTreeSet::new();
        for _ in 0..256 {
            match self.pull() {
                DeliveryPullReplyV2::Page(page) => {
                    for id in page.offered.iter().chain(&page.truncated) {
                        assert!(shown.insert(id.clone()), "early duplicate offer {id:?}");
                    }
                }
                DeliveryPullReplyV2::Unchanged { .. } => {}
                other => panic!("admitted V2 drain refused: {other:?}"),
            }
            let work = self
                .f
                .service
                .presentation
                .last_work
                .lock()
                .unwrap()
                .unwrap();
            assert!(
                work.initial + work.context + work.revisit + work.ordinary + work.notices <= 433,
                "fresh discovery bound stays independent of ledger reconciliation: {work:?}"
            );
            if shown.len() == wanted {
                return shown;
            }
        }
        panic!(
            "bounded V2 drain showed {} of {wanted} messages",
            shown.len()
        );
    }

    fn ack(&self, messages: &[MessageId], mod_ack: bool, lazy: bool, operation: &str) {
        use crate::protocol::{
            commands::{Ack, AckModDelivered},
            watch::{ModAckOutcome, ModDeliveryVia},
        };
        for (n, batch) in messages.chunks(100).enumerate() {
            let operation = OperationId::new(format!("{operation}-{n}"));
            let command = if mod_ack {
                Command::AckModDelivered(AckModDelivered {
                    via: if lazy {
                        ModDeliveryVia::Append
                    } else {
                        ModDeliveryVia::Context
                    },
                    messages: batch.to_vec(),
                    operation,
                    claim: self.recipient.clone(),
                })
            } else {
                Command::Ack(Ack {
                    messages: batch.to_vec(),
                    operation,
                    claim: self.recipient.clone(),
                })
            };
            let result = self.f.service.handle(command, peer(), &budget()).unwrap();
            if mod_ack {
                let CommandResult::ModDeliveryAcked(report) = result else {
                    panic!("actual Watch ACK result")
                };
                assert_eq!(report.results.len(), batch.len());
                assert!(
                    report
                        .results
                        .iter()
                        .all(|v| v.result == ModAckOutcome::Settled),
                    "real canonical delivery settlement: {report:?}"
                );
            }
        }
    }
}

impl Drop for OfferWindowFixture {
    fn drop(&mut self) {
        use crate::ports::ModChannels;
        if let Some((registry, channel, _)) = &self.watch {
            registry.unregister(*channel, self.f.clock.utc_now());
        }
    }
}

// Read the public body continuation completely before claiming display of an
// identity whose V2 frame was truncated. No ledger or cursor SQL is written.
fn reclamation_read_full_body(f: &Fixture, message: &MessageId) -> String {
    reclamation_read_full_body_with_page(f, message, 4096)
}

fn reclamation_read_full_body_with_page(
    f: &Fixture,
    message: &MessageId,
    max_bytes: u32,
) -> String {
    use crate::protocol::{
        commands::{BodyReadRequest, MessageQuery},
        results::MessageContent,
    };
    // max_bytes bounds the encoded response, including summary and two
    // continuation representations, rather than 4096 raw body bytes. The
    // largest fixture is 58,000 bytes; require at least 1024 bytes of progress
    // per partial reply, giving a finite ceiling of 57 reads including the tail.
    const MAX_FIXTURE_BYTES: usize = 58_000;
    const MIN_PARTIAL_BYTES: usize = 1024;
    let mut cursor = None;
    let mut complete = String::new();
    let mut expected_total = None;
    for _ in 0..MAX_FIXTURE_BYTES.div_ceil(MIN_PARTIAL_BYTES) {
        let previous_cursor = cursor.clone();
        let CommandResult::Message(read) = f
            .service
            .handle(
                Command::Message(MessageQuery {
                    message: message.clone(),
                    body: BodyReadRequest {
                        cursor,
                        offset: None,
                        max_bytes,
                    },
                }),
                peer(),
                &budget(),
            )
            .unwrap()
        else {
            panic!("public body continuation")
        };
        let MessageContent::Ordinary {
            body_data,
            body_offset,
            body_total_bytes,
            body_complete,
            body_next_cursor,
            ..
        } = read.content
        else {
            panic!("ordinary body")
        };
        assert!(body_total_bytes <= MAX_FIXTURE_BYTES as u64);
        assert_eq!(
            *expected_total.get_or_insert(body_total_bytes),
            body_total_bytes
        );
        assert_eq!(body_offset as usize, complete.len());
        if !body_complete {
            assert!(
                body_data.len() >= MIN_PARTIAL_BYTES,
                "partial reply must make bounded byte progress"
            );
        }
        complete.push_str(&body_data);
        if body_complete {
            assert!(body_next_cursor.is_none());
            assert_eq!(complete.len() as u64, body_total_bytes);
            return complete;
        }
        let next = body_next_cursor.expect("bounded continuation makes progress");
        assert_ne!(Some(&next), previous_cursor.as_ref());
        let position = crate::protocol::pagination::Cursor::decode(&next).unwrap();
        assert_eq!(position.after_ordinal, complete.len() as u64);
        assert_eq!(position.high_water_ordinal, body_total_bytes);
        assert!(position.after_ordinal > body_offset);
        assert!(position.after_ordinal < body_total_bytes);
        cursor = Some(next);
    }
    panic!("fixture full body exceeds bounded continuation reads")
}

fn reclamation_cursor(f: &Fixture, claim: &CallerClaim) -> i64 {
    f.db.query_row(
        "SELECT seq FROM binding_cursors WHERE binding_id=?1 AND thread_id='t'",
        [binding_for_claim(f, claim).as_str()],
        |row| row.get(0),
    )
    .unwrap()
}

fn reclamation_advance_displayed(
    f: &Fixture,
    claim: &CallerClaim,
    displayed: &[MessageId],
    operation: &str,
) {
    for (n, batch) in displayed.chunks(100).enumerate() {
        let CommandResult::CursorsAdvanced(advanced) = f
            .service
            .handle(
                Command::AdvanceCursors(crate::protocol::commands::AdvanceCursors {
                    claim: claim.clone(),
                    operation: OperationId::new(format!("{operation}-{n}")),
                    displayed: batch.to_vec(),
                }),
                peer(),
                &budget(),
            )
            .unwrap()
        else {
            panic!("public binding display progress")
        };
        assert_eq!(advanced, batch);
    }
}

fn assert_v2_offer_window_reclaimed(watch: bool, lazy: bool) {
    use crate::{
        delivery::ledger::MAX_OFFERS_PER_BINDING,
        protocol::{commands::DeliveryMode, delivery::DeliveryPullReplyV2},
    };
    let window = OfferWindowFixture::new(watch);
    let mode = if lazy {
        DeliveryMode::Lazy
    } else {
        DeliveryMode::Ordinary
    };
    let sent = (0..MAX_OFFERS_PER_BINDING)
        .map(|n| window.send(n, mode, "complete canonical offer window"))
        .collect::<Vec<_>>();
    assert_eq!(window.drain(256), sent.iter().cloned().collect());
    assert_eq!(
        window.offers().len(),
        256,
        "real V2 issuance filled live capacity"
    );
    let before_ack = window.offers();
    let binding = binding_for_claim(&window.f, &window.recipient);
    let cursor = || {
        window
            .f
            .db
            .query_row::<i64, _, _>(
                "SELECT seq FROM binding_cursors WHERE binding_id=?1 AND thread_id='t'",
                [binding.as_str()],
                |row| row.get(0),
            )
            .unwrap()
    };
    let before_cursor = cursor();
    window.ack(&sent, watch, lazy, "offer-window-settle");
    if !watch {
        assert_eq!(
            cursor(),
            before_cursor,
            "manual seat ACK does not claim body display"
        );
        window.pull();
        assert_eq!(
            window.offers(),
            before_ack,
            "seat settlement alone preserves this binding's unread offer stamps"
        );
        assert!(
            before_ack.values().all(|stamp| !stamp.truncated),
            "all 256 V2 bodies were actually complete"
        );
        reclamation_advance_displayed(
            &window.f,
            &window.recipient,
            &sent,
            "offer-window-full-display",
        );
        assert!(cursor() > before_cursor);
    }
    for id in &sent {
        let state: String = window
            .f
            .db
            .query_row(
                if lazy {
                    "SELECT state FROM lazy_recipients WHERE message_id=?1 AND seat_id=?2"
                } else {
                    "SELECT state FROM receipt_state WHERE message_id=?1 AND seat_id=?2"
                },
                params![id.as_str(), window.recipient.seat.as_str()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(state, if lazy { "displayed" } else { "acked" });
    }
    let successor = window.send(256, mode, "new identity after cumulative capacity");
    let mut emitted = false;
    for n in 0..16 {
        if let DeliveryPullReplyV2::Page(page) = window.pull() {
            assert!(page.truncated.is_empty());
            assert!(page.offered.iter().all(|id| id == &successor));
            emitted |= page.offered.contains(&successor);
        }
        if n == 0 {
            let work = window
                .f
                .service
                .presentation
                .last_work
                .lock()
                .unwrap()
                .unwrap();
            assert_eq!(work.offer_reconciliation_visits, 256);
            assert_eq!(
                work.offer_reconciliation_probes,
                if lazy { 2560 } else { 2304 }
            );
        }
        if emitted {
            break;
        }
    }
    assert!(
        emitted,
        "canonical terminal offers must free capacity in the same V2 binding"
    );
    assert_eq!(
        window.offers().keys().cloned().collect::<Vec<_>>(),
        vec![successor]
    );
    let ledger = window.f.state.lock();
    let entry = ledger
        .get(&binding_for_claim(&window.f, &window.recipient))
        .unwrap();
    assert_eq!(entry.generation, window.recipient.binding_generation);
    assert_eq!(
        entry.binding_id,
        binding_for_claim(&window.f, &window.recipient)
    );
}

// Kills: treating MAX_OFFERS_PER_BINDING as cumulative lifetime throughput when
// Watch settles actual receipt-state rows through AckModDelivered.
#[test]
fn presentation_watch_settled_receipts_reclaim_live_offer_capacity() {
    assert_v2_offer_window_reclaimed(true, false);
}

// Kills: reconciling only the mod ACK route, leaving actual manual ACKs stuck
// in a capable Hook consumer's live-offer table.
#[test]
fn presentation_hook_manual_ack_reclaims_live_offer_capacity() {
    assert_v2_offer_window_reclaimed(false, false);
}

// Kills: checking only ordinary receipts and retaining displayed published
// lazy rows forever even after the canonical Watch completion writer runs.
#[test]
fn presentation_watch_completed_lazy_rows_reclaim_live_offer_capacity() {
    assert_v2_offer_window_reclaimed(true, true);
}

// Kills: cap eviction, reclaiming truncated/pending offers as presentation
// success, and resetting an unsettled id's count/timestamps during reclamation.
#[test]
fn presentation_reclamation_preserves_pending_truncated_and_reoffer_history() {
    use crate::protocol::{
        commands::{AckModDelivered, DeliveryMode},
        delivery::{
            DeliveryPullReplyV2, DeliveryPullV2, PresentationConsumer, PresentationRenderRequest,
        },
        watch::{ModAckOutcome, ModAckReason, ModDeliveryVia},
    };
    let window = OfferWindowFixture::new(true);
    let big = "x".repeat(WATCH_BODY_LIMIT_BYTES + 1);
    let mut sent = (0..256)
        .map(|n| {
            window.send(
                n,
                DeliveryMode::Ordinary,
                if n == 0 { &big } else { "pending" },
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(window.drain(256), sent[..256].iter().cloned().collect());
    let first = window.offers();
    assert!(first[&sent[0]].truncated);
    for _ in 0..4 {
        window.pull();
    }
    assert_eq!(
        window.offers(),
        first,
        "pending offers neither evict nor reoffer early"
    );
    let CommandResult::ModDeliveryAcked(report) = window
        .f
        .service
        .handle(
            Command::AckModDelivered(AckModDelivered {
                via: ModDeliveryVia::Context,
                messages: vec![sent[0].clone()],
                operation: OperationId::new("offer-window-truncated-refusal"),
                claim: window.recipient.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("truncated ACK result")
    };
    assert_eq!(report.results[0].result, ModAckOutcome::RefusedTerminal);
    assert_eq!(report.results[0].reason, Some(ModAckReason::Truncated));
    window.pull();
    assert_eq!(
        window.offers(),
        first,
        "refused delivery ACK is not settlement"
    );
    advance(&window.f, REOFFER_AFTER_MS);
    assert_eq!(window.drain(256), sent[..256].iter().cloned().collect());
    let second = window.offers();
    assert!(second.values().all(|v| v.count == 2));
    // Add the withheld fresh identity after observing the due reoffers. A
    // zero-slot fresh carry otherwise stops ordinary discovery
    // before it can revisit the older IDs in this same finite walk.
    sent.push(window.send(256, DeliveryMode::Ordinary, "waiting for a reclaimed slot"));
    for _ in 0..4 {
        if let DeliveryPullReplyV2::Page(page) = window.pull() {
            assert!(!page.offered.contains(&sent[256]));
        }
    }
    assert_eq!(
        window.offers(),
        second,
        "the fresh identity cannot evict pending offers at capacity"
    );
    // Manual ACK settles the seat; the public body continuation and explicit
    // display below advance this binding. Keep one short offer canonically pending.
    window.ack(&sent[..255], false, false, "offer-window-manual-settle");
    assert_eq!(
        window.offers(),
        second,
        "manual ACK has not fabricated display or changed offer history"
    );
    assert_eq!(reclamation_read_full_body(&window.f, &sent[0]), big);
    reclamation_advance_displayed(
        &window.f,
        &window.recipient,
        &sent[..255],
        "offer-window-continuation-displayed",
    );
    assert_eq!(
        window.offers(),
        second,
        "only the next admitted pull reconciles the display-complete keys"
    );
    let mut emitted = false;
    for _ in 0..16 {
        if let DeliveryPullReplyV2::Page(page) = window.pull() {
            emitted |= page.offered.contains(&sent[256]);
            assert!(
                !page.offered.contains(&sent[255]),
                "surviving pending offer is not due"
            );
        }
        if emitted {
            break;
        }
    }
    assert!(
        emitted,
        "manual ACK of a truncated offer must reclaim its slot"
    );
    let retained = window.offers();
    assert_eq!(retained.len(), 2);
    assert_eq!(
        retained[&sent[255]], second[&sent[255]],
        "reconciliation preserves live reoffer history"
    );
    let CommandResult::DeliveryPulledV2(DeliveryPullReplyV2::Refused {
        reason: Refusal::SessionMismatch,
    }) = window
        .f
        .service
        .handle(
            Command::DeliveryPullV2(DeliveryPullV2 {
                claim: window.recipient.clone(),
                consumer: PresentationConsumer::Watch,
                render: PresentationRenderRequest::WatchJsonLinesV1,
                presentation_lease: Some("ffffffffffffffffffffffffffffffff".into()),
                marker_prefix: vec![],
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("stale owner must refuse")
    };
    assert_eq!(
        window.offers(),
        retained,
        "refused owner cannot reconcile another owner's ledger"
    );
    advance(&window.f, REOFFER_AFTER_MS);
    assert_eq!(
        window.drain(2),
        [sent[255].clone(), sent[256].clone()].into_iter().collect()
    );
    assert_eq!(window.offers()[&sent[255]].count, MAX_OFFERS_PER_GENERATION);
    advance(&window.f, REOFFER_AFTER_MS);
    assert_eq!(window.drain(1), [sent[256].clone()].into_iter().collect());
    let parked = window.offers();
    assert!(
        parked
            .values()
            .all(|v| v.count == MAX_OFFERS_PER_GENERATION)
    );
    advance(&window.f, REOFFER_AFTER_MS);
    for _ in 0..4 {
        window.pull();
    }
    assert_eq!(
        window.offers(),
        parked,
        "pending offers remain parked at the original generation limit"
    );

    // A real canonical check-in creates a new session binding at the same mapping
    // generation and disconnects its same-process predecessor. The old owner
    // must refuse before it can drop that predecessor's retained offers.
    let next = check_in(
        &window.f,
        fresh_execution(
            claim(
                "c",
                "pc",
                Harness::Claude,
                "offer-window-next",
                window.recipient.binding_generation,
            ),
            2,
        ),
        "offer-window-next-ci",
    );
    assert_eq!(next.binding_generation, window.recipient.binding_generation);
    assert_ne!(
        binding_for_claim(&window.f, &next),
        binding_for_claim(&window.f, &window.recipient)
    );
    assert_eq!(
        window
            .f
            .db
            .query_row::<String, _, _>(
                "SELECT liveness FROM occupant_bindings WHERE binding_id=?1",
                [binding_for_claim(&window.f, &window.recipient).as_str()],
                |row| row.get(0)
            )
            .unwrap(),
        "disconnected"
    );
    assert!(matches!(window.pull(), DeliveryPullReplyV2::Refused { .. }));
    assert_eq!(
        window.offers(),
        parked,
        "disconnected predecessor cannot reconcile ledger state"
    );
}

fn full_pending_offer_window_with_fresh_carry() -> (OfferWindowFixture, Vec<MessageId>) {
    use crate::protocol::commands::DeliveryMode;
    let window = OfferWindowFixture::new(true);
    let big = "x".repeat(WATCH_BODY_LIMIT_BYTES + 1);
    let sent = (0..257)
        .map(|n| {
            window.send(
                n,
                DeliveryMode::Ordinary,
                if n == 0 { &big } else { "pending" },
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(window.drain(256), sent[..256].iter().cloned().collect());
    let offers = window.offers();
    assert_eq!(offers.len(), 256);
    assert!(offers.values().all(|offer| offer.count == 1));
    assert!(offers[&sent[0]].truncated);
    assert!(!offers.contains_key(&sent[256]));
    assert_eq!(window.f.db.query_row(
        "SELECT count(*) FROM prepared_recipients pr JOIN send_manifests sm ON sm.preparation_id=pr.preparation_id LEFT JOIN receipt_state rs ON rs.message_id=sm.message_id AND rs.seat_id=pr.seat_id WHERE pr.seat_id='c' AND COALESCE(rs.state,'pending')='pending' AND pr.ack_required=1",
        [], |r| r.get::<_,i64>(0),
    ).unwrap(), 257, "real sends remain pending; presentation does not ACK");
    (window, sent)
}

fn capacity_normal_state(window: &OfferWindowFixture) -> String {
    window.f.db.query_row("SELECT json_array(ordinary_version,ordinary_done,ordinary_cursor,ordinary_pending,ordinary_partial,ordinary_skip_body,ordinary_scan_passive_generation) FROM invitation_presentation_consumers WHERE seat_id=?1 AND consumer=?2",
        rusqlite::params![window.recipient.seat.as_str(), if window.watch.is_some() { "watch" } else { "hook" }], |r| r.get(0)).unwrap()
}

fn assert_capacity_watch_page_bound(page: &crate::protocol::delivery::DeliveryPageV2) {
    assert!(page.items.len() <= 32);
    let bytes: usize = crate::delivery::presentation::presentation_watch_lines(&page.items)
        .iter()
        .map(|line| serde_json::to_vec(line).unwrap().len() + 1)
        .sum();
    assert!(bytes <= 65536, "actual Watch JSON-lines bytes={bytes}");
}

fn assert_full_pending_offer_work(window: &OfferWindowFixture) {
    let work = window
        .f
        .service
        .presentation
        .last_work
        .lock()
        .unwrap()
        .unwrap();
    assert!(
        work.initial + work.context + work.revisit + work.ordinary + work.notices <= 433,
        "full pending carry must respect fresh discovery: {work:?}"
    );
    assert!(work.ordinary <= 100);
    assert!(work.ordinary_due_key_probes <= 1303);
    assert!(work.ordinary_due_seeks <= 100);
    assert!(work.ordinary_due_hydrated <= 32);
    assert!(work.output_passes <= 352);
    assert_eq!(work.offer_reconciliation_visits, 256);
    assert_eq!(work.offer_reconciliation_probes, 1024);
}

// Kills: an unoffered fresh carry at zero slots driving endless actionable
// page.more continuations despite every known offer being blocked until due.
#[test]
fn presentation_full_pending_carry_reaches_quiet_without_evicting_offers() {
    use crate::protocol::delivery::DeliveryPullReplyV2;
    let (window, sent) = full_pending_offer_window_with_fresh_carry();
    let original = window.offers();
    let mut quiet = false;
    for _ in 0..16 {
        match window.pull() {
            DeliveryPullReplyV2::Unchanged { .. } => quiet = true,
            DeliveryPullReplyV2::Page(page) => {
                assert!(
                    page.offered.is_empty() && page.truncated.is_empty(),
                    "no offer is due and the fresh identity must wait at capacity"
                );
                quiet = !page.more;
            }
            other => panic!("live full-capacity owner refused: {other:?}"),
        }
        assert_full_pending_offer_work(&window);
        assert_eq!(
            window.offers(),
            original,
            "quieting cannot evict or reset pending offers"
        );
        if quiet {
            break;
        }
    }
    assert!(
        quiet,
        "full pending carry must stop actionable continuation within bounded pulls"
    );
    assert!(!window.offers().contains_key(&sent[256]));
}

// Kills: quieting the full carry by permanently parking it, which also starves
// already known pending offers once their original 60-second reoffer is due.
#[test]
fn presentation_full_pending_carry_serves_due_known_offers_without_admitting_fresh_id() {
    use crate::protocol::delivery::DeliveryPullReplyV2;
    let (window, sent) = full_pending_offer_window_with_fresh_carry();
    let original = window.offers();
    // Let ordinary discovery encounter the retained fresh ID before the clock
    // advances, without requiring the separate quiet-continuation behavior.
    for _ in 0..4 {
        window.pull();
        assert_full_pending_offer_work(&window);
        assert_eq!(window.offers(), original);
    }
    let parked = capacity_normal_state(&window);
    advance(&window.f, REOFFER_AFTER_MS);
    let due_at = window.f.clock.utc_now();
    let mut shown = std::collections::BTreeSet::new();
    for _ in 0..16 {
        match window.pull() {
            DeliveryPullReplyV2::Page(page) => {
                assert_capacity_watch_page_bound(&page);
                for id in page.offered.iter().chain(&page.truncated) {
                    assert!(
                        original.contains_key(id),
                        "only due known IDs fit at capacity"
                    );
                    assert_ne!(id, &sent[256]);
                    assert!(
                        shown.insert(id.clone()),
                        "same-time duplicate reoffer {id:?}"
                    );
                }
            }
            DeliveryPullReplyV2::Unchanged { .. } => {}
            other => panic!("live due-offer owner refused: {other:?}"),
        }
        assert_full_pending_offer_work(&window);
        assert_eq!(capacity_normal_state(&window), parked);
        let retained = window.offers();
        assert_eq!(retained.len(), 256);
        for (id, before) in &original {
            let after = retained[id];
            assert_eq!(after.first_at, before.first_at);
            assert_eq!(after.truncated, before.truncated);
            assert_eq!(after.count, if shown.contains(id) { 2 } else { 1 });
            assert_eq!(
                after.last_at,
                if shown.contains(id) {
                    due_at
                } else {
                    before.last_at
                }
            );
        }
        if shown.len() == 256 {
            break;
        }
    }
    assert!(
        !shown.is_empty(),
        "due known offers must make progress past a retained fresh ID"
    );
    assert_eq!(
        shown,
        sent[..256].iter().cloned().collect(),
        "the finite due set must progress within bounded pulls"
    );
    assert!(!window.offers().contains_key(&sent[256]));
    let twice = window.offers();
    for _ in 0..4 {
        if let DeliveryPullReplyV2::Page(page) = window.pull() {
            assert_capacity_watch_page_bound(&page);
            assert!(
                page.offered.is_empty() && page.truncated.is_empty(),
                "due progress cannot reset the original reoffer spacing"
            );
        }
        assert_full_pending_offer_work(&window);
        assert_eq!(window.offers(), twice);
    }
    advance(&window.f, REOFFER_AFTER_MS);
    let mut third = std::collections::BTreeSet::new();
    for _ in 0..16 {
        if let DeliveryPullReplyV2::Page(page) = window.pull() {
            assert_capacity_watch_page_bound(&page);
            third.extend(page.offered.into_iter().chain(page.truncated));
        }
        assert_eq!(capacity_normal_state(&window), parked);
        if third.len() == 256 {
            break;
        }
    }
    assert_eq!(third, sent[..256].iter().cloned().collect());
    assert!(window.offers().values().all(|stamp| stamp.count == 3));
    advance(&window.f, REOFFER_AFTER_MS);
    assert!(matches!(
        window.pull(),
        DeliveryPullReplyV2::Unchanged { .. }
    ));
    window.ack(
        std::slice::from_ref(&sent[1]),
        false,
        false,
        "capacity-resume",
    );
    // Reading the short ACKed identity cannot jump over the earlier truncated
    // body. Read its public continuation, then report exactly this full prefix.
    assert_eq!(
        reclamation_read_full_body(&window.f, &sent[0]),
        "x".repeat(WATCH_BODY_LIMIT_BYTES + 1)
    );
    reclamation_advance_displayed(
        &window.f,
        &window.recipient,
        &sent[..2],
        "capacity-resume-full-prefix",
    );
    assert_eq!(window.f.db.query_row::<i64, _, _>(
        "SELECT count(*) FROM receipt_state WHERE message_id=?1 AND seat_id='c' AND state='acked'",
        [sent[0].as_str()], |row| row.get(0),
    ).unwrap(), 0, "reading the prior truncated body does not ACK it");
    let retained = window.offers();
    let DeliveryPullReplyV2::Page(resumed) = window.pull() else {
        panic!("same-call reclaimed slot");
    };
    assert!(resumed.offered.contains(&sent[256]));
    assert_eq!(
        window.offers()[&sent[0]],
        retained[&sent[0]],
        "read but still pending predecessor keeps its complete count-three stamp"
    );
}

fn offer_reconciliation_input(
    f: &Fixture,
    claim: &CallerClaim,
    known: Vec<MessageId>,
) -> crate::ports::PresentationInput {
    use crate::protocol::delivery::{
        DeliveryPullV2, PresentationConsumer, PresentationRenderRequest,
    };
    crate::ports::PresentationInput {
        binding_id: binding_for_claim(f, claim),
        owner_replay: None,
        ordinary_due_snapshot: None,
        frozen_render: presentation_watch_frozen(f, claim),
        render_profile: crate::delivery::presentation::PresentationRenderProfile::WatchJsonLinesV1,
        pull: DeliveryPullV2 {
            claim: claim.clone(),
            consumer: PresentationConsumer::Watch,
            render: PresentationRenderRequest::WatchJsonLinesV1,
            presentation_lease: Some("0123456789abcdef0123456789abcdef".into()),
            marker_prefix: vec![],
        },
        attention_version: 0,
        ordinary_version: 0,
        with_attention: false,
        retry_messages: false,
        blocked_messages: known.clone(),
        new_message_slots: 256usize.saturating_sub(known.len()),
        known_messages: known,
    }
}

// Kills: a full pending walk instead of exact bounded snapshot keys, skipping
// reconciliation on Unchanged, and omitting the legacy authority fallback.
#[test]
fn presentation_reclamation_legacy_terminal_delta_has_exact_bounded_work_on_unchanged() {
    use crate::protocol::delivery::DeliveryPullReplyV2;
    let f = fixture(257);
    let recipient = pull_seat(&f);
    f.db.execute("UPDATE receipts SET state='acked' WHERE seat_id='s'", [])
        .unwrap();
    // A new consumer must finish its finite ordinary scan before it can be
    // quiet. Prime that real walk without any ledger hints or reconciliation.
    let mut quiet = false;
    for _ in 0..16 {
        let primed = f
            .store
            .presentation_pull(
                offer_reconciliation_input(&f, &recipient, vec![]),
                &budget(),
            )
            .unwrap();
        assert_eq!(primed.work.offer_reconciliation_visits, 0);
        assert_eq!(primed.work.offer_reconciliation_probes, 0);
        assert!(primed.reclaimed_messages.is_empty());
        assert!(
            primed.work.initial
                + primed.work.context
                + primed.work.revisit
                + primed.work.ordinary
                + primed.work.notices
                <= 433
        );
        match primed.reply {
            DeliveryPullReplyV2::Unchanged { .. } => {
                quiet = true;
                break;
            }
            DeliveryPullReplyV2::Page(_) => {}
            other => panic!("finite consumer prime refused: {other:?}"),
        }
    }
    assert!(
        quiet,
        "finite empty consumer walk must reach quiet within bounded pulls"
    );
    let known = (1..=256)
        .map(|n| MessageId::new(format!("m{n}")))
        .collect::<Vec<_>>();
    let output = f
        .store
        .presentation_pull(
            offer_reconciliation_input(&f, &recipient, known.clone()),
            &budget(),
        )
        .unwrap();
    assert!(matches!(
        output.reply,
        DeliveryPullReplyV2::Unchanged { .. }
    ));
    assert_eq!(
        output
            .reclaimed_messages
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>(),
        known.into_iter().collect()
    );
    assert_eq!(output.work.offer_reconciliation_visits, 256);
    assert_eq!(output.work.offer_reconciliation_probes, 2560);
    assert_eq!(output.work.ordinary_publication_visits, 0);
}

// Kills: deleting pending/missing snapshot entries, probing duplicate hints
// repeatedly, and reading receipt authority for an absent message.
#[test]
fn presentation_reclamation_keeps_pending_missing_and_deduplicates_snapshot_work() {
    let f = fixture(2);
    let recipient = pull_seat(&f);
    f.db.execute(
        "UPDATE receipts SET state='recipient_retired' WHERE message_id='m2' AND seat_id='s'",
        [],
    )
    .unwrap();
    let output = f
        .store
        .presentation_pull(
            offer_reconciliation_input(&f, &recipient, ids(&["m1", "m2", "missing", "m2"])),
            &budget(),
        )
        .unwrap();
    assert_eq!(output.reclaimed_messages, ids(&["m2"]));
    assert_eq!(output.work.offer_reconciliation_visits, 3);
    assert_eq!(output.work.offer_reconciliation_probes, 16);
    assert_eq!(receipt_state(&f, "m1"), "pending");
}

// Kills: a contradictory legacy row overriding manifest authority, a missing
// sparse row being treated as terminal, or explicit materialized waivers being
// needlessly retained until the offer cap becomes a lifetime throughput cap.
#[test]
fn presentation_reclamation_preserves_logical_precedence_and_explicit_waiver_flags() {
    use crate::protocol::commands::DeliveryMode;
    for (state, waiver, legacy_required, want) in [
        (None, "", 1, false),
        (Some("pending"), "", 1, false),
        (Some("acked"), "", 1, true),
        (Some("recipient_retired"), "", 1, true),
        (None, "prepared", 1, true),
        (Some("pending"), "sparse", 1, true),
        (None, "", 0, false),
    ] {
        let window = OfferWindowFixture::new(false);
        let sibling = second_binding(&window.f);
        let message = window.send(0, DeliveryMode::Ordinary, "logical authority");
        let f = &window.f;
        f.db.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,ack_required) VALUES (?1,'t','s','acked',300000,?2)",params![message.as_str(),legacy_required]).unwrap();
        if let Some(state) = state {
            f.db.execute("INSERT INTO receipt_state(message_id,seat_id,state,ack_required) VALUES (?1,'s',?2,?3)",params![message.as_str(),state,if waiver=="sparse" {0} else {1}]).unwrap();
        }
        if waiver == "prepared" {
            f.db.execute("UPDATE prepared_recipients SET ack_required=0 WHERE seat_id='s' AND preparation_id=(SELECT preparation_id FROM send_manifests WHERE message_id=?1)",[message.as_str()]).unwrap();
        }
        let reconcile = |claim: &CallerClaim| {
            f.store
                .presentation_pull(
                    offer_reconciliation_input(f, claim, vec![message.clone()]),
                    &budget(),
                )
                .unwrap()
        };
        let expected_probes = if want { 9 } else { 4 };
        for claim in [&window.recipient, &sibling] {
            let unread = reconcile(claim);
            assert!(
                unread.reclaimed_messages.is_empty(),
                "seat authority does not complete either binding's unread body: state={state:?} waiver={waiver}"
            );
            assert_eq!(unread.work.offer_reconciliation_visits, 1);
            assert_eq!(unread.work.offer_reconciliation_probes, expected_probes);
        }
        assert_eq!(reclamation_read_full_body(f, &message), "logical authority");
        reclamation_advance_displayed(
            f,
            &window.recipient,
            std::slice::from_ref(&message),
            "logical-authority-full-display",
        );
        let output = reconcile(&window.recipient);
        assert_eq!(
            output.reclaimed_messages,
            if want { vec![message.clone()] } else { vec![] },
            "state={state:?} waiver={waiver} legacy_required={legacy_required}"
        );
        assert_eq!(output.work.offer_reconciliation_visits, 1);
        assert_eq!(output.work.offer_reconciliation_probes, expected_probes);
        let still_unread = reconcile(&sibling);
        assert!(
            still_unread.reclaimed_messages.is_empty(),
            "only the fully displayed binding may reclaim a seat-terminal identity"
        );
        assert_eq!(still_unread.work.offer_reconciliation_visits, 1);
        assert_eq!(
            still_unread.work.offer_reconciliation_probes,
            expected_probes
        );
    }
    // Legacy waiver flags are themselves canonical materialized terminal
    // evidence when no manifest addressed authority exists.
    let f = fixture(1);
    let recipient = pull_seat(&f);
    f.db.execute(
        "UPDATE receipts SET ack_required=0 WHERE message_id='m1' AND seat_id='s'",
        [],
    )
    .unwrap();
    let output = f
        .store
        .presentation_pull(
            offer_reconciliation_input(&f, &recipient, ids(&["m1"])),
            &budget(),
        )
        .unwrap();
    assert_eq!(output.reclaimed_messages, ids(&["m1"]));
    assert_eq!(output.work.offer_reconciliation_probes, 10);
}

// Kills: confusing staged or uncompleted lazy rows with canonically published
// displayed progress. Migration26 introduced only manifest-published lazy mail.
#[test]
fn presentation_reclamation_requires_published_addressed_lazy_completion() {
    use crate::protocol::{
        commands::{BodyReadRequest, CompleteInboxDelivery, MessageQuery},
        results::MessageContent,
    };
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let sibling = second_binding(&f);
    // Complete the first row only; the still-unread later row remains a contiguous
    // cursor barrier. Both bindings existed before any of these publications.
    for id in [
        "msg-old-displayed",
        "msg-old-pending",
        "msg-unpublished-displayed",
    ] {
        lazy_message(&f, id, "retained lazy body");
    }
    f.db.execute("UPDATE lazy_recipients SET state='displayed' WHERE seat_id='s' AND message_id!='msg-old-pending'", []).unwrap();
    f.db.execute(
        "DELETE FROM send_manifests WHERE message_id='msg-unpublished-displayed'",
        [],
    )
    .unwrap();
    let reconcile = |claim: &CallerClaim| {
        f.store
            .presentation_pull(
                offer_reconciliation_input(
                    &f,
                    claim,
                    ids(&[
                        "msg-old-pending",
                        "msg-old-displayed",
                        "msg-unpublished-displayed",
                    ]),
                ),
                &budget(),
            )
            .unwrap()
    };
    for claim in [&recipient, &sibling] {
        let output = reconcile(claim);
        assert!(
            output.reclaimed_messages.is_empty(),
            "historical seat display is not this binding's completion"
        );
        assert_eq!(output.work.offer_reconciliation_visits, 3);
        assert_eq!(output.work.offer_reconciliation_probes, 30);
    }
    let CommandResult::Message(read) = f
        .service
        .handle(
            Command::Message(MessageQuery {
                message: MessageId::new("msg-old-displayed"),
                body: BodyReadRequest {
                    cursor: None,
                    offset: None,
                    max_bytes: 4096,
                },
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("public full body read")
    };
    let MessageContent::Ordinary {
        body_data,
        body_complete,
        body_next_cursor,
        ..
    } = read.content
    else {
        panic!("lazy message carries ordinary body content")
    };
    assert_eq!(body_data, "retained lazy body");
    assert!(body_complete);
    assert!(body_next_cursor.is_none());
    let shown = ids(&["msg-old-displayed"]);
    let CommandResult::InboxDeliveryCompleted(completed) = f
        .service
        .handle(
            Command::CompleteInboxDelivery(CompleteInboxDelivery {
                messages: shown.clone(),
                operation: OperationId::new("reclamation-full-lazy-display"),
                claim: recipient.clone(),
                via: None,
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("public text-inbox completion")
    };
    assert_eq!(completed, shown);
    let output = reconcile(&recipient);
    assert_eq!(output.reclaimed_messages, shown);
    assert_eq!(output.work.offer_reconciliation_visits, 3);
    assert_eq!(output.work.offer_reconciliation_probes, 30);
    let other = reconcile(&sibling);
    assert!(
        other.reclaimed_messages.is_empty(),
        "the sibling still has unread published mail despite the seat display state"
    );
    assert_eq!(other.work.offer_reconciliation_visits, 3);
    assert_eq!(other.work.offer_reconciliation_probes, 30);
    assert_eq!(lazy_state(&f, "msg-old-pending"), "pending");
    assert_eq!(lazy_state(&f, "msg-old-displayed"), "displayed");
    assert_eq!(lazy_state(&f, "msg-unpublished-displayed"), "displayed");
}

// Kills: suppressing the terminal delta on recovery replay, or running its
// keyed reads before exact canonical scope/frozen-render admission.
#[test]
fn presentation_reclamation_replay_returns_delta_and_refusals_do_no_reconciliation() {
    use crate::protocol::delivery::DeliveryPullReplyV2;
    let f = fixture(1);
    let recipient = pull_seat(&f);
    let inviter = check_in(
        &f,
        claim("x", "px", fourth(), "reclamation-inviter", 1),
        "reclamation-inviter-ci",
    );
    canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 1, "reclamation-replay");
    f.db.execute(
        "UPDATE receipts SET state='acked' WHERE message_id='m1' AND seat_id='s'",
        [],
    )
    .unwrap();
    let first = f
        .store
        .presentation_pull(
            offer_reconciliation_input(&f, &recipient, vec![]),
            &budget(),
        )
        .unwrap();
    assert!(
        matches!(first.reply, DeliveryPullReplyV2::Page(ref page) if page.presentation_token.is_some())
    );
    let input = offer_reconciliation_input(&f, &recipient, ids(&["m1"]));
    assert!(input.frozen_render.is_some());
    let replay = f.store.presentation_pull(input.clone(), &budget()).unwrap();
    assert!(replay.work.replay > 0);
    assert_eq!(replay.reclaimed_messages, ids(&["m1"]));
    assert_eq!(replay.work.offer_reconciliation_visits, 1);
    assert_eq!(replay.work.offer_reconciliation_probes, 10);
    for scope_refusal in [true, false] {
        let mut refused = input.clone();
        if scope_refusal {
            refused.pull.claim = fresh_execution(recipient.clone(), 3);
        } else {
            refused.frozen_render = None;
        }
        let output = f.store.presentation_pull(refused, &budget()).unwrap();
        assert!(matches!(output.reply, DeliveryPullReplyV2::Refused { .. }));
        assert!(output.reclaimed_messages.is_empty());
        assert_eq!(output.work.offer_reconciliation_visits, 0);
        assert_eq!(output.work.offer_reconciliation_probes, 0);
    }
}

// Kills: silent truncation/cap raising on unbounded internal snapshots. Refuse
// before private presentation progress is changed or terminal delta is applied.
#[test]
fn presentation_reclamation_refuses_oversize_hints_before_effects() {
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let count = || {
        f.db.query_row(
            "SELECT count(*) FROM invitation_presentation_consumers",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
    };
    let before = count();
    for field in ["known", "blocked", "slots"] {
        let mut input = offer_reconciliation_input(&f, &recipient, vec![]);
        match field {
            "known" => {
                input.known_messages = (0..257)
                    .map(|n| MessageId::new(format!("unknown-{n}")))
                    .collect()
            }
            "blocked" => {
                input.blocked_messages = (0..257)
                    .map(|n| MessageId::new(format!("unknown-{n}")))
                    .collect()
            }
            _ => input.new_message_slots = 257,
        }
        let error = f.store.presentation_pull(input, &budget()).unwrap_err();
        assert_eq!(
            error.code,
            crate::protocol::results::ErrorCode::InvalidRequest
        );
        assert_eq!(
            count(),
            before,
            "{field} overflow must not mutate presentation state"
        );
    }
}

#[test]
fn presentation_publication_foreign_refresh_refuses_without_touching_hook_or_watch() {
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let inviter = check_in(
        &f,
        fixture_claim(&f, "x", "px", fourth(), "foreign-refresh-inviter"),
        "foreign-refresh-ci",
    );
    canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 1, "foreign-refresh");
    let _ = presentation_test_pull(&f, &recipient);
    let hook = || {
        f.db.query_row("SELECT outbox_token,outbox_text,ordinary_publication_after,ordinary_passive_generation FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?))).unwrap()
    };
    let before_hook = hook();
    f.store
        .presentation_refresh(
            &recipient.seat,
            &binding_for_claim(&f, &recipient),
            recipient.binding_generation,
            recipient.native_session.as_str(),
            &budget(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        hook(),
        before_hook,
        "Watch maintenance is independent of Hook presentation"
    );
    f.db.execute(
        "INSERT INTO host_instances(id,created_at) VALUES ('foreign-instance',0)",
        [],
    )
    .unwrap();
    f.db.execute("UPDATE invitation_presentation_consumers SET instance_id='foreign-instance' WHERE seat_id='s' AND consumer='watch'",[]).unwrap();
    let watch = || {
        f.db.query_row("SELECT instance_id,mod_generation,mod_execution,ordinary_publication_after,ordinary_passive_generation FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='watch'",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?,r.get::<_,i64>(3)?,r.get::<_,i64>(4)?))).unwrap()
    };
    let before = watch();
    let result = f.store.presentation_refresh(
        &recipient.seat,
        &binding_for_claim(&f, &recipient),
        recipient.binding_generation,
        recipient.native_session.as_str(),
        &budget(),
    );
    assert!(matches!(
        result,
        Err(crate::protocol::results::ApiError {
            code: crate::protocol::results::ErrorCode::StoreCorrupt,
            ..
        })
    ));
    assert_eq!(watch(), before);
    assert_eq!(hook(), before_hook);
}

#[test]
fn presentation_handler_initial_ordinary_receipt_delivers_body_without_invented_attention() {
    use crate::protocol::{
        commands::{DeliveryMode, SendMessage},
        delivery::{DeliveryPullReplyV2, PresentationControlStampV2},
    };
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let sender = check_in(
        &f,
        fixture_claim(&f, "x", "px", fourth(), "initial-ordinary-sender"),
        "initial-ordinary-ci",
    );
    f.service
        .handle(
            Command::SendMessage(SendMessage {
                delivery_mode: DeliveryMode::Ordinary,
                user_intent: None,
                thread: ThreadId::new("t"),
                body: "ordinary bounded response needed".into(),
                direct_recipient: None,
                invited_recipients: vec![],
                deadline_millis: None,
                operation: OperationId::new("initial-ordinary-send"),
                claim: sender,
                relays_user: false,
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM invitations WHERE seat_id='s' AND state='pending'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let DeliveryPullReplyV2::Page(page) = presentation_test_pull(&f, &recipient) else {
        panic!("actual ordinary body")
    };
    assert!(page.text.contains("ordinary bounded response needed")); // M Attention relevance is confirming invitations plus warnings;
    // ordinary ACK obligations retain their own nudge-class message output.
    assert!(
        !page
            .emitted_controls
            .iter()
            .any(|v| matches!(v, PresentationControlStampV2::Attention { .. }))
    );
    assert!(!page.offered.is_empty());
    assert_eq!(f.db.query_row("SELECT count(*) FROM prepared_recipients pr JOIN send_manifests sm ON sm.preparation_id=pr.preparation_id LEFT JOIN receipt_state rs ON rs.message_id=sm.message_id AND rs.seat_id=pr.seat_id WHERE pr.seat_id='s' AND COALESCE(rs.state,'pending')='pending' AND pr.ack_required=1",[],|r|r.get::<_,i64>(0)).unwrap(),1,"body presentation does not ACK ordinary obligation");
}

#[test]
fn presentation_publication_watch_reconnect_keeps_positive_frontier_without_native_wake() {
    use crate::{
        ports::{ModChannelSink, ModChannels, ModStoreReads},
        protocol::{
            commands::{DeliveryMode, SendMessage},
            watch::{ModDeliverySetting, WatchFrame, WatchRequest},
        },
        service::mod_channels::ModChannelRegistry,
    };
    struct Sink;
    impl ModChannelSink for Sink {
        fn push(&self, _: WatchFrame) -> bool {
            true
        }
    }
    let mut f = fixture(0);
    let recipient = check_in(
        &f,
        fixture_claim(
            &f,
            "c",
            "pc",
            Harness::Claude,
            "publication-reconnect-recipient",
        ),
        "publication-reconnect-ci",
    );
    let sender = check_in(
        &f,
        fixture_claim(&f, "x", "px", fourth(), "publication-reconnect-sender"),
        "publication-reconnect-sender-ci",
    );
    let reads: Arc<dyn ModStoreReads> = f.store.clone();
    let registry = Arc::new(ModChannelRegistry::new(
        f.clock.clone(),
        reads,
        "i".into(),
        Arc::new(|| {}),
        ModDeliverySetting::On,
    ));
    f.service = DomainService::new("i".into(), f.store.clone(), f.clock.clone())
        .with_cooperative_owner(501, Arc::new(FairWriter::new(32)))
        .with_delivery_pull(f.state.clone())
        .with_mod_channels(registry.clone());
    let request = WatchRequest {
        presentation_v2: true,
        claim: recipient.clone(),
    };
    let (first, _) = f
        .service
        .watch_register(&request, Arc::new(Sink), &budget())
        .unwrap();
    let old = registry.presentation_lease(first).unwrap();
    for n in 0..201 {
        f.service
            .handle(
                Command::SendMessage(SendMessage {
                    delivery_mode: DeliveryMode::Lazy,
                    user_intent: None,
                    thread: ThreadId::new("t"),
                    body: format!("reconnect body {n}"),
                    direct_recipient: None,
                    invited_recipients: vec![],
                    deadline_millis: None,
                    operation: OperationId::new(format!("publication-reconnect-{n}")),
                    claim: sender.clone(),
                    relays_user: false,
                }),
                peer(),
                &budget(),
            )
            .unwrap();
    }
    let native = wake_attention(&f);
    registry.pass(f.clock.utc_now()).unwrap();
    let state = || {
        f.db.query_row("SELECT ordinary_publication_after,ordinary_publication_high,ordinary_passive_generation FROM invitation_presentation_consumers WHERE seat_id='c' AND consumer='watch'",[],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?))).unwrap()
    };
    let before = state();
    assert!(before.0 > 0 && before.0 < before.1);
    assert_eq!(before.2, 1);
    registry.unregister(first, f.clock.utc_now());
    assert!(!registry.presentation_owner_live(
        &recipient.seat,
        &binding_for_claim(&f, &recipient),
        recipient.binding_generation,
        recipient.native_session.as_str(),
        &old
    ));
    assert_eq!(
        state(),
        before,
        "grace transition does not rewrite canonical cursor"
    );
    let (second, _) = f
        .service
        .watch_register(&request, Arc::new(Sink), &budget())
        .unwrap();
    let new = registry.presentation_lease(second).unwrap();
    assert_ne!(old, new);
    registry.pass(f.clock.utc_now()).unwrap();
    let continued = state();
    assert!(continued.0 > before.0);
    assert_eq!(continued.1, before.1);
    assert_eq!(continued.2, 2);
    assert_eq!(
        wake_attention(&f),
        native,
        "passive reconnect maintenance never creates native eligibility"
    );
    registry.unregister(second, f.clock.utc_now());
}

#[test]
fn presentation_publication_generation_overflow_rolls_back_frontier_and_version() {
    use crate::protocol::commands::{DeliveryMode, SendMessage};
    for field in ["ordinary_passive_generation", "mod_version"] {
        let f = fixture(0);
        let recipient = pull_seat(&f);
        let sender = check_in(
            &f,
            fixture_claim(&f, "x", "px", fourth(), "publication-overflow-sender"),
            "publication-overflow-ci",
        );
        f.store
            .presentation_refresh(
                &recipient.seat,
                &binding_for_claim(&f, &recipient),
                recipient.binding_generation,
                recipient.native_session.as_str(),
                &budget(),
            )
            .unwrap()
            .unwrap();
        f.db.execute(&format!("UPDATE invitation_presentation_consumers SET {field}=?1 WHERE seat_id='s' AND consumer='watch'"),[i64::MAX]).unwrap();
        f.service
            .handle(
                Command::SendMessage(SendMessage {
                    delivery_mode: DeliveryMode::Lazy,
                    user_intent: None,
                    thread: ThreadId::new("t"),
                    body: "overflow addressed body".into(),
                    direct_recipient: None,
                    invited_recipients: vec![],
                    deadline_millis: None,
                    operation: OperationId::new("publication-overflow-send"),
                    claim: sender,
                    relays_user: false,
                }),
                peer(),
                &budget(),
            )
            .unwrap();
        let snapshot = || {
            f.db.query_row("SELECT ordinary_publication_after,ordinary_publication_high,ordinary_passive_generation,ordinary_scan_passive_generation,mod_version FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='watch'",[],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?,r.get::<_,i64>(4)?))).unwrap()
        };
        let before = snapshot();
        let result = f.store.presentation_refresh(
            &recipient.seat,
            &binding_for_claim(&f, &recipient),
            recipient.binding_generation,
            recipient.native_session.as_str(),
            &budget(),
        );
        assert!(
            matches!(
                result,
                Err(crate::protocol::results::ApiError {
                    code: crate::protocol::results::ErrorCode::SequenceExhausted,
                    ..
                })
            ),
            "{field} overflow must refuse: {result:?}"
        );
        assert_eq!(
            snapshot(),
            before,
            "no consumed publication or partial version write on overflow"
        );
    }
}

#[test]
fn presentation_quiet_channel_observes_new_ordinary_ack_message() {
    use crate::{
        ports::{ModChannelSink, ModChannels, ModStoreReads},
        protocol::{
            commands::{DeliveryMode, SendMessage},
            watch::{ModDeliverySetting, WatchFrame, WatchRequest},
        },
        service::mod_channels::ModChannelRegistry,
    };
    #[derive(Default)]
    struct Sink(std::sync::Mutex<Vec<WatchFrame>>);
    impl ModChannelSink for Sink {
        fn push(&self, frame: WatchFrame) -> bool {
            self.0.lock().unwrap().push(frame);
            true
        }
    }
    let mut f = fixture(0);
    let recipient = check_in(
        &f,
        fixture_claim(&f, "c", "pc", Harness::Claude, "quiet-ordinary-recipient"),
        "quiet-ordinary-ci",
    );
    let sender = check_in(
        &f,
        fixture_claim(&f, "x", "px", fourth(), "quiet-ordinary-sender"),
        "quiet-ordinary-sender-ci",
    );
    let reads: Arc<dyn ModStoreReads> = f.store.clone();
    let registry = Arc::new(ModChannelRegistry::new(
        f.clock.clone(),
        reads,
        "i".into(),
        Arc::new(|| {}),
        ModDeliverySetting::On,
    ));
    f.service = DomainService::new("i".into(), f.store.clone(), f.clock.clone())
        .with_cooperative_owner(501, Arc::new(FairWriter::new(32)))
        .with_delivery_pull(f.state.clone())
        .with_mod_channels(registry.clone());
    let sink = Arc::new(Sink::default());
    let (channel, _) = f
        .service
        .watch_register(
            &WatchRequest {
                presentation_v2: true,
                claim: recipient.clone(),
            },
            sink.clone(),
            &budget(),
        )
        .unwrap();
    registry.pass(f.clock.utc_now()).unwrap();
    sink.0.lock().unwrap().clear();
    let native = || {
        f.db.query_row(
            "SELECT COALESCE((SELECT attention_version FROM wake_work WHERE seat_id='c'),0)",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
    };
    let before = native();
    let CommandResult::MessageSent(message) = f
        .service
        .handle(
            Command::SendMessage(SendMessage {
                delivery_mode: DeliveryMode::Ordinary,
                user_intent: None,
                thread: ThreadId::new("t"),
                body: "new ordinary ACK content after quiet registration".into(),
                direct_recipient: None,
                invited_recipients: vec![],
                deadline_millis: None,
                operation: OperationId::new("quiet-ordinary-send"),
                claim: sender,
                relays_user: false,
            }),
            peer(),
            &budget(),
        )
        .unwrap()
    else {
        panic!("ordinary message")
    };
    assert_eq!(native(), before, "native materializer has not yet run");

    let job:String=f.db.query_row("SELECT id FROM work_jobs WHERE kind='send_attention' AND subject_id=(SELECT preparation_id FROM send_manifests WHERE message_id=?1)",[message.as_str()],|r|r.get(0)).unwrap();
    finish_presentation_attention_job(&f, &job);
    assert!(native() > before);
    registry.mark_dirty();
    registry.pass(f.clock.utc_now()).unwrap();
    let frames = sink.0.lock().unwrap().clone();
    assert!(
        matches!(frames.as_slice(), [WatchFrame::Attention { .. }]),
        "new ordinary ACK message needs an admitted channel drain after quiet registration: {frames:?}"
    );
    registry.pass(f.clock.utc_now()).unwrap();
    assert_eq!(
        *sink.0.lock().unwrap(),
        frames,
        "unchanged ordinary publication does not repeat drains"
    );

    let CommandResult::DeliveryPulledV2(crate::protocol::delivery::DeliveryPullReplyV2::Page(page)) =
        f.service
            .handle(
                Command::DeliveryPullV2(crate::protocol::delivery::DeliveryPullV2 {
                    claim: recipient.clone(),
                    consumer: crate::protocol::delivery::PresentationConsumer::Watch,
                    render: crate::protocol::delivery::PresentationRenderRequest::WatchJsonLinesV1,
                    presentation_lease: registry.presentation_lease(channel),
                    marker_prefix: vec![],
                }),
                peer(),
                &budget(),
            )
            .unwrap()
    else {
        panic!("ordinary admitted drain")
    };
    assert!(page.items.iter().any(|v|matches!(v,crate::protocol::delivery::PresentationItemV2::Message{id,lazy:false,content} if id==&message && content.body=="new ordinary ACK content after quiet registration" && content.ack_required)),"ordered Watch item delivers exact ordinary body and ACK requirement");
    assert!(
        page.emitted_controls.is_empty(),
        "ordinary receipt alone does not invent generic Attention"
    );
    assert_eq!(
        f.db.query_row(
            "SELECT state FROM receipt_state WHERE message_id=?1 AND seat_id='c'",
            [message.as_str()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "pending",
        "display does not ACK"
    );
    f.clock.0.store(
        1_000_000 + crate::protocol::watch::MOD_STALL_AFTER_MS as i64 + 1,
        Ordering::Relaxed,
    );
    assert!(
        registry.stalled(&recipient.seat, f.clock.utc_now()),
        "successful native observer push preserves ordinary-only legacy publication cutoff"
    );
    registry.unregister(channel, f.clock.utc_now());
}

// These fixtures catch unbounded/private ID reads and unchecked constructors in
// each independently reachable cleanup, dirty-sizing and revisit branch.
fn private_invitation_id_fixture(branch: &str) -> (Fixture, CallerClaim, i64) {
    use crate::protocol::delivery::DeliveryPullReplyV2;
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let inviter = check_in(
        &f,
        fixture_claim(&f, "x", "px", fourth(), "private-id-inviter"),
        "private-id-ci",
    );
    canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 1, "private-id");
    let DeliveryPullReplyV2::Page(page) = presentation_test_pull(&f, &recipient) else {
        panic!("admitted Hook page")
    };
    assert_eq!(presentation_invitations(&page).len(), 1);
    presentation_test_confirm(&f, &recipient, &page);
    let ordinal = f.db.query_row(
        "SELECT revisit_ordinal FROM invitation_presentation_items WHERE seat_id='s' AND consumer='hook'",
        [],
        |r| r.get::<_, i64>(0),
    ).unwrap();
    let cleanup_after = if branch == "cleanup" { 0 } else { ordinal };
    let dirty_after = if branch == "dirty" { 0 } else { ordinal };
    let required_bytes = if branch == "revisit" { 1 } else { i64::MAX };
    f.db.execute(
        "UPDATE invitation_presentation_items SET required_render_bytes=?1 WHERE seat_id='s' AND consumer='hook'",
        [required_bytes],
    ).unwrap();
    f.db.execute(
        "UPDATE invitation_presentation_consumers SET initial_after=?1,initial_high=?1,context_after_seq=COALESCE((SELECT MAX(last_change_seq) FROM invitation_context_latest),0),context_high=COALESCE((SELECT MAX(last_change_seq) FROM invitation_context_latest),0),context_after_thread=?2,context_thread='',context_selected_seq=0,context_invitation_after=0,context_invitation_high=0,cleanup_after=?3,dirty_after=?4,dirty_high=?1,revisit_after_bytes=0,revisit_after_ordinal=0,revisit_high_bytes=?5,revisit_high_ordinal=?6 WHERE seat_id='s' AND consumer='hook'",
        params![ordinal, "\u{10ffff}", cleanup_after, dirty_after, i64::from(branch == "revisit"), if branch == "revisit" { ordinal } else { 0 }],
    ).unwrap();
    assert!(f.db.query_row(
        "SELECT outbox_token IS NULL FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='hook'",
        [], |r| r.get::<_, bool>(0),
    ).unwrap(), "confirmed allocation cannot bypass private reads through replay");
    let selected = f.db.query_row(
        "SELECT (SELECT COUNT(*) FROM invitation_presentation_items i WHERE i.seat_id=s.seat_id AND i.consumer=s.consumer AND i.revisit_ordinal>s.cleanup_after),(SELECT COUNT(*) FROM invitation_presentation_items i WHERE i.seat_id=s.seat_id AND i.consumer=s.consumer AND i.revisit_ordinal>s.dirty_after AND i.revisit_ordinal<=s.dirty_high),(SELECT COUNT(*) FROM invitation_presentation_items i WHERE i.seat_id=s.seat_id AND i.consumer=s.consumer AND i.generation=s.generation AND (i.required_render_bytes,i.revisit_ordinal)>(s.revisit_after_bytes,s.revisit_after_ordinal) AND (i.required_render_bytes,i.revisit_ordinal)<=(s.revisit_high_bytes,s.revisit_high_ordinal) AND i.required_render_bytes<=65536) FROM invitation_presentation_consumers s WHERE s.seat_id='s' AND s.consumer='hook'",
        [], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)),
    ).unwrap();
    assert_eq!(
        selected,
        match branch {
            "cleanup" => (1, 0, 0),
            "dirty" => (0, 1, 0),
            "revisit" => (0, 0, 1),
            _ => panic!("unknown branch"),
        },
        "one private reader is independently reachable"
    );
    (f, recipient, ordinal)
}

// Preserve raw SQLite type and bytes, including malformed UTF-8. Snapshot every
// table so private frontiers, canonical invitations, receipts and warnings all
// participate in the no-effect assertion; row sorting also supports WITHOUT ROWID.
fn private_invitation_id_snapshot(db: &Connection) -> Vec<(String, Vec<Vec<u8>>)> {
    use rusqlite::types::ValueRef;
    let mut tables = db.prepare(
        "SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    ).unwrap();
    let names = tables
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    names
        .into_iter()
        .map(|name| {
            let mut query = db
                .prepare(&format!("SELECT * FROM \"{}\"", name.replace('"', "\"\"")))
                .unwrap();
            let columns = query.column_count();
            let mut rows = query
                .query_map([], |row| {
                    let mut bytes = Vec::new();
                    for column in 0..columns {
                        let (tag, raw) = match row.get_ref(column)? {
                            ValueRef::Null => (0, Vec::new()),
                            ValueRef::Integer(v) => (1, v.to_le_bytes().to_vec()),
                            ValueRef::Real(v) => (2, v.to_bits().to_le_bytes().to_vec()),
                            ValueRef::Text(v) => (3, v.to_vec()),
                            ValueRef::Blob(v) => (4, v.to_vec()),
                        };
                        bytes.push(tag);
                        bytes.extend_from_slice(&(raw.len() as u64).to_le_bytes());
                        bytes.extend_from_slice(&raw);
                    }
                    Ok(bytes)
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            rows.sort();
            (name, rows)
        })
        .collect()
}

fn repair_private_invitation_id(
    f: &Fixture,
    value: rusqlite::types::Value,
    cast_text: bool,
) -> rusqlite::Result<usize> {
    repair_private_invitation_id_for_consumer(f, "hook", value, cast_text)
}

fn repair_private_invitation_id_for_consumer(
    f: &Fixture,
    consumer: &str,
    value: rusqlite::types::Value,
    cast_text: bool,
) -> rusqlite::Result<usize> {
    // Deliberately corrupt only the private ID through a short-lived repair
    // connection. Never relax the service/writer connection's foreign keys.
    let repair = Connection::open(f.directory.join("store.db")).unwrap();
    repair.pragma_update(None, "foreign_keys", true).unwrap();
    let before = repair
        .pragma_query_value(None, "foreign_keys", |r| r.get::<_, bool>(0))
        .unwrap();
    repair.pragma_update(None, "foreign_keys", false).unwrap();
    assert!(
        !repair
            .pragma_query_value(None, "foreign_keys", |r| r.get::<_, bool>(0))
            .unwrap()
    );
    let sql = if cast_text {
        "UPDATE invitation_presentation_items SET invitation_id=CAST(?1 AS TEXT) WHERE seat_id='s' AND consumer=?2"
    } else {
        "UPDATE invitation_presentation_items SET invitation_id=?1 WHERE seat_id='s' AND consumer=?2"
    };
    let result = repair.execute(sql, params![value, consumer]);
    repair.pragma_update(None, "foreign_keys", before).unwrap();
    assert!(
        repair
            .pragma_query_value(None, "foreign_keys", |r| r.get::<_, bool>(0))
            .unwrap()
    );
    assert!(
        f.db.pragma_query_value(None, "foreign_keys", |r| r.get::<_, bool>(0))
            .unwrap()
    );
    result
}

fn pull_private_invitation_id(
    f: &Fixture,
    recipient: &CallerClaim,
) -> Result<crate::ports::PresentationOutput, crate::protocol::results::ApiError> {
    use crate::{
        delivery::presentation::{FrozenPresentationRender, PresentationRenderProfile},
        ports::PresentationInput,
        protocol::delivery::{DeliveryPullV2, PresentationConsumer, PresentationRenderRequest},
    };
    let registry = crate::harness::registry::builtins();
    let adapter = registry
        .by_id(registry.agent(recipient.harness.as_str()).unwrap())
        .unwrap();
    let codec = adapter.presentation_codec(0).unwrap();
    let frozen_render = f
        .store
        .presentation_render_selection(recipient, PresentationConsumer::Hook, &budget())
        .unwrap()
        .map(|identity| FrozenPresentationRender {
            identity,
            codec: Some(codec),
        });
    f.store.presentation_pull(
        PresentationInput {
            binding_id: binding_for_claim(f, recipient),
            owner_replay: None,
            ordinary_due_snapshot: None,
            frozen_render,
            render_profile: PresentationRenderProfile::Hook {
                confirmable_selector: 0,
                passive_selector: 0,
                confirmable: codec,
                passive: codec,
            },
            pull: DeliveryPullV2 {
                claim: recipient.clone(),
                consumer: PresentationConsumer::Hook,
                render: PresentationRenderRequest::Hook {
                    confirmable_selector: 0,
                    passive_selector: 0,
                },
                presentation_lease: None,
                marker_prefix: vec![],
            },
            attention_version: 0,
            ordinary_version: 0,
            with_attention: false,
            retry_messages: false,
            blocked_messages: vec![],
            known_messages: vec![],
            new_message_slots: 256,
        },
        &budget(),
    )
}

#[test]
fn presentation_private_invitation_ids_refuse_corruption_in_each_reader_without_effects() {
    use crate::protocol::results::{ApiError, ErrorCode};
    use rusqlite::types::Value;
    let mut failures = Vec::new();
    for branch in ["cleanup", "dirty", "revisit"] {
        for (kind, value) in [
            ("empty", Value::Text(String::new())),
            ("overlong", Value::Text("x".repeat(129))),
            ("space", Value::Text("bad id".into())),
            ("tab", Value::Text("bad\tid".into())),
            ("control", Value::Text("bad\0id".into())),
            ("non_ascii", Value::Text("λ".into())),
            ("invalid_utf8", Value::Blob(vec![0x80])),
        ] {
            let (f, recipient, _) = private_invitation_id_fixture(branch);
            assert_eq!(repair_private_invitation_id(&f, value, true).unwrap(), 1);
            let before = private_invitation_id_snapshot(&f.db);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                pull_private_invitation_id(&f, &recipient)
            }));
            if !matches!(
                result,
                Ok(Err(ApiError {
                    code: ErrorCode::StoreCorrupt,
                    ..
                }))
            ) {
                failures.push(format!(
                    "{branch}/{kind}: expected StoreCorrupt without panic, got {result:?}"
                ));
            }
            if private_invitation_id_snapshot(&f.db) != before {
                failures.push(format!(
                    "{branch}/{kind}: private or canonical rows changed after refusal"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn presentation_private_invitation_ids_allow_exact_128_byte_ids_in_each_reader() {
    for branch in ["cleanup", "dirty", "revisit"] {
        let (f, recipient, _) = private_invitation_id_fixture(branch);
        assert_eq!(
            repair_private_invitation_id(&f, rusqlite::types::Value::Text("x".repeat(128)), true)
                .unwrap(),
            1
        );
        let output = pull_private_invitation_id(&f, &recipient).unwrap();
        assert_eq!(output.work.initial, 0);
        assert_eq!(output.work.context, 0);
        assert_eq!(
            output.work.revisit, 1,
            "{branch}: valid boundary ID reaches its private reader"
        );
        assert_eq!(
            f.db.query_row(
                "SELECT COUNT(*) FROM invitations WHERE seat_id='s' AND state='pending'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            f.db.query_row("SELECT COUNT(*) FROM receipts", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn presentation_private_invitation_id_wrong_types_refuse_at_strict_storage_boundary() {
    for value in [
        rusqlite::types::Value::Blob(vec![0x80]),
        rusqlite::types::Value::Null,
    ] {
        let (f, _, _) = private_invitation_id_fixture("cleanup");
        let before = private_invitation_id_snapshot(&f.db);
        assert!(
            repair_private_invitation_id(&f, value, false).is_err(),
            "STRICT TEXT NOT NULL blocks constructing wrong-type corruption"
        );
        assert_eq!(private_invitation_id_snapshot(&f.db), before);
    }
}

fn private_watch_cleanup_id_fixture() -> (Fixture, CallerClaim) {
    // Keep a genuine Hook allocation/owner alongside independently initialized
    // passive Watch state; the corruption fixture must not manufacture a binding.
    let (f, recipient, ordinal) = private_invitation_id_fixture("cleanup");
    f.store
        .presentation_refresh(
            &recipient.seat,
            &binding_for_claim(&f, &recipient),
            recipient.binding_generation,
            recipient.native_session.as_str(),
            &budget(),
        )
        .unwrap()
        .unwrap();
    f.db.execute(
        "UPDATE invitation_presentation_consumers SET mod_initial_after=?1,mod_initial_high=?1,mod_context_after_seq=COALESCE((SELECT MAX(last_change_seq) FROM invitation_context_latest),0),mod_context_high=COALESCE((SELECT MAX(last_change_seq) FROM invitation_context_latest),0),mod_context_after_thread=?2,mod_context_thread='',mod_context_selected_seq=0,mod_context_invitation_after=0,mod_context_invitation_high=0,mod_cleanup_after=0,mod_cleanup_high=?1 WHERE seat_id='s' AND consumer='watch'",
        params![ordinal, "\u{10ffff}"],
    ).unwrap();
    let selected = f.db.query_row(
        "SELECT (SELECT COUNT(*) FROM digest_pending_invitations d WHERE d.seat_id=s.seat_id AND d.ordinal>s.mod_initial_after AND d.ordinal<=s.mod_initial_high),(SELECT COUNT(*) FROM invitation_context_latest c WHERE c.instance_id=s.instance_id AND (c.last_change_seq,c.thread_id)>(s.mod_context_after_seq,s.mod_context_after_thread) AND c.last_change_seq<=s.mod_context_high),(SELECT COUNT(*) FROM invitation_presentation_items i WHERE i.seat_id=s.seat_id AND i.consumer='watch' AND i.revisit_ordinal>s.mod_cleanup_after AND i.revisit_ordinal<=s.mod_cleanup_high AND i.mod_key!='' AND i.mod_generation=s.mod_generation AND i.mod_execution=s.mod_execution) FROM invitation_presentation_consumers s WHERE s.seat_id='s' AND s.consumer='watch'",
        [], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)),
    ).unwrap();
    assert_eq!(
        selected,
        (0, 0, 1),
        "only the owned passive Watch cleanup row is reachable"
    );
    assert!(f.db.query_row(
        "SELECT s.mod_generation=b.generation AND s.mod_execution=b.execution_id AND b.native_session=?1 FROM invitation_presentation_consumers s JOIN occupant_bindings b ON b.seat_id=s.seat_id AND b.ended_at IS NULL WHERE s.seat_id='s' AND s.consumer='watch'",
        [recipient.native_session.as_str()], |r| r.get::<_, bool>(0),
    ).unwrap(), "passive cleanup retains the genuine canonical owner");
    (f, recipient)
}

#[test]
fn presentation_private_watch_cleanup_ids_refuse_malformed_identity_without_effects() {
    use crate::protocol::results::{ApiError, ErrorCode};
    let mut failures = Vec::new();
    for (kind, id) in [
        ("empty", ""),
        ("space", "bad id"),
        ("tab", "bad\tid"),
        ("control", "bad\0id"),
        ("non_ascii", "λ"),
    ] {
        let (f, recipient) = private_watch_cleanup_id_fixture();
        assert_eq!(
            repair_private_invitation_id_for_consumer(
                &f,
                "watch",
                rusqlite::types::Value::Text(id.into()),
                true,
            )
            .unwrap(),
            1
        );
        let before = private_invitation_id_snapshot(&f.db);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.store.presentation_refresh(
                &recipient.seat,
                &binding_for_claim(&f, &recipient),
                recipient.binding_generation,
                recipient.native_session.as_str(),
                &budget(),
            )
        }));
        if !matches!(
            result,
            Ok(Err(ApiError {
                code: ErrorCode::StoreCorrupt,
                ..
            }))
        ) {
            failures.push(format!(
                "passive_watch_cleanup/{kind}: expected StoreCorrupt without panic, got {result:?}"
            ));
        }
        if private_invitation_id_snapshot(&f.db) != before {
            failures.push(format!(
                "passive_watch_cleanup/{kind}: private or canonical rows changed after refusal"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn presentation_private_watch_cleanup_ids_enforce_borrowed_text_bounds_without_effects() {
    use crate::protocol::results::{ApiError, ErrorCode};
    for (kind, value) in [
        ("overlong", rusqlite::types::Value::Text("x".repeat(129))),
        ("invalid_utf8", rusqlite::types::Value::Blob(vec![0x80])),
    ] {
        let (f, recipient) = private_watch_cleanup_id_fixture();
        assert_eq!(
            repair_private_invitation_id_for_consumer(&f, "watch", value, true).unwrap(),
            1
        );
        let before = private_invitation_id_snapshot(&f.db);
        let result = f.store.presentation_refresh(
            &recipient.seat,
            &binding_for_claim(&f, &recipient),
            recipient.binding_generation,
            recipient.native_session.as_str(),
            &budget(),
        );
        assert!(
            matches!(
                result,
                Err(ApiError {
                    code: ErrorCode::StoreCorrupt,
                    ..
                })
            ),
            "{kind}: {result:?}"
        );
        assert_eq!(private_invitation_id_snapshot(&f.db), before);
    }
}

#[test]
fn presentation_private_watch_cleanup_ids_allow_128_byte_id_without_canonical_owner_changes() {
    let (f, recipient) = private_watch_cleanup_id_fixture();
    let owner = || {
        f.db.query_row(
        "SELECT generation,native_session,execution_id,observation_provenance FROM occupant_bindings WHERE seat_id='s' AND ended_at IS NULL",
        [], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?)),
    ).unwrap()
    };
    let before_owner = owner();
    assert_eq!(
        repair_private_invitation_id_for_consumer(
            &f,
            "watch",
            rusqlite::types::Value::Text("x".repeat(128)),
            true,
        )
        .unwrap(),
        1
    );
    let output = f
        .store
        .presentation_refresh(
            &recipient.seat,
            &binding_for_claim(&f, &recipient),
            recipient.binding_generation,
            recipient.native_session.as_str(),
            &budget(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(output.context_visits, 0);
    assert_eq!(
        output.lifecycle_visits, 1,
        "only passive cleanup is visited"
    );
    assert_eq!(output.ordinary_publication_visits, 0);
    assert_eq!(owner(), before_owner);
    assert_eq!(f.db.query_row("SELECT COUNT(*) FROM invitation_presentation_items WHERE seat_id='s' AND consumer='watch'", [], |r| r.get::<_, i64>(0)).unwrap(), 0, "valid orphan private row is reclaimed");
    assert_eq!(f.db.query_row("SELECT COUNT(*) FROM invitation_presentation_items WHERE seat_id='s' AND consumer='hook'", [], |r| r.get::<_, i64>(0)).unwrap(), 1, "Hook owner state remains intact");
    assert_eq!(
        f.db.query_row(
            "SELECT COUNT(*) FROM invitations WHERE seat_id='s' AND state='pending'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        f.db.query_row("SELECT COUNT(*) FROM receipts", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}

// Kills: retrying the same absent first 100 IDs forever, or treating misses as settlement.
#[test]
fn presentation_capacity_missing_due_keys_finish_a_finite_pass() {
    use crate::protocol::delivery::DeliveryPullReplyV2;
    let window = OfferWindowFixture::new(true);
    // Prime independent controls/publication before installing conservative hints.
    for _ in 0..4 {
        window.pull();
    }
    {
        let mut ledger = window.f.state.lock();
        let entry = ledger
            .binding_mut(
                &window.recipient.seat,
                &binding_for_claim(&window.f, &window.recipient),
                window.recipient.binding_generation,
            )
            .unwrap();
        for n in 0..256 {
            entry.record(
                &MessageId::new(format!("absent-{n:03}")),
                window.f.clock.utc_now(),
                false,
            );
        }
    }
    advance(&window.f, REOFFER_AFTER_MS);
    let original = window.offers();
    let mut visits = Vec::new();
    for _ in 0..3 {
        let reply = window.pull();
        visits.push(
            window
                .f
                .service
                .presentation
                .last_work
                .lock()
                .unwrap()
                .unwrap()
                .ordinary,
        );
        assert_eq!(
            window.offers(),
            original,
            "a missing row is not terminal evidence"
        );
        assert!(
            matches!(reply, DeliveryPullReplyV2::Unchanged { .. })
                || matches!(reply, DeliveryPullReplyV2::Page(ref page) if page.offered.is_empty() && page.truncated.is_empty())
        );
    }
    assert_eq!(visits, vec![100, 100, 56]);
    for _ in 0..3 {
        assert!(matches!(
            window.pull(),
            DeliveryPullReplyV2::Unchanged { .. }
        ));
        assert_eq!(
            window
                .f
                .service
                .presentation
                .last_work
                .lock()
                .unwrap()
                .unwrap()
                .ordinary,
            0
        );
        assert_eq!(window.offers(), original);
    }
}

// Kills CDR-1: conservative missing due hints perpetuating a completed normal scan.
#[test]
fn presentation_capacity_reclaimed_or_initially_free_with_missing_due_keys_quiets() {
    use crate::protocol::{commands::DeliveryMode, delivery::DeliveryPullReplyV2};
    for reclaim in [false, true] {
        let window = OfferWindowFixture::new(true);
        let terminal = reclaim.then(|| window.send(0, DeliveryMode::Ordinary, "settled"));
        if let Some(id) = &terminal {
            assert_eq!(
                window.drain(1),
                std::collections::BTreeSet::from([id.clone()])
            );
            let stamps = window.offers();
            let cursor = reclamation_cursor(&window.f, &window.recipient);
            window.ack(std::slice::from_ref(id), false, false, "capacity-terminal");
            window.pull();
            assert_eq!(reclamation_cursor(&window.f, &window.recipient), cursor);
            assert_eq!(window.offers(), stamps, "ACK alone keeps the unread offer");
            assert!(!stamps[id].truncated, "the short body was fully emitted");
            reclamation_advance_displayed(
                &window.f,
                &window.recipient,
                std::slice::from_ref(id),
                "capacity-terminal-displayed",
            );
        }
        {
            let mut ledger = window.f.state.lock();
            let entry = ledger
                .binding_mut(
                    &window.recipient.seat,
                    &binding_for_claim(&window.f, &window.recipient),
                    window.recipient.binding_generation,
                )
                .unwrap();
            for n in 0..255 {
                entry.record(
                    &MessageId::new(format!("absent-{n:03}")),
                    window.f.clock.utc_now(),
                    false,
                );
            }
        }
        advance(&window.f, REOFFER_AFTER_MS);
        let original = window.offers();
        // Only one source row exists; four quanta allow control and publication work.
        let mut quiet = false;
        for _ in 0..4 {
            quiet = match window.pull() {
                DeliveryPullReplyV2::Unchanged { .. } => true,
                DeliveryPullReplyV2::Page(page) => !page.more,
                other => panic!("admitted fixture refused: {other:?}"),
            };
            if quiet {
                break;
            }
        }
        assert!(
            quiet,
            "missing due hints cannot restart a finished positive-capacity pass"
        );
        let retained = window.offers();
        assert_eq!(retained.len(), 255);
        for (id, stamp) in retained {
            assert_eq!(original[&id], stamp);
        }
    }
}

// Kills: final continuation using input slots instead of remaining capacity.
#[test]
fn presentation_capacity_last_slot_parks_unknown_carry_and_only_continues_for_due_keys() {
    use crate::protocol::{commands::DeliveryMode, delivery::DeliveryPullReplyV2};
    for missing_due in [false, true] {
        let window = OfferWindowFixture::new(true);
        for _ in 0..4 {
            window.pull();
        }
        {
            let mut ledger = window.f.state.lock();
            let entry = ledger
                .binding_mut(
                    &window.recipient.seat,
                    &binding_for_claim(&window.f, &window.recipient),
                    window.recipient.binding_generation,
                )
                .unwrap();
            for n in 0..255 {
                let now = window.f.clock.utc_now();
                entry.record(
                    &MessageId::new(format!("missing-{n:03}")),
                    if missing_due && n == 0 {
                        UtcMillis(now.0 - REOFFER_AFTER_MS)
                    } else {
                        now
                    },
                    false,
                );
            }
        }
        let first = window.send(0, DeliveryMode::Ordinary, "last slot");
        let parked_id = window.send(1, DeliveryMode::Ordinary, "must wait");
        let mut allocated = false;
        for _ in 0..4 {
            if let DeliveryPullReplyV2::Page(page) = window.pull()
                && page.offered.contains(&first)
            {
                assert_eq!(
                    page.more, missing_due,
                    "only a distinct runnable due key justifies continuation"
                );
                allocated = true;
                break;
            }
        }
        assert!(allocated);
        let parked = capacity_normal_state(&window);
        assert!(matches!(
            window.pull(),
            DeliveryPullReplyV2::Unchanged { .. }
        ));
        assert_eq!(capacity_normal_state(&window), parked);
        let work = window
            .f
            .service
            .presentation
            .last_work
            .lock()
            .unwrap()
            .unwrap();
        assert_eq!(work.ordinary, usize::from(missing_due));
        assert_eq!(window.offers().len(), 256);
        assert!(!window.offers().contains_key(&parked_id));
        let stamps = window.offers();
        let cursor = reclamation_cursor(&window.f, &window.recipient);
        window.ack(std::slice::from_ref(&first), false, false, "last-slot-ack");
        window.pull();
        assert_eq!(reclamation_cursor(&window.f, &window.recipient), cursor);
        assert_eq!(
            window.offers(),
            stamps,
            "ACK alone cannot free the last slot"
        );
        assert_eq!(capacity_normal_state(&window), parked);
        assert!(
            !stamps[&first].truncated,
            "the first body was fully emitted"
        );
        reclamation_advance_displayed(
            &window.f,
            &window.recipient,
            std::slice::from_ref(&first),
            "last-slot-displayed",
        );
        let DeliveryPullReplyV2::Page(page) = window.pull() else {
            panic!("resume retained sibling");
        };
        assert!(page.offered.contains(&parked_id));
    }
}

fn capacity_missing_input(
    f: &Fixture,
    recipient: &CallerClaim,
    count: usize,
) -> crate::ports::PresentationInput {
    let mut ledger = crate::delivery::ledger::OfferedLedger::new();
    let entry = ledger
        .binding_mut(
            &recipient.seat,
            &binding_for_claim(f, recipient),
            recipient.binding_generation,
        )
        .unwrap();
    for n in 0..count {
        entry.record(
            &MessageId::new(format!("missing-{n:03}")),
            UtcMillis(0),
            false,
        );
    }
    let mut input =
        offer_reconciliation_input(f, recipient, entry.offers.keys().cloned().collect());
    input.retry_messages = true;
    input.blocked_messages.clear();
    input.ordinary_due_snapshot =
        Some(entry.due_snapshot(crate::protocol::delivery::PresentationConsumer::Watch));
    input
}

// Kills: replay spending a second source slice, storing ordinary debt in outbox_more,
// or confirming passive context as acceptance/ACK. Tests both capacity predicates.
#[test]
fn presentation_capacity_replay_preserves_finite_due_progress_and_positive_slot_quiet() {
    use crate::protocol::delivery::{
        DeliveryPresentationConfirmV2, DeliveryPullReplyV2, PresentationConsumer,
    };
    for count in [255, 256] {
        let f = fixture(0);
        let recipient = pull_seat(&f);
        let inviter = check_in(
            &f,
            fixture_claim(&f, "x", "px", fourth(), "capacity-replay-inviter"),
            "capacity-replay-ci",
        );
        canonical_lazy_invitation_backlog(&f, &recipient, &inviter, 1, "capacity-replay");
        let mut input = capacity_missing_input(&f, &recipient, count);
        let first = f.store.presentation_pull(input.clone(), &budget()).unwrap();
        let DeliveryPullReplyV2::Page(page) = &first.reply else {
            panic!("passive context output");
        };
        assert!(page.offered.is_empty() && page.truncated.is_empty());
        assert_eq!(page.more, count == 256);
        let token = page.presentation_token.clone().unwrap();
        input.ordinary_due_snapshot = first.ordinary_due_update;
        input.frozen_render = presentation_watch_frozen(&f, &recipient);
        let before = input.ordinary_due_snapshot.clone();
        let replay = f.store.presentation_pull(input.clone(), &budget()).unwrap();
        assert!(replay.work.replay > 0);
        assert_eq!(
            (
                replay.work.ordinary,
                replay.work.ordinary_due_key_probes,
                replay.work.ordinary_due_seeks
            ),
            (0, 0, 0)
        );
        assert_eq!(replay.ordinary_due_update, before);
        assert!(
            matches!(replay.reply, DeliveryPullReplyV2::Page(ref page) if page.more == (count == 256))
        );
        assert_eq!(f.db.query_row("SELECT outbox_more FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='watch'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        f.store
            .presentation_confirm(
                &DeliveryPresentationConfirmV2 {
                    claim: recipient.clone(),
                    consumer: PresentationConsumer::Watch,
                    token,
                    presentation_lease: Some("0123456789abcdef0123456789abcdef".into()),
                    emitted_controls: page.emitted_controls.clone(),
                },
                &budget(),
            )
            .unwrap();
        input.frozen_render = presentation_watch_frozen(&f, &recipient);
        let mut quiet = false;
        for _ in 0..3 {
            let output = f.store.presentation_pull(input.clone(), &budget()).unwrap();
            quiet = match output.reply {
                DeliveryPullReplyV2::Unchanged { .. } => true,
                DeliveryPullReplyV2::Page(page) => !page.more,
                other => panic!("{other:?}"),
            };
            input.ordinary_due_snapshot = output.ordinary_due_update;
            if quiet {
                break;
            }
        }
        assert!(quiet);
        assert_eq!(
            f.db.query_row(
                "SELECT count(*) FROM invitations WHERE seat_id='s' AND state='pending'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(input.ordinary_due_snapshot.unwrap().stamps.len(), count);
    }
}

// Kills: trusting malformed bounded hints, and persisting negative progress on
// rollback/refusal. A changed render/canonical signal must retry the finite set.
#[test]
fn presentation_capacity_snapshot_validation_scope_and_failed_calls() {
    use crate::protocol::delivery::DeliveryPullReplyV2;
    let f = fixture(0);
    let recipient = pull_seat(&f);
    let sibling = second_binding(&f);
    let foreign_binding = binding_for_claim(&f, &sibling);
    let input = capacity_missing_input(&f, &recipient, 256);
    for mutation in 0..7 {
        let mut bad = input.clone();
        match mutation {
            0 => bad.known_messages[1] = bad.known_messages[0].clone(),
            1 => bad.blocked_messages.push(MessageId::new("not-known")),
            2 => {
                bad.ordinary_due_snapshot
                    .as_mut()
                    .unwrap()
                    .stamps
                    .pop_first();
            }
            3 => bad.ordinary_due_snapshot.as_mut().unwrap().generation += 1,
            4 => bad.ordinary_due_snapshot.as_mut().unwrap().binding_id = foreign_binding.clone(),
            5 => {
                bad.ordinary_due_snapshot.as_mut().unwrap().cache.deferred =
                    bad.ordinary_due_snapshot.as_ref().unwrap().stamps.clone()
            }
            _ => {
                bad.ordinary_due_snapshot
                    .as_mut()
                    .unwrap()
                    .stamps
                    .values_mut()
                    .next()
                    .unwrap()
                    .count = 0
            }
        }
        assert_eq!(
            f.store.presentation_pull(bad, &budget()).unwrap_err().code,
            crate::protocol::results::ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        f.db.query_row(
            "SELECT count(*) FROM invitation_presentation_consumers",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let mut input = input;
    for expected in [100, 200, 256] {
        let output = f.store.presentation_pull(input.clone(), &budget()).unwrap();
        assert_eq!(
            output
                .ordinary_due_update
                .as_ref()
                .unwrap()
                .cache
                .deferred
                .len(),
            expected
        );
        input.ordinary_due_snapshot = output.ordinary_due_update;
    }
    let before = input.ordinary_due_snapshot.clone();
    let mut refused = input.clone();
    refused.pull.claim = fresh_execution(recipient.clone(), 2);
    let refused = f.store.presentation_pull(refused, &budget()).unwrap();
    assert!(matches!(refused.reply, DeliveryPullReplyV2::Refused { .. }));
    assert!(refused.ordinary_due_update.is_none());
    input.pull.marker_prefix = vec!["different-render".into()];
    let cancelled = budget();
    cancelled.cancellation.cancel();
    assert!(
        f.store
            .presentation_pull(input.clone(), &cancelled)
            .is_err()
    );
    assert_eq!(input.ordinary_due_snapshot, before);
    let old_render: String = f.db.query_row("SELECT render_key FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='watch'",[],|r|r.get(0)).unwrap();
    f.db.execute_batch("CREATE TRIGGER capacity_test_save_failure BEFORE UPDATE ON invitation_presentation_consumers BEGIN SELECT RAISE(ABORT,'capacity test rollback'); END;").unwrap();
    assert!(
        f.store.presentation_pull(input.clone(), &budget()).is_err(),
        "save failure after due work cannot return cache progress"
    );
    f.db.execute_batch("DROP TRIGGER capacity_test_save_failure;")
        .unwrap();
    assert_eq!(f.db.query_row("SELECT render_key FROM invitation_presentation_consumers WHERE seat_id='s' AND consumer='watch'",[],|r|r.get::<_,String>(0)).unwrap(),old_render);
    assert_eq!(input.ordinary_due_snapshot, before);
    let retried = f.store.presentation_pull(input.clone(), &budget()).unwrap();
    assert_eq!(retried.work.ordinary, 100);
    assert_eq!(
        retried
            .ordinary_due_update
            .as_ref()
            .unwrap()
            .cache
            .deferred
            .len(),
        100
    );
    input.ordinary_due_snapshot = retried.ordinary_due_update;
    input.ordinary_version += 1;
    let signal = f.store.presentation_pull(input.clone(), &budget()).unwrap();
    assert_eq!(signal.work.ordinary, 100);
    assert_eq!(
        signal
            .ordinary_due_update
            .as_ref()
            .unwrap()
            .cache
            .deferred
            .len(),
        100
    );
    input.ordinary_due_snapshot = signal.ordinary_due_update;
    let old_passive = input
        .ordinary_due_snapshot
        .as_ref()
        .unwrap()
        .cache
        .scope
        .as_ref()
        .unwrap()
        .passive_generation;
    let sender = check_in(
        &f,
        fixture_claim(&f, "x", "px", fourth(), "capacity-publication"),
        "capacity-publication-ci",
    );
    f.service
        .handle(
            Command::SendMessage(crate::protocol::commands::SendMessage {
                delivery_mode: crate::protocol::commands::DeliveryMode::Lazy,
                user_intent: None,
                thread: ThreadId::new("t"),
                body: "new passive publication".into(),
                direct_recipient: None,
                invited_recipients: vec![],
                deadline_millis: None,
                operation: OperationId::new("capacity-publication-send"),
                claim: sender,
                relays_user: false,
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    let published = f.store.presentation_pull(input.clone(), &budget()).unwrap();
    let update = published.ordinary_due_update.unwrap();
    assert_eq!(published.work.ordinary, 100);
    assert_eq!(
        update.cache.deferred.len(),
        100,
        "passive publication invalidates the old negative pass at unchanged ordinary_version"
    );
    assert!(update.cache.scope.as_ref().unwrap().passive_generation > old_passive);
    input.ordinary_due_snapshot = Some(update);
    f.db.execute(
        "UPDATE seats SET state='retired',retired_at=1000000,retired_seq=100 WHERE id='s'",
        [],
    )
    .unwrap();
    let retired = f.store.presentation_pull(input, &budget()).unwrap();
    assert!(matches!(retired.reply, DeliveryPullReplyV2::Refused { .. }));
    assert!(retired.ordinary_due_update.is_none());
    assert_eq!(
        (
            retired.work.ordinary,
            retired.work.ordinary_due_key_probes,
            retired.work.ordinary_due_seeks
        ),
        (0, 0, 0)
    );
}

// Kills: due hydration losing lazy promotion or marker provenance, and a
// standalone unrenderable key endlessly requesting continuation at one render.
#[test]
fn presentation_capacity_due_promoted_lazy_defers_oversize_render_then_retries() {
    use crate::protocol::{
        commands::DeliveryMode,
        delivery::{DeliveryPullReplyV2, PresentationItemV2},
    };
    let window = OfferWindowFixture::new(true);
    window
        .f
        .db
        .execute("UPDATE seats SET wake_on='all' WHERE id='c'", [])
        .unwrap();
    let body = format!("{}é", "\0".repeat(65534));
    let id = window.send(0, DeliveryMode::Lazy, &body);
    assert_eq!(
        window.drain(1),
        std::collections::BTreeSet::from([id.clone()])
    );
    let original_offer = window.offers()[&id];
    advance(&window.f, REOFFER_AFTER_MS);
    let mut input = capacity_missing_input(&window.f, &window.recipient, 256);
    input.known_messages.pop();
    input.known_messages.push(id.clone());
    let snapshot = input.ordinary_due_snapshot.as_mut().unwrap();
    snapshot.stamps.pop_last();
    snapshot.stamps.insert(id.clone(), original_offer);
    input.pull.marker_prefix = vec!["\0".repeat(512); 8];
    let mut deferred = false;
    for _ in 0..4 {
        let output = window
            .f
            .store
            .presentation_pull(input.clone(), &budget())
            .unwrap();
        assert!(output.work.ordinary <= 100 && output.work.ordinary_due_hydrated <= 32);
        assert!(
            output.work.ordinary_due_key_probes <= 1303 && output.work.ordinary_due_seeks <= 100
        );
        assert!(output.work.output_passes <= 352);
        if let DeliveryPullReplyV2::Page(page) = &output.reply {
            assert_capacity_watch_page_bound(page);
            assert!(page.offered.is_empty() && page.truncated.is_empty());
            window.confirm_nonmessage_presentation(&output.reply);
        }
        input.ordinary_due_snapshot = output.ordinary_due_update;
        input.frozen_render = presentation_watch_frozen(&window.f, &window.recipient);
        if input
            .ordinary_due_snapshot
            .as_ref()
            .unwrap()
            .cache
            .deferred
            .len()
            == 256
        {
            deferred = true;
            break;
        }
    }
    assert!(
        deferred,
        "standalone rendered >64KiB key must be finitely deferred"
    );
    let quiet = window
        .f
        .store
        .presentation_pull(input.clone(), &budget())
        .unwrap();
    assert!(matches!(quiet.reply, DeliveryPullReplyV2::Unchanged { .. }));
    assert_eq!(quiet.work.ordinary, 0);
    assert_eq!(window.offers()[&id], original_offer);
    input.pull.marker_prefix.clear();
    let mut shown = false;
    for _ in 0..4 {
        let output = window
            .f
            .store
            .presentation_pull(input.clone(), &budget())
            .unwrap();
        assert!(output.work.ordinary <= 100 && output.work.ordinary_due_hydrated <= 32);
        assert!(
            output.work.ordinary_due_key_probes <= 1303 && output.work.ordinary_due_seeks <= 100
        );
        assert!(output.work.output_passes <= 352);
        if let DeliveryPullReplyV2::Page(page) = &output.reply {
            assert_capacity_watch_page_bound(page);
            for item in &page.items {
                if let PresentationItemV2::Message {
                    id: actual,
                    lazy,
                    content,
                } = item
                {
                    assert_eq!(actual, &id);
                    assert!(
                        !lazy,
                        "wake-on-all promotion survives canonical due hydration"
                    );
                    assert!(!content.ack_required, "promotion creates no receipt");
                    assert!(content.truncated);
                    assert_eq!(content.body_len, 65536);
                    assert_eq!(
                        content.body.matches("herdr-threads body").count(),
                        1,
                        "one producer marker"
                    );
                    assert!(!content.body.contains("herdr-threads ack"));
                    shown = true;
                }
            }
            window.confirm_nonmessage_presentation(&output.reply);
        }
        input.ordinary_due_snapshot = output.ordinary_due_update;
        input.frozen_render = presentation_watch_frozen(&window.f, &window.recipient);
        if shown {
            break;
        }
    }
    assert!(shown);
    assert_eq!(
        window
            .f
            .db
            .query_row(
                "SELECT state FROM lazy_recipients WHERE message_id=?1 AND seat_id='c'",
                [id.as_str()],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "pending"
    );
    assert_eq!(
        window.offers()[&id],
        original_offer,
        "direct store output cannot claim service recording or lazy completion"
    );
}

// Kills: zero-capacity due traversal resetting or reserializing a partial normal
// body, or failing to resume that exact body in the call that reclaims a slot.
#[test]
fn presentation_capacity_due_walk_preserves_partial_body_until_real_ack() {
    use crate::protocol::{commands::DeliveryMode, delivery::DeliveryPullReplyV2};
    for lazy in [false, true] {
        let window = OfferWindowFixture::new(true);
        for _ in 0..4 {
            window.pull();
        }
        {
            let mut ledger = window.f.state.lock();
            let entry = ledger
                .binding_mut(
                    &window.recipient.seat,
                    &binding_for_claim(&window.f, &window.recipient),
                    window.recipient.binding_generation,
                )
                .unwrap();
            for n in 0..255 {
                entry.record(
                    &MessageId::new(format!("absent-{n:03}")),
                    window.f.clock.utc_now(),
                    false,
                );
            }
        }
        let mode = if lazy {
            DeliveryMode::Lazy
        } else {
            DeliveryMode::Ordinary
        };
        let first = window.send(0, mode, &"a".repeat(58000));
        let second = window.send(1, mode, &"é".repeat(32768));
        let mut allocated = false;
        for _ in 0..4 {
            if let DeliveryPullReplyV2::Page(page) = window.pull()
                && page.truncated.contains(&first)
            {
                allocated = true;
                break;
            }
        }
        assert!(allocated);
        let partial: String = window.f.db.query_row("SELECT ordinary_partial FROM invitation_presentation_consumers WHERE seat_id='c' AND consumer='watch'",[],|r|r.get(0)).unwrap();
        let partial: std::collections::HashMap<String, String> =
            serde_json::from_str(&partial).unwrap();
        assert!(
            partial
                .get(second.as_str())
                .is_some_and(|body| !body.is_empty()),
            "fixture must retain a real source-created partial body"
        );
        let parked = capacity_normal_state(&window);
        advance(&window.f, REOFFER_AFTER_MS);
        let mut reoffered = false;
        for _ in 0..4 {
            if let DeliveryPullReplyV2::Page(page) = window.pull() {
                reoffered |= page.truncated.contains(&first);
            }
            assert_eq!(capacity_normal_state(&window), parked);
        }
        assert!(reoffered);
        let retained = window.offers();
        let cursor = reclamation_cursor(&window.f, &window.recipient);
        if !lazy {
            window.ack(
                std::slice::from_ref(&first),
                false,
                false,
                "partial-release",
            );
            window.pull();
            assert_eq!(reclamation_cursor(&window.f, &window.recipient), cursor);
            assert_eq!(
                window.offers(),
                retained,
                "manual ACK retains unread full stamps"
            );
            assert_eq!(capacity_normal_state(&window), parked);
        }
        assert_eq!(
            reclamation_read_full_body(&window.f, &first),
            "a".repeat(58000)
        );
        assert_eq!(
            reclamation_cursor(&window.f, &window.recipient),
            cursor,
            "read-only body RPC does not claim display by itself"
        );
        assert_eq!(window.offers(), retained);
        if lazy {
            window
                .f
                .service
                .handle(
                    Command::CompleteInboxDelivery(
                        crate::protocol::commands::CompleteInboxDelivery {
                            messages: vec![first.clone()],
                            operation: OperationId::new("partial-lazy-completion"),
                            claim: window.recipient.clone(),
                            via: None,
                        },
                    ),
                    peer(),
                    &budget(),
                )
                .unwrap();
            assert_eq!(
                window
                    .f
                    .db
                    .query_row(
                        "SELECT state FROM lazy_recipients WHERE message_id=?1 AND seat_id='c'",
                        [first.as_str()],
                        |r| r.get::<_, String>(0)
                    )
                    .unwrap(),
                "displayed"
            );
        } else {
            reclamation_advance_displayed(
                &window.f,
                &window.recipient,
                std::slice::from_ref(&first),
                "partial-first-full-display",
            );
        }
        assert!(reclamation_cursor(&window.f, &window.recipient) > cursor);
        let DeliveryPullReplyV2::Page(page) = window.pull() else {
            panic!("same-call partial resume");
        };
        assert!(page.truncated.contains(&second));
        let work = window
            .f
            .service
            .presentation
            .last_work
            .lock()
            .unwrap()
            .unwrap();
        assert_eq!(
            work.ordinary_due_key_probes, 0,
            "one ordinary source per call"
        );
        assert!(work.ordinary > 0);
    }
}

// Kills: globally suppressing continuation at capacity and starving independent
// passive invitation/context/notice work, or treating confirmation as acceptance.
#[test]
fn presentation_capacity_parked_ordinary_still_drains_passive_context_and_notices() {
    use crate::protocol::{
        commands::SetTopic,
        delivery::{DeliveryPullReplyV2, PresentationItemV2},
    };
    let window = OfferWindowFixture::new(false);
    for _ in 0..4 {
        window.pull();
    }
    {
        let mut ledger = window.f.state.lock();
        let entry = ledger
            .binding_mut(
                &window.recipient.seat,
                &binding_for_claim(&window.f, &window.recipient),
                window.recipient.binding_generation,
            )
            .unwrap();
        for n in 0..256 {
            entry.record(
                &MessageId::new(format!("absent-{n:03}")),
                window.f.clock.utc_now(),
                false,
            );
        }
    }
    let original = window.offers();
    let thread = canonical_lazy_invitation_backlog(
        &window.f,
        &window.recipient,
        &window.sender,
        1,
        "parked-passive",
    );
    service_notice(&window.f, "capacity-notice");
    let (mut invitation, mut notice, mut quiet) = (false, false, false);
    for _ in 0..8 {
        match window.pull() {
            DeliveryPullReplyV2::Page(page) => {
                for item in &page.items {
                    match item {
                        PresentationItemV2::Invitation { .. } => invitation = true,
                        PresentationItemV2::Notice { warning, .. }
                            if warning.as_str() == "capacity-notice" =>
                        {
                            notice = true
                        }
                        _ => {}
                    }
                }
                assert!(page.offered.is_empty() && page.truncated.is_empty());
                quiet = !page.more;
            }
            DeliveryPullReplyV2::Unchanged { .. } => quiet = true,
            other => panic!("{other:?}"),
        }
        assert_eq!(window.offers(), original);
        if quiet {
            break;
        }
    }
    assert!(invitation && notice && quiet);
    window
        .f
        .service
        .handle(
            Command::SetTopic(SetTopic {
                thread,
                topic: "updated capacity context".into(),
                operation: OperationId::new("capacity-context-update"),
                claim: window.sender.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    let mut changed = false;
    for _ in 0..4 {
        if let DeliveryPullReplyV2::Page(page) = window.pull() {
            changed |= page.text.contains("updated capacity context");
        }
        if changed {
            break;
        }
    }
    assert!(changed);
    assert_eq!(window.offers(), original);
    assert_eq!(
        window
            .f
            .db
            .query_row(
                "SELECT count(*) FROM invitations WHERE seat_id='s' AND state='pending'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}

// Kills CIR-1: a valid concurrently accumulated count above the policy reoffer
// threshold refusing every V2 pull, including independent passive work.
#[test]
fn presentation_capacity_retained_concurrent_offer_count_does_not_refuse_passive_work() {
    use crate::protocol::delivery::{DeliveryPullReplyV2, PresentationItemV2};
    let window = OfferWindowFixture::new(false);
    for _ in 0..4 {
        window.pull();
    }
    let known = window.send(
        0,
        crate::protocol::commands::DeliveryMode::Ordinary,
        "concurrent body",
    );
    {
        let mut ledger = window.f.state.lock();
        let entry = ledger
            .binding_mut(
                &window.recipient.seat,
                &binding_for_claim(&window.f, &window.recipient),
                window.recipient.binding_generation,
            )
            .unwrap();
        // Both routes can snapshot count two, then finish their recordings.
        for _ in 0..2 {
            entry.record(&known, window.f.clock.utc_now(), false);
        }
    }
    advance(&window.f, REOFFER_AFTER_MS);
    let race_claim = window.recipient.clone();
    let race_binding = binding_for_claim(&window.f, &race_claim);
    let race_id = known.clone();
    let race_now = window.f.clock.utc_now();
    *window.f.state.probe.before_record.lock().unwrap() =
        Some(Box::new(move |ledger: &mut OfferedLedger| {
            let entry = ledger
                .binding_mut(
                    &race_claim.seat,
                    &race_binding,
                    race_claim.binding_generation,
                )
                .unwrap();
            assert_eq!(
                entry.offers[&race_id].count, 2,
                "V1 selected the due second stamp"
            );
            entry.record(&race_id, race_now, false); // another route finishes first
        }));
    let selected = page(pull(&window.f, &window.recipient, None));
    assert!(selected.offered.contains(&known));
    let original = window.offers();
    assert_eq!(
        original[&known].count, 4,
        "actual V1 recording preserves the interleaving"
    );
    service_notice(&window.f, "concurrent-count-notice");
    let mut delivered = false;
    for _ in 0..4 {
        if let DeliveryPullReplyV2::Page(page) = window.pull() {
            delivered |= page.items.iter().any(|item|matches!(item,PresentationItemV2::Notice{warning,..} if warning.as_str()=="concurrent-count-notice"));
            assert!(page.offered.is_empty() && page.truncated.is_empty());
        }
        assert_eq!(window.offers(), original);
        if delivered {
            break;
        }
    }
    assert!(delivered);
    assert_eq!(original[&known].count, 4);
    let cursor = reclamation_cursor(&window.f, &window.recipient);
    window.ack(
        std::slice::from_ref(&known),
        false,
        false,
        "concurrent-history-ack",
    );
    window.pull();
    assert_eq!(reclamation_cursor(&window.f, &window.recipient), cursor);
    assert_eq!(
        window.offers(),
        original,
        "ACK alone preserves the complete count-four stamp"
    );
    assert!(
        !original[&known].truncated,
        "V1 fully emitted the short body"
    );
    reclamation_advance_displayed(
        &window.f,
        &window.recipient,
        std::slice::from_ref(&known),
        "concurrent-history-displayed",
    );
    window.pull();
    assert!(
        !window.offers().contains_key(&known),
        "V2 canonical reclamation remains available"
    );
}

// Kills: tying a negative cache to durable consumer state or waiting for client
// acknowledgement before remembering a successful miss traversal.
#[test]
fn presentation_capacity_cache_progress_survives_lost_reply_but_not_ledger_restart() {
    use crate::protocol::{commands::DeliveryMode, delivery::DeliveryPullReplyV2};
    let mut window = OfferWindowFixture::new(false);
    for _ in 0..4 {
        window.pull();
    }
    {
        let mut ledger = window.f.state.lock();
        let entry = ledger
            .binding_mut(
                &window.recipient.seat,
                &binding_for_claim(&window.f, &window.recipient),
                window.recipient.binding_generation,
            )
            .unwrap();
        for n in 0..256 {
            entry.record(
                &MessageId::new(format!("absent-{n:03}")),
                window.f.clock.utc_now(),
                false,
            );
        }
    }
    let parked = window.send(0, DeliveryMode::Lazy, "survives process-local cache");
    for _ in 0..4 {
        window.pull();
    }
    advance(&window.f, REOFFER_AFTER_MS);
    // Model response loss after successful service completion: no client consumes
    // the first returned page, yet the next call must pass its100 attempted IDs.
    drop(window.pull());
    assert_eq!(
        window
            .f
            .service
            .presentation
            .last_work
            .lock()
            .unwrap()
            .unwrap()
            .ordinary,
        100
    );
    drop(window.pull());
    assert_eq!(
        window
            .f
            .service
            .presentation
            .last_work
            .lock()
            .unwrap()
            .unwrap()
            .ordinary,
        100
    );
    assert!(matches!(
        window.pull(),
        DeliveryPullReplyV2::Unchanged { .. }
    ));
    assert_eq!(
        window
            .f
            .service
            .presentation
            .last_work
            .lock()
            .unwrap()
            .unwrap()
            .ordinary,
        56
    );
    restart(&mut window.f);
    let DeliveryPullReplyV2::Page(page) = window.pull() else {
        panic!("new process has ordinary capacity");
    };
    assert!(page.offered.contains(&parked));
    assert_eq!(window.offers().len(), 1);
    assert_eq!(
        window
            .f
            .db
            .query_row(
                "SELECT state FROM lazy_recipients WHERE message_id=?1 AND seat_id='s'",
                [parked.as_str()],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "pending"
    );
}

// Kills: negative-caching a renderable due key merely because another class
// consumed this page's bytes, instead of retrying its standalone opportunity.
#[test]
fn presentation_capacity_due_size_failure_after_passive_output_retries_standalone() {
    use crate::protocol::{
        commands::DeliveryMode,
        delivery::{DeliveryPullReplyV2, PresentationItemV2},
    };
    let window = OfferWindowFixture::new(true);
    let id = window.send(0, DeliveryMode::Lazy, &"\0".repeat(65536));
    assert_eq!(
        window.drain(1),
        std::collections::BTreeSet::from([id.clone()])
    );
    advance(&window.f, REOFFER_AFTER_MS);
    let thread = canonical_lazy_invitation_backlog(
        &window.f,
        &window.recipient,
        &window.sender,
        1,
        "due-size-retry",
    );
    window
        .f
        .db
        .execute(
            "UPDATE threads SET topic=?1,goal=?1 WHERE id=?2",
            params!["\0".repeat(1024), thread.as_str()],
        )
        .unwrap();
    window.f.db.execute("UPDATE invitation_presentation_consumers SET rotation=2 WHERE seat_id='c' AND consumer='watch'",[]).unwrap();
    let mut input = capacity_missing_input(&window.f, &window.recipient, 256);
    input.known_messages.pop();
    input.blocked_messages = input.known_messages.clone();
    input.known_messages.push(id.clone());
    let snapshot = input.ordinary_due_snapshot.as_mut().unwrap();
    snapshot.stamps.pop_last();
    snapshot.stamps.insert(id.clone(), window.offers()[&id]);
    input.pull.marker_prefix = vec!["\0".repeat(1024)];
    let first = window
        .f
        .store
        .presentation_pull(input.clone(), &budget())
        .unwrap();
    let DeliveryPullReplyV2::Page(page) = &first.reply else {
        panic!("earlier passive class output");
    };
    assert_capacity_watch_page_bound(page);
    assert!(
        page.items
            .iter()
            .any(|item| matches!(item, PresentationItemV2::Invitation { .. }))
    );
    assert!(page.offered.is_empty() && page.truncated.is_empty());
    assert!(page.more, "due key still runnable after earlier output");
    assert_eq!(first.work.ordinary_due_hydrated, 1);
    assert!(
        !first
            .ordinary_due_update
            .as_ref()
            .unwrap()
            .cache
            .deferred
            .contains_key(&id)
    );
    window.confirm_nonmessage_presentation(&first.reply);
    input.ordinary_due_snapshot = first.ordinary_due_update;
    input.frozen_render = presentation_watch_frozen(&window.f, &window.recipient);
    let next = window.f.store.presentation_pull(input, &budget()).unwrap();
    let DeliveryPullReplyV2::Page(page) = next.reply else {
        panic!("standalone due opportunity");
    };
    assert_capacity_watch_page_bound(&page);
    assert_eq!(page.truncated, vec![id]);
    assert!(!page.more);
    assert_eq!(next.work.ordinary_due_hydrated, 1);
}

// Kills physical recovery recording a logical offer, refilling on signal/render,
// positive-capacity omission, and body/marker/receipt regressions.
#[test]
fn presentation_owner_replay_nonfull_once_per_lease_preserves_physical_metadata() {
    use crate::protocol::{
        commands::DeliveryMode,
        delivery::{DeliveryPullReplyV2, PresentationItemV2},
    };
    for (mode, promoted, large) in [
        (DeliveryMode::Ordinary, false, false),
        (DeliveryMode::Ordinary, false, true),
        (DeliveryMode::Lazy, false, false),
        (DeliveryMode::Lazy, true, true),
    ] {
        let mut window = OfferWindowFixture::new(true);
        if promoted {
            window
                .f
                .db
                .execute("UPDATE seats SET wake_on='all' WHERE id='c'", [])
                .unwrap();
        }
        let body = if large {
            "body".repeat(4096)
        } else {
            "whole body".into()
        };
        let id = window.send(0, mode, &body);
        window.drain(1);
        for _ in 0..4 {
            window.pull();
        }
        let stamps = window.offers();
        let walk = capacity_normal_state(&window);
        let old_lease = window.replace_watch();
        let mut stale = window.watch_request();
        stale.presentation_lease = Some(old_lease);
        assert!(matches!(
            window
                .f
                .service
                .handle(Command::DeliveryPullV2(stale), peer(), &budget())
                .unwrap(),
            CommandResult::DeliveryPulledV2(DeliveryPullReplyV2::Refused { .. })
        ));
        let DeliveryPullReplyV2::Page(page) = window.raw_watch_pull() else {
            panic!("immediate owner replay")
        };
        assert_capacity_watch_page_bound(&page);
        assert_eq!(page.offered.contains(&id), !large);
        assert_eq!(page.truncated.contains(&id), large);
        let message = page
            .items
            .iter()
            .find_map(|item| match item {
                PresentationItemV2::Message {
                    id: actual,
                    lazy,
                    content,
                } if actual == &id => Some((*lazy, content)),
                _ => None,
            })
            .unwrap();
        assert_eq!(message.0, mode == DeliveryMode::Lazy && !promoted);
        assert_eq!(message.1.truncated, large);
        assert_eq!(
            message.1.body.matches("herdr-threads body").count(),
            usize::from(large)
        );
        if mode == DeliveryMode::Lazy {
            assert!(!message.1.ack_required);
            assert!(!message.1.body.contains("herdr-threads ack"));
            assert_eq!(
                window
                    .f
                    .db
                    .query_row(
                        "SELECT state FROM lazy_recipients WHERE message_id=?1 AND seat_id='c'",
                        [id.as_str()],
                        |r| r.get::<_, String>(0)
                    )
                    .unwrap(),
                "pending"
            );
        }
        assert_eq!(window.offers(), stamps);
        assert_eq!(capacity_normal_state(&window), walk);
        window.confirm_nonmessage_presentation(&DeliveryPullReplyV2::Page(page));
        // Successful response loss consumes only this lease's physical attempt.
        for n in 0..4 {
            let mut request = window.watch_request();
            request.marker_prefix = vec![format!("prefix-{n}")];
            let CommandResult::DeliveryPulledV2(reply) = window
                .f
                .service
                .handle(Command::DeliveryPullV2(request), peer(), &budget())
                .unwrap()
            else {
                panic!("V2 response")
            };
            if let DeliveryPullReplyV2::Page(page) = &reply {
                assert!(page.offered.is_empty() && page.truncated.is_empty());
            }
            window.confirm_nonmessage_presentation(&reply);
        }
        assert_eq!(window.offers(), stamps);
        window.replace_watch();
        let DeliveryPullReplyV2::Page(again) = window.pull() else {
            panic!("new lease recovers lost output")
        };
        assert!(again.offered.contains(&id) || again.truncated.contains(&id));
        assert_eq!(window.offers(), stamps);
    }
}

// Kills zero-capacity omission, combined physical consumed IDs (WRD-2), normal
// walk reset/carry consumption, and physical offer recording.
#[test]
fn presentation_owner_replay_full_final_page_keeps_logical_due_continuation() {
    use crate::protocol::delivery::DeliveryPullReplyV2;
    let (mut window, sent) = full_pending_offer_window_with_fresh_carry();
    for _ in 0..4 {
        window.pull();
    }
    let due = window.offers().keys().last().unwrap().clone();
    {
        let mut ledger = window.f.state.lock();
        let entry = ledger
            .binding_mut(
                &window.recipient.seat,
                &binding_for_claim(&window.f, &window.recipient),
                window.recipient.binding_generation,
            )
            .unwrap();
        // Actual supported recorder reaches parked three and one due two.
        let ids = entry.offers.keys().cloned().collect::<Vec<_>>();
        for id in ids {
            entry.record(&id, window.f.clock.utc_now(), entry.offers[&id].truncated);
            if id != due {
                entry.record(&id, window.f.clock.utc_now(), entry.offers[&id].truncated);
            }
        }
    }
    advance(&window.f, REOFFER_AFTER_MS);
    let stamps = window.offers();
    let walk = capacity_normal_state(&window);
    window.replace_watch();
    let mut shown = std::collections::BTreeSet::new();
    for _ in 0..32 {
        let DeliveryPullReplyV2::Page(page) = window.pull() else {
            panic!("owner recovery must continue")
        };
        assert_capacity_watch_page_bound(&page);
        for id in page.offered.iter().chain(&page.truncated) {
            assert!(shown.insert(id.clone()));
        }
        assert_eq!(window.offers(), stamps);
        assert_eq!(capacity_normal_state(&window), walk);
        assert_full_pending_offer_work(&window);
        assert!(
            page.more,
            "final physical page leaves the unchanged logical due key runnable"
        );
        if shown.len() == 256 {
            assert!(
                page.offered.contains(&due) || page.truncated.contains(&due),
                "due key sorts onto final physical page"
            );
            break;
        }
    }
    assert_eq!(shown.len(), 256);
    assert!(!shown.contains(&sent[256]));
    let DeliveryPullReplyV2::Page(logical) = window.pull() else {
        panic!("logical third offer follows owner completion")
    };
    assert_eq!(
        logical
            .offered
            .iter()
            .chain(&logical.truncated)
            .cloned()
            .collect::<Vec<_>>(),
        vec![due.clone()]
    );
    let after = window.offers();
    for (id, stamp) in &stamps {
        if id == &due {
            assert_eq!(after[id].count, 3);
        } else {
            assert_eq!(&after[id], stamp);
        }
    }
    assert_eq!(capacity_normal_state(&window), walk);
    let cursor = reclamation_cursor(&window.f, &window.recipient);
    window.ack(
        std::slice::from_ref(&due),
        false,
        false,
        "owner-full-resume",
    );
    window.pull();
    assert_eq!(reclamation_cursor(&window.f, &window.recipient), cursor);
    assert_eq!(
        window.offers(),
        after,
        "manual ACK retains all 256 unread stamps"
    );
    assert_eq!(capacity_normal_state(&window), walk);
    // The chosen due key is ordered by ID, not thread sequence. Its display
    // must include the actual prefix and the first body's full continuation.
    assert_eq!(
        reclamation_read_full_body(&window.f, &sent[0]),
        "x".repeat(WATCH_BODY_LIMIT_BYTES + 1)
    );
    let due_position = sent[..256].iter().position(|id| id == &due).unwrap();
    assert!(
        sent[..=due_position]
            .iter()
            .skip(1)
            .all(|id| !after[id].truncated)
    );
    reclamation_advance_displayed(
        &window.f,
        &window.recipient,
        &sent[..=due_position],
        "owner-full-displayed-prefix",
    );
    let DeliveryPullReplyV2::Page(resumed) = window.pull() else {
        panic!("full display and ACK resume carried unknown257")
    };
    assert!(resumed.offered.contains(&sent[256]));
    let resumed_stamps = window.offers();
    for (id, stamp) in &after {
        if id != &due {
            assert_eq!(
                &resumed_stamps[id], stamp,
                "unsettled prefix keeps its full stamp"
            );
        }
    }
    assert_eq!(window.f.db.query_row::<i64, _, _>(
        "SELECT count(*) FROM prepared_recipients pr JOIN send_manifests sm ON sm.preparation_id=pr.preparation_id LEFT JOIN receipt_state rs ON rs.message_id=sm.message_id AND rs.seat_id=pr.seat_id WHERE pr.seat_id='c' AND COALESCE(rs.state,'pending')='pending' AND pr.ack_required=1",
        [], |row| row.get(0),
    ).unwrap(), 256, "display settles no other receipt and never displays unknown257 early");
}

fn owner_outlook(window: &OfferWindowFixture) -> (bool, Option<u64>, Option<u64>, NextPull) {
    let ledger = window.f.state.lock();
    let entry = ledger
        .get(&binding_for_claim(&window.f, &window.recipient))
        .unwrap();
    (
        entry.more_pending,
        entry.read_attention,
        entry.whole_read,
        entry.next_pull(777, window.f.clock.utc_now()),
    )
}

// Kills filtered-false assignment and loss of the empty/final recovery disposition.
#[test]
fn presentation_owner_replay_missing_quanta_preserve_current_native_outlook() {
    use crate::protocol::delivery::DeliveryPullReplyV2;
    for more in [false, true] {
        let mut window = OfferWindowFixture::new(true);
        for _ in 0..4 {
            window.pull();
        }
        {
            let mut ledger = window.f.state.lock();
            let entry = ledger
                .binding_mut(
                    &window.recipient.seat,
                    &binding_for_claim(&window.f, &window.recipient),
                    window.recipient.binding_generation,
                )
                .unwrap();
            for n in 0..256 {
                entry.record(
                    &MessageId::new(format!("missing-owner-{n:03}")),
                    window.f.clock.utc_now(),
                    false,
                );
            }
            entry.more_pending = more;
            entry.read_attention = Some(777);
            entry.whole_read = Some(888);
        }
        let stamps = window.offers();
        let outlook = owner_outlook(&window);
        let walk = capacity_normal_state(&window);
        window.replace_watch();
        for (n, attempts) in [100, 100, 56].into_iter().enumerate() {
            let reply = window.raw_watch_pull();
            assert_eq!(owner_outlook(&window), outlook, "empty quantum {n}");
            assert_eq!(window.offers(), stamps, "missing is not terminal authority");
            assert_eq!(capacity_normal_state(&window), walk);
            let work = window
                .f
                .service
                .presentation
                .last_work
                .lock()
                .unwrap()
                .unwrap();
            assert_eq!(work.ordinary, attempts);
            assert_eq!(work.ordinary_due_hydrated, 0);
            assert!(work.ordinary_due_key_probes <= 1303 && work.ordinary_due_seeks <= 100);
            if n < 2 {
                assert!(
                    matches!(reply, DeliveryPullReplyV2::Page(ref page) if page.more && page.items.is_empty())
                );
            } else {
                assert!(matches!(reply, DeliveryPullReplyV2::Unchanged { .. }));
            }
        }
        assert!(matches!(
            window.raw_watch_pull(),
            DeliveryPullReplyV2::Unchanged { .. }
        ));
        assert_eq!(
            window
                .f
                .service
                .presentation
                .last_work
                .lock()
                .unwrap()
                .unwrap()
                .ordinary,
            0
        );
    }
}

// Kills replacing/retiring the old nonordinary outbox to unlock recovery,
// hydrating ordinary bodies on outbox replay, and clearing native outlook there.
#[test]
fn presentation_owner_replay_waits_for_original_outbox_then_continues() {
    use crate::protocol::{commands::DeliveryMode, delivery::DeliveryPullReplyV2};
    for more in [false, true] {
        let mut window = OfferWindowFixture::new(true);
        for _ in 0..4 {
            window.pull();
        }
        let id = window.send(0, DeliveryMode::Ordinary, "pending across outbox");
        canonical_lazy_invitation_backlog(
            &window.f,
            &window.recipient,
            &window.sender,
            1,
            "owner-outbox",
        );
        let first = window.raw_watch_pull();
        let DeliveryPullReplyV2::Page(page) = &first else {
            panic!("mixed original output")
        };
        assert!(page.offered.contains(&id));
        let token = page
            .presentation_token
            .clone()
            .expect("real nonordinary token");
        let before = owner_outbox_state(&window);
        let stamps = window.offers();
        {
            let mut ledger = window.f.state.lock();
            let entry = ledger
                .binding_mut(
                    &window.recipient.seat,
                    &binding_for_claim(&window.f, &window.recipient),
                    window.recipient.binding_generation,
                )
                .unwrap();
            entry.more_pending = more;
            entry.read_attention = Some(777);
            entry.whole_read = Some(888);
        }
        let outlook = owner_outlook(&window);
        window.replace_watch();
        let replay = window.raw_watch_pull();
        let DeliveryPullReplyV2::Page(page) = &replay else {
            panic!("old outbox recovery")
        };
        assert_eq!(page.presentation_token.as_ref(), Some(&token));
        assert!(page.more && page.offered.is_empty() && page.truncated.is_empty());
        assert_eq!(owner_outbox_state(&window), before);
        assert_eq!(owner_outlook(&window), outlook);
        assert_eq!(
            window
                .f
                .service
                .presentation
                .last_work
                .lock()
                .unwrap()
                .unwrap()
                .ordinary,
            0
        );
        window.confirm_nonmessage_presentation(&replay);
        let DeliveryPullReplyV2::Page(next) = window.raw_watch_pull() else {
            panic!("advertised immediate ordinary continuation")
        };
        assert!(next.offered.contains(&id));
        assert_eq!(window.offers(), stamps);
        assert_eq!(owner_outlook(&window), outlook);
    }
}

fn owner_outbox_state(window: &OfferWindowFixture) -> String {
    window.f.db.query_row("SELECT json_array(outbox_token,outbox_descriptors,outbox_retired_mask,outbox_confirmed_mask,outbox_selection,outbox_codec_key) FROM invitation_presentation_consumers WHERE seat_id='c' AND consumer='watch'", [], |r| r.get(0)).unwrap()
}

fn owner_store_input(
    f: &Fixture,
    claim: &CallerClaim,
    registry: &dyn crate::ports::ModChannels,
    lease: &str,
) -> crate::ports::PresentationInput {
    let due = f
        .state
        .lock()
        .binding_mut(
            &claim.seat,
            &binding_for_claim(f, claim),
            claim.binding_generation,
        )
        .unwrap()
        .due_snapshot(crate::protocol::delivery::PresentationConsumer::Watch);
    let mut input = offer_reconciliation_input(f, claim, due.stamps.keys().cloned().collect());
    input.pull.presentation_lease = Some(lease.into());
    input.blocked_messages = due
        .stamps
        .iter()
        .filter(|(_, stamp)| !stamp.is_due(f.clock.utc_now()))
        .map(|(id, _)| id.clone())
        .collect();
    input.retry_messages = true;
    input.owner_replay = registry.presentation_replay_snapshot(claim, lease, &due);
    input.ordinary_due_snapshot = Some(due);
    input
}

// WR2 deliberately uses real V1 admission/recording and canonical Watch store
// allocation: current adapters expose mutually exclusive HookPull/PushChannel.
// No binding rewrite or fabricated count-four stamp is used.
#[test]
fn presentation_owner_replay_accepts_actual_v1_count_four_and_nonordinary_work() {
    use crate::{
        ports::{ModChannelRegistration, ModChannelSink, ModChannels, ModStoreReads},
        protocol::{
            commands::DeliveryMode,
            delivery::{DeliveryPullReplyV2, PresentationItemV2},
            watch::{ModDeliverySetting, WatchFrame},
        },
        service::mod_channels::ModChannelRegistry,
    };
    struct Sink;
    impl ModChannelSink for Sink {
        fn push(&self, _: WatchFrame) -> bool {
            true
        }
    }
    let window = OfferWindowFixture::new(false);
    let id = window.send(0, DeliveryMode::Ordinary, "actual concurrent V1 history");
    {
        let mut ledger = window.f.state.lock();
        let entry = ledger
            .binding_mut(
                &window.recipient.seat,
                &binding_for_claim(&window.f, &window.recipient),
                window.recipient.binding_generation,
            )
            .unwrap();
        for _ in 0..2 {
            entry.record(&id, window.f.clock.utc_now(), false);
        }
    }
    advance(&window.f, REOFFER_AFTER_MS);
    let race_claim = window.recipient.clone();
    let race_binding = binding_for_claim(&window.f, &race_claim);
    let race_id = id.clone();
    let now = window.f.clock.utc_now();
    *window.f.state.probe.before_record.lock().unwrap() = Some(Box::new(move |ledger| {
        let entry = ledger
            .binding_mut(
                &race_claim.seat,
                &race_binding,
                race_claim.binding_generation,
            )
            .unwrap();
        assert_eq!(entry.offers[&race_id].count, 2);
        entry.record(&race_id, now, false);
    }));
    assert!(
        page(pull(&window.f, &window.recipient, None))
            .offered
            .contains(&id)
    );
    let stamps = window.offers();
    assert_eq!(stamps[&id].count, 4);
    let reads: Arc<dyn ModStoreReads> = window.f.store.clone();
    let registry = ModChannelRegistry::new(
        window.f.clock.clone(),
        reads,
        "i".into(),
        Arc::new(|| {}),
        ModDeliverySetting::On,
    );
    let channel = registry
        .register(
            ModChannelRegistration {
                mode: crate::ports::ModChannelMode::PresentationV2,
                binding: binding_for_claim(&window.f, &window.recipient),
                seat: window.recipient.seat.clone(),
                binding_generation: window.recipient.binding_generation,
                native_session: window.recipient.native_session.clone(),
                harness: window.recipient.harness,
                idle_wake: crate::harness::adapter::IdleWake::Native,
                registered_at: now,
            },
            Arc::new(Sink),
        )
        .unwrap();
    let lease = registry.presentation_lease(channel).unwrap();
    service_notice(&window.f, "owner-count-four-notice");
    let input = owner_store_input(&window.f, &window.recipient, &registry, &lease);
    let output = window
        .f
        .store
        .presentation_pull(input.clone(), &budget())
        .unwrap();
    let DeliveryPullReplyV2::Page(page) = &output.reply else {
        panic!("count-four physical recovery")
    };
    assert!(page.offered.contains(&id));
    assert!(page.items.iter().any(|item| matches!(item, PresentationItemV2::Notice { warning, .. } if warning.as_str()=="owner-count-four-notice")));
    assert_eq!(output.owner_replay.emitted.get(&id), Some(&stamps[&id]));
    assert_eq!(window.offers(), stamps);
    registry.apply_presentation_replay(
        input.owner_replay.unwrap(),
        &output.owner_replay.completed,
        input.ordinary_due_snapshot.as_ref().unwrap(),
    );
    let next = owner_store_input(&window.f, &window.recipient, &registry, &lease);
    assert!(next.owner_replay.as_ref().unwrap().remaining.is_empty());
    registry.unregister(channel, now);
}

// Kills eager initialization on snapshot/refusal, reset after LRU recreation,
// stamp-only-last_at comparisons, stale lease progress and same-lease refill.
#[test]
fn presentation_owner_replay_registry_success_scope_and_incarnation_lifetime() {
    use crate::ports::ModChannels;
    use crate::protocol::delivery::PresentationConsumer;
    let mut window = OfferWindowFixture::new(true);
    let (registry, _, lease) = window.watch.as_ref().unwrap();
    let registry = registry.clone();
    let lease = lease.clone();
    let id = MessageId::new("owner-lifetime-missing");
    {
        let mut ledger = window.f.state.lock();
        ledger
            .binding_mut(
                &window.recipient.seat,
                &binding_for_claim(&window.f, &window.recipient),
                window.recipient.binding_generation,
            )
            .unwrap()
            .record(&id, window.f.clock.utc_now(), false);
    }
    let initial = owner_store_input(&window.f, &window.recipient, registry.as_ref(), &lease);
    let captured = initial.owner_replay.clone().unwrap();
    assert_eq!(captured.remaining.len(), 1);
    assert_eq!(
        owner_store_input(&window.f, &window.recipient, registry.as_ref(), &lease).owner_replay,
        Some(captured.clone()),
        "snapshot getter installs no progress"
    );
    {
        let mut ledger = window.f.state.lock();
        let entry = ledger
            .binding_mut(
                &window.recipient.seat,
                &binding_for_claim(&window.f, &window.recipient),
                window.recipient.binding_generation,
            )
            .unwrap();
        entry.record(&id, window.f.clock.utc_now(), true); // same last_at; full stamp changed
    }
    // First successful application must abandon the obsolete captured stamp.
    let current = window
        .f
        .state
        .lock()
        .binding_mut(
            &window.recipient.seat,
            &binding_for_claim(&window.f, &window.recipient),
            window.recipient.binding_generation,
        )
        .unwrap()
        .due_snapshot(PresentationConsumer::Watch);
    registry.apply_presentation_replay(captured.clone(), &Default::default(), &current);
    assert!(
        owner_store_input(&window.f, &window.recipient, registry.as_ref(), &lease)
            .owner_replay
            .unwrap()
            .remaining
            .is_empty()
    );
    // A newer registration captures the current tuple; old progress cannot touch it.
    window.replace_watch();
    let new_lease = window.watch.as_ref().unwrap().2.clone();
    let successor = owner_store_input(&window.f, &window.recipient, registry.as_ref(), &new_lease)
        .owner_replay
        .unwrap();
    registry.apply_presentation_replay(captured, &current.stamps, &current);
    assert_eq!(
        owner_store_input(&window.f, &window.recipient, registry.as_ref(), &new_lease).owner_replay,
        Some(successor.clone())
    );
    registry.apply_presentation_replay(successor, &Default::default(), &current);
    // Genuine ledger lifetime reset with unchanged canonical generation.
    *window.f.state.lock() = crate::delivery::ledger::OfferedLedger::new();
    {
        let mut ledger = window.f.state.lock();
        ledger
            .binding_mut(
                &window.recipient.seat,
                &binding_for_claim(&window.f, &window.recipient),
                window.recipient.binding_generation,
            )
            .unwrap()
            .record(&id, window.f.clock.utc_now(), false);
    }
    let recreated = owner_store_input(&window.f, &window.recipient, registry.as_ref(), &new_lease);
    assert!(
        recreated
            .owner_replay
            .as_ref()
            .unwrap()
            .remaining
            .is_empty()
    );
    registry.apply_presentation_replay(
        recreated.owner_replay.unwrap(),
        &Default::default(),
        recreated.ordinary_due_snapshot.as_ref().unwrap(),
    );
    assert!(
        owner_store_input(&window.f, &window.recipient, registry.as_ref(), &new_lease)
            .owner_replay
            .unwrap()
            .remaining
            .is_empty()
    );
    registry.unregister(window.watch.as_ref().unwrap().1, window.f.clock.utc_now());
    assert!(
        registry
            .presentation_replay_snapshot(&window.recipient, &new_lease, &current)
            .is_none(),
        "grace grants no replay authority"
    );
}

// Kills snapshot initialization before store success and progress on rollback.
#[test]
fn presentation_owner_replay_refusal_and_store_error_do_not_consume_first_capture() {
    use crate::{
        ports::ModChannels,
        protocol::{commands::DeliveryMode, delivery::DeliveryPullReplyV2, results::ErrorCode},
        test_support::failpoints::Failpoint,
    };
    let mut window = OfferWindowFixture::new(true);
    let id = window.send(0, DeliveryMode::Ordinary, "recover after rollback");
    window.drain(1);
    window.replace_watch();
    let scope = window
        .f
        .directory
        .join("store.db")
        .to_string_lossy()
        .into_owned();
    let fault = Failpoint::error("store.writer.acquired", scope, ErrorCode::StoreBusy);
    assert!(
        window
            .f
            .service
            .handle(
                Command::DeliveryPullV2(window.watch_request()),
                peer(),
                &budget()
            )
            .is_err()
    );
    assert_eq!(fault.fired(), 1);
    drop(fault);
    window.f.db.execute_batch("CREATE TRIGGER owner_replay_save_failure BEFORE UPDATE ON invitation_presentation_consumers BEGIN SELECT RAISE(ABORT,'owner replay rollback'); END;").unwrap();
    assert!(
        window
            .f
            .service
            .handle(
                Command::DeliveryPullV2(window.watch_request()),
                peer(),
                &budget()
            )
            .is_err()
    );
    window
        .f
        .db
        .execute_batch("DROP TRIGGER owner_replay_save_failure;")
        .unwrap();
    let cancelled = budget();
    cancelled.cancellation.cancel();
    assert!(matches!(
        window.f.service.handle(
            Command::DeliveryPullV2(window.watch_request()),
            peer(),
            &cancelled,
        ),
        Err(_)
            | Ok(CommandResult::DeliveryPulledV2(
                DeliveryPullReplyV2::Refused { .. }
            ))
    ));
    let mut stale = window.watch_request();
    stale.claim.execution = ExecutionId::new("other-execution");
    assert!(matches!(
        window
            .f
            .service
            .handle(Command::DeliveryPullV2(stale), peer(), &budget())
            .unwrap(),
        CommandResult::DeliveryPulledV2(DeliveryPullReplyV2::Refused { .. })
    ));
    let (registry, _, lease) = window.watch.as_ref().unwrap();
    let input = owner_store_input(&window.f, &window.recipient, registry.as_ref(), lease);
    assert!(
        input
            .owner_replay
            .as_ref()
            .unwrap()
            .remaining
            .contains_key(&id)
    );
    // Getter remains pure after refused/error store work.
    let DeliveryPullReplyV2::Page(page) = window.raw_watch_pull() else {
        panic!("first successful recovery")
    };
    assert!(page.offered.contains(&id));
    assert_eq!(window.offers()[&id].count, 1);
    let current = window
        .f
        .state
        .lock()
        .binding_mut(
            &window.recipient.seat,
            &binding_for_claim(&window.f, &window.recipient),
            window.recipient.binding_generation,
        )
        .unwrap()
        .due_snapshot(crate::protocol::delivery::PresentationConsumer::Watch);
    assert!(
        registry
            .presentation_replay_snapshot(&window.recipient, lease, &current)
            .unwrap()
            .remaining
            .is_empty()
    );
}

// WR17: the existing scoped store barrier runs after the service snapshot.
// This changes the same shared fields as a concurrent V1 completion, without
// inventing a dual-mechanism harness. It kills pre-store snapshot restoration.
#[test]
fn presentation_owner_replay_preserves_post_snapshot_native_outlook_and_newer_stamp() {
    use crate::{
        protocol::{commands::DeliveryMode, delivery::DeliveryPullReplyV2},
        test_support::failpoints::Failpoint,
    };
    for more in [false, true] {
        let mut window = OfferWindowFixture::new(true);
        let id = window.send(0, DeliveryMode::Ordinary, "concurrent ledger update");
        window.drain(1);
        for _ in 0..4 {
            window.pull();
        }
        {
            let mut ledger = window.f.state.lock();
            let entry = ledger
                .binding_mut(
                    &window.recipient.seat,
                    &binding_for_claim(&window.f, &window.recipient),
                    window.recipient.binding_generation,
                )
                .unwrap();
            entry.more_pending = !more;
            entry.read_attention = Some(111);
            entry.whole_read = Some(222);
        }
        window.replace_watch();
        let state = window.f.state.clone();
        let claim = window.recipient.clone();
        let binding = binding_for_claim(&window.f, &claim);
        let now = window.f.clock.utc_now();
        let replayed = id.clone();
        let barrier = Failpoint::connection(
            "store.writer.acquired",
            window.f.directory.join("store.db").to_string_lossy(),
            move |_| {
                let mut ledger = state.lock();
                let entry = ledger
                    .binding_mut(&claim.seat, &binding, claim.binding_generation)
                    .unwrap();
                entry.record(&replayed, now, true);
                entry.more_pending = more;
                entry.read_attention = Some(777);
                entry.whole_read = Some(888);
            },
        );
        let DeliveryPullReplyV2::Page(page) = window.raw_watch_pull() else {
            panic!("concurrent physical replay")
        };
        assert!(page.offered.contains(&id));
        assert_eq!(barrier.fired(), 1);
        drop(barrier);
        let outlook = owner_outlook(&window);
        assert_eq!(
            (outlook.0, outlook.1, outlook.2),
            (more, Some(777), Some(888))
        );
        assert_eq!(
            outlook.3,
            if more {
                NextPull::Delivers
            } else {
                NextPull::ReofferAt(UtcMillis(now.0 + REOFFER_AFTER_MS))
            }
        );
        let stamp = window.offers()[&id];
        assert_eq!(
            (stamp.count, stamp.truncated),
            (2, true),
            "physical replay cannot overwrite newer logical stamp"
        );
        for _ in 0..3 {
            let reply = window.raw_watch_pull();
            if let DeliveryPullReplyV2::Page(page) = reply {
                assert!(!page.offered.contains(&id));
            }
        }
        assert_eq!(window.offers()[&id], stamp);
    }
}

// Kills replay resetting source-created partials or using normal discovery.
#[test]
fn presentation_owner_replay_preserves_partial_body_until_real_ack() {
    use crate::protocol::{commands::DeliveryMode, delivery::DeliveryPullReplyV2};
    for lazy in [false, true] {
        let mut window = OfferWindowFixture::new(true);
        for _ in 0..4 {
            window.pull();
        }
        {
            let mut ledger = window.f.state.lock();
            let entry = ledger
                .binding_mut(
                    &window.recipient.seat,
                    &binding_for_claim(&window.f, &window.recipient),
                    window.recipient.binding_generation,
                )
                .unwrap();
            for n in 0..255 {
                entry.record(
                    &MessageId::new(format!("absent-{n:03}")),
                    window.f.clock.utc_now(),
                    false,
                );
            }
        }
        let mode = if lazy {
            DeliveryMode::Lazy
        } else {
            DeliveryMode::Ordinary
        };
        let first = window.send(0, mode, &"a".repeat(58000));
        let second = window.send(1, mode, &"é".repeat(32768));
        let mut allocated = false;
        for _ in 0..4 {
            if let DeliveryPullReplyV2::Page(page) = window.pull()
                && page.truncated.contains(&first)
            {
                allocated = true;
                break;
            }
        }
        assert!(allocated);
        let partial: String = window.f.db.query_row("SELECT ordinary_partial FROM invitation_presentation_consumers WHERE seat_id='c' AND consumer='watch'",[],|r|r.get(0)).unwrap();
        let partial: std::collections::HashMap<String, String> =
            serde_json::from_str(&partial).unwrap();
        assert!(
            partial
                .get(second.as_str())
                .is_some_and(|body| !body.is_empty()),
            "fixture must retain a real source-created partial body"
        );
        let parked = capacity_normal_state(&window);
        let stamps = window.offers();
        window.replace_watch();
        let mut reoffered = false;
        for _ in 0..4 {
            if let DeliveryPullReplyV2::Page(page) = window.pull() {
                reoffered |= page.truncated.contains(&first);
            }
            assert_eq!(capacity_normal_state(&window), parked);
            assert_eq!(window.offers(), stamps);
        }
        assert!(reoffered);
        let retained = window.offers();
        let cursor = reclamation_cursor(&window.f, &window.recipient);
        if !lazy {
            window.ack(
                std::slice::from_ref(&first),
                false,
                false,
                "partial-release",
            );
            window.pull();
            assert_eq!(reclamation_cursor(&window.f, &window.recipient), cursor);
            assert_eq!(
                window.offers(),
                retained,
                "manual ACK retains unread full stamps"
            );
            assert_eq!(capacity_normal_state(&window), parked);
        }
        assert_eq!(
            reclamation_read_full_body(&window.f, &first),
            "a".repeat(58000)
        );
        assert_eq!(
            reclamation_cursor(&window.f, &window.recipient),
            cursor,
            "read-only body RPC does not claim display by itself"
        );
        assert_eq!(window.offers(), retained);
        if lazy {
            window
                .f
                .service
                .handle(
                    Command::CompleteInboxDelivery(
                        crate::protocol::commands::CompleteInboxDelivery {
                            messages: vec![first.clone()],
                            operation: OperationId::new("partial-lazy-completion"),
                            claim: window.recipient.clone(),
                            via: None,
                        },
                    ),
                    peer(),
                    &budget(),
                )
                .unwrap();
            assert_eq!(
                window
                    .f
                    .db
                    .query_row(
                        "SELECT state FROM lazy_recipients WHERE message_id=?1 AND seat_id='c'",
                        [first.as_str()],
                        |r| r.get::<_, String>(0)
                    )
                    .unwrap(),
                "displayed"
            );
        } else {
            reclamation_advance_displayed(
                &window.f,
                &window.recipient,
                std::slice::from_ref(&first),
                "partial-first-full-display",
            );
        }
        assert!(reclamation_cursor(&window.f, &window.recipient) > cursor);
        let DeliveryPullReplyV2::Page(page) = window.pull() else {
            panic!("same-call partial resume");
        };
        assert!(page.truncated.contains(&second));
        let work = window
            .f
            .service
            .presentation
            .last_work
            .lock()
            .unwrap()
            .unwrap();
        assert_eq!(
            work.ordinary_due_key_probes, 0,
            "one ordinary source per call"
        );
        assert!(work.ordinary > 0);
    }
}

// Kills replacement interleaving between canonical allocation and replay
// progress installation. Uses the existing real store writer pause.
#[test]
fn presentation_owner_replay_owner_permit_fences_replacement_during_store() {
    use crate::{
        ports::{ModChannelSink, ModChannels},
        protocol::{
            commands::DeliveryMode,
            delivery::DeliveryPullReplyV2,
            watch::{WatchFrame, WatchRequest},
        },
        test_support::failpoints::Failpoint,
    };
    struct Sink;
    impl ModChannelSink for Sink {
        fn push(&self, _: WatchFrame) -> bool {
            true
        }
    }
    let mut window = OfferWindowFixture::new(true);
    let id = window.send(0, DeliveryMode::Ordinary, "owner race body");
    window.drain(1);
    window.replace_watch();
    let request = window.watch_request();
    let stale = request.clone();
    let register = WatchRequest {
        presentation_v2: true,
        claim: window.recipient.clone(),
    };
    let (registry, _, _) = window.watch.as_ref().unwrap();
    let pause = Failpoint::pause(
        "store.writer.acquired",
        window.f.directory.join("store.db").to_string_lossy(),
    );
    let service = &window.f.service;
    let (result, replacement) = std::thread::scope(|scope| {
        let pulling =
            scope.spawn(|| service.handle(Command::DeliveryPullV2(request), peer(), &budget()));
        let fired = pause.wait_fired(1, std::time::Duration::from_secs(5));
        let held = registry
            .presentation_owner(&window.recipient.seat)
            .unwrap()
            .unwrap()
            .try_lock()
            .is_err();
        let (observed_tx, observed_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let permit = registry
            .presentation_owner(&window.recipient.seat)
            .unwrap()
            .unwrap();
        let replacing = scope.spawn(move || {
            let replacement_permit = registry
                .presentation_owner(&register.claim.seat)
                .unwrap()
                .unwrap();
            observed_tx
                .send((
                    Arc::ptr_eq(&permit, &replacement_permit),
                    replacement_permit.try_lock().is_err(),
                ))
                .unwrap();
            let result = service.watch_register(&register, Arc::new(Sink), &budget());
            done_tx.send(()).unwrap();
            result
        });
        // The replacement thread itself has observed the exact permit's
        // contention. register() immediately locks that same registry permit;
        // this does not instrument its internal Mutex::lock call.
        let observed = observed_rx.recv_timeout(std::time::Duration::from_secs(5));
        let completion = done_rx.try_recv();
        pause.release();
        assert_eq!(observed.unwrap(), (true, true));
        assert!(matches!(
            completion,
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
        let result = pulling.join().unwrap().unwrap();
        let replacement = replacing.join().unwrap().unwrap();
        assert!(
            fired && held,
            "canonical store pause must hold the same replacement permit"
        );
        (result, replacement)
    });
    drop(pause);
    assert!(
        matches!(result, CommandResult::DeliveryPulledV2(DeliveryPullReplyV2::Page(ref page)) if page.offered.contains(&id))
    );
    assert!(matches!(
        service
            .handle(Command::DeliveryPullV2(stale), peer(), &budget())
            .unwrap(),
        CommandResult::DeliveryPulledV2(DeliveryPullReplyV2::Refused { .. })
    ));
    window.watch = Some((
        registry.clone(),
        replacement.0,
        registry.presentation_lease(replacement.0).unwrap(),
    ));
    let DeliveryPullReplyV2::Page(page) = window.raw_watch_pull() else {
        panic!("successor immediately recaptures pending body")
    };
    assert!(page.offered.contains(&id));
    assert_eq!(window.offers()[&id].count, 1);
}

// Kills accepting foreign execution/lease/stamps, zero counts, oversized work
// or owner replay on Hook. Every refused input leaves canonical state unchanged.
#[test]
fn presentation_owner_replay_validates_full_scope_and_nonzero_count_domain() {
    use crate::protocol::results::ErrorCode;
    let window = OfferWindowFixture::new(true);
    let id = MessageId::new("owner-validation-missing");
    {
        let mut ledger = window.f.state.lock();
        let entry = ledger
            .binding_mut(
                &window.recipient.seat,
                &binding_for_claim(&window.f, &window.recipient),
                window.recipient.binding_generation,
            )
            .unwrap();
        for _ in 0..255 {
            entry.record(&id, window.f.clock.utc_now(), false);
        }
    }
    let (registry, _, lease) = window.watch.as_ref().unwrap();
    let input = owner_store_input(&window.f, &window.recipient, registry.as_ref(), lease);
    for mutation in 0..9 {
        let mut bad = input.clone();
        let replay = bad.owner_replay.as_mut().unwrap();
        match mutation {
            0 => replay.scope.execution = ExecutionId::new("foreign"),
            1 => replay.scope.lease = "00000000000000000000000000000000".into(),
            2 => replay.scope.incarnation[0] ^= 1,
            3 => replay.scope.generation += 1,
            4 => replay.remaining.get_mut(&id).unwrap().truncated = true,
            5 => {
                replay.remaining.get_mut(&id).unwrap().count = 0;
                bad.ordinary_due_snapshot
                    .as_mut()
                    .unwrap()
                    .stamps
                    .get_mut(&id)
                    .unwrap()
                    .count = 0;
            }
            6 => replay.scope.native_session = "foreign".into(),
            7 => replay.active = false,
            _ => {
                bad.pull.consumer = crate::protocol::delivery::PresentationConsumer::Hook;
                bad.pull.render = crate::protocol::delivery::PresentationRenderRequest::Hook {
                    confirmable_selector: 0,
                    passive_selector: 0,
                };
                bad.pull.presentation_lease = None;
            }
        }
        assert_eq!(
            window
                .f
                .store
                .presentation_pull(bad, &budget())
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        window
            .f
            .db
            .query_row(
                "SELECT count(*) FROM invitation_presentation_consumers",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    let output = window.f.store.presentation_pull(input, &budget()).unwrap();
    assert_eq!(
        output.owner_replay.completed[&id].count, 255,
        "full nonzero recorded u8 domain"
    );
    assert_eq!(window.offers()[&id].count, 255);
}

fn install_owner_hint(input: &mut crate::ports::PresentationInput) {
    let due = input.ordinary_due_snapshot.as_ref().unwrap();
    input.owner_replay = Some(crate::ports::WatchReplaySnapshot {
        scope: crate::ports::WatchReplayScope {
            binding_id: input.binding_id.clone(),
            seat: input.pull.claim.seat.clone(),
            generation: input.pull.claim.binding_generation,
            native_session: input.pull.claim.native_session.as_str().into(),
            execution: input.pull.claim.execution.clone(),
            lease: input.pull.presentation_lease.clone().unwrap(),
            incarnation: due.incarnation,
        },
        remaining: due.stamps.clone(),
        active: !due.stamps.is_empty(),
    });
}

// Kills trusting physical hints as addressing/publication/settlement authority,
// missing manifest-first fallbacks, and spending more than one ordinary slice.
#[test]
fn presentation_owner_replay_mixed_invalid_keys_and_physical_fallback_drain_finitely() {
    use crate::protocol::delivery::{
        DeliveryPresentationConfirmV2, DeliveryPullReplyV2, PresentationConsumer,
    };
    let f = fixture(3);
    service_notice(&f, "owner-wrong-mode");
    lazy_message(
        &f,
        "msg-owner-unpublished",
        "must never recover unpublished body",
    );
    f.db.execute(
        "DELETE FROM send_manifests WHERE message_id='msg-owner-unpublished'",
        [],
    )
    .unwrap();
    let recipient = pull_seat(&f);
    f.service
        .handle(
            Command::Ack(crate::protocol::commands::Ack {
                messages: ids(&["m2"]),
                operation: OperationId::new("owner-mixed-acked"),
                claim: recipient.clone(),
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    f.db.execute(
        "DELETE FROM receipts WHERE message_id='m3' AND seat_id='s'",
        [],
    )
    .unwrap();
    let mut input = capacity_missing_input(&f, &recipient, 251);
    for id in ids(&[
        "m1",
        "m2",
        "m3",
        "owner-wrong-mode",
        "msg-owner-unpublished",
    ]) {
        input.known_messages.push(id.clone());
        input.ordinary_due_snapshot.as_mut().unwrap().stamps.insert(
            id,
            crate::delivery::ledger::Offer {
                first_at: UtcMillis(0),
                last_at: f.clock.utc_now(),
                count: 3,
                truncated: false,
            },
        );
    }
    input.blocked_messages = input.known_messages.clone();
    input.new_message_slots = 0;
    install_owner_hint(&mut input);
    let initial = input.ordinary_due_snapshot.as_ref().unwrap().stamps.clone();
    let mut emitted = Vec::new();
    let mut attempts = 0;
    for _ in 0..3 {
        let output = f.store.presentation_pull(input.clone(), &budget()).unwrap();
        assert_eq!(
            output.owner_replay.source,
            crate::ports::OrdinaryPresentationSource::OwnerReplay
        );
        assert!(output.work.ordinary <= 100 && output.work.ordinary_due_hydrated <= 32);
        assert!(
            output.work.ordinary_due_key_probes <= 1303 && output.work.ordinary_due_seeks <= 100
        );
        assert!(
            output.work.output_passes <= 352
                && output.work.offer_reconciliation_visits <= 256
                && output.work.offer_reconciliation_probes <= 1280
        );
        assert!(
            output.work.initial
                + output.work.context
                + output.work.revisit
                + output.work.ordinary
                + output.work.notices
                + output.work.ordinary_publication_visits
                + output.work.relevance_sources
                + output.work.relevance_fallback
                <= 433
        );
        attempts += output.work.ordinary;
        if let DeliveryPullReplyV2::Page(page) = &output.reply {
            assert_capacity_watch_page_bound(page);
            emitted.extend(page.offered.iter().chain(&page.truncated).cloned());
            if let Some(token) = &page.presentation_token {
                f.store
                    .presentation_confirm(
                        &DeliveryPresentationConfirmV2 {
                            claim: recipient.clone(),
                            consumer: PresentationConsumer::Watch,
                            token: token.clone(),
                            presentation_lease: input.pull.presentation_lease.clone(),
                            emitted_controls: page.emitted_controls.clone(),
                        },
                        &budget(),
                    )
                    .unwrap();
            }
        }
        for id in output.reclaimed_messages {
            assert_eq!(
                id.as_str(),
                "m2",
                "only actual terminal evidence releases capacity"
            );
            input.known_messages.retain(|v| v != &id);
            input.blocked_messages.retain(|v| v != &id);
            input.new_message_slots += 1;
        }
        let replay = input.owner_replay.as_mut().unwrap();
        replay
            .remaining
            .retain(|id, stamp| output.owner_replay.completed.get(id) != Some(stamp));
        input.ordinary_due_snapshot = output.ordinary_due_update;
        input
            .ordinary_due_snapshot
            .as_mut()
            .unwrap()
            .stamps
            .retain(|id, _| input.known_messages.contains(id));
        input.frozen_render = presentation_watch_frozen(&f, &recipient);
    }
    assert!(input.owner_replay.unwrap().remaining.is_empty());
    assert_eq!(
        attempts, 255,
        "one canonically terminal key reclaimed; all other attempts bounded"
    );
    assert_eq!(
        emitted,
        ids(&["m1"]),
        "physical receipt fallback remains canonical"
    );
    assert_eq!(receipt_state(&f, "m1"), "pending");
    assert_eq!(receipt_state(&f, "m2"), "acked");
    assert_eq!(lazy_state(&f, "msg-owner-unpublished"), "pending");
    for (id, stamp) in &input.ordinary_due_snapshot.unwrap().stamps {
        assert_eq!(initial[id], *stamp);
    }
}

// Kills treating passive mixed owner output as a logical native outlook update.
#[test]
fn presentation_owner_replay_mixed_invitation_page_preserves_native_outlook() {
    use crate::protocol::{
        commands::DeliveryMode,
        delivery::{DeliveryPullReplyV2, PresentationItemV2},
    };
    for more in [false, true] {
        let mut window = OfferWindowFixture::new(true);
        let id = window.send(0, DeliveryMode::Ordinary, "mixed physical replay");
        window.drain(1);
        for _ in 0..4 {
            window.pull();
        }
        canonical_lazy_invitation_backlog(
            &window.f,
            &window.recipient,
            &window.sender,
            1,
            "owner-mixed-passive",
        );
        {
            let mut ledger = window.f.state.lock();
            let entry = ledger
                .binding_mut(
                    &window.recipient.seat,
                    &binding_for_claim(&window.f, &window.recipient),
                    window.recipient.binding_generation,
                )
                .unwrap();
            entry.more_pending = more;
            entry.read_attention = Some(777);
            entry.whole_read = Some(888);
        }
        let outlook = owner_outlook(&window);
        let stamps = window.offers();
        window.replace_watch();
        let DeliveryPullReplyV2::Page(page) = window.raw_watch_pull() else {
            panic!("mixed page")
        };
        assert!(page.offered.contains(&id));
        assert!(
            page.items
                .iter()
                .any(|item| matches!(item, PresentationItemV2::Invitation { .. }))
        );
        assert_eq!(owner_outlook(&window), outlook);
        assert_eq!(window.offers(), stamps);
        assert_eq!(
            window
                .f
                .db
                .query_row(
                    "SELECT count(*) FROM invitations WHERE seat_id='c' AND state='pending'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
    }
}

// Kills retaining owner leases/progress across a daemon-local restart or
// resurrecting historical logical stamps rather than normal fresh allocation.
#[test]
fn presentation_owner_replay_restart_discards_lease_and_recovers_canonical_pending() {
    use crate::{
        ports::{ModChannels, ModStoreReads},
        protocol::{
            commands::DeliveryMode, delivery::DeliveryPullReplyV2, watch::ModDeliverySetting,
        },
        service::mod_channels::ModChannelRegistry,
    };
    let mut window = OfferWindowFixture::new(true);
    let id = window.send(0, DeliveryMode::Ordinary, "pending across daemon restart");
    window.drain(1);
    for _ in 0..4 {
        window.pull();
    }
    let stale = window.watch_request();
    let old_registry = window.watch.as_ref().unwrap().0.clone();
    old_registry.close_all(
        crate::protocol::watch::WatchCloseReason::Stopping,
        window.f.clock.utc_now(),
    );
    restart(&mut window.f);
    let reads: Arc<dyn ModStoreReads> = window.f.store.clone();
    let registry = Arc::new(ModChannelRegistry::new(
        window.f.clock.clone(),
        reads,
        "i".into(),
        Arc::new(|| {}),
        ModDeliverySetting::On,
    ));
    window.f.service =
        DomainService::new("i".into(), window.f.store.clone(), window.f.clock.clone())
            .with_cooperative_owner(501, Arc::new(FairWriter::new(32)))
            .with_delivery_pull(window.f.state.clone())
            .with_mod_channels(registry.clone());
    window.watch = Some((registry, window.watch.as_ref().unwrap().1, String::new()));
    window.replace_watch();
    assert!(matches!(
        window
            .f
            .service
            .handle(Command::DeliveryPullV2(stale), peer(), &budget())
            .unwrap(),
        CommandResult::DeliveryPulledV2(DeliveryPullReplyV2::Refused { .. })
    ));
    let DeliveryPullReplyV2::Page(page) = window.raw_watch_pull() else {
        panic!("normal fresh offer after local state loss")
    };
    assert!(page.offered.contains(&id));
    assert_eq!(window.offers()[&id].count, 1);
    assert_eq!(
        window
            .f
            .service
            .presentation
            .last_work
            .lock()
            .unwrap()
            .unwrap()
            .ordinary_due_key_probes,
        0
    );
    assert!(!old_registry.presentation_owner_live(
        &window.recipient.seat,
        &binding_for_claim(&window.f, &window.recipient),
        window.recipient.binding_generation,
        window.recipient.native_session.as_str(),
        window.watch.as_ref().unwrap().2.as_str()
    ));
}

// Kills endlessly probing an unrenderable standalone owner key, populating
// logical due deferrals from owner failures, or refilling on render changes.
#[test]
fn presentation_owner_replay_standalone_unrenderable_attempt_is_finite_and_local() {
    use crate::protocol::{commands::DeliveryMode, delivery::DeliveryPullReplyV2};
    let mut window = OfferWindowFixture::new(true);
    window
        .f
        .db
        .execute("UPDATE seats SET wake_on='all' WHERE id='c'", [])
        .unwrap();
    let id = window.send(0, DeliveryMode::Lazy, &"\0".repeat(65536));
    window.drain(1);
    for _ in 0..4 {
        window.pull();
    }
    let stamps = window.offers();
    let walk = capacity_normal_state(&window);
    window.replace_watch();
    let mut request = window.watch_request();
    request.marker_prefix = vec!["\0".repeat(512); 8];
    let CommandResult::DeliveryPulledV2(reply) = window
        .f
        .service
        .handle(Command::DeliveryPullV2(request), peer(), &budget())
        .unwrap()
    else {
        panic!("V2 response")
    };
    if let DeliveryPullReplyV2::Page(page) = &reply {
        assert!(page.offered.is_empty() && page.truncated.is_empty());
        assert_capacity_watch_page_bound(page);
    }
    assert_eq!(
        window
            .f
            .service
            .presentation
            .last_work
            .lock()
            .unwrap()
            .unwrap()
            .ordinary_due_hydrated,
        1
    );
    assert_eq!(window.offers(), stamps);
    assert_eq!(capacity_normal_state(&window), walk);
    let due = window
        .f
        .state
        .lock()
        .binding_mut(
            &window.recipient.seat,
            &binding_for_claim(&window.f, &window.recipient),
            window.recipient.binding_generation,
        )
        .unwrap()
        .due_snapshot(crate::protocol::delivery::PresentationConsumer::Watch);
    assert!(
        due.cache.deferred.is_empty(),
        "physical failed render never creates logical deferral"
    );
    for _ in 0..3 {
        let reply = window.raw_watch_pull();
        if let DeliveryPullReplyV2::Page(page) = reply {
            assert!(page.offered.is_empty() && page.truncated.is_empty());
        }
    }
    window.replace_watch();
    let DeliveryPullReplyV2::Page(page) = window.raw_watch_pull() else {
        panic!("new lease retries physical rendering")
    };
    assert!(page.truncated.contains(&id));
    assert_eq!(window.offers(), stamps);
}

// Kills dropping active recovery context on a real deferred control-only call.
// A full real invitation page defers Attention; its old token is normally
// confirmed before this separate control opportunity and ordinary replay.
#[test]
fn presentation_owner_replay_deferred_control_only_preserves_current_outlook() {
    use crate::protocol::{
        commands::{CreateThread, DeliveryMode, InvitationConfirmation, Invite},
        delivery::{DeliveryPullReplyV2, PresentationItemV2},
    };
    for more in [false, true] {
        let mut window = OfferWindowFixture::new(true);
        let id = window.send(
            0,
            DeliveryMode::Ordinary,
            "pending through deferred control",
        );
        window.drain(1);
        for _ in 0..4 {
            window.pull();
        }
        canonical_lazy_invitation_backlog(
            &window.f,
            &window.recipient,
            &window.sender,
            32,
            "owner-control-passive",
        );
        let CommandResult::ThreadCreated(thread) = window
            .f
            .service
            .handle(
                Command::CreateThread(CreateThread {
                    name: None,
                    topic: "confirming owner control".into(),
                    goal: "review".into(),
                    operation: OperationId::new("owner-control-thread"),
                    claim: window.sender.clone(),
                }),
                peer(),
                &budget(),
            )
            .unwrap()
        else {
            panic!("real thread")
        };
        window
            .f
            .service
            .handle(
                Command::Invite(Invite {
                    confirmation: Some(InvitationConfirmation::RequireAck),
                    thread,
                    seat: window.recipient.seat.clone(),
                    deadline_millis: Some(60_000),
                    operation: OperationId::new("owner-control-invite"),
                    claim: window.sender.clone(),
                }),
                peer(),
                &budget(),
            )
            .unwrap();
        let DeliveryPullReplyV2::Page(full) = window.raw_watch_pull() else {
            panic!("full invitation output")
        };
        assert_eq!(full.items.len(), 32);
        assert!(full.emitted_controls.is_empty());
        assert!(window.f.db.query_row("SELECT json_extract(control_state,'$.fresh_since_control') FROM invitation_presentation_consumers WHERE seat_id='c' AND consumer='watch'", [], |r|r.get::<_,bool>(0)).unwrap(), "actual full page deferred Attention");
        {
            let mut ledger = window.f.state.lock();
            let entry = ledger
                .binding_mut(
                    &window.recipient.seat,
                    &binding_for_claim(&window.f, &window.recipient),
                    window.recipient.binding_generation,
                )
                .unwrap();
            entry.more_pending = more;
            entry.read_attention = Some(777);
            entry.whole_read = Some(888);
        }
        let outlook = owner_outlook(&window);
        let stamps = window.offers();
        window.replace_watch();
        let replay = window.raw_watch_pull();
        assert!(
            matches!(&replay,DeliveryPullReplyV2::Page(page) if page.presentation_token == full.presentation_token && page.more)
        );
        window.confirm_nonmessage_presentation(&replay);
        let control = window.raw_watch_pull();
        let DeliveryPullReplyV2::Page(page) = &control else {
            panic!("separate deferred Attention")
        };
        assert_eq!(page.items.len(), 1);
        assert!(matches!(
            &page.items[0],
            PresentationItemV2::Attention { .. }
        ));
        assert!(
            page.more,
            "owner recovery debt survives control-only allocation"
        );
        assert!(page.offered.is_empty() && page.truncated.is_empty());
        assert_eq!(
            window
                .f
                .service
                .presentation
                .last_work
                .lock()
                .unwrap()
                .unwrap()
                .ordinary,
            0
        );
        assert_eq!(owner_outlook(&window), outlook);
        assert_eq!(window.offers(), stamps);
        window.confirm_nonmessage_presentation(&control);
        let DeliveryPullReplyV2::Page(body) = window.raw_watch_pull() else {
            panic!("continued physical body")
        };
        assert!(body.offered.contains(&id));
        assert_eq!(window.offers(), stamps);
    }
}

// Causal WR17 prune-all RED: old outbox recovery initializes owner debt;
// supported logical recording changes every captured stamp before the next
// snapshot. The active pass is now empty, and a new passive invitation produces
// a Page without logical Message IDs at either capacity.
#[test]
fn presentation_owner_replay_pruned_all_passive_page_preserves_current_outlook() {
    use crate::{
        protocol::{
            commands::DeliveryMode,
            delivery::{DeliveryPullReplyV2, PresentationItemV2},
        },
        test_support::failpoints::Failpoint,
    };
    for full in [false, true] {
        for more in [false, true] {
            for concurrent in [false, true] {
                let mut window = OfferWindowFixture::new(true);
                let id = window.send(0, DeliveryMode::Ordinary, "pruned owner stamp");
                window.drain(1);
                for _ in 0..4 {
                    window.pull();
                }
                if full {
                    let mut ledger = window.f.state.lock();
                    let entry = ledger
                        .binding_mut(
                            &window.recipient.seat,
                            &binding_for_claim(&window.f, &window.recipient),
                            window.recipient.binding_generation,
                        )
                        .unwrap();
                    for n in 0..255 {
                        entry.record(
                            &MessageId::new(format!("prune-all-missing-{n:03}")),
                            window.f.clock.utc_now(),
                            false,
                        );
                    }
                }
                canonical_lazy_invitation_backlog(
                    &window.f,
                    &window.recipient,
                    &window.sender,
                    1,
                    "prune-all-old",
                );
                let DeliveryPullReplyV2::Page(old) = window.raw_watch_pull() else {
                    panic!("old outbox")
                };
                assert!(old.presentation_token.is_some());
                window.replace_watch();
                let replay = window.raw_watch_pull();
                assert!(
                    matches!(&replay,DeliveryPullReplyV2::Page(page) if page.presentation_token==old.presentation_token && page.more)
                );
                window.confirm_nonmessage_presentation(&replay);
                let (registry, _, lease) = window.watch.as_ref().unwrap();
                assert_eq!(
                    owner_store_input(&window.f, &window.recipient, registry.as_ref(), lease)
                        .owner_replay
                        .unwrap()
                        .remaining
                        .len(),
                    if full { 256 } else { 1 }
                );
                {
                    let mut ledger = window.f.state.lock();
                    let entry = ledger
                        .binding_mut(
                            &window.recipient.seat,
                            &binding_for_claim(&window.f, &window.recipient),
                            window.recipient.binding_generation,
                        )
                        .unwrap();
                    let ids = entry.offers.keys().cloned().collect::<Vec<_>>();
                    for id in ids {
                        entry.record(&id, window.f.clock.utc_now(), false);
                    }
                    entry.more_pending = more;
                    entry.read_attention = Some(777);
                    entry.whole_read = Some(888);
                }
                canonical_lazy_invitation_backlog(
                    &window.f,
                    &window.recipient,
                    &window.sender,
                    1,
                    "prune-all-new",
                );
                let input =
                    owner_store_input(&window.f, &window.recipient, registry.as_ref(), lease);
                let snapshot = input.owner_replay.unwrap();
                assert!(
                    snapshot.active && snapshot.remaining.is_empty(),
                    "actual captured work was pruned, not an initially empty lease"
                );
                assert_eq!(input.new_message_slots, if full { 0 } else { 255 });
                let stamps = window.offers();
                let state = window.f.state.clone();
                let claim = window.recipient.clone();
                let binding = binding_for_claim(&window.f, &claim);
                let barrier = concurrent.then(|| {
                    Failpoint::connection(
                        "store.writer.acquired",
                        window.f.directory.join("store.db").to_string_lossy(),
                        move |_| {
                            let mut ledger = state.lock();
                            let entry = ledger
                                .binding_mut(&claim.seat, &binding, claim.binding_generation)
                                .unwrap();
                            entry.more_pending = !more;
                        },
                    )
                });
                let DeliveryPullReplyV2::Page(page) = window.raw_watch_pull() else {
                    panic!("passive output after prune-all")
                };
                assert!(page.offered.is_empty() && page.truncated.is_empty());
                assert!(
                    page.items
                        .iter()
                        .any(|item| matches!(item, PresentationItemV2::Invitation { .. }))
                );
                assert_eq!(window.offers(), stamps);
                assert_eq!(window.offers()[&id].count, 2);
                let expected = if concurrent { !more } else { more };
                assert_eq!(
                    owner_outlook(&window),
                    (
                        expected,
                        Some(777),
                        Some(888),
                        if expected {
                            NextPull::Delivers
                        } else {
                            NextPull::ReofferAt(UtcMillis(
                                window.f.clock.utc_now().0 + REOFFER_AFTER_MS,
                            ))
                        }
                    ),
                    "full={full} before={more} concurrent={concurrent}"
                );
                if let Some(barrier) = barrier {
                    assert_eq!(barrier.fired(), 1);
                }
            }
        }
    }
}

// Negative control for the prune-all fix: a real normal logical allocation is
// allowed to update native outlook after recovery candidates were invalidated.
#[test]
fn presentation_owner_replay_pruned_all_allows_real_logical_allocation_bookkeeping() {
    use crate::protocol::{commands::DeliveryMode, delivery::DeliveryPullReplyV2};
    let mut observed_outlook_change = false;
    for before in [false, true] {
        let mut window = OfferWindowFixture::new(true);
        let id = window.send(0, DeliveryMode::Ordinary, "captured old");
        window.drain(1);
        for _ in 0..4 {
            window.pull();
        }
        canonical_lazy_invitation_backlog(
            &window.f,
            &window.recipient,
            &window.sender,
            1,
            "prune-logical-old",
        );
        assert!(matches!(
            window.raw_watch_pull(),
            DeliveryPullReplyV2::Page(_)
        ));
        window.replace_watch();
        let replay = window.raw_watch_pull();
        window.confirm_nonmessage_presentation(&replay);
        {
            let mut ledger = window.f.state.lock();
            let entry = ledger
                .binding_mut(
                    &window.recipient.seat,
                    &binding_for_claim(&window.f, &window.recipient),
                    window.recipient.binding_generation,
                )
                .unwrap();
            entry.record(&id, window.f.clock.utc_now(), false);
            entry.more_pending = before;
        }
        let fresh = window.send(1, DeliveryMode::Ordinary, "independent logical allocation");
        assert_eq!(
            owner_outlook(&window).0,
            before,
            "logical allocation starts with the seeded outlook"
        );
        let old_stamp = window.offers()[&id];
        let DeliveryPullReplyV2::Page(page) = window.raw_watch_pull() else {
            panic!("logical output")
        };
        assert!(page.offered.contains(&fresh));
        assert!(!page.offered.contains(&id));
        assert_eq!(window.offers()[&fresh].count, 1);
        assert_eq!(
            window.offers()[&id],
            old_stamp,
            "blocked captured offer remains unchanged"
        );
        assert_eq!(old_stamp.count, 2);
        assert_eq!(
            owner_outlook(&window).0,
            page.more,
            "legitimate logical allocation retains normal bookkeeping"
        );
        observed_outlook_change |= before != page.more;
    }
    assert!(
        observed_outlook_change,
        "at least one actual logical allocation must change the prior outlook; unconditional preservation must fail"
    );
}

// WR9: cancel only after a real exact-key statement completed. A subsequent
// attempt observes the exhausted budget; neither capture nor partial progress
// may install. The scoped trace observes work, without replacing store logic.
#[test]
fn presentation_owner_replay_mid_work_budget_keeps_first_capture_uninstalled() {
    use crate::{
        ports::ModChannels,
        protocol::{commands::DeliveryMode, delivery::DeliveryPullReplyV2, time::Cancellation},
        test_support::failpoints::Failpoint,
    };
    use std::sync::atomic::{AtomicPtr, AtomicUsize};

    struct TraceState {
        handle: AtomicPtr<rusqlite::ffi::sqlite3>,
        completed: AtomicUsize,
        cancellation: Cancellation,
    }
    struct TraceGuard(Arc<TraceState>);
    impl Drop for TraceGuard {
        fn drop(&mut self) {
            let handle = self.0.handle.load(Ordering::SeqCst);
            if !handle.is_null() {
                // SAFETY: this fixture's SqliteStore retains its sole writer
                // connection until after the guard. The synchronous call has
                // returned and no other thread accesses this fixture's writer.
                unsafe {
                    rusqlite::ffi::sqlite3_trace_v2(handle, 0, None, std::ptr::null_mut());
                }
            }
        }
    }
    unsafe extern "C" fn completed_key(
        _: u32,
        data: *mut std::ffi::c_void,
        statement: *mut std::ffi::c_void,
        _: *mut std::ffi::c_void,
    ) -> i32 {
        // SAFETY: SQLite provides a live statement for PROFILE and the guard
        // retains the registered TraceState until after unregistration.
        let state = unsafe { &*data.cast::<TraceState>() };
        let sql = unsafe { rusqlite::ffi::sqlite3_sql(statement.cast()) };
        if !sql.is_null()
            && unsafe { std::ffi::CStr::from_ptr(sql) }
                .to_bytes()
                .starts_with(
                b"SELECT delivery_mode,thread_id,COALESCE(decision_seq,0) FROM messages WHERE id=",
            )
        {
            state.completed.fetch_add(1, Ordering::SeqCst);
            state.cancellation.cancel();
        }
        0
    }

    let mut window = OfferWindowFixture::new(true);
    let real = window.send(0, DeliveryMode::Ordinary, "budget retry body");
    window.drain(1);
    window.replace_watch();
    let add_missing = |id: &MessageId| {
        window
            .f
            .state
            .lock()
            .binding_mut(
                &window.recipient.seat,
                &binding_for_claim(&window.f, &window.recipient),
                window.recipient.binding_generation,
            )
            .unwrap()
            .record(id, window.f.clock.utc_now(), false);
    };
    add_missing(&MessageId::new("000-budget-first"));
    add_missing(&MessageId::new("001-budget-second"));
    let stamps = window.offers();
    let normal = capacity_normal_state(&window);
    let cancelled = budget();
    let trace = TraceGuard(Arc::new(TraceState {
        handle: AtomicPtr::new(std::ptr::null_mut()),
        completed: AtomicUsize::new(0),
        cancellation: cancelled.cancellation.clone(),
    }));
    let state = trace.0.clone();
    let installed = Failpoint::connection(
        "store.writer.acquired",
        window.f.directory.join("store.db").to_string_lossy(),
        move |db| {
            // SAFETY: this hook owns the real writer; the guard's Arc keeps
            // callback state alive, and Drop removes the callback first.
            unsafe {
                let handle = db.handle();
                state.handle.store(handle, Ordering::SeqCst);
                assert_eq!(
                    rusqlite::ffi::sqlite3_trace_v2(
                        handle,
                        rusqlite::ffi::SQLITE_TRACE_PROFILE,
                        Some(completed_key),
                        Arc::as_ptr(&state).cast_mut().cast(),
                    ),
                    rusqlite::ffi::SQLITE_OK
                );
            }
        },
    );
    assert!(
        window
            .f
            .service
            .handle(
                Command::DeliveryPullV2(window.watch_request()),
                peer(),
                &cancelled,
            )
            .is_err()
    );
    assert_eq!(installed.fired(), 1);
    assert_eq!(
        trace.0.completed.load(Ordering::SeqCst),
        1,
        "one actual exact-key attempt completed before exhaustion"
    );
    drop(installed);
    drop(trace);
    assert_eq!(window.offers(), stamps);
    assert_eq!(capacity_normal_state(&window), normal);
    let later = MessageId::new("002-budget-added-after-error");
    add_missing(&later);
    let (registry, _, lease) = window.watch.as_ref().unwrap();
    let snapshot = owner_store_input(&window.f, &window.recipient, registry.as_ref(), lease)
        .owner_replay
        .unwrap();
    assert_eq!(
        snapshot.remaining,
        window.offers(),
        "failed work did not initialize capture or consume first attempt"
    );
    assert!(
        snapshot.remaining.contains_key(&later),
        "first successful capture includes newly recorded work"
    );
    let DeliveryPullReplyV2::Page(page) = window.raw_watch_pull() else {
        panic!("same-lease retry")
    };
    assert!(page.offered.contains(&real));
    assert_eq!(window.offers()[&real].count, 1);
    let work = window
        .f
        .service
        .presentation
        .last_work
        .lock()
        .unwrap()
        .unwrap();
    assert_eq!(work.rpc_passes, 1);
    assert!(
        work.rpc_bytes < 96 * 1024,
        "bounded Watch typed output leaves final RPC headroom"
    );
    let current = window
        .f
        .state
        .lock()
        .binding_mut(
            &window.recipient.seat,
            &binding_for_claim(&window.f, &window.recipient),
            window.recipient.binding_generation,
        )
        .unwrap()
        .due_snapshot(crate::protocol::delivery::PresentationConsumer::Watch);
    assert!(
        registry
            .presentation_replay_snapshot(&window.recipient, lease, &current)
            .unwrap()
            .remaining
            .is_empty()
    );
}

// Q4 body display and captured Lazy continuation controls.
#[path = "q4_body_controls.rs"]
mod q4_body_controls;
