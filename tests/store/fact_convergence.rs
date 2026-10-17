//! Cross-kind convergence of the fact kinds (fact-kinds design Testing
//! "Convergence", "Deadline scope", "Archival", "Fence", "Oracle"; sections 13
//! and 15; ht-5fr.4.18): the scenarios that need several kind leaves at once,
//! run over two or three stores.
//!
//! Commands run through the store's own entry points with permits from
//! `issue_cooperative_permit` (the `Node::cmd_*` helpers below); facts a
//! scenario needs concurrent are authored through the kind helpers of the
//! leaf suites. Overdue checks use payload deadlines at or before the test
//! clock's fixed UTC reading (`SENT_AT`), which is an injected clock past
//! `deadline_at`.
//!
//! Every test ends in [`check_no_remote_delivery`]: no node delivers to or
//! wakes a seat homed elsewhere.

use herdr_threads::ports::{
    CooperativePermitRequest, DurableWorkAdmission, SendPreparationProgress, StorePort,
    WorkAdmission,
};
use herdr_threads::protocol::authority::{
    CallerClaim, CallerRole, Harness, LivenessFingerprint, MutationPermit, ObligationRef,
};
use herdr_threads::protocol::commands::{
    Accept, CheckIn, CheckInMode, CreateThread, DeliveryMode, EnsureDirectThread, Invite, Join,
    Leave, Reject, SendMessage, SetThreadName, SetTopic, ThreadMutation,
};
use herdr_threads::protocol::ids::{
    ExecutionId, HostTargetId, InvitationId, MessageId, NativeSessionId, OperationId,
    RetirementJobId, SeatId, ThreadId,
};
use herdr_threads::protocol::results::{ApiError, CommandResult, ErrorCode};
use herdr_threads::protocol::time::{Clock, MonoInstant, UtcMillis};
use herdr_threads::store::archival::{self, Runtime};
use herdr_threads::store::materialization::advance_work;
use herdr_threads::store::receipts::{self, ReceiptDueCursor};
use herdr_threads::store::{control, invitation_due, messages, schema, seats};
use herdr_threads::sync::facts::payload::ReceiptObligation;
use herdr_threads::test_support::journal_oracle::{
    replay_into_empty, replicated_projection, replicated_tables, strict_violations,
};
use herdr_threads::test_support::{
    assert_no_local_delivery_for_remote, fact_exchange, write_guard,
};

use crate::fact_convergence_seat::begin_retirement;
use crate::facts_invitation::CREATED_AT;
use crate::facts_membership::thread_of;
use crate::facts_message::{FIVE_MIN, SENT_AT, sent, spec};
use crate::facts_receipt::{drain, session_of, ship_origin};
use crate::facts_seat::{Node, budget, hold_off, hold_on, registry};
use crate::facts_thread::sync_all;

// ---- the command driver --------------------------------------------------------

/// A canonical UUID execution derived from the seat's tail (the shape
/// `Node::bind_commands` registers).
fn execution_of(seat: &SeatId) -> String {
    let tail: String = seat
        .as_str()
        .bytes()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
        .chars()
        .rev()
        .take(12)
        .collect();
    format!("00000000-0000-4000-8000-{tail:0>12}")
}

impl Node {
    /// The cooperative claim of `seat` bound by [`Node::bind_commands`].
    pub fn claim_of(&self, seat: &SeatId) -> CallerClaim {
        CallerClaim {
            host: herdr_threads::test_support::test_host_locator(),
            instance: self.id.to_string(),
            seat: seat.clone(),
            binding_generation: 1,
            role: CallerRole::TopLevel,
            harness: Harness::Codex,
            native_session: NativeSessionId::new(session_of(seat)),
            execution: ExecutionId::new(execution_of(seat)),
            target: HostTargetId::new(format!("p-{}", seat.as_str())),
            agent_id: None,
            liveness: Some(LivenessFingerprint::TEST),
        }
    }

    /// A cooperative permit for `claim`, issued the way the daemon issues one.
    pub fn permit_for(
        &self,
        claim: &CallerClaim,
        operation: &str,
        obligation: ObligationRef,
        payload_hash: [u8; 32],
        check_in_mode: Option<CheckInMode>,
    ) -> Result<MutationPermit, ApiError> {
        self.store.issue_cooperative_permit(
            CooperativePermitRequest {
                claim: claim.clone(),
                operation: OperationId::new(operation),
                obligation,
                payload_hash,
                check_in_mode,
            },
            &budget(),
        )
    }

    /// Runs `command` under a fresh writer with the write guard armed.
    fn with_writer<R>(
        &self,
        f: impl FnOnce(&mut rusqlite::Connection) -> Result<R, ApiError>,
    ) -> Result<R, ApiError> {
        let mut conn = self.raw();
        let _guard = write_guard::arm(&conn);
        f(&mut conn)
    }

    pub fn cmd_create_thread(
        &self,
        seat: &SeatId,
        topic: &str,
        operation: &str,
    ) -> Result<ThreadId, ApiError> {
        let command = CreateThread {
            name: None,
            topic: topic.into(),
            goal: "goal".into(),
            operation: OperationId::new(operation),
            claim: self.claim_of(seat),
        };
        let permit = self.permit_for(
            &command.claim,
            operation,
            ObligationRef::CheckIn(seat.clone()),
            control::cooperative_payload_hash("create_thread", &command)?,
            None,
        )?;
        match self.with_writer(|conn| {
            control::create_thread(self.ctx(), conn, &budget(), &command, permit)
        })? {
            CommandResult::ThreadCreated(thread) => Ok(thread),
            other => panic!("create_thread: {other:?}"),
        }
    }

    pub fn cmd_invite(
        &self,
        inviter: &SeatId,
        thread: &ThreadId,
        invitee: &SeatId,
        operation: &str,
    ) -> Result<InvitationId, ApiError> {
        let command = Invite {
            confirmation: None,
            thread: thread.clone(),
            seat: invitee.clone(),
            deadline_millis: Some(FIVE_MIN as u64),
            operation: OperationId::new(operation),
            claim: self.claim_of(inviter),
        };
        let permit = self.permit_for(
            &command.claim,
            operation,
            ObligationRef::Control(thread.clone()),
            control::cooperative_payload_hash("invite", &command)?,
            None,
        )?;
        match self.with_writer(|conn| {
            control::invite(self.ctx(), conn, &budget(), &command, permit, None)
        })? {
            CommandResult::Invitation(id) => Ok(id),
            other => panic!("invite: {other:?}"),
        }
    }

    pub fn cmd_accept(
        &self,
        seat: &SeatId,
        thread: &ThreadId,
        invitation: &InvitationId,
        operation: &str,
    ) -> Result<CommandResult, ApiError> {
        let command = Accept {
            thread: thread.clone(),
            operation: OperationId::new(operation),
            claim: self.claim_of(seat),
        };
        let permit = self.permit_for(
            &command.claim,
            operation,
            ObligationRef::Invitation(invitation.clone()),
            control::cooperative_payload_hash("accept", &command)?,
            None,
        )?;
        self.with_writer(|conn| control::accept(self.ctx(), conn, &budget(), &command, permit))
    }

