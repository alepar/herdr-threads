use super::channel_archival::{joined_agent, thread};
use herdr_threads::{
    cli::journal::IntentScope,
    protocol::{handoff::*, ids::*, results::ErrorCode, time::UtcMillis},
    store::{schema, topology_handoff},
};
use rusqlite::Connection;

pub(super) fn namespace() -> HandoffNamespace {
    HandoffNamespace {
        instance: "i".into(),
        state_dir: "/state".into(),
        host_endpoint: "/host.sock".into(),
    }
}

pub(super) fn identity() -> BootstrapIdentity {
    let claim = super::handoff_fences::identity().claim;
    let mut identity = BootstrapIdentity {
        compound: OperationId::new("bootstrap"),
        scope: IntentScope::Cooperative {
            instance: "i".into(),
            seat: claim.seat.clone(),
        },
        claim,
        digest: "a".repeat(64),
        payload: BootstrapPayload {
            handoff: HandoffPayload {
                namespace: namespace(),
                keys: HandoffKeys {
                    compound: OperationId::new("bootstrap"),
                    begin: OperationId::new("begin-bootstrap"),
                    create: OperationId::new("create"),
                    invite: OperationId::new("invite"),
                    send: OperationId::new("send"),
                    complete: OperationId::new("complete"),
                },
                channel: HandoffChannel::Existing {
                    thread: ThreadId::new("t"),
                },
                body: "work".into(),
            },
            workspace: HostTargetId::new("w"),
            cwd: "/cwd".into(),
            label: "peer".into(),
            focus: false,
            env: Default::default(),
            launch: BootstrapLaunch {
                harness: herdr_threads::protocol::authority::Harness::Codex,
                binary: None,
                name: None,
                argv: vec!["--model".into(), "fixed".into()],
            },
            handoff_key: OperationId::new("handoff"),
            resolve_key: OperationId::new("resolve"),
            attach_key: OperationId::new("attach"),
            linked_complete_key: OperationId::new("linked-complete"),
        },
    };
    identity.digest = identity.semantic_digest().unwrap();
    identity.validate().unwrap();
    identity
}

fn setup(db: &Connection) {
    schema::initialize(db, || UtcMillis(0)).unwrap();
    assert_eq!(
        db.pragma_query_value::<i64, _>(None, "user_version", |r| r.get(0))
            .unwrap(),
        28
    );
    db.execute_batch("INSERT INTO host_instances(id,created_at) VALUES('i',0)")
        .unwrap();
    thread(db);
    joined_agent(db);
}

/// The database image `setup` produces. Building the schema replays every
/// migration and its verification, far more than most cases cost, so each
/// test process builds it once; every fixture is a private copy of the image,
/// as fresh and independent as a newly built one.
fn image() -> &'static [u8] {
    use rusqlite::ffi;
    static IMAGE: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    IMAGE.get_or_init(|| {
        let db = Connection::open_in_memory().unwrap();
        setup(&db);
        let mut size: ffi::sqlite3_int64 = 0;
        // SAFETY: serialize the live main schema of our own open connection;
        // the returned sqlite3_malloc buffer is copied and freed here.
        unsafe {
            let data = ffi::sqlite3_serialize(db.handle(), c"main".as_ptr(), &mut size, 0);
            assert!(!data.is_null(), "serialize the fixture schema");
            let image = std::slice::from_raw_parts(data, size as usize).to_vec();
            ffi::sqlite3_free(data.cast());
            image
        }
    })
}

pub(super) fn fixture() -> Connection {
    use rusqlite::ffi;
    let image = image();
    let db = Connection::open_in_memory().unwrap();
    let size = image.len() as ffi::sqlite3_int64;
    // SAFETY: SQLite takes ownership of the sqlite3_malloc64 copy (FREEONCLOSE)
    // and may grow it (RESIZEABLE); the connection handle is ours and open.
    unsafe {
        let buffer = ffi::sqlite3_malloc64(image.len() as u64).cast::<u8>();
        assert!(!buffer.is_null(), "allocate the fixture image");
        std::ptr::copy_nonoverlapping(image.as_ptr(), buffer, image.len());
        let rc = ffi::sqlite3_deserialize(
            db.handle(),
            c"main".as_ptr(),
            buffer,
            size,
            size,
            (ffi::SQLITE_DESERIALIZE_FREEONCLOSE | ffi::SQLITE_DESERIALIZE_RESIZEABLE) as _,
        );
        assert_eq!(rc, ffi::SQLITE_OK, "deserialize the fixture schema");
    }
    db
}

fn file_fixture() -> (
    super::handoff_fences::Directory,
    std::path::PathBuf,
    Connection,
) {
    let directory = super::handoff_fences::Directory(
        std::env::temp_dir().join(format!("ht-topology-{}", uuid::Uuid::new_v4())),
    );
    std::fs::create_dir(&directory.0).unwrap();
    let path = directory.0.join("store.db");
    // A serialized image is a complete database file.
    std::fs::write(&path, image()).unwrap();
    let db = Connection::open(&path).unwrap();
    (directory, path, db)
}

