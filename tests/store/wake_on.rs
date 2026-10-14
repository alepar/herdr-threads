//! DP8 `wake_on` storage: the migration columns, the accountable set, the
//! idempotent replay and the refusals.
use crate::protocol::{
    authority::{CallerClaim, CallerRole, Harness, MutationPermit, ObligationRef},
    commands::{SetWakeOn, WakeOn},
    ids::*,
    results::{CommandResult, ErrorCode, WakeOnState},
    time::{CallBudget, Clock, MonoInstant, UtcMillis},
};
use crate::store::{connection::StoreContext, control, schema, seats};
use rusqlite::{Connection, params};
use std::sync::Arc;

struct TestClock;
impl Clock for TestClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(1000)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(100)
    }
}

fn setup() -> (StoreContext, Connection) {
    let path = crate::test_support::scratch::temp_path(format!(
        "wake-on-test-{}.db",
        uuid::Uuid::new_v4()
    ));
    let context = StoreContext::new(path, Arc::new(TestClock));
    let conn = context.open_writer().unwrap();
    conn.execute(
        "INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'b',1,1)",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,unavailability_episode) VALUES ('s','i','resolved','native','ps',1,1,0,1)", []).unwrap();
    conn.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','ps','b',1,1,0,'fresh','term-ps','inc','coherent_enumeration',1)", []).unwrap();
    conn.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) VALUES ('s',1,'ps','b',1,1,'claude','ns','00000000-0000-4000-8000-0000000000aa','cooperative_top_level',0,'term-ps','inc')", []).unwrap();
    (context, conn)
}