    pub fn cmd_reject(
        &self,
        seat: &SeatId,
        thread: &ThreadId,
        invitation: &InvitationId,
        operation: &str,
    ) -> Result<CommandResult, ApiError> {
        let command = Reject {
            thread: thread.clone(),
            invitation: invitation.clone(),
            reason: "not now".into(),
            operation: OperationId::new(operation),
            claim: self.claim_of(seat),
        };
        let permit = self.permit_for(
            &command.claim,
            operation,
            ObligationRef::Invitation(invitation.clone()),
            control::cooperative_payload_hash("reject", &command)?,
            None,
        )?;
        self.with_writer(|conn| control::reject(self.ctx(), conn, &budget(), &command, permit))
    }

    pub fn cmd_join(
        &self,
        seat: &SeatId,
        thread: &ThreadId,
        operation: &str,
    ) -> Result<CommandResult, ApiError> {
        let command = Join {
            thread: thread.clone(),
            operation: OperationId::new(operation),
            claim: self.claim_of(seat),
        };
        let permit = self.permit_for(
            &command.claim,
            operation,
            ObligationRef::Control(thread.clone()),
            control::cooperative_payload_hash("join", &command)?,
            None,
        )?;
        self.with_writer(|conn| control::join(self.ctx(), conn, &budget(), &command, permit))
    }

    pub fn cmd_leave(
        &self,
        seat: &SeatId,
        thread: &ThreadId,
        operation: &str,
    ) -> Result<CommandResult, ApiError> {
        let command = Leave {
            thread: thread.clone(),
            operation: OperationId::new(operation),
            claim: self.claim_of(seat),
        };
        let permit = self.permit_for(
            &command.claim,
            operation,
            ObligationRef::Control(thread.clone()),
            control::cooperative_payload_hash("leave", &command)?,
            None,
        )?;
        self.with_writer(|conn| control::leave(self.ctx(), conn, &budget(), &command, permit))
    }

    pub fn cmd_set_topic(
        &self,
        seat: &SeatId,
        thread: &ThreadId,
        topic: &str,
        operation: &str,
    ) -> Result<CommandResult, ApiError> {
        let command = SetTopic {
            thread: thread.clone(),
            topic: topic.into(),
            operation: OperationId::new(operation),
            claim: self.claim_of(seat),
        };
        let permit = self.permit_for(
            &command.claim,
            operation,
            ObligationRef::Control(thread.clone()),
            control::cooperative_payload_hash("set_topic", &command)?,
            None,
        )?;
        self.with_writer(|conn| control::set_topic(self.ctx(), conn, &budget(), &command, permit))
    }

    pub fn cmd_set_thread_name(
        &self,
        seat: &SeatId,
        thread: &ThreadId,
        name: Option<&str>,
        operation: &str,
    ) -> Result<CommandResult, ApiError> {
        let command = SetThreadName {
            thread: thread.clone(),
            name: name.map(str::to_owned),
            operation: OperationId::new(operation),
            claim: self.claim_of(seat),
        };
        let permit = self.permit_for(
            &command.claim,
            operation,
            ObligationRef::Control(thread.clone()),
            control::cooperative_payload_hash("set_thread_name", &command)?,
            None,
        )?;
        self.with_writer(|conn| {
            control::set_thread_name(self.ctx(), conn, &budget(), &command, permit)
        })
    }

    fn thread_mutation(
        &self,
        seat: &SeatId,
        thread: &ThreadId,
        operation: &str,
        kind: &str,
        run: impl FnOnce(
            &herdr_threads::store::connection::StoreContext,
            &mut rusqlite::Connection,
            &ThreadMutation,
            MutationPermit,
        ) -> Result<CommandResult, ApiError>,
    ) -> Result<CommandResult, ApiError> {
        let command = ThreadMutation {
            thread: thread.clone(),
            operation: OperationId::new(operation),
            claim: self.claim_of(seat),
        };
        let permit = self.permit_for(
            &command.claim,
            operation,
            ObligationRef::Control(thread.clone()),
            control::cooperative_payload_hash(kind, &command)?,
            None,
        )?;
        self.with_writer(|conn| run(self.ctx(), conn, &command, permit))
    }

    pub fn cmd_archive(
        &self,
        seat: &SeatId,
        thread: &ThreadId,
        operation: &str,
    ) -> Result<CommandResult, ApiError> {
        self.thread_mutation(seat, thread, operation, "archive", |ctx, conn, c, p| {
            control::archive(ctx, conn, &budget(), c, p)
        })
    }

    pub fn cmd_reopen(
        &self,
        seat: &SeatId,
        thread: &ThreadId,
        operation: &str,
    ) -> Result<CommandResult, ApiError> {
        self.thread_mutation(seat, thread, operation, "reopen", |ctx, conn, c, p| {
            control::reopen(ctx, conn, &budget(), c, p)
        })
    }

    pub fn cmd_ensure_direct(
        &self,
        seat: &SeatId,
        recipient: &SeatId,
        operation: &str,
    ) -> Result<CommandResult, ApiError> {
        let command = EnsureDirectThread {
            creation_policy: None,
            recipient: recipient.clone(),
            operation: OperationId::new(operation),
            claim: self.claim_of(seat),
        };
        let permit = self.permit_for(
            &command.claim,
            operation,
            ObligationRef::CheckIn(seat.clone()),
            control::cooperative_payload_hash("ensure_direct_thread", &command)?,
            None,
        )?;
        self.with_writer(|conn| {
            control::ensure_direct_thread(self.ctx(), conn, &budget(), &command, permit, None)
        })
    }

    /// The real send, prepared to ready (the unguarded preparation phase) and
    /// not yet published.
    pub fn prepare_send(
        &self,
        seat: &SeatId,
        thread: &ThreadId,
        mode: DeliveryMode,
        operation: &str,
    ) -> Result<SendMessage, ApiError> {
        let request = self.prepare_request(seat, thread, mode, operation);
        self.prepare_steps(&request)?;
        Ok(request)
    }

    pub fn prepare_request(
        &self,
        seat: &SeatId,
        thread: &ThreadId,
        mode: DeliveryMode,
        operation: &str,
    ) -> SendMessage {
        SendMessage {
            delivery_mode: mode,
            priority: None,
            user_intent: None,
            thread: thread.clone(),
            body: format!("body of {operation}"),
            direct_recipient: None,
            invited_recipients: vec![],
            deadline_millis: None,
            operation: OperationId::new(operation),
            claim: self.claim_of(seat),
            relays_user: false,
        }
    }

    pub fn prepare_steps(&self, request: &SendMessage) -> Result<(), ApiError> {
        let mut conn = self.raw();
        for _ in 0..10_000 {
            let step = messages::prepare_send_step(
                self.ctx(),
                &mut conn,
                request,
                messages::MessageLimits::default(),
                &budget(),
                DurableWorkAdmission { max_units: 16 },
            )?;
            if !matches!(step, SendPreparationProgress::More { .. }) {
                break;
            }
        }
        Ok(())
    }

    /// Publishes a prepared send with the write guard armed.
    pub fn publish_prepared(&self, request: &SendMessage) -> Result<MessageId, ApiError> {
        let mut conn = self.raw();
        let _guard = write_guard::arm(&conn);
        let mut permit = self.permit_for(
            &request.claim,
            request.operation.as_str(),
            ObligationRef::Control(request.thread.clone()),
            schema::canonical_digest(&messages::send_payload(request))?,
            None,
        )?;
        match messages::publish_send(
            self.ctx(),
            &mut conn,
            request,
            &mut permit,
            &budget(),
            || 65_536,
        )? {
            CommandResult::MessageSent(message) => Ok(message),
            other => panic!("send: {other:?}"),
        }
    }

