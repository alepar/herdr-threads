//! Prospective Q4 controls over canonical writers and public caller-bound reads.
//! No cursor, membership, delivery flag or publication is seeded by these tests.
use super::*;
use crate::{
    ports::LocalService,
    protocol::{
        authority::{CallerRole, LivenessFingerprint, PeerIdentity},
        commands::{
            AdvanceCursors, BodyReadRequest, CheckIn, CheckInMode, Command, CompleteInboxDelivery,
            EnsureCreationPolicy, InboxQuery, InvitationConfirmation, MessageQuery, Reject,
        },
        pagination::{MAX_PAGE_BYTES, PageRequest},
        results::{InboxBatchV2Item, MessageContent},
    },
    service::{dispatch::DomainService, fair_writer::FairWriter},
};
use rusqlite::OptionalExtension;

struct Q4Readers {
    service: DomainService,
    db: Connection,
    path: PathBuf,
    sender: CallerClaim,
    a: CallerClaim,
}
impl Drop for Q4Readers {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        for suffix in ["-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.path.display()));
        }
    }
}
impl Q4Readers {
    fn new() -> Self {
        let (context, db, path, clock) = fixture(100);
        let store: Arc<dyn StorePort> =
            Arc::new(SqliteStore::new(context, "i", StoreSettings::default()).unwrap());
        let service = DomainService::new("i".into(), store, clock)
            .with_cooperative_owner(501, Arc::new(FairWriter::new(32)));
        let sender = Self::enroll(&service, "s1", "q4-sender", 1, 101);
        let a = Self::enroll(&service, "s2", "q4-a", 1, 202);
        Self {
            service,
            db,
            path,
            sender,
            a,
        }
    }
    fn enroll(
        service: &DomainService,
        seat: &str,
        session: &str,
        execution: u8,
        pid: u32,
    ) -> CallerClaim {
        let mut caller = claim(seat);
        caller.native_session = NativeSessionId::new(session);
        caller.execution =
            ExecutionId::new(format!("00000000-0000-4000-8000-0000000000{execution:02}"));
        caller.role = CallerRole::TopLevel;
        caller.liveness = Some(LivenessFingerprint { pid, start: 1 });
        let CommandResult::CheckedIn(result) = service
            .handle(
                Command::CheckIn(CheckIn {
                    mode: CheckInMode::Lifecycle {
                        expected_binding_generation: caller.binding_generation,
                    },
                    claim: caller,
                    operation: OperationId::new(format!("q4-ci-{session}")),
                    lineage_parent: None,
                }),
                PeerIdentity::from_kernel(501),
                &budget(),
            )
            .unwrap()
        else {
            panic!("SETUP: canonical lifecycle binding")
        };
        result.context
    }
    fn binding(&self, caller: &CallerClaim) -> BindingId {
        crate::store::bindings::match_claim(
            &self.db,
            &crate::store::bindings::BindingKey::from_claim(caller),
        )
        .unwrap()
        .expect("exact canonical binding")
        .binding_id
    }
    fn fresh_reader(&self, session: &str, n: u8) -> CallerClaim {
        let b = Self::enroll(&self.service, "s2", session, n, 202 + u32::from(n));
        assert_ne!(self.binding(&self.a), self.binding(&b));
        let connected: i64 = self.db.query_row("SELECT count(*) FROM occupant_bindings WHERE seat_id='s2' AND ended_at IS NULL AND liveness='connected'", [], |row| row.get(0)).unwrap();
        assert!(
            connected >= 2,
            "independent fingerprints keep both bindings live"
        );
        b
    }
    fn call(&self, command: Command) -> CommandResult {
        self.service
            .handle(command, PeerIdentity::from_kernel(501), &budget())
            .unwrap()
    }
    fn pair(&self) -> ThreadId {
        let CommandResult::DirectThreadEnsured(result) =
            self.call(Command::EnsureDirectThread(EnsureDirectThread {
                creation_policy: Some(EnsureCreationPolicy {
                    confirmation: InvitationConfirmation::Lazy,
                    deadline_millis: None,
                }),
                recipient: self.a.seat.clone(),
                operation: OperationId::new("q4-pair"),
                claim: self.sender.clone(),
            }))
        else {
            panic!("SETUP: actual immutable direct pair")
        };
        assert_eq!(result.recipient, self.a.seat);
        let invitation = result
            .invitation
            .expect("SETUP: initial pending invitation");
        assert_eq!(invitation.episode, 1);
        assert_eq!(invitation.confirmation, InvitationConfirmation::Lazy);
        assert!(invitation.effective_deadline_at.is_none());
        result.thread
    }
    fn send(&self, thread: &ThreadId, body: &str, key: &str) -> MessageId {
        let CommandResult::MessageSent(id) = self.call(Command::SendMessage(SendMessage {
            priority: None,
            delivery_mode: DeliveryMode::Lazy,
            thread: thread.clone(),
            body: body.into(),
            direct_recipient: Some(self.a.seat.clone()),
            invited_recipients: vec![],
            deadline_millis: None,
            operation: OperationId::new(key),
            claim: self.sender.clone(),
            relays_user: false,
            user_intent: None,
        })) else {
            panic!("SETUP: canonical addressed Lazy send")
        };
        let row: (String,i64,String,String) = self.db.query_row(
            "SELECT r.state,r.legacy_pending,m.delivery_mode,r.seat_id FROM lazy_recipients r JOIN send_manifests sm ON sm.preparation_id=r.preparation_id AND sm.message_id=r.message_id AND sm.thread_id=r.thread_id JOIN messages m ON m.id=sm.message_id AND m.instance_id=sm.instance_id AND m.thread_id=sm.thread_id WHERE m.id=?1 AND sm.instance_id='i'",
            [id.as_str()], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
        ).unwrap();
        assert_eq!(row, ("pending".into(), 0, "lazy".into(), "s2".into()));
        id
    }
    fn cursor(&self, caller: &CallerClaim, thread: &ThreadId) -> Option<i64> {
        self.db
            .query_row(
                "SELECT seq FROM binding_cursors WHERE binding_id=?1 AND thread_id=?2",
                rusqlite::params![self.binding(caller).as_str(), thread.as_str()],
                |row| row.get(0),
            )
            .optional()
            .unwrap()
    }
    fn sequence(&self, message: &MessageId) -> i64 {
        self.db
            .query_row(
                "SELECT sequence FROM messages WHERE id=?1",
                [message.as_str()],
                |row| row.get(0),
            )
            .unwrap()
    }
    fn passive_authority(&self, thread: &ThreadId) -> String {
        self.db.query_row("SELECT json_array((SELECT count(*) FROM receipts WHERE thread_id=?1),(SELECT count(*) FROM receipt_state rs JOIN messages m ON m.id=rs.message_id WHERE m.thread_id=?1),(SELECT count(*) FROM invitations WHERE thread_id=?1 AND state='accepted'),(SELECT count(*) FROM membership_intervals WHERE thread_id=?1 AND seat_id='s2'),COALESCE((SELECT attention_version FROM wake_work WHERE seat_id='s2'),0),COALESCE((SELECT reason_bits FROM wake_work WHERE seat_id='s2'),0))", [thread.as_str()], |row| row.get(0)).unwrap()
    }
    fn page(
        &self,
        caller: &CallerClaim,
        cursor: Option<String>,
        max_bytes: u32,
    ) -> crate::protocol::pagination::Page<InboxBatchV2Item> {
        let CommandResult::InboxBatchV2(page) = self.call(Command::InboxBatchV2(InboxQuery {
            thread: None,
            seat: Some(caller.seat.clone()),
            claim: Some(caller.clone()),
            page: PageRequest {
                cursor,
                limit: 100,
                max_bytes,
            },
        })) else {
            panic!("public own-binding inbox")
        };
        page
    }
    fn lazy(&self, caller: &CallerClaim) -> Vec<(MessageId, String)> {
        let page = self.page(caller, None, MAX_PAGE_BYTES);
        assert!(
            page.next_cursor.is_none(),
            "small fixture finishes one genuine public page"
        );
        page.items
            .into_iter()
            .filter_map(|item| match item {
                InboxBatchV2Item::LazyMessage {
                    message,
                    body,
                    body_start: 0,
                    body_end,
                    body_len,
                    wakes: false,
                    ..
                } => {
                    assert_eq!(body_end, body_len, "small body is actually complete");
                    Some((message, body))
                }
                InboxBatchV2Item::LazyMessage { .. } => {
                    panic!("unexpected partial/waking small Lazy body")
                }
                _ => None,
            })
            .collect()
    }
    fn body(&self, message: &MessageId, expected: &str) {
        let CommandResult::Message(read) = self.call(Command::Message(MessageQuery {
            message: message.clone(),
            body: BodyReadRequest {
                cursor: None,
                offset: None,
                max_bytes: MAX_PAGE_BYTES,
            },
        })) else {
            panic!("public body query")
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
        assert_eq!(body_offset, 0);
        assert_eq!(body_data, expected);
        assert_eq!(body_total_bytes, expected.len() as u64);
        assert!(body_complete && body_next_cursor.is_none());
    }
    fn advance(&self, caller: &CallerClaim, shown: &[MessageId], key: &str) {
        assert!(matches!(
            self.call(Command::AdvanceCursors(AdvanceCursors {
                in_turn: false,
                invitations: vec![],
                claim: caller.clone(),
                displayed: shown.to_vec(),
                operation: OperationId::new(key),
            })),
            CommandResult::CursorsAdvanced(_)
        ));
    }
    fn complete(&self, caller: &CallerClaim, shown: &[MessageId], key: &str) {
        assert!(matches!(
            self.call(Command::CompleteInboxDelivery(CompleteInboxDelivery {
                claim: caller.clone(),
                messages: shown.to_vec(),
                operation: OperationId::new(key),
                via: None,
            })),
            CommandResult::InboxDeliveryCompleted(_)
        ));
    }
    fn accept(&self, thread: &ThreadId, key: &str) {
        assert!(matches!(
            self.call(Command::Accept(Accept {
                thread: thread.clone(),
                operation: OperationId::new(key),
                claim: self.a.clone(),
            })),
            CommandResult::Accepted(_)
        ));
    }
    fn reject(&self, thread: &ThreadId) {
        let invitation = self.db.query_row("SELECT id FROM invitations WHERE thread_id=?1 AND seat_id='s2' ORDER BY episode DESC LIMIT 1", [thread.as_str()], |row| row.get::<_,String>(0)).unwrap();
        assert!(matches!(
            self.call(Command::Reject(Reject {
                thread: thread.clone(),
                invitation: InvitationId::new(invitation),
                reason: "declined initial invitation".into(),
                operation: OperationId::new("q4-reject"),
                claim: self.a.clone(),
            })),
            CommandResult::Rejected(_)
        ));
    }
}

#[test]
fn q4_initial_published_direct_lazy_is_readable_without_a_recipient_cursor() {
    let f = Q4Readers::new();
    let thread = f.pair();
    let message = f.send(&thread, "initial exact passive body", "q4-initial-send");
    assert_eq!(f.cursor(&f.a, &thread), None);
    let authority = f.passive_authority(&thread);
    assert_eq!(
        authority, "[0,0,0,0,0,0]",
        "SETUP: Lazy body/invitation creates no receipt, join or native wake"
    );
    assert_eq!(
        f.lazy(&f.a),
        [(message.clone(), "initial exact passive body".into())]
    );
    assert_eq!(
        f.cursor(&f.a, &thread),
        None,
        "a read never creates a cursor"
    );
    assert_eq!(
        f.passive_authority(&thread),
        authority,
        "read is no invitation/receipt/wake authority"
    );
}

#[test]
fn q4_direct_lazy_completion_is_independent_for_two_initial_bindings() {
    let f = Q4Readers::new();
    let b = f.fresh_reader("q4-b", 2);
    let thread = f.pair();
    let message = f.send(
        &thread,
        "one body for independent bindings",
        "q4-independent-send",
    );
    assert_eq!(f.cursor(&f.a, &thread), None);
    assert_eq!(f.cursor(&b, &thread), None);
    let want = vec![(message.clone(), "one body for independent bindings".into())];
    assert_eq!(f.lazy(&f.a), want);
    assert_eq!(f.lazy(&b), want);
    let authority = f.passive_authority(&thread);
    f.complete(&f.a, std::slice::from_ref(&message), "q4-a-complete");
    assert!(
        f.cursor(&f.a, &thread)
            .is_some_and(|seq| seq >= f.sequence(&message))
    );
    assert_eq!(f.cursor(&b, &thread), None);
    assert!(f.lazy(&f.a).is_empty());
    assert_eq!(f.lazy(&b), want);
    assert_eq!(f.passive_authority(&thread), authority);
    assert_eq!(
        f.db.query_row::<(String, i64), _, _>(
            "SELECT state,legacy_pending FROM lazy_recipients WHERE message_id=?1 AND seat_id='s2'",
            [message.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?))
        )
        .unwrap(),
        ("displayed".into(), 0)
    );
}

