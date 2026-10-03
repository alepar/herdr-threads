use super::*;
use crate::{
    ports::{
        ClosureEvidence, DuePhaseProgress, DueScanRequest, DueScanState, DurableWorkAdmission,
        OperationReadScope, ReadContext, RegisterAvailableRequest, SendPreparationProgress,
        StorePort, WorkAdmission,
    },
    protocol::{
        authority::{CallerClaim, Harness, MutationPermit},
        commands::{
            Accept, Ack, CheckIn, Command, CreateThread, DirectoryMembership, DirectoryQuery,
            InboxQuery, Invite, OperationStatusQuery, PermitMutation, SendMessage, ThreadMutation,
        },
        ids::{
            ExecutionId, HostBootId, HostTargetId, NativeSessionId, OperationId, SeatId, ThreadId,
        },
        output::OutputSpec,
        pagination::PageRequest,
        results::CommandResult,
        time::{CallBudget, Cancellation, Clock, MonoInstant, UtcMillis},
    },
};
use std::sync::Arc;

struct FixedClock;
impl Clock for FixedClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(100)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(1)
    }
}

#[test]
fn service_recovery_audit_is_durable_and_uses_a_reserved_operation_scope() {
    let instance = uuid::Uuid::new_v4().to_string();
    let boot = uuid::Uuid::new_v4();
    let path =
        std::env::temp_dir().join(format!("herdr-recovery-audit-{}.db", uuid::Uuid::new_v4()));
    let settings = StoreSettings {
        daemon_boot: Some(boot),
        ..StoreSettings::default()
    };
    let store = SqliteStore::new(
        connection::StoreContext::new(path.clone(), Arc::new(FixedClock)),
        instance.clone(),
        settings.clone(),
    )
    .unwrap();
    let budget = CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Cancellation::default(),
    };
    let peer = crate::protocol::authority::PeerIdentity::from_kernel(501);
    store
        .audit_service_disconnect(&boot.to_string(), 7, peer, &budget)
        .unwrap();
    drop(store);
    let reopened = SqliteStore::new(
        connection::StoreContext::new(path.clone(), Arc::new(FixedClock)),
        instance.clone(),
        settings,
    )
    .unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    let scope = format!("service-recovery-audit:{instance}");
    let key = format!("{boot}:7");
    let (stored_scope, stored_key, payload, digest_len): (String, String, String, i64) = db
        .query_row(
            "SELECT actor_scope,operation_key,result_json,length(digest) FROM operations",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(stored_scope, scope);
    assert_eq!(stored_key, key);
    assert_eq!(digest_len, 32);
    assert!(!stored_scope.starts_with("seat:"));
    assert!(!stored_scope.starts_with("operator:"));
    assert!(!stored_scope.starts_with("service-allocation:"));
    let event: serde_json::Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(event["event"], "operator_service_disconnect");
    assert_eq!(event["instance"], instance);
    assert_eq!(event["daemon_boot"], boot.to_string());
    assert_eq!(event["connection_generation"], 7);
    assert_eq!(event["actor"], "operator:local-user:501");
    let cancelled = CallBudget {
        deadline: MonoInstant(1000),
        cancellation: Cancellation::default(),
    };
    cancelled.cancellation.cancel();
    assert_eq!(
        reopened
            .audit_service_disconnect(&boot.to_string(), 8, peer, &cancelled)
            .unwrap_err()
            .code,
        ErrorCode::Cancelled
    );
    let count: i64 = db
        .query_row("SELECT count(*) FROM operations", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);
    drop(reopened);
    drop(db);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn public_facade_invitation_due_scan_is_idempotent_after_reopen() {
    let path = std::env::temp_dir().join(format!("herdr-facade-{}.db", uuid::Uuid::new_v4()));
    let context = connection::StoreContext::new(path.clone(), Arc::new(FixedClock));
    let db = context.open_writer().unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at) VALUES ('i',0)",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('s2','i','resolved','native',1,0)",[]).unwrap();
    db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0)",[]).unwrap();
    db.execute(
        "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t','s2','invited')",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES ('inv','t','s2',1,'pending',0,1,100,100)",[]).unwrap();
    drop(db);
    let store = SqliteStore::new(context, "i", StoreSettings::default()).unwrap();
    let request = DueScanRequest {
        state: DueScanState::default(),
        max_candidates: 10,
        run_invitations: true,
        run_receipts: false,
    };
    let first = StorePort::due_obligations(&store, request.clone(), &budget()).unwrap();
    assert_eq!(first.warnings_added, 1);
    assert!(matches!(first.invitations, DuePhaseProgress::Complete));
    drop(store);
    let reopened = SqliteStore::new(
        connection::StoreContext::new(path.clone(), Arc::new(FixedClock)),
        "i",
        StoreSettings::default(),
    )
    .unwrap();
    let second = StorePort::due_obligations(&reopened, request, &budget()).unwrap();
    assert_eq!(second.warnings_added, 0);
    let db = reopened.context.open_writer().unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM messages WHERE thread_id='t' AND kind='warn'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    drop(db);
    drop(reopened);
    let _ = std::fs::remove_file(path);
}

