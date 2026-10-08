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
        25
    );
    // Real allocated27 DDL on actual25 fixture; this does not register27 or emulate26.
    db.execute_batch(include_str!("../../migrations/0027_handoff_topology.sql"))
        .unwrap();
    db.execute_batch("INSERT INTO host_instances(id,created_at) VALUES('i',0)")
        .unwrap();
    thread(db);
    joined_agent(db);
}

pub(super) fn fixture() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    setup(&db);
    db
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
    assert_eq!(counts(&tx), (1, 1, 13, 0));
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
    assert_eq!(counts(&tx), (1, 1, 13, 0));
    tx.commit().unwrap();
}

#[test]
fn real_product_ddl_defines_normalized_records_without_registering_startup() {
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
    assert_eq!(counts(&tx), (1, 1, 13, 0));
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
                if replay { (1, 1, 13, 0) } else { (0, 0, 0, 0) }
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
    assert_eq!(counts(&tx), (1, 1, 13, 0));
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
    assert_eq!(counts(&db), (1, 1, 13, 0));
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
    assert_eq!(counts(&tx), (1, 1, 13, 0));
    assert!(tx.execute("INSERT INTO bootstrap_attempts(parent_id,attempt,state,reserve_key,record_key,check_key) VALUES(1,0,'prepared','r0','d0','c0')",[]).is_err());
    assert!(tx.execute("INSERT INTO bootstrap_attempts(parent_id,attempt,state,reserve_key,record_key,check_key) VALUES(1,1,'prepared','r1','d1','c1')",[]).is_err());
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
    let mut db = fixture();
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
        identity: id.clone(),
        expected_attempt: BootstrapAttempt::first(),
        operation: OperationId::new("placeholder"),
        disposition: disposition.clone(),
    };
    decision.operation = decision.decision_operation().unwrap();
    let retained = BootstrapRecoveryResult {
        identity: id.clone(),
        attempt: BootstrapAttempt::first(),
        operation: decision.operation.clone(),
        disposition,
        operator_uid: 501,
        operator_provenance: "operator:local-user:501".into(),
        creation: None,
        state: BootstrapState::Cancelled,
    };
    tx.execute("INSERT INTO bootstrap_recovery_decisions(parent_id,attempt,operation,result_json) VALUES(1,1,?1,?2)",rusqlite::params![decision.operation.as_str(),serde_json::to_vec(&retained).unwrap()]).unwrap();
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
    let mut changed = id;
    changed.claim.binding_generation = 2;
    changed.digest = changed.semantic_digest().unwrap();
    assert_eq!(
        topology_handoff::begin_pending(&tx, &namespace(), &changed, UtcMillis(4))
            .unwrap_err()
            .code,
        ErrorCode::OperationPayloadMismatch
    );
    assert_eq!(counts(&tx), (1, 1, 14, 0));
    tx.commit().unwrap();
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
        assert_eq!(counts(&tx), (1, 1, 13, 0));
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
    let mut db = fixture();
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
    tx.execute_batch("DROP TRIGGER bootstrap_report_immutable; PRAGMA ignore_check_constraints=ON")
        .unwrap();
    tx.execute(
        "UPDATE bootstrap_reports SET completed_json=?1 WHERE parent_id=1",
        [vec![b' '; topology_handoff::MAX_COMPLETED_BYTES + 1]],
    )
    .unwrap();
    assert_eq!(
        topology_handoff::current(&tx, &namespace(), &id)
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
            tx.execute("INSERT INTO bootstrap_attempts(parent_id,attempt,state,reserve_key,record_key,check_key) VALUES(1,?1,?2,?3,?4,?5)",rusqlite::params![number,if number==1001 {"prepared"}else{"not_submitted"},attempt.operation(&id.compound,"reserve").unwrap().as_str(),attempt.operation(&id.compound,"record").unwrap().as_str(),attempt.operation(&id.compound,"check").unwrap().as_str()]).unwrap();
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