fn counts(db: &Connection) -> (i64, i64, i64, i64) {
    db.query_row("SELECT (SELECT count(*) FROM bootstrap_handoffs),(SELECT count(*) FROM bootstrap_attempts),(SELECT count(*) FROM bootstrap_child_keys),(SELECT count(*) FROM messages)", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap()
}

#[test]
fn begin_installs_attempt_one_and_thread_protection_without_delivery() {
    let mut db = fixture();
    let tx = db.transaction().unwrap();
    let result =
        topology_handoff::begin_pending(&tx, &namespace(), &identity(), UtcMillis(5)).unwrap();
    assert_eq!(result.state, BootstrapState::Prepared);
    assert_eq!(result.attempt, BootstrapAttempt::first());
    assert_eq!(result.attempt_state, BootstrapAttemptState::Prepared);
    assert_eq!(counts(&tx), (1, 1, 14, 0));
    assert_eq!(
        tx.query_row(
            "SELECT thread_id FROM bootstrap_handoffs WHERE state='prepared'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "t"
    );
    assert_eq!(
        tx.query_row("SELECT count(*) FROM occupant_bindings", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        topology_handoff::current(&tx, &namespace(), &identity()).unwrap(),
        Some(result.clone())
    );
    assert_eq!(
        topology_handoff::begin_pending(&tx, &namespace(), &identity(), UtcMillis(9)).unwrap(),
        result
    );
    assert_eq!(counts(&tx), (1, 1, 14, 0));
    tx.commit().unwrap();
}

#[test]
fn registered_product_ddl_defines_normalized_records() {
    let db = fixture();
    for name in [
        "bootstrap_handoffs",
        "bootstrap_attempts",
        "bootstrap_child_keys",
        "bootstrap_recovery_decisions",
        "bootstrap_attachments",
        "bootstrap_reports",
    ] {
        assert!(
            db.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?1)",
                [name],
                |r| r.get::<_, bool>(0)
            )
            .unwrap(),
            "real topology table missing: {name}"
        );
    }
    assert!(
        db.pragma_query_value::<bool, _>(None, "foreign_keys", |r| r.get(0))
            .unwrap()
    );
}

#[test]
fn canonical_namespace_and_full_identity_conflicts_leave_rows_unchanged() {
    let mut db = fixture();
    let tx = db.transaction().unwrap();
    let id = identity();
    topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(0)).unwrap();
    let mut foreign = namespace();
    foreign.state_dir = "/other-state".into();
    assert_eq!(
        topology_handoff::current(&tx, &foreign, &id)
            .unwrap_err()
            .code,
        ErrorCode::InstanceMismatch
    );
    assert_eq!(
        topology_handoff::begin_pending(&tx, &foreign, &id, UtcMillis(1))
            .unwrap_err()
            .code,
        ErrorCode::InstanceMismatch
    );
    let mut changed = id;
    changed.payload.launch.argv.push("--other".into());
    changed.digest = changed.semantic_digest().unwrap();
    assert_eq!(
        topology_handoff::begin_pending(&tx, &namespace(), &changed, UtcMillis(1))
            .unwrap_err()
            .code,
        ErrorCode::OperationPayloadMismatch
    );
    assert_eq!(counts(&tx), (1, 1, 14, 0));
    tx.commit().unwrap();
}

#[test]
fn live_begin_and_replay_recheck_a2_archive_and_membership() {
    for (mutation, expected) in [
        ("UPDATE seats SET generation=2", ErrorCode::CallerUnverified),
        (
            "UPDATE occupant_bindings SET native_session='changed'",
            ErrorCode::CallerUnverified,
        ),
        (
            "UPDATE host_instances SET host_epoch=2",
            ErrorCode::CallerUnverified,
        ),
        ("UPDATE threads SET archived=1", ErrorCode::Archived),
        (
            "UPDATE memberships SET state='left'",
            ErrorCode::MembershipRequired,
        ),
    ] {
        for replay in [false, true] {
            let mut db = fixture();
            let tx = db.transaction().unwrap();
            if replay {
                topology_handoff::begin_pending(&tx, &namespace(), &identity(), UtcMillis(0))
                    .unwrap();
            }
            tx.execute_batch(mutation).unwrap();
            assert_eq!(
                topology_handoff::begin_pending(&tx, &namespace(), &identity(), UtcMillis(1))
                    .unwrap_err()
                    .code,
                expected
            );
            assert_eq!(
                counts(&tx),
                if replay { (1, 1, 14, 0) } else { (0, 0, 0, 0) }
            );
            tx.commit().unwrap();
        }
    }
}

