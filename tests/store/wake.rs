use super::*;
use crate::protocol::time::{Cancellation, MonoInstant};
use std::sync::Arc;

struct WakeClock;
const DAEMON_BOOT: &str = "00000000-0000-0000-0000-000000000001";
impl Clock for WakeClock {
    fn utc_now(&self) -> UtcMillis {
        UtcMillis(100)
    }
    fn monotonic_now(&self) -> MonoInstant {
        MonoInstant(1_000)
    }
}
fn budget() -> CallBudget {
    CallBudget {
        deadline: MonoInstant(10_000),
        cancellation: Cancellation::default(),
    }
}

#[test]
fn pending_invitation_is_a_wake_candidate_before_work_row_projection() {
    let path = std::env::temp_dir().join(format!("herdr-wake-{}.db", uuid::Uuid::new_v4()));
    let context = StoreContext::new(path.clone(), Arc::new(WakeClock));
    let db = context.open_writer().unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'host',1,1)",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('s','i','resolved','native','pane',1,1,0)",[]).unwrap();
    db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','pane','host',1,1,0,'fresh','term-'||'pane','inc','coherent_enumeration',1)",[]).unwrap();
    db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0)",[]).unwrap();
    db.execute(
        "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t','s','invited')",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES ('inv','t','s',1,'pending',0,1,100,100)",[]).unwrap();
    drop(db);
    let store = SqliteStore::new(context, "i", StoreSettings::default()).unwrap();
    let candidates = StorePort::wake_candidates(&store, PageRequest::default(), &budget()).unwrap();
    assert_eq!(candidates.items.len(), 1);
    assert!(candidates.items[0].has_pending_invitation);
    assert!(candidates.items[0].has_actionable_work());
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[test]
fn wake_candidate_resumes_past_one_hundred_settled_receipts() {
    let path = std::env::temp_dir().join(format!("herdr-wake-{}.db", uuid::Uuid::new_v4()));
    let context = StoreContext::new(path.clone(), Arc::new(WakeClock));
    let db = context.open_writer().unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'host',1,101)",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('s','i','resolved','native','pane',1,1,0)",[]).unwrap();
    db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','pane','host',1,1,0,'fresh','term-'||'pane','inc','coherent_enumeration',1)",[]).unwrap();
    db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0)",[]).unwrap();
    for n in 0..101 {
        let message = format!("m{n}");
        db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_seq,decision_at) VALUES (?1,'i','t',?2,'ordinary','body',?2,0)",params![message,n+1]).unwrap();
        db.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES (?1,'t','s',?2,100)",
            params![message,if n==100 {"pending"} else {"acked"}]).unwrap();
    }
    drop(db);
    let store = SqliteStore::new(context, "i", StoreSettings::default()).unwrap();
    let first = StorePort::wake_candidates(&store, PageRequest::default(), &budget()).unwrap();
    assert!(first.items.is_empty());
    assert!(first.has_more && first.next_cursor.is_some());
    let second = StorePort::wake_candidates(
        &store,
        PageRequest {
            cursor: first.next_cursor,
            ..PageRequest::default()
        },
        &budget(),
    )
    .unwrap();
    assert_eq!(second.items.len(), 1);
    assert!(second.items[0].has_pending_receipt);
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[test]
fn partial_attention_cursor_requires_its_complete_encoded_page_budget() {
    let path = std::env::temp_dir().join(format!("herdr-wake-{}.db", uuid::Uuid::new_v4()));
    let context = StoreContext::new(path.clone(), Arc::new(WakeClock));
    let db = context.open_writer().unwrap();
    db.execute_batch("\
        INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'host',1,101);\
        INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('s','i','resolved','native','pane',1,1,0);\
        INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','pane','host',1,1,0,'fresh','term-'||'pane','inc','coherent_enumeration',1);\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0);\
    ").unwrap();
    for n in 0..101 {
        let message = format!("m{n}");
        db.execute("INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_seq,decision_at) VALUES (?1,'i','t',?2,'ordinary','body',?2,0)",params![message,n+1]).unwrap();
        db.execute("INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES (?1,'t','s',?2,100)",
            params![message,if n==100 {"pending"} else {"acked"}]).unwrap();
    }
    drop(db);
    let store = SqliteStore::new(context, "i", StoreSettings::default()).unwrap();
    let complete = StorePort::wake_candidates(&store, PageRequest::default(), &budget()).unwrap();
    let minimum = serde_json::to_vec(&complete).unwrap().len() as u32;
    assert!(minimum > 256 && complete.items.is_empty() && complete.has_more);
    let exact = StorePort::wake_candidates(
        &store,
        PageRequest {
            max_bytes: minimum,
            ..PageRequest::default()
        },
        &budget(),
    )
    .unwrap();
    assert_eq!(exact, complete);
    let error = StorePort::wake_candidates(
        &store,
        PageRequest {
            max_bytes: minimum - 1,
            ..PageRequest::default()
        },
        &budget(),
    )
    .unwrap_err();
    assert_eq!(
        error.code,
        crate::protocol::results::ErrorCode::InvalidBudget
    );
    assert_eq!(error.required_minimum_bytes, Some(minimum));
    drop(store);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn wake_candidate_finds_unprojected_manifest_receipt_and_warning() {
    let path = std::env::temp_dir().join(format!("herdr-wake-{}.db", uuid::Uuid::new_v4()));
    let context = StoreContext::new(path.clone(), Arc::new(WakeClock));
    let db = context.open_writer().unwrap();
    db.execute_batch("\
        INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'host',1,10);\
        INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('s','i','resolved','native','pane',1,1,0);\
        INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','pane','host',1,1,0,'fresh','term-'||'pane','inc','coherent_enumeration',1);\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0);\
        INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES ('p','i','actor','o',zeroblob(32),'t',0,0,0,0,0,0,1,'sealed');\
        INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES ('p','t','s',1,300,0);\
        INSERT INTO prepared_unavailable_warnings(preparation_id,warning_key,warning_id,affected_seat_id,unavailability_episode,warning_offset,event_json) VALUES ('p','key','warning','s',1,1,'{}');\
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('m','i','t',1,'ordinary','body',1000,10);\
        INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('p','m','i','t',10,1000,1,0,1,1);\
    ").unwrap();
    drop(db);
    let store = SqliteStore::new(context, "i", StoreSettings::default()).unwrap();
    let page = StorePort::wake_candidates(&store, PageRequest::default(), &budget()).unwrap();
    assert_eq!(page.items.len(), 1);
    assert!(page.items[0].has_pending_receipt);
    assert_eq!(page.items[0].actionable_warning_seq, Some(10));
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[test]
fn second_published_receipt_advances_wake_frontier_before_projection() {
    use crate::ports::LogicalPublicationKey;
    let path = std::env::temp_dir().join(format!("herdr-wake-{}.db", uuid::Uuid::new_v4()));
    let context = StoreContext::new(path.clone(), Arc::new(WakeClock));
    let db = context.open_writer().unwrap();
    db.execute_batch("\
        INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'host',1,10);\
        INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('s','i','resolved','native','pane',1,1,0);\
        INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','pane','host',1,1,0,'fresh','term-'||'pane','inc','coherent_enumeration',1);\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0);\
        INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES ('p1','i','actor','o1',zeroblob(32),'t',0,0,0,0,0,0,1,'sealed');\
        INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES ('p1','t','s',1,300,0);\
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('m1','i','t',1,'ordinary','body',1000,10);\
        INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('p1','m1','i','t',10,1000,1,0,1,0);\
    ").unwrap();
    drop(db);
    let store = SqliteStore::new(context, "i", StoreSettings::default()).unwrap();
    let first = StorePort::wake_candidates(&store, PageRequest::default(), &budget())
        .unwrap()
        .items
        .remove(0);
    let first_frontier = first.attention_witness.unwrap().frontier();
    assert_eq!(
        first_frontier.addressed_receipt,
        Some(LogicalPublicationKey {
            decision_seq: 10,
            event_offset: 0
        })
    );
    let db = store.context.open_writer().unwrap();
    db.execute_batch("\
        INSERT INTO send_preparations(id,instance_id,operation_scope,operation_key,digest,thread_id,captured_membership_revision,captured_lifecycle_revision,captured_eligibility_revision,captured_timeline_revision,captured_config_revision,interval_high_water,recipient_high_water,status) VALUES ('p2','i','actor','o2',zeroblob(32),'t',0,0,0,0,0,0,2,'sealed');\
        INSERT INTO prepared_recipients(preparation_id,thread_id,seat_id,receipt_ordinal,frozen_duration_ms,eligible_at_snapshot) VALUES ('p2','t','s',2,300,0);\
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_at,decision_seq) VALUES ('m2','i','t',2,'ordinary','body',1100,11);\
        INSERT INTO send_manifests(preparation_id,message_id,instance_id,thread_id,decision_seq,decision_at,base_sequence,interval_high_water,recipient_count,warning_count) VALUES ('p2','m2','i','t',11,1100,2,0,1,0);\
        UPDATE host_instances SET decision_seq=11 WHERE id='i';\
    ").unwrap();
    drop(db);
    let second = StorePort::wake_candidates(&store, PageRequest::default(), &budget())
        .unwrap()
        .items
        .remove(0);
    let second_frontier = second.attention_witness.unwrap().frontier();
    assert_eq!(
        second_frontier.addressed_receipt,
        Some(LogicalPublicationKey {
            decision_seq: 11,
            event_offset: 0
        })
    );
    assert!(second_frontier.advanced_beyond(&first_frontier));
    drop(store);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn reservation_persists_daemon_boot_attempt_and_delay_then_ignores_late_completion() {
    let path = std::env::temp_dir().join(format!("herdr-wake-{}.db", uuid::Uuid::new_v4()));
    let context = StoreContext::new(path.clone(), Arc::new(WakeClock));
    let db = context.open_writer().unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'host',1,1)",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('s','i','resolved','native','pane',1,1,0)",[]).unwrap();
    db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','pane','host',1,1,0,'fresh','term-'||'pane','inc','coherent_enumeration',1)",[]).unwrap();
    db.execute("UPDATE observed_targets SET occupancy='occupied',ui_state='idle',verified_execution='exec',top_level_occupant=1 WHERE instance_id='i' AND target_id='pane'",[]).unwrap();
    db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES ('s',1,1,'pane','host',1,'codex','session','exec','fresh',0,0,'term-'||'pane','inc')",[]).unwrap();
    db.execute("INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0)",[]).unwrap();
    db.execute(
        "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t','s','invited')",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES ('inv','t','s',1,'pending',0,1,100,100)",[]).unwrap();
    drop(db);
    let settings = StoreSettings {
        daemon_boot: Some(uuid::Uuid::parse_str(DAEMON_BOOT).unwrap()),
        minimum_wake_delay_ms: 120_000,
        ..StoreSettings::default()
    };
    let store = SqliteStore::new(context, "i", settings).unwrap();
    let stale_candidate = StorePort::wake_candidates(&store, PageRequest::default(), &budget())
        .unwrap()
        .items
        .remove(0);
    let db = store.context.open_writer().unwrap();
    db.execute(
        "UPDATE host_instances SET decision_seq=decision_seq+1 WHERE id='i'",
        [],
    )
    .unwrap();
    drop(db);
    assert!(
        StorePort::reserve_wake(&store, &stale_candidate, &budget())
            .unwrap()
            .is_none()
    );
    let candidate = StorePort::wake_candidates(&store, PageRequest::default(), &budget())
        .unwrap()
        .items
        .remove(0);
    let reservation = StorePort::reserve_wake(&store, &candidate, &budget())
        .unwrap()
        .unwrap();
    assert!(StorePort::validate_wake_reservation(&store, &reservation, &budget()).unwrap());
    assert_eq!(reservation.lease_until, MonoInstant(6_000));
    let db = store.context.open_writer().unwrap();
    db.execute(
        "UPDATE occupant_bindings SET target_id='other' WHERE seat_id='s'",
        [],
    )
    .unwrap();
    drop(db);
    assert!(!StorePort::validate_wake_reservation(&store, &reservation, &budget()).unwrap());
    let db = store.context.open_writer().unwrap();
    db.execute(
        "UPDATE occupant_bindings SET target_id='pane' WHERE seat_id='s'",
        [],
    )
    .unwrap();
    drop(db);
    let db = store.context.open_writer().unwrap();
    db.execute(
        "UPDATE observed_targets SET generation=2 WHERE instance_id='i' AND target_id='pane'",
        [],
    )
    .unwrap();
    drop(db);
    assert!(!StorePort::validate_wake_reservation(&store, &reservation, &budget()).unwrap());
    let db = store.context.open_writer().unwrap();
    db.execute(
        "UPDATE observed_targets SET generation=1 WHERE instance_id='i' AND target_id='pane'",
        [],
    )
    .unwrap();
    db.execute(
        "UPDATE host_instances SET decision_seq=decision_seq+1 WHERE id='i'",
        [],
    )
    .unwrap();
    drop(db);
    assert!(!StorePort::validate_wake_reservation(&store, &reservation, &budget()).unwrap());
    assert_eq!(reservation.retained_minimum_delay_ms, 120_000);
    assert_eq!(reservation.retained_effective_delay_ms, 120_000);
    let db = store.context.open_writer().unwrap();
    let row:(String,i64,i64)=db.query_row("SELECT reservation_boot,minimum_delay_ms,effective_delay_ms FROM wake_work WHERE seat_id='s'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(row, (DAEMON_BOOT.into(), 120_000, 120_000));
    drop(db);
    StorePort::complete_wake(
        &store,
        WakeAttemptId::new("obsolete"),
        WakeOutcome::Submitted,
        &budget(),
    )
    .unwrap();
    let db = store.context.open_writer().unwrap();
    assert_eq!(
        db.query_row(
            "SELECT reservation_id FROM wake_work WHERE seat_id='s'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        reservation.attempt.as_str()
    );
    drop(db);
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[test]
fn old_boot_recovery_is_exact_preserves_history_and_cannot_clear_successor() {
    use crate::ports::{WakeRecoveryOutcome, WakeRecoveryRequest};
    let path = std::env::temp_dir().join(format!("herdr-wake-{}.db", uuid::Uuid::new_v4()));
    let context = StoreContext::new(path.clone(), Arc::new(WakeClock));
    let db = context.open_writer().unwrap();
    db.execute_batch("\
        INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'host',1,1);\
        INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('s','i','resolved','native','pane',1,1,0);\
        INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,occupancy,ui_state,verified_execution,top_level_occupant,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','pane','host',1,1,0,'fresh','occupied','idle','exec',1,'term-'||'pane','inc','coherent_enumeration',1);\
        INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES ('s',1,1,'pane','host',1,'codex','session','exec','fresh',0,0,'term-'||'pane','inc');\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0);\
        INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t','s','invited');\
        INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES ('inv','t','s',1,'pending',0,1,100,100);\
    ").unwrap();
    drop(db);
    let old_boot = uuid::Uuid::parse_str(DAEMON_BOOT).unwrap();
    let next_boot = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
    let settings = |boot| StoreSettings {
        daemon_boot: Some(boot),
        minimum_wake_delay_ms: 120_000,
        ..StoreSettings::default()
    };
    let old = SqliteStore::new(context, "i", settings(old_boot)).unwrap();
    let candidate = StorePort::wake_candidates(&old, PageRequest::default(), &budget())
        .unwrap()
        .items
        .remove(0);
    let reservation = StorePort::reserve_wake(&old, &candidate, &budget())
        .unwrap()
        .unwrap();
    drop(old);

    let reopened = SqliteStore::new(
        StoreContext::new(path.clone(), Arc::new(WakeClock)),
        "i",
        settings(next_boot),
    )
    .unwrap();
    let page =
        StorePort::wake_recovery_candidates(&reopened, PageRequest::default(), &budget()).unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].attempt, reservation.attempt);
    assert_eq!(page.items[0].prior_daemon_boot, old_boot);
    let request = WakeRecoveryRequest {
        instance: "i".into(),
        seat: SeatId::new("s"),
        attempt: reservation.attempt.clone(),
        prior_daemon_boot: old_boot,
        elected_boot: next_boot,
    };
    let mut false_owner = request.clone();
    false_owner.elected_boot = old_boot;
    assert!(StorePort::recover_wake_reservation(&reopened, false_owner, &budget()).is_err());
    let db = reopened.context.open_writer().unwrap();
    db.execute("CREATE TRIGGER fail_recovery BEFORE UPDATE ON wake_work BEGIN SELECT RAISE(ABORT,'recovery fault'); END", []).unwrap();
    drop(db);
    assert!(StorePort::recover_wake_reservation(&reopened, request.clone(), &budget()).is_err());
    let db = reopened.context.open_writer().unwrap();
    let still_active: String = db
        .query_row(
            "SELECT reservation_id FROM wake_work WHERE seat_id='s'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(still_active, reservation.attempt.as_str());
    db.execute("DROP TRIGGER fail_recovery", []).unwrap();
    drop(db);
    assert_eq!(
        StorePort::recover_wake_reservation(&reopened, request.clone(), &budget()).unwrap(),
        WakeRecoveryOutcome::Recovered
    );
    assert_eq!(
        StorePort::recover_wake_reservation(&reopened, request.clone(), &budget()).unwrap(),
        WakeRecoveryOutcome::AlreadySettled
    );
    let db = reopened.context.open_writer().unwrap();
    let retained: (Option<String>,String,String,i64,i64,i64) = db.query_row("SELECT reservation_id,last_reservation_id,last_outcome,retry_step,minimum_delay_ms,effective_delay_ms FROM wake_work WHERE seat_id='s'", [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?))).unwrap();
    assert_eq!(
        retained,
        (
            None,
            reservation.attempt.as_str().into(),
            "outcome_unknown".into(),
            0,
            120_000,
            120_000
        )
    );
    drop(db);
    let next_candidate = StorePort::wake_candidates(&reopened, PageRequest::default(), &budget())
        .unwrap()
        .items
        .remove(0);
    assert!(next_candidate.reservation_id.is_none());
    assert_eq!(
        next_candidate.last_reservation_id.as_ref(),
        Some(&reservation.attempt)
    );
    let successor = StorePort::reserve_wake(&reopened, &next_candidate, &budget())
        .unwrap()
        .unwrap();
    assert_ne!(successor.attempt, reservation.attempt);
    assert_eq!(
        StorePort::recover_wake_reservation(&reopened, request, &budget()).unwrap(),
        WakeRecoveryOutcome::Stale
    );
    StorePort::complete_wake(
        &reopened,
        reservation.attempt,
        WakeOutcome::Submitted,
        &budget(),
    )
    .unwrap();
    let db = reopened.context.open_writer().unwrap();
    let active: String = db
        .query_row(
            "SELECT reservation_id FROM wake_work WHERE seat_id='s'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(active, successor.attempt.as_str());
    drop(db);
    drop(reopened);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn recovery_page_includes_retired_zero_reason_attempt_without_stealing_current_boot() {
    use crate::ports::{WakeRecoveryOutcome, WakeRecoveryRequest};
    let path = std::env::temp_dir().join(format!("herdr-wake-{}.db", uuid::Uuid::new_v4()));
    let context = StoreContext::new(path.clone(), Arc::new(WakeClock));
    let old_boot = uuid::Uuid::parse_str(DAEMON_BOOT).unwrap();
    let current_boot = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
    let db = context.open_writer().unwrap();
    db.execute_batch(&format!("\
        INSERT INTO host_instances(id,created_at) VALUES ('i',0);\
        INSERT INTO seats(id,instance_id,state,role,generation,created_at,retired_at,retired_seq) VALUES ('retired','i','retired','native',2,0,0,1),('current','i','resolved','native',1,0,NULL,NULL);\
        INSERT INTO wake_work(seat_id,reason_bits,reservation_id,reservation_boot,last_reservation_id,last_reservation_boot,retry_step,minimum_delay_ms,effective_delay_ms) VALUES
          ('retired',0,'old','{old_boot}','old','{old_boot}',2,30000,120000),
          ('current',0,'current','{current_boot}','current','{current_boot}',1,30000,60000);\
    ")).unwrap();
    drop(db);
    let store = SqliteStore::new(
        context,
        "i",
        StoreSettings {
            daemon_boot: Some(current_boot),
            ..StoreSettings::default()
        },
    )
    .unwrap();
    let page =
        StorePort::wake_recovery_candidates(&store, PageRequest::default(), &budget()).unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].seat.as_str(), "retired");
    assert_eq!(page.items[0].attempt.as_str(), "old");
    assert_eq!(
        StorePort::recover_wake_reservation(
            &store,
            WakeRecoveryRequest {
                instance: "i".into(),
                seat: SeatId::new("retired"),
                attempt: WakeAttemptId::new("old"),
                prior_daemon_boot: old_boot,
                elected_boot: current_boot,
            },
            &budget()
        )
        .unwrap(),
        WakeRecoveryOutcome::Recovered
    );
    let db = store.context.open_writer().unwrap();
    let retired:(Option<String>,i64,i64,i64,String)=db.query_row("SELECT reservation_id,reason_bits,retry_step,effective_delay_ms,last_outcome FROM wake_work WHERE seat_id='retired'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
    assert_eq!(retired, (None, 0, 2, 120000, "outcome_unknown".into()));
    let current: String = db
        .query_row(
            "SELECT reservation_id FROM wake_work WHERE seat_id='current'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(current, "current");
    drop(db);
    drop(store);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn recognized_idle_unregistered_occupant_gets_recovery_hint_reservation() {
    let path = std::env::temp_dir().join(format!("herdr-wake-{}.db", uuid::Uuid::new_v4()));
    let context = StoreContext::new(path.clone(), Arc::new(WakeClock));
    let db = context.open_writer().unwrap();
    db.execute_batch("\
        INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'host',1,1);\
        INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('s','i','resolved','native','pane',1,1,0);\
        INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,occupancy,ui_state,verified_execution,top_level_occupant,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','pane','host',1,1,0,'fresh','occupied','idle','exec',1,'term-'||'pane','inc','coherent_enumeration',1);\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0);\
        INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t','s','invited');\
        INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES ('inv','t','s',1,'pending',0,1,100,100);\
    ").unwrap();
    drop(db);
    let store = SqliteStore::new(
        context,
        "i",
        StoreSettings {
            daemon_boot: Some(uuid::Uuid::parse_str(DAEMON_BOOT).unwrap()),
            ..StoreSettings::default()
        },
    )
    .unwrap();
    let candidate = StorePort::wake_candidates(&store, PageRequest::default(), &budget())
        .unwrap()
        .items
        .remove(0);
    let reservation = StorePort::reserve_wake(&store, &candidate, &budget())
        .unwrap()
        .unwrap();
    assert!(
        matches!(&reservation.authority,crate::ports::ReservedWakeAuthority::RecoveryHint {execution} if execution.as_str()=="exec")
    );
    assert!(StorePort::validate_wake_reservation(&store, &reservation, &budget()).unwrap());
    let db = store.context.open_writer().unwrap();
    db.execute("UPDATE observed_targets SET ui_state='active_turn' WHERE instance_id='i' AND target_id='pane'",[]).unwrap();
    drop(db);
    assert!(!StorePort::validate_wake_reservation(&store, &reservation, &budget()).unwrap());
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[test]
fn completion_cas_ignores_old_attempt_and_releases_matching_attempt_without_losing_history() {
    let path = std::env::temp_dir().join(format!("herdr-wake-{}.db", uuid::Uuid::new_v4()));
    let context = StoreContext::new(path.clone(), Arc::new(WakeClock));
    let db = context.open_writer().unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at) VALUES ('i',0)",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('s','i','unresolved','native',2,0)",[]).unwrap();
    db.execute("INSERT INTO wake_work(seat_id,reason_bits,binding_generation,reservation_id,reservation_boot,retry_step,minimum_delay_ms,effective_delay_ms,last_reservation_id,last_reservation_boot) VALUES ('s',1,1,'current',?1,2,30000,120000,'current',?1)",[DAEMON_BOOT]).unwrap();
    drop(db);
    let store = SqliteStore::new(
        context,
        "i",
        StoreSettings {
            daemon_boot: Some(uuid::Uuid::parse_str(DAEMON_BOOT).unwrap()),
            ..StoreSettings::default()
        },
    )
    .unwrap();
    StorePort::complete_wake(
        &store,
        WakeAttemptId::new("old"),
        WakeOutcome::Submitted,
        &budget(),
    )
    .unwrap();
    let db = store.context.open_writer().unwrap();
    assert_eq!(
        db.query_row(
            "SELECT reservation_id FROM wake_work WHERE seat_id='s'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "current"
    );
    drop(db);
    StorePort::complete_wake(
        &store,
        WakeAttemptId::new("current"),
        WakeOutcome::Submitted,
        &budget(),
    )
    .unwrap();
    let db = store.context.open_writer().unwrap();
    let row:(Option<String>,String,i64,i64)=db.query_row("SELECT reservation_id,last_outcome,reason_bits,effective_delay_ms FROM wake_work WHERE seat_id='s'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(row, (None, "unsafe".into(), 1, 120000));
    drop(db);
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[test]
fn matching_completion_records_submitted_and_retains_zero_reason_history() {
    let path = std::env::temp_dir().join(format!("herdr-wake-{}.db", uuid::Uuid::new_v4()));
    let context = StoreContext::new(path.clone(), Arc::new(WakeClock));
    let db = context.open_writer().unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at) VALUES ('i',0)",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('s','i','resolved','native',1,0)",[]).unwrap();
    db.execute("INSERT INTO wake_work(seat_id,reason_bits,binding_generation,reservation_id,reservation_boot,retry_step,minimum_delay_ms,effective_delay_ms,last_reservation_id,last_reservation_boot) VALUES ('s',0,1,'current',?1,1,30000,60000,'current',?1)",[DAEMON_BOOT]).unwrap();
    drop(db);
    let store = SqliteStore::new(
        context,
        "i",
        StoreSettings {
            daemon_boot: Some(uuid::Uuid::parse_str(DAEMON_BOOT).unwrap()),
            ..StoreSettings::default()
        },
    )
    .unwrap();
    StorePort::complete_wake(
        &store,
        WakeAttemptId::new("current"),
        WakeOutcome::Submitted,
        &budget(),
    )
    .unwrap();
    let db = store.context.open_writer().unwrap();
    let row:(Option<String>,String,i64,i64,String)=db.query_row("SELECT reservation_id,last_outcome,reason_bits,effective_delay_ms,last_reservation_id FROM wake_work WHERE seat_id='s'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
    assert_eq!(row, (None, "submitted".into(), 0, 60000, "current".into()));
    drop(db);
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[test]
fn unresolved_recovery_hold_blocks_candidate_continuity_even_with_fresh_target() {
    let path = std::env::temp_dir().join(format!("herdr-wake-{}.db", uuid::Uuid::new_v4()));
    let context = StoreContext::new(path.clone(), Arc::new(WakeClock));
    let db = context.open_writer().unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'host',1,1)",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('s','i','resolved','native','pane',1,1,0)",[]).unwrap();
    db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','pane','host',1,1,0,'fresh','term-'||'pane','inc','coherent_enumeration',1)",[]).unwrap();
    db.execute("INSERT INTO recovery_holds(instance_id,target_id,baseline_boot,baseline_epoch,reason) VALUES ('i','pane','host',1,'repair')",[]).unwrap();
    let attention = effective::EffectiveSeatAttention {
        has_pending_invitation: true,
        has_pending_receipt: false,
        latest_warning_seq: None,
        frontier: Default::default(),
    };
    let candidate = wake::load_candidate(&db, "i", &SeatId::new("s"), &attention, 1).unwrap();
    assert!(!candidate.continuity_resolved);
    drop(db);
    let _ = std::fs::remove_file(path);
}

/// Cooperative native policy (D2 reasoning): a resolved seat whose effective
/// observation has no verified execution but a terminal in a verified server
/// incarnation gets a structural Cooperative reservation, registered or not.
/// Positive empty-shell or blocked/active UI evidence, a missing incarnation,
/// or a live binding naming another terminal or incarnation refuses it, and
/// the final store fence rejects later such evidence.
#[test]
fn unverified_execution_gets_structural_cooperative_reservation() {
    use crate::ports::ReservedWakeAuthority;
    let open = |registered: bool| {
        let path = std::env::temp_dir().join(format!("herdr-wake-{}.db", uuid::Uuid::new_v4()));
        let context = StoreContext::new(path.clone(), Arc::new(WakeClock));
        let db = context.open_writer().unwrap();
        db.execute_batch("\
            INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'host',1,1);\
            INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('s','i','resolved','native','pane',1,1,0);\
            INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','pane','host',1,1,0,'fresh','term-pane','inc','native_current_target',1);\
            INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0);\
            INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t','s','invited');\
            INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES ('inv','t','s',1,'pending',0,1,100,100);\
        ").unwrap();
        if registered {
            // A cooperative check-in binding: self-reported execution, older
            // host epoch, same terminal and incarnation.
            db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES ('s',1,1,'pane','host',0,'claude','session','self-reported','cooperative_top_level',0,0,'term-pane','inc')",[]).unwrap();
        }
        drop(db);
        let store = SqliteStore::new(
            context,
            "i",
            StoreSettings {
                daemon_boot: Some(uuid::Uuid::parse_str(DAEMON_BOOT).unwrap()),
                ..StoreSettings::default()
            },
        )
        .unwrap();
        (store, path)
    };
    let set = |store: &SqliteStore, sql: &str| {
        store
            .context
            .open_writer()
            .unwrap()
            .execute(sql, [])
            .unwrap();
    };
    let reserve = |store: &SqliteStore| {
        let candidate = StorePort::wake_candidates(store, PageRequest::default(), &budget())
            .unwrap()
            .items
            .remove(0);
        StorePort::reserve_wake(store, &candidate, &budget()).unwrap()
    };
    for registered in [false, true] {
        let (store, path) = open(registered);
        let reservation = reserve(&store).expect("cooperative structural reservation");
        assert_eq!(
            reservation.authority,
            ReservedWakeAuthority::Cooperative {
                terminal: crate::protocol::ids::TerminalId::new("term-pane"),
                incarnation: "inc".into(),
                binding_generation: registered.then_some(1),
                harness: registered.then(|| "claude".to_string()),
            }
        );
        assert!(
            StorePort::validate_wake_reservation(&store, &reservation, &budget()).unwrap(),
            "registered {registered}"
        );
        for (label, change, restore) in [
            (
                "approval",
                "UPDATE observed_targets SET ui_state='approval_or_question'",
                "UPDATE observed_targets SET ui_state='unknown'",
            ),
            (
                "human input",
                "UPDATE observed_targets SET ui_state='human_input'",
                "UPDATE observed_targets SET ui_state='unknown'",
            ),
            (
                "active turn",
                "UPDATE observed_targets SET ui_state='active_turn'",
                "UPDATE observed_targets SET ui_state='unknown'",
            ),
            (
                "empty shell",
                "UPDATE observed_targets SET occupancy='empty_shell'",
                "UPDATE observed_targets SET occupancy='unknown'",
            ),
            (
                "new terminal",
                "UPDATE observed_targets SET terminal_id='term-new'",
                "UPDATE observed_targets SET terminal_id='term-pane'",
            ),
            (
                "unverified incarnation",
                "UPDATE observed_targets SET incarnation=NULL,incarnation_source_kind=NULL,connection_epoch=NULL",
                "UPDATE observed_targets SET incarnation='inc',incarnation_source_kind='native_current_target',connection_epoch=1",
            ),
        ] {
            set(&store, change);
            assert!(
                !StorePort::validate_wake_reservation(&store, &reservation, &budget()).unwrap(),
                "{label} must fail the final store fence"
            );
            set(&store, restore);
        }
        assert!(StorePort::validate_wake_reservation(&store, &reservation, &budget()).unwrap());
        drop(store);
        let _ = std::fs::remove_file(path);
    }
    // A live binding on another terminal or incarnation is not this occupant.
    for change in [
        "UPDATE occupant_bindings SET terminal_id='term-old'",
        "UPDATE occupant_bindings SET incarnation='old-server'",
    ] {
        let (store, path) = open(true);
        set(&store, change);
        assert!(reserve(&store).is_none(), "{change}");
        drop(store);
        let _ = std::fs::remove_file(path);
    }
    // An empty shell is never reserved.
    let (store, path) = open(false);
    set(
        &store,
        "UPDATE observed_targets SET occupancy='empty_shell'",
    );
    assert!(reserve(&store).is_none());
    drop(store);
    let _ = std::fs::remove_file(path);
}

/// A seat with one open binding of `harness` on a fresh, verified-incarnation
/// target with a pending invitation: the cooperative reservation fixture.
fn bound_seat_store(harness: &str) -> (SqliteStore, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!("herdr-wake-{}.db", uuid::Uuid::new_v4()));
    let context = StoreContext::new(path.clone(), Arc::new(WakeClock));
    let db = context.open_writer().unwrap();
    db.execute_batch("\
        INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'host',1,1);\
        INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('s','i','resolved','native','pane',1,1,0);\
        INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','pane','host',1,1,0,'fresh','term-pane','inc','native_current_target',1);\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0);\
        INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t','s','invited');\
        INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES ('inv','t','s',1,'pending',0,1,100,100);\
    ").unwrap();
    db.execute(
        "INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES ('s',1,1,'pane','host',0,?1,'session','self-reported','cooperative_top_level',0,0,'term-pane','inc')",
        [harness],
    )
    .unwrap();
    drop(db);
    let store = SqliteStore::new(
        context,
        "i",
        StoreSettings {
            daemon_boot: Some(uuid::Uuid::parse_str(DAEMON_BOOT).unwrap()),
            ..StoreSettings::default()
        },
    )
    .unwrap();
    (store, path)
}

fn reserve_for_seat(store: &SqliteStore) -> Option<crate::ports::WakeReservation> {
    let candidate = StorePort::wake_candidates(store, PageRequest::default(), &budget())
        .unwrap()
        .items
        .remove(0);
    StorePort::reserve_wake(store, &candidate, &budget()).unwrap()
}

/// Kills: a reservation that drops the open binding's harness, so the host
/// adapter could not compare it with Herdr's detected agent kind.
#[test]
fn cooperative_reservation_carries_bound_harness() {
    for harness in ["claude", "codex"] {
        let (store, path) = bound_seat_store(harness);
        let reservation = reserve_for_seat(&store).expect("cooperative reservation");
        assert!(
            matches!(
                &reservation.authority,
                crate::ports::ReservedWakeAuthority::Cooperative { harness: Some(bound), .. }
                    if bound == harness
            ),
            "{harness}: {:?}",
            reservation.authority
        );
        drop(store);
        let _ = std::fs::remove_file(path);
    }
}

/// Kills: waking a human-bound seat because its pane shows a claude/codex
/// agent: a human binding never yields wake authority, whatever the
/// effective observation (a fresh, verified-incarnation terminal) shows.
#[test]
fn human_bound_seat_with_agent_in_pane_gets_no_wake_authority() {
    let (store, path) = bound_seat_store("human");
    let candidate = StorePort::wake_candidates(&store, PageRequest::default(), &budget())
        .unwrap()
        .items
        .remove(0);
    assert!(
        StorePort::reserve_wake(&store, &candidate, &budget())
            .unwrap()
            .is_none()
    );
    drop(store);
    let _ = std::fs::remove_file(path);
}
