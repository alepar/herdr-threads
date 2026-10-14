//! Q4 canonical direct acceptance: eligibility changes even when an own cursor is preserved.
//! Mounted as a child of delivery_pull's existing real-store test module.
use super::*;
use crate::protocol::{
    commands::{
        Accept, Ack, CompleteInboxDelivery, DeliveryMode, EnsureCreationPolicy, EnsureDirectThread,
        InvitationConfirmation, Invite, Reject, SendMessage,
    },
    delivery::{
        DeliveryPullReplyV2, PresentationConsumer, PresentationItemV2, PresentationRenderSelection,
    },
};
use rusqlite::{OptionalExtension, types::Value};
use std::collections::BTreeMap;

const ORDINARY_BODY: &str = "Q4 joined-only settled direct ordinary body";
const LAZY_BODY: &str = "Q4 current-episode direct Lazy body";

struct AcceptRig {
    q: Q3Channel,
    hook: CallerClaim,
    unrelated: CallerClaim,
    thread: ThreadId,
    invitation: InvitationId,
    lazy: MessageId,
    ordinary: MessageId,
    floor: i64,
}

fn send_direct(
    q: &Q3Channel,
    thread: &ThreadId,
    mode: DeliveryMode,
    body: &str,
    key: &str,
) -> MessageId {
    let result = q
        .window
        .f
        .service
        .handle(
            Command::SendMessage(SendMessage {
                priority: None,
                claim: q.window.sender.clone(),
                thread: thread.clone(),
                body: body.into(),
                delivery_mode: mode,
                direct_recipient: Some(q.window.recipient.seat.clone()),
                // Only an explicit ACK recipient owes a receipt (D9).
                invited_recipients: if mode == DeliveryMode::Ordinary {
                    vec![q.window.recipient.seat.clone()]
                } else {
                    vec![]
                },
                deadline_millis: None,
                user_intent: None,
                relays_user: false,
                operation: OperationId::new(key),
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    let CommandResult::MessageSent(id) = result else {
        panic!("canonical direct publication: {result:?}")
    };
    id
}

fn cursor(f: &Fixture, claim: &CallerClaim, thread: &ThreadId) -> Option<i64> {
    f.db.query_row(
        "SELECT seq FROM binding_cursors WHERE binding_id=?1 AND thread_id=?2",
        params![binding_for_claim(f, claim).as_str(), thread.as_str()],
        |row| row.get(0),
    )
    .optional()
    .unwrap()
}

fn message_sequence(f: &Fixture, message: &MessageId) -> i64 {
    f.db.query_row(
        "SELECT sequence FROM messages WHERE id=?1",
        [message.as_str()],
        |r| r.get(0),
    )
    .unwrap()
}

fn collect_page(
    reply: &DeliveryPullReplyV2,
    output: &mut BTreeMap<MessageId, (String, bool, bool, bool)>,
    consumer: PresentationConsumer,
) -> bool {
    match reply {
        DeliveryPullReplyV2::Unchanged { .. } => true,
        DeliveryPullReplyV2::Page(page) => {
            use crate::{
                delivery::presentation,
                protocol::watch::{WATCH_LINE_SCHEMA, WatchItem, WatchLine},
            };
            let lines = match (consumer, &page.render) {
                (PresentationConsumer::Watch, PresentationRenderSelection::WatchJsonLinesV1) => {
                    // Watch's real RPC page carries structured items and intentionally empty
                    // text. The CLI's output codec emits those items as JSON lines instead.
                    assert!(
                        page.text.is_empty(),
                        "WatchJsonLinesV1 must not carry Hook text: {page:?}"
                    );
                    let mut encoded = Vec::new();
                    for line in presentation::presentation_watch_lines(&page.items) {
                        serde_json::to_writer(&mut encoded, &line).unwrap();
                        encoded.push(b'\n');
                    }
                    assert_eq!(
                        encoded.len(),
                        page.output_bytes as usize,
                        "actual Watch codec size must match the store page's measured output"
                    );
                    let decoded = encoded
                        .split(|byte| *byte == b'\n')
                        .filter(|line| !line.is_empty())
                        .map(|line| {
                            serde_json::from_slice::<WatchLine>(line)
                                .expect("actual encoded Watch JSON line decodes")
                        })
                        .collect::<Vec<_>>();
                    assert_eq!(decoded.len(), page.items.len());
                    assert!(decoded.iter().all(|line| line.schema == WATCH_LINE_SCHEMA));
                    Some(decoded)
                }
                (PresentationConsumer::Hook, PresentationRenderSelection::Hook { .. }) => {
                    assert_eq!(
                        page.text,
                        presentation::presentation_hook_text(&page.items),
                        "actual Hook page text must contain the production item framing"
                    );
                    None
                }
                _ => panic!("consumer/render mismatch: {consumer:?}, {:?}", page.render),
            };
            for item in &page.items {
                if let PresentationItemV2::Message { id, lazy, content } = item {
                    if let Some(lines) = &lines {
                        let line = lines
                            .iter()
                            .find(|line| line.id == id.as_str())
                            .expect("actual encoded Watch line must name the message item");
                        match (&line.item, *lazy) {
                            (WatchItem::Lazy(decoded), true)
                            | (WatchItem::Message(decoded), false) => {
                                assert_eq!(
                                    decoded, content,
                                    "encoded/decoded Watch body and all delivery flags must match the actual store item"
                                );
                            }
                            other => panic!(
                                "actual Watch wire kind disagrees with the store item: {other:?}"
                            ),
                        }
                    } else {
                        assert!(
                            page.text.contains(&content.body),
                            "actual Hook rendered body must accompany item: {page:?}"
                        );
                    }
                    output.insert(
                        id.clone(),
                        (
                            content.body.clone(),
                            *lazy,
                            content.ack_required,
                            content.wakes,
                        ),
                    );
                }
            }
            !page.more
        }
        other => panic!("canonical admitted presentation refused: {other:?}"),
    }
}

fn drain_hook(f: &Fixture, claim: &CallerClaim) -> BTreeMap<MessageId, (String, bool, bool, bool)> {
    let mut output = BTreeMap::new();
    for _ in 0..256 {
        let reply = presentation_test_pull(f, claim);
        let quiet = collect_page(&reply, &mut output, PresentationConsumer::Hook);
        if let DeliveryPullReplyV2::Page(page) = &reply {
            presentation_test_confirm(f, claim, page);
        }
        if quiet {
            return output;
        }
    }
    panic!("bounded Hook continuation failed to quiet")
}

fn drain_watch(
    q: &Q3Channel,
    initial: Option<DeliveryPullReplyV2>,
) -> BTreeMap<MessageId, (String, bool, bool, bool)> {
    let mut output = BTreeMap::new();
    let mut reply = initial.unwrap_or_else(|| q.window.pull());
    for _ in 0..256 {
        if collect_page(&reply, &mut output, PresentationConsumer::Watch) {
            return output;
        }
        reply = q.window.pull();
    }
    panic!("bounded Watch continuation failed to quiet")
}

fn quiet_header(f: &Fixture, claim: &CallerClaim, consumer: &str) {
    let header: (i64, i64, i64) = f.db.query_row(
        "SELECT ordinary_done,ordinary_publication_after,ordinary_publication_high FROM invitation_presentation_consumers WHERE binding_id=?1 AND consumer=?2",
        params![binding_for_claim(f, claim).as_str(), consumer], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    ).unwrap();
    let head: i64 =
        f.db.query_row(
            "SELECT COALESCE(MAX(decision_seq),0) FROM send_manifests WHERE instance_id='i'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        header,
        (1, head, head),
        "SETUP: quiet reader and publication prefix caught up"
    );
}

impl AcceptRig {
    fn new() -> Self {
        let q = Q3Channel::new();
        let f = &q.window.f;
        let ensured = f
            .service
            .handle(
                Command::EnsureDirectThread(EnsureDirectThread {
                    claim: q.window.sender.clone(),
                    recipient: q.window.recipient.seat.clone(),
                    creation_policy: Some(EnsureCreationPolicy {
                        confirmation: InvitationConfirmation::Lazy,
                        deadline_millis: None,
                    }),
                    operation: OperationId::new("q4-accept-ensure"),
                }),
                peer(),
                &budget(),
            )
            .unwrap();
        let CommandResult::DirectThreadEnsured(ensured) = ensured else {
            panic!("modern public Ensure result")
        };
        let thread = ensured.thread;
        let initial = ensured.invitation.expect("new direct invitation");
        let seed = send_direct(
            &q,
            &thread,
            DeliveryMode::Lazy,
            "Q4 own completed early seed",
            "q4-accept-seed",
        );
        assert_eq!(
            reclamation_read_full_body(f, &seed),
            "Q4 own completed early seed"
        );
        let completion = f
            .service
            .handle(
                Command::CompleteInboxDelivery(CompleteInboxDelivery {
                    claim: q.window.recipient.clone(),
                    messages: vec![seed.clone()],
                    via: None,
                    operation: OperationId::new("q4-accept-seed-displayed"),
                }),
                peer(),
                &budget(),
            )
            .unwrap();
        assert!(matches!(
            completion,
            CommandResult::InboxDeliveryCompleted(_)
        ));
        assert_eq!(
            cursor(f, &q.window.recipient, &thread),
            Some(message_sequence(f, &seed)),
            "SETUP: real own display establishes retained cursor without joining"
        );
        f.service
            .handle(
                Command::Reject(Reject {
                    claim: q.window.recipient.clone(),
                    thread: thread.clone(),
                    invitation: initial.id,
                    reason: "complete earlier episode, then explicitly reinvite".into(),
                    operation: OperationId::new("q4-accept-reject"),
                }),
                peer(),
                &budget(),
            )
            .unwrap();
        let invitation = f
            .service
            .handle(
                Command::Invite(Invite {
                    claim: q.window.sender.clone(),
                    thread: thread.clone(),
                    seat: q.window.recipient.seat.clone(),
                    confirmation: Some(InvitationConfirmation::Lazy),
                    deadline_millis: None,
                    operation: OperationId::new("q4-accept-reinvite"),
                }),
                peer(),
                &budget(),
            )
            .unwrap();
        let CommandResult::Invitation(invitation) = invitation else {
            panic!("public reinvitation")
        };
        let floor: i64 = f.db.query_row(
            "SELECT sequence FROM messages WHERE event_key=?1 AND thread_id=?2 AND source_invitation_id=?3 AND kind='info'",
            params![format!("invite:{}", invitation.as_str()), thread.as_str(), invitation.as_str()], |r| r.get(0),
        ).unwrap();
        let lazy = send_direct(&q, &thread, DeliveryMode::Lazy, LAZY_BODY, "q4-accept-lazy");
        let ordinary = send_direct(
            &q,
            &thread,
            DeliveryMode::Ordinary,
            ORDINARY_BODY,
            "q4-accept-ordinary",
        );
        assert_eq!(reclamation_read_full_body(f, &ordinary), ORDINARY_BODY);
        let ack = f
            .service
            .handle(
                Command::Ack(Ack {
                    claim: q.window.recipient.clone(),
                    messages: vec![ordinary.clone()],
                    operation: OperationId::new("q4-accept-settle-ordinary"),
                }),
                peer(),
                &budget(),
            )
            .unwrap();
        assert!(matches!(ack, CommandResult::Acknowledged(_)));
        let job: String = f.db.query_row("SELECT id FROM work_jobs WHERE kind='send_attention' AND subject_id=(SELECT preparation_id FROM send_manifests WHERE message_id=?1)", [ordinary.as_str()], |r| r.get(0)).unwrap();
        finish_presentation_attention_job(f, &job);
        // Real enrollment while invited creates an independent, missing direct cursor.
        let mut hook = fixture_claim(f, "c", "pc", fourth(), "q4-accept-hook");
        hook.liveness.as_mut().unwrap().pid = 5151;
        let hook = check_in(f, fresh_execution(hook, 21), "q4-accept-hook-ci");
        assert!(cursor(f, &hook, &thread).is_none());
        let unrelated = pull_seat(f);
        drain_hook(f, &unrelated);
        let watch_prime = drain_watch(&q, None);
        assert_eq!(
            watch_prime.get(&lazy),
            Some(&(LAZY_BODY.into(), true, false, false)),
            "SETUP: retained-cursor Lazy is actually rendered before acceptance"
        );
        assert!(
            !watch_prime.contains_key(&ordinary),
            "SETUP: settled ordinary remains excluded while invited"
        );
        let hook_prime = drain_hook(f, &hook);
        assert!(!hook_prime.contains_key(&ordinary));
        // Clear all prior publication, ACK hint and invitation frames by actual passes/pulls.
        let manifests: i64 =
            f.db.query_row(
                "SELECT count(*) FROM send_manifests WHERE instance_id='i'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        for _ in 0..usize::try_from(manifests).unwrap().div_ceil(100) + 3 {
            q.registry().pass(now_utc(f)).unwrap();
            if !q.take_frames().is_empty() {
                drain_watch(&q, None);
            }
            drain_hook(f, &hook);
        }
        q.registry().pass(now_utc(f)).unwrap();
        assert!(
            q.take_frames().is_empty(),
            "SETUP: no prior frame remains to fake accept progress"
        );
        quiet_header(f, &q.window.recipient, "watch");
        quiet_header(f, &hook, "hook");
        assert!(
            cursor(f, &hook, &thread).is_none(),
            "rendering and physical confirmation do not complete Lazy"
        );
        assert_eq!(
            f.db.query_row::<String, _, _>(
                "SELECT state FROM memberships WHERE thread_id=?1 AND seat_id='c'",
                [thread.as_str()],
                |r| r.get(0)
            )
            .unwrap(),
            "invited"
        );
        Self {
            q,
            hook,
            unrelated,
            thread,
            invitation,
            lazy,
            ordinary,
            floor,
        }
    }

    fn accept(&self, key: &str) -> Result<CommandResult, crate::protocol::results::ApiError> {
        self.q.window.f.service.handle(
            Command::Accept(Accept {
                claim: self.q.window.recipient.clone(),
                thread: self.thread.clone(),
                operation: OperationId::new(key),
            }),
            peer(),
            &budget(),
        )
    }
}

// Causal RED is the actual post-frame body assertion, not just a private counter check.
#[test]
fn presentation_q4_accept_only_reopens_quiet_watch_and_hook_with_preserved_reader() {
    let rig = AcceptRig::new();
    let f = &rig.q.window.f;
    let own_cursor = cursor(f, &rig.q.window.recipient, &rig.thread);
    let watch_fields = q3_preserved_reader_state(f, &rig.q.window.recipient);
    let hook_fields = q3_preserved_reader_state(f, &rig.hook);
    let watch_counters = q3_reader_counters(f, &rig.q.window.recipient);
    let hook_counters = q3_reader_counters(f, &rig.hook);
    let unrelated = q3_consumer_snapshot(f, &rig.unrelated);
    let unrelated_cursors = q3_cursors(f, &rig.unrelated);
    let watch_offers = rig.q.window.offers();
    let receipts = q3_receipt_snapshot(f);
    let native = rig.q.native_version();
    let result = rig.accept("q4-accept-only").unwrap();
    assert!(
        matches!(result, CommandResult::Accepted(_)),
        "actual canonical acceptance succeeds"
    );
    // Capture transaction-only state before either bounded reader updates its private progress.
    let after_cursor = cursor(f, &rig.q.window.recipient, &rig.thread);
    let sibling_cursor = cursor(f, &rig.hook, &rig.thread);
    let watch_fields_after = q3_preserved_reader_state(f, &rig.q.window.recipient);
    let hook_fields_after = q3_preserved_reader_state(f, &rig.hook);
    let watch_counters_after = q3_reader_counters(f, &rig.q.window.recipient);
    let hook_counters_after = q3_reader_counters(f, &rig.hook);
    assert_eq!(q3_consumer_snapshot(f, &rig.unrelated), unrelated);
    assert_eq!(q3_cursors(f, &rig.unrelated), unrelated_cursors);
    assert_eq!(rig.q.window.offers(), watch_offers);
    assert_eq!(
        q3_receipt_snapshot(f),
        receipts,
        "acceptance creates no body display or ACK evidence"
    );
    assert_eq!(
        rig.q.native_version(),
        native,
        "Lazy invitation acceptance adds no native wake signal"
    );
    // Acceptance is the sole intervening semantic mutation. The parked Watch client may pull
    // only after the real registry frame; no extra send, reset, completion, notify or clock tick.
    let first = rig.q.frame_then_pull();
    let watch = drain_watch(&rig.q, Some(first));
    assert_eq!(
        watch.get(&rig.ordinary),
        Some(&(ORDINARY_BODY.into(), false, false, false)),
        "ACCEPTANCE READER RED: actual framed Watch drain must render newly joined settled ordinary"
    );
    let hook = drain_hook(f, &rig.hook);
    assert_eq!(
        hook.get(&rig.ordinary),
        Some(&(ORDINARY_BODY.into(), false, false, false)),
        "actual Hook drain must render newly joined settled ordinary"
    );
    assert_eq!(
        after_cursor, own_cursor,
        "preserve existing own stream exactly"
    );
    assert_eq!(
        sibling_cursor,
        Some(rig.floor),
        "missing sibling starts at canonical invitation event sequence, not head"
    );
    assert_eq!(watch_fields_after, watch_fields);
    assert_eq!(hook_fields_after, hook_fields);
    q3_assert_one_reader_signal(&watch_counters, &watch_counters_after);
    q3_assert_one_reader_signal(&hook_counters, &hook_counters_after);
    assert!(after_cursor.unwrap() < message_sequence(f, &rig.lazy));
    assert!(sibling_cursor.unwrap() < message_sequence(f, &rig.lazy));
    let current = q3_consumer_snapshot(f, &rig.q.window.recipient);
    let other = q3_consumer_snapshot(f, &rig.hook);
    let cursors = q4_rows(
        f,
        "SELECT * FROM binding_cursors ORDER BY binding_id,thread_id",
    );
    assert!(matches!(
        rig.accept("q4-accept-only").unwrap(),
        CommandResult::Accepted(_)
    ));
    assert_eq!(
        q3_consumer_snapshot(f, &rig.q.window.recipient),
        current,
        "retained accept replay does not invalidate"
    );
    assert_eq!(q3_consumer_snapshot(f, &rig.hook), other);
    assert_eq!(
        q4_rows(
            f,
            "SELECT * FROM binding_cursors ORDER BY binding_id,thread_id"
        ),
        cursors
    );
    assert!(matches!(
        rig.accept("q4-already-accepted-new-key").unwrap(),
        CommandResult::Accepted(_)
    ));
    assert_eq!(
        q3_consumer_snapshot(f, &rig.q.window.recipient),
        current,
        "already accepted fresh operation does not invalidate again"
    );
    assert_eq!(q3_consumer_snapshot(f, &rig.hook), other);
    assert_eq!(
        q4_rows(
            f,
            "SELECT * FROM binding_cursors ORDER BY binding_id,thread_id"
        ),
        cursors
    );
}

fn q4_rows(f: &Fixture, sql: &str) -> Vec<Vec<Value>> {
    let mut statement = f.db.prepare(sql).unwrap();
    statement
        .query_map([], |row| {
            (0..row.as_ref().column_count())
                .map(|i| row.get(i))
                .collect::<rusqlite::Result<Vec<Value>>>()
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

fn acceptance_snapshot(f: &Fixture) -> Vec<Vec<Vec<Value>>> {
    [
        "SELECT * FROM invitations ORDER BY id",
        "SELECT * FROM memberships ORDER BY thread_id,seat_id",
        "SELECT * FROM membership_intervals ORDER BY thread_id,seat_id,episode",
        "SELECT * FROM binding_cursors ORDER BY binding_id,thread_id",
        "SELECT * FROM invitation_presentation_consumers ORDER BY binding_id,consumer",
        "SELECT * FROM threads ORDER BY id",
        "SELECT * FROM messages ORDER BY id",
        "SELECT * FROM receipt_state ORDER BY message_id,seat_id",
        "SELECT * FROM receipts ORDER BY message_id,seat_id",
    ]
    .into_iter()
    .map(|sql| q4_rows(f, sql))
    .collect()
}

// Fault injection is restricted to private counter exhaustion / SQL write failure. Pair,
// invitation, published audience, cursor and completion state are all canonical producer output.
#[test]
fn presentation_q4_accept_private_failure_rolls_back_episode_join_all_readers() {
    for failure in [
        "ordinary_passive_generation",
        "mod_version",
        "later_binding_write",
    ] {
        let rig = AcceptRig::new();
        let f = &rig.q.window.f;
        let hook_binding = binding_for_claim(f, &rig.hook);
        if failure == "later_binding_write" {
            // Snapshot both target bindings' actual consumer counters. Abort an update only
            // after another target binding has stored both increments inside this transaction;
            // neither BindingId order nor Hook/Watch order is part of the acceptance contract.
            let mut baselines = Vec::new();
            for claim in [&rig.q.window.recipient, &rig.hook] {
                let binding = binding_for_claim(f, claim);
                let counters = q3_reader_counters(f, claim);
                assert!(
                    !counters.is_empty(),
                    "SETUP: both affected bindings have real current consumers"
                );
                for (consumer, passive, version) in counters {
                    baselines.push(serde_json::json!({
                        "binding": binding.as_str(), "consumer": consumer,
                        "passive": passive, "version": version,
                    }));
                }
            }
            let baseline = serde_json::to_string(&baselines)
                .unwrap()
                .replace('\'', "''");
            f.db.execute_batch(&format!(
                "CREATE TRIGGER q4_accept_late_failure \
                 BEFORE UPDATE OF ordinary_passive_generation,mod_version ON invitation_presentation_consumers \
                 WHEN NEW.binding_id IN (SELECT json_extract(value,'$.binding') FROM json_each('{baseline}')) \
                  AND EXISTS(SELECT 1 FROM json_each('{baseline}') prior \
                    JOIN invitation_presentation_consumers observed \
                      ON observed.binding_id=json_extract(prior.value,'$.binding') \
                     AND observed.consumer=json_extract(prior.value,'$.consumer') \
                    WHERE observed.binding_id<>NEW.binding_id \
                      AND observed.ordinary_passive_generation=json_extract(prior.value,'$.passive')+1 \
                      AND observed.mod_version=json_extract(prior.value,'$.version')+1) \
                 BEGIN SELECT RAISE(ABORT,'q4 after earlier tentative update'); END;"
            )).unwrap();
        } else {
            f.db.execute(&format!("UPDATE invitation_presentation_consumers SET {failure}=?1 WHERE binding_id=?2 AND consumer='hook'"), params![i64::MAX, hook_binding.as_str()]).unwrap();
        }
        let before = acceptance_snapshot(f);
        let offers = rig.q.window.offers();
        let result = rig.accept("q4-accept-atomic-failure");
        assert!(
            result.is_err(),
            "ACCEPTANCE ATOMICITY RED: checked private failure aborts whole canonical acceptance: {failure}: {result:?}"
        );
        let error = result.unwrap_err();
        if failure == "later_binding_write" {
            assert!(
                error.detail.contains("q4 after earlier tentative update"),
                "must exercise a later failure after an earlier current consumer update: {error:?}"
            );
        } else {
            assert_eq!(
                error.code,
                crate::protocol::results::ErrorCode::SequenceExhausted
            );
        }
        assert_eq!(
            acceptance_snapshot(f),
            before,
            "invitations, membership/intervals, all cursor/private rows and event writes roll back"
        );
        assert_eq!(rig.q.window.offers(), offers);
        assert_eq!(
            f.db.query_row::<String, _, _>(
                "SELECT state FROM invitations WHERE id=?1",
                [rig.invitation.as_str()],
                |r| r.get(0)
            )
            .unwrap(),
            "pending"
        );
    }
}

fn extra_hook(rig: &AcceptRig, session: &str, pid: u32, execution: u8) -> CallerClaim {
    let f = &rig.q.window.f;
    let mut claim = fixture_claim(f, "c", "pc", fourth(), session);
    claim.liveness.as_mut().unwrap().pid = pid;
    check_in(
        f,
        fresh_execution(claim, execution),
        &format!("{session}-ci"),
    )
}

// Existing acceptance fanout includes disconnected-open readers. Genuine enrollment supplies
// missing cursors; current and absent/stale consumers are distinguished without injected rows.
#[test]
fn presentation_q4_accept_fanout_includes_disconnected_open_and_keeps_absent_stale_consumers() {
    use crate::protocol::commands::{BindingEvent, BindingEventKind};
    let rig = AcceptRig::new();
    let f = &rig.q.window.f;
    let absent = extra_hook(&rig, "q4-absent-consumer", 5252, 22);
    assert!(q3_consumer_snapshot(f, &absent).is_empty());
    let disconnected = extra_hook(&rig, "q4-disconnected-reader", 5353, 23);
    drain_hook(f, &disconnected);
    f.service
        .handle(
            Command::BindingEvent(BindingEvent {
                claim: disconnected.clone(),
                kind: BindingEventKind::SessionEnd,
                child_session: None,
                operation: OperationId::new("q4-disconnect-reader"),
            }),
            peer(),
            &budget(),
        )
        .unwrap();
    assert_eq!(
        f.db.query_row::<(String, Option<i64>), _, _>(
            "SELECT liveness,ended_at FROM occupant_bindings WHERE binding_id=?1",
            [binding_for_claim(f, &disconnected).as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap(),
        ("disconnected".into(), None)
    );
    let stale = extra_hook(&rig, "q4-stale-consumer", 5454, 24);
    drain_hook(f, &stale);
    let binding = binding_for_claim(f, &stale);
    let stale = check_in(f, fresh_execution(stale, 25), "q4-stale-new-execution");
    assert_eq!(
        binding_for_claim(f, &stale),
        binding,
        "same canonical session keeps its BindingId"
    );
    assert_ne!(f.db.query_row::<String,_,_>(
        "SELECT execution FROM invitation_presentation_consumers WHERE binding_id=?1 AND consumer='hook'",
        [binding.as_str()], |r| r.get(0),
    ).unwrap(), stale.execution.as_str(), "SETUP: actual lifecycle leaves an old private header for lazy reset");
    for claim in [&absent, &disconnected, &stale] {
        assert!(cursor(f, claim, &rig.thread).is_none());
    }
    let current_before = q3_reader_counters(f, &disconnected);
    let stale_before = q3_consumer_snapshot(f, &stale);
    let unrelated = q3_consumer_snapshot(f, &rig.unrelated);
    assert!(matches!(
        rig.accept("q4-fanout-accept").unwrap(),
        CommandResult::Accepted(_)
    ));
    q3_assert_one_reader_signal(&current_before, &q3_reader_counters(f, &disconnected));
    for claim in [&absent, &disconnected, &stale] {
        assert_eq!(cursor(f, claim, &rig.thread), Some(rig.floor));
    }
    assert!(
        q3_consumer_snapshot(f, &absent).is_empty(),
        "no consumer is manufactured during fanout"
    );
    assert_eq!(
        q3_consumer_snapshot(f, &stale),
        stale_before,
        "stale primary/refresh scope is not promoted"
    );
    assert_eq!(q3_consumer_snapshot(f, &rig.unrelated), unrelated);
}

#[test]
fn presentation_q4_accept_preserves_real_nonempty_carry_outbox_and_full_offer_window() {
    let rig = AcceptRig::new();
    let f = &rig.q.window.f;
    let sent = (0..257)
        .map(|n| {
            rig.q.window.send(
                40_000 + n,
                DeliveryMode::Ordinary,
                "Q4 retained pending capacity body",
            )
        })
        .collect::<Vec<_>>();
    let rendered = drain_watch(&rig.q, None);
    assert_eq!(
        rig.q.window.offers().len(),
        crate::delivery::ledger::MAX_OFFERS_PER_BINDING
    );
    let binding = binding_for_claim(f, &rig.q.window.recipient);
    // Keep actual capture geometry in successful output too. Never print bodies or the ledger.
    let boundary = |stage: &str, completed: Option<&MessageId>, rendered_count: usize| {
        use crate::protocol::pagination::InboxBatchV2CursorState;
        let (done, encoded, pending, partial, skip): (i64, Option<String>, String, String, Option<String>) = f.db.query_row(
            "SELECT ordinary_done,ordinary_cursor,ordinary_pending,ordinary_partial,ordinary_skip_body FROM invitation_presentation_consumers WHERE binding_id=?1 AND consumer='watch'",
            [binding.as_str()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
        ).unwrap();
        let capture = encoded.as_ref().map(|value| {
            InboxBatchV2CursorState::decode_for(value, "i", &rig.q.window.recipient.seat).unwrap()
        });
        let carry: serde_json::Value = serde_json::from_str(&pending).unwrap();
        let carry_ids = carry["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| MessageId::parse(item["content"]["id"].as_str().unwrap()).unwrap())
            .collect::<Vec<_>>();
        let offers = rig.q.window.offers();
        let unoffered = sent
            .iter()
            .filter(|id| Some(*id) != completed && !offers.contains_key(*id))
            .collect::<Vec<_>>();
        let partial: serde_json::Value = serde_json::from_str(&partial).unwrap();
        eprintln!(
            "Q4 carry {stage}: offers={} sent_offers={} lazy={} rendered={rendered_count} unoffered={} ids={:?} done={done} cursor={:?} carry={carry_ids:?} partial={} skip={skip:?}",
            offers.len(),
            sent.iter().filter(|id| offers.contains_key(*id)).count(),
            offers.contains_key(&rig.lazy),
            unoffered.len(),
            &unoffered[..unoffered.len().min(2)],
            capture.as_ref().map(|c| (
                c.source,
                c.unread,
                c.lazy_after_ordinal,
                c.lazy_high_water_ordinal,
                c.publication_decision_high_water,
                c.attention.receipts.as_ref().map(|r| (
                    r.physical_after,
                    r.physical_high_water,
                    r.manifest_after,
                    r.manifest_high_water
                )),
                c.body.as_ref().map(|b| (&b.message, b.offset))
            )),
            partial.as_object().unwrap().len(),
        );
        carry_ids
    };
    let initial = rig.q.window.offers();
    let initial_carry = boundary("before-reclaim", None, rendered.len());
    let unoffered = sent
        .iter()
        .filter(|id| !initial.contains_key(*id))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        unoffered.len(),
        2,
        "SETUP: two canonical sends remain beyond the full offer boundary"
    );
    assert!(
        initial_carry.is_empty(),
        "SETUP: this experiment starts at an empty capture boundary"
    );
    assert!(
        initial.contains_key(&rig.lazy),
        "SETUP: actual invited Lazy occupies the other slot"
    );
    // Send order is canonical thread-t sequence order. Only a whole body actually serialized
    // by drain_watch may supply display evidence; explicit/manual ACK is insufficient.
    let completed = sent
        .iter()
        .find(|id| {
            rendered.contains_key(*id) && initial.get(*id).is_some_and(|stamp| !stamp.truncated)
        })
        .expect("SETUP: actually rendered whole ordinary in thread t");
    assert_eq!(
        rendered.get(completed),
        Some(&(
            "Q4 retained pending capacity body".into(),
            false,
            true,
            false
        ))
    );
    rig.q.window.ack(
        std::slice::from_ref(completed),
        true,
        false,
        "q4-retained-reclaim-one",
    );
    let refill_rendered = drain_watch(&rig.q, None);
    let carried = boundary("after-reclaim", Some(completed), refill_rendered.len());
    let refilled = rig.q.window.offers();
    assert_eq!(
        refilled.len(),
        crate::delivery::ledger::MAX_OFFERS_PER_BINDING,
        "SETUP: one reclaimed slot refills to 256"
    );
    assert!(
        !refilled.contains_key(completed),
        "SETUP: real whole display reclaims exactly its offer"
    );
    for (id, stamp) in &initial {
        if id != completed {
            assert_eq!(
                refilled.get(id),
                Some(stamp),
                "SETUP: untouched offer count/timestamps/truncation survive reclamation: {id:?}"
            );
        }
    }
    assert_eq!(
        unoffered
            .iter()
            .filter(|id| refilled.contains_key(*id))
            .count(),
        1,
        "SETUP: only one previously unknown send is admitted"
    );
    assert_eq!(
        refill_rendered.len(),
        1,
        "SETUP: bounded refill renders exactly one new body"
    );
    assert!(
        refill_rendered
            .keys()
            .all(|id| unoffered.contains(id) && refilled.contains_key(id)),
        "SETUP: refill renders the real pending tail"
    );
    assert!(
        !carried.is_empty(),
        "SETUP: bounded positive-slot capture retains a genuine pending tail"
    );
    for id in &carried {
        assert!(
            unoffered.contains(id) && !refilled.contains_key(id),
            "SETUP: carried body is a real unoffered send"
        );
        let pending: bool = f.db.query_row(
            "SELECT EXISTS(SELECT 1 FROM messages m JOIN send_manifests sm ON sm.message_id=m.id JOIN prepared_recipients pr ON pr.preparation_id=sm.preparation_id AND pr.seat_id=?2 LEFT JOIN receipt_state rs ON rs.message_id=m.id AND rs.seat_id=pr.seat_id WHERE m.id=?1 AND m.thread_id='t' AND pr.ack_required=1 AND COALESCE(rs.state,'pending')='pending' AND m.sequence>COALESCE((SELECT seq FROM binding_cursors WHERE binding_id=?3 AND thread_id=m.thread_id),0))",
            params![id.as_str(), rig.q.window.recipient.seat.as_str(), binding.as_str()], |r| r.get(0),
        ).unwrap();
        assert!(
            pending,
            "SETUP: carried ordinary remains canonically pending and unread: {id:?}"
        );
    }
    canonical_lazy_invitation_backlog(
        f,
        &rig.q.window.recipient,
        &rig.q.window.sender,
        1,
        "q4-retained-outbox",
    );
    let reply = rig.q.window.raw_watch_pull();
    let DeliveryPullReplyV2::Page(page) = reply else {
        panic!("actual nonordinary outbox allocation")
    };
    assert!(page.presentation_token.is_some() && !presentation_invitations(&page).is_empty());
    let retained: (String, Option<String>) = f.db.query_row("SELECT ordinary_pending,outbox_token FROM invitation_presentation_consumers WHERE binding_id=?1 AND consumer='watch'", [binding.as_str()], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
    let carry: serde_json::Value = serde_json::from_str(&retained.0).unwrap();
    assert!(
        !carry["items"].as_array().unwrap().is_empty(),
        "SETUP: actual unoffered carry at full capacity"
    );
    assert!(retained.1.is_some());
    let fields = q3_preserved_reader_state(f, &rig.q.window.recipient);
    let offers = rig.q.window.offers();
    let counters = q3_reader_counters(f, &rig.q.window.recipient);
    let receipts = q3_receipt_snapshot(f);
    let cursors = q3_cursors(f, &rig.q.window.recipient);
    assert!(matches!(
        rig.accept("q4-retained-accept").unwrap(),
        CommandResult::Accepted(_)
    ));
    q3_assert_one_reader_signal(&counters, &q3_reader_counters(f, &rig.q.window.recipient));
    assert_eq!(
        q3_preserved_reader_state(f, &rig.q.window.recipient),
        fields,
        "acceptance cannot reset nonempty carry/outbox/parked progress"
    );
    assert_eq!(
        rig.q.window.offers(),
        offers,
        "full logical offer stamps survive acceptance"
    );
    assert_eq!(
        q3_receipt_snapshot(f),
        receipts,
        "acceptance supplies no receipt/display evidence"
    );
    assert_eq!(
        q3_cursors(f, &rig.q.window.recipient),
        cursors,
        "acceptance preserves this reader's canonical display cursors"
    );
}