#[test]
fn new_channel_retains_frozen_create_key_without_creating_channel() {
    let mut db = fixture();
    let mut id = identity();
    id.payload.handoff.channel = HandoffChannel::New {
        name: None,
        topic: "topic".into(),
        goal: "goal".into(),
    };
    id.digest = id.semantic_digest().unwrap();
    let tx = db.transaction().unwrap();
    topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(0)).unwrap();
    let row: (Option<String>, String) = tx
        .query_row(
            "SELECT thread_id,create_key FROM bootstrap_handoffs",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(row, (None, "create".into()));
    assert_eq!(
        tx.query_row("SELECT count(*) FROM threads", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(counts(&tx), (1, 1, 14, 0));
}

#[test]
fn retained_identity_and_attempt_survive_database_reopen() {
    let directory = super::handoff_fences::Directory(
        std::env::temp_dir().join(format!("ht-topology-{}", uuid::Uuid::new_v4())),
    );
    std::fs::create_dir(&directory.0).unwrap();
    let path = directory.0.join("store.db");
    let expected = {
        let mut db = Connection::open(&path).unwrap();
        setup(&db);
        let tx = db.transaction().unwrap();
        let result =
            topology_handoff::begin_pending(&tx, &namespace(), &identity(), UtcMillis(10)).unwrap();
        tx.commit().unwrap();
        result
    };
    let db = Connection::open(path).unwrap();
    db.pragma_update(None, "foreign_keys", true).unwrap();
    assert_eq!(
        topology_handoff::current(&db, &namespace(), &identity()).unwrap(),
        Some(expected)
    );
    assert_eq!(counts(&db), (1, 1, 14, 0));
}

#[test]
fn registry_refuses_cross_role_collisions_and_invalid_attempts_atomically() {
    let mut db = fixture();
    let tx = db.transaction().unwrap();
    let id = identity();
    topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(0)).unwrap();
    let mut second = id.clone();
    second.compound = OperationId::new("bootstrap2");
    second.payload.handoff.keys.compound = second.compound.clone();
    second.payload.handoff.keys.begin = OperationId::new("begin2");
    second.payload.handoff.keys.create = OperationId::new("create2");
    second.payload.handoff.keys.invite = OperationId::new("invite2");
    second.payload.handoff.keys.send = OperationId::new("send2");
    second.payload.handoff.keys.complete = OperationId::new("complete2");
    second.payload.handoff_key = OperationId::new("handoff2");
    second.payload.resolve_key = OperationId::new("resolve2");
    second.payload.attach_key = OperationId::new("attach2");
    second.payload.linked_complete_key = OperationId::new("linked-complete2");
    // A second parent's compound colliding with the first parent's child is refused.
    second.compound = id.payload.handoff.keys.send.clone();
    second.payload.handoff.keys.compound = second.compound.clone();
    second.digest = second.semantic_digest().unwrap();
    assert_eq!(
        topology_handoff::begin_pending(&tx, &namespace(), &second, UtcMillis(0))
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(counts(&tx), (1, 1, 14, 0));
    assert!(tx.execute("INSERT INTO bootstrap_attempts(parent_id,attempt,state,reserve_key,record_key,check_key,not_submitted_key) VALUES(1,0,'prepared','r0','d0','c0','n0')",[]).is_err());
    assert!(tx.execute("INSERT INTO bootstrap_attempts(parent_id,attempt,state,reserve_key,record_key,check_key,not_submitted_key) VALUES(1,1,'prepared','r1','d1','c1','n1')",[]).is_err());
    assert!(
        tx.execute("UPDATE bootstrap_handoffs SET current_attempt=99", [])
            .is_ok()
    );
    assert!(
        tx.commit().is_err(),
        "active attempt FK must reject missing records at commit"
    );
    assert_eq!(counts(&db), (0, 0, 0, 0));
}

#[test]
fn lookup_fails_closed_on_tampered_identity_or_cross_record_state() {
    for tamper in [
        "DROP TRIGGER bootstrap_identity; UPDATE bootstrap_handoffs SET identity_json=CAST('{}' AS BLOB)",
        "PRAGMA ignore_check_constraints=ON; UPDATE bootstrap_attempts SET state='invented'",
        "PRAGMA foreign_keys=OFF; UPDATE bootstrap_handoffs SET current_attempt=99",
    ] {
        let mut db = fixture();
        {
            let tx = db.transaction().unwrap();
            topology_handoff::begin_pending(&tx, &namespace(), &identity(), UtcMillis(0)).unwrap();
            tx.commit().unwrap();
        }
        db.execute_batch(tamper).unwrap();
        assert_eq!(
            topology_handoff::current(&db, &namespace(), &identity())
                .unwrap_err()
                .code,
            ErrorCode::StoreCorrupt
        );
    }
}

#[test]
fn replay_and_thread_protection_queries_use_keys_instead_of_table_scans() {
    let mut db = fixture();
    let tx = db.transaction().unwrap();
    topology_handoff::begin_pending(&tx, &namespace(), &identity(), UtcMillis(0)).unwrap();
    for (sql, args) in [
        (
            "EXPLAIN QUERY PLAN SELECT id FROM bootstrap_handoffs WHERE instance_id=?1 AND state_dir=?2 AND host_endpoint=?3 AND actor_scope=?4 AND compound=?5",
            vec!["i", "/state", "/host.sock", "seat:s", "bootstrap"],
        ),
        (
            "EXPLAIN QUERY PLAN SELECT compound FROM bootstrap_handoffs WHERE thread_id=?1 AND state NOT IN ('completed','cancelled')",
            vec!["t"],
        ),
    ] {
        let mut stmt = tx.prepare(sql).unwrap();
        let plans = stmt
            .query_map(rusqlite::params_from_iter(args), |r| r.get::<_, String>(3))
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>();
        assert!(
            plans
                .iter()
                .any(|p| p.contains("SEARCH") && p.contains("INDEX")),
            "indexed lookup expected: {plans:?}"
        );
        assert!(
            !plans.iter().any(|p| p.contains("SCAN bootstrap_handoffs")),
            "unbounded lookup: {plans:?}"
        );
    }
    let bytes = tx
        .query_row("SELECT identity_json FROM bootstrap_handoffs", [], |r| {
            r.get::<_, Vec<u8>>(0)
        })
        .unwrap();
    assert_eq!(
        bytes,
        topology_handoff::encode_identity(&namespace(), &identity()).unwrap()
    );
}

#[test]
fn late_attempt_failure_rolls_back_helper_rows_even_when_caller_commits() {
    let mut db = fixture();
    db.execute_batch("CREATE TRIGGER reject_first_attempt BEFORE INSERT ON bootstrap_attempts BEGIN SELECT RAISE(ABORT,'injected first attempt failure'); END").unwrap();
    let tx = db.transaction().unwrap();
    assert_eq!(
        topology_handoff::begin_pending(&tx, &namespace(), &identity(), UtcMillis(0))
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    assert_eq!(counts(&tx), (0, 0, 0, 0));
    tx.commit().unwrap();
    assert_eq!(counts(&db), (0, 0, 0, 0));
}

#[test]
fn cancelled_replay_preserves_original_identity_without_current_live_authority() {
    let (_directory, path, mut db) = file_fixture();
    let id = identity();
    let tx = db.transaction().unwrap();
    topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(0)).unwrap();
    let disposition = BootstrapRecoveryDisposition::Cancelled {
        reason: "operator abandoned work".into(),
        quiescence: BootstrapQuiescenceAssertion::InspectedQuiescence,
        child_guard: BootstrapCancellationGuard {
            attached_child: None,
        },
    };
    let mut decision = RecoverBootstrap {
        inspection: None,
        identity: id.clone(),
        expected_attempt: BootstrapAttempt::first(),
        operation: OperationId::new("placeholder"),
        disposition: disposition.clone(),
    };
    decision.operation = decision.decision_operation().unwrap();
    let retained = BootstrapRecoveryResult {
        inspection: None,
        identity: id.clone(),
        attempt: BootstrapAttempt::first(),
        operation: decision.operation.clone(),
        disposition,
        operator_uid: 501,
        operator_provenance: "operator:local-user:501".into(),
        creation: None,
        state: BootstrapState::Cancelled,
    };
    tx.execute("INSERT INTO bootstrap_recovery_decisions(parent_id,attempt,operation,result_json,decision_kind) VALUES(1,1,?1,?2,'cancellation')",rusqlite::params![decision.operation.as_str(),serde_json::to_vec(&retained).unwrap()]).unwrap();
    tx.execute("UPDATE bootstrap_handoffs SET state='cancelled',terminal_at=2,latest_recovery_operation=?1 WHERE id=1",[decision.operation.as_str()]).unwrap();
    tx.execute_batch("UPDATE seats SET generation=2; UPDATE threads SET archived=1")
        .unwrap();
    let result = topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(3)).unwrap();
    assert_eq!(result.state, BootstrapState::Cancelled);
    assert_eq!(result.recovery, Some(Box::new(retained)));
    assert!(
        tx.execute(
            "UPDATE bootstrap_handoffs SET state='prepared',terminal_at=NULL",
            []
        )
        .is_err()
    );
    let mut changed = id.clone();
    changed.claim.binding_generation = 2;
    changed.digest = changed.semantic_digest().unwrap();
    assert_eq!(
        topology_handoff::begin_pending(&tx, &namespace(), &changed, UtcMillis(4))
            .unwrap_err()
            .code,
        ErrorCode::OperationPayloadMismatch
    );
    assert_eq!(counts(&tx), (1, 1, 15, 0));
    tx.commit().unwrap();
    drop(db);
    let db = Connection::open(path).unwrap();
    schema::initialize(&db, || UtcMillis(4)).unwrap();
    assert_eq!(
        topology_handoff::current(&db, &namespace(), &id).unwrap(),
        Some(result)
    );
    db.execute_batch(
        "DROP TRIGGER bootstrap_identity; UPDATE bootstrap_handoffs SET identity_json=x'7b7d'",
    )
    .unwrap();
    assert_eq!(
        topology_handoff::current(&db, &namespace(), &id)
            .unwrap_err()
            .code,
        ErrorCode::StoreCorrupt
    );
}

pub(super) fn created() -> herdr_threads::ports::CreatedTab {
    use herdr_threads::host::continuity::{LocalEndpointWitness, SocketIdentity};
    herdr_threads::ports::CreatedTab {
        correlation: HostCallId::new("correlation"),
        workspace: HostTargetId::new("w"),
        tab: HostTargetId::new("w:t2"),
        root_pane: HostTargetId::new("w:p2"),
        terminal: TerminalId::new("terminal"),
        host_incarnation: HostBootId::new("herdr-server:pid=42:start=1.000002:uid=501"),
        witness: LocalEndpointWitness {
            schema: 1,
            platform: "macos-proc-bsdinfo-v1".into(),
            endpoint: "/host.sock".into(),
            peer_uid: 501,
            peer_pid: 42,
            start_seconds: 1,
            start_microseconds: 2,
            socket: SocketIdentity {
                device: 1,
                inode: 2,
                birth_seconds: 1,
                birth_nanoseconds: 0,
                change_seconds: 1,
                change_nanoseconds: 0,
            },
        },
    }
}

fn record_created(tx: &rusqlite::Transaction<'_>) {
    let created = created();
    created.validate().unwrap();
    tx.execute("UPDATE bootstrap_attempts SET state='created',creation_json=?1 WHERE parent_id=1 AND attempt=1",[serde_json::to_vec(&created).unwrap()]).unwrap();
    tx.execute(
        "UPDATE bootstrap_handoffs SET state='created' WHERE id=1",
        [],
    )
    .unwrap();
}

#[test]
fn live_new_channel_replay_checks_its_recorded_thread_after_creation() {
    for (mutation, expected) in [
        ("UPDATE threads SET archived=1", ErrorCode::Archived),
        (
            "UPDATE memberships SET state='left'",
            ErrorCode::MembershipRequired,
        ),
    ] {
        let mut db = fixture();
        let tx = db.transaction().unwrap();
        let mut id = identity();
        id.payload.handoff.channel = HandoffChannel::New {
            name: None,
            topic: "topic".into(),
            goal: "goal".into(),
        };
        id.digest = id.semantic_digest().unwrap();
        topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(0)).unwrap();
        record_created(&tx);
        tx.execute("UPDATE bootstrap_handoffs SET thread_id='t' WHERE id=1", [])
            .unwrap();
        tx.execute_batch(mutation).unwrap();
        assert_eq!(
            topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(1))
                .unwrap_err()
                .code,
            expected
        );
        assert_eq!(counts(&tx), (1, 1, 14, 0));
        tx.commit().unwrap();
    }
}