#[test]
fn public_facade_late_accept_and_ack_share_unique_due_warnings() {
    let path = std::env::temp_dir().join(format!("herdr-facade-{}.db", uuid::Uuid::new_v4()));
    let context = connection::StoreContext::new(path.clone(), Arc::new(FixedClock));
    let db = context.open_writer().unwrap();
    db.execute_batch("\
        INSERT INTO host_instances(id,created_at,host_boot,host_epoch) VALUES ('i',0,'b',1);\
        INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('s','i','resolved','native','p',1,1,0);\
        INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','p','b',1,1,0,'fresh','term-'||'p','inc','coherent_enumeration',1);\
        INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) VALUES ('s',1,1,'p','b',1,'codex','n','00000000-0000-4000-8000-000000000001','cooperative_top_level',0,'term-'||'p','inc');\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence) VALUES ('t','i','topic','goal',0,0,2);\
        INSERT INTO memberships(thread_id,seat_id,state,episode) VALUES ('t','s','invited',1);\
        INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES ('inv','t','s',1,'pending',0,1,50,50);\
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_seq,decision_at) VALUES ('m','i','t',1,'ordinary','body',1,0);\
        INSERT INTO receipts(message_id,thread_id,seat_id,state,available_at,deadline_at,frozen_duration_ms) VALUES ('m','t','s','pending',0,50,50);\
    ").unwrap();
    drop(db);
    let store = SqliteStore::new(context, "i", StoreSettings::default()).unwrap();
    let accept = Accept {
        thread: ThreadId::new("t"),
        operation: OperationId::new("late-accept"),
        claim: fixture_claim("s", "p"),
    };
    assert!(matches!(
        permitted(&store, PermitMutation::Accept(accept.clone())).unwrap(),
        CommandResult::Accepted(_)
    ));
    let ack = Ack {
        messages: vec![crate::protocol::ids::MessageId::new("m")],
        operation: OperationId::new("late-ack"),
        claim: fixture_claim("s", "p"),
    };
    assert!(matches!(
        permitted(&store, PermitMutation::Ack(ack.clone())).unwrap(),
        CommandResult::Acknowledged(_)
    ));
    let scan = DueScanRequest {
        state: DueScanState::default(),
        max_candidates: 10,
        run_invitations: true,
        run_receipts: true,
    };
    assert_eq!(
        StorePort::due_obligations(&store, scan.clone(), &budget())
            .unwrap()
            .warnings_added,
        0
    );
    let db = store.context.open_writer().unwrap();
    let warnings: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE thread_id='t' AND kind='warn'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(warnings, 2);
    let by_source: (i64, i64) = db.query_row("SELECT SUM(source_invitation_id='inv'),SUM(source_message_id='m') FROM messages WHERE thread_id='t' AND kind='warn'", [], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
    assert_eq!(by_source, (1, 1));
    drop(db);
    drop(store);
    let reopened = SqliteStore::new(
        connection::StoreContext::new(path.clone(), Arc::new(FixedClock)),
        "i",
        StoreSettings::default(),
    )
    .unwrap();
    assert_eq!(
        StorePort::due_obligations(&reopened, scan, &budget())
            .unwrap()
            .warnings_added,
        0
    );
    drop(reopened);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn public_facade_operator_invites_into_an_archived_orphan_without_receipt_authority() {
    use crate::ports::OperatorRequest;
    use crate::protocol::{
        authority::{OperatorActor, PeerIdentity},
        commands::OperatorOrphanInvite,
    };
    let path = std::env::temp_dir().join(format!("herdr-facade-{}.db", uuid::Uuid::new_v4()));
    let context = connection::StoreContext::new(path.clone(), Arc::new(FixedClock));
    let db = context.open_writer().unwrap();
    db.execute_batch("\
        INSERT INTO host_instances(id,created_at) VALUES ('i',0);\
        INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('s','i','unresolved','native',1,0);\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,archived) VALUES ('orphan','i','topic','goal',0,0,1);\
    ").unwrap();
    drop(db);
    let store = SqliteStore::new(
        context,
        "i",
        StoreSettings {
            invitation_default_ms: Some(120_000),
            ..StoreSettings::default()
        },
    )
    .unwrap();
    let actor = OperatorActor::from_peer(PeerIdentity::from_kernel(501), 501).unwrap();
    let command = OperatorOrphanInvite {
        thread: ThreadId::new("orphan"),
        seat: SeatId::new("s"),
        deadline_millis: None,
        operation: OperationId::new("operator-orphan"),
    };
    let CommandResult::OperatorInvited(invitation) = StorePort::mutate_operator(
        &store,
        OperatorRequest::OrphanInvite(command),
        actor,
        &budget(),
    )
    .unwrap() else {
        panic!("wrong operator result")
    };
    let db = store.context.open_writer().unwrap();
    let row: (String, i64, i64) = db
        .query_row(
            "SELECT state,frozen_duration_ms,deadline_at FROM invitations WHERE id=?1",
            [invitation.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(row, ("pending".into(), 120_000, 120_100));
    assert_eq!(
        db.query_row(
            "SELECT archived FROM threads WHERE id='orphan'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    let audit: i64 = db.query_row("SELECT COUNT(*) FROM messages WHERE thread_id='orphan' AND kind='info' AND json_extract(event_json,'$.actor')='operator:local-user:501'", [], |row| row.get(0)).unwrap();
    assert_eq!(audit, 1);
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM receipts WHERE seat_id='s'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    drop(db);
    drop(store);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn public_facade_pages_participants_recipients_and_canonical_pending_ids() {
    use crate::cli::commands::{CliAction, parse_argv};
    use crate::protocol::commands::{ParticipantsQuery, PendingReceiptsQuery, RecipientsQuery};
    let path = std::env::temp_dir().join(format!("herdr-facade-{}.db", uuid::Uuid::new_v4()));
    let context = connection::StoreContext::new(path.clone(), Arc::new(FixedClock));
    let db = context.open_writer().unwrap();
    db.execute_batch("\
        INSERT INTO host_instances(id,created_at) VALUES ('i',0);\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence) VALUES ('t','i','topic','goal',0,0,2);\
    ").unwrap();
    for seat in ["s1", "s2", "s3"] {
        db.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES (?1,'i','resolved','native',1,0)", [seat]).unwrap();
        db.execute(
            "INSERT INTO memberships(thread_id,seat_id,state,joined_at) VALUES ('t',?1,'joined',0)",
            [seat],
        )
        .unwrap();
    }
    db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,decision_seq,decision_at) VALUES ('m','i','t',1,'ordinary','s1','body',1,0)", []).unwrap();
    for seat in ["s1", "s2", "s3"] {
        db.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES ('m','t',?1,'pending',300)", [seat]).unwrap();
    }
    drop(db);
    let store = SqliteStore::new(context, "i", StoreSettings::default()).unwrap();
    let read = ReadContext {
        instance: "i".into(),
        output: OutputSpec::default(),
        operation_scope: None,
    };
    let page = |cursor| PageRequest {
        cursor,
        limit: 1,
        max_bytes: 65_536,
    };
    let mut participants = Vec::new();
    let mut cursor = None;
    loop {
        let command = Command::Participants(ParticipantsQuery {
            thread: ThreadId::new("t"),
            page: page(cursor),
            caller: None,
        });
        let CommandResult::Participants(result) =
            StorePort::query(&store, &command, &read, &budget()).unwrap()
        else {
            panic!("wrong participants route")
        };
        participants.extend(result.items.iter().map(|row| row.seat.as_str().to_owned()));
        if let Some(argv) = &result.next_argv {
            assert!(
                matches!(parse_argv(argv.clone()).unwrap().action, CliAction::Wire(Command::Participants(q)) if q.thread.as_str()=="t" && q.page.cursor==result.next_cursor)
            );
        }
        cursor = result.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(participants, ["s1", "s2", "s3"]);
    let mut recipients = Vec::new();
    let mut cursor = None;
    loop {
        let command = Command::Recipients(RecipientsQuery {
            message: crate::protocol::ids::MessageId::new("m"),
            page: page(cursor),
        });
        let CommandResult::Recipients(result) =
            StorePort::query(&store, &command, &read, &budget()).unwrap()
        else {
            panic!("wrong recipients route")
        };
        recipients.extend(result.items.iter().map(|row| row.seat.as_str().to_owned()));
        if let Some(argv) = &result.next_argv {
            assert!(
                matches!(parse_argv(argv.clone()).unwrap().action, CliAction::Wire(Command::Recipients(q)) if q.message.as_str()=="m" && q.page.cursor==result.next_cursor)
            );
        }
        cursor = result.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(recipients, ["s1", "s2", "s3"]);
    let mut pending = Vec::new();
    let mut cursor = None;
    loop {
        let command = Command::PendingReceipts(PendingReceiptsQuery {
            seat: None,
            thread: Some(ThreadId::new("t")),
            page: page(cursor),
        });
        let CommandResult::PendingReceipts(result) =
            StorePort::query(&store, &command, &read, &budget()).unwrap()
        else {
            panic!("wrong pending route")
        };
        pending.extend(result.items.iter().map(|row| {
            (
                row.message.as_str().to_owned(),
                row.seat.as_str().to_owned(),
            )
        }));
        if let Some(argv) = &result.next_argv {
            assert!(
                matches!(parse_argv(argv.clone()).unwrap().action, CliAction::Wire(Command::PendingReceipts(q)) if q.thread.as_ref().map(ThreadId::as_str)==Some("t") && q.page.cursor==result.next_cursor)
            );
        }
        cursor = result.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(
        pending,
        [
            ("m".into(), "s1".into()),
            ("m".into(), "s2".into()),
            ("m".into(), "s3".into())
        ]
    );
    drop(store);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn receipt_only_due_admission_runs_even_when_invitation_has_priority() {
    let path = std::env::temp_dir().join(format!("herdr-facade-{}.db", uuid::Uuid::new_v4()));
    let context = connection::StoreContext::new(path.clone(), Arc::new(FixedClock));
    let db = context.open_writer().unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at) VALUES ('i',0)",
        [],
    )
    .unwrap();
    drop(db);
    let store = SqliteStore::new(context, "i", StoreSettings::default()).unwrap();
    let progress = StorePort::due_obligations(
        &store,
        DueScanRequest {
            state: DueScanState::default(),
            max_candidates: 5,
            run_invitations: false,
            run_receipts: true,
        },
        &budget(),
    )
    .unwrap();
    assert!(matches!(progress.invitations, DuePhaseProgress::Skipped));
    assert!(matches!(progress.receipts, DuePhaseProgress::Complete));
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[test]
fn one_candidate_due_budget_retains_unvisited_admitted_phase() {
    let path = std::env::temp_dir().join(format!("herdr-facade-{}.db", uuid::Uuid::new_v4()));
    let context = connection::StoreContext::new(path.clone(), Arc::new(FixedClock));
    let db = context.open_writer().unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at) VALUES ('i',0)",
        [],
    )
    .unwrap();
    drop(db);
    let store = SqliteStore::new(context, "i", StoreSettings::default()).unwrap();
    let first = StorePort::due_obligations(
        &store,
        DueScanRequest {
            state: DueScanState::default(),
            max_candidates: 1,
            run_invitations: true,
            run_receipts: true,
        },
        &budget(),
    )
    .unwrap();
    assert!(matches!(first.invitations, DuePhaseProgress::Complete));
    assert!(matches!(first.receipts, DuePhaseProgress::Skipped));
    assert!(first.has_more);
    let second = StorePort::due_obligations(
        &store,
        DueScanRequest {
            state: first.state,
            max_candidates: 1,
            run_invitations: true,
            run_receipts: true,
        },
        &budget(),
    )
    .unwrap();
    assert!(matches!(second.receipts, DuePhaseProgress::Complete));
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[test]
fn pending_work_pages_global_job_order_and_keeps_a_reachable_cursor() {
    let path = std::env::temp_dir().join(format!("herdr-facade-{}.db", uuid::Uuid::new_v4()));
    let context = connection::StoreContext::new(path.clone(), Arc::new(FixedClock));
    let db = context.open_writer().unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at) VALUES ('i',0)",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES ('j1','preparation_cleanup','p1',0)",[]).unwrap();
    db.execute("INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES ('j2','send_attention','p2',0)",[]).unwrap();
    drop(db);
    let store = SqliteStore::new(context, "i", StoreSettings::default()).unwrap();
    let first = StorePort::pending_work(
        &store,
        PageRequest {
            cursor: None,
            limit: 1,
            max_bytes: 4096,
        },
        &budget(),
    )
    .unwrap();
    assert_eq!(first.items.len(), 1);
    assert_eq!(first.items[0].id, "j1");
    assert!(first.has_more);
    let second = StorePort::pending_work(
        &store,
        PageRequest {
            cursor: first.next_cursor,
            limit: 1,
            max_bytes: 4096,
        },
        &budget(),
    )
    .unwrap();
    assert_eq!(second.items.len(), 1);
    assert_eq!(second.items[0].id, "j2");
    assert!(!second.has_more);
    drop(store);
    let _ = std::fs::remove_file(path);
}

// ht-p03.12.4: discovery reads `work_jobs_live`, so a full slice of completed
// jobs is never visited and the first page already holds the ready job.
#[test]
fn pending_work_skips_a_full_slice_of_completed_jobs() {
    let path = std::env::temp_dir().join(format!("herdr-facade-{}.db", uuid::Uuid::new_v4()));
    let context = connection::StoreContext::new(path.clone(), Arc::new(FixedClock));
    let db = context.open_writer().unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at) VALUES ('i',0)",
        [],
    )
    .unwrap();
    for n in 0..100 {
        db.execute("INSERT INTO work_jobs(id,kind,subject_id,high_water,status) VALUES (?1,'preparation_cleanup',?1,0,'complete')",[format!("done-{n}")]).unwrap();
    }
    db.execute("INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES ('ready','send_attention','ready',0)",[]).unwrap();
    drop(db);
    let store = SqliteStore::new(context, "i", StoreSettings::default()).unwrap();
    let first = StorePort::pending_work(&store, PageRequest::default(), &budget()).unwrap();
    assert_eq!(first.items.len(), 1);
    assert_eq!(first.items[0].id, "ready");
    assert!(!first.has_more && first.next_cursor.is_none());
    drop(store);
    let _ = std::fs::remove_file(path);
}

// With only completed jobs the live scan is empty and the page is final: no
// continuation cursor, and the page fits the smallest legal byte budget.
#[test]
fn pending_work_with_only_completed_jobs_is_an_empty_final_page() {
    let path = std::env::temp_dir().join(format!("herdr-facade-{}.db", uuid::Uuid::new_v4()));
    let context = connection::StoreContext::new(path.clone(), Arc::new(FixedClock));
    let db = context.open_writer().unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at) VALUES ('i',0)",
        [],
    )
    .unwrap();
    for n in 0..100 {
        db.execute("INSERT INTO work_jobs(id,kind,subject_id,high_water,status) VALUES (?1,'preparation_cleanup',?1,0,'complete')", [format!("done-{n}")]).unwrap();
    }
    drop(db);
    let store = SqliteStore::new(context, "i", StoreSettings::default()).unwrap();
    let complete = StorePort::pending_work(&store, PageRequest::default(), &budget()).unwrap();
    assert!(complete.items.is_empty() && !complete.has_more && complete.next_cursor.is_none());
    assert!(serde_json::to_vec(&complete).unwrap().len() <= 256);
    let smallest = StorePort::pending_work(
        &store,
        PageRequest {
            max_bytes: 256,
            ..PageRequest::default()
        },
        &budget(),
    )
    .unwrap();
    assert_eq!(smallest, complete);
    drop(store);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn pending_work_fits_escaped_candidate_and_reaches_withheld_row() {
    let path = std::env::temp_dir().join(format!("herdr-facade-{}.db", uuid::Uuid::new_v4()));
    let context = connection::StoreContext::new(path.clone(), Arc::new(FixedClock));
    let db = context.open_writer().unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at) VALUES ('i',0)",
        [],
    )
    .unwrap();
    let escaped = "job\"\\\n\t雪";
    let later_id = format!("later-{}", "x".repeat(1_500));
    db.execute(
        "INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES (?1,'send_attention',?1,0)",
        [escaped],
    )
    .unwrap();
    db.execute(
        "INSERT INTO work_jobs(id,kind,subject_id,high_water) VALUES (?1,'send_attention',?1,0)",
        [&later_id],
    )
    .unwrap();
    drop(db);
    let store = SqliteStore::new(context, "i", StoreSettings::default()).unwrap();
    let first_only = StorePort::pending_work(
        &store,
        PageRequest {
            limit: 1,
            ..PageRequest::default()
        },
        &budget(),
    )
    .unwrap();
    let exact = serde_json::to_vec(&first_only).unwrap().len() as u32 + 16;
    let first = StorePort::pending_work(
        &store,
        PageRequest {
            limit: 2,
            max_bytes: exact,
            cursor: None,
        },
        &budget(),
    )
    .unwrap();
    assert_eq!(first.items.len(), 1);
    assert_eq!(first.items[0].id, escaped);
    assert!(first.has_more);
    assert!(serde_json::to_vec(&first).unwrap().len() as u32 <= exact);
    let later = StorePort::pending_work(
        &store,
        PageRequest {
            cursor: first.next_cursor,
            ..PageRequest::default()
        },
        &budget(),
    )
    .unwrap();
    assert_eq!(
        later
            .items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        [later_id.as_str()]
    );
    drop(store);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn public_facade_publishes_an_empty_coherent_snapshot_atomically() {
    use crate::ports::{
        EnumerationEvidence, EvidenceKind, HostSnapshot, IncarnationEvidence, SnapshotHeader,
    };
    let path = std::env::temp_dir().join(format!("herdr-facade-{}.db", uuid::Uuid::new_v4()));
    let context = connection::StoreContext::new(path.clone(), Arc::new(FixedClock));
    let db = context.open_writer().unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at) VALUES ('i',0)",
        [],
    )
    .unwrap();
    drop(db);
    let store = SqliteStore::new(context, "i", StoreSettings::default()).unwrap();
    let admission = StorePort::begin_host_observation(&store, "i", &budget()).unwrap();
    let snapshot = HostSnapshot {
        boot: HostBootId::new("b"),
        epoch: 1,
        observation_sequence: 1,
        complete: true,
        enumeration: EnumerationEvidence::CoherentVerified,
        incarnation: IncarnationEvidence::Verified {
            identity: "incarnation".into(),
            evidence_kind: EvidenceKind::CoherentEnumeration,
        },
        targets: vec![],
    };
    let header = SnapshotHeader::from_captured(admission, &snapshot).unwrap();
    let stage = StorePort::begin_snapshot_stage(&store, header, &budget()).unwrap();
    assert_eq!(stage.expected_targets, 0);
    StorePort::seal_snapshot_stage(&store, &stage.id, &budget()).unwrap();
    let published = StorePort::publish_snapshot_stage(&store, &stage.id, &budget()).unwrap();
    assert_eq!(published.id, stage.id);
    assert_eq!(published.target_count, 0);
    let saved = StorePort::saved_seats_page(&store, &published.id, 0, None, 16, &budget()).unwrap();
    assert!(saved.seats.is_empty());
    assert!(!saved.has_more);
    let db = store.context.open_writer().unwrap();
    assert_eq!(
        db.query_row(
            "SELECT active_snapshot_id FROM host_instances WHERE id='i'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        published.id.as_str()
    );
    drop(db);
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[test]
fn public_facade_retirement_fences_before_bounded_cleanup() {
    let path = std::env::temp_dir().join(format!("herdr-facade-{}.db", uuid::Uuid::new_v4()));
    let context = connection::StoreContext::new(path.clone(), Arc::new(FixedClock));
    let db = context.open_writer().unwrap();
    db.execute("INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'b',1,1)",[]).unwrap();
    db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('s2','i','resolved','native','p2',1,1,0)",[]).unwrap();
    db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','p2','b',1,1,0,'fresh','term-'||'p2','inc','coherent_enumeration',1)",[]).unwrap();
    db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0)",[]).unwrap();
    db.execute(
        "INSERT INTO memberships(thread_id,seat_id,state,joined_at) VALUES ('t','s2','joined',0)",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('t','s2',1,1)",[]).unwrap();
    drop(db);
    let store = SqliteStore::new(context, "i", StoreSettings::default()).unwrap();
    let job = StorePort::begin_retirement(
        &store,
        SeatId::new("s2"),
        ClosureEvidence {
            host_boot: HostBootId::new("b"),
            epoch: 1,
            target: HostTargetId::new("p2"),
            generation: 1,
        },
        &budget(),
    )
    .unwrap();
    let db = store.context.open_writer().unwrap();
    assert_eq!(
        db.query_row("SELECT state FROM seats WHERE id='s2'", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "retired"
    );
    assert_eq!(
        db.query_row(
            "SELECT state FROM memberships WHERE thread_id='t' AND seat_id='s2'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "joined"
    );
    drop(db);
    let progress =
        StorePort::advance_retirement(&store, job.id, WorkAdmission::Background, &budget())
            .unwrap();
    assert!(progress.complete && progress.processed_this_turn <= 16);
    drop(store);
    let _ = std::fs::remove_file(path);
}

struct AdvancingClock(std::sync::atomic::AtomicI64);
impl Clock for AdvancingClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(self.0.load(std::sync::atomic::Ordering::SeqCst))
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(1)
    }
}

#[test]
fn public_facade_mixed_retirement_keeps_cutover_across_quanta_and_reopen() {
    use crate::protocol::{
        authority::{OperatorActor, PeerIdentity},
        commands::{OperatorOrphanInvite, PendingReceiptsQuery},
        results::CleanupState,
    };
    let path = std::env::temp_dir().join(format!("herdr-facade-{}.db", uuid::Uuid::new_v4()));
    let clock = Arc::new(AdvancingClock(std::sync::atomic::AtomicI64::new(100)));
    let context = connection::StoreContext::new(path.clone(), clock.clone());
    let db = context.open_writer().unwrap();
    db.execute_batch("\
        INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'b',1,30);\
        INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('retiring','i','resolved','native','pane',1,1,0);\
        INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('other','i','resolved','native',1,0);\
        INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','pane','b',1,1,0,'fresh','term-'||'pane','inc','coherent_enumeration',1);\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence) VALUES ('receipts','i','topic','goal',0,0,21);\
        INSERT INTO memberships(thread_id,seat_id,state,joined_at) VALUES ('receipts','retiring','joined',0);\
        INSERT INTO membership_intervals(thread_id,seat_id,episode,joined_seq) VALUES ('receipts','retiring',1,1);\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,archived) VALUES ('unrelated','i','topic','goal',0,0,1);\
    ").unwrap();
    for (name, deadline) in [("late-invite", 90), ("early-invite", 101)] {
        db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES (?1,'i','topic','goal',0,0)", [name]).unwrap();
        db.execute(
            "INSERT INTO memberships(thread_id,seat_id,state) VALUES (?1,'retiring','invited')",
            [name],
        )
        .unwrap();
        db.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES (?1,?1,'retiring',1,'pending',0,1,?2,?2)", rusqlite::params![name,deadline]).unwrap();
    }
    for n in 0..20 {
        let id = format!("receipt-{n}");
        db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,decision_seq,decision_at) VALUES (?1,'i','receipts',?2,'ordinary','other','body',?2,0)", rusqlite::params![id,n+1]).unwrap();
        db.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,available_at,deadline_at) VALUES (?1,'receipts','retiring',?2,90,0,?3)", rusqlite::params![id,if n==0 {"acked"} else {"pending"},if n==19 {101} else {90}]).unwrap();
    }
    for (n, duration) in [(21, 90), (22, 101)] {
        let prep = format!("logical-prep-{n}");
        let message = format!("logical-message-{n}");
        db.execute("INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,recipient_count,status) VALUES (?1,'i','seat:other',?1,zeroblob(32),'receipts',0,0,0,0,0,0,0,1,'sealed')", [&prep]).unwrap();
        db.execute("INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES (?1,'receipts','retiring',1,?2,1)", rusqlite::params![prep,duration]).unwrap();
        db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,actor_seat_id,body,decision_seq,decision_at) VALUES (?1,'i','receipts',?2,'ordinary','other','body',?2,0)", rusqlite::params![message,n]).unwrap();
        db.execute("INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES (?1,?2,'i','receipts',?3,0,?3,0,1,0)", rusqlite::params![prep,message,n]).unwrap();
    }
    db.execute(
        "UPDATE threads SET next_sequence=23 WHERE id='receipts'",
        [],
    )
    .unwrap();
    drop(db);
    let store = SqliteStore::new(context, "i", StoreSettings::default()).unwrap();
    let job = StorePort::begin_retirement(
        &store,
        SeatId::new("retiring"),
        ClosureEvidence {
            host_boot: HostBootId::new("b"),
            epoch: 1,
            target: HostTargetId::new("pane"),
            generation: 1,
        },
        &budget(),
    )
    .unwrap();
    assert_eq!(job.retired_at, UtcMillis(100));
    let read = ReadContext {
        instance: "i".into(),
        output: OutputSpec::default(),
        operation_scope: None,
    };
    let CommandResult::PendingReceipts(pending) = StorePort::query(
        &store,
        &Command::PendingReceipts(PendingReceiptsQuery {
            seat: Some(SeatId::new("retiring")),
            thread: Some(ThreadId::new("receipts")),
            page: PageRequest::default(),
        }),
        &read,
        &budget(),
    )
    .unwrap() else {
        panic!("wrong pending result")
    };
    assert!(pending.items.is_empty());
    let status =
        StorePort::pending_retirement_jobs(&store, PageRequest::default(), &budget()).unwrap();
    assert_eq!(status.items.len(), 1);
    assert!(status.items[0].effective_retired);
    assert!(!status.items[0].warning_history_complete);
    let db = store.context.open_writer().unwrap();
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM receipts WHERE state='recipient_retired'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT state FROM memberships WHERE thread_id='receipts'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "joined"
    );
    drop(db);
    clock.0.store(200, std::sync::atomic::Ordering::SeqCst);
    // This foreground mutation gets a separate writer decision while cleanup is pending.
    let actor = OperatorActor::from_peer(PeerIdentity::from_kernel(501), 501).unwrap();
    StorePort::mutate_operator(
        &store,
        crate::ports::OperatorRequest::OrphanInvite(OperatorOrphanInvite {
            thread: ThreadId::new("unrelated"),
            seat: SeatId::new("other"),
            deadline_millis: None,
            operation: OperationId::new("foreground"),
        }),
        actor,
        &budget(),
    )
    .unwrap();
    let first =
        StorePort::advance_retirement(&store, job.id.clone(), WorkAdmission::Background, &budget())
            .unwrap();
    assert!(!first.complete);
    assert!(first.processed_this_turn <= 16);
    assert!(first.processed_this_turn > 0);
    let prefix = first.processed_total;
    drop(store);
    let store = SqliteStore::new(
        connection::StoreContext::new(path.clone(), clock),
        "i",
        StoreSettings::default(),
    )
    .unwrap();
    let status =
        StorePort::pending_retirement_jobs(&store, PageRequest::default(), &budget()).unwrap();
    assert_eq!(status.items[0].processed_units, prefix);
    let mut turns = 1;
    loop {
        let progress = StorePort::advance_retirement(
            &store,
            job.id.clone(),
            WorkAdmission::Background,
            &budget(),
        )
        .unwrap();
        assert!(progress.processed_this_turn <= 16);
        turns += 1;
        if progress.complete {
            break;
        }
        assert!(turns < 10);
    }
    assert!(turns > 1);
    let scan = DueScanRequest {
        state: DueScanState::default(),
        max_candidates: 100,
        run_invitations: true,
        run_receipts: true,
    };
    for _ in 0..2 {
        assert_eq!(
            StorePort::due_obligations(&store, scan.clone(), &budget())
                .unwrap()
                .warnings_added,
            0
        );
    }
    let status =
        StorePort::pending_retirement_jobs(&store, PageRequest::default(), &budget()).unwrap();
    assert_eq!(status.items[0].cleanup_state, CleanupState::Complete);
    assert!(status.items[0].warning_history_complete);
    let db = store.context.open_writer().unwrap();
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM receipts WHERE state='recipient_retired'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        19
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM receipts WHERE state='acked'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(db.query_row("SELECT COUNT(*) FROM invitations WHERE seat_id='retiring' AND state='recipient_retired'",[],|row|row.get::<_,i64>(0)).unwrap(),2);
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM messages WHERE kind='warn'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        20
    );
    assert_eq!(db.query_row("SELECT COUNT(*) FROM messages WHERE kind='warn' AND (source_message_id='receipt-19' OR source_invitation_id='early-invite' OR source_message_id='logical-message-22')",[],|row|row.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM receipt_state WHERE state='recipient_retired'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM retirement_audits WHERE job_id=?1",
            [job.id.as_str()],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        3
    );
    assert_eq!(db.query_row("SELECT COUNT(*) FROM (SELECT event_key FROM messages WHERE kind='warn' GROUP BY event_key HAVING COUNT(*)>1)",[],|row|row.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(db.query_row("SELECT COUNT(*) FROM messages w JOIN messages a ON w.thread_id=a.thread_id WHERE w.kind='warn' AND a.event_key LIKE 'retirement:%' AND w.sequence>=a.sequence",[],|row|row.get::<_,i64>(0)).unwrap(),0);
    drop(db);
    drop(store);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn public_facade_operator_fresh_and_rebind_keep_provenance_without_receipt_authority() {
    use crate::{
        ports::{
            EnumerationEvidence, EvidenceKind, ExecutionEvidence, HostObservation, HostSnapshot,
            HostUiState, IncarnationEvidence, ObservationProvenance, OperatorRequest,
            OperatorTargetGuard, SnapshotHeader, StructuralOccupancy,
        },
        protocol::{
            authority::{OperatorActor, PeerIdentity},
            commands::{OperatorCommand, OperatorFreshSeat, OperatorRebind},
            ids::{HostCallId, TerminalId},
        },
    };
    let path = std::env::temp_dir().join(format!("herdr-facade-{}.db", uuid::Uuid::new_v4()));
    let context = connection::StoreContext::new(path.clone(), Arc::new(FixedClock));
    let db = context.open_writer().unwrap();
    db.execute_batch("\
        INSERT INTO host_instances(id,created_at) VALUES ('i',0);\
        INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('saved','i','unresolved','native',7,0);\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at,next_sequence) VALUES ('t','i','topic','goal',0,0,2);\
        INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t','saved','invited');\
        INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES ('pending-invite','t','saved',1,'pending',0,1,300,300);\
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_seq,decision_at) VALUES ('pending-message','i','t',1,'ordinary','body',1,0);\
        INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES ('pending-message','t','saved','pending',300);\
        INSERT INTO wake_work(seat_id,reason_bits,retry_step,reservation_id,reservation_boot,reserved_at_utc,minimum_delay_ms,effective_delay_ms,last_reservation_id,last_reservation_boot,last_reserved_at_utc,last_outcome) VALUES ('saved',1,3,'active-attempt','prior-daemon',50,60000,120000,'active-attempt','prior-daemon',50,'submitted');\
    ").unwrap();
    drop(db);
    let store = SqliteStore::new(context, "i", StoreSettings::default()).unwrap();
    let admission = StorePort::begin_host_observation(&store, "i", &budget()).unwrap();
    let snapshot = HostSnapshot {
        boot: HostBootId::new("b"),
        epoch: 1,
        observation_sequence: 1,
        complete: true,
        enumeration: EnumerationEvidence::CoherentVerified,
        incarnation: IncarnationEvidence::Verified {
            identity: "incarnation".into(),
            evidence_kind: EvidenceKind::CoherentEnumeration,
        },
        targets: vec![],
    };
    let stage = StorePort::begin_snapshot_stage(
        &store,
        SnapshotHeader::from_captured(admission, &snapshot).unwrap(),
        &budget(),
    )
    .unwrap();
    StorePort::seal_snapshot_stage(&store, &stage.id, &budget()).unwrap();
    StorePort::publish_snapshot_stage(&store, &stage.id, &budget()).unwrap();
    let observe = |target: &str, sequence: u64| HostObservation {
        target: HostTargetId::new(target),
        host_boot: HostBootId::new("b"),
        epoch: 1,
        generation: 3,
        observed_at_utc: UtcMillis(100),
        observed_at_mono: MonoInstant(1),
        provenance: ObservationProvenance::FreshCurrentTarget,
        occupant: None,
        ui: HostUiState::Idle,
        terminal: Some(TerminalId::new(format!("terminal-{target}"))),
        occupancy: StructuralOccupancy::EmptyShell,
        incarnation: IncarnationEvidence::Verified {
            identity: "incarnation".into(),
            evidence_kind: EvidenceKind::NativeCurrentTarget,
        },
        execution: ExecutionEvidence::Unknown,
        call_id: HostCallId::new(format!("call-{sequence}")),
        connection_epoch: 1,
        observation_sequence: sequence,
        started_at_mono: MonoInstant(1),
        completed_at_mono: MonoInstant(1),
    };
    let publish = |observation: &HostObservation| {
        let admission = StorePort::begin_host_observation(&store, "i", &budget()).unwrap();
        assert!(
            StorePort::publish_current_target_observation(
                &store,
                &admission,
                observation,
                &budget()
            )
            .unwrap()
        );
    };
    let fresh_observation = observe("fresh", 2);
    publish(&fresh_observation);
    let fresh = OperatorFreshSeat {
        target: HostTargetId::new("fresh"),
        operation: OperationId::new("operator-fresh"),
    };
    let guard = OperatorTargetGuard::try_new(
        "i",
        &OperatorCommand::FreshSeat(fresh.clone()),
        fresh_observation.clone(),
    )
    .unwrap();
    let actor = || OperatorActor::from_peer(PeerIdentity::from_kernel(501), 501).unwrap();
    let CommandResult::OperatorFreshSeat(new_seat) = StorePort::mutate_operator(
        &store,
        OperatorRequest::FreshSeat(fresh.clone(), guard),
        actor(),
        &budget(),
    )
    .unwrap() else {
        panic!("wrong fresh route")
    };
    let replay_guard = OperatorTargetGuard::try_new(
        "i",
        &OperatorCommand::FreshSeat(fresh.clone()),
        fresh_observation,
    )
    .unwrap();
    assert_eq!(
        StorePort::mutate_operator(
            &store,
            OperatorRequest::FreshSeat(fresh, replay_guard),
            actor(),
            &budget()
        )
        .unwrap(),
        CommandResult::OperatorFreshSeat(new_seat.clone())
    );
    let rebind_observation = observe("repair", 3);
    publish(&rebind_observation);
    let rebind = OperatorRebind {
        seat: SeatId::new("saved"),
        target: HostTargetId::new("repair"),
        operation: OperationId::new("operator-rebind"),
    };
    let guard = OperatorTargetGuard::try_new(
        "i",
        &OperatorCommand::Rebind(rebind.clone()),
        rebind_observation,
    )
    .unwrap();
    assert_eq!(
        StorePort::mutate_operator(
            &store,
            OperatorRequest::Rebind(rebind, guard),
            actor(),
            &budget()
        )
        .unwrap(),
        CommandResult::OperatorRebound(SeatId::new("saved"))
    );
    let db = store.context.open_writer().unwrap();
    assert_eq!(db.query_row("SELECT COUNT(*) FROM allocation_decisions WHERE operator_label='operator:local-user:501'",[],|row|row.get::<_,i64>(0)).unwrap(),2);
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM seats WHERE role='operator_fresh'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT target_id,generation FROM seats WHERE id='saved'",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        )
        .unwrap(),
        ("repair".into(), 8)
    );
    assert_eq!(
        db.query_row(
            "SELECT state FROM invitations WHERE id='pending-invite'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "pending"
    );
    assert_eq!(
        db.query_row(
            "SELECT state FROM receipts WHERE message_id='pending-message' AND seat_id='saved'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "pending"
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM occupant_bindings WHERE ended_at IS NULL",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM seat_availability", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(db.query_row("SELECT reservation_id,reservation_boot,retry_step,minimum_delay_ms,effective_delay_ms,last_reservation_id,last_reservation_boot,last_reserved_at_utc,last_outcome FROM wake_work WHERE seat_id='saved'",[],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,i64>(2)?,row.get::<_,i64>(3)?,row.get::<_,i64>(4)?,row.get::<_,String>(5)?,row.get::<_,String>(6)?,row.get::<_,i64>(7)?,row.get::<_,String>(8)?))).unwrap(),
        ("active-attempt".into(),"prior-daemon".into(),3,60000,120000,"active-attempt".into(),"prior-daemon".into(),50,"submitted".into()));
    for (seat, target, sequence) in [(new_seat.as_str(), "fresh", 2), ("saved", "repair", 3)] {
        assert_eq!(db.query_row("SELECT structural_terminal_id,structural_incarnation,structural_incarnation_kind,structural_host_boot,structural_host_epoch,structural_connection_epoch,structural_observation_sequence FROM seats WHERE id=?1",[seat],|row|Ok((row.get::<_,Option<String>>(0)?,row.get::<_,Option<String>>(1)?,row.get::<_,Option<String>>(2)?,row.get::<_,Option<String>>(3)?,row.get::<_,Option<i64>>(4)?,row.get::<_,Option<i64>>(5)?,row.get::<_,Option<i64>>(6)?))).unwrap(),
            (Some(format!("terminal-{target}")),Some("incarnation".into()),Some("native_current_target".into()),Some("b".into()),Some(1),Some(1),Some(sequence)));
    }
    drop(db);
    drop(store);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn public_facade_check_in_commits_anchor_and_exact_empty_offer() {
    let path = std::env::temp_dir().join(format!("herdr-facade-{}.db", uuid::Uuid::new_v4()));
    let context = connection::StoreContext::new(path.clone(), Arc::new(FixedClock));
    let db = context.open_writer().unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at,host_boot,host_epoch) VALUES ('i',0,'b',1)",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('s1','i','resolved','native','p1',1,1,0)",[]).unwrap();
    db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','p1','b',1,1,0,'fresh','term-'||'p1','inc','coherent_enumeration',1)",[]).unwrap();
    drop(db);
    let store = SqliteStore::new(context, "i", StoreSettings::default()).unwrap();
    let command = CheckIn {
        mode: crate::protocol::commands::CheckInMode::Lifecycle {
            expected_binding_generation: 1,
        },
        claim: fixture_claim("s1", "p1"),
        operation: OperationId::new("check-in"),
    };
    let permit = StorePort::issue_cooperative_permit(
        &store,
        cooperative_permit_request(&PermitMutation::CheckIn(command.clone())).unwrap(),
        &budget(),
    )
    .unwrap();
    let request = RegisterAvailableRequest {
        command,
        read: ReadContext {
            instance: "i".into(),
            output: OutputSpec::default(),
            operation_scope: None,
        },
        operator: None,
    };
    let CommandResult::CheckedIn(offer) =
        StorePort::register_available(&store, request, permit, &budget()).unwrap()
    else {
        panic!()
    };
    assert_eq!(offer.warning_count, 0);
    assert!(offer.warnings.items.is_empty() && offer.inbox.items.is_empty());
    let db = store.context.open_writer().unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM seat_availability WHERE seat_id='s1'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert!(
        db.query_row(
            "SELECT offered_through_seq FROM warning_offer WHERE seat_id='s1'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap()
            > 0
    );
    drop(db);
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[test]
fn public_facade_rejects_expired_permit() {
    struct LateClock;
    impl Clock for LateClock {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(100)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(252)
        }
    }
    let path = std::env::temp_dir().join(format!("herdr-facade-{}.db", uuid::Uuid::new_v4()));
    let context = connection::StoreContext::new(path.clone(), Arc::new(LateClock));
    let db = context.open_writer().unwrap();
    db.execute_batch("\
        INSERT INTO host_instances(id,created_at,host_boot,host_epoch) VALUES ('i',0,'b',1);\
        INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('s','i','resolved','native','p',1,1,0);\
        INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','p','b',1,1,0,'fresh','term-'||'p','inc','coherent_enumeration',1);\
        INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) VALUES ('s',1,1,'p','b',1,'codex','n','00000000-0000-4000-8000-000000000001','cooperative_top_level',0,'term-'||'p','inc');\
    ").unwrap();
    drop(db);
    let store = SqliteStore::new(context, "i", StoreSettings::default()).unwrap();
    let command = CreateThread {
        topic: "expired".into(),
        goal: "goal".into(),
        claim: fixture_claim("s", "p"),
        operation: OperationId::new("expired"),
    };
    let request =
        cooperative_permit_request(&PermitMutation::CreateThread(command.clone())).unwrap();
    // Issued far earlier than the store's decision sample (monotonic 252).
    let permit = MutationPermit::cooperative(
        request.claim,
        request.operation,
        request.obligation,
        request.payload_hash,
        MonoInstant(1),
        (1, 0),
        budget(),
    );
    let error = StorePort::mutate(
        &store,
        PermitMutation::CreateThread(command),
        permit,
        &budget(),
    )
    .unwrap_err();
    assert_eq!(
        error.code,
        crate::protocol::results::ErrorCode::CallerUnverified
    );
    assert!(error.detail.contains("expired"));
    let db = store.context.open_writer().unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM threads", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM operations", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT decision_seq FROM host_instances WHERE id='i'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    drop(db);
    drop(store);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn public_facade_creates_thread_with_current_cooperative_permit() {
    let path = std::env::temp_dir().join(format!("herdr-facade-{}.db", uuid::Uuid::new_v4()));
    let context = connection::StoreContext::new(path.clone(), Arc::new(FixedClock));
    let db = context.open_writer().unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at,host_boot,host_epoch) VALUES ('i',0,'b',1)",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('s1','i','resolved','native','p1',1,1,0)",[]).unwrap();
    db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','p1','b',1,1,0,'fresh','term-'||'p1','inc','coherent_enumeration',1)",[]).unwrap();
    db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) VALUES ('s1',1,1,'p1','b',1,'codex','n','00000000-0000-4000-8000-000000000001','cooperative_top_level',0,'term-'||'p1','inc');",[]).unwrap();
    drop(db);
    let store = SqliteStore::new(context, "i", StoreSettings::default()).unwrap();
    let command = CreateThread {
        topic: "topic".into(),
        goal: "goal".into(),
        operation: OperationId::new("create"),
        claim: fixture_claim("s1", "p1"),
    };
    let CommandResult::ThreadCreated(thread) =
        permitted(&store, PermitMutation::CreateThread(command)).unwrap()
    else {
        panic!()
    };
    let db = store.context.open_writer().unwrap();
    assert_eq!(
        db.query_row(
            "SELECT goal FROM threads WHERE id=?1",
            [thread.as_str()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "goal"
    );
    drop(db);
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[test]
fn operation_status_uses_only_trusted_read_scope() {
    let path = std::env::temp_dir().join(format!("herdr-facade-{}.db", uuid::Uuid::new_v4()));
    let context = connection::StoreContext::new(path.clone(), Arc::new(FixedClock));
    let db = context.open_writer().unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at) VALUES ('i',0)",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('s1','i','unresolved','native',0,0),('s2','i','unresolved','native',0,0)",[]).unwrap();
    let original =
        serde_json::to_string(&CommandResult::ThreadCreated(ThreadId::new("secret"))).unwrap();
    db.execute("INSERT INTO operations(actor_scope,operation_key,digest,result_json,decided_at) VALUES ('seat:s1','op',zeroblob(32),?1,100)",[original]).unwrap();
    drop(db);
    let store = SqliteStore::new(context, "i", StoreSettings::default()).unwrap();
    let command = Command::OperationStatus(OperationStatusQuery {
        operation: OperationId::new("op"),
    });
    let mut read = ReadContext {
        instance: "i".into(),
        output: OutputSpec::default(),
        operation_scope: None,
    };
    assert!(StorePort::query(&store, &command, &read, &budget()).is_err());
    read.operation_scope = Some(OperationReadScope::Seat(SeatId::new("s2")));
    let CommandResult::OperationStatus(other) =
        StorePort::query(&store, &command, &read, &budget()).unwrap()
    else {
        panic!()
    };
    assert!(!other.committed);
    read.operation_scope = Some(OperationReadScope::Seat(SeatId::new("s1")));
    let CommandResult::OperationStatus(own) =
        StorePort::query(&store, &command, &read, &budget()).unwrap()
    else {
        panic!()
    };
    assert!(own.committed);
    assert_eq!(own.result_id.as_deref(), Some("secret"));
    drop(store);
    let _ = std::fs::remove_file(path);
}

fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(1_000),
        cancellation: Cancellation::default(),
    }
}