    /// The real send: prepare to ready, then publish. Returns the message.
    pub fn cmd_send(
        &self,
        seat: &SeatId,
        thread: &ThreadId,
        mode: DeliveryMode,
        operation: &str,
    ) -> Result<MessageId, ApiError> {
        let request = self.prepare_send(seat, thread, mode, operation)?;
        self.publish_prepared(&request)
    }

    /// An ACK send (`--require-ack`, p0) naming `acks` as the explicit ACK
    /// recipients: under D9 only they owe a receipt.
    pub fn cmd_send_ack(
        &self,
        seat: &SeatId,
        thread: &ThreadId,
        acks: &[&SeatId],
        operation: &str,
    ) -> Result<MessageId, ApiError> {
        let mut request = self.prepare_request(seat, thread, DeliveryMode::Ordinary, operation);
        request.invited_recipients = acks.iter().map(|seat| (*seat).clone()).collect();
        self.prepare_steps(&request)?;
        self.publish_prepared(&request)
    }

    /// Whether `seat` can be named by a check-in permit.
    pub fn check_in_permit(&self, seat: &SeatId) -> Result<MutationPermit, ApiError> {
        let command = CheckIn {
            mode: CheckInMode::Current,
            claim: self.claim_of(seat),
            operation: OperationId::new("check-in-op"),
            lineage_parent: None,
        };
        self.permit_for(
            &command.claim,
            "check-in-op",
            ObligationRef::CheckIn(seat.clone()),
            schema::canonical_digest(&seats::check_in_payload(&command))?,
            Some(CheckInMode::Current),
        )
    }

    /// Runs every unfinished retirement job to completion.
    pub fn finish_retirements(&self) {
        let ids = self.query("SELECT id FROM retirements WHERE status<>'complete'", |r| {
            r.get::<_, String>(0)
        });
        let mut conn = self.raw();
        for id in ids {
            for _ in 0..50 {
                let progress = control::advance_retirement(
                    self.ctx(),
                    &mut conn,
                    RetirementJobId::new(id.clone()),
                    WorkAdmission::Background,
                    &budget(),
                )
                .unwrap();
                if progress.complete {
                    break;
                }
            }
        }
    }

    /// One receipt deadline scan; the number of overdue warnings it wrote.
    pub fn scan_receipts(&self) -> u16 {
        let mut conn = self.raw();
        let mut cursor = ReceiptDueCursor::default();
        receipts::scan_due(self.ctx(), &mut conn, 20, &mut cursor)
            .unwrap()
            .warnings
    }

    /// One invitation deadline scan; the number of overdue warnings it wrote.
    pub fn scan_invitations(&self) -> u16 {
        let mut conn = self.raw();
        invitation_due::scan_invitation_due_batch(self.ctx(), &mut conn, None, 10)
            .unwrap()
            .warnings_added
    }

    /// The overdue warnings on the receipt and invitation families.
    pub fn overdue_total(&self) -> i64 {
        self.count("SELECT count(*) FROM messages WHERE kind='warn' AND event_key LIKE 'overdue:%'")
    }

    /// The overdue warning events written for `message`'s receipts.
    pub fn receipt_overdue(&self, message: &MessageId) -> i64 {
        self.count(&format!(
            "SELECT count(*) FROM messages WHERE kind='warn' AND source_message_id='{}'",
            message.as_str()
        ))
    }

    /// Runs the message's send job when it has an unfinished one (a lazy
    /// message has none; a job another pass finished is not run twice).
    pub fn drive_if_any(&self, message: &MessageId) {
        if self.count(&format!(
            "SELECT count(*) FROM work_jobs WHERE id='work:send:{}' AND status IN ('pending','failed')",
            message.as_str()
        )) > 0
        {
            self.drive_send(message);
        }
    }

    pub fn receipt_state_of(&self, message: &MessageId, seat: &SeatId) -> Option<String> {
        self.receipt(message, seat).map(|r| r.0)
    }
}

fn check_no_remote_delivery(nodes: &[&Node]) {
    for node in nodes {
        assert_no_local_delivery_for_remote(&node.raw());
    }
}

// ---- the world ------------------------------------------------------------------

/// A (the author, owner of `thread`) and B (hosts `agent`, `guest` and
/// `retiree`), both command-capable. `author`, `agent` and `retiree` are
/// joined to `thread` on both nodes. `retiree` is enrolled but not bound for
/// commands, so its home node can retire it with a closure proof.
struct World {
    a: Node,
    b: Node,
    thread: ThreadId,
    author: SeatId,
    agent: SeatId,
    guest: SeatId,
    retiree: SeatId,
}

fn world(seed: u8) -> World {
    let reg = registry();
    let a = Node::open_commands(seed, u64::from(seed), reg);
    let b = Node::open_commands(seed + 1, u64::from(seed + 1), reg);
    let (thread, author) = thread_of(&a, "shared");
    a.bind_commands(&author);
    a.publish(&author).unwrap();
    a.joined(&thread, &author, 1).unwrap();
    let (agent, guest) = (b.local_seat(), b.local_seat());
    for seat in [&agent, &guest] {
        b.bind_commands(seat);
        b.publish(seat).unwrap();
    }
    let retiree = b.local_seat();
    b.enroll(&retiree, "codex", "session-retiree").unwrap();
    sync_all(&[&a, &b]);
    b.joined(&thread, &agent, 1).unwrap();
    b.joined(&thread, &retiree, 1).unwrap();
    sync_all(&[&a, &b]);
    World {
        a,
        b,
        thread,
        author,
        agent,
        guest,
        retiree,
    }
}

impl World {
    /// A authors an ordinary message to `agent` sent at `sent_at`; its
    /// receipt deadline is `sent_at + FIVE_MIN`.
    fn send_at(&self, sent_at: i64) -> MessageId {
        self.send_to(&self.agent, sent_at)
    }

    fn send_to(&self, recipient: &SeatId, sent_at: i64) -> MessageId {
        let message = self.a.stage_prep(
            &self.thread,
            "ordinary",
            &[(recipient, FIVE_MIN, true)],
            &[],
        );
        let mut payload = sent(
            &message,
            &self.thread,
            &self.author,
            DeliveryMode::Ordinary,
            FIVE_MIN,
            vec![spec(
                recipient,
                ReceiptObligation::Ack,
                Some(sent_at + FIVE_MIN),
            )],
        );
        payload.sent_at = sent_at;
        self.a.author_sent(&payload).unwrap();
        message
    }

    /// A message whose receipt deadline the test clock is already past.
    fn send_overdue(&self) -> MessageId {
        self.send_at(SENT_AT - 2 * FIVE_MIN)
    }

    /// A invites `invitee` with a deadline of `deadline_at`; returns the id.
    fn invite_with_deadline(
        &self,
        thread: &ThreadId,
        invitee: &SeatId,
        deadline_at: i64,
    ) -> InvitationId {
        let mut payload = self.a.created_payload(thread, &self.author, invitee);
        // A deadline is always created_at + duration_ms: move the creation.
        payload.created_at = deadline_at - payload.duration_ms;
        payload.deadline_at = deadline_at;
        self.a.author_created(&payload).unwrap();
        payload.invitation
    }