#[test]
fn impossible_attempt_number_is_saved_corruption_not_transient_busy() {
    let mut db = fixture();
    {
        let tx = db.transaction().unwrap();
        topology_handoff::begin_pending(&tx, &namespace(), &identity(), UtcMillis(0)).unwrap();
        tx.commit().unwrap();
    }
    db.execute_batch("PRAGMA foreign_keys=OFF; PRAGMA ignore_check_constraints=ON; UPDATE bootstrap_handoffs SET current_attempt=-1").unwrap();
    assert_eq!(
        topology_handoff::current(&db, &namespace(), &identity())
            .unwrap_err()
            .code,
        ErrorCode::StoreCorrupt
    );
}

#[test]
fn bounded_slots_refuse_oversized_or_missing_terminal_reports_without_truncation() {
    let mut db = fixture();
    let tx = db.transaction().unwrap();
    topology_handoff::begin_pending(&tx, &namespace(), &identity(), UtcMillis(0)).unwrap();
    assert!(
        tx.execute(
            "UPDATE bootstrap_attempts SET state='created',creation_json=?1 WHERE parent_id=1",
            [vec![b' '; topology_handoff::MAX_CREATION_BYTES + 1]]
        )
        .is_err()
    );
    assert!(
        tx.execute(
            "INSERT INTO bootstrap_attachments(parent_id,attempt,attachment_json) VALUES(1,1,?1)",
            [vec![b' '; topology_handoff::MAX_ATTACHMENT_BYTES + 1]]
        )
        .is_err()
    );
    assert!(tx.execute("INSERT INTO bootstrap_recovery_decisions(parent_id,attempt,operation,result_json) VALUES(1,1,'decision',?1)",[vec![b' ';topology_handoff::MAX_RECOVERY_BYTES+1]]).is_err());
    tx.execute(
        "UPDATE bootstrap_handoffs SET state='completed',terminal_at=1 WHERE id=1",
        [],
    )
    .unwrap();
    assert_eq!(
        topology_handoff::current(&tx, &namespace(), &identity())
            .unwrap_err()
            .code,
        ErrorCode::StoreCorrupt,
        "completed rows without real retained report refuse"
    );
}

