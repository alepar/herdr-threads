//! The read surface of a reversible retirement (ht-ma6i.10, spec 4.3, 4.5, 4.7): `retirement_state`
//! and pins for the retired reads that need no predicate of their own. A failing pin is a finding on
//! its read site. Mounted from `tests/store/reversible_retirement.rs`, whose fixtures it uses.
use super::*;
use crate::{
    ports::{CooperativePermitRequest, DurableWorkAdmission, ReadContext, SendPreparationProgress},
    protocol::{
        authority::{
            CallerClaim, CallerRole, Harness, LivenessFingerprint, MutationPermit, ObligationRef,
        },
        commands::{
            Command, CreateThread, DeliveryMode, EnsureDirectThread, HistoryQuery, InboxQuery,
            InvitationConfirmation, Invite, PendingReceiptsQuery, PermitMutation, SeatInspectQuery,
            SendMessage, WarningsQuery,
        },
        ids::{BindingId, ExecutionId, NativeSessionId},
        pagination::PageRequest,
        results::{CommandResult, ContinuityStatus, SeatInspection, SeatRetirementState},
    },
    store::{
        control, messages,
        read_cursors::{self, SeedMode},
        schema,
    },
};

fn page() -> PageRequest {
    PageRequest {
        cursor: None,
        limit: 50,
        max_bytes: 65_536,
    }
}

fn read_context(instance: &str) -> ReadContext {
    ReadContext {
        instance: instance.into(),
        output: Default::default(),
        operation_scope: None,
    }
}

fn inspect_on(store: &SqliteStore, instance: &str, seat: &SeatId) -> SeatInspection {
    let result = StorePort::query(
        store,
        &Command::SeatInspect(SeatInspectQuery {
            seat: seat.clone(),
            page: PageRequest { limit: 1, ..page() },
        }),
        &read_context(instance),
        &budget(),
    )
    .unwrap();
    let CommandResult::SeatInspect(inspection) = result else {
        panic!("inspect answered {result:?}");
    };
    inspection
}

/// The fixture seat's own registered binding (`seed_seat`).
fn claim(seat: &str) -> CallerClaim {
    CallerClaim {
        host: crate::test_support::test_host_locator(),
        instance: INSTANCE.into(),
        seat: SeatId::new(seat),
        binding_generation: 1,
        role: CallerRole::TopLevel,
        harness: Harness::Codex,
        native_session: NativeSessionId::new("n"),
        execution: ExecutionId::new(history::fixture_execution(seat)),
        target: HostTargetId::new(seat),
        agent_id: None,
        liveness: Some(LivenessFingerprint::TEST),
    }
}

fn mutate_as(
    f: &Fx,
    seat: &str,
    operation: &str,
    obligation: ObligationRef,
    payload_hash: [u8; 32],
    mutation: PermitMutation,
) -> Result<CommandResult, ApiError> {
    let permit: MutationPermit = f.store.issue_cooperative_permit(
        CooperativePermitRequest {
            claim: claim(seat),
            operation: OperationId::new(operation),
            obligation,
            payload_hash,
            check_in_mode: None,
        },
        &budget(),
    )?;
    f.store.mutate(mutation, permit, &budget())
}