fn claim(role: CallerRole, generation: u64) -> CallerClaim {
    CallerClaim {
        instance: "i".into(),
        seat: SeatId::new("s"),
        binding_generation: generation,
        role,
        harness: Harness::Claude,
        native_session: NativeSessionId::new("ns"),
        execution: ExecutionId::new("00000000-0000-4000-8000-0000000000aa"),
        target: HostTargetId::new("ps"),
        agent_id: None,
        liveness: Some(crate::protocol::authority::LivenessFingerprint::TEST),
    }
}
fn command(value: WakeOn, operation: &str) -> SetWakeOn {
    SetWakeOn {
        value,
        operation: OperationId::new(operation),
        claim: claim(CallerRole::TopLevel, 1),
    }
}
fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(u64::MAX),
        cancellation: Default::default(),
    }
}
fn set(
    context: &StoreContext,
    conn: &mut Connection,
    command: &SetWakeOn,
) -> Result<WakeOnState, crate::protocol::results::ApiError> {
    let digest = control::cooperative_payload_hash("set_wake_on", command).unwrap();
    let permit = MutationPermit::cooperative(
        command.claim.clone(),
        command.operation.clone(),
        ObligationRef::CheckIn(command.claim.seat.clone()),
        digest,
        MonoInstant(50),
        (1, 0),
        budget(),
    );
    match seats::set_wake_on(context, conn, &budget(), command, permit)? {
        CommandResult::WakeOn(state) => Ok(state),
        other => panic!("unexpected result {other:?}"),
    }
}
fn seat() -> SeatId {
    SeatId::new("s")
}
/// `(reason_bits, attention_version)` of the seat's wake_work row, if any.
fn wake_work(conn: &Connection) -> Option<(i64, i64)> {
    conn.query_row(
        "SELECT reason_bits,attention_version FROM wake_work WHERE seat_id='s'",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .ok()
}

#[test]
fn fresh_seat_reads_nudge_and_unknown_seat_is_not_found() {
    let (_, conn) = setup();
    assert_eq!(seats::wake_on(&conn, &seat()).unwrap(), WakeOn::Nudge);
    assert_eq!(
        seats::wake_on(&conn, &SeatId::new("nobody"))
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
}

#[test]
fn set_all_changes_reads_all_and_notes_new_attention() {
    let (context, mut conn) = setup();
    assert_eq!(wake_work(&conn), None);
    let state = set(&context, &mut conn, &command(WakeOn::All, "op1")).unwrap();
    assert_eq!(
        state,
        WakeOnState {
            seat: seat(),
            wake_on: WakeOn::All,
            changed: true
        }
    );
    assert_eq!(seats::wake_on(&conn, &seat()).unwrap(), WakeOn::All);
    assert_eq!(wake_work(&conn), Some((1, 1)), "reason bit 1, version 1");
}

#[test]
fn setting_the_stored_value_again_is_unchanged_and_does_not_bump() {
    let (context, mut conn) = setup();
    set(&context, &mut conn, &command(WakeOn::All, "op1")).unwrap();
    let again = set(&context, &mut conn, &command(WakeOn::All, "op2")).unwrap();
    assert!(!again.changed);
    assert_eq!(again.wake_on, WakeOn::All);
    assert_eq!(wake_work(&conn), Some((1, 1)), "no second bump");
    // Back to nudge is a change and re-judges again.
    let back = set(&context, &mut conn, &command(WakeOn::Nudge, "op3")).unwrap();
    assert!(back.changed);
    assert_eq!(wake_work(&conn), Some((1, 2)));
}

#[test]
fn setting_nudge_on_a_fresh_seat_is_unchanged() {
    let (context, mut conn) = setup();
    let state = set(&context, &mut conn, &command(WakeOn::Nudge, "op1")).unwrap();
    assert!(!state.changed);
    assert_eq!(wake_work(&conn), None);
}

#[test]
fn replay_of_the_same_operation_returns_the_recorded_result() {
    let (context, mut conn) = setup();
    let first = set(&context, &mut conn, &command(WakeOn::All, "op1")).unwrap();
    assert!(first.changed);
    let replay = set(&context, &mut conn, &command(WakeOn::All, "op1")).unwrap();
    assert_eq!(replay, first, "recorded result, still changed: true");
    assert_eq!(wake_work(&conn), Some((1, 1)));
    // The same key with a different value is a digest conflict, not a write.
    assert!(set(&context, &mut conn, &command(WakeOn::Nudge, "op1")).is_err());
    assert_eq!(seats::wake_on(&conn, &seat()).unwrap(), WakeOn::All);
}

#[test]
fn declared_subagent_is_refused() {
    let (context, mut conn) = setup();
    let mut subagent = command(WakeOn::All, "op1");
    subagent.claim = claim(CallerRole::Subagent, 1);
    let error = set(&context, &mut conn, &subagent).unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert_eq!(seats::wake_on(&conn, &seat()).unwrap(), WakeOn::Nudge);
}

#[test]
fn claim_whose_binding_is_not_current_is_refused() {
    let (context, mut conn) = setup();
    let mut stale = command(WakeOn::All, "op1");
    stale.claim = claim(CallerRole::TopLevel, 7);
    assert!(set(&context, &mut conn, &stale).is_err());
    assert_eq!(seats::wake_on(&conn, &seat()).unwrap(), WakeOn::Nudge);
    assert_eq!(wake_work(&conn), None);
}

#[test]
fn value_survives_a_binding_rotation() {
    let (context, mut conn) = setup();
    set(&context, &mut conn, &command(WakeOn::All, "op1")).unwrap();
    conn.execute(
        "UPDATE occupant_bindings SET ended_at=1 WHERE seat_id='s' AND generation=1",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) VALUES ('s',2,'ps','b',1,1,'claude','ns2','00000000-0000-4000-8000-0000000000bb','cooperative_top_level',0,'term-ps','inc')", []).unwrap();
    conn.execute("UPDATE seats SET generation=2 WHERE id='s'", [])
        .unwrap();
    assert_eq!(seats::wake_on(&conn, &seat()).unwrap(), WakeOn::All);
}

#[test]
fn raw_update_to_an_unknown_value_fails_the_check() {
    let (_, conn) = setup();
    let error = conn
        .execute("UPDATE seats SET wake_on='bogus' WHERE id='s'", [])
        .unwrap_err();
    assert!(error.to_string().contains("CHECK"), "{error}");
}

#[test]
fn lazy_ordinal_rejects_zero_and_accepts_null_or_positive() {
    let (_, conn) = setup();
    conn.execute(
        "INSERT INTO wake_work(seat_id,reason_bits,attention_version) VALUES ('s',1,1)",
        [],
    )
    .unwrap();
    let last_lazy = |conn: &Connection| -> Option<i64> {
        conn.query_row(
            "SELECT last_lazy_ordinal FROM wake_work WHERE seat_id='s'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(last_lazy(&conn), None, "default NULL");
    assert!(
        conn.execute(
            "UPDATE wake_work SET last_lazy_ordinal=0 WHERE seat_id='s'",
            []
        )
        .is_err()
    );
    conn.execute(
        "UPDATE wake_work SET last_lazy_ordinal=5 WHERE seat_id='s'",
        [],
    )
    .unwrap();
    assert_eq!(last_lazy(&conn), Some(5));
}

/// A database at the historical schema version 30 (main's released chain), then migrated
/// through V31 and the session-binding migrations after it.
#[test]
fn migration_up_adds_the_columns_with_their_defaults() {
    let previous = crate::store::connection::tests::adapter_historical_database(30);
    previous
        .execute_batch(
            "INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'b',1,1);
             INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,unavailability_episode) VALUES ('old','i','resolved','native','po',1,1,0,1);
             INSERT INTO wake_work(seat_id,reason_bits,attention_version) VALUES ('old',1,1);",
        )
        .unwrap();
    let has_column = |table: &str, column: &str| -> bool {
        previous
            .query_row(
                &format!("SELECT count(*) FROM pragma_table_info('{table}') WHERE name=?1"),
                [column],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
            == 1
    };
    assert!(!has_column("seats", "wake_on") && !has_column("wake_work", "last_lazy_ordinal"));
    schema::initialize(&previous, || UtcMillis(0)).unwrap();
    assert_eq!(
        previous
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        schema::LATEST_VERSION
    );
    assert!(has_column("seats", "wake_on") && has_column("wake_work", "last_lazy_ordinal"));
    let wake_on: String = previous
        .query_row("SELECT wake_on FROM seats WHERE id='old'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(wake_on, "nudge");
    let lazy: Option<i64> = previous
        .query_row(
            "SELECT last_lazy_ordinal FROM wake_work WHERE seat_id='old'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(lazy, None);
    assert!(
        previous
            .execute(
                "UPDATE wake_work SET last_lazy_ordinal=0 WHERE seat_id='old'",
                params![]
            )
            .is_err()
    );
    assert!(
        previous
            .execute("UPDATE seats SET wake_on='bogus' WHERE id='old'", [])
            .is_err()
    );
}

#[test]
fn a_database_missing_a_wake_on_column_is_incompatible() {
    let (_, conn) = setup();
    conn.execute_batch("ALTER TABLE seats DROP COLUMN wake_on")
        .unwrap();
    assert_eq!(
        schema::initialize(&conn, || UtcMillis(0)).unwrap_err().code,
        ErrorCode::IncompatibleSchema
    );
}