#[test]
fn full_completed_report_with_large_json_escaping_replays_and_rejects_tamper() {
    use sha2::{Digest, Sha256};
    let (_directory, path, mut db) = file_fixture();
    let tx = db.transaction().unwrap();
    let mut id = identity();
    id.payload.launch.argv = vec!["\"".repeat(4096); 14];
    id.digest = id.semantic_digest().unwrap();
    id.validate().unwrap();
    topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(0)).unwrap();
    record_created(&tx);
    let attachment = BootstrapAttachment {
        attempt: BootstrapAttempt::first(),
        created: created(),
        resolve_operation: id.payload.resolve_key.clone(),
        resolved_seat: SeatId::new("peer"),
        handoff: HandoffIdentity {
            compound: id.payload.handoff_key.clone(),
            digest: "b".repeat(64),
            claim: id.claim.clone(),
            thread: Some(ThreadId::new("t")),
            recipient: SeatId::new("peer"),
            create_key: id.payload.handoff.keys.create.clone(),
            invite_key: id.payload.handoff.keys.invite.clone(),
            send_key: id.payload.handoff.keys.send.clone(),
        },
    };
    attachment.validate(&id).unwrap();
    let mut report = serde_json::json!({"outcome":"started","pane":"w:p2","seat":"peer","harness":"codex","argv":id.payload.launch.argv.clone(),"details":""});
    let remaining = 1024 * 1024 - serde_json::to_vec(&report).unwrap().len();
    report["details"] = serde_json::Value::String(format!(
        "{}{}",
        "\"".repeat(remaining / 2),
        if remaining.is_multiple_of(2) { "" } else { "x" }
    ));
    assert_eq!(serde_json::to_vec(&report).unwrap().len(), 1024 * 1024);
    let retained = LinkedBootstrapReport {
        thread: ThreadId::new("t"),
        recipient: SeatId::new("peer"),
        pane: HostTargetId::new("w:p2"),
        kind: "launch".into(),
        launch: id.payload.launch.clone(),
        report_digest: format!("{:x}", Sha256::digest(serde_json::to_vec(&report).unwrap())),
        report,
        terminal: created().terminal,
        host_incarnation: created().host_incarnation,
    };
    retained.validate().unwrap();
    let completed = CompletedBootstrapResult {
        identity: id.clone(),
        attachment: attachment.clone(),
        retained,
        legacy_result: HandoffResult {
            compound: id.payload.handoff_key.clone(),
            thread: Some(ThreadId::new("t")),
            state: HandoffState::Completed,
        },
    };
    let bytes = serde_json::to_vec(&completed).unwrap();
    assert!(
        bytes.len() > 1024 * 1024 && bytes.len() < topology_handoff::MAX_COMPLETED_BYTES,
        "full typed report, identity, attachment, launch argv and legacy result fit retained ceiling"
    );
    tx.execute(
        "INSERT INTO bootstrap_attachments(parent_id,attempt,attachment_json) VALUES(1,1,?1)",
        [serde_json::to_vec(&attachment).unwrap()],
    )
    .unwrap();
    assert!(
        tx.execute(
            "INSERT INTO bootstrap_reports(parent_id,completed_json) VALUES(1,?1)",
            [vec![b' '; topology_handoff::MAX_COMPLETED_BYTES + 1]]
        )
        .is_err()
    );
    tx.execute(
        "INSERT INTO bootstrap_reports(parent_id,completed_json) VALUES(1,?1)",
        [bytes],
    )
    .unwrap();
    tx.execute(
        "UPDATE bootstrap_handoffs SET state='completed',terminal_at=2 WHERE id=1",
        [],
    )
    .unwrap();
    tx.execute_batch("UPDATE seats SET generation=2; UPDATE threads SET archived=1")
        .unwrap();
    let replay = topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(3)).unwrap();
    assert_eq!(replay.completed, Some(Box::new(completed)));
    tx.commit().unwrap();
    drop(db);
    let db = Connection::open(path).unwrap();
    schema::initialize(&db, || UtcMillis(4)).unwrap();
    assert_eq!(
        topology_handoff::current(&db, &namespace(), &id).unwrap(),
        Some(replay)
    );
    // A terminal result must still reconstruct its exact normalized current attempt.
    db.execute_batch("PRAGMA foreign_keys=OFF; DROP TRIGGER bootstrap_terminal; UPDATE bootstrap_handoffs SET current_attempt=2").unwrap();
    assert_eq!(
        topology_handoff::current(&db, &namespace(), &id)
            .unwrap_err()
            .code,
        ErrorCode::StoreCorrupt
    );
    db.execute_batch("UPDATE bootstrap_handoffs SET current_attempt=1; PRAGMA foreign_keys=ON")
        .unwrap();
    db.execute_batch("DROP TRIGGER bootstrap_report_immutable; PRAGMA ignore_check_constraints=ON")
        .unwrap();
    db.execute(
        "UPDATE bootstrap_reports SET completed_json=?1 WHERE parent_id=1",
        [vec![b' '; topology_handoff::MAX_COMPLETED_BYTES + 1]],
    )
    .unwrap();
    assert_eq!(
        topology_handoff::current(&db, &namespace(), &id)
            .unwrap_err()
            .code,
        ErrorCode::StoreCorrupt
    );
}

#[test]
fn invalid_terminal_metadata_and_revision_are_retained_corruption() {
    for tamper in [
        "PRAGMA ignore_check_constraints=ON; UPDATE bootstrap_handoffs SET terminal_at=9",
        "PRAGMA ignore_check_constraints=ON; UPDATE bootstrap_handoffs SET administrative_revision=-1",
        "DROP TRIGGER bootstrap_keys_retained; DELETE FROM bootstrap_child_keys WHERE role='compound'",
    ] {
        let mut db = fixture();
        {
            let tx = db.transaction().unwrap();
            topology_handoff::begin_pending(&tx, &namespace(), &identity(), UtcMillis(0)).unwrap();
            tx.commit().unwrap();
        }
        db.execute_batch(tamper).unwrap();
        assert_eq!(
            topology_handoff::current(&db, &namespace(), &identity())
                .unwrap_err()
                .code,
            ErrorCode::StoreCorrupt
        );
    }
}

#[test]
fn current_key_lookup_stays_bounded_after_many_retained_attempts() {
    let mut db = fixture();
    let id = identity();
    {
        let tx = db.transaction().unwrap();
        topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(0)).unwrap();
        tx.execute(
            "UPDATE bootstrap_attempts SET state='not_submitted' WHERE parent_id=1 AND attempt=1",
            [],
        )
        .unwrap();
        for number in 2..=1001 {
            let attempt = BootstrapAttempt::new(number).unwrap();
            tx.execute("INSERT INTO bootstrap_attempts(parent_id,attempt,state,reserve_key,record_key,check_key,not_submitted_key) VALUES(1,?1,?2,?3,?4,?5,?6)",rusqlite::params![number,if number==1001 {"prepared"}else{"not_submitted"},attempt.operation(&id.compound,"reserve").unwrap().as_str(),attempt.operation(&id.compound,"record").unwrap().as_str(),attempt.operation(&id.compound,"check").unwrap().as_str(),attempt.operation(&id.compound,"not_submitted").unwrap().as_str()]).unwrap();
        }
        tx.execute(
            "UPDATE bootstrap_handoffs SET current_attempt=1001 WHERE id=1",
            [],
        )
        .unwrap();
        tx.commit().unwrap();
    }
    // A connection-local VM budget catches a registry lookup that scans the
    // parent's entire history. No wall-clock/other-run/process state is used.
    unsafe extern "C" fn budget(context: *mut std::ffi::c_void) -> i32 {
        // The counter remains live until the connection-local callback is cleared.
        let count = unsafe { &mut *context.cast::<usize>() };
        *count += 1;
        i32::from(*count > 20)
    }
    let mut progress_calls = 0usize;
    unsafe {
        rusqlite::ffi::sqlite3_progress_handler(
            db.handle(),
            100,
            Some(budget),
            std::ptr::from_mut(&mut progress_calls).cast(),
        );
    }
    let result = topology_handoff::current(&db, &namespace(), &id);
    unsafe {
        rusqlite::ffi::sqlite3_progress_handler(db.handle(), 0, None, std::ptr::null_mut());
    }
    assert_eq!(
        result.unwrap().unwrap().attempt,
        BootstrapAttempt::new(1001).unwrap()
    );
}