impl Fx {
    fn inspect(&self, seat: &str) -> SeatInspection {
        inspect_on(&self.store, INSTANCE, &SeatId::new(seat))
    }
    /// A native send by `seat`'s binding, through preparation and publication.
    fn native_send(
        &self,
        seat: &str,
        thread: &ThreadId,
        direct_recipient: Option<&str>,
        ack_from: &[&str],
        deadline_millis: Option<u64>,
        operation: &str,
    ) -> Result<MessageId, ApiError> {
        let send = SendMessage {
            priority: None,
            delivery_mode: DeliveryMode::Ordinary,
            user_intent: None,
            thread: thread.clone(),
            body: "body".into(),
            direct_recipient: direct_recipient.map(SeatId::new),
            invited_recipients: ack_from.iter().map(|s| SeatId::new(*s)).collect(),
            deadline_millis,
            operation: OperationId::new(operation),
            claim: claim(seat),
            relays_user: false,
        };
        while !matches!(
            self.store.prepare_send_step(
                &send,
                DurableWorkAdmission::new(16).unwrap(),
                &budget()
            )?,
            SendPreparationProgress::Ready { .. }
        ) {}
        let digest = schema::canonical_digest(&messages::send_payload(&send)).unwrap();
        let result = mutate_as(
            self,
            seat,
            operation,
            ObligationRef::Control(thread.clone()),
            digest,
            PermitMutation::SendMessage(send),
        )?;
        let CommandResult::MessageSent(message) = result else {
            panic!("send answered {result:?}");
        };
        Ok(message)
    }
    fn ensure_direct(&self, seat: &str, recipient: &str, op: &str) -> Result<ThreadId, ApiError> {
        let ensure = EnsureDirectThread {
            creation_policy: None,
            recipient: SeatId::new(recipient),
            operation: OperationId::new(op),
            claim: claim(seat),
        };
        let hash = control::cooperative_payload_hash("ensure_direct_thread", &ensure).unwrap();
        let result = mutate_as(
            self,
            seat,
            op,
            ObligationRef::CheckIn(SeatId::new(seat)),
            hash,
            PermitMutation::EnsureDirectThread(ensure),
        )?;
        let CommandResult::ThreadCreated(thread) = result else {
            panic!("ensure answered {result:?}");
        };
        Ok(thread)
    }
    fn native_thread(&self, seat: &str, op: &str) -> ThreadId {
        let create = CreateThread {
            name: None,
            topic: "topic".into(),
            goal: "goal".into(),
            operation: OperationId::new(op),
            claim: claim(seat),
        };
        let hash = control::cooperative_payload_hash("create_thread", &create).unwrap();
        let result = mutate_as(
            self,
            seat,
            op,
            ObligationRef::CheckIn(SeatId::new(seat)),
            hash,
            PermitMutation::CreateThread(create),
        )
        .unwrap();
        let CommandResult::ThreadCreated(thread) = result else {
            panic!("create answered {result:?}");
        };
        thread
    }
    fn native_invite(
        &self,
        inviter: &str,
        thread: &ThreadId,
        seat: &str,
        deadline_millis: Option<u64>,
        op: &str,
    ) -> Result<CommandResult, ApiError> {
        let invite = Invite {
            confirmation: Some(InvitationConfirmation::RequireAck),
            thread: thread.clone(),
            seat: SeatId::new(seat),
            deadline_millis,
            operation: OperationId::new(op),
            claim: claim(inviter),
        };
        let hash = control::cooperative_payload_hash("invite", &invite).unwrap();
        mutate_as(
            self,
            inviter,
            op,
            ObligationRef::Control(thread.clone()),
            hash,
            PermitMutation::Invite(invite),
        )
    }
    /// The cursors the seat's first binding started with, which a later binding inherits.
    fn seed_cursors(&self, seat: &str) {
        let binding: String = self
            .db()
            .query_row(
                "SELECT binding_id FROM occupant_bindings WHERE seat_id=?1",
                [seat],
                |r| r.get(0),
            )
            .unwrap();
        read_cursors::seed(&self.db(), &BindingId::new(binding), SeedMode::Fresh).unwrap();
    }
    /// A seat coming back onto its target with a fresh binding, as a reattach leaves it; the
    /// binding's claim.
    fn reattach(&self, seat: &str) -> CallerClaim {
        let mut db = self.db();
        let tx = db.transaction().unwrap();
        seats::operator::rebind_unresolved_seat(
            &tx,
            INSTANCE,
            crate::test_support::test_host_id(),
            &SeatId::new(seat),
            seat,
            1,
            self.clock.utc_now(),
        )
        .unwrap();
        tx.commit().unwrap();
        let generation = self.int(&format!("SELECT generation FROM seats WHERE id='{seat}'"));
        let execution = history::fixture_execution(&format!("{seat}-reattached"));
        db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation,registered_at) VALUES (?1,?2,1,?1,'b',1,'codex','fresh',?3,'cooperative_top_level',0,'term-'||?1,'inc',0)", params![seat, generation, execution]).unwrap();
        let binding: String = db
            .query_row(
                "SELECT binding_id FROM occupant_bindings WHERE seat_id=?1 AND native_session='fresh'",
                [seat],
                |r| r.get(0),
            )
            .unwrap();
        read_cursors::seed(&db, &BindingId::new(binding), SeedMode::Fresh).unwrap();
        CallerClaim {
            binding_generation: generation as u64,
            native_session: NativeSessionId::new("fresh"),
            execution: ExecutionId::new(execution),
            ..claim(seat)
        }
    }
    fn inbox_json(&self, reader: &CallerClaim) -> String {
        let result = StorePort::query(
            &self.store,
            &Command::InboxBatchV2(InboxQuery {
                pushed: false,
                thread: None,
                claim: Some(reader.clone()),
                seat: Some(reader.seat.clone()),
                page: page(),
                seat_alerts: false,
            }),
            &read_context(INSTANCE),
            &budget(),
        )
        .unwrap();
        serde_json::to_string(&result).unwrap()
    }
    /// Every column of the seat's rows in `table`, to compare physically.
    fn snapshot(&self, table: &str, seat: &str) -> Vec<String> {
        let db = self.db();
        let mut stmt = db
            .prepare(&format!(
                "SELECT * FROM {table} WHERE seat_id=?1 ORDER BY rowid"
            ))
            .unwrap();
        let columns = stmt.column_count();
        stmt.query_map([seat], |row| {
            let cells: Vec<String> = (0..columns)
                .map(|i| format!("{:?}", row.get_ref(i).unwrap()))
                .collect();
            Ok(cells.join("|"))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
    }
    /// The delivery rows a fan-out staged for `message`.
    fn audience(&self, message: &MessageId) -> Vec<String> {
        let db = self.db();
        let mut stmt = db
            .prepare("SELECT seat_id FROM lazy_recipients WHERE message_id=?1 ORDER BY seat_id")
            .unwrap();
        stmt.query_map([message.as_str()], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }
}

/// Kills: a retirement state on a seat that does not read retired; a reversible seat reading absent
/// or a final seat present; an unretired row reading reversible.
#[test]
fn retired_state_reads_reversible_or_final_and_absent_otherwise() {
    let f = fx(&["s1", "s2"]);
    f.key("s1");
    assert_eq!(f.inspect("s1").retirement_state, None, "live");

    f.close("s1").unwrap();
    let frozen = f.inspect("s1");
    assert_eq!(frozen.summary.continuity, ContinuityStatus::Retired);
    assert_eq!(
        frozen.retirement_state,
        Some(SeatRetirementState::Reversible)
    );
    assert_eq!(frozen.retirement, None, "no job until finalized");
    assert!(frozen.continuity_key.is_some(), "the key stays");

    f.unretire("s1", "u").unwrap();
    let waiting = f.inspect("s1");
    assert_eq!(waiting.summary.continuity, ContinuityStatus::Unresolved);
    assert_eq!(
        waiting.retirement_state, None,
        "an unretired seat is absent"
    );
    assert_eq!(waiting.summary.target, None);

    f.finalize("s1", "f").unwrap();
    let finalized = f.inspect("s1");
    assert_eq!(finalized.summary.continuity, ContinuityStatus::Retired);
    assert_eq!(finalized.retirement_state, None, "final is absent");
    assert!(finalized.retirement.is_some());

    // A keyless close never went through a reversible row: the pre-branch shape.
    f.close("s2").unwrap();
    let keyless = f.inspect("s2");
    assert_eq!(keyless.summary.continuity, ContinuityStatus::Retired);
    assert_eq!(keyless.retirement_state, None, "keyless final is absent");
}

/// `SeatInspection`'s field set at protocol 9 (v3.0.0, `main` before ht-ma6i), as a stale client
/// decodes it: any field it does not know fails the decode.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct Protocol9SeatInspection {
    summary: serde_json::Value,
    mapping: serde_json::Value,
    hold: serde_json::Value,
    retirement: serde_json::Value,
    #[serde(default)]
    open_binding: serde_json::Value,
    history: serde_json::Value,
    #[serde(default)]
    directory: serde_json::Value,
}

/// Kills: a finally retired seat (keyed and finalized, or keyless and closed) emitting
/// `retirement_state`, which a protocol-9 client refuses as an unknown field.
#[test]
fn a_finally_retired_inspection_decodes_under_the_protocol_9_shape() {
    let f = fx(&["s1", "s2"]);
    f.key("s1");
    f.close("s1").unwrap();
    let reversible = serde_json::to_value(f.inspect("s1")).unwrap();
    assert_eq!(reversible["retirement_state"], "reversible");
    f.finalize("s1", "f").unwrap();
    f.close("s2").unwrap();
    for seat in ["s1", "s2"] {
        let value = serde_json::to_value(f.inspect(seat)).unwrap();
        assert_eq!(value["summary"]["continuity"], "retired", "{seat}");
        assert!(value.get("retirement_state").is_none(), "{seat}: {value}");
        serde_json::from_value::<Protocol9SeatInspection>(value)
            .unwrap_or_else(|e| panic!("{seat}: protocol 9 decode: {e}"));
    }
}

/// Kills: a window send opening or reusing a pair with a frozen seat, or a refusal other than the one
/// a finally retired seat gets. The fresh pair is the CLI's direct-send path, whose first step is
/// the ensure.
#[test]
fn direct_send_is_refused_as_retired() {
    let f = fx(&["s1", "s2", "s3", "s4"]);
    f.key("s1");
    let frozen = f.ensure_direct("s2", "s1", "frozen-pair").unwrap();
    let finalized = f.ensure_direct("s2", "s4", "final-pair").unwrap();
    // The pair carries a direct send while its seats are live: the refusals below are retirement's.
    for (pair, seat) in [(&frozen, "s1"), (&finalized, "s4")] {
        f.native_send("s2", pair, Some(seat), &[], None, &format!("before-{seat}"))
            .unwrap();
    }
    f.close("s1").unwrap();
    f.close("s4").unwrap();

    let window = f
        .native_send("s2", &frozen, Some("s1"), &[], None, "window")
        .unwrap_err();
    assert_eq!(window.code, ErrorCode::TargetUnresolved, "{window:?}");
    assert_eq!(
        window.detail,
        "delivery recipient unresolved, held or invalidated"
    );
    let today = f
        .native_send("s2", &finalized, Some("s4"), &[], None, "final")
        .unwrap_err();
    assert_eq!((window.code, &window.detail), (today.code, &today.detail));

    let fresh = f.ensure_direct("s3", "s1", "fresh").unwrap_err();
    assert_eq!(fresh.code, ErrorCode::TargetUnresolved, "{fresh:?}");
    assert_eq!(
        f.int("SELECT count(*) FROM direct_threads WHERE 's3' IN (seat_a,seat_b)"),
        0
    );
}

/// Kills: an ACK request accepted for a frozen seat, on the native and the service path.
#[test]
fn explicit_ack_recipient_is_refused() {
    let f = fx(&["s1", "s2"]);
    f.key("s1");
    let thread = ThreadId::new("t");
    // A live seat passes the same request: the fixture can send it.
    f.native_send("s2", &thread, None, &["s1"], Some(1_000), "before")
        .unwrap();
    f.close("s1").unwrap();

    let error = f
        .native_send("s2", &thread, None, &["s1"], Some(1_000), "window")
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest, "{error:?}");
    assert_eq!(error.detail, "explicit recipient invalid or retired");

    let error = f.send("service", &["s1"], 1_000).unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest, "{error:?}");
    assert_eq!(error.detail, "recipient retired: s1");
}

