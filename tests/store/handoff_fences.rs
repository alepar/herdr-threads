use super::channel_archival::{fixture, joined_agent, thread};
use herdr_threads::{
    protocol::{
        authority::{CallerClaim, CallerRole, Harness},
        handoff::*,
        ids::*,
        time::UtcMillis,
    },
    store::handoff,
};
pub(super) fn identity() -> HandoffIdentity {
    HandoffIdentity {
        compound: OperationId::new("compound"),
        digest: "a".repeat(64),
        claim: CallerClaim {
            instance: "i".into(),
            seat: SeatId::new("s"),
            target: HostTargetId::new("p"),
            binding_generation: 1,
            role: CallerRole::TopLevel,
            harness: Harness::Codex,
            native_session: NativeSessionId::new("session"),
            execution: ExecutionId::new("00000000-0000-4000-8000-000000000001"),
        },
        thread: Some(ThreadId::new("t")),
        recipient: SeatId::new("s"),
        create_key: OperationId::new("create"),
        invite_key: OperationId::new("invite"),
        send_key: OperationId::new("send"),
    }
}
#[test]
fn handoff_begin_installs_exact_thread_protection_before_any_side_effect() {
    let mut db = fixture();
    thread(&db);
    joined_agent(&db);
    let tx = db.transaction().unwrap();
    let result = handoff::begin_pending(&tx, &identity(), UtcMillis(0)).unwrap();
    assert_eq!(result.state, HandoffState::Live);
    assert!(tx.query_row("SELECT EXISTS(SELECT 1 FROM channel_handoff_fences WHERE thread_id='t' AND state='live')",[],|r|r.get::<_,bool>(0)).unwrap(),"Begin must install durable protection");
    assert_eq!(
        tx.query_row("SELECT count(*) FROM messages", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    tx.commit().unwrap();
}
#[test]
fn handoff_begin_completed_replay_is_historical_but_live_replay_checks_current_guards() {
    let mut db = fixture();
    thread(&db);
    joined_agent(&db);
    let tx = db.transaction().unwrap();
    let id = identity();
    handoff::begin_pending(&tx, &id, UtcMillis(0)).unwrap();
    tx.execute_batch("UPDATE threads SET archived=1 WHERE id='t'")
        .unwrap();
    assert!(
        handoff::begin_pending(&tx, &id, UtcMillis(1)).is_err(),
        "live replay must check current archive guard"
    );
    tx.execute_batch("UPDATE channel_handoff_fences SET state='completed',completed_at=1 WHERE compound='compound'; UPDATE seats SET generation=2 WHERE id='s'").unwrap();
    assert_eq!(
        handoff::begin_pending(&tx, &id, UtcMillis(2))
            .unwrap()
            .state,
        HandoffState::Completed
    );
}
#[test]
fn handoff_begin_refuses_changed_immutable_plan_identity() {
    let mut db = fixture();
    thread(&db);
    joined_agent(&db);
    let tx = db.transaction().unwrap();
    let id = identity();
    handoff::begin_pending(&tx, &id, UtcMillis(0)).unwrap();
    let mut changed = id;
    changed.digest = "b".repeat(64);
    assert!(
        handoff::begin_pending(&tx, &changed, UtcMillis(1)).is_err(),
        "compound key must not rebase onto a changed plan"
    );
}
#[test]
fn handoff_completed_tombstone_absorbs_delayed_import_and_retained_restart_intent() {
    let mut db = fixture();
    thread(&db);
    joined_agent(&db);
    let id = identity();
    {
        let tx = db.transaction().unwrap();
        handoff::begin_pending(&tx, &id, UtcMillis(0)).unwrap();
        tx.commit().unwrap();
    }
    // Importer captured this exact journal before Complete, but applies later.
    let captured = id.clone();
    {
        let tx = db.transaction().unwrap();
        handoff::complete_pending(&tx, &id, UtcMillis(1)).unwrap();
        tx.commit().unwrap();
    }
    db.execute_batch(
        "UPDATE threads SET archived=1 WHERE id='t'; UPDATE seats SET generation=2 WHERE id='s'",
    )
    .unwrap();
    for source in [
        "delayed-pre-complete-capture",
        "retained-intent-after-restart",
    ] {
        let tx = db.transaction().unwrap();
        let result = handoff::import_hint(
            &tx,
            &captured,
            Some(&ThreadId::new("t")),
            source,
            UtcMillis(2),
        )
        .unwrap();
        assert_eq!(
            result.state,
            HandoffState::Completed,
            "terminal identity must win in the insertion transaction"
        );
        assert_eq!(
            tx.query_row(
                "SELECT count(*) FROM channel_handoff_fences WHERE state='live'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            handoff::begin_pending(&tx, &id, UtcMillis(3))
                .unwrap()
                .state,
            HandoffState::Completed
        );
        tx.commit().unwrap();
    }
}
#[test]
fn handoff_legacy_hint_is_sticky_and_cannot_change_bindings_or_receipts() {
    let mut db = fixture();
    thread(&db);
    joined_agent(&db);
    db.execute_batch("UPDATE seats SET generation=2 WHERE id='s'")
        .unwrap(); // frozen actor is no longer live
    let tx = db.transaction().unwrap();
    let result = handoff::import_hint(&tx, &identity(), None, "source", UtcMillis(0)).unwrap();
    assert_eq!(result.state, HandoffState::Live);
    assert!(tx.query_row("SELECT EXISTS(SELECT 1 FROM channel_handoff_fences WHERE origin='legacy_local_journal_hint' AND state='live')",[],|r|r.get::<_,bool>(0)).unwrap());
    assert_eq!(
        tx.query_row("SELECT generation FROM seats WHERE id='s'", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        tx.query_row("SELECT count(*) FROM receipts", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn handoff_new_thread_attachment_matches_exact_actor_scope_and_create_key() {
    let mut db = fixture();
    thread(&db);
    joined_agent(&db);
    let mut id = identity();
    id.thread = None;
    let tx = db.transaction().unwrap();
    handoff::begin_pending(&tx, &id, UtcMillis(0)).unwrap();
    handoff::attach_created(&tx, "i", "seat:other", "create", &ThreadId::new("t")).unwrap();
    assert!(
        handoff::current(&tx, &id)
            .unwrap()
            .unwrap()
            .thread
            .is_none()
    );
    handoff::attach_created(&tx, "i", "seat:s", "create", &ThreadId::new("t")).unwrap();
    assert_eq!(
        handoff::current(&tx, &id).unwrap().unwrap().thread,
        Some(ThreadId::new("t"))
    );
    tx.rollback().unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM channel_handoff_fences", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

pub(super) struct Directory(pub(super) std::path::PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Clock;
impl herdr_threads::protocol::time::Clock for Clock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(100)
    }
    fn monotonic_now(&self) -> herdr_threads::protocol::time::MonoInstant {
        herdr_threads::protocol::time::MonoInstant(100)
    }
}
pub(super) struct StoreFixture {
    pub(super) store: herdr_threads::store::SqliteStore,
    pub(super) db: rusqlite::Connection,
    pub(super) _directory: Directory,
}
pub(super) fn store_fixture() -> StoreFixture {
    store_fixture_with_clock(std::sync::Arc::new(Clock))
}
pub(super) fn store_fixture_with_clock(
    clock: std::sync::Arc<dyn herdr_threads::protocol::time::Clock>,
) -> StoreFixture {
    use herdr_threads::store::{SqliteStore, StoreSettings, connection::StoreContext};
    let directory =
        Directory(std::env::temp_dir().join(format!("handoff-fences-{}", uuid::Uuid::new_v4())));
    std::fs::create_dir(&directory.0).unwrap();
    let path = directory.0.join("store.db");
    let store = SqliteStore::new(
        StoreContext::new(path.clone(), clock),
        "i",
        StoreSettings::default(),
    )
    .unwrap();
    let db = rusqlite::Connection::open(path).unwrap();
    db.pragma_update(None, "foreign_keys", "ON").unwrap();
    db.execute_batch("INSERT INTO host_instances(id,created_at) VALUES('i',0)")
        .unwrap();
    thread(&db);
    joined_agent(&db);
    StoreFixture {
        store,
        db,
        _directory: directory,
    }
}
fn mutate(
    store: &herdr_threads::store::SqliteStore,
    mutation: herdr_threads::protocol::commands::PermitMutation,
) -> Result<
    herdr_threads::protocol::results::CommandResult,
    herdr_threads::protocol::results::ApiError,
> {
    use herdr_threads::ports::StorePort;
    let budget = herdr_threads::protocol::time::CallBudget {
        deadline: herdr_threads::protocol::time::MonoInstant(1000),
        cancellation: Default::default(),
    };
    let permit = store.issue_cooperative_permit(
        herdr_threads::store::cooperative_permit_request(&mutation)?,
        &budget,
    )?;
    store.mutate(mutation, permit, &budget)
}
#[test]
fn handoff_accountable_replay_presents_current_terminal_state_before_live_guards() {
    use herdr_threads::protocol::{commands::PermitMutation, results::CommandResult};
    let fixture = store_fixture();
    let begin = HandoffMutation {
        identity: identity(),
        operation: OperationId::new("begin"),
    };
    assert!(
        matches!(
            mutate(&fixture.store, PermitMutation::BeginHandoff(begin.clone())),
            Ok(CommandResult::Handoff(_))
        ),
        "Begin must cross the accountable store route"
    );
    let complete = HandoffMutation {
        identity: identity(),
        operation: OperationId::new("complete"),
    };
    mutate(
        &fixture.store,
        PermitMutation::CompleteHandoff(complete.clone()),
    )
    .unwrap();
    fixture.db.execute_batch("UPDATE threads SET archived=1 WHERE id='t'; UPDATE seats SET generation=2 WHERE id='s'").unwrap();
    for command in [
        PermitMutation::BeginHandoff(begin),
        PermitMutation::CompleteHandoff(complete),
    ] {
        let CommandResult::Handoff(result) = mutate(&fixture.store, command).unwrap() else {
            panic!("wrong fence result");
        };
        assert_eq!(
            result.state,
            HandoffState::Completed,
            "historical operation result must not hide current completion"
        );
    }
}
#[test]
fn handoff_real_create_attaches_atomically_and_begin_replay_reports_current_thread() {
    use herdr_threads::protocol::{
        commands::{CreateThread, PermitMutation},
        results::CommandResult,
    };
    let fixture = store_fixture();
    let mut id = identity();
    id.thread = None;
    let begin = HandoffMutation {
        identity: id.clone(),
        operation: OperationId::new("begin"),
    };
    mutate(&fixture.store, PermitMutation::BeginHandoff(begin.clone())).unwrap();
    let CommandResult::ThreadCreated(created) = mutate(
        &fixture.store,
        PermitMutation::CreateThread(CreateThread {
            name: None,
            topic: "new".into(),
            goal: "goal".into(),
            operation: id.create_key.clone(),
            claim: id.claim.clone(),
        }),
    )
    .unwrap() else {
        panic!("not created");
    };
    let CommandResult::Handoff(current) =
        mutate(&fixture.store, PermitMutation::BeginHandoff(begin.clone())).unwrap()
    else {
        panic!("not handoff");
    };
    assert_eq!(
        current.thread,
        Some(created.clone()),
        "Begin history must present the thread attached by CREATE"
    );
    fixture
        .db
        .execute(
            "UPDATE memberships SET state='left',left_at=101 WHERE thread_id=?1 AND seat_id='s'",
            [created.as_str()],
        )
        .unwrap();
    assert!(
        mutate(&fixture.store, PermitMutation::BeginHandoff(begin)).is_err(),
        "live historical replay must still check membership"
    );
}

#[test]
fn handoff_actual22_upgrade_imports_already_committed_create_then_replays_without_unattached_fence()
{
    use herdr_threads::{
        protocol::{
            commands::{CreateThread, PermitMutation},
            results::CommandResult,
        },
        store::{
            SqliteStore, StoreSettings, connection::StoreContext, control::cooperative_payload_hash,
        },
    };
    let directory =
        Directory(std::env::temp_dir().join(format!("legacy-create-{}", uuid::Uuid::new_v4())));
    std::fs::create_dir(&directory.0).unwrap();
    let path = directory.0.join("store.db");
    let mut db = rusqlite::Connection::open(&path).unwrap();
    let mut migrations = std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/migrations"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect::<Vec<_>>();
    migrations.sort();
    for path in migrations {
        if path.file_name().unwrap().to_str().unwrap()[..4]
            .parse::<u32>()
            .unwrap()
            <= 22
        {
            db.execute_batch(&std::fs::read_to_string(path).unwrap())
                .unwrap();
        }
    }
    db.pragma_update(None, "user_version", 22).unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at) VALUES('i',0)",
        [],
    )
    .unwrap();
    thread(&db);
    joined_agent(&db);
    let mut id = identity();
    id.thread = None;
    let create = CreateThread {
        name: None,
        topic: "topic".into(),
        goal: "goal".into(),
        operation: id.create_key.clone(),
        claim: id.claim.clone(),
    };
    let historical =
        serde_json::to_string(&CommandResult::ThreadCreated(ThreadId::new("t"))).unwrap();
    db.execute("INSERT INTO operations(actor_scope,operation_key,digest,result_json,decided_at) VALUES('seat:s',?1,?2,?3,0)",rusqlite::params![id.create_key.as_str(),cooperative_payload_hash("create_thread",&create).unwrap().as_slice(),historical]).unwrap();
    let store = SqliteStore::new(
        StoreContext::new(path, std::sync::Arc::new(Clock)),
        "i",
        StoreSettings::default(),
    )
    .unwrap();
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
            .unwrap(),
        26
    );
    let tx = db.transaction().unwrap();
    let imported = handoff::import_hint(&tx, &id, None, "legacy22", UtcMillis(100)).unwrap();
    assert_eq!(imported.thread, Some(ThreadId::new("t")));
    tx.commit().unwrap();
    let CommandResult::Handoff(result) = mutate(
        &store,
        PermitMutation::BeginHandoff(HandoffMutation {
            identity: id.clone(),
            operation: OperationId::new("begin"),
        }),
    )
    .unwrap() else {
        panic!("handoff")
    };
    assert_eq!(result.thread, Some(ThreadId::new("t")));
    assert_eq!(
        mutate(&store, PermitMutation::CreateThread(create)).unwrap(),
        CommandResult::ThreadCreated(ThreadId::new("t"))
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM threads", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM channel_handoff_fences WHERE state='live' AND thread_id IS NULL",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT result_json FROM operations WHERE operation_key=?1",
            [id.create_key.as_str()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        historical
    );
}