#[path = "topology_handoff_schema.rs"]
mod schema_tests;

fn saved_recovery(
    tx: &rusqlite::Transaction<'_>,
    id: &BootstrapIdentity,
    disposition: BootstrapRecoveryDisposition,
    state: BootstrapState,
    creation: Option<herdr_threads::ports::CreatedTab>,
) -> BootstrapRecoveryResult {
    let mut request = RecoverBootstrap {
        inspection: None,
        identity: id.clone(),
        expected_attempt: BootstrapAttempt::first(),
        operation: OperationId::new("placeholder"),
        disposition,
    };
    request.operation = request.decision_operation().unwrap();
    let result = BootstrapRecoveryResult {
        inspection: None,
        identity: id.clone(),
        attempt: request.expected_attempt,
        operation: request.operation,
        disposition: request.disposition,
        operator_uid: 501,
        operator_provenance: "operator:local-user:501".into(),
        creation,
        state,
    };
    tx.execute("INSERT INTO bootstrap_recovery_decisions(parent_id,attempt,operation,result_json,decision_kind) VALUES(1,1,?1,?2,?3)",rusqlite::params![result.operation.as_str(),serde_json::to_vec(&result).unwrap(),if matches!(result.disposition,BootstrapRecoveryDisposition::Cancelled{..}) {"cancellation"}else{"recovery"}]).unwrap();
    tx.execute(
        "UPDATE bootstrap_handoffs SET latest_recovery_operation=?1 WHERE id=1",
        [result.operation.as_str()],
    )
    .unwrap();
    result
}
fn cancellation() -> BootstrapRecoveryDisposition {
    BootstrapRecoveryDisposition::Cancelled {
        reason: "operator abandoned work".into(),
        quiescence: BootstrapQuiescenceAssertion::InspectedQuiescence,
        child_guard: BootstrapCancellationGuard {
            attached_child: None,
        },
    }
}
fn next_prepared(tx: &rusqlite::Transaction<'_>, id: &BootstrapIdentity) {
    let attempt = BootstrapAttempt::new(2).unwrap();
    tx.execute("INSERT INTO bootstrap_attempts(parent_id,attempt,state,reserve_key,record_key,check_key,not_submitted_key) VALUES(1,2,'prepared',?1,?2,?3,?4)",rusqlite::params![attempt.operation(&id.compound,"reserve").unwrap().as_str(),attempt.operation(&id.compound,"record").unwrap().as_str(),attempt.operation(&id.compound,"check").unwrap().as_str(),attempt.operation(&id.compound,"not_submitted").unwrap().as_str()]).unwrap();
    tx.execute(
        "UPDATE bootstrap_handoffs SET current_attempt=2,state='prepared' WHERE id=1",
        [],
    )
    .unwrap();
}
#[test]
fn recovery_cancelled_reopen_refuses_invalid_foreign_or_unrecorded_creation() {
    let mut malformed = created();
    malformed.witness.schema = 0;
    let mut foreign_workspace = created();
    foreign_workspace.workspace = HostTargetId::new("foreign");
    foreign_workspace.tab = HostTargetId::new("foreign:t2");
    foreign_workspace.root_pane = HostTargetId::new("foreign:p2");
    foreign_workspace.validate().unwrap();
    let mut foreign_endpoint = created();
    foreign_endpoint.witness.endpoint = "/foreign.sock".into();
    foreign_endpoint.validate().unwrap();
    let mut results = Vec::new();
    for evidence in [malformed, foreign_workspace, foreign_endpoint, created()] {
        let (_directory, path, mut db) = file_fixture();
        let id = identity();
        let tx = db.transaction().unwrap();
        topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(0)).unwrap();
        saved_recovery(
            &tx,
            &id,
            cancellation(),
            BootstrapState::Cancelled,
            Some(evidence),
        );
        tx.execute(
            "UPDATE bootstrap_handoffs SET state='cancelled',terminal_at=1 WHERE id=1",
            [],
        )
        .unwrap();
        tx.commit().unwrap();
        drop(db);
        let mut db = Connection::open(path).unwrap();
        schema::initialize(&db, || UtcMillis(2)).unwrap();
        let before = counts(&db);
        results.push(
            topology_handoff::current(&db, &namespace(), &id)
                .err()
                .map(|e| e.code),
        );
        let tx = db.transaction().unwrap();
        results.push(
            topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(3))
                .err()
                .map(|e| e.code),
        );
        tx.commit().unwrap();
        assert_eq!(counts(&db), before);
    }
    assert_eq!(results, vec![Some(ErrorCode::StoreCorrupt); 8]);
}
#[test]
fn recovery_cancelled_reopen_refuses_an_older_attempt_decision() {
    let (_directory, path, mut db) = file_fixture();
    let id = identity();
    let tx = db.transaction().unwrap();
    topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(0)).unwrap();
    next_prepared(&tx, &id);
    saved_recovery(&tx, &id, cancellation(), BootstrapState::Cancelled, None);
    tx.execute(
        "UPDATE bootstrap_handoffs SET state='cancelled',terminal_at=1 WHERE id=1",
        [],
    )
    .unwrap();
    tx.commit().unwrap();
    drop(db);
    let db = Connection::open(path).unwrap();
    schema::initialize(&db, || UtcMillis(2)).unwrap();
    assert_eq!(
        topology_handoff::current(&db, &namespace(), &id)
            .unwrap_err()
            .code,
        ErrorCode::StoreCorrupt
    );
}
#[test]
fn recovery_historical_noncreation_before_later_prepared_attempt_remains_valid() {
    for historical_state in [
        "prepared",
        "possible_creation",
        "not_submitted",
        "outcome_unknown",
    ] {
        for later_created in [false, true] {
            let (_directory, path, mut db) = file_fixture();
            let id = identity();
            let tx = db.transaction().unwrap();
            topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(0)).unwrap();
            tx.execute(
                "UPDATE bootstrap_attempts SET state=?1 WHERE parent_id=1",
                [historical_state],
            )
            .unwrap();
            let result = saved_recovery(
                &tx,
                &id,
                BootstrapRecoveryDisposition::NotCreated {
                    quiescence: BootstrapQuiescenceAssertion::InspectedNoncreationAndQuiescence,
                },
                BootstrapState::Prepared,
                None,
            );
            next_prepared(&tx, &id);
            if later_created {
                tx.execute("UPDATE bootstrap_attempts SET state='created',creation_json=?1 WHERE parent_id=1 AND attempt=2", [serde_json::to_vec(&created()).unwrap()]).unwrap();
                tx.execute(
                    "UPDATE bootstrap_handoffs SET state='created' WHERE id=1",
                    [],
                )
                .unwrap();
            }
            tx.commit().unwrap();
            drop(db);
            let mut db = Connection::open(path).unwrap();
            schema::initialize(&db, || UtcMillis(2)).unwrap();
            let status = topology_handoff::current(&db, &namespace(), &id)
                .unwrap()
                .unwrap();
            assert_eq!(status.attempt, BootstrapAttempt::new(2).unwrap());
            assert_eq!(
                status.state,
                if later_created {
                    BootstrapState::Created
                } else {
                    BootstrapState::Prepared
                }
            );
            assert_eq!(
                status.creation,
                if later_created { Some(created()) } else { None }
            );
            assert_eq!(status.recovery, Some(Box::new(result.clone())));
            let tx = db.transaction().unwrap();
            let old_request = RecoverBootstrap {
                inspection: None,
                identity: id.clone(),
                expected_attempt: result.attempt,
                operation: result.operation.clone(),
                disposition: result.disposition.clone(),
            };
            let request_bytes = serde_json::to_vec(&old_request).unwrap();
            let result_bytes: Vec<u8> = tx
                .query_row(
                    "SELECT result_json FROM bootstrap_recovery_decisions",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                topology_handoff::attempts::recover(
                    &tx,
                    &namespace(),
                    &old_request,
                    501,
                    UtcMillis(3),
                    None
                )
                .unwrap(),
                result
            );
            assert_eq!(serde_json::to_vec(&old_request).unwrap(), request_bytes);
            assert_eq!(
                tx.query_row(
                    "SELECT result_json FROM bootstrap_recovery_decisions",
                    [],
                    |r| r.get::<_, Vec<u8>>(0)
                )
                .unwrap(),
                result_bytes
            );
            assert_eq!(
                topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(3)).unwrap(),
                status
            );
            tx.commit().unwrap();
        }
    }
}