/// Kills: an invitation written for a frozen seat, on the native and the service path.
#[test]
fn invite_is_refused() {
    let f = fx(&["s1", "s2", "s3"]);
    f.key("s1");
    let thread = f.native_thread("s2", "create");
    // A live seat is invitable: the fixture can invite.
    f.native_invite("s2", &thread, "s3", None, "live").unwrap();
    f.close("s1").unwrap();

    let error = f
        .native_invite("s2", &thread, "s1", None, "native")
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::TargetUnresolved, "{error:?}");
    assert_eq!(
        error.detail,
        "invite target missing, retired, or outside instance"
    );

    let error = f
        .op(ServiceOperation::Invite(ServiceInvite {
            thread: ThreadId::new("t"),
            seat: SeatId::new("s1"),
            constraint: InvitationConstraint::Required,
            deadline_millis: Some(1_000),
            operation: OperationId::new("service"),
        }))
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::TargetUnresolved, "{error:?}");
    assert_eq!(
        error.detail,
        "invite target missing, retired, or outside instance"
    );
    assert_eq!(
        f.int("SELECT count(*) FROM invitations WHERE seat_id='s1'"),
        0
    );
}

/// Kills: a frozen member in a group send's audience; the fan-out refusing the whole send.
#[test]
fn group_fan_out_skips_it() {
    let f = fx(&["s1", "s2", "s3"]);
    f.key("s1");
    let thread = ThreadId::new("t");
    let before = f
        .native_send("s2", &thread, None, &[], None, "before")
        .unwrap();
    assert_eq!(f.audience(&before), ["s1", "s2", "s3"]);
    f.close("s1").unwrap();
    let window = f
        .native_send("s2", &thread, None, &[], None, "window")
        .unwrap();
    assert_eq!(f.audience(&window), ["s2", "s3"]);
}