    /// Both nodes drive the message's send job.
    fn deliver(&self, message: &MessageId) {
        sync_all(&[&self.a, &self.b]);
        self.a.drive_send(message);
        self.b.drive_send(message);
    }

    fn done(&self) {
        check_no_remote_delivery(&[&self.a, &self.b]);
    }
}

// ---- retirement against sends ----------------------------------------------------

#[test]
fn retirement_vs_send_both_orders_recipient_retired_overdue_only_on_a() {
    let w = world(60);
    let reg = registry();
    // Two bystanders ingest the concurrent logs in opposite orders.
    let (x, y) = (
        Node::open_commands(70, 70, reg),
        Node::open_commands(71, 71, reg),
    );
    sync_all(&[&w.a, &w.b, &x, &y]);

    // Concurrent: A sends to the retiree (deadline already past the clock) and
    // its worker publishes the receipt for the remote seat, while B retires
    // that seat.
    let message = w.send_to(&w.retiree, SENT_AT - 2 * FIVE_MIN);
    w.a.drive_send(&message);
    assert_eq!(
        w.a.receipt_state_of(&message, &w.retiree).as_deref(),
        Some("pending")
    );
    begin_retirement(&w.b, &w.retiree).unwrap();
    // X sees the message first, Y the retirement first; B (retirement, then
    // the message) and A (message, then the retirement) cross in one sync.
    for source in [&w.a, &w.b] {
        fact_exchange::pull(&x.store, &source.store);
    }
    for source in [&w.b, &w.a] {
        fact_exchange::pull(&y.store, &source.store);
    }
    fact_exchange::sync(&w.a.store, &w.b.store);
    let nodes = [&w.a, &w.b, &x, &y];
    for node in nodes {
        node.drive_if_any(&message);
        node.scan_receipts();
        node.finish_retirements();
        node.drive_if_any(&message);
    }
    for node in nodes {
        assert_eq!(
            node.receipt_state_of(&message, &w.retiree).as_deref(),
            Some("recipient_retired"),
            "{:?}: the receipt ends recipient_retired",
            node.id
        );
        assert_eq!(
            node.receipt_overdue(&message),
            i64::from(node.id == w.a.id),
            "{:?}: only the author's node records the overdue",
            node.id
        );
    }
    assert_eq!(w.a.overdue_total(), 1);
    for node in [&w.b, &x, &y] {
        assert_eq!(node.overdue_total(), 0, "{:?}", node.id);
    }
    check_no_remote_delivery(&nodes);
}

// ---- the deadline lane's scope across nodes ----------------------------------------

#[test]
fn deadline_scope_receipts_across_nodes() {
    let w = world(62);
    let message = w.send_overdue();
    w.deliver(&message);
    assert_eq!(
        w.b.receipt_state_of(&message, &w.agent).as_deref(),
        Some("pending")
    );
    assert_eq!(
        w.a.receipt_state_of(&message, &w.agent).as_deref(),
        Some("pending")
    );

    assert_eq!(
        w.b.scan_receipts(),
        0,
        "the recipient's node inserts nothing"
    );
    assert_eq!(w.b.overdue_total(), 0);
    assert_eq!(
        w.a.scan_receipts(),
        1,
        "the sender's node records one overdue for the remote recipient"
    );
    assert_eq!(w.a.receipt_overdue(&message), 1);
    assert_eq!(w.a.scan_receipts(), 0, "and records it once");
    w.done();
}

#[test]
fn deadline_scope_invitations_across_nodes() {
    let w = world(64);
    let id = w.invite_with_deadline(&w.thread, &w.guest, CREATED_AT - 1);
    sync_all(&[&w.a, &w.b]);
    assert_eq!(w.b.state_of(&id).as_deref(), Some("pending"));
    assert_eq!(w.a.state_of(&id).as_deref(), Some("pending"));

    assert_eq!(
        w.b.scan_invitations(),
        0,
        "the invitee's node inserts nothing"
    );
    assert_eq!(w.b.overdue_warnings(), 0);
    assert_eq!(
        w.a.scan_invitations(),
        1,
        "the inviter's node records one overdue for the remote invitee"
    );
    assert_eq!(w.a.overdue_warnings(), 1);
    assert_eq!(w.a.scan_invitations(), 0);
    w.done();
}

#[test]
fn late_ack_overdue_only_on_sender() {
    let w = world(66);
    let message = w.send_overdue();
    w.deliver(&message);
    // B acks after the deadline through the real command.
    let CommandResult::Acknowledged(result) =
        w.b.ack(&w.agent, "op-late-ack", &[&message], false)
            .unwrap()
    else {
        panic!("an ACK result");
    };
    assert_eq!(result.acknowledged, vec![message.clone()]);
    fact_exchange::sync(&w.a.store, &w.b.store);
    for node in [&w.a, &w.b] {
        assert_eq!(
            node.receipt_state_of(&message, &w.agent).as_deref(),
            Some("acked"),
            "{:?}",
            node.id
        );
    }
    assert_eq!(
        w.a.receipt_overdue(&message),
        1,
        "the sender's node records the late overdue"
    );
    assert_eq!(
        w.b.receipt_overdue(&message),
        0,
        "the acker's node does not"
    );
    assert_eq!(w.a.overdue_total(), 1);
    assert_eq!(w.b.overdue_total(), 0);
    w.done();
}

#[test]
fn late_answer_overdue_only_on_inviter() {
    let w = world(68);
    let id = w.invite_with_deadline(&w.thread, &w.guest, CREATED_AT - 1);
    sync_all(&[&w.a, &w.b]);
    // The answer carries `ANSWERED_AT`, after the deadline `CREATED_AT - 1`.
    w.b.author_answered(&w.b.accept_payload(&w.thread, &id, &w.guest))
        .unwrap();
    fact_exchange::sync(&w.a.store, &w.b.store);
    for node in [&w.a, &w.b] {
        assert_eq!(
            node.state_of(&id).as_deref(),
            Some("accepted"),
            "{:?}",
            node.id
        );
    }
    assert_eq!(w.a.overdue_warnings(), 1, "the inviter's node records it");
    assert_eq!(w.b.overdue_warnings(), 0, "the invitee's node does not");
    w.done();
}

#[test]
fn retirement_job_on_b_warns_nothing_for_a_messages() {
    let w = world(72);
    let message = w.send_to(&w.retiree, SENT_AT - 2 * FIVE_MIN);
    let second = w.a.create_thread(&w.author, "second", None).unwrap();
    w.a.joined(&second, &w.author, 1).unwrap();
    sync_all(&[&w.a, &w.b]);
    let id = w.invite_with_deadline(&second, &w.retiree, CREATED_AT - 1);
    w.deliver(&message);
    assert_eq!(
        w.b.receipt_state_of(&message, &w.retiree).as_deref(),
        Some("pending")
    );
    assert_eq!(w.b.state_of(&id).as_deref(), Some("pending"));

    // B retires the seat; its job settles A's message and A's invitation.
    begin_retirement(&w.b, &w.retiree).unwrap();
    w.b.finish_retirements();
    assert_eq!(
        w.b.receipt_state_of(&message, &w.retiree).as_deref(),
        Some("recipient_retired")
    );
    assert_eq!(w.b.state_of(&id).as_deref(), Some("recipient_retired"));
    assert_eq!(w.b.overdue_total(), 0, "B warns nothing for A's items");
    assert_eq!(w.b.receipt_overdue(&message), 0);

    // A's replicated retirement job runs late: A is the node that warns.
    fact_exchange::sync(&w.a.store, &w.b.store);
    w.a.finish_retirements();
    assert_eq!(w.a.receipt_overdue(&message), 1, "A records the overdue");
    assert_eq!(w.a.overdue_warnings(), 1);
    assert_eq!(w.b.overdue_total(), 0);
    w.done();
}