#[test]
fn recovery_created_pane_refuses_result_state_or_normalized_evidence_mismatch() {
    let mut failures = Vec::new();
    for case in 0..5 {
        let mut db = fixture();
        let id = identity();
        let tx = db.transaction().unwrap();
        topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(0)).unwrap();
        let mut normalized = created();
        if case == 3 {
            normalized.root_pane = HostTargetId::new("w:p3");
        }
        tx.execute(
            "UPDATE bootstrap_attempts SET state='created',creation_json=?1 WHERE parent_id=1",
            [serde_json::to_vec(&normalized).unwrap()],
        )
        .unwrap();
        tx.execute(
            "UPDATE bootstrap_handoffs SET state='created' WHERE id=1",
            [],
        )
        .unwrap();
        if case == 4 {
            next_prepared(&tx, &id);
        }
        let mut evidence = created();
        if case == 2 {
            evidence.root_pane = HostTargetId::new("w:p4");
        }
        saved_recovery(
            &tx,
            &id,
            BootstrapRecoveryDisposition::CreatedPane {
                evidence,
                structural_reference: HostCallId::new("inspection"),
            },
            if case == 1 {
                BootstrapState::Prepared
            } else {
                BootstrapState::Created
            },
            if case == 0 { None } else { Some(created()) },
        );
        failures.push(
            topology_handoff::current(&tx, &namespace(), &id)
                .err()
                .map(|e| e.code),
        );
    }
    assert_eq!(failures, vec![Some(ErrorCode::StoreCorrupt); 5]);
}

#[test]
fn recovery_noncreation_refuses_creation_or_wrong_snapshot_state() {
    let mut failures = Vec::new();
    for bad_creation in [true, false] {
        let mut db = fixture();
        let id = identity();
        let tx = db.transaction().unwrap();
        topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(0)).unwrap();
        next_prepared(&tx, &id);
        saved_recovery(
            &tx,
            &id,
            BootstrapRecoveryDisposition::NotCreated {
                quiescence: BootstrapQuiescenceAssertion::InspectedNoncreationAndQuiescence,
            },
            if bad_creation {
                BootstrapState::Prepared
            } else {
                BootstrapState::Created
            },
            if bad_creation { Some(created()) } else { None },
        );
        failures.push(
            topology_handoff::current(&tx, &namespace(), &id)
                .err()
                .map(|e| e.code),
        );
    }
    assert_eq!(failures, vec![Some(ErrorCode::StoreCorrupt); 2]);
}
#[test]
fn recovery_matching_created_and_cancelled_evidence_survive_reopen() {
    for cancelled in [false, true] {
        let (_directory, path, mut db) = file_fixture();
        let id = identity();
        let tx = db.transaction().unwrap();
        topology_handoff::begin_pending(&tx, &namespace(), &id, UtcMillis(0)).unwrap();
        record_created(&tx);
        let state = if cancelled {
            BootstrapState::Cancelled
        } else {
            BootstrapState::Created
        };
        let disposition = if cancelled {
            cancellation()
        } else {
            BootstrapRecoveryDisposition::CreatedPane {
                evidence: created(),
                structural_reference: HostCallId::new("inspection"),
            }
        };
        let retained = saved_recovery(&tx, &id, disposition, state, Some(created()));
        if cancelled {
            tx.execute(
                "UPDATE bootstrap_handoffs SET state='cancelled',terminal_at=1 WHERE id=1",
                [],
            )
            .unwrap();
        }
        tx.commit().unwrap();
        drop(db);
        let db = Connection::open(path).unwrap();
        schema::initialize(&db, || UtcMillis(2)).unwrap();
        let result = topology_handoff::current(&db, &namespace(), &id)
            .unwrap()
            .unwrap();
        assert_eq!(result.state, state);
        assert_eq!(result.creation, Some(created()));
        assert_eq!(result.recovery, Some(Box::new(retained)));
    }
}