/// Kills: a frozen seat's receipt reading pending to its sender, or the fence writing a terminal
/// state the way back could not undo.
#[test]
fn receipts_read_recipient_retired() {
    let f = fx(&["s1"]);
    f.key("s1");
    let logical = f.send("logical", &["s1"], 1_000).unwrap();
    let materialized = f.send("materialized", &["s1"], 1_000).unwrap();
    f.drain(&materialized);
    for message in [&logical, &materialized] {
        assert_eq!(
            f.receipt(message, "s1").state,
            EffectiveReceiptState::Pending
        );
    }
    f.close("s1").unwrap();
    for message in [&logical, &materialized] {
        assert_eq!(
            f.receipt(message, "s1").state,
            EffectiveReceiptState::RecipientRetired
        );
    }
    assert_eq!(
        f.text(&format!(
            "SELECT state FROM receipt_state WHERE seat_id='s1' AND message_id='{}'",
            materialized.as_str()
        )),
        "pending",
        "physically unchanged"
    );
    f.unretire("s1", "u").unwrap();
    for message in [&logical, &materialized] {
        assert_eq!(
            f.receipt(message, "s1").state,
            EffectiveReceiptState::Pending
        );
    }
}

/// Kills: the effective read following the physical row, or the row being rewritten to get there.
#[test]
fn effective_membership_reads_retired() {
    let f = fx(&["s1"]);
    f.key("s1");
    let read = |f: &Fx| {
        service_substrate::effective_membership(&f.db(), &ThreadId::new("t"), &SeatId::new("s1"))
            .unwrap()
            .voluntary
    };
    assert_eq!(read(&f), VoluntaryMembershipState::Joined);
    f.close("s1").unwrap();
    assert_eq!(read(&f), VoluntaryMembershipState::Retired);
    assert_eq!(
        f.text("SELECT voluntary_state FROM memberships WHERE seat_id='s1'"),
        "joined"
    );
}

