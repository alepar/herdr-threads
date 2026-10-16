//! Nudge threshold storage and its legacy `wake_on` alias (`all` = -1, `nudge` = 0): the
//! migration columns, the accountable sets, the idempotent replay and the refusals.
use crate::protocol::{
    authority::{CallerClaim, CallerRole, Harness, MutationPermit, ObligationRef},
    commands::{SetNudgeThreshold, SetWakeOn, WakeOn},
    ids::*,
    priority::NudgeThreshold,
    results::{CommandResult, ErrorCode, NudgeThresholdState, WakeOnState},
    time::{CallBudget, Clock, MonoInstant, UtcMillis},
};
use crate::store::{connection::StoreContext, control, schema, seats};
use rusqlite::Connection;
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
    let context = StoreContext::new(path, Arc::new(TestClock)).with_test_host();
    let conn = context.open_writer().unwrap();
    conn.execute_batch(
        "UPDATE herdr_hosts SET host_boot='b',host_epoch=1; UPDATE local_node SET decision_seq=1",
    )
    .unwrap();
    conn.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at,unavailability_episode,home_node,is_local,host_id) VALUES ('s','34750f98bd59fcfc946da45aaabe933b','resolved','native','ps',1,1,0,1,(SELECT id FROM local_node),1,(SELECT min(host_id) FROM herdr_hosts))", []).unwrap();
    conn.execute("INSERT INTO observed_targets(host_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ((SELECT min(host_id) FROM herdr_hosts),'ps','b',1,1,0,'fresh','term-ps','inc','coherent_enumeration',1)", []).unwrap();
    conn.execute("INSERT INTO occupant_bindings(seat_id,generation,target_id,host_boot,host_epoch,target_generation,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) VALUES ('s',1,'ps','b',1,1,'claude','ns','00000000-0000-4000-8000-0000000000aa','cooperative_top_level',0,'term-ps','inc')", []).unwrap();
    (context, conn)
}