fn fixture_claim(seat: &str, target: &str) -> CallerClaim {
    CallerClaim {
        instance: "i".into(),
        seat: SeatId::new(seat),
        binding_generation: 1,
        role: crate::protocol::authority::CallerRole::TopLevel,
        harness: Harness::Codex,
        native_session: NativeSessionId::new("n"),
        execution: ExecutionId::new("00000000-0000-4000-8000-000000000001"),
        target: HostTargetId::new(target),
    }
}
/// Mutates through the production cooperative path: the service-local permit
/// issuer derives the permit, obligation and digest from the command itself.
fn permitted(
    store: &SqliteStore,
    mutation: PermitMutation,
) -> Result<CommandResult, crate::protocol::results::ApiError> {
    let request = cooperative_permit_request(&mutation)?;
    let permit = StorePort::issue_cooperative_permit(store, request, &budget())?;
    StorePort::mutate(store, mutation, permit, &budget())
}

#[test]
fn public_facade_creates_invites_sends_accepts_acks_and_archives_with_stable_ids() {
    let path = std::env::temp_dir().join(format!("herdr-facade-{}.db", uuid::Uuid::new_v4()));
    let context = connection::StoreContext::new(path.clone(), Arc::new(FixedClock));
    let db = context.open_writer().unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at,host_boot,host_epoch) VALUES ('i',0,'b',1)",
        [],
    )
    .unwrap();
    for seat in ["s1", "s2"] {
        db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES (?1,'i','resolved','native',?1,1,1,0)",[seat]).unwrap();
        db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i',?1,'b',1,1,0,'fresh','term-'||?1,'inc','coherent_enumeration',1)",[seat]).unwrap();
        db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,terminal_id,incarnation) VALUES (?1,1,1,?1,'b',1,'codex','n','00000000-0000-4000-8000-000000000001','cooperative_top_level',0,'term-'||?1,'inc')",[seat]).unwrap();
    }
    drop(db);
    let store = SqliteStore::new(context, "i", StoreSettings::default()).unwrap();
    let create = CreateThread {
        topic: "topic".into(),
        goal: "goal".into(),
        operation: OperationId::new("create"),
        claim: fixture_claim("s1", "s1"),
    };
    let CommandResult::ThreadCreated(thread) =
        permitted(&store, PermitMutation::CreateThread(create.clone())).unwrap()
    else {
        panic!()
    };
    let invite = Invite {
        thread: thread.clone(),
        seat: SeatId::new("s2"),
        deadline_millis: Some(1000),
        operation: OperationId::new("invite"),
        claim: fixture_claim("s1", "s1"),
    };
    let CommandResult::Invitation(invitation) =
        permitted(&store, PermitMutation::Invite(invite.clone())).unwrap()
    else {
        panic!()
    };
    let send = SendMessage {
        thread: thread.clone(),
        body: "hello".into(),
        invited_recipients: vec![SeatId::new("s2")],
        deadline_millis: None,
        operation: OperationId::new("send"),
        claim: fixture_claim("s1", "s1"),
    };
    loop {
        match StorePort::prepare_send_step(
            &store,
            &send,
            DurableWorkAdmission::new(16).unwrap(),
            &budget(),
        )
        .unwrap()
        {
            SendPreparationProgress::Ready { .. } => break,
            SendPreparationProgress::More { visited, .. } => assert!(visited > 0),
            SendPreparationProgress::Committed(_) => panic!("unexpected committed preparation"),
        }
    }
    let CommandResult::MessageSent(message) =
        permitted(&store, PermitMutation::SendMessage(send.clone())).unwrap()
    else {
        panic!()
    };
    let replay = permitted(&store, PermitMutation::SendMessage(send.clone())).unwrap();
    assert_eq!(replay, CommandResult::MessageSent(message.clone()));
    let accept = Accept {
        thread: thread.clone(),
        operation: OperationId::new("accept"),
        claim: fixture_claim("s2", "s2"),
    };
    let CommandResult::Accepted(accepted) =
        permitted(&store, PermitMutation::Accept(accept.clone())).unwrap()
    else {
        panic!()
    };
    assert_eq!(accepted, invitation);
    let ack = Ack {
        messages: vec![message.clone()],
        operation: OperationId::new("ack"),
        claim: fixture_claim("s2", "s2"),
    };
    let CommandResult::Acknowledged(ids) =
        permitted(&store, PermitMutation::Ack(ack.clone())).unwrap()
    else {
        panic!()
    };
    assert_eq!(ids.acknowledged, vec![message.clone()]);
    let replay = permitted(&store, PermitMutation::Ack(ack.clone())).unwrap();
    assert_eq!(replay, CommandResult::Acknowledged(ids));
    let archive = ThreadMutation {
        thread: thread.clone(),
        operation: OperationId::new("archive"),
        claim: fixture_claim("s1", "s1"),
    };
    let CommandResult::Archived(archived) =
        permitted(&store, PermitMutation::Archive(archive.clone())).unwrap()
    else {
        panic!()
    };
    assert_eq!(archived, thread);
    let db = store.context.open_writer().unwrap();
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM send_manifests WHERE message_id=?1",
            [message.as_str()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    drop(db);
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[test]
fn public_facade_routes_directory_through_real_query_connection() {
    let path = std::env::temp_dir().join(format!("herdr-facade-{}.db", uuid::Uuid::new_v4()));
    let context = connection::StoreContext::new(path.clone(), Arc::new(FixedClock));
    let db = context.open_writer().unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at) VALUES ('i',0)",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0)", []).unwrap();
    drop(db);
    let store = SqliteStore::new(context, "i", StoreSettings::default()).unwrap();
    let command = Command::Directory(DirectoryQuery {
        membership: None,
        membership_filter: DirectoryMembership::All,
        topic_contains: None,
        page: PageRequest {
            cursor: None,
            limit: 20,
            max_bytes: 16_384,
        },
    });
    let read = ReadContext {
        instance: "i".into(),
        output: OutputSpec::default(),
        operation_scope: None,
    };
    let CommandResult::Directory(page) =
        StorePort::query(&store, &command, &read, &budget()).unwrap()
    else {
        panic!("wrong query route")
    };
    assert_eq!(page.items.len(), 1);
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[test]
fn default_directory_and_inbox_resolve_only_the_trusted_selected_seat() {
    let path = std::env::temp_dir().join(format!("herdr-facade-{}.db", uuid::Uuid::new_v4()));
    let context = connection::StoreContext::new(path.clone(), Arc::new(FixedClock));
    let db = context.open_writer().unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at) VALUES ('i',0)",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('s1','i','unresolved','native',0,0),('s2','i','unresolved','native',0,0)",[]).unwrap();
    db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t1','i','one','goal',0,0),('t2','i','two','goal',0,0)",[]).unwrap();
    db.execute("INSERT INTO memberships(thread_id,seat_id,state,joined_at) VALUES ('t1','s1','joined',0),('t2','s2','joined',0)",[]).unwrap();
    drop(db);
    let store = SqliteStore::new(context, "i", StoreSettings::default()).unwrap();
    let command = Command::Directory(DirectoryQuery {
        membership: None,
        membership_filter: DirectoryMembership::Default,
        topic_contains: None,
        page: PageRequest::default(),
    });
    let read = ReadContext {
        instance: "i".into(),
        output: OutputSpec::default(),
        operation_scope: Some(OperationReadScope::Seat(SeatId::new("s1"))),
    };
    let CommandResult::Directory(page) =
        StorePort::query(&store, &command, &read, &budget()).unwrap()
    else {
        panic!()
    };
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].thread, ThreadId::new("t1"));
    let inbox = Command::Inbox(InboxQuery {
        seat: None,
        page: PageRequest::default(),
    });
    let CommandResult::Inbox(page) = StorePort::query(&store, &inbox, &read, &budget()).unwrap()
    else {
        panic!()
    };
    assert!(page.items.is_empty());
    let unscoped = ReadContext {
        operation_scope: None,
        ..read
    };
    assert!(StorePort::query(&store, &command, &unscoped, &budget()).is_err());
    assert!(StorePort::query(&store, &inbox, &unscoped, &budget()).is_err());
    drop(store);
    let _ = std::fs::remove_file(path);
}