/// Kills: the fence settling or stamping anything it needs back on un-retire.
#[test]
fn memberships_and_episodes_are_physically_unchanged() {
    let f = fx(&["s1"]);
    f.key("s1");
    f.op(ServiceOperation::Invite(ServiceInvite {
        thread: ThreadId::new("t"),
        seat: SeatId::new("s1"),
        constraint: InvitationConstraint::Required,
        deadline_millis: Some(1_000),
        operation: OperationId::new("require"),
    }))
    .unwrap();
    let tables = [
        "memberships",
        "membership_intervals",
        "requirement_episodes",
    ];
    let before = tables.map(|table| f.snapshot(table, "s1"));
    for rows in &before {
        assert!(!rows.is_empty(), "the snapshot compares something");
    }
    f.at(300);
    f.close("s1").unwrap();
    assert_eq!(tables.map(|table| f.snapshot(table, "s1")), before);
}

/// Spec 4.3: a window message leaves the seat out of its audience, so it owes nothing and no row
/// offers it; it is unread history, past the reattached binding's cursor in a thread the seat never
/// left. The message from before the window is the control: it arrives as an ordinary delivery row.
/// Kills: the membership interval closing at the fence; a cursor fast-forwarded over the window; a
/// window row or obligation appearing from nowhere; history refused to the returning seat.
#[test]
fn window_messages_are_unread_on_reattach() {
    let f = fx(&["s1", "s2"]);
    f.key("s1");
    f.seed_cursors("s1");
    let thread = ThreadId::new("t");
    let before = f
        .native_send("s2", &thread, None, &[], None, "before")
        .unwrap();
    f.close("s1").unwrap();
    let window = f
        .native_send("s2", &thread, None, &[], None, "window")
        .unwrap();
    f.unretire("s1", "u").unwrap();
    let reader = f.reattach("s1");
    assert_eq!(f.seat_state("s1"), "resolved");

    let inbox = f.inbox_json(&reader);
    assert!(inbox.contains(before.as_str()), "{inbox}");
    assert!(!inbox.contains(window.as_str()), "{inbox}");
    assert_eq!(
        f.text("SELECT voluntary_state FROM memberships WHERE seat_id='s1'"),
        "joined"
    );
    assert_eq!(
        f.int(
            "SELECT count(*) FROM membership_intervals WHERE seat_id='s1' AND left_seq IS NOT NULL"
        ),
        0,
        "the interval never closed"
    );
    let sequence = f.int(&format!(
        "SELECT sequence FROM messages WHERE id='{}'",
        window.as_str()
    ));
    let cursor = f.int(
        "SELECT COALESCE((SELECT seq FROM binding_cursors c JOIN occupant_bindings b ON b.binding_id=c.binding_id WHERE b.seat_id='s1' AND b.native_session='fresh' AND c.thread_id='t'),0)",
    );
    assert!(sequence > cursor, "{sequence} > {cursor}");

    let result = StorePort::query(
        &f.store,
        &Command::History(HistoryQuery {
            thread,
            page: page(),
            initial: None,
            full_bodies: false,
        }),
        &read_context(INSTANCE),
        &budget(),
    )
    .unwrap();
    let history = serde_json::to_string(&result).unwrap();
    assert!(history.contains(window.as_str()), "{history}");
}