// ---- invitations of remote seats ----------------------------------------------------

#[test]
fn invite_remote_seat_wake_only_on_b() {
    let w = world(74);
    let id =
        w.a.cmd_invite(&w.author, &w.thread, &w.guest, "op-invite-guest")
            .unwrap();
    assert!(
        w.a.attention(&w.guest).is_none(),
        "A wakes nobody for the remote seat"
    );
    fact_exchange::sync(&w.a.store, &w.b.store);
    assert_eq!(w.b.state_of(&id).as_deref(), Some("pending"));
    assert_eq!(w.a.state_of(&id).as_deref(), Some("pending"));
    assert!(
        w.b.attention(&w.guest).is_some(),
        "B holds the invitation wake_work for its local seat"
    );
    assert_eq!(w.b.wake_seats(), vec![w.guest.as_str().to_owned()]);
    assert!(
        w.a.wake_seats().is_empty() && w.a.attention(&w.guest).is_none(),
        "A holds none for the remote seat"
    );
    w.done();
}

#[test]
fn invite_and_pair_never_referenced_seat_on_b() {
    let reg = registry();
    let a = Node::open_commands(76, 76, reg);
    let b = Node::open_commands(77, 77, reg);
    let (thread, author) = thread_of(&a, "shared");
    a.bind_commands(&author);
    a.publish(&author).unwrap();
    a.joined(&thread, &author, 1).unwrap();
    // B enrolls a seat no fact has referenced.
    let seat = b.local_seat();
    b.bind_commands(&seat);
    b.enroll(&seat, "codex", "session-fresh").unwrap();
    fact_exchange::sync(&a.store, &b.store);
    let row = a.seat(&seat).expect("A lists the seat");
    assert_eq!((row.0.as_str(), row.1, row.3), ("remote", false, true));

    // A invites it: B holds the pending invitation and its wake_work.
    let id = a
        .cmd_invite(&author, &thread, &seat, "op-invite-fresh")
        .unwrap();
    fact_exchange::sync(&a.store, &b.store);
    assert_eq!(b.state_of(&id).as_deref(), Some("pending"));
    assert!(b.attention(&seat).is_some(), "B wakes its seat");
    assert!(a.attention(&seat).is_none());

    // A opens a direct pair with it.
    let CommandResult::ThreadCreated(pair) = a
        .cmd_ensure_direct(&author, &seat, "op-pair-fresh")
        .unwrap()
    else {
        panic!("a direct-pair result");
    };
    assert!(pair.as_str().starts_with('d'));
    fact_exchange::sync(&a.store, &b.store);
    for node in [&a, &b] {
        assert!(
            node.thread_row(&pair).is_some(),
            "{:?} holds the pair",
            node.id
        );
        assert!(node.membership(&pair, &author).is_some());
        assert!(node.membership(&pair, &seat).is_some());
    }
    assert_eq!(
        a.membership(&pair, &author).map(|m| m.0),
        b.membership(&pair, &author).map(|m| m.0)
    );
    assert_eq!(
        a.membership(&pair, &seat).map(|m| m.0),
        b.membership(&pair, &seat).map(|m| m.0)
    );
    // B hosts the seat, so B authored its auto-join: both nodes hold it joined.
    for node in [&a, &b] {
        assert_eq!(
            node.membership(&pair, &seat).map(|m| m.0).as_deref(),
            Some("joined"),
            "{:?}",
            node.id
        );
    }
    check_no_remote_delivery(&[&a, &b]);
}

// ---- a remote seat cannot be occupied --------------------------------------------------

#[test]
fn remote_seat_not_occupiable_on_a() {
    let w = world(78);
    let message = w.send_at(SENT_AT);
    w.deliver(&message);
    let invitation = w.invite_with_deadline(&w.thread, &w.guest, SENT_AT + FIVE_MIN);
    sync_all(&[&w.a, &w.b]);
    let remote = &w.guest;
    assert_eq!(w.a.seat(remote).unwrap().0, "remote");
    let journal_a = w.a.journal();
    let journal_b = w.b.journal();
    let projection_b = w.b.all_projection();

    let refused = |error: ApiError, what: &str| {
        assert_eq!(error.code, ErrorCode::CallerUnverified, "{what}: {error:?}");
        assert_eq!(error.detail, "cooperative seat is not local", "{what}");
    };
    let claim = w.a.claim_of(remote);
    let permit = |operation: &str, obligation: ObligationRef, hash: [u8; 32]| {
        w.a.permit_for(&claim, operation, obligation, hash, None)
    };
    // send
    let request = SendMessage {
        delivery_mode: DeliveryMode::Lazy,
        priority: None,
        user_intent: None,
        thread: w.thread.clone(),
        body: "x".into(),
        direct_recipient: None,
        invited_recipients: vec![],
        deadline_millis: None,
        operation: OperationId::new("op-send-remote"),
        claim: claim.clone(),
        relays_user: false,
    };
    refused(
        permit(
            "op-send-remote",
            ObligationRef::Control(w.thread.clone()),
            schema::canonical_digest(&messages::send_payload(&request)).unwrap(),
        )
        .map(|_| ())
        .unwrap_err(),
        "send",
    );
    // ack
    let ack = herdr_threads::protocol::commands::Ack {
        messages: vec![message.clone()],
        operation: OperationId::new("op-ack-remote"),
        claim: claim.clone(),
    };
    refused(
        permit(
            "op-ack-remote",
            ObligationRef::CheckIn(remote.clone()),
            schema::canonical_digest(&receipts::ack_payload(&ack)).unwrap(),
        )
        .map(|_| ())
        .unwrap_err(),
        "ack",
    );
    // accept
    let accept = Accept {
        thread: w.thread.clone(),
        operation: OperationId::new("op-accept-remote"),
        claim: claim.clone(),
    };
    refused(
        permit(
            "op-accept-remote",
            ObligationRef::Invitation(invitation.clone()),
            control::cooperative_payload_hash("accept", &accept).unwrap(),
        )
        .map(|_| ())
        .unwrap_err(),
        "accept",
    );
    // check-in
    refused(
        w.a.check_in_permit(remote).map(|_| ()).unwrap_err(),
        "check-in",
    );

    // Nothing was authored or changed anywhere.
    assert_eq!(w.a.journal(), journal_a, "A's journal gained no entry");
    fact_exchange::sync(&w.a.store, &w.b.store);
    assert_eq!(w.a.journal(), journal_a, "and none after a sync");
    assert_eq!(w.b.journal(), journal_b, "B authored nothing either");
    assert_eq!(w.b.all_projection(), projection_b, "B's state is unchanged");
    w.done();
}

// ---- the oracle's scripted workload ------------------------------------------------------

/// A fixed clock for the replay's workers.
struct WorkerClock;

impl Clock for WorkerClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(SENT_AT)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(1)
    }
}

