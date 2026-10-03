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
fn wake_candidate_is_found_past_one_hundred_settled_receipts() {
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
    // The pending probe finds the one pending receipt without walking the 100
    // settled ones, so the seat is complete within the first page.
    let second = StorePort::wake_candidates(&store, PageRequest::default(), &budget()).unwrap();
    assert!(!second.has_more && second.next_cursor.is_none());
    assert_eq!(second.items.len(), 1);
    assert!(second.items[0].has_pending_receipt);
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[test]
fn wake_page_requires_its_complete_encoded_page_budget() {
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
    assert!(minimum > 256 && complete.items.len() == 1 && !complete.has_more);
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
        None,
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
        None,
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
        None,
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
        None,
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
        None,
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
        // A cooperative check-in binding is required (TRUST-POLICY A4):
        // self-reported execution, older host epoch, same terminal and
        // incarnation; `registered` only sets `registered_at`.
        db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES ('s',1,1,'pane','host',0,'claude','session','self-reported','cooperative_top_level',0,?1,'term-pane','inc')",[registered.then_some(0_i64)]).unwrap();
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
                harness: Some("claude".to_string()),
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

/// The pre-ht-p03.12.9 recovery walk, kept as an oracle: a LEFT JOIN over every
/// seat of the instance, one page of at most 100 seats after `after`.
pub(super) fn old_walk(
    db: &rusqlite::Connection,
    instance: &str,
    after: u64,
    high: u64,
) -> Vec<(u64, String, Option<String>, Option<String>)> {
    let mut statement = db.prepare("SELECT s.ordinal,s.id,w.reservation_id,w.reservation_boot FROM seats s LEFT JOIN wake_work w ON w.seat_id=s.id WHERE s.instance_id=?1 AND s.ordinal>?2 AND s.ordinal<=?3 ORDER BY s.ordinal LIMIT 100").unwrap();
    statement
        .query_map(params![instance, after as i64, high as i64], |row| {
            Ok((
                row.get::<_, i64>(0)? as u64,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// What the old walk's candidate filter keeps, drained over every page: seats
/// with a reservation whose boot is not `current_boot`, in seat-ordinal order.
fn old_candidates(
    db: &rusqlite::Connection,
    instance: &str,
    current_boot: &str,
) -> Vec<(String, String, String)> {
    let high: i64 = db
        .query_row(
            "SELECT COALESCE(MAX(ordinal),0) FROM seats WHERE instance_id=?1",
            [instance],
            |row| row.get(0),
        )
        .unwrap();
    let (mut after, mut out) = (0u64, Vec::new());
    loop {
        let page = old_walk(db, instance, after, high as u64);
        let Some(last) = page.last() else { break };
        after = last.0;
        for (_, seat, attempt, boot) in page {
            if let (Some(attempt), Some(boot)) = (attempt, boot)
                && boot != current_boot
            {
                out.push((seat, attempt, boot));
            }
        }
    }
    out
}

/// `n` seats: every fifth retired, every third with a reservation (alternating
/// the old and the current daemon boot), every third-plus-one with a settled
/// (unreserved) wake_work row; retired and reserved overlap.
fn seed_recovery_seats(db: &rusqlite::Connection, n: u64, old: &str, current: &str) {
    db.execute_batch(&format!("\
        INSERT INTO host_instances(id,created_at) VALUES ('i',0);\
        WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<{n})\
        INSERT INTO seats(id,instance_id,state,role,generation,created_at,retired_at,retired_seq) SELECT 's'||x,'i',IIF(x%5=0,'retired','resolved'),'native',1,0,IIF(x%5=0,0,NULL),IIF(x%5=0,x,NULL) FROM n;\
        INSERT INTO wake_work(seat_id,reservation_id,reservation_boot) SELECT id,'r'||ordinal,IIF(ordinal%2=0,'{old}','{current}') FROM seats WHERE ordinal%3=0;\
        INSERT INTO wake_work(seat_id) SELECT id FROM seats WHERE ordinal%3=1;\
    ")).unwrap();
}

fn walk_all(store: &SqliteStore, limit: u16) -> (Vec<(String, String, String)>, usize) {
    let (mut cursor, mut out, mut pages) = (None, Vec::new(), 0);
    loop {
        let page = StorePort::wake_recovery_candidates(
            store,
            PageRequest {
                cursor,
                limit,
                ..PageRequest::default()
            },
            &budget(),
        )
        .unwrap();
        pages += 1;
        out.extend(page.items.iter().map(|c| {
            (
                c.seat.as_str().to_string(),
                c.attempt.as_str().to_string(),
                c.prior_daemon_boot.to_string(),
            )
        }));
        cursor = page.next_cursor;
        if cursor.is_none() {
            return (out, pages);
        }
    }
}

fn recovery_store(seats: u64) -> (SqliteStore, std::path::PathBuf, rusqlite::Connection) {
    let path = std::env::temp_dir().join(format!("herdr-wake-{}.db", uuid::Uuid::new_v4()));
    let context = StoreContext::new(path.clone(), Arc::new(WakeClock));
    let db = context.open_writer().unwrap();
    let current = "00000000-0000-0000-0000-000000000002";
    seed_recovery_seats(&db, seats, DAEMON_BOOT, current);
    let store = SqliteStore::new(
        context,
        "i",
        StoreSettings {
            daemon_boot: Some(uuid::Uuid::parse_str(current).unwrap()),
            ..StoreSettings::default()
        },
    )
    .unwrap();
    (store, path, db)
}

// Kills: a walk that drops mid-reservation seats of retired seats, mis-orders
// candidates, applies the boot filter wrongly, or loses the tail after the last
// reserved seat (the cursor must finish at the high water).
#[test]
fn recovery_walk_equals_the_old_walk() {
    let (store, path, db) = recovery_store(500);
    let expected = old_candidates(&db, "i", "00000000-0000-0000-0000-000000000002");
    assert_eq!(
        expected.len(),
        83,
        "fixture: reserved seats on the old boot"
    );
    assert!(
        expected.iter().any(|(seat, ..)| {
            let ordinal: u64 = seat[1..].parse().unwrap();
            ordinal.is_multiple_of(5)
        }),
        "fixture must include reserved retired seats"
    );
    let (got, _) = walk_all(&store, 100);
    assert_eq!(got, expected);
    drop((store, db));
    let _ = std::fs::remove_file(path);
}

// Kills: a cursor that does not resume after the last returned seat (repeats
// or skips candidates at a page boundary), and a first page that is not capped
// by the 100-row walk or the page limit.
#[test]
fn multi_page_recovery_walk_round_trips_its_cursor() {
    let (store, path, db) = recovery_store(900);
    let expected = old_candidates(&db, "i", "00000000-0000-0000-0000-000000000002");
    assert!(expected.len() > 100, "{}", expected.len());
    let (got, pages) = walk_all(&store, 100);
    assert!(
        pages >= 2,
        "{pages} pages for {} candidates",
        expected.len()
    );
    assert_eq!(got, expected);
    let (got_small, small_pages) = walk_all(&store, 7);
    assert!(small_pages > pages);
    assert_eq!(got_small, expected);
    drop((store, db));
    let _ = std::fs::remove_file(path);
}

/// A resolved seat with one live reservation `current` at ladder step 2 and a
/// pre-reservation row (`prior`, step 1) the caller carries as `PriorLadder`.
fn refusal_store(path: &std::path::Path) -> SqliteStore {
    let context = StoreContext::new(path.to_path_buf(), Arc::new(WakeClock));
    let db = context.open_writer().unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at) VALUES ('i',0)",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES ('s','i','resolved','native',1,0)",[]).unwrap();
    db.execute("INSERT INTO wake_work(seat_id,reason_bits,binding_generation,reservation_id,reservation_boot,reserved_at_utc,retry_step,minimum_delay_ms,effective_delay_ms,last_reservation_id,last_reservation_boot,last_reserved_at_utc,last_invitation_seq,last_invitation_offset) VALUES ('s',1,1,'current',?1,100,2,30000,120000,'current',?1,100,7,3)",[DAEMON_BOOT]).unwrap();
    drop(db);
    SqliteStore::new(
        context,
        "i",
        StoreSettings {
            daemon_boot: Some(uuid::Uuid::parse_str(DAEMON_BOOT).unwrap()),
            ..StoreSettings::default()
        },
    )
    .unwrap()
}
fn prior_ladder() -> crate::ports::PriorLadder {
    crate::ports::PriorLadder {
        retry_step: 1,
        minimum_delay_ms: 30_000,
        effective_delay_ms: 60_000,
        last_reservation_id: Some(WakeAttemptId::new("prior")),
        last_reservation_boot: Some(crate::protocol::ids::HostBootId::new("prior-boot")),
        last_reserved_at_utc: Some(UtcMillis(50)),
        last_reserved_frontier: crate::ports::LogicalAttentionFrontier {
            invitation: Some(crate::ports::LogicalPublicationKey {
                decision_seq: 4,
                event_offset: 1,
            }),
            addressed_receipt: None,
            actionable_warning: None,
        },
    }
}
type LadderRow = (
    Option<String>,
    i64,
    i64,
    i64,
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<String>,
);
fn ladder_row(store: &SqliteStore) -> LadderRow {
    let db = store.context.open_writer().unwrap();
    db.query_row("SELECT reservation_id,retry_step,minimum_delay_ms,effective_delay_ms,last_reservation_id,last_reservation_boot,last_reserved_at_utc,last_invitation_seq,last_invitation_offset,last_outcome FROM wake_work WHERE seat_id='s'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?))).unwrap()
}

#[test]
fn refused_completion_restores_the_prior_ladder_in_the_fenced_update() {
    // Kills: a Refused completion that leaves the advanced step in place (the
    // ladder would climb on every pre-send refusal) or restores a stale row.
    for (cause, outcome) in [
        (crate::ports::RefusalCause::Unsafe, "unsafe"),
        (crate::ports::RefusalCause::Unavailable, "unavailable"),
        (crate::ports::RefusalCause::TimedOut, "timed_out"),
    ] {
        let path = std::env::temp_dir().join(format!("herdr-wake-{}.db", uuid::Uuid::new_v4()));
        let store = refusal_store(&path);
        let matched = StorePort::complete_wake(
            &store,
            WakeAttemptId::new("current"),
            WakeOutcome::Refused(cause),
            Some(&prior_ladder()),
            &budget(),
        )
        .unwrap();
        assert!(matched, "{cause:?}");
        assert_eq!(
            ladder_row(&store),
            (
                None,
                1,
                30_000,
                60_000,
                Some("prior".into()),
                Some("prior-boot".into()),
                Some(50),
                Some(4),
                Some(1),
                Some(outcome.into())
            ),
            "{cause:?}"
        );
        drop(store);
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn non_refused_completions_ignore_a_supplied_restore_and_report_no_match() {
    // Kills: restoring on OutcomeUnknown (the unsent-prompt outcome keeps the
    // advanced step) or returning true for a plain settlement.
    let path = std::env::temp_dir().join(format!("herdr-wake-{}.db", uuid::Uuid::new_v4()));
    let store = refusal_store(&path);
    let matched = StorePort::complete_wake(
        &store,
        WakeAttemptId::new("current"),
        WakeOutcome::OutcomeUnknown,
        Some(&prior_ladder()),
        &budget(),
    )
    .unwrap();
    assert!(!matched);
    assert_eq!(
        ladder_row(&store),
        (
            None,
            2,
            30_000,
            120_000,
            Some("current".into()),
            Some(DAEMON_BOOT.into()),
            Some(100),
            Some(7),
            Some(3),
            Some("outcome_unknown".into())
        )
    );
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[test]
fn fence_miss_restores_nothing_and_keeps_the_advanced_step() {
    // Design roast r1 (ht-p03.56): mark-unresolved / registration loss clears
    // reservation_id between reserve and a Refused completion. The fenced
    // restore matches 0 rows: no error, the step stays advanced by one.
    let path = std::env::temp_dir().join(format!("herdr-wake-{}.db", uuid::Uuid::new_v4()));
    let store = refusal_store(&path);
    let db = store.context.open_writer().unwrap();
    db.execute(
        "UPDATE wake_work SET reservation_id=NULL,reservation_boot=NULL,binding_generation=NULL WHERE seat_id='s'",
        [],
    )
    .unwrap();
    drop(db);
    let before = ladder_row(&store);
    let matched = StorePort::complete_wake(
        &store,
        WakeAttemptId::new("current"),
        WakeOutcome::Refused(crate::ports::RefusalCause::Unavailable),
        Some(&prior_ladder()),
        &budget(),
    )
    .unwrap();
    assert!(!matched);
    assert_eq!(ladder_row(&store), before);
    assert_eq!(
        before.1, 2,
        "retry_step advanced by exactly one, not restored"
    );
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[test]
fn refused_completion_of_a_different_boot_matches_nothing() {
    // Kills: a restore fenced on reservation_id alone, which would let a stale
    // boot rewrite a successor daemon's reservation.
    let path = std::env::temp_dir().join(format!("herdr-wake-{}.db", uuid::Uuid::new_v4()));
    let store = refusal_store(&path);
    let db = store.context.open_writer().unwrap();
    db.execute(
        "UPDATE wake_work SET reservation_boot='00000000-0000-0000-0000-000000000009' WHERE seat_id='s'",
        [],
    )
    .unwrap();
    drop(db);
    let before = ladder_row(&store);
    let matched = StorePort::complete_wake(
        &store,
        WakeAttemptId::new("current"),
        WakeOutcome::Refused(crate::ports::RefusalCause::Unsafe),
        Some(&prior_ladder()),
        &budget(),
    )
    .unwrap();
    assert!(!matched);
    assert_eq!(ladder_row(&store), before);
    drop(store);
    let _ = std::fs::remove_file(path);
}

/// A file-backed store on instance `i`, thread `t`, and one live seat per
/// `(id, harness)`: occupied idle pane, current binding of that harness, an
/// invited membership and a pending invitation (decision sequence 1).
fn seeded_store(seats: &[(&str, &str)]) -> (SqliteStore, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!("herdr-wake-{}.db", uuid::Uuid::new_v4()));
    let context = StoreContext::new(path.clone(), Arc::new(WakeClock));
    let db = context.open_writer().unwrap();
    db.execute_batch("\
        INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'host',1,1);\
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0);\
    ").unwrap();
    for (seat, harness) in seats {
        let pane = format!("pane-{seat}");
        db.execute("INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES (?1,'i','resolved','native',?2,1,1,0)", params![seat, pane]).unwrap();
        db.execute("INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,occupancy,ui_state,verified_execution,top_level_occupant,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i',?1,'host',1,1,0,'fresh','occupied','idle','exec',1,'term-'||?1,'inc','coherent_enumeration',1)", [&pane]).unwrap();
        db.execute("INSERT INTO occupant_bindings(seat_id,generation,target_generation,target_id,host_boot,host_epoch,harness,native_session,execution_id,observation_provenance,observed_at,registered_at,terminal_id,incarnation) VALUES (?1,1,1,?2,'host',1,?3,'session','exec','fresh',0,0,'term-'||?2,'inc')", params![seat, pane, harness]).unwrap();
        db.execute(
            "INSERT INTO memberships(thread_id,seat_id,state) VALUES ('t',?1,'invited')",
            [seat],
        )
        .unwrap();
        db.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES ('inv-'||?1,'t',?1,1,'pending',0,1,100,100)", [seat]).unwrap();
    }
    drop(db);
    let settings = StoreSettings {
        daemon_boot: Some(uuid::Uuid::parse_str(DAEMON_BOOT).unwrap()),
        minimum_wake_delay_ms: 120_000,
        ..StoreSettings::default()
    };
    (SqliteStore::new(context, "i", settings).unwrap(), path)
}

fn page_seats(store: &SqliteStore) -> Vec<String> {
    let mut seats = Vec::new();
    let mut cursor = None;
    loop {
        let page = StorePort::wake_candidates(
            store,
            PageRequest {
                cursor: cursor.take(),
                ..PageRequest::default()
            },
            &budget(),
        )
        .unwrap();
        seats.extend(page.items.iter().map(|c| c.seat.as_str().to_owned()));
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => return seats,
        }
    }
}

/// Settle seat `seat`'s pending invitation and commit a decision.
fn accept_invitation(store: &SqliteStore, seat: &str) {
    let db = store.context.open_writer().unwrap();
    db.execute("UPDATE invitations SET state='accepted',accepted_at=1,accepted_actor_seat_id=?1,accepted_generation=1,accepted_observation='proof' WHERE id='inv-'||?1", [seat]).unwrap();
    db.execute(
        "UPDATE host_instances SET decision_seq=decision_seq+1 WHERE id='i'",
        [],
    )
    .unwrap();
}

// Kills: dropping the human exclusion from the seat walk (the human seat's
// pending invitation and receipt would list it), and any path that gives a
// human seat a reservation or a ladder step.
#[test]
fn human_bound_seat_never_in_a_candidate_page() {
    let (store, path) = seeded_store(&[("h", "human"), ("a", "codex")]);
    let db = store.context.open_writer().unwrap();
    db.execute_batch("\
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_seq,decision_at) VALUES ('m','i','t',1,'ordinary','body',1,0);\
        INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES ('m','t','h','pending',100);\
        INSERT INTO wake_work(seat_id,retry_step) VALUES ('h',3);\
    ").unwrap();
    drop(db);
    for _ in 0..3 {
        assert_eq!(page_seats(&store), ["a"]);
    }
    let candidate = StorePort::wake_candidates(&store, PageRequest::default(), &budget())
        .unwrap()
        .items
        .remove(0);
    assert_eq!(candidate.seat.as_str(), "a");
    assert!(
        StorePort::reserve_wake(&store, &candidate, &budget())
            .unwrap()
            .is_some()
    );
    let db = store.context.open_writer().unwrap();
    let human: (i64, Option<String>) = db
        .query_row(
            "SELECT retry_step,reservation_id FROM wake_work WHERE seat_id='h'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(human, (3, None), "the human seat's ladder never moves");
    drop(db);
    drop(store);
    let _ = std::fs::remove_file(path);
}

// Kills: re-emitting settled seats (the old `|| historical` rule): a seat with
// a wake_work row must leave the page once settled, keep its row untouched,
// and resume the ladder (retry_step 2) on its next actionable episode.
#[test]
fn settled_seat_leaves_the_page_and_keeps_its_wake_work_row() {
    let (store, path) = seeded_store(&[("s", "codex")]);
    let db = store.context.open_writer().unwrap();
    db.execute("INSERT INTO wake_work(seat_id,retry_step,last_invitation_seq,last_invitation_offset,last_outcome) VALUES ('s',2,1,1,'submitted')", []).unwrap();
    drop(db);
    let row = |store: &SqliteStore| -> (i64, Option<i64>, Option<i64>, Option<String>) {
        store
            .context
            .open_writer()
            .unwrap()
            .query_row(
                "SELECT retry_step,last_invitation_seq,last_invitation_offset,last_outcome FROM wake_work WHERE seat_id='s'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap()
    };
    let before = row(&store);
    assert_eq!(page_seats(&store), ["s"]);
    accept_invitation(&store, "s");
    for _ in 0..2 {
        assert!(
            page_seats(&store).is_empty(),
            "settled seat is not re-listed"
        );
    }
    assert_eq!(row(&store), before, "discovery never touches wake_work");
    let db = store.context.open_writer().unwrap();
    db.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES ('inv-2','t','s',2,'pending',0,3,100,100)", []).unwrap();
    db.execute("UPDATE host_instances SET decision_seq=3 WHERE id='i'", [])
        .unwrap();
    drop(db);
    let resumed = StorePort::wake_candidates(&store, PageRequest::default(), &budget())
        .unwrap()
        .items;
    assert_eq!(resumed.len(), 1);
    assert_eq!(
        resumed[0].retry_step, 2,
        "the ladder resumes, it does not reset"
    );
    drop(store);
    let _ = std::fs::remove_file(path);
}

// Kills: tying recovery to discovery (a seat that settled while reserved must
// still be reached by the recovery walk after a restart).
#[test]
fn seat_mid_reservation_when_it_settles_is_reached_by_recovery() {
    let (store, path) = seeded_store(&[("s", "codex")]);
    let candidate = StorePort::wake_candidates(&store, PageRequest::default(), &budget())
        .unwrap()
        .items
        .remove(0);
    let reservation = StorePort::reserve_wake(&store, &candidate, &budget())
        .unwrap()
        .unwrap();
    accept_invitation(&store, "s");
    assert!(page_seats(&store).is_empty());
    drop(store);
    // A restarted daemon (new boot) recovers the prior boot's reservation.
    let next_boot = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
    let reopened = SqliteStore::new(
        StoreContext::new(path.clone(), Arc::new(WakeClock)),
        "i",
        StoreSettings {
            daemon_boot: Some(next_boot),
            minimum_wake_delay_ms: 120_000,
            ..StoreSettings::default()
        },
    )
    .unwrap();
    assert!(page_seats(&reopened).is_empty());
    let recovery =
        StorePort::wake_recovery_candidates(&reopened, PageRequest::default(), &budget()).unwrap();
    assert_eq!(recovery.items.len(), 1);
    assert_eq!(recovery.items[0].attempt, reservation.attempt);
    drop(reopened);
    let _ = std::fs::remove_file(path);
}

// Kills: a witness stamped from anything but host_instances.decision_seq at
// page read: a decision that commits between discovery and reserve changes no
// attention row here, yet must refuse the reservation.
#[test]
fn reservation_after_a_decision_commit_is_refused() {
    let (store, path) = seeded_store(&[("s", "codex")]);
    let candidate = StorePort::wake_candidates(&store, PageRequest::default(), &budget())
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
        StorePort::reserve_wake(&store, &candidate, &budget())
            .unwrap()
            .is_none()
    );
    let reserved: i64 = store
        .context
        .open_writer()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM wake_work WHERE reservation_id IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(reserved, 0);
    let fresh = StorePort::wake_candidates(&store, PageRequest::default(), &budget())
        .unwrap()
        .items
        .remove(0);
    assert!(
        StorePort::reserve_wake(&store, &fresh, &budget())
            .unwrap()
            .is_some(),
        "a candidate read after the commit reserves"
    );
    drop(store);
    let _ = std::fs::remove_file(path);
}

// Kills: a decoder that still accepts attention / last_examined_key /
// scope_revision (the path still resumes mid-seat from a legacy cursor).
#[test]
fn legacy_wake_cursor_is_invalid_cursor() {
    use crate::protocol::pagination::SeatAttentionCursorState;
    let (store, path) = seeded_store(&[("s", "codex")]);
    let legacy = WakePageCursor {
        after_ordinal: 0,
        high_water_ordinal: 1,
        legacy: Some(LegacyWakePosition {
            last_examined_key: "s".into(),
            scope_revision: 1,
            attention: SeatAttentionCursorState {
                invitation_after_seq: 0,
                invitation_after_ordinal: 0,
                invitations_done: false,
                has_pending_invitation: false,
                invitation_frontier: None,
                receipts: None,
                receipts_done: false,
                has_pending_receipt: false,
                receipt_frontier_seq: None,
                physical_warning_after: 0,
                physical_warning_high_water: 0,
                manifest_warning_after: 0,
                manifest_warning_high_water: 0,
                next_manifest_warning: false,
                latest_warning_seq: None,
                latest_warning_offset: None,
            },
        }),
    }
    .encode("i")
    .unwrap();
    let error = StorePort::wake_candidates(
        &store,
        PageRequest {
            cursor: Some(legacy),
            ..PageRequest::default()
        },
        &budget(),
    )
    .unwrap_err();
    assert_eq!(
        error.code,
        crate::protocol::results::ErrorCode::InvalidCursor
    );
    drop(store);
    let _ = std::fs::remove_file(path);
}

// Kills: a walk that ends at the last live seat but reports `after < high_water`
// because retired seats follow it: the page would carry a cursor forever and a
// scheduler would never wrap back to the first seat.
#[test]
fn trailing_retired_seats_do_not_leave_a_dangling_cursor() {
    let (store, path) = seeded_store(&[("s", "codex")]);
    let db = store.context.open_writer().unwrap();
    db.execute("INSERT INTO seats(id,instance_id,state,role,generation,target_generation,created_at,retired_at,retired_seq) VALUES ('gone','i','retired','native',1,1,0,1,1)", []).unwrap();
    drop(db);
    let page = StorePort::wake_candidates(&store, PageRequest::default(), &budget()).unwrap();
    assert_eq!(page.items.len(), 1);
    assert!(!page.has_more && page.next_cursor.is_none(), "{page:?}");
    drop(store);
    let _ = std::fs::remove_file(path);
}

// Kills (Wave 18, ht-p03.12.5): dropping the human exclusion from the seat walk
// (the person's seat, holding a pending invitation and a pending receipt, would
// be listed on some page) and dropping the human guard from reservation
// authority (a candidate read while the seat was still agent-bound would still
// reserve and move the ladder after `me init` bound a person to the seat).
// The seat is first an ordinary codex occupant with a ladder row, so a real
// candidate exists for it; the person then takes the seat.
#[test]
fn human_bound_seat_with_pending_acks_is_never_a_wake_candidate() {
    let (store, path) = seeded_store(&[("h", "codex")]);
    let db = store.context.open_writer().unwrap();
    db.execute_batch("\
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_seq,decision_at) VALUES ('m','i','t',1,'ordinary','body',1,0);\
        INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES ('m','t','h','pending',100);\
        INSERT INTO wake_work(seat_id,retry_step) VALUES ('h',3);\
    ").unwrap();
    drop(db);
    let candidate = StorePort::wake_candidates(&store, PageRequest::default(), &budget())
        .unwrap()
        .items
        .remove(0);
    assert_eq!(candidate.seat.as_str(), "h");
    assert!(candidate.has_pending_invitation && candidate.has_pending_receipt);
    assert_eq!(candidate.retry_step, 3);
    // `me init` binds a person to the seat.
    let db = store.context.open_writer().unwrap();
    db.execute(
        "UPDATE occupant_bindings SET harness='human' WHERE seat_id='h' AND ended_at IS NULL",
        [],
    )
    .unwrap();
    drop(db);
    for limit in [1u16, 2, 100] {
        let mut listed = Vec::new();
        let mut cursor = None;
        loop {
            let page = StorePort::wake_candidates(
                &store,
                PageRequest {
                    cursor: cursor.take(),
                    limit,
                    ..PageRequest::default()
                },
                &budget(),
            )
            .unwrap();
            listed.extend(page.items.iter().map(|c| c.seat.as_str().to_owned()));
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        assert!(listed.is_empty(), "limit {limit}: {listed:?}");
    }
    assert!(
        StorePort::reserve_wake(&store, &candidate, &budget())
            .unwrap()
            .is_none(),
        "a human-bound seat is never reserved"
    );
    let db = store.context.open_writer().unwrap();
    let row: (i64, Option<String>) = db
        .query_row(
            "SELECT retry_step,reservation_id FROM wake_work WHERE seat_id='h'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(row, (3, None), "the person's ladder row never moves");
    drop(db);
    drop(store);
    let _ = std::fs::remove_file(path);
}

/// Kills: a cooperative wake authority for a seat with no open binding (the
/// harness comparison would be skipped and any recognized agent prompted).
#[test]
fn cooperative_wake_requires_an_open_binding() {
    let (store, path) = bound_seat_store("claude");
    assert!(reserve_for_seat(&store).is_some(), "control: bound seat");
    store
        .context
        .open_writer()
        .unwrap()
        .execute("UPDATE occupant_bindings SET ended_at=1", [])
        .unwrap();
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
    // This branch's wake discovery (B1) already leaves a person's seat out of
    // the candidates; any candidate that does appear must not reserve.
    let candidates = StorePort::wake_candidates(&store, PageRequest::default(), &budget())
        .unwrap()
        .items;
    for candidate in &candidates {
        assert!(
            StorePort::reserve_wake(&store, candidate, &budget())
                .unwrap()
                .is_none()
        );
    }
    // Control: the same fixture bound to an agent does yield wake authority.
    let (agent, agent_path) = bound_seat_store("claude");
    assert!(reserve_for_seat(&agent).is_some());
    drop(agent);
    let _ = std::fs::remove_file(agent_path);
    drop(store);
    let _ = std::fs::remove_file(path);
}

/// TRUST-POLICY A3 `managed_launch` (ht-5n6): a seat whose agent was launched
/// without a prompt and never checked in (Codex 0.159.3 runs no SessionStart
/// before its first turn) still gets the lost-prompt idle-recovery wake: the
/// launch binding is the open binding cooperative wake authority requires,
/// unregistered (no binding generation) and naming the launched harness.
/// Kills: the unbound seat that never reserved a wake (bd ht-5n6).
#[test]
fn managed_launch_binding_reserves_a_cooperative_wake() {
    let (store, path) = bound_seat_store("codex");
    store
        .context
        .open_writer()
        .unwrap()
        .execute("DELETE FROM occupant_bindings", [])
        .unwrap();
    let unbound = StorePort::wake_candidates(&store, PageRequest::default(), &budget())
        .unwrap()
        .items
        .remove(0);
    assert!(
        StorePort::reserve_wake(&store, &unbound, &budget())
            .unwrap()
            .is_none(),
        "control: no binding, no wake"
    );
    let recorded = StorePort::record_managed_launch(
        &store,
        crate::protocol::commands::RecordManagedLaunch {
            seat: SeatId::new("s"),
            target: crate::protocol::ids::HostTargetId::new("pane"),
            harness: crate::protocol::authority::Harness::Codex,
            terminal: crate::protocol::ids::TerminalId::new("term-pane"),
            incarnation: "inc".into(),
            host_boot: crate::protocol::ids::HostBootId::new("host"),
            target_generation: 1,
        },
        &budget(),
    )
    .unwrap();
    assert!(matches!(
        recorded,
        crate::protocol::results::CommandResult::ManagedLaunchRecorded(ref record) if record.recorded
    ));
    let reservation = reserve_for_seat(&store).expect("launched seat gets a wake");
    assert_eq!(
        reservation.authority,
        crate::ports::ReservedWakeAuthority::Cooperative {
            terminal: crate::protocol::ids::TerminalId::new("term-pane"),
            incarnation: "inc".into(),
            binding_generation: None,
            harness: Some("codex".to_string()),
        }
    );
    assert!(StorePort::validate_wake_reservation(&store, &reservation, &budget()).unwrap());
    let wake_generation: Option<i64> = store
        .context
        .open_writer()
        .unwrap()
        .query_row(
            "SELECT binding_generation FROM wake_work WHERE seat_id='s'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(wake_generation, None);
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[test]
fn durable_batch_uses_oldest_saturated_publication_and_clears_when_work_drains() {
    let path = std::env::temp_dir().join(format!("herdr-batch-{}.db", uuid::Uuid::new_v4()));
    let context = StoreContext::new(path.clone(), Arc::new(WakeClock));
    let db = context.open_writer().unwrap();
    db.execute_batch("INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'host',1,300);
        INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('s','i','resolved','native','pane',1,1,0);
        INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','pane','host',1,1,0,'fresh','term-pane','inc','coherent_enumeration',1);
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0);").unwrap();
    for n in 1..=150 {
        db.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES (?1,'t','s',?2,'pending',?3,?2,100000,100000)",params![format!("inv-{n}"),n,if n==1 {0}else{90}]).unwrap();
    }
    drop(db);
    let store = SqliteStore::new(context, "i", StoreSettings::default()).unwrap();
    let candidate = StorePort::wake_candidates(&store, PageRequest::default(), &budget())
        .unwrap()
        .items
        .remove(0);
    let first = StorePort::wake_batch_window(&store, &candidate, &budget())
        .unwrap()
        .unwrap();
    assert_eq!(first, (UtcMillis(30_000), 30_000));
    let db = store.context.open_writer().unwrap();
    db.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES ('later','t','s',151,'pending',99,151,100000,100000)",[]).unwrap();
    drop(db);
    assert_eq!(
        StorePort::wake_batch_window(&store, &candidate, &budget()).unwrap(),
        Some(first)
    );
    let db = store.context.open_writer().unwrap();
    db.execute(
        "UPDATE invitations SET state='recipient_retired',retired_at=100",
        [],
    )
    .unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM wake_batches", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    db.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES ('fresh','t','s',152,'pending',100,152,100000,100000)",[]).unwrap();
    drop(db);
    let candidate = StorePort::wake_candidates(&store, PageRequest::default(), &budget())
        .unwrap()
        .items
        .remove(0);
    assert_eq!(
        StorePort::wake_batch_window(&store, &candidate, &budget()).unwrap(),
        Some((UtcMillis(30_100), 30_000))
    );
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[test]
fn canonical_batch_cleanup_clears_catch_up_holds_and_release_starts_new_window() {
    let path = std::env::temp_dir().join(format!("herdr-batch-hold-{}.db", uuid::Uuid::new_v4()));
    let context = StoreContext::new(path.clone(), Arc::new(WakeClock));
    let db = context.open_writer().unwrap();
    db.execute_batch("INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'host',1,3);
        INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('s','i','resolved','native','pane',1,1,0);
        INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','pane','host',1,1,0,'fresh','term-pane','inc','coherent_enumeration',1);
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0);
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_seq,decision_at) VALUES ('m','i','t',1,'ordinary','body',1,0);
        INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms) VALUES ('m','t','s','pending',100000);").unwrap();
    drop(db);
    let store = SqliteStore::new(context, "i", StoreSettings::default()).unwrap();
    let candidate = StorePort::wake_candidates(&store, PageRequest::default(), &budget())
        .unwrap()
        .items
        .remove(0);
    assert_eq!(
        StorePort::wake_batch_window(&store, &candidate, &budget()).unwrap(),
        Some((UtcMillis(30_000), 30_000))
    );
    let db = store.context.open_writer().unwrap();
    db.execute("INSERT INTO catch_up(seat_id,thread_id,frontier_seq,binding_generation,execution_id,entered_at,state) VALUES ('s','t',0,1,'e',100,'active')",[]).unwrap();
    drop(db);
    // The restarted process has never tracked this batch in its runtime map.
    drop(store);
    let store = SqliteStore::new(
        StoreContext::new(path.clone(), Arc::new(WakeClock)),
        "i",
        StoreSettings::default(),
    )
    .unwrap();
    assert_eq!(
        StorePort::wake_batch_seats(&store, None, 16, &budget()).unwrap(),
        vec![SeatId::new("s")]
    );
    let db = store.context.open_writer().unwrap();
    db.execute("INSERT INTO invitations(id,thread_id,seat_id,episode,state,created_at,created_decision_seq,deadline_at,frozen_duration_ms) VALUES ('keep','t','s',1,'pending',100,3,100000,100000)",[]).unwrap();
    drop(db);
    assert!(!StorePort::clear_wake_batch_if_empty(&store, &SeatId::new("s"), &budget()).unwrap());
    let db = store.context.open_writer().unwrap();
    assert_eq!(
        db.query_row(
            "SELECT deadline_at FROM wake_batches WHERE seat_id='s'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        30_000
    );
    db.execute(
        "UPDATE invitations SET state='recipient_retired',retired_at=100 WHERE id='keep'",
        [],
    )
    .unwrap();
    drop(db);
    assert!(
        StorePort::wake_candidates(&store, PageRequest::default(), &budget())
            .unwrap()
            .items
            .is_empty()
    );
    assert!(StorePort::clear_wake_batch_if_empty(&store, &SeatId::new("s"), &budget()).unwrap());
    let db = store.context.open_writer().unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM wake_batches", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    db.execute(
        "UPDATE catch_up SET state='ended',end_reason='ready',ended_at=200,release_seq=2",
        [],
    )
    .unwrap();
    drop(db);
    let candidate = StorePort::wake_candidates(&store, PageRequest::default(), &budget())
        .unwrap()
        .items
        .remove(0);
    assert_eq!(
        StorePort::wake_batch_window(&store, &candidate, &budget()).unwrap(),
        Some((UtcMillis(30_200), 30_000))
    );
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[test]
fn durable_batch_window_enumeration_pages_only_retained_windows() {
    let path = std::env::temp_dir().join(format!("herdr-batch-page-{}.db", uuid::Uuid::new_v4()));
    let context = StoreContext::new(path.clone(), Arc::new(WakeClock));
    let db = context.open_writer().unwrap();
    db.execute(
        "INSERT INTO host_instances(id,created_at) VALUES ('i',0)",
        [],
    )
    .unwrap();
    for n in 0..25 {
        let seat = format!("seat-{n:02}");
        db.execute("INSERT INTO seats(id,instance_id,state,role,generation,created_at) VALUES (?1,'i','unresolved','native',0,0)",[&seat]).unwrap();
        db.execute(
            "INSERT INTO wake_batches(seat_id,deadline_at) VALUES (?1,30000)",
            [&seat],
        )
        .unwrap();
    }
    drop(db);
    let store = SqliteStore::new(context, "i", StoreSettings::default()).unwrap();
    let first = StorePort::wake_batch_seats(&store, None, 16, &budget()).unwrap();
    assert_eq!(first.len(), 16);
    assert_eq!(first[0], SeatId::new("seat-00"));
    let db = store.context.open_writer().unwrap();
    db.execute(
        "DELETE FROM wake_batches WHERE seat_id=?1",
        [first.last().unwrap().as_str()],
    )
    .unwrap();
    drop(db);
    let second = StorePort::wake_batch_seats(&store, first.last(), 16, &budget()).unwrap();
    assert_eq!(second.len(), 9);
    assert_eq!(second[0], SeatId::new("seat-16"));
    assert!(
        StorePort::wake_batch_seats(&store, second.last(), 16, &budget())
            .unwrap()
            .is_empty()
    );
    assert!(StorePort::wake_batch_seats(&store, None, 17, &budget()).is_err());
    drop(store);
    let _ = std::fs::remove_file(path);
}

#[test]
fn batch_window_ignores_human_waived_receipts_and_clears_last_required_waiver() {
    let path = std::env::temp_dir().join(format!("herdr-batch-waived-{}.db", uuid::Uuid::new_v4()));
    let context = StoreContext::new(path.clone(), Arc::new(WakeClock));
    let db = context.open_writer().unwrap();
    db.execute_batch("INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES ('i',0,'host',1,3);
        INSERT INTO seats(id,instance_id,state,role,target_id,generation,target_generation,created_at) VALUES ('s','i','resolved','native','pane',1,1,0);
        INSERT INTO observed_targets(instance_id,target_id,host_boot,epoch,generation,observed_at,provenance,terminal_id,incarnation,incarnation_source_kind,connection_epoch) VALUES ('i','pane','host',1,1,0,'fresh','term-pane','inc','coherent_enumeration',1);
        INSERT INTO threads(id,instance_id,topic,goal,created_at,updated_at) VALUES ('t','i','topic','goal',0,0);
        INSERT INTO messages(id,instance_id,thread_id,sequence,kind,body,decision_seq,decision_at) VALUES ('waived','i','t',1,'ordinary','body',1,0),('required','i','t',2,'ordinary','body',2,90);
        INSERT INTO receipts(message_id,thread_id,seat_id,state,frozen_duration_ms,ack_required) VALUES ('waived','t','s','pending',100000,0),('required','t','s','pending',100000,1);").unwrap();
    drop(db);
    let store = SqliteStore::new(context, "i", StoreSettings::default()).unwrap();
    let candidate = StorePort::wake_candidates(&store, PageRequest::default(), &budget())
        .unwrap()
        .items
        .remove(0);
    assert_eq!(
        StorePort::wake_batch_window(&store, &candidate, &budget()).unwrap(),
        Some((UtcMillis(30_090), 30_000))
    );
    let db = store.context.open_writer().unwrap();
    db.execute(
        "UPDATE receipts SET ack_required=0 WHERE message_id='required'",
        [],
    )
    .unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM wake_batches", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(db);
    drop(store);
    let _ = std::fs::remove_file(path);
}