/// The budget the store's writer enforces, on a clock that runs. Kills: a shift that walks rows one
/// by one, or fails a large backlog partway.
#[test]
fn shift_of_1000_pending_rows_finishes_inside_the_call_budget() {
    const CALL_BUDGET_MS: u64 = 3_000;
    let f = fx(&["s1"]);
    f.key("s1");
    for ordinal in 0..1_000 {
        let message = f
            .send(&format!("backlog-{ordinal}"), &["s1"], 1_000)
            .unwrap();
        // Half of the backlog physical, half logical: both arms of the shift.
        if ordinal % 2 == 0 {
            f.drain(&message);
        }
    }
    f.at(300);
    f.close("s1").unwrap();
    f.at(800);

    f.clock.1.set(std::time::Instant::now()).unwrap();
    let call = CallBudget {
        deadline: MonoInstant(f.clock.monotonic_now().0 + CALL_BUDGET_MS),
        cancellation: Cancellation::default(),
    };
    StorePort::service_operation(
        &f.store,
        ServiceOperation::UnretireSeat(UnretireSeat {
            seat: SeatId::new("s1"),
            operation: OperationId::new("u"),
        }),
        &f.connection,
        &f.gate,
        &call,
        None,
    )
    .unwrap();
    assert!(!call.is_exhausted(f.clock.as_ref()));

    // 100 + 1000 at send time, moved by the 500 ms window.
    assert_eq!(
        f.int("SELECT count(*) FROM receipt_state WHERE seat_id='s1' AND deadline_at=1600"),
        1_000
    );
    assert_eq!(
        f.int("SELECT count(*) FROM receipt_state WHERE seat_id='s1' AND deadline_at<>1600"),
        0
    );
}

/// Kills: the freeze reaching what the seat is owed: an ACK request of its own losing or moving its
/// deadline, or pending-receipts hiding it on reattach.
#[test]
fn own_ack_request_keeps_its_deadline_and_shows_in_pending_receipts() {
    let f = fx(&["s1", "s2"]);
    f.key("s1");
    let thread = ThreadId::new("t");
    let message = f
        .native_send("s1", &thread, None, &["s2"], Some(1_000), "ask")
        .unwrap();
    f.drain(&message);
    let deadline = f.receipt(&message, "s2").deadline_at;
    assert_eq!(deadline, Some(1_100));
    f.at(300);
    f.close("s1").unwrap();
    assert_eq!(f.receipt(&message, "s2").deadline_at, deadline, "frozen");
    assert_eq!(
        f.receipt(&message, "s2").state,
        EffectiveReceiptState::Pending
    );
    f.at(800);
    f.unretire("s1", "u").unwrap();
    assert_eq!(f.receipt(&message, "s2").deadline_at, deadline, "unretired");
    f.reattach("s1");
    f.at(2_000);

    let result = StorePort::query(
        &f.store,
        &Command::PendingReceipts(PendingReceiptsQuery {
            seat: None,
            thread: Some(thread),
            page: page(),
        }),
        &read_context(INSTANCE),
        &budget(),
    )
    .unwrap();
    let CommandResult::PendingReceipts(pending) = result else {
        panic!("pending answered {result:?}");
    };
    let row = pending
        .items
        .iter()
        .find(|row| row.message == message)
        .expect("its request is listed on reattach");
    assert_eq!(row.seat, SeatId::new("s2"));
    assert_eq!(row.sender, Some(SeatId::new("s1")));
    assert_eq!(row.deadline, Some(UtcMillis(1_100)));
    assert!(row.overdue, "past its unmoved deadline");
}