/// Runs the node-local workers a live node has run by now on `store`: the
/// send jobs that project the receipts, then the retirement jobs. The
/// journal replay applies facts only; the receipt rows and the closed
/// memberships a live node holds are these workers' output, so the replay
/// runs them before the projections are compared.
pub fn run_workers(store: &herdr_threads::store::SqliteStore) {
    let ctx = store.context();
    let mut conn = ctx.open_writer().unwrap();
    let jobs: Vec<String> = conn
        .prepare("SELECT id FROM work_jobs WHERE status IN ('pending','failed') ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for job in jobs {
        for _ in 0..64 {
            let progress = advance_work(
                &mut conn,
                &job,
                DurableWorkAdmission { max_units: 16 },
                &budget(),
                &WorkerClock,
            )
            .unwrap();
            if !progress.has_more {
                break;
            }
        }
    }
    let retirements: Vec<String> = conn
        .prepare("SELECT id FROM retirements WHERE status<>'complete' ORDER BY ordinal")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for id in retirements {
        for _ in 0..50 {
            let progress = control::advance_retirement(
                ctx,
                &mut conn,
                RetirementJobId::new(id.clone()),
                WorkAdmission::Background,
                &budget(),
            )
            .unwrap();
            if progress.complete {
                break;
            }
        }
    }
}

#[test]
fn register_events_carry_the_actor_on_both_nodes() {
    let w = world(90);
    w.a.cmd_set_topic(&w.author, &w.thread, "riddles", "op-topic")
        .unwrap();
    w.a.cmd_set_thread_name(&w.author, &w.thread, Some("tea"), "op-name")
        .unwrap();
    w.a.cmd_archive(&w.author, &w.thread, "op-archive").unwrap();
    w.a.cmd_reopen(&w.author, &w.thread, "op-reopen").unwrap();
    sync_all(&[&w.a, &w.b]);
    for (label, node) in [("A (local)", &w.a), ("B (replicated)", &w.b)] {
        for (field, actions) in [
            ("topic", vec!["set_topic"]),
            ("name", vec!["set_thread_name"]),
            ("archived", vec!["archive", "reopen"]),
        ] {
            let events = node.register_event_json(&w.thread, field);
            let seen: Vec<_> = events
                .iter()
                .map(|e| e["action"].as_str().unwrap().to_owned())
                .collect();
            assert_eq!(seen, actions, "{label} {field}");
            for event in &events {
                assert_eq!(
                    event["actor_seat"].as_str(),
                    Some(w.author.as_str()),
                    "{label} {field}: {event}"
                );
            }
        }
    }
}

#[test]
fn oracle_scripted_workload_strict() {
    let w = world(80);
    let reg = registry();
    let _ = reg;
    // create + topic
    let t2 =
        w.a.cmd_create_thread(&w.author, "second", "op-create-2")
            .unwrap();
    w.a.cmd_set_topic(&w.author, &w.thread, "renamed", "op-topic")
        .unwrap();
    // pair
    let CommandResult::ThreadCreated(pair) =
        w.a.cmd_ensure_direct(&w.author, &w.agent, "op-pair")
            .unwrap()
    else {
        panic!("a direct-pair result");
    };
    // invite, accept, reject
    let to_accept =
        w.a.cmd_invite(&w.author, &t2, &w.agent, "op-invite-accept")
            .unwrap();
    let to_reject =
        w.a.cmd_invite(&w.author, &w.thread, &w.guest, "op-invite-reject")
            .unwrap();
    sync_all(&[&w.a, &w.b]);
    w.b.cmd_accept(&w.agent, &t2, &to_accept, "op-accept")
        .unwrap();
    w.b.cmd_reject(&w.guest, &w.thread, &to_reject, "op-reject")
        .unwrap();
    sync_all(&[&w.a, &w.b]);
    // sends: lazy and ordinary from A, a reply from B, both ACKed
    let lazy =
        w.a.cmd_send(&w.author, &w.thread, DeliveryMode::Lazy, "op-send-lazy")
            .unwrap();
    let ordinary =
        w.a.cmd_send_ack(&w.author, &w.thread, &[&w.agent], "op-send-ordinary")
            .unwrap();
    w.deliver(&ordinary);
    w.a.drive_if_any(&lazy);
    w.b.drive_if_any(&lazy);
    w.b.ack(&w.agent, "op-ack", &[&ordinary], false).unwrap();
    let reply =
        w.b.cmd_send(&w.agent, &w.thread, DeliveryMode::Lazy, "op-send-reply")
            .unwrap();
    sync_all(&[&w.a, &w.b]);
    w.a.drive_if_any(&reply);
    // archive, then retire
    w.a.cmd_archive(&w.author, &t2, "op-archive").unwrap();
    begin_retirement(&w.b, &w.retiree).unwrap();
    sync_all(&[&w.a, &w.b]);
    sync_all(&[&w.a, &w.b]);
    w.a.finish_retirements();
    w.b.finish_retirements();

    assert_eq!(w.a.thread_row(&t2).map(|r| r.2), Some(true));
    assert!(w.a.thread_row(&pair).is_some());
    let specs = replicated_tables();
    for node in [&w.a, &w.b] {
        let failed: Vec<_> = node
            .journal()
            .into_iter()
            .filter(|(_, state, _)| state != "applied")
            .collect();
        assert!(failed.is_empty(), "{:?}: {failed:?}", node.id);
        let replay = replay_into_empty(&node.store);
        run_workers(&replay);
        assert_eq!(
            strict_violations(&node.store, &replay, &specs),
            Vec::<String>::new(),
            "{:?}",
            node.id
        );
        let live = replicated_projection(&node.store, replay.node(), &specs);
        let replayed = replicated_projection(&replay, replay.node(), &specs);
        assert_eq!(live, replayed, "{:?} matches its replay", node.id);
        // The comparison is not vacuous: the workload filled the tables.
        for table in [
            "threads",
            "direct_threads",
            "seats",
            "memberships",
            "invitations",
            "invitation_rejections",
            "messages",
            "prepared_recipients",
            "receipt_state",
        ] {
            assert!(
                live.iter()
                    .any(|(label, rows)| label.starts_with(table) && !rows.is_empty()),
                "{:?}: {table} projects rows",
                node.id
            );
        }
    }
    // The workload exercised every kind it names.
    for kind in [
        "seat.created",
        "thread.created",
        "thread.register",
        "thread.direct",
        "membership.joined",
        "invitation.created",
        "invitation.answered",
        "message.sent",
        "receipt.settled",
        "seat.retired",
    ] {
        assert!(
            w.a.count_kind(kind) + w.b.count_kind(kind) > 0,
            "the workload covers {kind}"
        );
    }
    let _ = (&lazy, &reply);
    w.done();
}

// ---- archival with remote members ---------------------------------------------------

/// Ticks the archival lane of `node` once a minute from `from` through
/// `until` (monotonic ms), with a one-minute quiet period.
fn tick_archival(node: &Node, from: i64, until: i64) {
    let instance = node.id.to_string();
    let conn = node.raw();
    let hosts = herdr_threads::test_support::test_host_evidence(0, true);
    for at in (from..=until).step_by(60_000) {
        let rt = Runtime {
            boot: "boot".into(),
            mono: at,
            utc: UtcMillis(SENT_AT + at),
            after_ms: 60_000,
            hosts: hosts.clone(),
            legacy_source: Some("covered".into()),
        };
        for _ in 0..30 {
            if !archival::advance_with(&conn, Some(node.ctx()), &instance, &rt)
                .unwrap()
                .has_more
            {
                break;
            }
        }
    }
}

