use crate::{
    ports::{CooperativePermitRequest, ReadContext, RegisterAvailableRequest, StorePort},
    protocol::{
        authority::{CallerClaim, CallerRole, Harness, ObligationRef},
        commands::{CheckIn, CheckInMode},
        ids::*,
        results::{CommandResult, ErrorCode},
        time::{CallBudget, Clock, MonoInstant, UtcMillis},
    },
    store::{SqliteStore, StoreSettings, connection::StoreContext, schema},
};
use rusqlite::Connection;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

pub(crate) struct TestClock(AtomicU64);
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(self.0.load(Ordering::SeqCst) as i64)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(self.0.load(Ordering::SeqCst))
    }
}
struct PrivateDirectory(std::path::PathBuf);
impl Drop for PrivateDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
// Field order drops the SQLite owner before the last directory reference.
pub(crate) struct OwnedFixture<T> {
    pub(crate) value: T,
    _directory: Arc<PrivateDirectory>,
}
impl<T> std::ops::Deref for OwnedFixture<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}
impl<T> std::ops::DerefMut for OwnedFixture<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.value
    }
}
pub(crate) fn fixture() -> (
    OwnedFixture<SqliteStore>,
    OwnedFixture<Connection>,
    Arc<TestClock>,
) {
    let (store, conn, clock, _) = fixture_with_context();
    (store, conn, clock)
}
fn fixture_with_context() -> (
    OwnedFixture<SqliteStore>,
    OwnedFixture<Connection>,
    Arc<TestClock>,
    StoreContext,
) {
    let directory = Arc::new(PrivateDirectory(
        std::env::temp_dir().join(format!("cooperative-fixture-{}", uuid::Uuid::new_v4())),
    ));
    std::fs::create_dir(&directory.0).unwrap();
    let clock = Arc::new(TestClock(AtomicU64::new(100)));
    let context = StoreContext::new(directory.0.join("store.db"), clock.clone());
    let directory_path = directory.0.join("store.db");
    let conn = context.open_writer().unwrap();
    conn.execute(
        "INSERT INTO host_instances(id,created_at,host_boot,host_epoch) VALUES ('i',0,'b',1)",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('s','i','resolved','native','p',0,0,0)", []).unwrap();
    conn.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observation_sequence,provenance,occupancy,ui_state,top_level_occupant,observed_at,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','p','b',1,0,1,'fresh','unknown','unknown',0,0,'term-'||'p','inc','coherent_enumeration',1)", []).unwrap();
    (
        OwnedFixture {
            value: SqliteStore::new(context, "i", StoreSettings::default()).unwrap(),
            _directory: directory.clone(),
        },
        OwnedFixture {
            value: conn,
            _directory: directory,
        },
        clock.clone(),
        StoreContext::new(directory_path, clock),
    )
}
pub(crate) fn claim() -> CallerClaim {
    CallerClaim {
        instance: "i".into(),
        seat: SeatId::new("s"),
        binding_generation: 0,
        role: CallerRole::TopLevel,
        harness: Harness::Codex,
        native_session: NativeSessionId::new("plugin_context:n"),
        execution: ExecutionId::new("00000000-0000-4000-8000-000000000001"),
        target: HostTargetId::new("p"),
    }
}
pub(crate) fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Default::default(),
    }
}
fn wait_for_domain_writer(store: &SqliteStore) -> bool {
    let start = std::time::Instant::now();
    loop {
        if matches!(
            store.writer.try_lock(),
            Err(std::sync::TryLockError::WouldBlock)
        ) {
            return true;
        }
        if start.elapsed() >= std::time::Duration::from_millis(500) {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}
fn request(command: &CheckIn) -> CooperativePermitRequest {
    CooperativePermitRequest {
        claim: command.claim.clone(),
        operation: command.operation.clone(),
        obligation: ObligationRef::CheckIn(command.claim.seat.clone()),
        payload_hash: schema::canonical_digest(&("check_in", &command.mode, &command.claim))
            .unwrap(),
        check_in_mode: Some(command.mode),
    }
}
#[test]
fn mapped_generation_zero_can_issue_cooperative_permit_without_native_attestation() {
    let (store, conn, _) = fixture();
    let command = CheckIn {
        mode: CheckInMode::Lifecycle {
            expected_binding_generation: 0,
        },
        claim: claim(),
        operation: OperationId::new("op"),
    };
    assert!(
        store
            .issue_cooperative_permit(request(&command), &budget())
            .is_ok()
    );
    let verified: Option<String> = conn
        .query_row("SELECT verified_execution FROM observed_targets", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(verified, None);
}
pub(crate) fn check_in(
    store: &SqliteStore,
    command: CheckIn,
) -> Result<crate::protocol::results::CheckInResult, crate::protocol::results::ApiError> {
    check_in_as(store, command, None)
}
fn check_in_as(
    store: &SqliteStore,
    command: CheckIn,
    operator: Option<crate::protocol::authority::OperatorActor>,
) -> Result<crate::protocol::results::CheckInResult, crate::protocol::results::ApiError> {
    let permit = store.issue_cooperative_permit(request(&command), &budget())?;
    let result = store.register_available(
        RegisterAvailableRequest {
            command,
            read: ReadContext {
                instance: "i".into(),
                output: Default::default(),
                operation_scope: None,
            },
            operator,
        },
        permit,
        &budget(),
    )?;
    let CommandResult::CheckedIn(result) = result else {
        panic!("wrong result")
    };
    Ok(result)
}
pub(crate) fn lifecycle(context: CallerClaim, operation: &str) -> CheckIn {
    CheckIn {
        mode: CheckInMode::Lifecycle {
            expected_binding_generation: context.binding_generation,
        },
        claim: context,
        operation: OperationId::new(operation),
    }
}
#[test]
fn lifecycle_cas_current_and_response_loss_replay_preserve_context_and_anchors() {
    let (store, conn, _) = fixture();
    let initial = lifecycle(claim(), "initial");
    let result = check_in(&store, initial.clone()).unwrap();
    assert_eq!(result.context.binding_generation, 1);
    assert_eq!(check_in(&store, initial.clone()).unwrap(), result);
    let current = CheckIn {
        mode: CheckInMode::Current,
        claim: result.context.clone(),
        operation: OperationId::new("current"),
    };
    let repeated = check_in(&store, current).unwrap();
    assert_eq!(repeated.context, result.context);
    let anchors: i64 = conn
        .query_row("SELECT count(*) FROM seat_availability", [], |r| r.get(0))
        .unwrap();
    assert_eq!(anchors, 1);
    let mut replacement = result.context.clone();
    replacement.execution = ExecutionId::new("00000000-0000-4000-8000-000000000002");
    let successor = check_in(&store, lifecycle(replacement, "successor")).unwrap();
    assert_eq!(successor.context.binding_generation, 2);
    let historical = check_in(&store, initial).unwrap();
    assert_eq!(
        serde_json::to_value(&historical).unwrap()["context_disposition"],
        "historical"
    );
    assert_eq!(historical.context, result.context);
    assert_eq!(historical.inbox, result.inbox);
    assert_eq!(historical.warnings, result.warnings);
    let stale = CheckIn {
        mode: CheckInMode::Current,
        claim: result.context,
        operation: OperationId::new("stale"),
    };
    assert_eq!(
        check_in(&store, stale).unwrap_err().code,
        ErrorCode::CallerUnverified
    );
    assert_eq!(
        conn.query_row("SELECT generation FROM seats WHERE id='s'", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
}
fn obligations(conn: &Connection) {
    conn.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence) VALUES ('t','i','topic','goal',0,0,2)",[]).unwrap();
    conn.execute(
        "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t','s','invited')",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at) VALUES ('inv','t','s',1,'pending',1,0,1000,1000)",[]).unwrap();
    conn.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,decision_seq,body,decision_at) VALUES ('m','i','t',1,'ordinary',100,'mail',0)",[]).unwrap();
    conn.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) VALUES ('m','t','s','pending',1000,0,1000)",[]).unwrap();
}
#[test]
fn cooperative_accept_and_ack_are_explicit_and_preserve_started_deadlines() {
    use crate::protocol::commands::{Accept, Ack, PermitMutation};
    let (store, conn, _) = fixture();
    obligations(&conn);
    let result = check_in(&store, lifecycle(claim(), "initial")).unwrap();
    assert_eq!(
        conn.query_row("SELECT state FROM receipts", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "pending"
    );
    assert_eq!(
        conn.query_row("SELECT state FROM invitations", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "pending"
    );
    let accept = Accept {
        claim: result.context.clone(),
        thread: ThreadId::new("t"),
        operation: OperationId::new("accept"),
    };
    let digest = super::control::cooperative_payload_hash("accept", &accept).unwrap();
    let permit = store
        .issue_cooperative_permit(
            CooperativePermitRequest {
                claim: accept.claim.clone(),
                operation: accept.operation.clone(),
                obligation: ObligationRef::Invitation(InvitationId::new("inv")),
                payload_hash: digest,
                check_in_mode: None,
            },
            &budget(),
        )
        .unwrap();
    assert!(matches!(
        store
            .mutate(PermitMutation::Accept(accept), permit, &budget())
            .unwrap(),
        CommandResult::Accepted(_)
    ));
    let ack = Ack {
        claim: result.context,
        messages: vec![MessageId::new("m")],
        operation: OperationId::new("ack"),
    };
    let digest = schema::canonical_digest(&crate::store::receipts::ack_payload(&ack)).unwrap();
    let permit = store
        .issue_cooperative_permit(
            CooperativePermitRequest {
                claim: ack.claim.clone(),
                operation: ack.operation.clone(),
                obligation: ObligationRef::CheckIn(SeatId::new("s")),
                payload_hash: digest,
                check_in_mode: None,
            },
            &budget(),
        )
        .unwrap();
    assert!(matches!(
        store
            .mutate(PermitMutation::Ack(ack), permit, &budget())
            .unwrap(),
        CommandResult::Acknowledged(_)
    ));
    assert_eq!(
        conn.query_row("SELECT state FROM receipts", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "acked"
    );
    assert_eq!(
        conn.query_row("SELECT deadline_at FROM receipts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1000
    );
    let observation: String = conn
        .query_row("SELECT ack_observation FROM receipts", [], |r| r.get(0))
        .unwrap();
    assert!(observation.contains("cooperative_top_level"));
}
#[test]
fn cooperative_send_uses_claim_context_and_keeps_exact_replay() {
    use crate::{
        ports::{DurableWorkAdmission, SendPreparationProgress},
        protocol::commands::{CreateThread, PermitMutation, SendMessage},
    };
    let (store, conn, _) = fixture();
    let result = check_in(&store, lifecycle(claim(), "initial")).unwrap();
    let create = CreateThread {
        claim: result.context.clone(),
        topic: "topic".into(),
        goal: "goal".into(),
        operation: OperationId::new("create"),
    };
    let permit = store
        .issue_cooperative_permit(
            CooperativePermitRequest {
                claim: create.claim.clone(),
                operation: create.operation.clone(),
                obligation: ObligationRef::CheckIn(SeatId::new("s")),
                payload_hash: super::control::cooperative_payload_hash("create_thread", &create)
                    .unwrap(),
                check_in_mode: None,
            },
            &budget(),
        )
        .unwrap();
    let CommandResult::ThreadCreated(thread) = store
        .mutate(PermitMutation::CreateThread(create), permit, &budget())
        .unwrap()
    else {
        panic!()
    };
    let send = SendMessage {
        claim: result.context,
        thread,
        body: "explicit model mail".into(),
        invited_recipients: vec![],
        deadline_millis: None,
        operation: OperationId::new("send"),
        relays_user: false,
    };
    loop {
        match store
            .prepare_send_step(&send, DurableWorkAdmission { max_units: 16 }, &budget())
            .unwrap()
        {
            SendPreparationProgress::Ready { .. } => break,
            SendPreparationProgress::More { .. } => {}
            SendPreparationProgress::Committed(_) => panic!(),
        }
    }
    let permit_request = CooperativePermitRequest {
        claim: send.claim.clone(),
        operation: send.operation.clone(),
        obligation: ObligationRef::Control(send.thread.clone()),
        payload_hash: schema::canonical_digest(&crate::store::messages::send_payload(&send))
            .unwrap(),
        check_in_mode: None,
    };
    let permit = store
        .issue_cooperative_permit(permit_request.clone(), &budget())
        .unwrap();
    let first = store
        .mutate(PermitMutation::SendMessage(send.clone()), permit, &budget())
        .unwrap();
    let permit = store
        .issue_cooperative_permit(permit_request, &budget())
        .unwrap();
    assert_eq!(
        store
            .mutate(PermitMutation::SendMessage(send), permit, &budget())
            .unwrap(),
        first
    );
    let observation: String = conn
        .query_row(
            "SELECT native_observation FROM messages WHERE kind='ordinary'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(observation.contains("cooperative_top_level"));
}
#[test]
fn cooperative_operation_reuse_across_control_kinds_is_rejected() {
    use crate::protocol::commands::{Accept, Leave, PermitMutation};
    let (store, conn, _) = fixture();
    obligations(&conn);
    let result = check_in(&store, lifecycle(claim(), "initial")).unwrap();
    let accept = Accept {
        claim: result.context.clone(),
        thread: ThreadId::new("t"),
        operation: OperationId::new("same-op"),
    };
    let digest = super::control::cooperative_payload_hash("accept", &accept).unwrap();
    let request = CooperativePermitRequest {
        claim: accept.claim.clone(),
        operation: accept.operation.clone(),
        obligation: ObligationRef::Invitation(InvitationId::new("inv")),
        payload_hash: digest,
        check_in_mode: None,
    };
    let permit = store
        .issue_cooperative_permit(request.clone(), &budget())
        .unwrap();
    store
        .mutate(PermitMutation::Accept(accept.clone()), permit, &budget())
        .unwrap();
    let leave = Leave {
        claim: accept.claim,
        thread: accept.thread,
        operation: accept.operation,
    };
    let permit = store.issue_cooperative_permit(request, &budget()).unwrap();
    assert_eq!(
        store
            .mutate(PermitMutation::Leave(leave), permit, &budget())
            .unwrap_err()
            .code,
        ErrorCode::OperationPayloadMismatch
    );
}
#[test]
fn lifecycle_cas_conflict_and_non_uuid_execution_are_rejected() {
    let (store, conn, _) = fixture();
    let first = check_in(&store, lifecycle(claim(), "first")).unwrap();
    let mut stale = claim();
    stale.execution = ExecutionId::new("00000000-0000-4000-8000-000000000003");
    assert_eq!(
        check_in(&store, lifecycle(stale, "stale-cas"))
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    let mut invalid = first.context;
    invalid.execution = ExecutionId::new("not-a-uuid");
    assert_eq!(
        check_in(&store, lifecycle(invalid, "bad-execution"))
            .unwrap_err()
            .code,
        ErrorCode::CallerUnverified
    );
    assert_eq!(
        conn.query_row("SELECT generation FROM seats WHERE id='s'", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
#[test]
fn ended_uuid_wrong_instance_child_and_exact_payload_reuse_are_fenced() {
    let (store, conn, _) = fixture();
    let first_command = lifecycle(claim(), "first");
    let first = check_in(&store, first_command.clone()).unwrap();
    let mut replacement = first.context.clone();
    replacement.execution = ExecutionId::new("00000000-0000-4000-8000-000000000002");
    let second = check_in(&store, lifecycle(replacement, "second")).unwrap();
    let mut ended = second.context.clone();
    ended.execution = first.context.execution;
    assert_eq!(
        check_in(&store, lifecycle(ended, "ended"))
            .unwrap_err()
            .code,
        ErrorCode::CallerUnverified
    );
    let mut wrong_instance = first_command.clone();
    wrong_instance.claim.instance = "another-instance".into();
    assert_eq!(
        check_in(&store, wrong_instance).unwrap_err().code,
        ErrorCode::CallerUnverified
    );
    let mut child = second.context.clone();
    child.role = CallerRole::Subagent;
    assert_eq!(
        check_in(
            &store,
            CheckIn {
                mode: CheckInMode::Current,
                claim: child,
                operation: OperationId::new("child")
            }
        )
        .unwrap_err()
        .code,
        ErrorCode::CallerUnverified
    );
    let mut altered = first_command;
    altered.mode = CheckInMode::Current;
    assert_eq!(
        check_in(&store, altered).unwrap_err().code,
        ErrorCode::OperationPayloadMismatch
    );
    // Honest limitation: the same caller can declare top_level. Cooperation,
    // rather than native execution attestation, determines this declared role.
    assert!(
        check_in(
            &store,
            CheckIn {
                mode: CheckInMode::Current,
                claim: second.context,
                operation: OperationId::new("declared-top-level")
            }
        )
        .is_ok()
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM occupant_bindings", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
}
#[test]
fn accept_issuer_freezes_invitation_episode_and_rejects_successor_race() {
    use crate::protocol::commands::{Accept, PermitMutation};
    let (store, conn, _) = fixture();
    obligations(&conn);
    let first = check_in(&store, lifecycle(claim(), "first")).unwrap();
    let accept = Accept {
        claim: first.context,
        thread: ThreadId::new("t"),
        operation: OperationId::new("racing-accept"),
    };
    let permit = store
        .issue_cooperative_permit(
            super::cooperative_permit_request(&PermitMutation::Accept(accept.clone())).unwrap(),
            &budget(),
        )
        .unwrap();
    conn.execute("UPDATE invitations SET state='accepted',accepted_at=100,accepted_actor_seat_id='s',accepted_generation=1,accepted_observation='prior-model' WHERE id='inv'",[]).unwrap();
    conn.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_decision_seq,created_at,frozen_duration_ms,deadline_at) VALUES ('inv-b','t','s',2,'pending',101,100,1000,1100)",[]).unwrap();
    conn.execute("UPDATE memberships SET episode=2,state='invited'", [])
        .unwrap();
    assert_eq!(
        store
            .mutate(PermitMutation::Accept(accept.clone()), permit, &budget())
            .unwrap_err()
            .code,
        ErrorCode::CallerUnverified
    );
    assert_eq!(
        conn.query_row("SELECT state FROM invitations WHERE id='inv-b'", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        "pending"
    );
    let permit = store
        .issue_cooperative_permit(
            super::cooperative_permit_request(&PermitMutation::Accept(accept.clone())).unwrap(),
            &budget(),
        )
        .unwrap();
    store
        .mutate(PermitMutation::Accept(accept), permit, &budget())
        .unwrap();
    assert_eq!(
        conn.query_row("SELECT state FROM invitations WHERE id='inv-b'", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        "accepted"
    );
}
#[test]
fn legacy_v1_startup_adds_index_and_preserves_rows_for_bounded_execution_lookup() {
    let conn = Connection::open_in_memory().unwrap();
    conn.pragma_update(None, "foreign_keys", "ON").unwrap();
    conn.execute_batch(include_str!("../../migrations/0001_initial.sql"))
        .unwrap();
    conn.pragma_update(None, "user_version", 1).unwrap();
    let execution = "00000000-0000-4000-8000-000000000001";
    conn.execute_batch("\
        INSERT INTO host_instances(id,created_at) VALUES ('i',0);\
        INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('s','i','resolved','native',1,0);\
        INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,ended_at) VALUES ('s',1,'p','b',1,'codex','n','00000000-0000-4000-8000-000000000001','cooperative_top_level',0,1);\
    ").unwrap();
    // The accepted v1 database may predate the additive execution index.
    conn.execute_batch("DROP INDEX IF EXISTS occupant_bindings_execution")
        .unwrap();
    schema::initialize(&conn).unwrap();
    assert_eq!(
        conn.query_row("SELECT execution_id FROM occupant_bindings", [], |r| r
            .get::<_, String>(
            0
        ))
        .unwrap(),
        execution
    );
    let mut statement=conn.prepare("EXPLAIN QUERY PLAN SELECT EXISTS(SELECT 1 FROM occupant_bindings WHERE seat_id=?1 AND execution_id=?2)").unwrap();
    let plan: Vec<String> = statement
        .query_map(rusqlite::params!["s", execution], |r| r.get(3))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(
        plan.iter()
            .any(|detail| detail.contains("SEARCH") && detail.contains("execution_id=?")),
        "{plan:?}"
    );
    assert!(
        plan.iter()
            .any(|detail| detail.contains("SEARCH")
                && detail.contains("seat_id=? AND execution_id=?")),
        "{plan:?}"
    );
    assert_eq!(
        conn.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        14
    );
}
fn dispatch_check_in(
    store: &SqliteStore,
    command: CheckIn,
    permit: crate::protocol::authority::MutationPermit,
    call_budget: &CallBudget,
) -> Result<CommandResult, crate::protocol::results::ApiError> {
    store.register_available(
        RegisterAvailableRequest {
            command,
            read: ReadContext {
                instance: "i".into(),
                output: Default::default(),
                operation_scope: None,
            },
            operator: None,
        },
        permit,
        call_budget,
    )
}
#[test]
fn decision_rechecks_permit_expiry_retirement_hold_mapping_and_known_invalidation() {
    for change in ["expiry", "retire", "hold", "mapping", "invalidation"] {
        let (store, conn, clock) = fixture();
        let command = lifecycle(claim(), "initial");
        let permit = store
            .issue_cooperative_permit(request(&command), &budget())
            .unwrap();
        match change {
            "expiry" => clock.0.store(351, Ordering::SeqCst),
            "retire" => {
                conn.execute(
                    "UPDATE seats SET state='retired',retired_at=200,retired_seq=1",
                    [],
                )
                .unwrap();
            }
            "hold" => {
                conn.execute("INSERT INTO recovery_holds(instance_id,target_id,baseline_boot,baseline_epoch,reason) VALUES ('i','p','b',1,'hold')",[]).unwrap();
            }
            "mapping" => {
                conn.execute("UPDATE seats SET target_generation=1", [])
                    .unwrap();
                conn.execute("UPDATE observed_targets SET generation=1", [])
                    .unwrap();
            }
            "invalidation" => {
                conn.execute("UPDATE host_instances SET invalidation_revision=1", [])
                    .unwrap();
            }
            _ => unreachable!(),
        }
        assert_eq!(
            dispatch_check_in(&store, command, permit, &budget())
                .unwrap_err()
                .code,
            ErrorCode::CallerUnverified,
            "{change}"
        );
        assert_eq!(
            conn.query_row("SELECT count(*) FROM operations", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0,
            "{change}"
        );
        assert_eq!(
            conn.query_row("SELECT count(*) FROM seat_availability", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0,
            "{change}"
        );
    }
}
#[test]
fn budget_cancellation_and_deadline_interrupt_sqlite_begin_without_publication() {
    use std::time::{Duration, Instant};
    for cancelled in [true, false] {
        let (store, conn, clock) = fixture();
        let store = Arc::new(store);
        let command = lifecycle(claim(), "initial");
        let call_budget = budget();
        let permit = store
            .issue_cooperative_permit(request(&command), &call_budget)
            .unwrap();
        conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        let task_store = store.clone();
        let task_budget = call_budget.clone();
        let start = Instant::now();
        let task = std::thread::spawn(move || {
            dispatch_check_in(&task_store, command, permit, &task_budget)
        });
        std::thread::sleep(Duration::from_millis(20));
        if cancelled {
            call_budget.cancellation.cancel();
        } else {
            clock.0.store(1000, Ordering::SeqCst);
        }
        let error = task.join().unwrap().unwrap_err();
        assert_eq!(
            error.code,
            if cancelled {
                ErrorCode::Cancelled
            } else {
                ErrorCode::DeadlineExceeded
            }
        );
        assert!(
            start.elapsed() < Duration::from_millis(750),
            "bounded BEGIN took {:?}",
            start.elapsed()
        );
        conn.execute_batch("ROLLBACK").unwrap();
        assert_eq!(
            conn.query_row("SELECT count(*) FROM operations", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            conn.query_row("SELECT generation FROM seats", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
#[test]
fn expired_permit_after_sqlite_writer_wait_cannot_publish() {
    use std::time::Duration;
    let (store, conn, clock) = fixture();
    let store = Arc::new(store);
    let command = lifecycle(claim(), "initial");
    let call_budget = budget();
    let permit = store
        .issue_cooperative_permit(request(&command), &call_budget)
        .unwrap();
    conn.execute_batch("BEGIN IMMEDIATE").unwrap();
    let task_store = store.clone();
    let task =
        std::thread::spawn(move || dispatch_check_in(&task_store, command, permit, &call_budget));
    std::thread::sleep(Duration::from_millis(20));
    clock.0.store(351, Ordering::SeqCst);
    conn.execute_batch("COMMIT").unwrap();
    assert_eq!(
        task.join().unwrap().unwrap_err().code,
        ErrorCode::CallerUnverified
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM operations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn cooperative_offer_failure_rolls_back_binding_anchor_frontier_and_operation() {
    let (store, conn, _) = fixture();
    let command = lifecycle(claim(), "initial");
    let permit = store
        .issue_cooperative_permit(request(&command), &budget())
        .unwrap();
    let mut writer = store.context.open_writer().unwrap();
    let error = super::seats::register_available(
        &store.context,
        &mut writer,
        &command,
        None,
        permit,
        &budget(),
        |_, _, _| {
            Err(super::connection::api_error(
                ErrorCode::InvalidBudget,
                "bounded offer failed",
            ))
        },
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidBudget);
    for table in [
        "occupant_bindings",
        "seat_availability",
        "warning_offer",
        "operations",
    ] {
        assert_eq!(
            conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0,
            "{table}"
        );
    }
    assert_eq!(
        conn.query_row("SELECT generation FROM seats", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        check_in(&store, command)
            .unwrap()
            .context
            .binding_generation,
        1
    );
}
#[test]
fn replay_metadata_preserves_stored_domain_result_and_selected_output_bounds() {
    use crate::protocol::{
        output::{OutputFormat, OutputSpec, encode_selected},
        results::CheckInContextDisposition,
    };
    let (store, conn, _) = fixture();
    obligations(&conn);
    let original = lifecycle(claim(), "initial");
    let first = check_in(&store, original.clone()).unwrap();
    let stored: String = conn
        .query_row(
            "SELECT result_json FROM operations WHERE operation_key='initial'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    conn.execute("UPDATE occupant_bindings SET registered_at=NULL", [])
        .unwrap();
    let suspended = check_in(&store, original.clone()).unwrap();
    assert_eq!(
        suspended.context_disposition,
        CheckInContextDisposition::Current
    );
    assert_eq!(
        conn.query_row("SELECT registered_at FROM occupant_bindings", [], |r| {
            r.get::<_, Option<i64>>(0)
        })
        .unwrap(),
        None
    );
    let mut successor = first.context.clone();
    successor.execution = ExecutionId::new("00000000-0000-4000-8000-000000000002");
    check_in(&store, lifecycle(successor, "successor")).unwrap();
    let historical = check_in(&store, original).unwrap();
    assert_eq!(
        historical.context_disposition,
        CheckInContextDisposition::Historical
    );
    assert_eq!(
        conn.query_row(
            "SELECT result_json FROM operations WHERE operation_key='initial'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        stored
    );
    assert_eq!(historical.context, first.context);
    assert_eq!(historical.inbox, first.inbox);
    assert_eq!(historical.warnings, first.warnings);
    for format in [OutputFormat::Json, OutputFormat::Text] {
        let spec = OutputSpec {
            format,
            ..Default::default()
        };
        let current_bytes =
            encode_selected(&CommandResult::CheckedIn(first.clone()), &spec).unwrap();
        let historical_bytes =
            encode_selected(&CommandResult::CheckedIn(historical.clone()), &spec).unwrap();
        assert!(
            historical_bytes.len()
                <= crate::protocol::pagination::PageRequest::default().max_bytes as usize
        );
        assert_eq!(historical_bytes.len(), current_bytes.len() + 3);
    }
    assert_eq!(
        conn.query_row("SELECT deadline_at FROM receipts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1000
    );
    assert_eq!(
        conn.query_row("SELECT state FROM receipts", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "pending"
    );
}
#[test]
fn lifecycle_generation_exceeds_seat_and_binding_history_maximum() {
    let (store, conn, _) = fixture();
    conn.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,ended_at) VALUES ('s',9,'p','b',1,0,'codex','old','00000000-0000-4000-8000-000000000009','cooperative_top_level',0,1)",[]).unwrap();
    assert_eq!(
        check_in(&store, lifecycle(claim(), "initial"))
            .unwrap()
            .context
            .binding_generation,
        10
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM occupant_bindings WHERE ended_at IS NULL",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}
#[test]
fn additive_index_wrong_shape_is_rejected_without_replacing_it() {
    let (_, conn, _) = fixture();
    conn.execute_batch("DROP INDEX occupant_bindings_execution; CREATE INDEX occupant_bindings_execution ON occupant_bindings(seat_id,ordinal)").unwrap();
    assert_eq!(
        schema::initialize(&conn).unwrap_err().code,
        ErrorCode::IncompatibleSchema
    );
    let second_column: String = conn
        .query_row(
            "SELECT name FROM pragma_index_info('occupant_bindings_execution') WHERE seqno=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(second_column, "ordinal");
}
#[test]
fn canonical_factory_matches_invite_topic_archive_reopen_and_leave_decisions() {
    use crate::protocol::commands::{
        CreateThread, Invite, Leave, PermitMutation, SetTopic, ThreadMutation,
    };
    let (store, conn, _) = fixture();
    let first = check_in(&store, lifecycle(claim(), "initial")).unwrap();
    let run = |mutation: PermitMutation| {
        let input = super::cooperative_permit_request(&mutation).unwrap();
        let permit = store.issue_cooperative_permit(input, &budget()).unwrap();
        store.mutate(mutation, permit, &budget()).unwrap()
    };
    let CommandResult::ThreadCreated(thread) = run(PermitMutation::CreateThread(CreateThread {
        claim: first.context.clone(),
        topic: "topic".into(),
        goal: "goal".into(),
        operation: OperationId::new("create"),
    })) else {
        panic!()
    };
    conn.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('other','i','resolved','native','other-target',0,0,0)",[]).unwrap();
    assert!(matches!(
        run(PermitMutation::Invite(Invite {
            claim: first.context.clone(),
            thread: thread.clone(),
            seat: SeatId::new("other"),
            deadline_millis: Some(1000),
            operation: OperationId::new("invite")
        })),
        CommandResult::Invitation(_)
    ));
    assert_eq!(
        run(PermitMutation::SetTopic(SetTopic {
            claim: first.context.clone(),
            thread: thread.clone(),
            topic: "changed".into(),
            operation: OperationId::new("topic")
        })),
        CommandResult::TopicChanged(thread.clone())
    );
    assert_eq!(
        run(PermitMutation::Archive(ThreadMutation {
            claim: first.context.clone(),
            thread: thread.clone(),
            operation: OperationId::new("archive")
        })),
        CommandResult::Archived(thread.clone())
    );
    assert_eq!(
        run(PermitMutation::Reopen(ThreadMutation {
            claim: first.context.clone(),
            thread: thread.clone(),
            operation: OperationId::new("reopen")
        })),
        CommandResult::Reopened(thread.clone())
    );
    assert_eq!(
        run(PermitMutation::Leave(Leave {
            claim: first.context,
            thread: thread.clone(),
            operation: OperationId::new("leave")
        })),
        CommandResult::Left(thread)
    );
}
#[test]
fn cooperative_ack_budget_remains_live_during_sqlite_writer_wait() {
    use crate::protocol::commands::{Ack, PermitMutation};
    use std::time::{Duration, Instant};
    let (store, conn, _) = fixture();
    obligations(&conn);
    let first = check_in(&store, lifecycle(claim(), "initial")).unwrap();
    let mutation = PermitMutation::Ack(Ack {
        claim: first.context,
        messages: vec![MessageId::new("m")],
        operation: OperationId::new("waiting-ack"),
    });
    let call_budget = budget();
    let permit = store
        .issue_cooperative_permit(
            super::cooperative_permit_request(&mutation).unwrap(),
            &call_budget,
        )
        .unwrap();
    let store = Arc::new(store);
    let task_budget = call_budget.clone();
    let task_store = store.clone();
    conn.execute_batch("BEGIN IMMEDIATE").unwrap();
    let start = Instant::now();
    let task = std::thread::spawn(move || task_store.mutate(mutation, permit, &task_budget));
    std::thread::sleep(Duration::from_millis(20));
    call_budget.cancellation.cancel();
    assert_eq!(task.join().unwrap().unwrap_err().code, ErrorCode::Cancelled);
    assert!(start.elapsed() < Duration::from_millis(750));
    conn.execute_batch("ROLLBACK").unwrap();
    assert_eq!(
        conn.query_row("SELECT state FROM receipts", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "pending"
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM operations WHERE operation_key='waiting-ack'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}
#[test]
fn legacy_check_in_result_diagnostic_preserves_digest_conflict_precedence() {
    let (store, conn, _) = fixture();
    let command = lifecycle(claim(), "initial");
    let first = check_in(&store, command.clone()).unwrap();
    let mut legacy = serde_json::to_value(CommandResult::CheckedIn(first)).unwrap();
    legacy["data"].as_object_mut().unwrap().remove("context");
    legacy["data"]
        .as_object_mut()
        .unwrap()
        .remove("context_disposition");
    let json = serde_json::to_string(&legacy).unwrap();
    conn.execute(
        "UPDATE operations SET result_json=?1 WHERE operation_key='initial'",
        [&json],
    )
    .unwrap();
    assert_eq!(
        check_in(&store, command.clone()).unwrap_err().code,
        ErrorCode::Unsupported
    );
    let mut altered = command;
    altered.mode = CheckInMode::Current;
    assert_eq!(
        check_in(&store, altered).unwrap_err().code,
        ErrorCode::OperationPayloadMismatch
    );
    assert_eq!(
        conn.query_row(
            "SELECT result_json FROM operations WHERE operation_key='initial'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        json
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM occupant_bindings", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn recorded_context_without_disposition_is_presented_from_locked_occupancy() {
    use crate::protocol::results::CheckInContextDisposition;
    let (store, conn, _) = fixture();
    let command = lifecycle(claim(), "initial");
    let first = check_in(&store, command.clone()).unwrap();
    let mut stored = serde_json::to_value(CommandResult::CheckedIn(first.clone())).unwrap();
    stored["data"]
        .as_object_mut()
        .unwrap()
        .remove("context_disposition");
    // Public decoding must stay strict even when storage supports this intermediate row.
    assert!(serde_json::from_value::<CommandResult>(stored.clone()).is_err());
    let raw = serde_json::to_string(&stored).unwrap();
    conn.execute(
        "UPDATE operations SET result_json=?1 WHERE operation_key='initial'",
        [&raw],
    )
    .unwrap();
    conn.execute("UPDATE occupant_bindings SET registered_at=NULL", [])
        .unwrap();
    let current = check_in(&store, command.clone()).unwrap();
    assert_eq!(
        current.context_disposition,
        CheckInContextDisposition::Current
    );
    assert_eq!(current.context, first.context);
    assert_eq!(
        conn.query_row("SELECT registered_at FROM occupant_bindings", [], |r| {
            r.get::<_, Option<i64>>(0)
        })
        .unwrap(),
        None
    );
    let mut successor = first.context.clone();
    successor.execution = ExecutionId::new("00000000-0000-4000-8000-000000000002");
    check_in(&store, lifecycle(successor, "successor")).unwrap();
    let historical = check_in(&store, command).unwrap();
    assert_eq!(
        historical.context_disposition,
        CheckInContextDisposition::Historical
    );
    assert_eq!(historical.context, first.context);
    assert_eq!(historical.inbox, first.inbox);
    assert_eq!(historical.warnings, first.warnings);
    assert_eq!(
        conn.query_row(
            "SELECT result_json FROM operations WHERE operation_key='initial'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        raw
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM seat_availability", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn partial_legacy_context_is_unsupported_without_reconstruction() {
    let (store, conn, _) = fixture();
    let command = lifecycle(claim(), "initial");
    let first = check_in(&store, command.clone()).unwrap();
    let mut stored = serde_json::to_value(CommandResult::CheckedIn(first)).unwrap();
    stored["data"]["context"]
        .as_object_mut()
        .unwrap()
        .remove("role");
    let raw = serde_json::to_string(&stored).unwrap();
    conn.execute(
        "UPDATE operations SET result_json=?1 WHERE operation_key='initial'",
        [&raw],
    )
    .unwrap();
    assert_eq!(
        check_in(&store, command).unwrap_err().code,
        ErrorCode::Unsupported
    );
    assert_eq!(
        conn.query_row(
            "SELECT result_json FROM operations WHERE operation_key='initial'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        raw
    );
    assert_eq!(
        conn.query_row("SELECT generation FROM seats", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn alternate_uuid_spellings_reject_without_binding_or_operation_changes() {
    let (store, conn, _) = fixture();
    let first = check_in(&store, lifecycle(claim(), "initial")).unwrap();
    for (index, spelling) in [
        "00000000000040008000000000000001",
        "ABCDEF00-0000-4000-8000-000000000001",
        "{00000000-0000-4000-8000-000000000001}",
        "urn:uuid:00000000-0000-4000-8000-000000000001",
    ]
    .into_iter()
    .enumerate()
    {
        assert!(uuid::Uuid::parse_str(spelling).is_ok());
        let mut context = first.context.clone();
        context.execution = ExecutionId::new(spelling);
        assert_eq!(
            check_in(&store, lifecycle(context, &format!("alternate-{index}")))
                .unwrap_err()
                .code,
            ErrorCode::CallerUnverified
        );
        assert_eq!(
            conn.query_row("SELECT generation FROM seats", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            conn.query_row("SELECT count(*) FROM occupant_bindings", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            conn.query_row("SELECT count(*) FROM seat_availability", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            conn.query_row("SELECT count(*) FROM operations", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
    let mut context = first.context;
    context.execution = ExecutionId::new("abcdef00-0000-4000-8000-000000000001");
    assert_eq!(
        check_in(&store, lifecycle(context, "canonical-successor"))
            .unwrap()
            .context
            .binding_generation,
        2
    );
}

#[test]
fn stored_noncanonical_context_replay_is_exact_without_normalizing_history() {
    use crate::protocol::results::CheckInContextDisposition;
    let (store, conn, _) = fixture();
    let mut command = lifecycle(claim(), "initial");
    let first = check_in(&store, command.clone()).unwrap();
    let spelling = "00000000000040008000000000000001";
    command.claim.execution = ExecutionId::new(spelling);
    let mut stored = serde_json::to_value(CommandResult::CheckedIn(first.clone())).unwrap();
    stored["data"]["context"]["execution"] = serde_json::json!(spelling);
    let raw = serde_json::to_string(&stored).unwrap();
    let digest = request(&command).payload_hash;
    conn.execute(
        "UPDATE operations SET digest=?1,result_json=?2 WHERE operation_key='initial'",
        rusqlite::params![digest.as_slice(), &raw],
    )
    .unwrap();
    let replay = check_in(&store, command).unwrap();
    assert_eq!(replay.context.execution.as_str(), spelling);
    assert_eq!(
        replay.context_disposition,
        CheckInContextDisposition::Historical
    );
    assert_eq!(replay.inbox, first.inbox);
    assert_eq!(
        conn.query_row(
            "SELECT result_json FROM operations WHERE operation_key='initial'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        raw
    );
    assert_eq!(
        conn.query_row("SELECT execution_id FROM occupant_bindings", [], |r| r
            .get::<_, String>(
            0
        ))
        .unwrap(),
        first.context.execution.as_str()
    );
    assert_eq!(
        conn.query_row("SELECT generation FROM seats", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn independent_current_and_issuance_budgets_stop_accountable_sqlite_waits() {
    use crate::{
        ports::{DurableWorkAdmission, SendPreparationProgress},
        protocol::commands::{Ack, CreateThread, PermitMutation, SendMessage},
    };
    use std::time::{Duration, Instant};
    for route in ["control", "ack", "send"] {
        for constraint in [
            "current_cancel",
            "current_expire",
            "issuance_cancel",
            "issuance_expire",
        ] {
            let (store, conn, clock) = fixture();
            obligations(&conn);
            let first = check_in(&store, lifecycle(claim(), "initial")).unwrap();
            let create = PermitMutation::CreateThread(CreateThread {
                claim: first.context.clone(),
                topic: "topic".into(),
                goal: "goal".into(),
                operation: OperationId::new("create"),
            });
            let mutation = match route {
                "control" => create,
                "ack" => PermitMutation::Ack(Ack {
                    claim: first.context.clone(),
                    messages: vec![MessageId::new("m")],
                    operation: OperationId::new("ack"),
                }),
                "send" => {
                    let permit = store
                        .issue_cooperative_permit(
                            super::cooperative_permit_request(&create).unwrap(),
                            &budget(),
                        )
                        .unwrap();
                    let CommandResult::ThreadCreated(thread) =
                        store.mutate(create, permit, &budget()).unwrap()
                    else {
                        panic!()
                    };
                    let send = SendMessage {
                        claim: first.context.clone(),
                        thread,
                        body: "mail".into(),
                        invited_recipients: vec![],
                        deadline_millis: None,
                        operation: OperationId::new("send"),
                        relays_user: false,
                    };
                    loop {
                        match store
                            .prepare_send_step(
                                &send,
                                DurableWorkAdmission { max_units: 16 },
                                &budget(),
                            )
                            .unwrap()
                        {
                            SendPreparationProgress::Ready { .. } => break,
                            SendPreparationProgress::More { .. } => {}
                            SendPreparationProgress::Committed(_) => panic!(),
                        }
                    }
                    PermitMutation::SendMessage(send)
                }
                _ => unreachable!(),
            };
            let mut issuance = budget();
            let mut current = budget();
            if constraint == "issuance_expire" {
                issuance.deadline = MonoInstant(150);
            } else if constraint == "current_expire" {
                current.deadline = MonoInstant(150);
            }
            let permit = store
                .issue_cooperative_permit(
                    super::cooperative_permit_request(&mutation).unwrap(),
                    &issuance,
                )
                .unwrap();
            let snapshot = || {
                (
                    conn.query_row("SELECT count(*) FROM operations", [], |r| {
                        r.get::<_, i64>(0)
                    })
                    .unwrap(),
                    conn.query_row("SELECT count(*) FROM threads", [], |r| r.get::<_, i64>(0))
                        .unwrap(),
                    conn.query_row("SELECT count(*) FROM messages", [], |r| r.get::<_, i64>(0))
                        .unwrap(),
                    conn.query_row("SELECT state FROM receipts WHERE message_id='m'", [], |r| {
                        r.get::<_, String>(0)
                    })
                    .unwrap(),
                )
            };
            let before = snapshot();
            conn.execute_batch("BEGIN IMMEDIATE").unwrap();
            let store = Arc::new(store);
            let task_store = store.clone();
            let task_current = current.clone();
            let task_mutation = mutation.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            let start = Instant::now();
            let worker = std::thread::spawn(move || {
                tx.send(task_store.mutate(task_mutation, permit, &task_current))
                    .unwrap();
            });
            let acquired_writer = wait_for_domain_writer(&store);
            std::thread::sleep(Duration::from_millis(20));
            match constraint {
                "current_cancel" => current.cancellation.cancel(),
                "issuance_cancel" => issuance.cancellation.cancel(),
                _ => clock.0.store(150, Ordering::SeqCst),
            }
            let result = rx.recv_timeout(Duration::from_millis(500));
            let returned_while_blocked = result.is_ok();
            conn.execute_batch("ROLLBACK").unwrap();
            let result =
                result.unwrap_or_else(|_| rx.recv_timeout(Duration::from_secs(3)).unwrap());
            worker.join().unwrap();
            assert!(
                acquired_writer,
                "{route}/{constraint} never acquired domain writer"
            );
            assert!(
                returned_while_blocked,
                "{route}/{constraint} ignored the live budget while blocked"
            );
            assert!(start.elapsed() < Duration::from_millis(750));
            assert_eq!(
                result.unwrap_err().code,
                if constraint.ends_with("cancel") {
                    ErrorCode::Cancelled
                } else {
                    ErrorCode::DeadlineExceeded
                },
                "{route}/{constraint}"
            );
            assert_eq!(
                snapshot(),
                before,
                "{route}/{constraint} wrote domain state"
            );
            clock.0.store(100, Ordering::SeqCst);
            let fresh = budget();
            let permit = store
                .issue_cooperative_permit(
                    super::cooperative_permit_request(&mutation).unwrap(),
                    &fresh,
                )
                .unwrap();
            assert!(
                store.mutate(mutation, permit, &fresh).is_ok(),
                "{route}/{constraint} writer reuse"
            );
        }
    }
}

#[test]
fn execution_index_rejects_incompatible_key_metadata_without_replacing_it() {
    for shape in [
        "seat_id COLLATE NOCASE,execution_id",
        "seat_id,execution_id COLLATE NOCASE",
        "seat_id DESC,execution_id",
        "seat_id,execution_id DESC",
        "seat_id,lower(execution_id)",
        "execution_id,seat_id",
    ] {
        let (_, conn, _) = fixture();
        conn.execute_batch(&format!("DROP INDEX occupant_bindings_execution; CREATE INDEX occupant_bindings_execution ON occupant_bindings({shape})")).unwrap();
        let raw: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name='occupant_bindings_execution'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            schema::initialize(&conn).unwrap_err().code,
            ErrorCode::IncompatibleSchema,
            "{shape}"
        );
        assert_eq!(
            conn.query_row(
                "SELECT sql FROM sqlite_master WHERE name='occupant_bindings_execution'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            raw
        );
    }
    for sql in [
        "CREATE UNIQUE INDEX occupant_bindings_execution ON occupant_bindings(seat_id,execution_id)",
        "CREATE INDEX occupant_bindings_execution ON occupant_bindings(seat_id,execution_id) WHERE ended_at IS NULL",
    ] {
        let (_, conn, _) = fixture();
        conn.execute_batch(&format!("DROP INDEX occupant_bindings_execution; {sql}"))
            .unwrap();
        assert_eq!(
            schema::initialize(&conn).unwrap_err().code,
            ErrorCode::IncompatibleSchema
        );
        assert_eq!(
            conn.query_row(
                "SELECT sql FROM sqlite_master WHERE name='occupant_bindings_execution'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            sql
        );
    }
}

#[test]
fn cooperative_fixture_removes_sqlite_artifacts_after_normal_and_panic_drop() {
    let path = {
        let (store, conn, _) = fixture();
        let path = std::path::PathBuf::from(conn.path().unwrap());
        assert!(path.exists());
        drop(store);
        drop(conn);
        path
    };
    assert!(!path.exists(), "normal fixture left {}", path.display());
    assert!(!path.parent().unwrap().exists());
    let mut panic_path = None;
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let (_store, conn, _) = fixture();
            panic_path = Some(std::path::PathBuf::from(conn.path().unwrap()));
            panic!("intentional fixture cleanup proof");
        }))
        .is_err()
    );
    let panic_path = panic_path.unwrap();
    assert!(!panic_path.exists());
    assert!(!panic_path.parent().unwrap().exists());
}

#[test]
fn fixture_directory_lives_until_worker_and_reopened_connections_close() {
    let (store, conn, _) = fixture();
    let path = std::path::PathBuf::from(conn.path().unwrap());
    let directory = path.parent().unwrap().to_owned();
    let reopened = Connection::open(&path).unwrap();
    assert_eq!(
        reopened
            .query_row("SELECT count(*) FROM seats", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    drop(reopened);
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (finish_tx, finish_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _store = store;
        started_tx.send(()).unwrap();
        finish_rx.recv().unwrap();
    });
    started_rx.recv().unwrap();
    drop(conn);
    assert!(directory.exists());
    assert!(path.exists());
    assert!(directory.join("store.db-wal").exists());
    assert!(directory.join("store.db-shm").exists());
    finish_tx.send(()).unwrap();
    worker.join().unwrap();
    assert!(!directory.exists());
}

#[test]
fn dual_budget_final_validation_and_postdecision_outcomes_are_preserved() {
    for source in ["current", "issuance"] {
        for expires in [false, true] {
            let (store, mut conn, clock) = fixture();
            let mut current = budget();
            let mut issuance = budget();
            if expires {
                if source == "current" {
                    current.deadline = MonoInstant(150);
                } else {
                    issuance.deadline = MonoInstant(150);
                }
            }
            let error = store
                .context
                .execute_budgeted_decision_with_constraints(
                    &mut conn,
                    &current,
                    Some(&issuance),
                    |_| {
                        if expires {
                            clock.0.store(150, Ordering::SeqCst);
                        } else if source == "current" {
                            current.cancellation.cancel();
                        } else {
                            issuance.cancellation.cancel();
                        }
                        Ok(())
                    },
                    |tx, _, ()| {
                        tx.execute("UPDATE host_instances SET host_epoch=2", [])
                            .map_err(super::connection::store_error)?;
                        Ok(())
                    },
                )
                .unwrap_err();
            assert_eq!(
                error.code,
                if expires {
                    ErrorCode::DeadlineExceeded
                } else {
                    ErrorCode::Cancelled
                }
            );
            assert_eq!(
                conn.query_row("SELECT host_epoch FROM host_instances", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                1
            );
            clock.0.store(100, Ordering::SeqCst);
            let current = budget();
            let issuance = budget();
            let result = store
                .context
                .execute_budgeted_decision_with_constraints(
                    &mut conn,
                    &current,
                    Some(&issuance),
                    |_| Ok(()),
                    |tx, _, ()| {
                        current.cancellation.cancel();
                        issuance.cancellation.cancel();
                        tx.execute("UPDATE host_instances SET host_epoch=2", [])
                            .map_err(super::connection::store_error)?;
                        Ok("accepted")
                    },
                )
                .unwrap();
            assert_eq!(result, "accepted");
            assert_eq!(
                conn.query_row("SELECT host_epoch FROM host_instances", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                2
            );
        }
    }
}

#[test]
fn dual_budget_progress_interrupts_validation_and_keeps_writer_reusable() {
    use std::time::{Duration, Instant};
    for source in ["current", "issuance"] {
        for expires in [false, true] {
            let (store, mut conn, clock) = fixture();
            let mut current = budget();
            let mut issuance = budget();
            if expires {
                if source == "current" {
                    current.deadline = MonoInstant(150);
                } else {
                    issuance.deadline = MonoInstant(150);
                }
            }
            let cancel = if source == "current" {
                current.cancellation.clone()
            } else {
                issuance.cancellation.clone()
            };
            let worker_clock = clock.clone();
            let worker = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(20));
                if expires {
                    worker_clock.0.store(150, Ordering::SeqCst);
                } else {
                    cancel.cancel();
                }
            });
            let start = Instant::now();
            let result=store.context.execute_budgeted_decision_with_constraints(&mut conn,&current,Some(&issuance), |tx| {
            tx.query_row("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000000000) SELECT sum(x) FROM n",[],|row|row.get::<_,i64>(0)).map_err(super::connection::store_error)
        }, |tx,_,_| {tx.execute("UPDATE host_instances SET host_epoch=2",[]).map_err(super::connection::store_error)?;Ok(())});
            worker.join().unwrap();
            assert_eq!(
                result.unwrap_err().code,
                if expires {
                    ErrorCode::DeadlineExceeded
                } else {
                    ErrorCode::Cancelled
                }
            );
            assert!(start.elapsed() < Duration::from_millis(750));
            assert_eq!(
                conn.query_row("SELECT host_epoch FROM host_instances", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                1
            );
            clock.0.store(100, Ordering::SeqCst);
            store
                .context
                .execute_budgeted_decision(&mut conn, &budget(), |_| Ok(()), |_, _, ()| Ok(()))
                .unwrap();
        }
    }
}

#[test]
fn historical_accountable_replay_obeys_its_independent_current_call_budget() {
    use crate::protocol::commands::{CreateThread, PermitMutation};
    use std::time::Duration;
    let (store, conn, _) = fixture();
    let first = check_in(&store, lifecycle(claim(), "initial")).unwrap();
    let mutation = PermitMutation::CreateThread(CreateThread {
        claim: first.context.clone(),
        topic: "topic".into(),
        goal: "goal".into(),
        operation: OperationId::new("create"),
    });
    let input = super::cooperative_permit_request(&mutation).unwrap();
    let permit = store
        .issue_cooperative_permit(input.clone(), &budget())
        .unwrap();
    let original = store.mutate(mutation.clone(), permit, &budget()).unwrap();
    let raw: String = conn
        .query_row(
            "SELECT result_json FROM operations WHERE operation_key='create'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let mut successor = first.context;
    successor.execution = ExecutionId::new("00000000-0000-4000-8000-000000000002");
    check_in(&store, lifecycle(successor, "successor")).unwrap();
    let issuance = budget();
    let current = budget();
    let permit = store
        .issue_cooperative_permit(input.clone(), &issuance)
        .unwrap();
    conn.execute_batch("BEGIN IMMEDIATE").unwrap();
    let store = Arc::new(store);
    let task_store = store.clone();
    let task_current = current.clone();
    let task_mutation = mutation.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        tx.send(task_store.mutate(task_mutation, permit, &task_current))
            .unwrap();
    });
    let acquired_writer = wait_for_domain_writer(&store);
    std::thread::sleep(Duration::from_millis(20));
    current.cancellation.cancel();
    let result = rx.recv_timeout(Duration::from_millis(500));
    let bounded = result.is_ok();
    conn.execute_batch("ROLLBACK").unwrap();
    let result = result.unwrap_or_else(|_| rx.recv_timeout(Duration::from_secs(3)).unwrap());
    worker.join().unwrap();
    assert!(acquired_writer);
    assert!(bounded);
    assert_eq!(result.unwrap_err().code, ErrorCode::Cancelled);
    let fresh = budget();
    let permit = store.issue_cooperative_permit(input, &fresh).unwrap();
    assert_eq!(store.mutate(mutation, permit, &fresh).unwrap(), original);
    assert_eq!(
        conn.query_row(
            "SELECT result_json FROM operations WHERE operation_key='create'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        raw
    );
    assert_eq!(
        conn.query_row("SELECT generation FROM seats", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM threads WHERE topic='topic'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

/// Kills: a cooperative binding stored without the terminal/incarnation of
/// the verified observation that proved its mapping (the ht-4is.11.1 B1
/// defect: such a seat can never be reconfirmed after a host invalidation),
/// a Current re-registration that keeps stale evidence, and removal of the
/// explicit actionable refusal when the observation carries no evidence.
#[test]
fn cooperative_binding_records_observation_evidence_and_refuses_without_it() {
    let (store, conn, _) = fixture();
    let evidence = |conn: &Connection| -> Vec<(i64, Option<String>, Option<String>)> {
        let mut stmt = conn
            .prepare("SELECT generation,terminal_id,incarnation FROM occupant_bindings WHERE ended_at IS NULL")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    let result = check_in(&store, lifecycle(claim(), "initial")).unwrap();
    assert_eq!(
        evidence(&conn),
        vec![(1, Some("term-p".into()), Some("inc".into()))]
    );
    // The host re-verifies the same pane under a new server incarnation.
    conn.execute(
        "UPDATE observed_targets SET terminal_id='term-p2',incarnation='inc2'",
        [],
    )
    .unwrap();
    let current = CheckIn {
        mode: CheckInMode::Current,
        claim: result.context.clone(),
        operation: OperationId::new("current"),
    };
    check_in(&store, current).unwrap();
    assert_eq!(
        evidence(&conn),
        vec![(1, Some("term-p2".into()), Some("inc2".into()))]
    );
    // Without verified terminal/incarnation evidence nothing is registered.
    conn.execute("UPDATE observed_targets SET terminal_id=NULL,incarnation=NULL,incarnation_source_kind=NULL,connection_epoch=NULL", []).unwrap();
    let count = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
    let (bindings, operations, anchors) = (
        count("SELECT count(*) FROM occupant_bindings"),
        count("SELECT count(*) FROM operations"),
        count("SELECT count(*) FROM seat_availability"),
    );
    let mut successor = result.context.clone();
    successor.execution = ExecutionId::new("00000000-0000-4000-8000-000000000002");
    let error = check_in(&store, lifecycle(successor, "no-evidence")).unwrap_err();
    assert_eq!(error.code, ErrorCode::StaleHostObservation, "{error:?}");
    assert!(
        error.detail.contains("reconfirmation evidence"),
        "{error:?}"
    );
    assert_eq!(count("SELECT count(*) FROM occupant_bindings"), bindings);
    assert_eq!(count("SELECT count(*) FROM operations"), operations);
    assert_eq!(count("SELECT count(*) FROM seat_availability"), anchors);
    assert_eq!(
        evidence(&conn),
        vec![(1, Some("term-p2".into()), Some("inc2".into()))]
    );
}

/// The seat's pending programmatic warnings as the digest and check-in count
/// them, and the projection rows kept for the seat (settlement deletes none).
fn pending_notices(conn: &Connection) -> ((u64, bool), i64) {
    conn.execute_batch("BEGIN DEFERRED").unwrap();
    let digest =
        crate::store::attention::seat_digest(conn, "i", &SeatId::new("s"), &|| Ok(())).unwrap();
    let count = crate::store::attention::seat_pending_warnings(conn, "s", &|| Ok(()))
        .unwrap()
        .count();
    let projected: i64 = conn
        .query_row(
            "SELECT count(*) FROM digest_programmatic_warnings WHERE seat_id='s'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    conn.execute_batch("COMMIT").unwrap();
    assert_eq!(digest.digest.warnings.count, count.0);
    (count, projected)
}

fn current(context: &CallerClaim, operation: &str) -> CheckIn {
    CheckIn {
        mode: CheckInMode::Current,
        claim: context.clone(),
        operation: OperationId::new(operation),
    }
}

fn carried(offer: &crate::protocol::results::CheckInResult) -> (Vec<String>, bool) {
    (
        offer
            .notices
            .items
            .iter()
            .map(|notice| notice.warning.as_str().to_owned())
            .collect(),
        offer.notices.has_more,
    )
}

fn names(notices: &[MessageId]) -> Vec<String> {
    notices.iter().map(|m| m.as_str().to_owned()).collect()
}

// Wave-2 fix2 root decision (a), through the real check-in writer: settlement
// is the occupant-scoped monotone offered frontier. Each committed offer
// (Current or lifecycle) carries the capped page of the oldest notices above
// its occupant's frontier (`MAX_NOTICE_PAGE_ITEMS`, 16) and settles exactly
// that page; projection rows are never deleted. 20 notices: the first Current
// offer carries 1..16 (has_more) and leaves 4 pending, the second carries
// 17..20, the third carries none. A projected notice published later is
// carried by the next offer; a notice still in the projection backlog counts
// as pending but is never carried (so never settled) until it is projected.
// A replacement occupant's frontier starts empty: its lifecycle offer carries
// the oldest page again.
// Kills: M-delete-all-settlement (settlement deleting the seat's projection
// rows: the kept-row counts fall), M-settle-beyond-page (the frontier jumps
// past the carried page: 0 pending after the first offer instead of 4), and
// M-frontier-not-occupant-scoped (the successor inherits the predecessor's
// frontier: its first offer carries nothing instead of notices 1..16).
#[test]
fn programmatic_notices_settle_page_by_page_for_the_current_occupant() {
    use crate::protocol::results::MAX_NOTICE_PAGE_ITEMS;
    use crate::test_support::history::{
        drain_programmatic_projection, write_programmatic_warnings,
        write_unprojected_programmatic_warnings,
    };
    assert_eq!(MAX_NOTICE_PAGE_ITEMS, 16);
    let (store, mut conn, _, context) = fixture_with_context();
    conn.execute_batch("\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence) VALUES ('n','i','notices','g',0,0,1);\
        INSERT INTO memberships(thread_id,seat_id,state) VALUES ('n','s','joined');\
        INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('n','s',1,1);\
    ").unwrap();
    let first = check_in(&store, lifecycle(claim(), "initial")).unwrap();
    let notices = write_programmatic_warnings(&context, &mut conn, "n", 20, "a").unwrap();
    let all = names(&notices);
    assert_eq!(pending_notices(&conn), ((20, false), 20));
    let offer = check_in(&store, current(&first.context, "current-1")).unwrap();
    assert_eq!(
        (offer.warning_count, offer.warning_count_has_more),
        (20, false)
    );
    assert_eq!(carried(&offer), (all[..16].to_vec(), true));
    assert_eq!(pending_notices(&conn), ((4, false), 20));
    let offer = check_in(&store, current(&first.context, "current-2")).unwrap();
    assert_eq!(offer.warning_count, 4);
    assert_eq!(carried(&offer), (all[16..].to_vec(), false));
    assert_eq!(pending_notices(&conn), ((0, false), 20));
    let offer = check_in(&store, current(&first.context, "current-3")).unwrap();
    assert_eq!(carried(&offer), (vec![], false));
    assert_eq!(pending_notices(&conn), ((0, false), 20));
    // Every notice, settled or not, stays in the offer's warning history.
    let history: Vec<&str> = offer
        .warnings
        .items
        .iter()
        .map(|w| w.warning.as_str())
        .collect();
    for notice in &all {
        assert!(history.contains(&notice.as_str()), "{history:?}");
    }
    // Published after the offers: one projected, one still in the backlog.
    let later = write_programmatic_warnings(&context, &mut conn, "n", 1, "b").unwrap();
    let unprojected =
        write_unprojected_programmatic_warnings(&context, &mut conn, "n", 1, "c").unwrap();
    assert_eq!(pending_notices(&conn), ((2, false), 21));
    let offer = check_in(&store, current(&first.context, "current-4")).unwrap();
    assert_eq!(offer.warning_count, 2);
    assert_eq!(carried(&offer), (names(&later), false));
    assert_eq!(pending_notices(&conn), ((1, false), 21));
    drain_programmatic_projection(&context, &mut conn, &unprojected[0]).unwrap();
    assert_eq!(pending_notices(&conn), ((1, false), 22));
    let offer = check_in(&store, current(&first.context, "current-5")).unwrap();
    assert_eq!(carried(&offer), (names(&unprojected), false));
    assert_eq!(pending_notices(&conn), ((0, false), 22));
    // Replacement: the successor's frontier is its own, starting empty.
    let mut replacement = first.context.clone();
    replacement.execution = ExecutionId::new("00000000-0000-4000-8000-000000000002");
    let successor = check_in(&store, lifecycle(replacement, "successor")).unwrap();
    assert_eq!(successor.context.binding_generation, 2);
    assert_eq!(
        (successor.warning_count, successor.warning_count_has_more),
        (22, false)
    );
    assert_eq!(carried(&successor), (all[..16].to_vec(), true));
    assert_eq!(pending_notices(&conn), ((6, false), 22));
}

pub(crate) fn human_claim(from: &CallerClaim, generation: u64) -> CallerClaim {
    CallerClaim {
        harness: Harness::Human,
        native_session: NativeSessionId::new("plugin_context:person"),
        execution: ExecutionId::new("00000000-0000-4000-8000-0000000000aa"),
        binding_generation: generation,
        ..from.clone()
    }
}
fn open_bindings(conn: &Connection) -> Vec<(i64, String, String)> {
    let mut stmt = conn
        .prepare("SELECT generation,harness,observation_provenance FROM occupant_bindings WHERE seat_id='s' AND ended_at IS NULL")
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}
fn binding_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT count(*) FROM occupant_bindings", [], |r| r.get(0))
        .unwrap()
}
fn decision_count(conn: &Connection, kind: &str) -> i64 {
    conn.query_row(
        "SELECT count(*) FROM allocation_decisions WHERE kind=?1",
        [kind],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn human_lifecycle_check_in_over_cooperative_top_level_binding_is_refused() {
    let (store, conn, _) = fixture();
    let agent = check_in(&store, lifecycle(claim(), "agent")).unwrap();
    let before = binding_count(&conn);
    let refused = check_in(
        &store,
        lifecycle(
            human_claim(&agent.context, agent.context.binding_generation),
            "human",
        ),
    )
    .unwrap_err();
    assert_eq!(refused.code, ErrorCode::Unauthorized);
    assert!(
        refused.detail.contains("herdr-threads me init --operator"),
        "{}",
        refused.detail
    );
    assert_eq!(
        open_bindings(&conn),
        vec![(1, "codex".to_owned(), "cooperative_top_level".to_owned())]
    );
    assert_eq!(binding_count(&conn), before);
    let generation: i64 = conn
        .query_row("SELECT generation FROM seats WHERE id='s'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(generation, 1);
}

#[test]
fn human_to_agent_lifecycle_check_in_still_replaces_human_binding() {
    let (store, conn, _) = fixture();
    let human = check_in(&store, lifecycle(human_claim(&claim(), 0), "human")).unwrap();
    assert_eq!(
        open_bindings(&conn),
        vec![(1, "human".to_owned(), "operator_human".to_owned())]
    );
    let mut agent = claim();
    agent.binding_generation = human.context.binding_generation;
    let result = check_in(&store, lifecycle(agent, "agent")).unwrap();
    assert_eq!(result.context.binding_generation, 2);
    assert_eq!(
        open_bindings(&conn),
        vec![(2, "codex".to_owned(), "cooperative_top_level".to_owned())]
    );
}

#[test]
fn agent_lifecycle_check_ins_on_own_target_always_replace() {
    let (store, conn, _) = fixture();
    let mut context = claim();
    for (round, event) in ["startup", "clear", "resume"].into_iter().enumerate() {
        let result = check_in(&store, lifecycle(context.clone(), event)).unwrap();
        assert_eq!(
            result.context.binding_generation,
            round as u64 + 1,
            "{event}"
        );
        assert_eq!(
            open_bindings(&conn),
            vec![(
                round as i64 + 1,
                "codex".to_owned(),
                "cooperative_top_level".to_owned()
            )],
            "{event}"
        );
        context = result.context;
        context.execution = ExecutionId::new(format!("00000000-0000-4000-8000-00000000010{round}"));
    }
    assert_eq!(binding_count(&conn), 3);
    let kinds: i64 = conn
        .query_row(
            "SELECT count(*) FROM allocation_decisions WHERE kind='cooperative_continuity'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    assert_eq!(kinds, 0);
}

#[test]
fn operator_human_override_replaces_agent_binding_and_is_audited() {
    use crate::protocol::authority::{OperatorActor, PeerIdentity};
    let (store, conn, _) = fixture();
    let agent = check_in(&store, lifecycle(claim(), "agent")).unwrap();
    let actor = OperatorActor::from_peer(PeerIdentity::from_kernel(501), 501).unwrap();
    let result = check_in_as(
        &store,
        lifecycle(
            human_claim(&agent.context, agent.context.binding_generation),
            "human",
        ),
        Some(actor),
    )
    .unwrap();
    assert_eq!(result.context.binding_generation, 2);
    assert_eq!(
        open_bindings(&conn),
        vec![(2, "human".to_owned(), "operator_human".to_owned())]
    );
    assert_eq!(decision_count(&conn, "operator_human_override"), 1);
    let label: String = conn
        .query_row(
            "SELECT operator_label FROM allocation_decisions WHERE kind='operator_human_override'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(label, "operator:local-user:501");
    // The label never leaks into receipts or messages.
    let leaked: i64 = conn
        .query_row(
            "SELECT (SELECT count(*) FROM receipts WHERE state LIKE '%operator:local-user%') + (SELECT count(*) FROM messages WHERE body LIKE '%operator:local-user%')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(leaked, 0);
}