#[path = "topology_attachment.rs"]
mod attachment;
#[path = "topology_attempts.rs"]
mod attempts;

fn current_main_historical26() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    for sql in [
        include_str!("../../migrations/0001_initial.sql"),
        include_str!("../../migrations/0002_service_substrate.sql"),
        include_str!("../../migrations/0003_invitation_cancellations.sql"),
        include_str!("../../migrations/0004_voluntary_membership.sql"),
        include_str!("../../migrations/0005_service_notifications.sql"),
        include_str!("../../migrations/0006_retirement_health.sql"),
        include_str!("../../migrations/0007_attention_digest.sql"),
        include_str!("../../migrations/0008_digest_pending_paths.sql"),
        include_str!("../../migrations/0009_human_occupant.sql"),
        include_str!("../../migrations/0010_b5_trust_guards.sql"),
        include_str!("../../migrations/0011_cooperative_only.sql"),
        include_str!("../../migrations/0012_harness_version_evidence.sql"),
        include_str!("../../migrations/0013_thread_summaries.sql"),
        include_str!("../../migrations/0014_catch_up_release.sql"),
        include_str!("../../migrations/0015_preparation_retention.sql"),
        include_str!("../../migrations/0016_human_receipt_waivers.sql"),
        include_str!("../../migrations/0017_wake_batches.sql"),
        include_str!("../../migrations/0018_warning_conditions.sql"),
        include_str!("../../migrations/0019_thread_names.sql"),
        include_str!("../../migrations/0020_recent_activity.sql"),
        include_str!("../../migrations/0021_invitation_rejections.sql"),
        include_str!("../../migrations/0022_user_message_intent.sql"),
        include_str!("../../migrations/0023_channel_archival.sql"),
        include_str!("../../migrations/0024_harness_contract_diagnostics.sql"),
        include_str!("../../migrations/0025_warning_notice_delivery.sql"),
        include_str!("../../migrations/0026_lazy_message_delivery.sql"),
    ] {
        db.execute_batch(sql).unwrap();
    }
    db.pragma_update(None, "user_version", 26).unwrap();
    db
}

#[test]
fn current_main_fresh_and_canonical26_upgrade_to_topology28() {
    for historical in [false, true] {
        let db = if historical {
            current_main_historical26()
        } else {
            Connection::open_in_memory().unwrap()
        };
        let saved = if historical {
            db.execute_batch("INSERT INTO host_instances(id,created_at) VALUES('i',0)")
                .unwrap();
            thread(&db);
            joined_agent(&db);
            let old_claim = r#"{"instance":"i","seat":"s","target":"p","binding_generation":1,"role":"top_level","harness":"codex","native_session":"session","execution":"00000000-0000-4000-8000-000000000001"}"#;
            assert_eq!(
                serde_json::from_str::<herdr_threads::protocol::authority::CallerClaim>(old_claim)
                    .unwrap(),
                super::handoff_fences::identity().claim
            );
            db.execute("INSERT INTO channel_handoff_fences(instance_id,actor_scope,compound,digest,claim_json,recipient,create_key,invite_key,send_key,original_thread,thread_id,origin,state,created_at) VALUES('i','seat:s','compound',?1,?2,'s','create','invite','send','t','t','cooperative_pending_claim','live',1)", rusqlite::params!["a".repeat(64), old_claim]).unwrap();
            Some(
                db.query_row("SELECT claim_json FROM channel_handoff_fences", [], |r| {
                    r.get::<_, String>(0)
                })
                .unwrap(),
            )
        } else {
            None
        };
        schema::initialize(&db, || UtcMillis(0)).unwrap();
        assert_eq!(
            db.pragma_query_value::<i64, _>(None, "user_version", |r| r.get(0))
                .unwrap(),
            28
        );
        assert!(
            db.query_row(
                "SELECT name FROM sqlite_master WHERE name='bootstrap_handoffs'",
                [],
                |r| r.get::<_, String>(0)
            )
            .is_ok()
        );
        if let Some(saved) = saved {
            assert_eq!(
                db.query_row("SELECT claim_json FROM channel_handoff_fences", [], |r| {
                    r.get::<_, String>(0)
                })
                .unwrap(),
                saved
            );
            assert_eq!(
                db.query_row("SELECT ordinal FROM occupant_bindings", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                1
            );
            assert_eq!(
                db.query_row(
                    "SELECT seq FROM sqlite_sequence WHERE name='occupant_bindings'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                1
            );
        }
        schema::initialize(&db, || UtcMillis(2)).unwrap();
    }
}

#[test]
fn current_main_topology28_late_failure_rolls_back() {
    let db = current_main_historical26();
    db.execute_batch(include_str!("../../migrations/0027_harness_adapters.sql"))
        .unwrap();
    db.pragma_update(None, "user_version", 27).unwrap();
    db.execute_batch("CREATE TABLE bootstrap_reports(blocker TEXT)")
        .unwrap();
    let before = db
        .prepare("SELECT type,name,sql FROM sqlite_master ORDER BY type,name")
        .unwrap()
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect::<Vec<_>>();
    assert!(schema::initialize(&db, || UtcMillis(1)).is_err());
    assert_eq!(
        db.pragma_query_value::<i64, _>(None, "user_version", |r| r.get(0))
            .unwrap(),
        27
    );
    assert_eq!(
        db.prepare("SELECT type,name,sql FROM sqlite_master ORDER BY type,name")
            .unwrap()
            .query_map([], |r| Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?
            )))
            .unwrap()
            .map(Result::unwrap)
            .collect::<Vec<_>>(),
        before
    );
}

#[test]
fn current_main_rejects_unpublished_topology27_shape() {
    let db = current_main_historical26();
    // Exact unlanded Root SQL bytes; deliberately not canonical Main adapter27.
    db.execute_batch(include_str!("../../migrations/0028_handoff_topology.sql"))
        .unwrap();
    db.pragma_update(None, "user_version", 27).unwrap();
    let before: String = db
        .query_row(
            "SELECT group_concat(sql) FROM (SELECT sql FROM sqlite_master ORDER BY name)",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        schema::initialize(&db, || UtcMillis(1)).unwrap_err().code,
        ErrorCode::IncompatibleSchema
    );
    assert_eq!(
        db.pragma_query_value::<i64, _>(None, "user_version", |r| r.get(0))
            .unwrap(),
        27
    );
    assert_eq!(
        db.query_row(
            "SELECT group_concat(sql) FROM (SELECT sql FROM sqlite_master ORDER BY name)",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        before
    );
}
