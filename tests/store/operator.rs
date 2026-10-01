use crate::{
    ports::{OperatorRequest, StorePort},
    protocol::{
        authority::{OperatorActor, PeerIdentity},
        commands::{OperatorCommand, OperatorFreshSeat, OperatorOrphanInvite, OperatorRebind},
        ids::{HostTargetId, InvitationId, OperationId, SeatId, ThreadId},
        results::{CommandResult, ErrorCode},
        time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis},
    },
    store::{SqliteStore, StoreSettings, connection::StoreContext},
};
use sha2::{Digest, Sha256};
use std::sync::Arc;
struct Fixed;
impl Clock for Fixed {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(100)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(1)
    }
}
fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Cancellation::default(),
    }
}
fn actor(uid: u32) -> OperatorActor {
    OperatorActor::from_peer(PeerIdentity::from_kernel(uid), uid).unwrap()
}
struct Fixture {
    path: std::path::PathBuf,
    store: SqliteStore,
}
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("operator-{}.db", uuid::Uuid::new_v4()));
        let context = StoreContext::new(path.clone(), Arc::new(Fixed));
        let db = context.open_writer().unwrap();
        db.execute_batch("INSERT INTO host_instances(id,created_at) VALUES ('i',0),('foreign',0); INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('s','i','unresolved','native',1,0),('foreign-seat','foreign','unresolved','native',1,0); INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0),('foreign-thread','foreign','topic','goal',0,0);").unwrap();
        Self {
            path,
            store: SqliteStore::new(context, "i", StoreSettings::default()).unwrap(),
        }
    }
    fn db(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(&self.path).unwrap()
    }
    // Literal legacy JSON is independent of the production digest tuple helper.
    fn seed(&self, scope: &str, literal: &str, result: &CommandResult) {
        self.db().execute("INSERT INTO operations(actor_scope,operation_key,digest,result_json,decided_at) VALUES (?1,'op',?2,?3,0)",rusqlite::params![scope,Sha256::digest(literal.as_bytes()).as_slice(),serde_json::to_string(result).unwrap()]).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
fn fresh() -> OperatorCommand {
    OperatorCommand::FreshSeat(OperatorFreshSeat {
        target: HostTargetId::new("p"),
        operation: OperationId::new("op"),
    })
}
fn rebind() -> OperatorCommand {
    OperatorCommand::Rebind(OperatorRebind {
        seat: SeatId::new("s"),
        target: HostTargetId::new("p"),
        operation: OperationId::new("op"),
    })
}
fn orphan() -> OperatorCommand {
    OperatorCommand::OrphanInvite(OperatorOrphanInvite {
        thread: ThreadId::new("t"),
        seat: SeatId::new("s"),
        deadline_millis: Some(90),
        operation: OperationId::new("op"),
    })
}
#[test]
fn replay_miss_is_read_only_and_scope_isolated() {
    let f = Fixture::new();
    let db = f.db();
    let before: i64 = db
        .query_row("PRAGMA data_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        f.store
            .replay_operator(fresh(), actor(501), &budget())
            .unwrap(),
        None
    );
    assert_eq!(
        db.query_row("PRAGMA data_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        before
    );
    f.seed(
        "operator:foreign:local-user:501",
        r#"["operator_fresh","foreign","p",null]"#,
        &CommandResult::OperatorFreshSeat(SeatId::new("old")),
    );
    assert_eq!(
        f.store
            .replay_operator(fresh(), actor(501), &budget())
            .unwrap(),
        None
    );
}
#[test]
fn replay_reads_legacy_literal_digests_and_original_typed_results_without_live_state() {
    for (command, literal, result) in [
        (
            fresh(),
            r#"["operator_fresh","i","p",null]"#,
            CommandResult::OperatorFreshSeat(SeatId::new("old")),
        ),
        (
            rebind(),
            r#"["operator_rebind","i","p","s"]"#,
            CommandResult::OperatorRebound(SeatId::new("s")),
        ),
        (
            orphan(),
            r#"["operator_orphan_invite","t","s",90]"#,
            CommandResult::OperatorInvited(InvitationId::new("inv")),
        ),
    ] {
        let f = Fixture::new();
        f.seed("operator:i:local-user:501", literal, &result);
        f.db().execute_batch("UPDATE seats SET state='retired',retired_at=100; UPDATE host_instances SET host_epoch=5;").unwrap();
        assert_eq!(
            f.store
                .replay_operator(command.clone(), actor(501), &budget())
                .unwrap(),
            Some(result)
        );
        assert_eq!(
            f.store
                .replay_operator(command, actor(502), &budget())
                .unwrap(),
            None
        );
    }
}
#[test]
fn replay_rejects_payload_reuse_wrong_result_and_exhausted_budget() {
    let f = Fixture::new();
    f.seed(
        "operator:i:local-user:501",
        r#"["operator_fresh","i","p",null]"#,
        &CommandResult::OperatorFreshSeat(SeatId::new("old")),
    );
    for command in [
        rebind(),
        orphan(),
        OperatorCommand::FreshSeat(OperatorFreshSeat {
            target: HostTargetId::new("other"),
            operation: OperationId::new("op"),
        }),
    ] {
        assert_eq!(
            f.store
                .replay_operator(command, actor(501), &budget())
                .unwrap_err()
                .code,
            ErrorCode::OperationPayloadMismatch
        );
    }
    f.db()
        .execute(
            "UPDATE operations SET result_json=?1",
            [serde_json::to_string(&CommandResult::SeatResolved(SeatId::new("old"))).unwrap()],
        )
        .unwrap();
    assert_eq!(
        f.store
            .replay_operator(fresh(), actor(501), &budget())
            .unwrap_err()
            .code,
        ErrorCode::StoreCorrupt
    );
    f.db()
        .execute("UPDATE operations SET result_json='garbage'", [])
        .unwrap();
    assert_eq!(
        f.store
            .replay_operator(fresh(), actor(501), &budget())
            .unwrap_err()
            .code,
        ErrorCode::StoreCorrupt
    );
    let b = budget();
    b.cancellation.cancel();
    assert_eq!(
        f.store
            .replay_operator(fresh(), actor(501), &b)
            .unwrap_err()
            .code,
        ErrorCode::Cancelled
    );
    let b = CallBudget {
        deadline: MonoInstant(1),
        cancellation: Cancellation::default(),
    };
    assert_eq!(
        f.store
            .replay_operator(fresh(), actor(501), &b)
            .unwrap_err()
            .code,
        ErrorCode::DeadlineExceeded
    );
}
#[test]
fn orphan_mutation_rejects_foreign_thread_and_seat_before_writing() {
    let f = Fixture::new();
    for (thread, seat) in [("foreign-thread", "foreign-seat"), ("t", "foreign-seat")] {
        let command = OperatorOrphanInvite {
            thread: ThreadId::new(thread),
            seat: SeatId::new(seat),
            deadline_millis: Some(90),
            operation: OperationId::new("foreign-op"),
        };
        assert_eq!(
            f.store
                .mutate_operator(
                    OperatorRequest::OrphanInvite(command),
                    actor(501),
                    &budget()
                )
                .unwrap_err()
                .code,
            ErrorCode::TargetUnresolved
        );
    }
    assert_eq!(
        f.db()
            .query_row("SELECT count(*) FROM operations", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn guarded_repair_cannot_choose_foreign_replay_scope() {
    use crate::ports::*;
    use crate::protocol::ids::{HostBootId, HostCallId};
    let f = Fixture::new();
    f.seed(
        "operator:foreign:local-user:501",
        r#"["operator_fresh","foreign","p",null]"#,
        &CommandResult::OperatorFreshSeat(SeatId::new("old")),
    );
    let OperatorCommand::FreshSeat(command) = fresh() else {
        unreachable!()
    };
    let observation = HostObservation {
        target: command.target.clone(),
        host_boot: HostBootId::new("b"),
        epoch: 1,
        generation: 1,
        observed_at_utc: UtcMillis(100),
        observed_at_mono: MonoInstant(1),
        provenance: ObservationProvenance::FreshCurrentTarget,
        occupant: None,
        ui: HostUiState::Idle,
        terminal: None,
        occupancy: StructuralOccupancy::EmptyShell,
        incarnation: IncarnationEvidence::Unknown,
        execution: ExecutionEvidence::Unknown,
        call_id: HostCallId::new("call"),
        connection_epoch: 1,
        observation_sequence: 1,
        started_at_mono: MonoInstant(1),
        completed_at_mono: MonoInstant(1),
    };
    let guard = OperatorTargetGuard::try_new(
        "foreign",
        &OperatorCommand::FreshSeat(command.clone()),
        observation,
    )
    .unwrap();
    assert_eq!(
        f.store
            .mutate_operator(
                OperatorRequest::FreshSeat(command, guard),
                actor(501),
                &budget()
            )
            .unwrap_err()
            .code,
        ErrorCode::StaleHostObservation
    );
}
#[test]
fn orphan_replay_binds_source_target_and_explicit_deadline() {
    let f = Fixture::new();
    f.seed(
        "operator:i:local-user:501",
        r#"["operator_orphan_invite","t","s",90]"#,
        &CommandResult::OperatorInvited(InvitationId::new("inv")),
    );
    let OperatorCommand::OrphanInvite(command) = orphan() else {
        unreachable!()
    };
    for changed in [
        OperatorOrphanInvite {
            thread: ThreadId::new("other"),
            ..command.clone()
        },
        OperatorOrphanInvite {
            seat: SeatId::new("other"),
            ..command.clone()
        },
        OperatorOrphanInvite {
            deadline_millis: None,
            ..command.clone()
        },
    ] {
        assert_eq!(
            f.store
                .replay_operator(
                    OperatorCommand::OrphanInvite(changed),
                    actor(501),
                    &budget()
                )
                .unwrap_err()
                .code,
            ErrorCode::OperationPayloadMismatch
        );
    }
}
#[test]
fn deciding_transaction_rechecks_commit_after_replay_miss_before_joined_state() {
    let f = Fixture::new();
    assert_eq!(
        f.store
            .replay_operator(orphan(), actor(501), &budget())
            .unwrap(),
        None
    );
    let committed = std::thread::scope(|scope| {
        scope
            .spawn(|| {
                let OperatorCommand::OrphanInvite(command) = orphan() else {
                    unreachable!()
                };
                f.store
                    .mutate_operator(
                        OperatorRequest::OrphanInvite(command),
                        actor(501),
                        &budget(),
                    )
                    .unwrap()
            })
            .join()
            .unwrap()
    });
    let db = rusqlite::Connection::open(&f.path).unwrap();
    db.execute(
        "UPDATE memberships SET state='joined',joined_at=100 WHERE thread_id='t'",
        [],
    )
    .unwrap();
    let OperatorCommand::OrphanInvite(command) = orphan() else {
        unreachable!()
    };
    assert_eq!(
        f.store
            .mutate_operator(
                OperatorRequest::OrphanInvite(command),
                actor(501),
                &budget()
            )
            .unwrap(),
        committed
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM invitations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM operations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
#[test]
fn replay_bounds_encoded_records_and_accepts_maximum_escaped_legacy_id() {
    let f = Fixture::new();
    let maximum = SeatId::new(format!("{}{}", "\"".repeat(64), "\\".repeat(64)));
    let result = CommandResult::OperatorFreshSeat(maximum);
    f.seed(
        "operator:i:local-user:501",
        r#"["operator_fresh","i","p",null]"#,
        &result,
    );
    assert_eq!(
        f.store
            .replay_operator(fresh(), actor(501), &budget())
            .unwrap(),
        Some(result.clone())
    );
    let padded = format!(
        "{}{}",
        " ".repeat(513),
        serde_json::to_string(&result).unwrap()
    );
    f.db()
        .execute("UPDATE operations SET result_json=?1", [padded])
        .unwrap();
    assert_eq!(
        f.store
            .replay_operator(fresh(), actor(501), &budget())
            .unwrap_err()
            .code,
        ErrorCode::StoreCorrupt
    );
}
#[test]
fn replay_bounds_digest_shape_before_decoding_record() {
    let f = Fixture::new();
    f.seed(
        "operator:i:local-user:501",
        r#"["operator_fresh","i","p",null]"#,
        &CommandResult::OperatorFreshSeat(SeatId::new("old")),
    );
    f.db()
        .execute_batch(
            "PRAGMA ignore_check_constraints=ON; UPDATE operations SET digest=zeroblob(100000);",
        )
        .unwrap();
    assert_eq!(
        f.store
            .replay_operator(fresh(), actor(501), &budget())
            .unwrap_err()
            .code,
        ErrorCode::StoreCorrupt
    );
}