fn archived(node: &Node, thread: &ThreadId) -> bool {
    node.thread_row(thread).expect("the thread").2
}

impl World {
    /// A lazy message B's `agent` authors into `thread` for the thread's other
    /// members (`author` on A, `retiree` on B).
    fn b_posts_lazy(&self, thread: &ThreadId) -> MessageId {
        let message = self.b.stage_prep(thread, "lazy", &[], &[&self.retiree]);
        let payload = sent(
            &message,
            thread,
            &self.agent,
            DeliveryMode::Lazy,
            0,
            vec![
                spec(&self.author, ReceiptObligation::None, None),
                spec(&self.retiree, ReceiptObligation::None, None),
            ],
        );
        self.b.author_sent(&payload).unwrap();
        message
    }
}

#[test]
fn archival_defers_until_remote_member_retired_then_replicates() {
    let w = world(82);
    // A quiet thread on A whose only joined member is B's retiree.
    let creator = w.a.local_seat();
    let quiet = w.a.create_thread(&creator, "quiet", None).unwrap();
    sync_all(&[&w.a, &w.b]);
    w.b.joined(&quiet, &w.retiree, 1).unwrap();
    sync_all(&[&w.a, &w.b]);
    assert_eq!(
        w.a.membership(&quiet, &w.retiree).map(|m| m.0).as_deref(),
        Some("joined")
    );

    // The quiet period passes ten times over: a live remote member defers it.
    tick_archival(&w.a, 0, 600_000);
    assert!(
        !archived(&w.a, &quiet),
        "a joined remote member defers archival"
    );

    // The member's retirement applies on A: the thread archives after a
    // fresh quiet period, and the decision replicates as a register.
    begin_retirement(&w.b, &w.retiree).unwrap();
    fact_exchange::sync(&w.a.store, &w.b.store);
    assert_eq!(w.a.seat(&w.retiree).unwrap().0, "retired");
    w.a.finish_retirements();
    tick_archival(&w.a, 660_000, 1_800_000);
    assert!(archived(&w.a, &quiet), "archived once the member retired");
    let registers: Vec<_> =
        w.a.journal()
            .into_iter()
            .filter(|(k, _, _)| k == "thread.register")
            .collect();
    assert_eq!(registers.len(), 1, "one register fact: {registers:?}");
    assert_eq!(registers[0].1, "applied");

    fact_exchange::sync(&w.a.store, &w.b.store);
    assert!(archived(&w.b, &quiet), "B sees it archived");
    assert_eq!(
        w.a.register(&quiet, "archived").map(|r| r.0),
        w.b.register(&quiet, "archived").map(|r| r.0)
    );
    assert_eq!(w.a.thread_projection(), w.b.thread_projection());
    w.done();
}

#[test]
fn message_into_archived_thread_applies() {
    let w = world(84);
    w.a.cmd_archive(&w.author, &w.thread, "op-archive").unwrap();
    fact_exchange::sync(&w.a.store, &w.b.store);
    assert!(archived(&w.a, &w.thread) && archived(&w.b, &w.thread));

    // A post into the archived thread (authored before its author could
    // have refused it) still applies on the receiver.
    let message = w.b_posts_lazy(&w.thread);
    fact_exchange::sync(&w.a.store, &w.b.store);
    assert!(
        w.a.journal().iter().all(|(_, state, _)| state == "applied"),
        "{:?}",
        w.a.journal()
    );
    assert_eq!(
        w.a.count(&format!(
            "SELECT count(*) FROM messages WHERE id='{}' AND kind='ordinary'",
            message.as_str()
        )),
        1,
        "A holds the message"
    );
    assert_eq!(
        w.a.count(&format!(
            "SELECT count(*) FROM lazy_recipients WHERE seat_id='{}'",
            w.author.as_str()
        )),
        1,
        "and A's author seat has the lazy row"
    );
    assert!(archived(&w.a, &w.thread), "the thread stays archived");
    w.done();
}

#[test]
fn archival_accepted_race_documented() {
    let w = world(88);
    // A thread whose only A-side member is an inactive local seat.
    let holder = w.a.local_seat();
    w.a.publish(&holder).unwrap();
    let racy = w.a.create_thread(&holder, "racy", None).unwrap();
    w.a.joined(&racy, &holder, 1).unwrap();
    sync_all(&[&w.a, &w.b]);

    // B's agent joins and posts; neither reaches A before A's lane runs.
    w.b.joined(&racy, &w.agent, 1).unwrap();
    let post = {
        let message = w.b.stage_prep(&racy, "lazy", &[], &[&w.retiree]);
        let payload = sent(
            &message,
            &racy,
            &w.agent,
            DeliveryMode::Lazy,
            0,
            vec![
                spec(&holder, ReceiptObligation::None, None),
                spec(&w.retiree, ReceiptObligation::None, None),
            ],
        );
        w.b.author_sent(&payload).unwrap();
        message
    };
    tick_archival(&w.a, 0, 600_000);
    assert!(archived(&w.a, &racy), "A archived without knowing the join");
    assert!(!archived(&w.b, &racy));

    // After the exchange: archived on both, the join and the post applied,
    // the post delivered to A's local member.
    fact_exchange::sync(&w.a.store, &w.b.store);
    for node in [&w.a, &w.b] {
        assert!(archived(node, &racy), "{:?}", node.id);
        assert_eq!(
            node.membership(&racy, &w.agent).map(|m| m.0).as_deref(),
            Some("joined"),
            "{:?}",
            node.id
        );
    }
    assert!(w.a.journal().iter().all(|(_, state, _)| state == "applied"));
    assert_eq!(
        w.a.count(&format!(
            "SELECT count(*) FROM messages WHERE id='{}'",
            post.as_str()
        )),
        1,
        "the post applied on A"
    );
    assert_eq!(
        w.a.count(&format!(
            "SELECT count(*) FROM lazy_recipients WHERE seat_id='{}'",
            holder.as_str()
        )),
        1,
        "and delivered to A's member"
    );

    // An explicit reopen restores it everywhere.
    w.b.cmd_reopen(&w.agent, &racy, "op-reopen").unwrap();
    fact_exchange::sync(&w.a.store, &w.b.store);
    for node in [&w.a, &w.b] {
        assert!(!archived(node, &racy), "reopened on {:?}", node.id);
    }
    assert_eq!(w.a.thread_projection(), w.b.thread_projection());
    w.done();
}

// ---- the send fence across nodes ---------------------------------------------------------

