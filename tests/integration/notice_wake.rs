//! Informational warning transitions (another seat's overdue receipt opening
//! or clearing) are delivered to every member as notices on the next check-in
//! (TRUST-POLICY A7). They must not wake a member who owes nothing: the wake
//! prompt sends that member to an inbox with no actionable item. The affected
//! seat's own open overdue warning stays wake-eligible (the hard-deadline
//! backstop). Real canonical store mutations; no daemon or global state.
use herdr_threads::{
    ports::{
        DueScanRequest, DueScanState, DurableWorkAdmission, ReadContext, RegisterAvailableRequest,
        SendPreparationProgress, StorePort, WakeCandidate,
    },
    protocol::{
        authority::{CallerClaim, CallerRole, Harness},
        commands::*,
        ids::*,
        output::OutputSpec,
        pagination::PageRequest,
        results::{CheckInResult, CommandResult, InboxBatchV2Item},
        time::{CallBudget, Clock, MonoInstant, UtcMillis},
    },
    store::{self, SqliteStore, StoreSettings, attention, connection::StoreContext},
    test_support::isolation::TestIsolation,
};
use rusqlite::{Connection, params};
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

struct FixedClock(AtomicI64);
impl Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(self.0.load(Ordering::Relaxed))
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(100)
    }
}
fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    }
}
fn read_context() -> ReadContext {
    ReadContext {
        instance: "i".into(),
        output: OutputSpec::default(),
        operation_scope: None,
    }
}
struct Fixture {
    // SQLite owners drop before the isolation directory.
    store: SqliteStore,
    db: Connection,
    clock: Arc<FixedClock>,
    _iso: TestIsolation,
}
impl Fixture {
    fn new() -> Self {
        let iso = TestIsolation::new("notice-wake");
        let clock = Arc::new(FixedClock(AtomicI64::new(1000)));
        let context = StoreContext::new(iso.path("store.db"), clock.clone());
        let db = context.open_writer().unwrap();
        db.execute_batch("INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES('i',0,'b',1,1);
            INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES('t','i','topic','goal',0,0);").unwrap();
        let store = SqliteStore::new(
            StoreContext::new(iso.path("store.db"), clock.clone()),
            "i",
            StoreSettings::default(),
        )
        .unwrap();
        let f = Self {
            store,
            db,
            clock,
            _iso: iso,
        };
        for (n, seat) in ["author", "agent", "bystander"].into_iter().enumerate() {
            f.member(seat, n + 1);
        }
        f
    }
    fn member(&self, seat: &str, n: usize) {
        let execution = format!("00000000-0000-4000-8000-00000000000{n}");
        self.db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES(?1,'i','resolved','native',?1,1,0,0)",[seat]).unwrap();
        self.db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch,ui_state) VALUES('i',?1,'b',1,0,1,'fresh',0,'term-'||?1,'inc','coherent_enumeration',1,'idle')",[seat]).unwrap();
        self.db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES(?1,1,?1,'b',1,0,'codex','session-'||?1,?2,'cooperative_top_level',0,0,'term-'||?1,'inc')",params![seat, execution]).unwrap();
        self.db
            .execute(
                "INSERT INTO memberships(thread_id,seat_id,state) VALUES('t',?1,'joined')",
                [seat],
            )
            .unwrap();
        self.db.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES('t',?1,1,1)",[seat]).unwrap();
    }
    fn claim(&self, seat: &str) -> CallerClaim {
        let (generation, execution): (i64, String) = self.db.query_row("SELECT generation,execution_id FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL",[seat],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        CallerClaim {
            instance: "i".into(),
            seat: SeatId::new(seat),
            binding_generation: generation as u64,
            role: CallerRole::TopLevel,
            harness: Harness::Codex,
            native_session: NativeSessionId::new(format!("session-{seat}")),
            execution: ExecutionId::new(execution),
            target: HostTargetId::new(seat),
        }
    }
    fn mutate(&self, mutation: PermitMutation) -> CommandResult {
        let permit = self
            .store
            .issue_cooperative_permit(
                store::cooperative_permit_request(&mutation).unwrap(),
                &budget(),
            )
            .unwrap();
        self.store.mutate(mutation, permit, &budget()).unwrap()
    }
    /// The author's critical message, addressed to every member with an ACK
    /// deadline (the observed `t2ceBcgea` shape).
    fn send_with_deadline(&self) -> MessageId {
        let request = SendMessage {
            delivery_mode: DeliveryMode::Ordinary,
            thread: ThreadId::new("t"),
            body: "critical".into(),
            invited_recipients: vec![],
            deadline_millis: Some(60_000),
            operation: OperationId::new("send"),
            claim: self.claim("author"),
            relays_user: false,
            user_intent: None,
        };
        for _ in 0..100 {
            if matches!(
                self.store
                    .prepare_send_step(&request, DurableWorkAdmission { max_units: 1 }, &budget())
                    .unwrap(),
                SendPreparationProgress::Ready { .. }
            ) {
                let CommandResult::MessageSent(id) =
                    self.mutate(PermitMutation::SendMessage(request))
                else {
                    panic!("send")
                };
                return id;
            }
        }
        panic!("preparation did not converge")
    }
    fn ack(&self, seat: &str, message: &MessageId, op: &str) {
        self.mutate(PermitMutation::Ack(Ack {
            messages: vec![message.clone()],
            operation: OperationId::new(op),
            claim: self.claim(seat),
        }));
    }
    fn check(&self, seat: &str, op: &str) -> CheckInResult {
        let command = CheckIn {
            mode: CheckInMode::Current,
            claim: self.claim(seat),
            operation: OperationId::new(op),
        };
        let permit = self
            .store
            .issue_cooperative_permit(
                store::cooperative_permit_request(&PermitMutation::CheckIn(command.clone()))
                    .unwrap(),
                &budget(),
            )
            .unwrap();
        let CommandResult::CheckedIn(result) = self
            .store
            .register_available(
                RegisterAvailableRequest {
                    command,
                    read: read_context(),
                    operator: None,
                },
                permit,
                &budget(),
            )
            .unwrap()
        else {
            panic!("check")
        };
        result
    }
    /// Run due scans and every outstanding durable work job to completion.
    fn settle(&self) {
        let mut state = DueScanState::default();
        for _ in 0..16 {
            let progress = self
                .store
                .due_obligations(
                    DueScanRequest {
                        state,
                        max_candidates: 16,
                        run_invitations: true,
                        run_receipts: true,
                    },
                    &budget(),
                )
                .unwrap();
            state = progress.state;
            for _ in 0..64 {
                let page = self
                    .store
                    .pending_work(PageRequest::default(), &budget())
                    .unwrap();
                if page.items.is_empty() {
                    break;
                }
                for job in page.items {
                    self.store
                        .advance_work(&job.id, DurableWorkAdmission { max_units: 16 }, &budget())
                        .unwrap();
                }
            }
        }
    }
    fn candidate(&self, seat: &str) -> Option<WakeCandidate> {
        self.store
            .wake_candidates(PageRequest::default(), &budget())
            .unwrap()
            .items
            .into_iter()
            .find(|c| c.seat.as_str() == seat && c.has_actionable_work())
    }
    fn inbox(&self, seat: &str) -> Vec<InboxBatchV2Item> {
        let CommandResult::InboxBatchV2(page) = self
            .store
            .query(
                &Command::InboxBatchV2(InboxQuery {
                    seat: Some(SeatId::new(seat)),
                    page: PageRequest::default(),
                }),
                &read_context(),
                &budget(),
            )
            .unwrap()
        else {
            panic!("inbox")
        };
        page.items
    }
    fn wake_warning(&self, seat: &str) -> Option<i64> {
        attention::wake_seat_attention(&self.db, seat)
            .unwrap()
            .attention
            .latest_warning_seq
    }
    fn digest_warnings(&self, seat: &str) -> u64 {
        attention::seat_digest(&self.db, "i", &SeatId::new(seat), &|| Ok(()))
            .unwrap()
            .digest
            .warnings
            .count
    }
    fn conditions(&self) -> Vec<(String, Option<String>)> {
        let mut stmt = self
            .db
            .prepare(
                "SELECT open_warning_id,clear_warning_id FROM warning_conditions ORDER BY ordinal",
            )
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }
}

fn notice_ids(offer: &CheckInResult) -> Vec<String> {
    offer
        .notices
        .items
        .iter()
        .map(|n| n.warning.as_str().to_owned())
        .collect()
}

/// Catches the observed churn: an author who owes nothing was woken once when
/// another seat's ACK went overdue and again when the late ACK cleared it, and
/// the prompted inbox had nothing to show. Also catches over-correction: the
/// affected seat must stay woken by its open overdue warning, every member
/// still receives both transitions on its next check-in, and history keeps
/// the open, the clear and the late ACK.
#[test]
fn other_seat_overdue_transitions_never_wake_members_who_owe_nothing() {
    let f = Fixture::new();
    let message = f.send_with_deadline();
    f.ack("bystander", &message, "ack-bystander");
    f.settle();
    for seat in ["author", "agent", "bystander"] {
        f.check(seat, &format!("check-before-{seat}"));
    }
    assert!(f.candidate("author").is_none());
    assert!(f.candidate("bystander").is_none());

    // Only "agent" misses the deadline.
    f.clock.0.store(1000 + 120_000, Ordering::Relaxed);
    f.settle();
    let [(open, opened_clear)]: [(String, Option<String>); 1] =
        f.conditions().try_into().expect("one overdue condition");
    assert_eq!(opened_clear, None);
    for seat in ["author", "bystander"] {
        assert!(f.candidate(seat).is_none(), "{seat} owes nothing");
        assert_eq!(f.wake_warning(seat), None, "{seat}");
        // Still delivered as a notice (TRUST-POLICY A7), not dropped.
        assert_eq!(f.digest_warnings(seat), 1, "{seat}");
    }
    let affected = f.candidate("agent").expect("the late recipient is woken");
    assert!(affected.has_pending_receipt);
    assert!(
        affected.actionable_warning_seq.is_some()
            && !affected.warning_offered_for_current_occupant(),
        "the hard-deadline warning stays a wake backstop: {affected:?}"
    );
    assert!(
        attention::seat_has_pending_wake_notices(&f.db, "agent").unwrap(),
        "an unoffered own open warning still voids a covering offer"
    );
    assert_eq!(
        f.wake_warning("agent"),
        affected.actionable_warning_seq.map(|s| s as i64)
    );
    assert_eq!(
        notice_ids(&f.check("author", "check-open")),
        std::slice::from_ref(&open)
    );

    f.ack("agent", &message, "ack-agent-late");
    f.settle();
    let [(reopened, clear)]: [(String, Option<String>); 1] =
        f.conditions().try_into().expect("still one condition");
    let clear = clear.expect("the late ACK clears the condition");
    assert_eq!(reopened, open);
    for seat in ["author", "agent", "bystander"] {
        assert!(f.candidate(seat).is_none(), "a clear wakes nobody: {seat}");
        assert_eq!(f.wake_warning(seat), None, "{seat}");
    }
    assert!(
        f.inbox("author")
            .iter()
            .all(|item| !matches!(item, InboxBatchV2Item::Message { .. }))
    );
    assert_eq!(
        notice_ids(&f.check("author", "check-clear")),
        std::slice::from_ref(&clear)
    );
    assert_eq!(
        notice_ids(&f.check("bystander", "check-bystander")),
        [open.clone(), clear.clone()]
    );
    assert!(
        f.inbox("author").is_empty(),
        "settled notices leave no inbox work"
    );
    // The late ACK is the agent's own explicit receipt.
    let (state, actor): (String, String) =
        f.db.query_row(
            "SELECT state,ack_actor_seat_id FROM receipts WHERE message_id=?1 AND seat_id='agent' UNION ALL SELECT state,ack_actor_seat_id FROM receipt_state WHERE message_id=?1 AND seat_id='agent'",
            [message.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((state.as_str(), actor.as_str()), ("acked", "agent"));
}

/// Catches another seat's transitions re-arming a late seat's warning wake
/// after its own overdue notice was carried, through either the pending set or
/// the offer-frontier probe, while that seat's own receipt still wakes it.
#[test]
fn another_seats_transitions_do_not_rearm_a_carried_warning() {
    let f = Fixture::new();
    let message = f.send_with_deadline();
    f.settle();
    for seat in ["author", "agent", "bystander"] {
        f.check(seat, &format!("check-before-{seat}"));
    }
    f.clock.0.store(1000 + 120_000, Ordering::Relaxed);
    f.settle();
    assert_eq!(f.conditions().len(), 2, "agent and bystander are both late");
    let carried = notice_ids(&f.check("agent", "check-agent-open"));
    assert_eq!(carried.len(), 2, "both opens reach agent: {carried:?}");
    let settled = f.candidate("agent").expect("the pending receipt remains");
    assert_eq!(settled.actionable_warning_seq, None, "{settled:?}");

    f.ack("bystander", &message, "ack-bystander-late");
    f.settle();
    assert!(attention::seat_has_pending_notices(&f.db, "agent").unwrap());
    assert!(
        !attention::seat_has_pending_wake_notices(&f.db, "agent").unwrap(),
        "bystander's clear must not void agent's warning offer"
    );
    let candidate = f.candidate("agent").expect("the pending receipt remains");
    assert!(candidate.has_pending_receipt);
    assert_eq!(candidate.actionable_warning_seq, None, "{candidate:?}");
    assert!(f.candidate("author").is_none());
    assert!(f.candidate("bystander").is_none());
}