#[test]
fn q4_initial_accept_preserves_each_bindings_unread_body_and_display_gap() {
    for display_later in [false, true] {
        let f = Q4Readers::new();
        let b = f.fresh_reader("q4-gap-b", 2);
        let thread = f.pair();
        let first = f.send(&thread, "earlier undisplayed body", "q4-gap-first");
        let later = f.send(&thread, "later inspected body", "q4-gap-later");
        if display_later {
            f.body(&later, "later inspected body");
            f.advance(&f.a, std::slice::from_ref(&later), "q4-gap-later-display");
        }
        let a_before = f.cursor(&f.a, &thread);
        assert!(
            a_before.unwrap_or(0) < f.sequence(&first),
            "later display cannot pass the unshown earlier body"
        );
        assert_eq!(f.cursor(&b, &thread), None);
        f.accept(&thread, "q4-initial-accept");
        if let Some(before) = a_before {
            assert_eq!(
                f.cursor(&f.a, &thread),
                Some(before),
                "accept preserves an established own gap"
            );
        }
        assert!(f.cursor(&f.a, &thread).unwrap() < f.sequence(&first));
        assert!(f.cursor(&b, &thread).unwrap() < f.sequence(&first));
        let want = vec![
            (first.clone(), "earlier undisplayed body".into()),
            (later.clone(), "later inspected body".into()),
        ];
        assert_eq!(f.lazy(&f.a), want);
        assert_eq!(f.lazy(&b), want);
        assert_eq!(
            f.db.query_row::<i64, _, _>(
                "SELECT count(*) FROM membership_intervals WHERE thread_id=?1 AND seat_id='s2'",
                [thread.as_str()],
                |row| row.get(0)
            )
            .unwrap(),
            1
        );
        assert_eq!(
            f.db.query_row::<i64, _, _>(
                "SELECT count(*) FROM receipts WHERE thread_id=?1",
                [thread.as_str()],
                |row| row.get(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(f.db.query_row::<i64,_,_>("SELECT count(*) FROM receipt_state rs JOIN messages m ON m.id=rs.message_id WHERE m.thread_id=?1",[thread.as_str()],|row|row.get(0)).unwrap(),0);
    }
}

#[test]
fn q4_binding_enrolled_after_direct_join_keeps_main_seat_maximum_no_backlog() {
    let f = Q4Readers::new();
    let b = f.fresh_reader("q4-old-unread-b", 2);
    let thread = f.pair();
    let old = f.send(&thread, "old body A has displayed", "q4-old-before-join");
    f.body(&old, "old body A has displayed");
    f.complete(&f.a, std::slice::from_ref(&old), "q4-old-a-complete");
    f.accept(&thread, "q4-join-before-new-binding");
    let a_cursor = f.cursor(&f.a, &thread).unwrap();
    let b_cursor = f.cursor(&b, &thread).unwrap();
    assert!(
        b_cursor < f.sequence(&old),
        "existing unread sibling is preserved by initial accept"
    );
    let c = f.fresh_reader("q4-post-join-c", 3);
    assert_eq!(
        f.cursor(&c, &thread),
        Some(a_cursor.max(b_cursor)),
        "joined enrollment retains Main seat maximum"
    );
    assert!(f.lazy(&c).is_empty());
    assert_eq!(
        f.lazy(&b),
        [(old.clone(), "old body A has displayed".into())]
    );
    let fresh = f.send(
        &thread,
        "new body after joined enrollment",
        "q4-new-after-join",
    );
    assert_eq!(
        f.lazy(&c),
        [(fresh, "new body after joined enrollment".into())]
    );
}

#[test]
fn q4_rejection_prevents_cursorless_direct_lazy_revival_or_display_bootstrap() {
    let f = Q4Readers::new();
    let thread = f.pair();
    let old = f.send(&thread, "body before rejection", "q4-rejected-send");
    assert_eq!(f.cursor(&f.a, &thread), None);
    f.reject(&thread);
    let b = f.fresh_reader("q4-rejected-b", 2);
    assert_eq!(f.cursor(&b, &thread), None);
    let authority = f.passive_authority(&thread);
    assert!(f.lazy(&f.a).is_empty());
    assert!(f.lazy(&b).is_empty());
    f.body(&old, "body before rejection");
    f.advance(&b, std::slice::from_ref(&old), "q4-rejected-late-display");
    assert_eq!(
        f.cursor(&b, &thread),
        None,
        "rejected cursorless reader cannot bootstrap an old stream"
    );
    assert_eq!(f.passive_authority(&thread), authority);
    assert_eq!(f.db.query_row::<String,_,_>("SELECT COALESCE(voluntary_state,state) FROM memberships WHERE thread_id=?1 AND seat_id='s2'",[thread.as_str()],|row|row.get(0)).unwrap(),"absent");
}

#[test]
fn q4_episode_two_fresh_direct_lazy_is_admitted_without_old_backlog_before_or_after_accept() {
    let f = Q4Readers::new();
    let thread = f.pair();
    let prefix = f.send(
        &thread,
        "old prefix actually displayed by A",
        "q4-episode-one-prefix",
    );
    f.body(&prefix, "old prefix actually displayed by A");
    f.complete(
        &f.a,
        std::slice::from_ref(&prefix),
        "q4-episode-one-prefix-complete",
    );
    let established = f.cursor(&f.a, &thread).unwrap();
    assert!(established >= f.sequence(&prefix));
    let old = f.send(
        &thread,
        "old undisplayed episode gap",
        "q4-episode-one-body",
    );
    assert!(established < f.sequence(&old));
    f.reject(&thread);
    assert_eq!(f.cursor(&f.a, &thread), Some(established));
    let CommandResult::Invitation(invitation) = f.call(Command::Invite(Invite {
        thread: thread.clone(),
        seat: f.a.seat.clone(),
        confirmation: Some(InvitationConfirmation::Lazy),
        deadline_millis: None,
        operation: OperationId::new("q4-episode-two-invite"),
        claim: f.sender.clone(),
    })) else {
        panic!("SETUP: genuine fresh reinvitation")
    };
    let fresh = f.send(&thread, "fresh episode two body", "q4-episode-two-body");
    let order:(i64,i64,i64,i64)=f.db.query_row("SELECT i.episode,i.created_decision_seq,(SELECT decision_seq FROM send_manifests WHERE message_id=?2),(SELECT decision_seq FROM send_manifests WHERE message_id=?3) FROM invitations i WHERE i.id=?1",rusqlite::params![invitation.as_str(),old.as_str(),fresh.as_str()],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).unwrap();
    assert_eq!(order.0, 2);
    assert!(order.2 < order.1 && order.1 < order.3);
    let b = f.fresh_reader("q4-episode-two-b", 2);
    let c = f.fresh_reader("q4-episode-two-c", 3);
    let d = f.fresh_reader("q4-episode-two-d", 4);
    assert_eq!(f.cursor(&d, &thread), None);
    assert_eq!(f.cursor(&b, &thread), None);
    assert_eq!(f.cursor(&c, &thread), None);
    let want = vec![(fresh.clone(), "fresh episode two body".into())];
    assert_eq!(f.lazy(&b), want);
    assert_eq!(f.lazy(&c), want);
    assert_eq!(f.lazy(&d), want);
    assert_eq!(
        f.cursor(&d, &thread),
        None,
        "public reads leave D cursorless"
    );
    let established_stream = vec![
        (old.clone(), "old undisplayed episode gap".into()),
        (fresh.clone(), "fresh episode two body".into()),
    ];
    assert_eq!(
        f.lazy(&f.a),
        established_stream,
        "the existing cursor retains already-addressed work"
    );
    let authority = f.passive_authority(&thread);
    f.body(&fresh, "fresh episode two body");
    f.complete(
        &b,
        std::slice::from_ref(&fresh),
        "q4-episode-two-b-complete",
    );
    let b_cursor = f.cursor(&b, &thread).unwrap();
    assert!(
        b_cursor >= f.sequence(&fresh),
        "canonical completion passes old history excluded from B's current stream"
    );
    assert_eq!(f.cursor(&c, &thread), None);
    assert_eq!(
        f.lazy(&c),
        want,
        "B's shared Lazy bookkeeping is not C's display"
    );
    assert!(f.lazy(&b).is_empty());
    f.body(&fresh, "fresh episode two body");
    f.advance(
        &c,
        std::slice::from_ref(&fresh),
        "q4-episode-two-c-body-display",
    );
    let c_cursor = f.cursor(&c, &thread).unwrap();
    assert!(
        c_cursor >= f.sequence(&fresh),
        "public full-body display passes C's same excluded old gap"
    );
    assert!(f.lazy(&c).is_empty());
    assert_eq!(f.cursor(&f.a, &thread), Some(established));
    assert_eq!(f.lazy(&f.a), established_stream);
    assert_eq!(
        f.passive_authority(&thread),
        authority,
        "neither display owner acquires participation, receipts or native wake"
    );
    // D has received no display/completion claim. Shared completion by B
    // cannot turn acceptance's missing-cursor floor into a displayed body.
    assert_eq!(f.cursor(&d, &thread), None);
    assert_eq!(f.lazy(&d), want);
    assert_eq!(f.cursor(&d, &thread), None);
    f.accept(&thread, "q4-episode-two-accept");
    let d_floor = f
        .cursor(&d, &thread)
        .expect("accept seeds D's current-episode floor");
    assert!(
        f.sequence(&old) < d_floor && d_floor < f.sequence(&fresh),
        "episode floor excludes old history while preserving fresh unread body"
    );
    assert_eq!(f.lazy(&d), want);
    assert_eq!(
        f.cursor(&f.a, &thread),
        Some(established),
        "later acceptance preserves the established old stream"
    );
    assert_eq!(f.cursor(&b, &thread), Some(b_cursor));
    assert_eq!(f.cursor(&c, &thread), Some(c_cursor));
    assert_eq!(f.lazy(&f.a), established_stream);
    assert!(f.lazy(&b).is_empty());
    assert!(f.lazy(&c).is_empty());
    assert_eq!(
        f.db.query_row::<String, _, _>(
            "SELECT state FROM lazy_recipients WHERE message_id=?1 AND seat_id='s2'",
            [old.as_str()],
            |row| row.get(0)
        )
        .unwrap(),
        "pending",
        "old addressed history is retained, not deleted to satisfy scope"
    );
}