#[test]
fn fence_across_nodes_20_remote_10_lazy() {
    let w = world(86);
    // Ten lazy sends of A's author, prepared against one snapshot.
    let prepared: Vec<SendMessage> = (0..10)
        .map(|n| {
            w.a.prepare_send(
                &w.author,
                &w.thread,
                DeliveryMode::Lazy,
                &format!("fence-op-{n}"),
            )
            .unwrap()
        })
        .collect();
    let revision = |node: &Node| {
        node.count(&format!(
            "SELECT timeline_revision FROM threads WHERE id='{}'",
            w.thread.as_str()
        ))
    };
    let before = revision(&w.a);
    // Twenty remote messages apply into the thread on A, two between
    // consecutive publishes.
    for n in 0..20 {
        w.b_posts_lazy(&w.thread);
        fact_exchange::sync(&w.a.store, &w.b.store);
        if n % 2 == 1 {
            let published = w.a.publish_prepared(&prepared[n / 2]);
            assert!(
                published.is_ok(),
                "send {} failed on a moved timeline: {published:?}",
                n / 2
            );
        }
    }
    assert!(revision(&w.a) >= before + 30, "the timeline moved");
    assert_eq!(
        w.a.count("SELECT count(*) FROM messages WHERE body LIKE 'body of fence-op-%'"),
        10
    );
    fact_exchange::sync(&w.a.store, &w.b.store);
    assert_eq!(
        w.b.count("SELECT count(*) FROM messages WHERE body LIKE 'body of fence-op-%'"),
        10,
        "all ten reach B"
    );
    assert_eq!(
        w.a.count("SELECT count(*) FROM messages WHERE kind='ordinary'"),
        w.b.count("SELECT count(*) FROM messages WHERE kind='ordinary'")
    );
    w.done();
}

// ---- the restore-window authoring hold across commands -----------------------------------------

/// `(kind, state)` journal rows of `kind` still waiting on `node`.
fn pending_of(node: &Node, kind: &str) -> usize {
    node.journal()
        .iter()
        .filter(|(k, state, _)| k == kind && state == "pending")
        .count()
}

#[test]
fn restore_window_hold_across_commands() {
    let w = world(90);
    let reg = registry();
    // Threads and invitations A offers the agent.
    let thread = |name: &str| {
        w.a.cmd_create_thread(&w.author, name, &format!("op-create-{name}"))
            .unwrap()
    };
    let (t_acc, t_rej, t_join, t_old_acc, t_old_join) = (
        thread("accept"),
        thread("reject"),
        thread("join"),
        thread("old-accept"),
        thread("old-join"),
    );
    let (inv_acc, inv_rej, inv_old) = (
        w.a.cmd_invite(&w.author, &t_acc, &w.agent, "op-inv-acc")
            .unwrap(),
        w.a.cmd_invite(&w.author, &t_rej, &w.agent, "op-inv-rej")
            .unwrap(),
        w.a.cmd_invite(&w.author, &t_old_acc, &w.agent, "op-inv-old")
            .unwrap(),
    );
    let message = w.send_at(SENT_AT);
    w.deliver(&message);

    // B's other incarnation (its old log) already answered one invitation
    // and joined one thread; this store is the restored one that has not
    // pulled that tail yet.
    let old = Node::open(91, 7, reg, true);
    assert_eq!(old.id, w.b.id, "the same node");
    old.add_local_seat(&w.agent);
    old.publish(&w.agent).unwrap();
    fact_exchange::sync(&w.a.store, &old.store);
    old.author_answered(&old.accept_payload(&t_old_acc, &inv_old, &w.agent))
        .unwrap();
    old.joined(&t_old_join, &w.agent, 1).unwrap();

    hold_on(&w.b);
    let journal_before = w.b.journal();
    let refused = |what: &str, result: Result<CommandResult, ApiError>| {
        let error = result.expect_err(what);
        assert_eq!(error.code, ErrorCode::ServiceBusy, "{what}: retryable");
        assert_eq!(error.detail, "own history catching up", "{what}");
        assert_eq!(
            w.b.journal(),
            journal_before,
            "{what}: the journal is unchanged"
        );
        assert_eq!(
            w.b.count("SELECT count(*) FROM fact_attribution_staging"),
            0,
            "{what}: no staged attribution left behind"
        );
    };
    refused("ack", w.b.ack(&w.agent, "op-ack-hold", &[&message], false));
    refused(
        "accept",
        w.b.cmd_accept(&w.agent, &t_acc, &inv_acc, "op-acc-hold"),
    );
    refused(
        "reject",
        w.b.cmd_reject(&w.agent, &t_rej, &inv_rej, "op-rej-hold"),
    );
    refused("join", w.b.cmd_join(&w.agent, &t_join, "op-join-hold"));
    refused("leave", w.b.cmd_leave(&w.agent, &w.thread, "op-leave-hold"));
    refused(
        "create_thread",
        w.b.cmd_create_thread(&w.agent, "held", "op-create-hold")
            .map(CommandResult::ThreadCreated),
    );
    refused(
        "ensure_direct_thread",
        w.b.cmd_ensure_direct(&w.agent, &w.author, "op-pair-hold"),
    );

    // Send and invite are not held.
    let (sends, invites) = (
        w.b.count_kind("message.sent"),
        w.b.count_kind("invitation.created"),
    );
    let sent_during =
        w.b.cmd_send(&w.agent, &w.thread, DeliveryMode::Lazy, "op-send-hold")
            .unwrap();
    let invited_during =
        w.b.cmd_invite(&w.agent, &w.thread, &w.guest, "op-invite-hold")
            .unwrap();
    assert_eq!(w.b.count_kind("message.sent"), sends + 1);
    assert_eq!(w.b.count_kind("invitation.created"), invites + 1);
    let _ = (sent_during, invited_during);

    // The old tail arrives during the hold and is held, not applied.
    ship_origin(&old, old.id, &w.b.store);
    assert_eq!(pending_of(&w.b, "invitation.answered"), 1);
    assert_eq!(pending_of(&w.b, "membership.joined"), 1);

    // The hold clears: the same accept and join succeed.
    hold_off(&w.b);
    let answered = w.b.count_kind("invitation.answered");
    let joined = w.b.count_kind("membership.joined");
    w.b.cmd_accept(&w.agent, &t_acc, &inv_acc, "op-acc-2")
        .unwrap();
    w.b.cmd_join(&w.agent, &t_join, "op-join-2").unwrap();
    assert_eq!(w.b.count_kind("invitation.answered"), answered + 1);
    assert_eq!(w.b.count_kind("membership.joined"), joined + 1);

    // Where the old log already held the answer or the transition, the
    // restored node authors no second one.
    drain(&w.b.store);
    assert_eq!(pending_of(&w.b, "invitation.answered"), 0);
    assert_eq!(pending_of(&w.b, "membership.joined"), 0);
    assert_eq!(w.b.state_of(&inv_old).as_deref(), Some("accepted"));
    assert_eq!(
        w.b.membership(&t_old_join, &w.agent)
            .map(|m| m.0)
            .as_deref(),
        Some("joined")
    );
    let answered = w.b.count_kind("invitation.answered");
    let joined = w.b.count_kind("membership.joined");
    let accept_again = w.b.cmd_accept(&w.agent, &t_old_acc, &inv_old, "op-acc-old");
    let join_again = w.b.cmd_join(&w.agent, &t_old_join, "op-join-old");
    assert!(
        matches!(accept_again, Ok(CommandResult::Accepted(_))),
        "the accept is idempotent: {accept_again:?}"
    );
    assert!(
        matches!(join_again, Ok(CommandResult::Joined(_))),
        "the join is idempotent: {join_again:?}"
    );
    assert_eq!(
        w.b.count_kind("invitation.answered"),
        answered,
        "no second answer"
    );
    assert_eq!(
        w.b.count_kind("membership.joined"),
        joined,
        "no second transition"
    );
    w.done();
}