fn claim(role: CallerRole, generation: u64) -> CallerClaim {
    CallerClaim {
        host: crate::test_support::test_host_locator(),
        instance: "34750f98bd59fcfc946da45aaabe933b".into(),
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
fn threshold_command(value: i64, operation: &str) -> SetNudgeThreshold {
    SetNudgeThreshold {
        value: NudgeThreshold::new(value).unwrap(),
        operation: OperationId::new(operation),
        claim: claim(CallerRole::TopLevel, 1),
    }
}
fn set_threshold(
    context: &StoreContext,
    conn: &mut Connection,
    command: &SetNudgeThreshold,
) -> Result<NudgeThresholdState, crate::protocol::results::ApiError> {
    let digest = control::cooperative_payload_hash("set_nudge_threshold", command).unwrap();
    let permit = MutationPermit::cooperative(
        command.claim.clone(),
        command.operation.clone(),
        ObligationRef::CheckIn(command.claim.seat.clone()),
        digest,
        MonoInstant(50),
        (1, 0),
        budget(),
    );
    match seats::set_nudge_threshold(context, conn, &budget(), command, permit)? {
        CommandResult::NudgeThreshold(state) => Ok(state),
        other => panic!("unexpected result {other:?}"),
    }
}
fn threshold(conn: &Connection) -> i64 {
    i64::from(seats::nudge_threshold(conn, &seat()).unwrap())
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
    assert_eq!(threshold(&conn), 0, "a fresh seat's threshold is 0");
    for read in [
        seats::wake_on(&conn, &SeatId::new("nobody")).map(|_| ()),
        seats::nudge_threshold(&conn, &SeatId::new("nobody")).map(|_| ()),
    ] {
        assert_eq!(read.unwrap_err().code, ErrorCode::NotFound);
    }
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
    assert_eq!(threshold(&conn), -1, "`all` is the threshold -1");
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

/// Kills: a legacy `wake_on` command that stores `all`/`nudge` without moving the threshold, or a
/// `WakeOnQuery` that reports `all` for a positive threshold.
#[test]
fn legacy_wake_on_round_trip_through_the_threshold() {
    let (context, mut conn) = setup();
    set(&context, &mut conn, &command(WakeOn::All, "op1")).unwrap();
    assert_eq!(threshold(&conn), -1);
    set(&context, &mut conn, &command(WakeOn::Nudge, "op2")).unwrap();
    assert_eq!(threshold(&conn), 0);
    assert_eq!(seats::wake_on(&conn, &seat()).unwrap(), WakeOn::Nudge);
    // T = 3 is not expressible as an alias: the legacy read reports `nudge`, and the audit column agrees.
    let state = set_threshold(&context, &mut conn, &threshold_command(3, "op3")).unwrap();
    assert_eq!(
        state,
        NudgeThresholdState {
            seat: seat(),
            threshold: NudgeThreshold::new(3).unwrap(),
            changed: true
        }
    );
    assert_eq!(threshold(&conn), 3);
    assert_eq!(seats::wake_on(&conn, &seat()).unwrap(), WakeOn::Nudge);
    let audit: String = conn
        .query_row("SELECT wake_on FROM seats WHERE id='s'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(audit, "nudge");
    // Setting `nudge` while T = 3 is a change (3 -> 0); the same value again is not.
    let back = set(&context, &mut conn, &command(WakeOn::Nudge, "op4")).unwrap();
    assert!(back.changed);
    assert_eq!(threshold(&conn), 0);
    // A negative threshold clamps to -1 and the legacy read reports `all`.
    set_threshold(&context, &mut conn, &threshold_command(-5, "op5")).unwrap();
    assert_eq!(threshold(&conn), -1);
    assert_eq!(seats::wake_on(&conn, &seat()).unwrap(), WakeOn::All);
    let audit: String = conn
        .query_row("SELECT wake_on FROM seats WHERE id='s'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(audit, "all");
}

#[test]
fn threshold_set_notes_attention_only_on_change_and_replays() {
    let (context, mut conn) = setup();
    let first = set_threshold(&context, &mut conn, &threshold_command(2, "op1")).unwrap();
    assert!(first.changed);
    assert_eq!(wake_work(&conn), Some((1, 1)));
    let again = set_threshold(&context, &mut conn, &threshold_command(2, "op2")).unwrap();
    assert!(!again.changed);
    assert_eq!(wake_work(&conn), Some((1, 1)), "no second bump");
    let replay = set_threshold(&context, &mut conn, &threshold_command(2, "op1")).unwrap();
    assert_eq!(replay, first, "recorded result");
    assert!(set_threshold(&context, &mut conn, &threshold_command(4, "op1")).is_err());
    assert_eq!(threshold(&conn), 2);
}

#[test]
fn threshold_setter_refuses_a_subagent_with_its_own_detail() {
    let (context, mut conn) = setup();
    let mut subagent = threshold_command(5, "op1");
    subagent.claim = claim(CallerRole::Subagent, 1);
    let error = set_threshold(&context, &mut conn, &subagent).unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert!(error.detail.contains("nudge threshold"), "{error:?}");
    let mut legacy = command(WakeOn::All, "op2");
    legacy.claim = claim(CallerRole::Subagent, 1);
    let error = set(&context, &mut conn, &legacy).unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert!(error.detail.contains("wake_on"), "{error:?}");
    assert_eq!(threshold(&conn), 0);
    let mut stale = threshold_command(5, "op3");
    stale.claim = claim(CallerRole::TopLevel, 7);
    assert!(set_threshold(&context, &mut conn, &stale).is_err());
    assert_eq!(threshold(&conn), 0);
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

#[test]
fn a_database_missing_a_wake_on_column_is_incompatible() {
    let (_, conn) = setup();
    conn.execute_batch("ALTER TABLE seats DROP COLUMN wake_on")
        .unwrap();
    assert_eq!(
        schema::initialize(
            &conn,
            || UtcMillis(0),
            &crate::test_support::test_node_seed()
        )
        .unwrap_err()
        .code,
        ErrorCode::IncompatibleSchema
    );
}

const FOREIGN_NODE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

/// A seat homed on another node (no host, no target), plus a directory row naming that node `bx1`.
fn remote_seat(conn: &Connection) -> SeatId {
    conn.execute(
        "INSERT INTO seats(id,instance_id,state,role,generation,created_at,home_node,is_local,remote_kind,host_id,published) \
         VALUES ('r','34750f98bd59fcfc946da45aaabe933b','remote','native',0,0,?1,0,'agent',NULL,1)",
        [FOREIGN_NODE],
    )
    .unwrap();
    crate::store::node_directory::upsert(
        conn,
        &crate::store::node_directory::NodeDirectoryRow {
            node_uuid: FOREIGN_NODE.parse().unwrap(),
            tag: "bx1".parse().unwrap(),
            tag_k: 0,
            host_label: None,
            first_seen_at: 0,
            tag_hlc: None,
        },
    )
    .unwrap();
    SeatId::new("r")
}

#[test]
fn remote_seat_threshold_read_is_refused_naming_its_home_node() {
    let (_, conn) = setup();
    let remote = remote_seat(&conn);
    let errors = [
        seats::nudge_threshold(&conn, &remote)
            .map(|_| ())
            .unwrap_err(),
        seats::wake_on(&conn, &remote).map(|_| ()).unwrap_err(),
    ];
    for error in errors {
        assert_eq!(error.code, ErrorCode::CallerUnverified);
        assert_eq!(
            error.detail,
            "seat r is homed on node bx1; its nudge threshold is set there"
        );
    }
}

#[test]
fn remote_seat_threshold_set_is_refused_before_any_write() {
    let (context, mut conn) = setup();
    let remote = remote_seat(&conn);
    let mut command = threshold_command(3, "op-remote");
    command.claim.seat = remote.clone();
    let before = conn.total_changes();
    let error = set_threshold(&context, &mut conn, &command).unwrap_err();
    assert_eq!(error.code, ErrorCode::CallerUnverified);
    assert!(
        error.detail.contains("homed on node bx1"),
        "{}",
        error.detail
    );
    let mut wake = SetWakeOn {
        value: WakeOn::All,
        operation: OperationId::new("op-remote-wake"),
        claim: command.claim.clone(),
    };
    wake.claim.seat = remote;
    let error = set(&context, &mut conn, &wake).unwrap_err();
    assert!(
        error.detail.contains("homed on node bx1"),
        "{}",
        error.detail
    );
    assert_eq!(conn.total_changes(), before, "a refused set writes nothing");
    let stored: i64 = conn
        .query_row("SELECT nudge_threshold FROM seats WHERE id='r'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(stored, 0);
}