/// Kills: a frozen inviter losing its own overdue warning, end to end through the warnings read.
#[test]
fn own_invitation_overdue_warning_is_visible() {
    let f = fx(&["s1", "s2"]);
    f.key("s1");
    let thread = f.native_thread("s1", "create");
    f.native_invite("s1", &thread, "s2", Some(50), "ask")
        .unwrap();
    f.close("s1").unwrap();
    f.at(1_000);
    let warning: String = {
        let mut db = f.db();
        invitation_due::scan_invitation_due_batch(&f.store.context, &mut db, None, 10).unwrap();
        db.query_row(
            "SELECT warning_message_id FROM invitations WHERE seat_id='s2'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };
    f.unretire("s1", "u").unwrap();
    f.reattach("s1");
    let result = StorePort::query(
        &f.store,
        &Command::Warnings(WarningsQuery {
            seat: SeatId::new("s1"),
            page: page(),
        }),
        &read_context(INSTANCE),
        &budget(),
    )
    .unwrap();
    let json = serde_json::to_string(&result).unwrap();
    assert!(json.contains(&warning), "{json}");
}

/// A peer's reads, over real fact exchange.
#[cfg(feature = "test-support")]
mod peer {
    use super::*;
    use crate::{
        node::{NodeIdentity, NodeUuid, tag_for},
        ports::RetirementInputs,
        protocol::ids::{generate_public_id, prefix},
        store::{connection::StoreContext, journal},
        sync::facts::FactRegistry,
        test_support::{TEST_HOST_ENDPOINT, fact_exchange, isolation::TestIsolation},
    };

    /// One journal-open store of node `seed`, exchanging facts with the other's.
    struct Peer {
        store: SqliteStore,
        id: NodeUuid,
        clock: Arc<TestClock>,
        instance: String,
        author: ServiceAuthorId,
        _isolation: TestIsolation,
    }

    fn peer_node(seed: u8) -> Peer {
        let isolation = TestIsolation::new("reversible-reads");
        let clock = Arc::new(TestClock(AtomicI64::new(1_000), Default::default()));
        let context = StoreContext::new(isolation.path("store.db"), clock.clone())
            .with_journal_seed([seed; 32])
            .with_test_host();
        context.seed_incarnations(u64::from(seed));
        let identity = Arc::new(NodeIdentity::from_seed([seed; 32]));
        let id = identity.uuid();
        let mut writer = context.open_writer().unwrap();
        journal::open(
            &context,
            &mut writer,
            identity,
            "reversible-reads",
            FactRegistry::builtins(),
        )
        .unwrap();
        drop(writer);
        context.attach_host(TEST_HOST_ENDPOINT).unwrap();
        let store = SqliteStore::new(context, id.to_string(), StoreSettings::default()).unwrap();
        let mut db = store.context().open_writer().unwrap();
        let instance: String = db
            .query_row("SELECT id FROM local_node", [], |r| r.get(0))
            .unwrap();
        let tx = db.transaction().unwrap();
        let author =
            service_substrate::ensure_reserved_author(&tx, &instance, UtcMillis(0)).unwrap();
        tx.commit().unwrap();
        Peer {
            store,
            id,
            clock,
            instance,
            author,
            _isolation: isolation,
        }
    }

    impl Peer {
        fn run<R>(
            &self,
            f: impl FnOnce(&Transaction<'_>, DecisionInstant) -> Result<R, ApiError>,
        ) -> Result<R, ApiError> {
            let mut conn = self.store.context().open_writer().unwrap();
            self.store
                .context()
                .execute_decision(&mut conn, |_| Ok(()), |tx, d, ()| f(tx, d))
        }
        /// A published, resolved local seat holding a key.
        fn key_holder(&self) -> SeatId {
            let seat = SeatId::new(generate_public_id(prefix::SEAT, &tag_for(self.id, 0)));
            let db = self.store.context().open_writer().unwrap();
            db.execute(
                "INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,home_node,is_local,host_id) \
                 VALUES (?1,?2,'resolved','native','p-'||?1,1,1,0,?2,1,(SELECT min(host_id) FROM herdr_hosts))",
                params![seat.as_str(), self.instance],
            )
            .unwrap();
            db.execute(
                "INSERT INTO continuity_keys(key,owner_author_id,seat_id,reserved_at,bound_at,bound_by) VALUES ('k-'||?2,?1,?2,1,1,'service_reservation')",
                params![self.author.as_str(), seat.as_str()],
            )
            .unwrap();
            self.run(|tx, d| seats::ensure_published(tx, self.store.context(), d, &seat))
                .unwrap();
            seat
        }
        fn close(&self, seat: &SeatId) {
            self.run(|tx, d| {
                control::begin_retirement_authored(
                    tx,
                    self.store.context(),
                    d,
                    RetireReason::Closed,
                    seat.clone(),
                    &self.instance,
                    RetirementInputs {
                        boot: "boot".into(),
                        epoch: 1,
                        target: "target".into(),
                        generation: 1,
                        closure_evidence: None,
                        absence_evidence: None,
                    },
                )
            })
            .unwrap();
        }
        fn unretire(&self, seat: &SeatId) {
            self.run(|tx, d| {
                unretire(
                    tx,
                    self.store.context(),
                    &self.instance,
                    &self.author,
                    &UnretireSeat {
                        seat: seat.clone(),
                        operation: OperationId::new(format!("u-{}", uuid::Uuid::new_v4())),
                    },
                    d,
                )
            })
            .unwrap();
        }
        fn finalize(&self, seat: &SeatId) {
            self.run(|tx, d| {
                finalize(
                    tx,
                    self.store.context(),
                    &self.instance,
                    FinalizeBy::Service(&self.author),
                    seat,
                    d,
                )
            })
            .unwrap();
        }
        fn inspect(&self, seat: &SeatId) -> SeatInspection {
            inspect_on(&self.store, &self.instance, seat)
        }
    }

    /// Kills: a peer's retired row reading final while the seat can still return, or a retirement state
    /// left behind once `seat.unretired` restored it.
    #[test]
    fn peer_reads_reversible_then_absent_after_unretired() {
        let (home, peer) = (peer_node(111), peer_node(112));
        let seat = home.key_holder();
        fact_exchange::sync(&home.store, &peer.store);
        assert_eq!(peer.inspect(&seat).retirement_state, None, "remote");

        home.clock.0.store(2_000, Ordering::SeqCst);
        home.close(&seat);
        fact_exchange::sync(&home.store, &peer.store);
        let frozen = peer.inspect(&seat);
        assert_eq!(frozen.summary.continuity, ContinuityStatus::Retired);
        assert_eq!(
            frozen.retirement_state,
            Some(SeatRetirementState::Reversible)
        );

        home.clock.0.store(2_500, Ordering::SeqCst);
        home.unretire(&seat);
        fact_exchange::sync(&home.store, &peer.store);
        let back = peer.inspect(&seat);
        assert_eq!(back.summary.continuity, ContinuityStatus::Remote);
        assert_eq!(back.retirement_state, None);

        // The second episode, finalized: final reads absent on the peer too.
        home.clock.0.store(3_000, Ordering::SeqCst);
        home.close(&seat);
        home.clock.0.store(3_500, Ordering::SeqCst);
        home.finalize(&seat);
        fact_exchange::sync(&home.store, &peer.store);
        let finalized = peer.inspect(&seat);
        assert_eq!(finalized.summary.continuity, ContinuityStatus::Retired);
        assert_eq!(finalized.retirement_state, None, "final is absent");
    }

    /// Kills: the key crossing to a peer in a fact, at any point of the window. The home node holds it
    /// throughout, so the absence is the peer's and not the fixture's.
    #[test]
    fn continuity_key_is_absent_on_a_peer() {
        let (home, peer) = (peer_node(113), peer_node(114));
        let seat = home.key_holder();
        let check = || {
            fact_exchange::sync(&home.store, &peer.store);
            assert!(home.inspect(&seat).continuity_key.is_some());
            assert_eq!(peer.inspect(&seat).continuity_key, None);
        };
        check();
        home.close(&seat);
        check();
        home.unretire(&seat);
        check();
    }
}
