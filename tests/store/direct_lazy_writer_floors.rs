//! Prospective Q4 controls over canonical writers and public caller-bound reads.
//! No cursor, membership, delivery flag or publication is seeded by these tests.
use super::*;
use crate::{
    ports::{LocalService, ServiceAuthorityGate},
    protocol::service::{
        EnsureManagedThread, InvitationConstraint, ServiceInvite, ServiceOperation, ServiceResult,
    },
    protocol::{
        authority::{CallerRole, LivenessFingerprint, PeerIdentity},
        commands::{
            AdvanceCursors, BodyReadRequest, CheckIn, CheckInMode, Command, CompleteInboxDelivery,
            EnsureCreationPolicy, InboxQuery, InvitationConfirmation, Join, MessageQuery, Reject,
        },
        pagination::{MAX_PAGE_BYTES, PageRequest},
        results::{InboxBatchV2Item, MessageContent},
    },
    service::{dispatch::DomainService, fair_writer::FairWriter},
};
use rusqlite::OptionalExtension;
use sha2::Digest;

struct Q4WriterReaders {
    service: DomainService,
    db: Connection,
    path: PathBuf,
    sender: CallerClaim,
    a: CallerClaim,
}
impl Drop for Q4WriterReaders {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        for suffix in ["-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.path.display()));
        }
    }
}
impl Q4WriterReaders {
    fn new() -> Self {
        let (context, db, path, clock) = fixture(100);
        db.pragma_update(
            None,
            "application_id",
            crate::store::control::LEGACY_PAIR_MARKER,
        )
        .unwrap();
        let store: Arc<dyn StorePort> =
            Arc::new(SqliteStore::new(context, "i", StoreSettings::default()).unwrap());
        let service = DomainService::new("i".into(), store, clock)
            .with_cooperative_owner(501, Arc::new(FairWriter::new(32)));
        let sender = Self::enroll(&service, "s1", "q4-writer-sender", 1, 101);
        let a = Self::enroll(&service, "s2", "q4-writer-a", 1, 202);
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
                    operation: OperationId::new(format!("q4-writer-ci-{session}")),
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
                operation: OperationId::new("q4-writer-pair"),
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
    fn page(
        &self,
        caller: &CallerClaim,
        cursor: Option<String>,
        max_bytes: u32,
    ) -> crate::protocol::pagination::Page<InboxBatchV2Item> {
        let CommandResult::InboxBatchV2(page) = self.call(Command::InboxBatchV2(InboxQuery {
            pushed: false,
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
                receipt_candidates: Vec::new(),
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

impl Q4WriterReaders {
    fn context(&self) -> StoreContext {
        StoreContext::new(self.path.clone(), Arc::new(FixedClock(AtomicI64::new(100))))
    }
    fn service_operation(&self, operation: ServiceOperation) -> Result<ServiceResult, ApiError> {
        let author = ServiceAuthorId::new(format!("graph:{:x}", sha2::Sha256::digest(b"i")));
        let gate = GraphGate(author.clone());
        let connection = gate.register("i", "q4-writer-boot", author).unwrap();
        let context = self.context();
        let mut writer = context.open_writer().unwrap();
        crate::store::service_controls::operate(
            &context,
            &mut writer,
            "i",
            operation,
            &connection,
            &gate,
            &budget(),
            None,
        )
    }
    fn ordinary_service_invite(&self, thread: &ThreadId, key: &str) -> InvitationId {
        let ServiceResult::Invitation(result) = self
            .service_operation(ServiceOperation::Invite(ServiceInvite {
                thread: thread.clone(),
                seat: self.a.seat.clone(),
                constraint: InvitationConstraint::Ordinary,
                deadline_millis: Some(10_000),
                operation: OperationId::new(key),
            }))
            .expect("SETUP: registered Ordinary service writer admits canonical pair endpoint")
        else {
            panic!("SETUP: service returned an ordinary invitation")
        };
        assert!(result.requirement.is_none());
        result.invitation
    }
    fn native_invite(&self, thread: &ThreadId, key: &str) -> InvitationId {
        let CommandResult::Invitation(invitation) = self.call(Command::Invite(Invite {
            thread: thread.clone(),
            seat: self.a.seat.clone(),
            confirmation: Some(InvitationConfirmation::Lazy),
            deadline_millis: None,
            operation: OperationId::new(key),
            claim: self.sender.clone(),
        })) else {
            panic!("SETUP: joined native caller admits exact pair endpoint")
        };
        invitation
    }
    fn operator_invite(&self, thread: &ThreadId, key: &str) -> InvitationId {
        let context = self.context();
        let mut writer = context.open_writer().unwrap();
        let actor = OperatorActor::from_peer(PeerIdentity::from_kernel(501), 501).unwrap();
        let CommandResult::OperatorInvited(invitation) =
            crate::store::control::operator_orphan_invite(
                &context,
                &mut writer,
                "i",
                &OperatorOrphanInvite {
                    thread: thread.clone(),
                    seat: self.a.seat.clone(),
                    deadline_millis: Some(10_000),
                    operation: OperationId::new(key),
                },
                actor,
                None,
            )
            .expect("SETUP: both endpoints left; canonical operator writer admits orphan")
        else {
            panic!("SETUP: operator returned an ordinary invitation")
        };
        invitation
    }
    fn leave(&self, thread: &ThreadId, caller: &CallerClaim, key: &str) {
        assert!(matches!(
            self.call(Command::Leave(Leave {
                thread: thread.clone(),
                claim: caller.clone(),
                operation: OperationId::new(key),
            })),
            CommandResult::Left(_)
        ));
    }
    fn sender_join(&self, thread: &ThreadId) {
        assert!(matches!(
            self.call(Command::Join(Join {
                thread: thread.clone(),
                claim: self.sender.clone(),
                operation: OperationId::new("q4-writer-sender-rejoin"),
            })),
            CommandResult::Joined(_)
        ));
    }
    fn invitation_row(
        &self,
        invitation: &InvitationId,
    ) -> (i64, i64, Option<String>, Option<String>) {
        self.db.query_row(
            "SELECT episode,created_decision_seq,confirmation,inviter_seat_id FROM invitations WHERE id=?1",
            [invitation.as_str()], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        ).unwrap()
    }
    fn invitation_event(&self, invitation: &InvitationId) -> (i64, i64, String) {
        let rows: i64 = self.db.query_row(
            "SELECT count(*) FROM messages WHERE source_invitation_id=?1 AND json_extract(event_json,'$.action') IN ('invite','service_invite','operator_orphan_invite')",
            [invitation.as_str()], |row| row.get(0),
        ).unwrap();
        assert_eq!(
            rows, 1,
            "SETUP: one real canonical invitation event provides the episode boundary"
        );
        self.db.query_row(
            "SELECT sequence,decision_seq,json_extract(event_json,'$.action') FROM messages WHERE source_invitation_id=?1 AND json_extract(event_json,'$.action') IN ('invite','service_invite','operator_orphan_invite')",
            [invitation.as_str()], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap()
    }
    // Exact durable rows: operations/replay records may grow on fresh duplicate keys;
    // the episode, floor evidence, readers and immutable pair must remain identical.
    fn floor_authority_rows(&self) -> Vec<(String, Vec<Vec<String>>)> {
        [
            "threads",
            "direct_threads",
            "memberships",
            "membership_intervals",
            "invitations",
            "invitation_cancellations",
            "invitation_rejections",
            "requirement_episodes",
            "messages",
            "send_manifests",
            "lazy_recipients",
            "binding_cursors",
            "wake_work",
        ]
        .into_iter()
        .map(|table| {
            let query = format!("SELECT * FROM {table}");
            let columns = self.db.prepare(&query).unwrap().column_count();
            let order = (1..=columns)
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join(",");
            let mut stmt = self
                .db
                .prepare(&format!("{query} ORDER BY {order}"))
                .unwrap();
            let rows = stmt
                .query_map([], |row| {
                    (0..columns)
                        .map(|i| {
                            row.get::<_, rusqlite::types::Value>(i)
                                .map(|value| format!("{value:?}"))
                        })
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            (table.to_owned(), rows)
        })
        .collect()
    }
}

fn assert_current_episode_readers(
    f: &Q4WriterReaders,
    thread: &ThreadId,
    invitation: &InvitationId,
    old: &MessageId,
    old_cursor: i64,
    expected_action: &str,
) {
    let fresh = f.send(thread, "new episode exact body", "q4-writer-fresh-body");
    let (episode, decision, confirmation, inviter) = f.invitation_row(invitation);
    assert_eq!(episode, 2, "SETUP: actual later invitation episode");
    assert_eq!(
        (confirmation, inviter),
        (None, None),
        "SETUP: legitimate timed writer has no native inviter"
    );
    let event = f.invitation_event(invitation);
    assert_eq!(event.2, expected_action);
    assert!(
        event.1 >= decision,
        "SETUP: canonical event is at or after invitation creation decision"
    );
    assert!(
        f.sequence(old) < event.0 && event.0 < f.sequence(&fresh),
        "SETUP: canonical boundary lies between old backlog and published fresh body"
    );
    let published: (i64,i64) = f.db.query_row(
        "SELECT (SELECT decision_seq FROM send_manifests WHERE message_id=?1),(SELECT decision_seq FROM send_manifests WHERE message_id=?2)",
        rusqlite::params![old.as_str(),fresh.as_str()], |row| Ok((row.get(0)?,row.get(1)?)),
    ).unwrap();
    assert!(published.0 < decision && decision <= event.1 && event.1 < published.1);
    let b = f.fresh_reader("q4-writer-b", 2);
    let c = f.fresh_reader("q4-writer-c", 3);
    let d = f.fresh_reader("q4-writer-d", 4);
    for caller in [&b, &c, &d] {
        assert_eq!(f.cursor(caller, thread), None);
    }
    let fresh_only = vec![(fresh.clone(), "new episode exact body".into())];
    for caller in [&b, &c, &d] {
        assert_eq!(
            f.lazy(caller),
            fresh_only,
            "cursorless binding gets current episode body, excluding old backlog"
        );
        assert_eq!(
            f.cursor(caller, thread),
            None,
            "passive page creates no floor row"
        );
    }
    let old_stream = vec![
        (old.clone(), "old episode unread body".into()),
        (fresh.clone(), "new episode exact body".into()),
    ];
    assert_eq!(f.cursor(&f.a, thread), Some(old_cursor));
    assert_eq!(
        f.lazy(&f.a),
        old_stream,
        "existing cursor preserves its old stream"
    );
    f.complete(&b, std::slice::from_ref(&fresh), "q4-writer-b-complete");
    let b_cursor = f
        .cursor(&b, thread)
        .expect("completion establishes actual binding progress");
    assert!(
        b_cursor >= f.sequence(&fresh),
        "current episode floor lets completion pass old gaps"
    );
    assert_eq!(f.cursor(&c, thread), None);
    assert_eq!(f.cursor(&d, thread), None);
    assert_eq!(
        f.lazy(&c),
        fresh_only,
        "shared display does not consume sibling body"
    );
    f.body(&fresh, "new episode exact body");
    f.advance(&c, std::slice::from_ref(&fresh), "q4-writer-c-display");
    let c_cursor = f
        .cursor(&c, thread)
        .expect("full public body read establishes actual binding progress");
    assert!(
        c_cursor >= f.sequence(&fresh),
        "AdvanceCursors passes excluded old gaps independently"
    );
    assert_eq!(f.cursor(&d, thread), None);
    assert_eq!(f.lazy(&d), fresh_only);
    assert_eq!(f.cursor(&f.a, thread), Some(old_cursor));
    assert_eq!(f.lazy(&f.a), old_stream);
    f.accept(thread, "q4-writer-accept-later");
    assert_eq!(
        f.cursor(&f.a, thread),
        Some(old_cursor),
        "accept preserves old stream cursor"
    );
    assert_eq!(f.cursor(&b, thread), Some(b_cursor));
    assert_eq!(f.cursor(&c, thread), Some(c_cursor));
    let d_floor = f
        .cursor(&d, thread)
        .expect("accept initializes missing cursor from current episode boundary");
    assert!(f.sequence(old) < d_floor && d_floor < f.sequence(&fresh));
    assert_eq!(
        f.lazy(&d),
        fresh_only,
        "accept keeps current episode body unread"
    );
    assert_eq!(f.lazy(&f.a), old_stream);
    let old_state: (String, i64) =
        f.db.query_row(
            "SELECT state,legacy_pending FROM lazy_recipients WHERE message_id=?1 AND seat_id='s2'",
            [old.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        old_state,
        ("pending".into(), 0),
        "old gap is retained, rather than erased to make fresh display pass"
    );
}

#[test]
fn q4_operator_orphan_episode_floor_admits_fresh_body_and_preserves_existing_stream() {
    let f = Q4WriterReaders::new();
    let thread = f.pair();
    f.accept(&thread, "q4-writer-initial-accept");
    let old_cursor = f
        .cursor(&f.a, &thread)
        .expect("SETUP: existing accepted reader cursor");
    let old = f.send(&thread, "old episode unread body", "q4-writer-old-body");
    assert!(old_cursor < f.sequence(&old));
    f.leave(&thread, &f.a, "q4-writer-recipient-leave");
    f.leave(&thread, &f.sender, "q4-writer-sender-leave");
    let states: (i64,i64) = f.db.query_row(
        "SELECT count(*),sum(state='left' AND voluntary_state='left') FROM memberships WHERE thread_id=?1",
        [thread.as_str()], |row| Ok((row.get(0)?,row.get(1)?)),
    ).unwrap();
    assert_eq!(
        states,
        (2, 2),
        "SETUP: both immutable endpoints legitimately left"
    );
    let invitation = f.operator_invite(&thread, "q4-writer-operator-invite");
    f.sender_join(&thread);
    assert_current_episode_readers(
        &f,
        &thread,
        &invitation,
        &old,
        old_cursor,
        "operator_orphan_invite",
    );
}

#[test]
fn q4_ordinary_service_episode_floor_admits_fresh_body_and_preserves_existing_stream() {
    let f = Q4WriterReaders::new();
    let thread = f.pair();
    f.accept(&thread, "q4-writer-initial-accept");
    let old_cursor = f
        .cursor(&f.a, &thread)
        .expect("SETUP: existing accepted reader cursor");
    let old = f.send(&thread, "old episode unread body", "q4-writer-old-body");
    assert!(old_cursor < f.sequence(&old));
    f.leave(&thread, &f.a, "q4-writer-recipient-leave");
    let joined: String =
        f.db.query_row(
            "SELECT state FROM memberships WHERE thread_id=?1 AND seat_id='s1'",
            [thread.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(joined, "joined", "SETUP: sender remains canonically joined");
    let invitation = f.ordinary_service_invite(&thread, "q4-writer-service-invite");
    assert_current_episode_readers(&f, &thread, &invitation, &old, old_cursor, "service_invite");
}

#[test]
fn q4_native_pending_episode_reused_by_native_and_service_writers_keeps_exact_floor_rows() {
    let f = Q4WriterReaders::new();
    let thread = f.pair();
    let old = f.send(
        &thread,
        "old duplicate-control backlog",
        "q4-writer-duplicate-old",
    );
    f.reject(&thread);
    let invitation = f.native_invite(&thread, "q4-writer-native-new-episode");
    let fresh = f.send(
        &thread,
        "fresh duplicate-control body",
        "q4-writer-duplicate-fresh",
    );
    let event = f.invitation_event(&invitation);
    assert!(f.sequence(&old) < event.0 && event.0 < f.sequence(&fresh));
    assert_eq!(f.invitation_row(&invitation).0, 2);
    let rows = f.floor_authority_rows();
    assert_eq!(
        f.native_invite(&thread, "q4-writer-native-new-episode"),
        invitation
    );
    assert_eq!(
        f.floor_authority_rows(),
        rows,
        "exact native operation replay leaves episode/floor unchanged"
    );
    assert_eq!(
        f.native_invite(&thread, "q4-writer-native-reuse-fresh-key"),
        invitation
    );
    assert_eq!(
        f.floor_authority_rows(),
        rows,
        "fresh native operation reuses immutable pending episode"
    );
    assert_eq!(
        f.ordinary_service_invite(&thread, "q4-writer-service-reuse-native"),
        invitation
    );
    assert_eq!(
        f.floor_authority_rows(),
        rows,
        "service writer reuses native episode without replacing floor or Lazy policy"
    );
    assert_eq!(
        f.ordinary_service_invite(&thread, "q4-writer-service-reuse-native"),
        invitation
    );
    assert_eq!(
        f.floor_authority_rows(),
        rows,
        "exact service replay keeps floor unchanged"
    );
    assert_eq!(
        f.ordinary_service_invite(&thread, "q4-writer-service-reuse-native-fresh-key"),
        invitation
    );
    assert_eq!(f.floor_authority_rows(), rows);
}

#[test]
fn q4_service_pending_episode_reused_by_service_and_native_writers_keeps_exact_floor_rows() {
    let f = Q4WriterReaders::new();
    let thread = f.pair();
    let old = f.send(
        &thread,
        "old duplicate-control backlog",
        "q4-writer-duplicate-old",
    );
    f.reject(&thread);
    let invitation = f.ordinary_service_invite(&thread, "q4-writer-service-new-episode");
    let fresh = f.send(
        &thread,
        "fresh duplicate-control body",
        "q4-writer-duplicate-fresh",
    );
    let event = f.invitation_event(&invitation);
    assert!(f.sequence(&old) < event.0 && event.0 < f.sequence(&fresh));
    assert_eq!(f.invitation_row(&invitation).0, 2);
    let rows = f.floor_authority_rows();
    assert_eq!(
        f.ordinary_service_invite(&thread, "q4-writer-service-new-episode"),
        invitation
    );
    assert_eq!(f.floor_authority_rows(), rows);
    assert_eq!(
        f.ordinary_service_invite(&thread, "q4-writer-service-reuse-fresh-key"),
        invitation
    );
    assert_eq!(
        f.floor_authority_rows(),
        rows,
        "fresh service operation reuses pending timed episode"
    );
    assert_eq!(
        f.native_invite(&thread, "q4-writer-native-reuse-service"),
        invitation
    );
    assert_eq!(
        f.floor_authority_rows(),
        rows,
        "native Lazy request preserves service's existing timed episode and floor"
    );
    assert_eq!(
        f.native_invite(&thread, "q4-writer-native-reuse-service"),
        invitation
    );
    assert_eq!(f.floor_authority_rows(), rows);
    assert_eq!(
        f.native_invite(&thread, "q4-writer-native-reuse-service-fresh-key"),
        invitation
    );
    assert_eq!(f.floor_authority_rows(), rows);
}

#[test]
fn q4_pair_refuses_managed_ownership_and_required_writer_without_changing_floor_authority() {
    let f = Q4WriterReaders::new();
    let thread = f.pair();
    let _message = f.send(
        &thread,
        "pending direct body survives refused writer",
        "q4-writer-refusal-body",
    );
    // Register the legitimate service identity before snapshotting refusal effects.
    let author = ServiceAuthorId::new(format!("graph:{:x}", sha2::Sha256::digest(b"i")));
    let gate = GraphGate(author.clone());
    let connection = gate.register("i", "q4-writer-boot", author).unwrap();
    let context = f.context();
    let mut writer = context.open_writer().unwrap();
    let rows = f.floor_authority_rows();
    let own = ServiceOperation::EnsureThread(EnsureManagedThread {
        thread: thread.clone(),
        topic: "must remain automatic".into(),
        goal: "must remain two endpoints".into(),
        operation: OperationId::new("q4-writer-refused-managed"),
    });
    assert_eq!(
        crate::store::service_controls::operate(
            &context,
            &mut writer,
            "i",
            own,
            &connection,
            &gate,
            &budget(),
            None
        )
        .unwrap_err()
        .code,
        ErrorCode::Unauthorized
    );
    assert_eq!(f.floor_authority_rows(), rows);
    let required = ServiceOperation::Invite(ServiceInvite {
        thread: thread.clone(),
        seat: f.a.seat.clone(),
        constraint: InvitationConstraint::Required,
        deadline_millis: Some(10_000),
        operation: OperationId::new("q4-writer-refused-required"),
    });
    assert_eq!(
        crate::store::service_controls::operate(
            &context,
            &mut writer,
            "i",
            required,
            &connection,
            &gate,
            &budget(),
            None
        )
        .unwrap_err()
        .code,
        ErrorCode::RequiredInvitationNeedsManagedThread
    );
    assert_eq!(
        f.floor_authority_rows(),
        rows,
        "refusal neither adopts pair nor attaches requirements nor changes episode/readers"
    );
}
